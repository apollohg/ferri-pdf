//! CSS Text 4 §8.4 `text-autospace` and §8.5 `text-spacing-trim` through layout. Expected
//! advances come from the independently shaped samples recorded with the CJK spacing fixtures.
mod common;

use common::{assert_labelled_pt, painted_box, render_trace};
use ferri_pdf::{DecorationLine, DisplayItem, LayoutTrace, TextLine};

const SIZE_PT: f32 = 10.0;
const HALT: &str = "CJKSpacingTest.otf";
const NO_HALT: &str = "CJKSpacingTest-NoHalt.otf";
const FACES: &str = "@font-face{font-family:Halt;src:url('../../fixtures/fonts/CJKSpacingTest.otf')}\
                     @font-face{font-family:NoHalt;src:url('../../fixtures/fonts/CJKSpacingTest-NoHalt.otf')}";
const RED: [u8; 3] = [255, 0, 0];
const IDEOGRAPHIC_SPACE: char = '\u{3000}';

fn reference() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/fonts/CJKSpacingTest-source.json")).unwrap()
}

/// The shaped `[offset, advance]` of `ch` in `face` at the test size, with `halt` or not.
fn shaped(face: &str, ch: char, halt: bool) -> [f32; 2] {
    let reference = reference();
    let face = &reference["faces"][face];
    let units = face["units_per_em"].as_f64().unwrap() as f32;
    let sample = &face["samples"][ch.to_string()][if halt { "halt" } else { "default" }][0];
    ["offset", "advance"].map(|key| sample[key].as_f64().unwrap() as f32 * SIZE_PT / units)
}

fn advance(ch: char) -> f32 {
    shaped(HALT, ch, false)[1]
}

/// §8.4.1: 1/8 of the advance of 水 in the fixture.
fn inter_script() -> f32 {
    advance('水') / 8.0
}

fn render(css: &str, body: &str) -> LayoutTrace {
    render_trace(&format!(
        "<!doctype html><style>{FACES}@page{{size:400pt 600pt;margin:20pt}}\
         body{{margin:0;font:{SIZE_PT}pt/20pt Halt}}p{{margin:0}}{css}</style>{body}"
    ))
}

fn lines(trace: &LayoutTrace) -> Vec<&TextLine> {
    trace.pages.iter().flatten().collect()
}

fn line<'a>(trace: &'a LayoutTrace, text: &str) -> &'a TextLine {
    lines(trace)
        .into_iter()
        .find(|line| line.text == text)
        .unwrap_or_else(|| {
            panic!(
                "no line is {text:?}: {:?}",
                lines(trace)
                    .iter()
                    .map(|line| &line.text)
                    .collect::<Vec<_>>()
            )
        })
}

/// A painted glyph: its character, pen and drawn origins and used advance, from the line's
/// left edge.
#[derive(Debug)]
struct Placed {
    ch: char,
    pen: f32,
    drawn: f32,
    advance: f32,
}

fn placed(line: &TextLine) -> Vec<Placed> {
    let mut glyphs = Vec::new();
    for run in &line.runs {
        let mut pen = run.x;
        for glyph in run.glyphs() {
            let advance = glyph.advance(run.font_size);
            glyphs.push(Placed {
                ch: run.text[glyph.text_range.clone()].chars().next().unwrap(),
                pen,
                drawn: pen + glyph.x_offset * run.font_size,
                advance,
            });
            pen += advance;
        }
    }
    glyphs.sort_by(|a, b| a.pen.total_cmp(&b.pen));
    glyphs
}

/// The space between consecutive glyphs beyond the natural advance of the left one.
fn gaps(line: &TextLine) -> Vec<f32> {
    placed(line)
        .windows(2)
        .map(|pair| pair[1].pen - pair[0].pen - advance(pair[0].ch))
        .collect()
}

