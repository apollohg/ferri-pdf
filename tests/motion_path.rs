mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{DisplayItem, LayoutTrace, LinkShape, Rect, render_html};

const TOLERANCE_PT: f32 = 0.001;
const BOX_CSS: &str = "section{position:relative;width:200pt;height:100pt}#box{width:20pt;height:10pt;margin-left:40pt;background:red;font:8pt/10pt monospace}#after{width:20pt;height:5pt;background:blue}";
const BODY: &str =
    "<section><div id=box><a href='https://example.org/'>T</a></div><div id=after></div></section>";

fn trace(css: &str) -> LayoutTrace {
    trace_body(css, BODY)
}
fn trace_body(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{BOX_CSS}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("motion path fixture {css:?} must render: {error}"))
    .layout
    .expect("layout trace requested")
}

#[test]
fn path_distance_moves_paint_and_links_without_moving_layout_boxes() {
    // Motion Path 1 §2.1–2.7: 50% of a 160px path is 60pt from the containing box's origin.
    let css = "#box{offset-path:path('M0 0 H160');offset-position:0 0;offset-distance:50%;offset-anchor:center;offset-rotate:0deg}";
    let actual = trace(css);
    let matrices: Vec<_> = actual.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::PushTransform(matrix) => Some(*matrix),
            _ => None,
        })
        .collect();
    assert_eq!(
        matrices.len(),
        1,
        "motion alone must produce one shared transform: {matrices:?}"
    );
    let expected = [1.0, 0.0, 0.0, 1.0, 10.0, -5.0];
    assert!(
        matrices[0]
            .iter()
            .zip(expected)
            .all(|(a, b)| (a - b).abs() < TOLERANCE_PT),
        "path/anchor matrix {:?}, expected {expected:?}",
        matrices[0]
    );
    let mut boxes: Vec<_> = actual.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => Some(paint.rect),
            _ => None,
        })
        .collect();
    boxes.sort_by(|a, b| a.y.total_cmp(&b.y));
    assert_eq!(
        boxes,
        [
            Rect {
                x: 76.0,
                y: 36.0,
                width: 20.0,
                height: 10.0
            },
            Rect {
                x: 36.0,
                y: 46.0,
                width: 20.0,
                height: 5.0
            }
        ],
        "motion transforms paint while normal flow and box sizes remain intact"
    );
    let reference = trace("#box{transform:translate(10pt,-5pt)}");
    assert_eq!(
        actual.links, reference.links,
        "the shared motion effect must move link activation with painted glyphs"
    );
}

#[test]
fn none_disables_the_offset_transform_even_with_noninitial_offset_properties() {
    // Normative §2.1 overrides the older contradictory offset-position/anchor examples.
    let actual = trace(
        "#box{offset-path:none;offset-position:90% 80%;offset-distance:100%;offset-anchor:right bottom;offset-rotate:reverse 30deg}",
    );
    assert!(
        actual.display_lists[0]
            .iter()
            .all(|item| !matches!(item, DisplayItem::PushTransform(_))),
        "offset-path:none has no offset transform"
    );
    assert_eq!(
        actual.links,
        trace("").links,
        "none preserves ordinary link positions"
    );
}

