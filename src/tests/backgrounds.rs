use super::common;
use crate::graphics::{BackgroundImage, Color, ResolvedGradient, Shading, TileImage, image_size};
use crate::paint::NaturalDimensions;
use crate::paint::{BackgroundSize, GradientStop, Repeat, SizeComponent, rounded_size};
use crate::{BoxPaint, DisplayItem, Rect, render_html};
use common::{PROFILE, local_options as options};
use std::sync::{Arc, Mutex};

const TOLERANCE: f32 = 1.0e-3;
const COLOR_TOLERANCE: f32 = 1.5;
const PAGE_AREA: Rect = Rect {
    x: 36.0,
    y: 36.0,
    width: 288.0,
    height: 408.0,
};

#[test]
fn formatting_preserves_computed_background_and_border_image_urls() {
    use crate::document::{BlockContent, StyledDocument};
    use crate::resources::FileResolver;
    let settings = options();
    let reference = settings
        .base
        .resolve("../assets/image-pattern.png")
        .unwrap()
        .to_string();
    let document = StyledDocument::parse(
        &format!("<!doctype html><style>{PROFILE}html{{background-image:url(../assets/image-pattern.png);border-image-source:url(../assets/image-pattern.png)}}body,div{{background-image:inherit;border-image-source:inherit}}</style><div>Computed sources</div>"),
        &settings.base,
        &[],
        &FileResolver::default(),
    ).unwrap();
    let root = document.root();
    let expected = serde_json::json!({"Url": {"url": reference, "resolution": null, "scale": crate::paint::INITIAL_IMAGE_SCALE}});
    assert_eq!(
        serde_json::to_value(&root.style().decoration.backgrounds.images).unwrap(),
        serde_json::json!([expected]),
        "formatting must keep the root's computed URL rather than replace it with a loaded image"
    );
    assert_eq!(
        serde_json::to_value(&root.style().decoration.border_image.source).unwrap(),
        expected
    );
    let mut pending = vec![root];
    let mut found = false;
    while let Some(block) = pending.pop() {
        if block.children().iter().any(|child| matches!(child, BlockContent::Inline(paragraph) if paragraph.text().contains("Computed sources"))) {
            assert_eq!(serde_json::to_value(&block.style().decoration.backgrounds.images).unwrap(), serde_json::json!([expected]), "explicitly inherited computed values must retain their source URL");
            assert_eq!(serde_json::to_value(&block.style().decoration.border_image.source).unwrap(), expected);
            found = true;
        }
        pending.extend(block.children().iter().filter_map(|child| match child {
            BlockContent::Block(block) | BlockContent::Positioned(block) => Some(block),
            BlockContent::Inline(_) | BlockContent::Events(_) => None,
        }));
    }
    assert!(found, "the actual formatted div must be inspected");
}

fn paints(css: &str, body: &str) -> Vec<Vec<BoxPaint>> {
    let html = format!("<style>{PROFILE}{css}</style>{body}");
    let trace = render_html(&html, &options())
        .unwrap_or_else(|error| panic!("{html} must render: {error}"))
        .layout
        .unwrap();
    trace
        .display_lists
        .iter()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint) => Some(paint.clone()),
                    _ => None,
                })
                .collect()
        })
        .collect()
}
fn first(css: &str, body: &str) -> BoxPaint {
    paints(css, body).remove(0).remove(0)
}
/// The single gradient layer of a 200pt by 100pt box.
fn gradient(image: &str) -> ResolvedGradient {
    let paint = first(
        &format!("div{{width:200pt;height:100pt;background-image:{image}}}"),
        "<div>A</div>",
    );
    let layers = paint.background_layers(PAGE_AREA).unwrap();
    assert_eq!(layers.len(), 1, "{image} is one layer");
    match &layers[0].image {
        TileImage::Gradient(gradient) => gradient.clone(),
        TileImage::Image(..) | TileImage::Color(_) | TileImage::CrossFade(..) => {
            panic!("{image} must resolve to a gradient")
        }
    }
}
fn close(label: &str, actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < TOLERANCE,
        "{label}: {actual} differs from {expected}"
    );
}
fn assert_stops(label: &str, actual: &[GradientStop], expected: &[(f32, [u8; 3], f32)]) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{label}: stop count\nactual   {actual:?}\nexpected {expected:?}"
    );
    for (index, (stop, (offset, rgb, alpha))) in actual.iter().zip(expected).enumerate() {
        close(
            &format!("{label}: stop {index} offset"),
            stop.offset,
            *offset,
        );
        close(
            &format!("{label}: stop {index} alpha"),
            stop.color.alpha,
            *alpha,
        );
        for channel in 0..3 {
            assert!(
                (f32::from(stop.color.rgb[channel]) - f32::from(rgb[channel])).abs()
                    <= COLOR_TOLERANCE,
                "{label}: stop {index} colour {:?}, expected {rgb:?}\nactual {actual:?}",
                stop.color.rgb
            );
        }
    }
}

