mod common;

use common::{assert_pt, local_options};
use ferri_pdf::{RenderOutput, render_html};

const PROFILE: &str = "@font-face{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}@page{size:240pt 200pt;margin:20pt}html,body,section,div,p,h1{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}h1{break-after:auto}";

#[test]
fn leading_side_breaks_suppress_empty_sheets_and_alternate_from_the_first_printed_page() {
    use ferri_pdf_style::PageSide;
    for (direction, before, first_side) in [
        ("ltr", "left", PageSide::Left),
        ("ltr", "right", PageSide::Right),
        ("ltr", "page", PageSide::Left),
        ("rtl", "auto", PageSide::Left),
        ("rtl", "recto", PageSide::Left),
        ("rtl", "verso", PageSide::Right),
    ] {
        let output = render(
            &format!(
                "@page:left{{size:300pt 200pt}}@page:right{{size:500pt 200pt}}html{{direction:{direction}}}p:first-child{{break-before:{before}}}p+p{{break-before:page}}"
            ),
            "<p>First</p><p>Second</p>",
        );
        let trace = output.layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            2,
            "{direction}/{before} inserted a leading empty sheet"
        );
        assert_eq!(trace.pages[0][0].text, "First");
        assert_eq!(trace.pages[1][0].text, "Second");
        assert_eq!(trace.page_records[0].request.side, first_side);
        assert_ne!(trace.page_records[1].request.side, first_side);
        for (index, record) in trace.page_records.iter().enumerate() {
            assert_eq!(record.request.first, index == 0);
            assert!(!record.request.blank);
            assert_eq!(
                record.geometry.paper[0],
                if record.request.side == PageSide::Left {
                    300.0
                } else {
                    500.0
                }
            );
        }
    }
}

#[test]
fn body_direction_controls_page_progression_and_recto() {
    use ferri_pdf_style::PageSide;
    let output = render(
        "html{direction:ltr}body{direction:rtl}@page:left{size:300pt 200pt}@page:right{size:500pt 200pt}p+p{break-before:recto}",
        "<p>First</p><p>Second</p>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.len(),
        3,
        "RTL recto advances to the next left page"
    );
    assert_eq!(
        trace
            .page_records
            .iter()
            .map(|page| page.request.side)
            .collect::<Vec<_>>(),
        [PageSide::Left, PageSide::Right, PageSide::Left]
    );
    assert!(trace.page_records[1].request.blank);
    assert_eq!(trace.pages[0][0].text, "First");
    assert_eq!(trace.pages[2][0].text, "Second");
}

#[test]
fn forced_page_sides_insert_blank_pages_with_their_own_selected_geometry() {
    for (value, pages) in [("left", 2), ("right", 3), ("recto", 3), ("verso", 2)] {
        let output = render(
            &format!("@page:blank{{size:320pt 180pt}}p+p{{break-before:{value}}}"),
            "<p>Before</p><p>After</p>",
        );
        let trace = output.layout.unwrap();
        assert_eq!(trace.pages.len(), pages, "break-before:{value}");
        assert_eq!(trace.pages[0][0].text, "Before");
        assert_eq!(trace.pages[pages - 1][0].text, "After");
        if pages == 3 {
            assert!(
                trace.pages[1].is_empty(),
                "inserted page has no body content"
            );
            assert_eq!(trace.page_records[1].geometry.paper, [320.0, 180.0]);
        }
        assert_eq!(trace.page_records[pages - 1].geometry.paper, [240.0, 200.0]);
    }
}

#[test]
fn named_side_breaks_select_pending_type_for_blanks_and_restore_parent_scope() {
    use ferri_pdf_style::PageSide;
    for (direction, side, first, blank) in [
        ("ltr", "right", PageSide::Right, PageSide::Left),
        ("rtl", "left", PageSide::Left, PageSide::Right),
    ] {
        let output = render(
            &format!(
                "body{{page:Parent;direction:{direction}}}@page Parent{{size:360pt 200pt}}@page A{{size:240pt 200pt}}@page B{{size:300pt 200pt}}@page B:blank{{size:320pt 180pt;@top-center{{content:'Blank'}}}}.before{{page:A}}.after{{page:B;break-before:{side}}}"
            ),
            "<section><p class=before>Before</p><p class=after>After</p>Return</section>",
        );
        let trace = output.layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            4,
            "{direction}: combined name/side break"
        );
        for (index, (name, width, is_blank, expected_side, text)) in [
            ("A", 240.0, false, first, "Before"),
            ("B", 320.0, true, blank, "Blank"),
            ("B", 300.0, false, first, "After"),
            ("Parent", 360.0, false, blank, "Return"),
        ]
        .into_iter()
        .enumerate()
        {
            let record = &trace.page_records[index];
            assert_eq!(
                record.request.name.as_deref(),
                Some(name),
                "{direction}, page {index}"
            );
            assert_eq!(record.request.side, expected_side);
            assert_eq!(record.request.blank, is_blank);
            assert_eq!(record.request.first, index == 0);
            assert_pt(record.geometry.paper[0], width);
            assert_eq!(
                trace.pages[index]
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>(),
                [text]
            );
        }
    }
}

#[test]
fn table_row_names_change_physical_pages_and_remeasure_the_grid() {
    for body in [
        "<table><tr class=a><td>First</td><td>RightA</td></tr><tr class=b><td>Second</td><td>RightB</td></tr><tr class=a><td>Third</td><td>RightC</td></tr></table>",
        "<table><tbody class=a><tr><td>First</td><td>RightA</td></tr></tbody><tbody class=b><tr><td>Second</td><td>RightB</td></tr></tbody><tbody class=a><tr><td>Third</td><td>RightC</td></tr></tbody></table>",
    ] {
        let output = render(
            "@page A{size:240pt 200pt}@page B{size:400pt 200pt}.a{page:A}.b{page:B}table{width:100%;table-layout:fixed;border-spacing:0;break-inside:avoid}td{padding:0}",
            body,
        );
        let trace = output.layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            3,
            "row boundaries must force named pages: {body}"
        );
        for (index, (name, width, text)) in [
            ("A", 240.0, ["First", "RightA"]),
            ("B", 400.0, ["Second", "RightB"]),
            ("A", 240.0, ["Third", "RightC"]),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                trace.page_records[index].request.name.as_deref(),
                Some(name)
            );
            assert_pt(trace.page_records[index].geometry.paper[0], width);
            assert_eq!(
                trace.pages[index]
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>(),
                text
            );
            assert_pt(trace.pages[index][1].x, width / 2.0);
        }
    }
}

#[test]
fn table_row_name_and_side_break_share_one_transition_and_ignore_cell_names() {
    let output = render(
        "body{page:Parent}@page Parent{size:360pt 200pt}@page A{size:240pt 200pt}@page B{size:400pt 200pt}@page B:blank{size:320pt 180pt}.a{page:A}.b{page:B;break-before:right}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;page:Ignored}td p{page:AlsoIgnored}",
        "<table><tr class=a><td><p>First</p></td></tr><tr class=b><td><p>Second</p></td></tr></table><p>Return</p>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 4);
    for (index, (name, width, blank, text)) in [
        ("A", 240.0, false, vec!["First"]),
        ("B", 320.0, true, vec![]),
        ("B", 400.0, false, vec!["Second"]),
        ("Parent", 360.0, false, vec!["Return"]),
    ]
    .into_iter()
    .enumerate()
    {
        let record = &trace.page_records[index];
        assert_eq!(record.request.name.as_deref(), Some(name), "page {index}");
        assert_eq!(record.request.blank, blank);
        assert_pt(record.geometry.paper[0], width);
        assert_eq!(
            trace.pages[index]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            text
        );
    }
}

#[test]
fn named_table_breaks_limit_keep_lookahead_to_the_first_page_fragment() {
    let output = render(
        "body{page:A}@page{size:240pt 80pt;margin:10pt}p{height:40pt}table{width:100%;border-spacing:0;break-inside:avoid}td{padding:0}tr+tr{page:B}",
        "<p>Before</p><table><tr><td>First</td></tr><tr><td>Second</td></tr><tr><td>Third</td></tr></table>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.len(),
        2,
        "a forced name change must override the table keep"
    );
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Before", "First"]
    );
    assert_eq!(
        trace.pages[1]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Second", "Third"]
    );
    assert_eq!(trace.page_records[1].request.name.as_deref(), Some("B"));
}

#[test]
fn nested_table_names_do_not_escape_a_non_applicable_cell_subtree() {
    let output = render(
        "body{page:A}table{width:100%;border-spacing:0}td{padding:0}.nested tr+tr{page:B}",
        "<table><tr><td><table class=nested><tr><td>First</td></tr><tr><td>Second</td></tr></table></td></tr></table>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.len(),
        1,
        "page does not apply to the containing table cell"
    );
    assert_eq!(trace.page_records[0].request.name.as_deref(), Some("A"));
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["First", "Second"]
    );
}

#[test]
fn row_group_breaks_apply_at_group_edges_and_combine_with_row_constraints() {
    for (css, count) in [
        (".second{break-before:right}", 3),
        (".first{break-after:right}", 3),
        (
            ".second{break-before:right}.second tr:first-child{break-before:left}",
            2,
        ),
        (
            ".first{break-after:right}.first tr:last-child{break-after:left}",
            2,
        ),
        (".first{break-after:right}.second{break-before:left}", 2),
    ] {
        let output = render(
            &format!("table{{width:100%;border-spacing:0}}td{{padding:0}}{css}"),
            "<table><tbody class=first><tr><td>First</td></tr><tr><td>Next</td></tr></tbody><tbody class=second><tr><td>Last</td></tr><tr><td>End</td></tr></tbody></table>",
        );
        let trace = output.layout.unwrap();
        assert_eq!(trace.pages.len(), count, "{css}");
        assert_eq!(
            trace.pages[0]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["First", "Next"],
            "{css}"
        );
        assert_eq!(
            trace.pages[count - 1]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Last", "End"],
            "{css}"
        );
        if count == 3 {
            assert!(trace.page_records[1].request.blank, "{css}");
            assert!(trace.pages[1].is_empty(), "{css}");
        }
    }
}

