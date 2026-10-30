use super::common;
use crate::graphics::DisplayItem;
use crate::render_html;
use common::{PROFILE, local_options};

const PAGE_CSS: &str =
    "@page{size:300pt 120pt;margin:10pt}body{font:10pt/10pt Spike}p{margin:0}.fn{float:footnote}";

fn render(css: &str, body: &str) -> Result<crate::RenderOutput, crate::Error> {
    render_html(
        &format!("<style>{PROFILE}{PAGE_CSS}{css}</style>{body}"),
        &local_options(),
    )
}

/// Each page's flow lines and the lines its footnote area paints.
fn pages(trace: &crate::LayoutTrace) -> Vec<(Vec<String>, Vec<String>)> {
    trace
        .pages
        .iter()
        .zip(&trace.display_lists)
        .map(|(lines, items)| {
            (
                lines.iter().map(|line| line.text.clone()).collect(),
                items
                    .iter()
                    .filter_map(|item| match item {
                        DisplayItem::Frame(frame) => {
                            Some(frame.lines.iter().map(|line| line.text.clone()))
                        }
                        _ => None,
                    })
                    .flatten()
                    .collect(),
            )
        })
        .collect()
}

fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn footnote_counter_is_predefined_incremented_per_footnote_and_shared_by_call_and_marker() {
    let trace = render(
        "::footnote-call{content:'[' counter(footnote) ']'}::footnote-marker{content:counter(footnote,lower-roman) ') '}.skip{counter-increment:footnote 5}body{counter-reset:chapter}",
        "<p>A<span class=fn>One</span> B<span class='fn skip'>Two</span> C<span class=fn>Three</span></p>",
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        pages(&trace),
        [(
            texts(&["A[1] B[6] C[7]"]),
            texts(&["i) One", "vi) Two", "vii) Three"])
        )],
        "an author reset of other counters keeps the root's footnote counter; an explicit increment replaces the implicit one"
    );
}

#[test]
fn footnotes_left_over_at_the_end_take_pages_of_their_own_without_max_height() {
    let note = (1..=12)
        .map(|index| format!("N{index}"))
        .collect::<Vec<_>>()
        .join("<br>");
    let trace = render(
        "@page{@footnote{max-height:30pt}}",
        &format!("<p>Body<span class=fn>{note}</span></p>"),
    )
    .unwrap()
    .layout
    .unwrap();
    let pages = pages(&trace);
    assert_eq!(pages.len(), 2, "{pages:?}");
    assert_eq!(pages[0].0, texts(&["Body1"]));
    assert_eq!(
        pages[1].0,
        Vec::<String>::new(),
        "the extra page has no flow"
    );
    // Each page paints the whole body and clips it to the part it holds.
    assert_eq!(pages[0].1.len(), 12);
    assert_eq!(pages[1].1.len(), 12);
}

#[test]
fn float_footnote_applies_only_where_floats_do() {
    let trace = render(
        ".flex{display:flex}.abs{position:absolute;top:50pt}",
        "<div class=flex><span class=fn>Item</span></div><p>Text<span class='fn abs'>Positioned</span></p>",
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        pages(&trace),
        [(texts(&["Item", "Text", "Positioned"]), Vec::new())],
        "flex items and absolutely positioned boxes are not footnotes"
    );
}

#[test]
fn inline_footnote_display_is_reported() {
    let error = render(
        ".fn{footnote-display:inline}",
        "<p>Text<span class=fn>Note</span></p>",
    )
    .err()
    .expect("inline footnotes must not render as block footnotes");
    assert!(
        error.to_string().contains("footnote-display: inline"),
        "unexpected error: {error}"
    );
}
