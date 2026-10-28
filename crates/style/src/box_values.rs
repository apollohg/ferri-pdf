use crate::{
    StyleError, calc::Kind, component::Component, grammar::Keyword, units::Relative, values::math,
};
use serde::Serialize;

pub use ferri_pdf_paint::LengthPercentage;

/// Specified `<length-percentage>` to points, a fraction, or a calculation over both.
pub(super) fn length_percentage(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<LengthPercentage, StyleError> {
    match value.unwrap() {
        Component::Length(length) => Ok(LengthPercentage::Length(length.points(relative)?)),
        Component::Percentage(fraction) if fraction.is_finite() => {
            Ok(LengthPercentage::Percentage(*fraction))
        }
        Component::Percentage(_) => Err("non-finite percentage".into()),
        Component::Math(source, Kind::Length, range) => {
            Ok(math(source, relative, Kind::Length, Some(Kind::Length))?.length_percentage(*range))
        }
        Component::Math(source, Kind::Percentage, range) => Ok(LengthPercentage::Percentage(
            math(source, relative, Kind::Percentage, Some(Kind::Percentage))?.resolve(0.0, *range),
        )),
        other => Err(unsupported(other)),
    }
}

/// Font percentages need a font-size basis; an already absolute length does not.
pub(super) fn font_length_percentage(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<f32, StyleError> {
    match length_percentage(value, relative)? {
        LengthPercentage::Length(value) => Ok(value),
        other => Ok(other.resolve(relative.font.size()?)),
    }
}

/// A matched value the engine does not compute.
pub(super) fn unsupported(value: &Component) -> StyleError {
    StyleError::Unsupported(value.to_string())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
    Collapse,
}

impl Visibility {
    pub fn is_visible(self) -> bool {
        self == Self::Visible
    }

    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Visible) => Ok(Self::Visible),
            Some(Keyword::Hidden) => Ok(Self::Hidden),
            Some(Keyword::Collapse) => Ok(Self::Collapse),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub enum Size {
    #[default]
    Auto,
    Value(LengthPercentage),
    Intrinsic(crate::IntrinsicSizing),
}
impl Size {
    /// Percentages, including inside calculations, need a definite basis.
    pub fn resolve_definite(&self, containing_size: Option<f32>) -> Option<f32> {
        match self {
            Self::Auto | Self::Intrinsic(_) => None,
            Self::Value(LengthPercentage::Length(value)) => Some(*value),
            Self::Value(value) => containing_size.map(|basis| value.resolve(basis)),
        }
    }
    pub const ZERO: Self = Self::Value(LengthPercentage::ZERO);
    pub fn resolve(&self, containing_width: f32) -> Option<f32> {
        match self {
            Self::Auto | Self::Intrinsic(_) => None,
            Self::Value(value) => Some(value.resolve(containing_width)),
        }
    }
    pub fn is_zero(&self) -> bool {
        matches!(self, Self::Value(value) if value.is_zero())
    }
    /// CSS Sizing 3 §3.2: intrinsic keywords give a non-replaced box its automatic block size.
    pub fn is_auto_block_size(&self, replaced: bool) -> bool {
        match self {
            Self::Auto => true,
            Self::Intrinsic(_) => !replaced,
            Self::Value(_) => false,
        }
    }
    /// `auto`, and `none` for maximum sizes, are unconstrained.
    pub(super) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto | Keyword::None) => Ok(Self::Auto),
            Some(Keyword::MinContent) => Ok(Self::Intrinsic(crate::IntrinsicSizing::MinContent)),
            Some(Keyword::MaxContent) => Ok(Self::Intrinsic(crate::IntrinsicSizing::MaxContent)),
            Some(Keyword::FitContent) => Ok(Self::Intrinsic(crate::IntrinsicSizing::FitContent)),
            _ => length_percentage(value, relative).map(Self::Value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub enum Display {
    #[default]
    Inline,
    Block,
    FlowRoot,
    Flex,
    Grid,
    ListItem,
    FlowRootListItem,
    Table,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableRow,
    TableCell,
    TableColumn,
    TableColumnGroup,
    TableCaption,
    InlineBlock,
    InlineFlex,
    InlineGrid,
    InlineTable,
    Contents,
    None,
    Unsupported(String),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum BreakBetween {
    #[default]
    Auto,
    Avoid,
    AvoidColumn,
    Page,
    Column,
    Left,
    Right,
    Recto,
    Verso,
}
impl BreakBetween {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Avoid) => Ok(Self::Avoid),
            Some(Keyword::AvoidColumn) => Ok(Self::AvoidColumn),
            Some(Keyword::Page | Keyword::Always) => Ok(Self::Page),
            Some(Keyword::Column) => Ok(Self::Column),
            Some(Keyword::Left) => Ok(Self::Left),
            Some(Keyword::Right) => Ok(Self::Right),
            Some(Keyword::Recto) => Ok(Self::Recto),
            Some(Keyword::Verso) => Ok(Self::Verso),
            _ => Err(unsupported(value)),
        }
    }
    pub fn is_forced(self) -> bool {
        matches!(
            self,
            Self::Page | Self::Column | Self::Left | Self::Right | Self::Recto | Self::Verso
        )
    }
    pub fn page_side(self, rtl: bool) -> Option<crate::PageSide> {
        use crate::PageSide::{Left, Right};
        match self {
            Self::Left => Some(Left),
            Self::Right => Some(Right),
            Self::Recto => Some(if rtl { Left } else { Right }),
            Self::Verso => Some(if rtl { Right } else { Left }),
            _ => None,
        }
    }
    /// Combines coincident breaks in flow order, retaining the latest side constraint.
    #[must_use]
    pub fn combine(self, later: Self) -> Self {
        if later.page_side(false).is_some() {
            later
        } else if self.page_side(false).is_some() {
            self
        } else {
            [Self::Page, Self::Column, Self::Avoid, Self::AvoidColumn]
                .into_iter()
                .find(|value| *value == self || *value == later)
                .unwrap_or(Self::Auto)
        }
    }
}

/// `break-inside`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub enum BreakInside {
    #[default]
    Auto,
    Avoid,
    AvoidColumn,
}
impl BreakInside {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Avoid) => Ok(Self::Avoid),
            Some(Keyword::AvoidColumn) => Ok(Self::AvoidColumn),
            _ => Err(unsupported(value)),
        }
    }
}

