//! Face measurements that font-relative lengths (CSS Values 4 §6.1.1), line boxes and
//! `font-size-adjust` (CSS Fonts 5 §3.4) use.
use crate::fonts::{
    DEFAULT_FAMILY, FontAdjustments, FontBook, FontData, FontError, FontFamily, FontId,
    FontInstance, FontPreferences, FontSelection, GlyphMatch,
};
use crate::resources::ResourceResolver;
use serde::Serialize;
use std::sync::OnceLock;
use ttf_parser::Face;

/// CSS Values 4 §6.1.1: an x-height that cannot be determined is 0.5em.
const FALLBACK_X_HEIGHT_EM: f32 = 0.5;
const HALF_EM: f32 = 0.5;
const LOWERCASE_X: char = 'x';
const CAPITAL_H: char = 'H';
/// `ch` measures the advance of "0" and `ic` that of "水" (CSS Values 4 §6.1.1).
pub(crate) const ZERO: char = '0';
pub(crate) const WATER_IDEOGRAPH: char = '\u{6c34}';
/// CSS Values 4 §6.1.1 sizes where the font has no such glyph.
pub(crate) const FALLBACK_ZERO_ADVANCE_EM: f32 = 0.5;
pub(crate) const FALLBACK_IDEOGRAPH_ADVANCE_EM: f32 = 1.0;
const OS2_TABLE: &[u8; 4] = b"OS/2";
const OS2_AVERAGE_WIDTH: usize = 2;
const HHEA_TABLE: &[u8; 4] = b"hhea";
const HHEA_MAXIMUM_ADVANCE: usize = 10;

/// Vertical metrics of one face, in em.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FaceMetrics {
    pub ascent: f32,
    pub math_scaling: Option<[f32; 2]>,
    pub descent: f32,
    pub line_gap: f32,
    /// OS/2 `sxHeight`, else the top of `x`, else 0.5em.
    pub x_height: f32,
    /// OS/2 `sCapHeight`, else the top of `H`, else the ascent.
    pub cap_height: f32,
    /// Advances of "0" and "水", and the vertical advance of "水", where the face has them.
    pub zero_advance: Option<f32>,
    pub ideograph_advance: Option<f32>,
    pub ideograph_height: Option<f32>,
    /// OS/2 `xAvgCharWidth` and hhea `advanceWidthMax`, which size form controls by
    /// characters (HTML §15.5.6); the advance of "0" stands in where a face lacks them.
    pub average_advance: f32,
    pub maximum_advance: f32,
    pub baselines: crate::fonts::BaselineTable,
}
impl FaceMetrics {
    pub(crate) fn of(face: &Face<'_>) -> Self {
        let units_per_em = f32::from(face.units_per_em());
        let glyph_top = |ch| {
            face.glyph_index(ch)
                .and_then(|glyph| face.glyph_bounding_box(glyph))
                .map(|bounds| bounds.y_max)
        };
        let ascent = f32::from(face.ascender()) / units_per_em;
        let descent = -f32::from(face.descender()) / units_per_em;
        let x_height = face
            .x_height()
            .filter(|height| *height > 0)
            .or_else(|| glyph_top(LOWERCASE_X))
            .map_or(FALLBACK_X_HEIGHT_EM, |height| {
                f32::from(height) / units_per_em
            });
        let zero_advance = advance(face, ZERO);
        let fallback = zero_advance.unwrap_or(FALLBACK_ZERO_ADVANCE_EM);
        let table_advance = |tag: &[u8; 4], offset: usize| {
            face.raw_face()
                .table(ttf_parser::Tag::from_bytes(tag))
                .and_then(|table| table.get(offset..offset + 2))
                .map(|bytes| i16::from_be_bytes([bytes[0], bytes[1]]))
                .filter(|value| *value > 0)
                .map(|value| f32::from(value) / units_per_em)
        };
        Self {
            math_scaling: face
                .tables()
                .math
                .and_then(|math| math.constants)
                .map(|constants| {
                    const PERCENT: f32 = 100.0;
                    [
                        f32::from(constants.script_percent_scale_down()) / PERCENT,
                        f32::from(constants.script_script_percent_scale_down()) / PERCENT,
                    ]
                }),
            average_advance: table_advance(OS2_TABLE, OS2_AVERAGE_WIDTH).unwrap_or(fallback),
            maximum_advance: table_advance(HHEA_TABLE, HHEA_MAXIMUM_ADVANCE).unwrap_or(fallback),
            ascent,
            descent,
            line_gap: f32::from(face.line_gap()) / units_per_em,
            x_height,
            cap_height: face
                .capital_height()
                .filter(|height| *height > 0)
                .or_else(|| glyph_top(CAPITAL_H))
                .map_or(ascent, |height| f32::from(height) / units_per_em),
            zero_advance,
            ideograph_advance: advance(face, WATER_IDEOGRAPH),
            ideograph_height: face
                .glyph_index(WATER_IDEOGRAPH)
                .filter(|glyph| glyph.0 != 0)
                .and_then(|glyph| face.glyph_ver_advance(glyph))
                .map(|height| f32::from(height) / units_per_em),
            baselines: crate::fonts::BaselineTable::of(face, ascent, descent, x_height),
        }
    }
    /// The horizontal line metrics `overrides` replace; upright typesetting keeps the font's
    /// vertical metrics (CSS Fonts 5 §4.5).
    pub(crate) fn overridden(mut self, overrides: crate::fonts::MetricOverrides) -> Self {
        self.ascent = overrides.ascent.unwrap_or(self.ascent);
        self.descent = overrides.descent.unwrap_or(self.descent);
        self.line_gap = overrides.line_gap.unwrap_or(self.line_gap);
        self
    }
    pub(crate) fn upright(mut self, face: &Face<'_>) -> Self {
        let units = f32::from(face.units_per_em());
        self.ascent = face
            .vertical_ascender()
            .map_or(HALF_EM, |value| f32::from(value) / units);
        self.descent = face
            .vertical_descender()
            .map_or(HALF_EM, |value| -f32::from(value) / units);
        self.line_gap = face
            .vertical_line_gap()
            .map_or(0.0, |value| f32::from(value) / units);
        self.baselines = self.baselines.upright(face);
        self
    }
    /// `line-height: normal`.
    pub(crate) fn normal_line_height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }
    /// The metric `font-size-adjust` normalizes, in em. Missing or zero glyph measures use
    /// the CSS Values 4 §6.1.1 fallbacks of the corresponding units.
    pub(crate) fn size_metric(&self, metric: SizeMetric) -> f32 {
        let measured =
            |value: Option<f32>, fallback| value.filter(|v| *v > 0.0).unwrap_or(fallback);
        match metric {
            SizeMetric::ExHeight => self.x_height,
            SizeMetric::CapHeight => self.cap_height,
            SizeMetric::ChWidth => measured(self.zero_advance, FALLBACK_ZERO_ADVANCE_EM),
            SizeMetric::IcWidth => measured(self.ideograph_advance, FALLBACK_IDEOGRAPH_ADVANCE_EM),
            SizeMetric::IcHeight => measured(self.ideograph_height, FALLBACK_IDEOGRAPH_ADVANCE_EM),
        }
    }
}

