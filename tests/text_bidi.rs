mod common;

fn source(css: &str, body: &str) -> String {
    format!(
        "<!doctype html><style>{}@page{{size:600pt 500pt;margin:20pt}}html,body,p{{margin:0;padding:0;font:20pt/24pt 'DejaVu Sans'}}{css}</style>{body}",
        common::PROFILE
    )
}

fn render(css: &str, body: &str) -> ferri_pdf::LayoutTrace {
    ferri_pdf::render_html(&source(css, body), &common::local_options())
        .unwrap_or_else(|error| panic!("bidi rendering {css:?} / {body:?}: {error}"))
        .layout
        .unwrap()
}

#[test]
fn legacy_visual_order_uses_the_effective_document_encoding() {
    for (label, expected) in [
        ("ISO-8859-8", "X אב Y"),
        ("visual", "X אב Y"),
        ("ISO-8859-8-I", "X בא Y"),
        ("logical", "X בא Y"),
    ] {
        let mut bytes = source("", &format!("<meta charset={label}><p>X ")).into_bytes();
        bytes.extend_from_slice(b"\xe0\xe1 Y</p>");
        let output = ferri_pdf::render_bytes(&bytes, &common::local_options()).unwrap();
        assert!(
            output.diagnostics.is_empty(),
            "valid UA rules must not produce diagnostics: {:?}",
            output.diagnostics
        );
        let trace = output.layout.unwrap();
        assert_eq!(
            trace.pages[0][0].text, "X אב Y",
            "the two Hebrew encoding variants decode identical characters: {label}"
        );
        assert_eq!(
            visual_text(&trace.pages[0][0]),
            expected,
            "the effective encoding controls visual-order UA rules: {label}"
        );
    }
    let unicode = source("", "<meta charset=ISO-8859-8><p>X אב Y</p>");
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend_from_slice(unicode.as_bytes());
    let trace = ferri_pdf::render_bytes(&bom, &common::local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        visual_text(&trace.pages[0][0]),
        "X בא Y",
        "a UTF-8 BOM wins over the visual Hebrew declaration"
    );
    assert_eq!(
        visual_text(&render("", "<meta charset=ISO-8859-8><p>X אב Y</p>").pages[0][0]),
        "X בא Y",
        "Unicode entry is already decoded and must ignore a legacy encoding declaration"
    );
}

#[test]
fn late_visual_encoding_and_shadow_text_share_the_document_rule() {
    const AFTER_PRESCAN: usize = 1100;
    let mut bytes = source(
        "",
        &format!(
            "<!--{}--><meta charset=visual><div><template shadowrootmode=open><p>X ",
            "x".repeat(AFTER_PRESCAN)
        ),
    )
    .into_bytes();
    bytes.extend_from_slice(b"\xe0\xe1 Y</p></template></div>");
    let trace = ferri_pdf::render_bytes(&bytes, &common::local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(trace.pages[0][0].text, "X אב Y");
    assert_eq!(
        visual_text(&trace.pages[0][0]),
        "X אב Y",
        "parser-triggered encoding restart must supply the final encoding to shadow UA rules"
    );
}

#[test]
fn authored_bidi_and_origin_rollback_override_visual_encoding_defaults() {
    for (author, user, expected) in [
        ("", "", "X אב Y"),
        ("p{unicode-bidi:normal}", "", "X בא Y"),
        ("p{unicode-bidi:revert}", "", "X אב Y"),
        ("", "p{unicode-bidi:normal}", "X בא Y"),
    ] {
        let html = source(author, "<meta charset=visual><p>X &#x5d0;&#x5d1; Y</p>");
        let mut options = common::local_options();
        options.user_stylesheets.push(user.into());
        let trace = ferri_pdf::render_bytes(html.as_bytes(), &options)
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            visual_text(&trace.pages[0][0]),
            expected,
            "visual Hebrew must remain a normal UA default: author={author:?}, user={user:?}"
        );
    }
}

fn visual_text(line: &ferri_pdf::TextLine) -> String {
    line.runs
        .iter()
        .flat_map(|run| {
            run.glyphs()
                .iter()
                .map(|glyph| &run.text[glyph.text_range.clone()])
        })
        .collect()
}

