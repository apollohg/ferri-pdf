//! CSS Values 4 math functions, type-checked into graphics calculation trees.
use crate::paint::{
    BinaryFunction, Calculation, LengthPercentage, RoundingStrategy, UnaryFunction,
};
use crate::style::StyleError;
use crate::style::anchor_values::{AnchorDimension, AnchorEvaluation, AnchorQuery, AnchorSide};
use crate::style::grammar::Keyword;
use crate::style::parser::MAX_COMPONENT_DEPTH;
use crate::style::units::{self, Dimension, Relative};
use cssparser::{BasicParseError, Delimiter, ParseError, ParseErrorKind, Parser, Token};
use std::cell::Cell;

const MATH_FUNCTIONS: &[&str] = &[
    "calc",
    "min",
    "max",
    "clamp",
    "round",
    "mod",
    "rem",
    "abs",
    "sign",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "pow",
    "sqrt",
    "hypot",
    "log",
    "exp",
    "progress",
    "sibling-count",
    "sibling-index",
    "media-progress",
    "calc-mix",
];
#[derive(Default)]
pub(crate) struct Work {
    steps: Cell<usize>,
    depth: Cell<usize>,
}
impl Work {
    pub(crate) fn steps(&self) -> usize {
        self.steps.get()
    }
    pub(super) fn check_value_nesting(&self) -> Result<(), StyleError> {
        if self.depth.get() >= crate::style::variables::MAX_VALUE_DEPTH {
            Err(StyleError::VariableLimit {
                resource: "value nesting",
                limit: crate::style::variables::MAX_VALUE_DEPTH,
            })
        } else {
            Ok(())
        }
    }

    pub(crate) fn charge(&self, amount: usize) -> Result<(), StyleError> {
        let steps = self.steps.get().saturating_add(amount);
        self.steps.set(steps);
        if steps > crate::style::matcher::MAX_MATCH_STEPS {
            Err(StyleError::GrammarLimit {
                resource: "value computation work",
                limit: crate::style::matcher::MAX_MATCH_STEPS,
            })
        } else {
            Ok(())
        }
    }
    pub(super) fn enter(&self, grammar: bool) -> WorkGuard<'_> {
        if self.depth.get() == 0 && !grammar {
            self.steps.set(0);
        }
        self.depth.set(self.depth.get() + 1);
        WorkGuard(self)
    }
}
pub(super) struct WorkGuard<'a>(&'a Work);
impl Drop for WorkGuard<'_> {
    fn drop(&mut self) {
        self.0.depth.set(self.0.depth.get() - 1);
    }
}

const CLAMP_ARGUMENTS: usize = 3;
pub(super) const CALC_SIZE_FUNCTION: &str = "calc-size";
/// Bounds the calculation that substituting nested `calc-size()` bases produces (CSS Values 5
/// §10.1), as variable substitution is bounded.
const MAX_CALC_SIZE_NODES: usize = 4096;
const ANCHOR_SIZE_FUNCTION: &str = "anchor-size";
const NONE: &str = "none";
/// Integers round half towards positive infinity.
const INTEGER_ROUNDING_MIDPOINT: f32 = 0.5;

/// Top-level types a property can accept from a math function.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Number,
    Length,
    Angle,
    Time,
    Frequency,
    Resolution,
    Percentage,
}

#[derive(Clone, Copy, Debug)]
enum Base {
    Length,
    Angle,
    Time,
    Frequency,
    Resolution,
    Percentage,
}
const BASE_TYPES: usize = 6;

/// CSS Values 4 type: exponents of each base type; all zero is `<number>`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CalcType([i32; BASE_TYPES]);
impl CalcType {
    const NUMBER: Self = Self([0; BASE_TYPES]);
    fn base(base: Base) -> Self {
        let mut exponents = [0; BASE_TYPES];
        exponents[base as usize] = 1;
        Self(exponents)
    }
    fn of(kind: Kind) -> Self {
        match kind {
            Kind::Number => Self::NUMBER,
            Kind::Length => Self::base(Base::Length),
            Kind::Angle => Self::base(Base::Angle),
            Kind::Time => Self::base(Base::Time),
            Kind::Frequency => Self::base(Base::Frequency),
            Kind::Resolution => Self::base(Base::Resolution),
            Kind::Percentage => Self::base(Base::Percentage),
        }
    }
    fn multiply(self, other: Self) -> Self {
        Self(std::array::from_fn(|base| {
            self.0[base].saturating_add(other.0[base])
        }))
    }
    fn invert(self) -> Self {
        Self(self.0.map(|exponent| -exponent))
    }
    fn kind(self) -> Option<Kind> {
        [
            Kind::Number,
            Kind::Length,
            Kind::Angle,
            Kind::Time,
            Kind::Frequency,
            Kind::Resolution,
            Kind::Percentage,
        ]
        .into_iter()
        .find(|kind| Self::of(*kind) == self)
    }
}

