//! Shorthand expansion from match trees. Omitted longhands take their initial value unless the
//! defining specification says otherwise.
use crate::style::{
    StyleError,
    component::Component,
    declaration::{self, Specified},
    grammar::{Keyword, Label, NODES, Node, Production, Property},
    logical::{BOTTOM, LEFT},
    wide_keywords::WideKeyword,
};
use std::sync::Arc;

type Longhands = Vec<(Property, Specified)>;

/// `<line-width> || <line-style> || <color>` longhands of one border side.
type BorderSide = [Property; 3];
const BORDER_TOP: BorderSide = [
    Property::BorderTopWidth,
    Property::BorderTopStyle,
    Property::BorderTopColor,
];
const BORDER_RIGHT: BorderSide = [
    Property::BorderRightWidth,
    Property::BorderRightStyle,
    Property::BorderRightColor,
];
const BORDER_BOTTOM: BorderSide = [
    Property::BorderBottomWidth,
    Property::BorderBottomStyle,
    Property::BorderBottomColor,
];
const BORDER_LEFT: BorderSide = [
    Property::BorderLeftWidth,
    Property::BorderLeftStyle,
    Property::BorderLeftColor,
];
const BORDER_BLOCK_START: BorderSide = [
    Property::BorderBlockStartWidth,
    Property::BorderBlockStartStyle,
    Property::BorderBlockStartColor,
];
const BORDER_BLOCK_END: BorderSide = [
    Property::BorderBlockEndWidth,
    Property::BorderBlockEndStyle,
    Property::BorderBlockEndColor,
];
const BORDER_INLINE_START: BorderSide = [
    Property::BorderInlineStartWidth,
    Property::BorderInlineStartStyle,
    Property::BorderInlineStartColor,
];
const BORDER_INLINE_END: BorderSide = [
    Property::BorderInlineEndWidth,
    Property::BorderInlineEndStyle,
    Property::BorderInlineEndColor,
];
const BORDER_IMAGE: [Property; 5] = [
    Property::BorderImageSource,
    Property::BorderImageSlice,
    Property::BorderImageWidth,
    Property::BorderImageOutset,
    Property::BorderImageRepeat,
];
/// Source, slice with width and outset, and repeat.
const BORDER_IMAGE_SLOTS: usize = 3;
/// `bg-layer` slots; the final layer adds `<'background-color'>` (CSS Backgrounds 3 §3.10).
const LAYER_IMAGE: usize = 0;
const LAYER_POSITION: usize = 1;
const LAYER_REPEAT: usize = 2;
const LAYER_ATTACHMENT: usize = 3;
const LAYER_ORIGIN: usize = 4;
const LAYER_CLIP: usize = 5;
const LAYER_COLOR: usize = 6;
/// CSS Flexbox 1 §7.1.1: `flex: <grow>` means `<grow> 1 0`.
const FLEX_OMITTED_FACTOR: &str = "1";
const FLEX_OMITTED_BASIS: &str = "0%";
const FLEX_NONE: [&str; 3] = ["0", "0", "auto"];
/// CSS Grid 1 §7.8: the auto-flow keyword's side of the slash names the flow axis.
const ROW_FLOW: &str = "row";
const COLUMN_FLOW: &str = "column";
const DENSE: &str = "dense";
/// `<single-transition>` slots in grammar order; the first `<time>` is the duration.
const TRANSITION: [Property; 5] = [
    Property::TransitionProperty,
    Property::TransitionDuration,
    Property::TransitionTimingFunction,
    Property::TransitionDelay,
    Property::TransitionBehavior,
];
/// `<single-animation>` slots in grammar order.
const ANIMATION: [Property; 9] = [
    Property::AnimationDuration,
    Property::AnimationTimingFunction,
    Property::AnimationDelay,
    Property::AnimationIterationCount,
    Property::AnimationDirection,
    Property::AnimationFillMode,
    Property::AnimationPlayState,
    Property::AnimationName,
    Property::AnimationTimeline,
];
/// Scroll-driven Animations 1 §A: reset-only sub-properties of `animation`.
const ANIMATION_RESET: [Property; 2] = [Property::AnimationRangeStart, Property::AnimationRangeEnd];
/// Scroll-driven Animations 1 §A: an omitted range end with a named start ends that range.
const NAMED_RANGE_END: f32 = 1.0;
/// CSS Text 4 §3: the `white-space` keywords and their collapse and wrap-mode longhands.
const WHITE_SPACE_KEYWORDS: [(Keyword, &str, &str); 4] = [
    (Keyword::Normal, "collapse", "wrap"),
    (Keyword::Pre, "preserve", "nowrap"),
    (Keyword::PreWrap, "preserve", "wrap"),
    (Keyword::PreLine, "preserve-breaks", "wrap"),
];
/// CSS Overflow 4 §5.1 and §5.1.1: the `continue` value a clamping integer sets.
const CLAMP_DISCARD: &str = "discard";
const CLAMP_WEBKIT_DISCARD: &str = "-webkit-discard";
const CLAMP_ELLIPSIS: &str = "auto";
/// Compat §3.4.4: `-webkit-box-orient` maps onto `flex-direction`.
const ORIENT_ROW: &str = "row";
const ORIENT_COLUMN: &str = "column";
const SYNTHESIS_ON: &str = "auto";
const SYNTHESIS_OFF: &str = "none";