fn matrices(layout: &LayoutTrace) -> Vec<[f32; 6]> {
    layout
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            DisplayItem::PushTransform(matrix) => Some(*matrix),
            _ => None,
        })
        .collect()
}
fn equivalent(motion: &str, transform: &str) {
    let actual = trace(&format!(
        "#box{{offset-anchor:0 0;offset-rotate:0deg;{motion}}}"
    ));
    let expected = trace(&format!(
        "#box{{transform-origin:0 0;transform:{transform}}}"
    ));
    let a = matrices(&actual);
    let e = matrices(&expected);
    assert_eq!(
        a.len(),
        e.len(),
        "motion {motion}: {a:?} vs transform {transform}: {e:?}"
    );
    for (actual, expected) in a.iter().zip(&e) {
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(a, e)| (a - e).abs() < TOLERANCE_PT),
            "motion {motion}: {actual:?} vs transform {transform}: {expected:?}"
        );
    }
    assert_eq!(actual.links.len(), expected.links.len());
    for (actual, expected) in actual.links.iter().zip(&expected.links) {
        assert_eq!(actual.len(), expected.len(), "link count follows {motion}");
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.link, expected.link);
            let points = |shape: &LinkShape| match shape {
                LinkShape::Rect(rect) => vec![rect.x, rect.y, rect.width, rect.height],
                LinkShape::Quads(quads) => quads.iter().flatten().flatten().copied().collect(),
            };
            let (a, e) = (points(&actual.shape), points(&expected.shape));
            assert_eq!(a.len(), e.len(), "link shape follows {motion}");
            assert!(
                a.iter().zip(&e).all(|(a, e)| (a - e).abs() < TOLERANCE_PT),
                "link geometry follows {motion}: {a:?} vs {e:?}"
            );
        }
    }
}

#[test]
fn open_closed_and_compound_paths_use_total_length_and_svg_directionality() {
    for (path, distance, transform) in [
        ("M0 0 H160", "-50%", "translate(-40pt,0pt)"),
        ("M0 0 H160", "150%", "translate(80pt,0pt)"),
        ("M0 0 H160 V160 H0 Z", "125%", "translate(80pt,0pt)"),
        ("M0 0 H160 V160 H0 Z", "-25%", "translate(-40pt,120pt)"),
        ("M0 0 H80 M160 80 V160", "75%", "translate(80pt,90pt)"),
        ("M20 40", "200%", "translate(-25pt,30pt)"),
    ] {
        equivalent(
            &format!("offset-path:path('{path}');offset-distance:{distance}"),
            transform,
        );
    }
    equivalent(
        "offset-path:path('M0 0 H80 V80');offset-distance:50%;offset-rotate:auto",
        "translate(20pt,0pt) rotate(90deg)",
    );
    equivalent(
        "offset-path:path('M0 0 H80 V80');offset-distance:75%;offset-rotate:reverse 30deg",
        "translate(20pt,30pt) rotate(300deg)",
    );
    equivalent(
        "offset-path:path('M0 0 C0 0 0 0 80 0');offset-distance:0%;offset-rotate:auto",
        "translate(-40pt,0pt)",
    );
}

#[test]
fn rays_resolve_starting_positions_extents_and_containment() {
    for (path, distance, transform) in [
        ("ray(90deg)", "100%", "translate(110pt,50pt)"),
        ("ray(90deg farthest-side)", "100%", "translate(160pt,50pt)"),
        ("ray(90deg sides)", "100%", "translate(160pt,50pt)"),
        ("ray(90deg contain)", "200%", "translate(100pt,50pt)"),
        ("ray(90deg contain)", "-10pt", "translate(60pt,50pt)"),
        ("ray(90deg)", "-25pt", "translate(35pt,50pt)"),
        ("ray(0deg at 20pt 30pt)", "10pt", "translate(-20pt,20pt)"),
    ] {
        equivalent(
            &format!("offset-path:{path};offset-distance:{distance}"),
            transform,
        );
    }
    equivalent(
        "offset-path:ray(90deg farthest-side);offset-position:auto;offset-distance:10pt",
        "translate(10pt,0pt)",
    );
    equivalent(
        "offset-path:ray(90deg at 10pt 20pt);offset-position:90% 90%;offset-distance:5pt",
        "translate(-25pt,20pt)",
    );
}

