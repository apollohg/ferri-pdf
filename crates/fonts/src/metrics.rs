//! Face measurements that font-relative lengths (CSS Values 4 §6.1.1) and line boxes use.
use crate::{
    DEFAULT_FAMILY, FontBook, FontData, FontError, FontFamily, FontId, FontPreferences,
    FontSelection, GlyphMatch,
};
use ferri_pdf_resources::ResourceResolver;
use std::sync::OnceLock;
use ttf_parser::Face;

/// CSS Values 4 §6.1.1: an x-height that cannot be determined is 0.5em.
const FALLBACK_X_HEIGHT_EM: f32 = 0.5;
const LOWERCASE_X: char = 'x';
const CAPITAL_H: char = 'H';

/// Vertical metrics of one face, in em.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    /// OS/2 `sxHeight`, else the top of `x`, else 0.5em.
    pub x_height: f32,
    /// OS/2 `sCapHeight`, else the top of `H`, else the ascent.
    pub cap_height: f32,
}
impl FaceMetrics {
    pub fn of(face: &Face<'_>) -> Self {
        let units_per_em = f32::from(face.units_per_em());
        let glyph_top = |ch| {
            face.glyph_index(ch)
                .and_then(|glyph| face.glyph_bounding_box(glyph))
                .map(|bounds| bounds.y_max)
        };
        let ascent = f32::from(face.ascender()) / units_per_em;
        Self {
            ascent,
            descent: -f32::from(face.descender()) / units_per_em,
            line_gap: f32::from(face.line_gap()) / units_per_em,
            x_height: face
                .x_height()
                .filter(|height| *height > 0)
                .or_else(|| glyph_top(LOWERCASE_X))
                .map_or(FALLBACK_X_HEIGHT_EM, |height| {
                    f32::from(height) / units_per_em
                }),
            cap_height: face
                .capital_height()
                .filter(|height| *height > 0)
                .or_else(|| glyph_top(CAPITAL_H))
                .map_or(ascent, |height| f32::from(height) / units_per_em),
        }
    }
    /// `line-height: normal`.
    pub fn normal_line_height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }
}

/// The horizontal advance of `ch` in em, when the face has a glyph for it.
pub fn advance(face: &Face<'_>, ch: char) -> Option<f32> {
    let glyph = face.glyph_index(ch).filter(|glyph| glyph.0 != 0)?;
    Some(f32::from(face.glyph_hor_advance(glyph)?) / f32::from(face.units_per_em()))
}

impl FontBook {
    /// Metrics of the first face in `candidates` whose data is available.
    pub fn first_available_metrics(
        &self,
        candidates: &[FontId],
        selection: FontSelection,
        resolver: &dyn ResourceResolver,
    ) -> Result<FaceMetrics, FontError> {
        for &id in candidates {
            if let Some(instance) = self.instance(id, selection, resolver)? {
                return Ok(FaceMetrics::of(&instance.face));
            }
        }
        Err(FontError::NoAvailableFace)
    }
    /// The advance of `ch` in em in the first face of `candidates` that has it, as rendering
    /// would select it.
    pub fn advance(
        &self,
        candidates: &[FontId],
        selection: FontSelection,
        ch: char,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<f32>, FontError> {
        self.find_glyph(candidates, |id| -> Result<_, FontError> {
            let Some(instance) = self.instance(id, selection, resolver)? else {
                return Ok(GlyphMatch::Unavailable);
            };
            Ok(advance(&instance.face, ch).map_or(GlyphMatch::Missing, GlyphMatch::Found))
        })
        .map(|found| found.map(|(_, advance)| advance))
    }
}

/// The bundled faces the initial family resolves to. Where no element supplies a font,
/// relative lengths use initial values (Media Queries 4 §1.3).
pub struct InitialFont {
    faces: Vec<Face<'static>>,
}
impl InitialFont {
    /// Initial metrics use installed bundled faces, independent of document webfonts.
    pub fn from_preferences(preferences: &FontPreferences) -> Self {
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
                    FontData::Loaded(_) | FontData::Lazy { .. } => None,
                })
                .map(|data| parse(data).expect("bundled fonts are valid"))
                .collect(),
        }
    }
    pub fn metrics(&self) -> FaceMetrics {
        FaceMetrics::of(&self.faces[0])
    }
    /// The advance of `ch` in em in the first initial face that has it.
    pub fn advance(&self, ch: char) -> Option<f32> {
        self.faces.iter().find_map(|face| advance(face, ch))
    }
}

pub fn initial_font() -> &'static InitialFont {
    static INITIAL: OnceLock<InitialFont> = OnceLock::new();
    INITIAL.get_or_init(|| InitialFont::from_preferences(&FontPreferences::default()))
}

fn parse(data: &[u8]) -> Result<Face<'_>, FontError> {
    Face::parse(data, 0).map_err(FontError::InvalidFont)
}
