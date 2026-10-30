//! Computed values of SVG content written as the declarations and geometry attributes that the
//! SVG renderer reads, so that the document's cascade alone decides them (SVG 2 §6.4).
use crate::fonts::{FontFamily, FontStyle, FontWeight, FontWidth, GenericFamily};
use crate::paint::{
    AxisRadius, BasicShape, BlendMode, BoxArea, Calculation, CircleRadius, Color, CornerRadii,
    EllipseRadii, FilterFunction, GeometryBox, LengthPercentage, POINTS_PER_PIXEL, RadialExtent,
    Segment, ShapeBox,
};
use crate::style::{
    Baseline, Display, Overflow, Size, Style, StyleError, TransformBox, Visibility,
    svg_values::{Paint, PaintFallback, image_rendering_keyword, svg_keyword},
};
use std::fmt::Write;

/// SVG 2 §10: text content elements, which take the font and text properties.
const TEXT_CONTENT: &[&str] = &["text", "tspan", "textPath", "tref"];
const PERCENT: f32 = 100.0;
const CALC_POINT_UNIT: &str = "1pt";
/// CSS Fonts 4 §2.3: the `font-width` keywords and their percentages.
const WIDTH_KEYWORDS: [(FontWidth, &str); 9] = [
    (FontWidth::ULTRA_CONDENSED, "ultra-condensed"),
    (FontWidth::EXTRA_CONDENSED, "extra-condensed"),
    (FontWidth::CONDENSED, "condensed"),
    (FontWidth::SEMI_CONDENSED, "semi-condensed"),
    (FontWidth::NORMAL, "normal"),
    (FontWidth::SEMI_EXPANDED, "semi-expanded"),
    (FontWidth::EXPANDED, "expanded"),
    (FontWidth::EXTRA_EXPANDED, "extra-expanded"),
    (FontWidth::ULTRA_EXPANDED, "ultra-expanded"),
];
const NONE: &str = "none";
const TRANSFORM_ORIGIN: &str = "transform-origin";
const TRANSFORM_BOX: &str = "transform-box";
const FILL_BOX: &str = "fill-box";
const STROKE_BOX: &str = "stroke-box";
const VIEW_BOX: &str = "view-box";
const CLOSEST_SIDE: &str = "closest-side";
const FARTHEST_SIDE: &str = "farthest-side";
/// Elements whose viewport the SVG renderer clips as `overflow` says.
const OVERFLOW_ELEMENTS: [&str; 3] = ["marker", "svg", "symbol"];
/// Elements whose viewport the SVG renderer clips whatever their `overflow`.
const ALWAYS_CLIPPED_ELEMENTS: [&str; 2] = ["image", "pattern"];
/// Elements whose renderer measures their bounding box before placing their transform.
const BOXED_TRANSFORM_ELEMENTS: &[&str] = &[
    "circle", "ellipse", "g", "image", "line", "path", "polygon", "polyline", "rect", "svg",
    "switch", "text",
];
/// CSS Fonts 4 §2.1.3 generic families that the SVG renderer also takes as generic.
const UNCONFIGURED_GENERICS: [&str; 2] = ["cursive", "fantasy"];
const CALCULATED_LENGTH: &str = "a calculation mixing lengths and percentages in SVG content";

pub(super) fn is_text_content(tag: &str) -> bool {
    TEXT_CONTENT.contains(&tag)
}

