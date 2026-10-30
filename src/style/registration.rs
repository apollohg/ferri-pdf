//! Registered custom properties (CSS Properties and Values API 1): `@property` rules (§3) and
//! the computed values their syntaxes give (§2.4).
use crate::paint::{BinaryFunction, Calculation, Easing, LengthPercentage, RoundingStrategy};
use crate::paint::{StepPosition, UnaryFunction};
use crate::resources::ResourceBase;
use crate::style::{
    POINTS_PER_PIXEL, StyleError,
    box_values::length_percentage,
    calc::Kind,
    component::Component,
    grammar::Base,
    matcher::{self, ValueContext},
    parser::{Declaration, absolute_url_text},
    syntax::{Atom, List, Syntax},
    units::{self, Dimension, FontUnit, Length, LengthUnit, Relative, Scale},
    values,
    variables::Expanded,
};
use cssparser::{Parser, ParserInput, Token};
use std::{collections::HashMap, fmt::Write};

const SYNTAX: &str = "syntax";
const INHERITS: &str = "inherits";
const INITIAL_VALUE: &str = "initial-value";
const TRUE: &str = "true";
const FALSE: &str = "false";
const PIXELS: LengthUnit = LengthUnit {
    name: "px",
    scale: Scale::Pixels,
};
/// Tree-counting functions resolve against the element (CSS Values 5 §9).
const ELEMENT_FUNCTIONS: [&str; 2] = ["sibling-index", "sibling-count"];
const PERCENT: f32 = 100.0;

/// A custom property registration (§2.1).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Registration {
    pub(super) syntax: Syntax,
    pub(super) inherits: bool,
    /// The computed initial value; `None` is the guaranteed-invalid value.
    pub(super) initial: Option<Expanded>,
}
impl Registration {
    /// Whether values compute by a syntax other than the universal one.
    pub(super) fn typed(&self) -> bool {
        !matches!(self.syntax, Syntax::Universal)
    }
}

/// The document's registrations by custom property name.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Registrations(HashMap<String, Registration>);
impl Registrations {
    pub(super) fn get(&self, name: &str) -> Option<&Registration> {
        self.0.get(name)
    }
    /// The registration of `name` when its values compute by a typed syntax.
    pub(super) fn typed(&self, name: &str) -> Option<&Registration> {
        self.get(name).filter(|registration| registration.typed())
    }
    pub(super) fn non_inherited(&self) -> impl Iterator<Item = (&str, &Registration)> {
        self.0
            .iter()
            .filter(|(_, registration)| !registration.inherits)
            .map(|(name, registration)| (name.as_str(), registration))
    }
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Registers `name`; the last valid rule in document order wins (§2.1).
    pub(super) fn insert(&mut self, name: String, registration: Registration) {
        self.0.insert(name, registration);
    }
}

/// The registration an `@property` rule's descriptors give, or `None` when the rule is invalid
/// (§3). Invalid and unknown descriptors are returned for reporting; they do not invalidate
/// the rule.
pub(super) fn parse<'d>(
    declarations: &'d [Declaration],
    base: &ResourceBase,
    name: &str,
) -> Result<(Option<Registration>, Vec<&'d Declaration>), StyleError> {
    let (mut syntax, mut inherits, mut initial) = (None, None, None);
    let mut invalid = Vec::new();
    for declaration in declarations {
        let accepted = !declaration.important
            && match declaration.name.as_str() {
                SYNTAX => syntax_descriptor(&declaration.value)?
                    .map(|value| syntax = Some(value))
                    .is_some(),
                INHERITS => inherits_descriptor(&declaration.value)
                    .map(|value| inherits = Some(value))
                    .is_some(),
                INITIAL_VALUE => {
                    initial = Some(declaration.value.as_str());
                    true
                }
                _ => false,
            };
        if !accepted {
            invalid.push(declaration);
        }
    }
    let (Some(syntax), Some(inherits)) = (syntax, inherits) else {
        return Ok((None, invalid));
    };
    let initial = match (&syntax, initial) {
        (Syntax::Universal, None) => None,
        (Syntax::Universal, Some(source)) => Some(Expanded::literal(source.to_owned())?),
        (Syntax::Alternatives(_), None) => return Ok((None, invalid)),
        (Syntax::Alternatives(_), Some(source)) => {
            match independent_initial(&syntax, source, base, name)? {
                Some(value) => Some(value),
                None => return Ok((None, invalid)),
            }
        }
    };
    Ok((
        Some(Registration {
            syntax,
            inherits,
            initial,
        }),
        invalid,
    ))
}

