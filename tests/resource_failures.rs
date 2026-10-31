//! Missing, denied, truncated, garbled and oversized resources never make rendering fail:
//! images take their broken state (HTML §4.8.4.3) or are invalid (CSS Images 4 §2.4),
//! fonts fall back (CSS Fonts 4 §4.3), and style sheets and nested documents are skipped,
//! each with a diagnostic.
mod common;

use ferri_pdf::{
    DisplayItem, FileResolver, ImageError, RenderDiagnostic, RenderOutput, Resource, ResourceBase,
    ResourceError, ResourceResolver, render_with_resolver,
};
use std::collections::HashMap;

const SEED: u64 = 0x5eed_f00d_cafe_d00d;
const GARBLED_VARIANTS: usize = 6;
/// Bytes each garbled variant overwrites, per kilobyte of the original.
const GARBLED_BYTES_PER_KIB: usize = 16;
const HEADER_BYTES: usize = 16;
const ALT: &str = "ALT";
const FALLBACK: &str = "FALLBACK";
const MISSING: &str = "missing";
const DENIED: &str = "denied";
const FILE_SCHEME: &str = "file";
const SHEET_FONT_SIZE_PT: f32 = 17.0;
/// Bytes a sparsely damaged font overwrites.
const SPARSE_GARBLED_BYTES: usize = 64;
const INSTANCE_FAILURE: &str = "font instance";

/// The bytes a garbled variant of `bytes` bytes overwrites.
fn garbled_count(bytes: usize) -> usize {
    (bytes * GARBLED_BYTES_PER_KIB / 1024).max(1)
}

/// A deterministic xorshift64* sequence.
struct Seeded(u64);
impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % bound as u64).expect("bound fits usize")
    }
    fn bytes(&mut self, count: usize) -> Vec<u8> {
        (0..count).map(|_| self.next().to_le_bytes()[0]).collect()
    }
    /// `data` with `count` bytes in `range` overwritten.
    fn garble(&mut self, data: &[u8], range: std::ops::Range<usize>, count: usize) -> Vec<u8> {
        let mut garbled = data.to_vec();
        for byte in self.bytes(count) {
            let at = range.start + self.below(range.len());
            garbled[at] = byte;
        }
        garbled
    }
}

/// Resources by file name, however a document references them. Names starting with
/// [`DENIED`] are denied by policy, and other references the corpus lacks go to the file
/// resolver, which also rejects malformed `data:` URLs and unknown schemes.
#[derive(Default)]
struct Corpus(HashMap<String, Vec<u8>>);
impl Corpus {
    fn insert(&mut self, name: &str, bytes: Vec<u8>) {
        self.0.insert(name.into(), bytes);
    }
}
impl ResourceResolver for Corpus {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        let url = base.resolve(reference)?;
        let name = (url.scheme() == FILE_SCHEME)
            .then(|| url.path_segments()?.next_back())
            .flatten()
            .unwrap_or_default();
        if name.starts_with(DENIED) {
            return Err(ResourceError::Denied(url));
        }
        match self.0.get(name) {
            Some(bytes) => Ok(Resource {
                bytes: bytes.clone(),
                base: base.clone(),
            }),
            None => FileResolver::default().resolve(reference, base),
        }
    }
}

fn encoded(format: image::ImageFormat) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 255, 255]))
        .write_to(&mut std::io::Cursor::new(&mut bytes), format)
        .expect("encode fixture image");
    bytes
}

fn png_chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
    let mut crc = flate2::Crc::new();
    crc.update(&kind);
    crc.update(data);
    let length = u32::try_from(data.len()).expect("chunk fits");
    [
        &length.to_be_bytes()[..],
        &kind,
        data,
        &crc.sum().to_be_bytes(),
    ]
    .concat()
}

/// A PNG whose header claims `u32::MAX >> 1` pixels square.
fn huge_png() -> Vec<u8> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    const SIDE: u32 = 0x7fff_ffff;
    let header = [
        &SIDE.to_be_bytes()[..],
        &SIDE.to_be_bytes(),
        &[8, 6, 0, 0, 0],
    ]
    .concat();
    [
        SIGNATURE.to_vec(),
        png_chunk(*b"IHDR", &header),
        png_chunk(*b"IDAT", &[]),
        png_chunk(*b"IEND", &[]),
    ]
    .concat()
}

