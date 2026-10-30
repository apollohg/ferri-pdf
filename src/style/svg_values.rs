//! SVG 2 presentation properties: painting (§13), paint servers (§14), geometry (§7), paths
//! (§9.3), masking and filter primitives. Inline SVG content takes their computed values from
//! the document's cascade (SVG 2 §6.4). Every one applies only to SVG elements (shapes, text
//! content, containers, gradients, stops, masks or filter primitives, per each property's
//! "Applies to"), so they have no effect on boxes of other namespaces.
use crate::paint::{AbsoluteColor, LengthPercentage};
use crate::style::{
    InheritedColor, Style, StyleError, WhiteSpaceCollapse,
    box_values::{length_percentage, unsupported},
    cascade::Origin,
    component::Component,
    declaration::{self, PropertyDeclaration, Specified},
    grammar::{Keyword, Property},
    properties::Input,
    units::{Axis, Relative},
    values::number,
    wide_keywords::WideKeyword,
};
use std::sync::Arc;

/// A computed `<paint>` (SVG 2 §13.2): colours computed, references kept.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Paint {
    None,
    Color(InheritedColor),
    /// A paint server, with the fallback used when the reference is invalid.
    Server {
        url: Box<str>,
        fallback: Option<PaintFallback>,
    },
    ContextFill,
    ContextStroke,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaintFallback {
    None,
    Color(InheritedColor),
}

/// One of the three painting operations `paint-order` orders (SVG 2 §13.8).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaintLayer {
    Fill,
    Stroke,
    Markers,
}
impl PaintLayer {
    fn from_keyword(keyword: Keyword) -> Option<Self> {
        match keyword {
            Keyword::Fill => Some(Self::Fill),
            Keyword::Stroke => Some(Self::Stroke),
            Keyword::Markers => Some(Self::Markers),
            _ => None,
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Fill => Keyword::Fill.name(),
            Self::Stroke => Keyword::Stroke.name(),
            Self::Markers => Keyword::Markers.name(),
        }
    }
}

/// The computed SVG presentation properties of one element.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SvgStyle {
    pub fill: Paint,
    pub fill_opacity: f32,
    pub fill_rule: Keyword,
    pub stroke: Paint,
    /// Fill and Stroke 3 §4.4.1 `stroke-color`, drawn where `stroke` paints nothing: it is
    /// `none`, or a reference without a fallback that names no paint server.
    pub stroke_color: InheritedColor,
    pub stroke_opacity: f32,
    /// Points, or a percentage of the normalized viewport diagonal.
    pub stroke_width: LengthPercentage,
    /// The stroke width of CSS text in points: a percentage refers to the scaled viewport size,
    /// the geometric mean of the viewport's width and height (Fill and Stroke 3 §4.2.1).
    pub text_stroke_width: f32,
    /// `None` is `none`.
    pub stroke_dasharray: Option<Arc<[LengthPercentage]>>,
    pub stroke_dashoffset: LengthPercentage,
    pub stroke_linecap: Keyword,
    pub stroke_linejoin: Keyword,
    pub stroke_miterlimit: f32,
    /// `marker-start`, `marker-mid` and `marker-end`; `None` is `none`.
    pub markers: [Option<Box<str>>; 3],
    /// The operations painted first, as specified; omitted ones follow in their normal order,
    /// and none is `normal`.
    pub paint_order: Box<[PaintLayer]>,
    pub stop_color: InheritedColor,
    pub stop_opacity: f32,
    pub flood_color: InheritedColor,
    pub flood_opacity: f32,
    pub lighting_color: InheritedColor,
    pub clip_rule: Keyword,
    pub mask_type: Keyword,
    pub color_interpolation: Keyword,
    pub color_interpolation_filters: Keyword,
    pub text_anchor: Keyword,
    pub shape_rendering: Keyword,
    /// SVG 2 §8.13 `vector-effect`.
    pub vector_effect: Keyword,
    pub text_rendering: Keyword,
    pub image_rendering: Keyword,
    pub cx: LengthPercentage,
    pub cy: LengthPercentage,
    pub r: LengthPercentage,
    /// `None` is `auto`.
    pub rx: Option<LengthPercentage>,
    pub ry: Option<LengthPercentage>,
    pub x: LengthPercentage,
    pub y: LengthPercentage,
    /// Path data; `None` is `none`.
    pub d: Option<Arc<str>>,
}

