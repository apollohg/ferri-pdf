use super::{DocumentTree, Element, ElementId, TreeScopeId};
use crate::{DomError, NodeData, Selector, SelectorScope, TreeContext, shadow::ShadowRoot};
use html5ever::{Attribute, QualName};
use roxmltree::{Document, Node, NodeType};
use scraper::{Html, node};
use selectors::matching::SelectorCaches;
use std::{cell::RefCell, collections::HashMap};

const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";
const XMLNS_PREFIX: &str = "xmlns";
const XML_PREFIX: &str = "xml";
const XML_INSTANCE_NODES: &str = "XML instance nodes";

/// Original XML nodes viewed through the shared static selector engine.
pub struct XmlTree<'doc, 'input> {
    source: &'doc Document<'input>,
    tree: DocumentTree,
    nodes: Vec<Option<ego_tree::NodeId>>,
    instances: HashMap<TreeScopeId, HashMap<roxmltree::NodeId, ego_tree::NodeId>>,
    instance_nodes: usize,
    caches: RefCell<SelectorCaches>,
}

impl<'doc, 'input> XmlTree<'doc, 'input> {
    pub fn new(source: &'doc Document<'input>) -> Self {
        let mut html = Html::new_document();
        let mut nodes = vec![Some(html.tree.root().id())];
        for source in source.descendants().skip(1) {
            let data = match source.node_type() {
                NodeType::Root => continue,
                NodeType::Element => {
                    let attributes = source
                        .attributes()
                        .map(|attribute| Attribute {
                            name: QualName::new(
                                None,
                                attribute.namespace().unwrap_or_default().into(),
                                attribute.name().into(),
                            ),
                            value: attribute.value().into(),
                        })
                        .chain(
                            namespace_declarations(source).map(|(prefix, uri)| Attribute {
                                name: QualName::new(
                                    prefix.map(|_| XMLNS_PREFIX.into()),
                                    XMLNS_NAMESPACE.into(),
                                    prefix.unwrap_or(XMLNS_PREFIX).into(),
                                ),
                                value: uri.into(),
                            }),
                        )
                        .collect();
                    let name = source.tag_name();
                    NodeData::Element(node::Element::new(
                        QualName::new(
                            None,
                            name.namespace().unwrap_or_default().into(),
                            name.name().into(),
                        ),
                        attributes,
                    ))
                }
                NodeType::Text => NodeData::Text(node::Text {
                    text: source.text().unwrap_or_default().into(),
                }),
                NodeType::Comment => NodeData::Comment(node::Comment {
                    comment: source.text().unwrap_or_default().into(),
                }),
                NodeType::PI => {
                    let pi = source.pi().expect("processing instruction");
                    NodeData::ProcessingInstruction(node::ProcessingInstruction {
                        target: pi.target.into(),
                        data: pi.value.unwrap_or_default().into(),
                    })
                }
            };
            let parent = nodes[source
                .parent()
                .expect("XML child has a parent")
                .id()
                .get_usize()]
            .expect("preorder visits the parent first");
            let id = html.tree.get_mut(parent).unwrap().append(data).id();
            let index = source.id().get_usize();
            nodes.resize(index + 1, None);
            nodes[index] = Some(id);
        }
        Self {
            source,
            tree: DocumentTree::from_parsed_with_mode(html, Default::default(), false),
            nodes,
            instances: HashMap::new(),
            instance_nodes: 0,
            caches: RefCell::new(SelectorCaches::default()),
        }
    }

