//! PDF serialization of ordered display lists; no document parsing or layout.
mod backgrounds;
mod images;
mod navigation;
mod painting;
mod pixels;
mod shadows;
mod text;
use ferri_pdf_document::{DocumentMetadata, Navigation};
use ferri_pdf_fonts::{FontBook, FontId};
use ferri_pdf_graphics::DisplayItem;
use ferri_pdf_layout::LayoutTrace;
use krilla::{Document, geom::Transform, page::PageSettings};

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
    Svg(#[source] ferri_pdf_images::ImageError),
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
    UnresolvedImage(#[source] ferri_pdf_graphics::UnresolvedImage),
    #[error("invalid paint geometry")]
    InvalidGeometry,
    #[error("invalid paint alpha {0}")]
    InvalidAlpha(f32),
    #[error("{0}")]
    Serialization(#[from] krilla::error::KrillaError),
}

/// An independently laid-out document to append to a shared PDF.
pub(crate) struct PdfPart<'a> {
    pub trace: &'a LayoutTrace,
    pub fonts: &'a FontBook,
    pub navigation: &'a Navigation,
    pub metadata: &'a DocumentMetadata,
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
    Ok(encode_with_diagnostics(trace, fonts, navigation, metadata, images)?.pdf)
}

pub(crate) fn encode_with_diagnostics(
    trace: &LayoutTrace,
    fonts: &FontBook,
    navigation: &Navigation,
    metadata: &DocumentMetadata,
    images: &ImageOptions,
) -> Result<EncodedPdf, PdfError> {
    encode_parts_with_diagnostics(
        &[PdfPart {
            trace,
            fonts,
            navigation,
            metadata,
        }],
        images,
    )
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
    let mut images = images::ImageWriter::default();
    let mut image_diagnostics = Vec::with_capacity(parts.len());
    let mut outline = krilla::outline::Outline::new();
    for (part_index, part) in parts.iter().enumerate() {
        let trace = part.trace;
        let mut text = text::TextWriter::new(part.fonts);
        for (page_index, (lines, items)) in trace.pages.iter().zip(&trace.display_lists).enumerate()
        {
            let record = &trace.page_records[page_index];
            let [width, height] = record.geometry.paper;
            let page_area = record.geometry.content_box();
            let settings = PageSettings::from_wh(width, height)
                .ok_or(PdfError::InvalidPageSize { width, height })?;
            let production_rect = |rect: ferri_pdf_graphics::Rect| {
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
            let mut scopes = Vec::new();
            let mut viewport = record.output.media;
            let painted = (|| -> Result<(), PdfError> {
                for item in items {
                    match item {
                        DisplayItem::Image(paint) => images.draw(paint, viewport, &mut surface)?,
                        DisplayItem::Box(box_paint) => painting::draw(
                            box_paint,
                            page_area,
                            viewport,
                            &mut surface,
                            &mut images,
                        )?,
                        DisplayItem::Text { line } => {
                            let fragment =
                                lines.get(*line).ok_or(PdfError::InvalidTextReference {
                                    page: page_index,
                                    line: *line,
                                })?;
                            text.draw(fragment, &mut surface)?;
                        }
                        DisplayItem::Decoration(decoration) => {
                            for fill in decoration.fills() {
                                painting::draw_fill(fill, viewport, &mut surface)?;
                            }
                        }
                        DisplayItem::PushClip(clip) => {
                            scopes.push((
                                painting::push_clip(clip, viewport, &mut surface)?,
                                viewport,
                            ));
                        }
                        DisplayItem::PushTransform([a, b, c, d, e, f]) => {
                            surface.push_transform(&Transform::from_row(*a, *b, *c, *d, *e, *f));
                            scopes.push((1, viewport));
                        }
                        DisplayItem::PushOpacity(alpha) => {
                            let opacity = krilla::num::NormalizedF32::new(*alpha)
                                .ok_or(PdfError::InvalidAlpha(*alpha))?;
                            let nested_viewport = if opacity == krilla::num::NormalizedF32::ONE {
                                viewport
                            } else {
                                painting::local_bounds(viewport, &surface)?
                            };
                            surface.push_opacity(opacity);
                            scopes.push((1, viewport));
                            viewport = nested_viewport;
                        }
                        DisplayItem::Pop => {
                            let (count, previous) =
                                scopes.pop().ok_or(PdfError::InvalidGeometry)?;
                            painting::pop_clips(count, &mut surface);
                            viewport = previous;
                        }
                    }
                }
                if !scopes.is_empty() {
                    return Err(PdfError::InvalidGeometry);
                }
                Ok(())
            })();
            while let Some((count, _)) = scopes.pop() {
                painting::pop_clips(count, &mut surface);
            }
            surface.pop();
            if painted.is_ok() {
                for mark in &record.output.marks {
                    painting::draw(
                        mark,
                        page_area,
                        record.output.media,
                        &mut surface,
                        &mut images,
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