#[test]
fn corner_gradients_run_perpendicular_to_the_other_diagonal_across_the_whole_box() {
    let resolved = gradient("linear-gradient(to top right,red,blue)");
    let Shading::Axial { start, end } = resolved.shading else {
        panic!("linear gradients are axial: {:?}", resolved.shading)
    };
    let direction = [end[0] - start[0], end[1] - start[1]];
    let length = direction[0].hypot(direction[1]);
    let angle = 100.0f32.atan2(200.0);
    close(
        "gradient line length is |W sin a| + |H cos a| (CSS Images 3 §3.1.1)",
        length,
        200.0 * angle.sin() + 100.0 * angle.cos(),
    );
    close(
        "the line is perpendicular to the top-left to bottom-right diagonal",
        direction[0] * 200.0 + direction[1] * 100.0,
        0.0,
    );
    close(
        "the line centre is the box centre (x)",
        f32::midpoint(start[0], end[0]),
        100.0,
    );
    close(
        "the line centre is the box centre (y)",
        f32::midpoint(start[1], end[1]),
        50.0,
    );
    assert!(
        direction[0] > 0.0 && direction[1] < 0.0,
        "to top right must point up and right in y-down coordinates: {direction:?}"
    );
}

#[test]
fn stop_fixup_interpolates_outside_stops_and_clamps_decreasing_positions() {
    let resolved = gradient("linear-gradient(red -20%,blue 50%,lime 50%,yellow 30%)");
    const RED_TO_BLUE_AT_ZERO: [u8; 3] = [182, 0, 73];
    assert_stops(
        "the -20% stop is cut at the line start and yellow 30% moves to 50%",
        &resolved.stops,
        &[
            (0.0, RED_TO_BLUE_AT_ZERO, 1.0),
            (0.5, [0, 0, 255], 1.0),
            (0.5, [0, 255, 0], 1.0),
            (0.5, [255, 255, 0], 1.0),
            (1.0, [255, 255, 0], 1.0),
        ],
    );
}

#[test]
fn single_stop_gradients_fill_the_tile_independently_of_stop_position_and_repetition() {
    for function in [
        "linear-gradient",
        "radial-gradient",
        "conic-gradient",
        "repeating-linear-gradient",
        "repeating-radial-gradient",
        "repeating-conic-gradient",
    ] {
        let positions: &[&str] = if function.contains("conic") {
            &["", " -90deg", " 150%", " 30deg 60deg"]
        } else {
            &["", " -30pt", " 150%", " 30pt 60pt"]
        };
        for position in positions {
            let source = format!("{function}(rgb(20 80 160 / 40%){position})");
            let resolved = gradient(&source);
            assert!(!resolved.stops.is_empty(), "{source} must paint its color");
            for stop in &resolved.stops {
                assert_eq!(stop.color.rgb, [20, 80, 160], "{source}: {stop:?}");
                close(&source, stop.color.alpha, 0.4);
            }
            for point in [[0.0, 0.0], [100.0, 50.0], [200.0, 100.0]] {
                close(
                    &format!("{source} opacity at {point:?}"),
                    resolved.alpha_at(point),
                    0.4,
                );
            }
        }
    }
}

#[test]
fn interpolation_is_premultiplied_so_transparent_ends_keep_their_neighbour_colour() {
    let resolved = gradient("linear-gradient(to right,red,transparent)");
    assert_stops(
        "red to transparent fades alpha without darkening",
        &resolved.stops,
        &[(0.0, [255, 0, 0], 1.0), (1.0, [255, 0, 0], 0.0)],
    );
    let half = gradient("linear-gradient(to right,rgba(255,0,0,0.5),blue)");
    let middle = half
        .stops
        .iter()
        .find(|stop| (stop.offset - 0.5).abs() < TOLERANCE)
        .expect("unequal alphas are sampled across the segment");
    // Premultiplied midpoint: (0.25 red + 0.5 blue) / 0.75 alpha.
    assert_eq!(
        middle.color.rgb,
        [85, 0, 170],
        "premultiplied sRGB weights each colour by its alpha"
    );
    close("alpha interpolates linearly", middle.color.alpha, 0.75);
}

#[test]
fn transition_hints_move_the_midpoint_colour_to_the_hint() {
    let resolved = gradient("linear-gradient(to right,red,30%,blue)");
    let at_hint = resolved
        .stops
        .iter()
        .min_by(|a, b| {
            (a.offset - 0.3)
                .abs()
                .partial_cmp(&(b.offset - 0.3).abs())
                .unwrap()
        })
        .unwrap();
    close("a sample lies on the hint", at_hint.offset, 0.3125);
    let weight = 0.3125f32.powf(0.5f32.ln() / 0.3f32.ln());
    assert!(
        (f32::from(at_hint.color.rgb[2]) - 255.0 * weight).abs() <= COLOR_TOLERANCE,
        "hinted weight t^(ln .5 / ln h) gives blue {} near the hint: {at_hint:?}",
        255.0 * weight
    );
}

