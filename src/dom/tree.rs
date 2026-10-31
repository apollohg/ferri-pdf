use crate::dom::{HTML_NAMESPACE, NodeData, QuirksMode, Selector};
use ego_tree::{NodeId, NodeRef, iter::Edge};
use scraper::{ElementRef, Html};
use selectors::matching::SelectorCaches;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

mod details;
mod instances;
mod internal;
mod select;
pub(crate) mod table;
mod xml;
pub(crate) use xml::XmlTree;

pub(crate) const DOCUMENT_CONTEXT_ORDER: usize = 0;
const FIRST_DOCUMENT_ID: u64 = 1;
static NEXT_DOCUMENT_ID: AtomicU64 = AtomicU64::new(FIRST_DOCUMENT_ID);
#[cfg(test)]
const TEMPLATE_ELEMENT: &str = "template";
const HTML_ELEMENT: &str = "html";
const BODY_ELEMENT: &str = "body";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TreeRelation {
    Selector,
    Inheritance,
    Formatting,
}

/// A handle is qualified by its owning document, even after the tree is moved.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct ElementId {
    document: u64,
    node: NodeId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct TreeScopeId {
    document: u64,
    root: NodeId,
}

#[derive(Debug)]
pub(crate) struct DocumentTree {
    html: Html,
    pub(crate) is_html_document: bool,
    identity: u64,
    pub(crate) document_url: Option<url::Url>,
    pub(crate) base_url: Option<url::Url>,
    pub(crate) default_language: String,
    pub(crate) shadow: crate::dom::shadow::ShadowState,
    pub(crate) initial_states: HashMap<NodeId, crate::dom::InitialState>,
    /// `object` elements that represent an image or their content navigable rather than their
    /// fallback content (HTML §4.8.7).
    embedding_objects: HashSet<NodeId>,
    sibling_positions: RefCell<HashMap<(NodeId, TreeRelation), (usize, usize)>>,
    /// Each table element's columns, formed on first use.
    tables: RefCell<HashMap<NodeId, std::sync::Arc<table::TableModel>>>,
    /// The `select` elements whose computed `appearance` gives them base appearance, which
    /// style computation records before their user-agent styles match (HTML §15.5.16).
    base_selects: RefCell<HashSet<NodeId>>,
    /// The positions of scroll markers relative to their groups' active markers, which the
    /// last layout determined (CSS Overflow 5 §3.1.8).
    pub(crate) scroll_markers: RefCell<crate::dom::ScrollMarkerStates>,
    /// Nodes cloned into use-element shadow trees so far.
    instance_nodes: usize,
    /// The tree limits building the tree reached, past which it dropped content.
    pub(crate) limits: Vec<crate::dom::DomError>,
}

