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
    let shown = |page| -> Vec<String> {
        common::visible_frame_columns(&trace, page)
            .into_iter()
            .flat_map(|(_, texts)| texts)
            .collect()
    };
    assert_eq!(
        shown(0),
        texts(&["1. N1", "N2", "N3"]),
        "the 30pt area holds three lines"
    );
    let rest: Vec<String> = (4..=12).map(|index| format!("N{index}")).collect();
    assert_eq!(shown(1), rest, "the rest of the body on a page of its own");
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

#[test]
fn footnote_bodies_contain_their_floats_as_formatting_context_roots() {
    // A footnote element floats (CSS GCPM 3 §2.2), so it establishes a block formatting
    // context whose automatic height includes its floats (CSS 2.1 §9.4.1, §10.6.7); the area
    // grows to hold the 50pt float above the page area's bottom at 110pt.
    const PAGE_AREA_BOTTOM_PT: f32 = 110.0;
    const FLOAT_PT: f32 = 50.0;
    let trace = render(
        ".f{float:left;width:20pt;height:50pt;background:rgb(255,0,0)}",
        "<p>Text<span class=fn><span class=f></span>Note</span></p>",
    )
    .unwrap()
    .layout
    .unwrap();
    let floats = common::page_boxes(&trace, [255, 0, 0]);
    assert_eq!(floats.len(), 1, "one float paints: {floats:?}");
    let float = floats[0];
    common::assert_labelled_pt("float height", float.height, FLOAT_PT);
    common::assert_labelled_pt(
        "float bottom at the footnote area's bottom",
        float.y + float.height,
        PAGE_AREA_BOTTOM_PT,
    );
}

#[test]
fn cloned_footnote_bodies_wrap_each_part_in_their_border_and_padding() {
    // CSS Fragmentation 3 §5.4: a cloned body split across footnote areas repeats its 3pt
    // border and 5pt padding around each part; a sliced one is cut at the break. Eight 10pt
    // lines do not fit the 100pt page area below the 40pt spacer, so the body breaks.
    const EDGE_PT: f32 = 8.0;
    const LINE_PT: f32 = 10.0;
    const PAGE_AREA_BOTTOM_PT: f32 = 110.0;
    let body = "<div style='height:40pt'></div><p>Call<span class=fn>1<br>2<br>3<br>4<br>5<br>6<br>7<br>8</span></p>";
    let boxes = |decoration: &str| {
        let trace = render(
            &format!(
                ".fn{{border:3pt solid;padding:5pt;background:rgb(0,255,0);box-decoration-break:{decoration}}}"
            ),
            body,
        )
        .unwrap()
        .layout
        .unwrap();
        common::page_boxes(&trace, [0, 255, 0])
    };
    let cloned = boxes("clone");
    assert_eq!(cloned.len(), 2, "one box per part: {cloned:?}");
    let mut lines = 0.0;
    for (page, rect) in cloned.iter().enumerate() {
        let content = rect.height - 2.0 * EDGE_PT;
        assert!(
            content > 0.0 && (content / LINE_PT).fract() == 0.0,
            "part {page} wraps whole lines in both edges: {rect:?}"
        );
        lines += content / LINE_PT;
        common::assert_labelled_pt(
            &format!("part {page} bottom"),
            rect.y + rect.height,
            PAGE_AREA_BOTTOM_PT,
        );
    }
    common::assert_labelled_pt("lines across the parts", lines, 8.0);
    let sliced = boxes("slice");
    let whole = 2.0 * EDGE_PT + 8.0 * LINE_PT;
    assert!(
        sliced
            .iter()
            .all(|rect| (rect.height - whole).abs() < 0.001),
        "a sliced body keeps its whole box: {sliced:?}"
    );
}

