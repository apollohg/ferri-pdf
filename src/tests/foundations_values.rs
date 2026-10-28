use super::common;
use crate::dom::{DocumentTree, Selector};
use crate::resources::{FileResolver, ResourceResolver};
use crate::style::{LengthPercentage, Size, Style, StyleError, Stylesheet, StylesheetSource};
use std::fmt::Write;

const FONT_SIZE_PT: f32 = 20.0;
const SERIF_ZERO_PT: f32 = 10.0;
const FIXTURE_ZERO_PT: f32 = 12.0;
const MEDIA_PAPER_PT: [f32; 2] = [600.0, 800.0];

fn font_base() -> crate::ResourceBase {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fonts")
        .into()
}

fn computed(css: &str) -> Style {
    computed_with(css, &FileResolver::default())
        .unwrap_or_else(|error| panic!("CSS {css}: {error}"))
}

fn computed_with(css: &str, resolver: &dyn ResourceResolver) -> Result<Style, StyleError> {
    computed_in(
        css,
        "<div id=parent><p id=target>0</p></div>",
        "#target",
        &crate::style::PrintSettings::default(),
        resolver,
    )
}

fn computed_in(
    css: &str,
    html: &str,
    selector: &str,
    settings: &crate::style::PrintSettings,
    resolver: &dyn ResourceResolver,
) -> Result<Style, StyleError> {
    let sheet = Stylesheet::parse_with_settings(
        &[StylesheetSource {
            text: css.into(),
            base: font_base(),
        }],
        &[],
        &font_base(),
        settings,
    )
    .unwrap();
    let tree = DocumentTree::parse(html).unwrap();
    let element = tree
        .select(&Selector::parse(selector).unwrap())
        .next()
        .unwrap();
    let mut ancestors = element
        .ancestors(crate::dom::TreeRelation::Inheritance)
        .collect::<Vec<_>>();
    ancestors.reverse();
    let mut style = Style::default();
    for element in ancestors.into_iter().chain([element]) {
        style = sheet.computed_with_resolver(element, &style, resolver)?;
    }
    Ok(style)
}

fn points(value: &Size) -> f32 {
    match value {
        Size::Value(LengthPercentage::Length(points)) => *points,
        other => panic!("expected a computed absolute size, got {other:?}"),
    }
}

fn media_computed(css: &str, resolver: &dyn ResourceResolver) -> Style {
    computed_in(
        css,
        "<div id=parent><p id=target>0</p></div>",
        "#target",
        &crate::style::PrintSettings {
            paper: MEDIA_PAPER_PT,
            ..Default::default()
        },
        resolver,
    )
    .unwrap()
}

#[test]
fn attribute_syntax_type_keywords_are_ascii_case_insensitive() {
    let style = computed_in(
        "#target{width:13pt;width:attr(data-size <LENGTH>,99pt);color:attr(data-color '<CoLoR>');transform:attr(data-transform <TRANSFORM-LIST>)}",
        "<p id=target data-size='17pt' data-color='red' data-transform='translateX(5pt) scale(2)'>Attribute</p>",
        "#target", &Default::default(), &FileResolver::default(),
    ).unwrap();
    common::assert_pt(points(&style.width), 17.0);
    assert_eq!(style.color.rgb, [255, 0, 0]);
    assert_eq!(style.transform.len(), 2);
}

#[test]
fn attribute_tokens_substitute_in_context_and_keep_relative_values_for_the_receiver() {
    let style = computed_in(
        "#parent{font-size:10pt;--size:attr(data-size <length>)}#target{font-size:20pt;--seed:17pt;width:var(--size);height:attr(data-expression <length>,99pt);margin-left:attr(data-invalid <length>,3pt);background-image:attr(data-image <image>)}",
        "<div id=parent data-size='1em'><p id=target data-expression='if(media(print):calc(var(--seed) + attr(data-extra &lt;length>));else:99pt)' data-extra='2pt' data-invalid='var(--missing)' data-image='linear-gradient(red,blue)'>Attribute</p></div>",
        "#target", &Default::default(), &FileResolver::default(),
    ).unwrap();
    common::assert_pt(points(&style.width), 20.0);
    common::assert_pt(points(&style.height), 19.0);
    common::assert_pt(points(&style.margin[3]), 3.0);
    assert!(
        style.decoration.backgrounds.has_images(),
        "a typed gradient must pass through the shared image grammar"
    );
}

#[test]
fn attribute_typed_strings_parse_css_escapes_and_custom_ident_rejects_reserved_values() {
    let trace = common::render_trace(
        r#"<style>@page{size:400pt 200pt;margin:10pt}body,p{margin:0;font:10pt/12pt serif}p::before{content:attr(data-label <string>) '|' attr(data-label)}</style><p data-label="'A\41'">Body</p>"#,
    );
    assert_eq!(trace.pages[0][0].text, r"AA|'A\41'Body");
    for (value, expected) in [("marker", 17.0), ("initial", 3.0), ("default", 3.0)] {
        let style = computed_in(
            "#target{--label:attr(data-label <custom-ident>,missing);width:if(style(--label:marker):17pt;else:3pt)}",
            &format!("<p id=target data-label='{value}'>Attribute</p>"),
            "#target", &Default::default(), &FileResolver::default(),
        ).unwrap();
        common::assert_pt(points(&style.width), expected);
    }
}

#[test]
fn attribute_recursion_preserves_the_shared_resource_error_instead_of_using_fallback() {
    let error = computed_in(
        "#target{width:attr(data-size <length>,17pt)}",
        "<p id=target data-size='attr(data-size &lt;length>,19pt)'>Attribute</p>",
        "#target",
        &Default::default(),
        &FileResolver::default(),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            StyleError::VariableLimit {
                resource: "value nesting",
                ..
            }
        ),
        "equal-size recursive DOM substitutions must remain bounded and keep their typed error: {error:?}"
    );
}

#[test]
fn attribute_values_match_all_numeric_syntax_types_in_context() {
    for (syntax, value, comparison) in [
        ("<number>", "17", "17"),
        ("<integer>", "17", "17"),
        ("<length>", "calc(10pt + 7pt)", "17pt"),
        ("<length-percentage>", "50%", "50%"),
        ("<percentage>", "50%", "50%"),
        ("<angle>", ".5turn", "180deg"),
        ("<time>", "1000ms", "1s"),
        ("<resolution>", "2dppx", "192dpi"),
    ] {
        let css = format!(
            "#target{{--value:attr(data-value {syntax}, invalid);width:if(style(--value = {comparison}):17pt;else:99pt)}}"
        );
        let html = format!("<p id=target data-value='{value}'>Attribute</p>");
        let style = computed_in(
            &css,
            &html,
            "#target",
            &Default::default(),
            &FileResolver::default(),
        )
        .unwrap_or_else(|error| panic!("typed attribute {syntax} / {value}: {error}"));
        common::assert_pt(points(&style.width), 17.0);
    }
}

#[test]
fn attribute_values_share_shorthand_math_and_syntax_alternatives() {
    let style = computed_in(
        "#target{width:calc(3pt + attr(data-size <length>));margin:attr(data-edges '<length>+');height:attr(data-auto <length> | auto);font-family:attr(data-families <string>#);color:attr(data-color <color>);transform:attr(data-transform <transform-list>)}",
        "<p id=target data-size='14pt' data-edges='1pt 2pt 3pt 4pt' data-auto='auto' data-families='&quot;One&quot;,&quot;Two&quot;' data-color='red' data-transform='translateX(5pt) scale(2)'>Attribute</p>",
        "#target",&Default::default(),&FileResolver::default(),
    ).unwrap();
    common::assert_pt(points(&style.width), 17.0);
    assert_eq!(style.height, Size::Auto);
    for (actual, expected) in style.margin.iter().zip([1.0, 2.0, 3.0, 4.0]) {
        common::assert_pt(points(actual), expected);
    }
    assert_eq!(
        style.families,
        vec![
            crate::fonts::FontFamily::Named("One".into()),
            crate::fonts::FontFamily::Named("Two".into())
        ]
    );
    assert_eq!(style.color.rgb, [255, 0, 0]);
    assert_eq!(
        style.transform.len(),
        2,
        "the shared transform grammar must preserve both functions"
    );
    let universal = computed_in(
        "#target{padding:attr(data-edges *)}",
        "<p id=target data-edges='1pt 2pt'>Attribute</p>",
        "#target",
        &Default::default(),
        &FileResolver::default(),
    )
    .unwrap();
    assert_eq!(
        universal.padding,
        [1.0, 2.0, 1.0, 2.0].map(LengthPercentage::Length)
    );
}

#[test]
fn attribute_values_separate_missing_empty_invalid_and_syntax_error_stages() {
    for (attribute, value, expected) in [
        (
            "data-size='17pt'",
            "attr(data-size <length>,99pt)",
            Some(17.0),
        ),
        ("data-size=''", "attr(data-size <length>,17pt)", Some(17.0)),
        ("", "attr(data-size <length>,17pt)", Some(17.0)),
        (
            "data-size='bad'",
            "attr(data-size <length>,17pt)",
            Some(17.0),
        ),
        (
            "data-size='bad'",
            "attr(data-size <number>,17pt)",
            Some(17.0),
        ),
        ("data-size='bad'", "attr(data-size <length>)", None),
        ("data-size='17pt'", "attr(data-size)", None),
        ("", "attr(data-size <length>,{})", None),
        (
            "data-size='17pt'",
            "attr(data-size < length >,99pt)",
            Some(13.0),
        ),
        (
            "data-size='17pt'",
            "attr(data-size <length> +,99pt)",
            Some(13.0),
        ),
        (
            "data-size='17pt'",
            "attr(data-size <transform-list>+,99pt)",
            Some(13.0),
        ),
        ("", "attr(data-size <length>,)", Some(13.0)),
    ] {
        let css = format!("#target{{width:13pt;width:{value}}}");
        let html = format!("<p id=target {attribute}>Attribute</p>");
        let style = computed_in(
            &css,
            &html,
            "#target",
            &Default::default(),
            &FileResolver::default(),
        )
        .unwrap();
        match expected {
            Some(expected) => assert!(
                (points(&style.width) - expected).abs() < 0.001,
                "{attribute} / {value}: {:?}, expected {expected}",
                style.width
            ),
            None => assert_eq!(
                style.width,
                Size::Auto,
                "invalid computed attribute result must reset rather than revive width: {attribute} / {value}"
            ),
        }
    }
}

#[test]
fn attribute_custom_values_freeze_lookup_on_the_declaring_element() {
    let style = computed_in(
        "#parent{--size:attr(data-size <length>);--label:attr(data-label)}#target{width:var(--size);height:attr(data-size <length>);margin-left:if(style(--label:\"Parent\"):3pt;else:99pt)}",
        "<div id=parent data-size='17pt' data-label='Parent'><p id=target data-size='29pt' data-label='Child'>Attribute</p></div>",
        "#target",&Default::default(),&FileResolver::default(),
    ).unwrap();
    common::assert_pt(points(&style.width), 17.0);
    common::assert_pt(points(&style.height), 29.0);
    common::assert_pt(points(&style.margin[3]), 3.0);
}

#[test]
fn attribute_string_values_use_the_originating_element_without_css_unescaping() {
    let trace = common::render_trace(
        r"<style>@page{size:400pt 200pt;margin:10pt}body,p{margin:0;font:10pt/12pt serif}p::before{white-space:pre;content:attr(DATA-X) '|' attr(data-empty,'Fallback') '|' attr(missing,'Missing')}</style><p data-x='A\41 /*literal*/' data-empty=''>Body</p>",
    );
    assert_eq!(trace.pages[0][0].text, r"A\41 /*literal*/||MissingBody");
}

#[test]
fn attribute_names_keep_namespace_and_xml_case_sensitivity() {
    let document =
        roxmltree::Document::parse("<root xmlns:d='urn:data' d:Size='17pt' size='29pt'/>").unwrap();
    let tree = crate::dom::XmlTree::new(&document);
    let element = tree.element(document.root_element()).unwrap();
    let sheet = Stylesheet::parse(&[StylesheetSource{text:"@namespace d 'urn:data';root{width:attr(d|Size <length>);height:attr(size <length>);margin-left:attr(Size <length>,3pt);margin-right:attr(missing|Size <length>,4pt)}".into(),base:font_base()}]).unwrap();
    let style = sheet.computed(element, &Style::default()).unwrap();
    common::assert_pt(points(&style.width), 17.0);
    common::assert_pt(points(&style.height), 29.0);
    common::assert_pt(points(&style.margin[3]), 3.0);
    common::assert_pt(points(&style.margin[1]), 4.0);
}

#[test]
fn attribute_taint_covers_selected_custom_reference_fallbacks() {
    for value in [
        "var(--missing,url(safe.png) attr(data-color <color>))",
        "inherit(--missing,url(safe.png) attr(data-color <color>))",
    ] {
        let style = computed_in(
            &format!("#target{{background:{value}}}"),
            "<p id=target data-color=red>Attribute</p>",
            "#target",
            &Default::default(),
            &FileResolver::default(),
        )
        .unwrap();
        assert!(
            !style.decoration.backgrounds.has_images(),
            "a selected tainted custom-reference fallback taints its whole replacement: {value}"
        );
    }
}

#[test]
fn attribute_taint_blocks_url_use_through_inherited_custom_values() {
    for css in [
        "#target{background-image:attr(data-asset <url>)}",
        "#parent{--asset:attr(data-asset <url>)}#target{background-image:var(--asset)}",
        "#parent{--asset:attr(data-asset *)}#target{--copy:var(--asset);background-image:var(--copy)}",
    ] {
        let style = computed_in(css,"<div id=parent data-asset='url(&quot;secret.png&quot;)'><p id=target data-asset='url(&quot;secret.png&quot;)'>Attribute</p></div>","#target",&Default::default(),&FileResolver::default()).unwrap();
        assert!(
            !style.decoration.backgrounds.has_images(),
            "attribute data cannot become a URL: {css}"
        );
    }
    let safe = computed_in(
        "#target{background:url(safe.png) attr(data-color <color>)}",
        "<p id=target data-color='red'>Attribute</p>",
        "#target",
        &Default::default(),
        &FileResolver::default(),
    )
    .unwrap();
    assert!(
        safe.decoration.backgrounds.has_images(),
        "an independent authored URL remains valid beside an attribute color"
    );
    assert_eq!(safe.decoration.background.rgb, [255, 0, 0]);
}

#[test]
fn attribute_url_taint_and_quoted_url_templates_keep_computed_invalidity() {
    for value in [
        "url(\"safe.png\" attr(data-extra *))",
        "image-set(attr(data-name) 1x)",
        "image(attr(data-name))",
    ] {
        let css = format!(
            "#target{{background-image:linear-gradient(red,blue);background-image:{value}}}"
        );
        let style = computed_in(
            &css,
            "<p id=target data-extra='red' data-name='secret.png'>Attribute</p>",
            "#target",
            &Default::default(),
            &FileResolver::default(),
        )
        .unwrap();
        assert!(
            !style.decoration.backgrounds.has_images(),
            "computed invalidity must not revive the earlier gradient: {value}"
        );
    }
    let empty=computed_in("#target{background-image:linear-gradient(red,blue);background-image:url(\"safe.png\" attr(data-extra *))}","<p id=target data-extra=''>Attribute</p>","#target",&Default::default(),&FileResolver::default()).unwrap();
    assert!(
        !empty.decoration.backgrounds.has_images(),
        "an empty tainted substitution inside a URL keeps its function ownership"
    );
    let untainted = computed("#target{--empty:;background-image:url(\"safe.png\" var(--empty))}");
    let expected = font_base().resolve("safe.png").unwrap().to_string();
    assert_eq!(
        serde_json::to_value(&untainted.decoration.backgrounds.images).unwrap(),
        serde_json::json!([{"Url": {"url": expected, "resolution": 1.0}}]),
        "an untainted empty substitution yields a valid URL bound at the declaration"
    );
}

#[test]
fn attribute_taint_is_checked_in_style_query_url_operands() {
    let style=computed_in("#target{background-image:url(https://example.invalid/asset.png);width:if(style(background-image:attr(data-asset <url>)):99pt;else:17pt)}","<p id=target data-asset='url(https://example.invalid/asset.png)'>Attribute</p>","#target",&Default::default(),&FileResolver::default()).unwrap();
    common::assert_pt(points(&style.width), 17.0);
}

#[test]
fn attribute_substitution_preserves_whole_value_syntax_constraints() {
    let html = "<p id=target data-width='17pt'>Attribute</p>";
    let selected=computed_in("#target{width:first-valid(toggle(10pt,attr(data-width <length>)),attr(data-width <length>))}",html,"#target",&Default::default(),&FileResolver::default()).unwrap();
    common::assert_pt(points(&selected.width), 17.0);
    let invalid = computed_in(
        "#target{width:11pt;width:if(else:toggle(10pt,attr(data-width <length>)))}",
        html,
        "#target",
        &Default::default(),
        &FileResolver::default(),
    )
    .unwrap();
    assert_eq!(
        invalid.width,
        Size::Auto,
        "a selected invalid whole value remains invalid at computed time"
    );
    let queried=computed_in("#target{width:10pt;margin-left:if(style(width:toggle(10pt,attr(data-width <length>))):99pt;else:17pt)}",html,"#target",&Default::default(),&FileResolver::default()).unwrap();
    common::assert_pt(points(&queried.margin[3]), 17.0);
    let supports = computed(
        "#target{margin-left:17pt}@supports(width:toggle(10pt,attr(data-width <length>))){#target{margin-left:99pt}}",
    );
    common::assert_pt(points(&supports.margin[3]), 17.0);
}

#[test]
fn conditional_values_select_the_first_matching_print_and_supports_branch() {
    for value in [
        "if(media(print):17pt;else:99pt)",
        "if(media(screen):99pt;supports(display:flex):17pt;else:88pt)",
        "if(supports(display:ruby):99pt;supports((width:12pt) and (color:red)):17pt;else:88pt)",
        "if((media(print) and supports(display:flex)) or media(screen):17pt;else:99pt)",
        "if(future-query():99pt;else:17pt)",
        "if(not future-query():99pt;else:17pt)",
        "if(else:17pt;media(print):99pt)",
        r"i\66(media(print):17pt;else:99pt)",
    ] {
        common::assert_pt(
            points(&computed(&format!("#target{{width:{value}}}")).width),
            17.0,
        );
    }
    let settings = crate::style::PrintSettings {
        paper: MEDIA_PAPER_PT,
        preferences: crate::style::PrintPreferences {
            color_scheme: crate::style::ColorScheme::Dark,
            ..Default::default()
        },
        ..Default::default()
    };
    let style = computed_in(
        "#target{width:if(media((width:800px) and (prefers-color-scheme:dark)):17pt;else:99pt)}",
        "<p id=target>Print</p>",
        "#target",
        &settings,
        &FileResolver::default(),
    )
    .unwrap();
    common::assert_pt(points(&style.width), 17.0);
}