/// The font metric `font-size-adjust` normalizes (CSS Fonts 5 §3.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SizeMetric {
    ExHeight,
    CapHeight,
    ChWidth,
    IcWidth,
    IcHeight,
}

/// A computed `font-size-adjust`: the metric, as a proportion of the computed font size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct SizeAdjust {
    pub metric: SizeMetric,
    pub value: f32,
}
impl SizeAdjust {
    /// `u / s = m / m′` for a face with metrics `metrics`.
    pub(crate) fn scale(self, metrics: &FaceMetrics) -> f32 {
        let actual = metrics.size_metric(self.metric);
        if actual > 0.0 {
            self.value / actual
        } else {
            1.0
        }
    }
}

/// The horizontal advance of `ch` in em, when the face has a glyph for it.
pub(crate) fn advance(face: &Face<'_>, ch: char) -> Option<f32> {
    advance_in_axis(face, ch, false)
}

pub(crate) fn advance_in_axis(face: &Face<'_>, ch: char, vertical: bool) -> Option<f32> {
    let glyph = face.glyph_index(ch).filter(|glyph| glyph.0 != 0)?;
    let advance = if vertical {
        face.glyph_ver_advance(glyph)
            .unwrap_or_else(|| face.units_per_em())
    } else {
        face.glyph_hor_advance(glyph)?
    };
    Some(f32::from(advance) / f32::from(face.units_per_em()))
}