/// The `syntax` descriptor: a `<string>` holding a syntax definition (§3.1).
fn syntax_descriptor(source: &str) -> Result<Option<Syntax>, StyleError> {
    let mut raw = ParserInput::new(source);
    let input = &mut Parser::new(&mut raw);
    let Ok(definition) = input.parse_entirely(|input| {
        input
            .expect_string_cloned()
            .map_err(cssparser::ParseError::<()>::from)
    }) else {
        return Ok(None);
    };
    let mut raw = ParserInput::new(&definition);
    Syntax::parse(&mut Parser::new(&mut raw), 0)
}

/// The `inherits` descriptor: `true | false` (§3.2).
fn inherits_descriptor(source: &str) -> Option<bool> {
    let mut raw = ParserInput::new(source);
    let input = &mut Parser::new(&mut raw);
    let keyword = input
        .parse_entirely(|input| {
            input
                .expect_ident_cloned()
                .map_err(cssparser::ParseError::<()>::from)
        })
        .ok()?;
    [(TRUE, true), (FALSE, false)]
        .into_iter()
        .find_map(|(name, value)| keyword.eq_ignore_ascii_case(name).then_some(value))
}

/// A typed `initial-value`, which must parse by the syntax and be computationally independent
/// (§3.3): computable from the value and global information alone, which excludes font-,
/// viewport- and container-relative lengths, element-dependent colours and tree-counting
/// functions (§4.1).
fn independent_initial(
    syntax: &Syntax,
    source: &str,
    base: &ResourceBase,
    name: &str,
) -> Result<Option<Expanded>, StyleError> {
    if depends_on_element(source) {
        return Ok(None);
    }
    compute(
        syntax,
        &Expanded::literal(source.to_owned())?,
        &Relative::initial(),
        Some(base),
        name,
    )
}

fn depends_on_element(source: &str) -> bool {
    let mut dependent = false;
    visit_tokens(source, &mut |token| {
        dependent |= match token {
            Token::Dimension { unit, .. } => matches!(
                units::dimension(unit),
                Some(Dimension::Length(LengthUnit {
                    scale: Scale::Font(_)
                        | Scale::Root(_)
                        | Scale::Viewport(_)
                        | Scale::ViewportLogical(_)
                        | Scale::Container(_)
                        | Scale::ContainerLogical(_),
                    ..
                }))
            ),
            Token::Function(name) => ELEMENT_FUNCTIONS
                .iter()
                .any(|function| name.eq_ignore_ascii_case(function)),
            _ => false,
        };
    });
    dependent
}

