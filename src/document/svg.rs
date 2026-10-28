//! Standalone XML for `<svg>` elements embedded in HTML, so they share the SVG image pipeline.
use crate::dom::{
    DocumentTree, Element, ElementId, NodeData, SVG_NAMESPACE, TreeRelation, XLINK_NAMESPACE,
    XML_NAMESPACE,
};
use std::{collections::HashMap, fmt::Write, sync::Arc};

/// Serializes the element's subtree as an SVG document with explicit namespaces.
/// Elements and attributes with undeclared prefixes (editor metadata) are omitted.
pub(crate) fn serialize(element: Element<'_>) -> String {
    let mut xml = String::new();
    let mut stack = vec![(element.as_node(), false)];
    while let Some((node, opened)) = stack.pop() {
        match node.value() {
            NodeData::Element(child) if !child.name().contains(':') => {
                if opened {
                    let _ = write!(xml, "</{}>", child.name());
                    continue;
                }
                let _ = write!(xml, "<{}", child.name());
                if node == element.as_node() {
                    let _ = write!(
                        xml,
                        " xmlns=\"{SVG_NAMESPACE}\" xmlns:xlink=\"{XLINK_NAMESPACE}\""
                    );
                }
                for (name, value) in &child.attrs {
                    let prefix = match &*name.ns {
                        "" if !name.local.contains(':') => "",
                        XLINK_NAMESPACE => "xlink:",
                        XML_NAMESPACE => "xml:",
                        _ => continue,
                    };
                    let _ = write!(xml, " {prefix}{}=\"", name.local);
                    escape(&mut xml, value, true);
                    xml.push('"');
                }
                xml.push('>');
                stack.push((node, true));
                stack.extend(
                    node.children(TreeRelation::Selector)
                        .rev()
                        .map(|child| (child, false)),
                );
            }
            NodeData::Text(text) => escape(&mut xml, text, false),
            _ => {}
        }
    }
    xml
}

/// SVG elements that CSS `url()` references select: Filter Effects 1 §5 `filter` and CSS
/// Masking 1 §7.1 `mask` elements.
const REFERENCED_ELEMENTS: [&str; 2] = ["filter", "mask"];

/// The document's referenceable SVG resources by `id`, each with the standalone XML of the
/// outermost `<svg>` holding it, so that references inside that SVG resolve too.
#[derive(Default)]
pub(crate) struct SvgDefinitions(HashMap<String, SvgDefinition>);

#[derive(Clone)]
pub(crate) struct SvgDefinition {
    /// `filter` or `mask`.
    pub(crate) element: &'static str,
    pub(crate) xml: Arc<str>,
}

impl SvgDefinitions {
    pub(crate) fn collect(tree: &DocumentTree) -> Self {
        let mut roots: HashMap<ElementId, Arc<str>> = HashMap::new();
        let mut definitions = HashMap::new();
        for element in tree.elements() {
            let Some(name) = REFERENCED_ELEMENTS
                .into_iter()
                .find(|name| element.namespace() == SVG_NAMESPACE && element.name() == *name)
            else {
                continue;
            };
            let Some(id) = element.attr("id").filter(|id| !id.is_empty()) else {
                continue;
            };
            let Some(root) = element
                .ancestors(TreeRelation::Selector)
                .filter(|ancestor| {
                    ancestor.namespace() == SVG_NAMESPACE && ancestor.name() == "svg"
                })
                .last()
            else {
                continue;
            };
            let xml = roots
                .entry(root.id())
                .or_insert_with(|| serialize(root).into())
                .clone();
            definitions
                .entry(id.to_owned())
                .or_insert(SvgDefinition { element: name, xml });
        }
        Self(definitions)
    }
    /// The definition a same-document `url(#id)` reference names.
    pub(crate) fn get(&self, reference: &str) -> Option<&SvgDefinition> {
        self.0.get(reference.strip_prefix('#')?)
    }
}

fn escape(xml: &mut String, text: &str, attribute: bool) {
    for ch in text.chars() {
        match ch {
            '&' => xml.push_str("&amp;"),
            '<' => xml.push_str("&lt;"),
            '>' => xml.push_str("&gt;"),
            '"' if attribute => xml.push_str("&quot;"),
            ch => xml.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::{DocumentTree, Selector};

    #[test]
    fn html_parsed_svg_becomes_namespaced_xml_without_undeclared_prefixes() {
        let html = DocumentTree::parse(
            "<p><svg viewBox='0 0 10 10' xmlns:sodipodi='x'><sodipodi:namedview/><linearGradient id=g><stop offset=0 /></linearGradient><use xlink:href='#g' sodipodi:role='a'/><text>A &amp; B &lt;</text><!--note--></svg></p>",
        ).expect("parse test HTML");
        let selector = Selector::parse("svg").unwrap();
        let xml = serialize(html.select(&selector).next().unwrap());
        assert_eq!(
            xml,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 10 10\"><linearGradient id=\"g\"><stop offset=\"0\"></stop></linearGradient><use xlink:href=\"#g\"></use><text>A &amp; B &lt;</text></svg>",
            "HTML adjusts SVG name casing and namespaces; XML output keeps them, escapes text, and drops editor-only prefixes"
        );
    }
}
