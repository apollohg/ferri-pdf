//! WebAssembly bindings: a render job the JavaScript package drives, fetching the resources
//! each attempt reports missing until the render completes with everything in memory.
#![cfg(all(target_family = "wasm", target_os = "unknown"))]

mod options;
mod resources;

use ferri_pdf::{
    Error, HeadingLocation, HtmlPart, LayoutTrace, PdfError, RenderDiagnostic, RenderOptions,
    ResourceBase, ResourceMetadata,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::options::{Settings, parse_url, resource_base};
use crate::resources::SuppliedResources;

const PART_BASE_URL: &str = "part baseUrl";
const RESOURCE_URL: &str = "resource url";
const UNFINISHED: &str = "the render job has not finished";

enum Html {
    Text(String),
    Bytes(Vec<u8>),
}

struct Part {
    html: Html,
    base: Option<ResourceBase>,
}

impl Part {
    fn html(&self) -> HtmlPart<'_> {
        let part = match &self.html {
            Html::Text(text) => HtmlPart::from_html(text),
            Html::Bytes(bytes) => HtmlPart::from_bytes(bytes),
        };
        match &self.base {
            Some(base) => part.with_base(base.clone()),
            None => part,
        }
    }
}

/// A recovered input problem, or with `loses_content`, content a resource limit left out.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Diagnostic {
    message: String,
    loses_content: bool,
}

fn diagnostics(diagnostics: Vec<RenderDiagnostic>) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| Diagnostic {
            loses_content: diagnostic.loses_content(),
            message: diagnostic.to_string(),
        })
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PartReport {
    /// One-based first page in the combined PDF; absent for a single document.
    #[serde(skip_serializing_if = "Option::is_none")]
    first_page: Option<usize>,
    page_count: usize,
    headings: Vec<HeadingLocation>,
    diagnostics: Vec<Diagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    layout: Option<LayoutTrace>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MultipartReport {
    page_count: usize,
    parts: Vec<PartReport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Failure {
    message: String,
    /// The content a resource limit left out, when that is why the render failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    lost: Option<Vec<Diagnostic>>,
    /// The one-based part that failed, in a multipart render.
    #[serde(skip_serializing_if = "Option::is_none")]
    part: Option<usize>,
}

/// What a finished job tells JavaScript besides the PDF bytes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum Report {
    Document(PartReport),
    Multipart(MultipartReport),
    Error(Failure),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        let message = error.to_string();
        let (part, lost) = match error {
            Error::ContentLost { diagnostics } => (None, Some(diagnostics)),
            Error::Part { part, source } => match *source {
                Error::ContentLost { diagnostics } => (Some(part), Some(diagnostics)),
                _ => (Some(part), None),
            },
            _ => (None, None),
        };
        Self {
            message,
            lost: lost.map(diagnostics),
            part,
        }
    }
}

struct Outcome {
    pdf: Vec<u8>,
    report: Report,
}

/// One render of a document, or of several parts into one PDF. JavaScript calls `advance`
/// until it returns no URLs, answering each URL with `supply` or `fail` in between.
#[wasm_bindgen]
pub struct RenderJob {
    parts: Vec<Part>,
    multipart: bool,
    options: RenderOptions,
    resources: SuppliedResources,
    outcome: Option<Outcome>,
}

#[wasm_bindgen]
impl RenderJob {
    /// `settings` is the JSON of the package's render options; `multipart` renders the parts
    /// as `renderParts` does, even when there is only one.
    #[wasm_bindgen(constructor)]
    pub fn new(settings: &str, multipart: bool) -> Result<RenderJob, JsError> {
        let settings = Settings::parse(settings).map_err(|error| JsError::new(&error))?;
        let resources = SuppliedResources::new(settings.fetch_limits());
        Ok(Self {
            parts: Vec::new(),
            multipart,
            options: settings
                .into_options(multipart)
                .map_err(|error| JsError::new(&error))?,
            resources,
            outcome: None,
        })
    }

    /// Adds a document that is already decoded; `base_url` overrides the job's base for it,
    /// as a folder if `directory` and else as the document's own URL.
    #[wasm_bindgen(js_name = addText)]
    pub fn add_text(
        &mut self,
        html: String,
        base_url: Option<String>,
        directory: bool,
    ) -> Result<(), JsError> {
        self.add(Html::Text(html), base_url, directory)
    }

    /// Adds a document whose encoding the HTML rules detect.
    #[wasm_bindgen(js_name = addBytes)]
    pub fn add_bytes(
        &mut self,
        html: Vec<u8>,
        base_url: Option<String>,
        directory: bool,
    ) -> Result<(), JsError> {
        self.add(Html::Bytes(html), base_url, directory)
    }

