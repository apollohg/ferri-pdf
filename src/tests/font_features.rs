//! Font feature, variation and palette properties checked against independent `HarfBuzz` and
//! `FontTools` measurements of the fixture fonts.
use super::common;
use crate::layout::{LayoutTrace, TextLine};
use crate::paint::Color;

const SIZE_PT: f32 = 20.0;
const LINE_HEIGHT_PT: f32 = 30.0;
const FACES: &str = "@font-face{font-family:Serif4;src:url(SourceSerif4-Regular.ttf)}@font-face{font-family:Flex;src:url(RobotoFlex.ttf)}@font-face{font-family:Presentation;src:url(PresentationTest.ttf)}@font-face{font-family:Palette;src:url(PaletteTest.ttf)}";
const ADVANCE_TOLERANCE_EM: f32 = 1e-6;
/// The shaper reports whole font units; `FontTools` keeps fractional variation advances.
const VARIABLE_UNIT_TOLERANCE: f32 = 1.0;

fn reference() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/FontFeatures-reference.json"
    ))
    .unwrap()
}

/// Glyph IDs and advances, in em, that `HarfBuzz` shaped for a recorded sample.
fn shaped(name: &str) -> (Vec<u32>, Vec<f32>) {
    let reference = reference();
    let sample = &reference["samples"][name];
    let upem = reference["fonts"][sample["font"].as_str().unwrap()]["units_per_em"]
        .as_f64()
        .unwrap() as f32;
    sample["glyphs"]
        .as_array()
        .unwrap_or_else(|| panic!("reference sample {name}"))
        .iter()
        .map(|glyph| {
            (
                glyph["g"].as_u64().unwrap() as u32,
                glyph["ax"].as_f64().unwrap() as f32 / upem,
            )
        })
        .unzip()
}

fn options() -> crate::RenderOptions {
    let mut options = common::local_options();
    options.base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fonts")
        .into();
    options
}

fn render(rules: &str, body: &str) -> Result<LayoutTrace, crate::Error> {
    let html = format!(
        "<style>{FACES}@page{{size:400pt 400pt;margin:20pt}}p{{margin:0;font-size:{SIZE_PT}pt;line-height:{LINE_HEIGHT_PT}pt}}{rules}</style>{body}"
    );
    crate::render_html(&html, &options()).map(|output| output.layout.unwrap())
}

fn line(rules: &str, style: &str, text: &str) -> TextLine {
    render(rules, &format!("<p style=\"{style}\">{text}</p>"))
        .unwrap_or_else(|error| panic!("{rules} {style} {text:?}: {error}"))
        .pages
        .remove(0)
        .remove(0)
}

fn glyph_ids(line: &TextLine) -> Vec<u32> {
    line.runs
        .iter()
        .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.id))
        .collect()
}

fn advances(line: &TextLine) -> Vec<f32> {
    line.runs
        .iter()
        .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.x_advance))
        .collect()
}

fn assert_shaped(line: &TextLine, sample: &str, label: &str) {
    let (expected_ids, expected_advances) = shaped(sample);
    assert_eq!(glyph_ids(line), expected_ids, "{label}: glyphs of {sample}");
    for (index, (actual, expected)) in advances(line).iter().zip(&expected_advances).enumerate() {
        assert!(
            (actual - expected).abs() <= ADVANCE_TOLERANCE_EM,
            "{label}: advance {index} of {sample}: expected {expected} em, got {actual} em"
        );
    }
    assert!(
        line.runs.iter().all(|run| run.font_size == SIZE_PT),
        "{label}: native features keep the font size"
    );
    common::assert_labelled_pt(
        label,
        line.width,
        expected_advances.iter().sum::<f32>() * SIZE_PT,
    );
}

