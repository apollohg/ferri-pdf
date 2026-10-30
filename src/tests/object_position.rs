use super::common;

use crate::graphics::ImagePaint;
use crate::{DisplayItem, LayoutTrace, render_html};
use common::{assert_pt, local_options};

const PROFILE: &str = "@page{size:400pt 600pt;margin:20pt}html,body,p{margin:0;padding:0;font:10pt/12pt serif}img{display:block;width:100pt;height:80pt;padding:10pt;border:2pt solid blue;object-fit:none}";
const IMAGE: &str = "../assets/image-pattern.png";

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!(
            "<!doctype html><style>{}{PROFILE}{css}</style>{body}",
            common::PROFILE
        ),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("object-position CSS `{css}` with `{body}`: {error}"))
    .layout
    .unwrap()
}

fn images(trace: &LayoutTrace) -> Vec<&ImagePaint> {
    trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            DisplayItem::Image(paint) => Some(paint),
            _ => None,
        })
        .collect()
}

fn assert_placement(paint: &ImagePaint, offset: [f32; 2], size: [f32; 2]) {
    let clip = paint
        .clip
        .as_ref()
        .expect("replaced content always clips to its content box");
    assert_eq!([clip.rect.width, clip.rect.height], [100.0, 80.0]);
    assert_pt(paint.rect.x - clip.rect.x, offset[0]);
    assert_pt(paint.rect.y - clip.rect.y, offset[1]);
    assert_eq!([paint.rect.width, paint.rect.height], size);
}

#[test]
fn object_positions_align_concrete_objects_without_changing_reserved_flow() {
    let mut following_baseline = None;
    for (fit, position, offset, size) in [
        ("none", "", [20.0, 25.0], [60.0, 30.0]),
        ("none", "top", [20.0, 0.0], [60.0, 30.0]),
        ("none", "right 10pt bottom 20pt", [30.0, 30.0], [60.0, 30.0]),
        ("none", "bottom 20pt right 10pt", [30.0, 30.0], [60.0, 30.0]),
        ("none", "25% 75%", [10.0, 37.5], [60.0, 30.0]),
        ("none", "calc(25% + 5pt) 0%", [15.0, 0.0], [60.0, 30.0]),
        ("none", "-20% 120%", [-8.0, 60.0], [60.0, 30.0]),
        ("none", "right -5pt bottom -2pt", [45.0, 52.0], [60.0, 30.0]),
        ("contain", "left bottom", [0.0, 30.0], [100.0, 50.0]),
        ("contain", "25% 75%", [0.0, 22.5], [100.0, 50.0]),
        ("cover", "right top", [-60.0, 0.0], [160.0, 80.0]),
        ("cover", "25% 75%", [-15.0, 0.0], [160.0, 80.0]),
        ("fill", "10pt -5pt", [10.0, -5.0], [100.0, 80.0]),
        ("fill", "75% 25%", [0.0, 0.0], [100.0, 80.0]),
        ("scale-down", "right bottom", [40.0, 50.0], [60.0, 30.0]),
    ] {
        let declarations = if position.is_empty() {
            format!("img{{object-fit:{fit}}}")
        } else {
            format!("img{{object-fit:{fit};object-position:{position}}}")
        };
        let trace = render(&declarations, &format!("<img src='{IMAGE}'><p>After</p>"));
        assert_eq!(trace.pages.len(), 1, "{fit}/{position} must preserve flow");
        let paints = images(&trace);
        assert_eq!(paints.len(), 1, "{fit}/{position} must paint one object");
        assert_placement(paints[0], offset, size);
        let clip = paints[0].clip.as_ref().unwrap().rect;
        assert_eq!([clip.x, clip.y], [32.0, 32.0]);
        let following = &trace.pages[0][0];
        assert_eq!(following.text, "After");
        let baseline = *following_baseline.get_or_insert(following.baseline);
        assert_pt(following.baseline, baseline);
    }
}

#[test]
fn object_position_defaulting_and_substitution_share_the_common_cascade() {
    let trace = render(
        "section{object-position:right bottom;--pos:bottom 20pt right 10pt}\
         .inherit{object-position:inherit}.unset{object-position:unset}\
         .invalid{object-position:right 10pt bottom 20pt;object-position:1pt 2pt 3pt}\
         .variable{object-position:var(--pos)}\
         .attribute{object-position:calc(25% + attr(data-x <length>)) 0%}\
         .missing{object-position:right bottom;object-position:var(--missing)}\
         .query{width:20pt}@supports(object-position:right 10pt bottom 20pt){.query{width:100pt}}",
        &format!(
            "<section><img src='{IMAGE}'><img class=inherit src='{IMAGE}'><img class=unset src='{IMAGE}'><img class=invalid src='{IMAGE}'><img class=variable src='{IMAGE}'><img class=attribute data-x=5pt src='{IMAGE}'><img class=missing src='{IMAGE}'><img class=query src='{IMAGE}'></section>"
        ),
    );
    let paints = images(&trace);
    assert_eq!(paints.len(), 8);
    for (paint, offset) in paints.iter().zip([
        [20.0, 25.0],
        [40.0, 50.0],
        [20.0, 25.0],
        [30.0, 30.0],
        [30.0, 30.0],
        [15.0, 0.0],
        [20.0, 25.0],
        [20.0, 25.0],
    ]) {
        assert_placement(paint, offset, [60.0, 30.0]);
        assert!(std::sync::Arc::ptr_eq(&paint.image, &paints[0].image));
    }
}

