mod common;

use ferri_pdf::{
    FileResolver, HtmlPart, Resource, ResourceBase, ResourceError, ResourceResolver,
    render_parts_with_resolver, render_with_resolver,
};

const BROKEN_IMAGE: &str = "broken.png";

struct ImageResolver(Option<Vec<u8>>);
impl ResourceResolver for ImageResolver {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        if reference == BROKEN_IMAGE {
            return self
                .0
                .as_ref()
                .map(|bytes| Resource {
                    bytes: bytes.clone(),
                    base: base.clone(),
                })
                .ok_or_else(|| ResourceError::Read {
                    path: BROKEN_IMAGE.into(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "synthetic missing image",
                    ),
                });
        }
        FileResolver::default().resolve(reference, base)
    }
}

fn corrupt_png_samples() -> Vec<u8> {
    let mut bytes = include_bytes!("oracle/assets/image-pattern.png").to_vec();
    let start = bytes
        .windows(4)
        .position(|chunk| chunk == b"IDAT")
        .expect("PNG has image data");
    bytes[start + 4] ^= u8::MAX;
    bytes
}

#[test]
fn unavailable_and_corrupt_images_do_not_discard_surrounding_document_content() {
    for (case, bytes) in [
        ("missing", None),
        ("header", Some(b"broken image".to_vec())),
        ("samples", Some(corrupt_png_samples())),
    ] {
        let html = format!(
            "<style>{}</style><p>Before</p><img src='{BROKEN_IMAGE}'><img src='{BROKEN_IMAGE}'><p>After</p><img src='../assets/image-pattern.png' width=20>",
            common::PROFILE
        );
        let output = render_with_resolver(&html, &common::local_options(), &ImageResolver(bytes))
            .unwrap_or_else(|error| {
                panic!("{case}: one failed image must not abort the PDF: {error}")
            });
        assert!(output.pdf.starts_with(b"%PDF-"));
        assert_eq!(
            output.diagnostics.len(),
            1,
            "{case}: report each failed source once"
        );
        let trace = output.layout.unwrap();
        for label in ["Before", "After"] {
            assert!(
                trace.pages.iter().flatten().any(|line| line.text == label),
                "{case}: lost {label}"
            );
        }
    }
}

#[test]
fn image_failure_diagnostics_are_retained_for_each_multipart_document() {
    let html = format!(
        "<style>{}</style><img src='{BROKEN_IMAGE}'><p>Part</p>",
        common::PROFILE
    );
    let options = common::local_options();
    let parts = [HtmlPart::from_html(&html), HtmlPart::from_html(&html)];
    let output = render_parts_with_resolver(
        &parts,
        &options,
        &ImageResolver(Some(corrupt_png_samples())),
    )
    .unwrap();
    assert_eq!(output.parts.len(), 2);
    for (index, part) in output.parts.iter().enumerate() {
        assert_eq!(
            part.diagnostics.len(),
            1,
            "part {index} must retain its image diagnostic"
        );
    }
}

#[test]
fn absent_or_unavailable_image_sources_preserve_alternative_text() {
    for image in [
        "<img alt='Alternative'>",
        "<img src='broken.png' alt='Alternative'>",
    ] {
        let html = format!(
            "<style>{}</style><p>Before</p>{image}<p>After</p>",
            common::PROFILE
        );
        let output =
            render_with_resolver(&html, &common::local_options(), &ImageResolver(None)).unwrap();
        assert!(
            output
                .layout
                .unwrap()
                .pages
                .iter()
                .flatten()
                .any(|line| line.text == "Alternative")
        );
    }
}

#[test]
fn corrupt_marker_samples_choose_the_text_fallback_before_layout() {
    for position in ["inside", "outside"] {
        for marker in ["decimal", "none"] {
            let html = format!(
                "<style>{}li{{list-style-position:{position};list-style-type:{marker};list-style-image:url('{BROKEN_IMAGE}')}}</style><ol><li>Before</li><li>After</li></ol>",
                common::PROFILE
            );
            let output = render_with_resolver(
                &html,
                &common::local_options(),
                &ImageResolver(Some(corrupt_png_samples())),
            )
            .unwrap();
            assert_eq!(output.diagnostics.len(), 1);
            let trace = output.layout.unwrap();
            let text = trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<String>();
            assert!(
                text.contains("Before") && text.contains("After"),
                "{text:?}"
            );
            assert_eq!(
                text.contains("1. "),
                marker == "decimal",
                "fallback {position}/{marker}: {text:?}"
            );
            assert_eq!(
                text.contains("2. "),
                marker == "decimal",
                "fallback {position}/{marker}: {text:?}"
            );
        }
    }
}
