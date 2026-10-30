//! Block measurement, page fragmentation, and positioned text fragments.
mod anchors;
mod breaks;
mod flex;
mod float_area;
mod floats;
mod geometry;
mod grid;
mod images;
mod inline;
mod intrinsic;
mod margin;
mod markers;
mod mathml;
mod measure;
mod motion;
mod multicol;
mod navigation;
mod page_counters;
mod page_margins;
mod page_output;
mod page_paint;
mod pagination;
mod physical_pages;
mod positioned;
mod ratio;
mod source;
mod sticky;
mod strings;
mod tables;
mod viewport;
mod work;
mod writing;
use crate::document::StyledDocument;
use crate::fonts::FontDescriptor;
use crate::graphics::{Color, DisplayItem, Rect};
use crate::resources::ResourceResolver;
use crate::text::{GlyphRun, TextContext, TextError};
use serde::Serialize;

pub use navigation::{Destination, HeadingLocation, LinkArea, LinkShape};
pub(crate) use page_output::{PageOutput, PageRecord};
pub(crate) use viewport::ViewportPaint;

pub(crate) const MAX_LAYOUT_PAGES: usize = 10_000;
/// Aggregate layout-work units per job, shared by measurement and pagination probes.
/// Units count source visits, text bytes, grid slots and solver candidate items.
/// Exhaustion returns [`LayoutError::WorkLimit`]; this is not a time or memory limit.
pub(crate) const MAX_LAYOUT_WORK: usize = 100_000_000;

#[derive(Clone, Debug, Serialize)]
pub struct TextLine {
    pub paragraph: usize,
    pub text: String,
    pub x: f32,
    pub baseline: f32,
    pub width: f32,
    pub height: f32,
    pub runs: Vec<GlyphRun>,
    pub font_size: f32,
    pub color: Color,
}

/// The elements whose renderings `element()` images show, and the document's root element.
#[derive(Default)]
pub(crate) struct ElementTargets {
    pub targets: std::collections::HashSet<crate::dom::ElementId>,
    pub root: Option<crate::dom::ElementId>,
}

impl TextLine {
    /// The line drawn with every colour opaque, so that it draws the geometry of its text.
    pub(crate) fn opaque(mut self) -> Self {
        self.color.alpha = Color::BLACK.alpha;
        self.runs = self.runs.into_iter().map(GlyphRun::opaque).collect();
        self
    }
}

#[derive(Serialize)]
pub struct LayoutTrace {
    pub page_width: f32,
    pub page_height: f32,
    /// Page content area and positioning area of document `fixed` backgrounds.
    pub page_area: Rect,
    pub page_output: PageOutput,
    pub page_records: Vec<PageRecord>,
    pub pages: Vec<Vec<TextLine>>,
    pub fonts: Vec<FontDescriptor>,
    pub display_lists: Vec<Vec<DisplayItem>>,
    /// Hyperlink activation areas of each page.
    pub links: Vec<Vec<LinkArea>>,
    /// Where each of the document's destinations is, if its element has a box.
    pub destinations: Vec<Option<Destination>>,
    /// The renderings of the elements that `element()` images show (CSS Images 4 §2.7).
    #[serde(skip)]
    pub(crate) element_images: Box<ElementRenderings>,
}

/// The renderings of the elements that `element()` images show, by element. Layouts of
/// nested navigables stack, so their frames hold these boxed.
pub(crate) type ElementRenderings =
    std::collections::HashMap<crate::dom::ElementId, std::sync::Arc<ViewportPaint>>;

