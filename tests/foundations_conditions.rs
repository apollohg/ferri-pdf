mod common;
use std::fmt::Write;

use common::{assert_pt, local_options};
use ferri_pdf::{
    ColorScheme, ContrastPreference, MotionPreference, PrintSettings, StyleError, render_html,
};

const PAPER: [f32; 2] = [600.0, 800.0];
const MATCHED_SIZE: f32 = 19.0;
const UNMATCHED_SIZE: f32 = 12.0;

fn query_size(query: &str, settings: PrintSettings) -> f32 {
    condition_size(&format!("media {query}"), settings)
}

fn condition_size(condition: &str, settings: PrintSettings) -> f32 {
    let mut options = local_options();
    options.print = settings;
    let profile = common::PROFILE;
    let html = format!(
        "<!doctype html><style>{profile}@page{{size:200pt 300pt;margin:0}}html{{font-size:99pt}}body,p{{margin:0}}p{{font-size:12pt}}@{condition}{{p{{font-size:19pt}}}}</style><p>Condition</p>"
    );
    render_html(&html, &options).unwrap().layout.unwrap().pages[0][0].font_size
}

#[test]
fn supports_requires_usable_behavior_not_only_known_grammar() {
    for (declaration, expected) in [
        ("display:flex", true),
        ("display:ruby", false),
        ("display:grid", true),
        ("display:inline-grid", true),
        ("width:12pt", true),
        ("width:12unknown", false),
        ("width:min-content", true),
        ("width:fit-content(10pt)", false),
        ("float:left", true),
        ("float:inline-end", true),
        ("clear:both", true),
        ("float:top", false),
        ("hyphens:auto", false),
        ("hyphens:manual", true),
        ("tab-size:2pt", false),
        ("tab-size:8", true),
        ("font-style:oblique 14deg", true),
        ("font-style:oblique 20deg", true),
        ("text-transform:uppercase full-width", true),
        ("background-image:linear-gradient(red,blue)", true),
        (
            "background-image:linear-gradient(red,blue),conic-gradient(red,blue)",
            true,
        ),
        (
            "background-image:linear-gradient(red,blue),element(#source)",
            false,
        ),
        ("color:var(--ink)", true),
        ("unknown-property:var(--ink)", false),
    ] {
        let actual = condition_size(&format!("supports ({declaration})"), Default::default());
        assert_eq!(
            actual,
            if expected {
                MATCHED_SIZE
            } else {
                UNMATCHED_SIZE
            },
            "{declaration}"
        );
    }
    let positive = "<!doctype html><style>@supports (width:120pt) and (display:flex){p{width:120pt;display:flex;color:#123456;background:#123456}}</style><p>Applied</p>";
    let output = render_html(positive, &local_options()).unwrap();
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages[0][0].runs[0].color.rgb, [0x12, 0x34, 0x56]);
    let paint = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            ferri_pdf::DisplayItem::Box(paint)
                if paint.decoration.background.rgb == [0x12, 0x34, 0x56] =>
            {
                Some(paint)
            }
            _ => None,
        })
        .expect("supported flex width and background reach the display list");
    assert_pt(paint.rect.width, 120.0);
    assert!(matches!(
        render_html(
            "<style>@supports (width:12pt){p{display:ruby}}</style><p>Unsupported</p>",
            &local_options()
        ),
        Err(ferri_pdf::Error::Css(StyleError::Unsupported(_)))
    ));
}

