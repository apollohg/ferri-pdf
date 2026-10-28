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
