use crate::style::{
    Style, StyleError, calc,
    conditions::{Condition, Truth, block_source},
    declaration::{self, Specified},
    grammar::Property,
    media::{Comparison, MediaQuery, comparison, matches_media},
    parser::{failure, substitution_argument},
    print::PrintEnvironment,
    properties,
    selector::Namespaces,
    supports::SupportsCondition,
    units::{self, Dimension, Relative},
    variables::{self, Value},
    wide_keywords::WideKeyword,
};
use cssparser::{Delimiter, Parser, ParserInput, Token};
use std::collections::HashSet;

pub(super) const FUNCTION: &str = "if";

#[derive(Clone)]
pub(super) struct Expression {
    branches: Vec<(Condition<Test>, Value)>,
    dependencies: HashSet<String>,
}

impl std::fmt::Debug for Expression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conditional")
            .field("branches", &self.branches.len())
            .field("dependencies", &self.dependencies)
            .finish()
    }
}

impl Expression {
    pub(super) fn parse(
        input: &mut Parser<'_, '_>,
        depth: usize,
        property_syntax: bool,
    ) -> Result<Option<Self>, StyleError> {
        let mut branches = Vec::new();
        let mut dependencies = HashSet::new();
        let parsed = input.parse_entirely(|input| {
            while !input.is_exhausted() {
                let start = input.position();
                let source = loop {
                    let before = input.state();
                    match input.next()?.clone() {
                        Token::Colon => break input.slice(start..before.position()),
                        Token::Function(_)
                        | Token::ParenthesisBlock
                        | Token::SquareBracketBlock
                        | Token::CurlyBracketBlock => {
                            input.parse_nested_block(|nested| {
                                Ok::<_, cssparser::ParseError<'_, StyleError>>(block_source(nested))
                            })?;
                        }
                        Token::Semicolon => {
                            return Err(input.new_custom_error(StyleError::Invalid(
                                "if() condition has no colon".into(),
                            )));
                        }
                        _ => {}
                    }
                };
                let mut condition_input = ParserInput::new(source);
                let condition_parser = &mut Parser::new(&mut condition_input);
                let condition = if condition_parser
                    .try_parse(|input| {
                        input.expect_ident_matching("else")?;
                        input.expect_exhausted()
                    })
                    .is_ok()
                {
                    Condition::constant(Truth::True)
                } else {
                    let Some(condition) =
                        Condition::parse_fallible(source, true, Truth::Unknown, Test::parse)
                            .map_err(|error| input.new_custom_error(error))?
                    else {
                        return Err(input.new_custom_error(StyleError::Invalid(
                            "invalid if() condition".into(),
                        )));
                    };
                    condition
                };
                variables::dependencies_in(source, depth, &mut dependencies)
                    .map_err(|error| input.new_custom_error(error))?;
                let source = input.parse_until_before(Delimiter::Semicolon, |input| {
                    substitution_argument(input, false)
                })?;
                let Some(value) = Value::parse_context(source, depth, property_syntax)
                    .map_err(|error| input.new_custom_error(error))?
                else {
                    return Err(
                        input.new_custom_error(StyleError::Invalid("invalid if() branch".into()))
                    );
                };
                branches.push((condition, value));
                if !input.is_exhausted() {
                    input.expect_semicolon()?;
                }
            }
            if branches.is_empty() {
                return Err(input.new_custom_error(StyleError::Invalid("empty if()".into())));
            }
            Ok(())
        });
        match parsed.map_err(failure) {
            Ok(()) => Ok(Some(Self {
                branches,
                dependencies,
            })),
            Err(StyleError::Invalid(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(super) fn select(
        &self,
        test: &mut impl FnMut(&Test) -> Result<Truth, StyleError>,
    ) -> Result<Option<&Value>, StyleError> {
        for (condition, value) in &self.branches {
            if condition.try_evaluate(&mut *test)? == Truth::True {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    pub(super) fn dependencies(&self, output: &mut HashSet<String>) {
        output.extend(self.dependencies.iter().cloned());
        for (_, value) in &self.branches {
            value.dependencies(output);
        }
    }
}

#[derive(Clone)]
pub(super) enum Test {
    Media(MediaQuery),
    Supports(SupportsCondition),
    Style(StyleQuery),
}

impl Test {
    fn parse(source: &str) -> Result<Option<Self>, StyleError> {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        let Ok(Token::Function(name)) = input.next().cloned() else {
            return Ok(None);
        };
        let result = input
            .parse_nested_block(|input| {
                let source = block_source(input);
                let test = if name.eq_ignore_ascii_case("media") {
                    Some(Self::Media(
                        MediaQuery::parse(source).map_err(|error| input.new_custom_error(error))?,
                    ))
                } else if name.eq_ignore_ascii_case("supports") {
                    let direct = crate::style::supports::SupportTest::declaration(source).is_some();
                    let source = if direct {
                        format!("({source})")
                    } else {
                        source.to_owned()
                    };
                    SupportsCondition::parse_with_namespaces(&source, &Namespaces::default())
                        .map_err(|error| input.new_custom_error(error))?
                        .map(Self::Supports)
                } else if name.eq_ignore_ascii_case("style") {
                    StyleQuery::parse(source)
                        .map_err(|error| input.new_custom_error(error))?
                        .map(Self::Style)
                } else {
                    None
                };
                Ok::<_, cssparser::ParseError<'_, StyleError>>(test)
            })
            .map_err(failure)?;
        if input.expect_exhausted().is_err() {
            return Ok(None);
        }
        Ok(result)
    }

    pub(super) fn evaluate(
        &self,
        environment: &PrintEnvironment,
        namespaces: Option<&Namespaces>,
        values: &impl QueryValues,
    ) -> Result<Truth, StyleError> {
        Ok(match self {
            Self::Media(query) => matches_media(query, environment).into(),
            Self::Supports(query) => query.matches_with(&environment.supports, namespaces).into(),
            Self::Style(query) => query.evaluate(values)?,
        })
    }
}

pub(super) trait QueryValues {
    fn property(&self, property: Property) -> Result<Style, StyleError>;
    fn candidate(&self, property: Property, value: &Specified) -> Result<Style, StyleError>;
    fn custom(&self, name: &str) -> Result<Option<variables::Expanded>, StyleError>;
    fn inherited_custom(&self, name: &str) -> Result<Option<variables::Expanded>, StyleError>;
    fn substitute(
        &self,
        value: &str,
        property_syntax: bool,
    ) -> Result<Option<variables::Expanded>, StyleError>;
    fn scalar(&self, value: &str) -> Result<Option<Scalar>, StyleError>;
}

#[derive(Clone)]
enum Name {
    Property(Property),
    Custom(String),
}
impl Name {
    fn parse(name: &str) -> Option<Self> {
        if name.starts_with(declaration::CUSTOM_PROPERTY_PREFIX) {
            return (name.len() > declaration::CUSTOM_PROPERTY_PREFIX.len())
                .then(|| Self::Custom(name.to_owned()));
        }
        let property = Property::from_name(name)?;
        declaration::longhands(property)
            .any(properties::supports_property)
            .then_some(Self::Property(property))
    }
}

#[derive(Clone)]
enum Feature {
    Boolean(Name),
    Plain(Name, String),
    Range(Vec<String>, Vec<Comparison>),
}

#[derive(Clone)]
pub(super) struct StyleQuery(Condition<Feature>);
impl StyleQuery {
    fn parse(source: &str) -> Result<Option<Self>, StyleError> {
        if let Some(feature) = Feature::parse(source) {
            return Ok(Some(Self(Condition::value(feature))));
        }
        Ok(Condition::parse(source, true, Truth::False, Feature::parse)?.map(Self))
    }
    fn evaluate(&self, values: &impl QueryValues) -> Result<Truth, StyleError> {
        self.0
            .try_evaluate(|feature| feature.evaluate(values).map(Truth::from))
    }
}

impl Feature {
    fn parse(source: &str) -> Option<Self> {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        if let Ok(name) = input.try_parse(Parser::expect_ident_cloned)
            && let Some(name) = Name::parse(&name)
        {
            if input.is_exhausted() {
                return Some(Self::Boolean(name));
            }
            if input.try_parse(Parser::expect_colon).is_ok() {
                let value = block_source(input);
                if split_range(value).is_some() {
                    return None;
                }
                return Some(Self::Plain(name, value.to_owned()));
            }
        }
        let (values, comparisons) = split_range(source)?;
        Some(Self::Range(values, comparisons))
    }

    fn evaluate(&self, values: &impl QueryValues) -> Result<bool, StyleError> {
        match self {
            Self::Boolean(Name::Custom(name)) => Ok(values.custom(name)?.is_some()),
            Self::Plain(Name::Custom(name), source) => {
                let Some(source) = values.substitute(source, false)? else {
                    return Ok(false);
                };
                let expected = match WideKeyword::parse(source.as_str()) {
                    Some(WideKeyword::Initial) => None,
                    Some(WideKeyword::Inherit | WideKeyword::Unset) => {
                        values.inherited_custom(name)?
                    }
                    Some(WideKeyword::Revert | WideKeyword::RevertLayer) => return Ok(false),
                    None => Some(source),
                };
                Ok(match (values.custom(name)?, expected) {
                    (None, None) => true,
                    (Some(actual), Some(expected)) => actual.as_str() == expected.as_str(),
                    _ => false,
                })
            }
            Self::Boolean(Name::Property(property)) => {
                for longhand in declaration::longhands(*property) {
                    let actual = values.property(longhand)?;
                    let initial =
                        values.candidate(longhand, &Specified::Wide(WideKeyword::Initial))?;
                    if properties::projection(longhand, &actual)
                        != properties::projection(longhand, &initial)
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Plain(Name::Property(property), source) => {
                let Some(source) = values.substitute(source, true)? else {
                    return Ok(false);
                };
                let Some(expanded) = declaration::expand_expanded(*property, &source)? else {
                    return Ok(false);
                };
                for (property, value) in expanded {
                    if rollback(&value) {
                        return Ok(false);
                    }
                    let actual = values.property(property)?;
                    let expected = values.candidate(property, &value)?;
                    if properties::projection(property, &actual)
                        != properties::projection(property, &expected)
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Range(operands, comparisons) => {
                let mut scalars = Vec::with_capacity(operands.len());
                for operand in operands {
                    let mut input = ParserInput::new(operand);
                    let input = &mut Parser::new(&mut input);
                    let custom = input
                        .try_parse(|input| {
                            let name = input.expect_ident_cloned()?;
                            input.expect_exhausted()?;
                            Ok::<_, cssparser::BasicParseError<'_>>(name)
                        })
                        .ok()
                        .filter(|name| name.starts_with(declaration::CUSTOM_PROPERTY_PREFIX));
                    let source = if let Some(name) = custom {
                        values.custom(&name)?
                    } else {
                        values.substitute(operand, false)?
                    };
                    let Some(source) = source else {
                        return Ok(false);
                    };
                    let Some(scalar) = values.scalar(source.as_str())? else {
                        return Ok(false);
                    };
                    scalars.push(scalar);
                }
                for (pair, comparison) in scalars.windows(2).zip(comparisons) {
                    if !pair[0].compatible(pair[1])
                        || !comparison.compare(pair[0].value.into(), pair[1].value.into())
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }
}

fn rollback(value: &Specified) -> bool {
    match value {
        Specified::Wide(WideKeyword::Revert | WideKeyword::RevertLayer) => true,
        Specified::Toggle(values) => values.iter().any(|value| rollback(&value.value)),
        _ => false,
    }
}

fn split_range(source: &str) -> Option<(Vec<String>, Vec<Comparison>)> {
    let mut raw = ParserInput::new(source);
    let input = &mut Parser::new(&mut raw);
    let mut start = input.position();
    let mut values = Vec::new();
    let mut comparisons = Vec::new();
    while !input.is_exhausted() {
        let before = input.state();
        let token = input.next().ok()?.clone();
        if matches!(token, Token::Delim('<' | '>' | '=')) {
            let value = crate::style::parser::trim_value(input.slice(start..before.position()));
            if value.is_empty() {
                return None;
            }
            values.push(value.to_owned());
            input.reset(&before);
            comparisons.push(comparison(input)?);
            start = input.position();
        } else if matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        ) {
            input
                .parse_nested_block(|nested| {
                    Ok::<_, cssparser::ParseError<'_, ()>>(block_source(nested))
                })
                .ok()?;
        }
    }
    let last = crate::style::parser::trim_value(input.slice_from(start));
    if last.is_empty() || comparisons.is_empty() || comparisons.len() > 2 {
        return None;
    }
    if comparisons.len() == 2
        && (comparisons[0].ascending().is_none()
            || comparisons[0].ascending() != comparisons[1].ascending())
    {
        return None;
    }
    values.push(last.to_owned());
    Some((values, comparisons))
}

#[derive(Clone, Copy)]
pub(super) struct Scalar {
    kind: calc::Kind,
    value: f32,
}
impl Scalar {
    fn compatible(self, other: Self) -> bool {
        self.kind == other.kind
            || (self.kind == calc::Kind::Number
                && self.value == 0.0
                && other.kind == calc::Kind::Length)
            || (other.kind == calc::Kind::Number
                && other.value == 0.0
                && self.kind == calc::Kind::Length)
    }

    pub(super) fn parse(source: &str, relative: &Relative<'_>) -> Result<Option<Self>, StyleError> {
        let mut raw = ParserInput::new(source);
        let input = &mut Parser::new(&mut raw);
        if let Some(math) = calc::math_function(
            input,
            calc::Context {
                relative,
                percentage: None,
                channels: &[],
            },
        ) {
            let math = match math {
                Ok(math) => math,
                Err(calc::MathError::Invalid(_)) => return Ok(None),
                Err(calc::MathError::Resolution(error)) => return Err(error),
            };
            let Some(kind) = math.kind() else {
                return Ok(None);
            };
            if input.expect_exhausted().is_err() {
                return Ok(None);
            }
            return Ok(Some(Self {
                kind,
                value: math.resolve(1.0, calc::Range::ALL),
            }));
        }
        let (kind, value) = match input.next().cloned() {
            Ok(Token::Number { value, .. }) => (calc::Kind::Number, value),
            Ok(Token::Percentage { unit_value, .. }) => (calc::Kind::Percentage, unit_value),
            Ok(Token::Dimension { value, unit, .. }) => match units::dimension(&unit) {
                Some(Dimension::Length(unit)) => {
                    (calc::Kind::Length, value * relative.per_unit(unit.scale)?)
                }
                Some(Dimension::Angle(scale)) => (calc::Kind::Angle, value * scale),
                Some(Dimension::Time(scale)) => (calc::Kind::Time, value * scale),
                Some(Dimension::Frequency(scale)) => (calc::Kind::Frequency, value * scale),
                Some(Dimension::Resolution(scale)) => (calc::Kind::Resolution, value * scale),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        if input.expect_exhausted().is_err() {
            return Ok(None);
        }
        Ok(Some(Self {
            kind,
            value: crate::paint::Calculation::Value(value).resolve(0.0),
        }))
    }
}
