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

const PARAGRAPH: &str = "This is a somewhat long paragraph that will be broken into several \
                         lines by the layout of its block.";

fn texts(trace: &crate::LayoutTrace) -> Vec<String> {
    trace.pages[0]
        .iter()
        .map(|line| line.text.clone())
        .collect()
}

#[test]
fn first_line_styles_the_first_formatted_line_after_line_breaking() {
    let styled = trace(
        "p{width:200pt}p::first-line{font-size:16pt;color:red;text-transform:uppercase}",
        &format!("<p>{PARAGRAPH}</p>"),
    );
    let large = trace(
        "p{width:200pt;font-size:16pt;text-transform:uppercase}",
        &format!("<p>{PARAGRAPH}</p>"),
    );
    let plain = trace("p{width:200pt}", &format!("<p>{PARAGRAPH}</p>"));
    let (styled, large, plain) = (texts(&styled), texts(&large), texts(&plain));
    assert_eq!(
        styled[0], large[0],
        "the first line breaks where its ::first-line font size and casing fit: \
         {styled:#?} {large:#?}"
    );
    assert_ne!(
        large[0].len(),
        plain[0].len(),
        "the test needs the first-line styles to move the break"
    );
    let rest = PARAGRAPH[large[0].len()..].trim_start();
    let following = texts(&trace("p{width:200pt}", &format!("<p>{rest}</p>")));
    assert_eq!(
        styled[1..],
        following[..],
        "later lines lay out the remaining text with the paragraph's own styles"
    );
    let lines = styled_lines(&trace(
        "p{width:200pt}p::first-line{font-size:16pt;color:red}",
        &format!("<p>{PARAGRAPH}</p>"),
    ));
    assert!(
        lines[0].1.iter().all(|run| run.1 == RED && run.2 == 16.0),
        "the first line takes the ::first-line styles: {lines:#?}"
    );
    assert!(
        lines[1..]
            .iter()
            .flat_map(|line| &line.1)
            .all(|run| run.1 == BLACK && run.2 == 12.0),
        "later lines keep the paragraph's styles: {lines:#?}"
    );
}

#[test]
fn first_line_inherits_into_descendants_unless_they_set_their_own_values() {
    let lines = styled_lines(&trace(
        "p{width:300pt}p::first-line{font-size:16pt;color:red;direction:rtl}span{color:blue}\
         em{font-size:12pt;font-style:normal}b{font-weight:normal}p::first-letter{color:green}",
        "<p>Start <span>own colour</span> <em>own size</em> <b>inherited</b> then enough text \
         to reach a second line of the paragraph</p>",
    ));
    assert_eq!(
        lines[0].1,
        [
            styled("S", GREEN, 16.0),
            styled("tart ", RED, 16.0),
            styled("own colour", BLUE, 16.0),
            styled(" ", RED, 16.0),
            styled("own size", RED, 12.0),
            styled(" ", RED, 16.0),
            styled("inherited", RED, 16.0),
        ],
        "descendants on the first line inherit from ::first-line; ::first-letter nests \
         inside it; direction does not apply (CSS Pseudo-Elements 4 §2.1.2): {lines:#?}"
    );
    assert!(
        lines[1].1.iter().all(|run| run.2 == 12.0),
        "the second line keeps the original styles: {lines:#?}"
    );
}

#[test]
fn first_line_box_paints_its_background_and_decorations_on_the_line_only() {
    let trace = trace(
        "div{width:200pt}div::first-line{background:yellow;text-decoration:underline}",
        &format!("<div><p>{PARAGRAPH}</p><p>Second paragraph</p></div>"),
    );
    let yellow = common::painted_box(&trace, [255, 255, 0]);
    let first = &trace.pages[0][0];
    assert_labelled_pt("first-line background left", yellow.x, first.x);
    assert_labelled_pt("first-line background width", yellow.width, first.width);
    let underlines: Vec<_> = trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            crate::DisplayItem::Decoration(paint) => Some(paint.rect),
            _ => None,
        })
        .collect();
    assert_eq!(
        underlines.len(),
        1,
        "only the first formatted line of the div, in its first p, is underlined: \
         {underlines:?}"
    );
    assert_labelled_pt("underline width", underlines[0].width, first.width);
}

#[test]
fn first_line_needs_the_first_formatted_line_and_shrinks_to_fit_with_its_styles() {
    let lines = styled_lines(&trace(
        "p::first-line{color:red}",
        "<p><br>First</p><div style='display:inline-block'><p>Inline</p></div>",
    ));
    assert!(
        line_runs(&lines, "First").iter().all(|run| run.1 == BLACK),
        "a forced break makes an empty first formatted line: {lines:#?}"
    );
    assert!(
        line_runs(&lines, "Inline").iter().all(|run| run.1 == RED),
        "an inline-block's paragraph has its own first line: {lines:#?}"
    );
    let trace = trace(
        "div{float:left}p::first-line{font-size:24pt}",
        "<div><p>Wide line<br>narrow</p></div>",
    );
    let first = &trace.pages[0][0];
    let float = trace.pages[0]
        .iter()
        .map(|line| line.width)
        .fold(0.0f32, f32::max);
    assert_labelled_pt(
        "the float shrinks to the max-content of its ::first-line styled first line",
        float,
        first.width,
    );
    assert!(
        first.runs.iter().all(|run| run.font_size == 24.0),
        "first line at 24pt"
    );
}