impl Default for SvgStyle {
    fn default() -> Self {
        let opaque = 1.0;
        let color = InheritedColor::specified;
        Self {
            fill: Paint::Color(color(crate::paint::Color::BLACK)),
            fill_opacity: opaque,
            fill_rule: Keyword::Nonzero,
            stroke: Paint::None,
            stroke_color: color(crate::paint::Color::TRANSPARENT),
            stroke_opacity: opaque,
            stroke_width: LengthPercentage::Length(crate::style::POINTS_PER_PIXEL),
            text_stroke_width: crate::style::POINTS_PER_PIXEL,
            stroke_dasharray: None,
            stroke_dashoffset: LengthPercentage::ZERO,
            stroke_linecap: Keyword::Butt,
            stroke_linejoin: Keyword::Miter,
            stroke_miterlimit: DEFAULT_MITER_LIMIT,
            markers: [None, None, None],
            paint_order: Box::default(),
            stop_color: color(crate::paint::Color::BLACK),
            stop_opacity: opaque,
            flood_color: color(crate::paint::Color::BLACK),
            flood_opacity: opaque,
            lighting_color: color(crate::paint::Color::WHITE),
            clip_rule: Keyword::Nonzero,
            mask_type: Keyword::Luminance,
            color_interpolation: Keyword::Srgb,
            color_interpolation_filters: Keyword::Linearrgb,
            text_anchor: Keyword::Start,
            shape_rendering: Keyword::Auto,
            vector_effect: Keyword::None,
            text_rendering: Keyword::Auto,
            image_rendering: Keyword::Auto,
            cx: LengthPercentage::ZERO,
            cy: LengthPercentage::ZERO,
            r: LengthPercentage::ZERO,
            rx: None,
            ry: None,
            x: LengthPercentage::ZERO,
            y: LengthPercentage::ZERO,
            d: None,
        }
    }
}

/// SVG 2 §13.5.5 initial `stroke-miterlimit`.
const DEFAULT_MITER_LIMIT: f32 = 4.0;

impl SvgStyle {
    /// Re-resolves `currentcolor`-dependent colours against the element's own colour.
    pub(super) fn resolve(
        style: &mut Arc<Self>,
        relative: &Relative<'_>,
        current: &AbsoluteColor,
    ) -> Result<(), StyleError> {
        let dependent = [&style.fill, &style.stroke]
            .into_iter()
            .any(Paint::depends_on_color)
            || [
                &style.stroke_color,
                &style.stop_color,
                &style.flood_color,
                &style.lighting_color,
            ]
            .into_iter()
            .any(InheritedColor::depends_on_color);
        if !dependent {
            return Ok(());
        }
        let style = Arc::make_mut(style);
        style.fill.resolve(relative, current)?;
        style.stroke.resolve(relative, current)?;
        for color in [
            &mut style.stroke_color,
            &mut style.stop_color,
            &mut style.flood_color,
            &mut style.lighting_color,
        ] {
            color.resolve(relative, current)?;
        }
        Ok(())
    }
}

