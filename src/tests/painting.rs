use super::common;
use crate::render_html;
use crate::{BoxPaint, DisplayItem, Rect};
use common::{PROFILE, local_options as options};
fn boxes(items: &[DisplayItem]) -> Vec<&BoxPaint> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) => Some(paint),
            _ => None,
        })
        .collect()
}

#[test]
fn solid_borders_participate_in_box_sizing_and_vertical_flow() {
    let html = format!(
        "<style>{PROFILE}p{{margin:0}}</style><div style='width:100pt;padding:8pt;border:6pt solid navy;background:#ffcc00'>A</div><p>B</p>"
    );
    let output = render_html(&html, &options())
        .expect("solid box decoration must render through the public library");
    let trace = output.layout.unwrap();
    let lines = &trace.pages[0];
    const EXPECTED_A_X_PT: f32 = 50.0;
    const EXPECTED_BASELINES_PT: [f32; 2] = [63.1543, 95.1543];
    assert!((lines[0].x - EXPECTED_A_X_PT).abs() < 0.001);
    for (line, expected) in lines.iter().zip(EXPECTED_BASELINES_PT) {
        assert!(
            (line.baseline - expected).abs() < 0.001,
            "{} baseline {}, expected {expected}",
            line.text,
            line.baseline
        );
    }
    let paints = boxes(&trace.display_lists[0]);
    assert_eq!(paints.len(), 1, "only the decorated div needs a box paint");
    assert_eq!(
        paints[0].rect,
        Rect {
            x: 36.0,
            y: 36.0,
            width: 128.0,
            height: 46.0
        },
        "border-box geometry matches the independently measured raster"
    );
    assert_eq!(paints[0].decoration.background.rgb, [255, 204, 0]);
    assert!(
        matches!(trace.display_lists[0][0], DisplayItem::Box(_)),
        "box background and borders must precede the contained text"
    );
}

#[test]
fn sliced_borders_preserve_content_coordinates_across_pages() {
    let body = (0..7)
        .map(|i| format!("A{i}"))
        .collect::<Vec<_>>()
        .join("<br>");
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 180pt;margin:36pt}}p{{margin:0}}</style><div style='border:4pt solid navy;padding:6pt;background:#ffcc00'>{body}</div><p>B</p>"
    );
    let trace = render_html(&html, &options())
        .expect("decorated blocks must fragment")
        .layout
        .unwrap();
    assert_eq!(trace.pages.len(), 2);
    assert_eq!(
        trace.pages[0].len(),
        5,
        "a 108pt fragment fits five 18pt lines after the 10pt start edge"
    );
    assert!((trace.pages[0][0].baseline - 59.1543).abs() < 0.001);
    assert!(
        (trace.pages[1][0].baseline - 49.1543).abs() < 0.001,
        "a sliced continuation has no repeated top border or padding"
    );
    assert!((trace.pages[1].last().unwrap().baseline - 95.1543).abs() < 0.001);
    for (page, height) in [108.0, 46.0].into_iter().enumerate() {
        let paints = boxes(&trace.display_lists[page]);
        assert_eq!(paints.len(), 1);
        assert_eq!(
            paints[0].rect,
            Rect {
                x: 36.0,
                y: 36.0,
                width: 288.0,
                height
            }
        );
        assert_eq!(paints[0].first_fragment, page == 0);
        assert_eq!(paints[0].last_fragment, page == 1);
    }
}

#[test]
fn border_shorthands_share_the_cascade_and_resolve_currentcolor_after_text_color() {
    let html = format!(
        "<style>{PROFILE}</style><div style='border-left-width:0!important;border:4pt SOLID currentColor;color:hsl(120 100% 25%);background:#f008'>A</div>"
    );
    let trace = render_html(&html, &options())
        .expect("case-insensitive keywords and functional colours must share the normal cascade")
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[0][0].x, 36.0,
        "important left width must beat the later shorthand"
    );
    let paint = boxes(&trace.display_lists[0])[0];
    assert_eq!(paint.decoration.borders[0].color.rgb, [0, 128, 0]);
    assert_eq!(paint.decoration.background.rgb, [255, 0, 0]);
    const HEX_ALPHA: f32 = 8.0 / 15.0;
    assert!((paint.decoration.background.alpha - HEX_ALPHA).abs() < 0.000_001);
}

#[test]
fn body_background_is_propagated_once_to_the_page_canvas() {
    let html = format!(
        "<style>{PROFILE}body{{background:rgba(255,0,0,0.5)}}p{{background:navy}}</style><p>A</p>"
    );
    let trace = render_html(&html, &options()).unwrap().layout.unwrap();
    let paints = boxes(&trace.display_lists[0]);
    assert_eq!(
        paints.len(),
        2,
        "propagated body colour must not be painted again on its own box"
    );
    assert_eq!(
        paints[0].rect,
        Rect {
            x: 36.0,
            y: 36.0,
            width: 288.0,
            height: 408.0
        }
    );
    assert_eq!(paints[0].decoration.background.alpha, 0.5);
    assert_eq!(
        paints[1].rect.height, 18.0,
        "the child background covers its line box only"
    );
}

fn fill_colors(paint: &BoxPaint) -> Vec<[u8; 3]> {
    paint
        .border_fills()
        .iter()
        .map(|fill| fill.color.rgb)
        .collect()
}

