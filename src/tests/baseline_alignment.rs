//! Baseline alignment of inline-level boxes (CSS Inline 3 §4) against the profile font's
//! `DejaVu Sans` metrics and the `BASE` table of the Source Serif 4 fixture, read with
//! `FontTools`.
use super::common;
use crate::{BoxPaint, DisplayItem, LayoutTrace, render_html};
use common::{PROFILE, local_options};

const TOLERANCE_PT: f32 = 1e-3;
const PARENT_PT: f32 = 20.0;
const CHILD_PT: f32 = 10.0;
/// The profile's inherited line height.
const LINE_HEIGHT_PT: f32 = 18.0;
const DEJAVU_UNITS: f32 = 2048.0;
/// `hhea` ascender and descender, the top of `x` (`OS/2` version 1 has no `sxHeight`) and
/// the `MATH` axis height. Without a `BASE` table the ideographic edges are the ascent and
/// descent, so the central baseline lies halfway between them.
const DEJAVU_ASCENT: f32 = 1901.0 / DEJAVU_UNITS;
const DEJAVU_DESCENT: f32 = 483.0 / DEJAVU_UNITS;
const DEJAVU_X_HEIGHT: f32 = 1120.0 / DEJAVU_UNITS;
const DEJAVU_MATH: f32 = 642.0 / DEJAVU_UNITS;
const DEJAVU_CENTRAL: f32 = (DEJAVU_ASCENT - DEJAVU_DESCENT) / 2.0;
/// Source Serif 4 records only the `ideo` baseline, 165 units below `romn` of 1000; the
/// ideographic-over baseline is then 1em above it.
const SERIF_IDEOGRAPHIC_UNDER: f32 = -0.165;
const SERIF_CENTRAL: f32 = SERIF_IDEOGRAPHIC_UNDER + 0.5;
const SERIF_FACE: &str =
    "@font-face{font-family:Serif4;src:url('../../fixtures/fonts/SourceSerif4-Regular.ttf')}";
/// CSS Inline 3 Appendix A.2's suggested hanging baseline.
const FALLBACK_HANGING: f32 = 0.6;
const SUBSCRIPT_DESCENT_FRACTION: f32 = 0.5;
const SUPERSCRIPT_ASCENT_FRACTION: f32 = 0.5;

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{SERIF_FACE}p{{font-size:{PARENT_PT}pt}}span{{font-size:{CHILD_PT}pt}}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("baseline fixture must render: {error}\n{css}\n{body}"))
    .layout
    .unwrap()
}

/// How far the span's text baseline lies below the paragraph's.
fn span_offset(css: &str) -> f32 {
    let trace = render(css, "<p>X<span>x</span></p>");
    let line = &trace.pages[0][0];
    line.runs[1].baseline_shift - line.runs[0].baseline_shift
}

fn assert_pt(actual: f32, expected: f32, label: &str) {
    assert!(
        (actual - expected).abs() <= TOLERANCE_PT,
        "{label}: {actual}pt, expected {expected}pt"
    );
}

#[test]
fn alignment_baselines_match_the_parent_baseline_of_the_same_type() {
    let difference = PARENT_PT - CHILD_PT;
    for (value, expected) in [
        ("baseline", 0.0),
        ("alphabetic", 0.0),
        ("central", -DEJAVU_CENTRAL * difference),
        ("middle", -DEJAVU_X_HEIGHT / 2.0 * difference),
        ("mathematical", -DEJAVU_MATH * difference),
        ("ideographic", DEJAVU_DESCENT * difference),
        ("text-after-edge", DEJAVU_DESCENT * difference),
        ("text-before-edge", -DEJAVU_ASCENT * difference),
    ] {
        assert_pt(
            span_offset(&format!("span{{alignment-baseline:{value}}}")),
            expected,
            &format!("alignment-baseline: {value}"),
        );
    }
}

#[test]
fn the_dominant_baseline_aligns_children_that_align_by_baseline() {
    assert_pt(
        span_offset("p{dominant-baseline:central}"),
        -DEJAVU_CENTRAL * (PARENT_PT - CHILD_PT),
        "the paragraph's central baseline",
    );
    assert_pt(
        span_offset("p{dominant-baseline:central}span{alignment-baseline:alphabetic}"),
        0.0,
        "an explicit alignment baseline overrides the dominant one",
    );
    assert_pt(
        span_offset("p{dominant-baseline:hanging}"),
        -FALLBACK_HANGING * (PARENT_PT - CHILD_PT),
        "without BASE or Devanagari Ka the hanging baseline is the suggested 0.6em",
    );
}