#[test]
fn numeric_values_select_the_independently_shaped_figures() {
    for (declarations, text, sample) in [
        ("", "1011", "default_figures"),
        (
            "font-variant-numeric:tabular-nums",
            "1011",
            "tabular_figures",
        ),
        (
            "font-variant-numeric:proportional-nums",
            "1011",
            "proportional_figures",
        ),
        ("font-variant:oldstyle-nums", "1011", "oldstyle_figures"),
        (
            "font-variant-numeric:oldstyle-nums proportional-nums",
            "1011",
            "oldstyle_proportional_figures",
        ),
        ("font-variant-numeric:slashed-zero", "0", "slashed_zero"),
        (
            "font-variant-numeric:diagonal-fractions",
            "1/2",
            "diagonal_fractions",
        ),
        ("font-variant-numeric:ordinal", "2a", "ordinals"),
        (
            "font-feature-settings:'pnum'",
            "1011",
            "proportional_figures",
        ),
        (
            "font-variant-numeric:proportional-nums;font-feature-settings:'pnum' 0",
            "1011",
            "default_figures",
        ),
        (
            "font-variant-numeric:proportional-nums;font:20pt/30pt Serif4",
            "1011",
            "default_figures",
        ),
    ] {
        let line = line("", &format!("font-family:Serif4;{declarations}"), text);
        assert_shaped(&line, sample, declarations);
    }
    let (_, tabular) = shaped("tabular_figures");
    let (_, proportional) = shaped("proportional_figures");
    assert!(
        tabular.windows(2).all(|pair| pair[0] == pair[1]) && proportional[0] != proportional[1],
        "the reference must distinguish tabular from proportional figures"
    );
}

#[test]
fn ligature_values_and_tracking_select_the_shaped_ligatures() {
    for (declarations, text, sample) in [
        ("", "fi", "common_ligature"),
        (
            "font-variant-ligatures:common-ligatures",
            "fi",
            "common_ligature",
        ),
        (
            "font-variant-ligatures:no-common-ligatures",
            "fi",
            "no_common_ligature",
        ),
        ("font-variant:none", "fi", "no_common_ligature"),
        (
            "font-variant-ligatures:discretionary-ligatures",
            "st",
            "discretionary_ligature",
        ),
        (
            "font-variant-ligatures:historical-ligatures",
            "\u{17f}t",
            "historical_ligature",
        ),
    ] {
        let line = line(
            "",
            &format!("font-family:'DejaVu Sans';{declarations}"),
            text,
        );
        assert_shaped(&line, sample, declarations);
    }
    let (ligature, _) = shaped("common_ligature");
    let (separate, _) = shaped("no_common_ligature");
    for declarations in [
        "letter-spacing:2pt",
        "letter-spacing:2pt;font-variant-ligatures:common-ligatures",
    ] {
        let tracked = line(
            "",
            &format!("font-family:'DejaVu Sans';{declarations}"),
            "fi",
        );
        assert_eq!(
            glyph_ids(&tracked),
            separate,
            "tracking suppresses common ligatures after font-variant (§7.2): {declarations}"
        );
    }
    let tracked = line(
        "",
        "font-family:'DejaVu Sans';letter-spacing:2pt;font-feature-settings:'liga'",
        "fi",
    );
    assert_eq!(
        glyph_ids(&tracked),
        ligature,
        "font-feature-settings applies after tracking"
    );
}

#[test]
fn alternates_use_the_names_the_rendering_family_defines() {
    let rules = "@font-feature-values DejaVu Sans{@stylistic{alt:1}}@font-feature-values Other{@stylistic{other:1}}";
    for (declarations, sample) in [
        (
            "font-variant-alternates:stylistic(alt)",
            "stylistic_alternate",
        ),
        ("font-variant:stylistic(alt)", "stylistic_alternate"),
        (
            "font-variant-alternates:stylistic(other)",
            "default_alternate",
        ),
        (
            "font-variant-alternates:stylistic(ALT)",
            "default_alternate",
        ),
    ] {
        let style = format!("font-family:'DejaVu Sans';{declarations}");
        let line = line(rules, &style, "a");
        assert_shaped(&line, sample, declarations);
    }
}

