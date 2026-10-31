use crate::dom::{DocumentTree, ElementId, HTML_NAMESPACE, QuirksMode, TreeScopeId};
use crate::resources::{
    CSS_MEDIA_TYPE, ResolvedResource, ResourceBase, ResourceError, ResourceResolver, Url,
};
use crate::style::{
    PrintSettings, SourceLocation, StyleDiagnostic, StyleDiagnosticKind, StyleError,
    cascade::Origin,
    conditions::{
        self, Applicability, Definitions, NameKind, NamedDefinition, RuleCondition, RuleQuery,
    },
    layers::{LayerName, LayerNode, LayerTree},
    media::MediaQuery,
    parser::{ContentRule, ImportRule, Rule, rules},
    print::{Presentation, PrintEnvironment},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

pub(super) const MAX_STYLESHEET_BYTES: usize = 64 * 1024 * 1024;
const IGNORED_STYLESHEET: &str = "the stylesheet is ignored";
const IGNORED_RULE: &str = "the rule is ignored";
const TRUNCATED_STYLESHEET: &str = "the rest of the stylesheet is ignored";
pub(super) const MAX_IMPORT_DEPTH: usize = 64;
pub(super) const MAX_STYLESHEET_RULES: usize = 1_000_000;
const MAX_STYLESHEET_WORK: usize = MAX_STYLESHEET_RULES;
const BYTES: &str = "stylesheet bytes";
const RULES: &str = "stylesheet rules";
const DEPTH: &str = "import depth";
const NESTING_WORK: &str = "stylesheet nesting work";
const VISUAL_HEBREW_ENCODING: &str = "ISO-8859-8";
const UA_SOURCE_ORDER: usize = 0;

pub(crate) struct DecodedStylesheet {
    pub text: String,
    pub encoding: String,
    pub had_errors: bool,
}
pub(crate) type CssDecoder = fn(&[u8], Option<&str>, Option<&str>) -> DecodedStylesheet;

pub(crate) enum StylesheetInput {
    Inline {
        text: String,
        base: ResourceBase,
        media: String,
        owner: Option<ElementId>,
        fallback_encoding: String,
    },
    External {
        reference: String,
        base: ResourceBase,
        media: String,
        owner: Option<ElementId>,
        fallback_encoding: String,
    },
}
#[derive(Default)]
pub(crate) struct StylesheetSources {
    pub base: ResourceBase,
    pub document_url: Option<Url>,
    /// Canonical document encoding name; empty retains Unicode rendering defaults.
    pub document_encoding: String,
    pub document: Vec<StylesheetInput>,
    pub author: Vec<StylesheetInput>,
    pub user: Vec<StylesheetInput>,
}

pub(crate) enum SourceDiagnostic {
    Decoding {
        base: ResourceBase,
        reference: String,
        encoding: String,
    },
    Unavailable {
        base: ResourceBase,
        reference: String,
        error: ResourceError,
    },
}

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub bytes: usize,
    pub depth: usize,
    pub rules: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes: MAX_STYLESHEET_BYTES,
            depth: MAX_IMPORT_DEPTH,
            rules: MAX_STYLESHEET_RULES,
        }
    }
}
pub(super) struct Budget {
    limits: Limits,
    bytes: usize,
    rules: usize,
    nesting: usize,
    work_limit: usize,
}
impl Budget {
    fn new(limits: Limits) -> Self {
        Self {
            limits,
            bytes: 0,
            rules: 0,
            nesting: 0,
            work_limit: MAX_STYLESHEET_WORK,
        }
    }
    pub(super) fn unlimited() -> Self {
        let mut budget = Self::new(Limits {
            bytes: usize::MAX,
            depth: usize::MAX,
            rules: usize::MAX,
        });
        budget.work_limit = usize::MAX;
        budget
    }
    fn bytes(&mut self, bytes: usize, location: SourceLocation) -> Result<(), StyleError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|&bytes| bytes <= self.limits.bytes)
            .ok_or_else(|| StyleError::StylesheetLimit {
                resource: BYTES,
                limit: self.limits.bytes,
                location: Box::new(location),
            })?;
        Ok(())
    }
    pub(crate) fn rule(
        &mut self,
        location: impl FnOnce() -> SourceLocation,
    ) -> Result<(), StyleError> {
        if self.rules == self.limits.rules {
            return Err(StyleError::StylesheetLimit {
                resource: RULES,
                limit: self.limits.rules,
                location: Box::new(location()),
            });
        }
        self.rules += 1;
        Ok(())
    }
    fn nesting(
        &mut self,
        work: usize,
        location: impl FnOnce() -> SourceLocation,
    ) -> Result<(), StyleError> {
        self.nesting = self
            .nesting
            .checked_add(work)
            .filter(|&work| work <= self.work_limit)
            .ok_or_else(|| StyleError::StylesheetLimit {
                resource: NESTING_WORK,
                limit: self.work_limit,
                location: Box::new(location()),
            })?;
        Ok(())
    }
}

pub(super) enum PendingKind {
    Content(ContentRule),
    LayerStatement(Vec<LayerName>),
    LayerBlock(Option<LayerName>, usize),
}
pub(super) struct PendingRule {
    pub kind: PendingKind,
    pub base: ResourceBase,
    pub origin: Origin,
    pub scope: usize,
    pub condition: Option<usize>,
    pub style_scope: Option<usize>,
    /// The innermost enclosing `@container` rule.
    pub container: Option<usize>,
    pub tree_scope: Option<TreeScopeId>,
    pub encapsulation_order: usize,
}
pub(super) struct PreparedStylesheets {
    pub all: Vec<PendingRule>,
    pub layers: LayerTree,
    pub scopes: Vec<Option<LayerNode>>,
    pub style_scopes: Vec<crate::style::scope::Scope>,
    pub container_rules: Vec<crate::style::container_queries::ContainerRule>,
    pub conditions: Vec<RuleCondition>,
    pub environment: PrintEnvironment,
    pub diagnostics: RefCell<Vec<StyleDiagnostic>>,
    pub source_diagnostics: Vec<SourceDiagnostic>,
    pub document_base: ResourceBase,
}

