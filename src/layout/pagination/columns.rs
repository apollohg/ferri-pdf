//! Column boxes as fragmentainers (CSS Multi-column Layout 1). Each row segment of a
//! multi-column container is laid out by a nested paginator whose pages are its column boxes,
//! then placed into the enclosing flow one row of columns per fragmentainer.
use super::{
    ActiveBox, BreakRequest, Container, EffectNode, FLOW_LEVEL, FRAGMENTATION_TOLERANCE_PT,
    FlowCursor, Fragmentainers, OrderSnapshots, PaginationState, Paginator, PaintKey, PaintOrder,
    PaintOrders, Pending, ScopedPaint, SlicedBox, StickyConstraints, nested::Placement, overflows,
};
use crate::graphics::{Border, BoxDecoration, BoxPaint, DisplayItem, Rect};
use crate::layout::{
    LayoutError, TextLine,
    breaks::{BreakTail, Forced, first_fragment},
    measure::{FlowItem, Measurer, first_baseline, last_baseline},
    multicol::{ColumnGeometry, ColumnSegment, MeasuredColumns, WrapRows, rule_paint},
};
use crate::style::ColumnFill;
use std::collections::HashMap;
use std::rc::Rc;

#[cfg(test)]
mod tests;

/// Half-extent of the page area of rows not yet placed, where all content counts as visible.
const UNPLACED_EXTENT_PT: f32 = 1.0e6;
const UNPLACED_PAGE: Rect = Rect {
    x: -UNPLACED_EXTENT_PT,
    y: -UNPLACED_EXTENT_PT,
    width: 2.0 * UNPLACED_EXTENT_PT,
    height: 2.0 * UNPLACED_EXTENT_PT,
};

/// How a row of columns ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowEnd {
    /// The content ended in the row.
    Content,
    /// The content continues in the next fragmentainer.
    Overflow,
    /// A forced page break ended the row.
    Forced(Forced),
}

#[derive(Clone, Copy)]
struct RowFrame {
    geometry: ColumnGeometry,
    first: usize,
    offset: [f32; 2],
    height: f32,
    reference_height: f32,
    /// The container's definite height ends in this row, so further columns continue in the
    /// inline direction (§8.2).
    unbounded: bool,
    /// The page area in the coordinates of the row's first column.
    page_rect: Rect,
    /// Leaving the row is a page break as well as a column break.
    breaks_page: bool,
    /// The physical page the row is placed on, when the enclosing flow breaks into pages.
    physical_page: Option<usize>,
    /// Content precedes the row on its physical page.
    follows_flow: bool,
}

/// The rows of column boxes a row segment is laid out in, and how far its flow reached.
#[derive(Clone)]
pub(super) struct ColumnRows {
    geometry: ColumnGeometry,
    frames: Vec<RowFrame>,
    /// Column height in rows after the placed ones.
    provisional: f32,
    containing_height: Option<f32>,
    offset: [f32; 2],
    extents: Vec<f32>,
    ends: Vec<RowEnd>,
    /// The sides of each column's first and last formatted lines that are trimmed (CSS
    /// Inline 3 §6.2).
    text_trim: [bool; 2],
}
impl ColumnRows {
    fn new(geometry: ColumnGeometry, provisional: f32) -> Self {
        Self {
            geometry,
            frames: Vec::new(),
            provisional,
            containing_height: None,
            offset: [0.0; 2],
            extents: Vec::new(),
            ends: Vec::new(),
            text_trim: [false; 2],
        }
    }
    pub(super) fn text_trim(&self) -> [bool; 2] {
        self.text_trim
    }
    fn geometry(&self, row: usize) -> ColumnGeometry {
        self.frames
            .get(row)
            .or_else(|| self.frames.last())
            .map_or(self.geometry, |frame| frame.geometry)
    }
    fn row(&self, page: usize) -> usize {
        let row = self
            .frames
            .partition_point(|frame| frame.first <= page)
            .saturating_sub(1);
        match self.frames.get(row) {
            Some(frame) if frame.unbounded => row,
            Some(frame) => row + (page - frame.first) / frame.geometry.count,
            None => page / self.geometry.count,
        }
    }
    fn first(&self, row: usize) -> usize {
        if let Some(frame) = self.frames.get(row) {
            frame.first
        } else if let Some(frame) = self.frames.last() {
            frame.first + (row + 1 - self.frames.len()) * frame.geometry.count
        } else {
            row * self.geometry.count
        }
    }
    pub(super) fn content_box(&self, page: usize) -> Rect {
        let geometry = self.geometry(self.row(page));
        Rect {
            x: geometry.left(0),
            y: 0.0,
            width: geometry.width,
            height: self
                .frames
                .get(self.row(page))
                .map_or(self.provisional, |frame| frame.reference_height),
        }
    }
    pub(super) fn containing(&self, page: usize) -> (f32, f32, bool, Option<f32>) {
        let geometry = self.geometry(self.row(page));
        (
            geometry.left(0),
            geometry.width,
            geometry.rtl,
            self.containing_height,
        )
    }
    pub(super) fn offset(&self, page: usize) -> [f32; 2] {
        self.frames
            .get(self.row(page))
            .or_else(|| self.frames.last())
            .map_or(self.offset, |frame| frame.offset)
    }
    /// Whether the break after column `page` is also a page break: the column ends a bounded
    /// row whose successor is in the next enclosing fragmentainer (CSS Fragmentation 3 §4.2).
    pub(super) fn breaks_page(&self, page: usize) -> bool {
        let row = self.row(page);
        self.frames.get(row).is_some_and(|frame| {
            frame.breaks_page && !frame.unbounded && page + 1 == frame.first + frame.geometry.count
        })
    }
    /// The physical page holding column `page`, once its row is placed.
    pub(super) fn physical_page(&self, page: usize) -> Option<usize> {
        self.frames
            .get(self.row(page))
            .and_then(|frame| frame.physical_page)
    }
    /// Whether content precedes column `page` on its physical page: earlier columns of its
    /// row, or the flow before the row.
    pub(super) fn follows_flow(&self, page: usize) -> bool {
        self.frames
            .get(self.row(page))
            .is_some_and(|frame| frame.follows_flow || page > frame.first)
    }
    pub(super) fn has_next_row(&self, page: usize) -> bool {
        self.frames
            .get(self.row(page))
            .is_none_or(|frame| !frame.unbounded)
    }
    pub(super) fn height(&self, page: usize) -> f32 {
        self.frames
            .get(self.row(page))
            .map_or(self.provisional, |frame| frame.height)
    }
    pub(super) fn page_rect(&self, page: usize) -> Rect {
        let row = self.row(page);
        match self.frames.get(row) {
            Some(frame) => Rect {
                x: frame.page_rect.x - frame.geometry.shift(page - self.first(row)),
                ..frame.page_rect
            },
            None => UNPLACED_PAGE,
        }
    }
    /// The first column of the row after `page`'s, at or after `candidate`.
    pub(super) fn next_row(&self, page: usize, candidate: usize) -> usize {
        let target = candidate.max(self.first(self.row(page) + 1));
        let row = self.row(target);
        if target == self.first(row) || self.frames.get(row).is_some_and(|frame| frame.unbounded) {
            target
        } else {
            self.first(row + 1)
        }
    }
    pub(super) fn leave(&mut self, page: usize, y: f32) {
        if self.extents.len() <= page {
            self.extents.resize(page + 1, 0.0);
        }
        self.extents[page] = self.extents[page].max(y);
    }
    pub(super) fn exit(&mut self, page: usize, next: usize, forced: Option<Forced>) {
        let (from, to) = (self.row(page), self.row(next));
        if to > from {
            if self.ends.len() <= from {
                self.ends.resize(from + 1, RowEnd::Content);
            }
            self.ends[from] = if let Some(forced) = forced.filter(|forced| forced.is_page()) {
                RowEnd::Forced(forced)
            } else {
                RowEnd::Overflow
            };
        }
    }
    fn end(&self, row: usize) -> RowEnd {
        self.ends.get(row).copied().unwrap_or(RowEnd::Content)
    }
    fn pages(&self, row: usize, laid_out: usize) -> std::ops::Range<usize> {
        let first = self.first(row);
        let end = if self.frames[row].unbounded {
            laid_out
        } else {
            laid_out.min(first + self.geometry(row).count)
        };
        first..end.max(first)
    }
    /// The tallest column of `row`: the used column height of an auto-height container.
    fn extent(&self, row: usize, laid_out: usize) -> f32 {
        self.pages(row, laid_out)
            .filter_map(|page| self.extents.get(page))
            .fold(0.0, |extent, column| extent.max(*column))
    }
}

