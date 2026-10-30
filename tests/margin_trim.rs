//! CSS Box 4 §3.3 `margin-trim` (`none | in-flow | all`) on block, multi-column, flex and
//! grid containers, on floats, and at fragmentation breaks.
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

/// The content box top of the container, whose border is one point wide.
fn content_top(container: Rect) -> f32 {
    container.y + 1.0
}

#[test]
fn block_containers_trim_the_first_and_last_child_margins_and_those_collapsed_with_them() {
    let children = format!(
        ".a,.b{{height:{CHILD}pt;margin:{MARGIN}pt 0}}.wrap{{margin-bottom:5pt}}.inner{{margin-top:30pt}}"
    );
    let body = "<section class=c><div><div class=a></div></div><div class='wrap'><div class=b></div></div></section>";
    for (trim, top, gap, bottom) in [
        ("none", MARGIN, MARGIN, MARGIN),
        ("in-flow", 0.0, MARGIN, 0.0),
        ("all", 0.0, MARGIN, 0.0),
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
            gap,
        );
        assert_labelled_pt(
            &format!("{css}: last child and the margins collapsed with its end"),
            container.y + container.height - 1.0 - (second.y + second.height),
            bottom,
        );
    }
}

#[test]
fn trimming_keeps_the_containers_own_margins_and_line_boxes() {
    let css = format!(
        ".c{{margin-trim:in-flow;margin-top:15pt}}.a{{height:{CHILD}pt;margin-top:{MARGIN}pt}}.b{{height:{CHILD}pt}}"
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
        ".c{{columns:2;margin-trim:in-flow}}.a,.b{{height:{CHILD}pt;margin:{MARGIN}pt 0;break-inside:avoid}}"
    );
    let [container, first, _] = boxes(&render(
        &css,
        "<section class=c><div class=a></div><div class=b></div></section>",
    ));
    assert_labelled_pt(&css, first.y, content_top(container));
}

#[test]
fn margin_trim_all_trims_float_margins_at_the_container_edges() {
    let float = format!(
        ".a{{float:left;width:30pt;height:{CHILD}pt;margin:{MARGIN}pt}}.b{{float:right;width:30pt;height:{CHILD}pt;margin:{MARGIN}pt}}"
    );
    let body = "<section class=c><div class=a></div><div class=b></div></section>";
    for (trim, inset) in [("in-flow", MARGIN), ("all", 0.0)] {
        let css = format!("{float}.c{{margin-trim:{trim};display:flow-root}}");
        let [container, left, right] = boxes(&render(&css, body));
        assert_labelled_pt(
            &format!("{trim}: left float left"),
            left.x - (container.x + 1.0),
            inset,
        );
        assert_labelled_pt(
            &format!("{trim}: left float top"),
            left.y - content_top(container),
            inset,
        );
        assert_labelled_pt(
            &format!("{trim}: right float right"),
            container.x + container.width - 1.0 - (right.x + right.width),
            inset,
        );
        assert_labelled_pt(
            &format!("{trim}: floats' block-end margins enclosed by the root"),
            container.y + container.height - 1.0 - (left.y + left.height),
            inset,
        );
    }
    let css = format!("{float}.c{{margin-trim:all;display:flow-root}}");
    let [container, first, _] = boxes(&render(
        &css,
        "<section class=c><div class=b style='float:none;width:auto;height:5pt;margin:0'></div><div class=a></div></section>",
    ));
    assert_labelled_pt(
        "a float below in-flow content keeps its block-start margin",
        first.y - (content_top(container) + 5.0),
        MARGIN,
    );
}

#[test]
fn flex_containers_trim_the_margins_adjoining_their_block_edges() {
    let items = format!(".a,.b{{width:100pt;height:{CHILD}pt;margin:{MARGIN}pt}}");
    let body = "<section class=c><div class=a></div><div class=b></div></section>";
    let css = format!("{items}.c{{display:flex;margin-trim:in-flow}}");
    let [container, first, second] = boxes(&render(&css, body));
    assert_labelled_pt(
        "row: items on the only line lose the top margin",
        first.y,
        content_top(container),
    );
    assert_labelled_pt("row: and the bottom margin", container.height - 2.0, CHILD);
    assert_labelled_pt(
        "row: inline margins stay",
        second.x - (first.x + first.width),
        2.0 * MARGIN,
    );
    let css = format!("{items}.c{{display:flex;flex-wrap:wrap;width:150pt;margin-trim:in-flow}}");
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
        container.y + container.height - 1.0 - (second.y + second.height),
        0.0,
    );
    for (direction, start, end) in [("column", FIRST, SECOND), ("column-reverse", SECOND, FIRST)] {
        let css =
            format!("{items}.c{{display:flex;flex-direction:{direction};margin-trim:in-flow}}");
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
            container.y + container.height - 1.0 - (end.y + end.height),
            0.0,
        );
        assert_labelled_pt(
            &format!("{direction}: between items"),
            container.height - 2.0,
            2.0 * CHILD + 2.0 * MARGIN,
        );
    }
}

#[test]
fn grid_containers_trim_the_margins_in_their_first_and_last_rows() {
    let css = format!(
        ".c{{display:grid;margin-trim:in-flow}}.a,.b{{height:{CHILD}pt;margin:{MARGIN}pt 0}}"
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
        container.y + container.height - 1.0 - (second.y + second.height),
        0.0,
    );
}

#[test]
fn cloned_fragments_trim_margins_kept_after_a_forced_break() {
    for (decoration, trim, kept) in [
        ("clone", "in-flow", 0.0),
        ("clone", "none", MARGIN),
        ("slice", "in-flow", MARGIN),
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
