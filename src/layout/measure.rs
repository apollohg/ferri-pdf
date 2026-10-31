use crate::document::{BlockBox, BlockContent, BoxNavigation};
use crate::graphics::{BoxDecoration, BoxPaint, Rect};
use crate::layout::{
    LayoutError,
    breaks::Fragmentainer,
    margin::MarginStrut,
    multicol::{ColumnFlow, ColumnGeometry, MeasuredColumns, SpannerFragment},
};
use crate::style::{BreakBetween, Display, Style};
use crate::text::{Line, TextContext};

pub(super) mod frames;
mod metrics;
mod space;
pub(super) use frames::{Failure, FrameKey};
pub(super) use metrics::{ItemMetrics, MetricsKey};
pub(super) use space::FlowSpace;

/// Cached paragraph intrinsic widths at most; the cache empties when full.
const MAX_PARAGRAPH_WIDTHS: usize = 4096;
/// Paragraph intrinsic widths by the paragraph's address and the widths of its atomic inlines
/// and of their `::first-line` boxes.
pub(super) type ParagraphWidths = std::rc::Rc<
    std::cell::RefCell<std::collections::HashMap<(usize, Vec<u32>), crate::text::IntrinsicWidths>>,
>;

#[derive(Clone, Copy)]
pub(super) struct BlockSizing {
    pub natural_height: f32,
    edges: [f32; 2],
    pub used_height: Option<f32>,
    alignment: f32,
}
impl BlockSizing {
    fn new(
        natural_height: f32,
        geometry: &crate::layout::geometry::HorizontalBox,
        used_height: Option<f32>,
    ) -> Self {
        Self {
            natural_height,
            edges: [
                geometry.padding[0] + geometry.border[0],
                geometry.padding[2] + geometry.border[2],
            ],
            used_height,
            alignment: 0.0,
        }
    }

    pub(crate) fn resolve(
        &self,
        style: &Style,
        geometry: &crate::layout::geometry::HorizontalBox,
        containing_height: Option<f32>,
        fragment: Option<SpannerFragment>,
    ) -> (f32, bool) {
        let top_delta = geometry.padding[0] + geometry.border[0] - self.edges[0];
        let bottom_delta = geometry.padding[2] + geometry.border[2] - self.edges[1];
        let vertical =
            crate::layout::geometry::VerticalBox::new(style, geometry, containing_height)
                .with_used_height(
                    self.used_height
                        .map(|height| (height - self.alignment).max(0.0)),
                );
        if let Some(mut fragment) = fragment {
            fragment.natural_height = (fragment.natural_height + top_delta + bottom_delta).max(0.0);
            let natural = self.natural_height
                + if fragment.first || style.clone_decoration {
                    top_delta
                } else {
                    0.0
                }
                + if fragment.last || style.clone_decoration {
                    bottom_delta
                } else {
                    0.0
                };
            fragment.resolve_size(style, geometry, &vertical, natural.max(0.0))
        } else {
            let natural = (self.natural_height + top_delta + bottom_delta).max(0.0);
            let height = vertical.used_height(natural);
            (height, height != natural || vertical.definite().is_some())
        }
    }
}

#[derive(Clone)]
pub(super) struct MeasuredBlock<'a> {
    pub(super) frame_key: Option<std::rc::Rc<FrameKey>>,
    pub(super) frame_owner: Option<Box<FrameOwner<'a>>>,
    pub(super) math: Option<Box<crate::layout::mathml::MathBox<'a>>>,
    pub grid: Option<std::rc::Rc<crate::layout::grid::Geometry<'a>>>,
    pub source: &'a BlockBox,
    pub geometry: crate::layout::geometry::HorizontalBox,
    pub containing_height: Option<f32>,
    pub style: &'a Style,
    pub canvas_source: bool,
    pub offset: [f32; 2],
    pub before: MarginStrut<'a>,
    pub after: MarginStrut<'a>,
    pub clearance_before: Option<Box<MarginStrut<'a>>>,
    pub clearance_target: Option<usize>,
    pub through: bool,
    pub height: f32,
    pub sizing: Option<BlockSizing>,
    pub constrained: bool,
    /// Establishes an independent formatting context, so margins inside it never adjoin a
    /// break outside it (CSS Fragmentation 3 §5.2).
    pub independent: bool,
    pub overflow_height: f32,
    pub first_fragment: f32,
    pub has_forced: bool,
    pub break_before: BreakBetween,
    pub break_after: BreakBetween,
    pub items: Vec<FlowItem<'a>>,
    pub paint: Option<BoxPaint>,
    /// The line fragments a box laid out as one paints in place of `paint`'s ink: an inline
    /// initial letter whose contents wrap (CSS Inline 3 §7.8.4).
    pub line_fragments: Vec<BoxPaint>,
    pub navigation: BoxNavigation,
    pub end_space: f32,
    pub start_space: f32,
    pub repeat_space: f32,
    pub marker: Option<Box<crate::layout::markers::MeasuredMarker>>,
    pub table: Option<Box<TableFlow<'a>>>,
    /// A fragment before a spanner of a box the spanner splits: the box continues after it.
    pub continues: bool,
    pub spanner_fragment: Option<SpannerFragment>,
}
impl MeasuredBlock<'_> {
    pub(crate) fn align_cell(&mut self, offset: f32, assigned_height: f32) {
        if self.style.display == Display::TableCell {
            crate::layout::writing::align_cell_frame(self, offset, assigned_height);
        }
        if let Some(sizing) = &mut self.sizing {
            sizing.alignment = offset;
            (self.height, self.constrained) = sizing.resolve(
                self.style,
                &self.geometry,
                self.containing_height,
                self.spanner_fragment,
            );
        }
    }

    /// Resize the retained piece without reconstructing source consumed before its spanner.
    pub(crate) fn resize_fragment(
        &mut self,
        geometry: crate::layout::geometry::HorizontalBox,
        containing_height: Option<f32>,
        fragment: SpannerFragment,
    ) {
        let top = geometry.padding[0] + geometry.border[0];
        let bottom = geometry.padding[2] + geometry.border[2];
        if (fragment.first || self.style.clone_decoration)
            && let Some(FlowItem::Edge(BlockEdge::Start, space)) = self.items.first_mut()
        {
            *space = top;
        }
        if (fragment.last || self.style.clone_decoration)
            && let Some(FlowItem::Edge(BlockEdge::End, space)) = self.items.last_mut()
        {
            *space = bottom;
        }
        (self.height, self.constrained) = self
            .sizing
            .expect("spanner pieces retain their source sizing")
            .resolve(self.style, &geometry, containing_height, Some(fragment));
        self.spanner_fragment = Some(fragment);
        self.overflow_height = overflow_height(&self.items).max(self.height);
        self.first_fragment =
            crate::layout::breaks::first_fragment(&self.items).min(self.overflow_height);
        self.start_space = top;
        self.end_space = if fragment.last || self.style.clone_decoration {
            bottom
        } else {
            0.0
        };
        self.offset = self.style.relative_offset(
            geometry.containing_width,
            containing_height,
            geometry.containing_rtl,
        );
        if let Some(paint) = &mut self.paint {
            paint.rect = geometry.border_box(paint.rect.height);
            paint.padding = geometry.padding;
        }
        self.geometry = geometry;
        self.containing_height = containing_height;
    }

    pub(crate) fn retain_fragment(&mut self) {
        self.paint.get_or_insert_with(|| BoxPaint {
            padding: self.geometry.padding,
            ..BoxPaint::whole(
                self.geometry.border_box(self.height),
                BoxDecoration::default(),
            )
        });
    }

    pub(crate) fn first_baseline(&self) -> Option<f32> {
        first_baseline(&self.items)
    }
    /// Baseline of the last line box in flow, from the border-box top.
    pub(crate) fn last_baseline(&self) -> Option<f32> {
        last_baseline(&self.items)
    }
}

/// Baseline of the first line box in a flow, from its top.
pub(super) fn first_baseline(items: &[FlowItem<'_>]) -> Option<f32> {
    let mut y = 0.0;
    for item in items {
        let baseline = match item {
            FlowItem::Paragraph(paragraph) => paragraph
                .lines
                .iter()
                .find(|line| !line.phantom)
                .map(|line| line.ascent),
            FlowItem::Block(block) | FlowItem::Spanner(block) => block.first_baseline(),
            FlowItem::Columns(columns) => columns.baselines[0],
            FlowItem::Frame { baselines, .. } => baselines[0],
            FlowItem::Row(row) => row.baselines.map_or_else(
                || {
                    row.cells.first().and_then(|cell| {
                        cell.block.first_baseline().map(|value| value + cell.offset)
                    })
                },
                |baselines| baselines[0],
            ),
            _ => None,
        };
        if let Some(baseline) = baseline {
            return Some(y + baseline);
        }
        y += item.height();
    }
    None
}

/// Baseline of the last line box in a flow, from its top.
pub(super) fn last_baseline(items: &[FlowItem<'_>]) -> Option<f32> {
    let mut y = items.iter().map(FlowItem::height).sum::<f32>();
    for item in items.iter().rev() {
        y -= item.height();
        let baseline = match item {
            FlowItem::Paragraph(paragraph) => paragraph
                .lines
                .iter()
                .rev()
                .find(|line| !line.phantom)
                .map(|line| paragraph.height - line.height + line.ascent),
            FlowItem::Block(block) | FlowItem::Spanner(block) => block.last_baseline(),
            FlowItem::Columns(columns) => columns.baselines[1],
            FlowItem::Frame { baselines, .. } => baselines[1],
            FlowItem::Row(row) => row.baselines.map_or_else(
                || {
                    row.cells.first().and_then(|cell| {
                        cell.block.first_baseline().map(|value| value + cell.offset)
                    })
                },
                |baselines| baselines[1],
            ),
            _ => None,
        };
        if let Some(baseline) = baseline {
            return Some(y + baseline);
        }
    }
    None
}

#[derive(Clone)]
pub(super) struct MeasuredParagraph<'a> {
    pub index: usize,
    pub paragraph: &'a crate::document::Paragraph,
    pub style: &'a Style,
    pub x: f32,
    pub width: f32,
    pub containing_height: Option<f32>,
    pub height: f32,
    pub lines: Vec<Line>,
    pub positions: Vec<crate::text::TextPosition<'a>>,
    pub anchors: Vec<LineAnchor<'a>>,
    /// Atomic inlines in paragraph order, taken when their line is placed.
    pub atomics: Vec<Option<crate::layout::inline::MeasuredAtomic<'a>>>,
    /// The `::first-line` boxes of the atomic inlines that have one, taken when the first
    /// formatted line is placed.
    pub first_line_atomics: Vec<Option<crate::layout::inline::MeasuredAtomic<'a>>>,
    /// The region break of a `continue: discard` container that ends the paragraph.
    pub clamp: Option<crate::text::ParagraphClamp>,
}

impl MeasuredParagraph<'_> {
    pub(super) fn is_phantom(&self) -> bool {
        self.line_count() == 0
    }

    fn line_count(&self) -> usize {
        self.lines.iter().filter(|line| !line.phantom).count()
    }
}

/// The line boxes a `continue: discard` container still holds before its region break
/// (CSS Overflow 4 §5.2), and whether in-flow content follows the measured box inside it.
#[derive(Clone, Copy)]
pub(super) struct LineBudget {
    pub lines: usize,
    pub following: bool,
}

