use crate::paint::{BorderStyle, BoxDecoration, CornerShape, Rect};

#[test]
fn concave_border_styles_do_not_fill_cutouts_or_disconnected_padding_islands() {
    use crate::paint::{Border, BoxPaint, Color, LengthPercentage};
    use std::sync::Arc;
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 80.0,
    };
    for widths in [[4.0; 4], [4.0, 8.0, 12.0, 16.0]] {
        for style in [
            BorderStyle::Solid,
            BorderStyle::Double,
            BorderStyle::Groove,
            BorderStyle::Ridge,
            BorderStyle::Dotted,
            BorderStyle::Dashed,
        ] {
            let paint: BoxPaint = BoxPaint::whole(
                rect,
                BoxDecoration {
                    radii: Arc::new(std::array::from_fn(|corner| {
                        std::array::from_fn(|_| {
                            if corner % 2 == 0 {
                                LengthPercentage::Percentage(0.5)
                            } else {
                                LengthPercentage::ZERO
                            }
                        })
                    })),
                    corner_shapes: Arc::new([
                        CornerShape::NOTCH,
                        CornerShape::ROUND,
                        CornerShape::NOTCH,
                        CornerShape::ROUND,
                    ]),
                    borders: widths.map(|width| Border {
                        width,
                        style,
                        color: Color::BLACK,
                    }),
                    ..Default::default()
                },
            );
            let fills = paint.border_fills();
            let pdf = super::render([100, 80], |surface| {
                for fill in &fills {
                    crate::writer::painting::draw_fill(fill.clone(), rect, surface).unwrap();
                }
            });
            let mut pixels = tiny_skia::Pixmap::new(100, 80).unwrap();
            for fill in &fills {
                crate::paint::cover(
                    &fill.shape,
                    rect,
                    tiny_skia::Transform::identity(),
                    &mut pixels,
                    None,
                );
            }
            let [top, right, bottom, left] = widths;
            let mut ink = [0; 2];
            for y in 0..80 {
                for x in 0..100 {
                    let [px, py] = [x as f32 + 0.5, y as f32 + 0.5];
                    let outer = (px >= 50.0 && py < 40.0) || (px < 50.0 && py >= 40.0);
                    let inner =
                        (px >= 50.0 + top && px < 100.0 - right && py >= top && py < 40.0 - right)
                            || (px >= left
                                && px < 50.0 - bottom
                                && py >= 40.0 + left
                                && py < 80.0 - bottom);
                    for (backend, alpha) in [
                        pixels.pixels()[y * 100 + x].alpha(),
                        pdf[(y * 100 + x) * 4 + 3],
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        assert!(
                            alpha == 0 || (outer && !inner),
                            "{style:?}, backend={backend}, widths={widths:?}: pixel({x},{y}) alpha={alpha} escaped the border region"
                        );
                        if style == BorderStyle::Solid && outer && !inner {
                            assert!(
                                alpha > 0,
                                "widths={widths:?}: solid border lost pixel({x},{y})"
                            );
                        }
                        ink[backend] += usize::from(alpha > 0);
                    }
                }
            }
            assert!(
                ink.iter().all(|&count| count > 50),
                "{style:?}, widths={widths:?}: test must paint a visible border, got {ink:?} pixels"
            );
        }
    }
}

#[test]
fn completely_consumed_corner_regions_emit_no_border_ink() {
    use crate::paint::{Border, BoxPaint, Color, LengthPercentage};
    use std::sync::Arc;
    let paint: BoxPaint = BoxPaint::whole(
        Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 80.0,
        },
        BoxDecoration {
            radii: Arc::new(std::array::from_fn(|_| {
                std::array::from_fn(|_| LengthPercentage::Percentage(0.5))
            })),
            corner_shapes: Arc::new([CornerShape::NOTCH; 4]),
            borders: [Border {
                width: 4.0,
                style: BorderStyle::Solid,
                color: Color::BLACK,
            }; 4],
            ..Default::default()
        },
    );
    let pixels = super::render([100, 80], |surface| {
        for fill in paint.border_fills() {
            crate::writer::painting::draw_fill(fill, paint.rect, surface)
                .expect("an empty border region is valid geometry");
        }
    });
    assert!(
        pixels.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0),
        "four half-box notches leave no border region"
    );
}

#[test]
fn shaped_outlines_offset_the_original_border_contour() {
    use crate::paint::{BoxPaint, Color, LengthPercentage, Outline, OutlineStyle};
    use std::sync::Arc;
    let rect = Rect {
        x: 20.0,
        y: 20.0,
        width: 100.0,
        height: 80.0,
    };
    for style in [
        BorderStyle::Solid,
        BorderStyle::Double,
        BorderStyle::Groove,
        BorderStyle::Ridge,
        BorderStyle::Dotted,
        BorderStyle::Dashed,
    ] {
        for offset in [6.0, -6.0, -100.0] {
            let paint = BoxPaint::<std::convert::Infallible>::whole(
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
                    outline: Outline {
                        width: 4.0,
                        offset,
                        style: OutlineStyle::Line(style),
                        color: Color::BLACK,
                    },
                    ..Default::default()
                },
            );
            let outline = paint.outline().unwrap();
            let pixels = super::render([140, 120], |surface| {
                for fill in outline.border_fills() {
                    crate::writer::painting::draw_fill(
                        fill,
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 140.0,
                            height: 120.0,
                        },
                        surface,
                    )
                    .unwrap();
                }
            });
            let horizontal = (-offset - 4.0_f32).min(46.0);
            let vertical = (-offset - 4.0_f32).min(36.0);
            let contains = |x: f32, y: f32, depth: f32| {
                let h = horizontal + depth;
                let v = vertical + depth;
                x >= 20.0 + h
                    && x < 120.0 - h
                    && y >= 20.0 + v
                    && y < 100.0 - v
                    && !(x < 70.0 + v && y < 60.0 + h)
                    && !(x >= 70.0 - v && y >= 60.0 - h)
            };
            let mut ink = 0;
            for y in 0..120 {
                for x in 0..140 {
                    let px = x as f32 + 0.5;
                    let py = y as f32 + 0.5;
                    let expected = if contains(px, py, 0.0) && !contains(px, py, 4.0) {
                        u8::MAX
                    } else {
                        0
                    };
                    let alpha = pixels[(y * 140 + x) * 4 + 3];
                    assert!(
                        alpha == 0 || expected > 0,
                        "style={style:?},offset={offset},pixel=({x},{y}) escaped the outline region"
                    );
                    if style == BorderStyle::Solid {
                        assert_eq!(alpha, expected, "offset={offset},pixel=({x},{y})");
                    }
                    ink += usize::from(alpha > 0);
                }
            }
            assert!(
                offset == -100.0 || ink > 0,
                "style={style:?},offset={offset}: outline vanished"
            );
        }
    }
}
