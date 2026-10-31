//! Block measurement, page fragmentation, and positioned text fragments.
mod anchors;
mod breaks;
mod diagnostics;
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
mod scroll_markers;
mod source;
mod sticky;
mod strings;
mod tables;
mod viewport;
pub(crate) mod work;
mod writing;
use crate::document::StyledDocument;
use crate::fonts::FontDescriptor;
use crate::graphics::{Color, DisplayItem, Rect};
use crate::resources::ResourceResolver;
use crate::text::{GlyphRun, TextContext, TextError};
use serde::Serialize;

pub(crate) use diagnostics::Diagnostics;
pub use diagnostics::{Convergence, LayoutDiagnostic, WorkFallback};
pub use navigation::{Destination, HeadingLocation, LinkArea, LinkShape};
pub(crate) use page_output::{PageOutput, PageRecord};
pub(crate) use viewport::ViewportPaint;

pub(crate) const MAX_LAYOUT_PAGES: usize = 10_000;
/// Aggregate layout-work units per job, shared by measurement and pagination probes.
/// Units count source visits, text bytes, grid slots and solver candidate items.
/// Exhaustion returns [`LayoutError::WorkLimit`]; this is not a time or memory limit.
pub(crate) const MAX_LAYOUT_WORK: usize = 100_000_000;
/// Paginations of a layout pass that move calls off pages their footnotes do not fit.
const MAX_FOOTNOTE_PAGINATIONS: usize = 64;
/// Layout passes that size pages to the content the pass before measured.
const MAX_PAGE_SIZE_PASSES: usize = 16;
/// Boxes omitted one at a time where layout fails at them, before all content is omitted.
const MAX_OMITTED_BOXES: usize = 16;

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

/// The elements whose renderings `element()` images show, the document's root element, and
/// the renderings of those shown outside the document's flow, in footnote areas and
/// page-margin boxes; the last layout's renderings and the alpha its floats sampled of them,
/// and the alpha this layout's floats sample.
#[derive(Default)]
pub(crate) struct ElementTargets {
    pub targets: std::collections::HashSet<crate::dom::ElementId>,
    pub root: Option<crate::dom::ElementId>,
    pub displayed: std::cell::RefCell<pagination::Renderings>,
    pub previous: ElementRenderings,
    pub shapes: crate::document::ShapeSamples,
    pub sampled: std::cell::RefCell<crate::document::ShapeSamples>,
}
impl ElementTargets {
    /// The alpha of the last layout's rendering of `element` stretched over `samples` pixels,
    /// which a float's `shape-outside` reads (CSS Shapes 1 §6.1).
    pub(crate) fn alpha(
        &self,
        element: crate::dom::ElementId,
        samples: [usize; 2],
    ) -> Result<std::sync::Arc<[u8]>, LayoutError> {
        let key = (element, samples);
        let alpha = match self.shapes.get(&key) {
            Some(alpha) => alpha.clone(),
            None => crate::writer::rendering_alpha(element, samples, &self.previous)
                .map_err(|error| LayoutError::Raster(Box::new(error)))?
                .into(),
        };
        self.sampled.borrow_mut().insert(key, alpha.clone());
        Ok(alpha)
    }
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
        pages,
        scroll_markers,
        diagnostics: _,
    } = layout_pass(document, resolver, budget.clone())?;
    let markers_settled = document.scroll_markers_settled(&scroll_markers);
    let settled = document.styles().query_sizes().settled(&container_sizes)
        && document.styles().anchor_frames().settled(&anchor_frames)
        && document.element_images().settled(&elements)
        && *document.page_content() == pages
        && markers_settled;
    if generated.is_none() && settled {
        return Ok(trace);
    }
    if !markers_settled {
        document.measured_scroll_markers(scroll_markers);
    }
    let values = generated.unwrap_or_else(|| document.generated_values());
    budget.spend(1)?;
    let mut owned =
        document.fork_for_layout(values.clone(), container_sizes, anchor_frames, resolver)?;
    if !document.element_images().settled(&elements) {
        owned.measured_elements(elements, trace.element_images.clone());
        owned.reformat(values.clone(), resolver)?;
    }
    owned.measured_pages(pages)?;
    stabilize(&mut owned, resolver, &budget, vec![values])
}

pub(crate) fn layout_document(
    document: StyledDocument,
    resolver: &dyn ResourceResolver,
) -> Result<(StyledDocument, LayoutTrace), LayoutError> {
    layout_within(document, resolver, MAX_LAYOUT_WORK)
}