#[test]
fn stylesets_combine_names_and_later_definitions_replace_earlier_ones() {
    let (plain, _) = shaped("default_cyrillic");
    let (one, _) = shaped("styleset_one");
    let (two, _) = shaped("styleset_two");
    let both: Vec<u32> = vec![one[0], two[0]];
    for (rules, value, expected) in [
        (
            "@font-feature-values Serif4{@styleset{one:1;two:2}}",
            "styleset(one)",
            vec![one[0], plain[1]],
        ),
        (
            "@font-feature-values Serif4{@styleset{one:1;two:2}}",
            "styleset(two)",
            vec![plain[0], two[0]],
        ),
        (
            "@font-feature-values Serif4{@styleset{one:1;two:2}}",
            "styleset(one, two)",
            both.clone(),
        ),
        (
            "@font-feature-values Serif4{@styleset{pair:1 2}}",
            "styleset(pair)",
            both.clone(),
        ),
        (
            "@font-feature-values Serif4{@styleset{one:1}}@font-feature-values Other, Serif4{@styleset{one:2}}",
            "styleset(one)",
            vec![plain[0], two[0]],
        ),
        (
            "@font-feature-values Serif4{@styleset{one:1}@styleset{two:2}}",
            "styleset(one, two)",
            both.clone(),
        ),
        (
            "@font-feature-values Serif4{@styleset{bad:-1;worse:1.5;one:1}annotation{two:2}}",
            "styleset(bad, worse, one, two)",
            vec![one[0], plain[1]],
        ),
        (
            "@font-feature-values Serif4{@swash{one:1 2}}",
            "swash(one)",
            plain.clone(),
        ),
        (
            "@font-feature-values Serif4{@styleset{zero:0;large:100}}",
            "styleset(zero, large)",
            plain.clone(),
        ),
        (
            "@font-feature-values serif, Serif4{@styleset{one:1}}",
            "styleset(one)",
            plain.clone(),
        ),
    ] {
        let line = line(
            rules,
            &format!("font-family:Serif4;font-variant-alternates:{value}"),
            "\u{432}\u{431}",
        );
        assert_eq!(glyph_ids(&line), expected, "{rules} with {value}");
    }
}

#[test]
fn kerning_follows_font_kerning_and_feature_precedence() {
    let (kerned_ids, kerned) = shaped("kerned");
    let (unkerned_ids, unkerned) = shaped("unkerned");
    assert_eq!(kerned_ids, unkerned_ids);
    assert!(kerned[0] < unkerned[0], "the reference pair must kern");
    let rules = "@font-face{font-family:Unkerned;src:local('Liberation Serif');font-feature-settings:'kern' 0}";
    for (declarations, sample) in [
        ("font-family:'Liberation Serif'", "kerned"),
        (
            "font-family:'Liberation Serif';font-kerning:normal",
            "kerned",
        ),
        (
            "font-family:'Liberation Serif';font-kerning:none",
            "unkerned",
        ),
        (
            "font-family:'Liberation Serif';font-feature-settings:'kern' 0",
            "unkerned",
        ),
        (
            "font-family:'Liberation Serif';font-kerning:none;font-feature-settings:'kern'",
            "kerned",
        ),
        ("font-family:Unkerned", "unkerned"),
        ("font-family:Unkerned;font-kerning:normal", "kerned"),
        ("font-family:Unkerned;font-kerning:auto", "unkerned"),
    ] {
        assert_shaped(&line(rules, declarations, "AVA"), sample, declarations);
    }
    let pair = line("", "font-family:'Liberation Serif'", "AVA").width;
    let apart = line(
        "",
        "font-family:'Liberation Serif';font-kerning:none",
        "AVA",
    )
    .width;
    common::assert_pt(
        apart - pair,
        (unkerned[0] - kerned[0] + unkerned[1] - kerned[1]) * SIZE_PT,
    );
}

