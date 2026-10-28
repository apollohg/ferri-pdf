use crate::layout::{
    LayoutError,
    breaks::Fragmentainer,
    geometry::horizontal,
    margin::MarginStrut,
    multicol::{ColumnFlow, ColumnGeometry, MeasuredColumns, SpannerFragment},
};
use ferri_pdf_document::{BlockBox, BlockContent, BoxNavigation};
use ferri_pdf_graphics::{BoxDecoration, BoxPaint, Rect};
use ferri_pdf_style::{BreakBetween, Display, Style};
use ferri_pdf_text::{Line, TextContext};

mod metrics;
pub(super) use metrics::{ItemMetrics, MetricsKey};

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
    pub(crate) fn align_cell(&mut self, offset: f32) {
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
            FlowItem::Paragraph(paragraph) => paragraph.lines.first().map(|line| line.ascent),
            FlowItem::Block(block) | FlowItem::Spanner(block) => block.first_baseline(),
            FlowItem::Columns(columns) => columns.baselines[0],
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
                .last()
                .map(|line| paragraph.height - line.height + line.ascent),
            FlowItem::Block(block) | FlowItem::Spanner(block) => block.last_baseline(),
            FlowItem::Columns(columns) => columns.baselines[1],
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
    pub paragraph: &'a ferri_pdf_document::Paragraph,
    pub style: &'a Style,
    pub x: f32,
    pub width: f32,
    pub containing_height: Option<f32>,
    pub height: f32,
    pub lines: Vec<Line>,
    pub positions: Vec<ferri_pdf_text::TextPosition<'a>>,
    pub anchors: Vec<LineAnchor<'a>>,
    /// Atomic inlines in paragraph order, taken when their line is placed.
    pub atomics: Vec<Option<crate::layout::inline::MeasuredAtomic<'a>>>,
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
    pub after_events: Vec<&'a ferri_pdf_document::SemanticEvent>,
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
    pub after_events: Vec<&'a ferri_pdf_document::SemanticEvent>,
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
    pub heights: Vec<ferri_pdf_table::CellHeight>,
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
pub(super) enum FlowItem<'a> {
    Events(&'a [ferri_pdf_document::SemanticEvent]),
    Image {
        paint: ferri_pdf_graphics::ImagePaint,
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
            Self::Image { height, .. } => *height,
            Self::Block(block) | Self::Spanner(block) => block.height,
            Self::Columns(columns) => columns.height,
            Self::Paragraph(paragraph) => paragraph.height,
            Self::Row(row) => row.height,
            Self::Margin(margin) => margin.value(),
            Self::Space(value) | Self::Edge(_, value) => *value,
            Self::Positioned(_) | Self::Events(_) => 0.0,
        }
    }
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

pub(super) struct Measurer<'a> {
    pub work: crate::layout::work::Budget,
    pub text: &'a TextContext<'a>,
    pub canvas_images: bool,
    pub page_height: f32,
    pub sources: crate::layout::source::SourceIds,
    pub clearance_targets: std::collections::HashMap<*const BlockBox, usize>,
    pub grid_parent: Option<crate::layout::grid::ParentContext>,
    pub item_metrics: std::collections::HashMap<MetricsKey, ItemMetrics>,
    /// The fragmentainer of the flow being measured.
    pub fragmentainer: Fragmentainer,
    /// The multi-column container whose formatting context the flow belongs to.
    pub column_flow: Option<ColumnFlow>,
    /// The spanner being measured, which establishes a formatting context (§6.1).
    pub spanner: Option<*const BlockBox>,
    /// The flow is laid out without page breaks: inside an atomic inline or an absolutely
    /// positioned box.
    pub unbroken: bool,
    /// Named-page boundaries are excluded beneath table cells.
    pub named_pages: bool,
}
impl Measurer<'_> {
    pub(crate) fn intrinsic_widths(
        &self,
        paragraph: &ferri_pdf_document::Paragraph,
        atomics: &[ferri_pdf_text::IntrinsicWidths],
    ) -> Result<ferri_pdf_text::IntrinsicWidths, LayoutError> {
        self.work.spend(paragraph.text().len().max(1))?;
        Ok(self.text.intrinsic_widths(paragraph, atomics)?)
    }

    pub(crate) fn text_lines(
        &self,
        paragraph: &ferri_pdf_document::Paragraph,
        width: f32,
        atomics: &[ferri_pdf_text::AtomicMetrics],
    ) -> Result<Vec<Line>, LayoutError> {
        self.work.spend(paragraph.text().len().max(1))?;
        Ok(self.text.layout_paragraph(paragraph, width, atomics)?)
    }

    pub(super) fn box_paint(
        &self,
        block: &BlockBox,
        geometry: &crate::layout::geometry::HorizontalBox,
    ) -> Option<BoxPaint> {
        let style = block.style();
        let mut decoration = block.decoration().clone();
        if block.is_canvas_source() && !self.canvas_images {
            decoration.clear_background();
        }
        // Clips, containing blocks and navigation need their page fragments even without ink.
        let fragments = (block.is_canvas_source() && self.canvas_images)
            || style.clipped_axes().contains(&true)
            || style.position != ferri_pdf_style::Position::Static
            || !style.transform.is_empty()
            || !block.navigation().is_empty();
        let rect = geometry.border_box(0.0);
        (style.display != Display::None && (decoration.is_visible() || fragments)).then(|| {
            BoxPaint {
                draw_background: !block.is_canvas_source(),
                padding: geometry.padding,
                ..BoxPaint::whole(rect, decoration)
            }
        })
    }
    pub(crate) fn block<'a>(
        &mut self,
        block: &'a BlockBox,
        x: f32,
        width: f32,
        containing_rtl: bool,
        root: bool,
        containing_height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, LayoutError> {
        let style = block.style();
        if style.display.is_table() {
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
        self.format_block(block, geometry, root, containing_height, None)
    }

    /// Measures `child` as a spanner across the multi-column container of `flow`.
    fn spanner<'a>(
        &mut self,
        child: &'a BlockBox,
        flow: ColumnFlow,
    ) -> Result<MeasuredBlock<'a>, LayoutError> {
        let saved = (self.fragmentainer, self.spanner);
        self.fragmentainer = flow.fragmentainer;
        self.spanner = Some(child);
        let measured = self.block(
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

    pub(super) fn format_block<'a>(
        &mut self,
        block: &'a BlockBox,
        geometry: crate::layout::geometry::HorizontalBox,
        root: bool,
        containing_height: Option<f32>,
        used_height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, LayoutError> {
        let context = (self.fragmentainer, self.column_flow);
        let measured = self.format_flow(block, geometry, root, containing_height, used_height);
        (self.fragmentainer, self.column_flow) = context;
        measured
    }

    fn start_flow<'a>(
        &mut self,
        block: &'a BlockBox,
        geometry: crate::layout::geometry::HorizontalBox,
        root: bool,
        containing_height: Option<f32>,
        used_height: Option<f32>,
    ) -> Result<FlowStart<'a>, LayoutError> {
        self.work.spend(1)?;
        let restore_column_flow = self.column_flow;
        let style = block.style();
        let spanning = self
            .spanner
            .is_some_and(|spanner| std::ptr::eq(spanner, block));
        if spanning
            || !matches!(style.display, Display::Block | Display::ListItem)
            || style.establishes_formatting_context()
        {
            self.column_flow = None;
        }
        let started = (|| {
            let index = self.sources.block(block);
            if !geometry.width.is_finite()
                || !geometry.x.is_finite()
                || geometry
                    .padding
                    .iter()
                    .chain(geometry.margin.iter().flatten())
                    .any(|p| !p.is_finite())
            {
                return Err(LayoutError::NonFiniteBlockGeometry { block: index });
            }
            if block.image().is_some() {
                return crate::layout::images::measure(
                    self,
                    block,
                    &geometry,
                    containing_height,
                    used_height,
                )
                .map(FlowStart::Complete);
            }
            if style.display.is_grid() {
                return crate::layout::grid::measure(
                    self,
                    block,
                    &geometry,
                    containing_height,
                    used_height,
                )
                .map(FlowStart::Complete);
            }
            if style.display.is_flex() {
                return crate::layout::flex::measure(
                    self,
                    block,
                    &geometry,
                    containing_height,
                    used_height,
                )
                .map(FlowStart::Complete);
            }
            let vertical =
                crate::layout::geometry::VerticalBox::new(style, &geometry, containing_height)
                    .with_used_height(used_height);
            let marker = if let Some(paragraph) = block.marker() {
                let index = self.sources.paragraph(paragraph);
                Some(crate::layout::markers::measure(
                    paragraph,
                    self,
                    index,
                    geometry.x - geometry.padding[3] - geometry.border[3],
                    geometry.x + geometry.width + geometry.padding[1] + geometry.border[1],
                )?)
            } else {
                None
            };
            let top_edge = geometry.padding[0] + geometry.border[0];
            let bottom_edge = geometry.padding[2] + geometry.border[2];
            let independent = style.establishes_formatting_context() || spanning;
            let edges = self.fragmentainer.edges(style);
            let columns = (style.display != Display::None && style.multicol.is_multicol())
                .then(|| ColumnGeometry::resolve(style, geometry.x, geometry.width));
            let (content_x, content_width) = columns
                .as_ref()
                .map_or((geometry.x, geometry.width), |columns| {
                    (columns.left(0), columns.width)
                });
            let outer = (self.fragmentainer, self.column_flow);
            if columns.is_some() {
                self.fragmentainer = Fragmentainer::Column;
                self.column_flow = Some(ColumnFlow {
                    x: geometry.x,
                    width: geometry.width,
                    rtl: style.rtl,
                    containing_height: vertical.definite(),
                    fragmentainer: outer.0,
                });
            }
            let visible = style.display != Display::None;
            Ok(FlowStart::Pending(FlowFrame {
                block,
                root,
                index,
                containing_height,
                used_height,
                cell_percentage_chain: false,
                vertical,
                before: MarginStrut::edge(block, 0, geometry.containing_width),
                after: MarginStrut::edge(block, 2, geometry.containing_width),
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
                outer,
                restore_column_flow,
                previous_break_after: BreakBetween::Auto,
                children: if visible { block.children() } else { &[] }.iter(),
                marker,
            }))
        })();
        if !matches!(&started, Ok(FlowStart::Pending(_))) {
            self.column_flow = restore_column_flow;
        }
        started
    }

    fn format_flow<'a>(
        &mut self,
        block: &'a BlockBox,
        geometry: crate::layout::geometry::HorizontalBox,
        root: bool,
        containing_height: Option<f32>,
        used_height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, LayoutError> {
        let mut frames =
            match self.start_flow(block, geometry, root, containing_height, used_height)? {
                FlowStart::Complete(measured) => return Ok(measured),
                FlowStart::Pending(frame) => vec![frame],
            };
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
                Some(BlockContent::Block(child)) => {
                    if let Some(flow) = self.column_flow.filter(|_| child.style().multicol.span_all)
                    {
                        frame.push_block(self.spanner(child, flow)?, true);
                    } else if child.style().display.is_table() {
                        frame.push_block(
                            self.block(
                                child,
                                frame.content_x,
                                frame.content_width,
                                frame.block.style().rtl,
                                false,
                                frame.vertical.definite(),
                            )?,
                            false,
                        );
                    } else {
                        let geometry = block_geometry(
                            self,
                            child,
                            frame.content_x,
                            frame.content_width,
                            frame.block.style().rtl,
                            frame.vertical.definite(),
                        )?;
                        let child_style = child.style();
                        let cell_percentage_chain = (frame.block.style().display
                            == Display::TableCell
                            || frame.cell_percentage_chain)
                            && frame.vertical.definite().is_none()
                            && child.image().is_none()
                            && matches!(child_style.height, ferri_pdf_style::Size::Value(_))
                            && child_style.height.resolve_definite(None).is_none();
                        let used_height = (cell_percentage_chain
                            && matches!(
                                child_style.overflow[1],
                                ferri_pdf_style::Overflow::Auto | ferri_pdf_style::Overflow::Scroll
                            ))
                        .then(|| {
                            let edges = geometry.padding[0]
                                + geometry.padding[2]
                                + geometry.border[0]
                                + geometry.border[2];
                            ferri_pdf_style::SizeConstraints::new(
                                [
                                    &ferri_pdf_style::Size::ZERO,
                                    &child_style.min_height,
                                    &child_style.max_height,
                                ],
                                None,
                                child_style.box_sizing,
                                edges,
                            )
                            .constrain(0.0)
                        });
                        match self.start_flow(
                            child,
                            geometry,
                            false,
                            frame.vertical.definite(),
                            used_height,
                        )? {
                            FlowStart::Complete(measured) => frame.push_block(measured, false),
                            FlowStart::Pending(mut frame) => {
                                frame.cell_percentage_chain = cell_percentage_chain;
                                frames.push(frame);
                            }
                        }
                    }
                }
                Some(BlockContent::Inline(paragraph)) => {
                    let paragraph = self.paragraph(
                        paragraph,
                        frame.content_x,
                        frame.content_width,
                        frame.vertical.definite(),
                        frame.block.style().rtl,
                    )?;
                    frame.previous_break_after = BreakBetween::Auto;
                    frame.push(FlowItem::Paragraph(paragraph));
                }
                None => {
                    let frame = frames.pop().unwrap();
                    let restore = frame.restore_column_flow;
                    let measured = frame.finish(self)?;
                    self.column_flow = restore;
                    if let Some(parent) = frames.last_mut() {
                        parent.push_block(measured, false);
                    } else {
                        return Ok(measured);
                    }
                }
            }
        }
    }

    fn paragraph<'a>(
        &mut self,
        paragraph: &'a ferri_pdf_document::Paragraph,
        x: f32,
        width: f32,
        containing_height: Option<f32>,
        rtl: bool,
    ) -> Result<MeasuredParagraph<'a>, LayoutError> {
        self.work.spend(paragraph.text().len().max(1))?;
        let index = self.sources.paragraph(paragraph);
        let atomics = paragraph
            .atomics()
            .iter()
            .map(|atomic| {
                crate::layout::inline::measure(self, &atomic.block, width, containing_height, rtl)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let metrics: Vec<_> = atomics.iter().map(|atomic| atomic.metrics).collect();
        let prepared = self.text.prepare_paragraph(paragraph);
        let mut stream = prepared.lines(prepared.start(), width, &metrics)?;
        let mut lines = Vec::new();
        let mut positions = vec![stream.checkpoint()];
        while let Some(line) = stream.next() {
            self.work.spend(1)?;
            lines.push(line?);
            positions.push(stream.checkpoint());
        }
        let height = lines.iter().map(|line| line.height).sum();
        let anchors = line_anchors(paragraph, &lines, true);
        Ok(MeasuredParagraph {
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
            atomics: atomics.into_iter().map(Some).collect(),
        })
    }
}