#[test]
fn repeating_gradients_tile_their_period_and_zero_periods_render_the_average() {
    let resolved = gradient("repeating-linear-gradient(to right,red 5pt,blue 15pt)");
    let period = 10.0 / 200.0;
    let hard_stops: Vec<f32> = resolved
        .stops
        .windows(2)
        .filter(|pair| pair[0].offset == pair[1].offset)
        .map(|pair| pair[0].offset)
        .collect();
    assert_eq!(
        hard_stops.len(),
        20,
        "a 10pt period restarts 20 times across 200pt: {hard_stops:?}"
    );
    for (index, offset) in hard_stops.iter().enumerate() {
        close(
            &format!("restart {index} lies at 5pt plus whole periods"),
            *offset,
            period / 2.0 + period * index as f32,
        );
    }
    let average = gradient("repeating-linear-gradient(red 10pt,blue 10pt)");
    assert_stops(
        "coincident first and last stops paint their average colour",
        &average.stops,
        &[(0.0, [128, 0, 128], 1.0)],
    );
}

#[test]
fn radial_extents_and_overflowing_stops_size_the_shading() {
    let circle = gradient("radial-gradient(circle,red,blue)");
    let Shading::Radial {
        center,
        radii,
        scale_y,
    } = circle.shading
    else {
        panic!("radial gradients are radial shadings")
    };
    close("centre x", center[0], 100.0);
    close("centre y", center[1], 50.0);
    close(
        "farthest-corner circle reaches the corner",
        radii[1],
        100.0f32.hypot(50.0),
    );
    close("circles are not scaled", scale_y, 1.0);
    let ellipse =
        gradient("radial-gradient(ellipse closest-side at 30% 50%,red,blue 50%,lime 120%)");
    let Shading::Radial { radii, scale_y, .. } = ellipse.shading else {
        panic!("radial gradients are radial shadings")
    };
    close(
        "the 120% stop extends the shading past the 60pt horizontal radius",
        radii[1],
        72.0,
    );
    close("closest-side ellipse is 60pt by 50pt", scale_y, 50.0 / 60.0);
    assert_stops(
        "stop offsets are renormalised to the extended radius",
        &ellipse.stops,
        &[
            (0.0, [255, 0, 0], 1.0),
            (0.5 / 1.2, [0, 0, 255], 1.0),
            (1.0, [0, 255, 0], 1.0),
        ],
    );
}

#[test]
fn background_size_keywords_and_auto_follow_the_natural_ratio() {
    let raster = NaturalDimensions {
        width: Some(60.0),
        height: Some(30.0),
        ratio: Some(2.0),
    };
    let area = [110.0, 60.0];
    let auto = BackgroundSize::Explicit([const { SizeComponent::Auto }; 2]);
    let cases = [
        (
            "auto keeps the natural size",
            auto.clone(),
            raster,
            [60.0, 30.0],
        ),
        (
            "cover fills the taller axis",
            BackgroundSize::Cover,
            raster,
            [120.0, 60.0],
        ),
        (
            "contain fits the wider axis",
            BackgroundSize::Contain,
            raster,
            [110.0, 55.0],
        ),
        (
            "one length derives the other from the ratio",
            BackgroundSize::Explicit([
                SizeComponent::Value(crate::LengthPercentage::Length(40.0)),
                SizeComponent::Auto,
            ]),
            raster,
            [40.0, 20.0],
        ),
        (
            "a ratio-only image is contained",
            auto.clone(),
            NaturalDimensions {
                width: None,
                height: None,
                ratio: Some(0.5),
            },
            [30.0, 60.0],
        ),
        (
            "an image without natural size or ratio fills the area",
            BackgroundSize::Cover,
            NaturalDimensions::default(),
            area,
        ),
    ];
    for (label, size, natural, expected) in cases {
        let actual = image_size(&size, natural, area);
        close(&format!("{label} (width)"), actual[0], expected[0]);
        close(&format!("{label} (height)"), actual[1], expected[1]);
    }
    let both = rounded_size([60.0, 30.0], &auto, [Repeat::Round; 2], area);
    close("round fits round(110/60) = 2 tiles", both[0], 55.0);
    close("round fits 60/30 = 2 tiles", both[1], 30.0);
    let one = rounded_size([60.0, 30.0], &auto, [Repeat::Round, Repeat::NoRepeat], area);
    close(
        "rounding one axis restores the ratio on an auto other axis",
        one[1],
        27.5,
    );
}