#[test]
fn leading_group_break_moves_the_table_border_with_its_content() {
    let output = render(
        "table{width:100%;border-spacing:0;border-top:6pt solid red}td{padding:0}tbody{break-before:right}",
        "<p>Before</p><table><tbody><tr><td>First</td></tr></tbody></table>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 3);
    let red_border_count = |page: usize| {
        trace.display_lists[page].iter().filter(|item| {
        matches!(item, ferri_pdf::DisplayItem::Box(paint) if paint.decoration.borders.iter().any(|border| border.color.rgb == [255, 0, 0] && border.width > 0.0))
    }).count()
    };
    assert_eq!(
        red_border_count(0),
        0,
        "the table must not start before its propagated break"
    );
    assert_eq!(
        red_border_count(2),
        1,
        "the table border belongs with its first row"
    );
    assert_eq!(trace.pages[2][0].text, "First");
}

#[test]
fn trailing_row_group_break_wins_over_the_table_edge() {
    let output = render(
        "table{width:100%;border-spacing:0;break-after:right}tbody{break-after:left}td{padding:0}",
        "<table><tbody><tr><td>First</td></tr></tbody></table><p>After</p>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.len(),
        2,
        "the descendant's side constraint is later in flow"
    );
    assert_eq!(trace.pages[1][0].text, "After");
}

#[test]
fn resumed_cell_restores_cloned_percentage_reserves_once_per_page() {
    let lines = (0..14)
        .map(|index| format!("L{index:02}"))
        .collect::<Vec<_>>();
    let body = lines
        .iter()
        .map(|line| format!("<p>{line}</p>"))
        .collect::<String>();
    let output = render(
        "@page{size:240pt 100pt;margin:10pt}@page:left{size:400pt 100pt}section{padding-bottom:10%;box-decoration-break:clone}table{width:100%;border-spacing:0}td{padding:0;vertical-align:top}",
        &format!("<section><table><tr><td>{body}</td></tr></table></section>"),
    );
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.len(),
        3,
        "cloned end padding reserves 22pt, 38pt, then 22pt"
    );
    for (page, expected) in trace
        .pages
        .iter()
        .zip([&lines[..5], &lines[5..9], &lines[9..]])
    {
        assert_eq!(
            page.iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn later_side_break_wins_over_previous_sibling_and_ancestor() {
    let output = render(
        ".before{break-after:right}section{break-before:right}section p{break-before:left}",
        "<p class=before>Before</p><section><p>After</p></section>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 2);
    assert_eq!(trace.pages[1][0].text, "After");
}

#[test]
fn forced_side_blank_page_preserves_canvas_and_margin_content() {
    let output = render(
        "body{background:blue}@page{font:10pt Fixed;@top-center{content:'Ordinary'}}@page:blank{@top-center{content:'Blank'}}p+p{break-before:right}",
        "<p>Before</p><p>After</p>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 3);
    assert!(trace.page_records[1].request.blank);
    assert_eq!(
        trace.pages[1]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Blank"]
    );
    assert!(!trace.display_lists[1].is_empty());
}

#[test]
fn forced_side_break_inside_a_table_preserves_parallel_content() {
    let output = render(
        "table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;vertical-align:top}.after{break-before:right}",
        "<table><tr><td><p>Before</p><p class=after>After</p></td><td>Neighbour</td></tr></table>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 3);
    assert!(trace.pages[1].is_empty());
    assert!(trace.page_records[1].request.blank);
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Before", "Neighbour"]
    );
    assert_eq!(trace.pages[2][0].text, "After");
}

#[test]
fn parallel_content_replaces_blank_page_geometry_and_canvas_independently_of_cell_order() {
    let words = tokens('B', 60);
    let forced = "<td><p>Before</p><p class=after>After</p></td>";
    let flowing = format!("<td>{}</td>", words.join(" "));
    for cells in [format!("{forced}{flowing}"), format!("{flowing}{forced}")] {
        let output = render(
            "@page{size:300pt 80pt;margin:10pt}@page:blank{size:100pt 50pt}body{background:blue}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;vertical-align:top}.after{break-before:right}",
            &format!("<table><tr>{cells}</tr></table>"),
        );
        let trace = output.layout.unwrap();
        assert_eq!(trace.pages.len(), 3);
        assert!(
            !trace.pages[1].is_empty(),
            "the parallel cell continues onto the skipped page"
        );
        assert!(!trace.page_records[1].request.blank);
        assert_eq!(trace.page_records[1].geometry.paper, [300.0, 80.0]);
        let actual = trace
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.text.split_whitespace())
            .filter(|word| word.starts_with('B') && *word != "Before")
            .collect::<Vec<_>>();
        assert_eq!(actual, words, "parallel source tokens must be emitted once");
        let canvas = trace.display_lists[1]
            .iter()
            .find_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .expect("page two retains its body canvas");
        assert_pt(canvas.rect.width, 280.0);
        assert_pt(canvas.rect.height, 60.0);
        assert_pt(canvas.background_clips().border.rect.width, 280.0);
    }
}

#[test]
fn forced_side_break_inside_columns_reaches_the_physical_page() {
    let output = render(
        "section{column-count:2;column-gap:10pt}.after{break-before:right}",
        "<section><p>Before</p><p class=after>After</p></section>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 3);
    assert_eq!(trace.pages[0][0].text, "Before");
    assert!(trace.pages[1].is_empty());
    assert!(trace.page_records[1].request.blank);
    assert_eq!(trace.pages[2][0].text, "After");
}

fn render(css: &str, body: &str) -> RenderOutput {
    render_html(
        &format!(
            "<!doctype html><style>{}{PROFILE}{css}</style>{body}",
            common::PROFILE
        ),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("named-page CSS `{css}` with `{body}`: {error}"))
}

fn tokens(prefix: char, count: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("{prefix}{index:03}"))
        .collect()
}

#[test]
fn named_first_page_is_selected_by_body_and_unused_types_do_not_apply() {
    let css =
        "@page cases_page{size:300pt 400pt;margin:20pt}@page unused{size:900pt 800pt;margin:70pt}";
    let unused = render(css, "<h1 id=plain>Default</h1>");
    let trace = unused.layout.as_ref().unwrap();
    assert_eq!([trace.page_width, trace.page_height], [240.0, 200.0]);
    assert_pt(unused.headings[0].top, 20.0);
    let selected = render(
        &format!("{css}body{{page:cases_page}}"),
        "<h1 id=selected>Cases</h1>",
    );
    let trace = selected.layout.as_ref().unwrap();
    assert_eq!([trace.page_width, trace.page_height], [300.0, 400.0]);
    assert_eq!(selected.page_count, 1);
    assert_eq!(selected.headings[0].page, 1);
    assert_pt(selected.headings[0].top, 20.0);
    assert_pt(trace.pages[0][0].x, 20.0);
}

#[test]
fn changing_page_names_reflows_sections_at_each_selected_content_width() {
    let groups = [tokens('A', 120), tokens('B', 120), tokens('C', 120)];
    let css = "@page A{size:300pt 400pt;margin:20pt}@page B{size:500pt 300pt;margin:10pt}.a{page:A}.b{page:B}";
    let body = groups
        .iter()
        .enumerate()
        .map(|(index, words)| {
            format!(
                "<section class={}><p>{}</p></section>",
                if index == 1 { "b" } else { "a" },
                words.join(" ")
            )
        })
        .collect::<String>();
    let output = render(css, &body);
    assert_eq!(
        output.page_count, 3,
        "name changes supply the breaks without explicit break-before"
    );
    let trace = output.layout.unwrap();
    for (index, count, origin) in [(0, 9, 20.0), (1, 16, 10.0), (2, 8, 20.0)] {
        let lines = &trace.pages[index];
        assert_eq!(
            lines[0].text.split_ascii_whitespace().count(),
            count,
            "page {} must wrap at its own width; DejaVuSans A/B/C=1401/1405/1430, digit=1303, space=651, em=2048 at 10pt",
            index + 1
        );
        assert_pt(lines[0].x, origin);
        let actual = lines
            .iter()
            .flat_map(|line| line.text.split_ascii_whitespace())
            .collect::<Vec<_>>();
        assert_eq!(
            actual, groups[index],
            "every source token must appear once on its selected page"
        );
    }
}

#[test]
fn equal_page_names_share_a_page_and_nested_contents_scope_restores_its_parent() {
    let css = "@page A{size:300pt 400pt;margin:20pt}@page B{size:500pt 300pt;margin:10pt}body{page:A}.same{page:A}.hidden{display:none;page:B}.contents{display:contents;page:B}";
    let shared = render(
        css,
        "<section class=same><p>First</p></section><section class=same><p>Second</p></section>",
    );
    assert_eq!(shared.page_count, 1, "equal types are not forced breaks");
    let nested = render(
        css,
        "<section class=hidden><p>Hidden</p></section><p>Before</p><p><span style='page:B'>Stay A</span></p><div class=contents><section><p>Inside B</p></section></div><p>After</p>",
    );
    assert_eq!(nested.page_count, 3);
    let trace = nested.layout.unwrap();
    let printed = trace
        .pages
        .iter()
        .map(|page| {
            page.iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        printed,
        [vec!["Before", "Stay A"], vec!["Inside B"], vec!["After"]]
    );
    for (page, x) in [(0, 20.0), (1, 10.0), (2, 20.0)] {
        assert_pt(trace.pages[page][0].x, x);
    }
}

#[test]
fn page_selector_names_and_pseudos_use_decoded_identity_and_page_specificity() {
    let css = r"@page cases_page{size:240pt 160pt;margin:20pt}@page Cases_Page{size:999pt 999pt;margin:90pt}@page ca\73 es_page:first{margin-left:10pt}@page cases_page:right{margin-left:40pt}@page cases_page:left{margin-left:30pt}@page cases_page:first:first{margin-left:12pt}@page cases_page:left:right{margin-left:99pt}body{page:cases_page}p+p{break-before:page}";
    let output = render(css, "<p>First</p><p>Second</p><p>Third</p>");
    assert_eq!(output.page_count, 3);
    let trace = output.layout.unwrap();
    assert_eq!([trace.page_width, trace.page_height], [240.0, 160.0]);
    for (page, x) in [(0, 12.0), (1, 30.0), (2, 40.0)] {
        assert_pt(trace.pages[page][0].x, x);
    }
}

#[test]
fn page_candidates_keep_important_layer_order_and_auto_selectors_never_match() {
    let css = "@layer first,second;@layer first{@page chosen{margin-left:10pt!important}}@layer second{@page chosen{margin-left:20pt!important}}@page chosen{size:300pt 400pt;margin-left:40pt!important}@page auto{size:900pt 900pt;margin:90pt}@page AUTO{size:800pt 800pt;margin:80pt}body{page:chosen}";
    let output = render(css, "<p>Selected</p>");
    let trace = output.layout.unwrap();
    assert_eq!([trace.page_width, trace.page_height], [300.0, 400.0]);
    assert_pt(trace.pages[0][0].x, 10.0);
    let unmatched = render(
        "@page auto{size:900pt 900pt}@page AUTO{margin:90pt}",
        "<p>Default</p>",
    );
    let trace = unmatched.layout.unwrap();
    assert_eq!([trace.page_width, trace.page_height], [240.0, 200.0]);
    assert_pt(trace.pages[0][0].x, 20.0);
}

#[test]
fn one_paragraph_resumes_source_tokens_across_alternating_page_widths() {
    let words = tokens('A', 240);
    let output = render(
        "@page flow{size:300pt 80pt;margin:10pt}@page flow:left{size:500pt 80pt}body{page:flow}",
        &format!("<p>{}</p>", words.join(" ")),
    );
    assert_eq!(output.page_count, 4);
    let trace = output.layout.unwrap();
    for (page, line_count, first_line_words) in [(0, 6, 9), (1, 6, 16), (2, 6, 9), (3, 3, 16)] {
        assert_eq!(
            trace.pages[page].len(),
            line_count,
            "page {} line count after real width reflow",
            page + 1
        );
        assert_eq!(
            trace.pages[page][0].text.split_ascii_whitespace().count(),
            first_line_words
        );
        assert_pt(trace.pages[page][0].x, 10.0);
    }
    let printed = trace
        .pages
        .iter()
        .flatten()
        .flat_map(|line| line.text.split_ascii_whitespace())
        .collect::<Vec<_>>();
    assert_eq!(
        printed, words,
        "line ordinals are not valid continuation cursors when width changes"
    );
}

#[test]
fn page_changes_refresh_nested_percentage_edges_and_restored_relative_offsets() {
    let output = render(
        "@page A{size:300pt 400pt;margin:20pt}@page B{size:500pt 300pt;margin:10pt}main{padding-left:10%;position:relative;left:5%}article{position:relative;left:5%}section{padding-left:10%}.a{page:A}.b{page:B}p{position:relative;left:5%}",
        "<main><article><section class=a><p>First</p></section><section class=b><p>Second</p></section></article><p class=b>Following</p></main>",
    );
    assert_eq!(output.page_count, 2);
    let trace = output.layout.unwrap();
    for (text, page, expected_x) in [
        ("First", 0, 104.63),
        ("Second", 1, 166.24),
        ("Following", 1, 103.6),
    ] {
        let line = trace.pages[page]
            .iter()
            .find(|line| line.text == text)
            .unwrap();
        assert_pt(line.x, expected_x);
    }
}

#[test]
fn paragraph_reflow_keeps_late_anchors_out_of_earlier_lookahead_windows() {
    let words = tokens('A', 240);
    let markup = words
        .iter()
        .enumerate()
        .map(|(index, word)| match index {
            10 => format!("<a href='#late'>{word}</a>"),
            220 => format!("<span id=late>{word}</span><span class=anchor>Marker</span>"),
            _ => word.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    let output = render(
        "@page flow{size:300pt 80pt;margin:10pt}@page flow:left{size:500pt 80pt}body{page:flow}.anchor{position:absolute}",
        &format!("<p>{markup}</p>"),
    );
    assert_eq!(output.page_count, 4);
    let trace = output.layout.unwrap();
    let markers = trace
        .pages
        .iter()
        .enumerate()
        .flat_map(|(page, lines)| {
            lines
                .iter()
                .filter(|line| line.text == "Marker")
                .map(move |_| page)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        markers,
        [3],
        "the late positioned anchor must be emitted once on its source line's page"
    );
    assert_eq!(
        trace
            .destinations
            .iter()
            .flatten()
            .map(|destination| destination.page)
            .collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(
        trace.links.iter().map(Vec::len).collect::<Vec<_>>(),
        [1, 0, 0, 0]
    );
    let printed = trace
        .pages
        .iter()
        .flatten()
        .filter(|line| line.text != "Marker")
        .flat_map(|line| line.text.split_ascii_whitespace())
        .collect::<Vec<_>>();
    assert_eq!(
        printed, words,
        "navigation and positioned content must not consume or duplicate paragraph tokens"
    );
}

#[test]
fn widows_count_lines_at_the_destination_page_width() {
    let words = tokens('A', 70);
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}p{widows:2;orphans:2}",
        &format!("<p>{}</p>", words.join(" ")),
    );
    let trace = output.layout.unwrap();
    assert_eq!(output.page_count, 2);
    assert_eq!(
        trace.pages.iter().map(Vec::len).collect::<Vec<_>>(),
        [5, 2],
        "leave enough source text for two lines at the wider destination, not two lines at the old width"
    );
    let actual = trace
        .pages
        .iter()
        .flatten()
        .flat_map(|line| line.text.split_ascii_whitespace())
        .collect::<Vec<_>>();
    assert_eq!(
        actual, words,
        "moving the break must preserve every token once"
    );
}

#[test]
fn narrower_destination_does_not_reserve_widows_at_the_old_width() {
    let words = tokens('A', 110);
    let output = render(
        "@page{size:500pt 80pt;margin:10pt}@page:left{size:300pt 80pt}p{widows:2;orphans:2}",
        &format!("<p>{}</p>", words.join(" ")),
    );
    let trace = output.layout.unwrap();
    assert_eq!(output.page_count, 2);
    assert_eq!(
        trace.pages.iter().map(Vec::len).collect::<Vec<_>>(),
        [6, 2],
        "fourteen remaining tokens form two narrow lines; the wider page can stay full"
    );
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.text.split_ascii_whitespace())
            .collect::<Vec<_>>(),
        words
    );
}

#[test]
fn continued_inline_objects_resolve_percent_height_on_their_physical_page() {
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:300pt 140pt}html,body,p{height:100%}.object{display:inline-block;width:20pt;height:50%;background:red;vertical-align:top}",
        "<p>First<br>Second<br>Third<br>Fourth<br>Fifth<br>Sixth<br><span class=object></span>Last</p>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(output.page_count, 2);
    let objects = trace
        .display_lists
        .iter()
        .enumerate()
        .flat_map(|(page, items)| {
            items.iter().filter_map(move |item| match item {
                ferri_pdf::DisplayItem::Box(paint) if paint.rect.width == 20.0 => {
                    Some((page, paint.rect.height))
                }
                _ => None,
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        objects,
        [(1, 60.0)],
        "the inline object uses half the second page's 120pt content height, even though page widths match"
    );
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        [
            "First",
            "Second",
            "Third",
            "Fourth",
            "Fifth",
            "Sixth",
            "\u{fffc}Last"
        ]
    );
}

#[test]
fn new_table_flex_and_column_containers_use_the_selected_page_width() {
    for (mode, markup, css) in [
        (
            "table",
            "<table class=next><tr><td>Left</td><td>Right</td></tr></table>",
            "table{table-layout:fixed;width:100%;border-spacing:0}td{padding:0}",
        ),
        (
            "flex",
            "<section class=next><div>Left</div><div>Right</div></section>",
            "section{display:flex}section>div{flex:1}",
        ),
        (
            "columns",
            "<section class=next><p>Left</p><p style='break-before:column'>Right</p></section>",
            "section{column-count:2;column-gap:0}",
        ),
    ] {
        let output = render(
            &format!(
                "@page A{{size:300pt 400pt;margin:20pt}}@page B{{size:500pt 300pt;margin:10pt}}body{{page:A}}.next{{page:B}}{css}"
            ),
            &format!("<p>Before</p>{markup}"),
        );
        assert_eq!(output.page_count, 2, "{mode} must select B at its boundary");
        let trace = output.layout.unwrap();
        for (word, expected) in [("Left", 10.0), ("Right", 250.0)] {
            let line = trace.pages[1]
                .iter()
                .find(|line| line.text == word)
                .unwrap_or_else(|| {
                    panic!(
                        "{mode} must preserve {word} on B: {:?}",
                        trace.pages[1]
                            .iter()
                            .map(|line| &line.text)
                            .collect::<Vec<_>>()
                    )
                });
            assert!(
                (line.x - expected).abs() < 0.01,
                "{mode} {word}: x={} expected {expected}",
                line.x
            );
        }
    }
}

#[test]
fn kept_following_block_is_measured_before_deciding_to_break() {
    let preceding = tokens('A', 118);
    let following = tokens('B', 25);
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}.kept{break-inside:avoid}",
        &format!(
            "<p>{}</p><div class=kept>{}</div>",
            preceding.join(" "),
            following.join(" ")
        ),
    );
    assert_eq!(
        output.page_count, 2,
        "the wider second page fits four preceding lines plus two kept lines; old-width sizing must not force a third page"
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.iter().map(Vec::len).collect::<Vec<_>>(), [6, 6]);
    let printed = trace
        .pages
        .iter()
        .flatten()
        .flat_map(|line| line.text.split_ascii_whitespace())
        .collect::<Vec<_>>();
    assert_eq!(
        printed,
        preceding
            .iter()
            .chain(&following)
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
}

#[test]
fn continued_table_rows_and_repeated_groups_use_each_pages_columns() {
    let rows = (0..12)
        .map(|index| format!("<tr><td>L{index:02}</td><td>R{index:02}</td></tr>"))
        .collect::<String>();
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td,th{padding:0;font:10pt/10pt Fixed;text-align:left}",
        &format!(
            "<table><thead><tr><th>HeaderLeft</th><th>HeaderRight</th></tr></thead><tbody>{rows}</tbody><tfoot><tr><td>FooterLeft</td><td>FooterRight</td></tr></tfoot></table>"
        ),
    );
    assert_eq!(
        output.page_count, 3,
        "each page fits one header, four body rows and one footer"
    );
    let trace = output.layout.unwrap();
    let mut body = Vec::new();
    for (page, lines) in trace.pages.iter().enumerate() {
        let right = if page == 1 { 250.0 } else { 150.0 };
        for label in ["HeaderLeft", "HeaderRight", "FooterLeft", "FooterRight"] {
            assert_eq!(
                lines.iter().filter(|line| line.text == label).count(),
                1,
                "{label} must repeat once on page {}",
                page + 1
            );
        }
        for line in lines {
            let on_right = line.text.ends_with("Right") || line.text.starts_with('R');
            let expected_x = if on_right { right } else { 10.0 };
            assert!(
                (line.x - expected_x).abs() < 0.01,
                "page {} {:?}: x={} expected {expected_x}",
                page + 1,
                line.text,
                line.x
            );
            if line.text.starts_with('L') || line.text.starts_with('R') {
                body.push(line.text.clone());
            }
        }
    }
    assert_eq!(
        body,
        (0..12)
            .flat_map(|index| [format!("L{index:02}"), format!("R{index:02}")])
            .collect::<Vec<_>>(),
        "body cells occur once in source order; only groups repeat"
    );
}

#[test]
fn continued_fixed_table_retains_widths_from_its_consumed_first_row() {
    let rows = (0..12)
        .map(|index| {
            format!(
                "<tr><td {}>L{index:02}</td><td>R{index:02}</td></tr>",
                if index == 0 { "style='width:25%'" } else { "" }
            )
        })
        .collect::<String>();
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed}",
        &format!("<table>{rows}</table>"),
    );
    assert_eq!(output.page_count, 2);
    let trace = output.layout.unwrap();
    let printed = trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        printed,
        (0..12)
            .flat_map(|index| [format!("L{index:02}"), format!("R{index:02}")])
            .collect::<Vec<_>>()
    );
    for (page, right) in [(0, 80.0), (1, 130.0)] {
        for line in &trace.pages[page] {
            assert_pt(
                line.x,
                if line.text.starts_with('R') {
                    right
                } else {
                    10.0
                },
            );
        }
    }
}

#[test]
fn table_continuation_geometry_does_not_depend_on_background_ink() {
    let rows = (0..12)
        .map(|index| format!("<tr><td>Row{index:02}</td></tr>"))
        .collect::<String>();
    let css = "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0;padding:1% 0;box-decoration-break:clone}td{padding:0;font:10pt/10pt Fixed}";
    let transparent = render(css, &format!("<table>{rows}</table>"));
    let painted = render(
        &format!("{css}table{{background:red}}"),
        &format!("<table>{rows}</table>"),
    );
    assert_eq!(transparent.page_count, painted.page_count);
    let transparent = transparent.layout.unwrap();
    let painted = painted.layout.unwrap();
    for (page, (clear, colored)) in transparent.pages.iter().zip(&painted.pages).enumerate() {
        assert_eq!(
            clear.len(),
            colored.len(),
            "page {} row count must be independent of paint",
            page + 1
        );
        for (clear, colored) in clear.iter().zip(colored) {
            assert_eq!(clear.text, colored.text);
            assert!(
                (clear.baseline - colored.baseline).abs() < 0.01,
                "page {} {:?}: transparent baseline {} differs from painted {}",
                page + 1,
                clear.text,
                clear.baseline,
                colored.baseline
            );
        }
    }
}

#[test]
fn active_table_cell_resumes_source_text_at_each_page_width() {
    let words = tokens('A', 240);
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}",
        &format!("<table><tr><td>{}</td></tr></table>", words.join(" ")),
    );
    assert_eq!(
        output.page_count, 4,
        "a cell spanning pages must resume at the physical page width"
    );
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.iter().map(Vec::len).collect::<Vec<_>>(),
        [6, 6, 6, 3]
    );
    for (page, count) in [(0, 9), (1, 16), (2, 9), (3, 16)] {
        assert_eq!(
            trace.pages[page][0].text.split_ascii_whitespace().count(),
            count
        );
    }
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.text.split_ascii_whitespace())
            .collect::<Vec<_>>(),
        words,
        "every cell token must be emitted once across its continuation"
    );
}

