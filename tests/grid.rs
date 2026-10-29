mod common;
use common::{PROFILE, assert_pt, local_options, painted_box};
use ferri_pdf::{LayoutTrace, render_html};

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}html,body,p{{margin:0}}.grid{{display:grid}}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("grid rendering failed for {css:?}: {error}"))
    .layout
    .expect("layout requested")
}

#[test]
fn fixed_and_fractional_tracks_use_real_item_boxes() {
    let trace = render(
        ".grid{width:240pt;grid-template-columns:60pt 1fr 2fr;gap:10pt}.grid>div{height:20pt}.a{background:red}.b{background:lime}.c{background:blue}",
        "<div class=grid><div class=a>A</div><div class=b>B</div><div class=c>C</div></div><p>After</p>",
    );
    for (color, x, width) in [
        ([255, 0, 0], 36.0, 60.0),
        ([0, 255, 0], 106.0, 160.0 / 3.0),
        ([0, 0, 255], 116.0 + 160.0 / 3.0, 320.0 / 3.0),
    ] {
        let rect = painted_box(&trace, color);
        assert_pt(rect.x, x);
        assert_pt(rect.y, 36.0);
        assert_pt(rect.width, width);
        assert_pt(rect.height, 20.0);
    }
    let a = trace.pages[0].iter().find(|line| line.text == "A").unwrap();
    let after = trace.pages[0]
        .iter()
        .find(|line| line.text == "After")
        .unwrap();
    assert_pt(after.baseline - a.baseline, 20.0);
}

#[test]
fn named_areas_span_tracks_and_keep_gaps_inside_the_area() {
    let trace = render(
        ".grid{width:210pt;grid-template-columns:80pt 120pt;grid-template-rows:30pt 40pt;gap:10pt;grid-template-areas:'head head' 'side main'}.head{grid-area:head;background:red}.side{grid-area:side;background:lime}.main{grid-area:main;background:blue}",
        "<div class=grid><div class=main>Main</div><div class=head>Heading</div><div class=side>Side</div></div>",
    );
    for (color, x, y, width, height) in [
        ([255, 0, 0], 36.0, 36.0, 210.0, 30.0),
        ([0, 255, 0], 36.0, 76.0, 80.0, 40.0),
        ([0, 0, 255], 126.0, 76.0, 120.0, 40.0),
    ] {
        let rect = painted_box(&trace, color);
        assert_pt(rect.x, x);
        assert_pt(rect.y, y);
        assert_pt(rect.width, width);
        assert_pt(rect.height, height);
    }
}

#[test]
fn dense_grid_placement_backfills_real_content_without_reordering_it() {
    let trace = render(
        ".grid{width:150pt;grid-template-columns:repeat(3,50pt);grid-auto-rows:20pt;grid-auto-flow:row dense}.wide{grid-column:span 2}.last{background:blue}",
        "<div class=grid><div class=wide>First</div><div class=wide>Second</div><div class=last>Third</div></div>",
    );
    let last = painted_box(&trace, [0, 0, 255]);
    assert_pt(last.x, 136.0);
    assert_pt(last.y, 36.0);
    assert_pt(last.width, 50.0);
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["First", "Second", "Third"]
    );
}

