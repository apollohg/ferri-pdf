use super::common;
use crate::{DisplayItem, render_html};
use common::{PROFILE, assert_pt, local_options};

fn render(css: &str, body: &str) -> crate::RenderOutput {
    render_html(&format!("<style>{PROFILE}ul{{margin:0;padding-left:40pt}}li{{font:12pt/18pt Spike}}{css}</style>{body}"), &local_options())
        .unwrap_or_else(|error| panic!("image marker render failed: {error}; {css}; {body}"))
}

#[test]
fn gradient_markers_use_one_em_and_precede_inside_text() {
    let output = render(
        "li{list-style:inside none;list-style-image:linear-gradient(red,blue)}",
        "<ul><li>Item</li></ul>",
    );
    let trace = output.layout.unwrap();
    let marker =
        trace.display_lists[0]
            .iter()
            .find_map(|item| match item {
                DisplayItem::Box(paint)
                    if !paint.decoration.backgrounds.images.iter().all(|image| {
                        matches!(image, ferri_pdf_graphics::BackgroundImage::None)
                    }) =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .expect("image marker has a paint box");
    assert_pt(marker.rect.width, 12.0);
    assert_pt(marker.rect.height, 12.0);
    let item = trace.pages[0]
        .iter()
        .find(|line| line.text.contains("Item"))
        .unwrap();
    let run = item
        .runs
        .iter()
        .find(|run| run.text.contains("Item"))
        .unwrap();
    assert!(item.x + run.x >= marker.rect.x + marker.rect.width);
}

#[test]
fn unavailable_marker_images_fall_back_without_aborting_the_pdf() {
    for source in ["missing.png", "data:image/png;base64,YmFk"] {
        let output = render(
            &format!("li{{list-style-type:decimal;list-style-image:url('{source}')}}"),
            "<ul><li>Item</li><li>Next</li></ul>",
        );
        assert_eq!(
            output.diagnostics.len(),
            1,
            "deduplicate failed source {source}"
        );
        let trace = output.layout.unwrap();
        let text: Vec<_> = trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect();
        assert!(text.contains(&"1. "), "missing fallback: {text:?}");
        assert!(text.contains(&"2. "), "missing fallback: {text:?}");
    }
}

fn markers(trace: &crate::LayoutTrace) -> Vec<(usize, &ferri_pdf_graphics::BoxPaint)> {
    trace
        .display_lists
        .iter()
        .enumerate()
        .flat_map(|(page, items)| {
            items.iter().filter_map(move |item| match item {
                DisplayItem::Box(paint)
                    if paint.decoration.backgrounds.images.iter().any(|image| {
                        !matches!(image, ferri_pdf_graphics::BackgroundImage::None)
                    }) =>
                {
                    Some((page, paint))
                }
                _ => None,
            })
        })
        .collect()
}

#[test]
fn outside_image_markers_share_the_first_baseline_and_obey_direction() {
    for direction in ["ltr", "rtl"] {
        let output = render(
            &format!(
                "li{{direction:{direction};padding:5pt 7pt;list-style-image:linear-gradient(red,blue)}}"
            ),
            "<ul><li><p>Item</p></li></ul>",
        );
        let trace = output.layout.unwrap();
        let markers = markers(&trace);
        assert_eq!(markers.len(), 1);
        let paint = markers[0].1;
        let text = trace.pages[0]
            .iter()
            .find(|line| line.text == "Item")
            .unwrap();
        assert_pt(paint.rect.y + paint.rect.height, text.baseline);
        if direction == "ltr" {
            assert!(paint.rect.x + paint.rect.width < text.x - 7.0);
        } else {
            assert!(paint.rect.x > text.x + text.width + 7.0);
        }
    }
}

#[test]
fn svg_marker_sizing_uses_intrinsics_and_a_one_em_default_object() {
    for (attributes, expected) in [
        ("width='16' height='8'", [12.0, 6.0]),
        ("viewBox='0 0 2 1'", [12.0, 6.0]),
        ("viewBox='0 0 1 2'", [6.0, 12.0]),
        ("width='20' viewBox='0 0 2 1'", [15.0, 7.5]),
        ("", [12.0, 12.0]),
    ] {
        for position in ["inside", "outside"] {
            let css = format!(
                "li{{list-style-position:{position};list-style-image:url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' {attributes}><rect width='100%' height='100%' fill='red'/></svg>\")}}"
            );
            let output = render(&css, "<ul><li>Item</li></ul>");
            assert!(
                output.diagnostics.is_empty(),
                "{attributes}: {:?}",
                output.diagnostics
            );
            let trace = output.layout.unwrap();
            let boxes = markers(&trace);
            assert_eq!(boxes.len(), 1, "{attributes} {position}");
            assert_pt(boxes[0].1.rect.width, expected[0]);
            assert_pt(boxes[0].1.rect.height, expected[1]);
        }
    }
}

#[test]
fn markers_are_inherited_reset_by_shorthand_and_emitted_once_after_fragmentation() {
    let output = render(
        "@page{size:180pt 100pt;margin:15pt}ul{list-style-image:linear-gradient(red,blue)}li{widows:1;orphans:1}.reset{list-style:decimal}",
        "<ul><li>One<br>Two<br>Three<br>Four<br>Five</li><li class=reset>Reset</li><li></li></ul>",
    );
    let trace = output.layout.unwrap();
    assert!(trace.pages.len() >= 2);
    assert_eq!(
        markers(&trace).len(),
        2,
        "one image per non-reset item, including empty item"
    );
    assert!(trace.pages.iter().flatten().any(|line| line.text == "2. "));
}

#[test]
fn marker_image_capability_queries_use_the_real_image_grammar() {
    let output = render(
        "li{list-style:none}@supports(list-style-image:radial-gradient(red,blue)){li{list-style-image:radial-gradient(red,blue)}}",
        "<ul><li>Item</li></ul>",
    );
    assert_eq!(markers(&output.layout.unwrap()).len(), 1);
}

#[test]
fn nested_and_empty_items_keep_their_own_marker_font_size() {
    for position in ["inside", "outside"] {
        let output = render(
            &format!(
                "ul{{list-style-image:linear-gradient(red,blue)}}li{{list-style-position:{position}}}"
            ),
            "<ul><li>Outer<ul><li style='font-size:18pt'>Inner</li><li></li></ul></li></ul>",
        );
        let trace = output.layout.unwrap();
        let boxes = markers(&trace);
        let mut sizes = boxes
            .iter()
            .map(|(_, paint)| paint.rect.width)
            .collect::<Vec<_>>();
        sizes.sort_by(f32::total_cmp);
        assert_eq!(
            sizes,
            [12.0, 12.0, 18.0],
            "{position}: each nested/empty item owns exactly one marker"
        );
        for (_, paint) in boxes {
            assert_pt(paint.rect.height, paint.rect.width);
        }
    }
}
