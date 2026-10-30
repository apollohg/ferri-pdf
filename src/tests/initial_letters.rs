//! Initial letters (CSS Inline 3 §7), checked against the profile font's own metrics.
use super::common;
use crate::render_html;
use common::{PROFILE, assert_labelled_pt, local_options, painted_box};

const PROFILE_FONT: &[u8] = include_bytes!("../../tests/oracle/assets/DejaVuSans.ttf");
/// The profile's `body` font size and line height, and its page content box origin.
const FONT_SIZE: f32 = 12.0;
const LINE_HEIGHT: f32 = 18.0;
const CONTENT_LEFT: f32 = 36.0;
const CONTENT_TOP: f32 = 36.0;
const CONTENT_WIDTH: f32 = 288.0;
const TEXT: &str = "Lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor \
                    incididunt ut labore et dolore magna aliqua ut enim ad minim veniam quis \
                    nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo";
const LIME: [u8; 3] = [0, 255, 0];
const RED: [u8; 3] = [255, 0, 0];

fn render(css: &str, body: &str) -> crate::LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("render failed: {error}\ncss: {css}\nbody: {body}"))
    .layout
    .expect("layout requested")
}

fn render_error(css: &str, body: &str) -> String {
    match render_html(
        &format!("<style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    ) {
        Ok(_) => panic!("expected the render to fail\ncss: {css}\nbody: {body}"),
        Err(error) => error.to_string(),
    }
}

/// The profile face's metrics in em: cap height (the top of `H`, as it has no OS/2 cap
/// height), ascent and descent.
struct Font {
    cap: f32,
    ascent: f32,
    descent: f32,
}
impl Font {
    fn profile() -> Self {
        let face = ttf_parser::Face::parse(PROFILE_FONT, 0).expect("profile font parses");
        let em = f32::from(face.units_per_em());
        let cap = face
            .glyph_index('H')
            .and_then(|glyph| face.glyph_bounding_box(glyph))
            .expect("the profile font has an H");
        Self {
            cap: f32::from(cap.y_max) / em,
            ascent: f32::from(face.ascender()) / em,
            descent: -f32::from(face.descender()) / em,
        }
    }
    /// The used size of a letter whose cap height spans `lines` lines of profile text from
    /// its cap height to its baseline (§7.5.3).
    fn letter_size(&self, lines: f32) -> f32 {
        ((lines - 1.0) * LINE_HEIGHT + self.cap * FONT_SIZE) / self.cap
    }
    /// The first profile line's baseline below its line box top.
    fn baseline(&self) -> f32 {
        let content = (self.ascent + self.descent) * FONT_SIZE;
        self.ascent * FONT_SIZE + (LINE_HEIGHT - content) / 2.0
    }
    /// The left side bearing of `ch` at `size`.
    fn bearing(ch: char, size: f32) -> f32 {
        let face = ttf_parser::Face::parse(PROFILE_FONT, 0).expect("profile font parses");
        let bounds = face
            .glyph_index(ch)
            .and_then(|glyph| face.glyph_bounding_box(glyph))
            .expect("the profile font has the glyph");
        f32::from(bounds.x_min) / f32::from(face.units_per_em()) * size
    }
}

/// The first page's line holding `text` alone, and the other lines in order.
fn split<'a>(
    trace: &'a crate::LayoutTrace,
    text: &str,
) -> (&'a crate::TextLine, Vec<&'a crate::TextLine>) {
    let lines = &trace.pages[0];
    let letter = lines
        .iter()
        .find(|line| line.text == text)
        .unwrap_or_else(|| panic!("no line {text:?} among {:#?}", texts(lines)));
    let rest = lines
        .iter()
        .filter(|line| !std::ptr::eq(*line, letter))
        .collect();
    (letter, rest)
}

/// Where a line's text starts: its first run, after any initial letter the line holds.
fn start(line: &crate::TextLine) -> f32 {
    line.x + line.runs.first().map_or(0.0, |run| run.x)
}

fn texts(lines: &[crate::TextLine]) -> Vec<(String, f32, f32)> {
    lines
        .iter()
        .map(|line| (line.text.clone(), start(line), line.baseline))
        .collect()
}

