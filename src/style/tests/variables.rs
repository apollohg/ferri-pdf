use crate::dom::{DocumentTree, Selector};
use crate::resources::FileResolver;
use crate::style::{LengthPercentage, Style, Stylesheet, StylesheetSource};
use std::fmt::Write;

fn computed(css: &str, html: &str) -> Vec<Style> {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap();
    let html = DocumentTree::parse(html).expect("parse test HTML");
    let mut styles = Vec::new();
    for element in html.select(&Selector::parse("#root, #child, #leaf").unwrap()) {
        let parent = styles.last().cloned().unwrap_or_default();
        styles.push(
            sheet
                .computed_with_resolver(element, &parent, &FileResolver::default())
                .unwrap(),
        );
    }
    styles
}
#[test]
fn aliases_resolve_before_inheritance_and_names_preserve_case() {
    let s = computed(
        "#root{--ink:#224466;--Ink:#aa3300;--alias:var(--ink)}#child{--ink:green;color:var(--alias)}#leaf{color:var(--Ink)}",
        "<section id=root><div id=child><p id=leaf>X</p></div></section>",
    );
    assert_eq!(s[1].color.rgb, [34, 68, 102]);
    assert_eq!(s[2].color.rgb, [170, 51, 0]);
}
#[test]
fn cycles_include_unused_fallback_edges_and_outer_fallback_survives() {
    for declarations in [
        "--a:var(--b);--b:var(--a)",
        "--a:var(--a,red)",
        "--present:blue;--a:var(--present,var(--b));--b:var(--a)",
    ] {
        let s = computed(
            &format!("#root{{{declarations};color:var(--a,#008800)}}"),
            "<p id=root>X</p>",
        );
        assert_eq!(s[0].color.rgb, [0, 136, 0], "cycle {declarations}");
    }
}
#[test]
fn substitutions_expand_shorthands_without_losing_longhand_priority() {
    let s = computed(
        "#root{--edges:2pt 4pt 6pt 8pt}#child{padding:var(--edges)!important;padding-left:30pt}#leaf{padding:var(--edges);padding-left:12pt}",
        "<section id=root><div id=child><p id=leaf>X</p></div></section>",
    );
    assert_eq!(
        s[1].padding,
        [2.0, 4.0, 6.0, 8.0].map(LengthPercentage::Length)
    );
    assert_eq!(s[2].padding[3], LengthPercentage::Length(12.0));
}
#[test]
fn missing_variables_reset_winners_without_reviving_older_declarations() {
    let s = computed(
        "#root{color:green;font-size:15pt}#child{color:red;color:var(--missing);font-size:24pt;font-size:var(--missing);margin-left:20pt;margin-left:var(--missing)}",
        "<section id=root><h2 id=child>X</h2></section>",
    );
    assert_eq!(s[1].color, s[0].color);
    assert_eq!(s[1].size, 15.0);
    assert!(s[1].margin[3].is_zero());
}

#[test]
fn invalid_substituted_values_and_token_boundaries_reset_properties() {
    let s = computed(
        "#root{color:green}#child{--bad:24pt;--n:12;color:red;color:var(--bad);margin-left:20pt;margin-left:var(--n)pt}",
        "<section id=root><p id=child>X</p></section>",
    );
    assert_eq!(s[1].color, s[0].color, "a dimension is not a color");
    assert!(
        s[1].margin[3].is_zero(),
        "substitution cannot concatenate a number and ident into a dimension"
    );
}

#[test]
fn invalid_shorthand_substitution_resets_every_component() {
    let s = computed(
        "#root{--edges:2pt red 6pt 8pt;padding:20pt;padding:var(--edges)}",
        "<p id=root>X</p>",
    );
    assert!(
        s[0].padding.iter().all(LengthPercentage::is_zero),
        "one invalid component invalidates the whole shorthand"
    );
}

