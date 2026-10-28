//! CSS Text Decoration 3: propagation to in-flow content, decorating-box font metrics and
//! painting order. Font metrics are read from the bundled DejaVu Sans and STIX General files.
mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{
    Color, DecorationLine, DecorationPaint, DisplayItem, LayoutTrace, TextLine, render_html,
};

const TOLERANCE_PT: f32 = 0.001;
const DEJAVU_UNITS_PER_EM: f32 = 2048.0;
/// `post` underline position (top edge, negative below the baseline) and thickness.
const DEJAVU_UNDERLINE: [f32; 2] = [-130.0, 90.0];
const STIX_UNITS_PER_EM: f32 = 1000.0;
/// `OS/2` strikeout position (top edge above the baseline) and size.
const STIX_STRIKEOUT: [f32; 2] = [306.0, 50.0];
const RED: [u8; 3] = [255, 0, 0];
const STIX_FACE: &str = "@font-face{font-family:Stix;src:url('../assets/STIXGeneral.ttf')}";

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{STIX_FACE}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("decoration fixture must render: {error}\n{body}"))
    .layout
    .unwrap()
}
fn decorations(trace: &LayoutTrace) -> Vec<DecorationPaint> {
    trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Decoration(paint) => Some(*paint),
            _ => None,
        })
        .collect()
}
fn line<'a>(trace: &'a LayoutTrace, text: &str) -> &'a TextLine {
    trace.pages[0]
        .iter()
        .find(|line| line.text.contains(text))
        .unwrap_or_else(|| panic!("no line contains {text:?}"))
}
fn assert_close(label: &str, actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= TOLERANCE_PT,
        "{label}: got {actual}, expected {expected}"
    );
}
/// `[left, right]` page extents of the decorations of `kind`, left to right.
fn extents(paints: &[DecorationPaint], kind: DecorationLine) -> Vec<[f32; 2]> {
    let mut extents: Vec<[f32; 2]> = paints
        .iter()
        .filter(|paint| paint.line == kind)
        .map(|paint| [paint.rect.x, paint.rect.x + paint.rect.width])
        .collect();
    extents.sort_by(|a, b| a[0].total_cmp(&b[0]));
    extents
}

#[test]
fn propagated_lines_use_the_specifying_colour_and_the_paragraph_font() {
    // This underline is red but 12pt thick and deep, from the paragraph.
    let trace = render(
        ".d{text-decoration:underline;color:red;font-size:20pt}.d p{color:blue;font-size:12pt}",
        r#"<div class="d"><p>Paragraph</p></div>"#,
    );
    let paints = decorations(&trace);
    let text = line(&trace, "Paragraph");
    assert_eq!(paints.len(), 1, "one underline: {paints:?}");
    let underline = paints[0];
    let em = 12.0 / DEJAVU_UNITS_PER_EM;
    assert_eq!(
        underline.color,
        Color {
            rgb: RED,
            alpha: 1.0
        },
        "currentcolor resolves on the div that specifies the line"
    );
    assert_close(
        "thickness from the paragraph's 12pt font",
        underline.rect.height,
        DEJAVU_UNDERLINE[1] * em,
    );
    assert_close(
        "top edge at the post underline position below the paragraph baseline",
        underline.rect.y - text.baseline,
        -DEJAVU_UNDERLINE[0] * em,
    );
    assert_close("starts with the text", underline.rect.x, text.x);
    assert_close("spans the text", underline.rect.width, text.width);
}