impl Paint {
    fn compute(
        value: &Component,
        relative: &Relative<'_>,
        current: &AbsoluteColor,
    ) -> Result<Self, StyleError> {
        let items = value.items();
        match (items.first().map(Component::unwrap), items.get(1)) {
            (Some(Component::Url(url)), fallback) => Ok(Self::Server {
                url: url.as_ref().into(),
                fallback: fallback
                    .and_then(|fallback| fallback.items().first())
                    .map(|fallback| match fallback.keyword() {
                        Some(Keyword::None) => Ok(PaintFallback::None),
                        _ => InheritedColor::compute(fallback, relative, current)
                            .map(PaintFallback::Color),
                    })
                    .transpose()?,
            }),
            _ => Ok(match value.keyword() {
                Some(Keyword::None) => Self::None,
                Some(Keyword::ContextFill) => Self::ContextFill,
                Some(Keyword::ContextStroke) => Self::ContextStroke,
                _ => Self::Color(InheritedColor::compute(value, relative, current)?),
            }),
        }
    }
    fn depends_on_color(&self) -> bool {
        match self {
            Self::Color(color)
            | Self::Server {
                fallback: Some(PaintFallback::Color(color)),
                ..
            } => color.depends_on_color(),
            _ => false,
        }
    }
    fn resolve(
        &mut self,
        relative: &Relative<'_>,
        current: &AbsoluteColor,
    ) -> Result<(), StyleError> {
        match self {
            Self::Color(color)
            | Self::Server {
                fallback: Some(PaintFallback::Color(color)),
                ..
            } => color.resolve(relative, current),
            _ => Ok(()),
        }
    }
}