#[test]
fn three_dimensional_styles_shade_top_left_and_bottom_right() {
    // Navy (0,0,128): dark 0 0 0.251, light 0.5 0.5 0.751.
    const DARK: [u8; 3] = [0, 0, 64];
    const LIGHT: [u8; 3] = [128, 128, 192];
    for (style, expected) in [
        ("groove", vec![DARK, LIGHT, LIGHT, DARK]),
        ("ridge", vec![LIGHT, DARK, DARK, LIGHT]),
        ("inset", vec![DARK, LIGHT]),
        ("outset", vec![LIGHT, DARK]),
    ] {
        let paint = first_paint(
            &format!("div{{width:100pt;height:50pt;border:6pt {style} navy}}"),
            "<div></div>",
        );
        assert_eq!(
            fill_colors(&paint),
            expected,
            "{style}: left+top run bands, then right+bottom run bands"
        );
    }
    // Groove left+top outer band, in y-down page coordinates.
    const EXPECTED_GROOVE_OUTER: [[f32; 2]; 6] = [
        [36.0, 98.0],
        [36.0, 36.0],
        [148.0, 36.0],
        [145.0, 39.0],
        [39.0, 39.0],
        [39.0, 95.0],
    ];
    let paint = first_paint(
        "div{width:100pt;height:50pt;border:6pt groove navy}",
        "<div></div>",
    );
    let crate::Shape::Path { contours, .. } = &paint.border_fills()[0].shape else {
        panic!("groove bands are filled polygons")
    };
    assert_points(
        "groove outer band",
        &contour_points(&contours[0]),
        &EXPECTED_GROOVE_OUTER,
    );
    for (color, dark, light) in [
        ("black", [0; 3], [128; 3]),
        ("white", [128; 3], [255; 3]),
        ("red", [128, 0, 0], [255, 128, 128]),
    ] {
        let paint = first_paint(
            &format!("div{{width:100pt;height:20pt;border:6pt inset {color}}}"),
            "<div></div>",
        );
        assert_eq!(
            fill_colors(&paint),
            vec![dark, light],
            "inset {color}: dark halves each channel, light mixes halfway to white"
        );
    }
}

#[test]
fn double_borders_split_each_side_into_thirds() {
    let paint = first_paint(
        "div{width:100pt;height:50pt;border:solid navy;border-style:double;border-width:4pt 7pt 10pt 2pt}",
        "<div></div>",
    );
    let fills = paint.border_fills();
    assert_eq!(
        fills.len(),
        1,
        "one colour and style: a single even-odd ring"
    );
    let crate::Shape::Path {
        contours, even_odd, ..
    } = &fills[0].shape
    else {
        panic!("double ring is a path")
    };
    assert!(*even_odd);
    let rect = paint.rect;
    let widths = [4.0, 7.0, 10.0, 2.0];
    for (index, contour) in contours.iter().enumerate() {
        let t = index as f32 / 3.0;
        let [top, right, bottom, left] = widths.map(|width: f32| width * t);
        assert_points(
            &format!("double boundary {index}"),
            &contour_points(contour),
            &[
                [rect.x + left, rect.y + top],
                [rect.x + rect.width - right, rect.y + top],
                [rect.x + rect.width - right, rect.y + rect.height - bottom],
                [rect.x + left, rect.y + rect.height - bottom],
            ],
        );
    }
    let mixed = first_paint(
        "div{width:100pt;height:20pt;border:6pt double;border-color:red green blue yellow}",
        "<div></div>",
    );
    let fills = mixed.border_fills();
    assert_eq!(
        fills.iter().map(|fill| fill.color.rgb).collect::<Vec<_>>(),
        vec![
            [255, 255, 0],
            [255, 255, 0],
            [255, 0, 0],
            [255, 0, 0],
            [0, 128, 0],
            [0, 128, 0],
            [0, 0, 255],
            [0, 0, 255]
        ],
        "per side: outer line then inner line, left, top, right, bottom"
    );
    let crate::Shape::Path { contours, .. } = &fills[1].shape else {
        panic!("double lines are polygons")
    };
    let rect = mixed.rect;
    assert_points(
        "left inner line, mitred on the 2/3 and inner rings",
        &contour_points(&contours[0]),
        &[
            [rect.x + 4.0, rect.y + rect.height - 4.0],
            [rect.x + 4.0, rect.y + 4.0],
            [rect.x + 6.0, rect.y + 6.0],
            [rect.x + 6.0, rect.y + rect.height - 6.0],
        ],
    );
}

#[test]
fn rounded_three_dimensional_bands_follow_inset_rings() {
    let paint = first_paint(
        "div{width:100pt;height:20pt;border:6pt groove navy;border-radius:12pt}",
        "<div></div>",
    );
    let fills = paint.border_fills();
    assert_eq!(fills.len(), 8, "two bands for each clipped side");
    let crate::Shape::Path { contours, clip, .. } = &fills[1].shape else {
        panic!("rounded bands are paths")
    };
    assert!(
        clip.is_some(),
        "each side stays inside its corner diagonals"
    );
    let points = contour_points(&contours[0]);
    // The left inner band starts on the half-width ring (radius 9) and ends on the
    // inner ring (radius 6) of the bottom-left corner.
    let rect = paint.rect;
    assert_points(
        "left inner band start",
        &points[..1],
        &[[rect.x + 12.0, rect.y + rect.height - 3.0]],
    );
    assert!(
        points.iter().any(
            |point| (point[0] - (rect.x + 12.0)).abs() < POINT_TOLERANCE_PT
                && (point[1] - (rect.y + rect.height - 6.0)).abs() < POINT_TOLERANCE_PT
        ),
        "the band closes on the inner ring: {points:?}"
    );
}

