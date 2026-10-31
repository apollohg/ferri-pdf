use crate::fonts::{FontBook, FontId};
use crate::graphics::Projection;
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
    orientation: crate::text::GlyphOrientation,
    scale: f32,
}
impl Glyph for PdfGlyph<'_> {
    fn glyph_id(&self) -> GlyphId {
        GlyphId::new(self.glyph.id)
    }
    fn text_range(&self) -> Range<usize> {
        self.glyph.text_range.clone()
    }
    fn x_advance(&self, size: f32) -> f32 {
        self.orientation
            .font_vector([self.advance, self.glyph.y_advance])[0]
            * size
            / self.scale
    }
    fn x_offset(&self, size: f32) -> f32 {
        self.orientation
            .font_vector([self.glyph.x_offset, self.glyph.y_offset])[0]
            * size
            / self.scale
    }
    // Krilla's vertical offsets and advances point up, as OpenType's do; positioned glyphs'
    // point down.
    fn y_offset(&self, size: f32) -> f32 {
        -self
            .orientation
            .font_vector([self.glyph.x_offset, self.glyph.y_offset])[1]
            * size
    }
    fn y_advance(&self, size: f32) -> f32 {
        -self
            .orientation
            .font_vector([self.advance, self.glyph.y_advance])[1]
            * size
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
pub(crate) fn palette_color(color: crate::graphics::Color) -> Result<PaletteColor, PdfError> {
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
    /// A number of a run left out since the last [`Self::take_unwritable`], which PDF cannot
    /// write.
    unwritable: Option<f64>,
}
impl<'a> TextWriter<'a> {
    pub(crate) fn new(fonts: &'a FontBook) -> Self {
        Self {
            fonts,
            data: vec![None; fonts.faces().len()],
            embedded: HashMap::new(),
            unwritable: None,
        }
    }
    /// A number of a run left out since the last call, which PDF cannot write.
    pub(crate) fn take_unwritable(&mut self) -> Option<f64> {
        self.unwritable.take()
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
        self.draw_line(line, Plane::Flat, surface)
    }
    /// Draws `line` under `projection`, each cluster of glyphs through the projection's tangent
    /// at the cluster's ink centre; the line's runs must all have clusters.
    pub(crate) fn draw_projected(
        &mut self,
        line: &TextLine,
        projection: &Projection,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        self.draw_line(line, Plane::Projected(projection), surface)
    }
    fn draw_line(
        &mut self,
        line: &TextLine,
        plane: Plane<'_>,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let origin = [line.x, line.baseline];
        if line.runs.iter().any(|run| run.rtl || run.has_bidi_scopes()) {
            surface.with_actual_text(&line.text, |surface| {
                self.draw_runs(&line.runs, origin, Paint::Text, plane, surface)
            })
        } else {
            self.draw_runs(&line.runs, origin, Paint::Text, plane, surface)
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
        self.draw_runs(runs, origin, Paint::Silhouette, Plane::Flat, surface)
    }
    /// The clusters of `run` drawn from `pen` on a line from `origin`, bounded by their glyphs'
    /// outlines where the face has no colour glyphs.
    pub(crate) fn clusters(
        &self,
        run: &GlyphRun,
        origin: [f32; 2],
        pen: [f32; 2],
    ) -> Result<Option<Vec<crate::text::Cluster>>, PdfError> {
        let face = self
            .fonts
            .face(run.font_id)
            .ok_or(PdfError::InvalidFontId(run.font_id))?;
        let data = face.data().ok_or(PdfError::InvalidFont)?;
        let outlines =
            crate::fonts::Silhouettes::new(data, &run.variations).ok_or(PdfError::InvalidFont)?;
        Ok(run.clusters(origin, pen, |glyph| outlines.outline_box(glyph)))
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
        plane: Plane<'_>,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let mut glyphs = Vec::new();
        for run in runs {
            if run.font_size == 0.0 {
                continue;
            }
            // A run placed past what PDF writes is left out, not the rest of its line.
            let placed = [origin[0] + run.x, origin[1] + run.baseline_shift];
            if let Some(number) = crate::writer::numbers::unwritable_number(&(run, placed)) {
                self.unwritable.get_or_insert(number);
                continue;
            }
            let font = self.font(run)?;
            glyphs.clear();
            for glyph in run.glyphs() {
                glyphs.push(PdfGlyph {
                    glyph,
                    orientation: run.orientation,
                    scale: run.inline_scale,
                    advance: glyph
                        .normalized_advance(run.font_size)
                        .ok_or(PdfError::InvalidGeometry)?,
                });
            }
            for [x, y] in run.copies(origin) {
                let oriented = run.orientation != crate::text::GlyphOrientation::Horizontal
                    || run.inline_scale != 1.0;
                if oriented {
                    let [a, b, c, d] = run.orientation.matrix();
                    let (a, b) = (a * run.inline_scale, b * run.inline_scale);
                    surface.push_transform(&Transform::from_row(
                        a,
                        b,
                        c,
                        d,
                        x - a * x - c * y,
                        y - b * x - d * y,
                    ));
                }
                let shear = run.shear(y);
                let sheared =
                    |surface: &mut Surface<'_>, pen: [f32; 2], glyphs: &[PdfGlyph<'_>]| {
                        if let Some([a, b, c, d, e, f]) = shear {
                            surface.push_transform(&Transform::from_row(a, b, c, d, e, f));
                        }
                        let drawn = paint_glyphs(
                            surface,
                            run,
                            Point::from_xy(pen[0], pen[1]),
                            glyphs,
                            &font,
                            paint,
                        );
                        if shear.is_some() {
                            surface.pop();
                        }
                        drawn
                    };
                match plane {
                    Plane::Flat => sheared(surface, [x, y], &glyphs)?,
                    Plane::Projected(projection) => {
                        for cluster in self
                            .clusters(run, origin, [x, y])?
                            .ok_or(PdfError::InvalidGeometry)?
                        {
                            let ink = cluster.ink;
                            let centre = [ink.x + ink.width / 2.0, ink.y + ink.height / 2.0];
                            let [a, b, c, d, e, f] =
                                crate::paint::projection::tangent(*projection, centre);
                            surface.push_transform(&Transform::from_row(a, b, c, d, e, f));
                            let drawn = sheared(surface, cluster.pen, &glyphs[cluster.glyphs]);
                            surface.pop();
                            drawn?;
                        }
                    }
                }
                if oriented {
                    surface.pop();
                }
            }
            if !run.marks.is_empty() {
                let marks = if paint == Paint::Silhouette {
                    Paint::Silhouette
                } else {
                    Paint::Annotation
                };
                self.draw_runs(&run.marks, origin, marks, plane, surface)?;
            }
            if matches!(plane, Plane::Projected(_)) && !run.mark_shapes.is_empty() {
                return Err(PdfError::InvalidGeometry);
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

/// Where glyphs draw: in the surface's plane, or projected onto it cluster by cluster.
#[derive(Clone, Copy)]
enum Plane<'p> {
    Flat,
    Projected(&'p Projection),
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
