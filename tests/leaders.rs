mod common;
use common::{PROFILE, local_options};

fn render(css: &str, body: &str) -> ferri_pdf::LayoutTrace {
    ferri_pdf::render_html(
        &format!("<style>{PROFILE}@page{{size:300pt 300pt;margin:36pt}}p{{margin:0;width:200pt;font:10pt/15pt Spike}}{css}</style>{body}"),
        &local_options(),
    )
    .expect("leaders must fit through the shared inline layout")
    .layout
    .unwrap()
}

#[test]
fn leader_keywords_and_custom_patterns_repeat_between_their_neighbours() {
    for (value, pattern) in [
        ("dotted", "."),
        ("solid", "_"),
        ("space", " "),
        ("'ab'", "ab"),
        ("'x\\a y'", "xy"),
    ] {
        let trace = render(
            &format!("p::after{{content:leader({value}) 'Z'}}"),
            "<p>A</p>",
        );
        assert_eq!(trace.pages[0].len(), 1, "{value} must stay on one line");
        let line = &trace.pages[0][0];
        let text: String = line.runs.iter().map(|run| run.text.as_str()).collect();
        assert!(
            text.starts_with('A') && text.ends_with('Z'),
            "{value}: {text:?}"
        );
        assert!(
            text.matches(pattern).count() > 2,
            "{value} did not expand: {text:?}"
        );
    }
}

#[test]
fn leader_end_phase_is_independent_of_the_before_content_width() {
    let trace = render("p::after{content:leader('.') 'Z'}", "<p>A</p><p>AAAA</p>");
    assert_eq!(trace.pages[0].len(), 2);
    let dots = |index: usize| {
        trace.pages[0][index]
            .runs
            .iter()
            .filter(|run| run.text.contains('.'))
            .map(|run| run.x)
            .collect::<Vec<_>>()
    };
    let first = dots(0);
    let second = dots(1);
    assert!(!first.is_empty() && !second.is_empty());
    common::assert_pt(*first.last().unwrap(), *second.last().unwrap());
}

#[test]
fn wrapped_before_content_keeps_the_leader_on_its_final_line() {
    let trace = render(
        "p{width:60pt}p::after{content:leader('.') 'Z'}",
        "<p>Before content wraps across several lines</p>",
    );
    assert_eq!(trace.pages[0].len(), 6);
    for line in &trace.pages[0][..5] {
        assert!(
            !line.text.contains('.'),
            "leader appeared before the before content finished: {}",
            line.text
        );
    }
    assert!(trace.pages[0][5].text.starts_with("lines."));
    assert!(trace.pages[0][5].text.ends_with('Z'));
}

#[test]
fn leader_phase_must_leave_one_complete_custom_pattern() {
    let trace = render(
        "p{width:60pt}p::after{content:leader('abcd') 'Z'}",
        "<p>AAA</p>",
    );
    assert_eq!(
        trace.pages[0].len(),
        2,
        "a partial pattern cannot satisfy the minimum full copy"
    );
    assert_eq!(trace.pages[0][0].text, "AAA");
    assert!(
        trace.pages[0][1].text.contains("abcd"),
        "{}",
        trace.pages[0][1].text
    );
}

#[test]
fn isolated_leader_does_not_create_a_line_of_its_own() {
    let trace = render("p::after{content:leader('.')}", "<p></p>");
    assert!(
        trace
            .pages
            .iter()
            .flatten()
            .all(|line| !line.text.contains('.')),
        "a leader cannot be the only content on its line"
    );
}

#[test]
fn leaders_keep_rtl_source_text_and_end_alignment() {
    let trace = render(
        "p{direction:rtl}p::after{content:leader('.') 'Z'}",
        "<p>A<span>B</span></p>",
    );
    let line = &trace.pages[0][0];
    assert!(
        line.text.starts_with("AB"),
        "logical trace text must remain in source order: {}",
        line.text
    );
    assert!(line.text.ends_with('Z'));
}

#[test]
fn multiple_leaders_share_available_space_without_changing_source_order() {
    let trace = render(
        "p::after{content:leader('.') 'M' leader('_') 'Z'}",
        "<p>A</p>",
    );
    assert_eq!(trace.pages[0].len(), 1);
    let line = &trace.pages[0][0];
    common::assert_pt(line.width, 200.0);
    let (before, after) = line.text.split_once('M').unwrap();
    assert!(
        before.starts_with('A') && before.matches('.').count() > 2,
        "{}",
        line.text
    );
    assert!(
        after.ends_with('Z') && after.matches('_').count() > 2,
        "{}",
        line.text
    );
}

#[test]
fn leaders_in_page_margins_use_the_same_fitting_pipeline() {
    let trace = render(
        "@page{@top-center{width:200pt;content:'A' leader('.') 'Z';font:10pt/15pt Spike}}",
        "<p>Body</p>",
    );
    let line = trace.pages[0]
        .iter()
        .find(|line| line.text.starts_with("A."))
        .expect("margin leader");
    common::assert_pt(line.width, 200.0);
    assert!(line.text.ends_with('Z') && line.text.matches('.').count() > 2);
}

