//! Nested navigables (HTML §7.3) and the documents navigating them loads (HTML §7.5), laid
//! out once their viewport size is known.
use crate::document::{DocumentDiagnostic, DocumentError, ParseOptions, StyledDocument};
use crate::fonts::FontNamespace;
use crate::images::{Image, ImageCache};
use crate::resources::{ResolvedResource, ResourceBase, ResourceResolver, Url, sniffing};
use crate::style::PrintSettings;
use std::sync::{Arc, Mutex, PoisonError};

/// Navigables nested beyond this show nothing. A navigable never loads a URL one of its ancestors
/// shows (HTML §4.8.5), so only finite `srcdoc` nesting reaches it; each level lays a whole
/// document out on the stack.
pub(crate) const MAX_NAVIGABLE_DEPTH: usize = 8;
const UTF8: &str = "UTF-8";
const ABOUT_SCHEME: &str = "about";
const BLANK_PATH: &str = "blank";
const TEXT_TYPES: &[&str] = &["text/css", sniffing::PLAIN_TEXT, "text/vtt"];

/// What the documents of an embedding document's child navigables inherit from it.
#[derive(Clone, Debug)]
pub(crate) struct Navigable {
    pub settings: PrintSettings,
    pub user: Vec<String>,
    /// Document URLs of the embedding document and the documents containing it, outermost
    /// first; `None` where unknown.
    pub ancestors: Vec<Option<Url>>,
}
impl Navigable {
    /// A child navigable's inheritance, unless nesting exceeds [`MAX_NAVIGABLE_DEPTH`].
    pub(crate) fn child(&self) -> Option<Self> {
        (self.ancestors.len() <= MAX_NAVIGABLE_DEPTH).then(|| self.clone())
    }
    /// HTML §4.8.5 shared attribute processing: a navigable never loads a URL an inclusive
    /// ancestor shows, fragments aside.
    pub(crate) fn shows(&self, url: &Url) -> bool {
        let unfragmented = |url: &Url| {
            let mut url = url.clone();
            url.set_fragment(None);
            url
        };
        let url = unfragmented(url);
        self.ancestors
            .iter()
            .flatten()
            .any(|ancestor| unfragmented(ancestor) == url)
    }
}

/// HTML §7.5.4 text document types besides JavaScript and JSON.
pub(crate) fn is_text(essence: &str) -> bool {
    TEXT_TYPES.contains(&essence)
}

pub(crate) fn is_about_blank(url: &Url) -> bool {
    url.scheme() == ABOUT_SCHEME && url.path() == BLANK_PATH
}

#[derive(Debug)]
enum Source {
    Html {
        text: String,
        encoding: String,
        base: ResourceBase,
    },
    Svg(Arc<Image>),
}

/// The active document of a nested navigable, as navigation loaded it.
#[derive(Debug)]
pub struct Frame {
    source: Source,
    navigable: Navigable,
    views: Mutex<Vec<([u32; 2], Arc<Image>)>>,
    diagnostics: Mutex<Vec<DocumentDiagnostic>>,
}

/// A frame's document prepared for one viewport size.
pub(crate) enum FrameDocument {
    Html(Box<StyledDocument>),
    Svg(Arc<Image>),
}

impl Frame {
    fn new(source: Source, navigable: Navigable) -> Arc<Self> {
        Arc::new(Self {
            source,
            navigable,
            views: Mutex::default(),
            diagnostics: Mutex::default(),
        })
    }
    /// The initial `about:blank` document, which a navigable keeps when nothing replaces it;
    /// it inherits its creator's base URL.
    pub(crate) fn blank(base: &ResourceBase, navigable: Navigable) -> Arc<Self> {
        Self::html(String::new(), base, navigable)
    }
    /// An HTML document, such as an `iframe srcdoc` document, whose fallback base URL is
    /// `base` (HTML §2.4.1).
    pub(crate) fn html(text: String, base: &ResourceBase, navigable: Navigable) -> Arc<Self> {
        Self::new(
            Source::Html {
                text,
                encoding: UTF8.into(),
                base: base.clone(),
            },
            navigable,
        )
    }