/// Paint context of the flow a row segment continues.
#[derive(Clone)]
struct FlowStart {
    track_overflow: bool,
    offset: [f32; 2],
    key: PaintKey,
    context: PaintKey,
    orders: PaintOrders,
    /// List markers waiting for the first line box.
    markers: Vec<crate::layout::markers::MeasuredMarker>,
}

/// Which rows of a segment are balanced (§7.1).
#[derive(Clone, Copy)]
struct Balance {
    before_spanner: bool,
    /// The row where the content ends.
    last: bool,
    /// Rows a forced page break ends.
    forced: bool,
}
impl Balance {
    /// Content before a spanner is always balanced; `auto` fills columns sequentially only
    /// where the column height is constrained.
    fn new(fill: ColumnFill, before_spanner: bool, constrained: bool) -> Self {
        Self {
            before_spanner,
            last: before_spanner || fill != ColumnFill::Auto || !constrained,
            forced: fill == ColumnFill::BalanceAll,
        }
    }
}

#[derive(Clone)]
pub(super) struct ColumnCursor<'a, 't> {
    geometry: ColumnGeometry,
    orthogonal: bool,
    rule: Border,
    fill: ColumnFill,
    remaining: Option<f32>,
    containing_height: Option<f32>,
    segments: std::vec::IntoIter<ColumnSegment<'a>>,
    text_trim: [bool; 2],
    wrap: Option<WrapRows>,
    tail: BreakTail,
    spanner: Option<(Box<FlowCursor<'a, 't>>, f32)>,
    row: Option<ColumnRow<'a, 't>>,
    entry: Option<(Vec<FlowItem<'a>>, Balance)>,
}
impl<'a: 't, 't: 'a> ColumnCursor<'a, 't> {
    pub(super) fn detach_orders(&mut self, snapshots: &mut OrderSnapshots) {
        if let Some((spanner, _)) = &mut self.spanner {
            spanner.detach_orders(snapshots);
        }
        if let Some(row) = &mut self.row {
            row.layout.detach_orders(snapshots);
            for placement in &mut row.placements {
                placement.position.detach_orders(snapshots);
            }
        }
    }
    pub(super) fn new(columns: MeasuredColumns<'a>, tail: BreakTail) -> Self {
        Self {
            geometry: columns.geometry,
            orthogonal: columns.orthogonal,
            rule: columns.rule,
            fill: columns.fill,
            remaining: columns.definite,
            containing_height: columns.definite,
            segments: columns.segments.into_iter(),
            text_trim: columns.text_trim,
            wrap: columns.wrap,
            tail,
            spanner: None,
            row: None,
            entry: None,
        }
    }
}

#[derive(Clone)]
struct ColumnRow<'a, 't> {
    tail: BreakTail,
    layout: ColumnLayout<'a, 't>,
    balance: Balance,
    placements: Vec<RowPlacement<'a>>,
    /// The fragmentainer of the last row placed, which a wrapped row follows after a gap.
    last_page: Option<usize>,
}

#[derive(Clone)]
struct RowPlacement<'a> {
    position: super::cells::FlowPosition<'a>,
    used: f32,
}

