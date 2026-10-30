//! PDF serialization of ordered display lists; no document parsing or layout.
mod backdrops;
mod backgrounds;
mod cross_fade;
mod display;
mod filters;
mod images;
mod masks;
mod navigation;
mod painting;
mod pixels;
mod raster;
mod shadows;
#[cfg(test)]
mod tests;
mod text;
mod text_shadows;
use crate::document::{DocumentMetadata, Navigation};
use crate::fonts::{FontBook, FontId};

use crate::layout::LayoutTrace;
use krilla::{Document, page::PageSettings};

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
    #[error("{0}")]
    Svg(#[source] crate::images::ImageError),
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
    #[error("{0}")]
    Serialization(#[from] krilla::error::KrillaError),
}

/// An independently laid-out document to append to a shared PDF.
pub(crate) struct PdfPart<'a> {
    pub trace: &'a LayoutTrace,
    pub fonts: &'a FontBook,
    pub navigation: &'a Navigation,
    pub metadata: &'a DocumentMetadata,
    pub definitions: &'a crate::document::svg::SvgDefinitions,
}

/// PDF bytes and recoverable image-decoding diagnostics for each input part.
pub(crate) struct EncodedPdf {
    pub pdf: Vec<u8>,
    pub image_diagnostics: Vec<Vec<String>>,
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
    let mut image_diagnostics = Vec::with_capacity(parts.len());
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
            let [width, height] = record.geometry.paper;
            let page_area = record.geometry.content_box();
            let settings = PageSettings::from_wh(width, height)
                .ok_or(PdfError::InvalidPageSize { width, height })?;
            let production_rect = |rect: crate::graphics::Rect| {
                krilla::geom::Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
                    .ok_or(PdfError::InvalidGeometry)
            };
            let settings = settings
                .with_media_box(Some(production_rect(record.output.media)?))
                .with_trim_box(Some(production_rect(record.output.trim)?))
                .with_bleed_box(Some(production_rect(record.output.bleed)?));
            let mut page = document.start_page_with(settings);
            let mut surface = page.surface();
            let bleed_clip = painting::rect_path(record.output.bleed)?;
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
        image_diagnostics.push(images.take_diagnostics());
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
        image_diagnostics,
    })
}
