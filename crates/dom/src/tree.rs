use crate::{HTML_NAMESPACE, NodeData, QuirksMode, Selector};
use ego_tree::{NodeId, NodeRef, iter::Edge};
use scraper::{ElementRef, Html};
use selectors::matching::SelectorCaches;
use std::{
    cell::RefCell,
    collections::HashMap,
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

mod xml;
pub use xml::XmlTree;

pub const DOCUMENT_CONTEXT_ORDER: usize = 0;
const FIRST_DOCUMENT_ID: u64 = 1;
static NEXT_DOCUMENT_ID: AtomicU64 = AtomicU64::new(FIRST_DOCUMENT_ID);
const TEMPLATE_ELEMENT: &str = "template";
const HTML_ELEMENT: &str = "html";
const BODY_ELEMENT: &str = "body";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TreeRelation {
    Selector,
    Inheritance,
    Formatting,
}

/// A handle is qualified by its owning document, even after the tree is moved.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ElementId {
    document: u64,
    node: NodeId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TreeScopeId {
    document: u64,
    root: NodeId,
}

#[derive(Debug)]
pub struct DocumentTree {
    html: Html,
    pub(crate) is_html_document: bool,
    identity: u64,
    pub(crate) document_url: Option<url::Url>,
    pub(crate) base_url: Option<url::Url>,
    pub(crate) default_language: String,
    pub(crate) shadow: crate::shadow::ShadowState,
    pub(crate) initial_states: HashMap<NodeId, crate::InitialState>,
    sibling_positions: RefCell<HashMap<(NodeId, TreeRelation), (usize, usize)>>,
}

impl DocumentTree {
    pub(crate) fn from_parsed(html: Html, shadow: crate::shadow::ShadowState) -> Self {
        Self::from_parsed_with_mode(html, shadow, true)
    }
    fn from_parsed_with_mode(
        html: Html,
        shadow: crate::shadow::ShadowState,
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
            sibling_positions: RefCell::default(),
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
                    .split(crate::state::is_html_whitespace)
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
        crate::state::prepare_initial_states(
            nodes.into_iter().filter_map(|id| {
                ElementRef::wrap(self.html.tree.get(id)?).map(|raw| Element { tree: self, raw })
            }),
            &mut states,
            &self.default_language,
        );
        self.initial_states = states;
    }
    pub fn set_document_urls(&mut self, document: Option<url::Url>, base: url::Url) {
        self.document_url = document;
        self.base_url = Some(base);
    }
    pub fn quirks_mode(&self) -> QuirksMode {
        self.html.quirks_mode
    }
    pub fn root_element(&self) -> Element<'_> {
        Element {
            tree: self,
            raw: self.html.root_element(),
        }
    }
    pub fn body_element(&self) -> Option<Element<'_>> {
        let root = self.root_element();
        (root.namespace() == HTML_NAMESPACE && root.name() == HTML_ELEMENT)
            .then(|| {
                root.children(TreeRelation::Selector).find(|child| {
                    child.namespace() == HTML_NAMESPACE && child.name() == BODY_ELEMENT
                })
            })
            .flatten()
    }
    pub fn element(&self, id: ElementId) -> Option<Element<'_>> {
        if id.document != self.identity {
            return None;
        }
        ElementRef::wrap(self.html.tree.get(id.node)?).map(|raw| Element { tree: self, raw })
    }
    pub fn elements(&self) -> impl Iterator<Item = Element<'_>> {
        Node {
            tree: self,
            raw: self.html.tree.root(),
        }
        .inclusive_descendants(TreeRelation::Selector)
        .filter_map(Node::element)
    }
    pub fn shadow_including_elements(&self) -> impl Iterator<Item = Element<'_>> {
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
    pub fn select<'a, 's>(
        &'a self,
        selector: &'s Selector,
    ) -> impl Iterator<Item = Element<'a>> + use<'a, 's> {
        let mut caches = SelectorCaches::default();
        self.elements()
            .filter(move |element| selector.matches(*element, &mut caches, None))
    }
}

