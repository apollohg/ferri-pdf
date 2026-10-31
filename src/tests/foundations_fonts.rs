use super::common;
use crate::dom::{DocumentTree, Selector};
use crate::fonts::{FontFamily, FontStyle, FontWeight, GenericFamily};
use crate::style::{LineHeight, Style, StyleError, Stylesheet, StylesheetSource};

fn computed(css: &str) -> Result<Style, StyleError> {
    computed_at(css, Default::default())
}

fn computed_at(css: &str, base: crate::ResourceBase) -> Result<Style, StyleError> {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base,
    }])?;
    let tree = DocumentTree::parse("<div id=parent><p id=target>Font text</p></div>");
    let parent = tree
        .select(&Selector::parse("#parent").unwrap())
        .next()
        .unwrap();
    let target = tree
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    let parent = sheet.computed(parent, &Style::default())?;
    sheet.computed(target, &parent)
}

#[test]
fn font_shorthand_forms_and_var_expansion() {
    for (source, size, leading, style, weight) in [
        (
            "12pt/18pt serif",
            12.0,
            Some(18.0),
            FontStyle::Normal,
            FontWeight::NORMAL,
        ),
        (
            "italic bold 18px/1.5 serif",
            13.5,
            Some(20.25),
            FontStyle::Italic,
            FontWeight::BOLD,
        ),
        (
            "700 italic 120%/140% serif",
            24.0,
            Some(33.6),
            FontStyle::Italic,
            FontWeight::BOLD,
        ),
        (
            "normal normal normal normal 14pt serif",
            14.0,
            None,
            FontStyle::Normal,
            FontWeight::NORMAL,
        ),
        (
            "calc(8pt * 2)/2 serif",
            16.0,
            Some(32.0),
            FontStyle::Normal,
            FontWeight::NORMAL,
        ),
    ] {
        let actual = computed(&format!(
            "#parent{{font-size:20pt}}#target{{--face:{source};font:var(--face)}}"
        ))
        .unwrap_or_else(|error| panic!("font {source} must compute: {error}"));
        common::assert_pt(actual.size, size);
        match (actual.line_height.resolve(actual.size), leading) {
            (Some(actual), Some(expected)) => common::assert_pt(actual, expected),
            (actual, expected) => assert_eq!(actual, expected, "font {source}"),
        }
        assert_eq!(actual.font_style, style, "font {source}");
        assert_eq!(actual.font_weight, weight, "font {source}");
        assert_eq!(
            actual.families,
            vec![FontFamily::Generic(GenericFamily::Serif)]
        );
    }
}

#[test]
fn font_shorthand_resets_only_its_defined_properties() {
    let features = |style: &Style| {
        (
            style.font_features.clone(),
            style.font_adjustments.clone(),
            style.font_variant_caps,
        )
    };
    let initial = features(&Style::default());
    for (property, value) in [
        ("font-feature-settings", "'kern' 0"),
        ("font-kerning", "none"),
        ("font-language-override", "'TRK'"),
        ("font-optical-sizing", "none"),
        ("font-size-adjust", "0.6"),
        ("font-variant-alternates", "historical-forms"),
        ("font-variant-caps", "small-caps"),
        ("font-variant-east-asian", "ruby"),
        ("font-variant-emoji", "emoji"),
        ("font-variant-ligatures", "none"),
        ("font-variant-numeric", "oldstyle-nums"),
        ("font-variant-position", "super"),
        ("font-variation-settings", "'wght' 750"),
    ] {
        let before = computed(&format!("#target{{{property}:{value}}}"))
            .unwrap_or_else(|error| panic!("{property}:{value} must compute: {error}"));
        assert_ne!(
            features(&before),
            initial,
            "the control must set a non-initial value for {property}:{value}"
        );
        let after = computed(&format!("#target{{{property}:{value};font:12pt serif}}"))
            .unwrap_or_else(|error| panic!("{property}:{value} then font must compute: {error}"));
        assert_eq!(
            features(&after),
            initial,
            "font must reset {property}:{value} to its initial value"
        );
    }
    let kept = computed(
        "#target{font-palette:light;font-synthesis:none;font-width:condensed;font:12pt serif}",
    )
    .unwrap();
    assert_eq!(kept.size, 12.0);
    assert_eq!(kept.font_width, crate::fonts::FontWidth::NORMAL);
    assert_eq!(
        kept.font_synthesis,
        crate::fonts::FontSynthesis::default(),
        "font must leave synthesis independently cascaded"
    );
    assert_eq!(
        kept.font_features.palette,
        crate::fonts::FontPalette::Light,
        "font must leave font-palette independently cascaded"
    );
}

#[test]
fn palette_mixes_equivalent_to_one_palette_compute_to_it() {
    use crate::fonts::FontPalette;
    for (value, expected) in [
        ("palette-mix(in oklab, light, light)", FontPalette::Light),
        (
            "palette-mix(in srgb, --a 100%, dark 0%)",
            FontPalette::Named("--a".into()),
        ),
        (
            "palette-mix(in hsl, normal 70%, normal)",
            FontPalette::Normal,
        ),
    ] {
        let style = computed(&format!("#target{{font-palette:{value}}}")).unwrap();
        assert_eq!(style.font_features.palette, expected, "{value}");
    }
    for value in [
        "palette-mix(in srgb, light, dark)",
        "palette-mix(in srgb, light 30%, light 30%)",
    ] {
        let style = computed(&format!("#target{{font-palette:{value}}}")).unwrap();
        assert!(
            matches!(style.font_features.palette, FontPalette::Mix(_)),
            "{value} mixes distinct palettes or keeps a shortfall's alpha: {:?}",
            style.font_features.palette
        );
    }
}