fn assert_gaps(label: &str, line: &TextLine, expected: &[f32]) {
    let actual = gaps(line);
    assert_eq!(
        actual.len(),
        expected.len(),
        "{label}: glyphs {:?}",
        placed(line)
    );
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert_labelled_pt(
            &format!(
                "{label}: gap {index} of {:?} (gaps {:?})",
                line.text,
                gaps(line)
            ),
            *actual,
            *expected,
        );
    }
}

#[test]
fn inter_script_spacing_follows_the_classes_each_value_names() {
    #[derive(Clone, Copy, PartialEq)]
    enum Pair {
        Alpha,
        Numeric,
        Neither,
    }
    // Ideographs include kana; fullwidth letters and digits, punctuation and symbols are
    // not letters or numerals.
    let pairs = [
        ("漢a", Pair::Alpha),
        ("a漢", Pair::Alpha),
        ("あB", Pair::Alpha),
        ("Bア", Pair::Alpha),
        ("ア1", Pair::Numeric),
        ("1漢", Pair::Numeric),
        ("漢Ａ", Pair::Neither),
        ("漢１", Pair::Neither),
        ("漢(", Pair::Neither),
        ("♥a", Pair::Neither),
        ("ab", Pair::Neither),
        ("漢字", Pair::Neither),
    ];
    for (value, alpha, numeric) in [
        ("normal", true, true),
        ("auto", true, true),
        ("ideograph-alpha", true, false),
        ("ideograph-numeric", false, true),
        ("ideograph-numeric ideograph-alpha insert", true, true),
        ("punctuation", false, false),
        ("replace", false, false),
        ("no-autospace", false, false),
    ] {
        let body: String = pairs
            .iter()
            .map(|(text, _)| format!("<p>{text}</p>"))
            .collect();
        let trace = render(&format!("p{{text-autospace:{value}}}"), &body);
        for (text, pair) in pairs {
            let spaced = (pair == Pair::Alpha && alpha) || (pair == Pair::Numeric && numeric);
            assert_gaps(
                &format!("text-autospace:{value}"),
                line(&trace, text),
                &[if spaced { inter_script() } else { 0.0 }],
            );
        }
    }
}

#[test]
fn existing_spaces_suppress_spacing_unless_replace_removes_a_single_space() {
    let space = advance(' ');
    let trace = render(
        "",
        "<p>漢 a</p>\
         <p style='text-autospace:ideograph-alpha replace'>漢 b</p>\
         <p style='text-autospace:ideograph-alpha replace'>1 漢</p>\
         <p style='text-autospace:ideograph-alpha replace;white-space:pre'>漢  c</p>\
         <p style='text-autospace:ideograph-alpha replace'>漢\u{a0}x</p>",
    );
    assert_gaps("insert keeps the space", line(&trace, "漢 a"), &[0.0, 0.0]);
    let replaced = line(&trace, "漢 b");
    assert_gaps(
        "replace removes the space and inserts the spacing",
        replaced,
        &[inter_script(), -space],
    );
    assert_eq!(
        replaced.text, "漢 b",
        "replaced spaces remain in the content"
    );
    assert_gaps(
        "replace without ideograph-numeric keeps the space before a numeral",
        line(&trace, "1 漢"),
        &[0.0, 0.0],
    );
    assert_gaps(
        "two preserved spaces are not one space to replace",
        line(&trace, "漢  c"),
        &[0.0, 0.0, 0.0],
    );
    assert_gaps(
        "other space characters suppress the spacing",
        line(&trace, "漢\u{a0}x"),
        &[0.0, 0.0],
    );
}