#[test]
fn collapsed_inset_and_outset_draw_as_ridge_and_groove() {
    const DARK: [u8; 3] = [0, 0, 64];
    const LIGHT: [u8; 3] = [128, 128, 192];
    for (style, interior) in [("inset", [DARK, LIGHT]), ("outset", [LIGHT, DARK])] {
        let trace = render_html(
            &format!(
                "<style>{PROFILE}table{{border-collapse:collapse}}td{{padding:6pt;border:6pt {style} navy}}</style><table><tr><td>A</td><td>B</td></tr></table>"
            ),
            &options(),
        )
        .unwrap()
        .layout
        .unwrap();
        let runs = boxes(&trace.display_lists[0]);
        // The interior grid line is a right side between the two cells.
        let middle = runs
            .iter()
            .find(|run| {
                run.decoration.borders[1].color.alpha > 0.0
                    && run.rect.x > 40.0
                    && run.rect.width <= 6.0 + POINT_TOLERANCE_PT
            })
            .expect("interior vertical grid line");
        assert_eq!(
            fill_colors(middle),
            interior,
            "collapsed {style} paints the interior right side as {} (outer half, inner half)",
            if style == "inset" { "ridge" } else { "groove" }
        );
    }
}

#[test]
fn fixed_backgrounds_scroll_inside_transformed_boxes() {
    let trace = render_html(
        &format!(
            "<style>{PROFILE}div{{height:20pt;background:linear-gradient(red,blue) fixed}}.t{{transform:rotate(3deg)}}.w{{transform:translateX(5pt);background:none}}</style><div class='t'></div><div class='w'><div></div></div><div></div>"
        ),
        &options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let mut depth = 0;
    let mut seen = Vec::new();
    for item in &trace.display_lists[0] {
        match item {
            DisplayItem::PushTransform(_) => depth += 1,
            DisplayItem::Pop => depth -= 1,
            DisplayItem::Box(paint) if paint.decoration.backgrounds.has_images() => seen.push((
                depth,
                format!("{:?}", paint.decoration.backgrounds.attachments),
            )),
            _ => {}
        }
    }
    assert_eq!(
        seen,
        vec![
            (0, "[Fixed]".to_owned()),
            (1, "[Scroll]".to_owned()),
            (1, "[Scroll]".to_owned())
        ],
        "the untransformed box stays fixed; the transformed box and the descendant of another, painted later as stacking contexts, scroll"
    );
}

#[test]
fn html_table_border_attribute_maps_to_outset_table_and_inset_cells() {
    let trace = render_html(
        &format!(
            "<style>{PROFILE}td{{width:40pt;padding:10pt}}</style><table border='3'><tbody><tr><td></td></tr></tbody></table><table border='0'><tr><td></td></tr></table><table border='x'><tr><td><table><tr><td></td></tr></table></td></tr></table>"
        ),
        &options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let painted: Vec<_> = boxes(&trace.display_lists[0])
        .into_iter()
        .map(|paint| {
            (
                paint.decoration.borders[0].style,
                paint.decoration.borders[0].width,
            )
        })
        .collect();
    use crate::BorderStyle::{Inset, Outset};
    assert_eq!(
        painted,
        vec![(Outset, 2.25), (Inset, 0.75), (Outset, 0.75), (Inset, 0.75)],
        "border=3 is 3px outset with 1px inset cells; border=0 paints nothing; an unparseable value is 1px; nested tables without the attribute stay unbordered"
    );
}

#[test]
fn normal_flow_block_backgrounds_are_below_all_inline_text() {
    let html = format!(
        "<style>{PROFILE}</style><div>FIRST</div><div style='margin-top:-18pt;background:red;padding-left:100pt'>SECOND</div>"
    );
    let trace = render_html(&html, &options()).unwrap().layout.unwrap();
    assert_eq!(
        trace.pages[0][0].baseline, trace.pages[0][1].baseline,
        "both text lines intentionally overlap the red background"
    );
    let mut text_seen = false;
    for item in &trace.display_lists[0] {
        match item {
            DisplayItem::Text { .. } => text_seen = true,
            DisplayItem::Frame(_)
            | DisplayItem::Scene(_)
            | DisplayItem::Image(_)
            | DisplayItem::Decoration(_)
            | DisplayItem::TextShadow(_) => {
                panic!("text and box fixture unexpectedly emitted an image or decoration")
            }
            DisplayItem::Box(_) => assert!(
                !text_seen,
                "a later normal-flow background must not cover earlier inline text; FIRST paints above the red box"
            ),
            DisplayItem::PushClip(_)
            | DisplayItem::PushTransform(_)
            | DisplayItem::PushProjection(_)
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PushBlend(_)
            | DisplayItem::PushFilter(_)
            | DisplayItem::PushMask(_)
            | DisplayItem::PushBackdropRoot
            | DisplayItem::Backdrop(_)
            | DisplayItem::Pop => {
                panic!("fixture has no paint effects")
            }
        }
    }
}

#[test]
fn forced_breaks_fill_fragments_and_respect_slice_and_clone_edges() {
    for mode in ["slice", "clone"] {
        let html = format!(
            "<style>{PROFILE}@page{{size:360pt 180pt;margin:36pt}}p{{margin:0}}</style><div style='box-decoration-break:{mode};border:2pt solid navy;padding:6pt;background:#ffcc00'><p>A</p><p class='new-page'>B</p></div><p>C</p>"
        );
        let trace = render_html(&html, &options()).unwrap().layout.unwrap();
        for page in 0..2 {
            let top_edge = if page == 0 || mode == "clone" {
                8.0
            } else {
                0.0
            };
            const TEXT_BASELINE_PT: f32 = 49.1543;
            assert!(
                (trace.pages[page][0].baseline - TEXT_BASELINE_PT - top_edge).abs() < 0.001,
                "{mode} on page {page} must retain only its unsliced top edge: {}",
                trace.pages[page][0].baseline
            );
            let paint = boxes(&trace.display_lists[page])[0];
            let expected_height = if page == 0 {
                108.0
            } else {
                18.0 + top_edge + 8.0
            };
            assert_eq!(
                paint.rect.height, expected_height,
                "{mode} page {page}: broken content fills the remaining fragmentainer"
            );
            assert_eq!(paint.first_fragment, page == 0 || mode == "clone");
            assert_eq!(paint.last_fragment, page == 1 || mode == "clone");
        }
    }
    let body = (0..7)
        .map(|i| format!("A{i}"))
        .collect::<Vec<_>>()
        .join("<br>");
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 180pt;margin:36pt}}</style><div style='box-decoration-break:clone;border:4pt solid navy;padding:6pt;background:#ffcc00'>{body}</div>"
    );
    let trace = render_html(&html, &options()).unwrap().layout.unwrap();
    assert!((trace.pages[1][0].baseline - 59.1543).abs() < 0.001);
    for (page, expected) in [108.0, 74.0].into_iter().enumerate() {
        let paint = boxes(&trace.display_lists[page])[0];
        assert_eq!(
            paint.rect.height, expected,
            "explicit clone repeats edges on automatic breaks too"
        );
    }
}

