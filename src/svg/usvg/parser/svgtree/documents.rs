//! `url()` and `href` references into other SVG documents (SVG 2 §5.3.2, CSS Values 4 §4.5):
//! each referenced document is parsed with its own style sheets and appended to the tree as a
//! second, unrendered root element, its ids and internal references renamed apart from the
//! referencing document's so that every reference resolves in the merged tree.
use std::collections::HashMap;
use std::sync::Arc;

use super::{AId, Attribute, Document, EId, NodeData, NodeId, NodeKind, ShortRange};
use crate::resources::Url;
use crate::svg::usvg::parser::{Error, LinkedDocument};

/// The id prefix of the elements of appended documents; a number keeps it apart from ids the
/// documents already use.
const PREFIX: &str = "ferri-pdf-document-";
/// The documents one tree may reference, directly or through other documents.
const MAX_LINKED_DOCUMENTS: usize = 32;
const FRAGMENT: char = '#';

/// The document part and fragment of a reference to an element of another document.
fn split(reference: &str) -> Option<(&str, &str)> {
    let (document, id) = reference.rsplit_once(FRAGMENT)?;
    (!document.is_empty()).then_some((document, id))
}

/// Elements whose `href` names an image resource rather than an element.
fn links_image(tag: EId) -> bool {
    matches!(tag, EId::Image | EId::FeImage)
}

/// Rewrites the element references of `document`'s attributes that `map` maps. `href`
/// values of image elements go to `image` instead, which may rewrite them too.
fn rewrite_references(
    document: &mut Document<'_>,
    map: &mut dyn FnMut(&str) -> Option<String>,
    image: &dyn Fn(&str) -> Option<String>,
) {
    let elements: Vec<(EId, ShortRange)> = document
        .nodes
        .iter()
        .filter_map(|node| match node.kind {
            NodeKind::Element {
                tag_name,
                attributes,
            } => Some((tag_name, attributes)),
            _ => None,
        })
        .collect();
    for (tag, range) in elements {
        for attribute in &mut document.attrs[range.to_urange()] {
            let value = attribute.value.as_str();
            let rewritten = match attribute.name {
                AId::Href if links_image(tag) => image(value),
                AId::Href => map(value),
                AId::Id => None,
                _ => crate::style::rewrite_urls(value, &mut *map),
            };
            if let Some(rewritten) = rewritten {
                attribute.value = roxmltree::StringStorage::new_owned(&rewritten);
            }
        }
    }
}

/// The other documents whose elements `document` references, as written.
fn referenced(document: &mut Document<'_>) -> Vec<String> {
    let mut documents: Vec<String> = Vec::new();
    rewrite_references(
        document,
        &mut |reference| {
            if let Some((name, _)) = split(reference)
                && !documents.iter().any(|known| known == name)
            {
                documents.push(name.to_owned());
            }
            None
        },
        &|_| None,
    );
    documents
}

