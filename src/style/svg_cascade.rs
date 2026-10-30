//! The raw SVG cascade's custom properties use the same bounded substitution as document CSS.
pub(crate) use crate::style::units::{FontUnit, FontUnits, Relative};
pub(crate) use crate::style::wide_keywords::WideKeyword;
use crate::style::{Size, StyleError, declaration, grammar::Property, variables};
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

/// `None` is `auto`, which a `width` or `height` whose basis is a keyword behaves as.
pub(crate) fn length(
    source: &str,
    property: &str,
    relative: &Relative<'_>,
) -> Result<Option<crate::paint::LengthPercentage>, StyleError> {
    use crate::style::{box_values, grammar::Keyword};
    let Some(property) = sized_property(property) else {
        let Some(value) = declaration::value(Property::MarginLeft, source, &mut 0)? else {
            return Ok(None);
        };
        if value.keyword() == Some(Keyword::Auto) {
            return Ok(None);
        }
        return box_values::length_percentage(&value, relative).map(Some);
    };
    Ok(match size(property, source, relative)? {
        Some(Size::Value(value)) => Some(value),
        Some(Size::Calculated(calculated)) => calculated.definite(),
        Some(Size::Auto | Size::Intrinsic(_)) | None => None,
    })
}

/// The `calc-size()` calculation of a `width` or `height` whose basis is a keyword, which
/// gives the used size from the automatic size the element gives that basis (CSS Values 5
/// §10.2).
pub(crate) fn automatic_size(
    source: &str,
    property: &str,
    relative: &Relative<'_>,
) -> Result<Option<crate::paint::Calculation>, StyleError> {
    let Some(property) = sized_property(property) else {
        return Ok(None);
    };
    Ok(match size(property, source, relative)? {
        Some(Size::Calculated(calculated)) if calculated.definite().is_none() => {
            Some(calculated.calculation.clone())
        }
        _ => None,
    })
}

fn sized_property(property: &str) -> Option<Property> {
    match property {
        "width" => Some(Property::Width),
        "height" => Some(Property::Height),
        _ => None,
    }
}

/// SVG content sizes no box from its contents: `auto` gives an image or a viewport its
/// natural size or `100%` and other elements none (SVG 2 §7.8), so the CSS Sizing 3 §3.1 intrinsic
/// keywords, also as a `calc-size()` basis, behave as `auto`.
fn size(
    property: Property,
    source: &str,
    relative: &Relative<'_>,
) -> Result<Option<Size>, StyleError> {
    let Some(value) = declaration::value(property, source, &mut 0)? else {
        return Ok(None);
    };
    Size::from_component(&value, relative).map(Some)
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

/// SVG 2 §6.6: attributes of these properties are presentation attributes only on some
/// elements, which `presentation_property` names.
const ELEMENT_GEOMETRY: [Property; 11] = [
    Property::Cx,
    Property::Cy,
    Property::R,
    Property::Rx,
    Property::Ry,
    Property::D,
    Property::X,
    Property::Y,
    Property::Width,
    Property::Height,
    Property::Transform,
];
/// Flow-relative properties map to physical ones by the writing mode and direction.
const FLOW: [Property; 2] = [Property::WritingMode, Property::Direction];

/// The computed style of an element of a standalone SVG document `tag`, whose parent's is
/// `parent`, as the document computes its own (SVG 2 §6.4): its own attributes as
/// presentation attributes, overridden by the longhand `declarations` that won its cascade.
/// Its font size is `size` points, as the SVG renderer resolves it, and `relative` measures
/// its lengths.
pub(crate) fn computed_style(
    tag: &str,
    parent: &crate::style::Style,
    attributes: &[(&str, &str)],
    declarations: &[(&str, String)],
    size: f32,
    relative: &Relative<'_>,
) -> Result<crate::style::Style, StyleError> {
    use crate::style::logical;
    let mut style = crate::style::Style::default();
    crate::style::properties::inherit(&mut style, parent);
    style.size = size;
    let mut longhands = Vec::new();
    for &(name, value) in attributes {
        let Some(property) = Property::from_name(name).filter(|property| {
            !ELEMENT_GEOMETRY.contains(property)
                || crate::style::svg_values::presentation_property(tag, name).is_some()
        }) else {
            continue;
        };
        longhands.extend(declaration::expand_presentation(property, value)?.unwrap_or_default());
    }
    for (name, value) in declarations {
        if let Some(property) = Property::from_name(name) {
            longhands.extend(declaration::expand(property, value)?.unwrap_or_default());
        }
    }
    longhands.sort_by_key(|(property, _)| !FLOW.contains(property));
    for (property, value) in &longhands {
        if *property == Property::FontSize {
            continue;
        }
        let property = if logical::is_flow_relative(*property) {
            logical::physical(*property, logical::Flow::of(&style))
        } else {
            *property
        };
        crate::style::properties::apply(property, value, &mut style, parent, relative)?;
    }
    Ok(style)
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
