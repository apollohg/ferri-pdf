use super::common;

use crate::document::OBJECT_REPLACEMENT_CHARACTER;
use crate::graphics::ImagePaint;
use crate::images::Image;
use crate::layout::ViewportPaint;
use crate::{DisplayItem, LayoutTrace, RenderDiagnostic, RenderOptions, render_html};
use common::{assert_labelled_pt, local_options};

/// A 288pt-wide page area with a 12pt/18pt body font.
const PAGE: &str = "@page{size:360pt 480pt;margin:36pt}";
const PIXEL: f32 = 0.75;
const IFRAME_BORDER: f32 = 2.0 * PIXEL;
const IMAGE: &str = "../assets/image-pattern.png";
const IMAGE_PIXELS: [f32; 2] = [80.0, 40.0];

fn render(body: &str) -> crate::RenderOutput {
    render_html(
        &format!(
            "<!doctype html><style>{}{PAGE}</style>{body}",
            common::PROFILE
        ),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("rendering `{body}`: {error}"))
}

fn trace(body: &str) -> LayoutTrace {
    render(body).layout.unwrap()
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

fn viewports(trace: &LayoutTrace) -> Vec<(&ImagePaint, &ViewportPaint)> {
    images(trace)
        .into_iter()
        .filter_map(|paint| match paint.image.as_ref() {
            Image::Viewport(view) => Some((paint, view.as_ref())),
            _ => None,
        })
        .collect()
}

fn texts(lines: &[crate::TextLine]) -> Vec<&str> {
    lines.iter().map(|line| line.text.trim()).collect()
}

fn outer_texts(trace: &LayoutTrace) -> Vec<&str> {
    trace.pages.iter().flat_map(|page| texts(page)).collect()
}

fn boxes(view: &ViewportPaint, rgb: [u8; 3]) -> Vec<crate::Rect> {
    view.items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == rgb => Some(paint.rect),
            _ => None,
        })
        .collect()
}

#[test]
fn iframes_show_their_document_in_a_default_size_inset_bordered_viewport() {
    let trace = trace(
        "<p>before</p><iframe srcdoc=\"<style>body{margin:0;font:10pt/20pt serif}</style><p>Nested</p>\"></iframe><p>after</p>",
    );
    let [(paint, view)] = viewports(&trace)[..] else {
        panic!("one iframe viewport, got {:?}", images(&trace));
    };
    let clip = paint
        .clip
        .as_ref()
        .expect("replaced content clips to its content box")
        .rect;
    assert_labelled_pt("viewport width", paint.rect.width, 300.0 * PIXEL);
    assert_labelled_pt("viewport height", paint.rect.height, 150.0 * PIXEL);
    assert_labelled_pt("content clip width", clip.width, 300.0 * PIXEL);
    assert_labelled_pt(
        "viewport left after the 2px inset border",
        paint.rect.x,
        36.0 + IFRAME_BORDER,
    );
    assert_eq!(
        [view.area.width, view.area.height],
        [paint.rect.width, paint.rect.height],
        "the nested page area is the whole viewport"
    );
    assert_eq!(
        texts(&view.lines),
        ["Nested"],
        "the nested document lays out in the viewport"
    );
    assert_labelled_pt(
        "nested text at the viewport's left edge",
        view.lines[0].x,
        0.0,
    );
    assert_eq!(
        outer_texts(&trace).join("|"),
        format!("before|{OBJECT_REPLACEMENT_CHARACTER}|after"),
        "nested text belongs to the viewport, not the page"
    );
}

#[test]
fn nested_documents_evaluate_media_queries_and_viewport_units_against_their_viewport() {
    let nested = "<style>@page{margin:50pt}body{margin:0}div{background:rgb(0,0,255)}\
        @media (width:144pt) and (height:90pt) and (overflow-block:none){div{background:rgb(0,128,0)}}\
        div{width:50vw;height:20vh}</style><div></div>";
    let trace = trace(&format!(
        "<iframe style='width:50%;height:90pt;border:0' srcdoc=\"{nested}\"></iframe>"
    ));
    let [(paint, view)] = viewports(&trace)[..] else {
        panic!("one viewport");
    };
    assert_labelled_pt("percentage width of the page area", paint.rect.width, 144.0);
    let [green] = boxes(view, [0, 128, 0])[..] else {
        panic!(
            "the 144pt by 90pt continuous viewport must match the media query: {:?}",
            view.items
        );
    };
    assert_eq!(
        [green.x, green.y, green.width, green.height],
        [0.0, 0.0, 72.0, 18.0],
        "viewport units resolve against the viewport and @page margins do not apply"
    );
}