/// `<length-percentage> | <number>`: a number is a length in user units, which are CSS pixels.
pub(super) fn user_length(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<LengthPercentage, StyleError> {
    match value.unwrap() {
        Component::Number(_) | Component::Math(_, crate::style::calc::Kind::Number, _) => Ok(
            LengthPercentage::Length(number(value, relative)? * crate::style::POINTS_PER_PIXEL),
        ),
        _ => length_percentage(value, relative),
    }
}

/// SVG 2 §7: a negative `r`, `rx` or `ry` is treated as the initial value.
fn non_negative(value: LengthPercentage) -> Option<LengthPercentage> {
    match value {
        LengthPercentage::Length(length) | LengthPercentage::Percentage(length) if length < 0.0 => {
            None
        }
        value => Some(value),
    }
}

/// Whether the handler computes `value`, for `@supports`.
pub(super) fn supported(
    apply: crate::style::properties::Apply,
    value: &Component,
    relative: &Relative<'_>,
) -> bool {
    apply(
        &mut Style::default(),
        &Style::default(),
        Input::Value(value),
        relative,
    )
    .is_ok()
}

/// Values the grammar matches that the property's prose excludes.
pub(super) fn valid(property: Property, value: &Component) -> bool {
    if property != Property::PaintOrder {
        return true;
    }
    let mut keywords = Vec::new();
    value.keywords(&mut keywords);
    keywords
        .iter()
        .enumerate()
        .all(|(index, keyword)| !keywords[..index].contains(keyword))
}

/// Handlers set one field, and skip the copy-on-write when an inherited value is unchanged.
macro_rules! svg_property {
    ($name:ident, $field:ident, |$value:ident, $style:ident, $relative:ident| $compute:expr) => {
        pub(super) fn $name(
            style: &mut Style,
            _parent: &Style,
            input: Input<'_>,
            relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            let computed = match input {
                Input::Value($value) => {
                    let $style: &Style = style;
                    let $relative = relative;
                    $compute
                }
                Input::Copy(source) => source.svg.$field.clone(),
            };
            if style.svg.$field != computed {
                Arc::make_mut(&mut style.svg).$field = computed;
            }
            Ok(())
        }
    };
}

/// A keyword property computes as specified.
macro_rules! svg_keyword {
    ($name:ident, $field:ident) => {
        svg_property!($name, $field, |value, _style, _relative| value
            .keyword()
            .ok_or_else(|| unsupported(value))?);
    };
}

/// A colour-valued property whose `currentcolor` each element resolves against its own colour.
macro_rules! svg_color {
    ($name:ident, $field:ident, $compute:path) => {
        pub(super) fn $name(
            style: &mut Style,
            _parent: &Style,
            input: Input<'_>,
            relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            let computed = match input {
                Input::Value(value) => $compute(value, relative, &style.computed_color)?,
                Input::Copy(source) => {
                    let mut computed = source.svg.$field.clone();
                    computed.resolve(relative, &style.computed_color)?;
                    computed
                }
            };
            if style.svg.$field != computed {
                Arc::make_mut(&mut style.svg).$field = computed;
            }
            Ok(())
        }
    };
}

svg_color!(fill, fill, Paint::compute);
svg_color!(stroke, stroke, Paint::compute);
svg_color!(stroke_color, stroke_color, InheritedColor::compute);
svg_color!(stop_color, stop_color, InheritedColor::compute);
svg_color!(flood_color, flood_color, InheritedColor::compute);
svg_color!(lighting_color, lighting_color, InheritedColor::compute);
svg_property!(fill_opacity, fill_opacity, |value, _style, relative| {
    crate::style::properties::opacity_value(value, relative)?
});
svg_property!(stroke_opacity, stroke_opacity, |value, _style, relative| {
    crate::style::properties::opacity_value(value, relative)?
});
svg_property!(stop_opacity, stop_opacity, |value, _style, relative| {
    crate::style::properties::opacity_value(value, relative)?
});
svg_property!(flood_opacity, flood_opacity, |value, _style, relative| {
    crate::style::properties::opacity_value(value, relative)?
});
pub(super) fn stroke_width(
    style: &mut Style,
    _parent: &Style,
    input: Input<'_>,
    relative: &Relative<'_>,
) -> Result<(), StyleError> {
    let (width, text) = match input {
        Input::Value(value) => {
            let width = user_length(value, relative)?;
            let text = if let LengthPercentage::Length(points) = width {
                points
            } else {
                let size = relative.viewport.dimension(Axis::Width)?
                    * relative.viewport.dimension(Axis::Height)?;
                width.resolve(size.sqrt())
            };
            (width, text)
        }
        Input::Copy(source) => (
            source.svg.stroke_width.clone(),
            source.svg.text_stroke_width,
        ),
    };
    if style.svg.stroke_width != width || style.svg.text_stroke_width != text {
        let svg = Arc::make_mut(&mut style.svg);
        svg.stroke_width = width;
        svg.text_stroke_width = text;
    }
    Ok(())
}
svg_property!(
    stroke_dashoffset,
    stroke_dashoffset,
    |value, _style, relative| user_length(value, relative)?
);
svg_property!(
    stroke_dasharray,
    stroke_dasharray,
    |value, _style, relative| {
        if value.keyword() == Some(Keyword::None) {
            None
        } else {
            Some(
                value
                    .items()
                    .iter()
                    .flat_map(Component::items)
                    .map(|length| user_length(length, relative))
                    .collect::<Result<Arc<[_]>, _>>()?,
            )
        }
    }
);
svg_property!(
    stroke_miterlimit,
    stroke_miterlimit,
    |value, _style, relative| number(value, relative)?
);
svg_property!(paint_order, paint_order, |value, _style, _relative| {
    let mut keywords = Vec::new();
    value.keywords(&mut keywords);
    keywords
        .into_iter()
        .filter_map(PaintLayer::from_keyword)
        .collect()
});
// §13.9: `auto` leaves the colour space to the user agent, which keeps sRGB.
svg_keyword!(color_interpolation, color_interpolation);
svg_keyword!(fill_rule, fill_rule);
svg_keyword!(clip_rule, clip_rule);
svg_keyword!(stroke_linecap, stroke_linecap);
svg_keyword!(stroke_linejoin, stroke_linejoin);
svg_keyword!(mask_type, mask_type);
svg_keyword!(color_interpolation_filters, color_interpolation_filters);
svg_keyword!(text_anchor, text_anchor);
svg_keyword!(vector_effect, vector_effect);
// SVG 2 §13.10 rendering hints: vector output carries no anti-aliasing or legibility trade-off,
// which the PDF consumer makes.
svg_keyword!(shape_rendering, shape_rendering);
svg_keyword!(text_rendering, text_rendering);
// CSS Images 3 §5.3: images are drawn without interpolation, as `pixelated`, `crisp-edges`
// and `optimizeSpeed` require; smoothing for the other values is a recommendation.
svg_keyword!(image_rendering, image_rendering);

/// `marker-start`, `marker-mid` and `marker-end` by index.
macro_rules! svg_marker {
    ($name:ident, $index:literal) => {
        pub(super) fn $name(
            style: &mut Style,
            _parent: &Style,
            input: Input<'_>,
            _relative: &Relative<'_>,
        ) -> Result<(), StyleError> {
            let computed = match input {
                Input::Value(value) => match value.unwrap() {
                    Component::Url(url) => Some(url.as_ref().into()),
                    _ => None,
                },
                Input::Copy(source) => source.svg.markers[$index].clone(),
            };
            if style.svg.markers[$index] != computed {
                Arc::make_mut(&mut style.svg).markers[$index] = computed;
            }
            Ok(())
        }
    };
}
svg_marker!(marker_start, 0);
svg_marker!(marker_mid, 1);
svg_marker!(marker_end, 2);

svg_property!(cx, cx, |value, _style, relative| length_percentage(
    value, relative
)?);
svg_property!(cy, cy, |value, _style, relative| length_percentage(
    value, relative
)?);
svg_property!(x, x, |value, _style, relative| length_percentage(
    value, relative
)?);
svg_property!(y, y, |value, _style, relative| length_percentage(
    value, relative
)?);
svg_property!(r, r, |value, _style, relative| {
    non_negative(length_percentage(value, relative)?).unwrap_or(LengthPercentage::ZERO)
});
svg_property!(rx, rx, |value, _style, relative| match value.keyword() {
    Some(Keyword::Auto) => None,
    _ => non_negative(length_percentage(value, relative)?),
});
svg_property!(ry, ry, |value, _style, relative| match value.keyword() {
    Some(Keyword::Auto) => None,
    _ => non_negative(length_percentage(value, relative)?),
});
svg_property!(d, d, |value, _style, _relative| match value.unwrap() {
    Component::String(path) => Some(Arc::from(&**path)),
    _ => None,
});

/// The SVG spelling of keywords that SVG attribute syntax writes in camel case.
pub(crate) fn svg_keyword(keyword: Keyword) -> &'static str {
    match keyword {
        Keyword::Srgb => "sRGB",
        Keyword::Linearrgb => "linearRGB",
        Keyword::Optimizespeed => "optimizeSpeed",
        Keyword::Optimizequality => "optimizeQuality",
        Keyword::Optimizelegibility => "optimizeLegibility",
        Keyword::Geometricprecision => "geometricPrecision",
        Keyword::Crispedges => "crispEdges",
        other => other.name(),
    }
}