/// Owns nested layout state without retaining a borrow of the shared measurer.
#[derive(Clone)]
struct ColumnLayout<'a, 't> {
    state: PaginationState<'a, 't, ()>,
    cursor: Option<Box<FlowCursor<'a, 't>>>,
    request: Option<BreakRequest>,
}
impl<'a: 't, 't: 'a> ColumnLayout<'a, 't> {
    fn new(
        measurer: &mut Measurer<'t>,
        items: &[FlowItem<'a>],
        mut rows: ColumnRows,
        start: &FlowStart,
    ) -> Result<Self, LayoutError> {
        measurer.work.spend(items.len().max(1))?;
        rows.offset = start.offset;
        Ok(Self {
            state: Paginator::nested(measurer, rows, start).with_measurer(()),
            cursor: Some(FlowCursor::new(items.to_vec(), BreakTail::default())),
            request: None,
        })
    }

    fn snapshot(&self, work: &crate::layout::work::Budget) -> Result<Self, LayoutError> {
        let paints: usize = self
            .state
            .display_lists
            .iter()
            .chain(&self.state.foreground)
            .map(Vec::len)
            .sum();
        let items: usize = self
            .cursor
            .iter()
            .flat_map(|cursor| &cursor.frames)
            .map(|frame| frame.items.len())
            .sum();
        work.spend((self.state.pages.len() + paints + items).max(1))?;
        let mut snapshot = self.clone();
        let mut orders = OrderSnapshots::default();
        snapshot.detach_orders(&mut orders);
        work.spend(orders.len())?;
        Ok(snapshot)
    }

    fn detach_orders(&mut self, orders: &mut OrderSnapshots) {
        self.state.detach_orders(orders);
        if let Some(cursor) = &mut self.cursor {
            cursor.detach_orders(orders);
        }
    }

    fn rows(&self) -> &ColumnRows {
        let Fragmentainers::Columns(rows) = &self.state.fragmentainers else {
            unreachable!("column layouts own column fragmentainers");
        };
        rows
    }

    fn rows_mut(&mut self) -> &mut ColumnRows {
        let Fragmentainers::Columns(rows) = &mut self.state.fragmentainers else {
            unreachable!("column layouts own column fragmentainers");
        };
        rows
    }
    fn probe_row(
        self,
        measurer: &mut Measurer<'t>,
        row: usize,
    ) -> Result<(Self, RowEnd), LayoutError> {
        let mut candidate = self;
        let unbounded = std::mem::replace(&mut candidate.rows_mut().frames[row].unbounded, false);
        let (mut candidate, end) = candidate.through_row(measurer, row)?;
        candidate.rows_mut().frames[row].unbounded = unbounded;
        Ok((candidate, end))
    }

    fn push_frame(&mut self, mut frame: RowFrame) -> usize {
        let page = self.state.page;
        let rows = self.rows_mut();
        let row = rows.frames.len();
        frame.first = rows.first(row);
        rows.frames.push(frame);
        let content_box = rows.content_box(page);
        self.state.height = self.state.fragmentainers.height(self.state.page);
        self.state.page_rect = self.state.fragmentainers.rect(self.state.page);
        if row == 0 {
            self.state.canvas_rect = content_box;
            self.state.canvas.rect = self.state.page_rect;
        }
        row
    }

    fn through_row(
        mut self,
        measurer: &mut Measurer<'t>,
        row: usize,
    ) -> Result<(Self, RowEnd), LayoutError> {
        loop {
            if self.cursor.is_none() || self.rows().row(self.state.page) > row {
                let end = self.rows().end(row);
                return Ok((self, end));
            }
            if let Some(request) = self.request {
                let next = self.state.fragmentainers.next(
                    self.state.page,
                    self.state.page + 1,
                    request.forced,
                );
                if self.rows().row(next) > row {
                    let end = request
                        .forced
                        .filter(|forced| forced.is_page())
                        .map_or(RowEnd::Overflow, RowEnd::Forced);
                    return Ok((self, end));
                }
            }
            self = self.advance(measurer)?;
            let end = self.state.trimmed_flow_end();
            if let Fragmentainers::Columns(rows) = &mut self.state.fragmentainers {
                rows.leave(self.state.page, end);
            }
        }
    }

    fn extent(&self, row: usize) -> f32 {
        self.rows().extent(row, self.state.pages.len())
    }

    fn used_height(
        &self,
        row: usize,
        height: f32,
        definite: bool,
        balance: Balance,
        end: RowEnd,
    ) -> f32 {
        if definite && !(balance.before_spanner && end == RowEnd::Content) {
            height
        } else {
            self.extent(row)
        }
    }

    fn fits(&self, row: usize, end: RowEnd) -> bool {
        end != RowEnd::Overflow
            && self.rows().pages(row, self.state.pages.len()).len()
                <= self.rows().geometry(row).count
    }

    fn advance(self, measurer: &mut Measurer<'t>) -> Result<Self, LayoutError> {
        let mut paginator = self.state.with_measurer(measurer);
        if let Some(request) = self.request {
            paginator.new_page(request.forced, request.preserve_empty)?;
        }
        let (cursor, request) = paginator
            .resume_flow(
                self.cursor
                    .expect("completed column layouts are not resumed"),
            )?
            .unzip();
        if cursor.is_none() {
            // The columns' own inline content has no block container here.
            paginator.place_inline_pending(0)?;
        }
        Ok(Self {
            state: paginator.with_measurer(()),
            cursor,
            request,
        })
    }

    fn set_row_height(&mut self, row: usize, height: f32) {
        self.rows_mut().frames[row].height = height;
        self.state.height = self.state.fragmentainers.height(self.state.page);
    }

    fn into_output(self, measurer: &mut Measurer<'t>) -> Result<ColumnOutput<'a>, LayoutError> {
        assert!(
            self.cursor.is_none(),
            "column output requires a completed flow"
        );
        self.state.with_measurer(measurer).into_output()
    }
}

/// A row segment laid out in column boxes.
struct ColumnOutput<'a> {
    anchors: crate::layout::anchors::Measurements,
    overflow: Option<std::collections::BTreeMap<usize, Rect>>,
    string_locations: crate::layout::strings::StringLocations,
    pages: Vec<Vec<TextLine>>,
    geometric_baselines: Vec<[Option<f32>; 2]>,
    display_lists: Vec<Vec<ScopedPaint>>,
    foreground: Vec<Vec<ScopedPaint>>,
    underlays: HashMap<(usize, usize), Vec<ScopedPaint>>,
    flow_content: Vec<bool>,
    /// Columns holding content, between which rules are drawn (§4).
    content: Vec<bool>,
    clips: Vec<EffectNode<'a>>,
    sticky: Vec<StickyConstraints>,
    sliced: Vec<SlicedBox<'a>>,
    /// Absolutely positioned and fixed boxes whose containing block is outside the columns.
    pending: Vec<Pending<'a>>,
    orders: PaintOrders,
    frame_requests: Vec<Rc<super::MeasuredBlock<'a>>>,
    frame_reads: Vec<super::frames::Key>,
    rows: ColumnRows,
    markers_left: bool,
}
impl<'a: 't, 'm, 't: 'a> Paginator<'a, 'm, 't> {
    fn nested(measurer: &'m mut Measurer<'t>, rows: ColumnRows, start: &FlowStart) -> Self {
        let page_rect = rows.page_rect(0);
        let content_box = rows.content_box(0);
        let mut paginator = Self::new(
            measurer,
            Fragmentainers::Columns(rows),
            BoxPaint::whole(page_rect, BoxDecoration::default()),
            page_rect,
        );
        paginator.string_locations.retain_column_events();
        paginator.root_overflow = start.track_overflow.then(Default::default);
        paginator.canvas_rect = content_box;
        paginator.offset = start.offset;
        paginator.key = start.key.clone();
        paginator.context = start.context.clone();
        paginator.orders = start.orders.snapshot();
        // Column content takes the multi-column container's perspective and 3D rendering
        // context when its effects are placed in the container's flow.
        paginator.parent_3d = Some(paginator.effect_node(None, super::Effect::InheritedParent3d));
        paginator.containers.push(Container {
            pending: Vec::new(),
            fixed: false,
        });
        for marker in &start.markers {
            paginator.boxes.push(ActiveBox {
                grid_orders: HashMap::new(),
                grid: None,
                overflow: None,
                visible: true,
                pending: Vec::new(),
                float_placement: None,
                geometry: None,
                page_scope: None,
                open_groups: 0,
                offset: start.offset,
                scope: start.key.clone(),
                order: PaintOrder::FIRST,
                clip: None,
                group: None,
                clip_node: None,
                outer_clip_node: None,
                transform_node: None,
                motion_reference_node: None,
                motion_overflow: Vec::new(),
                clip_rect_node: None,
                clip_path_node: None,
                parent_3d_node: None,
                sticky_node: None,
                element_node: None,
                sticky_children: Vec::new(),
                sticky_scrolled: Vec::new(),
                group_node: None,
                paint: None,
                fragments: Vec::new(),
                start_space: 0.0,
                repeat_space: 0.0,
                end_space: 0.0,
                reserves_end: false,
                clone: false,
                marker: Some(Box::new(marker.clone())),
                sliced: None,
                canvas_root: false,
                navigation: crate::document::BoxNavigation::default(),
            });
        }
        paginator
    }

