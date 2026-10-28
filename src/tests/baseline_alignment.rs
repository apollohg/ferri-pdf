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
