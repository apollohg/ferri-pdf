//! CSS Properties and Values API 1: `@property` registrations and the computed values,
//! inheritance and initial values they give custom properties.
use crate::dom::{DocumentTree, Selector};
use crate::resources::{FileResolver, ResourceBase};
use crate::style::{LengthPercentage, POINTS_PER_PIXEL, Size, Style, Stylesheet, StylesheetSource};

const TREE: &str = "<section id=root data-image=a.png data-size=4pt><div id=child><p id=leaf>X</p></div></section>";

/// The computed styles of `#root`, `#child` and `#leaf`, and the stylesheet's diagnostics.
fn computed_in(css: &str, base: ResourceBase) -> (Vec<Style>, Vec<String>) {
    let mut sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base,
    }])
    .unwrap_or_else(|error| panic!("parse `{css}`: {error}"));
    let html = DocumentTree::parse(TREE);
    let mut styles = Vec::new();
    for element in html.select(&Selector::parse("#root, #child, #leaf").unwrap()) {
        let parent = styles.last().cloned().unwrap_or_default();
        styles.push(
            sheet
                .computed_with_resolver(element, &parent, &FileResolver::default())
                .unwrap_or_else(|error| panic!("compute `{css}`: {error}")),
        );
    }
    let diagnostics = sheet
        .take_diagnostics()
        .iter()
        .map(ToString::to_string)
        .collect();
    (styles, diagnostics)
}

fn computed(css: &str) -> Vec<Style> {
    computed_in(css, ResourceBase::default()).0
}

fn custom(style: &Style, name: &str) -> Option<String> {
    style
        .custom_properties
        .get(name)
        .cloned()
        .flatten()
        .map(|value| value.as_str().to_owned())
}

fn width(style: &Style) -> f32 {
    match &style.width {
        Size::Value(LengthPercentage::Length(points)) => *points,
        other => panic!("expected an absolute width, got {other:?}"),
    }
}

fn assert_close(actual: f32, expected: f32, context: &str) {
    assert!(
        (actual - expected).abs() < 1e-3,
        "{context}: expected {expected}, got {actual}"
    );
}

#[test]
fn registered_lengths_compute_to_pixels_and_inherit_absolute() {
    let s = computed(
        "@property --gap { syntax: '<length>'; inherits: true; initial-value: 1in }\
         #root { font-size: 10pt; --gap: 2em } #child { font-size: 20pt; width: var(--gap) }\
         #leaf { width: var(--gap); --gap: 3em }",
    );
    let root = 20.0 / POINTS_PER_PIXEL;
    assert_eq!(
        custom(&s[0], "--gap").as_deref(),
        Some(format!("{root}px").as_str()),
        "2em at a 10pt font computes to canonical pixels on the declaring element"
    );
    assert_eq!(
        custom(&s[1], "--gap"),
        custom(&s[0], "--gap"),
        "the absolute value inherits, not the 2em token"
    );
    assert_close(
        width(&s[1]),
        20.0,
        "the child's width substitutes the root's 2em",
    );
    assert_close(
        width(&s[2]),
        60.0,
        "the leaf's own 3em resolves against its 20pt font",
    );
}

#[test]
fn unregistered_properties_keep_tokens_for_contrast() {
    let s = computed(
        "#root { font-size: 10pt; --gap: 2em } #child { font-size: 20pt; width: var(--gap) }",
    );
    assert_eq!(custom(&s[1], "--gap").as_deref(), Some("2em"));
    assert_close(
        width(&s[1]),
        40.0,
        "unregistered em resolves on the using element",
    );
}

#[test]
fn initial_values_apply_without_declarations_and_stop_inheritance_when_registered() {
    let s = computed(
        "@property --ink { syntax: '<color>'; inherits: false; initial-value: rgb(0 128 0) }\
         @property --size { syntax: '<length>'; inherits: true; initial-value: 12pt }\
         #root { --ink: rgb(255 0 0); color: var(--ink) }\
         #child { color: var(--ink); width: var(--size) }\
         #leaf { --ink: inherit; color: var(--ink) }",
    );
    assert_eq!(s[0].color.rgb, [255, 0, 0], "the root's own declaration");
    assert_eq!(
        s[1].color.rgb,
        [0, 128, 0],
        "a non-inheriting registration takes its initial value where it is not declared"
    );
    assert_eq!(
        s[2].color.rgb,
        [0, 128, 0],
        "inherit takes the parent's (initial) value explicitly"
    );
    assert_close(
        width(&s[1]),
        12.0,
        "an undeclared inheriting registration has its initial value",
    );
    assert_eq!(custom(&s[1], "--size").as_deref(), Some("16px"));
}

