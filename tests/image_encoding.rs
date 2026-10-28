mod common;

use ferri_pdf::{ImageDpi, ImageOptions, JpegQuality, render_html};

#[test]
fn jpeg_quality_changes_encoding_without_changing_layout_or_image_placement() {
    let html = "<style>img{width:120pt;height:90pt}</style><img src='../assets/image-pattern.jpg'>";
    let mut options = common::local_options();
    let original = render_html(html, &options).unwrap();
    options.images = ImageOptions {
        jpeg_quality: Some(JpegQuality::new(60).unwrap()),
        ..Default::default()
    };
    let compressed = render_html(html, &options).unwrap();
    assert_ne!(
        original.pdf, compressed.pdf,
        "the public API must reach the encoded JPEG stream"
    );
    assert_eq!(original.page_count, compressed.page_count);
    assert_eq!(
        serde_json::to_value(original.layout.unwrap()).unwrap(),
        serde_json::to_value(compressed.layout.unwrap()).unwrap(),
        "encoding cannot alter measured text, image rectangles or page geometry"
    );
}

#[test]
fn jpeg_quality_preserves_lossless_alpha_and_high_precision_image_output() {
    let html = "<img src='../assets/image-avif-alpha-grid-reference.png'><img src='../assets/image-high-depth.png'>";
    let mut options = common::local_options();
    let original = render_html(html, &options).unwrap();
    options.images.jpeg_quality = Some(JpegQuality::new(60).unwrap());
    let compressed = render_html(html, &options).unwrap();
    assert_eq!(
        original.pdf, compressed.pdf,
        "a JPEG setting cannot convert lossless or transparent input to JPEG"
    );
}

#[test]
fn image_dpi_changes_samples_without_changing_document_layout() {
    let html = "<style>img{width:12pt;height:9pt}</style><img src='../assets/image-pattern.jpg'>";
    let mut options = common::local_options();
    let original = render_html(html, &options).unwrap();
    options.images.dpi = Some(ImageDpi::new(150.0).unwrap());
    let reduced = render_html(html, &options).unwrap();
    assert_ne!(
        original.pdf, reduced.pdf,
        "DPI must change the serialized raster"
    );
    assert_eq!(original.page_count, reduced.page_count);
    assert_eq!(
        serde_json::to_value(original.layout.unwrap()).unwrap(),
        serde_json::to_value(reduced.layout.unwrap()).unwrap(),
        "DPI cannot affect intrinsic image sizing or pagination"
    );
}