#[test]
fn baseline_shifts_raise_lower_and_add_to_vertical_align() {
    for (css, expected) in [
        ("span{baseline-shift:3pt}", -3.0),
        ("span{baseline-shift:-2pt}", 2.0),
        ("span{baseline-shift:50%}", -LINE_HEIGHT_PT / 2.0),
        (
            "span{baseline-shift:sub}",
            DEJAVU_DESCENT * PARENT_PT * SUBSCRIPT_DESCENT_FRACTION,
        ),
        (
            "span{baseline-shift:super}",
            -DEJAVU_ASCENT * PARENT_PT * SUPERSCRIPT_ASCENT_FRACTION,
        ),
        ("span{baseline-shift:baseline}", 0.0),
        ("span{vertical-align:2pt;baseline-shift:3pt}", -5.0),
        (
            "span{alignment-baseline:central;baseline-shift:1pt}",
            -DEJAVU_CENTRAL * (PARENT_PT - CHILD_PT) - 1.0,
        ),
    ] {
        assert_pt(span_offset(css), expected, css);
    }
}

#[test]
fn base_tables_place_the_ideographic_and_central_baselines() {
    let difference = PARENT_PT - CHILD_PT;
    for (value, expected) in [
        ("ideographic", -SERIF_IDEOGRAPHIC_UNDER * difference),
        ("central", -SERIF_CENTRAL * difference),
    ] {
        assert_pt(
            span_offset(&format!(
                "p{{font-family:Serif4}}span{{alignment-baseline:{value}}}"
            )),
            expected,
            &format!("Source Serif 4 {value} from BASE"),
        );
    }
}

fn red_box(trace: &LayoutTrace) -> &BoxPaint {
    trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == [255, 0, 0] => {
                Some(paint)
            }
            _ => None,
        })
        .expect("the inline block paints red")
}

#[test]
fn atomic_inlines_without_content_baselines_synthesize_them_from_the_margin_box() {
    let height = 20.0;
    let trace = render(
        &format!(
            "i{{display:inline-block;width:10pt;height:{height}pt;background:red;alignment-baseline:central}}"
        ),
        "<p>X<i></i></p>",
    );
    let line = &trace.pages[0][0];
    let paint = red_box(&trace);
    assert_pt(
        paint.rect.y + paint.rect.height / 2.0,
        line.baseline - DEJAVU_CENTRAL * PARENT_PT,
        "the box's middle lies on the paragraph's central baseline",
    );
}

#[test]
fn baseline_source_chooses_the_first_or_last_line_of_an_inline_block() {
    // The block's top, and its first and last baselines, above the paragraph's baseline.
    let measure = |source: &str| {
        let trace = render(
            &format!("i{{display:inline-block;background:red;baseline-source:{source}}}"),
            "<p>X<i><span>a<br>b</span></i></p>",
        );
        let baseline = |start: char| {
            trace.pages[0]
                .iter()
                .find(|line| line.text.starts_with(start))
                .unwrap_or_else(|| panic!("a line starting {start:?}"))
                .baseline
        };
        let paragraph = baseline('X');
        [red_box(&trace).rect.y, baseline('a'), baseline('b')].map(|y| paragraph - y)
    };
    let [auto, last, first] = ["auto", "last", "first"].map(measure);
    assert_pt(
        auto[0],
        last[0],
        "inline blocks align by their last line by default",
    );
    assert_pt(
        last[2],
        0.0,
        "last: the second line sits on the paragraph baseline",
    );
    assert_pt(
        first[1],
        0.0,
        "first: the first line sits on the paragraph baseline",
    );
    assert_pt(
        last[0] - first[0],
        last[1] - last[2],
        "the block moves by the distance between its baselines",
    );
}

#[test]
fn line_height_steps_round_line_boxes_up_and_centre_them() {
    let step = 30.0;
    let trace = render(
        &format!("p{{margin:0;line-height:{LINE_HEIGHT_PT}pt}}.s{{line-height-step:{step}pt}}"),
        "<p class=s>X</p><p>X</p><p class=s>X<span style=\"font-size:40pt;line-height:40pt\">X</span></p>",
    );
    let [stepped, plain, tall] = [0, 1, 2].map(|index| &trace.pages[0][index]);
    assert_pt(stepped.height, step, "an 18pt line takes one 30pt step");
    assert_pt(
        plain.height,
        LINE_HEIGHT_PT,
        "lines without a step keep their height",
    );
    assert_pt(tall.height, 2.0 * step, "a line over one step takes two");
    // The plain line's top is the stepped line's bottom; its baseline lies the same distance
    // below its top as the stepped line's would without the step.
    assert_pt(
        plain.baseline - stepped.baseline,
        step - (step - LINE_HEIGHT_PT) / 2.0,
        "the stepped line's content is centred in its step",
    );
}

/// `DejaVu Sans` has no `OS/2` cap height, so the top of `H` gives it.
const DEJAVU_CAP_HEIGHT: f32 = 1493.0 / DEJAVU_UNITS;

