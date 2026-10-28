//! Compositing 1 blending and isolation values.
use crate::paint::BlendMode;
use crate::style::{StyleError, box_values::unsupported, component::Component, grammar::Keyword};

/// `<blend-mode>`.
pub(super) fn blend_mode(value: &Component) -> Result<BlendMode, StyleError> {
    Ok(match value.keyword() {
        Some(Keyword::Normal) => BlendMode::Normal,
        Some(Keyword::Multiply) => BlendMode::Multiply,
        Some(Keyword::Screen) => BlendMode::Screen,
        Some(Keyword::Overlay) => BlendMode::Overlay,
        Some(Keyword::Darken) => BlendMode::Darken,
        Some(Keyword::Lighten) => BlendMode::Lighten,
        Some(Keyword::ColorDodge) => BlendMode::ColorDodge,
        Some(Keyword::ColorBurn) => BlendMode::ColorBurn,
        Some(Keyword::HardLight) => BlendMode::HardLight,
        Some(Keyword::SoftLight) => BlendMode::SoftLight,
        Some(Keyword::Difference) => BlendMode::Difference,
        Some(Keyword::Exclusion) => BlendMode::Exclusion,
        Some(Keyword::Hue) => BlendMode::Hue,
        Some(Keyword::Saturation) => BlendMode::Saturation,
        Some(Keyword::Color) => BlendMode::Color,
        Some(Keyword::Luminosity) => BlendMode::Luminosity,
        Some(Keyword::PlusLighter | Keyword::PlusDarker) => {
            return Err(StyleError::Unsupported(
                "additive compositing, which no PDF blend mode or transparency group \
                 expresses (PDF 32000 §11.3.5)"
                    .into(),
            ));
        }
        _ => return Err(unsupported(value)),
    })
}

/// `<blend-mode>#`, one per background layer.
pub(super) fn blend_modes(value: &Component) -> Result<Vec<BlendMode>, StyleError> {
    value.items().iter().map(blend_mode).collect()
}

/// `<isolation-mode>` as whether the element is isolated.
pub(super) fn isolation(value: &Component) -> Result<bool, StyleError> {
    match value.keyword() {
        Some(Keyword::Isolate) => Ok(true),
        Some(Keyword::Auto) => Ok(false),
        _ => Err(unsupported(value)),
    }
}