    /// Renders with the resources supplied so far. Returns the URLs to fetch before calling
    /// again, or none once the job has finished.
    pub fn advance(&mut self) -> Vec<String> {
        if self.outcome.is_some() {
            return Vec::new();
        }
        match self.render() {
            Err(Error::Interrupted) => self.resources.start_round(),
            Ok(outcome) => {
                self.outcome = Some(outcome);
                Vec::new()
            }
            Err(error) => {
                self.outcome = Some(Outcome {
                    pdf: Vec::new(),
                    report: Report::Error(error.into()),
                });
                Vec::new()
            }
        }
    }

    /// Bytes the job may still be supplied; a longer response is not worth reading.
    #[wasm_bindgen(js_name = remainingBytes)]
    pub fn remaining_bytes(&self) -> usize {
        self.resources.remaining_bytes()
    }

    /// Why a response longer than `remainingBytes` is unavailable.
    #[wasm_bindgen(js_name = byteLimitReason)]
    pub fn byte_limit_reason(&self) -> String {
        self.resources.byte_limit_reason()
    }

    /// Answers `url` with its bytes; `location` is where redirects ended, if they moved it.
    pub fn supply(
        &mut self,
        url: &str,
        data: Vec<u8>,
        content_type: Option<String>,
        location: Option<String>,
    ) -> Result<(), JsError> {
        let location = location
            .map(|location| parse_url(RESOURCE_URL, &location))
            .transpose()
            .map_err(|error| JsError::new(&error))?;
        let metadata = content_type
            .map(|content_type| ResourceMetadata::from_content_type(&content_type))
            .unwrap_or_default();
        self.resources
            .supply(resource_url(url)?, data, metadata, location);
        Ok(())
    }

    /// Answers `url` as unavailable, which the render reports as a diagnostic.
    pub fn fail(&mut self, url: &str, reason: &str) -> Result<(), JsError> {
        self.resources.fail(resource_url(url)?, reason);
        Ok(())
    }

    /// The finished job's PDF, which is empty if the render failed; the job keeps no copy.
    #[wasm_bindgen(js_name = takePdf)]
    pub fn take_pdf(&mut self) -> Result<Vec<u8>, JsError> {
        self.outcome
            .as_mut()
            .map(|outcome| std::mem::take(&mut outcome.pdf))
            .ok_or_else(|| JsError::new(UNFINISHED))
    }

    /// The finished job's page count, headings, diagnostics and layout, or its error, as
    /// JSON.
    pub fn report(&self) -> Result<String, JsError> {
        let outcome = self
            .outcome
            .as_ref()
            .ok_or_else(|| JsError::new(UNFINISHED))?;
        serde_json::to_string(&outcome.report).map_err(|error| JsError::new(&error.to_string()))
    }
}

fn resource_url(url: &str) -> Result<ferri_pdf::Url, JsError> {
    parse_url(RESOURCE_URL, url).map_err(|error| JsError::new(&error))
}

impl RenderJob {
    fn add(
        &mut self,
        html: Html,
        base_url: Option<String>,
        directory: bool,
    ) -> Result<(), JsError> {
        let base = base_url
            .map(|url| resource_base(PART_BASE_URL, &url, directory))
            .transpose()
            .map_err(|error| JsError::new(&error))?;
        self.parts.push(Part { html, base });
        Ok(())
    }

    fn render(&self) -> Result<Outcome, Error> {
        if self.multipart {
            let parts: Vec<_> = self.parts.iter().map(Part::html).collect();
            let rendered =
                ferri_pdf::render_parts_with_resolver(&parts, &self.options, &self.resources)?;
            return Ok(Outcome {
                pdf: rendered.pdf,
                report: Report::Multipart(MultipartReport {
                    page_count: rendered.page_count,
                    parts: rendered
                        .parts
                        .into_iter()
                        .map(|part| PartReport {
                            first_page: Some(part.first_page),
                            page_count: part.page_count,
                            headings: part.headings,
                            diagnostics: diagnostics(part.diagnostics),
                            layout: part.layout,
                        })
                        .collect(),
                }),
            });
        }
        let rendered = match self.parts.first().map(|part| &part.html) {
            Some(Html::Text(text)) => {
                ferri_pdf::render_with_resolver(text, &self.options, &self.resources)
            }
            Some(Html::Bytes(bytes)) => {
                ferri_pdf::render_bytes_with_resolver(bytes, &self.options, &self.resources)
            }
            None => Err(PdfError::EmptyParts.into()),
        }?;
        Ok(Outcome {
            pdf: rendered.pdf,
            report: Report::Document(PartReport {
                first_page: None,
                page_count: rendered.page_count,
                headings: rendered.headings,
                diagnostics: diagnostics(rendered.diagnostics),
                layout: rendered.layout,
            }),
        })
    }
}
