//! CSS Logical 1 §4: each flow-relative box property pairs with the physical property its
//! element's own writing mode and direction map it to, and the pair shares one computed value.
use crate::style::{Style, grammar::Property};
use std::sync::Arc;

pub(crate) const TOP: usize = 0;
pub(crate) const RIGHT: usize = 1;
pub(crate) const BOTTOM: usize = 2;
pub(crate) const LEFT: usize = 3;
/// Physical corners in `border-radius` order, as the two sides meeting there.
const CORNER_SIDES: [[usize; 2]; 4] = [[TOP, LEFT], [TOP, RIGHT], [BOTTOM, RIGHT], [BOTTOM, LEFT]];
/// Physical dimensions: `width`/`overflow-x`, then `height`/`overflow-y`.
const HORIZONTAL: usize = 0;
const VERTICAL: usize = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum WritingMode {
    #[default]
    HorizontalTb,
    VerticalRl,
    VerticalLr,
    SidewaysRl,
    SidewaysLr,
}
impl WritingMode {
    pub(crate) fn from_component(
        value: &crate::style::component::Component,
    ) -> Result<Self, crate::style::StyleError> {
        use crate::style::grammar::Keyword;
        match value.keyword() {
            Some(
                Keyword::HorizontalTb | Keyword::Lr | Keyword::LrTb | Keyword::Rl | Keyword::RlTb,
            ) => Ok(Self::HorizontalTb),
            Some(Keyword::VerticalRl | Keyword::Tb | Keyword::TbRl) => Ok(Self::VerticalRl),
            Some(Keyword::VerticalLr) => Ok(Self::VerticalLr),
            Some(Keyword::SidewaysRl) => Ok(Self::SidewaysRl),
            Some(Keyword::SidewaysLr) => Ok(Self::SidewaysLr),
            _ => Err(crate::style::box_values::unsupported(value)),
        }
    }
    pub(crate) fn vertical(self) -> bool {
        self != Self::HorizontalTb
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::HorizontalTb => "horizontal-tb",
            Self::VerticalRl => "vertical-rl",
            Self::VerticalLr => "vertical-lr",
            Self::SidewaysRl => "sideways-rl",
            Self::SidewaysLr => "sideways-lr",
        }
    }
    pub(crate) fn page_progression_rtl(self, rtl: bool) -> bool {
        match self {
            Self::HorizontalTb => rtl,
            Self::VerticalRl | Self::SidewaysRl => true,
            Self::VerticalLr | Self::SidewaysLr => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum TextOrientation {
    #[default]
    Mixed,
    Upright,
    Sideways,
}
impl TextOrientation {
    pub(crate) fn ranges(
        self,
        text: &str,
        range: std::ops::Range<usize>,
        vertical_forms: bool,
    ) -> Vec<(std::ops::Range<usize>, bool)> {
        use unicode_segmentation::UnicodeSegmentation;
        let mut pieces: Vec<(std::ops::Range<usize>, bool)> = Vec::new();
        for (offset, cluster) in text[range.clone()].grapheme_indices(true) {
            let first = cluster.chars().next().expect("a grapheme is nonempty");
            let upright = match self {
                Self::Upright => true,
                Self::Sideways => false,
                Self::Mixed => match unicode_vo::char_orientation(first) {
                    unicode_vo::Orientation::Upright
                    | unicode_vo::Orientation::TransformedOrUpright => true,
                    unicode_vo::Orientation::TransformedOrRotated => vertical_forms,
                    unicode_vo::Orientation::Rotated => false,
                },
            };
            let start = range.start + offset;
            if let Some((previous, orientation)) = pieces.last_mut()
                && *orientation == upright
            {
                previous.end = start + cluster.len();
            } else {
                pieces.push((start..start + cluster.len(), upright));
            }
        }
        pieces
    }
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        [Self::Mixed, Self::Upright, Self::Sideways]
            .into_iter()
            .find(|value| name.eq_ignore_ascii_case(value.name()))
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Mixed => "mixed",
            Self::Upright => "upright",
            Self::Sideways => "sideways",
        }
    }
    pub(crate) fn from_component(
        value: &crate::style::component::Component,
    ) -> Result<Self, crate::style::StyleError> {
        value
            .keyword()
            .and_then(|keyword| Self::from_name(keyword.name()))
            .ok_or_else(|| crate::style::box_values::unsupported(value))
    }
}