/// Every token of `source`, including those inside blocks and functions.
fn visit_tokens(source: &str, visit: &mut impl FnMut(&Token<'_>)) {
    fn scan(input: &mut Parser<'_, '_>, visit: &mut impl FnMut(&Token<'_>)) {
        while let Ok(token) = input.next().cloned() {
            visit(&token);
            if matches!(
                token,
                Token::Function(_)
                    | Token::ParenthesisBlock
                    | Token::SquareBracketBlock
                    | Token::CurlyBracketBlock
            ) {
                let _ = input.parse_nested_block(|nested| {
                    scan(nested, visit);
                    Ok::<_, cssparser::ParseError<'_, ()>>(())
                });
            }
        }
    }
    let mut raw = ParserInput::new(source);
    scan(&mut Parser::new(&mut raw), visit);
}

/// The element properties a registered value's font-relative lengths depend on, which add
/// dependency edges (§2.7.2); on the root element, root-relative units name its own font.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct FontDependencies {
    pub(super) font_size: bool,
    pub(super) line_height: bool,
}
pub(super) fn font_dependencies(source: &str, root: bool) -> FontDependencies {
    let mut dependencies = FontDependencies::default();
    visit_tokens(source, &mut |token| {
        if let Token::Dimension { unit, .. } = token
            && let Some(Dimension::Length(LengthUnit { scale, .. })) = units::dimension(unit)
        {
            let unit = match scale {
                Scale::Font(unit) => Some(unit),
                Scale::Root(unit) if root => Some(unit),
                _ => None,
            };
            if let Some(unit) = unit {
                dependencies.font_size = true;
                dependencies.line_height |= unit == FontUnit::Lh;
            }
        }
    });
    dependencies
}

/// The computed value of `value` by `syntax` (§2.4), or `None` when it does not parse by the
/// syntax and is invalid at computed-value time. Relative URLs resolve against `base`, the
/// declaring stylesheet's location; `name` is the registered property, for diagnostics. A
/// value substituted from an attribute stays attribute-tainted, and cannot supply a URL (CSS
/// Values 5 §7.7).
pub(super) fn compute(
    syntax: &Syntax,
    value: &Expanded,
    relative: &Relative<'_>,
    base: Option<&ResourceBase>,
    name: &str,
) -> Result<Option<Expanded>, StyleError> {
    let Syntax::Alternatives(clauses) = syntax else {
        return Ok(Some(value.clone()));
    };
    let mut steps = 0;
    if value.has_taint() {
        let origins = matcher::Origins::new(value);
        let matched = matcher::match_clause(
            clauses,
            value.as_str(),
            &mut steps,
            ValueContext::Substituted(&origins),
        )?;
        if matched.is_none() || origins.invalid_url() {
            relative.work.charge(steps)?;
            return Ok(None);
        }
    }
    let tainted = value.has_taint();
    let value = value.as_str();
    let resolved;
    let value = match base {
        Some(base)
            if clauses.iter().any(|clause| {
                matches!(
                    clause.atom,
                    Atom::Base(Base::Url)
                        | Atom::Production(crate::style::grammar::Production::Image)
                )
            }) =>
        {
            resolved = absolute_url_text(value, base, name)?;
            resolved.as_deref().unwrap_or(value)
        }
        _ => value,
    };
    let matched = matcher::match_clause(clauses, value, &mut steps, ValueContext::Nested(relative));
    relative.work.charge(steps)?;
    let Some((clause, matched)) = matched? else {
        return Ok(None);
    };
    let integer = clause.atom == Atom::Base(Base::Integer);
    let item = |component: &Component| -> Result<String, StyleError> {
        Ok(if integer {
            values::integer(component, relative)?.to_string()
        } else {
            computed(component, relative)?.to_string()
        })
    };
    let serialized = match clause.list {
        List::Once => item(&matched)?,
        list => matched
            .items()
            .iter()
            .filter(|component| !matches!(component, Component::Delim(_)))
            .map(item)
            .collect::<Result<Vec<_>, _>>()?
            .join(if list == List::Comma { ", " } else { " " }),
    };
    let computed = Expanded::literal(serialized)?;
    Ok(Some(if tainted {
        computed.attribute_tainted()
    } else {
        computed
    }))
}

/// `value` with lengths in canonical pixels, math functions resolved except for their
/// percentage terms, and colours that do not depend on `currentcolor` made absolute
/// (CSS Values 4 §4.4, CSS Color 4 §14).
fn computed(value: &Component, relative: &Relative<'_>) -> Result<Component, StyleError> {
    Ok(match value {
        Component::Length(length) => pixels(length.points(relative)?),
        Component::Math(_, Kind::Length, range) => match length_percentage(value, relative)? {
            LengthPercentage::Length(points) => pixels(points),
            LengthPercentage::Percentage(fraction) => Component::Percentage(fraction),
            LengthPercentage::Calc(calculation) => Component::Math(
                serialize_calculation(&calculation).into(),
                Kind::Length,
                *range,
            ),
        },
        Component::Math(_, Kind::Percentage, _) => {
            Component::Percentage(values::percentage(value, relative)?)
        }
        Component::Math(source, kind, range) => {
            let resolved = values::math(source, relative, *kind, None)?.resolve(0.0, *range);
            match kind {
                Kind::Number => Component::Number(resolved),
                Kind::Angle => Component::Angle(resolved),
                Kind::Time => Component::Time(resolved),
                Kind::Resolution => Component::Resolution(resolved),
                Kind::Frequency => Component::Dimension(resolved, FREQUENCY_UNIT.into()),
                Kind::Length | Kind::Percentage => unreachable!("matched above"),
            }
        }
        Component::MathColor(_) if crate::style::resolution::contains_current_color(value) => {
            value.clone()
        }
        Component::MathColor(_) => Component::Color(crate::style::colors::absolute(
            value,
            relative,
            &crate::paint::AbsoluteColor::BLACK,
        )?),
        Component::Reference(label, inner) => {
            Component::Reference(*label, Box::new(computed(inner, relative)?))
        }
        Component::Function(name, body) => Component::Function(
            *name,
            body.as_deref()
                .map(|body| computed(body, relative).map(Box::new))
                .transpose()?,
        ),
        Component::Block(kind, body) => {
            Component::Block(*kind, Box::new(computed(body, relative)?))
        }
        Component::Group(items) => Component::Group(
            items
                .iter()
                .map(|item| computed(item, relative))
                .collect::<Result<_, _>>()?,
        ),
        Component::Slots(slots) => Component::Slots(
            slots
                .iter()
                .map(|slot| {
                    slot.as_ref()
                        .map(|slot| computed(slot, relative))
                        .transpose()
                })
                .collect::<Result<_, _>>()?,
        ),
        other => other.clone(),
    })
}
const FREQUENCY_UNIT: &str = "hz";

fn pixels(points: f32) -> Component {
    Component::Length(Length {
        value: points / POINTS_PER_PIXEL,
        unit: PIXELS,
    })
}

/// A `<length-percentage>` calculation whose percentage terms await their basis, as a math
/// function. Leaves are points where the node is a length and plain numbers elsewhere; trig
/// functions take and return radians.
fn serialize_calculation(calculation: &Calculation) -> String {
    let mut output = String::from("calc(");
    write_calculation(&mut output, calculation, true);
    output.push(')');
    output
}

/// Whether `node` is a length: it has a percentage term outside functions that yield
/// numbers.
fn is_length(node: &Calculation) -> bool {
    match node {
        Calculation::Value(_) | Calculation::Invert(_) | Calculation::Eased(..) => false,
        Calculation::Percentage(_) | Calculation::Size => true,
        Calculation::Sum(items)
        | Calculation::Product(items)
        | Calculation::Min(items)
        | Calculation::Max(items)
        | Calculation::Hypot(items) => items.iter().any(is_length),
        Calculation::Unary(function, argument) => {
            *function == UnaryFunction::Abs && is_length(argument)
        }
        Calculation::Binary(function, operands) => {
            matches!(
                function,
                BinaryFunction::Round(_) | BinaryFunction::Mod | BinaryFunction::Rem
            ) && operands.iter().any(is_length)
        }
        Calculation::Progress(_) => false,
        Calculation::Mix(operands) => operands[1..].iter().any(is_length),
    }
}

fn write_number(output: &mut String, value: f32, length: bool) {
    let value = if length {
        value / POINTS_PER_PIXEL
    } else {
        value
    };
    let keyword = if value.is_nan() {
        Some("NaN")
    } else if value == f32::INFINITY {
        Some("infinity")
    } else if value == f32::NEG_INFINITY {
        Some("-infinity")
    } else {
        None
    };
    match (keyword, length) {
        (Some(keyword), true) => write!(output, "({keyword} * 1px)"),
        (Some(keyword), false) => write!(output, "{keyword}"),
        (None, true) => write!(output, "{value}px"),
        (None, false) => write!(output, "{value}"),
    }
    .expect("writing to a string cannot fail");
}

fn write_list(output: &mut String, items: &[Calculation], separator: &str, length: bool) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            output.push_str(separator);
        }
        write_calculation(output, item, length);
    }
}