/// An `image-rendering` keyword in CSS Images 3 terms: §5.3 has `optimizeSpeed` behave as
/// `crisp-edges` and `optimizeQuality` as `smooth`.
pub(crate) fn image_rendering_keyword(value: Keyword) -> &'static str {
    match value {
        Keyword::Optimizespeed => Keyword::CrispEdges.name(),
        Keyword::Optimizequality => Keyword::Smooth.name(),
        other => other.name(),
    }
}

/// Properties applying to SVG elements that their renderer does not take from the cascade:
/// their presentation attributes reach it directly, so only initial values are supported.
const UNCASCADED: [Property; 4] = [
    Property::BaselineShift,
    Property::AlignmentBaseline,
    Property::TextDecorationLine,
    Property::UnicodeBidi,
];
/// Inherited properties of SVG text content that its renderer does not implement.
const UNCASCADED_TEXT: [Property; 4] = [
    Property::FontVariantEmoji,
    Property::FontPalette,
    Property::FontSynthesisWeight,
    Property::FontSynthesisStyle,
];

/// The initial computed values of the properties SVG content supports only at their initial
/// value.
pub(crate) struct InitialOnly(Vec<(Property, Style)>);
impl InitialOnly {
    pub(crate) fn new() -> Self {
        let initial = Style::default();
        Self(
            UNCASCADED
                .into_iter()
                .chain(UNCASCADED_TEXT)
                .map(|property| {
                    (
                        property,
                        crate::style::properties::projection(property, &initial),
                    )
                })
                .collect(),
        )
    }
    /// Fails on the first of those properties that `style` gives another value; inherited
    /// text properties only matter on text content elements.
    pub(crate) fn check(&self, style: &Style, text: bool) -> Result<(), StyleError> {
        let count = if text { self.0.len() } else { UNCASCADED.len() };
        match self.0[..count].iter().find(|(property, initial)| {
            crate::style::properties::projection(*property, style) != *initial
        }) {
            Some((property, _)) => Err(StyleError::Unsupported(format!(
                "{} other than its initial value on SVG content",
                property.name()
            ))),
            None => Ok(()),
        }
    }
}