fn describe(lines: &[&crate::TextLine]) -> Vec<(String, f32, f32)> {
    lines
        .iter()
        .map(|line| (line.text.clone(), start(line), line.baseline))
        .collect()
}

#[test]
fn drop_cap_spans_three_lines_and_excludes_their_start() {
    let font = Font::profile();
    let trace = render(
        "p::first-letter{initial-letter:3}",
        &format!("<p>{TEXT}</p>"),
    );
    let (letter, lines) = split(&trace, "L");
    let size = font.letter_size(3.0);
    assert_labelled_pt("drop cap font size", letter.runs[0].font_size, size);
    assert_labelled_pt(
        "the drop cap's baseline is the third line's",
        letter.baseline,
        lines[2].baseline,
    );
    assert_labelled_pt(
        "the drop cap's cap height is the first line's",
        letter.baseline - font.cap * size,
        lines[0].baseline - font.cap * FONT_SIZE,
    );
    assert_labelled_pt(
        "the first line keeps its place",
        lines[0].baseline,
        CONTENT_TOP + font.baseline(),
    );
    assert_labelled_pt(
        "zero padding and borders kern the glyph to the line start (§7.6.2)",
        letter.x + Font::bearing('L', size),
        CONTENT_LEFT,
    );
    for (index, line) in lines.iter().enumerate().take(3) {
        assert_labelled_pt(
            &format!(
                "line {index} starts after the letter: {:#?}",
                describe(&lines)
            ),
            start(line),
            letter.x + letter.width,
        );
    }
    assert_labelled_pt(
        &format!(
            "the fourth line is below the letter: {:#?}",
            describe(&lines)
        ),
        start(lines[3]),
        CONTENT_LEFT,
    );
}

#[test]
fn letter_size_follows_its_lines_and_sink_follows_drop_and_raise() {
    let font = Font::profile();
    for (value, lines, sink) in [
        ("2", 2.0, 2),
        ("2.5", 2.5, 2),
        ("drop 3", 3.0, 3),
        ("3 3", 3.0, 3),
        ("raise 3", 3.0, 1),
        ("3 1", 3.0, 1),
        ("2.51 3", 2.51, 3),
    ] {
        let trace = render(
            &format!("p::first-letter{{initial-letter:{value}}}"),
            &format!("<p>{TEXT}</p>"),
        );
        let (letter, rest) = split(&trace, "L");
        let size = font.letter_size(lines);
        assert_labelled_pt(
            &format!("initial-letter: {value} font size"),
            letter.runs[0].font_size,
            size,
        );
        if lines >= sink as f32 {
            assert_labelled_pt(
                &format!(
                    "initial-letter: {value} sits on line {sink}: {:#?}",
                    describe(&rest)
                ),
                letter.baseline,
                rest[sink - 1].baseline,
            );
        } else {
            assert_labelled_pt(
                &format!("initial-letter: {value} hangs from the first line's cap height"),
                letter.baseline - font.cap * size,
                rest[0].baseline - font.cap * FONT_SIZE,
            );
            for (index, line) in rest.iter().enumerate().take(sink) {
                assert_labelled_pt(
                    &format!("initial-letter: {value} excludes line {index} of its sink"),
                    start(line),
                    letter.x + letter.width,
                );
            }
            assert_labelled_pt(
                &format!("initial-letter: {value} leaves the line after its sink"),
                start(rest[sink]),
                CONTENT_LEFT,
            );
        }
    }
}

#[test]
fn raised_letters_move_their_line_down_to_stay_inside_the_content_box() {
    let font = Font::profile();
    for (value, sink) in [("3 1", 1), ("3 2", 2)] {
        let trace = render(
            &format!("p::first-letter{{initial-letter:{value}}}"),
            &format!("<p>{TEXT}</p>"),
        );
        let (letter, rest) = split(&trace, "L");
        let size = font.letter_size(3.0);
        assert_labelled_pt(
            &format!("initial-letter: {value} keeps its cap height at the content top (§7.6.1)"),
            letter.baseline - font.cap * size,
            CONTENT_TOP,
        );
        assert_labelled_pt(
            &format!("initial-letter: {value} aligns to line {sink}"),
            letter.baseline,
            rest[sink - 1].baseline,
        );
        assert_labelled_pt(
            &format!("initial-letter: {value} excludes only the lines of its sink"),
            start(rest[sink]),
            CONTENT_LEFT,
        );
    }
}