#[test]
fn patterned_borders_reserve_the_same_space_as_solid_borders() {
    for pattern in ["dotted", "dashed"] {
        let html = format!(
            "<style>{PROFILE}p{{margin:0}}</style><div style='width:120pt;height:40pt;border:4pt {pattern} navy;padding:6pt'>Pattern</div><p>After</p>"
        );
        let output = render_html(&html, &options()).expect("patterned borders must render");
        let trace = output.layout.unwrap();
        let paint = boxes(&trace.display_lists[0])[0];
        assert_eq!(
            paint.rect,
            Rect {
                x: 36.0,
                y: 36.0,
                width: 140.0,
                height: 60.0
            }
        );
        assert!((trace.pages[0][0].x - 46.0).abs() < 0.001);
        assert!(
            (trace.pages[0][1].baseline - 109.1543).abs() < 0.001,
            "following text must include the entire border box for {pattern}"
        );
    }
}

#[test]
fn narrow_page_margin_dots_stay_inside_the_border_box() {
    use crate::graphics::Shape;
    let html = format!(
        "<style>{PROFILE}html{{font-family:Spike}}@page{{margin-top:4pt;@top-center{{content:'X';width:60pt;border:4pt dotted navy}}}}</style><p>Body</p>"
    );
    let trace = render_html(&html, &options()).unwrap().layout.unwrap();
    let paints = boxes(&trace.display_lists[0]);
    let paint = paints
        .iter()
        .find(|paint| paint.rect.height == 4.0)
        .expect("fixture must exercise a margin box shorter than its combined borders");
    let mut dots = 0;
    for fill in paint.border_fills() {
        if let Shape::Pattern {
            contours,
            width,
            dash,
            round,
            clip: None,
        } = fill.shape
        {
            assert!(round);
            for contour in contours {
                let [crate::Segment::Line(end)] = contour.segments[..] else {
                    panic!("square dotted sides are single straight strokes: {contour:?}")
                };
                let points = [contour.start, end];
                let dx = points[1][0] - points[0][0];
                let dy = points[1][1] - points[0][1];
                let length = dx.hypot(dy);
                let radius = width / 2.0;
                for index in 0..=(length / dash[1]).floor() as usize {
                    let distance = index as f32 * dash[1];
                    let ratio = if length == 0.0 {
                        0.0
                    } else {
                        distance / length
                    };
                    let x = points[0][0] + dx * ratio;
                    let y = points[0][1] + dy * ratio;
                    assert!(
                        x - radius >= paint.rect.x
                            && x + radius <= paint.rect.x + paint.rect.width
                            && y - radius >= paint.rect.y
                            && y + radius <= paint.rect.y + paint.rect.height,
                        "dot at ({x}, {y}) with radius {radius} escapes {:?}",
                        paint.rect
                    );
                    dots += 1;
                }
            }
        }
    }
    assert!(dots > 0, "must exercise unclipped circular border marks");
}

#[test]
fn subnormal_border_widths_keep_valid_positive_dash_spacing() {
    use crate::graphics::{Border, BorderStyle, BoxDecoration, Color, Shape};
    const SUBNORMAL_WIDTH: f32 = 1e-40;
    let paint = BoxPaint::whole(
        Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        },
        BoxDecoration {
            background: Color::TRANSPARENT,
            borders: [Border {
                width: SUBNORMAL_WIDTH,
                style: BorderStyle::Dotted,
                color: Color::BLACK,
            }; 4],
            ..BoxDecoration::default()
        },
    );
    for fill in paint.border_fills() {
        let Shape::Pattern { dash, .. } = fill.shape else {
            panic!("expected dotted stroke")
        };
        assert!(
            dash[1].is_finite() && dash[1] > 0.0,
            "finite CSS widths must not become invalid all-zero PDF dash arrays: {dash:?}"
        );
    }
}

const ROUNDED_BOX: &str = "div{width:100pt;height:30pt;padding:4pt;margin-bottom:8pt}";
const POINT_TOLERANCE_PT: f32 = 0.001;

