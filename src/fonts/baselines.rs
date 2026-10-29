//! Horizontal and vertical baseline sets (CSS Inline 3 §3.3 and Writing Modes 4 §4).
use skrifa::raw::{FontRef, TableProvider, types::Tag};
use ttf_parser::Face;

const ROMAN: Tag = Tag::new(b"romn");
const IDEOGRAPHIC_UNDER: Tag = Tag::new(b"ideo");
const IDEOGRAPHIC_OVER: Tag = Tag::new(b"idtp");
const IDEOGRAPHIC_INK_UNDER: Tag = Tag::new(b"icfb");
const IDEOGRAPHIC_INK_OVER: Tag = Tag::new(b"icft");
const HANGING: Tag = Tag::new(b"hang");
const MATH: Tag = Tag::new(b"math");
/// The script whose baselines apply when the table has no default script record.
const DEFAULT_SCRIPT: Tag = Tag::new(b"DFLT");
const LATIN_SCRIPT: Tag = Tag::new(b"latn");
/// Ideographic-over and ideographic-under baselines are typically 1em apart.
const IDEOGRAPHIC_EM: f32 = 1.0;
/// The minus sign's centre is the math baseline; the top of Devanagari Ka is the hanging one.
const MINUS_SIGN: char = '\u{2212}';
const DEVANAGARI_KA: char = '\u{0915}';
/// The bounding box of 永 gives the ideographic character face edges.
const ETERNITY_IDEOGRAPH: char = '\u{6C38}';
/// The suggested hanging baseline where the font gives no way to find it.
const FALLBACK_HANGING_EM: f32 = 0.6;

/// Baselines above the horizontal alphabetic or vertical central origin, in em.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BaselineTable {
    pub alphabetic: f32,
    pub ideographic_over: f32,
    pub ideographic_under: f32,
    pub ideographic_ink_over: f32,
    pub ideographic_ink_under: f32,
    pub hanging: f32,
    pub math: f32,
}

/// The coordinates a `BASE` axis gives the default or Latin script, by tag, in
/// font units above its roman baseline.
fn recorded(face: &Face<'_>, vertical: bool) -> Vec<(Tag, f32)> {
    let read = || -> Option<Vec<(Tag, f32)>> {
        let font = FontRef::new(face.raw_face().data).ok()?;
        let base = font.base().ok()?;
        let axis = if vertical {
            base.vert_axis()?
        } else {
            base.horiz_axis()?
        }
        .ok()?;
        let tags = axis.base_tag_list()?.ok()?;
        let scripts = axis.base_script_list().ok()?;
        let records = scripts.base_script_records();
        let record = records
            .iter()
            .find(|record| record.base_script_tag() == DEFAULT_SCRIPT)
            .or_else(|| {
                records
                    .iter()
                    .find(|record| record.base_script_tag() == LATIN_SCRIPT)
            })
            .or_else(|| records.first())?;
        let values = record
            .base_script(scripts.offset_data())
            .ok()?
            .base_values()?
            .ok()?;
        let coordinates: Vec<(Tag, f32)> = tags
            .baseline_tags()
            .iter()
            .zip(values.base_coords().iter())
            .filter_map(|(tag, coordinate)| {
                Some((tag.get(), f32::from(coordinate.ok()?.coordinate())))
            })
            .collect();
        let roman = coordinates
            .iter()
            .find(|(tag, _)| *tag == ROMAN)
            .map_or(0.0, |(_, coordinate)| *coordinate);
        Some(
            coordinates
                .into_iter()
                .map(|(tag, coordinate)| (tag, coordinate - roman))
                .collect(),
        )
    };
    read().unwrap_or_default()
}

/// The bottom and top of `ch`'s glyph, in font units.
fn glyph_extent(face: &Face<'_>, ch: char, vertical: bool) -> Option<[f32; 2]> {
    let glyph = face.glyph_index(ch).filter(|glyph| glyph.0 != 0)?;
    let bounds = face.glyph_bounding_box(glyph)?;
    Some(if vertical {
        let origin = f32::from(face.glyph_hor_advance(glyph)?) / 2.0;
        [bounds.x_min, bounds.x_max].map(|edge| f32::from(edge) - origin)
    } else {
        [bounds.y_min, bounds.y_max].map(f32::from)
    })
}

