//! CSS Overflow 3 `text-overflow: ellipsis`: what is hidden, where the ellipsis goes and
//! how inline boxes, atomic inlines and right-to-left lines are truncated.
mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{BoxPaint, DisplayItem, GlyphRun, LayoutTrace, TextLine, render_html};

const TOLERANCE_PT: f32 = 0.001;
const ELLIPSIS: &str = "\u{2026}";
const CONTENT_LEFT_PT: f32 = 36.0;
const BOX_WIDTH_PT: f32 = 80.0;
const BOX: &str =
    "div{width:80pt;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;font-size:10pt}";

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{BOX}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("text-overflow fixture must render: {error}\n{body}"))
    .layout
    .unwrap()
}
fn only_line(trace: &LayoutTrace) -> &TextLine {
    assert_eq!(
        trace.pages[0].len(),
        1,
        "one line: {:?}",
        trace.pages[0].iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    &trace.pages[0][0]
}
/// Page extent `[left, right]` of a run.
fn extent(line: &TextLine, run: &GlyphRun) -> [f32; 2] {
    [line.x + run.x, line.x + run.x + run.width]
}
fn ellipsis(line: &TextLine) -> &GlyphRun {
    let last = line.runs.last().expect("an ellipsed line has runs");
    assert_eq!(
        last.text,
        ELLIPSIS,
        "the ellipsis ends the line's runs: {:?}",
        line.runs.iter().map(|run| &run.text).collect::<Vec<_>>()
    );
    last
}
/// Text of the glyph clusters a run still paints.
fn painted(run: &GlyphRun) -> String {
    let mut clusters: Vec<_> = run
        .glyphs()
        .iter()
        .map(|glyph| glyph.text_range.clone())
        .collect();
    clusters.sort_by_key(|range| range.start);
    clusters.dedup();
    clusters
        .iter()
        .map(|range| &run.text[range.clone()])
        .collect()
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

#[test]
fn a_truncated_inline_box_ends_at_the_ellipsis_without_its_end_edge() {
    let trace = render(
        ".box{background:#fea;padding:0 4pt;border:1pt solid navy;color:blue;font-size:8pt}div{color:red}",
        r#"<div>Over <span class="box">flowing words</span> on</div>"#,
    );
    let line = only_line(&trace);
    let ellipsis = ellipsis(line);
    let [start, end] = extent(line, ellipsis);
    assert_eq!(
        (ellipsis.font_size, ellipsis.color.rgb),
        (10.0, [255, 0, 0]),
        "the ellipsis is styled by the block, not the truncated span"
    );
    assert!(
        end <= CONTENT_LEFT_PT + BOX_WIDTH_PT + TOLERANCE_PT,
        "the ellipsis fits in the line box: ends at {end}"
    );
    let span = line
        .runs
        .iter()
        .find(|run| run.text.starts_with("flowing"))
        .expect("the span's run is kept in part");
    let kept = painted(span);
    assert!(
        "flowing words".starts_with(&kept) && kept.len() < "flowing words".len(),
        "whole clusters are hidden from the span's end: painted {kept:?}"
    );
    assert!(
        (extent(line, span)[1] - start).abs() <= TOLERANCE_PT,
        "the ellipsis immediately follows the remaining text: {:?} then {start}",
        extent(line, span)
    );
    let fragment = boxes(&trace)
        .into_iter()
        .find(|paint| paint.decoration.background.rgb == [0xff, 0xee, 0xaa])
        .expect("the span's fragment is painted");
    assert_eq!(
        fragment.inline_edges,
        [true, false],
        "the span keeps its start edge and loses its hidden end edge"
    );
    assert!(
        (fragment.rect.x + fragment.rect.width - start).abs() <= TOLERANCE_PT,
        "the span's fragment is clipped where the ellipsis begins: {:?} vs {start}",
        fragment.rect
    );
    assert!(
        line.runs
            .iter()
            .all(|run| !run.text.contains("on") || run.text == "flowing words"),
        "the text after the span is hidden: {:?}",
        line.runs.iter().map(|run| &run.text).collect::<Vec<_>>()
    );
}

#[test]
fn hidden_atomic_inlines_are_not_painted_and_box_starts_without_content_are_dropped() {
    let trace = render(
        ".ib{display:inline-block;width:60pt;height:8pt;background:#9cf}em{margin-left:20pt}",
        r#"<div>Over <span class="ib"></span> on</div><div>Overflowing <em>words</em></div>"#,
    );
    assert!(
        boxes(&trace)
            .iter()
            .all(|paint| paint.decoration.background.rgb != [0x99, 0xcc, 0xff]),
        "the inline-block that would overlap the ellipsis is hidden whole"
    );
    let [first, second] = [&trace.pages[0][0], &trace.pages[0][1]];
    let over = &first.runs[0];
    assert_eq!(over.text, "Over ", "the leading text stays");
    assert!(
        (extent(first, ellipsis(first))[0] - extent(first, over)[1]).abs() <= TOLERANCE_PT,
        "the ellipsis follows the text before the hidden inline-block"
    );
    let before = &second.runs[0];
    assert!(
        second.runs.iter().all(|run| !run.text.contains("words")),
        "the em beyond the ellipsis is hidden: {:?}",
        second.runs.iter().map(|run| &run.text).collect::<Vec<_>>()
    );
    assert!(
        (extent(second, ellipsis(second))[0] - extent(second, before)[1]).abs() <= TOLERANCE_PT,
        "a hidden em takes its left margin with it, so the ellipsis follows {:?}",
        before.text
    );
}

#[test]
fn right_to_left_lines_lose_their_left_end_to_an_ellipsis_at_the_left() {
    let trace = render("", r#"<div dir="rtl">אבג דהו זחט יכל מנס עפצ</div>"#);
    let line = only_line(&trace);
    let ellipsis = ellipsis(line);
    let [left, _] = extent(line, ellipsis);
    let content: Vec<[f32; 2]> = line.runs[..line.runs.len() - 1]
        .iter()
        .map(|run| extent(line, run))
        .collect();
    let right = content
        .iter()
        .map(|[_, right]| *right)
        .fold(f32::MIN, f32::max);
    assert!(
        (right - (CONTENT_LEFT_PT + BOX_WIDTH_PT)).abs() <= TOLERANCE_PT,
        "kept text stays aligned to the inline start at the right: ends at {right}"
    );
    assert!(
        left >= CONTENT_LEFT_PT - TOLERANCE_PT
            && content
                .iter()
                .all(|[start, _]| *start >= extent(line, ellipsis)[1] - TOLERANCE_PT),
        "the ellipsis is the leftmost content and fits: ellipsis at {left}, content {content:?}"
    );
    let kept: String = line.runs[..line.runs.len() - 1]
        .iter()
        .map(painted)
        .collect();
    assert!(
        kept.starts_with("אבג") && !kept.contains("עפצ"),
        "the logical end is hidden, the start kept: {kept:?}"
    );
}

#[test]
fn truncation_never_splits_a_grapheme_cluster() {
    let trace = render(
        "",
        "<div>A&#x301;B&#x301;C&#x301;D&#x301;E&#x301;F&#x301;G&#x301;H&#x301;I&#x301;J&#x301;K&#x301;L&#x301;M&#x301;</div>",
    );
    let line = only_line(&trace);
    ellipsis(line);
    let run = &line.runs[0];
    let kept: Vec<char> = painted(run).chars().collect();
    assert!(
        !kept.is_empty()
            && kept.len().is_multiple_of(2)
            && kept
                .chunks(2)
                .all(|pair| pair[0].is_ascii_uppercase() && pair[1] == '\u{301}'),
        "each kept base keeps its combining accent: {kept:?}"
    );
}

#[test]
fn a_line_whose_first_character_cannot_precede_the_ellipsis_is_clipped() {
    let trace = render(
        "",
        r#"<div style="width:12pt">Overflowing</div><div style="width:18pt">Overflowing</div>"#,
    );
    let [clipped, ellipsed] = [&trace.pages[0][0], &trace.pages[0][1]];
    assert!(
        clipped.runs.iter().all(|run| run.text != ELLIPSIS)
            && painted(&clipped.runs[0]) == "Overflowing",
        "\"O\" and an ellipsis exceed 12pt, so nothing is hidden: {:?}",
        clipped.runs.iter().map(painted).collect::<Vec<_>>()
    );
    assert_eq!(
        painted(&ellipsed.runs[0]),
        "O",
        "at 18pt the first character is followed by the ellipsis"
    );
    ellipsis(ellipsed);
}

#[test]
fn two_values_ellipse_only_at_the_edge_where_each_direction_ends() {
    let trace = render(
        "div{text-overflow:clip ellipsis}",
        r#"<div>Overflowing words on one line</div><div dir="rtl">אבג דהו זחט יכל מנס עפצ</div>"#,
    );
    let [ltr, rtl] = [&trace.pages[0][0], &trace.pages[0][1]];
    ellipsis(ltr);
    assert!(
        rtl.runs.iter().all(|run| run.text != ELLIPSIS),
        "a right-to-left line ends at the line-left edge, whose value is clip: {:?}",
        rtl.runs.iter().map(painted).collect::<Vec<_>>()
    );
}

#[test]
fn single_line_text_inputs_ellipse_their_value_regardless_of_overflow() {
    const VALUE: &str = "an overflowing value";
    for (input, ellipsed) in [
        (format!("<input value='{VALUE}'>"), true),
        (format!("<input type=search value='{VALUE}'>"), true),
        (format!("<input placeholder='{VALUE}'>"), true),
        (
            format!("<input style='overflow:visible' value='{VALUE}'>"),
            true,
        ),
        (
            format!("<input style='text-overflow:clip' value='{VALUE}'>"),
            false,
        ),
    ] {
        let trace = render(
            "input{width:50pt;text-overflow:ellipsis;font-size:10pt}",
            &input,
        );
        // The body's line holds the control as an atomic inline; the value is its own line.
        let line = trace.pages[0]
            .iter()
            .find(|line| line.text.starts_with(&VALUE[..1]))
            .unwrap_or_else(|| panic!("{input}: the value's line"));
        let runs: Vec<_> = line.runs.iter().map(painted).collect();
        let last = line.runs.last().expect("the value has runs");
        assert_eq!(last.text == ELLIPSIS, ellipsed, "{input}: {runs:?}");
        if ellipsed {
            let kept: String = runs[..runs.len() - 1].concat();
            assert!(
                VALUE.starts_with(&kept) && kept.len() < VALUE.len(),
                "{input}: whole clusters are hidden from the value's end: {kept:?}"
            );
        }
    }
}
