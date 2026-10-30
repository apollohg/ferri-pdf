use super::{
    captions,
    collapse::CollapsedGrid,
    model::Data,
    paint::{Areas, Columns, FragmentBackground, Layers},
};
use crate::document::{BlockBox, BlockContent, SemanticEvent};
use crate::graphics::{Border, BorderStyle, BoxDecoration, Color, Rect};
use crate::layout::measure::{
    BlockEdge, FlowItem, MeasuredBlock, MeasuredCell, MeasuredRow, Measurer, RowTarget, TableFlow,
};
use crate::layout::measure::{BlockSource, Failure};
use crate::layout::{LayoutError, geometry, margin::MarginStrut};
use crate::style::{BreakBetween, Display, Style, TableLayout};
use crate::table::{CellHeight, HeightConstraint, HeightInput, WidthInput};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    ops::Range,
    rc::Rc,
};

/// Grid rows of the header and footer groups, which repeat on every page of the table.
fn repeated_rows(data: &Data<'_>) -> [Range<usize>; 2] {
    let group_role = |index: usize| {
        data.groups[index]
            .source
            .map(|source| source.style().display.clone())
    };
    let rows = data.grid.rows;
    let header = match data.groups.first() {
        Some(group) if group_role(0) == Some(Display::TableHeaderGroup) => 0..group.rows.len(),
        _ => 0..0,
    };
    let footer = match data.groups.last() {
        Some(group) if group_role(data.groups.len() - 1) == Some(Display::TableFooterGroup) => {
            rows - group.rows.len()..rows
        }
        _ => rows..rows,
    };
    [header, footer]
}

/// Collapsed borders paint along grid lines instead of around each box; the box keeps its
/// half-border insets as transparent borders, so background origins and clips stay inside them.
pub(crate) fn collapsed_decoration(
    mut decoration: BoxDecoration,
    insets: [f32; 4],
) -> BoxDecoration {
    decoration.borders = insets.map(|width| Border {
        width,
        style: BorderStyle::Solid,
        color: Color::TRANSPARENT,
    });
    decoration
}

fn collapsed_cell_decoration(decoration: BoxDecoration, insets: [f32; 4]) -> BoxDecoration {
    let mut decoration = collapsed_decoration(decoration, insets);
    let initial = BoxDecoration::default();
    decoration.radii = initial.radii;
    decoration.corner_shapes = initial.corner_shapes;
    decoration.border_image = initial.border_image;
    if decoration.shadows.iter().any(|shadow| !shadow.inset) {
        std::sync::Arc::make_mut(&mut decoration.shadows).retain(|shadow| shadow.inset);
    }
    decoration
}

pub(crate) fn cell_height(measured: &MeasuredBlock<'_>) -> CellHeight {
    CellHeight {
        height: measured.height,
        baseline: measured.first_baseline().unwrap_or(measured.height),
        align: measured.style.cell_alignment(),
        constraint: HeightConstraint {
            specified: measured.style.height.clone(),
            minimum: measured.style.min_height.clone(),
            maximum: measured.style.max_height.clone(),
            box_sizing: measured.style.box_sizing,
            edges: measured.geometry.padding[0]
                + measured.geometry.padding[2]
                + measured.geometry.border[0]
                + measured.geometry.border[2],
        },
    }
}

/// Physical x of each vertical grid line in logical column order.
fn grid_lines(x: f32, widths: &[f32], table: f32, rtl: bool) -> Vec<f32> {
    std::iter::once(0.0)
        .chain(widths.iter().scan(0.0, |position, width| {
            *position += width;
            Some(*position)
        }))
        .map(|position| x + if rtl { table - position } else { position })
        .collect()
}

pub(crate) fn measure<'a>(
    measurer: &mut Measurer<'a>,
    block: &'a BlockBox,
    x: f32,
    width: f32,
    rtl: bool,
    containing_height: Option<f32>,
) -> Result<MeasuredBlock<'a>, Failure> {
    measure_placed(
        measurer,
        block,
        x,
        width,
        rtl,
        containing_height,
        geometry::horizontal_with_edges,
    )
    .map(|(table, _)| table)
}

/// Measures a table whose wrapper box geometry comes from `place`, returning that geometry.
pub(crate) fn measure_placed<'a>(
    measurer: &mut Measurer<'a>,
    block: &'a BlockBox,
    x: f32,
    width: f32,
    rtl: bool,
    containing_height: Option<f32>,
    place: fn(&Style, f32, f32, bool, Option<f32>, geometry::Edges) -> geometry::HorizontalBox,
) -> Result<(MeasuredBlock<'a>, geometry::HorizontalBox), Failure> {
    let plan = TablePlan::new(measurer, block, containing_height, None, place)?;
    let resolved = plan.resolve_geometry(measurer, x, width, rtl)?;
    plan.measure_wrapped(measurer, &resolved)
}

pub(crate) fn measure_format<'a>(
    measurer: &mut Measurer<'a>,
    block: impl Into<BlockSource<'a>>,
    geometry: &geometry::HorizontalBox,
    containing_height: Option<f32>,
    used_height: Option<f32>,
) -> Result<MeasuredBlock<'a>, Failure> {
    let plan = TablePlan::new(
        measurer,
        block,
        containing_height,
        used_height,
        geometry::horizontal_with_edges,
    )?;
    let resolved = plan.resolve_positioned(measurer, geometry)?;
    plan.measure_wrapped(measurer, &resolved)
        .map(|(block, _)| block)
}

/// Measures a table whose `used_height` belongs to its table wrapper box, as a flex or
/// stretched grid item's does: the wrapper is sized as if the captions were part of the table
/// box's border and padding (CSS Flexbox 1 §4), while `measure` takes the table box's height.
pub(crate) fn measure_wrapper<'a>(
    measurer: &mut Measurer<'a>,
    used_height: Option<f32>,
    mut measure: impl FnMut(&mut Measurer<'a>, Option<f32>) -> Result<MeasuredBlock<'a>, Failure>,
) -> Result<MeasuredBlock<'a>, Failure> {
    let mut wrapper = measure(measurer, used_height)?;
    let captions = wrapper.height - captions::principal(&mut wrapper).height;
    match used_height {
        Some(used) if captions > 0.0 => measure(measurer, Some((used - captions).max(0.0))),
        _ => Ok(wrapper),
    }
}

struct WidthContributions {
    cells: Vec<crate::table::CellWidth>,
    caption_min: f32,
}

pub(crate) struct TableGeometry {
    pub containing_height: Option<f32>,
    pub geometry: geometry::HorizontalBox,
    edges: geometry::Edges,
    sizes: crate::table::ColumnWidths,
    original_sizes: Option<crate::table::ColumnWidths>,
    collapsed_columns: Rc<[bool]>,
    starts: Vec<f32>,
    spacing: [f32; 2],
}
impl TableGeometry {
    pub(crate) fn grid_lines(&self, rtl: bool) -> Vec<f32> {
        grid_lines(self.geometry.x, &self.sizes.widths, self.sizes.table, rtl)
    }
    pub(crate) fn columns(&self, rtl: bool) -> Columns<'_> {
        Columns {
            x: self.geometry.x,
            table: self.sizes.table,
            rtl,
            starts: &self.starts,
            widths: &self.sizes.widths,
            collapsed: &self.collapsed_columns,
            horizontal: self.spacing[0],
        }
    }
}

#[derive(Clone)]
pub(crate) struct RowGeometry {
    original_starts: Option<Vec<f32>>,
    pub starts: Vec<f32>,
    pub offsets: Vec<f32>,
    pub height: f32,
}