#[derive(Clone)]
struct Context {
    base: ResourceBase,
    encoding: String,
    origin: Origin,
    scope: usize,
    condition: Option<usize>,
    order: Vec<usize>,
    depth: usize,
    path: Vec<Url>,
    implicit_root: Option<ElementId>,
    tree_scope: Option<TreeScopeId>,
    encapsulation_order: usize,
}
struct SourceSet {
    condition: usize,
    title: String,
    alternate: bool,
    unavailable: bool,
}
struct Fetch {
    slot: Option<usize>,
    reference: String,
    context: Context,
    location: SourceLocation,
}
struct Loader<'a> {
    tree: &'a DocumentTree,
    document_url: Option<Url>,
    resolver: &'a dyn ResourceResolver,
    decode: CssDecoder,
    prepared: PreparedStylesheets,
    roots: BTreeMap<(Origin, Option<TreeScopeId>), usize>,
    sets: Vec<SourceSet>,
    forced_set: Option<String>,
    selection_dirty: bool,
    all: BTreeMap<Vec<usize>, PendingRule>,
    definitions: Definitions,
    dirty: bool,
    applicable: Vec<Applicability>,
    fetches: BTreeMap<Vec<usize>, Fetch>,
    cache: HashMap<(ResourceBase, String), Arc<ResolvedResource>>,
    budget: Budget,
    trusted_budget: Budget,
}

