//! Computed values of SVG content written as the declarations and geometry attributes that the
//! SVG renderer reads, so that the document's cascade alone decides them (SVG 2 §6.4).
use crate::fonts::{FontFamily, FontStyle, FontWeight, FontWidth, GenericFamily};
use crate::paint::{BlendMode, Color, FilterFunction, LengthPercentage, POINTS_PER_PIXEL};
use crate::style::{
    Baseline, Display, Size, Style, StyleError, Visibility,
    svg_values::{Paint, PaintFallback, image_rendering_keyword, svg_keyword},
};
use std::fmt::Write;

/// SVG 2 §10: text content elements, which take the font and text properties.
const TEXT_CONTENT: &[&str] = &["text", "tspan", "textPath", "tref"];
const PERCENT: f32 = 100.0;
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
    push("visibility", visibility(style.visibility));
    if !root {
        push("opacity", &number(style.opacity));
        push("mix-blend-mode", blend_mode(style.mix_blend_mode)?);
        push("isolation", if style.isolate { "isolate" } else { "auto" });
        push("filter", &filters(&style.filter));
        match style.display {
            Display::None => push("display", "none"),
            Display::Contents => push("display", "contents"),
            _ => {}
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
