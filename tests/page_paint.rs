mod common;
use common::{PROFILE, assert_pt};
use ferri_pdf::{DisplayItem, LayoutTrace, Rect};

fn render(css: &str, body: &str) -> LayoutTrace {
    common::render_trace(&format!(
        "<!doctype html><style>{PROFILE}@page{{size:400pt 600pt;margin:20pt;padding:10pt;border:2pt solid #cc2200}}body,p{{margin:0;font:8pt/12pt serif}}{css}</style>{body}"
    ))
}
fn rect(actual: Rect, expected: [f32; 4]) {
    for (actual, expected) in [actual.x, actual.y, actual.width, actual.height]
        .into_iter()
        .zip(expected)
    {
        assert_pt(actual, expected);
    }
}

#[test]
fn sliced_decoration_stays_joined_across_column_spanners() {
    let trace = render(
        ".columns{column-count:2;column-gap:20pt}.box{border:4pt solid blue;\
         border-radius:20pt;background:#123456}.part{height:40pt}.span{column-span:all;height:10pt}",
        "<div class=columns><div class=box><div class=part></div>\
         <div class=span></div><div class=part></div></div></div>",
    );
    let paints: Vec<_> = trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == [0x12, 0x34, 0x56] => {
                Some(paint)
            }
            _ => None,
        })
        .collect();
    assert!(
        paints.len() >= 3,
        "a spanner divides its decorated ancestor: {paints:?}"
    );
    let total: f32 = paints.iter().map(|paint| paint.rect.height).sum();
    let mut before = 0.0;
    for (index, paint) in paints.iter().enumerate() {
        let whole = paint
            .background_box
            .expect("all ancestor segments need their common unbroken decoration box");
        assert_pt(whole.height, total);
        assert_pt(whole.y, paint.rect.y - before);
        assert_eq!(paint.first_fragment, index == 0);
        assert_eq!(paint.last_fragment, index + 1 == paints.len());
        before += paint.rect.height;
    }
}

#[test]
fn page_background_canvas_border_and_content_follow_the_page_painting_order() {
    let trace = render(
        "@page{background:#102030}html{background:#405060}p{background:#708090;width:100pt;height:12pt}",
        "<p>Ink</p>",
    );
    let paints: Vec<_> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) => Some(paint),
            _ => None,
        })
        .collect();
    let background = paints
        .iter()
        .position(|paint| paint.decoration.background.rgb == [0x10, 0x20, 0x30])
        .expect("page background must paint");
    let canvas = paints
        .iter()
        .position(|paint| paint.decoration.background.rgb == [0x40, 0x50, 0x60])
        .expect("document canvas must paint");
    let border = paints
        .iter()
        .position(|paint| {
            paint.decoration.borders.iter().any(|border| {
                border.color.rgb == [0xcc, 0x22, 0]
                    && border.used_width() > 0.0
                    && border.color.alpha > 0.0
            })
        })
        .expect("page borders must paint");
    let content = paints
        .iter()
        .position(|paint| paint.decoration.background.rgb == [0x70, 0x80, 0x90])
        .expect("content must paint");
    assert!(
        background < canvas && canvas < border && border < content,
        "page layers must surround the document canvas correctly"
    );
    let fill = paints[background]
        .background_fill()
        .expect("page background fill");
    let ferri_pdf::Shape::Rect(area) = fill.shape else {
        panic!("unrounded page background rectangle")
    };
    rect(area, [0.0, 0.0, 400.0, 600.0]);
    rect(paints[canvas].rect, [20.0, 20.0, 360.0, 560.0]);
    rect(paints[border].rect, [20.0, 20.0, 360.0, 560.0]);
    rect(paints[content].rect, [32.0, 32.0, 100.0, 12.0]);
}