#[test]
fn caps_values_use_native_features_their_fallbacks_or_synthesis() {
    for (declarations, sample) in [
        ("font-variant-caps:all-small-caps", "all_small_caps"),
        ("font-variant-caps:petite-caps", "petite_caps_fallback"),
        ("font-variant-caps:all-petite-caps", "all_small_caps"),
        (
            "font-variant:all-small-caps;font-synthesis:none",
            "all_small_caps",
        ),
    ] {
        assert_shaped(
            &line("", &format!("font-family:Serif4;{declarations}"), "Ag"),
            sample,
            declarations,
        );
    }
    let plain = line("", "font-family:Serif4", "Ag");
    let titling = line(
        "",
        "font-family:Serif4;font-variant-caps:titling-caps",
        "Ag",
    );
    assert_eq!(
        glyph_ids(&titling),
        glyph_ids(&plain),
        "titling caps without titl change nothing"
    );
    common::assert_pt(titling.width, plain.width);
    let unicase = line("", "font-family:Serif4;font-variant-caps:unicase", "Ag");
    let sizes: Vec<_> = unicase
        .runs
        .iter()
        .map(|run| (run.text.as_str(), run.font_size))
        .collect();
    assert_eq!(
        sizes,
        [("A", 16.0), ("g", SIZE_PT)],
        "unicase without unic synthesizes small capitals from uppercase letters only"
    );
    assert_eq!(glyph_ids(&unicase), glyph_ids(&plain));
    let synthesized = line(
        "",
        "font-family:'Liberation Serif';font-variant-caps:all-small-caps",
        "Ag",
    );
    let capitals = line("", "font-family:'Liberation Serif';font-size:16pt", "AG");
    assert_eq!(synthesized.text, "Ag");
    assert_eq!(
        synthesized
            .runs
            .iter()
            .map(|run| run.font_size)
            .collect::<Vec<_>>(),
        [16.0],
        "all-small-caps synthesizes both cases as one run"
    );
    assert_eq!(glyph_ids(&synthesized), glyph_ids(&capitals));
    common::assert_pt(synthesized.width, capitals.width);
    let forbidden = line(
        "",
        "font-family:'Liberation Serif';font-variant-caps:all-small-caps;font-synthesis-small-caps:none",
        "Ag",
    );
    let original = line("", "font-family:'Liberation Serif'", "Ag");
    assert_eq!(glyph_ids(&forbidden), glyph_ids(&original));
    common::assert_pt(forbidden.width, original.width);
    let caseless = line(
        "",
        "font-family:'Liberation Serif';font-variant-caps:all-small-caps",
        "A1",
    );
    assert_eq!(
        caseless
            .runs
            .iter()
            .map(|run| (run.text.as_str(), run.font_size))
            .collect::<Vec<_>>(),
        [("A", 16.0), ("1", SIZE_PT)],
        "caseless characters keep their size"
    );
}

#[test]
fn language_override_selects_a_language_system_the_face_defines() {
    for (language, declarations, sample) in [
        ("en", "", "small_caps_english"),
        ("en", "font-language-override:'TRK'", "small_caps_turkish"),
        ("en", "font-language-override:'TRK '", "small_caps_turkish"),
        ("tr", "font-language-override:normal", "small_caps_turkish"),
        ("tr", "font-language-override:'XYZ'", "small_caps_turkish"),
        ("en", "font-language-override:'trk'", "small_caps_english"),
        (
            "en",
            "font-language-override:'TURKISH'",
            "small_caps_english",
        ),
    ] {
        let trace = render(
            "",
            &format!(
                "<p lang={language} style=\"font-family:Serif4;font-variant-caps:small-caps;{declarations}\">i</p>"
            ),
        )
        .unwrap();
        assert_shaped(
            &trace.pages[0][0],
            sample,
            &format!("lang={language} {declarations}"),
        );
    }
}

