use super::common;
use crate::graphics::DisplayItem;
use crate::render_html;
use common::{PROFILE, local_options};

const PAGE_CSS: &str = "@page{size:400pt 200pt;margin:40pt;@top-right{content:'S[' string(inner,last) ']'}}p{margin:0;height:120pt}";

fn render(css: &str, body: &str) -> Result<crate::RenderOutput, crate::Error> {
    render_html(
        &format!("<style>{PROFILE}{PAGE_CSS}{css}</style>{body}"),
        &local_options(),
    )
}

/// The text of the lines each page paints inside display frames, where margin boxes place
/// running elements.
fn frame_text(trace: &crate::LayoutTrace) -> Vec<Vec<String>> {
    trace
        .display_lists
        .iter()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| match item {
                    DisplayItem::Frame(frame) => {
                        Some(frame.lines.iter().map(|line| line.text.clone()))
                    }
                    _ => None,
                })
                .flatten()
                .collect()
        })
        .collect()
}

#[test]
fn running_contents_assign_strings_at_the_element_position_and_leave_the_flow() {
    let trace = render(
        "@page{@top-left{content:element(head)}}h1{position:running(head);font:10pt Spike;margin:0}span{string-set:inner content(text)}p.short{height:60pt}",
        "<p class=short>First</p><h1>Head <span>Inner</span></h1><p>Second</p><p>Third</p>",
    )
    .unwrap()
    .layout
    .unwrap();
    let texts: Vec<Vec<&str>> = trace
        .pages
        .iter()
        .map(|page| page.iter().map(|line| line.text.as_str()).collect())
        .collect();
    assert_eq!(
        texts,
        [
            vec!["First", "Second", "S[Inner]"],
            vec!["Third", "S[Inner]"]
        ],
        "the running element paints only in margins and its descendants assign strings on its page"
    );
    assert_eq!(
        frame_text(&trace),
        vec![vec!["Head Inner".to_owned()]; 2],
        "later pages take the entry value"
    );
}

#[test]
fn element_outside_page_margins_takes_the_initial_content() {
    let trace = render(
        "p.label::before{content:element(head)}p.replaced{content:element(head)}",
        "<p class=label>Label</p><p class=replaced>Replaced</p>",
    )
    .unwrap()
    .layout
    .unwrap();
    let body: Vec<&str> = trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .filter(|text| !text.starts_with("S["))
        .collect();
    assert_eq!(
        body,
        ["Label", "Replaced"],
        "element() is invalid at computed-value time outside page-margin boxes"
    );
}

#[test]
fn running_before_and_after_leave_the_flow_and_assign_at_their_own_positions() {
    let trace = render(
        "@page{@top-left{content:element(head)}@bottom-left{content:element(tail,start)}}body{counter-reset:ch}h1{counter-increment:ch;font:10pt Spike;margin:0}h1::before{position:running(head);content:'Chapter ' counter(ch)}h1::after{position:running(tail);content:'End ' counter(ch)}h1.last::after{content:none}p.short{height:60pt}",
        "<p class=short>First</p><h1>One</h1><p>Second</p><h1 class=last>Two</h1><p>Third</p>",
    )
    .unwrap()
    .layout
    .unwrap();
    let flow: Vec<Vec<&str>> = trace
        .pages
        .iter()
        .map(|page| {
            page.iter()
                .map(|line| line.text.as_str())
                .filter(|text| !text.starts_with("S["))
                .collect()
        })
        .collect();
    assert_eq!(
        flow,
        [vec!["First", "One", "Second"], vec!["Two", "Third"]],
        "running pseudo-elements generate no flow content"
    );
    assert_eq!(
        frame_text(&trace),
        [
            vec!["Chapter 1".to_owned()],
            vec!["Chapter 2".to_owned(), "End 1".to_owned()]
        ],
        "::after of One assigns on page 1 after its start, so page 1 has no start value and page 2 enters with it"
    );
}

#[test]
fn copies_of_running_elements_show_the_page_counters_of_each_page() {
    let trace = render(
        "@page{@top-left{content:element(head)}@bottom-left{content:element(foot)}}@page:nth(3){counter-increment:page 5;@bottom-left{counter-reset:page 40}}h1{position:running(head);font:10pt Spike;margin:0}h1::after{content:' ' counter(page) '/' counter(pages)}p.f::before{position:running(foot);content:'P' counter(page) ' of ' counter(pages,upper-roman)}",
        "<h1>Head</h1><p class=f>One</p><p>Two</p><p>Three</p>",
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        frame_text(&trace),
        [
            vec!["Head 1/3".to_owned(), "P1 of III".to_owned()],
            vec!["Head 2/3".to_owned(), "P2 of III".to_owned()],
            vec!["Head 7/3".to_owned(), "P40 of III".to_owned()],
        ],
        "each page's copy of a running element or pseudo-element shows the page counter its margin box sees"
    );
}

#[test]
fn copies_of_running_elements_keep_their_counters_and_strings_once() {
    let trace = render(
        "@page{@top-left{content:element(head)}}body{counter-reset:n}h1{position:running(head);font:10pt Spike;margin:0;counter-increment:n}h1::before{content:counter(n) '.' counter(page) ' '}h1 span{string-set:inner content(text)}p.after::before{content:'n=' counter(n)}",
        "<h1>Head <span>Inner</span></h1><p class=after>First</p><p>Second</p>",
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        frame_text(&trace),
        [
            vec!["1.1 Head Inner".to_owned()],
            vec!["1.2 Head Inner".to_owned()]
        ],
        "copies format from the counters at the running element's start"
    );
    let flow: Vec<Vec<&str>> = trace
        .pages
        .iter()
        .map(|page| page.iter().map(|line| line.text.as_str()).collect())
        .collect();
    assert_eq!(
        flow,
        [vec!["n=1First", "S[Inner]"], vec!["Second", "S[Inner]"]],
        "formatting copies changes neither the document's counters nor its strings"
    );
}

/// The background colours of the boxes each page paints inside display frames.
fn frame_backgrounds(trace: &crate::LayoutTrace) -> Vec<Vec<[u8; 3]>> {
    trace
        .display_lists
        .iter()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| match item {
                    DisplayItem::Frame(frame) => Some(frame.items.iter()),
                    _ => None,
                })
                .flatten()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                        Some(paint.decoration.background.rgb)
                    }
                    _ => None,
                })
                .collect()
        })
        .collect()
}

#[test]
fn copies_of_running_elements_paint_their_own_backgrounds() {
    // CSS Backgrounds 3 §2.11.2: only the root element and the body give their backgrounds to
    // the canvas; a running element shown with each page's counters paints its own.
    const AQUA: [u8; 3] = [0, 255, 255];
    let trace = render(
        "@page{@top-left{content:element(head)}}h1{position:running(head);font:10pt Spike;margin:0;background:aqua}h1::after{content:' ' counter(page)}",
        "<h1>Head</h1><p>First</p><p>Second</p>",
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        frame_text(&trace),
        [vec!["Head 1".to_owned()], vec!["Head 2".to_owned()]],
        "each page shows a copy with its own page number"
    );
    assert_eq!(
        frame_backgrounds(&trace),
        [vec![AQUA], vec![AQUA]],
        "each copy paints the running element's background"
    );
}
