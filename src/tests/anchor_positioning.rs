use super::common::{assert_pt, local_options};
use crate::{Color, DisplayItem, LayoutTrace, Rect, render_html};
use std::fmt::Write as _;

const TARGET_COLOR: Color = Color {
    rgb: [255, 0, 0],
    alpha: 1.0,
};
const PROFILE: &str = "@page{size:300pt 240pt;margin:20pt}html,body{margin:0;padding:0} .anchor{anchor-name:--a;width:60pt;height:30pt;margin-left:40pt;background:blue} .target{position:absolute;position-anchor:--a;width:20pt;height:10pt;background:red}";

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<!doctype html><style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("anchor CSS {css:?}, body {body:?}: {error}"))
    .layout
    .expect("layout was requested")
}

fn targets(trace: &LayoutTrace) -> Vec<(usize, Rect)> {
    trace
        .display_lists
        .iter()
        .enumerate()
        .flat_map(|(page, list)| {
            list.iter().filter_map(move |item| match item {
                DisplayItem::Box(paint) if paint.decoration.background == TARGET_COLOR => {
                    Some((page, paint.rect))
                }
                _ => None,
            })
        })
        .collect()
}

fn assert_rect(actual: Rect, expected: [f32; 4]) {
    for (actual, expected) in [actual.x, actual.y, actual.width, actual.height]
        .into_iter()
        .zip(expected)
    {
        assert_pt(actual, expected);
    }
}

#[test]
fn named_anchor_functions_resolve_insets_sizes_and_calc_against_the_border_box() {
    let trace = render(
        ".target{top:calc(anchor(bottom) + 5pt);left:anchor(left);width:anchor-size(width);height:anchor-size(height)}",
        "<div class=anchor></div><div class=target></div>",
    );
    let boxes = targets(&trace);
    assert_eq!(boxes.len(), 1, "one anchored target must paint");
    assert_rect(boxes[0].1, [60.0, 55.0, 60.0, 30.0]);
}

#[test]
fn anchor_scope_restricts_lookup_to_the_matching_subtree() {
    let trace = render(
        "section{anchor-scope:--a}.outside{anchor-name:--a;width:100pt;height:20pt;margin-left:120pt}",
        "<section><div class=anchor></div><div class=target style='top:anchor(bottom);left:anchor(left)'></div></section><div class=outside></div>",
    );
    assert_rect(targets(&trace)[0].1, [60.0, 50.0, 20.0, 10.0]);
}