/// Lays `document` out with `work_limit` units of work for each attempt.
fn layout_within(
    mut document: StyledDocument,
    resolver: &dyn ResourceResolver,
    work_limit: usize,
) -> Result<(StyledDocument, LayoutTrace), LayoutError> {
    // A first layout pass that runs out of work completes no layout to keep, so the layouts
    // after it omit the content from the box it started with half its work spent, then a
    // quarter, and so on; one that fails at a box omits that box. The last omits all content.
    let mut marks: Option<Vec<_>> = None;
    let mut omitted_boxes = 0;
    let mut content = true;
    loop {
        let budget = work::Budget::new(work_limit);
        let error = match stabilize(&mut document, resolver, &budget, Vec::new()) {
            Err(error) if content => error,
            result => return Ok((document, result?)),
        };
        let diagnostic = if let LayoutError::WorkLimit { limit } = error {
            let marks = marks.get_or_insert_with(|| budget.marks());
            content = std::iter::from_fn(|| marks.pop())
                .flatten()
                .any(|mark| document.omit(mark, true));
            LayoutDiagnostic::WorkLimit {
                limit,
                kept: WorkFallback::Truncated,
            }
        } else if error.fails_at_box() {
            omitted_boxes += 1;
            content = omitted_boxes <= MAX_OMITTED_BOXES
                && error
                    .failed_box(budget.frontier())
                    .is_some_and(|failed| document.omit(failed, false));
            LayoutDiagnostic::OmittedBox {
                reason: error.to_string(),
            }
        } else {
            return Err(error);
        };
        if !content {
            document.omit_content();
        }
        document.record_layout([diagnostic]);
    }
}

struct LayoutPass {
    trace: LayoutTrace,
    generated: Option<crate::document::GeneratedValues>,
    /// The content-box size each query container measured.
    container_sizes:
        std::collections::HashMap<crate::dom::ElementId, crate::style::ContainerMeasurement>,
    anchor_frames: crate::style::anchor_values::AnchorFrameMap,
    /// What the layout measured of the elements that `element()` images show.
    elements: Box<crate::document::ElementMeasurements>,
    /// What the layout measured of the content of intrinsically sized pages.
    pages: crate::document::PageContent,
    /// Where each scroll marker lies relative to its group's active marker (CSS Overflow 5
    /// §3.1.8).
    scroll_markers: crate::dom::ScrollMarkerStates,
    diagnostics: Vec<LayoutDiagnostic>,
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
    let mut measured_pages = Vec::new();
    let mut measured_markers = Vec::new();
    let mut completed: Option<(LayoutTrace, Vec<LayoutDiagnostic>)> = None;
    loop {
        let pass = match (layout_pass(document, resolver, budget.clone()), completed) {
            (Err(LayoutError::WorkLimit { limit }), Some((trace, mut diagnostics))) => {
                diagnostics.push(LayoutDiagnostic::WorkLimit {
                    limit,
                    kept: WorkFallback::LastPass,
                });
                return Ok(kept(document, trace, diagnostics));
            }
            // Reformatting restores the boxes a layout omits, so a re-layout failing at one
            // keeps the layout before it.
            (Err(error), Some((trace, mut diagnostics))) if error.fails_at_box() => {
                diagnostics.push(LayoutDiagnostic::FailedRelayout {
                    reason: error.to_string(),
                });
                return Ok(kept(document, trace, diagnostics));
            }
            (result, _) => result?,
        };
        let LayoutPass {
            trace,
            generated,
            container_sizes,
            anchor_frames,
            elements,
            pages,
            scroll_markers,
            mut diagnostics,
        } = pass;
        // CSS Conditional 5 §5: each container's descendants are styled with the size of
        // the container they query, so the styles stand once a layout reproduces every size
        // they consulted.
        let containers_settled = document.styles().query_sizes().settled(&container_sizes);
        let anchors_settled = document.styles().anchor_frames().settled(&anchor_frames);
        // CSS Images 4 §2.7: `element()` images resolve against the elements a layout
        // rendered, so they stand once a layout reproduces every rendering they consulted.
        let elements_settled = document.element_images().settled(&elements);
        // CSS Page 3 §3: pages sized to their content stand once a layout reproduces the
        // content each holds.
        let pages_settled = *document.page_content() == pages;
        // CSS Overflow 5 §3.1.8: the active markers follow from where layout places their
        // targets, so their styles stand once a layout reproduces the markers they styled.
        let markers_settled = document.scroll_markers_settled(&scroll_markers);
        let settled = containers_settled
            && anchors_settled
            && elements_settled
            && pages_settled
            && markers_settled;
        if generated.is_none() && settled {
            return Ok(kept(document, trace, diagnostics));
        }
        // A measurement that repeats one an earlier layout styled with alternates without
        // end, so the layout just completed is kept.
        let cycle = if generated
            .as_ref()
            .is_some_and(|values| settled && previous.contains(values))
        {
            Some(Convergence::GeneratedContent)
        } else if !containers_settled && measured_sizes.contains(&container_sizes) {
            Some(Convergence::ContainerQueries)
        } else if !anchors_settled && measured_anchors.contains(&anchor_frames) {
            Some(Convergence::AnchorPositioning)
        } else if !elements_settled && measured_elements.contains(&elements) {
            Some(Convergence::ElementImages)
        } else if !pages_settled
            && (measured_pages.contains(&pages) || measured_pages.len() == MAX_PAGE_SIZE_PASSES)
        {
            Some(Convergence::PageSizes)
        } else if !markers_settled && measured_markers.contains(&scroll_markers) {
            Some(Convergence::ScrollMarkers)
        } else {
            None
        };
        if let Some(convergence) = cycle {
            diagnostics.push(LayoutDiagnostic::Unsettled(convergence));
            return Ok(kept(document, trace, diagnostics));
        }
        let values = match generated {
            None => document.generated_values(),
            Some(values) => {
                previous.push(values.clone());
                values
            }
        };
        if !containers_settled {
            measured_sizes.push(container_sizes.clone());
            document.measured_containers(container_sizes);
        }
        if !anchors_settled {
            measured_anchors.push(anchor_frames.clone());
            document.measured_anchors(anchor_frames);
        }
        if !elements_settled {
            measured_elements.push(elements.clone());
            document.measured_elements(elements, trace.element_images.clone());
        }
        if !pages_settled {
            measured_pages.push(pages.clone());
            document.measured_pages(pages)?;
        }
        if !markers_settled {
            measured_markers.push(scroll_markers.clone());
            document.measured_scroll_markers(scroll_markers);
        }
        if let Err(work::Exhausted { limit }) = budget.spend(1) {
            diagnostics.push(LayoutDiagnostic::WorkLimit {
                limit,
                kept: WorkFallback::LastPass,
            });
            return Ok(kept(document, trace, diagnostics));
        }
        document.reformat(values, resolver)?;
        completed = Some((trace, diagnostics));
    }
}