#[test]
fn css_wide_keywords_and_invalid_values_follow_the_registration() {
    let s = computed(
        "@property --ink { syntax: '<color>'; inherits: false; initial-value: rgb(0 0 255) }\
         @property --mark { syntax: '<color>'; inherits: true; initial-value: rgb(0 0 255) }\
         #root { --ink: rgb(255 0 0); --mark: rgb(255 0 0) }\
         #child { --ink: inherit; --mark: url(not-a-color.png); color: var(--mark); background-color: var(--ink) }\
         #leaf { --ink: unset; --mark: initial; color: var(--ink); background-color: var(--mark) }",
    );
    assert_eq!(
        s[1].color.rgb,
        [255, 0, 0],
        "a value invalid at computed-value time is unset, which inherits for an inheriting registration"
    );
    assert_eq!(
        custom(&s[1], "--ink"),
        custom(&s[0], "--ink"),
        "inherit copies the parent's value of a non-inheriting registration"
    );
    assert_eq!(
        s[2].color.rgb,
        [0, 0, 255],
        "unset gives a non-inheriting registration its initial value"
    );
    assert_eq!(
        custom(&s[2], "--mark").as_deref(),
        Some("rgb(0, 0, 255)"),
        "initial is the registered initial value, not the guaranteed-invalid value"
    );
}

#[test]
fn invalid_at_computed_value_time_without_inheritance_takes_the_initial_value() {
    let s = computed(
        "@property --n { syntax: '<number>'; inherits: false; initial-value: 7 }\
         #root { --n: 3 } #child { --n: 3px; order: var(--n) } #leaf { --n: calc(2 * 4); order: var(--n) }",
    );
    assert_eq!(custom(&s[1], "--n").as_deref(), Some("7"));
    assert_eq!(
        custom(&s[2], "--n").as_deref(),
        Some("8"),
        "math functions compute"
    );
}

#[test]
fn invalid_rules_leave_properties_unregistered() {
    for (rule, why) in [
        (
            "@property --gap { inherits: true; initial-value: 1px }",
            "syntax is required",
        ),
        (
            "@property --gap { syntax: '<length>'; initial-value: 1px }",
            "inherits is required",
        ),
        (
            "@property --gap { syntax: '<length>'; inherits: true }",
            "a typed syntax needs an initial value",
        ),
        (
            "@property --gap { syntax: '<length>'; inherits: true; initial-value: 1em }",
            "initial values must be computationally independent",
        ),
        (
            "@property --gap { syntax: '<length>'; inherits: true; initial-value: 2vw }",
            "viewport units depend on the page",
        ),
        (
            "@property --gap { syntax: '<length>'; inherits: true; initial-value: red }",
            "initial values must match the syntax",
        ),
        (
            "@property --gap { syntax: <length>; inherits: true; initial-value: 1px }",
            "syntax is a string",
        ),
        (
            "@property --gap { syntax: '<length> +'; inherits: true; initial-value: 1px }",
            "multipliers follow their name directly",
        ),
        (
            "@property --gap { syntax: 'default'; inherits: true; initial-value: default }",
            "idents must be custom identifiers",
        ),
        (
            "@property --gap { syntax: '<length>'; inherits: maybe; initial-value: 1px }",
            "inherits is true or false",
        ),
        (
            "@property gap { syntax: '<length>'; inherits: true; initial-value: 1px }",
            "the name is a custom property name",
        ),
    ] {
        let (s, diagnostics) = computed_in(
            &format!(
                "{rule} #root {{ font-size: 10pt; --gap: 2em }} #child {{ font-size: 20pt; width: var(--gap) }}"
            ),
            ResourceBase::default(),
        );
        assert_eq!(
            custom(&s[1], "--gap").as_deref(),
            Some("2em"),
            "`{rule}` must not register --gap ({why})"
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("property")),
            "`{rule}` must be reported: {diagnostics:#?}"
        );
    }
}

