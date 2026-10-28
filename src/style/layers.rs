use crate::style::cascade::Origin;
use ferri_pdf_dom::TreeScopeId;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap},
};

pub(super) type LayerName = Vec<String>;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum LayerId {
    Named(usize),
    #[default]
    Unlayered,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum LayerPriority {
    Normal(LayerId),
    Important(Reverse<LayerId>),
}

impl LayerId {
    pub(crate) fn priority(self, important: bool) -> LayerPriority {
        if important {
            LayerPriority::Important(Reverse(self))
        } else {
            LayerPriority::Normal(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LayerNode(usize);

#[derive(Clone, Default)]
struct Node {
    children: Vec<LayerNode>,
    names: HashMap<String, LayerNode>,
}

#[derive(Clone, Default)]
pub(super) struct LayerTree {
    roots: BTreeMap<(Origin, Option<TreeScopeId>), LayerNode>,
    nodes: Vec<Node>,
    order: Vec<LayerId>,
}

impl LayerTree {
    pub(crate) fn root(&mut self, origin: Origin, scope: Option<TreeScopeId>) -> LayerNode {
        if let Some(root) = self.roots.get(&(origin, scope)) {
            return *root;
        }
        let root = self.allocate();
        self.roots.insert((origin, scope), root);
        root
    }

    fn allocate(&mut self) -> LayerNode {
        let node = LayerNode(self.nodes.len());
        self.nodes.push(Node::default());
        node
    }

    pub(crate) fn register(&mut self, mut parent: LayerNode, name: Option<&[String]>) -> LayerNode {
        let Some(name) = name else {
            let child = self.allocate();
            self.nodes[parent.0].children.push(child);
            return child;
        };
        for segment in name {
            parent = if let Some(child) = self.nodes[parent.0].names.get(segment) {
                *child
            } else {
                let child = self.allocate();
                self.nodes[parent.0].children.push(child);
                self.nodes[parent.0].names.insert(segment.clone(), child);
                child
            };
        }
        parent
    }

    /// Postorder puts a layer's direct rules after all of its nested layers.
    pub(crate) fn freeze(&mut self) {
        self.order = vec![LayerId::Unlayered; self.nodes.len()];
        let mut rank = 0;
        for &root in self.roots.values() {
            let mut stack = vec![(root, 0)];
            while let Some((node, next)) = stack.last_mut() {
                if let Some(&child) = self.nodes[node.0].children.get(*next) {
                    *next += 1;
                    stack.push((child, 0));
                } else {
                    let (node, _) = stack.pop().expect("the traversal has a current node");
                    if node != root {
                        self.order[node.0] = LayerId::Named(rank);
                        rank += 1;
                    }
                }
            }
        }
    }

    pub(crate) fn id(&self, node: LayerNode) -> LayerId {
        self.order[node.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::cascade::Origin;

    #[test]
    fn layer_tree_orders_children_before_parent_direct_rules() {
        let mut layers = LayerTree::default();
        let root = layers.root(Origin::Author, None);
        let outer = layers.register(root, Some(&["outer".into()]));
        let next = layers.register(root, Some(&["next".into()]));
        let child = layers.register(outer, Some(&["child".into()]));
        layers.freeze();
        assert!(layers.id(child) < layers.id(outer));
        assert!(layers.id(outer) < layers.id(next));
        assert!(layers.id(next) < layers.id(root));
    }

    #[test]
    fn layer_anonymous_identity_and_reopened_names_share_only_their_parent() {
        let mut layers = LayerTree::default();
        let root = layers.root(Origin::Author, None);
        let first = layers.register(root, None);
        let second = layers.register(root, None);
        assert_ne!(first, second);
        let child = layers.register(first, Some(&["child".into()]));
        assert_eq!(child, layers.register(first, Some(&["child".into()])));
        assert_ne!(child, layers.register(second, Some(&["child".into()])));
    }
}
