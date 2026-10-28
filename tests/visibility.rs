mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{DisplayItem, LayoutTrace, render_html};

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<!doctype html><style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    )
    .expect("visibility fixture must render")
    .layout
    .unwrap()
}

fn painted_text(trace: &LayoutTrace) -> Vec<String> {
    trace
        .display_lists
        .iter()
        .enumerate()
        .flat_map(|(page, items)| {
            items.iter().filter_map(move |item| match item {
                DisplayItem::Text { line } => Some(
                    trace.pages[page][*line]
                        .runs
                        .iter()
                        .map(|run| run.text.as_str())
                        .collect::<String>(),
                ),
                _ => None,
            })
        })
        .collect()
}

#[test]
fn hidden_blocks_preserve_space_and_allow_visible_descendants() {
    let body = "<div class=hidden><p>Hidden</p><p class=visible>Visible</p></div><p>After</p>";
    let base = "p{margin:0}.hidden{height:80pt;background:red;border:2pt solid blue}";
    let normal = trace(base, body);
    let hidden = trace(
        &format!("{base}.hidden{{visibility:hidden}}.visible{{visibility:visible}}"),
        body,
    );
    assert_eq!(painted_text(&hidden), ["Visible", "After"]);
    for label in ["Visible", "After"] {
        let locate = |output: &LayoutTrace| {
            output
                .pages
                .iter()
                .flatten()
                .find(|line| line.text == label)
                .map(|line| (line.x, line.baseline))
                .unwrap()
        };
        assert_eq!(
            locate(&hidden),
            locate(&normal),
            "{label}: visibility changed layout"
        );
    }
    assert!(!hidden.display_lists.iter().flatten().any(|item| matches!(item, DisplayItem::Box(paint) if paint.decoration.background.rgb == [255,0,0] && paint.decoration.background.alpha > 0.0)));
}

#[test]
fn hidden_inline_runs_preserve_advances_without_painting_their_text() {
    let body = "<p>Before<span>Hidden</span>After</p>";
    let normal = trace("", body);
    let hidden = trace("span{visibility:hidden}", body);
    let text = painted_text(&hidden).join("");
    assert!(!text.contains("Hidden"), "hidden text painted: {text}");
    assert!(text.contains("Before") && text.contains("After"));
    let after = |output: &LayoutTrace| {
        output
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.runs.iter().map(move |run| (line.x + run.x, &run.text)))
            .find(|(_, text)| text.contains("After"))
            .map(|(x, _)| x)
            .unwrap()
    };
    common::assert_pt(after(&hidden), after(&normal));
}

#[test]
fn hidden_table_layers_and_borders_do_not_hide_explicitly_visible_cells() {
    for borders in ["separate", "collapse"] {
        let output = trace(
            &format!(
                "table{{visibility:hidden;border-collapse:{borders};border:5pt solid red}}tbody{{background:red}}tr{{background:red}}td{{border:5pt solid red}}.visible{{visibility:visible;background:green;border:6pt solid green}}"
            ),
            "<table><tbody><tr><td>Hidden</td><td class=visible>Visible</td></tr></tbody></table>",
        );
        assert_eq!(painted_text(&output), ["Visible"], "{borders}");
        for item in output.display_lists.iter().flatten() {
            if let DisplayItem::Box(paint) = item {
                assert!(
                    paint.decoration.background.alpha == 0.0
                        || paint.decoration.background.rgb != [255, 0, 0],
                    "hidden {borders} background painted"
                );
                if borders == "separate" {
                    assert!(
                        !paint
                            .decoration
                            .borders
                            .iter()
                            .any(|border| border.used_width() > 0.0
                                && border.color.rgb == [255, 0, 0]),
                        "hidden separated border painted"
                    );
                }
            }
        }
    }
}

#[test]
fn hidden_links_are_absent_and_visible_descendant_links_remain() {
    let output = trace(
        ".hidden{visibility:hidden}.visible{visibility:visible}",
        "<a class=hidden href='https://example.org/hidden'>Hidden</a><div class=hidden><a class=visible href='https://example.org/visible'>Visible</a></div>",
    );
    assert_eq!(painted_text(&output), ["Visible"]);
    assert_eq!(output.links.iter().map(Vec::len).sum::<usize>(), 1);
}