pub(crate) fn expand(shorthand: Property, value: Component) -> Result<Longhands, StyleError> {
    use Property as P;
    Ok(match shorthand {
        P::Margin => edges(
            &value,
            [P::MarginTop, P::MarginRight, P::MarginBottom, P::MarginLeft],
        ),
        P::Padding => edges(
            &value,
            [
                P::PaddingTop,
                P::PaddingRight,
                P::PaddingBottom,
                P::PaddingLeft,
            ],
        ),
        P::Inset => edges(&value, [P::Top, P::Right, P::Bottom, P::Left]),
        P::BorderWidth => edges(
            &value,
            [
                P::BorderTopWidth,
                P::BorderRightWidth,
                P::BorderBottomWidth,
                P::BorderLeftWidth,
            ],
        ),
        P::BorderStyle => edges(
            &value,
            [
                P::BorderTopStyle,
                P::BorderRightStyle,
                P::BorderBottomStyle,
                P::BorderLeftStyle,
            ],
        ),
        P::BorderColor => edges(
            &value,
            [
                P::BorderTopColor,
                P::BorderRightColor,
                P::BorderBottomColor,
                P::BorderLeftColor,
            ],
        ),
        P::BorderRadius => radius(&value),
        // CSS Backgrounds 3 §4.4: `border` also resets `border-image`.
        P::Border => border(
            &value,
            &[BORDER_TOP, BORDER_RIGHT, BORDER_BOTTOM, BORDER_LEFT],
        )
        .into_iter()
        .chain(BORDER_IMAGE.map(|property| (property, initial())))
        .collect(),
        P::BorderImage => border_image(&value),
        P::Columns => columns(&value),
        P::BorderTop => border(&value, &[BORDER_TOP]),
        P::BorderRight => border(&value, &[BORDER_RIGHT]),
        P::BorderBottom => border(&value, &[BORDER_BOTTOM]),
        P::BorderLeft => border(&value, &[BORDER_LEFT]),
        P::MarginBlock => pair(&value, [P::MarginBlockStart, P::MarginBlockEnd]),
        P::MarginInline => pair(&value, [P::MarginInlineStart, P::MarginInlineEnd]),
        P::PaddingBlock => pair(&value, [P::PaddingBlockStart, P::PaddingBlockEnd]),
        P::PaddingInline => pair(&value, [P::PaddingInlineStart, P::PaddingInlineEnd]),
        P::InsetBlock => pair(&value, [P::InsetBlockStart, P::InsetBlockEnd]),
        P::InsetInline => pair(&value, [P::InsetInlineStart, P::InsetInlineEnd]),
        P::BorderBlockWidth => pair(&value, [P::BorderBlockStartWidth, P::BorderBlockEndWidth]),
        P::BorderInlineWidth => pair(&value, [P::BorderInlineStartWidth, P::BorderInlineEndWidth]),
        P::BorderBlockStyle => pair(&value, [P::BorderBlockStartStyle, P::BorderBlockEndStyle]),
        P::BorderInlineStyle => pair(&value, [P::BorderInlineStartStyle, P::BorderInlineEndStyle]),
        P::BorderBlockColor => pair(&value, [P::BorderBlockStartColor, P::BorderBlockEndColor]),
        P::BorderInlineColor => pair(&value, [P::BorderInlineStartColor, P::BorderInlineEndColor]),
        P::BorderBlockStart => border(&value, &[BORDER_BLOCK_START]),
        P::BorderBlockEnd => border(&value, &[BORDER_BLOCK_END]),
        P::BorderInlineStart => border(&value, &[BORDER_INLINE_START]),
        P::BorderInlineEnd => border(&value, &[BORDER_INLINE_END]),
        // CSS Logical 1 §4.5.4: both sides take the same value.
        P::BorderBlock => border(&value, &[BORDER_BLOCK_START, BORDER_BLOCK_END]),
        P::BorderInline => border(&value, &[BORDER_INLINE_START, BORDER_INLINE_END]),
        P::Overflow => pair(&value, [P::OverflowX, P::OverflowY]),
        P::Gap | P::GridGap => pair(&value, [P::RowGap, P::ColumnGap]),
        P::GridRowGap => alias(&value, P::RowGap),
        P::GridColumnGap => alias(&value, P::ColumnGap),
        P::GridRow => grid_lines(&value, [P::GridRowStart, P::GridRowEnd]),
        P::GridColumn => grid_lines(&value, [P::GridColumnStart, P::GridColumnEnd]),
        P::GridArea => grid_area(&value),
        P::GridTemplate => grid_template(&value),
        P::Grid => grid(&value)?,
        P::PlaceContent => pair(&value, [P::AlignContent, P::JustifyContent]),
        P::PlaceItems => pair(&value, [P::AlignItems, P::JustifyItems]),
        P::PlaceSelf => pair(&value, [P::AlignSelf, P::JustifySelf]),
        P::Flex => flex(&value)?,
        P::Font => crate::style::font_values::expand_font(&value)?,
        P::FontStretch => alias(value.unwrap(), P::FontWidth),
        P::FontVariant => crate::style::font_values::expand_variant(&value)?,
        P::FontSynthesis => font_synthesis(&value)?,
        P::Background => background(&value)?,
        P::BackgroundPosition => background_position(&value)?,
        P::PageBreakBefore => legacy_break(&value, P::BreakBefore)?,
        P::PageBreakAfter => legacy_break(&value, P::BreakAfter)?,
        P::PageBreakInside => legacy_break(&value, P::BreakInside)?,
        P::WordWrap => alias(&value, P::OverflowWrap),
        P::WhiteSpace => white_space(&value)?,
        P::TextWrap => slotted(&value, &[P::TextWrapMode, P::TextWrapStyle]),
        P::LineClamp => line_clamp(&value, false)?,
        P::DashWebkitLineClamp => line_clamp(&value, true)?,
        P::DashWebkitBoxOrient => box_orient(&value)?,
        P::Transition => slotted_list(&value, &TRANSITION),
        P::Animation => slotted_list(&value, &ANIMATION)
            .into_iter()
            .chain(ANIMATION_RESET.map(|property| (property, initial())))
            .collect(),
        P::ScrollTimeline => scroll_timeline(&value),
        P::ViewTimeline => view_timeline(&value),
        P::AnimationRange => animation_range(&value),
        P::ScrollMargin => edges(
            &value,
            [
                P::ScrollMarginTop,
                P::ScrollMarginRight,
                P::ScrollMarginBottom,
                P::ScrollMarginLeft,
            ],
        ),
        P::ScrollPadding => edges(
            &value,
            [
                P::ScrollPaddingTop,
                P::ScrollPaddingRight,
                P::ScrollPaddingBottom,
                P::ScrollPaddingLeft,
            ],
        ),
        P::OverscrollBehavior => pair(&value, [P::OverscrollBehaviorX, P::OverscrollBehaviorY]),
        P::InterestDelay => pair(&value, [P::InterestDelayStart, P::InterestDelayEnd]),
        _ => {
            if let Some(longhands) = unordered_references(shorthand, &value)? {
                longhands
            } else {
                let value = Arc::new(value);
                declaration::longhands(shorthand)
                    .map(|longhand| (longhand, Specified::Unexpanded(shorthand, value.clone())))
                    .collect()
            }
        }
    })
}