#[test]
fn layers_paint_bottom_first_with_their_own_clip_and_the_colour_follows_the_bottom_clip() {
    let paint = first(
        "div{width:100pt;height:40pt;border:4pt solid navy;padding:6pt;border-radius:20pt}",
        "<div style=\"background:url(../assets/image-pattern.png) no-repeat 0 0 content-box,linear-gradient(red,blue) padding-box yellow\">A</div>",
    );
    let layers = paint.background_layers(PAGE_AREA).unwrap();
    assert_eq!(layers.len(), 2);
    assert!(
        matches!(layers[0].image, TileImage::Gradient(_))
            && matches!(layers[1].image, TileImage::Image(..)),
        "the last listed layer is painted first"
    );
    let padding_box = &layers[0].clip;
    assert_eq!(
        padding_box.rect,
        Rect {
            x: 40.0,
            y: 40.0,
            width: 112.0,
            height: 52.0
        },
        "the gradient is clipped to the padding box"
    );
    assert_eq!(
        padding_box.radii[0],
        [16.0, 16.0],
        "padding-box curves shrink by the border width"
    );
    let content_box = &layers[1].clip;
    assert_eq!(
        content_box.radii[0],
        [10.0, 10.0],
        "content-box curves shrink by border and padding"
    );
    assert_eq!(
        layers[1].tile,
        Rect {
            x: 46.0,
            y: 46.0,
            width: 60.0,
            height: 30.0
        },
        "content-box origin places the 80x40px image at the content edge"
    );
    let fill = paint
        .background_fill()
        .expect("yellow is the bottom layer colour");
    let crate::Shape::Path { contours, .. } = &fill.shape else {
        panic!("a rounded colour area is a curved path: {:?}", fill.shape)
    };
    assert_eq!(
        contours[0].start,
        [40.0, 56.0],
        "the colour uses the bottom layer's padding-box clip, not the border box"
    );
}

#[test]
fn shorthand_layers_reset_omitted_longhands_and_split_position_size_and_boxes() {
    let paint = first(
        "div{width:100pt;height:40pt;background-size:50%;background-repeat:space}",
        "<div style=\"background:url(../assets/image-pattern.png) right 4pt bottom / 20pt auto repeat-y padding-box content-box,none\">A</div>",
    );
    let backgrounds = &paint.decoration.backgrounds;
    assert!(matches!(
        backgrounds.images[..],
        [BackgroundImage::Image { .. }, BackgroundImage::None]
    ));
    assert_eq!(
        backgrounds.sizes,
        vec![
            BackgroundSize::Explicit([
                SizeComponent::Value(crate::LengthPercentage::Length(20.0)),
                SizeComponent::Auto
            ]),
            BackgroundSize::default()
        ],
        "the shorthand resets the earlier background-size"
    );
    assert_eq!(
        backgrounds.repeats,
        vec![[Repeat::NoRepeat, Repeat::Repeat], [Repeat::Repeat; 2]],
        "repeat-y is no-repeat horizontally"
    );
    assert_eq!(backgrounds.origins[0], crate::graphics::BoxArea::PaddingBox);
    assert_eq!(
        backgrounds.clips[0],
        crate::graphics::BackgroundClip::Box(crate::graphics::BoxArea::ContentBox)
    );
    let layer = &paint.background_layers(PAGE_AREA).unwrap()[0];
    close(
        "right 4pt places the tile's right edge 4pt inside the padding box",
        layer.tile.x + layer.tile.width,
        36.0 + 100.0 - 4.0,
    );
    assert_eq!(layer.repeat, [false, true]);
}

#[test]
fn sliced_fragments_position_backgrounds_in_the_unbroken_box() {
    let pages = paints(
        "@page{size:360pt 180pt;margin:36pt}div{border:3pt solid navy;padding:4pt;background:linear-gradient(red,blue)}",
        &format!(
            "<div>{}</div>",
            (0..13)
                .map(|i| format!("A{i}"))
                .collect::<Vec<_>>()
                .join("<br>")
        ),
    );
    let fragments: Vec<&BoxPaint> = pages.iter().map(|page| &page[0]).collect();
    assert!(fragments.len() >= 2, "the fixture must fragment");
    let total: f32 = fragments.iter().map(|paint| paint.rect.height).sum();
    let mut before = 0.0;
    for (page, fragment) in fragments.iter().enumerate() {
        let unbroken = fragment
            .background_box
            .unwrap_or_else(|| panic!("fragment {page} keeps the unbroken box"));
        close(
            &format!("fragment {page} unbroken height"),
            unbroken.height,
            total,
        );
        close(
            &format!("fragment {page} is offset by the fragments before it"),
            fragment.rect.y - unbroken.y,
            before,
        );
        before += fragment.rect.height;
        let layer = &fragment.background_layers(PAGE_AREA).unwrap()[0];
        close(
            &format!("fragment {page} gradient tile spans the unbroken padding box"),
            layer.tile.height,
            total - 6.0,
        );
    }
}

struct RecordingResolver {
    requests: Mutex<Vec<String>>,
}
impl crate::ResourceResolver for RecordingResolver {
    fn resolve(
        &self,
        reference: &str,
        base: &crate::ResourceBase,
    ) -> Result<crate::Resource, crate::ResourceError> {
        let url = base.resolve(reference)?;
        self.requests.lock().unwrap().push(url.to_string());
        let bytes = if std::path::Path::new(url.path())
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("css"))
        {
            b"div{width:80pt;height:40pt;background:url(img/pattern.png) no-repeat}".to_vec()
        } else {
            std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/oracle/assets/image-pattern.png"
            ))
            .unwrap()
        };
        Ok(crate::Resource {
            bytes,
            base: crate::ResourceBase::Url(url),
        })
    }
}

