use crate::style::{
    StyleError, calc,
    conditions::{Condition, Truth, block_source},
    print::{
        ColorScheme, ContrastPreference, MotionPreference, PrintEnvironment, PrintPreferences,
    },
    units::{self, Dimension, Relative, Scale},
};
use cssparser::{Parser, ParserInput, Token};

#[derive(Clone)]
pub(super) struct MediaQuery {
    queries: Vec<Query>,
}

#[derive(Clone)]
enum Query {
    Constant(bool),
    Condition(Condition<Feature>),
    Type {
        matching: bool,
        negate: bool,
        condition: Option<Condition<Feature>>,
    },
}

#[derive(Clone)]
pub(super) struct Feature {
    name: String,
    test: Test,
}

#[derive(Clone)]
enum Test {
    Boolean,
    Plain(Value),
    Range(Vec<(Comparison, Value)>),
}

#[derive(Clone, Copy)]
pub(super) enum Comparison {
    Equal,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

impl Comparison {
    fn reverse(self) -> Self {
        match self {
            Self::Less => Self::Greater,
            Self::LessEqual => Self::GreaterEqual,
            Self::Greater => Self::Less,
            Self::GreaterEqual => Self::LessEqual,
            Self::Equal => Self::Equal,
        }
    }
    pub(super) fn ascending(self) -> Option<bool> {
        match self {
            Self::Less | Self::LessEqual => Some(true),
            Self::Greater | Self::GreaterEqual => Some(false),
            Self::Equal => None,
        }
    }
    pub(super) fn compare(self, left: f64, right: f64) -> bool {
        match self {
            Self::Equal => left == right,
            Self::Less => left < right,
            Self::LessEqual => left <= right,
            Self::Greater => left > right,
            Self::GreaterEqual => left >= right,
        }
    }
}

#[derive(Clone)]
enum Value {
    Number { value: f32, integer: bool },
    Dimension(f32, String),
    Ident(String),
    Math(String),
    Ratio(Box<Value>, Box<Value>),
}

#[derive(Clone, Copy)]
enum ValueKind {
    Length,
    Ratio,
    Resolution,
    Integer,
    Grid,
    Keywords(&'static [&'static str]),
}

impl ValueKind {
    fn accepts_number(self, value: f64) -> bool {
        !matches!(self, Self::Ratio) || value >= 0.0
    }
}

#[derive(Clone, Copy)]
enum Actual {
    Number(f64),
    Keyword(&'static str),
}

struct FeatureSpec {
    kind: ValueKind,
    actual: Option<Actual>,
    paper: bool,
}

impl FeatureSpec {
    fn number(kind: ValueKind, value: f64, paper: bool) -> Self {
        Self {
            kind,
            actual: Some(Actual::Number(value)),
            paper,
        }
    }
    fn keywords(
        allowed: &'static [&'static str],
        actual: Option<&'static str>,
        paper: bool,
    ) -> Self {
        Self {
            kind: ValueKind::Keywords(allowed),
            actual: actual.map(Actual::Keyword),
            paper,
        }
    }
}

const COLOR_COMPONENT_BITS: f64 = 8.0;
const SINGLE_VIEWPORT_SEGMENT: f64 = 1.0;
const NO_INDEXED_COLORS: f64 = 0.0;
const NO_MONOCHROME_BUFFER: f64 = 0.0;
const BITMAP_GRID: f64 = 0.0;
const LITERAL_RANGE: calc::Range = calc::Range {
    min: -f32::MAX,
    max: f32::MAX,
};

