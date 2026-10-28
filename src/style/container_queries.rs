//! CSS Conditional 5 §5–7: query containers, the `@container` rule and container query
//! length units.
//!
//! A container's size comes from layout. Each styling pass reads the sizes the previous
//! layout pass measured and records the ones it consults; the document is restyled and laid
//! out again until a layout reproduces every consulted size, so each container's descendants
//! are styled with the size of the container they query.
use crate::dom::ElementId;
use crate::resources::ResourceResolver;
use crate::style::{
    Style, StyleError, Stylesheet,
    conditional::{QueryValues, Scalar, StyleQuery},
    conditions::{Condition, Truth, block_source},
    declaration::Specified,
    grammar::Property,
    media::Feature,
    properties,
    units::Relative,
    variables,
};
use cssparser::{Parser, ParserInput, Token};
use std::{cell::RefCell, collections::HashMap, sync::Arc};

const STYLE_FUNCTION: &str = "style";
const SCROLL_STATE_FUNCTION: &str = "scroll-state";
const VARIABLE_FUNCTION: &str = "var";
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
    pub(super) fn unit_basis(&self, sizes: &QuerySizes) -> [Option<f32>; 2] {
        [0, 1].map(|axis| {
            self.iter()
                .find(|container| container.answers_axes([axis == 0, axis == 1]))
                .and_then(|container| sizes.size(container.element))
                .map(|size| size[axis])
        })
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

/// Query containers' content-box sizes, measured by layout and consulted by styling.
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

/// `@container <container-condition>#`: true if any condition is.
#[derive(Clone)]
pub(super) struct ContainerQuery(Vec<ContainerCondition>);

#[derive(Clone)]
struct ContainerCondition {
    name: Option<Box<str>>,
    query: Option<Condition<Test>>,
}

#[derive(Clone)]
enum Test {
    Size(Feature),
    /// A size feature with custom properties, substituted from the container's values.
    Substituted(Box<str>),
    Style(StyleQuery),
}

impl ContainerQuery {
    pub(super) fn parse(source: &str) -> Result<Self, StyleError> {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        let conditions = input
            .parse_comma_separated(|input| {
                ContainerCondition::parse(block_source(input))
                    .map_err(|error| input.new_custom_error(error))
            })
            .map_err(crate::style::parser::failure)?;
        Ok(Self(conditions))
    }

    /// Whether the query holds for an element whose inclusive-ancestor containers start at
    /// `ancestors`.
    pub(super) fn matches(
        &self,
        ancestors: &QueryContainers,
        sheet: &Stylesheet,
        resolver: &dyn ResourceResolver,
    ) -> Result<bool, StyleError> {
        for condition in &self.0 {
            if condition.evaluate(ancestors, sheet, resolver)? == Truth::True {
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
    fn evaluate(
        &self,
        ancestors: &QueryContainers,
        sheet: &Stylesheet,
        resolver: &dyn ResourceResolver,
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
            let values = ContainerValues {
                container,
                sheet,
                resolver,
            };
            let mut axes = [false; 2];
            for test in query.leaves() {
                let feature = match test {
                    Test::Size(feature) => Some(feature.clone()),
                    Test::Substituted(source) => values.substituted_feature(source)?,
                    Test::Style(_) => continue,
                };
                let Some(needed) = feature.as_ref().and_then(Feature::container_axes) else {
                    return Ok(Truth::Unknown);
                };
                axes = [axes[0] || needed[0], axes[1] || needed[1]];
            }
            if axes.contains(&true) && !container.answers_axes(axes) {
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
                return Err(StyleError::Unsupported(format!(
                    "@container {SCROLL_STATE_FUNCTION}({inner})"
                )));
            }
        }
        if let Some(feature) = Feature::parse(source) {
            return Ok(Some(Self::Size(feature)));
        }
        if uses_variables(source)
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
        }
    }
}

/// Whether `source` substitutes `var()` at any depth.
fn uses_variables(source: &str) -> bool {
    let mut pending = vec![source];
    while let Some(source) = pending.pop() {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        while let Ok(token) = input.next().cloned() {
            if let Token::Function(name) = &token
                && name.eq_ignore_ascii_case(VARIABLE_FUNCTION)
            {
                return true;
            }
            if matches!(
                token,
                Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock
            ) && let Ok(inner) = input.parse_nested_block(|nested| {
                Ok::<_, cssparser::ParseError<'_, ()>>(block_source(nested))
            }) {
                pending.push(inner);
            }
        }
    }
    false
}

/// A query container's computed values, against which its queries evaluate.
struct ContainerValues<'a> {
    container: &'a QueryContainer,
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
    /// A size feature after substituting the container's custom properties.
    fn substituted_feature(&self, source: &str) -> Result<Option<Feature>, StyleError> {
        Ok(self
            .substitute(source, false)?
            .and_then(|expanded| Feature::parse(expanded.as_str())))
    }
    fn size_feature(&self, feature: &Feature) -> Truth {
        let Some(size) = self.sheet.query_sizes.size(self.container.element) else {
            return Truth::Unknown;
        };
        feature.evaluate_container(size, &self.relative())
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
        let Some(value) = variables::Value::parse_context(source, 0, property_syntax)? else {
            return Ok(None);
        };
        let style = &self.container.style;
        value.substitute(&style.custom_properties, &style.inherited_custom_properties)
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

    /// Whether `@container` rule `index` and the rules enclosing it hold for an element
    /// whose inclusive-ancestor containers start at `ancestors`.
    pub(super) fn container_rule_matches(
        &self,
        index: usize,
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
                    .matches(ancestors, self, resolver)?;
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
