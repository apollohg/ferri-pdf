mod common;

use common::{PROFILE, assert_pt, local_options, painted_box};
use ferri_pdf::{DisplayItem, LayoutTrace, render_html};

const RED: [u8; 3] = [u8::MAX, 0, 0];
const BLUE: [u8; 3] = [0, 0, u8::MAX];
const TWO_ROWS: &str = "<table><tbody><tr><td class=first><div class=short></div></td></tr><tr><td class=second><div class=tall></div></td></tr></tbody></table><p>After</p>";

fn render(body: &str, css: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}@page{{size:300pt 400pt;margin:20pt}}body,p,div{{margin:0}}table{{width:160pt;border-spacing:0}}td{{padding:0;border:0;vertical-align:top}}.first{{background:red}}.second{{background:blue}}.short{{height:10pt}}.tall{{height:30pt}}{css}</style>{body}"),
        &local_options(),
    )
    .expect("table height constraints must render through the shared table layout")
    .layout
    .unwrap()
}

#[test]
fn table_height_constraints_distribute_rows_without_shrinking_intrinsic_content() {
    for collapse in ["separate", "collapse"] {
        for (constraint, expected) in [
            ("height:100pt", [40.0, 60.0]),
            ("height:20pt", [10.0, 30.0]),
            ("min-height:100pt;max-height:20pt", [40.0, 60.0]),
            ("height:100pt;max-height:60pt", [20.0, 40.0]),
            ("max-height:0", [10.0, 30.0]),
        ] {
            let trace = render(
                TWO_ROWS,
                &format!("table{{border-collapse:{collapse};{constraint}}}"),
            );
            let first = painted_box(&trace, RED);
            let second = painted_box(&trace, BLUE);
            assert_pt(first.height, expected[0]);
            assert_pt(second.height, expected[1]);
            assert_pt(second.y, first.y + first.height);
            let after = trace.pages[0]
                .iter()
                .find(|line| line.text == "After")
                .unwrap();
            assert!(
                after.baseline > second.y + second.height,
                "{collapse}/{constraint}: following flow overlaps assigned rows"
            );
        }
    }
}

#[test]
fn fixed_and_percentage_rows_use_reference_sizes_and_auto_row_distribution() {
    for (constraint, expected) in [
        ("tr:first-child{height:40pt}", [40.0, 60.0]),
        ("tr:first-child{height:25%}", [25.0, 75.0]),
        ("tr{height:80%}", [45.0, 55.0]),
        ("td.first{height:40pt}", [40.0, 60.0]),
        ("td.first{height:25%}", [25.0, 75.0]),
    ] {
        let trace = render(TWO_ROWS, &format!("table{{height:100pt}}{constraint}"));
        assert_pt(painted_box(&trace, RED).height, expected[0]);
        assert_pt(painted_box(&trace, BLUE).height, expected[1]);
    }
}

#[test]
fn row_group_height_does_not_replace_row_height_distribution() {
    let trace = render(TWO_ROWS, "tbody{height:100pt}");
    assert_pt(painted_box(&trace, RED).height, 10.0);
    assert_pt(painted_box(&trace, BLUE).height, 30.0);
}

#[test]
fn percentage_descendants_relayout_only_when_the_table_or_cell_height_qualifies() {
    let body = "<table><tr><td class=cell><div class=child>X</div></td><td><div class=other></div></td></tr></table>";
    for (constraint, expected) in [
        ("table{height:100pt}", 50.0),
        (".cell{height:100pt}", 50.0),
        ("table{height:50%}", 20.0),
        (".cell{height:100%}", 10.0),
        ("table{min-height:100pt}", 10.0),
    ] {
        let trace = render(
            body,
            &format!(
                "td{{font:10pt/10pt Spike}}.child{{height:50%;background:red}}.other{{height:40pt}}{constraint}"
            ),
        );
        assert_pt(painted_box(&trace, RED).height, expected);
    }
}