#[test]
fn basic_shapes_and_coordinate_boxes_start_at_the_normative_points() {
    // §2.1 applies a starting position only to functions accepting an omitted `at`.
    equivalent(
        "offset-path:path('M0 0 H160');offset-position:auto;offset-distance:50%",
        "translate(20pt,0pt)",
    );
    equivalent(
        "offset-path:circle(20pt at 50pt 30pt)",
        "translate(30pt,30pt)",
    );
    equivalent(
        "offset-path:ellipse(20pt 10pt at 50pt 30pt)",
        "translate(30pt,30pt)",
    );
    equivalent(
        "offset-path:circle(20pt);offset-position:50pt 30pt;offset-distance:25%",
        "translate(10pt,50pt)",
    );
    equivalent("offset-path:inset(10pt round 5pt)", "translate(-25pt,10pt)");
    equivalent(
        "offset-path:rect(10pt 180pt 80pt 20pt)",
        "translate(-20pt,10pt)",
    );
    equivalent(
        "offset-path:xywh(20pt 10pt 160pt 70pt)",
        "translate(-20pt,10pt)",
    );
    equivalent(
        "offset-path:polygon(10pt 20pt,100pt 20pt,100pt 80pt)",
        "translate(-30pt,20pt)",
    );
    equivalent(
        "offset-path:border-box;offset-position:80% 80%;offset-distance:50%",
        "translate(160pt,100pt)",
    );
}

#[test]
fn shorthand_resets_omitted_values_and_preserves_deferred_variables() {
    let long =
        "offset-path:path('M0 0 H160');offset-distance:50%;offset-anchor:0 0;offset-rotate:0deg";
    let short = "--path:path('M0 0 H160');offset:var(--path) 50% 0deg / 0 0";
    assert_eq!(
        matrices(&trace(&format!("#box{{{short}}}"))),
        matrices(&trace(&format!("#box{{{long}}}")))
    );
    let reset = trace(
        "#box{offset-path:path('M0 0 H160');offset-distance:90%;offset-anchor:right bottom;offset-rotate:reverse;offset:none}",
    );
    assert!(
        matrices(&reset).is_empty(),
        "offset:none resets all five properties and disables motion"
    );
    let inherited = "offset-path:circle(30pt);offset-distance:25%;offset-position:70pt 40pt;offset-anchor:80% 40%;offset-rotate:reverse 30deg";
    let parent = format!("section{{{inherited}}}");
    let actual = trace(&format!("{parent}#box{{offset:inherit}}"));
    let control = trace(&format!("{parent}#box{{{inherited}}}"));
    assert_eq!(
        matrices(&actual).len(),
        2,
        "the parent and inherited child each own a motion transform"
    );
    assert_matrices(
        &matrices(&actual),
        &matrices(&control),
        "offset:inherit copies all five parent computed values",
    );
    assert_eq!(
        actual.links, control.links,
        "inherited motion paints and activates links like explicit longhands"
    );
}

#[test]
fn svg_references_use_equivalent_shapes_viewport_units_and_invalid_reference_origin() {
    let body = format!(
        "{BODY}<svg xmlns='http://www.w3.org/2000/svg' style='display:none'><defs><path id='route' d='M0 0 H160'/><line id='diagonal' x1='10%' y1='20%' x2='90%' y2='80%'/><circle id='ring' cx='50%' cy='50%' r='20'/><g id='not-shape'><path d='M0 0 H999'/></g></defs></svg>"
    );
    for (url, distance, path) in [
        ("#route", "50%", "path('M0 0 H160')"),
        (
            "#diagonal",
            "50%",
            "path('M26.6666667 26.6666667 L240 106.6666667')",
        ),
        ("#ring", "0%", "circle(15pt at 100pt 50pt)"),
        ("#not-shape", "100%", "path('M0 0')"),
        ("#missing", "100%", "path('M0 0')"),
    ] {
        let css = |path: &str| {
            format!(
                "#box{{offset-path:{path};offset-distance:{distance};offset-anchor:0 0;offset-rotate:0deg}}"
            )
        };
        let actual = matrices(&trace_body(&css(&format!("url({url})")), &body));
        let expected = matrices(&trace_body(&css(path), &body));
        assert_eq!(actual.len(), expected.len());
        assert!(
            actual
                .iter()
                .flatten()
                .zip(expected.iter().flatten())
                .all(|(a, e)| (a - e).abs() < TOLERANCE_PT),
            "{url} at {distance}: {actual:?} vs {path}: {expected:?}"
        );
    }
}