    /// The document `size` points of viewport show, with media queries and viewport units
    /// evaluated against that size, embedded by an element of `color_scheme`.
    pub(crate) fn document(
        &self,
        size: [f32; 2],
        color_scheme: crate::style::ColorScheme,
        resolver: &dyn ResourceResolver,
    ) -> Result<FrameDocument, DocumentError> {
        let (text, encoding, base) = match &self.source {
            Source::Svg(image) => return Ok(FrameDocument::Svg(image.clone())),
            Source::Html {
                text,
                encoding,
                base,
            } => (text, encoding, base),
        };
        let settings = PrintSettings {
            paper: size,
            ..self.navigable.settings.clone()
        };
        StyledDocument::parse_decoded(
            text,
            encoding,
            &ParseOptions {
                base,
                additional: &[],
                user: &self.navigable.user,
                settings: &settings,
                capture_fragment_targets: false,
                presentation: crate::style::Presentation::Viewport { color_scheme },
                ancestors: &self.navigable.ancestors,
            },
            resolver,
        )
        .map(|document| FrameDocument::Html(Box::new(document)))
    }

    pub(crate) fn cached(&self, size: [f32; 2]) -> Option<Arc<Image>> {
        let key = size.map(f32::to_bits);
        self.views
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find_map(|(size, view)| (*size == key).then(|| view.clone()))
    }
    pub(crate) fn store(&self, size: [f32; 2], view: Arc<Image>) {
        self.views
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((size.map(f32::to_bits), view));
    }
    pub(crate) fn record_diagnostics(&self, diagnostics: Vec<DocumentDiagnostic>) {
        self.diagnostics
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(diagnostics);
    }
    pub(crate) fn take_diagnostics(&self) -> Vec<DocumentDiagnostic> {
        std::mem::take(
            &mut *self
                .diagnostics
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }
}

/// Escapes text for an HTML attribute value or text content.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '"' => escaped.push_str("&quot;"),
            character => escaped.push(character),
        }
    }
    escaped
}

/// HTML §7.5.1 "load a document" for a fetched `resource` at `url`. Types this user agent
/// neither renders nor views inline are handed off, leaving the initial `about:blank`.
pub(crate) fn load_document(
    url: &Url,
    resolved: ResolvedResource,
    navigable: Navigable,
    cache: &mut ImageCache,
    resolver: &dyn ResourceResolver,
) -> Result<Arc<Frame>, DocumentError> {
    let ResolvedResource { resource, metadata } = resolved;
    let computed = sniffing::computed_type(
        sniffing::supplied_type(url, &metadata).as_deref(),
        sniffing::apache_bug(url, &metadata),
        &resource.bytes,
        |essence| crate::images::SUPPORTED_TYPES.contains(&essence),
        |_| false,
    );
    let base = resource.base.clone();
    let charset = metadata.charset.as_deref();
    let frame = if computed == sniffing::HTML {
        // HTML §7.5.2.
        let (decoded, _) = crate::document::encoding::decode_document(&resource.bytes, charset);
        let frame = Frame::new(
            Source::Html {
                text: decoded.text,
                encoding: decoded.encoding.clone(),
                base: base.clone(),
            },
            navigable,
        );
        if decoded.had_errors {
            frame.record_diagnostics(vec![DocumentDiagnostic::Decoding {
                base,
                reference: Some(url.to_string()),
                encoding: decoded.encoding,
            }]);
        }
        frame
    } else if computed == sniffing::SVG {
        match cache.decode_optional(url.as_str(), &base, resource, resolver, FontNamespace::ROOT) {
            Some(image) => Frame::new(Source::Svg(image), navigable),
            None => Frame::blank(&base, navigable),
        }
    } else if sniffing::is_xml(&computed) {
        // HTML §7.5.3: without processing for this XML vocabulary there is nothing to render,
        // as for a type handed off to external software.
        let frame = Frame::blank(&base, navigable);
        frame.record_diagnostics(vec![DocumentDiagnostic::UnrenderedDocument {
            reference: url.to_string(),
            media_type: computed,
        }]);
        frame
    } else if sniffing::is_javascript(&computed)
        || sniffing::is_json(&computed)
        || is_text(&computed)
    {
        // HTML §7.5.4: the parser starts in a `pre` element and takes the rest as plain text.
        let decoded = crate::document::encoding::decode_text(&resource.bytes, charset);
        Frame::new(
            Source::Html {
                text: format!("<!DOCTYPE html><pre>\n{}", escape(&decoded.text)),
                encoding: decoded.encoding,
                base,
            },
            navigable,
        )
    } else if crate::images::SUPPORTED_TYPES.contains(&computed.as_str()) {
        // HTML §7.5.6.
        Frame::new(
            Source::Html {
                text: format!(
                    "<!DOCTYPE html><html><head></head><body><img src=\"{}\"></body></html>",
                    escape(url.as_str())
                ),
                encoding: UTF8.into(),
                base,
            },
            navigable,
        )
    } else {
        Frame::blank(&base, navigable)
    };
    Ok(frame)
}