#[test]
fn conditional_values_substitute_inside_math_and_pending_shorthands() {
    let style = computed(
        "#target{--choice:17pt;width:calc(3pt + if(media(print):var(--choice);else:var(--missing)));margin:1pt if(media(print):2pt 3pt;else:99pt) 4pt;padding:if(media(screen):;else:5pt)}",
    );
    common::assert_pt(points(&style.width), 20.0);
    for (actual, expected) in style.margin.iter().zip([1.0, 2.0, 3.0, 4.0]) {
        common::assert_pt(points(actual), expected);
    }
    assert_eq!(style.padding, [const { LengthPercentage::Length(5.0) }; 4]);
    let without_tokens = computed("#target{margin:1pt if(media(screen):99pt) 2pt}");
    for (actual, expected) in without_tokens.margin.iter().zip([1.0, 2.0, 1.0, 2.0]) {
        common::assert_pt(points(actual), expected);
    }
}

#[test]
fn conditional_values_distinguish_syntax_errors_from_computed_invalidity() {
    for value in ["if()", "if(else:17pt;;)", "if(media(print) 17pt)"] {
        let style = computed(&format!("#target{{width:13pt;width:{value}}}"));
        common::assert_pt(points(&style.width), 13.0);
    }
    for value in [
        "if(else:)",
        "if(media(screen):17pt)",
        "if(else:invalid)",
        "if(else:17)pt",
    ] {
        let style = computed(&format!("#target{{width:13pt;width:{value}}}"));
        assert!(
            matches!(style.width, Size::Auto),
            "computed-invalid {value} must reset the winner: {:?}",
            style.width
        );
    }
    let inherited = computed("#parent{width:23pt}#target{width:if(else:inherit)}");
    common::assert_pt(points(&inherited.width), 23.0);
    let rollback = computed(
        "@layer first,second;@layer first{#target{width:17pt}}@layer second{#target{width:if(media(print):revert-layer;else:99pt)}}",
    );
    common::assert_pt(points(&rollback.width), 17.0);
}

#[test]
fn conditional_custom_properties_preserve_parent_computation_and_cycle_edges() {
    let inherited = computed(
        "#parent{--mode:print;--choice:if(style(--mode:print):17pt;else:99pt)}#target{--mode:screen;width:var(--choice)}",
    );
    common::assert_pt(points(&inherited.width), 17.0);
    let unused_missing = computed(
        "#target{--choice:if(media(print):17pt;else:var(--missing));width:var(--choice,99pt)}",
    );
    common::assert_pt(points(&unused_missing.width), 17.0);
    let conservative_cycle = computed(
        "#target{--a:if(media(print):17pt;else:var(--b));--b:var(--a);width:var(--a,13pt)}",
    );
    common::assert_pt(points(&conservative_cycle.width), 13.0);
}

#[test]
fn conditional_style_queries_compare_computed_properties_and_shorthands() {
    for (declarations, query) in [
        ("color:red", "color:rgb(255,0,0)"),
        ("height:1in", "height:72pt"),
        ("font-size:20pt;height:2em", "height:40pt"),
        ("margin:1pt 2pt 3pt 4pt", "margin:1pt 2pt 3pt 4pt"),
        ("position:absolute;display:inline-flex", "display:flex"),
        ("overflow-y:hidden", "overflow-x:auto"),
        ("color:red", "color"),
        ("--mode:print", "--mode:print"),
        ("--empty:", "--empty"),
    ] {
        let style = computed(&format!(
            "#target{{{declarations};width:if(style({query}):17pt;else:99pt)}}"
        ));
        common::assert_pt(points(&style.width), 17.0);
    }
    for query in ["margin:1pt 2pt 3pt 5pt", "color:revert", "--missing"] {
        let style = computed(&format!(
            "#target{{margin:1pt 2pt 3pt 4pt;width:if(style({query}):99pt;else:17pt)}}"
        ));
        common::assert_pt(points(&style.width), 17.0);
    }
}

#[test]
fn conditional_style_ranges_share_typed_units_and_element_metrics() {
    for (declarations, query) in [
        ("--n:3", "1 < --n <= 3"),
        ("--n:3", "4 > --n >= 3"),
        ("--n:50%", "25% < --n < 75%"),
        ("--n:1in", "72pt = --n"),
        ("font-size:20pt;--n:2em", "--n = 40pt"),
        ("--n:0pt", "0 = --n"),
        ("--n:1s", "--n = 1000ms"),
        ("--n:1turn", "--n = 360deg"),
        ("--n:1khz", "--n = 1000hz"),
        ("--n:2dppx", "--n = 192dpi"),
        ("--n:calc(2 * 3)", "--n = 6"),
        ("--n:calc(1khz + 1khz)", "--n = 2000hz"),
    ] {
        let style = computed(&format!(
            "#target{{{declarations};width:if(style({query}):17pt;else:99pt)}}"
        ));
        common::assert_pt(points(&style.width), 17.0);
    }
    for query in ["3pt = 3", "1pt < 3%", "1 < 2 > 1", "--missing = 0"] {
        let style = computed(&format!(
            "#target{{width:if(style({query}):99pt;else:17pt)}}"
        ));
        common::assert_pt(points(&style.width), 17.0);
    }
}

#[test]
fn conditional_style_queries_report_computed_property_cycles_with_sources() {
    let error=computed_with("#target{width:if(style(height:20pt):17pt;else:99pt);\nheight:if(style(width:17pt):20pt;else:88pt)}",&FileResolver::default()).expect_err("mutually dependent queries must not use provisional styles");
    let StyleError::DependencyCycle {
        sources,
        dependencies,
    } = error
    else {
        panic!("expected a computed-property cycle, got {error:?}");
    };
    assert_eq!(
        sources.len(),
        2,
        "both authored declarations need locations: {sources:?}"
    );
    assert!(
        dependencies.iter().any(|name| name.ends_with("width")),
        "{dependencies:?}"
    );
    assert!(
        dependencies.iter().any(|name| name.ends_with("height")),
        "{dependencies:?}"
    );
}

#[test]
fn conditional_style_queries_report_cross_custom_and_ordinary_property_cycles() {
    let error = computed_with(
        "#target{--size:if(style(width:17pt):20pt;else:99pt);\nwidth:var(--size)}",
        &FileResolver::default(),
    )
    .expect_err("a custom-property query cycle must not read a provisional width");
    let StyleError::DependencyCycle {
        sources,
        dependencies,
    } = error
    else {
        panic!("expected a cross-property dependency cycle, got {error:?}");
    };
    assert_eq!(
        sources.len(),
        2,
        "the custom and ordinary declarations need locations: {sources:?}"
    );
    assert!(
        dependencies.iter().any(|name| name.ends_with("--size")),
        "{dependencies:?}"
    );
    assert!(
        dependencies.iter().any(|name| name.ends_with("width")),
        "{dependencies:?}"
    );
}

#[test]
fn conditional_values_reach_page_margin_boxes_and_originating_pseudo_elements() {
    let trace = common::render_trace(
        "<style>@page{size:200pt 300pt;margin:10pt;margin-left:if(media(print):20pt;else:99pt);@top-center{font:8pt serif;content:if(media(print):'Header';else:none)}}body,p{margin:0;font:8pt serif}p{--text:'Pseudo'}p::before{content:if(style(--text):var(--text);else:'Wrong')}</style><p>Body</p>",
    );
    common::assert_pt(trace.page_area.x, 20.0);
    let text = trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.contains("Pseudo") && text.contains("Body") && text.contains("Header"),
        "page and pseudo conditional output: {text:?}"
    );
}

#[test]
fn conditional_overflow_queries_resolve_independent_axes_before_dependent_sources() {
    for declarations in [
        "overflow-x:scroll;overflow-y:if(style(overflow-x:scroll):hidden;else:clip)",
        "overflow-y:hidden;overflow-x:if(style(overflow-y:hidden):scroll;else:clip)",
    ] {
        let style = computed(&format!(
            "#target{{{declarations};width:if(style(overflow-x:scroll):17pt;else:99pt)}}"
        ));
        common::assert_pt(points(&style.width), 17.0);
        assert_eq!(
            style.overflow,
            [
                crate::style::Overflow::Scroll,
                crate::style::Overflow::Hidden
            ]
        );
    }
}

#[test]
fn conditional_overflow_query_candidates_cycle_after_pairing_each_argument() {
    for (axis, other) in [("x", "y"), ("y", "x")] {
        let style = computed(&format!(
            "#parent{{overflow:auto}}#target{{overflow-{axis}:scroll;overflow-{other}:hidden;width:if(style(overflow-{axis}:toggle(visible,scroll)):17pt;else:99pt)}}"
        ));
        common::assert_pt(points(&style.width), 17.0);
    }
}

#[test]
fn conditional_supports_queries_use_the_declaring_stylesheet_namespace_map() {
    for css in [
        "@namespace s 'http://www.w3.org/2000/svg';#target{width:if(supports(selector(s|rect)):17pt;else:99pt)}",
        "@namespace s 'http://www.w3.org/2000/svg';@media print{#target{width:if(supports(selector(s|rect)):17pt;else:99pt)}}",
        "@namespace s 'http://www.w3.org/2000/svg';#target{--choice:if(supports(selector(s|rect)):17pt;else:99pt);width:var(--choice)}",
    ] {
        common::assert_pt(points(&computed(css).width), 17.0);
    }
    for css in [
        "@import 'data:text/css,%40namespace%20s%20url(http://www.w3.org/2000/svg);%23target%7Bwidth:if(supports(selector(s%7Crect)):17pt;else:99pt)%7D';",
        "@import 'data:text/css,%23target%7Bwidth:if(supports(selector(s%7Crect)):99pt;else:17pt)%7D';@namespace s 'http://www.w3.org/2000/svg';",
    ] {
        let html = format!(
            "<style>{}</style><p id=target>Imported conditional</p>",
            css.replace("width:", "font-size:")
        );
        let trace = common::render_trace(&html);
        common::assert_pt(trace.pages[0][0].font_size, 17.0);
    }
    let inline = computed_in(
        "@namespace s 'http://www.w3.org/2000/svg';",
        "<p id=target style='width:if(supports(selector(s|rect)):99pt;else:17pt)'>Inline</p>",
        "#target",
        &Default::default(),
        &FileResolver::default(),
    )
    .unwrap();
    common::assert_pt(points(&inline.width), 17.0);
}

#[test]
fn conditional_font_family_queries_keep_the_querying_declarations_tree_scope() {
    for body in [
        "<div style='font-family:Scoped'><template shadowrootmode=open><style>@font-face{font-family:Scoped;src:url(MetricGlyphTest.ttf)}p{font-family:inherit;font-size:if(style(font-family:Scoped):99pt;else:20pt)}</style><p>0</p></template></div>",
        "<div><template shadowrootmode=open><style>@font-face{font-family:Scoped;src:url(MetricGlyphTest.ttf)}::slotted(p){--choice:if(style(font-family:Scoped):99pt;else:20pt);font-size:var(--choice)}</style><slot></slot></template><p style='font-family:Scoped'>0</p></div>",
    ] {
        let html = format!(
            "<style>@font-face{{font-family:Scoped;src:url(FontStyleTest-Ital.ttf)}}@page{{size:200pt 300pt;margin:10pt}}body,p{{margin:0;font-size:20pt;line-height:24pt}}</style>{body}"
        );
        let mut options = common::local_options();
        options.base = font_base();
        let trace = crate::render_html(&html, &options).unwrap().layout.unwrap();
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "0")
            .expect("scoped conditional text");
        common::assert_pt(line.font_size, 20.0);
        assert!(
            (line.width - FIXTURE_ZERO_PT).abs() < 0.001,
            "scoped condition {body}: expected width {FIXTURE_ZERO_PT}, got {}",
            line.width
        );
    }
}

#[test]
fn conditional_font_queries_resolve_absolute_dimensions_without_font_cycles() {
    let style = computed(
        "#target{height:20pt;font-size:if(style(height:20pt):17pt;else:99pt);width:if(style(font-size:17pt):2em;else:99pt)}",
    );
    common::assert_pt(style.size, 17.0);
    common::assert_pt(points(&style.width), 34.0);
    let error = computed_with(
        "#target{height:2em;font-size:if(style(height:34pt):17pt;else:99pt)}",
        &FileResolver::default(),
    )
    .unwrap_err();
    match error {
        StyleError::DependencyCycle {
            sources,
            dependencies,
        } => {
            assert!(
                sources.len() >= 2,
                "both authored sources must be retained: {sources:?}"
            );
            assert!(
                dependencies
                    .iter()
                    .any(|dependency| dependency.ends_with("font-size")),
                "font dependency: {dependencies:?}"
            );
            assert!(
                dependencies
                    .iter()
                    .any(|dependency| dependency.ends_with("height")),
                "dimension dependency: {dependencies:?}"
            );
        }
        error => panic!("expected a font-size/dimension dependency cycle, got {error:?}"),
    }
}

#[test]
fn conditional_custom_dependencies_do_not_evaluate_unselected_replacements() {
    for declarations in [
        "--a:if(media(print):17pt;else:var(--b));--b:if(style(--a:17pt):20pt;else:99pt)",
        "--present:17pt;--a:var(--present,var(--b));--b:if(style(--a:17pt):20pt;else:99pt)",
    ] {
        let style = computed(&format!(
            "#target{{{declarations};width:var(--a);height:var(--b)}}"
        ));
        common::assert_pt(points(&style.width), 17.0);
        common::assert_pt(points(&style.height), 20.0);
    }
}

#[test]
fn conditional_queries_skip_inactive_metrics_and_keep_shared_limits() {
    let resolver = RecordingResolver {
        inner: FileResolver::default(),
        requests: Default::default(),
    };
    let face =
        "@font-face{font-family:Metric;src:url(FontStyleTest-Ital.ttf)}#target{font-family:Metric}";
    for value in [
        "if(media(print):17pt;style(1ch > 0pt):99pt)",
        "if(media(screen) and style(1ch > 0pt):99pt;else:17pt)",
        "if(supports(width:1ch):17pt;else:99pt)",
    ] {
        let style = computed_with(&format!("{face}#target{{width:{value}}}"), &resolver).unwrap();
        common::assert_pt(points(&style.width), 17.0);
        assert!(
            resolver.requests.borrow().is_empty(),
            "inactive query/capability must not request metrics: {value}"
        );
    }
    const EXCESSIVE_NESTING: usize = 140;
    let mut nested = "17pt".to_owned();
    for _ in 0..EXCESSIVE_NESTING {
        nested = format!("if(media(screen):{nested};else:17pt)");
    }
    let error = Stylesheet::parse(&[StylesheetSource {
        text: format!("#target{{width:{nested}}}"),
        base: font_base(),
    }])
    .err()
    .expect("excessive conditional nesting is rejected during stylesheet parsing");
    assert!(
        matches!(
            error,
            StyleError::VariableLimit {
                resource: "value nesting",
                ..
            }
        ),
        "all branches retain shared nesting limits: {error:?}"
    );
}

#[test]
fn conditional_custom_queries_keep_original_numeric_and_whitespace_text() {
    for (declarations, query, expected) in [
        ("--n:100.00", "--n:100.00", 17.0),
        ("--n:100.00", "--n:100.0", 99.0),
        ("--n:100.00", "--n:100", 99.0),
        ("--n:a  b", "--n:a b", 99.0),
        ("--n:a/**/b", "--n:a b", 99.0),
        ("--part:p;--n:var(--part)rint", "--n:print", 99.0),
        ("--part:p;--n:var(--part)rint", "--n:p/**/rint", 17.0),
        (
            "--part:/*baz*/;--n:/*foo*/var(--part)/*bar*/",
            "--n:/*foo*//*baz*//*bar*/",
            17.0,
        ),
        ("--n:var(--missing,/*fallback*/)", "--n:/*fallback*/", 17.0),
        ("--n:if(else:/*choice*/)", "--n:/*choice*/", 17.0),
        ("--n:if(else:p)rint", "--n:p/**/rint", 17.0),
        ("--n:a if(media(screen):x) b", "--n:a  b", 17.0),
        (r"--n:\70 ", r"--n:\70 ", 17.0),
        (r"--n:\70 ", "--n:p", 99.0),
        ("--n:\u{a0}text\u{a0}", "--n:text", 99.0),
        ("--n:  a  ", "--n:a", 17.0),
    ] {
        let style = computed(&format!(
            "#target{{{declarations};width:if(style({query}):17pt;else:99pt)}}"
        ));
        common::assert_pt(points(&style.width), expected);
    }
}

#[test]
fn conditional_page_queries_include_margins_and_descriptor_parent_context() {
    let trace = common::render_trace(
        "<style>html{font-size:20pt;color:red}@page{size:200pt 300pt;margin:10pt;margin-left:20pt;font-size:if(style(margin-left:20pt):30pt;else:99pt);marks:if(style(color:inherit):crop;else:none);bleed:if(style(font-size:30pt):5pt;else:99pt);@top-center{font-size:inherit;line-height:1;content:'Header'}}body,p{margin:0;font:8pt serif}</style><p>Body</p>",
    );
    let header = trace
        .pages
        .iter()
        .flatten()
        .find(|line| line.text == "Header")
        .expect("conditional page header");
    common::assert_pt(header.font_size, 30.0);
    common::assert_pt(trace.page_area.x, 20.0);
    common::assert_pt(trace.page_output.bleed.width, 210.0);
    common::assert_pt(trace.page_output.bleed.height, 310.0);
    assert_eq!(
        trace.page_output.marks.len(),
        8,
        "conditional crop marks use the actual parent color"
    );
}

#[test]
fn conditional_custom_rollbacks_recompute_cycles_before_publishing_values() {
    let style = computed(
        "@layer first,second;@layer first{#target{--a:var(--b,99pt);--b:var(--a,98pt)}}@layer second{#target{--b:if(media(print):revert-layer;else:17pt);width:var(--a,13pt)}}",
    );
    common::assert_pt(points(&style.width), 13.0);
    let inherited =
        computed("#parent{--a:17pt}#target{--a:if(media(print):inherit;else:99pt);width:var(--a)}");
    common::assert_pt(points(&inherited.width), 17.0);
}

#[test]
fn toggle_selects_after_the_first_matching_computed_value() {
    for (parent, arguments, expected) in [
        ("normal", "normal,bold", 700.0),
        ("bold", "normal,bold", 400.0),
        ("500", "normal,bold", 400.0),
        ("normal", "400,700,normal,900", 700.0),
        ("normal", "400", 400.0),
    ] {
        let style = computed(&format!(
            "#parent{{font-weight:{parent}}}#target{{font-weight:toggle({arguments})}}"
        ));
        assert_eq!(
            style.font_weight.value(),
            expected,
            "the first computed match determines the next value: parent {parent}, arguments {arguments}"
        );
    }
    let style = computed(
        "#parent{font-style:italic;list-style-type:circle}#target{font-style:toggle(italic,normal);list-style-type:toggle(disc,circle,square)}",
    );
    assert_eq!(style.font_style, crate::fonts::FontStyle::Normal);
    assert_eq!(
        style.list_style_type,
        crate::style::ListStyleType::Counter(
            crate::style::counter_style::CounterStyle::predefined("square")
        )
    );
}