fn spec(name: &str, paper: [f32; 2], preferences: &PrintPreferences) -> Option<FeatureSpec> {
    let number = FeatureSpec::number;
    let keywords = FeatureSpec::keywords;
    Some(match name {
        "width" | "device-width" => number(ValueKind::Length, paper[0].into(), true),
        "height" | "device-height" => number(ValueKind::Length, paper[1].into(), true),
        "aspect-ratio" | "device-aspect-ratio" => number(
            ValueKind::Ratio,
            f64::from(paper[0]) / f64::from(paper[1]),
            true,
        ),
        "orientation" => keywords(
            &["portrait", "landscape"],
            Some(if paper[0] <= paper[1] {
                "portrait"
            } else {
                "landscape"
            }),
            true,
        ),
        "resolution" => number(ValueKind::Resolution, f64::INFINITY, false),
        "color" => number(ValueKind::Integer, COLOR_COMPONENT_BITS, false),
        "color-index" => number(ValueKind::Integer, NO_INDEXED_COLORS, false),
        "monochrome" => number(ValueKind::Integer, NO_MONOCHROME_BUFFER, false),
        "grid" => number(ValueKind::Grid, BITMAP_GRID, false),
        "horizontal-viewport-segments" | "vertical-viewport-segments" => {
            number(ValueKind::Integer, SINGLE_VIEWPORT_SEGMENT, false)
        }
        "overflow-block" => keywords(&["none", "scroll", "paged"], Some("paged"), false),
        "overflow-inline" => keywords(&["none", "scroll"], Some("none"), false),
        "scan" => keywords(&["interlace", "progressive"], None, false),
        "display-mode" => keywords(
            &[
                "fullscreen",
                "standalone",
                "minimal-ui",
                "browser",
                "picture-in-picture",
            ],
            None,
            false,
        ),
        "update" => keywords(&["none", "slow", "fast"], Some("none"), false),
        "environment-blending" => keywords(
            &["opaque", "additive", "subtractive"],
            Some("opaque"),
            false,
        ),
        "color-gamut" => keywords(&["srgb", "p3", "rec2020"], Some("srgb"), false),
        "video-color-gamut" => keywords(&["srgb", "p3", "rec2020"], None, false),
        "dynamic-range" => keywords(&["standard", "high"], Some("standard"), false),
        "video-dynamic-range" => keywords(&["standard", "high"], None, false),
        "inverted-colors" => keywords(&["none", "inverted"], Some("none"), false),
        "pointer" | "any-pointer" => keywords(&["none", "coarse", "fine"], Some("none"), false),
        "hover" | "any-hover" => keywords(&["none", "hover"], Some("none"), false),
        "nav-controls" => keywords(&["none", "back"], Some("none"), false),
        "scripting" => keywords(&["none", "initial-only", "enabled"], Some("none"), false),
        "prefers-reduced-motion" => keywords(
            &["no-preference", "reduce"],
            Some(match preferences.reduced_motion {
                MotionPreference::NoPreference => "no-preference",
                MotionPreference::Reduce => "reduce",
            }),
            false,
        ),
        "prefers-reduced-transparency" | "prefers-reduced-data" => {
            keywords(&["no-preference", "reduce"], Some("no-preference"), false)
        }
        "prefers-contrast" => keywords(
            &["no-preference", "less", "more", "custom"],
            Some(match preferences.contrast {
                ContrastPreference::NoPreference => "no-preference",
                ContrastPreference::Less => "less",
                ContrastPreference::More => "more",
                ContrastPreference::Custom => "custom",
            }),
            false,
        ),
        "forced-colors" => keywords(&["none", "active"], Some("none"), false),
        "prefers-color-scheme" => keywords(
            &["light", "dark"],
            Some(match preferences.color_scheme {
                ColorScheme::Light => "light",
                ColorScheme::Dark => "dark",
            }),
            false,
        ),
        _ => return None,
    })
}

/// CSS Conditional 5 §6.1: the size features of a query container with content-box `size`.
fn container_spec(name: &str, [width, height]: [f32; 2]) -> Option<FeatureSpec> {
    Some(match name {
        "width" | "inline-size" => FeatureSpec::number(ValueKind::Length, width.into(), false),
        "height" | "block-size" => FeatureSpec::number(ValueKind::Length, height.into(), false),
        "aspect-ratio" => FeatureSpec::number(
            ValueKind::Ratio,
            f64::from(width) / f64::from(height),
            false,
        ),
        "orientation" => FeatureSpec::keywords(
            &["portrait", "landscape"],
            Some(if height >= width {
                "portrait"
            } else {
                "landscape"
            }),
            false,
        ),
        _ => return None,
    })
}

