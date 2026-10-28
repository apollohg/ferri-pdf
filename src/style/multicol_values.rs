//! CSS Multi-column Layout 1 computed values.
use crate::paint::{Border, BorderStyle, Color, DEFAULT_BORDER_WIDTH};
use crate::style::{StyleError, box_values::unsupported, component::Component, grammar::Keyword};
use serde::Serialize;

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
    /// A block container with a non-`auto` column count or width is a multi-column container
    /// (CSS Multi-column Layout 1 §2).
    pub(crate) fn is_multicol(&self) -> bool {
        self.count.is_some() || self.width.is_some()
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

/// `auto | <length [0,∞]>`.
pub(crate) fn width(
    value: &Component,
    relative: &crate::style::units::Relative<'_>,
) -> Result<Option<f32>, StyleError> {
    if value.keyword() == Some(Keyword::Auto) {
        return Ok(None);
    }
    let width = crate::style::values::length(value, relative)?;
    if !width.is_finite() || width < 0.0 {
        return Err(format!("invalid column width: {value}").into());
    }
    Ok(Some(width))
}
