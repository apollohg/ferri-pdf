//! CSS Inline 3 §4 baseline alignment: `dominant-baseline`, `alignment-baseline`,
//! `baseline-shift` and `baseline-source`.
use crate::style::{
    LengthPercentage, StyleError,
    box_values::{length_percentage, unsupported},
    component::Component,
    grammar::Keyword,
    units::Relative,
};
use serde::Serialize;

/// A baseline of a font or box (CSS Inline 3 §3.2); horizontal writing modes only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum Baseline {
    #[default]
    Alphabetic,
    /// The text-under baseline.
    TextBottom,
    /// The text-over baseline.
    TextTop,
    /// The ideographic-under baseline.
    Ideographic,
    /// The x-middle baseline.
    Middle,
    Central,
    Mathematical,
    Hanging,
}
impl Baseline {
    /// `dominant-baseline`; `auto` is alphabetic in horizontal writing modes (§4.1).
    pub(super) fn dominant(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto | Keyword::Alphabetic) => Ok(Self::Alphabetic),
            Some(Keyword::Hanging) => Ok(Self::Hanging),
            _ => Self::shared(value),
        }
    }
    /// `alignment-baseline`; `None` is `baseline`, the parent's dominant baseline. SVG's
    /// legacy `text-before-edge` and `text-after-edge` alias `text-top` and `text-bottom`
    /// (§4.2.2.1).
    pub(super) fn alignment(value: &Component) -> Result<Option<Self>, StyleError> {
        match value.keyword() {
            Some(Keyword::Baseline) => Ok(None),
            Some(Keyword::Alphabetic) => Ok(Some(Self::Alphabetic)),
            Some(Keyword::TextBeforeEdge) => Ok(Some(Self::TextTop)),
            Some(Keyword::TextAfterEdge) => Ok(Some(Self::TextBottom)),
            _ => Self::shared(value).map(Some),
        }
    }
    fn shared(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::TextBottom) => Ok(Self::TextBottom),
            Some(Keyword::TextTop) => Ok(Self::TextTop),
            Some(Keyword::Ideographic) => Ok(Self::Ideographic),
            Some(Keyword::Middle) => Ok(Self::Middle),
            Some(Keyword::Central) => Ok(Self::Central),
            Some(Keyword::Mathematical) => Ok(Self::Mathematical),
            _ => Err(unsupported(value)),
        }
    }
}

/// A computed `baseline-shift`; `baseline` computes to zero (§4.2.3.1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum BaselineShift {
    /// Raises the box; percentages refer to its line height.
    Raise(LengthPercentage),
    Sub,
    Super,
}
impl Default for BaselineShift {
    fn default() -> Self {
        Self::Raise(LengthPercentage::ZERO)
    }
}
impl BaselineShift {
    pub(super) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Sub) => Ok(Self::Sub),
            Some(Keyword::Super) => Ok(Self::Super),
            Some(Keyword::Baseline) => Ok(Self::default()),
            Some(_) => Err(unsupported(value)),
            None => length_percentage(value, relative).map(Self::Raise),
        }
    }
}

/// `baseline-source`: which baseline set of a box with several aligns it (§4.2.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum BaselineSource {
    /// The last baseline set for inline blocks, the first for other boxes.
    #[default]
    Auto,
    First,
    Last,
}
impl BaselineSource {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::First) => Ok(Self::First),
            Some(Keyword::Last) => Ok(Self::Last),
            _ => Err(unsupported(value)),
        }
    }
}

/// A box's baseline alignment, beside its legacy `vertical-align`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct BaselineAlignment {
    /// `dominant-baseline`, inherited.
    pub dominant: Baseline,
    /// `alignment-baseline`; `None` is `baseline`.
    pub alignment: Option<Baseline>,
    pub shift: BaselineShift,
    pub source: BaselineSource,
}

/// A `text-box-edge` metric: an over edge, or with `alphabetic` an under edge (§6.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum TextEdge {
    /// The text-over or text-under baseline: the ascent or descent.
    #[default]
    Text,
    Cap,
    Ex,
    Ideographic,
    IdeographicInk,
    Alphabetic,
}

/// A computed `text-box-trim` and `text-box-edge` (§6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct TextBox {
    /// Whether the block-start and block-end sides are trimmed.
    pub trim: [bool; 2],
    /// The over and under edges trimmed to; `auto` is `text`, since `line-fit-edge` is
    /// outside the pinned grammar and its initial `leading` reads as `text`.
    pub edges: [TextEdge; 2],
}
impl TextBox {
    pub(super) fn trim(value: &Component) -> Result<[bool; 2], StyleError> {
        match value.keyword() {
            Some(Keyword::None) => Ok([false, false]),
            Some(Keyword::TrimStart) => Ok([true, false]),
            Some(Keyword::TrimEnd) => Ok([false, true]),
            Some(Keyword::TrimBoth) => Ok([true, true]),
            _ => Err(unsupported(value)),
        }
    }
    /// One keyword names both edges where it can, otherwise the under edge is `text`
    /// (CSS Inline 3 §5.2 `<text-edge>`).
    pub(super) fn edges(value: &Component) -> Result<[TextEdge; 2], StyleError> {
        if value.keyword() == Some(Keyword::Auto) {
            return Ok([TextEdge::Text; 2]);
        }
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let edge = |keyword: Keyword| match keyword {
            Keyword::Text => Ok(TextEdge::Text),
            Keyword::Cap => Ok(TextEdge::Cap),
            Keyword::Ex => Ok(TextEdge::Ex),
            Keyword::Ideographic => Ok(TextEdge::Ideographic),
            Keyword::IdeographicInk => Ok(TextEdge::IdeographicInk),
            Keyword::Alphabetic => Ok(TextEdge::Alphabetic),
            _ => Err(unsupported(value)),
        };
        match keywords[..] {
            [over] => {
                let over = edge(over)?;
                let under = match over {
                    TextEdge::Cap | TextEdge::Ex => TextEdge::Text,
                    shared => shared,
                };
                Ok([over, under])
            }
            [over, under] => Ok([edge(over)?, edge(under)?]),
            _ => Err(unsupported(value)),
        }
    }
}