impl RowGeometry {
    pub(crate) fn content_height(&self, rows: Range<usize>, spacing: f32) -> f32 {
        let starts = self.original_starts.as_ref().unwrap_or(&self.starts);
        (starts[rows.end] - starts[rows.start] - spacing).max(0.0)
    }
    fn project(mut self, collapsed: &[bool]) -> Self {
        if !collapsed.iter().any(|&value| value) {
            return self;
        }
        let original = self.starts.clone();
        let mut removed = 0.0;
        for (index, &hidden) in collapsed.iter().enumerate() {
            if hidden {
                removed += original[index + 1] - original[index];
            }
            self.starts[index + 1] -= removed;
        }
        self.height = (self.height - removed).max(0.0);
        self.original_starts = Some(original);
        self
    }
    /// Row starts from `rows`, each followed by `spacing` unless `hidden` (CSS 2.1 §17.6.1.1).
    fn new(rows: crate::table::RowHeights, spacing: f32, hidden: &[bool]) -> Self {
        let mut starts = Vec::with_capacity(rows.heights.len() + 1);
        starts.push(0.0);
        for (row, height) in rows.heights.into_iter().enumerate() {
            let gap = if hidden.get(row).copied().unwrap_or(false) {
                0.0
            } else {
                spacing
            };
            starts.push(starts.last().unwrap() + height + gap);
        }
        Self {
            original_starts: None,
            starts,
            offsets: rows.offsets,
            height: rows.table,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CellAlignment {
    pub offset: f32,
    pub height: f32,
    pub first_fragment: f32,
    content_height: f32,
}

#[derive(Clone)]
pub(crate) struct CellAlignments {
    first: usize,
    start_row: usize,
    row_starts: Vec<f32>,
    spacing: f32,
    cells: Vec<CellAlignment>,
}

pub(crate) struct TablePlan<'a> {
    pub has_containing_ancestors: bool,
    pub has_structural_geometry: bool,
    pub has_scoped_internals: bool,
    /// Whether the backgrounds of a cell, track or track group clip to text (CSS Backgrounds
    /// 4 §2.8), which each cell's flow then marks.
    pub clips_to_text: bool,
    pub has_height_constraints: bool,
    border_owners: Option<Vec<Option<usize>>>,
    containing_height: Option<f32>,
    used_height: Option<f32>,
    native_height: Option<crate::style::SizeConstraints>,
    pub source: &'a BlockBox,
    pub style: &'a Style,
    named_pages: bool,
    data: Rc<Data<'a>>,
    entry_events: Vec<&'a SemanticEvent>,
    cell_indices: HashMap<*const BlockBox, usize>,
    /// The rows each row keeps with it at page breaks, at column breaks within a page and at
    /// column breaks that are also page breaks.
    row_keep_ends: [Vec<usize>; 3],
    layers: Layers,
    columns: Vec<crate::table::ColumnWidth>,
    widths: RefCell<HashMap<u32, Rc<WidthContributions>>>,
    place: fn(&Style, f32, f32, bool, Option<f32>, geometry::Edges) -> geometry::HorizontalBox,
}

#[derive(Clone)]
pub(crate) struct TableContaining<'a> {
    pub source: &'a BlockBox,
    pub rows: Range<usize>,
    columns: Option<Range<usize>>,
    pub(crate) ancestors: [Option<&'a BlockBox>; 2],
}
impl TableContaining<'_> {
    pub(crate) fn rect(
        &self,
        geometry: &TableGeometry,
        placement: &super::BandPlacement,
        starts: &[f32],
        rtl: bool,
        overflow: bool,
    ) -> Option<Rect> {
        let first = if overflow {
            self.rows.start
        } else {
            self.rows.start.max(placement.rows.start)
        };
        let end = if overflow {
            self.rows.end
        } else {
            self.rows.end.min(placement.rows.end)
        };
        if first > end || (first == end && !self.rows.is_empty()) {
            return None;
        }
        let base = starts[placement.rows.start];
        let slice = placement
            .slice
            .clone()
            .unwrap_or(0.0..starts[placement.rows.end] - base);
        let top = starts[first] - base;
        let bottom = if self.rows.is_empty() {
            top
        } else {
            starts[end] - base - geometry.spacing[1]
        };
        let (top, bottom) = if overflow {
            (top, bottom)
        } else {
            (top.max(slice.start), bottom.min(slice.end))
        };
        if bottom < top {
            return None;
        }
        let columns = geometry.columns(rtl);
        let span = self.columns.clone().unwrap_or(0..columns.widths.len());
        let (x, width) = columns.span(span.start, span.len());
        let offset = super::model::relative_offset(&self.ancestors, geometry.geometry.width, rtl);
        Some(Rect {
            x: x + offset[0],
            y: placement.y + top - slice.start + offset[1],
            width,
            height: bottom - top,
        })
    }
}
fn structural_geometry(style: &Style) -> bool {
    !style.anchor.names.is_empty() || style.is_grouped() || style.has_transform_effects()
}
fn collect_structural_events<'a>(source: &'a BlockBox, events: &mut Vec<&'a SemanticEvent>) {
    events.extend(source.semantic_event());
    for child in source.children() {
        match child {
            BlockContent::Events(pending) => events.extend(pending),
            BlockContent::Block(child) => collect_structural_events(child, events),
            _ => {}
        }
    }
}
fn table_entry_events(source: &BlockBox) -> Vec<&SemanticEvent> {
    let mut events: Vec<_> = source.leading_events().collect();
    for child in source.children() {
        if let BlockContent::Block(child) = child
            && matches!(
                child.style().display,
                Display::TableColumn | Display::TableColumnGroup
            )
        {
            collect_structural_events(child, &mut events);
            events.extend(source.following_events(child));
        }
    }
    events
}

