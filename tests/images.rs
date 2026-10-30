mod common;
use ferri_pdf::render_html;

#[test]
fn raster_blocks_reserve_intrinsic_size_before_following_text() {
    let mut options = common::local_options();
    options.base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/oracle/fixtures")
        .into();
    let output = render_html(include_str!("oracle/fixtures/image-block.html"), &options).unwrap();
    let trace = output.layout.unwrap();
    let lines = &trace.pages[0];
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Before", "After", "Last"]
    );
    for adjacent in lines.windows(2) {
        assert!(
            (adjacent[1].baseline - adjacent[0].baseline - 48.0).abs() < 0.01,
            "intrinsic image and text advance: {} -> {}",
            adjacent[0].baseline,
            adjacent[1].baseline
        );
    }
    assert!(output.pdf.starts_with(b"%PDF-"));
}

#[test]
fn hidden_images_do_not_load_missing_resources() {
    render_html(
        &format!(
            "<style>{}img{{display:none}}</style><img src='absent.png'><p>Visible</p>",
            common::PROFILE
        ),
        &common::local_options(),
    )
    .unwrap();
}

fn image_options() -> ferri_pdf::RenderOptions {
    let mut options = common::local_options();
    options.base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/oracle/fixtures")
        .into();
    options
}

#[test]
fn block_image_margins_follow_initial_forced_and_automatic_break_rules() {
    const PAGE_TOP: f32 = 10.0;
    const IMAGE_MARGIN: f32 = 7.0;
    const IMAGE_SIZE: f32 = 30.0;
    for (name, before, extra, page, top) in [
        ("initial", "", "", 0, PAGE_TOP + IMAGE_MARGIN),
        (
            "automatic",
            "<div style='height:170pt'></div>",
            "",
            1,
            PAGE_TOP,
        ),
        (
            "forced-before",
            "<div style='height:20pt'></div>",
            "img{break-before:page}",
            1,
            PAGE_TOP + IMAGE_MARGIN,
        ),
        (
            "forced-after",
            "<div style='height:20pt;break-after:page'></div>",
            "",
            1,
            PAGE_TOP + IMAGE_MARGIN,
        ),
        (
            "nonadjoining-inner-margin",
            "<div style='height:170pt'></div>",
            "section{padding:1pt}",
            1,
            PAGE_TOP + 1.0 + IMAGE_MARGIN,
        ),
    ] {
        let html = format!(
            "<!doctype html><style>{}@page{{size:200pt 200pt;margin:{PAGE_TOP}pt}}img{{display:block;width:{IMAGE_SIZE}pt;height:{IMAGE_SIZE}pt;margin-top:{IMAGE_MARGIN}pt}}{extra}</style>{before}<section><img src='../assets/image-pattern.png'></section>",
            common::PROFILE
        );
        let trace = render_html(&html, &image_options())
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .layout
            .unwrap();
        assert_eq!(trace.pages.len(), page + 1, "{name}: page count");
        let images: Vec<_> = trace
            .display_lists
            .iter()
            .enumerate()
            .flat_map(|(page, items)| {
                items.iter().filter_map(move |item| match item {
                    ferri_pdf::DisplayItem::Image(image) => Some((page, image)),
                    _ => None,
                })
            })
            .collect();
        assert_eq!(images.len(), 1, "{name}: image must paint exactly once");
        assert_eq!(images[0].0, page, "{name}: image page");
        common::assert_pt(images[0].1.rect.y, top);
        common::assert_pt(images[0].1.rect.height, IMAGE_SIZE);
        common::assert_pt(images[0].1.clip.as_ref().unwrap().rect.y, top);
    }
}

#[test]
fn oversized_raster_moves_to_its_own_page_without_fragmenting() {
    let output = render_html(&format!("<style>{}p{{margin:0}}img{{display:block;width:100pt;height:500pt}}</style><p>Before</p><img src='../assets/image-pattern.png'><p>After</p>",common::PROFILE), &image_options()).unwrap();
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.len(),
        3,
        "Before, the atomic oversized image, and After need separate pages"
    );
    assert_eq!(trace.pages[0][0].text, "Before");
    assert!(trace.pages[1].is_empty());
    assert_eq!(trace.pages[2][0].text, "After");
}

#[test]
fn moving_image_paint_off_page_preserves_its_normal_flow_page() {
    let output = render_html(&format!("<style>{}p{{margin:0}}img{{display:block;height:408pt;position:relative;left:1000pt}}</style><img src='../assets/image-pattern.png'><p>After</p>",common::PROFILE), &image_options()).unwrap();
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.len(),
        2,
        "relative offsets must not erase the page reserved by the image"
    );
    assert!(trace.pages[0].is_empty());
    assert_eq!(trace.pages[1][0].text, "After");
}