/// The `style` attribute value carrying the element's computed presentation properties. The
/// document paints the root's own box, which its opacity, blending and display govern.
pub(super) fn declarations(tag: &str, style: &Style, root: bool) -> Result<String, StyleError> {
    let svg = &style.svg;
    let mut output = String::new();
    let mut push = |name: &str, value: &str| {
        let _ = write!(output, "{name}:{value};");
    };
    if !root {
        push("zoom", &number(style.zoom));
    }
    push("fill", &paint(&svg.fill));
    push("fill-opacity", &number(svg.fill_opacity));
    push("fill-rule", svg.fill_rule.name());
    push("stroke", &paint(&svg.stroke));
    push("stroke-opacity", &number(svg.stroke_opacity));
    push("stroke-width", &length_percentage(&svg.stroke_width)?);
    push(
        "stroke-dasharray",
        &match &svg.stroke_dasharray {
            None => NONE.to_owned(),
            Some(dashes) => dashes
                .iter()
                .map(length_percentage)
                .collect::<Result<Vec<_>, _>>()?
                .join(","),
        },
    );
    push(
        "stroke-dashoffset",
        &length_percentage(&svg.stroke_dashoffset)?,
    );
    push("stroke-linecap", svg.stroke_linecap.name());
    push("stroke-linejoin", svg.stroke_linejoin.name());
    push("stroke-miterlimit", &number(svg.stroke_miterlimit));
    for (name, marker) in ["marker-start", "marker-mid", "marker-end"]
        .into_iter()
        .zip(&svg.markers)
    {
        push(
            name,
            &marker.as_deref().map_or_else(|| NONE.to_owned(), url),
        );
    }
    push(
        "paint-order",
        &if svg.paint_order.is_empty() {
            "normal".to_owned()
        } else {
            svg.paint_order
                .iter()
                .map(|layer| layer.name())
                .collect::<Vec<_>>()
                .join(" ")
        },
    );
    push("stop-color", &color(svg.stop_color.used));
    push("stop-opacity", &number(svg.stop_opacity));
    push("flood-color", &color(svg.flood_color.used));
    push("flood-opacity", &number(svg.flood_opacity));
    push("lighting-color", &color(svg.lighting_color.used));
    push("clip-rule", svg.clip_rule.name());
    push("mask-type", svg.mask_type.name());
    push(
        "color-interpolation-filters",
        svg_keyword(svg.color_interpolation_filters),
    );
    push("text-anchor", svg.text_anchor.name());
    push("shape-rendering", svg_keyword(svg.shape_rendering));
    push("text-rendering", svg_keyword(svg.text_rendering));
    push(
        "image-rendering",
        image_rendering_keyword(svg.image_rendering),
    );
    push("dominant-baseline", baseline(style.baseline.dominant));
    push("writing-mode", style.writing_mode.name());
    push("text-orientation", style.text_orientation.name());
    push("visibility", visibility(style.visibility));
    if !root {
        push("opacity", &number(style.opacity));
        push("mix-blend-mode", blend_mode(style.mix_blend_mode)?);
        push("isolation", if style.isolate { "isolate" } else { "auto" });
        push("filter", &filters(&style.filter));
        if let Some(overflow) = overflow(tag, style.overflow)? {
            push("overflow", overflow);
        }
        push("mask", &mask(&style.decoration.masks)?);
        push("clip-path", &clip_path(style)?);
        match style.display {
            Display::None => push("display", "none"),
            Display::Contents => push("display", "contents"),
            _ => {}
        }
    }
    if style
        .decoration
        .radii
        .iter()
        .flatten()
        .any(|radius| *radius != LengthPercentage::ZERO)
    {
        push(
            "border-radius",
            &radii_css(&style.decoration.radii, motion_length)?,
        );
    }
    if style.motion.active() {
        for (name, value) in motion_transform(style)? {
            push(name, &value);
        }
    }
    push("font-family", &families(&style.families));
    push("font-size", &pixels(style.size));
    push("letter-spacing", &pixels(style.letter_spacing));
    push("word-spacing", &length_percentage(&style.word_spacing)?);
    push(
        "font-optical-sizing",
        if style.font_adjustments.optical_sizing {
            "auto"
        } else {
            "none"
        },
    );
    if !style.font_adjustments.variations.is_empty() {
        push(
            "font-variation-settings",
            &style
                .font_adjustments
                .variations
                .iter()
                .map(|variation| {
                    format!(
                        "\"{}\" {}",
                        String::from_utf8_lossy(&variation.tag),
                        number(variation.value)
                    )
                })
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    if is_text_content(tag) {
        push("direction", if style.rtl { "rtl" } else { "ltr" });
        push("font-style", font_style(style)?);
        push("font-weight", &font_weight(style.font_weight)?);
        push("font-stretch", font_width(style.font_width)?);
        push("font-variant", font_variant(style.font_variant_caps)?);
        push(
            "font-kerning",
            if style.font_features.kerning == crate::style::FontKerning::None {
                "none"
            } else {
                "auto"
            },
        );
    }
    Ok(output)
}

/// The geometry attributes of `tag` with their computed values (SVG 2 §7), `None` where the
/// value is `auto` or `none` and the renderer's absent-attribute behaviour applies.
pub(super) fn geometry(
    tag: &str,
    style: &Style,
) -> Result<Vec<(&'static str, String)>, StyleError> {
    let svg = &style.svg;
    let mut attributes = Vec::new();
    for (name, value) in [
        ("cx", Some(&svg.cx)),
        ("cy", Some(&svg.cy)),
        ("r", Some(&svg.r)),
        ("rx", svg.rx.as_ref()),
        ("ry", svg.ry.as_ref()),
        ("x", Some(&svg.x)),
        ("y", Some(&svg.y)),
        ("width", size(&style.width)?),
        ("height", size(&style.height)?),
    ] {
        if let Some(value) = value
            && crate::style::svg_values::presentation_property(tag, name).is_some()
        {
            attributes.push((name, length_percentage(value)?));
        }
    }
    if crate::style::svg_values::presentation_property(tag, "d").is_some()
        && let Some(path) = &svg.d
    {
        attributes.push(("d", path.to_string()));
    }
    Ok(attributes)
}

/// The element's `transform` attribute (or its gradient or pattern counterpart), origin and
/// reference box (CSS Transforms 1 §2, §6), empty without a transform.
pub(super) fn transform(
    tag: &str,
    style: &Style,
) -> Result<Vec<(&'static str, String)>, StyleError> {
    if style.motion.active() {
        return Ok(Vec::new());
    }
    let Some([a, b, c, d, e, f]) = style.planar_transform()? else {
        return Ok(Vec::new());
    };
    let [x, y, _] = &style.transform_origin;
    let mut attributes = vec![
        (
            crate::style::svg_values::transform_attribute(tag),
            format!(
                "matrix({a} {b} {c} {d} {} {})",
                e / POINTS_PER_PIXEL,
                f / POINTS_PER_PIXEL
            ),
        ),
        (
            TRANSFORM_ORIGIN,
            format!("{} {}", length_percentage(x)?, length_percentage(y)?),
        ),
    ];
    let reference = match style.transform_box {
        TransformBox::ViewBox => None,
        TransformBox::ContentBox | TransformBox::FillBox => Some(FILL_BOX),
        TransformBox::BorderBox | TransformBox::StrokeBox => Some(STROKE_BOX),
    };
    if let Some(reference) = reference {
        if !BOXED_TRANSFORM_ELEMENTS.contains(&tag) {
            return Err(StyleError::Unsupported(format!(
                "transform-box: {reference} on an SVG {tag} element"
            )));
        }
        attributes.push((TRANSFORM_BOX, reference.into()));
    }
    Ok(attributes)
}

/// SVG 2 §3.11 `overflow` on the elements whose viewport it clips: `hidden` and `scroll` clip,
/// and without scrolling `auto` is `visible`; one clip rectangle serves both axes. Other
/// elements have no viewport to clip.
fn overflow(tag: &str, [x, y]: [Overflow; 2]) -> Result<Option<&'static str>, StyleError> {
    let always_clipped = ALWAYS_CLIPPED_ELEMENTS.contains(&tag);
    if !always_clipped && !OVERFLOW_ELEMENTS.contains(&tag) {
        return Ok(None);
    }
    let clips = |overflow| {
        matches!(
            overflow,
            Overflow::Hidden | Overflow::Clip | Overflow::Scroll
        )
    };
    match (clips(x), clips(y)) {
        (true, true) => Ok(Some("hidden")),
        (false, false) if !always_clipped => Ok(Some("visible")),
        (false, false) => Err(StyleError::Unsupported(format!(
            "overflow: visible on an SVG {tag} element, which its renderer always clips"
        ))),
        _ => Err(StyleError::Unsupported(
            "overflow clipping one axis of an SVG viewport".into(),
        )),
    }
}

/// CSS Masking 1 §6.1 `clip-path` as the SVG renderer reads it: a `clipPath` reference, or a
/// basic shape in SVG content's reference box, where content and padding boxes are the fill box
/// and border and margin boxes the stroke box.
fn clip_path(style: &Style) -> Result<String, StyleError> {
    if let Some(reference) = &style.svg.clip_reference {
        return Ok(url(reference));
    }
    let Some(clip) = &style.decoration.masks.clip_path else {
        return Ok(NONE.into());
    };
    let reference = match clip.reference {
        GeometryBox::Fill
        | GeometryBox::Shape(ShapeBox::Visual(BoxArea::ContentBox | BoxArea::PaddingBox)) => {
            FILL_BOX
        }
        GeometryBox::View => VIEW_BOX,
        GeometryBox::Stroke | GeometryBox::Shape(_) => STROKE_BOX,
    };
    Ok(match &clip.shape {
        None => reference.to_owned(),
        Some(shape) => format!("{} {reference}", basic_shape(shape, true, shape_length)?),
    })
}

/// A basic shape with lengths in CSS pixels and positions as edge offsets.
fn basic_shape(
    shape: &BasicShape,
    explicit_center: bool,
    length: fn(&LengthPercentage) -> Result<String, StyleError>,
) -> Result<String, StyleError> {
    let lengths = |values: &mut dyn Iterator<Item = &LengthPercentage>| {
        values
            .map(length)
            .collect::<Result<Vec<_>, _>>()
            .map(|values| values.join(" "))
    };
    let round = |radii: &CornerRadii| -> Result<String, StyleError> {
        if radii
            .iter()
            .flatten()
            .all(|radius| *radius == LengthPercentage::ZERO)
        {
            return Ok(String::new());
        }
        Ok(format!(" round {}", radii_css(radii, length)?))
    };
    let at = |center: &crate::paint::Position| -> Result<String, StyleError> {
        if explicit_center {
            Ok(format!("at {}", position(center, length)?))
        } else {
            Ok(String::new())
        }
    };
    let extent = |extent: RadialExtent| match extent {
        RadialExtent::ClosestSide => CLOSEST_SIDE,
        RadialExtent::FarthestSide => FARTHEST_SIDE,
        RadialExtent::ClosestCorner => "closest-corner",
        RadialExtent::FarthestCorner => "farthest-corner",
    };
    let axis = |radius: &AxisRadius| match radius {
        AxisRadius::ClosestSide => Ok(CLOSEST_SIDE.to_owned()),
        AxisRadius::FarthestSide => Ok(FARTHEST_SIDE.to_owned()),
        AxisRadius::Length(value) => length(value),
    };
    let rule = |even_odd: bool| if even_odd { "evenodd" } else { "nonzero" };
    Ok(match shape {
        BasicShape::Inset { insets, radii } => {
            format!("inset({}{})", lengths(&mut insets.iter())?, round(radii)?)
        }
        BasicShape::Rect { edges, radii } => {
            format!("rect({}{})", lengths(&mut edges.iter())?, round(radii)?)
        }
        BasicShape::Circle { radius, center } => format!(
            "circle({} {})",
            match radius {
                CircleRadius::Extent(radius) => extent(*radius).to_owned(),
                CircleRadius::Length(radius) => length(radius)?,
            },
            at(center)?
        ),
        BasicShape::Ellipse { radii, center } => format!(
            "ellipse({} {})",
            match radii {
                EllipseRadii::Extent(radii) => extent(*radii).to_owned(),
                EllipseRadii::Axes([x, y]) => format!("{} {}", axis(x)?, axis(y)?),
            },
            at(center)?
        ),
        BasicShape::Polygon { even_odd, points } => format!(
            "polygon({}, {})",
            rule(*even_odd),
            points
                .iter()
                .map(|point| lengths(&mut point.iter()))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        ),
        BasicShape::Path { even_odd, contours } => {
            let mut quoted = String::new();
            cssparser::serialize_string(&path_data(contours), &mut quoted)
                .expect("writing a String cannot fail");
            format!("path({}, {quoted})", rule(*even_odd))
        }
    })
}

/// SVG path data for contours in CSS pixels.
fn path_data(contours: &[crate::paint::Contour]) -> String {
    let mut data = String::new();
    for contour in contours {
        let [x, y] = contour.start;
        let _ = write!(data, "M{x} {y}");
        for segment in &contour.segments {
            let _ = match segment {
                Segment::Line([x, y]) => write!(data, "L{x} {y}"),
                Segment::Cubic([[x1, y1], [x2, y2], [x, y]]) => {
                    write!(data, "C{x1} {y1} {x2} {y2} {x} {y}")
                }
            };
        }
        if contour.closed {
            data.push('Z');
        }
    }
    data
}

/// CSS Masking 1 §7: a single reference to an SVG `mask` element, whose `mask-type` the
/// default `match-source` mode follows.
fn mask(masks: &crate::paint::Masks) -> Result<String, StyleError> {
    if !masks.has_layers() && !masks.border.is_drawn() {
        return Ok(NONE.into());
    }
    match masks.sources.as_slice() {
        [Some(reference)]
            if !masks.layers.has_images()
                && !masks.border.is_drawn()
                && masks.modes == [crate::paint::MaskMode::default()] =>
        {
            Ok(url(reference))
        }
        _ => Err(StyleError::Unsupported(
            "mask layers other than one mask element reference on SVG content".into(),
        )),
    }
}

fn size(size: &Size) -> Result<Option<&LengthPercentage>, StyleError> {
    match size {
        Size::Auto => Ok(None),
        Size::Value(value) => Ok(Some(value)),
        Size::Intrinsic(_) => Err(StyleError::Unsupported(
            "an intrinsic size keyword on SVG content".into(),
        )),
    }
}

fn number(value: f32) -> String {
    value.to_string()
}

/// A basic shape length; the SVG renderer reads a sum of lengths and percentages, such as an
/// `xywh()` inset, as `calc(P% + L)`.
fn shape_length(value: &LengthPercentage) -> Result<String, StyleError> {
    match value {
        LengthPercentage::Calc(calculation) if linear(calculation) => {
            let length = calculation.resolve(0.0);
            Ok(format!(
                "calc({}% + {})",
                (calculation.resolve(1.0) - length) * PERCENT,
                pixels(length)
            ))
        }
        _ => length_percentage(value),
    }
}

fn linear(calculation: &Calculation) -> bool {
    match calculation {
        Calculation::Value(_) | Calculation::Percentage(_) => true,
        Calculation::Sum(terms) => terms.iter().all(linear),
        _ => false,
    }
}

fn pixels(points: f32) -> String {
    format!("{}px", points / POINTS_PER_PIXEL)
}

fn length_percentage(value: &LengthPercentage) -> Result<String, StyleError> {
    match value {
        LengthPercentage::Length(points) => Ok(pixels(*points)),
        LengthPercentage::Percentage(fraction) => Ok(format!("{}%", fraction * PERCENT)),
        LengthPercentage::Calc(_) => Err(StyleError::Unsupported(CALCULATED_LENGTH.into())),
    }
}

fn color(color: Color) -> String {
    let [red, green, blue] = color.rgb;
    format!("rgba({red},{green},{blue},{})", color.alpha)
}

fn url(reference: &str) -> String {
    let mut quoted = String::new();
    cssparser::serialize_string(reference, &mut quoted).expect("writing a String cannot fail");
    format!("url({quoted})")
}

fn paint(paint: &Paint) -> String {
    match paint {
        Paint::None => NONE.into(),
        Paint::Color(paint) => color(paint.used),
        Paint::Server {
            url: reference,
            fallback,
        } => match fallback {
            None => url(reference),
            Some(PaintFallback::None) => format!("{} {NONE}", url(reference)),
            Some(PaintFallback::Color(fallback)) => {
                format!("{} {}", url(reference), color(fallback.used))
            }
        },
        Paint::ContextFill => "context-fill".into(),
        Paint::ContextStroke => "context-stroke".into(),
    }
}

/// CSS Inline 3 §4.1 baselines as SVG `dominant-baseline` keywords.
fn baseline(baseline: Baseline) -> &'static str {
    match baseline {
        Baseline::Auto => "auto",
        Baseline::Alphabetic => "alphabetic",
        Baseline::TextBottom => "text-after-edge",
        Baseline::TextTop => "text-before-edge",
        Baseline::Ideographic => "ideographic",
        Baseline::Middle => "middle",
        Baseline::Central => "central",
        Baseline::Mathematical => "mathematical",
        Baseline::Hanging => "hanging",
    }
}