#[test]
fn justification_does_not_expand_replaced_spaces() {
    let width = 80.0;
    let trace = render(
        &format!(
            "p{{width:{width}pt;text-align:justify;text-align-last:justify;\
             text-autospace:ideograph-alpha replace}}"
        ),
        "<p>漢 a b</p>",
    );
    let glyphs = placed(line(&trace, "漢 a b"));
    let [ideograph, replaced, a, word_space, b] = &glyphs[..] else {
        panic!("five glyphs expected: {glyphs:?}");
    };
    assert_labelled_pt("removed space advance", replaced.advance, 0.0);
    assert_labelled_pt(
        "the removed space sits where the spacing ends",
        a.pen - ideograph.pen,
        advance('漢') + inter_script(),
    );
    assert_labelled_pt(
        "the word space alone absorbs the justification",
        word_space.advance,
        width - advance('漢') - inter_script() - advance('a') - advance('b'),
    );
    assert_labelled_pt("line end", b.pen + b.advance, width);
}

#[test]
fn the_innermost_element_containing_a_boundary_sets_and_renders_its_spacing() {
    let trace = render(
        "",
        "<p style='font-size:20pt'><span style='font-size:10pt'>漢</span>a</p>\
         <p style='text-autospace:no-autospace'><span style='text-autospace:normal'>漢</span>b</p>\
         <p style='text-autospace:no-autospace'><span style='text-autospace:normal'>漢c</span></p>\
         <p style='text-decoration:underline'><span style='background:rgb(255,0,0)'>漢</span>x</p>\
         <p>漢<span style='margin-left:1pt'>y</span></p>",
    );
    let larger = placed(line(&trace, "漢a"));
    assert_labelled_pt(
        "the paragraph's 20pt font sizes the spacing around its child",
        larger[1].pen - larger[0].pen,
        advance('漢') + 2.0 * inter_script(),
    );
    assert_gaps(
        "the paragraph's value governs a boundary its child does not contain",
        line(&trace, "漢b"),
        &[0.0],
    );
    assert_gaps(
        "the child's value governs a boundary inside it",
        line(&trace, "漢c"),
        &[inter_script()],
    );
    let underlined = line(&trace, "漢x");
    assert_gaps("nested boundary", underlined, &[inter_script()]);
    let red = painted_box(&trace, RED);
    assert_labelled_pt(
        "the span's background excludes the spacing outside it",
        red.width,
        advance('漢'),
    );
    let underlines: Vec<_> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Decoration(paint) if paint.line == DecorationLine::Underline => {
                Some(paint.rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        underlines.len(),
        1,
        "the paragraph's underline continues across the spacing it renders: {underlines:?}"
    );
    assert_labelled_pt(
        "underline width",
        underlines[0].width,
        advance('漢') + inter_script() + advance('x'),
    );
    assert_gaps(
        "an intervening margin separates the characters",
        line(&trace, "漢y"),
        &[1.0],
    );
}

#[test]
fn spacing_is_inserted_between_visually_adjacent_characters_after_bidi_reordering() {
    // §8.4.2: the visual order is 漢 ♥ a 字, so only a and 字 adjoin across classes.
    let trace = render("", "<p>漢<bdo dir=rtl>a♥</bdo>字</p>");
    let glyphs = placed(line(&trace, "漢a♥字"));
    assert_eq!(
        glyphs.iter().map(|glyph| glyph.ch).collect::<String>(),
        "漢♥a字",
        "visual order"
    );
    assert_gaps(
        "visual adjacency",
        line(&trace, "漢a♥字"),
        &[0.0, 0.0, inter_script()],
    );
}

#[test]
fn french_punctuation_spacing_inserts_narrow_and_ordinary_no_break_spaces() {
    let narrow = advance(' ') / 2.0;
    let no_break = advance('\u{a0}');
    let removed = -advance(' ');
    let trace = render(
        "p{text-autospace:punctuation}",
        "<p lang=fr-CA>«a» b; c! x? y:</p><p lang=en>«z» A; B! a? b:</p>\
         <p lang=fr style='text-autospace:punctuation replace'>« c » x ; y</p>\
         <p lang=fr>« z » A ; B</p>",
    );
    assert_gaps(
        "French",
        line(&trace, "«a» b; c! x? y:"),
        &[
            narrow, narrow, 0.0, 0.0, narrow, 0.0, 0.0, narrow, 0.0, 0.0, narrow, 0.0, 0.0,
            no_break,
        ],
    );
    assert_gaps(
        "other languages",
        line(&trace, "«z» A; B! a? b:"),
        &[0.0; 14],
    );
    assert_gaps(
        "replace swaps each U+0020 for the required space",
        line(&trace, "« c » x ; y"),
        &[
            narrow, removed, narrow, removed, 0.0, 0.0, narrow, removed, 0.0, 0.0,
        ],
    );
    assert_gaps(
        "insert leaves existing spaces",
        line(&trace, "« z » A ; B"),
        &[0.0; 10],
    );
}

/// §8.5.2 classes of the trimmed punctuation.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Opening,
    Closing,
    MiddleDot,
}