fn write_function(output: &mut String, name: &str, items: &[Calculation], length: bool) {
    output.push_str(name);
    output.push('(');
    write_list(output, items, ", ", length);
    output.push(')');
}

/// Writes `node`, whose type is a length when `length`, as a calculation-tree term.
fn write_calculation(output: &mut String, node: &Calculation, length: bool) {
    match node {
        Calculation::Value(value) => write_number(output, *value, length),
        Calculation::Percentage(fraction) => {
            write!(output, "{}%", fraction * PERCENT).expect("writing to a string cannot fail");
        }
        Calculation::Size => output.push_str(crate::paint::SIZE_KEYWORD),
        Calculation::Sum(terms) => {
            output.push('(');
            write_list(output, terms, " + ", length);
            output.push(')');
        }
        Calculation::Product(factors) => {
            output.push('(');
            for (index, factor) in factors.iter().enumerate() {
                if index > 0 {
                    output.push_str(" * ");
                }
                write_calculation(output, factor, is_length(factor));
            }
            output.push(')');
        }
        Calculation::Invert(value) => {
            output.push_str("(1 / ");
            write_calculation(output, value, false);
            output.push(')');
        }
        Calculation::Min(items) => write_function(output, "min", items, length),
        Calculation::Max(items) => write_function(output, "max", items, length),
        Calculation::Hypot(items) => write_function(output, "hypot", items, length),
        Calculation::Unary(function, argument) => {
            let (name, radians) = match function {
                UnaryFunction::Abs => ("abs", false),
                UnaryFunction::Sign => ("sign", false),
                UnaryFunction::Sin => ("sin", false),
                UnaryFunction::Cos => ("cos", false),
                UnaryFunction::Tan => ("tan", false),
                UnaryFunction::Asin => ("asin", true),
                UnaryFunction::Acos => ("acos", true),
                UnaryFunction::Atan => ("atan", true),
                UnaryFunction::Sqrt => ("sqrt", false),
                UnaryFunction::Exp => ("exp", false),
                UnaryFunction::Ln => ("log", false),
            };
            write_radians(output, radians, |output| {
                write_function(
                    output,
                    name,
                    std::slice::from_ref(&**argument),
                    is_length(argument),
                );
            });
        }
        Calculation::Binary(function, operands) => {
            let operands_length = operands.iter().any(is_length);
            let name = match function {
                BinaryFunction::Round(strategy) => {
                    let strategy = match strategy {
                        RoundingStrategy::Nearest => "nearest",
                        RoundingStrategy::Up => "up",
                        RoundingStrategy::Down => "down",
                        RoundingStrategy::ToZero => "to-zero",
                    };
                    write!(output, "round({strategy}, ").expect("writing to a string cannot fail");
                    write_list(output, &operands[..], ", ", operands_length);
                    output.push(')');
                    return;
                }
                BinaryFunction::Mod => "mod",
                BinaryFunction::Rem => "rem",
                BinaryFunction::Atan2 => "atan2",
                BinaryFunction::Pow => "pow",
                BinaryFunction::Log => "log",
            };
            write_radians(output, *function == BinaryFunction::Atan2, |output| {
                write_function(output, name, &operands[..], operands_length);
            });
        }
        Calculation::Progress(operands) => {
            write_function(
                output,
                "progress",
                &operands[..],
                operands.iter().any(is_length),
            );
        }
        Calculation::Mix(operands) => {
            output.push_str("calc-mix(");
            match &operands[0] {
                Calculation::Eased(easing, progress) => {
                    write_calculation(output, progress, false);
                    output.push_str(" by ");
                    write_easing(output, easing);
                }
                progress => write_calculation(output, progress, false),
            }
            output.push_str(", ");
            write_list(output, &operands[1..], ", ", length);
            output.push(')');
        }
        Calculation::Eased(easing, input) => {
            output.push_str("calc-mix(");
            write_calculation(output, input, false);
            output.push_str(" by ");
            write_easing(output, easing);
            output.push_str(", 0, 1)");
        }
    }
}

