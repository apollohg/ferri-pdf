//! CSS Multi-column Layout 1: column geometry, spanners and measured column flows. Columns
//! are laid out as fragmentainers by the paginator (`pagination::columns`).
use crate::graphics::Border;
use crate::layout::measure::Failure;
use crate::layout::{
    breaks::{Fragmentainer, first_fragment},
    margin::MarginStrut,
    measure::{BlockEdge, FlowItem, MeasuredBlock, Measurer},
};
use crate::style::{BreakBetween, ColumnFill, Style};
use crate::text::IntrinsicWidths;

/// Bounds the column boxes of one row, as a zero column width and gap would otherwise
/// request unboundedly many.
pub(super) const MAX_COLUMN_COUNT: usize = 1000;

/// Used column count, width and gap (CSS Multi-column Layout 1 §3.4).
#[derive(Clone, Copy, Debug)]
pub(super) struct ColumnGeometry {
    pub count: usize,
    pub width: f32,
    pub inline_size: f32,
    pub gap: f32,
    /// Left content edge of the multi-column container.
    pub x: f32,
    pub rtl: bool,
}
impl ColumnGeometry {
    /// The §3.4 pseudo-algorithm over the container's content width `available`.
    pub(crate) fn resolve(style: &Style, x: f32, available: f32) -> Self {
        let gap = style.flex.gap[1].resolve_normal(available, style.size);
        let fitting = |width: f32| {
            let count = ((available + gap) / (width + gap)).floor();
            if count.is_finite() {
                (count.max(1.0) as usize).min(MAX_COLUMN_COUNT)
            } else {
                MAX_COLUMN_COUNT
            }
        };
        let specified = specified_count(style);
        let count = match (style.multicol.width, specified) {
            (None, Some(count)) => count,
            (Some(width), None) => fitting(width),
            (Some(width), Some(count)) => count.min(fitting(width)),
            (None, None) => 1,
        };
        Self {
            count,
            width: ((available + gap) / count as f32 - gap).max(0.0),
            inline_size: available,
            gap,
            x,
            rtl: style.used_rtl(),
        }
    }
    /// Left edge of column `index` in the inline direction; overflow columns continue past
    /// the container's inline end.
    pub(crate) fn left(&self, index: usize) -> f32 {
        let pitch = self.width + self.gap;
        if self.rtl {
            self.x + self.inline_size - self.width - index as f32 * pitch
        } else {
            self.x + index as f32 * pitch
        }
    }
    /// Horizontal offset of column `index` from column 0, where content is measured.
    pub(crate) fn shift(&self, index: usize) -> f32 {
        self.left(index) - self.left(0)
    }
    /// The middle of the gap between columns `index` and `index + 1`.
    pub(crate) fn rule_x(&self, index: usize) -> f32 {
        let [first, second] = [self.left(index), self.left(index + 1)];
        (first.min(second) + self.width + first.max(second)) / 2.0
    }
}

#[derive(Clone, Copy)]
pub(super) struct OrthogonalColumns {
    pub source: *const crate::document::BlockBox,
    pub geometry: ColumnGeometry,
    pub length: Option<f32>,
    pub principal: bool,
    pub block_auto: bool,
    pub content_max: f32,
    pub minimum: f32,
    pub maximum: Option<f32>,
}
impl OrthogonalColumns {
    pub(super) fn matches(self, block: &crate::document::BlockBox) -> bool {
        std::ptr::eq(self.source, block)
    }
    pub(super) fn constrain(self, value: f32) -> f32 {
        self.maximum
            .map_or(value, |maximum| value.min(maximum))
            .max(self.minimum)
    }
    pub(super) fn adjusted(mut self, columns: &MeasuredColumns<'_>, style: &Style) -> Option<Self> {
        let pitch = self.geometry.width + self.geometry.gap;
        let used = if pitch > 0.0 {
            ((columns.used_inline + self.geometry.gap) / pitch)
                .round()
                .max(1.0) as usize
        } else {
            1
        };
        if used == 1 && !columns.wraps {
            let preferred = self.constrain(self.content_max);
            let inline = if self.principal {
                preferred
            } else {
                self.geometry.inline_size
            };
            if self.geometry.width != preferred || self.geometry.inline_size != inline {
                self.geometry.width = preferred;
                self.geometry.inline_size = inline;
                self.geometry.gap = style.flex.gap[1].resolve_normal(inline, style.size);
                return Some(self);
            }
        }
        if style.multicol.count.is_none() && used > self.geometry.count {
            self.geometry.count = used;
            if self.principal {
                let gap = style.flex.gap[1].resolve_normal(0.0, style.size);
                self.geometry.inline_size = self.constrain(
                    self.geometry.width * used as f32 + gap * used.saturating_sub(1) as f32,
                );
            }
            self.geometry.gap =
                style.flex.gap[1].resolve_normal(self.geometry.inline_size, style.size);
            return Some(self);
        }
        if used == 1
            && self.block_auto
            && self.length.is_some_and(|length| {
                super::pagination::overflows(length, columns.content_height)
                    || super::pagination::overflows(columns.content_height, length)
            })
        {
            self.length = Some(columns.content_height);
            return Some(self);
        }
        None
    }
}

