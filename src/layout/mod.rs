//! Block measurement, page fragmentation, and positioned text fragments.
mod breaks;
mod flex;
mod floats;
mod geometry;
mod grid;
mod images;
mod inline;
mod intrinsic;
mod margin;
mod markers;
mod measure;
mod multicol;
mod navigation;
mod page_margins;
mod page_output;
mod page_paint;
mod pagination;
mod physical_pages;
mod positioned;
mod source;
mod strings;
mod tables;
mod work;
use crate::document::StyledDocument;
use crate::fonts::FontDescriptor;
use crate::graphics::{Color, DisplayItem, Rect};
use crate::text::{GlyphRun, TextContext, TextError};
use ferri_pdf_resources::ResourceResolver;
use serde::Serialize;

pub use navigation::{Destination, HeadingLocation, LinkArea, LinkShape};
pub(crate) use page_output::{PageOutput, PageRecord};

pub(crate) const MAX_LAYOUT_PAGES: usize = 10_000;
/// Aggregate layout-work units per job, shared by measurement and pagination probes.
/// Units count source visits, text bytes, grid slots and solver candidate items.
/// Exhaustion returns [`LayoutError::WorkLimit`]; this is not a time or memory limit.
pub(crate) const MAX_LAYOUT_WORK: usize = 100_000_000;

#[derive(Clone, Serialize)]
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
}

/// Resolves layout without replacing the caller's formatting or navigation metadata.
/// Use `layout_document` when serializing navigation with the resolved trace.
#[cfg(test)]
pub(crate) fn layout(
    document: &StyledDocument,
    resolver: &dyn ResourceResolver,
) -> Result<LayoutTrace, LayoutError> {
    let budget = work::Budget::new(MAX_LAYOUT_WORK);
    let LayoutPass { trace, generated } = layout_pass(document, resolver, budget.clone())?;
    let Some(values) = generated else {
        return Ok(trace);
    };
    budget.spend(1)?;
    let owned = document.fork_for_layout(values.clone(), resolver)?;
    stabilize(owned, resolver, &budget, vec![values]).map(|(_, trace)| trace)
}

pub(crate) fn layout_document(
    document: StyledDocument,
    resolver: &dyn ResourceResolver,
) -> Result<(StyledDocument, LayoutTrace), LayoutError> {
    stabilize(
        document,
        resolver,
        &work::Budget::new(MAX_LAYOUT_WORK),
        Vec::new(),
    )
}

struct LayoutPass {
    trace: LayoutTrace,
    generated: Option<crate::document::GeneratedValues>,
}

fn stabilize(
    mut document: StyledDocument,
    resolver: &dyn ResourceResolver,
    budget: &work::Budget,
    mut previous: Vec<crate::document::GeneratedValues>,
) -> Result<(StyledDocument, LayoutTrace), LayoutError> {
    loop {
        let LayoutPass { trace, generated } = layout_pass(&document, resolver, budget.clone())?;
        let Some(values) = generated else {
            return Ok((document, trace));
        };
        if previous.contains(&values) {
            return Err(LayoutError::GeneratedContentCycle);
        }
        previous.push(values.clone());
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
    let [page_width, page_height] = page.geometry.paper;
    let canvas_rect = page.geometry.content_box();
    let content_height = canvas_rect.height;
    let first_record = PageRecord::new(page)?;
    let page_output = first_record.output.clone();
    let painting =
        page_paint::PagePaint::new(page, document.canvas_background(), page_output.bleed);
    let mut measurer = measure::Measurer {
        work: budget,
        text: &text,
        canvas_images: document.canvas_background().backgrounds.has_images(),
        page_height: content_height,
        sources: source::SourceIds::new(document.root()),
        item_metrics: Default::default(),
        grid_parent: None,
        clearance_targets: Default::default(),
        fragmentainer: breaks::Fragmentainer::Page,
        column_flow: None,
        spanner: None,
        unbroken: false,
        named_pages: true,
    };
    let root = (document.root().style().display != crate::style::Display::None)
        .then(|| {
            measurer.block(
                document.root(),
                canvas_rect.x,
                canvas_rect.width,
                document.root().style().rtl,
                true,
                Some(content_height),
            )
        })
        .transpose()?;
    let pagination::Paginated {
        mut pages,
        mut display_lists,
        mut links,
        destinations,
        page_boxes,
        page_records,
        string_locations,
    } = pagination::Paginator::for_document(
        &mut measurer,
        physical_pages::PhysicalPages::new(document, resolver)?,
        painting.canvas.clone(),
        canvas_rect,
    )
    .document(
        root,
        document.navigation().destinations,
        document.root().leading_events(),
    )?;
    for ((list, page), record) in display_lists.iter_mut().zip(&page_boxes).zip(&page_records) {
        page_paint::PagePaint::new(page, document.canvas_background(), record.output.bleed)
            .decorate(std::slice::from_mut(list));
    }
    let strings = string_locations.resolve(document.running_strings(), pages.len())?;
    page_margins::render(
        &page_boxes,
        &strings,
        &measurer,
        measurer.sources.paragraph_count(),
        &mut pages,
        &mut display_lists,
    )?;
    for (areas, record) in links.iter_mut().zip(&page_records) {
        navigation::clip_links(areas, record.output.bleed);
    }
    let destination_pages: Vec<_> = destinations
        .iter()
        .map(|destination| destination.as_ref().map(|destination| destination.page + 1))
        .collect();
    let generated =
        document.resolve_generated(pages.len(), &destination_pages, &mut |context, request| {
            strings.at_context(context, request)
        })?;
    Ok(LayoutPass {
        generated,
        trace: LayoutTrace {
            page_width,
            page_height,
            page_area: canvas_rect,
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
        },
    })
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum LayoutError {
    #[error("generated content resolution repeats without reaching a fixed layout")]
    GeneratedContentCycle,
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
    #[error("unsupported table formatting at block {block}: {feature}")]
    UnsupportedTable { block: usize, feature: &'static str },
    #[error("{0}")]
    Flex(#[from] crate::flex::FlexError),
    #[error("unsupported flex formatting: {feature}")]
    UnsupportedFlex { feature: &'static str },
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
