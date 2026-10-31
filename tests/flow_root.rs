mod common;

use common::{assert_pt, local_options, painted_box};
use ferri_pdf::{LayoutTrace, render_html};

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!(
            "<style>{}p,div,ol,li{{margin:0}}p{{font:10pt/12pt Spike}}{css}</style>{body}",
            common::PROFILE
        ),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("flow-root fixture: {error}; CSS={css}; HTML={body}"))
    .layout
    .unwrap()
}

#[test]
fn block_roots_contain_floats_without_clipping_visible_overflow() {
    for display in ["flow-root", "block flow-root", "flow-root block"] {
        let trace = render(
            &format!(
                ".root{{display:{display};width:80pt;background:rgb(1,2,3)}}.float{{float:left;width:40pt;height:48pt}}.overflow{{width:120pt;height:12pt;background:rgb(4,5,6)}}"
            ),
            "<div class=root><div class=float></div><div class=overflow></div></div><p>After</p>",
        );
        let root = painted_box(&trace, [1, 2, 3]);
        assert_pt(root.height, 48.0);
        assert_pt(painted_box(&trace, [4, 5, 6]).width, 120.0);
        assert!(
            trace.pages[0]
                .iter()
                .find(|line| line.text == "After")
                .unwrap()
                .baseline
                > root.y + root.height
        );
    }
}

#[test]
fn roots_avoid_external_floats_and_contain_child_margins() {
    let trace = render(
        ".float{float:left;width:60pt;height:48pt}.root{display:flow-root;width:100pt;background:rgb(1,2,3)}p{margin:12pt 0}",
        "<div class=float></div><div class=root><p>Inside</p></div>",
    );
    let root = painted_box(&trace, [1, 2, 3]);
    assert_pt(root.x, 96.0);
    assert_pt(root.y, 36.0);
    assert_pt(root.height, 36.0);
}

/// The container's, the float's and the formatting context root's painted boxes, the root
/// styled `root` beside a float styled `float` in a 300pt container.
fn beside_float(float: &str, root: &str) -> [ferri_pdf::Rect; 3] {
    let trace = render(
        &format!(
            ".c{{width:300pt;background:rgb(7,8,9)}}\
             .f{{width:100pt;height:50pt;background:rgb(4,5,6);{float}}}\
             .r{{height:20pt;background:rgb(1,2,3);{root}}}"
        ),
        "<div class=c><div class=f></div><div class=r></div></div>",
    );
    [[7, 8, 9], [4, 5, 6], [1, 2, 3]].map(|color| painted_box(&trace, color))
}

#[test]
fn roots_fill_the_space_beside_floats_with_auto_and_stretch_widths() {
    // CSS 2.1 §9.5 and CSS Sizing 3 §4.2: a formatting context root's border box does not
    // overlap floats, and an automatic or stretch width fills the space beside them.
    for root in [
        "overflow:hidden",
        "display:flow-root",
        "display:flow-root;width:stretch",
        "display:flex",
    ] {
        let [container, float, left] = beside_float("float:left", root);
        assert_pt(left.x, float.x + float.width);
        assert_pt(left.y, container.y);
        assert_pt(left.width, container.width - float.width);
        let [container, float, right] = beside_float("float:right", root);
        assert_pt(right.x, container.x);
        assert_pt(right.y, container.y);
        assert_pt(right.width, float.x - container.x);
    }
}

#[test]
fn roots_beside_floats_keep_their_margins_and_move_below_where_they_cannot_fit() {
    let [container, float, root] = beside_float("float:left", "display:flow-root;margin-left:10pt");
    assert_pt(root.x, float.x + float.width + 10.0);
    assert_pt(root.width, container.width - float.width - 10.0);
    // A minimum wider than the space beside the float fills the containing block below it.
    let [container, float, root] = beside_float("float:left", "display:flow-root;min-width:250pt");
    assert_pt(root.y, float.y + float.height);
    assert_pt(root.x, container.x);
    assert_pt(root.width, container.width);
    // A root taller than the float stays narrowed beside it along its whole height.
    let [container, float, root] = beside_float("float:left", "display:flow-root;height:80pt");
    assert_pt(root.y, container.y);
    assert_pt(root.width, container.width - float.width);
}

#[test]
fn flow_root_list_items_keep_counters_and_contain_their_floats() {
    for display in [
        "flow-root list-item",
        "block flow-root list-item",
        "list-item flow-root",
    ] {
        let trace = render(
            &format!(
                "li{{display:{display};list-style-position:inside}}.float{{float:left;width:40pt;height:48pt}}"
            ),
            "<ol start=3><li><div class=float></div><p>First</p></li><li value=7>Second</li><li>Third</li></ol>",
        );
        let text = trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<String>();
        for marker in ["3.", "7. ", "8. "] {
            assert!(text.contains(marker), "{display}: {text:?}");
        }
        let second = trace.pages[0]
            .iter()
            .find(|line| line.text.contains("Second"))
            .unwrap();
        assert!(second.baseline > 36.0 + 48.0);
        let reversed = render(
            &format!("li{{display:{display};list-style-position:inside}}"),
            "<ol reversed><li>First</li><li>Second</li><li>Third</li></ol>",
        );
        assert_eq!(
            reversed
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["3. First", "2. Second", "1. Third"],
            "automatic reversed values for {display}",
        );
    }
}

#[test]
fn named_roots_continue_on_pages_and_capability_queries_match() {
    let trace = render(
        "@page chapter{size:150pt 100pt;margin:10pt}.root{page:chapter;width:1pt;background:rgb(1,2,3)}p{height:30pt;orphans:1;widows:1}@supports(display:flow-root){.root{display:flow-root;width:100pt}}",
        "<div class=root><p>One</p><p>Two</p><p>Three</p><p>Four</p></div>",
    );
    assert!(trace.pages.len() >= 2);
    for item in trace.display_lists.iter().flatten() {
        if let ferri_pdf::DisplayItem::Box(paint) = item
            && paint.decoration.background.rgb == [1, 2, 3]
        {
            assert_pt(paint.rect.width, 100.0);
        }
    }
    for line in trace.pages.iter().flatten() {
        assert_pt(line.x, 10.0);
    }
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["One", "Two", "Three", "Four"]
    );
}
