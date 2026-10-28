//! Inline-level boxes: edges, fragments, vertical alignment and atomic inlines.
use super::common;
use crate::{BoxPaint, DisplayItem, LayoutTrace, Rect, TextLine, render_html};
use common::{PROFILE, local_options};

const TOLERANCE_PT: f32 = 0.01;
/// x-height comes from the `x` glyph bounds.
const X_HEIGHT_TOLERANCE_PT: f32 = 0.02;
const TALL_PAGE: &str = "@page{size:360pt 900pt}";
const DEJAVU_SPACE_UNITS: f32 = 651.0;
const DEJAVU_UNITS_PER_EM: f32 = 2048.0;
const PROFILE_FONT_SIZE_PT: f32 = 12.0;
/// Runs after an inline box begin with the collapsed space before the next word.
const SPACE_PT: f32 = DEJAVU_SPACE_UNITS / DEJAVU_UNITS_PER_EM * PROFILE_FONT_SIZE_PT;

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("inline fixture must render: {error}\n{body}"))
    .layout
    .unwrap()
}
fn line<'a>(trace: &'a LayoutTrace, text: &str) -> &'a TextLine {
    trace.pages[0]
        .iter()
        .find(|line| line.text.contains(text))
        .unwrap_or_else(|| {
            panic!(
                "no line contains {text:?}: {:?}",
                trace.pages[0].iter().map(|l| &l.text).collect::<Vec<_>>()
            )
        })
}
/// Page x and baseline of the run holding `text`.
fn run_position(trace: &LayoutTrace, text: &str) -> [f32; 2] {
    for line in &trace.pages[0] {
        for run in &line.runs {
            if run.text.contains(text) {
                return [line.x + run.x, line.baseline + run.baseline_shift];
            }
        }
    }
    panic!("no run contains {text:?}")
}
fn boxes(trace: &LayoutTrace) -> Vec<&BoxPaint> {
    trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) => Some(paint),
            _ => None,
        })
        .collect()
}
fn assert_close(label: &str, actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: {actual} differs from expected {expected} by more than {tolerance}"
    );
}
fn assert_rect(label: &str, actual: Rect, expected: [f32; 4]) {
    for (name, value, wanted) in [
        ("x", actual.x, expected[0]),
        ("y", actual.y, expected[1]),
        ("width", actual.width, expected[2]),
        ("height", actual.height, expected[3]),
    ] {
        assert_close(
            &format!("{label} {name} ({actual:?})"),
            value,
            wanted,
            TOLERANCE_PT,
        );
    }
}

#[test]
fn horizontal_edges_advance_text_and_vertical_edges_paint_outside_the_line() {
    let trace = render(
        ".a{background:#fea;padding:4pt 10pt;border:2pt solid navy;margin:0 6pt}",
        "<p>AA <span class='a'>BB</span> CC</p><p>Next</p>",
    );
    assert_close(
        "BB after margin, border and padding",
        run_position(&trace, "BB")[0],
        74.5664,
        TOLERANCE_PT,
    );
    assert_close(
        "CC after the closing edges",
        run_position(&trace, "CC")[0],
        112.8457 - SPACE_PT,
        TOLERANCE_PT,
    );
    let paints = boxes(&trace);
    assert_eq!(
        paints.len(),
        1,
        "one fragment for the unbroken span: {paints:?}"
    );
    assert_rect(
        "border box spans content area plus padding and border",
        paints[0].rect,
        [62.5664, 32.0156, 40.4648, 25.9688],
    );
    assert_close(
        "vertical padding leaves the 18pt line height unchanged",
        line(&trace, "Next").baseline - line(&trace, "AA").baseline,
        30.0,
        TOLERANCE_PT,
    );
    let order: Vec<&str> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(_) => Some("box"),
            DisplayItem::Text { line } => Some(trace.pages[0][*line].text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        ["box", "AA BB CC", "Next"],
        "inline backgrounds paint below the text of their line"
    );
}