impl Document<'_> {
    /// Appends the documents that this one's references name, as `load` returns them, and
    /// rewrites those references to the appended elements. A document that does not load or
    /// parse as SVG leaves its references naming no element, so each property's fallback for
    /// an invalid reference applies.
    pub(crate) fn link_documents(
        &mut self,
        load: &dyn Fn(&str) -> Option<LinkedDocument>,
    ) -> Result<(), Error> {
        let mut pending: Vec<(Option<Url>, String)> = referenced(self)
            .into_iter()
            .map(|document| (None, document))
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        let mut loaded: Vec<LinkedDocument> = Vec::new();
        // The index into `loaded` of each document reference, by its referencing document.
        let mut names: HashMap<(Option<Url>, String), Option<usize>> = HashMap::new();
        while let Some(key) = pending.pop() {
            if names.contains_key(&key) {
                continue;
            }
            let linked = match &key.0 {
                Some(base) => base.join(&key.1).ok().and_then(|url| load(url.as_str())),
                None => load(&key.1),
            };
            let (index, new) = match linked {
                Some(linked) => match loaded.iter().position(|known| known.url == linked.url) {
                    Some(index) => (Some(index), false),
                    None if loaded.len() < MAX_LINKED_DOCUMENTS => {
                        loaded.push(linked);
                        (Some(loaded.len() - 1), true)
                    }
                    None => (None, false),
                },
                None => (None, false),
            };
            names.insert(key, index);
            if let Some(index) = index.filter(|_| new)
                && let Some(mut document) = parse(&loaded[index].text)?
            {
                let base = loaded[index].url.clone();
                pending.extend(
                    referenced(&mut document)
                        .into_iter()
                        .map(|nested| (Some(base.clone()), nested)),
                );
            }
        }
        let mut parsed: Vec<Option<Document<'_>>> = loaded
            .iter()
            .map(|linked| parse(&linked.text))
            .collect::<Result<_, _>>()?;
        let prefix = self.unused_prefix(&parsed);
        let target = |base: Option<&Url>, reference: &str| -> Option<String> {
            let (document, id) = split(reference)?;
            Some(match names.get(&(base.cloned(), document.to_owned())) {
                Some(Some(index)) => format!("{FRAGMENT}{prefix}{index}-{id}"),
                _ => format!("{FRAGMENT}{prefix}"),
            })
        };
        rewrite_references(self, &mut |reference| target(None, reference), &|_| None);
        for (index, document) in parsed.iter_mut().enumerate() {
            let Some(document) = document else {
                continue;
            };
            let base = &loaded[index].url;
            let local = format!("{prefix}{index}-");
            rewrite_references(
                document,
                &mut |reference| match reference.strip_prefix(FRAGMENT) {
                    Some(id) => Some(format!("{FRAGMENT}{local}{id}")),
                    None => target(Some(base), reference),
                },
                // Image resources resolve against their own document.
                &|reference| match reference.strip_prefix(FRAGMENT) {
                    Some(id) => Some(format!("{FRAGMENT}{local}{id}")),
                    None => base.join(reference).ok().map(String::from),
                },
            );
            self.append_document(document, &local);
        }
        super::parse::fix_recursions(self);
        Ok(())
    }

    /// A prefix that no id of this document or of `others` starts with.
    fn unused_prefix(&self, others: &[Option<Document<'_>>]) -> String {
        (0..)
            .map(|attempt| format!("{PREFIX}{attempt}-"))
            .find(|prefix| {
                self.links
                    .keys()
                    .chain(others.iter().flatten().flat_map(|other| other.links.keys()))
                    .all(|id| !id.starts_with(prefix))
            })
            .expect("finitely many ids leave some prefix unused")
    }

    /// Appends the root element of `other`, whose references are rewritten already, after
    /// this document's, with ids prefixed by `local`.
    fn append_document(&mut self, other: &Document<'_>, local: &str) {
        let Some((root, _)) = other.nodes[0].children else {
            return;
        };
        // `other`'s nodes after its root follow this document's; its root is this root.
        let offset = self.nodes.len() - 1;
        let map = |id: NodeId| match id.get_usize() {
            0 => NodeId::from(0),
            index => NodeId::from(index + offset),
        };
        let attribute_offset =
            u32::try_from(self.attrs.len()).expect("attributes fit the node index range");
        for attribute in &other.attrs {
            let value = if attribute.name == AId::Id {
                format!("{local}{}", attribute.value.as_str())
            } else {
                attribute.value.as_str().to_owned()
            };
            self.attrs.push(Attribute {
                name: attribute.name,
                value: roxmltree::StringStorage::new_owned(&value),
                important: attribute.important,
                inherited: attribute.inherited,
            });
        }
        for node in other.nodes.iter().skip(1) {
            self.nodes.push(NodeData {
                effective_zoom: node.effective_zoom,
                parent: node.parent.map(map),
                next_sibling: node.next_sibling.map(map),
                children: node.children.map(|(first, last)| (map(first), map(last))),
                kind: match &node.kind {
                    NodeKind::Root => NodeKind::Root,
                    NodeKind::Element {
                        tag_name,
                        attributes,
                    } => NodeKind::Element {
                        tag_name: *tag_name,
                        attributes: ShortRange::new(
                            attributes.start + attribute_offset,
                            attributes.end + attribute_offset,
                        ),
                    },
                    NodeKind::Text(text) => NodeKind::Text(text.clone()),
                },
            });
        }
        let appended = map(root);
        if let Some((first, last)) = self.nodes[0].children {
            self.nodes[last.get_usize()].next_sibling = Some(appended);
            self.nodes[0].children = Some((first, appended));
        }
        for (id, node) in &other.links {
            self.links.insert(format!("{local}{id}"), map(*node));
        }
    }
}

/// `text` as an SVG document with its own style sheets, `None` when it is not one.
fn parse(text: &Arc<str>) -> Result<Option<Document<'_>>, Error> {
    let Ok(xml) = roxmltree::Document::parse_with_options(text, crate::svg::usvg::xml_options())
    else {
        return Ok(None);
    };
    let Ok(document) = Document::parse_tree(&xml, None) else {
        return Ok(None);
    };
    if let Some(error) = &document.style_error {
        return Err(Error::Style(error.clone()));
    }
    if let Some(limit) = document.css_limit {
        return Err(Error::CssComponentDepthLimit { limit });
    }
    Ok(Some(document))
}