/// `data` with every occurrence of the big-endian dimension pair after `marker` replaced by
/// `side` by `side`.
fn huge_dimensions(data: &[u8], marker: &[u8], offset: usize, side: &[u8]) -> Vec<u8> {
    let mut huge = data.to_vec();
    let at = data
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap_or_else(|| panic!("fixture has marker {marker:?}"))
        + offset;
    huge[at..at + side.len()].copy_from_slice(side);
    huge[at + side.len()..at + 2 * side.len()].copy_from_slice(side);
    huge
}

/// Each image format's intact fixture and a variant whose header claims dimensions past the
/// decoding budget.
fn image_fixtures() -> Vec<(&'static str, Vec<u8>, Vec<u8>)> {
    let jpeg = include_bytes!("oracle/assets/image-pattern.jpg").to_vec();
    let gif = encoded(image::ImageFormat::Gif);
    let avif = include_bytes!("oracle/assets/image-pattern.avif").to_vec();
    let heic = include_bytes!("oracle/assets/image-pattern.heic").to_vec();
    let mut huge_gif = gif.clone();
    huge_gif[6..10].copy_from_slice(&[0xff; 4]);
    let huge_webp = [
        &b"RIFF"[..],
        &22_u32.to_le_bytes(),
        b"WEBPVP8X",
        &10_u32.to_le_bytes(),
        &[0; 4],
        &[0xff, 0x3f, 0x00, 0xff, 0x3f, 0x00],
    ]
    .concat();
    vec![
        (
            "png",
            include_bytes!("oracle/assets/image-pattern.png").to_vec(),
            huge_png(),
        ),
        (
            "jpg",
            jpeg.clone(),
            huge_dimensions(&jpeg, &[0xff, 0xc0], 5, &[0xff, 0xff]),
        ),
        ("gif", gif, huge_gif),
        ("webp", encoded(image::ImageFormat::WebP), huge_webp),
        (
            "avif",
            avif.clone(),
            huge_dimensions(&avif, b"ispe", 8, &[0x00, 0xff, 0xff, 0xff]),
        ),
        (
            "heic",
            heic.clone(),
            huge_dimensions(&heic, b"ispe", 8, &[0x00, 0xff, 0xff, 0xff]),
        ),
        (
            "svg",
            include_bytes!("oracle/assets/image-pattern.svg").to_vec(),
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="1e30" height="1e30"/>"#.to_vec(),
        ),
    ]
}

/// The image contexts of one document, each naming `reference`.
fn image_document(reference: &str) -> String {
    format!(
        "<!doctype html><style>{}</style><p>Before</p><img src='{reference}' alt='{ALT}'>\
         <div style=\"width:20px;height:20px;background:url('{reference}');border:4px solid;border-image:url('{reference}') 2\"></div>\
         <ul style=\"list-style-image:url('{reference}')\"><li>item</li></ul>\
         <p style=\"content:url('{reference}')\">replaced</p>\
         <div style=\"float:left;width:20px;height:20px;shape-outside:url('{reference}')\"></div>\
         <svg width='20' height='20'><image href='{reference}' width='10' height='10'/></svg>\
         <video poster='{reference}' width='20' height='20'></video>\
         <object data='{reference}' width='20' height='20'>{FALLBACK}</object><p>After</p>",
        common::PROFILE
    )
}

fn texts(output: &RenderOutput) -> Vec<String> {
    output
        .layout
        .as_ref()
        .expect("layout requested")
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.clone())
        .collect()
}

fn render(case: &str, html: &str, corpus: &Corpus) -> RenderOutput {
    let output = render_with_resolver(html, &common::local_options(), corpus)
        .unwrap_or_else(|error| panic!("{case}: rendering must not fail: {error}"));
    assert!(
        output.pdf.starts_with(b"%PDF-"),
        "{case}: a PDF is produced"
    );
    let texts = texts(&output);
    for label in ["Before", "After"] {
        assert!(
            texts.iter().any(|text| text.contains(label)),
            "{case}: lost {label:?}: {texts:?}"
        );
    }
    output
}

