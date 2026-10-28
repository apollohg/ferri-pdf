//! CSS Backgrounds 3 §6 `border-image-*` longhands from their match trees.
use crate::paint::{BorderImageOutset, BorderImageRepeat, BorderImageWidth, ImageSlice};
use crate::style::{
    StyleError,
    box_values::{length_percentage, unsupported},
    calc::Kind,
    component::Component,
    grammar::Keyword,
    units::Relative,
    values::{length, number},
};

/// `[ <number> | <percentage> ]{1,4} && fill?` slots.
const SLICE_OFFSETS: usize = 0;
const SLICE_FILL: usize = 1;
const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

/// One to four values for top, right, bottom and left (CSS Backgrounds 3 §4.1).
fn edges<T: Clone>(values: &[T]) -> Result<[T; 4], StyleError> {
    let index = |side: usize| match (values.len(), side) {
        (1, _) => 0,
        (2 | 3, LEFT) => RIGHT,
        (2, BOTTOM) => TOP,
        _ => side,
    };
    if !(1..=4).contains(&values.len()) {
        return Err(format!("expected one to four edge values, found {}", values.len()).into());
    }
    Ok(std::array::from_fn(|side| values[index(side)].clone()))
}

fn number_like(value: &Component) -> bool {
    matches!(
        value.unwrap(),
        Component::Number(_) | Component::Integer(_) | Component::Math(_, Kind::Number, _)
    )
}

/// `border-image-slice`, whose `fill` may come first, or `mask-border-slice`, whose
/// `fill` follows the offsets (CSS Masking 1 §8.3).
pub(super) fn slice(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<([ImageSlice; 4], bool), StyleError> {
    let mut keywords = Vec::new();
    let offsets = if let Some(offsets) = value.slot(SLICE_OFFSETS) {
        if let Some(fill) = value.slot(SLICE_FILL) {
            fill.keywords(&mut keywords);
        }
        offsets.items()
    } else {
        value.keywords(&mut keywords);
        value.items()
    };
    // The offsets repeat as a nested group when `fill` follows them.
    let offsets = offsets
        .iter()
        .flat_map(Component::items)
        .filter(|item| item.keyword() != Some(Keyword::Fill))
        .map(|item| {
            Ok(if number_like(item) {
                ImageSlice::Number(number(item, relative)?)
            } else {
                match length_percentage(item, relative)? {
                    crate::paint::LengthPercentage::Percentage(fraction) => {
                        ImageSlice::Percentage(fraction)
                    }
                    _ => return Err(unsupported(item)),
                }
            })
        })
        .collect::<Result<Vec<_>, StyleError>>()?;
    Ok((edges(&offsets)?, keywords.contains(&Keyword::Fill)))
}

pub(super) fn widths(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<[BorderImageWidth; 4], StyleError> {
    edges(
        &value
            .items()
            .iter()
            .map(|item| {
                Ok(if item.keyword() == Some(Keyword::Auto) {
                    BorderImageWidth::Auto
                } else if number_like(item) {
                    BorderImageWidth::Number(number(item, relative)?)
                } else {
                    BorderImageWidth::Length(length_percentage(item, relative)?)
                })
            })
            .collect::<Result<Vec<_>, StyleError>>()?,
    )
}

pub(super) fn outset(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<[BorderImageOutset; 4], StyleError> {
    edges(
        &value
            .items()
            .iter()
            .map(|item| {
                Ok(if number_like(item) {
                    BorderImageOutset::Number(number(item, relative)?)
                } else {
                    BorderImageOutset::Length(length(item, relative)?)
                })
            })
            .collect::<Result<Vec<_>, StyleError>>()?,
    )
}

/// One keyword for both axes, or horizontal then vertical.
pub(super) fn repeat(value: &Component) -> Result<[BorderImageRepeat; 2], StyleError> {
    let keywords = value
        .items()
        .iter()
        .map(|item| match item.keyword() {
            Some(Keyword::Stretch) => Ok(BorderImageRepeat::Stretch),
            Some(Keyword::Repeat) => Ok(BorderImageRepeat::Repeat),
            Some(Keyword::Round) => Ok(BorderImageRepeat::Round),
            Some(Keyword::Space) => Ok(BorderImageRepeat::Space),
            _ => Err(unsupported(item)),
        })
        .collect::<Result<Vec<_>, StyleError>>()?;
    match keywords.as_slice() {
        [both] => Ok([*both; 2]),
        [horizontal, vertical] => Ok([*horizontal, *vertical]),
        _ => Err(unsupported(value)),
    }
}
