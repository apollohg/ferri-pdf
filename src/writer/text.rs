use crate::fonts::{FontBook, FontId};
use crate::layout::TextLine;
use crate::text::{GlyphRun, PositionedGlyph};
use crate::writer::PdfError;
use krilla::{
    geom::{Point, Transform},
    surface::Surface,
    text::{Font, Glyph, GlyphId, PaletteColor, Tag},
};
use std::collections::HashMap;
use std::ops::Range;

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
    // Krilla's vertical offsets and advances point up, as OpenType's do; positioned glyphs'
    // point down.
    fn y_offset(&self, size: f32) -> f32 {
        -self.glyph.y_offset * size
    }
    fn y_advance(&self, size: f32) -> f32 {
        -self.glyph.y_advance * size
    }
    fn location(&self) -> Option<krilla::surface::Location> {
        None
    }
}

/// A face's variation coordinates and palette entries, which select distinct PDF fonts.
type InstanceKey = (FontId, Vec<([u8; 4], u32)>, Option<Vec<PaletteColor>>);

const CHANNEL_MAX: f32 = 255.0;

/// OpenType tables holding glyphs drawn in their own fixed colours: bitmaps and SVG.
const FIXED_COLOR_TABLES: [&[u8; 4]; 3] = [b"sbix", b"CBDT", b"SVG "];

/// A `font-palette` colour as a krilla palette entry; colours are already sRGB.
fn palette_color(color: crate::graphics::Color) -> Result<PaletteColor, PdfError> {
    let alpha = (color.alpha * CHANNEL_MAX).round();
    if !(0.0..=CHANNEL_MAX).contains(&alpha) {
        return Err(PdfError::InvalidAlpha(color.alpha));
    }
    let [red, green, blue] = color.rgb;
    Ok([red, green, blue, alpha as u8])
}

