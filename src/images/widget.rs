//! Native appearances of the widgets laid out as replaced elements (CSS UI 4 §7.2 `auto`,
//! HTML §15.5): vector drawings made once the widget box's used size is known, so geometry
//! stays crisp at any size authors give the box.
use crate::images::{Image, ImageCache, ImageError, NaturalDimensions};
use crate::paint::Color;
use std::fmt::Write as _;
use std::sync::Arc;

/// Outline width, in CSS pixels.
const STROKE: f32 = 1.0;
/// Thickness of a slider track, in CSS pixels.
const TRACK_THICKNESS: f32 = 4.0;
/// Opacity of disabled widgets' drawing (HTML §4.10.18.5 leaves the look to the UA).
const DISABLED_OPACITY: f32 = 0.45;
/// The checkbox and radio glyphs are drawn in a 16-unit square, centred in the box.
const GLYPH_BOX: f32 = 16.0;
const CHECK_MARK: &str = "M4.2 8.4 7 11.2 11.9 5.4";
const CHECKBOX_RADIUS: f32 = 2.5;
const RADIO_RADIUS: f32 = 6.5;
const RADIO_DOT_RADIUS: f32 = 3.5;
const CHECK_STROKE: f32 = 2.0;
/// The drop-down arrow is a chevron in a 10-unit square.
const ARROW_BOX: f32 = 10.0;
const ARROW: &str = "M2 3.5 5 6.5 8 3.5";
const ARROW_STROKE: f32 = 1.5;
/// Tick marks of a slider's list of suggestions extend this far beyond its track.
const TICK_LENGTH: f32 = 3.0;
/// An indeterminate progress bar shows a stripe this fraction of its width.
const INDETERMINATE_STRIPE: f32 = 0.3;

/// The colours a native appearance draws with, resolved from the element's colour scheme
/// and `accent-color` (CSS UI 4 §7.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WidgetPalette {
    pub accent: Color,
    pub accent_text: Color,
    pub field: Color,
    pub border: Color,
    pub track: Color,
}

/// How full a gauge is drawn and in which of its regions the value falls (HTML §4.10.14).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GaugeFill {
    Optimum,
    Suboptimal,
    EvenLessGood,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WidgetDrawing {
    Checkbox {
        checked: bool,
    },
    Radio {
        checked: bool,
    },
    /// A slider whose thumb sits at `position` from its lowest value; `ticks` are the
    /// positions of its suggested values. Primitive appearance keeps only the thumb, which
    /// operating the widget needs (CSS UI 4 §7.2.3).
    Range {
        position: f32,
        ticks: Vec<f32>,
        track: bool,
    },
    /// A gauge filled to `position`; primitive appearance keeps the fill, which no CSS
    /// expresses, and leaves the track to the author's box.
    Meter {
        position: f32,
        fill: GaugeFill,
        track: bool,
    },
    /// A progress bar filled to `position`, or indeterminate.
    Progress {
        position: Option<f32>,
        track: bool,
    },
    /// The button of a drop-down box (HTML §15.5.16).
    DropDownArrow,
}

/// A widget's native drawing and its natural size in CSS pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub(crate) natural: [f32; 2],
    pub(crate) drawing: WidgetDrawing,
    pub(crate) palette: WidgetPalette,
    /// The lowest value is on the right (HTML §15.5.8, §15.5.14, §15.5.15).
    pub(crate) rtl: bool,
    pub(crate) disabled: bool,
}

fn rgba(color: Color) -> String {
    let [red, green, blue] = color.rgb;
    format!("rgba({red},{green},{blue},{})", color.alpha)
}

impl Widget {
    pub(crate) fn natural_size(&self) -> NaturalDimensions {
        let [width, height] = self.natural;
        NaturalDimensions {
            width: Some(width),
            height: Some(height),
            ratio: Some(width / height),
        }
    }