fn visibility(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Visible => "visible",
        Visibility::Hidden => "hidden",
        Visibility::Collapse => "collapse",
    }
}

fn blend_mode(mode: BlendMode) -> Result<&'static str, StyleError> {
    Ok(match mode {
        BlendMode::Normal => "normal",
        BlendMode::Multiply => "multiply",
        BlendMode::Screen => "screen",
        BlendMode::Overlay => "overlay",
        BlendMode::Darken => "darken",
        BlendMode::Lighten => "lighten",
        BlendMode::ColorDodge => "color-dodge",
        BlendMode::ColorBurn => "color-burn",
        BlendMode::HardLight => "hard-light",
        BlendMode::SoftLight => "soft-light",
        BlendMode::Difference => "difference",
        BlendMode::Exclusion => "exclusion",
        BlendMode::Hue => "hue",
        BlendMode::Saturation => "saturation",
        BlendMode::Color => "color",
        BlendMode::Luminosity => "luminosity",
        // The SVG renderer draws vector groups with PDF blend modes, none of which adds, and
        // the additive operators' raster read-back works on the document's display list only.
        BlendMode::PlusDarker | BlendMode::PlusLighter => {
            return Err(StyleError::Unsupported(
                "additive mix-blend-mode on SVG content, which composites as vectors".into(),
            ));
        }
    })
}