impl BaselineTable {
    pub(crate) fn upright(self, face: &Face<'_>) -> Self {
        let units = f32::from(face.units_per_em());
        let recorded = recorded(face, true);
        let base = |tag| {
            recorded
                .iter()
                .find(|(found, _)| *found == tag)
                .map(|(_, coordinate)| coordinate / units)
        };
        let (over, under, center, alphabetic) = if recorded.is_empty() {
            let center = f32::midpoint(self.ideographic_over, self.ideographic_under);
            (IDEOGRAPHIC_EM / 2.0, -IDEOGRAPHIC_EM / 2.0, 0.0, -center)
        } else {
            let under = base(IDEOGRAPHIC_UNDER)
                .or_else(|| base(IDEOGRAPHIC_OVER).map(|over| over - IDEOGRAPHIC_EM))
                .unwrap_or(0.0);
            let over = base(IDEOGRAPHIC_OVER).unwrap_or(under + IDEOGRAPHIC_EM);
            let center = f32::midpoint(over, under);
            (over, under, center, -center)
        };
        let glyph =
            |ch| glyph_extent(face, ch, true).map(|bounds| bounds.map(|value| value / units));
        let ink = glyph(ETERNITY_IDEOGRAPH);
        Self {
            alphabetic,
            ideographic_over: over - center,
            ideographic_under: under - center,
            ideographic_ink_over: base(IDEOGRAPHIC_INK_OVER)
                .map(|value| value - center)
                .or(ink.map(|[_, over]| over))
                .unwrap_or(over - center),
            ideographic_ink_under: base(IDEOGRAPHIC_INK_UNDER)
                .map(|value| value - center)
                .or(ink.map(|[under, _]| under))
                .unwrap_or(under - center),
            hanging: base(HANGING)
                .map(|value| value - center)
                .or_else(|| glyph(DEVANAGARI_KA).map(|[_, over]| over))
                .unwrap_or(self.hanging + alphabetic),
            math: base(MATH)
                .map(|value| value - center)
                .or_else(|| glyph(MINUS_SIGN).map(|[under, over]| f32::midpoint(under, over)))
                .unwrap_or(self.math + alphabetic),
        }
    }

    /// `ascent` and `descent` are the face's, and `x_height` its x-height, in em.
    pub(crate) fn of(face: &Face<'_>, ascent: f32, descent: f32, x_height: f32) -> Self {
        let units = f32::from(face.units_per_em());
        let recorded = recorded(face, false);
        let base = |wanted: Tag| {
            recorded
                .iter()
                .find(|(tag, _)| *tag == wanted)
                .map(|(_, coordinate)| coordinate / units)
        };
        let [ideographic_over, ideographic_under] =
            match (base(IDEOGRAPHIC_OVER), base(IDEOGRAPHIC_UNDER)) {
                (Some(over), Some(under)) => [over, under],
                (Some(over), None) => [over, over - IDEOGRAPHIC_EM],
                (None, Some(under)) => [under + IDEOGRAPHIC_EM, under],
                // CJK fonts' ascent and descent typically match the ideographic edges.
                (None, None) => [ascent, -descent],
            };
        let math = base(MATH)
            .or_else(|| {
                let axis = face.tables().math?.constants?.axis_height().value;
                Some(f32::from(axis) / units)
            })
            .or_else(|| {
                glyph_extent(face, MINUS_SIGN, false)
                    .map(|[bottom, top]| f32::midpoint(bottom, top) / units)
            })
            .unwrap_or(x_height / 2.0);
        let hanging = base(HANGING)
            .or_else(|| glyph_extent(face, DEVANAGARI_KA, false).map(|[_, top]| top / units))
            .unwrap_or(FALLBACK_HANGING_EM);
        let face_edges = glyph_extent(face, ETERNITY_IDEOGRAPH, false)
            .map(|edges| edges.map(|edge| edge / units));
        let ideographic_ink_over = base(IDEOGRAPHIC_INK_OVER)
            .or(face_edges.map(|[_, top]| top))
            .unwrap_or(ideographic_over);
        let ideographic_ink_under = base(IDEOGRAPHIC_INK_UNDER)
            .or(face_edges.map(|[bottom, _]| bottom))
            .unwrap_or(ideographic_under);
        Self {
            alphabetic: 0.0,
            ideographic_over,
            ideographic_under,
            ideographic_ink_over,
            ideographic_ink_under,
            hanging,
            math,
        }
    }
}