/// The layout `stabilize` keeps, with what its pass degraded retained by `document`.
fn kept(
    document: &mut StyledDocument,
    trace: LayoutTrace,
    diagnostics: Vec<LayoutDiagnostic>,
) -> LayoutTrace {
    document.record_layout(diagnostics);
    trace
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
    let [inline_axis, block_axis] = crate::document::page_axes(flow);
    // The initial containing block the document's styles took, which takes page sizes that
    // depend on content as automatic: orthogonal flows find their available space in it
    // (CSS Writing Modes 3 §7.3).
    let initial = sheet.page_area();
    let mut measurer = measure::Measurer::new(
        &text,
        document.root(),
        [initial[inline_axis], initial[block_axis]],
        budget,
    );
    measurer.page_height = content_height;
    for block in document
        .running_strings()
        .iter()
        .filter_map(|assignment| assignment.element.as_ref())
        .flat_map(crate::document::RunningElement::blocks)
        .chain(document.footnotes().bodies.iter().map(|body| &body.block))
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
                page.containing_block_size(flow, content_height),
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
    // CSS GCPM 3 §2.8: calls in nested layouts whose footnotes must stay with them and do
    // not fit move off their pages in a new pagination, until no call needs to move.
    let retained = (!continuous
        && document
            .footnotes()
            .elements
            .iter()
            .any(|footnote| footnote.policy != crate::style::FootnotePolicy::Auto))
    .then(|| Box::new(root.clone()));
    let mut paginated = paginate_flow(
        document,
        resolver,
        measurer,
        root,
        &painting.canvas,
        canvas_rect,
    )?;
    let mut paginations = 1;
    while !paginated.footnote_deferrals.is_empty() {
        if paginations == MAX_FOOTNOTE_PAGINATIONS {
            measurer
                .diagnostics
                .record(LayoutDiagnostic::Unsettled(Convergence::FootnoteDeferrals));
            break;
        }
        paginations += 1;
        std::rc::Rc::make_mut(&mut measurer.footnote_deferrals)
            .append(&mut paginated.footnote_deferrals);
        let root = retained
            .as_deref()
            .expect("only footnotes kept with their calls defer them")
            .clone();
        paginated = paginate_flow(
            document,
            resolver,
            measurer,
            root,
            &painting.canvas,
            canvas_rect,
        )?;
    }
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
        mut text_areas,
        mut renderings,
        footnote_deferrals: _,
        event_lines: _,
        inline_footnotes,
        page_extents,
        flow_end: _,
    } = paginated;
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
    let page_transforms: Vec<_> = page_boxes
        .iter()
        .map(|page| page_paint::page_transform(page))
        .collect();
    for (destination, bounds) in destinations.iter_mut().zip(destination_bounds) {
        if let (Some(destination), Some(bounds)) = (destination, bounds) {
            let frame = writing::Frame::physical(flow, page_boxes[destination.page].geometry.paper);
            let bounds = frame.rect(bounds);
            destination.top =
                page_transforms[destination.page]
                    .as_ref()
                    .map_or(bounds.y, |region| {
                        navigation::map_region(navigation::rect_corners(bounds).to_vec(), region)
                            .into_iter()
                            .map(|[_, y]| y)
                            .fold(f32::INFINITY, f32::min)
                    });
        }
    }
    let mut underlays = vec![Vec::new(); page_boxes.len()];
    let generated = paged_content(
        document,
        &page_boxes,
        (string_locations, inline_footnotes),
        &destinations,
        resolver,
        measurer,
        (
            &mut pages,
            &mut display_lists,
            &mut links,
            &mut text_areas,
            &mut underlays,
        ),
    )?;
    for ((((list, page), record), text_area), underlay) in display_lists
        .iter_mut()
        .zip(&page_boxes)
        .zip(&page_records)
        .zip(text_areas.iter_mut())
        .zip(underlays)
    {
        page_paint::PagePaint::new(page, document.canvas_background(), record.output.bleed)
            .decorate(list, text_area.take(), underlay);
    }
    for (((list, page), areas), transform) in display_lists
        .iter_mut()
        .zip(&page_boxes)
        .zip(links.iter_mut())
        .zip(&page_transforms)
    {
        list.extend(page_paint::page_outline(page));
        page_paint::composite(page, transform.as_ref(), list, areas);
    }
    for (areas, record) in links.iter_mut().zip(&page_records) {
        navigation::clip_links(areas, record.output.bleed);
    }
    anchors.physical(flow, &page_boxes);
    let scroll_markers = scroll_marker_states(document, &anchors, &page_boxes, flow);
    renderings.adopt(measurer.element_targets.displayed.take());
    let (elements, element_images) = element_renderings(
        document,
        &sheet.fonts,
        renderings,
        &measurer.element_targets.sampled.take(),
    )?;
    Ok(LayoutPass {
        pages: page_content(document, &page_boxes, page_extents, measurer)?,
        generated,
        elements,
        container_sizes: std::mem::take(&mut measurer.container_sizes),
        anchor_frames: anchors.frames(sheet, &document.svg_definitions(), &measurer.work)?,
        scroll_markers,
        diagnostics: measurer.diagnostics.take(),
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

/// Where each scroll marker lies relative to its group's active marker, as the layout
/// measured in `anchors` on `pages` placed scroll targets (CSS Overflow 5 §3.1.8).
fn scroll_marker_states(
    document: &StyledDocument,
    anchors: &anchors::Measurements,
    pages: &[std::sync::Arc<crate::document::PageBox>],
    flow: crate::style::logical::Flow,
) -> crate::dom::ScrollMarkerStates {
    let groups = document.scroll_marker_groups();
    if groups.is_empty() {
        return crate::dom::ScrollMarkerStates::new();
    }
    let areas = pages
        .iter()
        .map(|page| page.geometry.content_box())
        .collect();
    scroll_markers::Calculation::new(
        groups,
        &anchors.placed_boxes(),
        anchors.ink_areas(),
        (areas, flow),
        document.root_element(),
    )
    .states()
}

/// What a layout measured of the content of pages whose size properties take intrinsic size
/// keywords (CSS Page 3 §3), where its pages have them: the root's contributions in the
/// inline axis, and the content each page holds in the block axis.
fn page_content<'t>(
    document: &'t StyledDocument,
    page_boxes: &[std::sync::Arc<crate::document::PageBox>],
    page_extents: Vec<Option<f32>>,
    measurer: &mut measure::Measurer<'t>,
) -> Result<crate::document::PageContent, LayoutError> {
    let [inline_axis, block_axis] =
        crate::document::page_axes(crate::style::logical::Flow::used(document.root().style()));
    let content_sized = |axis: usize| {
        page_boxes
            .iter()
            .any(|page| page.geometry.intrinsic[axis].is_some())
    };
    // CSS Sizing 3 §5.2.1: the root's percentages, cyclic as the page area sizes to the root,
    // contribute as automatic sizes, measured in the initial containing block.
    let available = document.styles().page_area()[inline_axis];
    Ok(crate::document::PageContent {
        inline: content_sized(inline_axis)
            .then(|| {
                measurer.run(|measurer| intrinsic::measure(document.root(), measurer, available))
            })
            .transpose()?,
        blocks: if content_sized(block_axis) {
            page_extents
        } else {
            Vec::new()
        },
    })
}

/// Paginates the measured root once, with the footnote deferrals `measurer` holds.
fn paginate_flow<'t>(
    document: &'t StyledDocument,
    resolver: &'t dyn ResourceResolver,
    measurer: &mut measure::Measurer<'t>,
    root: Option<measure::MeasuredBlock<'t>>,
    canvas: &crate::graphics::BoxPaint,
    canvas_rect: Rect,
) -> Result<pagination::Paginated, LayoutError> {
    let paginator = pagination::Paginator::for_document(
        measurer,
        physical_pages::PhysicalPages::new(document, resolver)?,
        canvas.clone(),
        canvas_rect,
    );
    if document.styles().presentation() == crate::style::Presentation::Paged {
        paginator
    } else {
        paginator.unbroken()
    }
    .document(
        root,
        document.navigation().destinations,
        document.root().leading_events(),
    )
}