/// A component as the value of `longhand`, shaped as parsing it directly would be.
pub(super) fn longhand(longhand: Property, component: &Component) -> Specified {
    let inner = match component {
        Component::Reference(Label::Property(_), inner) => inner.as_ref().clone(),
        other => other.clone(),
    };
    Specified::Value(Arc::new(Component::Reference(
        Label::Property(longhand),
        Box::new(inner),
    )))
}

fn initial() -> Specified {
    Specified::Wide(WideKeyword::Initial)
}

/// A value the shorthand's specification gives for omitted longhands.
fn literal(property: Property, source: &str) -> Result<Specified, StyleError> {
    declaration::value(property, source, &mut 0)?
        .map(|value| Specified::Value(Arc::new(value)))
        .ok_or_else(|| StyleError::Invalid(format!("{source} is not a {}", property.name())))
}

/// CSS Backgrounds 3 §4.1: one to four values for top, right, bottom and left.
fn edges(value: &Component, sides: [Property; 4]) -> Longhands {
    let items = value.items();
    let index = |side: usize| match (items.len(), side) {
        (1, _) => 0,
        (2 | 3, LEFT) => 1,
        (2, BOTTOM) => 0,
        _ => side,
    };
    sides
        .iter()
        .enumerate()
        .map(|(side, property)| (*property, longhand(*property, &items[index(side)])))
        .collect()
}