impl FontBook {
    /// The face that renders text at computed `size`, and its used size: the `size-adjust`
    /// descriptor scales the size, `font-size-adjust` then normalizes it (CSS Fonts 5 §4.4),
    /// and optical sizing follows the adjusted size (CSS Fonts 4 §8.1).
    pub(crate) fn sized_instance(
        &self,
        id: FontId,
        selection: FontSelection,
        adjustments: &FontAdjustments,
        size: f32,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<(FontInstance<'_>, f32)>, FontError> {
        let Some(instance) = self.instance(id, selection, adjustments, size, resolver)? else {
            return Ok(None);
        };
        let descriptor = self
            .face(id)
            .ok_or(FontError::UnknownFont(id))?
            .settings
            .size_adjust;
        // `size-adjust` scales the metric `font-size-adjust` reads as much as the size, so
        // the normalized size does not depend on it. A size past the finite range clamps to
        // it, as an infinite `calc()` length does (CSS Values 4 §10.9).
        let used = adjustments
            .size_adjust
            .map_or(size * descriptor, |adjust| {
                size * adjust.scale(&FaceMetrics::of(&instance.face))
            })
            .min(f32::MAX);
        if used == size {
            return Ok(Some((instance, size)));
        }
        let optical = adjustments.optical_sizing
            && instance
                .face
                .variation_axes()
                .into_iter()
                .any(|axis| axis.tag == super::loading::OPTICAL_SIZE_AXIS);
        if optical {
            return Ok(self
                .instance(id, selection, adjustments, used, resolver)?
                .map(|instance| (instance, used)));
        }
        Ok(Some((instance, used)))
    }

    /// Metrics, in em, of the first face in `candidates` whose data is available, and its used
    /// size per computed size.
    pub(crate) fn first_available_metrics(
        &self,
        candidates: &[FontId],
        selection: FontSelection,
        adjustments: &FontAdjustments,
        size: f32,
        resolver: &dyn ResourceResolver,
    ) -> Result<(FaceMetrics, f32), FontError> {
        for id in self.first_available_candidates(candidates) {
            if let Some((instance, used)) =
                self.sized_instance(id, selection, adjustments, size, resolver)?
            {
                return Ok((instance.metrics(), scale(used, size)));
            }
        }
        Err(FontError::NoAvailableFace)
    }
    /// The advance of `ch` in em in the first face of `candidates` that has it, as rendering
    /// would select it, and that face's used size per computed size.
    pub(crate) fn advance(
        &self,
        candidates: &[FontId],
        selection: FontSelection,
        adjustments: &FontAdjustments,
        size: f32,
        (ch, vertical): (char, bool),
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<(f32, f32)>, FontError> {
        let mut buffer = [0; 4];
        self.find_glyph(
            candidates,
            ch.encode_utf8(&mut buffer),
            |id| -> Result<_, FontError> {
                let Some((instance, used)) =
                    self.sized_instance(id, selection, adjustments, size, resolver)?
                else {
                    return Ok(GlyphMatch::Unavailable);
                };
                Ok(advance_in_axis(&instance.face, ch, vertical)
                    .map_or(GlyphMatch::Missing, |advance| {
                        GlyphMatch::Found((advance, scale(used, size)))
                    }))
            },
        )
        .map(|found| found.map(|(_, advance)| advance))
    }
}

/// Used size per computed size; a zero computed size has no adjustment to apply.
fn scale(used: f32, size: f32) -> f32 {
    if size == 0.0 { 1.0 } else { used / size }
}

/// The bundled faces the initial family resolves to. Where no element supplies a font,
/// relative lengths use initial values (Media Queries 4 §1.3).
pub(crate) struct InitialFont {
    faces: Vec<Face<'static>>,
}
impl InitialFont {
    /// Initial metrics use installed bundled faces, independent of document webfonts.
    pub(crate) fn from_preferences(preferences: &FontPreferences) -> Self {
        let book = FontBook::with_preferences(preferences.clone());
        let candidates = book.candidates(
            &[FontFamily::Generic(DEFAULT_FAMILY)],
            FontSelection::default(),
        );
        Self {
            faces: candidates
                .iter()
                .filter_map(|id| match book.face(*id)?.data {
                    FontData::Bundled(data) => Some(data),
                    #[cfg(test)]
                    FontData::Loaded(_) => None,
                    FontData::Lazy { .. } => None,
                })
                .map(|data| parse(data).expect("bundled fonts are valid"))
                .collect(),
        }
    }
    pub(crate) fn metrics(&self) -> FaceMetrics {
        FaceMetrics::of(&self.faces[0])
    }
    /// The advance of `ch` in em in the first initial face that has it.
    pub(crate) fn advance(&self, (ch, vertical): (char, bool)) -> Option<f32> {
        self.faces
            .iter()
            .find_map(|face| advance_in_axis(face, ch, vertical))
    }
}

pub(crate) fn initial_font() -> &'static InitialFont {
    static INITIAL: OnceLock<InitialFont> = OnceLock::new();
    INITIAL.get_or_init(|| InitialFont::from_preferences(&FontPreferences::default()))
}

fn parse(data: &[u8]) -> Result<Face<'_>, FontError> {
    Face::parse(data, 0).map_err(FontError::InvalidFont)
}