/// Whether `diagnostic` reports the image `reference` names, which some contexts resolve to
/// an absolute URL first.
fn image_reported(diagnostic: &RenderDiagnostic, reference: &str) -> bool {
    match diagnostic {
        RenderDiagnostic::UnavailableImage(failure) => failure.reference.ends_with(reference),
        RenderDiagnostic::UnavailableEmbedded {
            reference: embedded,
            ..
        } => embedded.ends_with(reference),
        RenderDiagnostic::ImageDecode { .. } => true,
        _ => false,
    }
}

#[test]
fn broken_and_damaged_images_of_every_format_render_their_fallbacks() {
    let mut seeded = Seeded(SEED);
    for (format, intact, huge) in image_fixtures() {
        let mut broken = vec![
            ("zero".to_owned(), Vec::new()),
            ("header".to_owned(), intact[..HEADER_BYTES].to_vec()),
        ];
        let mut damaged = vec![("truncated".to_owned(), intact[..intact.len() / 2].to_vec())];
        // A vector image has no decoding budget: its huge natural size is merely scaled. AVIF
        // and HEIF images decode at their coded size, not their `ispe` property's.
        if matches!(format, "svg" | "avif" | "heic") {
            damaged.push(("huge".to_owned(), huge));
        } else {
            broken.push(("huge".to_owned(), huge));
        }
        for variant in 0..GARBLED_VARIANTS {
            damaged.push((
                format!("garbled-{variant}"),
                seeded.garble(&intact, 0..intact.len(), garbled_count(intact.len())),
            ));
        }
        for (variant, bytes, must_report) in broken
            .into_iter()
            .map(|(variant, bytes)| (variant, bytes, true))
            .chain(
                damaged
                    .into_iter()
                    .map(|(variant, bytes)| (variant, bytes, false)),
            )
        {
            let reference = format!("{variant}.{format}");
            let case = format!("{reference} ({} bytes)", bytes.len());
            let mut corpus = Corpus::default();
            corpus.insert(&reference, bytes);
            let output = render(&case, &image_document(&reference), &corpus);
            for diagnostic in &output.diagnostics {
                assert!(
                    image_reported(diagnostic, &reference),
                    "{case}: unexpected diagnostic {diagnostic}"
                );
            }
            let shows_alt = texts(&output).iter().any(|text| text.contains(ALT));
            if must_report {
                assert!(
                    !output.diagnostics.is_empty(),
                    "{case}: the broken image is reported"
                );
                assert!(
                    shows_alt,
                    "{case}: the broken img represents its alt text (HTML §4.8.4.3.1)"
                );
            } else {
                let painted = output
                    .layout
                    .as_ref()
                    .expect("layout requested")
                    .display_lists
                    .iter()
                    .flatten()
                    .any(|item| matches!(item, DisplayItem::Image(_)));
                assert!(
                    shows_alt || painted || !output.diagnostics.is_empty(),
                    "{case}: a damaged image either decodes and paints, or is reported"
                );
            }
        }
    }
}

/// `image-pattern.avif` with one byte of its colour item's AV1 tile data flipped. The frame
/// fails to decode, which made rav1d 1.1.0 abort the process (`vendor/rav1d/PATCHES.md`).
#[test]
fn av1_frame_decode_failures_render_the_broken_image() {
    const REFERENCE: &str = "av1-frame-error.avif";
    let mut corpus = Corpus::default();
    corpus.insert(
        REFERENCE,
        include_bytes!("fixtures/images/av1-frame-error.avif").to_vec(),
    );
    let output = render(
        REFERENCE,
        &format!(
            "<!doctype html><style>{}</style><p>Before</p><img src='{REFERENCE}' alt='{ALT}'><p>After</p>",
            common::PROFILE
        ),
        &corpus,
    );
    assert!(
        matches!(
            output.diagnostics.as_slice(),
            [RenderDiagnostic::UnavailableImage(failure)]
                if failure.reference == REFERENCE
                    && matches!(failure.error, ImageError::AvifDecode(_))
        ),
        "the AV1 decode error is reported once: {:?}",
        output.diagnostics
    );
    assert!(
        texts(&output).iter().any(|text| text.contains(ALT)),
        "the broken img represents its alt text (HTML §4.8.4.3.1): {:?}",
        texts(&output)
    );
}