#[test]
fn stylesheet_urls_resolve_against_the_stylesheet_and_inline_urls_against_the_document() {
    let resolver = Arc::new(RecordingResolver {
        requests: Mutex::new(Vec::new()),
    });
    let options = crate::RenderOptions {
        base: crate::ResourceBase::Url("https://example.test/doc/page.html".parse().unwrap()),
        ..Default::default()
    };
    crate::render_with_resolver(
        "<link rel=stylesheet href=\"../styles/sheet.css\"><div>A</div><p style=\"background:url(local.png)\">B</p>",
        &options,
        resolver.as_ref(),
    )
    .expect("remote stylesheet and images render");
    let requests = resolver.requests.lock().unwrap().clone();
    assert!(
        requests.contains(&"https://example.test/styles/img/pattern.png".to_owned()),
        "a stylesheet url() is relative to the stylesheet: {requests:?}"
    );
    assert!(
        requests.contains(&"https://example.test/doc/local.png".to_owned()),
        "an inline style url() is relative to the document: {requests:?}"
    );
}

#[test]
fn page_document_and_margin_backgrounds_share_images_without_mutating_computed_urls() {
    use crate::document::StyledDocument;
    const IMAGE: &str = "https://example.test/shared.png";
    let resolver = RecordingResolver {
        requests: Mutex::default(),
    };
    let options = crate::RenderOptions {
        base: crate::ResourceBase::Url("https://example.test/doc/page.html".parse().unwrap()),
        ..Default::default()
    };
    let document = StyledDocument::parse(
        &format!("<!doctype html><style>@page{{background-image:url({IMAGE});@top-left{{content:'';background-image:url({IMAGE})}}}}html{{background-image:url({IMAGE})}}</style>"),
        &options.base,
        &[],
        &resolver,
    ).expect("page, document, and margin images load through the same resource cache");
    assert_eq!(*resolver.requests.lock().unwrap(), [IMAGE]);
    let page = document.page_box();
    assert_eq!(
        page.margins.len(),
        1,
        "empty generated content still creates its margin box"
    );
    let decorations = [
        (&page.style.decoration, &page.decoration),
        (
            &document.root().style().decoration,
            document.root().decoration(),
        ),
        (
            &page.margins[0].margin.style.decoration,
            &page.margins[0].decoration,
        ),
    ];
    let mut images = Vec::new();
    for (computed, materialized) in decorations {
        assert_eq!(
            serde_json::to_value(&computed.backgrounds.images).unwrap(),
            serde_json::json!([{"Url": {"url": IMAGE, "resolution": null, "scale": crate::paint::INITIAL_IMAGE_SCALE}}]),
            "computed image must retain the absolute source URL"
        );
        let BackgroundImage::Image { image, .. } = &materialized.backgrounds.images[0] else {
            panic!("drawable decoration must contain the decoded image")
        };
        images.push(image);
    }
    assert!(
        images.windows(2).all(|pair| Arc::ptr_eq(pair[0], pair[1])),
        "all three consumers must share the decoded image"
    );
}

#[test]
fn hidden_canvas_sources_do_not_load_images_or_suppress_page_paint() {
    for source in ["html", "body"] {
        let resolver = RecordingResolver {
            requests: Mutex::default(),
        };
        let options = crate::RenderOptions {
            base: crate::ResourceBase::Url("https://example.test/doc/page.html".parse().unwrap()),
            include_layout: true,
            ..Default::default()
        };
        let output = crate::render_with_resolver(
            &format!("<!doctype html><style>@page{{background:#102030;border:2pt solid red}}{source}{{display:none;background:url(hidden.png)}}</style><body>Hidden</body>"),
            &options,
            &resolver,
        ).expect("hidden canvas image must not be fetched");
        assert!(
            resolver.requests.lock().unwrap().is_empty(),
            "{source} background must not load"
        );
        let trace = output.layout.unwrap();
        assert_eq!(
            trace.pages.len(),
            1,
            "page painting must not create more content pages"
        );
        assert!(trace.pages[0].is_empty(), "hidden content must not paint");
        let boxes: Vec<_> = trace.display_lists[0]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Box(paint) => Some(paint),
                _ => None,
            })
            .collect();
        assert_eq!(
            boxes.len(),
            2,
            "the independent page background and border must paint for hidden {source}"
        );
        assert_eq!(boxes[0].decoration.background.rgb, [0x10, 0x20, 0x30]);
        assert!(
            boxes[1]
                .decoration
                .borders
                .iter()
                .all(|border| border.used_width() == 2.0)
        );
    }
}

#[test]
fn active_page_and_margin_images_preserve_resource_policy_errors() {
    const RESOURCE_LIMIT_BYTES: usize = 1;
    let resolver = crate::FileResolver {
        max_resource_bytes: RESOURCE_LIMIT_BYTES,
        ..Default::default()
    };
    for css in [
        "@page{background-image:url(../assets/image-pattern.png)}",
        "@page{@top-left{content:'';background-image:url(../assets/image-pattern.png)}}",
    ] {
        let output = crate::render_with_resolver(
            &format!("<!doctype html><style>{css}</style>"),
            &options(),
            &resolver,
        );
        assert!(
            matches!(
                output,
                Err(crate::Error::Image(crate::ImageError::Resource(
                    crate::ResourceError::TooLarge {
                        limit: RESOURCE_LIMIT_BYTES,
                        ..
                    }
                )))
            ),
            "{css}: page images must propagate the original resource size policy error"
        );
    }
}

