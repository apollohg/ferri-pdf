//! Text shadows (CSS Text Decoration 3 §4). A sharp shadow fills its colour through an alpha
//! soft mask of the glyph and decoration silhouettes; a blurred one rasterizes them, blurs
//! the coverage, and paints it as a blurred box shadow is painted.
use crate::fonts::{FontBook, Silhouettes};
use crate::graphics::{Color, Contour, Fill, Rect, ShadowPaint, ShadowShape, TextShadowPaint};
use crate::text::GlyphRun;
use crate::writer::{PdfError, painting, text::TextWriter};
use krilla::{
    geom::Transform,
    mask::{Mask, MaskType},
    paint::FillRule,
    surface::Surface,
};
use std::sync::Arc;

const ALPHA_MAX: f32 = 255.0;

fn translate(rect: Rect, [dx, dy]: [f32; 2]) -> Rect {
    Rect {
        x: rect.x + dx,
        y: rect.y + dy,
        ..rect
    }
}

/// The decorations' fills, opaque, and the area of them to shadow: the shadowed runs'
/// horizontal extent.
fn decoration_fills(shadow: &TextShadowPaint) -> (Vec<Fill>, Option<Rect>) {
    let fills: Vec<Fill> = shadow
        .decorations
        .iter()
        .flat_map(|decoration| {
            crate::graphics::DecorationPaint {
                color: Color::BLACK,
                ..*decoration
            }
            .fills()
        })
        .collect();
    let [left, right] = shadow.extent;
    let cut = fills
        .iter()
        .filter_map(|fill| crate::paint::shape_bounds(&fill.shape))
        .reduce(Rect::union)
        .map(|bounds| Rect {
            x: left,
            width: right - left,
            ..bounds
        });
    (fills, cut)
}

/// The unshadowed area the runs' glyphs and the decorations can paint.
fn ink(shadow: &TextShadowPaint, cut: Option<Rect>) -> Option<Rect> {
    shadow
        .runs
        .iter()
        .filter_map(|run| run.ink_rect(shadow.origin))
        .chain(cut)
        .filter(|rect| rect.width > 0.0 && rect.height > 0.0)
        .reduce(Rect::union)
}

pub(super) fn draw<'a>(
    shadow: &'a TextShadowPaint,
    text: &mut TextWriter<'a>,
    fonts: &FontBook,
    viewport: Rect,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let (fills, cut) = decoration_fills(shadow);
    let Some(ink) = ink(shadow, cut) else {
        return Ok(());
    };
    let cast = crate::paint::BoxShadow {
        offset: shadow.offset,
        blur: shadow.blur,
        spread: 0.0,
        color: shadow.color,
        inset: false,
    };
    if shadow.blur > 0.0 {
        return draw_blurred(shadow, &cast, (&fills, cut), ink, fonts, surface);
    }
    let [dx, dy] = shadow.offset;
    let mut builder = surface.stream_builder();
    let mut silhouette = builder.surface();
    silhouette.push_transform(&Transform::from_translate(dx, dy));
    let drawn = (|| -> Result<(), PdfError> {
        text.draw_silhouette(&shadow.runs, shadow.origin, &mut silhouette)?;
        if let Some(cut) = cut {
            silhouette.push_clip_path(&painting::rect_path(cut)?, &FillRule::NonZero);
            let filled = fills
                .into_iter()
                .try_for_each(|fill| painting::draw_fill(fill, viewport, &mut silhouette));
            silhouette.pop();
            filled?;
        }
        Ok(())
    })();
    silhouette.pop();
    silhouette.finish();
    let stream = builder.finish();
    drawn?;
    surface.push_mask(Mask::new(stream, MaskType::Alpha));
    surface.set_fill(Some(painting::fill(shadow.color)?));
    surface.draw_path(&painting::rect_path(translate(ink, shadow.offset))?);
    surface.pop();
    Ok(())
}