/// SVG 2 §6.6 presentation attributes that the cascade takes on every SVG element. The SVG
/// renderer reads the others directly.
const PRESENTATION_ATTRIBUTES: [Property; 51] = [
    Property::ClipPath,
    Property::ClipRule,
    Property::Color,
    Property::ColorInterpolation,
    Property::ColorInterpolationFilters,
    Property::Direction,
    Property::Display,
    Property::DominantBaseline,
    Property::Fill,
    Property::FillOpacity,
    Property::FillRule,
    Property::Filter,
    Property::FloodColor,
    Property::FloodOpacity,
    Property::FontFamily,
    Property::FontSize,
    Property::FontStretch,
    Property::FontStyle,
    Property::FontVariant,
    Property::FontWeight,
    Property::GlyphOrientationVertical,
    Property::ImageRendering,
    Property::LetterSpacing,
    Property::LightingColor,
    Property::MarkerEnd,
    Property::MarkerMid,
    Property::MarkerStart,
    Property::Mask,
    Property::MaskType,
    Property::Opacity,
    Property::Overflow,
    Property::PaintOrder,
    Property::ShapeRendering,
    Property::StopColor,
    Property::StopOpacity,
    Property::Stroke,
    Property::StrokeDasharray,
    Property::StrokeDashoffset,
    Property::StrokeLinecap,
    Property::StrokeLinejoin,
    Property::StrokeMiterlimit,
    Property::StrokeOpacity,
    Property::StrokeWidth,
    Property::TextAnchor,
    Property::TextOrientation,
    Property::TextRendering,
    Property::TransformOrigin,
    Property::VectorEffect,
    Property::Visibility,
    Property::WordSpacing,
    Property::WritingMode,
];
const SIZED_ELEMENTS: &[&str] = &["foreignObject", "image", "rect", "svg", "symbol", "use"];
const CIRCULAR_ELEMENTS: &[&str] = &["circle", "ellipse"];
const ROUNDED_ELEMENTS: &[&str] = &["ellipse", "rect"];
/// §6.6: geometry properties have presentation attributes only on these elements.
const GEOMETRY_ATTRIBUTES: [(Property, &[&str]); 10] = [
    (Property::Cx, CIRCULAR_ELEMENTS),
    (Property::Cy, CIRCULAR_ELEMENTS),
    (Property::R, &["circle"]),
    (Property::Rx, ROUNDED_ELEMENTS),
    (Property::Ry, ROUNDED_ELEMENTS),
    (Property::D, &["path"]),
    (Property::X, SIZED_ELEMENTS),
    (Property::Y, SIZED_ELEMENTS),
    (Property::Width, SIZED_ELEMENTS),
    (Property::Height, SIZED_ELEMENTS),
];
/// §6.6: the `transform` property's presentation attributes, by element; any other element's
/// is `transform`.
const TRANSFORM_ATTRIBUTES: [(&str, &str); 3] = [
    ("linearGradient", "gradientTransform"),
    ("radialGradient", "gradientTransform"),
    ("pattern", "patternTransform"),
];
const TRANSFORM_ATTRIBUTE: &str = "transform";
/// SVG Animations elements, whose `fill` attribute is the animation's fill mode instead.
const ANIMATION_ELEMENTS: &[&str] = &["animate", "animateMotion", "animateTransform", "set"];

