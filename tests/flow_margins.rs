mod common;
#[path = "common/weasy_reference.rs"]
mod weasy_reference;

use common::assert_pt;
use weasy_reference::{assert_weasy_lines, render_weasy_case};

#[test]
fn initial_collapsed_margins_and_break_partitions_match_independent_reference() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/flow/weasy70-margins.json")).unwrap();
    assert_weasy_lines(&reference);
}

#[test]
fn automatic_column_breaks_truncate_adjoining_margins_after_the_initial_flow() {
    const INITIAL_MARGIN_PT: f32 = 12.0;
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/flow/weasy70-margins.json")).unwrap();
    let case = &reference["observations"][0];
    let trace = render_weasy_case(&reference, case).layout.unwrap();
    assert_eq!(trace.pages.len(), 2);
    for (page, labels) in trace
        .pages
        .iter()
        .zip([["First", "Second"], ["Third", "Fourth"]])
    {
        assert_eq!(
            page.iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            labels,
            "automatic columns must preserve each authored line exactly once"
        );
    }
    let control = reference["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "reset-body")
        .unwrap();
    let baseline = control["pages"][0][0]["baseline"].as_f64().unwrap() as f32;
    for (index, label) in ["First", "Second", "Third", "Fourth"]
        .into_iter()
        .enumerate()
    {
        let line = trace.pages[index / 2]
            .iter()
            .find(|line| line.text == label)
            .unwrap();
        assert_pt(
            line.baseline,
            baseline + if index == 0 { INITIAL_MARGIN_PT } else { 0.0 },
        );
    }
}
