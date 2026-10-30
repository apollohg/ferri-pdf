//! CSS Writing Modes 4: vertical glyph selection and metrics from independent `HarfBuzz` samples.
use super::common;
use crate::layout::LayoutTrace;
use crate::text::GlyphOrientation;
use std::fmt::Write as _;

const SIZE_PT: f32 = 20.0;
const TOLERANCE: f32 = 1e-6;
const UNITS: f32 = 1000.0;

fn render(mode: &str, declarations: &str, body: &str) -> LayoutTrace {
    let mut options = common::local_options();
    options.base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fonts")
        .into();
    let html = format!(
        "<style>@font-face{{font-family:CJK;src:url(CJKWritingTest.otf)}}\
        @page{{size:240pt 180pt;margin:20pt}}html{{writing-mode:{mode}}}\
        body{{margin:0;font:{SIZE_PT}pt/1 CJK;{declarations}}}p{{margin:0}}</style>{body}"
    );
    crate::render_html(&html, &options)
        .unwrap_or_else(|error| panic!("{mode}/{declarations}: {error}"))
        .layout
        .unwrap()
}

#[test]
fn vertical_substitutions_origins_and_advances_match_the_pinned_font_samples() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/CJKWritingTest-source.json"
    ))
    .unwrap();
    for mode in ["vertical-rl", "vertical-lr"] {
        for ch in "漢字水あア「」（）〔〕、。Ａ".chars() {
            let trace = render(mode, "", &format!("<p>{ch}</p>"));
            let run = &trace.pages[0][0].runs[0];
            let actual = &run.glyphs()[0];
            let expected = &reference["samples"]["ttb"][ch.to_string()][0];
            assert_eq!(
                actual.id,
                expected["g"].as_u64().unwrap() as u32,
                "{mode}/{ch}: vertical glyph"
            );
            for (name, actual, expected) in [
                (
                    "inline advance",
                    actual.x_advance,
                    -(expected["ay"].as_f64().unwrap() as f32) / UNITS,
                ),
                (
                    "vertical origin",
                    actual.x_offset,
                    -(expected["dy"].as_f64().unwrap() as f32) / UNITS,
                ),
                (
                    "cross-axis origin",
                    actual.y_offset,
                    expected["dx"].as_f64().unwrap() as f32 / UNITS
                        * if mode == "vertical-rl" { -1.0 } else { 1.0 },
                ),
            ] {
                assert!(
                    (actual - expected).abs() <= TOLERANCE,
                    "{mode}/{ch}: {name}: expected {expected}, got {actual}"
                );
            }
            assert!(
                matches!(
                    run.orientation,
                    GlyphOrientation::UprightRl | GlyphOrientation::UprightLr
                ),
                "{mode}/{ch}: {:?}",
                run.orientation
            );
        }
    }
    let sample = &reference["samples"];
    assert_ne!(
        sample["ltr"]["「"][0]["g"], sample["ttb"]["「"][0]["g"],
        "the fixture must distinguish horizontal and vertical punctuation glyphs"
    );
}

#[test]
fn mixed_orientation_keeps_cjk_upright_and_turns_latin_clockwise_in_both_column_orders() {
    for mode in ["vertical-rl", "vertical-lr"] {
        let trace = render(mode, "", "<p>漢A字</p>");
        let runs = &trace.pages[0][0].runs;
        let expected = if mode == "vertical-rl" {
            [
                GlyphOrientation::UprightRl,
                GlyphOrientation::Horizontal,
                GlyphOrientation::UprightRl,
            ]
        } else {
            [
                GlyphOrientation::UprightLr,
                GlyphOrientation::ReflectedSideways,
                GlyphOrientation::UprightLr,
            ]
        };
        assert_eq!(
            runs.iter().map(|run| run.orientation).collect::<Vec<_>>(),
            expected,
            "{mode}: mixed text orientation"
        );
    }
}

#[test]
fn orthogonal_frames_keep_their_links_and_destinations_in_physical_page_coordinates() {
    let trace = render(
        "vertical-rl",
        "",
        "<p>漢<span style='writing-mode:horizontal-tb'><a id=target href='#target'>A</a></span>字</p>",
    );
    assert_eq!(
        trace.links[0].len(),
        1,
        "the orthogonal hyperlink must survive frame pagination"
    );
    let destination = trace
        .destinations
        .iter()
        .flatten()
        .next()
        .expect("the orthogonal id has a destination");
    assert_eq!(destination.page, 0);
    assert!(
        destination.top >= 20.0 && destination.top < 160.0,
        "physical destination: {destination:?}"
    );
}

#[test]
fn combined_fullwidth_characters_select_native_width_forms_before_geometric_compression() {
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/fonts/CJKWritingTest-source.json"
    ))
    .unwrap();
    for (count, feature) in [(2, "hwid"), (3, "twid"), (4, "qwid")] {
        let trace = render(
            "vertical-rl",
            "text-combine-upright:all",
            &format!("<p>{}</p>", "Ａ".repeat(count)),
        );
        let run = &trace.pages[0][0].runs[0];
        let expected = reference["samples"]["widths"][feature][0]["g"]
            .as_u64()
            .unwrap() as u32;
        if count == 2 {
            assert_ne!(
                expected,
                reference["samples"]["ltr"]["A"][0]["g"].as_u64().unwrap() as u32,
                "fixture must retain hwid substitution"
            );
        }
        assert!(
            run.glyphs().iter().all(|glyph| glyph.id == expected),
            "{count} characters must select the font's {feature} forms: {:?}",
            run.glyphs()
        );
        let advance = reference["samples"]["widths"][feature][0]["ax"]
            .as_f64()
            .unwrap() as f32
            / UNITS;
        let expected_scale = (1.0 / (count as f32 * advance)).min(1.0);
        assert!(
            (run.inline_scale - expected_scale).abs() <= TOLERANCE,
            "{feature}: native half-width forms precede compression; unavailable third/quarter forms need it: {}",
            run.inline_scale
        );
    }
}

