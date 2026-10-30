use crate::style::{
    LengthPercentage, Size, StyleError,
    box_values::{length_percentage, unsupported},
    component::Component,
    grammar::Keyword,
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
    /// CSS Anchor Positioning 1 §4.2: centred over the default anchor box; `center` for boxes
    /// that are not absolutely positioned or have no default anchor box.
    AnchorCenter,
}
impl Alignment {
    /// An alignment and its `<overflow-position>`; each property's grammar admits only its
    /// own keywords, and `last` baselines are not modelled.
    pub(crate) fn from_component(
        value: &Component,
    ) -> Result<(Self, OverflowPosition), StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let (overflow, keywords) = match keywords.split_first() {
            Some((Keyword::Unsafe, rest)) => (OverflowPosition::Unsafe, rest),
            Some((Keyword::Safe, rest)) => (OverflowPosition::Safe, rest),
            _ => (OverflowPosition::Default, keywords.as_slice()),
        };
        Self::from_keywords(keywords)
            .map(|alignment| (alignment, overflow))
            .ok_or_else(|| unsupported(value))
    }
    /// §4.2: `anchor-center` behaves as `center` outside absolute positioning.
    pub(crate) fn in_flow(self) -> Self {
        if self == Self::AnchorCenter {
            Self::Center
        } else {
            self
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
            [Keyword::AnchorCenter] => Self::AnchorCenter,
            _ => return None,
        })
    }
}
/// CSS Box Alignment 3 §4.4 `<overflow-position>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum OverflowPosition {
    /// No keyword: the layout mode's automatic safety (§4.4.1), which honours the alignment
    /// outside scroll containers and absolute positioning.
    #[default]
    Default,
    Safe,
    Unsafe,
}
impl OverflowPosition {
    /// §4.4: a `safe` alignment subject that overflows its container (negative `free`
    /// space) aligns as `flex-start` instead.
    pub(crate) fn starts(self, free: f32) -> bool {
        self == Self::Safe && free < 0.0
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum FlexBasis {
    Content,
    /// `auto`, which defers to the main size property, or a `<'width'>` value.
    Size(Size),
}
impl Default for FlexBasis {
    fn default() -> Self {
        Self::Size(Size::Auto)
    }
}
impl FlexBasis {
    pub(crate) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::Content) {
            return Ok(Self::Content);
        }
        Size::from_component(value, relative).map(Self::Size)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct FlexStyle {
    pub direction: FlexDirection,
    pub wrap: FlexWrap,
    pub justify: Alignment,
    pub justify_overflow: OverflowPosition,
    pub align_items: Alignment,
    pub align_items_overflow: OverflowPosition,
    pub align_content: Alignment,
    pub align_content_overflow: OverflowPosition,
    pub align_self: Alignment,
    pub align_self_overflow: OverflowPosition,
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
            justify_overflow: OverflowPosition::Default,
            align_items: Alignment::Normal,
            align_items_overflow: OverflowPosition::Default,
            align_content: Alignment::Normal,
            align_content_overflow: OverflowPosition::Default,
            align_self: Alignment::Auto,
            align_self_overflow: OverflowPosition::Default,
            gap: [const { Gap::Normal }; 2],
            grow: 0.0,
            shrink: 1.0,
            basis: FlexBasis::Size(Size::Auto),
            order: 0,
            legacy_box: false,
        }
    }
}
impl FlexStyle {
    /// The item's `align-self`, with `auto` taken from the `container`'s `align-items`.
    pub(crate) fn used_align_self(&self, container: &Self) -> (Alignment, OverflowPosition) {
        match self.align_self.in_flow() {
            Alignment::Auto => (container.align_items, container.align_items_overflow),
            align => (align, self.align_self_overflow),
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
