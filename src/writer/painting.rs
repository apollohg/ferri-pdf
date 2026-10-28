use crate::writer::PdfError;
use ferri_pdf_graphics::{BoxPaint, Color, Contour, Point, Segment, Shape};
use krilla::{
    geom::{PathBuilder, Rect, Transform},
    num::NormalizedF32,
    paint::{Fill, FillRule, LineCap, Stroke, StrokeDash},
    surface::Surface,
};

pub(super) fn fill(color: Color) -> Result<Fill, PdfError> {
    Ok(Fill {
        paint: krilla::color::rgb::Color::new(color.rgb[0], color.rgb[1], color.rgb[2]).into(),
        opacity: NormalizedF32::new(color.alpha).ok_or(PdfError::InvalidAlpha(color.alpha))?,
        ..Default::default()
    })
}
/// Outer shadows, background colour, image layers bottom-most first, inset shadows, then
/// borders or the border image (Backgrounds 3 §6, §7.1).
pub(super) fn draw(
    paint: &BoxPaint,
    page_area: ferri_pdf_graphics::Rect,
    viewport: ferri_pdf_graphics::Rect,
    surface: &mut Surface<'_>,
    images: &mut crate::writer::images::ImageWriter,
) -> Result<(), PdfError> {
    if paint.rect.width <= 0.0 || paint.rect.height <= 0.0 {
        return Ok(());
    }
    let rounded = paint
        .decoration
        .radii
        .iter()
        .flatten()
        .any(|radius| *radius != ferri_pdf_graphics::LengthPercentage::ZERO);
    if paint.background_geometry.is_none() && (rounded || !paint.decoration.shadows.is_empty()) {
        let assembled = paint.assembled_box();
        let origin = assembled.unwrap_or(paint.rect);
        let mut local = paint.clone();
        local.rect = ferri_pdf_graphics::Rect {
            x: 0.0,
            y: 0.0,
            ..origin
        };
        local.background_box = paint
            .background_box
            .filter(|_| assembled.is_none())
            .map(|rect| ferri_pdf_graphics::Rect {
                x: rect.x - origin.x,
                y: rect.y - origin.y,
                ..rect
            });
        local.first_fragment = true;
        local.last_fragment = true;
        local.inline_edges = ferri_pdf_graphics::INLINE_EDGES;
        let bounds = local_bounds(viewport, surface)?;
        let local_viewport = ferri_pdf_graphics::Rect {
            x: bounds.x - origin.x,
            y: bounds.y - origin.y,
            ..bounds
        };
        let mut clips = 0;
        if let Some(fragment) = paint
            .box_area(ferri_pdf_graphics::BoxArea::BorderBox)
            .fragment_bounds()
        {
            let bounds = fragment.clip_rect(bounds);
            surface.push_clip_path(&rect_path(bounds)?, &FillRule::NonZero);
            clips = 1;
        }
        let page_area = ferri_pdf_graphics::Rect {
            x: page_area.x - origin.x,
            y: page_area.y - origin.y,
            ..page_area
        };
        surface.push_transform(&Transform::from_translate(origin.x, origin.y));
        surface.push_isolated();
        let result = draw_box(&local, page_area, local_viewport, surface, images);
        surface.pop();
        surface.pop();
        pop_clips(clips, surface);
        return result;
    }
    draw_box(paint, page_area, viewport, surface, images)
}

fn draw_box(
    paint: &BoxPaint,
    page_area: ferri_pdf_graphics::Rect,
    viewport: ferri_pdf_graphics::Rect,
    surface: &mut Surface<'_>,
    images: &mut crate::writer::images::ImageWriter,
) -> Result<(), PdfError> {
    for shadow in paint.shadows(false) {
        crate::writer::shadows::draw(&shadow, surface)?;
    }
    if let Some(background) = paint.background_fill() {
        draw_fill(background, viewport, surface)?;
    }
    crate::writer::backgrounds::draw(paint, page_area, viewport, surface, images)?;
    for shadow in paint.shadows(true) {
        crate::writer::shadows::draw(&shadow, surface)?;
    }
    // A border image replaces the border styles (Backgrounds 3 §6).
    if let Some(border_image) = paint.border_image().map_err(PdfError::UnresolvedImage)? {
        return crate::writer::backgrounds::draw_border_image(
            &border_image,
            viewport,
            surface,
            images,
        );
    }
    for item in paint.border_fills() {
        draw_fill(item, viewport, surface)?;
    }
    Ok(())
}

pub(super) fn rect_path(rect: ferri_pdf_graphics::Rect) -> Result<krilla::geom::Path, PdfError> {
    let mut path = PathBuilder::new();
    path.push_rect(
        Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
            .ok_or(PdfError::InvalidGeometry)?,
    );
    path.finish().ok_or(PdfError::InvalidGeometry)
}

