//! Compositing 1 §5.8 source-over compositing with §10 blending, and the Compositing 2
//! §9.1.14–§9.1.15 additive operators, on premultiplied RGBA samples in `0..=1`; and the
//! colour that source-over drawing must paint over a backdrop to produce a given result.
use crate::paint::{BlendMode, srgb_from_linear, srgb_to_linear};

pub(crate) const ALPHA: usize = 3;
/// A premultiplied RGBA sample.
pub(crate) type Sample = [f32; 4];
type Rgb = [f32; 3];

/// Compositing 1 §10.2 luminosity coefficients of the non-separable modes.
const LUMINOSITY: Rgb = [0.3, 0.59, 0.11];
const HALF: f32 = 0.5;
const QUARTER: f32 = 0.25;
const CHANNEL_MAX: f32 = 255.0;

/// An 8-bit premultiplied pixel as a sample.
pub(crate) fn sample(pixel: tiny_skia::PremultipliedColorU8) -> Sample {
    [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
        .map(|channel| f32::from(channel) / CHANNEL_MAX)
}

/// A sample as the nearest 8-bit premultiplied pixel.
pub(crate) fn pixel(sample: Sample) -> tiny_skia::PremultipliedColorU8 {
    let [red, green, blue, alpha] =
        sample.map(|channel| (channel.clamp(0.0, 1.0) * CHANNEL_MAX).round() as u8);
    tiny_skia::PremultipliedColorU8::from_rgba(
        red.min(alpha),
        green.min(alpha),
        blue.min(alpha),
        alpha,
    )
    .expect("colour channels are clamped to the alpha")
}

/// `source` composited onto `backdrop` by `mode`.
pub(crate) fn composite(mode: BlendMode, source: Sample, backdrop: Sample) -> Sample {
    if matches!(mode, BlendMode::PlusDarker | BlendMode::PlusLighter) {
        return additive_color(mode, source, backdrop);
    }
    let (source_alpha, backdrop_alpha) = (source[ALPHA], backdrop[ALPHA]);
    let blended = blend(mode, unpremultiplied(backdrop), unpremultiplied(source));
    // §10: cs = (1 − αb)·Cs + αb·B(Cb, Cs), composited source-over (§5.8).
    std::array::from_fn(|channel| {
        if channel == ALPHA {
            source_alpha + backdrop_alpha * (1.0 - source_alpha)
        } else {
            (1.0 - backdrop_alpha) * source[channel]
                + source_alpha * backdrop_alpha * blended[channel]
                + (1.0 - source_alpha) * backdrop[channel]
        }
    })
}

/// The premultiplied result of the additive `operator`, `plus-darker` or `plus-lighter`.
pub(crate) fn additive_color(operator: BlendMode, source: Sample, backdrop: Sample) -> Sample {
    let alpha = (source[ALPHA] + backdrop[ALPHA]).min(1.0);
    std::array::from_fn(|channel| match operator {
        _ if channel == ALPHA => alpha,
        // §9.1.14 as written leaves two opaque layers transparent; this is the premultiplied
        // form of Core Graphics' `kCGBlendModePlusDarker`, which the operator comes from:
        // `co = max(0, αo − ((αs − cs) + (αb − cb)))`.
        BlendMode::PlusDarker => (alpha
            - ((source[ALPHA] - source[channel]) + (backdrop[ALPHA] - backdrop[channel])))
            .max(0.0),
        // §9.1.15: `co = min(1, cs + cb)`.
        _ => (source[channel] + backdrop[channel]).min(1.0),
    })
}

/// The premultiplied colour that source-over compositing onto `backdrop` turns into `result`.
/// Over an opaque backdrop that is the result itself; otherwise `αx = (αo − αb) / (1 − αb)`
/// and `cx = co − cb·(1 − αx)`.
pub(crate) fn over_color(result: Sample, backdrop: Sample) -> Sample {
    let alpha = if backdrop[ALPHA] >= 1.0 {
        1.0
    } else {
        ((result[ALPHA] - backdrop[ALPHA]) / (1.0 - backdrop[ALPHA])).clamp(0.0, 1.0)
    };
    std::array::from_fn(|channel| {
        if channel == ALPHA {
            alpha
        } else {
            (result[channel] - backdrop[channel] * (1.0 - alpha)).clamp(0.0, alpha)
        }
    })
}

/// A premultiplied sRGB sample in linear light (SVG 2 §13.9 `linearRGB`), still premultiplied.
pub(crate) fn to_linear(sample: Sample) -> Sample {
    transfer(sample, srgb_to_linear)
}

/// A premultiplied linear-light sample in sRGB, still premultiplied.
pub(crate) fn from_linear(sample: Sample) -> Sample {
    transfer(sample, srgb_from_linear)
}

fn transfer(sample: Sample, function: fn(f64) -> f64) -> Sample {
    let alpha = sample[ALPHA];
    let colour = unpremultiplied(sample);
    std::array::from_fn(|channel| {
        if channel == ALPHA {
            alpha
        } else {
            function(f64::from(colour[channel])) as f32 * alpha
        }
    })
}

fn unpremultiplied(sample: Sample) -> Rgb {
    let alpha = sample[ALPHA];
    std::array::from_fn(|channel| {
        if alpha > 0.0 {
            (sample[channel] / alpha).clamp(0.0, 1.0)
        } else {
            0.0
        }
    })
}

/// The §10 blend function `B(Cb, Cs)` of a separable or non-separable mode.
fn blend(mode: BlendMode, backdrop: Rgb, source: Rgb) -> Rgb {
    let separable = |function: fn(f32, f32) -> f32| -> Rgb {
        std::array::from_fn(|channel| function(backdrop[channel], source[channel]))
    };
    match mode {
        BlendMode::Normal | BlendMode::PlusDarker | BlendMode::PlusLighter => source,
        BlendMode::Multiply => separable(|b, s| b * s),
        BlendMode::Screen => separable(screen),
        BlendMode::Overlay => separable(|b, s| hard_light(s, b)),
        BlendMode::Darken => separable(f32::min),
        BlendMode::Lighten => separable(f32::max),
        BlendMode::ColorDodge => separable(|b, s| {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }),
        BlendMode::ColorBurn => separable(|b, s| {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - b) / s).min(1.0)
            }
        }),
        BlendMode::HardLight => separable(hard_light),
        BlendMode::SoftLight => separable(|b, s| {
            if s <= HALF {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= QUARTER {
                    ((16.0 * b - 12.0) * b + 4.0) * b
                } else {
                    b.sqrt()
                };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }),
        BlendMode::Difference => separable(|b, s| (b - s).abs()),
        BlendMode::Exclusion => separable(|b, s| b + s - 2.0 * b * s),
        BlendMode::Hue => set_luminosity(
            set_saturation(source, saturation(backdrop)),
            luminosity(backdrop),
        ),
        BlendMode::Saturation => set_luminosity(
            set_saturation(backdrop, saturation(source)),
            luminosity(backdrop),
        ),
        BlendMode::Color => set_luminosity(source, luminosity(backdrop)),
        BlendMode::Luminosity => set_luminosity(backdrop, luminosity(source)),
    }
}