#[derive(Clone, Copy)]
pub struct Element<'a> {
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
    pub fn quirks_mode(self) -> QuirksMode {
        self.tree.quirks_mode()
    }
    pub fn id(self) -> ElementId {
        ElementId {
            document: self.tree.identity,
            node: self.raw.id(),
        }
    }
    pub fn tree_scope(self) -> TreeScopeId {
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
    pub fn shadow_root(self) -> Option<Node<'a>> {
        let root = self.tree.shadow.roots.get(&self.raw.id())?;
        Some(Node {
            tree: self.tree,
            raw: self.tree.html.tree.get(root.fragment).unwrap(),
        })
    }
    pub fn is_in_document_tree(self) -> bool {
        self.tree_scope().root == self.tree.html.tree.root().id()
    }
    pub fn encapsulation_order(self) -> usize {
        if self.is_in_document_tree() {
            DOCUMENT_CONTEXT_ORDER
        } else {
            self.tree.shadow.context_order[&self.tree_scope().root]
        }
    }
    pub fn scope_host(self, scope: TreeScopeId) -> Option<Self> {
        if scope.document != self.tree.identity {
            return None;
        }
        let host = self.tree.shadow.hosts.get(&scope.root)?;
        ElementRef::wrap(self.tree.html.tree.get(*host)?).map(|raw| Self {
            tree: self.tree,
            raw,
        })
    }
    pub fn shadow_host(self) -> Option<Self> {
        self.scope_host(self.tree_scope())
    }
    pub fn assigned_slot(self) -> Option<Self> {
        let slot = self.tree.shadow.assigned.get(&self.raw.id())?;
        ElementRef::wrap(self.tree.html.tree.get(*slot)?).map(|raw| Self {
            tree: self.tree,
            raw,
        })
    }
    pub(crate) fn alias_slot(self) -> Option<Self> {
        let node = self.tree.shadow.alias_slot(self.raw.id())?;
        self.tree.element(ElementId {
            document: self.tree.identity,
            node,
        })
    }
    pub fn shadow_including_parent(self) -> Option<Self> {
        self.parent(TreeRelation::Selector)
            .or_else(|| self.shadow_host())
    }
    pub fn value(self) -> &'a scraper::node::Element {
        self.raw.value()
    }
    pub fn name(self) -> &'a str {
        self.value().name()
    }
    pub fn namespace(self) -> &'a str {
        self.value().name.ns.as_ref()
    }
    pub fn attr(self, name: &str) -> Option<&'a str> {
        self.raw.attr(name)
    }
    pub(crate) fn is_html_element_in_html_document(self) -> bool {
        self.tree.is_html_document && self.namespace() == HTML_NAMESPACE
    }
    /// CSS attribute lookup by namespace and local name, with document-language sensitivity.
    pub fn attr_in_namespace(self, namespace: Option<&str>, name: &str) -> Option<&'a str> {
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
    pub fn as_node(self) -> Node<'a> {
        Node {
            tree: self.tree,
            raw: *self.raw,
        }
    }
    pub fn parent(self, relation: TreeRelation) -> Option<Self> {
        self.as_node().parent(relation)?.element()
    }
    pub fn ancestors(self, relation: TreeRelation) -> impl Iterator<Item = Self> + use<'a> {
        std::iter::successors(self.parent(relation), move |element| {
            element.parent(relation)
        })
    }
    pub fn children(
        self,
        relation: TreeRelation,
    ) -> impl DoubleEndedIterator<Item = Self> + use<'a> {
        self.child_nodes(relation).filter_map(Node::element)
    }
    /// One-based element index and inclusive element count in the requested tree.
    pub fn sibling_position(self, relation: TreeRelation) -> (usize, usize) {
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
    pub fn child_nodes(self, relation: TreeRelation) -> ChildNodes<'a> {
        self.as_node().children(relation)
    }
    pub fn text(self) -> impl Iterator<Item = &'a str> + use<'a> {
        self.text_in(TreeRelation::Selector)
    }
    pub fn text_in(self, relation: TreeRelation) -> impl Iterator<Item = &'a str> + use<'a> {
        self.as_node()
            .inclusive_descendants(relation)
            .filter_map(|node| node.value().as_text().map(|text| &**text))
    }
    pub fn matches(self, selector: &Selector) -> bool {
        selector.matches(self, &mut SelectorCaches::default(), None)
    }
    pub fn matches_in_scope(self, selector: &Selector, scope: crate::ScopeRoot) -> bool {
        self.matches_in_tree(
            selector,
            crate::TreeContext::Root(self.tree_scope()),
            Some(scope),
        )
    }
    pub fn matches_in_tree(
        self,
        selector: &Selector,
        context: crate::TreeContext,
        scope: Option<crate::ScopeRoot>,
    ) -> bool {
        self.matches_in_selector_scope(
            selector,
            context,
            crate::SelectorScope {
                root: scope,
                origin: None,
            },
        )
    }
    pub fn matches_in_selector_scope(
        self,
        selector: &Selector,
        context: crate::TreeContext,
        scope: crate::SelectorScope,
    ) -> bool {
        if let Some(crate::ScopeRoot::Element(id)) = scope.root
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
        selector.matches_context(self, &mut SelectorCaches::default(), scope, context)
    }
    pub fn template_contents(self) -> Option<Node<'a>> {
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
pub struct Node<'a> {
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
    pub fn value(self) -> &'a NodeData {
        self.raw.value()
    }
    pub fn element(self) -> Option<Element<'a>> {
        ElementRef::wrap(self.raw).map(|raw| Element {
            tree: self.tree,
            raw,
        })
    }
    pub fn parent(self, relation: TreeRelation) -> Option<Self> {
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
    pub fn children(self, relation: TreeRelation) -> ChildNodes<'a> {
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
    pub fn inclusive_descendants(
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
pub struct ChildNodes<'a> {
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