#[test]
fn toggle_shorthands_cycle_each_longhand_independently() {
    let style =
        computed("#parent{margin:1pt 4pt 4pt 5pt}#target{margin:toggle(1pt 2pt,4pt,1pt 5pt 4pt)}");
    assert_eq!(
        style.margin.map(|value| points(&value)),
        [4.0, 5.0, 4.0, 2.0]
    );
}

#[test]
fn toggle_compares_canonical_values_without_resolving_percentages() {
    let style = computed(
        "#parent{margin-left:1in;color:rgb(255 0 0);width:50%}#target{margin-left:toggle(72pt,3pt);color:toggle(#f00,blue);width:toggle(50%,80pt)}",
    );
    common::assert_pt(points(&style.margin[3]), 3.0);
    assert_eq!(style.color.rgb, [0, 0, 255]);
    common::assert_pt(points(&style.width), 80.0);
    let style = computed("#parent{width:0pt}#target{width:toggle(0%,80pt)}");
    assert_eq!(
        style.width.resolve_definite(None),
        None,
        "percentages and lengths remain distinct computed values"
    );
    assert_eq!(style.width.resolve_definite(Some(200.0)), Some(0.0));
}

#[test]
fn toggle_uses_the_receiving_property_font_context() {
    let style = computed(
        "#parent{font-size:20pt;margin-top:10pt}#target{font-size:10pt;margin-top:toggle(1em,30pt)}",
    );
    common::assert_pt(points(&style.margin[0]), 30.0);
    let style = computed("#parent{font-size:20pt}#target{font-size:toggle(1em,30pt)}");
    common::assert_pt(style.size, 30.0);
    let style =
        computed("#parent{width:10pt}#target{--cycle:toggle(10pt,30pt);width:var(--cycle)}");
    common::assert_pt(points(&style.width), 30.0);
}

#[test]
fn toggle_invalid_arguments_and_forbidden_nesting_are_rejected_before_cascade() {
    for source in [
        "toggle()",
        "toggle(10pt,red)",
        "toggle(10pt,)",
        "toggle(10pt,calc(20pt))",
        "toggle(10pt,attr(data-width))",
        "toggle(10pt,toggle(20pt,30pt))",
        "10pt toggle(20pt,30pt)",
    ] {
        let style = computed(&format!("#target{{width:11pt;width:{source}}}"));
        common::assert_pt(points(&style.width), 11.0);
    }
    let style = computed(
        "#target{margin-left:7pt}@supports(width:toggle(10pt,20pt)){#target{margin-left:13pt}}@supports(width:toggle(10pt,red)){#target{margin-left:99pt}}",
    );
    common::assert_pt(points(&style.margin[3]), 13.0);
}

