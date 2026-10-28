mod common;

use common::{PROFILE, assert_pt, local_options, painted_box};
use ferri_pdf::{DisplayItem, LayoutTrace, render_html};

const RED: [u8; 3] = [u8::MAX, 0, 0];
const BLUE: [u8; 3] = [0, 0, u8::MAX];
const BODY: &str = "<table><tr><td><div></div></td></tr></table><p>After</p>";

fn render(body: &str, css: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}@page{{size:300pt 400pt;margin:20pt}}body,p{{margin:0}}table{{border-spacing:0;background:red}}td{{padding:0}}td div{{width:10pt;height:10pt}}{css}</style>{body}"),
        &local_options(),
    )
    .expect("table width constraints must render through shared sizing")
    .layout
    .unwrap()
}

#[test]
fn table_constraints_resolve_before_distribution_and_keep_intrinsic_floors() {
    for collapse in ["separate", "collapse"] {
        for (css, expected) in [
            ("width:100pt;max-width:60pt", 60.0),
            ("width:100pt;min-width:120pt;max-width:60pt", 120.0),
            ("width:auto;min-width:80pt", 80.0),
            ("width:100pt;max-width:5pt", 10.0),
            ("width:100%;max-width:50%", 130.0),
        ] {
            let trace = render(BODY, &format!("table{{border-collapse:{collapse};{css}}}"));
            assert_pt(painted_box(&trace, RED).width, expected);
        }
    }
}

#[test]
fn row_and_group_width_constraints_do_not_resize_the_grid() {
    for selector in ["tr", "tbody"] {
        let trace = render(
            BODY,
            &format!("{selector}{{min-width:200pt;max-width:1pt}}"),
        );
        assert_pt(painted_box(&trace, RED).width, 10.0);
    }
}

#[test]
fn column_minimums_and_percentage_maximums_contribute_to_tracks() {
    let body = "<table><colgroup><col class=first><col></colgroup><tr><td class=a><div></div></td><td><div></div></td></tr></table>";
    for (css, table, first) in [
        (".first{min-width:40pt}", 50.0, 40.0),
        (
            ".first{width:30pt;min-width:40pt;max-width:20pt}",
            50.0,
            40.0,
        ),
        (".first{min-width:90%}", 20.0, 10.0),
        (
            "table{width:100pt}.first{width:80%;max-width:40%}",
            100.0,
            40.0,
        ),
    ] {
        let trace = render(body, &format!(".a{{background:blue}}{css}"));
        assert_pt(painted_box(&trace, RED).width, table);
        assert_pt(painted_box(&trace, BLUE).width, first);
    }
}

#[test]
fn fixed_layout_keeps_length_minimums_and_caps_percentage_column_contributions() {
    let body = "<table><col class=first><col><tr><td class=a><div></div></td><td><div></div></td></tr></table>";
    for (css, expected) in [
        ("width:30pt;min-width:40pt;max-width:20pt", 40.0),
        ("width:80%;max-width:40%", 40.0),
        ("min-width:80pt", 50.0),
    ] {
        let trace = render(
            body,
            &format!("table{{width:100pt;table-layout:fixed}}.a{{background:blue}}.first{{{css}}}"),
        );
        assert_pt(painted_box(&trace, BLUE).width, expected);
    }
}

#[test]
fn root_constraints_account_for_edges_captions_empty_grids_and_auto_margins() {
    for (sizing, expected) in [("content-box", 84.0), ("border-box", 60.0)] {
        let trace = render(
            BODY,
            &format!(
                "table{{width:100pt;max-width:60pt;box-sizing:{sizing};padding:10pt;border:2pt solid black;margin:auto}}"
            ),
        );
        let table = painted_box(&trace, RED);
        assert_pt(table.width, expected);
        assert_pt(table.x, 20.0 + (260.0 - expected) / 2.0);
    }
    let empty = render(
        "<table></table>",
        "table{height:10pt;min-width:80pt;max-width:20pt}",
    );
    assert_pt(painted_box(&empty, RED).width, 80.0);
    let caption = render(
        "<table><caption><div style='width:100pt;height:10pt'></div></caption><tr><td><div></div></td></tr></table>",
        "table{max-width:20pt}",
    );
    assert_pt(painted_box(&caption, RED).width, 100.0);
}