/// Values a property allows; literals outside it are invalid, calculations clamp into it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Range {
    pub(super) min: f32,
    pub(super) max: f32,
}
impl Range {
    pub(super) const ALL: Self = Self {
        min: f32::NEG_INFINITY,
        max: f32::INFINITY,
    };
    pub(super) fn contains(self, value: f32) -> bool {
        (self.min..=self.max).contains(&value)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Context<'r> {
    pub(super) relative: &'r Relative<'r>,
    /// Percentage hint; `None` preserves the independent percentage type.
    pub(super) percentage: Option<Kind>,
    /// Relative colour channel keywords and their `<number>` values (CSS Color 5 §4.2).
    pub(super) channels: &'r [(&'r str, f32)],
    /// The anchor functions the property accepts, and what they resolve against.
    pub(super) anchors: Option<AnchorEvaluation<'r>>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum MathError {
    #[error("invalid math function: {0}")]
    Invalid(String),
    #[error(transparent)]
    Resolution(#[from] StyleError),
}
impl From<MathError> for String {
    fn from(error: MathError) -> Self {
        error.to_string()
    }
}

pub(super) struct Math {
    kind: Option<Kind>,
    calculation: Calculation,
}
impl Math {
    /// Type-checks the top-level result.
    pub(super) fn expect(self, kinds: &[Kind]) -> Result<Self, MathError> {
        if !self.kind.is_some_and(|kind| kinds.contains(&kind)) {
            return Err(MathError::Invalid(format!(
                "{:?} where {kinds:?} is required",
                self.kind
            )));
        }
        Ok(self)
    }
    /// Only meaningful after `expect`, which rejects types without a kind.
    pub(super) fn kind(&self) -> Option<Kind> {
        self.kind
    }
    pub(super) fn resolve(&self, basis: f32, range: Range) -> f32 {
        self.calculation.resolve(basis).clamp(range.min, range.max)
    }
    pub(super) fn integer(&self, range: Range) -> i32 {
        let value = self.calculation.evaluate(0.0);
        let value = if value.is_nan() { 0.0 } else { value };
        let rounded = value.round();
        let rounded = if value.fract() == -INTEGER_ROUNDING_MIDPOINT {
            rounded + 1.0
        } else {
            rounded
        };
        rounded.clamp(range.min, range.max) as i32
    }
    /// Keeps percentage terms for layout, clamping the used value into `range`.
    pub(super) fn length_percentage(self, range: Range) -> LengthPercentage {
        let mut calculation = match self.calculation {
            Calculation::Value(_) => return LengthPercentage::Length(self.resolve(0.0, range)),
            // Bases are non-negative, so an in-range fraction stays in range.
            Calculation::Percentage(fraction)
                if fraction.is_finite() && range.contains(fraction) =>
            {
                return LengthPercentage::Percentage(unsigned_zero(fraction));
            }
            calculation => calculation,
        };
        if range.min.is_finite() {
            calculation = Calculation::Max(vec![calculation, Calculation::Value(range.min)]);
        }
        if range.max.is_finite() {
            calculation = Calculation::Min(vec![calculation, Calculation::Value(range.max)]);
        }
        calculation.into()
    }
}

pub(super) fn is_math_function(name: &str) -> bool {
    MATH_FUNCTIONS
        .iter()
        .any(|function| name.eq_ignore_ascii_case(function))
}

/// Parses the next component when it is a math function; otherwise consumes nothing.
pub(super) fn math_function(
    input: &mut Parser<'_, '_>,
    context: Context<'_>,
) -> Option<Result<Math, MathError>> {
    let start = input.state();
    match input.next() {
        Ok(Token::Function(name))
            if is_math_function(name)
                || context
                    .anchors
                    .is_some_and(|anchors| anchors.functions.allows(name)) =>
        {
            let name = name.to_ascii_lowercase();
            let _work = context.relative.work.enter(context.relative.grammar_only);
            let mut parser = MathParser {
                context,
                depth: 0,
                size: false,
            };
            let result = if parser
                .context
                .anchors
                .is_some_and(|anchors| anchors.functions.allows(&name))
            {
                let size = name.eq_ignore_ascii_case(ANCHOR_SIZE_FUNCTION);
                parser.nested(input, |parser, input| parser.anchor_function(size, input))
            } else {
                parser.function(&name, input)
            };
            Some(
                result
                    .map(|(calculation, kind)| Math {
                        kind: kind.kind(),
                        calculation,
                    })
                    .map_err(|error| match error.kind {
                        ParseErrorKind::Custom(error) => error,
                        ParseErrorKind::Basic(error) => MathError::Invalid(format!("{error:?}")),
                    }),
            )
        }
        _ => {
            input.reset(&start);
            None
        }
    }
}

