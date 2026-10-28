mod common;
use common::{PROFILE, assert_pt, local_options, painted_box};
use ferri_pdf::{LayoutTrace, render_html};
fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!(
            "<style>{PROFILE}html,body,p{{margin:0}}.grid,.sub{{display:grid}}{css}</style>{body}"
        ),
        &local_options(),
    )
    .unwrap()
    .layout
    .unwrap()
}
#[test]
fn fixed_parent_columns_and_rows_reach_descendant_boxes() {
    let trace = render(
        ".grid{width:210pt;grid-template-columns:80pt 120pt;grid-template-rows:30pt 40pt;gap:10pt}.sub{grid-column:1/-1;grid-row:1/-1;grid-template-columns:subgrid;grid-template-rows:subgrid}.a{background:red}.b{background:blue;grid-row:2;grid-column:2}",
        "<div class=grid><div class=sub><div class=a>A</div><div class=b>B</div></div></div>",
    );
    for (color, x, y, w, h) in [
        ([255, 0, 0], 36.0, 36.0, 80.0, 30.0),
        ([0, 0, 255], 126.0, 76.0, 120.0, 40.0),
    ] {
        let rect = painted_box(&trace, color);
        assert_pt(rect.x, x);
        assert_pt(rect.y, y);
        assert_pt(rect.width, w);
        assert_pt(rect.height, h);
    }
}
#[test]
fn descendants_size_intrinsic_parent_columns() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:auto 100pt;justify-content:start}.sub{grid-column:1/-1;grid-template-columns:subgrid}.a{width:60pt;height:20pt;background:red}.b{height:20pt;background:blue}",
        "<div class=grid><div class=sub><div class=a></div><div class=b></div></div></div>",
    );
    let rect = painted_box(&trace, [0, 0, 255]);
    assert_pt(rect.x, 96.0);
    assert_pt(rect.width, 100.0);
}
#[test]
fn descendants_size_intrinsic_parent_rows() {
    let trace = render(
        ".grid{width:100pt;grid-template-columns:100pt;grid-template-rows:auto auto;row-gap:10pt}.sub{grid-row:1/-1;grid-template-rows:subgrid}.a{height:30pt;background:red}.b{height:40pt;background:blue}",
        "<div class=grid><div class=sub><div class=a></div><div class=b></div></div></div>",
    );
    let rect = painted_box(&trace, [0, 0, 255]);
    assert_pt(rect.y, 76.0);
    assert_pt(rect.height, 40.0);
}
#[test]
fn automatic_subgrid_span_comes_from_local_line_name_count() {
    let trace = render(
        ".grid{width:150pt;grid-template-columns:40pt 50pt 60pt}.sub{grid-template-columns:subgrid [a] [b] [c]}.a{height:20pt;background:red}.b{height:20pt;background:blue}.next{height:20pt;background:lime}",
        "<div class=grid><div class=sub><div class=a></div><div class=b></div></div><div class=next></div></div>",
    );
    let rect = painted_box(&trace, [0, 0, 255]);
    assert_pt(rect.x, 76.0);
    assert_pt(rect.width, 50.0);
    let rect = painted_box(&trace, [0, 255, 0]);
    assert_pt(rect.x, 126.0);
    assert_pt(rect.width, 60.0);
}
#[test]
fn paginated_shared_rows_preserve_text_once() {
    let trace = render(
        "@page{size:200pt 150pt;margin:10pt}.grid{width:100pt;grid-template-columns:100pt;grid-template-rows:100pt 100pt}.sub{grid-row:1/-1;grid-template-rows:subgrid}.a{background:red}.b{background:blue}",
        "<div class=grid><div class=sub><div class=a>FIRST</div><div class=b>SECOND</div></div></div><p>AFTER</p>",
    );
    assert!(trace.pages.len() >= 2);
    let words = trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>();
    for label in ["FIRST", "SECOND", "AFTER"] {
        assert_eq!(
            words.iter().filter(|text| **text == label).count(),
            1,
            "{label}: {words:?}"
        );
    }
}
#[test]
fn shared_axis_ignores_size_constraints_and_centres_changed_gutters() {
    let trace = render(
        ".grid{width:300pt;grid-template-columns:100pt 150pt;column-gap:50pt}.sub{grid-column:1/-1;grid-template-columns:subgrid;column-gap:0;padding-left:10pt;padding-right:20pt;width:1pt;min-width:2pt;max-width:3pt;justify-self:end}.a{height:20pt;background:red}.b{height:20pt;background:blue}",
        "<div class=grid><div class=sub><div class=a></div><div class=b></div></div></div>",
    );
    let first = painted_box(&trace, [255, 0, 0]);
    assert_pt(first.x, 46.0);
    assert_pt(first.width, 115.0);
    let second = painted_box(&trace, [0, 0, 255]);
    assert_pt(second.x, 161.0);
    assert_pt(second.width, 155.0);
}

