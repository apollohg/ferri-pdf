//! The customizable `select` element: the `selectedcontent` clones that parsing leaves
//! (HTML §4.10.17), and the internal shadow trees of `select`, `optgroup` and `option`
//! (HTML §15.5.16), which render only with base appearance.
use super::{DocumentTree, Element, TreeRelation, internal::InternalNode};
use crate::dom::{HTML_NAMESPACE, NodeData};
use ego_tree::NodeId;
use scraper::ElementRef;

const SELECT_ELEMENT: &str = "select";
const OPTION_ELEMENT: &str = "option";
const OPTGROUP_ELEMENT: &str = "optgroup";
const SELECTEDCONTENT_ELEMENT: &str = "selectedcontent";
const BUTTON_ELEMENT: &str = "button";
const LEGEND_ELEMENT: &str = "legend";
const DIV_ELEMENT: &str = "div";
const SPAN_ELEMENT: &str = "span";
const MULTIPLE_ATTRIBUTE: &str = "multiple";
const LABEL_ATTRIBUTE: &str = "label";
/// The select popover is a closed popover, which generates no box (HTML §15.3.1:
/// `[popover]:not(:popover-open)`).
const CLOSED_POPOVER_STYLE: &str = "display: none;";
/// HTML §15.5.16: the styles of the optgroup label element.
const OPTGROUP_LABEL_STYLE: &str = "padding-inline: 0.5em; min-block-size: 1lh;";

fn named(element: Element<'_>, name: &str) -> bool {
    element.namespace() == HTML_NAMESPACE && element.name() == name
}