#[test]
fn grid_rows_paginate_without_losing_or_duplicating_content() {
    let trace = render(
        "@page{size:240pt 120pt;margin:20pt}.grid{grid-template-columns:1fr 1fr;grid-auto-rows:40pt}p{font:10pt/14pt Spike}",
        &format!(
            "<div class=grid>{}</div>",
            (0..8)
                .map(|index| format!("<p>Cell{index}</p>"))
                .collect::<String>()
        ),
    );
    assert_eq!(trace.pages.len(), 2);
    for (page, labels) in [
        ["Cell0", "Cell1", "Cell2", "Cell3"],
        ["Cell4", "Cell5", "Cell6", "Cell7"],
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(
            trace.pages[page]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            *labels,
            "page {page}: each grid item must survive once; positions {:?}",
            trace.pages[page]
                .iter()
                .map(|line| (&line.text, line.baseline))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn table_items_distribute_grid_assigned_height_through_their_rows() {
    for (constraint, expected) in [("", 80.0), ("max-height:40pt", 40.0)] {
        let trace = render(
            &format!(
                ".grid{{grid-template-columns:120pt;grid-template-rows:80pt}}table{{border-spacing:0;background:lime;{constraint}}}td{{padding:0;vertical-align:top;font:12pt/18pt Spike}}.fill{{height:100%;background:red}}"
            ),
            "<div class=grid><table><tr><td><div class=fill>First</div></td></tr><tr><td>Second</td></tr></table></div><p>After</p>",
        );
        let rect = painted_box(&trace, [0, 255, 0]);
        assert_pt(rect.width, 120.0);
        assert_pt(rect.height, expected);
        assert_pt(painted_box(&trace, [255, 0, 0]).height, expected / 2.0);
        let line = |label| {
            trace.pages[0]
                .iter()
                .find(|line| line.text == label)
                .unwrap()
        };
        assert_pt(
            line("Second").baseline - line("First").baseline,
            expected / 2.0,
        );
        assert_pt(line("After").baseline - line("First").baseline, 80.0);
    }
}

#[test]
fn baseline_alignment_increases_row_size_for_the_aligned_item_extent() {
    let trace = render(
        ".grid{grid-template-columns:80pt 120pt;align-items:baseline;background:blue}.small{font:10pt/10pt Spike;padding-bottom:40pt;background:red}.large{font:30pt/30pt Spike;background:lime}",
        "<div class=grid><p class=small>Small</p><p class=large>Large</p></div><p>After</p>",
    );
    let line = |label| {
        trace.pages[0]
            .iter()
            .find(|line| line.text == label)
            .unwrap()
    };
    assert_pt(line("Small").baseline, line("Large").baseline);
    let grid = painted_box(&trace, [0, 0, 255]);
    let small = painted_box(&trace, [255, 0, 0]);
    let large = painted_box(&trace, [0, 255, 0]);
    assert_pt(small.height, 50.0);
    assert_pt(large.height, 30.0);
    assert!(
        small.y > large.y,
        "the smaller ascent must move to the shared baseline"
    );
    assert_pt(grid.y + grid.height, small.y + small.height);
    assert!(line("After").baseline > grid.y + grid.height);
}

#[test]
fn baseline_participation_excludes_cycles_and_auto_margins() {
    for (row, item, aligned) in [
        ("auto", "height:100%", false),
        ("80pt", "height:100%", true),
        ("80pt", "margin-top:auto", false),
    ] {
        let trace = render(
            &format!(
                ".grid{{grid-template-columns:80pt 120pt;grid-template-rows:{row};align-items:baseline;background:blue}}.small{{font:10pt/10pt Spike;background:red;{item}}}.large{{font:30pt/30pt Spike}}"
            ),
            "<div class=grid><p class=small>Small</p><p class=large>Large</p></div>",
        );
        let line = |label| {
            trace.pages[0]
                .iter()
                .find(|line| line.text == label)
                .unwrap()
        };
        let difference = (line("Small").baseline - line("Large").baseline).abs();
        assert_eq!(
            difference < 0.001,
            aligned,
            "row={row}, item={item}: baseline difference {difference}"
        );
        let grid = painted_box(&trace, [0, 0, 255]);
        let small = painted_box(&trace, [255, 0, 0]);
        if row == "auto" {
            assert_pt(grid.height, 30.0);
            assert_pt(small.y, grid.y);
        } else if !aligned {
            assert_pt(small.y + small.height, grid.y + grid.height);
        }
    }
}

#[test]
fn inline_grid_uses_the_first_grid_row_baseline_despite_source_order() {
    let trace = render(
        ".grid{display:inline-grid;grid-template-columns:80pt;grid-template-rows:40pt 40pt}.late{grid-row:2}.early{grid-row:1}",
        "<p>Anchor <span class=grid><span class=late>Late</span><span class=early>Early</span></span></p>",
    );
    let line = |label: &str| {
        trace.pages[0]
            .iter()
            .find(|line| line.text.starts_with(label))
            .unwrap()
    };
    assert_pt(line("Anchor").baseline, line("Early").baseline);
    assert_pt(line("Late").baseline - line("Early").baseline, 40.0);
}

#[test]
fn positioned_grid_areas_use_track_edges_but_static_alignment_uses_content_edges() {
    for (positioning, expected) in [
        ("inset:0", (134.0, 84.0, 120.0, 40.0)),
        (
            "width:20pt;height:10pt;justify-self:center;align-self:end",
            (139.0, 114.0, 20.0, 10.0),
        ),
    ] {
        let trace = render(
            &format!(
                ".grid{{position:relative;width:210pt;padding:8pt;grid-template-columns:80pt 120pt;grid-template-rows:30pt 40pt;gap:10pt}}.placed{{position:absolute;grid-column:2/3;grid-row:2/3;background:red;{positioning}}}"
            ),
            "<div class=grid><div class=placed></div></div>",
        );
        let rect = painted_box(&trace, [255, 0, 0]);
        assert_pt(rect.x, expected.0);
        assert_pt(rect.y, expected.1);
        assert_pt(rect.width, expected.2);
        assert_pt(rect.height, expected.3);
    }
}

#[test]
fn grid_containing_blocks_position_descendants_through_static_items() {
    for (container, child) in [
        ("position:relative", "position:absolute"),
        ("transform:translate(0,0)", "position:fixed"),
    ] {
        let trace = render(
            &format!(
                ".grid{{{container};width:210pt;padding:8pt;grid-template-columns:80pt 120pt;grid-template-rows:30pt 40pt;gap:10pt}}.placed{{{child};inset:0;grid-column:2/3;grid-row:2/3;background:red}}"
            ),
            "<div class=grid><div><div class=placed></div></div></div>",
        );
        let rect = painted_box(&trace, [255, 0, 0]);
        assert_pt(rect.x, 134.0);
        assert_pt(rect.y, 84.0);
        assert_pt(rect.width, 120.0);
        assert_pt(rect.height, 40.0);
    }
}

#[test]
fn absolute_grid_lines_respect_rtl_gutters_and_auto_padding_edges() {
    for (direction, placement, x, width) in [
        ("ltr", "2/3", 134.0, 120.0),
        ("rtl", "2/3", 44.0, 120.0),
        ("ltr", "2/auto", 134.0, 128.0),
        ("rtl", "2/auto", 36.0, 128.0),
        ("ltr", "auto/2", 36.0, 88.0),
        ("rtl", "auto/2", 174.0, 88.0),
    ] {
        let trace = render(
            &format!(
                ".grid{{position:relative;direction:{direction};width:210pt;padding:8pt;grid-template-columns:80pt 120pt;grid-template-rows:30pt;gap:10pt}}.placed{{position:absolute;inset:0;grid-column:{placement};grid-row:1/2;background:red}}"
            ),
            "<div class=grid><div class=placed></div></div>",
        );
        let rect = painted_box(&trace, [255, 0, 0]);
        assert_pt(rect.x, x);
        assert_pt(rect.width, width);
        assert_pt(rect.y, 44.0);
        assert_pt(rect.height, 30.0);
    }
}

#[test]
fn static_grid_parent_aligns_absolute_children_inside_ancestor_containing_block() {
    for (direction, alignment, x) in [
        ("ltr", "start", 44.0),
        ("rtl", "start", 234.0),
        ("rtl", "end", 44.0),
        ("rtl", "self-start", 44.0),
        ("rtl", "self-end", 234.0),
        ("rtl", "left", 44.0),
        ("rtl", "right", 234.0),
        ("rtl", "center", 139.0),
    ] {
        let trace = render(
            &format!(
                ".outer{{position:relative;width:300pt;height:160pt}}.grid{{direction:{direction};width:210pt;padding:8pt;grid-template-columns:80pt 120pt;grid-template-rows:80pt;justify-items:{alignment};align-items:end}}.placed{{position:absolute;direction:ltr;width:20pt;height:10pt;grid-column:2/3;background:red}}"
            ),
            "<div class=outer><div class=grid><div class=placed></div></div></div>",
        );
        let rect = painted_box(&trace, [255, 0, 0]);
        assert_pt(rect.x, x);
        assert_pt(rect.y, 114.0);
        assert_pt(rect.width, 20.0);
        assert_pt(rect.height, 10.0);
    }
}

#[test]
fn first_and_last_grid_row_breaks_propagate_to_the_container() {
    for (declaration, before_page, after_page) in [
        ("break-before:page", 1, 1),
        ("break-after:page", 0, 1),
        ("break-before:page;break-after:page", 1, 2),
    ] {
        let trace = render(
            &format!(
                ".grid{{grid-template-columns:80pt 80pt;grid-template-rows:30pt}}.break{{{declaration}}}"
            ),
            "<p>Before</p><div class=grid><div>Peer</div><div class=break>Break</div></div><p>After</p>",
        );
        for (label, expected) in [
            ("Before", 0),
            ("Peer", before_page),
            ("Break", before_page),
            ("After", after_page),
        ] {
            let pages: Vec<_> = trace
                .pages
                .iter()
                .enumerate()
                .filter_map(|(index, page)| {
                    page.iter().any(|line| line.text == label).then_some(index)
                })
                .collect();
            assert_eq!(
                pages,
                [expected],
                "{declaration}: {label} must appear exactly once on its propagated page"
            );
        }
    }
}

#[test]
fn internal_grid_item_breaks_leave_peers_independent_and_grow_auto_rows() {
    let trace = render(
        ".grid{grid-template-columns:100pt 100pt;grid-template-rows:auto auto}.continued{break-before:page}",
        "<div class=grid><div><p>First</p><p class=continued>Continued</p></div><div><p>Peer first</p><p>Peer second</p></div><div>Next row</div></div><p>After</p>",
    );
    for (label, expected_page) in [
        ("First", 0),
        ("Peer first", 0),
        ("Peer second", 0),
        ("Continued", 1),
        ("Next row", 1),
        ("After", 1),
    ] {
        let pages: Vec<_> = trace
            .pages
            .iter()
            .enumerate()
            .filter_map(|(index, page)| page.iter().any(|line| line.text == label).then_some(index))
            .collect();
        assert_eq!(
            pages,
            [expected_page],
            "{label}: item-internal breaks must expand its auto row without forcing peers"
        );
    }
    let line = |label| {
        trace.pages[1]
            .iter()
            .find(|line| line.text == label)
            .unwrap()
    };
    assert!(line("Next row").baseline > line("Continued").baseline);
    assert!(line("After").baseline > line("Next row").baseline);
}

#[test]
fn grid_row_boundaries_propagate_forced_breaks_across_peers() {
    for rule in [".first{break-after:page}", ".later{break-before:page}"] {
        let trace = render(
            &format!(
                ".grid{{grid-template-columns:100pt 100pt;grid-template-rows:30pt 30pt;gap:10pt}}{rule}"
            ),
            "<div class=grid><div class=first>First</div><div>First peer</div><div class=later>Later</div><div>Later peer</div></div>",
        );
        assert_eq!(trace.pages.len(), 2, "{rule}");
        for (page, labels) in [(0, ["First", "First peer"]), (1, ["Later", "Later peer"])] {
            let actual: Vec<_> = trace.pages[page]
                .iter()
                .map(|line| line.text.as_str())
                .collect();
            assert_eq!(actual, labels, "{rule}: all peers move at the row boundary");
        }
        assert_pt(trace.pages[0][0].baseline, trace.pages[1][0].baseline);
    }
}

#[test]
fn fixed_rows_and_spanning_items_do_not_grow_individual_grid_rows() {
    for (tracks, span) in [("40pt 40pt", false), ("auto auto", true)] {
        let area = if span { "grid-row:1/3" } else { "grid-row:1" };
        let trace = render(
            &format!(
                ".grid{{grid-template-columns:100pt 100pt;grid-template-rows:{tracks}}}.long{{{area};grid-column:1}}.peer{{grid-column:2;grid-row:1}}.next{{grid-column:2;grid-row:2}}.continued{{break-before:page}}"
            ),
            "<div class=grid><div class=long><p>First</p><p class=continued>Continued</p></div><div class=peer>Peer</div><div class=next>Next row</div></div>",
        );
        for (label, expected_page) in [("First", 0), ("Peer", 0), ("Next row", 0), ("Continued", 1)]
        {
            let pages: Vec<_> = trace
                .pages
                .iter()
                .enumerate()
                .filter_map(|(index, page)| {
                    page.iter().any(|line| line.text == label).then_some(index)
                })
                .collect();
            assert_eq!(
                pages,
                [expected_page],
                "tracks={tracks}, span={span}: {label}"
            );
        }
    }
}

#[test]
fn grid_gutters_and_distributed_spacing_are_suppressed_at_page_breaks() {
    for (spacing, rows) in [
        ("row-gap:20pt", "90pt 20pt"),
        ("row-gap:5pt", "90pt 20pt"),
        ("height:130pt;align-content:space-between", "90pt 20pt"),
    ] {
        let trace = render(
            &format!(
                "@page{{size:200pt 120pt;margin:10pt}}.grid{{grid-template-columns:100pt;grid-template-rows:{rows};{spacing}}}.first{{background:red}}.second{{background:blue}}"
            ),
            "<div class=grid><div class=first></div><div class=second>Second</div></div>",
        );
        let second = painted_box(&trace, [0, 0, 255]);
        assert_pt(second.y, 10.0);
        assert_eq!(
            trace
                .pages
                .iter()
                .position(|page| page.iter().any(|line| line.text == "Second")),
            Some(1),
            "{spacing}"
        );
    }
}

#[test]
fn avoid_breaks_keep_grid_row_groups_together_when_they_fit_a_fresh_page() {
    for (prefix, tracks, body, rule) in [
        (
            "<div style='height:70pt'></div>",
            "20pt 20pt",
            "<div class=one>One</div><div class=two>Two</div>",
            ".two{break-before:avoid}",
        ),
        (
            "",
            "60pt 20pt 20pt",
            "<div>Lead</div><div class=one>One</div><div class=two>Two</div>",
            ".one{break-after:avoid}",
        ),
    ] {
        let trace = render(
            &format!(
                "@page{{size:200pt 120pt;margin:10pt}}.grid{{grid-template-columns:100pt;grid-template-rows:{tracks};row-gap:5pt}}{rule}"
            ),
            &format!("{prefix}<div class=grid>{body}</div>"),
        );
        for label in ["One", "Two"] {
            let pages: Vec<_> = trace
                .pages
                .iter()
                .enumerate()
                .filter_map(|(index, page)| {
                    page.iter().any(|line| line.text == label).then_some(index)
                })
                .collect();
            assert_eq!(
                pages,
                [1],
                "{rule}: {label} belongs with the adjoining row on the fresh page"
            );
        }
        let one = trace.pages[1]
            .iter()
            .find(|line| line.text == "One")
            .unwrap();
        let two = trace.pages[1]
            .iter()
            .find(|line| line.text == "Two")
            .unwrap();
        assert_pt(two.baseline - one.baseline, 25.0);
    }
}

#[test]
fn grid_row_avoidance_yields_to_forced_breaks_and_oversized_groups() {
    for (tracks, rule) in [
        (
            "20pt 20pt",
            ".one{break-after:avoid}.two{break-before:page}",
        ),
        (
            "60pt 60pt",
            ".grid>div{break-inside:avoid}.one{break-after:avoid}",
        ),
    ] {
        let trace = render(
            &format!(
                "@page{{size:200pt 120pt;margin:10pt}}.grid{{grid-template-columns:100pt;grid-template-rows:{tracks}}}{rule}"
            ),
            "<div class=grid><div class=one>One</div><div class=two>Two</div></div>",
        );
        assert_eq!(trace.pages.len(), 2, "{tracks}: {rule}");
        for (index, label) in ["One", "Two"].into_iter().enumerate() {
            let actual: Vec<_> = trace.pages[index]
                .iter()
                .map(|line| line.text.as_str())
                .collect();
            assert_eq!(actual, [label], "{tracks}: {rule}");
        }
    }
}

#[test]
fn continuing_grid_items_use_the_destination_page_column_widths() {
    for (first_width, later_width) in [(220, 420), (420, 220)] {
        let trace = render(
            &format!(
                "@page{{size:{later_width}pt 180pt;margin:10pt}}@page:first{{size:{first_width}pt 180pt}}.grid{{grid-template-columns:1fr 1fr;gap:10pt}}.next{{break-before:page}}"
            ),
            "<div class=grid><div><p>FirstA</p><p class=next>SecondA alpha beta gamma delta epsilon zeta eta theta</p></div><div><p>FirstB</p><p class=next>SecondB</p></div></div>",
        );
        assert_eq!(trace.pages.len(), 2, "{first_width} to {later_width}");
        let first_b = trace.pages[0]
            .iter()
            .find(|line| line.text == "FirstB")
            .unwrap();
        let second_b = trace.pages[1]
            .iter()
            .find(|line| line.text == "SecondB")
            .unwrap();
        assert_pt(first_b.x, 20.0 + (first_width as f32 - 30.0) / 2.0);
        assert_pt(second_b.x, 20.0 + (later_width as f32 - 30.0) / 2.0);
        let expected = render(
            &format!(
                "@page{{size:{later_width}pt 180pt;margin:10pt}}.grid{{grid-template-columns:1fr 1fr;gap:10pt}}"
            ),
            "<div class=grid><div><p>SecondA alpha beta gamma delta epsilon zeta eta theta</p></div><div><p>SecondB</p></div></div>",
        );
        let actual: Vec<_> = trace.pages[1]
            .iter()
            .map(|line| (line.text.as_str(), line.x, line.width))
            .collect();
        let expected: Vec<_> = expected.pages[0]
            .iter()
            .map(|line| (line.text.as_str(), line.x, line.width))
            .collect();
        assert_eq!(
            actual, expected,
            "continued text must wrap like the same Grid on the destination page"
        );
    }
}

#[test]
fn later_grid_rows_and_table_items_resolve_named_page_geometry() {
    let trace = render(
        "body{page:flow}@page flow{size:420pt 180pt;margin:10pt}@page flow:first{size:220pt 180pt}.grid{grid-template-columns:1fr 1fr;grid-template-rows:20pt auto;gap:10pt}.later{grid-row:2;break-before:page}table{background:red;border-spacing:0}td{padding:0}",
        "<div class=grid><div>FirstA</div><div>FirstB</div><table class=later><tr><td>Table text</td></tr></table><div class=later>LaterB</div></div>",
    );
    assert_eq!(trace.pages.len(), 2);
    let table = painted_box(&trace, [255, 0, 0]);
    assert_pt(table.x, 10.0);
    assert_pt(table.width, 195.0);
    let later = trace.pages[1]
        .iter()
        .find(|line| line.text == "LaterB")
        .unwrap();
    assert_pt(later.x, 215.0);
}

#[test]
fn automatic_positioned_grid_edges_include_overflowing_tracks_and_padding() {
    for (direction, width, x) in [("ltr", 100, 134.0), ("rtl", 100, -74.0)] {
        let trace = render(
            &format!(
                ".grid{{position:relative;direction:{direction};width:{width}pt;height:40pt;padding:8pt;overflow:hidden;grid-template-columns:80pt 120pt;grid-template-rows:30pt 40pt;gap:10pt}}.placed{{position:absolute;grid-column:2/auto;grid-row:2/auto;inset:0;background:red}}"
            ),
            "<div class=grid><div></div><div></div><div class=placed></div></div>",
        );
        let rect = painted_box(&trace, [255, 0, 0]);
        assert_pt(rect.x, x);
        assert_pt(rect.y, 84.0);
        assert_pt(rect.width, 128.0);
        assert_pt(rect.height, 48.0);
    }
}

#[test]
fn grid_rows_continue_through_columns_and_pages_without_losing_items() {
    let body = format!(
        "<section class=columns><div class=grid>{}</div></section>",
        (0..12)
            .map(|index| format!("<div>Item{index:02}</div>"))
            .collect::<String>()
    );
    let trace = render(
        "@page{size:420pt 100pt;margin:10pt}.columns{column-count:2;column-gap:20pt;column-fill:auto}.grid{grid-template-columns:1fr 1fr;grid-auto-rows:40pt;column-gap:10pt}",
        &body,
    );
    assert_eq!(
        trace.pages.len(),
        2,
        "six Grid rows must occupy three columns across two pages"
    );
    let first_baseline = trace.pages[0]
        .iter()
        .find(|line| line.text == "Item00")
        .unwrap()
        .baseline;
    for index in 0..12 {
        let label = format!("Item{index:02}");
        let matches: Vec<_> = trace
            .pages
            .iter()
            .enumerate()
            .flat_map(|(page, lines)| {
                lines
                    .iter()
                    .filter(|line| line.text == label)
                    .map(move |line| (page, line))
            })
            .collect();
        assert_eq!(matches.len(), 1, "{label} must occur exactly once");
        let (page, line) = matches[0];
        assert_eq!(page, index / 8, "{label} on the wrong page");
        assert_pt(
            line.x,
            10.0 + ((index / 4) % 2) as f32 * 210.0 + (index % 2) as f32 * 100.0,
        );
        assert_pt(
            line.baseline,
            first_baseline + ((index / 2) % 2) as f32 * 40.0,
        );
    }
}

#[test]
fn automatic_repeats_collapse_only_unoccupied_auto_fit_tracks() {
    for (repeat, width) in [("auto-fill", 80.0), ("auto-fit", 125.0)] {
        let trace = render(
            &format!(
                ".grid{{width:260pt;grid-template-columns:repeat({repeat},minmax(60pt,1fr));gap:10pt}}.a{{background:red;height:20pt}}.b{{background:blue;height:20pt}}"
            ),
            "<div class=grid><div class=a>A</div><div class=b>B</div></div>",
        );
        let first = painted_box(&trace, [255, 0, 0]);
        let second = painted_box(&trace, [0, 0, 255]);
        assert_pt(first.width, width);
        assert_pt(second.width, width);
        assert_pt(second.x - first.x, width + 10.0);
    }
}

#[test]
fn indefinite_percentage_rows_resolve_after_container_intrinsic_height() {
    for (tracks, gap, second_offset) in [
        ("50% 50%", "0", 30.0),
        ("75% 75%", "0", 45.0),
        ("auto auto", "10%", 26.0),
    ] {
        let trace = render(
            &format!(
                ".grid{{width:100pt;grid-template-rows:{tracks};row-gap:{gap};background:lime}}.a{{height:20pt;background:red}}.b{{height:40pt;background:blue}}"
            ),
            "<div class=grid><div class=a>A</div><div class=b>B</div></div><p>After</p>",
        );
        let grid = painted_box(&trace, [0, 255, 0]);
        let first = painted_box(&trace, [255, 0, 0]);
        let second = painted_box(&trace, [0, 0, 255]);
        assert_pt(grid.height, 60.0);
        assert_pt(first.y, grid.y);
        assert_pt(second.y, grid.y + second_offset);
        assert_pt(second.height, 40.0);
    }
}

#[test]
fn anonymous_generated_and_contents_items_share_grid_placement() {
    let trace = render(
        ".grid{grid-template-columns:repeat(5,60pt);grid-auto-rows:30pt}.grid::before{content:'Before'}.grid::after{content:'After'}.contents{display:contents}",
        "<div class=grid>Raw<span class=contents><span>First</span> \n <span>Second</span></span> \n </div>",
    );
    assert_eq!(trace.pages.len(), 1);
    let lines = &trace.pages[0];
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Before", "Raw", "First", "Second", "After"]
    );
    for (index, line) in lines.iter().enumerate() {
        assert_pt(line.x, 36.0 + index as f32 * 60.0);
        assert_pt(line.baseline, lines[0].baseline);
    }
}

#[test]
fn nested_grid_flex_and_table_items_keep_their_assigned_area_widths() {
    let trace = render(
        ".grid{width:320pt;grid-template-columns:100pt 100pt 100pt;gap:10pt}.nested{display:grid;grid-template-columns:1fr 1fr;gap:10pt;background:red}.flex{display:flex;background:lime}.flex>div{flex:1;min-width:0}table{width:100%;border-spacing:0;background:blue}td{padding:0;width:50%}",
        "<div class=grid><div class=nested><div>N1</div><div>N2</div></div><div class=flex><div>F1</div><div>F2</div></div><table><tr><td>T1</td><td>T2</td></tr></table></div>",
    );
    for (color, x) in [
        ([255, 0, 0], 36.0),
        ([0, 255, 0], 146.0),
        ([0, 0, 255], 256.0),
    ] {
        let rect = painted_box(&trace, color);
        assert_pt(rect.x, x);
        assert_pt(rect.width, 100.0);
    }
    for (label, x) in [
        ("N1", 36.0),
        ("N2", 91.0),
        ("F1", 146.0),
        ("F2", 196.0),
        ("T1", 256.0),
        ("T2", 306.0),
    ] {
        let line = trace.pages[0]
            .iter()
            .find(|line| line.text == label)
            .unwrap_or_else(|| panic!("missing nested item {label}"));
        assert_pt(line.x, x);
    }
}

#[test]
fn grid_order_changes_placement_without_changing_logical_trace_order() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:repeat(4,50pt)}.first{order:2}.second{order:-1}.empty{order:1}.last{order:3}",
        "<div class=grid><div class=first>First</div><div class=empty></div><div class=second>Second</div><div class=last>Last</div></div>",
    );
    let lines = &trace.pages[0];
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["First", "Second", "Last"]
    );
    for (line, x) in lines.iter().zip([136.0, 36.0, 186.0]) {
        assert_pt(line.x, x);
    }
}