#[test]
fn singular_root_keeps_canvas_ink_and_page_paint() {
    let trace = render(
        "html{height:80pt;background:#405060;transform:scale(0)}",
        "<p>Hidden</p>",
    );
    let canvas = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == [0x40, 0x50, 0x60] => {
                Some(paint)
            }
            _ => None,
        })
        .expect("canvas retains its computed background separately from visibility");
    assert!(
        canvas.has_ink(),
        "propagated canvas ink is independent of the singular root transform"
    );
    assert!(canvas.background_fill().is_some());
    assert!(
        canvas
            .background_layers(trace.page_area)
            .unwrap()
            .is_empty()
    );
    assert!(trace.display_lists[0].iter().any(|item| matches!(item, DisplayItem::Box(paint) if paint.decoration.borders.iter().any(|border| border.used_width() > 0.0 && border.color.alpha > 0.0))), "page border stays visible independently of root visibility");
}

#[test]
fn page_image_layers_use_page_origins_and_ignore_background_clip() {
    let trace = render(
        "@page{margin:10pt 20pt 30pt 40pt;padding:1pt 2pt 3pt 4pt;border-width:2pt 3pt 4pt 5pt;background-image:linear-gradient(red,blue),linear-gradient(blue,red);background-size:20pt 30pt;background-repeat:no-repeat;background-origin:content-box,padding-box;background-clip:content-box,padding-box;background-attachment:scroll,fixed}",
        "<p>Layers</p>",
    );
    let paint = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.backgrounds.images.len() == 2 => {
                Some(paint)
            }
            _ => None,
        })
        .expect("page image layers");
    let layers = paint.background_layers(trace.page_area).unwrap();
    assert_eq!(layers.len(), 2);
    rect(layers[0].clip.rect, [0.0, 0.0, 400.0, 600.0]);
    rect(layers[0].tile, [0.0, 0.0, 20.0, 30.0]);
    rect(layers[1].clip.rect, [0.0, 0.0, 400.0, 600.0]);
    rect(layers[1].tile, [49.0, 13.0, 20.0, 30.0]);
}

#[test]
fn canvas_backgrounds_use_root_geometry_including_body_propagation() {
    for source in ["html", "body"] {
        for formatting in [
            "display:block",
            "display:flex;flex-direction:column",
            "display:table;border-spacing:0",
            "column-count:2;column-gap:0",
        ] {
            let trace = render(
                &format!(
                    "html{{{formatting};width:120pt;margin:0;padding:7pt;border:3pt solid transparent}}{source}{{background:linear-gradient(red,blue) content-box no-repeat;background-size:10pt 10pt}}"
                ),
                "<p>Root origin</p>",
            );
            let layers: Vec<_> = trace.display_lists[0]
                .iter()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint) => {
                        Some(paint.background_layers(trace.page_area).unwrap())
                    }
                    _ => None,
                })
                .flatten()
                .collect();
            assert_eq!(
                layers.len(),
                1,
                "{source}/{formatting} background must be propagated exactly once"
            );
            rect(layers[0].tile, [42.0, 42.0, 10.0, 10.0]);
            rect(layers[0].clip.rect, [32.0, 32.0, 336.0, 536.0]);
        }
    }
}

