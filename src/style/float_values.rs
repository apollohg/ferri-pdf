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
    /// Moves the element to the page's footnote area (CSS GCPM 3 §2.3).
    Footnote,
}

impl Float {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::None) => Ok(Self::None),
            Some(Keyword::Left) => Ok(Self::Left),
            Some(Keyword::Right) => Ok(Self::Right),
            Some(Keyword::InlineStart) => Ok(Self::InlineStart),
            Some(Keyword::InlineEnd) => Ok(Self::InlineEnd),
            Some(Keyword::Footnote) => Ok(Self::Footnote),
            _ => Err(unsupported(value)),
        }
    }

    pub(crate) fn side(self, rtl: bool) -> Option<FloatSide> {
        match self {
            Self::None | Self::Footnote => None,
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

/// `footnote-display` (CSS GCPM 3 §2.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum FootnoteDisplay {
    #[default]
    Block,
    Inline,
    /// Inline when footnotes fit on one line of the footnote area, otherwise block.
    Compact,
}
impl FootnoteDisplay {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Block) => Ok(Self::Block),
            Some(Keyword::Inline) => Ok(Self::Inline),
            Some(Keyword::Compact) => Ok(Self::Compact),
            _ => Err(unsupported(value)),
        }
    }
}

/// `footnote-policy` (CSS GCPM 3 §2.8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum FootnotePolicy {
    /// The footnote body may follow its call onto later pages.
    #[default]
    Auto,
    /// The line holding the call moves to the page that fits its footnote.
    Line,
    /// The paragraph holding the call moves to the page that fits its footnote.
    Block,
}
impl FootnotePolicy {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Line) => Ok(Self::Line),
            Some(Keyword::Block) => Ok(Self::Block),
            _ => Err(unsupported(value)),
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