/// Two values where an omitted second copies the first.
fn pair(value: &Component, properties: [Property; 2]) -> Longhands {
    let items: Vec<&Component> = match value.unwrap() {
        Component::Group(items) => items
            .iter()
            .flat_map(|item| match item {
                Component::Group(nested) => nested.iter().collect(),
                single => vec![single],
            })
            .collect(),
        single => vec![single],
    };
    let first = items[0];
    let second = items.get(1).copied().unwrap_or(first);
    vec![
        (properties[0], longhand(properties[0], first)),
        (properties[1], longhand(properties[1], second)),
    ]
}

/// `<length-percentage>{1,4} [ / <length-percentage>{1,4} ]?`; an omitted vertical list
/// repeats the horizontal one.
fn radius(value: &Component) -> Longhands {
    let [horizontal, vertical] = match value.unwrap() {
        Component::Group(parts) => [
            parts[0].items(),
            parts[1]
                .items()
                .first()
                .and_then(|slash| slash.items().get(1))
                .map_or(&[][..], Component::items),
        ],
        single => [std::slice::from_ref(single), &[][..]],
    };
    let corner = |values: &[Component], index: usize| match (values.len(), index) {
        (1, _) => 0,
        (2 | 3, 3) => 1,
        (2, 2) => 0,
        _ => index,
    };
    [
        Property::BorderTopLeftRadius,
        Property::BorderTopRightRadius,
        Property::BorderBottomRightRadius,
        Property::BorderBottomLeftRadius,
    ]
    .iter()
    .enumerate()
    .map(|(index, property)| {
        let mut radii = vec![horizontal[corner(horizontal, index)].clone()];
        if !vertical.is_empty() {
            radii.push(vertical[corner(vertical, index)].clone());
        }
        (*property, longhand(*property, &Component::Group(radii)))
    })
    .collect()
}

/// `<line-width> || <line-style> || <color>` for each side.
fn border(value: &Component, sides: &[BorderSide]) -> Longhands {
    sides
        .iter()
        .flat_map(|side| {
            side.iter().enumerate().map(|(part, property)| {
                let specified = value
                    .slot(part)
                    .map_or_else(initial, |slot| longhand(*property, slot));
                (*property, specified)
            })
        })
        .collect()
}

/// `<'border-image-source'> || <'border-image-slice'> [ / <'border-image-width'> |
/// / <'border-image-width'>? / <'border-image-outset'> ]? || <'border-image-repeat'>`.
fn border_image(value: &Component) -> Longhands {
    BORDER_IMAGE
        .iter()
        .map(|property| {
            let found = (0..BORDER_IMAGE_SLOTS)
                .filter_map(|slot| value.slot(slot))
                .find_map(|slot| slot.find(Label::Property(*property)));
            (
                *property,
                found.map_or_else(initial, |component| longhand(*property, component)),
            )
        })
        .collect()
}

/// `[ <'column-width'> || <'column-count'> ] [ / <'column-height'> ]?`.
fn columns(value: &Component) -> Longhands {
    [
        Property::ColumnWidth,
        Property::ColumnCount,
        Property::ColumnHeight,
    ]
    .iter()
    .map(|property| {
        (
            *property,
            value
                .find(Label::Property(*property))
                .map_or_else(initial, |component| longhand(*property, component)),
        )
    })
    .collect()
}

fn flex(value: &Component) -> Result<Longhands, StyleError> {
    let properties = [
        Property::FlexGrow,
        Property::FlexShrink,
        Property::FlexBasis,
    ];
    if value.keyword() == Some(Keyword::None) {
        return properties
            .iter()
            .zip(FLEX_NONE)
            .map(|(property, source)| Ok((*property, literal(*property, source)?)))
            .collect();
    }
    let factors = value.slot(0);
    let grow = factors.and_then(|factors| factors.find(Label::Property(Property::FlexGrow)));
    let shrink = factors.and_then(|factors| factors.find(Label::Property(Property::FlexShrink)));
    let basis = value.slot(1);
    let resolve = |property: Property, found: Option<&Component>, omitted: &str| {
        found.map_or_else(
            || literal(property, omitted),
            |component| Ok(longhand(property, component)),
        )
    };
    Ok(vec![
        (
            Property::FlexGrow,
            resolve(Property::FlexGrow, grow, FLEX_OMITTED_FACTOR)?,
        ),
        (
            Property::FlexShrink,
            resolve(Property::FlexShrink, shrink, FLEX_OMITTED_FACTOR)?,
        ),
        (
            Property::FlexBasis,
            resolve(Property::FlexBasis, basis, FLEX_OMITTED_BASIS)?,
        ),
    ])
}