/// CSS Overflow 4 §5.3: `continue: discard` makes a block container a fragmentation container
/// that captures region breaks, whose content after the first is not rendered.
fn discards(style: &Style) -> bool {
    style.clamp.continuation == crate::style::Continue::Discard
        && matches!(
            style.display,
            Display::Block
                | Display::ListItem
                | Display::FlowRoot
                | Display::FlowRootListItem
                | Display::InlineBlock
                | Display::TableCell
                | Display::TableCaption
        )
}

/// Counts the line boxes of a measured flow in its block formatting context, and those of
/// them ending within `limit` of the flow's top, until the first that does not.
fn lines_within(items: &[FlowItem<'_>], top: f32, limit: f32, counts: &mut [usize; 2]) {
    let mut y = top;
    for item in items {
        match item {
            FlowItem::Paragraph(paragraph) => {
                for line in &paragraph.lines {
                    y += line.height;
                    if line.phantom {
                        continue;
                    }
                    if counts[0] == counts[1] && y <= limit {
                        counts[0] += 1;
                    }
                    counts[1] += 1;
                }
                continue;
            }
            FlowItem::Block(block) if !block.independent => {
                lines_within(&block.items, y, limit, counts);
            }
            _ => {}
        }
        y += item.height();
    }
}
/// A collapsing wrapper contributes only ignored source lines, without independent content.
fn phantom_flow(block: &MeasuredBlock<'_>) -> bool {
    let mut pending = vec![block];
    let mut phantom = false;
    while let Some(block) = pending.pop() {
        if !block.through
            || block.independent
            || block.marker.is_some()
            || block.has_forced
            || block.before.value() != 0.0
            || block.after.value() != 0.0
            || block
                .paint
                .as_ref()
                .is_some_and(|paint| paint.decoration.is_visible())
        {
            return false;
        }
        for item in &block.items {
            match item {
                FlowItem::Paragraph(paragraph) if paragraph.is_phantom() => phantom = true,
                FlowItem::Block(child) => pending.push(child),
                FlowItem::Events(_) | FlowItem::Positioned(_) => {}
                FlowItem::Edge(_, value) | FlowItem::Space(value) if *value == 0.0 => {}
                FlowItem::Margin(margin) if margin.value() == 0.0 => {}
                _ => return false,
            }
        }
    }
    phantom
}

/// Absolutely positioned box anchored in a paragraph line.
#[derive(Clone)]
pub(super) struct LineAnchor<'a> {
    pub block: &'a BlockBox,
    pub line: usize,
    /// Static x from the line's left edge; block-level boxes span the paragraph instead.
    pub x: Option<f32>,
    /// Block-level boxes sit below the line holding the preceding content.
    pub below: bool,
    /// Innermost inline box holding the anchor.
    pub inline_box: Option<usize>,
}
/// Out-of-flow box anchored between flow items.
#[derive(Clone)]
pub(super) struct PositionedFlow<'a> {
    pub block: &'a BlockBox,
    pub left: f32,
    pub right: f32,
    /// Collapsed margin preceding the hypothetical box.
    pub margin: f32,
    pub alignment: Option<crate::layout::positioned::StaticAlignment>,
}
#[derive(Clone)]
pub(super) struct MeasuredCell<'a> {
    pub clipped_height: Option<f32>,
    pub after_events: Vec<&'a crate::document::SemanticEvent>,
    pub grid_cell: Option<usize>,
    pub backgrounds: Vec<crate::layout::tables::FragmentBackground<'a>>,
    pub block: MeasuredBlock<'a>,
    pub offset: f32,
    /// Unaligned start of a fragmented table cell's border box within its band.
    pub paint_offset: Option<f32>,
    pub stretch: bool,
    pub after: f32,
    /// Flex items honour `z-index` without positioning.
    pub flex_item: bool,
}
#[derive(Clone)]
pub(super) struct RowTarget {
    pub visible: bool,
    pub row: usize,
    pub rect: Rect,
    pub offset: [f32; 2],
    pub navigation: BoxNavigation,
}
#[derive(Clone)]
pub(super) struct CollapsedItem<'a> {
    pub source: &'a BlockBox,
    pub offset: f32,
    pub after_events: Vec<&'a crate::document::SemanticEvent>,
}

#[derive(Clone)]
pub(super) struct RowText<'a> {
    pub source: &'a crate::document::Paragraph,
    pub line: Line,
    /// Layout x before paginator offsets; the baseline is relative to the row top.
    pub x: f32,
    pub baseline: f32,
}

#[derive(Clone)]
pub(super) enum RowPaint<'a> {
    Box(Box<BoxPaint>),
    Text(Box<RowText<'a>>),
}

#[derive(Clone)]
pub(super) struct MeasuredRow<'a> {
    pub grid: Option<std::rc::Rc<crate::layout::grid::Rows>>,
    pub baselines: Option<[Option<f32>; 2]>,
    pub collapsed: Vec<CollapsedItem<'a>>,
    pub background_rows: Vec<crate::layout::tables::BackgroundRows>,
    pub cell_heights: Vec<(usize, f32)>,
    pub height: f32,
    pub first_fragment: f32,
    pub paints: Vec<BoxPaint>,
    /// Root-owned paints emitted once after child branches restore the row layer.
    pub after: Vec<RowPaint<'a>>,
    /// Originating grid rows and border boxes of rows, groups and cells carrying navigation.
    pub targets: Vec<RowTarget>,
    pub cells: Vec<MeasuredCell<'a>>,
    pub break_before: BreakBetween,
    pub break_after: BreakBetween,
    /// Grid rows of a table band; flex lines have none.
    pub band: Option<std::ops::Range<usize>>,
}
/// Table row groups repeated on every page and collapsed borders painted once placed.
#[derive(Clone)]
pub(super) struct TableFlow<'a> {
    pub row_starts: std::rc::Rc<[f32]>,
    pub original_row_starts: Option<std::rc::Rc<[f32]>>,
    pub plan: std::rc::Rc<crate::layout::tables::TablePlan<'a>>,
    pub geometry: crate::layout::geometry::HorizontalBox,
    pub heights: Vec<crate::table::CellHeight>,
    /// Header and footer bands, empty unless they repeat.
    pub header: Vec<MeasuredRow<'a>>,
    pub footer: Vec<MeasuredRow<'a>>,
    /// Grid rows between the header and footer groups.
    pub body: std::ops::Range<usize>,
    /// Height reserved below body bands for the repeated footer.
    pub footer_height: f32,
    /// Vertical border spacing following every band.
    pub spacing: f32,
    pub collapsed: Option<crate::layout::tables::CollapsedGrid>,
}
#[derive(Clone, Copy)]
pub(super) enum BlockEdge {
    Start,
    End,
}

#[derive(Clone)]
pub(super) struct MeasuredTableBox<'a> {
    pub area: crate::layout::tables::TableContaining<'a>,
    pub edges: [BreakBetween; 2],
    pub events: Vec<&'a crate::document::SemanticEvent>,
}

#[derive(Clone)]
pub(super) enum FlowItem<'a> {
    Events(&'a [crate::document::SemanticEvent]),
    TableBox(Box<MeasuredTableBox<'a>>),
    Frame {
        inner: std::rc::Rc<MeasuredBlock<'a>>,
        area: Rect,
        height: f32,
        baselines: [Option<f32>; 2],
    },
    Image {
        paint: crate::graphics::ImagePaint,
        visible: bool,
        height: f32,
    },
    Block(Box<MeasuredBlock<'a>>),
    Paragraph(MeasuredParagraph<'a>),
    Margin(MarginStrut<'a>),
    Space(f32),
    Edge(BlockEdge, f32),
    Row(Box<MeasuredRow<'a>>),
    Positioned(PositionedFlow<'a>),
    /// The content of a multi-column container.
    Columns(Box<MeasuredColumns<'a>>),
    /// A `column-span: all` box measured across the multi-column container.
    Spanner(Box<MeasuredBlock<'a>>),
}
impl FlowItem<'_> {
    pub(crate) fn height(&self) -> f32 {
        match self {
            Self::Image { height, .. } | Self::Frame { height, .. } => *height,
            Self::Block(block) | Self::Spanner(block) => block.height,
            Self::Columns(columns) => columns.height,
            Self::Paragraph(paragraph) => paragraph.height,
            Self::Row(row) => row.height,
            Self::Margin(margin) => margin.value(),
            Self::Space(value) | Self::Edge(_, value) => *value,
            Self::Positioned(_) | Self::Events(_) | Self::TableBox(_) => 0.0,
        }
    }
}

/// Immediate native-frame dependencies; their own descendants are separate jobs.
pub(super) fn frame_dependencies<'a>(
    root: &MeasuredBlock<'a>,
) -> Vec<std::rc::Rc<MeasuredBlock<'a>>> {
    enum Entry<'r, 'a> {
        Block(&'r MeasuredBlock<'a>),
        Item(&'r FlowItem<'a>),
        Row(&'r MeasuredRow<'a>),
    }
    let mut pending = vec![Entry::Block(root)];
    let mut seen = std::collections::HashSet::new();
    let mut frames = Vec::new();
    while let Some(entry) = pending.pop() {
        match entry {
            Entry::Block(block) => {
                if let Some(table) = &block.table {
                    pending.extend(table.footer.iter().rev().map(Entry::Row));
                }
                pending.extend(block.items.iter().rev().map(Entry::Item));
                if let Some(table) = &block.table {
                    pending.extend(table.header.iter().rev().map(Entry::Row));
                }
            }
            Entry::Row(row) => {
                pending.extend(row.cells.iter().rev().map(|cell| Entry::Block(&cell.block)));
            }
            Entry::Item(item) => match item {
                FlowItem::Frame { inner, .. } => {
                    if seen.insert(std::rc::Rc::as_ptr(inner)) {
                        frames.push(inner.clone());
                    }
                }
                FlowItem::Block(block) | FlowItem::Spanner(block) => {
                    pending.push(Entry::Block(block));
                }
                FlowItem::Row(row) => pending.push(Entry::Row(row)),
                FlowItem::Paragraph(paragraph) => pending.extend(
                    paragraph
                        .atomics
                        .iter()
                        .chain(&paragraph.first_line_atomics)
                        .rev()
                        .flatten()
                        .map(|atomic| Entry::Block(&atomic.block)),
                ),
                FlowItem::Columns(columns) => {
                    for segment in columns.segments.iter().rev() {
                        let (crate::layout::multicol::ColumnSegment::Row(items)
                        | crate::layout::multicol::ColumnSegment::Spanner(items)) = segment;
                        pending.extend(items.iter().rev().map(Entry::Item));
                    }
                }
                FlowItem::TableBox(_)
                | FlowItem::Events(_)
                | FlowItem::Image { .. }
                | FlowItem::Margin(_)
                | FlowItem::Space(_)
                | FlowItem::Edge(_, _)
                | FlowItem::Positioned(_) => {}
            },
        }
    }
    frames
}

pub(super) fn overflow_height(items: &[FlowItem<'_>]) -> f32 {
    let mut position = 0.0;
    let mut extent = 0.0f32;
    for item in items {
        let height = match item {
            FlowItem::Block(block) | FlowItem::Spanner(block) => block.overflow_height,
            FlowItem::Row(row) => row.cells.iter().fold(row.height, |height, cell| {
                height.max(cell.offset + cell.block.overflow_height)
            }),
            _ => item.height(),
        };
        extent = extent.max(position + height);
        position += item.height();
    }
    extent
}

/// A box's own selected style with its canonical content and layout identity.
#[derive(Clone, Copy)]
pub(crate) struct BlockSource<'a> {
    pub block: &'a BlockBox,
    pub selected: &'a Style,
}
impl<'a> From<&'a BlockBox> for BlockSource<'a> {
    fn from(block: &'a BlockBox) -> Self {
        Self {
            block,
            selected: block.style(),
        }
    }
}
impl std::ops::Deref for BlockSource<'_> {
    type Target = BlockBox;
    fn deref(&self) -> &Self::Target {
        self.block
    }
}
impl<'a> BlockSource<'a> {
    pub(crate) fn style(self) -> &'a Style {
        self.selected
    }
    pub(crate) fn children(self) -> &'a [BlockContent] {
        self.block.children()
    }
    pub(crate) fn marker(self) -> Option<&'a crate::document::Paragraph> {
        self.block.marker()
    }
    pub(crate) fn flow(self) -> Option<&'a BlockBox> {
        self.block.flow()
    }
    pub(super) fn collapsed_table(self) -> Option<&'a BlockBox> {
        self.flow().filter(|inner| {
            inner.style().display.is_table()
                && inner.style().table.collapse == crate::style::BorderCollapse::Collapse
        })
    }

    pub(crate) fn layout_flow(self) -> crate::style::logical::Flow {
        self.block.layout_flow()
    }
}

