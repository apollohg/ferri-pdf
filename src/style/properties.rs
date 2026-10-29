//! The registry of supported longhands: computed values from match trees. Every other
//! property is rejected here unless it keeps its initial value, which the engine renders.
use crate::fonts::FontNamespace;
use crate::paint::{BorderStyle, DEFAULT_BORDER_WIDTH, LengthPercentage, OutlineStyle};
use crate::style::{
    BoxSizing, BreakBetween, BreakInside, Display, HangingPunctuation, HyphenateLimitChars,
    LineBreak, ListStylePosition, ListStyleType, Style, StyleError, TextAlign, TextAutospace,
    TextJustify, TextSpacingTrim, TextWrapStyle, Visibility, WhiteSpaceCollapse, WordBreak,
    anchor_values::{self, AnchorTarget, HORIZONTAL, VERTICAL},
    box_values::{length_percentage, unsupported},
    calc::Kind,
    component::Component,
    declaration::{self, Specified},
    flex_values::{Alignment, FlexBasis, FlexDirection, FlexWrap, factor, gap},
    grammar::{Keyword, Property},
    grid_values, inert,
    logical::{BOTTOM, LEFT, RIGHT, TOP},
    svg_values,
    text_values::{self, LineHeight, OverflowWrap, TextTransform},
    units::Relative,
    wide_keywords::Resolved,
};

/// Between a property name and its value in unsupported-value errors.
pub(super) const DECLARATION_SEPARATOR: &str = ": ";
const RELATIVE_FONT_SIZE_RATIO: f32 = 6.0 / 5.0;
const THIN_BORDER_PT: f32 = 0.5625;
const THICK_BORDER_PT: f32 = 2.8125;
/// Axis indices of the `background-position-x` and `-y` lists.
const POSITION_X: usize = 0;
const POSITION_Y: usize = 1;
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

pub(super) type Apply = fn(&mut Style, &Style, Input<'_>, &Relative<'_>) -> Result<(), StyleError>;
type Check = fn(&Component, &Relative<'_>) -> bool;

struct Handler {
    apply: Apply,
    check: Option<Check>,
}

/// Counter style names resolve from the element's tree scope (CSS Counter Styles 3 §5).
fn element_scope<'a>(
    relative: &'a Relative<'_>,
    style: &Style,
) -> crate::style::counter_style::Scope<'a> {
    crate::style::counter_style::Scope::of(relative, style.element_font_namespace())
}
fn validation_scope<'a>(relative: &'a Relative<'_>) -> crate::style::counter_style::Scope<'a> {
    crate::style::counter_style::Scope::of(relative, FontNamespace::ROOT)
}

/// Flow-relative properties share their physical counterparts' grammar and computation.
fn computing(property: Property) -> Property {
    crate::style::logical::physical(property, crate::style::logical::Flow::default())
}

pub(super) fn supports_property(property: Property) -> bool {
    inert::defined(computing(property))
        || handler(computing(property)).is_some_and(|handler| handler.check.is_some())
}

