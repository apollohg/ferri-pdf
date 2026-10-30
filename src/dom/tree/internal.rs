//! Internal shadow trees that HTML gives some elements for rendering (HTML §15.5): slots
//! taking the host's child nodes, with fallback content in internal elements.
use super::DocumentTree;
use crate::dom::{HTML_NAMESPACE, NodeData, shadow::ShadowRoot};
use ego_tree::NodeId;
use html5ever::{Attribute, QualName};
use scraper::node;

const SLOT_ELEMENT: &str = "slot";
const STYLE_ATTRIBUTE: &str = "style";

/// A node of an internal shadow tree.
pub(super) enum InternalNode {
    /// An HTML element, styled by its `style` attribute.
    Element {
        name: &'static str,
        style: Option<&'static str>,
        children: Vec<InternalNode>,
    },
    Text(String),
    /// A slot taking `assigned` child nodes of the host; its `fallback` content renders when
    /// it takes none.
    Slot {
        style: Option<&'static str>,
        assigned: Vec<NodeId>,
        fallback: Vec<InternalNode>,
    },
}

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
    /// Attaches `children` as the shadow tree of `host`, returning its slots in tree order.
    pub(super) fn attach_internal_shadow(
        &mut self,
        host: NodeId,
        children: Vec<InternalNode>,
    ) -> Vec<NodeId> {
        let depth = self
            .element(super::ElementId {
                document: self.identity,
                node: host,
            })
            .and_then(super::Element::shadow_host)
            .map_or(1, |parent| self.shadow.roots[&parent.raw.id()].depth + 1);
        self.sibling_positions.get_mut().clear();
        let fragment = self.html.tree.orphan(NodeData::Fragment).id();
        let mut created = Vec::new();
        let mut slots = Vec::new();
        let mut pending: Vec<_> = children
            .into_iter()
            .rev()
            .map(|child| (fragment, child))
            .collect();
        while let Some((parent, child)) = pending.pop() {
            let (data, children, slot) = match child {
                InternalNode::Element {
                    name,
                    style,
                    children,
                } => (html_element(name, style), children, None),
                InternalNode::Text(text) => (
                    NodeData::Text(node::Text { text: text.into() }),
                    Vec::new(),
                    None,
                ),
                InternalNode::Slot {
                    style,
                    assigned,
                    fallback,
                } => (html_element(SLOT_ELEMENT, style), fallback, Some(assigned)),
            };
            let id = self.html.tree.get_mut(parent).unwrap().append(data).id();
            created.push(id);
            if let Some(assigned) = slot {
                let shadow = &mut self.shadow;
                for child in &assigned {
                    shadow.assigned.insert(*child, id);
                }
                // The fallback content is the slot's children, recorded once appended.
                let falls_back = assigned.is_empty() && !children.is_empty();
                if falls_back || !assigned.is_empty() {
                    shadow.populated_slots.insert(id);
                }
                slots.push((id, falls_back));
                shadow.slots.insert(id, assigned);
            }
            pending.extend(children.into_iter().rev().map(|child| (id, child)));
        }
        for &(slot, falls_back) in &slots {
            if falls_back {
                let fallback: Vec<_> = self
                    .html
                    .tree
                    .get(slot)
                    .unwrap()
                    .children()
                    .map(|child| child.id())
                    .collect();
                for child in fallback {
                    self.shadow.fallback_slots.insert(child, slot);
                }
            }
        }
        for &node in &created {
            self.shadow.scopes.insert(node, fragment);
        }
        self.shadow
            .roots
            .insert(host, ShadowRoot { fragment, depth });
        self.shadow.hosts.insert(fragment, host);
        self.shadow.order_contexts([fragment]);
        self.cache_initial_states(created);
        slots.into_iter().map(|(slot, _)| slot).collect()
    }
}
