use crate::graphics::{BoxPaint, Color, Contour, Point, Segment, Shape};
use crate::writer::PdfError;
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
/// A solid, butt-capped, miter-joined stroke.
pub(super) fn stroke(color: Color, width: f32) -> Result<Stroke, PdfError> {
    let color = fill(color)?;
    Ok(Stroke {
        paint: color.paint,
        opacity: color.opacity,
        width,
        ..Default::default()
    })
}
/// Outer shadows, background colour, image layers bottom-most first, inset shadows, then
/// borders or the border image (Backgrounds 3 §6, §7.1).
pub(super) fn draw(
    paint: &BoxPaint,
    page_area: crate::graphics::Rect,
    viewport: crate::graphics::Rect,
    surface: &mut Surface<'_>,
    images: &mut crate::writer::images::ImageWriter,
) -> Result<(), PdfError> {
    let shifted;
    let paint = match paint.legend.filter(|_| paint.first_fragment) {
        Some(cut) => {
            let mut fieldset = paint.clone();
            use crate::style::logical::{BOTTOM, LEFT, RIGHT, TOP};
            match cut.side {
                TOP => {
                    fieldset.rect.y += cut.inset;
                    fieldset.rect.height -= cut.inset;
                }
                RIGHT => fieldset.rect.width -= cut.inset,
                BOTTOM => fieldset.rect.height -= cut.inset,
                LEFT => {
                    fieldset.rect.x += cut.inset;
                    fieldset.rect.width -= cut.inset;
                }
                _ => return Err(PdfError::InvalidGeometry),
            }
            shifted = fieldset;
            &shifted
        }
        None => paint,
    };
    if paint.rect.width <= 0.0 || paint.rect.height <= 0.0 {
        return Ok(());
    }
    let rounded = paint
        .decoration
        .radii
        .iter()
        .flatten()
        .any(|radius| *radius != crate::graphics::LengthPercentage::ZERO);
    if paint.background_geometry.is_none() && (rounded || !paint.decoration.shadows.is_empty()) {
        let assembled = paint.assembled_box();
        let origin = assembled.unwrap_or(paint.rect);
        let mut local = paint.clone();
        local.rect = crate::graphics::Rect {
            x: 0.0,
            y: 0.0,
            ..origin
        };
        local.background_box = paint
            .background_box
            .filter(|_| assembled.is_none())
            .map(|rect| crate::graphics::Rect {
                x: rect.x - origin.x,
                y: rect.y - origin.y,
                ..rect
            });
        local.first_fragment = true;
        local.last_fragment = true;
        local.inline_edges = crate::graphics::INLINE_EDGES;
        let bounds = local_bounds(viewport, surface)?;
        let local_viewport = crate::graphics::Rect {
            x: bounds.x - origin.x,
            y: bounds.y - origin.y,
            ..bounds
        };
        let mut clips = 0;
        if let Some(fragment) = paint
            .box_area(crate::graphics::BoxArea::BorderBox)
            .fragment_bounds()
        {
            let bounds = fragment.clip_rect(bounds);
            surface.push_clip_path(&rect_path(bounds)?, &FillRule::NonZero);
            clips = 1;
        }
        let page_area = crate::graphics::Rect {
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
    page_area: crate::graphics::Rect,
    viewport: crate::graphics::Rect,
    surface: &mut Surface<'_>,
    images: &mut crate::writer::images::ImageWriter,
) -> Result<(), PdfError> {
    for shadow in paint.shadows(false) {
        crate::writer::shadows::draw(&shadow, surface)?;
    }
    // Blended layers mix only with the box's own background, as an isolated group
    // (Compositing 1 §3.4.3).
    let blended = paint
        .decoration
        .backgrounds
        .blend_modes
        .iter()
        .any(|mode| *mode != crate::graphics::BlendMode::Normal);
    let (background_viewport, groups) = if blended {
        let nested = local_bounds(viewport, surface)?;
        (
            nested,
            push_blend(crate::graphics::BlendMode::Normal, surface)?,
        )
    } else {
        (viewport, 0)
    };
    let background = (|| {
        if let Some(background) = paint.background_fill() {
            draw_fill(background, background_viewport, surface)?;
        }
        crate::writer::backgrounds::draw(paint, page_area, background_viewport, surface, images)
    })();
    pop_clips(groups, surface);
    background?;
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
    let gapped = paint.legend.is_some() && paint.first_fragment;
    if gapped {
        push_legend_gap(paint, viewport, surface)?;
    }
    let borders = paint
        .border_fills()
        .into_iter()
        .try_for_each(|item| draw_fill(item, viewport, surface));
    if gapped {
        surface.pop();
    }
    borders
}

/// Clips out the part of a fieldset's block-start border behind its rendered legend.
fn push_legend_gap(
    paint: &BoxPaint,
    viewport: crate::graphics::Rect,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let cut = paint.legend.expect("a gapped box has a legend");
    let bounds = local_bounds(viewport, surface)?;
    let mut path = PathBuilder::new();
    path.push_rect(
        Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height)
            .ok_or(PdfError::InvalidGeometry)?,
    );
    use crate::style::logical::{BOTTOM, LEFT, RIGHT, TOP};
    let width = paint.decoration.borders[cut.side].used_width();
    let gap = cut.gap[1] - cut.gap[0];
    let hole = match cut.side {
        TOP => Rect::from_xywh(paint.rect.x + cut.gap[0], paint.rect.y, gap, width),
        RIGHT => Rect::from_xywh(
            paint.rect.x + paint.rect.width - width,
            paint.rect.y + cut.gap[0],
            width,
            gap,
        ),
        BOTTOM => Rect::from_xywh(
            paint.rect.x + cut.gap[0],
            paint.rect.y + paint.rect.height - width,
            gap,
            width,
        ),
        LEFT => Rect::from_xywh(paint.rect.x, paint.rect.y + cut.gap[0], width, gap),
        _ => return Err(PdfError::InvalidGeometry),
    }
    .ok_or(PdfError::InvalidGeometry)?;
    path.push_rect(hole);
    surface.push_clip_path(
        &path.finish().ok_or(PdfError::InvalidGeometry)?,
        &FillRule::EvenOdd,
    );
    Ok(())
}

pub(super) fn rect_path(rect: crate::graphics::Rect) -> Result<krilla::geom::Path, PdfError> {
    let mut path = PathBuilder::new();
    path.push_rect(
        Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
            .ok_or(PdfError::InvalidGeometry)?,
    );
    path.finish().ok_or(PdfError::InvalidGeometry)
}

/// The larger factor by which `transform` scales lengths along its axes.
pub(super) fn transform_scale(transform: Transform) -> f32 {
    transform
        .sx()
        .hypot(transform.ky())
        .max(transform.kx().hypot(transform.sy()))
}

/// Bounds of the physical surface in the active drawing coordinates.
pub(super) fn local_bounds(
    viewport: crate::graphics::Rect,
    surface: &Surface<'_>,
) -> Result<crate::graphics::Rect, PdfError> {
    let transform = surface.cur_transform();
    let determinant = f64::from(transform.sx()) * f64::from(transform.sy())
        - f64::from(transform.kx()) * f64::from(transform.ky());
    if determinant == 0.0 {
        return Ok(crate::graphics::Rect {
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
    Ok(crate::graphics::Rect {
        x: rect.left(),
        y: rect.top(),
        width: rect.width(),
        height: rect.height(),
    })
}

/// Keep original curves and intersect only the changed fragment edges.
pub(super) fn push_clip(
    clip: &crate::graphics::Clip,
    viewport: crate::graphics::Rect,
    surface: &mut Surface<'_>,
) -> Result<usize, PdfError> {
    if clip.axes != [true; 2] {
        let bounds = clip.bounds().clip_rect(local_bounds(viewport, surface)?);
        surface.push_clip_path(&rect_path(bounds)?, &FillRule::NonZero);
        return Ok(1);
    }
    let contours = clip.contours();
    let outline = if contours.is_empty() {
        rect_path(crate::graphics::Rect {
            width: 0.0,
            height: 0.0,
            ..clip.rect
        })?
    } else {
        contour_path(&contours)?
    };
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

/// Opens an isolated transparency group composited with `mode`, returning the pushes to pop.
pub(super) fn push_blend(
    mode: crate::graphics::BlendMode,
    surface: &mut Surface<'_>,
) -> Result<usize, PdfError> {
    surface.push_blend_mode(pdf_blend_mode(mode)?);
    surface.push_isolated();
    Ok(2)
}

/// The PDF blend mode of `mode`; the additive operators have none and composite in pixels
/// instead.
pub(super) fn pdf_blend_mode(
    mode: crate::graphics::BlendMode,
) -> Result<krilla::blend::BlendMode, PdfError> {
    use crate::graphics::BlendMode as Css;
    use krilla::blend::BlendMode as Pdf;
    Ok(match mode {
        Css::Normal => Pdf::Normal,
        Css::Multiply => Pdf::Multiply,
        Css::Screen => Pdf::Screen,
        Css::Overlay => Pdf::Overlay,
        Css::Darken => Pdf::Darken,
        Css::Lighten => Pdf::Lighten,
        Css::ColorDodge => Pdf::ColorDodge,
        Css::ColorBurn => Pdf::ColorBurn,
        Css::HardLight => Pdf::HardLight,
        Css::SoftLight => Pdf::SoftLight,
        Css::Difference => Pdf::Difference,
        Css::Exclusion => Pdf::Exclusion,
        Css::Hue => Pdf::Hue,
        Css::Saturation => Pdf::Saturation,
        Css::Color => Pdf::Color,
        Css::Luminosity => Pdf::Luminosity,
        Css::PlusDarker | Css::PlusLighter => return Err(PdfError::VectorAdditive),
    })
}

pub(super) fn pop_clips(count: usize, surface: &mut Surface<'_>) {
    for _ in 0..count {
        surface.pop();
    }
}

pub(super) fn draw_fill(
    item: crate::graphics::Fill,
    viewport: crate::graphics::Rect,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let color = fill(item.color)?;
    match item.shape {
        Shape::ContourClipped {
            shape,
            contours,
            even_odd,
        } => {
            if !contours.is_empty() {
                let path = contour_path(&contours)?;
                surface.push_clip_path(
                    &path,
                    if even_odd {
                        &FillRule::EvenOdd
                    } else {
                        &FillRule::NonZero
                    },
                );
                let result = draw_fill(
                    crate::graphics::Fill {
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
        Shape::Clipped { shape, bounds } => {
            let bounds = bounds.clip_rect(local_bounds(viewport, surface)?);
            if bounds.width > 0.0 && bounds.height > 0.0 {
                surface.push_clip_path(&rect_path(bounds)?, &FillRule::NonZero);
                let result = draw_fill(
                    crate::graphics::Fill {
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
            if contours.is_empty() {
                return Ok(());
            }
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
            surface.set_stroke(Some(stroke(item.color, width)?));
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
