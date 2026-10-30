//! CSS Ruby 1 layout through the layout trace: column sizing and `ruby-align` (§§3.1, 4.3),
//! overhang (§5.1), stacking and line spacing (§§3.1.2, 3.6), merging (§4.2) and breaking
//! (§3.4). Expected geometry comes from the CJK spacing fixture's own metrics: every glyph it
//! has for these tests advances one em, and Latin `a` its recorded advance.
mod common;

use common::{assert_labelled_pt, render_trace};
use ferri_pdf::{LayoutTrace, TextLine};

const SIZE_PT: f32 = 20.0;
/// The UA style sheet sets annotations at half size (CSS Ruby 1 §A.1).
const ANNOTATION_PT: f32 = SIZE_PT / 2.0;
const LINE_HEIGHT_PT: f32 = 40.0;
const FONT: &[u8] = include_bytes!("fixtures/fonts/CJKSpacingTest.otf");
const FACE: &str = "@font-face{font-family:J;src:url('../../fixtures/fonts/CJKSpacingTest.otf')}";

/// The fixture's `[ascent, descent]` per em, from its `hhea` table.
fn extent() -> [f32; 2] {
    let face = ttf_parser::Face::parse(FONT, 0).expect("the fixture parses");
    let units = f32::from(face.units_per_em());
    [
        f32::from(face.ascender()) / units,
        -f32::from(face.descender()) / units,
    ]
}

fn latin_advance(ch: char) -> f32 {
    let face = ttf_parser::Face::parse(FONT, 0).expect("the fixture parses");
    let glyph = face.glyph_index(ch).expect("the fixture has the letter");
    f32::from(face.glyph_hor_advance(glyph).unwrap()) / f32::from(face.units_per_em())
}

fn render(css: &str, body: &str) -> LayoutTrace {
    render_trace(&format!(
        "<!doctype html><style>{FACE}@page{{size:400pt 400pt;margin:20pt}}\
         body{{margin:0;font:{SIZE_PT}pt/{LINE_HEIGHT_PT}pt J}}p{{margin:0}}{css}</style>{body}"
    ))
}

fn line<'a>(trace: &'a LayoutTrace, text: &str) -> &'a TextLine {
    let lines: Vec<&TextLine> = trace.pages.iter().flatten().collect();
    lines
        .iter()
        .find(|line| line.text == text)
        .copied()
        .unwrap_or_else(|| {
            panic!(
                "no line is {text:?}: {:?}",
                lines.iter().map(|line| &line.text).collect::<Vec<_>>()
            )
        })
}

/// Each glyph's character and pen position from the page's left content edge.
fn glyphs(line: &TextLine) -> Vec<(String, f32)> {
    let mut glyphs = Vec::new();
    for run in &line.runs {
        let mut pen = line.x + run.x;
        for glyph in run.glyphs() {
            glyphs.push((run.text[glyph.text_range.clone()].to_owned(), pen));
            pen += glyph.advance(run.font_size);
        }
    }
    glyphs
}

/// Asserts the glyphs of `line` and their positions from `origin`, a page x.
fn assert_glyphs(label: &str, line: &TextLine, origin: f32, expected: &[(&str, f32)]) {
    let actual = glyphs(line);
    assert_eq!(
        actual
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>(),
        expected.iter().map(|(text, _)| *text).collect::<Vec<_>>(),
        "{label}: glyphs {actual:?}"
    );
    for ((text, x), (_, expected)) in actual.iter().zip(expected) {
        assert_labelled_pt(&format!("{label}: {text} at"), x - origin, *expected);
    }
}

/// The leading the line needs above its base container so that identical lines would not
/// overlap, where `levels` annotation levels lie over it (§3.6).
fn over_leading(levels: f32) -> f32 {
    let [ascent, descent] = extent();
    ((ascent + descent) * (SIZE_PT + levels * ANNOTATION_PT) - LINE_HEIGHT_PT).max(0.0)
}

#[test]
fn annotation_is_centred_on_the_base_container_with_the_leading_it_needs() {
    let trace = render("", "<p>あ<ruby>漢<rt>あ</rt></ruby>あ</p>");
    let base = line(&trace, "あ漢あ");
    let annotation = line(&trace, "あ");
    let [ascent, descent] = extent();
    assert_labelled_pt(
        "annotation start",
        annotation.x - base.x,
        SIZE_PT + (SIZE_PT - ANNOTATION_PT) / 2.0,
    );
    assert_labelled_pt(
        "annotation baseline above the base's",
        base.baseline - annotation.baseline,
        ascent * SIZE_PT + descent * ANNOTATION_PT,
    );
    assert_labelled_pt(
        "the line grows by the leading over its base container",
        base.height,
        LINE_HEIGHT_PT + over_leading(1.0),
    );
}