/// The elements whose renderings a layout of `document` records.
fn element_targets(document: &StyledDocument) -> std::rc::Rc<ElementTargets> {
    std::rc::Rc::new(ElementTargets {
        targets: document.element_images().targets(),
        root: Some(document.root_element()),
        displayed: std::cell::RefCell::default(),
        previous: document.element_renderings().clone(),
        shapes: document.element_images().shapes().clone(),
        sampled: std::cell::RefCell::default(),
    })
}

/// The states of the elements that `element()` images show, the renderings of those that
/// generate stacking contexts, drawn with the document's faces and SVG definitions, and the
/// alpha of these renderings where the layout's floats `sampled` the last layout's.
#[expect(
    clippy::boxed_local,
    reason = "layouts of nested navigables stack, so the layout frame holds the renderings boxed"
)]
fn element_renderings(
    document: &StyledDocument,
    fonts: &crate::fonts::FontBook,
    renderings: Box<pagination::Renderings>,
    sampled: &crate::document::ShapeSamples,
) -> Result<
    (
        Box<crate::document::ElementMeasurements>,
        Box<ElementRenderings>,
    ),
    LayoutError,
> {
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
    let paints: ElementRenderings = renderings
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
    // CSS Shapes 1 §6.1: a float's shape stands once the rendering it sampled stops changing;
    // an element no longer rendered changes its state instead.
    let shapes = sampled
        .keys()
        .filter(|(element, _)| paints.contains_key(element))
        .map(|&(element, samples)| {
            crate::writer::rendering_alpha(element, samples, &paints)
                .map(|alpha| ((element, samples), alpha.into()))
                .map_err(|error| LayoutError::Raster(Box::new(error)))
        })
        .collect::<Result<_, LayoutError>>()?;
    Ok((Box::new(images.measured(states, shapes)), Box::new(paints)))
}