#[derive(Clone)]
pub(super) struct FrameOwner<'a> {
    pub source: BlockSource<'a>,
    pub paint: BoxPaint,
    pub containing: Rect,
    pub padding: [f32; 4],
}

pub(super) struct NativeSizing {
    pub source: *const BlockBox,
    pub vertical: crate::layout::geometry::VerticalBox,
    pub scroll_container: bool,
}

#[derive(Clone)]
pub(super) struct Measurer<'a> {
    pub(super) native_sizing: Option<std::rc::Rc<NativeSizing>>,
    /// The elements whose renderings `element()` images show.
    pub(super) element_targets: std::rc::Rc<crate::layout::ElementTargets>,
    pub(super) table_data: std::rc::Rc<crate::layout::tables::Inputs<'a>>,
    pub(super) frame_measurements: std::rc::Rc<std::cell::RefCell<frames::Measurements<'a>>>,
    pub(super) raw_templates:
        std::rc::Rc<std::cell::RefCell<crate::layout::pagination::frames::Templates<'a>>>,
    pub(super) raw_pagination: bool,
    pub(super) column_measurement: bool,
    pub(super) orthogonal_columns: Option<crate::layout::multicol::OrthogonalColumns>,
    pub(super) space: FlowSpace,
    pub work: crate::layout::work::Budget,
    /// What the layout pass degraded, shared by every probe.
    pub diagnostics: crate::layout::Diagnostics,
    pub text: &'a TextContext<'a>,
    pub canvas_images: bool,
    pub motion: std::rc::Rc<crate::layout::motion::Resources>,
    pub page_height: f32,
    pub sources: crate::layout::source::SourceIds,
    pub clearance_targets: std::collections::HashMap<*const BlockBox, usize>,
    pub grid_parent: Option<crate::layout::grid::ParentContext>,
    pub item_metrics: std::collections::HashMap<MetricsKey, ItemMetrics>,
    /// Paragraphs' intrinsic widths, shared by every probe, as nested shrink-to-fit boxes
    /// measure their contents again at each level.
    pub(super) paragraph_widths: ParagraphWidths,
    /// The fragmentainer of the flow being measured.
    pub fragmentainer: Fragmentainer,
    /// The multi-column container whose formatting context the flow belongs to.
    pub column_flow: Option<ColumnFlow>,
    /// The spanner being measured, which establishes a formatting context (§6.1).
    pub spanner: Option<*const BlockBox>,
    /// The flex or grid item being measured at the border-box width its container resolved.
    pub sized_item: Option<*const BlockBox>,
    /// The box avoiding floats being measured in the inline space beside them: its left edge
    /// and width (CSS 2.1 §9.5).
    pub float_space: Option<(*const BlockBox, [f32; 2])>,
    /// The flow is laid out without page breaks: inside an atomic inline or an absolutely
    /// positioned box.
    pub unbroken: bool,
    /// Named-page boundaries are excluded beneath table cells.
    pub named_pages: bool,
    /// The content-box size of each query container's first box fragment.
    pub container_sizes:
        std::collections::HashMap<crate::dom::ElementId, crate::style::ContainerMeasurement>,
    /// The part of its contents' paragraph that the ruby annotation box being measured lays
    /// out, on a line breaking inside its bases (CSS Ruby 1 §3.4.2).
    pub annotation_part: Option<(*const crate::document::Paragraph, std::ops::Range<usize>)>,
    /// The pages that calls in nested layouts must leave for their footnotes to fit.
    pub footnote_deferrals: std::rc::Rc<crate::layout::pagination::FootnoteDeferrals>,
}
impl<'a> Measurer<'a> {
    pub(super) fn new(
        text: &'a TextContext<'a>,
        root: &'a BlockBox,
        size: [f32; 2],
        work: crate::layout::work::Budget,
    ) -> Self {
        Measurer {
            native_sizing: None,
            element_targets: std::rc::Rc::default(),
            table_data: Default::default(),
            frame_measurements: Default::default(),
            raw_templates: Default::default(),
            raw_pagination: false,
            column_measurement: false,
            orthogonal_columns: None,
            space: FlowSpace::new(crate::style::logical::Flow::used(root.style()), size),
            work,
            diagnostics: crate::layout::Diagnostics::default(),
            text,
            canvas_images: false,
            motion: std::rc::Rc::new(crate::layout::motion::Resources::new(
                Default::default(),
                text.fonts(),
            )),
            page_height: size[1],
            sources: crate::layout::source::SourceIds::new(root),
            item_metrics: Default::default(),
            paragraph_widths: Default::default(),
            grid_parent: None,
            clearance_targets: Default::default(),
            fragmentainer: Fragmentainer::Page,
            column_flow: None,
            spanner: None,
            sized_item: None,
            float_space: None,
            unbroken: false,
            named_pages: true,
            container_sizes: Default::default(),
            annotation_part: None,
            footnote_deferrals: std::rc::Rc::default(),
        }
    }

    pub(super) fn native_vertical(
        &self,
        block: &BlockBox,
    ) -> Option<crate::layout::geometry::VerticalBox> {
        self.native_sizing
            .as_ref()
            .filter(|sizing| std::ptr::eq(sizing.source, block))
            .map(|sizing| sizing.vertical.clone())
    }

    pub(super) fn vertical_box(
        &self,
        block: BlockSource<'_>,
        vertical: crate::layout::geometry::VerticalBox,
    ) -> crate::layout::geometry::VerticalBox {
        self.native_vertical(block.block).unwrap_or(vertical)
    }

    pub(super) fn with_space<T>(
        &mut self,
        space: FlowSpace,
        compute: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = std::mem::replace(&mut self.space, space);
        let result = compute(self);
        self.space = previous;
        result
    }

    /// Measures `item`, a flex or grid item, at the width its container resolved for it.
    pub(super) fn with_sized_item<T>(
        &mut self,
        item: &BlockBox,
        compute: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = self.sized_item.replace(std::ptr::from_ref(item));
        let result = compute(self);
        self.sized_item = previous;
        result
    }

    /// The inline space beside floats that `block`, avoiding them, is measured in.
    fn float_space(&self, block: &BlockBox) -> Option<[f32; 2]> {
        self.float_space
            .filter(|(source, _)| std::ptr::eq(*source, block))
            .map(|(_, space)| space)
    }

    /// Whether `block`'s container resolved its width: its flexed, stretched or fit-content
    /// size as a flex or grid item.
    pub(super) fn sized_by_container(&self, block: &BlockBox) -> bool {
        self.sized_item
            .is_some_and(|item| std::ptr::eq(item, block))
    }

    pub(crate) fn intrinsic_widths(
        &self,
        paragraph: &crate::document::Paragraph,
        atomics: &[crate::text::IntrinsicWidths],
        first_line_atomics: &[crate::text::IntrinsicWidths],
    ) -> Result<crate::text::IntrinsicWidths, LayoutError> {
        let key = (
            std::ptr::from_ref(paragraph) as usize,
            atomics
                .iter()
                .chain(first_line_atomics)
                .flat_map(|widths| [widths.min.to_bits(), widths.max.to_bits()])
                .collect(),
        );
        if let Some(widths) = self.paragraph_widths.borrow().get(&key) {
            self.work.spend(1)?;
            return Ok(*widths);
        }
        self.work.spend(paragraph.text().len().max(1))?;
        let widths = self
            .text
            .intrinsic_widths(paragraph, atomics, first_line_atomics)?;
        let mut cache = self.paragraph_widths.borrow_mut();
        if cache.len() >= MAX_PARAGRAPH_WIDTHS {
            cache.clear();
        }
        cache.insert(key, widths);
        Ok(widths)
    }

    pub(crate) fn text_lines(
        &self,
        paragraph: &crate::document::Paragraph,
        width: f32,
        atomics: &[crate::text::AtomicMetrics],
    ) -> Result<Vec<Line>, LayoutError> {
        self.work.spend(paragraph.text().len().max(1))?;
        Ok(self.text.layout_paragraph(paragraph, width, atomics)?)
    }

    /// The lines of `paragraph`, the content of a `continue: discard` container whose block
    /// size is `extent` and whose clamp is `clamp`: at most `max-lines` line boxes and those
    /// before the content overflows `extent`, never fewer than one, the last ellipsized when
    /// content follows it (CSS Overflow 4 §5.2–5.3, §4.4).
    pub(crate) fn discarded_text_lines(
        &self,
        paragraph: &crate::document::Paragraph,
        width: f32,
        atomics: &[crate::text::AtomicMetrics],
        extent: f32,
        clamp: &crate::style::LineClamp,
    ) -> Result<Vec<Line>, LayoutError> {
        let lines = self.text_lines(paragraph, width, atomics)?;
        let mut used = 0.0;
        let fitting = lines
            .iter()
            .take_while(|line| {
                used += line.height;
                !crate::layout::pagination::overflows(used, extent)
            })
            .count();
        let kept = clamp.max_lines.unwrap_or(usize::MAX).min(fitting.max(1));
        if kept >= lines.len() {
            return Ok(lines);
        }
        self.work.spend(paragraph.text().len().max(1))?;
        let prepared =
            self.text
                .prepare_paragraph(paragraph)
                .clamped(Some(crate::text::ParagraphClamp {
                    lines: kept,
                    ellipsis: Some(clamp.block_ellipsis.clone())
                        .filter(|ellipsis| *ellipsis != crate::style::BlockEllipsis::None),
                }));
        Ok(prepared
            .lines(prepared.start(), width, atomics)?
            .collect::<Result<_, _>>()?)
    }