#[test]
fn the_last_valid_rule_in_document_order_registers_and_unknown_descriptors_are_ignored() {
    let (s, diagnostics) = computed_in(
        "@property --gap { syntax: '<color>'; inherits: true; initial-value: red }\
         @property --gap { syntax: '<length>'; inherits: false; initial-value: 5px; colour: blue }\
         @property --gap { syntax: '<number>'; initial-value: 1 }\
         #root { --gap: 2pt } #child { width: var(--gap) }",
        ResourceBase::default(),
    );
    assert_close(
        width(&s[1]),
        3.75,
        "the second rule registers a non-inheriting length",
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("colour")),
        "the unknown descriptor is reported without invalidating its rule: {diagnostics:#?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("InvalidRule")),
        "the third, invalid rule is reported: {diagnostics:#?}"
    );
}

#[test]
fn universal_syntax_keeps_tokens_with_registered_inheritance_and_initial_value() {
    let s = computed(
        "@property --any { syntax: '*'; inherits: false; initial-value: 2em  3em }\
         @property --none { syntax: '*'; inherits: true }\
         #root { --any: 1px; --none: 4px } #child { font-size: 20pt; margin-left: var(--any) } #leaf { margin-left: var(--none) }",
    );
    assert_eq!(custom(&s[1], "--any").as_deref(), Some("2em  3em"));
    assert_eq!(custom(&s[2], "--none").as_deref(), Some("4px"));
}

#[test]
fn font_relative_cycles_invalidate_the_property_and_font_size() {
    for (css, order) in [
        (
            "#child { --f: 2em; font-size: var(--f); width: var(--f) }",
            "font-size first",
        ),
        (
            "#child { width: var(--f); --f: 2em; font-size: var(--f, 9pt) }",
            "custom property first",
        ),
    ] {
        let s = computed(&format!(
            "@property --f {{ syntax: '<length>'; inherits: false; initial-value: 3px }}\
             #root {{ font-size: 10pt }} {css}"
        ));
        assert_close(
            s[1].size,
            10.0,
            &format!("{order}: font-size behaves as unset"),
        );
        assert_eq!(
            custom(&s[1], "--f").as_deref(),
            Some("3px"),
            "{order}: the custom property is invalid at computed-value time"
        );
        assert_close(
            width(&s[1]),
            2.25,
            &format!("{order}: width takes the initial value"),
        );
    }
    for (css, size) in [
        ("#child { --f: 20px; font-size: var(--f) }", 15.0),
        (
            "#child { --f: 2em; line-height: var(--f); font-size: 15pt }",
            15.0,
        ),
    ] {
        let s = computed(&format!(
            "@property --f {{ syntax: '<length>'; inherits: false; initial-value: 3px }}\
             #root {{ font-size: 10pt }} {css}"
        ));
        assert_close(s[1].size, size, &format!("`{css}` has no cycle"));
    }
    let s = computed(
        "@property --l { syntax: '<length>'; inherits: false; initial-value: 3px }\
         #root { font-size: 10pt } #child { --l: 2lh; line-height: var(--l); font-size: 20pt; width: var(--l) }",
    );
    assert_close(s[1].size, 20.0, "lh adds no cycle through font-size");
    assert_close(width(&s[1]), 2.25, "lh closes a cycle with line-height");
}

#[test]
fn fallbacks_must_match_the_registered_syntax_even_when_unused() {
    let s = computed(
        "@property --w { syntax: '<length>'; inherits: true; initial-value: 4pt }\
         #root { --w: 5pt; width: var(--w, red) } #child { width: var(--w, 1pt) }\
         #leaf { width: var(--unset-name, var(--w, 2pt)) }",
    );
    assert_eq!(
        s[0].width,
        Size::Auto,
        "a fallback outside the syntax invalidates the value"
    );
    assert_close(width(&s[1]), 5.0, "a matching unused fallback is accepted");
    assert_close(
        width(&s[2]),
        5.0,
        "nested references check their own fallbacks",
    );
}

