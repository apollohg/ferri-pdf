//! CSS Text Decoration 3 and 4: propagation to in-flow content, decorating-box font metrics and
//! painting order, skipping and insets. Font metrics are read from the bundled `DejaVu` Sans and STIX General files.
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

/// A paragraph underlined across a bordered, padded span, an overlined inline-block and an
/// absolutely positioned box; `skip` is the span's `text-decoration-skip`.
fn skipping_fixture(skip: &str) -> LayoutTrace {
    render(
        &format!(
            ".u{{text-decoration:underline}}.pad{{padding:0 8pt;border-left:2pt solid navy;text-decoration-skip:{skip}}}.ib{{display:inline-block;padding:0 3pt;text-decoration:overline}}"
        ),
        r#"<p class="u" style="position:relative">Aa <span class="pad">bb</span> <span class="ib">ib</span> cc<span style="position:absolute;left:200pt">abs</span></p>"#,
    )
}

#[test]
fn lines_cross_nested_box_edges_but_skip_atomic_inlines_and_out_of_flow_boxes() {
    let trace = skipping_fixture("objects");
    let paints = decorations(&trace);
    let outer = line(&trace, "Aa");
    let underlines = extents(&paints, DecorationLine::Underline);
    let atomic = line(&trace, "ib");
    let atomic_box = [atomic.x - 3.0, atomic.x + atomic.width + 3.0];
    assert_eq!(
        underlines.len(),
        2,
        "CSS Text Decoration 3 §2.1: the line crosses the span's border and padding, and \
         breaks only around the inline-block: {underlines:?}"
    );
    assert_close("the line starts with the text", underlines[0][0], outer.x);
    assert_close(
        "the line reaches the inline-block's margin edge",
        underlines[0][1],
        atomic_box[0],
    );
    assert_close(
        "the line resumes after the inline-block",
        underlines[1][0],
        atomic_box[1],
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
fn box_decoration_skipping_leaves_a_descendant_border_and_padding_undecorated() {
    let trace = skipping_fixture("objects box-decoration");
    let outer = line(&trace, "Aa");
    let bb = outer
        .runs
        .iter()
        .find(|run| run.text == "bb")
        .expect("the padded span has its own run");
    let underlines = extents(&decorations(&trace), DecorationLine::Underline);
    assert_eq!(
        underlines.len(),
        4,
        "\"Aa \", the span's text, the space before the inline-block, then \" cc\": {underlines:?}"
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
}

#[test]
fn without_object_skipping_the_line_crosses_an_atomic_inline() {
    let trace = render(
        ".u{text-decoration:underline;text-decoration-skip:none}.ib{display:inline-block;width:30pt}",
        r#"<p class="u">Aa<span class="ib"></span>cc</p>"#,
    );
    let text = line(&trace, "Aa");
    let underlines = extents(&decorations(&trace), DecorationLine::Underline);
    assert_eq!(
        underlines.len(),
        1,
        "one continuous line under the text and the inline-block: {underlines:?}"
    );
    assert_close("from the text's start", underlines[0][0], text.x);
    assert_close(
        "to its end, across the 30pt inline-block",
        underlines[0][1],
        text.x + text.width,
    );
}

#[test]
fn space_skipping_interrupts_the_line_at_spacers_only() {
    let underlines = |skip: &str| {
        let trace = render(
            &format!(".u{{text-decoration:underline;white-space:pre;text-decoration-skip:{skip}}}"),
            r#"<p class="u">  ab cd  </p>"#,
        );
        let text = line(&trace, "ab");
        let mut extents = extents(&decorations(&trace), DecorationLine::Underline);
        for extent in &mut extents {
            *extent = extent.map(|x| x - text.x);
        }
        (extents, text.width)
    };
    let (all, width) = underlines("none");
    assert_eq!(all.len(), 1, "nothing skipped: {all:?}");
    assert_close("spaces are decorated", all[0][1] - all[0][0], width);
    let (spaces, _) = underlines("spaces");
    assert_eq!(spaces.len(), 2, "\"ab\" and \"cd\" only: {spaces:?}");
    let (edges, _) = underlines("leading-spaces trailing-spaces");
    assert_eq!(edges.len(), 1, "the inner space stays decorated: {edges:?}");
    assert_close("leading spaces are skipped", edges[0][0], spaces[0][0]);
    assert_close("trailing spaces are skipped", edges[0][1], spaces[1][1]);
    let (leading, _) = underlines("leading-spaces");
    assert_close("only leading spaces", leading[0][0], spaces[0][0]);
    assert_close("trailing spaces stay", leading[0][1], width);
}

#[test]
fn insets_move_line_ends_inside_the_decorating_box() {
    let trace = render(
        "u{text-decoration-thickness:2pt}.a{text-decoration-inset:3pt 5pt}.b{text-decoration-inset:auto}.c{text-decoration-skip:edges}.d{text-decoration-inset:-4pt}",
        r#"<p><u class="a">ab</u><u class="b">cd</u><u class="c">ef</u><u class="d">gh</u></p>"#,
    );
    let text = line(&trace, "abcd");
    let underlines = extents(&decorations(&trace), DecorationLine::Underline);
    assert_eq!(underlines.len(), 4, "one line per box: {underlines:?}");
    let boxes: Vec<[f32; 2]> = text
        .runs
        .iter()
        .map(|run| [text.x + run.x, text.x + run.x + run.width])
        .collect();
    let expected = [[3.0, 5.0], [1.0, 1.0], [1.0, 1.0], [-4.0, -4.0]];
    for (index, ((line, run), [start, end])) in
        underlines.iter().zip(&boxes).zip(expected).enumerate()
    {
        assert_close(
            &format!("box {index} start inset {start}pt (half the 2pt thickness for auto)"),
            line[0] - run[0],
            start,
        );
        assert_close(
            &format!("box {index} end inset {end}pt"),
            run[1] - line[1],
            end,
        );
    }
}

#[test]
fn a_sliced_box_insets_only_its_first_start_and_last_end() {
    let trace = render(
        "p{width:60pt}u{text-decoration-inset:6pt}",
        "<p><u>aaaa bbbb cccc</u></p>",
    );
    let lines: Vec<&TextLine> = trace.pages[0].iter().collect();
    assert!(lines.len() >= 2, "the box breaks across lines: {lines:?}");
    let paints = decorations(&trace);
    let starts_and_ends: Vec<[f32; 2]> = lines
        .iter()
        .map(|line| {
            let extent = paints
                .iter()
                .find(|paint| {
                    paint.rect.y > line.baseline - line.height
                        && paint.rect.y < line.baseline + line.height
                })
                .map(|paint| [paint.rect.x, paint.rect.x + paint.rect.width])
                .expect("each line is underlined");
            let run = line.runs.first().expect("text on the line");
            [extent[0] - (line.x + run.x), extent[1]]
        })
        .collect();
    assert_close("the first line starts inset", starts_and_ends[0][0], 6.0);
    assert_close(
        "the last line starts at its text",
        starts_and_ends[starts_and_ends.len() - 1][0],
        0.0,
    );
}

#[test]
fn ink_skipping_interrupts_underlines_at_descenders_but_not_line_throughs() {
    let paints = |skip: &str| {
        let trace = render(
            &format!(
                "p{{font-size:40pt;text-decoration:underline line-through;text-decoration-skip-ink:{skip}}}"
            ),
            "<p>xgx</p>",
        );
        let text = line(&trace, "xgx");
        let glyph = text.runs[0].width / 3.0;
        (decorations(&trace), text.x, glyph)
    };
    let (none, _, _) = paints("none");
    assert_eq!(
        extents(&none, DecorationLine::Underline).len(),
        1,
        "`none` draws one continuous underline"
    );
    let (auto, _, _) = paints("auto");
    assert_eq!(
        extents(&auto, DecorationLine::Underline).len(),
        1,
        "`auto` may draw continuous lines, as here"
    );
    let (all, x, glyph) = paints("all");
    let underlines = extents(&all, DecorationLine::Underline);
    assert_eq!(
        underlines.len(),
        2,
        "the g's descender interrupts the underline once: {underlines:?}"
    );
    let gap = [underlines[0][1] - x, underlines[1][0] - x];
    assert!(
        gap[0] > glyph * 0.5
            && gap[0] < glyph * 1.5
            && gap[1] > glyph * 1.5
            && gap[1] < glyph * 2.5,
        "the gap {gap:?} lies around the g, the middle {glyph}pt glyph"
    );
    assert_eq!(
        extents(&all, DecorationLine::LineThrough).len(),
        1,
        "line-throughs never skip ink"
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