impl<'a> Loader<'a> {
    fn new(
        tree: &'a DocumentTree,
        sources: &StylesheetSources,
        resolver: &'a dyn ResourceResolver,
        settings: &PrintSettings,
        presentation: Presentation,
        decode: CssDecoder,
        limits: Limits,
    ) -> Result<Self, StyleError> {
        let mut environment = PrintEnvironment::new(settings, presentation)?;
        environment.page_color_schemes = crate::style::color_scheme::page_color_schemes(tree);
        Ok(Self {
            tree,
            document_url: sources
                .document_url
                .clone()
                .or_else(|| sources.base.resolve("").ok()),
            resolver,
            decode,
            prepared: PreparedStylesheets {
                all: Vec::new(),
                layers: LayerTree::default(),
                scopes: Vec::new(),
                style_scopes: Vec::new(),
                container_rules: Vec::new(),
                conditions: Vec::new(),
                environment,
                diagnostics: RefCell::new(Vec::new()),
                source_diagnostics: Vec::new(),
                document_base: sources.base.clone(),
            },
            roots: BTreeMap::new(),
            sets: Vec::new(),
            forced_set: tree
                .elements()
                .filter(|element| {
                    element.namespace() == HTML_NAMESPACE
                        && element.name() == "meta"
                        && element
                            .attr("http-equiv")
                            .is_some_and(|value| value.eq_ignore_ascii_case("default-style"))
                })
                .filter_map(|element| element.attr("content").filter(|value| !value.is_empty()))
                .last()
                .map(str::to_owned),
            selection_dirty: true,
            all: BTreeMap::new(),
            definitions: Definitions::new(),
            dirty: true,
            applicable: Vec::new(),
            fetches: BTreeMap::new(),
            cache: HashMap::new(),
            budget: Budget::new(limits),
            trusted_budget: Budget::unlimited(),
        })
    }
    fn condition(
        &mut self,
        parent: Option<usize>,
        previous: Option<usize>,
        mut query: RuleQuery,
    ) -> usize {
        query.bind(&self.prepared.environment);
        let index = self.prepared.conditions.len();
        self.prepared.conditions.push(RuleCondition {
            parent,
            previous,
            query,
        });
        index
    }
    fn define(
        &mut self,
        kind: NameKind,
        name: String,
        condition: Option<usize>,
        query: RuleQuery,
        order: Vec<usize>,
    ) {
        let definitions = self
            .definitions
            .entry(kind)
            .or_default()
            .entry(name)
            .or_default();
        let index = definitions.partition_point(|definition| definition.order < order);
        definitions.insert(
            index,
            NamedDefinition {
                order,
                condition,
                query,
            },
        );
        self.dirty = true;
    }
    fn root(&mut self, origin: Origin, tree_scope: Option<TreeScopeId>) -> usize {
        *self.roots.entry((origin, tree_scope)).or_insert_with(|| {
            let index = self.prepared.scopes.len();
            self.prepared
                .scopes
                .push(Some(self.prepared.layers.root(origin, tree_scope)));
            index
        })
    }
    fn input(
        &mut self,
        input: &StylesheetInput,
        origin: Origin,
        order: Vec<usize>,
    ) -> Result<(), StyleError> {
        let (base, media, owner, encoding) = match input {
            StylesheetInput::Inline {
                base,
                media,
                owner,
                fallback_encoding,
                ..
            }
            | StylesheetInput::External {
                base,
                media,
                owner,
                fallback_encoding,
                ..
            } => (base, media, owner, fallback_encoding),
        };
        if owner.is_some_and(|owner| self.tree.element(owner).is_none()) {
            return Err(StyleError::Invalid(
                "stylesheet owner belongs to another document".into(),
            ));
        }
        let owner = owner.and_then(|owner| self.tree.element(owner));
        let tree_scope = owner
            .filter(|owner| !owner.is_in_document_tree())
            .map(crate::dom::Element::tree_scope);
        let scope = self.root(origin, tree_scope);
        let (slot, selection) = if let Some(element) = owner.filter(|_| origin == Origin::Author) {
            let condition = self.condition(None, None, RuleQuery::Constant(true));
            let slot = self.sets.len();
            self.sets.push(SourceSet {
                condition,
                title: if tree_scope.is_none() {
                    element.attr("title").unwrap_or_default()
                } else {
                    ""
                }
                .into(),
                alternate: element.name() == "link" && element.has_rel("alternate"),
                unavailable: false,
            });
            self.selection_dirty = true;
            (Some(slot), Some(condition))
        } else {
            (None, None)
        };
        let condition =
            Some(self.condition(selection, None, RuleQuery::Media(MediaQuery::parse(media)?)));
        let context = Context {
            base: base.clone(),
            encoding: encoding.clone(),
            origin,
            scope,
            condition,
            order,
            depth: 0,
            path: Vec::new(),
            implicit_root: owner
                .and_then(|owner| {
                    owner
                        .parent(crate::dom::TreeRelation::Selector)
                        .or_else(|| owner.shadow_host())
                })
                .map(crate::dom::Element::id),
            tree_scope,
            encapsulation_order: owner.map_or(crate::dom::DOCUMENT_CONTEXT_ORDER, |owner| {
                owner.encapsulation_order()
            }),
        };
        match input {
            StylesheetInput::Inline { text, .. } => {
                if origin != Origin::UserAgent
                    && let Err(error) = self.budget.bytes(text.len(), location(base))
                {
                    self.ignored(&error, IGNORED_STYLESHEET);
                    return Ok(());
                }
                self.sheet(Arc::from(text.as_str()), &context);
                Ok(())
            }
            StylesheetInput::External { reference, .. } => {
                self.fetches.insert(
                    context.order.clone(),
                    Fetch {
                        slot,
                        reference: reference.clone(),
                        location: location(base),
                        context,
                    },
                );
                Ok(())
            }
        }
    }
    /// Adds a stylesheet's rules. Past a stylesheet limit the rest of it is ignored: the
    /// rules before the limit apply.
    fn sheet(&mut self, text: Arc<str>, context: &Context) {
        if let Err(error) = self.sheet_rules(text, context) {
            self.ignored(&error, TRUNCATED_STYLESHEET);
        }
    }
    /// Reports stylesheet content ignored for `error`, at the limit's location.
    fn ignored(&self, error: &StyleError, consequence: &str) {
        let location = match error {
            StyleError::StylesheetLimit { location, .. } => (**location).clone(),
            _ => location(&self.prepared.document_base),
        };
        ignored(&self.prepared.diagnostics, location, error, consequence);
    }
    fn sheet_rules(&mut self, text: Arc<str>, context: &Context) -> Result<(), StyleError> {
        let budget = if context.origin == Origin::UserAgent {
            &mut self.trusted_budget
        } else {
            &mut self.budget
        };
        let parsed = rules(
            text,
            &context.base,
            crate::style::selector::selector_origin(context.origin),
            &self.prepared.diagnostics,
            budget,
        )?;
        let mut pending: Vec<_> = parsed
            .into_iter()
            .enumerate()
            .rev()
            .map(|(index, rule)| {
                let mut order = context.order.clone();
                order.push(index);
                (
                    rule,
                    context.scope,
                    context.condition,
                    0,
                    order,
                    None,
                    None,
                    None,
                )
            })
            .collect();
        let mut chains = vec![None];
        while let Some((
            rule,
            parent,
            condition,
            list,
            order,
            nearest_style,
            style_scope,
            container,
        )) = pending.pop()
        {
            let previous = chains[list].take();
            let kind = match rule {
                Rule::Namespace | Rule::Inert => continue,
                Rule::Scope(scope, source) => {
                    let budget = if context.origin == Origin::UserAgent {
                        &mut self.trusted_budget
                    } else {
                        &mut self.budget
                    };
                    let ancestors: &[crate::style::selector::RuleSelector] =
                        nearest_style.as_deref().unwrap_or(&[]);
                    budget.nesting(scope.work(ancestors), || source.location())?;
                    let scope = scope.prepare(
                        ancestors,
                        style_scope,
                        context.implicit_root,
                        crate::style::selector::tree_context(context.origin, context.tree_scope),
                    )?;
                    let index = self.prepared.style_scopes.len();
                    self.prepared.style_scopes.push(scope);
                    let parsed = source.parse(&self.prepared.diagnostics, budget)?;
                    let child = chains.len();
                    chains.push(None);
                    pending.extend(parsed.into_iter().enumerate().rev().map(
                        |(child_index, rule)| {
                            let mut order = order.clone();
                            order.push(child_index);
                            (
                                rule,
                                parent,
                                condition,
                                child,
                                order,
                                nearest_style.clone(),
                                Some(index),
                                container,
                            )
                        },
                    ));
                    continue;
                }
                Rule::Container(query, source) => {
                    let index = self.prepared.container_rules.len();
                    self.prepared.container_rules.push(
                        crate::style::container_queries::ContainerRule {
                            parent: container,
                            query,
                        },
                    );
                    let budget = if context.origin == Origin::UserAgent {
                        &mut self.trusted_budget
                    } else {
                        &mut self.budget
                    };
                    let parsed = source.parse(&self.prepared.diagnostics, budget)?;
                    let child = chains.len();
                    chains.push(None);
                    pending.extend(parsed.into_iter().enumerate().rev().map(
                        |(child_index, rule)| {
                            let mut order = order.clone();
                            order.push(child_index);
                            (
                                rule,
                                parent,
                                condition,
                                child,
                                order,
                                nearest_style.clone(),
                                style_scope,
                                Some(index),
                            )
                        },
                    ));
                    continue;
                }
                Rule::Style(rule) => {
                    let budget = if context.origin == Origin::UserAgent {
                        &mut self.trusted_budget
                    } else {
                        &mut self.budget
                    };
                    let ancestors: &[crate::style::selector::RuleSelector] =
                        nearest_style.as_deref().unwrap_or(&[]);
                    if nearest_style.is_some() {
                        budget.nesting(rule.work(ancestors), || rule.location())?;
                    }
                    // A nested rule whose selectors resolve past the selector limits is invalid.
                    let nested = match crate::style::nesting::prepare_nested(ancestors, &rule) {
                        Ok(nested) => nested,
                        Err(error) => {
                            ignored(
                                &self.prepared.diagnostics,
                                rule.location(),
                                &error,
                                IGNORED_RULE,
                            );
                            continue;
                        }
                    };
                    for prepared in nested {
                        let parsed = prepared.source.parse(&self.prepared.diagnostics, budget)?;
                        let child = chains.len();
                        chains.push(None);
                        pending.extend(parsed.into_iter().enumerate().rev().map(
                            |(index, rule)| {
                                let mut order = order.clone();
                                order.push(index);
                                (
                                    rule,
                                    parent,
                                    condition,
                                    child,
                                    order,
                                    Some(prepared.selectors.clone()),
                                    style_scope,
                                    container,
                                )
                            },
                        ));
                    }
                    continue;
                }
                Rule::Declarations(declarations, initial) => {
                    let selectors = nearest_style.ok_or_else(|| {
                        StyleError::Invalid("declarations without an ancestor style rule".into())
                    })?;
                    let budget = if context.origin == Origin::UserAgent {
                        &mut self.trusted_budget
                    } else {
                        &mut self.budget
                    };
                    if !initial {
                        budget.rule(|| declarations[0].location.clone())?;
                    }
                    budget.nesting(selectors.len(), || declarations[0].location.clone())?;
                    PendingKind::Content(ContentRule::Style(selectors.to_vec(), declarations))
                }
                Rule::Content(rule) => PendingKind::Content(rule),
                Rule::LayerStatement(names) => PendingKind::LayerStatement(names),
                Rule::LayerBlock(name, source) => {
                    let scope = self.prepared.scopes.len();
                    self.prepared.scopes.push(None);
                    let budget = if context.origin == Origin::UserAgent {
                        &mut self.trusted_budget
                    } else {
                        &mut self.budget
                    };
                    let parsed = source.parse(&self.prepared.diagnostics, budget)?;
                    let child = chains.len();
                    chains.push(None);
                    pending.extend(parsed.into_iter().enumerate().rev().map(|(index, rule)| {
                        let mut order = order.clone();
                        order.push(index);
                        (
                            rule,
                            scope,
                            condition,
                            child,
                            order,
                            nearest_style.clone(),
                            style_scope,
                            container,
                        )
                    }));
                    PendingKind::LayerBlock(name, scope)
                }
                Rule::Conditional(query, alternative, source) => {
                    let index =
                        self.condition(condition, if alternative { previous } else { None }, query);
                    chains[list] = Some(index);
                    let budget = if context.origin == Origin::UserAgent {
                        &mut self.trusted_budget
                    } else {
                        &mut self.budget
                    };
                    let parsed = source.parse(&self.prepared.diagnostics, budget)?;
                    let child = chains.len();
                    chains.push(None);
                    pending.extend(parsed.into_iter().enumerate().rev().map(
                        |(child_index, rule)| {
                            let mut order = order.clone();
                            order.push(child_index);
                            (
                                rule,
                                parent,
                                Some(index),
                                child,
                                order,
                                nearest_style.clone(),
                                style_scope,
                                container,
                            )
                        },
                    ));
                    continue;
                }
                Rule::CustomMedia(name, query) => {
                    self.define(
                        NameKind::Media,
                        name,
                        condition,
                        RuleQuery::Media(query),
                        order,
                    );
                    continue;
                }
                Rule::SupportDefinition(name, source) => {
                    let budget = if context.origin == Origin::UserAgent {
                        &mut self.trusted_budget
                    } else {
                        &mut self.budget
                    };
                    let supported = source.supports(budget)?;
                    self.define(
                        NameKind::Supports,
                        name,
                        condition,
                        RuleQuery::Constant(supported),
                        order,
                    );
                    chains[list] =
                        Some(self.condition(condition, None, RuleQuery::Constant(supported)));
                    continue;
                }
                Rule::Import(import, location) => {
                    let context = Context {
                        scope: parent,
                        condition,
                        order,
                        ..context.clone()
                    };
                    self.import(import, location, context);
                    continue;
                }
            };
            self.all.insert(
                order,
                PendingRule {
                    kind,
                    base: context.base.clone(),
                    origin: context.origin,
                    scope: parent,
                    condition,
                    style_scope,
                    container,
                    tree_scope: context.tree_scope,
                    encapsulation_order: context.encapsulation_order,
                },
            );
        }
        Ok(())
    }
    fn import(&mut self, import: ImportRule, location: SourceLocation, mut context: Context) {
        context.condition =
            Some(self.condition(context.condition, None, RuleQuery::Media(import.media)));
        if let Some(support) = import.support {
            context.condition =
                Some(self.condition(context.condition, None, RuleQuery::Supports(support)));
        }
        if let Some(layer) = import.layer {
            let scope = self.prepared.scopes.len();
            self.prepared.scopes.push(None);
            self.all.insert(
                context.order.clone(),
                PendingRule {
                    kind: PendingKind::LayerBlock(layer, scope),
                    base: context.base.clone(),
                    origin: context.origin,
                    scope: context.scope,
                    condition: context.condition,
                    style_scope: None,
                    container: None,
                    tree_scope: context.tree_scope,
                    encapsulation_order: context.encapsulation_order,
                },
            );
            context.scope = scope;
        }
        context.depth += 1;
        self.fetches.insert(
            context.order.clone(),
            Fetch {
                slot: None,
                reference: import.reference,
                context,
                location,
            },
        );
    }
    fn unavailable(&mut self, slot: Option<usize>) {
        if let Some(slot) = slot {
            self.sets[slot].unavailable = true;
            self.selection_dirty = true;
        }
    }
    fn refresh(&mut self) {
        if self.selection_dirty {
            let preferred = self.forced_set.as_ref().or_else(|| {
                self.sets
                    .iter()
                    .find(|set| !set.unavailable && !set.alternate && !set.title.is_empty())
                    .map(|set| &set.title)
            });
            for set in &self.sets {
                let enabled =
                    !set.unavailable && (set.title.is_empty() || preferred == Some(&set.title));
                let query = &mut self.prepared.conditions[set.condition].query;
                if !matches!(query,RuleQuery::Constant(value) if *value==enabled) {
                    *query = RuleQuery::Constant(enabled);
                    self.dirty = true;
                }
            }
            self.selection_dirty = false;
        }
        if self.dirty {
            self.prepared.environment.custom_media.clear();
            self.prepared.environment.supports.borrow_mut().clear();
            conditions::resolve_names(
                &self.definitions,
                &self.prepared.conditions,
                &mut self.prepared.environment,
            );
            self.applicable.clear();
            self.dirty = false;
        }
        conditions::extend_applicability(
            &self.prepared.conditions,
            &self.prepared.environment,
            &mut self.applicable,
        );
    }
    fn cycle(&self, fetch: &Fetch, url: &Url) {
        self.prepared
            .diagnostics
            .borrow_mut()
            .push(StyleDiagnostic {
                location: fetch.location.clone(),
                kind: StyleDiagnosticKind::ImportCycle,
                context: format!("stylesheet import cycle: {url}"),
            });
    }
    /// A source that cannot be fetched, or that policy denies, is skipped as unavailable
    /// (CSS Cascade 5 §2.1; HTML §4.2.4 "the link is an unavailable source").
    fn fetch(&mut self, fetch: Fetch) {
        if fetch.context.depth > self.budget.limits.depth {
            self.prepared
                .diagnostics
                .borrow_mut()
                .push(StyleDiagnostic {
                    location: fetch.location,
                    kind: StyleDiagnosticKind::UnavailableSource,
                    context: format!(
                        "stylesheet {} exceeds the {DEPTH} limit {}",
                        fetch.reference, self.budget.limits.depth
                    ),
                });
            return;
        }
        let mut requested = match fetch.context.base.resolve(&fetch.reference) {
            Ok(requested) => requested,
            Err(error) => {
                self.unavailable_source(fetch, error);
                return;
            }
        };
        requested.set_fragment(None);
        if fetch.context.path.contains(&requested) {
            self.cycle(&fetch, &requested);
            return;
        }
        let key = (fetch.context.base.clone(), fetch.reference.clone());
        let resource = if let Some(resource) = self.cache.get(&key) {
            resource.clone()
        } else {
            match self
                .resolver
                .resolve_with_metadata(&fetch.reference, &fetch.context.base)
            {
                Ok(resource) => Arc::new(resource),
                Err(error) => {
                    self.unavailable_source(fetch, error);
                    return;
                }
            }
        };
        let mut identity = if requested.scheme() == "data" {
            requested.clone()
        } else {
            match &resource.resource.base {
                ResourceBase::Url(url) => url.clone(),
                ResourceBase::Directory(_) => requested.clone(),
            }
        };
        identity.set_fragment(None);
        if fetch.context.path.contains(&identity) {
            self.cycle(&fetch, &identity);
            return;
        }
        if let Err(error) = self
            .budget
            .bytes(resource.resource.bytes.len(), fetch.location.clone())
        {
            self.unavailable(fetch.slot);
            self.ignored(&error, IGNORED_STYLESHEET);
            return;
        }
        if !self.css_type(resource.metadata.media_type.as_deref(), &identity) {
            self.unavailable(fetch.slot);
            self.prepared
                .diagnostics
                .borrow_mut()
                .push(StyleDiagnostic {
                    location: fetch.location,
                    kind: StyleDiagnosticKind::UnavailableSource,
                    context: format!(
                        "stylesheet {} has unsupported Content-Type {:?}",
                        fetch.reference, resource.metadata.media_type
                    ),
                });
            return;
        }
        self.cache.entry(key).or_insert_with(|| resource.clone());
        let decoded = (self.decode)(
            &resource.resource.bytes,
            resource.metadata.charset.as_deref(),
            Some(&fetch.context.encoding),
        );
        if decoded.text.len() > resource.resource.bytes.len()
            && let Err(error) = self.budget.bytes(
                decoded.text.len() - resource.resource.bytes.len(),
                fetch.location.clone(),
            )
        {
            self.unavailable(fetch.slot);
            self.ignored(&error, IGNORED_STYLESHEET);
            return;
        }
        if decoded.had_errors {
            self.prepared
                .source_diagnostics
                .push(SourceDiagnostic::Decoding {
                    base: resource.resource.base.clone(),
                    reference: fetch.reference,
                    encoding: decoded.encoding.clone(),
                });
        }
        let mut context = fetch.context;
        context.base = resource.resource.base.clone();
        context.encoding = decoded.encoding;
        context.path.push(requested);
        context.path.push(identity);
        self.sheet(Arc::from(decoded.text), &context);
    }
    fn unavailable_source(&mut self, fetch: Fetch, error: ResourceError) {
        self.unavailable(fetch.slot);
        self.prepared
            .source_diagnostics
            .push(SourceDiagnostic::Unavailable {
                base: fetch.context.base,
                reference: fetch.reference,
                error,
            });
    }
    fn css_type(&self, media_type: Option<&str>, response: &Url) -> bool {
        media_type.is_none_or(|kind| kind.eq_ignore_ascii_case(CSS_MEDIA_TYPE))
            || (self.tree.quirks_mode() == QuirksMode::Quirks
                && self
                    .document_url
                    .as_ref()
                    .is_some_and(|document| document.origin() == response.origin()))
    }
    fn finish(mut self) -> PreparedStylesheets {
        loop {
            self.refresh();
            let next = self
                .fetches
                .iter()
                .find(|(_, fetch)| {
                    fetch
                        .context
                        .condition
                        .is_none_or(|index| self.applicable[index].matching)
                })
                .map(|(order, _)| order.clone());
            let Some(order) = next else {
                break;
            };
            let fetch = self
                .fetches
                .remove(&order)
                .expect("the selected source is pending");
            self.fetch(fetch);
        }
        self.prepared.all = self.all.into_values().collect();
        self.prepared
    }
}
/// Reports stylesheet content at `location` ignored for `error`.
fn ignored(
    diagnostics: &RefCell<Vec<StyleDiagnostic>>,
    location: SourceLocation,
    error: &StyleError,
    consequence: &str,
) {
    diagnostics.borrow_mut().push(StyleDiagnostic {
        location,
        kind: StyleDiagnosticKind::of(error, StyleDiagnosticKind::InvalidRule),
        context: format!("{error}; {consequence}"),
    });
}
fn location(base: &ResourceBase) -> SourceLocation {
    SourceLocation {
        base: base.clone(),
        line: 1,
        column: 1,
    }
}