pub(super) struct RangeFeature {
    pub(super) kind: calc::Kind,
    pub(super) actual: f32,
    value_kind: ValueKind,
}

impl RangeFeature {
    pub(super) fn calculation(&self, math: &calc::Math) -> Option<crate::paint::Calculation> {
        let value = math.resolve(0.0, calc::Range::ALL);
        if !self.value_kind.accepts_number(value.into()) {
            return None;
        }
        Some(crate::paint::Calculation::Value(
            if matches!(self.value_kind, ValueKind::Integer) {
                math.integer(calc::Range::ALL) as f32
            } else {
                value
            },
        ))
    }
}

pub(super) fn range_feature(name: &str, paper: [f32; 2]) -> Option<RangeFeature> {
    let spec = spec(name, paper, &PrintPreferences::default())?;
    let kind = match spec.kind {
        ValueKind::Length => calc::Kind::Length,
        ValueKind::Ratio | ValueKind::Integer => calc::Kind::Number,
        ValueKind::Resolution => calc::Kind::Resolution,
        ValueKind::Grid | ValueKind::Keywords(_) => return None,
    };
    let Actual::Number(actual) = spec.actual? else {
        return None;
    };
    Some(RangeFeature {
        kind,
        actual: actual as f32,
        value_kind: spec.kind,
    })
}

pub(super) fn comparison(input: &mut Parser<'_, '_>) -> Option<Comparison> {
    let token = input.next().ok()?.clone();
    let operator = match token {
        Token::Delim('<') => Comparison::Less,
        Token::Delim('>') => Comparison::Greater,
        Token::Delim('=') => return Some(Comparison::Equal),
        _ => return None,
    };
    let state = input.state();
    if let Ok(Token::Delim('=')) = input.next_including_whitespace() {
        Some(match operator {
            Comparison::Less => Comparison::LessEqual,
            _ => Comparison::GreaterEqual,
        })
    } else {
        input.reset(&state);
        Some(operator)
    }
}

impl Value {
    fn parse(input: &mut Parser<'_, '_>) -> Option<Self> {
        let value = Self::atom(input)?;
        if input.try_parse(|input| input.expect_delim('/')).is_ok() {
            Some(Self::Ratio(Box::new(value), Box::new(Self::atom(input)?)))
        } else {
            Some(value)
        }
    }

    fn atom(input: &mut Parser<'_, '_>) -> Option<Self> {
        let start = input.position();
        let value = match input.next().ok()?.clone() {
            Token::Number {
                value, int_value, ..
            } => Self::Number {
                value: value.clamp(LITERAL_RANGE.min, LITERAL_RANGE.max),
                integer: int_value.is_some(),
            },
            Token::Dimension { value, unit, .. } => Self::Dimension(
                value.clamp(LITERAL_RANGE.min, LITERAL_RANGE.max),
                unit.to_string(),
            ),
            Token::Ident(name) => Self::Ident(name.to_ascii_lowercase()),
            Token::Function(name) if calc::is_math_function(&name) => {
                input
                    .parse_nested_block(|input| {
                        block_source(input);
                        Ok::<_, cssparser::ParseError<'_, ()>>(())
                    })
                    .ok()?;
                Self::Math(input.slice_from(start).into())
            }
            _ => return None,
        };
        Some(value)
    }