#[test]
fn replaced_grid_items_resolve_area_height_before_preserving_aspect_ratio() {
    for (sizing, width, height) in [
        ("", 60.0, 30.0),
        ("height:40pt", 80.0, 40.0),
        ("height:50%", 80.0, 40.0),
        ("max-width:50%", 50.0, 25.0),
        ("justify-self:stretch;align-self:stretch", 100.0, 80.0),
    ] {
        let trace = render(
            &format!(
                ".grid{{width:100pt;grid-template-columns:100pt;grid-template-rows:80pt;align-items:normal;justify-items:normal}}img{{background:red;{sizing}}}"
            ),
            "<div class=grid><img src=\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='80' height='40'><rect width='80' height='40' fill='blue'/></svg>\"></div>",
        );
        let image = painted_box(&trace, [255, 0, 0]);
        assert!(
            (image.width - width).abs() < 0.001,
            "{sizing}: expected width {width}, got {}",
            image.width
        );
        assert_pt(image.height, height);
    }
}

#[test]
fn grid_image_automatic_minimum_respects_zero_override_and_flexible_spans() {
    for (minimum, peer_offset) in [("", 70.0), ("min-width:0", 30.0)] {
        let trace = render(
            &format!(
                ".grid{{width:50pt;grid-template-columns:1fr 20pt;gap:10pt}}img{{{minimum}}}.peer{{background:red}}"
            ),
            "<div class=grid><img src=\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='80' height='40'></svg>\"><div class=peer>Peer</div></div>",
        );
        assert_pt(painted_box(&trace, [255, 0, 0]).x, 36.0 + peer_offset);
    }
    let trace = render(
        ".grid{width:50pt;grid-template-columns:1fr 1fr;gap:10pt}img{grid-column:span 2}.peer{background:red;min-width:0;grid-column:2;grid-row:2}",
        "<div class=grid><img src=\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='80' height='40'></svg>\"><div class=peer>Peer</div></div>",
    );
    assert_pt(painted_box(&trace, [255, 0, 0]).x, 66.0);
}