/// Resolves layout without replacing the caller's formatting or navigation metadata.
/// Use `layout_document` when serializing navigation with the resolved trace.
#[cfg(test)]
pub(crate) fn layout(
    document: &StyledDocument,
    resolver: &dyn ResourceResolver,
) -> Result<LayoutTrace, LayoutError> {
    let budget = work::Budget::new(MAX_LAYOUT_WORK);
    let LayoutPass {
        trace,
        generated,
        container_sizes,
        anchor_frames,
        elements,
    } = layout_pass(document, resolver, budget.clone())?;
    let settled = document.styles().query_sizes().settled(&container_sizes)
        && document.styles().anchor_frames().settled(&anchor_frames)
        && document.element_images().settled(&elements);
    if generated.is_none() && settled {
        return Ok(trace);
    }
    let values = generated.unwrap_or_else(|| document.generated_values());
    budget.spend(1)?;
    let mut owned =
        document.fork_for_layout(values.clone(), container_sizes, anchor_frames, resolver)?;
    if !document.element_images().settled(&elements) {
        owned.measured_elements(elements);
        owned.reformat(values.clone(), resolver)?;
    }
    stabilize(&mut owned, resolver, &budget, vec![values])
}

pub(crate) fn layout_document(
    mut document: StyledDocument,
    resolver: &dyn ResourceResolver,
) -> Result<(StyledDocument, LayoutTrace), LayoutError> {
    let trace = stabilize(
        &mut document,
        resolver,
        &work::Budget::new(MAX_LAYOUT_WORK),
        Vec::new(),
    )?;
    Ok((document, trace))
}

struct LayoutPass {
    trace: LayoutTrace,
    generated: Option<crate::document::GeneratedValues>,
    /// The content-box size each query container measured.
    container_sizes: std::collections::HashMap<crate::dom::ElementId, [f32; 2]>,
    anchor_frames: crate::style::anchor_values::AnchorFrameMap,
    /// What the layout measured of the elements that `element()` images show.
    elements: Box<crate::document::ElementMeasurements>,
}

fn stabilize(
    document: &mut StyledDocument,
    resolver: &dyn ResourceResolver,
    budget: &work::Budget,
    mut previous: Vec<crate::document::GeneratedValues>,
) -> Result<LayoutTrace, LayoutError> {
    let mut measured_sizes = Vec::new();
    let mut measured_anchors = Vec::new();
    let mut measured_elements = Vec::new();
    loop {
        let LayoutPass {
            trace,
            generated,
            container_sizes,
            anchor_frames,
            elements,
        } = layout_pass(document, resolver, budget.clone())?;
        // CSS Conditional 5 §5: each container's descendants are styled with the size of
        // the container they query, so the styles stand once a layout reproduces every size
        // they consulted.
        let containers_settled = document.styles().query_sizes().settled(&container_sizes);
        let anchors_settled = document.styles().anchor_frames().settled(&anchor_frames);
        // CSS Images 4 §2.7: `element()` images resolve against the elements a layout
        // rendered, so they stand once a layout reproduces every rendering they consulted.
        let elements_settled = document.element_images().settled(&elements);
        let settled = containers_settled && anchors_settled && elements_settled;
        let values = match generated {
            None if settled => return Ok(trace),
            None => document.generated_values(),
            Some(values) => {
                if settled && previous.contains(&values) {
                    return Err(LayoutError::GeneratedContentCycle);
                }
                previous.push(values.clone());
                values
            }
        };
        if !containers_settled {
            if measured_sizes.contains(&container_sizes) {
                return Err(LayoutError::ContainerQueryCycle);
            }
            measured_sizes.push(container_sizes.clone());
            document.measured_containers(container_sizes);
        }
        if !anchors_settled {
            if measured_anchors.contains(&anchor_frames) {
                return Err(LayoutError::AnchorPositioningCycle);
            }
            measured_anchors.push(anchor_frames.clone());
            document.measured_anchors(anchor_frames);
        }
        if !elements_settled {
            if measured_elements.contains(&elements) {
                return Err(LayoutError::ElementImageCycle);
            }
            measured_elements.push(elements.clone());
            document.measured_elements(elements);
        }
        budget.spend(1)?;
        document.reformat(values, resolver)?;
    }
}

#[cfg(test)]
fn layout_with_work_limit(
    document: &StyledDocument,
    resolver: &dyn ResourceResolver,
    work_limit: usize,
) -> Result<LayoutTrace, LayoutError> {
    layout_pass(document, resolver, work::Budget::new(work_limit)).map(|pass| pass.trace)
}