type Typed = (Calculation, CalcType);
type MathResult<'i> = Result<Typed, ParseError<'i, MathError>>;

struct MathParser<'r> {
    context: Context<'r>,
    depth: usize,
    /// The `calc-size()` calculation being parsed allows the `size` keyword.
    size: bool,
}
impl MathParser<'_> {
    fn invalid<'i, T>(
        input: &Parser<'i, '_>,
        message: impl Into<String>,
    ) -> Result<T, ParseError<'i, MathError>> {
        Err(input.new_custom_error(MathError::Invalid(message.into())))
    }
    fn nested<'i, T>(
        &mut self,
        input: &mut Parser<'i, '_>,
        parse: impl FnOnce(&mut Self, &mut Parser<'i, '_>) -> Result<T, ParseError<'i, MathError>>,
    ) -> Result<T, ParseError<'i, MathError>> {
        self.context
            .relative
            .work
            .charge(1)
            .map_err(|error| input.new_custom_error(MathError::Resolution(error)))?;
        if self.depth >= MAX_COMPONENT_DEPTH {
            return Self::invalid(input, format!("nesting exceeds {MAX_COMPONENT_DEPTH}"));
        }
        self.depth += 1;
        let result = input.parse_nested_block(|nested| parse(self, nested));
        self.depth -= 1;
        result
    }
    fn function<'i>(&mut self, name: &str, input: &mut Parser<'i, '_>) -> MathResult<'i> {
        self.nested(input, |parser, input| match name {
            "calc" => parser.sum(input),
            "clamp" => parser.clamp(input),
            "round" => parser.round(input),
            "media-progress" => parser.media_progress(input),
            "calc-mix" => parser.calc_mix(input),
            "sibling-count" | "sibling-index" => {
                input.expect_exhausted()?;
                let Some((index, count)) = parser.context.relative.tree.siblings() else {
                    return Self::invalid(input, "tree counting requires an element context");
                };
                let value = if name == "sibling-count" {
                    count
                } else {
                    index
                };
                Ok((Calculation::Value(value as f32), CalcType::NUMBER))
            }
            _ => {
                let (items, kind) = parser.arguments(input)?;
                Self::apply(name, items, kind, input)
            }
        })
    }
    fn component_source<'i>(
        input: &mut Parser<'i, '_>,
    ) -> Result<&'i str, ParseError<'i, MathError>> {
        let start = input.position();
        if matches!(
            input.next()?,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        ) {
            input
                .parse_nested_block(crate::style::parser::consume)
                .map_err(ParseError::into::<MathError>)?;
        }
        Ok(input.slice_from(start).trim())
    }
    fn mix_easing<'i>(
        &mut self,
        input: &mut Parser<'i, '_>,
    ) -> Result<crate::paint::Easing, ParseError<'i, MathError>> {
        let source = Self::component_source(input)?;
        crate::style::easing::parse(source, self.context.relative)
            .map_err(|error| input.new_custom_error(MathError::Resolution(error)))?
            .ok_or_else(|| {
                input.new_custom_error(MathError::Invalid("invalid easing function".into()))
            })
    }
    fn calc_mix<'i>(&mut self, input: &mut Parser<'i, '_>) -> MathResult<'i> {
        let (mut progress, easing, timeline) =
            input.parse_until_before(Delimiter::Comma, |input| {
                let mut easing = if input
                    .try_parse(|input| input.expect_ident_matching("by"))
                    .is_ok()
                {
                    Some(self.mix_easing(input)?)
                } else {
                    None
                };
                let mut timeline = false;
                let progress = if let Ok(percentage) = input.try_parse(Parser::expect_percentage) {
                    Calculation::Value(unsigned_zero(percentage))
                } else {
                    let state = input.state();
                    let numeric = matches!(input.next(), Ok(Token::Number { .. })) || {
                        input.reset(&state);
                        matches!(input.next(),Ok(Token::Function(name)) if is_math_function(name))
                    };
                    input.reset(&state);
                    if numeric {
                        let (progress, kind) = self.value(input)?;
                        if kind != CalcType::NUMBER {
                            return Self::invalid(input, "calc-mix() progress must be a number");
                        }
                        progress
                    } else {
                        let source = Self::component_source(input)?;
                        let mut steps = self.context.relative.work.steps();
                        let value = crate::style::matcher::match_value(
                            crate::style::grammar::Property::AnimationTimeline
                                .data()
                                .root,
                            source,
                            &mut steps,
                            crate::style::matcher::ValueContext::Nested(self.context.relative),
                        )
                        .map_err(|error| input.new_custom_error(MathError::Resolution(error)))?;
                        let Some(value) = value else {
                            return Self::invalid(input, "invalid interpolation progress");
                        };
                        let [value] = value.items() else {
                            return Self::invalid(input, "one timeline is required");
                        };
                        if matches!(value.keyword(), Some(Keyword::None | Keyword::Auto)) {
                            return Self::invalid(
                                input,
                                "auto and none cannot supply interpolation progress",
                            );
                        }
                        timeline = true;
                        Calculation::Value(0.0)
                    }
                };
                if input
                    .try_parse(|input| input.expect_ident_matching("by"))
                    .is_ok()
                {
                    if easing.is_some() {
                        return Self::invalid(input, "duplicate easing function");
                    }
                    easing = Some(self.mix_easing(input)?);
                }
                input.expect_exhausted()?;
                Ok((progress, easing, timeline))
            })?;
        input.expect_comma()?;
        let (start, first_kind) =
            input.parse_until_before(Delimiter::Comma, |input| self.sum(input))?;
        input.expect_comma()?;
        let (end, second_kind) = self.sum(input)?;
        let kind = Self::unify(input, [first_kind, second_kind].into_iter())?;
        if timeline && !self.context.relative.grammar_only {
            return Err(
                input.new_custom_error(MathError::Resolution(StyleError::Unsupported(
                    "animation timeline interpolation is outside the static print profile".into(),
                ))),
            );
        }
        if let Some(easing) = easing {
            progress = Calculation::eased(easing, progress);
        }
        Ok((Calculation::mix(progress, start, end), kind))
    }
    fn media_progress<'i>(&mut self, input: &mut Parser<'i, '_>) -> MathResult<'i> {
        let name = input.expect_ident()?.to_ascii_lowercase();
        let paper = self.context.relative.paper;
        let Some(feature) = crate::style::media::range_feature(&name, paper) else {
            return Self::invalid(input, "media-progress() requires a range media feature");
        };
        input.expect_comma()?;
        let mut relative = Relative::media(paper, self.context.relative.initial_font.clone());
        relative.work = self.context.relative.work.clone();
        let mut parser = MathParser {
            context: Context {
                relative: &relative,
                percentage: None,
                channels: &[],
                anchors: None,
            },
            depth: self.depth,
            size: false,
        };
        let mut endpoint = |input: &mut Parser<'i, '_>| {
            let zero = input
                .try_parse(|input| {
                    let value = input.expect_number()?;
                    if value != 0.0 {
                        return Err(input.new_basic_unexpected_token_error(Token::Number {
                            has_sign: false,
                            value,
                            int_value: None,
                        }));
                    }
                    input.expect_exhausted()
                })
                .is_ok();
            let (calculation, kind) = if zero {
                (
                    Calculation::Value(0.0),
                    if feature.kind == Kind::Length {
                        CalcType::of(Kind::Length)
                    } else {
                        CalcType::NUMBER
                    },
                )
            } else {
                parser.sum(input)?
            };
            if kind != CalcType::of(feature.kind) {
                return Self::invalid(input, "invalid media-progress() feature value type");
            }
            let math = Math {
                kind: Some(feature.kind),
                calculation,
            };
            let Some(calculation) = feature.calculation(&math) else {
                return Self::invalid(input, "invalid media-progress() feature value");
            };
            Ok(calculation)
        };
        let start = input.parse_until_before(Delimiter::Comma, &mut endpoint)?;
        input.expect_comma()?;
        let end = endpoint(input)?;
        Ok((
            Calculation::progress(Calculation::Value(feature.actual), start, end),
            CalcType::NUMBER,
        ))
    }
    /// Type-checks the comma-separated arguments of every other math function.
    fn apply<'i>(
        name: &str,
        mut items: Vec<Calculation>,
        kind: CalcType,
        input: &Parser<'i, '_>,
    ) -> MathResult<'i> {
        let count = items.len();
        let mut take = || items.remove(0);
        let angle = CalcType::base(Base::Angle);
        let radians = |argument: Calculation| {
            if kind == angle {
                argument.scale(1.0_f32.to_radians())
            } else {
                argument
            }
        };
        let number = kind == CalcType::NUMBER;
        Ok(match (name, count) {
            ("min", _) => (Calculation::Min(items).simplify(), kind),
            ("max", _) => (Calculation::Max(items).simplify(), kind),
            ("hypot", _) => (Calculation::Hypot(items).simplify(), kind),
            ("progress", 3) => (
                Calculation::progress(take(), take(), take()),
                CalcType::NUMBER,
            ),
            ("mod", 2) => (
                Calculation::binary(BinaryFunction::Mod, take(), take()),
                kind,
            ),
            ("rem", 2) => (
                Calculation::binary(BinaryFunction::Rem, take(), take()),
                kind,
            ),
            ("atan2", 2) => (
                Calculation::binary(BinaryFunction::Atan2, take(), take())
                    .scale(1.0_f32.to_degrees()),
                angle,
            ),
            ("pow", 2) if number => (
                Calculation::binary(BinaryFunction::Pow, take(), take()),
                kind,
            ),
            ("log", 1) if number => (Calculation::unary(UnaryFunction::Ln, take()), kind),
            ("log", 2) if number => (
                Calculation::binary(BinaryFunction::Log, take(), take()),
                kind,
            ),
            ("abs", 1) => (Calculation::unary(UnaryFunction::Abs, take()), kind),
            ("sign", 1) => (
                Calculation::unary(UnaryFunction::Sign, take()),
                CalcType::NUMBER,
            ),
            ("sin" | "cos" | "tan", 1) if number || kind == angle => {
                let function = match name {
                    "sin" => UnaryFunction::Sin,
                    "cos" => UnaryFunction::Cos,
                    _ => UnaryFunction::Tan,
                };
                (
                    Calculation::unary(function, radians(take())),
                    CalcType::NUMBER,
                )
            }
            ("asin" | "acos" | "atan", 1) if number => {
                let function = match name {
                    "asin" => UnaryFunction::Asin,
                    "acos" => UnaryFunction::Acos,
                    _ => UnaryFunction::Atan,
                };
                (
                    Calculation::unary(function, take()).scale(1.0_f32.to_degrees()),
                    angle,
                )
            }
            ("sqrt", 1) if number => (Calculation::unary(UnaryFunction::Sqrt, take()), kind),
            ("exp", 1) if number => (Calculation::unary(UnaryFunction::Exp, take()), kind),
            _ => {
                return Self::invalid(
                    input,
                    format!("{name}() does not accept {count} {kind:?} arguments"),
                );
            }
        })
    }
    fn arguments<'i>(
        &mut self,
        input: &mut Parser<'i, '_>,
    ) -> Result<(Vec<Calculation>, CalcType), ParseError<'i, MathError>> {
        let typed = input.parse_comma_separated(|input| self.sum(input))?;
        let kind = Self::unify(input, typed.iter().map(|(_, kind)| *kind))?;
        Ok((typed.into_iter().map(|(item, _)| item).collect(), kind))
    }
    fn unify<'i>(
        input: &Parser<'i, '_>,
        mut kinds: impl Iterator<Item = CalcType>,
    ) -> Result<CalcType, ParseError<'i, MathError>> {
        let first = kinds
            .next()
            .ok_or_else(|| input.new_custom_error(MathError::Invalid("missing argument".into())))?;
        if kinds.any(|kind| kind != first) {
            return Self::invalid(input, "arguments have mismatched types");
        }
        Ok(first)
    }
    fn clamp<'i>(&mut self, input: &mut Parser<'i, '_>) -> MathResult<'i> {
        let bounds = input.parse_comma_separated(|input| {
            if input
                .try_parse(|input| input.expect_ident_matching(NONE))
                .is_ok()
            {
                Ok(None)
            } else {
                self.sum(input).map(Some)
            }
        })?;
        let Ok([minimum, Some((value, kind)), maximum]) = <[_; CLAMP_ARGUMENTS]>::try_from(bounds)
        else {
            return Self::invalid(
                input,
                "clamp() takes a minimum or none, a value, and a maximum or none",
            );
        };
        let kind = Self::unify(
            input,
            [&minimum, &maximum]
                .into_iter()
                .flatten()
                .map(|(_, kind)| *kind)
                .chain([kind]),
        )?;
        let mut result = value;
        if let Some((maximum, _)) = maximum {
            result = Calculation::Min(vec![result, maximum]).simplify();
        }
        if let Some((minimum, _)) = minimum {
            result = Calculation::Max(vec![minimum, result]).simplify();
        }
        Ok((result, kind))
    }
    fn round<'i>(&mut self, input: &mut Parser<'i, '_>) -> MathResult<'i> {
        let strategy = input
            .try_parse(|input| {
                let name = input.expect_ident()?.to_ascii_lowercase();
                let strategy = match name.as_str() {
                    "nearest" => RoundingStrategy::Nearest,
                    "up" => RoundingStrategy::Up,
                    "down" => RoundingStrategy::Down,
                    "to-zero" => RoundingStrategy::ToZero,
                    _ => return Err(input.new_custom_error::<_, ()>(())),
                };
                input.expect_comma()?;
                Ok(strategy)
            })
            .unwrap_or(RoundingStrategy::Nearest);
        let (mut items, kind) = self.arguments(input)?;
        match items.len() {
            1 if kind == CalcType::NUMBER => items.push(Calculation::Value(1.0)),
            2 => {}
            _ => {
                return Self::invalid(
                    input,
                    "round() takes a value and a step; only numbers may omit the step",
                );
            }
        }
        let value = items.remove(0);
        let step = items.remove(0);
        Ok((
            Calculation::binary(BinaryFunction::Round(strategy), value, step),
            kind,
        ))
    }
    /// The anchor name and side or size before an anchor function's fallback.
    fn anchor_query<'i>(
        size: bool,
        input: &mut Parser<'i, '_>,
    ) -> Result<AnchorQuery, ParseError<'i, MathError>> {
        input.parse_until_before(Delimiter::Comma, |input| {
            let mut name = None;
            let mut side = None;
            let mut dimension = None;
            while !input.is_exhausted() {
                let token = input.next()?.clone();
                match token {
                    Token::Ident(ident) if ident.starts_with("--") && name.is_none() => {
                        name = Some(std::sync::Arc::from(ident.as_ref()));
                    }
                    Token::Ident(ident) if size && dimension.is_none() => {
                        let Some(found) = AnchorDimension::from_name(&ident) else {
                            return Self::invalid(input, format!("unknown anchor size {ident}"));
                        };
                        dimension = Some(found);
                    }
                    Token::Ident(ident) if !size && side.is_none() => {
                        let Some(found) = AnchorSide::from_name(&ident) else {
                            return Self::invalid(input, format!("unknown anchor side {ident}"));
                        };
                        side = Some(found);
                    }
                    Token::Percentage { unit_value, .. } if !size && side.is_none() => {
                        side = Some(AnchorSide::Fraction(unsigned_zero(unit_value)));
                    }
                    token => return Err(input.new_unexpected_token_error(token)),
                }
            }
            Ok(if size {
                AnchorQuery::Size { name, dimension }
            } else {
                let Some(side) = side else {
                    return Self::invalid(input, "anchor() requires an anchor side");
                };
                AnchorQuery::Anchor { name, side }
            })
        })
    }
    /// CSS Anchor Positioning 1 §3.2 `anchor()` and §5.1 `anchor-size()`: a `<length>` that
    /// the element's anchors resolve, else the fallback; one unresolvable without a fallback
    /// is recorded, making the declaration invalid at computed-value time.
    fn anchor_function<'i>(&mut self, size: bool, input: &mut Parser<'i, '_>) -> MathResult<'i> {
        // CSS Values 4 §2.6: omitting `anchor-size()`'s optional query also omits the comma
        // before its fallback, as in `anchor-size(12pt)`.
        let fallback_only = size && {
            let start = input.state();
            let query = matches!(input.next(), Ok(Token::Ident(_)) | Err(_));
            input.reset(&start);
            !query
        };
        let query = if fallback_only {
            AnchorQuery::Size {
                name: None,
                dimension: None,
            }
        } else {
            Self::anchor_query(size, input)?
        };
        let context = self.context.anchors.and_then(|anchors| anchors.context);
        let resolved = context.and_then(|context| context.resolve(&query));
        let checkpoint = context.map(crate::style::anchor_values::AnchorContext::checkpoint);
        let fallback = if fallback_only || input.try_parse(Parser::expect_comma).is_ok() {
            let unitless_zero = input
                .try_parse(|input| {
                    if input.expect_number()? != 0.0 {
                        return Err(input.new_custom_error(()));
                    }
                    input.expect_exhausted()?;
                    Ok::<_, ParseError<'_, ()>>(())
                })
                .is_ok();
            let (fallback, kind) = if unitless_zero {
                (Calculation::Value(0.0), CalcType::of(Kind::Length))
            } else {
                self.sum(input)?
            };
            if kind != CalcType::of(Kind::Length) {
                return Self::invalid(
                    input,
                    "an anchor function fallback is a <length-percentage>",
                );
            }
            Some(fallback)
        } else {
            None
        };
        input.expect_exhausted()?;
        let length = CalcType::of(Kind::Length);
        let Some(context) = context else {
            return Ok((Calculation::Value(0.0), length));
        };
        if resolved.is_some()
            && let Some(checkpoint) = checkpoint
        {
            context.restore(checkpoint);
        }
        Ok(match (resolved, fallback) {
            (Some(value), _) => (Calculation::Value(value), length),
            (None, Some(fallback)) => (fallback, length),
            (None, None) => {
                context.fail();
                (Calculation::Value(0.0), length)
            }
        })
    }
    /// `+` and `-` require surrounding whitespace.
    fn sum<'i>(&mut self, input: &mut Parser<'i, '_>) -> MathResult<'i> {
        let (first, kind) = self.product(input)?;
        let mut terms = vec![first];
        while let Ok(negative) = input.try_parse(|input| -> Result<bool, BasicParseError<'i>> {
            input.expect_whitespace()?;
            let negative = match input.next_including_whitespace()? {
                Token::Delim('+') => false,
                Token::Delim('-') => true,
                token => {
                    let token = token.clone();
                    return Err(input.new_basic_unexpected_token_error(token));
                }
            };
            input.expect_whitespace()?;
            Ok(negative)
        }) {
            let (term, term_kind) = self.product(input)?;
            if term_kind != kind {
                return Self::invalid(input, format!("cannot add {kind:?} and {term_kind:?}"));
            }
            terms.push(if negative { term.negate() } else { term });
        }
        Ok((Calculation::Sum(terms).simplify(), kind))
    }
    /// Products and quotients multiply types; only the top-level result is type-checked.
    fn product<'i>(&mut self, input: &mut Parser<'i, '_>) -> MathResult<'i> {
        let (first, mut kind) = self.value(input)?;
        let mut factors = vec![first];
        while let Ok(divide) = input.try_parse(|input| -> Result<bool, BasicParseError<'i>> {
            match input.next()? {
                Token::Delim('*') => Ok(false),
                Token::Delim('/') => Ok(true),
                token => {
                    let token = token.clone();
                    Err(input.new_basic_unexpected_token_error(token))
                }
            }
        }) {
            let (factor, factor_kind) = self.value(input)?;
            if divide {
                kind = kind.multiply(factor_kind.invert());
                factors.push(Calculation::Invert(Box::new(factor)).simplify());
            } else {
                kind = kind.multiply(factor_kind);
                factors.push(factor);
            }
        }
        Ok((Calculation::Product(factors).simplify(), kind))
    }
    fn value<'i>(&mut self, input: &mut Parser<'i, '_>) -> MathResult<'i> {
        self.context
            .relative
            .work
            .charge(1)
            .map_err(|error| input.new_custom_error(MathError::Resolution(error)))?;
        let token = input.next()?.clone();
        match token {
            Token::Number { value, .. } => {
                Ok((Calculation::Value(unsigned_zero(value)), CalcType::NUMBER))
            }
            Token::Percentage { unit_value, .. } => match self.context.percentage {
                Some(kind @ (Kind::Length | Kind::Angle)) => Ok((
                    Calculation::percentage(unsigned_zero(unit_value)),
                    CalcType::of(kind),
                )),
                Some(kind) => Ok((
                    Calculation::Value(unsigned_zero(unit_value)),
                    CalcType::of(kind),
                )),
                None => Ok((
                    Calculation::Value(unsigned_zero(unit_value)),
                    CalcType::of(Kind::Percentage),
                )),
            },
            Token::Dimension { value, unit, .. } => {
                let value = unsigned_zero(value);
                let (canonical, base) =
                    match units::dimension(&unit) {
                        Some(Dimension::Length(length)) => (
                            value
                                * self.context.relative.per_unit(length.scale).map_err(
                                    |error| input.new_custom_error(MathError::Resolution(error)),
                                )?,
                            Base::Length,
                        ),
                        Some(Dimension::Angle(degrees)) => (value * degrees, Base::Angle),
                        Some(Dimension::Time(seconds)) => (value * seconds, Base::Time),
                        Some(Dimension::Frequency(hertz)) => (value * hertz, Base::Frequency),
                        Some(Dimension::Resolution(dppx)) => (value * dppx, Base::Resolution),
                        Some(Dimension::Flex) | None => {
                            return Self::invalid(input, format!("unsupported unit: {unit}"));
                        }
                    };
                Ok((Calculation::Value(canonical), CalcType::base(base)))
            }
            Token::Ident(name)
                if self.size && name.eq_ignore_ascii_case(crate::paint::SIZE_KEYWORD) =>
            {
                Ok((Calculation::Size, CalcType::of(Kind::Length)))
            }
            Token::Ident(name) => {
                if let Some((_, value)) = self
                    .context
                    .channels
                    .iter()
                    .find(|(channel, _)| name.eq_ignore_ascii_case(channel))
                {
                    return Ok((Calculation::Value(*value), CalcType::NUMBER));
                }
                let value = match &*name.to_ascii_lowercase() {
                    "e" => std::f32::consts::E,
                    "pi" => std::f32::consts::PI,
                    "infinity" => f32::INFINITY,
                    "-infinity" => f32::NEG_INFINITY,
                    "nan" => f32::NAN,
                    _ => return Self::invalid(input, format!("unknown keyword: {name}")),
                };
                Ok((Calculation::Value(value), CalcType::NUMBER))
            }
            Token::ParenthesisBlock => self.nested(input, Self::sum),
            Token::Function(name)
                if self
                    .context
                    .anchors
                    .is_some_and(|anchors| anchors.functions.allows(&name)) =>
            {
                let size = name.eq_ignore_ascii_case(ANCHOR_SIZE_FUNCTION);
                self.nested(input, |parser, input| parser.anchor_function(size, input))
            }
            Token::Function(name) if is_math_function(&name) => {
                self.function(&name.to_ascii_lowercase(), input)
            }
            token => Err(input.new_unexpected_token_error(token)),
        }
    }
}