#[test]
fn actual_containing_blocks_supply_boxes_across_static_ancestors_and_page_sizes() {
    let body = "<section style='position:relative;padding:10pt;border:5pt solid black;margin:30pt'><div style='margin-left:20pt;width:70pt;height:70pt'><div id=box style='position:absolute;left:5pt;top:7pt'></div></div></section>";
    let motion = trace_body(
        "#box{offset-path:inset(0) content-box;offset-anchor:0 0;offset-rotate:0deg;offset-distance:25%}",
        body,
    );
    // The containing element is section, not the intervening static div; its content path
    // has perimeter600pt, so 25% is150pt along its top edge.
    let reference = trace_body("#box{transform:translate(115pt,3pt)}", body);
    let actual = matrices(&motion);
    let expected = matrices(&reference);
    assert_eq!(
        actual, expected,
        "absolute offset-path references the full actual containing element, including content-box origin"
    );

    let body = "<section style='padding:10pt;border:5pt solid black;margin:30pt;transform:translate(2pt,3pt)'><div style='position:relative;width:70pt'><div id=box style='position:fixed;left:5pt;top:7pt'></div></div></section>";
    let motion = trace_body(
        "#box{offset-path:inset(0) content-box;offset-anchor:0 0;offset-rotate:0deg;offset-distance:25%}",
        body,
    );
    let reference = trace_body("#box{transform:translate(115pt,3pt)}", body);
    assert_eq!(
        matrices(&motion),
        matrices(&reference),
        "fixed offset-path references the transformed fixed CB across a positioned ancestor"
    );

    let body = "<section><div><div id=box></div></div></section>";
    let actual = trace_body(
        "section>div{width:120pt;height:40pt}#box{offset-path:ray(90deg farthest-side);offset-anchor:0 0;offset-rotate:0deg;offset-distance:100%}",
        body,
    );
    let expected = trace_body(
        "section>div{width:120pt;height:40pt}#box{transform:translate(80pt,20pt)}",
        body,
    );
    assert_matrices(
        &matrices(&actual),
        &matrices(&expected),
        "unpainted ordinary CB",
    );
}

#[test]
fn fixed_motion_references_the_used_area_of_each_physical_page() {
    let css = "@page{size:300pt 220pt;margin:30pt}@page:first{size:240pt 180pt;margin:20pt}#box{position:fixed;left:10pt;top:10pt;margin:0;offset-path:ray(90deg sides);offset-distance:100%;offset-anchor:0 0;offset-rotate:0deg}.next{break-before:page}";
    let actual = trace_body(css, "<div id=box></div><p>One</p><p class=next>Two</p>");
    let matrices = matrices(&actual);
    assert_matrices(
        &matrices,
        &[
            [1.0, 0.0, 0.0, 1.0, 190.0, 60.0],
            [1.0, 0.0, 0.0, 1.0, 230.0, 70.0],
        ],
        "each physical page's real content area",
    );
}

#[test]
fn external_svg_shape_references_and_non_svg_resources_follow_url_semantics() {
    equivalent(
        "offset-path:url('data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%3E%3Cpath%20id=%22route%22%20d=%22M0%200%20H160%22/%3E%3C/svg%3E#route');offset-distance:50%",
        "translate(20pt,0pt)",
    );
    equivalent(
        "offset-path:url('data:text/plain,not-svg#route');offset-distance:50%",
        "translate(-40pt,0pt)",
    );
}

fn assert_matrices(actual: &[[f32; 6]], expected: &[[f32; 6]], context: &str) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{context}: {actual:?} vs {expected:?}"
    );
    assert!(
        actual
            .iter()
            .flatten()
            .zip(expected.iter().flatten())
            .all(|(a, e)| (a - e).abs() < TOLERANCE_PT),
        "{context}: {actual:?} vs {expected:?}"
    );
}

