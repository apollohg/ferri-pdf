use crate::dom::{DocumentTree, HTML_NAMESPACE};
use crate::style::print::{Presentation, PrintEnvironment};
use crate::style::source_set::{dimension_source, select};
use crate::style::{ImageSource, POINTS_PER_PIXEL, PrintSettings};

/// A 600px-wide page sheet, so `100vw` is 600 CSS pixels.
const PAPER_PIXELS: [f32; 2] = [600.0, 800.0];

fn environment() -> PrintEnvironment {
    PrintEnvironment::new(
        &PrintSettings {
            paper: PAPER_PIXELS.map(|pixels| pixels * POINTS_PER_PIXEL),
            ..Default::default()
        },
        Presentation::Paged,
    )
    .unwrap()
}

fn selected(html: &str) -> (Option<ImageSource>, Option<String>) {
    let tree = DocumentTree::parse(html).unwrap();
    let img = tree
        .elements()
        .find(|element| element.namespace() == HTML_NAMESPACE && element.name() == "img")
        .expect("the probe has an img");
    let environment = environment();
    let dimensions = dimension_source(img, &environment);
    (
        select(img, &environment),
        dimensions.attr("id").map(str::to_owned),
    )
}

fn source(url: &str, density: f32) -> ImageSource {
    ImageSource {
        url: url.into(),
        density,
    }
}

#[test]
fn infinite_print_resolution_selects_the_densest_distinct_candidate() {
    for (html, expected) in [
        ("<img src=a.png>", Some(source("a.png", 1.0))),
        (
            "<img src=a.png srcset='b.png 2x, c.png 3x'>",
            Some(source("c.png", 3.0)),
        ),
        (
            "<img src=a.png srcset='b.png 1x, c.png 1x'>",
            Some(source("b.png", 1.0)),
        ),
        (
            "<img srcset='b.png 2x, c.png 2x, d.png 0.5x'>",
            Some(source("b.png", 2.0)),
        ),
        ("<img src=''>", None),
        ("<img srcset=' , ,'>", None),
        ("<img srcset='a.png 1e1x'>", Some(source("a.png", 10.0))),
    ] {
        assert_eq!(selected(html).0, expected, "{html}");
    }
}

#[test]
fn width_descriptors_divide_by_the_first_matching_source_size() {
    for (sizes, size) in [
        ("", 600.0),
        ("300px", 300.0),
        ("(min-width: 10000px) 10px, 150px", 150.0),
        ("(max-width: 700px) 50vw, 10px", 300.0),
        ("(width: 600px) 132px, 10px", 132.0),
        ("calc(100px + 2em), 10px", 132.0),
        ("(min-width: 1px)calc(132px), 10px", 132.0),
        ("auto, 120px", 120.0),
        ("-5px, 75px", 75.0),
        ("50%, 60px", 60.0),
        ("(min-width: 10000px) 10px", 600.0),
    ] {
        let html = format!("<img srcset='w.png 1200w' sizes='{sizes}'>");
        let density = selected(&html).0.expect("one width candidate").density;
        assert!(
            (density - 1200.0 / size).abs() < 1e-4,
            "sizes {sizes:?} must give a {size}px source size, got density {density}"
        );
    }
}

#[test]
fn srcset_descriptor_errors_drop_only_their_candidate() {
    for (srcset, expected) in [
        ("a.png 2x 3x, b.png 1x", Some(source("b.png", 1.0))),
        ("a.png 100w 2x, b.png", Some(source("b.png", 1.0))),
        ("a.png 0w, b.png 5x", Some(source("b.png", 5.0))),
        ("a.png 10h, b.png 1x", Some(source("b.png", 1.0))),
        ("a.png 300w 150h", Some(source("a.png", 0.5))),
        (
            "a.png +2x, b.png 1.x, c.png .5x",
            Some(source("c.png", 0.5)),
        ),
        ("a.png (future) 2x", None),
        ("a.png,b.png 2x", Some(source("a.png,b.png", 2.0))),
        ("a.png 2x,,,b.png 3x", Some(source("b.png", 3.0))),
    ] {
        let html = format!("<img srcset='{srcset}' sizes='600px'>");
        assert_eq!(selected(&html).0, expected, "srcset {srcset:?}");
    }
}

#[test]
fn picture_sources_before_the_img_select_by_media_type_and_srcset() {
    let picture =
        |sources: &str| format!("<picture>{sources}<img id=img src=fallback.png></picture>");
    for (sources, expected, dimensions) in [
        (
            "<source media='(min-width: 10000px)' srcset=wide.png><source srcset=print.png media=print>",
            Some(source("print.png", 1.0)),
            "img",
        ),
        (
            "<source type='image/x-unknown' srcset=x.png><source type='image/webp; codecs=x' srcset=w.png id=webp width=10>",
            Some(source("w.png", 1.0)),
            "webp",
        ),
        (
            "<source src=ignored.png><source srcset=''>",
            Some(source("fallback.png", 1.0)),
            "img",
        ),
        (
            "<source srcset='s.png 400w' sizes=200px height=5 id=sized>",
            Some(source("s.png", 2.0)),
            "sized",
        ),
    ] {
        assert_eq!(
            selected(&picture(sources)),
            (expected, Some(dimensions.to_owned())),
            "{sources}"
        );
    }
    assert_eq!(
        selected("<picture><img id=img src=first.png><source srcset=after.png></picture>").0,
        Some(source("first.png", 1.0)),
        "sources after the img are not considered"
    );
}