fn first_paint(css: &str, body: &str) -> BoxPaint {
    let html = format!("<style>{PROFILE}{css}</style>{body}");
    let trace = render_html(&html, &options()).unwrap().layout.unwrap();
    boxes(&trace.display_lists[0])[0].clone()
}
fn contour_points(contour: &crate::Contour) -> Vec<[f32; 2]> {
    let mut points = vec![contour.start];
    for segment in &contour.segments {
        match segment {
            crate::Segment::Line(point) => points.push(*point),
            crate::Segment::Cubic(controls) => points.extend(controls),
        }
    }
    points
}
fn assert_points(label: &str, actual: &[[f32; 2]], expected: &[[f32; 2]]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{label}: point count\nactual   {actual:?}\nexpected {expected:?}"
    );
    for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (a[0] - e[0]).abs() < POINT_TOLERANCE_PT && (a[1] - e[1]).abs() < POINT_TOLERANCE_PT,
            "{label}: point {index} is {a:?}, expected {e:?}\nactual   {actual:?}\nexpected {expected:?}"
        );
    }
}
struct StrokeSide {
    points: Vec<[f32; 2]>,
    dash: [f32; 2],
    clip: Option<Vec<[f32; 2]>>,
}
fn patterns(paint: &BoxPaint) -> Vec<StrokeSide> {
    paint
        .border_fills()
        .into_iter()
        .filter_map(|fill| match fill.shape {
            crate::Shape::Pattern {
                contours,
                dash,
                clip,
                ..
            } => Some(StrokeSide {
                points: contour_points(&contours[0]),
                dash,
                clip,
            }),
            _ => None,
        })
        .collect()
}

#[test]
fn elliptical_percentage_radii_build_corner_curves() {
    const EXPECTED_BACKGROUND: [[f32; 2]; 17] = [
        [36.0, 66.0],
        [36.0, 49.44],
        [54.816, 36.0],
        [78.0, 36.0],
        [148.0, 36.0],
        [163.456, 36.0],
        [176.0, 42.72],
        [176.0, 51.0],
        [176.0, 60.0],
        [176.0, 79.872],
        [150.912, 96.0],
        [120.0, 96.0],
        [50.0, 96.0],
        [42.272, 96.0],
        [36.0, 90.624],
        [36.0, 84.0],
        [36.0, 84.0],
    ];
    let paint = first_paint(
        "div{width:120pt;height:40pt;border:4pt solid navy;background:#ffeeaa;border-radius:30% 20% 40% 10% / 50% 25% 60% 20%;padding:6pt}",
        "<div>Ellipse</div>",
    );
    let background = paint
        .background_fill()
        .expect("the box has a background colour");
    let crate::Shape::Path { contours, .. } = &background.shape else {
        panic!(
            "rounded background must be a curved path: {:?}",
            background.shape
        )
    };
    let mut points = contour_points(&contours[0]);
    points.push(*points.last().unwrap());
    assert_points("elliptical background", &points, &EXPECTED_BACKGROUND);
    let borders = paint.border_fills();
    let crate::Shape::Path {
        contours, even_odd, ..
    } = &borders[0].shape
    else {
        panic!(
            "uniform solid border must be one ring: {:?}",
            borders[0].shape
        )
    };
    assert!(
        *even_odd && contours.len() == 2,
        "outer and inner contours fill even-odd"
    );
}

#[test]
fn overlapping_radii_share_one_reduction_factor() {
    const EXPECTED_OUTER: [[f32; 2]; 16] = [
        [36.0, 84.5714],
        [36.0, 57.76],
        [57.76, 36.0],
        [84.5714, 36.0],
        [145.1429, 36.0],
        [166.592, 36.0],
        [184.0, 53.408],
        [184.0, 74.8571],
        [184.0, 74.8571],
        [184.0, 90.944],
        [170.944, 104.0],
        [154.8571, 104.0],
        [55.4286, 104.0],
        [44.704, 104.0],
        [36.0, 95.296],
        [36.0, 84.5714],
    ];
    let paint = first_paint(
        "div{width:120pt;height:40pt;border:8pt solid navy;border-radius:100pt 80pt 60pt 40pt;background:#ffeeaa;padding:6pt}",
        "<div>Overlap</div>",
    );
    let crate::Shape::Path { contours, .. } = &paint
        .background_fill()
        .expect("the box has a background colour")
        .shape
    else {
        panic!("rounded background must be a curved path")
    };
    assert_points(
        "overlap background",
        &contour_points(&contours[0]),
        &EXPECTED_OUTER,
    );
}

#[test]
fn rounded_dots_start_on_corner_diagonals_and_fit_exact_arc_lengths() {
    const EXPECTED_DASHES: [[f32; 2]; 4] =
        [[0.0, 9.2192], [0.0, 8.0167], [0.0, 9.2192], [0.0, 8.0167]];
    const EXPECTED_LEFT_START: [f32; 2] = [42.5409, 75.4591];
    const EXPECTED_LEFT_END: [f32; 2] = [45.1086, 41.344];
    const EXPECTED_LEFT_CLIP: [[f32; 2]; 5] = [
        [36.0, 82.0],
        [36.0, 36.0],
        [40.0, 40.0],
        [59.0, 59.0],
        [40.0, 78.0],
    ];
    let paint = first_paint(
        &format!("{ROUNDED_BOX}div{{border:4pt dotted navy;border-radius:30pt / 12pt}}"),
        "<div>A</div>",
    );
    let sides = patterns(&paint);
    let dashes: Vec<_> = sides.iter().map(|side| side.dash).collect();
    assert_points(
        "left, top, right, bottom dot spacing",
        &dashes,
        &EXPECTED_DASHES,
    );
    let left = &sides[0].points;
    assert_points(
        "left stroke ends",
        &[left[0], *left.last().unwrap()],
        &[EXPECTED_LEFT_START, EXPECTED_LEFT_END],
    );
    assert_points(
        "left clip",
        sides[0].clip.as_ref().unwrap(),
        &EXPECTED_LEFT_CLIP,
    );
}

