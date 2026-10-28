//! The registry of supported longhands: computed values from match trees. Every other
//! property is rejected here unless it keeps its initial value, which the engine renders.
use crate::{
    BoxSizing, BreakBetween, BreakInside, Display, ListStylePosition, ListStyleType, Size, Style,
    StyleError, TextAlign, Visibility, WhiteSpace,
    box_values::{length_percentage, unsupported},
    calc::Kind,
    component::Component,
    declaration::{self, Specified},
    flex_values::{Alignment, FlexBasis, FlexDirection, FlexWrap, factor, gap},
    grammar::{Keyword, Property},
    grid_values,
    text_values::{LineHeight, OverflowWrap, TextTransform},
    units::Relative,
    wide_keywords::Resolved,
};
use ferri_pdf_paint::{BorderStyle, DEFAULT_BORDER_WIDTH, LengthPercentage};

const RELATIVE_FONT_SIZE_RATIO: f32 = 6.0 / 5.0;
const THIN_BORDER_PT: f32 = 0.5625;
const THICK_BORDER_PT: f32 = 2.8125;
/// CSS Fonts 4 §2.5 absolute-size scaling factors against `medium`.
const ABSOLUTE_SIZES: [(Keyword, f32); 8] = [
    (Keyword::XxSmall, 3.0 / 5.0),
    (Keyword::XSmall, 3.0 / 4.0),
    (Keyword::Small, 8.0 / 9.0),
    (Keyword::Medium, 1.0),
    (Keyword::Large, 6.0 / 5.0),
    (Keyword::XLarge, 3.0 / 2.0),
    (Keyword::XxLarge, 2.0),
    (Keyword::XxxLarge, 3.0),
];