#[test]
fn position_variants_are_native_only_when_the_whole_run_has_them() {
    let liberation = "font-family:'Liberation Serif'";
    assert_shaped(
        &line(
            "",
            &format!("{liberation};font-variant-position:super"),
            "45",
        ),
        "native_superscript",
        "native superscripts",
    );
    assert_shaped(
        &line("", "font-family:Serif4;font-variant:super", "2a"),
        "superscripts",
        "native superscript letters",
    );
    let (plain, plain_advances) = shaped("plain_superscript_candidates");
    for (position, rises) in [("super", true), ("sub", false)] {
        let line = line(
            "",
            &format!("{liberation};font-variant-position:{position}"),
            "4x",
        );
        assert_eq!(line.runs.len(), 1, "{position}: one run, never a mixture");
        let run = &line.runs[0];
        assert_eq!(
            glyph_ids(&line),
            plain,
            "{position}: a partial variant set synthesizes from the default glyphs"
        );
        assert!(
            run.font_size < SIZE_PT && run.font_size > SIZE_PT * 2.0 / 3.0,
            "{position}: synthesized glyphs are reduced, got {} pt",
            run.font_size
        );
        assert!(
            run.synthesis.position,
            "{position}: the run records synthesis"
        );
        let offsets: Vec<f32> = run
            .glyphs()
            .iter()
            .map(|glyph| glyph.y_offset * run.font_size)
            .collect();
        assert!(
            offsets
                .windows(2)
                .all(|pair| (pair[0] - pair[1]).abs() < ADVANCE_TOLERANCE_EM),
            "{position}: every glyph shares one baseline offset: {offsets:?}"
        );
        assert_eq!(
            offsets[0] < 0.0,
            rises,
            "{position}: offset {offsets:?} (y grows downwards)"
        );
        common::assert_pt(
            line.width,
            plain_advances.iter().sum::<f32>() * run.font_size,
        );
        common::assert_pt(line.height, LINE_HEIGHT_PT);
    }
    let partial = line(
        "",
        &format!("{liberation};font-variant-position:super;font-synthesis-position:none"),
        "4x",
    );
    let (native, _) = shaped("native_superscript");
    assert_eq!(
        glyph_ids(&partial),
        [native[0], plain[1]],
        "without synthesis the font's own variants apply where they exist"
    );
    assert!(partial.runs.iter().all(|run| run.font_size == SIZE_PT));
    let spaced = line(
        "",
        &format!("{liberation};font-variant-position:super"),
        "4 5",
    );
    assert!(
        spaced
            .runs
            .iter()
            .all(|run| run.font_size == SIZE_PT && !run.synthesis.position),
        "spaces need no variant glyph"
    );
}

#[test]
fn font_size_adjust_normalizes_the_metric_of_each_face() {
    let metrics: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/FontUnit-metrics.json"
    ))
    .unwrap();
    let metric = |face: &str, name: &str| {
        let face = &metrics["faces"][face];
        face[name].as_f64().unwrap() as f32 / face["units_per_em"].as_f64().unwrap() as f32
    };
    let serif_x = metric("InitialSerif", "x_height");
    let fixture_x = metric("SourceSerif4", "x_height");
    let trace = render(
        "",
        "<p style=\"font-size-adjust:0.5\"><span style=\"font-family:Serif4\">x</span>x</p>",
    )
    .unwrap();
    let mixed = &trace.pages[0][0];
    let sizes: Vec<f32> = mixed.runs.iter().map(|run| run.font_size).collect();
    assert_eq!(sizes.len(), 2);
    common::assert_labelled_pt("adjusted fixture size", sizes[0], SIZE_PT * 0.5 / fixture_x);
    common::assert_labelled_pt("adjusted serif size", sizes[1], SIZE_PT * 0.5 / serif_x);
    common::assert_labelled_pt("equal x-heights", sizes[0] * fixture_x, sizes[1] * serif_x);
    common::assert_labelled_pt(
        "numeric line-height uses the computed size",
        line("", "font-size-adjust:0.5", "x").height,
        LINE_HEIGHT_PT,
    );
    for (declarations, expected) in [
        ("font-size-adjust:none", SIZE_PT),
        (
            "font-size-adjust:cap-height 0.7",
            SIZE_PT * 0.7 / metric("InitialSerif", "cap_height"),
        ),
        (
            "font-size-adjust:ch-width 0.6",
            SIZE_PT * 0.6 / metric("InitialSerif", "zero_advance"),
        ),
        // Liberation Serif has no "水"; its ideographic advance is assumed to be 1em.
        ("font-size-adjust:ic-width 0.5", SIZE_PT * 0.5),
        ("font-size-adjust:ic-height 0.5", SIZE_PT * 0.5),
        ("font-size-adjust:from-font", SIZE_PT),
        ("font-size-adjust:cap-height from-font", SIZE_PT),
    ] {
        let line = line("", declarations, "x");
        common::assert_labelled_pt(declarations, line.runs[0].font_size, expected);
    }
    let inherited = render(
        "",
        "<p style=\"font-family:Serif4;font-size-adjust:cap-height from-font\"><span style=\"font-family:serif\">H</span></p>",
    )
    .unwrap();
    common::assert_labelled_pt(
        "from-font computes to the parent's first available font metric, which children inherit",
        inherited.pages[0][0].runs[0].font_size,
        SIZE_PT * metric("SourceSerif4", "cap_height") / metric("InitialSerif", "cap_height"),
    );
}

