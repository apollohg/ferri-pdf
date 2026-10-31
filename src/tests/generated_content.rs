use super::common;
use crate::render_html;
use common::{PROFILE, local_options};

#[test]
fn pseudo_elements_inherit_from_origin_and_keep_independent_cascade() {
    let html = format!(
        r#"<style>{PROFILE}
      p::before {{content:"[";color:blue}}
      p::after {{content:"]"}}
      .item::before {{content:attr(data-prefix) " ";font-size:18pt}}
      #target:before {{content:"Winner " !important}}
      p {{content:normal}}
      span::before {{content:"inner "}}
      .hidden::before {{content:"hidden";display:none}}
      </style><p id="target" class="item" data-prefix="Attribute">Body <span>word</span></p><p class="item" data-prefix="Attribute">Next</p><p class="hidden">Last</p>"#
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let text: Vec<_> = trace.pages[0]
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        text,
        ["Winner Body inner word]", "Attribute Next]", "Last]"]
    );
    let first = &trace.pages[0][0];
    assert_eq!(
        first.runs[0].font_size, 18.0,
        "pseudo styles inherit and cascade without changing the origin"
    );
    assert_eq!(first.runs[0].color.rgb, [0, 0, 255]);
    assert!(
        first
            .runs
            .iter()
            .any(|run| run.text.contains("Body") && run.font_size == 12.0)
    );
}

#[test]
fn generated_blocks_use_normal_flow_breaks_and_decorations() {
    let html = format!(
        r#"<style>{PROFILE}
      p::before {{content:"Before";display:block;background:#ddeeff;padding:3pt}}
      p::after {{content:"After";display:block;break-before:page}}
      </style><p>Body</p>"#
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(trace.pages.len(), 2);
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Before", "Body"]
    );
    assert_eq!(trace.pages[1][0].text, "After");
    assert_eq!(trace.pages[0][0].x, 39.0);
    assert_eq!(trace.pages[0][1].x, 36.0);
}

#[test]
fn selector_lists_and_descendant_universals_target_generated_boxes_only() {
    let html = format!(
        r#"<style>{PROFILE}
      [data-value="::before"] > span::before, p:after {{content:"!"}}
      section ::after {{content:"?"}}
      span::before {{color:red}}
      </style><section data-value="::before"><span>A</span></section><p>B</p>"#
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["!A?", "B!"]
    );
    for selector in [
        "p::before span",
        "p::after::before",
        "p: before",
        "p::before:hover",
    ] {
        let html = format!(
            "<style>{PROFILE}{selector}{{content:'X'}}p::after{{content:'Y'}}</style><p>B</p>"
        );
        let output = render_html(&html, &local_options()).unwrap();
        assert_eq!(
            output.layout.unwrap().pages[0][0].text,
            "BY",
            "invalid selector must be discarded without retargeting or removing its valid sibling: {selector}"
        );
        assert!(output.diagnostics.iter().any(|diagnostic| matches!(diagnostic, crate::RenderDiagnostic::Css(diagnostic) if diagnostic.kind == crate::style::StyleDiagnosticKind::InvalidRule)), "invalid selector needs a source diagnostic: {selector}");
    }
}

#[test]
fn generated_content_surrounds_breaks_and_attribute_names_follow_html() {
    let html = format!(
        r#"<style>{PROFILE}
      br::before{{content:"before"}}br::after{{content:"after"}}
      p::before{{content:attr(DATA-X)}}
      </style><p data-x="Hidden">A<br>B</p><p>C<br style="display:block">D</p>"#
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["HiddenAbefore", "afterB", "C", "before", "after", "D"]
    );
}

#[test]
fn generated_attribute_names_follow_html_case_insensitivity() {
    let trace = common::render_trace(
        "<style>p::before{content:attr(DATA-X)}</style><p data-x='Attribute'>Body</p>",
    );
    assert_eq!(
        trace.pages[0][0].text, "AttributeBody",
        "CSS attribute names follow the HTML document's ASCII-insensitive attribute matching"
    );
}

#[test]
fn repeated_attribute_expansion_is_bounded_before_inline_collection() {
    const ATTRIBUTE_BYTES: usize = 64 * 1024;
    const REPETITIONS: usize = 1025;
    let attribute = "a".repeat(ATTRIBUTE_BYTES);
    let content = "attr(data-value) ".repeat(REPETITIONS);
    let html = format!(
        "<style>{PROFILE}p::before{{content:{content}}}</style><p data-value='{attribute}'>Body</p>"
    );
    let output = render_html(&html, &local_options())
        .unwrap_or_else(|error| panic!("repeated attribute expansion must not fail: {error}"));
    const SUBSTITUTION_LIMIT_BYTES: usize = 1024 * 1024;
    let bound = format!("CSS variable expanded value bytes exceeds {SUBSTITUTION_LIMIT_BYTES}");
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            crate::RenderDiagnostic::Degraded(crate::style::Degradation::InvalidAtComputedValueTime {
                declaration,
                reason,
            }) if declaration.starts_with("content: ") && *reason == bound
        )),
        "shared computed substitution must bound expansion before inline collection: {:?}",
        output.diagnostics
    );
    assert_eq!(
        output.layout.unwrap().pages[0][0].text,
        "Body",
        "the bounded content is invalid at computed-value time, so ::before generates nothing"
    );
}