#[test]
fn assigned_row_height_fragments_backgrounds_and_preserves_following_flow() {
    const TEXT_ROWS: &str = "<table><tr><td class=first>A</td></tr><tr><td class=second>B</td></tr></table><p>After</p>";
    for (body, constraint, heights) in [
        (TWO_ROWS, "table{height:200pt}", [90.0, 110.0]),
        (
            TWO_ROWS,
            "tr:first-child{height:90pt}tr:last-child{height:110pt}",
            [90.0, 110.0],
        ),
        (
            TEXT_ROWS,
            "table{height:200pt}td{font:10pt/10pt Spike}",
            [100.0, 100.0],
        ),
        (
            TEXT_ROWS,
            "tr{height:100pt}td{font:10pt/10pt Spike}",
            [100.0, 100.0],
        ),
    ] {
        let trace = render(
            body,
            &format!("@page{{size:300pt 100pt;margin:20pt}}{constraint}"),
        );
        assert_eq!(
            trace.pages.len(),
            4,
            "{constraint}: assigned rows span four pages"
        );
        for (color, expected) in [(RED, heights[0]), (BLUE, heights[1])] {
            let mut total = 0.0;
            for (page, items) in trace.display_lists.iter().enumerate() {
                for item in items {
                    if let DisplayItem::Box(paint) = item
                        && paint.decoration.background.rgb == color
                        && paint.decoration.background.alpha == 1.0
                    {
                        assert!(
                            paint.rect.y >= 20.0 && paint.rect.y + paint.rect.height <= 80.001,
                            "{constraint}, page {page}: background exceeds content area: {:?}",
                            paint.rect
                        );
                        total += paint.rect.height;
                    }
                }
            }
            assert_pt(total, expected);
        }
        let after = trace
            .pages
            .iter()
            .enumerate()
            .flat_map(|(page, lines)| {
                lines
                    .iter()
                    .filter(|line| line.text == "After")
                    .map(move |line| (page, line))
            })
            .collect::<Vec<_>>();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].0, 3);
        assert!(
            after[0].1.baseline > 40.0 && after[0].1.baseline <= 80.0,
            "{constraint}: following paragraph must follow the final 20pt of the grid"
        );
    }
}

#[test]
fn positioned_cells_align_against_the_assigned_row_height() {
    let body = "<table><tr><td><div class=child>X</div></td><td><div class=other></div></td></tr></table><p>After</p>";
    for position in ["static", "relative"] {
        for (alignment, expected_y) in [("middle", 65.0), ("bottom", 110.0)] {
            let trace = render(
                body,
                &format!(
                    "table{{height:100pt}}td{{position:{position};vertical-align:{alignment};font:10pt/10pt Spike}}.child{{height:10pt;background:red}}.other{{height:40pt}}"
                ),
            );
            let rect = painted_box(&trace, RED);
            let after = trace.pages[0]
                .iter()
                .find(|line| line.text == "After")
                .unwrap();
            assert!(
                after.baseline > 120.0 && after.baseline < 140.0,
                "{position}/{alignment}: alignment must not enlarge the 100pt table: {}",
                after.baseline
            );
            assert!(
                (rect.y - expected_y).abs() < 0.001,
                "{position}/{alignment}: expected child y={expected_y}, got {rect:?}"
            );
        }
    }
}

#[test]
fn percentage_content_overflow_does_not_redistribute_the_grid() {
    let trace = render(
        "<table><tr><td style='background:blue'><div class=child>X</div></td></tr></table><p>After</p>",
        "table{height:100pt}td{font:10pt/10pt Spike}.child{height:200%;background:red}",
    );
    assert_pt(painted_box(&trace, RED).height, 200.0);
    assert_pt(painted_box(&trace, BLUE).height, 100.0);
    let after = trace.pages[0]
        .iter()
        .find(|line| line.text == "After")
        .unwrap();
    assert!(
        after.baseline > 120.0 && after.baseline < 140.0,
        "overflow must not increase assigned grid height: {}",
        after.baseline
    );
}

#[test]
fn percentage_relayout_preserves_alignment_origin_without_growing_the_table() {
    let body = "<table><tr><td><div class=child>X</div></td><td><div class=other></div></td></tr></table><p>After</p>";
    for alignment in ["middle", "bottom"] {
        let css = format!(
            "table{{height:100pt}}td{{vertical-align:{alignment};font:10pt/10pt Spike}}.child{{background:red}}.other{{height:40pt}}"
        );
        let first = render(body, &format!("{css}.child{{height:10pt}}"));
        let second = render(body, &format!("{css}.child{{height:50%}}"));
        assert_pt(painted_box(&second, RED).height, 50.0);
        assert_pt(painted_box(&second, RED).y, painted_box(&first, RED).y);
        let after = |trace: &LayoutTrace| {
            trace.pages[0]
                .iter()
                .find(|line| line.text == "After")
                .unwrap()
                .baseline
        };
        assert_pt(after(&second), after(&first));
    }
}

