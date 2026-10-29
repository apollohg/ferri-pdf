//! `ruby-position`, `ruby-merge`, `ruby-align` and `ruby-overhang` (CSS Ruby 1 §§4–5).
use crate::style::{StyleError, box_values::unsupported, component::Component, grammar::Keyword};
use serde::Serialize;

/// `ruby-position` (§4.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) enum RubyPosition {
    /// Over or `under` the base, or with `alternate` opposite the previous level.
    Interlinear {
        alternate: bool,
        under: bool,
    },
    InterCharacter,
}
impl Default for RubyPosition {
    fn default() -> Self {
        Self::Interlinear {
            alternate: true,
            under: false,
        }
    }
}
impl RubyPosition {
    fn from_component(value: &Component) -> Result<Self, StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        if keywords == [Keyword::InterCharacter] {
            return Ok(Self::InterCharacter);
        }
        let mut alternate = false;
        let mut under = false;
        for keyword in keywords {
            match keyword {
                Keyword::Alternate => alternate = true,
                Keyword::Over => under = false,
                Keyword::Under => under = true,
                _ => return Err(unsupported(value)),
            }
        }
        Ok(Self::Interlinear { alternate, under })
    }
    /// Whether an interlinear level with this position lies under its base, after the
    /// interlinear level `previous` in its segment, which lies under where `Some(true)`.
    pub(crate) fn under(self, previous: Option<bool>) -> bool {
        match (self, previous) {
            (
                Self::Interlinear {
                    alternate: true, ..
                },
                Some(previous),
            ) => !previous,
            (Self::Interlinear { under, .. }, _) => under,
            (Self::InterCharacter, _) => false,
        }
    }
}

/// `ruby-merge` (§4.2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum RubyMerge {
    #[default]
    Separate,
    Merge,
    /// Separate where every annotation fits over its bases, else merged (§4.2: the
    /// simplest of the conventions `auto` permits).
    Auto,
}

/// `ruby-align` (§4.3).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum RubyAlign {
    Start,
    Center,
    SpaceBetween,
    #[default]
    SpaceAround,
}

/// The computed ruby properties, which all inherit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RubyStyle {
    pub position: RubyPosition,
    pub merge: RubyMerge,
    pub align: RubyAlign,
    /// `ruby-overhang: auto`; `false` is `none` (§5.1).
    pub overhang: bool,
}
impl Default for RubyStyle {
    fn default() -> Self {
        Self {
            position: RubyPosition::default(),
            merge: RubyMerge::default(),
            align: RubyAlign::default(),
            overhang: true,
        }
    }
}
impl RubyStyle {
    pub(super) fn position(value: &Component) -> Result<RubyPosition, StyleError> {
        RubyPosition::from_component(value)
    }
    pub(super) fn merge(value: &Component) -> Result<RubyMerge, StyleError> {
        match value.keyword() {
            Some(Keyword::Separate) => Ok(RubyMerge::Separate),
            Some(Keyword::Merge) => Ok(RubyMerge::Merge),
            Some(Keyword::Auto) => Ok(RubyMerge::Auto),
            _ => Err(unsupported(value)),
        }
    }
    pub(super) fn align(value: &Component) -> Result<RubyAlign, StyleError> {
        match value.keyword() {
            Some(Keyword::Start) => Ok(RubyAlign::Start),
            Some(Keyword::Center) => Ok(RubyAlign::Center),
            Some(Keyword::SpaceBetween) => Ok(RubyAlign::SpaceBetween),
            Some(Keyword::SpaceAround) => Ok(RubyAlign::SpaceAround),
            _ => Err(unsupported(value)),
        }
    }
    pub(super) fn overhang(value: &Component) -> Result<bool, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(true),
            Some(Keyword::None) => Ok(false),
            _ => Err(unsupported(value)),
        }
    }
}
