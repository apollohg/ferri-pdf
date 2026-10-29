use crate::style::{
    StyleError, calc::Kind, component::Component, grammar::Keyword, units::Relative, values::math,
};
use serde::Serialize;

pub(crate) use crate::paint::LengthPercentage;

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

/// A `<length-percentage>` math function whose anchor functions resolve through `anchors`.
pub(super) fn anchored_length_percentage(
    value: &Component,
    relative: &Relative<'_>,
    anchors: crate::style::anchor_values::AnchorEvaluation<'_>,
) -> Result<LengthPercentage, StyleError> {
    match value.unwrap() {
        Component::Math(source, Kind::Length, range) => {
            Ok(crate::style::values::anchored_math(source, relative, anchors)?
                .length_percentage(*range))
        }
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
pub(crate) enum Visibility {
    #[default]
    Visible,
    Hidden,
    Collapse,
}

impl Visibility {
    pub(crate) fn is_visible(self) -> bool {
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
pub(crate) enum Size {
    #[default]
    Auto,
    Value(LengthPercentage),
    Intrinsic(crate::style::IntrinsicSizing),
}
impl Size {
    /// Percentages, including inside calculations, need a definite basis.
    pub(crate) fn resolve_definite(&self, containing_size: Option<f32>) -> Option<f32> {
        match self {
            Self::Auto | Self::Intrinsic(_) => None,
            Self::Value(LengthPercentage::Length(value)) => Some(*value),
            Self::Value(value) => containing_size.map(|basis| value.resolve(basis)),
        }
    }
    pub(crate) const ZERO: Self = Self::Value(LengthPercentage::ZERO);
    pub(crate) fn resolve(&self, containing_width: f32) -> Option<f32> {
        match self {
            Self::Auto | Self::Intrinsic(_) => None,
            Self::Value(value) => Some(value.resolve(containing_width)),
        }
    }
    pub(crate) fn is_zero(&self) -> bool {
        matches!(self, Self::Value(value) if value.is_zero())
    }
    /// CSS Sizing 3 §3.2: intrinsic keywords give a non-replaced box its automatic block size.
    pub(crate) fn is_auto_block_size(&self, replaced: bool) -> bool {
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
            Some(Keyword::MinContent) => {
                Ok(Self::Intrinsic(crate::style::IntrinsicSizing::MinContent))
            }
            Some(Keyword::MaxContent) => {
                Ok(Self::Intrinsic(crate::style::IntrinsicSizing::MaxContent))
            }
            Some(Keyword::FitContent) => {
                Ok(Self::Intrinsic(crate::style::IntrinsicSizing::FitContent))
            }
            _ => length_percentage(value, relative).map(Self::Value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum Display {
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
    /// `inline flow list-item`: an inline box with a marker.
    InlineListItem,
    /// `inline flow-root list-item`: an inline block with a marker.
    InlineFlowRootListItem,
    /// `ruby` or `inline ruby`: an inline-level ruby container (CSS Ruby 1 §2.1).
    Ruby,
    /// `block ruby`: a block box holding an inline-level ruby container (CSS Ruby 1 §2.1.2).
    BlockRuby,
    RubyBase,
    RubyText,
    RubyBaseContainer,
    RubyTextContainer,
    /// A run-in box (CSS Display 3 §5), laid out as the inline-level display it holds.
    RunIn(Box<Display>),
    Contents,
    None,
    Unsupported(String),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) enum BreakBetween {
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
    pub(crate) fn is_forced(self) -> bool {
        matches!(
            self,
            Self::Page | Self::Column | Self::Left | Self::Right | Self::Recto | Self::Verso
        )
    }
    pub(crate) fn page_side(self, rtl: bool) -> Option<crate::style::PageSide> {
        use crate::style::PageSide::{Left, Right};
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
    pub(crate) fn combine(self, later: Self) -> Self {
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
pub(crate) enum BreakInside {
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

/// CSS Display 3 §2.7 equivalences of the legacy keywords the engine models, and the
/// Compat §3.4.5 keyword mappings of `-webkit-box` and `-webkit-inline-box`.
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
        &[
            &[Keyword::Flex],
            &[Keyword::Block, Keyword::Flex],
            &[Keyword::DashWebkitBox],
        ],
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
        &[
            &[Keyword::InlineFlex],
            &[Keyword::Inline, Keyword::Flex],
            &[Keyword::DashWebkitInlineBox],
        ],
    ),
    (
        Display::InlineGrid,
        &[&[Keyword::InlineGrid], &[Keyword::Inline, Keyword::Grid]],
    ),
    (
        Display::InlineTable,
        &[&[Keyword::InlineTable], &[Keyword::Inline, Keyword::Table]],
    ),
    (
        Display::InlineListItem,
        &[
            &[Keyword::Inline, Keyword::ListItem],
            &[Keyword::Inline, Keyword::Flow, Keyword::ListItem],
            // The legacy keyword of earlier CSS Display 3 drafts.
            &[Keyword::InlineListItem],
        ],
    ),
    (
        Display::InlineFlowRootListItem,
        &[&[Keyword::Inline, Keyword::FlowRoot, Keyword::ListItem]],
    ),
    (
        Display::Ruby,
        &[&[Keyword::Ruby], &[Keyword::Inline, Keyword::Ruby]],
    ),
    (Display::BlockRuby, &[&[Keyword::Block, Keyword::Ruby]]),
    (Display::RubyBase, &[&[Keyword::RubyBase]]),
    (Display::RubyText, &[&[Keyword::RubyText]]),
    (Display::RubyBaseContainer, &[&[Keyword::RubyBaseContainer]]),
    (Display::RubyTextContainer, &[&[Keyword::RubyTextContainer]]),
    (Display::Contents, &[&[Keyword::Contents]]),
    (Display::None, &[&[Keyword::None]]),
];

impl Display {
    /// A block container at block level in its parent's block formatting context, whose first
    /// formatted line is its parent's when it is the first in-flow child (CSS Pseudo-Elements 4
    /// §2.1.1).
    pub(crate) fn is_block_level_block_container(&self) -> bool {
        matches!(
            self,
            Self::Block
                | Self::BlockRuby
                | Self::FlowRoot
                | Self::ListItem
                | Self::FlowRootListItem
        )
    }
    /// A block container, which has first formatted lines.
    pub(crate) fn is_block_container(&self) -> bool {
        self.is_block_level_block_container()
            || matches!(
                self,
                Self::InlineBlock
                    | Self::InlineFlowRootListItem
                    | Self::TableCell
                    | Self::TableCaption
            )
    }
    pub(crate) fn is_list_item(&self) -> bool {
        match self {
            Self::ListItem
            | Self::FlowRootListItem
            | Self::InlineListItem
            | Self::InlineFlowRootListItem => true,
            Self::RunIn(inner) => inner.is_list_item(),
            _ => false,
        }
    }
    /// The inline-level display of a run-in box, which lays out as that display wherever it
    /// does not run in.
    pub(crate) fn run_in(&self) -> Option<&Self> {
        match self {
            Self::RunIn(inner) => Some(inner),
            _ => None,
        }
    }
    /// A non-replaced inline box, whose contents join its inline formatting context: inner
    /// display type `flow` at inline or run-in level.
    pub(crate) fn is_inline_box(&self) -> bool {
        match self {
            Self::Inline | Self::InlineListItem => true,
            Self::RunIn(inner) => inner.is_inline_box(),
            _ => false,
        }
    }
    /// An inline-level block container, `inline flow-root` (CSS Display 3 §2.6).
    pub(crate) fn is_inline_block(&self) -> bool {
        match self {
            Self::InlineBlock | Self::InlineFlowRootListItem => true,
            Self::RunIn(inner) => inner.is_inline_block(),
            _ => false,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), StyleError> {
        match self {
            Self::Unsupported(value) => Err(StyleError::Unsupported(format!(
                "{}{}{value}",
                crate::style::grammar::Property::Display.name(),
                crate::style::properties::DECLARATION_SEPARATOR
            ))),
            _ => Ok(()),
        }
    }
    pub(crate) fn is_row_group(&self) -> bool {
        matches!(
            self,
            Self::TableRowGroup | Self::TableHeaderGroup | Self::TableFooterGroup
        )
    }
    pub(crate) fn is_table_child(&self) -> bool {
        self.is_row_group()
            || matches!(
                self,
                Self::TableRow | Self::TableColumn | Self::TableColumnGroup | Self::TableCaption
            )
    }
    pub(crate) fn is_table_internal(&self) -> bool {
        self.is_row_group()
            || matches!(
                self,
                Self::TableRow | Self::TableColumn | Self::TableColumnGroup | Self::TableCell
            )
    }
    /// A ruby base, annotation or level container, which only a ruby container holds
    /// (CSS Ruby 1 §2.1.1).
    pub(crate) fn is_ruby_internal(&self) -> bool {
        matches!(
            self,
            Self::RubyBase | Self::RubyText | Self::RubyBaseContainer | Self::RubyTextContainer
        )
    }
    /// A ruby container or internal ruby box, whose in-flow children are inlinified (CSS
    /// Ruby 1 §2.2).
    pub(crate) fn is_ruby(&self) -> bool {
        *self == Self::Ruby || self.is_ruby_internal()
    }
    pub(crate) fn is_inline_level(&self) -> bool {
        match self {
            Self::Inline | Self::InlineListItem | Self::Ruby | Self::RunIn(_) => true,
            atomic if atomic.is_atomic_inline() => true,
            Self::Unsupported(value) => value.to_ascii_lowercase().starts_with("inline-"),
            _ => false,
        }
    }
    /// Inline-level boxes laid out as one unbreakable unit in their line (CSS 2.1 §9.2.2).
    pub(crate) fn is_atomic_inline(&self) -> bool {
        match self {
            Self::InlineBlock
            | Self::InlineFlex
            | Self::InlineGrid
            | Self::InlineTable
            | Self::InlineFlowRootListItem => true,
            Self::RunIn(inner) => inner.is_atomic_inline(),
            _ => false,
        }
    }
    /// Establishes a flex formatting context, at block or inline level.
    pub(crate) fn is_flex(&self) -> bool {
        match self {
            Self::Flex | Self::InlineFlex => true,
            Self::RunIn(inner) => inner.is_flex(),
            _ => false,
        }
    }
    /// Establishes a grid formatting context, at block or inline level.
    pub(crate) fn is_grid(&self) -> bool {
        match self {
            Self::Grid | Self::InlineGrid => true,
            Self::RunIn(inner) => inner.is_grid(),
            _ => false,
        }
    }
    /// Flex and grid containers lay out their in-flow children as blockified items.
    pub(crate) fn is_item_container(&self) -> bool {
        self.is_flex() || self.is_grid()
    }
    /// Generates a table wrapper box, at block or inline level.
    pub(crate) fn is_table(&self) -> bool {
        match self {
            Self::Table | Self::InlineTable => true,
            Self::RunIn(inner) => inner.is_table(),
            _ => false,
        }
    }
    /// CSS Display 3 blockification for flex and grid items and absolutely positioned boxes.
    #[must_use]
    pub(crate) fn blockified(&self) -> Self {
        match self {
            // CSS Display 3 §2.7: inline and run-in flow-root boxes become block boxes.
            Self::Inline | Self::InlineBlock => Self::Block,
            Self::InlineListItem | Self::InlineFlowRootListItem => Self::ListItem,
            Self::InlineFlex => Self::Flex,
            Self::InlineGrid => Self::Grid,
            Self::InlineTable => Self::Table,
            Self::RunIn(inner) => inner.blockified(),
            // CSS Ruby 1 §2.1.2: a blockified ruby container keeps its inner display, and
            // internal ruby boxes become block boxes.
            Self::Ruby => Self::BlockRuby,
            internal
                if internal.is_table_internal()
                    || internal.is_ruby_internal()
                    || *internal == Self::TableCaption =>
            {
                Self::Block
            }
            other => other.clone(),
        }
    }
    /// CSS Overflow 4 §5.1.1 distinguishes the `-webkit-box` forms of flex containers.
    pub(super) fn is_legacy_box(value: &Component) -> bool {
        matches!(
            value.keyword(),
            Some(Keyword::DashWebkitBox | Keyword::DashWebkitInlineBox)
        )
    }
    /// CSS Display 3 §2.7: the outer display type becomes inline; a block container becomes
    /// an inline block, and layout-internal boxes are unaffected.
    #[must_use]
    pub(crate) fn inlinified(&self) -> Self {
        match self {
            Self::Block | Self::FlowRoot => Self::InlineBlock,
            Self::ListItem | Self::FlowRootListItem => Self::InlineFlowRootListItem,
            Self::Flex => Self::InlineFlex,
            Self::Grid => Self::InlineGrid,
            Self::Table => Self::InlineTable,
            Self::BlockRuby => Self::Ruby,
            Self::RunIn(inner) => (**inner).clone(),
            other => other.clone(),
        }
    }
    /// `||` and `&&` slots follow the grammar, so keywords arrive as outside, inside, marker.
    pub(super) fn from_component(value: &Component) -> Self {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        if keywords.first() == Some(&Keyword::RunIn) {
            // A run-in box is the inline-level box of the same inner display (§2.1).
            keywords[0] = Keyword::Inline;
            let inline = Self::from_keywords(&keywords);
            return if inline.is_inline_level() && !matches!(inline, Self::Unsupported(_)) {
                Self::RunIn(Box::new(inline))
            } else {
                keywords[0] = Keyword::RunIn;
                Self::unsupported(&keywords)
            };
        }
        Self::from_keywords(&keywords)
    }
    fn unsupported(keywords: &[Keyword]) -> Self {
        Self::Unsupported(
            keywords
                .iter()
                .map(|keyword| keyword.name())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
    fn from_keywords(keywords: &[Keyword]) -> Self {
        DISPLAY_FORMS
            .iter()
            .find(|(_, forms)| forms.contains(&keywords))
            .map_or_else(
                || Self::unsupported(keywords),
                |(display, _)| display.clone(),
            )
    }
}
