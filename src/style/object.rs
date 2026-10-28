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