/// Inverse trigonometric functions yield angles; the tree holds them as radian numbers.
fn write_radians(output: &mut String, radians: bool, write: impl FnOnce(&mut String)) {
    if radians {
        output.push('(');
    }
    write(output);
    if radians {
        output.push_str(" / 1rad)");
    }
}

fn write_easing(output: &mut String, easing: &Easing) {
    match easing {
        Easing::Linear => output.push_str("linear"),
        Easing::CubicBezier([x1, y1, x2, y2]) => {
            write!(output, "cubic-bezier({x1}, {y1}, {x2}, {y2})")
                .expect("writing to a string cannot fail");
        }
        Easing::Steps { count, position } => {
            let position = match position {
                StepPosition::Start => "jump-start",
                StepPosition::End => "jump-end",
                StepPosition::None => "jump-none",
                StepPosition::Both => "jump-both",
            };
            write!(output, "steps({count}, {position})").expect("writing to a string cannot fail");
        }
        Easing::Piecewise(points) => {
            output.push_str("linear(");
            for (index, point) in points.iter().enumerate() {
                if index > 0 {
                    output.push_str(", ");
                }
                write!(output, "{} {}%", point.output, point.input * PERCENT)
                    .expect("writing to a string cannot fail");
            }
            output.push(')');
        }
    }
}