#[test]
fn percentage_math_keeps_its_basis_through_nested_numeric_operations() {
    equivalent(
        "offset-path:path('M0 0 H160');offset-distance:calc(min(40, calc(50% / 1pt)) * 1pt)",
        "translate(0pt,0pt)",
    );
}

#[test]
fn motion_scrollable_overflow_waits_for_the_actual_containing_box_height() {
    let body =
        "<div class=grid><div class=owner><div id=box></div></div><div class=marker></div></div>";
    let css = ".grid{display:grid;position:relative;width:100pt;grid-template-columns:100pt;grid-template-rows:100pt}.owner{width:100pt}#box{margin:0}.marker{position:absolute;right:0;top:0;width:5pt;height:5pt;background:lime}";
    let actual = trace_body(
        &format!(
            "{css}#box{{offset-path:ray(90deg farthest-side at 50% 100%);offset-distance:100%;offset-anchor:0 0;offset-rotate:0deg}}"
        ),
        body,
    );
    let expected = trace_body(
        &format!("{css}#box{{transform:translate(150pt,100pt)}}"),
        body,
    );
    let marker = |layout: &LayoutTrace| {
        layout.display_lists[0]
            .iter()
            .find_map(|item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.rgb == [0, 255, 0] => {
                    Some(paint.rect)
                }
                _ => None,
            })
            .expect("the grid's absolute end marker paints")
    };
    assert_eq!(
        marker(&actual),
        marker(&expected),
        "the marker uses the final motion-expanded scrollable bounds, after its owner's auto height resolves"
    );
}

#[test]
fn table_motion_parts_and_cells_propagate_the_final_shared_scrollable_bounds() {
    let body = "<div class=grid><table><tbody><tr><td></td></tr></tbody></table><div class=marker></div></div>";
    let common = ".grid{display:grid;position:relative;width:100pt;grid-template-columns:100pt;grid-template-rows:40pt}table{width:40pt;border-spacing:0}td{padding:0;height:100pt}.marker{position:absolute;right:0;top:0;width:5pt;height:5pt;background:lime}";
    let marker = |layout: &LayoutTrace| {
        layout.display_lists[0]
            .iter()
            .find_map(|item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.rgb == [0, 255, 0] => {
                    Some(paint.rect)
                }
                _ => None,
            })
            .expect("grid overflow marker")
    };
    for part in ["table", "tbody", "tr", "td", "tbody,td"] {
        let movement = if part == "table" {
            "100pt,40pt"
        } else {
            "120pt,100pt"
        };
        let actual = trace_body(
            &format!(
                "{common}{part}{{offset-path:ray(90deg farthest-side at 50% 100%);offset-distance:100%;offset-anchor:0 0;offset-rotate:0deg}}"
            ),
            body,
        );
        let expected = trace_body(
            &format!("{common}{part}{{transform:translate({movement})}}"),
            body,
        );
        assert_eq!(
            marker(&actual),
            marker(&expected),
            "final scrollable bounds of {part} motion match the same ordinary transform, including both cell and structural ancestor effects"
        );
    }
}

#[test]
fn offset_anchor_and_transform_origin_compose_between_individual_scale_and_transform() {
    equivalent(
        "offset-path:path('M0 0 H160');offset-distance:50%;transform-origin:0 0;scale:2;transform:translate(10pt,0)",
        "scale(2) translate(30pt,0)",
    );
    equivalent(
        "offset-path:path('M80 0 V80');offset-distance:50%;offset-rotate:auto;transform-origin:right bottom",
        "translate(20pt,30pt) rotate(90deg)",
    );
    equivalent(
        "offset-path:path('M0 0 H160');offset-distance:50%;offset-anchor:auto;transform-origin:20% 80%",
        "translate(16pt,-8pt)",
    );
    equivalent(
        "offset-path:path('M0 0 H160');offset-distance:50%;offset-anchor:right bottom",
        "translate(0pt,-10pt)",
    );
    equivalent(
        "offset:0 0 ray(90deg) 10pt reverse / right bottom;offset:ray(90deg);offset-rotate:0deg",
        "translate(50pt,45pt)",
    );
}

