mod cells;
mod columns;
mod floats;
mod order;
mod overflow;
mod paragraph;
mod rows;
use order::{OrderSnapshots, PaintOrder, PaintOrders};

use paragraph::ParagraphCursor;

use crate::document::{BlockBox, BoxNavigation};
use crate::graphics::{
    Backgrounds, BoxAreas, BoxDecoration, BoxPaint, Clip, DecorationPaint, DisplayItem, Rect,
};
use crate::layout::breaks::{BreakTail, Forced, Fragmentainer, fragmentainer_height};
use crate::layout::margin::MarginStrut;
use crate::layout::navigation::{Destination, LinkArea};
use crate::layout::positioned::StaticPosition;
use crate::layout::{
    LayoutError, MAX_LAYOUT_PAGES, TextLine,
    measure::{
        BlockEdge, FlowItem, MeasuredBlock, MeasuredParagraph, MeasuredRow, Measurer, TableFlow,
    },
    tables::BandPlacement,
};
use crate::style::Position;
pub(super) use columns::measure_columns;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    rc::Rc,
};

/// Painting order: a stacking-context path of `(z-index, tree order)` levels, compared
/// lexicographically (CSS 2.1 Appendix E).
type PaintKey = Rc<[(i32, PaintOrder)]>;
/// Where content paints: the stacking context of its positioned and stacking-context
/// descendants, the key of its own items, its innermost opacity group and enclosing effect,
/// and the relative offset of the inline boxes it lies in (CSS Position 3 §3.3).
#[derive(Clone, PartialEq)]
struct Layer {
    context: PaintKey,
    key: PaintKey,
    group: Option<usize>,
    clip: Option<usize>,
    offset: [f32; 2],
}
/// Glyph runs and decorations of one line that paint in one layer.
struct LinePart {
    layer: Layer,
    runs: Vec<crate::text::GlyphRun>,
    decorations: Vec<DecorationPaint>,
}
/// The containing block of a paragraph's inline boxes, which their percentage offsets resolve
/// against.
#[derive(Clone, Copy)]
struct InlineContaining {
    width: f32,
    height: Option<f32>,
    rtl: bool,
}
impl InlineContaining {
    /// The containing block a block container's geometry `(x, width, rtl, height)` gives.
    fn of((_, width, rtl, height): (f32, f32, bool, Option<f32>)) -> Self {
        Self { width, height, rtl }
    }
    fn relative(self, style: &crate::style::Style) -> [f32; 2] {
        style.relative_offset(self.width, self.height, self.rtl)
    }
}
/// A paragraph being painted: its containing block and its inline boxes' painted fragments.
#[derive(Clone)]
struct InlineLayers<'p> {
    paragraph: &'p crate::document::Paragraph,
    containing: InlineContaining,
    fragments: Vec<Vec<(usize, usize)>>,
}
/// Absolutely positioned descendants of a non-atomic inline box waiting for the end of its
/// block container, and the content boxes of its fragments by page and line in layout
/// order, which form their containing block (CSS Position 3 §2.1).
#[derive(Clone)]
struct InlineContainer<'a> {
    pending: Vec<Pending<'a>>,
    lines: Vec<(usize, Vec<Rect>)>,
    rtl: bool,
    /// The effect enclosing the box's content.
    clip: Option<usize>,
    /// Depth in `Paginator::boxes` of its block container.
    depth: usize,
}
/// Identifies an inline element by the style its split boxes share.
type InlineElement = *const crate::style::Style;
/// Whether an inline box paints its content in its own layer: a stacking context or a
/// relatively or sticky positioned box (CSS 2.1 Appendix E).
fn paints_in_own_layer(style: &crate::style::Style) -> bool {
    style.position != Position::Static || style.is_grouped() || style.forced_stacking_context()
}
/// A stacking context's own background precedes its negative z-index descendants.
const ROOT_LEVEL: (i32, PaintOrder) = (i32::MIN, PaintOrder::FIRST);
/// Non-positioned content of a stacking context precedes its z-index 0 descendants.
const FLOW_LEVEL: (i32, PaintOrder) = (0, PaintOrder::FIRST);
/// Within flow, float groups follow block backgrounds and precede inline paint keys.
const FLOAT_PAINT_LEVEL: i32 = -1;
/// Outlines follow every other level of their stacking context.
const OUTLINE_LEVEL: i32 = i32::MAX;

fn extend(key: &PaintKey, level: (i32, PaintOrder)) -> PaintKey {
    key.iter().cloned().chain(std::iter::once(level)).collect()
}

#[derive(Clone)]
struct PositionedLayer {
    context: PaintKey,
    key: PaintKey,
    background: PaintKey,
}

fn positioned_layer(
    context: &PaintKey,
    z_index: Option<i32>,
    order: PaintOrder,
) -> PositionedLayer {
    if let Some(z) = z_index {
        let context = extend(context, (z, order));
        PositionedLayer {
            key: extend(&context, FLOW_LEVEL),
            background: extend(&context, ROOT_LEVEL),
            context,
        }
    } else {
        let key = extend(context, (0, order));
        PositionedLayer {
            context: context.clone(),
            background: key.clone(),
            key,
        }
    }
}

/// Fixed boxes repeat on every page; this bounds boxes × pages.
const MAX_FIXED_PLACEMENTS: usize = 100_000;
/// Retain common page geometries without keeping one alignment vector per physical page.
const MAX_TABLE_ALIGNMENT_GEOMETRIES: usize = 8;
/// Rounding noise in summed `f32` geometry, so that `(H - edges) + edges` still fits `H`.
const FRAGMENTATION_TOLERANCE_PT: f32 = 0.01;

/// Whether content reaching `extent` overflows a fragmentainer ending at `limit`.
pub(super) fn overflows(extent: f32, limit: f32) -> bool {
    extent > limit + FRAGMENTATION_TOLERANCE_PT
}

/// Absolutely positioned box waiting for its containing block to finish.
#[derive(Clone)]
struct Pending<'a> {
    block: &'a BlockBox,
    position: StaticPosition,
    context: PaintKey,
    group: Option<usize>,
    /// 3D placement node of the box's parent.
    parent_3d: Option<usize>,
    order: PaintOrder,
    orders: PaintOrders,
}

/// Positioned descendants waiting for a containing block to close.
#[derive(Clone)]
struct Container<'a> {
    pending: Vec<Pending<'a>>,
    /// Transformed boxes also contain fixed descendants (CSS Transforms 1 §2).
    fixed: bool,
}

impl Container<'_> {
    fn detach_orders(&mut self, snapshots: &mut OrderSnapshots) {
        for pending in &mut self.pending {
            snapshots.detach(&mut pending.orders);
        }
    }
}

/// Fixed position and paint context of a box laid out without page breaks.
struct Unbroken {
    page: usize,
    /// Border-box top below the page content top.
    y: f32,
    offset: [f32; 2],
    clip: Option<usize>,
    context: PaintKey,
    group: Option<usize>,
    parent_3d: Option<usize>,
    key: PaintKey,
    reserved_order: Option<PaintOrder>,
}

/// Where an absolutely positioned box resolves its containing block.
enum Containing<'a> {
    Rect {
        rect: Rect,
        rtl: bool,
        clip: Option<usize>,
    },
    /// Padding boxes of a positioned ancestor's page fragments.
    Box {
        grid: Option<Rc<crate::layout::grid::Geometry<'a>>>,
        overflow: BTreeMap<usize, Rect>,
        fragments: Vec<(usize, usize)>,
        areas: Vec<(usize, Rect)>,
        rtl: bool,
        clip: Option<usize>,
    },
    /// The page area: the initial containing block, and every page for fixed boxes.
    Page,
}

#[derive(Clone)]
struct ScopedPaint {
    visible: bool,
    scope: PaintKey,
    order: PaintOrder,
    /// Innermost clip or transform enclosing the item.
    clip: Option<usize>,
    /// Innermost opacity group enclosing the item.
    group: Option<usize>,
    /// Glyph ink of a text line; lines whose ink misses a clip are omitted.
    ink: Option<Rect>,
    item: DisplayItem,
    /// Hyperlink and destination located by a box item's border box.
    navigation: BoxNavigation,
}

/// Paint effect of one box, resolved per page from that page's fragment.
#[derive(Clone, Copy)]
enum Effect<'a> {
    /// Overflow clip to the fragment's padding box along the clipped axes.
    Clip([bool; 2]),
    BorderClip,
    /// A resolved paint fragment clip, with exterior edges left open.
    PaintClip(Clip),
    /// The `clip` region of an absolutely positioned box, from its fragment's border box.
    RectClip(&'a crate::style::Style),
    /// The box's transform about its fragment's reference box, placed by its parent's
    /// `Parent3d` node.
    Transform(&'a crate::style::Style, Option<usize>),
    /// A sticky box's shift on each page, constrained by `Paginator::sticky` entry `usize`.
    Sticky(&'a crate::style::Style, usize),
    /// The perspective and 3D rendering context a box gives its transformed children, resolved
    /// from its fragment on each page; never an enclosing effect.
    Parent3d(&'a crate::style::Style),
    /// Compositing of a stacking context. Groups nest by stacking context, independently of
    /// clips and transforms, which follow containing blocks, unless anchored.
    Group(Group<'a>),
}
/// How a stacking context composites; every stacking context is an isolated group, which
/// paints as its content does unless something inside it blends (Compositing 1 §3.2).
#[derive(Clone, Copy)]
struct Group<'a> {
    opacity: f32,
    blend: crate::graphics::BlendMode,
    isolated: bool,
    filter: &'a [crate::graphics::FilterFunction],
    masked: bool,
    /// The innermost clip or transform of the element's own box, inside which a group that
    /// does not commute with clipping and transforming opens.
    anchor: Option<usize>,
}
impl<'a> Group<'a> {
    const IDENTITY: Self = Self {
        opacity: 1.0,
        blend: crate::graphics::BlendMode::Normal,
        isolated: false,
        filter: &[],
        masked: false,
        anchor: None,
    };
    fn of(style: &'a crate::style::Style) -> Self {
        Self {
            opacity: style.opacity,
            blend: style.mix_blend_mode,
            isolated: style.isolate,
            filter: &style.filter,
            masked: style.decoration.masks.is_masked(),
            anchor: None,
        }
    }
    /// Filters act in the element's local coordinates and are clipped by its ancestors'
    /// clips afterwards (Filter Effects 1 §5), and masks are positioned in those coordinates
    /// (CSS Masking 1 §7), so their group opens inside the clips and transforms of the
    /// element's own box.
    fn anchored(&self) -> bool {
        !self.filter.is_empty() || self.masked
    }
    /// The display items opening this group, outermost first: an isolated blend group, an
    /// opacity group, which is isolated itself, the mask, then the filter, which applies
    /// before the others (Filter Effects 1 §5). `fragments` are the element's box fragments
    /// on the page.
    fn pushes(self, fragments: &[BoxPaint]) -> Vec<DisplayItem> {
        let bounds = fragments.iter().map(|paint| paint.rect).reduce(Rect::union);
        let translucent = self.opacity < 1.0;
        let blend =
            self.blend != crate::graphics::BlendMode::Normal || (self.isolated && !translucent);
        blend
            .then_some(DisplayItem::PushBlend(self.blend))
            .into_iter()
            .chain(translucent.then_some(DisplayItem::PushOpacity(self.opacity)))
            .chain(self.masked.then(|| {
                DisplayItem::PushMask(crate::graphics::MaskPaint {
                    fragments: fragments.to_vec(),
                })
            }))
            .chain((!self.filter.is_empty()).then(|| {
                DisplayItem::PushFilter(crate::graphics::FilterPaint {
                    functions: self.filter.to_vec(),
                    bounds,
                })
            }))
            .collect()
    }
}
#[derive(Clone)]
struct EffectNode<'a> {
    /// Reference boxes for structural elements without their own painted box, and the border
    /// boxes of a sticky inline box's fragments.
    areas: Vec<(usize, Rect)>,
    /// Inline box fragments an anchored group's element paints, by page.
    paints: Vec<(usize, BoxPaint)>,
    /// Enclosing clip or transform, or for a group the enclosing group.
    parent: Option<usize>,
    effect: Effect<'a>,
    fragments: Vec<(usize, usize)>,
    /// The stacking context a group composites; `None` for other effects.
    context: Option<PaintKey>,
}
/// An effect resolved for one page.
#[derive(Clone, Copy)]
pub(crate) enum Region {
    Clip(Clip),
    Transform([f32; 6]),
}

/// The fragmentainers that page indices address.
#[derive(Clone)]
enum Fragmentainers<'a, 'r> {
    /// Owned geometry and style selection for each physical page.
    Pages(crate::layout::physical_pages::PhysicalPages<'a, 'r>),
    /// The column boxes of a multi-column row segment.
    Columns(columns::ColumnRows),
}
impl Fragmentainers<'_, '_> {
    fn kind(&self) -> Fragmentainer {
        match self {
            Self::Pages(_) => Fragmentainer::Page,
            Self::Columns(_) => Fragmentainer::Column,
        }
    }
    fn top(&self, page: usize) -> f32 {
        match self {
            Self::Pages(pages) => pages.records[page].geometry.content_box().y,
            Self::Columns(_) => 0.0,
        }
    }
    fn height(&self, page: usize) -> f32 {
        fragmentainer_height(match self {
            Self::Pages(pages) => pages.records[page].geometry.area[1],
            Self::Columns(rows) => rows.height(page),
        })
    }
    fn content_box(&self, page: usize) -> Rect {
        match self {
            Self::Pages(pages) => pages.records[page].geometry.content_box(),
            Self::Columns(rows) => rows.content_box(page),
        }
    }
    /// The page area in the coordinates of fragmentainer `page`.
    fn rect(&self, page: usize) -> Rect {
        match self {
            Self::Pages(pages) => {
                let record = &pages.records[page];
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: record.geometry.paper[0],
                    height: record.geometry.paper[1],
                }
                .union(record.output.bleed)
            }
            Self::Columns(rows) => rows.page_rect(page),
        }
    }
    /// The fragmentainer a break after `page` moves to: `candidate`, or for a forced page
    /// break between columns, the first column of a later row.
    fn next(&self, page: usize, candidate: usize, forced: Option<Forced>) -> usize {
        match self {
            Self::Columns(rows) if forced.is_some_and(Forced::is_page) => {
                rows.next_row(page, candidate)
            }
            Self::Pages(pages) => {
                let side = forced
                    .and_then(|forced| forced.side(pages.document.styles().page_progression_rtl()));
                candidate + usize::from(side.is_some_and(|side| pages.side(candidate) != side))
            }
            Self::Columns(_) => candidate,
        }
    }
}

/// A table being paginated, with the bands placed on each page for its collapsed borders.
#[derive(Clone)]
struct BandPage<'a> {
    header: Vec<MeasuredRow<'a>>,
    footer: Vec<MeasuredRow<'a>>,
    header_start: f32,
    header_height: f32,
    body_start: f32,
    footer_height: f32,
    footer_start: f32,
    spacing: f32,
}
#[derive(Clone)]
struct BandContinuation<'a> {
    table: usize,
    boxes: usize,
    first_page: usize,
    minimum: f32,
    pages: BTreeMap<usize, BandPage<'a>>,
}

#[derive(Clone)]
struct TableBackground {
    repeated: bool,
    rows: crate::layout::tables::BackgroundRows,
    page: usize,
    slot: usize,
    underlay: Option<usize>,
    before: f32,
}
#[derive(Clone)]
struct TableLayer {
    clip: Option<usize>,
    paint: PositionedLayer,
    group: Option<usize>,
}

#[derive(Clone)]
struct ActiveTable<'a> {
    shadows: Vec<crate::layout::tables::TableContaining<'a>>,
    transforms: Vec<(crate::layout::tables::TableContaining<'a>, usize)>,
    layers: HashMap<usize, TableLayer>,
    cell_fragments: HashMap<(usize, usize), usize>,
    cell_overflow: HashMap<(usize, usize), Rect>,
    positioned: Vec<(crate::layout::tables::TableContaining<'a>, Vec<Pending<'a>>)>,
    cell_heights: Vec<f32>,
    backgrounds: Vec<TableBackground>,
    flow: Box<TableFlow<'a>>,
    x_offset: f32,
    placements: Vec<(usize, BandPlacement)>,
    grids: BTreeMap<usize, crate::layout::tables::CollapsedGrid>,
    rows: HashMap<usize, MeasuredRow<'a>>,
    next_body: usize,
}

impl ActiveTable<'_> {
    fn repeats(&self, rows: &std::ops::Range<usize>) -> bool {
        [&self.flow.header, &self.flow.footer]
            .into_iter()
            .any(|bands| {
                let first = bands.first().and_then(|band| band.band.as_ref());
                let last = bands.last().and_then(|band| band.band.as_ref());
                first
                    .zip(last)
                    .is_some_and(|(first, last)| rows.start >= first.start && rows.end <= last.end)
            })
    }
}

#[derive(Clone)]
struct ActiveGeometry<'a> {
    grid_item: bool,
    grid: Option<Rc<crate::layout::grid::Geometry<'a>>>,
    relative: [f32; 2],
    floating: Option<floats::FloatFlow>,
    source: &'a BlockBox,
    horizontal: crate::layout::geometry::HorizontalBox,
    containing_height: Option<f32>,
    normal: bool,
    table_plan: Option<Rc<crate::layout::tables::TablePlan<'a>>>,
    table_wrapper: bool,
    sizing: Option<crate::layout::measure::BlockSizing>,
    height: f32,
    constrained: bool,
    spanner_fragment: Option<crate::layout::multicol::SpannerFragment>,
}
impl ActiveGeometry<'_> {
    fn definite_height(&self) -> Option<f32> {
        if self.table_wrapper {
            None
        } else {
            crate::layout::geometry::VerticalBox::new(
                self.source.style(),
                &self.horizontal,
                self.containing_height,
            )
            .with_used_height(self.sizing.and_then(|sizing| sizing.used_height))
            .definite()
        }
    }
    fn start_space(&self) -> f32 {
        self.horizontal.padding[0] + self.horizontal.border[0]
    }
    fn end_space(&self, cloned: bool) -> f32 {
        if cloned || self.spanner_fragment.is_none_or(|fragment| fragment.last) {
            self.horizontal.padding[2] + self.horizontal.border[2]
        } else {
            0.0
        }
    }
}

#[derive(Clone)]
struct ActiveBox<'a> {
    grid_orders: HashMap<*const BlockBox, PaintOrder>,
    grid: Option<Rc<crate::layout::grid::Geometry<'a>>>,
    overflow: Option<BTreeMap<usize, Rect>>,
    visible: bool,
    pending: Vec<Pending<'a>>,
    float_placement: Option<floats::FloatPlacement>,
    geometry: Option<ActiveGeometry<'a>>,
    #[expect(
        clippy::option_option,
        reason = "outer is whether the box is a page boundary, inner is its used page name"
    )]
    page_scope: Option<Option<std::sync::Arc<str>>>,
    offset: [f32; 2],
    scope: PaintKey,
    order: PaintOrder,
    /// Effect enclosing this box's own paint.
    clip: Option<usize>,
    group: Option<usize>,
    clip_node: Option<usize>,
    transform_node: Option<usize>,
    clip_rect_node: Option<usize>,
    parent_3d_node: Option<usize>,
    sticky_node: Option<usize>,
    /// `Paginator::sticky` entries whose containing block, and whose nearest scroll
    /// container, this box is.
    sticky_children: Vec<usize>,
    sticky_scrolled: Vec<usize>,
    /// The box's own anchored group, which its fragments bound.
    group_node: Option<usize>,
    paint: Option<(BoxPaint, usize)>,
    fragments: Vec<(usize, usize)>,
    start_space: f32,
    repeat_space: f32,
    end_space: f32,
    reserves_end: bool,
    clone: bool,
    marker: Option<Box<crate::layout::markers::MeasuredMarker>>,
    /// Index in `Paginator::sliced` of this box's assembled decoration.
    sliced: Option<usize>,
    canvas_root: bool,
    navigation: BoxNavigation,
}

impl ActiveBox<'_> {
    fn record_float_fragment(&mut self, page: usize, top: f32, last: bool) {
        if let (Some(placement), Some((paint, _)), Some(geometry)) =
            (&mut self.float_placement, &self.paint, &self.geometry)
        {
            placement.record(page, top, paint, geometry, last);
        }
    }
}

/// What constrains a sticky box (CSS Position 3 §3.4): its margins, the fragments of its
/// containing block, and those of its nearest scroll container, whose padding boxes are the
/// scrollports; without one the page area is.
#[derive(Clone, Default)]
pub(super) struct StickyConstraints {
    margins: [f32; 4],
    containing_rtl: bool,
    containing: Vec<(usize, usize)>,
    scrollport: Option<Vec<(usize, usize)>>,
}

#[derive(Clone)]
struct SlicedBox<'a> {
    style: &'a crate::style::Style,
    fragments: Vec<(usize, usize)>,
}

#[derive(Clone)]
struct ExtentCursor {
    reserve_end: bool,
    page: usize,
    origin: f32,
    remaining: f64,
    advance: Option<f32>,
}

struct ExtentSpace {
    start: f32,
    cloned_end: f32,
    bottom_reserve: f32,
    height: Option<f32>,
}

#[derive(Clone)]
struct BlockEnd {
    fragmented_height: Option<f32>,
    reflowed: bool,
    extent: Option<ExtentCursor>,
    float_natural_height: Option<f32>,
}

#[derive(Clone)]
struct BlockFrame<'a> {
    grow_after_break: bool,
    clipped_height: Option<f32>,
    defer_positioned: bool,
    end: Option<BlockEnd>,
    style: &'a crate::style::Style,
    height: f32,
    independent: bool,
    break_after: crate::style::BreakBetween,
    continues: bool,
    table: bool,
    /// Contains positioned descendants, which place when it ends.
    containing: bool,
    start_page: usize,
    forced_breaks: usize,
    start_y: f32,
    previous_offset: [f32; 2],
    previous_key: PaintKey,
    previous_context: PaintKey,
    previous_clip: Option<usize>,
    previous_group: Option<usize>,
    previous_parent_3d: Option<usize>,
    previous_reserve: f32,
    split: SplitInline<'a>,
    items: Vec<FlowItem<'a>>,
}
/// The inline boxes a block splits (CSS 2.1 §9.2.1.1): those that are the containing blocks
/// of positioned descendants, with the layer inside each, and the sticky and anchored group
/// nodes whose regions its fragments bound.
#[derive(Clone, Default)]
struct SplitInline<'a> {
    containers: Vec<(&'a crate::style::Style, Layer)>,
    nodes: Vec<usize>,
}