#[test]
fn hidden_body_preserves_canvas_background_and_initial_reset() {
    let output = trace(
        "body{visibility:hidden;background:red}.initial{visibility:initial}.unset{visibility:unset}",
        "<p class=unset>Hidden</p><p class=initial>Visible</p>",
    );
    assert_eq!(painted_text(&output), ["Visible"]);
    assert!(output.display_lists.iter().flatten().any(|item| matches!(item, DisplayItem::Box(paint) if paint.decoration.background.rgb == [255,0,0] && paint.decoration.background.alpha > 0.0)), "hidden body must still provide the canvas background");
}

#[test]
fn fully_hidden_collapsed_table_keeps_geometry_without_border_ink() {
    let body = "<table><tr><td>Hidden</td></tr></table><p>After</p>";
    let css = "table{border-collapse:collapse;border:20pt solid red}td{width:80pt;height:80pt}";
    let normal = trace(css, body);
    let hidden = trace(&format!("{css}table{{visibility:hidden}}"), body);
    assert_eq!(painted_text(&hidden), ["After"]);
    assert!(!hidden.display_lists.iter().flatten().any(|item| matches!(item, DisplayItem::Box(paint) if paint.decoration.borders.iter().any(|border| border.used_width() > 0.0))));
    let after = |output: &LayoutTrace| {
        output
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "After")
            .unwrap()
            .baseline
    };
    common::assert_pt(after(&hidden), after(&normal));
}

#[test]
fn collapsed_flex_item_removes_main_space_and_retains_line_cross_size() {
    let body = "<section><div>A</div><div class=collapsed><span class=visible>Hidden</span></div><div>B</div></section><p>After</p>";
    let css = "p{margin:0}section{display:flex;width:180pt;gap:10pt;align-items:start}section>div{width:40pt;flex-shrink:0}.collapsed{height:70pt}.visible{visibility:visible}";
    let normal = trace(css, body);
    let collapsed = trace(&format!("{css}.collapsed{{visibility:collapse}}"), body);
    assert_eq!(
        painted_text(&collapsed),
        ["A", "B", "After"],
        "collapsed flex subtree cannot be made visible by descendants"
    );
    let locate = |output: &LayoutTrace, label: &str| {
        let line = output
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == label)
            .unwrap();
        (line.x, line.baseline)
    };
    common::assert_pt(locate(&collapsed, "B").0 - locate(&collapsed, "A").0, 50.0);
    common::assert_pt(locate(&collapsed, "After").1, locate(&normal, "After").1);
}

#[test]
fn collapsed_flex_rewrap_uses_original_line_strut() {
    let body =
        "<section><div>A</div><div class=collapsed>Hidden</div><div>B</div></section><p>After</p>";
    let css = "p{margin:0}section{display:flex;flex-wrap:wrap;width:90pt;gap:10pt;align-items:start}section>div{width:40pt;flex-shrink:0}.collapsed{height:70pt;visibility:collapse}";
    let output = trace(css, body);
    let a = output.pages[0]
        .iter()
        .find(|line| line.text == "A")
        .unwrap();
    let b = output.pages[0]
        .iter()
        .find(|line| line.text == "B")
        .unwrap();
    let after = output.pages[0]
        .iter()
        .find(|line| line.text == "After")
        .unwrap();
    common::assert_pt(a.baseline, b.baseline);
    common::assert_pt(b.x - a.x, 50.0);
    common::assert_pt(after.baseline - a.baseline, 70.0);
}

#[test]
fn collapsed_flex_subtrees_preserve_counters_and_running_strings() {
    let output = trace(
        "@page{@top-left{content:'Header:' string(label)}}body{counter-reset:n}section{display:flex}.collapsed{visibility:collapse;counter-increment:n}span{string-set:label content(text)}p::before{content:counter(n) ':'}",
        "<section><div>A</div><div class=collapsed><span>Retained</span><div style='display:inline-block;string-set:label content(text)'>Nested</div></div><div>B</div></section><p>After</p>",
    );
    let text = painted_text(&output).join("|");
    assert!(
        text.contains("1:After"),
        "collapsed counter increment lost: {text}"
    );
    assert!(
        text.contains("Header:Retained"),
        "collapsed string assignment lost: {text}"
    );
    assert!(
        !text.contains("|Nested|"),
        "collapsed atomic descendant painted: {text}"
    );
}