/// The `[offset, advance]` of `ch` set half-width at `size`: the `halt` adjustment of the
/// face that has it, else the blank half of the ideographic advance, which a middle dot
/// loses a quarter of from each side.
fn half_width(face: &str, ch: char, kind: Kind, size: f32) -> [f32; 2] {
    let scale = size / SIZE_PT;
    let [offset, advance] = if face == HALT {
        shaped(HALT, ch, true)
    } else {
        let full = shaped(NO_HALT, '水', false)[1];
        match kind {
            Kind::Opening => [-full / 2.0, full / 2.0],
            Kind::Closing => [0.0, full / 2.0],
            Kind::MiddleDot => [-full / 4.0, full / 2.0],
        }
    };
    [offset * scale, advance * scale]
}

/// Asserts each glyph's drawn offset and advance: `trimmed` glyphs are half-width and the
/// others keep their natural `[0, advance]`.
fn assert_set(label: &str, face: &str, line: &TextLine, trimmed: &[(usize, Kind)]) {
    for (index, glyph) in placed(line).iter().enumerate() {
        let [offset, advance] = trimmed
            .iter()
            .find(|(position, _)| *position == index)
            .map_or_else(
                || [0.0, shaped(face, glyph.ch, false)[1]],
                |(_, kind)| half_width(face, glyph.ch, *kind, SIZE_PT),
            );
        let label = format!(
            "{label} ({face}): glyph {index} {:?} of {:?}",
            glyph.ch, line.text
        );
        assert_labelled_pt(&format!("{label} offset"), glyph.drawn - glyph.pen, offset);
        assert_labelled_pt(&format!("{label} advance"), glyph.advance, advance);
    }
}

#[test]
fn adjacent_fullwidth_punctuation_collapses_with_or_without_halt() {
    use Kind::{Closing, Opening};
    let space = IDEOGRAPHIC_SPACE;
    let cases: Vec<(String, Vec<(usize, Kind)>)> = vec![
        ("水〔（水".into(), vec![(2, Opening)]),
        ("水・（水".into(), vec![(2, Opening)]),
        ("水〕（水".into(), vec![(2, Opening)]),
        (format!("水{space}（水"), vec![(2, Opening)]),
        ("水）〕水".into(), vec![(1, Closing)]),
        ("水）・水".into(), vec![(1, Closing)]),
        (format!("水）{space}水"), vec![(1, Closing)]),
        ("水(（水".into(), vec![(2, Opening)]),
        ("水）)水".into(), vec![(1, Closing)]),
        ("水漢（水".into(), vec![]),
        ("水）漢水".into(), vec![]),
        // Fullwidth dots drawn in the left half are closing punctuation; centred colons are
        // middle dots, which collapse their neighbours without being trimmed.
        ("水、」水".into(), vec![(1, Closing)]),
        ("水：（水".into(), vec![(2, Opening)]),
        ("水）：水".into(), vec![(1, Closing)]),
    ];
    let body: String = cases
        .iter()
        .map(|(text, _)| format!("<p>{text}</p>"))
        .collect();
    for (face, family) in [(HALT, "Halt"), (NO_HALT, "NoHalt")] {
        let trace = render(&format!("p{{font-family:{family}}}"), &body);
        let spaced = render(
            &format!("p{{font-family:{family};text-spacing-trim:space-all}}"),
            &body,
        );
        for (text, trimmed) in &cases {
            assert_set("normal", face, line(&trace, text), trimmed);
            assert_set("space-all", face, line(&spaced, text), &[]);
        }
    }
}