impl DocumentTree {
    pub(crate) fn from_parsed(html: Html, shadow: crate::dom::shadow::ShadowState) -> Self {
        Self::from_parsed_with_mode(html, shadow, true)
    }
    fn from_parsed_with_mode(
        html: Html,
        shadow: crate::dom::shadow::ShadowState,
        is_html_document: bool,
    ) -> Self {
        #[allow(deprecated, reason = "try_update is unavailable before Rust 1.99")]
        let identity = NEXT_DOCUMENT_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("document identity space exhausted");
        let mut tree = Self {
            html,
            is_html_document,
            identity,
            document_url: None,
            base_url: None,
            default_language: String::new(),
            shadow,
            initial_states: HashMap::new(),
            embedding_objects: HashSet::new(),
            sibling_positions: RefCell::default(),
            tables: RefCell::default(),
            base_selects: RefCell::default(),
            scroll_markers: RefCell::default(),
            instance_nodes: 0,
            limits: Vec::new(),
        };
        let connected = if tree.shadow.roots.is_empty() {
            Vec::new()
        } else {
            tree.shadow_including_elements()
                .filter_map(|element| element.shadow_root().map(|root| root.raw.id()))
                .collect()
        };
        tree.shadow.order_contexts(connected);
        tree.default_language = tree
            .elements()
            .filter(|element| element.namespace() == HTML_NAMESPACE && element.name() == "meta")
            .filter(|element| {
                element
                    .attr("http-equiv")
                    .is_some_and(|value| value.eq_ignore_ascii_case("content-language"))
            })
            .filter_map(|element| element.attr("content"))
            .filter(|value| !value.contains(','))
            .filter_map(|value| {
                value
                    .split(crate::dom::state::is_html_whitespace)
                    .find(|word| !word.is_empty())
            })
            .last()
            .unwrap_or_default()
            .to_owned();
        let nodes: Vec<_> = tree.html.tree.nodes().map(|node| node.id()).collect();
        tree.cache_initial_states(nodes);
        tree
    }
    fn cache_initial_states(&mut self, nodes: impl IntoIterator<Item = NodeId>) {
        let mut states = std::mem::take(&mut self.initial_states);
        crate::dom::state::prepare_initial_states(
            nodes.into_iter().filter_map(|id| {
                ElementRef::wrap(self.html.tree.get(id)?).map(|raw| Element { tree: self, raw })
            }),
            &mut states,
            &self.default_language,
        );
        self.initial_states = states;
    }
    pub(crate) fn set_document_urls(&mut self, document: Option<url::Url>, base: url::Url) {
        self.document_url = document;
        self.base_url = Some(base);
    }
    pub(crate) fn quirks_mode(&self) -> QuirksMode {
        self.html.quirks_mode
    }
    /// Records what the `object` element `id` represents once its resource is examined.
    pub(crate) fn set_object_embeds(&mut self, id: ElementId) {
        debug_assert_eq!(
            id.document, self.identity,
            "object belongs to this document"
        );
        self.embedding_objects.insert(id.node);
    }
    pub(crate) fn root_element(&self) -> Element<'_> {
        Element {
            tree: self,
            raw: self.html.root_element(),
        }
    }
    pub(crate) fn body_element(&self) -> Option<Element<'_>> {
        let root = self.root_element();
        (root.namespace() == HTML_NAMESPACE && root.name() == HTML_ELEMENT)
            .then(|| {
                root.children(TreeRelation::Selector).find(|child| {
                    child.namespace() == HTML_NAMESPACE && child.name() == BODY_ELEMENT
                })
            })
            .flatten()
    }
    pub(crate) fn element(&self, id: ElementId) -> Option<Element<'_>> {
        if id.document != self.identity {
            return None;
        }
        ElementRef::wrap(self.html.tree.get(id.node)?).map(|raw| Element { tree: self, raw })
    }
    pub(crate) fn elements(&self) -> impl Iterator<Item = Element<'_>> {
        Node {
            tree: self,
            raw: self.html.tree.root(),
        }
        .inclusive_descendants(TreeRelation::Selector)
        .filter_map(Node::element)
    }
    pub(crate) fn shadow_including_elements(&self) -> impl Iterator<Item = Element<'_>> {
        let ordinary = self
            .shadow
            .roots
            .is_empty()
            .then(|| self.elements())
            .into_iter()
            .flatten();
        let mut pending = Vec::new();
        if !self.shadow.roots.is_empty() {
            pending.push(self.html.tree.root().id());
        }
        let shadow = std::iter::from_fn(move || {
            loop {
                let id = pending.pop()?;
                let raw = self.html.tree.get(id).unwrap();
                pending.extend(
                    raw.children()
                        .rev()
                        .filter(|node| !node.value().is_fragment())
                        .map(|node| node.id()),
                );
                if let Some(root) = self.shadow.roots.get(&id) {
                    pending.extend(
                        self.html
                            .tree
                            .get(root.fragment)
                            .unwrap()
                            .children()
                            .rev()
                            .map(|node| node.id()),
                    );
                }
                if let Some(raw) = ElementRef::wrap(raw) {
                    return Some(Element { tree: self, raw });
                }
            }
        });
        ordinary.chain(shadow)
    }
    #[cfg(test)]
    pub(crate) fn select<'a, 's>(
        &'a self,
        selector: &'s Selector,
    ) -> impl Iterator<Item = Element<'a>> + use<'a, 's> {
        let mut caches = SelectorCaches::default();
        self.elements()
            .filter(move |element| selector.matches(*element, &mut caches, None))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Element<'a> {
    pub(crate) tree: &'a DocumentTree,
    pub(crate) raw: ElementRef<'a>,
}
impl PartialEq for Element<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}
impl Eq for Element<'_> {}
impl fmt::Debug for Element<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Element")
            .field("id", &self.id())
            .field("name", &self.name())
            .finish()
    }
}
impl<'a> Element<'a> {
    pub(crate) fn quirks_mode(self) -> QuirksMode {
        self.tree.quirks_mode()
    }
    pub(crate) fn id(self) -> ElementId {
        ElementId {
            document: self.tree.identity,
            node: self.raw.id(),
        }
    }
    /// An `object` element representing an image or its content navigable, not its fallback.
    pub(crate) fn object_embeds(self) -> bool {
        self.tree.embedding_objects.contains(&self.raw.id())
    }
    pub(crate) fn tree_scope(self) -> TreeScopeId {
        TreeScopeId {
            document: self.tree.identity,
            root: self
                .tree
                .shadow
                .scopes
                .get(&self.raw.id())
                .copied()
                .unwrap_or(self.tree.html.tree.root().id()),
        }
    }
    /// The tree scope whose style sheets style the element: in a use-element shadow tree, that
    /// of the referenced element (SVG 2 §5.5.3); elsewhere its own.
    pub(crate) fn stylesheet_scope(self) -> TreeScopeId {
        let scope = self.tree_scope();
        self.tree
            .shadow
            .instance_sources
            .get(&scope.root)
            .map_or(scope, |&root| TreeScopeId {
                document: scope.document,
                root,
            })
    }
    /// The document's own style sheets style the element.
    pub(crate) fn styled_by_document(self) -> bool {
        self.stylesheet_scope().root == self.tree.html.tree.root().id()
    }
    pub(crate) fn shadow_root(self) -> Option<Node<'a>> {
        let root = self.tree.shadow.roots.get(&self.raw.id())?;
        Some(Node {
            tree: self.tree,
            raw: self.tree.html.tree.get(root.fragment).unwrap(),
        })
    }
    pub(crate) fn is_in_document_tree(self) -> bool {
        self.tree_scope().root == self.tree.html.tree.root().id()
    }
    pub(crate) fn encapsulation_order(self) -> usize {
        if self.is_in_document_tree() {
            DOCUMENT_CONTEXT_ORDER
        } else {
            self.tree.shadow.context_order[&self.tree_scope().root]
        }
    }
    pub(crate) fn scope_host(self, scope: TreeScopeId) -> Option<Self> {
        if scope.document != self.tree.identity {
            return None;
        }
        let host = self.tree.shadow.hosts.get(&scope.root)?;
        ElementRef::wrap(self.tree.html.tree.get(*host)?).map(|raw| Self {
            tree: self.tree,
            raw,
        })
    }
    pub(crate) fn shadow_host(self) -> Option<Self> {
        self.scope_host(self.tree_scope())
    }
    pub(crate) fn alias_slot(self) -> Option<Self> {
        let node = self.tree.shadow.alias_slot(self.raw.id())?;
        self.tree.element(ElementId {
            document: self.tree.identity,
            node,
        })
    }
    pub(crate) fn shadow_including_parent(self) -> Option<Self> {
        self.parent(TreeRelation::Selector)
            .or_else(|| self.shadow_host())
    }
    pub(crate) fn value(self) -> &'a scraper::node::Element {
        self.raw.value()
    }
    pub(crate) fn name(self) -> &'a str {
        self.value().name()
    }
    pub(crate) fn namespace(self) -> &'a str {
        self.value().name.ns.as_ref()
    }
    pub(crate) fn attr(self, name: &str) -> Option<&'a str> {
        self.raw.attr(name)
    }
    pub(crate) fn is_html_element_in_html_document(self) -> bool {
        self.tree.is_html_document && self.namespace() == HTML_NAMESPACE
    }
    /// CSS attribute lookup by namespace and local name, with document-language sensitivity.
    pub(crate) fn attr_in_namespace(self, namespace: Option<&str>, name: &str) -> Option<&'a str> {
        let insensitive = self.is_html_element_in_html_document();
        self.value()
            .attrs
            .iter()
            .find(|(attribute, _)| {
                namespace.map_or_else(
                    || attribute.ns.is_empty(),
                    |namespace| attribute.ns.as_ref() == namespace,
                ) && if insensitive {
                    attribute.local.as_ref().eq_ignore_ascii_case(name)
                } else {
                    attribute.local.as_ref() == name
                }
            })
            .map(|(_, value)| value.as_ref())
    }
    pub(crate) fn as_node(self) -> Node<'a> {
        Node {
            tree: self.tree,
            raw: *self.raw,
        }
    }
    pub(crate) fn parent(self, relation: TreeRelation) -> Option<Self> {
        self.as_node().parent(relation)?.element()
    }
    pub(crate) fn ancestors(self, relation: TreeRelation) -> impl Iterator<Item = Self> + use<'a> {
        std::iter::successors(self.parent(relation), move |element| {
            element.parent(relation)
        })
    }
    pub(crate) fn children(
        self,
        relation: TreeRelation,
    ) -> impl DoubleEndedIterator<Item = Self> + use<'a> {
        self.child_nodes(relation).filter_map(Node::element)
    }
    /// One-based element index and inclusive element count in the requested tree.
    pub(crate) fn sibling_position(self, relation: TreeRelation) -> (usize, usize) {
        const FIRST_INDEX: usize = 1;
        let key = (self.raw.id(), relation);
        if let Some(position) = self.tree.sibling_positions.borrow().get(&key) {
            return *position;
        }
        let mut positions = self.tree.sibling_positions.borrow_mut();
        if let Some(parent) = self.as_node().parent(relation) {
            let count = parent.children(relation).filter_map(Node::element).count();
            for (index, sibling) in parent
                .children(relation)
                .filter_map(Node::element)
                .enumerate()
            {
                positions.insert((sibling.raw.id(), relation), (index + FIRST_INDEX, count));
            }
        }
        *positions.entry(key).or_insert((FIRST_INDEX, FIRST_INDEX))
    }
    pub(crate) fn child_nodes(self, relation: TreeRelation) -> ChildNodes<'a> {
        self.as_node().children(relation)
    }
    pub(crate) fn text(self) -> impl Iterator<Item = &'a str> + use<'a> {
        self.text_in(TreeRelation::Selector)
    }
    pub(crate) fn text_in(self, relation: TreeRelation) -> impl Iterator<Item = &'a str> + use<'a> {
        self.as_node()
            .inclusive_descendants(relation)
            .filter_map(|node| node.value().as_text().map(|text| &**text))
    }
    #[cfg(test)]
    pub(crate) fn matches(self, selector: &Selector) -> bool {
        selector.matches(self, &mut SelectorCaches::default(), None)
    }
    pub(crate) fn matches_in_selector_scope(
        self,
        selector: &Selector,
        context: crate::dom::TreeContext,
        scope: crate::dom::SelectorScope,
        ancestors: Option<&selectors::bloom::BloomFilter>,
    ) -> bool {
        if let Some(crate::dom::ScopeRoot::Element(id)) = scope.root
            && self.tree.element(id).is_none()
        {
            return false;
        }
        if scope
            .origin
            .is_some_and(|id| self.tree.element(id).is_none())
        {
            return false;
        }
        selector.matches_context(
            self,
            &mut SelectorCaches::default(),
            scope,
            context,
            ancestors,
        )
    }
    #[cfg(test)]
    pub(crate) fn template_contents(self) -> Option<Node<'a>> {
        if self.namespace() != HTML_NAMESPACE || self.name() != TEMPLATE_ELEMENT {
            return None;
        }
        self.raw
            .children()
            .find(|node| node.value().is_fragment())
            .map(|raw| Node {
                tree: self.tree,
                raw,
            })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Node<'a> {
    tree: &'a DocumentTree,
    raw: NodeRef<'a, NodeData>,
}
impl PartialEq for Node<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.tree.identity == other.tree.identity && self.raw.id() == other.raw.id()
    }
}
impl Eq for Node<'_> {}
impl fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value().fmt(f)
    }
}
impl<'a> Node<'a> {
    pub(crate) fn value(self) -> &'a NodeData {
        self.raw.value()
    }
    pub(crate) fn element(self) -> Option<Element<'a>> {
        ElementRef::wrap(self.raw).map(|raw| Element {
            tree: self.tree,
            raw,
        })
    }
    pub(crate) fn parent(self, relation: TreeRelation) -> Option<Self> {
        if self.value().is_fragment() {
            return None;
        }
        let raw = if relation == TreeRelation::Selector || self.tree.shadow.roots.is_empty() {
            self.raw.parent()?
        } else if let Some(slot) = self.tree.shadow.assigned.get(&self.raw.id()) {
            self.tree.html.tree.get(*slot)?
        } else {
            let parent = self.raw.parent()?;
            if let Some(host) = self.tree.shadow.hosts.get(&parent.id()) {
                self.tree.html.tree.get(*host)?
            } else if self.tree.shadow.roots.contains_key(&parent.id())
                || self
                    .tree
                    .shadow
                    .slots
                    .get(&parent.id())
                    .is_some_and(|nodes| !nodes.is_empty())
            {
                return None;
            } else {
                parent
            }
        };
        Some(Self {
            tree: self.tree,
            raw,
        })
    }
    pub(crate) fn children(self, relation: TreeRelation) -> ChildNodes<'a> {
        let projected = (relation != TreeRelation::Selector)
            .then(|| self.tree.shadow.slots.get(&self.raw.id()))
            .flatten()
            .filter(|nodes| !nodes.is_empty());
        let inner = if let Some(assigned) = projected {
            ChildList::Assigned(assigned.iter())
        } else {
            let raw = if relation == TreeRelation::Selector {
                self.raw
            } else {
                self.tree
                    .shadow
                    .roots
                    .get(&self.raw.id())
                    .map_or(self.raw, |root| {
                        self.tree.html.tree.get(root.fragment).unwrap()
                    })
            };
            ChildList::Raw(raw.children())
        };
        ChildNodes {
            tree: self.tree,
            inner,
        }
    }
    /// Includes the starting node; template contents remain separate document fragments.
    pub(crate) fn inclusive_descendants(
        self,
        relation: TreeRelation,
    ) -> impl Iterator<Item = Self> + use<'a> {
        let projected = relation != TreeRelation::Selector && !self.tree.shadow.roots.is_empty();
        let traversal = (!projected)
            .then(|| self.raw.traverse())
            .into_iter()
            .flatten();
        let mut fragments = 0;
        let ordinary = traversal.filter_map(move |edge| match edge {
            Edge::Open(raw) if raw.value().is_fragment() && raw.id() != self.raw.id() => {
                fragments += 1;
                None
            }
            Edge::Close(raw) if raw.value().is_fragment() && raw.id() != self.raw.id() => {
                fragments -= 1;
                None
            }
            Edge::Open(raw) if fragments == 0 => Some(Self {
                tree: self.tree,
                raw,
            }),
            _ => None,
        });
        let mut pending = Vec::new();
        if projected {
            pending.push(self);
        }
        let flat = std::iter::from_fn(move || {
            let node = pending.pop()?;
            pending.extend(node.children(relation).rev());
            Some(node)
        });
        ordinary.chain(flat)
    }
}

/// Original or projected children, retaining double-ended traversal without allocation.
pub(crate) struct ChildNodes<'a> {
    tree: &'a DocumentTree,
    inner: ChildList<'a>,
}
enum ChildList<'a> {
    Raw(ego_tree::iter::Children<'a, NodeData>),
    Assigned(std::slice::Iter<'a, NodeId>),
}
impl<'a> Iterator for ChildNodes<'a> {
    type Item = Node<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let raw = match &mut self.inner {
            ChildList::Raw(nodes) => nodes.find(|node| !node.value().is_fragment())?,
            ChildList::Assigned(nodes) => self.tree.html.tree.get(*nodes.next()?)?,
        };
        Some(Node {
            tree: self.tree,
            raw,
        })
    }
}
impl DoubleEndedIterator for ChildNodes<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let raw = match &mut self.inner {
            ChildList::Raw(nodes) => nodes.rfind(|node| !node.value().is_fragment())?,
            ChildList::Assigned(nodes) => self.tree.html.tree.get(*nodes.next_back()?)?,
        };
        Some(Node {
            tree: self.tree,
            raw,
        })
    }
}
