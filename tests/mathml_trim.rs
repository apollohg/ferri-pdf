//! Retained native `MathML` content must remain reachable by ordinary CSS text-box trimming.
mod common;

use common::{PROFILE, assert_labelled_pt, render_trace};

fn baselines(body: &str, css: &str) -> [f32; 2] {
    let trace = render_trace(&format!(
        "<!doctype html><style>{PROFILE}p{{margin:0}}.content{{line-height:50pt}}{css}</style>{body}"
    ));
    ["A", "B"].map(|text| {
        let lines: Vec<_> = trace
            .pages
            .iter()
            .flatten()
            .filter(|line| line.text == text)
            .collect();
        assert_eq!(
            lines.len(),
            1,
            "canonical {text} must survive once: {:?}",
            trace.pages
        );
        lines[0].baseline
    })
}

#[test]
fn css_fallback_descendants_keep_their_own_text_box_trim() {
    let ordinary = "<div class=content>A</div><p>B</p>";
    let native = "<math display=block><mtext><div class=content>A</div></mtext></math><p>B</p>";
    let plain = baselines(ordinary, "");
    let expected = baselines(ordinary, ".content{text-box-trim:trim-both}");
    assert!(
        expected[1] < plain[1],
        "the actual ordinary trim control must shrink the50pt line"
    );
    let untrimmed = baselines(native, "");
    let trimmed = baselines(native, ".content{text-box-trim:trim-both}");
    assert_labelled_pt(
        "the following ordinary line starts at the same untrimmed block end",
        untrimmed[1],
        plain[1],
    );
    assert_labelled_pt(
        "a retained CSS descendant's own trim reaches the following ordinary line",
        trimmed[1],
        expected[1],
    );
    assert_labelled_pt(
        "the native-local A baseline receives the same trim adjustment",
        trimmed[0] - untrimmed[0],
        expected[0] - plain[0],
    );
}

#[test]
fn native_math_does_not_forward_host_text_box_trim_to_its_content() {
    let body = "<section><math display=block><mtext><div class=content>A</div></mtext></math></section><p>B</p>";
    let expected = baselines(body, "");
    for css in [
        "math{text-box-trim:trim-both}",
        "section{text-box-trim:trim-both}",
    ] {
        assert_eq!(
            baselines(body, css),
            expected,
            "{css}: the native boundary retains descendant line metrics unless that CSS descendant itself requests trimming"
        );
    }
}