#[test]
fn fragmented_canvas_uses_root_slice_or_clone_geometry_and_page_fixed_geometry() {
    for source in ["html", "body"] {
        for decoration in ["slice", "clone"] {
            let trace = render(
                &format!(
                    "html{{padding:7pt;border:3pt solid transparent;box-decoration-break:{decoration}}}{source}{{background-image:linear-gradient(red,blue),linear-gradient(blue,red);background-origin:content-box;background-clip:content-box;background-repeat:no-repeat;background-size:10pt 100%,20pt 20pt;background-attachment:scroll,fixed;background-position:left top,right bottom}}"
                ),
                "<p>First</p><p class='new-page'>Second</p>",
            );
            assert_eq!(
                trace.pages.len(),
                2,
                "{source}/{decoration}: one forced page break"
            );
            for (page, items) in trace.display_lists.iter().enumerate() {
                let layers: Vec<_> = items
                    .iter()
                    .filter_map(|item| match item {
                        DisplayItem::Box(paint) => {
                            Some(paint.background_layers(trace.page_area).unwrap())
                        }
                        _ => None,
                    })
                    .flatten()
                    .collect();
                assert_eq!(
                    layers.len(),
                    2,
                    "{source}/{decoration}, page {page}: canvas paints exactly once"
                );
                rect(layers[0].tile, [348.0, 548.0, 20.0, 20.0]);
                rect(layers[0].clip.rect, [32.0, 32.0, 336.0, 536.0]);
                let (top, height) = match (decoration, page) {
                    ("slice", 0) => (42.0, 538.0),
                    ("slice", 1) => (-494.0, 538.0),
                    ("clone", 0) => (42.0, 516.0),
                    ("clone", 1) => (42.0, 12.0),
                    _ => unreachable!(),
                };
                rect(layers[1].tile, [42.0, top, 10.0, height]);
            }
        }
    }
}

#[test]
fn production_descriptors_share_cascade_units_and_recovery_without_repagination() {
    let cases = [
        ("", 0.0, 0),
        ("@page{bleed:8pt}", 8.0, 0),
        ("@page{bleed:-6pt}", -6.0, 0),
        ("@page{marks:crop}", 6.0, 8),
        ("@page{marks:cross}", 0.0, 8),
        ("@page{marks:cross crop}", 6.0, 16),
        ("html{--b:7pt}@page{--b:9pt;bleed:var(--b)}", 9.0, 0),
        (
            "html{font-size:10pt}@page{font-size:30pt;bleed:calc(1em + 1rem + 1vw)}",
            41.6,
            0,
        ),
        ("@page{bleed:4pt;bleed:10%}", 4.0, 0),
        ("@page{bleed:4pt;bleed:var(--missing,10%)}", 0.0, 0),
        ("@page{marks:crop;marks:var(--missing,bogus)}", 0.0, 0),
        (
            "@page{bleed:8pt;bleed:initial;marks:crop;marks:unset}",
            0.0,
            0,
        ),
        ("@page{bleed:8pt;bleed:inherit}", 0.0, 0),
        (
            "@page{bleed:4pt!important;bleed:8pt;marks:crop!important;marks:cross}",
            4.0,
            8,
        ),
        (
            "@layer base,override;@layer base{@page{bleed:4pt;marks:crop}}@layer override{@page{bleed:8pt;marks:cross}@page{bleed:revert-layer;marks:revert-layer}}",
            4.0,
            8,
        ),
        ("@media(min-width:0pt){@page{size:1pt;bleed:8pt}}", 8.0, 0),
    ];
    let prefix =
        "@page{size:200pt 300pt;margin:20pt;padding:0;border:0}p{margin:0;font:12pt/18pt serif}";
    let body = "<p>A<br>B</p><p style='break-before:page'>C</p>";
    let baseline = render(prefix, body);
    for (css, bleed, marks) in cases {
        let trace = render(&format!("{prefix}{css}"), body);
        assert_eq!(
            trace.pages.len(),
            2,
            "production must not affect pagination: {css}"
        );
        for (page, reference) in trace.pages.iter().zip(&baseline.pages) {
            assert_eq!(page.len(), reference.len(), "{css}");
            for (line, reference) in page.iter().zip(reference) {
                assert_eq!(line.text, reference.text, "{css}");
                assert_pt(line.x, reference.x);
                assert_pt(line.baseline, reference.baseline);
                assert_pt(line.width, reference.width);
            }
        }
        rect(trace.page_area, [20.0, 20.0, 160.0, 260.0]);
        rect(trace.page_output.trim, [0.0, 0.0, 200.0, 300.0]);
        rect(
            trace.page_output.bleed,
            [-bleed, -bleed, 200.0 + 2.0 * bleed, 300.0 + 2.0 * bleed],
        );
        assert_eq!(trace.page_output.marks.len(), marks, "{css}");
    }
}