#[test]
fn positioned_inline_containing_boxes_supply_their_real_border_padding_and_content_areas() {
    let body = "<section><span class=cb>Text<div id=box style='position:absolute;left:0;top:0'></div></span></section>";
    for (area, [top, right, bottom, left]) in [
        ("border-box", [0.0; 4]),
        ("padding-box", [2.0, 4.0, 6.0, 8.0]),
        ("content-box", [5.0, 11.0, 15.0, 13.0]),
    ] {
        let layout = trace_body(
            &format!(
                ".cb{{position:relative;padding:3pt 7pt 9pt 5pt;border:solid black;border-width:2pt 4pt 6pt 8pt;background:blue}}#box{{offset-path:circle(0) {area};offset-anchor:0 0;offset-rotate:0deg}}"
            ),
            body,
        );
        let paint = |color| {
            layout.display_lists[0]
                .iter()
                .find_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.background.rgb == color => {
                        Some(paint.rect)
                    }
                    _ => None,
                })
                .expect("the real inline CB and target paint")
        };
        let cb = paint([0, 0, 255]);
        let own = paint([255, 0, 0]);
        let reference = Rect {
            x: cb.x + left,
            y: cb.y + top,
            width: cb.width - left - right,
            height: cb.height - top - bottom,
        };
        assert_matrices(
            &matrices(&layout),
            &[[
                1.0,
                0.0,
                0.0,
                1.0,
                reference.x + reference.width / 2.0 - own.x,
                reference.y + reference.height / 2.0 - own.y,
            ]],
            area,
        );
    }
}

#[test]
fn motion_establishes_the_same_absolute_and_fixed_containing_block_as_a_transform() {
    for position in ["absolute", "fixed"] {
        let body = format!(
            "<section><div id=owner><div><div class=child style='position:{position}'><a href='https://example.org/'>T</a></div></div></div></section>"
        );
        let css = "#owner{width:100pt;height:50pt;margin-left:30pt;padding:5pt;border:2pt solid black}.child{right:0;bottom:0;width:10pt;height:10pt;background:lime}";
        let actual = trace_body(
            &format!("{css}#owner{{offset:path('M0 0 H160') 50% 0deg / 0 0}}"),
            &body,
        );
        let expected = trace_body(
            &format!("{css}#owner{{transform:translate(30pt,0)}}"),
            &body,
        );
        assert_matrices(&matrices(&actual), &matrices(&expected), position);
        assert_eq!(
            actual.links, expected.links,
            "{position} descendants use motion's containing block and shared paint effect"
        );
    }
}

#[test]
fn coordinate_only_paths_use_the_containing_elements_border_radius_value_in_the_chosen_box() {
    let css = "section{padding:10pt;border:5pt solid black;border-radius:20pt}#box{offset-anchor:0 0;offset-rotate:0deg}";
    for area in ["border-box", "padding-box", "content-box"] {
        let actual = trace(&format!("{css}#box{{offset-path:{area}}}"));
        let explicit = trace(&format!(
            "{css}#box{{offset-path:inset(0 round 20pt) {area}}}"
        ));
        assert_matrices(&matrices(&actual), &matrices(&explicit), area);
    }
}