#[test]
fn tiny_dotted_radii_extrapolate_the_closing_arc() {
    const EXPECTED_LEFT_CLOSING_ARC: [[f32; 2]; 3] =
        [[38.0, 38.0212], [39.4086, 37.3694], [40.3079, 38.9624]];
    const EXPECTED_LEFT_DASH: [f32; 2] = [0.0, 8.3142];
    let paint = first_paint(
        &format!("{ROUNDED_BOX}div{{border:4pt dotted navy;border-radius:3pt}}"),
        "<div>A</div>",
    );
    let left = &patterns(&paint)[0];
    let points = &left.points;
    assert_points(
        "closing arc beyond t=1",
        &points[points.len() - 3..],
        &EXPECTED_LEFT_CLOSING_ARC,
    );
    assert_points("left dot spacing", &[left.dash], &[EXPECTED_LEFT_DASH]);
}

#[test]
fn dashed_sides_fit_gaps_to_straight_runs_plus_half_corners() {
    const EXPECTED_ROUNDED: [[f32; 2]; 4] =
        [[9.0, 17.6381], [6.0, 4.2457], [18.0, 12.0], [12.0, 11.9149]];
    const EXPECTED_SQUARE_GAPS: [f32; 3] = [16.0, 10.0, 13.3333];
    let paint = first_paint(
        &format!(
            "{ROUNDED_BOX}div{{border-style:dashed;border-color:navy;border-width:2pt 6pt 4pt 3pt;border-radius:14pt}}"
        ),
        "<div>A</div>",
    );
    let dashes: Vec<_> = patterns(&paint).iter().map(|side| side.dash).collect();
    assert_points("rounded dashed sides", &dashes, &EXPECTED_ROUNDED);
    let html = format!(
        "<style>{PROFILE}div{{height:30pt;border-style:solid solid dashed solid;border-width:4pt;border-color:navy;margin-bottom:8pt}}</style><div style='width:60pt'>A</div><div style='width:70pt'>B</div><div style='width:80pt'>C</div>"
    );
    let trace = render_html(&html, &options()).unwrap().layout.unwrap();
    for (paint, expected) in boxes(&trace.display_lists[0])
        .into_iter()
        .zip(EXPECTED_SQUARE_GAPS)
    {
        let dash = patterns(paint)[0].dash;
        assert!(
            (dash[1] - expected).abs() < POINT_TOLERANCE_PT,
            "square dashed bottom of {:?}: gap {dash:?}, expected {expected}",
            paint.rect
        );
    }
}