    /// `block` laid out at `width` as the only content of a `continue: discard` container
    /// whose block size is `extent` and which keeps at most `max_lines` line boxes: the line
    /// boxes before its content overflows `extent`, never fewer than one (CSS Overflow 4
    /// §5.2–5.3).
    pub(crate) fn discarded_block(
        &mut self,
        block: &'a BlockBox,
        width: f32,
        rtl: bool,
        extent: f32,
        max_lines: Option<usize>,
    ) -> Result<MeasuredBlock<'a>, LayoutError> {
        self.run(|measurer| {
            let measured = measurer.measure_block(block, 0.0, width, rtl, false, None)?;
            let mut counts = [0; 2];
            lines_within(
                &measured.items,
                measured.before.value(),
                extent,
                &mut counts,
            );
            let [fitting, total] = counts;
            let lines = max_lines.unwrap_or(usize::MAX).min(fitting.max(1));
            if lines >= total {
                return Ok(measured);
            }
            let geometry = block_geometry(measurer, block, 0.0, width, rtl, None)?;
            measurer.format_flow(
                block.into(),
                geometry,
                false,
                None,
                None,
                Some(LineBudget {
                    lines,
                    following: true,
                }),
            )
        })
    }

    pub(super) fn box_edges(
        &self,
        block: impl Into<BlockSource<'a>>,
        width: f32,
    ) -> Result<crate::layout::geometry::Edges, Failure> {
        let block = block.into();
        if let Some(inner) = block.collapsed_table() {
            let own = crate::style::logical::Flow::used(inner.style());
            let parent = block.layout_flow();
            let edges = self
                .table_data
                .get(inner)?
                .table_edges(inner.style(), width);
            return Ok(crate::layout::geometry::Edges {
                padding: parent
                    .horizontal()
                    .sides(parent, &own.sides(own.horizontal(), &edges.padding)),
                border: parent
                    .horizontal()
                    .sides(parent, &own.sides(own.horizontal(), &edges.border)),
            });
        }
        if block.style().display.is_table() {
            return Ok(self
                .table_data
                .get(block)?
                .table_edges(block.style(), width));
        }
        Ok(crate::layout::geometry::Edges::of(block.style(), width))
    }

    pub(super) fn box_paint<'b>(
        &self,
        block: impl Into<BlockSource<'b>>,
        geometry: &crate::layout::geometry::HorizontalBox,
    ) -> Option<BoxPaint> {
        let block = block.into();
        let style = block.style();
        let mut decoration = block.decoration().clone();
        if block.is_canvas_source() && !self.canvas_images {
            decoration.clear_background();
        }
        // Clips, containing blocks, scrollports, mask and clip-path areas and navigation need
        // their page fragments even without ink, as do float exclusion contours.
        let fragments = (block.is_canvas_source() && self.canvas_images)
            || style.floating().is_some()
            || style.clipped_axes().contains(&true)
            || style.decoration.masks.is_masked()
            || style.contains_absolute()
            || style.is_scroll_container()
            || contains_child(block.block, |style| {
                is_sticky(style) || style.motion.active()
            })
            || bounds_enclosing_inline(block.block)
            || !block.navigation().is_empty();
        let rect = geometry.border_box(0.0);
        (style.display != Display::None && (decoration.is_visible() || fragments)).then(|| {
            BoxPaint {
                draw_background: !block.is_canvas_source(),
                padding: geometry.padding,
                margins: geometry.used_margins(),
                ..BoxPaint::whole(rect, decoration)
            }
        })
    }
    pub(crate) fn block(
        &mut self,
        block: &'a BlockBox,
        x: f32,
        width: f32,
        rtl: bool,
        root: bool,
        height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, LayoutError> {
        self.run(|measurer| measurer.measure_block(block, x, width, rtl, root, height))
    }
    pub(super) fn format_block(
        &mut self,
        block: &'a BlockBox,
        geometry: &crate::layout::geometry::HorizontalBox,
        root: bool,
        containing: Option<f32>,
        height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, LayoutError> {
        self.run(|measurer| {
            measurer.measure_format(block, geometry.clone(), root, containing, height)
        })
    }
    pub(crate) fn measure_block(
        &mut self,
        block: &'a BlockBox,
        x: f32,
        width: f32,
        containing_rtl: bool,
        root: bool,
        containing_height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let style = block.style();
        if style.display.is_table() {
            self.work.start(block);
            return crate::layout::tables::measure(
                self,
                block,
                x,
                width,
                containing_rtl,
                containing_height,
            );
        }
        let geometry = block_geometry(self, block, x, width, containing_rtl, containing_height)?;
        self.measure_format(block, geometry, root, containing_height, None)
    }

    /// Measures `child` as a spanner across the multi-column container of `flow`.
    fn spanner(
        &mut self,
        child: &'a BlockBox,
        flow: ColumnFlow,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let saved = (self.fragmentainer, self.spanner);
        self.fragmentainer = flow.fragmentainer;
        self.spanner = Some(child);
        let measured = self.measure_block(
            child,
            flow.x,
            flow.width,
            flow.rtl,
            false,
            flow.containing_height,
        );
        (self.fragmentainer, self.spanner) = saved;
        measured
    }

    pub(super) fn measure_format(
        &mut self,
        block: impl Into<BlockSource<'a>>,
        geometry: crate::layout::geometry::HorizontalBox,
        root: bool,
        containing_height: Option<f32>,
        used_height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        self.measure_flow(
            block.into(),
            geometry,
            root,
            containing_height,
            used_height,
            None,
        )
    }

    /// A block's flow; a `continue: discard` container keeps at most `budget` line boxes and
    /// the content before its block size overflows.
    fn measure_flow(
        &mut self,
        block: BlockSource<'a>,
        geometry: crate::layout::geometry::HorizontalBox,
        root: bool,
        containing_height: Option<f32>,
        used_height: Option<f32>,
        budget: Option<LineBudget>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let context = (
            self.fragmentainer,
            self.column_flow,
            self.space,
            self.orthogonal_columns,
        );
        let measured = if discards(block.style()) {
            self.discarding_flow(
                block,
                geometry,
                root,
                containing_height,
                used_height,
                budget,
            )
        } else {
            self.format_flow(block, geometry, root, containing_height, used_height, None)
        };
        (
            self.fragmentainer,
            self.column_flow,
            self.space,
            self.orthogonal_columns,
        ) = context;
        measured
    }

    /// CSS Overflow 4 §5.2–5.3: a region break after `max-lines` line boxes or an earlier
    /// break of an enclosing container, or else the unforced break where content overflows
    /// the block size, which follows the last line box that fits but never precedes the first.
    fn discarding_flow(
        &mut self,
        block: BlockSource<'a>,
        geometry: crate::layout::geometry::HorizontalBox,
        root: bool,
        containing_height: Option<f32>,
        used_height: Option<f32>,
        outer: Option<LineBudget>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let style = block.style();
        let budget = match (style.clamp.max_lines, outer) {
            (Some(lines), Some(outer)) => Some(LineBudget {
                lines: lines.min(outer.lines),
                following: outer.following && outer.lines <= lines,
            }),
            (Some(lines), None) => Some(LineBudget {
                lines,
                following: false,
            }),
            (None, outer) => outer,
        };
        let context = (
            self.fragmentainer,
            self.column_flow,
            self.space,
            self.orthogonal_columns,
        );
        let measured = self.format_flow(
            block,
            geometry.clone(),
            root,
            containing_height,
            used_height,
            budget,
        )?;
        if style.multicol.is_multicol() {
            return self.discarding_columns(
                block,
                &geometry,
                root,
                [containing_height, used_height],
                measured,
                context,
            );
        }
        if measured
            .sizing
            .is_none_or(|sizing| sizing.natural_height <= measured.height)
        {
            return Ok(measured);
        }
        let mut counts = [0; 2];
        lines_within(
            &measured.items,
            0.0,
            measured.height - measured.end_space,
            &mut counts,
        );
        let [fitting, total] = counts;
        if fitting >= total {
            return Ok(measured);
        }
        (
            self.fragmentainer,
            self.column_flow,
            self.space,
            self.orthogonal_columns,
        ) = context;
        self.format_flow(
            block,
            geometry,
            root,
            containing_height,
            used_height,
            Some(LineBudget {
                lines: fitting.max(1),
                following: true,
            }),
        )
    }

    /// CSS Overflow 4 §5.3: a multi-column container's overflow columns follow its unforced
    /// region break, so it keeps the most line boxes whose columns fit, but never fewer than
    /// one.
    fn discarding_columns(
        &mut self,
        block: BlockSource<'a>,
        geometry: &crate::layout::geometry::HorizontalBox,
        root: bool,
        [containing_height, used_height]: [Option<f32>; 2],
        measured: MeasuredBlock<'a>,
        context: (
            Fragmentainer,
            Option<ColumnFlow>,
            FlowSpace,
            Option<crate::layout::multicol::OrthogonalColumns>,
        ),
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let overflow_columns = |measured: &MeasuredBlock<'_>| {
            measured.items.iter().any(|item| {
                matches!(item, FlowItem::Columns(columns) if crate::layout::pagination::overflows(
                    columns.used_inline,
                    columns.geometry.inline_size,
                ))
            })
        };
        if !overflow_columns(&measured) {
            return Ok(measured);
        }
        let mut counts = [0; 2];
        for item in &measured.items {
            if let FlowItem::Columns(columns) = item {
                for segment in &columns.segments {
                    if let crate::layout::multicol::ColumnSegment::Row(items) = segment {
                        lines_within(items, 0.0, f32::INFINITY, &mut counts);
                    }
                }
            }
        }
        let measure = |measurer: &mut Self, lines: usize| {
            (
                measurer.fragmentainer,
                measurer.column_flow,
                measurer.space,
                measurer.orthogonal_columns,
            ) = context;
            measurer.format_flow(
                block,
                geometry.clone(),
                root,
                containing_height,
                used_height,
                Some(LineBudget {
                    lines,
                    following: true,
                }),
            )
        };
        // Bisects between line counts whose columns fit and overflow.
        let [mut fitting, mut overflowing] = [0, counts[1]];
        let mut kept = None;
        while overflowing - fitting > 1 {
            let lines = fitting.midpoint(overflowing);
            let candidate = measure(self, lines)?;
            if overflow_columns(&candidate) {
                overflowing = lines;
            } else {
                fitting = lines;
                kept = Some(candidate);
            }
        }
        match kept {
            Some(measured) => Ok(measured),
            None => measure(self, 1),
        }
    }

    /// Lines the discarding container of the flow still holds, unless an independent
    /// formatting context separates the current frame from it.
    fn budget(frames: &[FlowFrame<'_>]) -> Option<usize> {
        let clamp = frames[0].clamp.as_ref()?;
        (!frames[1..].iter().any(|frame| frame.independent)).then_some(clamp.remaining)
    }

    /// Whether content other than ignored lines follows the prospective region break.
    fn following(&self, frames: &[FlowFrame<'a>]) -> Result<bool, Failure> {
        if frames[0]
            .clamp
            .as_ref()
            .is_some_and(|clamp| clamp.following)
        {
            return Ok(true);
        }
        let mut probe = self.clone();
        for frame in frames.iter().rev() {
            for child in frame.children.clone() {
                let budget = Some(LineBudget {
                    lines: 1,
                    following: true,
                });
                match child {
                    BlockContent::Inline(paragraph) => {
                        if !probe
                            .paragraph(
                                paragraph,
                                frame.content_x,
                                frame.content_width,
                                frame.vertical.definite(),
                                frame.block.style().used_rtl(),
                                budget,
                            )?
                            .is_phantom()
                        {
                            return Ok(true);
                        }
                    }
                    BlockContent::Block(child) => {
                        let style = child.style();
                        if child.image().is_some()
                            || child.flow().is_some()
                            || !matches!(style.display, Display::Block | Display::ListItem)
                            || style.establishes_formatting_context()
                        {
                            return Ok(true);
                        }
                        let mut geometry = block_geometry(
                            &probe,
                            child,
                            frame.content_x,
                            frame.content_width,
                            frame.block.style().used_rtl(),
                            frame.vertical.definite(),
                        )?;
                        frame.collapse_stretch_margins(&mut geometry);
                        let measured = probe.measure_flow(
                            child.into(),
                            geometry,
                            false,
                            frame.vertical.definite(),
                            None,
                            budget,
                        )?;
                        if !phantom_flow(&measured) {
                            return Ok(true);
                        }
                    }
                    BlockContent::Events(_) | BlockContent::Positioned(_) => {}
                }
            }
            (
                probe.fragmentainer,
                _,
                probe.space,
                probe.orthogonal_columns,
            ) = *frame.outer;
            probe.column_flow = frame.restore_column_flow;
        }
        Ok(false)
    }

    /// Counts `lines` against the discarding container. At its region break, content after
    /// the break is not rendered and every box the break crosses ends its first fragment.
    fn spend_lines(frames: &mut [FlowFrame<'_>], lines: usize, broken: bool) {
        let Some(clamp) = frames[0].clamp.as_mut() else {
            return;
        };
        clamp.remaining = clamp.remaining.saturating_sub(lines);
        if broken {
            for frame in frames.iter_mut() {
                frame.children = [].iter();
            }
            for frame in &mut frames[1..] {
                frame.fragmented = true;
            }
        }
    }

    fn start_flow(
        &mut self,
        block: BlockSource<'a>,
        geometry: crate::layout::geometry::HorizontalBox,
        root: bool,
        containing_height: Option<f32>,
        used_height: Option<f32>,
    ) -> Result<FlowStart<'a>, Failure> {
        self.work.start(block.block);
        self.work.spend(1)?;
        let outer = OuterFlow {
            column_flow: self.column_flow,
            space: self.space,
            columns: self.orthogonal_columns,
        };
        let style = block.style();
        let spanning = self
            .spanner
            .is_some_and(|spanner| std::ptr::eq(spanner, block.block));
        if spanning
            || !matches!(style.display, Display::Block | Display::ListItem)
            || style.establishes_formatting_context()
        {
            self.column_flow = None;
        }
        let started = self.started_flow(
            block,
            geometry,
            (root, spanning),
            (containing_height, used_height),
            outer,
        );
        if !matches!(&started, Ok(FlowStart::Pending(_))) {
            self.column_flow = outer.column_flow;
            self.space = outer.space;
            self.orthogonal_columns = outer.columns;
        }
        started
    }

    /// The flow of `block`, measured whole by the formatting context it establishes, which
    /// measures in a frame of its own as such contexts nest, or else left open in a frame.
    fn started_flow(
        &mut self,
        block: BlockSource<'a>,
        geometry: crate::layout::geometry::HorizontalBox,
        (root, spanning): (bool, bool),
        (containing_height, used_height): (Option<f32>, Option<f32>),
        outer: OuterFlow,
    ) -> Result<FlowStart<'a>, Failure> {
        let style = block.style();
        if block.flow().is_some() {
            return self.measured_flow(|measurer| {
                measurer.frame(block, &geometry, containing_height, used_height)
            });
        }
        if block.math_content().is_some()
            && matches!(style.display, Display::BlockMath | Display::InlineMath)
        {
            return self.measured_flow(|measurer| {
                measurer.request(
                    block,
                    &geometry,
                    containing_height,
                    used_height,
                    frames::RequestKind::Math(crate::layout::mathml::LayoutKey::for_flow(block)),
                )
            });
        }
        if self.space.indefinite_inline
            && block.image().is_none()
            && style.display.is_block_container()
            && *style.width.basis() == crate::style::Size::Auto
            && !self
                .orthogonal_columns
                .is_some_and(|columns| columns.matches(block.block))
        {
            self.orthogonal_columns = Some(self.orthogonal_flow_columns(
                block,
                &geometry,
                containing_height,
                used_height,
            )?);
        }
        if self
            .orthogonal_columns
            .is_some_and(|columns| columns.matches(block.block))
        {
            self.space.indefinite_inline = false;
        }
        if self.native_vertical(block.block).is_none() {
            self.space = self
                .space
                .enter(block, &geometry, containing_height, used_height);
        }
        if !geometry.width.is_finite()
            || !geometry.x.is_finite()
            || geometry
                .padding
                .iter()
                .chain(geometry.margin.iter().flatten())
                .any(|p| !p.is_finite())
        {
            return Err(LayoutError::NonFiniteBlockGeometry {
                block: std::ptr::from_ref(block.block),
            }
            .into());
        }
        if block.image().is_some() {
            return self.measured_flow(|measurer| {
                crate::layout::images::measure(
                    measurer,
                    block,
                    &geometry,
                    containing_height,
                    used_height,
                )
                .map_err(Failure::from)
            });
        }
        if style.display.is_table() {
            return self.measured_flow(|measurer| {
                crate::layout::tables::measure_format(
                    measurer,
                    block,
                    &geometry,
                    containing_height,
                    used_height,
                )
            });
        }
        if style.display.is_grid() {
            return self.measured_flow(|measurer| {
                crate::layout::grid::measure(
                    measurer,
                    block,
                    &geometry,
                    containing_height,
                    used_height,
                )
            });
        }
        if style.display.is_flex() {
            return self.measured_flow(|measurer| {
                crate::layout::flex::measure(
                    measurer,
                    block,
                    &geometry,
                    containing_height,
                    used_height,
                )
            });
        }
        self.flow_frame(
            block,
            geometry,
            (root, spanning),
            (containing_height, used_height),
            outer,
        )
    }

    /// A flow that `measure` measures whole.
    fn measured_flow(
        &mut self,
        measure: impl FnOnce(&mut Self) -> Result<MeasuredBlock<'a>, Failure>,
    ) -> Result<FlowStart<'a>, Failure> {
        measure(self).map(FlowStart::Complete)
    }

    /// The columns an orthogonal flow of `block` without a definite inline size lays its
    /// content out in.
    #[inline(never)]
    fn orthogonal_flow_columns(
        &mut self,
        block: BlockSource<'a>,
        geometry: &crate::layout::geometry::HorizontalBox,
        containing_height: Option<f32>,
        used_height: Option<f32>,
    ) -> Result<crate::layout::multicol::OrthogonalColumns, Failure> {
        let style = block.style();
        let horizontal =
            geometry.padding[1] + geometry.padding[3] + geometry.border[1] + geometry.border[3];
        let available = (self
            .space
            .fallback(crate::style::logical::Flow::used(style), None)
            - horizontal
            - geometry.margin[1].unwrap_or(0.0)
            - geometry.margin[3].unwrap_or(0.0))
        .max(0.0);
        let widths = crate::layout::intrinsic::flow(block.block, self, available)?;
        let vertical =
            crate::layout::geometry::VerticalBox::new(style, geometry, containing_height)
                .with_used_height(used_height);
        let limits = crate::style::SizeConstraints::new(
            [&style.width, &style.min_width, &style.max_width],
            None,
            style.box_sizing,
            horizontal,
        )
        .with_intrinsic(
            [&style.width, &style.min_width, &style.max_width],
            widths,
            None,
        );
        Ok(crate::layout::multicol::OrthogonalColumns {
            principal: false,
            block_auto: vertical.definite().is_none()
                && used_height.is_none()
                && style.height.is_auto_block_size(false),
            content_max: widths.max,
            minimum: limits.minimum,
            maximum: limits.maximum,
            source: std::ptr::from_ref(block.block),
            geometry: ColumnGeometry {
                count: crate::layout::multicol::specified_count(style).unwrap_or(1),
                width: crate::layout::multicol::orthogonal_width(style, widths, available),
                inline_size: geometry.width,
                gap: style.flex.gap[1].resolve_normal(geometry.width, style.size),
                x: geometry.x,
                rtl: style.used_rtl(),
            },
            length: vertical.definite().or_else(|| {
                (style.height.is_auto_block_size(false)
                    && (style.multicol.count.is_none() || style.multicol.width.is_none()))
                .then_some(containing_height)
                .flatten()
                .map(|height| {
                    (height
                        - geometry.padding[0]
                        - geometry.padding[2]
                        - geometry.border[0]
                        - geometry.border[2]
                        - geometry.margin[0].unwrap_or(0.0)
                        - geometry.margin[2].unwrap_or(0.0))
                    .max(0.0)
                })
            }),
        })
    }

    /// The frame that measures the flow of `block` child by child.
    #[inline(never)]
    fn flow_frame(
        &mut self,
        block: BlockSource<'a>,
        geometry: crate::layout::geometry::HorizontalBox,
        (root, spanning): (bool, bool),
        (containing_height, used_height): (Option<f32>, Option<f32>),
        outer: OuterFlow,
    ) -> Result<FlowStart<'a>, Failure> {
        let style = block.style();
        let vertical = self.vertical_box(
            block,
            crate::layout::geometry::VerticalBox::new(style, &geometry, containing_height)
                .with_used_height(used_height),
        );
        let marker = if let Some(paragraph) = block.marker() {
            let index = self.sources.paragraph(paragraph);
            Some(Box::new(crate::layout::markers::measure(
                paragraph,
                self,
                index,
                geometry.x - geometry.padding[3] - geometry.border[3],
                geometry.x + geometry.width + geometry.padding[1] + geometry.border[1],
                super::anchors::key(style),
            )?))
        } else {
            None
        };
        let top_edge = geometry.padding[0] + geometry.border[0];
        let bottom_edge = geometry.padding[2] + geometry.border[2];
        let independent = style.establishes_formatting_context()
            || spanning
            || self
                .orthogonal_columns
                .is_some_and(|columns| columns.matches(block.block));
        let edges = self.fragmentainer.edges(style);
        let columns = self
            .orthogonal_columns
            .filter(|columns| columns.matches(block.block))
            .map(|columns| columns.geometry)
            .or_else(|| {
                (style.display != Display::None && style.multicol.is_multicol())
                    .then(|| ColumnGeometry::resolve(style, geometry.x, geometry.width))
            });
        let (content_x, content_width) = columns
            .as_ref()
            .map_or((geometry.x, geometry.width), |columns| {
                (columns.left(0), columns.width)
            });
        let enclosing = Box::new((
            self.fragmentainer,
            self.column_flow,
            outer.space,
            outer.columns,
        ));
        if columns.is_some() {
            self.fragmentainer = Fragmentainer::Column;
            self.column_flow = Some(ColumnFlow {
                x: geometry.x,
                width: geometry.width,
                rtl: style.used_rtl(),
                containing_height: vertical.definite(),
                fragmentainer: enclosing.0,
            });
        }
        let visible = style.display != Display::None;
        Ok(FlowStart::Pending(FlowFrame {
            block,
            root,
            containing_height,
            used_height,
            cell_percentage_chain: false,
            vertical,
            before: MarginStrut::edge(block.block, 0, geometry.containing_width),
            after: MarginStrut::edge(block.block, 2, geometry.containing_width),
            geometry,
            pending: MarginStrut::default(),
            items: if visible {
                vec![FlowItem::Edge(BlockEdge::Start, top_edge)]
            } else {
                Vec::new()
            },
            at_start: true,
            top_edge,
            bottom_edge,
            independent,
            collapse_top: !root && !independent && top_edge == 0.0,
            edges,
            has_forced: edges.iter().any(|edge| edge.is_forced()),
            columns,
            content_x,
            content_width,
            outer: enclosing,
            restore_column_flow: outer.column_flow,
            previous_break_after: BreakBetween::Auto,
            children: if visible { block.children() } else { &[] }.iter(),
            marker,
            clamp: None,
            fragmented: false,
        }))
    }

    fn format_flow(
        &mut self,
        block: BlockSource<'a>,
        geometry: crate::layout::geometry::HorizontalBox,
        root: bool,
        containing_height: Option<f32>,
        used_height: Option<f32>,
        budget: Option<LineBudget>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        match self.start_flow(block, geometry, root, containing_height, used_height) {
            Ok(FlowStart::Complete(measured)) => Ok(measured),
            Ok(FlowStart::Pending(frame)) => self.continue_flow(frame, budget),
            Err(error) => Err(error),
        }
    }

    fn continue_flow(
        &mut self,
        mut frame: FlowFrame<'a>,
        budget: Option<LineBudget>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        frame.clamp = budget.map(|budget| FrameClamp {
            remaining: budget.lines,
            following: budget.following,
        });
        let mut frames = vec![frame];
        loop {
            let frame = frames.last_mut().unwrap();
            let child = frame.children.next();
            match child {
                Some(BlockContent::Events(events)) => frame.items.push(FlowItem::Events(events)),
                Some(BlockContent::Positioned(child)) => {
                    frame.items.push(FlowItem::Positioned(PositionedFlow {
                        block: child,
                        left: frame.content_x,
                        right: frame.content_x + frame.content_width,
                        margin: frame.pending.value(),
                        alignment: None,
                    }));
                }
                Some(BlockContent::Block(child)) => self.flow_block(&mut frames, child)?,
                Some(BlockContent::Inline(paragraph)) => {
                    self.flow_paragraph(&mut frames, paragraph)?;
                }
                None => {
                    if let Some(measured) = self.close_flow(&mut frames)? {
                        return Ok(measured);
                    }
                }
            }
        }
    }

    /// Measures a block child of the innermost frame, or opens a frame for its flow. Kept
    /// apart from `continue_flow`, whose frame each nested flow's measurement stacks, and each
    /// kind of child apart from the others, as nested measurements stack only theirs.
    fn flow_block(
        &mut self,
        frames: &mut Vec<FlowFrame<'a>>,
        child: &'a BlockBox,
    ) -> Result<(), Failure> {
        if let Some(flow) = self.column_flow.filter(|_| child.style().multicol.span_all) {
            self.flow_spanner(frames, child, flow)
        } else if child.style().display.is_table() {
            self.flow_table(frames, child)
        } else if discards(child.style()) {
            self.flow_discarding(frames, child)
        } else {
            self.flow_child(frames, child)
        }
    }

    #[inline(never)]
    fn flow_spanner(
        &mut self,
        frames: &mut [FlowFrame<'a>],
        child: &'a BlockBox,
        flow: ColumnFlow,
    ) -> Result<(), Failure> {
        let measured = self.spanner(child, flow)?;
        frames.last_mut().unwrap().push_block(measured, true);
        Ok(())
    }

    #[inline(never)]
    fn flow_table(
        &mut self,
        frames: &mut [FlowFrame<'a>],
        child: &'a BlockBox,
    ) -> Result<(), Failure> {
        let frame = frames.last().unwrap();
        let measured = self.measure_block(
            child,
            frame.content_x,
            frame.content_width,
            frame.block.style().used_rtl(),
            false,
            frame.vertical.definite(),
        );
        frames.last_mut().unwrap().push_measured(measured)
    }

    #[inline(never)]
    fn flow_discarding(
        &mut self,
        frames: &mut [FlowFrame<'a>],
        child: &'a BlockBox,
    ) -> Result<(), Failure> {
        let frame = frames.last().unwrap();
        let mut geometry = block_geometry(
            self,
            child,
            frame.content_x,
            frame.content_width,
            frame.block.style().used_rtl(),
            frame.vertical.definite(),
        )?;
        frame.collapse_stretch_margins(&mut geometry);
        let containing = frame.vertical.definite();
        let budget = Self::budget(frames)
            .map(|lines| {
                Ok::<_, Failure>(LineBudget {
                    lines,
                    following: self.following(frames)?,
                })
            })
            .transpose()?;
        let measured =
            self.measure_flow(child.into(), geometry, false, containing, None, budget)?;
        let mut counts = [0; 2];
        if !measured.independent {
            lines_within(&measured.items, 0.0, f32::INFINITY, &mut counts);
        }
        let broken = budget.is_some_and(|budget| counts[1] >= budget.lines && budget.following);
        frames.last_mut().unwrap().push_block(measured, false);
        if budget.is_some() {
            Self::spend_lines(frames, counts[1], broken);
        }
        Ok(())
    }

    /// Measures an in-flow block child that neither spans columns, nor is a table, nor
    /// discards its overflow, or opens a frame for its flow.
    #[inline(never)]
    fn flow_child(
        &mut self,
        frames: &mut Vec<FlowFrame<'a>>,
        child: &'a BlockBox,
    ) -> Result<(), Failure> {
        let (geometry, used_height, cell_percentage_chain) =
            self.child_flow_geometry(frames.last().unwrap(), child)?;
        let containing = frames.last().unwrap().vertical.definite();
        match self.start_flow(child.into(), geometry, false, containing, used_height) {
            Ok(FlowStart::Complete(measured)) => {
                frames.last_mut().unwrap().push_block(measured, false);
            }
            Ok(FlowStart::Pending(mut frame)) => {
                frame.cell_percentage_chain = cell_percentage_chain;
                frames.push(frame);
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    /// The geometry of the in-flow block child `child` of `frame`, the used height it takes as
    /// a percentage height in a table cell's chain of such heights, and whether it is in one.
    #[inline(never)]
    fn child_flow_geometry(
        &mut self,
        frame: &FlowFrame<'a>,
        child: &'a BlockBox,
    ) -> Result<(crate::layout::geometry::HorizontalBox, Option<f32>, bool), Failure> {
        let mut geometry = block_geometry(
            self,
            child,
            frame.content_x,
            frame.content_width,
            frame.block.style().used_rtl(),
            frame.vertical.definite(),
        )?;
        frame.collapse_stretch_margins(&mut geometry);
        let child_style = child.style();
        let cell_percentage_chain = (frame.block.style().display == Display::TableCell
            || frame.cell_percentage_chain)
            && frame.vertical.definite().is_none()
            && child.image().is_none()
            && matches!(child_style.height.basis(), crate::style::Size::Value(_))
            && child_style.height.resolve_definite(None).is_none();
        let used_height = (cell_percentage_chain
            && matches!(
                child_style.overflow[1],
                crate::style::Overflow::Auto | crate::style::Overflow::Scroll
            ))
        .then(|| {
            let edges =
                geometry.padding[0] + geometry.padding[2] + geometry.border[0] + geometry.border[2];
            crate::style::SizeConstraints::new(
                [
                    &crate::style::Size::ZERO,
                    &child_style.min_height,
                    &child_style.max_height,
                ],
                None,
                child_style.box_sizing,
                edges,
            )
            .constrain(0.0)
        });
        Ok((geometry, used_height, cell_percentage_chain))
    }

    /// Measures a paragraph of the innermost frame.
    fn flow_paragraph(
        &mut self,
        frames: &mut [FlowFrame<'a>],
        paragraph: &'a crate::document::Paragraph,
    ) -> Result<(), Failure> {
        let frame = frames.last().unwrap();
        let (x, width, containing, rtl) = (
            frame.content_x,
            frame.content_width,
            frame.vertical.definite(),
            frame.block.style().used_rtl(),
        );
        let budget = Self::budget(frames).map(|lines| LineBudget {
            lines,
            following: false,
        });
        let mut paragraph = self.paragraph(paragraph, x, width, containing, rtl, budget)?;
        let lines = paragraph.line_count();
        if lines > 0
            && budget.is_some_and(|budget| lines == budget.lines)
            && paragraph.clamp.is_none()
            && self.following(frames)?
        {
            self.clamp_paragraph(&mut paragraph)?;
        }
        let broken = paragraph.clamp.is_some();
        let frame = frames.last_mut().unwrap();
        if !paragraph.is_phantom() {
            frame.previous_break_after = BreakBetween::Auto;
        }
        frame.push(FlowItem::Paragraph(paragraph));
        if budget.is_some() {
            Self::spend_lines(frames, lines, broken);
        }
        Ok(())
    }

    /// Finishes the innermost frame into its parent, or restarts it; the measured outermost
    /// block once it finishes.
    fn close_flow(
        &mut self,
        frames: &mut Vec<FlowFrame<'a>>,
    ) -> Result<Option<MeasuredBlock<'a>>, Failure> {
        let frame = frames.pop().unwrap();
        let restore = frame.restore_column_flow;
        let finished = frame.finish(self)?;
        self.column_flow = restore;
        let measured = match finished {
            FlowFinished::Complete(measured) => measured,
            FlowFinished::Restart {
                block,
                columns,
                root,
            } => {
                self.work.spend(1)?;
                let previous_columns = self.orthogonal_columns;
                self.orthogonal_columns = Some(columns);
                let FlowStart::Pending(mut next) = self.start_flow(
                    BlockSource {
                        block: block.source,
                        selected: block.style,
                    },
                    block.geometry,
                    root,
                    block.containing_height,
                    block.sizing.and_then(|sizing| sizing.used_height),
                )?
                else {
                    unreachable!("an automatic column block has a flow continuation")
                };
                next.outer.3 = previous_columns;
                frames.push(next);
                return Ok(None);
            }
        };
        let Some(parent) = frames.last_mut() else {
            return Ok(Some(measured));
        };
        parent.push_block(measured, false);
        Ok(None)
    }

    fn paragraph(
        &mut self,
        paragraph: &'a crate::document::Paragraph,
        x: f32,
        width: f32,
        containing_height: Option<f32>,
        rtl: bool,
        budget: Option<LineBudget>,
    ) -> Result<MeasuredParagraph<'a>, Failure> {
        self.work.spend(paragraph.text().len().max(1))?;
        let index = self.sources.paragraph(paragraph);
        let part = self
            .annotation_part
            .take_if(|(contents, _)| std::ptr::eq(*contents, paragraph))
            .map(|(_, part)| part);
        let mut atomics = Vec::new();
        let mut first_line_atomics = Vec::new();
        for (index, atomic) in paragraph.inline_blocks().enumerate() {
            let containing = self.atomic_containing_height(paragraph, index, containing_height)?;
            atomics.push(Some(crate::layout::inline::measure(
                self,
                atomic.block(),
                width,
                containing,
                rtl,
            )?));
            let first_line = match atomic.block().first_line_box() {
                Some(block) => Some(crate::layout::inline::measure(
                    self, block, width, containing, rtl,
                )?),
                None => None,
            };
            first_line_atomics.push(first_line);
        }
        self.paragraph_lines(
            (paragraph, index, part),
            (x, width, containing_height),
            budget,
            (atomics, first_line_atomics),
        )
    }

    // The helpers below keep their locals off `paragraph`, whose frame each nested atomic
    // inline stacks.
    /// The height of the containing block of atomic inline `index` in `paragraph`: that of the
    /// base an inter-character annotation belongs to, or `containing_height`.
    #[inline(never)]
    fn atomic_containing_height(
        &self,
        paragraph: &crate::document::Paragraph,
        index: usize,
        containing_height: Option<f32>,
    ) -> Result<Option<f32>, Failure> {
        let base_height = paragraph
            .inter_character_base_style(index)
            .map(|style| {
                crate::text::BoxMetrics::of(self.text, style)
                    .map(|metrics| metrics.ascent + metrics.descent)
            })
            .transpose()?;
        Ok(base_height.or(containing_height))
    }

    /// The lines of `paragraph`, numbered `index`, holding its measured atomic inlines: those
    /// of its annotation contents' `part` a line breaking inside the bases holds, or all.
    #[inline(never)]
    fn paragraph_lines(
        &mut self,
        (paragraph, index, part): (
            &'a crate::document::Paragraph,
            usize,
            Option<std::ops::Range<usize>>,
        ),
        (x, width, containing_height): (f32, f32, Option<f32>),
        budget: Option<LineBudget>,
        (atomics, first_line_atomics): (
            Vec<Option<crate::layout::inline::MeasuredAtomic<'a>>>,
            Vec<Option<crate::layout::inline::MeasuredAtomic<'a>>>,
        ),
    ) -> Result<MeasuredParagraph<'a>, Failure> {
        let mut metrics = Vec::new();
        crate::layout::inline::paragraph_metrics(paragraph, &atomics, &mut metrics);
        let mut first_line_metrics = Vec::new();
        crate::layout::inline::first_line_metrics(
            paragraph,
            &metrics,
            &first_line_atomics,
            &mut first_line_metrics,
        );
        let text = self.text;
        let prepared = text.prepare_paragraph(paragraph);
        let mut stream = match part {
            Some(part) => prepared.lines_between(part, width, &metrics)?,
            None => prepared.lines(prepared.start(), width, &metrics)?,
        }
        .with_first_line_atomics(&first_line_metrics)?;
        let mut lines = Vec::new();
        let mut positions = vec![stream.checkpoint()];
        let limit = budget.map_or(usize::MAX, |budget| budget.lines);
        let mut count = 0;
        let mut more = false;
        while let Some(line) = stream.next() {
            self.work.spend(1)?;
            let line = line?;
            if !line.phantom {
                if count == limit {
                    more = true;
                    break;
                }
                count += 1;
            }
            lines.push(line);
            positions.push(stream.checkpoint());
        }
        let broken = count > 0
            && budget.is_some_and(|budget| count == budget.lines && (more || budget.following));
        // An initial letter moves its line down to stay below the content edge (CSS Inline 3
        // §7.6.1).
        let rise = atomics
            .first()
            .and_then(|atomic| atomic.as_ref()?.letter)
            .map_or(0.0, |letter| letter.rise());
        let height = lines.iter().map(|line| line.height).sum::<f32>() + rise;
        let anchors = line_anchors(paragraph, &lines, true);
        let mut measured = MeasuredParagraph {
            index,
            paragraph,
            style: paragraph.style(),
            x,
            width,
            containing_height,
            height,
            lines,
            positions,
            anchors,
            atomics,
            first_line_atomics,
            clamp: None,
        };
        if broken {
            self.clamp_paragraph(&mut measured)?;
        }
        Ok(measured)
    }

    fn clamp_paragraph(&self, paragraph: &mut MeasuredParagraph<'a>) -> Result<(), Failure> {
        let lines = paragraph
            .lines
            .iter()
            .rposition(|line| !line.phantom)
            .expect("only a real line can end a clamped paragraph")
            + 1;
        let clamp = crate::text::ParagraphClamp {
            lines,
            ellipsis: Some(paragraph.style.clamp.block_ellipsis.clone())
                .filter(|ellipsis| *ellipsis != crate::style::BlockEllipsis::None),
        };
        if clamp.ellipsis.is_some() {
            let mut metrics = Vec::new();
            crate::layout::inline::paragraph_metrics(
                paragraph.paragraph,
                &paragraph.atomics,
                &mut metrics,
            );
            let mut first_line_metrics = Vec::new();
            crate::layout::inline::first_line_metrics(
                paragraph.paragraph,
                &metrics,
                &paragraph.first_line_atomics,
                &mut first_line_metrics,
            );
            let prepared = self
                .text
                .prepare_paragraph(paragraph.paragraph)
                .clamped(Some(clamp.clone()));
            let mut stream = prepared
                .lines(paragraph.positions[lines - 1], paragraph.width, &metrics)?
                .with_first_line_atomics(&first_line_metrics)?;
            if let Some(line) = stream.next() {
                let line = line?;
                paragraph.height += line.height - paragraph.lines[lines - 1].height;
                paragraph.lines[lines - 1] = line;
                paragraph.positions[lines] = stream.checkpoint();
            }
        }
        paragraph.lines.truncate(lines);
        paragraph.positions.truncate(lines + 1);
        paragraph.anchors = line_anchors(paragraph.paragraph, &paragraph.lines, false);
        paragraph.clamp = Some(clamp);
        Ok(())
    }
}

pub(super) fn block_geometry<'a>(
    measurer: &Measurer<'a>,
    block: &'a BlockBox,
    x: f32,
    width: f32,
    rtl: bool,
    containing_height: Option<f32>,
) -> Result<crate::layout::geometry::HorizontalBox, Failure> {
    let style = block.style();
    let edges = measurer.box_edges(block, width)?;
    // A box avoiding floats fills the space beside them in place of its containing block.
    let space = measurer.float_space(block).unwrap_or([x, width]);
    let available_width = space[1];
    let used_width = match block.image() {
        Some(_) => {
            Some(crate::layout::images::size(block, width, width, containing_height, None, None)[0])
        }
        None if BlockSource::from(block)
            .collapsed_table()
            .is_some_and(|inner| {
                block.layout_flow().writing_mode.vertical()
                    == inner.style().used_writing_mode().vertical()
            }) =>
        {
            Some(crate::layout::inline::shrink_to_fit(
                measurer,
                block,
                available_width,
                containing_height,
            )?)
        }
        // CSS Align 3 §6.1.1: a `justify-self` other than `normal` or `stretch` sizes an
        // automatic width as fit-content.
        None if style.width == crate::style::Size::Auto
            && style.block_justify_self().is_some_and(|(alignment, _)| {
                !matches!(
                    alignment,
                    crate::style::Alignment::Normal | crate::style::Alignment::Stretch
                )
            }) =>
        {
            Some(crate::layout::inline::shrink_to_fit(
                measurer,
                block,
                available_width,
                containing_height,
            )?)
        }
        None => crate::layout::ratio::width_from_height(
            measurer,
            block,
            width,
            containing_height,
            &edges,
        )?,
    };
    let geometry = crate::layout::geometry::horizontal_in_space(
        style,
        (x, width, rtl),
        space,
        used_width,
        edges,
    );
    let transfers = crate::layout::ratio::PreferredRatio::of_box(style, &edges).is_some();
    if used_width.is_some()
        || (!transfers
            && ![&style.width, &style.min_width, &style.max_width]
                .iter()
                .any(|size| size.intrinsic().is_some()))
    {
        return Ok(geometry);
    }
    let margins = [geometry.margin[3], geometry.margin[1]];
    let available = (available_width
        - edges.horizontal()
        - margins[0].unwrap_or(0.0)
        - margins[1].unwrap_or(0.0))
    .max(0.0);
    let mut constraints = crate::layout::intrinsic::width_constraints(
        block,
        measurer,
        Some(width),
        edges.horizontal(),
        Some(available),
    )?;
    crate::layout::ratio::transfer_to_width(style, &mut constraints, containing_height, &edges);
    let used = constraints.normal_flow(
        available_width,
        edges.horizontal(),
        margins,
        None,
        rtl,
        crate::style::Overconstraint::AdjustEndMargin,
    );
    Ok(crate::layout::geometry::horizontal_in_space(
        style,
        (x, width, rtl),
        space,
        Some(used.size),
        edges,
    ))
}

enum FlowFinished<'a> {
    Complete(MeasuredBlock<'a>),
    Restart {
        block: MeasuredBlock<'a>,
        columns: crate::layout::multicol::OrthogonalColumns,
        root: bool,
    },
}

#[expect(
    clippy::large_enum_variant,
    reason = "measurement results and continuations stay inline to avoid an allocation per probe"
)]
enum FlowStart<'a> {
    Complete(MeasuredBlock<'a>),
    Pending(FlowFrame<'a>),
}

/// The flow state around a flow that its start replaces, restored once the flow is done.
#[derive(Clone, Copy)]
struct OuterFlow {
    column_flow: Option<ColumnFlow>,
    space: FlowSpace,
    columns: Option<crate::layout::multicol::OrthogonalColumns>,
}

struct FlowFrame<'a> {
    block: BlockSource<'a>,
    geometry: crate::layout::geometry::HorizontalBox,
    root: bool,
    containing_height: Option<f32>,
    used_height: Option<f32>,
    cell_percentage_chain: bool,
    vertical: crate::layout::geometry::VerticalBox,
    before: MarginStrut<'a>,
    after: MarginStrut<'a>,
    pending: MarginStrut<'a>,
    items: Vec<FlowItem<'a>>,
    at_start: bool,
    top_edge: f32,
    bottom_edge: f32,
    independent: bool,
    collapse_top: bool,
    edges: [BreakBetween; 2],
    has_forced: bool,
    columns: Option<ColumnGeometry>,
    content_x: f32,
    content_width: f32,
    outer: Box<(
        Fragmentainer,
        Option<ColumnFlow>,
        FlowSpace,
        Option<crate::layout::multicol::OrthogonalColumns>,
    )>,
    restore_column_flow: Option<ColumnFlow>,
    previous_break_after: BreakBetween,
    children: std::slice::Iter<'a, BlockContent>,
    marker: Option<Box<crate::layout::markers::MeasuredMarker>>,
    /// The region break budget of the `continue: discard` container this frame measures.
    clamp: Option<FrameClamp>,
    /// A region break inside the box ends its first fragment, which keeps no end edge.
    fragmented: bool,
}