#[test]
fn space_after_a_sunken_letter_collapses_but_a_raised_one_keeps_it() {
    let first_line = |value: &str| {
        let trace = render(
            &format!("span{{initial-letter:{value}}}"),
            "<p><span>Words</span> follow the letter</p>",
        );
        let (_, rest) = split(&trace, "Words");
        rest[0].text.clone()
    };
    let placeholder = crate::document::OBJECT_REPLACEMENT_CHARACTER;
    assert_eq!(
        first_line("2"),
        format!("{placeholder}follow the letter"),
        "collapsible space after a sunken letter is collapsed away (§7.8.1)"
    );
    assert_eq!(
        first_line("2 1"),
        format!("{placeholder} follow the letter"),
        "a raised letter's line keeps the word space after it"
    );
}

#[test]
fn atomic_initial_letters_take_the_block_size_of_their_lines() {
    let font = Font::profile();
    let span = LINE_HEIGHT + font.cap * FONT_SIZE;
    let trace = render(
        "span{display:inline-block;initial-letter:2;background:lime;width:20pt}",
        &format!("<p><span></span>{TEXT}</p>"),
    );
    let lime = painted_box(&trace, LIME);
    assert_labelled_pt(
        "an automatic block size spans the letter's lines (§7.5.5)",
        lime.height,
        span,
    );
    let lines = &trace.pages[0];
    assert_labelled_pt(
        "its content box bottom is the second line's baseline",
        lime.y + lime.height,
        lines[1].baseline,
    );
    assert_labelled_pt(
        "its content box top is the first line's cap height",
        lime.y,
        lines[0].baseline - font.cap * FONT_SIZE,
    );
    for line in &lines[..2] {
        assert_labelled_pt(
            "lines beside the box start after it",
            start(line),
            lime.x + 20.0,
        );
    }
    assert_labelled_pt("the third line is below it", start(&lines[2]), CONTENT_LEFT);
}

#[test]
fn inside_markers_and_the_first_inline_box_become_initial_letters() {
    let font = Font::profile();
    let size = font.letter_size(2.0);
    let trace = render(
        "ol{margin:0;padding:0;list-style-position:inside}li::marker{initial-letter:2}",
        &format!("<ol><li>{TEXT}</li></ol>"),
    );
    let (marker, lines) = split(&trace, "1. ");
    assert_labelled_pt("inside marker letter size", marker.runs[0].font_size, size);
    assert_labelled_pt(
        "the marker letter sits on the second line",
        marker.baseline,
        lines[1].baseline,
    );
    let trace = render(
        "em{initial-letter:2}b{initial-letter:3}",
        &format!("<p><span><em><b>T</b>wo</em> words</span> {TEXT}</p>"),
    );
    let (letter, _) = split(&trace, "Two");
    for run in &letter.runs {
        assert_labelled_pt(
            &format!(
                "the first-most inline box is the letter, and its descendants' own \
                 initial-letter is ignored (§7.3.1): {:?}",
                run.text
            ),
            run.font_size,
            size,
        );
    }
}

#[test]
fn initial_letter_is_normal_after_other_inline_content() {
    for (css, body) in [
        ("span{initial-letter:2}", "<p>A <span>late</span> span</p>"),
        ("b{initial-letter:2}", "<p><i></i><b>second</b> child</p>"),
        (
            "ol{list-style-position:inside}li::first-letter{initial-letter:2}",
            "<ol><li>After the marker</li></ol>",
        ),
    ] {
        let trace = render(css, body);
        for line in &trace.pages[0] {
            for run in &line.runs {
                assert_labelled_pt(
                    &format!("{body}: {:?} keeps the computed font size", run.text),
                    run.font_size,
                    FONT_SIZE,
                );
            }
        }
    }
}