    fn into_output(mut self) -> Result<ColumnOutput<'a>, LayoutError> {
        self.complete_column_motion()?;
        self.enter(self.page);
        let content = (0..self.pages.len())
            .map(|page| self.flow_content[page] || self.page_has_ink(page))
            .collect();
        let mut pending = self
            .containers
            .pop()
            .map(|container| container.pending)
            .unwrap_or_default();
        pending.append(&mut self.fixed);
        let Fragmentainers::Columns(rows) = self.fragmentainers else {
            unreachable!("nested paginators lay out columns");
        };
        Ok(ColumnOutput {
            anchors: self.anchors,
            overflow: self.root_overflow,
            string_locations: self.string_locations,
            pages: self.pages,
            geometric_baselines: self.geometric_baselines,
            display_lists: self.display_lists,
            foreground: self.foreground,
            underlays: self.underlays,
            flow_content: self.flow_content,
            content,
            clips: self.clips,
            sticky: self.sticky,
            sliced: self.sliced,
            pending,
            orders: self.orders.snapshot(),
            frame_requests: self.frame_requests,
            frame_reads: self.frame_reads,
            rows,
            markers_left: self.boxes.iter().any(|active| active.marker.is_some()),
        })
    }

    pub(super) fn complete_column_motion(&mut self) -> Result<(), LayoutError> {
        if let Some(node) = self.motion_column {
            let Fragmentainers::Columns(rows) = &self.fragmentainers else {
                unreachable!("column references belong to column paginators");
            };
            self.clips[node].areas = (0..self.pages.len())
                .map(|column| (column, rows.content_box(column)))
                .collect();
        }
        self.complete_column_overflow()
    }