/// CSS Fonts 4 §3.9: each listed feature may be synthesized; the rest may not.
fn font_synthesis(value: &Component) -> Result<Longhands, StyleError> {
    [
        Property::FontSynthesisWeight,
        Property::FontSynthesisStyle,
        Property::FontSynthesisSmallCaps,
        Property::FontSynthesisPosition,
    ]
    .iter()
    .enumerate()
    .map(|(index, property)| {
        let allowed = value.slot(index).is_some();
        Ok((
            *property,
            literal(
                *property,
                if allowed { SYNTHESIS_ON } else { SYNTHESIS_OFF },
            )?,
        ))
    })
    .collect()
}

/// CSS Fragmentation 3 §3.4: legacy `page-break-*` map `always` to `page`.
fn legacy_break(value: &Component, property: Property) -> Result<Longhands, StyleError> {
    let keyword = match value.keyword() {
        Some(Keyword::Always) => Keyword::Page,
        Some(keyword) => keyword,
        None => return Err(StyleError::Invalid(format!("legacy break {value}"))),
    };
    Ok(vec![(property, literal(property, keyword.name())?)])
}

fn white_space(value: &Component) -> Result<Longhands, StyleError> {
    if let Some(keyword) = value.keyword()
        && let Some((_, collapse, wrap)) = WHITE_SPACE_KEYWORDS
            .iter()
            .find(|(candidate, ..)| *candidate == keyword)
    {
        return Ok(vec![
            (
                Property::WhiteSpaceCollapse,
                literal(Property::WhiteSpaceCollapse, collapse)?,
            ),
            (
                Property::TextWrapMode,
                literal(Property::TextWrapMode, wrap)?,
            ),
        ]);
    }
    Ok(slotted(
        value,
        &[Property::WhiteSpaceCollapse, Property::TextWrapMode],
    ))
}

/// `||` slots in grammar order, one per longhand; omitted ones take their initial value.
fn slotted(value: &Component, properties: &[Property]) -> Longhands {
    properties
        .iter()
        .enumerate()
        .map(|(slot, &property)| {
            let specified = value
                .slot(slot)
                .map_or_else(initial, |slot| longhand(property, slot));
            (property, specified)
        })
        .collect()
}

/// `none` resets the longhands; an integer also discards later content and adds an
/// ellipsis. The legacy form always sets the ellipsis and discards only in `-webkit-box`.
fn line_clamp(value: &Component, legacy: bool) -> Result<Longhands, StyleError> {
    let ellipsis = match value.find(Label::Property(Property::BlockEllipsis)) {
        Some(ellipsis) => longhand(Property::BlockEllipsis, ellipsis),
        None if legacy || value.keyword() != Some(Keyword::None) => {
            literal(Property::BlockEllipsis, CLAMP_ELLIPSIS)?
        }
        None => initial(),
    };
    let (lines, continuation) = if value.keyword() == Some(Keyword::None) {
        (initial(), initial())
    } else {
        (
            longhand(Property::MaxLines, &value.items()[0]),
            literal(
                Property::Continue,
                if legacy {
                    CLAMP_WEBKIT_DISCARD
                } else {
                    CLAMP_DISCARD
                },
            )?,
        )
    };
    Ok(vec![
        (Property::MaxLines, lines),
        (Property::BlockEllipsis, ellipsis),
        (Property::Continue, continuation),
    ])
}

fn box_orient(value: &Component) -> Result<Longhands, StyleError> {
    let direction = match value.keyword() {
        Some(Keyword::Horizontal | Keyword::InlineAxis) => ORIENT_ROW,
        Some(Keyword::Vertical | Keyword::BlockAxis) => ORIENT_COLUMN,
        _ => return Err(StyleError::Invalid(format!("box orientation {value}"))),
    };
    Ok(vec![(
        Property::FlexDirection,
        literal(Property::FlexDirection, direction)?,
    )])
}

/// A legacy name alias sharing the value of the property it names.
fn alias(value: &Component, property: Property) -> Longhands {
    vec![(property, longhand(property, value))]
}

/// Comma-separated `||` items, each slot setting one item of the longhand at its index.
fn slotted_list(value: &Component, properties: &[Property]) -> Longhands {
    let items: Vec<&Component> = value.items().iter().collect();
    properties
        .iter()
        .enumerate()
        .map(|(index, property)| coordinated(*property, &items, &|item| item.slot(index).cloned()))
        .collect()
}

/// The optional part after an item's first component, which the grammar groups.
fn optional(item: &Component) -> Option<&Component> {
    item.items().get(1).and_then(|part| part.items().first())
}

/// `[ <name> <axis>? ]#`.
fn scroll_timeline(value: &Component) -> Longhands {
    let items: Vec<&Component> = value.items().iter().collect();
    vec![
        coordinated(Property::ScrollTimelineName, &items, &|item| {
            item.items().first().cloned()
        }),
        coordinated(Property::ScrollTimelineAxis, &items, &|item| {
            optional(item).cloned()
        }),
    ]
}