#[test]
fn first_line_styles_reach_the_initial_letter() {
    let trace = render(
        "p::first-line{color:red}p::first-letter{initial-letter:2}",
        &format!("<p>{TEXT}</p>"),
    );
    let (letter, lines) = split(&trace, "L");
    assert_eq!(
        letter.runs[0].color.rgb, RED,
        "::first-letter inherits from ::first-line"
    );
    assert_eq!(lines[0].runs[0].color.rgb, RED);
    assert_ne!(lines[1].runs[0].color.rgb, RED);
}

#[test]
fn short_paragraph_letters_exclude_the_next_block_until_it_clears_them() {
    let font = Font::profile();
    let size = font.letter_size(3.0);
    let following = |css: &str| {
        let trace = render(
            &format!("p{{margin:0}}p.short::first-letter{{initial-letter:3}}{css}"),
            &format!("<p class=short>Short.</p><p class=next>{TEXT}</p>"),
        );
        let (letter, lines) = split(&trace, "S");
        let next: Vec<_> = lines.into_iter().skip(1).collect();
        (
            letter.x + letter.width,
            letter.baseline,
            start(next[0]),
            next[0].baseline,
            describe(&next),
        )
    };
    let (right, _, x, baseline, lines) = following("");
    assert_labelled_pt(
        &format!("the next paragraph wraps around the letter (§7.9.2): {lines:#?}"),
        x,
        right,
    );
    assert_labelled_pt(
        "the next paragraph follows the short one",
        baseline,
        CONTENT_TOP + LINE_HEIGHT + font.baseline(),
    );
    for css in [
        "p.next::first-letter{initial-letter:2}",
        "p.next{overflow:hidden}",
        "p.next{clear:left}",
    ] {
        let (_, letter_baseline, x, baseline, lines) = following(css);
        let line_top = baseline - font.baseline();
        assert!(
            line_top >= letter_baseline - 0.001,
            "{css}: the next block clears the letter, whose bottom is its baseline \
             {letter_baseline}, but its first line box starts at {line_top}: {lines:#?}"
        );
        if css.contains("first-letter") {
            continue;
        }
        assert_labelled_pt(
            &format!("{css}: the cleared line starts at the edge"),
            x,
            CONTENT_LEFT,
        );
    }
    let _ = size;
}

#[test]
fn floats_beside_a_letter_follow_its_line_or_clear_it() {
    let font = Font::profile();
    let trace = render(
        "p::first-letter{initial-letter:3}i{float:left;width:20pt;height:10pt;background:lime}",
        &format!("<p>Lorem <i></i>{TEXT}</p>"),
    );
    let lime = painted_box(&trace, LIME);
    let (letter, lines) = split(&trace, "L");
    assert_labelled_pt(
        "a float of the first line moves past the letter to the edge (§7.9.3)",
        lime.x,
        CONTENT_LEFT,
    );
    assert_labelled_pt(
        "the letter follows the float",
        letter.x + Font::bearing('L', font.letter_size(3.0)),
        CONTENT_LEFT + 20.0,
    );
    assert_labelled_pt(
        "the first line follows the letter",
        start(lines[0]),
        letter.x + letter.width,
    );
    let trace = render(
        "p::first-letter{initial-letter:3}i{float:left;width:20pt;height:10pt;background:lime}",
        &format!("<p>{TEXT} <i></i>{TEXT}</p>"),
    );
    let lime = painted_box(&trace, LIME);
    let (letter, _) = split(&trace, "L");
    assert!(
        lime.y >= letter.baseline - 0.001,
        "an inline-start float after the first line clears the letter: float top {}, \
         letter bottom {}",
        lime.y,
        letter.baseline
    );
}

