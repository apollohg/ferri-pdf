//! Cascade of the CSS Text 4 wrapping properties and the CSS Overflow 4 clamping shorthands.
use crate::dom::{DocumentTree, Selector};
use crate::resources::FileResolver;
use crate::style::{
    BlockEllipsis, Continue, Display, HangingEnd, HangingPunctuation, HyphenateLimitChars,
    LineBreak, LineClamp, Style, Stylesheet, StylesheetSource, TextJustify, TextWrapStyle,
    WhiteSpace, WhiteSpaceCollapse, WordBreak,
};

/// Computed styles of `#root` and its `#child`.
fn computed(css: &str) -> [Style; 2] {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap();
    let html = DocumentTree::parse("<div id=root><p id=child>X</p></div>");
    let mut styles: Vec<Style> = Vec::new();
    for element in html.select(&Selector::parse("#root, #child").unwrap()) {
        let parent = styles.last().cloned().unwrap_or_default();
        styles.push(
            sheet
                .computed_with_resolver(element, &parent, &FileResolver::default())
                .unwrap_or_else(|error| panic!("{css}: {error}")),
        );
    }
    styles.try_into().expect("two elements")
}

#[test]
fn white_space_and_text_wrap_set_their_longhands() {
    let space = |collapse, wrap| WhiteSpace { collapse, wrap };
    for (css, expected, style) in [
        (
            "white-space:pre-line",
            space(WhiteSpaceCollapse::PreserveBreaks, true),
            TextWrapStyle::Auto,
        ),
        (
            "white-space:preserve-spaces nowrap",
            space(WhiteSpaceCollapse::PreserveSpaces, false),
            TextWrapStyle::Auto,
        ),
        (
            "white-space:nowrap",
            space(WhiteSpaceCollapse::Collapse, false),
            TextWrapStyle::Auto,
        ),
        (
            "white-space:break-spaces",
            space(WhiteSpaceCollapse::BreakSpaces, true),
            TextWrapStyle::Auto,
        ),
        (
            "white-space:pre;text-wrap-mode:wrap",
            WhiteSpace::PRE_WRAP,
            TextWrapStyle::Auto,
        ),
        (
            "text-wrap:balance nowrap",
            WhiteSpace::NOWRAP,
            TextWrapStyle::Balance,
        ),
        (
            "white-space:nowrap;text-wrap:pretty",
            WhiteSpace::NORMAL,
            TextWrapStyle::Pretty,
        ),
        (
            "text-wrap-style:stable;white-space:pre",
            WhiteSpace::PRE,
            TextWrapStyle::Stable,
        ),
    ] {
        let [root, child] = computed(&format!("#root{{{css}}}"));
        assert_eq!(root.white_space, expected, "{css}");
        assert_eq!(
            root.line_breaking.wrap_style, style,
            "{css}: text-wrap-style"
        );
        assert_eq!(child.white_space, expected, "{css}: inherited");
    }
}

#[test]
fn line_breaking_controls_compute_and_inherit() {
    let [root, child] = computed(
        "#root{word-break:keep-all;line-break:loose;text-justify:distribute;\
         hanging-punctuation:allow-end first;hyphenate-character:'=';\
         hyphenate-limit-chars:6 3}",
    );
    for (style, label) in [(&root, "root"), (&child, "inherited")] {
        let breaking = &style.line_breaking;
        assert_eq!(breaking.word_break, WordBreak::KeepAll, "{label}");
        assert_eq!(breaking.line_break, LineBreak::Loose, "{label}");
        assert_eq!(
            breaking.justify,
            TextJustify::InterCharacter,
            "{label}: distribute computes to inter-character"
        );
        assert_eq!(
            breaking.hanging_punctuation,
            HangingPunctuation {
                first: true,
                end: Some(HangingEnd::Allow),
                last: false,
            },
            "{label}"
        );
        assert_eq!(
            breaking.hyphenate_character.as_deref(),
            Some("="),
            "{label}"
        );
        assert_eq!(
            breaking.hyphenate_limit_chars,
            HyphenateLimitChars {
                word: Some(6),
                before: Some(3),
                after: Some(3),
            },
            "{label}: an omitted after-minimum equals the before-minimum"
        );
    }
    let [root, _] = computed("#root{hyphenate-limit-chars:auto 2;word-break:break-word}");
    assert_eq!(
        root.line_breaking.hyphenate_limit_chars,
        HyphenateLimitChars {
            word: None,
            before: Some(2),
            after: Some(2),
        }
    );
    assert_eq!(root.line_breaking.word_break, WordBreak::BreakWord);
    let [root, _] = computed("#root{hyphenate-limit-chars:4 auto 1}");
    assert_eq!(
        root.line_breaking.hyphenate_limit_chars,
        HyphenateLimitChars {
            word: Some(4),
            before: None,
            after: Some(1),
        }
    );
}