#[test]
fn system_keywords_and_named_family_positions_differ() {
    for keyword in [
        "caption",
        "icon",
        "menu",
        "message-box",
        "small-caption",
        "status-bar",
    ] {
        let actual = computed(&format!(
            "#parent{{font-size:25pt;font-weight:bold;font-style:italic}}#target{{font:{keyword}}}"
        ))
        .unwrap();
        let default = Style::default();
        assert_eq!(
            actual.size, default.size,
            "{keyword} uses the static UA substitute"
        );
        assert_eq!(actual.families, default.families);
        assert_eq!(actual.font_style, default.font_style);
        assert_eq!(actual.font_weight, default.font_weight);
        assert_eq!(actual.line_height, LineHeight::Normal);
    }
    let actual = computed("#target{font:18pt menu, 'serif', sans-serif}").unwrap();
    assert_eq!(
        actual.families,
        vec![
            FontFamily::Named("menu".into()),
            FontFamily::Named("serif".into()),
            FontFamily::Generic(GenericFamily::SansSerif)
        ]
    );
}

#[test]
fn font_width_stretch_aliases_share_cascade_and_variables() {
    for declarations in [
        "font-width:condensed;font-stretch:normal",
        "font-stretch:condensed;font-width:normal",
        "font-width:condensed;font:12pt serif",
        "--width:normal;font-width:condensed;font-stretch:var(--width)",
        "--width:condensed;font-stretch:var(--width);font-width:normal",
    ] {
        let actual = computed(&format!("#target{{{declarations}}}")).unwrap_or_else(|error| {
            panic!("a later canonical normal width must win for {declarations}: {error}")
        });
        assert_eq!(
            actual.font_width,
            crate::fonts::FontWidth::NORMAL,
            "{declarations}"
        );
    }
    let actual = computed("#target{font-width:condensed!important;font-stretch:normal}").unwrap();
    assert_eq!(actual.font_width, crate::fonts::FontWidth::CONDENSED);
}

#[test]
fn invalid_whole_fonts_and_invalid_computed_fonts_follow_different_cascade_rules() {
    for invalid in [
        "italic italic 12pt serif",
        "bold 700 12pt serif",
        "normal normal normal normal normal 12pt serif",
        "12pt",
        "12pt/ serif",
        "12pt serif,",
        "condensed expanded 12pt serif",
        "75% 12pt serif",
    ] {
        let actual = computed(&format!(
            "#target{{font-size:17pt;font-weight:bold;font:{invalid}}}"
        ))
        .unwrap();
        assert_eq!(actual.size, 17.0, "invalid whole shorthand {invalid}");
        assert_eq!(actual.font_weight, FontWeight::BOLD);
    }
    let actual = computed("#parent{font-size:20pt;font-weight:normal}#target{font-size:17pt;font-weight:bold;font:var(--missing)}").unwrap();
    assert_eq!(
        actual.size, 20.0,
        "invalid-at-computed-time font is unset rather than falling back to an earlier size"
    );
    assert_eq!(actual.font_weight, FontWeight::NORMAL);
}

#[test]
fn font_shorthand_inheritance_and_layer_rollback_keep_the_selected_values() {
    let actual =
        computed("#parent{font:italic bold 20pt/1.5 serif}#target{font:inherit;font-size:10pt}")
            .unwrap();
    assert_eq!(actual.size, 10.0);
    assert_eq!(actual.line_height.resolve(actual.size), Some(15.0));
    assert_eq!(actual.font_style, FontStyle::Italic);
    assert_eq!(actual.font_weight, FontWeight::BOLD);
    let actual = computed("@layer base, override;@layer base{#target{font:bold 16pt/2 serif}}@layer override{#target{font:italic 20pt monospace;font:revert-layer}}").unwrap();
    assert_eq!(actual.size, 16.0);
    assert_eq!(actual.font_weight, FontWeight::BOLD);
    assert_eq!(actual.line_height.resolve(actual.size), Some(32.0));
    let actual =
        computed("#target{font:bold 17pt/2 serif;font-synthesis:none;all:initial}").unwrap();
    let initial = Style::default();
    assert_eq!(actual.size, initial.size);
    assert_eq!(actual.families, initial.families);
    assert_eq!(actual.font_weight, initial.font_weight);
    assert_eq!(actual.line_height, initial.line_height);
    assert_eq!(
        actual.font_synthesis, initial.font_synthesis,
        "all resets synthesis even though font does not"
    );
}

#[test]
fn font_shorthand_queries_follow_actual_supported_longhands() {
    for (source, supported) in [
        ("12pt serif", true),
        ("normal normal normal normal 14pt serif", true),
        ("italic bold 16pt/1.5 serif", true),
        ("menu", true),
        ("small-caps 12pt serif", true),
        ("condensed 12pt serif", true),
        ("oblique 25deg 12pt serif", true),
        ("12pt", false),
    ] {
        let actual = computed(&format!(
            "#target{{font-size:9pt}}@supports(font:{source}){{#target{{font-size:18pt}}}}"
        ))
        .unwrap();
        assert_eq!(
            actual.size,
            if supported { 18.0 } else { 9.0 },
            "support query for font:{source}"
        );
    }
}

#[test]
fn default_font_shorthand_reaches_shaping_and_pdf_serialization() {
    let html = format!(
        "<style>{}p{{font:italic bold 18pt/1.5 serif}}</style><p>Shared font shorthand</p>",
        common::PROFILE
    );
    let output = crate::render_html(&html, &common::local_options()).unwrap();
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 1);
    let line = &trace.pages[0][0];
    assert_eq!(line.text, "Shared font shorthand");
    assert_eq!(line.font_size, 18.0);
    assert!(!line.runs.is_empty());
}

const VARIABLE_FAMILY: &str = "Variable Fixture";
const VARIABLE_SIZE_PT: f32 = 20.0;

