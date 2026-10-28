mod common;

use common::{PROFILE, assert_pt, local_options};
use ferri_pdf::{DisplayItem, LayoutTrace, render_html};

const SHIFT_X: f32 = 13.0;
const SHIFT_Y: f32 = 9.0;

fn render(body: &str, css: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}p{{margin:0}}table{{width:160pt;border-spacing:0}}td{{padding:3pt;border:2pt solid}}{css}</style>{body}"),
        &local_options(),
    )
    .expect("positioned internal table boxes must render")
    .layout
    .unwrap()
}

fn line(trace: &LayoutTrace, text: &str) -> (usize, f32, f32) {
    let matches: Vec<_> = trace
        .pages
        .iter()
        .enumerate()
        .flat_map(|(page, lines)| {
            lines
                .iter()
                .filter(move |line| line.text == text)
                .map(move |line| (page, line.x, line.baseline))
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected one painted line {text:?}, got {matches:?}"
    );
    matches[0]
}

#[test]
fn internal_relative_offsets_follow_originating_rows_and_accumulate_once() {
    let body = "<table><tbody class=group><tr class=first><td class=span rowspan=2>A<br>A2</td><td>B</td></tr><tr class=second><td>C</td></tr></tbody><tbody><tr><td>D</td><td>E</td></tr></tbody></table><p>After</p>";
    for collapse in ["separate", "collapse"] {
        let base = format!("table{{border-collapse:{collapse}}}");
        let normal = render(body, &base);
        for (selector, moved) in [
            (".span", &["A", "A2"][..]),
            (".first", &["A", "A2", "B"][..]),
            (".second", &["C"][..]),
            (".group", &["A", "A2", "B", "C"][..]),
        ] {
            let shifted = render(
                body,
                &format!("{base}{selector}{{position:relative;left:{SHIFT_X}pt;top:{SHIFT_Y}pt}}"),
            );
            assert_eq!(shifted.pages.len(), normal.pages.len());
            for text in ["A", "A2", "B", "C", "D", "E", "After"] {
                let (page, x, y) = line(&normal, text);
                let (shifted_page, shifted_x, shifted_y) = line(&shifted, text);
                assert_eq!(
                    shifted_page, page,
                    "{selector}/{collapse}: {text} retains its flow page"
                );
                let factor = if moved.contains(&text) { 1.0 } else { 0.0 };
                assert_pt(shifted_x - x, SHIFT_X * factor);
                assert_pt(shifted_y - y, SHIFT_Y * factor);
            }
        }
        let nested = render(
            body,
            &format!(
                "{base}.group,.first,.span{{position:relative;left:{SHIFT_X}pt;top:{SHIFT_Y}pt}}"
            ),
        );
        for (text, factor) in [
            ("A", 3.0),
            ("A2", 3.0),
            ("B", 2.0),
            ("C", 1.0),
            ("D", 0.0),
            ("E", 0.0),
            ("After", 0.0),
        ] {
            let (_, x, y) = line(&normal, text);
            let (_, shifted_x, shifted_y) = line(&nested, text);
            assert_pt(shifted_x - x, SHIFT_X * factor);
            assert_pt(shifted_y - y, SHIFT_Y * factor);
        }
    }
}

#[test]
fn cell_background_and_text_share_offsets_in_whole_and_fragmented_rows() {
    for collapse in ["separate", "collapse"] {
        for count in [1, 40] {
            let content = (0..count)
                .map(|index| format!("Cell{index}<br>"))
                .collect::<String>();
            let body = format!(
                "<table><tr><td class=move>{content}</td><td>Other</td></tr></table><p>After</p>"
            );
            let base = format!(
                "@page{{size:240pt 160pt;margin:20pt}}table{{border-collapse:{collapse}}}.move{{background:red}}"
            );
            let normal = render(&body, &base);
            let shifted = render(
                &body,
                &format!("{base}.move{{position:relative;left:{SHIFT_X}pt;top:{SHIFT_Y}pt}}"),
            );
            assert_eq!(normal.pages.len(), shifted.pages.len());
            if count > 1 {
                assert!(normal.pages.len() > 1, "long fixture must fragment");
            }
            for (before, after) in normal.display_lists.iter().zip(&shifted.display_lists) {
                let red = |items: &[DisplayItem]| {
                    items
                        .iter()
                        .filter_map(|item| match item {
                            DisplayItem::Box(paint)
                                if paint.decoration.background.rgb == [u8::MAX, 0, 0]
                                    && paint.decoration.background.alpha > 0.9 =>
                            {
                                Some(paint.rect)
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                };
                let before = red(before);
                let after = red(after);
                assert_eq!(before.len(), after.len());
                for (before, after) in before.iter().zip(&after) {
                    assert_pt(after.x - before.x, SHIFT_X);
                    assert_pt(after.y - before.y, SHIFT_Y);
                    assert_pt(after.width, before.width);
                    assert_pt(after.height, before.height);
                }
            }
            for index in 0..count {
                let text = format!("Cell{index}");
                let (page, x, y) = line(&normal, &text);
                let (shifted_page, shifted_x, shifted_y) = line(&shifted, &text);
                assert_eq!(page, shifted_page);
                assert_pt(shifted_x - x, SHIFT_X);
                assert_pt(shifted_y - y, SHIFT_Y);
            }
            assert_eq!(line(&normal, "After"), line(&shifted, "After"));
        }
    }
}

#[test]
fn positioned_cell_containing_block_uses_assigned_height_before_vertical_alignment() {
    let body = "<table><tr><td class=container>A<span class=anchor>X</span></td><td>B<br>C<br>Z</td></tr></table>";
    for align in ["top", "middle", "bottom"] {
        let trace = render(
            body,
            &format!(
                ".container{{position:relative;vertical-align:{align}}}.anchor{{position:absolute;left:0;bottom:0}}"
            ),
        );
        let (_, anchor_x, anchor_y) = line(&trace, "X");
        let (_, cell_x, _) = line(&trace, "A");
        let (_, _, last_y) = line(&trace, "Z");
        const CELL_PADDING: f32 = 3.0;
        assert_pt(anchor_x, cell_x - CELL_PADDING);
        assert_pt(anchor_y, last_y + CELL_PADDING);
    }
}

#[test]
fn relative_row_and_group_establish_containing_blocks_for_cell_descendants() {
    let body = "<p>Before</p><table><tbody><tr><td>A<span class=anchor>X</span></td><td>B</td></tr></tbody></table>";
    for selector in ["tr", "tbody"] {
        let trace = render(
            body,
            &format!("{selector}{{position:relative}}.anchor{{position:absolute;left:0;top:0}}"),
        );
        let (_, ax, ay) = line(&trace, "A");
        let (_, x, y) = line(&trace, "X");
        const CELL_INSET: f32 = 5.0;
        assert_pt(x, ax - CELL_INSET);
        assert_pt(y, ay - CELL_INSET);
    }
}

#[test]
fn row_and_group_percentage_insets_use_final_assigned_rectangles() {
    let body = "<p>Before</p><table><tbody><tr><td>A</td><td>B</td></tr><tr><td>C<span class=anchor>X</span></td><td>D</td></tr></tbody></table>";
    for selector in ["tr", "tbody"] {
        let trace = render(
            body,
            &format!(
                "{selector}{{position:relative;left:{SHIFT_X}pt;top:{SHIFT_Y}pt}}.anchor{{position:absolute;left:50%;top:50%}}"
            ),
        );
        let (_, ax, ay) = line(&trace, "A");
        let (_, _, cy) = line(&trace, "C");
        let (_, x, y) = line(&trace, "X");
        const CELL_INSET: f32 = 5.0;
        const HALF_TABLE_WIDTH: f32 = 80.0;
        assert_pt(x, ax - CELL_INSET + HALF_TABLE_WIDTH);
        let half_height = if selector == "tr" {
            (cy - ay) / 2.0
        } else {
            0.0
        };
        assert_pt(y, cy - CELL_INSET + half_height);
    }
}

#[test]
fn fragmented_group_anchors_resolve_on_their_own_page() {
    let rows = (0..30)
        .map(|index| format!("<tr><td>Row{index}<span class=anchor>Anchor{index}</span></td></tr>"))
        .collect::<String>();
    let trace = render(
        &format!("<table><tbody>{rows}</tbody></table>"),
        "@page{size:240pt 160pt;margin:20pt}tbody{position:relative}.anchor{position:absolute;left:0;top:0}",
    );
    assert!(trace.pages.len() > 1, "fixture must fragment its row group");
    for index in 0..30 {
        let (page, _, _) = line(&trace, &format!("Row{index}"));
        let (anchor_page, x, y) = line(&trace, &format!("Anchor{index}"));
        assert_eq!(anchor_page, page);
        let first = trace.pages[page]
            .iter()
            .filter(|line| line.text.starts_with("Row"))
            .min_by(|a, b| a.baseline.total_cmp(&b.baseline))
            .unwrap();
        const CELL_INSET: f32 = 5.0;
        assert_pt(x, first.x - CELL_INSET);
        assert_pt(y, first.baseline - CELL_INSET);
    }
}

#[test]
fn internal_percentage_offsets_reflow_on_alternating_page_widths_and_obey_direction() {
    let lines = (0..40)
        .map(|index| format!("Line{index}"))
        .collect::<Vec<_>>()
        .join("<br>");
    let body = format!("<table><tbody><tr><td>{lines}</td></tr></tbody></table>");
    for direction in ["ltr", "rtl"] {
        let base = format!(
            "@page{{size:240pt 160pt;margin:20pt}}@page:left{{size:400pt 160pt}}table{{width:100%;direction:{direction}}}td{{padding:0;border:0}}"
        );
        let normal = render(&body, &base);
        assert!(normal.pages.len() > 1);
        for selector in ["td", "tr", "tbody"] {
            let shifted = render(
                &body,
                &format!(
                    "{base}{selector}{{position:relative;left:10%;right:20%;top:{SHIFT_Y}pt}}"
                ),
            );
            assert_eq!(shifted.pages.len(), normal.pages.len());
            for index in 0..40 {
                let text = format!("Line{index}");
                let (page, x, y) = line(&normal, &text);
                let (actual_page, actual_x, actual_y) = line(&shifted, &text);
                assert_eq!(
                    actual_page, page,
                    "{selector}/{direction}: {text} flow page"
                );
                const PAGE_MARGINS: f32 = 40.0;
                let width = normal.page_records[page].geometry.paper[0] - PAGE_MARGINS;
                let fraction = if direction == "rtl" { -0.2 } else { 0.1 };
                assert_pt(actual_x - x, width * fraction);
                assert_pt(actual_y - y, SHIFT_Y);
            }
        }
    }
}

#[test]
fn internal_destinations_follow_relative_paint_in_whole_and_split_rows() {
    for count in [1, 40] {
        let content = (0..count)
            .map(|index| format!("Cell{index}<br>"))
            .collect::<String>();
        for selector in ["td", "tr", "tbody"] {
            let body = format!(
                "<a href='#target'>Jump</a><table><tbody{}><tr{}><td{}>{content}</td></tr></tbody></table>",
                if selector == "tbody" {
                    " id=target"
                } else {
                    ""
                },
                if selector == "tr" { " id=target" } else { "" },
                if selector == "td" { " id=target" } else { "" }
            );
            let css = "@page{size:240pt 160pt;margin:20pt}";
            let normal = render(&body, css);
            let moved = render(
                &body,
                &format!("{css}{selector}{{position:relative;top:{SHIFT_Y}pt}}"),
            );
            assert_eq!(normal.destinations.len(), 1);
            let before = normal.destinations[0].expect("normal target must exist");
            let after = moved.destinations[0].expect("shifted target must exist");
            assert_eq!(before.page, after.page);
            assert_pt(after.top - before.top, SHIFT_Y);
        }
    }
}

#[test]
fn relative_rows_move_their_background_layers_with_originating_cells() {
    for count in [1, 40] {
        let content = (0..count)
            .map(|index| format!("Cell{index}<br>"))
            .collect::<String>();
        let body = format!(
            "<table><tbody><tr><td>{content}</td><td>B</td></tr></tbody></table><p>After</p>"
        );
        for selector in ["tr", "tbody"] {
            let css = format!("@page{{size:240pt 160pt;margin:20pt}}{selector}{{background:blue}}");
            let normal = render(&body, &css);
            let paints = |trace: &LayoutTrace| {
                trace
                    .display_lists
                    .iter()
                    .enumerate()
                    .flat_map(|(page, items)| {
                        items.iter().filter_map(move |item| match item {
                            DisplayItem::Box(paint)
                                if paint.decoration.background.rgb == [0, 0, u8::MAX]
                                    && paint.decoration.background.alpha > 0.9 =>
                            {
                                Some((page, paint.rect))
                            }
                            _ => None,
                        })
                    })
                    .collect::<Vec<_>>()
            };
            let before = paints(&normal);
            assert!(
                !before.is_empty(),
                "fixture must paint the structural background"
            );
            if count > 1 {
                assert!(
                    before.iter().any(|(page, _)| *page > 0),
                    "background must span pages"
                );
            }
            for shifted_selector in [selector, "td"] {
                let moved = render(
                    &body,
                    &format!(
                        "{css}{shifted_selector}{{position:relative;left:{SHIFT_X}pt;top:{SHIFT_Y}pt}}"
                    ),
                );
                let after = paints(&moved);
                assert_eq!(before.len(), after.len());
                let factor = if shifted_selector == selector {
                    1.0
                } else {
                    0.0
                };
                for ((before_page, before), (after_page, after)) in before.iter().zip(&after) {
                    assert_eq!(before_page, after_page);
                    assert_pt(after.x - before.x, SHIFT_X * factor);
                    assert_pt(after.y - before.y, SHIFT_Y * factor);
                    assert_pt(after.width, before.width);
                    assert_pt(after.height, before.height);
                }
            }
        }
    }
}
