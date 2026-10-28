//! HTML and CSS to PDF, implemented in Rust.
//!
//! The compatibility corpus is executable with `python3 tests/oracle/run.py`.
//! Unsupported formatting returns an error rather than an inaccurate PDF.
//!
//! ```
//! use ferri_pdf::{render_html, RenderOptions};
//! let rendered = render_html("<p>Hello from Rust.</p>", &RenderOptions::default())?;
//! assert_eq!(rendered.page_count, 1);
//! # Ok::<(), ferri_pdf::Error>(())
//! ```
#![forbid(unsafe_code)]

// The shared integration-test helpers name the public API as `ferri_pdf`.
#[cfg(test)]
extern crate self as ferri_pdf;

pub mod resources;

mod document;
mod dom;
mod flex;
mod fonts;
mod graphics;
mod grid;
mod images;
mod layout;
mod multipart;
mod paint;
mod spec_data;
mod style;
mod svg;
mod table;
#[cfg(test)]
mod tests;
mod text;
mod writer;

pub use multipart::{
    HtmlPart, MultipartOutput, PartOutput, render_parts, render_parts_with_resolver,
};

use crate::document::{DocumentError, StyledDocument};
use crate::layout::LayoutError;
use crate::text::TextError;

pub use crate::document::DocumentDiagnostic as RenderDiagnostic;
pub use crate::fonts::{
    FontDescriptor, FontFaceStyle, FontFamily, FontId, FontPreferences, FontRange, FontSelection,
    FontSlant, FontStyle, FontSynthesis, FontVariation, FontWeight, FontWidth, GenericFamily,
};
pub use crate::graphics::{
    BlendMode, Border, BorderStyle, BoxDecoration, BoxPaint, Clip, ClipBounds, Color, Contour,
    DecorationLine, DecorationPaint, DecorationStyle, DisplayItem, Fill, INLINE_EDGES,
    LengthPercentage, Outline, OutlineStyle, Point, Radii, Rect, Segment, ShadowPaint, ShadowShape,
    Shape,
};
pub use crate::images::ImageError;
pub use crate::layout::{Destination, HeadingLocation, LayoutTrace, LinkArea, LinkShape, TextLine};
pub use crate::resources::{
    FileResolver, ResolvedResource, Resource, ResourceBase, ResourceError, ResourceMetadata,
    ResourceResolver, Url,
};
#[cfg(feature = "http")]
pub use crate::resources::{LoaderOptions, UrlResolver};
pub use crate::style::{
    ColorScheme, ContrastPreference, MotionPreference, PrintPreferences, PrintSettings, StyleError,
};
pub use crate::text::{GlyphRun, PositionedGlyph};
pub use crate::writer::{
    ImageDpi, ImageOptions, InvalidImageDpi, InvalidJpegQuality, JpegQuality, PdfError,
};

/// Settings for a rendering job. CSS controls page size and formatting.
#[derive(Clone, Debug, Default)]
pub struct RenderOptions {
    /// Local directory or full document URL for resolving relative resources.
    pub base: ResourceBase,
    /// Additional author stylesheets, appended after document stylesheets.
    pub stylesheets: Vec<String>,
    /// User-origin stylesheets, including accessibility and personal print preferences.
    pub user_stylesheets: Vec<String>,
    /// Physical paper and deterministic media-query preferences.
    pub print: PrintSettings,
    /// Retain the measured layout for diagnostics and integration tooling.
    pub include_layout: bool,
    /// Optional raster encoding controls, applied after layout.
    pub images: ImageOptions,
}

/// A completed rendering job. No output file is written by the library.
pub struct RenderOutput {
    /// The complete PDF byte stream.
    pub pdf: Vec<u8>,
    /// Number of PDF pages.
    pub page_count: usize,
    /// Located HTML headings, in document order, with one-based page numbers.
    pub headings: Vec<HeadingLocation>,
    /// Measured page fragments, present only when requested.
    pub layout: Option<LayoutTrace>,
    /// Recoverable input and resource issues encountered during rendering.
    pub diagnostics: Vec<RenderDiagnostic>,
}