fn variable_options() -> crate::RenderOptions {
    let mut options = common::local_options();
    options.base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fonts")
        .into();
    options
}

fn variable_font(descriptors: &str, declarations: &str) -> crate::RenderOutput {
    let html = format!(
        "<style>@font-face{{font-family:'{VARIABLE_FAMILY}';src:url(RobotoFlex.ttf) format('truetype');{descriptors}}}@page{{size:300pt 200pt;margin:20pt}}p{{margin:0;font-family:'{VARIABLE_FAMILY}';font-size:{VARIABLE_SIZE_PT}pt;line-height:24pt;font-optical-sizing:none;{declarations}}}</style><p>H</p>"
    );
    crate::render_html(&html, &variable_options()).unwrap_or_else(|error| {
        panic!("variable face {descriptors:?}, request {declarations:?}: {error}")
    })
}

fn independent_variable_advance(sample: usize) -> f32 {
    let metrics: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/RobotoFlex-metrics.json"
    ))
    .unwrap();
    let units = metrics["samples"][sample]["advance"].as_f64().unwrap();
    let upem = metrics["units_per_em"].as_f64().unwrap();
    // The shaper reports nearest whole font units; FontTools retains fractional advances.
    (units.round() / upem) as f32 * VARIABLE_SIZE_PT
}

#[test]
fn variable_weight_width_and_slant_use_independently_measured_instances() {
    let metrics: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/RobotoFlex-metrics.json"
    ))
    .unwrap();
    for (sample, measured) in metrics["samples"].as_array().unwrap().iter().enumerate() {
        let location = &measured["location"];
        let weight = location["wght"].as_f64().unwrap();
        let width = location["wdth"].as_f64().unwrap();
        let angle = -location["slnt"].as_f64().unwrap();
        let request =
            format!("font-weight:{weight};font-width:{width}%;font-style:oblique {angle}deg");
        let output = variable_font("", &request);
        let trace = output.layout.unwrap();
        let line = &trace.pages[0][0];
        assert_eq!(line.text, "H", "{request}");
        common::assert_pt(line.width, independent_variable_advance(sample));
        for run in &line.runs {
            assert_eq!(trace.fonts[run.font_id.index()].family, VARIABLE_FAMILY);
            assert!(
                !run.synthesis.bold && !run.synthesis.oblique,
                "native axes must supply this instance, without synthetic styling: {request}"
            );
        }
    }
}

#[test]
fn descriptor_ranges_clamp_instances_and_reversed_endpoints_are_swapped() {
    for descriptors in [
        "font-weight:300 600;font-width:75% 125%;font-style:oblique 0deg 10deg",
        "font-weight:600 300;font-width:125% 75%;font-style:oblique 10deg 0deg",
    ] {
        for (request, sample) in [
            (
                "font-weight:900;font-width:200%;font-style:oblique 25deg",
                9,
            ),
            ("font-weight:100;font-width:50%;font-style:normal", 10),
        ] {
            let trace = variable_font(descriptors, request).layout.unwrap();
            common::assert_pt(
                trace.pages[0][0].width,
                independent_variable_advance(sample),
            );
        }
    }
}

#[test]
fn face_matching_narrows_width_before_style_and_weight() {
    let faces: String = [
        (75, "normal", 400),
        (100, "italic", 400),
        (100, "normal", 700),
        (125, "normal", 400),
    ]
    .into_iter()
    .map(|(width, style, weight)| {
        format!("@font-face{{font-family:'{VARIABLE_FAMILY}';src:url(RobotoFlex.ttf);font-width:{width}%;font-style:{style};font-weight:{weight}}}")
    })
    .collect();
    for (request, expected_units) in [
        ("font-width:100%;font-style:normal;font-weight:400", 1508.0),
        ("font-width:100%;font-style:italic;font-weight:700", 1458.0),
        ("font-width:90%;font-style:italic;font-weight:700", 1406.0),
        ("font-width:110%;font-style:normal;font-weight:400", 1536.0),
    ] {
        let html = format!(
            "<style>{faces}p{{font-family:'{VARIABLE_FAMILY}';font-size:{VARIABLE_SIZE_PT}pt;line-height:24pt;font-optical-sizing:none;{request}}}</style><p>H</p>"
        );
        let output = crate::render_html(&html, &variable_options())
            .unwrap_or_else(|error| panic!("face matching for {request}: {error}"));
        common::assert_pt(
            output.layout.unwrap().pages[0][0].width,
            expected_units / 2048.0 * VARIABLE_SIZE_PT,
        );
    }
}

#[test]
fn modern_font_source_hints_skip_unsupported_technology_and_report_policy_failures() {
    const FONT_BYTE_LIMIT: usize = 4;
    let html = format!(
        "<style>@font-face{{font-family:'{VARIABLE_FAMILY}';src:url('../../../assets/fonts/DejaVuSans.ttf') format(truetype) tech(features-graphite),url(RobotoFlex.ttf) format(truetype) tech(variations)}}p{{font-family:'{VARIABLE_FAMILY}';font-size:{VARIABLE_SIZE_PT}pt;line-height:24pt;font-optical-sizing:none}}</style><p>H</p>"
    );
    let options = variable_options();
    let output = crate::render_html(&html, &options)
        .expect("a supported modern format/technology entry must survive an unsupported one");
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.fonts[trace.pages[0][0].runs[0].font_id.index()].family,
        VARIABLE_FAMILY,
        "unsupported source hints must not cause the entire face to disappear"
    );
    common::assert_pt(trace.pages[0][0].width, independent_variable_advance(1));
    assert_eq!(
        computed("#target{font-size:10pt}@supports font-tech(variations){#target{font-size:20pt}}")
            .unwrap()
            .size,
        VARIABLE_SIZE_PT,
        "variation support queries must agree with the real source and rendering path"
    );
    let resolver = crate::FileResolver {
        max_resource_bytes: FONT_BYTE_LIMIT,
        ..Default::default()
    };
    let output = crate::render_with_resolver(&html, &options, &resolver)
        .expect("a font over the resource limit is unavailable, not fatal");
    assert!(
        matches!(
            output.diagnostics.as_slice(),
            [crate::RenderDiagnostic::UnavailableFont(failure)]
                if failure.reference == "RobotoFlex.ttf"
                    && matches!(failure.error, crate::fonts::FontError::Resource(
                        crate::ResourceError::TooLarge { limit: FONT_BYTE_LIMIT, .. }
                    ))
        ),
        "font technology fallback must still report the resource policy failure: {:?}",
        output.diagnostics
    );
    let trace = output.layout.unwrap();
    assert_ne!(
        trace.fonts[trace.pages[0][0].runs[0].font_id.index()].family,
        VARIABLE_FAMILY,
        "text falls back past the unavailable face"
    );
}