#[test]
fn collapsed_flex_columns_remove_main_extent_in_both_directions() {
    for direction in ["column", "column-reverse"] {
        let output = trace(
            &format!(
                "p{{margin:0}}section{{display:flex;flex-direction:{direction};gap:10pt}}section>div{{height:20pt;flex-shrink:0}}.collapsed{{visibility:collapse;height:70pt}}"
            ),
            "<section><div>A</div><div class=collapsed>Hidden</div><div>B</div></section><p>After</p>",
        );
        let baseline = |label| {
            output.pages[0]
                .iter()
                .find(|line| line.text == label)
                .unwrap()
                .baseline
        };
        common::assert_pt((baseline("B") - baseline("A")).abs(), 30.0);
        common::assert_pt(baseline("After") - baseline("A").min(baseline("B")), 50.0);
    }
}

#[test]
fn collapsed_column_assignment_follows_the_preceding_fragmented_item() {
    let body = format!(
        "<section><div>{}</div><div class=collapsed>Hidden</div><div>After</div></section>",
        "<p>Line</p>".repeat(9)
    );
    let output = trace(
        "@page{size:240pt 100pt;margin:10pt;@top-left{content:'Header:' string(label,last)}}p{margin:0}body{font-size:10pt;line-height:14pt}section{display:flex;flex-direction:column}section>div{flex-shrink:0}.collapsed{visibility:collapse;string-set:label 'Middle'}",
        &body,
    );
    let headers: Vec<_> = output
        .pages
        .iter()
        .map(|page| {
            page.iter()
                .find(|line| line.text.starts_with("Header:"))
                .unwrap()
                .text
                .as_str()
        })
        .collect();
    assert_eq!(
        headers[0], "Header:",
        "collapsed assignment was committed before its source position: {headers:?}"
    );
    assert_eq!(headers[1], "Header:Middle");
    assert_eq!(
        headers.len(),
        2,
        "collapsed event must not add layout space: {headers:?}"
    );
    assert!(output.pages[1].iter().any(|line| line.text == "After"));
}

#[test]
fn fully_collapsed_flex_line_retains_its_strut_without_descendant_ink() {
    let output = trace(
        "p{margin:0}section{display:flex}section>div{visibility:collapse;height:70pt;width:40pt}span{visibility:visible}",
        "<p>Before</p><section><div><span>Hidden</span></div></section><p>After</p>",
    );
    assert_eq!(painted_text(&output), ["Before", "After"]);
    let before = output.pages[0]
        .iter()
        .find(|line| line.text == "Before")
        .unwrap();
    let after = output.pages[0]
        .iter()
        .find(|line| line.text == "After")
        .unwrap();
    common::assert_pt(after.baseline - before.baseline, before.height + 70.0);
}

#[test]
fn collapsed_inline_flex_item_does_not_contribute_main_axis_intrinsic_width() {
    let output = trace(
        "section{display:inline-flex;background:red;gap:10pt}section>div{width:40pt;flex-shrink:0}.collapsed{visibility:collapse;width:140pt;height:70pt}",
        "<section><div>A</div><div class=collapsed>Hidden</div><div>B</div></section>",
    );
    let rect = output
        .display_lists
        .iter()
        .flatten()
        .find_map(|item| match item {
            DisplayItem::Box(paint)
                if paint.decoration.background.rgb == [255, 0, 0]
                    && paint.decoration.background.alpha > 0.0 =>
            {
                Some(paint.rect)
            }
            _ => None,
        })
        .unwrap();
    common::assert_pt(rect.width, 90.0);
    common::assert_pt(rect.height, 70.0);
}

#[test]
fn collapsed_flex_strut_fragments_without_painting_or_losing_occupied_pages() {
    let output = trace(
        "@page{size:240pt 100pt;margin:10pt}p{margin:0}section{display:flex}.collapsed{visibility:collapse;height:180pt}",
        "<section><div class=collapsed>Hidden</div></section><p>After</p>",
    );
    assert_eq!(output.pages.len(), 3);
    assert_eq!(painted_text(&output), ["After"]);
    assert!(output.pages[2].iter().any(|line| line.text == "After"));
}