#[test]
fn broken_inline_boxes_slice_their_edges_unless_cloned() {
    let css = "p{width:120pt}.a{background:#fea;padding:4pt 10pt;border:2pt solid navy;margin:0 6pt;border-radius:5pt}.c{box-decoration-break:clone}";
    let sliced = render(css, "<p>AA <span class='a'>BB CC DD EE FF</span> GG</p>");
    let fragments = boxes(&sliced);
    assert_eq!(fragments.len(), 2, "one fragment per line: {fragments:?}");
    assert_eq!(
        fragments[0].inline_edges,
        [true, false],
        "first fragment keeps only its start edge"
    );
    assert_eq!(
        fragments[1].inline_edges,
        [false, true],
        "last fragment keeps only its end edge"
    );
    assert_close(
        "first fragment left",
        fragments[0].rect.x,
        62.5664,
        TOLERANCE_PT,
    );
    assert_close(
        "first fragment ends at its last glyph",
        fragments[0].rect.x + fragments[0].rect.width,
        152.877,
        TOLERANCE_PT,
    );
    assert_close(
        "second fragment starts at the line edge",
        fragments[1].rect.x,
        36.0,
        TOLERANCE_PT,
    );
    assert_close(
        "second fragment right edge",
        fragments[1].rect.x + fragments[1].rect.width,
        61.8047,
        TOLERANCE_PT,
    );
    assert!(
        fragments[0].radii() == [[5.0; 2]; 4],
        "sliced corners belong to the whole inline box before it is cut: {:?}",
        fragments[0].radii()
    );
    assert_close(
        "GG follows the closing edges",
        run_position(&sliced, "GG")[0],
        71.6191 - SPACE_PT,
        TOLERANCE_PT,
    );
    let cloned = render(css, "<p>AA <span class='a c'>BB CC DD EE FF</span> GG</p>");
    let fragments = boxes(&cloned);
    assert!(
        fragments
            .iter()
            .all(|paint| paint.inline_edges == [true; 2]),
        "cloned fragments each get both edges: {fragments:?}"
    );
    assert_close(
        "EE moves to line two because each fragment carries both edges",
        run_position(&cloned, "EE")[0],
        54.0,
        TOLERANCE_PT,
    );
    assert_close(
        "GG after the cloned edges",
        run_position(&cloned, "GG")[0],
        108.5977 - SPACE_PT,
        TOLERANCE_PT,
    );
}

#[test]
fn vertical_alignment_positions_boxes_and_grows_line_boxes() {
    let values = [
        "sub",
        "super",
        "text-top",
        "text-bottom",
        "middle",
        "top",
        "bottom",
        "5pt",
        "50%",
        "-20pt",
    ];
    let body: String = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            format!("<p>X{index} <span style='vertical-align:{value}'>v{index}</span></p>")
        })
        .collect();
    let trace = render(&format!("{TALL_PAGE}span{{font-size:6pt}}"), &body);
    // (paragraph baseline, aligned span baseline) per value, top-down.
    let expected = [
        (49.1543, 50.5693),
        (86.1387, 80.5693),
        (116.1387, 116.0771),
        (150.1699, 146.0771),
        (180.1699, 178.9717),
        (211.0488, 208.9717),
        (241.0488, 238.9717),
        (273.9717, 268.9717),
        (310.8945, 301.8945),
        (340.8945, 360.8945),
    ];
    for (index, (value, (paragraph, span))) in values.iter().zip(expected).enumerate() {
        let tolerance = if *value == "middle" {
            X_HEIGHT_TOLERANCE_PT
        } else {
            TOLERANCE_PT
        };
        assert_close(
            &format!("baseline of the paragraph holding vertical-align:{value}"),
            run_position(&trace, &format!("X{index}"))[1],
            paragraph,
            tolerance,
        );
        assert_close(
            &format!("baseline of the span with vertical-align:{value}"),
            run_position(&trace, &format!("v{index}"))[1],
            span,
            tolerance,
        );
    }
}

#[test]
fn top_and_bottom_subtrees_extend_lines_downward_and_empty_boxes_keep_struts() {
    let trace = render(
        TALL_PAGE,
        "<p>Xo <span style='line-height:40pt;vertical-align:top'>tall</span> <span style='line-height:40pt;vertical-align:bottom'>low</span></p><p>Xp <span style='padding:0 4pt;line-height:40pt'></span>Z</p><p>Next</p>",
    );
    let first = line(&trace, "Xo");
    assert_close(
        "baseline-aligned content stays at the top",
        first.baseline - 36.0,
        13.1543,
        TOLERANCE_PT,
    );
    assert_close(
        "the 40pt subtrees set the line height",
        first.height,
        40.0,
        TOLERANCE_PT,
    );
    assert_close(
        "top-aligned span baseline",
        run_position(&trace, "tall")[1],
        36.0 + 24.1543,
        TOLERANCE_PT,
    );
    assert_close(
        "bottom-aligned span baseline",
        run_position(&trace, "low")[1],
        36.0 + 24.1543,
        TOLERANCE_PT,
    );
    let second = line(&trace, "Xp");
    assert_close(
        "an empty inline box still contributes its 40pt strut",
        second.baseline - (36.0 + 40.0 + 12.0),
        24.1543,
        TOLERANCE_PT,
    );
    assert_close(
        "the empty box's padding advances Z",
        run_position(&trace, "Z")[0],
        63.6523,
        TOLERANCE_PT,
    );
}