fn layout_pass(
    document: &StyledDocument,
    resolver: &dyn ResourceResolver,
    budget: work::Budget,
) -> Result<LayoutPass, LayoutError> {
    let sheet = document.styles();
    let text = TextContext::new(&sheet.fonts, resolver);
    let page = document.page_box();
    let flow = crate::style::logical::Flow::used(document.root().style());
    let mut logical_geometry = page.geometry.clone();
    writing::Frame::physical(flow, page.geometry.paper)
        .inverse()
        .geometry(&mut logical_geometry);
    let canvas_rect = logical_geometry.content_box();
    let content_height = canvas_rect.height;
    let continuous = sheet.presentation() != crate::style::Presentation::Paged;
    let mut measurer = measure::Measurer::new(
        &text,
        document.root(),
        [canvas_rect.width, content_height],
        budget,
    );
    for block in document
        .running_strings()
        .iter()
        .filter_map(|assignment| assignment.element.as_ref())
        .flat_map(crate::document::RunningElement::blocks)
        .chain(document.footnotes())
    {
        measurer.sources.register(block);
    }
    measurer.canvas_images = document.canvas_background().backgrounds.has_images();
    measurer.element_targets = element_targets(document);
    measurer.unbroken = continuous;
    measurer.named_pages = !continuous;
    measurer.motion = std::rc::Rc::new(motion::Resources::new(
        document.svg_definitions(),
        &sheet.fonts,
    ));
    let root = (document.root().style().display != crate::style::Display::None)
        .then(|| {
            measurer.block(
                document.root(),
                canvas_rect.x,
                canvas_rect.width,
                document.root().style().used_rtl(),
                true,
                Some(content_height),
            )
        })
        .transpose()?;
    paginate(document, resolver, &mut measurer, root, canvas_rect)
}

/// Fragments the measured root into pages and paints them. Kept apart from `layout_pass`,
/// whose frame every nested navigable's layout stacks.
fn paginate<'t>(
    document: &'t StyledDocument,
    resolver: &'t dyn ResourceResolver,
    measurer: &mut measure::Measurer<'t>,
    root: Option<measure::MeasuredBlock<'t>>,
    canvas_rect: Rect,
) -> Result<LayoutPass, LayoutError> {
    let sheet = document.styles();
    let page = document.page_box();
    let [page_width, page_height] = page.geometry.paper;
    let flow = crate::style::logical::Flow::used(document.root().style());
    let continuous = sheet.presentation() != crate::style::Presentation::Paged;
    let page_output = PageRecord::new(page)?.output;
    let painting = page_paint::PagePaint::in_flow(page, document.canvas_background(), flow)?;
    let paginator = pagination::Paginator::for_document(
        measurer,
        physical_pages::PhysicalPages::new(document, resolver)?,
        painting.canvas.clone(),
        canvas_rect,
    );
    let pagination::Paginated {
        mut anchors,
        mut pages,
        mut display_lists,
        mut links,
        mut destinations,
        destination_bounds,
        page_boxes,
        mut page_records,
        string_locations,
        line_boxes: _,
        root_text_area: _,
        renderings,
    } = if continuous {
        paginator.unbroken()
    } else {
        paginator
    }
    .document(
        root,
        document.navigation().destinations,
        document.root().leading_events(),
    )?;
    for (index, page) in page_boxes.iter().enumerate() {
        let frame = writing::Frame::physical(flow, page.geometry.paper);
        for area in &mut links[index] {
            match &mut area.shape {
                LinkShape::Rect(rect) => *rect = frame.rect(*rect),
                LinkShape::Quads(quads) => {
                    for point in quads.iter_mut().flatten() {
                        *point = frame.point(*point);
                    }
                }
            }
        }
        page_records[index] = PageRecord::new(page)?;
    }
    for (destination, bounds) in destinations.iter_mut().zip(destination_bounds) {
        if let (Some(destination), Some(bounds)) = (destination, bounds) {
            let frame = writing::Frame::physical(flow, page_boxes[destination.page].geometry.paper);
            destination.top = frame.rect(bounds).y;
        }
    }
    for ((list, page), record) in display_lists.iter_mut().zip(&page_boxes).zip(&page_records) {
        page_paint::PagePaint::new(page, document.canvas_background(), record.output.bleed)
            .decorate(std::slice::from_mut(list));
    }
    let generated = paged_content(
        document,
        &page_boxes,
        string_locations,
        &destinations,
        resolver,
        measurer,
        (&mut pages, &mut display_lists, &mut links),
    )?;
    for (list, page) in display_lists.iter_mut().zip(&page_boxes) {
        list.extend(page_paint::page_outline(page));
    }
    for (areas, record) in links.iter_mut().zip(&page_records) {
        navigation::clip_links(areas, record.output.bleed);
    }
    anchors.physical(flow, &page_boxes);
    let (elements, element_images) = element_renderings(document, &sheet.fonts, renderings);
    Ok(LayoutPass {
        generated,
        elements,
        container_sizes: std::mem::take(&mut measurer.container_sizes),
        anchor_frames: anchors.frames(sheet, &document.svg_definitions(), &measurer.work)?,
        trace: LayoutTrace {
            page_width,
            page_height,
            page_area: page.geometry.content_box(),
            page_output,
            page_records,
            pages,
            display_lists,
            links,
            destinations,
            fonts: sheet
                .fonts
                .faces()
                .iter()
                .map(|font| font.descriptor().clone())
                .collect(),
            element_images,
        },
    })
}