#[test]
fn optical_sizing_and_variation_settings_select_independent_instances() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/RobotoFlex-optical.json"
    ))
    .unwrap();
    let upem = reference["units_per_em"].as_f64().unwrap() as f32;
    let advance = |location: serde_json::Value| {
        reference["samples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|sample| sample["location"] == location)
            .unwrap_or_else(|| panic!("optical sample {location}"))["advance"]
            .as_f64()
            .unwrap() as f32
    };
    let rules = "@font-face{font-family:Described;src:url(RobotoFlex.ttf);font-variation-settings:'opsz' 40}";
    for (declarations, location) in [
        ("font-family:Flex", serde_json::json!({"opsz": 20})),
        (
            "font-family:Flex;font-size:6pt",
            serde_json::json!({"opsz": 8}),
        ),
        (
            "font-family:Flex;font-size:200pt",
            serde_json::json!({"opsz": 144}),
        ),
        (
            "font-family:Flex;font-optical-sizing:none",
            serde_json::json!({"opsz": 14}),
        ),
        (
            "font-family:Flex;font-variation-settings:'opsz' 40",
            serde_json::json!({"opsz": 40}),
        ),
        (
            "font-family:Flex;font-variation-settings:'opsz' 400",
            serde_json::json!({"opsz": 144}),
        ),
        (
            "font-family:Flex;font-variation-settings:'wght' 700",
            serde_json::json!({"opsz": 20, "wght": 700}),
        ),
        (
            "font-family:Flex;font-weight:700;font-variation-settings:'wght' 400, 'ABCD' 5",
            serde_json::json!({"opsz": 20}),
        ),
        ("font-family:Described", serde_json::json!({"opsz": 20})),
        (
            "font-family:Described;font-optical-sizing:none",
            serde_json::json!({"opsz": 40}),
        ),
    ] {
        let line = line(rules, declarations, "H");
        let run = &line.runs[0];
        let actual = run.glyphs()[0].x_advance * upem;
        let expected = advance(location.clone());
        assert!(
            (actual - expected).abs() <= VARIABLE_UNIT_TOLERANCE,
            "{declarations}: expected the {location} advance {expected}, got {actual} font units"
        );
    }
    let adjusted = line("", "font-family:Flex;font-size-adjust:0.8", "H");
    let used = adjusted.runs[0].font_size;
    assert!(used > SIZE_PT, "the fixture must enlarge the font: {used}");
    let direct = line("", &format!("font-family:Flex;font-size:{used}pt"), "H");
    assert_eq!(
        adjusted.runs[0].glyphs()[0].x_advance,
        direct.runs[0].glyphs()[0].x_advance,
        "optical sizing follows the adjusted size (§8.1)"
    );
}