#[test]
fn toggle_generated_content_uses_computed_content_on_its_parent() {
    for (parent, arguments, generated) in [
        ("none", r#"none,"Selected""#, true),
        ("normal", r#"normal,"Selected""#, false),
        ("normal", r#"inherit,"Selected""#, true),
    ] {
        let trace = common::render_trace(&format!(
            "<style>{}p{{margin:0;font:8pt serif;content:{parent}}}p::before{{display:block;content:toggle({arguments})}}</style><p>Body</p>",
            common::PROFILE
        ));
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .any(|line| line.text == "Selected"),
            generated,
            "computed content context: parent {parent}, arguments {arguments}"
        );
    }
    let trace = common::render_trace(&format!(
        "<style>{}@page{{@top-center{{content:toggle(inherit,'Header')}}}}p{{margin:0;font:8pt serif}}</style><p>Body</p>",
        common::PROFILE
    ));
    assert!(
        trace
            .pages
            .iter()
            .flatten()
            .any(|line| line.text == "Header"),
        "page margin toggle must compare the page parent's computed content"
    );
}

#[test]
fn toggle_rollback_candidates_use_the_existing_origin_and_layer_cascade() {
    for (parent, expected) in [("17pt", 30.0), ("99pt", 17.0)] {
        let style = computed(&format!(
            "@layer early,late;#parent{{width:{parent}}}@layer early{{#target{{width:17pt}}}}@layer late{{#target{{width:toggle(revert-layer,30pt)}}}}"
        ));
        common::assert_pt(points(&style.width), expected);
    }
}

#[test]
fn toggle_shared_rollback_values_do_not_expand_into_repeated_computation() {
    const LAYERS: usize = 14;
    let mut css = "@layer base{#target{width:17pt}}".to_owned();
    for index in 0..LAYERS {
        let _ = write!(
            css,
            "@layer level{index}{{#target{{width:toggle(revert-layer,revert-layer)}}}}"
        );
    }
    common::assert_pt(points(&computed(&css).width), 17.0);
}

#[test]
fn toggle_font_family_comparison_and_rollback_keep_tree_scopes() {
    for body in [
        "<div style='font-family:Scoped'><template shadowrootmode=open><style>@font-face{font-family:Scoped;src:url(FontStyleTest-Ital.ttf)}p{font-family:toggle(Scoped,serif)}</style><p>0</p></template></div>",
        "<div style='font-family:serif'><template shadowrootmode=open><style>@font-face{font-family:Scoped;src:url(FontStyleTest-Ital.ttf)}p{font-family:toggle(serif,Scoped)}</style><p>0</p></template></div>",
        "<div style='font-family:serif'><template shadowrootmode=open><style>@font-face{font-family:Document;src:url(MetricGlyphTest.ttf)}::slotted(#target){font-family:toggle(revert-layer,serif)!important}</style><slot></slot></template><p id=target style='font-family:Document'>0</p></div>",
    ] {
        let html = format!(
            "<style>@font-face{{font-family:Scoped;src:url(MetricGlyphTest.ttf)}}@font-face{{font-family:Document;src:url(FontStyleTest-Ital.ttf)}}@page{{size:200pt 300pt;margin:10pt}}body,p{{margin:0;font-size:20pt;line-height:24pt}}</style>{body}"
        );
        let mut options = common::local_options();
        options.base = font_base();
        let trace = crate::render_html(&html, &options).unwrap().layout.unwrap();
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "0")
            .expect("scoped zero glyph");
        assert_eq!(
            line.width, FIXTURE_ZERO_PT,
            "tree-scoped family selection: {body}; fonts {:?}",
            trace.fonts
        );
    }
}

#[test]
fn toggle_page_comparison_requests_the_root_computed_property() {
    let trace = common::render_trace(
        "<style>html{margin-left:20pt}@page{size:200pt 300pt;margin:10pt;margin-left:toggle(20pt,30pt)}body,p{margin:0;font:8pt serif}</style><p>Page comparison</p>",
    );
    common::assert_pt(trace.page_area.x, 30.0);
    common::assert_pt(trace.page_area.width, 160.0);
}

#[test]
fn toggle_content_capabilities_include_alternative_value_computation() {
    let style = computed(
        "#target{margin-left:7pt}@supports(content:toggle('x'/'alt',none)){#target{margin-left:13pt}}@supports(content:toggle('x'/counter(missing),none)){#target{margin-left:17pt}}@supports(content:toggle('x'/counter(missing,symbols(cyclic 'x')),none)){#target{margin-left:99pt}}",
    );
    common::assert_pt(points(&style.margin[3]), 99.0);
}

#[test]
fn toggle_display_candidates_include_computed_blockification() {
    let absolute = computed(
        "#parent{display:block}#target{position:absolute;display:toggle(inline,inline-flex,block)}",
    );
    assert_eq!(
        absolute.display,
        crate::style::Display::Flex,
        "blockified inline matches the parent's block and selects inline-flex, which computes to flex"
    );
    let item =
        computed("#parent{display:flex}#target{display:toggle(inline-flex,inline-block,block)}");
    assert_eq!(
        item.display,
        crate::style::Display::Block,
        "blockification converts inline-block to block flow for legacy compatibility"
    );
    assert!(
        item.establishes_formatting_context(),
        "a flex item establishes an independent formatting context"
    );
}

#[test]
fn toggle_overflow_candidates_include_axis_pairing() {
    for other in ["hidden", "toggle(hidden,scroll)"] {
        let style = computed(&format!(
            "#parent{{overflow:auto}}#target{{overflow-y:{other};overflow-x:toggle(visible,scroll)}}"
        ));
        assert_eq!(
            style.overflow[0],
            crate::style::Overflow::Scroll,
            "visible computes to auto beside {other}, matches the parent's auto, then selects scroll"
        );
    }
}

#[test]
fn overflow_toggle_pairing_keeps_raw_values_and_avoids_false_cycles() {
    use crate::style::Overflow::{Auto, Clip, Hidden, Scroll, Visible};
    for (parent, target, expected) in [
        ("visible", "overflow:toggle(visible,clip)", [Clip, Clip]),
        ("clip", "overflow:toggle(visible,clip)", [Visible, Visible]),
        (
            "auto",
            "overflow-x:toggle(visible,clip);overflow-y:toggle(hidden,scroll)",
            [Hidden, Hidden],
        ),
        (
            "auto",
            "overflow-y:toggle(visible,clip);overflow-x:toggle(hidden,scroll)",
            [Hidden, Hidden],
        ),
        (
            "visible",
            "overflow-x:toggle(inherit,scroll);overflow-y:initial",
            [Scroll, Auto],
        ),
        (
            "auto",
            "overflow-x:toggle(initial,scroll);overflow-y:inherit",
            [Scroll, Auto],
        ),
    ] {
        let style = computed(&format!("#parent{{overflow:{parent}}}#target{{{target}}}"));
        assert_eq!(style.overflow, expected, "parent {parent}; target {target}");
    }
}

#[test]
fn mutually_dependent_overflow_toggles_report_authored_cycle_sources() {
    let error = computed_with(
        "#parent{overflow:auto}\n#target{overflow-x:toggle(visible,scroll);\noverflow-y:toggle(visible,scroll)}",
        &FileResolver::default(),
    ).expect_err("mutually dependent computed overflow axes must not guess a fixed point");
    let StyleError::DependencyCycle {
        sources,
        dependencies,
    } = error
    else {
        panic!("expected a typed computed-value dependency cycle, got {error:?}");
    };
    assert_eq!(dependencies, ["overflow-x", "overflow-y", "overflow-x"]);
    assert_eq!(
        sources.len(),
        2,
        "both authored declarations need source locations: {sources:?}"
    );
    assert_eq!(
        sources.iter().map(|source| source.line).collect::<Vec<_>>(),
        [2, 3]
    );
    assert!(sources.iter().all(|source| source.base == font_base()));
}

#[test]
fn toggle_display_comparison_uses_contents_exceptions_and_box_ancestry() {
    let image = computed_in(
        "#parent{display:none}#target{display:toggle(contents,block)}",
        "<div id=parent><img id=target></div>",
        "#target",
        &Default::default(),
        &FileResolver::default(),
    )
    .unwrap();
    assert_eq!(
        image.display,
        crate::style::Display::Block,
        "contents computes to none on img and matches the parent's computed none"
    );
    let item = computed_in(
        "#parent{display:flex}section{display:contents}#target{display:toggle(inline-block,inline-flex)}",
        "<div id=parent><section><p id=target>item</p></section></div>", "#target", &Default::default(), &FileResolver::default(),
    ).unwrap();
    assert_eq!(
        item.display,
        crate::style::Display::Block,
        "a boxless ancestor preserves the flex item context before candidate comparison"
    );
}

#[test]
fn computed_display_blockifies_roots_and_flex_items_before_value_comparison() {
    let root = computed_in(
        "html{display:inline-flex}",
        "<p>Root</p>",
        "html",
        &Default::default(),
        &FileResolver::default(),
    )
    .unwrap();
    assert_eq!(
        root.display,
        crate::style::Display::Flex,
        "the root's computed outer display is block"
    );
    let child = computed("#parent{display:flex}#target{display:inline-block}");
    assert_eq!(
        child.display,
        crate::style::Display::Block,
        "a flex item's inline-block computes to block flow"
    );
    assert!(
        child.establishes_formatting_context(),
        "a flex item establishes an independent formatting context"
    );
}

#[test]
fn toggle_rollback_keeps_filters_from_higher_priority_rollback_declarations() {
    let style = computed(
        "@layer first,second;#parent{width:17pt}@layer first{#target{width:17pt;width:revert-layer!important}}@layer second{#target{width:toggle(revert-layer,30pt)!important}}",
    );
    assert!(
        matches!(style.width, Size::Auto),
        "both important-layer rollbacks remove their layers, so normal width:17pt must not reappear: {:?}",
        style.width
    );
}

#[test]
fn inherited_value_references_read_the_parent_computed_tokens() {
    for (css, expected) in [
        (
            "#parent{--x:17pt}#target{--x:99pt;width:inherit(--x)}",
            17.0,
        ),
        ("html{--x:17pt}#target{--x:99pt;width:inherit(--x)}", 17.0),
        (
            "#parent{--seed:17pt;--x:var(--seed)}#target{--seed:99pt;width:inherit(--x)}",
            17.0,
        ),
        (
            "#parent{--x:2em}#target{font-size:20pt;width:inherit(--x)}",
            40.0,
        ),
        (r"#parent{--x:17pt}#target{width:inh\65 rit(--x)}", 17.0),
    ] {
        common::assert_pt(points(&computed(css).width), expected);
    }
}

#[test]
fn inherited_value_references_can_increment_the_same_custom_property() {
    let style = computed(
        "html{--n:5pt}#parent{--n:calc(inherit(--n,0pt) + 1pt)}#target{--n:calc(inherit(--n,0pt) + 1pt);width:var(--n)}",
    );
    common::assert_pt(points(&style.width), 7.0);
    let style = computed("#target{--n:calc(inherit(--n,5pt) + 1pt);width:var(--n)}");
    common::assert_pt(points(&style.width), 6.0);
}

#[test]
fn inherited_value_fallbacks_keep_local_lookup_and_empty_values_distinct() {
    for (css, expected) in [
        (
            "#parent{--x:17pt}#target{--fallback:29pt;width:inherit(--x,var(--fallback))}",
            17.0,
        ),
        (
            "#target{--fallback:29pt;width:inherit(--missing,var(--fallback))}",
            29.0,
        ),
        (
            "#parent{--x:17pt}#target{width:inherit(--missing,inherit(--x))}",
            17.0,
        ),
        ("#target{width:inherit(--missing,) 21pt}", 21.0),
        (
            "#parent{--empty:}#target{--empty:99pt;width:inherit(--empty,29pt) 21pt}",
            21.0,
        ),
        ("#parent{--x:initial}#target{width:inherit(--x,23pt)}", 23.0),
    ] {
        common::assert_pt(points(&computed(css).width), expected);
    }
    let style = computed("#target{width:11pt;width:inherit(--missing)}");
    assert!(
        matches!(style.width, Size::Auto),
        "a valid unresolved inheritance reference is invalid at computed time, without reviving width:11pt"
    );
    let style = computed_in(
        "html{--x:99pt;width:inherit(--x,13pt)}",
        "<p>Root</p>",
        "html",
        &Default::default(),
        &FileResolver::default(),
    )
    .unwrap();
    common::assert_pt(points(&style.width), 13.0);
}

#[test]
fn inherited_value_references_preserve_shorthands_and_rollback_keywords() {
    let style = computed(
        "#parent{--edges:2pt 3pt 4pt 5pt;width:17pt}#target{--edges:99pt;margin:inherit(--edges);width:inherit(--missing,inherit)}",
    );
    assert_eq!(
        style.margin.map(|value| points(&value)),
        [2.0, 3.0, 4.0, 5.0]
    );
    common::assert_pt(points(&style.width), 17.0);
    let style = computed(
        "@layer early,late;@layer early{#target{width:17pt}}@layer late{#target{width:31pt;width:inherit(--missing,revert-layer)}}",
    );
    common::assert_pt(points(&style.width), 17.0);
}

#[test]
fn inherited_value_syntax_and_token_boundaries_use_the_substitution_stage() {
    for invalid in [
        "inherit()",
        "inherit(width)",
        "inherit(--)",
        "inherit(--x 12pt)",
        "inherit(--x,!bad)",
    ] {
        let style = computed(&format!("#target{{width:11pt;width:{invalid}}}"));
        common::assert_pt(points(&style.width), 11.0);
    }
    let style = computed("#parent{--x:12}#target{width:11pt;width:inherit(--x)pt}");
    assert!(
        matches!(style.width, Size::Auto),
        "substituted number and following identifier must not merge into a dimension"
    );
    let style = computed(
        "#target{margin-left:7pt}@supports(width:inherit(--x)){#target{margin-left:13pt}}@supports(width:inherit(width)){#target{margin-left:99pt}}",
    );
    common::assert_pt(points(&style.margin[3]), 13.0);
}

#[test]
fn inherited_value_references_use_the_root_parent_for_page_geometry() {
    let trace = common::render_trace(
        "<style>html{--sheet:200pt 300pt;--edge:10pt} @page{--sheet:400pt 500pt;--edge:80pt;size:inherit(--sheet);margin:inherit(--edge);padding:0;border:0}body,p{margin:0;font:8pt serif}</style><p>Inherited page</p>",
    );
    common::assert_pt(trace.page_width, 200.0);
    common::assert_pt(trace.page_height, 300.0);
    common::assert_pt(trace.page_area.width, 180.0);
    common::assert_pt(trace.page_area.height, 280.0);
}

#[test]
fn inherited_value_fallbacks_follow_strict_comma_argument_rules() {
    let style = computed("#parent{--x:17pt}#target{width:11pt;width:inherit(--x,21pt,23pt)}");
    common::assert_pt(points(&style.width), 11.0);
    for css in [
        "#target{font-family:inherit(--missing,{Named,serif})}",
        "#parent{--family:Named,serif}#target{font-family:inherit(--family)}",
    ] {
        assert_eq!(
            computed(css).families,
            [
                crate::fonts::FontFamily::Named("Named".into()),
                crate::fonts::FontFamily::Generic(crate::fonts::GenericFamily::Serif)
            ],
            "comma lists arrive through one wrapped fallback or already-computed parent tokens: {css}"
        );
    }
}

#[test]
fn inherited_value_reference_cycles_keep_local_fallback_dependencies() {
    let style = computed("#parent{--n:17pt}#target{--n:inherit(--n,var(--n));width:var(--n,23pt)}");
    common::assert_pt(points(&style.width), 23.0);
    let style = computed("#parent{--n:17pt}#target{--n:inherit(--n);width:var(--n,23pt)}");
    common::assert_pt(points(&style.width), 17.0);
    let style = computed(
        "#target{--n:inherit(--missing,var(--seed));--seed:var(--other);--other:29pt;width:var(--n)}",
    );
    common::assert_pt(points(&style.width), 29.0);
}

#[test]
fn inherited_value_reference_nesting_keeps_the_shared_substitution_bound() {
    const NESTING: usize = 130;
    let mut source = "1pt".to_owned();
    for _ in 0..NESTING {
        source = format!("inherit(--missing,{source})");
    }
    let result = Stylesheet::parse(&[StylesheetSource {
        text: format!("#target{{width:{source}}}"),
        base: font_base(),
    }]);
    assert!(
        matches!(result, Err(StyleError::VariableLimit { .. })),
        "inherit-only substitutions must retain the value nesting quota"
    );
}

#[test]
fn calc_mix_interpolates_typed_values_without_clamping_progress() {
    for (source, expected) in [
        ("calc-mix(25%,10pt,30pt)", 15.0),
        ("calc-mix(.25,10pt,30pt)", 15.0),
        ("calc-mix(1.5,10pt,30pt)", 40.0),
        ("calc-mix(-1,10pt,30pt)", -10.0),
        ("calc-mix(calc(1 / 4),10pt,30pt)", 15.0),
        ("calc-mix(progress(1,0,4),10pt,30pt)", 15.0),
        ("calc-mix(media-progress(color,0,16),10pt,30pt)", 20.0),
        ("calc-mix(0,10pt,30pt)", 10.0),
        ("calc-mix(1,10pt,30pt)", 30.0),
        ("calc-mix(50%,1in,2.54cm)", 72.0),
        ("calc(calc-mix(.5,1Hz,3Hz) / 1Hz * 1pt)", 2.0),
        ("calc(calc-mix(.5,1s,3000ms) / 1s * 1pt)", 2.0),
        ("calc(calc-mix(.5,1dppx,3dppx) / 1dppx * 1pt)", 2.0),
    ] {
        let style = computed(&format!("#target{{margin-left:{source}}}"));
        assert_eq!(
            points(&style.margin[3]),
            expected,
            "interpolation uses the typed endpoints and unbounded progress: {source}"
        );
    }
    let style = computed(
        "#target{order:calc-mix(.25,1,7);z-index:calc-mix(.25,-1,-7);font-style:oblique calc-mix(.25,0deg,40deg)}",
    );
    assert_eq!(style.flex.order, 3);
    assert_eq!(style.z_index, Some(-2));
    common::assert_pt(style.font_slant.value(), 10.0);
}

#[test]
fn calc_mix_easing_covers_bezier_steps_and_piecewise_linear_functions() {
    for (progress, easing, expected) in [
        (".5", "linear", 50.0),
        ("2", "linear", 200.0),
        (".5", "ease", 80.24034),
        (".5", "ease-in", 31.53568),
        (".5", "ease-out", 68.46432),
        (".5", "ease-in-out", 50.0),
        (".125", "cubic-bezier(0,0,0,1)", 50.0),
        (".875", "cubic-bezier(1,0,1,0)", 12.5),
        ("-.5", "cubic-bezier(.5,1,1,1)", -100.0),
        ("1.5", "cubic-bezier(0,0,.5,0)", 200.0),
        (".3", "steps(4)", 25.0),
        ("0", "steps(4,start)", 25.0),
        ("1", "steps(4,jump-start)", 100.0),
        ("0", "steps(4,jump-both)", 20.0),
        (".5", "steps(4,jump-none)", 66.666_664),
        ("-.25", "steps(4,end)", -25.0),
        ("1.25", "steps(4,end)", 125.0),
        (".5", "step-start", 100.0),
        (".5", "step-end", 0.0),
        (".2", "linear(0 20%,.5 10%,1)", 50.0),
        (".5", "linear(0,.25 25% 75%,1)", 25.0),
        (".875", "linear(0,.25 25% 75%,1)", 62.5),
        ("1.5", "linear(0,.25,1)", 175.0),
        (".5", "linear(0% 0,100% 1)", 50.0),
    ] {
        let source = format!("calc-mix({progress} by {easing},0pt,100pt)");
        let style = computed(&format!("#target{{margin-left:{source}}}"));
        let actual = points(&style.margin[3]);
        assert!(
            (actual - expected).abs() <= 0.001,
            "easing must map the input progress before interpolation: {source}: expected {expected}, got {actual}"
        );
    }
    common::assert_pt(
        points(&computed("#target{margin-left:calc-mix(by ease-in-out .5,0pt,100pt)}").margin[3]),
        50.0,
    );
}

#[test]
fn calc_mix_keeps_used_value_progress_and_endpoints_deferred() {
    for (source, zero, hundred, two_hundred) in [
        ("calc-mix(.5,10pt,50%)", 5.0, 30.0, 55.0),
        ("calc-mix(0,10pt,50%)", 10.0, 10.0, 10.0),
        ("calc-mix(calc(50% / 10pt),10pt,30pt)", 10.0, 110.0, 210.0),
        (
            "calc-mix(calc(50% / 100pt) by steps(4,end),10pt,30pt)",
            10.0,
            20.0,
            30.0,
        ),
    ] {
        let style = computed(&format!("#target{{width:{source}}}"));
        assert_eq!(
            style.width.resolve_definite(None),
            None,
            "indefinite remains distinct from zero: {source}"
        );
        for (basis, expected) in [(0.0, zero), (100.0, hundred), (200.0, two_hundred)] {
            common::assert_pt(style.width.resolve_definite(Some(basis)).unwrap(), expected);
        }
    }
}

#[test]
fn calc_mix_rejects_invalid_types_easing_and_arity_before_cascading() {
    for source in [
        "calc-mix(.5,1pt,2s)",
        "calc-mix(calc(100% / 7),10pt,30pt)",
        "calc-mix(.5,10pt)",
        "calc-mix(.5,10pt,30pt,40pt)",
        "calc-mix(.5 by invalid,10pt,30pt)",
        "calc-mix(.5 by steps(0),10pt,30pt)",
        "calc-mix(.5 by steps(1,jump-none),10pt,30pt)",
        "calc-mix(.5 by cubic-bezier(-1,0,1,1),10pt,30pt)",
        "calc-mix(.5 by linear(0),10pt,30pt)",
        "calc-mix(.5 by linear(0,1) by ease,10pt,30pt)",
        "calc-mix(none,10pt,30pt)",
        "calc-mix(auto,10pt,30pt)",
    ] {
        let style = computed(&format!("#target{{margin-left:11pt;margin-left:{source}}}"));
        assert_eq!(
            points(&style.margin[3]),
            11.0,
            "invalid notation must retain a valid preceding declaration: {source}"
        );
    }
    let style =
        computed("#target{--end:1s;margin-left:11pt;margin-left:calc-mix(.5,10pt,var(--end))}");
    assert_eq!(
        points(&style.margin[3]),
        0.0,
        "a substituted type failure is invalid at computed-value time"
    );
}

#[test]
fn calc_mix_uses_element_fonts_variables_and_capability_checks() {
    let resolver = RecordingResolver {
        inner: FileResolver::default(),
        requests: Default::default(),
    };
    let style=computed_with("@font-face{font-family:Metric;src:url(FontStyleTest-Ital.ttf)}#target{font-family:Metric;font-size:20pt;--end:2ch;width:calc-mix(.5,1em,var(--end));height:first-valid(calc-mix(.25,10pt,30pt),99pt)}@supports(width:calc-mix(.5 by steps(2),10pt,30pt)){#target{margin-left:13pt}}",&resolver).unwrap();
    common::assert_pt(points(&style.width), 22.0);
    common::assert_pt(points(&style.height), 15.0);
    common::assert_pt(points(&style.margin[3]), 13.0);
    assert_eq!(&*resolver.requests.borrow(), &["FontStyleTest-Ital.ttf"]);
}

#[test]
fn calc_mix_reports_excluded_animation_timelines_without_fabricating_progress() {
    for timeline in ["--clock", "scroll()", "view()"] {
        let result = computed_with(
            &format!("#target{{width:calc-mix({timeline},10pt,30pt)}}"),
            &FileResolver::default(),
        );
        assert!(
            matches!(result,Err(StyleError::Unsupported(ref message)) if message.contains("timeline")),
            "valid timeline syntax must report the approved profile exclusion: {timeline}: {result:?}"
        );
    }
    let style = computed(
        "#target{width:11pt}@supports(width:calc-mix(--clock,10pt,30pt)){#target{width:99pt}}",
    );
    common::assert_pt(points(&style.width), 11.0);
}

#[test]
fn calc_mix_easing_parameters_keep_element_context_and_computed_ranges() {
    for (easing, expected) in [
        ("steps(calc(-10),start)", 100.0),
        ("steps(calc(1),jump-none)", 100.0),
        ("steps(calc(5 / 2),start)", 66.666_664),
    ] {
        let style = computed(&format!(
            "#target{{margin-left:calc-mix(.5 by {easing},0pt,100pt)}}"
        ));
        common::assert_pt(points(&style.margin[3]), expected);
    }
    let resolver = RecordingResolver {
        inner: FileResolver::default(),
        requests: Default::default(),
    };
    let style=computed_with("@font-face{font-family:Metric;src:url(FontStyleTest-Ital.ttf)}#target{font-family:Metric;font-size:20pt;width:calc-mix(.5 by cubic-bezier(0,calc(1ch / 1pt),1,0),0pt,100pt)}",&resolver).unwrap();
    common::assert_pt(points(&style.width), 462.5);
    assert_eq!(&*resolver.requests.borrow(), &["FontStyleTest-Ital.ttf"]);
}

#[test]
fn calc_mix_retains_ieee_intermediates_and_receiving_property_ranges() {
    let style = computed(
        "#target{margin-left:calc(7pt + calc-mix(calc(NaN),10pt,30pt));margin-right:calc(7pt + calc-mix(0,1pt,calc(infinity * 1pt)));font-size:calc-mix(-1,10pt,30pt);order:calc-mix(.5,20000000,20000000)}",
    );
    common::assert_pt(points(&style.margin[3]), 0.0);
    common::assert_pt(points(&style.margin[1]), 0.0);
    common::assert_pt(style.size, 0.0);
    assert_eq!(style.flex.order, 20_000_000);
}

#[test]
fn calc_mix_nested_easing_retains_a_shared_work_quota() {
    const NESTING: usize = 26;
    let mut control = ".5".to_owned();
    for _ in 0..NESTING {
        control = format!("calc-mix(.5 by cubic-bezier({control},0,1,1),0,1)");
    }
    let result = Stylesheet::parse(&[StylesheetSource {
        text: format!("#target{{width:calc-mix(.5 by cubic-bezier({control},0,1,1),0pt,100pt)}}"),
        base: font_base(),
    }]);
    assert!(
        matches!(result, Err(StyleError::GrammarLimit { .. })),
        "nested easing reparses must consume the enclosing work quota"
    );
}

#[test]
fn media_progress_uses_the_shared_range_feature_values_and_types() {
    for (source, expected) in [
        ("media-progress(width,0pt,1000pt)", 60.0),
        ("media-progress(device-width,0pt,1000pt)", 60.0),
        ("media-progress(height,0pt,1000pt)", 80.0),
        ("media-progress(device-height,0pt,1000pt)", 80.0),
        ("media-progress(aspect-ratio,0,1)", 75.0),
        ("media-progress(device-aspect-ratio,0,1)", 75.0),
        ("media-progress(color,0,16)", 50.0),
        ("media-progress(color-index,0,1)", 0.0),
        ("media-progress(monochrome,0,1)", 0.0),
        ("media-progress(horizontal-viewport-segments,0,2)", 50.0),
        ("media-progress(vertical-viewport-segments,0,2)", 50.0),
        ("media-progress(width,0pt,200pt)", 300.0),
        ("media-progress(width,1000pt,0pt)", 40.0),
        ("media-progress(width,-200pt,200pt)", 200.0),
        ("media-progress(width,0,1000pt)", 60.0),
        ("media-progress(width,100pt + 100pt,1200pt)", 40.0),
        ("media-progress(color,.5,16.5)", 43.75),
        (
            "media-progress(width,media-progress(color,0,16) * 400pt,1200pt)",
            40.0,
        ),
    ] {
        let style = media_computed(
            &format!("#target{{margin-left:calc({source} * 100pt)}}"),
            &FileResolver::default(),
        );
        common::assert_pt(points(&style.margin[3]), expected);
    }
    let style = media_computed(
        "#target{width:calc(media-progress(width,0pt,1000pt) * 100%)}",
        &FileResolver::default(),
    );
    assert_eq!(
        style.width.resolve_definite(None),
        None,
        "the enclosing property keeps its indefinite percentage basis"
    );
    common::assert_pt(style.width.resolve_definite(Some(200.0)).unwrap(), 120.0);
}

#[test]
fn media_progress_endpoints_use_initial_fonts_and_the_media_viewport() {
    let resolver = RecordingResolver {
        inner: FileResolver::default(),
        requests: Default::default(),
    };
    for (endpoint, expected) in [("50em", 30.0), ("100ch", 30.0), ("50vw", 60.0)] {
        let style = media_computed(
            &format!(
                "@font-face{{font-family:Metric;src:url(FontStyleTest-Ital.ttf)}}html{{font-size:80pt}}#target{{font-family:Metric;font-size:100pt;margin-left:calc(media-progress(width,0pt,{endpoint}) * 30pt)}}"
            ),
            &resolver,
        );
        common::assert_pt(points(&style.margin[3]), expected);
    }
    assert!(
        resolver.requests.borrow().is_empty(),
        "media endpoints must not fetch the styled element's author fonts"
    );
}

#[test]
fn media_progress_rejects_invalid_features_and_feature_value_types() {
    for source in [
        "media-progress(grid,0,1)",
        "media-progress(orientation,0,1)",
        "media-progress(unknown,0,1)",
        "media-progress(min-width,0pt,100pt)",
        "media-progress(width,0%,100%)",
        "media-progress(width,0pt,100%)",
        "media-progress(width,0pt,100)",
        "media-progress(color,0pt,16pt)",
        "media-progress(width,0pt)",
        "media-progress(width,0pt,100pt,200pt)",
        "media-progress(width from 0pt to 100pt)",
        "media-progress(aspect-ratio,-1,1)",
        "media-progress(aspect-ratio,0,-1/2)",
        "media-progress(width,sibling-index() * 1pt,100pt)",
    ] {
        let style = media_computed(
            &format!("#target{{margin-left:11pt;margin-left:calc({source} * 1pt)}}"),
            &FileResolver::default(),
        );
        assert_eq!(
            points(&style.margin[3]),
            11.0,
            "invalid feature syntax must retain the earlier declaration: {source}"
        );
    }
    let style = media_computed(
        "#target{--end:100%;width:17pt;width:calc(media-progress(width,0pt,var(--end)) * 1pt)}",
        &FileResolver::default(),
    );
    assert!(
        matches!(style.width, Size::Auto),
        "feature percentages remain invalid after substitution even in a percentage-capable property"
    );
    let style = media_computed(
        "#target{width:first-valid(calc(media-progress(width,0pt,1000pt) * 100pt),99pt)}@supports (width:calc(media-progress(width,0pt,100%)*1pt)){#target{height:99pt}}@supports (opacity:media-progress(color,0,16)){#target{margin-left:13pt}}",
        &FileResolver::default(),
    );
    common::assert_pt(points(&style.width), 60.0);
    assert!(matches!(style.height, Size::Auto));
    common::assert_pt(points(&style.margin[3]), 13.0);
}

#[test]
fn media_progress_applies_media_value_boundaries_before_interpolation() {
    for (source, expected) in [
        ("media-progress(width,600pt,600pt)", 7.0),
        ("media-progress(width,500pt,500pt)", 8.0),
        ("media-progress(width,700pt,700pt)", 6.0),
        ("media-progress(width,0pt,infinity * 1pt)", 8.0),
        ("media-progress(width,infinity * 1pt,infinity * 1pt)", 6.0),
        ("media-progress(width,NaN * 1pt,1000pt)", 8.0),
        ("media-progress(resolution,1dppx,2dppx)", 8.0),
    ] {
        let style = media_computed(
            &format!("#target{{margin-left:calc(7pt + sign({source}) * 1pt)}}"),
            &FileResolver::default(),
        );
        common::assert_pt(points(&style.margin[3]), expected);
    }
}

#[test]
fn media_progress_in_font_face_descriptors_uses_the_configured_paper() {
    for (descriptor, alternative, requested) in [
        (
            "font-weight:calc(media-progress(width,0pt,1000pt) * 1000)",
            "font-weight:650",
            "font-weight:600",
        ),
        (
            "font-width:calc(media-progress(width,0pt,1000pt) * 200%)",
            "font-width:130%",
            "font-width:120%",
        ),
        (
            "font-style:oblique calc(media-progress(width,0pt,1000pt) * 30deg)",
            "font-style:oblique 19deg",
            "font-style:oblique 18deg",
        ),
    ] {
        let resolver = RecordingResolver {
            inner: FileResolver::default(),
            requests: Default::default(),
        };
        let style = media_computed(
            &format!(
                "@font-face{{font-family:Media;src:url(FontStyleTest-Ital.ttf);{descriptor}}}@font-face{{font-family:Media;src:url(MetricGlyphTest.ttf);{alternative}}}#target{{font-family:Media;{requested};font-size:20pt;width:1ch}}"
            ),
            &resolver,
        );
        assert_eq!(
            points(&style.width),
            FIXTURE_ZERO_PT,
            "the configured media value must control the selected face: {descriptor}"
        );
        assert_eq!(
            &*resolver.requests.borrow(),
            &["FontStyleTest-Ital.ttf"],
            "descriptor evaluation must select the face for the actual print environment: {descriptor}"
        );
    }
}

#[test]
fn media_progress_keeps_indirect_paper_dependencies_in_page_size_conditions() {
    let mut options = common::local_options();
    options.print.paper = MEDIA_PAPER_PT;
    let trace=crate::render_html("<style>@page{size:200pt 300pt;margin:0}body,p{margin:0;font:8pt serif}@media(color > calc(media-progress(width,0pt,1000pt) * 10)){p{font-size:21pt}@page{size:400pt 500pt}}</style><p>Media</p>",&options).unwrap().layout.unwrap();
    common::assert_pt(trace.pages[0][0].font_size, 21.0);
    common::assert_pt(trace.page_width, 200.0);
    common::assert_pt(trace.page_height, 300.0);
}

#[test]
fn tree_counting_functions_resolve_against_element_siblings_in_typed_math() {
    let style = computed_in(
        "#target{order:sibling-count();z-index:sibling-index();font-size:calc(sibling-index() * 6pt);line-height:sibling-count();width:calc(100% / sibling-count());margin-left:calc(sibling-index() * 3pt);opacity:calc(sibling-count() / 8);color:rgb(calc(sibling-index() * 50) 0 0);--index:sibling-index();height:calc(var(--index) * 4pt)}",
        "<section>text<!--comment--><i></i><p id=target>0</p><span hidden></span><b style='display:none'></b></section>",
        "#target", &crate::style::PrintSettings::default(), &FileResolver::default(),
    ).unwrap();
    assert_eq!(
        style.flex.order, 4,
        "hidden elements count; text and comments do not"
    );
    assert_eq!(
        style.z_index,
        Some(2),
        "the index starts at one and includes every element type"
    );
    common::assert_pt(style.size, 12.0);
    assert_eq!(style.line_height, crate::style::LineHeight::Number(4.0));
    common::assert_pt(style.width.resolve_definite(Some(200.0)).unwrap(), 50.0);
    assert_eq!(style.width.resolve_definite(None), None);
    common::assert_pt(points(&style.margin[3]), 6.0);
    common::assert_pt(style.opacity, 0.5);
    assert_eq!(style.color.rgb, [100, 0, 0]);
    common::assert_pt(points(&style.height), 8.0);
}

#[test]
fn tree_counting_functions_preserve_syntax_validation_and_consumer_context() {
    for invalid in [
        "sibling-count(1)",
        "sibling-index(,)",
        "sibling-index(of p)",
    ] {
        let style = computed(&format!("#target{{order:11;order:{invalid}}}"));
        assert_eq!(
            style.flex.order, 11,
            "tree-counting functions take no arguments: {invalid}"
        );
    }
    let style = computed_in(
        "section{--index:sibling-index()}#target{order:SiBlInG-CoUnT();z-index:sibling-\\69ndex();height:calc(var(--index) * 7pt);width:first-valid(calc(sibling-index() * 6pt),99pt)}@supports(width:calc(sibling-count() * 1pt)){#target{margin-left:14pt}}",
        "<section><i></i><p id=target>0</p><b></b></section>","#target", &crate::style::PrintSettings::default(), &FileResolver::default(),
    ).unwrap();
    assert_eq!(style.flex.order, 3);
    assert_eq!(style.z_index, Some(2));
    common::assert_pt(points(&style.height), 14.0);
    common::assert_pt(points(&style.width), 12.0);
    common::assert_pt(points(&style.margin[3]), 14.0);
}

#[test]
fn tree_counting_uses_the_pinned_flat_tree_instead_of_light_siblings() {
    let html = "<div id=host><template shadowrootmode=open><slot name=second></slot><slot name=first></slot></template><p id=one slot=first>One</p><p id=two slot=second>Two</p><p id=three slot=second>Three</p><p>Unassigned</p></div>";
    let css = "p{order:sibling-count();z-index:sibling-index()}";
    for (selector, count, index) in [("#one", 1, 1), ("#two", 2, 1), ("#three", 2, 2)] {
        let style = computed_in(
            css,
            html,
            selector,
            &crate::style::PrintSettings::default(),
            &FileResolver::default(),
        )
        .unwrap();
        assert_eq!(
            (style.flex.order, style.z_index),
            (count, Some(index)),
            "slot assignment defines the flat sibling group for {selector}"
        );
    }
    let source = format!(
        "<style>@page{{size:200pt 300pt;margin:0}}body,p{{margin:0;font:8pt serif}}p::before{{display:block;content:'Before';font-size:calc(sibling-index() * 8pt)}}p:not(#three)::before{{content:none}}</style>{html}"
    );
    let trace = common::render_trace(&source);
    let line = trace
        .pages
        .iter()
        .flatten()
        .find(|line| line.text == "Before")
        .expect("originating-element pseudo must print");
    common::assert_pt(line.font_size, 16.0);
}

#[test]
fn tree_counting_root_and_page_contexts_do_not_invent_an_element_for_media() {
    let trace = common::render_trace(
        "<style>html{font-size:calc(sibling-count() * 10pt)}@page{size:calc(sibling-count() * 200pt) calc(sibling-index() * 300pt);margin:calc(sibling-count() * 10pt);font-size:calc(sibling-index() * 12pt)}body,p{margin:0}p{font-size:inherit}@media(width > calc(sibling-count() * 1pt)){p{font-size:99pt}}</style><p>Root</p>",
    );
    common::assert_pt(trace.page_width, 200.0);
    common::assert_pt(trace.page_height, 300.0);
    common::assert_pt(trace.page_area.x, 10.0);
    common::assert_pt(trace.pages[0][0].font_size, 10.0);
}

#[test]
fn tree_counting_stays_invalid_in_font_descriptors_and_negated_media_queries() {
    let resolver = RecordingResolver {
        inner: FileResolver::default(),
        requests: Default::default(),
    };
    let style = computed_with(
        "@font-face{font-family:Metric;src:url(FontStyleTest-Ital.ttf);font-weight:400;font-weight:calc(sibling-count() * 700)}#target{font-family:Metric;font-size:20pt;width:1ch;margin-left:12pt}@supports-condition --font{@font-face{font-weight:sibling-count()}}@supports (--font){#target{margin-left:99pt}}@supports-condition --page{@page{size:calc(sibling-count() * 200pt) 300pt}}@supports (--page){#target{height:14pt}}@media not (width > calc(sibling-index() * 1pt)){#target{margin-left:98pt}}",
        &resolver,
    ).unwrap();
    common::assert_pt(points(&style.width), FIXTURE_ZERO_PT);
    common::assert_pt(points(&style.margin[3]), 12.0);
    common::assert_pt(points(&style.height), 14.0);
    assert_eq!(&*resolver.requests.borrow(), &["FontStyleTest-Ital.ttf"]);
}

#[test]
fn comma_containing_arguments_unwrap_one_syntactic_brace_group() {
    for (source, expected) in [
        ("first-valid({12pt},20pt)", 12.0),
        ("first-valid(red,{calc(10pt + 2pt)})", 12.0),
        ("first-valid({{12pt}},20pt)", 20.0),
        ("var(--missing,{12pt})", 12.0),
        ("var(--missing,{}) 12pt", 12.0),
        ("var(--missing,{/* empty */}) 12pt", 12.0),
        ("var(--missing,{var(--other,{12pt})})", 12.0),
    ] {
        let style = computed(&format!("#target{{margin-left:{source}}}"));
        assert_eq!(
            points(&style.margin[3]),
            expected,
            "only the argument's syntactic wrapper is removed: {source}"
        );
    }
    for source in [
        "first-valid(rgb(1,2,3),{Named,serif})",
        "var(--missing,{Named,serif})",
        "var(--missing,Named,serif)",
    ] {
        let style = computed(&format!("#target{{font-family:{source}}}"));
        assert_eq!(
            style.families,
            [
                crate::fonts::FontFamily::Named("Named".into()),
                crate::fonts::FontFamily::Generic(crate::fonts::GenericFamily::Serif)
            ],
            "a wrapped comma list remains one argument: {source}"
        );
    }
}

#[test]
fn comma_argument_wrappers_enforce_the_complete_argument_grammar() {
    for source in [
        "first-valid({12pt} junk,20pt)",
        "first-valid(junk {12pt},20pt)",
        "first-valid({12pt}{13pt},20pt)",
        "first-valid({},20pt)",
        "first-valid({/* empty */},20pt)",
        "first-valid({12pt!important},20pt)",
        "first-valid({12pt;},20pt)",
        "var(--missing,{12pt} 13pt)",
        "var(--missing,{12pt},13pt)",
    ] {
        let style = computed(&format!("#target{{margin-left:11pt;margin-left:{source}}}"));
        assert_eq!(
            points(&style.margin[3]),
            11.0,
            "malformed argument syntax is rejected before cascading: {source}"
        );
    }
    let defined = computed("#target{--n:12pt;margin-left:11pt;margin-left:var(--n,{12pt},13pt)}");
    assert_eq!(
        points(&defined.margin[3]),
        11.0,
        "unused fallbacks still require valid syntax"
    );
}

#[test]
fn argument_wrappers_follow_variable_substitution_and_keep_nested_block_data() {
    let style = computed(
        "#target{--list:Named,serif;--n:12pt;font-family:first-valid({var(--list)},monospace);margin-left:first-valid({var(--n)},20pt);--data:var(--missing,{foo({raw:1;})});width:first-valid(var(--data),13pt);--invalid:{raw:1;};height:11pt;height:first-valid(var(--invalid),14pt)}",
    );
    assert_eq!(
        style.families,
        [
            crate::fonts::FontFamily::Named("Named".into()),
            crate::fonts::FontFamily::Generic(crate::fonts::GenericFamily::Serif)
        ]
    );
    assert_eq!(points(&style.margin[3]), 12.0);
    assert_eq!(points(&style.width), 13.0);
    assert!(
        matches!(style.height, Size::Auto),
        "a wrapper exposed by substitution must still obey declaration-value restrictions at computed-value time"
    );
}

#[test]
fn first_valid_selects_the_first_usable_whole_property_value() {
    let style = computed(
        "#target{width:first-valid(fit-content(10pt), calc(40pt + 2pt), 99pt);margin:first-valid(red, 1pt 2pt 3pt 4pt);margin-left:9pt;border:first-valid(12pt invalid,2pt solid rgb(20 40 60));color:FiRsT-VaLiD(17pt,rgb(30,60,90))}",
    );
    assert_eq!(
        points(&style.width),
        42.0,
        "recognizing fit-content() syntax must not select an unsupported active value"
    );
    assert_eq!(
        style.margin.each_ref().map(points),
        [1.0, 2.0, 3.0, 9.0],
        "the selected shorthand expands without defeating later longhands"
    );
    assert!(
        style
            .decoration
            .borders
            .iter()
            .all(|border| border.used_width() == 2.0 && border.color.rgb == [20, 40, 60])
    );
    assert_eq!(style.color.rgb, [30, 60, 90]);
    let deferred = computed("#target{width:first-valid(red,calc(50% - 10pt),12pt)}");
    assert_eq!(deferred.width.resolve_definite(None), None);
    assert_eq!(deferred.width.resolve_definite(Some(200.0)), Some(90.0));
    let escaped = computed(r"#target{margin-left:f\69 rst-valid(red,13pt)}");
    assert_eq!(points(&escaped.margin[3]), 13.0);
    let font =
        computed("#target{font:first-valid(invalid-font,italic 12pt/18pt serif);font-size:16pt}");
    assert_eq!(font.size, 16.0);
    assert_eq!(font.line_height.resolve(font.size), Some(18.0));
    assert_eq!(font.font_style, crate::fonts::FontStyle::Italic);
}

#[test]
fn first_valid_distinguishes_invalid_syntax_from_computed_failure() {
    for source in [
        "first-valid(red,blue)",
        "first-valid(first-valid(red),15pt)",
    ] {
        let style = computed(&format!("#target{{margin-left:11pt;margin-left:{source}}}"));
        assert_eq!(
            points(&style.margin[3]),
            0.0,
            "no valid choice makes the winner invalid at computed-value time: {source}"
        );
    }
    for source in [
        "first-valid()",
        "first-valid(,12pt)",
        "first-valid(12pt,)",
        "first-valid(12pt) 1pt",
        "calc(first-valid(12pt))",
        "first-valid(12pt, red !important)",
        "first-valid(12pt,var(b))",
        "first-valid(red,var(b),12pt)",
        "first-valid(12pt,var())",
        r"first-valid(12pt,v\61r(b))",
    ] {
        let style = computed(&format!("#target{{margin-left:11pt;margin-left:{source}}}"));
        assert_eq!(
            points(&style.margin[3]),
            11.0,
            "malformed or non-whole values are rejected at parse time: {source}"
        );
    }
    let inherited =
        computed("#parent{color:green}#target{color:red;color:first-valid(12pt,invalid-color)}");
    assert_eq!(
        inherited.color.rgb,
        [0, 128, 0],
        "computed failure inherits instead of reviving red"
    );
    let nested = computed("#target{margin-left:first-valid(red,first-valid(blue,13pt),15pt)}");
    assert_eq!(points(&nested.margin[3]), 13.0);
}

#[test]
fn first_valid_observes_substitution_before_contextual_selection() {
    let nonwhole =
        computed("#target{--n:12pt;margin-left:11pt;margin-left:calc(first-valid(var(--n)))}");
    assert_eq!(
        points(&nonwhole.margin[3]),
        0.0,
        "a whole-value function stays invalid inside numeric syntax after substitution"
    );
    let opaque = computed(
        "#target{--x:foo(a!important);--y:foo({raw:var(--present);});--present:yes;width:first-valid(var(--x),13pt);height:first-valid(var(--y),14pt)}",
    );
    assert_eq!(
        points(&opaque.width),
        13.0,
        "opaque block tokens remain valid custom-property data before contextual selection"
    );
    assert_eq!(points(&opaque.height), 14.0);
    let malformed =
        computed("#target{--n:9pt;--n:first-valid(12pt,var(b));margin-left:var(--n,17pt)}");
    assert_eq!(
        points(&malformed.margin[3]),
        9.0,
        "malformed substitution syntax invalidates a custom declaration before cascading"
    );
    let invalid_template = computed(
        "#parent{--none:first-valid(red,blue);--partial:first-valid(red,12pt) extra}#target{margin-top:8pt;margin-top:var(--none,15pt);margin-bottom:8pt;margin-bottom:var(--partial)}",
    );
    assert_eq!(
        points(&invalid_template.margin[0]),
        0.0,
        "a token-valid custom value does not activate the var fallback"
    );
    assert_eq!(
        points(&invalid_template.margin[2]),
        0.0,
        "custom tokens become invalid only in the consuming property's grammar"
    );
    let style = computed(
        "#parent{--choice:first-valid(red,15pt);--args:red,13pt;--empty:;--n:12}#target{--bad:blue;margin-left:first-valid(var(--bad),17pt);width:var(--choice);color:var(--choice);height:first-valid(var(--args),20pt);padding-left:var(--empty) first-valid(red,9pt);margin-right:first-valid(var(--n)pt,21pt)}",
    );
    assert_eq!(points(&style.margin[3]), 17.0);
    assert_eq!(
        points(&style.width),
        15.0,
        "custom properties retain the tokens for their consuming property's grammar"
    );
    assert_eq!(style.color.rgb, [255, 0, 0]);
    assert_eq!(
        points(&style.height),
        13.0,
        "substituted commas participate in the function's argument grammar"
    );
    assert_eq!(style.padding[3].resolve(200.0), 9.0);
    assert_eq!(
        points(&style.margin[1]),
        21.0,
        "substitution cannot join separate tokens into a dimension"
    );
    for source in [
        "first-valid(var(--missing),17pt)",
        "first-valid(17pt,var(--missing))",
    ] {
        let style = computed(&format!("#target{{margin-left:11pt;margin-left:{source}}}"));
        assert_eq!(
            points(&style.margin[3]),
            0.0,
            "failed arbitrary substitution precedes value introspection: {source}"
        );
    }
}

#[test]
fn first_valid_wide_keywords_use_the_common_cascade() {
    let inherited =
        computed("#parent{margin-left:6pt}#target{margin-left:first-valid(red,inherit,20pt)}");
    assert_eq!(points(&inherited.margin[3]), 6.0);
    let rollback = computed(
        "@layer base,later;@layer base{#target{margin-left:7pt}}@layer later{#target{margin-left:12pt;margin-left:first-valid(red,revert-layer,20pt)}}",
    );
    assert_eq!(
        points(&rollback.margin[3]),
        7.0,
        "selected rollback keywords must recascade normally"
    );
}

#[test]
fn first_valid_page_properties_share_selection_and_defaulting() {
    let trace = common::render_trace(
        "<style>@page{size:400pt 600pt;size:first-valid(invalid-size,200pt 300pt);margin:first-valid(red,10pt);padding:0;border:0;bleed:first-valid(red,4pt);marks:first-valid(red,crop)}body{margin:0}p{margin:0;font:8pt serif}</style><p>First valid</p>",
    );
    common::assert_pt(trace.page_width, 200.0);
    common::assert_pt(trace.page_height, 300.0);
    common::assert_pt(trace.page_area.x, 10.0);
    common::assert_pt(trace.page_area.width, 180.0);
    let reset = common::render_trace(
        "<style>@page{size:200pt 300pt;size:first-valid(red,blue);margin:0}body{margin:0}p{margin:0;font:8pt serif}</style><p>Default paper</p>",
    );
    common::assert_pt(reset.page_width, common::local_options().print.paper[0]);
    let malformed = common::render_trace(
        "<style>@page{size:400pt 600pt;size:first-valid(var(b),200pt 300pt);margin:0}body{margin:0}p{margin:0;font:8pt serif}</style><p>Previous page</p>",
    );
    common::assert_pt(malformed.page_width, 400.0);
}

#[test]
fn first_valid_support_queries_and_metric_resources_share_capabilities() {
    let resolver = RecordingResolver {
        inner: FileResolver::default(),
        requests: Default::default(),
    };
    let face = "@font-face{font-family:Metric;src:url(FontStyleTest-Ital.ttf)}#target{font-family:Metric;font-size:20pt}";
    let unused = computed_with(
        &format!("{face}#target{{width:first-valid(13pt,1ch)}}"),
        &resolver,
    )
    .unwrap();
    assert_eq!(points(&unused.width), 13.0);
    assert!(
        resolver.requests.borrow().is_empty(),
        "unselected metric candidates must not fetch fonts"
    );
    let css = format!(
        "{face}#target{{width:5pt}}@supports(width:first-valid(red,1ch)){{#target{{width:13pt}}}}@supports(width:first-valid(red,blue)){{#target{{height:14pt}}}}@supports(width:first-valid()){{#target{{width:99pt}}}}"
    );
    let style = computed_with(&css, &resolver).unwrap();
    assert_eq!(points(&style.width), 13.0);
    assert_eq!(
        points(&style.height),
        14.0,
        "the recognized notation may fail only at computed-value time"
    );
    assert!(
        resolver.requests.borrow().is_empty(),
        "capability queries must not fetch author fonts"
    );
    let style = computed_with(
        &format!("{face}#target{{width:first-valid(red,1ch,12pt)}}"),
        &resolver,
    )
    .unwrap();
    assert_eq!(points(&style.width), FIXTURE_ZERO_PT);
    assert_eq!(&*resolver.requests.borrow(), &["FontStyleTest-Ital.ttf"]);
}

#[test]
fn first_valid_uses_pseudo_and_flat_tree_contexts() {
    let trace = common::render_trace(
        "<style>@page{size:200pt 300pt;margin:0}body,p{margin:0;font:8pt serif}p{--marker:first-valid(12pt,'Before ')}p::before{content:var(--marker)}p::after{content:first-valid(12pt,' After')}</style><p>Body</p>",
    );
    let printed = trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(
        printed, "Before Body After",
        "whole-value selection must use the generated content property's grammar"
    );
    for (rules, expected) in [
        ("body,p{margin:0;font:8pt serif}", 8.0),
        ("body{margin:0;font:8pt serif}p{margin:0}", 19.0),
    ] {
        let source = format!(
            "<style>@page{{size:200pt 300pt;margin:0}}{rules}#host{{--choice:first-valid(red,17pt)}}</style><div id=host><template shadowrootmode=open><style>slot{{--choice:first-valid(red,19pt)}}::slotted(p){{font-size:var(--choice)}}</style><slot></slot></template><p>Slotted</p></div>"
        );
        let trace = common::render_trace(&source);
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "Slotted")
            .expect("slotted text must print");
        common::assert_pt(line.font_size, expected);
    }
}

