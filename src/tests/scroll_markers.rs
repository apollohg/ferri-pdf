//! CSS Overflow 5 §3.1: scroll marker groups and the scroll marker pseudo-classes, as a
//! static rendering at the initial scroll position shows them.
use super::common::{PROFILE, local_options};
use crate::render_html;

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 128, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const BLACK: [u8; 3] = [0, 0, 0];

/// Every text run of every page as (text, colour).
fn runs(css: &str, body: &str) -> Vec<(String, [u8; 3])> {
    let trace = render_html(
        &format!("<style>{PROFILE}@page{{size:400pt 400pt;margin:20pt}}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("render failed: {error}\ncss: {css}\nbody: {body}"))
    .layout
    .expect("layout requested");
    trace
        .pages
        .iter()
        .flatten()
        .flat_map(|line| line.runs.iter())
        .map(|run| (run.text.trim().to_owned(), run.color.rgb))
        .filter(|(text, _)| !text.is_empty())
        .collect()
}

fn colour(runs: &[(String, [u8; 3])], text: &str) -> [u8; 3] {
    runs.iter()
        .find(|(run, _)| run == text)
        .unwrap_or_else(|| panic!("no run {text:?} among {runs:#?}"))
        .1
}

const MARKER_STATES: &str = "a:target-before{color:rgb(0,128,0)}a:target-current{color:red}\
    a:target-after{color:blue}";

#[test]
fn a_table_of_contents_marks_the_section_at_the_top_of_the_document_current() {
    let runs = runs(
        &format!(
            "ol{{scroll-target-group:auto}}{MARKER_STATES}.chapter{{height:200pt}}\
             a.outside{{color:black}}"
        ),
        "<ol><li><a href='#intro'>Intro</a></li><li><a href='#one'>One</a></li>\
         <li><a href='#two'>Two</a></li></ol><a class=outside href='#one'>Outside</a>\
         <div id=intro class=chapter>Introduction</div><div id=one class=chapter>Chapter one</div>\
         <div id=two class=chapter>Chapter two</div>",
    );
    assert_eq!(
        ["Intro", "One", "Two", "Outside"].map(|text| colour(&runs, text)),
        [RED, BLUE, BLUE, BLACK],
        "§3.1.8: at the initial scroll position the first section is the one the viewport is \
         at, so its marker matches :target-current and those after it :target-after; a link \
         outside the scroll-target-group container is no scroll marker: {runs:#?}"
    );
}

#[test]
fn scroll_padding_and_scroll_margin_move_the_targets_scrolling_into_view_aligns() {
    let scenario = |extra: &str| {
        let runs = runs(
            &format!(
                "nav{{scroll-target-group:auto}}{MARKER_STATES}\
                 .scroller{{overflow:auto;height:100pt}}.scroller p{{height:20pt;margin:0}}{extra}"
            ),
            "<nav><a href='#a'>First</a> <a href='#b'>Second</a> <a href='#c'>Third</a></nav>\
             <div class=scroller><p id=a>A</p><p id=b>B</p><p id=c>C</p></div>",
        );
        ["First", "Second", "Third"].map(|text| colour(&runs, text))
    };
    assert_eq!(
        scenario(""),
        [RED, BLUE, BLUE],
        "the first target already lies at the scrollport's start"
    );
    assert_eq!(
        scenario(".scroller{scroll-padding-top:30pt}"),
        [GREEN, RED, BLUE],
        "30pt of scroll padding puts the first two targets before the initial scroll position \
         of a container whose contents fit, and the later of them is active"
    );
    assert_eq!(
        scenario(".scroller{scroll-padding-top:30pt}#b{scroll-margin-top:-15pt}"),
        [RED, BLUE, BLUE],
        "a negative scroll margin shrinks the second target's area past the initial position"
    );
}

#[test]
fn scroll_marker_pseudo_elements_take_the_scroll_marker_pseudo_classes() {
    let runs = runs(
        ".carousel{overflow:auto;height:60pt;scroll-marker-group:after}\
         .carousel li{height:50pt}.carousel li::scroll-marker{content:'M' counter(list-item)}\
         ::scroll-marker:target-after{color:rgb(0,128,0)}\
         li::scroll-marker{color:blue}\
         ::scroll-marker:target-current{color:red}",
        "<ol class=carousel><li>One</li><li>Two</li><li>Three</li></ol>",
    );
    assert_eq!(
        ["M1", "M2", "M3"].map(|text| colour(&runs, text)),
        [RED, GREEN, GREEN],
        "the first slide's marker is active; the pseudo-class outweighs the type selector \
         declared after it: {runs:#?}"
    );
}

#[test]
fn only_scroll_markers_take_the_scroll_marker_pseudo_classes_after_a_pseudo_element() {
    for (selector, valid) in [
        ("::scroll-marker:target-current", true),
        ("li::scroll-marker:not(:target-before)", true),
        ("::scroll-marker:hover", true),
        ("::scroll-marker-group:focus-within", true),
        ("::scroll-marker-group:target-current", false),
        ("::before:target-current", false),
        ("::scroll-marker:first-child", false),
        ("::scroll-marker:target-current .x", false),
    ] {
        let runs = runs(
            &format!("p{{color:red}}@supports selector({selector}){{p{{color:blue}}}}"),
            "<p>Probe</p>",
        );
        assert_eq!(
            colour(&runs, "Probe") == BLUE,
            valid,
            "selector({selector}) must {}parse",
            if valid { "" } else { "not " }
        );
    }
}