#[test]
fn supports_has_no_resource_or_font_side_effects() {
    use ferri_pdf::{
        Resource, ResourceBase, ResourceError, ResourceResolver, render_with_resolver,
    };
    use std::cell::Cell;
    struct QueryResources {
        calls: Cell<usize>,
    }
    impl ResourceResolver for QueryResources {
        fn resolve(
            &self,
            _reference: &str,
            base: &ResourceBase,
        ) -> Result<Resource, ResourceError> {
            self.calls.set(self.calls.get() + 1);
            Ok(Resource {
                bytes: Vec::new(),
                base: base.clone(),
            })
        }
    }
    let resources = QueryResources {
        calls: Cell::new(0),
    };
    let html = "<!doctype html><style>p{font-family:Probe}@supports-condition --probe{@font-face{font-family:Probe;src:url('probe-font.ttf')}@page{size:100pt 200pt}p{background-image:url('probe-only.png')}}@font-face{font-family:Unused;src:url('unused-font.ttf')}@supports (background-image:url('query-only.png')) and (font-family:Unused){p{font-size:19pt}}@supports (display:ruby){p{background-image:url('inactive.png');font-family:Unused}}</style><p>Query</p>";
    let output = render_with_resolver(html, &local_options(), &resources).unwrap();
    assert_eq!(
        resources.calls.get(),
        0,
        "support queries cannot resolve queried URLs or font faces"
    );
    let trace = output.layout.unwrap();
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
    assert_eq!(
        [trace.page_width, trace.page_height],
        PrintSettings::default().paper,
        "a probe cannot apply page descriptors"
    );
}

#[test]
fn supports_compounds_custom_properties_and_general_enclosed() {
    for (condition, expected) in [
        ("(--flag:)", true),
        ("(--:red)", false),
        ("(--flag:{arbitrary:tokens})", true),
        ("(width:12pt!important)", true),
        ("(width:12pt;)", false),
        ("(width:12pt !nope)", false),
        ("not future-function()", true),
        ("future-function()", false),
        ("not (--unrecognized)", true),
        ("(width:12pt) and (display:ruby) or (color:red)", false),
        ("((width:12pt) and (display:ruby)) or (color:red)", true),
        ("selector(p > .notice)", true),
        ("selector(p,q)", false),
        ("selector(:is(p,:not-a-pseudo))", false),
        ("selector(:where(p,:not-a-pseudo))", false),
        ("selector(:host(p))", true),
        ("selector(:host)", true),
        ("selector(:host-context(body))", true),
        ("selector(:host-context(body div))", false),
        ("selector(p::before)", true),
        ("selector(p::not-a-pseudo)", false),
    ] {
        let actual = condition_size(&format!("supports {condition}"), Default::default());
        assert_eq!(
            actual,
            if expected {
                MATCHED_SIZE
            } else {
                UNMATCHED_SIZE
            },
            "{condition}"
        );
    }
}