#[test]
fn column_boxes_reference_in_flow_motion_but_absolute_motion_uses_the_multicol_element() {
    let css = "section{column-count:2;column-gap:20pt;column-fill:auto;height:60pt}.move{width:20pt;height:10pt;background:red;offset-path:ray(90deg farthest-side at 50% 100%);offset-distance:100%;offset-anchor:0 0;offset-rotate:0deg}.second{break-before:column}";
    let actual = trace_body(
        css,
        "<section><div class=move></div><div class='move second'></div></section>",
    );
    assert_matrices(
        &matrices(&actual),
        &[[1.0, 0.0, 0.0, 1.0, 105.0, 60.0]; 2],
        "anonymous90×60pt column CBs survive effect adoption into the page",
    );
    let actual = trace_body(
        &format!("{css}#box{{margin:0;position:absolute;left:0;top:0}}"),
        "<section><p>One</p><p class=second>Two</p><div id=box class=move></div></section>",
    );
    assert_matrices(
        &matrices(&actual),
        &[[1.0, 0.0, 0.0, 1.0, 200.0, 60.0]],
        "absolute motion skips anonymous columns and references the200×60pt multicol CB",
    );
}

#[test]
fn same_document_absolute_urls_and_percent_encoded_fragments_reuse_the_first_real_target() {
    use ferri_pdf::{
        Resource, ResourceBase, ResourceError, ResourceResolver, Url, render_with_resolver,
    };
    use std::cell::Cell;
    struct DenyFetch(Cell<usize>);
    impl ResourceResolver for DenyFetch {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.0.set(self.0.get() + 1);
            Err(ResourceError::Denied(base.resolve(reference)?))
        }
    }
    const DOCUMENT: &str = "https://example.test/motion.html";
    let resolver = DenyFetch(Cell::new(0));
    let options = ferri_pdf::RenderOptions {
        base: ResourceBase::Url(Url::parse(DOCUMENT).unwrap()),
        include_layout: true,
        ..Default::default()
    };
    for (reference, expected) in [
        ("https://example.test/motion.html#r%6Fute", 20.0),
        ("#duplicate", -40.0),
    ] {
        let html = format!(
            "<style>@page{{size:360pt 480pt;margin:36pt}}html,body{{margin:0;padding:0}}{BOX_CSS}#box{{offset-path:url('{reference}');offset-distance:50%;offset-anchor:0 0;offset-rotate:0deg}}</style><div id=duplicate style='display:none'></div><section><div id=box></div></section><svg style='display:none'><defs><path id=route d='M0 0 H160'/><path id=duplicate d='M0 0 H999'/></defs></svg>"
        );
        let layout = render_with_resolver(&html, &options, &resolver)
            .expect("local URL target needs no resource fetch")
            .layout
            .unwrap();
        assert_matrices(
            &matrices(&layout),
            &[[1.0, 0.0, 0.0, 1.0, expected, 0.0]],
            reference,
        );
    }
    assert_eq!(
        resolver.0.get(),
        0,
        "same-document URL forms must use the document target, including a non-SVG first duplicate ID"
    );
}

#[test]
fn page_fragment_references_use_each_containing_fragment_after_its_height_finishes() {
    let css = "@page{size:240pt 140pt;margin:10pt}section{width:100pt;height:auto;padding:3pt 7pt 5pt 11pt;border:2pt solid black;background:navy}.move{width:20pt;height:10pt;background:red;offset-path:ray(90deg farthest-side at 50% 100%) content-box;offset-distance:100%;offset-anchor:0 0;offset-rotate:0deg}.second{break-before:page}";
    let layout = trace_body(
        css,
        "<section><div class=move></div><div style='height:30pt'></div><div class='move second'></div></section>",
    );
    assert_eq!(
        layout.display_lists.len(),
        2,
        "the containing block fragments across two pages"
    );
    let expected: Vec<_> = layout
        .display_lists
        .iter()
        .map(|items| {
            let find = |color| {
                items
                    .iter()
                    .find_map(|item| match item {
                        DisplayItem::Box(paint) if paint.decoration.background.rgb == color => {
                            Some(paint)
                        }
                        _ => None,
                    })
                    .expect("both the containing fragment and motion child paint on each page")
            };
            let parent = find([0, 0, 128]);
            let reference = parent.inner_clip(parent.padding, [true; 2]).rect;
            let own = find([255, 0, 0]).rect;
            [
                1.0,
                0.0,
                0.0,
                1.0,
                reference.x + reference.width / 2.0 + (reference.width / 2.0).max(reference.height)
                    - own.x,
                reference.y + reference.height - own.y,
            ]
        })
        .collect();
    assert_matrices(
        &matrices(&layout),
        &expected,
        "per-page containing fragments supply their actual content geometry",
    );
    assert_ne!(
        expected[0][5], expected[1][5],
        "different fragment heights make this regression sensitive to stale or source-box height"
    );
}