#[test]
fn letters_lean_on_the_start_side_and_its_indent() {
    let font = Font::profile();
    let size = font.letter_size(2.0);
    let trace = render(
        "p{text-indent:20pt}p::first-letter{initial-letter:2}",
        &format!("<p>{TEXT}</p>"),
    );
    let (letter, lines) = split(&trace, "L");
    assert_labelled_pt(
        "text-indent shifts the letter (§7.8.2)",
        letter.x + Font::bearing('L', size),
        CONTENT_LEFT + 20.0,
    );
    assert_labelled_pt(
        "the second line is shortened by the shifted letter",
        start(lines[1]),
        letter.x + letter.width,
    );
    let trace = render(
        "p{direction:rtl;text-align:start}p::first-letter{initial-letter:2}",
        &format!("<p>{TEXT}</p>"),
    );
    let (letter, lines) = split(&trace, "L");
    assert!(
        letter.x + letter.width > CONTENT_LEFT + CONTENT_WIDTH - 1.0,
        "a right-to-left letter sits at the right edge: {} + {}",
        letter.x,
        letter.width
    );
    for line in &lines[..2] {
        let end = line
            .runs
            .iter()
            .map(|run| line.x + run.x + run.width)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            end <= letter.x + 0.001,
            "right-to-left text beside the letter ends before it at {}: {:#?}",
            letter.x,
            describe(&lines)
        );
    }
}

#[test]
fn letter_paragraphs_move_whole_to_the_next_page_rather_than_break_beside_the_letter() {
    // Two of the three lines beside the letter fit below the filler.
    let filler = "<div style=height:366pt></div>";
    let trace = render(
        "p::first-letter{initial-letter:3}",
        &format!("{filler}<p>{TEXT}</p>"),
    );
    assert!(
        trace.pages[0].is_empty(),
        "two lines fit at the page end, but breaking beside the letter is avoided \
         (§7.9.4): {:#?}",
        texts(&trace.pages[0])
    );
    assert!(
        trace.pages[1].iter().any(|line| line.text == "L"),
        "the letter moves with its paragraph"
    );
    let control = render("", &format!("{filler}<p>{TEXT}</p>"));
    assert!(
        !control.pages[0].is_empty(),
        "without the letter the paragraph splits there"
    );
}

#[test]
fn block_level_content_ends_the_letter_and_its_box_continues_after() {
    let font = Font::profile();
    let trace = render(
        "span{initial-letter:2}div{margin:0}",
        &format!("<div><span>W<div>{TEXT}</div>c</span> tail</div>"),
    );
    let (letter, lines) = split(&trace, "W");
    assert_labelled_pt(
        "the letter keeps its size",
        letter.runs[0].font_size,
        font.letter_size(2.0),
    );
    assert_eq!(
        lines[0].text,
        "\u{fffc}",
        "the letter alone takes the line before the block: {:#?}",
        describe(&lines)
    );
    assert_labelled_pt(
        "the letter's baseline is the block's first line's (§7.9.2)",
        letter.baseline,
        lines[1].baseline,
    );
    assert!(
        start(lines[1]) >= letter.x + letter.width,
        "the block's first line starts after the letter: {:#?}",
        describe(&lines)
    );
    assert_labelled_pt(
        "the block's next line clears the letter",
        start(lines[2]),
        CONTENT_LEFT,
    );
    let rest = lines.last().expect("the box continues after the block");
    assert_eq!(rest.text, "c tail", "{:#?}", describe(&lines));
    assert!(
        rest.runs.iter().all(|run| run.font_size == FONT_SIZE),
        "the box's fragments after the block are ordinary text: {:#?}",
        rest.runs
    );
}