#[test]
fn fragmented_cell_backgrounds_follow_actual_text_fragments() {
    let words = tokens('A', 240);
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;background:red}",
        &format!("<table><tr><td>{}</td></tr></table>", words.join(" ")),
    );
    assert_eq!(output.page_count, 4);
    let trace = output.layout.unwrap();
    for (page, (width, height)) in [(280.0, 60.0), (480.0, 60.0), (280.0, 60.0), (480.0, 30.0)]
        .into_iter()
        .enumerate()
    {
        let paints = trace.display_lists[page]
            .iter()
            .filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [255, 0, 0]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            paints.len(),
            1,
            "page {} must have one cell background fragment",
            page + 1
        );
        assert_pt(paints[0].rect.x, 10.0);
        assert_pt(paints[0].rect.y, 10.0);
        assert_pt(paints[0].rect.width, width);
        assert_pt(paints[0].rect.height, height);
    }
}

#[test]
fn unequal_table_cells_restore_geometry_of_the_furthest_page() {
    let left = tokens('A', 90);
    let right = tokens('B', 40);
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}.after{margin-left:10%}",
        &format!(
            "<table><tr><td>{}</td><td>{}</td></tr></table><p class=after>After</p>",
            left.join(" "),
            right.join(" ")
        ),
    );
    assert_eq!(output.page_count, 3);
    let trace = output.layout.unwrap();
    let after = trace.pages[2]
        .iter()
        .find(|line| line.text == "After")
        .expect("following flow belongs after the longer cell");
    assert_pt(after.x, 38.0);
    for (prefix, expected) in [('A', left), ('B', right)] {
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .flat_map(|line| line.text.split_ascii_whitespace())
                .filter(|word| word.starts_with(prefix) && *word != "After")
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn short_table_cell_stretch_uses_each_pages_column_geometry() {
    let left = tokens('A', 90);
    let right = tokens('B', 40);
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 100pt;margin:20pt 10pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}td+td{background:blue}",
        &format!(
            "<table><tr><td>{}</td><td>{}</td></tr></table>",
            left.join(" "),
            right.join(" ")
        ),
    );
    assert_eq!(output.page_count, 3);
    let trace = output.layout.unwrap();
    for (page, (x, width, height)) in [
        (150.0, 140.0, 60.0),
        (250.0, 240.0, 60.0),
        (150.0, 140.0, 50.0),
    ]
    .into_iter()
    .enumerate()
    {
        let paints = trace.display_lists[page]
            .iter()
            .filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            paints.len(),
            1,
            "page {} must contain one stretched blue cell; boxes {:?}",
            page + 1,
            trace.display_lists[page]
                .iter()
                .filter_map(|item| match item {
                    ferri_pdf::DisplayItem::Box(paint) =>
                        Some((paint.decoration.background.rgb, paint.rect)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        );
        for (label, actual, expected) in [
            ("y", paints[0].rect.y, if page == 1 { 20.0 } else { 10.0 }),
            ("x", paints[0].rect.x, x),
            ("width", paints[0].rect.width, width),
            ("height", paints[0].rect.height, height),
        ] {
            assert!(
                (actual - expected).abs() < 0.01,
                "page {} blue cell {label}: {actual} expected {expected}",
                page + 1
            );
        }
    }
}

#[test]
fn zero_height_child_preserves_overflow_from_its_parents_padding() {
    let css = "@page{size:300pt 80pt;margin:10pt}.outer{padding-top:80pt}.zero{height:0}";
    let reference = render(css, "<p>After</p>").layout.unwrap();
    let output = render(
        css,
        "<div class=outer><div class=zero></div><p>After</p></div>",
    );
    assert_eq!(output.page_count, 2);
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages[1].len(), 1);
    assert_eq!(trace.pages[1][0].text, "After");
    assert_pt(
        trace.pages[1][0].baseline - reference.pages[0][0].baseline,
        20.0,
    );
}

#[test]
fn table_alignment_carries_remaining_padding_fraction_to_each_page_width() {
    let words = tokens('A', 90);
    for (alignment, wide_height, pages, page, offset) in [
        ("middle", 80, 3, 1, 25.0),
        ("bottom", 80, 3, 2, 40.0),
        ("middle", 60, 4, 1, 25.0),
        ("bottom", 60, 4, 3, 10.0),
    ] {
        for short_first in [false, true] {
            let long = format!("<td>{}</td>", words.join(" "));
            let short = format!("<td style='vertical-align:{alignment}'>Short</td>");
            let cells = if short_first {
                format!("{short}{long}")
            } else {
                format!("{long}{short}")
            };
            let output = render(
                &format!(
                    "@page{{size:300pt 80pt;margin:10pt}}@page:left{{size:500pt {wide_height}pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}}"
                ),
                &format!("<table><tr>{cells}</tr></table>"),
            );
            assert_eq!(
                output.page_count, pages,
                "{alignment}, short first {short_first}"
            );
            let trace = output.layout.unwrap();
            let first = trace.pages[0]
                .iter()
                .find(|line| line.text.starts_with("A000"))
                .unwrap();
            let short = trace.pages[page]
                .iter()
                .find(|line| line.text == "Short")
                .unwrap_or_else(|| panic!("{alignment}: expected Short on page {}", page + 1));
            // At 140/240pt, 90 tokens occupy 23/12 lines. Middle padding is 110/55pt;
            // after consuming 60/110, its remaining fraction becomes 25pt on page two.
            // Bottom padding is 220/110pt: 60pt, 60pt, then 40pt are consumed.
            assert_pt(short.baseline - first.baseline, offset);
            let actual = trace
                .pages
                .iter()
                .flatten()
                .flat_map(|line| line.text.split_ascii_whitespace())
                .filter(|word| word.starts_with("A0"))
                .collect::<Vec<_>>();
            assert_eq!(actual, words);
        }
    }
}

#[test]
fn aligned_multiline_cell_reflows_after_its_padding_ends() {
    let long_words = tokens('A', 90);
    let short_words = tokens('B', 40);
    for (alignment, offset, counts) in [
        ("middle", 35.0 / 13.0, [0, 40, 0]),
        ("bottom", 490.0 / 13.0, [0, 16, 24]),
    ] {
        for short_first in [false, true] {
            let long = format!("<td>{}</td>", long_words.join(" "));
            let short = format!(
                "<td style='vertical-align:{alignment}'><div><p>{}</p></div></td>",
                short_words.join(" ")
            );
            let cells = if short_first {
                format!("{short}{long}")
            } else {
                format!("{long}{short}")
            };
            let output = render(
                "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}",
                &format!("<table><tr>{cells}</tr></table>"),
            );
            assert_eq!(
                output.page_count, 3,
                "{alignment}, short first {short_first}"
            );
            let trace = output.layout.unwrap();
            let first = trace.pages[0]
                .iter()
                .find(|line| line.text.starts_with("A000"))
                .unwrap();
            let short = trace.pages[1]
                .iter()
                .find(|line| line.text.starts_with("B000"))
                .unwrap();
            assert_pt(short.baseline - first.baseline, offset);
            for (page, expected) in counts.into_iter().enumerate() {
                let count = trace.pages[page]
                    .iter()
                    .flat_map(|line| line.text.split_ascii_whitespace())
                    .filter(|word| word.starts_with("B0"))
                    .count();
                assert_eq!(count, expected, "{alignment} page {}", page + 1);
            }
            for (prefix, expected) in [("A0", &long_words), ("B0", &short_words)] {
                let actual = trace
                    .pages
                    .iter()
                    .flatten()
                    .flat_map(|line| line.text.split_ascii_whitespace())
                    .filter(|word| word.starts_with(prefix))
                    .collect::<Vec<_>>();
                assert_eq!(
                    actual, *expected,
                    "{alignment}: {prefix} source conservation"
                );
            }
        }
    }
}

#[test]
fn split_rowspan_alignment_does_not_depend_on_sibling_cell_order() {
    let words = tokens('A', 90);
    // Three columns admit 3/5 tokens per line, giving middle padding 145/85pt
    // and bottom padding 290/170pt on narrow/wide pages.
    for (alignment, expected_page, expected_offset) in [
        ("middle", 1, 85.0 * (1.0 - 60.0 / 145.0)),
        ("bottom", 3, 170.0 * (1.0 - 120.0 / 290.0 - 60.0 / 170.0)),
    ] {
        let mut placements = Vec::new();
        for short_first in [false, true] {
            let long = format!("<td rowspan=2>{}</td>", words.join(" "));
            let short = format!("<td rowspan=2 style='vertical-align:{alignment}'>Short</td>");
            let cells = if short_first {
                format!("{short}{long}")
            } else {
                format!("{long}{short}")
            };
            let output = render(
                "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}",
                &format!("<table><tr>{cells}<td>Top</td></tr><tr><td>Bottom</td></tr></table>"),
            );
            let trace = output.layout.unwrap();
            let actual = trace
                .pages
                .iter()
                .flatten()
                .flat_map(|line| line.text.split_ascii_whitespace())
                .filter(|word| word.starts_with("A0"))
                .collect::<Vec<_>>();
            assert_eq!(actual, words, "{alignment}, short first {short_first}");
            let short = trace
                .pages
                .iter()
                .enumerate()
                .flat_map(|(page, lines)| {
                    lines
                        .iter()
                        .filter(|line| line.text == "Short")
                        .map(move |line| (page, line.baseline))
                })
                .collect::<Vec<_>>();
            assert_eq!(short.len(), 1, "{alignment}, short first {short_first}");
            let first = trace.pages[0]
                .iter()
                .find(|line| line.text.starts_with("A000"))
                .unwrap();
            assert_eq!(output.page_count, 4);
            assert_eq!(short[0].0, expected_page);
            assert_pt(short[0].1 - first.baseline, expected_offset);
            placements.push((output.page_count, short[0]));
        }
        assert_eq!(
            placements[0].0, placements[1].0,
            "{alignment}: swapping equal-width sibling cells changes page count: {placements:?}"
        );
        assert_eq!(
            placements[0].1.0, placements[1].1.0,
            "{alignment}: swapping equal-width sibling cells changes alignment page: {placements:?}"
        );
        assert_pt(placements[0].1.1, placements[1].1.1);
    }
}

#[test]
fn aligned_fragmented_cell_background_starts_at_the_row_not_its_text() {
    let lines = (0..18)
        .map(|index| format!("Line{index:02}"))
        .collect::<Vec<_>>()
        .join("<br>");
    for alignment in ["middle", "bottom"] {
        let output = render(
            &format!(
                "@page{{size:300pt 80pt;margin:10pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}}td+td{{vertical-align:{alignment};background:blue}}"
            ),
            &format!("<table><tr><td>{lines}</td><td>Short</td></tr></table>"),
        );
        assert_eq!(output.page_count, 3, "{alignment}");
        let trace = output.layout.unwrap();
        let short = trace
            .pages
            .iter()
            .enumerate()
            .find_map(|(page, lines)| {
                lines
                    .iter()
                    .find(|line| line.text == "Short")
                    .map(|line| (page, line.baseline))
            })
            .unwrap();
        assert_eq!(short.0, if alignment == "middle" { 1 } else { 2 });
        for (page, items) in trace.display_lists.iter().enumerate() {
            let paints = items
                .iter()
                .filter_map(|item| match item {
                    ferri_pdf::DisplayItem::Box(paint)
                        if paint.decoration.background.rgb == [0, 0, 255]
                            && paint.decoration.background.alpha > 0.0 =>
                    {
                        Some(paint)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                paints.len(),
                1,
                "{alignment} page {} needs a background even before text begins",
                page + 1
            );
            let background = paints[0]
                .background_box
                .expect("sliced cell background must span all fragments in page order");
            assert_pt(background.height, 180.0);
            assert_pt(background.y, 10.0 - page as f32 * 60.0);
            for (label, actual, expected) in [
                ("x", paints[0].rect.x, 150.0),
                ("y", paints[0].rect.y, 10.0),
                ("width", paints[0].rect.width, 140.0),
                ("height", paints[0].rect.height, 60.0),
            ] {
                assert!(
                    (actual - expected).abs() < 0.01,
                    "{alignment} page {} {label}: {actual}, expected {expected}",
                    page + 1
                );
            }
        }
    }
}

#[test]
fn forced_break_inside_a_cell_preserves_parallel_content_and_backgrounds() {
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;vertical-align:top;background:red}td+td{background:blue}.next{break-before:page}",
        "<table><tr><td><p><a href='#second'>First</a></p><p class=next id=second>Second</p></td><td>Neighbour</td></tr></table>",
    );
    assert_eq!(output.page_count, 2);
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["First", "Neighbour"]
    );
    assert_eq!(
        trace.pages[1]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Second"]
    );
    assert_pt(trace.pages[1][0].x, 10.0);
    assert_eq!(
        trace
            .destinations
            .iter()
            .flatten()
            .map(|destination| destination.page)
            .collect::<Vec<_>>(),
        [1]
    );
    for (page, width) in [(0, 140.0), (1, 240.0)] {
        for (rgb, x) in [([255, 0, 0], 10.0), ([0, 0, 255], 10.0 + width)] {
            let paints = trace.display_lists[page]
                .iter()
                .filter_map(|item| match item {
                    ferri_pdf::DisplayItem::Box(paint)
                        if paint.decoration.background.rgb == rgb
                            && paint.decoration.background.alpha > 0.0 =>
                    {
                        Some(paint)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                paints.len(),
                1,
                "page {} requires one {rgb:?} cell fragment",
                page + 1
            );
            assert_pt(paints[0].rect.x, x);
            assert_pt(paints[0].rect.width, width);
            assert_pt(paints[0].rect.height, if page == 0 { 60.0 } else { 10.0 });
        }
    }
}

#[test]
fn block_after_forced_cell_break_is_measured_at_destination_column_width() {
    let words = tokens('A', 32);
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;vertical-align:top}.next{break-before:page}",
        &format!(
            "<table><tr><td><p>Before</p><p class=next>{}</p></td><td>Neighbour</td></tr></table>",
            words.join(" ")
        ),
    );
    assert_eq!(
        output.page_count, 2,
        "four lines fit the wider destination column"
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages[1].len(), 4);
    assert_eq!(trace.pages[1][0].text.split_ascii_whitespace().count(), 8);
    assert_eq!(
        trace.pages[1]
            .iter()
            .flat_map(|line| line.text.split_ascii_whitespace())
            .collect::<Vec<_>>(),
        words
    );
}