#[test]
fn emoji_presentation_appends_selectors_that_fonts_may_ignore() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/PresentationTest-source.json"
    ))
    .unwrap();
    let glyph = |text: &str| {
        reference["samples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|sample| sample["text"] == text)
            .unwrap_or_else(|| panic!("presentation sample {text:?}"))["glyph_id"]
            .as_u64()
            .unwrap() as u32
    };
    let (heart, beverage) = ("\u{2764}", "\u{2615}");
    let text = format!("{heart}{beverage}");
    for (declarations, source, expected) in [
        ("", text.clone(), [glyph(heart), glyph(beverage)]),
        (
            "font-variant-emoji:normal",
            text.clone(),
            [glyph(heart), glyph(beverage)],
        ),
        (
            "font-variant-emoji:text",
            text.clone(),
            [glyph("\u{2764}\u{fe0e}"), glyph("\u{2615}\u{fe0e}")],
        ),
        (
            "font-variant-emoji:emoji",
            text.clone(),
            [glyph("\u{2764}\u{fe0f}"), glyph("\u{2615}\u{fe0f}")],
        ),
        // U+2764 defaults to text presentation and U+2615 to emoji presentation (UTS #51).
        (
            "font-variant:unicode",
            text.clone(),
            [glyph("\u{2764}\u{fe0e}"), glyph("\u{2615}\u{fe0f}")],
        ),
        (
            "font-variant-emoji:text",
            format!("{heart}\u{fe0f}{beverage}"),
            [glyph("\u{2764}\u{fe0f}"), glyph("\u{2615}\u{fe0e}")],
        ),
    ] {
        let line = line(
            "",
            &format!("font-family:Presentation;{declarations}"),
            &source,
        );
        assert_eq!(glyph_ids(&line), expected, "{declarations} {source:?}");
        assert_eq!(line.text, source, "{declarations}: source text is retained");
    }
    let trace = render(
        "",
        &format!(
            "<p style=\"font-family:'DejaVu Sans',Presentation\">{heart}</p><p style=\"font-family:'DejaVu Sans',Presentation;font-variant-emoji:emoji\">{heart}</p><p style=\"font-family:'DejaVu Sans';font-variant-emoji:emoji\">{heart}</p>"
        ),
    )
    .unwrap();
    let family = |line: usize| {
        trace.fonts[trace.pages[0][line].runs[0].font_id.index()]
            .family
            .as_str()
    };
    assert_eq!(family(0), "DejaVu Sans");
    assert_eq!(
        (family(1), glyph_ids(&trace.pages[0][1])),
        ("Presentation", vec![glyph("\u{2764}\u{fe0f}")]),
        "fallback prefers a face with the implied presentation sequence"
    );
    assert_eq!(
        (family(2), glyph_ids(&trace.pages[0][2])),
        ("DejaVu Sans", glyph_ids(&trace.pages[0][0])),
        "a selector no face supports is ignored"
    );
}

