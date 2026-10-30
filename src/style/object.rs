use crate::style::{StyleError, box_values::unsupported, component::Component, grammar::Keyword};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum ObjectFit {
    #[default]
    Fill,
    Contain,
    Cover,
    None,
    ScaleDown,
}
impl ObjectFit {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Fill) => Ok(Self::Fill),
            Some(Keyword::Contain) => Ok(Self::Contain),
            Some(Keyword::Cover) => Ok(Self::Cover),
            Some(Keyword::None) => Ok(Self::None),
            Some(Keyword::ScaleDown) => Ok(Self::ScaleDown),
            _ => Err(unsupported(value)),
        }
    }
    pub(crate) fn size(self, intrinsic: [f32; 2], bounds: [f32; 2]) -> [f32; 2] {
        let [width, height] = intrinsic;
        let scale = match self {
            Self::Fill => return bounds,
            Self::None => 1.0,
            Self::Contain => (bounds[0] / width).min(bounds[1] / height),
            Self::Cover => (bounds[0] / width).max(bounds[1] / height),
            Self::ScaleDown => (bounds[0] / width).min(bounds[1] / height).min(1.0),
        };
        [width * scale, height * scale]
    }
}

/// `[ from-image || <resolution> ] && snap?` slots.
const SOURCE_SLOT: usize = 0;
const FROM_IMAGE_SLOT: usize = 0;
const RESOLUTION_SLOT: usize = 1;
/// The initial `image-resolution`, one image pixel per CSS pixel.
const INITIAL_DPPX: f32 = 1.0;

/// Computed `image-resolution` (CSS Images 4 §6.1). `snap` rounds a resolution to map an image
/// pixel to whole device pixels, and vector PDF output has no device pixels to snap to
/// (`resolution: infinite`, Media Queries 4 §5.1), so it leaves every resolution unchanged.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub(crate) struct ImageResolution {
    pub from_image: bool,
    /// Dots per CSS pixel.
    pub dppx: Option<f32>,
}
impl Default for ImageResolution {
    fn default() -> Self {
        Self {
            from_image: false,
            dppx: Some(INITIAL_DPPX),
        }
    }
}
impl ImageResolution {
    pub(crate) fn from_component(
        value: &Component,
        relative: &crate::style::units::Relative<'_>,
    ) -> Result<Self, StyleError> {
        let source = value.slot(SOURCE_SLOT).ok_or_else(|| unsupported(value))?;
        let dppx = source
            .slot(RESOLUTION_SLOT)
            .map(|resolution| crate::style::values::resolution(resolution, relative))
            .transpose()?;
        if dppx.is_some_and(|dppx| dppx <= 0.0) {
            return Err(StyleError::Invalid(format!(
                "non-positive image-resolution: {value}"
            )));
        }
        Ok(Self {
            from_image: source.slot(FROM_IMAGE_SLOT).is_some(),
            dppx,
        })
    }
    /// Image pixels per CSS pixel along each axis of an image that records `recorded`.
    pub(crate) fn density(self, recorded: Option<[f32; 2]>) -> [f32; 2] {
        match recorded.filter(|_| self.from_image) {
            Some(recorded) => recorded,
            None => [self.dppx.unwrap_or(INITIAL_DPPX); 2],
        }
    }
}

/// `[ <angle> || flip ]` slots.
const ANGLE_SLOT: usize = 0;
const FLIP_SLOT: usize = 1;
const QUARTER_TURN_DEG: f32 = 90.0;
const QUARTER_TURNS: f32 = 4.0;

/// Computed `image-orientation` (CSS Images 3 §5.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum ImageOrientation {
    #[default]
    FromImage,
    None,
    /// Clockwise quarter turns in `0..4`, then a horizontal flip.
    Turned {
        turns: u8,
        flip: bool,
    },
}
impl ImageOrientation {
    pub(crate) fn from_component(
        value: &Component,
        relative: &crate::style::units::Relative<'_>,
    ) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::FromImage) => return Ok(Self::FromImage),
            Some(Keyword::None) => return Ok(Self::None),
            _ => {}
        }
        // The angle rounds to the nearest quarter turn, up when halfway, modulo a turn.
        let degrees = value
            .slot(ANGLE_SLOT)
            .map(|angle| crate::style::values::angle(angle, relative))
            .transpose()?
            .unwrap_or_default();
        let turns = (degrees / QUARTER_TURN_DEG + 0.5)
            .floor()
            .rem_euclid(QUARTER_TURNS) as u8;
        Ok(Self::Turned {
            turns,
            flip: value.slot(FLIP_SLOT).is_some(),
        })
    }
}