    pub fn element(&self, source: Node<'_, '_>) -> Option<Element<'_>> {
        self.element_in(source, None)
    }

    pub fn element_in(
        &self,
        source: Node<'_, '_>,
        instance: Option<TreeScopeId>,
    ) -> Option<Element<'_>> {
        if !std::ptr::eq(self.source, source.document()) {
            return None;
        }
        self.tree.element(ElementId {
            document: self.tree.identity,
            node: match instance {
                Some(scope) => *self.instances.get(&scope)?.get(&source.id())?,
                None => self.nodes.get(source.id().get_usize()).copied().flatten()?,
            },
        })
    }

    pub fn matches(&self, source: Node<'_, '_>, selector: &Selector) -> bool {
        self.element(source)
            .is_some_and(|element| self.matches_element(element.id(), selector, None))
    }

    pub fn matches_element(
        &self,
        element: ElementId,
        selector: &Selector,
        context: Option<TreeScopeId>,
    ) -> bool {
        self.tree.element(element).is_some_and(|element| {
            selector.matches_context(
                element,
                &mut self.caches.borrow_mut(),
                SelectorScope::default(),
                TreeContext::Root(context.unwrap_or_else(|| element.tree_scope())),
            )
        })
    }

    /// Clones one referenced subtree into a distinct shadow context at its actual host.
    pub fn create_instance(
        &mut self,
        source: Node<'_, '_>,
        host: ElementId,
        node_limit: usize,
    ) -> Result<Option<TreeScopeId>, DomError> {
        if !std::ptr::eq(self.source, source.document()) || !source.is_element() {
            return Ok(None);
        }
        let Some(host_element) = self.tree.element(host) else {
            return Ok(None);
        };
        if host_element.shadow_root().is_some() {
            return Ok(None);
        }
        let depth = host_element.shadow_host().map_or(1, |parent| {
            self.tree.shadow.roots[&parent.raw.id()].depth + 1
        });
        let available = node_limit.saturating_sub(self.instance_nodes);
        let count = source
            .descendants()
            .take(available.saturating_add(1))
            .count();
        if count > available {
            return Err(DomError::TreeLimit {
                resource: XML_INSTANCE_NODES,
                limit: node_limit,
            });
        }
        // Ego-tree can relocate node storage; selector caches contain opaque pointers.
        *self.caches.get_mut() = SelectorCaches::default();
        self.tree.sibling_positions.get_mut().clear();
        let fragment = self.tree.html.tree.orphan(NodeData::Fragment).id();
        let mut nodes = HashMap::with_capacity(count);
        for node in source.descendants() {
            let parent = if node == source {
                fragment
            } else {
                nodes[&node.parent().expect("XML child has a parent").id()]
            };
            let original = self.nodes[node.id().get_usize()].expect("original XML node");
            let data = self.tree.html.tree.get(original).unwrap().value().clone();
            let cloned = self
                .tree
                .html
                .tree
                .get_mut(parent)
                .unwrap()
                .append(data)
                .id();
            self.tree.shadow.scopes.insert(cloned, fragment);
            nodes.insert(node.id(), cloned);
        }
        self.tree
            .shadow
            .roots
            .insert(host.node, ShadowRoot { fragment, depth });
        self.tree.shadow.hosts.insert(fragment, host.node);
        self.tree.shadow.order_contexts([fragment]);
        let scope = TreeScopeId {
            document: self.tree.identity,
            root: fragment,
        };
        self.tree.cache_initial_states(nodes.values().copied());
        self.instances.insert(scope, nodes);
        self.instance_nodes += count;
        Ok(Some(scope))
    }
}

/// Namespace declarations written on an element's start tag, in source order.
/// roxmltree only exposes in-scope bindings, so redundant redeclarations are read from the tag.
fn namespace_declarations<'a>(
    element: Node<'a, '_>,
) -> impl Iterator<Item = (Option<&'a str>, &'a str)> {
    let tag = &element.document().input_text()[element.range().start..];
    start_tag_attribute_names(tag).filter_map(move |name| {
        let prefix = match name.strip_prefix(XMLNS_PREFIX)? {
            "" => None,
            rest => Some(rest.strip_prefix(':')?),
        };
        // The parser rejects any other `xml` binding and omits it from in-scope namespaces.
        let uri = if prefix == Some(XML_PREFIX) {
            crate::XML_NAMESPACE
        } else {
            element.lookup_namespace_uri(prefix).unwrap_or_default()
        };
        Some((prefix, uri))
    })
}

fn start_tag_attribute_names(tag: &str) -> impl Iterator<Item = &str> {
    let bytes = tag.as_bytes();
    let ends_name = |byte: &u8| byte.is_ascii_whitespace() || matches!(byte, b'=' | b'/' | b'>');
    let mut index = bytes.iter().position(ends_name).unwrap_or(bytes.len());
    std::iter::from_fn(move || {
        while bytes.get(index)?.is_ascii_whitespace() {
            index += 1;
        }
        let start = index;
        index += bytes[index..].iter().position(ends_name)?;
        if index == start {
            return None;
        }
        let name = &tag[start..index];
        index += bytes[index..]
            .iter()
            .position(|byte| matches!(byte, b'\'' | b'"'))?;
        let quote = bytes[index];
        index += bytes[index + 1..].iter().position(|&byte| byte == quote)? + 2;
        Some(name)
    })
}