pub(super) fn block_geometry(
    measurer: &Measurer<'_>,
    block: &BlockBox,
    x: f32,
    width: f32,
    rtl: bool,
    containing_height: Option<f32>,
) -> Result<crate::layout::geometry::HorizontalBox, LayoutError> {
    let used_width = block.image().map(|_| {
        crate::layout::images::size(block, width, width, containing_height, None, None)[0]
    });
    let style = block.style();
    let geometry = horizontal(style, x, width, rtl, used_width);
    if used_width.is_some()
        || ![&style.width, &style.min_width, &style.max_width]
            .iter()
            .any(|size| matches!(size, ferri_pdf_style::Size::Intrinsic(_)))
    {
        return Ok(geometry);
    }
    let edges = crate::layout::geometry::Edges::of(style, width).horizontal();
    let margins = [geometry.margin[3], geometry.margin[1]];
    let available =
        (width - edges - margins[0].unwrap_or(0.0) - margins[1].unwrap_or(0.0)).max(0.0);
    let constraints = crate::layout::intrinsic::width_constraints(
        block,
        measurer,
        Some(width),
        edges,
        Some(available),
    )?;
    let used = constraints.normal_flow(
        width,
        edges,
        margins,
        None,
        rtl,
        ferri_pdf_style::Overconstraint::AdjustEndMargin,
    );
    Ok(horizontal(style, x, width, rtl, Some(used.size)))
}

