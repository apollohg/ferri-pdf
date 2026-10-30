mod common;
#[path = "common/weasy_reference.rs"]
mod weasy_reference;
use common::{PROFILE, assert_pt, local_options};
use ferri_pdf::{DisplayItem, TextLine, render_html};
use weasy_reference::{assert_weasy_lines, render_weasy_case};

const GEOMETRY_TOLERANCE_PT: f32 = 0.001;
const HTML_CELL_PADDING_PT: f32 = 0.75;

#[test]
fn row_boundary_keeps_and_forces_end_spanning_alignment_before_the_footer() {
    const ROW_HEIGHT: f32 = 22.5;
    for boundary in [".next{break-after:avoid}", ".next{break-before:page}"] {
        for alignment in ["middle", "bottom"] {
            let html = format!(
                "<!doctype html><style>{PROFILE}@page{{size:240pt 140pt;margin:10pt}}body{{font:10pt/14pt Spike}}p{{margin:0;line-height:22.5pt}}table{{border-collapse:collapse;width:100%}}td,th{{padding:4pt;border:.5pt solid;text-align:left;font-weight:normal;vertical-align:{alignment}}}tr{{break-inside:avoid}}{boundary}</style><p>Lead</p><table><thead><tr><th colspan=2>H</th></tr></thead><tbody><tr><td rowspan=3>S</td><td>A</td></tr><tr class=next><td>B</td></tr><tr><td>C</td></tr></tbody><tfoot><tr><td colspan=2>F</td></tr></tfoot></table>"
            );
            let trace = render_html(&html, &local_options())
                .unwrap()
                .layout
                .unwrap();
            assert_eq!(trace.pages.len(), 2, "{boundary}, {alignment}");
            let baseline = |text: &str| {
                trace.pages[0]
                    .iter()
                    .find(|line| line.text == text)
                    .unwrap_or_else(|| {
                        panic!("{boundary}, {alignment}: missing {text} on first page")
                    })
                    .baseline
            };
            assert_pt(baseline("S"), baseline("A"));
            assert_pt(baseline("F") - baseline("A"), ROW_HEIGHT);
            for text in ["B", "C"] {
                assert!(
                    trace.pages[1].iter().any(|line| line.text == text),
                    "{boundary}, {alignment}: missing {text} on continuation"
                );
            }
        }
    }
}

#[test]
fn kept_rowspan_fragments_align_with_rows_and_leave_the_gap_after_the_footer() {
    const ROW_HEIGHT: f32 = 22.5;
    for alignment in ["top", "middle", "bottom"] {
        let rows = (1..=4)
            .map(|index| format!("<tr><td rowspan=2>S{index}</td><td>A{index}</td></tr><tr><td>B{index}</td></tr>"))
            .collect::<String>();
        let html = format!(
            "<!doctype html><style>{PROFILE}@page{{size:240pt 140pt;margin:10pt}}body{{font:10pt/14pt Spike}}table{{border-collapse:collapse;width:100%}}td,th{{padding:4pt;border:.5pt solid;text-align:left;font-weight:normal;vertical-align:{alignment}}}tr{{break-inside:avoid}}</style><table><thead><tr><th colspan=2>H</th></tr></thead><tbody>{rows}</tbody><tfoot><tr><td colspan=2>F</td></tr></tfoot></table>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            trace.pages.len(),
            3,
            "{alignment}: the kept rows occupy three pages"
        );
        let baseline = |page: usize, text: &str| {
            trace.pages[page]
                .iter()
                .find(|line| line.text == text)
                .unwrap_or_else(|| panic!("{alignment}: missing {text} on page {}", page + 1))
                .baseline
        };
        assert_pt(baseline(0, "S2"), baseline(0, "A2"));
        for (page, last) in [(0, "A2"), (1, "B3"), (2, "B4")] {
            assert_pt(baseline(page, "F") - baseline(page, last), ROW_HEIGHT);
        }
    }
}