/// The elements whose renderings a layout of `document` records.
fn element_targets(document: &StyledDocument) -> std::rc::Rc<ElementTargets> {
    std::rc::Rc::new(ElementTargets {
        targets: document.element_images().targets(),
        root: Some(document.root_element()),
    })
}

/// The states of the elements that `element()` images show, and the renderings of those
/// that generate stacking contexts, drawn with the document's faces and SVG definitions.
#[expect(
    clippy::boxed_local,
    reason = "layouts of nested navigables stack, so the layout frame holds the renderings boxed"
)]
fn element_renderings(
    document: &StyledDocument,
    fonts: &crate::fonts::FontBook,
    renderings: Box<pagination::Renderings>,
) -> (
    Box<crate::document::ElementMeasurements>,
    Box<ElementRenderings>,
) {
    use crate::document::ElementState;
    let images = document.element_images();
    let states = images
        .targets()
        .into_iter()
        .map(|element| {
            let state = if let Some(rendering) = renderings.stacked.get(&element) {
                ElementState::Stacked(
                    rendering
                        .size
                        .map(|extent| extent / crate::style::POINTS_PER_PIXEL),
                )
            } else if renderings.unstacked.contains(&element) {
                ElementState::Unstacked
            } else {
                ElementState::NotRendered
            };
            (element, state)
        })
        .collect();
    let definitions = document.svg_definitions();
    let paints = renderings
        .stacked
        .into_iter()
        .map(|(element, rendering)| {
            let [width, height] = rendering.size;
            (
                element,
                std::sync::Arc::new(ViewportPaint {
                    items: rendering.items,
                    lines: Vec::new(),
                    fonts: fonts.clone(),
                    definitions: definitions.clone(),
                    area: Rect {
                        x: 0.0,
                        y: 0.0,
                        width,
                        height,
                    },
                    elements: Box::default(),
                }),
            )
        })
        .collect();
    (Box::new(images.measured(states)), Box::new(paints))
}

/// Each page's text lines, display list and link areas.
type PageOutputs<'p> = (
    &'p mut [Vec<TextLine>],
    &'p mut [Vec<DisplayItem>],
    &'p mut [Vec<LinkArea>],
);