enum FlowStart<'a> {
    Complete(MeasuredBlock<'a>),
    Pending(FlowFrame<'a>),
}

struct FlowFrame<'a> {
    block: &'a BlockBox,
    geometry: crate::layout::geometry::HorizontalBox,
    root: bool,
    index: usize,
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
    outer: (Fragmentainer, Option<ColumnFlow>),
    restore_column_flow: Option<ColumnFlow>,
    previous_break_after: BreakBetween,
    children: std::slice::Iter<'a, BlockContent>,
    marker: Option<crate::layout::markers::MeasuredMarker>,
}
impl<'a> FlowFrame<'a> {
    fn push(&mut self, item: FlowItem<'a>) {
        let cleared = matches!(&item, FlowItem::Block(block) | FlowItem::Spanner(block) if block.clearance_target.is_some());
        if self.at_start && self.collapse_top && !cleared {
            self.before.merge(self.pending.clone().within(self.block));
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
    fn push_block(&mut self, mut block: MeasuredBlock<'a>, spanner: bool) {
        if block.style.clear != ferri_pdf_style::Clear::None || block.clearance_before.is_some() {
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
    fn finish(self, measurer: &mut Measurer<'_>) -> Result<MeasuredBlock<'a>, LayoutError> {
        let Self {
            block,
            geometry,
            root,
            index,
            containing_height,
            used_height,
            vertical,
            mut before,
            mut after,
            pending,
            mut items,
            at_start,
            top_edge,
            bottom_edge,
            independent,
            edges,
            mut has_forced,
            columns,
            outer,
            marker,
            ..
        } = self;
        let style = block.style();
        let clearance_target = measurer
            .clearance_targets
            .get(&std::ptr::from_ref::<BlockBox>(block))
            .copied();
        let through = clearance_target.is_none()
            && !root
            && !independent
            && at_start
            && top_edge == 0.0
            && bottom_edge == 0.0
            && vertical.collapse_through();
        if through {
            before.merge(pending.clone().within(block));
            before.merge(after);
            after = MarginStrut::default();
        } else if !root && !independent && bottom_edge == 0.0 && vertical.collapse_bottom() {
            let mut collapsed = pending.within(block);
            collapsed.merge(after);
            after = collapsed;
        } else {
            items.push(FlowItem::Margin(pending));
        }
        (measurer.fragmentainer, measurer.column_flow) = outer;
        if let Some(columns) = columns {
            let content = items.split_off(1);
            let columns = crate::layout::multicol::measure(
                measurer,
                columns,
                style,
                content,
                vertical.definite(),
            )?;
            has_forced = edges.iter().any(|edge| edge.is_forced()) || columns.forces_page;
            items.push(FlowItem::Columns(Box::new(columns)));
        }
        items.push(FlowItem::Edge(BlockEdge::End, bottom_edge));
        let mut height = items.iter().map(FlowItem::height).sum::<f32>();
        if !height.is_finite() {
            return Err(LayoutError::NonFiniteBlockGeometry { block: index });
        }
        if height < 0.0 {
            items.push(FlowItem::Space(-height));
            height = 0.0;
        }
        let natural_height = height;
        height = vertical.used_height(height);
        if !height.is_finite() {
            return Err(LayoutError::NonFiniteBlockGeometry { block: index });
        }
        let constrained = height != natural_height || vertical.definite().is_some();
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
        let (break_before, break_after) = crate::layout::breaks::boundaries(&items, edges);
        let paint = measurer.box_paint(block, &geometry);
        Ok(MeasuredBlock {
            grid: None,
            source: block,
            geometry: geometry.clone(),
            containing_height,
            offset: style.relative_offset(
                geometry.containing_width,
                containing_height,
                geometry.containing_rtl,
            ),
            style,
            canvas_source: block.is_canvas_source(),
            clearance_before: (style.clear != ferri_pdf_style::Clear::None || items.iter().any(|item| matches!(item, FlowItem::Block(child) | FlowItem::Spanner(child) if child.clearance_before.is_some()))).then(|| Box::new(MarginStrut::default())),
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
            marker: marker.map(Box::new),
            table: None,
            continues: false,
            spanner_fragment: None,
        })
    }
}

/// Static positions of a paragraph's absolutely positioned boxes (CSS 2.1 §10.3.7).
pub(super) fn line_anchors<'a>(
    paragraph: &'a ferri_pdf_document::Paragraph,
    lines: &[Line],
    complete: bool,
) -> Vec<LineAnchor<'a>> {
    let rtl = paragraph.style().rtl;
    paragraph
        .anchors()
        .iter()
        .filter(|anchor| anchor.block.style().floating().is_none())
        .filter(|anchor| {
            complete
                || lines.last().is_some_and(|line| {
                    if anchor.block.style().inline_level_origin {
                        anchor.offset < line.range.end
                    } else {
                        anchor.offset <= line.range.end
                    }
                })
        })
        .map(|anchor| {
            let block = &anchor.block;
            let last = lines.len().saturating_sub(1);
            if block.style().inline_level_origin {
                let line = lines
                    .iter()
                    .position(|line| anchor.offset < line.range.end)
                    .unwrap_or(last);
                LineAnchor {
                    block,
                    line,
                    x: lines.get(line).map(|line| line.x_at(anchor.offset, rtl)),
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
