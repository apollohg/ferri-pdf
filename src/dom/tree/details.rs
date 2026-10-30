//! The `details` element's internal shadow tree (HTML §15.5.5): a slot for its first
//! `summary` child, falling back to a default summary, and a slot for its other children,
//! which the `::details-content` pseudo-element represents.
use super::{DocumentTree, Element, TreeRelation, internal::InternalNode};
use crate::dom::{HTML_NAMESPACE, NodeData};
use ego_tree::NodeId;
use scraper::ElementRef;

const DETAILS_ELEMENT: &str = "details";
const SUMMARY_ELEMENT: &str = "summary";
const OPEN_ATTRIBUTE: &str = "open";
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
        let (children, summary, open) = {
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
            (children, summary, details.attr(OPEN_ATTRIBUTE).is_some())
        };
        let (content_style, summary_style) = if open {
            (OPEN_CONTENT_STYLE, OPEN_SUMMARY_STYLE)
        } else {
            (CLOSED_CONTENT_STYLE, CLOSED_SUMMARY_STYLE)
        };
        let (summary, content): (Vec<_>, Vec<_>) = children
            .into_iter()
            .partition(|child| Some(*child) == summary);
        let slots = self.attach_internal_shadow(
            host,
            vec![
                InternalNode::Slot {
                    style: None,
                    assigned: summary,
                    fallback: vec![InternalNode::Element {
                        name: SUMMARY_ELEMENT,
                        style: Some(summary_style),
                        children: vec![InternalNode::Text(DEFAULT_SUMMARY.into())],
                    }],
                },
                InternalNode::Slot {
                    style: Some(content_style),
                    assigned: content,
                    fallback: Vec::new(),
                },
            ],
        );
        self.shadow.details_content.insert(slots[1], host);
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
