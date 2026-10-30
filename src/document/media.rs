//! Media elements' replaced content. No media data is decoded, so the ready state stays
//! `HAVE_NOTHING`: a `video` represents its poster frame (HTML §4.8.11), and every media
//! element exposes its user interface because scripting is disabled (HTML §4.8.11.13).
use crate::document::{DocumentError, formatting::ImageResources};
use crate::dom::Element;
use crate::fonts::FontNamespace;
use crate::images::Image;
use crate::style::{POINTS_PER_PIXEL, Style};
use std::sync::Arc;

/// `line-height: normal`, for sizing the audio controls "about one line high" (HTML
/// §15.4.1): the upper end of CSS 2.1 §10.8.1's suggested range.
const NORMAL_LINE_HEIGHT: f32 = 1.2;
/// The controls' width in control heights: a play button, a seek bar and a volume button.
const CONTROLS_ASPECT_RATIO: f32 = 8.0;
/// Controls drawn in a viewBox one unit high, at the start of the media timeline.
const AUDIO_CONTROLS: &str = concat!(
    r#"<rect x="0.02" y="0.02" width="7.96" height="0.96" rx="0.48" fill="none" "#,
    r#"stroke="{color}" stroke-opacity="{faint}" stroke-width="0.04"/>"#,
    r#"<path d="M0.38 0.28L0.74 0.5L0.38 0.72Z" fill="{color}" fill-opacity="{alpha}"/>"#,
    r#"<rect x="1.15" y="0.47" width="5.7" height="0.06" rx="0.03" fill="{color}" "#,
    r#"fill-opacity="{faint}"/>"#,
    r#"<circle cx="1.2" cy="0.5" r="0.11" fill="{color}" fill-opacity="{alpha}"/>"#,
    r#"<path d="M7.1 0.42H7.22L7.38 0.28V0.72L7.22 0.58H7.1Z" fill="{color}" "#,
    r#"fill-opacity="{alpha}"/>"#,
    r#"<path d="M7.5 0.38Q7.6 0.5 7.5 0.62" fill="none" stroke="{color}" "#,
    r#"stroke-opacity="{alpha}" stroke-width="0.05"/>"#,
);
/// The seek bar and outline's opacity relative to the controls' colour.
const FAINT_OPACITY: f32 = 0.35;

/// HTML §4.8.11: the poster frame, or transparent black with no natural dimensions. With
/// scripting disabled the poster loads eagerly (HTML §2.5.7). A video's controls overlay it
/// and disappear when the user does not need them (HTML §15.4.1), as on paper.
pub(super) fn poster(
    element: Element<'_>,
    style: &Style,
    resources: &mut ImageResources<'_>,
) -> Arc<Image> {
    element
        .attr("poster")
        .filter(|reference| !reference.is_empty())
        .and_then(|reference| {
            resources.cache.load_optional(
                reference,
                resources.base,
                resources.resolver,
                style.element_font_namespace(),
            )
        })
        .map_or_else(
            || Arc::new(Image::Transparent),
            |image| crate::images::oriented_content(image, style),
        )
}

/// HTML §15.4.1: an `audio` element exposing its user interface is a replaced element about
/// one line high and as wide as its play button, seek bar and volume button, drawn in the
/// element's `color`.
pub(super) fn audio_controls(
    style: &Style,
    resources: &mut ImageResources<'_>,
) -> Result<Arc<Image>, DocumentError> {
    let height = style
        .line_height
        .resolve(style.size)
        .unwrap_or(style.size * NORMAL_LINE_HEIGHT)
        / POINTS_PER_PIXEL;
    let [red, green, blue] = style.color.rgb;
    let controls = AUDIO_CONTROLS
        .replace("{color}", &format!("rgb({red},{green},{blue})"))
        .replace("{alpha}", &style.color.alpha.to_string())
        .replace("{faint}", &(style.color.alpha * FAINT_OPACITY).to_string());
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{height}" viewBox="0 0 {CONTROLS_ASPECT_RATIO} 1">{controls}</svg>"#,
        height * CONTROLS_ASPECT_RATIO,
    );
    Ok(resources.cache.inline_svg_in_namespace(
        svg,
        resources.base,
        resources.resolver,
        FontNamespace::ROOT,
    )?)
}