#[test]
fn empty_patterns_and_zero_size_do_not_loop_or_create_nonfinite_geometry() {
    for css in [
        "p::after{content:leader('') 'Z'}",
        "p::after{font-size:0;content:leader('.') 'Z'}",
    ] {
        let trace = render(css, "<p>A</p>");
        assert!(trace.pages[0].iter().all(|line| line.width.is_finite()));
    }
}

#[test]
fn decorated_leader_alone_keeps_its_box_without_painting_a_leader() {
    let trace = render("p::after{border:1pt solid;content:leader('.')}", "<p></p>");
    assert!(
        trace
            .pages
            .iter()
            .flatten()
            .all(|line| !line.text.contains('.'))
    );
}

#[test]
fn expanded_leader_links_cover_the_complete_inline_box() {
    let trace = render(
        "a::after{content:leader('.') 'Z'}",
        "<p><a href='https://example.org'>A</a></p>",
    );
    assert_eq!(trace.links[0].len(), 1);
    assert!(
        matches!(&trace.links[0][0].shape, ferri_pdf::LinkShape::Rect(rect) if rect.width > 190.0),
        "leader must remain within the hyperlink's expanded box"
    );
}

#[test]
fn leaders_with_multiple_script_runs_repeat_the_complete_pattern() {
    let trace = render("p::after{content:leader('aβ') 'Z'}", "<p>A</p>");
    let text = &trace.pages[0][0].text;
    assert!(
        text.matches("aβ").count() > 2,
        "pattern runs were reordered: {text}"
    );
}

#[test]
fn huge_leader_expansion_fails_before_allocating_repetitions() {
    let html = format!(
        "<style>{PROFILE}p{{width:200pt;font:10pt Spike}}p::after{{font-size:0.000001pt;content:leader('.') 'Z'}}</style><p>A</p>"
    );
    let error = ferri_pdf::render_html(&html, &local_options())
        .err()
        .expect("the expansion must exceed its text bound");
    assert!(
        matches!(
            error,
            ferri_pdf::Error::Layout(ref message) if *message == ferri_pdf_text::TextError::LeaderExpansionLimit { limit: ferri_pdf_document::MAX_INLINE_TEXT_BYTES }.to_string()
        ),
        "{error:?}"
    );
}

#[test]
fn nonadvancing_leaders_do_not_claim_unused_line_width() {
    let plain = render("", "<p>A</p>");
    let zero = render("p::after{font-size:0;content:leader('.') 'Z'}", "<p>A</p>");
    common::assert_pt(zero.pages[0][0].width, plain.pages[0][0].width);
    let plain = render("p::after{letter-spacing:-4pt;content:'.Z'}", "<p>A</p>");
    let negative = render(
        "p::after{letter-spacing:-4pt;content:leader('.') 'Z'}",
        "<p>A</p>",
    );
    common::assert_pt(negative.pages[0][0].width, plain.pages[0][0].width);
}

#[test]
fn rtl_leaders_place_after_content_at_the_left_edge() {
    let trace = render(
        "p{direction:rtl}p::after{content:leader('.') 'ב'}",
        "<p>א</p>",
    );
    let line = &trace.pages[0][0];
    assert!(
        line.text.starts_with('א') && line.text.ends_with('ב'),
        "{}",
        line.text
    );
    let after = line.runs.iter().find(|run| run.text == "ב").unwrap();
    common::assert_pt(after.x, 0.0);
    let before = line.runs.iter().find(|run| run.text == "א").unwrap();
    common::assert_pt(before.x + before.width, 200.0);
}

#[test]
fn leader_fitting_uses_the_float_exclusion_width() {
    let trace = render(
        ".float{float:left;width:60pt;height:30pt}p::after{content:leader('.') 'Z'}",
        "<div class=float></div><p>A</p>",
    );
    let line = trace.pages[0]
        .iter()
        .find(|line| line.text.starts_with('A'))
        .unwrap();
    common::assert_pt(line.width, 140.0);
    common::assert_pt(line.x, 96.0);
}

#[test]
fn whitespace_after_a_leader_does_not_strand_it_on_a_line_alone() {
    let trace = render(
        "p{width:10pt}p::after{content:leader('.') ' ZZ'}",
        "<p>A</p>",
    );
    assert_eq!(
        trace.pages[0].len(),
        2,
        "a leader-only intermediate line is forbidden"
    );
    assert_eq!(trace.pages[0][0].text, "A");
    assert_eq!(trace.pages[0][1].text, ". ZZ");
}

#[test]
fn emergency_wrapping_also_keeps_a_leader_with_after_content() {
    let trace = render(
        "p{width:10pt;overflow-wrap:anywhere}p::after{content:leader('.') ' ZZ'}",
        "<p>A</p>",
    );
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["A", ". Z", "Z"]
    );
}
