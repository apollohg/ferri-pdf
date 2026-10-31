//! PDF serialization of ordered display lists; no document parsing or layout.
mod backdrops;
mod backgrounds;
mod bounds;
mod cross_fade;
mod display;
mod filters;
mod images;
mod masks;
mod navigation;
mod painting;
mod pixels;
mod projection;
mod raster;
pub(crate) use raster::rendering_alpha;
mod scene;
mod shadows;
#[cfg(test)]
mod tests;
pub(crate) mod text;
mod text_shadows;
use crate::document::{DocumentMetadata, Navigation};
use crate::fonts::{FontBook, FontId};

use crate::layout::LayoutTrace;
use crate::style::PageOrientation;
use krilla::{
    Document,
    page::{PageRotation, PageSettings},
};

pub use krilla::image::{ImageDpi, InvalidImageDpi, InvalidJpegQuality, JpegQuality};

/// Optional raster encoding controls; defaults retain source encoding and precision.
#[derive(Clone, Debug, Default)]
pub struct ImageOptions {
    /// Encode JPEG and opaque decoded photos at this quality, converting high precision to eight bits.
    /// Transparent samples stay lossless; optional encoding failures retain the existing representation.
    pub jpeg_quality: Option<JpegQuality>,
    /// Target physical resolution, retaining the largest demand across shared uses.
    /// Original samples are retained if the resampling budget is exceeded.
    pub dpi: Option<ImageDpi>,
}

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("a PDF rendering job requires at least one part")]
    EmptyParts,
    #[error("image serialization: {0}")]
    Image(String),
    #[error("font is not a valid supported OpenType font")]
    InvalidFont,
    #[error("font ID {0:?} is absent from the rendering job")]
    InvalidFontId(FontId),
    #[error("invalid PDF page dimensions: {width} by {height}")]
    InvalidPageSize { width: f32, height: f32 },
    #[error("layout has {pages} text pages and {display_lists} display lists")]
    DisplayListPageCount { pages: usize, display_lists: usize },
    #[error("layout has {pages} text pages and link areas for {link_pages}")]
    LinkPageCount { pages: usize, link_pages: usize },
    #[error("layout has {pages} text pages and {records} physical page records")]
    PageRecordCount { pages: usize, records: usize },
    #[error("display list on page {page} refers to absent text line {line}")]
    InvalidTextReference { page: usize, line: usize },
    #[error("{0}")]
    UnresolvedImage(#[source] crate::graphics::UnresolvedImage),
    #[error("invalid paint geometry")]
    InvalidGeometry,
    #[error("invalid paint alpha {0}")]
    InvalidAlpha(f32),
    #[error("a {width} by {height} pixel filter raster exceeds the rasterizer's limit")]
    RasterSize { width: f32, height: f32 },
    #[error("rasterizing a filtered group failed")]
    Raster,
    /// Compositing 2 §9.1.14 `plus-darker` and §9.1.15 `plus-lighter` have no PDF blend mode; only display-list groups,
    /// which composite it in pixels, may use it.
    #[error("additive compositing outside a display-list group")]
    VectorAdditive,
    #[error("mask or filter effect has no reference box")]
    UnplacedEffect,
    #[error("a background-clip: text layer has no text area")]
    MissingTextArea,
    #[error("an element() image names an element the layout did not render")]
    MissingElementRendering,
    /// The CSS mask layers of SVG content draw through the display list that places it.
    #[error("SVG content with CSS mask layers drawn outside a display list")]
    DetachedSvgMasks,
    #[error("{0}")]
    Serialization(#[from] krilla::error::KrillaError),
}

impl PdfError {
    /// Whether the error breaks an invariant of the layout being written, rather than
    /// following from what the document asks to paint.
    pub(super) fn is_internal(&self) -> bool {
        matches!(
            self,
            Self::EmptyParts
                | Self::InvalidFontId(_)
                | Self::DisplayListPageCount { .. }
                | Self::LinkPageCount { .. }
                | Self::PageRecordCount { .. }
                | Self::InvalidTextReference { .. }
        )
    }
}

/// A painting degradation retained with the rendering result.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PaintDiagnostic {
    /// A display item whose geometry or effect could not be drawn is omitted with the
    /// content it groups.
    #[error("a painted item is omitted: {reason}")]
    Omitted { reason: String },
    /// Shared `mask` references multiply with each level of nesting; past a page's budget,
    /// each draws as a transparent black layer.
    #[error("mask elements drawn past {limit} on a page are transparent black layers")]
    MaskElementDrawLimit { limit: usize },
    /// A mask image layer whose raster exceeds the rasterizer's limits at the drawing's
    /// resolution is drawn at the highest resolution within them.
    #[error("a mask image too large to rasterize is drawn at a lower resolution")]
    ReducedResolution,
    /// A page larger than a PDF describes is cut to `limit` points along each side.
    #[error("pages larger than {limit}pt along a side are cut to it")]
    PageSize { limit: f32 },
}

