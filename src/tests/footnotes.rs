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

const INLINE_CSS: &str = ".fn{footnote-display:inline}.fn::after{content:' '}";

#[test]
fn inline_footnotes_share_the_lines_of_one_inline_formatting_context() {
    let trace = render(
        INLINE_CSS,
        "<div>A<span class=fn>One</span> B<div class=fn>Two</div> C<span class=fn>Three</span></div>",
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        pages(&trace),
        [(texts(&["A1 B2 C3"]), texts(&["1. One 2. Two 3. Three"]))],
        "consecutive inline footnotes, block elements among them, flow on one line"
    );
}

/// CSS 2.1 §9.2.1.1: the anonymous block holding inline footnotes inherits from the footnote
/// area enclosing it, which each page styles, not from the footnotes' parent element.
#[test]
fn inline_footnote_lines_inherit_from_the_footnote_area_of_their_page() {
    const AREA_WIDTH: f32 = 280.0;
    let trace = render(
        &format!(
            "{INLINE_CSS}p{{text-align:center;line-height:10pt}}\
             @page{{@footnote{{line-height:20pt}}}}\
             @page:first{{@footnote{{text-align:right;line-height:30pt}}}}"
        ),
        "<p>A<span class=fn>One</span></p><p style=break-before:page>B<span class=fn>Two</span></p>",
    )
    .unwrap()
    .layout
    .unwrap();
    let line = |page: usize| {
        let lines: Vec<_> = trace.display_lists[page]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Frame(frame) => Some(&frame.lines),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(lines.len(), 1, "page {page} footnote area lines: {lines:?}");
        lines[0]
    };
    let (first, second) = (line(0), line(1));
    assert!(
        (first.x + first.width - AREA_WIDTH).abs() < 0.01 && (first.height - 30.0).abs() < 0.01,
        "the first page's area aligns its line right on a 30pt strut: {first:?}"
    );
    assert!(
        second.x.abs() < 0.01 && (second.height - 20.0).abs() < 0.01,
        "the second page's area aligns its line to the start on a 20pt strut: {second:?}"
    );
}

#[test]
fn a_block_footnote_between_inline_footnotes_ends_their_line() {
    let trace = render(
        &format!("{INLINE_CSS}.block{{footnote-display:block}}"),
        "<p>A<span class=fn>One</span> B<span class='fn block'>Two</span> C<span class=fn>Three</span> D<span class=fn>Four</span></p>",
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        pages(&trace)[0].1,
        texts(&["1. One", "2. Two", "3. Three 4. Four"]),
        "only inline footnotes adjacent in the area share lines"
    );
}

#[test]
fn inline_footnotes_on_later_pages_start_their_own_lines_after_their_calls() {
    let body: String = (1..=14)
        .map(|index| format!("<p>P{index}<span class=fn>N{index}</span></p>"))
        .collect();
    let trace = render(INLINE_CSS, &body).unwrap().layout.unwrap();
    let pages = pages(&trace);
    let page_of = |text: &str, notes: bool| {
        pages
            .iter()
            .position(|(lines, area)| {
                (if notes { area } else { lines })
                    .iter()
                    .any(|line| line.split(' ').any(|word| word == text))
            })
            .unwrap_or_else(|| panic!("{text} is not rendered: {pages:?}"))
    };
    for index in 1..=14 {
        let call = page_of(&format!("P{index}{index}"), false);
        let note = page_of(&format!("N{index}"), true);
        assert!(
            note >= call,
            "footnote {index} lies on page {note} before its call on page {call}: {pages:?}"
        );
    }
    for (page, (_, area)) in pages.iter().enumerate() {
        let notes = area
            .iter()
            .flat_map(|line| line.split(' '))
            .filter(|word| word.starts_with('N'))
            .count();
        assert!(
            notes < 2 || area.len() < notes,
            "the {notes} footnotes of page {page} share lines: {pages:?}"
        );
    }
}

#[test]
fn inline_footnotes_keep_their_policies() {
    let trace = render(
        &format!("{INLINE_CSS}.fn{{footnote-policy:line}}p{{widows:1;orphans:1}}"),
        &format!(
            "<p style=height:70pt>Top</p><p>First<br>Second<span class=fn>{TALL_NOTE}</span></p>"
        ),
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        pages(&trace),
        [
            (texts(&["Top", "First"]), Vec::new()),
            (texts(&["Second1"]), texts(&["1. A", "B", "C", "D"]))
        ],
        "the call line moves with its whole inline footnote"
    );
}

#[test]
fn calls_inside_tables_columns_and_positioned_boxes_place_footnotes_on_their_page() {
    let trace = render(
        "td{padding:0}table{border-spacing:0}.cols{columns:2;column-gap:10pt}.abs{position:absolute;top:60pt}",
        "<table><tr><td>Cell<span class=fn>CellNote</span></td></tr></table><div class=cols>Col<span class=fn>ColNote</span></div><div class=abs>Abs<span class=fn>AbsNote</span></div><p>After</p>",
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        pages(&trace),
        [(
            texts(&["Cell1", "Col2", "After", "Abs3"]),
            texts(&["1. CellNote", "2. ColNote", "3. AbsNote"])
        )],
        "bodies of calls in nested layouts join the area of the page holding them"
    );
}