#[test]
fn first_valid_candidates_retain_one_matching_work_budget() {
    const CANDIDATES: usize = 1_100_000;
    for (property, fallback) in [("width", "12pt"), ("flex-grow", "1")] {
        let source = format!(
            "#target{{{property}:first-valid({}{fallback})}}",
            "x,".repeat(CANDIDATES)
        );
        let result = Stylesheet::parse(&[StylesheetSource {
            text: source,
            base: font_base(),
        }]);
        assert!(
            matches!(
                result,
                Err(StyleError::GrammarLimit {
                    resource: "matching steps",
                    ..
                })
            ),
            "candidate matching must retain one quota even for primitive grammar nodes: {property}"
        );
    }
}

#[test]
fn finite_math_values_preserve_the_range_already_supported_by_literals() {
    for magnitude in [20_000_000.0_f32, 1.0e20, f32::MAX] {
        let literal = computed(&format!(
            "#target{{margin-left:{magnitude:e}pt;font-size:{magnitude:e}pt;line-height:1pt;flex-grow:{magnitude:e};font-width:{magnitude:e}%}}"
        ));
        let math = computed(&format!(
            "#target{{margin-left:calc({magnitude:e}pt);font-size:calc({magnitude:e}pt);line-height:1pt;flex-grow:calc({magnitude:e});font-width:calc({magnitude:e}%)}}"
        ));
        assert_eq!(points(&literal.margin[3]), magnitude);
        assert_eq!(
            points(&math.margin[3]),
            magnitude,
            "calc must preserve a representable length: {magnitude}"
        );
        assert_eq!(
            math.size, literal.size,
            "font-size must have the same finite range for math and literals: {magnitude}"
        );
        assert_eq!(
            math.flex.grow, literal.flex.grow,
            "numbers must have the same finite range for math and literals: {magnitude}"
        );
        assert_eq!(
            math.font_width, literal.font_width,
            "percentage conversion must preserve the font interface's finite range: {magnitude}"
        );
    }
    let positive = computed(
        "#target{margin-left:calc(infinity * 1pt);font-size:calc(infinity * 1pt);line-height:1pt;flex-grow:calc(infinity);font-width:calc(infinity * 1%)}",
    );
    assert_eq!(
        points(&positive.margin[3]),
        f32::MAX,
        "infinity clamps to the supported finite length boundary"
    );
    assert_eq!(positive.size, f32::MAX);
    assert_eq!(positive.flex.grow, f32::MAX);
    assert_eq!(positive.font_width.value(), f32::MAX);
    assert_eq!(
        points(&computed("#target{margin-left:calc(-infinity * 1pt)}").margin[3]),
        f32::MIN
    );
}