#[test]
fn later_spanning_row_keeps_its_percentage_basis_on_a_wider_page() {
    let trace = render(
        "<table><tr><td rowspan=2><div class=span></div></td><td style='background:blue'>Top</td></tr><tr><td><div class=child>X</div></td></tr></table><p>After</p>",
        "@page{size:300pt 100pt;margin:20pt}@page:left{size:500pt 100pt}table{width:100%;height:200pt;table-layout:fixed}td{font:10pt/10pt Spike}.span{height:200pt}.child{height:50%;background:red}",
    );
    let red = trace
        .display_lists
        .iter()
        .enumerate()
        .flat_map(|(page, items)| {
            items.iter().filter_map(move |item| match item {
                DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == RED
                        && paint.decoration.background.alpha == 1.0 =>
                {
                    Some((page, paint.rect))
                }
                _ => None,
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        red.first().map(|(page, _)| *page),
        Some(1),
        "second row begins on the wider page: {red:?}"
    );
    assert_pt(red[0].1.x, 250.0);
    assert_pt(red.iter().map(|(_, rect)| rect.height).sum(), 50.0);
    let first_row_height: f32 = trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            DisplayItem::Box(paint)
                if paint.decoration.background.rgb == BLUE
                    && paint.decoration.background.alpha == 1.0 =>
            {
                Some(paint.rect.height)
            }
            _ => None,
        })
        .sum();
    assert_pt(first_row_height, 100.0);
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .filter(|line| line.text == "X")
            .count(),
        1
    );
}

#[test]
fn table_height_box_sizing_excludes_captions_and_accounts_for_edges() {
    let control = render("<p>After</p>", "");
    let baseline = control.pages[0][0].baseline;
    for (sizing, outer, inner) in [("border-box", 100.0, 88.0), ("content-box", 112.0, 100.0)] {
        for side in ["top", "bottom"] {
            let trace = render(
                "<table><caption>Caption</caption><tr><td><div style='height:10pt'></div></td></tr></table><p>After</p>",
                &format!(
                    "table{{height:100pt;box-sizing:{sizing};border:2pt solid black;padding:4pt;background:blue}}td{{background:red}}caption{{caption-side:{side};height:20pt;margin:0;padding:0;font:10pt/10pt Spike}}"
                ),
            );
            let grid = painted_box(&trace, BLUE);
            let cell = painted_box(&trace, RED);
            assert_pt(grid.y, if side == "top" { 40.0 } else { 20.0 });
            assert_pt(grid.height, outer);
            assert_pt(cell.y, grid.y + 6.0);
            assert_pt(cell.height, inner);
            let after = trace.pages[0]
                .iter()
                .find(|line| line.text == "After")
                .unwrap();
            assert_pt(after.baseline, baseline + outer + 20.0);
        }
    }
}

#[test]
fn empty_table_height_occupies_flow_without_creating_rows() {
    let control = render("<p>After</p>", "");
    let trace = render(
        "<table></table><p>After</p>",
        "table{height:100pt;background:blue}",
    );
    assert_pt(painted_box(&trace, BLUE).height, 100.0);
    let after = trace.pages[0]
        .iter()
        .find(|line| line.text == "After")
        .unwrap();
    assert_pt(after.baseline, control.pages[0][0].baseline + 100.0);
}

#[test]
fn first_pass_percentage_scroll_boxes_have_zero_height_but_keep_minimums() {
    let control = render("<p>After</p>", "");
    let baseline = control.pages[0][0].baseline;
    for (overflow, minimum, expected) in [
        ("auto", 0.0, 0.0),
        ("scroll", 0.0, 0.0),
        ("auto", 20.0, 20.0),
        ("hidden", 0.0, 10.0),
        ("clip", 0.0, 10.0),
        ("visible", 0.0, 10.0),
    ] {
        let trace = render(
            "<table><tr><td><div class=child>X</div></td></tr></table><p>After</p>",
            &format!(
                "td{{font:10pt/10pt Spike}}.child{{height:50%;overflow:{overflow};min-height:{minimum}pt}}"
            ),
        );
        let after = trace.pages[0]
            .iter()
            .find(|line| line.text == "After")
            .unwrap();
        assert!(
            (after.baseline - baseline - expected).abs() < 0.001,
            "overflow={overflow}, min-height={minimum}: expected grid height {expected}, got {}",
            after.baseline - baseline
        );
    }
}

#[test]
fn first_pass_zero_height_follows_percentage_chains_but_not_auto_wrappers() {
    let control = render("<p>After</p>", "");
    let baseline = control.pages[0][0].baseline;
    for (wrapper, expected) in [("50%", 0.0), ("auto", 10.0)] {
        let trace = render(
            "<table><tr><td><div class=wrapper><div class=child>X</div></div></td></tr></table><p>After</p>",
            &format!(
                "td{{font:10pt/10pt Spike}}.wrapper{{height:{wrapper}}}.child{{height:50%;overflow:auto}}"
            ),
        );
        let after = trace.pages[0]
            .iter()
            .find(|line| line.text == "After")
            .unwrap();
        assert!(
            (after.baseline - baseline - expected).abs() < 0.001,
            "wrapper height {wrapper}: expected table height {expected}, got {}",
            after.baseline - baseline
        );
    }
}