#[test]
fn table_layer_backgrounds_cover_every_fragment_of_a_long_cell() {
    let words = tokens('A', 240);
    for layer in ["colgroup", "col", "tbody", "tr"] {
        let output = render(
            &format!(
                "@page{{size:300pt 80pt;margin:10pt}}@page:left{{size:500pt 80pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}}{layer}{{background:red}}"
            ),
            &format!(
                "<table><colgroup><col></colgroup><tbody><tr><td>{}</td></tr></tbody></table>",
                words.join(" ")
            ),
        );
        assert_eq!(output.page_count, 4, "{layer}");
        let trace = output.layout.unwrap();
        for (page, (width, height)) in [(280.0, 60.0), (480.0, 60.0), (280.0, 60.0), (480.0, 30.0)]
            .into_iter()
            .enumerate()
        {
            let paints = trace.display_lists[page]
                .iter()
                .filter_map(|item| match item {
                    ferri_pdf::DisplayItem::Box(paint)
                        if paint.decoration.background.rgb == [255, 0, 0]
                            && paint.decoration.background.alpha > 0.0 =>
                    {
                        Some(paint)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                paints.len(),
                1,
                "{layer} page {} requires one clipped layer",
                page + 1
            );
            for (label, actual, expected) in [
                ("x", paints[0].rect.x, 10.0),
                ("y", paints[0].rect.y, 10.0),
                ("width", paints[0].rect.width, width),
                ("height", paints[0].rect.height, height),
            ] {
                assert!(
                    (actual - expected).abs() < 0.01,
                    "{layer} page {} {label}: {actual}, expected {expected}",
                    page + 1
                );
            }
        }
    }
}

#[test]
fn fragmented_table_layer_images_keep_their_origin_and_precede_cell_paint() {
    let words = tokens('A', 240).join(" ");
    for layer in ["colgroup", "col", "tbody", "tr"] {
        let output = render(
            &format!(
                "@page{{size:300pt 80pt;margin:10pt}}@page:left{{size:500pt 80pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;background:lime}}{layer}{{background-image:linear-gradient(red 50%,blue 50%)}}"
            ),
            &format!(
                "<table><colgroup><col></colgroup><tbody><tr><td>{words}</td></tr></tbody></table>"
            ),
        );
        assert_eq!(output.page_count, 4, "{layer}");
        let trace = output.layout.unwrap();
        for (page, items) in trace.display_lists.iter().enumerate() {
            let (index, image) = items
                .iter()
                .enumerate()
                .find_map(|(index, item)| match item {
                    ferri_pdf::DisplayItem::Box(paint)
                        if paint.decoration.backgrounds.has_images() =>
                    {
                        Some((index, paint))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{layer} page {page} lost its image layer"));
            let area = image
                .background_box
                .expect("table layer image has a positioning area");
            assert_pt(area.y, 10.0 - page as f32 * 60.0);
            assert_pt(area.height, 210.0);
            assert_pt(area.width, if page % 2 == 0 { 280.0 } else { 480.0 });
            let cell = items
                .iter()
                .position(|item| {
                    matches!(item,
                ferri_pdf::DisplayItem::Box(paint) if paint.decoration.background.rgb == [0, 255, 0]
                    && paint.decoration.background.alpha > 0.0)
                })
                .expect("cell background");
            assert!(
                index < cell,
                "{layer} page {page}: layer {index} must precede cell {cell}"
            );
        }
    }
}

#[test]
fn table_group_image_positioning_area_includes_resumed_and_following_bands() {
    let words = tokens('A', 240).join(" ");
    for layer in ["colgroup", "col", "tbody"] {
        let output = render(
            &format!(
                "@page{{size:300pt 80pt;margin:10pt}}@page:left{{size:500pt 80pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}}{layer}{{background-image:linear-gradient(red 50%,blue 50%)}}"
            ),
            &format!(
                "<table><colgroup><col></colgroup><tbody><tr><td>{words}</td></tr><tr><td>End</td></tr></tbody></table>"
            ),
        );
        assert_eq!(output.page_count, 4, "{layer}");
        let trace = output.layout.unwrap();
        let mut count = 0;
        for (page, items) in trace.display_lists.iter().enumerate() {
            for image in items.iter().filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint) if paint.decoration.backgrounds.has_images() => {
                    Some(paint)
                }
                _ => None,
            }) {
                count += 1;
                let area = image
                    .background_box
                    .expect("table layer image positioning area");
                assert!(
                    (area.height - 220.0).abs() < 0.01,
                    "{layer} page {page} layer at {:?}: whole group height is 220, got {}",
                    image.rect,
                    area.height
                );
                assert_pt(area.y, 10.0 - page as f32 * 60.0);
            }
        }
        assert_eq!(
            count, 5,
            "four long-cell fragments and the following cell: {layer}"
        );
    }
}

#[test]
fn table_column_images_include_multiple_split_rows_and_keep_group_boundaries() {
    let first = tokens('A', 240).join(" ");
    let second = tokens('B', 240).join(" ");
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}colgroup{background-image:linear-gradient(red,blue)}tbody{background-image:linear-gradient(lime,yellow)}",
        &format!(
            "<table><colgroup><col></colgroup><tbody><tr><td>{first}</td></tr></tbody><tbody><tr><td>{second}</td></tr></tbody></table>"
        ),
    );
    assert_eq!(output.page_count, 7);
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.iter().map(Vec::len).collect::<Vec<_>>(),
        [6, 6, 6, 6, 6, 6, 5]
    );
    for (page, items) in trace.display_lists.iter().enumerate() {
        let images = items
            .iter()
            .filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint) if paint.decoration.backgrounds.has_images() => {
                    Some(paint)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(images.len(), if page == 3 { 4 } else { 2 }, "page {page}");
        const BACKGROUND_LAYERS: usize = 2;
        for pair in images.as_chunks::<BACKGROUND_LAYERS>().0 {
            let column = pair[0].background_box.unwrap();
            let group = pair[1].background_box.unwrap();
            assert_pt(column.height, 410.0);
            assert_pt(column.y, 10.0 - page as f32 * 60.0);
            let second_group = page > 3 || (page == 3 && pair[0].rect.y > 10.0);
            assert_pt(group.height, if second_group { 200.0 } else { 210.0 });
            assert_pt(
                group.y,
                (if second_group { 220.0 } else { 10.0 }) - page as f32 * 60.0,
            );
        }
    }
}

#[test]
fn repeated_header_image_keeps_each_instances_used_height() {
    let header = tokens('H', 30).join(" ");
    let rows = (0..12)
        .map(|index| format!("<tr><td>Body{index}</td></tr>"))
        .collect::<String>();
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}thead{background-image:linear-gradient(red,blue)}",
        &format!("<table><thead><tr><td>{header}</td></tr></thead><tbody>{rows}</tbody></table>"),
    );
    assert_eq!(output.page_count, 4);
    let trace = output.layout.unwrap();
    for (page, items) in trace.display_lists.iter().enumerate() {
        let images = items
            .iter()
            .filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint) if paint.decoration.backgrounds.has_images() => {
                    Some(paint)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(images.len(), 1, "page {page}");
        let expected = if page % 2 == 0 { 40.0 } else { 20.0 };
        assert_pt(images[0].rect.height, expected);
        assert_pt(images[0].background_box.unwrap().height, expected);
        assert_pt(images[0].background_box.unwrap().y, 10.0);
    }
}

#[test]
fn collapsed_cell_sides_follow_every_fragment_and_destination_width() {
    let words = tokens('A', 240).join(" ");
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-collapse:collapse}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;border-left:2pt solid red;border-right:2pt solid red}",
        &format!("<table><tr><td>{words}</td></tr></table>"),
    );
    assert_eq!(output.page_count, 4);
    let trace = output.layout.unwrap();
    for (page, (width, height)) in [(300.0, 60.0), (500.0, 60.0), (300.0, 60.0), (500.0, 30.0)]
        .into_iter()
        .enumerate()
    {
        let mut sides = trace.display_lists[page]
            .iter()
            .filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.borders.iter().any(|border| {
                        border.width == 2.0
                            && border.color.rgb == [255, 0, 0]
                            && border.color.alpha > 0.0
                    }) =>
                {
                    Some(paint.rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        sides.sort_by(|a, b| a.x.total_cmp(&b.x));
        assert_eq!(sides.len(), 2, "page {page}: {sides:?}");
        for (rect, x) in sides.iter().zip([10.0, width - 12.0]) {
            assert!(
                (rect.height - height).abs() < 0.01,
                "page {page}: {rect:?} must have height {height}"
            );
            assert_pt(rect.x, x);
            assert_pt(rect.y, 10.0);
            assert_pt(rect.width, 2.0);
        }
    }
}

#[test]
fn collapsed_row_page_cuts_do_not_create_horizontal_grid_borders() {
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-collapse:collapse}td{padding:0;vertical-align:top;border:2pt solid red}p+p{break-before:page}",
        "<table><tr><td><p>First</p><p>Middle</p><p>Last</p></td></tr></table>",
    );
    assert_eq!(output.page_count, 3);
    let trace = output.layout.unwrap();
    for (page, items) in trace.display_lists.iter().enumerate() {
        let borders = items
            .iter()
            .filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.borders.iter().any(|border| {
                        border.width == 2.0
                            && border.color.rgb == [255, 0, 0]
                            && border.color.alpha > 0.0
                    }) =>
                {
                    Some(paint.rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let horizontal = borders
            .iter()
            .filter(|rect| rect.width > rect.height)
            .collect::<Vec<_>>();
        assert_eq!(
            horizontal.len(),
            usize::from(page != 1),
            "page {page}: only actual row boundaries have horizontal borders: {borders:?}"
        );
        assert_eq!(
            borders
                .iter()
                .filter(|rect| rect.height > rect.width)
                .count(),
            2,
            "page {page}: {borders:?}"
        );
        for rect in borders {
            assert!(
                rect.y >= 9.0 && rect.y + rect.height <= 71.0,
                "page {page} border outside fragment: {rect:?}"
            );
        }
    }
}

#[test]
fn active_table_row_repeats_headers_and_footers_without_replaying_body_text() {
    let words = tokens('A', 240);
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}",
        &format!(
            "<table><thead><tr><td>Head</td></tr></thead><tbody><tr><td>{}</td></tr></tbody><tfoot><tr><td>Foot</td></tr></tfoot></table>",
            words.join(" ")
        ),
    );
    let trace = output.layout.unwrap();
    for (page, lines) in trace.pages.iter().enumerate() {
        assert_eq!(
            lines.iter().filter(|line| line.text == "Head").count(),
            1,
            "page {page}: header: {:?}",
            lines.iter().map(|line| &line.text).collect::<Vec<_>>()
        );
        assert_eq!(
            lines.iter().filter(|line| line.text == "Foot").count(),
            1,
            "page {page}: footer: {:?}",
            lines.iter().map(|line| &line.text).collect::<Vec<_>>()
        );
    }
    for (page, lines) in trace.pages.iter().enumerate() {
        let header = lines.iter().find(|line| line.text == "Head").unwrap();
        let footer = lines.iter().find(|line| line.text == "Foot").unwrap();
        let body = lines
            .iter()
            .filter(|line| line.text != "Head" && line.text != "Foot")
            .collect::<Vec<_>>();
        assert_eq!(body.len(), if page == 5 { 1 } else { 4 });
        assert_pt(body[0].baseline - header.baseline, 10.0);
        assert_pt(footer.baseline - body.last().unwrap().baseline, 10.0);
    }
    assert_eq!(output.page_count, 6);
    let body = trace
        .pages
        .iter()
        .flat_map(|lines| lines.iter())
        .filter(|line| line.text != "Head" && line.text != "Foot")
        .flat_map(|line| line.text.split_ascii_whitespace())
        .collect::<Vec<_>>();
    assert_eq!(body, words, "body source is consumed exactly once");
}

#[test]
fn unequal_active_cells_share_repeated_groups_and_reserved_paint_areas() {
    let left = tokens('A', 80);
    let right = tokens('B', 40);
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}tbody td+td{background:blue}",
        &format!(
            "<table><thead><tr><td>HeadLeft</td><td>HeadRight</td></tr></thead><tbody><tr><td>{}</td><td>{}</td></tr></tbody><tfoot><tr><td>FootLeft</td><td>FootRight</td></tr></tfoot></table>",
            left.join(" "),
            right.join(" ")
        ),
    );
    assert_eq!(output.page_count, 4);
    let trace = output.layout.unwrap();
    for (page, lines) in trace.pages.iter().enumerate() {
        for label in ["HeadLeft", "HeadRight", "FootLeft", "FootRight"] {
            let found = lines
                .iter()
                .filter(|line| line.text == label)
                .collect::<Vec<_>>();
            assert_eq!(
                found.len(),
                1,
                "page {page}: {label} must repeat once across both lanes"
            );
            assert_pt(
                found[0].x,
                if label.ends_with("Right") {
                    if page % 2 == 0 { 150.0 } else { 250.0 }
                } else {
                    10.0
                },
            );
        }
        let backgrounds = trace.display_lists[page]
            .iter()
            .filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint.rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(backgrounds.len(), 1, "page {page}");
        assert_pt(backgrounds[0].y, 20.0);
        assert_pt(backgrounds[0].height, if page == 3 { 20.0 } else { 40.0 });
        assert_pt(backgrounds[0].x, if page % 2 == 0 { 150.0 } else { 250.0 });
    }
    for (prefix, expected) in [('A', left), ('B', right)] {
        let actual = trace
            .pages
            .iter()
            .flat_map(|lines| lines.iter())
            .filter(|line| line.text.starts_with(prefix))
            .flat_map(|line| line.text.split_ascii_whitespace())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "cell {prefix} consumes source once");
    }
}

#[test]
fn active_row_reserves_remeasured_header_height_on_each_destination_page() {
    let headers = tokens('H', 30);
    let words = tokens('A', 240);
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}",
        &format!(
            "<table><thead><tr><td>{}</td></tr></thead><tbody><tr><td>{}</td></tr></tbody><tfoot><tr><td>Foot</td></tr></tfoot></table>",
            headers.join(" "),
            words.join(" ")
        ),
    );
    assert_eq!(output.page_count, 10);
    let trace = output.layout.unwrap();
    for (page, lines) in trace.pages.iter().enumerate() {
        let head = lines
            .iter()
            .filter(|line| line.text.starts_with('H'))
            .collect::<Vec<_>>();
        let body = lines
            .iter()
            .filter(|line| line.text.starts_with('A'))
            .collect::<Vec<_>>();
        let foot = lines
            .iter()
            .filter(|line| line.text == "Foot")
            .collect::<Vec<_>>();
        assert_eq!(head.len(), if page % 2 == 0 { 4 } else { 2 }, "page {page}");
        assert_eq!(
            head.iter()
                .flat_map(|line| line.text.split_ascii_whitespace())
                .collect::<Vec<_>>(),
            headers,
            "page {page} repeats the full header once"
        );
        assert_eq!(foot.len(), 1, "page {page}");
        assert!(
            !body.is_empty(),
            "page {page} must progress through the body"
        );
        assert_pt(body[0].baseline - head.last().unwrap().baseline, 10.0);
        assert_pt(foot[0].baseline - body.last().unwrap().baseline, 10.0);
    }
    let actual = trace
        .pages
        .iter()
        .flat_map(|lines| lines.iter())
        .filter(|line| line.text.starts_with('A'))
        .flat_map(|line| line.text.split_ascii_whitespace())
        .collect::<Vec<_>>();
    assert_eq!(actual, words);
}