#[test]
fn deferred_special_math_keeps_its_basis_until_used_value_time() {
    for (source, at_zero, at_hundred) in [
        ("calc(7pt + pow(0% / 0pt, 0) * 1pt)", 0.0, 0.0),
        (
            "calc(7pt + hypot(1pt * 1pt / (50% - 50%), 0pt / 0))",
            0.0,
            0.0,
        ),
        (
            "calc(7pt + sign(log((50% - 50%) / 1pt, .5)) * 1pt)",
            6.0,
            6.0,
        ),
        ("calc(sign(1 / mod(-100%, 20pt)) * 10pt)", 10.0, 10.0),
        (
            "calc(round(up, 50% * 1e-30, 1e30pt) / 1e30pt * 10pt)",
            0.0,
            10.0,
        ),
    ] {
        let style = computed(&format!("#target{{width:{source}}}"));
        assert_eq!(
            style.width.resolve_definite(None),
            None,
            "unknown geometry must remain deferred: {source}"
        );
        for (basis, expected) in [(0.0, at_zero), (100.0, at_hundred)] {
            common::assert_pt(
                style.width.resolve_definite(Some(basis)).expect(source),
                expected,
            );
        }
    }
}

#[test]
fn math_functions_keep_nan_infectious_through_special_cases() {
    let mut failures = Vec::new();
    for source in [
        "pow(NaN, 0)",
        "pow(1, NaN)",
        "pow(1, infinity)",
        "pow(-1, -infinity)",
        "hypot(infinity, NaN)",
        "hypot(NaN, infinity)",
        "round(infinity, NaN)",
        "round(NaN, infinity)",
        "min(1, NaN)",
        "max(NaN, 1)",
        "clamp(0, NaN, 1)",
        "mod(NaN, 1)",
        "rem(1, NaN)",
        "abs(NaN)",
        "sign(NaN)",
        "sin(NaN)",
        "cos(NaN)",
        "tan(NaN)",
        "asin(NaN) / 1deg",
        "acos(NaN) / 1deg",
        "atan(NaN) / 1deg",
        "atan2(NaN, 1) / 1deg",
        "sqrt(NaN)",
        "log(NaN)",
        "log(1, NaN)",
        "exp(NaN)",
        "progress(NaN, 0, 1)",
    ] {
        let style = computed(&format!(
            "#target{{margin-left:calc(7pt + ({source}) * 1pt)}}"
        ));
        let actual = points(&style.margin[3]);
        if actual != 0.0 {
            failures.push((source, actual));
        }
    }
    assert!(
        failures.is_empty(),
        "NaN must survive surrounding arithmetic until the property boundary: {failures:#?}"
    );
}

#[test]
fn math_zero_signs_survive_comparisons_moduli_and_domain_rules() {
    let mut failures = Vec::new();
    for (source, expected) in [
        ("min(0, -1 * 0)", -10.0),
        ("min(-1 * 0, 0)", -10.0),
        ("max(0, -1 * 0)", 10.0),
        ("max(-1 * 0, 0)", 10.0),
        ("clamp(0, -1 * 0, 1)", 10.0),
        ("mod(-4, 2)", 10.0),
        ("mod(4, -2)", -10.0),
        ("mod(-1 * 0, 2)", 10.0),
        ("mod(0, -2)", -10.0),
        ("rem(-4, 2)", -10.0),
        ("rem(4, -2)", 10.0),
        ("round(up, -.25, 1)", -10.0),
        ("round(down, .25, 1)", 10.0),
        ("round(-.25, 1)", -10.0),
        ("round(.25, 1)", 10.0),
        ("round(to-zero, -.25, 1)", -10.0),
        ("round(-1 * 0, 1)", -10.0),
        ("round(up, -1 * 0, 1)", -10.0),
        ("round(down, -1 * 0, 1)", -10.0),
        ("round(to-zero, -1 * 0, 1)", -10.0),
        ("hypot(-1 * 0)", 10.0),
        ("abs(-1 * 0)", 10.0),
        ("sqrt(-1 * 0)", -10.0),
        ("sin(-1 * 0)", -10.0),
        ("tan(-1 * 0)", -10.0),
        ("asin(-1 * 0) / 1deg", -10.0),
        ("atan(-1 * 0) / 1deg", -10.0),
        ("log(1, .5)", 10.0),
        ("log(1, 0)", 10.0),
        ("pow(-1 * 0, 3)", -10.0),
        ("pow(-infinity, -3)", -10.0),
        ("exp(-infinity)", 10.0),
    ] {
        let style = computed(&format!(
            "#target{{margin-left:calc(sign(1 / ({source})) * 10pt)}}"
        ));
        let actual = points(&style.margin[3]);
        if actual != expected {
            failures.push((source, actual, expected));
        }
    }
    assert!(
        failures.is_empty(),
        "nested arithmetic must distinguish positive and negative zero: {failures:#?}"
    );
}

#[test]
fn logarithm_domains_follow_the_pinned_publication() {
    let mut failures = Vec::new();
    for (value, base, expected) in [
        ("0", ".5", 6.0),
        ("0", "0", 6.0),
        ("infinity", ".5", 8.0),
        ("infinity", "0", 8.0),
        ("2", ".5", 6.0),
        (".5", ".5", 8.0),
        ("-2", "2", 0.0),
        ("2", "-2", 0.0),
        ("1", "1", 0.0),
        ("0", "1", 0.0),
        ("infinity", "1", 0.0),
    ] {
        let style = computed(&format!(
            "#target{{margin-left:calc(7pt + sign(log({value}, {base})) * 1pt)}}"
        ));
        let actual = points(&style.margin[3]);
        if actual != expected {
            failures.push((value, base, actual, expected));
        }
    }
    assert!(
        failures.is_empty(),
        "logarithm argument rules precede general finite evaluation: {failures:#?}"
    );
}

#[test]
fn stepped_math_handles_finite_values_with_extreme_quotients() {
    for (source, expected) in [
        ("calc(round(1pt, 1e-40pt) * 20)", 20.0),
        ("calc(round(up, 1e-30pt, 1e30pt) / 1e30pt * 10pt)", 10.0),
        ("calc(round(down, -1e-30pt, 1e30pt) / 1e30pt * 10pt)", -10.0),
    ] {
        let style = computed(&format!("#target{{margin-left:{source}}}"));
        common::assert_pt(points(&style.margin[3]), expected);
    }
}

#[test]
fn integer_math_preserves_large_values_and_rounds_half_towards_positive_infinity() {
    for (source, expected) in [
        ("8388609", 8_388_609),
        ("-8388609", -8_388_609),
        ("20000000", 20_000_000),
        ("-20000000", -20_000_000),
        ("2147483647", i32::MAX),
        ("-2147483648", i32::MIN),
        ("infinity", i32::MAX),
        ("-infinity", i32::MIN),
        ("NaN", 0),
        ("2.5", 3),
        ("-2.5", -2),
        ("-.5", 0),
        ("8388607.5", 8_388_608),
        ("-8388607.5", -8_388_607),
    ] {
        let style = computed(&format!(
            "#target{{order:calc({source});z-index:calc({source})}}"
        ));
        assert_eq!(
            style.flex.order, expected,
            "integer math must round within the integer domain: {source}"
        );
        assert_eq!(
            style.z_index,
            Some(expected),
            "all integer consumers must use the same conversion: {source}"
        );
    }
    let positive = computed("#target{widows:calc(NaN);orphans:calc(-infinity)}");
    assert_eq!(
        positive.widows, 1,
        "NaN becomes zero before the property's positive range is applied"
    );
    assert_eq!(
        positive.orphans, 1,
        "negative infinity clamps at the property's lower bound"
    );
}

#[test]
fn deferred_zero_percentages_remain_distinct_from_absolute_zero() {
    for (source, expected) in [
        ("calc(0%)", 0.0),
        ("calc(50% - 50%)", 0.0),
        ("calc(0 * 50%)", 0.0),
        ("calc(10pt + 0%)", 10.0),
    ] {
        let style = computed(&format!("#target{{width:{source}}}"));
        assert_eq!(
            style.width.resolve_definite(None),
            None,
            "an explicit percentage retains an indefinite basis: {source}"
        );
        assert_eq!(
            style.width.resolve_definite(Some(0.0)),
            Some(expected),
            "zero is a definite basis: {source}"
        );
        assert_eq!(
            style.width.resolve_definite(Some(200.0)),
            Some(expected),
            "a supplied basis resolves the expression: {source}"
        );
    }
}

#[test]
fn deferred_simplification_keeps_zero_terms_that_control_ieee_signs() {
    let style =
        computed("#target{margin-left:calc(sign(1 / ((1pt - 1pt) + min(-1%, 0pt) * 0)) * 10pt)}");
    assert_eq!(style.margin[3].resolve_definite(None), None);
    assert_eq!(
        style.margin[3].resolve_definite(Some(200.0)),
        Some(10.0),
        "the explicit positive zero must make +0 plus a deferred negative zero positive"
    );
}

#[test]
fn unhinted_percentage_ratios_keep_number_and_percentage_types_distinct() {
    for source in ["calc(50% / 100%)", "min(50%, 75%) / 100%"] {
        let value = if source.starts_with("min") {
            format!("calc({source})")
        } else {
            source.into()
        };
        let style = computed(&format!("#target{{opacity:{value}}}"));
        assert_eq!(
            style.opacity, 0.5,
            "percentage cancellation must produce a number: {value}"
        );
    }
    let invalid = computed("#target{opacity:.75;opacity:calc(.25 + 25%)}");
    assert_eq!(
        invalid.opacity, 0.75,
        "numbers and percentages cannot be added in opacity"
    );
    let width = computed("#target{font-width:calc(25% + 50%)}");
    assert_eq!(
        width.font_width,
        crate::fonts::FontWidth::CONDENSED,
        "a percentage function must retain percentage-point units at the font interface"
    );
}

#[test]
fn math_literal_zero_is_unsigned_while_derived_zero_keeps_its_internal_sign() {
    for (source, expected) in [
        ("calc(sign(1 / -0) * 10pt)", 10.0),
        ("calc(sign(1 / -0pt) * 10pt)", 10.0),
        ("calc(sign(1 / -0%) * 10pt)", 10.0),
        ("calc(sign(1 / (-1 * 0)) * 10pt)", -10.0),
    ] {
        let style = computed(&format!("#target{{margin-left:{source}}}"));
        assert_eq!(
            style.margin[3].resolve_definite(Some(200.0)),
            Some(expected),
            "literal/derived zero: {source}"
        );
    }
    let style = computed("#target{margin-left:calc(-1 * 0pt)}");
    assert!(
        points(&style.margin[3]).is_sign_positive(),
        "negative zero is censored only at the property boundary"
    );
}

#[test]
fn math_colors_use_the_element_font_context_before_being_inherited() {
    for (expression, expected) in [
        ("calc(1em / 1rem * 100)", 167),
        ("calc(progress(2em, 1rem, 4rem) * 255)", 198),
        ("calc(1lh / 1rlh * 100)", 133),
    ] {
        let style = computed(&format!(
            "html{{font-size:12pt;line-height:30pt}}#parent{{font-size:20pt;line-height:40pt;color:rgb({expression} 0 0)}}#target{{font-size:40pt}}"
        ));
        assert_eq!(
            style.color.rgb,
            [expected, 0, 0],
            "colour math computes in the declaring parent's font context, then inherits its result: {expression}"
        );
    }
}

#[test]
fn static_progress_uses_the_pinned_grammar_and_numeric_types() {
    for (source, expected) in [
        ("progress(1, 0, 2)", 0.5),
        ("PrOgReSs(1, 0, 2)", 0.5),
        ("progress(250ms, 0s, 1s)", 0.25),
        ("progress(1turn, 0deg, 720deg)", 0.5),
        ("progress(1khz, 0hz, 2khz)", 0.5),
        ("progress(96dpi, 0dppx, 2dppx)", 0.5),
        ("progress(72pt, 0px, 1in)", 1.0),
        ("progress(75%, 0%, 100%)", 0.75),
        ("progress(min(25%, 75%), 0%, 100%)", 0.25),
    ] {
        let style = computed(&format!("#target{{opacity:{source}}}"));
        assert_eq!(
            style.opacity, expected,
            "progress must cancel consistent argument dimensions: {source}"
        );
    }
    for (source, expected) in [
        ("progress(3, 0, 2)", 30.0),
        ("progress(-1, 0, 2)", -10.0),
        ("progress(2, 3, 1)", 10.0),
    ] {
        let style = computed(&format!("#target{{margin-left:calc({source} * 20pt)}}"));
        assert_eq!(
            points(&style.margin[3]),
            expected,
            "progress itself is not clamped: {source}"
        );
    }
    let style = computed(
        "html{font-size:12pt}#target{font-size:20pt;margin-left:calc(progress(2em, 1rem, 4rem) * 36pt);font-width:calc(progress(75%, 0%, 100%) * 100%);color:rgb(calc(progress(3, 0, 4) * 255) 0 0)}",
    );
    common::assert_pt(points(&style.margin[3]), 28.0);
    assert_eq!(style.font_width, crate::fonts::FontWidth::CONDENSED);
    assert_eq!(style.color.rgb, [191, 0, 0]);
}

#[test]
fn progress_keeps_deferred_bases_and_handles_collapsed_ranges() {
    let style = computed("#target{width:calc(progress(50%, 0pt, 200pt) * 100pt)}");
    assert_eq!(
        style.width.resolve_definite(None),
        None,
        "an indefinite basis remains indefinite"
    );
    assert_eq!(style.width.resolve_definite(Some(0.0)), Some(0.0));
    assert_eq!(style.width.resolve_definite(Some(100.0)), Some(25.0));
    assert_eq!(style.width.resolve_definite(Some(200.0)), Some(50.0));
    let collapsed = computed("#target{width:calc(7pt + progress(50%, 25%, 25%) * 1pt)}");
    assert_eq!(collapsed.width.resolve_definite(None), None);
    assert_eq!(
        collapsed.width.resolve_definite(Some(0.0)),
        Some(7.0),
        "equal used endpoints and value yield zero progress, not NaN"
    );
}

#[test]
fn progress_special_values_are_preserved_until_the_property_boundary() {
    for (source, expected) in [
        ("calc(7pt + progress(2, 2, 2) * 1pt)", 7.0),
        (
            "calc(7pt + progress(infinity, infinity, infinity) * 1pt)",
            7.0,
        ),
        ("calc(1e7pt / progress(3, 2, 2))", 0.0),
        ("calc(7pt + progress(NaN, 2, 2) * 1pt)", 0.0),
        ("calc(7pt + progress(infinity, 0, infinity) * 1pt)", 0.0),
        ("calc(10pt * sign(1 / progress(-0, 0, 1)))", 10.0),
        ("calc(10pt * sign(1 / progress(-1 * 0, 0, 1)))", -10.0),
    ] {
        let style = computed(&format!("#target{{margin-left:1pt;margin-left:{source}}}"));
        assert_eq!(
            points(&style.margin[3]),
            expected,
            "progress special values: {source}"
        );
    }
}

#[test]
fn progress_validation_support_queries_and_substitution_share_the_evaluator() {
    for invalid in [
        "progress()",
        "progress(1)",
        "progress(1, 0)",
        "progress(1, 0, 2, 3)",
        "progress(1pt, 0px, 2s)",
        "progress(50%, 0, 1)",
        "progress(1 from 0 to 2)",
    ] {
        let style = computed(&format!("#target{{opacity:.75;opacity:{invalid}}}"));
        assert_eq!(
            style.opacity, 0.75,
            "invalid typed/arity/grammar progress must recover: {invalid}"
        );
    }
    let style = computed(
        "#target{--value:25%;--range:0%,100%;opacity:progress(var(--value),var(--range))}",
    );
    assert_eq!(style.opacity, 0.25);
    let style = computed("#target{--value:1s;opacity:.75;opacity:progress(var(--value),0pt,2pt)}");
    assert_eq!(
        style.opacity, 1.0,
        "invalid substitution resets the winning opacity instead of reviving the previous one"
    );
    let style = computed(
        "#target{margin-left:3pt}@supports(opacity:progress(50%,0%,100%)){#target{margin-left:17pt}}@supports(width:progress(1,0,2)){#target{margin-left:99pt}}@supports(opacity:progress(1 from 0 to 2)){#target{margin-left:99pt}}",
    );
    common::assert_pt(points(&style.margin[3]), 17.0);
}