#[test]
fn inline_block_baselines_use_the_last_line_or_the_bottom_margin_edge() {
    let trace = render(
        &format!(
            "{TALL_PAGE}.ib{{display:inline-block;background:#fea;border:2pt solid navy;padding:3pt;margin:4pt 5pt}}"
        ),
        "<p>Xa <span class='ib'>One<br>Two</span> after</p><p>Xb <span class='ib' style='width:50pt;height:30pt'></span> after</p>",
    );
    assert_close(
        "the paragraph baseline aligns with the inline-block's last line",
        line(&trace, "Xa").baseline,
        76.1543,
        TOLERANCE_PT,
    );
    assert_close(
        "the inline-block's last line shares that baseline",
        line(&trace, "Two").baseline,
        76.1543,
        TOLERANCE_PT,
    );
    assert_close(
        "text after the margin box",
        run_position(&trace, "after")[0],
        103.6367 - SPACE_PT,
        TOLERANCE_PT,
    );
    let paints = boxes(&trace);
    assert_rect(
        "shrink-to-fit inline-block border box",
        paints[0].rect,
        [60.3887, 40.0, 34.4336, 46.0],
    );
    assert_close(
        "an empty inline-block sits on the baseline by its bottom margin edge",
        line(&trace, "Xb").baseline,
        150.0,
        TOLERANCE_PT,
    );
    assert_rect(
        "sized inline-block border box",
        paints[1].rect,
        [60.6523, 106.0, 60.0, 40.0],
    );
}

#[test]
fn atomic_inlines_offer_breaks_on_both_sides_except_under_nowrap() {
    let css = format!(
        "{TALL_PAGE}p{{width:40pt}}img{{width:20pt;height:10pt}}.ib{{display:inline-block;width:20pt;height:10pt;background:#fea}}"
    );
    let trace = render(
        &css,
        "<p>AAA<img src='../assets/image-pattern.png'>BBB</p><p>A<span class='ib'></span><span class='ib'></span><span class='ib'></span></p><p style='white-space:nowrap'>AAA <span class='ib'></span> BBB</p>",
    );
    let lines: Vec<&str> = trace.pages[0]
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        lines,
        [
            "AAA",
            "\u{fffc}",
            "BBB",
            "A\u{fffc}",
            "\u{fffc}\u{fffc}",
            "AAA \u{fffc} BBB"
        ],
        "atomics break from adjacent text and from each other, never under nowrap"
    );
    let image = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Image(paint) => Some(paint.rect),
            _ => None,
        })
        .expect("the inline image is painted");
    assert_rect(
        "image on its own line, bottom on the baseline",
        image,
        [36.0, 57.1543, 20.0, 10.0],
    );
    let paints = boxes(&trace);
    assert_rect(
        "first inline-block follows A",
        paints[0].rect,
        [44.209, 105.1543, 20.0, 10.0],
    );
    assert_rect(
        "second inline-block wraps to the next line",
        paints[1].rect,
        [36.0, 123.1543, 20.0, 10.0],
    );
    assert_close(
        "nowrap keeps the inline-block on the overflowing line",
        paints[3].rect.x,
        65.1094,
        TOLERANCE_PT,
    );
}

#[test]
fn inline_svg_elements_are_vector_images_on_the_baseline() {
    let trace = render(
        "p{margin:0}",
        "<p>Vector <svg width='20' height='10' viewBox='0 0 2 1'><rect width='2' height='1' fill='navy'/></svg> after</p>",
    );
    let paint = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Image(paint) => Some(paint),
            _ => None,
        })
        .expect("inline svg must paint as an image");
    assert!(
        matches!(paint.image.as_ref(), ferri_pdf_images::Image::Svg(_)),
        "inline svg keeps its vector form"
    );
    let baseline = line(&trace, "Vector").baseline;
    assert_close(
        "svg width from its attribute in CSS pixels",
        paint.rect.width,
        15.0,
        TOLERANCE_PT,
    );
    assert_close(
        "svg bottom edge on the baseline",
        paint.rect.y + paint.rect.height,
        baseline,
        TOLERANCE_PT,
    );
    assert_close(
        "the run after the svg, led by its space, starts at the svg right edge",
        run_position(&trace, "after")[0],
        paint.rect.x + paint.rect.width,
        TOLERANCE_PT,
    );
}

