use crate::graphics::{Clip, Contour, Rect};
use crate::writer::painting;
use std::sync::Arc;

const SURFACE: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 80.0,
    height: 60.0,
};

fn render(clip: &Clip) -> Vec<u8> {
    super::render([80, 60], |surface| {
        let count = painting::push_clip(clip, SURFACE, surface)
            .expect("a valid disconnected or empty outline must serialize");
        surface.draw_path(&painting::rect_path(SURFACE).unwrap());
        painting::pop_clips(count, surface);
    })
}

#[test]
fn pdf_clips_disconnected_islands_and_their_fragment_edges() {
    let clip = Clip {
        rect: Rect {
            y: 10.0,
            height: 40.0,
            ..SURFACE
        },
        radii: Default::default(),
        outline: Some(Arc::new(vec![
            Contour::polygon(&[[40.0, 0.0], [80.0, 0.0], [80.0, 30.0], [40.0, 30.0]]),
            Contour::polygon(&[[0.0, 30.0], [40.0, 30.0], [40.0, 60.0], [0.0, 60.0]]),
        ])),
        reference_box: Some(SURFACE),
        axes: [true; 2],
    };
    let pixels = render(&clip);
    for y in 0..60 {
        for x in 0..80 {
            let expected = if (10..50).contains(&y) && ((x >= 40 && y < 30) || (x < 40 && y >= 30))
            {
                u8::MAX
            } else {
                0
            };
            assert_eq!(
                pixels[(y * 80 + x) * 4 + 3],
                expected,
                "pixel ({x},{y}): union of the two islands intersected with the fragment"
            );
        }
    }
}

#[test]
fn pdf_empty_outline_clips_all_ink_but_an_open_axis_remains_a_strip() {
    let mut clip = Clip {
        rect: Rect {
            y: 10.0,
            height: 40.0,
            ..SURFACE
        },
        radii: Default::default(),
        outline: Some(Arc::new(Vec::new())),
        reference_box: None,
        axes: [true; 2],
    };
    assert!(
        render(&clip)
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 0),
        "an empty inner area must not restore its bounding rectangle"
    );
    clip.axes = [false, true];
    let pixels = render(&clip);
    for y in 0..60 {
        for x in 0..80 {
            let expected = if (10..50).contains(&y) { u8::MAX } else { 0 };
            assert_eq!(
                pixels[(y * 80 + x) * 4 + 3],
                expected,
                "pixel ({x},{y}): a single clipped axis ignores the corner outline"
            );
        }
    }
}

#[test]
fn pdf_clip_preserves_a_hole_when_all_contours_are_reflected() {
    let outline = vec![
        Contour::polygon(&[[0.0, 0.0], [80.0, 0.0], [80.0, 60.0], [0.0, 60.0]]),
        Contour::polygon(&[[10.0, 10.0], [10.0, 40.0], [30.0, 40.0], [30.0, 10.0]]),
    ];
    for reflected in [false, true] {
        let contours = if reflected {
            outline
                .iter()
                .map(|contour| contour.map(|[x, y]| [SURFACE.width - x, y]))
                .collect()
        } else {
            outline.clone()
        };
        let clip = Clip {
            rect: SURFACE,
            radii: Default::default(),
            outline: Some(Arc::new(contours)),
            axes: [true; 2],
            reference_box: None,
        };
        let pixels = render(&clip);
        for y in 0..60 {
            for x in 0..80 {
                let source_x = if reflected { 79 - x } else { x };
                let expected = if (10..30).contains(&source_x) && (10..40).contains(&y) {
                    0
                } else {
                    u8::MAX
                };
                assert_eq!(
                    pixels[(y * 80 + x) * 4 + 3],
                    expected,
                    "pixel ({x},{y}), reflected={reflected}: the inner contour remains a hole"
                );
            }
        }
    }
}

#[test]
fn compound_fill_clips_preserve_holes_empty_regions_and_nested_bounds() {
    use crate::graphics::{ClipBounds, Color, Fill, Shape};
    let outer = Contour::polygon(&[[10.0, 10.0], [70.0, 10.0], [70.0, 50.0], [10.0, 50.0]]);
    let hole = Contour::polygon(&[[30.0, 20.0], [50.0, 20.0], [50.0, 40.0], [30.0, 40.0]]);
    for (empty, even_odd) in [(false, true), (false, false), (true, true), (true, false)] {
        let shape = Shape::Clipped {
            shape: Box::new(Shape::ContourClipped {
                shape: Box::new(Shape::Rect(SURFACE)),
                contours: if empty {
                    Vec::new()
                } else {
                    vec![outer.clone(), hole.clone()]
                },
                even_odd,
            }),
            bounds: ClipBounds::new(
                Rect {
                    x: 20.0,
                    width: 40.0,
                    ..SURFACE
                },
                [true; 2],
            ),
        };
        let pdf = super::render([80, 60], |surface| {
            painting::draw_fill(
                Fill {
                    shape: shape.clone(),
                    color: Color::BLACK,
                },
                SURFACE,
                surface,
            )
            .unwrap();
        });
        let mut raster = tiny_skia::Pixmap::new(80, 60).unwrap();
        crate::paint::cover(
            &shape,
            SURFACE,
            tiny_skia::Transform::identity(),
            &mut raster,
            None,
        );
        for y in 0..60 {
            for x in 0..80 {
                let expected = if !empty
                    && (20..60).contains(&x)
                    && (10..50).contains(&y)
                    && !(even_odd && (30..50).contains(&x) && (20..40).contains(&y))
                {
                    u8::MAX
                } else {
                    0
                };
                let index = y * 80 + x;
                assert_eq!(
                    pdf[index * 4 + 3],
                    expected,
                    "PDF ({x},{y}), empty={empty}, even_odd={even_odd}"
                );
                assert_eq!(
                    raster.pixels()[index].alpha(),
                    expected,
                    "raster ({x},{y}), empty={empty}, even_odd={even_odd}"
                );
            }
        }
        if empty {
            assert!(crate::paint::shape_bounds(&shape).is_none());
        } else {
            assert_eq!(
                crate::paint::shape_bounds(&shape),
                Some(Rect {
                    x: 20.0,
                    y: 10.0,
                    width: 40.0,
                    height: 40.0
                })
            );
        }
    }
}

