//! Text shadows, emphasis marks and strokes in the layout trace (CSS Text Decoration 3 §3–4,
//! Compatibility §3.4.6), measured against the profile font's `DejaVu Sans` metrics.
use super::common;
use crate::graphics::{Color, DecorationLine, DisplayItem, TextShadowPaint};
use crate::{LayoutTrace, TextLine, render_html};
use common::{PROFILE, local_options};

const TOLERANCE_PT: f32 = 1e-3;
const DEJAVU_UNITS_PER_EM: f32 = 2048.0;
/// `DejaVu Sans` `hhea` ascender and descender.
const DEJAVU_ASCENT: f32 = 1901.0 / DEJAVU_UNITS_PER_EM;
const DEJAVU_DESCENT: f32 = 483.0 / DEJAVU_UNITS_PER_EM;
const SIZE_PT: f32 = 20.0;
const RED: Color = Color {
    rgb: [255, 0, 0],
    alpha: 1.0,
};
const GREEN: Color = Color {
    rgb: [0, 128, 0],
    alpha: 1.0,
};

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}p{{font-size:{SIZE_PT}pt}}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("text effect fixture must render: {error}\n{css}\n{body}"))
    .layout
    .unwrap()
}

fn assert_pt(actual: f32, expected: f32, label: &str) {
    assert!(
        (actual - expected).abs() <= TOLERANCE_PT,
        "{label}: {actual}pt, expected {expected}pt"
    );
}

fn shadows(trace: &LayoutTrace) -> Vec<&TextShadowPaint> {
    trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::TextShadow(shadow) => Some(shadow),
            _ => None,
        })
        .collect()
}

#[test]
fn shadow_layers_paint_bottom_first_beneath_decorations_and_text() {
    let trace = render(
        "p{text-shadow:1pt 2pt red,3pt 4pt 5pt blue;text-decoration:underline}span{text-shadow:none}",
        "<p>ab <span>cd</span></p>",
    );
    let kinds: Vec<String> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::TextShadow(shadow) => Some(format!("shadow {:?}", shadow.offset)),
            DisplayItem::Decoration(decoration) => Some(format!("{:?}", decoration.line)),
            DisplayItem::Text { .. } => Some("text".into()),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "shadow [3.0, 4.0]",
            "shadow [1.0, 2.0]",
            "Underline",
            "text"
        ],
        "the last shadow is the lowest; all shadows precede the decorations and the text"
    );
    let line = &trace.pages[0][0];
    let shadowed = shadows(&trace);
    for shadow in &shadowed {
        let texts: Vec<&str> = shadow.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(texts, ["ab "], "only the paragraph's own run casts shadows");
        let run = &line.runs[0];
        assert_pt(shadow.extent[0], line.x + run.x, "shadowed extent start");
        assert_pt(
            shadow.extent[1],
            line.x + run.x + run.width,
            "shadowed extent end",
        );
        assert_eq!(
            shadow
                .decorations
                .iter()
                .map(|decoration| decoration.line)
                .collect::<Vec<_>>(),
            [DecorationLine::Underline],
            "the paragraph's underline is shadowed with its text"
        );
    }
    assert_eq!(
        shadowed[0].blur, 5.0,
        "the blue layer keeps its blur radius"
    );
    assert_eq!(shadowed[1].blur, 0.0);
}

#[test]
fn inherited_currentcolor_shadow_and_mark_colours_follow_each_element() {
    let trace = render(
        "div{color:green;text-shadow:1pt 1pt;text-emphasis:dot}span{color:red}",
        "<div><p>a <span>b</span></p></div>",
    );
    let colours: Vec<Color> = shadows(&trace).iter().map(|shadow| shadow.color).collect();
    assert_eq!(
        colours,
        [GREEN, RED],
        "a shadow without a colour is currentcolor, resolved by the inheriting span"
    );
    let line = &trace.pages[0][0];
    let marks: Vec<(&str, Color)> = line
        .runs
        .iter()
        .flat_map(|run| {
            run.marks
                .iter()
                .map(move |mark| (run.text.as_str(), mark.color))
        })
        .collect();
    assert_eq!(
        marks,
        [("a ", GREEN), ("b", RED)],
        "the space takes no mark and each mark is its element's colour"
    );
}

