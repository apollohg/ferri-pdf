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