#[test]
fn geometry_box_clips_keep_concave_corners_and_signed_box_offsets() {
    use crate::paint::{
        BasicShape, Border, BorderStyle, BoxArea, BoxDecoration, BoxPaint, ClipPath, Color,
        CornerShape, Fill, GeometryBox, LengthPercentage, ShapeBox,
    };
    let rect = Rect {
        x: 20.0,
        y: 20.0,
        width: 100.0,
        height: 80.0,
    };
    let mut paint = BoxPaint::<std::convert::Infallible>::whole(
        rect,
        BoxDecoration {
            radii: Arc::new(std::array::from_fn(|_| {
                std::array::from_fn(|_| LengthPercentage::Percentage(0.5))
            })),
            corner_shapes: Arc::new([
                CornerShape::NOTCH,
                CornerShape::SQUARE,
                CornerShape::NOTCH,
                CornerShape::SQUARE,
            ]),
            borders: [4.0, 8.0, 12.0, 16.0].map(|width| Border {
                width,
                style: BorderStyle::Solid,
                color: Color::BLACK,
            }),
            ..Default::default()
        },
    );
    paint.padding = [3.0; 4];
    paint.margins = [6.0; 4];
    for (reference, insets) in [
        (
            GeometryBox::Shape(ShapeBox::Visual(BoxArea::BorderBox)),
            [0.0; 4],
        ),
        (
            GeometryBox::Shape(ShapeBox::Visual(BoxArea::PaddingBox)),
            [4.0, 8.0, 12.0, 16.0],
        ),
        (
            GeometryBox::Shape(ShapeBox::Visual(BoxArea::ContentBox)),
            [7.0, 11.0, 15.0, 19.0],
        ),
        (GeometryBox::Shape(ShapeBox::Margin), [-6.0; 4]),
        (GeometryBox::Shape(ShapeBox::Margin), [30.0; 4]),
        (GeometryBox::Fill, [7.0, 11.0, 15.0, 19.0]),
        (GeometryBox::Stroke, [0.0; 4]),
        (GeometryBox::View, [0.0; 4]),
    ] {
        if reference == GeometryBox::Shape(ShapeBox::Margin) {
            paint.margins = insets.map(|inset| -inset);
        }
        for basic in [false, true] {
            let clip_path = ClipPath {
                reference,
                shape: basic.then_some(BasicShape::Inset {
                    insets: [LengthPercentage::ZERO; 4],
                    radii: std::array::from_fn(|_| [LengthPercentage::ZERO; 2]),
                }),
            };
            let shape = clip_path.outline(&paint);
            let mut fragment = paint.clone();
            fragment.background_box = Some(rect);
            fragment.rect.y += rect.height / 2.0;
            fragment.rect.height /= 2.0;
            fragment.first_fragment = false;
            let crate::paint::Shape::Path {
                contours: whole, ..
            } = &shape
            else {
                panic!("geometry boxes produce filled paths");
            };
            let crate::paint::Shape::Path {
                contours: sliced, ..
            } = clip_path.outline(&fragment)
            else {
                panic!("sliced geometry boxes produce filled paths");
            };
            assert_eq!(
                *whole, sliced,
                "{reference:?}: a fragment retains the assembled path"
            );
            let viewport = Rect {
                x: 0.0,
                y: 0.0,
                width: 140.0,
                height: 120.0,
            };
            let pdf = super::render([140, 120], |surface| {
                painting::draw_fill(
                    Fill {
                        shape: shape.clone(),
                        color: Color::BLACK,
                    },
                    viewport,
                    surface,
                )
                .unwrap();
            });
            let mut raster = tiny_skia::Pixmap::new(140, 120).unwrap();
            crate::paint::cover(
                &shape,
                viewport,
                tiny_skia::Transform::identity(),
                &mut raster,
                None,
            );
            let [top, right, bottom, left] = insets;
            for y in 0..120 {
                for x in 0..140 {
                    let px = x as f32 + 0.5;
                    let py = y as f32 + 0.5;
                    let inside = px >= 20.0 + left
                        && px < 120.0 - right
                        && py >= 20.0 + top
                        && py < 100.0 - bottom;
                    let cut = (px < 70.0 + top && py < 60.0 + left)
                        || (px >= 70.0 - bottom && py >= 60.0 - right);
                    let expected = if inside && (basic || !cut) {
                        u8::MAX
                    } else {
                        0
                    };
                    for (backend, alpha) in [
                        pdf[(y * 140 + x) * 4 + 3],
                        raster.pixels()[y * 140 + x].alpha(),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        assert_eq!(
                            alpha, expected,
                            "reference={reference:?}, basic={basic}, backend={backend}, pixel=({x},{y})"
                        );
                    }
                }
            }
        }
    }
}