/// Each page's text lines, display list, link areas, text area when its page box's backgrounds
/// clip to text, and the items painting behind its document canvas.
pub(super) type PageOutputs<'p> = (
    &'p mut [Vec<TextLine>],
    &'p mut [Vec<DisplayItem>],
    &'p mut [Vec<LinkArea>],
    &'p mut [Option<crate::graphics::FlowPaint>],
    &'p mut [Vec<DisplayItem>],
);

/// Paints the page-margin boxes and resolves the generated content that depends on where
/// pagination placed the document: its semantic events, and the inline formatting contexts of
/// its footnote areas. Kept apart from `layout_pass`, whose frame every
/// nested navigable's layout stacks.
fn paged_content<'t>(
    document: &'t StyledDocument,
    page_boxes: &[std::sync::Arc<crate::document::PageBox>],
    (string_locations, inline_footnotes): (
        strings::StringLocations,
        crate::document::InlineFootnotePlacement,
    ),
    destinations: &[Option<Destination>],
    resolver: &dyn ResourceResolver,
    measurer: &mut measure::Measurer<'t>,
    (pages, display_lists, links, text_areas, underlays): PageOutputs<'_>,
) -> Result<Option<crate::document::GeneratedValues>, LayoutError> {
    let counters = page_counters::PageCounters::new(page_boxes);
    let strings = string_locations.resolve(
        document.running_strings(),
        &counters,
        pages.len(),
        document.styles().degradations(),
    )?;
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
        (pages, display_lists, links, text_areas, underlays),
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
            inline_footnotes,
        },
    )?)
}