pub(super) fn specified_count(style: &Style) -> Option<usize> {
    style.multicol.count.map(|count| {
        usize::try_from(count).map_or(MAX_COLUMN_COUNT, |count| count.min(MAX_COLUMN_COUNT))
    })
}

pub(super) fn orthogonal_width(style: &Style, widths: IntrinsicWidths, available: f32) -> f32 {
    style.multicol.width.unwrap_or_else(|| {
        let gap = style.flex.gap[1].resolve_normal(0.0, style.size);
        let count = specified_count(style).unwrap_or(1);
        widths
            .max
            .min((available - gap * count.saturating_sub(1) as f32).max(0.0))
            .max(widths.min)
    })
}

/// The multi-column container whose column flow is being measured: in-flow block-level
/// descendants in its formatting context with `column-span: all` span it (§6.1).
#[derive(Clone, Copy)]
pub(super) struct ColumnFlow {
    pub x: f32,
    pub width: f32,
    pub rtl: bool,
    pub containing_height: Option<f32>,
    /// The fragmentainer the container itself, and so each spanner, breaks into.
    pub fragmentainer: Fragmentainer,
}

/// Source edges and remaining used size of an ancestor split by column spanners.
#[derive(Clone, Copy)]
pub(super) struct SpannerFragment {
    pub first: bool,
    pub last: bool,
    pub natural_height: f32,
    pub remaining: f64,
}

impl SpannerFragment {
    pub(crate) fn cloned_extent(
        &self,
        style: &Style,
        geometry: &crate::layout::geometry::HorizontalBox,
    ) -> f32 {
        if !style.clone_decoration {
            return 0.0;
        }
        let start = if self.first {
            0.0
        } else {
            geometry.padding[0] + geometry.border[0]
        };
        let end = if self.last {
            0.0
        } else {
            geometry.padding[2] + geometry.border[2]
        };
        start + end
    }
    pub(crate) fn resolve_size(
        &self,
        style: &Style,
        geometry: &crate::layout::geometry::HorizontalBox,
        vertical: &crate::layout::geometry::VerticalBox,
        natural: f32,
    ) -> (f32, bool) {
        let full_height = vertical.used_height(self.natural_height);
        let constrained = full_height != self.natural_height || vertical.definite().is_some();
        let remaining = (f64::from(full_height) * self.remaining) as f32;
        let cloned = self.cloned_extent(style, geometry);
        let height = if constrained {
            (if self.last {
                remaining
            } else {
                (natural - cloned).max(0.0).min(remaining)
            }) + cloned
        } else {
            natural
        };
        (height, constrained)
    }
}

/// The fixed rows of column boxes a multi-column container's overflowing content wraps into
/// in the block direction (CSS Multi-column Layout 2 §4.4).
#[derive(Clone, Copy, Debug)]
pub(super) struct WrapRows {
    /// The column height: `column-height`, or else the container's definite height.
    pub height: f32,
    /// The row gap between consecutive rows.
    pub gap: f32,
}
impl WrapRows {
    pub(super) fn of(style: &Style, definite: Option<f32>) -> Option<Self> {
        style.multicol.wraps().then_some(())?;
        Some(Self {
            height: style.multicol.height.or(definite)?,
            gap: style.flex.gap[0].resolve_normal(definite.unwrap_or(0.0), style.size),
        })
    }
}

