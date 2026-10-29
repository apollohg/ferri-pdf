use crate::style::{
    Style, StyleError,
    declaration::{self, Declared, Pending, PropertyDeclaration, Specified},
    grammar::Property,
    layers::{LayerId, LayerPriority},
    logical::{self, Flow, Flows},
    wide_keywords::WideKeyword,
};
use std::{
    cell::RefCell,
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap},
    rc::Rc,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Origin {
    UserAgent,
    User,
    AuthorHint,
    Author,
}

impl Origin {
    fn rollback(self) -> Self {
        match self {
            Self::Author | Self::AuthorHint => Self::AuthorHint,
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct DeclarationContext {
    pub origin: Origin,
    pub attached: bool,
    pub encapsulation_order: usize,
    pub layer: LayerId,
    pub specificity: u32,
    pub scope_proximity: Option<usize>,
    pub order: usize,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum Level {
    NormalUserAgent,
    NormalUser,
    PresentationalHint,
    NormalAuthor,
    ImportantAuthor,
    ImportantUser,
    ImportantUserAgent,
}

#[derive(Clone)]
pub(super) struct Candidate<T> {
    pub value: T,
    pub location: Option<Arc<crate::style::SourceLocation>>,
    pub context: DeclarationContext,
    pub important: bool,
}

impl<T> Candidate<T> {
    pub(crate) fn new(value: T, context: DeclarationContext, important: bool) -> Self {
        Self {
            value,
            location: None,
            context,
            important,
        }
    }

    pub(crate) fn with_location(
        mut self,
        location: Option<Arc<crate::style::SourceLocation>>,
    ) -> Self {
        self.location = location;
        self
    }

    fn priority(
        &self,
    ) -> (
        Level,
        Reverse<usize>,
        bool,
        LayerPriority,
        u32,
        Reverse<usize>,
        usize,
    ) {
        let level = match (self.context.origin, self.important) {
            (Origin::UserAgent, false) => Level::NormalUserAgent,
            (Origin::User, false) => Level::NormalUser,
            (Origin::AuthorHint, false) => Level::PresentationalHint,
            (Origin::Author, false) => Level::NormalAuthor,
            (Origin::Author | Origin::AuthorHint, true) => Level::ImportantAuthor,
            (Origin::User, true) => Level::ImportantUser,
            (Origin::UserAgent, true) => Level::ImportantUserAgent,
        };
        (
            level,
            Reverse(if self.important {
                usize::MAX - self.context.encapsulation_order
            } else {
                self.context.encapsulation_order
            }),
            self.context.attached,
            self.context.layer.priority(self.important),
            self.context.specificity,
            Reverse(self.context.scope_proximity.unwrap_or(usize::MAX)),
            self.context.order,
        )
    }
}

#[derive(Clone, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Rollback {
    ceiling: Option<Origin>,
    detached: Option<(Origin, usize)>,
    layer_origin: Option<Origin>,
    normal_layers: BTreeMap<(Origin, usize), LayerId>,
    important_layers: BTreeSet<(Origin, usize, LayerId)>,
}

impl Rollback {
    fn excludes<T>(&self, candidate: &Candidate<T>) -> bool {
        let context = candidate.context;
        self.ceiling.is_some_and(|origin| context.origin >= origin)
            || (self.detached == Some((context.origin, context.encapsulation_order))
                && context.attached)
            || self
                .layer_origin
                .is_some_and(|origin| context.origin > origin)
            || (candidate.important
                && self.important_layers.contains(&(
                    context.origin,
                    context.encapsulation_order,
                    context.layer,
                )))
            || (!candidate.important
                && self
                    .normal_layers
                    .get(&(context.origin, context.encapsulation_order))
                    .is_some_and(|layer| context.layer >= *layer))
    }

    fn apply<T>(&mut self, candidate: &Candidate<T>, keyword: Option<WideKeyword>) -> bool {
        let context = candidate.context;
        match keyword {
            Some(WideKeyword::RevertLayer) if candidate.important && context.attached => {
                self.detached = Some((context.origin, context.encapsulation_order));
            }
            Some(WideKeyword::Revert) => {
                self.ceiling = Some(context.origin.rollback());
            }
            Some(WideKeyword::RevertLayer) => {
                self.layer_origin = Some(
                    self.layer_origin
                        .map_or(context.origin, |origin| origin.min(context.origin)),
                );
                self.important_layers.insert((
                    context.origin,
                    context.encapsulation_order,
                    context.layer,
                ));
                self.normal_layers
                    .entry((context.origin, context.encapsulation_order))
                    .and_modify(|layer| *layer = (*layer).min(context.layer))
                    .or_insert(context.layer);
            }
            _ => return false,
        }
        true
    }
}

/// Values must be sorted first; substitution can produce a valid rollback keyword too.
pub(super) fn choose<T, R>(
    candidates: &[Candidate<T>],
    mut compute: impl FnMut(usize, &T, &Rollback) -> Result<R, StyleError>,
    keyword: impl Fn(&R) -> Option<WideKeyword>,
) -> Result<Option<(usize, R)>, StyleError> {
    choose_from(
        candidates,
        Rollback::default(),
        |index, value, rollback| compute(index, value, rollback).map(Some),
        keyword,
    )
}

fn choose_from<T, R>(
    candidates: &[Candidate<T>],
    mut rollback: Rollback,
    mut compute: impl FnMut(usize, &T, &Rollback) -> Result<Option<R>, StyleError>,
    keyword: impl Fn(&R) -> Option<WideKeyword>,
) -> Result<Option<(usize, R)>, StyleError> {
    for (index, candidate) in candidates.iter().enumerate().rev() {
        if rollback.excludes(candidate) {
            continue;
        }
        let Some(value) = compute(index, &candidate.value, &rollback)? else {
            continue;
        };
        if !rollback.apply(candidate, keyword(&value)) {
            return Ok(Some((index, value)));
        }
    }
    Ok(None)
}

pub(super) fn sort<T>(candidates: &mut [Candidate<T>]) {
    candidates.sort_by_key(Candidate::priority);
}

pub(super) type Winners = BTreeMap<Property, Specified>;

#[derive(Clone)]
enum Value {
    Specified(Specified),
    HeaderAlignment(Specified),
    Pending(Rc<Pending>),
}

#[derive(Clone, Default)]
pub(super) struct Cascaded {
    properties: BTreeMap<Property, Vec<Candidate<(Value, crate::fonts::FontNamespace)>>>,
    custom: CustomProperties,
}

pub(super) type CustomProperties = HashMap<Arc<str>, Vec<Candidate<CustomValue>>>;
#[derive(Clone)]
pub(super) struct CustomValue {
    pub source: Arc<str>,
    pub namespace: crate::fonts::FontNamespace,
    pub namespaces: Option<crate::style::selector::Namespaces>,
    contextual: bool,
}

impl Cascaded {
    pub(super) fn header_alignment(&mut self, namespace: crate::fonts::FontNamespace) {
        let value = Specified::Value(Arc::new(crate::style::component::Component::Keyword(
            crate::style::grammar::Keyword::Center,
        )));
        self.properties
            .entry(Property::TextAlign)
            .or_default()
            .push(Candidate::new(
                (Value::HeaderAlignment(value), namespace),
                DeclarationContext {
                    origin: Origin::UserAgent,
                    attached: false,
                    encapsulation_order: crate::dom::DOCUMENT_CONTEXT_ORDER,
                    layer: LayerId::Unlayered,
                    specificity: crate::style::user_agent::CONDITIONAL_SPECIFICITY,
                    scope_proximity: None,
                    order: 0,
                },
                false,
            ));
    }
    /// Whether an author-origin value of any of `properties` survives rolling back
    /// `revert` and `revert-layer` (CSS UI 4 §7.2.1).
    pub(super) fn author_cascades(&self, properties: &[Property]) -> Result<bool, StyleError> {
        for property in properties {
            let Some(candidates) = self.properties.get(property) else {
                continue;
            };
            let mut candidates = candidates.clone();
            sort(&mut candidates);
            let winner = choose(
                &candidates,
                |_, value, _| Ok(value.0.clone()),
                |value| match value {
                    Value::Specified(specified) | Value::HeaderAlignment(specified) => {
                        specified_keyword(specified)
                    }
                    Value::Pending(_) => None,
                },
            )?;
            if winner.is_some_and(|(index, _)| {
                matches!(
                    candidates[index].context.origin,
                    Origin::Author | Origin::AuthorHint
                )
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn is_empty(&self) -> bool {
        self.properties.is_empty() && self.custom.is_empty()
    }
    pub(super) fn extend(&mut self, other: Self) {
        for (property, values) in other.properties {
            self.properties.entry(property).or_default().extend(values);
        }
        for (name, values) in other.custom {
            self.custom.entry(name).or_default().extend(values);
        }
    }
    pub(super) fn contextual(&self) -> bool {
        self.custom.values().flatten().any(|candidate| candidate.value.contextual)
            || self.properties.values().flatten().any(|candidate| matches!(&candidate.value.0, Value::Pending(pending) if pending.value.requires_context()))
    }

    pub(super) fn into_parts(self) -> (Properties, CustomProperties) {
        (Properties::new(self.properties), self.custom)
    }
}

pub(super) fn cascade<'a>(
    declarations: impl IntoIterator<Item = &'a PropertyDeclaration>,
) -> Cascaded {
    let mut values = Cascaded::default();
    for (order, declaration) in declarations.into_iter().enumerate() {
        insert(
            &mut values,
            declaration,
            DeclarationContext {
                encapsulation_order: crate::dom::DOCUMENT_CONTEXT_ORDER,
                origin: declaration.origin,
                attached: false,
                layer: declaration.layer,
                specificity: 0,
                scope_proximity: None,
                order,
            },
        );
    }
    values
}

pub(super) fn insert(
    values: &mut Cascaded,
    declaration: &PropertyDeclaration,
    context: DeclarationContext,
) {
    let candidate = |value| {
        Candidate::new(value, context, declaration.important)
            .with_location(declaration.location.clone())
    };
    match &declaration.declared {
        Declared::Custom(name, value, contextual) => {
            values.custom.entry(name.clone()).or_default().push(
                Candidate::new(
                    CustomValue {
                        source: value.clone(),
                        namespace: declaration.font_namespace,
                        namespaces: declaration.namespaces.clone(),
                        contextual: *contextual,
                    },
                    context,
                    declaration.important,
                )
                .with_location(declaration.location.clone()),
            );
        }
        Declared::Longhand(property, specified) => {
            values
                .properties
                .entry(*property)
                .or_default()
                .push(candidate((
                    Value::Specified(specified.clone()),
                    declaration.font_namespace,
                )));
        }
        Declared::Pending(property, pending) => {
            values
                .properties
                .entry(*property)
                .or_default()
                .push(candidate((
                    Value::Pending(pending.clone()),
                    declaration.font_namespace,
                )));
        }
    }
}

fn specified_keyword(value: &Specified) -> Option<WideKeyword> {
    match value {
        Specified::Wide(keyword) => Some(*keyword),
        _ => None,
    }
}

pub(super) struct CustomWinner {
    pub value: CustomValue,
    pub location: Option<Arc<crate::style::SourceLocation>>,
}

pub(super) struct Customs {
    declarations: RefCell<CustomProperties>,
    selected: RefCell<HashMap<String, usize>>,
    pub names: Vec<Arc<str>>,
}
impl Customs {
    pub(super) fn new(mut declarations: CustomProperties) -> Self {
        for candidates in declarations.values_mut() {
            sort(candidates);
        }
        let mut names: Vec<_> = declarations.keys().cloned().collect();
        names.sort();
        Self {
            declarations: RefCell::new(declarations),
            selected: RefCell::default(),
            names,
        }
    }
    fn select(&self, name: &str) -> Result<Option<CustomWinner>, StyleError> {
        let declarations = self.declarations.borrow();
        let Some(candidates) = declarations.get(name) else {
            return Ok(None);
        };
        let winner = choose(
            candidates,
            |_, value, _| Ok(value.clone()),
            |value| WideKeyword::parse(&value.source),
        )?;
        Ok(winner.map(|(index, value)| {
            self.selected.borrow_mut().insert(name.to_owned(), index);
            CustomWinner {
                value,
                location: candidates[index].location.clone(),
            }
        }))
    }
    pub(super) fn current(&self, name: &str) -> Option<CustomWinner> {
        let index = self.selected.borrow().get(name).copied()?;
        let declarations = self.declarations.borrow();
        let candidate = &declarations.get(name)?[index];
        Some(CustomWinner {
            value: candidate.value.clone(),
            location: candidate.location.clone(),
        })
    }
    pub(super) fn sources(&self) -> Result<HashMap<String, String>, StyleError> {
        let mut sources = HashMap::new();
        for name in &self.names {
            if let Some(winner) = self.select(name)? {
                sources.insert(name.to_string(), winner.value.source.to_string());
            }
        }
        Ok(sources)
    }
    pub(super) fn recascade(
        &self,
        name: &str,
        keyword: WideKeyword,
    ) -> Result<Option<String>, StyleError> {
        if let Some(index) = self.selected.borrow().get(name).copied() {
            self.declarations
                .borrow_mut()
                .get_mut(name)
                .expect("selected custom property exists")[index]
                .value
                .source = keyword.name().into();
        }
        self.select(name)
            .map(|winner| winner.map(|winner| winner.value.source.to_string()))
    }
}

fn custom_properties(
    parent: &crate::style::variables::Environment,
    declarations: CustomProperties,
) -> Result<crate::style::variables::Environment, StyleError> {
    let declarations = Customs::new(declarations);
    loop {
        match crate::style::variables::resolve(parent, declarations.sources()?)? {
            crate::style::variables::Resolution::Complete(computed) => return Ok(computed),
            crate::style::variables::Resolution::Wide { name, keyword } => {
                declarations.recascade(&name, keyword)?;
            }
        }
    }
}

/// Invalid computed values become unset; only valid rollback keywords revisit candidates.
pub(super) fn resolve(
    values: Cascaded,
    style: &mut Style,
    parent: &Style,
) -> Result<Winners, StyleError> {
    resolve_located(values, style, parent, |_, _| {})
}

pub(super) fn resolve_located(
    values: Cascaded,
    style: &mut Style,
    parent: &Style,
    mut selected: impl FnMut(Property, Option<&crate::style::SourceLocation>),
) -> Result<Winners, StyleError> {
    let (declarations, custom) = values.into_parts();
    style.custom_properties = custom_properties(&style.custom_properties, custom)?;
    let flows = if declarations.flow_relative() {
        Some(element_flows(&declarations, style, parent)?)
    } else {
        None
    };
    let mut winners = Winners::new();
    for property in declarations.properties() {
        let winner = declarations.resolve_with(
            property,
            &mut || Ok(style.align),
            &mut || Ok(flows.expect("flow-relative candidates computed the element's flows")),
            &mut |pending, _| {
                declaration::substitute(
                    pending,
                    &style.custom_properties,
                    &style.inherited_custom_properties,
                )
            },
        )?;
        if let Some(winner) = winner {
            selected(property, winner.location.as_deref());
            if property == Property::FontFamily
                && matches!(winner.value, Specified::Value(_) | Specified::Toggle(_))
            {
                style.font_namespace = winner.namespace;
            }
            winners.insert(property, winner.value);
        }
    }
    Ok(winners)
}

/// CSS Logical 1 §4: the element's own direction computes before its flow-relative
/// declarations cascade with the physical ones.
fn element_flows(
    declarations: &Properties,
    style: &Style,
    parent: &Style,
) -> Result<Flows, StyleError> {
    let direction = declarations.resolve_with(
        Property::Direction,
        &mut || Ok(style.align),
        &mut || unreachable!("direction belongs to no logical property group"),
        &mut |pending, _| {
            declaration::substitute(
                pending,
                &style.custom_properties,
                &style.inherited_custom_properties,
            )
        },
    )?;
    let mut element = parent.clone();
    if let Some(direction) = direction {
        crate::style::properties::apply(
            Property::Direction,
            &direction.value,
            &mut element,
            parent,
            &crate::style::units::Relative::initial(),
        )?;
    }
    Ok(Flows {
        element: Flow::of(&element),
        parent: Flow::of(parent),
    })
}

type PropertyCandidate = Candidate<(Value, crate::fonts::FontNamespace)>;
type Expanded = Arc<[(Property, Specified)]>;
type Substituted = HashMap<*const Pending, Expanded>;
type Substitute<'a> = dyn FnMut(
        &Pending,
        crate::fonts::FontNamespace,
    ) -> Result<Option<Vec<(Property, Specified)>>, StyleError>
    + 'a;
type ParentAlignment<'a> = dyn FnMut() -> Result<crate::style::TextAlign, StyleError> + 'a;
type ElementFlows<'a> = dyn FnMut() -> Result<Flows, StyleError> + 'a;

pub(super) struct Winner {
    pub value: Specified,
    pub namespace: crate::fonts::FontNamespace,
    pub location: Option<Arc<crate::style::SourceLocation>>,
}

pub(super) struct Properties {
    candidates: BTreeMap<Property, Vec<PropertyCandidate>>,
    substituted: RefCell<Substituted>,
}

impl Properties {
    fn new(mut candidates: BTreeMap<Property, Vec<PropertyCandidate>>) -> Self {
        for values in candidates.values_mut() {
            sort(values);
        }
        Self {
            candidates,
            substituted: RefCell::default(),
        }
    }

    /// Every property with candidates; flow-relative ones compute into the physical members
    /// of their group.
    pub(super) fn properties(&self) -> impl Iterator<Item = Property> {
        let mut properties = BTreeSet::new();
        for property in self.candidates.keys() {
            match logical::physical_properties(*property) {
                Some(physical) => properties.extend(physical),
                None => {
                    properties.insert(*property);
                }
            }
        }
        properties.into_iter()
    }

    fn flow_relative(&self) -> bool {
        self.candidates
            .keys()
            .any(|property| logical::is_flow_relative(*property))
    }

    /// CSS Logical 1 §4: a physical property's candidates cascade together with those of the
    /// flow-relative property sharing its computed value. Among equal priorities the physical
    /// candidates come last, as shorthands covering both set physical longhands last.
    fn paired(
        &self,
        property: Property,
        flows: &mut ElementFlows<'_>,
    ) -> Result<Option<Paired>, StyleError> {
        if !logical::flow_relative_properties(property)
            .iter()
            .any(|logical| self.candidates.contains_key(logical))
        {
            return Ok(None);
        }
        let flows = flows()?;
        let Some((logical, flow_relative)) = logical::counterpart(property, flows.element)
            .and_then(|logical| Some((logical, self.candidates.get(&logical)?)))
        else {
            return Ok(None);
        };
        let mut candidates: Vec<(Property, PropertyCandidate)> = flow_relative
            .iter()
            .map(|candidate| (logical, candidate.clone()))
            .chain(
                self.candidates
                    .get(&property)
                    .into_iter()
                    .flatten()
                    .map(|candidate| (property, candidate.clone())),
            )
            .collect();
        candidates.sort_by_key(|(_, candidate)| candidate.priority());
        let (declared, candidates) = candidates.into_iter().unzip();
        Ok(Some(Paired {
            candidates,
            declared,
            flows,
        }))
    }

    pub(super) fn resolve_with(
        &self,
        property: Property,
        parent_alignment: &mut ParentAlignment<'_>,
        flows: &mut ElementFlows<'_>,
        substitute: &mut Substitute<'_>,
    ) -> Result<Option<Winner>, StyleError> {
        if logical::is_flow_relative(property) {
            return Ok(None);
        }
        let paired = self.paired(property, flows)?;
        let (candidates, declared) = match &paired {
            Some(paired) => (&paired.candidates[..], Some(&paired.declared[..])),
            None => match self.candidates.get(&property) {
                Some(candidates) => (&candidates[..], None),
                None => return Ok(None),
            },
        };
        let mut resolver = PropertyResolver {
            property,
            candidates,
            declared,
            substituted: &self.substituted,
            substitute,
            parent_alignment,
            memo: BTreeMap::new(),
            depth: 0,
            work: 0,
        };
        let Some((index, (value, namespace))) = choose_from(
            candidates,
            Rollback::default(),
            |index, _, rollback| resolver.value(index, rollback),
            |(value, _)| specified_keyword(value),
        )?
        else {
            return Ok(None);
        };
        let value = match &paired {
            Some(paired)
                if paired.declared[index] != property
                    && crate::style::resolution::needs_parent(property, &value)
                    && logical::physical(paired.declared[index], paired.flows.parent)
                        != property =>
            {
                Specified::FlowRelative(paired.flows.element, Arc::new(value))
            }
            _ => value,
        };
        Ok(Some(Winner {
            value,
            namespace,
            location: candidates[index].location.clone(),
        }))
    }
}

/// A physical property's candidates merged with its flow-relative counterpart's, with the
/// property each was declared for.
struct Paired {
    candidates: Vec<PropertyCandidate>,
    declared: Vec<Property>,
    flows: Flows,
}

struct PropertyResolver<'a, 's> {
    property: Property,
    candidates: &'a [PropertyCandidate],
    /// The property each candidate was declared for, when not all were declared for `property`.
    declared: Option<&'a [Property]>,
    substituted: &'a RefCell<Substituted>,
    substitute: &'a mut Substitute<'s>,
    parent_alignment: &'a mut ParentAlignment<'s>,
    memo: BTreeMap<(usize, Rollback), (Specified, crate::fonts::FontNamespace)>,
    depth: usize,
    work: usize,
}
impl PropertyResolver<'_, '_> {
    fn charge(&mut self) -> Result<(), StyleError> {
        self.work = self.work.saturating_add(1);
        if self.work > crate::style::matcher::MAX_MATCH_STEPS {
            return Err(StyleError::GrammarLimit {
                resource: "cascade value computation",
                limit: crate::style::matcher::MAX_MATCH_STEPS,
            });
        }
        Ok(())
    }
    fn value(
        &mut self,
        index: usize,
        rollback: &Rollback,
    ) -> Result<Option<(Specified, crate::fonts::FontNamespace)>, StyleError> {
        self.charge()?;
        let key = (index, rollback.clone());
        if let Some(value) = self.memo.get(&key) {
            return Ok(Some(value.clone()));
        }
        let (value, namespace) = &self.candidates[index].value;
        let namespace = *namespace;
        let value = match value {
            Value::Specified(value) => value.clone(),
            Value::HeaderAlignment(value) => {
                if (self.parent_alignment)()? != crate::style::TextAlign::Start {
                    return Ok(None);
                }
                value.clone()
            }
            Value::Pending(pending) => {
                let key = Rc::as_ptr(pending);
                let cached = self.substituted.borrow().get(&key).cloned();
                let expanded = if let Some(expanded) = cached {
                    expanded
                } else {
                    let expanded: Expanded = (self.substitute)(pending, namespace)?
                        .unwrap_or_default()
                        .into();
                    self.substituted.borrow_mut().insert(key, expanded.clone());
                    expanded
                };
                let declared = self
                    .declared
                    .map_or(self.property, |declared| declared[index]);
                expanded
                    .iter()
                    .find(|(property, _)| *property == declared)
                    .map_or(Specified::Wide(WideKeyword::Unset), |(_, value)| {
                        value.clone()
                    })
            }
        };
        let Specified::Toggle(values) = value else {
            return Ok(Some((value, namespace)));
        };
        if self.depth >= crate::style::parser::MAX_COMPONENT_DEPTH {
            return Err(StyleError::GrammarLimit {
                resource: "cascade value nesting",
                limit: crate::style::parser::MAX_COMPONENT_DEPTH,
            });
        }
        self.depth += 1;
        let result = (|| {
            let mut resolved = Vec::with_capacity(values.len());
            for argument in values.iter() {
                self.charge()?;
                let (value, owner) = match argument.value {
                    Specified::Wide(keyword @ (WideKeyword::Revert | WideKeyword::RevertLayer)) => {
                        self.rollback(index, keyword, rollback)?
                    }
                    _ => (argument.value.clone(), namespace),
                };
                resolved.push(declaration::ToggleArgument {
                    value,
                    font_namespace: (self.property == Property::FontFamily).then_some(owner),
                    location: self.candidates[index].location.clone(),
                });
            }
            Ok((Specified::Toggle(resolved.into()), namespace))
        })();
        self.depth -= 1;
        if let Ok(value) = &result {
            self.memo.insert(key, value.clone());
        }
        result.map(Some)
    }
    fn rollback(
        &mut self,
        index: usize,
        keyword: WideKeyword,
        inherited: &Rollback,
    ) -> Result<(Specified, crate::fonts::FontNamespace), StyleError> {
        let candidates = self.candidates;
        let namespace = candidates[index].value.1;
        let mut rollback = inherited.clone();
        rollback.apply(&candidates[index], Some(keyword));
        Ok(choose_from(
            &candidates[..index],
            rollback,
            |current, _, rollback| self.value(current, rollback),
            |(value, _)| specified_keyword(value),
        )?
        .map_or(
            (Specified::Wide(WideKeyword::Unset), namespace),
            |(_, value)| value,
        ))
    }
}
