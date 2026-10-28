mod common;
use common::local_options;
use ferri_pdf::{FileResolver, ResourceError, ResourceResolver, render_html};
use std::path::{Path, PathBuf};

#[test]
fn data_urls_decode_bytes_and_preserve_the_containing_base() {
    let resolver = FileResolver::default();
    for reference in [
        "data:application/octet-stream;base64,AAH%2B%2Fw%3D%3D",
        "data:,\0%01%FE%FF",
        "DATA:;BASE64,AAH+ /w==\n",
        "data: ;  BASE64 \t,AAH+/w==",
    ] {
        let resource = resolver
            .resolve(reference, &Path::new("assets").into())
            .unwrap();
        assert_eq!(
            resource.bytes,
            [0, 1, 254, 255],
            "decoded bytes for {reference:?}"
        );
        assert_eq!(resource.base, PathBuf::from("assets").into());
    }
    assert_eq!(
        resolver
            .resolve("data:,a+b%23c#fragment", &Path::new(".").into())
            .unwrap()
            .bytes,
        b"a+b#c"
    );
}

#[test]
fn embedded_resources_obey_the_decoded_size_limit() {
    let resolver = FileResolver {
        max_resource_bytes: 4,
        ..Default::default()
    };
    for reference in ["data:,1234", "data:;base64,MTIzNA==", "data:;base64,MTIzNA"] {
        assert_eq!(
            resolver
                .resolve(reference, &Path::new(".").into())
                .unwrap()
                .bytes,
            b"1234"
        );
    }
    for reference in [
        "data:,12345",
        "data:;base64,MTIzNDU=",
        "data:;base64,MTIzNDU",
    ] {
        assert!(
            matches!(
                resolver.resolve(reference, &Path::new(".").into()),
                Err(ResourceError::TooLarge { limit: 4, .. })
            ),
            "decoded resource must be bounded: {reference}"
        );
    }
    for reference in ["data:text/plain", "data:;base64,!bad", "data:;base64,A"] {
        assert!(
            matches!(
                resolver.resolve(reference, &Path::new(".").into()),
                Err(ResourceError::InvalidDataUrl(_))
            ),
            "malformed resource must be rejected: {reference}"
        );
    }
}

#[test]
fn embedded_stylesheets_use_the_same_public_resource_pipeline() {
    let output = render_html(&format!("<style>{}</style><link rel=stylesheet href='data:text/css,p%7Bfont-size%3A20pt%3Bcolor%3Ared%7D'><p>Embedded stylesheet</p>",common::PROFILE),&local_options()).unwrap();
    let trace = output.layout.unwrap();
    let line = &trace.pages[0][0];
    assert_eq!(line.font_size, 20.0);
    assert_eq!(line.color.rgb, [255, 0, 0]);
}

#[test]
fn literal_percent_signs_and_utf8_survive_data_url_decoding() {
    let resolver = FileResolver::default();
    for payload in ["100%", "%x0", "%0", "%x%41", "café", "%%"] {
        let expected = payload.replace("%41", "A");
        assert_eq!(
            resolver
                .resolve(&format!("data:,{payload}"), &Path::new(".").into())
                .unwrap()
                .bytes,
            expected.as_bytes(),
            "literal payload {payload:?}"
        );
    }
}

#[test]
fn data_url_limits_handle_empty_and_unbounded_resources() {
    for limit in [0, usize::MAX] {
        let resolver = FileResolver {
            max_resource_bytes: limit,
            ..Default::default()
        };
        for reference in ["data:,", "data:;base64,"] {
            assert!(
                resolver
                    .resolve(reference, &Path::new(".").into())
                    .unwrap()
                    .bytes
                    .is_empty()
            );
        }
        let result = resolver.resolve("data:;base64,YQ==", &Path::new(".").into());
        if limit == 0 {
            assert!(matches!(
                result,
                Err(ResourceError::TooLarge { limit: 0, .. })
            ));
        } else {
            assert_eq!(result.unwrap().bytes, b"a");
        }
    }
}

#[test]
fn legacy_resolver_metadata_adapter_preserves_resource() {
    use ferri_pdf::{Resource, ResourceBase, ResourceMetadata};
    use std::cell::Cell;

    struct LegacyResolver {
        calls: Cell<usize>,
    }
    impl ResourceResolver for LegacyResolver {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.calls.set(self.calls.get() + 1);
            assert_eq!(reference, "legacy.css");
            assert_eq!(base, &Path::new("assets").into());
            Ok(Resource {
                bytes: b"p{color:red}".to_vec(),
                base: Path::new("redirected").into(),
            })
        }
    }
    let legacy = LegacyResolver {
        calls: Cell::new(0),
    };
    let resolver: &dyn ResourceResolver = &legacy;
    let resolved = resolver
        .resolve_with_metadata("legacy.css", &Path::new("assets").into())
        .unwrap();
    assert_eq!(resolved.resource.bytes, b"p{color:red}");
    assert_eq!(resolved.resource.base, Path::new("redirected").into());
    assert_eq!(resolved.metadata, ResourceMetadata::default());
    assert_eq!(legacy.calls.get(), 1, "metadata adapter must resolve once");
}

#[test]
fn data_metadata_preserves_mime_parameters_and_default_type() {
    let base = Path::new("assets").into();
    for (reference, media_type, charset) in [
        (
            "data:TEXT/CSS;charset=\"windows-1252\";base64,YQ==",
            "text/css",
            Some("windows-1252"),
        ),
        ("data:;charset=latin1,a", "text/plain", Some("latin1")),
        ("data:,a", "text/plain", Some("US-ASCII")),
        ("data:invalid,a", "text/plain", Some("US-ASCII")),
        (
            "data:text/css;note=\"a;charset=wrong\";charset=utf-8,a",
            "text/css",
            Some("utf-8"),
        ),
    ] {
        let resolved = FileResolver::default()
            .resolve_with_metadata(reference, &base)
            .unwrap();
        assert_eq!(resolved.resource.bytes, b"a", "{reference}");
        assert_eq!(resolved.resource.base, base);
        assert_eq!(resolved.metadata.media_type.as_deref(), Some(media_type));
        assert_eq!(resolved.metadata.charset.as_deref(), charset);
    }
}
