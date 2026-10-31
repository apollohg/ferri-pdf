//! Canonical CSS fallback content inside native `MathML` must have one paint/event owner.
mod common;

use common::{PROFILE, local_options};
use ferri_pdf::{LayoutTrace, render_html};

fn trace(body: &str, css: &str) -> LayoutTrace {
    render_html(
        &format!("<!doctype html><style>{PROFILE}p{{margin:0}}{css}</style>{body}"),
        &local_options(),
    )
    .expect("retained MathML fallback must render")
    .layout
    .expect("layout requested")
}

#[test]
fn css_fallback_keeps_positioned_children_once_with_their_real_destinations() {
    for position in ["absolute", "fixed"] {
        for child in [
            format!(
                "<mrow id=spot style='position:{position};left:100pt;top:30pt'><mn>F</mn></mrow><span>A</span>"
            ),
            format!(
                "<div>A</div><div id=spot style='position:{position};left:100pt;top:30pt'>F</div>"
            ),
        ] {
            let actual = trace(
                &format!("<math><mtext>{child}</mtext></math><p><a href='#spot'>After</a></p>"),
                "",
            );
            let lines: Vec<_> = actual.pages.iter().flatten().collect();
            let fixed: Vec<_> = lines.iter().filter(|line| line.text == "F").collect();
            assert_eq!(
                fixed.len(),
                1,
                "{position} canonical fallback child must paint once: {lines:?}"
            );
            assert_eq!(
                lines.iter().filter(|line| line.text == "A").count(),
                1,
                "ordinary fallback content must remain selectable exactly once"
            );
            assert_eq!(
                actual.links.iter().map(Vec::len).sum::<usize>(),
                1,
                "the following link retains its canonical source"
            );
            assert_eq!(
                actual
                    .destinations
                    .iter()
                    .filter(|destination| destination.is_some())
                    .count(),
                1,
                "the positioned child retains its one real destination"
            );
        }
    }
}

#[test]
fn css_fallback_keeps_generated_counters_and_running_strings() {
    let actual = trace(
        "<math><mtext class=token><div>A</div><div style='string-set:label \"Assigned\"'></div></mtext></math><p class=after></p>",
        "@page{@top-left{content:string(label,last)}}body{counter-reset:term 7}.token::before{counter-increment:term;content:counter(term)}.after::before{content:counter(term)}",
    );
    let lines: Vec<_> = actual
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        lines.iter().filter(|text| **text == "8").count(),
        2,
        "the one Math-generated increment is read once inside fallback and once afterward: {lines:?}"
    );
    assert_eq!(
        lines.iter().filter(|text| **text == "Assigned").count(),
        1,
        "the canonical CSS fallback must still transport its string assignment to the page header: {lines:?}"
    );
    assert_eq!(lines.iter().filter(|text| **text == "A").count(), 1);
}

#[test]
fn stretchy_operators_whose_content_breaks_into_lines_lay_out_unstretched_as_css_content() {
    let lines = |stretchy: &str| -> Vec<(String, f32, f32)> {
        trace(
            &format!(
                "<math><mrow><mo stretchy={stretchy}>(</mo><mfrac><mi>x</mi><mi>y</mi></mfrac></mrow></math>"
            ),
            "mo::before{content:'a\\A b';white-space:pre}",
        )
        .pages
        .iter()
        .flatten()
        .map(|line| (line.text.clone(), line.x, line.baseline))
        .collect()
    };
    let stretchy = lines("true");
    let texts: Vec<_> = stretchy.iter().map(|(text, ..)| text.as_str()).collect();
    assert!(
        texts.contains(&"a") && texts.contains(&"b("),
        "the forced break splits the operator's content into the lines a and b( (MathML Core §3.2.1.1): {stretchy:?}"
    );
    assert_eq!(
        stretchy,
        lines("false"),
        "an operator laid out as CSS content has no glyph to stretch, so stretchy changes nothing"
    );
}