#[test]
fn aligned_short_cell_counts_only_body_space_between_repeated_groups() {
    let words = tokens('A', 80).join(" ");
    for (align, expected_page) in [("middle", 2), ("bottom", 4)] {
        let output = render(
            &format!(
                "@page{{size:300pt 80pt;margin:10pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}}tbody td+td{{vertical-align:{align};background:blue}}"
            ),
            &format!(
                "<table><thead><tr><td>HeadLeft</td><td>HeadRight</td></tr></thead><tbody><tr><td>{words}</td><td>Short</td></tr></tbody><tfoot><tr><td>FootLeft</td><td>FootRight</td></tr></tfoot></table>"
            ),
        );
        assert_eq!(output.page_count, 5, "{align}");
        let trace = output.layout.unwrap();
        let short_pages = trace
            .pages
            .iter()
            .enumerate()
            .filter_map(|(page, lines)| {
                lines
                    .iter()
                    .any(|line| line.text == "Short")
                    .then_some(page)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            short_pages,
            [expected_page],
            "{align}: repeated groups do not consume cell alignment space"
        );
        for (page, items) in trace.display_lists.iter().enumerate() {
            let blue = items
                .iter()
                .filter_map(|item| match item {
                    ferri_pdf::DisplayItem::Box(paint)
                        if paint.decoration.background.rgb == [0, 0, 255]
                            && paint.decoration.background.alpha > 0.0 =>
                    {
                        Some(paint.rect)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(blue.len(), 1, "{align} page {page}");
            assert_pt(blue[0].y, 20.0);
            assert_pt(blue[0].height, 40.0);
        }
    }
}

#[test]
fn row_fitting_a_bare_page_splits_to_leave_room_for_repeated_groups() {
    let words = tokens('A', 45).join(" ");
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}",
        &format!(
            "<table><thead><tr><td>Head</td></tr></thead><tbody><tr><td>{words}</td></tr></tbody><tfoot><tr><td>Foot</td></tr></tfoot></table>"
        ),
    );
    assert_eq!(output.page_count, 2);
    let trace = output.layout.unwrap();
    for (page, lines) in trace.pages.iter().enumerate() {
        let texts = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            texts.iter().filter(|&&text| text == "Head").count(),
            1,
            "page {page}: {texts:?}"
        );
        assert_eq!(
            texts.iter().filter(|&&text| text == "Foot").count(),
            1,
            "page {page}: {texts:?}"
        );
        assert_eq!(
            texts.iter().filter(|text| text.starts_with('A')).count(),
            if page == 0 { 4 } else { 1 }
        );
    }
}

#[test]
fn nested_active_tables_share_page_space_without_overlapping_repeated_groups() {
    let words = tokens('A', 120);
    let output = render(
        "@page{size:300pt 100pt;margin:10pt}@page:left{size:500pt 100pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}",
        &format!(
            "<table><thead><tr><td>OuterHead</td></tr></thead><tbody><tr><td><table><thead><tr><td>InnerHead</td></tr></thead><tbody><tr><td>{}</td></tr></tbody><tfoot><tr><td>InnerFoot</td></tr></tfoot></table></td></tr></tbody><tfoot><tr><td>OuterFoot</td></tr></tfoot></table>",
            words.join(" ")
        ),
    );
    assert_eq!(output.page_count, 3);
    let trace = output.layout.unwrap();
    for (page, lines) in trace.pages.iter().enumerate() {
        let find = |label: &str| {
            let matching = lines
                .iter()
                .filter(|line| line.text == label)
                .collect::<Vec<_>>();
            assert_eq!(
                matching.len(),
                1,
                "page {page} {label}: {:?}",
                lines.iter().map(|line| &line.text).collect::<Vec<_>>()
            );
            matching[0]
        };
        let body = lines
            .iter()
            .filter(|line| line.text.starts_with('A'))
            .collect::<Vec<_>>();
        assert_eq!(body.len(), if page == 2 { 3 } else { 4 }, "page {page}");
        assert_pt(
            find("InnerHead").baseline - find("OuterHead").baseline,
            10.0,
        );
        assert_pt(body[0].baseline - find("InnerHead").baseline, 10.0);
        assert_pt(
            find("InnerFoot").baseline - body.last().unwrap().baseline,
            10.0,
        );
        assert_pt(
            find("OuterFoot").baseline - find("InnerFoot").baseline,
            10.0,
        );
    }
    let actual = trace
        .pages
        .iter()
        .flat_map(|lines| lines.iter())
        .filter(|line| line.text.starts_with('A'))
        .flat_map(|line| line.text.split_ascii_whitespace())
        .collect::<Vec<_>>();
    assert_eq!(actual, words);
}

#[test]
fn nested_table_final_footer_uses_the_destination_width() {
    let words = tokens('A', 90).join(" ");
    let output = render(
        "@page{size:300pt 100pt;margin:10pt}@page:left{size:500pt 100pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}.inner tfoot{background:blue}",
        &format!(
            "<table><thead><tr><td>OuterHead</td></tr></thead><tbody><tr><td><table class=inner><thead><tr><td>InnerHead</td></tr></thead><tbody><tr><td>{words}</td></tr></tbody><tfoot><tr><td>InnerFoot</td></tr></tfoot></table></td></tr></tbody><tfoot><tr><td>OuterFoot</td></tr></tfoot></table>"
        ),
    );
    assert_eq!(output.page_count, 2);
    let trace = output.layout.unwrap();
    for (page, items) in trace.display_lists.iter().enumerate() {
        let blue = items
            .iter()
            .filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint.rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(blue.len(), 1, "page {page}");
        assert_pt(blue[0].width, if page == 0 { 280.0 } else { 480.0 });
        assert_pt(blue[0].x, 10.0);
        assert_pt(blue[0].y, 70.0);
        assert_pt(blue[0].height, 10.0);
    }
}

#[test]
fn nested_row_boundaries_reserve_both_inner_and_outer_footers() {
    let rows = (0..12)
        .map(|index| format!("<tr><td>R{index:02}</td></tr>"))
        .collect::<String>();
    let output = render(
        "@page{size:300pt 100pt;margin:10pt}@page:left{size:500pt 100pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}",
        &format!(
            "<table><thead><tr><td>OuterHead</td></tr></thead><tbody><tr><td><table><thead><tr><td>InnerHead</td></tr></thead><tbody>{rows}</tbody><tfoot><tr><td>InnerFoot</td></tr></tfoot></table></td></tr></tbody><tfoot><tr><td>OuterFoot</td></tr></tfoot></table>"
        ),
    );
    let trace = output.layout.unwrap();
    assert_eq!(
        output.page_count,
        3,
        "{:?}",
        trace
            .pages
            .iter()
            .map(|lines| lines
                .iter()
                .map(|line| (&line.text, line.baseline))
                .collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
    let mut consumed = Vec::new();
    for (page, lines) in trace.pages.iter().enumerate() {
        let find = |label: &str| {
            let matching = lines
                .iter()
                .filter(|line| line.text == label)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "page {page}: {label}");
            matching[0]
        };
        let body = lines
            .iter()
            .filter(|line| line.text.starts_with('R'))
            .collect::<Vec<_>>();
        assert_eq!(
            body.len(),
            4,
            "page {page}: {:?}",
            lines.iter().map(|line| &line.text).collect::<Vec<_>>()
        );
        assert_pt(body[0].baseline - find("InnerHead").baseline, 10.0);
        assert_pt(
            find("InnerFoot").baseline - body.last().unwrap().baseline,
            10.0,
        );
        assert_pt(
            find("OuterFoot").baseline - find("InnerFoot").baseline,
            10.0,
        );
        consumed.extend(body.iter().map(|line| line.text.clone()));
    }
    assert_eq!(
        consumed,
        (0..12)
            .map(|index| format!("R{index:02}"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn distributed_row_space_preserves_the_remaining_line_capacity() {
    const CONTENT_BOTTOM: f32 = 70.0;
    let words = tokens('A', 90).join(" ");
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}",
        &format!(
            "<table><tr><td rowspan=2>{words}</td><td>Top</td></tr><tr><td>Bottom</td></tr></table>"
        ),
    );
    let trace = output.layout.unwrap();
    let mut bottom_count = 0;
    for (page, lines) in trace.pages.iter().enumerate() {
        for line in lines {
            bottom_count += usize::from(line.text == "Bottom");
            assert!(
                line.baseline <= CONTENT_BOTTOM,
                "page {page}: {:?} baseline {} extends beyond content bottom {CONTENT_BOTTOM}",
                line.text,
                line.baseline,
            );
        }
    }
    assert_eq!(
        bottom_count, 1,
        "following-row content must occur exactly once"
    );
}

#[test]
fn nonspanning_cell_background_stops_at_its_own_row_inside_a_split_band() {
    let words = tokens('A', 90).join(" ");
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}.short{background:blue}",
        &format!(
            "<table><tr><td rowspan=2>{words}</td><td class=short>Top</td></tr><tr><td>Bottom</td></tr></table>"
        ),
    );
    assert_eq!(output.page_count, 4);
    let trace = output.layout.unwrap();
    for (page, items) in trace.display_lists.iter().enumerate() {
        let blue = items
            .iter()
            .filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint.rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            blue.len(),
            usize::from(page < 2),
            "page {page}: short first-row cell must not paint later rows: {blue:?}"
        );
        if page < 2 {
            assert_pt(blue[0].x, 150.0);
            assert_pt(blue[0].y, 10.0);
            // The 230pt spanning cell grows both 10pt rows to 115pt.
            assert_pt(blue[0].height, if page == 0 { 60.0 } else { 55.0 });
        }
    }
}

#[test]
fn later_row_in_a_split_span_starts_after_actual_preceding_row_progress() {
    let left = tokens('A', 120).join(" ");
    let right = tokens('B', 60).join(" ");
    let output = render(
        "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}.first{background:blue}.last{background:lime}",
        &format!(
            "<table><tr><td rowspan=2>{left}</td><td class=first>{right}</td></tr><tr><td class=last>Bottom</td></tr></table>"
        ),
    );
    assert_eq!(output.page_count, 4);
    let trace = output.layout.unwrap();
    let bottom = trace
        .pages
        .iter()
        .enumerate()
        .flat_map(|(page, lines)| {
            lines
                .iter()
                .filter(|line| line.text == "Bottom")
                .map(move |line| (page, line))
        })
        .collect::<Vec<_>>();
    assert_eq!(bottom.len(), 1);
    // Reflow produces 210pt of spanning content and 110pt/10pt row minima.
    const FIRST_ROW_HEIGHT: f32 = 210.0 * 110.0 / (110.0 + 10.0);
    const PAGE_CAPACITY: f32 = 60.0;
    assert_eq!(bottom[0].0, 3, "following row follows distributed height");
    assert_pt(bottom[0].1.x, 250.0);
    assert_pt(
        bottom[0].1.baseline - trace.pages[3][0].baseline,
        FIRST_ROW_HEIGHT - 3.0 * PAGE_CAPACITY,
    );
    for (page, items) in trace.display_lists.iter().enumerate() {
        let green = items
            .iter()
            .filter_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 255, 0]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint.rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            green.len(),
            usize::from(page == 3),
            "page {page}: {green:?}"
        );
        if page == 3 {
            assert_pt(green[0].x, if page % 2 == 0 { 150.0 } else { 250.0 });
            assert_pt(green[0].y, 10.0 + FIRST_ROW_HEIGHT - 3.0 * PAGE_CAPACITY);
            let height = 210.0 - FIRST_ROW_HEIGHT;
            assert!(
                (green[0].height - height).abs() < 0.01,
                "page {page}: expected height {height}, got {:?}",
                green[0]
            );
        }
    }
}

#[test]
fn internal_row_constraints_apply_while_a_spanning_cell_continues() {
    for (constraint, width, next_page) in [
        ("break-before:page", 240.0, 1),
        ("page:wide", 400.0, 1),
        ("break-before:right", 240.0, 2),
        ("page:wide;break-before:right", 400.0, 2),
    ] {
        for reversed in [false, true] {
            for count in [1, 60] {
                let words = (0..count)
                    .map(|index| format!("S{index:03}"))
                    .collect::<Vec<_>>();
                let span = format!("<td class=span rowspan=2>{}</td>", words.join(" "));
                let first = if reversed {
                    format!("<td>First</td>{span}")
                } else {
                    format!("{span}<td>First</td>")
                };
                let output = render(
                    &format!(
                        "@page{{size:240pt 120pt;margin:10pt}}@page wide{{size:400pt 120pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}}.span{{background:#ddd;border:1pt solid red}}.second{{{constraint}}}"
                    ),
                    &format!(
                        "<table><tr>{first}</tr><tr class=second><td>Second</td></tr></table><p>After</p>"
                    ),
                );
                let trace = output.layout.unwrap();
                let locate = |label: &str| {
                    trace
                        .pages
                        .iter()
                        .enumerate()
                        .filter_map(|(page, lines)| {
                            lines.iter().any(|line| line.text == label).then_some(page)
                        })
                        .collect::<Vec<_>>()
                };
                assert_eq!(locate("First"), [0], "{constraint}/{count}: first row");
                assert_eq!(
                    locate("Second"),
                    [next_page],
                    "{constraint}/{count}: internal row boundary must select the next page before resuming the span"
                );
                if next_page == 2 {
                    assert!(trace.page_records[1].request.blank);
                    assert!(trace.pages[1].is_empty());
                    assert!(!trace.display_lists[1].iter().any(|item| matches!(item, ferri_pdf::DisplayItem::Box(paint) if paint.decoration.background.rgb == [221, 221, 221])), "{constraint}/{count}/{reversed}: the spanning cell must not paint the side-inserted blank page");
                }
                let second = trace.pages[next_page]
                    .iter()
                    .find(|line| line.text == "Second")
                    .unwrap();
                assert_pt(second.x, if reversed { 10.0 } else { width / 2.0 });
                assert_pt(trace.page_records[next_page].geometry.paper[0], width);
                let actual = trace
                    .pages
                    .iter()
                    .flatten()
                    .flat_map(|line| line.text.split_whitespace())
                    .filter(|word| word.starts_with('S') && *word != "Second")
                    .collect::<Vec<_>>();
                assert_eq!(
                    actual, words,
                    "{constraint}/{count}: spanning source must be consumed once and in order"
                );
                assert_eq!(locate("After").len(), 1, "following flow must survive");
            }
        }
    }
}

#[test]
fn internal_row_breaks_visit_empty_rows_and_keep_crossing_spans_alive() {
    for count in [1, 60] {
        let words = (0..count)
            .map(|index| format!("S{index:03}"))
            .collect::<Vec<_>>();
        let trace = render(
            "@page{size:240pt 120pt;margin:10pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}.span{background:#ddd;border:1pt solid red}.second{break-before:page}",
            &format!("<table><tr><td class=span rowspan=3>{}</td><td>First</td></tr><tr class=second></tr><tr><td>Third</td></tr></table><p>After</p>", words.join(" ")),
        ).layout.unwrap();
        assert!(trace.pages[0].iter().any(|line| line.text == "First"));
        assert!(
            trace.pages[1].iter().any(|line| line.text == "Third"),
            "{count}: the empty second source row must still create its forced boundary"
        );
        let actual = trace
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.text.split_whitespace())
            .filter(|word| word.starts_with('S'))
            .collect::<Vec<_>>();
        assert_eq!(actual, words);
        for (page, lines) in trace.pages.iter().enumerate() {
            if lines.iter().any(|line| line.text.starts_with('S')) {
                assert!(trace.display_lists[page].iter().any(|item| matches!(item, ferri_pdf::DisplayItem::Box(paint) if paint.decoration.background.rgb == [221, 221, 221])), "{count}: spanning content on page {page} must retain its background");
            }
        }
    }
}

