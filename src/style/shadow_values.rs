//! CSS Backgrounds 3 `box-shadow`: `none | <shadow>#` from its match tree.
use crate::paint::{BoxShadow, Color};
use crate::style::{
    StyleError, box_values::unsupported, component::Component, grammar::Keyword, units::Relative,
    values::length,
};

/// `inset? && <length>{2,4} && <color>?` slots.
const INSET_SLOT: usize = 0;
const LENGTHS_SLOT: usize = 1;
const COLOR_SLOT: usize = 2;
const BLUR_INDEX: usize = 2;
const SPREAD_INDEX: usize = 3;

pub(super) fn shadows(
    value: &Component,
    relative: &Relative<'_>,
    current: Color,
) -> Result<Vec<BoxShadow>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(Vec::new());
    }
    value
        .items()
        .iter()
        .map(|item| shadow(item, relative, current))
        .collect()
}

fn shadow(
    value: &Component,
    relative: &Relative<'_>,
    current: Color,
) -> Result<BoxShadow, StyleError> {
    let lengths = value
        .slot(LENGTHS_SLOT)
        .ok_or_else(|| unsupported(value))?
        .items()
        .iter()
        .map(|item| length(item, relative))
        .collect::<Result<Vec<_>, _>>()?;
    let blur = lengths.get(BLUR_INDEX).copied().unwrap_or(0.0);
    if blur < 0.0 {
        return Err(format!("negative box-shadow blur radius: {value}").into());
    }
    let mut keywords = Vec::new();
    if let Some(inset) = value.slot(INSET_SLOT) {
        inset.keywords(&mut keywords);
    }
    Ok(BoxShadow {
        offset: [lengths[0], lengths[1]],
        blur,
        spread: lengths.get(SPREAD_INDEX).copied().unwrap_or(0.0),
        // `<color>?` matches as a group of at most one colour.
        color: value
            .slot(COLOR_SLOT)
            .and_then(|color| color.items().first())
            .map(|color| crate::style::colors::color(color, relative, current))
            .transpose()?
            .unwrap_or(current),
        inset: keywords.contains(&Keyword::Inset),
    })
}