#[test]
fn footnote_area_perspective_projects_its_transformed_bodies() {
    // CSS Transforms 2 §4.1.4: the area's perspective places its transformed child, the
    // body, as a box with the same perspective places a child in the flow. The one-line area
    // spans the 280pt by 10pt bottom of the page area, which the reference box occupies.
    const BODY: [u8; 3] = [0, 255, 0];
    let corners = |trace: &crate::LayoutTrace| -> Vec<[f32; 2]> {
        let mut found = None;
        let mut search = |items: &[DisplayItem], origin: [f32; 2]| {
            let mut projection = None;
            for item in items {
                match item {
                    DisplayItem::PushProjection(matrix) => projection = Some(*matrix),
                    DisplayItem::Box(paint) if paint.decoration.background.rgb == BODY => {
                        found = Some((projection, paint.rect, origin));
                    }
                    _ => {}
                }
            }
        };
        let page = &trace.display_lists[0];
        search(page, [0.0; 2]);
        for item in page {
            if let DisplayItem::Frame(frame) = item {
                search(&frame.items, [frame.matrix[4], frame.matrix[5]]);
            }
        }
        let (projection, rect, origin) = found.expect("the body paints");
        let projection = projection.expect("the body is projected");
        [
            [rect.x, rect.y],
            [rect.x + rect.width, rect.y],
            [rect.x, rect.y + rect.height],
            [rect.x + rect.width, rect.y + rect.height],
        ]
        .map(|corner| {
            let [x, y] = crate::paint::projection::apply(projection, corner);
            [x + origin[0], y + origin[1]]
        })
        .to_vec()
    };
    let footnote = render(
        "@page{@footnote{perspective:100pt}}.fn{background:rgb(0,255,0);transform:rotateX(45deg)}",
        "<p>Call<span class=fn>Note</span></p>",
    )
    .unwrap()
    .layout
    .unwrap();
    let reference = render(
        ".area{position:absolute;left:0;top:90pt;width:280pt;height:10pt;perspective:100pt}.body{height:10pt;background:rgb(0,255,0);transform:rotateX(45deg)}",
        "<div class=area><div class=body></div></div>",
    )
    .unwrap()
    .layout
    .unwrap();
    let [projected, expected] = [corners(&footnote), corners(&reference)];
    for (corner, (actual, expected)) in projected.iter().zip(&expected).enumerate() {
        for axis in 0..2 {
            common::assert_labelled_pt(
                &format!("corner {corner} axis {axis}: {projected:?} against {expected:?}"),
                actual[axis],
                expected[axis],
            );
        }
    }
}

/// The texts among `items`, each with whether a transform or projection places it.
fn transformed_texts(
    items: &[DisplayItem],
    lines: &[crate::layout::TextLine],
    transformed: bool,
) -> Vec<(String, bool)> {
    let mut found = Vec::new();
    let mut open = Vec::new();
    for item in items {
        let inside = transformed || open.iter().any(|transform| *transform);
        match item {
            DisplayItem::Pop => {
                open.pop();
            }
            DisplayItem::PushProjection(_) | DisplayItem::PushTransform(_) => open.push(true),
            item if item.is_push() => open.push(false),
            DisplayItem::Text { line } => found.push((lines[*line].text.clone(), inside)),
            DisplayItem::Frame(frame) => {
                found.extend(transformed_texts(&frame.items, &frame.lines, inside));
            }
            _ => {}
        }
    }
    found
}

#[test]
fn preserved_3d_footnote_bodies_add_their_planes_text_to_the_area_text() {
    // CSS Transforms 2 §4.1.2: the body establishes a 3D rendering context, which marks its
    // items without a region of its own; its rotated child's text joins the text the area's
    // backgrounds clip to (CSS Backgrounds 4 §2.8) through the child's plane.
    const AREA: [u8; 3] = [0, 0, 255];
    let trace = render(
        "@page{@footnote{background:rgb(0,0,255);background-clip:text}}.fn{transform-style:preserve-3d}.fn b{display:block;transform:rotateX(30deg)}",
        "<p>Call<span class=fn><b>Note</b></span></p>",
    )
    .unwrap()
    .layout
    .unwrap();
    let area = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == AREA => {
                paint.text_area.clone()
            }
            _ => None,
        })
        .expect("the footnote area paints its text-clipped background with a text area");
    let texts = transformed_texts(&area.items, &area.lines, false);
    assert!(
        texts.contains(&("Note".to_owned(), true)),
        "the rotated child's text clips the area's background through its plane: {texts:?}"
    );
}