/// Content between spanners, laid out in a row of columns, or a spanner with its margins.
#[derive(Clone)]
pub(super) enum ColumnSegment<'a> {
    Row(Vec<FlowItem<'a>>),
    Spanner(Vec<FlowItem<'a>>),
}

pub(super) struct ColumnMeasurement {
    pub height: f32,
    pub baselines: [Option<f32>; 2],
    pub used_inline: f32,
    pub content_height: f32,
}

#[derive(Clone)]
pub(super) struct MeasuredColumns<'a> {
    pub orthogonal: bool,
    pub wraps: bool,
    pub geometry: ColumnGeometry,
    pub rule: Border,
    pub fill: ColumnFill,
    /// A definite content height fixes the column height; columns beyond the count then
    /// continue in the inline direction (§8.2).
    pub definite: Option<f32>,
    pub segments: Vec<ColumnSegment<'a>>,
    /// Height laid out from the top of an empty page, or unbroken.
    pub height: f32,
    pub used_inline: f32,
    pub content_height: f32,
    pub first_fragment: f32,
    /// First and last baselines from the top of the columns.
    pub baselines: [Option<f32>; 2],
    /// Page breaks at the start and end of the content apply to the container too.
    pub break_before: BreakBetween,
    pub break_after: BreakBetween,
    /// The content holds a forced page break.
    pub forces_page: bool,
    /// The sides of each column's first and last formatted lines that are trimmed.
    pub text_trim: [bool; 2],
    /// Rows the content wraps into instead of overflow columns.
    pub wrap: Option<WrapRows>,
}

/// Measures a multi-column container's content flow, already measured at the column width.
pub(super) fn measure<'a>(
    measurer: &mut Measurer<'a>,
    geometry: ColumnGeometry,
    style: &Style,
    content: Vec<FlowItem<'a>>,
    definite: Option<f32>,
    orthogonal: bool,
    text_trim: [bool; 2],
) -> Result<MeasuredColumns<'a>, Failure> {
    let forces_page = forces_page(&content);
    let wraps = wraps(&content);
    let edge = |item: Option<&FlowItem<'_>>, after: bool| match item {
        Some(FlowItem::Block(block) | FlowItem::Spanner(block)) => {
            let value = if after {
                block.break_after
            } else {
                block.break_before
            };
            if value == BreakBetween::Page {
                value
            } else {
                BreakBetween::Auto
            }
        }
        _ => BreakBetween::Auto,
    };
    let is_flow = |item: &&FlowItem<'_>| {
        !matches!(
            item,
            FlowItem::Margin(_) | FlowItem::Space(_) | FlowItem::Edge(_, _)
        )
    };
    let break_before = edge(content.iter().find(is_flow), false);
    let break_after = edge(content.iter().rev().find(is_flow), true);
    let segments = segments(hoist_spanners(content));
    let first_fragment = segments.first().map_or(0.0, |segment| match segment {
        ColumnSegment::Row(items) | ColumnSegment::Spanner(items) => first_fragment(items),
    });
    let wrap = WrapRows::of(style, definite);
    // Without wrapping, a `column-height` fixes the one row's height as a definite height
    // does, and later columns overflow in the inline direction (§4.2).
    let definite = if wrap.is_some() {
        definite
    } else {
        style.multicol.height.or(definite)
    };
    let mut columns = MeasuredColumns {
        orthogonal,
        wraps,
        geometry,
        rule: style.multicol.rule,
        fill: style.multicol.fill,
        definite,
        segments,
        height: 0.0,
        used_inline: geometry.width,
        content_height: 0.0,
        first_fragment,
        baselines: [None; 2],
        break_before,
        break_after,
        forces_page,
        text_trim,
        wrap,
    };
    let measurement = crate::layout::pagination::measure_columns(measurer, &columns)?;
    columns.height = measurement.height;
    columns.baselines = measurement.baselines;
    columns.used_inline = measurement.used_inline;
    columns.content_height = measurement.content_height;
    Ok(columns)
}

fn wraps(items: &[FlowItem<'_>]) -> bool {
    enum Entry<'b, 'a> {
        Items(&'b [FlowItem<'a>]),
        Block(&'b MeasuredBlock<'a>),
    }
    let mut pending = vec![Entry::Items(items)];
    while let Some(entry) = pending.pop() {
        let items = match entry {
            Entry::Items(items) => items,
            Entry::Block(block) => {
                if let Some(table) = &block.table {
                    pending.extend(
                        table
                            .header
                            .iter()
                            .chain(&table.footer)
                            .flat_map(|row| &row.cells)
                            .map(|cell| Entry::Block(&cell.block)),
                    );
                }
                &block.items
            }
        };
        for item in items {
            match item {
                FlowItem::Paragraph(paragraph)
                    if paragraph.lines.iter().any(|line| line.wrapped) =>
                {
                    return true;
                }
                FlowItem::Block(block) | FlowItem::Spanner(block) => {
                    pending.push(Entry::Block(block));
                }
                FlowItem::Row(row) => {
                    pending.extend(row.cells.iter().map(|cell| Entry::Block(&cell.block)));
                }
                FlowItem::Columns(columns) => {
                    pending.extend(columns.segments.iter().map(|segment| {
                        let (ColumnSegment::Row(items) | ColumnSegment::Spanner(items)) = segment;
                        Entry::Items(items)
                    }));
                }
                _ => {}
            }
        }
    }
    false
}

