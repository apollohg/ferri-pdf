use crate::resources::{ResourceMetadata, Url, sniffing};

fn unknown(bytes: &[u8]) -> String {
    sniffing::computed_type(None, false, bytes, |_| true, |_| true)
}

#[test]
fn unknown_resources_sniff_scriptable_types_before_binary_signatures() {
    for (bytes, expected) in [
        (&b" \t\n<!doctype HTML>"[..], "text/html"),
        (b"<HtMl>", "text/html"),
        (b"<p>", "text/html"),
        (b"<!-- comment -->", "text/html"),
        (b"\x0C<b ", "text/html"),
        (b"<?xml version='1.0'?>", "text/xml"),
        (b"%PDF-1.7", "application/pdf"),
        (b"%!PS-Adobe-3.0", "application/postscript"),
        (b"\xEF\xBB\xBFtext", "text/plain"),
        (b"\x89PNG\r\n\x1A\n\0\0\0\rIHDR", "image/png"),
        (b"GIF89a....", "image/gif"),
        (b"RIFF\x10\0\0\0WEBPVP8 ", "image/webp"),
        (b"RIFF\x10\0\0\0WAVEfmt ", "audio/wave"),
        (b"ID3\x03", "audio/mpeg"),
        (b"\0\0\0\x18ftypmp42\0\0\0\0isommp42", "video/mp4"),
        (
            b"\x1A\x45\xDF\xA3\x9F\x42\x86\x81\x01\x42\x82\x84webm\x42\x87",
            "video/webm",
        ),
        (b"PK\x03\x04", "application/zip"),
        (b"plain words only", "text/plain"),
        (b"\0\x01\x02binary", "application/octet-stream"),
    ] {
        assert_eq!(
            unknown(bytes),
            expected,
            "MIME Sniffing §7.1 for {:?}",
            String::from_utf8_lossy(bytes)
        );
    }
}

#[test]
fn html_tags_need_a_tag_terminating_byte_and_xml_is_case_sensitive() {
    assert_eq!(
        unknown(b"<pre>"),
        "text/plain",
        "<P must be followed by a space or `>`, so <pre> is not the <P pattern"
    );
    assert_eq!(
        unknown(b"<p"),
        "text/plain",
        "a tag at the end of the header has no terminating byte"
    );
    assert_eq!(
        unknown(b"<?XML version='1.0'?>"),
        "text/plain",
        "the XML declaration row matches case-sensitively"
    );
}

#[test]
fn supplied_types_govern_except_unknown_and_apache_text() {
    let png = b"\x89PNG\r\n\x1A\n\0\0\0\rIHDR";
    let computed = |supplied, apache_bug, bytes: &[u8]| {
        sniffing::computed_type(Some(supplied), apache_bug, bytes, |_| true, |_| false)
    };
    assert_eq!(
        computed("text/html", false, png),
        "text/html",
        "HTML types are never sniffed"
    );
    assert_eq!(
        computed("image/svg+xml", false, png),
        "image/svg+xml",
        "XML types are never sniffed"
    );
    assert_eq!(
        computed("*/*", false, png),
        "image/png",
        "*/* is an unknown type"
    );
    assert_eq!(
        computed("image/gif", false, png),
        "image/png",
        "supported image types are sniffed"
    );
    assert_eq!(
        computed("video/mp4", false, png),
        "video/mp4",
        "unsupported media types are kept"
    );
    assert_eq!(
        computed("text/plain", true, b"\0\x01binary"),
        "application/octet-stream",
        "the Apache text/plain bug distinguishes binary resources"
    );
    assert_eq!(computed("text/plain", true, b"words"), "text/plain");
}

#[test]
fn local_files_take_the_type_the_file_system_gives_their_extension() {
    let file = |path: &str| Url::parse(&format!("file:///documents/{path}")).unwrap();
    let empty = ResourceMetadata::default();
    assert_eq!(
        sniffing::supplied_type(&file("page.HTML"), &empty).as_deref(),
        Some("text/html")
    );
    assert_eq!(
        sniffing::supplied_type(&file("logo.svg"), &empty).as_deref(),
        Some("image/svg+xml")
    );
    assert_eq!(
        sniffing::supplied_type(&file("data.bin"), &empty),
        None,
        "unknown extensions supply no type"
    );
    assert_eq!(sniffing::supplied_type(&file("README"), &empty), None);
    let remote = Url::parse("https://example.org/page.html").unwrap();
    assert_eq!(
        sniffing::supplied_type(&remote, &empty),
        None,
        "HTTP resources never take a type from their extension"
    );
    let declared = ResourceMetadata::from_content_type("text/plain; charset=UTF-8");
    assert!(sniffing::apache_bug(&remote, &declared));
    assert!(
        !sniffing::apache_bug(&file("page.txt"), &declared),
        "only HTTP responses have the bug"
    );
}