#[test]
fn deep_native_flows_render_once_with_stacking_contexts_and_dynamic_positioned_jobs() {
    for (label, positioned, stacking) in [
        ("flow", false, false),
        ("stacking", false, true),
        ("positioned", true, false),
    ] {
        deep_native_flow(label, positioned, stacking, false);
    }
}

#[test]
fn deep_native_flows_inside_columns_render_once_on_the_default_stack() {
    deep_native_flow("columns", false, false, true);
}

fn deep_native_flow(label: &str, positioned: bool, stacking: bool, columns: bool) {
    const DEPTH: usize = 254;
    let mut body = String::new();
    for index in 0..DEPTH {
        let mode = if index % 2 == 0 {
            "vertical-rl"
        } else {
            "horizontal-tb"
        };
        let position = if positioned && index > 0 {
            "position:absolute;left:0;top:0;"
        } else if stacking {
            "position:relative;z-index:0;"
        } else {
            ""
        };
        let column_style = if columns { "column-count:1;" } else { "" };
        write!(
            body,
            "<section style='writing-mode:{mode};width:40pt;height:40pt;{position}{column_style}'>"
        )
        .unwrap();
        if positioned {
            body.push_str("<div style='position:relative;width:40pt;height:40pt'>");
        }
    }
    body.push_str("<p>A</p>");
    for _ in 0..DEPTH {
        if positioned {
            body.push_str("</div>");
        }
        body.push_str("</section>");
    }
    eprintln!("rendering {label}: {DEPTH} native boundaries");
    let trace = render(
        "horizontal-tb",
        "position:relative;width:100pt;height:100pt",
        &body,
    );
    let text: String = trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        text, "A",
        "{label}: complete native jobs must preserve the leaf once; provisional frame passes must never publish paint or text"
    );
    assert_eq!(
        trace.pages.len(),
        1,
        "{label}: nested fixed-size frames stay on one page"
    );
}

#[test]
fn implicit_columns_keep_their_content_and_trim_only_the_box_edges() {
    let lines = |declaration: &str| {
        let html = format!(
            "<style>section{{writing-mode:vertical-rl;width:30pt;{declaration}}}</style>\
             <section>A<br>B<br>C<br>D</section>"
        );
        crate::render_html(&html, &common::local_options())
            .unwrap_or_else(|error| panic!("{declaration}: {error}"))
            .layout
            .unwrap()
            .pages
            .concat()
            .into_iter()
            .map(|line| (line.text, line.x, line.baseline))
            .collect::<Vec<_>>()
    };
    let plain = lines("");
    assert_eq!(
        plain.len(),
        4,
        "two implicit columns of two lines: {plain:?}"
    );
    assert!(
        plain[2].1 > plain[0].1,
        "the third line starts a second column"
    );
    assert_eq!(
        lines("continue:discard"),
        plain,
        "an orthogonal flow's implicit columns are not overflow columns to discard"
    );
    let trimmed = lines("text-box-trim:trim-start");
    assert!(
        trimmed[0].2 < plain[0].2,
        "the box's first formatted line trims: {trimmed:?}"
    );
    assert_eq!(
        trimmed[2..],
        plain[2..],
        "implicit columns are not a multi-column container's columns to trim"
    );
}

#[test]
fn combined_emphasis_uses_one_object_replacement_character_for_skip_rules() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        for text in ["12", "ＡＡ"] {
            let body = format!("<p>{text}</p>");
            let plain = render(mode, "text-combine-upright:all", &body);
            let plain = &plain.pages[0][0];
            for (skip, expected) in [("spaces punctuation", 1), ("symbols", 0), ("narrow", 0)] {
                let trace = render(
                    mode,
                    &format!(
                        "text-combine-upright:all;text-emphasis:dot;text-emphasis-skip:{skip}"
                    ),
                    &body,
                );
                let line = &trace.pages[0][0];
                assert!(
                    line.runs.iter().any(|run| run.combined),
                    "{mode}/{text}: fixture must combine its characters"
                );
                let marks: usize = line
                    .runs
                    .iter()
                    .map(|run| run.marks.len() + run.mark_shapes.len())
                    .sum();
                assert_eq!(
                    marks, expected,
                    "{mode}/{text}/{skip}: one combined unit represents U+FFFC for emphasis eligibility"
                );
                assert!(
                    (line.width - plain.width).abs() <= TOLERANCE,
                    "{mode}/{text}/{skip}: emphasis leaves base advances unchanged"
                );
                if expected == 0 {
                    assert!(
                        (line.height - plain.height).abs() <= TOLERANCE,
                        "{mode}/{text}/{skip}: skipped marks must not grow line leading: {} vs {}",
                        line.height,
                        plain.height
                    );
                }
            }
        }
    }
}