#[test]
fn sliced_fragments_retain_the_unbroken_corners_across_the_break() {
    const RADIUS_PT: f32 = 40.0;
    const FIRST_FRAGMENT_HEIGHT_PT: f32 = 300.0 - 2.0 * 36.0 - 200.0;
    let html = format!(
        "<style>{PROFILE}@page{{size:400pt 300pt;margin:36pt}}.s{{height:200pt}}div.b{{orphans:1;border:4pt solid navy;border-radius:40pt;background:#fea}}</style><div class='s'></div><div class='b'>{}</div>",
        ["A"; 13].join("<br>")
    );
    let trace = render_html(&html, &options()).unwrap().layout.unwrap();
    for page in 0..2 {
        let paint = boxes(&trace.display_lists[page])[0];
        assert_eq!(
            paint.radii(),
            [[RADIUS_PT; 2]; 4],
            "slice preserves the whole box's radius on page {page}"
        );
        let clip = paint.box_area(crate::graphics::BoxArea::BorderBox);
        let whole = clip
            .reference_box
            .expect("the curve continues across the fragment cut");
        assert_eq!(whole, paint.background_box.unwrap());
        assert_eq!(clip.rect, paint.rect);
    }
    assert_eq!(
        boxes(&trace.display_lists[0])[0].rect.height,
        FIRST_FRAGMENT_HEIGHT_PT
    );
    let cloned = render_html(
        &html.replace(
            "border-radius:40pt",
            "box-decoration-break:clone;border-radius:40pt",
        ),
        &options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let first = boxes(&cloned.display_lists[0])[0];
    assert_eq!(
        first.radii(),
        [[FIRST_FRAGMENT_HEIGHT_PT / 2.0; 2]; 4],
        "clone resolves the overlapping corners independently"
    );
}

#[test]
fn border_radius_shorthands_longhands_and_variables_share_the_cascade() {
    use crate::LengthPercentage::{Length, Percentage};
    let paint = first_paint(
        "div{--r:20pt;border-radius:10pt var(--r) / 5pt;border-top-right-radius:30% 1em;background:navy;height:20pt}",
        "<div>A</div>",
    );
    assert_eq!(
        paint.decoration.radii.as_ref(),
        &[
            [Length(10.0), Length(5.0)],
            [Percentage(0.3), Length(12.0)],
            [Length(10.0), Length(5.0)],
            [Length(20.0), Length(5.0)],
        ]
    );
    // CSS Syntax 3 drops invalid declarations, so the earlier radius stays in force.
    for invalid in [
        "border-radius:-1pt",
        "border-radius:1pt / 2pt / 3pt",
        "border-top-left-radius:1pt 2pt 3pt",
        "border-top-position:inherit",
    ] {
        let paint = first_paint(
            &format!("div{{border-radius:7pt;{invalid};background:navy;height:20pt}}"),
            "<div>A</div>",
        );
        assert_eq!(
            paint.decoration.radii.as_ref(),
            &std::array::from_fn::<_, 4, _>(|_| [Length(7.0), Length(7.0)]),
            "{invalid} must be dropped without replacing or misrouting the valid radius"
        );
    }
}

fn sharp_contours(paint: &crate::ShadowPaint) -> &[crate::Contour] {
    match &paint.shape {
        crate::ShadowShape::Sharp(contours) => contours,
        crate::ShadowShape::Blurred(mask) => {
            panic!("unblurred shadow became a mask over {:?}", mask.rect)
        }
    }
}

#[test]
fn shadow_longhands_coordinate_lists_against_offsets_including_none_entries() {
    let paint = first_paint(
        "div{background:white;height:30pt;box-shadow-offset:2pt 3pt,none,7pt,11pt 13pt;box-shadow-color:red,blue;box-shadow-blur:0,4pt;box-shadow-spread:1pt;box-shadow-position:inset,outset}",
        "<div></div>",
    );
    let shadows = paint.decoration.shadows.as_ref();
    assert_eq!(
        shadows.len(),
        3,
        "a none offset suppresses just its own layer"
    );
    for (index, (actual, (offset, colour, blur, inset))) in shadows
        .iter()
        .zip([
            ([2.0, 3.0], [255, 0, 0], 0.0, true),
            ([7.0, 7.0], [255, 0, 0], 0.0, true),
            ([11.0, 13.0], [0, 0, 255], 4.0, false),
        ])
        .enumerate()
    {
        assert_eq!(actual.offset, offset, "shadow {index}: offsets");
        assert_eq!(actual.color.rgb, colour, "shadow {index}: repeated colour");
        assert_eq!(actual.blur, blur, "shadow {index}: repeated blur");
        assert_eq!(actual.spread, 1.0, "shadow {index}: repeated spread");
        assert_eq!(actual.inset, inset, "shadow {index}: repeated position");
    }
}

#[test]
fn shadow_shorthand_resets_and_longhands_override_each_component() {
    let overridden = first_paint(
        "div{background:white;height:30pt;box-shadow:3pt 4pt 5pt 6pt red inset,7pt 8pt navy;box-shadow-color:green;box-shadow-offset:9pt;box-shadow-position:outset}",
        "<div></div>",
    );
    let shadows = overridden.decoration.shadows.as_ref();
    assert_eq!(shadows.len(), 1, "offset list determines layer count");
    assert_eq!(shadows[0].offset, [9.0, 9.0]);
    assert_eq!(shadows[0].blur, 5.0);
    assert_eq!(shadows[0].spread, 6.0);
    assert_eq!(shadows[0].color.rgb, [0, 128, 0]);
    assert!(!shadows[0].inset);
    let reset = first_paint(
        "div{background:white;height:30pt;color:navy;box-shadow:3pt 4pt 5pt 6pt red inset;box-shadow:none,2pt 3pt;box-shadow-offset:7pt,9pt}",
        "<div></div>",
    );
    let shadows = reset.decoration.shadows.as_ref();
    assert_eq!(shadows.len(), 2);
    assert_eq!(
        shadows[0].color.alpha, 0.0,
        "none shorthand layer resets its colour to transparent"
    );
    assert_eq!(
        shadows[1].color.rgb,
        [0, 0, 128],
        "omitted colour of an enabled layer is currentcolor"
    );
    for shadow in shadows {
        assert_eq!(shadow.blur, 0.0, "shorthand resets blur");
        assert_eq!(shadow.spread, 0.0, "shorthand resets spread");
        assert!(!shadow.inset, "shorthand resets position");
    }
}

#[test]
fn shadow_longhands_inherit_computed_lists_and_drop_invalid_negative_blur() {
    let html = format!(
        "<style>{PROFILE}section{{color:red;box-shadow-offset:2pt 4pt,6pt;box-shadow-color:currentcolor,blue;box-shadow-blur:3pt;box-shadow-spread:-2pt;box-shadow-position:inset}}div{{background:white;height:30pt;color:green;box-shadow:inherit;box-shadow-blur:-1pt;box-shadow-offset:var(--offset);--offset:5pt,7pt 9pt}}</style><section><div></div></section>"
    );
    let trace = render_html(&html, &options())
        .expect("inherited shadow lists must render")
        .layout
        .unwrap();
    let child = boxes(&trace.display_lists[0])
        .into_iter()
        .find(|paint| paint.decoration.background.rgb == [255; 3])
        .expect("white child decoration");
    let shadows = child.decoration.shadows.as_ref();
    assert_eq!(shadows.len(), 2);
    assert_eq!(shadows[0].offset, [5.0, 5.0]);
    assert_eq!(shadows[1].offset, [7.0, 9.0]);
    assert_eq!(
        shadows[0].color.rgb,
        [0, 128, 0],
        "inherited currentcolor resolves against the child's own color"
    );
    assert_eq!(shadows[1].color.rgb, [0, 0, 255]);
    for shadow in shadows {
        assert_eq!(
            shadow.blur, 3.0,
            "invalid literal blur preserves inherited value"
        );
        assert_eq!(shadow.spread, -2.0);
        assert!(shadow.inset);
    }
}

#[test]
fn shadow_shapes_spread_their_radii_and_clip_around_the_box() {
    let paint = first_paint(
        "div{width:100pt;height:40pt;border:2pt solid navy;border-radius:10pt 2pt 10pt 10pt;box-shadow:6pt 4pt 0 5pt navy,inset 0 0 0 3pt red}",
        "<div></div>",
    );
    let rect = paint.rect;
    let outer = paint.shadows(false);
    assert_eq!(outer.len(), 1, "one outer shadow");
    let shape = contour_points(&sharp_contours(&outer[0])[0]);
    // Grown 5pt each side and moved (6, 4): the 10pt corner gains the spread, the 2pt corner
    // gains 5 × (1 + (0.4 − 1)³) = 3.92.
    let left = rect.x - 5.0 + 6.0;
    let top = rect.y - 5.0 + 4.0;
    assert_points(
        "spread shadow top-left arc and the start of the sharpened top-right arc",
        &[shape[0], shape[3], shape[4]],
        &[
            [left, top + 15.0],
            [left + 15.0, top],
            [left + rect.width + 10.0 - 5.92, top],
        ],
    );
    let clip: Vec<_> = outer[0].clip.iter().map(contour_points).collect();
    assert_points(
        "clip bounds: the union of the box and the shadow",
        &clip[0],
        &[
            [rect.x, top],
            [left + rect.width + 10.0, top],
            [left + rect.width + 10.0, top + rect.height + 10.0],
            [rect.x, top + rect.height + 10.0],
        ],
    );
    assert_eq!(
        clip[1][0],
        [rect.x, rect.y + 10.0],
        "the rounded border box is cut out of the clip"
    );
    let inset = paint.shadows(true);
    let contours = sharp_contours(&inset[0]);
    assert_eq!(
        contours.len(),
        2,
        "padding rectangle minus the hole, even-odd"
    );
    // Padding radius 10 − 2 = 8, shrunk by the 3pt spread to 5.
    assert_points(
        "inset hole start",
        &contour_points(&contours[1])[..1],
        &[[rect.x + 2.0 + 3.0, rect.y + 2.0 + 3.0 + 5.0]],
    );
}

#[test]
fn sliced_shadows_stop_at_the_fragment_edge_and_blur_into_a_mask() {
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 180pt;margin:36pt}}div{{box-shadow:4pt 6pt 8pt navy}}</style><div>{}</div>",
        ["A"; 12].join("<br>")
    );
    let trace = render_html(&html, &options()).unwrap().layout.unwrap();
    let first = boxes(&trace.display_lists[0])[0];
    let shadow = &first.shadows(false)[0];
    let crate::ShadowShape::Blurred(mask) = &shadow.shape else {
        panic!("a blurred shadow is a soft mask")
    };
    let bottom = first.rect.y + first.rect.height;
    assert!(
        (shadow.bounds.y + shadow.bounds.height - bottom).abs() < POINT_TOLERANCE_PT,
        "the sliced bottom edge cuts the shadow at {bottom}: {:?}",
        shadow.bounds
    );
    // 3σ = 12pt beyond the shape, which the offsets move 4pt right and 6pt down.
    assert!(
        (mask.rect.y - (first.rect.y + 6.0 - 12.0)).abs() < POINT_TOLERANCE_PT
            && (mask.rect.x - (first.rect.x + 4.0 - 12.0)).abs() < POINT_TOLERANCE_PT,
        "mask origin {:?} for box {:?}",
        mask.rect,
        first.rect
    );
    assert!(
        mask.rect.y + mask.rect.height > bottom,
        "the common mask retains the unbroken shadow beyond the fragment cut: {:?}",
        mask.rect
    );
}

