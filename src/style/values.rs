use crate::fonts::{
    FontFaceStyle, FontFamily, FontRange, FontSlant, FontSource, FontStyle, FontWeight, FontWidth,
    GenericFamily,
};
use crate::style::{
    StyleError,
    anchor_values::AnchorEvaluation,
    box_values::unsupported,
    calc::{Context, Kind, Math, MathError, math_function},
    component::Component,
    grammar::{Keyword, Label, Production},
    units::Relative,
};
use cssparser::{Parser, ParserInput};

const SUPPORTED_FONT_FORMATS: [&str; 2] = ["truetype", "opentype"];
pub(super) fn supports_font_format(name: &str) -> bool {
    SUPPORTED_FONT_FORMATS
        .iter()
        .any(|supported| name.eq_ignore_ascii_case(supported))
}

/// A matched math function, resolved against the element's relative lengths.
pub(super) fn math(
    source: &str,
    relative: &Relative<'_>,
    kind: Kind,
    percentage: Option<Kind>,
) -> Result<Math, StyleError> {
    math_with_anchors(source, relative, kind, percentage, None)
}

/// A matched `<length-percentage>` math function whose anchor functions resolve through
/// `anchors` (CSS Anchor Positioning 1 §3.2, §5.1).
pub(super) fn anchored_math(
    source: &str,
    relative: &Relative<'_>,
    anchors: AnchorEvaluation<'_>,
) -> Result<Math, StyleError> {
    math_with_anchors(
        source,
        relative,
        Kind::Length,
        Some(Kind::Length),
        Some(anchors),
    )
}

fn math_with_anchors(
    source: &str,
    relative: &Relative<'_>,
    kind: Kind,
    percentage: Option<Kind>,
    anchors: Option<AnchorEvaluation<'_>>,
) -> Result<Math, StyleError> {
    let mut input = ParserInput::new(source);
    let context = Context {
        relative,
        percentage,
        channels: &[],
        anchors,
    };
    match math_function(&mut Parser::new(&mut input), context)
        .map(|math| math.and_then(|math| math.expect(&[kind])))
    {
        Some(Ok(math)) => Ok(math),
        Some(Err(MathError::Resolution(error))) => Err(error),
        Some(Err(error)) => Err(error.to_string().into()),
        None => Err(format!("not a math function: {source}").into()),
    }
}

/// `<number>`; calculations clamp into the grammar's range.
pub(super) fn number(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    match value.unwrap() {
        Component::Number(number) => Ok(*number),
        Component::Integer(integer) => Ok(*integer as f32),
        Component::Math(source, Kind::Number, range) => {
            Ok(math(source, relative, Kind::Number, None)?.resolve(0.0, *range))
        }
        other => Err(unsupported(other)),
    }
}
/// `<integer>`; calculations round to the nearest integer within the grammar's range.
pub(super) fn integer(value: &Component, relative: &Relative<'_>) -> Result<i32, StyleError> {
    match value.unwrap() {
        Component::Integer(integer) => Ok(*integer),
        Component::Math(source, Kind::Number, range) => {
            Ok(math(source, relative, Kind::Number, None)?.integer(*range))
        }
        other => Err(unsupported(other)),
    }
}
/// `<angle>` or `<zero>` in degrees.
pub(super) fn angle(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    match value.unwrap() {
        Component::Angle(degrees) => Ok(*degrees),
        Component::Number(zero) => Ok(*zero),
        Component::Math(source, Kind::Angle, range) => {
            Ok(math(source, relative, Kind::Angle, None)?.resolve(0.0, *range))
        }
        other => Err(unsupported(other)),
    }
}
/// `<resolution>` in dots per CSS pixel.
pub(super) fn resolution(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    match value.unwrap() {
        Component::Resolution(dppx) => Ok(*dppx),
        Component::Math(source, Kind::Resolution, range) => {
            Ok(math(source, relative, Kind::Resolution, None)?.resolve(0.0, *range))
        }
        other => Err(unsupported(other)),
    }
}
/// `<length>` in points.
/// A `<percentage>` as a fraction.
pub(super) fn percentage(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    match value.unwrap() {
        Component::Percentage(fraction) => Ok(*fraction),
        Component::Math(source, Kind::Percentage, range) => {
            Ok(
                math(source, relative, Kind::Percentage, Some(Kind::Percentage))?
                    .resolve(0.0, *range),
            )
        }
        other => Err(unsupported(other)),
    }
}