#[test]
fn metric_fallback_matches_the_rendering_face() {
    for (italic_source, expected, family) in [
        ("MetricGlyphTest.ttf", SERIF_ZERO_PT, "Liberation Serif"),
        ("missing.ttf", FIXTURE_ZERO_PT, "Metric"),
    ] {
        let css = format!(
            "@font-face{{font-family:Metric;src:url(FontStyleTest-Ital.ttf);font-style:normal}}@font-face{{font-family:Metric;src:url({italic_source});font-style:italic}}p{{font-family:Metric,serif;font-style:italic;font-synthesis:none;font-size:{FONT_SIZE_PT}pt;line-height:24pt;width:1ch;margin:0}}"
        );
        let mut options = common::local_options();
        options.base = font_base();
        let trace = crate::render_html(&format!("<style>{css}</style><p>0</p>"), &options)
            .unwrap()
            .layout
            .unwrap();
        let line = &trace.pages[0][0];
        common::assert_pt(line.width, expected);
        assert_eq!(trace.fonts[line.runs[0].font_id.index()].family, family);
        common::assert_pt(points(&computed(&css).width), expected);
    }
}

#[test]
fn missing_x_and_cap_metrics_use_defined_fallbacks() {
    let style = computed(
        "@font-face{font-family:Metric;src:url(MetricGlyphTest.ttf)}#target{font-family:Metric;font-size:20pt;width:1ex;height:1cap;margin-left:1lh}",
    );
    common::assert_pt(points(&style.width), 10.0);
    common::assert_pt(points(&style.height), 19.0);
    common::assert_pt(points(&style.margin[3]), 24.0);
}

#[test]
fn zero_font_and_line_metrics_remain_valid_css_values() {
    for (css, size, leading) in [
        ("#target{font-size:0}", 0.0, None),
        ("#target{line-height:0}", 12.0, Some(0.0)),
        ("#target{line-height:0pt}", 12.0, Some(0.0)),
        (
            "#target{font-size:0;line-height:0;width:1ex;height:1lh}",
            0.0,
            Some(0.0),
        ),
    ] {
        let style = computed(css);
        common::assert_pt(style.size, size);
        assert_eq!(style.line_height.resolve(style.size), leading, "{css}");
    }
    let html = format!(
        "<style>{}@page{{size:300pt 200pt;margin:20pt}}p{{margin:0;font:20pt/24pt serif}}span{{font-size:0;letter-spacing:3pt;word-spacing:2pt;line-height:0}}</style><p>A<span>a b</span>B</p>",
        common::PROFILE
    );
    let output = crate::render_html(&html, &common::local_options())
        .expect("zero-size text must retain finite layout and explicit spacing");
    let trace = output.layout.unwrap();
    let line = &trace.pages[0][0];
    let zero = line
        .runs
        .iter()
        .find(|run| run.text == "a b")
        .expect("zero-size text participates in layout");
    common::assert_pt(zero.font_size, 0.0);
    common::assert_pt(zero.width, 11.0);
    assert!(line.width.is_finite());
}

const TWO_FONTS: &str = "@font-face{font-family:Root;src:url(SourceSerif4-Regular.ttf)}@font-face{font-family:Body;src:url(RobotoFlex.ttf)}html{font-family:Root;font-size:40pt;line-height:60pt;font-optical-sizing:none}body{font-family:Body;font-size:10pt;line-height:15pt}";

#[test]
fn font_root_and_line_height_units_use_the_correct_metrics() {
    let style = computed(&format!(
        "{TWO_FONTS}#target{{font-size:20pt;line-height:normal;margin:1ex 1cap 1ch 1ic;padding:1rem 1rex 1rcap 1rch;width:1lh;height:1rlh}}"
    ));
    for (side, expected) in [
        1052.0 / 2048.0 * 20.0,
        1456.0 / 2048.0 * 20.0,
        1156.0 / 2048.0 * 20.0,
        20.0,
    ]
    .into_iter()
    .enumerate()
    {
        common::assert_pt(points(&style.margin[side]), expected);
    }
    for (side, expected) in [40.0, 19.0, 26.8, 20.0].into_iter().enumerate() {
        common::assert_pt(style.padding[side].resolve(0.0), expected);
    }
    common::assert_pt(points(&style.width), 2400.0 / 2048.0 * 20.0);
    common::assert_pt(points(&style.height), 60.0);
}

#[test]
fn root_font_size_uses_initial_relative_basis() {
    let style = computed(
        "@font-face{font-family:Root;src:url(SourceSerif4-Regular.ttf)}html{font-family:Root;font-size:2rex;line-height:2rlh}#target{font-size:20pt;margin-left:1rem;margin-right:1rex;padding-top:1rlh}",
    );
    let root_size = 2.0 * 940.0 / 2048.0 * 12.0;
    common::assert_pt(points(&style.margin[3]), root_size);
    common::assert_pt(points(&style.margin[1]), root_size * 0.475);
    common::assert_pt(style.padding[0].resolve(0.0), 2.0 * 2355.0 / 2048.0 * 12.0);
}

#[test]
fn nested_math_keeps_metric_dependencies() {
    let style = computed(&format!(
        "{TWO_FONTS}#target{{font-family:Root;font-size:calc(2ex + 1ch);line-height:1.5;margin-left:max(1ch,min(2ex,1cap));margin-right:calc(1rch + 2rem);height:calc(1lh + 2rlh);width:calc(50% + 1ch + 1rex)}}"
    ));
    let size = (1052.0 * 2.0 + 1156.0) / 2048.0 * 10.0;
    common::assert_pt(style.size, size);
    common::assert_pt(points(&style.margin[3]), size * 0.67);
    common::assert_pt(points(&style.margin[1]), 100.0);
    common::assert_pt(points(&style.height), size * 1.5 + 120.0);
    let Size::Value(width) = &style.width else {
        panic!("mixed math must retain its deferred percentage")
    };
    common::assert_pt(width.resolve(200.0), 100.0 + size * 0.5 + 19.0);
    common::assert_pt(width.resolve(400.0), 200.0 + size * 0.5 + 19.0);
}

struct RecordingResolver {
    inner: FileResolver,
    requests: std::cell::RefCell<Vec<String>>,
}
impl ResourceResolver for RecordingResolver {
    fn resolve(
        &self,
        reference: &str,
        base: &crate::ResourceBase,
    ) -> Result<crate::resources::Resource, crate::resources::ResourceError> {
        self.requests.borrow_mut().push(reference.into());
        self.inner.resolve(reference, base)
    }
}

#[test]
fn inactive_metric_values_and_support_queries_do_not_load_author_faces() {
    let resolver = RecordingResolver {
        inner: FileResolver::default(),
        requests: Default::default(),
    };
    let face =
        "@font-face{font-family:Metric;src:url(MetricGlyphTest.ttf)}#target{font-family:Metric}";
    let style = computed_with(&format!("{face}#target{{width:1ex;width:12pt}}@media not print{{#target{{height:1ch}}}}@supports(width:calc(1ch + 1ex)){{#target{{height:13pt}}}}"), &resolver).unwrap();
    common::assert_pt(points(&style.width), 12.0);
    common::assert_pt(points(&style.height), 13.0);
    assert!(
        resolver.requests.borrow().is_empty(),
        "inactive values or capability queries must not resolve the selected font: {:?}",
        resolver.requests.borrow()
    );
    let style = computed_with(
        &format!("{face}#target{{width:1ex;font-size:20pt}}"),
        &resolver,
    )
    .unwrap();
    common::assert_pt(points(&style.width), 10.0);
    assert_eq!(&*resolver.requests.borrow(), &["MetricGlyphTest.ttf"]);
}

#[test]
fn math_color_metrics_load_only_for_the_computed_value() {
    let resolver = RecordingResolver {
        inner: FileResolver::default(),
        requests: Default::default(),
    };
    let face = "@font-face{font-family:Metric;src:url(FontStyleTest-Ital.ttf)}#target{font-family:Metric;font-size:20pt}";
    let channel = "calc(1ch / 1pt * 10)";
    let style = computed_with(
        &format!("{face}#target{{color:black}}@supports(color:rgb({channel} 0 0)){{#target{{width:13pt}}}}"),
        &resolver,
    ).unwrap();
    common::assert_pt(points(&style.width), 13.0);
    assert!(
        resolver.requests.borrow().is_empty(),
        "colour capability queries must not fetch font metrics"
    );
    let style = computed_with(
        &format!("{face}#target{{color:rgb({channel} 0 0)}}"),
        &resolver,
    )
    .unwrap();
    assert_eq!(
        style.color.rgb,
        [(FIXTURE_ZERO_PT * 10.0) as u8, 0, 0],
        "the channel must measure the selected author face"
    );
    assert_eq!(&*resolver.requests.borrow(), &["FontStyleTest-Ital.ttf"]);
}

#[test]
fn active_metric_policy_failures_are_not_font_availability_fallbacks() {
    const RESOURCE_LIMIT_BYTES: usize = 1;
    let resolver = FileResolver {
        max_resource_bytes: RESOURCE_LIMIT_BYTES,
        ..Default::default()
    };
    for declaration in [
        "width:1ch",
        "color:rgb(calc(1ch / 1pt * 10) 0 0)",
        "width:first-valid(1ch,12pt)",
        "width:calc-mix(.5 by cubic-bezier(0,calc(1ch / 1pt),1,0),0pt,100pt)",
        "width:inherit(--missing,1ch)",
        "width:toggle(1pt,1ch)",
        "width:if(media(print):1ch;else:12pt)",
        "width:if(style(1ch > 0pt):12pt;else:14pt)",
        "width:attr(data-missing <length>,1ch)",
    ] {
        let error = computed_with(&format!("@font-face{{font-family:Metric;src:url(MetricGlyphTest.ttf)}}#target{{font-family:Metric;{declaration}}}"), &resolver).unwrap_err();
        assert!(
            matches!(
                error,
                StyleError::Font(crate::fonts::FontError::Resource(
                    crate::resources::ResourceError::TooLarge {
                        limit: RESOURCE_LIMIT_BYTES,
                        ..
                    }
                ))
            ),
            "active metric loading must preserve the resource error for {declaration}: {error}"
        );
    }
}

#[test]
fn ideographic_units_measure_real_fallback_glyphs_including_zero_advance() {
    for (source, expected) in [("IcTestHalfWidth.ttf", 10.0), ("IcTestZeroWidth.ttf", 0.0)] {
        let css = format!(
            "@font-face{{font-family:Latin;src:url(MetricGlyphTest.ttf)}}@font-face{{font-family:Water;src:url({source})}}#target{{font-family:Latin,Water;font-size:20pt;line-height:24pt;width:1ic;margin:0}}"
        );
        common::assert_pt(points(&computed(&css).width), expected);
        let mut options = common::local_options();
        options.base = font_base();
        let trace = crate::render_html(
            &format!("<style>{css}</style><p id=target>水</p>"),
            &options,
        )
        .unwrap()
        .layout
        .unwrap();
        let line = &trace.pages[0][0];
        common::assert_pt(line.width, expected);
        assert_eq!(line.text, "水");
        assert_eq!(trace.fonts[line.runs[0].font_id.index()].family, "Water");
        assert_eq!(line.runs[0].glyphs()[0].id, 3);
    }
}

#[test]
fn page_font_size_em_ex_uses_initial_metrics_without_resetting_other_bases() {
    const INITIAL_X_HEIGHT_PT: f32 = 940.0 / 2048.0 * 12.0;
    for (declarations, margin) in [
        ("font-size:2em;margin-left:1em", 24.0),
        ("font-size:2ex;margin-left:1em", 2.0 * INITIAL_X_HEIGHT_PT),
        ("font-size:calc(1em + 50%);margin-left:1em", 24.5),
        ("font-size:200%;margin-left:1em", 50.0),
        ("font-size:larger;margin-left:1em", 30.0),
        ("font-size:2em;margin-left:1ex", 1120.0 / 2048.0 * 24.0),
        ("font-size:2em;margin-left:1ch", 1303.0 / 2048.0 * 24.0),
        (
            "font-size:2em;font-weight:calc(30 * 1em / 1pt);margin-left:1ch",
            1425.0 / 2048.0 * 24.0,
        ),
        ("font-size:2em;line-height:3;margin-left:1lh", 72.0),
        ("font-size:2em;margin-left:1rem", 25.0),
    ] {
        let html = format!(
            "<style>{}@page{{size:400pt 600pt;margin:0;{declarations}}}html{{font-family:'DejaVu Sans';font-size:25pt}}</style><p>Page font context</p>",
            common::PROFILE
        );
        let trace = crate::render_html(&html, &common::local_options())
            .unwrap_or_else(|error| panic!("page font {declarations}: {error}"))
            .layout
            .unwrap();
        common::assert_pt(trace.page_area.x, margin);
        common::assert_pt(trace.page_area.width, 400.0 - margin);
    }
}

#[test]
fn page_root_units_do_not_measure_the_page_as_the_document_root() {
    let mut sheet = Stylesheet::parse(&[StylesheetSource {
        text: "@page{size:400pt 600pt;font-size:30pt;margin:1rem 0 0 2em}".into(),
        base: Default::default(),
    }])
    .unwrap();
    sheet.resolve_page_geometry(&Style::default()).unwrap();
    common::assert_pt(sheet.margin[0], 12.0);
    common::assert_pt(sheet.margin[3], 60.0);
}

#[test]
fn page_geometry_resolves_acyclic_cross_axis_dependencies() {
    for (css, paper, area) in [
        (
            "@page{size:20em 600pt;font-size:1vh;margin:20pt}",
            [112.0, 600.0],
            [72.0, 560.0],
        ),
        (
            "@page{size:400pt 20em;font-size:1vw;margin:20pt}",
            [400.0, 72.0],
            [360.0, 32.0],
        ),
        (
            "@page{size:400pt 600pt;font-size:12pt;margin:1em}html{font-size:10vw;width:50vw}",
            [400.0, 600.0],
            [376.0, 576.0],
        ),
    ] {
        let trace = common::render_trace(&format!(
            "<style>{}body,p{{margin:0;font:8pt serif}}{css}</style><p>Axis</p>",
            common::PROFILE
        ));
        common::assert_pt(trace.page_width, paper[0]);
        common::assert_pt(trace.page_height, paper[1]);
        common::assert_pt(trace.page_area.width, area[0]);
        common::assert_pt(trace.page_area.height, area[1]);
    }
}

#[test]
fn undefined_geometry_cycles_are_reported_instead_of_using_provisional_values() {
    for css in [
        "@page{size:400pt 600pt;margin:2em}html{font-size:1vw}",
        "@page{size:100vw 600pt;margin:20pt}",
        "@page{size:400pt 600pt;margin:1vh}",
        "@page{size:400pt 600pt;margin:20pt;padding-left:1vw}",
        "@page{size:400pt 600pt;margin:20pt;border:1vw solid}",
    ] {
        let result = crate::render_html(
            &format!("<style>{css}</style><p>Cycle</p>"),
            &common::local_options(),
        );
        let Err(error) = result else {
            panic!("geometry cycle was silently accepted: {css}")
        };
        assert!(
            matches!(error, crate::Error::Css(_)),
            "cycle must be a style dependency error: {error}"
        );
        assert!(
            error.to_string().contains("cycle"),
            "cycle must retain its disposition: {error}"
        );
    }
}