#[test]
fn zoomed_frames_resolve_nested_viewport_units_in_unzoomed_coordinates() {
    let nested = "<style>body{margin:0}div{width:50vw;height:50vh;background:blue}@media(width:100pt) and (height:40pt){div{background:rgb(0,128,0)}}p{margin:0;font:10pt/12pt serif}</style><div></div><p>Nested</p>";
    let trace = trace(&format!(
        "<iframe style='display:block;border:0;zoom:2;width:100pt;height:40pt' srcdoc=\"{nested}\"></iframe>"
    ));
    let [(paint, view)] = viewports(&trace)[..] else {
        panic!("one zoomed viewport");
    };
    assert_eq!([paint.rect.width, paint.rect.height], [200.0, 80.0]);
    assert_eq!(
        [view.area.width, view.area.height],
        [100.0, 40.0],
        "nested CSS viewport excludes embedding zoom"
    );
    let [green] = boxes(view, [0, 128, 0])[..] else {
        panic!(
            "nested media query must match the unzoomed viewport: {:?}",
            view.items
        );
    };
    assert_eq!([green.width, green.height], [50.0, 20.0]);
    assert_eq!(texts(&view.lines), ["Nested"]);
    assert_eq!(
        view.lines[0].runs[0].font_size, 10.0,
        "nested font metrics are zoomed when composited"
    );
}

#[test]
fn nested_documents_are_continuous_media_clipped_to_the_viewport() {
    let nested = "<style>body{margin:0}p{margin:0;height:40pt}h1{break-before:page;margin:0;font-size:10pt}\
        </style><p>first</p><h1>second</h1><p style='height:400pt'>tall</p><p>below</p>";
    let output = render(&format!(
        "<iframe style='width:200pt;height:60pt' srcdoc=\"{nested}\"></iframe>"
    ));
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.len(),
        1,
        "an iframe is monolithic and never fragments the page"
    );
    let [(paint, view)] = viewports(&trace)[..] else {
        panic!("one viewport");
    };
    let second = view
        .lines
        .iter()
        .find(|line| line.text.trim() == "second")
        .expect("a forced break does not move content out of continuous media");
    assert!(
        second.baseline > 40.0 && second.baseline < 60.0,
        "the heading follows the first paragraph in the same flow, at {}",
        second.baseline
    );
    let clip = paint.clip.as_ref().unwrap().rect;
    assert_eq!(
        [clip.width, clip.height],
        [200.0, 60.0],
        "overflow beyond the viewport is clipped to the iframe's content box"
    );
}

#[test]
fn navigables_never_load_an_ancestor_document() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/fixtures");
    let url = crate::Url::from_file_path(directory.join("self.html")).unwrap();
    let options = RenderOptions {
        base: url.clone().into(),
        include_layout: true,
        ..Default::default()
    };
    let trace = render_html(
        &format!("<p>outer</p><iframe src='{url}#fragment'></iframe>"),
        &options,
    )
    .expect("a self-referencing iframe keeps its initial about:blank document")
    .layout
    .unwrap();
    let [(_, view)] = viewports(&trace)[..] else {
        panic!("one viewport");
    };
    assert!(
        view.lines.is_empty(),
        "the ancestor URL must not load again, fragment aside"
    );
}

#[test]
fn srcdoc_nesting_is_bounded() {
    let nest = |depth: usize| {
        (0..depth).fold(String::from("<p>leaf</p>"), |inner, _| {
            format!(
                "<iframe srcdoc=\"{}\"></iframe>",
                inner.replace('&', "&amp;").replace('"', "&quot;")
            )
        })
    };
    let deepest = trace_depth(&render(&nest(crate::document::frames::MAX_NAVIGABLE_DEPTH)));
    assert_eq!(
        deepest,
        crate::document::frames::MAX_NAVIGABLE_DEPTH,
        "every permitted level lays out"
    );
    let error = render_html(
        &nest(crate::document::frames::MAX_NAVIGABLE_DEPTH + 1),
        &local_options(),
    )
    .err()
    .expect("nesting beyond the limit must not render silently");
    assert!(
        error.to_string().contains("nested navigables exceed"),
        "unexpected error: {error}"
    );
}