#[test]
fn spanning_grid_continuations_share_forced_row_boundaries_without_moving_later_rows() {
    let trace = render(
        "@page{size:240pt 160pt;margin:10pt}.grid{grid-template-columns:100pt 100pt;grid-template-rows:40pt 40pt 40pt;row-gap:10pt}.span{grid-column:1;grid-row:1/4}.first{grid-area:1/2}.second{grid-area:2/2;break-before:page}.third{grid-area:3/2}.next{break-before:page}",
        "<div class=grid><div class=span><p>SpanA</p><p class=next>SpanB</p><p class=next>SpanC</p></div><div class=first>PeerA</div><div class=second>PeerB</div><div class=third>PeerC</div></div><p>After</p>",
    );
    assert_eq!(trace.pages.len(), 3);
    for (label, expected_page, offset) in [
        ("SpanA", 0, 0.0),
        ("PeerA", 0, 0.0),
        ("SpanB", 1, 0.0),
        ("PeerB", 1, 0.0),
        ("PeerC", 1, 50.0),
        ("SpanC", 2, 0.0),
        ("After", 1, 90.0),
    ] {
        let found: Vec<_> = trace
            .pages
            .iter()
            .enumerate()
            .flat_map(|(page, lines)| {
                lines
                    .iter()
                    .filter(move |line| line.text == label)
                    .map(move |line| (page, line))
            })
            .collect();
        assert_eq!(found.len(), 1, "{label} must be conserved");
        let (page, line) = found[0];
        assert_eq!(
            page, expected_page,
            "{label} must follow its own track or continuation"
        );
        assert_pt(line.baseline, trace.pages[0][0].baseline + offset);
    }
}