impl LayoutError {
    /// Whether the error follows from one box's geometry or structure, which layout cannot
    /// form.
    fn fails_at_box(&self) -> bool {
        if let Self::Text(error) = self {
            return matches!(
                **error,
                TextError::NonFiniteGeometry
                    | TextError::GlyphMatchingLimit { .. }
                    | TextError::BidiExpansionLimit { .. }
                    | TextError::CaseExpansionLimit
                    | TextError::LeaderExpansionLimit { .. }
            );
        }
        matches!(
            self,
            Self::NonFiniteBlockGeometry { .. }
                | Self::Table(_)
                | Self::Flex(_)
                | Self::Grid(_)
                | Self::UnsupportedFlex { .. }
                | Self::InvalidTableStructure
                | Self::InvalidGridStructure
                | Self::MissingClipFragment { .. }
                | Self::MissingContainingFragment { .. }
                | Self::Raster(_)
                | Self::Image(_)
        )
    }

    /// The box an error that [fails at a box](Self::fails_at_box) fails at: the one it names,
    /// else `frontier`, the box layout last started.
    fn failed_box(
        &self,
        frontier: Option<*const crate::document::BlockBox>,
    ) -> Option<*const crate::document::BlockBox> {
        match self {
            Self::NonFiniteBlockGeometry { block } => Some(*block),
            _ => frontier,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum LayoutError {
    #[error("{0}")]
    Raster(Box<crate::writer::PdfError>),
    #[error("non-finite page production geometry")]
    NonFinitePageOutputGeometry,
    #[error("non-finite page-margin geometry at {location:?}")]
    NonFinitePageMarginGeometry {
        location: crate::style::MarginLocation,
    },
    #[error("{0}")]
    Style(Box<crate::style::StyleError>),
    #[error("{0}")]
    Document(Box<crate::document::DocumentError>),
    #[error("{0}")]
    Image(Box<crate::images::ImageError>),
    /// Content needs a page past the page limit; pagination omits it.
    #[error("layout exceeds {limit} pages")]
    PageLimit { limit: usize },
    #[error("layout exceeds {limit} work units")]
    WorkLimit { limit: usize },
    #[error("block geometry is not finite")]
    NonFiniteBlockGeometry {
        block: *const crate::document::BlockBox,
    },
    #[error("{0}")]
    Text(Box<TextError>),
    #[error("{0}")]
    Table(Box<crate::table::TableError>),
    #[error("{0}")]
    Flex(Box<crate::flex::FlexError>),
    #[error("unsupported flex formatting: {feature}")]
    UnsupportedFlex { feature: &'static str },
    #[error("invalid normalized table structure")]
    InvalidTableStructure,
    #[error("{0}")]
    Grid(Box<crate::grid::GridError>),
    #[error("invalid normalized grid structure")]
    InvalidGridStructure,
    #[error("overflow clip has no box fragment on page {page}")]
    MissingClipFragment { page: usize },
    #[error("containing block has no fragment on anchor page {page}")]
    MissingContainingFragment { page: usize },
}
// Errors from other stages stay boxed: every `?` in nested layout's deep recursion holds
// layout errors in its stack frame.
macro_rules! boxed_layout_error {
    ($($variant:ident($error:ty)),* $(,)?) => { $(impl From<$error> for LayoutError {
        fn from(error: $error) -> Self { Self::$variant(Box::new(error)) }
    })* };
}
boxed_layout_error!(
    Style(crate::style::StyleError),
    Document(crate::document::DocumentError),
    Image(crate::images::ImageError),
    Text(TextError),
    Table(crate::table::TableError),
    Flex(crate::flex::FlexError),
    Grid(crate::grid::GridError),
);