#[test]
fn empty_values_css_wide_keywords_and_escaped_functions_remain_distinct() {
    let s = computed(
        r"#root{--ink:#008800;color:green;--empty:}#child{--ink:inherit;color:v\61 r(--ink)}#leaf{--ink:initial;color:var(--empty,red);background-color:var(--ink,#224466)}",
        "<section id=root><div id=child><p id=leaf>X</p></div></section>",
    );
    assert_eq!(s[1].color.rgb, [0, 136, 0]);
    assert_eq!(
        s[2].color, s[1].color,
        "an empty valid variable prevents fallback, then fails the color grammar"
    );
    assert_eq!(s[2].decoration.background.rgb, [34, 68, 102]);
}

#[test]
fn recursive_expansion_is_rejected_before_unbounded_allocation() {
    const EXPANSION_LEVELS: usize = 22;
    let mut css = String::from("#root{--v0:x;");
    for level in 1..EXPANSION_LEVELS {
        let _ = write!(
            css,
            "--v{level}:var(--v{}) var(--v{});",
            level - 1,
            level - 1
        );
    }
    css.push('}');
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css,
        base: Default::default(),
    }])
    .unwrap();
    let html = DocumentTree::parse("<p id=root>X</p>").expect("parse test HTML");
    let element = html
        .select(&Selector::parse("#root").unwrap())
        .next()
        .unwrap();
    assert!(
        matches!(
            sheet.computed_with_resolver(element, &Style::default(), &FileResolver::default()),
            Err(crate::style::StyleError::VariableLimit {
                resource: "expanded value bytes",
                ..
            })
        ),
        "exponential substitution must stop at its explicit byte bound"
    );
}

#[test]
fn overlapping_cycles_invalidate_all_members_independently_of_hash_order() {
    const CASCADE_REPETITIONS: usize = 64;
    for _ in 0..CASCADE_REPETITIONS {
        let s = computed(
            "#root{--a:var(--b) var(--c);--b:var(--a);--c:var(--b,red);color:var(--c,green)}",
            "<p id=root>X</p>",
        );
        assert_eq!(
            s[0].color.rgb,
            [0, 128, 0],
            "every member of the overlapping cycle must be invalid"
        );
    }
}

#[test]
fn empty_custom_properties_still_consume_the_environment_budget() {
    const PROPERTY_COUNT: usize = 20_000;
    let mut css = String::from("#root{");
    for index in 0..PROPERTY_COUNT {
        let _ = write!(css, "--empty{index}:;");
    }
    css.push('}');
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css,
        base: Default::default(),
    }])
    .unwrap();
    let html = DocumentTree::parse("<p id=root>X</p>").expect("parse test HTML");
    let selector = Selector::parse("#root").unwrap();
    let element = html.select(&selector).next().unwrap();
    assert!(
        matches!(
            sheet.computed_with_resolver(element, &Style::default(), &FileResolver::default()),
            Err(crate::style::StyleError::VariableLimit {
                resource: "environment entries",
                ..
            })
        ),
        "empty values must not bypass the variable map allocation bound"
    );
}

#[test]
fn invalid_substitutions_reset_display_content_and_page_size() {
    let mut sheet = Stylesheet::parse(&[StylesheetSource {
        text: "#root{--bad:red;--display:potato potato;display:block;display:var(--display)}#root::before{content:'Old';content:var(--bad)}@page{size:400pt 500pt;size:var(--bad)}".into(),
        base: Default::default(),
    }]).unwrap();
    let html = DocumentTree::parse("<p id=root>Body</p>").expect("parse test HTML");
    let selector = Selector::parse("#root").unwrap();
    let element = html.select(&selector).next().unwrap();
    let style = sheet
        .computed_with_resolver(element, &Style::default(), &FileResolver::default())
        .unwrap();
    assert_eq!(
        style.display,
        crate::style::Display::Inline,
        "invalid computed display resets to its initial value, not an earlier declaration"
    );
    assert!(
        sheet
            .pseudo_with_resolver(
                element,
                &style,
                crate::style::PseudoElement::Before,
                &FileResolver::default()
            )
            .unwrap()
            .is_none(),
        "an invalid computed content value suppresses the pseudo element"
    );
    sheet
        .resolve_page_geometry_with_resolver(&style, &FileResolver::default())
        .unwrap();
    assert_eq!(
        sheet.page,
        [595.2756, 841.8898],
        "invalid computed page size resolves to auto"
    );
}

