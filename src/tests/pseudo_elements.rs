use super::common;
use crate::render_html;
use common::{PROFILE, assert_labelled_pt, local_options};

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 128, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const BLACK: [u8; 3] = [0, 0, 0];

fn trace(css: &str, body: &str) -> crate::LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}ol,ul{{margin:0;padding-left:60pt}}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("render failed: {error}\ncss: {css}\nbody: {body}"))
    .layout
    .expect("layout requested")
}

/// Every run on the first page as (text, colour, font size, line x + run x).
fn runs(trace: &crate::LayoutTrace) -> Vec<(String, [u8; 3], f32, f32)> {
    trace.pages[0]
        .iter()
        .flat_map(|line| {
            line.runs.iter().map(move |run| {
                (
                    run.text.clone(),
                    run.color.rgb,
                    run.font_size,
                    line.x + run.x,
                )
            })
        })
        .collect()
}

fn run<'a>(runs: &'a [(String, [u8; 3], f32, f32)], text: &str) -> &'a (String, [u8; 3], f32, f32) {
    runs.iter()
        .find(|run| run.0 == text)
        .unwrap_or_else(|| panic!("no run {text:?} among {runs:#?}"))
}

#[test]
fn marker_rules_style_the_marker_text_and_not_the_list_item() {
    let trace = trace(
        "li{color:blue}li::marker{color:red;font-size:20pt}",
        "<ol><li>Item</li></ol>",
    );
    let runs = runs(&trace);
    let marker = run(&runs, "1. ");
    assert_eq!(marker.1, RED, "::marker colour: {runs:#?}");
    assert_labelled_pt("::marker font size", marker.2, 20.0);
    let item = run(&runs, "Item");
    assert_eq!(
        item.1, BLUE,
        "list item text keeps its own colour: {runs:#?}"
    );
    assert_labelled_pt("list item font size", item.2, 12.0);
}

#[test]
fn marker_inherits_from_its_list_item_and_ignores_box_properties() {
    let plain = runs(&trace("li{color:green}", "<ol><li>Item</li></ol>"));
    let boxed = runs(&trace(
        "li{color:green}li::marker{margin-left:50pt;padding:9pt;border:3pt solid;display:block;\
         position:absolute;float:right;width:80pt;text-decoration:underline;background:red}",
        "<ol><li>Item</li></ol>",
    ));
    let (plain, boxed) = (run(&plain, "1. "), run(&boxed, "1. "));
    assert_eq!(plain.1, GREEN, "the marker inherits the list item colour");
    assert_eq!(
        plain, boxed,
        "only inherited text properties, content, direction, unicode-bidi and \
         text-combine-upright apply to ::marker (CSS Lists 3 §3.1.1)"
    );
}

#[test]
fn marker_content_replaces_the_counter_text_or_suppresses_the_marker() {
    let trace = trace(
        "ol{list-style-type:lower-roman}li.note::marker{content:'Note ' counter(list-item) ': '}\
         li.none::marker{content:none}li.empty::marker{content:''}",
        "<ol><li>One</li><li class=note>Two</li><li class=none>Three</li><li class=empty>Four</li><li>Five</li></ol>",
    );
    let text: Vec<_> = trace.pages[0]
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        text,
        [
            "i. ", "One", "Note 2: ", "Two", "Three", "Four", "v. ", "Five"
        ],
        "content on ::marker is generated as for ::before; none and empty content make no box"
    );
}

#[test]
fn marker_white_space_and_text_transform_defaults_yield_to_author_rules() {
    let trace = trace(
        "ul{list-style-type:'a  b';text-transform:uppercase}li+li::marker{white-space:normal;\
         text-transform:inherit}",
        "<ul><li>x</li><li>y</li></ul>",
    );
    let text: Vec<_> = trace.pages[0]
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        text,
        ["a  b", "X", "A B", "Y"],
        "the UA rule ::marker {{ white-space: pre; text-transform: none }} is cascaded"
    );
}

#[test]
fn inside_and_inline_list_item_markers_are_styled_inline_boxes() {
    let trace = trace(
        "li{list-style-position:inside}li::marker{color:red;font-weight:bold}\
         span{display:inline list-item;list-style-type:'* '}span::marker{color:blue}",
        "<ol><li>\n   Item</li></ol><p>Lead <span>inline</span></p>",
    );
    assert_eq!(
        trace.pages[0][0].text, "1. Item",
        "collapsible white space after the pre-spaced inside marker is discarded"
    );
    let runs = runs(&trace);
    let marker = run(&runs, "1. ");
    let item = run(&runs, "Item");
    assert_eq!(marker.1, RED, "inside ::marker colour: {runs:#?}");
    assert_eq!(item.1, BLACK, "list item text keeps its colour: {runs:#?}");
    assert!(
        marker.3 < item.3,
        "the inside marker precedes the content on its line: {runs:#?}"
    );
    let inline = runs
        .iter()
        .find(|run| run.0.starts_with('*'))
        .unwrap_or_else(|| panic!("no inline list item marker among {runs:#?}"));
    assert_eq!(
        inline.1, BLUE,
        "inline list item ::marker colour: {runs:#?}"
    );
}

#[test]
fn marker_rules_cascade_by_specificity_and_reject_pseudo_class_syntax() {
    let trace = trace(
        "ol li::marker{color:red}::marker{color:blue}li:marker{color:green}",
        "<ol><li>Item</li></ol>",
    );
    let runs = runs(&trace);
    assert_eq!(
        run(&runs, "1. ").1,
        RED,
        "ol li::marker outweighs ::marker, and the single-colon :marker is not a \
         pseudo-element (Selectors 4 §16): {runs:#?}"
    );
}