pub(super) struct TextWriter<'a> {
    fonts: &'a FontBook,
    data: Vec<Option<krilla::Data>>,
    embedded: HashMap<InstanceKey, Font>,
}
impl<'a> TextWriter<'a> {
    pub(crate) fn new(fonts: &'a FontBook) -> Self {
        Self {
            fonts,
            data: vec![None; fonts.faces().len()],
            embedded: HashMap::new(),
        }
    }
    /// Whether every colour `runs` paint comes from their colours, strokes, palettes and
    /// marks, which excludes bitmap and SVG colour glyphs.
    pub(crate) fn paints_run_colors(&self, runs: &[GlyphRun]) -> Result<bool, PdfError> {
        for run in runs {
            let face = self
                .fonts
                .face(run.font_id)
                .ok_or(PdfError::InvalidFontId(run.font_id))?;
            let data = face.data().ok_or(PdfError::InvalidFont)?;
            let parsed = ttf_parser::RawFace::parse(data, 0).map_err(|_| PdfError::InvalidFont)?;
            if FIXED_COLOR_TABLES
                .iter()
                .any(|tag| parsed.table(ttf_parser::Tag::from_bytes(tag)).is_some())
                || !self.paints_run_colors(&run.marks)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(crate) fn draw(
        &mut self,
        line: &TextLine,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let origin = [line.x, line.baseline];
        if line.runs.iter().any(|run| run.rtl || run.has_bidi_scopes()) {
            surface.with_actual_text(&line.text, |surface| {
                self.draw_runs(&line.runs, origin, Paint::Text, surface)
            })
        } else {
            self.draw_runs(&line.runs, origin, Paint::Text, surface)
        }
    }
    /// The outlines of `runs` placed from their line's left edge and baseline `origin`, with
    /// their strokes, filled opaquely as paths: the shape they cast as a text shadow.
    pub(crate) fn draw_silhouette(
        &mut self,
        runs: &[GlyphRun],
        origin: [f32; 2],
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        self.draw_runs(runs, origin, Paint::Silhouette, surface)
    }
    fn font(&mut self, run: &GlyphRun) -> Result<Font, PdfError> {
        let palette = run
            .palette
            .as_deref()
            .map(|colors| colors.iter().copied().map(palette_color).collect())
            .transpose()?;
        let key = (
            run.font_id,
            run.variations
                .iter()
                .map(|variation| (variation.tag, variation.value.to_bits()))
                .collect(),
            palette,
        );
        if let Some(font) = self.embedded.get(&key) {
            return Ok(font.clone());
        }
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
        let font = Font::new_variable_with_palette(
            data.as_ref().unwrap().clone(),
            0,
            &coordinates,
            key.2.clone(),
        )
        .ok_or(PdfError::InvalidFont)?;
        self.embedded.insert(key, font.clone());
        Ok(font)
    }
    fn draw_runs(
        &mut self,
        runs: &[GlyphRun],
        origin: [f32; 2],
        paint: Paint,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let mut glyphs = Vec::new();
        for run in runs {
            if run.font_size == 0.0 {
                continue;
            }
            let font = self.font(run)?;
            glyphs.clear();
            for glyph in run.glyphs() {
                glyphs.push(PdfGlyph {
                    glyph,
                    advance: glyph
                        .normalized_advance(run.font_size)
                        .ok_or(PdfError::InvalidGeometry)?,
                });
            }
            let shear = run.shear(origin[1] + run.baseline_shift);
            if let Some([a, b, c, d, e, f]) = shear {
                surface.push_transform(&Transform::from_row(a, b, c, d, e, f));
            }
            for [x, y] in run.copies(origin) {
                let pen = Point::from_xy(x, y);
                paint_glyphs(surface, run, pen, &glyphs, &font, paint)?;
            }
            if shear.is_some() {
                surface.pop();
            }
            if !run.marks.is_empty() {
                let marks = if paint == Paint::Silhouette {
                    Paint::Silhouette
                } else {
                    Paint::Annotation
                };
                self.draw_runs(&run.marks, origin, marks, surface)?;
            }
            for mark in &run.mark_shapes {
                let color = if paint == Paint::Silhouette {
                    crate::graphics::Color::BLACK
                } else {
                    mark.color
                };
                surface.push_transform(&Transform::from_translate(origin[0], origin[1]));
                surface.set_fill(Some(krilla::paint::Fill {
                    rule: krilla::paint::FillRule::EvenOdd,
                    ..crate::writer::painting::fill(color)?
                }));
                surface.draw_path(&crate::writer::painting::contour_path(&mark.contours)?);
                surface.pop();
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Paint {
    Text,
    /// Glyphs that are not text content, such as emphasis marks, as outline paths.
    Annotation,
    Silhouette,
}

/// Fills a run's glyphs and strokes them with `-webkit-text-stroke` as one text object, in
/// stroke-only or fill-then-stroke render mode. A PDF text object strokes and fills at one
/// opacity, so a fill and stroke of different opacities stroke the outlines separately.
/// Annotations are outline paths and silhouettes opaque ones.
fn paint_glyphs(
    surface: &mut Surface<'_>,
    run: &GlyphRun,
    origin: Point,
    glyphs: &[PdfGlyph<'_>],
    font: &Font,
    paint: Paint,
) -> Result<(), PdfError> {
    let silhouette = paint == Paint::Silhouette;
    let paint_color = |color| {
        if silhouette {
            crate::graphics::Color::BLACK
        } else {
            color
        }
    };
    let fill = crate::writer::painting::fill(paint_color(run.color))?;
    let stroke = run
        .stroke
        .map(|stroke| crate::writer::painting::stroke(paint_color(stroke.color), stroke.width))
        .transpose()?;
    let draw = |surface: &mut Surface<'_>, outlined: bool| {
        surface.draw_glyphs(
            origin,
            glyphs,
            font.clone(),
            &run.text,
            run.font_size,
            outlined,
        );
    };
    match stroke {
        None => {
            surface.set_fill(Some(fill));
            draw(surface, paint != Paint::Text);
        }
        Some(stroke) if silhouette => {
            surface.set_fill(Some(fill));
            surface.set_stroke(Some(stroke));
            draw(surface, true);
        }
        Some(stroke) if run.color.alpha == 0.0 || stroke.opacity == fill.opacity => {
            surface.set_fill((run.color.alpha > 0.0).then_some(fill));
            surface.set_stroke(Some(stroke));
            draw(surface, false);
        }
        Some(stroke) => {
            surface.set_fill(Some(fill));
            draw(surface, false);
            surface.set_fill(None);
            surface.set_stroke(Some(stroke));
            draw(surface, true);
        }
    }
    surface.set_stroke(None);
    Ok(())
}
