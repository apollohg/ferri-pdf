mod common;

use common::{assert_pt, local_options};
use ferri_pdf::{DisplayItem, FontPreferences, RenderOptions, render_bytes, render_html};

const FONT_SIZE_PT: f32 = 20.0;
const INITIAL_FONT_SIZE_PT: f32 = 12.0;
const BASE: &str =
    "@page{size:240pt 160pt;margin:10pt}html,body,p{margin:0;padding:0;font:20pt/24pt sans-serif}";

fn preferences() -> FontPreferences {
    FontPreferences {
        serif: vec!["DejaVu Serif".into()],
        sans_serif: vec!["DejaVu Sans".into()],
        monospace: vec!["DejaVu Sans Mono".into()],
        fallback: vec!["DejaVu Sans Mono".into()],
    }
}
fn options() -> RenderOptions {
    let mut options = local_options();
    options.print.fonts = preferences();
    options
}
fn reference() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/fonts/font-preferences-reference.json"
    ))
    .unwrap()
}

#[test]
fn generic_choices_reach_both_rendering_entry_points_and_glyph_metrics() {
    let reference = reference();
    for (generic, family) in [
        ("sans-serif", "DejaVu Sans"),
        ("serif", "DejaVu Serif"),
        ("monospace", "DejaVu Sans Mono"),
    ] {
        let html =
            format!("<!doctype html><style>{BASE}p{{font-family:{generic}}}</style><p>0</p>");
        for output in [
            render_html(&html, &options()).unwrap(),
            render_bytes(html.as_bytes(), &options()).unwrap(),
        ] {
            assert!(output.diagnostics.is_empty());
            let trace = output.layout.unwrap();
            let line = &trace.pages[0][0];
            assert_eq!(line.text, "0");
            assert_eq!(trace.fonts[line.runs[0].font_id.index()].family, family);
            let face = &reference["faces"][family];
            assert_pt(
                line.runs[0].width,
                (FONT_SIZE_PT * face["zero_advance"].as_f64().unwrap() as f32)
                    / face["units_per_em"].as_f64().unwrap() as f32,
            );
        }
    }
}

#[test]
fn preferred_fallback_only_replaces_missing_coverage_after_explicit_families() {
    let mut options = options();
    options.base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fonts")
        .into();
    let html = format!(
        "<!doctype html><style>{BASE}@font-face{{font-family:Primary;src:url(VariationSequenceTest.ttf)}}p{{font-family:Primary}}</style><p>A→A</p>"
    );
    let trace = render_html(&html, &options).unwrap().layout.unwrap();
    let line = &trace.pages[0][0];
    assert_eq!(line.text, "A→A");
    assert_eq!(
        line.runs
            .iter()
            .map(|run| (
                run.text.as_str(),
                trace.fonts[run.font_id.index()].family.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            ("A", "Primary"),
            ("→", "DejaVu Sans Mono"),
            ("A", "Primary")
        ]
    );
    let html = format!(
        "<!doctype html><style>{BASE}@font-face{{font-family:Primary;src:url(VariationSequenceTest.ttf)}}p{{font-family:Primary,'Liberation Serif'}}</style><p>A→A</p>"
    );
    let trace = render_html(&html, &options).unwrap().layout.unwrap();
    assert_eq!(
        trace.pages[0][0]
            .runs
            .iter()
            .map(|run| trace.fonts[run.font_id.index()].family.as_str())
            .collect::<Vec<_>>(),
        ["Primary", "Liberation Serif", "Primary"],
        "an explicit family with coverage precedes the configured installed fallback"
    );
}

#[test]
fn generic_choices_supply_element_relative_units_before_layout() {
    let html = format!(
        "<!doctype html><style>{BASE}div{{width:1ch;height:1ex;background:blue}}</style><div></div><p>0</p>"
    );
    let trace = render_html(&html, &options()).unwrap().layout.unwrap();
    let face = &reference()["faces"]["DejaVu Sans"];
    let paint = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == [0, 0, 255] => {
                Some(paint)
            }
            _ => None,
        })
        .unwrap();
    let unit = FONT_SIZE_PT / face["units_per_em"].as_f64().unwrap() as f32;
    assert_pt(
        paint.rect.width,
        unit * face["zero_advance"].as_f64().unwrap() as f32,
    );
    assert_pt(
        paint.rect.height,
        unit * face["x_height"].as_f64().unwrap() as f32,
    );
}

