//! CSS Shapes 1 float areas: each line box beside a float is shortened by the extent of the
//! float's `shape-outside`, grown by `shape-margin` and clipped to its margin box, over the
//! line box's whole block range (§2, §3, §4, §5, §6).
mod common;
use base64::Engine;
use common::{PROFILE, render_trace};
use ferri_pdf::{LayoutTrace, TextLine};

const PAGE_LEFT: f32 = 36.0;
const PAGE_RIGHT: f32 = 324.0;
const PAGE_TOP: f32 = 36.0;
const PAGE_CONTENT_HEIGHT: f32 = 408.0;
const LINE_HEIGHT: f32 = 18.0;
/// Curves are flattened to within 0.05pt and images sampled at four pixels per point.
const CURVE_TOLERANCE: f32 = 0.1;
const SAMPLE_TOLERANCE: f32 = 0.3;
const WORDS: &str = "aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt uu vv ww xx yy \
     zz aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt uu vv ww xx yy zz aa bb cc \
     dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt uu vv ww xx yy zz aa bb cc dd ee ff gg";

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_trace(&format!(
        "<style>{PROFILE}p{{margin:0}}.f{{float:left}}{css}</style>{body}"
    ))
}

/// The top of a line box: lines are `LINE_HEIGHT` tall and their baselines sit at the same
/// depth below their tops.
fn line_top(line: &TextLine, first: &TextLine) -> f32 {
    PAGE_TOP + line.baseline - first.baseline
}

/// Asserts that each line beside the float starts at `expected(top)`, in float coordinates
/// whose origin is the float's margin box corner at the page area's top left.
fn assert_left_edges(
    label: &str,
    lines: &[TextLine],
    rows: std::ops::Range<usize>,
    tolerance: f32,
    expected: impl Fn(f32) -> f32,
) {
    assert!(
        lines.len() >= rows.end,
        "{label}: only {} lines",
        lines.len()
    );
    for (index, line) in lines[rows].iter().enumerate() {
        let top = line_top(line, &lines[0]) - PAGE_TOP;
        let edge = PAGE_LEFT + expected(top);
        assert!(
            (line.x - edge).abs() <= tolerance,
            "{label}: line {index} ({:?}) at block offset {top}pt starts at {}pt, expected the \
             float area's edge {edge}pt (±{tolerance}pt)",
            line.text,
            line.x
        );
    }
}

/// The right edge of a circle of `radius` centred `center` down the float, over the line box
/// from `top` to `top + LINE_HEIGHT`.
fn circle_edge(center: [f32; 2], radius: f32, top: f32) -> f32 {
    let bottom = top + LINE_HEIGHT;
    let dy = if (top..=bottom).contains(&center[1]) {
        0.0
    } else {
        (top - center[1]).abs().min((bottom - center[1]).abs())
    };
    center[0] + (radius * radius - dy * dy).max(0.0).sqrt()
}

#[test]
fn circle_shortens_each_line_box_by_its_extent_over_the_whole_line() {
    let trace = trace(
        ".f{width:108pt;height:108pt;shape-outside:circle(50%)}",
        &format!("<div class=f></div><p>{WORDS}</p>"),
    );
    let lines = &trace.pages[0];
    assert_left_edges("circle(50%)", lines, 0..6, CURVE_TOLERANCE, |top| {
        circle_edge([54.0, 54.0], 54.0, top)
    });
    assert_left_edges("below the float", lines, 6..8, 0.0, |_| 0.0);
}