#[test]
fn letters_too_long_for_a_line_wrap_each_line_as_a_letter() {
    let font = Font::profile();
    let trace = render(
        "span{initial-letter:3}",
        &format!("<p><span>Once more</span> {TEXT}</p>"),
    );
    let (first, rest) = split(&trace, "Once");
    let last = rest[0];
    let lines = &rest[1..];
    let size = font.letter_size(3.0);
    assert_eq!(last.text, "more", "{:#?}", describe(&rest));
    for row in [first, last] {
        assert_labelled_pt("each letter line's size", row.runs[0].font_size, size);
    }
    assert_labelled_pt(
        "the first letter line drops from the paragraph's first line",
        first.baseline - font.cap * size,
        CONTENT_TOP + font.baseline() - font.cap * FONT_SIZE,
    );
    let top = lines[0].baseline - font.baseline();
    assert!(
        top >= first.baseline && top <= first.baseline + font.descent * size,
        "the last letter line originates on the first line box below the first letter \
         line's glyphs, at {top}: {:#?}",
        describe(&rest)
    );
    assert_labelled_pt(
        "the last letter line sinks three lines from there",
        last.baseline,
        lines[2].baseline,
    );
    assert_labelled_pt(
        "the last letter line starts at the line's start",
        last.x,
        CONTENT_LEFT,
    );
    for line in &lines[..3] {
        assert_labelled_pt(
            "normal text follows the last letter line",
            start(line),
            last.x + last.width,
        );
    }
    assert_labelled_pt(
        "the line after the sink clears the letter",
        start(lines[3]),
        CONTENT_LEFT,
    );
    let rtl = render(
        "p{direction:rtl}span{initial-letter:3}",
        &format!("<p><span>Once more</span> {TEXT}</p>"),
    );
    let (_, rest) = split(&rtl, "Once");
    let last = rest[0];
    assert_labelled_pt(
        "a right-to-left letter's last line ends at the line's right start",
        last.x + last.width,
        CONTENT_LEFT + CONTENT_WIDTH,
    );
    let text_end = rest[1].x
        + rest[1]
            .runs
            .iter()
            .map(|run| run.x + run.width)
            .fold(0.0, f32::max);
    assert!(
        text_end <= last.x + 0.001,
        "right-to-left text ends before the last letter line at {}: {:#?}",
        last.x,
        describe(&rest)
    );
}

#[test]
fn letters_needing_layout_to_size_or_wrap_are_unsupported() {
    for (css, body, expected) in [
        (
            "p::first-letter{initial-letter:2;initial-letter-align:border-box;padding:5%}",
            "<p>Text</p>",
            "percentage block-axis padding",
        ),
        (
            "span{initial-letter:3;background:lime}",
            "<p><span>many words that cannot fit on one line at this size</span></p>",
            "box decorations of contents wrapping onto several lines",
        ),
    ] {
        let error = render_error(css, body);
        assert!(
            error.contains(expected),
            "{css} {body}: expected an error naming {expected:?}, got {error:?}"
        );
    }
}

#[test]
fn inline_box_ancestors_start_after_the_letter() {
    let trace = render(
        "p::first-letter{initial-letter:2}span{background:lime}",
        "<p><span>Lorem ipsum</span> dolor</p>",
    );
    let (letter, _) = split(&trace, "L");
    let lime = painted_box(&trace, LIME);
    assert_labelled_pt(
        "the ancestor inline box is drawn outside the letter (§7.8.3)",
        lime.x,
        letter.x + letter.width,
    );
}

#[test]
fn leading_alignment_spans_the_lines_half_leading_edges() {
    let font = Font::profile();
    let trace = render(
        "p::first-letter{initial-letter:3;initial-letter-align:leading}",
        &format!("<p>{TEXT}</p>"),
    );
    let (letter, _) = split(&trace, "L");
    assert_labelled_pt(
        "the cap height meets the first line box's top",
        letter.baseline - font.cap * letter.runs[0].font_size,
        CONTENT_TOP,
    );
    assert_labelled_pt(
        "the baseline meets the third line box's bottom",
        letter.baseline,
        CONTENT_TOP + 3.0 * LINE_HEIGHT,
    );
}

#[test]
fn content_languages_choose_the_alignment_by_the_ua_style_sheet() {
    let size = |lang: &str| {
        let trace = render(
            "p::first-letter{initial-letter:2}",
            &format!("<p lang={lang}>{TEXT}</p>"),
        );
        split(&trace, "L").0.runs[0].font_size
    };
    let alphabetic = size("en");
    assert_labelled_pt(
        "alphabetic alignment for English",
        alphabetic,
        Font::profile().letter_size(2.0),
    );
    let ideographic = size("ja");
    assert!(
        (ideographic - alphabetic).abs() > 0.1,
        "Japanese aligns to the ideographic ink box: {ideographic} vs {alphabetic} (§7.4.1)"
    );
    assert_labelled_pt(
        "a Latin script tag restores alphabetic alignment",
        size("ja-Latn"),
        alphabetic,
    );
    assert!(
        (size("hi") - alphabetic).abs() > 0.1,
        "Hindi aligns to the hanging baseline"
    );
}