#[test]
fn constrained_tables_feed_inline_and_nested_intrinsic_widths() {
    for (body, selector) in [
        (
            "<table><tr><td><div></div></td></tr></table>",
            "table{display:inline-table;min-width:80pt}",
        ),
        (
            "<table><tr><td><table class=inner><tr><td><div></div></td></tr></table></td></tr></table>",
            ".inner{background:blue;min-width:80pt}",
        ),
    ] {
        let trace = render(body, selector);
        assert_pt(painted_box(&trace, RED).width, 80.0);
    }
}

#[test]
fn percentage_root_constraints_follow_named_page_width_changes() {
    let trace = render(
        "<table><tr><td>First</td></tr><tr class=wide><td>Second</td></tr></table>",
        "@page wide{size:500pt 400pt;margin:20pt}table{width:100%;max-width:50%}.wide{page:wide;break-before:page}",
    );
    assert_eq!(trace.pages.len(), 2, "one source row per named page");
    for (page, expected) in [130.0, 230.0].into_iter().enumerate() {
        let widths: Vec<_> = trace.display_lists[page]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == RED
                        && paint.decoration.background.alpha == 1.0 =>
                {
                    Some(paint.rect.width)
                }
                _ => None,
            })
            .collect();
        assert!(!widths.is_empty(), "page {page}: missing table background");
        for width in widths {
            assert_pt(width, expected);
        }
    }
}

#[test]
fn constrained_positioned_table_recalculates_end_insets() {
    for position in ["absolute", "fixed"] {
        let trace = render(
            BODY,
            &format!("table{{position:{position};right:0;top:0;width:100pt;max-width:60pt}}"),
        );
        let table = painted_box(&trace, RED);
        assert_pt(table.width, 60.0);
        assert_pt(table.x, 220.0);
    }
}

#[test]
fn column_spans_and_group_minimums_apply_once_per_track() {
    for columns in [
        "<col span=2 style='min-width:40pt'>",
        "<colgroup span=2 style='min-width:40pt'></colgroup>",
    ] {
        let trace = render(
            &format!("<table>{columns}<tr><td><div></div></td><td><div></div></td></tr></table>"),
            "",
        );
        assert_pt(painted_box(&trace, RED).width, 80.0);
    }
    let trace = render(
        "<table><tr><td><div></div></td></tr><tr><td><div style='width:100pt'></div></td></tr></table>",
        "table{table-layout:fixed;min-width:40pt}",
    );
    assert_pt(painted_box(&trace, RED).width, 100.0);
}

#[test]
fn cyclic_percentage_maximum_does_not_collapse_intrinsic_wrapper_width() {
    let body = "<section><table><tr><td>alpha beta gamma delta epsilon</td></tr></table></section>";
    let control = render(body, "section{display:inline-block;background:blue}");
    let constrained = render(
        body,
        "section{display:inline-block;background:blue}table{max-width:50%}",
    );
    let wrapper = painted_box(&constrained, BLUE);
    assert_pt(wrapper.width, painted_box(&control, BLUE).width);
    assert_pt(painted_box(&constrained, RED).width, wrapper.width / 2.0);
    assert!(
        constrained.pages[0].len() > control.pages[0].len(),
        "the used maximum wraps text without shrinking the intrinsic wrapper"
    );
}

#[test]
fn cyclic_minimum_retains_its_length_component_in_intrinsic_sizing() {
    let trace = render(
        "<section><table><tr><td><div></div></td></tr></table></section>",
        "section{display:inline-block;background:blue}table{min-width:calc(80pt + 10%)}",
    );
    assert_pt(painted_box(&trace, BLUE).width, 80.0);
    assert_pt(painted_box(&trace, RED).width, 88.0);
}
