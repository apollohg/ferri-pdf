//! The raw SVG cascade's custom properties use the same bounded substitution as document CSS.
pub(crate) use crate::style::units::{FontUnit, FontUnits, Relative};
pub(crate) use crate::style::wide_keywords::WideKeyword;
use crate::style::{StyleError, declaration, grammar::Property, variables};
use std::{borrow::Cow, collections::HashMap};

pub(crate) fn relative<'a>(
    font: &'a dyn FontUnits,
    root: &'a dyn FontUnits,
    viewport: [f32; 2],
    dpi: f32,
) -> Relative<'a> {
    use crate::style::units::{CSS_PIXELS_PER_INCH, FontBasis, Viewport};
    Relative {
        font: FontBasis::Deferred(font),
        root: FontBasis::Deferred(root),
        viewport: Viewport::Known(viewport, false),
        physical_unit_scale: dpi / CSS_PIXELS_PER_INCH,
        ..Relative::initial()
    }
}

pub(crate) fn length(
    source: &str,
    property: &str,
    relative: &Relative<'_>,
) -> Result<Option<crate::paint::LengthPercentage>, StyleError> {
    use crate::style::{box_values, grammar::Keyword};
    let property = match property {
        "width" => Property::Width,
        "height" => Property::Height,
        _ => Property::MarginLeft,
    };
    let Some(value) = declaration::value(property, source, &mut 0)? else {
        return Ok(None);
    };
    if value.keyword() == Some(Keyword::Auto) {
        return Ok(None);
    }
    box_values::length_percentage(&value, relative).map(Some)
}

pub(crate) fn font_size(
    source: &str,
    initial_size: f32,
    relative: &Relative<'_>,
) -> Result<Option<f32>, StyleError> {
    let Some(value) = declaration::value(Property::FontSize, source, &mut 0)? else {
        return Ok(None);
    };
    if value.keyword() == Some(crate::style::grammar::Keyword::Math) {
        return relative.font.size().map(Some);
    }
    crate::style::properties::font_size_value(&value, relative, initial_size).map(Some)
}

pub(crate) fn length_list(
    source: &str,
    property: &str,
    relative: &Relative<'_>,
) -> Result<Option<Vec<crate::paint::LengthPercentage>>, StyleError> {
    use crate::style::{component::Component, grammar::Keyword, matcher, parser, svg_values};
    use cssparser::Token;
    let mut steps = 0;
    if property == "stroke-dasharray" {
        let Some(value) = declaration::value(Property::StrokeDasharray, source, &mut steps)? else {
            return Ok(None);
        };
        if value.keyword() == Some(Keyword::None) {
            return Ok(None);
        }
        return value
            .items()
            .iter()
            .flat_map(Component::items)
            .map(|value| svg_values::user_length(value, relative))
            .collect::<Result<Vec<_>, _>>()
            .map(Some);
    }
    if !parser::balanced_components(source)? {
        return Ok(None);
    }
    let components = parser::component_values(source);
    let mut values = Vec::new();
    let mut after_comma = false;
    for (index, (start, token)) in components.iter().enumerate() {
        match token {
            Token::WhiteSpace(_) => continue,
            Token::Comma => {
                if values.is_empty() || after_comma {
                    return Ok(None);
                }
                after_comma = true;
                continue;
            }
            _ => {}
        }
        let end = components
            .get(index + 1)
            .map_or(source.len(), |(start, _)| *start);
        let Some(value) = matcher::match_value(
            Property::StrokeDashoffset.data().root,
            &source[*start..end],
            &mut steps,
            matcher::ValueContext::Presentation,
        )?
        else {
            return Ok(None);
        };
        values.push(svg_values::user_length(&value, relative)?);
        after_comma = false;
    }
    Ok((!after_comma).then_some(values))
}

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

pub(crate) fn zoom(source: &str) -> Result<Option<String>, StyleError> {
    use declaration::Specified;
    let Some(mut declarations) = declaration::expand(Property::Zoom, source)? else {
        return Ok(None);
    };
    let Some((_, value)) = declarations.pop() else {
        return Ok(None);
    };
    match value {
        Specified::Value(value) => {
            crate::style::properties::zoom_value(&value, &Relative::initial())
                .map(|value| Some(value.to_string()))
        }
        Specified::Wide(keyword) => Ok(Some(keyword.name().to_owned())),
        _ => Ok(None),
    }
}

/// The SVG filter-function parser requires bare fragment URLs.
pub(crate) fn filter_source(source: &str) -> Result<Option<String>, StyleError> {
    let Some(mut declarations) = declaration::expand(Property::Filter, source)? else {
        return Ok(None);
    };
    let Some((_, declaration::Specified::Value(value))) = declarations.pop() else {
        return Ok(None);
    };
    Ok(Some(
        value
            .items()
            .iter()
            .map(|item| match item.unwrap() {
                crate::style::component::Component::Url(reference)
                    if reference.starts_with('#') =>
                {
                    format!(
                        "url(#{})",
                        percent_encoding::utf8_percent_encode(
                            &reference[1..],
                            percent_encoding::NON_ALPHANUMERIC
                        )
                    )
                }
                _ => item.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" "),
    ))
}