#[test]
fn lines_skip_nested_box_edges_atomic_inlines_and_out_of_flow_boxes() {
    let trace = render(
        ".u{text-decoration:underline}.pad{padding:0 8pt;border-left:2pt solid navy}.ib{display:inline-block;padding:0 3pt;text-decoration:overline}",
        r#"<p class="u" style="position:relative">Aa <span class="pad">bb</span> <span class="ib">ib</span> cc<span style="position:absolute;left:200pt">abs</span></p>"#,
    );
    let paints = decorations(&trace);
    let outer = line(&trace, "Aa");
    let runs: Vec<[f32; 2]> = outer
        .runs
        .iter()
        .map(|run| [outer.x + run.x, outer.x + run.x + run.width])
        .collect();
    let bb = outer
        .runs
        .iter()
        .find(|run| run.text == "bb")
        .expect("the padded span has its own run");
    let underlines = extents(&paints, DecorationLine::Underline);
    assert_eq!(
        underlines.len(),
        4,
        "\"Aa \", the span's text, the space before the inline-block, then \" cc\": {underlines:?} over runs {runs:?}"
    );
    assert_close(
        "the first line stops at the span's left border",
        underlines[0][1],
        outer.x + bb.x - 10.0,
    );
    assert_close(
        "the span's padding and border are not underlined",
        underlines[1][0],
        outer.x + bb.x,
    );
    let atomic = line(&trace, "ib");
    let atomic_box = [atomic.x - 3.0, atomic.x + atomic.width + 3.0];
    assert!(
        underlines
            .iter()
            .all(|[left, right]| *right <= atomic_box[0] + TOLERANCE_PT
                || *left >= atomic_box[1] - TOLERANCE_PT),
        "no underline passes under the inline-block {atomic_box:?}: {underlines:?}"
    );
    let overlines = extents(&paints, DecorationLine::Overline);
    assert_eq!(
        overlines,
        vec![[atomic.x, atomic.x + atomic.width]],
        "the inline-block decorates its own text only"
    );
    let absolute = line(&trace, "abs");
    assert!(
        paints
            .iter()
            .all(|paint| paint.rect.x + paint.rect.width <= absolute.x + TOLERANCE_PT),
        "the absolutely positioned box receives no decorations: {paints:?}"
    );
}

#[test]
fn a_decorating_span_keeps_its_own_metrics_and_raised_text_keeps_the_root_line() {
    let trace = render(
        ".u{text-decoration:underline}.s{vertical-align:super;font-size:20pt;font-family:Stix;text-decoration:line-through}",
        r#"<p class="u">Root <span class="s">up<span style="font-size:10pt">small</span></span></p>"#,
    );
    let paints = decorations(&trace);
    let text = line(&trace, "Root");
    let underlines: Vec<_> = paints
        .iter()
        .filter(|paint| paint.line == DecorationLine::Underline)
        .collect();
    assert_eq!(
        underlines.len(),
        1,
        "one root underline across raised text: {underlines:?}"
    );
    assert_close(
        "the root underline ignores the span's superscript shift",
        underlines[0].rect.y - text.baseline,
        -DEJAVU_UNDERLINE[0] * 12.0 / DEJAVU_UNITS_PER_EM,
    );
    let span = text
        .runs
        .iter()
        .find(|run| run.text == "up")
        .expect("the span's text");
    let through: Vec<_> = paints
        .iter()
        .filter(|paint| paint.line == DecorationLine::LineThrough)
        .collect();
    assert_eq!(
        through.len(),
        1,
        "the span's line-through also crosses its smaller child: {through:?}"
    );
    let em = 20.0 / STIX_UNITS_PER_EM;
    assert_close(
        "line-through thickness from the span's OS/2 strikeout size",
        through[0].rect.height,
        STIX_STRIKEOUT[1] * em,
    );
    assert_close(
        "line-through top from the span's OS/2 strikeout position at its raised baseline",
        through[0].rect.y - (text.baseline + span.baseline_shift),
        -STIX_STRIKEOUT[0] * em,
    );
    let items = &trace.display_lists[0];
    let text_item = items
        .iter()
        .position(|item| matches!(item, DisplayItem::Text { .. }))
        .unwrap();
    let position = |kind: DecorationLine| {
        items
            .iter()
            .position(|item| matches!(item, DisplayItem::Decoration(paint) if paint.line == kind))
            .unwrap()
    };
    assert!(
        position(DecorationLine::Underline) < text_item
            && text_item < position(DecorationLine::LineThrough),
        "underlines paint below the text and line-throughs above it (CSS 2.1 E.2): {items:?}"
    );
}