    pub(super) fn for_frame(measurer: &'m mut Measurer<'t>, area: Rect) -> Self {
        let rows = ColumnRows::new(
            ColumnGeometry {
                count: 1,
                width: area.width,
                inline_size: area.width,
                gap: 0.0,
                x: area.x,
                rtl: false,
            },
            area.height,
        );
        let start = FlowStart {
            track_overflow: false,
            offset: [0.0; 2],
            key: Rc::from([FLOW_LEVEL]),
            context: Rc::from([]),
            orders: PaintOrders::default(),
            markers: Vec::new(),
        };
        Self::nested(measurer, rows, &start).unbroken()
    }

    /// Advances columns and spanners until the enclosing fragmentainer must change.
    pub(super) fn resume_columns(
        &mut self,
        cursor: &mut ColumnCursor<'a, 't>,
    ) -> Result<Option<BreakRequest>, LayoutError> {
        loop {
            if let Some((spanner, height)) = cursor.spanner.take() {
                if let Some((spanner, request)) = self.resume_flow(spanner)? {
                    cursor.spanner = Some((spanner, height));
                    return Ok(Some(request));
                }
                if let Some(remaining) = &mut cursor.remaining {
                    *remaining = (*remaining - height).max(0.0);
                }
            }
            if let Some((items, balance)) = cursor.entry.take() {
                cursor.row = Some(self.start_column_row(
                    &items,
                    if cursor.segments.len() == 0 {
                        cursor.tail
                    } else {
                        BreakTail::default()
                    },
                    cursor.geometry,
                    cursor.containing_height,
                    balance,
                    cursor.text_trim,
                )?);
            }
            if let Some(row) = cursor.row.take() {
                if let Some((row, request)) = self.column_fragment(
                    row,
                    cursor.geometry,
                    cursor.orthogonal,
                    cursor.rule,
                    &mut cursor.remaining,
                    cursor.wrap,
                )? {
                    cursor.row = Some(row);
                    if let Some(request) = request {
                        return Ok(Some(request));
                    }
                }
                continue;
            }
            let Some(segment) = cursor.segments.next() else {
                return Ok(None);
            };
            let last = cursor.segments.len() == 0;
            match segment {
                ColumnSegment::Spanner(items) => {
                    let height = items.iter().map(FlowItem::height).sum();
                    let tail = if last {
                        cursor.tail
                    } else {
                        BreakTail::default()
                    };
                    cursor.spanner = Some((FlowCursor::new(items, tail), height));
                }
                ColumnSegment::Row(items) => {
                    let balance = Balance::new(
                        cursor.fill,
                        !last,
                        cursor.remaining.is_some() || self.unbroken == 0,
                    );
                    let first = first_fragment(&items);
                    let reserve = self.continuation_reserve()?;
                    let tail = if last {
                        cursor.tail
                    } else {
                        BreakTail::default()
                    };
                    let required = first
                        + if first >= items.iter().map(FlowItem::height).sum::<f32>() {
                            self.closing_reserve(tail, reserve)?
                        } else {
                            reserve
                        };
                    let needs_break = self.unbroken == 0
                        && self.has_flow
                        && !overflows(required, self.height)
                        && overflows(self.y + self.pending_value() + required, self.height);
                    cursor.entry = Some((items, balance));
                    if needs_break {
                        return Ok(Some(BreakRequest::default()));
                    }
                }
            }
        }
    }

