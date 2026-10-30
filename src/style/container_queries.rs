//! CSS Conditional 5 §5–7: query containers, the `@container` rule and container query
//! length units.
//!
//! A container's size comes from layout. Each styling pass reads the sizes the previous
//! layout pass measured and records the ones it consults; the document is restyled and laid
//! out again until a layout reproduces every consulted size, so each container's descendants
//! are styled with the size of the container they query.
use crate::dom::{Element, ElementId};
use crate::resources::ResourceResolver;
use crate::style::{
    Style, StyleError, Stylesheet,
    conditional::{QueryValues, Scalar, StyleQuery},
    conditions::{Condition, Truth, block_source},
    declaration::Specified,
    grammar::Property,
    media::Feature,
    properties,
    selector::Namespaces,
    units::{Axis, Relative},
    variables,
};
use cssparser::{Parser, ParserInput, Token};
use std::{cell::RefCell, collections::HashMap, sync::Arc};

const STYLE_FUNCTION: &str = "style";
const SCROLL_STATE_FUNCTION: &str = "scroll-state";
/// CSS Conditional 5 §6.3.2: the edges `stuck` takes.
const EDGES: [&str; 8] = [
    "top",
    "right",
    "bottom",
    "left",
    "block-start",
    "inline-start",
    "block-end",
    "inline-end",
];
/// CSS Conditional 5 §6.3.4–6.3.5: the axes `scrollable` and `scrolled` also take.
const SCROLL_AXES: [&str; 4] = ["x", "y", "block", "inline"];
/// CSS Conditional 5 §6.3.3: the axes `snapped` takes.
const SNAP_AXES: [&str; 5] = ["x", "y", "block", "inline", "both"];
const NONE: &str = "none";
/// CSS Conditional 5 §5.4: names a `<container-name>` cannot take.
const RESERVED_NAMES: [&str; 4] = ["none", "and", "not", "or"];

/// An element that answers container queries for its descendants.
#[derive(Debug, PartialEq)]
pub(crate) struct QueryContainer {
    element: ElementId,
    /// The computed style, whose own containers are the element's ancestors.
    style: Style,
}

/// The nearest query container among an element's inclusive ancestors, which links to the
/// containers above it.
#[derive(Clone, Default)]
pub(crate) struct QueryContainers(Option<Arc<QueryContainer>>);
/// The containers an element queries are its context, not a computed value, so styles
/// compare equal whatever containers they were computed among.
impl PartialEq for QueryContainers {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl std::fmt::Debug for QueryContainers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("QueryContainers")
    }
}
impl QueryContainers {
    fn iter(&self) -> impl Iterator<Item = &QueryContainer> {
        std::iter::successors(self.0.as_deref(), |container| {
            container.style.containers.0.as_deref()
        })
    }
    /// Container query length units' inline and block basis: the content-box size of the
    /// nearest container answering size queries in each axis (CSS Conditional 5 §7).
    pub(super) fn unit_basis(&self, sizes: &QuerySizes, axis: Axis) -> Option<f32> {
        let index = usize::from(axis == Axis::Height);
        self.iter()
            .find(|container| {
                let block = (index == 1) != container.style.writing_mode.vertical();
                container.answers_axes([!block, block])
            })
            .and_then(|container| sizes.size(container.element))
            .map(|size| size[index])
    }
    pub(super) fn logical_unit_basis(&self, sizes: &QuerySizes, block: bool) -> Option<f32> {
        let container = self
            .iter()
            .find(|container| container.answers_axes([!block, block]))?;
        let index = usize::from(block != container.style.writing_mode.vertical());
        sizes.size(container.element).map(|size| size[index])
    }
}

impl QueryContainer {
    /// Size containment answers size queries in the axes `container-type` contains, unless
    /// the box is one that containment has no effect on.
    fn answers_axes(&self, [inline, block]: [bool; 2]) -> bool {
        let container_type = self.style.containment.container_type;
        self.style.containment().query_container
            && (!inline || container_type.size || container_type.inline_size)
            && (!block || container_type.size)
    }
}