#[test]
fn viewport_units_use_the_first_page_area_in_root_page_and_margin_contexts() {
    for (suffix, expected) in [
        ("vw", 36.0),
        ("vi", 36.0),
        ("vh", 50.0),
        ("vb", 50.0),
        ("vmin", 36.0),
        ("vmax", 50.0),
        ("svw", 36.0),
        ("lvw", 36.0),
        ("dvw", 36.0),
        ("svh", 50.0),
        ("lvh", 50.0),
        ("dvh", 50.0),
        ("svi", 36.0),
        ("lvi", 36.0),
        ("dvi", 36.0),
        ("svb", 50.0),
        ("lvb", 50.0),
        ("dvb", 50.0),
        ("svmin", 36.0),
        ("lvmin", 36.0),
        ("dvmin", 36.0),
        ("svmax", 50.0),
        ("lvmax", 50.0),
        ("dvmax", 50.0),
    ] {
        let html = format!(
            "<style>@page{{size:400pt 600pt;margin:50pt 20pt;font-size:1{suffix};@top-center{{content:'M';font-size:2em}}}}html{{font-size:1{suffix}}}body,p{{margin:0}}p{{width:10{suffix};height:12pt;background:#123456}}</style><p>B</p>"
        );
        let trace = common::render_trace(&html);
        let body = trace.pages[0]
            .iter()
            .find(|line| line.text == "B")
            .expect("body text");
        let margin = trace.pages[0]
            .iter()
            .find(|line| line.text == "M")
            .expect("margin text");
        common::assert_pt(body.font_size, expected / 10.0);
        common::assert_pt(margin.font_size, expected / 5.0);
        let paint = trace.display_lists[0]
            .iter()
            .find_map(|item| match item {
                crate::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0x12, 0x34, 0x56] =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .expect("viewport-sized box");
        common::assert_pt(paint.rect.width, expected);
    }
    let trace = common::render_trace(
        "<style>@page{size:400pt 600pt;margin:50pt 20pt;@top-center{content:'M';font-size:2vh}}body,p{margin:0;font:8pt serif}</style><p>B</p>",
    );
    let margin = trace.pages[0].iter().find(|line| line.text == "M").unwrap();
    common::assert_pt(margin.font_size, 10.0);
}

#[test]
fn geometry_cycle_reports_winning_declarations_and_the_closed_dependency_chain() {
    for css in [
        "@page{size:400pt 600pt;margin:2em}\nhtml{font-size:1vw}",
        "@page{size:400pt 600pt;margin:2em}\nhtml{font-size:calc(1vw + 1pt)}",
        "@page{size:400pt 600pt;margin:2em}\nhtml{--size:1vw;font-size:var(--size)}",
        "@layer low,high;@layer low{@page{size:400pt 600pt;margin:2em}}@layer high{@page{margin:revert-layer}}\nhtml{font-size:1vw}",
    ] {
        let mut sheet = Stylesheet::parse(&[StylesheetSource {
            text: css.into(),
            base: font_base(),
        }])
        .unwrap();
        let tree = DocumentTree::parse("<p>Cycle</p>").unwrap();
        let error = sheet
            .resolve_document_root(tree.root_element(), &FileResolver::default())
            .unwrap_err();
        match error {
            StyleError::DependencyCycle {
                sources,
                dependencies,
            } => {
                assert!(
                    sources.len() >= 2,
                    "retain both page and root declaration sites: {sources:?}"
                );
                assert!(
                    sources.iter().all(|source| source.base == font_base()),
                    "retain declaring resource identity: {sources:?}"
                );
                assert!(
                    sources.iter().any(|source| source.line == 2),
                    "retain root declaration line: {sources:?}"
                );
                assert_eq!(
                    dependencies.first(),
                    dependencies.last(),
                    "close the cycle: {dependencies:?}"
                );
                assert!(
                    dependencies.iter().any(|name| name == "Root.font-size"),
                    "name the root dependency: {dependencies:?}"
                );
                assert!(
                    dependencies
                        .iter()
                        .any(|name| name.starts_with("Page.margin-")),
                    "name the page dependency: {dependencies:?}"
                );
            }
            error => panic!("wrong geometry-cycle error for {css}: {error}"),
        }
    }
}

#[test]
fn invalid_geometry_declarations_and_custom_property_cycles_follow_cascade_defaulting() {
    for css in [
        "@page{size:400pt 600pt;size:100furlong;margin:20pt}",
        "@page{size:100vw 600pt;size:400pt 600pt;margin:20pt}",
        "html{--size:var(--other);--other:var(--size)}@page{size:400pt 600pt;margin:var(--size,20pt)}",
        "html{--size:var(--other);--other:var(--size);font-size:var(--size)}@page{size:400pt 600pt;margin:20pt}",
        "html{--size:var(--other);--other:var(--size)}@page{size:var(--size,400pt 600pt);margin:20pt}",
    ] {
        let trace = common::render_trace(&format!(
            "<style>{css}body,p{{margin:0;font:8pt serif}}</style><p>Recovery</p>"
        ));
        common::assert_pt(trace.page_width, 400.0);
        common::assert_pt(trace.page_height, 600.0);
        common::assert_pt(trace.page_area.width, 360.0);
        common::assert_pt(trace.page_area.height, 560.0);
    }
}

#[test]
fn page_box_edges_auto_margins_and_constraints_define_the_initial_containing_block() {
    for (declarations, paper, origin, area) in [
        (
            "margin:20pt;padding:10pt;border:2pt solid",
            [400.0, 600.0],
            [32.0, 32.0],
            [336.0, 536.0],
        ),
        (
            "margin:10%;padding:5%;border:2pt solid",
            [400.0, 600.0],
            [62.0, 92.0],
            [276.0, 416.0],
        ),
        (
            "margin:10%;padding:10%;width:50%;height:50%",
            [400.0, 600.0],
            [80.0, 120.0],
            [200.0, 300.0],
        ),
        (
            "margin:10%;padding:10%;max-width:200pt;max-height:300pt",
            [400.0, 600.0],
            [80.0, 120.0],
            [200.0, 300.0],
        ),
        (
            "margin:10%;padding:10%;width:200pt;height:300pt;box-sizing:border-box",
            [400.0, 600.0],
            [80.0, 120.0],
            [120.0, 180.0],
        ),
        (
            "margin:10%;padding:10%;width:500pt;height:700pt",
            [660.0, 940.0],
            [80.0, 120.0],
            [500.0, 700.0],
        ),
        (
            "margin:auto;padding:10pt;border:2pt solid;width:200pt;height:300pt",
            [400.0, 600.0],
            [100.0, 150.0],
            [200.0, 300.0],
        ),
        (
            "margin:20pt;padding:10pt;border:2pt solid;width:200pt;height:300pt",
            [400.0, 600.0],
            [32.0, 32.0],
            [200.0, 300.0],
        ),
        (
            "margin:20pt;padding:10pt;border:2pt solid;width:200pt;height:300pt;box-sizing:border-box",
            [400.0, 600.0],
            [32.0, 32.0],
            [176.0, 276.0],
        ),
        (
            "margin:20pt;padding:10pt;border:2pt solid;max-width:200pt;min-height:700pt",
            [400.0, 764.0],
            [32.0, 32.0],
            [200.0, 700.0],
        ),
        (
            "margin:20pt;padding:10pt;border:2pt solid;min-width:250pt;max-width:200pt",
            [400.0, 600.0],
            [32.0, 32.0],
            [250.0, 536.0],
        ),
        (
            "margin:20pt;margin-left:auto;padding:10pt;border:2pt solid;width:200pt",
            [400.0, 600.0],
            [168.0, 32.0],
            [200.0, 536.0],
        ),
        (
            "margin:20pt;border-style:solid",
            [400.0, 600.0],
            [20.0 + crate::paint::DEFAULT_BORDER_WIDTH; 2],
            [
                360.0 - 2.0 * crate::paint::DEFAULT_BORDER_WIDTH,
                560.0 - 2.0 * crate::paint::DEFAULT_BORDER_WIDTH,
            ],
        ),
        (
            "margin:auto;padding:10pt;border:2pt solid;width:500pt;height:700pt",
            [524.0, 724.0],
            [12.0, 12.0],
            [500.0, 700.0],
        ),
        (
            "margin:-10pt;padding:10pt;border:2pt solid",
            [400.0, 600.0],
            [2.0, 2.0],
            [396.0, 596.0],
        ),
    ] {
        let html = format!(
            "<style>@page{{size:400pt 600pt;{declarations}}}body,p{{margin:0;font:8pt/12pt serif}}p{{width:100vw;height:12pt;background:#123456}}</style><p>Page box</p>"
        );
        let trace = crate::render_html(&html, &common::local_options())
            .unwrap_or_else(|error| panic!("page box {declarations}: {error}"))
            .layout
            .unwrap();
        for (actual, expected) in [
            (trace.page_width, paper[0]),
            (trace.page_height, paper[1]),
            (trace.page_area.x, origin[0]),
            (trace.page_area.y, origin[1]),
            (trace.page_area.width, area[0]),
            (trace.page_area.height, area[1]),
        ] {
            common::assert_pt(actual, expected);
        }
        let paint = trace.display_lists[0]
            .iter()
            .find_map(|item| match item {
                crate::DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == [0x12, 0x34, 0x56] =>
                {
                    Some(paint)
                }
                _ => None,
            })
            .expect("viewport-sized document box");
        common::assert_pt(paint.rect.x, origin[0]);
        common::assert_pt(paint.rect.y, origin[1]);
        common::assert_pt(paint.rect.width, area[0]);
    }
}

#[test]
fn page_border_metrics_follow_computed_styles_before_geometry_dependencies() {
    for (css, inset, width) in [
        ("@page{border:1vw none}", 20.0, 360.0),
        ("@page{border:1vh hidden}", 20.0, 360.0),
        (
            "html{border:3pt solid}@page{border-width:inherit;border-style:solid}",
            23.0,
            354.0,
        ),
        (
            "html{border:3pt none}@page{border-width:inherit;border-style:solid}",
            20.0,
            360.0,
        ),
    ] {
        let mut sheet = Stylesheet::parse(&[StylesheetSource {
            text: format!("@page{{size:400pt 600pt;margin:20pt}}{css}"),
            base: Default::default(),
        }])
        .unwrap();
        let tree = DocumentTree::parse("<p>Borders</p>").unwrap();
        sheet
            .resolve_document_root(tree.root_element(), &FileResolver::default())
            .unwrap_or_else(|error| panic!("page border {css}: {error}"));
        let area = sheet.page_content_box();
        common::assert_pt(area.x, inset);
        common::assert_pt(area.width, width);
    }
}

#[test]
fn suppressed_borders_do_not_resolve_font_metric_lengths() {
    for border_style in ["none", "hidden", "solid"] {
        let resolver = RecordingResolver {
            inner: FileResolver::default(),
            requests: Default::default(),
        };
        let css = format!(
            "@font-face{{font-family:Metric;src:url(MetricGlyphTest.ttf)}}#target{{font-family:Metric;border:1ch {border_style}}}"
        );
        let style = computed_with(&css, &resolver).unwrap();
        if border_style == "solid" {
            assert_eq!(
                resolver.requests.borrow().len(),
                1,
                "visible border must measure the selected face"
            );
            assert!(
                style
                    .decoration
                    .borders
                    .iter()
                    .all(|border| border.used_width() > 0.0)
            );
        } else {
            assert!(
                resolver.requests.borrow().is_empty(),
                "{border_style} border has zero computed width and must not fetch metrics: {:?}",
                resolver.requests.borrow()
            );
            assert!(
                style
                    .decoration
                    .borders
                    .iter()
                    .all(|border| border.used_width() == 0.0)
            );
        }
    }
    let resolver = RecordingResolver {
        inner: FileResolver::default(),
        requests: Default::default(),
    };
    let mut options = common::local_options();
    options.base = font_base();
    crate::render_with_resolver("<style>@font-face{font-family:Metric;src:url(MetricGlyphTest.ttf)}@page{size:400pt 600pt;margin:20pt;font-family:Metric;border:1ch none}body,p{font:8pt serif}</style><p>Body</p>",&options,&resolver).unwrap();
    assert!(
        resolver.requests.borrow().is_empty(),
        "page border must not fetch its unpainted metric font"
    );
}

#[test]
fn preferred_sheet_is_distinct_from_an_overconstrained_page_box() {
    let trace = common::render_trace(
        "<style>@page{size:400pt 600pt;margin:20pt;padding:10pt;border:2pt solid;width:200pt;height:300pt;@top-center{content:'M';font:8pt/10pt serif}}body,p{margin:0;font:8pt serif}</style><p>Sheet</p>",
    );
    common::assert_pt(trace.page_width, 400.0);
    common::assert_pt(trace.page_height, 600.0);
    common::assert_pt(trace.page_area.x, 32.0);
    common::assert_pt(trace.page_area.y, 32.0);
    common::assert_pt(trace.page_area.width, 200.0);
    common::assert_pt(trace.page_area.height, 300.0);
    let header = trace.pages[0]
        .iter()
        .find(|line| line.text == "M")
        .expect("page-box header");
    common::assert_pt(header.x + header.width / 2.0, 132.0);
}

#[test]
fn large_negative_page_margins_keep_the_resolved_content_size() {
    let mut sheet = Stylesheet::parse(&[StylesheetSource {
        text: "@page{size:400pt 600pt;width:100pt;height:100pt;margin:-1e20pt}html{width:100vw;height:100vh}".into(),
        base: Default::default(),
    }]).unwrap();
    let tree = DocumentTree::parse("<p>Finite content area</p>").unwrap();
    let root = sheet
        .resolve_document_root(tree.root_element(), &FileResolver::default())
        .expect("finite negative margins must not erase an independently resolved content size");
    let area = sheet.page_content_box();
    assert_eq!([area.width, area.height], [100.0, 100.0]);
    assert_eq!([area.x, area.y], [-1e20, -1e20]);
    common::assert_pt(points(&root.width), 100.0);
    common::assert_pt(points(&root.height), 100.0);
}

#[test]
fn overflowing_page_margin_extents_are_rejected_before_viewport_computation() {
    let mut sheet = Stylesheet::parse(&[StylesheetSource {
        text: "@page{size:400pt 600pt;width:100pt;margin-left:-3e38pt;margin-right:-3e38pt}".into(),
        base: Default::default(),
    }])
    .unwrap();
    let tree = DocumentTree::parse("<p>Finite geometry</p>").unwrap();
    let result = sheet.resolve_document_root(tree.root_element(), &FileResolver::default());
    assert!(
        matches!(result, Err(StyleError::Invalid(_))),
        "nonfinite derived extent must be a style error: {result:?}"
    );
}

#[test]
fn inverted_page_margin_extents_preserve_layout_on_an_empty_output_axis() {
    for (declarations, placement, area, trim) in [
        (
            "width:100pt;margin-left:-100pt;margin-right:-100pt",
            "left:100pt",
            [-100.0, 0.0, 100.0, 600.0],
            [0.0, 600.0],
        ),
        (
            "height:100pt;margin-top:-100pt;margin-bottom:-100pt",
            "top:100pt",
            [0.0, -100.0, 400.0, 100.0],
            [400.0, 0.0],
        ),
        (
            "width:100pt;height:100pt;margin:-100pt",
            "left:100pt;top:100pt",
            [-100.0, -100.0, 100.0, 100.0],
            [0.0, 0.0],
        ),
    ] {
        let html = format!(
            "<style>@page{{size:400pt 600pt;margin:0;padding:0;border:0;{declarations};background:navy;@top-left{{content:'H';font:8pt/10pt serif}}}}body,p{{margin:0;font:12pt/14pt serif}}p{{position:relative;{placement}}}</style><p>A</p><p style='break-before:page'>B</p>"
        );
        let output = crate::render_html(&html, &common::local_options())
            .unwrap_or_else(|error| panic!("inverted page-margin extent {declarations}: {error}"));
        let trace = output.layout.unwrap();
        assert_eq!([trace.page_width, trace.page_height], [400.0, 600.0]);
        assert_eq!(
            [
                trace.page_area.x,
                trace.page_area.y,
                trace.page_area.width,
                trace.page_area.height
            ],
            area,
            "{declarations}"
        );
        assert_eq!(
            [trace.page_output.trim.width, trace.page_output.trim.height],
            trim,
            "{declarations}"
        );
        assert_eq!(
            trace.pages.len(),
            2,
            "forced breaks remain active: {declarations}"
        );
        let lines = trace
            .pages
            .iter()
            .flatten()
            .filter(|line| matches!(line.text.as_str(), "A" | "B"))
            .collect::<Vec<_>>();
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"],
            "{declarations}"
        );
        for line in lines {
            common::assert_pt(line.x, 0.0);
        }
    }
}

#[test]
fn zero_authored_page_sizes_keep_css_geometry_on_a_positive_output_sheet() {
    const CONFIGURED_PAPER: [f32; 2] = [320.0, 480.0];
    for (size, area, paper) in [
        ("0", [0.0, 0.0], CONFIGURED_PAPER),
        ("0pt 300pt", [0.0, 300.0], [320.0, 300.0]),
        ("200pt 0px", [200.0, 0.0], [200.0, 480.0]),
        ("calc(2pt - 2pt)", [0.0, 0.0], CONFIGURED_PAPER),
    ] {
        let css = format!(
            "@page{{size:{size};margin:0;padding:0;border:0}}html{{width:100vw;height:100vh}}body,p{{margin:0;font:12pt/14pt serif}}"
        );
        let mut options = common::local_options();
        options.print.paper = CONFIGURED_PAPER;
        let output = crate::render_html(&format!("<style>{css}</style><p>A<br>B</p>"), &options)
            .unwrap_or_else(|error| panic!("zero authored page size {size}: {error}"));
        let trace = output.layout.unwrap();
        assert_eq!([trace.page_width, trace.page_height], paper, "{size}");
        assert_eq!(
            [trace.page_area.width, trace.page_area.height],
            area,
            "{size}"
        );
        assert_eq!(
            [trace.page_output.trim.width, trace.page_output.trim.height],
            area,
            "{size}"
        );
        assert_eq!(
            trace.pages.len(),
            if area[1] == 0.0 { 2 } else { 1 },
            "{size}"
        );
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"],
            "{size}"
        );

        let mut sheet = Stylesheet::parse_with_settings(
            &[StylesheetSource {
                text: css,
                base: Default::default(),
            }],
            &[],
            &Default::default(),
            &options.print,
        )
        .unwrap();
        let tree = DocumentTree::parse("<p>Viewport</p>").unwrap();
        let root = sheet
            .resolve_document_root(tree.root_element(), &FileResolver::default())
            .unwrap();
        common::assert_pt(points(&root.width), area[0]);
        common::assert_pt(points(&root.height), area[1]);
    }
}

#[test]
fn zero_page_area_is_a_valid_first_page_viewport() {
    let mut sheet = Stylesheet::parse(&[StylesheetSource {
        text:
            "@page{size:400pt 600pt;margin:20pt;width:0;height:0}html{font-size:10vw;width:1vmin}"
                .into(),
        base: Default::default(),
    }])
    .unwrap();
    let tree = DocumentTree::parse("<p>Zero viewport</p>").unwrap();
    let root = sheet
        .resolve_document_root(tree.root_element(), &FileResolver::default())
        .expect("zero dimensions are valid CSS sizes, not dependency cycles or invalid geometry");
    assert_eq!(sheet.page, [400.0, 600.0]);
    assert_eq!(
        sheet.page_content_box(),
        crate::Rect {
            x: 20.0,
            y: 20.0,
            width: 0.0,
            height: 0.0
        }
    );
    common::assert_pt(root.size, 0.0);
    common::assert_pt(points(&root.width), 0.0);
}

#[test]
fn zero_height_fragmentainers_preserve_content_and_forced_breaks() {
    for (body, pages, expected) in [
        ("<p>A<br>B</p>", 2, vec!["A", "B"]),
        (
            "<div style='height:3px;background:navy'></div><p>C</p>",
            4,
            vec!["C"],
        ),
        (
            "<p>A</p><p style='break-before:page'>B</p>",
            2,
            vec!["A", "B"],
        ),
        (
            "<div style='display:flex'><p>A</p><p>B</p></div>",
            1,
            vec!["A", "B"],
        ),
        (
            "<table><tr><td>A</td><td>B</td></tr></table>",
            1,
            vec!["A", "B"],
        ),
        (
            "<table><tr><td><div><p>A</p></div></td><td><span>B</span></td></tr></table>",
            1,
            vec!["A", "B"],
        ),
        (
            "<table><tr><td style='vertical-align:middle'><img style='display:block;width:8pt;height:8pt' src='../assets/border-image-nine.png'></td><td>B</td></tr></table>",
            1,
            vec!["B"],
        ),
        (
            "<p>A</p><table><tr><td>B</td><td>C</td></tr></table><p>D</p>",
            3,
            vec!["A", "B", "C", "D"],
        ),
        (
            "<div style='column-count:2'>A<br>B</div>",
            1,
            vec!["A", "B"],
        ),
    ] {
        let trace = common::render_trace(&format!(
            "<style>@page{{size:400pt 600pt;margin:20pt;height:0}}body,p{{margin:0;font:12pt/14pt serif}}table{{border-spacing:0}}td{{padding:0}}</style>{body}"
        ));
        assert_eq!(
            trace.pages.len(),
            pages,
            "degenerate fragmentainers must make finite progress: {body}"
        );
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            expected,
            "content is preserved: {body}"
        );
        common::assert_pt(trace.page_area.height, 0.0);
    }
}

#[test]
fn oversized_lines_overflow_without_being_rejected() {
    let trace = common::render_trace(
        "<style>@page{size:400pt 600pt;margin:20pt;height:5pt}body,p{margin:0;font:12pt/14pt serif}</style><p>A<br>B<br>C</p>",
    );
    assert_eq!(
        trace.pages.len(),
        3,
        "one indivisible line makes progress in each undersized fragmentainer"
    );
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
    for page in &trace.pages {
        common::assert_pt(page[0].baseline, trace.pages[0][0].baseline);
    }
}

#[test]
fn subpixel_page_areas_keep_their_viewport_while_fragmentation_progresses_by_one_pixel() {
    const PROGRESS_PT: f32 = crate::graphics::POINTS_PER_PIXEL;
    for height in [0.0, 0.25, 0.5] {
        let trace = common::render_trace(&format!(
            "<style>@page{{size:400pt 600pt;margin:20pt;height:{height}pt}}body,p{{margin:0;font:12pt/14pt serif}}div{{height:3px;background:navy}}</style><div></div><p>C</p>"
        ));
        common::assert_pt(trace.page_area.height, height);
        assert_eq!(
            trace.pages.len(),
            4,
            "three CSS pixels of spacing then one indivisible line, height {height}"
        );
        for page in &trace.display_lists[..3] {
            let spacer = page
                .iter()
                .find_map(|item| match item {
                    crate::DisplayItem::Box(paint)
                        if paint.decoration.background.rgb == [0, 0, 128] =>
                    {
                        Some(paint)
                    }
                    _ => None,
                })
                .expect("each progress fragment retains its part of the spacer");
            common::assert_pt(spacer.rect.height, PROGRESS_PT);
        }
    }
}