pub(super) fn prepare_with_limits(
    tree: &DocumentTree,
    sources: &StylesheetSources,
    resolver: &dyn ResourceResolver,
    settings: &PrintSettings,
    presentation: Presentation,
    decode: CssDecoder,
    limits: Limits,
) -> Result<PreparedStylesheets, StyleError> {
    let mut loader = Loader::new(
        tree,
        sources,
        resolver,
        settings,
        presentation,
        decode,
        limits,
    )?;
    let visual = sources
        .document_encoding
        .eq_ignore_ascii_case(VISUAL_HEBREW_ENCODING)
        .then_some(include_str!("ua-visual.css"));
    for (index, text) in std::iter::once(include_str!("ua.css"))
        .chain(visual)
        .enumerate()
    {
        loader.input(
            &StylesheetInput::Inline {
                text: text.into(),
                base: sources.base.clone(),
                media: String::new(),
                owner: None,
                fallback_encoding: "UTF-8".into(),
            },
            Origin::UserAgent,
            vec![UA_SOURCE_ORDER, index],
        )?;
    }
    let inputs = sources
        .user
        .iter()
        .map(|input| (input, Origin::User))
        .chain(
            sources
                .document
                .iter()
                .chain(&sources.author)
                .map(|input| (input, Origin::Author)),
        );
    for (index, (input, origin)) in inputs.enumerate() {
        loader.input(input, origin, vec![index + 1])?;
    }
    Ok(loader.finish())
}
#[cfg(test)]
pub(super) fn inline(
    author: &[crate::style::StylesheetSource],
    user: &[crate::style::StylesheetSource],
    base: &ResourceBase,
    settings: &PrintSettings,
) -> Result<PreparedStylesheets, StyleError> {
    let convert = |source: &crate::style::StylesheetSource| StylesheetInput::Inline {
        text: source.text.clone(),
        base: source.base.clone(),
        media: String::new(),
        owner: None,
        fallback_encoding: "UTF-8".into(),
    };
    let sources = StylesheetSources {
        base: base.clone(),
        author: author.iter().map(convert).collect(),
        user: user.iter().map(convert).collect(),
        document: Vec::new(),
        document_url: base.resolve("").ok(),
        ..Default::default()
    };
    prepare_with_limits(
        &DocumentTree::parse(""),
        &sources,
        &crate::resources::FileResolver::default(),
        settings,
        Presentation::Paged,
        crate::document::encoding::stylesheet,
        Limits::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::DocumentTree;
    use crate::resources::{
        FileResolver, Resource, ResourceBase, ResourceError, ResourceResolver, Url,
    };
    use crate::style::{PrintSettings, StyleError};
    use std::cell::Cell;

    fn decode(bytes: &[u8], _: Option<&str>, _: Option<&str>) -> DecodedStylesheet {
        DecodedStylesheet {
            text: String::from_utf8(bytes.to_vec()).expect("UTF-8 test fixture"),
            encoding: "UTF-8".into(),
            had_errors: false,
        }
    }
    fn source(text: &str) -> StylesheetSources {
        StylesheetSources {
            author: vec![StylesheetInput::Inline {
                text: text.into(),
                base: ResourceBase::default(),
                media: String::new(),
                owner: None,
                fallback_encoding: "UTF-8".into(),
            }],
            ..Default::default()
        }
    }
    #[test]
    fn visual_hebrew_ua_rules_preserve_control_exceptions_and_selector_specificity() {
        use crate::dom::Selector;
        use crate::style::{Style, Stylesheet, UnicodeBidi};
        let tree = DocumentTree::parse(
            "<!doctype html><p id=paragraph></p><span id=inline></span><math><mi id=foreign>x</mi></math><svg><a id=foreign-link href='https://example.test'>Link</a></svg><input id=text dir=auto><input id=unknown type=unknown dir=auto><input id=email type=email dir=auto><input id=submit type=SUBMIT><input id=reset type=reset dir=auto><input id=button type=button><textarea id=textarea></textarea><textarea id=auto dir=auto></textarea><pre id=pre dir=auto></pre><bdo id=override dir=rtl></bdo><bdi id=isolate></bdi><bdi id=directed dir=auto></bdi>",
        );
        let sources = StylesheetSources {
            document_encoding: VISUAL_HEBREW_ENCODING.into(),
            ..Default::default()
        };
        let sheet = Stylesheet::prepare(
            &tree,
            &sources,
            &FileResolver::default(),
            &PrintSettings::default(),
            Presentation::Paged,
            decode,
        )
        .unwrap();
        for (id, expected) in [
            ("paragraph", UnicodeBidi::BidiOverride),
            ("inline", UnicodeBidi::BidiOverride),
            ("foreign", UnicodeBidi::BidiOverride),
            ("text", UnicodeBidi::Normal),
            ("unknown", UnicodeBidi::Normal),
            ("email", UnicodeBidi::Normal),
            ("submit", UnicodeBidi::BidiOverride),
            ("reset", UnicodeBidi::BidiOverride),
            ("button", UnicodeBidi::BidiOverride),
            ("textarea", UnicodeBidi::Normal),
            ("auto", UnicodeBidi::Plaintext),
            ("pre", UnicodeBidi::Plaintext),
            ("override", UnicodeBidi::IsolateOverride),
            ("isolate", UnicodeBidi::Isolate),
            ("directed", UnicodeBidi::BidiOverride),
        ] {
            let element = tree
                .select(&Selector::parse(&format!("#{id}")).unwrap())
                .next()
                .unwrap();
            assert_eq!(
                sheet
                    .computed(element, &Style::default())
                    .unwrap()
                    .unicode_bidi,
                expected,
                "the exact visual-encoding UA selectors must cascade for {id}"
            );
        }
        let foreign_link = tree
            .select(&Selector::parse("#foreign-link").unwrap())
            .next()
            .unwrap();
        assert_eq!(
            sheet
                .computed(foreign_link, &Style::default())
                .unwrap()
                .color
                .rgb,
            [0, 0, 0],
            "the base HTML UA link selector must retain its namespace despite the encoding rule's explicit wildcard"
        );
    }
    #[test]
    fn nesting_declaration_runs_charge_the_shared_rule_budget() {
        let tree = DocumentTree::parse("<p>Bounded</p>");
        let resolver = Resolver {
            calls: Cell::new(0),
        };
        let result = prepare_with_limits(
            &tree,
            &source("p{color:red;&{color:blue}color:green;&{color:yellow}color:black}"),
            &resolver,
            &PrintSettings::default(),
            Presentation::Paged,
            decode,
            Limits {
                rules: 4,
                ..Default::default()
            },
        );
        assert!(
            limit_exceeded(result)
                .context
                .contains("stylesheet rules exceeds 4"),
            "implicit declaration runs must not bypass the same prepared-rule limit as authored rules"
        );
    }
    #[test]
    fn nesting_work_is_bounded_before_binding_large_selector_lists() {
        const PARENT_SELECTORS: usize = 10_000;
        const CHILD_SELECTORS: usize = 200;
        let tree = DocumentTree::parse("<p>Bounded</p>");
        let resolver = Resolver {
            calls: Cell::new(0),
        };
        let parent = (0..PARENT_SELECTORS)
            .map(|index| format!(".parent{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let child = (0..CHILD_SELECTORS)
            .map(|index| format!(".child{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let css = format!("{parent}{{{child}{{color:red}}}}");
        let result = prepare_with_limits(
            &tree,
            &source(&css),
            &resolver,
            &PrintSettings::default(),
            Presentation::Paged,
            decode,
            Limits::default(),
        );
        assert!(
            limit_exceeded(result)
                .context
                .contains("stylesheet nesting work exceeds"),
            "a compact sheet with a large parent/child product must be rejected before allocating/binding the expansion"
        );
    }
    #[test]
    fn nesting_work_counts_repeated_resolved_parent_references() {
        const STYLE_DEPTH: usize = 24;
        let tree = DocumentTree::parse("<p>Bounded</p>");
        let resolver = Resolver {
            calls: Cell::new(0),
        };
        let css = format!(
            ".item{{{}color:red{}}}",
            "&&{".repeat(STYLE_DEPTH),
            "}".repeat(STYLE_DEPTH)
        );
        let result = prepare_with_limits(
            &tree,
            &source(&css),
            &resolver,
            &PrintSettings::default(),
            Presentation::Paged,
            decode,
            Limits::default(),
        );
        assert!(
            limit_exceeded(result)
                .context
                .contains("stylesheet nesting work exceeds"),
            "sharing the selector AST must not hide exponential logical work from repeated ampersands"
        );
    }
    /// The limit a preparation reports instead of failing: the input past it is ignored.
    fn limit_exceeded(result: Result<PreparedStylesheets, StyleError>) -> StyleDiagnostic {
        let prepared =
            result.unwrap_or_else(|error| panic!("a stylesheet limit must not fail: {error}"));
        let diagnostics = prepared.diagnostics.into_inner();
        diagnostics
            .iter()
            .find(|diagnostic| diagnostic.kind == StyleDiagnosticKind::LimitExceeded)
            .cloned()
            .unwrap_or_else(|| panic!("a reported stylesheet limit: {diagnostics:?}"))
    }
    struct Resolver {
        calls: Cell<usize>,
    }
    impl ResourceResolver for Resolver {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.calls.set(self.calls.get() + 1);
            Ok(Resource {
                bytes: if reference == "root.css" {
                    b"@import 'child.css';p{color:red}".to_vec()
                } else {
                    b"p{color:blue}".to_vec()
                },
                base: base.resolve(reference)?.into(),
            })
        }
    }
    #[test]
    fn loader_defaults_match_the_approved_resource_policies() {
        assert_eq!(MAX_STYLESHEET_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_IMPORT_DEPTH, 64);
        assert_eq!(MAX_STYLESHEET_RULES, 1_000_000);
    }
    #[test]
    fn loader_byte_and_rule_limits_are_inclusive_and_source_aware() {
        const CSS: &str = "p{color:red}";
        let tree = DocumentTree::parse("<p>Bounds</p>");
        let resolver = Resolver {
            calls: Cell::new(0),
        };
        let limits = Limits {
            bytes: CSS.len(),
            rules: 1,
            ..Default::default()
        };
        assert!(
            prepare_with_limits(
                &tree,
                &source(CSS),
                &resolver,
                &PrintSettings::default(),
                Presentation::Paged,
                decode,
                limits
            )
            .is_ok()
        );
        let result = prepare_with_limits(
            &tree,
            &source(CSS),
            &resolver,
            &PrintSettings::default(),
            Presentation::Paged,
            decode,
            Limits {
                bytes: CSS.len() - 1,
                ..limits
            },
        );
        let diagnostic = limit_exceeded(result);
        assert!(
            diagnostic.context.contains("stylesheet bytes exceeds")
                && diagnostic.location.base == ResourceBase::default(),
            "{diagnostic}"
        );
        let result = prepare_with_limits(
            &tree,
            &source("p{color:red}p{color:blue}"),
            &resolver,
            &PrintSettings::default(),
            Presentation::Paged,
            decode,
            limits,
        );
        limit_exceeded(result);
        let result = prepare_with_limits(
            &tree,
            &source("p{color:red}p{color:blue}"),
            &resolver,
            &PrintSettings::default(),
            Presentation::Paged,
            decode,
            Limits {
                bytes: MAX_STYLESHEET_BYTES,
                ..limits
            },
        );
        assert!(
            limit_exceeded(result)
                .context
                .contains("stylesheet rules exceeds 1")
        );
        assert_eq!(resolver.calls.get(), 0);
    }
    #[test]
    fn loader_skips_imports_past_the_depth_limit_before_fetching_them() {
        let base: ResourceBase = Url::parse("https://styles.test/document.html")
            .unwrap()
            .into();
        let sources = StylesheetSources {
            base: base.clone(),
            document: vec![StylesheetInput::External {
                reference: "root.css".into(),
                base,
                media: String::new(),
                owner: None,
                fallback_encoding: "UTF-8".into(),
            }],
            ..Default::default()
        };
        let resolver = Resolver {
            calls: Cell::new(0),
        };
        let tree = DocumentTree::parse("<p>Depth</p>");
        let prepared = prepare_with_limits(
            &tree,
            &sources,
            &resolver,
            &PrintSettings::default(),
            Presentation::Paged,
            decode,
            Limits {
                depth: 0,
                ..Default::default()
            },
        )
        .expect("an import past the depth limit is skipped, not fatal");
        let skipped = prepared
            .diagnostics
            .borrow()
            .iter()
            .filter(|diagnostic| {
                diagnostic.kind == StyleDiagnosticKind::UnavailableSource
                    && diagnostic.context.contains("child.css")
            })
            .count();
        assert_eq!(
            skipped,
            1,
            "the skipped import is reported: {:?}",
            prepared.diagnostics.borrow()
        );
        assert_eq!(
            resolver.calls.get(),
            1,
            "the over-limit child must not reach the resolver"
        );
    }
    #[cfg(unix)]
    #[test]
    fn loader_canonical_file_cycles_do_not_reparse_aliases() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/style/tests/fixtures/imports");
        let base: ResourceBase = directory.into();
        let sources = StylesheetSources {
            base: base.clone(),
            document: vec![StylesheetInput::External {
                reference: "canonical.css".into(),
                base,
                media: String::new(),
                owner: None,
                fallback_encoding: "UTF-8".into(),
            }],
            ..Default::default()
        };
        let tree = DocumentTree::parse("<p>Canonical cycle</p>");
        let prepared = prepare_with_limits(
            &tree,
            &sources,
            &FileResolver::default(),
            &PrintSettings::default(),
            Presentation::Paged,
            decode,
            Limits {
                bytes: include_str!("tests/fixtures/imports/canonical.css").len(),
                ..Default::default()
            },
        )
        .expect("a canonical alias cycle is cut before charging or parsing the same sheet again");
        assert_eq!(
            prepared
                .diagnostics
                .borrow()
                .iter()
                .filter(|diagnostic| diagnostic.kind == StyleDiagnosticKind::ImportCycle)
                .count(),
            1
        );
    }

    #[test]
    fn loader_rule_limits_cover_nested_groups_and_page_margin_rules() {
        let tree = DocumentTree::parse("<p>Nested limits</p>");
        let resolver = Resolver {
            calls: Cell::new(0),
        };
        for (css, limit) in [
            ("@media print{p{color:red}}", 1),
            ("@page{@top-center{content:'x'}}", 1),
            (
                "@supports-condition --probe{@page{@top-center{content:'x'}}}",
                2,
            ),
        ] {
            let result = prepare_with_limits(
                &tree,
                &source(css),
                &resolver,
                &PrintSettings::default(),
                Presentation::Paged,
                decode,
                Limits {
                    rules: limit,
                    ..Default::default()
                },
            );
            assert!(
                limit_exceeded(result)
                    .context
                    .contains("stylesheet rules exceeds"),
                "nested rule budget must cover {css}"
            );
        }
    }
    #[test]
    fn loader_nested_limit_errors_retain_the_declaring_source_and_line() {
        let tree = DocumentTree::parse("<p>Source</p>");
        let resolver = Resolver {
            calls: Cell::new(0),
        };
        let expected: ResourceBase = Url::parse("https://styles.test/nested.css").unwrap().into();
        for (css, limit, line) in [
            ("\n\n@media print{\n p{color:red}\n}", 1, 4),
            (
                "\n\n@supports-condition --probe{\n @page{\n @top-center{content:'x'}\n}}",
                2,
                5,
            ),
        ] {
            let mut sources = source(css);
            if let StylesheetInput::Inline { base, .. } = &mut sources.author[0] {
                *base = expected.clone();
            }
            let result = prepare_with_limits(
                &tree,
                &sources,
                &resolver,
                &PrintSettings::default(),
                Presentation::Paged,
                decode,
                Limits {
                    rules: limit,
                    ..Default::default()
                },
            );
            let diagnostic = limit_exceeded(result);
            assert!(
                diagnostic.location.base == expected && diagnostic.location.line == line,
                "source-aware nested limit: {css}: {diagnostic}"
            );
        }
    }
    #[test]
    fn loader_rejected_mime_bodies_still_obey_the_aggregate_byte_limit() {
        struct WrongType;
        impl ResourceResolver for WrongType {
            fn resolve(
                &self,
                reference: &str,
                base: &ResourceBase,
            ) -> Result<Resource, ResourceError> {
                Ok(Resource {
                    bytes: b"xx".to_vec(),
                    base: base.resolve(reference)?.into(),
                })
            }
            fn resolve_with_metadata(
                &self,
                reference: &str,
                base: &ResourceBase,
            ) -> Result<ResolvedResource, ResourceError> {
                Ok(ResolvedResource {
                    resource: self.resolve(reference, base)?,
                    metadata: crate::resources::ResourceMetadata {
                        media_type: Some("text/plain".into()),
                        ..Default::default()
                    },
                })
            }
        }
        let sources = StylesheetSources {
            document: vec![StylesheetInput::External {
                reference: "wrong-type.css".into(),
                base: ResourceBase::default(),
                media: String::new(),
                owner: None,
                fallback_encoding: "UTF-8".into(),
            }],
            ..Default::default()
        };
        let result = prepare_with_limits(
            &DocumentTree::parse("<!doctype html><p>Limit</p>"),
            &sources,
            &WrongType,
            &PrintSettings::default(),
            Presentation::Paged,
            decode,
            Limits {
                bytes: 1,
                ..Default::default()
            },
        );
        assert!(
            limit_exceeded(result)
                .context
                .contains("stylesheet bytes exceeds 1"),
            "rejected types must not accumulate unaccounted response bodies in the job cache"
        );
    }
}