#[test]
fn inline_boxes_split_around_blocks_keep_edges_only_at_their_outer_ends() {
    let trace = render(
        ".a{background:#fea;padding:0 8pt;border:2pt solid navy}",
        "<div>Before <span class='a'>inline start<div>Block inside</div>inline end</span> after</div>",
    );
    let fragments = boxes(&trace);
    assert_eq!(
        fragments.len(),
        2,
        "one fragment on each side of the block: {fragments:?}"
    );
    assert_eq!(
        fragments[0].inline_edges,
        [true, false],
        "the part before the block is open at its end"
    );
    assert_eq!(
        fragments[1].inline_edges,
        [false, true],
        "the part after the block is open at its start"
    );
    assert_close(
        "content after the block starts at the line edge",
        run_position(&trace, "inline end")[0],
        36.0,
        TOLERANCE_PT,
    );
    assert_close(
        "the block itself is not indented by the inline padding",
        line(&trace, "Block").x,
        36.0,
        TOLERANCE_PT,
    );
}

#[test]
fn inline_image_backgrounds_are_loaded_and_shared_across_block_splits() {
    use crate::graphics::BackgroundImage;
    use std::sync::Arc;
    let trace = render(
        ".a{background:url(../assets/image-pattern.png);padding:0 8pt;border:2pt solid navy}",
        "<div>Before <span class='a'>inline start<div>Block inside</div>inline end</span> after</div>",
    );
    let fragments = boxes(&trace);
    assert_eq!(
        fragments.len(),
        2,
        "the loaded background must paint both inline continuations"
    );
    let images: Vec<_> = fragments
        .iter()
        .map(|paint| {
            let [BackgroundImage::Image { image }] = paint.decoration.backgrounds.images.as_slice()
            else {
                panic!("each inline continuation must retain its loaded raster background")
            };
            assert_eq!(image.pixel_size(0.0), [80.0, 40.0]);
            image
        })
        .collect();
    assert!(
        Arc::ptr_eq(images[0], images[1]),
        "splitting an inline must share its existing image asset"
    );
    assert_eq!(fragments[0].inline_edges, [true, false]);
    assert_eq!(fragments[1].inline_edges, [false, true]);
}

#[test]
fn vertical_align_math_functions_resolve_against_the_box_line_height() {
    let trace = render(
        &format!("{TALL_PAGE}span{{font-size:6pt}}"),
        "<p>Xa <span style='vertical-align:50%'>va</span></p>\
         <p>Xb <span style='vertical-align:calc(50% + 2pt)'>vb</span></p>\
         <p>Xc <span style='vertical-align:calc(100% - 100%)'>vc</span></p>",
    );
    let raise = |label: &str| {
        run_position(&trace, &format!("X{label}"))[1]
            - run_position(&trace, &format!("v{label}"))[1]
    };
    assert_close(
        "calc(50% + 2pt) raises 2pt above 50%",
        raise("b"),
        raise("a") + 2.0,
        TOLERANCE_PT,
    );
    assert_close(
        "calc(100% - 100%) stays on the baseline",
        raise("c"),
        0.0,
        TOLERANCE_PT,
    );
}

#[test]
fn sliced_inline_backgrounds_span_the_unbroken_box_and_cloned_ones_restart() {
    let css = "p{width:120pt}.a{background:linear-gradient(to right,red,blue);padding:0 10pt}.c{box-decoration-break:clone}";
    let body = |class: &str| format!("<p>AA <span class='{class}'>BB CC DD EE FF</span> GG</p>");
    let sliced = render(css, &body("a"));
    let fragments = boxes(&sliced);
    assert_eq!(fragments.len(), 2, "one fragment per line: {fragments:?}");
    let total = fragments[0].rect.width + fragments[1].rect.width;
    for (index, before) in [(0, 0.0), (1, fragments[0].rect.width)] {
        let fragment = fragments[index];
        let unbroken = fragment
            .background_box
            .unwrap_or_else(|| panic!("sliced fragment {index} needs the unbroken box"));
        assert_rect(
            &format!("fragment {index} positions its gradient in the unbroken box"),
            unbroken,
            [
                fragment.rect.x - before,
                fragment.rect.y,
                total,
                fragment.rect.height,
            ],
        );
    }
    let cloned = render(css, &body("a c"));
    assert!(
        boxes(&cloned)
            .iter()
            .all(|paint| paint.background_box.is_none()),
        "cloned fragments position their gradients in their own boxes"
    );
}
