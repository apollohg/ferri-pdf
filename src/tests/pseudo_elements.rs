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

type StyledRun = (String, [u8; 3], f32);

/// The first page's text lines as (text, [(run text, colour, font size)]).
fn styled_lines(trace: &crate::LayoutTrace) -> Vec<(String, Vec<StyledRun>)> {
    trace.pages[0]
        .iter()
        .map(|line| {
            (
                line.text.clone(),
                line.runs
                    .iter()
                    .map(|run| (run.text.clone(), run.color.rgb, run.font_size))
                    .collect(),
            )
        })
        .collect()
}

fn line_runs(lines: &[(String, Vec<StyledRun>)], text: &str) -> Vec<StyledRun> {
    lines
        .iter()
        .find(|line| line.0 == text)
        .unwrap_or_else(|| panic!("no line {text:?} among {lines:#?}"))
        .1
        .clone()
}

fn styled(text: &str, color: [u8; 3], size: f32) -> StyledRun {
    (text.to_owned(), color, size)
}

#[test]
fn first_letter_wraps_the_first_letter_text_and_its_punctuation() {
    let lines = styled_lines(&trace(
        "p::first-letter{color:red;font-size:20pt}p.q::first-letter::prefix{color:blue}\
         p.q::first-letter::suffix{color:green}",
        "<p>Hello</p><p class=q>“(A)” quoted</p><p>67 million</p><p>\"\u{2009}Spaced\"</p>",
    ));
    assert_eq!(
        line_runs(&lines, "Hello"),
        [styled("H", RED, 20.0), styled("ello", BLACK, 12.0)]
    );
    assert_eq!(
        line_runs(&lines, "“(A)” quoted"),
        [
            styled("“(", BLUE, 20.0),
            styled("A", RED, 20.0),
            styled(")”", GREEN, 20.0),
            styled(" quoted", BLACK, 12.0)
        ],
        "::prefix and ::suffix nest inside the innermost ::first-letter and inherit from it"
    );
    assert_eq!(
        line_runs(&lines, "67 million"),
        [styled("6", RED, 20.0), styled("7 million", BLACK, 12.0)],
        "a digit is a first letter"
    );
    assert_eq!(
        line_runs(&lines, "\"\u{2009}Spaced\""),
        [
            styled("\"\u{2009}", RED, 20.0),
            styled("S", RED, 20.0),
            styled("paced\"", BLACK, 12.0)
        ],
        "typographic space between the preceding punctuation and the letter joins the \
         ::prefix, which inherits from the ::first-letter"
    );
}

#[test]
fn first_letter_takes_generated_and_descendant_text_and_nests_by_origin() {
    let lines = styled_lines(&trace(
        "p::first-letter{color:red}div::first-letter{font-size:20pt;color:green}\
         p.n::before{content:'Note: '}span{color:blue}p.s::first-letter{color:inherit}",
        "<p class=n>with before</p><div><p>Nested</p></div><p class=s><span>The</span> span</p>",
    ));
    assert_eq!(
        line_runs(&lines, "Note: with before"),
        [
            styled("N", RED, 12.0),
            styled("ote: ", BLACK, 12.0),
            styled("with before", BLACK, 12.0)
        ],
        "in-flow ::before content holds the first letter"
    );
    assert_eq!(
        line_runs(&lines, "Nested"),
        [styled("N", RED, 20.0), styled("ested", BLACK, 12.0)],
        "p::first-letter nests in, and inherits from, div::first-letter"
    );
    assert_eq!(
        line_runs(&lines, "The span"),
        [
            styled("T", BLUE, 12.0),
            styled("he", BLUE, 12.0),
            styled(" span", BLACK, 12.0)
        ],
        "the ::first-letter wraps text inside a descendant and inherits from it"
    );
}

#[test]
fn first_letter_needs_qualifying_text_first_on_the_first_formatted_line() {
    let lines = styled_lines(&trace(
        "p::first-letter,div::first-letter,li::first-letter{color:red}\
         li{list-style-position:inside}.flex{display:flex}",
        "<p><br>Second</p><p>...</p><div><span style='display:inline-block'>Inline</span> \
         block</div><div class=flex><span>Flex</span></div><ol><li>Item</li></ol>\
         <div><table><tr><td>Cell</td></tr></table>After</div>",
    ));
    for (line, reason) in [
        ("Second", "a forced break empties the first formatted line"),
        ("...", "punctuation alone has no first letter"),
        (
            "Inline",
            "an inline-block's own lines are not its container's",
        ),
        ("Flex", "a flex container is not a block container"),
        (
            "Cell",
            "a table cell's first line is not the enclosing block's",
        ),
        (
            "After",
            "the table wrapper first in flow takes the first formatted line",
        ),
    ] {
        assert!(
            line_runs(&lines, line).iter().all(|run| run.1 == BLACK),
            "{reason}: {lines:#?}"
        );
    }
    assert_eq!(
        line_runs(&lines, "1. Item"),
        [
            styled("1. ", BLACK, 12.0),
            styled("I", RED, 12.0),
            styled("tem", BLACK, 12.0)
        ],
        "an inside marker is ignored as if out of flow"
    );
}

#[test]
fn legacy_single_colon_first_letter_and_floated_first_letter() {
    let trace = trace(
        "p:first-letter{float:left;font-size:36pt;line-height:36pt;color:red;margin-right:3pt}",
        "<p>Floated first letter paragraph whose text wraps beside the floated letter for \
         several lines of ordinary body text.</p>",
    );
    let lines = &trace.pages[0];
    let geometry: Vec<_> = lines
        .iter()
        .map(|line| (line.text.as_str(), line.x, line.width))
        .collect();
    let letter = lines
        .iter()
        .find(|line| line.text == "F")
        .unwrap_or_else(|| panic!("the floated first letter has its own line: {geometry:#?}"));
    assert_eq!(letter.runs[0].color.rgb, RED);
    assert_labelled_pt("floated letter size", letter.runs[0].font_size, 36.0);
    let following: Vec<_> = lines.iter().filter(|line| line.text != "F").collect();
    assert!(
        following[0].text.starts_with("loated"),
        "the rest of the word follows the float: {geometry:#?}"
    );
    assert!(
        following[0].x >= letter.x + letter.width + 3.0 - 0.01,
        "lines beside the float start after its margin box: {geometry:#?}"
    );
    let last = following.last().expect("the paragraph has lines");
    assert!(
        (last.x - letter.x).abs() < 0.01,
        "lines below the float return to the content edge: {geometry:#?}"
    );
}