#[test]
fn css_bidi_scopes_follow_independent_unicode_character_records() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/bidi/character-reference.json")).unwrap();
    for case in reference["cases"].as_array().unwrap() {
        let css = match case["direction"].as_u64().unwrap() {
            0 => "p{direction:ltr}",
            1 => "p{direction:rtl}",
            2 => "p{unicode-bidi:plaintext}",
            _ => unreachable!(),
        };
        let trace = render(css, &format!("<p>{}</p>", case["body"].as_str().unwrap()));
        let line = &trace.pages[0][0];
        assert_eq!(
            line.text,
            case["logical"].as_str().unwrap(),
            "synthetic controls must not enter the source text: {case}"
        );
        assert_eq!(
            visual_text(line),
            case["visual"].as_str().unwrap(),
            "Unicode reference line {}",
            case["line"]
        );
    }
}

#[test]
fn html_direction_and_override_elements_use_the_common_bidi_path() {
    for (body, logical, visual) in [
        ("<p>X <bdo dir=rtl>ABC</bdo> Y</p>", "X ABC Y", "X CBA Y"),
        ("<p>X <span dir=auto>אב</span> Y</p>", "X אב Y", "X בא Y"),
        ("<p dir=RTL>ABC</p>", "ABC", "ABC"),
        ("<p><span dir=invalid>ABC</span></p>", "ABC", "ABC"),
    ] {
        let trace = render("", body);
        assert_eq!(trace.pages[0][0].text, logical);
        assert_eq!(visual_text(&trace.pages[0][0]), visual, "{body}");
    }
}

#[test]
fn bidi_overrides_reopen_after_forced_paragraph_breaks() {
    for body in [
        "<p style='direction:rtl;unicode-bidi:bidi-override'>abc<br>DEF</p>",
        "<p><span style='direction:rtl;unicode-bidi:isolate-override'>abc<br>DEF</span></p>",
    ] {
        let trace = render("", body);
        let visual = trace.pages[0].iter().map(visual_text).collect::<Vec<_>>();
        assert_eq!(visual, ["cba", "FED"], "{body}");
        assert_eq!(
            trace.pages[0]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["abc", "DEF"]
        );
    }
}

#[test]
fn bidi_wide_keywords_and_support_queries_keep_normal_and_override_distinct() {
    for value in [
        "normal",
        "embed",
        "isolate",
        "bidi-override",
        "isolate-override",
        "plaintext",
    ] {
        let trace = render(
            &format!("p{{font-size:9pt}}@supports(unicode-bidi:{value}){{p{{font-size:18pt}}}}"),
            "<p>ABC</p>",
        );
        common::assert_pt(trace.pages[0][0].font_size, 18.0);
    }
    for (declarations, expected) in [
        ("unicode-bidi:initial", "ABC"),
        ("unicode-bidi:unset", "ABC"),
        ("all:initial", "CBA"),
        ("unicode-bidi:normal", "ABC"),
        ("unicode-bidi:embed", "ABC"),
        ("unicode-bidi:isolate", "ABC"),
        ("unicode-bidi:plaintext", "ABC"),
        ("unicode-bidi:isolate-override", "CBA"),
    ] {
        let trace = render(
            &format!("span{{direction:rtl;unicode-bidi:bidi-override;{declarations}}}"),
            "<p><span>ABC</span></p>",
        );
        assert_eq!(visual_text(&trace.pages[0][0]), expected, "{declarations}");
    }
    for (declarations, expected) in [
        ("unicode-bidi:inherit", "a(bfאה)ו"),
        ("", "a(bאfה)ו"),
        ("unicode-bidi:unset", "a(bאfה)ו"),
    ] {
        let trace = render(
            &format!("p{{unicode-bidi:plaintext}}span{{{declarations}}}"),
            "<p>a(b<span>אf</span>ה)ו</p>",
        );
        assert_eq!(
            visual_text(&trace.pages[0][0]),
            expected,
            "non-inherited plaintext / {declarations}"
        );
    }
}

#[test]
fn source_bidi_controls_remain_effective_inside_css_scopes() {
    for (text, visual) in [("a\u{202c}BC", "aBC"), ("a\u{202a}b\u{202c}c", "cba")] {
        let trace = render("", &format!("<p><bdo dir=rtl>{text}</bdo></p>"));
        assert_eq!(
            trace.pages[0][0].text, text,
            "original controls must remain source characters"
        );
        assert_eq!(
            visual_text(&trace.pages[0][0]),
            visual,
            "source controls must participate in the same UBA stream as CSS scopes"
        );
    }
}

#[test]
fn inline_bidi_scopes_split_around_blocks_without_overriding_descendant_blocks() {
    let trace = render(
        "",
        "<div><span style='direction:rtl;unicode-bidi:isolate-override'>ab<div>CD</div>ef</span></div>",
    );
    assert_eq!(
        trace.pages[0].iter().map(visual_text).collect::<Vec<_>>(),
        ["ba", "CD", "fe"]
    );
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["ab", "CD", "ef"]
    );
}