impl DocumentTree {
    fn element_at(&self, node: NodeId) -> Element<'_> {
        Element {
            tree: self,
            raw: ElementRef::wrap(self.html.tree.get(node).unwrap()).expect("an element node"),
        }
    }

    /// The host's child elements and text, which its slots take.
    fn slotted_children(&self, host: NodeId) -> Vec<NodeId> {
        self.html
            .tree
            .get(host)
            .unwrap()
            .children()
            .filter(|child| matches!(child.value(), NodeData::Element(_) | NodeData::Text(_)))
            .map(|child| child.id())
            .collect()
    }

    /// HTML §4.10.17: the state parsing leaves each `selectedcontent` element in. Every
    /// enabled one holds clones of the children of its `select`'s first selected option, or
    /// nothing; scripting never changes the selection afterwards.
    pub(crate) fn update_selectedcontents(&mut self) {
        let mut updates = Vec::new();
        for select in self.shadow_including_elements() {
            if !named(select, SELECT_ELEMENT) || select.attr(MULTIPLE_ATTRIBUTE).is_some() {
                continue;
            }
            let option = select
                .initial_selected_options()
                .first()
                .map(|option| option.raw.id());
            for node in select.raw.descendants().skip(1) {
                let Some(element) = ElementRef::wrap(node).map(|raw| Element { tree: self, raw })
                else {
                    continue;
                };
                if named(element, SELECTEDCONTENT_ELEMENT)
                    && Self::selectedcontent_select(element) == Some(select)
                {
                    updates.push((node.id(), option));
                }
            }
        }
        if updates.is_empty() {
            return;
        }
        self.sibling_positions.get_mut().clear();
        self.tables.get_mut().clear();
        let mut created = Vec::new();
        for (target, option) in updates {
            let previous: Vec<_> = self
                .html
                .tree
                .get(target)
                .unwrap()
                .children()
                .map(|child| child.id())
                .collect();
            for child in previous {
                self.html.tree.get_mut(child).unwrap().detach();
            }
            let scope = self.shadow.scopes.get(&target).copied();
            let Some(option) = option else {
                continue;
            };
            let sources: Vec<_> = self
                .html
                .tree
                .get(option)
                .unwrap()
                .descendants()
                .skip(1)
                .map(|node| node.id())
                .collect();
            let mut clones = std::collections::HashMap::with_capacity(sources.len());
            for source in sources {
                let parent = self.html.tree.get(source).unwrap().parent().unwrap().id();
                let parent = if parent == option {
                    target
                } else {
                    clones[&parent]
                };
                let data = self.html.tree.get(source).unwrap().value().clone();
                let clone = self.html.tree.get_mut(parent).unwrap().append(data).id();
                if let Some(scope) = scope {
                    self.shadow.scopes.insert(clone, scope);
                }
                clones.insert(source, clone);
                created.push(clone);
            }
        }
        self.cache_initial_states(created);
    }

    /// HTML §4.10.17 "recalculate a selectedcontent element's disabledness": the `select`
    /// whose selection the element shows, or `None` while it is disabled.
    fn selectedcontent_select(selectedcontent: Element<'_>) -> Option<Element<'_>> {
        let mut nearest = None;
        for ancestor in selectedcontent.ancestors(TreeRelation::Selector) {
            if named(ancestor, SELECT_ELEMENT) {
                if nearest.is_some() {
                    return None;
                }
                nearest = Some(ancestor);
            } else if named(ancestor, OPTION_ELEMENT) || named(ancestor, SELECTEDCONTENT_ELEMENT) {
                return None;
            }
        }
        nearest
    }

    /// Gives every `select`, and every `optgroup` and `option` with an ancestor `select`, its
    /// internal shadow tree (HTML §15.5.16). Native and primitive appearance render the
    /// control's state instead, so only base appearance lays these out.
    pub(crate) fn attach_select_shadow_trees(&mut self) {
        let hosts: Vec<(NodeId, &'static str)> = self
            .shadow_including_elements()
            .filter_map(|element| {
                let name = [SELECT_ELEMENT, OPTGROUP_ELEMENT, OPTION_ELEMENT]
                    .into_iter()
                    .find(|name| named(element, name))?;
                (name == SELECT_ELEMENT || element.nearest_select().is_some())
                    .then(|| (element.raw.id(), name))
            })
            .collect();
        for (host, name) in hosts {
            let children = self.slotted_children(host);
            let shadow = match name {
                SELECT_ELEMENT => self.select_shadow(host, children),
                OPTGROUP_ELEMENT => self.optgroup_shadow(host, children),
                _ => self.option_shadow(host, children),
            };
            self.attach_internal_shadow(host, shadow);
        }
    }

    /// The first child element of `host` if it is named `name`.
    fn first_child_named(&self, host: NodeId, name: &str) -> Option<NodeId> {
        self.element_at(host)
            .children(TreeRelation::Selector)
            .next()
            .filter(|child| named(*child, name))
            .map(|child| child.raw.id())
    }

    /// A list box's slot takes all its children; a drop-down box's button slot takes a first
    /// child `button`, falling back to the select fallback button text, and its closed select
    /// popover holds a slot for the rest.
    fn select_shadow(&self, host: NodeId, children: Vec<NodeId>) -> Vec<InternalNode> {
        let select = self.element_at(host);
        if select.is_list_box() {
            return vec![InternalNode::Slot {
                style: None,
                assigned: children,
                fallback: Vec::new(),
            }];
        }
        // HTML §4.10.7 "clone selected option into select button".
        let text = select
            .initial_selected_options()
            .first()
            .map(|option| option.option_label())
            .unwrap_or_default();
        let button = self.first_child_named(host, BUTTON_ELEMENT);
        let (button, rest): (Vec<_>, Vec<_>) = children
            .into_iter()
            .partition(|child| Some(*child) == button);
        vec![
            InternalNode::Slot {
                style: None,
                assigned: button,
                fallback: vec![InternalNode::Element {
                    name: DIV_ELEMENT,
                    style: None,
                    children: vec![InternalNode::Text(text)],
                }],
            },
            InternalNode::Element {
                name: DIV_ELEMENT,
                style: Some(CLOSED_POPOVER_STYLE),
                children: vec![InternalNode::Slot {
                    style: None,
                    assigned: rest,
                    fallback: Vec::new(),
                }],
            },
        ]
    }

    /// An optgroup legend slot taking a first child `legend`, falling back to the optgroup
    /// label element, and an optgroup contents slot for the rest.
    fn optgroup_shadow(&self, host: NodeId, children: Vec<NodeId>) -> Vec<InternalNode> {
        let label = self
            .element_at(host)
            .attr(LABEL_ATTRIBUTE)
            .unwrap_or_default()
            .to_owned();
        let legend = self.first_child_named(host, LEGEND_ELEMENT);
        let (legend, rest): (Vec<_>, Vec<_>) = children
            .into_iter()
            .partition(|child| Some(*child) == legend);
        vec![
            InternalNode::Slot {
                style: None,
                assigned: legend,
                fallback: vec![InternalNode::Element {
                    name: DIV_ELEMENT,
                    style: Some(OPTGROUP_LABEL_STYLE),
                    children: vec![InternalNode::Text(label)],
                }],
            },
            InternalNode::Slot {
                style: None,
                assigned: rest,
                fallback: Vec::new(),
            },
        ]
    }

    /// An option contents slot taking the option's children unless a non-empty `label`
    /// attribute replaces them with the option label element.
    fn option_shadow(&self, host: NodeId, children: Vec<NodeId>) -> Vec<InternalNode> {
        let label = self
            .element_at(host)
            .attr(LABEL_ATTRIBUTE)
            .unwrap_or_default();
        let assigned = if label.is_empty() {
            children
        } else {
            Vec::new()
        };
        vec![InternalNode::Slot {
            style: None,
            assigned,
            fallback: vec![InternalNode::Element {
                name: SPAN_ELEMENT,
                style: None,
                children: vec![InternalNode::Text(label.to_owned())],
            }],
        }]
    }
}

impl Element<'_> {
    /// Whether this is a `select` element rendered with base appearance.
    pub(crate) fn is_base_select(self) -> bool {
        named(self, SELECT_ELEMENT) && self.tree.base_selects.borrow().contains(&self.raw.id())
    }

    /// Records that this `select` element's computed `appearance` gives it base appearance
    /// (CSS UI 4 §7.2: `base` and `base-select` on a `select`).
    pub(crate) fn set_base_select(self) {
        self.tree.base_selects.borrow_mut().insert(self.raw.id());
    }

    /// The nearest ancestor `select`, which an `option` or `optgroup` belongs to.
    fn nearest_select(self) -> Option<Self> {
        self.ancestors(TreeRelation::Selector)
            .find(|ancestor| named(*ancestor, SELECT_ELEMENT))
    }

    /// CSS Forms 1 §4.3: an `option` rendered with base appearance generates `::checkmark`.
    /// HTML §15.5.16 renders it so where its nearest ancestor `select`'s contents are, which
    /// a closed drop-down box's select popover never is in a static rendering: only a list
    /// box's contents are.
    pub(crate) fn generates_checkmark(self) -> bool {
        named(self, OPTION_ELEMENT)
            && self
                .nearest_select()
                .is_some_and(|select| select.is_base_select() && select.is_list_box())
    }

    /// CSS Forms 1 §4.1: `::picker-icon` exists on a control with base appearance that opens
    /// a picker: a drop-down box, not a list box.
    pub(crate) fn generates_picker_icon(self) -> bool {
        self.is_base_select() && !self.is_list_box()
    }
}