#[test]
fn right_floats_wrap_lines_on_their_left_and_rtl_inline_start_floats_are_right_floats() {
    for (label, css) in [
        ("float:right", ".f{float:right}"),
        (
            "rtl inline-start",
            "body{direction:rtl}.f{float:inline-start}",
        ),
    ] {
        let trace = trace(
            &format!(
                "{css}.f{{width:108pt;height:108pt;shape-outside:ellipse(farthest-side closest-side at 100% 50%)}}p{{text-align:right}}"
            ),
            &format!("<div class=f></div><p>{WORDS}</p>"),
        );
        let lines = &trace.pages[0];
        // An ellipse centred on the float's right edge, 108pt wide and 54pt tall.
        for line in &lines[..6] {
            let top = line_top(line, &lines[0]) - PAGE_TOP;
            let bottom = top + LINE_HEIGHT;
            let dy = if (top..=bottom).contains(&54.0) {
                0.0
            } else {
                (top - 54.0).abs().min((bottom - 54.0).abs())
            };
            let extent = 108.0 * (1.0 - (dy / 54.0).powi(2)).max(0.0).sqrt();
            let edge = PAGE_RIGHT - extent;
            assert!(
                (line.x + line.width - edge).abs() <= CURVE_TOLERANCE,
                "{label}: line {:?} at {top}pt ends at {}pt, expected {edge}pt",
                line.text,
                line.x + line.width
            );
        }
    }
}

#[test]
fn polygon_lines_and_lines_moved_down_to_the_first_position_that_fits() {
    let trace = trace(
        ".f{width:100pt;height:100pt;shape-outside:polygon(0 0,100% 0,0 100%)}\
         .w{width:150pt}.box{display:inline-block;width:80pt;height:18pt;vertical-align:top}",
        "<div class=w><div class=f></div><p>aa <span class=box></span> dd</p></div>",
    );
    let lines = &trace.pages[0];
    assert_left_edges("triangle", &lines[..1], 0..1, CURVE_TOLERANCE, |top| {
        100.0 - top
    });
    // The 80pt box fits once 50pt + y of the 150pt line is left beside the triangle, whose
    // edge over the line box is at its top: 100pt - y.
    let boxed = &lines[1];
    let top = line_top(boxed, &lines[0]) - PAGE_TOP;
    assert!(
        (top - 30.0).abs() <= 0.02,
        "the box's line moves down exactly until it fits at 30pt, not to a vertex or the float's \
         bottom: it is at {top}pt ({:?})",
        boxed.text
    );
    assert!(
        (boxed.x - (PAGE_LEFT + 70.0)).abs() <= 0.02,
        "it starts at the triangle's edge there: {}",
        boxed.x
    );
}

#[test]
fn lines_move_down_into_a_notch_narrowest_between_two_vertices() {
    // The float's right side runs in from (100, 0) to (20, 50) and out to (100, 100). A
    // 20pt line box straddling the inner vertex reaches furthest at its ends, least at
    // y = 40pt, so the 110pt box fits only from 37.5 to 42.5pt: strictly between the
    // positions 30 and 50pt where the vertex enters and leaves the line box.
    let trace = trace(
        ".f{width:100pt;height:100pt;shape-outside:polygon(0 0,100% 0,20% 50%,100% 100%,0 100%)}\
         .w{width:150pt}p{line-height:20pt}\
         .box{display:inline-block;width:110pt;height:20pt;vertical-align:top}",
        "<div class=w><div class=f></div><p>aa <span class=box></span></p></div>",
    );
    let lines = &trace.pages[0];
    assert_eq!(lines.len(), 2, "the word and the box take a line each");
    let top = lines[1].baseline - lines[0].baseline;
    assert!(
        (top - 37.5).abs() <= 0.02,
        "the box's line is at the notch's first fit, 37.5pt, not {top}pt"
    );
    assert!(
        (lines[1].x - (PAGE_LEFT + 40.0)).abs() <= 0.03,
        "it starts at the upper edge there, 40pt: {}",
        lines[1].x - PAGE_LEFT
    );
}

#[test]
fn fill_rule_decides_the_area_left_inside_the_margin_box() {
    // An outer rectangle twice the float's width around an inner one from 50% to 150%,
    // traced in the same direction: even-odd leaves a hole that the margin box clips into.
    let points = "0 0,200% 0,200% 100%,0 100%,0 0,50% 0,150% 0,150% 100%,50% 100%,50% 0";
    for (rule, edge) in [("nonzero", 100.0), ("evenodd", 50.0)] {
        let trace = trace(
            &format!(".f{{width:100pt;height:54pt;shape-outside:polygon({rule},{points})}}"),
            &format!("<div class=f></div><p>{WORDS}</p>"),
        );
        assert_left_edges(rule, &trace.pages[0], 0..3, CURVE_TOLERANCE, |_| edge);
    }
}

