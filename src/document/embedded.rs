//! What `iframe`, `embed` and `object` elements embed (HTML §4.8.5–§4.8.7), resolved before
//! boxes are built because an `object` showing its fallback content is not replaced.
use crate::document::frames::{self, Frame, Navigable};
use crate::document::{DocumentDiagnostic, DocumentError};
use crate::dom::{DocumentTree, Element, ElementId, HTML_NAMESPACE, TreeRelation};
use crate::fonts::FontNamespace;
use crate::images::{Image, ImageCache};
use crate::resources::{ResourceBase, ResourceMetadata, ResourceResolver, Url, sniffing};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// HTML §4.8.6 "display an indication that no plugin could be found": a neutral panel with a
/// prohibition sign, scaled into the element's box.
const NO_PLUGIN_INDICATION: &str = concat!(
    r#"<svg xmlns="http://www.w3.org/2000/svg">"#,
    r##"<rect width="100%" height="100%" fill="#ececec"/>"##,
    r#"<svg width="100%" height="100%" viewBox="0 0 100 100">"#,
    r##"<g fill="none" stroke="#8c8c8c" stroke-width="4">"##,
    r#"<circle cx="50" cy="50" r="16"/><line x1="39" y1="39" x2="61" y2="61"/></g></svg></svg>"#,
);
/// Elements whose descendants are not rendered, so their embedded content never loads.
const REPLACED_ANCESTORS: &[&str] = &["iframe", "embed", "video", "audio", "img"];

/// What an `iframe`, `embed` or `object` element represents.
pub(crate) enum Embedded {
    Image(Arc<Image>),
    Navigable(Arc<Frame>),
}

/// Content of the `iframe`, `embed` and `object` elements a document renders.
#[derive(Default)]
pub(crate) struct EmbeddedContent {
    replaced: HashMap<ElementId, Embedded>,
}
impl EmbeddedContent {
    pub(crate) fn content(&self, element: ElementId) -> Option<&Embedded> {
        self.replaced.get(&element)
    }
    pub(crate) fn take_diagnostics(&self) -> Vec<DocumentDiagnostic> {
        self.replaced
            .values()
            .filter_map(|embedded| match embedded {
                Embedded::Navigable(frame) => Some(frame.take_diagnostics()),
                Embedded::Image(_) => None,
            })
            .flatten()
            .collect()
    }
}

/// The document whose embedded content is being resolved.
pub(crate) struct Embedder<'a> {
    /// The document base URL.
    pub base: &'a ResourceBase,
    /// The navigable each embedding element creates for its own content.
    pub navigable: Navigable,
    pub resolver: &'a dyn ResourceResolver,
}

struct Resolution<'a, 'c> {
    embedder: &'a Embedder<'a>,
    cache: &'c mut ImageCache,
    diagnostics: Vec<DocumentDiagnostic>,
    no_plugin: Option<Arc<Image>>,
}

pub(crate) fn resolve(
    tree: &mut DocumentTree,
    embedder: &Embedder<'_>,
    cache: &mut ImageCache,
) -> Result<(EmbeddedContent, Vec<DocumentDiagnostic>), DocumentError> {
    let mut resolution = Resolution {
        embedder,
        cache,
        diagnostics: Vec::new(),
        no_plugin: None,
    };
    let mut content = EmbeddedContent::default();
    let mut objects = HashSet::new();
    for element in tree.shadow_including_elements() {
        if element.namespace() != HTML_NAMESPACE
            || !matches!(element.name(), "iframe" | "embed" | "object" | "frame")
            || element
                .ancestors(TreeRelation::Formatting)
                .any(|ancestor| hides_descendants(ancestor, &objects))
        {
            continue;
        }
        let replaced = match element.name() {
            "iframe" => Some(Embedded::Navigable(resolution.iframe(element)?)),
            // HTML §15.6: only frames in a frameset render their content navigable.
            "frame" => element
                .parent(TreeRelation::Formatting)
                .filter(|parent| {
                    parent.namespace() == HTML_NAMESPACE && parent.name() == "frameset"
                })
                .map(|_| resolution.frame(element))
                .transpose()?
                .map(Embedded::Navigable),
            "embed" => Some(Embedded::Image(resolution.embed(element)?)),
            _ => {
                let represented = resolution.object(element)?;
                if represented.is_some() {
                    objects.insert(element.id());
                }
                represented
            }
        };
        if let Some(replaced) = replaced {
            content.replaced.insert(element.id(), replaced);
        }
    }
    for object in objects {
        tree.set_object_embeds(object);
    }
    Ok((content, resolution.diagnostics))
}

fn hides_descendants(ancestor: Element<'_>, objects: &HashSet<ElementId>) -> bool {
    ancestor.namespace() != HTML_NAMESPACE
        || REPLACED_ANCESTORS.contains(&ancestor.name())
        || objects.contains(&ancestor.id())
}

/// A MIME type attribute's essence, or `None` when it is no valid MIME type.
fn essence(value: &str) -> Option<String> {
    ResourceMetadata::from_content_type(value).media_type
}

/// Types a nested navigable or image renders.
fn supported(essence: &str) -> bool {
    essence == sniffing::HTML
        || crate::images::SUPPORTED_TYPES.contains(&essence)
        || sniffing::is_javascript(essence)
        || sniffing::is_json(essence)
        || frames::is_text(essence)
}

impl Resolution<'_, '_> {
    fn unavailable(&mut self, reference: &str, error: crate::resources::ResourceError) {
        self.diagnostics
            .push(DocumentDiagnostic::UnavailableEmbedded {
                base: self.embedder.base.clone(),
                reference: reference.to_owned(),
                error,
            });
    }