#[test]
fn missing_denied_and_malformed_references_are_reported_once_each() {
    let corpus = Corpus::default();
    for reference in [
        "missing.png",
        "denied.png",
        "data:image/png;base64,!!",
        "unknown-scheme:image",
    ] {
        let output = render(reference, &image_document(reference), &corpus);
        let reported = output
            .diagnostics
            .iter()
            .filter(|diagnostic| image_reported(diagnostic, reference))
            .count();
        assert!(
            reported >= 1 && reported == output.diagnostics.len(),
            "{reference}: only the unavailable resource is reported: {:?}",
            output.diagnostics
        );
        let texts = texts(&output);
        for fallback in [ALT, FALLBACK] {
            assert!(
                texts.iter().any(|text| text.contains(fallback)),
                "{reference}: {fallback} content is rendered: {texts:?}"
            );
        }
    }
}

fn font_fixtures() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "variable",
            include_bytes!("fixtures/fonts/FontStyleTest.ttf").to_vec(),
        ),
        (
            "color",
            include_bytes!("fixtures/fonts/PaletteTest.ttf").to_vec(),
        ),
        (
            "cff",
            include_bytes!("fixtures/fonts/CJKSpacingTest.otf").to_vec(),
        ),
    ]
}

const TABLE_DIRECTORY: usize = 12;
const TABLE_RECORD: usize = 16;

fn table_count(font: &[u8]) -> usize {
    usize::from(u16::from_be_bytes([font[4], font[5]]))
}

/// The bytes of `font`'s table `tag`.
fn table(font: &[u8], tag: [u8; 4]) -> std::ops::Range<usize> {
    let field = |at: usize| {
        usize::try_from(u32::from_be_bytes(
            font[at..at + 4].try_into().expect("four bytes"),
        ))
        .expect("table field fits")
    };
    let record = (0..table_count(font))
        .map(|index| TABLE_DIRECTORY + index * TABLE_RECORD)
        .find(|record| font[*record..*record + 4] == tag)
        .unwrap_or_else(|| panic!("fixture has table {tag:?}"));
    let offset = field(record + 8);
    offset..offset + field(record + 12)
}

/// `font` with its glyph count set to one, so that its character map names glyphs it lacks.
fn one_glyph(font: &[u8]) -> Vec<u8> {
    const GLYPH_COUNT_OFFSET: usize = 4;
    let at = table(font, *b"maxp").start + GLYPH_COUNT_OFFSET;
    let mut shrunk = font.to_vec();
    shrunk[at..at + 2].copy_from_slice(&1_u16.to_be_bytes());
    shrunk
}

fn font_document(reference: &str) -> String {
    format!(
        "<!doctype html><style>{}@font-face{{font-family:Probe;src:url('{reference}')}}\
         p{{font-family:Probe,serif}}</style><p>Before Hello wörld ffi 123 <i>italic</i> <b>bold</b></p>\
         <p style='font-palette:dark;font-variant:small-caps;text-shadow:1px 1px red'>After 漢字</p>\
         <svg width='100' height='30'><text y='20' font-family='Probe'>svg text</text></svg>",
        common::PROFILE
    )
}

fn font_failures(output: &RenderOutput) -> Vec<(String, String)> {
    output
        .diagnostics
        .iter()
        .map(|diagnostic| match diagnostic {
            RenderDiagnostic::UnavailableFont(failure) => {
                (failure.reference.clone(), failure.error.to_string())
            }
            diagnostic => panic!("unexpected diagnostic {diagnostic}"),
        })
        .collect()
}