/// The largest page side a PDF describes, in points; larger pages are cut to it.
const MAX_PAGE_SIDE_PT: f32 = 1.0e7;

/// `rect` cut to the area a page within [`MAX_PAGE_SIDE_PT`] and its bleed can describe.
fn within_page_limits(rect: crate::graphics::Rect) -> crate::graphics::Rect {
    let [left, top] = [rect.x, rect.y].map(|edge| edge.max(-MAX_PAGE_SIDE_PT));
    let [right, bottom] =
        [rect.x + rect.width, rect.y + rect.height].map(|edge| edge.min(MAX_PAGE_SIDE_PT));
    crate::graphics::Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

/// An independently laid-out document to append to a shared PDF.
pub(crate) struct PdfPart<'a> {
    pub trace: &'a LayoutTrace,
    pub fonts: &'a FontBook,
    pub navigation: &'a Navigation,
    pub metadata: &'a DocumentMetadata,
    pub definitions: &'a crate::document::svg::SvgDefinitions,
}

/// PDF bytes and the recoverable image-decoding and painting diagnostics of each input part.
pub(crate) struct EncodedPdf {
    pub pdf: Vec<u8>,
    pub diagnostics: Vec<Vec<crate::document::DocumentDiagnostic>>,
}

#[cfg(test)]
pub(crate) fn encode(
    trace: &LayoutTrace,
    fonts: &FontBook,
    navigation: &Navigation,
    metadata: &DocumentMetadata,
    images: &ImageOptions,
) -> Result<Vec<u8>, PdfError> {
    Ok(encode_with_diagnostics(
        &PdfPart {
            trace,
            fonts,
            navigation,
            metadata,
            definitions: &crate::document::svg::SvgDefinitions::default(),
        },
        images,
    )?
    .pdf)
}

pub(crate) fn encode_with_diagnostics(
    part: &PdfPart<'_>,
    images: &ImageOptions,
) -> Result<EncodedPdf, PdfError> {
    encode_parts_with_diagnostics(std::slice::from_ref(part), images)
}

fn validate(trace: &LayoutTrace) -> Result<(), PdfError> {
    if trace.pages.len() != trace.display_lists.len() {
        return Err(PdfError::DisplayListPageCount {
            pages: trace.pages.len(),
            display_lists: trace.display_lists.len(),
        });
    }
    if trace.links.len() != trace.pages.len() {
        return Err(PdfError::LinkPageCount {
            pages: trace.pages.len(),
            link_pages: trace.links.len(),
        });
    }
    if trace.page_records.len() != trace.pages.len() {
        return Err(PdfError::PageRecordCount {
            pages: trace.pages.len(),
            records: trace.page_records.len(),
        });
    }
    Ok(())
}