#[test]
fn collapsed_wrapped_column_assignment_keeps_its_fragment_page() {
    let body = format!(
        "<section><div>{}</div><div class=collapsed>Hidden</div><div>After</div><div class=second>Other column</div></section>",
        "<p>Line</p>".repeat(9)
    );
    let output = trace(
        "@page{size:240pt 100pt;margin:10pt;@top-left{content:'Header:' string(label,last)}}p{margin:0}body{font-size:10pt;line-height:14pt}section{display:flex;flex-direction:column;flex-wrap:wrap;height:140pt;width:180pt}section>div{flex-shrink:0;width:80pt}.collapsed{visibility:collapse;string-set:label 'Middle'}.second{height:140pt}",
        &body,
    );
    let headers: Vec<_> = output
        .pages
        .iter()
        .map(|page| {
            page.iter()
                .find(|line| line.text.starts_with("Header:"))
                .unwrap()
                .text
                .as_str()
        })
        .collect();
    assert_eq!(
        headers,
        ["Header:", "Header:Middle"],
        "collapsed source was assigned to the wrong fragment"
    );
    assert_eq!(
        painted_text(&output)
            .iter()
            .filter(|text| *text == "After")
            .count(),
        1
    );
}

#[test]
fn collapsed_table_row_removes_space_but_preserves_column_constraints() {
    let body = "<table><tr class=gone><td>Wide intrinsic column</td><td>Hidden</td></tr><tr><td>A</td><td>B</td></tr></table><p>After</p>";
    let css = "p{margin:0}table{border-spacing:0}td{padding:0;height:40pt}";
    let normal = trace(css, body);
    let collapsed = trace(&format!("{css}.gone{{visibility:collapse}}"), body);
    let locate = |output: &LayoutTrace, label: &str| {
        let line = output.pages[0]
            .iter()
            .find(|line| line.text == label)
            .unwrap();
        (line.x, line.baseline)
    };
    for label in ["A", "B", "After"] {
        common::assert_pt(locate(&collapsed, label).0, locate(&normal, label).0);
        common::assert_pt(locate(&normal, label).1 - locate(&collapsed, label).1, 40.0);
    }
    assert_eq!(painted_text(&collapsed), ["A", "B", "After"]);
}

#[test]
fn collapsed_table_column_removes_only_its_sized_track_and_spacing() {
    let body =
        "<table><col><col class=gone><col><tr><td>A</td><td>Hidden</td><td>B</td></tr></table>";
    let css =
        "table{table-layout:fixed;width:196pt;border-spacing:4pt 0}col{width:60pt}td{padding:0}";
    let normal = trace(css, body);
    let collapsed = trace(&format!("{css}.gone{{visibility:collapse}}"), body);
    let x = |output: &LayoutTrace, label: &str| {
        output.pages[0]
            .iter()
            .find(|line| line.text == label)
            .unwrap()
            .x
    };
    common::assert_pt(x(&collapsed, "A"), x(&normal, "A"));
    common::assert_pt(x(&normal, "B") - x(&collapsed, "B"), 64.0);
    assert_eq!(painted_text(&collapsed), ["A", "B"]);
}

#[test]
fn visible_rowspan_is_clipped_to_the_remaining_rows_after_collapse() {
    let output = trace(
        "table{border-spacing:0}tr{height:40pt}td{padding:0;width:60pt}.gone{visibility:collapse}.span{visibility:visible;background:red}",
        "<table><tr class=gone><td class=span rowspan=2>Top<br>Bottom</td><td>Hidden</td></tr><tr><td>Other</td></tr></table><p>After</p>",
    );
    let rect = output
        .display_lists
        .iter()
        .flatten()
        .find_map(|item| match item {
            DisplayItem::Box(paint)
                if paint.decoration.background.rgb == [255, 0, 0]
                    && paint.decoration.background.alpha > 0.0 =>
            {
                Some(paint.rect)
            }
            _ => None,
        })
        .unwrap();
    common::assert_pt(rect.height, 40.0);
    assert!(painted_text(&output).iter().any(|text| text == "Top"));
}