#[test]
fn implicit_grid_tracks_cycle_backwards_before_explicit_lines() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:40pt 50pt;grid-auto-columns:20pt 30pt;column-gap:10pt;grid-auto-rows:20pt}.grid>div{grid-row:1}.before{grid-column:-5/-4;background:red}.explicit{grid-column:1/2;background:blue}.after{grid-column:3/4;background:lime}",
        "<div class=grid><div class=before>Before</div><div class=explicit>Explicit</div><div class=after>After</div></div>",
    );
    for (color, x, width) in [
        ([255, 0, 0], 36.0, 20.0),
        ([0, 0, 255], 106.0, 40.0),
        ([0, 255, 0], 216.0, 20.0),
    ] {
        let rect = painted_box(&trace, color);
        assert_pt(rect.x, x);
        assert_pt(rect.width, width);
        assert_pt(rect.y, 36.0);
    }
}

#[test]
fn nested_inline_grid_intrinsic_width_includes_tracks_gaps_and_edges() {
    let trace = render(
        ".outer{display:inline-grid;background:red;padding:5pt;border:2pt solid black}.inner{display:grid;grid-template-columns:max-content max-content;gap:10pt;background:blue}.first{width:40pt;height:20pt}.second{width:60pt;height:20pt}",
        "<div class=outer><div class=inner><div class=first>A</div><div class=second>B</div></div></div>",
    );
    let outer = painted_box(&trace, [255, 0, 0]);
    let inner = painted_box(&trace, [0, 0, 255]);
    assert_pt(outer.width, 124.0);
    assert_pt(inner.width, 110.0);
    assert_pt(inner.x - outer.x, 7.0);
    assert_pt(inner.height, 20.0);
}