/// A line's ascent and descent around its baseline, without trimming.
fn untrimmed() -> [f32; 2] {
    let leading = LINE_HEIGHT_PT - (DEJAVU_ASCENT + DEJAVU_DESCENT) * PARENT_PT;
    [
        DEJAVU_ASCENT * PARENT_PT + leading / 2.0,
        DEJAVU_DESCENT * PARENT_PT + leading / 2.0,
    ]
}

/// The ascent and descent of each line of a red paragraph, from its box and baselines.
fn line_extents(css: &str, body: &str) -> Vec<[f32; 2]> {
    let trace = render(
        &format!("p{{margin:0;line-height:{LINE_HEIGHT_PT}pt;background:red}}{css}"),
        body,
    );
    let paint = red_box(&trace);
    let baselines: Vec<f32> = trace.pages[0].iter().map(|line| line.baseline).collect();
    let mut top = paint.rect.y;
    let bottom = paint.rect.y + paint.rect.height;
    baselines
        .iter()
        .enumerate()
        .map(|(index, &baseline)| {
            let ascent = baseline - top;
            let next = baselines.get(index + 1).copied();
            let descent = next.map_or(bottom - baseline, |_| f32::NAN);
            top = baseline;
            [ascent, descent]
        })
        .collect()
}

#[test]
fn trimmed_block_lines_end_at_their_text_box_edges() {
    let [ascent, descent] = untrimmed();
    for (css, expected) in [
        ("", [ascent, descent]),
        (
            "p{text-box:trim-both cap alphabetic}",
            [DEJAVU_CAP_HEIGHT * PARENT_PT, 0.0],
        ),
        (
            "p{text-box-trim:trim-start}",
            [DEJAVU_ASCENT * PARENT_PT, descent],
        ),
        (
            "p{text-box-trim:trim-end}",
            [ascent, DEJAVU_DESCENT * PARENT_PT],
        ),
        (
            "p{text-box:ex}",
            [DEJAVU_X_HEIGHT * PARENT_PT, DEJAVU_DESCENT * PARENT_PT],
        ),
        // The edges are those of the line's containing block, the paragraph: `auto`.
        (
            "div{text-box:trim-both cap alphabetic}",
            [DEJAVU_ASCENT * PARENT_PT, DEJAVU_DESCENT * PARENT_PT],
        ),
        (
            "div{text-box-trim:trim-both}p{text-box-edge:cap alphabetic}",
            [DEJAVU_CAP_HEIGHT * PARENT_PT, 0.0],
        ),
        (
            "div{text-box-trim:trim-both;display:flex}",
            [ascent, descent],
        ),
    ] {
        let [[over, under]] = line_extents(css, "<div><p>Hx</p></div>")[..] else {
            panic!("{css}: one line");
        };
        assert_pt(over, expected[0], &format!("{css}: ascent"));
        assert_pt(under, expected[1], &format!("{css}: descent"));
    }
    let padded = line_extents(
        "div{text-box-trim:trim-both}p{padding-top:1pt;text-box-edge:cap alphabetic}",
        "<div><p>Hx</p></div>",
    );
    assert_pt(
        padded[0][0],
        1.0 + ascent,
        "padding on the paragraph's top stops the div's start trim",
    );
    assert_pt(
        padded[0][1],
        0.0,
        "the end trim still reaches the last line",
    );
    let two = line_extents("p{text-box:trim-both cap alphabetic}", "<p>H<br>x</p>");
    assert_pt(
        two[0][0],
        DEJAVU_CAP_HEIGHT * PARENT_PT,
        "the first line's over side",
    );
    assert_pt(
        two[1][0],
        descent + ascent,
        "between the lines nothing is trimmed: the first descent and second ascent remain",
    );
    assert_pt(two[1][1], 0.0, "the last line's under side");
}

#[test]
fn trimmed_inline_boxes_paint_their_content_box_to_the_edge_metrics() {
    let trace = render(
        &format!(
            "p{{line-height:{LINE_HEIGHT_PT}pt}}b{{background:red;text-box:trim-both ex alphabetic}}"
        ),
        "<p>X<b>x</b></p>",
    );
    let line = &trace.pages[0][0];
    let paint = red_box(&trace);
    assert_pt(
        paint.rect.y,
        line.baseline - DEJAVU_X_HEIGHT * PARENT_PT,
        "over edge at the x-height",
    );
    assert_pt(
        paint.rect.y + paint.rect.height,
        line.baseline,
        "under edge at the baseline",
    );
}

#[test]
fn trimming_each_column_or_fragment_is_reported_unsupported() {
    for css in [
        "div{columns:2;text-box-trim:trim-start}",
        "div{box-decoration-break:clone;text-box-trim:trim-end}",
    ] {
        let result = render_html(
            &format!("<style>{PROFILE}{css}</style><div><p>x</p></div>"),
            &local_options(),
        );
        assert!(
            matches!(
                result,
                Err(crate::Error::Css(crate::style::StyleError::Unsupported(_)))
            ),
            "{css}: {:?}",
            result.err()
        );
    }
}