#[test]
fn a_complete_closed_loop_uses_the_first_tangent_even_with_irrational_segment_length() {
    for distance in [
        "100%",
        "calc(100% + 0pt)",
        "200%",
        "calc(200% + 0pt)",
        "calc(-100% + 0pt)",
        "-100%",
    ] {
        equivalent(
            &format!(
                "offset-path:path('M0 0 H80 V80 Z');offset-distance:{distance};offset-rotate:auto"
            ),
            "translate(-40pt,0pt)",
        );
    }
}

#[test]
fn svg_equivalent_paths_retain_zero_size_shapes_and_single_point_subpaths() {
    let body = format!(
        "{BODY}<svg style='display:none'><defs><circle id=point cx=40 cy=60 r=0/><ellipse id=flat cx=40 cy=60 rx=20 ry=0/><rect id=line x=40 y=60 width=20 height=0/><polyline id=single points='40,60'/><path id=prefix d='M40 60 invalid'/></defs></svg>"
    );
    for (id, distance, expected) in [
        ("point", "50%", "path('M40 60')"),
        ("flat", "0%", "path('M60 60 H40 H20 H40 H60 Z')"),
        ("line", "25%", "path('M40 60 H60 V60 H40 Z')"),
        ("single", "100%", "path('M40 60')"),
        ("prefix", "100%", "path('M40 60')"),
    ] {
        let css = |path: &str| {
            format!(
                "#box{{offset-path:{path};offset-distance:{distance};offset-anchor:0 0;offset-rotate:0deg}}"
            )
        };
        assert_matrices(
            &matrices(&trace_body(&css(&format!("url(#{id})")), &body)),
            &matrices(&trace_body(&css(expected), &body)),
            id,
        );
    }
}

#[test]
fn anonymous_column_motion_overflow_waits_for_the_final_auto_balanced_extent() {
    let body =
        "<div class=grid><div class=owner><div id=box></div></div><div class=marker></div></div>";
    let css = ".grid{display:grid;position:relative;width:100pt;grid-template-columns:100pt;grid-template-rows:20pt}.owner{align-self:start;width:100pt;column-count:2;column-gap:10pt;column-fill:balance}#box{margin:0;break-inside:avoid}.marker{position:absolute;right:0;bottom:0;width:5pt;height:5pt;background:lime}";
    let marker = |layout: &LayoutTrace| {
        layout.display_lists[0]
            .iter()
            .find_map(|item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.rgb == [0, 255, 0] => {
                    Some(paint.rect)
                }
                _ => None,
            })
            .expect("the grid's end marker paints from its final scrollable area")
    };
    for (extra, path, distance, translation) in [
        (
            "",
            "ray(90deg farthest-side at 100% 100%)",
            "200%",
            "135pt,10pt",
        ),
        (
            ".owner{height:10pt}#box{width:0;height:0}",
            "path('M0 0 H180')",
            "100%",
            "135pt,0",
        ),
    ] {
        let actual = trace_body(
            &format!(
                "{css}{extra}#box{{offset-path:{path};offset-distance:{distance};offset-anchor:0 0;offset-rotate:0deg}}"
            ),
            body,
        );
        let expected = trace_body(
            &format!("{css}{extra}#box{{transform:translate({translation})}}"),
            body,
        );
        assert_eq!(
            marker(&actual),
            marker(&expected),
            "column motion contributes final45×10pt reference geometry before the grid places its end marker; {extra}"
        );
    }
}