#[test]
fn palettes_resolve_per_family_from_cpal_and_palette_rules() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/PaletteTest-source.json"
    ))
    .unwrap();
    let palette = |index: usize| -> Vec<Color> {
        reference["palettes"][index]
            .as_array()
            .unwrap()
            .iter()
            .map(|color| {
                let channel = |index: usize| color[index].as_u64().unwrap() as u8;
                Color {
                    rgb: [channel(0), channel(1), channel(2)],
                    alpha: f32::from(channel(3)) / f32::from(u8::MAX),
                }
            })
            .collect()
    };
    const MAGENTA: Color = Color {
        rgb: [255, 0, 255],
        alpha: 1.0,
    };
    let custom = |index: usize| {
        let mut colors = palette(index);
        colors[1] = MAGENTA;
        colors
    };
    let rules = "@font-palette-values --custom{font-family:Palette;base-palette:2;override-colors:1 rgb(255,0,255),7 red}@font-palette-values --custom{font-family:Other;base-palette:1}@font-palette-values --later{font-family:Palette;base-palette:1}@font-palette-values --later{font-family:Palette;base-palette:dark}@font-palette-values --range{font-family:Palette;base-palette:9}@font-palette-values --generic{font-family:Palette, serif;base-palette:1}@font-palette-values --current{font-family:Palette;base-palette:light;override-colors:1 magenta;override-colors:0 currentcolor}@font-palette-values --anonymous{base-palette:1}";
    for (value, expected) in [
        ("normal", None),
        ("light", Some(palette(1))),
        ("dark", Some(palette(2))),
        ("--custom", Some(custom(2))),
        ("--later", Some(palette(2))),
        ("--range", Some(palette(0))),
        ("--generic", None),
        ("--current", Some(custom(1))),
        ("--anonymous", None),
        ("--undefined", None),
    ] {
        let line = line(
            rules,
            &format!("font-family:Palette;font-palette:{value}"),
            "AHI",
        );
        assert_eq!(
            line.runs[0].palette.as_deref().map(<[Color]>::to_vec),
            expected,
            "font-palette:{value}"
        );
    }
    let plain = line("", "font-family:'DejaVu Sans';font-palette:light", "A");
    assert_eq!(
        plain.runs[0].palette, None,
        "faces without CPAL keep their colours"
    );
    // Descriptors substitute the root element's custom properties (CSS Fonts 4 §9.2).
    let substituted = |rules: &str| {
        line(
            &format!("@font-palette-values --variable{{font-family:Palette;base-palette:var(--base, 0);override-colors:var(--entry, 0) var(--colour)}}{rules}"),
            "font-family:Palette;font-palette:--variable",
            "A",
        )
        .runs[0]
            .palette
            .as_deref()
            .map(<[Color]>::to_vec)
    };
    let mut overridden = palette(1);
    overridden[1] = MAGENTA;
    assert_eq!(
        substituted(":root{--base:1;--entry:1;--colour:magenta}"),
        Some(overridden),
        "root custom properties substitute into the base palette and override colours"
    );
    assert_eq!(
        substituted(":root{--base:1}p{--colour:magenta}"),
        Some(palette(1)),
        "an element's custom properties do not reach the rule, whose undefined var() makes \
         override-colors invalid and ignored"
    );
}

/// Each entry mixed as `color-mix()` would: gamma-encoded sRGB averages here, weighted by the
/// normalized percentages; a shortfall below 100% scales the alpha (CSS Color 5 §3.2).
#[test]
fn palette_mixes_interpolate_each_entry_of_the_mixed_palettes() {
    let rules = "@font-palette-values --custom{font-family:Palette;base-palette:2;override-colors:1 rgb(255,0,255)}";
    let colour = |rgb: [u8; 3], alpha: f32| Color { rgb, alpha };
    for (value, expected) in [
        (
            "palette-mix(in srgb, light, dark)",
            vec![colour([110, 110, 100], 1.0), colour([100, 160, 100], 1.0)],
        ),
        (
            "palette-mix(in srgb, normal 25%, --custom)",
            vec![colour([215, 165, 0], 1.0), colour([191, 30, 191], 1.0)],
        ),
        (
            "palette-mix(in srgb, light 20%, dark 20%)",
            vec![colour([110, 110, 100], 0.4), colour([100, 160, 100], 0.4)],
        ),
        (
            "palette-mix(in srgb, palette-mix(in srgb, light, dark) 50%, normal)",
            vec![colour([155, 55, 50], 1.0), colour([50, 140, 50], 1.0)],
        ),
    ] {
        let line = line(
            rules,
            &format!("font-family:Palette;font-palette:{value}"),
            "A",
        );
        let actual = line.runs[0]
            .palette
            .as_deref()
            .map_or_else(|| panic!("{value} gives a palette"), <[Color]>::to_vec);
        assert_eq!(
            actual.len(),
            expected.len(),
            "{value}: one colour per entry"
        );
        for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
            assert!(
                actual.rgb == expected.rgb && (actual.alpha - expected.alpha).abs() < 1e-3,
                "{value}: entry {index} is {actual:?}, expected {expected:?}"
            );
        }
    }
}