/// CSS Display 3 §2.7 equivalences of the legacy keywords the engine models.
const DISPLAY_FORMS: &[(Display, &[&[Keyword]])] = &[
    (
        Display::Inline,
        &[&[Keyword::Inline], &[Keyword::Inline, Keyword::Flow]],
    ),
    (
        Display::Block,
        &[
            &[Keyword::Block],
            &[Keyword::Flow],
            &[Keyword::Block, Keyword::Flow],
        ],
    ),
    (
        Display::FlowRoot,
        &[&[Keyword::FlowRoot], &[Keyword::Block, Keyword::FlowRoot]],
    ),
    (
        Display::Flex,
        &[&[Keyword::Flex], &[Keyword::Block, Keyword::Flex]],
    ),
    (
        Display::Grid,
        &[&[Keyword::Grid], &[Keyword::Block, Keyword::Grid]],
    ),
    (
        Display::Table,
        &[&[Keyword::Table], &[Keyword::Block, Keyword::Table]],
    ),
    (
        Display::ListItem,
        &[
            &[Keyword::ListItem],
            &[Keyword::Block, Keyword::ListItem],
            &[Keyword::Flow, Keyword::ListItem],
            &[Keyword::Block, Keyword::Flow, Keyword::ListItem],
        ],
    ),
    (
        Display::FlowRootListItem,
        &[
            &[Keyword::FlowRoot, Keyword::ListItem],
            &[Keyword::Block, Keyword::FlowRoot, Keyword::ListItem],
        ],
    ),
    (Display::TableRowGroup, &[&[Keyword::TableRowGroup]]),
    (Display::TableHeaderGroup, &[&[Keyword::TableHeaderGroup]]),
    (Display::TableFooterGroup, &[&[Keyword::TableFooterGroup]]),
    (Display::TableRow, &[&[Keyword::TableRow]]),
    (Display::TableCell, &[&[Keyword::TableCell]]),
    (Display::TableColumn, &[&[Keyword::TableColumn]]),
    (Display::TableColumnGroup, &[&[Keyword::TableColumnGroup]]),
    (Display::TableCaption, &[&[Keyword::TableCaption]]),
    (
        Display::InlineBlock,
        &[
            &[Keyword::InlineBlock],
            &[Keyword::Inline, Keyword::FlowRoot],
        ],
    ),
    (
        Display::InlineFlex,
        &[&[Keyword::InlineFlex], &[Keyword::Inline, Keyword::Flex]],
    ),
    (
        Display::InlineGrid,
        &[&[Keyword::InlineGrid], &[Keyword::Inline, Keyword::Grid]],
    ),
    (
        Display::InlineTable,
        &[&[Keyword::InlineTable], &[Keyword::Inline, Keyword::Table]],
    ),
    (Display::Contents, &[&[Keyword::Contents]]),
    (Display::None, &[&[Keyword::None]]),
];