#[test]
fn collapsing_compares_the_font_sizes_of_closing_and_opening_punctuation() {
    const SMALL_PT: f32 = 5.0;
    let trace = render(
        "",
        &format!(
            "<p>水<span style='font-size:{SMALL_PT}pt'>）</span>（水</p>\
             <p>水）<span style='font-size:{SMALL_PT}pt'>（</span>水</p>"
        ),
    );
    let lines = lines(&trace);
    let smaller_closing = placed(lines[0]);
    let [_, closing, opening, _] = &smaller_closing[..] else {
        panic!("four glyphs: {smaller_closing:?}");
    };
    assert_labelled_pt(
        "closing punctuation before larger opening punctuation is half-width",
        closing.advance,
        half_width(HALT, '）', Kind::Closing, SMALL_PT)[1],
    );
    assert_labelled_pt(
        "opening punctuation after smaller closing punctuation stays full-width",
        opening.advance,
        advance('（'),
    );
    let smaller_opening = placed(lines[1]);
    let [_, closing, opening, _] = &smaller_opening[..] else {
        panic!("four glyphs: {smaller_opening:?}");
    };
    assert_labelled_pt(
        "closing punctuation before smaller opening punctuation stays full-width",
        closing.advance,
        advance('）'),
    );
    assert_labelled_pt(
        "opening punctuation after equal or larger closing punctuation is half-width",
        opening.advance,
        half_width(HALT, '（', Kind::Opening, SMALL_PT)[1],
    );
}

#[test]
fn trim_all_sets_every_fullwidth_punctuation_half_width_but_not_proportional_glyphs() {
    use Kind::{Closing, MiddleDot, Opening};
    let space = IDEOGRAPHIC_SPACE;
    let text = format!("「水」水・水：水、水{space}水");
    for (face, family) in [(HALT, "Halt"), (NO_HALT, "NoHalt")] {
        let trace = render(
            &format!("p{{font-family:{family};text-spacing-trim:trim-all}}"),
            &format!("<p>{text}</p><p>水‘水’水“水”水</p>"),
        );
        assert_set(
            "trim-all",
            face,
            line(&trace, &text),
            &[
                (0, Opening),
                (2, Closing),
                (4, MiddleDot),
                (6, MiddleDot),
                (8, Closing),
            ],
        );
        assert_set(
            "proportional quotes",
            face,
            line(&trace, "水‘水’水“水”水"),
            &[],
        );
    }
}

#[test]
fn line_start_opening_punctuation_follows_each_value() {
    // The first line, a wrapped line, a line after a forced break and another wrapped line.
    let first_lines = [true, false, true, false];
    for (value, trims) in [
        ("normal", [false; 4]),
        ("space-all", [false; 4]),
        ("trim-start", [true; 4]),
        ("trim-both", [true; 4]),
        ("trim-all", [true; 4]),
        ("space-first", first_lines.map(|first| !first)),
        ("auto", first_lines.map(|first| !first)),
    ] {
        let trace = render(
            &format!(
                "p{{width:{}pt;text-spacing-trim:{value}}}",
                2.0 * advance('水')
            ),
            "<p>「水「水<br>「水「水</p>",
        );
        let lines = lines(&trace);
        let texts: Vec<_> = lines.iter().map(|line| &line.text).collect();
        assert_eq!(lines.len(), trims.len(), "{value}: {texts:?}");
        for (index, (line, trimmed)) in lines.iter().zip(trims).enumerate() {
            let opening = &placed(line)[0];
            let [offset, advance] = if trimmed {
                half_width(HALT, '「', Kind::Opening, SIZE_PT)
            } else {
                [0.0, advance('「')]
            };
            let label = format!("{value}: line {index} {:?}", line.text);
            assert_labelled_pt(
                &format!("{label} offset"),
                opening.drawn - opening.pen,
                offset,
            );
            assert_labelled_pt(&format!("{label} advance"), opening.advance, advance);
        }
    }
}