#[test]
fn initial_root_and_page_units_use_the_preferred_initial_serif() {
    let reference = reference();
    let face = &reference["faces"]["DejaVu Serif"];
    let scale = INITIAL_FONT_SIZE_PT / face["units_per_em"].as_f64().unwrap() as f32;
    for (unit, length) in [
        ("ex", face["x_height"].as_f64().unwrap() as f32),
        ("ch", face["zero_advance"].as_f64().unwrap() as f32),
        (
            "lh",
            (face["ascender"].as_f64().unwrap() - face["descender"].as_f64().unwrap()
                + face["line_gap"].as_f64().unwrap()) as f32,
        ),
    ] {
        let html = format!(
            "<!doctype html><style>@page{{size:240pt 160pt;margin:10pt}}html{{font-size:1{unit}}}body,p{{margin:0;padding:0}}</style><p>H</p>"
        );
        let trace = render_html(&html, &options()).unwrap().layout.unwrap();
        assert_pt(trace.pages[0][0].runs[0].font_size, scale * length);
    }
    let html = "<!doctype html><style>@page{size:240pt 160pt;font-size:1ex;margin:1em}html,body,p{margin:0;padding:0;font:40pt/40pt sans-serif}</style><p>H</p>";
    let trace = render_html(html, &options()).unwrap().layout.unwrap();
    assert_pt(
        trace.page_records[0].geometry.content_box().x,
        scale * face["x_height"].as_f64().unwrap() as f32,
    );
}

#[test]
fn media_font_units_follow_preferences_and_remain_independent_of_authored_fonts() {
    let mut options = options();
    options.print.paper = [700.0, 800.0];
    let html = "<!doctype html><style>@page{size:240pt 160pt;margin:0}html,body,p{margin:0;padding:0;font:100pt/100pt monospace;color:blue}@media(min-width:100ch){p{color:red}}</style><p>H</p>";
    let trace = render_html(html, &options).unwrap().layout.unwrap();
    assert_eq!(
        trace.pages[0][0].runs[0].color.rgb,
        [0, 0, 255],
        "DejaVu Serif's 100ch exceeds 700pt; authored font and page box are not the media basis"
    );
    options.print.fonts = Default::default();
    let trace = render_html(html, &options).unwrap().layout.unwrap();
    assert_eq!(
        trace.pages[0][0].runs[0].color.rgb,
        [255, 0, 0],
        "the unchanged default's 100ch is 600pt"
    );
}

#[test]
fn installed_preferences_do_not_expose_webfonts_or_bypass_resource_policy() {
    struct Denied;
    impl ferri_pdf::ResourceResolver for Denied {
        fn resolve(
            &self,
            reference: &str,
            _: &ferri_pdf::ResourceBase,
        ) -> Result<ferri_pdf::Resource, ferri_pdf::ResourceError> {
            Err(ferri_pdf::ResourceError::OutsideRoot(reference.into()))
        }
    }
    let html = format!(
        "<!doctype html><style>{BASE}@font-face{{font-family:'DejaVu Sans';src:url(denied.ttf)}}</style><p>H</p>"
    );
    let trace = ferri_pdf::render_with_resolver(&html, &options(), &Denied)
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.fonts[trace.pages[0][0].runs[0].font_id.index()].family,
        "DejaVu Sans"
    );
    let html = format!(
        "<!doctype html><style>{BASE}@font-face{{font-family:'DejaVu Sans';src:url(denied.ttf)}}p{{font-family:'DejaVu Sans'}}</style><p>H</p>"
    );
    assert!(
        matches!(
            ferri_pdf::render_with_resolver(&html, &options(), &Denied),
            Err(ferri_pdf::Error::Css(ferri_pdf::StyleError::Resource(
                ferri_pdf::ResourceError::OutsideRoot(_)
            )))
        ),
        "explicit CSS webfonts must still propagate the resolver's policy failure"
    );
}

