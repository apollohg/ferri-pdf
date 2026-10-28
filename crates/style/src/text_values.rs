use crate::{
    StyleError, box_values::unsupported, calc::Kind, component::Component, grammar::Keyword,
    units::Relative,
};
use serde::Serialize;

pub const DEFAULT_TAB_SIZE: usize = 8;
/// `white-space` shorthand slots (CSS Text 4 §3).
const COLLAPSE_SLOT: usize = 0;
const WRAP_MODE_SLOT: usize = 1;
/// `text-transform` case slot.
const CASE_SLOT: usize = 0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnicodeBidi {
    #[default]
    Normal,
    Embed,
    Isolate,
    BidiOverride,
    IsolateOverride,
    Plaintext,
}
impl UnicodeBidi {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::Embed) => Ok(Self::Embed),
            Some(Keyword::Isolate) => Ok(Self::Isolate),
            Some(Keyword::BidiOverride) => Ok(Self::BidiOverride),
            Some(Keyword::IsolateOverride) => Ok(Self::IsolateOverride),
            Some(Keyword::Plaintext) => Ok(Self::Plaintext),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Hyphens {
    None,
    #[default]
    Manual,
}
impl Hyphens {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::None) => Ok(Self::None),
            Some(Keyword::Manual) => Ok(Self::Manual),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WhiteSpace {
    #[default]
    Normal,
    Nowrap,
    Pre,
    PreWrap,
    PreLine,
    BreakSpaces,
    BreakSpacesNowrap,
}
impl WhiteSpace {
    pub fn collapses_spaces(self) -> bool {
        matches!(self, Self::Normal | Self::Nowrap | Self::PreLine)
    }
    pub fn preserves_breaks(self) -> bool {
        !matches!(self, Self::Normal | Self::Nowrap)
    }
    pub fn wraps(self) -> bool {
        !matches!(self, Self::Pre | Self::Nowrap | Self::BreakSpacesNowrap)
    }
    /// The legacy keywords and the collapse and wrap-mode pairs they abbreviate.
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        let pair = match value.keyword() {
            Some(Keyword::Normal) => return Ok(Self::Normal),
            Some(Keyword::Pre) => return Ok(Self::Pre),
            Some(Keyword::PreWrap) => return Ok(Self::PreWrap),
            Some(Keyword::PreLine) => return Ok(Self::PreLine),
            _ => (
                value
                    .slot(COLLAPSE_SLOT)
                    .map_or(Some(Keyword::Collapse), Component::keyword),
                value
                    .slot(WRAP_MODE_SLOT)
                    .map_or(Some(Keyword::Wrap), Component::keyword),
            ),
        };
        match pair {
            (Some(Keyword::Collapse), Some(Keyword::Wrap)) => Ok(Self::Normal),
            (Some(Keyword::Collapse), Some(Keyword::Nowrap)) => Ok(Self::Nowrap),
            (Some(Keyword::Preserve), Some(Keyword::Nowrap)) => Ok(Self::Pre),
            (Some(Keyword::Preserve), Some(Keyword::Wrap)) => Ok(Self::PreWrap),
            (Some(Keyword::PreserveBreaks), Some(Keyword::Wrap)) => Ok(Self::PreLine),
            (Some(Keyword::BreakSpaces), Some(Keyword::Wrap)) => Ok(Self::BreakSpaces),
            (Some(Keyword::BreakSpaces), Some(Keyword::Nowrap)) => Ok(Self::BreakSpacesNowrap),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    #[default]
    Start,
    End,
    Left,
    Right,
    Center,
    Justify,
}
impl TextAlign {
    pub fn offset(self, available: f32, width: f32, rtl: bool) -> f32 {
        match self {
            Self::Right => available - width,
            Self::Start | Self::Justify if rtl => available - width,
            Self::End if !rtl => available - width,
            Self::Center => (available - width) / 2.0,
            _ => 0.0,
        }
    }
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Start) => Ok(Self::Start),
            Some(Keyword::End) => Ok(Self::End),
            Some(Keyword::Left) => Ok(Self::Left),
            Some(Keyword::Right) => Ok(Self::Right),
            Some(Keyword::Center) => Ok(Self::Center),
            Some(Keyword::Justify) => Ok(Self::Justify),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub enum LineHeight {
    #[default]
    Normal,
    Number(f32),
    Length(f32),
}
impl LineHeight {
    pub fn resolve(self, font_size: f32) -> Option<f32> {
        match self {
            Self::Normal => None,
            Self::Number(factor) => Some(factor * font_size),
            Self::Length(length) => Some(length),
        }
    }
    pub(super) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        match value.unwrap() {
            Component::Keyword(Keyword::Normal) => Ok(Self::Normal),
            number @ (Component::Number(_) | Component::Math(_, Kind::Number, _)) => {
                Ok(Self::Number(crate::values::number(number, relative)?))
            }
            other => {
                let height = crate::box_values::font_length_percentage(other, relative)?;
                if !height.is_finite() {
                    return Err("non-finite line height".into());
                }
                Ok(Self::Length(height))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TextCase {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct TextTransform {
    pub case: TextCase,
    pub full_width: bool,
    pub full_size_kana: bool,
}
impl TextTransform {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::None) {
            return Ok(Self::default());
        }
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let case = match value.slot(CASE_SLOT).and_then(Component::keyword) {
            Some(Keyword::Uppercase) => TextCase::Uppercase,
            Some(Keyword::Lowercase) => TextCase::Lowercase,
            Some(Keyword::Capitalize) => TextCase::Capitalize,
            None => TextCase::None,
            _ => return Err(unsupported(value)),
        };
        let transform = Self {
            case,
            full_width: keywords.contains(&Keyword::FullWidth),
            full_size_kana: keywords.contains(&Keyword::FullSizeKana),
        };
        if transform == Self::default() {
            Err(unsupported(value))
        } else {
            Ok(transform)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverflowWrap {
    #[default]
    Normal,
    Anywhere,
    BreakWord,
}
impl OverflowWrap {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::Anywhere) => Ok(Self::Anywhere),
            Some(Keyword::BreakWord) => Ok(Self::BreakWord),
            _ => Err(unsupported(value)),
        }
    }
}

/// Logical start-edge margin of selected formatted lines.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TextIndent {
    pub amount: crate::LengthPercentage,
    pub hanging: bool,
    pub each_line: bool,
}
impl Default for TextIndent {
    fn default() -> Self {
        Self {
            amount: crate::LengthPercentage::ZERO,
            hanging: false,
            each_line: false,
        }
    }
}
impl TextIndent {
    pub(crate) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        const AMOUNT: usize = 0;
        let amount = value.slot(AMOUNT).ok_or_else(|| unsupported(value))?;
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        Ok(Self {
            amount: crate::box_values::length_percentage(amount, relative)?,
            hanging: keywords.contains(&Keyword::Hanging),
            each_line: keywords.contains(&Keyword::EachLine),
        })
    }
    pub fn resolve(&self, basis: f32, first: bool, after_forced_break: bool) -> f32 {
        if (first || (self.each_line && after_forced_break)) == self.hanging {
            0.0
        } else {
            self.amount.resolve(basis)
        }
    }
}
