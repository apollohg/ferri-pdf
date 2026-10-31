use ferri_pdf::{Resource, ResourceBase, ResourceError, ResourceResolver, render_with_resolver};
use std::{cell::RefCell, path::PathBuf};

const CORRUPT_PIXEL_DATA_OFFSET: usize = 80;

struct Assets {
    calls: RefCell<Vec<(String, ResourceBase)>>,
    corrupt: bool,
    image_bytes: &'static [u8],
}
impl ResourceResolver for Assets {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        self.calls
            .borrow_mut()
            .push((reference.to_owned(), base.to_owned()));
        let (bytes, directory) = match reference {
            "outer.svg" => (br#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><image href="inner.svg" width="80" height="40"/><image href="inner.svg" width="80" height="40"/></svg>"#.to_vec(),"/virtual/outer"),
            "inner.svg" => (br#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><image href="pattern.png" width="80" height="40"/></svg>"#.to_vec(),"/virtual/inner"),
            "pattern.png" => { let mut bytes = self.image_bytes.to_vec(); if self.corrupt { bytes[CORRUPT_PIXEL_DATA_OFFSET] ^= 0xff; } (bytes,"/virtual/inner") },
            other => return Err(ResourceError::UnsupportedScheme(other.into())),
        };
        Ok(Resource {
            bytes,
            base: PathBuf::from(directory).into(),
        })
    }
}

#[test]
fn nested_svg_images_use_each_resource_base_and_reuse_the_document_asset() {
    let resolver = Assets {
        calls: Default::default(),
        corrupt: false,
        image_bytes: include_bytes!("oracle/assets/image-pattern.png"),
    };
    let output = render_with_resolver(
        "<style>img{display:block}</style><img src='outer.svg'><img src='outer.svg'>",
        &Default::default(),
        &resolver,
    )
    .unwrap();
    assert_eq!(output.page_count, 1);
    let calls = resolver.calls.borrow();
    assert_eq!(
        calls
            .iter()
            .map(|(reference, _)| reference.as_str())
            .collect::<Vec<_>>(),
        ["outer.svg", "inner.svg", "pattern.png"]
    );
    assert_eq!(calls[1].1, PathBuf::from("/virtual/outer").into());
    assert_eq!(calls[2].1, PathBuf::from("/virtual/inner").into());
}

#[test]
fn invalid_nested_bitmap_is_omitted_with_a_diagnostic() {
    let resolver = Assets {
        calls: Default::default(),
        corrupt: true,
        image_bytes: include_bytes!("oracle/assets/image-pattern.png"),
    };
    let result = render_with_resolver(
        "<style>img{display:block}</style><img src='outer.svg'>",
        &Default::default(),
        &resolver,
    );
    let output = result.unwrap();
    assert!(
        matches!(
            output.diagnostics.as_slice(),
            [ferri_pdf::RenderDiagnostic::ImageDecode { .. }]
        ),
        "corrupt nested samples must be omitted with a diagnostic"
    );
}

#[test]
fn external_svg_symbols_are_reported_and_the_rest_of_the_image_renders() {
    let output = ferri_pdf::render_html(
        "<style>img{display:block}</style><img src='data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2280%22 height=%2240%22%3E%3Cuse href=%22external.svg%23icon%22/%3E%3Crect width=%2210%22 height=%2210%22/%3E%3C/svg%3E'>",
        &ferri_pdf::RenderOptions {
            include_layout: true,
            ..Default::default()
        },
    )
    .unwrap();
    let reported: Vec<_> = output
        .diagnostics
        .iter()
        .map(|diagnostic| match diagnostic {
            ferri_pdf::RenderDiagnostic::UnavailableImage(failure) => match &failure.error {
                ferri_pdf::ImageError::ExternalSvgSymbol(reference) => reference.as_str(),
                ferri_pdf::ImageError::Resource(_) => failure.reference.as_str(),
                error => panic!("unexpected image failure {error}"),
            },
            diagnostic => panic!("unexpected diagnostic {diagnostic}"),
        })
        .collect();
    assert_eq!(
        reported,
        ["external.svg#icon", "external.svg"],
        "the external symbol is in error, and the document it names is unavailable"
    );
    assert!(
        output
            .layout
            .unwrap()
            .display_lists
            .iter()
            .flatten()
            .any(|item| matches!(item, ferri_pdf::DisplayItem::Image(_))),
        "the image still paints its other content"
    );
}

#[test]
fn heif_and_avif_nested_in_svg_uses_the_custom_loader_at_both_sample_depths() {
    for bytes in [
        include_bytes!("oracle/assets/image-pattern.heic").as_slice(),
        include_bytes!("oracle/assets/image-high-depth.heic").as_slice(),
        include_bytes!("oracle/assets/image-pattern.avif").as_slice(),
        include_bytes!("oracle/assets/image-high-depth.avif").as_slice(),
    ] {
        let resolver = Assets {
            calls: Default::default(),
            corrupt: false,
            image_bytes: bytes,
        };
        let output = render_with_resolver(
            "<style>img{display:block}</style><img src='outer.svg'><img src='outer.svg'>",
            &Default::default(),
            &resolver,
        )
        .unwrap();
        assert_eq!(output.page_count, 1);
        assert_eq!(
            resolver.calls.borrow().len(),
            3,
            "nested HEIF/AVIF must pass through the loader once, even when reused"
        );
        assert!(output.pdf.starts_with(b"%PDF-"));
    }
}