#[test]
fn broken_and_damaged_fonts_fall_back_and_report_each_failure() {
    let mut seeded = Seeded(SEED);
    const GARBAGE_BYTES: usize = 4096;
    let mut broken = vec![
        ("zero.ttf".to_owned(), Vec::new()),
        ("garbage.ttf".to_owned(), seeded.bytes(GARBAGE_BYTES)),
        (
            "image.ttf".to_owned(),
            include_bytes!("oracle/assets/image-pattern.png").to_vec(),
        ),
    ];
    let mut damaged = Vec::new();
    for (name, font) in font_fixtures() {
        broken.push((format!("header-{name}.ttf"), font[..HEADER_BYTES].to_vec()));
        damaged.push((
            format!("truncated-{name}.ttf"),
            font[..font.len() / 3].to_vec(),
        ));
        damaged.push((format!("one-glyph-{name}.ttf"), one_glyph(&font)));
        let directory = TABLE_DIRECTORY + TABLE_RECORD * table_count(&font);
        for variant in 0..GARBLED_VARIANTS {
            damaged.push((
                format!("garbled-{variant}-{name}.ttf"),
                seeded.garble(&font, directory..font.len(), garbled_count(font.len())),
            ));
        }
    }
    let variable = &font_fixtures()[0].1;
    let directory = TABLE_DIRECTORY + TABLE_RECORD * table_count(variable);
    let sparse: Vec<_> = (0..GARBLED_VARIANTS)
        .map(|_| seeded.garble(variable, directory..variable.len(), SPARSE_GARBLED_BYTES))
        .collect();
    for (reference, bytes) in broken {
        let mut corpus = Corpus::default();
        corpus.insert(&reference, bytes);
        let output = render(&reference, &font_document(&reference), &corpus);
        let failures = font_failures(&output);
        assert!(
            matches!(failures.as_slice(), [(skipped, _)] if *skipped == reference),
            "{reference}: the invalid source is skipped and reported once: {failures:?}"
        );
    }
    for reference in ["missing.ttf", "denied.ttf", "data:font/ttf;base64,!!"] {
        let output = render(reference, &font_document(reference), &Corpus::default());
        let failures = font_failures(&output);
        assert!(
            matches!(failures.as_slice(), [(skipped, _)] if skipped == reference),
            "{reference}: the unavailable source is reported once: {failures:?}"
        );
    }
    for (reference, bytes) in damaged {
        let mut corpus = Corpus::default();
        corpus.insert(&reference, bytes);
        let output = render(&reference, &font_document(&reference), &corpus);
        for (skipped, error) in font_failures(&output) {
            assert_eq!(skipped, reference, "{reference}: {error}");
        }
    }
    // Sparse damage can leave the default instance embeddable but not the italic one, whose
    // text then keeps the default instance.
    let mut instances = 0;
    for (variant, bytes) in sparse.into_iter().enumerate() {
        let reference = format!("sparse-{variant}.ttf");
        let mut corpus = Corpus::default();
        corpus.insert(&reference, bytes);
        let output = render(&reference, &font_document(&reference), &corpus);
        let failures = font_failures(&output);
        if !failures
            .iter()
            .any(|(_, error)| error.starts_with(INSTANCE_FAILURE))
        {
            continue;
        }
        instances += 1;
        let trace = output.layout.expect("layout requested");
        let italic = trace
            .pages
            .iter()
            .flatten()
            .flat_map(|line| &line.runs)
            .find(|run| run.text.contains("italic"))
            .expect("the italic run renders");
        assert_eq!(
            trace.fonts[italic.font_id.index()].family,
            "Probe",
            "{reference}: the face stays in use"
        );
        assert!(
            italic.variations.is_empty(),
            "{reference}: the italic run is drawn at the default instance: {:?}",
            italic.variations
        );
    }
    assert!(
        instances > 0,
        "the seeded sparse damage exercises an unembeddable instance"
    );
}