/// Bounds of the physical surface in the active drawing coordinates.
pub(super) fn local_bounds(
    viewport: ferri_pdf_graphics::Rect,
    surface: &Surface<'_>,
) -> Result<ferri_pdf_graphics::Rect, PdfError> {
    let transform = surface.cur_transform();
    let determinant = f64::from(transform.sx()) * f64::from(transform.sy())
        - f64::from(transform.kx()) * f64::from(transform.ky());
    if determinant == 0.0 {
        return Ok(ferri_pdf_graphics::Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
    }
    let inverse = transform.invert().ok_or(PdfError::InvalidGeometry)?;
    let rect = Rect::from_xywh(viewport.x, viewport.y, viewport.width, viewport.height)
        .and_then(|rect| rect.transform(inverse))
        .ok_or(PdfError::InvalidGeometry)?;
    Ok(ferri_pdf_graphics::Rect {
        x: rect.left(),
        y: rect.top(),
        width: rect.width(),
        height: rect.height(),
    })
}

/// Keep original curves and intersect only the changed fragment edges.
pub(super) fn push_clip(
    clip: &ferri_pdf_graphics::Clip,
    viewport: ferri_pdf_graphics::Rect,
    surface: &mut Surface<'_>,
) -> Result<usize, PdfError> {
    if clip.axes != [true; 2] {
        let bounds = clip.bounds().clip_rect(local_bounds(viewport, surface)?);
        surface.push_clip_path(&rect_path(bounds)?, &FillRule::NonZero);
        return Ok(1);
    }
    let outline = contour_path(&[clip.contour()])?;
    let bounds = clip
        .fragment_bounds()
        .map(|bounds| rect_path(bounds.clip_rect(local_bounds(viewport, surface)?)))
        .transpose()?;
    if let Some(bounds) = &bounds {
        surface.push_clip_path(bounds, &FillRule::NonZero);
    }
    surface.push_clip_path(&outline, &FillRule::NonZero);
    Ok(1 + usize::from(bounds.is_some()))
}

pub(super) fn pop_clips(count: usize, surface: &mut Surface<'_>) {
    for _ in 0..count {
        surface.pop();
    }
}

pub(super) fn draw_fill(
    item: ferri_pdf_graphics::Fill,
    viewport: ferri_pdf_graphics::Rect,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let color = fill(item.color)?;
    match item.shape {
        Shape::Clipped { shape, bounds } => {
            let bounds = bounds.clip_rect(local_bounds(viewport, surface)?);
            if bounds.width > 0.0 && bounds.height > 0.0 {
                surface.push_clip_path(&rect_path(bounds)?, &FillRule::NonZero);
                let result = draw_fill(
                    ferri_pdf_graphics::Fill {
                        shape: *shape,
                        color: item.color,
                    },
                    viewport,
                    surface,
                );
                surface.pop();
                result?;
            }
        }
        Shape::Rect(rect) => {
            surface.set_fill(Some(color));
            surface.draw_path(&rect_path(rect)?);
        }
        Shape::Path {
            contours,
            even_odd,
            clip,
        } => {
            let path = contour_path(&contours)?;
            clipped(surface, clip.as_deref(), |surface| {
                surface.set_fill(Some(Fill {
                    rule: if even_odd {
                        FillRule::EvenOdd
                    } else {
                        FillRule::NonZero
                    },
                    ..color
                }));
                surface.draw_path(&path);
            })?;
        }
        Shape::Pattern {
            contours,
            width,
            dash,
            round,
            clip,
        } => {
            let path = contour_path(&contours)?;
            clipped(surface, clip.as_deref(), |surface| {
                surface.set_fill(None);
                surface.set_stroke(Some(Stroke {
                    paint: color.paint,
                    opacity: color.opacity,
                    width,
                    dash: Some(StrokeDash {
                        array: dash.to_vec(),
                        offset: 0.0,
                    }),
                    line_cap: if round { LineCap::Round } else { LineCap::Butt },
                    ..Default::default()
                }));
                surface.draw_path(&path);
                surface.set_stroke(None);
            })?;
        }
        Shape::Stroke { contours, width } => {
            surface.set_fill(None);
            surface.set_stroke(Some(Stroke {
                paint: color.paint,
                opacity: color.opacity,
                width,
                ..Default::default()
            }));
            surface.draw_path(&contour_path(&contours)?);
            surface.set_stroke(None);
        }
    }
    Ok(())
}

fn clipped(
    surface: &mut Surface<'_>,
    clip: Option<&[Point]>,
    paint: impl FnOnce(&mut Surface<'_>),
) -> Result<(), PdfError> {
    let Some(points) = clip else {
        paint(surface);
        return Ok(());
    };
    let clip = contour_path(&[Contour::polygon(points)])?;
    surface.push_clip_path(&clip, &FillRule::EvenOdd);
    paint(surface);
    surface.pop();
    Ok(())
}

pub(super) fn contour_path(contours: &[Contour]) -> Result<krilla::geom::Path, PdfError> {
    let mut path = PathBuilder::new();
    for contour in contours {
        path.move_to(contour.start[0], contour.start[1]);
        for segment in &contour.segments {
            match *segment {
                Segment::Line(point) => path.line_to(point[0], point[1]),
                Segment::Cubic([first, second, end]) => {
                    path.cubic_to(first[0], first[1], second[0], second[1], end[0], end[1]);
                }
            }
        }
        if contour.closed {
            path.close();
        }
    }
    path.finish().ok_or(PdfError::InvalidGeometry)
}
