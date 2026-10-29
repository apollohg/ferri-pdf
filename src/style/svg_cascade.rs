//! The raw SVG cascade's custom properties use the same bounded substitution as document CSS.
pub(crate) use crate::style::wide_keywords::WideKeyword;
use crate::style::{StyleError, declaration, grammar::Property, variables};
use std::{borrow::Cow, collections::HashMap};

pub(crate) enum Substitution<'a> {
    Value(Cow<'a, str>),
    Unset,
    Discard,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Variables {
    values: variables::Environment,
    inherited: variables::Environment,
}
impl Variables {
    pub(crate) fn child(
        &self,
        declarations: HashMap<String, (String, bool)>,
        previous: HashMap<String, (String, bool)>,
    ) -> Result<Self, StyleError> {
        let sources = |values: HashMap<String, (String, bool)>| {
            values
                .into_iter()
                .map(|(name, (source, substitutions))| {
                    (
                        name,
                        if substitutions {
                            variables::Source::Css(source)
                        } else {
                            variables::Source::Literal(source)
                        },
                    )
                })
                .collect::<HashMap<_, _>>()
        };
        let declarations = sources(declarations);
        let previous = sources(previous);
        let variables::Resolution::Complete(values) = variables::resolve(
            &self.values,
            declarations,
            variables::WideHandling::Fallback(&std::cell::RefCell::new(previous)),
        )?
        else {
            unreachable!("origin-free custom properties resolve wide values through inheritance")
        };
        Ok(Self {
            values,
            inherited: self.values.clone(),
        })
    }
    pub(crate) fn substitute<'a>(
        &self,
        property: &str,
        source: &'a str,
        substitutions: bool,
    ) -> Result<Substitution<'a>, StyleError> {
        if !substitutions {
            return Ok(Substitution::Value(Cow::Borrowed(source)));
        }
        let value = match variables::Value::with_property_substitutions(source) {
            Ok(None) => return Ok(Substitution::Value(Cow::Borrowed(source))),
            Ok(Some(value)) => value,
            Err(StyleError::Invalid(_)) => return Ok(Substitution::Discard),
            Err(error) => return Err(error),
        };
        let Some(value) = value.substitute(&self.values, &self.inherited)? else {
            return Ok(Substitution::Unset);
        };
        if let Some(property) = Property::from_name(property)
            && declaration::expand_expanded(property, &value)?.is_none()
        {
            return Ok(Substitution::Unset);
        }
        Ok(Substitution::Value(Cow::Owned(value.as_str().to_owned())))
    }
}

pub(crate) fn expand(name: &str, source: &str) -> Result<Vec<(&'static str, String)>, StyleError> {
    use declaration::Specified;
    let Some(property) = Property::from_name(name) else {
        return Ok(Vec::new());
    };
    declaration::expand(property, source)?
        .unwrap_or_default()
        .into_iter()
        .map(|(property, value)| {
            let value = match value {
                Specified::Value(value) => value.to_string(),
                Specified::Wide(keyword) => keyword.name().to_owned(),
                _ => {
                    return Err(StyleError::Unsupported(format!(
                        "SVG declaration {}",
                        property.name()
                    )));
                }
            };
            Ok((property.name(), value))
        })
        .collect()
}

pub(crate) fn initial(name: &str) -> Option<String> {
    Property::from_name(name)
        .and_then(declaration::initial)
        .map(ToString::to_string)
}

pub(crate) fn substitution_function(name: &str) -> bool {
    variables::substitution_function(name)
}
pub(crate) fn valid_custom(source: &str, substitutions: bool) -> Result<bool, StyleError> {
    if !substitutions {
        return Ok(true);
    }
    variables::Value::parse(source).map(|value| value.is_some())
}