/// Append independent layouts with common output settings.
/// Cross-part links require navigation captured with `ParseOptions::capture_fragment_targets`.
pub(crate) fn encode_parts_with_diagnostics(
    parts: &[PdfPart<'_>],
    images: &ImageOptions,
) -> Result<EncodedPdf, PdfError> {
    let first = parts.first().ok_or(PdfError::EmptyParts)?;
    for part in parts {
        validate(part.trace)?;
    }
    let settings_dpi = images.dpi;
    let mut document = Document::new_with(krilla::SerializeSettings {
        jpeg_quality: images.jpeg_quality,
        image_dpi: images.dpi,
        ..Default::default()
    });
    document.set_metadata(navigation::metadata(first.metadata));
    let mut page_offset = 0;
    let destinations: Vec<_> = parts
        .iter()
        .map(|part| {
            let result = navigation::destinations(&part.trace.destinations, page_offset);
            page_offset += part.trace.pages.len();
            result
        })
        .collect();
    let links = navigation::resolve_links(parts, &destinations);
    let mut images = images::ImageWriter::new(raster::pixels_per_point(settings_dpi));
    let mut diagnostics = Vec::with_capacity(parts.len());
    let mut outline = krilla::outline::Outline::new();
    for (part_index, part) in parts.iter().enumerate() {
        let trace = part.trace;
        let mut text = text::TextWriter::new(part.fonts);
        let raster = raster::RasterContext::new(settings_dpi, part.definitions);
        images.elements.extend(
            trace
                .element_images
                .iter()
                .map(|(element, rendering)| (*element, rendering.clone())),
        );
        for (page_index, (lines, items)) in trace.pages.iter().zip(&trace.display_lists).enumerate()
        {
            let record = &trace.page_records[page_index];
            let [width, height] = record.geometry.paper.map(|side| side.min(MAX_PAGE_SIDE_PT));
            if [width, height] != record.geometry.paper {
                images.record(PaintDiagnostic::PageSize {
                    limit: MAX_PAGE_SIDE_PT,
                });
            }
            let page_area = record.geometry.content_box();
            let settings = PageSettings::from_wh(width, height)
                .ok_or(PdfError::InvalidPageSize { width, height })?;
            let production_rect = |rect: crate::graphics::Rect| {
                let rect = within_page_limits(rect);
                krilla::geom::Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
                    .ok_or(PdfError::InvalidGeometry)
            };
            let settings = settings
                .with_media_box(Some(production_rect(record.output.media)?))
                .with_trim_box(Some(production_rect(record.output.trim)?))
                .with_bleed_box(Some(production_rect(record.output.bleed)?))
                .with_rotation(match record.output.orientation {
                    PageOrientation::Upright => PageRotation::Upright,
                    PageOrientation::RotateLeft => PageRotation::CounterClockwise,
                    PageOrientation::RotateRight => PageRotation::Clockwise,
                });
            images.mask_element_draws = 0;
            let mut page = document.start_page_with(settings);
            let mut surface = page.surface();
            let bleed_clip = painting::rect_path(within_page_limits(record.output.bleed))?;
            surface.push_clip_path(&bleed_clip, &krilla::paint::FillRule::NonZero);
            let painted = display::Painter {
                lines,
                page: page_index,
                page_area,
                text: &mut text,
                fonts: part.fonts,
                images: &mut images,
                raster: &raster,
                prepared: backdrops::Prepared::default(),
            }
            .draw(
                items,
                display::Space {
                    viewport: record.output.media,
                    scale: 1.0,
                },
                &mut surface,
            );
            surface.pop();
            if painted.is_ok() {
                for mark in &record.output.marks {
                    painting::draw(
                        mark,
                        page_area,
                        record.output.media,
                        &mut surface,
                        &mut images,
                        &mut |_, _, _| Err(PdfError::MissingTextArea),
                    )?;
                }
            }
            surface.finish();
            if let Err(error) = painted {
                page.finish();
                return Err(error);
            }
            navigation::annotate(&mut page, &trace.links[page_index], &links[part_index])?;
            page.finish();
        }
        diagnostics.push(images.take_diagnostics());
        navigation::append_outline(
            &mut outline,
            &part.navigation.bookmarks,
            &destinations[part_index],
        );
    }
    drop(images);
    document.set_outline(outline);
    Ok(EncodedPdf {
        pdf: document.finish()?,
        diagnostics,
    })
}
