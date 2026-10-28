use crate::{
    StyleError,
    selector::{self, NamespaceMap, RuleSelector, SelectorMode},
};
use cssparser::{ParseError, Parser};
use ferri_pdf_dom::{Element, ElementId, ScopeRoot, SelectorScope, TreeContext, TreeRelation};
use std::collections::HashMap;

pub(super) struct ScopePrelude {
    start: Option<Vec<RuleSelector>>,
    end: Vec<RuleSelector>,
    nesting: bool,
}
impl ScopePrelude {
    pub(crate) fn parse<'i>(
        input: &mut Parser<'i, '_>,
        namespaces: &NamespaceMap,
        mode: SelectorMode,
        forgiving: bool,
    ) -> Result<Self, ParseError<'i, StyleError>> {
        let start = if input.try_parse(Parser::expect_parenthesis_block).is_ok() {
            Some(input.parse_nested_block(|input| {
                selector::scope_boundary(input, namespaces, mode, forgiving)
            })?)
        } else {
            None
        };
        let end = if input.is_exhausted() {
            Vec::new()
        } else {
            input.expect_ident_matching("to")?;
            input.expect_parenthesis_block()?;
            input.parse_nested_block(|input| {
                selector::scope_boundary(input, namespaces, SelectorMode::Scope, forgiving)
            })?
        };
        input.expect_exhausted()?;
        Ok(Self {
            start,
            end,
            nesting: mode == SelectorMode::Nesting,
        })
    }

    pub(crate) fn work(&self, parent: &[RuleSelector]) -> usize {
        self.start
            .as_ref()
            .map_or(0, |start| {
                selector::selector_work(start, if self.nesting { parent } else { &[] })
            })
            .saturating_add(selector::selector_work(&self.end, &[]))
    }

    pub(crate) fn prepare(
        self,
        parent: &[RuleSelector],
        outer: Option<usize>,
        implicit_root: Option<ElementId>,
        tree_context: TreeContext,
    ) -> Result<Scope, StyleError> {
        let start = self
            .start
            .map(|start| {
                selector::resolve_selectors(if self.nesting { parent } else { &[] }, &start)
            })
            .transpose()?;
        Ok(Scope {
            start,
            end: self.end,
            parent: outer,
            implicit_root,
            tree_context,
        })
    }
}

#[derive(Clone)]
pub(super) struct Scope {
    start: Option<Vec<RuleSelector>>,
    end: Vec<RuleSelector>,
    parent: Option<usize>,
    implicit_root: Option<ElementId>,
    tree_context: TreeContext,
}

#[derive(Clone, Copy)]
pub(super) struct ScopeMatch {
    pub root: ScopeRoot,
    pub proximity: usize,
}

pub(super) struct ScopeCache<'a> {
    element: Element<'a>,
    ancestors: HashMap<(TreeContext, ElementId), Vec<Element<'a>>>,
    matched: HashMap<(usize, ElementId, usize), Vec<ScopeMatch>>,
}
impl<'a> ScopeCache<'a> {
    pub(crate) fn new(element: Element<'a>) -> Self {
        Self {
            element,
            ancestors: HashMap::new(),
            matched: HashMap::new(),
        }
    }

    pub(crate) fn roots(
        &mut self,
        index: Option<usize>,
        scopes: &[Scope],
        selector: &RuleSelector,
    ) -> impl Iterator<Item = (SelectorScope, Option<usize>)> + '_ {
        let subjects =
            index.map(|index| selector.scope_origins(self.element, scopes[index].tree_context));
        for (origin, hops) in subjects.clone().into_iter().flatten() {
            let index = index.unwrap();
            let mut pending = Vec::new();
            let mut current = Some(index);
            while let Some(index) = current {
                if self.matched.contains_key(&(index, origin.id(), hops)) {
                    break;
                }
                pending.push(index);
                current = scopes[index].parent;
            }
            for index in pending.into_iter().rev() {
                let scope = &scopes[index];
                let element = origin;
                let ancestors = self
                    .ancestors
                    .entry((scope.tree_context, origin.id()))
                    .or_insert_with(|| {
                        let host = match scope.tree_context {
                            TreeContext::Root(root) => element.scope_host(root),
                            TreeContext::Document | TreeContext::All => None,
                        };
                        if host.is_some_and(|host| host.id() == element.id()) {
                            return vec![element];
                        }
                        let visible = match scope.tree_context {
                            TreeContext::Document => element.is_in_document_tree(),
                            TreeContext::Root(root) => element.tree_scope() == root,
                            TreeContext::All => true,
                        };
                        if !visible {
                            return Vec::new();
                        }
                        std::iter::once(element)
                            .chain(element.ancestors(TreeRelation::Selector))
                            .chain(host)
                            .collect()
                    });
                let document = match scope.tree_context {
                    TreeContext::Root(root) => {
                        element.is_in_document_tree() && element.tree_scope() == root
                    }
                    TreeContext::Document | TreeContext::All => element.is_in_document_tree(),
                };
                let candidates = ancestors
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(distance, element)| {
                        (Some(element), ScopeRoot::Element(element.id()), distance)
                    })
                    .chain(document.then_some((None, ScopeRoot::Document, ancestors.len())));
                let roots = candidates
                    .filter_map(|(element, root, distance)| {
                        let proximity = distance + hops;
                        let starts = |outer| {
                            scope.start.as_ref().map_or_else(
                                || {
                                    root == scope
                                        .implicit_root
                                        .map_or(ScopeRoot::Document, ScopeRoot::Element)
                                },
                                |selectors| {
                                    element.is_some_and(|element| {
                                        selectors.iter().any(|selector| {
                                            selector
                                                .matches(
                                                    &element,
                                                    None,
                                                    SelectorScope {
                                                        root: outer,
                                                        origin: None,
                                                    },
                                                    scope.tree_context,
                                                )
                                                .is_some()
                                        })
                                    })
                                },
                            )
                        };
                        let starts = if let Some(parent) = scope.parent {
                            self.matched[&(parent, origin.id(), hops)]
                                .iter()
                                .any(|parent| {
                                    proximity <= parent.proximity && starts(Some(parent.root))
                                })
                        } else {
                            starts(None)
                        };
                        if !starts
                            || ancestors[..(distance + 1).min(ancestors.len())].iter().any(
                                |element| {
                                    scope.end.iter().any(|selector| {
                                        selector
                                            .matches(
                                                element,
                                                None,
                                                SelectorScope {
                                                    root: Some(root),
                                                    origin: None,
                                                },
                                                scope.tree_context,
                                            )
                                            .is_some()
                                    })
                                },
                            )
                        {
                            return None;
                        }
                        Some(ScopeMatch { root, proximity })
                    })
                    .collect();
                self.matched.insert((index, origin.id(), hops), roots);
            }
        }
        let unscoped = index.is_none().then_some((SelectorScope::default(), None));
        let matched = &self.matched;
        let scoped = subjects
            .into_iter()
            .flatten()
            .flat_map(move |(origin, hops)| {
                matched
                    .get(&(index.unwrap(), origin.id(), hops))
                    .into_iter()
                    .flatten()
                    .map(move |matched| {
                        (
                            SelectorScope {
                                root: Some(matched.root),
                                origin: Some(origin.id()),
                            },
                            Some(matched.proximity),
                        )
                    })
            });
        unscoped.into_iter().chain(scoped)
    }
}