#[test]
fn configured_generic_and_fallback_names_do_not_assign_private_use_meanings() {
    let policy: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/fonts/character-policy-reference.json"
    ))
    .unwrap();
    let replacement = policy["characters"]["replacement"]["glyph_id"]
        .as_u64()
        .unwrap() as u32;
    let html = format!("<!doctype html><style>{BASE}</style><p>\u{ef00}</p>");
    let trace = render_html(&html, &options()).unwrap().layout.unwrap();
    assert_eq!(trace.pages[0][0].text, "\u{ef00}");
    assert_eq!(
        trace.pages[0][0]
            .runs
            .iter()
            .flat_map(|run| run.glyphs().iter().map(|glyph| glyph.id))
            .collect::<Vec<_>>(),
        [replacement]
    );
}

#[test]
fn dynamic_media_math_and_conditions_share_the_preferred_initial_font() {
    const PAPER_WIDTH_PT: f32 = 700.0;
    const ENDPOINT_CH: f32 = 100.0;
    const TARGET_WIDTH_PT: f32 = 100.0;
    let mut options = options();
    options.print.paper = [PAPER_WIDTH_PT, 800.0];
    let face = &reference()["faces"]["DejaVu Serif"];
    let endpoint = INITIAL_FONT_SIZE_PT * face["zero_advance"].as_f64().unwrap() as f32
        / face["units_per_em"].as_f64().unwrap() as f32
        * ENDPOINT_CH;
    for (width, expected) in [
        (
            "calc(100pt * media-progress(width,0ch,100ch))",
            TARGET_WIDTH_PT * PAPER_WIDTH_PT / endpoint,
        ),
        ("if(media((min-width:100ch)):99pt;else:17pt)", 17.0),
    ] {
        let html = format!(
            "<!doctype html><style>{BASE}html{{font:100pt/100pt monospace}}div{{width:{width};height:1pt;background:blue}}</style><div></div>"
        );
        let trace = render_html(&html, &options).unwrap().layout.unwrap();
        let paint = trace.display_lists[0]
            .iter()
            .find_map(|item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.rgb == [0, 0, 255] => {
                    Some(paint)
                }
                _ => None,
            })
            .unwrap();
        assert_pt(paint.rect.width, expected);
    }
}

#[test]
fn font_preferences_are_job_local_and_drive_table_wrapping_and_pagination() {
    const TABLE_WIDTH_PT: f32 = 60.0;
    const DIGITS: usize = 5;
    let reference = reference();
    let html = format!(
        "<!doctype html><style>{BASE}@page{{size:240pt 100pt}}html,body,p,td{{font:20pt/20pt sans-serif;widows:1;orphans:1}}.lead{{height:50pt}}table{{width:{TABLE_WIDTH_PT}pt;border-spacing:0}}td{{padding:0;overflow-wrap:anywhere}}</style><p class=lead>Lead</p><table><tr><td>00000</td></tr></table>"
    );
    for (options, family, pages) in [
        (
            options(),
            "DejaVu Sans",
            vec![vec!["Lead", "0000"], vec!["0"]],
        ),
        (
            local_options(),
            "Liberation Sans",
            vec![vec!["Lead", "00000"]],
        ),
    ] {
        let trace = render_html(&html, &options).unwrap().layout.unwrap();
        let words = trace
            .pages
            .iter()
            .map(|page| {
                page.iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            words, pages,
            "{family} must control actual cell line breaks and physical pages"
        );
        let face = &reference["faces"][family];
        let digit_width = FONT_SIZE_PT * face["zero_advance"].as_f64().unwrap() as f32
            / face["units_per_em"].as_f64().unwrap() as f32;
        assert_eq!(
            DIGITS as f32 * digit_width > TABLE_WIDTH_PT,
            family == "DejaVu Sans",
            "the independent source advances straddle the authored table width"
        );
        let lines = trace
            .pages
            .iter()
            .flatten()
            .filter(|line| line.text.starts_with('0'))
            .collect::<Vec<_>>();
        assert_eq!(
            lines.iter().map(|line| line.text.len()).sum::<usize>(),
            DIGITS
        );
        for line in lines {
            assert_pt(line.runs[0].width, line.text.len() as f32 * digit_width);
            assert_eq!(trace.fonts[line.runs[0].font_id.index()].family, family);
        }
    }
}