/// How many viewports nest inside the first one.
fn trace_depth(output: &crate::RenderOutput) -> usize {
    let trace = output.layout.as_ref().unwrap();
    let mut depth = 0;
    let mut items: &[DisplayItem] = &trace.display_lists[0];
    while let Some(view) = items.iter().find_map(|item| match item {
        DisplayItem::Image(paint) => match paint.image.as_ref() {
            Image::Viewport(view) => Some(view),
            _ => None,
        },
        _ => None,
    }) {
        depth += 1;
        items = &view.items;
    }
    depth
}

#[test]
fn objects_fall_back_to_their_children_until_a_resource_renders() {
    let trace = trace(&format!(
        "<object data=missing.png>one <object data='data:application/octet-stream,%00%01'>two \
         <object data='{IMAGE}' type=application/x-unsupported>three \
         <object data='{IMAGE}'>hidden</object></object></object></object> <object>no data</object>"
    ));
    assert_eq!(
        outer_texts(&trace),
        [format!(
            "one two three {OBJECT_REPLACEMENT_CHARACTER} no data"
        )],
        "a missing resource, a binary type and an unsupported type attribute all show fallback"
    );
    let decoded: Vec<_> = images(&trace)
        .into_iter()
        .filter(|paint| matches!(paint.image.as_ref(), Image::Raster(_)))
        .collect();
    let [image] = decoded[..] else {
        panic!("the innermost object represents its image");
    };
    assert_eq!(
        [image.rect.width, image.rect.height],
        IMAGE_PIXELS.map(|pixels| pixels * PIXEL),
        "an object image has the image's natural size"
    );
    let output = render("<iframe src=missing.html></iframe><object data=missing.png></object>");
    let unavailable: Vec<_> = output
        .diagnostics
        .iter()
        .filter_map(|diagnostic| match diagnostic {
            RenderDiagnostic::UnavailableEmbedded { reference, .. } => Some(reference.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        unavailable.len(),
        2,
        "fetch failures are diagnostics: {:?}",
        output.diagnostics
    );
}

#[test]
fn objects_and_iframes_render_html_svg_text_and_image_documents() {
    let svg = "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 2 1'><rect width='2' height='1'/></svg>";
    let trace = trace(&format!(
        "<object style='width:100pt;height:80pt' data=\"{svg}\"></object>\
         <iframe src='data:text/plain,a%20%3Cb%3E'></iframe>\
         <iframe src='{IMAGE}'></iframe>\
         <iframe src='data:application/pdf,%25PDF-1.7'></iframe>"
    ));
    let views = viewports(&trace);
    let [svg_view, text_view, image_view, pdf_view] = [0, 1, 2, 3].map(|index| views[index].1);
    let [DisplayItem::Image(root)] = &svg_view.items[..] else {
        panic!(
            "an SVG document paints its root element: {:?}",
            svg_view.items
        );
    };
    assert_eq!(
        [root.rect.width, root.rect.height],
        [100.0, 80.0],
        "an SVG root without width or height fills the viewport"
    );
    assert_eq!(
        texts(&text_view.lines),
        ["a <b>"],
        "text documents render their text verbatim"
    );
    assert!(
        image_view
            .items
            .iter()
            .any(|item| matches!(item, DisplayItem::Image(paint)
            if matches!(paint.image.as_ref(), Image::Raster(_)))),
        "image documents show the image"
    );
    assert!(
        pdf_view.items.is_empty() && pdf_view.lines.is_empty(),
        "types with no viewer are handed off, leaving about:blank"
    );
    let error = render_html(
        "<iframe src='data:text/xml,<root/>'></iframe>",
        &local_options(),
    )
    .err()
    .expect("XML documents without dedicated processing are unsupported");
    assert!(
        error
            .to_string()
            .contains("unsupported nested document type: text/xml"),
        "{error}"
    );
}

#[test]
fn embeds_without_a_plugin_display_an_indication_and_represent_nothing() {
    let trace = trace(&format!(
        "<embed src='{IMAGE}'><embed type=application/x-plugin style='width:40pt;height:20pt'><embed><embed src=missing.swf>"
    ));
    let embeds = images(&trace);
    let kinds: Vec<_> = embeds
        .iter()
        .map(|paint| match paint.image.as_ref() {
            Image::Svg(_) => "indication",
            Image::Transparent => "nothing",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["indication", "indication", "nothing", "nothing"]);
    assert_eq!(
        [embeds[0].rect.width, embeds[0].rect.height],
        [300.0 * PIXEL, 150.0 * PIXEL],
        "embedded content has the default object size"
    );
    assert_eq!([embeds[1].rect.width, embeds[1].rect.height], [40.0, 20.0]);
}

#[test]
fn videos_show_their_poster_and_audio_exposes_controls_one_line_high() {
    let trace = trace(&format!(
        "<video poster='{IMAGE}'></video><video poster='{IMAGE}' style='width:120pt;height:90pt'></video><video></video><audio></audio>"
    ));
    let paints = images(&trace);
    let [natural, contained, empty, controls] = paints[..] else {
        panic!("four media elements: {paints:?}");
    };
    assert_eq!(
        [natural.rect.width, natural.rect.height],
        IMAGE_PIXELS.map(|pixels| pixels * PIXEL),
        "the playback area takes the poster's natural size"
    );
    let clip = contained.clip.as_ref().unwrap().rect;
    assert_eq!(
        [contained.rect.width, contained.rect.height],
        [120.0, 60.0],
        "object-fit: contain"
    );
    assert_labelled_pt("poster centred vertically", contained.rect.y - clip.y, 15.0);
    assert!(matches!(empty.image.as_ref(), Image::Transparent));
    assert_eq!(
        [empty.rect.width, empty.rect.height],
        [300.0 * PIXEL, 150.0 * PIXEL],
        "a video without a poster has the default object size"
    );
    assert!(matches!(controls.image.as_ref(), Image::Svg(_)));
    assert_eq!(
        [controls.rect.width, controls.rect.height],
        [8.0 * 18.0, 18.0],
        "audio controls are one 18pt line high, eight control heights wide"
    );
}

#[test]
fn canvases_render_their_fallback_content_with_scripting_disabled() {
    let trace = trace("<p>a <canvas width=10 height=10>fallback <b>text</b></canvas> z</p>");
    assert_eq!(outer_texts(&trace), ["a fallback text z"]);
    assert!(
        images(&trace).is_empty(),
        "a fallback canvas is not replaced"
    );
}

#[test]
fn embedded_canvases_are_transparent_unless_the_colour_schemes_differ() {
    let surface = |iframe_scheme: &str, document_scheme: &str| {
        let trace = trace(&format!(
            "<iframe style='color-scheme:{iframe_scheme}' srcdoc=\"<style>:root{{color-scheme:{document_scheme}}}</style>\"></iframe>"
        ));
        let [(_, view)] = viewports(&trace)[..] else {
            panic!("one viewport");
        };
        view.items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Box(paint)
                    if paint.rect == view.area && paint.decoration.background.alpha == 1.0 =>
                {
                    Some(paint.decoration.background.rgb)
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        surface("light", "light"),
        Vec::<[u8; 3]>::new(),
        "matching schemes stay transparent"
    );
    assert_eq!(
        surface("dark", "dark"),
        Vec::<[u8; 3]>::new(),
        "matching schemes stay transparent"
    );
    assert_eq!(
        surface("dark", "light"),
        [[255, 255, 255]],
        "a light document in a dark iframe gets an opaque light Canvas"
    );
    let [dark] = surface("light", "dark")[..] else {
        panic!("a dark document in a light iframe gets an opaque Canvas");
    };
    assert_ne!(dark, [255, 255, 255], "the dark Canvas colour");
}

#[test]
fn width_and_height_attributes_map_to_aspect_ratio_hints() {
    let trace = trace(&format!(
        "<video width=400 height=100 style='width:120pt;height:auto'></video>\
         <img src='{IMAGE}' width=40 height=40 style='width:60pt;height:auto'>\
         <img src='{IMAGE}' sizes=auto srcset='{IMAGE} 80w' loading=lazy>\
         <canvas width=200 height=50 style='display:block;width:100pt;background:blue'></canvas>"
    ));
    let paints = images(&trace);
    let [video, square, auto] = paints[..] else {
        panic!("a video and two images: {paints:?}");
    };
    assert_eq!(
        [video.rect.width, video.rect.height],
        [120.0, 30.0],
        "a video without a poster takes its attributes' 4:1 ratio"
    );
    assert_eq!(
        [
            square.clip.as_ref().unwrap().rect.width,
            square.clip.as_ref().unwrap().rect.height
        ],
        [60.0, 30.0],
        "an image's natural ratio overrides the `auto` hint ratio"
    );
    assert_eq!(
        [
            auto.clip.as_ref().unwrap().rect.width,
            auto.clip.as_ref().unwrap().rect.height
        ],
        [300.0 * PIXEL, 150.0 * PIXEL],
        "sizes=auto images are size-contained at 300 by 150 pixels"
    );
    let canvas = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.rect.width == 100.0 => Some(paint.rect.height),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        canvas,
        [25.0],
        "a block fallback canvas takes its attributes' 4:1 ratio"
    );
}