#[test]
fn line_clamp_shorthands_set_max_lines_continue_and_block_ellipsis() {
    let clamp = |max_lines, block_ellipsis, continuation| LineClamp {
        max_lines,
        block_ellipsis,
        continuation,
    };
    for (css, expected) in [
        (
            "line-clamp:3",
            clamp(Some(3), BlockEllipsis::Auto, Continue::Discard),
        ),
        (
            "line-clamp:2 '(more)'",
            clamp(
                Some(2),
                BlockEllipsis::String("(more)".into()),
                Continue::Discard,
            ),
        ),
        (
            "line-clamp:4 none",
            clamp(Some(4), BlockEllipsis::None, Continue::Discard),
        ),
        (
            "line-clamp:3;line-clamp:none",
            clamp(None, BlockEllipsis::None, Continue::Auto),
        ),
        (
            "-webkit-line-clamp:2",
            clamp(Some(2), BlockEllipsis::Auto, Continue::WebkitDiscard),
        ),
        // CSS Overflow 4 §5.1.1: the legacy shorthand always sets the ellipsis.
        (
            "-webkit-line-clamp:none",
            clamp(None, BlockEllipsis::Auto, Continue::Auto),
        ),
        (
            "max-lines:5;continue:discard",
            clamp(Some(5), BlockEllipsis::None, Continue::Discard),
        ),
        ("line-clamp:0", LineClamp::default()),
    ] {
        let [root, child] = computed(&format!("#root{{{css}}}"));
        assert_eq!(root.clamp, expected, "{css}");
        assert_eq!(
            child.clamp,
            LineClamp {
                block_ellipsis: expected.block_ellipsis.clone(),
                ..LineClamp::default()
            },
            "{css}: only block-ellipsis inherits"
        );
    }
}

#[test]
fn legacy_box_keywords_map_onto_flex_layout() {
    use crate::style::flex_values::FlexDirection;
    let [root, child] = computed("#root{display:-webkit-box;-webkit-box-orient:vertical}");
    assert_eq!(root.display, Display::Flex);
    assert!(root.flex.legacy_box, "-webkit-box computes as itself");
    assert_eq!(root.flex.direction, FlexDirection::Column);
    assert!(!child.flex.legacy_box);
    let [root, _] = computed("#root{display:-webkit-inline-box;-webkit-box-orient:inline-axis}");
    assert_eq!(root.display, Display::InlineFlex);
    assert!(root.flex.legacy_box);
    assert_eq!(root.flex.direction, FlexDirection::Row);
    let [root, _] = computed("#root{display:-webkit-box;display:flex}");
    assert!(
        !root.flex.legacy_box,
        "a later display value replaces the legacy box"
    );
}

/// CSS Text 4 §8.6: `text-spacing` sets both character class spacing longhands, an omitted
/// one to its initial value, and they inherit.
#[test]
fn text_spacing_shorthand_sets_trim_and_autospace() {
    for (shorthand, trim, autospace) in [
        ("none", "space-all", "no-autospace"),
        ("auto", "auto", "auto"),
        ("normal", "normal", "normal"),
        ("trim-start", "trim-start", "normal"),
        ("ideograph-alpha", "normal", "ideograph-alpha"),
        (
            "ideograph-numeric replace trim-both",
            "trim-both",
            "ideograph-numeric replace",
        ),
    ] {
        let [root, child] = computed(&format!(
            "#root{{text-spacing-trim:trim-all;text-autospace:punctuation;text-spacing:{shorthand}}}"
        ));
        let [expected, _] = computed(&format!(
            "#root{{text-spacing-trim:{trim};text-autospace:{autospace}}}"
        ));
        assert_eq!(
            root.text_spacing, expected.text_spacing,
            "text-spacing:{shorthand} sets text-spacing-trim:{trim} and text-autospace:{autospace}"
        );
        assert_eq!(
            child.text_spacing, root.text_spacing,
            "text-spacing:{shorthand} inherits"
        );
    }
}