/// A rendering failure, identified by pipeline stage.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("part {part}: {source}")]
    Part {
        part: usize,
        #[source]
        source: Box<Error>,
    },
    #[error("parts {first} and {second} use the same document URL: {url}")]
    DuplicatePartUrl {
        url: Url,
        first: usize,
        second: usize,
    },
    #[error("HTML: {0}")]
    Html(#[source] DocumentError),
    #[error("image: {0}")]
    Image(#[from] ImageError),
    #[error("resource error: {0}")]
    Resource(#[from] ResourceError),
    #[error("CSS: {0}")]
    Css(#[from] StyleError),
    #[error("document formatting: {0}")]
    Document(String),
    #[error("layout: {0}")]
    Layout(String),
    #[error("font is not a valid supported OpenType font")]
    InvalidFont,
    #[error("PDF serialization: {0}")]
    Pdf(#[from] PdfError),
}

#[cfg(feature = "http")]
type DefaultResolver = UrlResolver;
#[cfg(not(feature = "http"))]
type DefaultResolver = FileResolver;

/// Render an HTML string and its CSS with local, data and, with the `http` feature, HTTP(S) resources.
pub fn render_html(html: &str, options: &RenderOptions) -> Result<RenderOutput, Error> {
    render_with_resolver(html, options, &DefaultResolver::default())
}

/// Render using a caller-supplied resource source, such as an in-memory asset map.
pub fn render_with_resolver(
    html: &str,
    options: &RenderOptions,
    resolver: &dyn ResourceResolver,
) -> Result<RenderOutput, Error> {
    render_document(
        StyledDocument::parse_with_settings(
            html,
            &options.base,
            &options.stylesheets,
            &options.user_stylesheets,
            &options.print,
            resolver,
        ),
        options,
        resolver,
    )
}

/// Render buffered HTML bytes using standards encoding detection.
pub fn render_bytes(bytes: &[u8], options: &RenderOptions) -> Result<RenderOutput, Error> {
    render_bytes_with_resolver(bytes, options, &DefaultResolver::default())
}

/// Render buffered HTML bytes with a caller-supplied resource resolver.
pub fn render_bytes_with_resolver(
    bytes: &[u8],
    options: &RenderOptions,
    resolver: &dyn ResourceResolver,
) -> Result<RenderOutput, Error> {
    render_document(
        StyledDocument::parse_bytes_with_settings(
            bytes,
            &options.base,
            &options.stylesheets,
            &options.user_stylesheets,
            &options.print,
            resolver,
        ),
        options,
        resolver,
    )
}

fn render_document(
    document: Result<StyledDocument, DocumentError>,
    options: &RenderOptions,
    resolver: &dyn ResourceResolver,
) -> Result<RenderOutput, Error> {
    let (document, trace) = prepare_document(document, resolver)?;
    let encoded = writer::encode_with_diagnostics(
        &trace,
        &document.styles().fonts,
        document.navigation(),
        document.metadata(),
        &options.images,
    )
    .map_err(pdf_error)?;
    let headings = trace.heading_locations(&document.navigation().headings);
    let mut diagnostics = document.into_diagnostics();
    diagnostics.extend(
        encoded
            .image_diagnostics
            .into_iter()
            .flatten()
            .map(|error| RenderDiagnostic::ImageDecode { error }),
    );
    Ok(RenderOutput {
        pdf: encoded.pdf,
        headings,
        page_count: trace.pages.len(),
        layout: options.include_layout.then_some(trace),
        diagnostics,
    })
}

fn prepare_document(
    document: Result<StyledDocument, DocumentError>,
    resolver: &dyn ResourceResolver,
) -> Result<(StyledDocument, LayoutTrace), Error> {
    let document = document.map_err(|error| match error {
        error @ DocumentError::Tree(_) => Error::Html(error),
        DocumentError::Style(error) => Error::Css(error),
        DocumentError::Resource(error) => Error::Css(StyleError::Resource(error)),
        DocumentError::Image(error) => Error::Image(error),
        DocumentError::Formatting(message) => Error::Document(message),
        error @ DocumentError::TextExpansionLimit { .. } => Error::Document(error.to_string()),
    })?;
    let (document, trace) =
        layout::layout_document(document, resolver).map_err(|error| match error {
            LayoutError::Text(TextError::Font(fonts::FontError::InvalidFont(_))) => {
                Error::InvalidFont
            }
            LayoutError::Text(TextError::Font(fonts::FontError::Resource(error))) => {
                Error::Css(StyleError::Resource(error))
            }
            LayoutError::Text(TextError::Font(error)) => Error::Css(StyleError::Font(error)),
            error => Error::Layout(error.to_string()),
        })?;
    Ok((document, trace))
}

fn pdf_error(error: PdfError) -> Error {
    match error {
        PdfError::InvalidFont => Error::InvalidFont,
        error => Error::Pdf(error),
    }
}