/// Uses the document's modern value and shorthand parsing for standalone SVG text too.
pub(crate) fn writing_attribute(name: &str, source: &str) -> Option<(&'static str, &'static str)> {
    use crate::style::{
        logical::{TextOrientation, WritingMode},
        wide_keywords::Resolved,
    };
    let (property, mapped, initial) = match name {
        "writing-mode" => (
            Property::WritingMode,
            "writing-mode",
            WritingMode::default().name(),
        ),
        "text-orientation" => (
            Property::TextOrientation,
            "text-orientation",
            TextOrientation::default().name(),
        ),
        "glyph-orientation-vertical" => (
            Property::GlyphOrientationVertical,
            "text-orientation",
            TextOrientation::default().name(),
        ),
        _ => return None,
    };
    let (_, value) = declaration::expand(property, source).ok()??.pop()?;
    let name = match value {
        Specified::Value(value) if mapped == "writing-mode" => {
            WritingMode::from_component(&value).ok()?.name()
        }
        Specified::Value(value) => TextOrientation::from_component(&value).ok()?.name(),
        Specified::Wide(wide @ (WideKeyword::Revert | WideKeyword::RevertLayer)) => wide.name(),
        Specified::Wide(wide) => match wide.resolve(true) {
            Resolved::Initial => initial,
            Resolved::Inherit => "inherit",
        },
        _ => return None,
    };
    Some((mapped, name))
}

/// The property that the attribute `name` of the SVG element `tag` is a presentation
/// attribute for, if the cascade takes it.
pub(crate) fn presentation_property(tag: &str, name: &str) -> Option<Property> {
    if name == transform_attribute(tag) {
        return Some(Property::Transform);
    }
    let property = Property::from_name(name)?;
    if let Some((_, elements)) = GEOMETRY_ATTRIBUTES
        .iter()
        .find(|(geometry, _)| *geometry == property)
    {
        return elements.contains(&tag).then_some(property);
    }
    (PRESENTATION_ATTRIBUTES.contains(&property)
        && !(property == Property::Fill && ANIMATION_ELEMENTS.contains(&tag)))
    .then_some(property)
}

/// The name of the `transform` presentation attribute of the SVG element `tag`.
pub(crate) fn transform_attribute(tag: &str) -> &'static str {
    TRANSFORM_ATTRIBUTES
        .iter()
        .find(|(element, _)| *element == tag)
        .map_or(TRANSFORM_ATTRIBUTE, |(_, name)| name)
}

/// Elements whose `overflow` the SVG 2 §6.8 user agent style sheet makes `hidden`; an `svg`
/// element only when nested in SVG content.
const CLIPPING_ELEMENTS: [&str; 4] = ["image", "marker", "pattern", "symbol"];
const SVG_ELEMENT: &str = "svg";
const FOREIGN_OBJECT_ELEMENT: &str = "foreignObject";

/// Whether the SVG renderer draws `element`, rather than the document laying it out as a box:
/// the outermost `svg` elements and those in a `foreignObject` are boxes.
fn is_svg_content(element: crate::dom::Element<'_>) -> bool {
    element.namespace() == crate::dom::SVG_NAMESPACE
        && element
            .parent(crate::dom::TreeRelation::Selector)
            .is_some_and(|parent| {
                parent.namespace() == crate::dom::SVG_NAMESPACE
                    && parent.name() != FOREIGN_OBJECT_ELEMENT
            })
}

/// `xml:space` (SVG 2 §11.10.3.2) and its value that preserves spaces.
const XML_SPACE: &str = "space";
const PRESERVE: &str = "preserve";