#[test]
fn invalid_font_faces_do_not_abort_document_rendering() {
    for face in [
        "@font-face{font-family:Missing}",
        "@font-face{src:url(missing.ttf)}",
        "@font-face{font-family:Missing;src:url(missing.ttf) format('unknown-format')}",
    ] {
        let html = format!("<style>{face}p{{font-family:Missing,serif}}</style><p>Fallback</p>");
        let output = crate::render_html(&html, &variable_options())
            .unwrap_or_else(|error| panic!("invalid face must be ignored: {face}: {error}"));
        assert_eq!(output.layout.unwrap().pages[0][0].text, "Fallback");
    }
}

#[test]
fn font_source_lists_recover_invalid_items_without_losing_valid_ones() {
    for sources in [
        "url(RobotoFlex.ttf) format(unknown),url(RobotoFlex.ttf) format(truetype)",
        "url(RobotoFlex.ttf) tech(unknown-tech),url(RobotoFlex.ttf)",
        "local(),url(RobotoFlex.ttf)",
        "url(RobotoFlex.ttf) format(truetype,woff2),url(RobotoFlex.ttf)",
        "url(RobotoFlex.ttf) format(unknown),url(RobotoFlex.ttf) format('truetype-variations')",
    ] {
        let html = format!(
            "<style>@font-face{{font-family:'{VARIABLE_FAMILY}';src:{sources}}}p{{font-family:'{VARIABLE_FAMILY}';font-size:{VARIABLE_SIZE_PT}pt;line-height:24pt;font-optical-sizing:none}}</style><p>H</p>"
        );
        let output = crate::render_html(&html, &variable_options())
            .unwrap_or_else(|error| panic!("source list {sources}: {error}"));
        let trace = output.layout.unwrap();
        let run = &trace.pages[0][0].runs[0];
        assert_eq!(
            trace.fonts[run.font_id.index()].family,
            VARIABLE_FAMILY,
            "invalid items must not discard a usable source: {sources}"
        );
        common::assert_pt(trace.pages[0][0].width, independent_variable_advance(1));
    }
}

#[test]
fn oblique_angle_bounds_compare_canonical_degrees_before_cascade() {
    for (source, expected) in [
        ("0.25turn", Some(90.0)),
        ("-0.25turn", Some(-90.0)),
        ("100grad", Some(90.0)),
        ("0.5rad", Some(0.5_f32.to_degrees())),
        ("0.5turn", None),
        ("101grad", None),
        ("-91deg", None),
    ] {
        let actual = computed(&format!(
            "#target{{font-style:italic;font-style:oblique {source}}}"
        ))
        .unwrap_or_else(|error| panic!("oblique {source}: {error}"));
        if let Some(degrees) = expected {
            assert_eq!(actual.font_style, FontStyle::Oblique);
            common::assert_pt(actual.font_slant.value(), degrees);
        } else {
            assert_eq!(
                actual.font_style,
                FontStyle::Italic,
                "invalid oblique {source} must leave the earlier declaration intact"
            );
        }
    }
}

#[test]
fn font_relative_units_measure_the_same_variable_instance_as_shaping() {
    let metrics: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/RobotoFlex-metrics.json"
    ))
    .unwrap();
    let parent = metrics["samples"][5]["zero_advance"].as_f64().unwrap();
    let child = metrics["samples"][1]["zero_advance"].as_f64().unwrap();
    let upem = metrics["units_per_em"].as_f64().unwrap();
    let parent_ch = parent.round() as f32 / upem as f32;
    let child_ch = child.round() as f32 / upem as f32;
    assert_ne!(
        parent_ch, child_ch,
        "the fixture must distinguish the parent and child width instances"
    );
    let actual = computed_at(
        &format!("@font-face{{font-family:'{VARIABLE_FAMILY}';src:url(RobotoFlex.ttf)}}#parent{{font-family:'{VARIABLE_FAMILY}';font-size:{VARIABLE_SIZE_PT}pt;font-width:75%;font-optical-sizing:none}}#target{{font-size:1ch;font-width:100%;width:1ch}}"),
        variable_options().base,
    ).unwrap();
    common::assert_pt(actual.size, parent_ch * VARIABLE_SIZE_PT);
    common::assert_pt(
        actual.width.resolve_definite(None).unwrap(),
        child_ch * actual.size,
    );
}

fn caps_render(text: &str, declarations: &str, language: &str) -> crate::RenderOutput {
    let html = format!(
        "<style>@font-face{{font-family:CapsFixture;src:url(SourceSerif4-Regular.ttf)}}@font-face{{font-family:Limited;src:url(FontStyleTest-Ital.ttf)}}@page{{size:300pt 200pt;margin:20pt}}p{{margin:0;font:20pt/28pt serif;{declarations}}}</style><p lang='{language}'>{text}</p>"
    );
    crate::render_html(&html, &variable_options()).unwrap_or_else(|error| {
        panic!("caps {text:?}, {declarations:?}, language {language:?}: {error}")
    })
}