#[test]
fn missing_or_wrong_axis_anchor_functions_use_their_fallback_or_initial_value() {
    let trace = render(
        ".target{position-anchor:--missing;top:anchor(bottom,7pt);left:anchor(top,11pt);width:anchor-size(width,35pt)}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [31.0, 27.0, 35.0, 10.0]);
}

#[test]
fn position_areas_and_anchor_center_align_against_the_default_anchor() {
    for (area, expected) in [
        ("bottom", [80.0, 50.0, 20.0, 10.0]),
        ("top", [80.0, 10.0, 20.0, 10.0]),
        ("right", [120.0, 30.0, 20.0, 10.0]),
        ("center", [80.0, 30.0, 20.0, 10.0]),
        ("bottom right", [120.0, 50.0, 20.0, 10.0]),
    ] {
        let trace = render(
            &format!(".target{{position-area:{area}}}"),
            "<div class=anchor></div><div class=target></div>",
        );
        assert_rect(targets(&trace)[0].1, expected);
    }
    let trace = render(
        ".target{top:anchor(bottom);justify-self:anchor-center}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [80.0, 50.0, 20.0, 10.0]);
}

#[test]
fn try_tactics_and_named_rules_select_the_first_fitting_option() {
    for fallback in ["flip-block", "--above"] {
        let trace = render(
            &format!(
                "@position-try --above{{position-area:top}}.anchor{{margin-top:170pt}}.target{{position-area:bottom;position-try-fallbacks:{fallback}}}"
            ),
            "<div class=anchor></div><div class=target></div>",
        );
        assert_rect(targets(&trace)[0].1, [80.0, 180.0, 20.0, 10.0]);
    }
}

#[test]
fn position_visibility_hides_invalid_invisible_and_overflowing_anchors() {
    for (css, body) in [
        (
            ".anchor{visibility:hidden}.target{position-area:bottom}",
            "<div class=anchor></div><div class=target></div>",
        ),
        (
            ".target{position-anchor:--missing;top:anchor(bottom);position-visibility:anchors-valid}",
            "<div class=target></div>",
        ),
        (
            ".target{top:215pt;position-visibility:no-overflow}",
            "<div class=anchor></div><div class=target></div>",
        ),
    ] {
        assert!(
            targets(&render(css, body)).is_empty(),
            "conditional visibility must hide {css}"
        );
    }
}

#[test]
fn absolute_anchor_order_rejects_cycles_and_forward_siblings() {
    let trace = render(
        ".anchor{position:absolute;left:40pt;top:30pt}.target{left:anchor(left,7pt);top:anchor(bottom,9pt)}",
        "<div class=target></div><div class=anchor></div>",
    );
    assert_rect(targets(&trace)[0].1, [27.0, 29.0, 20.0, 10.0]);
    let trace = render(
        ".target{anchor-name:--a;left:anchor(left,7pt);top:anchor(bottom,9pt)}",
        "<div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [27.0, 29.0, 20.0, 10.0]);
}

#[test]
fn anchor_functions_in_normal_flow_use_fallbacks_and_position_resets_default_anchor() {
    let trace = render(
        ".target{position:static;width:anchor-size(--a width,35pt)}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [20.0, 50.0, 35.0, 10.0]);
    let trace = render(
        ".target{position:absolute;top:anchor(bottom,9pt);left:anchor(left,7pt)}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [27.0, 29.0, 20.0, 10.0]);
}

#[test]
fn position_try_order_sorts_by_available_space_and_tactics_transform_anchor_sides() {
    let trace = render(
        ".target{position-area:left;position-try-fallbacks:right;position-try-order:most-width}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [120.0, 30.0, 20.0, 10.0]);
    let trace = render(
        ".anchor{margin-top:170pt}.target{left:anchor(left);top:anchor(bottom);position-try-fallbacks:flip-block}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [60.0, 180.0, 20.0, 10.0]);
}

#[test]
fn fallback_origin_preserves_important_and_discards_invalid_rule_declarations() {
    let trace = render(
        "@position-try --above{position-area:top}.anchor{margin-top:170pt}.target{position-area:bottom!important;position-try-fallbacks:--above}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [80.0, 220.0, 20.0, 10.0]);
    let trace = render(
        "@position-try --above{top:0!important;color:green;position-area:top}.anchor{margin-top:170pt}.target{position-area:bottom;position-try-fallbacks:--missing,--above}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [80.0, 180.0, 20.0, 10.0]);
}

#[test]
fn anchor_lookup_prefers_ancestors_and_otherwise_the_last_acceptable_element() {
    let trace = render(
        ".anchor{height:50pt}.later{anchor-name:--a;margin-left:150pt;width:40pt;height:20pt}.target{top:anchor(bottom);left:anchor(left)}",
        "<div class=anchor><div class=target></div></div><div class=later></div>",
    );
    assert_rect(targets(&trace)[0].1, [60.0, 70.0, 20.0, 10.0]);
    let trace = render(
        ".later{anchor-name:--a;margin-left:150pt;width:40pt;height:20pt}.target{top:anchor(bottom);left:anchor(left)}",
        "<div class=anchor></div><div class=target></div><div class=later></div>",
    );
    assert_rect(targets(&trace)[0].1, [170.0, 70.0, 20.0, 10.0]);
}

#[test]
fn inline_and_nested_containing_blocks_supply_anchor_border_geometry() {
    let trace = render(
        "section{position:relative;margin-left:30pt;padding:10pt}.target{top:anchor(bottom);left:anchor(left)}",
        "<section><div class=anchor></div><div class=target></div></section>",
    );
    assert_rect(targets(&trace)[0].1, [100.0, 60.0, 20.0, 10.0]);
    let trace = render(
        "span{anchor-name:--a;display:inline;font:10pt/20pt serif}.target{left:anchor(left);top:anchor(bottom)}",
        "<p style='margin:0;padding-left:40pt'><span>Anchor</span></p><div class=target></div>",
    );
    let target = targets(&trace)[0].1;
    assert_pt(target.x, 60.0);
    assert!(
        target.y > 20.0 && target.y < 45.0,
        "inline anchor must use its inline border, got {target:?}"
    );
}

#[test]
fn anchors_on_later_pages_and_fixed_repetition_keep_physical_coordinates() {
    let trace = render(
        ".anchor{break-before:page}.target{left:anchor(left);top:anchor(bottom)}",
        "<p>Page one</p><div class=anchor></div><div class=target></div>",
    );
    let boxes = targets(&trace);
    assert_eq!(
        boxes[0].0, 1,
        "the target must stay with the second-page anchor"
    );
    assert_rect(boxes[0].1, [60.0, 50.0, 20.0, 10.0]);
    let trace = render(
        ".target{position:fixed;position-anchor:--a;left:anchor(left);top:anchor(bottom)}",
        "<div class=anchor></div><div class=target></div><p style='break-before:page'>Page two</p>",
    );
    let boxes = targets(&trace);
    assert_eq!(
        boxes.len(),
        2,
        "fixed anchor placement repeats on physical pages"
    );
    for (_, rect) in boxes {
        assert_rect(rect, [60.0, 50.0, 20.0, 10.0]);
    }
}

#[test]
fn transformed_anchors_and_principal_box_filtering_affect_placement() {
    let trace = render(
        ".anchor{transform:translate(25pt,5pt)}.target{left:anchor(left);top:anchor(bottom)}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [85.0, 55.0, 20.0, 10.0]);
    for display in ["none", "contents"] {
        let trace = render(
            &format!(
                ".later{{anchor-name:--a;display:{display}}}.target{{left:anchor(left);top:anchor(bottom)}}"
            ),
            "<div class=anchor></div><div class=target></div><div class=later></div>",
        );
        assert_rect(targets(&trace)[0].1, [60.0, 50.0, 20.0, 10.0]);
    }
}

#[test]
fn clipping_intervening_ancestors_hides_anchors_but_shared_clipping_does_not() {
    let trace = render(
        "section{height:10pt;overflow:hidden}.anchor{margin-top:20pt}.target{left:anchor(left);top:anchor(bottom)}",
        "<section><div class=anchor></div></section><div class=target></div>",
    );
    assert!(
        targets(&trace).is_empty(),
        "an anchor fully clipped below the target's containing block must hide its target"
    );
    let trace = render(
        "section{position:relative;height:10pt;overflow:hidden}.anchor{margin-top:20pt}.target{left:anchor(left);top:anchor(bottom)}",
        "<section><div class=anchor></div><div class=target></div></section>",
    );
    assert!(trace.display_lists.iter().flatten().any(|item| matches!(item,DisplayItem::Box(paint) if paint.decoration.background == TARGET_COLOR)), "shared clipping must not force-hidden the target");
}

#[test]
fn column_measurement_probes_preserve_final_anchor_coordinates() {
    let trace = render(
        "section{columns:2;column-gap:20pt;height:80pt}.anchor{break-before:column;margin-left:10pt}.target{left:anchor(left);top:anchor(bottom)}",
        "<section><p style='margin:0;height:20pt'>First</p><div class=anchor></div><div class=target></div></section>",
    );
    assert_rect(targets(&trace)[0].1, [170.0, 50.0, 20.0, 10.0]);
}

#[test]
fn generated_pseudo_elements_resolve_anchors_and_position_options() {
    let trace = render(
        "body::after{content:'';display:block;position:absolute;position-anchor:--a;position-area:bottom;position-try-fallbacks:flip-block;width:20pt;height:10pt;background:red}.anchor{margin-top:170pt}",
        "<div class=anchor></div>",
    );
    assert_rect(targets(&trace)[0].1, [80.0, 180.0, 20.0, 10.0]);
}

#[test]
fn zero_fallbacks_and_percentage_anchor_sides_resolve_as_lengths() {
    let trace = render(
        ".target{left:anchor(--missing left,0);top:anchor(--missing bottom,0);width:anchor-size(--missing width,0)}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [20.0, 20.0, 0.0, 10.0]);
    let trace = render(
        ".target{left:anchor(25%);top:anchor(50%)}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [75.0, 35.0, 20.0, 10.0]);
}

#[test]
fn generated_anchor_order_follows_before_children_after_in_the_flat_tree() {
    let trace = render(
        ".anchor{anchor-name:none}.anchor::before,.anchor::after{content:'';display:block;anchor-name:--a;width:40pt;height:10pt}.anchor::after{margin-left:20pt}.target{top:anchor(bottom);left:anchor(left)}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [80.0, 40.0, 20.0, 10.0]);
}

#[test]
fn anchor_names_and_functions_keep_the_declaring_stylesheets_tree_scope() {
    let trace = render(
        ".host::part(anchor){anchor-name:--a;width:60pt;height:30pt;margin-left:40pt;background:blue}.target{left:anchor(left,7pt);top:anchor(bottom,9pt)}",
        "<section class=host><template shadowrootmode=open><div part=anchor></div></template></section><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [60.0, 50.0, 20.0, 10.0]);
}

#[test]
fn position_try_revert_uses_the_user_origin_instead_of_the_author_base_value() {
    let css = "@position-try --user{top:revert}.target{left:40pt;top:215pt;position-try-fallbacks:--user}";
    let mut options = local_options();
    options.user_stylesheets.push(".target{top:150pt}".into());
    let trace = render_html(
        &format!("<!doctype html><style>{PROFILE}{css}</style><div class=target></div>"),
        &options,
    )
    .expect("position fallback revert must resolve the user cascade")
    .layout
    .expect("layout was requested");
    assert_rect(targets(&trace)[0].1, [60.0, 170.0, 20.0, 10.0]);
}

#[test]
fn fragmented_anchors_use_source_geometry_inside_and_fragment_bounds_outside() {
    let body = "<section><div class=anchor><p></p><p></p><p></p><p></p><p></p></div><div class=target></div></section>";
    let css = ".anchor{height:auto}.anchor p{height:60pt;margin:0}.target{left:0;top:0;width:anchor-size(width);height:anchor-size(height)}";
    let trace = render(&format!("{css}section{{position:relative}}"), body);
    assert_rect(targets(&trace)[0].1, [20.0, 20.0, 60.0, 300.0]);
    let trace = render(css, body);
    assert_rect(targets(&trace)[0].1, [20.0, 20.0, 60.0, 200.0]);
    let css = ".columns{columns:2;column-gap:20pt;height:80pt}.anchor{height:auto}.anchor p{height:60pt;margin:0}.target{left:0;top:0;width:anchor-size(width);height:anchor-size(height)}";
    let body = "<section class=columns><div class=wrapper><div class=anchor><p></p><p></p></div><div class=target></div></div></section>";
    let trace = render(css, body);
    assert_rect(targets(&trace)[0].1, [20.0, 20.0, 200.0, 60.0]);
    let trace = render(&format!("{css}.wrapper{{position:relative}}"), body);
    assert_rect(targets(&trace)[0].1, [160.0, 20.0, 60.0, 120.0]);
}

#[test]
fn repeated_fixed_boxes_compute_anchor_lengths_for_each_physical_page() {
    let trace = render(
        "@page wide{size:400pt 340pt}.next{page:wide;break-before:page}.target{position:fixed;position-anchor:--a;right:anchor(right);bottom:anchor(bottom);width:anchor-size(width);height:anchor-size(height)}",
        "<div class=anchor></div><div class=target></div><p class=next>Next</p>",
    );
    let boxes = targets(&trace);
    assert_eq!(boxes.len(), 2);
    for (_, rect) in boxes {
        assert_rect(rect, [60.0, 20.0, 60.0, 30.0]);
    }
    let trace = render(
        "@page wide{size:400pt 240pt}.next{page:wide;break-before:page}.anchor{position:fixed;left:50%;top:0;margin:0;width:50%}.target{position:fixed;position-anchor:--a;left:anchor(left);top:anchor(bottom);width:anchor-size(width)}",
        "<p>First</p><div class=anchor></div><div class=target></div><p class=next>Next</p>",
    );
    let boxes = targets(&trace);
    assert_eq!(boxes.len(), 2);
    assert_rect(boxes[0].1, [150.0, 50.0, 130.0, 10.0]);
    assert_rect(boxes[1].1, [200.0, 50.0, 180.0, 10.0]);
}

#[test]
fn position_try_shorthand_expands_order_fallbacks_and_resets_them() {
    let trace = render(
        ".target{position-area:left;position-try:most-width right}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [120.0, 30.0, 20.0, 10.0]);
    let trace = render(
        ".target{position-area:left;position-try:most-width right;position-try:none}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [40.0, 30.0, 20.0, 10.0]);
}

#[test]
fn page_specific_visibility_hides_whole_subtrees_and_plain_absolute_overflow() {
    let trace = render(
        "@page wide{size:400pt 240pt}.next{page:wide;break-before:page}.anchor{margin-left:200pt;width:20pt}.target{position:fixed;position-anchor:--a;position-area:right;width:60pt;position-visibility:no-overflow}",
        "<div class=anchor></div><div class=target><span style='visibility:visible'>Child</span></div><p class=next>Next</p>",
    );
    let boxes = targets(&trace);
    assert_eq!(
        boxes.iter().map(|(page, _)| *page).collect::<Vec<_>>(),
        [1],
        "only the wider page fits the anchored box"
    );
    assert!(
        trace.pages[1].iter().any(|line| line.text == "Child"),
        "the visible page must paint the subtree's text"
    );
    assert!(
        !trace.pages[0].iter().any(|line| line.text == "Child"),
        "force-hidden cannot be undone by a descendant"
    );
    let trace = render(
        ".target{position-anchor:none;top:215pt;position-visibility:no-overflow}",
        "<div class=target></div>",
    );
    assert!(
        targets(&trace).is_empty(),
        "no-overflow also applies without anchor references"
    );
}

#[test]
fn unused_anchor_fallbacks_do_not_invalidate_successful_outer_references() {
    let trace = render(
        ".target{left:anchor(left,anchor(--missing right));top:anchor(bottom);position-visibility:anchors-valid}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [60.0, 50.0, 20.0, 10.0]);
    let trace = render(
        ".target{left:anchor(top);top:anchor(bottom);position-visibility:anchors-valid}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_eq!(
        targets(&trace).len(),
        1,
        "an invalid axis does not make the named anchor element missing"
    );
}

#[test]
fn long_acyclic_anchor_dependencies_settle_within_the_shared_work_budget() {
    const LINKS: usize = 70;
    let mut body = String::from(
        "<div style='position:absolute;left:0;top:0;width:1pt;height:1pt;anchor-name:--p0'></div>",
    );
    for index in 1..=LINKS {
        let class = if index == LINKS { " class=target" } else { "" };
        write!(body, "<div{class} style='position:absolute;position-anchor:--p{};anchor-name:--p{index};left:anchor(right);top:0;width:1pt;height:1pt'></div>",index-1).expect("writing HTML to a String cannot fail");
    }
    let trace = render("", &body);
    assert_rect(targets(&trace)[0].1, [90.0, 20.0, 1.0, 1.0]);
}

#[test]
fn cloned_anchor_fragments_keep_unfragmented_source_dimensions() {
    let trace = render(
        "section{position:relative;columns:2;column-gap:20pt;height:80pt}.wrapper{position:relative}.anchor{height:auto;box-decoration-break:clone}.anchor p{height:60pt;margin:0}.target{left:0;top:0;width:anchor-size(width);height:anchor-size(height)}",
        "<section><div class=wrapper><div class=anchor><p></p><p></p></div><div class=target></div></div></section>",
    );
    assert_rect(targets(&trace)[0].1, [160.0, 20.0, 60.0, 120.0]);
}

#[test]
fn cross_page_effects_compare_resolved_matrices_and_clips() {
    let trace = render(
        "@page wide{size:400pt 240pt}.scope{position:relative;transform:translateX(50%);overflow:hidden}.next{page:wide;break-before:page}.target{left:anchor(left);top:anchor(bottom)}",
        "<section class=scope><div class=anchor></div><p class=next>Next</p><div class=target></div></section>",
    );
    let boxes = targets(&trace);
    assert_eq!(
        boxes[0].0, 1,
        "the target's containing block uses its second physical fragment"
    );
    assert_rect(boxes[0].1, [10.0, 50.0, 20.0, 10.0]);
}

#[test]
fn selected_try_styles_drive_ratio_transfer_and_natural_height_measurement() {
    for (base, option, content, expected) in [
        (
            "width:auto;height:10pt;aspect-ratio:2",
            "width:40pt",
            "",
            [20.0, 20.0, 40.0, 10.0],
        ),
        (
            "width:auto;height:10pt;aspect-ratio:2",
            "height:20pt",
            "",
            [20.0, 20.0, 40.0, 20.0],
        ),
        (
            "width:20pt;height:40pt",
            "height:auto",
            "<div style='height:10pt'></div>",
            [20.0, 20.0, 20.0, 10.0],
        ),
    ] {
        let trace = render(
            &format!(
                "section{{position:relative;width:160pt;height:100pt}}@position-try --fit{{left:0;{option}}}.target{{left:145pt;top:0;{base};position-try-fallbacks:--fit}}"
            ),
            &format!("<section><div class=target>{content}</div></section>"),
        );
        assert_rect(targets(&trace)[0].1, expected);
    }
}

#[test]
fn selected_try_styles_drive_specialized_root_sizing() {
    for display in ["block", "flex", "grid", "table"] {
        let content = if display == "table" {
            "<tbody><tr><td style='padding:0;height:10pt'></td></tr></tbody>"
        } else {
            "<div style='height:10pt'></div>"
        };
        let element = if display == "table" { "table" } else { "div" };
        let trace = render(
            &format!(
                "section{{position:relative;width:160pt;height:100pt}}@position-try --fit{{left:0;height:auto;min-height:15pt;max-height:20pt}}.target{{display:{display};border-spacing:0;left:145pt;top:0;width:20pt;height:40pt;position-try-fallbacks:--fit}}"
            ),
            &format!("<section><{element} class=target>{content}</{element}></section>"),
        );
        assert_rect(targets(&trace)[0].1, [20.0, 20.0, 20.0, 15.0]);
    }
    let trace = render(
        "section{position:relative;width:160pt;height:100pt}@position-try --fit{left:0;width:40pt;height:auto}.target{left:145pt;top:0;width:20pt;height:40pt;position-try-fallbacks:--fit}",
        "<section><img class=target src='data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2210%22 height=%2210%22/%3E'></section>",
    );
    assert_rect(targets(&trace)[0].1, [20.0, 20.0, 40.0, 40.0]);
}

#[test]
fn position_try_variable_shorthands_expand_after_substitution() {
    let trace = render(
        "section{position:relative;width:160pt;height:100pt}@position-try --fit{inset:var(--edges);margin:var(--margin);size:var(--size)}.target{--edges:0 auto auto 0;--margin:5pt;--size:40pt 20pt;left:145pt;top:0;position-try-fallbacks:--fit}",
        "<section><div class=target></div></section>",
    );
    assert_rect(targets(&trace)[0].1, [25.0, 25.0, 40.0, 20.0]);
}

#[test]
fn escaped_anchor_function_names_inside_math_resolve_normally() {
    let trace = render(
        r".target{left:calc(\61nchor(left) + 1pt);top:calc(\61nchor(bottom) + 2pt);width:calc(\61nchor-size(width) + 3pt)}",
        "<div class=anchor></div><div class=target></div>",
    );
    assert_rect(targets(&trace)[0].1, [61.0, 52.0, 63.0, 10.0]);
}

#[test]
fn anchors_visible_uses_ink_overflow_without_changing_border_geometry() {
    for ink in [
        "box-shadow:-20pt 0 green",
        "box-sizing:border-box;border:1pt solid transparent;border-image:linear-gradient(green,green) 1 / 1 / 20pt",
        "text-indent:-25pt;white-space:nowrap;font:10pt monospace",
    ] {
        let trace = render(
            &format!(
                "section{{width:20pt;height:40pt;overflow:hidden}}.anchor{{margin-left:30pt;width:10pt;height:10pt;{ink}}}.target{{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}}"
            ),
            "<section><div class=anchor>Ink</div></section><div class=target></div>",
        );
        assert_eq!(
            targets(&trace).len(),
            1,
            "ink from {ink} intersects the intervening clip"
        );
        assert_rect(targets(&trace)[0].1, [50.0, 30.0, 20.0, 10.0]);
    }
}

#[test]
fn anchors_visible_clips_against_paths_but_ignores_alpha_masks_and_opacity() {
    for (effect, visible) in [
        ("clip-path:inset(100%)", false),
        ("clip-path:circle(5pt at 0 0)", false),
        (
            "clip-path:path(evenodd,'M0 0H100V100H0Z M20 0H80V80H20Z')",
            false,
        ),
        ("clip-path:inset(0)", true),
        ("mask-image:linear-gradient(transparent,transparent)", true),
        ("opacity:0", true),
    ] {
        let trace = render(
            &format!(
                "section{{width:100pt;height:100pt;{effect}}}.anchor{{margin-left:30pt;width:10pt;height:10pt}}.target{{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}}"
            ),
            "<section><div class=anchor></div></section><div class=target></div>",
        );
        assert_eq!(
            !targets(&trace).is_empty(),
            visible,
            "anchor visibility with {effect}"
        );
    }
}

#[test]
fn rounded_overflow_clips_anchor_ink_at_the_corner() {
    let trace = render(
        "section{width:100pt;height:100pt;overflow:hidden;border-radius:50pt}.anchor{margin:0;width:10pt;height:10pt}.target{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}",
        "<section><div class=anchor></div></section><div class=target></div>",
    );
    assert!(
        targets(&trace).is_empty(),
        "the corner is outside the rounded overflow region"
    );
}

#[test]
fn outside_list_markers_and_descendant_shadows_contribute_anchor_ink() {
    for (extra, content) in [
        (
            "display:list-item;list-style-type:'Ink';font:10pt monospace",
            "",
        ),
        (
            "",
            "<div style='height:5pt;width:1pt;box-shadow:-20pt 0 green'></div>",
        ),
    ] {
        let trace = render(
            &format!(
                "section{{width:20pt;height:40pt;overflow:hidden}}.anchor{{margin-left:30pt;width:10pt;height:10pt;{extra}}}.target{{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}}"
            ),
            &format!(
                "<section><div class=anchor>{content}</div></section><div class=target></div>"
            ),
        );
        assert_eq!(
            targets(&trace).len(),
            1,
            "anchor contents must contribute ink: {extra} {content}"
        );
        assert_rect(targets(&trace)[0].1, [50.0, 30.0, 20.0, 10.0]);
    }
}

#[test]
fn filter_support_bounds_control_anchor_visibility_without_using_whole_filter_regions() {
    for (filter, primitive, visible) in [
        ("drop-shadow(-20pt 0 green)", "", true),
        ("blur(8pt)", "", true),
        ("url(#ink)", "<feColorMatrix/>", false),
        (
            "url(#ink)",
            "<feFlood x='40' y='26.666667' width='10' height='10' result='band'/><feComposite in='band' in2='band' operator='arithmetic' k1='-1'/>",
            false,
        ),
        (
            "url(#ink)",
            "<feFlood x='40' y='26.666667' width='10' height='10' result='band'/><feComposite in='band' in2='band' operator='arithmetic' k1='1' k2='-1'/>",
            false,
        ),
        (
            "url(#ink)",
            "<feFlood x='40' y='26.666667' width='10' height='10' result='band'/><feComposite in='band' in2='SourceGraphic' operator='arithmetic' k2='-1' k3='1'/>",
            false,
        ),
        (
            "url(#ink)",
            "<feSpecularLighting lighting-color='black' specularConstant='1'><feDistantLight elevation='90'/></feSpecularLighting>",
            false,
        ),
        (
            "url(#ink)",
            "<feSpecularLighting lighting-color='white' specularConstant='1'><feDistantLight elevation='90'/></feSpecularLighting>",
            true,
        ),
        (
            "url(#ink)",
            "<feDiffuseLighting lighting-color='black' diffuseConstant='1'><feDistantLight elevation='90'/></feDiffuseLighting>",
            true,
        ),
        (
            "url(#ink)",
            "<feDiffuseLighting diffuseConstant='0'><feDistantLight elevation='90'/></feDiffuseLighting>",
            true,
        ),
        ("url(#ink)", "<feOffset dx='30'/>", false),
        ("url(#ink)", "<feOffset dx='-26.666667'/>", true),
        ("url(#ink)", "<feGaussianBlur stdDeviation='8'/>", true),
        (
            "url(#ink)",
            "<feDropShadow dx='-26.666667' dy='0' stdDeviation='0'/>",
            true,
        ),
        (
            "url(#ink)",
            "<feFlood x='40' y='26.666667' width='10' height='10' flood-color='green'/>",
            true,
        ),
        (
            "url(#ink)",
            "<feFlood x='100' y='26.666667' width='10' height='10' flood-color='green'/>",
            false,
        ),
        (
            "url(#ink)",
            "<feImage x='40' y='26.666667' width='10' height='10' href='data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2210%22 height=%2210%22%3E%3Crect width=%2210%22 height=%2210%22 fill=%22green%22/%3E%3C/svg%3E'/>",
            true,
        ),
    ] {
        let trace = render(
            &format!(
                "section{{width:20pt;height:40pt;overflow:hidden}}.anchor{{margin-left:30pt;width:10pt;height:10pt;filter:{filter}}}.target{{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}}"
            ),
            &format!(
                "<svg style='display:none'><defs><filter id='ink' filterUnits='userSpaceOnUse' x='0' y='0' width='300' height='300'>{primitive}</filter></defs></svg><section><div class=anchor></div></section><div class=target></div>"
            ),
        );
        assert_eq!(
            !targets(&trace).is_empty(),
            visible,
            "filter {filter}: {primitive}"
        );
        if visible {
            assert_rect(targets(&trace)[0].1, [50.0, 30.0, 20.0, 10.0]);
        }
    }
}

#[test]
fn filtered_descendant_ink_is_clipped_after_filtering_and_does_not_leak_source_ink() {
    for (offset, indent, visible) in [
        (0.0, 0.0, false),
        (-20.0, 0.0, true),
        (20.0, 0.0, false),
        (40.0, -25.0, false),
    ] {
        let trace = render(
            "section{width:20pt;height:40pt;overflow:hidden}.anchor{margin-left:30pt;width:10pt;height:10pt}.target{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}",
            &format!(
                "<svg style='display:none'><defs><filter id='shift' filterUnits='userSpaceOnUse' x='0' y='0' width='300' height='300'><feOffset dx='{}'/></filter></defs></svg><section><div class=anchor><div style='filter:url(#shift);text-indent:{indent}pt'><span style='font:10pt monospace'>Ink</span></div></div></section><div class=target></div>",
                offset / crate::graphics::POINTS_PER_PIXEL
            ),
        );
        assert_eq!(
            !targets(&trace).is_empty(),
            visible,
            "filtered descendant offset {offset}pt"
        );
    }
    let trace = render(
        "section{width:20pt;height:40pt;overflow:hidden}.anchor{margin-left:30pt;width:10pt;height:10pt;filter:url(#clipped)}.target{position-visibility:anchors-visible}",
        "<svg style='display:none'><defs><filter id='clipped' filterUnits='userSpaceOnUse' x='80' y='0' width='200' height='200'><feOffset dx='-26.666667'/></filter></defs></svg><section><div class=anchor></div></section><div class=target></div>",
    );
    assert!(
        targets(&trace).is_empty(),
        "filter-region clipping removes the input before its offset"
    );
}

#[test]
fn anchor_visibility_requires_clipping_by_an_intervening_ancestor() {
    for css in [
        ".anchor{clip-path:inset(100%)}",
        ".anchor{transform:scale(0)}",
        ".anchor{filter:url(#empty)}",
    ] {
        let trace = render(
            &format!(
                "{css}.target{{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}}"
            ),
            "<svg style='display:none'><defs><filter id='empty'><feColorMatrix type='matrix' values='0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0'/></filter></defs></svg><div class=anchor></div><div class=target></div>",
        );
        assert_eq!(
            targets(&trace).len(),
            1,
            "{css} does not add an intervening ancestor clip"
        );
    }
}

#[test]
fn visible_children_of_hidden_filtered_boxes_contribute_anchor_ink() {
    let trace = render(
        "section{width:20pt;height:40pt;overflow:hidden}.anchor{margin-left:30pt;width:10pt;height:10pt}.hidden{visibility:hidden;filter:drop-shadow(-20pt 0 green)}.child{visibility:visible;width:1pt;height:5pt;background:green}.target{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}",
        "<section><div class=anchor><div class=hidden><div class=child></div></div></div></section><div class=target></div>",
    );
    assert_eq!(
        targets(&trace).len(),
        1,
        "a visible descendant still passes through its hidden parent's filter"
    );
    assert_rect(targets(&trace)[0].1, [50.0, 30.0, 20.0, 10.0]);
    let trace = render(
        "section{width:20pt;height:40pt;overflow:hidden}.anchor{margin-left:30pt;width:10pt;height:10pt}.hidden{visibility:hidden;width:10pt;height:10pt;filter:drop-shadow(-20pt 0 green)}.target{position-visibility:anchors-visible}",
        "<section><div class=anchor><div class=hidden></div></div></section><div class=target></div>",
    );
    assert!(
        targets(&trace).is_empty(),
        "a hidden empty filtered box contributes no input ink"
    );
}