#[test]
fn calls_inside_rows_split_across_pages_keep_their_footnotes_off_earlier_pages() {
    let lines = (1..=12)
        .map(|index| format!("L{index}"))
        .collect::<Vec<_>>()
        .join("<br>");
    let trace = render(
        "td{padding:0}table{border-spacing:0}",
        &format!("<table><tr><td>{lines}<span class=fn>Note</span></td></tr></table>"),
    )
    .unwrap()
    .layout
    .unwrap();
    let notes: Vec<_> = pages(&trace).into_iter().map(|(_, notes)| notes).collect();
    assert_eq!(
        notes,
        [Vec::new(), texts(&["1. Note"])],
        "a footnote is never placed on a page before its call"
    );
}

/// A four-line footnote that does not fit below a call 70pt down the page area.
const TALL_NOTE: &str = "A<br>B<br>C<br>D";
const NESTED_CSS: &str =
    "td{padding:0}table{border-spacing:0}p.tall{height:70pt}.cols{columns:2;column-gap:10pt}";

fn nested_pages(css: &str, body: &str) -> Vec<(Vec<String>, Vec<String>)> {
    let trace = render(&format!("{NESTED_CSS}{css}"), body)
        .unwrap_or_else(|error| panic!("{body}: {error}"))
        .layout
        .unwrap();
    pages(&trace)
}

#[test]
fn line_policies_move_calls_inside_nested_layouts_to_the_page_their_footnotes_fit() {
    // The inline block's own line, then the paragraph line holding it.
    const ATOMIC_LINES: &[&str] = &["Cell1", "\u{fffc}"];
    for (layout, body, lines) in [
        (
            "table cell",
            format!("<table><tr><td>Cell<span class=fn>{TALL_NOTE}</span></td></tr></table>"),
            &["Cell1"][..],
        ),
        (
            "multi-column container",
            format!("<div class=cols>Cell<span class=fn>{TALL_NOTE}</span></div>"),
            &["Cell1"],
        ),
        (
            "flex item",
            format!(
                "<div style=display:flex><div>Cell<span class=fn>{TALL_NOTE}</span></div></div>"
            ),
            &["Cell1"],
        ),
        (
            "grid item",
            format!(
                "<div style=display:grid><div>Cell<span class=fn>{TALL_NOTE}</span></div></div>"
            ),
            &["Cell1"],
        ),
        (
            "inline block",
            format!(
                "<p><span style=display:inline-block>Cell<span class=fn>{TALL_NOTE}</span></span></p>"
            ),
            ATOMIC_LINES,
        ),
    ] {
        let pages = nested_pages(
            ".fn{footnote-policy:line}",
            &format!("<p class=tall>Top</p>{body}"),
        );
        assert_eq!(
            pages,
            [
                (texts(&["Top"]), Vec::new()),
                (texts(lines), texts(&["1. A", "B", "C", "D"]))
            ],
            "the {layout}'s call line moves to the next page with its whole footnote"
        );
    }
}

#[test]
fn nested_line_and_block_policies_break_before_the_call_line_or_its_paragraph() {
    let body = format!(
        "<p class=tall style=height:60pt>Top</p><table><tr><td>First<br>Second<span class=fn>{TALL_NOTE}</span></td></tr></table>"
    );
    let line = nested_pages(".fn{footnote-policy:line}td{orphans:1;widows:1}", &body);
    assert_eq!(
        line,
        [
            (texts(&["Top", "First"]), Vec::new()),
            (texts(&["Second1"]), texts(&["1. A", "B", "C", "D"]))
        ],
        "line moves only the line holding the call"
    );
    let block = nested_pages(".fn{footnote-policy:block}td{orphans:1;widows:1}", &body);
    assert_eq!(
        block,
        [
            (texts(&["Top"]), Vec::new()),
            (
                texts(&["First", "Second1"]),
                texts(&["1. A", "B", "C", "D"])
            )
        ],
        "block moves the whole paragraph holding the call"
    );
}

#[test]
fn a_deferred_call_in_a_later_column_ends_its_row_of_columns() {
    let pages = nested_pages(
        ".fn{footnote-policy:line}",
        &format!(
            "<p class=tall>Top</p><div class=cols><p>Lead</p><p>Cell<span class=fn>{TALL_NOTE}</span></p></div>"
        ),
    );
    assert_eq!(
        pages,
        [
            (texts(&["Top", "Lead"]), Vec::new()),
            (texts(&["Cell1"]), texts(&["1. A", "B", "C", "D"]))
        ],
        "the forced page break before the call's line leaves the row on the first page"
    );
}

#[test]
fn calls_that_no_break_can_move_let_their_footnotes_continue_after_them() {
    // CSS Fragmentation 3 §4.1: forced breaks inside monolithic content are ignored; an
    // absolutely positioned box is laid out unbroken, and a cell line at the top of its page
    // has no earlier break.
    let positioned = nested_pages(
        ".fn{footnote-policy:line}.abs{position:absolute;top:76pt}",
        &format!("<p class=tall>Top</p><div class=abs>Abs<span class=fn>{TALL_NOTE}</span></div>"),
    );
    assert_eq!(positioned.len(), 2, "{positioned:?}");
    assert_eq!(positioned[0].0, texts(&["Top", "Abs1"]));
    assert_eq!(
        positioned[1].0,
        Vec::<String>::new(),
        "the footnote continues on a page of its own: {positioned:?}"
    );
    let tall = (1..=12)
        .map(|index| format!("N{index}"))
        .collect::<Vec<_>>()
        .join("<br>");
    let first = nested_pages(
        ".fn{footnote-policy:line}",
        &format!("<table><tr><td>Cell<span class=fn>{tall}</span></td></tr></table>"),
    );
    assert_eq!(
        first[0].0,
        texts(&["Cell1"]),
        "a call that starts its page stays on it: {first:?}"
    );
    assert_eq!(first.len(), 2, "the footnote continues on the next page");
}
