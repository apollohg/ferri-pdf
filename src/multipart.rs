//! Independent HTML documents sharing one PDF and navigation space.
use std::collections::HashMap;

use crate::writer::PdfPart;
use ferri_pdf_document::{ParseOptions, StyledDocument};

use crate::{
    DefaultResolver, Error, HeadingLocation, LayoutTrace, PdfError, RenderDiagnostic,
    RenderOptions, ResourceBase, ResourceResolver, pdf_error, prepare_document,
};

enum HtmlSource<'a> {
    Text(&'a str),
    Bytes(&'a [u8]),
}

/// A borrowed HTML input with an optional document-specific resource base.
pub struct HtmlPart<'a> {
    source: HtmlSource<'a>,
    base: Option<ResourceBase>,
}
impl<'a> HtmlPart<'a> {
    /// Use already-decoded HTML; charset declarations do not re-decode strings.
    pub fn from_html(html: &'a str) -> Self {
        Self {
            source: HtmlSource::Text(html),
            base: None,
        }
    }
    /// Detect the input encoding from its BOM and HTML metadata.
    pub fn from_bytes(bytes: &'a [u8]) -> Self {
        Self {
            source: HtmlSource::Bytes(bytes),
            base: None,
        }
    }
    /// Override the common base; distinct document URLs enable URL-scoped links.
    #[must_use]
    pub fn with_base(mut self, base: impl Into<ResourceBase>) -> Self {
        self.base = Some(base.into());
        self
    }
}

/// One part's navigation and diagnostics within a completed multipart PDF.
pub struct PartOutput {
    /// One-based first page in the combined PDF.
    pub first_page: usize,
    /// Number of pages produced by this part.
    pub page_count: usize,
    /// Heading page numbers refer to the combined PDF.
    pub headings: Vec<HeadingLocation>,
    /// Source indices and page indices remain local to this part.
    pub layout: Option<LayoutTrace>,
    /// Recoverable input and resource issues for this part.
    pub diagnostics: Vec<RenderDiagnostic>,
}

/// A completed multipart rendering job; the library writes no output files.
pub struct MultipartOutput {
    /// The complete combined PDF byte stream.
    pub pdf: Vec<u8>,
    /// Total pages across all parts.
    pub page_count: usize,
    /// Per-part results in input order.
    pub parts: Vec<PartOutput>,
}

/// Render independent HTML parts in order. Each part begins on a new page.
/// The first part supplies PDF metadata; output image settings apply to the whole job.
pub fn render_parts(
    parts: &[HtmlPart<'_>],
    options: &RenderOptions,
) -> Result<MultipartOutput, Error> {
    render_parts_with_resolver(parts, options, &DefaultResolver::default())
}

/// Render independent parts using a shared caller-supplied resource resolver.
pub fn render_parts_with_resolver(
    parts: &[HtmlPart<'_>],
    options: &RenderOptions,
    resolver: &dyn ResourceResolver,
) -> Result<MultipartOutput, Error> {
    if parts.is_empty() {
        return Err(PdfError::EmptyParts.into());
    }
    let mut prepared = Vec::with_capacity(parts.len());
    let mut document_urls = HashMap::new();
    for (index, part) in parts.iter().enumerate() {
        let parse = ParseOptions {
            base: part.base.as_ref().unwrap_or(&options.base),
            additional: &options.stylesheets,
            user: &options.user_stylesheets,
            settings: &options.print,
            capture_fragment_targets: true,
        };
        let document = match part.source {
            HtmlSource::Text(html) => StyledDocument::parse_with_options(html, &parse, resolver),
            HtmlSource::Bytes(bytes) => {
                StyledDocument::parse_bytes_with_options(bytes, &parse, resolver)
            }
        };
        let pair = prepare_document(document, resolver).map_err(|source| Error::Part {
            part: index + 1,
            source: Box::new(source),
        })?;
        if let Some(mut url) = pair
            .0
            .navigation()
            .fragment_targets
            .as_ref()
            .and_then(|targets| targets.document_url.clone())
        {
            url.set_fragment(None);
            if let Some(first) = document_urls.insert(url.clone(), index + 1) {
                return Err(Error::DuplicatePartUrl {
                    url,
                    first,
                    second: index + 1,
                });
            }
        }
        prepared.push(pair);
    }
    let writer_parts: Vec<_> = prepared
        .iter()
        .map(|(document, trace)| PdfPart {
            trace,
            fonts: &document.styles().fonts,
            navigation: document.navigation(),
            metadata: document.metadata(),
        })
        .collect();
    let encoded = crate::writer::encode_parts_with_diagnostics(&writer_parts, &options.images)
        .map_err(pdf_error)?;
    let mut image_diagnostics = encoded.image_diagnostics.into_iter();
    let mut page_count = 0;
    let parts = prepared
        .into_iter()
        .map(|(document, trace)| {
            let mut headings = trace.heading_locations(&document.navigation().headings);
            for heading in &mut headings {
                heading.page += page_count;
            }
            let mut diagnostics = document.into_diagnostics();
            diagnostics.extend(
                image_diagnostics
                    .next()
                    .expect("writer returns diagnostics for every part")
                    .into_iter()
                    .map(|error| RenderDiagnostic::ImageDecode { error }),
            );
            let result = PartOutput {
                first_page: page_count + 1,
                page_count: trace.pages.len(),
                headings,
                layout: options.include_layout.then_some(trace),
                diagnostics,
            };
            page_count += result.page_count;
            result
        })
        .collect();
    Ok(MultipartOutput {
        pdf: encoded.pdf,
        page_count,
        parts,
    })
}