pub(super) fn supports_value(
    property: Property,
    value: &Component,
    relative: &Relative<'_>,
) -> bool {
    if inert::defined(computing(property)) {
        return inert::compute(computing(property), value, relative).is_ok();
    }
    handler(computing(property))
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
    let protected_relative = if (style.anchor.protected.contains(&property)
        || style.anchor.namespaces.contains_key(&property))
        && relative.anchoring.is_some()
    {
        let mut protected = relative.clone();
        if let Some(anchoring) = &mut protected.anchoring {
            if style.anchor.protected.contains(&property) {
                anchoring.tactic = anchor_values::Tactic::IDENTITY;
            }
            anchoring.scope = style.anchor.namespaces.get(&property).copied();
        }
        Some(protected)
    } else {
        None
    };
    let relative = protected_relative.as_ref().unwrap_or(relative);
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
    if let Specified::FlowRelative(flow, value) = specified {
        let parent = crate::style::logical::reflowed(parent, *flow);
        return apply_value(property, value, style, &parent, relative, choices);
    }
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
    let inherited_parent;
    let input = match specified {
        Specified::Value(value) => Input::Value(value),
        Specified::Toggle(_) | Specified::FlowRelative(..) => {
            unreachable!("toggle and flow-relative values were computed above")
        }
        Specified::Unexpanded(shorthand, value) => {
            return Err(StyleError::Unsupported(format!(
                "{}{DECLARATION_SEPARATOR}{value}",
                shorthand.name()
            )));
        }
        Specified::Wide(keyword) => match keyword.resolve(property.inherited()) {
            Resolved::Inherit => {
                if let Some(tactic) = relative
                    .anchoring
                    .as_ref()
                    .map(|anchoring| anchoring.tactic)
                    .filter(|tactic| tactic.source_property(property) != property)
                {
                    inherited_parent = tactic.inherited_basis(parent);
                    Input::Copy(&inherited_parent)
                } else {
                    Input::Copy(parent)
                }
            }
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
        _ if inert::defined(property) => inert::apply(property, style, input, relative),
        Some(handler) => (handler.apply)(style, parent, input, relative),
        None => match input {
            Input::Copy(_) => Ok(()),
            Input::Value(value) if declaration::initial(property) == Some(value) => Ok(()),
            Input::Value(_) => Err(unsupported_property(property, input)),
        },
    };
    result.map_err(|error| attributed(property, input, error))?;
    if property == Property::Display {
        if style.display == Display::RubyText
            && parent.ruby.position == crate::style::RubyPosition::InterCharacter
            && !parent.used_writing_mode().vertical()
        {
            style.writing_mode = crate::style::logical::WritingMode::VerticalRl;
        }
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
            .map(crate::paint::PositionComponent::canonical);
    }
    if property == Property::PerspectiveOrigin {
        value.perspective_origin = value
            .perspective_origin
            .each_ref()
            .map(crate::paint::PositionComponent::canonical);
    }
    if let Some(axis) = [Property::BackgroundPositionX, Property::BackgroundPositionY]
        .iter()
        .position(|axis| *axis == property)
    {
        for position in
            &mut std::sync::Arc::make_mut(&mut value.decoration.backgrounds).positions[axis]
        {
            *position = position.canonical();
        }
    }
    if property == Property::FontFamily
        && !source
            .families
            .iter()
            .any(|family| matches!(family, crate::fonts::FontFamily::Named(_)))
    {
        value.font_namespace = Default::default();
    }
    if property == Property::Quotes {
        value.quotes = source.quotes.clone();
    }
    value
}

/// Unsupported values report the declaration they came from, once: errors that already name
/// a property, such as those of a property this one depends on, pass through. A reason other
/// than the unsupported part of the value follows the declaration.
fn attributed(property: Property, input: Input<'_>, error: StyleError) -> StyleError {
    let StyleError::Unsupported(detail) = error else {
        return error;
    };
    if detail
        .split_once(DECLARATION_SEPARATOR)
        .is_some_and(|(name, _)| Property::from_name(name).is_some())
    {
        return StyleError::Unsupported(detail);
    }
    match (unsupported_property(property, input), input) {
        (StyleError::Unsupported(declaration), Input::Value(value))
            if !value.to_string().contains(&detail) =>
        {
            StyleError::Unsupported(format!("{declaration} ({detail})"))
        }
        (declaration, _) => declaration,
    }
}

fn unsupported_property(property: Property, input: Input<'_>) -> StyleError {
    match input {
        Input::Value(value) => {
            StyleError::Unsupported(format!("{}{DECLARATION_SEPARATOR}{value}", property.name()))
        }
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
        Input::Value(value) => crate::style::Quotes::from_component(
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
    if inert::defined(property) {
        inert::copy(property, target, source);
    } else if let Some(handler) = handler(property) {
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
    inert::defined(computing(property)) || handler(computing(property)).is_some()
}

fn handler(property: Property) -> Option<Handler> {
    use Property as P;
    let (apply, check): (Apply, Option<Check>) = match property {
        P::StringSet => (
            string_set,
            Some(|value, relative| {
                crate::style::string_values::assignments(value, &validation_scope(relative)).is_ok()
            }),
        ),
        P::CounterReset => (
            counter_reset,
            Some(|v, r| crate::style::counter_values::resets(v, r).is_ok()),
        ),
        P::CounterIncrement => (
            counter_increment,
            Some(|v, r| {
                crate::style::counter_values::changes(v, r, crate::style::DEFAULT_COUNTER_INCREMENT)
                    .is_ok()
            }),
        ),
        P::CounterSet => (
            counter_set,
            Some(|v, r| {
                crate::style::counter_values::changes(v, r, crate::style::INITIAL_COUNTER_VALUE)
                    .is_ok()
            }),
        ),
        P::MathStyle => (math_style, Some(|v, _| math_compact(v).is_ok())),
        P::MathShift => (math_shift, Some(|v, _| math_compact(v).is_ok())),
        P::MathDepth => (
            math_depth,
            Some(|v, r| crate::style::math_values::depth(v, &Style::default(), r).is_ok()),
        ),
        P::FontSize => (
            font_size,
            Some(|v, r| v.keyword() == Some(Keyword::Math) || font_size_value(v, r).is_ok()),
        ),
        P::FontFamily => (font_family, Some(|_, _| true)),
        P::FontWeight => (
            font_weight,
            Some(|v, r| {
                crate::style::values::font_weight(v, crate::fonts::FontWeight::NORMAL, r).is_ok()
            }),
        ),
        P::FontWidth => (
            font_width,
            Some(|v, r| crate::style::values::font_width(v, r).is_ok()),
        ),
        P::FontStyle => (
            font_style,
            Some(|v, r| crate::style::values::font_style(v, r).is_ok()),
        ),
        P::FontSynthesisWeight => (font_synthesis_weight, Some(|_, _| true)),
        P::FontSynthesisStyle => (font_synthesis_style, Some(|_, _| true)),
        P::FontSynthesisSmallCaps => (font_synthesis_small_caps, Some(|_, _| true)),
        P::FontSynthesisPosition => (font_synthesis_position, Some(|_, _| true)),
        P::FontVariantCaps => (
            font_variant_caps,
            Some(|v, _| crate::style::FontVariantCaps::from_component(v).is_ok()),
        ),
        P::FontKerning => (font_kerning, Some(|_, _| true)),
        P::FontVariantLigatures => (font_variant_ligatures, Some(|_, _| true)),
        P::FontVariantPosition => (font_variant_position, Some(|_, _| true)),
        P::FontVariantNumeric => (font_variant_numeric, Some(|_, _| true)),
        P::FontVariantAlternates => (font_variant_alternates, Some(|_, _| true)),
        P::FontVariantEastAsian => (font_variant_east_asian, Some(|_, _| true)),
        P::FontVariantEmoji => (font_variant_emoji, Some(|_, _| true)),
        P::FontFeatureSettings => (font_feature_settings, Some(|_, _| true)),
        P::FontLanguageOverride => (font_language_override, Some(|_, _| true)),
        P::FontPalette => (
            font_palette,
            Some(|v, r| crate::style::font_values::palette(v, r).is_ok()),
        ),
        P::FontOpticalSizing => (font_optical_sizing, Some(|_, _| true)),
        P::FontVariationSettings => (
            font_variation_settings,
            Some(|v, r| crate::style::font_values::variation_settings(v, r).is_ok()),
        ),
        P::FontSizeAdjust => (
            font_size_adjust,
            Some(|v, r| crate::style::font_values::size_adjust(v, r).is_ok()),
        ),
        P::LineHeightStep => (
            line_height_step,
            Some(|v, r| line_height_step_value(v, r).is_ok()),
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
        P::WhiteSpaceCollapse => (
            white_space_collapse,
            Some(|v, _| WhiteSpaceCollapse::from_component(v).is_ok()),
        ),
        P::TextWrapMode => (text_wrap_mode, Some(|_, _| true)),
        P::TextWrapStyle => (
            text_wrap_style,
            Some(|v, _| TextWrapStyle::from_component(v).is_ok()),
        ),
        P::WordBreak => (
            word_break,
            Some(|v, _| WordBreak::from_component(v).is_ok()),
        ),
        P::LineBreak => (
            line_break,
            Some(|v, _| LineBreak::from_component(v).is_ok()),
        ),
        P::HangingPunctuation => (
            hanging_punctuation,
            Some(|v, _| HangingPunctuation::from_component(v).is_ok()),
        ),
        P::TextJustify => (
            text_justify,
            Some(|v, _| TextJustify::from_component(v).is_ok()),
        ),
        P::TextAutospace => (
            text_autospace,
            Some(|v, _| TextAutospace::from_component(v).is_ok()),
        ),
        P::TextSpacingTrim => (
            text_spacing_trim,
            Some(|v, _| TextSpacingTrim::from_component(v).is_ok()),
        ),
        P::HyphenateCharacter => (
            hyphenate_character,
            Some(|v, _| text_values::hyphenate_character(v).is_ok()),
        ),
        P::HyphenateLimitChars => (
            hyphenate_limit_chars,
            Some(|v, r| HyphenateLimitChars::from_component(v, r).is_ok()),
        ),
        P::MaxLines => (max_lines, Some(|v, r| max_lines_value(v, r).is_ok())),
        P::BlockEllipsis => (
            block_ellipsis,
            Some(|v, _| crate::style::BlockEllipsis::from_component(v).is_ok()),
        ),
        P::Continue => (
            continuation,
            Some(|v, _| crate::style::Continue::from_component(v).is_ok()),
        ),
        P::TabSize => (tab_size, Some(|v, r| tab_size_value(v, r).is_ok())),
        P::TextCombineUpright => (
            text_combine,
            Some(|v, r| crate::style::logical::TextCombine::from_component(v, r).is_ok()),
        ),
        P::TextOrientation => (
            text_orientation,
            Some(|v, _| crate::style::logical::TextOrientation::from_component(v).is_ok()),
        ),
        P::WritingMode => (
            writing_mode,
            Some(|v, _| crate::style::logical::WritingMode::from_component(v).is_ok()),
        ),
        P::Direction => (direction, Some(|_, _| true)),
        P::UnicodeBidi => (
            unicode_bidi,
            Some(|v, _| crate::style::UnicodeBidi::from_component(v).is_ok()),
        ),
        P::TextIndent => (
            text_indent,
            Some(|v, r| crate::style::TextIndent::from_component(v, r).is_ok()),
        ),
        P::TextAlignAll => (
            text_align_all,
            Some(|v, _| {
                v.keyword() == Some(Keyword::MatchParent) || TextAlign::from_component(v).is_ok()
            }),
        ),
        P::TextAlignLast => (
            text_align_last,
            Some(|v, _| {
                matches!(v.keyword(), Some(Keyword::Auto | Keyword::MatchParent))
                    || TextAlign::from_component(v).is_ok()
            }),
        ),
        P::Hyphens => (
            hyphens,
            Some(|v, _| crate::style::Hyphens::from_component(v).is_ok()),
        ),
        P::ColorScheme => (color_scheme, Some(|_, _| true)),
        P::Appearance => (
            appearance,
            Some(|v, _| crate::style::Appearance::from_component(v).is_ok()),
        ),
        P::FieldSizing => (
            field_sizing,
            Some(|v, _| crate::style::FieldSizing::from_component(v).is_ok()),
        ),
        P::AccentColor => (
            accent_color,
            Some(|v, relative| {
                v.keyword() == Some(Keyword::Auto)
                    || crate::style::colors::color(v, relative, &crate::paint::AbsoluteColor::BLACK)
                        .is_ok()
            }),
        ),
        P::Color => (
            color,
            Some(|v, relative| {
                crate::style::colors::color(v, relative, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::ListStyleImage => (
            list_style_image,
            Some(|v, r| {
                crate::style::background_values::image(v, r, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::ListStyleType => (
            list_style_type,
            Some(|v, r| ListStyleType::from_component(v, &validation_scope(r)).is_ok()),
        ),
        P::ListStylePosition => (
            list_style_position,
            Some(|v, _| ListStylePosition::from_component(v).is_ok()),
        ),
        P::MarginTop => (
            margin_top,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Margin(TOP))),
        ),
        P::MarginRight => (
            margin_right,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Margin(RIGHT))),
        ),
        P::MarginBottom => (
            margin_bottom,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Margin(BOTTOM))),
        ),
        P::MarginLeft => (
            margin_left,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Margin(LEFT))),
        ),
        P::PaddingTop => (padding_top, Some(|v, r| length_percentage(v, r).is_ok())),
        P::PaddingRight => (padding_right, Some(|v, r| length_percentage(v, r).is_ok())),
        P::PaddingBottom => (padding_bottom, Some(|v, r| length_percentage(v, r).is_ok())),
        P::PaddingLeft => (padding_left, Some(|v, r| length_percentage(v, r).is_ok())),
        P::Top => (
            top,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Inset(TOP))),
        ),
        P::Right => (
            right,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Inset(RIGHT))),
        ),
        P::Bottom => (
            bottom,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Inset(BOTTOM))),
        ),
        P::Left => (
            left,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Inset(LEFT))),
        ),
        P::Width => (
            width,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Size(HORIZONTAL))),
        ),
        P::MinWidth => (
            min_width,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Size(HORIZONTAL))),
        ),
        P::MaxWidth => (
            max_width,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Size(HORIZONTAL))),
        ),
        P::Height => (
            height,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Size(VERTICAL))),
        ),
        P::MinHeight => (
            min_height,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Size(VERTICAL))),
        ),
        P::MaxHeight => (
            max_height,
            Some(|v, r| anchor_values::supports(v, r, AnchorTarget::Size(VERTICAL))),
        ),
        P::BoxSizing => (box_sizing, Some(|_, _| true)),
        P::Display => (
            display,
            Some(|v, _| Display::from_component(v).validate().is_ok()),
        ),
        P::Position => (
            position,
            Some(|v, _| crate::style::Position::from_component(v).is_ok()),
        ),
        P::Float => (
            float,
            Some(|v, _| crate::style::Float::from_component(v).is_ok()),
        ),
        P::Clear => (
            clear,
            Some(|v, _| crate::style::Clear::from_component(v).is_ok()),
        ),
        P::ZIndex => (
            z_index,
            Some(|v, r| crate::style::position::z_index(v, r).is_ok()),
        ),
        P::OffsetPath => (
            offset_path,
            Some(|v, r| crate::style::motion::path(v, r).is_ok()),
        ),
        P::OffsetPosition => (
            offset_position,
            Some(|v, r| crate::style::motion::position(v, r).is_ok()),
        ),
        P::OffsetAnchor => (
            offset_anchor,
            Some(|v, r| crate::style::motion::anchor(v, r).is_ok()),
        ),
        P::OffsetDistance => (
            offset_distance,
            Some(|v, r| crate::style::motion::distance(v, r).is_ok()),
        ),
        P::OffsetRotate => (
            offset_rotate,
            Some(|v, r| crate::style::motion::rotate(v, r).is_ok()),
        ),
        P::Transform => (
            transform,
            Some(|v, r| crate::style::transform::transform(v, r).is_ok()),
        ),
        P::TransformOrigin => (
            transform_origin,
            Some(|v, r| crate::style::transform::origin(v, r).is_ok()),
        ),
        P::Clip => (
            clip,
            Some(|v, r| crate::style::position::clip_rect(v, r).is_ok()),
        ),
        P::Translate => (
            translate,
            Some(|v, r| crate::style::transform::translate(v, r).is_ok()),
        ),
        P::Rotate => (
            rotate,
            Some(|v, r| crate::style::transform::rotate(v, r).is_ok()),
        ),
        P::Scale => (
            scale,
            Some(|v, r| crate::style::transform::scale(v, r).is_ok()),
        ),
        P::TransformBox => (
            transform_box,
            Some(|v, _| crate::style::TransformBox::from_component(v).is_ok()),
        ),
        P::TransformStyle => (
            transform_style,
            Some(|v, _| crate::style::TransformStyle::from_component(v).is_ok()),
        ),
        P::Perspective => (
            perspective,
            Some(|v, r| crate::style::transform::perspective_distance(v, r).is_ok()),
        ),
        P::PerspectiveOrigin => (
            perspective_origin,
            Some(|v, r| crate::style::background_values::position(v, r).is_ok()),
        ),
        P::BackfaceVisibility => (backface_visibility, Some(|_, _| true)),
        P::ObjectFit => (
            object_fit,
            Some(|v, _| crate::style::ObjectFit::from_component(v).is_ok()),
        ),
        P::Page => (
            page,
            Some(|v, _| crate::style::Page::from_component(v).is_ok()),
        ),
        P::ObjectPosition => (
            object_position,
            Some(|v, r| crate::style::background_values::position(v, r).is_ok()),
        ),
        P::OverflowX => (
            overflow_x,
            Some(|v, _| crate::style::Overflow::from_component(v).is_ok()),
        ),
        P::OverflowY => (
            overflow_y,
            Some(|v, _| crate::style::Overflow::from_component(v).is_ok()),
        ),
        P::OverflowClipMarginTop => (
            overflow_clip_margin_top,
            Some(|v, r| crate::style::OverflowClipMargin::from_component(v, r).is_ok()),
        ),
        P::OverflowClipMarginRight => (
            overflow_clip_margin_right,
            Some(|v, r| crate::style::OverflowClipMargin::from_component(v, r).is_ok()),
        ),
        P::OverflowClipMarginBottom => (
            overflow_clip_margin_bottom,
            Some(|v, r| crate::style::OverflowClipMargin::from_component(v, r).is_ok()),
        ),
        P::OverflowClipMarginLeft => (
            overflow_clip_margin_left,
            Some(|v, r| crate::style::OverflowClipMargin::from_component(v, r).is_ok()),
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
        P::BoxShadowColor => (
            box_shadow_color,
            Some(|v, r| {
                crate::style::shadow_values::colors(v, r, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::BoxShadowOffset => (
            box_shadow_offset,
            Some(|v, r| crate::style::shadow_values::offsets(v, r).is_ok()),
        ),
        P::BoxShadowBlur => (
            box_shadow_blur,
            Some(|v, r| crate::style::shadow_values::lengths(v, r).is_ok()),
        ),
        P::BoxShadowSpread => (
            box_shadow_spread,
            Some(|v, r| crate::style::shadow_values::lengths(v, r).is_ok()),
        ),
        P::BoxShadowPosition => (
            box_shadow_position,
            Some(|v, _| crate::style::shadow_values::positions(v).is_ok()),
        ),
        P::TextShadow => (
            text_shadow,
            Some(|v, r| {
                crate::style::text_paint_values::text_shadows(
                    v,
                    r,
                    &crate::paint::AbsoluteColor::BLACK,
                )
                .is_ok()
            }),
        ),
        P::DashWebkitTextFillColor => (text_fill_color, Some(color_check)),
        P::DashWebkitTextStrokeColor => (text_stroke_color, Some(color_check)),
        P::DashWebkitTextStrokeWidth => (
            text_stroke_width,
            Some(|v, r| crate::style::text_paint_values::stroke_width(v, r).is_ok()),
        ),
        P::TextEmphasisStyle => (
            text_emphasis_style,
            Some(|v, _| crate::style::EmphasisStyle::from_component(v).is_ok()),
        ),
        P::TextEmphasisColor => (text_emphasis_color, Some(color_check)),
        P::TextEmphasisSkip => (
            text_emphasis_skip,
            Some(|v, _| crate::style::EmphasisSkip::from_component(v).is_ok()),
        ),
        P::TextEmphasisPosition => (
            text_emphasis_position,
            Some(|v, _| crate::style::EmphasisPosition::from_component(v).is_ok()),
        ),
        P::Visibility => (
            visibility,
            Some(|v, _| Visibility::from_component(v).is_ok()),
        ),
        P::Opacity => (opacity, Some(|v, r| opacity_value(v, r).is_ok())),
        P::ShapeOutside => (
            shape_outside,
            Some(|v, r| {
                crate::style::shape_values::shape_outside(v, r, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::ClipPath => (
            clip_path,
            Some(|v, r| crate::style::shape_values::clip_path(v, r).is_ok()),
        ),
        P::ShapeMargin => (shape_margin, Some(|v, r| length_percentage(v, r).is_ok())),
        P::ShapeImageThreshold => (
            shape_image_threshold,
            Some(|v, r| opacity_value(v, r).is_ok()),
        ),
        P::BorderImageSource => (
            border_image_source,
            Some(|v, r| {
                crate::style::background_values::image(v, r, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::BorderImageSlice => (
            border_image_slice,
            Some(|v, r| crate::style::border_image_values::slice(v, r).is_ok()),
        ),
        P::BorderImageWidth => (
            border_image_width,
            Some(|v, r| crate::style::border_image_values::widths(v, r).is_ok()),
        ),
        P::BorderImageOutset => (
            border_image_outset,
            Some(|v, r| crate::style::border_image_values::outset(v, r).is_ok()),
        ),
        P::BorderImageRepeat => (
            border_image_repeat,
            Some(|v, _| crate::style::border_image_values::repeat(v).is_ok()),
        ),
        P::TextDecorationLine => (
            text_decoration_line,
            Some(|v, _| crate::style::DecorationLines::from_component(v).is_ok()),
        ),
        P::TextDecorationStyle => (
            text_decoration_style,
            Some(|v, _| crate::style::decoration_values::decoration_style(v).is_ok()),
        ),
        P::TextDecorationColor => (
            text_decoration_color,
            Some(|v, relative| {
                crate::style::colors::color(v, relative, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::TextDecorationThickness => (
            text_decoration_thickness,
            Some(|v, r| crate::style::decoration_values::thickness(v, r).is_ok()),
        ),
        P::TextUnderlinePosition => (text_underline_position, Some(|_, _| true)),
        P::TextUnderlineOffset => (
            text_underline_offset,
            Some(|v, r| crate::style::decoration_values::underline_offset(v, r).is_ok()),
        ),
        P::TextDecorationSkipInk => (
            text_decoration_skip_ink,
            Some(|v, _| v.keyword() != Some(Keyword::All)),
        ),
        P::TextOverflow => (
            text_overflow,
            Some(|v, _| crate::style::TextOverflow::from_component(v).is_ok()),
        ),
        P::BackgroundColor => (
            background_color,
            Some(|v, relative| {
                crate::style::colors::color(v, relative, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::BackgroundImage => (
            background_image,
            Some(|v, r| {
                crate::style::background_values::images(v, r, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::BackgroundPositionX => (
            background_position_x,
            Some(|v, r| crate::style::background_values::axis_positions(v, r, false).is_ok()),
        ),
        P::BackgroundPositionY => (
            background_position_y,
            Some(|v, r| crate::style::background_values::axis_positions(v, r, false).is_ok()),
        ),
        P::BackgroundSize => (
            background_size,
            Some(|v, r| crate::style::background_values::sizes(v, r).is_ok()),
        ),
        P::BackgroundRepeat => (
            background_repeat,
            Some(|v, _| crate::style::background_values::repeats(v).is_ok()),
        ),
        P::BackgroundOrigin => (
            background_origin,
            Some(|v, _| crate::style::background_values::boxes(v).is_ok()),
        ),
        P::BackgroundClip => (
            background_clip,
            Some(|v, _| crate::style::background_values::boxes(v).is_ok()),
        ),
        P::BackgroundAttachment => (
            background_attachment,
            Some(|v, _| crate::style::background_values::attachments(v).is_ok()),
        ),
        P::BackgroundBlendMode => (
            background_blend_mode,
            Some(|v, _| crate::style::compositing_values::blend_modes(v).is_ok()),
        ),
        P::MixBlendMode => (
            mix_blend_mode,
            Some(|v, _| crate::style::compositing_values::mix_blend_mode(v).is_ok()),
        ),
        P::Filter => (
            filter,
            Some(|v, r| {
                crate::style::filter_values::filters(v, r, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::BackdropFilter => (
            backdrop_filter,
            Some(|v, r| {
                crate::style::filter_values::filters(v, r, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::MaskImage => (
            mask_image,
            Some(|v, r| {
                crate::style::mask_values::images(v, r, &crate::paint::AbsoluteColor::BLACK).is_ok()
            }),
        ),
        P::MaskPosition => (
            mask_position,
            Some(|v, r| crate::style::mask_values::positions(v, r).is_ok()),
        ),
        P::MaskSize => (
            mask_size,
            Some(|v, r| crate::style::background_values::sizes(v, r).is_ok()),
        ),
        P::MaskRepeat => (
            mask_repeat,
            Some(|v, _| crate::style::background_values::repeats(v).is_ok()),
        ),
        P::MaskOrigin => (
            mask_origin,
            Some(|v, _| crate::style::mask_values::origins(v).is_ok()),
        ),
        P::MaskClip => (
            mask_clip,
            Some(|v, _| crate::style::mask_values::clips(v).is_ok()),
        ),
        P::MaskMode => (
            mask_mode,
            Some(|v, _| crate::style::mask_values::modes(v).is_ok()),
        ),
        P::MaskComposite => (
            mask_composite,
            Some(|v, _| crate::style::mask_values::composites(v).is_ok()),
        ),
        P::MaskBorderSource => (
            mask_border_source,
            Some(|v, r| {
                crate::style::background_values::image(v, r, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::MaskBorderSlice => (
            mask_border_slice,
            Some(|v, r| crate::style::border_image_values::slice(v, r).is_ok()),
        ),
        P::MaskBorderWidth => (
            mask_border_width,
            Some(|v, r| crate::style::border_image_values::widths(v, r).is_ok()),
        ),
        P::MaskBorderOutset => (
            mask_border_outset,
            Some(|v, r| crate::style::border_image_values::outset(v, r).is_ok()),
        ),
        P::MaskBorderRepeat => (
            mask_border_repeat,
            Some(|v, _| crate::style::border_image_values::repeat(v).is_ok()),
        ),
        P::MaskBorderMode => (
            mask_border_mode,
            Some(|v, _| crate::style::mask_values::border_mode(v).is_ok()),
        ),
        P::Isolation => (
            isolation,
            Some(|v, _| crate::style::compositing_values::isolation(v).is_ok()),
        ),
        P::BorderTopWidth => (border_top_width, Some(|v, r| border_width(v, r).is_ok())),
        P::BorderRightWidth => (border_right_width, Some(|v, r| border_width(v, r).is_ok())),
        P::BorderBottomWidth => (border_bottom_width, Some(|v, r| border_width(v, r).is_ok())),
        P::BorderLeftWidth => (border_left_width, Some(|v, r| border_width(v, r).is_ok())),
        P::OutlineWidth => (outline_width, Some(|v, r| border_width(v, r).is_ok())),
        P::OutlineStyle => (outline_style, Some(|v, _| outline_style_value(v).is_ok())),
        P::OutlineColor => (
            outline_color,
            Some(|v, relative| {
                v.keyword() == Some(Keyword::Auto)
                    || crate::style::colors::color(v, relative, &crate::paint::AbsoluteColor::BLACK)
                        .is_ok()
            }),
        ),
        P::OutlineOffset => (
            outline_offset,
            Some(|v, r| crate::style::values::length(v, r).is_ok()),
        ),
        P::BorderTopStyle => (border_top_style, Some(|v, _| border_style(v).is_ok())),
        P::BorderRightStyle => (border_right_style, Some(|v, _| border_style(v).is_ok())),
        P::BorderBottomStyle => (border_bottom_style, Some(|v, _| border_style(v).is_ok())),
        P::BorderLeftStyle => (border_left_style, Some(|v, _| border_style(v).is_ok())),
        P::BorderTopColor => (
            border_top_color,
            Some(|v, relative| {
                crate::style::colors::color(v, relative, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::BorderRightColor => (
            border_right_color,
            Some(|v, relative| {
                crate::style::colors::color(v, relative, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::BorderBottomColor => (
            border_bottom_color,
            Some(|v, relative| {
                crate::style::colors::color(v, relative, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::BorderLeftColor => (
            border_left_color,
            Some(|v, relative| {
                crate::style::colors::color(v, relative, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
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
            Some(|v, r| {
                crate::style::generated::ComputedContent::from_component(
                    v,
                    crate::style::generated::ContentContext::BeforeAfter,
                    &validation_scope(r),
                )
                .is_ok_and(|content| content.supported())
            }),
        ),
        P::Quotes => (
            quotes,
            Some(|v, _| {
                crate::style::Quotes::from_component(v, "", &crate::style::Quotes::default())
                    .is_ok()
            }),
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
        P::Order => (
            order,
            Some(|v, r| crate::style::values::integer(v, r).is_ok()),
        ),
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
            Some(|v, _| Alignment::content_from_component(v).is_ok()),
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
            Some(|v, r| crate::style::multicol_values::count(v, r).is_ok()),
        ),
        P::ColumnWidth => (
            column_width,
            Some(|v, r| crate::style::multicol_values::width(v, r).is_ok()),
        ),
        P::ColumnFill => (
            column_fill,
            Some(|v, _| crate::style::ColumnFill::from_component(v).is_ok()),
        ),
        P::ColumnSpan => (column_span, Some(|_, _| true)),
        P::ColumnRuleWidth => (column_rule_width, Some(|v, r| border_width(v, r).is_ok())),
        P::ColumnRuleStyle => (column_rule_style, Some(|v, _| border_style(v).is_ok())),
        P::ColumnRuleColor => (
            column_rule_color,
            Some(|v, relative| {
                crate::style::colors::color(v, relative, &crate::paint::AbsoluteColor::BLACK)
                    .is_ok()
            }),
        ),
        P::TableLayout => (
            table_layout,
            Some(|v, _| crate::style::TableLayout::from_component(v).is_ok()),
        ),
        P::BorderCollapse => (
            border_collapse,
            Some(|v, _| crate::style::BorderCollapse::from_component(v).is_ok()),
        ),
        P::BorderSpacing => (
            border_spacing,
            Some(|v, r| crate::style::table_values::spacing(v, r).is_ok()),
        ),
        P::EmptyCells => (
            empty_cells,
            Some(|v, _| crate::style::table_values::empty_cells(v).is_ok()),
        ),
        P::CaptionSide => (
            caption_side,
            Some(|v, _| crate::style::CaptionSide::from_component(v).is_ok()),
        ),
        P::VerticalAlign => (
            vertical_align,
            Some(|v, r| crate::style::VerticalAlign::from_component(v, r).is_ok()),
        ),
        P::TextBoxTrim => (
            text_box_trim,
            Some(|v, _| crate::style::TextBox::trim(v).is_ok()),
        ),
        P::TextBoxEdge => (
            text_box_edge,
            Some(|v, _| crate::style::TextBox::edges(v).is_ok()),
        ),
        P::InitialLetter => (
            initial_letter,
            Some(|v, r| crate::style::InitialLetter::from_component(v, r).is_ok()),
        ),
        P::InitialLetterAlign => (
            initial_letter_align,
            Some(|v, _| crate::style::InitialLetterAlign::from_component(v).is_ok()),
        ),
        P::Fill => (
            svg_values::fill,
            Some(|v, r| svg_values::supported(svg_values::fill, v, r)),
        ),
        P::FillOpacity => (
            svg_values::fill_opacity,
            Some(|v, r| svg_values::supported(svg_values::fill_opacity, v, r)),
        ),
        P::FillRule => (
            svg_values::fill_rule,
            Some(|v, r| svg_values::supported(svg_values::fill_rule, v, r)),
        ),
        P::Stroke => (
            svg_values::stroke,
            Some(|v, r| svg_values::supported(svg_values::stroke, v, r)),
        ),
        P::StrokeOpacity => (
            svg_values::stroke_opacity,
            Some(|v, r| svg_values::supported(svg_values::stroke_opacity, v, r)),
        ),
        P::StrokeWidth => (
            svg_values::stroke_width,
            Some(|v, r| svg_values::supported(svg_values::stroke_width, v, r)),
        ),
        P::StrokeDasharray => (
            svg_values::stroke_dasharray,
            Some(|v, r| svg_values::supported(svg_values::stroke_dasharray, v, r)),
        ),
        P::StrokeDashoffset => (
            svg_values::stroke_dashoffset,
            Some(|v, r| svg_values::supported(svg_values::stroke_dashoffset, v, r)),
        ),
        P::StrokeLinecap => (
            svg_values::stroke_linecap,
            Some(|v, r| svg_values::supported(svg_values::stroke_linecap, v, r)),
        ),
        P::StrokeLinejoin => (
            svg_values::stroke_linejoin,
            Some(|v, r| svg_values::supported(svg_values::stroke_linejoin, v, r)),
        ),
        P::StrokeMiterlimit => (
            svg_values::stroke_miterlimit,
            Some(|v, r| svg_values::supported(svg_values::stroke_miterlimit, v, r)),
        ),
        P::MarkerStart => (
            svg_values::marker_start,
            Some(|v, r| svg_values::supported(svg_values::marker_start, v, r)),
        ),
        P::MarkerMid => (
            svg_values::marker_mid,
            Some(|v, r| svg_values::supported(svg_values::marker_mid, v, r)),
        ),
        P::MarkerEnd => (
            svg_values::marker_end,
            Some(|v, r| svg_values::supported(svg_values::marker_end, v, r)),
        ),
        P::PaintOrder => (
            svg_values::paint_order,
            Some(|v, r| svg_values::supported(svg_values::paint_order, v, r)),
        ),
        P::StopColor => (
            svg_values::stop_color,
            Some(|v, r| svg_values::supported(svg_values::stop_color, v, r)),
        ),
        P::StopOpacity => (
            svg_values::stop_opacity,
            Some(|v, r| svg_values::supported(svg_values::stop_opacity, v, r)),
        ),
        P::FloodColor => (
            svg_values::flood_color,
            Some(|v, r| svg_values::supported(svg_values::flood_color, v, r)),
        ),
        P::FloodOpacity => (
            svg_values::flood_opacity,
            Some(|v, r| svg_values::supported(svg_values::flood_opacity, v, r)),
        ),
        P::LightingColor => (
            svg_values::lighting_color,
            Some(|v, r| svg_values::supported(svg_values::lighting_color, v, r)),
        ),
        P::ClipRule => (
            svg_values::clip_rule,
            Some(|v, r| svg_values::supported(svg_values::clip_rule, v, r)),
        ),
        P::MaskType => (
            svg_values::mask_type,
            Some(|v, r| svg_values::supported(svg_values::mask_type, v, r)),
        ),
        P::ColorInterpolation => (
            svg_values::color_interpolation,
            Some(|v, r| svg_values::supported(svg_values::color_interpolation, v, r)),
        ),
        P::ColorInterpolationFilters => (
            svg_values::color_interpolation_filters,
            Some(|v, r| svg_values::supported(svg_values::color_interpolation_filters, v, r)),
        ),
        P::TextAnchor => (
            svg_values::text_anchor,
            Some(|v, r| svg_values::supported(svg_values::text_anchor, v, r)),
        ),
        P::ShapeRendering => (
            svg_values::shape_rendering,
            Some(|v, r| svg_values::supported(svg_values::shape_rendering, v, r)),
        ),
        P::TextRendering => (
            svg_values::text_rendering,
            Some(|v, r| svg_values::supported(svg_values::text_rendering, v, r)),
        ),
        P::ImageRendering => (
            svg_values::image_rendering,
            Some(|v, r| svg_values::supported(svg_values::image_rendering, v, r)),
        ),
        P::Cx => (
            svg_values::cx,
            Some(|v, r| svg_values::supported(svg_values::cx, v, r)),
        ),
        P::Cy => (
            svg_values::cy,
            Some(|v, r| svg_values::supported(svg_values::cy, v, r)),
        ),
        P::R => (
            svg_values::r,
            Some(|v, r| svg_values::supported(svg_values::r, v, r)),
        ),
        P::Rx => (
            svg_values::rx,
            Some(|v, r| svg_values::supported(svg_values::rx, v, r)),
        ),
        P::Ry => (
            svg_values::ry,
            Some(|v, r| svg_values::supported(svg_values::ry, v, r)),
        ),
        P::X => (
            svg_values::x,
            Some(|v, r| svg_values::supported(svg_values::x, v, r)),
        ),
        P::Y => (
            svg_values::y,
            Some(|v, r| svg_values::supported(svg_values::y, v, r)),
        ),
        P::D => (
            svg_values::d,
            Some(|v, r| svg_values::supported(svg_values::d, v, r)),
        ),
        P::RubyPosition => (
            ruby_position,
            Some(|v, _| crate::style::RubyStyle::position(v).is_ok()),
        ),
        P::RubyMerge => (
            ruby_merge,
            Some(|v, _| crate::style::RubyStyle::merge(v).is_ok()),
        ),
        P::RubyAlign => (
            ruby_align,
            Some(|v, _| crate::style::RubyStyle::align(v).is_ok()),
        ),
        P::RubyOverhang => (
            ruby_overhang,
            Some(|v, _| crate::style::RubyStyle::overhang(v).is_ok()),
        ),
        P::DominantBaseline => (
            dominant_baseline,
            Some(|v, _| crate::style::Baseline::dominant(v).is_ok()),
        ),
        P::AlignmentBaseline => (
            alignment_baseline,
            Some(|v, _| crate::style::Baseline::alignment(v).is_ok()),
        ),
        P::BaselineShift => (
            baseline_shift,
            Some(|v, r| crate::style::BaselineShift::from_component(v, r).is_ok()),
        ),
        P::BaselineSource => (
            baseline_source,
            Some(|v, _| crate::style::BaselineSource::from_component(v).is_ok()),
        ),
        P::TableColumnSpan => (
            table_column_span,
            Some(|v, r| crate::style::table_values::span(v, r).is_ok()),
        ),
        P::TableRowSpan => (
            table_row_span,
            Some(|v, r| crate::style::table_values::span(v, r).is_ok()),
        ),
        P::BookmarkLevel => (
            bookmark_level,
            Some(|v, r| v.keyword() == Some(Keyword::None) || positive_integer(v, r).is_ok()),
        ),
        P::BookmarkLabel => (
            bookmark_label,
            Some(|v, r| {
                crate::style::GeneratedContent::from_list(v, &validation_scope(r))
                    .is_ok_and(|content| content.supported())
            }),
        ),
        P::BookmarkState => (bookmark_state, Some(|_, _| true)),
        P::WillChange => (
            will_change,
            Some(|v, _| crate::style::will_change::WillChange::from_component(v).is_ok()),
        ),
        P::ViewTransitionName => (view_transition_name, Some(|_, _| true)),
        P::AspectRatio => (
            aspect_ratio,
            Some(|v, r| crate::style::AspectRatio::from_component(v, r).is_ok()),
        ),
        P::Contain => (
            contain,
            Some(|v, _| crate::style::Contain::from_component(v).is_ok()),
        ),
        P::ContainerType => (container_type, Some(|_, _| true)),
        P::ContainerName => (
            container_name,
            Some(|v, _| crate::style::containment::container_names(v).is_ok()),
        ),
        P::ContentVisibility => (
            content_visibility,
            Some(|v, _| crate::style::ContentVisibility::from_component(v).is_ok()),
        ),
        P::ContainIntrinsicWidth => (
            contain_intrinsic_width,
            Some(|v, r| crate::style::ContainIntrinsicSize::from_component(v, r).is_ok()),
        ),
        P::ContainIntrinsicHeight => (
            contain_intrinsic_height,
            Some(|v, r| crate::style::ContainIntrinsicSize::from_component(v, r).is_ok()),
        ),
        P::AnchorName => (
            anchor_name,
            Some(|v, _| anchor_values::anchor_names(v, FontNamespace::ROOT).is_ok()),
        ),
        P::AnchorScope => (
            anchor_scope,
            Some(|v, _| anchor_values::anchor_scope(v, FontNamespace::ROOT).is_ok()),
        ),
        P::PositionAnchor => (
            position_anchor,
            Some(|v, _| anchor_values::position_anchor(v, FontNamespace::ROOT).is_ok()),
        ),
        P::PositionArea => (
            position_area,
            Some(|v, _| anchor_values::position_area(v).is_ok()),
        ),
        P::PositionTryFallbacks => (
            position_try_fallbacks,
            Some(|v, _| anchor_values::try_fallbacks(v, FontNamespace::ROOT).is_ok()),
        ),
        P::PositionTryOrder => (
            position_try_order,
            Some(|v, _| anchor_values::try_order(v).is_ok()),
        ),
        P::PositionVisibility => (
            position_visibility,
            Some(|v, _| anchor_values::position_visibility(v).is_ok()),
        ),
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
    parent: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    if let Input::Copy(source) = input {
        style.size = source.size;
        return Ok(());
    }
    style.size = match input {
        Input::Copy(_) => unreachable!("computed copy returned above"),
        Input::Value(value) if value.keyword() == Some(Keyword::Math) => {
            parent.size
                * crate::style::math_values::scale(
                    parent.math.depth,
                    style.math.depth,
                    relative.font.math_scaling()?,
                )?
        }
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
            .map(|(_, factor)| factor * crate::style::DEFAULT_FONT_SIZE)
            .ok_or_else(|| unsupported(value))?,
        None => crate::style::box_values::font_length_percentage(value, relative)?,
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
        Input::Value(value) => style.families = crate::style::values::families(value),
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
        crate::style::values::font_weight(value, parent.font_weight, relative)?
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
        Input::Value(value) => crate::style::values::font_style(value, relative)?,
    };
    Ok(())
}
computed!(
    overflow_clip_margin_top,
    [overflow_clip_margin[TOP]],
    |value, style, _parent, relative| {
        crate::style::OverflowClipMargin::from_component(value, relative)?
    }
);
computed!(
    overflow_clip_margin_right,
    [overflow_clip_margin[RIGHT]],
    |value, style, _parent, relative| {
        crate::style::OverflowClipMargin::from_component(value, relative)?
    }
);
computed!(
    overflow_clip_margin_bottom,
    [overflow_clip_margin[BOTTOM]],
    |value, style, _parent, relative| {
        crate::style::OverflowClipMargin::from_component(value, relative)?
    }
);
computed!(
    overflow_clip_margin_left,
    [overflow_clip_margin[LEFT]],
    |value, style, _parent, relative| {
        crate::style::OverflowClipMargin::from_component(value, relative)?
    }
);
computed!(
    font_width,
    [font_width],
    |value, style, _parent, relative| { crate::style::values::font_width(value, relative)? }
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
    font_synthesis_position,
    [font_synthesis.position],
    |value, style, _parent, _relative| flag(
        value,
        &[(Keyword::Auto, true), (Keyword::None, false)]
    )?
);
computed!(
    font_variant_caps,
    [font_variant_caps],
    |value, style, _parent, _relative| crate::style::FontVariantCaps::from_component(value)?
);
computed!(
    font_kerning,
    [font_features.kerning],
    |value, style, _parent, _relative| crate::style::FontKerning::from_component(value)?
);
computed!(
    font_variant_ligatures,
    [font_features.ligatures],
    |value, style, _parent, _relative| crate::style::font_values::variant_features(value)?
);
computed!(
    font_variant_position,
    [font_features.position],
    |value, style, _parent, _relative| crate::style::FontVariantPosition::from_component(value)?
);
computed!(
    font_variant_numeric,
    [font_features.numeric],
    |value, style, _parent, _relative| crate::style::font_values::variant_features(value)?
);
computed!(
    font_variant_alternates,
    [font_features.alternates],
    |value, style, _parent, _relative| std::sync::Arc::new(
        crate::style::FontVariantAlternates::from_component(value)?
    )
);
computed!(
    font_variant_east_asian,
    [font_features.east_asian],
    |value, style, _parent, _relative| crate::style::font_values::variant_features(value)?
);
computed!(
    font_variant_emoji,
    [font_features.emoji],
    |value, style, _parent, _relative| crate::style::FontVariantEmoji::from_component(value)?
);
computed!(
    font_feature_settings,
    [font_features.settings],
    |value, style, _parent, _relative| crate::style::font_values::feature_settings(value)?
);
computed!(
    font_language_override,
    [font_features.language_override],
    |value, style, _parent, _relative| crate::style::font_values::language_override(value)?
);
computed!(
    font_palette,
    [font_features.palette],
    |value, style, _parent, relative| crate::style::font_values::palette(value, relative)?
);
computed!(
    font_optical_sizing,
    [font_adjustments.optical_sizing],
    |value, style, _parent, _relative| flag(
        value,
        &[(Keyword::Auto, true), (Keyword::None, false)]
    )?
);
computed!(
    font_variation_settings,
    [font_adjustments.variations],
    |value, style, _parent, relative| crate::style::font_values::variation_settings(
        value, relative
    )?
);
computed!(
    font_size_adjust,
    [font_adjustments.size_adjust],
    |value, style, _parent, relative| crate::style::font_values::size_adjust(value, relative)?
);
computed!(
    line_height,
    [line_height],
    |value, style, _parent, relative| { LineHeight::from_component(value, relative)? }
);
computed!(
    line_height_step,
    [line_height_step],
    |value, style, _parent, relative| line_height_step_value(value, relative)?
);
/// `<length [0,∞]>` (CSS Rhythmic Sizing 1 §3).
fn line_height_step_value(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    let step = crate::style::values::length(value, relative)?;
    if step < 0.0 {
        return Err(format!("negative line-height-step: {value}").into());
    }
    Ok(step)
}
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
    white_space_collapse,
    [white_space.collapse],
    |value, style, _parent, _relative| { WhiteSpaceCollapse::from_component(value)? }
);
computed!(
    text_wrap_mode,
    [white_space.wrap],
    |value, style, _parent, _relative| flag(
        value,
        &[(Keyword::Wrap, true), (Keyword::Nowrap, false)]
    )?
);
computed!(
    text_wrap_style,
    [line_breaking.wrap_style],
    |value, style, _parent, _relative| { TextWrapStyle::from_component(value)? }
);
computed!(
    word_break,
    [line_breaking.word_break],
    |value, style, _parent, _relative| { WordBreak::from_component(value)? }
);
computed!(
    line_break,
    [line_breaking.line_break],
    |value, style, _parent, _relative| { LineBreak::from_component(value)? }
);
computed!(
    hanging_punctuation,
    [line_breaking.hanging_punctuation],
    |value, style, _parent, _relative| { HangingPunctuation::from_component(value)? }
);
computed!(
    text_justify,
    [line_breaking.justify],
    |value, style, _parent, _relative| { TextJustify::from_component(value)? }
);
computed!(
    text_autospace,
    [text_spacing.autospace],
    |value, style, _parent, _relative| { TextAutospace::from_component(value)? }
);
computed!(
    text_spacing_trim,
    [text_spacing.trim],
    |value, style, _parent, _relative| { TextSpacingTrim::from_component(value)? }
);
computed!(
    hyphenate_character,
    [line_breaking.hyphenate_character],
    |value, style, _parent, _relative| { text_values::hyphenate_character(value)? }
);
computed!(
    hyphenate_limit_chars,
    [line_breaking.hyphenate_limit_chars],
    |value, style, _parent, relative| { HyphenateLimitChars::from_component(value, relative)? }
);
computed!(
    max_lines,
    [clamp.max_lines],
    |value, style, _parent, relative| { max_lines_value(value, relative)? }
);

fn max_lines_value(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<usize>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(None);
    }
    match usize::try_from(crate::style::values::integer(value, relative)?) {
        Ok(lines) if lines > 0 => Ok(Some(lines)),
        _ => Err(unsupported(value)),
    }
}
computed!(
    block_ellipsis,
    [clamp.block_ellipsis],
    |value, style, _parent, _relative| { crate::style::BlockEllipsis::from_component(value)? }
);
computed!(
    continuation,
    [clamp.continuation],
    |value, style, _parent, _relative| { crate::style::Continue::from_component(value)? }
);
computed!(tab_size, [tab_size], |value, style, _parent, relative| {
    tab_size_value(value, relative)?
});

fn tab_size_value(value: &Component, relative: &Relative<'_>) -> Result<usize, StyleError> {
    match value.unwrap() {
        Component::Length(_) | Component::Math(_, Kind::Length, _) => Err(unsupported(value)),
        _ => usize::try_from(crate::style::values::integer(value, relative)?)
            .map_err(|_| format!("negative tab-size: {value}").into()),
    }
}
computed!(
    unicode_bidi,
    [unicode_bidi],
    |value, style, _parent, _relative| { crate::style::UnicodeBidi::from_component(value)? }
);
computed!(
    text_combine,
    [text_combine],
    |value, style, _parent, relative| {
        crate::style::logical::TextCombine::from_component(value, relative)?
    }
);
computed!(
    text_orientation,
    [text_orientation],
    |value, style, _parent, _relative| {
        crate::style::logical::TextOrientation::from_component(value)?
    }
);
computed!(
    writing_mode,
    [writing_mode],
    |value, style, _parent, _relative| {
        crate::style::logical::WritingMode::from_component(value)?
    }
);
computed!(direction, [rtl], |value, style, _parent, _relative| flag(
    value,
    &[(Keyword::Rtl, true), (Keyword::Ltr, false)]
)?);
computed!(
    text_indent,
    [text_indent],
    |value, style, _parent, relative| {
        crate::style::TextIndent::from_component(value, relative)?
    }
);
computed!(
    text_align_all,
    [align],
    |value, style, parent, _relative| {
        if value.keyword() == Some(Keyword::MatchParent) {
            if style.document_root {
                TextAlign::Start
            } else {
                parent.align.physical(parent.rtl)
            }
        } else {
            TextAlign::from_component(value)?
        }
    }
);

computed!(
    text_align_last,
    [align_last],
    |value, style, parent, _relative| match value.keyword() {
        Some(Keyword::Auto) => None,
        Some(Keyword::MatchParent) if style.document_root => Some(TextAlign::Start),
        Some(Keyword::MatchParent) => parent.align_last.map(|align| align.physical(parent.rtl)),
        _ => Some(TextAlign::from_component(value)?),
    }
);
computed!(hyphens, [hyphens], |value, style, _parent, _relative| {
    crate::style::Hyphens::from_component(value)?
});

computed!(
    color_scheme,
    [color_scheme],
    |value, style, _parent, _relative| {
        crate::style::color_scheme::ColorSchemes::from_component(value)
    }
);
fn color(
    style: &mut Style,
    parent: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    style.computed_color = match input {
        Input::Value(value) => {
            crate::style::colors::absolute(value, relative, &parent.computed_color)?
        }
        Input::Copy(source) => source.computed_color,
    };
    style.color = style.computed_color.used();
    Ok(())
}
computed!(
    accent_color,
    [accent_color],
    |value, style, _parent, relative| match value.keyword() {
        Some(Keyword::Auto) => None,
        _ => Some(crate::style::colors::absolute(
            value,
            relative,
            &style.computed_color
        )?),
    }
);
computed!(
    list_style_image,
    [list_style_image],
    |value, style, _parent, relative| {
        crate::style::background_values::image(value, relative, &style.computed_color)?
    }
);
computed!(
    list_style_type,
    [list_style_type],
    |value, style, _parent, relative| {
        ListStyleType::from_component(value, &element_scope(relative, style))?
    }
);
computed!(
    list_style_position,
    [list_style_position],
    |value, style, _parent, _relative| { ListStylePosition::from_component(value)? }
);
computed!(
    margin_top,
    [margin[0]],
    |value, style, _parent, relative| {
        anchor_values::size(value, style, relative, AnchorTarget::Margin(TOP))?
    }
);
computed!(
    margin_right,
    [margin[1]],
    |value, style, _parent, relative| {
        anchor_values::size(value, style, relative, AnchorTarget::Margin(RIGHT))?
    }
);
computed!(
    margin_bottom,
    [margin[2]],
    |value, style, _parent, relative| {
        anchor_values::size(value, style, relative, AnchorTarget::Margin(BOTTOM))?
    }
);
computed!(
    margin_left,
    [margin[3]],
    |value, style, _parent, relative| {
        anchor_values::size(value, style, relative, AnchorTarget::Margin(LEFT))?
    }
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
    anchor_values::size(value, style, relative, AnchorTarget::Inset(TOP))?
});
computed!(right, [inset[1]], |value, style, _parent, relative| {
    anchor_values::size(value, style, relative, AnchorTarget::Inset(RIGHT))?
});
computed!(bottom, [inset[2]], |value, style, _parent, relative| {
    anchor_values::size(value, style, relative, AnchorTarget::Inset(BOTTOM))?
});
computed!(left, [inset[3]], |value, style, _parent, relative| {
    anchor_values::size(value, style, relative, AnchorTarget::Inset(LEFT))?
});
computed!(width, [width], |value, style, _parent, relative| {
    anchor_values::size(value, style, relative, AnchorTarget::Size(HORIZONTAL))?
});
computed!(min_width, [min_width], |value, style, _parent, relative| {
    anchor_values::size(value, style, relative, AnchorTarget::Size(HORIZONTAL))?
});
computed!(max_width, [max_width], |value, style, _parent, relative| {
    anchor_values::size(value, style, relative, AnchorTarget::Size(HORIZONTAL))?
});
computed!(height, [height], |value, style, _parent, relative| {
    anchor_values::size(value, style, relative, AnchorTarget::Size(VERTICAL))?
});
computed!(
    min_height,
    [min_height],
    |value, style, _parent, relative| {
        anchor_values::size(value, style, relative, AnchorTarget::Size(VERTICAL))?
    }
);
computed!(
    max_height,
    [max_height],
    |value, style, _parent, relative| {
        anchor_values::size(value, style, relative, AnchorTarget::Size(VERTICAL))?
    }
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
#[expect(
    clippy::unnecessary_wraps,
    reason = "signature is the fallible Apply handler type"
)]
fn display(
    style: &mut Style,
    _parent: &Style,
    input: Input<'_>,
    _relative: &Relative<'_>,
) -> Result<(), StyleError> {
    match input {
        Input::Copy(source) => {
            style.display = source.display.clone();
            style.flex.legacy_box = source.flex.legacy_box;
        }
        Input::Value(value) => {
            style.display = Display::from_component(value);
            style.flex.legacy_box = Display::is_legacy_box(value);
        }
    }
    Ok(())
}
computed!(position, [position], |value, style, _parent, _relative| {
    crate::style::Position::from_component(value)?
});
computed!(
    aspect_ratio,
    [aspect_ratio],
    |value, style, _parent, relative| {
        crate::style::AspectRatio::from_component(value, relative)?
    }
);
computed!(
    contain,
    [containment.contain],
    |value, style, _parent, _relative| { crate::style::Contain::from_component(value)? }
);
computed!(
    container_type,
    [containment.container_type],
    |value, style, _parent, _relative| { crate::style::ContainerType::from_component(value) }
);
computed!(
    container_name,
    [containment.container_names],
    |value, style, _parent, _relative| { crate::style::containment::container_names(value)? }
);
computed!(
    content_visibility,
    [containment.content_visibility],
    |value, style, _parent, _relative| { crate::style::ContentVisibility::from_component(value)? }
);
computed!(
    contain_intrinsic_width,
    [containment.intrinsic[0]],
    |value, style, _parent, relative| {
        crate::style::ContainIntrinsicSize::from_component(value, relative)?
    }
);
computed!(
    contain_intrinsic_height,
    [containment.intrinsic[1]],
    |value, style, _parent, relative| {
        crate::style::ContainIntrinsicSize::from_component(value, relative)?
    }
);
computed!(
    anchor_name,
    [anchor.names],
    |value, style, _parent, _relative| {
        anchor_values::anchor_names(
            value,
            style
                .anchor
                .namespaces
                .get(&Property::AnchorName)
                .copied()
                .unwrap_or_else(|| style.element_font_namespace()),
        )?
    }
);
computed!(
    anchor_scope,
    [anchor.scope],
    |value, style, _parent, _relative| {
        anchor_values::anchor_scope(
            value,
            style
                .anchor
                .namespaces
                .get(&Property::AnchorScope)
                .copied()
                .unwrap_or_else(|| style.element_font_namespace()),
        )?
    }
);
computed!(
    position_anchor,
    [anchor.position_anchor],
    |value, style, _parent, _relative| {
        anchor_values::position_anchor(
            value,
            style
                .anchor
                .namespaces
                .get(&Property::PositionAnchor)
                .copied()
                .unwrap_or_else(|| style.element_font_namespace()),
        )?
    }
);
computed!(
    position_area,
    [anchor.area],
    |value, style, _parent, _relative| { anchor_values::position_area(value)? }
);
computed!(
    position_try_fallbacks,
    [anchor.fallbacks],
    |value, style, _parent, _relative| {
        anchor_values::try_fallbacks(
            value,
            style
                .anchor
                .namespaces
                .get(&Property::PositionTryFallbacks)
                .copied()
                .unwrap_or_else(|| style.element_font_namespace()),
        )?
    }
);
computed!(
    position_try_order,
    [anchor.order],
    |value, style, _parent, _relative| { anchor_values::try_order(value)? }
);
computed!(
    position_visibility,
    [anchor.visibility],
    |value, style, _parent, _relative| { anchor_values::position_visibility(value)? }
);
computed!(z_index, [z_index], |value, style, _parent, relative| {
    crate::style::position::z_index(value, relative)?
});
macro_rules! motion {
    ($name:ident, $field:ident) => {
        fn $name(
            style: &mut Style,
            _: &Style,
            input: Input<'_>,
            relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            let value = match input {
                Input::Value(value) => crate::style::motion::$field(value, relative)?,
                Input::Copy(source) => source.motion.$field.to_owned(),
            };
            std::sync::Arc::make_mut(&mut style.motion).$field = value;
            Ok(())
        }
    };
}
motion!(offset_path, path);
motion!(offset_position, position);
motion!(offset_anchor, anchor);
motion!(offset_distance, distance);
motion!(offset_rotate, rotate);
computed!(transform, [transform], |value, style, _parent, relative| {
    crate::style::transform::transform(value, relative)?
});
computed!(
    transform_origin,
    [transform_origin],
    |value, style, _parent, relative| { crate::style::transform::origin(value, relative)? }
);
computed!(clip, [clip], |value, style, _parent, relative| {
    crate::style::position::clip_rect(value, relative)?
});
computed!(translate, [translate], |value, style, _parent, relative| {
    crate::style::transform::translate(value, relative)?
});
computed!(rotate, [rotate], |value, style, _parent, relative| {
    crate::style::transform::rotate(value, relative)?
});
computed!(scale, [scale], |value, style, _parent, relative| {
    crate::style::transform::scale(value, relative)?
});
computed!(
    transform_box,
    [transform_box],
    |value, style, _parent, _relative| crate::style::TransformBox::from_component(value)?
);
computed!(
    transform_style,
    [transform_style],
    |value, style, _parent, _relative| crate::style::TransformStyle::from_component(value)?
);
computed!(
    perspective,
    [perspective],
    |value, style, _parent, relative| {
        crate::style::transform::perspective_distance(value, relative)?
    }
);
computed!(
    perspective_origin,
    [perspective_origin],
    |value, style, _parent, relative| {
        crate::style::background_values::position(value, relative)?
    }
);
computed!(
    backface_visibility,
    [backface_hidden],
    |value, style, _parent, _relative| flag(
        value,
        &[(Keyword::Hidden, true), (Keyword::Visible, false)]
    )?
);
computed!(
    object_fit,
    [object_fit],
    |value, style, _parent, _relative| { crate::style::ObjectFit::from_component(value)? }
);
computed!(page, [page], |value, style, _parent, _relative| {
    crate::style::Page::from_component(value)?
});
computed!(
    object_position,
    [object_position],
    |value, style, _parent, relative| {
        crate::style::background_values::position(value, relative)?
    }
);
computed!(
    overflow_x,
    [overflow[0]],
    |value, style, _parent, _relative| { crate::style::Overflow::from_component(value)? }
);
computed!(
    overflow_y,
    [overflow[1]],
    |value, style, _parent, _relative| { crate::style::Overflow::from_component(value)? }
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
    crate::style::Float::from_component(value)?
});
computed!(clear, [clear], |value, style, _parent, _relative| {
    crate::style::Clear::from_component(value)?
});
computed!(widows, [widows], |value, style, _parent, relative| {
    positive_integer(value, relative)?
});
computed!(orphans, [orphans], |value, style, _parent, relative| {
    positive_integer(value, relative)?
});
/// `widows` and `orphans`; CSS Fragmentation 3 §4.4 makes zero and negatives invalid.
fn positive_integer(value: &Component, relative: &Relative<'_>) -> Result<usize, StyleError> {
    usize::try_from(crate::style::values::integer(value, relative)?)
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
computed!(
    shape_outside,
    [shape_outside],
    |value, style, _parent, relative| {
        std::sync::Arc::new(crate::style::shape_values::shape_outside(
            value,
            relative,
            &style.computed_color,
        )?)
    }
);
computed!(
    shape_margin,
    [shape_margin],
    |value, style, _parent, relative| length_percentage(value, relative)?
);
// CSS Shapes 1 §6.2: an `<opacity-value>` clamped to `0..=1`.
computed!(
    shape_image_threshold,
    [shape_image_threshold],
    |value, style, _parent, relative| opacity_value(value, relative)?
);
computed!(
    mix_blend_mode,
    [mix_blend_mode],
    |value, style, _parent, _relative| crate::style::compositing_values::mix_blend_mode(value)?
);
computed!(filter, [filter], |value, style, _parent, relative| {
    std::sync::Arc::new(crate::style::filter_values::filters(
        value,
        relative,
        &style.computed_color,
    )?)
});
fn clip_path(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    use crate::style::shape_values::ClipPathValue;
    let (shape, reference) = match input {
        Input::Value(value) => match crate::style::shape_values::clip_path(value, relative)? {
            ClipPathValue::None => (None, None),
            ClipPathValue::Reference(reference) => (None, Some(reference)),
            ClipPathValue::Shape(shape) => (Some(std::sync::Arc::new(shape)), None),
        },
        Input::Copy(source) => (
            source.decoration.masks.clip_path.clone(),
            source.svg.clip_reference.clone(),
        ),
    };
    style.masks_mut().clip_path = shape;
    if style.svg.clip_reference != reference {
        std::sync::Arc::make_mut(&mut style.svg).clip_reference = reference;
    }
    Ok(())
}
computed!(
    backdrop_filter,
    [backdrop_filter],
    |value, style, _parent, relative| {
        std::sync::Arc::new(crate::style::filter_values::filters(
            value,
            relative,
            &style.computed_color,
        )?)
    }
);
fn mask_image(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    let (images, sources) = match input {
        Input::Value(value) => {
            crate::style::mask_values::images(value, relative, &style.computed_color)?
        }
        Input::Copy(source) => (
            source.decoration.masks.layers.images.clone(),
            source.decoration.masks.sources.clone(),
        ),
    };
    let masks = style.masks_mut();
    std::sync::Arc::make_mut(&mut masks.layers).images = images;
    masks.sources = sources;
    Ok(())
}
fn mask_position(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    let positions = match input {
        Input::Value(value) => {
            let positions = crate::style::mask_values::positions(value, relative)?;
            let axis = |index: usize| {
                positions
                    .iter()
                    .map(|position| position[index].clone())
                    .collect()
            };
            [axis(POSITION_X), axis(POSITION_Y)]
        }
        Input::Copy(source) => source.decoration.masks.layers.positions.clone(),
    };
    std::sync::Arc::make_mut(&mut style.masks_mut().layers).positions = positions;
    Ok(())
}
fn mask_size(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    let sizes = match input {
        Input::Value(value) => crate::style::background_values::sizes(value, relative)?,
        Input::Copy(source) => source.decoration.masks.layers.sizes.clone(),
    };
    std::sync::Arc::make_mut(&mut style.masks_mut().layers).sizes = sizes;
    Ok(())
}
fn mask_repeat(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    _: &Relative<'_>,
) -> Result<(), StyleError> {
    let repeats = match input {
        Input::Value(value) => crate::style::background_values::repeats(value)?,
        Input::Copy(source) => source.decoration.masks.layers.repeats.clone(),
    };
    std::sync::Arc::make_mut(&mut style.masks_mut().layers).repeats = repeats;
    Ok(())
}
fn mask_origin(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    _: &Relative<'_>,
) -> Result<(), StyleError> {
    let origins = match input {
        Input::Value(value) => crate::style::mask_values::origins(value)?,
        Input::Copy(source) => source.decoration.masks.layers.origins.clone(),
    };
    std::sync::Arc::make_mut(&mut style.masks_mut().layers).origins = origins;
    Ok(())
}
fn mask_clip(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    _: &Relative<'_>,
) -> Result<(), StyleError> {
    let (clips, unclipped) = match input {
        Input::Value(value) => crate::style::mask_values::clips(value)?,
        Input::Copy(source) => (
            source.decoration.masks.layers.clips.clone(),
            source.decoration.masks.unclipped.clone(),
        ),
    };
    let masks = style.masks_mut();
    std::sync::Arc::make_mut(&mut masks.layers).clips = clips;
    masks.unclipped = unclipped;
    Ok(())
}
fn mask_mode(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    _: &Relative<'_>,
) -> Result<(), StyleError> {
    style.masks_mut().modes = match input {
        Input::Value(value) => crate::style::mask_values::modes(value)?,
        Input::Copy(source) => source.decoration.masks.modes.clone(),
    };
    Ok(())
}
fn mask_composite(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    _: &Relative<'_>,
) -> Result<(), StyleError> {
    style.masks_mut().composites = match input {
        Input::Value(value) => crate::style::mask_values::composites(value)?,
        Input::Copy(source) => source.decoration.masks.composites.clone(),
    };
    Ok(())
}
computed!(isolation, [isolate], |value, style, _parent, _relative| {
    crate::style::compositing_values::isolation(value)?
});

pub(super) fn opacity_value(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    Ok(match value.unwrap() {
        Component::Percentage(fraction) => *fraction,
        Component::Math(source, Kind::Percentage, range) => {
            crate::style::values::math(source, relative, Kind::Percentage, Some(Kind::Percentage))?
                .resolve(0.0, *range)
        }
        _ => crate::style::values::number(value, relative)?,
    }
    .clamp(0.0, 1.0))
}
macro_rules! shadow_list {
    ($name:ident, $field:ident, $compute:expr) => {
        fn $name(
            style: &mut Style,
            _: &Style,
            input: Input<'_>,
            relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            style.shadows.$field = match input {
                Input::Value(value) => ($compute)(value, &*style, relative)?,
                Input::Copy(source) => source.shadows.$field.clone(),
            };
            style.shadows.resolve(relative, &style.computed_color)?;
            style.decoration.shadows = std::sync::Arc::new(style.shadows.layers());
            Ok(())
        }
    };
}
shadow_list!(
    box_shadow_color,
    colors,
    |value: &Component, style: &Style, relative: &Relative<'_>| {
        crate::style::shadow_values::colors(value, relative, &style.computed_color)
    }
);
shadow_list!(
    box_shadow_offset,
    offsets,
    |value: &Component, _: &Style, relative: &Relative<'_>| {
        crate::style::shadow_values::offsets(value, relative)
    }
);
shadow_list!(
    box_shadow_blur,
    blurs,
    |value: &Component, _: &Style, relative: &Relative<'_>| {
        crate::style::shadow_values::lengths(value, relative)
    }
);
shadow_list!(
    box_shadow_spread,
    spreads,
    |value: &Component, _: &Style, relative: &Relative<'_>| {
        crate::style::shadow_values::lengths(value, relative)
    }
);
shadow_list!(
    box_shadow_position,
    positions,
    |value: &Component, _: &Style, _: &Relative<'_>| {
        crate::style::shadow_values::positions(value)
    }
);
fn color_check(value: &Component, relative: &Relative<'_>) -> bool {
    crate::style::colors::color(value, relative, &crate::paint::AbsoluteColor::BLACK).is_ok()
}
/// An inherited `<color>` keeping `currentcolor` for each inheriting element to resolve.
macro_rules! inherited_color {
    ($name:ident, $field:ident) => {
        fn $name(
            style: &mut Style,
            _parent: &Style,
            input: Input<'_>,
            relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            style.text_paint.$field = match input {
                Input::Value(value) => {
                    crate::style::InheritedColor::compute(value, relative, &style.computed_color)?
                }
                Input::Copy(source) => {
                    let mut color = source.text_paint.$field.clone();
                    color.resolve(relative, &style.computed_color)?;
                    color
                }
            };
            Ok(())
        }
    };
}
inherited_color!(text_fill_color, fill);
inherited_color!(text_stroke_color, stroke_color);
inherited_color!(text_emphasis_color, emphasis_color);
fn text_shadow(
    style: &mut Style,
    _parent: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    style.text_paint.shadows = match input {
        Input::Value(value) => {
            crate::style::text_paint_values::text_shadows(value, relative, &style.computed_color)?
        }
        Input::Copy(source) => {
            let mut shadows = source.text_paint.shadows.clone();
            crate::style::text_paint_values::resolve_shadows(
                &mut shadows,
                relative,
                &style.computed_color,
            )?;
            shadows
        }
    };
    Ok(())
}
computed!(
    text_stroke_width,
    [text_paint.stroke_width],
    |value, style, _parent, relative| crate::style::text_paint_values::stroke_width(
        value, relative
    )?
);
computed!(
    text_emphasis_style,
    [text_paint.emphasis],
    |value, style, _parent, _relative| crate::style::EmphasisStyle::from_component(value)?
);
computed!(
    text_emphasis_skip,
    [text_paint.emphasis_skip],
    |value, style, _parent, _relative| crate::style::EmphasisSkip::from_component(value)?
);
computed!(
    text_emphasis_position,
    [text_paint.emphasis_position],
    |value, style, _parent, _relative| crate::style::EmphasisPosition::from_component(value)?
);
computed!(
    text_decoration_line,
    [text_decorations.declared.lines],
    |value, style, _parent, _relative| crate::style::DecorationLines::from_component(value)?
);
computed!(
    text_decoration_style,
    [text_decorations.declared.style],
    |value, style, _parent, _relative| crate::style::decoration_values::decoration_style(value)?
);
computed!(
    text_decoration_color,
    [text_decorations.declared.color],
    |value, style, _parent, relative| crate::style::colors::color(
        value,
        relative,
        &style.computed_color
    )?
);
computed!(
    text_decoration_thickness,
    [text_decorations.declared.thickness],
    |value, style, _parent, relative| crate::style::decoration_values::thickness(value, relative)?
);
computed!(
    text_underline_position,
    [text_decorations.underline_position],
    |value, style, _parent, _relative| crate::style::UnderlinePosition::from_component(value)
);
computed!(
    text_underline_offset,
    [text_decorations.underline_offset],
    |value, style, _parent, relative| crate::style::decoration_values::underline_offset(
        value, relative
    )?
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
    |value, style, _parent, _relative| { crate::style::TextOverflow::from_component(value)? }
);
computed!(
    background_color,
    [decoration.background],
    |value, style, _parent, relative| {
        crate::style::colors::color(value, relative, &style.computed_color)?
    }
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
    crate::style::background_values::images(value, relative, &style.computed_color)?
});
/// `background-position-x` or `-y`, the list at `axis` of the layer positions.
fn background_position_axis(
    axis: usize,
    style: &mut Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    let computed = match input {
        Input::Value(value) => {
            crate::style::background_values::axis_positions(value, relative, style.rtl)?
        }
        Input::Copy(source) => source.decoration.backgrounds.positions[axis].clone(),
    };
    style.backgrounds_mut().positions[axis] = computed;
    Ok(())
}
fn background_position_x(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    background_position_axis(POSITION_X, style, input, relative)
}
fn background_position_y(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    background_position_axis(POSITION_Y, style, input, relative)
}
background!(background_size, sizes, |value, style, relative| {
    crate::style::background_values::sizes(value, relative)?
});
background!(background_repeat, repeats, |value, style, _relative| {
    crate::style::background_values::repeats(value)?
});
background!(background_origin, origins, |value, style, _relative| {
    crate::style::background_values::boxes(value)?
});
background!(background_clip, clips, |value, style, _relative| {
    crate::style::background_values::boxes(value)?
});
background!(
    background_attachment,
    attachments,
    |value, style, _relative| { crate::style::background_values::attachments(value)? }
);
background!(
    background_blend_mode,
    blend_modes,
    |value, style, _relative| { crate::style::compositing_values::blend_modes(value)? }
);

/// A `border-image-*` longhand, shared copy-on-write between styles.
/// A `border-image-*` or `mask-border-*` longhand of the border image `$image` returns.
macro_rules! border_image {
    ($name:ident, $image:ident, $image_mut:ident, [$($field:ident),+], |$value:ident, $style:ident, $relative:ident| $compute:expr) => {
        fn $name(
            $style: &mut Style,
            _: &Style,
            input: Input<'_>,
            $relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            let computed = match input {
                Input::Value($value) => $compute,
                Input::Copy(source) => {
                    let image = source.$image();
                    ($(image.$field.clone()),+)
                }
            };
            let image = $style.$image_mut();
            ($(image.$field),+) = computed;
            Ok(())
        }
    };
}
border_image!(
    border_image_source,
    border_image,
    border_image_mut,
    [source],
    |value, style, relative| {
        crate::style::background_values::image(value, relative, &style.computed_color)?
    }
);
border_image!(
    border_image_slice,
    border_image,
    border_image_mut,
    [slice, fill],
    |value, style, relative| { crate::style::border_image_values::slice(value, relative)? }
);
border_image!(
    border_image_width,
    border_image,
    border_image_mut,
    [widths],
    |value, style, relative| { crate::style::border_image_values::widths(value, relative)? }
);
border_image!(
    border_image_outset,
    border_image,
    border_image_mut,
    [outset],
    |value, style, relative| { crate::style::border_image_values::outset(value, relative)? }
);
border_image!(
    border_image_repeat,
    border_image,
    border_image_mut,
    [repeat],
    |value, style, _relative| { crate::style::border_image_values::repeat(value)? }
);
// CSS Masking 1 §8: the mask border image shares the border image grammar.
border_image!(
    mask_border_source,
    mask_border,
    mask_border_mut,
    [source],
    |value, style, relative| {
        crate::style::background_values::image(value, relative, &style.computed_color)?
    }
);
border_image!(
    mask_border_slice,
    mask_border,
    mask_border_mut,
    [slice, fill],
    |value, style, relative| { crate::style::border_image_values::slice(value, relative)? }
);
border_image!(
    mask_border_width,
    mask_border,
    mask_border_mut,
    [widths],
    |value, style, relative| { crate::style::border_image_values::widths(value, relative)? }
);
border_image!(
    mask_border_outset,
    mask_border,
    mask_border_mut,
    [outset],
    |value, style, relative| { crate::style::border_image_values::outset(value, relative)? }
);
border_image!(
    mask_border_repeat,
    mask_border,
    mask_border_mut,
    [repeat],
    |value, style, _relative| { crate::style::border_image_values::repeat(value)? }
);
fn mask_border_mode(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    _: &Relative<'_>,
) -> Result<(), StyleError> {
    style.masks_mut().border_mode = match input {
        Input::Value(value) => crate::style::mask_values::border_mode(value)?,
        Input::Copy(source) => source.decoration.masks.border_mode,
    };
    Ok(())
}

/// `<line-width>` converted to absolute points.
fn border_width(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    let width = match value.keyword() {
        Some(Keyword::Thin) => THIN_BORDER_PT,
        Some(Keyword::Medium) => DEFAULT_BORDER_WIDTH,
        Some(Keyword::Thick) => THICK_BORDER_PT,
        _ => crate::style::values::length(value, relative)?,
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
/// CSS UI 4 §3.2: the computed width is zero when `outline-style` is `none`.
fn outline_width(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    let outline = &mut style.decoration.outline;
    outline.width = match input {
        Input::Value(value) => border_width(value, relative)?,
        Input::Copy(source) => source.decoration.outline.computed_width(),
    };
    outline.width = outline.computed_width();
    Ok(())
}
/// `auto | <outline-line-style>`: the border line styles without `hidden`.
fn outline_style_value(value: &Component) -> Result<OutlineStyle, StyleError> {
    match value.keyword() {
        Some(Keyword::Auto) => Ok(OutlineStyle::Auto),
        Some(Keyword::Hidden) => Err(unsupported(value)),
        _ => border_style(value).map(OutlineStyle::Line),
    }
}
computed!(
    outline_style,
    [decoration.outline.style],
    |value, style, _parent, _relative| outline_style_value(value)?
);
// CSS UI 4 §3.4: `auto` is the accent colour with `auto` style and otherwise `currentcolor`.
computed!(
    outline_color,
    [decoration.outline.color],
    |value, style, _parent, relative| match value.keyword() {
        Some(Keyword::Auto) if style.decoration.outline.style == OutlineStyle::Auto => {
            crate::style::colors::accent(relative)
        }
        Some(Keyword::Auto) => style.color,
        _ => crate::style::colors::color(value, relative, &style.computed_color)?,
    }
);
computed!(
    outline_offset,
    [decoration.outline.offset],
    |value, style, _parent, relative| crate::style::values::length(value, relative)?
);
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
                crate::style::colors::color(value, relative, &style.computed_color)?
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
pub(super) fn corner_radius(
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
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    style.computed_content = match input {
        Input::Value(value) => crate::style::generated::ComputedContent::from_component(
            value,
            style.content_context,
            &element_scope(relative, style),
        )?,
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
    crate::style::values::integer(value, relative)?
});
computed!(
    justify_content,
    [flex.justify],
    |value, style, _parent, _relative| { Alignment::from_component(value)? }
);
computed!(
    appearance,
    [widget.appearance],
    |value, style, _parent, _relative| { crate::style::Appearance::from_component(value)? }
);
computed!(
    field_sizing,
    [widget.field_sizing],
    |value, style, _parent, _relative| { crate::style::FieldSizing::from_component(value)? }
);
computed!(
    align_items,
    [flex.align_items],
    |value, style, _parent, _relative| { Alignment::from_component(value)? }
);
fn align_content(
    style: &mut Style,
    _: &Style,
    input: Input<'_>,
    _: &Relative<'_>,
) -> Result<(), StyleError> {
    (style.flex.align_content, style.flex.align_content_unsafe) = match input {
        Input::Value(value) => Alignment::content_from_component(value)?,
        Input::Copy(source) => (source.flex.align_content, source.flex.align_content_unsafe),
    };
    Ok(())
}
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
    [grid.templates[crate::style::ROWS]],
    |value, style, _parent, relative| std::sync::Arc::new(grid_values::template(value, relative)?)
);
computed!(
    grid_template_columns,
    [grid.templates[crate::style::COLUMNS]],
    |value, style, _parent, relative| std::sync::Arc::new(grid_values::template(value, relative)?)
);
computed!(
    grid_template_areas,
    [grid.areas],
    |value, style, _parent, _relative| { grid_values::areas(value)? }
);
computed!(
    grid_auto_rows,
    [grid.auto_tracks[crate::style::ROWS]],
    |value, style, _parent, relative| grid_values::auto_tracks(value, relative)?
);
computed!(
    grid_auto_columns,
    [grid.auto_tracks[crate::style::COLUMNS]],
    |value, style, _parent, relative| grid_values::auto_tracks(value, relative)?
);
computed!(
    grid_auto_flow,
    [grid.auto_flow],
    |value, style, _parent, _relative| { grid_values::auto_flow(value) }
);
computed!(
    grid_row_start,
    [grid.placement[crate::style::ROWS][0]],
    |value, style, _parent, relative| grid_values::line(value, relative)?
);
computed!(
    grid_row_end,
    [grid.placement[crate::style::ROWS][1]],
    |value, style, _parent, relative| grid_values::line(value, relative)?
);
computed!(
    grid_column_start,
    [grid.placement[crate::style::COLUMNS][0]],
    |value, style, _parent, relative| grid_values::line(value, relative)?
);
computed!(
    grid_column_end,
    [grid.placement[crate::style::COLUMNS][1]],
    |value, style, _parent, relative| grid_values::line(value, relative)?
);
computed!(
    column_count,
    [multicol.count],
    |value, style, _parent, relative| { crate::style::multicol_values::count(value, relative)? }
);
computed!(
    column_width,
    [multicol.width],
    |value, style, _parent, relative| { crate::style::multicol_values::width(value, relative)? }
);
computed!(
    column_fill,
    [multicol.fill],
    |value, style, _parent, _relative| { crate::style::ColumnFill::from_component(value)? }
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
    |value, style, _parent, relative| {
        crate::style::colors::color(value, relative, &style.computed_color)?
    }
);

computed!(
    table_layout,
    [table.layout],
    |value, style, _parent, _relative| { crate::style::TableLayout::from_component(value)? }
);
computed!(
    border_collapse,
    [table.collapse],
    |value, style, _parent, _relative| { crate::style::BorderCollapse::from_component(value)? }
);
computed!(
    border_spacing,
    [table.spacing],
    |value, style, _parent, relative| { crate::style::table_values::spacing(value, relative)? }
);
computed!(
    empty_cells,
    [table.empty_cells],
    |value, style, _parent, _relative| { crate::style::table_values::empty_cells(value)? }
);
computed!(
    caption_side,
    [table.caption_side],
    |value, style, _parent, _relative| { crate::style::CaptionSide::from_component(value)? }
);
computed!(
    vertical_align,
    [table.vertical_align],
    |value, style, _parent, relative| {
        crate::style::VerticalAlign::from_component(value, relative)?
    }
);
computed!(
    text_box_trim,
    [text_box.trim],
    |value, style, _parent, _relative| crate::style::TextBox::trim(value)?
);
computed!(
    text_box_edge,
    [text_box.edges],
    |value, style, _parent, _relative| crate::style::TextBox::edges(value)?
);
computed!(
    initial_letter,
    [initial_letter],
    |value, style, _parent, relative| crate::style::InitialLetter::from_component(value, relative)?
);
computed!(
    initial_letter_align,
    [initial_letter_align],
    |value, style, _parent, _relative| crate::style::InitialLetterAlign::from_component(value)?
);
computed!(
    ruby_position,
    [ruby.position],
    |value, style, _parent, _relative| crate::style::RubyStyle::position(value)?
);
computed!(
    ruby_merge,
    [ruby.merge],
    |value, style, _parent, _relative| crate::style::RubyStyle::merge(value)?
);
computed!(
    ruby_align,
    [ruby.align],
    |value, style, _parent, _relative| crate::style::RubyStyle::align(value)?
);
computed!(
    ruby_overhang,
    [ruby.overhang],
    |value, style, _parent, _relative| crate::style::RubyStyle::overhang(value)?
);
computed!(
    dominant_baseline,
    [baseline.dominant],
    |value, style, _parent, _relative| crate::style::Baseline::dominant(value)?
);
computed!(
    alignment_baseline,
    [baseline.alignment],
    |value, style, _parent, _relative| crate::style::Baseline::alignment(value)?
);
computed!(
    baseline_shift,
    [baseline.shift],
    |value, style, _parent, relative| crate::style::BaselineShift::from_component(value, relative)?
);
computed!(
    baseline_source,
    [baseline.source],
    |value, style, _parent, _relative| crate::style::BaselineSource::from_component(value)?
);
computed!(
    table_column_span,
    [table.column_span],
    |value, style, _parent, relative| { crate::style::table_values::span(value, relative)? }
);
computed!(
    table_row_span,
    [table.row_span],
    |value, style, _parent, relative| { crate::style::table_values::span(value, relative)? }
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
    |value, style, _parent, relative| {
        crate::style::GeneratedContent::from_list(value, &element_scope(relative, style))?
    }
);
computed!(
    bookmark_state,
    [bookmark.open],
    |value, style, _parent, _relative| {
        flag(value, &[(Keyword::Open, true), (Keyword::Closed, false)])?
    }
);
computed!(
    will_change,
    [will_change],
    |value, style, _parent, _relative| {
        crate::style::will_change::WillChange::from_component(value)?
    }
);
computed!(
    view_transition_name,
    [view_transition_named],
    |value, style, _parent, _relative| { value.keyword() != Some(Keyword::None) }
);

computed!(
    counter_reset,
    [counters.reset],
    |value, style, _parent, relative| { crate::style::counter_values::resets(value, relative)? }
);

computed!(
    counter_increment,
    [counters.increment],
    |value, style, _parent, relative| {
        crate::style::counter_values::changes(
            value,
            relative,
            crate::style::DEFAULT_COUNTER_INCREMENT,
        )?
    }
);

computed!(
    counter_set,
    [counters.set],
    |value, style, _parent, relative| {
        crate::style::counter_values::changes(value, relative, crate::style::INITIAL_COUNTER_VALUE)?
    }
);

computed!(
    string_set,
    [string_set],
    |value, style, _parent, relative| {
        crate::style::string_values::assignments(value, &element_scope(relative, style))?
    }
);

fn math_compact(value: &Component) -> Result<bool, StyleError> {
    flag(value, &[(Keyword::Normal, false), (Keyword::Compact, true)])
}
computed!(
    math_style,
    [math.compact],
    |value, style, _parent, _relative| { math_compact(value)? }
);
computed!(
    math_shift,
    [math.shift],
    |value, style, _parent, _relative| { math_compact(value)? }
);
computed!(
    math_depth,
    [math.depth],
    |value, style, parent, relative| { crate::style::math_values::depth(value, parent, relative)? }
);