/// A specified value, or the computed value to copy for `inherit` and UA-dependent initials.
#[derive(Clone, Copy)]
pub(crate) enum Input<'a> {
    Value(&'a Component),
    Copy(&'a Style),
}

type Apply = fn(&mut Style, &Style, Input<'_>, &Relative<'_>) -> Result<(), StyleError>;
type Check = fn(&Component, &Relative<'_>) -> bool;

struct Handler {
    apply: Apply,
    check: Option<Check>,
}

pub(super) fn supports_property(property: Property) -> bool {
    handler(property).is_some_and(|handler| handler.check.is_some())
}

pub(super) fn supports_value(
    property: Property,
    value: &Component,
    relative: &Relative<'_>,
) -> bool {
    handler(property)
        .and_then(|handler| handler.check)
        .is_some_and(|check| check(value, relative))
}

/// Applies the cascaded value of one longhand, resolving relative lengths against `relative`.
pub(crate) fn apply(
    property: Property,
    specified: &Specified,
    style: &mut Style,
    parent: &Style,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    apply_value(
        property,
        specified,
        style,
        parent,
        relative,
        &mut std::collections::HashMap::new(),
    )
}

type ToggleChoices = std::collections::HashMap<*const [declaration::ToggleArgument], usize>;

fn apply_value(
    property: Property,
    specified: &Specified,
    style: &mut Style,
    parent: &Style,
    relative: &Relative<'_>,
    choices: &mut ToggleChoices,
) -> Result<(), StyleError> {
    if let Specified::Toggle(values) = specified {
        let _work = relative.work.enter(relative.grammar_only);
        relative.work.charge(1)?;
        let key = std::sync::Arc::as_ptr(values);
        let chosen = if let Some(&chosen) = choices.get(&key) {
            chosen
        } else {
            let chosen = toggle_index(values, &projection(property, parent), relative, |value| {
                let mut candidate = style.clone();
                if let Some(namespace) = value.font_namespace {
                    candidate.font_namespace = namespace;
                }
                apply_value(
                    property,
                    &value.value,
                    &mut candidate,
                    parent,
                    relative,
                    choices,
                )?;
                Ok(projection(property, &candidate))
            })?;
            choices.insert(key, chosen);
            chosen
        };
        if let Some(namespace) = values[chosen].font_namespace {
            style.font_namespace = namespace;
        }
        return apply_value(
            property,
            &values[chosen].value,
            style,
            parent,
            relative,
            choices,
        );
    }
    let user_agent;
    let input = match specified {
        Specified::Value(value) => Input::Value(value),
        Specified::Toggle(_) => unreachable!("toggle values were computed above"),
        Specified::Unexpanded(shorthand, value) => {
            return Err(StyleError::Unsupported(format!(
                "{}: {value}",
                shorthand.name()
            )));
        }
        Specified::Wide(keyword) => match keyword.resolve(property.inherited()) {
            Resolved::Inherit => Input::Copy(parent),
            Resolved::Initial => {
                if let Some(initial) = declaration::initial(property) {
                    Input::Value(initial)
                } else {
                    user_agent = Style::default();
                    Input::Copy(&user_agent)
                }
            }
        },
    };
    let result = match handler(property) {
        Some(handler) => (handler.apply)(style, parent, input, relative),
        None => match input {
            Input::Copy(_) => Ok(()),
            Input::Value(value) if declaration::initial(property) == Some(value) => Ok(()),
            Input::Value(_) => Err(unsupported_property(property, input)),
        },
    };
    result?;
    if property == Property::Display {
        style.normalize_display(relative);
    }
    Ok(())
}

pub(super) fn toggle_index<T: PartialEq>(
    values: &[declaration::ToggleArgument],
    inherited: &T,
    relative: &Relative<'_>,
    mut compute: impl FnMut(&declaration::ToggleArgument) -> Result<T, StyleError>,
) -> Result<usize, StyleError> {
    let mut chosen = 0;
    let mut matched = false;
    let mut next = false;
    for (index, argument) in values.iter().enumerate() {
        relative.work.charge(1)?;
        let value = compute(argument)?;
        if next {
            chosen = index;
            next = false;
        }
        if !matched && value == *inherited {
            matched = true;
            next = true;
        }
    }
    Ok(chosen)
}

pub(super) fn projection(property: Property, source: &Style) -> Style {
    let mut value = Style::default();
    copy(property, &mut value, source);
    if property == Property::ObjectPosition {
        value.object_position = value
            .object_position
            .each_ref()
            .map(ferri_pdf_paint::PositionComponent::canonical);
    }
    if property == Property::BackgroundPosition {
        for position in &mut std::sync::Arc::make_mut(&mut value.decoration.backgrounds).positions {
            *position = position
                .each_ref()
                .map(ferri_pdf_paint::PositionComponent::canonical);
        }
    }
    if property == Property::FontFamily
        && !source
            .families
            .iter()
            .any(|family| matches!(family, ferri_pdf_fonts::FontFamily::Named(_)))
    {
        value.font_namespace = Default::default();
    }
    if property == Property::Quotes {
        value.quotes = source.quotes.clone();
    }
    value
}

fn unsupported_property(property: Property, input: Input<'_>) -> StyleError {
    match input {
        Input::Value(value) => StyleError::Unsupported(format!("{}: {value}", property.name())),
        Input::Copy(_) => StyleError::Unsupported(property.name().to_owned()),
    }
}

fn quotes(
    style: &mut Style,
    parent: &Style,
    input: Input<'_>,
    _relative: &Relative<'_>,
) -> Result<(), StyleError> {
    style.quotes = match input {
        Input::Copy(source) => source.quotes.inherited(&source.language),
        Input::Value(value) => crate::Quotes::from_component(
            value,
            if parent.root_font.is_none() {
                &style.language
            } else {
                &parent.language
            },
            &parent.quotes,
        )?,
    };
    Ok(())
}

pub(super) fn copy(property: Property, target: &mut Style, source: &Style) {
    if let Some(handler) = handler(property) {
        (handler.apply)(target, source, Input::Copy(source), &Relative::initial())
            .expect("copying a computed property cannot fail");
    }
}

pub(crate) fn inherit(style: &mut Style, parent: &Style) {
    for property in Property::ALL {
        if property.inherited() {
            copy(property, style, parent);
        }
    }
}

/// Computed-field coverage is separate from usable value support.
#[cfg(test)]
pub(crate) fn computed(property: Property) -> bool {
    handler(property).is_some()
}

fn handler(property: Property) -> Option<Handler> {
    use Property as P;
    let (apply, check): (Apply, Option<Check>) = match property {
        P::StringSet => (
            string_set,
            Some(|value, _| crate::string_values::assignments(value).is_ok()),
        ),
        P::CounterReset => (
            counter_reset,
            Some(|v, r| crate::counter_values::resets(v, r).is_ok()),
        ),
        P::CounterIncrement => (
            counter_increment,
            Some(|v, r| {
                crate::counter_values::changes(v, r, crate::DEFAULT_COUNTER_INCREMENT).is_ok()
            }),
        ),
        P::CounterSet => (
            counter_set,
            Some(|v, r| crate::counter_values::changes(v, r, crate::INITIAL_COUNTER_VALUE).is_ok()),
        ),
        P::FontSize => (font_size, Some(|v, r| font_size_value(v, r).is_ok())),
        P::FontFamily => (font_family, Some(|_, _| true)),
        P::FontWeight => (
            font_weight,
            Some(|v, r| {
                crate::values::font_weight(v, ferri_pdf_fonts::FontWeight::NORMAL, r).is_ok()
            }),
        ),
        P::FontWidth => (
            font_width,
            Some(|v, r| crate::values::font_width(v, r).is_ok()),
        ),
        P::FontStyle => (
            font_style,
            Some(|v, r| crate::values::font_style(v, r).is_ok()),
        ),
        P::FontSynthesisWeight => (font_synthesis_weight, Some(|_, _| true)),
        P::FontSynthesisStyle => (font_synthesis_style, Some(|_, _| true)),
        P::FontSynthesisSmallCaps => (font_synthesis_small_caps, Some(|_, _| true)),
        P::FontSynthesisPosition => (without_variants, None),
        P::FontVariantCaps => (
            font_variant_caps,
            Some(|v, _| crate::FontVariantCaps::from_component(v).is_ok()),
        ),
        P::LineHeight => (
            line_height,
            Some(|v, r| LineHeight::from_component(v, r).is_ok()),
        ),
        P::LetterSpacing => (
            letter_spacing,
            Some(|v, r| v.keyword() == Some(Keyword::Normal) || length_percentage(v, r).is_ok()),
        ),
        P::WordSpacing => (
            word_spacing,
            Some(|v, r| v.keyword() == Some(Keyword::Normal) || length_percentage(v, r).is_ok()),
        ),
        P::TextTransform => (
            text_transform,
            Some(|v, _| TextTransform::from_component(v).is_ok()),
        ),
        P::OverflowWrap => (
            overflow_wrap,
            Some(|v, _| OverflowWrap::from_component(v).is_ok()),
        ),
        P::WhiteSpace => (
            white_space,
            Some(|v, _| WhiteSpace::from_component(v).is_ok()),
        ),
        P::TabSize => (tab_size, Some(|v, r| tab_size_value(v, r).is_ok())),
        P::Direction => (direction, Some(|_, _| true)),
        P::UnicodeBidi => (
            unicode_bidi,
            Some(|v, _| crate::UnicodeBidi::from_component(v).is_ok()),
        ),
        P::TextIndent => (
            text_indent,
            Some(|v, r| crate::TextIndent::from_component(v, r).is_ok()),
        ),
        P::TextAlign => (
            text_align,
            Some(|v, _| {
                v.keyword() == Some(Keyword::MatchParent) || TextAlign::from_component(v).is_ok()
            }),
        ),
        P::TextAlignLast => (
            text_align_last,
            Some(|v, _| v.keyword() == Some(Keyword::Auto) || TextAlign::from_component(v).is_ok()),
        ),
        P::Hyphens => (
            hyphens,
            Some(|v, _| crate::Hyphens::from_component(v).is_ok()),
        ),
        P::Color => (
            color,
            Some(|v, relative| {
                crate::colors::color(v, relative, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::ListStyleImage => (
            list_style_image,
            Some(|v, r| {
                crate::background_values::image(v, r, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::ListStyleType => (
            list_style_type,
            Some(|v, _| ListStyleType::from_component(v).is_ok()),
        ),
        P::ListStylePosition => (
            list_style_position,
            Some(|v, _| ListStylePosition::from_component(v).is_ok()),
        ),
        P::MarginTop => (margin_top, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::MarginRight => (
            margin_right,
            Some(|v, r| Size::from_component(v, r).is_ok()),
        ),
        P::MarginBottom => (
            margin_bottom,
            Some(|v, r| Size::from_component(v, r).is_ok()),
        ),
        P::MarginLeft => (margin_left, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::PaddingTop => (padding_top, Some(|v, r| length_percentage(v, r).is_ok())),
        P::PaddingRight => (padding_right, Some(|v, r| length_percentage(v, r).is_ok())),
        P::PaddingBottom => (padding_bottom, Some(|v, r| length_percentage(v, r).is_ok())),
        P::PaddingLeft => (padding_left, Some(|v, r| length_percentage(v, r).is_ok())),
        P::Top => (top, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::Right => (right, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::Bottom => (bottom, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::Left => (left, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::Width => (width, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::MinWidth => (min_width, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::MaxWidth => (max_width, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::Height => (height, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::MinHeight => (min_height, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::MaxHeight => (max_height, Some(|v, r| Size::from_component(v, r).is_ok())),
        P::BoxSizing => (box_sizing, Some(|_, _| true)),
        P::Display => (
            display,
            Some(|v, _| Display::from_component(v).validate().is_ok()),
        ),
        P::Position => (
            position,
            Some(|v, _| crate::Position::from_component(v).is_ok()),
        ),
        P::Float => (float, Some(|v, _| crate::Float::from_component(v).is_ok())),
        P::Clear => (clear, Some(|v, _| crate::Clear::from_component(v).is_ok())),
        P::ZIndex => (z_index, Some(|v, r| crate::position::z_index(v, r).is_ok())),
        P::Transform => (
            transform,
            Some(|v, r| crate::transform::transform(v, r).is_ok()),
        ),
        P::TransformOrigin => (
            transform_origin,
            Some(|v, r| crate::transform::origin(v, r).is_ok()),
        ),
        P::ObjectFit => (
            object_fit,
            Some(|v, _| crate::ObjectFit::from_component(v).is_ok()),
        ),
        P::Page => (page, Some(|v, _| crate::Page::from_component(v).is_ok())),
        P::ObjectPosition => (
            object_position,
            Some(|v, r| crate::background_values::position(v, r).is_ok()),
        ),
        P::OverflowX => (
            overflow_x,
            Some(|v, _| crate::Overflow::from_component(v).is_ok()),
        ),
        P::OverflowY => (
            overflow_y,
            Some(|v, _| crate::Overflow::from_component(v).is_ok()),
        ),
        P::BreakBefore => (
            break_before,
            Some(|v, _| BreakBetween::from_component(v).is_ok()),
        ),
        P::BreakAfter => (
            break_after,
            Some(|v, _| BreakBetween::from_component(v).is_ok()),
        ),
        P::BreakInside => (
            break_inside,
            Some(|v, _| BreakInside::from_component(v).is_ok()),
        ),
        P::Widows => (widows, Some(|v, r| positive_integer(v, r).is_ok())),
        P::Orphans => (orphans, Some(|v, r| positive_integer(v, r).is_ok())),
        P::BoxDecorationBreak => (box_decoration_break, Some(|_, _| true)),
        P::BoxShadow => (
            box_shadow,
            Some(|v, r| crate::shadow_values::shadows(v, r, ferri_pdf_paint::Color::BLACK).is_ok()),
        ),
        P::Visibility => (
            visibility,
            Some(|v, _| Visibility::from_component(v).is_ok()),
        ),
        P::Opacity => (opacity, Some(|v, r| opacity_value(v, r).is_ok())),
        P::BorderImageSource => (
            border_image_source,
            Some(|v, r| {
                crate::background_values::image(v, r, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::BorderImageSlice => (
            border_image_slice,
            Some(|v, r| crate::border_image_values::slice(v, r).is_ok()),
        ),
        P::BorderImageWidth => (
            border_image_width,
            Some(|v, r| crate::border_image_values::widths(v, r).is_ok()),
        ),
        P::BorderImageOutset => (
            border_image_outset,
            Some(|v, r| crate::border_image_values::outset(v, r).is_ok()),
        ),
        P::BorderImageRepeat => (
            border_image_repeat,
            Some(|v, _| crate::border_image_values::repeat(v).is_ok()),
        ),
        P::TextDecorationLine => (
            text_decoration_line,
            Some(|v, _| crate::DecorationLines::from_component(v).is_ok()),
        ),
        P::TextDecorationStyle => (
            text_decoration_style,
            Some(|v, _| crate::decoration_values::decoration_style(v).is_ok()),
        ),
        P::TextDecorationColor => (
            text_decoration_color,
            Some(|v, relative| {
                crate::colors::color(v, relative, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::TextDecorationThickness => (
            text_decoration_thickness,
            Some(|v, r| crate::decoration_values::thickness(v, r).is_ok()),
        ),
        P::TextUnderlinePosition => (text_underline_position, Some(|_, _| true)),
        P::TextUnderlineOffset => (
            text_underline_offset,
            Some(|v, r| crate::decoration_values::underline_offset(v, r).is_ok()),
        ),
        P::TextDecorationSkipInk => (
            text_decoration_skip_ink,
            Some(|v, _| v.keyword() != Some(Keyword::All)),
        ),
        P::TextOverflow => (
            text_overflow,
            Some(|v, _| crate::TextOverflow::from_component(v).is_ok()),
        ),
        P::BackgroundColor => (
            background_color,
            Some(|v, relative| {
                crate::colors::color(v, relative, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::BackgroundImage => (
            background_image,
            Some(|v, r| {
                crate::background_values::images(v, r, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::BackgroundPosition => (
            background_position,
            Some(|v, r| crate::background_values::positions(v, r).is_ok()),
        ),
        P::BackgroundSize => (
            background_size,
            Some(|v, r| crate::background_values::sizes(v, r).is_ok()),
        ),
        P::BackgroundRepeat => (
            background_repeat,
            Some(|v, _| crate::background_values::repeats(v).is_ok()),
        ),
        P::BackgroundOrigin => (
            background_origin,
            Some(|v, _| crate::background_values::boxes(v).is_ok()),
        ),
        P::BackgroundClip => (
            background_clip,
            Some(|v, _| crate::background_values::boxes(v).is_ok()),
        ),
        P::BackgroundAttachment => (
            background_attachment,
            Some(|v, _| crate::background_values::attachments(v).is_ok()),
        ),
        P::BorderTopWidth => (border_top_width, Some(|v, r| border_width(v, r).is_ok())),
        P::BorderRightWidth => (border_right_width, Some(|v, r| border_width(v, r).is_ok())),
        P::BorderBottomWidth => (border_bottom_width, Some(|v, r| border_width(v, r).is_ok())),
        P::BorderLeftWidth => (border_left_width, Some(|v, r| border_width(v, r).is_ok())),
        P::BorderTopStyle => (border_top_style, Some(|v, _| border_style(v).is_ok())),
        P::BorderRightStyle => (border_right_style, Some(|v, _| border_style(v).is_ok())),
        P::BorderBottomStyle => (border_bottom_style, Some(|v, _| border_style(v).is_ok())),
        P::BorderLeftStyle => (border_left_style, Some(|v, _| border_style(v).is_ok())),
        P::BorderTopColor => (
            border_top_color,
            Some(|v, relative| {
                crate::colors::color(v, relative, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::BorderRightColor => (
            border_right_color,
            Some(|v, relative| {
                crate::colors::color(v, relative, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::BorderBottomColor => (
            border_bottom_color,
            Some(|v, relative| {
                crate::colors::color(v, relative, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::BorderLeftColor => (
            border_left_color,
            Some(|v, relative| {
                crate::colors::color(v, relative, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::BorderTopLeftRadius => (
            border_top_left_radius,
            Some(|v, r| corner_radius(v, r).is_ok()),
        ),
        P::BorderTopRightRadius => (
            border_top_right_radius,
            Some(|v, r| corner_radius(v, r).is_ok()),
        ),
        P::BorderBottomRightRadius => (
            border_bottom_right_radius,
            Some(|v, r| corner_radius(v, r).is_ok()),
        ),
        P::BorderBottomLeftRadius => (
            border_bottom_left_radius,
            Some(|v, r| corner_radius(v, r).is_ok()),
        ),
        P::Content => (
            content,
            Some(|v, _| {
                crate::generated::ComputedContent::from_component(
                    v,
                    crate::generated::ContentContext::BeforeAfter,
                )
                .is_ok_and(|content| content.supported())
            }),
        ),
        P::Quotes => (
            quotes,
            Some(|v, _| crate::Quotes::from_component(v, "", &crate::Quotes::default()).is_ok()),
        ),
        P::FlexDirection => (
            flex_direction,
            Some(|v, _| FlexDirection::from_component(v).is_ok()),
        ),
        P::FlexWrap => (flex_wrap, Some(|v, _| FlexWrap::from_component(v).is_ok())),
        P::FlexGrow => (flex_grow, Some(|v, r| factor(v, r).is_ok())),
        P::FlexShrink => (flex_shrink, Some(|v, r| factor(v, r).is_ok())),
        P::FlexBasis => (
            flex_basis,
            Some(|v, r| FlexBasis::from_component(v, r).is_ok()),
        ),
        P::Order => (order, Some(|v, r| crate::values::integer(v, r).is_ok())),
        P::JustifyContent => (
            justify_content,
            Some(|v, _| Alignment::from_component(v).is_ok()),
        ),
        P::AlignItems => (
            align_items,
            Some(|v, _| Alignment::from_component(v).is_ok()),
        ),
        P::AlignContent => (
            align_content,
            Some(|v, _| Alignment::from_component(v).is_ok()),
        ),
        P::AlignSelf => (
            align_self,
            Some(|v, _| Alignment::from_component(v).is_ok()),
        ),
        P::RowGap => (row_gap, Some(|v, r| gap(v, r).is_ok())),
        P::ColumnGap => (column_gap, Some(|v, r| gap(v, r).is_ok())),
        P::JustifyItems => (justify_items, Some(|v, _| grid_values::justify(v).is_ok())),
        P::JustifySelf => (justify_self, Some(|v, _| grid_values::justify(v).is_ok())),
        P::GridTemplateRows => (
            grid_template_rows,
            Some(|v, r| grid_values::template(v, r).is_ok()),
        ),
        P::GridTemplateColumns => (
            grid_template_columns,
            Some(|v, r| grid_values::template(v, r).is_ok()),
        ),
        P::GridTemplateAreas => (
            grid_template_areas,
            Some(|v, _| grid_values::areas(v).is_ok()),
        ),
        P::GridAutoRows => (
            grid_auto_rows,
            Some(|v, r| grid_values::auto_tracks(v, r).is_ok()),
        ),
        P::GridAutoColumns => (
            grid_auto_columns,
            Some(|v, r| grid_values::auto_tracks(v, r).is_ok()),
        ),
        P::GridAutoFlow => (grid_auto_flow, Some(|_, _| true)),
        P::GridRowStart => (grid_row_start, Some(|v, r| grid_values::line(v, r).is_ok())),
        P::GridRowEnd => (grid_row_end, Some(|v, r| grid_values::line(v, r).is_ok())),
        P::GridColumnStart => (
            grid_column_start,
            Some(|v, r| grid_values::line(v, r).is_ok()),
        ),
        P::GridColumnEnd => (
            grid_column_end,
            Some(|v, r| grid_values::line(v, r).is_ok()),
        ),
        P::ColumnCount => (
            column_count,
            Some(|v, r| crate::multicol_values::count(v, r).is_ok()),
        ),
        P::ColumnWidth => (
            column_width,
            Some(|v, r| crate::multicol_values::width(v, r).is_ok()),
        ),
        P::ColumnFill => (
            column_fill,
            Some(|v, _| crate::ColumnFill::from_component(v).is_ok()),
        ),
        P::ColumnSpan => (column_span, Some(|_, _| true)),
        P::ColumnRuleWidth => (column_rule_width, Some(|v, r| border_width(v, r).is_ok())),
        P::ColumnRuleStyle => (column_rule_style, Some(|v, _| border_style(v).is_ok())),
        P::ColumnRuleColor => (
            column_rule_color,
            Some(|v, relative| {
                crate::colors::color(v, relative, ferri_pdf_paint::Color::BLACK).is_ok()
            }),
        ),
        P::TableLayout => (
            table_layout,
            Some(|v, _| crate::TableLayout::from_component(v).is_ok()),
        ),
        P::BorderCollapse => (
            border_collapse,
            Some(|v, _| crate::BorderCollapse::from_component(v).is_ok()),
        ),
        P::BorderSpacing => (
            border_spacing,
            Some(|v, r| crate::table_values::spacing(v, r).is_ok()),
        ),
        P::EmptyCells => (
            empty_cells,
            Some(|v, _| crate::table_values::empty_cells(v).is_ok()),
        ),
        P::CaptionSide => (
            caption_side,
            Some(|v, _| crate::CaptionSide::from_component(v).is_ok()),
        ),
        P::VerticalAlign => (
            vertical_align,
            Some(|v, r| crate::VerticalAlign::from_component(v, r).is_ok()),
        ),
        P::TableColumnSpan => (
            table_column_span,
            Some(|v, r| crate::table_values::span(v, r).is_ok()),
        ),
        P::TableRowSpan => (
            table_row_span,
            Some(|v, r| crate::table_values::span(v, r).is_ok()),
        ),
        P::BookmarkLevel => (
            bookmark_level,
            Some(|v, r| v.keyword() == Some(Keyword::None) || positive_integer(v, r).is_ok()),
        ),
        P::BookmarkLabel => (
            bookmark_label,
            Some(|v, _| {
                crate::GeneratedContent::from_list(v).is_ok_and(|content| content.supported())
            }),
        ),
        P::BookmarkState => (bookmark_state, Some(|_, _| true)),
        _ => return None,
    };
    Some(Handler { apply, check })
}

/// A handler computing `field` from the specified value, or copying it from the source style.
macro_rules! computed {
    ($name:ident, [$($field:tt)+], |$value:ident, $style:ident, $parent:ident, $relative:ident| $compute:expr) => {
        fn $name(
            $style: &mut Style,
            $parent: &Style,
            input: Input<'_>,
            $relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            let computed = match input {
                Input::Value($value) => $compute,
                Input::Copy(source) => source.$($field)+.to_owned(),
            };
            $style.$($field)+ = computed;
            Ok(())
        }
    };
}

/// A two-keyword property as a flag.
fn flag(value: &Component, choices: &[(Keyword, bool)]) -> Result<bool, StyleError> {
    value
        .keyword()
        .and_then(|keyword| choices.iter().find(|(choice, _)| *choice == keyword))
        .map(|(_, flag)| *flag)
        .ok_or_else(|| unsupported(value))
}

/// `relative` is the parent's: font-relative units in `font-size` measure the parent's font.
fn font_size(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    if let Input::Copy(source) = input {
        style.size = source.size;
        return Ok(());
    }
    style.size = match input {
        Input::Copy(_) => unreachable!("computed copy returned above"),
        Input::Value(value) => font_size_value(value, relative)?,
    };
    Ok(())
}

fn font_size_value(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    let size = match value.keyword() {
        Some(Keyword::Larger) => relative.font.size()? * RELATIVE_FONT_SIZE_RATIO,
        Some(Keyword::Smaller) => relative.font.size()? / RELATIVE_FONT_SIZE_RATIO,
        Some(name) => ABSOLUTE_SIZES
            .iter()
            .find(|(size, _)| *size == name)
            .map(|(_, factor)| factor * crate::DEFAULT_FONT_SIZE)
            .ok_or_else(|| unsupported(value))?,
        None => crate::box_values::font_length_percentage(value, relative)?,
    };
    if !size.is_finite() {
        return Err("non-finite font size".into());
    }
    Ok(size)
}
#[expect(
    clippy::unnecessary_wraps,
    reason = "signature is the fallible Apply handler type"
)]
fn font_family(
    style: &mut Style,
    _parent: &Style,
    input: Input<'_>,
    _relative: &Relative<'_>,
) -> Result<(), StyleError> {
    match input {
        Input::Value(value) => style.families = crate::values::families(value),
        Input::Copy(source) => {
            style.families.clone_from(&source.families);
            style.font_namespace = source.font_namespace;
        }
    }
    Ok(())
}
computed!(
    font_weight,
    [font_weight],
    |value, style, parent, relative| {
        crate::values::font_weight(value, parent.font_weight, relative)?
    }
);
fn font_style(
    style: &mut Style,
    _parent: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    (style.font_style, style.font_slant) = match input {
        Input::Copy(source) => (source.font_style, source.font_slant),
        Input::Value(value) => crate::values::font_style(value, relative)?,
    };
    Ok(())
}
computed!(
    font_width,
    [font_width],
    |value, style, _parent, relative| { crate::values::font_width(value, relative)? }
);
computed!(
    font_synthesis_weight,
    [font_synthesis.bold],
    |value, style, _parent, _relative| flag(
        value,
        &[(Keyword::Auto, true), (Keyword::None, false)]
    )?
);
computed!(
    font_synthesis_style,
    [font_synthesis.oblique],
    |value, style, _parent, _relative| flag(
        value,
        &[(Keyword::Auto, true), (Keyword::None, false)]
    )?
);
computed!(
    font_synthesis_small_caps,
    [font_synthesis.small_caps],
    |value, style, _parent, _relative| flag(
        value,
        &[(Keyword::Auto, true), (Keyword::None, false)]
    )?
);
computed!(
    font_variant_caps,
    [font_variant_caps],
    |value, style, _parent, _relative| crate::FontVariantCaps::from_component(value)?
);
/// Position synthesis has no effect until noninitial font-variant-position is supported.
fn without_variants(
    _: &mut Style,
    _: &Style,
    input: Input<'_>,
    _: &Relative<'_>,
) -> Result<(), StyleError> {
    match input {
        Input::Value(value) => {
            flag(value, &[(Keyword::Auto, true), (Keyword::None, false)]).map(|_| ())
        }
        Input::Copy(_) => Ok(()),
    }
}
computed!(
    line_height,
    [line_height],
    |value, style, _parent, relative| { LineHeight::from_component(value, relative)? }
);
computed!(
    letter_spacing,
    [letter_spacing],
    |value, style, _parent, relative| match value.keyword() {
        Some(Keyword::Normal) => 0.0,
        _ => length_percentage(value, relative)?.resolve(style.size),
    }
);
computed!(
    word_spacing,
    [word_spacing],
    |value, style, _parent, relative| match value.keyword() {
        Some(Keyword::Normal) => LengthPercentage::ZERO,
        _ => length_percentage(value, relative)?,
    }
);
computed!(
    text_transform,
    [text_transform],
    |value, style, _parent, _relative| { TextTransform::from_component(value)? }
);
computed!(
    overflow_wrap,
    [overflow_wrap],
    |value, style, _parent, _relative| { OverflowWrap::from_component(value)? }
);
computed!(
    white_space,
    [white_space],
    |value, style, _parent, _relative| { WhiteSpace::from_component(value)? }
);
computed!(tab_size, [tab_size], |value, style, _parent, relative| {
    tab_size_value(value, relative)?
});

fn tab_size_value(value: &Component, relative: &Relative<'_>) -> Result<usize, StyleError> {
    match value.unwrap() {
        Component::Length(_) | Component::Math(_, Kind::Length, _) => Err(unsupported(value)),
        _ => usize::try_from(crate::values::integer(value, relative)?)
            .map_err(|_| format!("negative tab-size: {value}").into()),
    }
}
computed!(
    unicode_bidi,
    [unicode_bidi],
    |value, style, _parent, _relative| { crate::UnicodeBidi::from_component(value)? }
);
computed!(direction, [rtl], |value, style, _parent, _relative| flag(
    value,
    &[(Keyword::Rtl, true), (Keyword::Ltr, false)]
)?);
computed!(
    text_indent,
    [text_indent],
    |value, style, _parent, relative| { crate::TextIndent::from_component(value, relative)? }
);
computed!(text_align, [align], |value, style, parent, _relative| {
    if value.keyword() == Some(Keyword::MatchParent) {
        match parent.align {
            TextAlign::Start => {
                if parent.rtl {
                    TextAlign::Right
                } else {
                    TextAlign::Left
                }
            }
            TextAlign::End => {
                if parent.rtl {
                    TextAlign::Left
                } else {
                    TextAlign::Right
                }
            }
            value => value,
        }
    } else {
        TextAlign::from_component(value)?
    }
});

computed!(
    text_align_last,
    [align_last],
    |value, style, _parent, _relative| match value.keyword() {
        Some(Keyword::Auto) => None,
        _ => Some(TextAlign::from_component(value)?),
    }
);
computed!(hyphens, [hyphens], |value, style, _parent, _relative| {
    crate::Hyphens::from_component(value)?
});

fn color(
    style: &mut Style,
    parent: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    style.color = match input {
        Input::Value(value) => crate::colors::color(value, relative, parent.color)?,
        Input::Copy(source) => source.color,
    };
    Ok(())
}
computed!(
    list_style_image,
    [list_style_image],
    |value, style, _parent, relative| {
        crate::background_values::image(value, relative, style.color)?
    }
);
computed!(
    list_style_type,
    [list_style_type],
    |value, style, _parent, _relative| { ListStyleType::from_component(value)? }
);
computed!(
    list_style_position,
    [list_style_position],
    |value, style, _parent, _relative| { ListStylePosition::from_component(value)? }
);
computed!(
    margin_top,
    [margin[0]],
    |value, style, _parent, relative| { Size::from_component(value, relative)? }
);
computed!(
    margin_right,
    [margin[1]],
    |value, style, _parent, relative| { Size::from_component(value, relative)? }
);
computed!(
    margin_bottom,
    [margin[2]],
    |value, style, _parent, relative| { Size::from_component(value, relative)? }
);
computed!(
    margin_left,
    [margin[3]],
    |value, style, _parent, relative| { Size::from_component(value, relative)? }
);
computed!(
    padding_top,
    [padding[0]],
    |value, style, _parent, relative| { length_percentage(value, relative)? }
);
computed!(
    padding_right,
    [padding[1]],
    |value, style, _parent, relative| { length_percentage(value, relative)? }
);
computed!(
    padding_bottom,
    [padding[2]],
    |value, style, _parent, relative| { length_percentage(value, relative)? }
);
computed!(
    padding_left,
    [padding[3]],
    |value, style, _parent, relative| { length_percentage(value, relative)? }
);
computed!(top, [inset[0]], |value, style, _parent, relative| {
    Size::from_component(value, relative)?
});
computed!(right, [inset[1]], |value, style, _parent, relative| {
    Size::from_component(value, relative)?
});
computed!(bottom, [inset[2]], |value, style, _parent, relative| {
    Size::from_component(value, relative)?
});
computed!(left, [inset[3]], |value, style, _parent, relative| {
    Size::from_component(value, relative)?
});
computed!(width, [width], |value, style, _parent, relative| {
    Size::from_component(value, relative)?
});
computed!(min_width, [min_width], |value, style, _parent, relative| {
    Size::from_component(value, relative)?
});
computed!(max_width, [max_width], |value, style, _parent, relative| {
    Size::from_component(value, relative)?
});
computed!(height, [height], |value, style, _parent, relative| {
    Size::from_component(value, relative)?
});
computed!(
    min_height,
    [min_height],
    |value, style, _parent, relative| { Size::from_component(value, relative)? }
);
computed!(
    max_height,
    [max_height],
    |value, style, _parent, relative| { Size::from_component(value, relative)? }
);
computed!(
    box_sizing,
    [box_sizing],
    |value, style, _parent, _relative| if flag(
        value,
        &[(Keyword::BorderBox, true), (Keyword::ContentBox, false)]
    )? {
        BoxSizing::BorderBox
    } else {
        BoxSizing::ContentBox
    }
);
computed!(display, [display], |value, style, _parent, _relative| {
    Display::from_component(value)
});
computed!(position, [position], |value, style, _parent, _relative| {
    crate::Position::from_component(value)?
});
computed!(z_index, [z_index], |value, style, _parent, relative| {
    crate::position::z_index(value, relative)?
});
computed!(transform, [transform], |value, style, _parent, relative| {
    crate::transform::transform(value, relative)?
});
computed!(
    transform_origin,
    [transform_origin],
    |value, style, _parent, relative| { crate::transform::origin(value, relative)? }
);
computed!(
    object_fit,
    [object_fit],
    |value, style, _parent, _relative| { crate::ObjectFit::from_component(value)? }
);
computed!(page, [page], |value, style, _parent, _relative| {
    crate::Page::from_component(value)?
});
computed!(
    object_position,
    [object_position],
    |value, style, _parent, relative| { crate::background_values::position(value, relative)? }
);
computed!(
    overflow_x,
    [overflow[0]],
    |value, style, _parent, _relative| { crate::Overflow::from_component(value)? }
);
computed!(
    overflow_y,
    [overflow[1]],
    |value, style, _parent, _relative| { crate::Overflow::from_component(value)? }
);
computed!(
    break_before,
    [break_before],
    |value, style, _parent, _relative| { BreakBetween::from_component(value)? }
);
computed!(
    break_after,
    [break_after],
    |value, style, _parent, _relative| { BreakBetween::from_component(value)? }
);
computed!(break_inside, [keep], |value, style, _parent, _relative| {
    BreakInside::from_component(value)?
});
computed!(float, [float], |value, style, _parent, _relative| {
    crate::Float::from_component(value)?
});
computed!(clear, [clear], |value, style, _parent, _relative| {
    crate::Clear::from_component(value)?
});
computed!(widows, [widows], |value, style, _parent, relative| {
    positive_integer(value, relative)?
});
computed!(orphans, [orphans], |value, style, _parent, relative| {
    positive_integer(value, relative)?
});
/// `widows` and `orphans`; CSS Fragmentation 3 §4.4 makes zero and negatives invalid.
fn positive_integer(value: &Component, relative: &Relative<'_>) -> Result<usize, StyleError> {
    usize::try_from(crate::values::integer(value, relative)?)
        .ok()
        .filter(|count| *count > 0)
        .ok_or_else(|| format!("invalid positive integer: {value}").into())
}
computed!(
    box_decoration_break,
    [clone_decoration],
    |value, style, _parent, _relative| flag(
        value,
        &[(Keyword::Clone, true), (Keyword::Slice, false)]
    )?
);
computed!(
    visibility,
    [visibility],
    |value, style, _parent, _relative| { Visibility::from_component(value)? }
);
// `<opacity-value>`: a number or percentage, clamped to `0..=1` (CSS Color 4 §15).
computed!(opacity, [opacity], |value, style, _parent, relative| {
    opacity_value(value, relative)?
});

fn opacity_value(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    Ok(match value.unwrap() {
        Component::Percentage(fraction) => *fraction,
        Component::Math(source, Kind::Percentage, range) => {
            crate::values::math(source, relative, Kind::Percentage, Some(Kind::Percentage))?
                .resolve(0.0, *range)
        }
        _ => crate::values::number(value, relative)?,
    }
    .clamp(0.0, 1.0))
}
computed!(
    box_shadow,
    [decoration.shadows],
    |value, style, _parent, relative| {
        std::sync::Arc::new(crate::shadow_values::shadows(value, relative, style.color)?)
    }
);
computed!(
    text_decoration_line,
    [text_decorations.declared.lines],
    |value, style, _parent, _relative| crate::DecorationLines::from_component(value)?
);
computed!(
    text_decoration_style,
    [text_decorations.declared.style],
    |value, style, _parent, _relative| crate::decoration_values::decoration_style(value)?
);
computed!(
    text_decoration_color,
    [text_decorations.declared.color],
    |value, style, _parent, relative| crate::colors::color(value, relative, style.color)?
);
computed!(
    text_decoration_thickness,
    [text_decorations.declared.thickness],
    |value, style, _parent, relative| crate::decoration_values::thickness(value, relative)?
);
computed!(
    text_underline_position,
    [text_decorations.underline_position],
    |value, style, _parent, _relative| crate::UnderlinePosition::from_component(value)
);
computed!(
    text_underline_offset,
    [text_decorations.underline_offset],
    |value, style, _parent, relative| crate::decoration_values::underline_offset(value, relative)?
);
/// Glyph ink is never skipped, which `auto` permits; `all` requires skipping.
fn text_decoration_skip_ink(
    _: &mut Style,
    _: &Style,
    input: Input<'_>,
    _: &Relative<'_>,
) -> Result<(), StyleError> {
    match input {
        Input::Value(value) if value.keyword() == Some(Keyword::All) => Err(unsupported(value)),
        _ => Ok(()),
    }
}
computed!(
    text_overflow,
    [text_overflow],
    |value, style, _parent, _relative| { crate::TextOverflow::from_component(value)? }
);
computed!(
    background_color,
    [decoration.background],
    |value, style, _parent, relative| { crate::colors::color(value, relative, style.color)? }
);

/// A `background-*` list longhand, shared copy-on-write between styles.
macro_rules! background {
    ($name:ident, $field:ident, |$value:ident, $style:ident, $relative:ident| $compute:expr) => {
        fn $name(
            $style: &mut Style,
            _: &Style,
            input: Input<'_>,
            $relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            let computed = match input {
                Input::Value($value) => $compute,
                Input::Copy(source) => source.decoration.backgrounds.$field.clone(),
            };
            $style.backgrounds_mut().$field = computed;
            Ok(())
        }
    };
}
background!(background_image, images, |value, style, relative| {
    crate::background_values::images(value, relative, style.color)?
});
background!(background_position, positions, |value, style, relative| {
    crate::background_values::positions(value, relative)?
});
background!(background_size, sizes, |value, style, relative| {
    crate::background_values::sizes(value, relative)?
});
background!(background_repeat, repeats, |value, style, _relative| {
    crate::background_values::repeats(value)?
});
background!(background_origin, origins, |value, style, _relative| {
    crate::background_values::boxes(value)?
});
background!(background_clip, clips, |value, style, _relative| {
    crate::background_values::boxes(value)?
});
background!(
    background_attachment,
    attachments,
    |value, style, _relative| { crate::background_values::attachments(value)? }
);

/// A `border-image-*` longhand, shared copy-on-write between styles.
macro_rules! border_image {
    ($name:ident, [$($field:ident),+], |$value:ident, $style:ident, $relative:ident| $compute:expr) => {
        fn $name(
            $style: &mut Style,
            _: &Style,
            input: Input<'_>,
            $relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            let computed = match input {
                Input::Value($value) => $compute,
                Input::Copy(source) => {
                    let image = &source.decoration.border_image;
                    ($(image.$field.clone()),+)
                }
            };
            let image = std::sync::Arc::make_mut(&mut $style.decoration.border_image);
            ($(image.$field),+) = computed;
            Ok(())
        }
    };
}
border_image!(border_image_source, [source], |value, style, relative| {
    crate::background_values::image(value, relative, style.color)?
});
border_image!(
    border_image_slice,
    [slice, fill],
    |value, style, relative| { crate::border_image_values::slice(value, relative)? }
);
border_image!(border_image_width, [widths], |value, style, relative| {
    crate::border_image_values::widths(value, relative)?
});
border_image!(border_image_outset, [outset], |value, style, relative| {
    crate::border_image_values::outset(value, relative)?
});
border_image!(border_image_repeat, [repeat], |value, style, _relative| {
    crate::border_image_values::repeat(value)?
});

/// `<line-width>` converted to absolute points.
fn border_width(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    let width = match value.keyword() {
        Some(Keyword::Thin) => THIN_BORDER_PT,
        Some(Keyword::Medium) => DEFAULT_BORDER_WIDTH,
        Some(Keyword::Thick) => THICK_BORDER_PT,
        _ => crate::values::length(value, relative)?,
    };
    if width < 0.0 {
        return Err("negative border width".into());
    }
    Ok(width)
}
fn border_style(value: &Component) -> Result<BorderStyle, StyleError> {
    match value.keyword() {
        Some(Keyword::None) => Ok(BorderStyle::None),
        Some(Keyword::Hidden) => Ok(BorderStyle::Hidden),
        Some(Keyword::Solid) => Ok(BorderStyle::Solid),
        Some(Keyword::Dotted) => Ok(BorderStyle::Dotted),
        Some(Keyword::Dashed) => Ok(BorderStyle::Dashed),
        Some(Keyword::Double) => Ok(BorderStyle::Double),
        Some(Keyword::Groove) => Ok(BorderStyle::Groove),
        Some(Keyword::Ridge) => Ok(BorderStyle::Ridge),
        Some(Keyword::Inset) => Ok(BorderStyle::Inset),
        Some(Keyword::Outset) => Ok(BorderStyle::Outset),
        _ => Err(StyleError::Unsupported(format!("border style {value}"))),
    }
}
macro_rules! border_side {
    ($width:ident, $style_name:ident, $color:ident, $side:literal) => {
        fn $width(
            style: &mut Style,
            _: &Style,
            input: Input<'_>,
            relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            if style.decoration.borders[$side].style.suppresses_width() {
                style.decoration.borders[$side].width = 0.0;
                return Ok(());
            }
            style.decoration.borders[$side].width = match input {
                Input::Value(value) => border_width(value, relative)?,
                // Computed width is zero where the source's style is none or hidden.
                Input::Copy(source) => source.decoration.borders[$side].used_width(),
            };
            Ok(())
        }
        computed!(
            $style_name,
            [decoration.borders[$side].style],
            |value, style, _parent, _relative| { border_style(value)? }
        );
        computed!(
            $color,
            [decoration.borders[$side].color],
            |value, style, _parent, relative| {
                crate::colors::color(value, relative, style.color)?
            }
        );
    };
}
border_side!(border_top_width, border_top_style, border_top_color, 0);
border_side!(
    border_right_width,
    border_right_style,
    border_right_color,
    1
);
border_side!(
    border_bottom_width,
    border_bottom_style,
    border_bottom_color,
    2
);
border_side!(border_left_width, border_left_style, border_left_color, 3);

/// `<length-percentage [0,∞]>{1,2}`: horizontal then vertical radius.
fn corner_radius(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<[LengthPercentage; 2], StyleError> {
    let mut radii = value
        .items()
        .iter()
        .map(|radius| length_percentage(radius, relative))
        .collect::<Result<Vec<_>, _>>()?;
    match radii.len() {
        1 => Ok([radii[0].clone(), radii.remove(0)]),
        2 => Ok([radii.remove(0), radii.remove(0)]),
        _ => Err(unsupported(value)),
    }
}
macro_rules! border_radius {
    ($name:ident, $corner:literal) => {
        fn $name(
            style: &mut Style,
            _: &Style,
            input: Input<'_>,
            relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            let radius = match input {
                Input::Value(value) => corner_radius(value, relative)?,
                Input::Copy(source) => source.decoration.radii[$corner].clone(),
            };
            std::sync::Arc::make_mut(&mut style.decoration.radii)[$corner] = radius;
            Ok(())
        }
    };
}
border_radius!(border_top_left_radius, 0);
border_radius!(border_top_right_radius, 1);
border_radius!(border_bottom_right_radius, 2);
border_radius!(border_bottom_left_radius, 3);

/// Pseudo-elements and margin boxes take `content` before other properties; elements keep
/// their own content, which `normal` and `none` describe.
fn content(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    _: &Relative<'_>,
) -> Result<(), StyleError> {
    style.computed_content = match input {
        Input::Value(value) => {
            crate::generated::ComputedContent::from_component(value, style.content_context)?
        }
        Input::Copy(source) => source.computed_content.clone(),
    };
    Ok(())
}

computed!(
    flex_direction,
    [flex.direction],
    |value, style, _parent, _relative| { FlexDirection::from_component(value)? }
);
computed!(
    flex_wrap,
    [flex.wrap],
    |value, style, _parent, _relative| { FlexWrap::from_component(value)? }
);
computed!(flex_grow, [flex.grow], |value, style, _parent, relative| {
    factor(value, relative)?
});
computed!(
    flex_shrink,
    [flex.shrink],
    |value, style, _parent, relative| factor(value, relative)?
);
computed!(
    flex_basis,
    [flex.basis],
    |value, style, _parent, relative| { FlexBasis::from_component(value, relative)? }
);
computed!(order, [flex.order], |value, style, _parent, relative| {
    crate::values::integer(value, relative)?
});
computed!(
    justify_content,
    [flex.justify],
    |value, style, _parent, _relative| { Alignment::from_component(value)? }
);
computed!(
    align_items,
    [flex.align_items],
    |value, style, _parent, _relative| { Alignment::from_component(value)? }
);
computed!(
    align_content,
    [flex.align_content],
    |value, style, _parent, _relative| { Alignment::from_component(value)? }
);
computed!(
    align_self,
    [flex.align_self],
    |value, style, _parent, _relative| { Alignment::from_component(value)? }
);
computed!(row_gap, [flex.gap[0]], |value, style, _parent, relative| {
    gap(value, relative)?
});
computed!(
    column_gap,
    [flex.gap[1]],
    |value, style, _parent, relative| gap(value, relative)?
);
fn justify_items(
    style: &mut Style,
    parent: &Style,
    input: Input<'_>,
    _relative: &Relative<'_>,
) -> Result<(), StyleError> {
    let source = match input {
        Input::Copy(source) => &source.grid,
        Input::Value(value) => {
            if let Some((alignment, legacy)) = grid_values::justify_items(value)? {
                (style.grid.justify_items, style.grid.legacy_justify) = (alignment, legacy);
                return Ok(());
            }
            style.grid.initial_justify_items(&parent.grid);
            return Ok(());
        }
    };
    (style.grid.justify_items, style.grid.legacy_justify) =
        (source.justify_items, source.legacy_justify);
    Ok(())
}
computed!(
    justify_self,
    [grid.justify_self],
    |value, style, _parent, _relative| { grid_values::justify(value)? }
);
computed!(
    grid_template_rows,
    [grid.templates[crate::ROWS]],
    |value, style, _parent, relative| std::sync::Arc::new(grid_values::template(value, relative)?)
);
computed!(
    grid_template_columns,
    [grid.templates[crate::COLUMNS]],
    |value, style, _parent, relative| std::sync::Arc::new(grid_values::template(value, relative)?)
);
computed!(
    grid_template_areas,
    [grid.areas],
    |value, style, _parent, _relative| { grid_values::areas(value)? }
);
computed!(
    grid_auto_rows,
    [grid.auto_tracks[crate::ROWS]],
    |value, style, _parent, relative| grid_values::auto_tracks(value, relative)?
);
computed!(
    grid_auto_columns,
    [grid.auto_tracks[crate::COLUMNS]],
    |value, style, _parent, relative| grid_values::auto_tracks(value, relative)?
);
computed!(
    grid_auto_flow,
    [grid.auto_flow],
    |value, style, _parent, _relative| { grid_values::auto_flow(value) }
);
computed!(
    grid_row_start,
    [grid.placement[crate::ROWS][0]],
    |value, style, _parent, relative| grid_values::line(value, relative)?
);
computed!(
    grid_row_end,
    [grid.placement[crate::ROWS][1]],
    |value, style, _parent, relative| grid_values::line(value, relative)?
);
computed!(
    grid_column_start,
    [grid.placement[crate::COLUMNS][0]],
    |value, style, _parent, relative| grid_values::line(value, relative)?
);
computed!(
    grid_column_end,
    [grid.placement[crate::COLUMNS][1]],
    |value, style, _parent, relative| grid_values::line(value, relative)?
);
computed!(
    column_count,
    [multicol.count],
    |value, style, _parent, relative| { crate::multicol_values::count(value, relative)? }
);
computed!(
    column_width,
    [multicol.width],
    |value, style, _parent, relative| { crate::multicol_values::width(value, relative)? }
);
computed!(
    column_fill,
    [multicol.fill],
    |value, style, _parent, _relative| { crate::ColumnFill::from_component(value)? }
);
computed!(
    column_span,
    [multicol.span_all],
    |value, style, _parent, _relative| flag(
        value,
        &[(Keyword::All, true), (Keyword::None, false)]
    )?
);
fn column_rule_width(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    style.multicol.rule.width = match input {
        Input::Value(value) => border_width(value, relative)?,
        // Computed width is zero where the source's rule style is none or hidden.
        Input::Copy(source) => source.multicol.rule.used_width(),
    };
    Ok(())
}
computed!(
    column_rule_style,
    [multicol.rule.style],
    |value, style, _parent, _relative| { border_style(value)? }
);
computed!(
    column_rule_color,
    [multicol.rule.color],
    |value, style, _parent, relative| { crate::colors::color(value, relative, style.color)? }
);

computed!(
    table_layout,
    [table.layout],
    |value, style, _parent, _relative| { crate::TableLayout::from_component(value)? }
);
computed!(
    border_collapse,
    [table.collapse],
    |value, style, _parent, _relative| { crate::BorderCollapse::from_component(value)? }
);
computed!(
    border_spacing,
    [table.spacing],
    |value, style, _parent, relative| { crate::table_values::spacing(value, relative)? }
);
computed!(
    empty_cells,
    [table.empty_cells],
    |value, style, _parent, _relative| { crate::table_values::empty_cells(value)? }
);
computed!(
    caption_side,
    [table.caption_side],
    |value, style, _parent, _relative| { crate::CaptionSide::from_component(value)? }
);
computed!(
    vertical_align,
    [table.vertical_align],
    |value, style, _parent, relative| { crate::VerticalAlign::from_component(value, relative)? }
);
computed!(
    table_column_span,
    [table.column_span],
    |value, style, _parent, relative| { crate::table_values::span(value, relative)? }
);
computed!(
    table_row_span,
    [table.row_span],
    |value, style, _parent, relative| { crate::table_values::span(value, relative)? }
);

computed!(
    bookmark_level,
    [bookmark.level],
    |value, style, _parent, relative| match value.keyword() {
        Some(Keyword::None) => None,
        _ => Some(positive_integer(value, relative)?),
    }
);
computed!(
    bookmark_label,
    [bookmark.label],
    |value, style, _parent, _relative| { crate::GeneratedContent::from_list(value)? }
);
computed!(
    bookmark_state,
    [bookmark.open],
    |value, style, _parent, _relative| {
        flag(value, &[(Keyword::Open, true), (Keyword::Closed, false)])?
    }
);

computed!(
    counter_reset,
    [counters.reset],
    |value, style, _parent, relative| { crate::counter_values::resets(value, relative)? }
);

computed!(
    counter_increment,
    [counters.increment],
    |value, style, _parent, relative| {
        crate::counter_values::changes(value, relative, crate::DEFAULT_COUNTER_INCREMENT)?
    }
);

computed!(
    counter_set,
    [counters.set],
    |value, style, _parent, relative| {
        crate::counter_values::changes(value, relative, crate::INITIAL_COUNTER_VALUE)?
    }
);

computed!(
    string_set,
    [string_set],
    |value, style, _parent, _relative| { crate::string_values::assignments(value)? }
);
