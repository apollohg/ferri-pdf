use crate::style::{StyleError, box_values::unsupported, component::Component, grammar::Keyword};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) enum FloatSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum Float {
    #[default]
    None,
    Left,
    Right,
    InlineStart,
    InlineEnd,
}

impl Float {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::None) => Ok(Self::None),
            Some(Keyword::Left) => Ok(Self::Left),
            Some(Keyword::Right) => Ok(Self::Right),
            Some(Keyword::InlineStart) => Ok(Self::InlineStart),
            Some(Keyword::InlineEnd) => Ok(Self::InlineEnd),
            _ => Err(unsupported(value)),
        }
    }

    pub(crate) fn side(self, rtl: bool) -> Option<FloatSide> {
        match self {
            Self::None => None,
            Self::Left => Some(FloatSide::Left),
            Self::Right => Some(FloatSide::Right),
            Self::InlineStart => Some(if rtl {
                FloatSide::Right
            } else {
                FloatSide::Left
            }),
            Self::InlineEnd => Some(if rtl {
                FloatSide::Left
            } else {
                FloatSide::Right
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum Clear {
    #[default]
    None,
    Left,
    Right,
    Both,
    InlineStart,
    InlineEnd,
}

impl Clear {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::None) => Ok(Self::None),
            Some(Keyword::Left) => Ok(Self::Left),
            Some(Keyword::Right) => Ok(Self::Right),
            Some(Keyword::Both) => Ok(Self::Both),
            Some(Keyword::InlineStart) => Ok(Self::InlineStart),
            Some(Keyword::InlineEnd) => Ok(Self::InlineEnd),
            _ => Err(unsupported(value)),
        }
    }

    pub(crate) fn clears(self, side: FloatSide, rtl: bool) -> bool {
        let selected = match self {
            Self::None => return false,
            Self::Both => return true,
            Self::Left => Float::Left,
            Self::Right => Float::Right,
            Self::InlineStart => Float::InlineStart,
            Self::InlineEnd => Float::InlineEnd,
        };
        selected.side(rtl) == Some(side)
    }
}