    fn scalar(&self, kind: ValueKind, relative: &Relative<'_>) -> Option<f64> {
        match (self, kind) {
            (Self::Number { value: 0.0, .. }, ValueKind::Length) => Some(0.0),
            (
                Self::Number {
                    value,
                    integer: true,
                },
                ValueKind::Integer | ValueKind::Grid,
            ) => Some((*value).into()),
            (Self::Dimension(value, name), ValueKind::Length) => match units::dimension(name)? {
                Dimension::Length(unit) => Some(
                    (*value * relative.per_unit(unit.scale).ok()?)
                        .clamp(LITERAL_RANGE.min, LITERAL_RANGE.max)
                        .into(),
                ),
                _ => None,
            },
            (Self::Dimension(value, name), ValueKind::Resolution) => {
                match units::dimension(name)? {
                    Dimension::Resolution(scale) => Some(f64::from(*value) * f64::from(scale)),
                    _ => None,
                }
            }
            (Self::Ident(name), ValueKind::Resolution) if name == "infinite" => Some(f64::INFINITY),
            (value, ValueKind::Ratio) => {
                let (left, right) = match value {
                    Self::Ratio(left, right) => (left.number(relative)?, right.number(relative)?),
                    _ => (value.number(relative)?, 1.0),
                };
                (kind.accepts_number(left) && kind.accepts_number(right)).then_some(left / right)
            }
            (Self::Math(_), kind) => {
                let math = self.calculation(relative)?;
                let expected = match kind {
                    ValueKind::Length => calc::Kind::Length,
                    ValueKind::Resolution => calc::Kind::Resolution,
                    ValueKind::Integer | ValueKind::Grid => calc::Kind::Number,
                    _ => return None,
                };
                let math = math.expect(&[expected]).ok()?;
                Some(if matches!(kind, ValueKind::Integer | ValueKind::Grid) {
                    math.integer(calc::Range::ALL).into()
                } else {
                    math.resolve(0.0, calc::Range::ALL).into()
                })
            }
            _ => None,
        }
    }

    fn number(&self, relative: &Relative<'_>) -> Option<f64> {
        match self {
            Self::Number { value, .. } => Some((*value).into()),
            Self::Math(_) => {
                let math = self
                    .calculation(relative)?
                    .expect(&[calc::Kind::Number])
                    .ok()?;
                Some(math.resolve(0.0, calc::Range::ALL).into())
            }
            _ => None,
        }
    }

    fn calculation(&self, relative: &Relative<'_>) -> Option<calc::Math> {
        let Self::Math(source) = self else {
            return None;
        };
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        let value = calc::math_function(
            input,
            calc::Context {
                relative,
                percentage: None,
                channels: &[],
            },
        )?
        .ok()?;
        input.expect_exhausted().ok()?;
        Some(value)
    }