    /// The drawing at a `size` box in CSS pixels, as an SVG image filling it.
    pub(crate) fn render(&self, size: [f32; 2]) -> Result<Arc<Image>, ImageError> {
        let [width, height] = size;
        let mut body = String::new();
        let palette = &self.palette;
        match &self.drawing {
            WidgetDrawing::Checkbox { checked } => {
                let (fill, stroke) = if *checked {
                    (palette.accent, palette.accent)
                } else {
                    (palette.field, palette.border)
                };
                let inset = STROKE / 2.0 + 1.0;
                let side = GLYPH_BOX - 2.0 * inset;
                write!(
                    body,
                    r#"<rect x="{inset}" y="{inset}" width="{side}" height="{side}" rx="{CHECKBOX_RADIUS}" fill="{}" stroke="{}" stroke-width="{STROKE}"/>"#,
                    rgba(fill),
                    rgba(stroke)
                )
                .expect("writing to a string cannot fail");
                if *checked {
                    write!(
                        body,
                        r#"<path d="{CHECK_MARK}" fill="none" stroke="{}" stroke-width="{CHECK_STROKE}" stroke-linecap="round" stroke-linejoin="round"/>"#,
                        rgba(palette.accent_text)
                    )
                    .expect("writing to a string cannot fail");
                }
                body = Self::centred_glyph(&body, GLYPH_BOX);
            }
            WidgetDrawing::Radio { checked } => {
                let centre = GLYPH_BOX / 2.0;
                write!(
                    body,
                    r#"<circle cx="{centre}" cy="{centre}" r="{RADIO_RADIUS}" fill="{}" stroke="{}" stroke-width="{STROKE}"/>"#,
                    rgba(palette.field),
                    rgba(if *checked { palette.accent } else { palette.border })
                )
                .expect("writing to a string cannot fail");
                if *checked {
                    write!(
                        body,
                        r#"<circle cx="{centre}" cy="{centre}" r="{RADIO_DOT_RADIUS}" fill="{}"/>"#,
                        rgba(palette.accent)
                    )
                    .expect("writing to a string cannot fail");
                }
                body = Self::centred_glyph(&body, GLYPH_BOX);
            }
            WidgetDrawing::Range {
                position,
                ticks,
                track,
            } => {
                let radius = (height / 2.0).min(width / 2.0) - STROKE;
                let travel = (width - 2.0 * (radius + STROKE)).max(0.0);
                let at = |position: f32| {
                    let along = if self.rtl { 1.0 - position } else { position };
                    radius + STROKE + along * travel
                };
                let middle = height / 2.0;
                if *track {
                    let top = middle - TRACK_THICKNESS / 2.0;
                    let thumb = at(*position);
                    let (fill_x, fill_width) = if self.rtl {
                        (thumb, width - thumb)
                    } else {
                        (0.0, thumb)
                    };
                    write!(
                        body,
                        r#"<rect x="0" y="{top}" width="{width}" height="{TRACK_THICKNESS}" rx="{}" fill="{}"/><rect x="{fill_x}" y="{top}" width="{fill_width}" height="{TRACK_THICKNESS}" rx="{}" fill="{}"/>"#,
                        TRACK_THICKNESS / 2.0,
                        rgba(palette.track),
                        TRACK_THICKNESS / 2.0,
                        rgba(palette.accent)
                    )
                    .expect("writing to a string cannot fail");
                    for tick in ticks {
                        let x = at(*tick);
                        write!(
                            body,
                            r#"<line x1="{x}" y1="{}" x2="{x}" y2="{}" stroke="{}" stroke-width="{STROKE}"/>"#,
                            top - TICK_LENGTH,
                            top + TRACK_THICKNESS + TICK_LENGTH,
                            rgba(palette.border)
                        )
                        .expect("writing to a string cannot fail");
                    }
                }
                write!(
                    body,
                    r#"<circle cx="{}" cy="{middle}" r="{}" fill="{}" stroke="{}" stroke-width="{STROKE}"/>"#,
                    at(*position),
                    radius.max(0.0),
                    rgba(palette.accent),
                    rgba(palette.field)
                )
                .expect("writing to a string cannot fail");
            }
            WidgetDrawing::Meter {
                position,
                fill,
                track,
            } => {
                let color = match fill {
                    GaugeFill::Optimum => palette.accent,
                    GaugeFill::Suboptimal => SUBOPTIMAL,
                    GaugeFill::EvenLessGood => EVEN_LESS_GOOD,
                };
                self.bar(&mut body, size, (0.0, *position), color, *track);
            }
            WidgetDrawing::Progress { position, track } => {
                // An indeterminate bar shows a stripe from its start, as a still frame.
                let span = position.map_or((0.0, INDETERMINATE_STRIPE), |position| (0.0, position));
                self.bar(&mut body, size, span, palette.accent, *track);
            }
            WidgetDrawing::DropDownArrow => {
                write!(
                    body,
                    r#"<path d="{ARROW}" fill="none" stroke="{}" stroke-width="{ARROW_STROKE}" stroke-linecap="round" stroke-linejoin="round"/>"#,
                    rgba(palette.border)
                )
                .expect("writing to a string cannot fail");
                body = Self::centred_glyph(&body, ARROW_BOX);
            }
        }
        let opacity = if self.disabled { DISABLED_OPACITY } else { 1.0 };
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"><g opacity="{opacity}">{body}</g></svg>"#
        );
        ImageCache::default().inline_svg_in_namespace(
            svg,
            Default::default(),
            &crate::resources::ResourceBase::default(),
            &crate::resources::FileResolver::default(),
            crate::fonts::FontNamespace::ROOT,
        )
    }

    /// A glyph drawn in a `side`-unit square, scaled uniformly into the middle of the box.
    fn centred_glyph(body: &str, side: f32) -> String {
        format!(
            r#"<svg width="100%" height="100%" viewBox="0 0 {side} {side}" preserveAspectRatio="xMidYMid meet">{body}</svg>"#
        )
    }

    /// A rounded bar filling the fraction `start..end` of the box from its start edge.
    fn bar(
        &self,
        body: &mut String,
        [width, height]: [f32; 2],
        (start, end): (f32, f32),
        color: Color,
        track: bool,
    ) {
        let radius = height / 2.0;
        if track {
            write!(
                body,
                r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{}" stroke="{}" stroke-width="{STROKE}"/>"#,
                STROKE / 2.0,
                STROKE / 2.0,
                (width - STROKE).max(0.0),
                (height - STROKE).max(0.0),
                (radius - STROKE / 2.0).max(0.0),
                rgba(self.palette.track),
                rgba(self.palette.border)
            )
            .expect("writing to a string cannot fail");
        }
        let (start, end) = if self.rtl {
            (1.0 - end, 1.0 - start)
        } else {
            (start, end)
        };
        write!(
            body,
            r#"<rect x="{}" y="0" width="{}" height="{height}" rx="{radius}" fill="{}"/>"#,
            start * width,
            (end - start) * width,
            rgba(color)
        )
        .expect("writing to a string cannot fail");
    }
}

/// Meter regions other than the optimum, in the conventional traffic-light colours.
const SUBOPTIMAL: Color = Color {
    rgb: [0xd9, 0x8e, 0x00],
    alpha: 1.0,
};
const EVEN_LESS_GOOD: Color = Color {
    rgb: [0xc4, 0x2b, 0x1c],
    alpha: 1.0,
};