/// A `calc-size()` basis after canonicalization (CSS Values 5 §10.1): an
/// `<intrinsic-size-keyword>`, `any`, or the `100%` that replaces a basis with percentages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CalcSizeBasis {
    Any,
    Percentage,
    Auto,
    MinContent,
    MaxContent,
    FitContent,
    Stretch,
}

/// A canonical `calc-size()`: its basis and its calculation in points, where `size` is the
/// basis's size.
#[derive(Clone, Debug)]
pub(crate) struct CalcSize {
    pub basis: CalcSizeBasis,
    pub calculation: Calculation,
}

/// Parses a whole `calc-size()` function, whose basis may be any `<intrinsic-size-keyword>`.
pub(super) fn calc_size(
    input: &mut Parser<'_, '_>,
    context: Context<'_>,
) -> Result<CalcSize, MathError> {
    let _work = context.relative.work.enter(context.relative.grammar_only);
    let mut parser = MathParser {
        context,
        depth: 0,
        size: false,
    };
    input
        .parse_entirely(|input| parser.calc_size(input))
        .map_err(|error| match error.kind {
            ParseErrorKind::Custom(error) => error,
            ParseErrorKind::Basic(error) => MathError::Invalid(format!("{error:?}")),
        })
}

/// A `calc-size()` basis as written.
enum GivenBasis {
    Keyword(CalcSizeBasis),
    Nested(CalcSize),
    Sum(Calculation),
}