/// Writing Modes 4 §5.1.3: only these legacy values alias `text-orientation`.
pub(super) fn glyph_orientation(
    value: &crate::style::component::Component,
) -> Option<crate::style::grammar::Keyword> {
    use crate::style::{component::Component, grammar::Keyword};
    const QUARTER_TURN: i32 = 90;
    match value.unwrap() {
        Component::Keyword(Keyword::Auto) => Some(Keyword::Mixed),
        Component::Integer(0) => Some(Keyword::Upright),
        Component::Integer(QUARTER_TURN) => Some(Keyword::Sideways),
        Component::Dimension(angle, unit) if unit.eq_ignore_ascii_case("deg") => {
            if *angle == 0.0 {
                Some(Keyword::Upright)
            } else if *angle == QUARTER_TURN as f32 {
                Some(Keyword::Sideways)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(super) fn valid(property: Property, value: &crate::style::component::Component) -> bool {
    property != Property::GlyphOrientationVertical || glyph_orientation(value).is_some()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) enum TextCombine {
    #[default]
    None,
    All,
    Digits(u8),
}
impl TextCombine {
    pub(super) fn from_component(
        value: &crate::style::component::Component,
        relative: &crate::style::units::Relative<'_>,
    ) -> Result<Self, crate::style::StyleError> {
        use crate::style::grammar::Keyword;
        if value.keyword() == Some(Keyword::None) {
            return Ok(Self::None);
        }
        if value.keyword() == Some(Keyword::All) {
            return Ok(Self::All);
        }
        let mut leaves = Vec::new();
        let mut pending = vec![value];
        while let Some(value) = pending.pop() {
            match value.unwrap() {
                crate::style::component::Component::Group(items) => {
                    pending.extend(items.iter().rev());
                }
                other => leaves.push(other),
            }
        }
        let mut digits = false;
        let mut count = 2;
        for leaf in leaves {
            if leaf.keyword() == Some(Keyword::Digits) {
                digits = true;
            } else {
                count = crate::style::values::integer(leaf, relative)?;
            }
        }
        if digits && (2..=4).contains(&count) {
            Ok(Self::Digits(count as u8))
        } else {
            Err(crate::style::box_values::unsupported(value))
        }
    }
    pub(crate) fn accepts(self, ch: char) -> bool {
        match self {
            Self::None => false,
            Self::All => ch != crate::document::OBJECT_REPLACEMENT_CHARACTER,
            Self::Digits(_) => ch.is_ascii_digit(),
        }
    }
}

/// How flow-relative terms map to physical ones for one box (CSS Writing Modes 4 §6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Flow {
    pub(crate) writing_mode: WritingMode,
    pub(crate) rtl: bool,
}

/// An element's flow and its parent's, whose corresponding properties it inherits.
#[derive(Clone, Copy)]
pub(crate) struct Flows {
    pub(crate) element: Flow,
    pub(crate) parent: Flow,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Block,
    Inline,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    Start,
    End,
}

#[derive(Clone, Copy)]
enum Position {
    Side(Axis, Edge),
    /// The block-axis edge, then the inline-axis edge (CSS Logical 1 §4.6).
    Corner(Edge, Edge),
    Dimension(Axis),
}

const SIDES: [Position; 4] = [
    Position::Side(Axis::Block, Edge::Start),
    Position::Side(Axis::Block, Edge::End),
    Position::Side(Axis::Inline, Edge::Start),
    Position::Side(Axis::Inline, Edge::End),
];
const CORNERS: [Position; 4] = [
    Position::Corner(Edge::Start, Edge::Start),
    Position::Corner(Edge::Start, Edge::End),
    Position::Corner(Edge::End, Edge::Start),
    Position::Corner(Edge::End, Edge::End),
];
const DIMENSIONS: [Position; 2] = [
    Position::Dimension(Axis::Block),
    Position::Dimension(Axis::Inline),
];

/// A logical property group: physical members by physical index, and flow-relative members
/// at the positions of the matching `positions` list.
struct Group {
    physical: &'static [Property],
    flow_relative: &'static [Property],
    positions: &'static [Position],
}

const GROUPS: [Group; 16] = {
    use Property as P;
    [
        Group {
            physical: &[P::MarginTop, P::MarginRight, P::MarginBottom, P::MarginLeft],
            flow_relative: &[
                P::MarginBlockStart,
                P::MarginBlockEnd,
                P::MarginInlineStart,
                P::MarginInlineEnd,
            ],
            positions: &SIDES,
        },
        Group {
            physical: &[
                P::PaddingTop,
                P::PaddingRight,
                P::PaddingBottom,
                P::PaddingLeft,
            ],
            flow_relative: &[
                P::PaddingBlockStart,
                P::PaddingBlockEnd,
                P::PaddingInlineStart,
                P::PaddingInlineEnd,
            ],
            positions: &SIDES,
        },
        Group {
            physical: &[P::Top, P::Right, P::Bottom, P::Left],
            flow_relative: &[
                P::InsetBlockStart,
                P::InsetBlockEnd,
                P::InsetInlineStart,
                P::InsetInlineEnd,
            ],
            positions: &SIDES,
        },
        Group {
            physical: &[
                P::BorderTopWidth,
                P::BorderRightWidth,
                P::BorderBottomWidth,
                P::BorderLeftWidth,
            ],
            flow_relative: &[
                P::BorderBlockStartWidth,
                P::BorderBlockEndWidth,
                P::BorderInlineStartWidth,
                P::BorderInlineEndWidth,
            ],
            positions: &SIDES,
        },
        Group {
            physical: &[
                P::BorderTopStyle,
                P::BorderRightStyle,
                P::BorderBottomStyle,
                P::BorderLeftStyle,
            ],
            flow_relative: &[
                P::BorderBlockStartStyle,
                P::BorderBlockEndStyle,
                P::BorderInlineStartStyle,
                P::BorderInlineEndStyle,
            ],
            positions: &SIDES,
        },
        Group {
            physical: &[
                P::BorderTopColor,
                P::BorderRightColor,
                P::BorderBottomColor,
                P::BorderLeftColor,
            ],
            flow_relative: &[
                P::BorderBlockStartColor,
                P::BorderBlockEndColor,
                P::BorderInlineStartColor,
                P::BorderInlineEndColor,
            ],
            positions: &SIDES,
        },
        Group {
            physical: &[
                P::BorderTopLeftRadius,
                P::BorderTopRightRadius,
                P::BorderBottomRightRadius,
                P::BorderBottomLeftRadius,
            ],
            flow_relative: &[
                P::BorderStartStartRadius,
                P::BorderStartEndRadius,
                P::BorderEndStartRadius,
                P::BorderEndEndRadius,
            ],
            positions: &CORNERS,
        },
        Group {
            physical: &[P::Width, P::Height],
            flow_relative: &[P::BlockSize, P::InlineSize],
            positions: &DIMENSIONS,
        },
        Group {
            physical: &[P::MinWidth, P::MinHeight],
            flow_relative: &[P::MinBlockSize, P::MinInlineSize],
            positions: &DIMENSIONS,
        },
        Group {
            physical: &[P::MaxWidth, P::MaxHeight],
            flow_relative: &[P::MaxBlockSize, P::MaxInlineSize],
            positions: &DIMENSIONS,
        },
        Group {
            physical: &[P::OverflowX, P::OverflowY],
            flow_relative: &[P::OverflowBlock, P::OverflowInline],
            positions: &DIMENSIONS,
        },
        Group {
            physical: &[P::ContainIntrinsicWidth, P::ContainIntrinsicHeight],
            flow_relative: &[P::ContainIntrinsicBlockSize, P::ContainIntrinsicInlineSize],
            positions: &DIMENSIONS,
        },
        Group {
            physical: &[
                P::ScrollMarginTop,
                P::ScrollMarginRight,
                P::ScrollMarginBottom,
                P::ScrollMarginLeft,
            ],
            flow_relative: &[
                P::ScrollMarginBlockStart,
                P::ScrollMarginBlockEnd,
                P::ScrollMarginInlineStart,
                P::ScrollMarginInlineEnd,
            ],
            positions: &SIDES,
        },
        Group {
            physical: &[
                P::ScrollPaddingTop,
                P::ScrollPaddingRight,
                P::ScrollPaddingBottom,
                P::ScrollPaddingLeft,
            ],
            flow_relative: &[
                P::ScrollPaddingBlockStart,
                P::ScrollPaddingBlockEnd,
                P::ScrollPaddingInlineStart,
                P::ScrollPaddingInlineEnd,
            ],
            positions: &SIDES,
        },
        Group {
            physical: &[P::OverscrollBehaviorX, P::OverscrollBehaviorY],
            flow_relative: &[P::OverscrollBehaviorBlock, P::OverscrollBehaviorInline],
            positions: &DIMENSIONS,
        },
        Group {
            physical: &[
                P::OverflowClipMarginTop,
                P::OverflowClipMarginRight,
                P::OverflowClipMarginBottom,
                P::OverflowClipMarginLeft,
            ],
            flow_relative: &[
                P::OverflowClipMarginBlockStart,
                P::OverflowClipMarginBlockEnd,
                P::OverflowClipMarginInlineStart,
                P::OverflowClipMarginInlineEnd,
            ],
            positions: &SIDES,
        },
    ]
};
/// The groups of properties without print effect, whose values live in `Style::inert`.
const INERT_GROUPS: std::ops::Range<usize> = 12..15;

impl Flow {
    pub(crate) fn of(style: &Style) -> Self {
        Self {
            writing_mode: style.writing_mode,
            rtl: style.rtl,
        }
    }

    pub(crate) fn used(style: &Style) -> Self {
        Self {
            writing_mode: style.used_writing_mode(),
            rtl: style.used_rtl(),
        }
    }

    pub(crate) fn horizontal(self) -> Self {
        Self {
            writing_mode: WritingMode::HorizontalTb,
            ..self
        }
    }

    pub(crate) fn block_axis(self) -> usize {
        self.dimension(Axis::Block)
    }

    pub(crate) fn axis_start(self, axis: usize) -> usize {
        self.side(
            if self.block_axis() == axis {
                Axis::Block
            } else {
                Axis::Inline
            },
            Edge::Start,
        )
    }

    pub(crate) fn sides<T: Clone>(self, from: Self, values: &[T; 4]) -> [T; 4] {
        std::array::from_fn(|index| values[self.source(from, &SIDES, index)].clone())
    }

    pub(crate) fn corners<T: Clone>(self, from: Self, values: &[T; 4]) -> [T; 4] {
        std::array::from_fn(|index| values[self.source(from, &CORNERS, index)].clone())
    }

    fn side(self, axis: Axis, edge: Edge) -> usize {
        let ends = match (self.writing_mode, axis) {
            (WritingMode::HorizontalTb, Axis::Block) => [TOP, BOTTOM],
            (WritingMode::HorizontalTb, Axis::Inline) => [LEFT, RIGHT],
            (WritingMode::VerticalRl | WritingMode::SidewaysRl, Axis::Block) => [RIGHT, LEFT],
            (WritingMode::VerticalLr | WritingMode::SidewaysLr, Axis::Block) => [LEFT, RIGHT],
            (WritingMode::SidewaysLr, Axis::Inline) => [BOTTOM, TOP],
            (_, Axis::Inline) => [TOP, BOTTOM],
        };
        let end = (edge == Edge::End) ^ (axis == Axis::Inline && self.rtl);
        ends[usize::from(end)]
    }

    fn dimension(self, axis: Axis) -> usize {
        if (axis == Axis::Block) == self.writing_mode.vertical() {
            HORIZONTAL
        } else {
            VERTICAL
        }
    }

    /// The physical index of `position` within its group.
    fn physical(self, position: Position) -> usize {
        match position {
            Position::Side(axis, edge) => self.side(axis, edge),
            Position::Corner(block, inline) => {
                let sides = [
                    self.side(Axis::Block, block),
                    self.side(Axis::Inline, inline),
                ];
                CORNER_SIDES
                    .iter()
                    .position(|corner| sides.iter().all(|side| corner.contains(side)))
                    .expect("a block-axis and an inline-axis side meet at a corner")
            }
            Position::Dimension(axis) => self.dimension(axis),
        }
    }

    /// The index in `from`'s flow holding what physical index `index` holds in this flow.
    fn source(self, from: Self, positions: &[Position], index: usize) -> usize {
        let position = positions
            .iter()
            .find(|position| self.physical(**position) == index)
            .expect("every physical index has a flow-relative position");
        from.physical(*position)
    }
}

fn group(property: Property) -> Option<&'static Group> {
    GROUPS
        .iter()
        .find(|group| group.physical.contains(&property) || group.flow_relative.contains(&property))
}

/// The physical properties a flow-relative property can share its computed value with.
pub(crate) fn physical_properties(property: Property) -> Option<&'static [Property]> {
    group(property)
        .filter(|group| group.flow_relative.contains(&property))
        .map(|group| group.physical)
}

pub(crate) fn is_flow_relative(property: Property) -> bool {
    physical_properties(property).is_some()
}

/// The flow-relative members of a physical property's group.
pub(crate) fn flow_relative_properties(property: Property) -> &'static [Property] {
    group(property)
        .filter(|group| group.physical.contains(&property))
        .map_or(&[], |group| group.flow_relative)
}