/// Whether the flow, or a box in it, forces a page break.
fn forces_page(items: &[FlowItem<'_>]) -> bool {
    items.iter().any(|item| match item {
        FlowItem::Block(block) | FlowItem::Spanner(block) => {
            block.break_before == BreakBetween::Page
                || block.break_after == BreakBetween::Page
                || forces_page(&block.items)
        }
        FlowItem::Row(row) => {
            row.break_before == BreakBetween::Page
                || row.break_after == BreakBetween::Page
                || row.cells.iter().any(|cell| forces_page(&cell.block.items))
        }
        FlowItem::Columns(columns) => columns.forces_page,
        FlowItem::TableBox(table) => table.edges.iter().any(|edge| {
            crate::layout::breaks::Forced::of(*edge)
                .is_some_and(crate::layout::breaks::Forced::is_page)
        }),
        FlowItem::Frame { .. }
        | FlowItem::Image { .. }
        | FlowItem::Paragraph(_)
        | FlowItem::Margin(_)
        | FlowItem::Space(_)
        | FlowItem::Edge(_, _)
        | FlowItem::Positioned(_)
        | FlowItem::Events(_) => false,
    })
}

fn holds_spanner(items: &[FlowItem<'_>]) -> bool {
    items.iter().any(|item| match item {
        FlowItem::Spanner(_) => true,
        FlowItem::Block(block) => holds_spanner(&block.items),
        _ => false,
    })
}

/// Lifts spanners out of the boxes holding them, splitting each such box into fragments
/// before and after its spanners (§6.1).
fn hoist_spanners(items: Vec<FlowItem<'_>>) -> Vec<FlowItem<'_>> {
    let mut hoisted = Vec::with_capacity(items.len());
    for item in items {
        match item {
            FlowItem::Block(block) if holds_spanner(&block.items) => {
                hoisted.extend(split(*block));
            }
            item => hoisted.push(item),
        }
    }
    hoisted
}

/// A box split by the spanners in its flow: its fragments with the spanners, and the margins
/// adjoining them, between them.
fn split(mut block: MeasuredBlock<'_>) -> Vec<FlowItem<'_>> {
    let clone = block.style.clone_decoration;
    let mut pieces = vec![Vec::new()];
    let mut groups = Vec::new();
    let mut items = hoist_spanners(std::mem::take(&mut block.items))
        .into_iter()
        .peekable();
    while let Some(item) = items.next() {
        let piece = pieces.last_mut().expect("a piece is open");
        let FlowItem::Spanner(spanner) = item else {
            piece.push(item);
            continue;
        };
        let mut group = Vec::new();
        if matches!(piece.last(), Some(FlowItem::Margin(_))) {
            group.extend(piece.pop());
        }
        group.push(FlowItem::Spanner(spanner));
        if matches!(items.peek(), Some(FlowItem::Margin(_))) {
            group.extend(items.next());
        }
        groups.push(group);
        pieces.push(Vec::new());
    }
    let natural_height = pieces
        .iter()
        .flatten()
        .map(FlowItem::height)
        .sum::<f32>()
        .max(0.0);
    let sizing = block
        .sizing
        .as_mut()
        .expect("spanner ancestors retain flow sizing");
    sizing.natural_height = natural_height;
    block.height = sizing
        .resolve(block.style, &block.geometry, block.containing_height, None)
        .0;
    let count = pieces.len();
    let mut consumed = 0.0;
    let mut output = Vec::new();
    let mut groups = groups.into_iter();
    for (index, mut items) in pieces.into_iter().enumerate() {
        let first = index == 0;
        let last = index + 1 == count;
        if !first && clone {
            items.insert(0, FlowItem::Edge(BlockEdge::Start, block.start_space));
        }
        if !last && clone {
            items.push(FlowItem::Edge(BlockEdge::End, block.end_space));
        }
        if let Some(part) = fragment(&block, items, first, last, natural_height, &mut consumed) {
            output.push(FlowItem::Block(Box::new(part)));
        }
        output.extend(groups.next().into_iter().flatten());
    }
    output
}

/// One fragment of a box split by spanners, or `None` when it holds nothing.
fn fragment<'a>(
    source: &MeasuredBlock<'a>,
    items: Vec<FlowItem<'a>>,
    first: bool,
    last: bool,
    natural_height: f32,
    consumed: &mut f32,
) -> Option<MeasuredBlock<'a>> {
    let natural: f32 = items.iter().map(FlowItem::height).sum();
    let holds_content = items.iter().any(|item| {
        !matches!(
            item,
            FlowItem::Margin(_) | FlowItem::Space(_) | FlowItem::Edge(_, _)
        )
    });
    if !(natural != 0.0 || holds_content || last || first && source.marker.is_some()) {
        return None;
    }
    // Remaining used size follows source progress (CSS Fragmentation 3 §5.1).
    let remaining = if source.height > 0.0 {
        f64::from((source.height - *consumed).max(0.0)) / f64::from(source.height)
    } else {
        1.0
    };
    let fragment = SpannerFragment {
        first,
        last,
        natural_height,
        remaining,
    };
    let mut sizing = source.sizing.expect("spanner ancestors retain flow sizing");
    sizing.natural_height = natural.max(0.0);
    let (height, constrained) = sizing.resolve(
        source.style,
        &source.geometry,
        source.containing_height,
        Some(fragment),
    );
    *consumed += (height - fragment.cloned_extent(source.style, &source.geometry)).max(0.0);
    let overflow_height = crate::layout::measure::overflow_height(&items).max(height);
    let first_fragment = first_fragment(&items).min(overflow_height);
    let mut paint = source.paint.clone();
    if let Some(paint) = &mut paint {
        paint.first_fragment = first || source.style.clone_decoration;
    }
    Some(MeasuredBlock {
        frame_key: None,
        frame_owner: None,
        math: None,
        clearance_before: None,
        clearance_target: source.clearance_target,
        before: if first {
            source.before.clone()
        } else {
            MarginStrut::default()
        },
        after: if last {
            source.after.clone()
        } else {
            MarginStrut::default()
        },
        through: false,
        height,
        sizing: Some(sizing),
        constrained,
        overflow_height,
        first_fragment,
        break_before: if first {
            source.break_before
        } else {
            BreakBetween::Auto
        },
        break_after: if last {
            source.break_after
        } else {
            BreakBetween::Auto
        },
        items,
        paint,
        end_space: if last || source.style.clone_decoration {
            source.end_space
        } else {
            0.0
        },
        continues: !last,
        spanner_fragment: Some(fragment),
        line_fragments: Vec::new(),
        marker: if first { source.marker.clone() } else { None },
        ..source.clone()
    })
}

/// Splits the hoisted column flow into rows of columns and spanners; margins adjoining a
/// spanner collapse with its own outside the rows.
fn segments(items: Vec<FlowItem<'_>>) -> Vec<ColumnSegment<'_>> {
    let mut segments = Vec::new();
    let mut row = Vec::new();
    let mut items = items.into_iter().peekable();
    while let Some(item) = items.next() {
        match item {
            FlowItem::Spanner(spanner) => {
                let mut before = MarginStrut::default();
                while let Some(FlowItem::Margin(value)) = row.last() {
                    let mut earlier = value.clone();
                    earlier.merge(before);
                    before = earlier;
                    row.pop();
                }
                if !row.is_empty() {
                    segments.push(ColumnSegment::Row(std::mem::take(&mut row)));
                }
                let mut after = MarginStrut::default();
                while let Some(FlowItem::Margin(value)) = items.peek() {
                    after.merge(value.clone());
                    items.next();
                }
                segments.push(ColumnSegment::Spanner(vec![
                    FlowItem::Margin(before),
                    FlowItem::Spanner(spanner),
                    FlowItem::Margin(after),
                ]));
            }
            item => row.push(item),
        }
    }
    if !row.is_empty() || segments.is_empty() {
        segments.push(ColumnSegment::Row(row));
    }
    segments
}

/// Content-box min- and max-content widths of a multi-column container from those of its
/// content: a column at least `column-width` wide, repeated `column-count` times with gaps.
pub(super) fn intrinsic(style: &Style, mut column: IntrinsicWidths) -> IntrinsicWidths {
    if let Some(width) = style.multicol.width {
        column.min = column.min.max(width);
        column.max = column.max.max(width);
    }
    let count = style.multicol.count.map_or(1, |count| {
        usize::try_from(count).map_or(MAX_COLUMN_COUNT, |count| count.min(MAX_COLUMN_COUNT))
    });
    let gap = style.flex.gap[1].resolve_normal(0.0, style.size);
    let gaps = gap * (count - 1) as f32;
    IntrinsicWidths {
        min: column.min * count as f32 + gaps,
        max: column.max * count as f32 + gaps,
    }
}