/// `[ <name> [ <axis> || <inset> ]? ]#`.
fn view_timeline(value: &Component) -> Longhands {
    let items: Vec<&Component> = value.items().iter().collect();
    let part = |index: usize| move |item: &Component| optional(item)?.slot(index).cloned();
    vec![
        coordinated(Property::ViewTimelineName, &items, &|item| {
            item.items().first().cloned()
        }),
        coordinated(Property::ViewTimelineAxis, &items, &part(0)),
        coordinated(Property::ViewTimelineInset, &items, &part(1)),
    ]
}

/// `[ <start> <end>? ]#`; an omitted end after a named start is that name at 100%, and is
/// otherwise initial (Scroll-driven Animations 1 §A).
fn animation_range(value: &Component) -> Longhands {
    let items: Vec<&Component> = value.items().iter().collect();
    let end = |item: &Component| {
        optional(item).cloned().or_else(|| {
            let name = item
                .items()
                .first()?
                .find(Label::Production(Production::TimelineRangeName))?;
            Some(Component::Group(vec![
                Component::Reference(
                    Label::Production(Production::TimelineRangeName),
                    Box::new(name.clone()),
                ),
                Component::Group(vec![Component::Percentage(NAMED_RANGE_END)]),
            ]))
        })
    };
    vec![
        coordinated(Property::AnimationRangeStart, &items, &|item| {
            item.items().first().cloned()
        }),
        coordinated(Property::AnimationRangeEnd, &items, &end),
    ]
}

/// Per-layer longhand lists from `[ <bg-layer> , ]* <final-bg-layer>`.
fn background(value: &Component) -> Result<Longhands, StyleError> {
    let mut layers = Vec::new();
    value.unwrap().references(
        &[
            Label::Production(Production::BgLayer),
            Label::Production(Production::FinalBgLayer),
        ],
        &mut layers,
    );
    let layers: Vec<&Component> = layers.into_iter().map(|(_, layer)| layer).collect();
    let position = |layer: &Component| {
        layer
            .slot(LAYER_POSITION)
            .and_then(|slot| slot.find(Label::Production(Production::BgPosition)))
            .map(|found| {
                Component::Reference(
                    Label::Production(Production::BgPosition),
                    Box::new(found.clone()),
                )
            })
    };
    let size = |layer: &Component| {
        layer
            .slot(LAYER_POSITION)
            .and_then(|slot| slot.find(Label::Production(Production::BgSize)))
            .map(|found| {
                Component::Reference(
                    Label::Production(Production::BgSize),
                    Box::new(found.clone()),
                )
            })
    };
    let slot = |index: usize| move |layer: &Component| layer.slot(index).cloned();
    let clip = |layer: &Component| {
        layer
            .slot(LAYER_CLIP)
            .or_else(|| layer.slot(LAYER_ORIGIN))
            .map(|visual_box| {
                Component::Reference(
                    Label::Production(Production::BgClip),
                    Box::new(visual_box.clone()),
                )
            })
    };
    let mut longhands = vec![
        layered(Property::BackgroundImage, &layers, &slot(LAYER_IMAGE)),
        layered(Property::BackgroundSize, &layers, &size),
        layered(Property::BackgroundRepeat, &layers, &slot(LAYER_REPEAT)),
        layered(
            Property::BackgroundAttachment,
            &layers,
            &slot(LAYER_ATTACHMENT),
        ),
        layered(Property::BackgroundOrigin, &layers, &slot(LAYER_ORIGIN)),
        layered(Property::BackgroundClip, &layers, &clip),
    ];
    let color = layers
        .last()
        .and_then(|layer| layer.slot(LAYER_COLOR))
        .map_or_else(initial, |color| longhand(Property::BackgroundColor, color));
    longhands.push((Property::BackgroundColor, color));
    let positions: Vec<Option<Component>> = layers.iter().map(|layer| position(layer)).collect();
    longhands.extend(axis_lists(
        &positions.iter().map(Option::as_ref).collect::<Vec<_>>(),
    )?);
    Ok(longhands)
}

/// CSS Backgrounds 4: `background-position` sets `background-position-x` and `-y`.
fn background_position(value: &Component) -> Result<Longhands, StyleError> {
    axis_lists(&value.items().iter().map(Some).collect::<Vec<_>>())
}