#[test]
fn definite_percentage_parent_gap_affects_descendant_intrinsic_contributions() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:auto auto;column-gap:10%;justify-content:start}.sub{grid-column:1/-1;grid-template-columns:subgrid;column-gap:0;background:lime}.a,.b{width:60pt;height:20pt}.a{background:red}.b{background:blue}",
        "<div class=grid><div class=sub><div class=a></div><div class=b></div></div></div>",
    );
    let area = painted_box(&trace, [0, 255, 0]);
    assert_pt(area.width, 120.0);
    let second = painted_box(&trace, [0, 0, 255]);
    assert_pt(second.x, 96.0);
}

#[test]
fn nested_normal_gap_preserves_resolved_ancestor_percentage_gap() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:auto auto;column-gap:10%;justify-content:start}.sub{grid-column:1/-1;grid-template-columns:subgrid}.inner{column-gap:0;background:lime}.a,.b{width:60pt;height:20pt}.b{background:blue}",
        "<div class=grid><div class=sub><div class='sub inner'><div class=a></div><div class=b></div></div></div></div>",
    );
    assert_pt(painted_box(&trace, [0, 255, 0]).width, 120.0);
    assert_pt(painted_box(&trace, [0, 0, 255]).x, 96.0);
}

#[test]
fn cyclic_child_percentage_gap_is_zero_for_intrinsic_sizing_then_resolves_for_layout() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:auto auto;justify-content:start}.sub{grid-column:1/-1;grid-template-columns:subgrid;column-gap:10%;background:lime}.a,.b{width:60pt;height:20pt}.b{background:blue}",
        "<div class=grid><div class=sub><div class=a></div><div class=b></div></div></div>",
    );
    assert_pt(painted_box(&trace, [0, 255, 0]).width, 120.0);
    assert_pt(painted_box(&trace, [0, 0, 255]).x, 102.0);
}

#[test]
fn definite_percentage_row_gap_affects_descendant_height_contributions() {
    let trace = render(
        ".grid{width:100pt;height:200pt;grid-template-columns:100pt;grid-template-rows:auto auto;row-gap:10%;align-content:start}.sub{grid-row:1/-1;grid-template-rows:subgrid;row-gap:0;background:lime}.a,.b{height:60pt}.b{background:blue}",
        "<div class=grid><div class=sub><div class=a></div><div class=b></div></div></div>",
    );
    assert_pt(painted_box(&trace, [0, 255, 0]).height, 120.0);
    assert_pt(painted_box(&trace, [0, 0, 255]).y, 96.0);
}

#[test]
fn continuing_subgrids_rewrap_against_destination_page_tracks() {
    for (first_width, later_width, depth) in
        [(220, 420, 1), (420, 220, 1), (220, 420, 2), (420, 220, 2)]
    {
        let wrap = |contents: &str| {
            format!(
                "<div class=grid>{}{contents}{}",
                "<div class=sub>".repeat(depth),
                "</div>".repeat(depth + 1)
            )
        };
        let css = format!(
            "@page{{size:{later_width}pt 180pt;margin:10pt}}@page:first{{size:{first_width}pt 180pt}}.grid{{grid-template-columns:1fr 1fr;gap:10pt}}.sub{{grid-column:1/-1;grid-template-columns:subgrid}}.next{{break-before:page}}"
        );
        let trace = render(
            &css,
            &wrap(
                "<div><p>FirstA</p><p class=next>SecondA alpha beta gamma delta epsilon zeta eta theta</p></div><div><p>FirstB</p><p class=next>SecondB</p></div>",
            ),
        );
        assert_eq!(trace.pages.len(), 2, "{first_width} to {later_width}");
        let expected = render(
            &format!(
                "@page{{size:{later_width}pt 180pt;margin:10pt}}.grid{{grid-template-columns:1fr 1fr;gap:10pt}}.sub{{grid-column:1/-1;grid-template-columns:subgrid}}"
            ),
            &wrap(
                "<div><p>SecondA alpha beta gamma delta epsilon zeta eta theta</p></div><div><p>SecondB</p></div>",
            ),
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
            "continued subgrid must wrap at destination widths: {first_width} to {later_width}"
        );
    }
}

