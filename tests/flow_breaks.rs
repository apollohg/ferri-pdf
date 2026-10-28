mod common;
#[path = "common/weasy_reference.rs"]
mod weasy_reference;

use common::assert_pt;
use weasy_reference::{assert_weasy_lines, render_weasy_case};

const PROBE_FIRST_BASELINE_PT: f32 = 18.461_792;
const PROBE_LINE_HEIGHT_PT: f32 = 10.0;

#[test]
fn list_defaults_keep_preceding_content_through_the_common_cascade_and_paginator() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/flow/weasy70-list-breaks.json")).unwrap();
    assert_weasy_lines(&reference);
}

#[test]
fn oversized_avoided_boxes_try_a_fresh_fragmentainer_without_losing_progress() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/flow/weasy70-avoid.json")).unwrap();
    assert_weasy_lines(&reference);
}

#[test]
fn forced_descendant_breaks_keep_the_fitting_prefix_and_every_following_line() {
    const LEAD_HEIGHT_PT: f32 = 30.0;
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/flow/weasy70-avoid.json")).unwrap();
    let trace = render_weasy_case(&reference, &reference["observations"][0])
        .layout
        .unwrap();
    let expected = [
        vec!["Lead", "A00", "A01", "A02", "A03", "A04"],
        vec![
            "A05", "A06", "A07", "A08", "A09", "A10", "A11", "A12", "A13", "A14",
        ],
        vec!["After"],
    ];
    assert_eq!(
        trace.pages.len(),
        expected.len(),
        "forced internal break and complete source"
    );
    for (number, (page, labels)) in trace.pages.iter().zip(expected).enumerate() {
        assert_eq!(
            page.iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            labels
        );
        for (index, line) in page.iter().enumerate() {
            let offset = if number == 0 && index > 0 {
                LEAD_HEIGHT_PT + (index - 1) as f32 * PROBE_LINE_HEIGHT_PT
            } else {
                index as f32 * PROBE_LINE_HEIGHT_PT
            };
            assert_pt(line.baseline, PROBE_FIRST_BASELINE_PT + offset);
        }
    }
}

#[test]
fn oversized_bottom_caption_starts_fresh_and_rewraps_its_kept_final_line() {
    const CAPTION_WORDS: usize = 60;
    const WIDE_WORDS_PER_LINE: usize = 6;
    const NARROW_WORDS_PER_LINE: usize = 3;
    const WIDE_LINES: usize = 9;
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/flow/weasy70-avoid.json")).unwrap();
    let case = reference["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "caption-bottom-avoid")
        .unwrap();
    let trace = render_weasy_case(&reference, case).layout.unwrap();
    let words = (0..CAPTION_WORDS)
        .map(|index| format!("S{index:03}"))
        .collect::<Vec<_>>();
    let boundary = WIDE_WORDS_PER_LINE * WIDE_LINES;
    let mut final_lines = words[boundary..]
        .chunks(NARROW_WORDS_PER_LINE)
        .map(|chunk| chunk.join(" "))
        .collect::<Vec<_>>();
    final_lines.push("After".into());
    let expected = [
        vec!["Cell".into()],
        words[..boundary]
            .chunks(WIDE_WORDS_PER_LINE)
            .map(|chunk| chunk.join(" "))
            .collect(),
        final_lines,
    ];
    assert_eq!(
        trace.pages.len(),
        expected.len(),
        "caption must leave the grid page and keep all source"
    );
    for (page, labels) in trace.pages.iter().zip(expected) {
        assert_eq!(
            page.iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            labels
        );
        for (index, line) in page.iter().enumerate() {
            assert_pt(
                line.baseline,
                PROBE_FIRST_BASELINE_PT + index as f32 * PROBE_LINE_HEIGHT_PT,
            );
        }
    }
}