fn marked_line(css: &str) -> TextLine {
    render(css, "<p>Hi</p>").pages[0].remove(0)
}

fn marked_text(line: &TextLine) -> Vec<&str> {
    line.runs
        .iter()
        .flat_map(|run| &run.marks)
        .map(|mark| &line.text[mark.source_range.clone()])
        .collect()
}

#[test]
fn emphasis_skip_selects_typographic_units_without_changing_their_advances() {
    const TEXT: &str = "A ,+# B";
    let plain = render("p{white-space:pre}", &format!("<p>{TEXT}</p>"));
    let width = plain.pages[0][0].width;
    for (skip, expected) in [
        ("spaces", vec!["A", ",", "+", "#", "B"]),
        ("punctuation", vec!["A", " ", "+", "#", " ", "B"]),
        ("symbols", vec!["A", " ", ",", " ", "B"]),
        ("spaces punctuation symbols", vec!["A", "B"]),
        ("narrow", vec![]),
    ] {
        let trace = render(
            &format!("p{{white-space:pre;text-emphasis:dot;text-emphasis-skip:{skip}}}"),
            &format!("<p>{TEXT}</p>"),
        );
        let line = &trace.pages[0][0];
        assert_eq!(
            marked_text(line),
            expected,
            "text-emphasis-skip:{skip} on {TEXT:?}"
        );
        assert_pt(
            line.width,
            width,
            "emphasis eligibility must not change base advances",
        );
    }
    let trace = render(
        "@font-face{font-family:Cjk;src:url('../../fixtures/fonts/CJKSpacingTest.otf')}p{font-family:Spike,Cjk;text-emphasis:dot;text-emphasis-skip:narrow}",
        "<p>AＡ漢</p>",
    );
    assert_eq!(
        marked_text(&trace.pages[0][0]),
        ["Ａ", "漢"],
        "narrow skips ASCII while retaining fullwidth and wide character units"
    );
    let trace = render(
        "p{white-space:pre;text-emphasis:dot;text-emphasis-skip:symbols}",
        "<p>  </p>",
    );
    assert_eq!(
        marked_text(&trace.pages[0][0]),
        [" ", " "],
        "spaces receive marks even when no base glyph has visible ink"
    );
}

#[test]
fn emphasis_skip_inheritance_and_initial_affect_the_actual_marks() {
    let trace = render(
        "div{--skip:symbols;text-emphasis:dot;text-emphasis-skip:var(--skip)}.initial{text-emphasis-skip:initial}",
        "<div><p>A,<span>+</span><span class=initial>+,</span>B</p></div>",
    );
    assert_eq!(
        marked_text(&trace.pages[0][0]),
        ["A", ",", "+", "B"],
        "the inherited symbols value marks punctuation, while initial restores spaces/punctuation exclusions"
    );
}

#[test]
fn skipped_emphasis_marks_do_not_inflate_line_leading() {
    let line = marked_line("p{line-height:20pt;text-emphasis:dot;text-emphasis-skip:narrow}");
    assert!(
        marked_text(&line).is_empty(),
        "narrow text has no marks: {:?}",
        marked_text(&line)
    );
    assert_pt(
        line.height,
        SIZE_PT,
        "skipped marks contribute no extra leading",
    );
}

#[test]
fn marks_are_half_size_centred_on_their_characters_and_abut_the_content_area() {
    for (css, under) in [
        ("p{text-emphasis:filled circle}", false),
        (
            "p{text-emphasis:filled circle;text-emphasis-position:under}",
            true,
        ),
    ] {
        let line = marked_line(css);
        let run = &line.runs[0];
        assert_eq!(run.marks.len(), 2, "{css}: one mark per letter");
        let mut pen = run.x;
        for (glyph, mark) in run.glyphs().iter().zip(&run.marks) {
            let advance = glyph.advance(run.font_size);
            assert_eq!(mark.font_size, SIZE_PT / 2.0, "{css}: marks are half size");
            assert_pt(
                mark.x + mark.width / 2.0,
                pen + advance / 2.0,
                &format!("{css}: mark centre over its character"),
            );
            pen += advance;
            let mark_ascent = DEJAVU_ASCENT * mark.font_size;
            let mark_descent = DEJAVU_DESCENT * mark.font_size;
            let expected = if under {
                DEJAVU_DESCENT * SIZE_PT + mark_ascent
            } else {
                -(DEJAVU_ASCENT * SIZE_PT + mark_descent)
            };
            assert_pt(
                mark.baseline_shift,
                expected,
                &format!("{css}: mark baseline below the text baseline"),
            );
        }
    }
    let chinese = render("p{text-emphasis:dot}", "<p lang=zh>Hi</p>").pages[0].remove(0);
    assert!(
        chinese.runs[0].marks[0].baseline_shift > 0.0,
        "auto places marks under Chinese text"
    );
}

