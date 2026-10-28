//! Filter Effects 1 §5 `<filter-value-list>` from its match tree.
use crate::paint::{AbsoluteColor, FilterFunction};
use crate::style::{
    StyleError,
    box_values::unsupported,
    component::Component,
    grammar::Keyword,
    transform::{arguments, number},
    units::Relative,
    values::{angle, length},
};

/// `none | <filter-value-list>`; `none` is empty.
pub(super) fn filters(
    value: &Component,
    relative: &Relative<'_>,
    current: &AbsoluteColor,
) -> Result<Vec<FilterFunction>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(Vec::new());
    }
    value
        .items()
        .iter()
        .map(|item| function(item, relative, current))
        .collect()
}

/// §6.2: omitted arguments compute to their defaults, which §6.1 gives per function.
fn function(
    value: &Component,
    relative: &Relative<'_>,
    current: &AbsoluteColor,
) -> Result<FilterFunction, StyleError> {
    let name = match value.unwrap() {
        // Filters in other documents are not loaded: only same-document references resolve.
        Component::Url(url) if !url.starts_with('#') => {
            return Err(StyleError::Unsupported(format!(
                "filter reference to another document: {value}"
            )));
        }
        Component::Url(url) => return Ok(FilterFunction::Url(url.clone())),
        Component::Function(name, _) => *name,
        _ => return Err(unsupported(value)),
    };
    let args = arguments(value);
    let amount = |default: f32| -> Result<f32, StyleError> {
        let amount = args
            .first()
            .map(|arg| number(arg, relative))
            .transpose()?
            .unwrap_or(default);
        if amount < 0.0 {
            return Err(format!("negative filter function amount: {value}").into());
        }
        Ok(amount)
    };
    Ok(match name {
        Keyword::Blur => {
            let deviation = args
                .first()
                .map(|arg| length(arg, relative))
                .transpose()?
                .unwrap_or(0.0);
            if deviation < 0.0 {
                return Err(format!("negative blur standard deviation: {value}").into());
            }
            FilterFunction::Blur(deviation)
        }
        Keyword::Brightness => FilterFunction::Brightness(amount(1.0)?),
        Keyword::Contrast => FilterFunction::Contrast(amount(1.0)?),
        Keyword::Grayscale => FilterFunction::Grayscale(amount(1.0)?),
        Keyword::Invert => FilterFunction::Invert(amount(1.0)?),
        Keyword::Opacity => FilterFunction::Opacity(amount(1.0)?),
        Keyword::Saturate => FilterFunction::Saturate(amount(1.0)?),
        Keyword::Sepia => FilterFunction::Sepia(amount(1.0)?),
        Keyword::HueRotate => FilterFunction::HueRotate(
            args.first()
                .map(|arg| angle(arg, relative))
                .transpose()?
                .unwrap_or(0.0),
        ),
        Keyword::DropShadow => drop_shadow(value, &args, relative, current)?,
        _ => return Err(unsupported(value)),
    })
}

/// `drop-shadow( <color>? && <length>{2,3} )`: offsets, then the standard deviation; the
/// colour defaults to `currentcolor`.
fn drop_shadow(
    value: &Component,
    args: &[&Component],
    relative: &Relative<'_>,
    current: &AbsoluteColor,
) -> Result<FilterFunction, StyleError> {
    let mut lengths = Vec::new();
    let mut color = None;
    for arg in args {
        match arg.unwrap() {
            Component::Color(_) | Component::MathColor(_) => {
                color = Some(crate::style::colors::color(arg, relative, current)?);
            }
            _ => lengths.push(length(arg, relative)?),
        }
    }
    let [x, y, deviation] = match lengths[..] {
        [x, y] => [x, y, 0.0],
        [x, y, deviation] => [x, y, deviation],
        _ => return Err(unsupported(value)),
    };
    if deviation < 0.0 {
        return Err(format!("negative drop-shadow standard deviation: {value}").into());
    }
    Ok(FilterFunction::DropShadow {
        offset: [x, y],
        deviation,
        color: color.unwrap_or_else(|| current.used()),
    })
}