#[test]
fn assigned_body_rows_fragment_between_repeated_fixed_height_groups() {
    for alignment in ["top", "middle", "bottom"] {
        let trace = render(
            "<table><thead><tr><td>H</td></tr></thead><tbody><tr><td class=first>A</td></tr><tr><td class=second>B</td></tr></tbody><tfoot><tr><td>F</td></tr></tfoot></table><p>After</p>",
            &format!(
                "@page{{size:300pt 100pt;margin:20pt}}table{{height:200pt}}td{{font:10pt/10pt Spike;vertical-align:{alignment}}}thead tr,tfoot tr{{height:10pt}}"
            ),
        );
        assert_eq!(
            trace.pages.len(),
            5,
            "180pt of body space fits 40pt per repeated page"
        );
        for (page, lines) in trace.pages.iter().enumerate() {
            for label in ["H", "F"] {
                assert_eq!(
                    lines.iter().filter(|line| line.text == label).count(),
                    1,
                    "page {page}: repeated {label} appears exactly once"
                );
            }
        }
        for label in ["A", "B", "After"] {
            assert_eq!(
                trace
                    .pages
                    .iter()
                    .flatten()
                    .filter(|line| line.text == label)
                    .count(),
                1,
                "body label {label} appears exactly once"
            );
        }
        for color in [RED, BLUE] {
            let height: f32 = trace
                .display_lists
                .iter()
                .flatten()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint)
                        if paint.decoration.background.rgb == color
                            && paint.decoration.background.alpha == 1.0 =>
                    {
                        Some(paint.rect.height)
                    }
                    _ => None,
                })
                .sum();
            assert!(
                (height - 90.0).abs() < 0.001,
                "{alignment}/{color:?}: expected 90pt, got {height}; fragments {:?}",
                trace
                    .display_lists
                    .iter()
                    .enumerate()
                    .flat_map(
                        |(page, items)| items.iter().filter_map(move |item| match item {
                            DisplayItem::Box(paint)
                                if paint.decoration.background.rgb == color
                                    && paint.decoration.background.alpha == 1.0 =>
                                Some((page, paint.rect)),
                            _ => None,
                        })
                    )
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn assigned_table_height_continues_across_columns_with_its_definite_basis() {
    let control = render("<p>After</p>", "");
    for height in ["100pt", "125%"] {
        let trace = render(
            "<section><table><tr><td class=first>A</td></tr><tr><td class=second>B</td></tr></table><p>After</p></section>",
            &format!(
                "section{{height:80pt;column-count:2;column-gap:10pt;column-fill:auto}}table{{width:100%;height:{height}}}td{{font:10pt/10pt Spike}}"
            ),
        );
        assert_eq!(
            trace.pages.len(),
            1,
            "{height}: the table fits two columns on one page"
        );
        for color in [RED, BLUE] {
            let total: f32 = trace
                .display_lists
                .iter()
                .flatten()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint)
                        if paint.decoration.background.rgb == color
                            && paint.decoration.background.alpha == 1.0 =>
                    {
                        Some(paint.rect.height)
                    }
                    _ => None,
                })
                .sum();
            assert_pt(total, 50.0);
        }
        let after = trace.pages[0]
            .iter()
            .find(|line| line.text == "After")
            .unwrap();
        assert_pt(after.x, 155.0);
        assert_pt(after.baseline, control.pages[0][0].baseline + 20.0);
    }
}

#[test]
fn spanning_height_constraints_share_rows_and_preserve_the_percentage_basis() {
    for collapse in ["separate", "collapse"] {
        for (constraint, expected) in [("80pt", [70.0, 30.0]), ("120%", [75.0, 25.0])] {
            let trace = render(
                "<table><tr><td class=span rowspan=2><div class=child></div></td><td class=first><div class=upper></div></td></tr><tr><td class=second><div class=lower></div></td></tr></table><p>After</p>",
                &format!(
                    "table{{height:100pt;border-collapse:{collapse}}}.span{{height:{constraint}}}.child{{height:50%;background:green}}.upper{{height:30pt}}.lower{{height:10pt}}"
                ),
            );
            let first = painted_box(&trace, RED);
            let second = painted_box(&trace, BLUE);
            assert_pt(first.height, expected[0]);
            assert_pt(second.height, expected[1]);
            assert_pt(second.y, first.y + first.height);
            assert_pt(painted_box(&trace, [0, 128, 0]).height, 50.0);
        }
    }
}
