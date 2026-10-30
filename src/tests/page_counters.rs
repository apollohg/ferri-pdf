use super::common;
use crate::graphics::DisplayItem;
use crate::render_html;
use common::{PROFILE, local_options};

/// Two 10pt lines fit each page area.
const PAGE_CSS: &str = "@page{size:300pt 60pt;margin:20pt 10pt}body{font:10pt/10pt Spike}p,h1{margin:0;font:inherit}.next{break-before:page}";

fn trace(css: &str, body: &str) -> crate::LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{PAGE_CSS}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("page counters in `{css}` with `{body}`: {error}"))
    .layout
    .unwrap()
}

/// Each page's flow and page-margin lines in painting order.
fn pages(css: &str, body: &str) -> Vec<Vec<String>> {
    trace(css, body)
        .pages
        .iter()
        .map(|lines| lines.iter().map(|line| line.text.clone()).collect())
        .collect()
}

fn expected(pages: &[&[&str]]) -> Vec<Vec<String>> {
    pages
        .iter()
        .map(|lines| lines.iter().map(|line| (*line).to_owned()).collect())
        .collect()
}

#[test]
fn page_context_counters_advance_with_each_page_box_and_pages_ignores_changes() {
    assert_eq!(
        pages(
            "@page{counter-increment:page 2 sheet;@top-left{content:counter(page) '/' counter(pages) ' s' counter(sheet)}}@page:first{counter-reset:page 10 pages 99;counter-increment:sheet pages 5}",
            "<p>A</p><p class=next>B</p><p class=next>C</p>"
        ),
        expected(&[&["A", "11/3 s1"], &["B", "13/3 s2"], &["C", "15/3 s3"]]),
        "the first page resets page to 10 and its increment, naming no page counter, keeps the implicit one; later pages' explicit increment of 2 replaces it; pages keeps the page total"
    );
}

#[test]
fn margin_contexts_keep_counters_of_their_own_that_obscure_the_page_context() {
    assert_eq!(
        pages(
            "@page{counter-increment:n 10;@top-left{content:'L' counter(n)}@top-right{counter-increment:n;content:'R' counter(n) ' P' counter(page)}}@page:nth(2){@top-right{counter-increment:n page 7}}",
            "<p>A</p><p class=next>B</p><p class=next>C</p>"
        ),
        expected(&[
            &["A", "L10", "R1 P1"],
            &["B", "L20", "R2 P7"],
            &["C", "L30", "R3 P7"]
        ]),
        "each margin location advances its own counters with every page that generates it, and its page counter obscures the page context's from then on"
    );
}

#[test]
fn reversed_page_context_counters_count_down_over_their_scope() {
    assert_eq!(
        pages(
            "@page:first{counter-reset:reversed(left)}@page{counter-increment:left -1;@top-left{content:'left ' counter(left)}}",
            "<p>A</p><p class=next>B</p><p class=next>C</p>"
        ),
        expected(&[&["A", "left 3"], &["B", "left 2"], &["C", "left 1"]]),
    );
}

#[test]
fn margin_boxes_show_document_counters_as_at_the_start_of_each_page() {
    assert_eq!(
        pages(
            "body{counter-reset:ch}h1{counter-increment:ch}h1.next{counter-increment:ch 10}p.long{white-space:pre-line;counter-increment:ch 100;orphans:1;widows:1}@page{@top-left{content:'ch ' counter(ch)}}",
            "<p>Intro</p><h1 class=next>One</h1><p>a</p><p>b</p><p class=long>l1\nl2\nl3\nl4</p><h1 class=next>Two</h1>"
        ),
        expected(&[
            &["Intro", "ch 0"],
            &["One", "a", "ch 0"],
            &["b", "l1", "ch 10"],
            &["l2", "l3", "ch 110"],
            &["l4", "ch 110"],
            &["Two", "ch 110"],
        ]),
        "a page starting at an element precedes its own changes; a page starting inside one follows them"
    );
}

#[test]
fn document_content_shows_the_page_counters_of_the_page_it_lies_on() {
    assert_eq!(
        pages(
            "@page{counter-increment:page 3}.n::after{content:' p' counter(page) '/' counter(pages,upper-roman)}.ref::after{content:' to ' target-counter('#last',page)}",
            "<p class=n>A</p><p class=ref>R</p><p class=next>B</p><p class='next n' id=last>C</p>"
        ),
        expected(&[&["A p3/III", "R to 9"], &["B"], &["C p9/III"]]),
    );
}

#[test]
fn bookmark_labels_show_the_page_counter_of_their_element() {
    let html = format!(
        "<style>{PROFILE}{PAGE_CSS}@page{{counter-increment:page 5}}h1{{bookmark-label:content(text) ' ' counter(page)}}</style><h1>First</h1><h1 class=next>Second</h1>"
    );
    let document = crate::document::StyledDocument::parse(
        &html,
        &local_options().base,
        &[],
        &crate::resources::FileResolver::default(),
    )
    .unwrap();
    let (document, _) =
        crate::layout::layout_document(document, &crate::resources::FileResolver::default())
            .unwrap();
    assert_eq!(
        document
            .navigation()
            .bookmarks
            .iter()
            .map(|bookmark| bookmark.label.as_str())
            .collect::<Vec<_>>(),
        ["First 5", "Second 10"]
    );
}

/// Each page's flow lines and the lines its footnote area paints.
fn calls_and_notes(css: &str, body: &str) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let trace = trace(css, body);
    let calls = trace
        .pages
        .iter()
        .map(|lines| lines.iter().map(|line| line.text.clone()).collect())
        .collect();
    let notes = trace
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
        .collect();
    (calls, notes)
}

#[test]
fn page_context_footnote_counter_resets_renumber_footnotes_by_call_page() {
    let (calls, notes) = calls_and_notes(
        "@page{size:300pt 80pt;counter-reset:footnote}.fn{float:footnote}",
        "<p>x<span class=fn>N1</span> y<span class=fn>N2</span></p><p class=next>z<span class=fn>N3</span></p>",
    );
    assert_eq!(calls, expected(&[&["x1 y2"], &["z1"]]));
    assert_eq!(
        notes,
        expected(&[&["1. N1", "2. N2"], &["1. N3"]]),
        "calls and markers share the value the page start resets"
    );
}

#[test]
fn reversed_page_context_footnote_resets_count_down_the_calls_of_each_page() {
    let (calls, notes) = calls_and_notes(
        "@page{size:300pt 100pt;counter-reset:reversed(footnote)}.fn{float:footnote;counter-increment:footnote -1}",
        "<p>x<span class=fn>N1</span> y<span class=fn>N2</span></p><p class=next>z<span class=fn>N3</span></p><p class=next>w<span class=fn>N4</span> v<span class=fn>N5</span> u<span class=fn>N6</span></p>",
    );
    assert_eq!(
        calls,
        expected(&[&["x2 y1"], &["z1"], &["w3 v2 u1"]]),
        "each page's reset takes the automatic initial value of a reversed counter over the updates up to the next page's reset"
    );
    assert_eq!(
        notes,
        expected(&[
            &["2. N1", "1. N2"],
            &["1. N3"],
            &["3. N4", "2. N5", "1. N6"]
        ])
    );
}