#[test]
fn small_caps_uses_available_features_or_defined_synthesis() {
    let measured: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/SmallCaps-metrics.json"
    ))
    .unwrap();
    for (sample, text, language) in [(0, "Aaß fi", "und"), (1, "iıIİ", "tr")] {
        let output = caps_render(
            text,
            "font-family:CapsFixture;font-variant:small-caps;font-synthesis:none",
            language,
        );
        let trace = output.layout.unwrap();
        let line = &trace.pages[0][0];
        let expected = &measured["samples"][sample];
        let glyphs = expected["glyphs"].as_array().unwrap();
        let actual: Vec<_> = line
            .runs
            .iter()
            .flat_map(|r| r.glyphs().iter().map(|g| g.id))
            .collect();
        assert_eq!(
            actual,
            glyphs
                .iter()
                .map(|g| g["g"].as_u64().unwrap() as u32)
                .collect::<Vec<_>>(),
            "native smcp for {text:?}"
        );
        common::assert_pt(
            line.width,
            glyphs
                .iter()
                .map(|g| g["ax"].as_f64().unwrap() as f32)
                .sum::<f32>()
                / expected["units_per_em"].as_f64().unwrap() as f32
                * 20.0,
        );
        assert_eq!(line.text, text);
        assert!(line.runs.iter().all(|r| r.font_size == 20.0));
    }
    let trace = caps_render("Aaß", "font-variant-caps:small-caps", "und")
        .layout
        .unwrap();
    let runs = &trace.pages[0][0].runs;
    assert_eq!(
        runs.iter().map(|r| r.text.as_str()).collect::<String>(),
        "Aaß"
    );
    assert_eq!(
        runs.iter().map(|r| r.font_size).collect::<Vec<_>>(),
        vec![20.0, 16.0]
    );
    let lower = &runs[1];
    assert_eq!(
        lower.glyphs().iter().map(|g| g.id).collect::<Vec<_>>(),
        vec![36, 54, 54]
    );
    assert_eq!(
        lower
            .glyphs()
            .iter()
            .map(|g| g.text_range.clone())
            .collect::<Vec<_>>(),
        vec![0..1, 1..3, 1..3]
    );
    common::assert_pt(lower.width, (1479.0 + 1139.0 * 2.0) / 2048.0 * 16.0);
    let trace = caps_render("iı", "font-variant-caps:small-caps", "tr")
        .layout
        .unwrap();
    let expected = &measured["samples"][3];
    common::assert_pt(
        trace.pages[0][0].width,
        expected["glyphs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["ax"].as_f64().unwrap() as f32)
            .sum::<f32>()
            / expected["units_per_em"].as_f64().unwrap() as f32
            * 16.0,
    );
    assert_eq!(
        trace.pages[0][0]
            .runs
            .iter()
            .flat_map(|r| r.glyphs().iter().map(|g| g.id))
            .collect::<Vec<_>>(),
        expected["glyphs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["g"].as_u64().unwrap() as u32)
            .collect::<Vec<_>>()
    );
}

#[test]
fn small_caps_preserves_text_clusters_fallback_and_line_breaks() {
    let normal = caps_render("Aaß 123", "", "und").layout.unwrap();
    let disabled = caps_render(
        "Aaß 123",
        "font-variant:small-caps;font-synthesis-small-caps:none",
        "und",
    )
    .layout
    .unwrap();
    common::assert_pt(disabled.pages[0][0].width, normal.pages[0][0].width);
    assert!(
        disabled.pages[0][0]
            .runs
            .iter()
            .all(|r| r.font_size == 20.0)
    );
    let fallback = caps_render(
        "aßЖ",
        "font-family:Limited,serif;font-variant:small-caps",
        "und",
    )
    .layout
    .unwrap();
    let runs = &fallback.pages[0][0].runs;
    assert_eq!(
        runs.iter().map(|r| r.text.as_str()).collect::<String>(),
        "aßЖ"
    );
    assert_eq!(fallback.fonts[runs[0].font_id.index()].family, "Limited");
    assert!(
        runs.iter()
            .any(|r| fallback.fonts[r.font_id.index()].family == "Liberation Serif")
    );
    let text = "Aaß test iı words many times wrapped across narrow lines ".repeat(4);
    let trace = caps_render(
        &text,
        "font:small-caps 20pt/28pt serif;width:85pt;letter-spacing:1pt;text-decoration:underline",
        "tr",
    )
    .layout
    .unwrap();
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    assert!(
        trace.pages.len() > 1,
        "the caps case must exercise page fragmentation"
    );
    assert!(lines.len() > 3);
    assert_eq!(
        lines
            .iter()
            .flat_map(|l| l.text.split_whitespace())
            .collect::<Vec<_>>(),
        text.split_whitespace().collect::<Vec<_>>()
    );
    assert!(lines.iter().all(|l| l.width <= 85.001));
    assert!(
        lines
            .iter()
            .flat_map(|l| &l.runs)
            .any(|r| r.font_size == 16.0)
    );
}

#[test]
fn small_caps_keep_nominal_line_and_link_metrics() {
    let normal = caps_render(
        "<a href='#end'>aաբ</a><span id=end></span>",
        "line-height:normal",
        "und",
    )
    .layout
    .unwrap();
    let caps = caps_render(
        "<a href='#end'>aաբ</a><span id=end></span>",
        "line-height:normal;font-variant:small-caps",
        "und",
    )
    .layout
    .unwrap();
    common::assert_pt(caps.pages[0][0].height, normal.pages[0][0].height);
    common::assert_pt(caps.pages[0][0].baseline, normal.pages[0][0].baseline);
    assert_eq!(caps.links.len(), normal.links.len());
    let (crate::LinkShape::Rect(normal), crate::LinkShape::Rect(caps)) =
        (&normal.links[0][0].shape, &caps.links[0][0].shape)
    else {
        panic!("untransformed text links must have rectangular activation areas");
    };
    common::assert_pt(caps.y, normal.y);
    common::assert_pt(caps.height, normal.height);
}

#[test]
fn small_caps_cascade_resets_and_synthesis_permissions_reach_shaping() {
    for (control, enabled) in [
        ("", true),
        ("font-synthesis:none", false),
        ("font-synthesis:weight style", false),
        ("font-synthesis:small-caps", true),
        ("font-synthesis-small-caps:none", false),
    ] {
        let trace = caps_render(
            "a 123",
            &format!("{control};font:small-caps 20pt/28pt serif"),
            "und",
        )
        .layout
        .unwrap();
        let runs = &trace.pages[0][0].runs;
        assert_eq!(
            runs[0].font_size,
            if enabled { 16.0 } else { 20.0 },
            "{control}"
        );
        assert_eq!(
            runs.last().unwrap().font_size,
            20.0,
            "caseless spaces and digits must retain full size: {control}"
        );
    }
    let inherited = computed("#parent{font-variant-caps:small-caps;font-synthesis:none}#target{font-variant-caps:inherit}").unwrap();
    assert_eq!(
        inherited.font_variant_caps,
        crate::style::FontVariantCaps::SmallCaps
    );
    assert_eq!(inherited.font_synthesis, Default::default());
    let reset =
        computed("#parent{font-variant:small-caps;font-synthesis:none}#target{font:20pt serif}")
            .unwrap();
    assert_eq!(
        reset.font_variant_caps,
        crate::style::FontVariantCaps::Normal
    );
    assert_eq!(reset.font_synthesis, Default::default());
    for (variant, caps, historical, styleset) in [
        (
            "all-small-caps",
            crate::style::FontVariantCaps::AllSmallCaps,
            false,
            &[][..],
        ),
        (
            "small-caps historical-forms",
            crate::style::FontVariantCaps::SmallCaps,
            true,
            &[],
        ),
        (
            "small-caps styleset(foo)",
            crate::style::FontVariantCaps::SmallCaps,
            false,
            &["foo"],
        ),
    ] {
        let style = computed(&format!("#target{{font-variant:{variant}}}"))
            .unwrap_or_else(|error| panic!("font-variant:{variant}: {error}"));
        assert_eq!(style.font_variant_caps, caps, "{variant}");
        let alternates = &style.font_features.alternates;
        assert_eq!(alternates.historical_forms, historical, "{variant}");
        assert_eq!(
            alternates
                .styleset
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<&str>>(),
            styleset,
            "{variant}"
        );
    }
    for (text, language, upper) in [("i\u{307}", "lt", "I"), ("άι", "el", "ΑΪ")] {
        let caps = caps_render(text, "font-variant-caps:small-caps", language)
            .layout
            .unwrap();
        let expected = caps_render(upper, "font-size:16pt", language)
            .layout
            .unwrap();
        let actual = &caps.pages[0][0];
        assert_eq!(actual.text, text);
        common::assert_pt(actual.width, expected.pages[0][0].width);
        assert_eq!(
            actual
                .runs
                .iter()
                .flat_map(|r| r.glyphs().iter().map(|g| g.id))
                .collect::<Vec<_>>(),
            expected.pages[0][0]
                .runs
                .iter()
                .flat_map(|r| r.glyphs().iter().map(|g| g.id))
                .collect::<Vec<_>>(),
            "contextual {language} capitals"
        );
        for run in &actual.runs {
            for glyph in run.glyphs() {
                assert!(
                    run.text.is_char_boundary(glyph.text_range.start)
                        && run.text.is_char_boundary(glyph.text_range.end)
                );
            }
        }
    }
}

fn character_policy_trace(families: &str, text: &str, declarations: &str) -> crate::LayoutTrace {
    crate::render_html(
        &format!("<!doctype html><style>html,body,p{{margin:0;padding:0;font:20pt/24pt {families}}}p{{{declarations}}}</style><p>{text}</p>"),
        &common::local_options(),
    ).unwrap_or_else(|error|panic!("character policy {families}/{text:?}: {error}")).layout.unwrap()
}

#[test]
fn exhausted_glyph_fallback_renders_visible_symbols_and_retains_source() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/character-policy-reference.json"
    ))
    .unwrap();
    let id = reference["characters"]["replacement"]["glyph_id"]
        .as_u64()
        .unwrap() as u32;
    let advance = reference["characters"]["replacement"]["advance"]
        .as_f64()
        .unwrap()
        / reference["units_per_em"].as_f64().unwrap()
        * 20.0;
    let text = "\u{0378}\u{10ffff}";
    let trace = character_policy_trace("serif", text, "");
    let line = &trace.pages[0][0];
    assert_eq!(line.text, text);
    let glyphs = line
        .runs
        .iter()
        .flat_map(crate::GlyphRun::glyphs)
        .collect::<Vec<_>>();
    assert_eq!(
        glyphs.len(),
        2,
        "each missing scalar needs a visible indication"
    );
    for glyph in glyphs {
        assert_eq!(glyph.id, id);
        common::assert_pt(glyph.advance(20.0), advance as f32);
    }
    assert!(
        line.runs
            .iter()
            .all(|run| trace.fonts[run.font_id.index()].family == "DejaVu Sans")
    );
}

