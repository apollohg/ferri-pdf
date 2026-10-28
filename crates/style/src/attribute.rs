//! Contextual attribute values using the shared syntax and substitution machinery.
use crate::{
    StyleError,
    conditions::block_source,
    syntax::Syntax,
    variables::{Expanded, Value},
};
use cssparser::{Delimiter, Parser};

pub(super) const FUNCTION: &str = "attr";
#[derive(Clone, Debug)]
pub(super) struct Attribute {
    pub prefix: Option<String>,
    pub name: String,
    syntax: Option<Syntax>,
    fallback: Option<Value>,
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
            let (prefix, name, syntax) = input.parse_until_before(Delimiter::Comma, |input| {
                let first = input.expect_ident_cloned()?.to_string();
                let (prefix, name) = if input.try_parse(|input| input.expect_delim('|')).is_ok() {
                    (Some(first), input.expect_ident_cloned()?.to_string())
                } else {
                    (None, first)
                };
                let syntax = if input.is_exhausted() {
                    None
                } else {
                    let source = block_source(input);
                    let mut raw = cssparser::ParserInput::new(source);
                    Some(
                        Syntax::parse(&mut Parser::new(&mut raw), depth)
                            .map_err(|error| input.new_custom_error(error))?
                            .ok_or_else(|| {
                                input.new_custom_error(StyleError::Invalid(
                                    "invalid attribute syntax".into(),
                                ))
                            })?,
                    )
                };
                Ok((prefix, name, syntax))
            })?;
            let fallback = if input.try_parse(Parser::expect_comma).is_ok() {
                if input.is_exhausted() {
                    return Err(input.new_custom_error(StyleError::Invalid(
                        "empty attribute fallback requires a wrapper".into(),
                    )));
                }
                let source = crate::parser::substitution_argument(input, false)?;
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
                syntax,
                fallback,
            })
        });
        match parsed.map_err(crate::parser::failure) {
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
            if let Some(syntax) = &self.syntax {
                if let Some(value) = Value::parse(source)?
                    && let Some(value) = context.expand_attribute_value(&value)?
                    && context.matches_attribute_syntax(syntax, &value)?
                {
                    return Ok(Some(value.attribute_tainted()));
                }
            } else {
                return Expanded::attribute_string(source).map(Some);
            }
        }
        if let Some(fallback) = &self.fallback {
            return context
                .expand_attribute_value(fallback)
                .map(|value| value.map(Expanded::attribute_tainted));
        }
        if self.syntax.is_none() {
            Expanded::attribute_string("").map(Some)
        } else {
            Ok(None)
        }
    }
}