/// Query containers' unzoomed content-box sizes, measured by layout and consulted by styling.
#[derive(Clone, Default)]
pub(crate) struct QuerySizes {
    /// Sizes the previous layout pass measured.
    known: HashMap<ElementId, [f32; 2]>,
    /// Sizes this styling pass consulted; `None` for a container not yet measured.
    consulted: RefCell<HashMap<ElementId, Option<[f32; 2]>>>,
}
impl QuerySizes {
    fn size(&self, element: ElementId) -> Option<[f32; 2]> {
        let size = self.known.get(&element).copied();
        self.consulted.borrow_mut().insert(element, size);
        size
    }
    /// Whether a layout that measured `measured` reproduces every size the styles consulted.
    pub(crate) fn settled(&self, measured: &HashMap<ElementId, [f32; 2]>) -> bool {
        self.consulted
            .borrow()
            .iter()
            .all(|(element, size)| measured.get(element) == size.as_ref())
    }
    /// Restyling starts from the sizes a layout measured.
    pub(crate) fn measured(&mut self, measured: HashMap<ElementId, [f32; 2]>) {
        self.known = measured;
        self.consulted.get_mut().clear();
    }
}

/// `@container <container-condition>#`: true if any condition is. The rule's namespaces
/// resolve `attr()` prefixes and `selector()` tests its features substitute.
#[derive(Clone)]
pub(super) struct ContainerQuery {
    conditions: Vec<ContainerCondition>,
    namespaces: Namespaces,
}

#[derive(Clone)]
struct ContainerCondition {
    name: Option<Box<str>>,
    query: Option<Condition<Test>>,
}

