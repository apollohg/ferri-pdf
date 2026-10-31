//! CSS Masking 1 §7 `mask-*` longhands from their match trees.
use crate::paint::{
    AbsoluteColor, BackgroundImage, BoxArea, GeometryBox, MaskComposite, MaskMode, Position,
    ShapeBox,
};
use crate::style::{
    StyleError, background_values, box_values::unsupported, component::Component, grammar::Keyword,
    units::Relative,
};

/// A URL with a fragment names an element, here a `mask` element.
const FRAGMENT: char = '#';

/// `mask-image` layers: images, and references to `mask` elements.
pub(super) type MaskImages = (Vec<BackgroundImage>, Vec<Option<Box<str>>>);

/// `<mask-reference>#`: images, and references to `mask` elements by layer.
pub(super) fn images(
    value: &Component,
    relative: &Relative<'_>,
    color: &AbsoluteColor,
) -> Result<MaskImages, StyleError> {
    value
        .items()
        .iter()
        .map(|item| match item.unwrap() {
            // §7.1: a reference naming no `mask` element is a transparent black layer.
            Component::Url(url) if url.contains(FRAGMENT) => {
                Ok((BackgroundImage::None, Some(url.clone())))
            }
            _ => Ok((background_values::image(item, relative, color)?, None)),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|layers| layers.into_iter().unzip())
}

/// `<position>#`, one position per layer.
pub(super) fn positions(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Vec<Position>, StyleError> {
    value
        .items()
        .iter()
        .map(|item| background_values::position(item, relative))
        .collect()
}

/// The box of an element with a CSS layout box: the content box for `fill-box` and the
/// border box for `stroke-box` and `view-box` (§7.5, §7.6).
pub(super) fn layout_area(reference: GeometryBox) -> BoxArea {
    match reference.layout() {
        ShapeBox::Visual(area) => area,
        ShapeBox::Margin => BoxArea::BorderBox,
    }
}

pub(super) fn origins(value: &Component) -> Result<Vec<GeometryBox>, StyleError> {
    value
        .items()
        .iter()
        .map(crate::style::shape_values::geometry_box)
        .collect()
}

/// `[ <coord-box> | no-clip ]#`: the clip boxes, and whether each layer is unclipped.
pub(super) fn clips(value: &Component) -> Result<(Vec<GeometryBox>, Vec<bool>), StyleError> {
    value
        .items()
        .iter()
        .map(|item| match item.keyword() {
            Some(Keyword::NoClip) => Ok((
                GeometryBox::Shape(ShapeBox::Visual(BoxArea::BorderBox)),
                true,
            )),
            _ => Ok((crate::style::shape_values::geometry_box(item)?, false)),
        })
        .collect::<Result<Vec<_>, StyleError>>()
        .map(|layers| layers.into_iter().unzip())
}

pub(super) fn modes(value: &Component) -> Result<Vec<MaskMode>, StyleError> {
    value
        .items()
        .iter()
        .map(|item| match item.keyword() {
            Some(Keyword::Alpha) => Ok(MaskMode::Alpha),
            Some(Keyword::Luminance) => Ok(MaskMode::Luminance),
            Some(Keyword::MatchSource) => Ok(MaskMode::MatchSource),
            _ => Err(unsupported(item)),
        })
        .collect()
}

pub(super) fn composites(value: &Component) -> Result<Vec<MaskComposite>, StyleError> {
    value
        .items()
        .iter()
        .map(|item| match item.keyword() {
            Some(Keyword::Add) => Ok(MaskComposite::Add),
            Some(Keyword::Subtract) => Ok(MaskComposite::Subtract),
            Some(Keyword::Intersect) => Ok(MaskComposite::Intersect),
            Some(Keyword::Exclude) => Ok(MaskComposite::Exclude),
            _ => Err(unsupported(item)),
        })
        .collect()
}

/// `mask-border-mode`: `luminance | alpha`.
pub(super) fn border_mode(value: &Component) -> Result<MaskMode, StyleError> {
    match value.keyword() {
        Some(Keyword::Alpha) => Ok(MaskMode::Alpha),
        Some(Keyword::Luminance) => Ok(MaskMode::Luminance),
        _ => Err(unsupported(value)),
    }
}