    fn uses_paper(&self, environment: &PrintEnvironment) -> bool {
        fn viewport(unit: &str) -> bool {
            matches!(
                units::dimension(unit),
                Some(Dimension::Length(units::LengthUnit {
                    scale: Scale::Viewport(_) | Scale::Container(_),
                    ..
                }))
            )
        }
        match self {
            Self::Dimension(_, unit) => viewport(unit),
            Self::Ratio(left, right) => {
                left.uses_paper(environment) || right.uses_paper(environment)
            }
            Self::Math(source) => {
                let mut pending = vec![source.as_str()];
                while let Some(source) = pending.pop() {
                    let mut input = ParserInput::new(source);
                    let input = &mut Parser::new(&mut input);
                    while let Ok(token) = input.next().cloned() {
                        match token {
                            Token::Dimension { unit, .. } if viewport(&unit) => return true,
                            token @ (Token::Function(_) | Token::ParenthesisBlock) => {
                                let progress = matches!(token, Token::Function(name) if name.eq_ignore_ascii_case("media-progress"));
                                if let Ok((source, paper)) = input.parse_nested_block(|input| {
                                    let paper = progress
                                        && input
                                            .try_parse(Parser::expect_ident_cloned)
                                            .ok()
                                            .and_then(|name| {
                                                spec(
                                                    &name.to_ascii_lowercase(),
                                                    environment.paper,
                                                    &environment.preferences,
                                                )
                                            })
                                            .is_some_and(|spec| spec.paper);
                                    Ok::<_, cssparser::ParseError<'_, ()>>((
                                        block_source(input),
                                        paper,
                                    ))
                                }) {
                                    if paper {
                                        return true;
                                    }
                                    pending.push(source);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }
}

impl Feature {
    pub(super) fn dependency(&self) -> Option<&str> {
        (self.name.starts_with("--") && matches!(self.test, Test::Boolean))
            .then_some(self.name.as_str())
    }

    pub(super) fn parse(source: &str) -> Option<Self> {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        let start = input.state();
        if let Ok(name) = input.expect_ident_cloned() {
            let name = name.to_string();
            if input.is_exhausted() {
                return Some(Self {
                    name,
                    test: Test::Boolean,
                });
            }
            if input.try_parse(Parser::expect_colon).is_ok() {
                let value = Value::parse(input)?;
                input.expect_exhausted().ok()?;
                return Some(Self {
                    name,
                    test: Test::Plain(value),
                });
            }
        }
        input.reset(&start);
        let first = Value::parse(input)?;
        let operator = comparison(input)?;
        let middle = Value::parse(input)?;
        if input.is_exhausted() {
            if let Value::Ident(name) = &first
                && name != "infinite"
            {
                return Some(Self {
                    name: name.clone(),
                    test: Test::Range(vec![(operator, middle)]),
                });
            }
            if let Value::Ident(name) = middle {
                return Some(Self {
                    name,
                    test: Test::Range(vec![(operator.reverse(), first)]),
                });
            }
            return None;
        }
        let next = comparison(input)?;
        if operator.ascending().is_none() || operator.ascending() != next.ascending() {
            return None;
        }
        let last = Value::parse(input)?;
        input.expect_exhausted().ok()?;
        let Value::Ident(name) = middle else {
            return None;
        };
        Some(Self {
            name,
            test: Test::Range(vec![(operator.reverse(), first), (next, last)]),
        })
    }

    pub(super) fn evaluate(&self, environment: &PrintEnvironment) -> Truth {
        if self.name.starts_with("--") {
            return if matches!(self.test, Test::Boolean) {
                environment
                    .custom_media
                    .get(&self.name)
                    .and_then(|value| value.matching)
                    .map_or(Truth::Unknown, Into::into)
            } else {
                Truth::Unknown
            };
        }
        self.evaluate_with(
            |name| spec(name, environment.paper, &environment.preferences),
            &environment.relative,
        )
    }

    /// CSS Conditional 5 §6.1: a size feature of a query container with content-box `size`,
    /// relative lengths resolving against `relative`, the container's computed values.
    pub(super) fn evaluate_container(&self, size: [f32; 2], relative: &Relative<'_>) -> Truth {
        self.evaluate_with(|name| container_spec(name, size), relative)
    }

    /// The width and height a size container feature queries; `None` for a feature that is
    /// not one, which no container answers.
    pub(super) fn container_axes(&self) -> Option<[bool; 2]> {
        self.definition_with(|name| container_spec(name, [0.0; 2]))?;
        let name = self.name.to_ascii_lowercase();
        let name = name
            .strip_prefix("min-")
            .or_else(|| name.strip_prefix("max-"))
            .unwrap_or(&name);
        Some(match name {
            "width" | "inline-size" => [true, false],
            "height" | "block-size" => [false, true],
            _ => [true, true],
        })
    }

    fn evaluate_with(
        &self,
        lookup: impl Fn(&str) -> Option<FeatureSpec>,
        relative: &Relative<'_>,
    ) -> Truth {
        let Some((spec, prefix)) = self.definition_with(lookup) else {
            return Truth::Unknown;
        };
        match &self.test {
            Test::Boolean => match spec.actual {
                Some(Actual::Number(value)) => (value != 0.0).into(),
                Some(Actual::Keyword(value)) => (!matches!(value, "none" | "no-preference")).into(),
                None => Truth::False,
            },
            Test::Plain(value) => {
                Self::compare(&spec, prefix.unwrap_or(Comparison::Equal), value, relative)
            }
            Test::Range(comparisons) => {
                let mut result = Truth::True;
                for (comparison, value) in comparisons {
                    let comparison = Self::compare(&spec, *comparison, value, relative);
                    if comparison == Truth::Unknown {
                        return Truth::Unknown;
                    }
                    result = result.and(comparison);
                }
                result
            }
        }
    }

    fn definition(
        &self,
        environment: &PrintEnvironment,
    ) -> Option<(FeatureSpec, Option<Comparison>)> {
        self.definition_with(|name| spec(name, environment.paper, &environment.preferences))
    }

    fn definition_with(
        &self,
        lookup: impl Fn(&str) -> Option<FeatureSpec>,
    ) -> Option<(FeatureSpec, Option<Comparison>)> {
        let name = self.name.to_ascii_lowercase();
        let (name, prefix) = match (name.strip_prefix("min-"), name.strip_prefix("max-")) {
            (Some(name), _) => (name, Some(Comparison::GreaterEqual)),
            (_, Some(name)) => (name, Some(Comparison::LessEqual)),
            _ => (name.as_str(), None),
        };
        let spec = lookup(name)?;
        if prefix.is_some()
            && (!matches!(self.test, Test::Plain(_))
                || matches!(spec.kind, ValueKind::Keywords(_) | ValueKind::Grid))
        {
            return None;
        }
        if matches!(self.test, Test::Range(_))
            && matches!(spec.kind, ValueKind::Grid | ValueKind::Keywords(_))
        {
            return None;
        }
        Some((spec, prefix))
    }

    fn compare(
        spec: &FeatureSpec,
        comparison: Comparison,
        value: &Value,
        relative: &Relative<'_>,
    ) -> Truth {
        match spec.kind {
            ValueKind::Keywords(allowed) => {
                let Value::Ident(value) = value else {
                    return Truth::Unknown;
                };
                if !allowed.contains(&value.as_str()) {
                    return Truth::Unknown;
                }
                matches!(spec.actual,Some(Actual::Keyword(actual)) if actual==value).into()
            }
            kind => {
                let Some(value) = value.scalar(kind, relative) else {
                    return Truth::Unknown;
                };
                if matches!(kind, ValueKind::Grid) && value != 0.0 && value != 1.0 {
                    return Truth::Unknown;
                }
                match spec.actual {
                    Some(Actual::Number(actual)) => comparison.compare(actual, value).into(),
                    _ => Truth::False,
                }
            }
        }
    }

    pub(super) fn uses_paper(&self, environment: &PrintEnvironment) -> bool {
        if self.name.starts_with("--") {
            return matches!(self.test, Test::Boolean)
                && environment
                    .custom_media
                    .get(&self.name)
                    .is_some_and(|value| value.paper_dependent);
        }
        let Some((spec, _)) = self.definition(environment) else {
            return false;
        };
        spec.paper
            || match &self.test {
                Test::Boolean => false,
                Test::Plain(value) => value.uses_paper(environment),
                Test::Range(values) => values
                    .iter()
                    .any(|(_, value)| value.uses_paper(environment)),
            }
    }
}

impl MediaQuery {
    pub(crate) fn constant(value: bool) -> Self {
        Self {
            queries: vec![Query::Constant(value)],
        }
    }

    pub(crate) fn parse(source: &str) -> Result<Self, StyleError> {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        if input.is_exhausted() {
            return Ok(Self::constant(true));
        }
        let queries = input
            .parse_comma_separated(|input| {
                Self::query(block_source(input)).map_err(|error| input.new_custom_error(error))
            })
            .map_err(crate::style::parser::failure)?;
        Ok(Self { queries })
    }

    fn query(source: &str) -> Result<Query, StyleError> {
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        let mut negate = false;
        let mut modifier = false;
        let start = input.state();
        if let Ok(ident) = input.try_parse(Parser::expect_ident_cloned) {
            if ident.eq_ignore_ascii_case("not") || ident.eq_ignore_ascii_case("only") {
                negate = ident.eq_ignore_ascii_case("not");
                modifier = true;
            } else {
                input.reset(&start);
            }
        }
        if let Ok(name) = input.try_parse(Parser::expect_ident_cloned) {
            if ["only", "not", "and", "or", "layer"]
                .iter()
                .any(|reserved| name.eq_ignore_ascii_case(reserved))
            {
                return Ok(Query::Constant(false));
            }
            let condition = if input.is_exhausted() {
                None
            } else {
                if input.expect_ident_matching("and").is_err() {
                    return Ok(Query::Constant(false));
                }
                let Some(condition) =
                    Condition::parse(block_source(input), false, Truth::Unknown, Feature::parse)?
                else {
                    return Ok(Query::Constant(false));
                };
                Some(condition)
            };
            return Ok(Query::Type {
                matching: name.eq_ignore_ascii_case("all") || name.eq_ignore_ascii_case("print"),
                negate,
                condition,
            });
        }
        if modifier && !negate {
            return Ok(Query::Constant(false));
        }
        Ok(
            Condition::parse(source, true, Truth::Unknown, Feature::parse)?
                .map_or(Query::Constant(false), Query::Condition),
        )
    }

    pub(crate) fn dependencies(&self) -> impl Iterator<Item = &str> {
        self.conditions()
            .flat_map(Condition::leaves)
            .filter_map(|feature| feature.dependency())
    }

    fn conditions(&self) -> impl Iterator<Item = &Condition<Feature>> {
        self.queries.iter().filter_map(|query| match query {
            Query::Condition(condition)
            | Query::Type {
                condition: Some(condition),
                ..
            } => Some(condition),
            _ => None,
        })
    }

    pub(crate) fn uses_paper(&self, environment: &PrintEnvironment) -> bool {
        self.conditions()
            .flat_map(Condition::leaves)
            .any(|feature| feature.uses_paper(environment))
    }
}

pub(super) fn matches_media(query: &MediaQuery, environment: &PrintEnvironment) -> bool {
    query.queries.iter().any(|query| match query {
        Query::Constant(value) => *value,
        Query::Condition(condition) => {
            condition.evaluate(|feature| feature.evaluate(environment)) == Truth::True
        }
        Query::Type {
            matching,
            negate,
            condition,
        } => {
            let value =
                Truth::from(*matching).and(condition.as_ref().map_or(Truth::True, |condition| {
                    condition.evaluate(|feature| feature.evaluate(environment))
                }));
            (if *negate { value.not() } else { value }) == Truth::True
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{PrintSettings, print::PrintEnvironment};

    fn matches(source: &str) -> bool {
        let environment = PrintEnvironment::new(&PrintSettings {
            paper: [600.0, 800.0],
            ..Default::default()
        })
        .unwrap();
        matches_media(&MediaQuery::parse(source).unwrap(), &environment)
    }

    #[test]
    fn media_static_features_follow_the_selected_print_definitions() {
        for query in [
            "(width:600pt)",
            "(height:800pt)",
            "(device-width:600pt)",
            "(device-height:800pt)",
            "(aspect-ratio:3/4)",
            "(device-aspect-ratio:.75)",
            "(orientation:portrait)",
            "(overflow-block:paged)",
            "(overflow-inline:none)",
            "(horizontal-viewport-segments:1)",
            "(vertical-viewport-segments:1)",
            "(resolution:infinite)",
            "(grid:0)",
            "(update:none)",
            "(environment-blending:opaque)",
            "(color:8)",
            "(color-index:0)",
            "(monochrome:0)",
            "(color-gamut:srgb)",
            "(dynamic-range:standard)",
            "(inverted-colors:none)",
            "(pointer:none)",
            "(hover:none)",
            "(any-pointer:none)",
            "(any-hover:none)",
            "(nav-controls:none)",
            "(scripting:none)",
            "(prefers-reduced-motion:no-preference)",
            "(prefers-reduced-transparency:no-preference)",
            "(prefers-contrast:no-preference)",
            "(forced-colors:none)",
            "(prefers-color-scheme:light)",
            "(prefers-reduced-data:no-preference)",
        ] {
            assert!(matches(query), "{query}");
        }
        for query in [
            "(scan)",
            "(scan:progressive)",
            "(display-mode:browser)",
            "(video-color-gamut:srgb)",
            "(video-dynamic-range:standard)",
            "(grid)",
            "(color-index)",
            "(monochrome)",
            "(pointer)",
            "(hover)",
            "(any-pointer)",
            "(any-hover)",
            "(nav-controls)",
            "(update)",
            "(scripting)",
            "(forced-colors)",
            "(inverted-colors)",
            "(prefers-contrast)",
            "(prefers-reduced-motion)",
            "(prefers-reduced-transparency)",
            "(prefers-reduced-data)",
            "(color-gamut:p3)",
            "(dynamic-range:high)",
        ] {
            assert!(!matches(query), "{query}");
        }
    }

    #[test]
    fn media_queries_use_kleene_logic_and_recover_each_list_item() {
        for (query, expected) in [
            ("", true),
            ("PRINT", true),
            ("only print", true),
            ("not screen", true),
            ("not unknown-device", true),
            ("not (unknown-feature)", false),
            ("not (color:unknown-value)", false),
            ("not future-function(1)", false),
            ("(future:1) or (color)", true),
            ("not ((future:1) and (monochrome))", true),
            ("(future:1) and (color)", false),
            ("(color) and (width) or (height)", false),
            ("((color) and (width)) or (height)", true),
            ("screen or (color)", false),
            ("print and ((color) or (hover))", true),
            ("or and (color)", false),
            ("layer", false),
            ("not layer", false),
            ("screen, &, print", true),
            ("(color, print), screen", false),
            ("only (color)", false),
            ("not (grid:2)", false),
            ("not (min-grid:0)", false),
            ("not (min-width)", false),
        ] {
            assert_eq!(matches(query), expected, "{query}");
        }
    }

    #[test]
    fn media_ranges_validate_types_and_operator_adjacency() {
        for (query, expected) in [
            ("(400pt < width <= 600pt)", true),
            ("(600pt >= width > 400pt)", true),
            ("(400pt < width > 300pt)", false),
            ("(width > = 400pt)", false),
            ("(width >/**/= 400pt)", true),
            ("not (width:-1pt)", true),
            ("(width > -1pt)", true),
            ("not (width:1%)", false),
            ("(width:calc(300pt * 2))", true),
            ("(width:50em)", true),
            ("(width:100vw)", true),
            ("(width:calc(50vw + 300pt))", true),
            ("(color:calc(4 * 2))", true),
            ("(color:8.5)", false),
            ("not (color:8.5)", false),
            ("(grid:-0)", true),
            ("(aspect-ratio:0.75)", true),
            ("(aspect-ratio < 1/0)", true),
            ("(aspect-ratio > 0/1)", true),
            ("(aspect-ratio:-1/2)", false),
            ("(min-resolution:infinite)", true),
            ("(max-resolution:1000000000dpi)", false),
            ("(resolution>1000dppx)", true),
            ("(resolution:1e999dpi)", false),
            ("(resolution>1e999dpi)", true),
            ("(width<1e999in)", true),
            ("not (width:1e999in)", true),
            ("(width>-1e999in)", true),
            ("not (1000pt < width < 1unknown)", false),
            ("not (2 < aspect-ratio < 1pt)", false),
        ] {
            assert_eq!(matches(query), expected, "{query}");
        }
    }

    #[test]
    fn media_condition_nesting_has_a_typed_bound_without_stack_growth() {
        use crate::style::conditions::MAX_CONDITION_DEPTH;
        let source = format!(
            "{}color{}",
            "(".repeat(MAX_CONDITION_DEPTH),
            ")".repeat(MAX_CONDITION_DEPTH)
        );
        assert!(matches(&source));
        let source = format!("({source})");
        assert!(matches!(
            MediaQuery::parse(&source),
            Err(StyleError::GrammarLimit {
                resource: "condition nesting",
                limit: MAX_CONDITION_DEPTH
            })
        ));
    }

    #[test]
    fn media_ratio_rejects_extra_operands_without_recursive_parsing() {
        const OPERANDS: usize = 10_000;
        let source = format!(
            "(aspect-ratio:{})",
            std::iter::repeat_n("1", OPERANDS)
                .collect::<Vec<_>>()
                .join("/")
        );
        assert!(!matches(&source));
    }
}