#[test]
fn row_avoidance_does_not_keep_connected_rowspan_rows_together() {
    let rows = (1..=4)
        .map(|index| format!("<tr><td rowspan=2>Group {index}</td><td>A{index}</td></tr><tr><td>B{index}</td></tr>"))
        .collect::<String>();
    let html = format!(
        "<!doctype html><style>{PROFILE}@page{{size:240pt 120pt;margin:10pt}}body{{font:10pt/20pt Spike}}table{{width:100%;border-collapse:collapse}}td,th{{padding:0;text-align:left;font-weight:normal}}tr{{break-inside:avoid}}</style><table><thead><tr><th colspan=2>Header</th></tr></thead><tbody>{rows}</tbody><tfoot><tr><td colspan=2>Footer</td></tr></tfoot></table>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages.len(),
        3,
        "Individual row keeps must allow sibling-row breaks: {:?}",
        trace
            .pages
            .iter()
            .map(|page| page
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
    let label = trace.pages[0]
        .iter()
        .find(|line| line.text == "Group 2")
        .expect("A spanning label that fits must remain in the first fragment");
    let last_row = trace.pages[0]
        .iter()
        .find(|line| line.text == "A2")
        .unwrap();
    assert_pt(label.baseline, last_row.baseline);
    for (page, expected) in [
        vec!["A1", "B1", "A2"],
        vec!["B2", "A3", "B3"],
        vec!["A4", "B4"],
    ]
    .iter()
    .enumerate()
    {
        let actual = trace.pages[page]
            .iter()
            .filter(|line| line.text.starts_with(['A', 'B']))
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(&actual, expected, "Row placement on page {}", page + 1);
        for repeated in ["Header", "Footer"] {
            assert_eq!(
                trace.pages[page]
                    .iter()
                    .filter(|line| line.text == repeated)
                    .count(),
                1,
                "Repeated {repeated} on page {}",
                page + 1
            );
        }
    }
}

#[test]
fn avoidance_keeps_multiline_rows_inside_a_fragmented_rowspan_band() {
    let html = format!(
        "<!doctype html><style>{PROFILE}@page{{size:240pt 120pt;margin:10pt}}body{{font:10pt/20pt Spike}}p{{margin:0}}table{{width:100%;border-collapse:collapse}}td,th{{padding:0;text-align:left;font-weight:normal;vertical-align:top;orphans:1;widows:1}}tr{{break-inside:avoid}}</style><p>Lead</p><table><thead><tr><th colspan=2>Header</th></tr></thead><tbody><tr><td rowspan=2>Span</td><td>A</td></tr><tr><td>B<br>C</td></tr></tbody><tfoot><tr><td colspan=2>Footer</td></tr></tfoot></table>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let pages = trace
        .pages
        .iter()
        .map(|page| {
            page.iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(pages.len(), 2, "Unexpected pagination: {pages:?}");
    assert!(
        pages[0].contains(&"A") && !pages[0].contains(&"B"),
        "The second row must move intact: {pages:?}"
    );
    assert!(
        pages[1].contains(&"B") && pages[1].contains(&"C"),
        "The second row must remain intact: {pages:?}"
    );
}

#[test]
fn rowspan_bands_respect_sibling_boundary_keeps() {
    let html = format!(
        "<!doctype html><style>{PROFILE}@page{{size:240pt 120pt;margin:10pt}}body{{font:10pt/20pt Spike}}p{{margin:0}}table{{width:100%;border-collapse:collapse}}td,th{{padding:0;text-align:left;font-weight:normal;vertical-align:top}}tr{{break-inside:avoid}}.keep{{break-after:avoid}}</style><p>Lead</p><table><thead><tr><th colspan=2>Header</th></tr></thead><tbody><tr><td rowspan=3>Span</td><td>A</td></tr><tr class=keep><td>B</td></tr><tr><td>C</td></tr></tbody><tfoot><tr><td colspan=2>Footer</td></tr></tfoot></table>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let pages = trace
        .pages
        .iter()
        .map(|page| {
            page.iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(pages.len(), 2, "Unexpected pagination: {pages:?}");
    assert!(
        pages[0].contains(&"A") && !pages[0].contains(&"B"),
        "The kept sibling pair must move together: {pages:?}"
    );
    assert!(
        pages[1].contains(&"B") && pages[1].contains(&"C"),
        "The sibling boundary must stay unbroken: {pages:?}"
    );
}

#[test]
fn spanning_cell_avoidance_only_keeps_its_own_rows() {
    let html = format!(
        "<!doctype html><style>{PROFILE}@page{{size:240pt 120pt;margin:10pt}}body{{font:10pt/20pt Spike}}p{{margin:0}}table{{width:100%;border-collapse:collapse}}td,th{{padding:0;text-align:left;font-weight:normal;vertical-align:top}}tr, .keep{{break-inside:avoid}}</style><p>Lead</p><table><thead><tr><th colspan=3>Header</th></tr></thead><tbody><tr><td rowspan=3>Span</td><td rowspan=2 class=keep>Kept</td><td>A</td></tr><tr><td>B</td></tr><tr><td>C</td><td>D</td></tr></tbody><tfoot><tr><td colspan=3>Footer</td></tr></tfoot></table>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let pages = trace
        .pages
        .iter()
        .map(|page| {
            page.iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(pages.len(), 2, "Unexpected pagination: {pages:?}");
    assert!(
        pages[0].contains(&"A") && pages[0].contains(&"B"),
        "The kept cell's two rows fit the first page: {pages:?}"
    );
    assert!(
        pages[1].contains(&"C") && pages[1].contains(&"D"),
        "The unrelated third row may break away: {pages:?}"
    );
}

#[test]
fn oversized_row_group_avoidance_relaxes_without_keeping_rowspan_pairs() {
    let rows = (1..=4)
        .map(|index| format!("<tr><td rowspan=2>Group {index}</td><td>A{index}</td></tr><tr><td>B{index}</td></tr>"))
        .collect::<String>();
    let html = format!(
        "<!doctype html><style>{PROFILE}@page{{size:240pt 120pt;margin:10pt}}body{{font:10pt/20pt Spike}}table{{width:100%;border-collapse:collapse}}td,th{{padding:0;text-align:left;font-weight:normal}}tr,tbody{{break-inside:avoid}}</style><table><thead><tr><th colspan=2>Header</th></tr></thead><tbody>{rows}</tbody><tfoot><tr><td colspan=2>Footer</td></tr></tfoot></table>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let pages = trace
        .pages
        .iter()
        .map(|page| {
            page.iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        pages.len(),
        3,
        "An oversized row-group keep must not keep arbitrary pairs: {pages:?}"
    );
    assert!(
        pages[0].contains(&"A2") && pages[1].contains(&"B2"),
        "Sibling rows remain independent after relaxing the group keep: {pages:?}"
    );
}

#[test]
fn moved_row_and_cell_destinations_follow_the_committed_fragment() {
    let html = format!(
        "<!doctype html><style>{PROFILE}@page{{size:240pt 120pt;margin:10pt}}body{{font:10pt/20pt Spike}}p{{margin:0}}table{{width:100%;border-collapse:collapse}}td,th{{padding:0;text-align:left;font-weight:normal;vertical-align:top;orphans:1;widows:1}}tr{{break-inside:avoid}}</style><p><a href='#row'>Row</a> <a href='#cell'>Cell</a></p><table><thead><tr><th colspan=2>Header</th></tr></thead><tbody><tr><td rowspan=2>Span</td><td>A</td></tr><tr id=row><td id=cell>B<br>C</td></tr></tbody><tfoot><tr><td colspan=2>Footer</td></tr></tfoot></table>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let [row, cell] = trace.destinations[..] else {
        panic!("Two destinations required: {:?}", trace.destinations)
    };
    for (name, destination) in [("row", row), ("cell", cell)] {
        let destination = destination.expect("The moved target must remain resolvable");
        assert_eq!(
            destination.page, 1,
            "{name} must target the second page: {destination:?}"
        );
        assert_pt(destination.top, 30.0);
    }
}

#[test]
fn fitting_table_scopes_keep_rows_together_but_forced_breaks_win() {
    for scope in ["table", "tbody", "td[rowspan]"] {
        for forced in [false, true] {
            let extra = if forced {
                ".next{break-before:page}"
            } else {
                ""
            };
            let html = format!(
                "<!doctype html><style>{PROFILE}@page{{size:240pt 120pt;margin:10pt}}body{{font:10pt/20pt Spike}}p{{margin:0}}table{{width:100%;border-collapse:collapse}}td,th{{padding:0;text-align:left;font-weight:normal;vertical-align:top}}tr,{scope}{{break-inside:avoid}}{extra}</style><p>Lead</p><table><thead><tr><th colspan=2>Header</th></tr></thead><tbody><tr><td rowspan=3>Span</td><td>A</td></tr><tr class=next><td>B</td></tr><tr><td>C</td></tr></tbody><tfoot><tr><td colspan=2>Footer</td></tr></tfoot></table>"
            );
            let trace = render_html(&html, &local_options())
                .unwrap()
                .layout
                .unwrap();
            let pages = trace
                .pages
                .iter()
                .map(|page| {
                    page.iter()
                        .map(|line| line.text.as_str())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            for label in ["A", "B", "C"] {
                assert_eq!(
                    pages
                        .iter()
                        .flatten()
                        .filter(|text| **text == label)
                        .count(),
                    1,
                    "{scope}, forced={forced}: {label} must occur once: {pages:?}"
                );
            }
            let page_of = |label| pages.iter().position(|page| page.contains(&label)).unwrap();
            if forced {
                assert!(
                    page_of("A") < page_of("B"),
                    "The forced row boundary must split {scope}: {pages:?}"
                );
                assert_eq!(
                    page_of("B"),
                    page_of("C"),
                    "The remaining kept rows fit together: {pages:?}"
                );
                assert!(pages.len() <= 3, "No extra empty fragments: {pages:?}");
            } else {
                assert_eq!(pages.len(), 2, "{scope}: {pages:?}");
                assert_eq!(
                    (page_of("A"), page_of("B"), page_of("C")),
                    (1, 1, 1),
                    "The fitting keep scope must move intact: {pages:?}"
                );
            }
        }
    }
}

#[test]
fn rowspan_columns_fragment_independently_at_their_own_line_boundaries() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tables/weasy70-rows.json")).unwrap();
    let output = render_weasy_case(&reference, &reference["parallel_flows"][0]);
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 2);
    for (page, expected) in [
        vec!["Lead", "A0", "A1", "A2", "A3", "A4", "B0", "B1", "B2", "B3"],
        vec!["A5", "A6", "A7", "C0", "C1", "After"],
    ]
    .iter()
    .enumerate()
    {
        let actual = &trace.pages[page];
        assert_eq!(actual.len(), expected.len(), "page {page} line count");
        for text in expected {
            assert_eq!(
                actual.iter().filter(|line| line.text == *text).count(),
                1,
                "page {page} must contain {text} exactly once"
            );
        }
    }
    let baseline = |page: usize, text: &str| {
        trace.pages[page]
            .iter()
            .find(|line| line.text == text)
            .unwrap()
            .baseline
    };
    const FIRST_ROW_HEIGHT: f32 = 80.0 * 40.0 / (40.0 + 20.0);
    const FIRST_PAGE_REMAINDER: f32 = 50.0;
    assert_pt(
        baseline(1, "C0") - baseline(1, "A5"),
        FIRST_ROW_HEIGHT - FIRST_PAGE_REMAINDER,
    );
    assert_pt(baseline(1, "C1") - baseline(1, "C0"), 10.0);
    assert_pt(baseline(1, "After") - baseline(1, "A7"), 10.0);
}

#[test]
fn rows_split_in_remaining_page_space_and_preserve_explicit_avoidance() {
    let mut reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tables/weasy70-rows.json")).unwrap();
    const ROWSPAN_FRONTIER: &str = "rowspan-first-row-frontier";
    const DISTRIBUTED_EXTRA: f64 = (80.0 - 40.0 - 20.0) * 40.0 / (40.0 + 20.0);
    // Preserve the frozen oracle; CSS Tables 3 distributes the spanning deficit.
    let frontier = reference["cases"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|case| case["name"] == ROWSPAN_FRONTIER)
        .unwrap();
    for line in frontier["pages"][1].as_array_mut().unwrap() {
        if matches!(line["text"].as_str(), Some("C0" | "C1")) {
            line["baseline"] = (line["baseline"].as_f64().unwrap() + DISTRIBUTED_EXTRA).into();
        }
    }
    assert_weasy_lines(&reference);
}

#[test]
fn sliced_cell_edges_do_not_reduce_nonfinal_page_line_capacity() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tables/weasy70-cell-edges.json")).unwrap();
    assert_weasy_lines(&reference);
}

#[test]
fn sliced_child_end_preserves_an_ongoing_parents_cloned_edge() {
    const LINE_HEIGHT_PT: f32 = 10.0;
    const CHILD_END_PT: f32 = 10.0;
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tables/weasy70-cell-edges.json")).unwrap();
    let case = reference["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "nested-slice-with-ongoing-cloned-parent")
        .unwrap();
    let trace = render_weasy_case(&reference, case).layout.unwrap();
    assert_eq!(trace.pages.len(), 2);
    let page_of = |text: &str| {
        let pages = trace
            .pages
            .iter()
            .enumerate()
            .filter(|(_, page)| page.iter().any(|line| line.text == text))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        assert_eq!(
            pages.len(),
            1,
            "{text} must appear on exactly one page: {pages:?}"
        );
        pages[0]
    };
    assert_eq!(page_of("A06"), 0);
    assert_eq!(
        page_of("A07"),
        1,
        "80pt of text plus a 10pt child end and 20pt cloned parent end cannot fit a 100pt fragment"
    );
    for index in 0..5 {
        assert_eq!(page_of(&format!("C{index:02}")), 1);
    }
    let final_lines = &trace.pages[1];
    let baseline = |text: &str| {
        final_lines
            .iter()
            .find(|line| line.text == text)
            .unwrap()
            .baseline
    };
    assert_pt(
        baseline("C00") - baseline("A07"),
        LINE_HEIGHT_PT + CHILD_END_PT,
    );
}

#[test]
fn fragmented_cell_decorations_keep_final_edges_and_every_line() {
    const PAGE_TOP_PT: f32 = 10.0;
    const PAGE_END_PT: f32 = 110.0;
    const CELL_WIDTH_PT: f32 = 110.0;
    const LINE_HEIGHT_PT: f32 = 10.0;
    const EDGE_TOLERANCE_PT: f32 = 0.03;
    const CELL_COLOR: [u8; 3] = [0, 0, 255];
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tables/weasy70-cell-edges.json")).unwrap();
    let baseline_offset = reference["cases"][0]["pages"][0][0]["baseline"]
        .as_f64()
        .unwrap() as f32
        - PAGE_TOP_PT;
    for (name, fragments, first_lines, cloned) in [
        ("nested-ends", 3, 10, false),
        ("columns-sliced-end", 3, 20, false),
        ("clone-bottom", 3, 9, true),
        ("clone-both", 4, 8, true),
    ] {
        let mut case = reference["observations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap()
            .clone();
        case["css"] = serde_json::Value::String(format!(
            "{}td:first-child{{background:blue;background-clip:content-box}}.inner{{background:red;background-clip:content-box}}",
            case["css"].as_str().unwrap()
        ));
        let trace = render_weasy_case(&reference, &case).layout.unwrap();
        let mut expected = case["pages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|page| page.as_array().unwrap())
            .map(|line| line["text"].as_str().unwrap())
            .collect::<Vec<_>>();
        let mut actual = trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        expected.sort_unstable();
        actual.sort_unstable();
        assert_eq!(
            actual, expected,
            "{name}: every authored line must survive exactly once"
        );
        assert_eq!(
            trace.pages[0]
                .iter()
                .filter(|line| line.text.starts_with('A'))
                .count(),
            first_lines,
            "{name}: intermediate sliced edges must not consume line capacity"
        );
        let mut cells = Vec::new();
        for (page, paints) in trace.display_lists.iter().enumerate() {
            for item in paints {
                if let DisplayItem::Box(paint) = item
                    && paint.decoration.background.rgb == CELL_COLOR
                {
                    assert_pt(paint.rect.x, PAGE_TOP_PT);
                    assert_pt(paint.rect.width, CELL_WIDTH_PT);
                    assert!(
                        paint.rect.y + paint.rect.height <= PAGE_END_PT + EDGE_TOLERANCE_PT,
                        "{name} page {page}: fragment edge exceeds the 100pt content area: {paint:?}"
                    );
                    let padding = if cloned || paint.last_fragment {
                        paint.padding[2]
                    } else {
                        0.0
                    };
                    for line in &trace.pages[page] {
                        if matches!(
                            line.text.as_bytes(),
                            [b'A' | b'C', b'0'..=b'9', b'0'..=b'9']
                        ) {
                            let line_end = line.baseline - baseline_offset + LINE_HEIGHT_PT;
                            assert!(
                                line_end
                                    <= paint.rect.y + paint.rect.height - padding
                                        + EDGE_TOLERANCE_PT,
                                "{name} page {page}: {} enters the ending padding",
                                line.text
                            );
                        }
                    }
                    cells.push(paint);
                }
            }
        }
        assert_eq!(
            cells.len(),
            fragments,
            "{name}: one first-cell paint per fragment"
        );
        let last = cells.last().unwrap();
        assert!(
            last.last_fragment,
            "{name}: final decoration edge must be retained"
        );
        assert_pt(last.padding[2], LINE_HEIGHT_PT);
    }
}

#[test]
fn completed_table_keeps_its_final_line_with_ancestor_end_padding() {
    const LINE_AND_PADDING_PT: f32 = 20.0;
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tables/weasy70-cell-edges.json")).unwrap();
    for name in ["outer-table-end", "nested-table-end"] {
        let case = reference["observations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap();
        let trace = render_weasy_case(&reference, case).layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            2,
            "{name}: final line and parent end share a continuation"
        );
        assert!(
            trace.pages[0].iter().any(|line| line.text == "A08"),
            "{name}: retain fitting earlier lines"
        );
        assert!(
            !trace.pages[0].iter().any(|line| line.text == "A09"),
            "{name}: a zero-gap child/parent boundary cannot strand padding on a page alone"
        );
        let final_line = trace.pages[1]
            .iter()
            .find(|line| line.text == "A09")
            .unwrap_or_else(|| panic!("{name}: final line must precede the 10pt ancestor padding"));
        let after = trace.pages[1]
            .iter()
            .find(|line| line.text == "After")
            .unwrap();
        assert_pt(after.baseline - final_line.baseline, LINE_AND_PADDING_PT);
    }
}

#[test]
fn final_column_trials_keep_cell_padding_and_repeated_footers_with_content() {
    const LINE_HEIGHT_PT: f32 = 10.0;
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tables/weasy70-cell-edges.json")).unwrap();
    for (name, capacity, final_gap, repeated) in [
        (
            "columns-final-19",
            &[18, 1][..],
            2.0 * LINE_HEIGHT_PT,
            false,
        ),
        (
            "columns-footer-final-47",
            &[16, 16, 14, 1][..],
            3.0 * LINE_HEIGHT_PT,
            true,
        ),
    ] {
        let case = reference["observations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap();
        let trace = render_weasy_case(&reference, case).layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            capacity.len(),
            "{name}: ending decoration cannot be isolated from the final source line"
        );
        for (page, &count) in capacity.iter().enumerate() {
            let labels = &trace.pages[page];
            assert_eq!(
                labels
                    .iter()
                    .filter(|line| matches!(line.text.as_bytes(), [b'A', b'0'..=b'9', b'0'..=b'9']))
                    .count(),
                count,
                "{name} page {page}: column content exceeds the final-edge capacity"
            );
            if repeated {
                for text in ["Head", "Foot"] {
                    assert_eq!(
                        labels.iter().filter(|line| line.text == text).count(),
                        1,
                        "{name} page {page}: {text} must accompany the body's cell fragment"
                    );
                }
            }
        }
        let lines = trace.pages.iter().flatten().collect::<Vec<_>>();
        let total = capacity.iter().sum::<usize>();
        for text in (0..total)
            .map(|index| format!("A{index:02}"))
            .chain(["B00".to_owned(), "After".to_owned()])
        {
            assert_eq!(
                lines.iter().filter(|line| line.text == text).count(),
                1,
                "{name}: {text} must survive exactly once"
            );
        }
        let last = trace.pages.last().unwrap();
        let baseline = |text: &str| last.iter().find(|line| line.text == text).unwrap().baseline;
        assert_pt(
            baseline("After") - baseline(&format!("A{:02}", total - 1)),
            final_gap,
        );
    }
}

#[test]
fn column_segments_keep_all_content_and_close_only_after_the_last_segment() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tables/weasy70-cell-edges.json")).unwrap();
    for (name, a_counts, c_counts) in [
        ("columns-nonfinal-spanner", &[19, 0, 0][..], &[0, 18, 1][..]),
        ("columns-spanner-ending-block", &[1, 0][..], &[0, 2][..]),
    ] {
        let case = reference["observations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap();
        let trace = render_weasy_case(&reference, case).layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            a_counts.len(),
            "{name}: wrong continuation count"
        );
        for (prefix, counts) in [('A', a_counts), ('C', c_counts)] {
            let mut index = 0;
            for (page, &count) in counts.iter().enumerate() {
                for _ in 0..count {
                    let label = format!("{prefix}{index:02}");
                    assert_eq!(
                        trace.pages[page]
                            .iter()
                            .filter(|line| line.text == label)
                            .count(),
                        1,
                        "{name} page {page}: {label} must be retained at the source-derived capacity"
                    );
                    index += 1;
                }
            }
            assert_eq!(
                trace
                    .pages
                    .iter()
                    .flatten()
                    .filter(|line| line.text.starts_with(prefix) && line.text != "After")
                    .count(),
                index,
                "{name}: {prefix} source conservation"
            );
        }
        for text in ["Span", "B00", "After"] {
            assert_eq!(
                trace
                    .pages
                    .iter()
                    .flatten()
                    .filter(|line| line.text == text)
                    .count(),
                1,
                "{name}: {text} must survive exactly once"
            );
        }
        let last = trace.pages.last().unwrap();
        let c_last = format!("C{:02}", c_counts.iter().sum::<usize>() - 1);
        let baseline = |text: &str| last.iter().find(|line| line.text == text).unwrap().baseline;
        assert_pt(baseline("After") - baseline(&c_last), 20.0);
    }
}

#[test]
fn automatic_cell_constraints_match_independent_weasyprint_70_geometry() {
    assert_weasy_widths(&[
        "max-length",
        "min-length",
        "min-wins",
        "max-percentage",
        "min-percentage",
        "max-zero-percentage",
        "border-box-limit",
        "wide-capped",
        "collapsed-limits",
        "fixed-layout-limits",
    ]);
}

#[test]
fn specified_cell_width_uses_its_min_content_floor_instead_of_max_content() {
    assert_weasy_widths(&[
        "narrow-length",
        "zero-length",
        "wide-within-specified-table",
    ]);
}

#[test]
fn percentage_only_columns_fill_the_specified_table_width() {
    assert_weasy_widths(&[
        "percentage-columns-under-total",
        "percentage-cells-under-total",
    ]);
}

fn assert_weasy_widths(names: &[&str]) {
    const REFERENCE_TOLERANCE_PT: f32 = 0.03;
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/tables/weasy70-widths.json")).unwrap();
    for name in names {
        let case = reference["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == *name)
            .unwrap_or_else(|| panic!("missing independently generated case {name}"));
        let result = render_weasy_case(&reference, case);
        let trace = result.layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            1,
            "reference page count for {}",
            case["name"]
        );
        for (key, color) in [("c0", [255, 0, 0]), ("c1", [0, 0, 255])] {
            let paint = trace.display_lists[0]
                .iter()
                .find_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.background.rgb == color => {
                        Some(paint)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing {key} cell paint for {}", case["name"]));
            let expected = case["widths"][key].as_f64().unwrap() as f32;
            assert!(
                (paint.rect.width - expected).abs() <= REFERENCE_TOLERANCE_PT,
                "{} {key}: actual {}pt, independent WeasyPrint 70 reference {expected}pt",
                case["name"],
                paint.rect.width
            );
        }
    }
}

fn native_line(lines: &[TextLine], text: &str, x: f32, baseline: f32) {
    let line = lines
        .iter()
        .find(|line| line.text == text)
        .unwrap_or_else(|| {
            panic!(
                "missing {text:?}; lines: {:?}",
                lines.iter().map(|line| &line.text).collect::<Vec<_>>()
            )
        });
    assert!(
        (line.x - x).abs() < GEOMETRY_TOLERANCE_PT
            && (line.baseline - baseline).abs() < GEOMETRY_TOLERANCE_PT,
        "{text:?}: native ({x}, {baseline}), actual ({}, {})",
        line.x,
        line.baseline
    );
}

#[test]
fn automatic_table_columns_use_content_widths_and_default_cell_spacing() {
    let result = render_html(
        include_str!("oracle/fixtures/table-auto.html"),
        &local_options(),
    )
    .expect("automatic table formatting must render through the library");
    let trace = result.layout.unwrap();
    assert_eq!(trace.pages.len(), 1);
    let lines = &trace.pages[0];
    native_line(lines, "Before", 36.0, 49.1543);
    // HTML supplies 1px on each cell edge; the CSS-role test below has no UA padding.
    native_line(
        lines,
        "A",
        37.5 + HTML_CELL_PADDING_PT,
        80.6543 + HTML_CELL_PADDING_PT,
    );
    native_line(
        lines,
        "Longer words",
        63.6973 + 3.0 * HTML_CELL_PADDING_PT,
        80.6543 + HTML_CELL_PADDING_PT,
    );
    native_line(
        lines,
        "BBB",
        37.5 + HTML_CELL_PADDING_PT,
        100.1543 + 3.0 * HTML_CELL_PADDING_PT,
    );
    native_line(
        lines,
        "C",
        63.6973 + 3.0 * HTML_CELL_PADDING_PT,
        100.1543 + 3.0 * HTML_CELL_PADDING_PT,
    );
    native_line(lines, "After", 36.0, 119.6543 + 4.0 * HTML_CELL_PADDING_PT);
}

#[test]
fn css_table_roles_use_the_same_grid_as_html_table_elements() {
    let html = format!(
        "<style>{PROFILE}.table{{display:table;border-spacing:1.5pt}}.row{{display:table-row}}.cell{{display:table-cell}}</style><p>Before</p><div class='table'><div class='row'><div class='cell'>A</div><div class='cell'>Longer words</div></div><div class='row'><div class='cell'>BBB</div><div class='cell'>C</div></div></div><p>After</p>"
    );
    let result =
        render_html(&html, &local_options()).expect("table roles must follow computed display");
    let trace = result.layout.unwrap();
    let lines = &trace.pages[0];
    native_line(lines, "A", 37.5, 80.6543);
    native_line(lines, "Longer words", 63.6973, 80.6543);
    native_line(lines, "BBB", 37.5, 100.1543);
    native_line(lines, "After", 36.0, 119.6543);
}

#[test]
fn separated_table_borders_and_padding_contribute_to_both_axes() {
    let result = render_html(
        include_str!("oracle/fixtures/table-separated.html"),
        &local_options(),
    )
    .expect("separated table borders must retain row and column geometry");
    let trace = result.layout.unwrap();
    let lines = &trace.pages[0];
    native_line(lines, "A", 46.0, 91.1543);
    native_line(lines, "Longer words", 82.6973, 91.1543);
    native_line(lines, "BBB", 46.0, 123.1543);
    native_line(lines, "After", 36.0, 153.1543);
    let boxes: Vec<_> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) => Some(paint),
            _ => None,
        })
        .collect();
    assert_eq!(
        boxes.len(),
        5,
        "table and four independently decorated cells"
    );
    assert!(
        (boxes[0].rect.height - 74.0).abs() < GEOMETRY_TOLERANCE_PT,
        "native separated table height 74pt, got {}",
        boxes[0].rect.height
    );
}

#[test]
fn captions_share_the_table_width_and_keep_top_and_bottom_placement() {
    let result = render_html(
        include_str!("oracle/fixtures/table-captions.html"),
        &local_options(),
    )
    .expect("caption boxes must participate in the table wrapper");
    let trace = result.layout.unwrap();
    let lines = &trace.pages[0];
    native_line(lines, "Top caption", 91.6904, 85.1543);
    native_line(lines, "A", 46.0, 121.1543);
    native_line(lines, "Bottom caption", 79.8076, 157.1543);
    // CSS Tables 3 §3.11 includes each caption margin once in the wrapper.
    native_line(lines, "After", 36.0, 181.1543);
}

#[test]
fn fixed_columns_wrap_cell_content_and_share_row_height_with_middle_alignment() {
    let result = render_html(
        include_str!("oracle/fixtures/table-fixed.html"),
        &local_options(),
    )
    .expect("fixed layout must use col widths independently of later content");
    let trace = result.layout.unwrap();
    let lines = &trace.pages[0];
    native_line(lines, "A", 46.0, 79.1543);
    native_line(lines, "Longer words", 110.0, 61.1543);
    native_line(lines, "wrap in this", 110.0, 79.1543);
    native_line(lines, "table", 110.0, 97.1543);
    native_line(lines, "BBB", 46.0, 129.1543);
    native_line(lines, "After", 36.0, 159.1543);
}

#[test]
fn fixed_table_columns_take_widths_from_columns_inside_sized_column_groups() {
    let html = format!(
        "<!doctype html><style>{PROFILE}table{{table-layout:fixed;width:200pt;border-spacing:0}}colgroup{{width:300pt}}td{{padding:0}}</style><table><colgroup><col style='width:20pt'><col></colgroup><tr><td>A</td><td>B</td></tr></table>"
    );
    let result = render_html(&html, &local_options()).unwrap();
    let trace = result.layout.unwrap();
    native_line(&trace.pages[0], "A", 36.0, 49.1543);
    native_line(&trace.pages[0], "B", 56.0, 49.1543);
}

#[test]
fn hidden_empty_cells_reveal_the_table_background_but_block_children_are_nonempty() {
    let result = render_html(
        include_str!("oracle/fixtures/table-empty-cells.html"),
        &local_options(),
    )
    .unwrap();
    let trace = result.layout.unwrap();
    let mut decorated_cells = 0;
    for item in &trace.display_lists[0] {
        if let DisplayItem::Box(paint) = item {
            if paint.decoration.borders[0].used_width() > 0.0 {
                decorated_cells += 1;
            }
            if paint.rect.x <= 42.0
                && paint.rect.x + paint.rect.width > 42.0
                && paint.rect.y <= 42.0
                && paint.rect.y + paint.rect.height > 42.0
            {
                assert!(
                    paint.rect.width > 150.0,
                    "only the table canvas may paint through the empty first cell: {paint:?}"
                );
            }
        }
    }
    assert_eq!(
        decorated_cells, 4,
        "only the two childless cells lose their borders; empty block children still count as content"
    );
}

#[test]
fn caption_margins_are_retained_at_the_start_of_a_page() {
    let result = render_html(
        include_str!("oracle/fixtures/table-caption-margins.html"),
        &local_options(),
    )
    .unwrap();
    let trace = result.layout.unwrap();
    native_line(&trace.pages[0], "First top", 101.6338, 53.1543);
    native_line(&trace.pages[0], "A", 46.0, 112.1543);
    native_line(&trace.pages[0], "After", 36.0, 142.1543);
}

#[test]
fn fragmented_tables_repeat_row_spacing_and_keep_all_rows_in_source_order() {
    let result = render_html(
        include_str!("oracle/fixtures/table-pagination.html"),
        &local_options(),
    )
    .unwrap();
    let trace = result.layout.unwrap();
    assert_eq!(trace.pages.len(), 6);
    let mut rows = Vec::new();
    for page in &trace.pages {
        rows.extend(
            page.iter()
                .filter(|line| line.text.starts_with("Row "))
                .map(|line| line.text.clone()),
        );
    }
    assert_eq!(
        rows,
        (0..24)
            .map(|row| format!("Row {row:02}"))
            .collect::<Vec<_>>()
    );
    native_line(&trace.pages[0], "Row 00", 46.0, 91.1543);
    native_line(&trace.pages[1], "Row 04", 46.0, 59.1543);
    native_line(&trace.pages[4], "Row 23", 46.0, 187.1543);
    native_line(&trace.pages[5], "After", 36.0, 49.1543);
}

#[test]
fn spanning_cells_take_the_entire_background_of_their_originating_column() {
    let html = format!(
        "<style>{PROFILE}table{{width:200pt;table-layout:fixed;border-spacing:0}}</style><table><colgroup style='background:yellow'><col style='background:red'><col style='background:blue'></colgroup><tr><td colspan='2'>A</td></tr><tr><td>B</td><td>C</td></tr></table>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let paints: Vec<_> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => Some(paint),
            _ => None,
        })
        .collect();
    let red = paints
        .iter()
        .find(|paint| paint.rect.y == 36.0 && paint.decoration.background.rgb == [255, 0, 0])
        .unwrap();
    assert_eq!(
        red.rect.width, 200.0,
        "the first column's background covers the entire 200pt spanning cell"
    );
    assert!(
        !paints
            .iter()
            .any(|paint| paint.rect.y == 36.0 && paint.decoration.background.rgb == [0, 0, 255]),
        "the second column cannot paint through a cell originating in the first column"
    );
}

fn trace(html: &str) -> ferri_pdf::LayoutTrace {
    render_html(html, &local_options())
        .expect("collapsed table fixture must render")
        .layout
        .unwrap()
}

/// Collapsed border runs: box paints with exactly one painted side.
fn border_runs(items: &[DisplayItem]) -> Vec<(&ferri_pdf::BoxPaint, ferri_pdf::Border)> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) => {
                let mut painted = paint
                    .decoration
                    .borders
                    .into_iter()
                    .filter(|border| border.used_width() > 0.0 && border.color.alpha > 0.0);
                let border = painted.next()?;
                painted.next().is_none().then_some((paint, border))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn collapsed_cells_inset_half_borders_and_wider_later_rows_spill_into_the_margin() {
    let trace = trace(include_str!("oracle/fixtures/table-collapse-widths.html"));
    let lines = &trace.pages[0];
    // The table border is half of A's 3pt start edge, table padding and
    // spacing are ignored, and C's 9pt start border insets its content by 4.5pt.
    native_line(lines, "A", 43.0, 58.6543);
    native_line(lines, "B", 65.3789, 58.1543);
    native_line(lines, "C", 46.0, 88.1543);
    native_line(lines, "D", 63.3789, 89.1543);
    native_line(lines, "After", 36.0, 114.1543);
    let runs = border_runs(&trace.display_lists[0]);
    let (spill, _) = runs
        .iter()
        .find(|(_, border)| border.width == 9.0)
        .unwrap_or_else(|| panic!("missing the 9pt run; runs: {runs:?}"));
    assert!(
        (spill.rect.x - 33.0).abs() < GEOMETRY_TOLERANCE_PT,
        "the 9pt border centred on the 37.5pt grid line spills 3pt into the margin: {:?}",
        spill.rect
    );
}

#[test]
fn collapsed_joints_belong_to_the_border_that_wins_conflict_resolution() {
    let trace = trace(include_str!("oracle/fixtures/table-collapse-widths.html"));
    let runs = border_runs(&trace.display_lists[0]);
    let colours: Vec<_> = runs.iter().map(|(_, border)| border.color.rgb).collect();
    let last_weaker = colours
        .iter()
        .rposition(|rgb| *rgb == [255, 0, 0] || *rgb == [0, 0, 255])
        .expect("red cell and blue table runs");
    let first_green = colours
        .iter()
        .position(|rgb| *rgb == [0, 128, 0])
        .expect("green 5pt runs");
    assert!(
        last_weaker < first_green,
        "wider borders paint last so they own their joints; paint order {colours:?}"
    );
}

#[test]
fn spanning_cells_take_collapsed_insets_from_the_first_segment_of_each_edge() {
    let trace = trace(include_str!("oracle/fixtures/table-collapse-spans.html"));
    let lines = &trace.pages[0];
    // The colspan cell's bottom edge has 2pt and 4pt segments; the 26pt row is sized from
    // the first one, and E's top edge likewise takes A's 4pt bottom segment.
    native_line(lines, "Wide cell", 61.2324, 55.1543);
    native_line(lines, "C", 61.2324, 82.1543);
    native_line(lines, "D", 92.2059, 82.1543);
    native_line(lines, "E", 42.0, 110.1543);
    native_line(lines, "F", 91.2059, 110.1543);
    native_line(lines, "After", 36.0, 133.1543);
}

#[test]
fn collapsed_page_breaks_paint_the_shared_row_border_on_both_fragments() {
    let trace = trace(include_str!(
        "oracle/fixtures/table-collapse-pagination.html"
    ));
    assert_eq!(trace.pages.len(), 4);
    let first = border_runs(&trace.display_lists[0]);
    let bottom = first
        .iter()
        .filter(|(paint, _)| paint.rect.width > paint.rect.height)
        .max_by(|a, b| a.0.rect.y.total_cmp(&b.0.rect.y))
        .unwrap();
    assert_eq!(
        (bottom.0.rect.y + bottom.0.rect.height, bottom.1.color.rgb),
        (181.0, [0, 128, 0]),
        "the first fragment ends with the full 3pt green border between R4 and R5: {:?}",
        bottom.0.rect
    );
    let vertical_end = first
        .iter()
        .filter(|(paint, _)| paint.rect.height > paint.rect.width)
        .map(|(paint, _)| paint.rect.y + paint.rect.height)
        .fold(0.0f32, f32::max);
    assert_eq!(
        vertical_end, 181.0,
        "vertical borders reach the outer edge of the break border"
    );
    let second = border_runs(&trace.display_lists[1]);
    let top = second
        .iter()
        .filter(|(paint, _)| paint.rect.width > paint.rect.height)
        .min_by(|a, b| a.0.rect.y.total_cmp(&b.0.rect.y))
        .unwrap();
    assert_eq!(
        (top.0.rect.y, top.0.rect.height, top.1.color.rgb),
        (36.5, 3.0, [0, 128, 0]),
        "the continuation repeats the break border centred 2pt (the table's top border width) below the page top"
    );
    native_line(&trace.pages[1], "R5", 44.0, 56.6543);
}

#[test]
fn header_and_footer_groups_repeat_on_every_page_in_both_border_models() {
    let collapsed = trace(include_str!("oracle/fixtures/table-collapse-repeat.html"));
    let separated = trace(include_str!("oracle/fixtures/table-repeat-separated.html"));
    for (name, trace, pages) in [("collapsed", &collapsed, 5), ("separated", &separated, 6)] {
        assert_eq!(trace.pages.len(), pages, "{name} page count");
        let mut rows = Vec::new();
        for (index, page) in trace.pages.iter().enumerate() {
            let count = |text: &str| page.iter().filter(|line| line.text == text).count();
            let fragment = usize::from(page.iter().any(|line| line.text.starts_with("Row ")));
            assert_eq!(
                (count("Head"), count("Foot")),
                (fragment, fragment),
                "{name} page {}: {:?}",
                index + 1,
                page.iter().map(|line| &line.text).collect::<Vec<_>>()
            );
            rows.extend(
                page.iter()
                    .filter(|line| line.text.starts_with("Row "))
                    .map(|line| line.text.clone()),
            );
        }
        assert_eq!(
            rows,
            (0..14).map(|row| format!("Row {row}")).collect::<Vec<_>>(),
            "{name} body rows appear once in order"
        );
    }
    native_line(&collapsed.pages[1], "Head", 44.0, 57.1543);
    native_line(&collapsed.pages[1], "Foot", 44.0, 169.6543);
    // The last body row's bottom edge resolves against the footer's 3pt top border.
    native_line(&collapsed.pages[4], "Foot", 44.0, 170.1543);
    native_line(&collapsed.pages[4], "After", 36.0, 196.1543);
    native_line(&separated.pages[1], "Head", 52.3311, 59.1543);
}

#[test]
fn empty_cells_keep_their_backgrounds_in_the_collapsing_model() {
    let trace = trace(include_str!(
        "oracle/fixtures/table-collapse-empty-cells.html"
    ));
    let backgrounds = trace.display_lists[0]
        .iter()
        .filter(|item| {
            matches!(item, DisplayItem::Box(paint)
                if paint.decoration.background.rgb == [0xdd; 3]
                    && paint.decoration.background.alpha > 0.0)
        })
        .count();
    assert_eq!(
        backgrounds, 4,
        "empty-cells applies only to separated borders (CSS 2.1 §17.6.1.1)"
    );
}

/// Background paints of one page by colour, in paint order.
fn background_rects(items: &[DisplayItem], rgb: [u8; 3]) -> Vec<ferri_pdf::Rect> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == rgb => {
                paint.background_fill().map(|fill| match fill.shape {
                    ferri_pdf::Shape::Rect(rect) => rect,
                    shape => panic!("square cell backgrounds fill rectangles: {shape:?}"),
                })
            }
            _ => None,
        })
        .collect()
}

#[test]
fn automatic_layout_cell_widths_honour_box_sizing() {
    const CELL: [u8; 3] = [0xff, 0xee, 0xaa];
    const WIDTH_PT: f32 = 4.0 / 2.54 * 72.0;
    const EDGES_PT: f32 = 2.0 * 10.0 + 2.0 * 1.0;
    let trace = trace(include_str!(
        "oracle/fixtures/table-cell-width-box-sizing.html"
    ));
    let cells = background_rects(&trace.display_lists[0], CELL);
    assert_eq!(cells.len(), 4, "two header cells per table: {cells:?}");
    for (label, rect, expected) in [
        ("content-box", cells[0], WIDTH_PT + EDGES_PT),
        ("border-box", cells[2], WIDTH_PT),
    ] {
        assert!(
            (rect.width - expected).abs() < GEOMETRY_TOLERANCE_PT,
            "{label} width:4cm is a {expected}pt border box under CSS Tables 3: {rect:?}"
        );
    }
}

#[test]
fn collapsed_cell_backgrounds_clip_inside_their_half_borders() {
    const CELL: [u8; 3] = [0xff, 0xee, 0xaa];
    let html = format!(
        "<style>{PROFILE}table{{border-collapse:collapse}}\
         td{{border:6pt solid navy;padding:2pt;background:#fea}}\
         .p{{background-clip:padding-box}}</style>\
         <table><tr><td>Border box</td><td class='p'>Padding box</td></tr></table>"
    );
    let trace = trace(&html);
    let cells: Vec<_> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == CELL => Some(paint),
            _ => None,
        })
        .collect();
    assert_eq!(cells.len(), 2, "one background per cell: {cells:?}");
    let fills = background_rects(&trace.display_lists[0], CELL);
    assert_eq!(
        fills[0], cells[0].rect,
        "border-box backgrounds reach the grid line centres"
    );
    let inset = ferri_pdf::Rect {
        x: cells[1].rect.x + 3.0,
        y: cells[1].rect.y + 3.0,
        width: cells[1].rect.width - 6.0,
        height: cells[1].rect.height - 6.0,
    };
    assert_eq!(
        fills[1], inset,
        "padding-box backgrounds stop at the cell's half of the 6pt collapsed borders"
    );
}

/// Page number and text of every line, in page order.
fn page_lines(trace: &ferri_pdf::LayoutTrace) -> Vec<(usize, &str)> {
    trace
        .pages
        .iter()
        .enumerate()
        .flat_map(|(index, page)| page.iter().map(move |line| (index + 1, line.text.as_str())))
        .collect()
}

#[test]
fn repeated_header_groups_move_with_the_first_body_row_instead_of_ending_a_page_alone() {
    let trace = trace(include_str!("oracle/fixtures/table-header-orphan.html"));
    let lines = page_lines(&trace);
    // Each table moves whole to the next page when only
    // its header (and footer) would fit; the last one fits its first body row.
    let expected = [
        (2, "One head", 49.1543),
        (2, "One body 1", 67.1543),
        (2, "One body 2", 85.1543),
        (4, "Two head A", 49.1543),
        (4, "Two head B", 67.1543),
        (4, "Two body 1", 85.1543),
        (6, "Foot head", 49.1543),
        (6, "Foot body 1", 67.1543),
        (6, "Foot body 2", 85.1543),
        (6, "Foot foot", 103.1543),
        (8, "Empty head", 49.1543),
        (8, "Empty body 1", 67.1543),
        (9, "Fits head", 101.1543),
        (9, "Fits body 1", 119.1543),
        (10, "Fits head", 49.1543),
        (10, "Fits body 2", 67.1543),
    ];
    assert_eq!(
        lines,
        expected
            .iter()
            .map(|&(page, text, _)| (page, text))
            .collect::<Vec<_>>(),
        "table lines by page"
    );
    let baselines: Vec<_> = trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.baseline)
        .collect();
    for ((page, text, baseline), actual) in expected.into_iter().zip(baselines) {
        assert!(
            (actual - baseline).abs() < GEOMETRY_TOLERANCE_PT,
            "page {page} {text:?}: expected baseline {baseline}, actual {actual}"
        );
    }
}

#[test]
fn forced_break_before_the_first_body_row_leaves_the_header_above_it() {
    let trace = trace(include_str!("oracle/fixtures/table-header-forced.html"));
    assert_eq!(
        page_lines(&trace),
        [
            (1, "Before"),
            (1, "Head"),
            (2, "Head"),
            (2, "Body 1"),
            (2, "Body 2")
        ],
        "break-before:page on the first body row is honoured where it occurs, and the header repeats above it"
    );
    native_line(&trace.pages[0], "Head", 36.0, 67.1543);
    native_line(&trace.pages[1], "Head", 36.0, 49.1543);
}

#[test]
fn caption_margins_contribute_once_to_the_table_wrapper() {
    for (top_after, bottom_before, bottom_after) in [(7, 0, 0), (-3, 0, 0), (7, 5, 11), (-3, 5, -2)]
    {
        let output=render_html(
            &format!("<!doctype html><style>{PROFILE}html,body,caption,td,p{{margin:0;padding:0;font:10pt/10pt Spike;widows:1;orphans:1}}table{{width:100pt;border-spacing:0}}caption{{text-align:left}}.top{{margin-bottom:{top_after}pt}}.bottom{{caption-side:bottom;margin-top:{bottom_before}pt;margin-bottom:{bottom_after}pt}}</style><table><caption class=top>Top</caption><caption class=bottom>Bottom</caption><tr><td>Cell</td></tr></table><p>After</p>"),
            &local_options(),
        ).expect("caption margins must obey ordinary block box accounting");
        let trace = output.layout.unwrap();
        assert_eq!(trace.pages.len(), 1);
        let lines = &trace.pages[0];
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Top", "Cell", "Bottom", "After"]
        );
        let expected = [
            0.0,
            10.0 + top_after as f32,
            20.0 + (top_after + bottom_before) as f32,
            30.0 + (top_after + bottom_before + bottom_after) as f32,
        ];
        for (line, gap) in lines.iter().zip(expected) {
            assert!(
                (line.baseline - lines[0].baseline - gap).abs() < GEOMETRY_TOLERANCE_PT,
                "margins ({top_after},{bottom_before},{bottom_after})/{}: expected gap {gap}, got {}",
                line.text,
                line.baseline - lines[0].baseline
            );
        }
    }
}

#[test]
fn multiple_caption_margins_collapse_without_extra_trailing_space() {
    for (margins, gaps) in [
        ([7, 3, 5, 4, 8, 2, 6], [0.0, 17.0, 32.0, 46.0, 64.0, 80.0]),
        (
            [-3, 5, -2, -1, -4, 2, -1],
            [0.0, 12.0, 20.0, 29.0, 37.0, 46.0],
        ),
    ] {
        let [
            first_after,
            second_before,
            second_after,
            bottom_before,
            bottom_after,
            last_before,
            last_after,
        ] = margins;
        let trace=render_html(
            &format!("<!doctype html><style>{PROFILE}html,body,caption,td,p{{margin:0;padding:0;font:10pt/10pt Spike;widows:1;orphans:1}}table{{width:100pt;border-spacing:0}}caption{{text-align:left}}.first{{margin-bottom:{first_after}pt}}.second{{margin:{second_before}pt 0 {second_after}pt}}.bottom{{caption-side:bottom;margin:{bottom_before}pt 0 {bottom_after}pt}}.last{{caption-side:bottom;margin:{last_before}pt 0 {last_after}pt}}</style><table><caption class=first>TopA</caption><caption class=second>TopB</caption><caption class=bottom>BottomA</caption><caption class=last>BottomB</caption><tr><td>Cell</td></tr></table><p>After</p>"),
            &local_options(),
        ).unwrap().layout.unwrap();
        assert_eq!(trace.pages.len(), 1);
        let lines = &trace.pages[0];
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["TopA", "TopB", "Cell", "BottomA", "BottomB", "After"]
        );
        for (line, gap) in lines.iter().zip(gaps) {
            assert!(
                (line.baseline - lines[0].baseline - gap).abs() < GEOMETRY_TOLERANCE_PT,
                "margins {margins:?}/{}: expected gap {gap}, got {}",
                line.text,
                line.baseline - lines[0].baseline
            );
        }
    }
}

#[test]
fn collapsed_rows_and_groups_ignore_outer_shadows() {
    for selector in ["tr", "tbody", "thead,tfoot", "tr,tbody,thead,tfoot"] {
        let html = format!(
            "<!doctype html><style>{PROFILE}table{{border-collapse:collapse}}{selector}{{box-shadow:8pt 3pt 2pt red}}td,th{{padding:0}}</style><table><thead><tr><th>H</th></tr></thead><tbody><tr><td>B</td></tr></tbody><tfoot><tr><td>F</td></tr></tfoot></table>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap_or_else(|error| panic!("{selector}: inapplicable shadow rejected: {error}"))
            .layout
            .unwrap();
        for item in trace.display_lists.iter().flatten() {
            if let DisplayItem::Box(paint) = item {
                assert!(
                    paint.decoration.shadows.is_empty(),
                    "{selector}: unexpected shadow {paint:?}"
                );
            }
        }
        for label in ["H", "B", "F"] {
            assert_eq!(
                trace
                    .pages
                    .iter()
                    .flatten()
                    .filter(|line| line.text == label)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn collapsed_cells_keep_inset_shadows_and_roots_keep_outer_shadows() {
    for collapse in ["collapse", "separate"] {
        let html = format!(
            "<!doctype html><style>{PROFILE}table{{border-collapse:{collapse};background:blue;box-shadow:8pt 0 red}}td{{padding:0;background:yellow;box-shadow:8pt 0 red,inset 0 0 0 2pt green}}</style><table><tr><td>A</td></tr></table>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let mut cells = 0;
        let mut roots = 0;
        for item in trace.display_lists.iter().flatten() {
            if let DisplayItem::Box(paint) = item {
                match paint.decoration.background.rgb {
                    [255, 255, 0] => {
                        cells += 1;
                        assert_eq!(
                            paint.shadows(true).len(),
                            1,
                            "{collapse}: inset shadow lost"
                        );
                        assert_eq!(
                            paint.shadows(false).len(),
                            usize::from(collapse == "separate"),
                            "{collapse}: wrong cell outer shadow applicability"
                        );
                    }
                    [0, 0, 255] => {
                        roots += 1;
                        assert_eq!(
                            paint.shadows(false).len(),
                            1,
                            "{collapse}: root shadow lost"
                        );
                    }
                    _ => {}
                }
            }
        }
        assert_eq!((cells, roots), (1, 1), "{collapse}: missing paint controls");
    }
}

#[test]
fn split_collapsed_cells_filter_outer_shadows_without_changing_nested_tables() {
    let html = format!(
        "<!doctype html><style>{PROFILE}@page{{size:200pt 100pt;margin:10pt}}body{{margin:0}}table{{border-collapse:collapse}}td{{padding:0;background:yellow;box-shadow:5pt 0 red,inset 0 0 0 2pt green}}p{{margin:0;height:35pt}}.nested{{border-collapse:separate}}.nested td{{background:blue}}</style><table><tr><td><p>A</p><p>B</p><p>C</p><table class=nested><tr><td>N</td></tr></table></td></tr></table><p>Z</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert!(trace.pages.len() > 1, "must exercise cell continuations");
    let mut fragments = 0;
    let mut nested = 0;
    for item in trace.display_lists.iter().flatten() {
        if let DisplayItem::Box(paint) = item {
            match paint.decoration.background.rgb {
                [255, 255, 0] => {
                    fragments += 1;
                    assert_eq!(
                        paint.decoration.shadows.len(),
                        1,
                        "fragment lost inset or retained outer shadow: {paint:?}"
                    );
                    assert!(paint.decoration.shadows[0].inset);
                }
                [0, 0, 255] => {
                    nested += 1;
                    assert_eq!(
                        paint.shadows(false).len(),
                        1,
                        "nested separated table lost outer shadow"
                    );
                    assert_eq!(paint.shadows(true).len(), 1);
                }
                _ => {}
            }
        }
    }
    assert!(fragments > 1);
    assert_eq!(nested, 1);
    for label in ["A", "B", "C", "N", "Z"] {
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .filter(|line| line.text == label)
                .count(),
            1,
            "source text {label} must survive exactly once"
        );
    }
}

#[test]
fn structural_table_shadows_use_one_box_per_row_or_group_without_changing_flow() {
    for (selector, count, height) in [("tr", 2, 25.0), ("tbody", 1, 50.0)] {
        for (model, shadow) in [
            ("separate", "4pt 3pt 0 2pt red"),
            ("separate", "inset 0 0 0 3pt red"),
            ("collapse", "inset 0 0 0 3pt red"),
        ] {
            let css = format!(
                "{PROFILE}table{{width:100pt;table-layout:fixed;border-collapse:{model};border-spacing:0}}td{{padding:0;height:25pt}}p{{margin:0}}"
            );
            let body = "<table><tbody><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></tbody></table><p>After</p>";
            let control = render_html(&format!("<style>{css}</style>{body}"), &local_options())
                .unwrap()
                .layout
                .unwrap();
            let output = render_html(
                &format!("<style>{css}{selector}{{box-shadow:{shadow}}}</style>{body}"),
                &local_options(),
            )
            .unwrap_or_else(|error| panic!("{selector}/{model}/{shadow}: {error}"))
            .layout
            .unwrap();
            let paints = output
                .display_lists
                .iter()
                .flatten()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint) if !paint.decoration.shadows.is_empty() => Some(paint),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                paints.len(),
                count,
                "{selector}/{model}: shadow duplicated across cells"
            );
            for paint in paints {
                assert_pt(paint.rect.width, 100.0);
                assert_pt(paint.rect.height, height);
            }
            let positions = |trace: &ferri_pdf::LayoutTrace| {
                trace
                    .pages
                    .iter()
                    .flatten()
                    .map(|line| (line.text.clone(), line.x, line.baseline))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                positions(&output),
                positions(&control),
                "{selector}/{model}: shadow changed flow"
            );
        }
    }
}

#[test]
fn row_and_group_border_images_paint_their_boxes_only_in_the_separated_model() {
    let image = "border-image:linear-gradient(red,red) 1 / 3pt";
    let body = "<table><tbody><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></tbody></table><p>After</p>";
    for (selector, rects) in [
        ("tr", vec![(36.0, 25.0), (61.0, 25.0)]),
        ("tbody", vec![(36.0, 50.0)]),
    ] {
        for model in ["separate", "collapse"] {
            let css = format!(
                "{PROFILE}table{{width:100pt;table-layout:fixed;border-collapse:{model};border-spacing:0}}td{{padding:0;height:25pt}}p{{margin:0}}"
            );
            let control = render_html(&format!("<style>{css}</style>{body}"), &local_options())
                .unwrap()
                .layout
                .unwrap();
            let output = render_html(
                &format!("<style>{css}{selector}{{{image}}}</style>{body}"),
                &local_options(),
            )
            .unwrap_or_else(|error| panic!("{selector}/{model}: {error}"))
            .layout
            .unwrap();
            let mut painted = output
                .display_lists
                .iter()
                .flatten()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.border_image.is_drawn() => {
                        Some(paint.rect)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            painted.sort_by(|left, right| left.y.total_cmp(&right.y));
            let expected = if model == "collapse" {
                Vec::new()
            } else {
                rects.clone()
            };
            assert_eq!(
                painted.len(),
                expected.len(),
                "{selector}/{model}: border image boxes {painted:?}"
            );
            for (rect, (y, height)) in painted.iter().zip(expected) {
                assert_pt(rect.x, 36.0);
                assert_pt(rect.width, 100.0);
                assert_pt(rect.y, y);
                assert_pt(rect.height, height);
            }
            let positions = |trace: &ferri_pdf::LayoutTrace| {
                trace
                    .pages
                    .iter()
                    .flatten()
                    .map(|line| (line.text.clone(), line.x, line.baseline))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                positions(&output),
                positions(&control),
                "{selector}/{model}: a border image must not change the flow"
            );
        }
    }
}

#[test]
fn split_row_group_border_image_slices_across_pages() {
    let html = format!(
        "<style>{PROFILE}@page{{size:200pt 100pt;margin:10pt}}table{{width:100%;border-spacing:0}}td{{padding:0}}tr{{height:30pt;break-inside:avoid}}tbody{{border-image:linear-gradient(red,red) 1 / 2pt}}</style><table><tbody><tr><td>1</td></tr><tr><td>2</td></tr><tr><td>3</td></tr><tr><td>4</td></tr></tbody></table>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let fragments = trace
        .display_lists
        .iter()
        .map(|page| {
            page.iter()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.border_image.is_drawn() => {
                        Some((paint.rect, paint.first_fragment, paint.last_fragment))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        fragments.iter().map(Vec::len).collect::<Vec<_>>(),
        [1, 1],
        "one group fragment per page: {fragments:?}"
    );
    let (first, starts, ends) = fragments[0][0];
    assert!(
        starts && !ends,
        "first fragment is open at its end: {fragments:?}"
    );
    assert_pt(first.height, 60.0);
    let (second, starts, ends) = fragments[1][0];
    assert!(
        !starts && ends,
        "second fragment is open at its start: {fragments:?}"
    );
    assert_pt(second.y, 10.0);
    assert_pt(second.height, 60.0);
}

#[test]
fn row_shadow_is_retained_when_all_its_cells_start_in_an_earlier_row() {
    let html = format!(
        "<style>{PROFILE}table{{width:100pt;border-spacing:0}}tr{{height:40pt}}td{{padding:0}}.shadow{{box-shadow:inset 0 0 0 3pt red}}</style><table><tbody><tr><td rowspan=2>Span</td></tr><tr class=shadow></tr></tbody></table>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let shadows = trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if !paint.decoration.shadows.is_empty() => Some(paint.rect),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        shadows.len(),
        1,
        "a row box exists even without an originating cell"
    );
    assert_pt(shadows[0].y, 76.0);
    assert_pt(shadows[0].height, 40.0);
    assert_pt(shadows[0].width, 100.0);
}

#[test]
fn structural_shadows_follow_repeated_groups_and_named_page_widths() {
    let html = format!(
        "<style>{PROFILE}@page{{size:200pt 100pt;margin:10pt}}@page wide{{size:240pt 100pt;margin:10pt}}table{{width:100%;border-spacing:0}}td,th{{padding:0;font-size:1pt;line-height:1pt}}thead,tfoot{{box-shadow:0 0 0 2pt red}}thead tr,tfoot tr{{height:10pt}}tbody tr{{height:30pt;break-inside:avoid}}.wide,tfoot{{page:wide}}</style><table><thead><tr><th></th></tr></thead><tbody><tr><td></td></tr><tr><td></td></tr><tr class=wide><td></td></tr><tr class=wide><td></td></tr></tbody><tfoot><tr><td></td></tr></tfoot></table>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let control = render_html(
        &html.replace("box-shadow:0 0 0 2pt red", "box-shadow:none"),
        &local_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        trace.display_lists.len(),
        control.display_lists.len(),
        "shadows must not add pages"
    );
    assert_eq!(
        trace.display_lists.len(),
        2,
        "unexpected fixture geometry: {:?}",
        trace
            .display_lists
            .iter()
            .map(|page| page
                .iter()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint) if !paint.decoration.shadows.is_empty() =>
                        Some(paint.rect),
                    _ => None,
                })
                .collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
    for (page, width) in [180.0, 220.0].into_iter().enumerate() {
        let mut shadows = trace.display_lists[page]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Box(paint) if !paint.decoration.shadows.is_empty() => Some(paint),
                _ => None,
            })
            .collect::<Vec<_>>();
        shadows.sort_by(|left, right| left.rect.y.total_cmp(&right.rect.y));
        assert_eq!(
            shadows.len(),
            2,
            "page {} needs one header and footer shadow",
            page + 1
        );
        for (paint, y) in shadows.into_iter().zip([10.0, 80.0]) {
            assert_pt(paint.rect.x, 10.0);
            assert_pt(paint.rect.y, y);
            assert_pt(paint.rect.width, width);
            assert_pt(paint.rect.height, 10.0);
            assert!(
                paint.first_fragment && paint.last_fragment,
                "repeated groups need complete decoration edges"
            );
        }
    }
}
