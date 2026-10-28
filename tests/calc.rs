mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{BoxPaint, DisplayItem, LayoutTrace, Rect, render_html};

fn render(body: &str, css: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}p{{margin:0}}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("calc layout must render: {error}"))
    .layout
    .unwrap()
}
fn paints(trace: &LayoutTrace) -> Vec<&BoxPaint> {
    trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) => Some(paint),
            _ => None,
        })
        .collect()
}
fn assert_rect(actual: Rect, expected: [f32; 4], context: &str) {
    let actual_values = [actual.x, actual.y, actual.width, actual.height];
    assert!(
        actual_values
            .iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() < 1e-3),
        "{context}: expected [x, y, width, height] {expected:?}, got {actual_values:?}"
    );
}

#[test]
fn one_calculation_resolves_against_each_containing_block() {
    let trace = render(
        "<section class=wide><div>A</div></section><section class=narrow><div>B</div></section>",
        "section{border:1pt solid navy}.wide{width:200pt}.narrow{width:100pt}div{height:10pt;background:#fea;width:calc(100% - 20pt);margin-left:min(10%, 15pt)}",
    );
    let boxes = paints(&trace);
    assert_rect(
        boxes[1].rect,
        [37.0 + 15.0, 37.0, 180.0, 10.0],
        "200pt container: width 100% - 20pt, margin min(20pt, 15pt)",
    );
    assert_rect(
        boxes[3].rect,
        [37.0 + 10.0, 49.0, 80.0, 10.0],
        "100pt container: width 100% - 20pt, margin min(10pt, 15pt)",
    );
}

#[test]
fn flex_bases_gaps_and_absolute_insets_keep_percentage_terms() {
    let trace = render(
        "<div class=row><div class=one></div><div class=two></div></div><div class=rel><div class=abs></div></div>",
        ".row{display:flex;width:200pt;height:20pt;gap:calc(5% + 2pt)}.one{flex:0 0 calc(50% - 30pt);background:#fea}.two{flex:0 0 calc(25% + 10pt);background:#9cf}\
         .rel{position:relative;width:200pt;height:100pt}.abs{position:absolute;left:calc(50% - 20pt);top:calc(100% - 30pt);width:max(25%, 10pt);height:clamp(5pt, 50%, 20pt);background:#fea}",
    );
    let boxes = paints(&trace);
    assert_rect(
        boxes[0].rect,
        [36.0, 36.0, 70.0, 20.0],
        "flex-basis 50% - 30pt",
    );
    assert_rect(
        boxes[1].rect,
        [36.0 + 70.0 + 12.0, 36.0, 60.0, 20.0],
        "gap 5% + 2pt before flex-basis 25% + 10pt",
    );
    assert_rect(
        boxes[3].rect,
        [36.0 + 80.0, 56.0 + 70.0, 50.0, 20.0],
        "absolute insets and sizes resolve against the 200pt by 100pt containing block",
    );
}

#[test]
fn radii_and_padding_calculations_resolve_against_their_own_box() {
    let trace = render(
        "<section><div>Text</div></section>",
        "section{width:200pt}div{width:100pt;height:40pt;padding-left:calc(10% + 2pt);background:#fea;border-radius:calc(10% + 2pt) / calc(50% - 10pt)}",
    );
    let paint = paints(&trace)[0];
    assert_rect(
        paint.rect,
        [36.0, 36.0, 122.0, 40.0],
        "padding-left 10% of the 200pt containing block plus 2pt",
    );
    assert_eq!(
        paint.radii()[0],
        [122.0 * 0.1 + 2.0, 40.0 * 0.5 - 10.0],
        "horizontal radius uses the border-box width, vertical the height"
    );
}