#[test]
fn inline_grid_percentage_columns_resolve_against_intrinsic_width() {
    for (tracks, gap, offset) in [
        ("50% 50%", "0", 50.0),
        ("75% 75%", "0", 75.0),
        ("auto auto", "10%", 50.0),
    ] {
        let trace = render(
            &format!(
                ".grid{{display:inline-grid;grid-template-columns:{tracks};column-gap:{gap};background:lime}}.first{{width:40pt;height:20pt}}.second{{width:60pt;height:20pt;background:red}}"
            ),
            "<div class=grid><div class=first>A</div><div class=second>B</div></div>",
        );
        let grid = painted_box(&trace, [0, 255, 0]);
        let second = painted_box(&trace, [255, 0, 0]);
        assert_pt(grid.width, 100.0);
        assert_pt(second.x - grid.x, offset);
        assert_pt(second.width, 60.0);
    }
}

#[test]
fn repeated_named_grid_lines_resolve_occurrences_and_named_spans() {
    let trace = render(
        ".grid{width:210pt;grid-template-columns:[edge] 40pt [edge] 60pt [edge] 90pt [edge];gap:10pt;grid-auto-rows:20pt}.single{grid-column:edge 2/edge 3;background:red}.span{grid-column:edge/span 2 edge;grid-row:2;background:blue}",
        "<div class=grid><div class=single>Single</div><div class=span>Span</div></div>",
    );
    let single = painted_box(&trace, [255, 0, 0]);
    let span = painted_box(&trace, [0, 0, 255]);
    assert_pt(single.x, 86.0);
    assert_pt(single.width, 60.0);
    assert_pt(span.x, 36.0);
    assert_pt(span.width, 110.0);
    assert_pt(span.y - single.y, 30.0);
}