#[test]
fn support_conditions_register_only_applicable_layers() {
    for (condition, expected) in [("(display:ruby)", 19.0), ("(width:12pt)", 23.0)] {
        let html = format!(
            "<!doctype html><style>@supports {condition}{{@layer first;}}@layer second,first;@layer second{{p{{--size:23pt}}}}@layer first{{p{{--size:19pt}}}}p{{font-size:var(--size)}}</style><p>Layers</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_pt(trace.pages[0][0].font_size, expected);
    }
}

#[test]
fn named_support_conditions_and_when_else_choose_the_first_matching_branch() {
    let html = "<!doctype html><style>p{font-size:12pt}@supports-condition --usable{width:12pt;color:#123456;}@supports-condition --unusable{display:ruby;}@when supports(--unusable){p{font-size:31pt}}@else media(width > 1px) and supports(--usable){p{font-size:19pt;color:#123456}}@else{p{font-size:23pt}}@supports (--usable){p{margin-left:7pt}}</style><p>Branch</p>";
    let trace = render_html(html, &local_options()).unwrap().layout.unwrap();
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
    assert_eq!(trace.pages[0][0].runs[0].color.rgb, [0x12, 0x34, 0x56]);
}

#[test]
fn extended_font_and_at_rule_queries_reflect_the_rendering_pipeline() {
    for (condition, expected) in [
        ("font-format(truetype)", true),
        ("font-format(opentype)", true),
        ("font-format(truetype-variations)", false),
        ("font-format(woff2)", false),
        ("font-format('truetype')", false),
        ("font-tech(features-opentype)", true),
        ("font-tech(features-aat)", true),
        ("font-tech(color-COLRv0)", true),
        ("font-tech(color-COLRv1)", true),
        ("font-tech(color-sbix)", true),
        ("font-tech(color-CBDT)", true),
        ("font-tech(color-SVG)", false),
        ("font-tech(palettes)", true),
        ("font-tech(features-graphite)", false),
        ("font-tech(variations)", true),
        ("font-tech(incremental)", false),
        ("at-rule(@media)", true),
        ("at-rule(@supports)", true),
        ("at-rule(@layer)", true),
        ("at-rule(@charset)", false),
        ("at-rule(@container)", false),
    ] {
        assert_eq!(
            condition_size(&format!("supports {condition}"), Default::default()),
            if expected {
                MATCHED_SIZE
            } else {
                UNMATCHED_SIZE
            },
            "{condition}"
        );
    }
}

#[test]
fn invalid_custom_media_feature_use_does_not_create_a_dependency_cycle() {
    let html = "<!doctype html><style>@custom-media --invalid (--invalid:unsupported);@media not (--invalid){p{font-size:19pt}}</style><p>Invalid feature</p>";
    let trace = render_html(html, &local_options()).unwrap().layout.unwrap();
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
}

#[test]
fn media_queries_use_paper_not_author_page() {
    for query in [
        "print and (width:800px) and (height:800pt)",
        "(599pt < width <= 600pt)",
        "(800pt >= height > 799pt)",
        "(aspect-ratio:3/4)",
        "(device-width:600pt) and (device-height:800pt)",
        "(orientation:portrait)",
        "(min-width:49em)",
    ] {
        assert_pt(
            query_size(
                query,
                PrintSettings {
                    paper: PAPER,
                    ..Default::default()
                },
            ),
            MATCHED_SIZE,
        );
    }
    assert_pt(
        query_size(
            "(width:200pt)",
            PrintSettings {
                paper: PAPER,
                ..Default::default()
            },
        ),
        UNMATCHED_SIZE,
    );
    let mut options = local_options();
    options.print.paper = PAPER;
    let html = "<!doctype html><style>@page{size:200pt 300pt;margin:0}@media print{@page{size:250pt 350pt}}@media (width:600pt){@layer page{@page{size:400pt 500pt!important;margin:7pt}}}</style><p>Text</p>";
    let trace = render_html(html, &options).unwrap().layout.unwrap();
    assert_eq!(
        [trace.page_width, trace.page_height],
        [250.0, 350.0],
        "a paper-qualified size must be ignored, while unqualified media types may set size"
    );
}

#[test]
fn vector_resolution_is_infinite() {
    for (query, matched) in [
        ("(resolution:infinite)", true),
        ("(resolution>1000000dpi)", true),
        ("(min-resolution:1000dpcm)", true),
        ("(resolution:96dpi)", false),
        ("(resolution<infinite)", false),
        ("not (resolution:-1dppx)", true),
    ] {
        assert_pt(
            query_size(query, Default::default()),
            if matched {
                MATCHED_SIZE
            } else {
                UNMATCHED_SIZE
            },
        );
    }
}

#[test]
fn configured_paper_controls_auto_and_orientation() {
    for (size, expected) in [
        ("", PAPER),
        ("size:auto", PAPER),
        ("size:portrait", PAPER),
        ("size:landscape", [800.0, 600.0]),
        ("size:letter", [612.0, 792.0]),
        ("size:240pt 320pt", [240.0, 320.0]),
    ] {
        let mut options = local_options();
        options.print.paper = PAPER;
        let html = format!("<!doctype html><style>@page{{margin:0;{size}}}</style><p>Paper</p>");
        let output = render_html(&html, &options).unwrap();
        let trace = output.layout.unwrap();
        assert_eq!([trace.page_width, trace.page_height], expected, "{size}");
    }
    for paper in [
        [0.0, 800.0],
        [-1.0, 800.0],
        [600.0, f32::INFINITY],
        [f32::NAN, 800.0],
    ] {
        let mut options = local_options();
        options.print.paper = paper;
        assert!(
            matches!(
                render_html("<p>Invalid</p>", &options),
                Err(ferri_pdf::Error::Css(
                    StyleError::InvalidPrintSettings { .. }
                ))
            ),
            "invalid paper {paper:?}"
        );
    }
}

#[test]
fn print_capabilities_preferences_and_unknown_features() {
    for (query, matched) in [
        (
            "(color:8) and (color-gamut:srgb) and (dynamic-range:standard)",
            true,
        ),
        (
            "(update:none) and (scripting:none) and (pointer:none) and (hover:none)",
            true,
        ),
        ("(overflow-block:paged) and (overflow-inline:none)", true),
        ("(prefers-color-scheme:light)", true),
        ("not (future-feature)", false),
        ("not future-device", true),
        ("(future-feature) or (color)", true),
        ("not ((future-feature) and (monochrome))", true),
        ("(color) and (hover) or (width)", false),
        ("only (color)", false),
        ("screen, print and (color)", true),
    ] {
        assert_pt(
            query_size(query, Default::default()),
            if matched {
                MATCHED_SIZE
            } else {
                UNMATCHED_SIZE
            },
        );
    }
    let mut settings = PrintSettings::default();
    settings.preferences.color_scheme = ColorScheme::Dark;
    settings.preferences.contrast = ContrastPreference::More;
    settings.preferences.reduced_motion = MotionPreference::Reduce;
    assert_pt(
        query_size(
            "(prefers-color-scheme:dark) and (prefers-contrast:more) and (prefers-reduced-motion:reduce)",
            settings,
        ),
        MATCHED_SIZE,
    );
}

#[test]
fn inactive_media_has_no_declaration_or_layer_registration_side_effects() {
    for (condition, expected) in [("screen", 19.0), ("print", 23.0)] {
        let html = format!(
            "<!doctype html><style>@page{{margin:0}}@media {condition}{{@layer first;}}@layer second,first;@layer second{{p{{--size:23pt}}}}@layer first{{p{{--size:19pt}}}}p{{font-size:var(--size)}}</style><p>Layer</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_pt(trace.pages[0][0].font_size, expected);
    }
    let html = "<!doctype html><style>@media screen{@page:first{size:1pt}p{display:ruby;background-image:url('missing.png')}}@media print{p{font-size:19pt}}</style><p>Active</p>";
    let output = render_html(html, &local_options())
        .expect("inactive valid unsupported values do not apply");
    assert_pt(output.layout.unwrap().pages[0][0].font_size, MATCHED_SIZE);
}

#[test]
fn custom_media_is_logical_forward_resolved_and_cycle_safe() {
    let html = "<!doctype html><style>@page{margin:0}p{font-size:12pt}@media (--later) and (color){p{font-size:19pt}}@custom-media --later (--color), (hover);@custom-media --color true;@custom-media --cycle-a (--cycle-b);@custom-media --cycle-b (--cycle-a);@media not (--cycle-a){p{font-size:31pt}}@custom-media --override false;@custom-media --override true;@media (--override){p{color:#123456}}</style><p>Alias</p>";
    let trace = render_html(html, &local_options()).unwrap().layout.unwrap();
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
    assert_eq!(trace.pages[0][0].runs[0].color.rgb, [0x12, 0x34, 0x56]);
}

#[test]
fn conditional_alias_definitions_obey_ancestry_and_keep_paper_dependencies() {
    let mut options = local_options();
    options.print.paper = PAPER;
    let html = "<!doctype html><style>@page{size:250pt 350pt;margin:0}p{font-size:12pt}@custom-media --gate true;@custom-media --size (width:600pt);@media screen{@custom-media --gate false;@media (--self){@custom-media --self true}}@media (--gate){@custom-media --active true;@media (--size){@page{size:400pt 500pt!important}}}@media (--active){p{font-size:19pt}}@custom-media --Flag true;@media (--flag){p{font-size:31pt}}</style><p>Conditional</p>";
    for output in [
        render_html(html, &options).unwrap(),
        ferri_pdf::render_bytes(html.as_bytes(), &options).unwrap(),
    ] {
        let trace = output.layout.unwrap();
        assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
        assert_eq!(
            [trace.page_width, trace.page_height],
            [250.0, 350.0],
            "a custom alias retains the paper dependency of its definition"
        );
    }
}

#[test]
fn custom_media_alias_chains_are_bounded_by_input_without_recursive_evaluation() {
    const ALIASES: usize = 2_000;
    let mut css = String::from("@page{margin:0}@media (--alias0){p{font-size:19pt}}");
    for index in 0..ALIASES {
        let _ = write!(css, "@custom-media --alias{index} (--alias{});", index + 1);
    }
    let _ = write!(css, "@custom-media --alias{ALIASES} true;");
    let trace = render_html(
        &format!("<!doctype html><style>{css}</style><p>Chain</p>"),
        &local_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
}

#[test]
fn named_paper_sizes_include_japanese_iso_and_portrait_ledger() {
    for (size, millimetres) in [("JIS-B4", [257.0, 364.0]), ("JIS-B5", [182.0, 257.0])] {
        let html =
            format!("<!doctype html><style>@page{{size:{size};margin:0}}</style><p>Named</p>");
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        const POINTS_PER_MILLIMETRE: f32 = 72.0 / 25.4;
        assert_pt(trace.page_width, millimetres[0] * POINTS_PER_MILLIMETRE);
        assert_pt(trace.page_height, millimetres[1] * POINTS_PER_MILLIMETRE);
    }
    let trace = render_html(
        "<!doctype html><style>@page{size:ledger;margin:0}</style><p>Ledger</p>",
        &local_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    assert_eq!([trace.page_width, trace.page_height], [792.0, 1224.0]);
}

#[test]
fn media_paper_units_and_document_page_area_units_remain_distinct() {
    let mut options = local_options();
    options.print.paper = PAPER;
    let html = "<!doctype html><style>@page{size:400pt 600pt;margin:50pt 20pt}body,p{margin:0}p{height:20pt}@media (width:600pt){p{width:100vw;background:#123456;font-size:19pt}@custom-media --paper-qualified true;}@media (color:calc(100vw / 75pt)){@page{size:200pt 300pt!important}}@media (--paper-qualified){@page{size:250pt 350pt!important}}</style><p>Viewport</p>";
    let trace = render_html(html, &options).unwrap().layout.unwrap();
    assert_eq!([trace.page_width, trace.page_height], [400.0, 600.0]);
    assert_pt(trace.page_area.width, 360.0);
    let paint = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            ferri_pdf::DisplayItem::Box(paint)
                if paint.decoration.background.rgb == [0x12, 0x34, 0x56] =>
            {
                Some(paint)
            }
            _ => None,
        })
        .expect("media-selected background is painted");
    assert_pt(paint.rect.width, 360.0);
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
}

#[test]
fn custom_media_parse_recovery_keeps_the_last_valid_definition() {
    let html = "<!doctype html><style>@page{margin:0}body{margin:0}@custom-media -- true;@custom-media --color/* */(color);@custom-media --keep true;@custom-media --keep false true;@custom-media --keep() false;@custom-media not-an-extension false;@media (--) and (--color) and (--keep){p{font-size:19pt}}@custom-media --false false;@media not (--false){p{color:#123456}}@custom-media --undefined-copy (--missing);@media not (--undefined-copy){p{margin-left:7pt}}</style><p>Recovery</p>";
    let output = render_html(html, &local_options()).unwrap();
    assert_eq!(
        output.diagnostics.len(),
        3,
        "three malformed custom definitions are discarded"
    );
    let trace = output.layout.unwrap();
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
    assert_pt(trace.pages[0][0].x, 7.0);
    assert_eq!(trace.pages[0][0].runs[0].color.rgb, [0x12, 0x34, 0x56]);
}

#[test]
fn unknown_media_features_do_not_create_paper_dependencies() {
    for (condition, expected) in [
        ("(min-orientation:portrait) or (color)", [300.0, 400.0]),
        ("(future-feature:100vw) or (color)", [300.0, 400.0]),
        ("(width:1pt) or (color)", [250.0, 350.0]),
    ] {
        let html = format!(
            "<!doctype html><style>@page{{size:250pt 350pt;margin:0}}@media {condition}{{@page{{size:300pt 400pt}}}}</style><p>Unknown</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            [trace.page_width, trace.page_height],
            expected,
            "{condition}"
        );
    }
}

#[test]
fn alias_selection_preserves_paper_dependencies_of_skipped_candidates() {
    for css in [
        "@custom-media --choice true;@media (width:1pt){@custom-media --choice false;}@media (--choice){@page{size:300pt 400pt}}",
        "@media (width:1pt){@custom-media --choice true;}@media (--choice) or (color){@page{size:300pt 400pt}}",
    ] {
        let html = format!(
            "<!doctype html><style>@page{{size:250pt 350pt;margin:0}}{css}</style><p>Selection</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            [trace.page_width, trace.page_height],
            [250.0, 350.0],
            "{css}"
        );
    }
}

#[test]
fn else_chains_obey_source_adjacency_and_parent_conditions() {
    for (prefix, gap, matched) in [
        ("@supports (display:ruby){}", " /* retained */ \n", true),
        ("@media screen{}", "", true),
        ("@media print{}", "", false),
        ("@when media(future-feature){}", "", true),
        ("@supports (display:ruby){}", ";", false),
        ("@supports (display:ruby){}", "<!-- -->", false),
        ("@supports (display:ruby){}", "@unknown{}", false),
        ("@supports (display:ruby){}", "@layer intervening;", false),
        ("@supports (display:ruby){}", "p{}", false),
        ("", "", false),
    ] {
        let html = format!(
            "<!doctype html><style>p{{font-size:12pt}}{prefix}{gap}@else{{p{{font-size:19pt}}}}@else supports(width:12pt){{p{{font-size:31pt}}}}</style><p>Adjacency</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            trace.pages[0][0].font_size,
            if matched {
                MATCHED_SIZE
            } else {
                UNMATCHED_SIZE
            },
            "{prefix} {gap:?}"
        );
    }
    let html = "<!doctype html><style>p{font-size:12pt}@media screen{@supports (display:ruby){}@else{p{font-size:31pt}}}@supports (width:12pt){@supports (display:ruby){}@else{p{font-size:19pt}}}@else{p{font-size:23pt}}</style><p>Nested</p>";
    let trace = render_html(html, &local_options()).unwrap().layout.unwrap();
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
}

#[test]
fn named_conditions_probe_all_features_without_applying_their_contents() {
    for (probe, expected) in [
        ("color:red; width:12pt;", true),
        ("p, p::before{color:red} @media screen{p{width:12pt}}", true),
        ("@media screen{p{display:ruby}}", false),
        ("p:is(p,:unknown){width:12pt}", false),
        ("&{color:red}", true),
        ("@container scroll-state(stuck:top){}", false),
        (
            "@font-face{font-family:Probe;src:url('must-not-fetch.ttf')}",
            true,
        ),
        ("@page{size:A4;@top-center{content:'Probe'}}", true),
        ("@page:first{size:A4}", true),
        ("@page chapter:left{size:A4}", true),
        ("@page:unknown{size:A4}", false),
        (
            "@font-face{font-family:Probe;src:url('font.ttf');font-stretch:condensed}",
            true,
        ),
        ("unknown:1;", false),
        ("width:12pt!important;", true),
        ("width:12pt!bad;", false),
    ] {
        let html = format!(
            "<!doctype html><style>@page{{size:{width}pt {height}pt}}body,p{{margin:0}}p{{font-size:12pt}}@supports-condition --probe{{{probe}}}@supports (--probe){{p{{font-size:19pt}}}}</style><p>Probe</p>",
            width = PAPER[0],
            height = PAPER[1],
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            trace.pages[0][0].font_size,
            if expected {
                MATCHED_SIZE
            } else {
                UNMATCHED_SIZE
            },
            "{probe}"
        );
        assert_eq!(
            trace.pages[0].len(),
            1,
            "a probe cannot generate page content"
        );
        assert_eq!(
            trace.pages[0][0].runs[0].color.rgb,
            [0, 0, 0],
            "a probe cannot apply its declaration values"
        );
        assert_eq!(
            [trace.page_width, trace.page_height],
            PAPER,
            "a probe cannot apply its page rules"
        );
    }
}

#[test]
fn names_and_branches_share_dependency_resolution_and_paper_metadata() {
    let html = "<!doctype html><style>@page{size:250pt 350pt;margin:0}p{font-size:12pt}@custom-media --usable false;@supports (--later){@custom-media --usable true;}@media (--usable){p{font-size:19pt}}@supports-condition --later{width:12pt;}@supports-condition --later{width:19pt;}@media (width:1pt){@supports-condition --later{display:ruby;}}@when media(width:1pt){}@else supports(--later){@supports-condition --paper{color:red;}}@supports (--paper){@page{size:400pt 500pt}p{color:#123456}}@supports (--cycle){@custom-media --cycle true;}@media (--cycle){@supports-condition --cycle{width:12pt;}}@supports (--cycle){p{font-size:31pt}}</style><p>Names</p>";
    let trace = render_html(html, &local_options()).unwrap().layout.unwrap();
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
    assert_eq!(trace.pages[0][0].runs[0].color.rgb, [0x12, 0x34, 0x56]);
    assert_eq!(
        [trace.page_width, trace.page_height],
        [250.0, 350.0],
        "named supports must retain skipped gates and preceding branches' paper dependencies"
    );
}

#[test]
fn comment_only_else_conditions_are_unconditional() {
    let html = "<!doctype html><style>p{font-size:12pt}@supports (display:ruby){}@else /* no condition */ {p{font-size:19pt}}@else{p{font-size:31pt}}</style><p>Comments</p>";
    let trace = render_html(html, &local_options()).unwrap().layout.unwrap();
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
}

#[test]
fn when_supports_operands_follow_the_selected_declaration_grammar() {
    for (condition, expected) in [
        ("supports(width:12pt)", true),
        ("not supports(display:ruby)", true),
        ("supports(font-format(truetype))", false),
        ("supports(selector(p))", false),
        ("not supports(selector(:unknown))", false),
        ("font-tech(features-opentype)", false),
        (
            "media(width > 1pt) and (supports(width:12pt) or supports(display:ruby))",
            true,
        ),
    ] {
        assert_eq!(
            condition_size(&format!("when {condition}"), Default::default()),
            if expected {
                MATCHED_SIZE
            } else {
                UNMATCHED_SIZE
            },
            "{condition}"
        );
    }
}

#[test]
fn named_descriptor_probes_check_usable_sources_and_pending_page_values() {
    for (probe, expected) in [
        (
            "@font-face{font-family:Probe;src:url('missing.woff2') format('woff2')}",
            false,
        ),
        (
            "@font-face{font-family:Probe;src:url('missing.ttf') format('truetype')}",
            true,
        ),
        ("@page{size:var(--paper)}", true),
        ("@page{size:initial}", true),
        ("@page{size:var(--)}", false),
    ] {
        let html = format!(
            "<!doctype html><style>p{{font-size:12pt}}@supports-condition --probe{{{probe}}}@supports (--probe){{p{{font-size:19pt}}}}</style><p>Descriptors</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            trace.pages[0][0].font_size,
            if expected {
                MATCHED_SIZE
            } else {
                UNMATCHED_SIZE
            },
            "{probe}"
        );
    }
}

#[test]
fn named_condition_definitions_can_start_chains_and_later_definitions_win() {
    for (body, expected) in [
        ("width:12pt;", UNMATCHED_SIZE),
        ("display:ruby;", MATCHED_SIZE),
    ] {
        let html = format!(
            "<!doctype html><style>p{{font-size:12pt}}@supports-condition --start{{{body}}}@else{{p{{font-size:19pt}}}}</style><p>Named chain</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_pt(trace.pages[0][0].font_size, expected);
    }
    let html = "<!doctype html><style>p{font-size:12pt}@supports (--last){p{font-size:31pt}}@supports not (--last){p{font-size:19pt}}@supports-condition --last{width:12pt;}@supports-condition --last{display:ruby;}</style><p>Last definition</p>";
    let trace = render_html(html, &local_options()).unwrap().layout.unwrap();
    assert_pt(trace.pages[0][0].font_size, MATCHED_SIZE);
}