#[test]
fn first_line_rejects_inheritance_it_cannot_render() {
    let atomic = render_html(
        &format!(
            "<style>{PROFILE}p::first-line{{color:red}}</style><p>Lead \
             <span style='display:inline-block'>box</span></p>"
        ),
        &local_options(),
    );
    let error = atomic
        .err()
        .expect("an inline-block inheriting first-line colour fails");
    assert!(
        error.to_string().contains("::first-line"),
        "the failure names the unsupported inheritance: {error}"
    );
    let later = render_html(
        &format!(
            "<style>{PROFILE}p{{width:60pt}}p::first-line{{color:red}}</style><p>Lead words \
             wrap <span style='display:inline-block'>box</span></p>"
        ),
        &local_options(),
    );
    assert!(
        later.is_ok(),
        "an inline-block after the first line inherits nothing from it: {:?}",
        later.err()
    );
    let casing = render_html(
        &format!("<style>{PROFILE}p::first-line{{text-transform:uppercase}}</style><p>straße</p>"),
        &local_options(),
    );
    let error = casing
        .err()
        .expect("uppercasing ß changes the source offsets");
    assert!(
        error.to_string().contains("::first-line text-transform"),
        "{error}"
    );
}

/// The page area's top edge in the shared profile, and its line height.
const PAGE_TOP: f32 = 36.0;
const LINE: f32 = 18.0;
const SLOT_PADDING: f32 = 6.0;
const DETAILS_CONTENT: &str =
    "details{color:green}details::details-content{background:blue;padding:6pt;color:red}";

#[test]
fn details_content_boxes_the_slotted_children_which_inherit_from_it() {
    let open = trace(
        DETAILS_CONTENT,
        "<details open><summary>Sum</summary>Body</details>",
    );
    let runs = runs(&open);
    assert_eq!(
        run(&runs, "Sum").1,
        GREEN,
        "the summary slot passes on the details colour"
    );
    assert_eq!(
        run(&runs, "Body").1,
        RED,
        "slotted children inherit from ::details-content (HTML §15.5.5)"
    );
    let slot = common::painted_box(&open, BLUE);
    assert_labelled_pt("content slot top", slot.y, PAGE_TOP + LINE);
    assert_labelled_pt(
        "content slot height",
        slot.height,
        LINE + 2.0 * SLOT_PADDING,
    );
    let closed = trace(
        DETAILS_CONTENT,
        "<details><summary>Sum</summary>Body</details>",
    );
    assert_eq!(
        texts(&closed),
        ["\u{25b8} Sum"],
        "a closed slot skips its contents"
    );
    let slot = common::painted_box(&closed, BLUE);
    assert_labelled_pt(
        "closed slot height: content-visibility: hidden adds size containment",
        slot.height,
        2.0 * SLOT_PADDING,
    );
    let generated = trace(
        "details::details-content::before{content:'Lead '}",
        "<details open>Body</details><details>Hidden</details>",
    );
    assert_eq!(
        texts(&generated),
        ["\u{25be} Details", "Lead Body", "\u{25b8} Details"],
        "the content slot generates ::before like an element, inside its skipped contents"
    );
}

#[test]
fn details_content_author_rules_override_the_slot_style_attribute() {
    for (css, expected) in [
        (
            "details::details-content{content-visibility:visible}",
            &["\u{25b8} Sum", "Closed"][..],
        ),
        (
            "details::details-content{display:contents}",
            &["\u{25b8} Sum", "Closed"],
        ),
        ("details::details-content{display:none}", &["\u{25b8} Sum"]),
    ] {
        assert_eq!(
            texts(&trace(
                css,
                "<details><summary>Sum</summary>Closed</details>"
            )),
            expected,
            "{css}: the outer context wins over the shadow tree's style attribute"
        );
    }
}

#[test]
fn details_content_matches_as_its_backing_element() {
    for (css, color) in [
        ("details::details-content{color:blue}", BLUE),
        ("::details-content{color:blue}", BLUE),
        ("div::details-content{color:blue}", BLACK),
        ("details::details-content:first-child{color:blue}", BLACK),
        (
            "details::details-content:nth-last-child(1){color:blue}",
            BLACK,
        ),
        ("details::details-content:empty{color:blue}", BLACK),
        ("details::details-content:has(*){color:blue}", BLACK),
        (
            "details::details-content:not(:first-child){color:blue}",
            BLUE,
        ),
        (
            "details::details-content:not(:empty, :only-child){color:blue}",
            BLUE,
        ),
        ("details::details-content:not(.x){color:blue}", BLUE),
        ("details::details-content:is(.x){color:blue}", BLACK),
        (
            "details::details-content:not(:has(*), :has(~ *)){color:blue}",
            BLUE,
        ),
        ("details::details-content:hover{color:blue}", BLACK),
        ("details{&::details-content{color:blue}}", BLUE),
        (
            "@scope (details) to (summary) {:scope::details-content{color:blue}}",
            BLUE,
        ),
        (
            "details::details-content{color:blue}details{color:green}",
            BLUE,
        ),
        (
            "details::details-content{color:blue}.x::details-content{color:green}",
            GREEN,
        ),
        ("slot{color:blue}", BLACK),
        (
            "details>*{color:green}details::details-content{color:blue}",
            GREEN,
        ),
    ] {
        let trace = trace(css, "<details class=x open><p>Body</p></details>");
        assert_eq!(run(&runs(&trace), "Body").1, color, "{css}");
    }
}
