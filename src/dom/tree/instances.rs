//! SVG 2 §5.5 use-element shadow trees: deep clones of referenced elements, hosted by `use`.
use super::{DocumentTree, ElementId, TreeScopeId};
use crate::dom::{DomError, NodeData, SVG_NAMESPACE, XLINK_NAMESPACE, shadow::ShadowRoot};
use ego_tree::NodeId;
use std::collections::HashMap;

/// Cloned nodes across all of a document's use-element shadow trees, which bounds the
/// exponential growth of nested references without a cycle.
const MAX_INSTANCE_NODES: usize = 1_000_000;
const INSTANCE_NODES: &str = "SVG use-element instance nodes";
const USE_ELEMENT: &str = "use";
const HREF_ATTRIBUTE: &str = "href";
const ID_ATTRIBUTE: &str = "id";
const FRAGMENT: char = '#';

/// A shadow tree's scope and the clone of each original node in it.
type Instance = (TreeScopeId, HashMap<NodeId, NodeId>);

/// A `use` element awaiting its shadow tree: the node hosting it, the original element it was
/// cloned from (itself, outside shadow trees), and the originals of the `use` elements whose
/// instances contain it.
struct Pending {
    host: NodeId,
    original: NodeId,
    enclosing: Vec<NodeId>,
}

impl DocumentTree {
    /// Clones the subtree of `original` into a new shadow root of `host`, returning the clone
    /// of each original node. `source` is the tree scope whose style sheets style the clone;
    /// `None` leaves its own scope's.
    pub(super) fn attach_instance(
        &mut self,
        original: NodeId,
        host: NodeId,
        source: Option<NodeId>,
        node_limit: usize,
    ) -> Result<Option<Instance>, DomError> {
        if self.shadow.roots.contains_key(&host) {
            return Ok(None);
        }
        let originals: Vec<NodeId> = self
            .html
            .tree
            .get(original)
            .expect("original node")
            .descendants()
            .map(|node| node.id())
            .collect();
        if originals.len() > node_limit.saturating_sub(self.instance_nodes) {
            return Err(DomError::TreeLimit {
                resource: INSTANCE_NODES,
                limit: node_limit,
            });
        }
        let depth = self
            .element(ElementId {
                document: self.identity,
                node: host,
            })
            .and_then(super::Element::shadow_host)
            .map_or(1, |parent| self.shadow.roots[&parent.raw.id()].depth + 1);
        self.sibling_positions.get_mut().clear();
        let fragment = self.html.tree.orphan(NodeData::Fragment).id();
        let mut clones = HashMap::with_capacity(originals.len());
        for &node in &originals {
            let parent = if node == original {
                fragment
            } else {
                clones[&self.html.tree.get(node).unwrap().parent().unwrap().id()]
            };
            let data = self.html.tree.get(node).unwrap().value().clone();
            let cloned = self.html.tree.get_mut(parent).unwrap().append(data).id();
            self.shadow.scopes.insert(cloned, fragment);
            clones.insert(node, cloned);
        }
        self.shadow
            .roots
            .insert(host, ShadowRoot { fragment, depth });
        self.shadow.hosts.insert(fragment, host);
        if let Some(source) = source {
            self.shadow.instance_sources.insert(fragment, source);
        }
        self.shadow.order_contexts([fragment]);
        self.cache_initial_states(clones.values().copied());
        self.instance_nodes += originals.len();
        Ok(Some((
            TreeScopeId {
                document: self.identity,
                root: fragment,
            },
            clones,
        )))
    }

    /// Gives every SVG `use` element referencing an element of its own node tree a shadow tree
    /// holding that element's clone (§5.5.1), nested `use` elements in clones included. A
    /// reference whose instance would contain the referencing element is circular, and gets
    /// none (§5.5.1); neither does a reference to another document.
    pub(crate) fn attach_use_instances(&mut self) {
        let mut identified: HashMap<(NodeId, &str), NodeId> = HashMap::new();
        let mut pending = Vec::new();
        for element in self.shadow_including_elements() {
            if let Some(id) = element.attr(ID_ATTRIBUTE) {
                identified
                    .entry((element.tree_scope().root, id))
                    .or_insert(element.raw.id());
            }
            if element.namespace() == SVG_NAMESPACE && element.name() == USE_ELEMENT {
                pending.push(Pending {
                    host: element.raw.id(),
                    original: element.raw.id(),
                    enclosing: Vec::new(),
                });
            }
        }
        let references: HashMap<NodeId, (NodeId, NodeId)> = pending
            .iter()
            .filter_map(|instance| {
                let element = self.element(ElementId {
                    document: self.identity,
                    node: instance.original,
                })?;
                let scope = element.tree_scope().root;
                // §16.1.6: `href` without a namespace wins over the deprecated `xlink:href`.
                let reference = element
                    .attr_in_namespace(None, HREF_ATTRIBUTE)
                    .or_else(|| element.attr_in_namespace(Some(XLINK_NAMESPACE), HREF_ATTRIBUTE))?
                    .strip_prefix(FRAGMENT)?;
                let target = *identified.get(&(scope, reference))?;
                Some((instance.original, (target, scope)))
            })
            .collect();
        pending.reverse();
        while let Some(instance) = pending.pop() {
            let Some(&(target, scope)) = references.get(&instance.original) else {
                continue;
            };
            let circular = std::iter::once(&instance.original)
                .chain(&instance.enclosing)
                .any(|&original| {
                    self.html
                        .tree
                        .get(original)
                        .unwrap()
                        .ancestors()
                        .any(|ancestor| ancestor.id() == target)
                        || original == target
                });
            if circular {
                continue;
            }
            // An instance past the node limit is not created: the `use` element renders
            // nothing, as for a reference to no element (SVG 2 §5.5.1).
            let clones = match self.attach_instance(
                target,
                instance.host,
                Some(scope),
                MAX_INSTANCE_NODES,
            ) {
                Ok(Some((_, clones))) => clones,
                Ok(None) => continue,
                Err(limit) => {
                    if !self.limits.contains(&limit) {
                        self.limits.push(limit);
                    }
                    continue;
                }
            };
            let mut enclosing = instance.enclosing;
            enclosing.push(instance.original);
            let mut nested: Vec<_> = clones
                .into_iter()
                .filter(|(original, _)| references.contains_key(original))
                .collect();
            // Tree order keeps the shadow trees' creation, and so their nodes, deterministic.
            nested.sort_unstable();
            pending.extend(nested.into_iter().rev().map(|(original, host)| Pending {
                host,
                original,
                enclosing: enclosing.clone(),
            }));
        }
    }
}