fn draw_blurred(
    shadow: &TextShadowPaint,
    cast: &crate::paint::BoxShadow,
    (fills, cut): (&[Fill], Option<Rect>),
    ink: Rect,
    fonts: &FontBook,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let spread = cast.extent();
    let region = Rect {
        x: ink.x + shadow.offset[0] - spread,
        y: ink.y + shadow.offset[1] - spread,
        width: ink.width + 2.0 * spread,
        height: ink.height + 2.0 * spread,
    };
    let runs: Vec<_> = shadow.runs.iter().flat_map(GlyphRun::with_marks).collect();
    let mut faces = Vec::with_capacity(runs.len());
    for run in &runs {
        let data = fonts
            .face(run.font_id)
            .ok_or(PdfError::InvalidFontId(run.font_id))?
            .data()
            .ok_or(PdfError::InvalidFont)?;
        faces.push(Silhouettes::new(data, &run.variations).ok_or(PdfError::InvalidFont)?);
    }
    let mask = crate::paint::blurred_shadow(region, cast.sigma(), |size, pitch| {
        let Some(mut pixmap) = tiny_skia::Pixmap::new(size[0] as u32, size[1] as u32) else {
            return vec![0.0; size[0] * size[1]];
        };
        let page = tiny_skia::Transform::from_row(
            1.0 / pitch[0],
            0.0,
            0.0,
            1.0 / pitch[1],
            -region.x / pitch[0],
            -region.y / pitch[1],
        )
        .pre_translate(shadow.offset[0], shadow.offset[1]);
        for (run, face) in runs.iter().zip(&faces) {
            if run.font_size == 0.0 {
                continue;
            }
            let scale = run.font_size / face.units_per_em();
            let shear = run
                .shear(shadow.origin[1] + run.baseline_shift)
                .map_or(tiny_skia::Transform::identity(), |[a, b, c, d, e, f]| {
                    tiny_skia::Transform::from_row(a, b, c, d, e, f)
                });
            let stroke = run.stroke.map(|stroke| stroke.width / scale);
            for pen in run.copies(shadow.origin) {
                for (glyph, [x, y]) in run.glyph_origins(pen) {
                    let transform = page
                        .pre_concat(shear)
                        .pre_translate(x, y)
                        .pre_scale(scale, -scale);
                    face.paint(glyph.id, transform, stroke, &mut pixmap);
                }
            }
        }
        let local = translate(region, [-shadow.offset[0], -shadow.offset[1]]);
        let [x, y] = shadow.origin;
        for mark in shadow.runs.iter().flat_map(|run| &run.mark_shapes) {
            crate::paint::cover(
                &mark.shape(),
                translate(local, [-x, -y]),
                page.pre_translate(x, y),
                &mut pixmap,
                None,
            );
        }
        if let Some(cut) = cut.and_then(|cut| {
            let rect = tiny_skia::Rect::from_xywh(cut.x, cut.y, cut.width, cut.height)?;
            let mut mask = tiny_skia::Mask::new(pixmap.width(), pixmap.height())?;
            mask.fill_path(
                &tiny_skia::PathBuilder::from_rect(rect),
                tiny_skia::FillRule::Winding,
                true,
                page,
            );
            Some(mask)
        }) {
            for fill in fills {
                crate::paint::cover(&fill.shape, local, page, &mut pixmap, Some(&cut));
            }
        }
        pixmap
            .pixels()
            .iter()
            .map(|pixel| f32::from(pixel.alpha()) / ALPHA_MAX)
            .collect()
    });
    let corners = [
        [region.x, region.y],
        [region.x + region.width, region.y],
        [region.x + region.width, region.y + region.height],
        [region.x, region.y + region.height],
    ];
    crate::writer::shadows::draw(
        &ShadowPaint {
            bounds: region,
            clip: vec![Contour::polygon(&corners)],
            shape: ShadowShape::Blurred(Arc::new(mask)),
            color: shadow.color,
        },
        surface,
    )
}