#[test]
fn object_fitting_keeps_content_geometry_and_clips_to_the_content_box() {
    let trace = render_html(
        include_str!("oracle/fixtures/image-fit.html"),
        &image_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let images: Vec<_> = trace.display_lists[0]
        .iter()
        .filter_map(|item| {
            if let ferri_pdf::DisplayItem::Image(paint) = item {
                Some(paint)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(images.len(), 5);
    for (paint, size) in images.iter().zip([
        [80.0, 35.0],
        [70.0, 35.0],
        [80.0, 40.0],
        [60.0, 30.0],
        [60.0, 30.0],
    ]) {
        assert_eq!([paint.rect.width, paint.rect.height], size);
        assert!(
            std::sync::Arc::ptr_eq(&paint.image, &images[0].image),
            "repeated references must reuse one parsed asset"
        );
    }
    for image in &images {
        let clip = image.clip.as_ref().unwrap().rect;
        assert_eq!([clip.width, clip.height], [80.0, 35.0]);
    }
}

#[test]
fn a_spacer_before_an_atomic_image_preserves_the_blank_page() {
    let trace = render_html(
        include_str!("oracle/fixtures/image-raster-pagination.html"),
        &image_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!(
        trace.pages.len(),
        4,
        "the explicit spacer, first image, oversized image, and final text occupy four pages"
    );
    assert!(trace.pages[0].is_empty());
    assert_eq!(trace.pages[1][0].text, "After");
    assert!(trace.pages[2].is_empty());
    assert_eq!(trace.pages[3][0].text, "Last");
}

#[test]
fn overlapping_images_and_text_paint_in_source_order_above_block_backgrounds() {
    let trace = render_html(
        include_str!("oracle/fixtures/image-paint-order.html"),
        &image_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let items = &trace.display_lists[0];
    let position = |predicate: &dyn Fn(&ferri_pdf::DisplayItem) -> bool| {
        items.iter().position(predicate).unwrap()
    };
    let background = position(&|item| matches!(item, ferri_pdf::DisplayItem::Box(_)));
    let before = position(
        &|item| matches!(item,ferri_pdf::DisplayItem::Text{line} if trace.pages[0][*line].text=="Before"),
    );
    let image = position(&|item| matches!(item, ferri_pdf::DisplayItem::Image(_)));
    let after = position(
        &|item| matches!(item,ferri_pdf::DisplayItem::Text{line} if trace.pages[0][*line].text=="After"),
    );
    assert!(
        background < before && before < image && image < after,
        "foreground order is preserved while later block backgrounds stay behind: background={background}, before={before}, image={image}, after={after}"
    );
}

#[test]
fn svg_blocks_use_explicit_dimensions_or_available_width_with_viewbox_ratio() {
    let trace = render_html(
        include_str!("oracle/fixtures/image-svg.html"),
        &image_options(),
    )
    .expect("external SVG blocks should render")
    .layout
    .unwrap();
    let sizes: Vec<_> = trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| {
            if let ferri_pdf::DisplayItem::Image(paint) = item {
                Some([paint.rect.width, paint.rect.height])
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        sizes,
        [[60.0, 30.0], [288.0, 144.0], [100.0, 50.0]],
        "SVG uses explicit dimensions, available width for ratio-only SVG, and CSS width with intrinsic ratio"
    );
}

#[test]
fn svg_unsized_fallback_does_not_expand_to_the_block_width() {
    let trace = render_html(
        "<style>img{display:block}</style><img src='../assets/svg-unsized.svg'>",
        &image_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let image = trace
        .display_lists
        .iter()
        .flatten()
        .find_map(|item| match item {
            ferri_pdf::DisplayItem::Image(paint) => Some(paint),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        [image.rect.width, image.rect.height],
        [225.0, 112.5],
        "SVG without intrinsic dimensions or ratio uses the 300 by 150 CSS pixel default"
    );
}

#[test]
fn heif_and_avif_images_decode_alpha_rotation_and_grid_through_the_public_renderer() {
    let mut errors = Vec::new();
    for (name, expected) in [
        ("image-lossy.heic", [60.0, 30.0]),
        ("image-high-depth.heic", [60.0, 30.0]),
        ("image-pattern.heic", [60.0, 30.0]),
        ("image-rotated.heic", [30.0, 60.0]),
        ("image-grid.heic", [60.0, 30.0]),
        ("image-lossy.avif", [60.0, 30.0]),
        ("image-high-depth.avif", [60.0, 30.0]),
        ("image-pattern.avif", [60.0, 30.0]),
        ("image-rotated.avif", [30.0, 60.0]),
        ("image-grid.avif", [60.0, 30.0]),
        ("image-alpha-grid.avif", [60.0, 30.0]),
    ] {
        let html = format!("<style>img{{display:block}}</style><img src='../assets/{name}'>");
        let output = match render_html(&html, &image_options()) {
            Ok(output) => output,
            Err(error) => {
                errors.push(format!("{name}: {error}"));
                continue;
            }
        };
        let trace = output.layout.unwrap();
        let paint = trace
            .display_lists
            .iter()
            .flatten()
            .find_map(|item| match item {
                ferri_pdf::DisplayItem::Image(paint) => Some(paint),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            [paint.rect.width, paint.rect.height],
            expected,
            "decoded intrinsic dimensions for {name}"
        );
    }
    assert!(errors.is_empty(), "HEIF/AVIF fixture failures: {errors:#?}");
}

#[test]
fn ratio_only_svg_stretches_into_the_available_width_when_shrink_to_fit() {
    let trace = render_html(
        include_str!("oracle/fixtures/image-ratio-intrinsic.html"),
        &image_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let rects: Vec<_> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            ferri_pdf::DisplayItem::Image(paint) => Some(paint.rect),
            _ => None,
        })
        .collect();
    // viewBox 80x40: the flex logo stretches to 200pt and max-width caps it at 60pt; the
    // inline-block and the table cell stretch into their 90pt containing block.
    let expected = [[60.0, 30.0], [90.0, 45.0], [90.0, 45.0]];
    assert_eq!(
        rects.len(),
        expected.len(),
        "every viewBox-only image must paint, got {rects:?}"
    );
    for (index, (rect, size)) in rects.iter().zip(expected).enumerate() {
        assert!(
            (rect.width - size[0]).abs() < 0.01 && (rect.height - size[1]).abs() < 0.01,
            "image {index}: {rect:?}, expected size {size:?}; all rects {rects:?}"
        );
    }
    assert!(
        (rects[0].x - (36.0 + (200.0 - 60.0) / 2.0)).abs() < 0.01,
        "align-items:center centres the capped logo in the 200pt column, got {:?}",
        rects[0]
    );
}
