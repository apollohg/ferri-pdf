use crate::layout::TextLine;
use crate::writer::PdfError;
use ferri_pdf_fonts::{FontBook, FontId};
use ferri_pdf_text::PositionedGlyph;
use krilla::{
    geom::{Point, Transform},
    surface::Surface,
    text::{Font, Glyph, GlyphId, Tag},
};
use std::collections::HashMap;
use std::ops::Range;

// Synthetic bold stroke width.
const SYNTHETIC_BOLD_OFFSET_EM: f32 = 0.02;

struct PdfGlyph<'a> {
    glyph: &'a PositionedGlyph,
    advance: f32,
}
impl Glyph for PdfGlyph<'_> {
    fn glyph_id(&self) -> GlyphId {
        GlyphId::new(self.glyph.id)
    }
    fn text_range(&self) -> Range<usize> {
        self.glyph.text_range.clone()
    }
    fn x_advance(&self, size: f32) -> f32 {
        self.advance * size
    }
    fn x_offset(&self, size: f32) -> f32 {
        self.glyph.x_offset * size
    }
    fn y_offset(&self, size: f32) -> f32 {
        self.glyph.y_offset * size
    }
    fn y_advance(&self, size: f32) -> f32 {
        self.glyph.y_advance * size
    }
    fn location(&self) -> Option<krilla::surface::Location> {
        None
    }
}

type InstanceKey = (FontId, Vec<([u8; 4], u32)>);

pub(super) struct TextWriter<'a> {
    fonts: &'a FontBook,
    data: Vec<Option<krilla::Data>>,
    embedded: HashMap<InstanceKey, Font>,
    glyphs: Vec<PdfGlyph<'a>>,
}
impl<'a> TextWriter<'a> {
    pub(crate) fn new(fonts: &'a FontBook) -> Self {
        Self {
            fonts,
            data: vec![None; fonts.faces().len()],
            embedded: HashMap::new(),
            glyphs: Vec::new(),
        }
    }
    pub(crate) fn draw(
        &mut self,
        line: &'a TextLine,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        if line.runs.iter().any(|run| run.rtl || run.has_bidi_scopes()) {
            surface.with_actual_text(&line.text, |surface| self.draw_runs(line, surface))
        } else {
            self.draw_runs(line, surface)
        }
    }
    fn draw_runs(&mut self, line: &'a TextLine, surface: &mut Surface<'_>) -> Result<(), PdfError> {
        for run in &line.runs {
            if run.font_size == 0.0 {
                continue;
            }
            let x = line.x + run.x;
            let key = (
                run.font_id,
                run.variations
                    .iter()
                    .map(|variation| (variation.tag, variation.value.to_bits()))
                    .collect(),
            );
            let font = if let Some(font) = self.embedded.get(&key) {
                font.clone()
            } else {
                let data = self
                    .data
                    .get_mut(run.font_id.index())
                    .ok_or(PdfError::InvalidFontId(run.font_id))?;
                if data.is_none() {
                    let face = self
                        .fonts
                        .face(run.font_id)
                        .ok_or(PdfError::InvalidFontId(run.font_id))?;
                    *data = Some(face.data().ok_or(PdfError::InvalidFont)?.to_vec().into());
                }
                let coordinates: Vec<_> = run
                    .variations
                    .iter()
                    .map(|variation| (Tag::new(&variation.tag), variation.value))
                    .collect();
                let font = Font::new_variable(data.as_ref().unwrap().clone(), 0, &coordinates)
                    .ok_or(PdfError::InvalidFont)?;
                self.embedded.insert(key, font.clone());
                font
            };
            self.glyphs.clear();
            for glyph in run.glyphs() {
                self.glyphs.push(PdfGlyph {
                    glyph,
                    advance: glyph
                        .normalized_advance(run.font_size)
                        .ok_or(PdfError::InvalidGeometry)?,
                });
            }
            surface.set_fill(Some(crate::writer::painting::fill(run.color)?));
            let oblique = run.synthesis.oblique;
            let baseline = line.baseline + run.baseline_shift;
            if oblique {
                surface.push_transform(&Transform::from_row(
                    1.0,
                    0.0,
                    -run.oblique_skew,
                    1.0,
                    run.oblique_skew * baseline,
                    0.0,
                ));
            }
            let origin = Point::from_xy(x, baseline);
            surface.draw_glyphs(
                origin,
                &self.glyphs,
                font.clone(),
                &run.text,
                run.font_size,
                false,
            );
            if run.synthesis.bold {
                surface.draw_glyphs(
                    Point::from_xy(
                        origin.x + run.font_size * SYNTHETIC_BOLD_OFFSET_EM,
                        origin.y,
                    ),
                    &self.glyphs,
                    font.clone(),
                    &run.text,
                    run.font_size,
                    false,
                );
            }
            if oblique {
                surface.pop();
            }
        }
        Ok(())
    }
}
