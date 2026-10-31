//! `@font-face` sources in WOFF 1.0, WOFF 2.0 and collections render with the face they
//! package (CSS Fonts 4 §4.3), which the `format()` hint admits; descriptors apply to it.
use ferri_pdf::{LayoutTrace, RenderOptions, render_html};
use std::path::Path;

const SIZE_PT: f32 = 20.0;
const TEXT: &str = "Hamburgefonstiv";
const FAMILY: &str = "Web";
const REGULAR: &str = "WoffTest-Regular.ttf";
const BOLD: &str = "WoffTest-Bold.ttf";
/// Shaped advances are rounded to whole font units.
const ADVANCE_TOLERANCE_PT: f32 = 0.01;

fn fixtures() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fonts")
}

fn render(case: &str, face: &str) -> LayoutTrace {
    let html = format!(
        "<style>@font-face{{font-family:{FAMILY};{face}}}@page{{size:400pt 200pt;margin:20pt}}\
         p{{margin:0;font-family:{FAMILY},serif;font-size:{SIZE_PT}pt;font-kerning:none}}</style><p>{TEXT}</p>"
    );
    let options = RenderOptions {
        base: fixtures().into(),
        include_layout: true,
        ..Default::default()
    };
    let output = render_html(&html, &options).unwrap_or_else(|error| panic!("{case}: {error}"));
    assert!(
        output.diagnostics.is_empty(),
        "{case}: the source loads without diagnostics: {:?}",
        output
            .diagnostics
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
    output.layout.expect("layout requested")
}

/// Checks that the line's glyphs are `reference`'s, with its advances at `size` points.
fn assert_face(case: &str, trace: &LayoutTrace, reference: &str, size: f32) {
    let data = std::fs::read(fixtures().join(reference)).expect("reference font");
    let face = ttf_parser::Face::parse(&data, 0).expect("reference font parses");
    let line = &trace.pages[0][0];
    assert_eq!(line.text, TEXT, "{case}");
    let [run] = line.runs.as_slice() else {
        panic!("{case}: one run in the web font: {:?}", line.runs);
    };
    assert_eq!(trace.fonts[run.font_id.index()].family, FAMILY, "{case}");
    let expected: Vec<_> = TEXT
        .chars()
        .map(|character| face.glyph_index(character).expect("mapped").0)
        .collect();
    let glyphs: Vec<_> = run.glyphs().iter().map(|glyph| glyph.id).collect();
    assert_eq!(
        glyphs,
        expected.iter().map(|id| u32::from(*id)).collect::<Vec<_>>(),
        "{case}: glyph IDs are {reference}'s"
    );
    let scale = size / f32::from(face.units_per_em());
    for (glyph, id) in run.glyphs().iter().zip(expected) {
        let advance = f32::from(face.glyph_hor_advance(ttf_parser::GlyphId(id)).unwrap()) * scale;
        assert!(
            (glyph.advance(run.font_size) - advance).abs() <= ADVANCE_TOLERANCE_PT,
            "{case}: glyph {id} advances {} pt, {reference} gives {advance} pt",
            glyph.advance(run.font_size)
        );
    }
}

#[test]
fn woff_woff2_and_collection_sources_render_their_packaged_face() {
    for (source, reference) in [
        ("url(WoffTest-Regular.woff) format('woff')", REGULAR),
        ("url(WoffTest-Regular.woff2) format('woff2')", REGULAR),
        (
            "url(WoffTest-Regular-NullGlyf.woff2) format(woff2)",
            REGULAR,
        ),
        ("url(WoffTest-Collection.woff2) format('woff2')", REGULAR),
        ("url(WoffTest-Collection.woff2#WoffTest-Bold)", BOLD),
        ("url(WoffTest.ttc#WoffTest-Bold) format(collection)", BOLD),
        (
            "url(WoffTest-Regular.ttf) format('embedded-opentype'),url(WoffTest-Collection.woff2#WoffTest-Bold) format('woff2')",
            BOLD,
        ),
    ] {
        let trace = render(source, &format!("src:{source}"));
        assert_face(source, &trace, reference, SIZE_PT);
    }
}

#[test]
fn face_descriptors_apply_to_decoded_woff2_faces() {
    const SIZE_ADJUST: f32 = 0.5;
    let face = "src:url(WoffTest-Regular.woff2) format('woff2');size-adjust:50%";
    assert_face(face, &render(face, face), REGULAR, SIZE_PT * SIZE_ADJUST);
    let face = "src:url(WoffTest-Regular.woff2) format('woff2');unicode-range:U+61-7A";
    let trace = render(face, face);
    let runs = &trace.pages[0][0].runs;
    let families: Vec<_> = runs
        .iter()
        .map(|run| {
            (
                run.text.as_str(),
                trace.fonts[run.font_id.index()].family.as_str(),
            )
        })
        .collect();
    assert_eq!(
        families.first(),
        Some(&("H", "Liberation Serif")),
        "{face}: H lies outside the range, so the next family draws it: {families:?}"
    );
    assert_eq!(
        families.get(1),
        Some(&(&TEXT[1..], FAMILY)),
        "{face}: lowercase letters use the decoded face: {families:?}"
    );
}