#[test]
fn collapsed_colspan_keeps_content_width_and_clips_to_projected_border_box() {
    for direction in ["ltr", "rtl"] {
        let body = "<table><col><col class=gone><tr><td colspan=2>Alpha beta gamma delta</td></tr></table>";
        let css = format!(
            "table{{direction:{direction};table-layout:fixed;width:120pt;border-spacing:0}}col{{width:60pt}}td{{padding:0;background:red;text-align:start}}"
        );
        let normal = trace(&css, body);
        let collapsed = trace(&format!("{css}.gone{{visibility:collapse}}"), body);
        let text = |output: &LayoutTrace| {
            output
                .pages
                .iter()
                .flatten()
                .filter(|line| !line.text.is_empty())
                .map(|line| (line.text.clone(), line.width))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            text(&collapsed),
            text(&normal),
            "{direction}: collapsed columns reflowed the spanning content"
        );
        let projected = collapsed
            .display_lists
            .iter()
            .flatten()
            .find_map(|item| match item {
                DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [255, 0, 0]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint.rect)
                }
                _ => None,
            })
            .unwrap();
        common::assert_pt(projected.width, 60.0);
        assert!(collapsed.display_lists.iter().flatten().any(|item| matches!(item, DisplayItem::PushClip(clip) if (clip.rect.width - 60.0).abs() < 0.001)), "{direction}: spanning content must be clipped");
    }
}

#[test]
fn collapsed_column_group_cannot_be_overridden_by_a_visible_child_column() {
    let output = trace(
        "table{table-layout:fixed;width:180pt;border-spacing:0}col{width:60pt}colgroup.gone{visibility:collapse}col.visible{visibility:visible}td{padding:0}",
        "<table><colgroup class=gone><col class=visible><col></colgroup><colgroup><col></colgroup><tr><td>HiddenOne</td><td>HiddenTwo</td><td>Visible</td></tr></table>",
    );
    assert_eq!(painted_text(&output), ["Visible"]);
    common::assert_pt(
        output.pages[0]
            .iter()
            .find(|line| line.text == "Visible")
            .unwrap()
            .x,
        36.0,
    );
}

#[test]
fn collapsed_rowspan_does_not_paginate_content_beyond_its_reduced_height() {
    let body = format!(
        "<table><tr class=gone><td rowspan=2 class=span>{}</td><td>Hidden</td></tr><tr><td>Visible</td></tr></table><p>After</p>",
        (0..12)
            .map(|index| format!("<p>Line{index}</p>"))
            .collect::<String>()
    );
    let output = trace(
        "@page{size:240pt 100pt;margin:10pt}body{font-size:10pt;line-height:14pt}p{margin:0}table{border-spacing:0}tr{height:90pt}td{padding:0;width:80pt;vertical-align:top}.gone{visibility:collapse}.span{visibility:visible;background:red}",
        &body,
    );
    assert_eq!(
        output.pages.len(),
        2,
        "collapsed span content recreated removed row height"
    );
    assert!(output.pages[1].iter().any(|line| line.text == "After"));
    assert!(
        !painted_text(&output).iter().any(|text| text == "Line11"),
        "content beyond the projected cell escaped clipping"
    );
}

#[test]
fn collapsed_row_group_removes_spacing_despite_visible_row_override() {
    let output = trace(
        "p{margin:0}table{border-spacing:0 4pt}tr{height:30pt}td{padding:0}.gone{visibility:collapse}.visible{visibility:visible}",
        "<p>Before</p><table><tbody class=gone><tr class=visible><td>HiddenOne</td></tr><tr><td>HiddenTwo</td></tr></tbody><tbody><tr><td>Visible</td></tr></tbody></table><p>After</p>",
    );
    assert_eq!(painted_text(&output), ["Before", "Visible", "After"]);
    let before = output.pages[0]
        .iter()
        .find(|line| line.text == "Before")
        .unwrap();
    let after = output.pages[0]
        .iter()
        .find(|line| line.text == "After")
        .unwrap();
    common::assert_pt(after.baseline - before.baseline - before.height, 38.0);
}

#[test]
fn collapsed_rowspan_preserves_percentage_child_containing_height() {
    let output = trace(
        "table{border-spacing:0}tr{height:40pt}td{padding:0;width:60pt;vertical-align:top}.gone{visibility:collapse}.span{visibility:visible;height:80pt}td div{height:50%;background:blue}",
        "<table><tr class=gone><td rowspan=2 class=span><div></div></td><td>Hidden</td></tr><tr><td>Visible</td></tr></table>",
    );
    let height = output
        .display_lists
        .iter()
        .flatten()
        .find_map(|item| match item {
            DisplayItem::Box(paint)
                if paint.decoration.background.rgb == [0, 0, 255]
                    && paint.decoration.background.alpha > 0.0 =>
            {
                Some(paint.rect.height)
            }
            _ => None,
        })
        .unwrap();
    common::assert_pt(height, 40.0);
}