#[test]
fn equivalent_positions_match_computed_style_queries() {
    let mut failures = Vec::new();
    for property in ["object-position", "background-position"] {
        for (actual, equivalent) in [
            ("right center", "100% 50%"),
            (
                "right 10pt bottom 20pt",
                "calc(100% - 10pt) calc(100% - 20pt)",
            ),
            ("right calc(10pt + 20%) top 0pt", "calc(80% - 10pt) 0pt"),
            ("right 0pt top 0pt", "calc(100% + 0pt) 0pt"),
            ("left 10pt bottom 0pt", "10pt 100%"),
            ("left 0pt top 0pt", "0% 0%"),
        ] {
            let trace = render(
                &format!(
                    "img{{{property}:{actual};width:if(style({property}:{equivalent}):100pt;else:90pt)}}"
                ),
                &format!("<img src='{IMAGE}'>"),
            );
            let paints = images(&trace);
            assert_eq!(paints.len(), 1);
            let query_width = paints[0].clip.as_ref().unwrap().rect.width;
            if query_width != 100.0 {
                failures.push(format!(
                "{property} `{actual}` vs `{equivalent}`: query width {query_width} (expected 100)"
            ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "computed position equivalence failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn equivalent_object_positions_cycle_against_the_parent_computed_value() {
    let mut failures = Vec::new();
    for (actual, equivalent) in [
        ("right center", "100% 50%"),
        ("right 10pt bottom 20pt", "right 10pt bottom 20pt"),
        ("right 0pt top 0pt", "100% 0pt"),
        ("left 10pt bottom 0pt", "10pt 100%"),
        ("left 0pt top 0pt", "0% 0%"),
    ] {
        let trace = render(
            &format!(
                "section{{object-position:{actual}}}img{{object-position:toggle({equivalent},left bottom)}}"
            ),
            &format!("<section><img src='{IMAGE}'></section>"),
        );
        let paints = images(&trace);
        assert_eq!(paints.len(), 1);
        let clip = paints[0].clip.as_ref().unwrap().rect;
        let offset = [paints[0].rect.x - clip.x, paints[0].rect.y - clip.y];
        if offset != [0.0, 50.0] {
            failures.push(format!("parent `{actual}` vs toggle argument `{equivalent}`: offset {offset:?} (expected [0,50])"));
        }
    }
    assert!(
        failures.is_empty(),
        "computed position cycling failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn object_placement_is_shared_by_inline_positioned_flex_table_and_rtl_boxes() {
    for (css, body) in [
        ("", format!("<img src='{IMAGE}'>")),
        (
            "img{display:inline}",
            format!("<p>Before <img src='{IMAGE}'> After</p>"),
        ),
        (
            "img{position:absolute;left:40pt;top:40pt}",
            format!("<img src='{IMAGE}'>"),
        ),
        (
            "section{display:flex;width:300pt}img{flex:none}",
            format!("<section><img src='{IMAGE}'></section>"),
        ),
        (
            "table{border-collapse:collapse}td{padding:0}",
            format!("<table><tr><td><img src='{IMAGE}'></td></tr></table>"),
        ),
        ("body{direction:rtl}", format!("<img src='{IMAGE}'>")),
    ] {
        let trace = render(
            &format!("img{{object-position:right 10pt bottom 20pt}}{css}"),
            &body,
        );
        let paints = images(&trace);
        assert_eq!(paints.len(), 1, "mode CSS `{css}` must paint one object");
        assert_placement(paints[0], [30.0, 30.0], [60.0, 30.0]);
    }
}

#[test]
fn positioned_objects_keep_orientation_shared_assets_and_atomic_pagination() {
    let trace = render(
        "img{object-position:right bottom}img+img{break-before:page}",
        "<img src='../assets/image-rotated.heic'><img src='../assets/image-rotated.heic'>",
    );
    assert_eq!(trace.pages.len(), 2);
    let paints = images(&trace);
    assert_eq!(paints.len(), 2);
    for paint in &paints {
        assert_placement(paint, [70.0, 20.0], [30.0, 60.0]);
    }
    assert!(std::sync::Arc::ptr_eq(&paints[0].image, &paints[1].image));
}

#[test]
fn object_content_clip_keeps_rounded_content_edge_geometry() {
    let trace = render(
        "img{border-radius:30pt;object-position:right bottom}",
        &format!("<img src='{IMAGE}'>"),
    );
    let paints = images(&trace);
    assert_eq!(paints.len(), 1);
    assert_placement(paints[0], [40.0, 50.0], [60.0, 30.0]);
    assert_eq!(paints[0].clip.as_ref().unwrap().radii, [[18.0; 2]; 4]);
}