#[test]
fn private_use_characters_require_an_explicit_non_generic_family() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/character-policy-reference.json"
    ))
    .unwrap();
    let replacement = reference["characters"]["replacement"]["glyph_id"]
        .as_u64()
        .unwrap() as u32;
    let covered = reference["characters"]["private_use"]["glyph_id"]
        .as_u64()
        .unwrap() as u32;
    for (families, id) in [
        ("sans-serif", replacement),
        ("'Liberation Serif', sans-serif", replacement),
        ("'DejaVu Sans'", covered),
        ("serif, 'DejaVu Sans'", covered),
    ] {
        let trace = character_policy_trace(families, "\u{ef00}", "");
        let line = &trace.pages[0][0];
        assert_eq!(line.text, "\u{ef00}");
        let ids = line
            .runs
            .iter()
            .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.id))
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [id],
            "{families}: generic coverage cannot define a private-use meaning"
        );
    }
}

#[test]
fn private_use_named_matching_keeps_its_font_namespace() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/character-policy-reference.json"
    ))
    .unwrap();
    let replacement = reference["characters"]["replacement"]["glyph_id"]
        .as_u64()
        .unwrap() as u32;
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/fonts/LiberationSerif-Regular.ttf");
    let trace=crate::render_html(
        &format!("<!doctype html><style>html,body,p{{margin:0;padding:0;font:20pt/24pt serif}}</style><div><template shadowrootmode=open><style>@font-face{{font-family:'DejaVu Sans';src:url('{}')}}p{{font-family:'DejaVu Sans',sans-serif}}</style><p>\u{ef00}</p></template></div>",font.display()),
        &common::local_options(),
    ).unwrap().layout.unwrap();
    let ids = trace.pages[0][0]
        .runs
        .iter()
        .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.id))
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [replacement],
        "the same family name in implicit root fallback is not an explicitly selected shadow face"
    );
}