struct FrameClamp {
    remaining: usize,
    /// Content follows the container in an enclosing one that breaks with it.
    following: bool,
}
impl<'a> FlowFrame<'a> {
    /// Treats as zero the margins of a block child's `geometry` that can collapse through
    /// this frame's edges (CSS Sizing 3 §4.2).
    fn collapse_stretch_margins(&self, geometry: &mut crate::layout::geometry::HorizontalBox) {
        let collapsing = !self.root && !self.independent;
        for (margin, edge) in geometry
            .stretch_margins
            .iter_mut()
            .zip([self.top_edge, self.bottom_edge])
        {
            if collapsing && edge == 0.0 {
                *margin = 0.0;
            }
        }
    }
    fn push(&mut self, item: FlowItem<'a>) {
        if matches!(&item, FlowItem::Paragraph(paragraph) if paragraph.is_phantom()) {
            self.items.push(item);
            return;
        }
        let cleared = matches!(&item, FlowItem::Block(block) | FlowItem::Spanner(block) if block.clearance_target.is_some());
        // CSS Box 4 §3.3: margin-trim discards the block-start margin of a block-level first
        // child and every margin collapsed with it.
        if self.at_start
            && matches!(&item, FlowItem::Block(_) | FlowItem::Spanner(_))
            && self.block.style().margin_trim.block_start
        {
            self.pending = MarginStrut::default();
        }
        if self.at_start && self.collapse_top && !cleared {
            self.before
                .merge(self.pending.clone().within(self.block.block));
        } else {
            let mut margin = self.pending.clone();
            if let FlowItem::Block(block) | FlowItem::Spanner(block) = &item
                && block.clearance_before.is_some()
            {
                margin = margin.for_clearance(block.source);
            }
            self.items.push(FlowItem::Margin(margin));
        }
        self.pending = match &item {
            FlowItem::Block(block) | FlowItem::Spanner(block) => block.after.clone(),
            _ => MarginStrut::default(),
        };
        self.items.push(item);
        self.at_start = false;
    }
    /// Pushes the block child a measurement measured, or passes on its failure.
    fn push_measured(
        &mut self,
        measured: Result<MeasuredBlock<'a>, Failure>,
    ) -> Result<(), Failure> {
        self.push_block(measured?, false);
        Ok(())
    }

