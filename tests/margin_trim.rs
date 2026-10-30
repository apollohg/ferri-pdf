//! CSS Box 4 §3.3 `margin-trim` (`none | [ block || inline ] | [ block-start || inline-start ||
//! block-end || inline-end ]`) on block, multi-column, flex and grid containers, and at
//! fragmentation breaks.
mod common;
use common::{assert_labelled_pt, local_options, painted_box, render_trace};
use ferri_pdf::{LayoutTrace, Rect};

const CONTAINER: [u8; 3] = [1, 2, 3];
const FIRST: [u8; 3] = [4, 5, 6];
const SECOND: [u8; 3] = [7, 8, 9];
/// Every probed child margin.
const MARGIN: f32 = 20.0;
/// Every probed child's height.
const CHILD: f32 = 10.0;
/// Every probed flex or grid item's width.
const ITEM: f32 = 100.0;

fn render(css: &str, body: &str) -> LayoutTrace {
    ferri_pdf::render_html(
        &format!(
            "<style>{}div,p,section{{margin:0}}.c{{background:rgb(1,2,3);border:1pt solid transparent}}.a{{background:rgb(4,5,6)}}.b{{background:rgb(7,8,9)}}{css}</style>{body}",
            common::PROFILE
        ),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("margin-trim: {error}; CSS={css}; HTML={body}"))
    .layout
    .unwrap()
}

fn boxes(trace: &LayoutTrace) -> [Rect; 3] {
    [CONTAINER, FIRST, SECOND].map(|color| painted_box(trace, color))
}

/// The content box edges of the container, whose border is one point wide.
fn content_top(container: Rect) -> f32 {
    container.y + 1.0
}
fn content_bottom(container: Rect) -> f32 {
    container.y + container.height - 1.0
}
fn content_left(container: Rect) -> f32 {
    container.x + 1.0
}
fn content_right(container: Rect) -> f32 {
    container.x + container.width - 1.0
}

#[test]
fn block_containers_trim_the_first_and_last_child_margins_and_those_collapsed_with_them() {
    let children = format!(
        ".a,.b{{height:{CHILD}pt;margin:{MARGIN}pt}}.wrap{{margin-bottom:5pt}}.inner{{margin-top:30pt}}"
    );
    let body = "<section class=c><div><div class=a></div></div><div class='wrap'><div class=b></div></div></section>";
    for (trim, top, bottom) in [
        ("none", MARGIN, MARGIN),
        ("block", 0.0, 0.0),
        ("block-start", 0.0, MARGIN),
        ("block-end", MARGIN, 0.0),
        ("block-end block-start", 0.0, 0.0),
        ("block inline", 0.0, 0.0),
        // §3.3.1: no effect on the inline-axis margins of block-level descendants.
        ("inline", MARGIN, MARGIN),
        ("inline-start inline-end", MARGIN, MARGIN),
        // The keywords of the superseded grammar are invalid and leave the initial value.
        ("in-flow", MARGIN, MARGIN),
        ("all", MARGIN, MARGIN),
    ] {
        let css = format!("{children}.c{{margin-trim:{trim}}}");
        let [container, first, second] = boxes(&render(&css, body));
        assert_labelled_pt(
            &format!("{css}: first child top"),
            first.y - content_top(container),
            top,
        );
        assert_labelled_pt(
            &format!("{css}: margin between siblings stays"),
            second.y - (first.y + first.height),
            MARGIN,
        );
        assert_labelled_pt(
            &format!("{css}: last child and the margins collapsed with its end"),
            content_bottom(container) - (second.y + second.height),
            bottom,
        );
        assert_labelled_pt(
            &format!("{css}: inline margins stay"),
            first.x - content_left(container),
            MARGIN,
        );
    }
}

#[test]
fn trimming_keeps_the_containers_own_margins_and_line_boxes() {
    let css = format!(
        ".c{{margin-trim:block;margin-top:15pt}}.a{{height:{CHILD}pt;margin-top:{MARGIN}pt}}.b{{height:{CHILD}pt}}"
    );
    let trace = render(
        &css,
        "<div class=b></div><section class=c><div class=a></div></section>",
    );
    let [container, first, second] = boxes(&trace);
    assert_labelled_pt(
        "the container's own margin",
        container.y - (second.y + second.height),
        15.0,
    );
    assert_labelled_pt("its first child's margin", first.y, content_top(container));
}

#[test]
fn multi_column_containers_trim_their_flow_edges() {
    let css = format!(
        ".c{{columns:2;margin-trim:block-start}}.a,.b{{height:{CHILD}pt;margin:{MARGIN}pt 0;break-inside:avoid}}"
    );
    let [container, first, _] = boxes(&render(
        &css,
        "<section class=c><div class=a></div><div class=b></div></section>",
    ));
    assert_labelled_pt(&css, first.y, content_top(container));
}