#[test]
fn root_backgrounds_use_root_positioning_while_fixed_layers_use_the_page_area() {
    let pages = paints(
        "body{background:url(../assets/image-pattern.png) right bottom no-repeat}div{height:20pt;background:url(../assets/image-pattern.png) fixed no-repeat 50% 50%}",
        "<p>A</p><div>B</div>",
    );
    let canvas = &pages[0][0];
    assert_eq!(canvas.rect, PAGE_AREA, "the canvas covers the page area");
    let layer = &canvas.background_layers(PAGE_AREA).unwrap()[0];
    const ROOT_HEIGHT_PT: f32 = 18.0 + 12.0 + 20.0;
    assert_eq!(
        [
            layer.tile.x + layer.tile.width,
            layer.tile.y + layer.tile.height
        ],
        [PAGE_AREA.x + PAGE_AREA.width, PAGE_AREA.y + ROOT_HEIGHT_PT],
        "the propagated body image uses the root box (paragraph line + margin + div)"
    );
    assert_eq!(
        pages[0]
            .iter()
            .skip(1)
            .filter(|paint| paint.decoration.backgrounds.has_images())
            .count(),
        1,
        "only the fixed div paints images; the body no longer paints its propagated background"
    );
    let fixed = pages[0]
        .iter()
        .find(|paint| paint.rect.height == 20.0)
        .expect("the fixed-background div paints");
    let layer = &fixed.background_layers(PAGE_AREA).unwrap()[0];
    close(
        "fixed layers centre in the page area, not the box",
        layer.tile.y,
        PAGE_AREA.y + (PAGE_AREA.height - 30.0) / 2.0,
    );
    assert_eq!(
        layer.clip.rect, fixed.rect,
        "fixed layers are still clipped to their own box"
    );
}

#[test]
fn colour_stop_currentcolor_and_invalid_gradients_are_resolved_at_computed_time() {
    for source in [
        "linear-gradient(currentcolor)",
        "linear-gradient(currentcolor,blue)",
    ] {
        let resolved = gradient(source);
        assert_eq!(
            resolved.stops[0].color,
            Color::BLACK,
            "{source}: currentcolor uses the element colour"
        );
    }
    // Outside the CSS Images grammar: the declaration is dropped for the earlier gradient.
    for invalid in [
        "linear-gradient(red,10%,20%,blue)",
        "linear-gradient(to left right,red,blue)",
    ] {
        let kept = gradient(&format!(
            "linear-gradient(red,blue);background-image:{invalid}"
        ));
        assert_eq!(
            kept.stops.len(),
            2,
            "{invalid} must be dropped, keeping the earlier two-stop gradient"
        );
    }
    let undrawable = "radial-gradient(circle 10% 20%,red,blue)";
    let html = format!("<style>{PROFILE}div{{background-image:{undrawable}}}</style><div>A</div>");
    assert!(
        render_html(&html, &options()).is_err(),
        "a circle with two radii must be reported rather than painted: {undrawable}"
    );
}

#[test]
fn math_functions_position_and_size_layers_against_each_positioning_area() {
    let pages = paints(
        "div{height:40pt;background:linear-gradient(red,blue) no-repeat;\
         background-size:calc(25% + 10pt) calc(50% - 5pt);\
         background-position:calc(100% - 10pt) calc(50% + 2pt)}\
         .narrow{width:100pt}.wide{width:200pt}\
         .negative{width:100pt;background-size:calc(10pt - 20pt) 10pt}",
        "<div class='narrow'>A</div><div class='wide'>B</div><div class='negative'>C</div>",
    );
    let boxes = &pages[0];
    assert_eq!(boxes.len(), 3, "one paint per box: {boxes:?}");
    for (paint, width) in [(&boxes[0], 100.0), (&boxes[1], 200.0)] {
        let layers = paint.background_layers(PAGE_AREA).unwrap();
        assert_eq!(layers.len(), 1, "{width}pt box has one gradient layer");
        let tile = layers[0].tile;
        let size = [0.25 * width + 10.0, 0.5 * 40.0 - 5.0];
        close(&format!("{width}pt box: calc() width"), tile.width, size[0]);
        close(
            &format!("{width}pt box: calc() height"),
            tile.height,
            size[1],
        );
        close(
            &format!("{width}pt box: calc(100% - 10pt) of the free width"),
            tile.x - paint.rect.x,
            width - size[0] - 10.0,
        );
        close(
            &format!("{width}pt box: calc(50% + 2pt) of the free height"),
            tile.y - paint.rect.y,
            0.5 * (40.0 - size[1]) + 2.0,
        );
    }
    let negative = &boxes[2];
    assert_eq!(
        negative.decoration.backgrounds.sizes[0],
        BackgroundSize::Explicit([
            SizeComponent::Value(crate::LengthPercentage::Length(0.0)),
            SizeComponent::Value(crate::LengthPercentage::Length(10.0)),
        ]),
        "a negative calc() size clamps to zero instead of invalidating the declaration"
    );
    assert!(
        negative.background_layers(PAGE_AREA).unwrap().is_empty(),
        "a zero-width tile paints nothing"
    );
}