fn screen(backdrop: f32, source: f32) -> f32 {
    backdrop + source - backdrop * source
}

fn hard_light(backdrop: f32, source: f32) -> f32 {
    if source <= HALF {
        backdrop * 2.0 * source
    } else {
        screen(backdrop, 2.0 * source - 1.0)
    }
}

fn luminosity(colour: Rgb) -> f32 {
    colour.iter().zip(LUMINOSITY).map(|(c, w)| c * w).sum()
}

fn set_luminosity(colour: Rgb, target: f32) -> Rgb {
    let shift = target - luminosity(colour);
    let shifted = colour.map(|channel| channel + shift);
    let lum = luminosity(shifted);
    let low = shifted.iter().copied().fold(f32::INFINITY, f32::min);
    let high = shifted.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    shifted.map(|channel| {
        if low < 0.0 {
            lum + (channel - lum) * lum / (lum - low)
        } else if high > 1.0 {
            lum + (channel - lum) * (1.0 - lum) / (high - lum)
        } else {
            channel
        }
    })
}

fn saturation(colour: Rgb) -> f32 {
    colour.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - colour.iter().copied().fold(f32::INFINITY, f32::min)
}

fn set_saturation(colour: Rgb, target: f32) -> Rgb {
    let high = colour.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let low = colour.iter().copied().fold(f32::INFINITY, f32::min);
    if high <= low {
        return [0.0; 3];
    }
    colour.map(|channel| (channel - low) * target / (high - low))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1.0e-5;

    fn close(actual: Sample, expected: Sample) -> bool {
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (a - e).abs() < EPSILON)
    }

    #[test]
    fn blending_over_a_transparent_backdrop_gives_the_source() {
        let source = [0.2, 0.1, 0.3, 0.5];
        for mode in [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Overlay,
            BlendMode::ColorBurn,
            BlendMode::SoftLight,
            BlendMode::Hue,
            BlendMode::Luminosity,
            BlendMode::PlusLighter,
            BlendMode::PlusDarker,
        ] {
            let result = composite(mode, source, [0.0; 4]);
            assert!(
                close(result, source),
                "§10: with αb = 0 the blend function drops out, so {mode:?} gives the source, got \
                 {result:?}"
            );
        }
    }

    #[test]
    fn opaque_layers_blend_by_the_mode_formula() {
        let source = [0.6, 0.2, 0.9, 1.0];
        let backdrop = [0.5, 0.8, 0.1, 1.0];
        for (mode, expected) in [
            (BlendMode::Normal, source),
            (BlendMode::Multiply, [0.3, 0.16, 0.09, 1.0]),
            (BlendMode::Screen, [0.8, 0.84, 0.91, 1.0]),
            (BlendMode::Darken, [0.5, 0.2, 0.1, 1.0]),
            (BlendMode::Difference, [0.1, 0.6, 0.8, 1.0]),
            (BlendMode::Exclusion, [0.5, 0.68, 0.82, 1.0]),
            // Overlay is hard light with the layers swapped: Cb ≤ 0.5 multiplies by 2Cb,
            // a lighter backdrop screens.
            (BlendMode::Overlay, [0.6, 0.68, 0.18, 1.0]),
            (BlendMode::PlusLighter, [1.0, 1.0, 1.0, 1.0]),
        ] {
            let result = composite(mode, source, backdrop);
            assert!(
                close(result, expected),
                "{mode:?} of {source:?} over {backdrop:?} gives {result:?}, expected {expected:?}"
            );
        }
    }

    #[test]
    fn luminosity_keeps_the_backdrop_hue_and_takes_the_source_luminosity() {
        let source = [0.5, 0.5, 0.5, 1.0];
        let backdrop = [0.2, 0.4, 0.6, 1.0];
        let result = composite(BlendMode::Luminosity, source, backdrop);
        let rgb = [result[0], result[1], result[2]];
        assert!(
            (luminosity(rgb) - 0.5).abs() < EPSILON
                && (rgb[1] - rgb[0] - 0.2).abs() < EPSILON
                && (rgb[2] - rgb[1] - 0.2).abs() < EPSILON,
            "SetLum shifts the backdrop by its luminosity difference: {rgb:?}"
        );
    }

    #[test]
    fn linear_light_round_trips_and_darkens_mid_greys() {
        let sample = [0.25, 0.1, 0.4, 0.5];
        let linear = to_linear(sample);
        assert!(
            linear[0] < sample[0] && (linear[ALPHA] - sample[ALPHA]).abs() < EPSILON,
            "sRGB 0.5 is about 0.214 in linear light, alpha unchanged: {linear:?}"
        );
        assert!(
            close(from_linear(linear), sample),
            "the transfer functions invert each other: {:?}",
            from_linear(linear)
        );
    }
}