pub(super) fn length(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    match value.unwrap() {
        Component::Length(length) => length.points(relative),
        Component::Math(source, Kind::Length, range) => {
            Ok(math(source, relative, Kind::Length, None)?.resolve(0.0, *range))
        }
        other => Err(unsupported(other)),
    }
}

/// `[ <family-name> | <generic-family> ]#`.
pub(super) fn families(value: &Component) -> Vec<FontFamily> {
    value
        .items()
        .iter()
        .map(|item| {
            let generic = item.find(Label::Production(Production::GenericFamily));
            match generic.and_then(Component::keyword) {
                Some(Keyword::SansSerif) => FontFamily::Generic(GenericFamily::SansSerif),
                Some(Keyword::Serif) => FontFamily::Generic(GenericFamily::Serif),
                Some(Keyword::Math) => FontFamily::Generic(GenericFamily::Math),
                Some(Keyword::Monospace) => FontFamily::Generic(GenericFamily::Monospace),
                // Generics without a configured face fall through to the next family.
                _ => FontFamily::Named(
                    generic.map_or_else(|| family_name(item), ToString::to_string),
                ),
            }
        })
        .collect()
}

/// `<string> | <custom-ident>+`; identifiers join with single spaces.
pub(super) fn family_name(value: &Component) -> String {
    match value.unwrap() {
        Component::String(name) => name.to_string(),
        other => other
            .items()
            .iter()
            .filter_map(|item| match item.unwrap() {
                Component::Ident(name) => Some(name.as_ref()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

const VARIATIONS_TECHNOLOGY: &str = "variations";
const LEGACY_VARIATION_FORMAT_SUFFIX: &str = "-variations";
const FONT_TECHNOLOGIES: &[&str] = &[
    "features-opentype",
    "features-aat",
    "color-COLRv0",
    "color-COLRv1",
    "color-sbix",
    "color-CBDT",
    VARIATIONS_TECHNOLOGY,
    "palettes",
];
pub(super) fn supports_font_technology(name: &str) -> bool {
    FONT_TECHNOLOGIES
        .iter()
        .any(|supported| name.eq_ignore_ascii_case(supported))
}

pub(super) fn font_sources(value: &Component) -> Result<Vec<FontSource>, StyleError> {
    let mut sources = Vec::new();
    for item in value.items() {
        if let Component::Function(Keyword::Local, Some(name)) = item.unwrap() {
            sources.push(FontSource::Local(family_name(name)));
            continue;
        }
        let Some(Component::Url(url)) = item.items().first().map(Component::unwrap) else {
            return Err(unsupported(item));
        };
        let format = item.find(Label::Production(Production::FontSourceFormat));
        let supported_format = format.is_none_or(|format| {
            format
                .arguments()
                .first()
                .is_some_and(|argument| match argument.unwrap() {
                    Component::String(name) => {
                        let normalized = name.to_ascii_lowercase();
                        normalized
                            .strip_suffix(LEGACY_VARIATION_FORMAT_SUFFIX)
                            .map_or_else(
                                || supports_font_format(&normalized),
                                |base| {
                                    supports_font_format(base)
                                        && supports_font_technology(VARIATIONS_TECHNOLOGY)
                                },
                            )
                    }
                    Component::Keyword(keyword) => supports_font_format(keyword.name()),
                    _ => false,
                })
        });
        let tech = item.find(Label::Production(Production::FontSourceTech));
        let supported_tech = tech.is_none_or(|tech| {
            tech.arguments().iter().all(|argument| {
                argument
                    .keyword()
                    .is_some_and(|keyword| supports_font_technology(keyword.name()))
            })
        });
        if supported_format && supported_tech {
            sources.push(FontSource::Url(url.to_string()));
        }
    }
    Ok(sources)
}

fn slant_angles(value: &Component, relative: &Relative<'_>) -> Result<Vec<FontSlant>, StyleError> {
    value
        .items()
        .iter()
        .flat_map(Component::items)
        .filter(|item| item.keyword().is_none())
        .map(|item| Ok(FontSlant::new(angle(item, relative)?)?))
        .collect()
}

pub(super) fn font_style(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<(FontStyle, FontSlant), StyleError> {
    let mut keywords = Vec::new();
    value.keywords(&mut keywords);
    match keywords.first() {
        Some(Keyword::Normal) => Ok((FontStyle::Normal, FontSlant::NORMAL)),
        Some(Keyword::Italic) => Ok((FontStyle::Italic, FontSlant::NORMAL)),
        Some(Keyword::Oblique) => {
            let angles = slant_angles(value, relative)?;
            Ok((
                FontStyle::Oblique,
                angles.first().copied().unwrap_or(FontSlant::DEFAULT),
            ))
        }
        _ => Err(unsupported(value)),
    }
}

pub(super) fn font_width(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<FontWidth, StyleError> {
    let percentage = match value.keyword() {
        Some(Keyword::Normal) => return Ok(FontWidth::NORMAL),
        Some(Keyword::UltraCondensed) => return Ok(FontWidth::ULTRA_CONDENSED),
        Some(Keyword::ExtraCondensed) => return Ok(FontWidth::EXTRA_CONDENSED),
        Some(Keyword::Condensed) => return Ok(FontWidth::CONDENSED),
        Some(Keyword::SemiCondensed) => return Ok(FontWidth::SEMI_CONDENSED),
        Some(Keyword::SemiExpanded) => return Ok(FontWidth::SEMI_EXPANDED),
        Some(Keyword::Expanded) => return Ok(FontWidth::EXPANDED),
        Some(Keyword::ExtraExpanded) => return Ok(FontWidth::EXTRA_EXPANDED),
        Some(Keyword::UltraExpanded) => return Ok(FontWidth::ULTRA_EXPANDED),
        _ => match value.unwrap() {
            Component::Percentage(fraction) => fraction * FontWidth::NORMAL.value(),
            Component::Math(source, Kind::Percentage, range) => {
                math(source, relative, Kind::Percentage, None)?.resolve(0.0, *range)
                    * FontWidth::NORMAL.value()
            }
            _ => return Err(unsupported(value)),
        },
    };
    Ok(FontWidth::new(percentage.clamp(f32::MIN, f32::MAX))?)
}

fn face_range<T: Copy + PartialOrd>(
    value: &Component,
    mut parse: impl FnMut(&Component) -> Result<T, StyleError>,
) -> Result<FontRange<T>, StyleError> {
    if value.keyword() == Some(Keyword::Auto) {
        return Ok(FontRange::Auto);
    }
    let values = value
        .items()
        .iter()
        .map(&mut parse)
        .collect::<Result<Vec<_>, _>>()?;
    match values[..] {
        [value] => Ok(FontRange::single(value)),
        [start, end] => Ok(FontRange::new(start, end)),
        _ => Err(unsupported(value)),
    }
}
pub(super) fn face_width(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<FontRange<FontWidth>, StyleError> {
    face_range(value, |value| font_width(value, relative))
}
pub(super) fn face_style(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<FontFaceStyle, StyleError> {
    if value.keyword() == Some(Keyword::Auto) {
        return Ok(FontFaceStyle::Auto);
    }
    let (style, _) = font_style(value, relative)?;
    Ok(match style {
        FontStyle::Normal => FontFaceStyle::Normal,
        FontStyle::Italic => FontFaceStyle::Italic,
        FontStyle::Oblique => {
            let angles = slant_angles(value, relative)?;
            FontFaceStyle::Oblique(match angles[..] {
                [] => FontRange::single(FontSlant::DEFAULT),
                [one] => FontRange::single(one),
                [start, end] => FontRange::new(start, end),
                _ => return Err(unsupported(value)),
            })
        }
    })
}

/// `<font-weight-absolute> | bolder | lighter`, relative keywords against `inherited`.
pub(super) fn font_weight(
    value: &Component,
    inherited: FontWeight,
    relative: &Relative<'_>,
) -> Result<FontWeight, StyleError> {
    match value.keyword() {
        Some(Keyword::Normal) => Ok(FontWeight::NORMAL),
        Some(Keyword::Bold) => Ok(FontWeight::BOLD),
        Some(Keyword::Bolder) => Ok(inherited.bolder()),
        Some(Keyword::Lighter) => Ok(inherited.lighter()),
        _ => Ok(FontWeight::new(number(value, relative)?)?),
    }
}

pub(super) fn face_weight(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<FontRange<FontWeight>, StyleError> {
    face_range(value, |value| {
        font_weight(value, FontWeight::NORMAL, relative)
    })
}