/// The non-premultiplied colour the PDF shading interpolates linearly between stops.
fn shaded(stops: &[GradientStop], offset: f32) -> [f32; 4] {
    let after = stops
        .partition_point(|stop| stop.offset < offset)
        .min(stops.len() - 1);
    let before = after.saturating_sub(1);
    let (a, b) = (stops[before], stops[after]);
    let t = if b.offset > a.offset {
        ((offset - a.offset) / (b.offset - a.offset)).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let channel = |index: usize| {
        f32::from(a.color.rgb[index])
            + (f32::from(b.color.rgb[index]) - f32::from(a.color.rgb[index])) * t
    };
    [
        channel(0),
        channel(1),
        channel(2),
        a.color.alpha + (b.color.alpha - a.color.alpha) * t,
    ]
}

/// Compares the shaded line with `expected(offset)` at every hundredth of the line.
fn assert_shading(label: &str, stops: &[GradientStop], expected: impl Fn(f32) -> Color) {
    for step in 0..=100 {
        let offset = step as f32 / 100.0;
        let actual = shaded(stops, offset);
        let expected = expected(offset);
        for channel in 0..3 {
            assert!(
                (actual[channel] - f32::from(expected.rgb[channel])).abs() <= COLOR_TOLERANCE,
                "{label}: at {offset} the shading is {actual:?}, the CSS colour {expected:?}\n\
                 stops {stops:?}"
            );
        }
        close(
            &format!("{label}: alpha at {offset}"),
            actual[3],
            expected.alpha,
        );
    }
}

#[test]
fn gradients_interpolate_in_their_colour_space_within_output_precision() {
    use crate::paint::{AbsoluteColor, ColorSpace, HueInterpolation, Interpolation};
    let red = AbsoluteColor::srgb([1.0, 0.0, 0.0], 1.0);
    let blue = AbsoluteColor::srgb([0.0, 0.0, 1.0], 1.0);
    let legacy = gradient("linear-gradient(to right,red,blue)");
    assert_eq!(
        legacy.stops.len(),
        2,
        "legacy colours interpolate in gamma-encoded sRGB, which PDF shading reproduces exactly \
         (CSS Color 4 §13.2): {:?}",
        legacy.stops
    );
    let oklab = Interpolation {
        space: ColorSpace::Oklab,
        hue: HueInterpolation::Shorter,
    };
    for image in [
        "linear-gradient(to right,color(srgb 1 0 0),color(srgb 0 0 1))",
        "linear-gradient(in oklab to right,red,blue)",
    ] {
        let resolved = gradient(image);
        assert!(
            resolved.stops.len() > 2,
            "{image}: Oklab is not linear in sRGB, so the line is sampled: {:?}",
            resolved.stops
        );
        assert_shading(image, &resolved.stops, |offset| {
            oklab.interpolate(&red, &blue, offset).used()
        });
    }
    let longer = Interpolation {
        space: ColorSpace::Oklch,
        hue: HueInterpolation::Longer,
    };
    let rainbow = gradient("linear-gradient(in oklch longer hue to right,red,blue)");
    assert_shading("the longer oklch arc", &rainbow.stops, |offset| {
        longer.interpolate(&red, &blue, offset).used()
    });
    let middle = shaded(&rainbow.stops, 0.5);
    assert!(
        middle[1] > middle[0] && middle[1] > middle[2],
        "the longer arc from red to blue passes through green: {middle:?}"
    );
    let fade = gradient("linear-gradient(in oklab to right,red,transparent)");
    assert!(
        fade.stops.iter().all(|stop| stop.color.rgb == [255, 0, 0]),
        "premultiplied interpolation fades red without darkening (§13.4): {:?}",
        fade.stops
    );
    let neutral = gradient("linear-gradient(in oklch to right,red,#888,green)");
    let shorter = Interpolation {
        space: ColorSpace::Oklch,
        hue: HueInterpolation::Shorter,
    };
    let grey = AbsoluteColor::from_rgb8([0x88; 3], 1.0);
    let green = AbsoluteColor::from_rgb8([0, 128, 0], 1.0);
    assert_shading(
        "grey's missing hue takes each neighbour's (CSS Images 4 §3.5.2)",
        &neutral.stops,
        |offset| {
            if offset <= 0.5 {
                shorter.interpolate(&red, &grey, offset * 2.0).used()
            } else {
                shorter
                    .interpolate(&grey, &green, offset * 2.0 - 1.0)
                    .used()
            }
        },
    );
}

#[test]
fn conic_gradients_place_angular_stops_around_their_rotated_centre() {
    let resolved =
        gradient("conic-gradient(from 90deg at 25% 30%,red,lime 25%,blue calc(45deg + 25%),red)");
    let Shading::Conic { center, angle } = resolved.shading else {
        panic!("conic gradients are sweep shadings: {:?}", resolved.shading)
    };
    close("centre x is 25% of 200pt", center[0], 50.0);
    close("centre y is 30% of 100pt", center[1], 30.0);
    close("from rotates the whole gradient", angle, 90.0);
    assert_stops(
        "angles and percentages are fractions of a turn (CSS Images 4 §3.3.2)",
        &resolved.stops,
        &[
            (0.0, [255, 0, 0], 1.0),
            (0.25, [0, 255, 0], 1.0),
            (0.375, [0, 0, 255], 1.0),
            (1.0, [255, 0, 0], 1.0),
        ],
    );
    let centred = gradient("conic-gradient(red,blue)");
    assert!(
        matches!(centred.shading, Shading::Conic { center: [100.0, 50.0], angle } if angle == 0.0),
        "the default centre is the box centre and the default angle 0deg: {:?}",
        centred.shading
    );
    let repeating = gradient("repeating-conic-gradient(red 0deg 10%,blue 10% 20%)");
    let restarts: Vec<f32> = repeating
        .stops
        .windows(2)
        .filter(|pair| pair[0].offset == pair[1].offset && pair[0].color != pair[1].color)
        .map(|pair| pair[0].offset)
        .collect();
    assert_eq!(
        restarts.len(),
        9,
        "a 20% period has hard edges at every tenth of the turn: {restarts:?}"
    );
    for (index, offset) in restarts.iter().enumerate() {
        close(&format!("edge {index}"), *offset, 0.1 * (index + 1) as f32);
    }
}

#[test]
fn image_set_chooses_the_highest_supported_resolution_as_natural_resolution() {
    let tile = |image: &str| {
        let paint = first(
            "div{width:200pt;height:100pt;background-repeat:no-repeat;border:1pt solid}",
            &format!("<div style=\"background-image:{image}\">A</div>"),
        );
        paint
            .background_layers(PAGE_AREA)
            .unwrap()
            .first()
            .map(|layer| [layer.tile.width, layer.tile.height])
    };
    assert_eq!(
        tile(
            "image-set(url(../assets/missing.png) 1x,url(../assets/image-pattern.png) 2x,\
             url(../assets/missing.png) 2x,url(../assets/missing.png) 4x type('image/x-unknown'))"
        ),
        Some([30.0, 15.0]),
        "the 80x40px image at 2x is 40x20px: later duplicates of a resolution and unsupported \
         types drop out (CSS Images 4 §2.4)"
    );
    assert_eq!(
        tile("image-set('../assets/image-pattern.png' 192dpi type('image/png; q=1'))"),
        Some([30.0, 15.0]),
        "strings are URLs and 192dpi is 2dppx"
    );
    assert_eq!(
        tile("image-set(url(../assets/image-pattern.png))"),
        Some([60.0, 30.0]),
        "an option without a resolution is 1x"
    );
    assert_eq!(
        tile("image-set(url(../assets/image-pattern.svg) 2x)"),
        tile("url(../assets/image-pattern.svg)"),
        "a resolution only scales raster images"
    );
    assert_eq!(
        tile("image-set(url(../assets/missing.png) type('text/plain'))"),
        None,
        "without a valid option image-set() is an invalid image, which paints nothing"
    );
    assert!(
        tile("image-set(linear-gradient(red,blue) 1x,url(../assets/missing.png) 0.5x)").is_some(),
        "generated images are options too"
    );
}

#[test]
fn a_dark_root_scheme_paints_the_canvas_surface_beneath_the_root_background() {
    let canvas_in = |css: &str, body: &str| {
        paints(css, body)
            .remove(0)
            .into_iter()
            .map(|paint| paint.decoration.background)
            .find(|background| background.alpha > 0.0)
    };
    let canvas = |css: &str| canvas_in(css, "<p>A</p>");
    let dark_canvas = Some(Color {
        rgb: [0x12; 3],
        alpha: 1.0,
    });
    assert_eq!(
        canvas_in(
            "",
            "<meta name=color-scheme content='not valid!'><meta name=COLOR-SCHEME content=dark><p>A</p>"
        ),
        dark_canvas,
        "color-scheme: normal takes the first valid meta value as the page's schemes \
         (HTML §4.2.5.1)"
    );
    assert_eq!(
        canvas(""),
        None,
        "a light canvas is the white paper, so nothing is painted"
    );
    assert_eq!(
        canvas(":root{color-scheme:dark}"),
        dark_canvas,
        "the root's used dark scheme makes the canvas surface its dark Canvas colour \
         (CSS Color Adjustment 1 §2.2)"
    );
    assert_eq!(
        canvas(":root{color-scheme:dark}body{background:rgb(255 0 0 / 50%)}"),
        Some(Color {
            rgb: [137, 9, 9],
            alpha: 1.0
        }),
        "the propagated half-opaque background composites over the surface"
    );
}
