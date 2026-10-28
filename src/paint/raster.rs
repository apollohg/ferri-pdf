//! Opaque coverage of fills on a pixel grid, for blurring.
use crate::paint::{Contour, Point, Rect, Segment, Shape};
use tiny_skia::{FillRule, Mask, Paint, Pixmap, Stroke, StrokeDash, Transform};

/// A rectangle enclosing what `shape` paints: its contours' points and control points, grown
/// by half a stroke.
pub(crate) fn bounds(shape: &Shape) -> Option<Rect> {
    let points = |contours: &[Contour], grow: f32| {
        let mut points = contours.iter().flat_map(|contour| {
            std::iter::once(contour.start).chain(contour.segments.iter().flat_map(|segment| {
                match *segment {
                    Segment::Line(point) => vec![point],
                    Segment::Cubic(points) => points.to_vec(),
                }
            }))
        });
        let first = points.next()?;
        let [left, top, right, bottom] = points.fold(
            [first[0], first[1], first[0], first[1]],
            |[left, top, right, bottom], [x, y]| {
                [left.min(x), top.min(y), right.max(x), bottom.max(y)]
            },
        );
        Some(Rect {
            x: left - grow,
            y: top - grow,
            width: right - left + 2.0 * grow,
            height: bottom - top + 2.0 * grow,
        })
    };
    match shape {
        Shape::Rect(rect) => Some(*rect),
        Shape::Clipped {
            shape,
            bounds: clip,
        } => bounds(shape).map(|rect| clip.clip_rect(rect)),
        Shape::Path { contours, .. } => points(contours, 0.0),
        Shape::Pattern {
            contours, width, ..
        }
        | Shape::Stroke { contours, width } => points(contours, width / 2.0),
    }
}

fn path(contours: &[Contour]) -> Option<tiny_skia::Path> {
    let mut path = tiny_skia::PathBuilder::new();
    for contour in contours {
        path.move_to(contour.start[0], contour.start[1]);
        for segment in &contour.segments {
            match *segment {
                Segment::Line([x, y]) => path.line_to(x, y),
                Segment::Cubic([a, b, end]) => {
                    path.cubic_to(a[0], a[1], b[0], b[1], end[0], end[1]);
                }
            }
        }
        if contour.closed {
            path.close();
        }
    }
    path.finish()
}

pub(crate) fn opaque() -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(tiny_skia::Color::BLACK);
    paint
}

/// `clip` narrowed to an even-odd polygon, both in page coordinates.
fn polygon_clip(
    clip: Option<&Mask>,
    polygon: Option<&[Point]>,
    transform: Transform,
    target: &Pixmap,
) -> Option<Mask> {
    let polygon = path(&[Contour::polygon(polygon?)])?;
    let mut mask = within(clip, target)?;
    mask.intersect_path(&polygon, FillRule::EvenOdd, true, transform);
    Some(mask)
}

/// A copy of `clip`, or a mask admitting all of `target`, to narrow further.
pub(crate) fn within(clip: Option<&Mask>, target: &Pixmap) -> Option<Mask> {
    if let Some(clip) = clip {
        return Some(clip.clone());
    }
    let mut mask = Mask::new(target.width(), target.height())?;
    mask.invert();
    Some(mask)
}

/// Paints `shape` opaquely onto `target`, whose pixels `transform` maps page coordinates to
/// and which covers `region` of the page, inside `clip` when given.
pub(crate) fn cover(
    shape: &Shape,
    region: Rect,
    transform: Transform,
    target: &mut Pixmap,
    clip: Option<&Mask>,
) {
    match shape {
        Shape::Rect(rect) => {
            if let Some(rect) = tiny_skia::Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
            {
                target.fill_rect(rect, &opaque(), transform, clip);
            }
        }
        Shape::Clipped { shape, bounds } => {
            let bounds = bounds.clip_rect(region);
            let narrowed =
                tiny_skia::Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height)
                    .and_then(|rect| {
                        let mut mask = within(clip, target)?;
                        mask.intersect_path(
                            &tiny_skia::PathBuilder::from_rect(rect),
                            FillRule::Winding,
                            true,
                            transform,
                        );
                        Some(mask)
                    });
            if let Some(mask) = narrowed {
                cover(shape, region, transform, target, Some(&mask));
            }
        }
        Shape::Path {
            contours,
            even_odd,
            clip: polygon,
        } => {
            let narrowed = polygon_clip(clip, polygon.as_deref(), transform, target);
            if let Some(path) = path(contours) {
                let rule = if *even_odd {
                    FillRule::EvenOdd
                } else {
                    FillRule::Winding
                };
                target.fill_path(
                    &path,
                    &opaque(),
                    rule,
                    transform,
                    narrowed.as_ref().or(clip),
                );
            }
        }
        Shape::Pattern {
            contours,
            width,
            dash,
            round,
            clip: polygon,
        } => {
            let narrowed = polygon_clip(clip, polygon.as_deref(), transform, target);
            if let Some(path) = path(contours) {
                let stroke = Stroke {
                    width: *width,
                    line_cap: if *round {
                        tiny_skia::LineCap::Round
                    } else {
                        tiny_skia::LineCap::Butt
                    },
                    dash: StrokeDash::new(dash.to_vec(), 0.0),
                    ..Stroke::default()
                };
                target.stroke_path(
                    &path,
                    &opaque(),
                    &stroke,
                    transform,
                    narrowed.as_ref().or(clip),
                );
            }
        }
        Shape::Stroke { contours, width } => {
            if let Some(path) = path(contours) {
                let stroke = Stroke {
                    width: *width,
                    ..Stroke::default()
                };
                target.stroke_path(&path, &opaque(), &stroke, transform, clip);
            }
        }
    }
}