    fn push_block(&mut self, mut block: MeasuredBlock<'a>, spanner: bool) {
        if block.style.clear != crate::style::Clear::None || block.clearance_before.is_some() {
            block.clearance_before = Some(Box::new(self.pending.clone()));
        }
        self.has_forced |= block.has_forced;
        self.pending.merge(
            if self.previous_break_after.is_forced() || block.break_before.is_forced() {
                block.before.clone().after_forced_break()
            } else {
                block.before.clone()
            },
        );
        self.previous_break_after = block.break_after;
        if spanner {
            self.push(FlowItem::Spanner(Box::new(block)));
        } else if block.through {
            self.pending.merge(block.after.clone());
            self.items.push(FlowItem::Block(Box::new(block)));
        } else {
            self.push(FlowItem::Block(Box::new(block)));
        }
    }
    fn finish(self, measurer: &mut Measurer<'a>) -> Result<FlowFinished<'a>, Failure> {
        let Self {
            block,
            geometry,
            root,
            containing_height,
            used_height,
            vertical,
            mut before,
            mut after,
            mut pending,
            mut items,
            at_start,
            top_edge,
            mut bottom_edge,
            independent,
            edges,
            mut has_forced,
            columns,
            outer,
            marker,
            fragmented,
            ..
        } = self;
        let style = block.style();
        // CSS Break 3 §5.2 and §5.4: margins adjoining the break are truncated, and a sliced
        // first fragment has no end edge.
        if fragmented {
            pending = MarginStrut::default();
            after = MarginStrut::default();
            if !style.clone_decoration {
                bottom_edge = 0.0;
            }
        }
        // CSS Box 4 §3.3: margin-trim discards the block-end margin of a block-level last child
        // and every margin collapsed with it.
        if style.margin_trim.block_end {
            pending = MarginStrut::default();
        }
        let clearance_target = measurer
            .clearance_targets
            .get(&std::ptr::from_ref::<BlockBox>(block.block))
            .copied();
        let through = clearance_target.is_none()
            && !root
            && !independent
            && at_start
            && top_edge == 0.0
            && bottom_edge == 0.0
            && vertical.collapse_through();
        if through {
            before.merge(pending.clone().within(block.block));
            before.merge(after);
            after = MarginStrut::default();
        } else if !root && !independent && bottom_edge == 0.0 && vertical.collapse_bottom() {
            let mut collapsed = pending.within(block.block);
            collapsed.merge(after);
            after = collapsed;
        } else {
            items.push(FlowItem::Margin(pending));
        }
        let orthogonal = measurer
            .orthogonal_columns
            .filter(|columns| columns.matches(block.block));
        (measurer.fragmentainer, measurer.column_flow) = (outer.0, outer.1);
        if let Some(columns) = columns {
            let content = items.split_off(1);
            let columns = crate::layout::multicol::measure(
                measurer,
                columns,
                style,
                content,
                orthogonal
                    .and_then(|columns| columns.length)
                    .or_else(|| vertical.definite()),
                orthogonal.is_some(),
                block.block.column_text_trim(),
            )?;
            has_forced = edges.iter().any(|edge| edge.is_forced()) || columns.forces_page;
            items.push(FlowItem::Columns(Box::new(columns)));
        }
        measurer.space = outer.2;
        measurer.orthogonal_columns = outer.3;
        items.push(FlowItem::Edge(BlockEdge::End, bottom_edge));
        let legend = place_legend(&mut items, &geometry);
        let top_edge = legend.map_or(top_edge, |(_, start)| start);
        let mut height = items.iter().map(FlowItem::height).sum::<f32>();
        if !height.is_finite() {
            return Err(LayoutError::NonFiniteBlockGeometry {
                block: std::ptr::from_ref(block.block),
            }
            .into());
        }
        if height < 0.0 {
            items.push(FlowItem::Space(-height));
            height = 0.0;
        }
        let mut natural_height = height;
        height = vertical.used_height(height);
        if !height.is_finite() {
            return Err(LayoutError::NonFiniteBlockGeometry {
                block: std::ptr::from_ref(block.block),
            }
            .into());
        }
        let constrained = height != natural_height || vertical.definite().is_some();
        let alignment = content_alignment(style, height - natural_height);
        if alignment != 0.0 {
            items.insert(1, FlowItem::Space(alignment));
            natural_height += alignment;
        }
        let overflow_height = overflow_height(&items).max(height);
        let first_fragment = if measurer.fragmentainer.keeps(style)
            && !has_forced
            && overflow_height <= measurer.page_height
        {
            overflow_height
        } else {
            (crate::layout::breaks::first_fragment(&items)
                + if style.clone_decoration {
                    bottom_edge
                } else {
                    0.0
                })
            .min(overflow_height)
        };
        let (break_before, break_after) = crate::layout::breaks::boundaries(style, &items, edges);
        let mut paint = measurer.box_paint(block, &geometry);
        if let Some(paint) = &mut paint {
            paint.legend = legend.map(|(cut, _)| cut);
        }
        let measured = MeasuredBlock {
        frame_key: None,
        frame_owner: None,
        math: None,
            grid: None,
            source: block.block,
            geometry: geometry.clone(),
            containing_height,
            offset: style.relative_offset(
                geometry.containing_width,
                containing_height,
                geometry.containing_rtl,
            ),
            style,
            canvas_source: block.is_canvas_source(),
            clearance_before: (style.clear != crate::style::Clear::None || items.iter().any(|item| matches!(item, FlowItem::Block(child) | FlowItem::Spanner(child) if child.clearance_before.is_some()))).then(|| Box::new(MarginStrut::default())),
            clearance_target,
            before,
            after,
            through,
            height,
            sizing: Some(BlockSizing::new(natural_height, &geometry, used_height)),
            constrained,
            independent: root || independent,
            overflow_height,
            first_fragment,
            has_forced,
            break_before,
            break_after,
            items,
            paint,
            navigation: block.navigation(),
            end_space: bottom_edge,
            start_space: top_edge,
            repeat_space: 0.0,
            marker,
            table: None,
            continues: false,
            spanner_fragment: None,
            line_fragments: Vec::new(),
        };
        if let Some(columns) = orthogonal.filter(|columns| !columns.principal) {
            let content = measured
                .items
                .iter()
                .find_map(|item| match item {
                    FlowItem::Columns(columns) => Some(columns),
                    _ => None,
                })
                .expect("an automatic column block measures columns");
            if let Some(columns) = columns.adjusted(content, style) {
                return Ok(FlowFinished::Restart {
                    block: measured,
                    columns,
                    root,
                });
            }
        }
        Ok(FlowFinished::Complete(measured))
    }
}

/// HTML §15.3.12: a fieldset's rendered legend sits in its block-start border area, as tall
/// as the border or the legend's margin box, whichever is greater, with the legend's border
/// box centred over the border. Returns where the border paints and the gap it leaves, and
/// the space before the legend, which replaces the block-start edge.
fn place_legend(
    items: &mut [FlowItem<'_>],
    geometry: &crate::layout::geometry::HorizontalBox,
) -> Option<(crate::graphics::LegendCut, f32)> {
    let index = items
        .iter()
        .position(|item| matches!(item, FlowItem::Block(_)))?;
    let (FlowItem::Edge(BlockEdge::Start, _), FlowItem::Margin(_), FlowItem::Margin(_)) = (
        items.first()?,
        items.get(index.checked_sub(1)?)?,
        items.get(index + 1)?,
    ) else {
        return None;
    };
    let FlowItem::Block(legend) = &items[index] else {
        unreachable!("the legend position holds a block")
    };
    if !legend.source.is_rendered_legend() {
        return None;
    }
    let border = geometry.border[0];
    let height = legend.height;
    let legend_box = legend.geometry.border_box(height);
    let fieldset_box = geometry.border_box(0.0);
    let [before, after] = [index - 1, index + 1].map(|at| items[at].height());
    let outer = before + height + after;
    let area = border.max(outer);
    let (top, inset) = if outer > border {
        (0.0, before + (height - border) / 2.0)
    } else {
        ((border - height) / 2.0 - before, 0.0)
    };
    items[0] = FlowItem::Edge(BlockEdge::Start, top);
    items[index + 1] = FlowItem::Space(area - top - before - height);
    Some((
        crate::graphics::LegendCut {
            side: crate::style::logical::TOP,
            inset,
            gap: [
                legend_box.x - fieldset_box.x,
                legend_box.x - fieldset_box.x + legend_box.width,
            ],
        },
        top,
    ))
}

/// CSS Box Alignment 3 §5.4: the block-axis offset of a block container's content within
/// `free` space. Overflowing content stays at the start edge unless aligned `unsafe`: a block
/// container without an overflow position aligns safely (§5.1.1), as a scroll container's
/// overflow into its unscrollable start is biased back (§4.4.1.3); distributed alignment
/// falls back to centring.
fn content_alignment(style: &Style, free: f32) -> f32 {
    use crate::style::Alignment;
    let factor = match style.flex.align_content {
        Alignment::Center | Alignment::SpaceAround | Alignment::SpaceEvenly => 0.5,
        // A lone box has no baseline-sharing group: last baseline falls back to safe end.
        Alignment::End | Alignment::FlexEnd | Alignment::LastBaseline => 1.0,
        _ => return 0.0,
    };
    if free < 0.0 && style.flex.align_content_overflow != crate::style::OverflowPosition::Unsafe {
        return 0.0;
    }
    free * factor
}

fn is_sticky(style: &Style) -> bool {
    style.position == crate::style::Position::Sticky
}

/// Whether the block is the containing block of a sticky box, which its fragments constrain
/// (CSS Position 3 §3.4).
fn contains_child(block: &BlockBox, check: impl Fn(&Style) -> bool) -> bool {
    let sticky = |child: &BlockBox| {
        check(child.style()) || child.enclosing_inline().iter().any(|style| check(style))
    };
    block.children().iter().any(|child| match child {
        BlockContent::Block(child) | BlockContent::Positioned(child) => sticky(child),
        BlockContent::Inline(paragraph) => {
            paragraph
                .inline_blocks()
                .any(|atomic| sticky(atomic.block()))
                || paragraph.boxes().iter().any(|inline| check(&inline.style))
        }
        BlockContent::Events(_) => false,
    })
}

/// Whether the block splits an inline box whose sticky shift, filter region or mask its
/// fragments bound with the inline box's own (CSS 2.1 §9.2.1.1).
fn bounds_enclosing_inline(block: &BlockBox) -> bool {
    block
        .enclosing_inline()
        .iter()
        .any(|style| is_sticky(style) || style.is_filtered() || style.decoration.masks.is_masked())
}

/// Static positions of a paragraph's absolutely positioned boxes (CSS 2.1 §10.3.7).
pub(super) fn line_anchors<'a>(
    paragraph: &'a crate::document::Paragraph,
    lines: &[Line],
    complete: bool,
) -> Vec<LineAnchor<'a>> {
    let rtl = paragraph.style().used_rtl();
    paragraph
        .anchors()
        .iter()
        .filter(|anchor| anchor.block().style().floating().is_none())
        .filter(|anchor| {
            complete
                || lines.last().is_some_and(|line| {
                    if anchor.block().style().inline_level_origin {
                        anchor.offset < line.range.end
                    } else {
                        anchor.offset <= line.range.end
                    }
                })
        })
        .map(|anchor| {
            let block = anchor.block();
            let last = lines.len().saturating_sub(1);
            if block.style().inline_level_origin {
                let line = lines
                    .iter()
                    .position(|line| anchor.offset < line.range.end)
                    .unwrap_or(last);
                LineAnchor {
                    block,
                    line,
                    x: lines
                        .get(line)
                        .map(|line| line.x_at(paragraph, anchor.offset, rtl)),
                    below: false,
                    inline_box: anchor.inline_box,
                }
            } else {
                let line = lines
                    .iter()
                    .position(|line| anchor.offset <= line.range.end)
                    .unwrap_or(last);
                LineAnchor {
                    block,
                    line,
                    x: None,
                    below: anchor.offset > 0 && !lines.is_empty(),
                    inline_box: anchor.inline_box,
                }
            }
        })
        .collect()
}