#[test]
fn degenerate_insets_keep_vertical_edges_and_horizontal_lines_are_empty() {
    for (label, shape, edge) in [
        // §6.1 examples: a vertical line at the reference box's midpoint.
        ("vertical line", "inset(0% 50% 0% 50%)", 50.0),
        // Left and right insets summing to 125% are reduced to 40% and 60%.
        ("overlapping insets", "inset(0 75% 0 50%)", 40.0),
        ("rect() edges floored", "rect(0 10pt 100% 30pt)", 30.0),
        ("xywh()", "xywh(20pt 0 15pt 100%)", 35.0),
        // A line along the block axis encloses nothing and leaves no float area.
        ("horizontal line", "inset(50% 0 50% 0)", 0.0),
    ] {
        let trace = trace(
            &format!(".f{{width:100pt;height:54pt;shape-outside:{shape}}}"),
            &format!("<div class=f></div><p>{WORDS}</p>"),
        );
        assert_left_edges(label, &trace.pages[0], 0..3, CURVE_TOLERANCE, |_| edge);
    }
}

#[test]
fn floats_are_placed_by_margin_boxes_not_by_their_float_areas() {
    let trace = trace(
        ".f{width:60pt;height:54pt}.a{shape-outside:inset(50% 0 50% 0)}",
        &format!("<div class='f a'></div><div class=f></div><p>{WORDS}</p>"),
    );
    assert_left_edges(
        "beside a plain float stacked beside an empty float area",
        &trace.pages[0],
        0..3,
        0.0,
        |_| 120.0,
    );
}

#[test]
fn box_values_follow_their_edges_and_corner_curvature() {
    // §5: a 60pt border-box square with 6pt borders, 6pt padding, 12pt margins and a 30pt
    // border radius. The margin box radius is 30 + 12 = 42pt (radius ≥ margin); the
    // content box radius is 30 - 6 - 6 = 18pt.
    let css = ".f{width:36pt;height:36pt;padding:6pt;border:6pt solid;margin:12pt;\
               border-radius:30pt}";
    for (label, value, inset, radius) in [
        ("margin-box", "margin-box", 0.0, 42.0),
        ("border-box", "border-box", 12.0, 30.0),
        ("padding-box", "padding-box", 18.0, 24.0),
        ("content-box", "content-box", 24.0, 18.0),
    ] {
        let trace = trace(
            &format!("{css}.f{{shape-outside:{value}}}p{{line-height:9pt}}"),
            &format!("<div class=f></div><p>{}</p>", [WORDS; 2].join(" ")),
        );
        let lines = &trace.pages[0];
        let right = 84.0 - inset;
        let [top_edge, bottom_edge] = [inset, 84.0 - inset];
        for line in &lines[..9] {
            let top = PAGE_TOP + line.baseline - lines[0].baseline - PAGE_TOP;
            let bottom = top + 9.0;
            let expected = if bottom <= top_edge || top >= bottom_edge {
                0.0
            } else {
                let corner = if bottom < top_edge + radius {
                    Some(top_edge + radius - bottom)
                } else if top > bottom_edge - radius {
                    Some(top - (bottom_edge - radius))
                } else {
                    None
                };
                corner.map_or(right, |dy| {
                    right - radius + (radius * radius - dy * dy).max(0.0).sqrt()
                })
            };
            assert!(
                (line.x - PAGE_LEFT - expected).abs() <= CURVE_TOLERANCE,
                "{label}: line {:?} over {top}..{bottom}pt starts {}pt in, expected {expected}pt",
                line.text,
                line.x - PAGE_LEFT
            );
        }
    }
}

