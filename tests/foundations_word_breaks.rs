mod common;

use common::{PROFILE, assert_pt, render_trace};
use ferri_pdf::LayoutTrace;

fn trace(css: &str, markup: &str) -> LayoutTrace {
    render_trace(&format!(
        "<!doctype html><style>{PROFILE}p{{margin:0}}{css}</style>{markup}"
    ))
}

fn lines(trace: &LayoutTrace) -> Vec<&str> {
    trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect()
}

#[test]
fn wbr_wraps_only_when_needed_and_adds_no_text_or_width() {
    let narrow = trace("p{width:45pt}", "<p><wbr>alpha<wbr><wbr>beta<wbr></p>");
    assert_eq!(lines(&narrow), ["alpha", "beta"]);
    let wide = trace("p{width:200pt}", "<p>alpha<wbr>beta</p><p>alphabeta</p>");
    assert_eq!(lines(&wide), ["alphabeta", "alphabeta"]);
    assert_pt(wide.pages[0][0].width, wide.pages[0][1].width);
    assert_eq!(
        wide.pages[0][0]
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>(),
        "alphabeta"
    );
}

#[test]
fn wbr_whitespace_cascade_and_generated_content_control_the_break() {
    assert_eq!(
        lines(&trace(
            "p{width:45pt;white-space:nowrap}",
            "<p>alpha<wbr>beta</p>"
        )),
        ["alphabeta"]
    );
    assert_eq!(
        lines(&trace(
            "p{width:45pt}",
            "<p><nobr>alpha<wbr>beta</nobr></p>"
        )),
        ["alpha", "beta"]
    );
    assert_eq!(
        lines(&trace(
            "p{width:45pt}wbr{white-space:nowrap}",
            "<p><nobr>alpha<wbr>beta</nobr></p>"
        )),
        ["alphabeta"]
    );
    assert_eq!(
        lines(&trace(
            "p{width:45pt}wbr{display:none}",
            "<p>alpha<wbr>beta</p>"
        )),
        ["alphabeta"]
    );
    assert_eq!(
        lines(&trace(
            "p{width:45pt}wbr::before{content:'a'}wbr::after{content:'b'}",
            "<p>alph<wbr>eta</p>"
        )),
        ["alpha", "beta"]
    );
}

#[test]
fn wbr_offsets_follow_collapsed_spaces_and_unicode_case_expansion() {
    assert_eq!(
        lines(&trace("p{width:45pt}", "<p>alpha   <wbr>beta</p>")),
        ["alpha", "beta"]
    );
    assert_eq!(
        lines(&trace(
            "p{width:200pt;text-transform:capitalize}",
            "<p>un<wbr>broken</p>"
        )),
        ["Unbroken"]
    );
    let expanded = trace("p{width:16pt;text-transform:lowercase}", "<p>İ<wbr>AB</p>");
    assert_eq!(lines(&expanded), ["i\u{307}", "ab"]);
    assert_eq!(
        expanded.pages[0][0]
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>(),
        "i\u{307}"
    );
}

#[test]
fn wbr_contributes_to_intrinsic_widths_and_survives_inline_block_splitting() {
    let flex = trace(
        ".row{display:flex;width:50pt}.word{flex:1}.fixed{width:10pt;flex:none}",
        "<div class=row><div class=word>alpha<wbr>beta</div><div class=fixed>X</div></div>",
    );
    assert!(lines(&flex).contains(&"alpha"));
    assert!(lines(&flex).contains(&"beta"));
    for line in &flex.pages[0] {
        assert!(
            line.x + line.width <= 86.001,
            "{:?} at {} + {} must fit the 50pt row",
            line.text,
            line.x,
            line.width
        );
    }
    let split = trace(
        ".outer{width:45pt}",
        "<div class=outer><span>alpha<wbr>beta<div>Block</div>delta<wbr>echo</span></div>",
    );
    assert_eq!(lines(&split), ["alpha", "beta", "Block", "delta", "echo"]);
}

#[test]
fn wbr_keeps_rtl_logical_text_and_utf8_source_ranges() {
    let output = trace("p{width:25pt;direction:rtl}", "<p>אב<wbr>גד</p>");
    assert_eq!(lines(&output), ["אב", "גד"]);
    for (line, range) in output.pages[0].iter().zip([0..4, 4..8]) {
        assert_eq!(line.runs.len(), 1);
        assert!(line.runs[0].rtl);
        assert_eq!(line.runs[0].source_range, range);
    }
}