#[test]
fn space_around_spreads_cjk_annotations_and_centres_latin_ones() {
    let trace = render("", "<p><ruby>漢字<rt>あア</rt></ruby></p>");
    let base = line(&trace, "漢字").x;
    // `text-justify: ruby` has one opportunity between the kana: half the extra space,
    // and half of another half at either end.
    let extra = 2.0 * SIZE_PT - 2.0 * ANNOTATION_PT;
    assert_glyphs(
        "space-around kana",
        line(&trace, "あア"),
        base,
        &[
            ("あ", extra / 4.0),
            ("ア", extra / 4.0 + ANNOTATION_PT + extra / 2.0),
        ],
    );
    let trace = render("", "<p><ruby>漢字<rt>ab</rt></ruby></p>");
    let width = (latin_advance('a') + latin_advance('b')) * ANNOTATION_PT;
    assert_labelled_pt(
        "Latin letters have no ruby justification opportunity, so they centre",
        line(&trace, "ab").x - line(&trace, "漢字").x,
        (2.0 * SIZE_PT - width) / 2.0,
    );
    for (align, expected) in [
        ("start", [0.0, ANNOTATION_PT]),
        ("center", [extra / 2.0, extra / 2.0 + ANNOTATION_PT]),
        ("space-between", [0.0, 2.0 * SIZE_PT - ANNOTATION_PT]),
    ] {
        let trace = render(
            &format!("rt{{ruby-align:{align}}}"),
            "<p><ruby>漢字<rt>あア</rt></ruby></p>",
        );
        assert_glyphs(
            align,
            line(&trace, "あア"),
            line(&trace, "漢字").x,
            &[("あ", expected[0]), ("ア", expected[1])],
        );
    }
}

#[test]
fn wider_annotations_widen_their_column_and_overhang_only_adjacent_kana() {
    let annotation = 3.0 * ANNOTATION_PT;
    let extra = annotation - SIZE_PT;
    // Half an ideograph in the annotation's font may overhang each side (§5.1).
    let overhang = (ANNOTATION_PT / 2.0).min(extra / 2.0);
    let trace = render("", "<p>あ<ruby>漢<rt>あああ</rt></ruby>あ</p>");
    let base = line(&trace, "あ漢あ");
    assert_glyphs(
        "overhanging kana",
        base,
        base.x,
        &[
            ("あ", 0.0),
            ("漢", SIZE_PT + extra / 2.0 - overhang),
            ("あ", 2.0 * SIZE_PT + extra - 2.0 * overhang),
        ],
    );
    assert_labelled_pt(
        "the annotation stays centred over its base",
        line(&trace, "あああ").x - line(&trace, "あ漢あ").x,
        SIZE_PT - overhang,
    );
    for (label, css, body) in [
        (
            "beside ideographs",
            "",
            "<p>字<ruby>漢<rt>あああ</rt></ruby>字</p>",
        ),
        (
            "ruby-overhang: none",
            "ruby{ruby-overhang:none}",
            "<p>あ<ruby>漢<rt>あああ</rt></ruby>あ</p>",
        ),
    ] {
        let trace = render(css, body);
        let base = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text.chars().count() == 3 && line.text.contains('漢'))
            .unwrap();
        let positions: Vec<f32> = glyphs(base).iter().map(|(_, x)| x - base.x).collect();
        assert_eq!(positions.len(), 3, "{label}");
        assert_labelled_pt(label, positions[1], SIZE_PT + extra / 2.0);
        assert_labelled_pt(label, positions[2], 2.0 * SIZE_PT + extra);
    }
}

#[test]
fn levels_stack_outward_over_and_under_the_base_container() {
    let trace = render("", "<p><ruby>漢字<rt>あア</rt><rtc>ab</rtc></ruby></p>");
    let [ascent, descent] = extent();
    let base = line(&trace, "漢字");
    assert_labelled_pt(
        "the first level lies over the base container",
        base.baseline - line(&trace, "あア").baseline,
        (ascent * SIZE_PT) + descent * ANNOTATION_PT,
    );
    assert_labelled_pt(
        "the second level alternates under it",
        line(&trace, "ab").baseline - base.baseline,
        descent * SIZE_PT + ascent * ANNOTATION_PT,
    );
    let ruby = (ascent + descent) * (SIZE_PT + 2.0 * ANNOTATION_PT);
    assert_labelled_pt(
        "identical lines would not overlap: the line is the ruby's height",
        base.height,
        ruby,
    );
    let trace = render(
        "rtc{ruby-position:over}",
        "<p><ruby>漢<rtc>あ</rtc><rtc>ア</rtc></ruby></p>",
    );
    assert_labelled_pt(
        "a second level over stacks above the first",
        line(&trace, "あ").baseline - line(&trace, "ア").baseline,
        (ascent + descent) * ANNOTATION_PT,
    );
}

#[test]
fn nested_ruby_annotations_lie_inside_the_outer_base_container() {
    let trace = render(
        "",
        "<p><ruby><rb><ruby>漢<rt>あ</rt></ruby></rb><rt>ア</rt></ruby></p>",
    );
    let [ascent, descent] = extent();
    assert_labelled_pt(
        "the outer annotation stacks over the inner one",
        line(&trace, "あ").baseline - line(&trace, "ア").baseline,
        (ascent + descent) * ANNOTATION_PT,
    );
}