#[test]
fn missing_symbols_keep_source_clusters_and_default_ignorables() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/character-policy-reference.json"
    ))
    .unwrap();
    let replacement = reference["characters"]["replacement"]["glyph_id"]
        .as_u64()
        .unwrap() as u32;
    let text = "a\u{0378}\u{200d}\u{10ffff}";
    for declarations in ["font-variant-caps:small-caps", "font-size:0pt"] {
        let trace = character_policy_trace("serif", text, declarations);
        let line = &trace.pages[0][0];
        assert_eq!(line.text, text);
        if declarations.contains("0pt") {
            common::assert_pt(line.width, 0.0);
        } else {
            let symbols = line
                .runs
                .iter()
                .filter(|run| trace.fonts[run.font_id.index()].family == "DejaVu Sans")
                .flat_map(crate::GlyphRun::glyphs)
                .filter(|glyph| glyph.id == replacement)
                .count();
            assert_eq!(
                symbols, 2,
                "the joiner must remain invisible while missing characters get symbols"
            );
        }
        for run in &line.runs {
            assert!(
                text.is_char_boundary(run.source_range.start)
                    && text.is_char_boundary(run.source_range.end)
            );
            for glyph in run.glyphs() {
                assert!(
                    run.text.is_char_boundary(glyph.text_range.start)
                        && run.text.is_char_boundary(glyph.text_range.end)
                );
            }
        }
    }
}

#[test]
fn incomplete_combining_clusters_keep_the_longest_supported_prefix() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/character-policy-reference.json"
    ))
    .unwrap();
    let replacement = reference["characters"]["replacement"]["glyph_id"]
        .as_u64()
        .unwrap() as u32;
    for (families, base, key, family) in [
        ("'DejaVu Sans'", '\u{ef00}', "private_use", "DejaVu Sans"),
        ("serif", 'Ω', "greek_prefix", "Liberation Serif"),
    ] {
        let text = format!("{base}\u{1ab0}");
        let trace = character_policy_trace(families, &text, "");
        let line = &trace.pages[0][0];
        assert_eq!(line.text, text);
        let ids = line
            .runs
            .iter()
            .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.id))
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                reference["characters"][key]["glyph_id"].as_u64().unwrap() as u32,
                replacement
            ],
            "{families}: supported prefix must survive missing mark fallback"
        );
        assert_eq!(trace.fonts[line.runs[0].font_id.index()].family, family);
    }
}

#[test]
fn variation_sequences_search_later_faces_before_discarding_the_selector() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/VariationSequenceTest-source.json"
    ))
    .unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fonts/VariationSequenceTest.ttf");
    for sample in reference["samples"].as_array().unwrap() {
        let text = sample["text"].as_str().unwrap();
        let html = format!(
            "<!doctype html><style>@font-face{{font-family:'Variation Fixture';src:url('{}')}}html,body,p{{margin:0;padding:0;font:20pt/24pt 'Liberation Serif','Variation Fixture'}}</style><p>{text}</p>",
            fixture.display()
        );
        let trace = crate::render_html(&html, &common::local_options())
            .unwrap()
            .layout
            .unwrap();
        let line = &trace.pages[0][0];
        assert_eq!(line.text, text);
        let runs = line
            .runs
            .iter()
            .filter(|run| !run.glyphs().is_empty())
            .collect::<Vec<_>>();
        assert_eq!(
            runs.len(),
            1,
            "{text:?}: selectors must stay with their base glyph"
        );
        let run = runs[0];
        let expected_family = if text == "A\u{fe01}" {
            "Liberation Serif"
        } else {
            "Variation Fixture"
        };
        assert_eq!(
            trace.fonts[run.font_id.index()].family,
            expected_family,
            "{text:?}: full sequence coverage must precede base-only fallback"
        );
        assert_eq!(
            run.glyphs().len(),
            1,
            "{text:?}: selectors have no separate visible glyph"
        );
        assert_eq!(
            run.glyphs()[0].id,
            sample["glyph_id"].as_u64().unwrap() as u32,
            "{text:?}: selected glyph must match independent HarfBuzz shaping"
        );
        common::assert_pt(
            run.width,
            (sample["advance"].as_f64().unwrap() / reference["units_per_em"].as_f64().unwrap()
                * 20.0) as f32,
        );
        assert_eq!(
            &run.text[run.glyphs()[0].text_range.clone()],
            text,
            "{text:?}: invisible selectors still identify the serialized variant"
        );
    }
}

