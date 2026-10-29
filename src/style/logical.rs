//! CSS Logical 1 §4: each flow-relative box property pairs with the physical property its
//! element's own writing mode and direction map it to, and the pair shares one computed value.
use crate::style::{Style, grammar::Property};
use std::sync::Arc;

pub(super) const TOP: usize = 0;
pub(super) const RIGHT: usize = 1;
pub(super) const BOTTOM: usize = 2;
pub(super) const LEFT: usize = 3;
/// Physical corners in `border-radius` order, as the two sides meeting there.
const CORNER_SIDES: [[usize; 2]; 4] = [[TOP, LEFT], [TOP, RIGHT], [BOTTOM, RIGHT], [BOTTOM, LEFT]];
/// Physical dimensions: `width`/`overflow-x`, then `height`/`overflow-y`.
const HORIZONTAL: usize = 0;
const VERTICAL: usize = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WritingMode {
    #[default]
    HorizontalTb,
}

/// How flow-relative terms map to physical ones for one box (CSS Writing Modes 4 §6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
            // The registry computes no `writing-mode` but its initial `horizontal-tb`.
            writing_mode: WritingMode::HorizontalTb,
            rtl: style.rtl,
        }
    }

    fn side(self, axis: Axis, edge: Edge) -> usize {
        match (self.writing_mode, axis) {
            (WritingMode::HorizontalTb, Axis::Block) => match edge {
                Edge::Start => TOP,
                Edge::End => BOTTOM,
            },
            (WritingMode::HorizontalTb, Axis::Inline) => {
                if (edge == Edge::Start) == self.rtl {
                    RIGHT
                } else {
                    LEFT
                }
            }
        }
    }

    fn dimension(self, axis: Axis) -> usize {
        match (self.writing_mode, axis) {
            (WritingMode::HorizontalTb, Axis::Inline) => HORIZONTAL,
            (WritingMode::HorizontalTb, Axis::Block) => VERTICAL,
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

/// `source` with its box properties arranged for `flow`: each physical property holds the
/// value of the property corresponding to it in `source`'s own flow.
pub(crate) fn reflowed(source: &Style, flow: Flow) -> Style {
    let from = Flow::of(source);
    let side = |index| flow.source(from, &SIDES, index);
    let corner = |index| flow.source(from, &CORNERS, index);
    let dimension = |index| flow.source(from, &DIMENSIONS, index);
    let dimensions = |values: [&crate::style::Size; 2]| -> [crate::style::Size; 2] {
        std::array::from_fn(|index| values[dimension(index)].clone())
    };
    let mut style = source.clone();
    style.margin = std::array::from_fn(|index| source.margin[side(index)].clone());
    style.padding = std::array::from_fn(|index| source.padding[side(index)].clone());
    style.inset = std::array::from_fn(|index| source.inset[side(index)].clone());
    style.decoration.borders = std::array::from_fn(|index| source.decoration.borders[side(index)]);
    style.decoration.radii = Arc::new(std::array::from_fn(|index| {
        source.decoration.radii[corner(index)].clone()
    }));
    [style.width, style.height] = dimensions([&source.width, &source.height]);
    [style.min_width, style.min_height] = dimensions([&source.min_width, &source.min_height]);
    [style.max_width, style.max_height] = dimensions([&source.max_width, &source.max_height]);
    style.overflow = std::array::from_fn(|index| source.overflow[dimension(index)]);
    style.overflow_clip_margin =
        std::array::from_fn(|index| source.overflow_clip_margin[side(index)]);
    for group in &GROUPS[INERT_GROUPS] {
        for (index, property) in group.physical.iter().enumerate() {
            let from = group.physical[flow.source(from, group.positions, index)];
            crate::style::inert::copy_from(*property, &mut style, from, source);
        }
    }
    style
}