impl<'a> TablePlan<'a> {
    pub(crate) fn row_entry_events(&self, index: usize) -> Vec<&'a SemanticEvent> {
        let row = self.data.rows[index];
        let mut events = Vec::new();
        let group = &self.data.groups[self.data.row_groups[index]];
        if group
            .rows
            .first()
            .is_some_and(|first| std::ptr::eq(*first, row))
            && let Some(source) = group.source
        {
            events.extend(source.semantic_event());
            events.extend(source.leading_events());
        }
        events.extend(row.semantic_event());
        for child in row.children() {
            if let BlockContent::Events(pending) = child {
                events.extend(pending);
            }
        }
        events
    }
    pub(crate) fn row_exit_events(&self, index: usize) -> Vec<&'a SemanticEvent> {
        let row = self.data.rows[index];
        let group = &self.data.groups[self.data.row_groups[index]];
        let mut events: Vec<_> = self.source.following_events(row).collect();
        if let Some(source) = group.source {
            events.extend(source.following_events(row));
            if group
                .rows
                .last()
                .is_some_and(|last| std::ptr::eq(*last, row))
            {
                events.extend(self.source.following_events(source));
            }
        }
        events
    }
    pub(crate) fn is_body_group(&self, source: &BlockBox) -> bool {
        let owns = |group: Option<&super::model::Group<'_>>| {
            group
                .and_then(|group| group.source)
                .is_some_and(|owner| std::ptr::eq(owner, source))
        };
        match source.style().display {
            Display::TableHeaderGroup => !owns(self.data.groups.first()),
            Display::TableFooterGroup => !owns(self.data.groups.last()),
            Display::TableRowGroup => true,
            _ => false,
        }
    }
    pub(crate) fn row_group_source(&self, index: usize) -> Option<&'a BlockBox> {
        self.data.groups[self.data.row_groups[index]].source
    }
    /// The page groups a row starts: its row group's when it is the group's first row, and
    /// its own (CSS GCPM 3 §3.2).
    pub(crate) fn row_starting_groups(&self, index: usize) -> Vec<std::sync::Arc<str>> {
        let row = self.data.rows[index];
        let group = &self.data.groups[self.data.row_groups[index]];
        group
            .source
            .filter(|_| {
                group
                    .rows
                    .first()
                    .is_some_and(|first| std::ptr::eq(*first, row))
            })
            .into_iter()
            .chain([row])
            .filter_map(|owner| owner.page_boundary().group.clone())
            .collect()
    }
    pub(crate) fn row_source(&self, index: usize) -> &'a BlockBox {
        self.data.rows[index]
    }
    pub(crate) fn row_edges(
        &self,
        row: usize,
        fragmentainer: crate::layout::breaks::Fragmentainer,
    ) -> [BreakBetween; 2] {
        let data = &self.data;
        let mut edges = fragmentainer.edges(data.rows[row].style());
        let group = data.row_groups[row];
        if let Some(source) = data.groups[group].source {
            let group_edges = fragmentainer.edges(source.style());
            if row == 0 || data.row_groups[row - 1] != group {
                edges[0] = group_edges[0].combine(edges[0]);
            }
            if row + 1 == data.grid.rows || data.row_groups[row + 1] != group {
                edges[1] = group_edges[1].combine(edges[1]);
            }
        }
        if self.named_pages
            && row > 0
            && data.rows[row - 1].page_boundary().end != data.rows[row].page_boundary().start
        {
            edges[0] = edges[0].combine(BreakBetween::Page);
        }
        edges
    }
    /// The end of the rows kept with `row` at a break into `fragmentainer`, which is a page
    /// break when `page_break` is set.
    pub(crate) fn row_keep_end(
        &self,
        row: usize,
        end: usize,
        fragmentainer: crate::layout::breaks::Fragmentainer,
        page_break: bool,
    ) -> usize {
        let index = match fragmentainer {
            crate::layout::breaks::Fragmentainer::Page => 0,
            crate::layout::breaks::Fragmentainer::Column => 1 + usize::from(page_break),
        };
        self.row_keep_ends[index][row].min(end)
    }
    fn resolve_row_keeps(
        &self,
        fragmentainer: crate::layout::breaks::Fragmentainer,
        page_break: bool,
    ) -> Vec<usize> {
        // Group keeps depend on available height and belong to band measurement.
        let data = &self.data;
        let mut protected = vec![0; data.grid.rows + 1];
        for (cell, source) in data.grid.cells.iter().zip(&data.cells) {
            if cell.rows > 1 && fragmentainer.keeps_at(source.style(), page_break) {
                protected[cell.row + 1] = protected[cell.row + 1].max(cell.row + cell.rows);
            }
        }
        let mut ends = vec![data.grid.rows; data.grid.rows];
        let mut until = 0;
        for next in 1..data.grid.rows {
            until = until.max(protected[next]);
            let edge = self.row_edges(next - 1, fragmentainer)[1]
                .combine(self.row_edges(next, fragmentainer)[0]);
            if edge.is_forced()
                || (next >= until && !crate::layout::breaks::avoids(edge, page_break))
            {
                ends[next - 1] = next;
            }
        }
        for row in (0..data.grid.rows.saturating_sub(1)).rev() {
            ends[row] = ends[row].min(ends[row + 1]);
        }
        ends
    }
    pub(crate) fn new(
        measurer: &Measurer<'a>,
        block: impl Into<BlockSource<'a>>,
        containing_height: Option<f32>,
        used_height: Option<f32>,
        place: fn(&Style, f32, f32, bool, Option<f32>, geometry::Edges) -> geometry::HorizontalBox,
    ) -> Result<Rc<Self>, Failure> {
        let block = block.into();
        measurer.work.spend(1)?;
        let data = measurer.table_data.get(block)?;
        let entry_events = table_entry_events(block.block);
        measurer
            .work
            .spend(data.grid.rows.saturating_add(data.grid.cells.len()).max(1))?;
        let has_containing_ancestors = data
            .rows
            .iter()
            .copied()
            .chain(data.groups.iter().filter_map(|group| group.source))
            .any(|source| source.style().contains_absolute());
        let has_structural_geometry = data
            .rows
            .iter()
            .copied()
            .chain(data.groups.iter().filter_map(|group| group.source))
            .chain(data.columns.iter().map(|column| column.source))
            .any(|source| structural_geometry(source.style()));
        let clips_to_text = data
            .columns
            .iter()
            .map(|column| column.source)
            .chain(data.rows.iter().copied())
            .chain(data.groups.iter().filter_map(|group| group.source))
            .chain(data.cells.iter().copied())
            .any(|source| source.style().decoration.backgrounds.clips_to_text());
        // Text-clipped backgrounds paint with the cells' fragments, whose text they clip to.
        let has_scoped_internals = has_structural_geometry
            || clips_to_text
            || data.collapsed_columns.iter().any(|&value| value)
            || data
                .rows
                .iter()
                .copied()
                .chain(data.groups.iter().filter_map(|group| group.source))
                .chain(data.cells.iter().copied())
                .any(|source| {
                    let style = source.style();
                    style.contains_absolute()
                        || !style.visibility.is_visible()
                        || !style.decoration.shadows.is_empty()
                        || style.decoration.outline.is_drawn()
                        || style.decoration.border_image.is_drawn()
                        || style.is_grouped()
                        || style.forced_stacking_context()
                });
        let border_owners = if has_scoped_internals && data.collapsed.is_some() {
            let slots = data.grid.rows * data.grid.columns;
            measurer.work.spend(slots)?;
            let mut owners = vec![None; slots];
            for (index, cell) in data.grid.cells.iter().enumerate() {
                for row in cell.row..cell.row + cell.rows {
                    owners[row * data.grid.columns + cell.column
                        ..row * data.grid.columns + cell.column + cell.columns]
                        .fill(Some(index));
                }
            }
            Some(owners)
        } else {
            None
        };
        let native_height = measurer
            .native_vertical(block.block)
            .map(|vertical| vertical.constraints());
        let has_height_constraints = used_height.is_some()
            || crate::layout::ratio::PreferredRatio::of(block.style(), None, [0.0; 2]).is_some()
            || native_height.as_ref().is_some_and(|constraints| {
                constraints.specified.is_some()
                    || constraints.minimum != 0.0
                    || constraints.maximum.is_some()
            })
            || std::iter::once(block)
                .chain(data.rows.iter().copied().map(BlockSource::from))
                .chain(data.cells.iter().copied().map(BlockSource::from))
                .any(|source| {
                    [
                        &source.style().height,
                        &source.style().min_height,
                        &source.style().max_height,
                    ]
                    .into_iter()
                    .any(|size| !size.is_auto_block_size(false))
                });
        let mut plan = Self {
            has_containing_ancestors,
            has_structural_geometry,
            has_scoped_internals,
            clips_to_text,
            has_height_constraints,
            border_owners,
            containing_height,
            used_height,
            native_height,
            source: block.block,
            style: block.style(),
            named_pages: measurer.named_pages && !measurer.unbroken,
            columns: data.column_widths(),
            layers: Layers::new(&data),
            cell_indices: data
                .cells
                .iter()
                .enumerate()
                .map(|(index, source)| (std::ptr::from_ref(*source), index))
                .collect(),
            data,
            entry_events,
            row_keep_ends: [Vec::new(), Vec::new(), Vec::new()],
            widths: RefCell::new(HashMap::new()),
            place,
        };
        plan.row_keep_ends = [
            (crate::layout::breaks::Fragmentainer::Page, true),
            (crate::layout::breaks::Fragmentainer::Column, false),
            (crate::layout::breaks::Fragmentainer::Column, true),
        ]
        .map(|(fragmentainer, page_break)| plan.resolve_row_keeps(fragmentainer, page_break));
        Ok(Rc::new(plan))
    }
    fn contributions(
        &self,
        measurer: &Measurer<'a>,
        width: f32,
    ) -> Result<Rc<WidthContributions>, Failure> {
        if let Some(contributions) = self.widths.borrow().get(&width.to_bits()) {
            return Ok(contributions.clone());
        }
        let contributions = Rc::new(WidthContributions {
            cells: self.data.widths(measurer, 0.0, width)?,
            caption_min: captions::minimum(&self.data.captions, measurer, width)?,
        });
        self.widths
            .borrow_mut()
            .insert(width.to_bits(), contributions.clone());
        Ok(contributions)
    }
    pub(crate) fn resolve_geometry(
        &self,
        measurer: &Measurer<'a>,
        x: f32,
        width: f32,
        rtl: bool,
    ) -> Result<TableGeometry, Failure> {
        self.resolve(measurer, x, width, rtl, None)
    }

    pub(crate) fn has_captions(&self) -> bool {
        self.data.has_captions()
    }

    pub(crate) fn edges(&self, width: f32) -> geometry::Edges {
        self.data.table_edges(self.style, width)
    }

    pub(crate) fn resolve_positioned(
        &self,
        measurer: &Measurer<'a>,
        geometry: &geometry::HorizontalBox,
    ) -> Result<TableGeometry, Failure> {
        self.resolve(
            measurer,
            geometry.containing_x,
            geometry.containing_width,
            geometry.containing_rtl,
            Some(geometry),
        )
    }

    pub(crate) fn measure_wrapped(
        self: &Rc<Self>,
        measurer: &mut Measurer<'a>,
        resolved: &TableGeometry,
    ) -> Result<(MeasuredBlock<'a>, geometry::HorizontalBox), Failure> {
        let containing_height = resolved.containing_height;
        let (table, geometry) = measure_plan(measurer, self, resolved, None, 0)?;
        let mut table = captions::wrap(measurer, table, &self.data.captions, &geometry)?;
        let style = self.style;
        if self.has_captions()
            && (style.contains_absolute()
                || !style.visibility.is_visible()
                || style.is_grouped()
                || style.forced_stacking_context()
                || style.clipped_axes().contains(&true))
        {
            table.retain_fragment();
        }
        table.containing_height = containing_height;
        table.offset = self.style.relative_offset(
            geometry.containing_width,
            containing_height,
            geometry.containing_rtl,
        );
        Ok((table, geometry))
    }

    fn resolve(
        &self,
        measurer: &Measurer<'a>,
        x: f32,
        width: f32,
        rtl: bool,
        positioned: Option<&geometry::HorizontalBox>,
    ) -> Result<TableGeometry, Failure> {
        measurer.work.spend(
            self.data
                .grid
                .rows
                .saturating_mul(self.data.grid.columns)
                .max(1),
        )?;
        let style = self.style;
        let data = &self.data;
        let index = measurer.sources.block(self.source);
        let place = self.place;
        let collapsed = data.collapsed.is_some();
        let table_edges = data.table_edges(style, width);
        let provisional = positioned
            .cloned()
            .unwrap_or_else(|| place(style, x, width, rtl, None, table_edges));
        let edges = provisional.padding[1]
            + provisional.padding[3]
            + provisional.border[1]
            + provisional.border[3];
        let available = (width
            - edges
            - provisional.margin[1].unwrap_or(0.0)
            - provisional.margin[3].unwrap_or(0.0))
        .max(0.0);
        let mut constraints = crate::layout::intrinsic::width_constraints(
            self.source,
            measurer,
            Some(width),
            edges,
            Some(available),
        )?;
        if let Some(geometry) = positioned {
            constraints.specified = Some(geometry.width);
        }
        // CSS Sizing 4 §4.2: a definite height gives an automatic width through the preferred
        // aspect ratio, which the columns still grow past.
        let vertical_edges = provisional.padding[0]
            + provisional.padding[2]
            + provisional.border[0]
            + provisional.border[2];
        if constraints.specified.is_none()
            && let Some(ratio) =
                crate::layout::ratio::PreferredRatio::of(style, None, [edges, vertical_edges])
            && let Some(height) = crate::style::SizeConstraints::new(
                [&style.height, &style.min_height, &style.max_height],
                self.containing_height,
                style.box_sizing,
                vertical_edges,
            )
            .specified
        {
            constraints.specified = Some(constraints.constrain(ratio.width(height)));
        }
        let contributions = self.contributions(measurer, width)?;
        let [horizontal, vertical] = if collapsed {
            [0.0; 2]
        } else {
            style.table.spacing
        };
        let mut sizes = data.grid.column_widths(&WidthInput {
            cells: &contributions.cells,
            columns: &self.columns,
            layout: if positioned.is_some() && style.width.resolve(width).is_none() {
                TableLayout::Auto
            } else {
                style.table.layout
            },
            table: constraints,
            available,
            spacing: horizontal,
            caption_min: (contributions.caption_min - edges).max(0.0),
        })?;
        let original_sizes = if data.collapsed_columns.iter().any(|&value| value) {
            let widths: Vec<_> = sizes
                .widths
                .iter()
                .zip(data.collapsed_columns.iter())
                .map(|(&width, &collapsed)| if collapsed { 0.0 } else { width })
                .collect();
            let removed = sizes
                .widths
                .iter()
                .zip(data.collapsed_columns.iter())
                .filter_map(|(&width, &collapsed)| collapsed.then_some(width + horizontal))
                .sum::<f32>();
            let projected = crate::table::ColumnWidths {
                widths,
                table: (sizes.table - removed).max(0.0),
            };
            Some(std::mem::replace(&mut sizes, projected))
        } else {
            None
        };
        let geometry = positioned.map_or_else(
            || place(style, x, width, rtl, Some(sizes.table), table_edges),
            |geometry| geometry::HorizontalBox {
                width: sizes.table,
                ..geometry.clone()
            },
        );
        if !geometry.x.is_finite() || !geometry.width.is_finite() {
            return Err(LayoutError::NonFiniteBlockGeometry { block: index }.into());
        }
        let mut starts = Vec::with_capacity(data.grid.columns);
        let mut cursor = horizontal;
        for (&column_width, &collapsed) in sizes.widths.iter().zip(data.collapsed_columns.iter()) {
            starts.push(cursor);
            if !collapsed {
                cursor += column_width + horizontal;
            }
        }
        Ok(TableGeometry {
            containing_height: self.containing_height,
            geometry,
            edges: table_edges,
            sizes,
            original_sizes,
            collapsed_columns: data.collapsed_columns.clone(),
            starts,
            spacing: [horizontal, vertical],
        })
    }
    pub(crate) fn has_collapsed_rows(&self) -> bool {
        self.data.collapsed_rows.iter().any(|&value| value)
    }
    /// Whether any of `rows` takes space: neither collapsed nor hidden as empty.
    pub(crate) fn rows_visible(&self, rows: Range<usize>) -> bool {
        rows.into_iter()
            .any(|row| !self.data.collapsed_rows[row] && !self.data.hidden_rows[row])
    }
    pub(crate) fn cell_collapsed(&self, index: usize) -> bool {
        let cell = &self.data.grid.cells[index];
        !self.rows_visible(cell.row..cell.row + cell.rows)
            || self.data.collapsed_columns[cell.column..cell.column + cell.columns]
                .iter()
                .all(|&value| value)
    }
    pub(crate) fn cell_rows_collapsed(&self, index: usize) -> bool {
        let cell = &self.data.grid.cells[index];
        self.data.collapsed_rows[cell.row..cell.row + cell.rows]
            .iter()
            .any(|&value| value)
    }
    pub(crate) fn cell_clipped(&self, source: &BlockBox) -> bool {
        self.cell_index(source).is_some_and(|index| {
            let cell = &self.data.grid.cells[index];
            self.data.collapsed_rows[cell.row..cell.row + cell.rows]
                .iter()
                .any(|&value| value)
                || self.data.collapsed_columns[cell.column..cell.column + cell.columns]
                    .iter()
                    .any(|&value| value)
        })
    }
    pub(crate) fn cell_index(&self, source: &BlockBox) -> Option<usize> {
        self.cell_indices.get(&std::ptr::from_ref(source)).copied()
    }
    /// The column and row of the grid slot the cell originates in.
    pub(crate) fn cell_origin(&self, index: usize) -> [usize; 2] {
        let cell = &self.data.grid.cells[index];
        [cell.column, cell.row]
    }
    pub(crate) fn cell_has_explicit_height(&self, index: usize) -> bool {
        self.data.cells[index]
            .style()
            .height
            .resolve_definite(None)
            .is_some()
            || *self.style.height.basis() != crate::style::Size::Auto
            || self.used_height.is_some()
            || self
                .native_height
                .as_ref()
                .is_some_and(|constraints| constraints.specified.is_some())
    }
    pub(crate) fn cell_used_height(
        &self,
        index: usize,
        assigned: f32,
        resolved: &TableGeometry,
    ) -> Option<f32> {
        self.cell_has_explicit_height(index).then(|| {
            let geometry = self.cell_geometry(index, resolved);
            let edges =
                geometry.padding[0] + geometry.padding[2] + geometry.border[0] + geometry.border[2];
            (assigned - edges).max(0.0)
        })
    }
    pub(crate) fn measure_cell(
        &self,
        measurer: &mut Measurer<'a>,
        index: usize,
        resolved: &TableGeometry,
        used_height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let source = self.data.cells[index];
        let geometry = self.cell_geometry(index, resolved);
        let named_pages = std::mem::replace(&mut measurer.named_pages, false);
        let measured = measurer.measure_format(source, geometry, true, None, used_height);
        measurer.named_pages = named_pages;
        let mut measured = measured?;
        crate::layout::mathml::table::align_cell(&mut measured);
        measured.offset = self.cell_offset(index, resolved.geometry.width);
        let Some(mut paint) = measured.paint.take() else {
            return Ok(measured);
        };
        if self.data.collapsed.is_some() {
            paint.decoration =
                collapsed_cell_decoration(paint.decoration, self.data.cell_borders(index));
        } else if self.data.hides_cell(index) {
            return Ok(measured);
        }
        crate::layout::writing::owner_decoration(&mut measured, &paint.decoration);
        measured.paint = Some(paint);
        Ok(measured)
    }
    pub(crate) fn cell_rows(&self, index: usize) -> Range<usize> {
        let cell = &self.data.grid.cells[index];
        cell.row..cell.row + cell.rows
    }
    pub(crate) fn cell_offset(&self, index: usize, width: f32) -> [f32; 2] {
        super::model::relative_offset(
            &self.data.cell_ancestors(index),
            width,
            self.style.used_rtl(),
        )
    }
    pub(crate) fn scoped_borders(&self) -> bool {
        self.border_owners.is_some()
    }
    pub(crate) fn border_owner(
        &self,
        row: usize,
        column: usize,
        width: f32,
    ) -> (Option<usize>, [f32; 2]) {
        let owner = self
            .border_owners
            .as_ref()
            .and_then(|owners| owners[row * self.data.grid.columns + column]);
        let offset = match owner {
            Some(index) => self.cell_offset(index, width),
            None => super::model::relative_offset(
                &[
                    self.data.groups[self.data.row_groups[row]].source,
                    Some(self.data.rows[row]),
                ],
                width,
                self.style.used_rtl(),
            ),
        };
        (owner, offset)
    }
    pub(crate) fn structural_decoration(&self, source: &BlockBox) -> BoxDecoration {
        let mut decoration = source.decoration().clone();
        if self.data.collapsed.is_some() {
            decoration.radii = BoxDecoration::default().radii;
            decoration.corner_shapes = BoxDecoration::default().corner_shapes;
        } else {
            // CSS 2 §17.6.1 ignores borders on tracks and track groups.
            decoration.borders = BoxDecoration::default().borders;
        }
        decoration
    }
    pub(crate) fn structural_shadow(
        &self,
        source: &BlockBox,
        inset: bool,
    ) -> Option<BoxDecoration> {
        if !inset && self.data.collapsed.is_some() {
            return None;
        }
        let decoration = self.structural_decoration(source);
        let shadows = decoration
            .shadows
            .iter()
            .filter(|shadow| shadow.inset == inset)
            .copied()
            .collect::<Vec<_>>();
        if shadows.is_empty() {
            return None;
        }
        Some(BoxDecoration {
            shadows: shadows.into(),
            radii: decoration.radii,
            corner_shapes: decoration.corner_shapes,
            ..Default::default()
        })
    }
    /// A row or row group's outline, drawn around each of its fragments; radii follow the
    /// separated borders model only, as collapsed tables have none.
    pub(crate) fn structural_outline(&self, source: &BlockBox) -> Option<BoxDecoration> {
        let decoration = self.structural_decoration(source);
        decoration.outline.is_drawn().then_some(BoxDecoration {
            outline: decoration.outline,
            radii: decoration.radii,
            corner_shapes: decoration.corner_shapes,
            ..Default::default()
        })
    }
    /// A row or row group's border image, which does not apply in the collapsing border
    /// model (CSS Backgrounds 3 §6.1); the separated model leaves it no border widths.
    pub(crate) fn structural_border_image(&self, source: &BlockBox) -> Option<BoxDecoration> {
        if self.data.collapsed.is_some() {
            return None;
        }
        let decoration = self.structural_decoration(source);
        decoration.border_image.is_drawn().then_some(BoxDecoration {
            border_image: decoration.border_image,
            ..Default::default()
        })
    }
    pub(crate) fn cell_parents(&self, index: usize) -> [Option<&'a BlockBox>; 2] {
        let [group, row, _] = self.data.cell_ancestors(index);
        [group, row]
    }
    pub(crate) fn cell_layers(&self, source: &BlockBox) -> [Option<TableContaining<'a>>; 2] {
        let Some(index) = self.cell_index(source) else {
            return [None, None];
        };
        self.row_layers(self.data.grid.cells[index].row)
    }
    pub(crate) fn row_layers(&self, row: usize) -> [Option<TableContaining<'a>>; 2] {
        let group = self.data.row_groups[row];
        let group_source = self.data.groups[group].source;
        let row_source = Some(self.data.rows[row]);
        [
            group_source.map(|source| TableContaining {
                source,
                rows: self.layers.group_rows[group].clone(),
                columns: None,
                ancestors: [group_source, None],
            }),
            row_source.map(|source| TableContaining {
                source,
                rows: row..row + 1,
                columns: None,
                ancestors: [group_source, row_source],
            }),
        ]
    }
    pub(crate) fn structural_boxes(
        &self,
        work: &crate::layout::work::Budget,
    ) -> Result<Vec<TableContaining<'a>>, LayoutError> {
        work.spend(
            self.data
                .rows
                .len()
                .saturating_add(self.data.groups.len())
                .saturating_add(self.data.columns.len()),
        )?;
        let rows = (0..self.data.rows.len()).map(|row| {
            self.row_layers(row)[1]
                .take()
                .expect("a table row has a source")
        });
        let groups = self
            .data
            .groups
            .iter()
            .enumerate()
            .filter_map(|(group, data)| {
                data.source.map(|source| TableContaining {
                    source,
                    rows: self.layers.group_rows[group].clone(),
                    columns: None,
                    ancestors: [Some(source), None],
                })
            });
        let columns = self.data.columns.iter().map(|column| TableContaining {
            source: column.source,
            rows: 0..self.data.grid.rows,
            columns: Some(column.start..column.start + column.span),
            ancestors: [None, None],
        });
        Ok(rows
            .chain(columns)
            .chain(groups)
            .filter(|area| area.rows.is_empty() || structural_geometry(area.source.style()))
            .collect())
    }
    pub(crate) fn cell_alignments(
        &self,
        measurer: &mut Measurer<'a>,
        band: Range<usize>,
        geometry: &TableGeometry,
    ) -> Result<CellAlignments, Failure> {
        let band = if self.has_height_constraints || self.has_collapsed_rows() {
            0..self.data.grid.rows
        } else {
            band
        };
        let range = self.data.row_ranges[band.start].start..self.data.row_ranges[band.end - 1].end;
        let grid = crate::table::Grid {
            cells: self.data.grid.cells[range.clone()]
                .iter()
                .map(|cell| crate::table::Cell {
                    row: cell.row - band.start,
                    ..*cell
                })
                .collect(),
            rows: band.len(),
            columns: self.data.grid.columns,
        };
        let mut first_fragments = Vec::with_capacity(range.len());
        let heights = range
            .clone()
            .map(|index| {
                let measured = self.measure_cell(measurer, index, geometry, None)?;
                first_fragments.push(measured.first_fragment);
                Ok(cell_height(&measured))
            })
            .collect::<Result<Vec<_>, Failure>>()?;
        measurer
            .work
            .spend(grid.rows.saturating_mul(grid.columns).max(1))?;
        let rows = if self.has_height_constraints || self.has_collapsed_rows() {
            self.resolve_rows(measurer, &heights, geometry.spacing[1], &geometry.geometry)?
        } else {
            let hidden = &self.data.hidden_rows[band.clone()];
            RowGeometry::new(
                grid.row_heights(&HeightInput {
                    hidden,
                    ..HeightInput::natural(&heights, geometry.spacing[1])
                })?,
                geometry.spacing[1],
                hidden,
            )
        };
        let alignments = grid
            .cells
            .iter()
            .zip(rows.offsets)
            .zip(first_fragments)
            .zip(&heights)
            .map(
                |(((cell, offset), first_fragment), measured)| CellAlignment {
                    content_height: measured.height,
                    offset,
                    height: rows.starts[cell.row + cell.rows]
                        - rows.starts[cell.row]
                        - geometry.spacing[1],
                    first_fragment,
                },
            )
            .collect();
        Ok(CellAlignments {
            first: range.start,
            start_row: band.start,
            row_starts: rows.starts,
            spacing: geometry.spacing[1],
            cells: alignments,
        })
    }
    pub(crate) fn fragment_alignment(
        &self,
        alignments: &CellAlignments,
        index: usize,
        available: f32,
        fresh: f32,
        fragmentainer: crate::layout::breaks::Fragmentainer,
        page_break: bool,
    ) -> f32 {
        let spacing = alignments.spacing;
        let alignment = alignments.cells[index - alignments.first];
        if crate::layout::pagination::overflows(alignment.height, fresh)
            || crate::layout::pagination::overflows(alignment.first_fragment, available)
        {
            return alignment.offset;
        }
        let cell_rows = self.cell_rows(index);
        let mut extent = 0.0;
        for row in cell_rows.clone() {
            let local = row - alignments.start_row;
            let height = alignments.row_starts[local + 1] - alignments.row_starts[local] - spacing;
            let keep_end = self.row_keep_end(
                row,
                alignments.start_row + alignments.row_starts.len() - 1,
                fragmentainer,
                page_break,
            );
            let prefix = alignments.row_starts[keep_end - alignments.start_row]
                - alignments.row_starts[local]
                - spacing;
            let required =
                if keep_end > row + 1 && !crate::layout::pagination::overflows(prefix, fresh) {
                    prefix
                } else if fragmentainer.keeps_at(self.row_source(row).style(), page_break)
                    && !crate::layout::pagination::overflows(height, fresh)
                {
                    height
                } else {
                    0.0
                };
            let forced = row > cell_rows.start
                && self.row_edges(row - 1, fragmentainer)[1]
                    .combine(self.row_edges(row, fragmentainer)[0])
                    .is_forced();
            if row > cell_rows.start
                && (forced
                    || (required > 0.0
                        && crate::layout::pagination::overflows(extent + required, available)))
            {
                let extra = (extent - spacing - alignment.content_height).max(0.0);
                return match self.data.cells[index].style().cell_alignment() {
                    crate::style::VerticalAlign::Middle => extra / 2.0,
                    crate::style::VerticalAlign::Bottom => extra,
                    _ => alignment
                        .offset
                        .min((extent - spacing - alignment.first_fragment).max(0.0)),
                };
            }
            extent += height + spacing;
        }
        alignment
            .offset
            .min((available - alignment.first_fragment).max(0.0))
    }
    pub(crate) fn resolve_rows(
        &self,
        measurer: &Measurer<'_>,
        cells: &[CellHeight],
        spacing: f32,
        geometry: &geometry::HorizontalBox,
    ) -> Result<RowGeometry, Failure> {
        measurer.work.spend(
            self.data
                .grid
                .rows
                .saturating_mul(self.data.grid.columns)
                .max(1),
        )?;
        if !self.has_height_constraints {
            return Ok(RowGeometry::new(
                self.data.grid.row_heights(&HeightInput {
                    hidden: &self.data.hidden_rows,
                    ..HeightInput::natural(cells, spacing)
                })?,
                spacing,
                &self.data.hidden_rows,
            )
            .project(&self.data.collapsed_rows));
        }
        let style = self.style;
        let rows = self
            .data
            .rows
            .iter()
            .map(|row| HeightConstraint {
                specified: row.style().height.clone(),
                minimum: row.style().min_height.clone(),
                maximum: row.style().max_height.clone(),
                ..HeightConstraint::default()
            })
            .collect::<Vec<_>>();
        let edges =
            geometry.padding[0] + geometry.padding[2] + geometry.border[0] + geometry.border[2];
        let rows = self.data.grid.row_heights(&HeightInput {
            cells,
            rows: &rows,
            spacing,
            hidden: &self.data.hidden_rows,
            table: self.native_height.clone().unwrap_or_else(|| {
                self.used_height.map_or_else(
                    || {
                        let mut constraints = crate::style::SizeConstraints::new(
                            [&style.height, &style.min_height, &style.max_height],
                            self.containing_height,
                            style.box_sizing,
                            edges,
                        );
                        // CSS Sizing 4 §4.1: a table box is no internal table box, so its
                        // preferred aspect ratio gives an automatic height, which its rows
                        // still grow past.
                        if constraints.specified.is_none() {
                            constraints.specified = crate::layout::ratio::PreferredRatio::of(
                                style,
                                None,
                                [
                                    geometry.padding[1]
                                        + geometry.padding[3]
                                        + geometry.border[1]
                                        + geometry.border[3],
                                    edges,
                                ],
                            )
                            .map(|ratio| ratio.height(geometry.width));
                        }
                        constraints
                    },
                    |height| crate::style::SizeConstraints::fixed(Some(height), 0.0, None),
                )
            }),
        })?;
        Ok(RowGeometry::new(rows, spacing, &self.data.hidden_rows)
            .project(&self.data.collapsed_rows))
    }
    pub(crate) fn used_row_starts(
        &self,
        measurer: &Measurer<'_>,
        heights: &[f32],
        spacing: f32,
        projection: Option<(&[f32], &[f32])>,
    ) -> Result<Vec<f32>, Failure> {
        // Border-box extents already include baseline and vertical alignment.
        let cells = heights
            .iter()
            .enumerate()
            .map(|(index, &height)| CellHeight {
                height: height
                    + projection.map_or(0.0, |(original, projected)| {
                        let cell = &self.data.grid.cells[index];
                        ((original[cell.row + cell.rows] - original[cell.row])
                            - (projected[cell.row + cell.rows] - projected[cell.row]))
                            .max(0.0)
                    }),
                baseline: 0.0,
                align: crate::style::VerticalAlign::Top,
                constraint: HeightConstraint::default(),
            })
            .collect::<Vec<_>>();
        measurer.work.spend(
            self.data
                .grid
                .rows
                .saturating_mul(self.data.grid.columns)
                .max(1),
        )?;
        Ok(RowGeometry::new(
            self.data.grid.row_heights(&HeightInput {
                hidden: &self.data.hidden_rows,
                ..HeightInput::natural(&cells, spacing)
            })?,
            spacing,
            &self.data.hidden_rows,
        )
        .project(&self.data.collapsed_rows)
        .starts)
    }
    pub(crate) fn cell_span(&self, index: usize, resolved: &TableGeometry) -> (f32, f32) {
        let cell = &self.data.grid.cells[index];
        resolved
            .columns(self.style.used_rtl())
            .span(cell.column, cell.columns)
    }
    pub(crate) fn cell_geometry(
        &self,
        index: usize,
        resolved: &TableGeometry,
    ) -> geometry::HorizontalBox {
        let cell = &self.data.grid.cells[index];
        let style = self.data.cells[index].style();
        let border = self.data.cell_borders(index);
        let original = resolved.original_sizes.as_ref().unwrap_or(&resolved.sizes);
        let padding = style
            .padding
            .each_ref()
            .map(|padding| padding.resolve(original.table));
        let (mut left, projected) = self.cell_span(index, resolved);
        let outer = original.widths[cell.column..cell.column + cell.columns]
            .iter()
            .sum::<f32>()
            + resolved.spacing[0] * cell.columns.saturating_sub(1) as f32;
        if self.style.used_rtl() {
            left += projected - outer;
        }
        geometry::HorizontalBox {
            containing_x: resolved.geometry.x,
            containing_width: original.table,
            containing_rtl: self.style.used_rtl(),
            x: left + padding[3] + border[3],
            width: (outer - padding[1] - padding[3] - border[1] - border[3]).max(0.0),
            padding,
            border,
            margin: [Some(0.0); 4],
            stretch_margins: [0.0; 2],
        }
    }
}