#[test]
fn block_containers_leave_float_margins() {
    let float = format!(
        ".a{{float:left;width:30pt;height:{CHILD}pt;margin:{MARGIN}pt}}.b{{float:right;width:30pt;height:{CHILD}pt;margin:{MARGIN}pt}}"
    );
    let css = format!("{float}.c{{margin-trim:block inline;display:flow-root}}");
    let [container, left, right] = boxes(&render(
        &css,
        "<section class=c><div class=a></div><div class=b></div></section>",
    ));
    assert_labelled_pt("left float left", left.x - content_left(container), MARGIN);
    assert_labelled_pt("left float top", left.y - content_top(container), MARGIN);
    assert_labelled_pt(
        "right float right",
        content_right(container) - (right.x + right.width),
        MARGIN,
    );
    assert_labelled_pt(
        "floats' block-end margins enclosed by the root",
        content_bottom(container) - (left.y + left.height),
        MARGIN,
    );
}

#[test]
fn flex_rows_trim_the_closest_line_at_block_sides_and_line_ends_at_inline_sides() {
    let items = format!(".a,.b{{width:{ITEM}pt;height:{CHILD}pt;margin:{MARGIN}pt}}");
    let body = "<section class=c><div class=a></div><div class=b></div></section>";
    let css = format!("{items}.c{{display:flex;margin-trim:block}}");
    let [container, first, second] = boxes(&render(&css, body));
    assert_labelled_pt("block: top margin", first.y, content_top(container));
    assert_labelled_pt("block: bottom margin", container.height - 2.0, CHILD);
    assert_labelled_pt(
        "block: inline margins stay",
        second.x - (first.x + first.width),
        2.0 * MARGIN,
    );
    let css = format!("{items}.c{{display:flex;flex-wrap:wrap;width:150pt;margin-trim:block}}");
    let [container, first, second] = boxes(&render(&css, body));
    assert_labelled_pt(
        "wrap: the first line's top",
        first.y,
        content_top(container),
    );
    assert_labelled_pt(
        "wrap: margins between lines stay",
        second.y - (first.y + first.height),
        2.0 * MARGIN,
    );
    assert_labelled_pt(
        "wrap: the last line's bottom",
        content_bottom(container) - (second.y + second.height),
        0.0,
    );
    // Shrink-to-fit containers leave no free space beside the outer items.
    for (declarations, [left, right], gaps) in [
        ("inline", [FIRST, SECOND], [0.0, 0.0]),
        ("inline-start", [FIRST, SECOND], [0.0, MARGIN]),
        ("inline-end", [FIRST, SECOND], [MARGIN, 0.0]),
        (
            "inline;flex-direction:row-reverse",
            [SECOND, FIRST],
            [0.0, 0.0],
        ),
        (
            "inline-start;flex-direction:row-reverse",
            [SECOND, FIRST],
            [0.0, MARGIN],
        ),
        ("inline-start;direction:rtl", [SECOND, FIRST], [MARGIN, 0.0]),
    ] {
        let css = format!("{items}.c{{display:inline-flex;margin-trim:{declarations}}}");
        let trace = render(&css, body);
        let container = painted_box(&trace, CONTAINER);
        let [left, right] = [left, right].map(|color| painted_box(&trace, color));
        assert_labelled_pt(
            &format!("{declarations}: the left item's left margin"),
            left.x - content_left(container),
            gaps[0],
        );
        assert_labelled_pt(
            &format!("{declarations}: margins between items stay"),
            right.x - (left.x + left.width),
            2.0 * MARGIN,
        );
        assert_labelled_pt(
            &format!("{declarations}: the right item's right margin"),
            content_right(container) - (right.x + right.width),
            gaps[1],
        );
    }
    let css = format!("{items}.c{{display:inline-flex;margin-trim:inline}}");
    let [container, ..] = boxes(&render(&css, body));
    assert_labelled_pt(
        "inline-flex: the max-content width drops the trimmed line ends",
        container.width - 2.0,
        2.0 * ITEM + 2.0 * MARGIN,
    );
    let css = format!(
        "{items}.c{{display:inline-flex;flex-wrap:wrap;width:min-content;margin-trim:inline}}"
    );
    let [container, ..] = boxes(&render(&css, body));
    assert_labelled_pt(
        "wrapping min-content: an item alone on its line loses both inline margins",
        container.width - 2.0,
        ITEM,
    );
}

#[test]
fn flex_columns_trim_line_ends_at_block_sides_and_the_closest_line_at_inline_sides() {
    let items = format!(".a,.b{{width:{ITEM}pt;height:{CHILD}pt;margin:{MARGIN}pt}}");
    let body = "<section class=c><div class=a></div><div class=b></div></section>";
    for (direction, start, end) in [("column", FIRST, SECOND), ("column-reverse", SECOND, FIRST)] {
        let css = format!("{items}.c{{display:flex;flex-direction:{direction};margin-trim:block}}");
        let trace = render(&css, body);
        let container = painted_box(&trace, CONTAINER);
        let [start, end] = [start, end].map(|color| painted_box(&trace, color));
        assert_labelled_pt(
            &format!("{direction}: block-start item"),
            start.y,
            content_top(container),
        );
        assert_labelled_pt(
            &format!("{direction}: block-end item"),
            content_bottom(container) - (end.y + end.height),
            0.0,
        );
        assert_labelled_pt(
            &format!("{direction}: between items"),
            container.height - 2.0,
            2.0 * CHILD + 2.0 * MARGIN,
        );
    }
    let css =
        format!("{items}.c{{display:inline-flex;flex-direction:column;margin-trim:inline-start}}");
    let [container, first, second] = boxes(&render(&css, body));
    for (label, item) in [("first", first), ("second", second)] {
        assert_labelled_pt(
            &format!("column inline-start: the {label} item on the only line"),
            item.x,
            content_left(container),
        );
    }
    assert_labelled_pt(
        "column inline-start: the max-content width keeps the inline-end margin",
        container.width - 2.0,
        ITEM + MARGIN,
    );
}