#[test]
fn border_image_shorthand_sizes_the_area_and_slices_from_the_cascade() {
    let paint = first_paint(
        "div{width:100pt;height:20pt;border:12pt solid navy;border-image:url(../assets/border-image-nine.png) 10 fill / 6pt 2 / 3pt 0.5 round space}",
        "<div></div>",
    );
    let image = paint
        .border_image()
        .unwrap()
        .expect("the border image replaces the border");
    let rect = paint.rect;
    // Outset 3pt top/bottom and 0.5 × 12pt left/right.
    let area = Rect {
        x: rect.x - 6.0,
        y: rect.y - 3.0,
        width: rect.width + 12.0,
        height: rect.height + 6.0,
    };
    assert_eq!(
        image.pieces[0].area,
        Rect {
            x: area.x,
            y: area.y,
            width: 24.0,
            height: 6.0
        },
        "top-left corner: 2 × 12pt left width, 6pt top width"
    );
    assert_eq!(
        image.size,
        [22.5, 22.5],
        "the 30px image is 22.5pt; 10 slices 7.5pt"
    );
    assert_eq!(image.pieces[0].source.width, 7.5);
    assert_eq!(
        image.pieces.len(),
        9,
        "fill paints the middle: {:#?}",
        image.pieces
    );
    let reset = first_paint(
        "div{width:100pt;height:20pt;border-image:url(../assets/border-image-nine.png) 10;border:12pt solid navy}",
        "<div></div>",
    );
    assert!(
        reset.border_image().unwrap().is_none(),
        "the border shorthand resets border-image (Backgrounds 3 §4.4)"
    );
    let unset_style = first_paint(
        "div{width:100pt;height:20pt;border-image:url(../assets/border-image-nine.png) 10 / 1}",
        "<div></div>",
    );
    let pieces = unset_style.border_image().unwrap().unwrap().pieces;
    assert!(
        pieces.is_empty(),
        "numbers multiply the computed border width, which is zero for border-style none: {pieces:#?}"
    );
}