#[test]
fn deeply_nested_values_are_rejected_during_stylesheet_parsing() {
    const NESTING_DEPTH: usize = 300;
    let source = format!(
        "#root{{--deep:{}green{}}}",
        "var(--missing,".repeat(NESTING_DEPTH),
        ")".repeat(NESTING_DEPTH)
    );
    let error = Stylesheet::parse(&[StylesheetSource {
        text: source,
        base: Default::default(),
    }])
    .err()
    .expect("nested source must be bounded before variable evaluation");
    assert!(
        matches!(
            error,
            crate::style::StyleError::GrammarLimit {
                resource: "component nesting",
                ..
            }
        ),
        "parse limit must retain its type: {error:?}"
    );
}

#[test]
fn substitution_preserves_display_values_and_white_space_longhands() {
    use crate::style::Display;
    // CSS Display 3 §2: `flow` alone is `block flow`, and `inline flex` is `inline-flex`.
    for (display, expected) in [
        ("run-in", Display::RunIn(Box::new(Display::Inline))),
        ("flow", Display::Block),
        ("inline flex", Display::InlineFlex),
        ("flow-root", Display::FlowRoot),
        ("block flow-root list-item", Display::FlowRootListItem),
        ("inline flow-root", Display::InlineBlock),
        ("list-item inline", Display::InlineListItem),
        (
            "inline flow-root list-item",
            Display::InlineFlowRootListItem,
        ),
        (
            "run-in flow-root list-item",
            Display::RunIn(Box::new(Display::InlineFlowRootListItem)),
        ),
        ("run-in ruby", Display::Unsupported("run-in ruby".into())),
    ] {
        let styles = computed(
            &format!("#root{{--display:{display};display:var(--display)}}"),
            "<p id=root>X</p>",
        );
        assert_eq!(
            styles[0].display, expected,
            "substituted display {display} must compute as declared"
        );
    }
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: "#root{--space:preserve-spaces;white-space:var(--space)}".into(),
        base: Default::default(),
    }])
    .unwrap();
    let html = DocumentTree::parse("<p id=root>X</p>").expect("parse test HTML");
    let selector = Selector::parse("#root").unwrap();
    let style = sheet
        .computed_with_resolver(
            html.select(&selector).next().unwrap(),
            &Style::default(),
            &FileResolver::default(),
        )
        .expect("a substituted white-space shorthand expands to its longhands");
    assert_eq!(
        style.white_space,
        crate::style::WhiteSpace {
            collapse: crate::style::WhiteSpaceCollapse::PreserveSpaces,
            wrap: true,
        },
        "CSS Text 4 §3: an omitted text-wrap-mode takes its initial value"
    );
}

#[test]
fn generated_function_argument_types_are_checked_after_substitution() {
    for value in ["attr(123)", "counter(123)", "counters(name,123)"] {
        let sheet = Stylesheet::parse(&[StylesheetSource {
            text: format!("#root::before{{--bad:{value};content:var(--bad)}}"),
            base: Default::default(),
        }])
        .unwrap();
        let html = DocumentTree::parse("<p id=root>Body</p>").expect("parse test HTML");
        let selector = Selector::parse("#root").unwrap();
        let element = html.select(&selector).next().unwrap();
        assert!(
            sheet
                .pseudo_with_resolver(
                    element,
                    &Style::default(),
                    crate::style::PseudoElement::Before,
                    &FileResolver::default()
                )
                .unwrap()
                .is_none(),
            "invalid function must reset generated content: {value}"
        );
    }
}

#[test]
fn color_functions_compute_after_substitution() {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: "#root{--ink:color-mix(in srgb, red, blue);color:var(--ink)}".into(),
        base: Default::default(),
    }])
    .unwrap();
    let html = DocumentTree::parse("<p id=root>Body</p>").expect("parse test HTML");
    let selector = Selector::parse("#root").unwrap();
    let style = sheet
        .computed_with_resolver(
            html.select(&selector).next().unwrap(),
            &Style::default(),
            &FileResolver::default(),
        )
        .expect("a substituted color-mix() computes");
    assert_eq!(
        style.color.rgb,
        [128, 0, 128],
        "an even sRGB mix of red and blue is rgb(127.5 0 127.5), rounded up"
    );
}
