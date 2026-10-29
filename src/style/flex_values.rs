use crate::style::{
    LengthPercentage, Size, StyleError,
    box_values::{length_percentage, unsupported},
    component::Component,
    grammar::{Keyword, Label, Production},
    units::Relative,
};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}
impl FlexDirection {
    pub(crate) fn column(self) -> bool {
        matches!(self, Self::Column | Self::ColumnReverse)
    }
    pub(crate) fn reverse(self) -> bool {
        matches!(self, Self::RowReverse | Self::ColumnReverse)
    }
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Row) => Ok(Self::Row),
            Some(Keyword::RowReverse) => Ok(Self::RowReverse),
            Some(Keyword::Column) => Ok(Self::Column),
            Some(Keyword::ColumnReverse) => Ok(Self::ColumnReverse),
            _ => Err(unsupported(value)),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}
impl FlexWrap {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Nowrap) => Ok(Self::NoWrap),
            Some(Keyword::Wrap) => Ok(Self::Wrap),
            Some(Keyword::WrapReverse) => Ok(Self::WrapReverse),
            _ => Err(unsupported(value)),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum Alignment {
    Auto,
    Normal,
    Start,
    End,
    FlexStart,
    FlexEnd,
    /// Start and end in the item's own writing mode.
    SelfStart,
    SelfEnd,
    Left,
    Right,
    Center,
    Stretch,
    /// First-baseline alignment.
    Baseline,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}
impl Alignment {
    /// Each property's grammar admits only its own keywords; overflow positions, `last`
    /// baselines and anchor centring are not modelled.
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        if value
            .find(Label::Production(Production::OverflowPosition))
            .is_some()
        {
            return Err(unsupported(value));
        }
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        Self::from_keywords(&keywords).ok_or_else(|| unsupported(value))
    }
    /// `align-content`, whose alignment may be explicitly `unsafe`: content overflows
    /// both edges equally. `safe` alignment is not modelled.
    pub(crate) fn content_from_component(value: &Component) -> Result<(Self, bool), StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let (unsafe_overflow, keywords) = match keywords.split_first() {
            Some((Keyword::Unsafe, rest)) => (true, rest),
            _ => (false, keywords.as_slice()),
        };
        match Self::from_keywords(keywords) {
            Some(alignment) if !keywords.contains(&Keyword::Safe) => {
                Ok((alignment, unsafe_overflow))
            }
            _ => Err(unsupported(value)),
        }
    }
    fn from_keywords(keywords: &[Keyword]) -> Option<Self> {
        Some(match keywords {
            [Keyword::Auto] => Self::Auto,
            [Keyword::Normal] => Self::Normal,
            [Keyword::Start] => Self::Start,
            [Keyword::End] => Self::End,
            [Keyword::FlexStart] => Self::FlexStart,
            [Keyword::FlexEnd] => Self::FlexEnd,
            [Keyword::SelfStart] => Self::SelfStart,
            [Keyword::SelfEnd] => Self::SelfEnd,
            [Keyword::Left] => Self::Left,
            [Keyword::Right] => Self::Right,
            [Keyword::Center] => Self::Center,
            [Keyword::Stretch] => Self::Stretch,
            [Keyword::Baseline] | [Keyword::First, Keyword::Baseline] => Self::Baseline,
            [Keyword::SpaceBetween] => Self::SpaceBetween,
            [Keyword::SpaceAround] => Self::SpaceAround,
            [Keyword::SpaceEvenly] => Self::SpaceEvenly,
            _ => return None,
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) enum FlexBasis {
    #[default]
    Auto,
    Content,
    Value(LengthPercentage),
    Intrinsic(crate::style::IntrinsicSizing),
}
impl FlexBasis {
    pub(crate) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::Content) {
            return Ok(Self::Content);
        }
        Ok(match Size::from_component(value, relative)? {
            Size::Auto => Self::Auto,
            Size::Value(basis) => Self::Value(basis),
            Size::Intrinsic(basis) => Self::Intrinsic(basis),
        })
    }
    pub(crate) fn resolve(&self, basis: Option<f32>) -> Option<f32> {
        match self {
            Self::Value(LengthPercentage::Length(value)) => Some(*value),
            Self::Value(value) => basis.map(|basis| value.resolve(basis)),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct FlexStyle {
    pub direction: FlexDirection,
    pub wrap: FlexWrap,
    pub justify: Alignment,
    pub align_items: Alignment,
    pub align_content: Alignment,
    /// `align-content` names `unsafe` overflow alignment.
    pub align_content_unsafe: bool,
    pub align_self: Alignment,
    pub gap: [Gap; 2],
    pub grow: f32,
    pub shrink: f32,
    pub basis: FlexBasis,
    pub order: i32,
    /// The computed `display` is `-webkit-box` or `-webkit-inline-box`.
    pub legacy_box: bool,
}
impl Default for FlexStyle {
    fn default() -> Self {
        Self {
            direction: FlexDirection::Row,
            wrap: FlexWrap::NoWrap,
            justify: Alignment::Normal,
            align_items: Alignment::Normal,
            align_content: Alignment::Normal,
            align_content_unsafe: false,
            align_self: Alignment::Auto,
            gap: [const { Gap::Normal }; 2],
            grow: 0.0,
            shrink: 1.0,
            basis: FlexBasis::Auto,
            order: 0,
            legacy_box: false,
        }
    }
}
/// `flex-grow` and `flex-shrink`; CSS Flexbox 1 §7.3 makes negative factors invalid.
pub(crate) fn factor(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    let factor = crate::style::values::number(value, relative)?;
    if !factor.is_finite() || factor < 0.0 {
        return Err(format!("invalid flex factor: {value}").into());
    }
    Ok(factor)
}
/// Computed `row-gap` or `column-gap`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum Gap {
    Normal,
    Length(LengthPercentage),
}
impl Gap {
    /// The used gap outside multi-column layout, where `normal` is zero (CSS Box Alignment 3
    /// §8.1).
    pub(crate) fn resolve(&self, basis: f32) -> f32 {
        self.resolve_normal(basis, 0.0)
    }
    /// The used gap where `normal` is `normal_gap`, as `1em` between columns of a
    /// multi-column container.
    pub(crate) fn resolve_normal(&self, basis: f32, normal_gap: f32) -> f32 {
        match self {
            Self::Normal => normal_gap,
            Self::Length(gap) => gap.resolve(basis),
        }
    }
}
/// `row-gap` and `column-gap`.
pub(crate) fn gap(value: &Component, relative: &Relative<'_>) -> Result<Gap, StyleError> {
    if value.keyword() == Some(Keyword::Normal) {
        return Ok(Gap::Normal);
    }
    let gap = length_percentage(value, relative)?;
    if gap.resolve(1.0) < 0.0 {
        return Err(format!("negative gap: {value}").into());
    }
    Ok(Gap::Length(gap))
}