#[test]
fn collapsed_rowspan_border_grid_uses_the_projected_height_once() {
    let css = "table{border-collapse:collapse}tr{height:40pt}td{padding:0;width:60pt;border:2pt solid blue;vertical-align:top}.gone{visibility:collapse}.span{visibility:visible}";
    let normal = trace(css, "<table><tr><td>Top</td></tr></table>");
    let collapsed = trace(
        css,
        "<table><tr class=gone><td rowspan=2 class=span>Top</td></tr><tr></tr></table>",
    );
    let bottom = |output: &LayoutTrace| {
        output
            .display_lists
            .iter()
            .flatten()
            .filter_map(|item| match item {
                DisplayItem::Box(paint)
                    if paint.decoration.borders.iter().any(|border| {
                        border.color.rgb == [0, 0, 255]
                            && border.color.alpha > 0.0
                            && border.used_width() > 0.0
                    }) =>
                {
                    Some(paint.rect.y + paint.rect.height)
                }
                _ => None,
            })
            .fold(f32::NEG_INFINITY, f32::max)
    };
    common::assert_pt(bottom(&collapsed), bottom(&normal));
}

#[test]
fn hidden_paragraphs_preserve_fragment_pages_and_following_content_position() {
    let body = format!(
        "<section class=hidden>{}</section><p>After</p>",
        (0..20)
            .map(|index| format!("<p>Line{index}</p>"))
            .collect::<String>()
    );
    let css = "@page{size:200pt 100pt;margin:10pt}p{margin:0;font-size:10pt;line-height:14pt;widows:1;orphans:1}";
    let visible = trace(css, &body);
    let hidden = trace(&format!("{css}.hidden{{visibility:hidden}}"), &body);
    assert!(
        visible.pages.len() > 2,
        "control must exercise fragmentation"
    );
    assert_eq!(
        hidden.pages.len(),
        visible.pages.len(),
        "hidden content lost occupied pages"
    );
    let locate = |output: &LayoutTrace| {
        output
            .pages
            .iter()
            .enumerate()
            .find_map(|(page, lines)| {
                lines
                    .iter()
                    .find(|line| line.text == "After")
                    .map(|line| (page, line.x, line.baseline))
            })
            .unwrap()
    };
    assert_eq!(locate(&hidden), locate(&visible));
    assert_eq!(painted_text(&hidden), ["After"]);
}

#[test]
fn collapsed_columns_recompute_projected_width_on_alternating_pages() {
    let output = trace(
        "@page{size:240pt 140pt;margin:20pt}@page:left{size:400pt 140pt;margin:20pt}table{width:100%;table-layout:fixed;border-spacing:0}col{width:50%}.gone{visibility:collapse}td{padding:0;vertical-align:top;background:red}.wide{break-before:page}p{margin:0}",
        "<table><col><col class=gone><tr><td colspan=2><p>First</p><p class=wide>Second</p></td></tr></table>",
    );
    assert_eq!(output.pages.len(), 2);
    assert!(output.pages[0].iter().any(|line| line.text == "First"));
    assert!(output.pages[1].iter().any(|line| line.text == "Second"));
    for (page, expected) in [100.0, 180.0].into_iter().enumerate() {
        common::assert_pt(
            output.page_records[page].geometry.paper[0],
            expected * 2.0 + 40.0,
        );
        let widths = output.display_lists[page]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [255, 0, 0]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint.rect.width)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            !widths.is_empty(),
            "page {page}: spanning cell paint missing"
        );
        for width in widths {
            common::assert_pt(width, expected);
        }
    }
}