/// Filter Effects 1 §6 functions in their computed form; lengths are CSS pixels.
fn filters(functions: &[FilterFunction]) -> String {
    if functions.is_empty() {
        return NONE.into();
    }
    functions
        .iter()
        .map(|function| match function {
            FilterFunction::Blur(deviation) => format!("blur({})", pixels(*deviation)),
            FilterFunction::Brightness(amount) => format!("brightness({amount})"),
            FilterFunction::Contrast(amount) => format!("contrast({amount})"),
            FilterFunction::DropShadow {
                offset: [x, y],
                deviation,
                color: shadow,
            } => format!(
                "drop-shadow({} {} {} {})",
                pixels(*x),
                pixels(*y),
                pixels(*deviation),
                color(*shadow)
            ),
            FilterFunction::Grayscale(amount) => format!("grayscale({amount})"),
            FilterFunction::HueRotate(degrees) => format!("hue-rotate({degrees}deg)"),
            FilterFunction::Invert(amount) => format!("invert({amount})"),
            FilterFunction::Opacity(amount) => format!("opacity({amount})"),
            FilterFunction::Saturate(amount) => format!("saturate({amount})"),
            FilterFunction::Sepia(amount) => format!("sepia({amount})"),
            FilterFunction::Url(reference) => url(reference),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn families(families: &[FontFamily]) -> String {
    families
        .iter()
        .map(|family| match family {
            // A generic family without a configured face computes as its name; it stays a
            // generic family, which `@font-face` names cannot claim (CSS Fonts 4 §2.1.3).
            FontFamily::Named(name) if UNCONFIGURED_GENERICS.contains(&name.as_str()) => {
                name.clone()
            }
            FontFamily::Named(name) => {
                let mut quoted = String::new();
                cssparser::serialize_string(name, &mut quoted)
                    .expect("writing a String cannot fail");
                quoted
            }
            FontFamily::Generic(GenericFamily::Serif) => "serif".into(),
            FontFamily::Generic(GenericFamily::SansSerif) => "sans-serif".into(),
            FontFamily::Generic(GenericFamily::Monospace) => "monospace".into(),
            FontFamily::Generic(GenericFamily::Math) => "math".into(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The renderer takes `oblique` at its default angle only.
fn font_style(style: &Style) -> Result<&'static str, StyleError> {
    match style.font_style {
        FontStyle::Normal => Ok("normal"),
        FontStyle::Italic => Ok("italic"),
        FontStyle::Oblique if style.font_slant == crate::fonts::FontSlant::DEFAULT => Ok("oblique"),
        FontStyle::Oblique => Err(StyleError::Unsupported(format!(
            "font-style: oblique {}deg on SVG text",
            style.font_slant.value()
        ))),
    }
}

/// The renderer's font matching takes whole weights, which covers every weight but a
/// fractional one.
fn font_weight(weight: FontWeight) -> Result<String, StyleError> {
    let value = weight.value();
    if value.fract() == 0.0 {
        Ok(number(value))
    } else {
        Err(StyleError::Unsupported(format!(
            "font-weight: {value} on SVG text"
        )))
    }
}

fn font_width(width: FontWidth) -> Result<&'static str, StyleError> {
    WIDTH_KEYWORDS
        .iter()
        .find(|(keyword, _)| *keyword == width)
        .map(|(_, name)| *name)
        .ok_or_else(|| {
            StyleError::Unsupported(format!("font-width: {}% on SVG text", width.value()))
        })
}

fn font_variant(caps: crate::style::FontVariantCaps) -> Result<&'static str, StyleError> {
    match caps {
        crate::style::FontVariantCaps::Normal => Ok("normal"),
        crate::style::FontVariantCaps::SmallCaps => Ok("small-caps"),
        other => Err(StyleError::Unsupported(format!(
            "font-variant-caps: {other:?} on SVG text"
        ))),
    }
}

fn position(
    [x, y]: &crate::paint::Position,
    length: fn(&LengthPercentage) -> Result<String, StyleError>,
) -> Result<String, StyleError> {
    Ok(format!(
        "{} {} {} {}",
        if x.from_end { "right" } else { "left" },
        length(&x.offset)?,
        if y.from_end { "bottom" } else { "top" },
        length(&y.offset)?
    ))
}
fn motion_transform(style: &Style) -> Result<Vec<(&'static str, String)>, StyleError> {
    use crate::style::motion::{Curve, OffsetPath, OffsetPosition, RaySize};
    let path = match &*style.motion.path {
        OffsetPath::None => NONE.to_owned(),
        OffsetPath::Path { curve, .. } => match curve {
            Curve::Box => VIEW_BOX.to_owned(),
            Curve::Shape {
                shape,
                explicit_center,
            } => basic_shape(shape, *explicit_center, motion_length)?,
            Curve::Reference(reference) => url(reference),
            Curve::Ray {
                angle,
                size,
                contain,
                at,
            } => format!(
                "ray({}deg {}{}{})",
                angle.to_degrees(),
                match size {
                    RaySize::ClosestSide => CLOSEST_SIDE,
                    RaySize::FarthestSide => FARTHEST_SIDE,
                    RaySize::ClosestCorner => "closest-corner",
                    RaySize::FarthestCorner => "farthest-corner",
                    RaySize::Sides => "sides",
                },
                if *contain { " contain" } else { "" },
                at.as_ref()
                    .map(|at| position(at, motion_length).map(|at| format!(" at {at}")))
                    .transpose()?
                    .unwrap_or_default()
            ),
        },
    };
    let mut attributes = vec![
        ("offset-path", path),
        ("offset-distance", motion_length(&style.motion.distance)?),
        (
            "offset-position",
            match &style.motion.position {
                OffsetPosition::Normal => "normal".to_owned(),
                OffsetPosition::Auto => "auto".to_owned(),
                OffsetPosition::Position(at) => position(at, motion_length)?,
            },
        ),
        (
            "offset-anchor",
            style
                .motion
                .anchor
                .as_ref()
                .map(|at| position(at, motion_length))
                .transpose()?
                .unwrap_or_else(|| "auto".to_owned()),
        ),
        (
            "offset-rotate",
            format!(
                "{}{}deg",
                if style.motion.rotate.tangent {
                    "auto "
                } else {
                    ""
                },
                style.motion.rotate.angle.to_degrees()
            ),
        ),
        (
            TRANSFORM_ORIGIN,
            style
                .transform_origin
                .iter()
                .map(motion_length)
                .collect::<Result<Vec<_>, _>>()?
                .join(" "),
        ),
        (
            TRANSFORM_BOX,
            match style.transform_box {
                TransformBox::ViewBox => VIEW_BOX,
                TransformBox::ContentBox | TransformBox::FillBox => FILL_BOX,
                TransformBox::BorderBox | TransformBox::StrokeBox => STROKE_BOX,
            }
            .to_owned(),
        ),
    ];
    if let Some(translate) = &style.translate {
        attributes.push((
            "translate",
            translate
                .iter()
                .map(motion_length)
                .collect::<Result<Vec<_>, _>>()?
                .join(" "),
        ));
    }
    if let Some(rotate) = style.rotate {
        attributes.push((
            "rotate",
            format!(
                "{} {} {} {}deg",
                rotate.axis[0],
                rotate.axis[1],
                rotate.axis[2],
                rotate.angle.to_degrees()
            ),
        ));
    }
    if let Some(scale) = style.scale {
        attributes.push(("scale", scale.map(|value| value.to_string()).join(" ")));
    }
    if !style.transform.is_empty() {
        attributes.push((
            "transform",
            style
                .transform
                .iter()
                .map(|function| function.css(motion_length))
                .collect::<Result<Vec<_>, _>>()?
                .join(" "),
        ));
    }
    Ok(attributes)
}

/// Calculation leaves are numeric points, including scalar factors. Normalize percentages
/// to numeric points inside the expression, then restore its length unit once at the root.
fn motion_length(value: &LengthPercentage) -> Result<String, StyleError> {
    Ok(match value {
        LengthPercentage::Calc(value) => {
            format!("calc({} * {CALC_POINT_UNIT})", calculated_number(value))
        }
        _ => return length_percentage(value),
    })
}
fn calculated_number(value: &Calculation) -> String {
    use crate::paint::{BinaryFunction as B, RoundingStrategy as R, UnaryFunction as U};
    let list = |items: &[Calculation]| {
        items
            .iter()
            .map(calculated_number)
            .collect::<Vec<_>>()
            .join(",")
    };
    match value {
        Calculation::Value(value) => {
            if value.is_nan() {
                "NaN".to_owned()
            } else if *value == f32::INFINITY {
                "infinity".to_owned()
            } else if *value == f32::NEG_INFINITY {
                "-infinity".to_owned()
            } else {
                value.to_string()
            }
        }
        Calculation::Percentage(value) => format!("({}% / {CALC_POINT_UNIT})", value * PERCENT),
        Calculation::Sum(items) | Calculation::Product(items) => format!(
            "({})",
            items
                .iter()
                .map(calculated_number)
                .collect::<Vec<_>>()
                .join(if matches!(value, Calculation::Sum(_)) {
                    " + "
                } else {
                    " * "
                })
        ),
        Calculation::Invert(value) => format!("(1 / {})", calculated_number(value)),
        Calculation::Min(items) => format!("min({})", list(items)),
        Calculation::Max(items) => format!("max({})", list(items)),
        Calculation::Hypot(items) => format!("hypot({})", list(items)),
        Calculation::Unary(function, value) => {
            let expression = format!(
                "{}({})",
                match function {
                    U::Abs => "abs",
                    U::Sign => "sign",
                    U::Sin => "sin",
                    U::Cos => "cos",
                    U::Tan => "tan",
                    U::Asin => "asin",
                    U::Acos => "acos",
                    U::Atan => "atan",
                    U::Sqrt => "sqrt",
                    U::Exp => "exp",
                    U::Ln => "log",
                },
                calculated_number(value)
            );
            if matches!(function, U::Asin | U::Acos | U::Atan) {
                format!("({expression} / 1rad)")
            } else {
                expression
            }
        }
        Calculation::Binary(function, values) => {
            let expression = match function {
                B::Round(strategy) => format!(
                    "round({},{})",
                    match strategy {
                        R::Nearest => "nearest",
                        R::Up => "up",
                        R::Down => "down",
                        R::ToZero => "to-zero",
                    },
                    list(&**values)
                ),
                function => format!(
                    "{}({})",
                    match function {
                        B::Mod => "mod",
                        B::Rem => "rem",
                        B::Atan2 => "atan2",
                        B::Pow => "pow",
                        B::Log => "log",
                        B::Round(_) => unreachable!(),
                    },
                    list(&**values)
                ),
            };
            if matches!(function, B::Atan2) {
                format!("({expression} / 1rad)")
            } else {
                expression
            }
        }
        Calculation::Progress(values) => format!("progress({})", list(&**values)),
        Calculation::Mix(values) => format!(
            "calc-mix(calc({}),{},{})",
            calculated_number(&values[0]),
            calculated_number(&values[1]),
            calculated_number(&values[2])
        ),
        Calculation::Eased(easing, value) => format!(
            "calc-mix(calc({}) by {},0,1)",
            calculated_number(value),
            easing_css(easing)
        ),
    }
}
fn easing_css(value: &crate::paint::Easing) -> String {
    use crate::paint::{Easing, StepPosition};
    match value {
        Easing::Linear => "linear".to_owned(),
        Easing::CubicBezier(values) => format!(
            "cubic-bezier({})",
            values.map(|value| value.to_string()).join(",")
        ),
        Easing::Steps { count, position } => format!(
            "steps({count},{})",
            match position {
                StepPosition::Start => "jump-start",
                StepPosition::End => "jump-end",
                StepPosition::None => "jump-none",
                StepPosition::Both => "jump-both",
            }
        ),
        Easing::Piecewise(points) => format!(
            "linear({})",
            points
                .iter()
                .map(|point| format!("{} {}%", point.output, point.input * PERCENT))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn radii_css(
    radii: &CornerRadii,
    length: fn(&LengthPercentage) -> Result<String, StyleError>,
) -> Result<String, StyleError> {
    let axis = |axis: usize| {
        radii
            .iter()
            .map(|radius| length(&radius[axis]))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| values.join(" "))
    };
    Ok(format!("{} / {}", axis(0)?, axis(1)?))
}