#[test]
fn descendants_share_baselines_with_parent_grid_peers() {
    for (small, peer, depth) in [(10, 30, 1), (30, 10, 1), (10, 30, 2), (30, 10, 2)] {
        let css = format!(
            ".grid{{width:200pt;grid-template-columns:100pt 100pt;align-items:baseline}}.sub{{grid-template-rows:subgrid;align-items:baseline;padding-top:5pt}}.small{{font-size:{small}pt;line-height:{}pt}}.peer{{font-size:{peer}pt;line-height:{}pt}}",
            small * 6 / 5,
            peer * 6 / 5
        );
        let body = format!(
            "<div class=grid>{}<div class=small>Small</div>{}<div class=peer>Peer</div></div><p>After</p>",
            "<div class=sub>".repeat(depth),
            "</div>".repeat(depth)
        );
        let trace = render(&css, &body);
        let baseline = |trace: &LayoutTrace, text: &str| {
            trace.pages[0]
                .iter()
                .find(|line| line.text == text)
                .unwrap()
                .baseline
        };
        assert_pt(baseline(&trace, "Small"), baseline(&trace, "Peer"));
        let flat = render(
            &format!("{css}.small{{padding-top:{}pt}}", depth * 5),
            "<div class=grid><div class=small>Small</div><div class=peer>Peer</div></div><p>After</p>",
        );
        assert_pt(baseline(&trace, "Small"), baseline(&flat, "Small"));
        assert_pt(baseline(&trace, "After"), baseline(&flat, "After"));
    }
}

#[test]
fn baseline_shifts_inflate_hypothetical_items_across_empty_edge_tracks() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:100pt 100pt;grid-template-rows:5pt auto 5pt;align-items:baseline;background:lime}.sub{grid-row:1/-1;grid-template-rows:subgrid;align-items:baseline;padding-bottom:50pt}.small{grid-row:2;font-size:10pt;line-height:12pt}.peer{grid-row:2;font-size:30pt;line-height:36pt}",
        "<div class=grid><div class=sub><div class=small>Small</div></div><div class=peer>Peer</div></div>",
    );
    let natural = render(
        ".small{font-size:10pt;line-height:12pt}",
        "<div class=small>Small</div>",
    );
    let baseline = |trace: &LayoutTrace, label: &str| {
        trace.pages[0]
            .iter()
            .find(|line| line.text == label)
            .unwrap()
            .baseline
    };
    assert_pt(baseline(&trace, "Small"), baseline(&trace, "Peer"));
    let shim = baseline(&trace, "Peer") - 5.0 - baseline(&natural, "Small");
    assert_pt(
        painted_box(&trace, [0, 255, 0]).height,
        5.0 + 12.0 + 50.0 + shim,
    );
}

#[test]
fn leading_padding_larger_than_empty_edge_track_preserves_baseline_alignment() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:100pt 100pt;grid-template-rows:5pt auto 5pt;align-items:baseline}.sub{grid-row:1/-1;grid-template-rows:subgrid;align-items:baseline;padding-top:25pt}.small{grid-row:2;font-size:10pt;line-height:12pt}.peer{grid-row:2;font-size:30pt;line-height:36pt}",
        "<div class=grid><div class=sub><div class=small>Small</div></div><div class=peer>Peer</div></div>",
    );
    let baseline = |text: &str| {
        trace.pages[0]
            .iter()
            .find(|line| line.text == text)
            .unwrap()
            .baseline
    };
    assert_pt(baseline("Small"), baseline("Peer"));
}

#[test]
fn percentage_height_images_share_their_final_baseline_with_parent_peers() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:100pt 100pt;grid-template-rows:60pt;align-items:baseline}.sub{grid-template-rows:subgrid;align-items:baseline}img{width:80pt;height:100%;align-self:baseline;background:red}.peer{font-size:30pt;line-height:36pt}",
        "<div class=grid><div class=sub><img src='../assets/image-pattern.png'></div><div class=peer>Peer</div></div>",
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

#[test]
fn standalone_subgrid_uses_ordinary_implicit_tracks_and_size_constraints() {
    let body = "<div class=sub><div class=a>A</div><div class=b>B</div></div><p>After</p>";
    let common = ".sub{width:140pt;height:80pt;grid-auto-columns:60pt;grid-auto-rows:30pt;gap:10pt;background:lime}.a{grid-column:1;background:red}.b{grid-column:2;background:blue}";
    let reference = render(common, body);
    let output = render(
        &format!("{common}.sub{{grid-template-columns:subgrid;grid-template-rows:subgrid}}"),
        body,
    );
    assert_eq!(
        serde_json::to_value(output).unwrap(),
        serde_json::to_value(reference).unwrap()
    );
}

#[test]
fn supports_subgrid_selects_the_rendered_shared_track_path() {
    let trace = render(
        ".grid{width:200pt;grid-template-columns:60pt 130pt;gap:10pt}.sub{grid-column:1/-1}.a{height:20pt;background:red}@supports(grid-template-columns:subgrid){.sub{grid-template-columns:subgrid}.a{grid-column:2}}",
        "<div class=grid><div class=sub><div class=a></div></div></div>",
    );
    let rect = painted_box(&trace, [255, 0, 0]);
    assert_pt(rect.x, 106.0);
    assert_pt(rect.width, 130.0);
}
