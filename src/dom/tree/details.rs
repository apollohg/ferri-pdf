//! The `details` element's internal shadow tree (HTML §15.5.5): a slot for its first
//! `summary` child, falling back to a default summary, and a slot for its other children,
//! which the `::details-content` pseudo-element represents.
use super::{DocumentTree, Element, TreeRelation};
use crate::dom::{HTML_NAMESPACE, NodeData, shadow::ShadowRoot};
use ego_tree::NodeId;
use html5ever::{Attribute, QualName};
use scraper::{ElementRef, node};

const DETAILS_ELEMENT: &str = "details";
const SUMMARY_ELEMENT: &str = "summary";
const SLOT_ELEMENT: &str = "slot";
const OPEN_ATTRIBUTE: &str = "open";
const STYLE_ATTRIBUTE: &str = "style";
/// The implementation-defined text of the default summary.
const DEFAULT_SUMMARY: &str = "Details";
/// The second slot's `style` attribute, which skips its contents while closed.
const CLOSED_CONTENT_STYLE: &str = "display: block; content-visibility: hidden;";
const OPEN_CONTENT_STYLE: &str = "display: block;";
/// The `:host summary` and `:host([open]) summary` rules, which within the shadow tree
/// reach only the default summary.
const CLOSED_SUMMARY_STYLE: &str =
    "display: list-item; counter-increment: list-item 0; list-style: disclosure-closed inside;";
const OPEN_SUMMARY_STYLE: &str =
    "display: list-item; counter-increment: list-item 0; list-style: disclosure-open inside;";

fn html_element(name: &str, style: Option<&str>) -> NodeData {
    NodeData::Element(node::Element::new(
        QualName::new(None, HTML_NAMESPACE.into(), name.into()),
        style
            .map(|style| Attribute {
                name: QualName::new(None, "".into(), STYLE_ATTRIBUTE.into()),
                value: style.into(),
            })
            .into_iter()
            .collect(),
    ))
}

impl DocumentTree {
    /// Gives every `details` element of the document and its shadow trees its internal
    /// shadow tree, including those cloned into use-element shadow trees.
    pub(crate) fn attach_details_shadow_trees(&mut self) {
        let hosts: Vec<NodeId> = self
            .shadow_including_elements()
            .filter(|element| {
                element.namespace() == HTML_NAMESPACE && element.name() == DETAILS_ELEMENT
            })
            .map(|element| element.raw.id())
            .collect();
        for host in hosts {
            self.attach_details(host);
        }
    }

    fn attach_details(&mut self, host: NodeId) {
        let (children, summary, open, depth) = {
            let details = Element {
                tree: self,
                raw: ElementRef::wrap(self.html.tree.get(host).unwrap())
                    .expect("details hosts are elements"),
            };
            let children: Vec<NodeId> = details
                .raw
                .children()
                .filter(|child| matches!(child.value(), NodeData::Element(_) | NodeData::Text(_)))
                .map(|child| child.id())
                .collect();
            let summary = details
                .children(TreeRelation::Selector)
                .find(|child| {
                    child.namespace() == HTML_NAMESPACE && child.name() == SUMMARY_ELEMENT
                })
                .map(|child| child.raw.id());
            let depth = details
                .shadow_host()
                .map_or(1, |parent| self.shadow.roots[&parent.raw.id()].depth + 1);
            (
                children,
                summary,
                details.attr(OPEN_ATTRIBUTE).is_some(),
                depth,
            )
        };
        let (content_style, summary_style) = if open {
            (OPEN_CONTENT_STYLE, OPEN_SUMMARY_STYLE)
        } else {
            (CLOSED_CONTENT_STYLE, CLOSED_SUMMARY_STYLE)
        };
        self.sibling_positions.get_mut().clear();
        let tree = &mut self.html.tree;
        let fragment = tree.orphan(NodeData::Fragment).id();
        let summary_slot = tree
            .get_mut(fragment)
            .unwrap()
            .append(html_element(SLOT_ELEMENT, None))
            .id();
        let content_slot = tree
            .get_mut(fragment)
            .unwrap()
            .append(html_element(SLOT_ELEMENT, Some(content_style)))
            .id();
        let default_summary = tree
            .get_mut(summary_slot)
            .unwrap()
            .append(html_element(SUMMARY_ELEMENT, Some(summary_style)))
            .id();
        let label = tree
            .get_mut(default_summary)
            .unwrap()
            .append(NodeData::Text(node::Text {
                text: DEFAULT_SUMMARY.into(),
            }))
            .id();
        let created = [summary_slot, content_slot, default_summary, label];
        let shadow = &mut self.shadow;
        for node in created {
            shadow.scopes.insert(node, fragment);
        }
        let (summary, content): (Vec<_>, Vec<_>) = children
            .into_iter()
            .partition(|child| Some(*child) == summary);
        for (slot, assigned) in [(summary_slot, &summary), (content_slot, &content)] {
            for child in assigned {
                shadow.assigned.insert(*child, slot);
            }
            if !assigned.is_empty() {
                shadow.populated_slots.insert(slot);
            }
        }
        if summary.is_empty() {
            shadow.fallback_slots.insert(default_summary, summary_slot);
            shadow.populated_slots.insert(summary_slot);
        }
        shadow.slots.insert(summary_slot, summary);
        shadow.slots.insert(content_slot, content);
        shadow.details_content.insert(content_slot, host);
        shadow.roots.insert(host, ShadowRoot { fragment, depth });
        shadow.hosts.insert(fragment, host);
        shadow.order_contexts([fragment]);
        self.cache_initial_states(created);
    }
}

impl Element<'_> {
    /// The `details` element whose `::details-content` pseudo-element this element is.
    pub(crate) fn details_content_host(self) -> Option<Self> {
        let host = *self.tree.shadow.details_content.get(&self.raw.id())?;
        ElementRef::wrap(self.tree.html.tree.get(host)?).map(|raw| Element {
            tree: self.tree,
            raw,
        })
    }
}
