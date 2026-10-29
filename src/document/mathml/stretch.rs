//! `MathML` Core §5.3.1–§5.3.2: size variants and connector-overlapped assemblies.
use crate::document::DocumentError;
use crate::text::PositionedGlyph;
use ttf_parser::{Face, GlyphId, math::GlyphAssembly};

const MAX_ASSEMBLY_GLYPHS: usize = 4096;
pub(super) struct Shape {
    pub glyphs: Vec<PositionedGlyph>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub italic: Option<f32>,
}
fn positioned(glyph: GlyphId, width: f32) -> PositionedGlyph {
    PositionedGlyph {
        id: u32::from(glyph.0),
        text_range: 0..0,
        x_advance: width,
        x_spacing: 0.0,
        x_offset: 0.0,
        y_offset: 0.0,
        y_advance: 0.0,
    }
}
fn single(face: &Face<'_>, glyph: GlyphId) -> Option<Shape> {
    let bounds = face.glyph_bounding_box(glyph)?;
    let em = f32::from(face.units_per_em());
    let width = f32::from(face.glyph_hor_advance(glyph)?) / em;
    Some(Shape {
        glyphs: vec![positioned(glyph, width)],
        width,
        ascent: f32::from(bounds.y_max) / em,
        descent: -f32::from(bounds.y_min) / em,
        italic: face
            .tables()
            .math
            .and_then(|table| table.glyph_info)
            .and_then(|info| info.italic_corrections)
            .and_then(|values| values.get(glyph))
            .map(|value| f32::from(value.value) / em),
    })
}
pub(super) fn shape(
    face: &Face<'_>,
    glyph: GlyphId,
    target: f32,
    horizontal: bool,
    variants_only: bool,
) -> Result<Option<Shape>, DocumentError> {
    let Some(variants) = face.tables().math.and_then(|math| math.variants) else {
        return Ok(None);
    };
    let construction = if horizontal {
        variants.horizontal_constructions.get(glyph)
    } else {
        variants.vertical_constructions.get(glyph)
    };
    let Some(construction) = construction else {
        return Ok(None);
    };
    let em = f32::from(face.units_per_em());
    let mut last = single(face, glyph);
    if !variants_only
        && last.as_ref().is_some_and(|shape| {
            if horizontal {
                shape.width >= target
            } else {
                shape.ascent + shape.descent >= target
            }
        })
    {
        return Ok(last);
    }
    for variant in construction.variants {
        let Some(next) = single(face, variant.variant_glyph) else {
            continue;
        };
        last = Some(next);
        if f32::from(variant.advance_measurement) / em >= target {
            return Ok(last);
        }
    }
    if !variants_only
        && let Some(assembly) = construction.assembly
        && let Some(assembled) = assemble(
            face,
            assembly,
            f32::from(variants.min_connector_overlap),
            target * em,
            horizontal,
        )?
    {
        return Ok(Some(assembled));
    }
    Ok(last)
}
fn assemble(
    face: &Face<'_>,
    assembly: GlyphAssembly<'_>,
    minimum_overlap: f32,
    target: f32,
    horizontal: bool,
) -> Result<Option<Shape>, DocumentError> {
    let parts: Vec<_> = assembly.parts.into_iter().collect();
    let (mut extenders, mut non_extenders, mut extender_advance, mut fixed_advance) =
        (0usize, 0usize, 0.0, 0.0);
    for part in &parts {
        if f32::from(part.start_connector_length) < minimum_overlap
            || f32::from(part.end_connector_length) < minimum_overlap
        {
            return Ok(None);
        }
        if part.part_flags.extender() {
            extenders += 1;
            extender_advance += f32::from(part.full_advance);
        } else {
            non_extenders += 1;
            fixed_advance += f32::from(part.full_advance);
        }
    }
    let growth = extender_advance - minimum_overlap * extenders as f32;
    if extenders == 0 || growth <= 0.0 {
        return Ok(None);
    }
    let repetitions = ((target - fixed_advance + minimum_overlap * (non_extenders as f32 - 1.0))
        / growth)
        .ceil()
        .max(0.0);
    if !repetitions.is_finite() || repetitions > MAX_ASSEMBLY_GLYPHS as f32 {
        return Err(DocumentError::Formatting(format!(
            "MathML glyph assembly exceeds {MAX_ASSEMBLY_GLYPHS} glyphs"
        )));
    }
    let repetitions = repetitions as usize;
    let Some(count) = extenders
        .checked_mul(repetitions)
        .and_then(|count| count.checked_add(non_extenders))
        .filter(|count| *count <= MAX_ASSEMBLY_GLYPHS)
    else {
        return Err(DocumentError::Formatting(format!(
            "MathML glyph assembly exceeds {MAX_ASSEMBLY_GLYPHS} glyphs"
        )));
    };
    let total = fixed_advance + repetitions as f32 * extender_advance;
    let mut overlap = if count <= 1 {
        0.0
    } else {
        (total - target) / (count - 1) as f32
    };
    if count > 1 {
        for (index, part) in parts.iter().enumerate() {
            if part.part_flags.extender() || index + 1 < parts.len() {
                overlap = overlap.min(f32::from(part.start_connector_length));
            }
            if part.part_flags.extender() || index > 0 {
                overlap = overlap.min(f32::from(part.end_connector_length));
            }
        }
        overlap = overlap.max(minimum_overlap);
    }
    let stretch = total - overlap * count.saturating_sub(1) as f32;
    let em = f32::from(face.units_per_em());
    let (mut width, mut ascent, mut descent) = (0.0f32, 0.0f32, 0.0f32);
    let mut glyphs = Vec::with_capacity(count);
    let mut position = 0.0;
    for part in &parts {
        let Some(bounds) = face.glyph_bounding_box(part.glyph_id) else {
            return Ok(None);
        };
        let Some(advance) = face.glyph_hor_advance(part.glyph_id) else {
            return Ok(None);
        };
        width = width.max(f32::from(advance));
        ascent = ascent.max(f32::from(bounds.y_max));
        descent = descent.max(-f32::from(bounds.y_min));
        let copies = if part.part_flags.extender() {
            repetitions
        } else {
            1
        };
        for _ in 0..copies {
            let mut glyph = positioned(part.glyph_id, 0.0);
            if horizontal {
                glyph.x_offset = position / em;
            } else {
                glyph.y_offset = (-position + f32::from(bounds.y_min)) / em;
            }
            glyphs.push(glyph);
            position += f32::from(part.full_advance) - overlap;
        }
    }
    if horizontal {
        width = stretch;
    } else {
        ascent = stretch;
        descent = 0.0;
    }
    if let Some(last) = glyphs.last_mut() {
        last.x_advance = width / em;
    }
    Ok(Some(Shape {
        glyphs,
        width: width / em,
        ascent: ascent / em,
        descent: descent / em,
        italic: Some(f32::from(assembly.italics_correction.value) / em),
    }))
}