pub(crate) fn resume<'a>(
    measurer: &mut Measurer<'a>,
    previous: &TableFlow<'a>,
    resolved: &TableGeometry,
    next_body: usize,
) -> Result<MeasuredBlock<'a>, Failure> {
    measure_plan(
        measurer,
        &previous.plan,
        resolved,
        Some(previous),
        next_body,
    )
    .map(|(table, _)| table)
}

fn append_table_boxes<'a>(
    boundary: usize,
    boxes: &mut BTreeMap<usize, Vec<FlowItem<'a>>>,
    items: &mut Vec<FlowItem<'a>>,
) {
    if let Some(boxes) = boxes.remove(&boundary) {
        items.extend(boxes);
    }
}

fn measure_plan<'a>(
    measurer: &mut Measurer<'a>,
    plan: &Rc<TablePlan<'a>>,
    resolved: &TableGeometry,
    previous: Option<&TableFlow<'a>>,
    next_body: usize,
) -> Result<(MeasuredBlock<'a>, geometry::HorizontalBox), Failure> {
    let block = BlockSource {
        block: plan.source,
        selected: plan.style,
    };
    let index = measurer.sources.block(block.block);
    let style = block.style();
    let data = &plan.data;
    measurer.work.spend(data.cells.len().max(1))?;
    let collapsed = data.collapsed.is_some();
    let geometry = resolved.geometry.clone();
    let table_edges = resolved.edges;
    let sizes = &resolved.sizes;
    let vertical = resolved.spacing[1];
    let columns = resolved.columns(style.used_rtl());
    let mut blocks = Vec::new();
    let mut heights = Vec::new();
    for (cell_index, cell) in data.grid.cells.iter().enumerate() {
        if let Some(previous) = previous
            && previous.body.contains(&cell.row)
            && cell.row < next_body
        {
            heights.push(previous.heights[cell_index].clone());
            blocks.push(None);
            continue;
        }
        let measured = plan.measure_cell(measurer, cell_index, resolved, None)?;
        heights.push(cell_height(&measured));
        blocks.push(Some(measured));
    }
    let mut has_forced = blocks.iter().flatten().any(|block| block.has_forced);
    let rows = plan.resolve_rows(measurer, &heights, vertical, &geometry)?;
    let row_starts = &rows.starts;
    let cell_offsets = &rows.offsets;
    let content_height = rows.height;
    let top = geometry.padding[0] + geometry.border[0];
    let bottom = geometry.padding[2] + geometry.border[2];
    let height = top + content_height + bottom;
    if !height.is_finite() {
        return Err(LayoutError::NonFiniteBlockGeometry { block: index }.into());
    }
    let [header_rows, footer_rows] = repeated_rows(data);
    let mut header = Vec::new();
    let mut footer = Vec::new();
    let repeated_height: f32 = [&header_rows, &footer_rows]
        .into_iter()
        .map(|rows| row_starts[rows.end] - row_starts[rows.start])
        .sum();
    let mut tallest_body: f32 = 0.0;
    let mut zero_boxes: BTreeMap<usize, Vec<FlowItem<'a>>> = BTreeMap::new();
    for area in plan.structural_boxes(&measurer.work)? {
        if area.rows.is_empty() {
            let edges = if area.source.style().display.is_row_group() {
                measurer.fragmentainer.edges(area.source.style())
            } else {
                [BreakBetween::Auto; 2]
            };
            let mut events = Vec::new();
            if area.source.style().display.is_row_group() {
                collect_structural_events(area.source, &mut events);
                events.extend(plan.source.following_events(area.source));
            }
            measurer.work.spend(events.len().max(1))?;
            has_forced |= edges.iter().any(|edge| edge.is_forced());
            zero_boxes
                .entry(area.rows.start)
                .or_default()
                .push(FlowItem::TableBox(Box::new(
                    crate::layout::measure::MeasuredTableBox {
                        area,
                        edges,
                        events,
                    },
                )));
        }
    }
    let mut items = vec![FlowItem::Edge(BlockEdge::Start, top)];
    items.extend(
        plan.entry_events
            .iter()
            .map(|event| FlowItem::Events(std::slice::from_ref(*event))),
    );
    append_table_boxes(0, &mut zero_boxes, &mut items);
    if data.grid.rows > 0 {
        items.push(FlowItem::Space(vertical));
    } else if content_height > 0.0 {
        items.push(FlowItem::Space(content_height));
    }
    let areas = Areas {
        columns: &columns,
        row_starts,
        vertical,
    };
    let mut row = 0;
    while row < data.grid.rows {
        let group_index = data.row_groups[row];
        let group_end = plan.layers.group_rows[group_index].end;
        let body_band = row >= header_rows.end && group_end <= footer_rows.start;
        let reserved_groups = if body_band { repeated_height } else { 0.0 };
        let available_height =
            crate::layout::breaks::fragmentainer_height(measurer.page_height) - reserved_groups;
        let group_kept = data.groups[group_index]
            .source
            .is_some_and(|source| measurer.fragmentainer.keeps(source.style()))
            && row_starts[group_end] - row_starts[row] + top + bottom <= available_height;
        let mut end = if group_kept { group_end } else { row + 1 };
        let mut scan = row;
        while scan < end {
            for cell in &data.grid.cells[data.row_ranges[scan].clone()] {
                end = end.max(cell.row + cell.rows);
            }
            scan += 1;
        }
        if previous.is_some_and(|flow| flow.body.contains(&row)) && end <= next_body {
            row = end;
            continue;
        }
        let band_height = (row_starts[end] - row_starts[row] - vertical).max(0.0);
        let range = data.row_ranges[row].start..data.row_ranges[end - 1].end;
        let forced = blocks[range.clone()]
            .iter()
            .flatten()
            .any(|block| block.has_forced);
        let internal_forced = (row + 1..end).any(|index| {
            plan.row_edges(index - 1, measurer.fragmentainer)[1].is_forced()
                || plan.row_edges(index, measurer.fragmentainer)[0].is_forced()
        });
        has_forced |= internal_forced;
        let kept = group_kept
            || data.rows[row..end]
                .iter()
                .chain(data.cells[range.clone()].iter())
                .any(|source| measurer.fragmentainer.keeps(source.style()));
        let expanded_cells = plan.has_height_constraints
            && range.clone().any(|index| {
                let cell = &data.grid.cells[index];
                blocks[index].as_ref().is_some_and(|block| {
                    row_starts[cell.row + cell.rows] - row_starts[cell.row] - vertical
                        > block.height
                })
            });
        let soft_break = expanded_cells
            || blocks[range.clone()]
                .iter()
                .flatten()
                .any(|block| block.first_fragment > 0.0 && block.first_fragment < block.height);
        let keep_end = plan.row_keep_end(
            row,
            end,
            measurer.fragmentainer,
            measurer.fragmentainer == crate::layout::breaks::Fragmentainer::Page,
        );
        let row_boundaries = end > row + 1
            && !group_kept
            && (keep_end < end || band_height + top + bottom + vertical > available_height);
        let fragments = !measurer.unbroken
            && (row_boundaries
                || forced
                || internal_forced
                || ((expanded_cells
                    || !blocks[range.clone()]
                        .iter()
                        .flatten()
                        .all(crate::layout::breaks::indivisible))
                    && ((body_band && !kept && soft_break)
                        || band_height + top + bottom + vertical > available_height)));
        let mut painted_cells = Vec::new();
        let mut collapsed_cells = Vec::new();
        let mut cell_heights = Vec::new();
        let mut visible_cells = Vec::new();
        let mut own_paints = Vec::new();
        let mut targets = band_targets(data, &areas, row..end);
        for (offset, slot) in blocks[range.clone()].iter_mut().enumerate() {
            let index = range.start + offset;
            let cell = &data.grid.cells[index];
            let mut measured = slot.take().unwrap();
            let stretch = fragments || plan.has_scoped_internals;
            let rect = areas.rect(
                cell.column..cell.column + cell.columns,
                cell.row..cell.row + cell.rows,
                row,
            );
            let content_height = rows.content_height(cell.row..cell.row + cell.rows, vertical);
            if let Some(used_height) = plan.cell_used_height(index, content_height, resolved) {
                measured = plan.measure_cell(measurer, index, resolved, Some(used_height))?;
            }
            measured.align_cell(cell_offsets[index], rect.height);
            cell_heights.push((index, rect.height));
            if plan.cell_collapsed(index) {
                collapsed_cells.push(crate::layout::measure::CollapsedItem {
                    source: measured.source,
                    offset: rect.y,
                    after_events: Vec::new(),
                });
                continue;
            }
            let hidden = data.hides_cell(index);
            if let Some(mut paint) = measured.paint.take() {
                paint.rect = rect;
                if stretch {
                    measured.paint = Some(paint);
                } else {
                    // The row paints the cell's box, at whose edges the cell still clips its
                    // contents (CSS Overflow 3 §3.1).
                    if measured.style.clip_edges().next().is_some() {
                        measured.paint = Some(paint.unpainted());
                    }
                    own_paints.push(paint);
                }
            }
            if !hidden {
                visible_cells.push(index);
            }
            if !stretch && !measured.navigation.is_empty() {
                targets.push(RowTarget {
                    visible: measured.style.visibility.is_visible(),
                    row: cell.row,
                    rect,
                    offset: [0.0; 2],
                    navigation: measured.navigation,
                });
            }
            let backgrounds: Vec<_> = if stretch && !hidden {
                plan.layers
                    .backgrounds(data, &[index])
                    .into_iter()
                    .map(|source| FragmentBackground::new(source, data))
                    .collect()
            } else {
                Vec::new()
            };
            if stretch {
                measured.retain_fragment();
            }
            painted_cells.push(MeasuredCell {
                clipped_height: plan
                    .cell_rows_collapsed(index)
                    .then_some((rect.height - cell_offsets[index]).max(0.0)),
                after_events: Vec::new(),
                grid_cell: Some(index),
                backgrounds,
                flex_item: false,
                block: measured,
                offset: rect.y + cell_offsets[index],
                paint_offset: stretch.then_some(rect.y),
                stretch,
                after: 0.0,
            });
        }
        let backgrounds = plan.layers.backgrounds(
            data,
            if fragments || plan.has_scoped_internals {
                &[]
            } else {
                &visible_cells
            },
        );
        let background_rows = backgrounds
            .iter()
            .map(|background| background.rows(data))
            .collect();
        let mut paints = backgrounds
            .iter()
            .map(|background| background.paint(data, &areas, row))
            .collect::<Vec<_>>();
        paints.extend(own_paints);
        let break_before = plan.row_edges(row, measurer.fragmentainer)[0];
        let break_after = plan.row_edges(end - 1, measurer.fragmentainer)[1];
        has_forced |= break_before.is_forced() || break_after.is_forced();
        let kept_height = (row_starts[keep_end] - row_starts[row] - vertical).max(0.0);
        let row_height = (row_starts[row + 1] - row_starts[row] - vertical).max(0.0);
        let first_fragment =
            if row_boundaries && keep_end > row + 1 && kept_height <= available_height {
                kept_height
            } else if row_boundaries
                && measurer.fragmentainer.keeps(data.rows[row].style())
                && row_height <= available_height
            {
                row_height
            } else if fragments && !group_kept {
                crate::layout::breaks::parallel_progress(
                    painted_cells
                        .iter()
                        .filter(|cell| plan.cell_rows(cell.grid_cell.unwrap()).start == row),
                    !expanded_cells,
                )
                .min(band_height)
            } else {
                band_height
            };
        let measured_row = MeasuredRow {
            grid: None,
            baselines: None,
            collapsed: collapsed_cells,
            background_rows,
            cell_heights,
            height: band_height,
            first_fragment,
            paints,
            after: Vec::new(),
            targets,
            cells: painted_cells,
            break_before,
            break_after,
            band: Some(row..end),
        };
        if end <= header_rows.end {
            header.push(measured_row.clone());
        } else if row >= footer_rows.start {
            footer.push(measured_row.clone());
        } else {
            tallest_body = tallest_body.max(measured_row.progress_height());
        }
        items.push(FlowItem::Row(Box::new(measured_row)));
        if plan.rows_visible(row..end) {
            items.push(FlowItem::Space(vertical));
        }
        append_table_boxes(end, &mut zero_boxes, &mut items);
        row = end;
    }
    items.push(FlowItem::Edge(BlockEdge::End, bottom));
    // Repeating groups must leave room for a body band on every page.
    let repeat = !(header.is_empty() && footer.is_empty())
        && top + repeated_height + tallest_body + 2.0 * vertical + bottom <= measurer.page_height;
    let footer_height = if repeat {
        footer.iter().map(|row| row.height + vertical).sum()
    } else {
        0.0
    };
    if !repeat {
        header.clear();
        footer.clear();
    }
    // A page holding only the repeated header would show it again on the next page, so the
    // header bands stay with the first body band unless a forced break separates them.
    if !header.is_empty() {
        for item in &mut items {
            match item {
                FlowItem::Row(row)
                    if row
                        .band
                        .as_ref()
                        .is_some_and(|band| band.end <= header_rows.end)
                        && !row.break_after.is_forced() =>
                {
                    row.break_after = BreakBetween::Avoid;
                }
                FlowItem::TableBox(table)
                    if table.area.rows.start <= header_rows.end
                        && table.area.source.style().display.is_row_group()
                        && !table.edges[1].is_forced() =>
                {
                    table.edges[1] = BreakBetween::Avoid;
                }
                _ => {}
            }
        }
    }
    let original_row_starts = rows.original_starts.map(Rc::from);
    let row_starts: Rc<[f32]> = rows.starts.into();
    let collapsed_grid = data.collapsed.clone().map(|borders| CollapsedGrid {
        borders,
        lines: grid_lines(geometry.x, &sizes.widths, sizes.table, style.used_rtl()),
        offsets: row_starts.clone(),
    });
    let table = Some(Box::new(TableFlow {
        original_row_starts,
        row_starts,
        plan: plan.clone(),
        geometry: geometry.clone(),
        heights,
        header,
        footer,
        body: header_rows.end..footer_rows.start,
        footer_height,
        spacing: vertical,
        collapsed: collapsed_grid,
    }));
    let overflow_height = crate::layout::measure::overflow_height(&items).max(height);
    let edges = measurer.fragmentainer.edges(style);
    let (break_before, break_after) = crate::layout::breaks::boundaries(style, &items, edges);
    let first_fragment = if !has_forced
        && measurer.fragmentainer.keeps(style)
        && overflow_height <= measurer.page_height
    {
        overflow_height
    } else {
        (crate::layout::breaks::first_fragment(&items) + bottom + footer_height).min(height)
    };
    let measured = MeasuredBlock {
        frame_key: None,
        frame_owner: None,
        math: None,
        grid: None,
        clearance_before: None,
        clearance_target: measurer
            .clearance_targets
            .get(&std::ptr::from_ref::<BlockBox>(block.block))
            .copied(),
        source: block.block,
        geometry: geometry.clone(),
        containing_height: resolved.containing_height,
        offset: if data.captions.is_empty() {
            style.relative_offset(
                geometry.containing_width,
                resolved.containing_height,
                geometry.containing_rtl,
            )
        } else {
            [0.0; 2]
        },
        style,
        canvas_source: block.is_canvas_source(),
        before: MarginStrut::edge(block.block, 0, geometry.containing_width),
        after: MarginStrut::edge(block.block, 2, geometry.containing_width),
        through: false,
        height,
        sizing: None,
        constrained: false,
        independent: true,
        overflow_height,
        first_fragment,
        has_forced: has_forced || edges.iter().any(|edge| edge.is_forced()),
        break_before,
        break_after,
        items,
        paint: measurer.box_paint(block, &geometry).map(|mut paint| {
            if collapsed {
                paint.decoration = collapsed_decoration(paint.decoration, table_edges.border);
            }
            paint
        }),
        navigation: block.navigation(),
        end_space: bottom,
        start_space: top,
        // A collapsed continuation keeps the table's top border width above its first grid line.
        repeat_space: if collapsed && !style.clone_decoration {
            top
        } else {
            vertical
        },
        marker: None,
        table,
        continues: false,
        spanner_fragment: None,
        line_fragments: Vec::new(),
    };
    Ok((measured, geometry))
}

