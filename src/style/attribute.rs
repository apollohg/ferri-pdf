//! Contextual attribute values using the shared syntax and substitution machinery.
use crate::style::{
    StyleError,
    conditions::block_source,
    grammar::{Base, Keyword},
    syntax::{Atom, Clause, List, Syntax},
    variables::{Expanded, Value},
};
use cssparser::{Delimiter, ParseError, Parser, Token};

pub(super) const FUNCTION: &str = "attr";
#[derive(Clone, Debug)]
pub(super) struct Attribute {
    pub prefix: Option<String>,
    pub name: String,
    kind: AttributeType,
    fallback: Option<Value>,
}
/// How the attribute's value becomes the substitution value: CSS Values 5 `<attr-type>`,
/// and the bare `<syntax>` of the pinned recommendation.
#[derive(Clone, Debug)]
enum AttributeType {
    /// `raw-string`, or no type: the literal value as a `<string>`.
    RawString,
    /// `type(<syntax>)`, `number` or a bare `<syntax>`: the value parsed against it.
    Syntax(Syntax),
    /// `<attr-unit>`: a `<number>` as a dimension or percentage in the unit.
    Unit(String),
}
impl AttributeType {
    fn parse<'i>(
        input: &mut Parser<'i, '_>,
        depth: usize,
    ) -> Result<Self, ParseError<'i, StyleError>> {
        if input.is_exhausted() {
            return Ok(Self::RawString);
        }
        let start = input.state();
        let kind = match input.next()?.clone() {
            Token::Function(name) if name.eq_ignore_ascii_case(Keyword::Type.name()) => Some(
                Self::Syntax(input.parse_nested_block(|nested| Self::syntax(nested, depth))?),
            ),
            Token::Ident(name) if name.eq_ignore_ascii_case(Keyword::RawString.name()) => {
                Some(Self::RawString)
            }
            Token::Ident(name) if name.eq_ignore_ascii_case(Keyword::Number.name()) => {
                Some(Self::Syntax(Syntax::Alternatives(vec![Clause {
                    atom: Atom::Base(Base::Number),
                    list: List::Once,
                }])))
            }
            Token::Ident(unit) if crate::style::units::dimension(&unit).is_some() => {
                Some(Self::Unit(unit.to_ascii_lowercase()))
            }
            Token::Delim(crate::style::matcher::PERCENT) => {
                Some(Self::Unit(crate::style::matcher::PERCENT.to_string()))
            }
            _ => None,
        };
        match kind {
            Some(kind) if input.is_exhausted() => Ok(kind),
            _ => {
                input.reset(&start);
                Self::syntax(input, depth).map(Self::Syntax)
            }
        }
    }
    fn syntax<'i>(
        input: &mut Parser<'i, '_>,
        depth: usize,
    ) -> Result<Syntax, ParseError<'i, StyleError>> {
        let source = block_source(input);
        let mut raw = cssparser::ParserInput::new(source);
        Syntax::parse(&mut Parser::new(&mut raw), depth)
            .map_err(|error| input.new_custom_error(error))?
            .ok_or_else(|| {
                input.new_custom_error(StyleError::Invalid("invalid attribute syntax".into()))
            })
    }
}
pub(super) trait Context {
    fn attribute_data(&self, prefix: Option<&str>, name: &str) -> Option<&str>;
    fn expand_attribute_value(&self, value: &Value) -> Result<Option<Expanded>, StyleError>;
    fn matches_attribute_syntax(
        &self,
        syntax: &Syntax,
        value: &Expanded,
    ) -> Result<bool, StyleError>;
}
impl Attribute {
    pub(super) fn parse(
        input: &mut Parser<'_, '_>,
        depth: usize,
        property_syntax: bool,
    ) -> Result<Option<Self>, StyleError> {
        let parsed = input.parse_entirely(|input| {
            let (prefix, name, kind) = input.parse_until_before(Delimiter::Comma, |input| {
                let first = input.expect_ident_cloned()?.to_string();
                let (prefix, name) = if input.try_parse(|input| input.expect_delim('|')).is_ok() {
                    (Some(first), input.expect_ident_cloned()?.to_string())
                } else {
                    (None, first)
                };
                Ok((prefix, name, AttributeType::parse(input, depth)?))
            })?;
            let fallback = if input.try_parse(Parser::expect_comma).is_ok() {
                if input.is_exhausted() {
                    return Err(input.new_custom_error(StyleError::Invalid(
                        "empty attribute fallback requires a wrapper".into(),
                    )));
                }
                let source = crate::style::parser::substitution_argument(input, false)?;
                Some(
                    Value::parse_context(source, depth, property_syntax)
                        .map_err(|error| input.new_custom_error(error))?
                        .ok_or_else(|| {
                            input.new_custom_error(StyleError::Invalid(
                                "invalid attribute fallback".into(),
                            ))
                        })?,
                )
            } else {
                None
            };
            Ok(Self {
                prefix,
                name,
                kind,
                fallback,
            })
        });
        match parsed.map_err(crate::style::parser::failure) {
            Ok(value) => Ok(Some(value)),
            Err(StyleError::Invalid(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub(super) fn dependencies(&self, names: &mut std::collections::HashSet<String>) {
        if let Some(value) = &self.fallback {
            value.dependencies(names);
        }
    }
    pub(super) fn resolve(&self, context: &impl Context) -> Result<Option<Expanded>, StyleError> {
        if let Some(source) = context.attribute_data(self.prefix.as_deref(), &self.name) {
            match &self.kind {
                AttributeType::RawString => return Expanded::attribute_string(source).map(Some),
                AttributeType::Syntax(syntax) => {
                    if let Some(value) = Value::parse(source)?
                        && let Some(value) = context.expand_attribute_value(&value)?
                        && context.matches_attribute_syntax(syntax, &value)?
                    {
                        return Ok(Some(value.attribute_tainted()));
                    }
                }
                AttributeType::Unit(unit) => {
                    if let Some(number) = number_source(source)
                        && let Some(value) = Value::parse(&format!("{number}{unit}"))?
                        && let Some(value) = context.expand_attribute_value(&value)?
                    {
                        return Ok(Some(value.attribute_tainted()));
                    }
                }
            }
        }
        if let Some(fallback) = &self.fallback {
            return context
                .expand_attribute_value(fallback)
                .map(|value| value.map(Expanded::attribute_tainted));
        }
        if matches!(self.kind, AttributeType::RawString) {
            Expanded::attribute_string("").map(Some)
        } else {
            Ok(None)
        }
    }
}

/// The source of the one `<number-token>` an attribute's value holds, if it is one.
fn number_source(value: &str) -> Option<&str> {
    let mut input = cssparser::ParserInput::new(value);
    Parser::new(&mut input)
        .parse_entirely(|input| {
            input.skip_whitespace();
            let start = input.position();
            input.expect_number()?;
            Ok::<_, ParseError<'_, ()>>(input.slice_from(start))
        })
        .ok()
}