#[test]
fn footnote_area_columns_balance_the_bodies_and_rule_their_gaps() {
    // CSS GCPM 3 §2.4: the footnote area is a block container, so `columns: 3` makes it a
    // multi-column container whose bodies balance (CSS Multi-column Layout 1 §7.1). Seven
    // lines take three 10pt lines a column; orphans and widows of 2 allow the first body to
    // break after its third line. The 280pt area with 10pt gaps has 86.67pt columns.
    const COLUMN_PITCH_PT: f32 = 280.0 / 3.0 + 10.0 / 3.0;
    let trace = render(
        "@page{@footnote{columns:3;column-gap:10pt;column-rule:2pt solid rgb(255,0,0)}}",
        "<p>A<span class=fn>A1<br>A2<br>A3<br>A4<br>A5</span> B<span class=fn>B</span> C<span class=fn>C</span></p>",
    )
    .unwrap()
    .layout
    .unwrap();
    let columns = common::visible_frame_columns(&trace, 0);
    let held: Vec<_> = columns.iter().map(|(_, texts)| texts.clone()).collect();
    assert_eq!(
        held,
        [
            texts(&["1. A1", "A2", "A3"]),
            texts(&["A4", "A5", "2. B"]),
            texts(&["3. C"])
        ],
        "{columns:?}"
    );
    for (index, (left, _)) in columns.iter().enumerate() {
        common::assert_labelled_pt(
            &format!("column {index} left"),
            *left,
            10.0 + index as f32 * COLUMN_PITCH_PT,
        );
    }
    let rules = common::column_rules(&trace, 0, [255, 0, 0]);
    assert_eq!(
        rules.len(),
        2,
        "a rule in each gap between filled columns: {rules:?}"
    );
    for (index, rule) in rules.iter().enumerate() {
        common::assert_labelled_pt(
            &format!("rule {index} centre"),
            rule.x + rule.width / 2.0,
            10.0 + (index + 1) as f32 * COLUMN_PITCH_PT - 5.0,
        );
        common::assert_labelled_pt(&format!("rule {index} top"), rule.y, 80.0);
        common::assert_labelled_pt(&format!("rule {index} height"), rule.height, 30.0);
    }
}

#[test]
fn footnote_area_columns_fill_to_max_height_and_continue_on_the_next_page() {
    // CSS GCPM 3 §2.4.2: `max-height` bounds the area, whose two columns then hold eight
    // lines; the rest of the body continues in the next page's area, which a page holding
    // only footnotes does not bound, balanced there.
    let note = (1..=13)
        .map(|index| format!("N{index}"))
        .collect::<Vec<_>>()
        .join("<br>");
    let trace = render(
        "@page{@footnote{columns:2;max-height:40pt}}",
        &format!("<p>Body<span class=fn>{note}</span></p>"),
    )
    .unwrap()
    .layout
    .unwrap();
    let pages: Vec<Vec<Vec<String>>> = (0..trace.display_lists.len())
        .map(|page| {
            common::visible_frame_columns(&trace, page)
                .into_iter()
                .map(|(_, texts)| texts)
                .collect()
        })
        .collect();
    assert_eq!(
        pages,
        [
            vec![
                texts(&["1. N1", "N2", "N3", "N4"]),
                texts(&["N5", "N6", "N7", "N8"])
            ],
            vec![texts(&["N9", "N10", "N11"]), texts(&["N12", "N13"])],
        ]
    );
}

#[test]
fn auto_column_fill_fills_footnote_columns_in_order_up_to_the_area_height() {
    // CSS Multi-column Layout 1 §7.1: `column-fill: auto` fills the first column to the
    // area's 40pt maximum height before the next; `balance` splits the three lines.
    let columns = |fill: &str| -> Vec<Vec<String>> {
        let trace = render(
            &format!(
                "@page{{@footnote{{columns:2;column-fill:{fill};max-height:40pt}}}}.fn{{orphans:1;widows:1}}"
            ),
            "<p>A<span class=fn>A1<br>A2<br>A3</span></p>",
        )
        .unwrap()
        .layout
        .unwrap();
        common::visible_frame_columns(&trace, 0)
            .into_iter()
            .map(|(_, texts)| texts)
            .collect()
    };
    assert_eq!(columns("auto"), [texts(&["1. A1", "A2", "A3"])]);
    assert_eq!(
        columns("balance"),
        [texts(&["1. A1", "A2"]), texts(&["A3"])],
        "balancing breaks after the second line, which orphans and widows of 1 allow"
    );
}