    fn child(&self) -> Result<Navigable, DocumentError> {
        self.embedder.navigable.child()
    }

    /// HTML §4.8.5 "process the iframe attributes"; with scripting disabled nothing loads
    /// lazily (HTML §2.5.7).
    fn iframe(&mut self, element: Element<'_>) -> Result<Arc<Frame>, DocumentError> {
        if let Some(srcdoc) = element.attr("srcdoc") {
            return Ok(Frame::html(
                srcdoc.to_owned(),
                self.embedder.base,
                self.child()?,
            ));
        }
        self.frame(element)
    }

    /// The `src` attribute navigation shared by iframes and frames (HTML §4.8.5, §16.3.1).
    fn frame(&mut self, element: Element<'_>) -> Result<Arc<Frame>, DocumentError> {
        let base = self.embedder.base;
        let navigable = self.child()?;
        let Some(url) = element
            .attr("src")
            .filter(|reference| !reference.is_empty())
            .and_then(|reference| base.resolve(reference).ok())
        else {
            return Ok(Frame::blank(base, navigable));
        };
        if self.embedder.navigable.shows(&url) || frames::is_about_blank(&url) {
            return Ok(Frame::blank(base, navigable));
        }
        self.navigate(&url, navigable)
    }

    fn navigate(&mut self, url: &Url, navigable: Navigable) -> Result<Arc<Frame>, DocumentError> {
        match self
            .embedder
            .resolver
            .resolve_with_metadata(url.as_str(), self.embedder.base)
        {
            Ok(resolved) => {
                frames::load_document(url, resolved, navigable, self.cache, self.embedder.resolver)
            }
            Err(error) => {
                self.unavailable(url.as_str(), error);
                Ok(Frame::blank(self.embedder.base, navigable))
            }
        }
    }

    /// HTML §4.8.6 embed element setup steps. No plugin handles any type, so a fetched or
    /// typed embed displays the no-plugin indication and represents nothing.
    fn embed(&mut self, element: Element<'_>) -> Result<Arc<Image>, DocumentError> {
        let indication = match (element.attr("src"), element.attr("type")) {
            (None, None) | (Some(""), _) => false,
            (Some(reference), _) => match self.embedder.base.resolve(reference) {
                Err(_) => false,
                Ok(url) => match self
                    .embedder
                    .resolver
                    .resolve_with_metadata(url.as_str(), self.embedder.base)
                {
                    Ok(_) => true,
                    Err(error) => {
                        self.unavailable(reference, error);
                        false
                    }
                },
            },
            (None, Some(_)) => true,
        };
        if !indication {
            return Ok(Arc::new(Image::Transparent));
        }
        if let Some(image) = &self.no_plugin {
            return Ok(image.clone());
        }
        let image = self.cache.inline_svg_in_namespace(
            NO_PLUGIN_INDICATION.into(),
            self.embedder.base,
            self.embedder.resolver,
            FontNamespace::ROOT,
        )?;
        self.no_plugin = Some(image.clone());
        Ok(image)
    }

    /// HTML §4.8.7: the image or content navigable an `object` represents, or `None` for its
    /// fallback content.
    fn object(&mut self, element: Element<'_>) -> Result<Option<Embedded>, DocumentError> {
        let Some(reference) = element.attr("data").filter(|data| !data.is_empty()) else {
            return Ok(None);
        };
        let declared = element.attr("type").map(essence);
        if declared.as_ref().is_some_and(|declared| {
            declared
                .as_deref()
                .is_none_or(|essence| !supported(essence))
        }) {
            return Ok(None);
        }
        let declared = declared.flatten();
        let Ok(url) = self.embedder.base.resolve(reference) else {
            return Ok(None);
        };
        let resolved = match self
            .embedder
            .resolver
            .resolve_with_metadata(url.as_str(), self.embedder.base)
        {
            Ok(resolved) => resolved,
            Err(error) => {
                self.unavailable(reference, error);
                return Ok(None);
            }
        };
        let Some(resource_type) = resource_type(&url, &resolved, declared) else {
            return Ok(None);
        };
        if sniffing::is_xml(&resource_type) || !resource_type.starts_with(sniffing::IMAGE_PREFIX) {
            let navigable = self.child()?;
            return frames::load_document(
                &url,
                resolved,
                navigable,
                self.cache,
                self.embedder.resolver,
            )
            .map(|frame| Some(Embedded::Navigable(frame)));
        }
        Ok(self
            .cache
            .decode_optional(
                reference,
                self.embedder.base,
                resolved.resource,
                self.embedder.resolver,
                FontNamespace::ROOT,
            )
            .map(Embedded::Image))
    }
}

/// HTML §4.8.7 "determine the resource type"; `None` is the unknown type.
fn resource_type(
    url: &Url,
    resolved: &crate::resources::ResolvedResource,
    declared: Option<String>,
) -> Option<String> {
    let bytes = &resolved.resource.bytes;
    if let Some(supplied) = sniffing::supplied_type(url, &resolved.metadata) {
        let binary = (supplied == sniffing::PLAIN_TEXT
            && sniffing::text_or_binary(bytes) != sniffing::PLAIN_TEXT)
            || supplied == sniffing::OCTET_STREAM;
        if !binary {
            return Some(supplied);
        }
        return declared.filter(|declared| {
            declared != sniffing::OCTET_STREAM
                && declared.starts_with(sniffing::IMAGE_PREFIX)
                && !sniffing::is_xml(declared)
        });
    }
    let tentative = declared.unwrap_or_else(|| {
        sniffing::computed_type(
            None,
            false,
            bytes,
            |essence| crate::images::SUPPORTED_TYPES.contains(&essence),
            |_| false,
        )
    });
    (tentative != sniffing::OCTET_STREAM).then_some(tentative)
}