#[derive(Clone)]
enum Test {
    Size(Feature),
    /// A size feature with substitution functions, substituted from the container.
    Substituted(Box<str>),
    Style(StyleQuery),
    ScrollState(Condition<ScrollStateFeature>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScrollState {
    Stuck,
    Snapped,
    Scrollable,
    Scrolled,
}

/// A `<scroll-state-feature>`: a name and, outside a boolean context, one of its keywords.
#[derive(Clone)]
struct ScrollStateFeature {
    state: ScrollState,
    none: Option<bool>,
}
impl ScrollStateFeature {
    fn parse(source: &str) -> Option<Self> {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        let name = input.expect_ident_cloned().ok()?;
        let (state, values): (_, &[&[&str]]) = match name.to_ascii_lowercase().as_str() {
            "stuck" => (ScrollState::Stuck, &[&EDGES]),
            "snapped" => (ScrollState::Snapped, &[&SNAP_AXES]),
            "scrollable" => (ScrollState::Scrollable, &[&EDGES, &SCROLL_AXES]),
            "scrolled" => (ScrollState::Scrolled, &[&EDGES, &SCROLL_AXES]),
            _ => return None,
        };
        if input.is_exhausted() {
            return Some(Self { state, none: None });
        }
        input.expect_colon().ok()?;
        let value = input.expect_ident_cloned().ok()?;
        input.expect_exhausted().ok()?;
        let none = value.eq_ignore_ascii_case(NONE);
        (none
            || values
                .iter()
                .flat_map(|values| values.iter())
                .any(|known| value.eq_ignore_ascii_case(known)))
        .then_some(Self {
            state,
            none: Some(none),
        })
    }

    /// Static media never scroll (CSS Overflow 3 §3.1.3): no scroll container has overflow
    /// a user can reach by scrolling or has been scrolled, and nothing snaps, so only `none`
    /// holds, and only for the boxes each feature describes. A boolean context is true for
    /// any value but `none`.
    fn evaluate(&self, style: &Style) -> Result<Truth, StyleError> {
        let described = match self.state {
            ScrollState::Scrollable | ScrollState::Scrolled => style.is_scroll_container(),
            ScrollState::Snapped => true,
            ScrollState::Stuck if style.position == crate::style::Position::Sticky => {
                return Err(StyleError::Unsupported(
                    "@container scroll-state(stuck) of a sticky query container".into(),
                ));
            }
            ScrollState::Stuck => false,
        };
        Ok((described && self.none == Some(true)).into())
    }
}

impl ContainerQuery {
    pub(super) fn parse(source: &str, namespaces: Namespaces) -> Result<Self, StyleError> {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        let conditions = input
            .parse_comma_separated(|input| {
                ContainerCondition::parse(block_source(input))
                    .map_err(|error| input.new_custom_error(error))
            })
            .map_err(crate::style::parser::failure)?;
        Ok(Self {
            conditions,
            namespaces,
        })
    }

    /// Whether the query holds for `element`, whose inclusive-ancestor containers start at
    /// `ancestors`.
    pub(super) fn matches(
        &self,
        element: Element<'_>,
        ancestors: &QueryContainers,
        sheet: &Stylesheet,
        resolver: &dyn ResourceResolver,
    ) -> Result<bool, StyleError> {
        for condition in &self.conditions {
            let values = |container| ContainerValues {
                container,
                element: element
                    .tree
                    .element(container.element)
                    .expect("a query container is an element of the queried element's tree"),
                namespaces: &self.namespaces,
                sheet,
                resolver,
            };
            if condition.evaluate(ancestors, values)? == Truth::True {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl ContainerCondition {
    fn parse(source: &str) -> Result<Self, StyleError> {
        let invalid = || StyleError::Invalid(format!("invalid container condition: {source}"));
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        let start = input.state();
        let name = match input.next() {
            Ok(Token::Ident(name)) if !name.eq_ignore_ascii_case("not") => {
                if RESERVED_NAMES
                    .iter()
                    .any(|reserved| name.eq_ignore_ascii_case(reserved))
                {
                    return Err(invalid());
                }
                Some(Box::from(name.as_ref()))
            }
            Ok(_) => {
                input.reset(&start);
                None
            }
            Err(_) => return Err(invalid()),
        };
        let rest = block_source(input);
        let query = if crate::style::parser::trim_value(rest).is_empty() {
            None
        } else {
            Some(
                Condition::parse_fallible(rest, true, Truth::Unknown, Test::parse)?
                    .ok_or_else(invalid)?,
            )
        };
        if name.is_none() && query.is_none() {
            return Err(invalid());
        }
        Ok(Self { name, query })
    }

    /// The condition against the nearest ancestor container with the name that answers
    /// every feature it queries; unknown without one (CSS Conditional 5 §5.4). Features
    /// that substitute custom properties take them from each candidate container.
    fn evaluate<'a>(
        &self,
        ancestors: &'a QueryContainers,
        values: impl Fn(&'a QueryContainer) -> ContainerValues<'a>,
    ) -> Result<Truth, StyleError> {
        for container in ancestors.iter() {
            if self.name.as_deref().is_some_and(|name| {
                !container
                    .style
                    .containment
                    .container_names
                    .iter()
                    .any(|candidate| candidate.as_ref() == name)
            }) {
                continue;
            }
            let Some(query) = &self.query else {
                return Ok(Truth::True);
            };
            let values = values(container);
            let mut axes = [false; 2];
            let mut scroll_state = false;
            for test in query.leaves() {
                let feature = match test {
                    Test::Size(feature) => Some(feature.clone()),
                    Test::Substituted(source) => values.substituted_feature(source)?,
                    Test::Style(_) => continue,
                    Test::ScrollState(_) => {
                        scroll_state = true;
                        continue;
                    }
                };
                let Some(needed) = feature.as_ref().and_then(|feature| {
                    feature.container_axes(container.style.writing_mode.vertical())
                }) else {
                    return Ok(Truth::Unknown);
                };
                axes = [axes[0] || needed[0], axes[1] || needed[1]];
            }
            if (axes.contains(&true) && !container.answers_axes(axes))
                || (scroll_state && !container.style.containment.container_type.scroll_state)
            {
                continue;
            }
            return query.try_evaluate(|test| test.evaluate(&values));
        }
        Ok(Truth::Unknown)
    }
}

impl Test {
    fn parse(source: &str) -> Result<Option<Self>, StyleError> {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        if let Ok(Token::Function(name)) = input.next().cloned()
            && let Ok(inner) = input.parse_nested_block(|nested| {
                Ok::<_, cssparser::ParseError<'_, ()>>(block_source(nested))
            })
            && input.is_exhausted()
        {
            if name.eq_ignore_ascii_case(STYLE_FUNCTION) {
                return Ok(StyleQuery::parse_general(inner, Truth::Unknown)?.map(Self::Style));
            }
            if name.eq_ignore_ascii_case(SCROLL_STATE_FUNCTION) {
                let query = match ScrollStateFeature::parse(inner) {
                    Some(feature) => Some(Condition::value(feature)),
                    None => {
                        Condition::parse(inner, true, Truth::Unknown, ScrollStateFeature::parse)?
                    }
                };
                return Ok(query.map(Self::ScrollState));
            }
        }
        if let Some(feature) = Feature::parse(source) {
            return Ok(Some(Self::Size(feature)));
        }
        if matches!(variables::Value::with_substitutions(source), Ok(Some(_)))
            && Condition::parse(source, true, Truth::Unknown, |_| None::<()>)?.is_none()
        {
            return Ok(Some(Self::Substituted(source.into())));
        }
        Ok(None)
    }

    fn evaluate(&self, values: &ContainerValues<'_>) -> Result<Truth, StyleError> {
        match self {
            Self::Size(feature) => Ok(values.size_feature(feature)),
            Self::Substituted(source) => match values.substituted_feature(source)? {
                Some(feature) => Ok(values.size_feature(&feature)),
                None => Ok(Truth::Unknown),
            },
            Self::Style(query) => query.evaluate(values),
            Self::ScrollState(query) => {
                query.try_evaluate(|feature| feature.evaluate(&values.container.style))
            }
        }
    }
}

/// A query container's computed values and attributes, against which its queries evaluate.
struct ContainerValues<'a> {
    container: &'a QueryContainer,
    element: Element<'a>,
    namespaces: &'a Namespaces,
    sheet: &'a Stylesheet,
    resolver: &'a dyn ResourceResolver,
}
impl ContainerValues<'_> {
    /// The container's own relative lengths (CSS Conditional 5 §6.1).
    fn relative(&self) -> Relative<'_> {
        self.sheet
            .inherited(&self.container.style, self.sheet.page_area(), self.resolver)
    }
    fn parent(&self) -> Style {
        self.container
            .style
            .containers
            .0
            .as_ref()
            .map_or_else(Style::default, |parent| parent.style.clone())
    }
    /// A size feature after the container's substitutions.
    fn substituted_feature(&self, source: &str) -> Result<Option<Feature>, StyleError> {
        Ok(self
            .substitute(source, false)?
            .and_then(|expanded| Feature::parse(expanded.as_str())))
    }
    fn size_feature(&self, feature: &Feature) -> Truth {
        let Some(size) = self.sheet.query_sizes.size(self.container.element) else {
            return Truth::Unknown;
        };
        feature.evaluate_container(
            size,
            self.container.style.writing_mode.vertical(),
            &self.relative(),
        )
    }
}
impl ContainerValues<'_> {
    /// Substitutes the container's custom properties and attributes, and evaluates `if()`
    /// tests against the container (CSS Values 5 §6–7).
    fn expand(&self, value: &variables::Value) -> Result<Option<variables::Expanded>, StyleError> {
        let style = &self.container.style;
        value.substitute_context(
            &mut |reference, name| {
                let source = match reference {
                    variables::Reference::Local => &style.custom_properties,
                    variables::Reference::Inherited => &style.inherited_custom_properties,
                };
                Ok(source.get(name).and_then(Clone::clone))
            },
            &mut |test| test.evaluate(&self.sheet.environment, Some(self.namespaces), self),
            &mut |attribute| attribute.resolve(self),
        )
    }
}
impl crate::style::attribute::Context for ContainerValues<'_> {
    fn attribute_data(&self, prefix: Option<&str>, name: &str) -> Option<&str> {
        let namespace = match prefix {
            Some(prefix) => Some(self.namespaces.borrow().get(prefix)?.clone()),
            None => None,
        };
        self.element.attr_in_namespace(namespace.as_deref(), name)
    }
    fn expand_attribute_value(
        &self,
        value: &variables::Value,
    ) -> Result<Option<variables::Expanded>, StyleError> {
        self.expand(value)
    }
    fn matches_attribute_syntax(
        &self,
        syntax: &crate::style::syntax::Syntax,
        value: &variables::Expanded,
    ) -> Result<bool, StyleError> {
        let relative = self.relative();
        let mut steps = 0;
        let result = crate::style::matcher::match_syntax(
            syntax,
            value.as_str(),
            &mut steps,
            crate::style::matcher::ValueContext::Nested(&relative),
        );
        relative.work.charge(steps)?;
        result
    }
}
impl QueryValues for ContainerValues<'_> {
    fn property(&self, _: Property) -> Result<Style, StyleError> {
        Ok(self.container.style.clone())
    }
    fn candidate(&self, property: Property, value: &Specified) -> Result<Style, StyleError> {
        let parent = self.parent();
        let mut candidate = self.container.style.clone();
        // `font-size` resolves font-relative lengths against the parent's font.
        let relative = if property == Property::FontSize {
            self.sheet
                .inherited(&parent, self.sheet.page_area(), self.resolver)
        } else {
            self.relative()
        };
        properties::apply(property, value, &mut candidate, &parent, &relative)?;
        Ok(candidate)
    }
    fn custom(&self, name: &str) -> Result<Option<variables::Expanded>, StyleError> {
        Ok(self
            .container
            .style
            .custom_properties
            .get(name)
            .cloned()
            .flatten())
    }
    fn inherited_custom(&self, name: &str) -> Result<Option<variables::Expanded>, StyleError> {
        Ok(self
            .container
            .style
            .inherited_custom_properties
            .get(name)
            .cloned()
            .flatten())
    }
    fn substitute(
        &self,
        source: &str,
        property_syntax: bool,
    ) -> Result<Option<variables::Expanded>, StyleError> {
        match variables::Value::parse_context(source, 0, property_syntax)? {
            Some(value) => self.expand(&value),
            None => Ok(None),
        }
    }
    fn scalar(&self, source: &str) -> Result<Option<Scalar>, StyleError> {
        Scalar::parse(source, &self.relative())
    }
}