#[test]
fn ignored_variation_selectors_do_not_remove_word_spacing() {
    for declarations in [
        "word-spacing:7pt",
        "word-spacing:7pt;width:100pt;text-align:justify;text-align-last:justify",
    ] {
        let plain = character_policy_trace("serif", "a b", declarations);
        let marked = character_policy_trace("serif", "a \u{fe01}b", declarations);
        common::assert_pt(marked.pages[0][0].width, plain.pages[0][0].width);
        assert_eq!(marked.pages[0][0].text, "a \u{fe01}b");
    }
}

#[test]
fn font_names_use_full_default_unicode_folding_without_normalization() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/font-name-matching.json"
    ))
    .unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/fonts/LiberationMono-Regular.ttf");
    for case in reference["cases"].as_array().unwrap() {
        let declared = case["declared"].as_str().unwrap();
        let requested = case["requested"].as_str().unwrap();
        let html = format!(
            "<!doctype html><html lang=tr><style>@font-face{{font-family:'{declared}';src:url('{}')}}p{{font:20pt/24pt '{requested}',serif}}</style><p>H</p></html>",
            fixture.display()
        );
        let trace = crate::render_html(&html, &common::local_options())
            .unwrap()
            .layout
            .unwrap();
        let run = &trace.pages[0][0].runs[0];
        let expected = if case["matches"].as_bool().unwrap() {
            declared
        } else {
            "Liberation Serif"
        };
        assert_eq!(
            trace.fonts[run.font_id.index()].family,
            expected,
            "{declared:?} / {requested:?}: default C/F folding must ignore the document language and retain canonical distinctions"
        );
    }
}

#[test]
fn shadow_font_names_fold_before_selecting_their_namespace() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
    let html = format!(
        "<!doctype html><style>@font-face{{font-family:STRASSE;src:url('{}')}}p{{font:20pt/24pt STRASSE}}</style><div><template shadowrootmode=open><style>@font-face{{font-family:'Straße';src:url('{}')}}p{{font:20pt/24pt STRASSE}}</style><p>H</p></template></div><p>H</p>",
        root.join("LiberationSerif-Regular.ttf").display(),
        root.join("LiberationMono-Regular.ttf").display()
    );
    let trace = crate::render_html(&html, &common::local_options())
        .unwrap()
        .layout
        .unwrap();
    let families = trace.pages[0]
        .iter()
        .map(|line| trace.fonts[line.runs[0].font_id.index()].family.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        families,
        ["Straße", "STRASSE"],
        "the folded shadow alias must hide the root face only inside its font namespace"
    );
    assert_ne!(trace.pages[0][0].width, trace.pages[0][1].width);
}

#[test]
fn local_font_full_and_postscript_names_use_the_same_unicode_fold() {
    for local in ["DejaVu ſans", "DejaVuſans"] {
        let html = format!(
            "<!doctype html><style>@font-face{{font-family:LocalAlias;src:local('{local}')}}p{{font:20pt/24pt LocalAlias}}</style><p>H</p>"
        );
        let trace = crate::render_html(&html, &common::local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            trace.fonts[trace.pages[0][0].runs[0].font_id.index()].family,
            "LocalAlias",
            "{local:?}: a Unicode-folded local name must resolve bundled data"
        );
    }
}

#[test]
fn font_size_adjust_scales_metric_units_but_not_em_or_numeric_line_heights() {
    let metrics: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/FontUnit-metrics.json"
    ))
    .unwrap();
    let serif = &metrics["faces"]["InitialSerif"];
    let em = |name: &str| {
        serif[name].as_f64().unwrap() as f32 / serif["units_per_em"].as_f64().unwrap() as f32
    };
    let size = 20.0;
    let used = size * 0.5 / em("x_height");
    let style = computed(
        "#target{font-size:20pt;font-size-adjust:0.5;line-height:normal;margin:1ex 1em 1ch 1cap;width:1lh}",
    )
    .unwrap();
    let length = |value: &crate::style::Size| value.resolve_definite(None).unwrap();
    common::assert_labelled_pt(
        "1ex keeps the specified aspect",
        length(&style.margin[0]),
        size * 0.5,
    );
    common::assert_labelled_pt("1em is the computed size", length(&style.margin[1]), size);
    common::assert_labelled_pt("1ch", length(&style.margin[2]), em("zero_advance") * used);
    common::assert_labelled_pt("1cap", length(&style.margin[3]), em("cap_height") * used);
    common::assert_labelled_pt(
        "normal line height uses the used size",
        length(&style.width),
        (em("ascender") - em("descender") + em("line_gap")) * used,
    );
    let numeric =
        computed("#target{font-size:20pt;font-size-adjust:0.5;line-height:1.5;width:1lh}").unwrap();
    common::assert_labelled_pt("numeric lh", length(&numeric.width), size * 1.5);
    let from_font = computed("#parent{font-size-adjust:ch-width from-font}").unwrap();
    assert_eq!(
        from_font.font_adjustments.size_adjust,
        Some(crate::fonts::SizeAdjust {
            metric: crate::fonts::SizeMetric::ChWidth,
            value: em("zero_advance"),
        }),
        "from-font computes to the first available font's metric, which the child inherits"
    );
}