#[derive(Clone)]
struct FlowFrame<'a> {
    items: std::iter::Zip<std::vec::IntoIter<FlowItem<'a>>, std::vec::IntoIter<BreakTail>>,
    block: Option<BlockFrame<'a>>,
}
impl<'a> FlowFrame<'a> {
    fn new(items: Vec<FlowItem<'a>>, tail: BreakTail, block: Option<BlockFrame<'a>>) -> Self {
        let tails = crate::layout::breaks::tail_heights(&items, tail);
        Self {
            items: items.into_iter().zip(tails),
            block,
        }
    }
}

#[derive(Clone)]
struct RowEntry<'a> {
    row: MeasuredRow<'a>,
    tail: BreakTail,
    body: bool,
    after_break: bool,
}

#[derive(Clone)]
struct FlowCursor<'a, 't> {
    frames: Vec<FlowFrame<'a>>,
    paragraph: Option<(ParagraphCursor<'a, 't>, BreakTail)>,
    row: Option<Box<rows::RowCursor<'a, 't>>>,
    columns: Option<Box<columns::ColumnCursor<'a, 't>>>,
    row_entry: Option<RowEntry<'a>>,
    ready: Option<(FlowItem<'a>, BreakTail)>,
}

impl<'a> FlowCursor<'a, '_> {
    fn detach_orders(&mut self, snapshots: &mut OrderSnapshots) {
        if let Some(row) = &mut self.row {
            row.detach_orders(snapshots);
        }
        if let Some(columns) = &mut self.columns {
            columns.detach_orders(snapshots);
        }
    }
    fn new(items: Vec<FlowItem<'a>>, tail: BreakTail) -> Box<Self> {
        Box::new(Self {
            frames: vec![FlowFrame::new(items, tail, None)],
            paragraph: None,
            row: None,
            columns: None,
            row_entry: None,
            ready: None,
        })
    }
}

#[derive(Clone, Copy)]
enum MarginStart {
    Initial,
    Unforced,
    Forced,
}

#[derive(Clone, Copy, Default)]
struct BreakRequest {
    forced: Option<Forced>,
    preserve_empty: bool,
}

pub(super) struct Paginated {
    pub pages: Vec<Vec<TextLine>>,
    pub display_lists: Vec<Vec<DisplayItem>>,
    pub links: Vec<Vec<LinkArea>>,
    pub destinations: Vec<Option<Destination>>,
    pub string_locations: crate::layout::strings::StringLocations,
    pub page_boxes: Vec<std::sync::Arc<crate::document::PageBox>>,
    pub page_records: Vec<crate::layout::PageRecord>,
}

pub(super) type Paginator<'a, 'm, 't> = PaginationState<'a, 't, &'m mut Measurer<'t>>;