#[test]
fn grid_last_baseline_is_exposed_through_an_inline_block() {
    let trace = render(
        ".wrapper{display:inline-block}.grid{grid-template-columns:80pt;grid-template-rows:40pt 60pt}.late{grid-row:2}.early{grid-row:1}",
        "<p>Anchor <span class=wrapper><span class=grid><span class=late>Late<br>Last</span><span class=early>Early</span></span></span></p>",
    );
    let line = |label: &str| {
        trace.pages[0]
            .iter()
            .find(|line| line.text.starts_with(label))
            .unwrap()
    };
    assert_pt(line("Anchor").baseline, line("Last").baseline);
    assert_pt(line("Late").baseline - line("Early").baseline, 40.0);
    assert!(line("Last").baseline > line("Late").baseline);
}

#[test]
fn grid_running_strings_retain_neutral_events_between_fragmented_items() {
    for display in ["none", "contents"] {
        let trace = render(
            &format!(
                "@page{{size:240pt 160pt;margin:30pt;@top-left{{content:'H[' string(heading,last) ']'}}}}.grid{{grid-template-columns:100pt}}.long{{string-set:heading 'First'}}.next,.last{{break-before:page}}.neutral{{display:{display};string-set:heading 'Middle'}}.last{{string-set:heading 'Last'}}"
            ),
            "<div class=grid><div class=long><p>First</p><p class=next>Continued</p></div><div class=neutral></div><div class=last>Last</div></div>",
        );
        let headers: Vec<_> = trace
            .pages
            .iter()
            .map(|page| {
                page.iter()
                    .find(|line| line.text.starts_with("H["))
                    .expect("running header")
                    .text
                    .as_str()
            })
            .collect();
        assert_eq!(
            headers,
            ["H[First]", "H[Middle]", "H[Last]"],
            "neutral display:{display}"
        );
    }
}

#[test]
fn reordered_grid_running_strings_use_source_assignment_order() {
    let trace = render(
        "@page{margin:30pt;@top-left{content:'F[' string(heading,first) ']L[' string(heading,last) ']'}}.grid{grid-template-columns:100pt 100pt}.first{order:2;string-set:heading 'Alpha'}.second{order:1;string-set:heading 'Beta'}",
        "<div class=grid><div class=first>First</div><div class=second>Second</div></div>",
    );
    let header = trace.pages[0]
        .iter()
        .find(|line| line.text.starts_with("F["))
        .unwrap();
    assert_eq!(header.text, "F[Alpha]L[Beta]");
}

#[test]
fn fragmented_grid_absolute_areas_keep_track_coordinates_on_the_anchor_page() {
    let trace = render(
        "@page{size:240pt 140pt;margin:10pt}.grid{position:relative;grid-template-columns:80pt 120pt;grid-template-rows:30pt 40pt;gap:10pt}.later{grid-area:2/1;break-before:page}.direct,.nested{position:absolute;grid-column:2/3;grid-row:2/3;inset:0}.direct{background:red}.nested{background:blue}",
        "<div class=grid><div>First</div><div class=later>Later<div class=nested>Nested</div></div><div class=direct>Direct</div></div>",
    );
    assert_eq!(
        trace.pages.len(),
        2,
        "absolute Grid content must not create pages"
    );
    for (label, color, page) in [("Direct", [255, 0, 0], 0), ("Nested", [0, 0, 255], 1)] {
        let rect = painted_box(&trace, color);
        assert_pt(rect.x, 100.0);
        assert_pt(rect.y, 50.0);
        assert_pt(rect.width, 120.0);
        assert_pt(rect.height, 40.0);
        assert!(
            trace.pages[page].iter().any(|line| line.text == label),
            "{label} must paint on its anchor page"
        );
    }
}