#[test]
fn typed_values_compute_lists_integers_colors_and_mixed_percentages() {
    let s = computed(
        "@property --list { syntax: '<length>#'; inherits: true; initial-value: 0px }\
         @property --row { syntax: '<length>+ | auto'; inherits: true; initial-value: auto }\
         @property --i { syntax: '<integer>'; inherits: true; initial-value: 0 }\
         @property --ink { syntax: '<color>'; inherits: true; initial-value: black }\
         @property --mixed { syntax: '<length-percentage>'; inherits: true; initial-value: 0px }\
         @property --turn { syntax: '<angle>'; inherits: true; initial-value: 0deg }\
         #root { font-size: 9pt; --list: 1em, 3pt; --row: 1em 2em; --i: calc(5 / 2);\
                 --ink: currentcolor; --mixed: calc(1em + 10%); --turn: 0.5turn; color: rgb(255 0 0) }\
         #child { color: rgb(0 0 255); background-color: var(--ink); width: var(--mixed); font-size: 30pt }",
    );
    assert_eq!(custom(&s[0], "--list").as_deref(), Some("12px, 4px"));
    assert_eq!(custom(&s[0], "--row").as_deref(), Some("12px 24px"));
    assert_eq!(
        custom(&s[0], "--i").as_deref(),
        Some("3"),
        "integers round half up"
    );
    assert_eq!(custom(&s[0], "--ink").as_deref(), Some("currentcolor"));
    assert_eq!(custom(&s[0], "--turn").as_deref(), Some("180deg"));
    assert_eq!(
        s[1].decoration.background.rgb,
        [0, 0, 255],
        "currentcolor stays a keyword and resolves where it is used"
    );
    match &s[1].width {
        Size::Value(LengthPercentage::Calc(calculation)) => {
            assert_close(
                calculation.resolve(0.0),
                9.0,
                "the length term is the root's 1em",
            );
            assert_close(
                calculation.resolve(200.0),
                29.0,
                "the percentage term keeps its basis",
            );
        }
        other => panic!("expected a calculation, got {other:?}"),
    }
}

#[test]
fn syntax_identifiers_match_codepoint_wise() {
    let s = computed(
        "@property --size { syntax: 'big | BIG'; inherits: false; initial-value: big }\
         #root { --size: BIG } #child { --size: Big }",
    );
    assert_eq!(custom(&s[0], "--size").as_deref(), Some("BIG"));
    assert_eq!(
        custom(&s[1], "--size").as_deref(),
        Some("big"),
        "Big matches neither"
    );
}

#[test]
fn registered_urls_resolve_against_the_declaring_stylesheet() {
    let base = ResourceBase::Url(
        crate::resources::Url::parse("https://example.test/styles/site.css").unwrap(),
    );
    let (s, _) = computed_in(
        "@property --image { syntax: '<url>'; inherits: true; initial-value: url(blank.png) }\
         #root { --image: url(../images/a.png) }",
        base,
    );
    assert_eq!(
        custom(&s[0], "--image").as_deref(),
        Some("url(\"https://example.test/images/a.png\")")
    );
    assert_eq!(
        custom(&Style::default(), "--image"),
        None,
        "registrations belong to a document"
    );
}

#[test]
fn style_queries_compare_registered_computed_values() {
    let s = computed(
        "@property --gap { syntax: '<length>'; inherits: true; initial-value: 0px }\
         #root { font-size: 10pt; --gap: 2em }\
         #child { font-size: 20pt; width: if(style(--gap: 20pt): 7pt; else: 9pt) }\
         #leaf { --gap: initial; width: if(style(--gap): 7pt; else: 9pt); height: if(style(--gap: initial): 7pt; else: 9pt) }",
    );
    assert_close(
        width(&s[1]),
        7.0,
        "the query value computes like the registered property",
    );
    assert_close(
        width(&s[2]),
        9.0,
        "the initial value does not hold in a boolean query",
    );
    match &s[2].height {
        Size::Value(LengthPercentage::Length(points)) => {
            assert_close(
                *points,
                7.0,
                "initial in a query is the registered initial value",
            );
        }
        other => panic!("expected an absolute height, got {other:?}"),
    }
}

#[test]
fn attribute_values_compute_but_cannot_supply_registered_urls() {
    let s = computed(
        "@property --picture { syntax: '<image> | none'; inherits: true; initial-value: none }\
         @property --size { syntax: '<length>'; inherits: true; initial-value: 0px }\
         #root { --picture: image-set(attr(data-image) 1x); --size: attr(data-size <length>) }\
         #child { --picture: image-set('b.png' 1x) }",
    );
    assert_eq!(
        custom(&s[0], "--picture").as_deref(),
        Some("none"),
        "an attribute-tainted image-set() URL is invalid at computed-value time"
    );
    assert_eq!(custom(&s[0], "--size").as_deref(), Some("5.3333335px"));
    assert!(
        custom(&s[1], "--picture").is_some_and(|value| value.contains("b.png")),
        "an untainted image computes: {:?}",
        custom(&s[1], "--picture")
    );
}