/// The physical property `property` maps to in `flow`; physical properties map to themselves.
pub(crate) fn physical(property: Property, flow: Flow) -> Property {
    let Some(group) = group(property) else {
        return property;
    };
    group
        .flow_relative
        .iter()
        .position(|candidate| *candidate == property)
        .map_or(property, |index| {
            group.physical[flow.physical(group.positions[index])]
        })
}

/// The flow-relative property sharing a physical property's computed value in `flow`.
pub(crate) fn counterpart(property: Property, flow: Flow) -> Option<Property> {
    let group = group(property)?;
    let index = group
        .physical
        .iter()
        .position(|candidate| *candidate == property)?;
    group
        .positions
        .iter()
        .position(|position| flow.physical(*position) == index)
        .map(|position| group.flow_relative[position])
}

/// The physical property holding, in flow `to`, what `property` holds in flow `from`.
pub(crate) fn corresponding(property: Property, from: Flow, to: Flow) -> Property {
    counterpart(property, from).map_or(property, |logical| physical(logical, to))
}

pub(crate) fn decoration<I: Clone>(
    source: &crate::paint::BoxDecoration<I>,
    from: Flow,
    to: Flow,
) -> crate::paint::BoxDecoration<I> {
    let mut result = source.clone();
    result.borders = to.sides(from, &source.borders);
    let mut radii = to.corners(from, &source.radii);
    if from.writing_mode.vertical() != to.writing_mode.vertical() {
        for corner in &mut radii {
            corner.swap(0, 1);
        }
    }
    result.radii = Arc::new(radii);
    result
}