impl Display {
    pub fn is_list_item(&self) -> bool {
        matches!(self, Self::ListItem | Self::FlowRootListItem)
    }

    pub fn validate(&self) -> Result<(), StyleError> {
        match self {
            Self::Unsupported(value) => Err(StyleError::Unsupported(format!("display:{value}"))),
            _ => Ok(()),
        }
    }
    pub fn is_row_group(&self) -> bool {
        matches!(
            self,
            Self::TableRowGroup | Self::TableHeaderGroup | Self::TableFooterGroup
        )
    }
    pub fn is_table_child(&self) -> bool {
        self.is_row_group()
            || matches!(
                self,
                Self::TableRow | Self::TableColumn | Self::TableColumnGroup | Self::TableCaption
            )
    }
    pub fn is_table_internal(&self) -> bool {
        self.is_row_group()
            || matches!(
                self,
                Self::TableRow | Self::TableColumn | Self::TableColumnGroup | Self::TableCell
            )
    }
    pub fn is_inline_level(&self) -> bool {
        match self {
            Self::Inline => true,
            atomic if atomic.is_atomic_inline() => true,
            Self::Unsupported(value) => value.to_ascii_lowercase().starts_with("inline-"),
            _ => false,
        }
    }
    /// Inline-level boxes laid out as one unbreakable unit in their line (CSS 2.1 §9.2.2).
    pub fn is_atomic_inline(&self) -> bool {
        matches!(
            self,
            Self::InlineBlock | Self::InlineFlex | Self::InlineGrid | Self::InlineTable
        )
    }
    /// Establishes a flex formatting context, at block or inline level.
    pub fn is_flex(&self) -> bool {
        matches!(self, Self::Flex | Self::InlineFlex)
    }
    /// Establishes a grid formatting context, at block or inline level.
    pub fn is_grid(&self) -> bool {
        matches!(self, Self::Grid | Self::InlineGrid)
    }
    /// Flex and grid containers lay out their in-flow children as blockified items.
    pub fn is_item_container(&self) -> bool {
        self.is_flex() || self.is_grid()
    }
    /// Generates a table wrapper box, at block or inline level.
    pub fn is_table(&self) -> bool {
        matches!(self, Self::Table | Self::InlineTable)
    }
    /// CSS Display 3 blockification for flex and grid items and absolutely positioned boxes.
    #[must_use]
    pub fn blockified(&self) -> Self {
        match self {
            Self::Inline | Self::InlineBlock => Self::Block,
            Self::InlineFlex => Self::Flex,
            Self::InlineGrid => Self::Grid,
            Self::InlineTable => Self::Table,
            internal if internal.is_table_internal() || *internal == Self::TableCaption => {
                Self::Block
            }
            other => other.clone(),
        }
    }
    /// `||` and `&&` slots follow the grammar, so keywords arrive as outside, inside, marker.
    pub(super) fn from_component(value: &Component) -> Self {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        DISPLAY_FORMS
            .iter()
            .find(|(_, forms)| forms.contains(&keywords.as_slice()))
            .map_or_else(
                || {
                    Self::Unsupported(
                        keywords
                            .iter()
                            .map(|keyword| keyword.name())
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                },
                |(display, _)| display.clone(),
            )
    }
}
