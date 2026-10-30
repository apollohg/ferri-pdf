use super::*;
use crate::style::logical::TextOrientation;

const FONT_SIZE: f32 = 24.0;

#[test]
fn svg_vertical_substitutions_origins_and_advances_match_independent_harfbuzz_samples() {
    let mut database = Database::new();
    database.load_font_data(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/fonts/CJKWritingTest.otf"
        ))
        .to_vec(),
    );
    let id = database.faces().next().expect("pinned fixture face").id;
    let font = Arc::new(database.load_font(id).expect("fixture metrics"));
    let reference: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fonts/CJKWritingTest-source.json"
    )))
    .unwrap();
    for (orientation, text) in [
        (TextOrientation::Mixed, "漢字水「」（）〔〕、。"),
        (TextOrientation::Upright, "漢A0123「」"),
    ] {
        for character in text.chars() {
            let text = character.to_string();
            let orientations = [(0..text.len(), orientation)];
            let glyphs = shape_text_with_font(
                &text,
                &text,
                0..text.len(),
                false,
                Some(&orientations),
                font.clone(),
                SpanShaping {
                    request: &Font {
                        families: Vec::new(),
                        default_family: true,
                        style: crate::svg::usvg::FontStyle::Normal,
                        slant: crate::fonts::FontSlant::NORMAL,
                        width: crate::fonts::FontWidth::NORMAL,
                        weight: crate::fonts::FontWeight::NORMAL,
                        variations: Vec::new(),
                        explicit_variations: Vec::new(),
                    },
                    style: &crate::style::Style::default(),
                    font_size: FONT_SIZE,
                    optical_sizing: crate::svg::usvg::FontOpticalSizing::Auto,
                    tracked: false,
                },
                &crate::svg::usvg::FaceSettings::default(),
                &[],
                &database,
            )
            .expect("vertical shape");
            assert_eq!(glyphs.len(), 1, "{orientation:?}/{character}");
            let actual = &glyphs[0];
            let expected = &reference["samples"]["ttb"][&text][0];
            assert!(
                actual.vertical,
                "{orientation:?}/{character}: vertical glyph geometry"
            );
            for (label, actual, expected) in [
                (
                    "glyph",
                    i64::from(actual.id.0),
                    expected["g"].as_i64().unwrap(),
                ),
                (
                    "horizontal origin",
                    i64::from(actual.dx),
                    expected["dx"].as_i64().unwrap(),
                ),
                (
                    "vertical origin",
                    i64::from(actual.dy),
                    expected["dy"].as_i64().unwrap(),
                ),
                (
                    "inline advance",
                    i64::from(actual.width),
                    -expected["ay"].as_i64().unwrap(),
                ),
            ] {
                assert_eq!(actual, expected, "{orientation:?}/{character}: {label}");
            }
            let cluster = form_glyph_clusters(&glyphs, &text, FONT_SIZE);
            let physical = Transform::from_rotate(90.0).pre_concat(cluster.glyphs[0].transform());
            let scale = font.scale(FONT_SIZE);
            let tolerance = f32::EPSILON * FONT_SIZE;
            for (label, actual, expected) in [
                ("upright scale x", physical.sx, scale),
                ("upright scale y", physical.sy, scale),
                ("rotation x", physical.kx, 0.0),
                ("rotation y", physical.ky, 0.0),
                (
                    "physical x origin",
                    physical.tx,
                    glyphs[0].dx as f32 * scale,
                ),
                (
                    "physical y origin",
                    physical.ty,
                    -glyphs[0].dy as f32 * scale,
                ),
            ] {
                assert!(
                    (actual - expected).abs() <= tolerance,
                    "{orientation:?}/{character}: {label}: {actual} != {expected}"
                );
            }
        }
    }
}