/// `source` with its box properties arranged for `flow`: each physical property holds the
/// value of the property corresponding to it in `source`'s own flow.
pub(crate) fn reflowed(source: &Style, flow: Flow) -> Style {
    reflowed_from(source, Flow::of(source), flow)
}

fn reflowed_from(source: &Style, from: Flow, flow: Flow) -> Style {
    let side = |index| flow.source(from, &SIDES, index);
    let dimension = |index| flow.source(from, &DIMENSIONS, index);
    let dimensions = |values: [&crate::style::Size; 2]| -> [crate::style::Size; 2] {
        std::array::from_fn(|index| values[dimension(index)].clone())
    };
    let mut style = source.clone();
    style.layout_writing_mode = from.writing_mode;
    style.anchor.reflowed_geometry(from, flow);
    style.anchor.pages = source
        .anchor
        .pages
        .iter()
        .map(|(page, style)| (*page, reflowed_from(style, from, flow)))
        .collect();
    style.anchor.options = source
        .anchor
        .options
        .iter()
        .map(|style| reflowed_from(style, from, flow))
        .collect();
    style.margin = std::array::from_fn(|index| source.margin[side(index)].clone());
    style.padding = std::array::from_fn(|index| source.padding[side(index)].clone());
    style.inset = std::array::from_fn(|index| source.inset[side(index)].clone());
    style.decoration = decoration(&source.decoration, from, flow);
    [style.width, style.height] = dimensions([&source.width, &source.height]);
    [style.min_width, style.min_height] = dimensions([&source.min_width, &source.min_height]);
    [style.max_width, style.max_height] = dimensions([&source.max_width, &source.max_height]);
    style.overflow = std::array::from_fn(|index| source.overflow[dimension(index)]);
    style.overflow_clip_margin =
        std::array::from_fn(|index| source.overflow_clip_margin[side(index)]);
    style.object_position = std::array::from_fn(|axis| {
        let edge = if axis == HORIZONTAL { LEFT } else { TOP };
        let source_edge = flow.source(from, &SIDES, edge);
        let source_axis = if matches!(source_edge, LEFT | RIGHT) {
            HORIZONTAL
        } else {
            VERTICAL
        };
        let mut position = source.object_position[source_axis].clone();
        position.from_end ^= matches!(source_edge, RIGHT | BOTTOM);
        position
    });
    if from.writing_mode.vertical() != flow.writing_mode.vertical() {
        style.aspect_ratio.ratio = source.aspect_ratio.ratio.map(f32::recip);
    }
    for group in &GROUPS[INERT_GROUPS] {
        for (index, property) in group.physical.iter().enumerate() {
            let from = group.physical[flow.source(from, group.positions, index)];
            crate::style::inert::copy_from(*property, &mut style, from, source);
        }
    }
    style
}