#[test]
fn marks_add_leading_as_ruby_does_when_the_line_height_is_too_small() {
    let content = (DEJAVU_ASCENT + DEJAVU_DESCENT) * SIZE_PT;
    let marked = content * 1.5;
    for (line_height, position, inflated) in [
        (20.0, "over", true),
        (20.0, "under", true),
        (40.0, "over", false),
    ] {
        let trace = render(
            &format!(
                "p{{line-height:{line_height}pt;margin:0}}.e{{text-emphasis:dot;text-emphasis-position:{position}}}"
            ),
            "<p>Hi</p><p class=e>Hi</p>",
        );
        let [plain, emphasized] = [&trace.pages[0][0], &trace.pages[0][1]];
        assert_pt(plain.height, line_height, "unmarked line height");
        let extra = if inflated { marked - line_height } else { 0.0 };
        let label = format!("{line_height}pt {position}");
        assert_pt(
            emphasized.height,
            line_height + extra,
            &format!("{label}: line height grows until three marked lines would not overlap"),
        );
        let above = if position == "over" { extra } else { 0.0 };
        assert_pt(
            emphasized.baseline - plain.baseline,
            line_height + above,
            &format!("{label}: the extra leading goes on the marks' side"),
        );
    }
}

#[test]
fn strokes_and_fills_reach_the_glyph_runs() {
    let line = marked_line("p{color:blue;-webkit-text-fill-color:red;-webkit-text-stroke:2pt}");
    let run = &line.runs[0];
    assert_eq!(run.color, RED, "the fill colour replaces color for glyphs");
    let stroke = run.stroke.expect("a 2pt stroke");
    assert_eq!(stroke.width, 2.0);
    assert_eq!(
        stroke.color,
        Color {
            rgb: [0, 0, 255],
            alpha: 1.0
        },
        "the stroke colour defaults to currentcolor"
    );
    assert!(
        marked_line("p{-webkit-text-stroke:0 red}").runs[0]
            .stroke
            .is_none(),
        "a zero width paints no stroke"
    );
}

#[test]
fn error_indications_paint_wavy_underlines_in_their_ua_colours() {
    for (line, color) in [("spelling-error", RED), ("grammar-error", GREEN)] {
        let trace = render(
            &format!(
                "p{{text-decoration:{line} blue dotted;text-decoration-thickness:3pt}}span{{text-decoration-line:underline;text-decoration-color:blue}}"
            ),
            "<p>Wrod <span>here</span></p>",
        );
        let decorations: Vec<_> = trace.display_lists[0]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Decoration(paint) => Some(paint),
                _ => None,
            })
            .collect();
        let errors: Vec<_> = decorations
            .iter()
            .filter(|paint| paint.color == color)
            .collect();
        assert!(
            !errors.is_empty()
                && errors
                    .iter()
                    .all(|paint| paint.line == DecorationLine::Underline
                        && paint.style == crate::graphics::DecorationStyle::Wavy),
            "{line} must paint a wavy underline in its UA colour, disregarding the authored \
             colour and style (CSS Text Decoration 4 §2.1): {decorations:?}"
        );
        assert_pt(
            errors[0].rect.height,
            3.0,
            &format!("{line} honours the authored thickness"),
        );
        assert!(
            decorations
                .iter()
                .any(|paint| paint.color.rgb == [0, 0, 255]
                    && paint.style == crate::graphics::DecorationStyle::Solid),
            "{line} leaves a descendant authored underline its own colour and style: {decorations:?}"
        );
    }
}