/// The `white-space-collapse` a `white-space` value sets, for standalone SVG text too; `None`
/// for `inherit` or an invalid value.
pub(crate) fn white_space_collapse(source: &str) -> Option<WhiteSpaceCollapse> {
    use crate::style::wide_keywords::Resolved;
    declaration::expand(Property::WhiteSpace, source)
        .ok()??
        .into_iter()
        .find(|(property, _)| *property == Property::WhiteSpaceCollapse)
        .and_then(|(_, value)| match value {
            Specified::Value(value) => WhiteSpaceCollapse::from_component(&value).ok(),
            Specified::Wide(wide) => match wide.resolve(true) {
                Resolved::Initial => Some(WhiteSpaceCollapse::default()),
                Resolved::Inherit => None,
            },
            _ => None,
        })
}

/// The SVG 2 §6.8 user agent style sheet's declarations for an SVG element: `overflow: hidden`
/// on viewport-establishing elements, `white-space-collapse: preserve-spaces` where
/// `xml:space="preserve"`, and `transform-origin: 0 0` on all but the outermost `svg` elements
/// and those in a `foreignObject`, which transform about their centre.
pub(super) fn user_agent_declarations(
    element: crate::dom::Element<'_>,
) -> Result<Vec<PropertyDeclaration>, StyleError> {
    let content = is_svg_content(element);
    let mut output = Vec::new();
    // Writing Modes 4 §3.2.1.2: legacy SVG attributes also establish a UA default.
    if let Some(value) = element.attr("writing-mode")
        && matches!(value, "lr" | "lr-tb" | "rl" | "rl-tb" | "tb" | "tb-rl")
    {
        declaration::append(&mut output, Origin::UserAgent, Property::WritingMode, value)?;
    }
    if element.value().attrs.iter().any(|(name, value)| {
        &*name.ns == crate::dom::XML_NAMESPACE && &*name.local == XML_SPACE && &**value == PRESERVE
    }) {
        declaration::append(
            &mut output,
            Origin::UserAgent,
            Property::WhiteSpaceCollapse,
            WhiteSpaceCollapse::PreserveSpaces.name(),
        )?;
    }
    if CLIPPING_ELEMENTS.contains(&element.name()) || (element.name() == SVG_ELEMENT && content) {
        declaration::append(&mut output, Origin::UserAgent, Property::Overflow, "hidden")?;
    }
    if element.name() != SVG_ELEMENT || content {
        declaration::append(
            &mut output,
            Origin::UserAgent,
            Property::TransformOrigin,
            "0 0",
        )?;
    }
    Ok(output)
}

/// The presentation attributes of an SVG element, as author-level declarations of specificity
/// zero (§6.6). An invalid value specifies the initial value (§4.2).
pub(super) fn presentation_hints(
    element: crate::dom::Element<'_>,
) -> Result<Vec<PropertyDeclaration>, StyleError> {
    let mut output = Vec::new();
    for (name, value) in &element.value().attrs {
        if !name.ns.is_empty() {
            continue;
        }
        let Some(property) = presentation_property(element.name(), &name.local) else {
            continue;
        };
        let longhands = if property == Property::Transform {
            // §6.6: the attribute takes the SVG 1.1 transform list syntax.
            <svgtypes::Transform as std::str::FromStr>::from_str(value)
                .ok()
                .map(|matrix| {
                    declaration::expand(
                        property,
                        &format!(
                            "matrix({}, {}, {}, {}, {}, {})",
                            matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f
                        ),
                    )
                })
                .transpose()?
                .flatten()
        } else if property == Property::D {
            // §9.3.2: the attribute's value is path data, which the property takes as a string.
            Some(vec![(
                property,
                Specified::Value(Arc::new(Component::String(value.as_ref().into()))),
            )])
        } else {
            declaration::expand_presentation(property, value)?
        };
        for (longhand, value) in
            longhands.unwrap_or_else(|| declaration::wide(property, WideKeyword::Initial))
        {
            declaration::append_longhand(&mut output, Origin::AuthorHint, longhand, value);
        }
    }
    Ok(output)
}