#[test]
fn collapsed_rows_preserve_repeated_groups_and_visible_row_positions() {
    for borders in ["separate", "collapse"] {
        let css = format!(
            "@page{{size:240pt 120pt;margin:10pt}}table{{width:120pt;table-layout:fixed;border-spacing:0;border-collapse:{borders}}}td{{padding:0;font-size:10pt;line-height:14pt}}tbody tr{{height:40pt}}.gone{{visibility:collapse}}"
        );
        let rows = (0..5)
            .map(|index| format!("<tr><td>Row{index}</td></tr>"))
            .collect::<Vec<_>>();
        let make_body = |rows: String| {
            format!(
                "<table><thead><tr><td>Header</td></tr></thead><tbody>{rows}</tbody><tfoot><tr><td>Footer</td></tr></tfoot></table><p>After</p>"
            )
        };
        let control = trace(&css, &make_body(rows.join("")));
        let collapsed = trace(
            &css,
            &make_body(rows.join("<tr class=gone><td>Hidden</td></tr>")),
        );
        assert!(control.pages.len() > 2);
        assert_eq!(
            collapsed.pages.len(),
            control.pages.len(),
            "{borders}: page count"
        );
        assert_eq!(
            painted_text(&collapsed),
            painted_text(&control),
            "{borders}: repeated group or body content changed"
        );
        for (page, (actual, expected)) in collapsed.pages.iter().zip(&control.pages).enumerate() {
            let positions = |lines: &[ferri_pdf::TextLine]| {
                lines
                    .iter()
                    .map(|line| (line.text.clone(), line.x, line.baseline))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                positions(actual),
                positions(expected),
                "{borders}: page {page} geometry"
            );
        }
    }
}

#[test]
fn collapsed_tracks_preserve_positioned_percentage_containing_dimensions() {
    let body = "<table><col><col class=gone><tr class=gone><td colspan=2 rowspan=2 class=span><div></div></td></tr><tr></tr></table>";
    let css = "table{width:120pt;table-layout:fixed;border-spacing:0}col{width:60pt}tr{height:40pt}td{padding:0;position:relative;height:80pt}.span{visibility:visible}td div{position:absolute;top:25%;left:0;width:50%;height:10pt;background:blue}";
    let normal = trace(css, body);
    let collapsed = trace(&format!("{css}.gone{{visibility:collapse}}"), body);
    let normal_rect = common::painted_box(&normal, [0, 0, 255]);
    let collapsed_rect = common::painted_box(&collapsed, [0, 0, 255]);
    common::assert_pt(normal_rect.width, 60.0);
    common::assert_pt(normal_rect.height, 10.0);
    assert_eq!(
        collapsed_rect, normal_rect,
        "positioned child used the projected containing dimensions"
    );
}

#[test]
fn collapsed_flex_struts_preserve_cross_size_without_baseline_participation() {
    for direction in ["row", "row-reverse"] {
        for wrapping in ["nowrap", "wrap-reverse"] {
            let output = trace(
                &format!(
                    "p{{margin:0}}section{{display:flex;flex-direction:{direction};flex-wrap:{wrapping};width:200pt;align-items:baseline}}section>div{{width:40pt;flex-shrink:0}}.a{{font:10pt/14pt Spike}}.b{{font:20pt/24pt Spike}}.gone{{visibility:collapse;height:70pt;font:40pt/50pt Spike}}"
                ),
                "<section><div class=a>A</div><div class=gone>Hidden</div><div class=b>B</div></section><p>After</p>",
            );
            let locate = |label| {
                output.pages[0]
                    .iter()
                    .find(|line| line.text == label)
                    .unwrap()
            };
            assert!(
                (locate("A").baseline - locate("B").baseline).abs() < 0.001,
                "{direction}/{wrapping}: A={} B={}",
                locate("A").baseline,
                locate("B").baseline
            );
            let control = trace(
                "p{margin:0}section{height:70pt}",
                "<section></section><p>After</p>",
            );
            common::assert_pt(locate("After").baseline, control.pages[0][0].baseline);
            assert_eq!(
                painted_text(&output).len(),
                3,
                "collapsed baseline item painted"
            );
        }
    }
}

#[test]
fn collapsed_rtl_colspan_positions_right_anchored_child_at_projected_inline_start() {
    let output = trace(
        "table{direction:rtl;width:120pt;table-layout:fixed;border-spacing:0}col{width:60pt}.gone{visibility:collapse}td{padding:0;position:relative;height:40pt}td div{position:absolute;right:0;top:0;width:50%;height:10pt;background:blue}",
        "<table><col><col class=gone><tr><td colspan=2><div></div></td></tr></table>",
    );
    let rect = common::painted_box(&output, [0, 0, 255]);
    common::assert_pt(rect.x, 36.0);
    common::assert_pt(rect.width, 60.0);
    common::assert_pt(rect.height, 10.0);
}

#[test]
fn hidden_anchor_keeps_only_its_visible_descendants_clickable() {
    let output = trace(
        "a{visibility:hidden}span{visibility:visible}",
        "<a href='https://example.org/target'>Hidden<span>Visible</span>Hidden</a>",
    );
    assert_eq!(painted_text(&output), ["Visible"]);
    assert_eq!(
        output.links.iter().map(Vec::len).sum::<usize>(),
        1,
        "visible link descendant must retain its clickable region"
    );
}