#[test]
fn absolute_grid_auto_edges_include_descendant_overflow_but_respect_clipping() {
    for (item, descendant, edge) in [
        ("width:160pt", "", 160.0),
        ("", "width:160pt", 160.0),
        ("overflow:hidden", "width:160pt", 100.0),
        ("overflow:hidden", "width:160pt;margin-left:200pt", 100.0),
        (
            "overflow:hidden",
            "width:160pt;position:relative;left:200pt",
            100.0,
        ),
        ("", "width:160pt;transform:translateX(30pt)", 190.0),
        ("width:160pt;margin-right:20pt", "", 180.0),
    ] {
        let trace = render(
            &format!(
                ".grid{{position:relative;width:100pt;grid-template-columns:100pt;grid-template-rows:40pt}}.item{{{item}}}.desc{{height:20pt;{descendant}}}.abs{{position:absolute;right:0;top:0;width:5pt;height:5pt;background:red}}"
            ),
            "<div class=grid><div class=item><div class=desc></div></div><div class=abs></div></div>",
        );
        let rect = painted_box(&trace, [255, 0, 0]);
        assert!(
            (rect.x - (36.0 + edge - 5.0)).abs() < 0.001,
            "item={item}, descendant={descendant}: expected edge {edge}, got {}",
            rect.x + 5.0 - 36.0
        );
    }
}

#[test]
fn grid_auto_edges_ignore_wholly_unreachable_descendants_and_include_rtl_end_overflow() {
    for (left, expected_bottom) in [(-200, 71.0), (-160, 71.0), (-159, 151.0)] {
        let trace = render(
            &format!(
                ".grid{{position:relative;width:100pt;grid-template-columns:100pt;grid-template-rows:40pt}}.desc{{position:relative;left:{left}pt;top:100pt;width:160pt;height:20pt}}.abs{{position:absolute;right:0;bottom:0;width:5pt;height:5pt;background:red}}"
            ),
            "<div class=grid><div><div class=desc></div></div><div class=abs></div></div>",
        );
        assert_pt(painted_box(&trace, [255, 0, 0]).y, expected_bottom);
    }
    let rtl = render(
        ".grid{position:relative;direction:rtl;margin-left:100pt;width:100pt;grid-template-columns:100pt;grid-template-rows:40pt}.item{width:160pt;margin-left:20pt}.abs{position:absolute;left:0;top:0;width:5pt;height:5pt;background:red}",
        "<div class=grid><div class=item></div><div class=abs></div></div>",
    );
    assert_pt(painted_box(&rtl, [255, 0, 0]).x, 56.0);
}

#[test]
fn grid_auto_edges_include_overflow_from_nested_columns() {
    for (prefix, clipping, edge) in [
        ("", "", 191.0),
        ("<div style=height:40pt></div>", "", 241.0),
        ("<div style=height:40pt></div>", "overflow:hidden;", 131.0),
    ] {
        let trace = render(
            &format!(
                ".grid{{position:relative;width:100pt;grid-template-columns:100pt;grid-template-rows:40pt}}.columns{{{clipping}columns:2;column-gap:0;height:40pt;column-fill:auto}}.wide{{width:160pt;height:20pt}}.abs{{position:absolute;right:0;top:0;width:5pt;height:5pt;background:red}}"
            ),
            &format!(
                "<div class=grid><div class=columns>{prefix}<div class=wide></div></div><div class=abs></div></div>"
            ),
        );
        assert_pt(painted_box(&trace, [255, 0, 0]).x, edge);
    }
}

#[test]
fn grid_overflow_from_positioned_descendants_respects_their_owner_clip() {
    for (clip, expected) in [("visible", 191.0), ("hidden", 131.0)] {
        let trace = render(
            &format!(
                ".grid{{position:relative;width:100pt;grid-template-columns:100pt;grid-template-rows:40pt}}.owner{{position:relative;overflow:{clip}}}.wide{{position:absolute;left:0;top:0;width:160pt;height:20pt}}.marker{{position:absolute;right:0;top:0;width:5pt;height:5pt;background:red}}"
            ),
            "<div class=grid><div class=owner><div class=wide></div></div><div class=marker></div></div>",
        );
        assert_pt(painted_box(&trace, [255, 0, 0]).x, expected);
    }
}

#[test]
fn transformed_table_parts_contribute_to_grid_scrollable_bounds() {
    for part in ["table", "tbody", "tr", "td"] {
        eprintln!("checking transformed {part} overflow");
        let trace = render(
            &format!(
                ".grid{{position:relative;width:100pt;grid-template-columns:100pt;grid-template-rows:40pt}}table{{width:160pt;border-spacing:0}}td{{padding:0;height:20pt}}{part}{{transform:translateX(30pt)}}.marker{{position:absolute;right:0;top:0;width:5pt;height:5pt;background:red}}"
            ),
            "<div class=grid><table><tbody><tr><td></td></tr></tbody></table><div class=marker></div></div>",
        );
        assert_pt(painted_box(&trace, [255, 0, 0]).x, 221.0);
    }
}

#[test]
fn invalid_subgrid_auto_fill_repetition_preserves_the_earlier_track_declaration() {
    let trace = render(
        ".grid{width:100pt;grid-template-columns:40pt;grid-template-columns:subgrid repeat(auto-fill,[a]) repeat(auto-fill,[b]);grid-template-rows:20pt}.item{background:red}",
        "<div class=grid><div class=item></div></div>",
    );
    assert_pt(painted_box(&trace, [255, 0, 0]).width, 40.0);
}

#[test]
fn percentage_height_images_share_their_final_baseline_in_fixed_rows() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:100pt 100pt;grid-template-rows:60pt;align-items:baseline}img{width:80pt;height:100%;align-self:baseline;background:red}.peer{font-size:30pt;line-height:36pt}",
        "<div class=grid><img src='../assets/image-pattern.png'><div class=peer>Peer</div></div>",
    );
    let image = painted_box(&trace, [255, 0, 0]);
    assert_pt(image.y, 36.0);
    assert_pt(image.height, 60.0);
    let peer = trace.pages[0]
        .iter()
        .find(|line| line.text == "Peer")
        .unwrap();
    assert_pt(peer.baseline, 96.0);
}