/// Border boxes, within the band of grid rows `band`, of its rows and of the row-group pieces
/// it holds, where those carry navigation.
fn band_targets(data: &Data<'_>, areas: &Areas<'_>, band: Range<usize>) -> Vec<RowTarget> {
    let columns = 0..data.grid.columns;
    let mut targets = Vec::new();
    let mut start = band.start;
    while start < band.end {
        let group = data.row_groups[start];
        let end = (start..band.end)
            .find(|&row| data.row_groups[row] != group)
            .unwrap_or(band.end);
        if let Some(source) = data.groups[group].source
            && !source.navigation().is_empty()
        {
            targets.push(RowTarget {
                visible: source.style().visibility.is_visible(),
                row: start,
                rect: areas.rect(columns.clone(), start..end, band.start),
                offset: super::model::relative_offset(
                    &[Some(source)],
                    areas.columns.table,
                    areas.columns.rtl,
                ),
                navigation: source.navigation(),
            });
        }
        start = end;
    }
    for row in band.clone() {
        let navigation = data.rows[row].navigation();
        if !navigation.is_empty() {
            targets.push(RowTarget {
                visible: data.rows[row].style().visibility.is_visible(),
                row,
                rect: areas.rect(columns.clone(), row..row + 1, band.start),
                offset: super::model::relative_offset(
                    &[
                        data.groups[data.row_groups[row]].source,
                        Some(data.rows[row]),
                    ],
                    areas.columns.table,
                    areas.columns.rtl,
                ),
                navigation,
            });
        }
    }
    targets
}