#[test]
fn grid_containers_trim_the_margins_in_their_edge_tracks() {
    let css = format!(
        ".c{{display:grid;margin-trim:block}}.a,.b{{height:{CHILD}pt;margin:{MARGIN}pt 0}}"
    );
    let [container, first, second] = boxes(&render(
        &css,
        "<section class=c><div class=a></div><div class=b></div></section>",
    ));
    assert_labelled_pt("the first row's top", first.y, content_top(container));
    assert_labelled_pt(
        "between rows",
        second.y - (first.y + first.height),
        2.0 * MARGIN,
    );
    assert_labelled_pt(
        "the last row's bottom",
        content_bottom(container) - (second.y + second.height),
        0.0,
    );
    let items = format!(".a,.b{{width:{ITEM}pt;height:{CHILD}pt;margin:{MARGIN}pt}}");
    let body = "<section class=c><div class=a></div><div class=b></div></section>";
    for (declarations, left, right) in [("", FIRST, SECOND), (";direction:rtl", SECOND, FIRST)] {
        let css = format!(
            "{items}.c{{display:inline-grid;grid-template-columns:auto auto;margin-trim:inline{declarations}}}"
        );
        let trace = render(&css, body);
        let container = painted_box(&trace, CONTAINER);
        let [left, right] = [left, right].map(|color| painted_box(&trace, color));
        assert_labelled_pt(
            &format!("{declarations}: the column at the left side"),
            left.x,
            content_left(container),
        );
        assert_labelled_pt(
            &format!("{declarations}: margins between columns stay"),
            right.x - (left.x + left.width),
            2.0 * MARGIN,
        );
        assert_labelled_pt(
            &format!("{declarations}: the column at the right side"),
            content_right(container),
            right.x + right.width,
        );
        assert_labelled_pt(
            &format!("{declarations}: auto columns size without the trimmed margins"),
            container.width - 2.0,
            2.0 * ITEM + 2.0 * MARGIN,
        );
    }
}

#[test]
fn cloned_fragments_trim_margins_kept_after_a_forced_break() {
    for (decoration, trim, kept) in [
        ("clone", "block-start", 0.0),
        ("clone", "block-end", MARGIN),
        ("clone", "none", MARGIN),
        ("slice", "block", MARGIN),
    ] {
        let trace = render_trace(&format!(
            "<!doctype html><style>{}p{{margin:0}}section{{box-decoration-break:{decoration};margin-trim:{trim}}}.next{{break-before:page;margin-top:{MARGIN}pt}}</style><section><p>First</p><p class=next>Second</p></section>",
            common::PROFILE
        ));
        let second = &trace.pages[1][0];
        let first = &trace.pages[0][0];
        assert_labelled_pt(
            &format!("{decoration}/{trim}: margin after the forced break"),
            second.baseline - first.baseline,
            kept,
        );
    }
}

#[test]
fn trimmed_sides_follow_the_containers_writing_mode() {
    let children = format!(".a,.b{{width:{CHILD}pt;height:{CHILD}pt;margin:{MARGIN}pt}}");
    let body = "<section class=c><div class=a></div><div class=b></div></section>";
    // Orthogonal flows lay the container out in its own writing mode; page coordinates.
    let page_boxes = |trace: &LayoutTrace| {
        [CONTAINER, FIRST, SECOND].map(|color| match common::page_boxes(trace, color)[..] {
            [rect] => rect,
            ref found => panic!("expected one {color:?} box, found {found:?}"),
        })
    };
    for (trim, gap) in [("none", MARGIN), ("block-start", 0.0), ("inline", MARGIN)] {
        let css = format!("{children}.c{{writing-mode:vertical-rl;margin-trim:{trim}}}");
        let [container, first, _] = page_boxes(&render(&css, body));
        assert_labelled_pt(
            &format!("vertical-rl {trim}: the block-start side is the right"),
            content_right(container) - (first.x + first.width),
            gap,
        );
    }
    let css =
        format!("{children}.c{{writing-mode:vertical-rl;display:flex;margin-trim:inline-start}}");
    let [container, first, second] = page_boxes(&render(&css, body));
    assert_labelled_pt(
        "vertical-rl flex: inline-start is the top side",
        first.y - content_top(container),
        0.0,
    );
    assert_labelled_pt(
        "vertical-rl flex: margins between items stay",
        second.y - (first.y + first.height),
        2.0 * MARGIN,
    );
}