#[test]
fn merging_shares_the_annotation_space_of_a_segment() {
    let body = "<p><ruby><rb>漢</rb><rb>字</rb><rt>あああ</rt><rt>ア</rt></ruby></p>";
    let separate = render("", body);
    // Separate columns: the first is as wide as its annotation; the base centres in it.
    let extra = 3.0 * ANNOTATION_PT - SIZE_PT;
    let base = line(&separate, "漢字");
    assert_glyphs(
        "separate",
        base,
        base.x,
        &[("漢", extra / 2.0), ("字", SIZE_PT + extra)],
    );
    for value in ["merge", "auto"] {
        let merged = render(&format!("ruby{{ruby-merge:{value}}}"), body);
        let base = line(&merged, "漢字");
        assert_glyphs(value, base, base.x, &[("漢", 0.0), ("字", SIZE_PT)]);
        assert_labelled_pt(
            &format!("{value}: one run of four kana fills both bases"),
            line(&merged, "ア").x - line(&merged, "あああ").x,
            3.0 * ANNOTATION_PT,
        );
    }
    let fitting = render(
        "ruby{ruby-merge:auto}",
        "<p><ruby><rb>漢</rb><rb>字</rb><rt>あ</rt><rt>ア</rt></ruby></p>",
    );
    assert_labelled_pt(
        "auto keeps annotations that fit separate",
        line(&fitting, "ア").x - line(&fitting, "漢字").x,
        SIZE_PT + (SIZE_PT - ANNOTATION_PT) / 2.0,
    );
}

#[test]
fn lines_break_between_bases_but_not_within_a_spanning_annotation() {
    let narrow = "p{width:50pt}";
    let trace = render(
        narrow,
        "<p><ruby><rb>漢</rb><rb>字</rb><rb>水</rb><rt>あ</rt><rt>ア</rt><rt>あ</rt></ruby></p>",
    );
    let first = line(&trace, "漢字");
    let second = line(&trace, "水");
    assert!(
        second.baseline > first.baseline,
        "the ruby breaks between its bases"
    );
    let annotations: Vec<f32> = trace
        .pages
        .iter()
        .flatten()
        .filter(|line| line.text == "あ")
        .map(|line| line.baseline)
        .collect();
    assert_eq!(annotations.len(), 2);
    assert!(
        annotations.iter().any(|y| *y < first.baseline)
            && annotations.iter().any(|y| *y > first.baseline),
        "each annotation stays with its base: {annotations:?}"
    );
    let trace = render(
        narrow,
        "<p><ruby><rb>漢</rb><rb>字</rb><rb>水</rb><rtc>あアあ</rtc></ruby></p>",
    );
    line(&trace, "漢字水");
}

#[test]
fn empty_bases_and_right_to_left_containers_keep_their_columns() {
    let trace = render(
        "",
        "<p><ruby><rb>漢</rb><rb></rb><rb>字</rb><rt>あ</rt><rt></rt><rt>ア</rt></ruby></p>",
    );
    let base = line(&trace, "漢字");
    assert_labelled_pt(
        "an empty base between two takes no width",
        line(&trace, "ア").x - base.x,
        SIZE_PT + (SIZE_PT - ANNOTATION_PT) / 2.0,
    );
    let trace = render(
        "rt{ruby-align:start}",
        "<p dir=rtl><ruby>漢字<rt>あ</rt></ruby></p>",
    );
    let base = line(&trace, "漢字");
    assert_labelled_pt(
        "start alignment is the right edge in a right-to-left container",
        line(&trace, "あ").x + ANNOTATION_PT,
        base.x + base.width,
    );
}

#[test]
fn table_parts_in_ruby_boxes_share_one_anonymous_inline_table() {
    let trace = render(
        ".c{display:table-cell}rt .c{font-size:inherit}",
        "<p><ruby>漢<span class=c>字</span> <span class=c>水</span><rt>あ<span class=c>ア</span></rt></ruby></p>",
    );
    let [first, second] = ["字", "水"].map(|text| line(&trace, text));
    assert_labelled_pt(
        "consecutive cells share a row, the white space between them removed",
        second.x - first.x,
        SIZE_PT,
    );
    assert_labelled_pt(
        "the cells' inline table sits on the base line",
        first.baseline,
        second.baseline,
    );
    let base = line(&trace, "漢\u{fffc}");
    assert_labelled_pt(
        "the inline table follows the base text in the ruby base",
        first.x,
        base.x + SIZE_PT,
    );
    let annotation = line(&trace, "あ\u{fffc}");
    let cell = line(&trace, "ア");
    assert_labelled_pt(
        "an annotation's table part follows its text on the annotation's line",
        cell.x,
        annotation.x + ANNOTATION_PT,
    );
    assert!(
        cell.baseline < base.baseline,
        "the annotation and its table lie over the base: {} vs {}",
        cell.baseline,
        base.baseline
    );
}