    fn start_column_row(
        &mut self,
        items: &[FlowItem<'a>],
        tail: BreakTail,
        geometry: ColumnGeometry,
        containing_height: Option<f32>,
        balance: Balance,
        text_trim: [bool; 2],
    ) -> Result<ColumnRow<'a, 't>, LayoutError> {
        self.flush_margin();
        let start = FlowStart {
            track_overflow: self.root_overflow.is_some()
                || self
                    .boxes
                    .last()
                    .is_some_and(|active| active.overflow.is_some()),
            offset: self.offset,
            key: self.key.clone(),
            context: self.context.clone(),
            orders: self.orders.snapshot(),
            markers: self
                .boxes
                .iter()
                .filter(|active| active.visible)
                .filter_map(|active| active.marker.as_deref().cloned())
                .collect(),
        };
        let mut rows = ColumnRows::new(
            geometry,
            if self.unbroken > 0 {
                f32::INFINITY
            } else {
                self.height - self.bottom_reserve
            },
        );
        rows.containing_height = containing_height;
        rows.text_trim = text_trim;
        Ok(ColumnRow {
            tail,
            layout: ColumnLayout::new(self.measurer, items, rows, &start)?,
            balance,
            placements: Vec::new(),
            last_page: None,
        })
    }

    /// Reserves one physical fragment, or one wrapped row; complete segment paint is adopted
    /// once. Returns the row to continue, with the break that precedes its next fragment, if
    /// any.
    fn column_fragment(
        &mut self,
        mut cursor: ColumnRow<'a, 't>,
        fallback: ColumnGeometry,
        orthogonal: bool,
        rule: Border,
        remaining: &mut Option<f32>,
        wrap: Option<WrapRows>,
    ) -> Result<Option<(ColumnRow<'a, 't>, Option<BreakRequest>)>, LayoutError> {
        let reserve = self.continuation_reserve()?;
        let closing_reserve = self.closing_reserve(cursor.tail, reserve)?;
        let gap = match wrap {
            Some(wrap) if cursor.last_page == Some(self.page) => wrap.gap,
            _ => 0.0,
        };
        let space = if self.unbroken > 0 {
            f32::INFINITY
        } else {
            (self.height - self.y - gap - closing_reserve).max(0.0)
        };
        let mut height = frame_height(*remaining, space, self.unbroken > 0);
        if let Some(wrap) = wrap {
            // A row of column boxes moves whole to the next page, where the page constrains
            // a taller one.
            if self.unbroken == 0 && self.has_flow && overflows(wrap.height, space) {
                return Ok(Some((cursor, Some(BreakRequest::default()))));
            }
            self.y += gap;
            height = (wrap.height.min(space), false);
        }
        let continuing_space = if self.unbroken > 0 {
            space
        } else {
            (self.height - self.y - reserve).max(0.0)
        };
        let geometry = self
            .boxes
            .last()
            .and_then(|active| active.geometry.as_ref())
            .map_or(fallback, |flow| {
                if orthogonal {
                    ColumnGeometry {
                        x: flow.horizontal.x,
                        ..fallback
                    }
                } else {
                    ColumnGeometry::resolve(
                        flow.source.style(),
                        flow.horizontal.x,
                        flow.horizontal.width,
                    )
                }
            });
        let frame = RowFrame {
            geometry,
            first: 0,
            offset: self.offset,
            height: height.0,
            reference_height: height.0,
            unbounded: height.1,
            page_rect: Rect {
                y: self.page_rect.y - self.top - self.y,
                ..self.page_rect
            },
            breaks_page: self.fragmentainers.breaks_page(self.page)
                && wrap.is_none_or(|wrap| {
                    overflows(self.y + height.0 + wrap.gap + wrap.height, space + self.y)
                }),
            physical_page: self.physical_page(),
            follows_flow: self.has_page_flow(),
        };
        let fixed = remaining.is_some() || wrap.is_some();
        let mut checkpoint = (wrap.is_none()
            && continuing_space > space + FRAGMENTATION_TOLERANCE_PT)
            .then(|| cursor.layout.snapshot(&self.measurer.work))
            .transpose()?;
        let row = cursor.layout.push_frame(frame);
        let (mut layout, mut end) = lay_out_row(self.measurer, cursor.layout, row, cursor.balance)?;
        let used = layout.used_height(row, height.0, fixed, cursor.balance, end);
        if end == RowEnd::Content
            && overflows(used, space)
            && self.has_flow
            && !overflows(
                self.repeated_start(self.page, self.boxes.len()) + used + closing_reserve,
                self.height,
            )
            && let Some(checkpoint) = checkpoint.take()
        {
            cursor.layout = checkpoint;
            return Ok(Some((cursor, Some(BreakRequest::default()))));
        }
        if end == RowEnd::Overflow
            && let Some(mut checkpoint) = checkpoint
        {
            let continuing_height = frame_height(*remaining, continuing_space, self.unbroken > 0);
            checkpoint.push_frame(RowFrame {
                height: continuing_height.0,
                unbounded: continuing_height.1,
                ..frame
            });
            let (continuing, continuing_end) =
                lay_out_row(self.measurer, checkpoint, row, cursor.balance)?;
            let continuing_used = continuing.used_height(
                row,
                continuing_height.0,
                remaining.is_some(),
                cursor.balance,
                continuing_end,
            );
            // A final row must still leave room for its closing edges and repeated footer.
            if continuing_end != RowEnd::Content || !overflows(continuing_used, space) {
                height = continuing_height;
                layout = continuing;
                end = continuing_end;
            }
        }
        cursor.layout = layout;
        let used = cursor
            .layout
            .used_height(row, height.0, fixed, cursor.balance, end);
        cursor.layout.rows_mut().frames[row].reference_height = used;
        cursor.placements.push(RowPlacement {
            position: self.flow_position(),
            used,
        });
        self.y += used;
        if used > 0.0 {
            self.has_flow = true;
            self.at_page_start = false;
            self.flow_content[self.page] = true;
        }
        if let Some(remaining) = remaining {
            *remaining = (*remaining - used).max(0.0);
        }
        let forced = match end {
            RowEnd::Content => {
                let final_position = self.flow_position();
                let mut output = cursor.layout.into_output(self.measurer)?;
                let mut placements = Vec::new();
                for (row, placement) in cursor.placements.into_iter().enumerate() {
                    self.measurer.work.spend(1)?;
                    self.restore_flow_position(placement.position)?;
                    self.place_columns(&mut output, row, placement.used, rule, &mut placements);
                }
                self.restore_flow_position(final_position)?;
                self.finish_columns(output, &placements);
                return Ok(None);
            }
            RowEnd::Overflow if wrap.is_some() => {
                cursor.last_page = Some(self.page);
                return Ok(Some((cursor, None)));
            }
            RowEnd::Overflow => None,
            RowEnd::Forced(forced) => Some(forced),
        };
        Ok(Some((
            cursor,
            Some(BreakRequest {
                forced,
                preserve_empty: true,
            }),
        )))
    }

    /// Places the columns of `row`, with their rules, at the current flow position.
    fn place_columns(
        &mut self,
        output: &mut ColumnOutput<'a>,
        row: usize,
        used: f32,
        rule: Border,
        placements: &mut Vec<(usize, usize)>,
    ) {
        let page = self.page;
        let top = self.top + self.y;
        let geometry = output.rows.geometry(row);
        let columns = output.rows.pages(row, output.pages.len());
        let first = columns.start;
        if rule.used_width() > 0.0 && used > 0.0 {
            let scope = self
                .boxes
                .last()
                .map_or_else(|| self.key.clone(), |active| active.scope.clone());
            for column in columns.clone().skip(1) {
                if output.content[column - 1] && output.content[column] {
                    let x = geometry.rule_x(column - 1 - first) + self.offset[0];
                    self.display_lists[page].push(ScopedPaint {
                        native: None,
                        visible: self.boxes.last().is_none_or(|active| active.visible),
                        order: self.orders.take(),
                        scope: scope.clone(),
                        clip: self.clip,
                        group: self.group,
                        ink: None,
                        item: DisplayItem::Box(rule_paint(rule, x, top + self.offset[1], used)),
                        navigation: crate::document::BoxNavigation::default(),
                    });
                }
            }
        }
        let mut effects = HashMap::new();
        for column in columns {
            self.string_locations
                .adopt_column(&mut output.string_locations, column, page);
            let shift = [geometry.shift(column - first), top];
            if let Some(bounds) = output
                .overflow
                .as_ref()
                .and_then(|bounds| bounds.get(&column))
            {
                self.record_overflow(Rect {
                    x: bounds.x + shift[0],
                    y: bounds.y + shift[1],
                    ..*bounds
                });
            }
            self.record_baselines(
                page,
                output.geometric_baselines[column].map(|baseline| baseline.map(|y| y + shift[1])),
            );
            let lines = self.pages[page].len();
            let slots = self.display_lists[page].len();
            if placements.len() <= column {
                placements.resize(column + 1, (page, slots));
            }
            placements[column] = (page, slots);
            let native_lines: std::collections::HashSet<_> = output.display_lists[column]
                .iter()
                .chain(&output.foreground[column])
                .filter_map(|paint| match paint.item {
                    DisplayItem::Text { line } if paint.native.is_some() => Some(line),
                    _ => None,
                })
                .collect();
            for (index, mut line) in std::mem::take(&mut output.pages[column])
                .into_iter()
                .enumerate()
            {
                if !native_lines.contains(&index) {
                    line.x += shift[0];
                    line.baseline += shift[1];
                }
                self.pages[page].push(line);
            }
            let place = Placement {
                column,
                page,
                shift,
                lines,
                slots,
                native: None,
            };
            for paint in std::mem::take(&mut output.display_lists[column]) {
                let paint = self.adopt(paint, &output.clips, &output.sticky, &place, &mut effects);
                self.display_lists[page].push(paint);
            }
            for paint in std::mem::take(&mut output.foreground[column]) {
                let paint = self.adopt(paint, &output.clips, &output.sticky, &place, &mut effects);
                self.foreground[page].push(paint);
            }
            let mut anchors = std::mem::take(&mut self.anchors);
            anchors.adopt(
                &mut output.anchors,
                (column, page, slots),
                |rect| Rect {
                    x: rect.x + shift[0],
                    y: rect.y + shift[1],
                    ..rect
                },
                |clip| {
                    clip.map(|clip| {
                        self.effect(&output.clips, &output.sticky, clip, &place, &mut effects)
                    })
                    .or(self.clip)
                },
            );
            self.anchors = anchors;
            self.adopt_underlays(
                &mut output.underlays,
                &output.clips,
                &output.sticky,
                &place,
                &mut effects,
            );
            self.frame_requests.append(&mut output.frame_requests);
            self.frame_reads.append(&mut output.frame_reads);
            self.flow_content[page] |= output.flow_content[column];
            let (anchored, others): (Vec<_>, Vec<_>) = std::mem::take(&mut output.pending)
                .into_iter()
                .partition(|pending| pending.position.page == column);
            output.pending = others;
            for mut pending in anchored {
                pending.position.page = page;
                pending.position.left += shift[0];
                pending.position.right += shift[0];
                pending.position.y += shift[1];
                pending.position.bottom += shift[1];
                pending.group = match pending.group {
                    Some(group) => Some(self.effect(
                        &output.clips,
                        &output.sticky,
                        group,
                        &place,
                        &mut effects,
                    )),
                    None => self.group,
                };
                pending.parent_3d = pending.parent_3d.map(|node| {
                    self.effect(&output.clips, &output.sticky, node, &place, &mut effects)
                });
                self.enqueue(pending);
            }
        }
        self.orders.commit(&output.orders);
        self.y += used;
        if used > 0.0 {
            self.has_flow = true;
            self.at_page_start = false;
        }
        self.page_content = self.current_page_has_ink();
    }

    /// Adopts the paint state the last row left behind once the segment ends.
    fn finish_columns(&mut self, output: ColumnOutput<'a>, placements: &[(usize, usize)]) {
        for group in output.sliced {
            self.register_slice(
                group.style,
                group
                    .fragments
                    .iter()
                    .map(|&(column, slot)| {
                        let (page, slots) = placements[column];
                        (page, slots + slot)
                    })
                    .collect(),
            );
        }
        if !output.markers_left {
            for active in &mut self.boxes {
                active.marker = None;
            }
        }
    }
}

/// The column height of the next row and whether the container's definite height ends in it.
fn frame_height(remaining: Option<f32>, space: f32, unbroken: bool) -> (f32, bool) {
    match remaining {
        Some(remaining) if !overflows(remaining, space) => (remaining, true),
        Some(_) => (space, false),
        None => (space, unbroken),
    }
}

/// Balances only the uncommitted row, retaining its resulting source checkpoint.
fn lay_out_row<'a: 't, 't: 'a>(
    measurer: &mut Measurer<'t>,
    mut checkpoint: ColumnLayout<'a, 't>,
    row: usize,
    balance: Balance,
) -> Result<(ColumnLayout<'a, 't>, RowEnd), LayoutError> {
    if !balance.last && !balance.forced {
        return checkpoint.through_row(measurer, row);
    }
    let (output, end) = checkpoint
        .snapshot(&measurer.work)?
        .through_row(measurer, row)?;
    let balanced = match end {
        RowEnd::Content => balance.last,
        RowEnd::Forced(_) => balance.forced,
        RowEnd::Overflow => false,
    };
    if !balanced || !output.fits(row, end) {
        return Ok((output, end));
    }
    let mut high = output.extent(row);
    checkpoint.set_row_height(row, high);
    let (mut best, high_end) = checkpoint
        .snapshot(&measurer.work)?
        .probe_row(measurer, row)?;
    if high_end != end || !best.fits(row, high_end) {
        return Ok((output, end));
    }
    let mut low = 0.0;
    while high - low > FRAGMENTATION_TOLERANCE_PT {
        let middle = low + (high - low) / 2.0;
        if middle == low || middle == high {
            break;
        }
        checkpoint.set_row_height(row, middle);
        let (trial, trial_end) = checkpoint
            .snapshot(&measurer.work)?
            .probe_row(measurer, row)?;
        if trial_end == end && trial.fits(row, trial_end) {
            high = middle;
            best = trial;
        } else {
            low = middle;
        }
    }
    Ok((best, end))
}

/// The height and first and last baselines of a multi-column container's content laid out
/// from the top of an empty page, or unbroken.
pub(crate) fn measure_columns<'a>(
    measurer: &mut Measurer<'a>,
    columns: &MeasuredColumns<'a>,
) -> Result<crate::layout::multicol::ColumnMeasurement, LayoutError> {
    let previous = std::mem::replace(&mut measurer.column_measurement, true);
    let result = (|| {
        let available = if measurer.unbroken {
            f32::INFINITY
        } else {
            measurer.page_height
        };
        let start = FlowStart {
            track_overflow: false,
            offset: [0.0; 2],
            key: Rc::from([FLOW_LEVEL]),
            context: Rc::from([]),
            orders: PaintOrders::default(),
            markers: Vec::new(),
        };
        let mut remaining = columns.definite;
        let mut height = 0.0;
        let mut baselines = [None; 2];
        let mut used_inline = 0.0f32;
        let mut content_height = 0.0;
        let inline_extent = |geometry: ColumnGeometry, count: usize| {
            if count == 0 {
                0.0
            } else {
                geometry.width + count.saturating_sub(1) as f32 * (geometry.width + geometry.gap)
            }
        };
        let count = columns.segments.len();
        for (index, segment) in columns.segments.iter().enumerate() {
            match segment {
                ColumnSegment::Spanner(items) => {
                    baselines[0] = baselines[0].or(first_baseline(items).map(|y| y + height));
                    if let Some(baseline) = last_baseline(items) {
                        baselines[1] = Some(baseline + height);
                    }
                    let spanner: f32 = items.iter().map(FlowItem::height).sum();
                    height += spanner;
                    content_height += spanner;
                    used_inline =
                        used_inline.max(inline_extent(columns.geometry, columns.geometry.count));
                    if let Some(remaining) = &mut remaining {
                        *remaining = (*remaining - spanner).max(0.0);
                    }
                }
                ColumnSegment::Row(items) => {
                    let balance = Balance::new(
                        columns.fill,
                        index + 1 < count,
                        columns.definite.is_some() || !measurer.unbroken,
                    );
                    let mut rows = ColumnRows::new(columns.geometry, available);
                    rows.containing_height = columns.definite;
                    rows.text_trim = columns.text_trim;
                    let mut layout = ColumnLayout::new(measurer, items, rows, &start)?;
                    let mut wrapped = false;
                    loop {
                        let frame = match columns.wrap {
                            Some(wrap) => {
                                if wrapped {
                                    height += wrap.gap;
                                }
                                (wrap.height.min(available), false)
                            }
                            None => frame_height(remaining, available, measurer.unbroken),
                        };
                        wrapped = true;
                        let row = layout.push_frame(RowFrame {
                            geometry: columns.geometry,
                            first: 0,
                            offset: [0.0; 2],
                            height: frame.0,
                            reference_height: frame.0,
                            unbounded: frame.1,
                            page_rect: UNPLACED_PAGE,
                            breaks_page: measurer.fragmentainer
                                == crate::layout::breaks::Fragmentainer::Page
                                && columns.wrap.is_none_or(|wrap| {
                                    overflows(height + frame.0 + wrap.gap + wrap.height, available)
                                }),
                            // Measurement places columns on no page.
                            physical_page: None,
                            follows_flow: false,
                        });
                        let (output, end) = lay_out_row(measurer, layout, row, balance)?;
                        let pages = output.rows().pages(row, output.state.pages.len());
                        used_inline = used_inline
                            .max(inline_extent(output.rows().geometry(row), pages.len()));
                        content_height += output.extent(row);
                        let first = pages
                            .clone()
                            .find_map(|page| output.state.geometric_baselines[page][0]);
                        baselines[0] = baselines[0].or(first.map(|y| y + height));
                        if let Some(last) = pages
                            .rev()
                            .find_map(|page| output.state.geometric_baselines[page][1])
                        {
                            baselines[1] = Some(last + height);
                        }
                        let used = output.used_height(
                            row,
                            frame.0,
                            columns.definite.is_some() || columns.wrap.is_some(),
                            balance,
                            end,
                        );
                        height += used;
                        if let Some(remaining) = &mut remaining {
                            *remaining = (*remaining - used).max(0.0);
                        }
                        layout = output;
                        if end == RowEnd::Content {
                            break;
                        }
                    }
                }
            }
        }
        Ok(crate::layout::multicol::ColumnMeasurement {
            height,
            baselines,
            used_inline,
            content_height,
        })
    })();
    measurer.column_measurement = previous;
    result
}