/// Paints the page-margin boxes and resolves the generated content that depends on where
/// pagination placed the document. Kept apart from `layout_pass`, whose frame every nested
/// navigable's layout stacks.
fn paged_content<'t>(
    document: &'t StyledDocument,
    page_boxes: &[std::sync::Arc<crate::document::PageBox>],
    string_locations: strings::StringLocations,
    destinations: &[Option<Destination>],
    resolver: &dyn ResourceResolver,
    measurer: &mut measure::Measurer<'t>,
    (pages, display_lists, links): PageOutputs<'_>,
) -> Result<Option<crate::document::GeneratedValues>, LayoutError> {
    let counters = page_counters::PageCounters::new(page_boxes);
    let strings = string_locations.resolve(document.running_strings(), &counters, pages.len())?;
    let first_margin_paragraph = measurer.sources.paragraph_count();
    page_margins::render(
        page_boxes,
        &page_margins::MarginContext {
            strings: &strings,
            running: document.running_strings(),
            destinations: document.navigation().destinations,
            counters: &counters,
            document,
            resolver,
        },
        measurer,
        first_margin_paragraph,
        pages,
        display_lists,
        links,
    )?;
    let destination_pages: Vec<_> = destinations
        .iter()
        .map(|destination| destination.as_ref().map(|destination| destination.page))
        .collect();
    Ok(document.resolve_generated(
        &destination_pages,
        crate::document::Paginated {
            pages: pages.len(),
            numbers: &|page| counters.page(page, None),
            context_page: &|context| strings.page_of(context),
            strings: &mut |context, request| strings.at_context(context, request),
            starts: counters.starts(page_boxes, strings.page_starts(pages.len())),
            running: page_margins::running(
                page_boxes,
                document.running_strings(),
                &strings,
                &counters,
            ),
        },
    )?)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum LayoutError {
    #[error("CSS anchor positioning did not stabilize")]
    AnchorPositioningCycle,
    #[error("generated content resolution repeats without reaching a fixed layout")]
    GeneratedContentCycle,
    #[error("container query sizes repeat without reaching a fixed layout")]
    ContainerQueryCycle,
    #[error("element() image renderings repeat without reaching a fixed layout")]
    ElementImageCycle,
    #[error("non-finite page production geometry")]
    NonFinitePageOutputGeometry,
    #[error("non-finite page-margin geometry at {location:?}")]
    NonFinitePageMarginGeometry {
        location: crate::style::MarginLocation,
    },
    #[error("{0}")]
    Style(#[from] crate::style::StyleError),
    #[error("{0}")]
    Document(#[from] crate::document::DocumentError),
    #[error("{0}")]
    Image(#[from] crate::images::ImageError),
    #[error("layout exceeds {limit} pages")]
    PageLimit { limit: usize },
    #[error("layout exceeds {limit} work units")]
    WorkLimit { limit: usize },
    #[error("non-finite geometry at block {block}")]
    NonFiniteBlockGeometry { block: usize },
    #[error("{0}")]
    Text(#[from] TextError),
    #[error("{0}")]
    Table(#[from] crate::table::TableError),
    #[error("{0}")]
    Flex(#[from] crate::flex::FlexError),
    #[error("unsupported flex formatting: {feature}")]
    UnsupportedFlex { feature: &'static str },
    #[error("unsupported footnote {footnote}: {feature}")]
    UnsupportedFootnote {
        footnote: usize,
        feature: &'static str,
    },
    #[error("unsupported initial letter: {feature}")]
    UnsupportedInitialLetter { feature: &'static str },
    #[error("invalid normalized table structure")]
    InvalidTableStructure,
    #[error("{0}")]
    Grid(#[from] crate::grid::GridError),
    #[error("invalid normalized grid structure")]
    InvalidGridStructure,
    #[error("overflow clip has no box fragment on page {page}")]
    MissingClipFragment { page: usize },
    #[error("containing block has no fragment on anchor page {page}")]
    MissingContainingFragment { page: usize },
    #[error("fixed boxes repeated on every page exceed {limit} placements")]
    FixedPlacementLimit { limit: usize },
}