#[test]
fn unavailable_style_sheets_and_nested_documents_are_skipped() {
    let mut seeded = Seeded(SEED);
    let mut corpus = Corpus::default();
    corpus.insert("binary.css", seeded.bytes(512));
    corpus.insert(
        "image.css",
        include_bytes!("oracle/assets/image-pattern.png").to_vec(),
    );
    corpus.insert("binary.html", seeded.bytes(512));
    corpus.insert(
        "nested.html",
        b"<!doctype html><link rel=stylesheet href=missing-nested.css><p>nested<img src=missing-nested.png alt=x></p><iframe src=denied-nested.html></iframe>".to_vec(),
    );
    for (reference, unavailable) in [
        ("missing.css", true),
        ("denied.css", true),
        ("data:text/css;base64,!!", true),
        ("unknown-scheme:sheet", true),
        ("binary.css", false),
        ("image.css", false),
    ] {
        for (kind, html) in [
            ("link", format!("<link rel=stylesheet href='{reference}'>")),
            (
                "import",
                format!("<style>@import url('{reference}');</style>"),
            ),
        ] {
            let case = format!("{kind} {reference}");
            let output = render(
                &case,
                &format!(
                    "<!doctype html>{html}<style>{}p{{font-size:{SHEET_FONT_SIZE_PT}pt}}</style><p>Before</p><p>After</p>",
                    common::PROFILE
                ),
                &corpus,
            );
            let sources: Vec<_> = output
                .diagnostics
                .iter()
                .filter_map(|diagnostic| match diagnostic {
                    RenderDiagnostic::UnavailableSource { reference, .. } => {
                        Some(reference.as_str())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                sources,
                if unavailable { vec![reference] } else { vec![] },
                "{case}: {:?}",
                output.diagnostics
            );
            common::assert_labelled_pt(
                &format!("{case}: the document's own rules still apply"),
                output.layout.unwrap().pages[0][0].font_size,
                SHEET_FONT_SIZE_PT,
            );
        }
    }
    for reference in [
        "missing.html",
        "denied.html",
        "binary.html",
        "nested.html",
        "data:text/html;base64,!!",
    ] {
        let html = format!(
            "<!doctype html><style>{}</style><p>Before</p><iframe src='{reference}' width=100 height=100></iframe>\
             <object data='{reference}' width=100 height=100>{FALLBACK}</object>\
             <embed src='{reference}' type=image/svg+xml width=20 height=20><p>After</p>",
            common::PROFILE
        );
        let output = render(reference, &html, &corpus);
        for diagnostic in &output.diagnostics {
            assert!(
                matches!(
                    diagnostic,
                    RenderDiagnostic::UnavailableEmbedded { .. }
                        | RenderDiagnostic::UnavailableSource { .. }
                        | RenderDiagnostic::UnavailableImage(_)
                        | RenderDiagnostic::Decoding { .. }
                ),
                "{reference}: unexpected diagnostic {diagnostic}"
            );
        }
        if reference.starts_with(MISSING) || reference.starts_with(DENIED) {
            assert!(
                output.diagnostics.iter().any(|diagnostic| matches!(
                    diagnostic,
                    RenderDiagnostic::UnavailableEmbedded { reference: embedded, .. } if embedded.ends_with(reference)
                )),
                "{reference}: the unavailable navigable is reported: {:?}",
                output.diagnostics
            );
            assert!(
                texts(&output).iter().any(|text| text.contains(FALLBACK)),
                "{reference}: the object shows its fallback content"
            );
        }
        if reference == "nested.html" {
            let nested: Vec<_> = output.diagnostics.iter().map(ToString::to_string).collect();
            for failed in [
                "missing-nested.css",
                "missing-nested.png",
                "denied-nested.html",
            ] {
                assert!(
                    nested.iter().any(|diagnostic| diagnostic.contains(failed)),
                    "the nested document reports {failed}: {nested:?}"
                );
            }
        }
    }
}

#[test]
fn unusable_base_urls_leave_the_document_base() {
    const IMAGE: &str = "pattern.png";
    let mut corpus = Corpus::default();
    corpus.insert(
        IMAGE,
        include_bytes!("oracle/assets/image-pattern.png").to_vec(),
    );
    for href in ["http://[unparsable", "data:,base", "javascript:void(0)"] {
        let output = render(
            href,
            &format!(
                "<!doctype html><base href='{href}'><style>{}</style><p>Before</p><img src='{IMAGE}' alt='{ALT}'><p>After</p>",
                common::PROFILE
            ),
            &corpus,
        );
        assert!(
            output.diagnostics.is_empty(),
            "{href}: the image resolves against the document base: {:?}",
            output.diagnostics
        );
        assert!(
            !texts(&output).iter().any(|text| text.contains(ALT)),
            "{href}: the image loads rather than showing its alt text"
        );
    }
}