#[test]
fn internal_row_constraints_do_not_fragment_positioned_tables() {
    let output = render(
        "@page{size:240pt 120pt;margin:10pt}@page wide{size:400pt 120pt}.positioned{position:absolute;left:10pt;top:10pt}table{border-spacing:0}td{padding:0}.second{page:wide;break-before:right}",
        "<div class=positioned><table><tr><td rowspan=2>Span</td><td>First</td></tr><tr class=second><td>Second</td></tr></table></div>",
    );
    assert_eq!(
        output.page_count, 1,
        "positioned table layout is unbroken even when source rows request breaks"
    );
    let trace = output.layout.unwrap();
    for label in ["Span", "First", "Second"] {
        assert_eq!(
            trace.pages[0]
                .iter()
                .filter(|line| line.text == label)
                .count(),
            1
        );
    }
}

#[test]
fn internal_row_boundaries_preserve_repeated_groups_and_collapsed_cell_content() {
    let words = (0..60)
        .map(|index| format!("S{index:03}"))
        .collect::<Vec<_>>();
    for collapse in ["separate", "collapse"] {
        let trace = render(
            &format!("@page{{size:240pt 120pt;margin:10pt}}@page wide{{size:400pt 120pt}}table{{width:100%;table-layout:fixed;border-spacing:0;border-collapse:{collapse}}}td{{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top;border:1pt solid red}}.second{{page:wide}}"),
            &format!("<table><thead><tr><td colspan=2>Header</td></tr></thead><tbody><tr><td rowspan=2>{}</td><td>First</td></tr><tr class=second><td>Second</td></tr></tbody><tfoot><tr><td colspan=2>Footer</td></tr></tfoot></table>", words.join(" ")),
        ).layout.unwrap();
        assert!(trace.pages[0].iter().any(|line| line.text == "First"));
        assert!(
            trace.pages[1].iter().any(|line| line.text == "Second"),
            "{collapse}: internal named row must start on the second page"
        );
        assert_pt(trace.page_records[1].geometry.paper[0], 400.0);
        let actual = trace
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.text.split_whitespace())
            .filter(|word| word.starts_with('S') && *word != "Second")
            .collect::<Vec<_>>();
        assert_eq!(
            actual, words,
            "{collapse}: crossing content must be conserved"
        );
        for (page, lines) in trace.pages.iter().enumerate() {
            if lines.iter().any(|line| line.text.starts_with('S')) {
                for label in ["Header", "Footer"] {
                    assert_eq!(
                        lines.iter().filter(|line| line.text == label).count(),
                        1,
                        "{collapse}: body page {page} must have one {label}; pages {:?}",
                        trace
                            .pages
                            .iter()
                            .map(|lines| lines.iter().map(|line| &line.text).collect::<Vec<_>>())
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
    }
}

#[test]
fn overlapping_rowspans_preserve_each_source_across_successive_named_boundaries() {
    let first = (0..90)
        .map(|index| format!("A{index:03}"))
        .collect::<Vec<_>>();
    let second = (0..45)
        .map(|index| format!("B{index:03}"))
        .collect::<Vec<_>>();
    let trace = render(
        "@page{size:240pt 120pt;margin:10pt}@page wide{size:400pt 120pt}@page third{size:300pt 120pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}.second{page:wide}.third{page:third;break-before:right}",
        &format!("<table><tr><td rowspan=3>{}</td><td rowspan=2>{}</td><td>First</td></tr><tr class=second><td>Second</td></tr><tr class=third><td>New</td><td>Third</td></tr></table><p>After</p>", first.join(" "), second.join(" ")),
    ).layout.unwrap();
    for (page, label, width) in [
        (0, "First", 240.0),
        (1, "Second", 400.0),
        (2, "Third", 300.0),
    ] {
        let pages = trace
            .pages
            .iter()
            .enumerate()
            .filter_map(|(index, lines)| {
                lines.iter().any(|line| line.text == label).then_some(index)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            pages,
            [page],
            "{label} must follow its source row's page boundary"
        );
        assert_pt(trace.page_records[page].geometry.paper[0], width);
    }
    for (prefix, expected) in [('A', first), ('B', second)] {
        let actual = trace
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.text.split_whitespace())
            .filter(|word| word.starts_with(prefix) && *word != "After")
            .collect::<Vec<_>>();
        assert_eq!(
            actual, expected,
            "{prefix}: each crossing span must keep its own source checkpoint"
        );
    }
}

#[test]
fn a_nested_table_yields_before_the_outer_row_changes_page_geometry() {
    let words = (0..60)
        .map(|index| format!("S{index:03}"))
        .collect::<Vec<_>>();
    for nested in ["paragraph", "rows", "rowspan"] {
        let inner = match nested {
            "rows" => words
                .iter()
                .map(|word| format!("<tr><td>{word}</td></tr>"))
                .collect::<String>(),
            "rowspan" => format!(
                "<tr><td rowspan=2>{}</td><td>InnerFirst</td></tr><tr class=inner-next><td>InnerSecond</td></tr>",
                words.join(" ")
            ),
            _ => format!("<tr><td>{}</td></tr>", words.join(" ")),
        };
        let trace = render(
        "@page{size:240pt 120pt;margin:10pt}@page wide{size:400pt 120pt}table{width:100%;table-layout:fixed;border-spacing:0}td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}.second{page:wide}.inner-next{break-before:page}",
        &format!("<table><tr><td rowspan=2><table>{inner}</table></td><td>First</td></tr><tr class=second><td>Second</td></tr></table><p>After</p>"),
    ).layout.unwrap();
        let second = trace
            .pages
            .iter()
            .enumerate()
            .filter_map(|(index, lines)| {
                lines
                    .iter()
                    .any(|line| line.text == "Second")
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            second,
            [1],
            "{nested}: nested cell traversal must not paint future pages before the enclosing row selects their geometry"
        );
        assert_pt(trace.page_records[1].geometry.paper[0], 400.0);
        let actual = trace
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.text.split_whitespace())
            .filter(|word| word.starts_with('S') && *word != "Second")
            .collect::<Vec<_>>();
        assert_eq!(actual, words);
    }
}

#[test]
fn definite_height_in_a_span_yields_before_the_next_named_row() {
    for property in ["height", "min-height"] {
        for (constraint, second_page) in [("page:wide", 1), ("page:wide;break-before:right", 2)] {
            let trace = render(
                &format!("@page{{size:240pt 120pt;margin:10pt}}@page wide{{size:400pt 120pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;vertical-align:top}}.second{{{constraint}}}.tall{{{property}:250pt;background:#ddd}}"),
                "<table><tr><td rowspan=2><div class=tall>Start</div><p>End</p></td><td>First</td></tr><tr class=second><td>Second</td></tr></table><p>After</p>",
            ).layout.unwrap();
            let second = trace
                .pages
                .iter()
                .position(|lines| lines.iter().any(|line| line.text == "Second"));
            assert_eq!(
                second,
                Some(second_page),
                "{property}/{constraint}: a definite-height descendant must yield before selecting the next physical page"
            );
            assert_pt(trace.page_records[second_page].geometry.paper[0], 400.0);
            let end = trace
                .pages
                .iter()
                .position(|lines| lines.iter().any(|line| line.text == "End"));
            assert_eq!(
                end,
                Some(second_page + 1),
                "{property}/{constraint}: blank sheets must not consume the retained height"
            );
            for label in ["Start", "End", "First", "Second", "After"] {
                assert_eq!(
                    trace
                        .pages
                        .iter()
                        .flatten()
                        .filter(|line| line.text == label)
                        .count(),
                    1,
                    "{property}/{constraint}: {label} must survive exactly once"
                );
            }
        }
    }
}

#[test]
fn columns_in_a_span_yield_before_the_next_named_row() {
    let words = (0..120)
        .map(|index| format!("S{index:03}"))
        .collect::<Vec<_>>();
    for variant in ["auto", "balance", "spanner", "forced"] {
        for (constraint, second_page) in [("page:wide", 1), ("page:wide;break-before:right", 2)] {
            let content = match variant {
                "spanner" => format!("<p class=spanner>{}</p>", words.join(" ")),
                "forced" => format!(
                    "<p>{}</p><p class=forced>{}</p>",
                    words[..10].join(" "),
                    words[10..].join(" ")
                ),
                _ => words.join(" "),
            };
            let fill = if variant == "balance" {
                "balance"
            } else {
                "auto"
            };
            let trace = render(
                &format!("@page{{size:240pt 120pt;margin:10pt}}@page wide{{size:400pt 120pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;vertical-align:top}}.second{{{constraint}}}.columns{{column-count:2;column-gap:10pt;column-fill:{fill}}}.spanner{{column-span:all}}.forced{{break-before:page}}"),
                &format!("<table><tr><td rowspan=2><div class=columns>{content}</div></td><td>First</td></tr><tr class=second><td>Second</td></tr></table><p>After</p>"),
            ).layout.unwrap();
            let second = trace
                .pages
                .iter()
                .position(|lines| lines.iter().any(|line| line.text == "Second"));
            assert_eq!(
                second,
                Some(second_page),
                "{variant}/{constraint}: column traversal must yield before selecting the next physical page"
            );
            assert_pt(trace.page_records[second_page].geometry.paper[0], 400.0);

            if second_page == 2 {
                assert!(
                    trace.pages[1].is_empty(),
                    "the side break must leave a blank sheet"
                );
            }
            let actual = trace
                .pages
                .iter()
                .flatten()
                .flat_map(|line| line.text.split_whitespace())
                .filter(|word| word.starts_with('S') && *word != "Second")
                .collect::<Vec<_>>();
            assert_eq!(
                actual, words,
                "{variant}/{constraint}: column continuations must preserve the source stream"
            );
            for label in ["First", "Second", "After"] {
                assert_eq!(
                    trace
                        .pages
                        .iter()
                        .flatten()
                        .filter(|line| line.text == label)
                        .count(),
                    1,
                    "{variant}/{constraint}: {label} must appear once"
                );
            }
        }
    }
}

#[test]
fn columns_resume_text_at_the_named_pages_column_width() {
    let words = (0..180)
        .map(|index| format!("S{index:03}"))
        .collect::<Vec<_>>();
    for (columns, second_x, per_line) in [
        ("column-count:2;column-gap:10pt", 110.0, 3),
        ("column-width:50pt;column-gap:10pt", 10.0 + 200.0 / 3.0, 2),
        ("column-count:2;column-gap:10%", 114.5, 3),
    ] {
        for (direction, wrapper) in [
            ("ltr", "none"),
            ("rtl", "none"),
            ("ltr", "section"),
            ("ltr", "table"),
            ("ltr", "relative"),
        ] {
            let content = match wrapper {
                "section" | "relative" => format!("<section>{}</section>", words.join(" ")),
                "table" => format!("<table><tr><td>{}</td></tr></table>", words.join(" ")),
                _ => words.join(" "),
            };
            let relative = if wrapper == "relative" {
                ".columns{position:relative;left:10%}.columns section{position:relative;left:5pt}"
            } else {
                ""
            };
            let second_x = second_x + if wrapper == "relative" { 24.0 } else { 0.0 };
            let trace = render(
                &format!("@page{{size:240pt 120pt;margin:10pt}}@page wide{{size:400pt 120pt}}@page wide:right{{size:240pt 120pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;vertical-align:top}}.second{{page:wide}}.columns{{{columns};column-fill:auto;direction:{direction};text-align:left}}{relative}"),
                &format!("<table><tr><td rowspan=2><div class=columns>{content}</div></td><td>First</td></tr><tr class=second><td>Second</td></tr></table><p>After</p>"),
            ).layout.unwrap();
            let body = |page: usize| {
                trace.pages[page]
                    .iter()
                    .filter(|line| line.text.starts_with('S') && line.text != "Second")
                    .collect::<Vec<_>>()
            };
            assert!(
                body(0)
                    .iter()
                    .all(|line| line.text.split_whitespace().count() == 1),
                "{columns}/{direction}/{wrapper}: the narrow page fits one token per line"
            );
            assert!(
                body(1)
                    .iter()
                    .any(|line| line.text.split_whitespace().count() == per_line),
                "{columns}/{direction}/{wrapper}: remaining text must rebreak at the destination width: {:?}",
                body(1)
                    .iter()
                    .map(|line| (&line.text, line.x))
                    .collect::<Vec<_>>()
            );
            assert!(
                body(1).iter().any(|line| (line.x - second_x).abs() < 0.01),
                "{columns}/{direction}/{wrapper}: missing destination column at x={second_x}"
            );
            assert!(
                body(2)
                    .iter()
                    .all(|line| line.text.split_whitespace().count() == 1),
                "{columns}/{direction}/{wrapper}: text must narrow again on the next right page"
            );
            let actual = trace
                .pages
                .iter()
                .flatten()
                .flat_map(|line| line.text.split_whitespace())
                .filter(|word| word.starts_with('S') && *word != "Second")
                .collect::<Vec<_>>();
            assert_eq!(
                actual, words,
                "{columns}/{direction}/{wrapper}: reflow must resume each source token exactly once"
            );
        }
    }
}

#[test]
fn changed_column_width_preserves_source_split_around_a_nested_spanner() {
    let words = (0..180)
        .map(|index| format!("S{index:03}"))
        .collect::<Vec<_>>();
    for decoration in ["slice", "clone"] {
        let trace = render(
        &format!("@page{{size:240pt 120pt;margin:10pt}}@page wide{{size:400pt 120pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;vertical-align:top}}.second{{page:wide}}.columns{{column-count:2;column-gap:10pt;column-fill:auto}}.spanner{{column-span:all}}section{{padding:5%;box-decoration-break:{decoration};background:#eee}}"),
        &format!("<table><tr><td rowspan=2><div class=columns><section><p>{}</p><p class=spanner>Heading</p><p>{}</p></section></div></td><td>First</td></tr><tr class=second><td>Second</td></tr></table><p>After</p>", words[..60].join(" "), words[60..].join(" ")),
    ).layout.unwrap();
        let actual = trace
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.text.split_whitespace())
            .filter(|word| word.starts_with('S') && *word != "Second")
            .collect::<Vec<_>>();
        assert_eq!(
            actual, words,
            "resizing a split ancestor must not reconstruct already consumed source around its spanner"
        );
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .filter(|line| line.text == "Heading")
                .count(),
            1
        );
        let resumed = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text.starts_with("S060"))
            .unwrap();
        assert_pt(resumed.x, 14.5);
    }
}

