mod common;
use common::{PROFILE, assert_pt, local_options};
use ferri_pdf::{LayoutTrace, render_html};

const HYPHEN: &str = "\u{2010}";
fn trace(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<!doctype html><style>{PROFILE}p{{margin:0}}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap()
    .layout
    .unwrap()
}

#[test]
fn manual_soft_hyphens_are_visible_only_at_selected_breaks() {
    let narrow = trace("p{width:45pt;hyphens:manual}", "<p>ex&shy;ample</p>");
    assert_eq!(
        narrow.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        [format!("ex{HYPHEN}"), "ample".into()]
    );
    assert!(narrow.pages[0][0].runs.iter().any(|run| run.text == HYPHEN));
    let wide = trace(
        "p{width:160pt;hyphens:manual}",
        "<p>ex&shy;ample</p><p>example</p>",
    );
    assert_eq!(wide.pages[0][0].text, "example");
    assert_pt(wide.pages[0][0].width, wide.pages[0][1].width);
    assert!(!wide.pages[0][0].runs.iter().any(|run| run.text == HYPHEN));
}

#[test]
fn none_suppresses_discretionary_hyphenation_but_not_visible_hyphens() {
    let trace = trace(
        "p{width:45pt;hyphens:none}",
        "<p>ex&shy;ample</p><p>example</p><p>ex-ample</p>",
    );
    assert_eq!(trace.pages[0][0].text, "example");
    assert_eq!(trace.pages[0][1].text, "example");
    assert_pt(trace.pages[0][0].width, trace.pages[0][1].width);
    assert_eq!(trace.pages[0][2].text, "ex-");
    assert_eq!(trace.pages[0][3].text, "ample");
}

#[test]
fn discretionary_glyph_uses_its_own_span_and_bidi_position() {
    let styled = trace(
        "p{width:45pt;hyphens:manual}",
        "<p>ex<span style='font-size:16pt;color:green'>&shy;</span>ample</p>",
    );
    let hyphen = styled.pages[0][0]
        .runs
        .iter()
        .find(|run| run.text == HYPHEN)
        .expect("soft hyphen creates a styled glyph even when its span contains no other text");
    assert_pt(hyphen.font_size, 16.0);
    assert_eq!(hyphen.color.rgb, [0, 128, 0]);
    let rtl = trace(
        "p{width:25pt;hyphens:manual;direction:rtl}",
        "<p>אב&shy;גד</p>",
    );
    assert_eq!(
        rtl.pages[0].len(),
        2,
        "first line {:?}, width {}",
        rtl.pages[0][0].text,
        rtl.pages[0][0].width
    );
    assert_eq!(
        rtl.pages[0][0].runs[0].text, HYPHEN,
        "the hyphen sits at the visual left edge of an RTL word"
    );
}

#[test]
fn conditional_hyphen_stays_with_an_ltr_word_in_an_rtl_paragraph() {
    let trace = trace(
        "p{width:45pt;hyphens:manual;direction:rtl}",
        "<p>ex<span style='color:green'>&shy;</span>ample</p>",
    );
    let first = &trace.pages[0][0];
    let hyphen = first.runs.iter().find(|run| run.text == HYPHEN).unwrap();
    let word = first
        .runs
        .iter()
        .find(|run| run.text.contains("ex"))
        .unwrap();
    assert!(
        hyphen.x >= word.x + word.width,
        "LTR word at {}pt + {}pt must precede its hyphen at {}pt",
        word.x,
        word.width,
        hyphen.x
    );
}

#[test]
fn hyphenation_participates_in_intrinsic_sizes_and_css_wide_defaults() {
    let trace = trace(
        ".row{display:flex;width:50pt}.word{flex:1;hyphens:initial}.fixed{width:10pt;flex:none}",
        "<div class=row><div class=word>ex&shy;ample</div><div class=fixed>X</div></div>",
    );
    assert!(
        trace.pages[0]
            .iter()
            .any(|line| line.text == format!("ex{HYPHEN}"))
    );
    assert!(trace.pages[0].iter().any(|line| line.text == "ample"));
    for line in &trace.pages[0] {
        assert!(
            line.x + line.width <= 86.001,
            "content {:?} at {}pt with width {}pt must fit the 50pt row beginning at 36pt",
            line.text,
            line.x,
            line.width
        );
    }
}