impl Stylesheet {
    /// Makes `element`, whose computed style is `style`, the nearest query container of its
    /// descendants. Every element answers style queries; without `@container` rules only
    /// size containers are recorded, which container query length units measure.
    pub(super) fn record_container(&self, element: ElementId, style: &mut Style) {
        let containment = &style.containment;
        if !self.container_rules.is_empty()
            || containment.container_type != crate::style::ContainerType::default()
            || !containment.container_names.is_empty()
        {
            let snapshot = style.clone();
            style.containers = QueryContainers(Some(Arc::new(QueryContainer {
                element,
                style: snapshot,
            })));
        }
    }

    /// Whether `@container` rule `index` and the rules enclosing it hold for `element`,
    /// whose inclusive-ancestor containers start at `ancestors`.
    pub(super) fn container_rule_matches(
        &self,
        index: usize,
        element: Element<'_>,
        ancestors: &QueryContainers,
        resolver: &dyn ResourceResolver,
        cache: &mut HashMap<usize, bool>,
    ) -> Result<bool, StyleError> {
        let mut next = Some(index);
        while let Some(index) = next {
            let matching = if let Some(matching) = cache.get(&index) {
                *matching
            } else {
                let matching = self.container_rules[index]
                    .query
                    .matches(element, ancestors, self, resolver)?;
                cache.insert(index, matching);
                matching
            };
            if !matching {
                return Ok(false);
            }
            next = self.container_rules[index].parent;
        }
        Ok(true)
    }

    /// The sizes this styling consulted, which a layout must reproduce.
    pub(crate) fn query_sizes(&self) -> &QuerySizes {
        &self.query_sizes
    }
    pub(crate) fn query_sizes_mut(&mut self) -> &mut QuerySizes {
        &mut self.query_sizes
    }
}

/// An `@container` rule and the one enclosing it.
#[derive(Clone)]
pub(super) struct ContainerRule {
    pub parent: Option<usize>,
    pub query: ContainerQuery,
}