/// `background-position-x` and `-y` lists with one item per `<bg-position>`, where an omitted
/// axis is `center` and an omitted position the longhands' initial item.
fn axis_lists(positions: &[Option<&Component>]) -> Result<Longhands, StyleError> {
    let axes = [Property::BackgroundPositionX, Property::BackgroundPositionY];
    if positions.iter().all(Option::is_none) {
        return Ok(axes.map(|axis| (axis, initial())).into());
    }
    let initial_item = |axis: Property| {
        declaration::initial(axis)
            .and_then(|initial| initial.items().first())
            .cloned()
            .ok_or_else(|| StyleError::Invalid(format!("{} has no initial item", axis.name())))
    };
    let mut lists = [Vec::new(), Vec::new()];
    for position in positions {
        let items = match position {
            Some(position) => crate::style::background_values::position_axes(position)?
                .map(|part| part.cloned().unwrap_or(Component::Keyword(Keyword::Center))),
            None => [initial_item(axes[0])?, initial_item(axes[1])?],
        };
        for (list, item) in lists.iter_mut().zip(items) {
            list.push(item);
        }
    }
    Ok(axes
        .into_iter()
        .zip(lists)
        .map(|(axis, items)| (axis, longhand(axis, &Component::Group(items))))
        .collect())
}

/// One list item per layer; layers that omit it repeat the longhand's initial item.
fn layered(
    property: Property,
    layers: &[&Component],
    item: &dyn Fn(&Component) -> Option<Component>,
) -> (Property, Specified) {
    if layers.iter().all(|layer| item(layer).is_none()) {
        return (property, initial());
    }
    coordinated(property, layers, item)
}

/// One item per entry, so coordinated lists keep the shorthand's length; entries that omit
/// it take the longhand's initial item.
fn coordinated(
    property: Property,
    entries: &[&Component],
    item: &dyn Fn(&Component) -> Option<Component>,
) -> (Property, Specified) {
    let fallback = declaration::initial(property)
        .and_then(|initial| initial.items().first())
        .cloned();
    let items = entries
        .iter()
        .map(|entry| item(entry).or_else(|| fallback.clone()))
        .collect::<Option<Vec<_>>>();
    match items {
        Some(items) => (property, longhand(property, &Component::Group(items))),
        None => (property, initial()),
    }
}