#[test]
fn short_cells_inside_columns_stretch_at_each_destination_width() {
    let words = (0..90)
        .map(|index| format!("S{index:03}"))
        .collect::<Vec<_>>();
    for (columns, wide_count) in [("column-count:2", 2), ("column-width:50pt", 3)] {
        for direction in ["ltr", "rtl"] {
            for alignment in ["top", "middle", "bottom"] {
                for relative in [false, true] {
                    let positioning = if relative {
                        ".columns{position:relative;left:10%}.inner-wrap{position:relative;left:5pt}"
                    } else {
                        ""
                    };
                    let trace = render(
                        &format!("@page{{size:240pt 120pt;margin:10pt}}@page wide{{size:400pt 120pt}}@page wide:right{{size:240pt 120pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;vertical-align:top}}.second{{page:wide}}.columns{{{columns};column-gap:10pt;column-fill:auto;direction:{direction}}}.short{{background:blue;vertical-align:{alignment}}}{positioning}"),
                        &format!("<table><tr><td rowspan=2><div class=columns><section class=inner-wrap><table class=inner><tr><td>{}</td><td class=short>Short</td></tr></table></section></div></td><td>First</td></tr><tr class=second><td>Second</td></tr></table><p>After</p>", words.join(" ")),
                    ).layout.unwrap();
                    assert_eq!(
                        trace
                            .pages
                            .iter()
                            .position(|lines| lines.iter().any(|line| line.text == "Second")),
                        Some(1),
                        "the enclosing named row must select the second physical page"
                    );
                    let mut wide_cells = trace.display_lists[1]
                        .iter()
                        .filter_map(|item| match item {
                            ferri_pdf::DisplayItem::Box(paint)
                                if paint.decoration.background.rgb == [0, 0, 255]
                                    && paint.decoration.background.alpha > 0.0 =>
                            {
                                Some(paint.rect)
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    wide_cells.sort_by(|left, right| left.x.total_cmp(&right.x));
                    assert_eq!(
                        wide_cells.len(),
                        wide_count,
                        "{columns}/{direction}/{alignment}/{relative}: every wide column must paint its short cell: {wide_cells:?}"
                    );
                    let column_width = (190.0 + 10.0) / wide_count as f32 - 10.0;
                    let offset = if relative { 24.0 } else { 0.0 };
                    for (index, rect) in wide_cells.iter().enumerate() {
                        let x = 10.0
                            + offset
                            + index as f32 * (column_width + 10.0)
                            + if direction == "ltr" {
                                column_width / 2.0
                            } else {
                                0.0
                            };
                        assert_pt(rect.x, x);
                        assert_pt(rect.width, column_width / 2.0);
                        assert_pt(rect.y, 10.0);
                        assert_pt(rect.height, 100.0);
                    }
                    let actual = trace
                        .pages
                        .iter()
                        .flatten()
                        .flat_map(|line| line.text.split_whitespace())
                        .filter(|word| {
                            word.starts_with('S') && *word != "Short" && *word != "Second"
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(
                        actual, words,
                        "{columns}/{direction}/{alignment}/{relative}: stretching the short cell must preserve its long sibling's source"
                    );
                    assert_eq!(
                        trace
                            .pages
                            .iter()
                            .flatten()
                            .filter(|line| line.text == "Short")
                            .count(),
                        1,
                        "the short sibling's text must appear once"
                    );
                }
            }
        }
    }
}

#[test]
fn margins_after_a_split_column_ancestor_use_the_destination_width() {
    let words = (0..40)
        .map(|index| format!("S{index:03}"))
        .collect::<Vec<_>>();
    for (margins, gap) in [
        ("section{margin-bottom:10%}", 19.0),
        ("section{margin-bottom:5pt}.tail{margin-bottom:10%}", 19.0),
        ("section{margin-bottom:10%}.tail{margin-bottom:7pt}", 19.0),
        ("section{width:50%}.tail{margin-bottom:20%}", 19.0),
        ("section{margin-bottom:calc(10% - 2pt)}", 17.0),
        ("section{margin-bottom:-10%}.tail{margin-bottom:2pt}", 3.0),
        ("section{margin-bottom:10%}.tail{margin-bottom:-2pt}", 17.0),
    ] {
        let words = if margins.contains("width:50%") {
            &words[..20]
        } else {
            &words[..]
        };
        for direction in ["ltr", "rtl"] {
            let trace = render(
                &format!("@page{{size:240pt 120pt;margin:10pt}}@page wide{{size:400pt 120pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;vertical-align:top}}.second{{page:wide}}.columns{{column-count:2;column-gap:10pt;column-fill:auto;direction:{direction}}}.spanner{{column-span:all}}{margins}"),
                &format!("<table><tr><td rowspan=2><div class=columns><section><p>{}</p><p class=spanner>Heading</p><p class=tail>Tail</p></section><p>Following</p></div></td><td>First</td></tr><tr class=second><td>Second</td></tr></table>", words.join(" ")),
            ).layout.unwrap();
            let tail = trace.pages[1]
                .iter()
                .find(|line| line.text == "Tail")
                .unwrap_or_else(|| {
                    panic!("{direction}/{margins}: tail must resume on the wide page")
                });
            let following = trace.pages[1]
                .iter()
                .find(|line| line.text == "Following")
                .unwrap_or_else(|| {
                    panic!("{direction}/{margins}: following sibling must fit on the wide page")
                });
            assert!(
                (following.baseline - tail.baseline - gap).abs() < 0.001,
                "{direction}/{margins}: expected gap{gap}, got {}",
                following.baseline - tail.baseline
            );
            let actual = trace
                .pages
                .iter()
                .flatten()
                .flat_map(|line| line.text.split_whitespace())
                .filter(|word| word.starts_with('S') && *word != "Second")
                .collect::<Vec<_>>();
            assert_eq!(actual, words, "margin resolution must not replay source");
        }
    }
}

#[test]
fn forced_break_margins_resolve_after_the_destination_ancestors_reflow() {
    for (width, expected) in [("100%", 38.0), ("50%", 19.0)] {
        let trace = render(
            &format!("@page{{size:240pt 120pt;margin:10pt}}@page:left{{size:400pt 120pt}}section{{width:{width}}}.before{{margin-bottom:50%}}.next{{break-before:page;margin-top:10%}}"),
            "<section><p class=before>Before</p><p class=next>Next</p></section>",
        ).layout.unwrap();
        assert_eq!(trace.pages.len(), 2);
        assert_pt(
            trace.pages[1][0].baseline - trace.pages[0][0].baseline,
            expected,
        );
    }
}

#[test]
fn long_runs_of_collapsed_percentage_margins_preserve_flow_and_release_sources() {
    const EMPTY_BLOCKS: usize = 32_768;
    let css = "@page{size:240pt 120pt;margin:10pt}div{margin-top:1%}";
    let reference = render(css, "<p>Before</p><div></div><p>After</p>")
        .layout
        .unwrap();
    let trace = render(
        css,
        &format!(
            "<p>Before</p>{}<p>After</p>",
            "<div></div>".repeat(EMPTY_BLOCKS)
        ),
    )
    .layout
    .unwrap();
    assert_eq!(
        trace.pages.len(),
        1,
        "collapsed empty siblings must not accumulate their margins"
    );
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Before", "After"]
    );
    assert_pt(trace.pages[0][1].baseline, reference.pages[0][1].baseline);
    assert_pt(
        trace.pages[0][1].baseline - trace.pages[0][0].baseline,
        12.2,
    );
}

fn spanner_height_trace(constraints: &str, decoration: &str) -> ferri_pdf::LayoutTrace {
    let trace = render(
        &format!("@page{{size:240pt 120pt;margin:10pt}}@page wide{{size:400pt 120pt}}table{{width:100%;table-layout:fixed;border-spacing:0}}td{{padding:0;font:10pt/10pt Fixed;vertical-align:top}}.second{{page:wide}}.columns{{column-count:2;column-gap:10pt;column-fill:auto}}.lead{{line-height:80pt}}.spanner{{column-span:all;line-height:20pt}}section{{{constraints};padding:20% 0;background:blue;box-decoration-break:{decoration}}}"),
        "<table><tr><td rowspan=2><div class=columns><p class=lead>Lead</p><section><p class=spanner>Heading</p><p>Tail</p></section><p>Following</p></div></td><td>First</td></tr><tr class=second><td>Second</td></tr></table>",
    ).layout.unwrap();
    for label in ["Lead", "Heading", "Tail", "Following", "First", "Second"] {
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .filter(|line| line.text == label)
                .count(),
            1,
            "{constraints}/{decoration}: {label} must appear once"
        );
    }
    trace
}

#[test]
fn split_column_ancestors_recalculate_height_constraints_at_destination_widths() {
    for (constraints, column, extent) in [
        ("height:100pt;box-sizing:border-box", 0, 90.0),
        ("min-height:100pt;box-sizing:border-box", 0, 90.0),
        (
            "height:100pt;box-sizing:content-box",
            1,
            136.0 * (1.0 - 10.0 / 120.0) - 100.0,
        ),
        (
            "max-height:40pt;box-sizing:border-box",
            0,
            40.0 * (1.0 - 10.0 / 30.0),
        ),
        (
            "min-height:100pt;max-height:80pt;box-sizing:border-box",
            0,
            90.0,
        ),
        (
            "height:50pt;min-height:100pt;box-sizing:border-box",
            0,
            90.0,
        ),
        ("max-height:60pt;box-sizing:border-box", 0, 28.0),
        ("height:auto", 0, 28.0),
    ] {
        let trace = spanner_height_trace(constraints, "slice");
        let following = trace.pages[1]
            .iter()
            .find(|line| line.text == "Following")
            .unwrap_or_else(|| {
                panic!("{constraints}: following source must resume on the wide page")
            });
        let tail = trace.pages[1]
            .iter()
            .find(|line| line.text == "Tail")
            .expect("Tail belongs to the wide page");
        assert!(
            (following.x - (10.0 + column as f32 * 100.0)).abs() < 0.001,
            "{constraints}: Following in wrong column at{}",
            following.x
        );
        assert!(
            (following.baseline - tail.baseline - extent).abs() < 0.001,
            "{constraints}: expected remaining extent{extent}, got{}",
            following.baseline - tail.baseline
        );
    }
}

#[test]
fn cloned_spanner_ancestor_edges_are_additional_to_its_declared_extent() {
    for (box_sizing, normal_height) in [("border-box", 100.0), ("content-box", 120.0)] {
        for decoration in ["padding:10pt", "padding:9pt;border:1pt solid red"] {
            for spanners in [1, 2] {
                for column_sizing in [
                    "column-fill:auto",
                    "column-fill:auto;height:200pt",
                    "column-fill:balance;height:200pt",
                ] {
                    let height = if column_sizing.contains("height:200pt") {
                        "50%"
                    } else {
                        "100pt"
                    };
                    let middle = if spanners == 2 {
                        "<p class=spanner>MiddleHeading</p><p>Middle</p>"
                    } else {
                        ""
                    };
                    let trace = render(
                        &format!("@page{{size:300pt 400pt;margin:10pt}}.columns{{column-count:2;column-gap:10pt;{column_sizing};background:lime}}section{{height:{height};box-sizing:{box_sizing};{decoration};background:blue;box-decoration-break:clone}}.spanner{{column-span:all}}"),
                        &format!("<div class=columns><section><p>Before</p><p class=spanner>Heading</p>{middle}<p>Tail</p></section><p>Following</p></div>"),
                    ).layout.unwrap();
                    let fragments = trace
                        .display_lists
                        .iter()
                        .flatten()
                        .filter_map(|item| match item {
                            ferri_pdf::DisplayItem::Box(paint)
                                if paint.decoration.background.rgb == [0, 0, 255]
                                    && paint.decoration.background.alpha > 0.0 =>
                            {
                                Some(paint)
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    assert!(
                        fragments.len() > spanners,
                        "each spanner must split its ancestor's decoration"
                    );
                    let total: f32 = fragments.iter().map(|paint| paint.rect.height).sum();
                    let expected = normal_height + (fragments.len() - 1) as f32 * 20.0;
                    assert!(
                        (total - expected).abs() < 0.01,
                        "{box_sizing}/{decoration}/{spanners}: {}fragments paint{total}pt, expected{expected}pt including additional cloned edges",
                        fragments.len()
                    );
                    assert!(
                        fragments
                            .iter()
                            .all(|paint| paint.first_fragment && paint.last_fragment)
                    );
                    let mut labels = vec!["Before", "Heading", "Tail", "Following"];
                    if spanners == 2 {
                        labels.extend(["MiddleHeading", "Middle"]);
                    }
                    for label in labels {
                        assert_eq!(
                            trace
                                .pages
                                .iter()
                                .flatten()
                                .filter(|line| line.text == label)
                                .count(),
                            1,
                            "{label} must appear once"
                        );
                    }
                    if column_sizing.contains("height:200pt") {
                        let container = trace
                            .display_lists
                            .iter()
                            .flatten()
                            .find_map(|item| match item {
                                ferri_pdf::DisplayItem::Box(paint)
                                    if paint.decoration.background.rgb == [0, 255, 0]
                                        && paint.decoration.background.alpha > 0.0 =>
                                {
                                    Some(paint.rect)
                                }
                                _ => None,
                            })
                            .expect("the column container must paint its declared height");
                        assert_pt(container.height, 200.0);
                    }
                }
            }
        }
    }
}

#[test]
fn cloned_spanner_edges_recalculate_at_each_destination_width() {
    const CLONED_BORDER_GAP_PT: f32 = 90.0 + 2.0 * 18.0 - 100.0;
    for (constraints, column, gap) in [
        (
            "height:100pt;box-sizing:border-box",
            1,
            CLONED_BORDER_GAP_PT,
        ),
        (
            "min-height:100pt;box-sizing:border-box",
            1,
            CLONED_BORDER_GAP_PT,
        ),
        (
            "height:100pt;box-sizing:content-box",
            1,
            136.0 * (1.0 - 10.0 / 120.0) + 2.0 * 18.0 - 100.0,
        ),
        (
            "max-height:40pt;box-sizing:border-box",
            0,
            40.0 * (1.0 - 10.0 / 30.0),
        ),
        (
            "min-height:100pt;max-height:80pt;box-sizing:border-box",
            1,
            CLONED_BORDER_GAP_PT,
        ),
        (
            "height:50pt;min-height:100pt;box-sizing:border-box",
            1,
            CLONED_BORDER_GAP_PT,
        ),
        ("max-height:60pt;box-sizing:border-box", 0, 28.0),
        ("height:auto", 0, 28.0),
    ] {
        let trace = spanner_height_trace(constraints, "clone");
        let following = trace.pages[1]
            .iter()
            .find(|line| line.text == "Following")
            .unwrap_or_else(|| panic!("{constraints}: following source must fit on the wide page"));
        let tail = trace.pages[1]
            .iter()
            .find(|line| line.text == "Tail")
            .expect("Tail belongs to the wide page");
        assert!(
            (following.x - (10.0 + column as f32 * 100.0)).abs() < 0.001,
            "{constraints}: Following in wrong column at{}",
            following.x
        );
        assert!(
            (following.baseline - tail.baseline - gap).abs() < 0.001,
            "{constraints}: expected gap{gap}, got{}",
            following.baseline - tail.baseline
        );
    }
}

#[test]
fn active_cloned_edges_use_each_pages_width_while_height_progress_is_retained() {
    for (name, nested, flowed, pages, gap) in [
        ("plain", false, false, 5, 68.0),
        ("flowed", false, true, 5, 68.0),
        ("nested", true, false, 8, 52.0),
    ] {
        let labels = if flowed {
            (1..=19)
                .map(|index| format!("Line{index:02}"))
                .collect::<Vec<_>>()
        } else {
            vec!["Top".to_owned()]
        };
        let body = format!("<div class=box>{}</div><p>After</p>", labels.join("<br>"));
        let body = if nested {
            format!("<div class=outer>{body}</div>")
        } else {
            body
        };
        let height = if nested { 270 } else { 250 };
        let trace = render(
            &format!("@page{{size:240pt 120pt;margin:10pt}}@page:left{{size:400pt 120pt}}.box{{height:{height}pt;box-sizing:border-box;padding:10% 0;background:blue;box-decoration-break:clone}}.outer{{padding:2% 0;box-decoration-break:clone}}"),
            &body,
        ).layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            pages,
            "{name}: source must retain the declared height"
        );
        let after = trace.pages[pages - 1]
            .iter()
            .find(|line| line.text == "After")
            .expect("After belongs to the final page");
        assert!(
            (after.baseline - trace.pages[0][0].baseline - gap).abs() < 0.001,
            "{name}: expected gap {gap}, got {}",
            after.baseline - trace.pages[0][0].baseline
        );
        for page in 0..pages {
            let paints = trace.display_lists[page]
                .iter()
                .filter_map(|item| match item {
                    ferri_pdf::DisplayItem::Box(paint)
                        if paint.decoration.background.rgb == [0, 0, 255]
                            && paint.decoration.background.alpha > 0.0 =>
                    {
                        Some(paint)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                paints.len(),
                1,
                "{name}/page{} must contain exactly one active clone fragment",
                page + 1
            );
            let expected = if nested {
                if page + 1 == pages {
                    70.8
                } else if page % 2 == 0 {
                    91.2
                } else {
                    84.8
                }
            } else if page + 1 == pages {
                90.0
            } else {
                100.0
            };
            assert!(
                (paints[0].rect.height - expected).abs() < 0.001,
                "{name}/page{}: expected painted height{expected}, got{}",
                page + 1,
                paints[0].rect.height
            );
            assert!(paints[0].first_fragment && paints[0].last_fragment);
        }
        let actual = trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .filter(|text| *text != "After")
            .collect::<Vec<_>>();
        assert_eq!(
            actual, labels,
            "{name}: source lines must appear exactly once"
        );
    }
}

#[test]
fn active_height_constraints_recalculate_without_losing_consumed_progress() {
    for (decoration, constraints, pages, final_height, gap) in [
        ("clone", "height:250pt", 7, 9954.0 / 163.0, 6368.0 / 163.0),
        (
            "clone",
            "min-height:250pt",
            7,
            9954.0 / 163.0,
            6368.0 / 163.0,
        ),
        (
            "clone",
            "height:500pt;max-height:250pt",
            7,
            9954.0 / 163.0,
            6368.0 / 163.0,
        ),
        (
            "clone",
            "height:100pt;min-height:250pt;max-height:200pt",
            7,
            9954.0 / 163.0,
            6368.0 / 163.0,
        ),
        ("slice", "height:330pt", 4, 16622.0 / 187.0, 12508.0 / 187.0),
    ] {
        let trace = render(
            &format!("@page{{size:240pt 120pt;margin:10pt}}@page:left{{size:400pt 120pt}}.box{{{constraints};box-sizing:content-box;padding:10% 0;background:blue;box-decoration-break:{decoration}}}"),
            "<div class=box>Top</div><p>After</p>",
        ).layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            pages,
            "{decoration}/{constraints}: wrong continuation page"
        );
        let after = trace.pages[pages - 1]
            .iter()
            .find(|line| line.text == "After")
            .expect("After belongs to the final page");
        assert!(
            (after.baseline - trace.pages[0][0].baseline - gap).abs() < 0.001,
            "{decoration}/{constraints}: expected baseline gap {gap}, got {}",
            after.baseline - trace.pages[0][0].baseline
        );
        let paint = trace.display_lists[pages - 1]
            .iter()
            .find_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255] =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .expect("final page retains the active background");
        assert!(
            (paint.rect.height - final_height).abs() < 0.001,
            "{decoration}/{constraints}: expected final painted extent {final_height}, got {}",
            paint.rect.height
        );
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Top", "After"]
        );
    }
}

#[test]
fn active_percentage_heights_follow_the_destination_containing_block() {
    for factor in [1, 2] {
        let parent = format!("html,body{{height:100%}}.outer{{height:{}%}}", 100 / factor);
        for constraints in [
            format!("height:{}%", 250 * factor),
            format!("min-height:{}%", 250 * factor),
            format!("height:{}%;max-height:{}%", 500 * factor, 250 * factor),
            format!(
                "height:{}%;min-height:{}%;max-height:{}%",
                100 * factor,
                250 * factor,
                200 * factor
            ),
        ] {
            let trace = render(
                &format!("@page{{size:240pt 120pt;margin:10pt}}@page:left{{size:400pt 180pt}}{parent}.box{{{constraints};background:blue}}"),
                "<div class=outer><div class=box>Top</div><p>After</p></div>",
            ).layout.unwrap();
            assert_eq!(
                trace.pages.len(),
                3,
                "{parent}/{constraints}: wrong continuation page"
            );
            let after = trace.pages[2]
                .iter()
                .find(|line| line.text == "After")
                .expect("After belongs on page 3");
            assert!(
                (after.baseline - trace.pages[0][0].baseline - 50.0).abs() < 0.001,
                "{parent}/{constraints}: expected baseline gap 50, got {}",
                after.baseline - trace.pages[0][0].baseline
            );
            let paint = trace.display_lists[2]
                .iter()
                .find_map(|item| match item {
                    ferri_pdf::DisplayItem::Box(paint)
                        if paint.decoration.background.rgb == [0, 0, 255] =>
                    {
                        Some(paint)
                    }
                    _ => None,
                })
                .expect("page 3 retains the active background");
            assert_pt(paint.rect.height, 50.0);
            assert_eq!(
                trace
                    .pages
                    .iter()
                    .flatten()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>(),
                ["Top", "After"]
            );
        }
    }
}

#[test]
fn initially_inactive_height_constraints_can_bind_after_a_page_change() {
    let labels = (1..=20)
        .map(|index| format!("Line{index:02}"))
        .collect::<Vec<_>>();
    for (constraint, left_height, pages, extent) in [
        ("min-height:150%", 180, 2, 120.0),
        ("max-height:250%", 80, 3, 20.0),
    ] {
        let trace = render(
            &format!("@page{{size:240pt 120pt;margin:10pt}}@page:left{{size:400pt {left_height}pt}}html,body,.outer{{height:100%}}.box{{{constraint};background:blue}}"),
            &format!("<div class=outer><div class=box>{}</div><p>After</p></div>",labels.join("<br>")),
        ).layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            pages,
            "{constraint}: wrong physical page count"
        );
        let after = trace.pages[pages - 1]
            .iter()
            .find(|line| line.text == "After")
            .expect("After belongs to the final page");
        assert!(
            (after.baseline - trace.pages[0][0].baseline - extent).abs() < 0.001,
            "{constraint}: expected baseline gap {extent}, got {}",
            after.baseline - trace.pages[0][0].baseline
        );
        let paint = trace.display_lists[pages - 1]
            .iter()
            .find_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255] =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .expect("final fragment must retain its background");
        assert_pt(paint.rect.height, extent);
        let mut expected = labels.clone();
        expected.push("After".to_owned());
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn active_percentage_padding_uses_its_destination_containing_width() {
    for (half, decoration, count, pages, gap, final_height) in [
        (false, "slice", 9, 2, 37.0, 48.0),
        (false, "slice", 10, 2, 47.0, 58.0),
        (false, "clone", 9, 2, 76.0, 87.0),
        (false, "clone", 10, 3, -11.0, 97.0),
        (true, "slice", 9, 2, 23.5, 29.0),
        (true, "slice", 10, 2, 23.5, 29.0),
        (true, "clone", 9, 2, 33.0, 38.5),
        (true, "clone", 10, 2, 43.0, 48.5),
    ] {
        let labels = (1..=count)
            .map(|index| format!("Line{index:02}"))
            .collect::<Vec<_>>();
        let outer = if half { ".outer{width:50%}" } else { "" };
        let trace = render(
            &format!("@page{{size:240pt 120pt;margin:10pt}}@page:left{{size:400pt 120pt}}{outer}.box{{padding:5% 0 10%;background:blue;box-decoration-break:{decoration}}}"),
            &format!("<div class=outer><div class=box>{}</div><p>After</p></div>",labels.join("<br>")),
        ).layout.unwrap();
        let context = format!("half={half}/{decoration}/lines={count}");
        assert_eq!(
            trace.pages.len(),
            pages,
            "{context}: wrong physical page count"
        );
        let after = trace.pages[pages - 1]
            .iter()
            .find(|line| line.text == "After")
            .expect("After belongs to the final page");
        assert!(
            (after.baseline - trace.pages[0][0].baseline - gap).abs() < 0.001,
            "{context}: expected baseline gap {gap}, got {}",
            after.baseline - trace.pages[0][0].baseline
        );
        let paint = trace.display_lists[1]
            .iter()
            .find_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255] =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .expect("page 2 retains the final background");
        assert!(
            (paint.rect.height - final_height).abs() < 0.001,
            "{context}: expected final painted height {final_height}, got {}",
            paint.rect.height
        );
        let mut expected = labels;
        expected.push("After".to_owned());
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            expected,
            "{context}: source lines must appear exactly once"
        );
    }
}

#[test]
fn active_table_padding_uses_the_same_destination_edges_as_its_rows() {
    for (decoration, count, pages, gap) in [
        ("slice", 9, 2, 37.0),
        ("slice", 10, 2, 47.0),
        ("clone", 9, 2, 76.0),
        ("clone", 10, 3, -11.0),
    ] {
        let labels = (1..=count)
            .map(|index| format!("Line{index:02}"))
            .collect::<Vec<_>>();
        let rows = labels
            .iter()
            .map(|label| format!("<tr><td>{label}</td></tr>"))
            .collect::<String>();
        let trace=render(
            &format!("@page{{size:240pt 120pt;margin:10pt}}@page:left{{size:400pt 120pt}}table{{width:100%;table-layout:fixed;border-spacing:0;padding:5% 0 10%;box-decoration-break:{decoration};background:blue}}td{{margin:0;padding:0;font:10pt/10pt Fixed;vertical-align:top}}"),
            &format!("<table>{rows}</table><p>After</p>"),
        ).layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            pages,
            "{decoration}/{count}: wrong physical page count"
        );
        if decoration == "slice" {
            assert_eq!(
                trace.pages[0]
                    .iter()
                    .filter(|line| line.text.starts_with("Line"))
                    .count(),
                8,
                "11pt top padding leaves 89pt for eight 10pt rows; sliced ends add no intermediate reserve"
            );
        }
        let after = trace.pages[pages - 1]
            .iter()
            .find(|line| line.text == "After")
            .expect("After belongs on the final page");
        assert!(
            (after.baseline - trace.pages[0][0].baseline - gap).abs() < 0.001,
            "{decoration}/{count}: expected gap {gap}, got {}",
            after.baseline - trace.pages[0][0].baseline
        );
        let mut expected = labels;
        expected.push("After".to_owned());
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
}

fn caption_entry_trace(
    side: &str,
    direction: &str,
    half: bool,
    boundary: &str,
    inner: bool,
) -> ferri_pdf::LayoutTrace {
    let width = if half { "width:50%;" } else { "" };
    let forced = if boundary == "before" {
        ".changed{break-before:page}"
    } else {
        ".lead{break-after:page}"
    };
    let content = if inner {
        "<p class=inner>Caption</p>"
    } else {
        "Caption"
    };
    render(
        &format!("@page{{size:240pt 120pt;margin:10pt}}@page:left{{size:400pt 120pt}}table{{width:50%;table-layout:fixed;border-spacing:0;direction:{direction}}}caption,td{{margin:0;padding:0;font:10pt/10pt Fixed;text-align:left;widows:1;orphans:1}}.lead{{margin-bottom:30pt}}.changed{{{width}caption-side:{side};margin:10% 0;background:blue}}.inner{{margin:5pt 0 7pt}}{forced}"),
        &format!("<table><caption class=lead>Before</caption><caption class=changed>{content}</caption><tr><td>Cell</td></tr></table><p>After</p>"),
    ).layout.unwrap()
}

#[test]
fn caption_entries_resolve_margins_and_width_outside_the_active_grid() {
    for side in ["top", "bottom"] {
        for direction in ["ltr", "rtl"] {
            for half in [false, true] {
                for boundary in if side == "top" {
                    &["before", "after"][..]
                } else {
                    &["before"][..]
                } {
                    let trace = caption_entry_trace(side, direction, half, boundary, false);
                    let context = format!("{side}/{direction}/half={half}/boundary={boundary}");
                    assert_eq!(trace.pages.len(), 2, "{context}: wrong page count");
                    let caption = trace.pages[1]
                        .iter()
                        .find(|line| line.text == "Caption")
                        .expect("Caption belongs on page2");
                    assert!(
                        (caption.baseline - trace.pages[0][0].baseline - 19.0).abs() < 0.001,
                        "{context}: caption must retain its destination19pt margin; got gap{}",
                        caption.baseline - trace.pages[0][0].baseline
                    );
                    assert_pt(
                        caption.x,
                        if half && direction == "rtl" {
                            105.0
                        } else {
                            10.0
                        },
                    );
                    let after = trace.pages[1]
                        .iter()
                        .find(|line| line.text == "After")
                        .expect("After belongs on page2");
                    let gap = if side == "top" { 58.0 } else { 48.0 };
                    assert!(
                        (after.baseline - trace.pages[0][0].baseline - gap).abs() < 0.001,
                        "{context}: expectedAfter gap{gap}, got{}",
                        after.baseline - trace.pages[0][0].baseline
                    );
                    let paint = trace.display_lists[1]
                        .iter()
                        .find_map(|item| match item {
                            ferri_pdf::DisplayItem::Box(paint)
                                if paint.decoration.background.rgb == [0, 0, 255] =>
                            {
                                Some(paint)
                            }
                            _ => None,
                        })
                        .expect("caption must paint on destinationpage");
                    assert_pt(paint.rect.width, if half { 95.0 } else { 190.0 });
                    assert_pt(paint.rect.y, 29.0);
                    let expected = if side == "top" {
                        vec!["Before", "Caption", "Cell", "After"]
                    } else {
                        vec!["Before", "Cell", "Caption", "After"]
                    };
                    assert_eq!(
                        trace
                            .pages
                            .iter()
                            .flatten()
                            .map(|line| line.text.as_str())
                            .collect::<Vec<_>>(),
                        expected,
                        "{context}: source order and single consumption"
                    );
                }
            }
        }
    }
}

#[test]
fn active_captions_reflow_while_their_table_grid_is_inactive() {
    let labels = (0..60)
        .map(|index| format!("S{index:03}"))
        .collect::<Vec<_>>();
    for (side, first_lines, final_height) in [("top", 10, 50.0), ("bottom", 9, 60.0)] {
        let trace=render(
            &format!("@page{{size:240pt 120pt;margin:10pt}}@page:left{{size:400pt 120pt}}table{{width:50%;table-layout:fixed;border-spacing:0}}caption,td{{margin:0;padding:0;font:10pt/10pt Fixed;text-align:left;widows:1;orphans:1}}caption{{caption-side:{side};background:blue;break-inside:auto}}td{{background:lime}}"),
            &format!("<table><caption>{}</caption><tr><td>Cell</td></tr></table><p>After</p>",labels.join(" ")),
        ).layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            2,
            "{side}: active caption must reflow into its190pt destination"
        );
        let counts = trace.pages[0]
            .iter()
            .filter(|line| line.text.starts_with('S'))
            .map(|line| line.text.split_whitespace().count())
            .collect::<Vec<_>>();
        assert_eq!(
            counts,
            vec![3; first_lines],
            "{side}: source-line progress on narrowpage"
        );
        let counts = trace.pages[1]
            .iter()
            .filter(|line| line.text.starts_with('S'))
            .map(|line| line.text.split_whitespace().count())
            .collect::<Vec<_>>();
        assert_eq!(
            counts,
            if side == "top" {
                vec![6; 5]
            } else {
                vec![6, 6, 6, 6, 6, 3]
            },
            "{side}: reflowed line progress on widepage"
        );
        let actual = trace
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.text.split_whitespace())
            .filter(|word| word.starts_with('S'))
            .collect::<Vec<_>>();
        assert_eq!(
            actual, labels,
            "{side}: caption tokens must be consumed once"
        );
        let after = trace.pages[1]
            .iter()
            .find(|line| line.text == "After")
            .expect("After follows the grid onpage2");
        assert!(
            (after.baseline - trace.pages[0][0].baseline - 60.0).abs() < 0.001,
            "{side}: expectedAfter gap60, got{}",
            after.baseline - trace.pages[0][0].baseline
        );
        let paint = trace.display_lists[1]
            .iter()
            .find_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255] =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .expect("active caption background continues");
        assert_pt(paint.rect.width, 190.0);
        assert_pt(paint.rect.height, final_height);
        let grid_page = if side == "top" { 1 } else { 0 };
        let grid = trace.display_lists[grid_page]
            .iter()
            .find_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 255, 0] =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .expect("grid keeps its selected width");
        assert_pt(grid.rect.width, if grid_page == 1 { 190.0 } else { 110.0 });
        let expected_cell_page = if side == "top" { 2 } else { 1 };
        let cell_page = trace
            .pages
            .iter()
            .position(|page| page.iter().any(|line| line.text == "Cell"))
            .unwrap()
            + 1;
        assert_eq!(cell_page, expected_cell_page);
    }
}

#[test]
fn remeasured_captions_keep_their_formatting_context_at_destination_entry() {
    for (side, after_gap) in [("top", 70.0), ("bottom", 60.0)] {
        let trace = caption_entry_trace(side, "ltr", false, "before", true);
        assert_eq!(trace.pages.len(), 2);
        for (label, gap) in [("Caption", 24.0), ("After", after_gap)] {
            let line = trace.pages[1]
                .iter()
                .find(|line| line.text == label)
                .expect("label belongs onpage2");
            assert!(
                (line.baseline - trace.pages[0][0].baseline - gap).abs() < 0.001,
                "{side}/{label}: expected gap{gap}, got{}",
                line.baseline - trace.pages[0][0].baseline
            );
        }
        let paint = trace.display_lists[1]
            .iter()
            .find_map(|item| match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255] =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .expect("caption retains internal margins in its background");
        assert_pt(paint.rect.y, 29.0);
        assert_pt(paint.rect.height, 22.0);
    }
}
