//! Standalone XML for `<svg>` elements embedded in HTML, so they share the SVG image pipeline.
use crate::dom::{Element, NodeData, SVG_NAMESPACE, TreeRelation, XLINK_NAMESPACE, XML_NAMESPACE};
use std::fmt::Write;

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