impl MathParser<'_> {
    fn calc_size<'i>(
        &mut self,
        input: &mut Parser<'i, '_>,
    ) -> Result<CalcSize, ParseError<'i, MathError>> {
        input.expect_function_matching(CALC_SIZE_FUNCTION)?;
        self.nested(input, |parser, input| {
            let outer = parser.size;
            parser.size = false;
            let result = parser.calc_size_arguments(input);
            parser.size = outer;
            result
        })
    }
    fn calc_size_arguments<'i>(
        &mut self,
        input: &mut Parser<'i, '_>,
    ) -> Result<CalcSize, ParseError<'i, MathError>> {
        let basis = self.calc_size_basis(input)?;
        input.expect_comma()?;
        self.size = !matches!(basis, GivenBasis::Keyword(CalcSizeBasis::Any));
        let (calculation, kind) = self.sum(input)?;
        if kind != CalcType::of(Kind::Length) {
            return Self::invalid(input, "a calc-size() calculation must be a length");
        }
        let canonical = match basis {
            GivenBasis::Keyword(basis) => CalcSize { basis, calculation },
            GivenBasis::Nested(inner) => CalcSize {
                basis: inner.basis,
                calculation: calculation.substitute_size(&inner.calculation),
            },
            GivenBasis::Sum(sum)
                if sum.any_leaf(&|leaf| matches!(leaf, Calculation::Percentage(_))) =>
            {
                CalcSize {
                    basis: CalcSizeBasis::Percentage,
                    calculation: calculation.substitute_size(&sum.depercentify()),
                }
            }
            GivenBasis::Sum(sum) => CalcSize {
                basis: CalcSizeBasis::Any,
                calculation: calculation.substitute_size(&sum),
            },
        };
        if canonical.calculation.node_count() > MAX_CALC_SIZE_NODES {
            return Self::invalid(input, "calc-size() substitution exceeds its limit");
        }
        Ok(canonical)
    }
    fn calc_size_basis<'i>(
        &mut self,
        input: &mut Parser<'i, '_>,
    ) -> Result<GivenBasis, ParseError<'i, MathError>> {
        if let Ok(name) = input.try_parse(Parser::expect_ident_cloned) {
            let basis = match Keyword::from_ident(&name) {
                Some(Keyword::Any) => CalcSizeBasis::Any,
                Some(Keyword::Auto) => CalcSizeBasis::Auto,
                Some(Keyword::MinContent) => CalcSizeBasis::MinContent,
                Some(Keyword::MaxContent) => CalcSizeBasis::MaxContent,
                Some(Keyword::FitContent) => CalcSizeBasis::FitContent,
                Some(Keyword::Stretch) => CalcSizeBasis::Stretch,
                _ => return Self::invalid(input, format!("invalid calc-size() basis: {name}")),
            };
            return Ok(GivenBasis::Keyword(basis));
        }
        if let Ok(inner) = input.try_parse(|input| self.calc_size(input)) {
            return Ok(GivenBasis::Nested(inner));
        }
        let (sum, kind) = self.sum(input)?;
        if kind != CalcType::of(Kind::Length) {
            return Self::invalid(input, "a calc-size() basis must be a length-percentage");
        }
        Ok(GivenBasis::Sum(sum))
    }
}

/// CSS numeric tokens and property-boundary zeros are unsigned.
fn unsigned_zero(value: f32) -> f32 {
    if value == 0.0 { 0.0 } else { value }
}