#[test]
fn line_end_closing_punctuation_is_trimmed_always_or_only_where_it_would_not_fit() {
    let ideograph = advance('水');
    let trimmed = half_width(HALT, '」', Kind::Closing, SIZE_PT)[1];
    let tight = 3.0 * ideograph + trimmed;
    let roomy = 4.5 * ideograph;
    for (value, trims_on_overflow, always) in [
        ("normal", true, false),
        ("space-first", true, false),
        ("trim-start", true, false),
        ("trim-both", true, true),
        ("auto", true, true),
        ("space-all", false, false),
    ] {
        for (width, overflows) in [(tight, true), (roomy, false)] {
            let trace = render(
                &format!("p{{width:{width}pt;text-spacing-trim:{value}}}"),
                "<p>水水水」水</p>",
            );
            let first = &trace.pages[0][0];
            let label = format!("{value} at {width}pt");
            if overflows && !trims_on_overflow {
                assert_eq!(
                    first.text, "水水",
                    "{label}: full-width closing punctuation cannot end the first line"
                );
                continue;
            }
            assert_eq!(first.text, "水水水」", "{label}");
            let closing = placed(first).pop().unwrap();
            assert_labelled_pt(
                &format!("{label}: closing advance"),
                closing.advance,
                if always || overflows {
                    trimmed
                } else {
                    advance('」')
                },
            );
            assert_labelled_pt(
                &format!("{label}: line width"),
                first.width,
                3.0 * ideograph + closing.advance,
            );
        }
    }
}

#[test]
fn min_content_trims_line_end_punctuation_that_max_content_keeps() {
    let ideograph = advance('水');
    let trimmed = half_width(HALT, '」', Kind::Closing, SIZE_PT)[1];
    for (value, sizing, expected) in [
        ("normal", "min-content", ideograph + trimmed),
        ("normal", "max-content", ideograph + advance('」')),
        ("space-all", "min-content", ideograph + advance('」')),
        ("trim-both", "max-content", ideograph + trimmed),
    ] {
        let trace = render(
            &format!("p{{width:{sizing};text-spacing-trim:{value};background:rgb(255,0,0)}}"),
            "<p>水」</p>",
        );
        let label = format!("{value} {sizing}");
        assert_labelled_pt(
            &format!("{label}: box width"),
            painted_box(&trace, RED).width,
            expected,
        );
        assert_labelled_pt(
            &format!("{label}: line width"),
            trace.pages[0][0].width,
            expected,
        );
    }
}

#[test]
fn svg_text_wraps_after_trimmed_punctuation_with_a_negative_advance() {
    // Letter spacing leaves 。 less than half its advance, which trim-all then removes, so
    // the stop ending the first line of the wrapped SVG text has a negative advance under
    // its conditional hang.
    let html = format!(
        "<!doctype html><style>{FACES}</style><svg width='200' height='100'>\
         <text x='0' y='40' style='font:20px Halt;inline-size:10px;\
         hanging-punctuation:allow-end;text-spacing-trim:trim-all;letter-spacing:-15px'>\
         水水。水</text></svg>"
    );
    let output = ferri_pdf::render_html(&html, &common::local_options());
    assert!(
        output.is_ok(),
        "wrapped SVG text must render its negative-advance stop: {:?}",
        output.err()
    );
}