/// Shorthands whose grammar is `<'a'> || <'b'> …` (or `&&`) over their own longhands: each
/// slot is its longhand's value and omitted longhands are initial.
fn unordered_references(
    shorthand: Property,
    value: &Component,
) -> Result<Option<Longhands>, StyleError> {
    let Node::Reference(_, target) = NODES[usize::from(shorthand.data().root)] else {
        return Ok(None);
    };
    let (Node::AnyOf(alternatives) | Node::AllOf(alternatives)) = &NODES[usize::from(target)]
    else {
        return Ok(None);
    };
    let direct = shorthand.longhands();
    let Some(properties) = alternatives
        .iter()
        .map(|alternative| match NODES[usize::from(*alternative)] {
            Node::Reference(Label::Property(property), _) if direct.contains(&property) => {
                Some(property)
            }
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(None);
    };
    if direct.iter().any(|longhand| !properties.contains(longhand)) {
        return Ok(None);
    }
    let mut longhands = Longhands::new();
    for (index, property) in properties.into_iter().enumerate() {
        match value.slot(index) {
            Some(slot) if !property.longhands().is_empty() => {
                longhands.extend(expand(property, slot.clone())?);
            }
            Some(slot) => longhands.push((property, longhand(property, slot))),
            None => longhands.extend(declaration::longhands(property).map(|l| (l, initial()))),
        }
    }
    Ok(Some(longhands))
}

/// A bare `<custom-ident>`, which omitted grid lines copy (CSS Grid 1 §8.4).
fn bare_name(line: &Component) -> bool {
    matches!(line.unwrap(), Component::Ident(_))
}

/// `grid-row` and `grid-column`: `<grid-line> [ / <grid-line> ]?`.
fn grid_lines(value: &Component, properties: [Property; 2]) -> Longhands {
    grid_placement(value, &properties, &[None, Some(0)])
}

/// `grid-area`: row start, column start, row end and column end. Omitted ends copy their
/// start; an omitted column start copies the row start.
fn grid_area(value: &Component) -> Longhands {
    grid_placement(
        value,
        &[
            Property::GridRowStart,
            Property::GridColumnStart,
            Property::GridRowEnd,
            Property::GridColumnEnd,
        ],
        &[None, Some(0), Some(0), Some(1)],
    )
}

/// Placement longhands from the written lines; an omitted one copies the line at its
/// `sources` index when that is a bare name, and is `auto` otherwise.
fn grid_placement(
    value: &Component,
    properties: &[Property],
    sources: &[Option<usize>],
) -> Longhands {
    let mut lines = Vec::new();
    value.references(&[Label::Production(Production::GridLine)], &mut lines);
    let mut resolved: Vec<Option<&Component>> = Vec::with_capacity(properties.len());
    for (index, source) in sources.iter().enumerate() {
        let line = lines.get(index).map(|(_, line)| *line).or_else(|| {
            source
                .and_then(|source| resolved[source])
                .filter(|line| bare_name(line))
        });
        resolved.push(line);
    }
    properties
        .iter()
        .zip(resolved)
        .map(|(property, line)| {
            (
                *property,
                line.map_or_else(initial, |line| longhand(*property, line)),
            )
        })
        .collect()
}

/// `auto`, the track size a template row without one takes.
fn auto_track() -> Component {
    Component::Reference(
        Label::Production(Production::TrackSize),
        Box::new(Component::Reference(
            Label::Production(Production::TrackBreadth),
            Box::new(Component::Keyword(Keyword::Auto)),
        )),
    )
}

/// `grid-template`: `none`, `<rows> / <columns>`, or area strings with row sizes and names.
fn grid_template(value: &Component) -> Longhands {
    let properties = [
        Property::GridTemplateRows,
        Property::GridTemplateColumns,
        Property::GridTemplateAreas,
    ];
    if value.keyword() == Some(Keyword::None) {
        return properties
            .iter()
            .map(|property| (*property, initial()))
            .collect();
    }
    if let Some(rows) = value.find(Label::Property(Property::GridTemplateRows)) {
        let columns = value.find(Label::Property(Property::GridTemplateColumns));
        return vec![
            (properties[0], longhand(properties[0], rows)),
            (
                properties[1],
                columns.map_or_else(initial, |columns| longhand(properties[1], columns)),
            ),
            (properties[2], initial()),
        ];
    }
    let parts = value.items();
    let rows = parts.first().map_or(&[][..], Component::items);
    let mut tracks = Vec::new();
    let mut strings = Vec::new();
    for row in rows {
        let [before, string, size, after] = row.items() else {
            continue;
        };
        tracks.extend(before.items().iter().cloned());
        strings.push(string.clone());
        tracks.push(size.items().first().cloned().unwrap_or_else(auto_track));
        tracks.extend(after.items().iter().cloned());
    }
    let columns = parts
        .get(1)
        .and_then(|tail| tail.find(Label::Production(Production::ExplicitTrackList)));
    vec![
        (
            properties[0],
            longhand(properties[0], &Component::Group(tracks)),
        ),
        (
            properties[1],
            columns.map_or_else(initial, |columns| longhand(properties[1], columns)),
        ),
        (
            properties[2],
            longhand(properties[2], &Component::Group(strings)),
        ),
    ]
}

/// `grid`: a template, or explicit tracks on one axis and auto-flow tracks on the other.
fn grid(value: &Component) -> Result<Longhands, StyleError> {
    let template = match value {
        Component::Reference(Label::Property(Property::Grid), inner) => inner.as_ref(),
        other => other,
    };
    let mut implicit = vec![
        (Property::GridAutoRows, initial()),
        (Property::GridAutoColumns, initial()),
        (Property::GridAutoFlow, initial()),
    ];
    if matches!(
        template,
        Component::Reference(Label::Property(Property::GridTemplate), _)
    ) {
        let mut longhands = grid_template(template);
        longhands.append(&mut implicit);
        return Ok(longhands);
    }
    let mut keywords = Vec::new();
    value.keywords(&mut keywords);
    let dense = keywords.contains(&Keyword::Dense);
    let rows = value.find(Label::Property(Property::GridTemplateRows));
    let (template, auto, flow) = match rows {
        Some(rows) => (
            (Property::GridTemplateRows, rows),
            Property::GridAutoColumns,
            COLUMN_FLOW,
        ),
        None => (
            (
                Property::GridTemplateColumns,
                value
                    .find(Label::Property(Property::GridTemplateColumns))
                    .ok_or_else(|| StyleError::Invalid(format!("grid: {value}")))?,
            ),
            Property::GridAutoRows,
            ROW_FLOW,
        ),
    };
    let flow = if dense {
        format!("{flow} {DENSE}")
    } else {
        flow.to_owned()
    };
    let mut longhands = vec![
        (Property::GridTemplateRows, initial()),
        (Property::GridTemplateColumns, initial()),
        (Property::GridTemplateAreas, initial()),
        (Property::GridAutoRows, initial()),
        (Property::GridAutoColumns, initial()),
        (
            Property::GridAutoFlow,
            literal(Property::GridAutoFlow, &flow)?,
        ),
    ];
    for (property, specified) in &mut longhands {
        if *property == template.0 {
            *specified = longhand(*property, template.1);
        } else if *property == auto
            && let Some(sizes) = value.find(Label::Property(auto))
        {
            *specified = longhand(*property, sizes);
        }
    }
    Ok(longhands)
}