// Keep snapshots and ownership transfers exhaustive when pagination state gains a field.
macro_rules! pagination_state {
    ($( $(#[$attr:meta])* $field:ident: $value:ty, )*) => {
        #[derive(Clone)]
        pub(super) struct PaginationState<'a, 't, M> {
            measurer: M,
            $( $(#[$attr])* $field: $value, )*
        }
        impl<'a, 't, M> PaginationState<'a, 't, M> {
            fn with_measurer<N>(self, measurer: N) -> PaginationState<'a, 't, N> {
                PaginationState { measurer, $( $field: self.$field, )* }
            }
            fn copy_state(&self) -> PaginationState<'a, 't, ()> {
                PaginationState { measurer: (), $( $field: Clone::clone(&self.$field), )* }
            }
            fn restore_state(&mut self, snapshot: PaginationState<'a, 't, ()>) {
                $( self.$field = snapshot.$field; )*
            }
        }
    };
}

pagination_state! {
    string_locations: crate::layout::strings::StringLocations,
    pages: Vec<Vec<TextLine>>,
    page: usize,
    furthest_page: usize,
    forced_breaks: usize,
    parallel_depth: usize,
    reflow_lanes: usize,
    height: f32,
    top: f32,
    page_rect: Rect,
    y: f32,
    has_flow: bool,
    at_page_start: bool,
    /// Initial flow margins differ from margins adjoining an actual break.
    margin_start: MarginStart,
    pending: Vec<MarginStrut<'a>>,
    pending_break: Option<Forced>,
    floats: Vec<crate::layout::floats::FloatContext>,
    fragmentainers: Fragmentainers<'a, 't>,
    display_lists: Vec<Vec<ScopedPaint>>,
    foreground: Vec<Vec<ScopedPaint>>,
    underlays: HashMap<(usize, usize), Vec<ScopedPaint>>,
    flow_content: Vec<bool>,
    offset: [f32; 2],
    /// Stacking context of the current content and the key of its items.
    context: PaintKey,
    key: PaintKey,
    orders: PaintOrders,
    /// Tree-order slot reserved by an absolutely positioned box's anchor.
    reserved_order: Option<PaintOrder>,
    /// The next block is a flex item, which honours `z-index` unpositioned.
    flex_item: bool,
    /// Absolutely positioned content is laid out without page breaks.
    unbroken: usize,
    containers: Vec<Container<'a>>,
    fixed: Vec<Pending<'a>>,
    clip: Option<usize>,
    /// Innermost opacity group of the current stacking context.
    group: Option<usize>,
    /// 3D placement node of the box whose children are being laid out.
    parent_3d: Option<usize>,
    clips: Vec<EffectNode<'a>>,
    boxes: Vec<ActiveBox<'a>>,
    root_overflow: Option<BTreeMap<usize, Rect>>,
    tables: Vec<ActiveTable<'a>>,
    bands: Vec<BandContinuation<'a>>,
    page_content: bool,
    canvas: BoxPaint,
    canvas_slots: Vec<Option<usize>>,
    canvas_roots: Vec<(usize, usize)>,
    canvas_style: Option<&'a crate::style::Style>,
    canvas_rect: Rect,
    bottom_reserve: f32,
    sliced: Vec<SlicedBox<'a>>,
    /// What constrains each sticky box, indexed by `Effect::Sticky`.
    sticky: Vec<StickyConstraints>,
    open_slices: HashMap<*const crate::style::Style, usize>,
    /// Layers of inline elements that paint in their own, shared by their split boxes; their
    /// offsets resolve where they paint.
    inline_layers: HashMap<InlineElement, Layer>,
    /// Inline containing blocks of positioned boxes in creation order.
    inline_containers: Vec<(InlineElement, InlineContainer<'a>)>,
}
impl<M> PaginationState<'_, '_, M> {
    fn detach_orders(&mut self, snapshots: &mut OrderSnapshots) {
        snapshots.detach(&mut self.orders);
        for container in &mut self.containers {
            container.detach_orders(snapshots);
        }
        for pending in self.fixed.iter_mut().chain(
            self.inline_containers
                .iter_mut()
                .flat_map(|(_, container)| &mut container.pending),
        ) {
            snapshots.detach(&mut pending.orders);
        }
    }
}

impl<'a, 'm, 't> Paginator<'a, 'm, 't> {
    fn snapshot(&self) -> Result<PaginationState<'a, 't, ()>, LayoutError> {
        let lines: usize = self.pages.iter().map(Vec::len).sum();
        let paints: usize = self
            .display_lists
            .iter()
            .chain(&self.foreground)
            .map(Vec::len)
            .sum();
        self.measurer
            .work
            .spend((lines + paints + self.boxes.len() + self.pages.len()).max(1))?;
        let mut snapshot = self.copy_state();
        let mut orders = OrderSnapshots::default();
        snapshot.detach_orders(&mut orders);
        self.measurer.work.spend(orders.len())?;
        Ok(snapshot)
    }

    pub(crate) fn for_document(
        measurer: &'m mut Measurer<'t>,
        pages: crate::layout::physical_pages::PhysicalPages<'a, 't>,
        canvas: BoxPaint,
        canvas_rect: Rect,
    ) -> Self {
        Self::new(measurer, Fragmentainers::Pages(pages), canvas, canvas_rect)
    }
    /// Lays the document out on its first page without fragmenting it, as continuous media.
    pub(crate) fn unbroken(mut self) -> Self {
        self.unbroken = 1;
        self
    }
    fn new(
        measurer: &'m mut Measurer<'t>,
        fragmentainers: Fragmentainers<'a, 't>,
        canvas: BoxPaint,
        canvas_rect: Rect,
    ) -> Self {
        let height = fragmentainers.height(0);
        let page_rect = fragmentainers.rect(0);
        let top = fragmentainers.top(0);
        let mut paginator = Self {
            measurer,
            string_locations: Default::default(),
            pages: vec![Vec::new()],
            page: 0,
            furthest_page: 0,
            forced_breaks: 0,
            parallel_depth: 0,
            reflow_lanes: 0,
            height: fragmentainer_height(height),
            top,
            page_rect,
            y: 0.0,
            has_flow: false,
            at_page_start: true,
            margin_start: MarginStart::Initial,
            pending: Vec::new(),
            pending_break: None,
            floats: vec![crate::layout::floats::FloatContext::default()],
            fragmentainers,
            display_lists: vec![Vec::new()],
            underlays: HashMap::new(),
            foreground: vec![Vec::new()],
            flow_content: vec![false],
            offset: [0.0; 2],
            context: Rc::from([]),
            key: Rc::from([FLOW_LEVEL]),
            orders: PaintOrders::default(),
            reserved_order: None,
            flex_item: false,
            unbroken: 0,
            containers: Vec::new(),
            fixed: Vec::new(),
            clip: None,
            group: None,
            parent_3d: None,
            clips: Vec::new(),
            boxes: Vec::new(),
            root_overflow: None,
            tables: Vec::new(),
            bands: Vec::new(),
            page_content: false,
            canvas,
            canvas_slots: vec![None],
            canvas_roots: Vec::new(),
            canvas_style: None,
            canvas_rect,
            bottom_reserve: 0.0,
            sliced: Vec::new(),
            sticky: Vec::new(),
            open_slices: HashMap::new(),
            inline_layers: HashMap::new(),
            inline_containers: Vec::new(),
        };
        paginator.canvas(0);
        paginator
    }
    fn canvas(&mut self, page: usize) {
        let canvas = if page == self.page {
            self.canvas.clone()
        } else {
            match &self.fragmentainers {
                Fragmentainers::Pages(pages) => {
                    crate::layout::page_paint::PagePaint::new(
                        &pages.boxes[page],
                        pages.document.canvas_background(),
                        pages.records[page].output.bleed,
                    )
                    .canvas
                }
                Fragmentainers::Columns(_) => self.canvas.clone(),
            }
        };
        if let Some(slot) = self.canvas_slots[page] {
            self.display_lists[page][slot].item = DisplayItem::Box(canvas);
        } else if canvas.has_ink() {
            self.canvas_slots[page] = Some(self.display_lists[page].len());
            self.display_lists[page].push(ScopedPaint {
                visible: true,
                order: self.orders.take(),
                scope: Rc::from([ROOT_LEVEL]),
                clip: None,
                group: None,
                ink: None,
                item: DisplayItem::Box(canvas),
                navigation: BoxNavigation::default(),
            });
        }
    }
    fn finish_box_fragment(
        page: &mut [ScopedPaint],
        active: &mut ActiveBox<'_>,
        end: f32,
        last: bool,
    ) -> bool {
        let Some((paint, slot)) = &mut active.paint else {
            return false;
        };
        paint.rect.height = (end + active.offset[1] - paint.rect.y).max(0.0);
        paint.last_fragment = last;
        page[*slot].item = DisplayItem::Box(paint.clone());
        paint.has_ink()
    }
    fn normal_box_has_ink(active: &ActiveBox<'_>, page_rect: Rect) -> bool {
        active.paint.as_ref().is_some_and(|(paint, _)| {
            let mut rect = paint.rect;
            rect.x -= active.offset[0];
            rect.y -= active.offset[1];
            paint.has_ink() && rect.intersects(page_rect)
        })
    }
    fn current_page_has_ink(&self) -> bool {
        self.page_has_ink(self.page)
    }
    /// The shift on `page` of a sticky box whose border box there is `border`, once every box
    /// has its fragments.
    fn sticky_shift(
        &self,
        style: &crate::style::Style,
        constraints: &StickyConstraints,
        page: usize,
        border: Rect,
    ) -> [f32; 2] {
        let fragment = |fragments: &[(usize, usize)]| {
            fragments
                .iter()
                .find(|(fragment_page, _)| *fragment_page == page)
                .and_then(|&(_, slot)| match &self.display_lists[page][slot].item {
                    DisplayItem::Box(paint) => Some(paint.clone()),
                    _ => None,
                })
        };
        let Some(containing) = fragment(&constraints.containing) else {
            return [0.0; 2];
        };
        let scrollport = match &constraints.scrollport {
            Some(fragments) => match fragment(fragments) {
                Some(scroller) => scroller.inner_clip([0.0; 4], [true; 2]).rect,
                None => return [0.0; 2],
            },
            None => match &self.fragmentainers {
                Fragmentainers::Pages(pages) => pages.records[page].geometry.content_box(),
                Fragmentainers::Columns(_) => self.canvas_rect,
            },
        };
        crate::layout::sticky::offset(
            style,
            border,
            constraints.margins,
            containing.inner_clip(containing.padding, [true; 2]).rect,
            scrollport,
            constraints.containing_rtl,
        )
    }

    fn page_has_ink(&self, page: usize) -> bool {
        let page_rect = self.fragmentainers.rect(page);
        !self.pages[page].is_empty()
            || self.foreground[page].iter().any(|paint| match &paint.item {
                DisplayItem::Image(image) => image.visible_rect().intersects(page_rect),
                DisplayItem::Box(fragment) => {
                    fragment.has_ink() && fragment.rect.intersects(page_rect)
                }
                // Decorations accompany text lines, which are counted above.
                DisplayItem::Text { .. }
                | DisplayItem::Decoration(_)
                | DisplayItem::TextShadow(_)
                | DisplayItem::PushClip(_)
                | DisplayItem::PushTransform(_)
                | DisplayItem::PushOpacity(_)
                | DisplayItem::PushBlend(_)
                | DisplayItem::PushFilter(_)
                | DisplayItem::PushMask(_)
                | DisplayItem::Pop => false,
            })
            || self.display_lists[page]
                .iter()
                .skip(usize::from(self.canvas.has_ink()))
                .any(|item| match &item.item {
                    DisplayItem::Box(paint) => paint.has_ink(),
                    DisplayItem::Image(paint) => paint.visible_rect().intersects(page_rect),
                    // Decorations accompany text lines, which are counted above.
                    DisplayItem::Text { .. }
                    | DisplayItem::Decoration(_)
                    | DisplayItem::TextShadow(_)
                    | DisplayItem::PushClip(_)
                    | DisplayItem::PushTransform(_)
                    | DisplayItem::PushOpacity(_)
                    | DisplayItem::PushBlend(_)
                    | DisplayItem::PushFilter(_)
                    | DisplayItem::PushMask(_)
                    | DisplayItem::Pop => false,
                })
    }
    fn pending_value(&self) -> f32 {
        self.pending
            .iter()
            .map(|margin| match (self.at_page_start, self.margin_start) {
                (false, _) | (_, MarginStart::Initial) => margin.value(),
                (true, MarginStart::Unforced) => 0.0,
                (true, MarginStart::Forced) => margin.after_break().unwrap_or(margin.value()),
            })
            .sum()
    }
    fn flush_margin(&mut self) {
        self.y += self.pending_value();
        self.pending.clear();
    }
    /// Moves to fragmentainer `page`, recording how far the flow reached in the one left.
    fn enter(&mut self, page: usize) {
        if self.unbroken == 0
            && let Fragmentainers::Columns(rows) = &mut self.fragmentainers
        {
            rows.leave(self.page, self.y);
        }
        self.page = page;
        if self.unbroken == 0 {
            self.furthest_page = self.furthest_page.max(page);
        }
        self.height = self.fragmentainers.height(page);
        self.page_rect = self.fragmentainers.rect(page);
        self.canvas_rect = self.fragmentainers.content_box(page);
        if let Fragmentainers::Pages(pages) = &self.fragmentainers {
            self.top = self.canvas_rect.y;
            self.measurer.page_height = self.height;
            self.canvas = crate::layout::page_paint::PagePaint::new(
                &pages.boxes[page],
                pages.document.canvas_background(),
                pages.records[page].output.bleed,
            )
            .canvas;
        }
        if page < self.display_lists.len() {
            self.canvas(page);
        }
    }
    fn push_page(&mut self) {
        self.pages.push(Vec::new());
        self.display_lists.push(Vec::new());
        self.foreground.push(Vec::new());
        self.flow_content.push(false);
        self.canvas_slots.push(None);
        self.canvas(self.pages.len() - 1);
    }
    fn physical_flow(&self) -> bool {
        self.unbroken == 0
            && self.parallel_depth == 0
            && matches!(self.fragmentainers, Fragmentainers::Pages(_))
    }
    fn continuation_flow(&self) -> bool {
        self.unbroken == 0 && (self.parallel_depth == 0 || self.reflow_lanes == self.parallel_depth)
    }
    fn normal_flow(&self, source: &BlockBox) -> bool {
        self.continuation_flow()
            && !(source.style().floating().is_some() && source.image().is_some())
            && matches!(
                source.style().display,
                crate::style::Display::Block
                    | crate::style::Display::FlowRoot
                    | crate::style::Display::FlowRootListItem
                    | crate::style::Display::Grid
                    | crate::style::Display::ListItem
                    | crate::style::Display::TableCell
                    | crate::style::Display::TableCaption
            )
    }
    fn containing_geometry(&self) -> (f32, f32, bool, Option<f32>) {
        match self
            .boxes
            .last()
            .and_then(|active| active.geometry.as_ref())
        {
            Some(parent) => (
                parent.horizontal.x,
                parent.horizontal.width,
                parent.source.style().rtl,
                parent.definite_height(),
            ),
            None if matches!(self.fragmentainers, Fragmentainers::Columns(_)) => {
                let Fragmentainers::Columns(rows) = &self.fragmentainers else {
                    unreachable!()
                };
                rows.containing(self.page)
            }
            None => (
                self.canvas_rect.x,
                self.canvas_rect.width,
                false,
                Some(self.canvas_rect.height),
            ),
        }
    }
    fn flow_geometry(
        &self,
        page: usize,
        area: Rect,
        extra: Option<&ActiveGeometry<'a>>,
    ) -> Result<Vec<Option<ActiveGeometry<'a>>>, LayoutError> {
        let mut containing = match &self.fragmentainers {
            Fragmentainers::Pages(_) => (area.x, area.width, false, Some(area.height)),
            Fragmentainers::Columns(rows) => {
                let (_, _, rtl, height) = rows.containing(self.page);
                (area.x, area.width, rtl, height)
            }
        };
        let mut tables: Vec<(
            &BlockBox,
            &crate::layout::tables::TablePlan<'a>,
            crate::layout::tables::TableGeometry,
        )> = Vec::new();
        let mut grids: Vec<(
            Rc<crate::layout::grid::Geometry<'a>>,
            crate::layout::geometry::HorizontalBox,
        )> = Vec::new();
        let inputs = self
            .boxes
            .iter()
            .map(|active| active.geometry.as_ref())
            .chain(extra.map(Some))
            .collect::<Vec<_>>();
        let mut result = Vec::with_capacity(inputs.len());
        for input in &inputs {
            let Some(mut flow) = input.cloned() else {
                result.push(None);
                continue;
            };
            let style = flow.source.style();
            let mut grid_item = None;
            for (grid, parent) in grids.iter().rev() {
                grid_item = grid.item(flow.source, parent, self.measurer)?;
                if grid_item.is_some() {
                    break;
                }
            }
            flow.grid_item = grid_item.is_some();
            if let Some(plan) = input.and_then(|flow| flow.table_plan.as_deref()) {
                if !tables
                    .iter()
                    .any(|(source, _, _)| std::ptr::eq(*source, flow.source))
                {
                    let mut resolved = if let Some((geometry, _)) = &grid_item {
                        plan.resolve_positioned(self.measurer, &geometry.geometry)?
                    } else {
                        plan.resolve_geometry(
                            self.measurer,
                            containing.0,
                            containing.1,
                            containing.2,
                        )?
                    };
                    resolved.containing_height = grid_item
                        .as_ref()
                        .map(|(_, height)| *height)
                        .or(containing.3);
                    flow.horizontal = resolved.geometry.clone();
                    flow.containing_height = resolved.containing_height;
                    self.reflow_float_geometry(page, &mut flow)?;
                    resolved.geometry.clone_from(&flow.horizontal);
                    tables.push((flow.source, plan, resolved));
                }
                let resolved = &tables
                    .iter()
                    .find(|(source, _, _)| std::ptr::eq(*source, flow.source))
                    .unwrap()
                    .2;
                flow.horizontal = resolved.geometry.clone();
                flow.containing_height = resolved.containing_height;
                if flow.table_wrapper {
                    flow.horizontal.x -= flow.horizontal.padding[3] + flow.horizontal.border[3];
                    flow.horizontal.width += flow.horizontal.padding[1]
                        + flow.horizontal.padding[3]
                        + flow.horizontal.border[1]
                        + flow.horizontal.border[3];
                    flow.horizontal.padding = [0.0; 4];
                    flow.horizontal.border = [0.0; 4];
                }
            } else if let Some((plan, resolved, cell)) =
                tables.iter().rev().find_map(|(_, plan, resolved)| {
                    plan.cell_index(flow.source)
                        .map(|cell| (*plan, resolved, cell))
                })
            {
                flow.horizontal = plan.cell_geometry(cell, resolved);
                flow.containing_height = None;
            } else if let Some((item, height)) = grid_item {
                if let Some(grid) = &flow.grid {
                    flow.grid = Some(grid.reparent(item.parent.clone(), self.measurer)?);
                }
                flow.horizontal = item.geometry;
                flow.containing_height = Some(height);
            } else if style.floating().is_some() && flow.source.image().is_none() {
                flow.horizontal = crate::layout::inline::shrink_to_fit_geometry(
                    self.measurer,
                    flow.source,
                    containing.1,
                    containing.3,
                    containing.2,
                )?;
                flow.horizontal.containing_x = containing.0;
                flow.containing_height = containing.3;
                self.reflow_float_geometry(page, &mut flow)?;
            } else if flow.normal {
                flow.horizontal = crate::layout::measure::block_geometry(
                    self.measurer,
                    flow.source,
                    containing.0,
                    containing.1,
                    containing.2,
                    containing.3,
                )?;
                flow.containing_height = containing.3;
            }
            if flow.normal
                && let Some(sizing) = flow.sizing
            {
                let (height, constrained) = sizing.resolve(
                    style,
                    &flow.horizontal,
                    flow.containing_height,
                    flow.spanner_fragment,
                );
                if !height.is_finite() {
                    return Err(LayoutError::NonFiniteBlockGeometry {
                        block: self.measurer.sources.block(flow.source),
                    });
                }
                flow.height = height;
                flow.constrained |= constrained;
            }
            flow.relative = if let Some(relative) =
                tables.iter().rev().find_map(|(_, plan, resolved)| {
                    plan.cell_index(flow.source)
                        .map(|cell| plan.cell_offset(cell, resolved.geometry.width))
                }) {
                relative
            } else if flow
                .table_plan
                .as_ref()
                .is_some_and(|plan| plan.has_captions())
                && !flow.table_wrapper
            {
                [0.0; 2]
            } else {
                // The inline boxes a block splits offset it with them (CSS 2.1 §9.2.1.1).
                let enclosing = InlineContaining::of(containing);
                flow.source.enclosing_inline().iter().fold(
                    style.relative_offset(
                        flow.horizontal.containing_width,
                        flow.containing_height,
                        flow.horizontal.containing_rtl,
                    ),
                    |[x, y], inline| {
                        let [dx, dy] = enclosing.relative(inline);
                        [x + dx, y + dy]
                    },
                )
            };
            if let Some(grid) = &flow.grid {
                grids.push((grid.clone(), flow.horizontal.clone()));
            }
            containing = (
                flow.horizontal.x,
                flow.horizontal.width,
                style.rtl,
                flow.definite_height(),
            );
            result.push(Some(flow));
        }
        Ok(result)
    }
    fn paragraph_geometry(&self) -> Option<(f32, f32, Option<f32>)> {
        if !self.continuation_flow() {
            return None;
        }
        match self
            .boxes
            .last()
            .and_then(|active| active.geometry.as_ref())
        {
            Some(flow) if flow.normal => {
                let (x, width, _, height) = self.containing_geometry();
                Some((x, width, height))
            }
            None if matches!(self.fragmentainers, Fragmentainers::Columns(_)) => {
                let (x, width, _, height) = self.containing_geometry();
                Some((x, width, height))
            }
            _ => None,
        }
    }
    fn next_paragraph_geometry(&self) -> Result<Option<(f32, Option<f32>)>, LayoutError> {
        if !self.continuation_flow() {
            return Ok(None);
        }
        let Fragmentainers::Pages(pages) = &self.fragmentainers else {
            return Ok(None);
        };
        let page = pages.resolve(self.page + 1)?;
        let path = self.flow_geometry(self.page + 1, page.geometry.content_box(), None)?;
        Ok(path
            .last()
            .and_then(Option::as_ref)
            .filter(|flow| flow.normal)
            .map(|flow| (flow.horizontal.width, flow.definite_height())))
    }
    fn reserved_box_space(&self) -> f32 {
        self.boxes
            .iter()
            .filter(|active| active.reserves_end)
            .map(|active| active.end_space)
            .sum()
    }
    fn reflow_open_boxes(&mut self) -> Result<(), LayoutError> {
        if !self.continuation_flow() {
            return Ok(());
        }
        self.reflow_box_geometry()
    }
    fn reflow_box_geometry(&mut self) -> Result<(), LayoutError> {
        let path = self.flow_geometry(self.page, self.canvas_rect, None)?;
        let mut offset = match &self.fragmentainers {
            Fragmentainers::Pages(_) => [0.0; 2],
            Fragmentainers::Columns(rows) => rows.offset(self.page),
        };
        let old_reserve: f32 = self.reserved_box_space();
        for (active, updated) in self.boxes.iter_mut().zip(path) {
            let Some(updated) = updated else {
                continue;
            };
            let flow = active.geometry.as_mut().unwrap();
            let changed = updated.horizontal != flow.horizontal;
            *flow = updated;
            if flow.normal || changed {
                if let Some(placement) = &mut active.float_placement {
                    placement.reflow(flow, offset);
                }
                let relative = flow.relative;
                offset[0] += relative[0];
                offset[1] += relative[1];
                active.offset = offset;
                active.start_space = flow.start_space();
                active.end_space = flow.end_space(active.clone);
                if let Some((paint, _)) = &mut active.paint {
                    paint.rect.x =
                        flow.horizontal.x - flow.horizontal.padding[3] - flow.horizontal.border[3]
                            + offset[0];
                    paint.rect.width = flow.horizontal.width
                        + flow.horizontal.padding[1]
                        + flow.horizontal.padding[3]
                        + flow.horizontal.border[1]
                        + flow.horizontal.border[3];
                    paint.padding = flow.horizontal.padding;
                }
            } else {
                offset = active.offset;
            }
        }
        self.offset = offset;
        let new_reserve: f32 = self.reserved_box_space();
        self.bottom_reserve += new_reserve - old_reserve;
        let width = self.containing_geometry().1;
        for margin in &mut self.pending {
            *margin = margin.resolve(width, self.measurer)?;
        }
        Ok(())
    }
    fn page_name(&mut self, name: Option<std::sync::Arc<str>>) -> bool {
        if self.unbroken != 0 || self.parallel_depth != 0 {
            return false;
        }
        let Fragmentainers::Pages(pages) = &mut self.fragmentainers else {
            return false;
        };
        let changed = pages.records[self.page].request.name != name;
        pages.name = name;
        changed
    }
    fn cloned_end_space(&self) -> f32 {
        self.boxes
            .iter()
            .filter(|active| active.clone)
            .map(|active| active.end_space)
            .sum()
    }
    fn new_page(
        &mut self,
        forced_break: Option<Forced>,
        preserve_empty: bool,
    ) -> Result<(), LayoutError> {
        self.measurer.work.spend(1)?;
        let forced = forced_break.is_some();
        if forced {
            self.forced_breaks += 1;
        }
        // CSS Break 3 §5.3 fills the fragmentainer, reserving cloned end edges.
        let mut end =
            self.top + self.height - self.cloned_end_space() - self.band_footer_height(self.page);
        if forced {
            // Margins before a forced break are truncated; those after it are kept.
            for margin in &mut self.pending {
                *margin = margin.clone().truncated();
            }
        }
        let page = &mut self.display_lists[self.page];
        for (index, active) in self.boxes.iter_mut().enumerate().rev() {
            for band in &self.bands {
                if band.boxes == index + 1 {
                    end += band
                        .pages
                        .get(&self.page)
                        .map_or(0.0, |page| page.footer_height);
                }
            }
            let clone = active.clone;
            if clone {
                end += active.end_space;
            }
            if Self::finish_box_fragment(page, active, end, clone)
                && active
                    .paint
                    .as_ref()
                    .is_some_and(|(paint, _)| paint.rect.intersects(self.page_rect))
            {
                self.page_content = true;
            }
            active.record_float_fragment(self.page, self.top, false);
            self.flow_content[self.page] |= Self::normal_box_has_ink(active, self.page_rect);
        }
        if forced || self.page_content || self.flow_content[self.page] || preserve_empty {
            let candidate = if forced && self.parallel_depth == 0 {
                self.furthest_page + 1
            } else {
                self.page + 1
            };
            let next = self.fragmentainers.next(self.page, candidate, forced_break);
            if next >= MAX_LAYOUT_PAGES {
                return Err(LayoutError::PageLimit {
                    limit: MAX_LAYOUT_PAGES,
                });
            }
            if let Fragmentainers::Columns(rows) = &mut self.fragmentainers {
                rows.exit(self.page, next, forced_break);
            }
            if let Fragmentainers::Pages(pages) = &mut self.fragmentainers {
                pages.ensure_break(next)?;
            }
            self.enter(next);
            while self.pages.len() <= self.page {
                self.push_page();
            }
        }
        self.reflow_open_boxes()?;
        self.ensure_band_pages(self.page)?;
        let page = &mut self.display_lists[self.page];
        let mut start = 0.0;
        for (index, active) in self.boxes.iter_mut().enumerate() {
            for band in &self.bands {
                if band.boxes == index {
                    start += band
                        .pages
                        .get(&self.page)
                        .map_or(0.0, |page| page.header_height);
                }
            }
            if let Some(floating) = active
                .geometry
                .as_mut()
                .and_then(|flow| flow.floating.as_mut())
            {
                start = start.max(floating.top);
                floating.top = start;
                if let Some(placement) = &mut active.float_placement {
                    start += placement.begin_fragment(start, active.clone);
                }
            }
            let clone = active.clone;
            if let Some((paint, slot)) = &mut active.paint {
                paint.rect.y = self.top + start + active.offset[1];
                paint.rect.height = 0.0;
                paint.first_fragment = clone;
                match active
                    .fragments
                    .binary_search_by_key(&self.page, |(index, _)| *index)
                {
                    Ok(index) => {
                        *slot = active.fragments[index].1;
                        page[*slot].item = DisplayItem::Box(paint.clone());
                    }
                    Err(index) => {
                        *slot = page.len();
                        page.push(ScopedPaint {
                            visible: active.visible,
                            order: active.order.clone(),
                            scope: active.scope.clone(),
                            clip: active.clip,
                            group: active.group,
                            ink: None,
                            item: DisplayItem::Box(paint.clone()),
                            navigation: active.navigation,
                        });
                        active.fragments.insert(index, (self.page, *slot));
                    }
                }
            }
            if clone {
                start += active.start_space;
            }
            start += active.repeat_space;
            if let Some(marker) = &mut active.marker {
                marker.fragment.baseline = self.top + start + marker.ascent + active.offset[1];
            }
        }
        for band in &self.bands {
            if band.boxes == self.boxes.len() {
                start += band
                    .pages
                    .get(&self.page)
                    .map_or(0.0, |page| page.header_height);
            }
        }
        self.y = start;
        self.has_flow = false;
        self.at_page_start = true;
        self.margin_start = if forced {
            MarginStart::Forced
        } else {
            MarginStart::Unforced
        };
        self.page_content = self.current_page_has_ink();
        Ok(())
    }
    fn block(
        &mut self,
        block: MeasuredBlock<'a>,
        tail: BreakTail,
    ) -> Result<ActiveBox<'a>, LayoutError> {
        let (block, request) = self.prepare_block(block, tail)?;
        if let Some(request) = request {
            self.new_page(request.forced, request.preserve_empty)?;
        }
        // CSS Containment 2 §3.1: a size containment box is monolithic.
        let monolithic = usize::from(block.style.containment().size[1]);
        let mut frame = self.start_block(block)?;
        self.unbroken += monolithic;
        let flowed = self.flow(std::mem::take(&mut frame.items), tail);
        self.unbroken -= monolithic;
        flowed?;
        while let Some(request) = self.resume_block_end(&mut frame)? {
            self.new_page(request.forced, request.preserve_empty)?;
        }
        self.finish_block(frame)
    }
    fn refresh_block(
        &mut self,
        mut block: MeasuredBlock<'a>,
    ) -> Result<MeasuredBlock<'a>, LayoutError> {
        let clearance_before = block.clearance_before.take();
        let starts_source = self.starts_source(block.source);
        if starts_source && self.continuation_flow() {
            let mut grid_item = None;
            for parent in self
                .boxes
                .iter()
                .rev()
                .filter_map(|active| active.geometry.as_ref())
            {
                if let Some(grid) = &parent.grid {
                    grid_item = grid.item(block.source, &parent.horizontal, self.measurer)?;
                    if grid_item.is_some() {
                        break;
                    }
                }
            }
            if let Some((item, height)) = grid_item {
                if item.geometry != block.geometry
                    || Some(height) != block.containing_height
                    || block
                        .grid
                        .as_ref()
                        .is_some_and(|grid| !grid.matches_parent(item.parent.as_ref()))
                {
                    block = item.measure_area(self.measurer, height)?;
                }
                block.clearance_before = clearance_before;
                return Ok(block);
            }
        }
        let source_geometry = self.physical_flow()
            || (self.continuation_flow()
                && block.style.display != crate::style::Display::TableCell);
        if source_geometry && starts_source && block.style.floating().is_none() {
            let (x, width, rtl, height) = self.containing_geometry();
            if x != block.geometry.containing_x
                || width != block.geometry.containing_width
                || rtl != block.geometry.containing_rtl
                || (!block.style.display.is_table() && height != block.containing_height)
            {
                if let Some(edges) = block.spanner_fragment {
                    self.measurer.work.spend(block.items.len().max(1))?;
                    let geometry = crate::layout::measure::block_geometry(
                        self.measurer,
                        block.source,
                        x,
                        width,
                        rtl,
                        height,
                    )?;
                    block.resize_fragment(geometry, height, edges);
                } else {
                    let root = self.boxes.is_empty()
                        && matches!(self.fragmentainers, Fragmentainers::Pages(_));
                    let previous = self.measurer.fragmentainer;
                    self.measurer.fragmentainer = self.fragmentainers.kind();
                    let measured = self
                        .measurer
                        .block(block.source, x, width, rtl, root, height);
                    self.measurer.fragmentainer = previous;
                    block = measured?;
                }
            }
        }
        block.clearance_before = clearance_before;
        Ok(block)
    }
    fn prepare_block(
        &mut self,
        mut block: MeasuredBlock<'a>,
        tail: BreakTail,
    ) -> Result<(MeasuredBlock<'a>, Option<BreakRequest>), LayoutError> {
        self.measurer.work.spend(1)?;
        let changed = if crate::document::PageBoundary::applies(&block.style.display) {
            self.page_name(block.source.page_boundary().start.clone())
        } else {
            false
        };
        let forced = Forced::combine(
            Forced::combine(self.pending_break, Forced::of(block.break_before)),
            changed.then_some(Forced::Page),
        );
        if forced.is_none() {
            block = self.refresh_block(block)?;
            block = self.resolve_clearance_margins(block)?;
            self.clear_block(&block)?;
            self.avoid_floats(&block)?;
        }
        let reserve = self.continuation_reserve()?;
        let fits_float = block.style.floating().is_some()
            && !overflows(block.overflow_height + reserve, self.height);
        let keep =
            (self.fragmentainers.kind().keeps(block.style) || fits_float) && !block.has_forced;
        let required = if block.first_fragment == 0.0 && block.height == 0.0 {
            0.0
        } else if keep {
            block.overflow_height + self.closing_reserve(tail, reserve)?
        } else if block.first_fragment >= block.height {
            block.first_fragment + self.closing_reserve(tail, reserve)?
        } else {
            block.first_fragment + reserve
        };
        let atomic = matches!(block.items.first(), Some(FlowItem::Image { .. }));
        let float_occupied = self
            .floats
            .last()
            .expect("flow has a float context")
            .occupies_page(self.page, self.height, &self.measurer.work)?;
        let occupied = self.has_flow || float_occupied;
        let request = (self.unbroken == 0
            && occupied
            && (forced.is_some()
                || (required > 0.0
                    && (!overflows(required, self.height) || atomic || keep)
                    && overflows(self.y + self.pending_value() + required, self.height))))
        .then_some(BreakRequest {
            forced,
            preserve_empty: atomic || float_occupied,
        });
        if changed && request.is_some() {
            self.pending.clear();
            self.pending.push(block.before.clone().after_forced_break());
        }
        Ok((block, request))
    }
    /// Whether `source` starts a box rather than continuing its parent's: a caption wrapper
    /// and its inner table share one source element.
    fn starts_source(&self, source: &BlockBox) -> bool {
        self.boxes.last().is_none_or(|parent| {
            parent
                .geometry
                .as_ref()
                .is_none_or(|flow| !std::ptr::eq(flow.source, source))
        })
    }
    fn start_block(&mut self, block: MeasuredBlock<'a>) -> Result<BlockFrame<'a>, LayoutError> {
        let mut block = self.refresh_block(block)?;
        let starts_source = self.starts_source(block.source);
        if let Some(element) = block.source.query_container() {
            let vertical = block.geometry.padding[0]
                + block.geometry.padding[2]
                + block.geometry.border[0]
                + block.geometry.border[2];
            self.measurer
                .container_sizes
                .entry(element)
                .or_insert([block.geometry.width, (block.height - vertical).max(0.0)]);
        }
        let track_overflow = (block.grid.is_some() && block.style.contains_absolute())
            || self
                .boxes
                .last()
                .is_some_and(|parent| parent.overflow.is_some())
            || self.root_overflow.is_some();
        if track_overflow {
            block.retain_fragment();
        }
        if self.at_page_start && !matches!(self.margin_start, MarginStart::Initial) {
            block = self.resolve_clearance_margins(block)?;
        }
        self.clear_block(&block)?;
        block.offset[0] += self.avoid_floats(&block)?;
        let table_plan = if let Some(table) = &block.table {
            Some(table.plan.clone())
        } else if block.style.display.is_table() {
            self.measurer.work.spend(block.items.len().max(1))?;
            block.items.iter().find_map(|item| {
                if let FlowItem::Block(child) = item
                    && std::ptr::eq(child.source, block.source)
                {
                    child.table.as_ref().map(|table| table.plan.clone())
                } else {
                    None
                }
            })
        } else {
            None
        };
        let table_wrapper = block.table.is_none() && table_plan.is_some();
        self.pending_break = None;
        if block.constrained {
            self.flush_margin();
        }
        let float_context = self.floats.len() - 1;
        if block.independent {
            self.floats
                .push(crate::layout::floats::FloatContext::default());
            self.flush_margin();
            self.at_page_start = false;
        }
        if let Some(event) = block.source.semantic_event() {
            self.string_locations.record(self.page, event);
        }
        let start_page = self.page;
        let forced_breaks = self.forced_breaks;
        let start_y = self.y;
        let previous_offset = self.offset;
        let previous_key = self.key.clone();
        let previous_context = self.context.clone();
        let previous_clip = self.clip;
        let previous_group = self.group;
        let previous_parent_3d = self.parent_3d;
        let split = self.enter_split_inline(&mut block, starts_source);
        self.offset[0] += block.offset[0];
        self.offset[1] += block.offset[1];
        let flex_item = std::mem::take(&mut self.flex_item);
        let owns_wrapper_properties = block
            .table
            .as_ref()
            .is_none_or(|table| !table.plan.has_captions());
        let positioned = owns_wrapper_properties && block.style.position != Position::Static;
        // Transforms, perspective and preserve-3d contain all descendants (CSS Transforms 2 §5).
        let transformed = owns_wrapper_properties && block.style.has_transform_effects();
        let grouped = owns_wrapper_properties && block.style.is_grouped();
        let isolated = owns_wrapper_properties && block.style.forced_stacking_context();
        let containing = owns_wrapper_properties && block.style.contains_absolute();
        let sticky = owns_wrapper_properties && block.style.position == Position::Sticky;
        if transformed
            && (block.style.perspective.is_some() || block.style.preserves_3d())
            && (block.style.multicol.is_multicol() || block.style.display.is_table())
        {
            return Err(LayoutError::Style(crate::style::StyleError::Unsupported(
                "perspective or transform-style: preserve-3d on a multi-column container or table"
                    .into(),
            )));
        }
        // A transform, opacity, blending, isolation or forced stacking context makes a stacking
        // context like z-index 0 (CSS Transforms 1 §3, CSS Color 4 §15, Compositing 1 §3.4,
        // CSS Will Change 1 §3.1).
        let z_index = (positioned || flex_item)
            .then_some(block.style.stacking_z_index())
            .flatten()
            .or((transformed || grouped || isolated).then_some(0));
        let mut own_group = None;
        let paint_key = if positioned || z_index.is_some() {
            let order = self.reserved_order.take().unwrap_or_else(|| self.order());
            let layer = positioned_layer(&self.context, z_index, order);
            if z_index.is_some() {
                let group = if grouped {
                    Group::of(block.style)
                } else {
                    Group::IDENTITY
                };
                let node = self.stacking_group(self.group, group, layer.context.clone());
                own_group = group.anchored().then_some(node);
                self.group = Some(node);
            }
            self.context = layer.context;
            self.key = layer.key;
            layer.background
        } else if block.style.floating().is_some() {
            let order = self.order();
            self.key = extend(&self.key, (FLOAT_PAINT_LEVEL, order));
            self.key.clone()
        } else {
            self.key.clone()
        };
        let previous_reserve = self.bottom_reserve;
        self.bottom_reserve += block.end_space;
        let sticky_node = (sticky && block.paint.is_some()).then(|| {
            self.sticky_node(
                block.style,
                block.geometry.margin.map(|margin| margin.unwrap_or(0.0)),
                block.geometry.containing_rtl,
                self.clip,
            )
        });
        if sticky_node.is_some() {
            self.clip = sticky_node;
        }
        let transform_node = (owns_wrapper_properties
            && block.style.is_transformed()
            && block.paint.is_some())
        .then(|| self.effect_node(self.clip, Effect::Transform(block.style, self.parent_3d)));
        if transform_node.is_some() {
            self.clip = transform_node;
        }
        // CSS Masking 1 §6.1: `clip` cuts the box and its descendants, in its own coordinates.
        let clip_rect_node = (owns_wrapper_properties
            && block.style.position.is_out_of_flow()
            && block.style.clip.is_some()
            && block.paint.is_some())
        .then(|| self.effect_node(self.clip, Effect::RectClip(block.style)));
        if clip_rect_node.is_some() {
            self.clip = clip_rect_node;
        }
        if let Some(node) = own_group {
            self.anchor_group(node, self.clip);
        }
        let parent_3d_node = (transformed
            && block.paint.is_some()
            && (block.style.perspective.is_some() || block.style.preserves_3d()))
        .then(|| self.effect_node(self.clip, Effect::Parent3d(block.style)));
        // Anonymous boxes are not their content's parent element.
        if !block.source.is_anonymous() {
            self.parent_3d = parent_3d_node;
        }
        let canvas_root = self.canvas.has_ink()
            && block.canvas_source
            && std::ptr::eq(*self.canvas_style.get_or_insert(block.style), block.style);
        let paint_order = self.order();
        let paint = if let Some(mut paint) = block.paint {
            if canvas_root {
                paint.decoration.backgrounds = self.canvas.decoration.backgrounds.clone();
                paint.decoration.background = self.canvas.decoration.background;
            }
            self.flush_margin();
            paint.rect.x += self.offset[0];
            paint.rect.y = self.top + self.y + self.offset[1];
            let page = &mut self.display_lists[self.page];
            let slot = page.len();
            page.push(ScopedPaint {
                visible: block.style.visibility.is_visible(),
                order: paint_order.clone(),
                scope: paint_key.clone(),
                clip: self.clip,
                group: self.group,
                ink: None,
                item: DisplayItem::Box(paint.clone()),
                navigation: block.navigation,
            });
            Some((paint, slot))
        } else {
            None
        };
        let marker = block.marker.map(|mut marker| {
            marker.fragment.baseline +=
                self.top + self.y + self.pending_value() + block.start_space + self.offset[1];
            marker.fragment.x += self.offset[0];
            marker
        });
        let fragments = paint
            .as_ref()
            .map(|(_, slot)| vec![(self.page, *slot)])
            .unwrap_or_default();
        let collapsed_clip = self
            .tables
            .last()
            .is_some_and(|table| table.flow.plan.cell_clipped(block.source));
        let clip_node = (owns_wrapper_properties
            && paint.is_some()
            && (collapsed_clip || block.style.clipped_axes().contains(&true)))
        .then(|| {
            self.clips.push(EffectNode {
                areas: Vec::new(),
                paints: Vec::new(),
                parent: self.clip,
                effect: if collapsed_clip {
                    Effect::BorderClip
                } else {
                    Effect::Clip(block.style.clipped_axes())
                },
                fragments: Vec::new(),
                context: None,
            });
            self.clips.len() - 1
        });
        let mut grid_children = if block.grid.is_some() {
            block
                .source
                .children()
                .iter()
                .filter_map(|child| match child {
                    crate::document::BlockContent::Block(child) => {
                        Some((child.style().flex.order, child))
                    }
                    crate::document::BlockContent::Positioned(child) => Some((0, child)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        grid_children.sort_by_key(|(order, _)| *order);
        let grid_orders = grid_children
            .into_iter()
            .map(|(_, child)| (std::ptr::from_ref(child), self.order()))
            .collect();
        self.boxes.push(ActiveBox {
            grid_orders,
            grid: block.grid.clone(),
            overflow: track_overflow.then(BTreeMap::new),
            visible: block.style.visibility.is_visible(),
            pending: Vec::new(),
            float_placement: None,
            geometry: Some(ActiveGeometry {
                grid_item: false,
                grid: block.grid.clone(),
                relative: block.offset,
                floating: block.style.floating().map(|_| floats::FloatFlow {
                    context: float_context,
                    top: 0.0,
                }),
                source: block.source,
                horizontal: block.geometry.clone(),
                containing_height: block.containing_height,
                normal: self.normal_flow(block.source),
                table_plan,
                table_wrapper,
                sizing: block.sizing,
                height: block.height,
                constrained: block.constrained,
                spanner_fragment: block.spanner_fragment,
            }),
            page_scope: crate::document::PageBoundary::applies(&block.style.display)
                .then(|| block.source.page_boundary().used.clone()),
            offset: self.offset,
            scope: paint_key,
            order: paint_order,
            clip: self.clip,
            group: self.group,
            clip_node,
            transform_node,
            clip_rect_node,
            parent_3d_node,
            sticky_node,
            sticky_children: Vec::new(),
            sticky_scrolled: Vec::new(),
            group_node: own_group,
            paint,
            fragments,
            start_space: block.start_space,
            repeat_space: block.repeat_space,
            end_space: block.end_space,
            reserves_end: true,
            clone: block.style.clone_decoration,
            marker,
            sliced: None,
            canvas_root,
            navigation: block.navigation,
        });
        if clip_node.is_some() {
            self.clip = clip_node;
        }
        if containing {
            self.containers.push(Container {
                pending: Vec::new(),
                fixed: block.style.contains_fixed(),
            });
        }
        let table = block.table.is_some();
        if let Some(flow) = block.table {
            self.tables.push(ActiveTable {
                shadows: Vec::new(),
                transforms: Vec::new(),
                layers: HashMap::new(),
                cell_fragments: HashMap::new(),
                cell_overflow: HashMap::new(),
                positioned: Vec::new(),
                cell_heights: vec![0.0; flow.heights.len()],
                backgrounds: Vec::new(),
                grids: flow
                    .collapsed
                    .clone()
                    .map(|grid| (self.page, grid))
                    .into_iter()
                    .collect(),
                rows: HashMap::new(),
                next_body: flow.body.start,
                flow,
                x_offset: self.offset[0],
                placements: Vec::new(),
            });
        }
        let continues = block.continues;
        Ok(BlockFrame {
            grow_after_break: false,
            clipped_height: None,
            defer_positioned: false,
            end: None,
            style: block.style,
            height: block.height,
            independent: block.independent,
            break_after: block.break_after,
            continues,
            table,
            containing,
            start_page,
            forced_breaks,
            start_y,
            previous_offset,
            previous_key,
            previous_context,
            previous_clip,
            previous_group,
            previous_parent_3d,
            previous_reserve,
            split,
            items: block.items,
        })
    }
    fn resume_block_end(
        &mut self,
        frame: &mut BlockFrame<'a>,
    ) -> Result<Option<BreakRequest>, LayoutError> {
        if frame.end.is_none() {
            let float_natural_height = self.float_enclosure(frame)?;
            if frame.table {
                let table = self.tables.last().expect("table pushed before its items");
                let starts = if !table.backgrounds.is_empty()
                    || table.flow.collapsed.is_some()
                    || !table.positioned.is_empty()
                    || !table.transforms.is_empty()
                    || !table.shadows.is_empty()
                {
                    table.flow.plan.used_row_starts(
                        self.measurer,
                        &table.cell_heights,
                        table.flow.spacing,
                        table
                            .flow
                            .original_row_starts
                            .as_deref()
                            .map(|original| (original, table.flow.row_starts.as_ref())),
                    )?
                } else {
                    Vec::new()
                };
                self.place_table_pending(&starts)?;
                let table = self
                    .tables
                    .pop()
                    .expect("table remains active through positioned descendants");
                self.table_backgrounds(&table, &starts);
                self.collapsed_borders(table, &starts)?;
            }
            let active = self
                .boxes
                .last_mut()
                .expect("a finishing block has an active box");
            let reflowed = self.page != frame.start_page
                && active.geometry.as_ref().is_some_and(|flow| flow.normal);
            active.reserves_end = false;
            self.bottom_reserve = if reflowed {
                self.bottom_reserve - active.end_space
            } else {
                frame.previous_reserve
            };
            let constrained = active
                .geometry
                .as_ref()
                .is_some_and(|flow| flow.constrained);
            if frame.independent {
                self.flush_margin();
            }
            let extent = ((constrained && self.forced_breaks == frame.forced_breaks)
                || float_natural_height.is_some()
                || frame.clipped_height.is_some())
            .then(|| Self::start_extent(frame.start_page, frame.start_y, true));
            let fragmented_height = if frame.grow_after_break && self.page > frame.start_page {
                let mut height = 0.0;
                let mut origin = frame.start_y;
                for page in frame.start_page..self.page {
                    let space = self.extent_space(page)?;
                    height += (self.fragmentainers.height(page)
                        - space.cloned_end
                        - self.band_footer_height(page)
                        - origin)
                        .max(0.0);
                    origin = self.extent_space(page + 1)?.start;
                }
                Some(height + (self.y - origin).max(0.0))
            } else {
                None
            };
            frame.end = Some(BlockEnd {
                fragmented_height,
                reflowed,
                extent,
                float_natural_height,
            });
        }
        let end = frame.end.as_mut().unwrap();
        if let Some(extent) = &mut end.extent {
            if let Some(request) = self.resume_extent(extent, |paginator, _, space| {
                if let Some(height) = frame.clipped_height {
                    return Ok(height);
                }
                if let Some(natural) = end.float_natural_height.or_else(|| {
                    end.fragmented_height
                        .map(|height| height.max(space.height.unwrap_or(frame.height)))
                }) {
                    let geometry = paginator
                        .boxes
                        .last()
                        .and_then(|active| active.geometry.as_ref())
                        .expect("enclosing block retains geometry");
                    Ok(crate::layout::geometry::VerticalBox::new(
                        frame.style,
                        &geometry.horizontal,
                        geometry.containing_height,
                    )
                    .used_height(natural))
                } else {
                    Ok(space.height.unwrap_or(frame.height))
                }
            })? {
                return Ok(Some(request));
            }
            end.extent = None;
        }
        Ok(None)
    }
    fn finish_block(&mut self, frame: BlockFrame<'a>) -> Result<ActiveBox<'a>, LayoutError> {
        let BlockFrame {
            style,
            height,
            break_after,
            continues,
            containing,
            previous_offset,
            previous_key,
            previous_context,
            previous_clip,
            previous_group,
            previous_parent_3d,
            split,
            end,
            ..
        } = frame;
        if frame.independent {
            self.floats
                .pop()
                .expect("independent flow owns a float context");
        }
        let reflowed = end
            .expect("block extent completed before final painting")
            .reflowed;
        if height > 0.0 {
            self.has_flow = true;
            // Margins after a box with height no longer adjoin the page start.
            self.at_page_start = false;
        }
        self.place_inline_pending(self.boxes.len())?;
        let mut active = self.boxes.pop().unwrap();
        for node in [
            active.clip_node,
            active.transform_node,
            active.clip_rect_node,
            active.parent_3d_node,
            active.sticky_node,
            active.group_node,
        ]
        .into_iter()
        .flatten()
        {
            self.clips[node].fragments.clone_from(&active.fragments);
        }
        for &node in &split.nodes {
            self.clips[node]
                .fragments
                .extend_from_slice(&active.fragments);
        }
        for &index in &active.sticky_children {
            self.sticky[index].containing.clone_from(&active.fragments);
        }
        for &index in &active.sticky_scrolled {
            self.sticky[index].scrollport = Some(active.fragments.clone());
        }
        for &(page, slot) in &active.fragments {
            if page > self.page
                && let DisplayItem::Box(paint) = &mut self.display_lists[page][slot].item
            {
                paint.rect.height = 0.0;
            }
        }
        if let Some(marker) = active.marker.take()
            && active.visible
        {
            self.emit_marker(*marker);
            self.has_flow = true;
            self.at_page_start = false;
        }
        // A fragment before a spanner keeps its end edge only when cloned.
        let last = !continues || active.clone;
        if Self::finish_box_fragment(
            &mut self.display_lists[self.page],
            &mut active,
            self.top + self.y,
            last,
        ) && active
            .paint
            .as_ref()
            .is_some_and(|(paint, _)| paint.rect.intersects(self.page_rect))
        {
            self.page_content = true;
        }
        active.record_float_fragment(self.page, self.top, !continues);
        // An in-flow box ending inside the page area occupies the page even without ink.
        self.flow_content[self.page] |= Self::normal_box_has_ink(&active, self.page_rect)
            || (self.unbroken == 0 && height > 0.0 && !overflows(self.y, self.height));
        if !active.clone && active.paint.is_some() {
            active.sliced = Some(self.register_slice(style, active.fragments.clone()));
        }
        if active.canvas_root {
            self.canvas_roots.extend(active.fragments.iter().copied());
        }
        if containing {
            active.pending = self
                .containers
                .pop()
                .map(|container| container.pending)
                .unwrap_or_default();
            if !frame.defer_positioned {
                self.place_box_pending(&mut active, style.rtl)?;
            }
        }
        if !frame.defer_positioned {
            self.propagate_overflow(&active, style)?;
        }
        self.offset = if reflowed || (self.page != frame.start_page && self.continuation_flow()) {
            self.boxes.last().map_or_else(
                || match &self.fragmentainers {
                    Fragmentainers::Pages(_) => previous_offset,
                    Fragmentainers::Columns(rows) => rows.offset(self.page),
                },
                |parent| parent.offset,
            )
        } else {
            previous_offset
        };
        self.key = previous_key;
        self.context = previous_context;
        self.clip = previous_clip;
        self.group = previous_group;
        self.parent_3d = previous_parent_3d;
        // The empty box each inline box continues in after the block, in a line box of zero
        // height (CSS 2.1 §9.4.2).
        let end = self.top + self.y;
        self.close_inline_containers(&split.containers);
        for (style, layer) in &split.containers {
            self.split_fragment(style, layer, end);
        }
        if let Some(forced) = Forced::of(break_after) {
            self.pending_break = Some(forced);
        }
        Ok(active)
    }
    /// Lays out a flow's items in order.
    fn flow(&mut self, items: Vec<FlowItem<'a>>, tail: BreakTail) -> Result<(), LayoutError> {
        let mut cursor = FlowCursor::new(items, tail);
        while let Some((next, request)) = self.resume_flow(cursor)? {
            self.new_page(request.forced, request.preserve_empty)?;
            cursor = next;
        }
        Ok(())
    }
    fn resume_flow(
        &mut self,
        mut cursor: Box<FlowCursor<'a, 't>>,
    ) -> Result<Option<(Box<FlowCursor<'a, 't>>, BreakRequest)>, LayoutError> {
        while !cursor.frames.is_empty() {
            if let Some(mut columns) = cursor.columns.take() {
                if let Some(request) = self.resume_columns(&mut columns)? {
                    cursor.columns = Some(columns);
                    return Ok(Some((cursor, request)));
                }
                continue;
            }
            if let Some(entry) = cursor.row_entry.take() {
                cursor.row = Some(Box::new(self.start_row(entry)?));
                continue;
            }
            if let Some(mut row) = cursor.row.take() {
                if let Some(request) = self.resume_row(&mut row)? {
                    cursor.row = Some(row);
                    return Ok(Some((cursor, request)));
                }
                self.finish_row(*row)?;
                continue;
            }
            if let Some((paragraph, tail)) = cursor.paragraph.take() {
                if let Some(next) = self.paragraph_fragment(paragraph, tail)? {
                    cursor.paragraph = Some((next, tail));
                    return Ok(Some((cursor, BreakRequest::default())));
                }
                continue;
            }
            let (item, tail, ready) = if let Some((item, tail)) = cursor.ready.take() {
                (item, tail, true)
            } else {
                let frame = cursor.frames.last_mut().unwrap();
                let Some((item, tail)) = frame.items.next() else {
                    if let Some(block) = &mut frame.block
                        && let Some(request) = self.resume_block_end(block)?
                    {
                        return Ok(Some((cursor, request)));
                    }
                    let frame = cursor.frames.pop().unwrap();
                    if let Some(block) = frame.block {
                        self.finish_block(block)?;
                    }
                    continue;
                };
                self.measurer.work.spend(1)?;
                (item, tail, false)
            };
            match item {
                FlowItem::Image {
                    mut paint,
                    height,
                    visible,
                } => {
                    self.flush_margin();
                    let mut normal_rect = paint.visible_rect();
                    normal_rect.y += self.top + self.y;
                    self.flow_content[self.page] |= normal_rect.intersects(self.page_rect);
                    paint.rect.x += self.offset[0];
                    paint.rect.y += self.top + self.y + self.offset[1];
                    if let Some(clip) = &mut paint.clip {
                        clip.translate([self.offset[0], self.top + self.y + self.offset[1]]);
                    }
                    if paint.visible_rect().intersects(self.page_rect)
                        && paint.rect.intersects(paint.visible_rect())
                    {
                        let scope = self.inline_key();
                        self.foreground[self.page].push(ScopedPaint {
                            visible,
                            order: self.orders.take(),
                            scope,
                            clip: self.clip,
                            group: self.group,
                            ink: None,
                            item: DisplayItem::Image(paint),
                            navigation: BoxNavigation::default(),
                        });
                        self.page_content = true;
                    }
                    self.y += height;
                    self.has_flow = true;
                    self.at_page_start = false;
                }
                FlowItem::Block(child) | FlowItem::Spanner(child) => {
                    let child = if ready {
                        *child
                    } else {
                        let (child, request) = self.prepare_block(*child, tail)?;
                        if let Some(request) = request {
                            cursor.ready = Some((FlowItem::Block(Box::new(child)), tail));
                            return Ok(Some((cursor, request)));
                        }
                        child
                    };
                    let mut block = self.start_block(child)?;
                    let items = std::mem::take(&mut block.items);
                    cursor.frames.push(FlowFrame::new(items, tail, Some(block)));
                }
                FlowItem::Columns(columns) => {
                    cursor.columns = Some(Box::new(columns::ColumnCursor::new(*columns, tail)));
                }
                FlowItem::Paragraph(paragraph) => {
                    if !ready && let Some(request) = self.paragraph_break() {
                        cursor.ready = Some((FlowItem::Paragraph(paragraph), tail));
                        return Ok(Some((cursor, request)));
                    }
                    cursor.paragraph = Some((self.start_paragraph(paragraph), tail));
                }
                FlowItem::Row(row) => {
                    let (entry, request) = self.prepare_row(*row, tail)?;
                    cursor.row_entry = Some(entry);
                    if let Some(request) = request {
                        return Ok(Some((cursor, request)));
                    }
                }
                FlowItem::Margin(margin) => {
                    let width = self.containing_geometry().1;
                    let margin = margin.resolve(width, self.measurer)?;
                    if !margin.is_empty() {
                        self.pending.push(margin);
                    }
                }
                FlowItem::Edge(edge, measured) => {
                    let value = self
                        .boxes
                        .last()
                        .filter(|active| self.continuation_flow() && active.geometry.is_some())
                        .map_or(measured, |active| match edge {
                            BlockEdge::Start => active.start_space,
                            BlockEdge::End => active.end_space,
                        });
                    if value != 0.0 {
                        self.space(value);
                    }
                }
                FlowItem::Space(value) if value != 0.0 => self.space(value),
                FlowItem::Space(_) => {}
                FlowItem::Events(events) => {
                    for event in events {
                        self.string_locations.record(self.page, event);
                    }
                }
                FlowItem::Positioned(item) => {
                    if item.block.style().floating().is_some() {
                        self.place_float(
                            item.block,
                            item.left,
                            item.right - item.left,
                            self.y + self.pending_value() + item.margin,
                        )?;
                        continue;
                    }
                    let position = StaticPosition {
                        page: self.page,
                        left: item.left + self.offset[0],
                        right: item.right + self.offset[0],
                        y: self.top + self.y + self.pending_value() + item.margin + self.offset[1],
                        alignment: item.alignment,
                    };
                    let layer = self.current_layer();
                    self.anchor(item.block, position, &layer, self.parent_3d);
                }
            }
        }
        Ok(None)
    }
    fn order(&self) -> PaintOrder {
        self.orders.take()
    }
    /// The group of stacking context `context` inside `parent`, which a blending group
    /// isolates.
    fn stacking_group(
        &mut self,
        parent: Option<usize>,
        group: Group<'a>,
        context: PaintKey,
    ) -> usize {
        if group.blend != crate::graphics::BlendMode::Normal {
            self.isolate(parent);
        }
        self.clips.push(EffectNode {
            areas: Vec::new(),
            paints: Vec::new(),
            parent,
            effect: Effect::Group(group),
            fragments: Vec::new(),
            context: Some(context),
        });
        self.clips.len() - 1
    }
    /// Content blends only with its own stacking context, so that context is isolated; the
    /// root stacking context is the page.
    fn isolate(&mut self, group: Option<usize>) {
        if let Some(group) = group
            && let Effect::Group(group) = &mut self.clips[group].effect
        {
            group.isolated = true;
        }
    }
    /// Opens anchored group `node` inside clip or transform `anchor` and those enclosing it.
    fn anchor_group(&mut self, node: usize, anchor: Option<usize>) {
        if let Effect::Group(group) = &mut self.clips[node].effect {
            group.anchor = anchor;
        }
    }
    fn effect_node(&mut self, parent: Option<usize>, effect: Effect<'a>) -> usize {
        self.clips.push(EffectNode {
            areas: Vec::new(),
            paints: Vec::new(),
            parent,
            effect,
            fragments: Vec::new(),
            context: None,
        });
        self.clips.len() - 1
    }
    /// The shift node of a sticky box with `margins` inside `parent`, constrained by the
    /// current block container and the nearest scroll container (CSS Position 3 §3.4).
    fn sticky_node(
        &mut self,
        style: &'a crate::style::Style,
        margins: [f32; 4],
        containing_rtl: bool,
        parent: Option<usize>,
    ) -> usize {
        let index = self.sticky.len();
        let scrolled = self.boxes.iter().rposition(|active| {
            active
                .geometry
                .as_ref()
                .is_some_and(|geometry| geometry.source.style().is_scroll_container())
        });
        self.sticky.push(StickyConstraints {
            margins,
            containing_rtl,
            containing: Vec::new(),
            scrollport: scrolled.map(|_| Vec::new()),
        });
        if let Some(parent) = self.boxes.last_mut() {
            parent.sticky_children.push(index);
        }
        if let Some(scrolled) = scrolled {
            self.boxes[scrolled].sticky_scrolled.push(index);
        }
        self.effect_node(parent, Effect::Sticky(style, index))
    }
    /// Each effect's region on the pages it covers, from its box fragments and the structural
    /// areas standing in for boxes. Groups apply on every page without geometry.
    fn effect_regions(&self) -> Result<Vec<Vec<(usize, Region)>>, LayoutError> {
        let fragment = |&(page, slot): &(usize, usize)| match &self.display_lists[page][slot].item {
            DisplayItem::Box(paint) => Some((page, paint)),
            _ => None,
        };
        let mut regions = Vec::with_capacity(self.clips.len());
        for node in &self.clips {
            let mut node_regions = Vec::new();
            let clip = |paint: &BoxPaint| match node.effect {
                Effect::Clip(axes) => Some(paint.inner_clip([0.0; 4], axes)),
                Effect::BorderClip => Some(paint.background_clips().border),
                Effect::PaintClip(clip) => Some(clip),
                Effect::RectClip(style) => style.clip_region(paint.rect).map(|rect| Clip {
                    rect,
                    radii: crate::graphics::Radii::default(),
                    reference_box: None,
                    axes: [true; 2],
                }),
                Effect::Transform(..)
                | Effect::Sticky(..)
                | Effect::Parent3d(_)
                | Effect::Group(_) => None,
            };
            match node.effect {
                Effect::Clip(_)
                | Effect::BorderClip
                | Effect::PaintClip(_)
                | Effect::RectClip(_) => {
                    for (page, paint) in node.fragments.iter().filter_map(fragment) {
                        node_regions.extend(clip(paint).map(|clip| (page, Region::Clip(clip))));
                    }
                }
                Effect::Transform(style, parent) => {
                    let areas = node
                        .fragments
                        .iter()
                        .filter_map(|slot| {
                            fragment(slot).map(|(page, paint)| (page, paint.fragment_areas()))
                        })
                        .chain(
                            node.areas
                                .iter()
                                .map(|&(page, rect)| (page, BoxAreas::single(rect))),
                        );
                    for (page, areas) in areas {
                        let parent = parent
                            .map(|parent| self.parent_3d_on(&self.clips[parent], page))
                            .transpose()?
                            .flatten();
                        if let Some(matrix) = style.transform_matrix(&areas, parent.as_ref())? {
                            node_regions.push((page, Region::Transform(matrix)));
                        }
                    }
                }
                Effect::Sticky(style, index) => {
                    // An inline box's border box on a page bounds its fragments there and the
                    // blocks splitting it.
                    let mut borders: BTreeMap<usize, Rect> = BTreeMap::new();
                    for (page, rect) in node
                        .fragments
                        .iter()
                        .filter_map(fragment)
                        .map(|(page, paint)| (page, paint.rect))
                        .chain(node.areas.iter().copied())
                    {
                        borders
                            .entry(page)
                            .and_modify(|border| *border = border.union(rect))
                            .or_insert(rect);
                    }
                    for (page, border) in borders {
                        let [x, y] = self.sticky_shift(style, &self.sticky[index], page, border);
                        node_regions.push((page, Region::Transform([1.0, 0.0, 0.0, 1.0, x, y])));
                    }
                }
                Effect::Parent3d(_) | Effect::Group(_) => {}
            }
            regions.push(node_regions);
        }
        Ok(regions)
    }
    /// Each anchored group's element's box fragments: its block fragments and inline boxes.
    fn group_bounds(&self) -> Vec<Vec<(usize, BoxPaint)>> {
        self.clips
            .iter()
            .map(|node| match node.effect {
                Effect::Group(group) if group.anchored() => node
                    .fragments
                    .iter()
                    .filter_map(|&(page, slot)| match &self.display_lists[page][slot].item {
                        DisplayItem::Box(paint) => Some((page, paint.clone())),
                        _ => None,
                    })
                    .chain(node.paints.iter().cloned())
                    .collect(),
                _ => Vec::new(),
            })
            .collect()
    }
    /// The placement a `Parent3d` node gives its children on `page`, from its fragment there.
    fn parent_3d_on(
        &self,
        node: &EffectNode<'_>,
        page: usize,
    ) -> Result<Option<crate::style::Parent3d>, LayoutError> {
        let Effect::Parent3d(style) = node.effect else {
            unreachable!("transforms are placed by parent 3D nodes")
        };
        let paint = node.fragments.iter().find_map(|&(fragment_page, slot)| {
            match &self.display_lists[fragment_page][slot].item {
                DisplayItem::Box(paint) if fragment_page == page => Some(paint),
                _ => None,
            }
        });
        match paint {
            Some(paint) => Ok(style.parent_3d(&paint.fragment_areas())),
            None => Err(LayoutError::MissingContainingFragment { page }),
        }
    }
    /// Records a static position in `layer` and reserves the box's tree-order paint slot.
    fn pending(
        &self,
        block: &'a BlockBox,
        position: StaticPosition,
        layer: &Layer,
        parent_3d: Option<usize>,
    ) -> Pending<'a> {
        let order = self.grid_order(block).unwrap_or_else(|| self.order());
        Pending {
            block,
            position,
            context: layer.context.clone(),
            group: layer.group,
            parent_3d,
            orders: order.descendants(),
            order,
        }
    }
    /// Queues a positioned box anchored at `position` in `layer`.
    fn anchor(
        &mut self,
        block: &'a BlockBox,
        position: StaticPosition,
        layer: &Layer,
        parent_3d: Option<usize>,
    ) {
        let pending = self.pending(block, position, layer, parent_3d);
        self.enqueue(pending);
    }
    fn grid_order(&self, block: &BlockBox) -> Option<PaintOrder> {
        self.boxes
            .last()?
            .grid_orders
            .get(&std::ptr::from_ref(block))
            .cloned()
    }
    fn place_box_pending(
        &mut self,
        active: &mut ActiveBox<'a>,
        rtl: bool,
    ) -> Result<(), LayoutError> {
        let mut areas = Vec::new();
        if !active.pending.is_empty()
            && let Some(flow) = &active.geometry
            && let Some(table) = self.tables.last()
            && table.flow.plan.cell_clipped(flow.source)
            && let Some(index) = table.flow.plan.cell_index(flow.source)
        {
            for &(page, slot) in &active.fragments {
                let DisplayItem::Box(paint) = &self.display_lists[page][slot].item else {
                    return Err(LayoutError::MissingContainingFragment { page });
                };
                let mut rect = paint.inner_clip([0.0; 4], [true; 2]).rect;
                let resolved = self.table_geometry(page)?;
                let geometry = table.flow.plan.cell_geometry(index, &resolved);
                rect.x += geometry.x
                    - geometry.padding[3]
                    - geometry.border[3]
                    - table.flow.plan.cell_span(index, &resolved).0;
                rect.width = geometry.width + geometry.padding[1] + geometry.padding[3];
                if table.flow.plan.cell_rows_collapsed(index) {
                    let rows = table.flow.plan.resolve_rows(
                        self.measurer,
                        &table.flow.heights,
                        table.flow.spacing,
                        &resolved.geometry,
                    )?;
                    rect.height = (rows
                        .content_height(table.flow.plan.cell_rows(index), table.flow.spacing)
                        - geometry.border[0]
                        - geometry.border[2])
                        .max(0.0);
                }
                areas.push((page, rect));
            }
        }
        let containing = Containing::Box {
            grid: active.grid.clone(),
            overflow: active.overflow.clone().unwrap_or_default(),
            fragments: active.fragments.clone(),
            areas,
            rtl,
            clip: active
                .clip_node
                .or(active.transform_node)
                .or(active.sticky_node)
                .or(active.clip),
        };
        let pending = std::mem::take(&mut active.pending);
        let track_overflow = active.overflow.is_some() && !pending.is_empty();
        if track_overflow {
            // Positioned descendants contribute through their owner's clips and transforms.
            self.boxes.push(active.clone());
        }
        let result = pending
            .into_iter()
            .try_for_each(|pending| self.place(pending, &containing));
        if track_overflow {
            active.overflow = self.boxes.pop().unwrap().overflow;
        }
        result
    }
    /// Queues a positioned box with the innermost container that contains it.
    fn enqueue(&mut self, pending: Pending<'a>) {
        let fixed = pending.block.style().position == Position::Fixed;
        match self
            .containers
            .iter_mut()
            .rev()
            .find(|container| container.fixed || !fixed)
        {
            Some(container) => container.pending.push(pending),
            None => self.fixed.push(pending),
        }
    }
    /// Anchors a positioned box at its static position on the current line, which the inline
    /// boxes holding it offset, with the innermost of them that contains it.
    fn line_anchor(
        &mut self,
        anchor: &crate::layout::measure::LineAnchor<'a>,
        x: f32,
        width: f32,
        align_offset: f32,
        line_height: f32,
        layers: &InlineLayers<'a>,
    ) {
        let layer = self.inline_layer(layers, anchor.inline_box);
        let left = x + self.offset[0] + layer.offset[0];
        let top = self.top + self.y + self.offset[1] + layer.offset[1];
        let position = match anchor.x {
            Some(inline) => StaticPosition {
                page: self.page,
                left: left + align_offset + inline,
                right: left + align_offset + inline,
                y: top,
                alignment: None,
            },
            None => StaticPosition {
                page: self.page,
                left,
                right: left + width,
                y: if anchor.below { top + line_height } else { top },
                alignment: None,
            },
        };
        // An inline box, which is not transformable, is the parent of an anchor inside it.
        let parent_3d = self.parent_3d.filter(|_| anchor.inline_box.is_none());
        let fixed = anchor.block.style().position == Position::Fixed;
        let boxes = layers.paragraph.boxes();
        let containing = std::iter::successors(anchor.inline_box, |&index| boxes[index].parent)
            .find(|&index| {
                let style = &boxes[index].style;
                if fixed {
                    style.inline_contains_fixed()
                } else {
                    style.inline_contains_absolute()
                }
            });
        let Some(index) = containing else {
            self.anchor(anchor.block, position, &layer, parent_3d);
            return;
        };
        let pending = self.pending(anchor.block, position, &layer, parent_3d);
        let clip = self.inline_layer(layers, Some(index)).clip;
        self.inline_container(&boxes[index].style, clip)
            .pending
            .push(pending);
    }
    /// Lays out an absolutely positioned box on its anchor page without page breaks.
    fn place(
        &mut self,
        pending: Pending<'a>,
        containing: &Containing<'_>,
    ) -> Result<(), LayoutError> {
        let page = pending.position.page;
        let (cb, rtl, clip) = match containing {
            Containing::Rect { rect, rtl, clip } => (*rect, *rtl, *clip),
            Containing::Page => (
                match &self.fragmentainers {
                    Fragmentainers::Pages(pages) => pages.records[page].geometry.content_box(),
                    Fragmentainers::Columns(_) => self.canvas_rect,
                },
                false,
                None,
            ),
            Containing::Box {
                grid,
                overflow,
                fragments,
                areas,
                rtl,
                clip,
            } => {
                let (_, slot) = fragments
                    .iter()
                    .find(|(fragment_page, _)| *fragment_page == page)
                    .ok_or(LayoutError::MissingContainingFragment { page })?;
                let DisplayItem::Box(paint) = &self.display_lists[page][*slot].item else {
                    return Err(LayoutError::MissingContainingFragment { page });
                };
                let rect = areas
                    .iter()
                    .find_map(|&(area_page, rect)| (area_page == page).then_some(rect))
                    .unwrap_or_else(|| paint.inner_clip([0.0; 4], [true; 2]).rect);
                let rect = if let Some(grid) = grid {
                    grid.area(
                        rect,
                        paint.padding,
                        overflow.get(&page).copied(),
                        &pending.block.style().grid.placement,
                        self.measurer,
                    )?
                } else {
                    rect
                };
                (rect, *rtl, *clip)
            }
        };
        let (measured, top) = crate::layout::positioned::place(
            self.measurer,
            pending.block,
            cb,
            rtl,
            &pending.position,
        )?;
        self.lay_out_positioned(measured, top, page, clip, pending)
    }
    fn lay_out_positioned(
        &mut self,
        measured: MeasuredBlock<'a>,
        top: f32,
        page: usize,
        clip: Option<usize>,
        pending: Pending<'a>,
    ) -> Result<(), LayoutError> {
        let orders = std::mem::replace(&mut self.orders, pending.orders.snapshot());
        let result = self.unbroken_block(
            measured,
            Unbroken {
                page,
                y: top - self.fragmentainers.top(page),
                offset: [0.0; 2],
                clip,
                context: pending.context,
                group: pending.group,
                parent_3d: pending.parent_3d,
                key: self.key.clone(),
                reserved_order: Some(pending.order),
            },
        );
        self.orders = orders;
        result
    }
    /// Lays out `measured` at a fixed position without page breaks, then restores the flow.
    fn unbroken_block(
        &mut self,
        measured: MeasuredBlock<'a>,
        at: Unbroken,
    ) -> Result<(), LayoutError> {
        self.unbroken_at(at, |paginator| {
            paginator.block(measured, BreakTail::default()).map(|_| ())
        })
    }
    fn unbroken_at(
        &mut self,
        at: Unbroken,
        emit: impl FnOnce(&mut Self) -> Result<(), LayoutError>,
    ) -> Result<(), LayoutError> {
        let saved_page = self.page;
        let saved_y = self.y;
        let saved_pending = std::mem::take(&mut self.pending);
        let saved_flow = (
            self.has_flow,
            self.at_page_start,
            self.margin_start,
            self.page_content,
        );
        let saved_offset = self.offset;
        let saved_clip = self.clip;
        let saved_key = std::mem::replace(&mut self.key, at.key);
        let saved_context = std::mem::replace(&mut self.context, at.context);
        let saved_group = std::mem::replace(&mut self.group, at.group);
        let saved_parent_3d = std::mem::replace(&mut self.parent_3d, at.parent_3d);
        let saved_break = self.pending_break;
        let saved_reserve = std::mem::take(&mut self.bottom_reserve);
        self.unbroken += 1;
        self.enter(at.page);
        self.y = at.y;
        self.offset = at.offset;
        self.clip = at.clip;
        self.has_flow = true;
        self.at_page_start = false;
        self.pending_break = None;
        self.reserved_order = at.reserved_order;
        let result = emit(self);
        self.enter(saved_page);
        self.unbroken -= 1;
        self.y = saved_y;
        self.pending = saved_pending;
        (
            self.has_flow,
            self.at_page_start,
            self.margin_start,
            self.page_content,
        ) = saved_flow;
        self.offset = saved_offset;
        self.clip = saved_clip;
        self.key = saved_key;
        self.context = saved_context;
        self.group = saved_group;
        self.parent_3d = saved_parent_3d;
        self.pending_break = saved_break;
        self.bottom_reserve = saved_reserve;
        result
    }
    /// Tree-order paint key for inline-level content of the current flow (CSS 2.1 Appendix E
    /// step 7): later lines, inline box fragments and atomic inlines paint above earlier ones.
    fn inline_key(&mut self) -> PaintKey {
        let order = self.order();
        extend(&self.key, (0, order))
    }
    fn select_page(&mut self, page: usize, y: f32) {
        self.enter(page);
        self.y = y;
        self.pending.clear();
        self.has_flow = y > 0.0;
        self.at_page_start = y == 0.0;
        self.margin_start = if page == 0 {
            MarginStart::Initial
        } else {
            MarginStart::Unforced
        };
        self.page_content = self.current_page_has_ink();
        for active in &mut self.boxes {
            if let Some((paint, slot)) = &mut active.paint
                && let Ok(index) = active
                    .fragments
                    .binary_search_by_key(&page, |(index, _)| *index)
            {
                *slot = active.fragments[index].1;
                if let DisplayItem::Box(saved) = &self.display_lists[page][*slot].item {
                    *paint = saved.clone();
                }
            }
        }
    }
    fn record_collapsed(&mut self, source: &'a BlockBox) -> Result<(), LayoutError> {
        let mut blocks = vec![source];
        let mut events = Vec::new();
        while let Some(source) = blocks.pop() {
            self.measurer.work.spend(1)?;
            events.extend(source.semantic_event());
            for child in source.children() {
                match child {
                    crate::document::BlockContent::Events(values) => events.extend(values),
                    crate::document::BlockContent::Block(child)
                    | crate::document::BlockContent::Positioned(child) => blocks.push(child),
                    crate::document::BlockContent::Inline(paragraph) => {
                        events.extend(paragraph.semantic_events().iter().map(|event| &event.event));
                        blocks.extend(
                            paragraph
                                .anchors()
                                .iter()
                                .chain(paragraph.atomics())
                                .map(|anchor| &anchor.block),
                        );
                    }
                }
            }
        }
        events.sort_by_key(|event| event.order.start);
        for event in events {
            let mut event = event.clone();
            event.visible = false;
            self.string_locations.record(self.page, &event);
        }

        Ok(())
    }
    fn finish_height(
        &mut self,
        start_page: usize,
        start_y: f32,
        height: f32,
    ) -> Result<(), LayoutError> {
        self.finish_extent(start_page, start_y, |_, _, _| Ok(height))
    }
    fn start_extent(page: usize, origin: f32, reserve_end: bool) -> ExtentCursor {
        ExtentCursor {
            reserve_end,
            page,
            origin,
            remaining: 1.0,
            advance: None,
        }
    }
    fn finish_extent(
        &mut self,
        start_page: usize,
        start_y: f32,
        mut resolve: impl FnMut(&mut Self, usize, &ExtentSpace) -> Result<f32, LayoutError>,
    ) -> Result<(), LayoutError> {
        // Structural row extents continue independently of the enclosing sliced ends.
        let mut cursor = Self::start_extent(start_page, start_y, false);
        while let Some(request) = self.resume_extent(&mut cursor, &mut resolve)? {
            self.new_page(request.forced, request.preserve_empty)?;
        }
        Ok(())
    }
    fn extent_space(&self, page: usize) -> Result<ExtentSpace, LayoutError> {
        let mut space = ExtentSpace {
            start: self.repeated_start(page, self.boxes.len()),
            cloned_end: self.cloned_end_space(),
            bottom_reserve: self.bottom_reserve,
            height: None,
        };
        if !self.continuation_flow() {
            return Ok(space);
        }
        self.measurer.work.spend(self.boxes.len().max(1))?;
        let path = self.flow_geometry(page, self.fragmentainers.content_box(page), None)?;
        space.height = path
            .last()
            .and_then(Option::as_ref)
            .filter(|flow| flow.normal)
            .map(|flow| flow.height);
        for (active, resolved) in self.boxes.iter().zip(path) {
            let Some(resolved) = resolved else {
                continue;
            };
            let previous = active
                .geometry
                .as_ref()
                .expect("resolved boxes retain their source geometry");
            if !resolved.normal && previous.horizontal == resolved.horizontal {
                continue;
            }
            let end_delta = resolved.end_space(active.clone) - previous.end_space(active.clone);
            if active.clone {
                space.start += resolved.start_space() - previous.start_space();
                space.cloned_end += end_delta;
            }
            if active.reserves_end {
                space.bottom_reserve += end_delta;
            }
        }
        Ok(space)
    }

    fn resume_extent(
        &mut self,
        cursor: &mut ExtentCursor,
        mut resolve: impl FnMut(&mut Self, usize, &ExtentSpace) -> Result<f32, LayoutError>,
    ) -> Result<Option<BreakRequest>, LayoutError> {
        if self.unbroken > 0 {
            self.y = cursor.origin + resolve(self, cursor.page, &self.extent_space(cursor.page)?)?;
            self.pending.clear();
            return Ok(None);
        }
        let mut overflow_start = cursor.advance.take();
        if overflow_start.is_some() {
            cursor.page = self.page;
            self.ensure_band_pages(cursor.page)?;
        }
        loop {
            self.measurer.work.spend(1)?;
            let mut space = self.extent_space(cursor.page)?;
            if !cursor.reserve_end {
                space.bottom_reserve = space.cloned_end;
            }
            if let Some(overflow) = overflow_start.take() {
                cursor.origin = space.start + overflow;
            }
            let extent = resolve(self, cursor.page, &space)?;
            let end_y = cursor.origin + (f64::from(extent) * cursor.remaining) as f32;
            if !overflows(
                end_y,
                self.fragmentainers.height(cursor.page)
                    - space.bottom_reserve
                    - self.band_footer_height(cursor.page),
            ) {
                if self.page > cursor.page {
                    self.select_page(cursor.page, end_y);
                } else {
                    self.y = end_y;
                    self.pending.clear();
                    if end_y > space.start {
                        self.has_flow = true;
                        self.at_page_start = false;
                    }
                }
                return Ok(None);
            }
            let end = self.fragmentainers.height(cursor.page)
                - space.cloned_end
                - self.band_footer_height(cursor.page);
            if cursor.page + 1 >= MAX_LAYOUT_PAGES
                || (end <= space.start
                    && !matches!(&self.fragmentainers, Fragmentainers::Columns(rows) if rows.has_next_row(cursor.page)))
            {
                return Err(LayoutError::PageLimit {
                    limit: MAX_LAYOUT_PAGES,
                });
            }
            if extent > 0.0 {
                cursor.remaining = (cursor.remaining
                    - f64::from((end - cursor.origin).max(0.0)) / f64::from(extent))
                .max(0.0);
            }
            let overflow = (cursor.origin - end).max(0.0);
            if cursor.page >= self.page {
                self.y = end;
                cursor.advance = Some(overflow);
                return Ok(Some(BreakRequest {
                    forced: None,
                    preserve_empty: true,
                }));
            }
            cursor.page += 1;
            self.ensure_band_pages(cursor.page)?;
            overflow_start = Some(overflow);
        }
    }
    fn space(&mut self, value: f32) {
        self.flush_margin();
        self.y += value;
        self.at_page_start = false;
    }
    fn refresh_table(&mut self) -> Result<(), LayoutError> {
        if !self.continuation_flow() {
            return Ok(());
        }
        let Some(table) = self.tables.last() else {
            return Ok(());
        };
        let source = table.flow.plan.source;
        let Some(first) = self.boxes.iter().position(|active| {
            active
                .geometry
                .as_ref()
                .is_some_and(|flow| std::ptr::eq(flow.source, source))
        }) else {
            return Ok(());
        };
        let geometry = self.table_geometry(self.page)?;
        if geometry.geometry == table.flow.geometry {
            return Ok(());
        }
        let mut measured =
            crate::layout::tables::resume(self.measurer, &table.flow, &geometry, table.next_body)?;
        let flow = measured
            .table
            .take()
            .expect("table measurement retains its plan");
        let table = self.tables.last_mut().unwrap();
        table.rows = measured
            .items
            .into_iter()
            .filter_map(|item| match item {
                FlowItem::Row(row) => Some((row.band.as_ref().unwrap().start, *row)),
                _ => None,
            })
            .collect();
        if let Some(grid) = &flow.collapsed {
            table.grids.insert(self.page, grid.clone());
        }
        table.flow = flow;
        let geometry = &measured.geometry;
        let inner = self
            .boxes
            .iter()
            .rposition(|active| {
                active
                    .geometry
                    .as_ref()
                    .is_some_and(|flow| std::ptr::eq(flow.source, source))
            })
            .expect("active table retains its inner box");
        for (index, active) in self.boxes.iter_mut().enumerate().skip(first) {
            let Some(current) = &mut active.geometry else {
                continue;
            };
            if !std::ptr::eq(current.source, source) {
                continue;
            }
            current.horizontal = geometry.clone();
            if index == inner {
                if active.reserves_end {
                    self.bottom_reserve += measured.end_space - active.end_space;
                }
                if self.at_page_start {
                    self.y += measured.repeat_space - active.repeat_space;
                    if active.clone {
                        self.y += measured.start_space - active.start_space;
                    }
                }
                active.start_space = measured.start_space;
                active.end_space = measured.end_space;
                active.repeat_space = measured.repeat_space;
                if let Some((paint, slot)) = &mut active.paint {
                    if let Some(replacement) = &measured.paint {
                        paint.rect.x = replacement.rect.x + active.offset[0];
                        paint.rect.width = replacement.rect.width;
                        paint.padding = replacement.padding;
                        paint.decoration = replacement.decoration.clone();
                    }
                    self.display_lists[self.page][*slot].item = DisplayItem::Box(paint.clone());
                }
            } else {
                current.horizontal.x -= geometry.padding[3] + geometry.border[3];
                current.horizontal.width += geometry.padding[1]
                    + geometry.padding[3]
                    + geometry.border[1]
                    + geometry.border[3];
                current.horizontal.padding = [0.0; 4];
                current.horizontal.border = [0.0; 4];
            }
        }
        Ok(())
    }
    fn current_table_row(&mut self, row: MeasuredRow<'a>) -> MeasuredRow<'a> {
        row.band
            .as_ref()
            .and_then(|band| {
                self.tables
                    .last_mut()
                    .and_then(|table| table.rows.remove(&band.start))
            })
            .unwrap_or(row)
    }
    fn prepare_row(
        &mut self,
        row: MeasuredRow<'a>,
        tail: BreakTail,
    ) -> Result<(RowEntry<'a>, Option<BreakRequest>), LayoutError> {
        if row.band.is_some() {
            self.refresh_table()?;
        }
        let row = self.current_table_row(row);
        let page_name = row.band.as_ref().map(|band| {
            self.tables
                .last()
                .expect("table bands belong to an active table")
                .flow
                .plan
                .row_source(band.start)
                .page_boundary()
                .start
                .clone()
        });
        let changed = page_name.is_some_and(|name| self.page_name(name));
        // Header and body bands leave room for a repeated footer; only body bands repeat groups.
        let (footer_reserve, body) = match (&row.band, self.tables.last()) {
            (Some(band), Some(table)) if band.end <= table.flow.body.end => (
                table.flow.footer_height,
                band.start >= table.flow.body.start,
            ),
            _ => (0.0, false),
        };
        let forced = Forced::combine(
            Forced::combine(self.pending_break, Forced::of(row.break_before)),
            changed.then_some(Forced::Page),
        );
        let reserve = self.continuation_reserve()?;
        let reserve = if row.first_fragment >= row.height {
            self.closing_reserve(tail, reserve)?
        } else {
            reserve
        };
        let required = row.first_fragment + reserve + footer_reserve;
        let request = if self.unbroken == 0
            && self.has_flow
            && (forced.is_some()
                || (((overflows(row.height, self.height) && row.indivisible())
                    || !overflows(required, self.height))
                    && overflows(self.y + self.pending_value() + required, self.height)))
        {
            if body {
                self.repeat_group(|flow| &flow.footer)?;
            }
            Some(BreakRequest {
                forced,
                preserve_empty: false,
            })
        } else {
            None
        };
        Ok((
            RowEntry {
                row,
                tail,
                body,
                after_break: request.is_some(),
            },
            request,
        ))
    }
    fn start_row(&mut self, entry: RowEntry<'a>) -> Result<rows::RowCursor<'a, 't>, LayoutError> {
        let RowEntry {
            mut row,
            tail,
            body,
            after_break,
        } = entry;
        if after_break {
            if row.band.is_some() {
                self.refresh_table()?;
                row = self.current_table_row(row);
            }
            if body {
                self.repeat_group(|flow| &flow.header)?;
            }
        }
        let end = row.band.as_ref().filter(|_| body).map(|band| band.end);
        Ok(self.start_parallel_row(row, tail, end))
    }
    /// Places copies of a repeated header or footer group at the current position.
    fn repeat_group(
        &mut self,
        group: impl for<'f> Fn(&'f TableFlow<'a>) -> &'f [MeasuredRow<'a>],
    ) -> Result<(), LayoutError> {
        let table = self
            .tables
            .last()
            .expect("repeated groups belong to a table");
        let rows = group(&table.flow).to_vec();
        let spacing = table.flow.spacing;
        for row in rows {
            self.place_row(row)?;
            self.space(spacing);
        }
        self.pending_break = None;
        Ok(())
    }
    fn place_row(&mut self, row: MeasuredRow<'a>) -> Result<(), LayoutError> {
        let mut cursor = self.start_parallel_row(row, BreakTail::default(), None);
        while let Some(request) = self.resume_row(&mut cursor)? {
            self.new_page(request.forced, request.preserve_empty)?;
        }
        self.finish_row(cursor)
    }
    fn table_geometry(
        &self,
        page: usize,
    ) -> Result<crate::layout::tables::TableGeometry, LayoutError> {
        self.table_geometry_at(self.tables.len() - 1, page)
    }
    fn table_geometry_at(
        &self,
        table_index: usize,
        page: usize,
    ) -> Result<crate::layout::tables::TableGeometry, LayoutError> {
        let table = &self.tables[table_index];
        let floating = table.flow.plan.source.style().floating().is_some();
        let path = if page == self.page && floating {
            self.boxes
                .iter()
                .map(|active| active.geometry.clone())
                .collect()
        } else {
            self.flow_geometry(page, self.fragmentainers.content_box(page), None)?
        };
        let flow = path
            .iter()
            .flatten()
            .rev()
            .find(|flow| std::ptr::eq(flow.source, table.flow.plan.source))
            .expect("table has active geometry");
        let mut resolved = if flow.grid_item {
            table
                .flow
                .plan
                .resolve_positioned(self.measurer, &flow.horizontal)?
        } else {
            table.flow.plan.resolve_geometry(
                self.measurer,
                flow.horizontal.containing_x,
                flow.horizontal.containing_width,
                flow.horizontal.containing_rtl,
            )?
        };
        resolved.containing_height = flow.containing_height;
        if floating {
            resolved.geometry.clone_from(&flow.horizontal);
        }
        Ok(resolved)
    }

    fn band_footer_height(&self, page: usize) -> f32 {
        if self.unbroken > 0 {
            return 0.0;
        }
        self.bands
            .iter()
            .filter_map(|band| band.pages.get(&page))
            .map(|page| page.footer_height)
            .sum()
    }
    fn continuation_reserve(&self) -> Result<f32, LayoutError> {
        let footer = self.band_footer_height(self.page);
        // Border and padding ends are nonnegative, so zero excludes cloned ends too.
        if self.bottom_reserve == 0.0 {
            return Ok(footer);
        }
        self.measurer.work.spend(self.boxes.len().max(1))?;
        Ok(self.cloned_end_space() + footer)
    }
    fn closing_reserve(&self, tail: BreakTail, reserve: f32) -> Result<f32, LayoutError> {
        if tail.end_boxes == 0 {
            return Ok(tail.height + reserve);
        }
        if self.bottom_reserve == 0.0 {
            return Ok(tail.height - tail.end_space + reserve);
        }
        self.measurer.work.spend(self.boxes.len().max(1))?;
        let mut end_space = 0.0;
        let mut cloned_space = 0.0;
        for active in self
            .boxes
            .iter()
            .rev()
            .filter(|active| active.reserves_end && active.geometry.is_some())
            .take(tail.end_boxes)
        {
            end_space += active.end_space;
            if active.clone {
                cloned_space += active.end_space;
            }
        }
        // Only cloned closing edges already occupy the continuation reserve.
        Ok(tail.height + end_space - tail.end_space + (reserve - cloned_space).max(0.0))
    }
    fn repeated_start(&self, page: usize, boxes: usize) -> f32 {
        let edges: f32 = self.boxes[..boxes]
            .iter()
            .map(|active| {
                (if active.clone {
                    active.start_space
                } else {
                    0.0
                }) + active.repeat_space
            })
            .sum();
        edges
            + self
                .bands
                .iter()
                .filter(|band| band.boxes <= boxes)
                .filter_map(|band| band.pages.get(&page))
                .map(|page| page.header_height)
                .sum::<f32>()
    }
    fn ensure_band_pages(&mut self, page: usize) -> Result<(), LayoutError> {
        for index in 0..self.bands.len() {
            if self.bands[index].pages.contains_key(&page) {
                continue;
            }
            let band = &self.bands[index];
            let geometry = self.table_geometry_at(band.table, page)?;
            let table = &self.tables[band.table];
            let old_height = self.measurer.page_height;
            self.measurer.page_height = self.fragmentainers.height(page);
            let measured = crate::layout::tables::resume(
                self.measurer,
                &table.flow,
                &geometry,
                table.flow.body.end,
            );
            self.measurer.page_height = old_height;
            let mut flow = measured?
                .table
                .expect("table group measurement retains its flow");
            let header_start = self.repeated_start(page, band.boxes);
            let footer_parent: f32 = self.boxes[..band.boxes]
                .iter()
                .filter(|active| active.clone)
                .map(|active| active.end_space)
                .sum();
            let outer_footer = self.band_footer_height(page);
            let group_height = |rows: &[MeasuredRow<'a>]| {
                rows.iter()
                    .map(|row| row.height + flow.spacing)
                    .sum::<f32>()
            };
            let mut header_height = group_height(&flow.header);
            let mut footer_height = group_height(&flow.footer);
            if overflows(
                header_start
                    + header_height
                    + footer_height
                    + footer_parent
                    + outer_footer
                    + band.minimum,
                self.fragmentainers.height(page),
            ) {
                flow.header.clear();
                flow.footer.clear();
                header_height = 0.0;
                footer_height = 0.0;
            }
            if let Some(grid) = flow.collapsed.take() {
                self.tables[band.table].grids.insert(page, grid);
            }
            self.bands[index].pages.insert(
                page,
                BandPage {
                    header: flow.header,
                    footer: flow.footer,
                    header_start,
                    header_height,
                    body_start: header_start + header_height,
                    footer_height,
                    footer_start: self.fragmentainers.height(page)
                        - footer_parent
                        - outer_footer
                        - footer_height,
                    spacing: flow.spacing,
                },
            );
        }
        Ok(())
    }
    fn emit_band_groups(
        &mut self,
        band: BandContinuation<'a>,
        last_body: bool,
        break_after: Option<Forced>,
    ) -> Result<(), LayoutError> {
        let last_page = self.page;
        for (page, groups) in band.pages {
            let separate_footer = last_body
                && (break_after.is_some()
                    || groups
                        .footer
                        .first()
                        .is_some_and(|row| row.break_before.is_forced()));
            for (emit, y, rows) in [
                (page != band.first_page, groups.header_start, groups.header),
                (
                    page != last_page || separate_footer,
                    groups.footer_start,
                    groups.footer,
                ),
            ] {
                if !emit || rows.is_empty() {
                    continue;
                }
                let at = Unbroken {
                    page,
                    y,
                    offset: self.offset,
                    clip: self.clip,
                    context: self.context.clone(),
                    group: self.group,
                    parent_3d: self.parent_3d,
                    key: self.key.clone(),
                    reserved_order: None,
                };
                self.unbroken_at(at, |paginator| {
                    for row in rows {
                        paginator.place_row(row)?;
                        paginator.space(groups.spacing);
                    }
                    Ok(())
                })?;
            }
        }
        Ok(())
    }
    fn table_fragment(
        &self,
        ancestor: &crate::layout::tables::TableContaining<'a>,
        geometry: &crate::layout::tables::TableGeometry,
        indices: &[usize],
        starts: &[f32],
        project_missing: bool,
    ) -> Result<Option<Rect>, LayoutError> {
        let table = self.tables.last().expect("active table");
        let first = indices
            .partition_point(|index| table.placements[*index].1.rows.end <= ancestor.rows.start);
        let mut rect: Option<Rect> = None;
        for index in indices[first..]
            .iter()
            .take_while(|index| table.placements[**index].1.rows.start < ancestor.rows.end)
        {
            self.measurer.work.spend(1)?;
            let placement = &table.placements[*index].1;
            if let Some(fragment) = ancestor.rect(
                geometry,
                placement,
                if placement.slice.is_some() {
                    starts
                } else {
                    &table.flow.row_starts
                },
                table.flow.plan.source.style().rtl,
                false,
            ) {
                rect = Some(rect.map_or(fragment, |area| area.union(fragment)));
            }
        }
        // A spanning cell can outlive its originating row's physical fragment.
        if project_missing
            && rect.is_none()
            && let Some(index) = indices.first()
        {
            let placement = &table.placements[*index].1;
            rect = ancestor.rect(
                geometry,
                placement,
                if placement.slice.is_some() {
                    starts
                } else {
                    &table.flow.row_starts
                },
                table.flow.plan.source.style().rtl,
                true,
            );
        }
        Ok(rect.map(|mut rect| {
            rect.x += table.x_offset;
            rect
        }))
    }
    fn paint_table_shadows(
        &mut self,
        shadows: &[crate::layout::tables::TableContaining<'a>],
        pages: &BTreeMap<usize, Vec<usize>>,
        starts: &[f32],
    ) -> Result<(), LayoutError> {
        let mut owners: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (index, ancestor) in shadows.iter().enumerate() {
            self.measurer.work.spend(ancestor.rows.len().max(1))?;
            for row in ancestor.rows.clone() {
                owners.entry(row).or_default().push(index);
            }
        }
        let mut fragments = vec![[Vec::new(), Vec::new()]; shadows.len()];
        for (&page, indices) in pages {
            let mut active = BTreeSet::new();
            let table = self.tables.last().expect("active table");
            for &index in indices {
                let rows = &table.placements[index].1.rows;
                for (_, sources) in owners.range(rows.clone()) {
                    self.measurer.work.spend(sources.len().max(1))?;
                    active.extend(sources.iter().copied());
                }
            }
            if active.is_empty() {
                continue;
            }
            let geometry = self.table_geometry(page)?;
            for index in active {
                let ancestor = &shadows[index];
                let Some(rect) =
                    self.table_fragment(ancestor, &geometry, indices, starts, false)?
                else {
                    continue;
                };
                let table = self.tables.last().expect("active table");
                let layer = table.layers[&(std::ptr::from_ref(ancestor.source) as usize)].clone();
                // The outline paints from an otherwise empty box after the group's content.
                if let Some(decoration) = table.flow.plan.structural_outline(ancestor.source) {
                    let order = self.orders.take();
                    self.display_lists[page].push(ScopedPaint {
                        visible: ancestor.source.style().visibility.is_visible(),
                        scope: layer.paint.background.clone(),
                        order,
                        clip: layer.clip,
                        group: layer.group,
                        ink: None,
                        item: DisplayItem::Box(BoxPaint::whole(rect, decoration)),
                        navigation: BoxNavigation::default(),
                    });
                }
                let decorations = [false, true]
                    .map(|inset| table.flow.plan.structural_shadow(ancestor.source, inset));
                for (inset, decoration) in [false, true].into_iter().zip(decorations) {
                    let Some(decoration) = decoration else {
                        continue;
                    };
                    let order = if inset {
                        self.orders.take()
                    } else {
                        PaintOrder::FIRST
                    };
                    fragments[index][usize::from(inset)]
                        .push((page, self.display_lists[page].len()));
                    self.display_lists[page].push(ScopedPaint {
                        visible: ancestor.source.style().visibility.is_visible(),
                        scope: layer.paint.background.clone(),
                        order,
                        clip: layer.clip,
                        group: layer.group,
                        ink: None,
                        item: DisplayItem::Box(BoxPaint::whole(rect, decoration)),
                        navigation: BoxNavigation::default(),
                    });
                }
            }
        }
        for (ancestor, fragments) in shadows.iter().zip(fragments) {
            if !ancestor.source.style().clone_decoration
                && !self
                    .tables
                    .last()
                    .expect("active table")
                    .repeats(&ancestor.rows)
            {
                for fragments in fragments {
                    for (index, &(page, slot)) in fragments.iter().enumerate() {
                        if let DisplayItem::Box(paint) = &mut self.display_lists[page][slot].item {
                            paint.first_fragment = index == 0;
                            paint.last_fragment = index + 1 == fragments.len();
                        }
                    }
                    assemble_background(&fragments, &mut self.display_lists, false, false);
                }
            }
        }
        Ok(())
    }
    fn place_table_pending(&mut self, starts: &[f32]) -> Result<(), LayoutError> {
        let table = self.tables.last_mut().expect("active table");
        let groups = std::mem::take(&mut table.positioned);
        let transforms = std::mem::take(&mut table.transforms);
        let shadows = std::mem::take(&mut table.shadows);
        if groups.is_empty() && transforms.is_empty() && shadows.is_empty() {
            return Ok(());
        }
        let mut pages: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let table = self.tables.last().expect("active table");
        for (index, (page, _)) in table.placements.iter().enumerate() {
            pages.entry(*page).or_default().push(index);
        }
        for indices in pages.values_mut() {
            indices.sort_by_key(|index| {
                let rows = &table.placements[*index].1.rows;
                (rows.start, rows.end)
            });
        }
        let mut rectangles: HashMap<(*const BlockBox, usize), Rect> = HashMap::new();
        let transform_indices: HashMap<_, _> = transforms
            .iter()
            .enumerate()
            .map(|(index, (ancestor, _))| (std::ptr::from_ref(ancestor.source), index))
            .collect();
        let mut transformed_pages: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        if !transforms.is_empty() {
            for &(cell, page) in table.cell_fragments.keys() {
                self.measurer.work.spend(1)?;
                for source in table.flow.plan.cell_parents(cell).into_iter().flatten() {
                    if let Some(&index) = transform_indices.get(&std::ptr::from_ref(source)) {
                        transformed_pages.entry(page).or_default().insert(index);
                    }
                }
            }
        }
        for (page, active) in transformed_pages {
            let geometry = self.table_geometry(page)?;
            let indices = pages
                .get(&page)
                .ok_or(LayoutError::MissingContainingFragment { page })?;
            for index in active {
                let (ancestor, node) = &transforms[index];
                if let Some(rect) =
                    self.table_fragment(ancestor, &geometry, indices, starts, true)?
                {
                    rectangles.insert((std::ptr::from_ref(ancestor.source), page), rect);
                    self.clips[*node].areas.push((page, rect));
                }
            }
        }
        let table = self.tables.last_mut().expect("active table");
        let cell_overflow = std::mem::take(&mut table.cell_overflow);
        for ((cell, page), mut area) in cell_overflow {
            let table = self.tables.last().expect("active table");
            for source in table
                .flow
                .plan
                .cell_parents(cell)
                .into_iter()
                .rev()
                .flatten()
            {
                if let Some(rect) = rectangles.get(&(std::ptr::from_ref(source), page))
                    && let Some(matrix) = source
                        .style()
                        .transform_matrix(&BoxAreas::single(*rect), None)?
                {
                    area = overflow::transformed_bounds(area, matrix);
                }
            }
            self.record_overflow_on(page, area);
        }
        self.paint_table_shadows(&shadows, &pages, starts)?;
        let mut geometry: Option<(usize, crate::layout::tables::TableGeometry)> = None;
        for (ancestor, pending) in groups {
            for pending in pending {
                let page = pending.position.page;
                let key = (std::ptr::from_ref(ancestor.source), page);
                let rect = if let Some(rect) = rectangles.get(&key) {
                    *rect
                } else {
                    if geometry.as_ref().is_none_or(|(cached, _)| *cached != page) {
                        geometry = Some((page, self.table_geometry(page)?));
                    }
                    let indices = pages
                        .get(&page)
                        .ok_or(LayoutError::MissingContainingFragment { page })?;
                    let rect = self
                        .table_fragment(
                            &ancestor,
                            &geometry.as_ref().unwrap().1,
                            indices,
                            starts,
                            true,
                        )?
                        .ok_or(LayoutError::MissingContainingFragment { page })?;
                    rectangles.insert(key, rect);
                    rect
                };
                let clip = self
                    .tables
                    .last()
                    .expect("active table")
                    .layers
                    .get(&(key.0 as usize))
                    .map_or(self.clip, |layer| layer.clip);
                self.place(
                    pending,
                    &Containing::Rect {
                        rect,
                        rtl: ancestor.source.style().rtl,
                        clip,
                    },
                )?;
            }
        }
        Ok(())
    }
    fn table_backgrounds(&mut self, table: &ActiveTable<'a>, starts: &[f32]) {
        for layer in &table.backgrounds {
            if layer.repeated {
                continue;
            }
            let item = match layer.underlay {
                Some(index) => {
                    &mut self
                        .underlays
                        .get_mut(&(layer.page, layer.slot))
                        .expect("registered table underlay")[index]
                        .item
                }
                None => &mut self.display_lists[layer.page][layer.slot].item,
            };
            let DisplayItem::Box(paint) = item else {
                unreachable!("table backgrounds are box paints")
            };
            let area = paint
                .background_box
                .as_mut()
                .expect("table background positioning area");
            area.y = paint.rect.y + starts[layer.rows.rows.start]
                - starts[layer.rows.cell_row]
                - layer.before;
            area.height =
                starts[layer.rows.rows.end] - starts[layer.rows.rows.start] - table.flow.spacing;
        }
    }
    /// Paints a collapsed table's borders for the bands placed on each page.
    fn collapsed_borders(
        &mut self,
        mut table: ActiveTable<'a>,
        starts: &[f32],
    ) -> Result<(), LayoutError> {
        table
            .placements
            .sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.y.total_cmp(&b.1.y)));
        let mut placements = std::mem::take(&mut table.placements).into_iter().peekable();
        while let Some((page, first)) = placements.next() {
            let mut bands = vec![first];
            while let Some((_, band)) = placements.next_if(|(next, _)| *next == page) {
                bands.push(band);
            }
            let Some((_, grid)) = table.grids.range(..=page).next_back() else {
                continue;
            };
            if table.flow.plan.scoped_borders() {
                let width = grid
                    .lines
                    .last()
                    .zip(grid.lines.first())
                    .map_or(0.0, |(end, start)| (end - start).abs());
                let paints = grid.positioned_paints(
                    &bands,
                    table.x_offset,
                    starts,
                    &self.measurer.work,
                    |row, column| table.flow.plan.border_owner(row, column, width),
                )?;
                let mut clips = HashMap::new();
                for border in paints {
                    let source = border
                        .cell
                        .and_then(|cell| table.cell_fragments.get(&(cell, page)))
                        .map(|slot| &self.display_lists[page][*slot]);
                    let visible = !border
                        .cell
                        .is_some_and(|cell| table.flow.plan.cell_collapsed(cell))
                        && source.map_or_else(
                            || table.flow.plan.source.style().visibility.is_visible(),
                            |source| source.visible,
                        );
                    let (scope, order, parent, group) = source.map_or_else(
                        || (self.key.clone(), self.orders.take(), self.clip, self.group),
                        |source| {
                            (
                                source.scope.clone(),
                                source.order.clone(),
                                source.clip,
                                source.group,
                            )
                        },
                    );
                    let rect = border.clip.rect;
                    let key = (
                        border.cell,
                        [
                            rect.x.to_bits(),
                            rect.y.to_bits(),
                            rect.width.to_bits(),
                            rect.height.to_bits(),
                        ],
                        border
                            .clip
                            .bounds()
                            .edges()
                            .map(|(axis, value, lower)| (axis, value.to_bits(), lower))
                            .collect::<Vec<_>>(),
                    );
                    let clip = *clips.entry(key).or_insert_with(|| {
                        let slot = self.display_lists[page].len();
                        self.display_lists[page].push(ScopedPaint {
                            visible: true,
                            order: order.clone(),
                            scope: scope.clone(),
                            clip: parent,
                            group,
                            ink: None,
                            item: DisplayItem::Box(BoxPaint::whole(rect, BoxDecoration::default())),
                            navigation: BoxNavigation::default(),
                        });
                        let clip = self.clips.len();
                        self.clips.push(EffectNode {
                            paints: Vec::new(),
                            areas: Vec::new(),
                            parent,
                            effect: Effect::PaintClip(border.clip),
                            fragments: vec![(page, slot)],
                            context: None,
                        });
                        clip
                    });
                    self.display_lists[page].push(ScopedPaint {
                        visible,
                        order,
                        scope,
                        clip: Some(clip),
                        group,
                        ink: None,
                        item: DisplayItem::Box(border.paint),
                        navigation: BoxNavigation::default(),
                    });
                }
                continue;
            }
            for paint in grid.paints(&bands, table.x_offset, starts) {
                if paint.has_ink() {
                    self.display_lists[page].push(ScopedPaint {
                        visible: true,
                        order: self.orders.take(),
                        scope: self.key.clone(),
                        clip: self.clip,
                        group: self.group,
                        ink: None,
                        item: DisplayItem::Box(paint),
                        navigation: BoxNavigation::default(),
                    });
                }
            }
        }
        Ok(())
    }
    fn stretch_box(
        &mut self,
        mut active: ActiveBox<'a>,
        after: f32,
        paint_start: Option<(usize, f32)>,
        backgrounds: &[crate::layout::tables::FragmentBackground<'a>],
        fragment_ends: &BTreeMap<usize, f32>,
    ) -> Result<Vec<(usize, Rect)>, LayoutError> {
        let Some((mut paint, mut slot)) = active.paint.take() else {
            return Ok(Vec::new());
        };
        let Some(&(last_page, _)) = active.fragments.last() else {
            return Ok(Vec::new());
        };
        let first_page = paint_start.map_or(last_page, |start| start.0);
        let mut layer_fragments = Vec::new();
        for page in first_page..=self.page {
            if matches!(&self.fragmentainers, Fragmentainers::Pages(pages) if pages.records[page].request.blank)
            {
                continue;
            }
            let existing = active
                .fragments
                .binary_search_by_key(&page, |(page, _)| *page)
                .ok();
            if let Some(index) = existing {
                slot = active.fragments[index].1;
                if let DisplayItem::Box(saved) = &self.display_lists[page][slot].item {
                    paint = saved.clone();
                }
            } else {
                paint.rect.height = 0.0;
            }
            let top = self.fragmentainers.top(page);
            let mut offset = active.offset;
            let mut reserve = self.bottom_reserve + self.band_footer_height(page);
            if let Some(flow) = &active.geometry
                && flow.source.style().display == crate::style::Display::TableCell
            {
                let path =
                    self.flow_geometry(page, self.fragmentainers.content_box(page), Some(flow))?;
                let base = match &self.fragmentainers {
                    Fragmentainers::Pages(_) => [0.0; 2],
                    Fragmentainers::Columns(rows) => rows.offset(page),
                };
                offset = path.iter().flatten().fold(base, |mut offset, flow| {
                    let relative = flow.relative;
                    offset[0] += relative[0];
                    offset[1] += relative[1];
                    offset
                });
                for (parent, resolved) in self.boxes.iter().zip(&path) {
                    if parent.reserves_end
                        && let Some(resolved) = resolved
                        && let Some(previous) = &parent.geometry
                    {
                        reserve +=
                            resolved.end_space(parent.clone) - previous.end_space(parent.clone);
                    }
                }
                let geometry = &path
                    .last()
                    .and_then(Option::as_ref)
                    .expect("stretched cell has source geometry")
                    .horizontal;
                paint.rect.x = geometry.x - geometry.padding[3] - geometry.border[3] + offset[0];
                paint.rect.width = geometry.width
                    + geometry.padding[1]
                    + geometry.padding[3]
                    + geometry.border[1]
                    + geometry.border[3];
                paint.padding = geometry.padding;
                if let Some(plan) = self.tables.last().map(|table| table.flow.plan.clone())
                    && plan.cell_clipped(flow.source)
                    && let Some(index) = plan.cell_index(flow.source)
                {
                    let resolved = self.table_geometry(page)?;
                    let (left, width) = plan.cell_span(index, &resolved);
                    paint.rect.x = left + offset[0];
                    paint.rect.width = width;
                }
            }
            if let Some((start_page, start_y)) = paint_start {
                let body_start = active
                    .geometry
                    .as_ref()
                    .and_then(|flow| {
                        self.bands
                            .iter()
                            .rev()
                            .find(|band| {
                                self.tables[band.table]
                                    .flow
                                    .plan
                                    .cell_index(flow.source)
                                    .is_some()
                            })
                            .and_then(|band| band.pages.get(&page))
                            .map(|page| page.body_start)
                    })
                    .unwrap_or(0.0);
                paint.rect.y = top
                    + offset[1]
                    + if page == start_page {
                        start_y
                    } else {
                        body_start
                    };
                paint.first_fragment = page == start_page || active.clone;
            } else if existing.is_none() {
                paint.rect.y = top + offset[1];
                paint.first_fragment = active.clone;
            }
            if existing.is_none() {
                slot = self.display_lists[page].len();
                self.display_lists[page].push(ScopedPaint {
                    visible: active.visible,
                    order: active.order.clone(),
                    scope: active.scope.clone(),
                    clip: active.clip,
                    group: active.group,
                    ink: None,
                    item: DisplayItem::Box(paint.clone()),
                    navigation: active.navigation,
                });
                let insert = |fragments: &mut Vec<(usize, usize)>| {
                    let index =
                        fragments.partition_point(|&(fragment_page, _)| fragment_page < page);
                    fragments.insert(index, (page, slot));
                };
                if let Some(index) = active.sliced {
                    insert(&mut self.sliced[index].fragments);
                }
                for node in [
                    active.clip_node,
                    active.transform_node,
                    active.clip_rect_node,
                    active.parent_3d_node,
                    active.sticky_node,
                    active.group_node,
                ]
                .into_iter()
                .flatten()
                {
                    insert(&mut self.clips[node].fragments);
                }
            }
            let end = if page == self.page {
                self.y - after
            } else {
                fragment_ends
                    .get(&page)
                    .copied()
                    .unwrap_or(self.fragmentainers.height(page) - reserve)
            };
            paint.rect.height = (top + end + offset[1] - paint.rect.y).max(paint.rect.height);
            paint.last_fragment = page == self.page || active.clone;
            layer_fragments.push((page, slot, paint.rect));
            self.display_lists[page][slot].item = DisplayItem::Box(paint.clone());
            if page != self.page {
                paint.rect.height = 0.0;
            }
        }
        let total = layer_fragments.iter().map(|(_, _, rect)| rect.height).sum();
        if let Some(table) = self.tables.last_mut()
            && let Some(cell) = active
                .geometry
                .as_ref()
                .and_then(|flow| table.flow.plan.cell_index(flow.source))
        {
            table.cell_heights[cell] = total;
            if table.flow.plan.scoped_borders() || !table.transforms.is_empty() {
                for &(page, slot, _) in &layer_fragments {
                    table.cell_fragments.insert((cell, page), slot);
                }
            }
        }
        if !backgrounds.is_empty() {
            let mut before = 0.0;
            for &(page, slot, rect) in &layer_fragments {
                let geometry = self.table_geometry(page)?;
                let table = self
                    .tables
                    .last()
                    .expect("table backgrounds have an active table");
                self.underlays.insert(
                    (page, slot),
                    backgrounds
                        .iter()
                        .map(|background| {
                            let ancestors = background.ancestors();
                            let own = ancestors.iter().rposition(Option::is_some);
                            let (scope, group, clip) = ancestors
                                .iter()
                                .enumerate()
                                .rev()
                                .find_map(|(index, source)| {
                                    source
                                        .and_then(|source| {
                                            table.layers.get(&(std::ptr::from_ref(source) as usize))
                                        })
                                        .map(|layer| {
                                            (
                                                if Some(index) == own {
                                                    layer.paint.background.clone()
                                                } else {
                                                    layer.paint.key.clone()
                                                },
                                                layer.group,
                                                layer.clip,
                                            )
                                        })
                                })
                                .unwrap_or_else(|| (self.key.clone(), self.group, self.clip));
                            ScopedPaint {
                                visible: true,
                                order: self.display_lists[page][slot].order.clone(),
                                scope,
                                clip,
                                group,
                                ink: None,
                                item: DisplayItem::Box(background.paint(
                                    rect,
                                    &geometry.columns(table.flow.plan.source.style().rtl),
                                )),
                                navigation: BoxNavigation::default(),
                            }
                        })
                        .collect(),
                );
                let table = self
                    .tables
                    .last_mut()
                    .expect("table underlay belongs to a table");
                for (index, background) in backgrounds.iter().enumerate() {
                    table.backgrounds.push(TableBackground {
                        repeated: false,
                        rows: background.rows.clone(),
                        page,
                        slot,
                        underlay: Some(index),
                        before,
                    });
                }
                before += rect.height;
            }
        }
        let rtl = active
            .geometry
            .as_ref()
            .is_some_and(|flow| flow.source.style().rtl);
        self.place_box_pending(&mut active, rtl)?;
        if active.overflow.is_some()
            && let Some(geometry) = &active.geometry
        {
            for &(page, slot, _) in &layer_fragments {
                let DisplayItem::Box(paint) = &self.display_lists[page][slot].item else {
                    continue;
                };
                let area = overflow::box_bounds(&active, geometry.source.style(), paint, page)?;
                if let Some(table) = self.tables.last_mut()
                    && !table.transforms.is_empty()
                    && let Some(cell) = table.flow.plan.cell_index(geometry.source)
                {
                    table.cell_overflow.insert((cell, page), area);
                }
                self.record_overflow_on(page, area);
            }
        }
        self.page_content = self.current_page_has_ink();
        Ok(layer_fragments
            .into_iter()
            .map(|(page, _, rect)| (page, rect))
            .collect())
    }
    /// Where the current flow's content paints.
    fn current_layer(&self) -> Layer {
        Layer {
            context: self.context.clone(),
            key: self.key.clone(),
            group: self.group,
            clip: self.clip,
            offset: [0.0; 2],
        }
    }
    /// The layer inside each inline box of `chain`, outermost first: that of the box itself
    /// when it paints in its own, created when first needed and shared by its split boxes,
    /// else the one enclosing it, offset by the relative offsets of the boxes resolved in
    /// `containing` (CSS Position 3 §3.3).
    fn chain_layers(
        &mut self,
        chain: &[&'a crate::style::Style],
        containing: InlineContaining,
    ) -> Vec<Layer> {
        let mut current = self.current_layer();
        let mut layers = Vec::with_capacity(chain.len());
        for &style in chain {
            if paints_in_own_layer(style) {
                let element = std::ptr::from_ref(style);
                let layer = if let Some(layer) = self.inline_layers.get(&element) {
                    layer.clone()
                } else {
                    let layer = self.new_inline_layer(style, &current, containing);
                    self.inline_layers.insert(element, layer.clone());
                    layer
                };
                let [x, y] = containing.relative(style);
                current = Layer {
                    offset: [current.offset[0] + x, current.offset[1] + y],
                    ..layer
                };
            }
            layers.push(current.clone());
        }
        layers
    }
    /// The layer of an inline box inside `parent`. Relative positioning makes a stacking
    /// context only with an integer `z-index`; sticky positioning, opacity, blending,
    /// isolation, filters, masks and forced stacking contexts always do, like z-index 0 (CSS
    /// Position 3 §3.3 and §3.4, CSS Color 4 §15, Compositing 1 §3.4).
    fn new_inline_layer(
        &mut self,
        style: &'a crate::style::Style,
        parent: &Layer,
        containing: InlineContaining,
    ) -> Layer {
        let order = self.order();
        let z_index = (style.position != Position::Static)
            .then(|| style.stacking_z_index())
            .flatten()
            .or((style.is_grouped() || style.forced_stacking_context()).then_some(0));
        let PositionedLayer { context, key, .. } =
            positioned_layer(&parent.context, z_index, order);
        let clip = if style.position == Position::Sticky {
            // Vertical margins do not apply to non-replaced inline boxes (CSS 2.1 §10.6.1).
            let margin = |side: usize| style.margin[side].resolve(containing.width).unwrap_or(0.0);
            let margins = [0.0, margin(1), 0.0, margin(3)];
            Some(self.sticky_node(style, margins, containing.rtl, parent.clip))
        } else {
            parent.clip
        };
        let group = if z_index.is_some() {
            let effect = if style.is_grouped() {
                Group::of(style)
            } else {
                Group::IDENTITY
            };
            let node = self.stacking_group(parent.group, effect, context.clone());
            if effect.anchored() {
                self.anchor_group(node, clip);
            }
            Some(node)
        } else {
            parent.group
        };
        Layer {
            context,
            key,
            group,
            clip,
            offset: [0.0; 2],
        }
    }
    /// The layer of content inside `inline_box`, or the current one for the root inline box.
    fn inline_layer(&mut self, layers: &InlineLayers<'a>, inline_box: Option<usize>) -> Layer {
        let chain = inline_chain(layers.paragraph, inline_box);
        self.chain_layers(&chain, layers.containing)
            .pop()
            .unwrap_or_else(|| self.current_layer())
    }
    /// Paint key of the next inline item in `layer`.
    fn layer_scope(&mut self, layer: &Layer) -> PaintKey {
        let order = self.order();
        extend(&layer.key, (0, order))
    }
    /// A line's glyph runs and decorations by the layer of the innermost inline box holding or
    /// drawing them, untouched ones first. Decorations over a positioned descendant of their
    /// box move with it (CSS Text Decoration 3 §2).
    fn split_line(
        &mut self,
        layers: &InlineLayers<'a>,
        runs: Vec<crate::text::GlyphRun>,
        decorations: &[DecorationPaint],
    ) -> Vec<LinePart> {
        fn part(parts: &mut Vec<LinePart>, layer: Layer) -> &mut LinePart {
            let index = if let Some(index) = parts.iter().position(|part| part.layer == layer) {
                index
            } else {
                parts.push(LinePart {
                    layer,
                    runs: Vec::new(),
                    decorations: Vec::new(),
                });
                parts.len() - 1
            };
            &mut parts[index]
        }
        let preserve_empty = runs.is_empty() && layers.paragraph.style().visibility.is_visible();
        let mut parts = vec![LinePart {
            layer: self.current_layer(),
            runs: Vec::new(),
            decorations: Vec::new(),
        }];
        for run in runs {
            let span = &layers.paragraph.spans()[run.source_span];
            if !span.style.visibility.is_visible() {
                continue;
            }
            let inline_box = span.inline_box;
            let layer = self.inline_layer(layers, inline_box);
            part(&mut parts, layer).runs.push(run);
        }
        for decoration in decorations {
            let style = decoration.owner.map_or_else(
                || layers.paragraph.style(),
                |owner| &layers.paragraph.boxes()[owner].style,
            );
            if !style.visibility.is_visible() {
                continue;
            }
            let mut layer = self.inline_layer(layers, decoration.owner);
            if decoration.positioned.is_some() {
                let carrier = self.inline_layer(layers, decoration.positioned);
                layer.offset = carrier.offset;
                layer.clip = carrier.clip;
            }
            part(&mut parts, layer).decorations.push(*decoration);
        }
        parts
            .retain(|part| preserve_empty || !part.runs.is_empty() || !part.decorations.is_empty());
        parts
    }
    fn emit_marker(&mut self, mut marker: crate::layout::markers::MeasuredMarker) {
        if let Some(mut paint) = marker.image.take() {
            paint.rect.x += marker.fragment.x;
            paint.rect.y += marker.fragment.baseline;
            if paint.rect.intersects(self.page_rect) {
                let scope = self.inline_key();
                self.foreground[self.page].push(ScopedPaint {
                    visible: true,
                    order: self.orders.take(),
                    scope,
                    clip: self.clip,
                    group: self.group,
                    ink: Some(paint.rect),
                    item: DisplayItem::Box(paint),
                    navigation: BoxNavigation::default(),
                });
                self.page_content = true;
            }
        }
        let layer = self.current_layer();
        self.emit_line(
            marker.fragment,
            marker.ink_bounds,
            &marker.decorations,
            &layer,
            &|_, _| true,
        );
    }

    /// Emits a visible text line between its decoration lines, above its text shadows, in
    /// `layer`, which offsets it; `decorates` tells which runs a decoration is drawn over.
    fn emit_line(
        &mut self,
        mut line: TextLine,
        ink_bounds: Option<[f32; 2]>,
        decorations: &[DecorationPaint],
        layer: &Layer,
        decorates: &dyn Fn(&crate::text::GlyphRun, &DecorationPaint) -> bool,
    ) {
        let visible = |baseline| {
            ink_bounds.is_none_or(|[above, below]| {
                baseline + below >= self.page_rect.y
                    && baseline + above <= self.page_rect.y + self.page_rect.height
            })
        };
        self.flow_content[self.page] |= visible(line.baseline - self.offset[1]);
        line.x += layer.offset[0];
        line.baseline += layer.offset[1];
        if visible(line.baseline) {
            let ink = ink_bounds.map(|[above, below]| Rect {
                x: line.x,
                y: line.baseline + above,
                width: line.width,
                height: below - above,
            });
            let shadows = crate::layout::inline::shadow_items(
                &line.runs,
                decorations,
                [line.x, line.baseline],
                decorates,
            );
            let [below, above] =
                crate::layout::inline::decoration_items(decorations, line.x, line.baseline);
            self.pages[self.page].push(line);
            let text = DisplayItem::Text {
                line: self.pages[self.page].len() - 1,
            };
            let scope = self.layer_scope(layer);
            for (item, ink) in shadows
                .into_iter()
                .chain(below)
                .map(|item| (item, None))
                .chain(std::iter::once((text, ink)))
                .chain(above.into_iter().map(|item| (item, None)))
            {
                self.foreground[self.page].push(ScopedPaint {
                    visible: true,
                    order: self.orders.take(),
                    scope: scope.clone(),
                    clip: layer.clip,
                    group: layer.group,
                    ink,
                    item,
                    navigation: BoxNavigation::default(),
                });
            }
            self.page_content = true;
        }
    }
    fn register_slice(
        &mut self,
        style: &'a crate::style::Style,
        fragments: Vec<(usize, usize)>,
    ) -> usize {
        let paint = |&(page, slot): &(usize, usize)| match &self.display_lists[page][slot].item {
            DisplayItem::Box(paint) => paint,
            _ => unreachable!("slice groups contain box paints"),
        };
        let first = fragments
            .first()
            .is_none_or(|fragment| paint(fragment).first_fragment);
        let last = fragments
            .last()
            .is_none_or(|fragment| paint(fragment).last_fragment);
        let key = std::ptr::from_ref(style);
        let existing = (!first)
            .then(|| self.open_slices.get(&key).copied())
            .flatten();
        let index = if let Some(index) = existing {
            self.sliced[index].fragments.extend(fragments);
            index
        } else {
            let index = self.sliced.len();
            self.sliced.push(SlicedBox { style, fragments });
            index
        };
        if last {
            self.open_slices.remove(&key);
        } else {
            self.open_slices.insert(key, index);
        }
        index
    }
    /// Assemble every decoration of a sliced box, including continuations around spanners.
    fn assemble_sliced_boxes(&mut self) {
        for group in &self.sliced {
            assemble_background(&group.fragments, &mut self.display_lists, false, false);
        }
    }
    fn position_canvas_backgrounds(&mut self) {
        for &(page, root_slot) in &self.canvas_roots {
            let Some(canvas_slot) = self.canvas_slots[page] else {
                continue;
            };
            let DisplayItem::Box(root) = &self.display_lists[page][root_slot].item else {
                continue;
            };
            let origins = root.background_origins();
            let DisplayItem::Box(canvas) = &mut self.display_lists[page][canvas_slot].item else {
                continue;
            };
            if let Some(geometry) = &mut canvas.background_geometry {
                std::sync::Arc::make_mut(geometry).origins = Some(origins);
            }
        }
        for page in self.display_lists.iter_mut().chain(&mut self.foreground) {
            for item in page {
                if let DisplayItem::Box(paint) = &mut item.item
                    && !paint.draw_background
                {
                    paint.decoration.clear_background();
                }
            }
        }
    }

    /// Paints a line's inline box fragments, then lays out its atomic inlines, in tree order.
    /// The line's top is the current flow position and `left` its left edge.
    fn inline_content(
        &mut self,
        line: &crate::text::Line,
        width: f32,
        left: f32,
        atomics: &mut [Option<crate::layout::inline::MeasuredAtomic<'a>>],
        layers: &mut InlineLayers<'a>,
    ) -> Result<(), LayoutError> {
        let source = layers.paragraph;
        let baseline = self.y + line.ascent;
        let origin = [left + self.offset[0], self.top + baseline + self.offset[1]];
        // Content boxes on this line of the boxes containing positioned descendants.
        let mut containing: Vec<(&'a crate::style::Style, Option<usize>, Vec<Rect>)> = Vec::new();
        for fragment in &line.boxes {
            let inline_box = &source.boxes()[fragment.index];
            let style: &'a crate::style::Style = &inline_box.style;
            let layer = self.inline_layer(layers, Some(fragment.index));
            let (mut border, padding) =
                crate::layout::inline::fragment_border_box(inline_box, fragment, width);
            border.x += origin[0] + layer.offset[0];
            border.y += origin[1] + layer.offset[1];
            // Positioned boxes overflow where they are offset to (CSS Overflow 3 §2.2).
            if layer.offset != [0.0; 2] {
                self.record_overflow(border);
            }
            if style.position == Position::Sticky
                && let Some(node) = layer.clip
            {
                self.clips[node].areas.push((self.page, border));
            }
            if style.inline_contains_absolute() {
                let content = crate::layout::inline::fragment_content_box(
                    inline_box,
                    fragment,
                    (border, padding),
                );
                match containing
                    .iter_mut()
                    .find(|(element, _, _)| std::ptr::eq(*element, style))
                {
                    Some((_, _, rects)) => rects.push(content),
                    None => containing.push((style, layer.clip, vec![content])),
                }
            }
            let Some(paint) =
                crate::layout::inline::fragment_paint(inline_box, fragment, (border, padding))
            else {
                continue;
            };
            self.page_content |= paint.has_ink() && paint.rect.intersects(self.page_rect);
            let scope = self.layer_scope(&layer);
            // The box's own filter or mask group is bounded by its fragments.
            if Group::of(style).anchored()
                && let Some(node) = layer.group
            {
                self.clips[node].paints.push((self.page, paint.clone()));
            }
            layers.fragments[fragment.index].push((self.page, self.foreground[self.page].len()));
            self.foreground[self.page].push(ScopedPaint {
                visible: style.visibility.is_visible(),
                order: self.orders.take(),
                scope,
                clip: layer.clip,
                group: layer.group,
                ink: None,
                item: DisplayItem::Box(paint),
                navigation: inline_box.navigation,
            });
        }
        for (style, clip, rects) in containing {
            let page = self.page;
            self.inline_container(style, clip).lines.push((page, rects));
        }
        for fragment in &line.links {
            let layer = self.inline_layer(layers, fragment.inline_box);
            let mut rect = fragment.rect;
            rect.x += origin[0] + layer.offset[0];
            rect.y += origin[1] + layer.offset[1];
            let scope = self.layer_scope(&layer);
            self.foreground[self.page].push(ScopedPaint {
                visible: fragment
                    .inline_box
                    .map_or_else(|| source.style(), |index| &source.boxes()[index].style)
                    .visibility
                    .is_visible(),
                order: self.orders.take(),
                scope,
                clip: layer.clip,
                group: layer.group,
                ink: None,
                item: DisplayItem::Box(BoxPaint {
                    inline_edges: [false; 2],
                    ..BoxPaint::whole(rect, Default::default())
                }),
                navigation: BoxNavigation {
                    link: Some(fragment.link),
                    destination: None,
                },
            });
        }
        for fragment in &line.atomics {
            let atomic = atomics[fragment.index]
                .take()
                .expect("each atomic inline is placed on exactly one line");
            let inline_box = source.atomics()[fragment.index].inline_box;
            let chain = inline_chain(source, inline_box);
            let chain_layers = self.chain_layers(&chain, layers.containing);
            let layer = chain_layers
                .last()
                .cloned()
                .unwrap_or_else(|| self.current_layer());
            let key = self.layer_scope(&layer);
            let opened = self.open_inline_containers(&chain, &chain_layers);
            self.unbroken_block(
                atomic.block,
                Unbroken {
                    page: self.page,
                    y: baseline + fragment.baseline - atomic.metrics.baseline + atomic.margin_top,
                    offset: [
                        self.offset[0] + left + fragment.x + layer.offset[0],
                        self.offset[1] + layer.offset[1],
                    ],
                    clip: layer.clip,
                    context: layer.context,
                    group: layer.group,
                    parent_3d: self.parent_3d.filter(|_| inline_box.is_none()),
                    key,
                    reserved_order: None,
                },
            )?;
            self.close_inline_containers(&opened);
        }
        Ok(())
    }
    /// Opens a container for each box of `chain` with layers `layers` that is the containing
    /// block of positioned descendants of the content laid out next (CSS Position 3 §2.1).
    fn open_inline_containers(
        &mut self,
        chain: &[&'a crate::style::Style],
        layers: &[Layer],
    ) -> Vec<(&'a crate::style::Style, Layer)> {
        let mut opened = Vec::new();
        for (&style, layer) in chain.iter().zip(layers) {
            if style.inline_contains_absolute() {
                self.containers.push(Container {
                    pending: Vec::new(),
                    fixed: style.inline_contains_fixed(),
                });
                opened.push((style, layer.clone()));
            }
        }
        opened
    }
    /// Closes containers that `open_inline_containers` opened, handing their positioned boxes
    /// to the inline boxes.
    fn close_inline_containers(&mut self, opened: &[(&'a crate::style::Style, Layer)]) {
        for (style, layer) in opened.iter().rev() {
            let pending = self
                .containers
                .pop()
                .expect("inline containers close in the order they opened")
                .pending;
            self.inline_container(style, layer.clip)
                .pending
                .extend(pending);
        }
    }
    /// The containing block that inline box `style` forms for positioned descendants, which
    /// waits for the end of its block container, the current one.
    fn inline_container(
        &mut self,
        style: &'a crate::style::Style,
        clip: Option<usize>,
    ) -> &mut InlineContainer<'a> {
        let element = std::ptr::from_ref(style);
        let index = if let Some(index) = self
            .inline_containers
            .iter()
            .position(|(container, _)| *container == element)
        {
            index
        } else {
            self.inline_containers.push((
                element,
                InlineContainer {
                    pending: Vec::new(),
                    lines: Vec::new(),
                    rtl: style.rtl,
                    clip,
                    depth: self.boxes.len(),
                },
            ));
            self.inline_containers.len() - 1
        };
        &mut self.inline_containers[index].1
    }
    /// Records the empty fragment of split inline box `style` with layer `layer` at the start
    /// of the current line at `y`, which a block splitting it adjoins.
    fn split_fragment(&mut self, style: &'a crate::style::Style, layer: &Layer, y: f32) {
        let (x, width, rtl, _) = self.containing_geometry();
        let rect = Rect {
            x: if rtl { x + width } else { x } + self.offset[0] + layer.offset[0],
            y: y + self.offset[1] + layer.offset[1],
            width: 0.0,
            height: 0.0,
        };
        let page = self.page;
        self.inline_container(style, layer.clip)
            .lines
            .push((page, vec![rect]));
    }
    /// Enters the layers of the inline boxes that `block` splits, when it starts its source
    /// box, with the offsets they give it.
    fn enter_split_inline(
        &mut self,
        block: &mut MeasuredBlock<'a>,
        starts_source: bool,
    ) -> SplitInline<'a> {
        let source: &'a BlockBox = block.source;
        if !starts_source || source.enclosing_inline().is_empty() {
            return SplitInline::default();
        }
        let chain: Vec<&'a crate::style::Style> = source
            .enclosing_inline()
            .iter()
            .map(|style| &**style)
            .collect();
        let layers = self.chain_layers(&chain, InlineContaining::of(self.containing_geometry()));
        let start = self.top + self.y + self.pending_value();
        let containers = self.open_inline_containers(&chain, &layers);
        for (style, layer) in &containers {
            if self.inline_container(style, layer.clip).lines.is_empty() {
                self.split_fragment(style, layer, start);
            }
        }
        let nodes = chain
            .iter()
            .zip(&layers)
            .filter_map(|(style, layer)| {
                if style.position == Position::Sticky {
                    layer.clip
                } else if Group::of(style).anchored() {
                    layer.group
                } else {
                    None
                }
            })
            .collect();
        let inner = layers.last().expect("the chain holds the enclosing boxes");
        self.context = inner.context.clone();
        self.key = inner.key.clone();
        self.group = inner.group;
        self.clip = inner.clip;
        // An inline box, which is not transformable, is the parent of the block.
        self.parent_3d = None;
        block.offset[0] += inner.offset[0];
        block.offset[1] += inner.offset[1];
        SplitInline { containers, nodes }
    }
    /// Places the positioned boxes of the inline containing blocks whose block container is
    /// at `depth` or deeper, which has ended.
    fn place_inline_pending(&mut self, depth: usize) -> Result<(), LayoutError> {
        let (ready, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.inline_containers)
            .into_iter()
            .partition(|(_, container)| container.depth >= depth);
        self.inline_containers = open;
        for (_, mut container) in ready {
            for pending in std::mem::take(&mut container.pending) {
                let page = pending.position.page;
                let rect = inline_containing_rect(&container, &self.fragmentainers, page)?;
                self.place(
                    pending,
                    &Containing::Rect {
                        rect,
                        rtl: container.rtl,
                        clip: container.clip,
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(crate) fn document(
        mut self,
        root: Option<MeasuredBlock<'a>>,
        destinations: usize,
        initial_events: impl Iterator<Item = &'a crate::document::SemanticEvent>,
    ) -> Result<Paginated, LayoutError> {
        if root.is_none() {
            for event in initial_events {
                self.string_locations.record(self.page, event);
            }
        }
        // Page breaks fall inside the root's formatting context.
        if let Some(mut root) = root {
            root.independent = false;
            self.containers.push(Container {
                pending: Vec::new(),
                fixed: false,
            });
            self.pending.push(root.before.clone());
            self.block(root, BreakTail::default())?;
            for pending in self.containers.pop().map(|c| c.pending).unwrap_or_default() {
                self.place(pending, &Containing::Page)?;
            }
        }
        // Trailing unpainted height need not create physical sheets (CSS Page 3 §3.2).
        // Semantic locations retain hidden content and empty destination elements.
        let page_count = (self.string_locations.last_page() + 1).max(
            (0..self.pages.len())
                .rfind(|&page| self.page_has_ink(page))
                .map_or(1, |page| page + 1),
        );
        // CSS 2.1 §9.6.1 repeats fixed content in each physical page's containing block.
        let fixed = std::mem::take(&mut self.fixed);
        if fixed.len().saturating_mul(page_count) > MAX_FIXED_PLACEMENTS {
            return Err(LayoutError::FixedPlacementLimit {
                limit: MAX_FIXED_PLACEMENTS,
            });
        }
        for pending in fixed {
            let mut measurements = HashMap::new();
            for page in 0..page_count {
                let canvas = match &self.fragmentainers {
                    Fragmentainers::Pages(pages) => pages.records[page].geometry.content_box(),
                    Fragmentainers::Columns(_) => self.canvas_rect,
                };
                let key = [canvas.x, canvas.y, canvas.width, canvas.height].map(f32::to_bits);
                if let std::collections::hash_map::Entry::Vacant(entry) = measurements.entry(key) {
                    entry.insert(crate::layout::positioned::place(
                        self.measurer,
                        pending.block,
                        canvas,
                        false,
                        &pending.position,
                    )?);
                }
                let (measured, top) = &measurements[&key];
                self.lay_out_positioned(measured.clone(), *top, page, None, pending.clone())?;
            }
        }
        self.assemble_sliced_boxes();
        self.position_canvas_backgrounds();
        let regions = self.effect_regions()?;
        let bounds = self.group_bounds();
        let mut display_lists = Vec::with_capacity(self.display_lists.len());
        let mut located = Located {
            links: Vec::with_capacity(self.display_lists.len()),
            destinations: vec![None; destinations],
        };
        for (page, (foreground, mut items)) in self
            .foreground
            .into_iter()
            .zip(self.display_lists)
            .take(page_count)
            .enumerate()
        {
            items = items
                .into_iter()
                .enumerate()
                .flat_map(|(slot, item)| {
                    self.underlays
                        .remove(&(page, slot))
                        .unwrap_or_default()
                        .into_iter()
                        .chain(std::iter::once(item))
                })
                .collect();
            items.extend(foreground);
            let outlines = outlines(&items, &self.clips);
            items.extend(outlines);
            items.sort_by(|a, b| a.scope.cmp(&b.scope).then_with(|| a.order.cmp(&b.order)));
            display_lists.push(clipped(
                items,
                page,
                &self.clips,
                &regions,
                &bounds,
                &mut located,
            )?);
        }
        let Fragmentainers::Pages(mut physical) = self.fragmentainers else {
            unreachable!("document pagination uses physical pages");
        };
        physical.boxes.truncate(page_count);
        physical.records.truncate(page_count);
        self.pages.truncate(page_count);
        Ok(Paginated {
            page_boxes: physical.boxes,
            page_records: physical.records,
            pages: self.pages,
            display_lists,
            links: located.links,
            destinations: located.destinations,
            string_locations: self.string_locations,
        })
    }
}

/// The styles of the inline boxes holding `inline_box` and of the box itself, outermost first.
fn inline_chain(
    paragraph: &crate::document::Paragraph,
    inline_box: Option<usize>,
) -> Vec<&crate::style::Style> {
    let mut chain: Vec<_> =
        std::iter::successors(inline_box, |&index| paragraph.boxes()[index].parent)
            .map(|index| &*paragraph.boxes()[index].style)
            .collect();
    chain.reverse();
    chain
}

/// The containing block that an inline box's fragments form on `page`: a rectangle from the
/// start-most content edges of its first fragment to the end-most content edges of its
/// end-most fragments in each axis, in the box's direction (CSS Position 3 §2.1), cut by the
/// page as a box continuing across page breaks is.
fn inline_containing_rect(
    container: &InlineContainer<'_>,
    fragmentainers: &Fragmentainers<'_, '_>,
    page: usize,
) -> Result<Rect, LayoutError> {
    let (Some((first_page, first_line)), Some((end_page, _))) =
        (container.lines.first(), container.lines.last())
    else {
        return Err(LayoutError::MissingContainingFragment { page });
    };
    let right = |rect: &Rect| rect.x + rect.width;
    let bottom = |rect: &Rect| rect.y + rect.height;
    let rects = || container.lines.iter().flat_map(|(_, rects)| rects);
    let first = first_line
        .iter()
        .copied()
        .reduce(|start, rect| {
            let starts_before = if container.rtl {
                right(&rect) > right(&start)
            } else {
                rect.x < start.x
            };
            if starts_before { rect } else { start }
        })
        .ok_or(LayoutError::MissingContainingFragment { page })?;
    let [left, right] = if container.rtl {
        [
            rects().map(|rect| rect.x).fold(f32::INFINITY, f32::min),
            right(&first),
        ]
    } else {
        [
            first.x,
            rects().map(right).fold(f32::NEG_INFINITY, f32::max),
        ]
    };
    let end = container
        .lines
        .iter()
        .filter(|(line_page, _)| line_page == end_page)
        .flat_map(|(_, rects)| rects)
        .map(bottom)
        .fold(f32::NEG_INFINITY, f32::max);
    let area = fragmentainers.content_box(page);
    let edge = |fragment_page: usize, on_page: f32| match page.cmp(&fragment_page) {
        std::cmp::Ordering::Less => area.y + area.height,
        std::cmp::Ordering::Equal => on_page,
        std::cmp::Ordering::Greater => area.y,
    };
    let top = edge(*first_page, first.y);
    let end = edge(*end_page, end);
    Ok(Rect {
        x: left,
        y: top,
        width: (right - left).max(0.0),
        height: (end - top).max(0.0),
    })
}

/// The outline of each box fragment, painted over everything else in the stacking context its
/// group composites (CSS 2.1 Appendix E step 10, which CSS UI 4 §3 leaves to implementations),
/// under the box's own clips, transforms and groups.
fn outlines(items: &[ScopedPaint], nodes: &[EffectNode<'_>]) -> Vec<ScopedPaint> {
    items
        .iter()
        .filter_map(|paint| {
            let DisplayItem::Box(fragment) = &paint.item else {
                return None;
            };
            let outline = fragment.outline()?;
            let context = paint
                .group
                .and_then(|group| nodes[group].context.clone())
                .unwrap_or_else(|| Rc::from([]));
            Some(ScopedPaint {
                scope: extend(&context, (OUTLINE_LEVEL, paint.order.clone())),
                ink: None,
                item: DisplayItem::Box(outline),
                navigation: BoxNavigation::default(),
                ..paint.clone()
            })
        })
        .collect()
}

/// Link areas by page, and each destination's first box fragment in page and paint order.
struct Located {
    links: Vec<Vec<LinkArea>>,
    destinations: Vec<Option<Destination>>,
}

/// One push of an item's effect chain: a group node or a clip or transform node.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    Group(usize),
    Clip(usize),
}

/// Wraps runs of items sharing an effect chain in push/pop pairs, and locates the link areas
/// and destinations of box items through that chain.
fn clipped(
    items: Vec<ScopedPaint>,
    page: usize,
    nodes: &[EffectNode<'_>],
    regions: &[Vec<(usize, Region)>],
    bounds: &[Vec<(usize, BoxPaint)>],
    located: &mut Located,
) -> Result<Vec<DisplayItem>, LayoutError> {
    let mut link_polygons = Vec::new();
    let mut output = Vec::with_capacity(items.len());
    // Open frames and the pushes each emitted.
    let mut open: Vec<(Frame, usize)> = Vec::new();
    let mut groups = Vec::new();
    let mut frames = Vec::new();
    let mut chain = Vec::new();
    for paint in items {
        if !paint.visible {
            continue;
        }
        chain.clear();
        let mut node = paint.clip;
        while let Some(index) = node {
            chain.push(index);
            node = nodes[index].parent;
        }
        chain.reverse();
        let mut regions_here = Vec::with_capacity(chain.len());
        for &index in &chain {
            regions_here.push(
                regions[index]
                    .iter()
                    .find_map(|(fragment_page, clip)| (*fragment_page == page).then_some(*clip))
                    .ok_or(LayoutError::MissingClipFragment { page })?,
            );
        }
        if regions_here.iter().any(|region| match region {
            Region::Transform([a, b, c, d, _, _]) => {
                f64::from(*a) * f64::from(*d) - f64::from(*b) * f64::from(*c) == 0.0
            }
            Region::Clip(_) => false,
        }) {
            continue;
        }
        if let DisplayItem::Box(fragment) = &paint.item {
            if let Some(link) = paint.navigation.link {
                let polygon =
                    crate::layout::navigation::page_polygon(fragment.rect, &regions_here, true);
                if !polygon.is_empty() {
                    link_polygons.push((link, polygon));
                }
            }
            if let Some(destination) = paint.navigation.destination
                && located.destinations[destination].is_none()
            {
                located.destinations[destination] = Some(crate::layout::navigation::destination(
                    page,
                    fragment.rect,
                    &regions_here,
                ));
            }
        }
        // Ink is compared only with clips in its own coordinate space.
        let untransformed = regions_here
            .iter()
            .rposition(|region| matches!(region, Region::Transform(_)))
            .map_or(0, |index| index + 1);
        if let Some(ink) = paint.ink
            && regions_here[untransformed..]
                .iter()
                .any(|region| matches!(region, Region::Clip(clip) if !clip.bounds().overlaps(ink)))
        {
            continue;
        }
        let mut item = paint.item;
        if untransformed > 0
            && let DisplayItem::Box(paint) = &mut item
        {
            Backgrounds::scroll_fixed(&mut paint.decoration.backgrounds);
        }
        groups.clear();
        let mut group = paint.group;
        while let Some(index) = group {
            let Effect::Group(effect) = nodes[index].effect else {
                unreachable!("group chains hold only group nodes")
            };
            // A group without opacity, blending, isolation or filters paints as its content
            // does.
            let fragments: Vec<BoxPaint> = bounds[index]
                .iter()
                .filter(|(fragment_page, _)| *fragment_page == page)
                .map(|(_, paint)| paint.clone())
                .collect();
            let pushes = effect.pushes(&fragments);
            if !pushes.is_empty() {
                groups.push((index, effect, pushes));
            }
            group = nodes[index].parent;
        }
        groups.reverse();
        // Clipping or transforming each member of a group equals doing so to the group, so
        // groups open outermost and clips and transforms reopen inside each group; anchored
        // groups open after the clips and transforms of their element's own box.
        frames.clear();
        let mut placed = 0;
        for (index, effect, _) in &groups {
            if effect.anchored() {
                let depth = match effect.anchor {
                    Some(anchor) => {
                        chain
                            .iter()
                            .position(|&node| node == anchor)
                            .ok_or(LayoutError::MissingClipFragment { page })?
                            + 1
                    }
                    None => 0,
                };
                frames.extend(
                    chain[placed..depth.max(placed)]
                        .iter()
                        .map(|&node| Frame::Clip(node)),
                );
                placed = depth.max(placed);
            }
            frames.push(Frame::Group(*index));
        }
        frames.extend(chain[placed..].iter().map(|&node| Frame::Clip(node)));
        let common = open
            .iter()
            .zip(&frames)
            .take_while(|((open, _), frame)| open == *frame)
            .count();
        for (_, pushes) in open.drain(common..).rev() {
            output.extend(std::iter::repeat_n(DisplayItem::Pop, pushes));
        }
        for &frame in &frames[common..] {
            let pushes = match frame {
                Frame::Group(index) => groups
                    .iter()
                    .find(|(group, _, _)| *group == index)
                    .map(|(_, _, pushes)| pushes.clone())
                    .unwrap_or_default(),
                Frame::Clip(index) => {
                    let position = chain
                        .iter()
                        .position(|&node| node == index)
                        .expect("clip frames come from the item's chain");
                    vec![match regions_here[position] {
                        Region::Clip(clip) => DisplayItem::PushClip(clip),
                        Region::Transform(matrix) => DisplayItem::PushTransform(matrix),
                    }]
                }
            };
            open.push((frame, pushes.len()));
            output.extend(pushes);
        }
        output.push(item);
    }
    let pops = open.iter().map(|(_, pushes)| pushes).sum::<usize>();
    output.extend(std::iter::repeat_n(DisplayItem::Pop, pops));
    located
        .links
        .push(crate::layout::navigation::link_areas(&link_polygons));
    Ok(output)
}

fn assemble_background(
    fragments: &[(usize, usize)],
    lists: &mut [Vec<ScopedPaint>],
    horizontal: bool,
    rtl: bool,
) {
    if fragments.len() < 2 {
        return;
    }
    let extent = |&(page, slot): &(usize, usize), lists: &[Vec<ScopedPaint>]| match &lists[page]
        [slot]
        .item
    {
        DisplayItem::Box(paint) => {
            if horizontal {
                paint.rect.width
            } else {
                paint.rect.height
            }
        }
        _ => unreachable!("slice groups contain box paints"),
    };
    let total: f32 = fragments
        .iter()
        .map(|fragment| extent(fragment, lists))
        .sum();
    let mut before = 0.0;
    for &(page, slot) in fragments {
        let size = extent(&(page, slot), lists);
        if let DisplayItem::Box(paint) = &mut lists[page][slot].item {
            paint.background_box = Some(if horizontal {
                Rect {
                    x: paint.rect.x - if rtl { total - before - size } else { before },
                    width: total,
                    ..paint.rect
                }
            } else {
                Rect {
                    y: paint.rect.y - before,
                    height: total,
                    ..paint.rect
                }
            });
        }
        before += size;
    }
}
