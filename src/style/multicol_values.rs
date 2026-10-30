//! CSS Multi-column Layout 1 computed values.
use crate::paint::{Border, BorderStyle, Color, DEFAULT_BORDER_WIDTH};
use crate::style::{StyleError, box_values::unsupported, component::Component, grammar::Keyword};
use serde::Serialize;

/// `column-wrap` (CSS Multi-column Layout 2 §4.4).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum ColumnWrap {
    /// `wrap` with a `column-height`, otherwise `nowrap`.
    #[default]
    Auto,
    NoWrap,
    Wrap,
}
impl ColumnWrap {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Nowrap) => Ok(Self::NoWrap),
            Some(Keyword::Wrap) => Ok(Self::Wrap),
            _ => Err(unsupported(value)),
        }
    }
}

/// `column-fill`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum ColumnFill {
    Auto,
    #[default]
    Balance,
    BalanceAll,
}
impl ColumnFill {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Balance) => Ok(Self::Balance),
            Some(Keyword::BalanceAll) => Ok(Self::BalanceAll),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct MulticolStyle {
    /// `column-count`; `None` is `auto`.
    pub count: Option<u32>,
    /// `column-width` in points; `None` is `auto`.
    pub width: Option<f32>,
    /// `column-height` in points; `None` is `auto`.
    pub height: Option<f32>,
    pub wrap: ColumnWrap,
    pub fill: ColumnFill,
    /// `column-span: all`.
    pub span_all: bool,
    /// `column-rule-width`, `column-rule-style` and `column-rule-color`.
    pub rule: Border,
}
impl Default for MulticolStyle {
    fn default() -> Self {
        Self {
            count: None,
            width: None,
            height: None,
            wrap: ColumnWrap::Auto,
            fill: ColumnFill::Balance,
            span_all: false,
            rule: Border {
                width: DEFAULT_BORDER_WIDTH,
                style: BorderStyle::None,
                color: Color::BLACK,
            },
        }
    }
}
impl MulticolStyle {
    /// A block container with a non-`auto` column count, width or height is a multi-column
    /// container (CSS Multi-column Layout 2 §3).
    pub(crate) fn is_multicol(&self) -> bool {
        self.count.is_some() || self.width.is_some() || self.height.is_some()
    }
    /// Whether overflowing content starts a new row of columns in the block direction rather
    /// than overflow columns in the inline direction (§4.4).
    pub(crate) fn wraps(&self) -> bool {
        match self.wrap {
            ColumnWrap::Auto => self.height.is_some(),
            ColumnWrap::NoWrap => false,
            ColumnWrap::Wrap => true,
        }
    }
}

/// `auto | <integer [1,∞]>`.
pub(crate) fn count(
    value: &Component,
    relative: &crate::style::units::Relative<'_>,
) -> Result<Option<u32>, StyleError> {
    if value.keyword() == Some(Keyword::Auto) {
        return Ok(None);
    }
    u32::try_from(crate::style::values::integer(value, relative)?)
        .ok()
        .filter(|count| *count > 0)
        .map(Some)
        .ok_or_else(|| format!("invalid column count: {value}").into())
}

/// `column-width` and `column-height`: `auto | <length [0,∞]>`.
pub(crate) fn length(
    value: &Component,
    relative: &crate::style::units::Relative<'_>,
) -> Result<Option<f32>, StyleError> {
    if value.keyword() == Some(Keyword::Auto) {
        return Ok(None);
    }
    let width = crate::style::values::length(value, relative)?;
    if !width.is_finite() || width < 0.0 {
        return Err(format!("invalid column length: {value}").into());
    }
    Ok(Some(width))
}