/// The outermost x within rows `top..=bottom` of a polygon grown by `margin`, sampled along
/// its edges as the union of discs around them (CSS Shapes 1 §6.3).
fn brute_force_margin_edge(polygon: &[[f32; 2]], margin: f32, [top, bottom]: [f32; 2]) -> f32 {
    const SAMPLES: usize = 4000;
    let mut edge = f32::NEG_INFINITY;
    for (index, start) in polygon.iter().enumerate() {
        let end = polygon[(index + 1) % polygon.len()];
        for step in 0..=SAMPLES {
            let t = step as f32 / SAMPLES as f32;
            let point = [
                start[0] + (end[0] - start[0]) * t,
                start[1] + (end[1] - start[1]) * t,
            ];
            let dy = if point[1] < top {
                top - point[1]
            } else if point[1] > bottom {
                point[1] - bottom
            } else {
                0.0
            };
            if dy <= margin {
                edge = edge.max(point[0] + (margin * margin - dy * dy).sqrt());
            }
        }
    }
    edge
}

#[test]
fn shape_margin_grows_a_polygon_by_its_perpendicular_offset_and_round_vertices() {
    // §6.3 example: a 100px square float with this polygon and a 10px margin, in points.
    let polygon = [
        [10.0, 10.0],
        [90.0, 50.0],
        [40.0, 50.0],
        [90.0, 90.0],
        [10.0, 90.0],
    ]
    .map(|point: [f32; 2]| point.map(|value| value * 0.75));
    let trace = trace(
        ".f{width:75pt;height:75pt;shape-margin:7.5pt;shape-outside:polygon(10px 10px,90px 50px,\
         40px 50px,90px 90px,10px 90px)}p{line-height:6pt}",
        &format!("<div class=f></div><p>{}</p>", [WORDS; 2].join(" ")),
    );
    let lines = &trace.pages[0];
    for line in &lines[..12] {
        let top = line.baseline - lines[0].baseline;
        let expected = brute_force_margin_edge(&polygon, 7.5, [top, top + 6.0]).clamp(0.0, 75.0);
        assert!(
            (line.x - PAGE_LEFT - expected).abs() <= CURVE_TOLERANCE,
            "line {:?} over {top}..{}pt starts {}pt in, expected the grown polygon's edge \
             {expected}pt",
            line.text,
            top + 6.0,
            line.x - PAGE_LEFT
        );
    }
    assert!(
        lines[..12]
            .iter()
            .any(|line| line.x - PAGE_LEFT < 75.0 - 1.0),
        "the notch between the right vertices must stay visible: {:?}",
        lines[..12]
            .iter()
            .map(|line| line.x - PAGE_LEFT)
            .collect::<Vec<_>>()
    );
}

#[test]
fn gradient_alpha_above_the_threshold_defines_the_area() {
    // Opacity falls from 1 at the left to 0 at the right of the 100pt content box.
    for (threshold, edge) in [("0.25", 75.0), ("60%", 40.0)] {
        let trace = trace(
            &format!(
                ".f{{width:100pt;height:54pt;shape-outside:linear-gradient(to left,transparent,black);\
                 shape-image-threshold:{threshold}}}"
            ),
            &format!("<div class=f></div><p>{WORDS}</p>"),
        );
        assert_left_edges(threshold, &trace.pages[0], 0..3, SAMPLE_TOLERANCE, |_| edge);
    }
    // Along the diagonal, opacity falls with x + y: above one half where x + y < 100pt, so
    // each line box reaches furthest at its top.
    let trace = trace(
        ".f{width:100pt;height:100pt;shape-image-threshold:0.5;\
         shape-outside:linear-gradient(to bottom right,black,transparent)}",
        &format!("<div class=f></div><p>{WORDS}</p>"),
    );
    assert_left_edges("diagonal", &trace.pages[0], 0..5, SAMPLE_TOLERANCE, |top| {
        100.0 - top
    });
}