#[test]
fn collapsed_flex_definite_cross_size_stretches_visible_items() {
    for wrapping in ["nowrap", "wrap", "wrap-reverse"] {
        let output = trace(
            &format!(
                "section{{display:flex;flex-wrap:{wrapping};width:200pt;height:90pt;gap:10pt}}section>div{{width:40pt;flex-shrink:0;background:blue}}.gone{{visibility:collapse;height:70pt;background:red}}p{{margin:0}}"
            ),
            "<section><div>A</div><div class=gone>Hidden</div><div>B</div></section><p>After</p>",
        );
        let boxes = output
            .display_lists
            .iter()
            .flatten()
            .filter_map(|item| match item {
                DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint.rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(boxes.len(), 2, "{wrapping}: visible stretched items");
        for rect in boxes {
            common::assert_pt(rect.height, 90.0);
        }
        let a = output.pages[0]
            .iter()
            .find(|line| line.text == "A")
            .unwrap();
        let b = output.pages[0]
            .iter()
            .find(|line| line.text == "B")
            .unwrap();
        common::assert_pt(b.x - a.x, 50.0);
        assert_eq!(painted_text(&output).len(), 3);
    }
}

#[test]
fn collapsed_columns_follow_named_row_page_geometry() {
    let output = trace(
        "@page{size:240pt 140pt;margin:20pt}@page wide{size:400pt 140pt;margin:20pt}table{width:100%;table-layout:fixed;border-spacing:0}col{width:50%}.gone{visibility:collapse}td{padding:0;background:blue}.wide{page:wide}",
        "<table><col><col class=gone><tr><td colspan=2>First</td></tr><tr class=wide><td colspan=2>Second</td></tr></table>",
    );
    assert_eq!(output.pages.len(), 2);
    for (page, expected) in [100.0, 180.0].into_iter().enumerate() {
        common::assert_pt(
            output.page_records[page].geometry.paper[0],
            expected * 2.0 + 40.0,
        );
        let widths = output.display_lists[page]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0, 0, 255]
                        && paint.decoration.background.alpha > 0.0 =>
                {
                    Some(paint.rect.width)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!widths.is_empty(), "page {page}: missing cell paint");
        for width in widths {
            common::assert_pt(width, expected);
        }
    }
}

#[test]
fn entirely_collapsed_rows_keep_caption_without_retaining_row_height() {
    let css = "table{width:120pt;border-spacing:0}caption{height:30pt}tr{height:40pt;visibility:collapse}td{padding:0}p{margin:0}";
    let collapsed = trace(
        css,
        "<table><caption>Caption</caption><tr><td>Hidden</td></tr><tr><td>Hidden</td></tr></table><p>After</p>",
    );
    let control = trace(
        "p{margin:0}div{height:30pt}",
        "<div>Caption</div><p>After</p>",
    );
    assert_eq!(painted_text(&collapsed), ["Caption", "After"]);
    let after = |output: &LayoutTrace| {
        output.pages[0]
            .iter()
            .find(|line| line.text == "After")
            .unwrap()
            .baseline
    };
    common::assert_pt(after(&collapsed), after(&control));
}

#[test]
fn fragmented_collapsed_rowspan_keeps_original_positioned_containing_height() {
    let body = format!(
        "<table><tr class=gone><td rowspan=2 class=span><div></div>{}</td></tr><tr></tr></table><p>After</p>",
        "<p>Line</p>".repeat(12)
    );
    let output = trace(
        "@page{size:240pt 100pt;margin:10pt}p{margin:0;font:10pt/14pt Spike;widows:1;orphans:1}table{border-spacing:0;width:120pt}tr{height:90pt}td{position:relative;padding:0;vertical-align:top}.gone{visibility:collapse}.span{visibility:visible}td div{position:absolute;left:0;top:25%;width:50%;height:10pt;background:blue}",
        &body,
    );
    assert_eq!(output.pages.len(), 2);
    let rect = common::painted_box(&output, [0, 0, 255]);
    common::assert_pt(rect.y, 55.0);
    common::assert_pt(rect.width, 60.0);
    common::assert_pt(rect.height, 10.0);
    assert!(output.pages[1].iter().any(|line| line.text == "After"));
}