/// A 4×2 PNG: opaque on the left half, `alpha` on the right half.
fn half_opaque_png(alpha: u8) -> String {
    let mut pixels = image::RgbaImage::new(4, 2);
    for (x, _, pixel) in pixels.enumerate_pixels_mut() {
        *pixel = image::Rgba([0, 0, 0, if x < 2 { u8::MAX } else { alpha }]);
    }
    let mut bytes = Vec::new();
    pixels
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode the probe PNG");
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

#[test]
fn raster_alpha_is_sized_to_the_content_box_and_thresholded() {
    // The image fills the 80pt content box inside 10pt padding (§4).
    let image = half_opaque_png(100);
    for (threshold, edge) in [(0.5, 50.0), (0.3, 90.0)] {
        let trace = trace(
            &format!(
                ".f{{width:80pt;height:36pt;padding:10pt;shape-outside:url({image});\
                 shape-image-threshold:{threshold}}}"
            ),
            &format!("<div class=f></div><p>{WORDS}</p>"),
        );
        // Every line box from 0 to 54pt meets the content box's rows from 10 to 46pt.
        let lines = &trace.pages[0];
        assert_left_edges(
            &format!("threshold {threshold}"),
            lines,
            0..3,
            0.001,
            |_| edge,
        );
        assert_left_edges(&format!("threshold {threshold}"), lines, 4..5, 0.0, |_| 0.0);
    }
}

#[test]
fn unavailable_or_undecodable_shape_images_leave_the_margin_box_with_a_diagnostic() {
    // §6.1: without a valid image the effect is as if `none` had been specified.
    let mut corrupt = half_opaque_png(0).into_bytes();
    let samples = corrupt.len() - 8;
    corrupt.truncate(samples);
    let corrupt = String::from_utf8(corrupt).expect("base64 stays ASCII");
    for (label, source) in [
        ("missing", "missing-shape.png".to_owned()),
        ("not an image", "data:image/png;base64,YmFk".to_owned()),
        ("truncated samples", corrupt),
    ] {
        let output = ferri_pdf::render_html(
            &format!(
                "<style>{PROFILE}p{{margin:0}}.f{{float:left;width:100pt;height:54pt;\
                 margin-right:8pt;shape-outside:url({source})}}</style>\
                 <div class=f></div><p>{WORDS}</p>"
            ),
            &common::local_options(),
        )
        .unwrap_or_else(|error| panic!("{label}: a failed shape image must not abort: {error}"));
        assert_eq!(
            output.diagnostics.len(),
            1,
            "{label}: the failed image is reported once: {:?}",
            output.diagnostics
        );
        let trace = output.layout.expect("layout requested");
        assert_left_edges(label, &trace.pages[0], 0..3, 0.0, |_| 108.0);
    }
}

#[test]
fn split_floats_wrap_each_page_around_their_slice_of_the_shape() {
    // A 100pt float 540pt tall breaks after the 408pt page area; its triangle widens down
    // the whole box, so page two continues from 408pt into the shape.
    let trace = trace(
        ".f{width:100pt;height:540pt;shape-outside:polygon(0 0,100% 100%,0 100%)}",
        &format!("<div class=f></div><p>{}</p>", [WORDS; 12].join(" ")),
    );
    assert!(trace.pages.len() >= 2, "the float continues on page two");
    let width_at = |y: f32| 100.0 * y / 540.0;
    assert_left_edges("page one", &trace.pages[0], 0..10, CURVE_TOLERANCE, |top| {
        width_at(top + LINE_HEIGHT)
    });
    assert_left_edges("page two", &trace.pages[1], 0..5, CURVE_TOLERANCE, |top| {
        width_at(PAGE_CONTENT_HEIGHT + top + LINE_HEIGHT)
    });
}

#[test]
fn clearance_uses_the_margin_box_not_the_float_area() {
    let trace = trace(
        ".f{width:100pt;height:90pt;shape-outside:circle(20pt at 20pt 20pt)}.c{clear:left}",
        "<div class=f></div><p>aa</p><p class=c>bb</p>",
    );
    let lines = &trace.pages[0];
    assert_eq!(lines.len(), 2, "two one-line paragraphs");
    assert_left_edges("first line", lines, 0..1, CURVE_TOLERANCE, |top| {
        circle_edge([20.0, 20.0], 20.0, top)
    });
    let cleared = line_top(&lines[1], &lines[0]);
    assert!(
        (cleared - (PAGE_TOP + 90.0)).abs() <= 0.001,
        "the cleared paragraph starts below the 90pt margin box, though the circle ends at \
         40pt: {cleared}"
    );
}
