use super::common;

use crate::style::StyleDiagnosticKind;
use crate::{
    RenderDiagnostic, Resource, ResourceBase, ResourceError, ResourceResolver, StyleError, Url,
    render_html, render_with_resolver,
};
use common::{assert_pt, local_options};

fn render_css(css: &str) -> crate::RenderOutput {
    render_html(
        &format!("<style>{css}</style><p>survives</p>"),
        &local_options(),
    )
    .unwrap()
}

#[test]
fn malformed_rule_recovers_at_correct_css_boundary() {
    let output = render_css("p, ] { color:red; font-size:99pt } p { font-size:18pt }");
    let line = &output.layout.as_ref().unwrap().pages[0][0];
    assert_eq!(line.text, "survives");
    assert_pt(line.font_size, 18.0);
    assert!(output.diagnostics.iter().any(
        |d| matches!(d, RenderDiagnostic::Css(d) if d.kind == StyleDiagnosticKind::InvalidRule)
    ));
}

#[test]
fn unknown_rule_does_not_consume_following_rule() {
    let output = render_css(
        "@future f([one; two]) { p { font-size:99pt } @inner { p{color:red} } } @future-statement x(y); p{font-size:18pt}",
    );
    assert_eq!(output.layout.as_ref().unwrap().pages[0][0].text, "survives");
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 18.0);
    assert_eq!(output.diagnostics.iter().filter(|d| matches!(d, RenderDiagnostic::Css(d) if d.kind == StyleDiagnosticKind::InvalidRule)).count(), 2);
}

#[test]
fn invalid_declaration_preserves_valid_siblings() {
    for css in [
        "p{broken f(one; two[three]);font-size:18pt;color:navy}",
        "p{color:url(bad\"quote);font-size:18pt;color:navy}",
        "p{color:\"bad\n; font-size:18pt;color:navy}",
        "p{font-size:calc(12pt + [bad]);font-size:18pt;color:navy}",
        "p{color:rgb(0,bad,0);font-size:18pt;color:navy}",
        "p{--bad:url(bad\"quote);font-size:18pt;color:navy}",
        "p{color:red !oops; font-size:18pt;color:navy}",
        "p{--keep:{x;y}; broken; font-size:18pt;color:navy}",
    ] {
        let output = render_css(css);
        let line = &output.layout.unwrap().pages[0][0];
        assert_pt(line.font_size, 18.0);
        assert_eq!(line.color.rgb, [0, 0, 128], "valid sibling after {css:?}");
        assert!(output.diagnostics.iter().any(|d| matches!(d, RenderDiagnostic::Css(d) if d.kind == StyleDiagnosticKind::InvalidDeclaration)), "missing source diagnostic for {css:?}");
    }
}

#[test]
fn nesting_diagnostics_distinguish_rules_from_declarations() {
    let output = render_css(
        "p{\n broken f(a;b);\n broken @not-an-at-rule;\n &bad{color:red}\n @unknown future;\n font-size:18pt}",
    );
    assert_pt(output.layout.as_ref().unwrap().pages[0][0].font_size, 18.0);
    let diagnostics: Vec<_> = output
        .diagnostics
        .iter()
        .filter_map(|diagnostic| match diagnostic {
            RenderDiagnostic::Css(diagnostic) => Some((diagnostic.location.line, diagnostic.kind)),
            _ => None,
        })
        .collect();
    assert_eq!(
        diagnostics,
        [
            (2, StyleDiagnosticKind::InvalidDeclaration),
            (3, StyleDiagnosticKind::InvalidDeclaration),
            (4, StyleDiagnosticKind::InvalidRule),
            (5, StyleDiagnosticKind::InvalidRule)
        ],
        "mixed block contents must preserve both diagnostic categories and their original CSS lines"
    );
}

#[test]
fn inline_declaration_recovery_uses_the_same_parser() {
    let output = render_html(
        "<p style='font-size:18pt; broken f(a;b); color:navy; font-size:20pt'>survives</p>",
        &local_options(),
    )
    .unwrap();
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 20.0);
    assert!(output.diagnostics.iter().any(|d| matches!(d, RenderDiagnostic::Css(d) if d.kind == StyleDiagnosticKind::InvalidDeclaration)));
}

#[test]
fn eof_closes_valid_constructs_and_at_rule_names_are_case_insensitive() {
    let output = render_html(&format!("<style>{}</style><style>@PAGE{{size:300pt 400pt; margin:20pt}}p{{font-size:18pt;color:navy</style><p>survives</p>", common::PROFILE), &local_options()).unwrap();
    let trace = output.layout.unwrap();
    assert_pt(trace.page_width, 300.0);
    assert_pt(trace.page_height, 400.0);
    assert_pt(trace.pages[0][0].font_size, 18.0);
}

#[test]
fn unsupported_active_winner_is_still_an_error() {
    let error = render_html(
        "<style>p{tab-size:8;tab-size:2pt}</style><p>grid</p>",
        &local_options(),
    )
    .err()
    .expect("active unsupported tab length must fail");
    assert!(
        matches!(error, crate::Error::Css(StyleError::Unsupported(_))),
        "active unsupported capability must retain CSS classification: {error:?}"
    );
    let output = render_css("p{tab-size:2pt;tab-size:8;font-size:18pt}");
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 18.0);
}

struct Sheet;
impl ResourceResolver for Sheet {
    fn resolve(&self, reference: &str, _: &ResourceBase) -> Result<Resource, ResourceError> {
        assert_eq!(reference, "sheet.css");
        Ok(Resource {
            bytes: b"p {\n  color:red;\n  broken;\n  font-size:18pt;\n}".to_vec(),
            base: Url::parse("https://example.test/styles/final.css")
                .unwrap()
                .into(),
        })
    }
}

#[test]
fn stylesheet_diagnostic_keeps_final_base_and_css_line() {
    let output = render_with_resolver(
        "<link rel=stylesheet href=sheet.css><p>survives</p>",
        &local_options(),
        &Sheet,
    )
    .unwrap();
    let diagnostic = output
        .diagnostics
        .iter()
        .find_map(|d| match d {
            RenderDiagnostic::Css(d) if d.kind == StyleDiagnosticKind::InvalidDeclaration => {
                Some(d)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        diagnostic.location.base,
        Url::parse("https://example.test/styles/final.css")
            .unwrap()
            .into()
    );
    assert_eq!(diagnostic.location.line, 3);
    assert!(diagnostic.location.column > 0);
    assert!(diagnostic.context.contains("broken"));
}

#[test]
fn syntax_recovery_does_not_swallow_parse_limits() {
    const OVERSIZED_NESTING: usize = 300;
    let css = format!(
        "p{{--deep:{}0{};font-size:18pt}}",
        "f(".repeat(OVERSIZED_NESTING),
        ")".repeat(OVERSIZED_NESTING)
    );
    let error = render_html(
        &format!("<style>{css}</style><p>bounded</p>"),
        &local_options(),
    )
    .err()
    .expect("component bound must fail");
    assert!(
        matches!(
            error,
            crate::Error::Css(StyleError::GrammarLimit {
                resource: "component nesting",
                ..
            })
        ),
        "typed parse limit must propagate: {error:?}"
    );
}

#[test]
fn syntax_recovery_does_not_swallow_resource_denial() {
    struct Denied;
    impl ResourceResolver for Denied {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            assert_eq!(reference, "blocked.ttf");
            Err(ResourceError::Denied(base.resolve(reference)?))
        }
    }
    let result = render_with_resolver(
        "<style>@font-face{font-family:Denied;src:url(blocked.ttf)}p{broken; font-family:Denied}</style><p>denied</p>",
        &local_options(),
        &Denied,
    );
    assert!(matches!(
        result,
        Err(crate::Error::Css(StyleError::Resource(
            ResourceError::Denied(_)
        )))
    ));
}

#[test]
fn curly_block_mixed_with_other_value_is_rejected_before_substitution() {
    let output = render_css("p{font-size:18pt;--n:20pt;font-size:var(--n) {};color:navy}");
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 18.0);
}

#[test]
fn unknown_property_url_does_not_require_resource_resolution() {
    let output = render_css("p{unknown-feature:url(http://[bad);font-size:18pt}");
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 18.0);
}

#[test]
fn implicit_eof_closes_a_valid_calculation() {
    let output = render_css("p{font-size:calc(6pt + 12pt");
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 18.0);
}

#[test]
fn invalid_page_rule_and_font_face_do_not_remove_valid_siblings() {
    let output = render_css(
        "@page :unknown {size:999pt} @font-face{font-family:MissingSource} p{font-size:18pt}",
    );
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 18.0);
    assert!(output.diagnostics.iter().filter(|d| matches!(d, RenderDiagnostic::Css(d) if d.kind == StyleDiagnosticKind::InvalidRule)).count() >= 2);
}

#[test]
fn unsupported_generated_winner_keeps_css_error_classification() {
    let error = render_html(
        "<style>p::before{content:'grid';tab-size:2pt}</style><p>body</p>",
        &local_options(),
    )
    .err()
    .expect("active generated unsupported tab length must fail");
    assert!(
        matches!(error, crate::Error::Css(StyleError::Unsupported(_))),
        "generated capability classification: {error:?}"
    );
}

#[test]
fn inline_diagnostics_use_document_base_after_an_external_sheet_redirect() {
    let document_base: ResourceBase = Url::parse("https://example.test/documents/report.html")
        .unwrap()
        .into();
    let mut options = local_options();
    options.base = document_base.clone();
    let output = render_with_resolver(
        "<link rel=stylesheet href=sheet.css><p style='broken; font-size:18pt'>survives</p>",
        &options,
        &Sheet,
    )
    .unwrap();
    let locations: Vec<_> = output
        .diagnostics
        .iter()
        .filter_map(|d| match d {
            RenderDiagnostic::Css(d) if d.kind == StyleDiagnosticKind::InvalidDeclaration => {
                Some(&d.location)
            }
            _ => None,
        })
        .collect();
    assert!(
        locations
            .iter()
            .any(|location| location.base == document_base && location.line == 1),
        "inline CSS must retain the document base: {locations:?}"
    );
    assert!(
        locations.iter().any(|location| location.base
            == Url::parse("https://example.test/styles/final.css")
                .unwrap()
                .into()
            && location.line == 3),
        "external CSS must retain its final resource base: {locations:?}"
    );
}

#[test]
fn valid_page_selection_resolves_without_invalid_syntax_diagnostics() {
    for prelude in [
        ":first",
        " :first",
        "named:first",
        " named:first",
        "named, :right",
    ] {
        let output = render_html(
            &format!(
                "<style>@page {prelude} {{size:300pt 400pt}}p{{page:named}}</style><p>page</p>"
            ),
            &local_options(),
        )
        .unwrap_or_else(|error| panic!("valid page selector {prelude:?}: {error}"));
        let trace = output.layout.unwrap();
        assert_eq!(
            [trace.page_width, trace.page_height],
            [300.0, 400.0],
            "selected geometry for {prelude:?}"
        );
        assert!(
            !output.diagnostics.iter().any(|d| matches!(d, RenderDiagnostic::Css(d) if d.kind == StyleDiagnosticKind::InvalidRule)),
            "valid page selector {prelude:?} must not be discarded"
        );
    }
    let output = render_css("@page named :first {size:999pt} p{font-size:18pt}");
    assert_pt(output.layout.unwrap().pages[0][0].font_size, 18.0);
    assert!(output.diagnostics.iter().any(
        |d| matches!(d, RenderDiagnostic::Css(d) if d.kind == StyleDiagnosticKind::InvalidRule)
    ));
}

#[test]
fn running_strings_are_active_in_assignment_and_page_margin_contexts() {
    let output =
        render_css("p{string-set:chapter content()}@page{@top-left{content:'H:' string(chapter)}}");
    let lines = &output.layout.as_ref().unwrap().pages[0];
    assert!(
        lines.iter().any(|line| line.text == "H:survives"),
        "the margin must use the captured assignment, not silently discard valid syntax"
    );
    assert!(!output.diagnostics.iter().any(|diagnostic| matches!(diagnostic, RenderDiagnostic::Css(diagnostic) if matches!(diagnostic.kind, StyleDiagnosticKind::InvalidDeclaration | StyleDiagnosticKind::InvalidRule))));
}

#[test]
fn valid_leader_content_is_not_discarded_as_invalid_syntax() {
    let declaration = "p::after{content:leader('.')}";
    let html = format!("<style>{declaration}</style><p id=chapter>Chapter</p>");
    let output = render_html(&html, &local_options()).expect("valid leader content must render");
    assert!(
        output
            .layout
            .unwrap()
            .pages
            .iter()
            .flatten()
            .any(|line| line.text.contains("..."))
    );
}

#[test]
fn book_content_grammar_preserves_cascade_and_invalid_value_recovery() {
    for value in [
        "string(chapter)",
        "string(chapter,first)",
        "string(chapter,start)",
        "string(chapter,last)",
        "string(chapter,first-except)",
        "leader(dotted)",
        "leader(solid)",
        "leader(space)",
        "leader('..')",
        "target-counter('#chapter',page)",
        "target-counter('#chapter',page,lower-roman)",
        "target-counters('#chapter',section,'.')",
        "target-counters('#chapter',section,'.',decimal)",
        "target-text('#chapter')",
        "target-text('#chapter',before)",
        "target-text('#chapter',after)",
        "target-text('#chapter',first-letter)",
        "contents",
        "content()",
    ] {
        let html = format!("<style>p::after{{content:{value};content:' tail'}}</style><p>Body</p>");
        let output = render_html(&html, &local_options()).unwrap();
        assert!(
            !output
                .diagnostics
                .iter()
                .any(|d| matches!(d, RenderDiagnostic::Css(d)
            if d.kind == StyleDiagnosticKind::InvalidDeclaration)),
            "valid overridden content: {value}"
        );
    }
    for value in [
        "chapter content()",
        "chapter content(before) ':' content(after), title attr(title)",
        "chapter counter(section) '.' counters(subsection,'.')",
        "none",
    ] {
        let html = format!("<style>p{{string-set:{value};string-set:none}}</style><p>Body</p>");
        let output = render_html(&html, &local_options()).unwrap();
        assert!(
            !output
                .diagnostics
                .iter()
                .any(|d| matches!(d, RenderDiagnostic::Css(d)
            if d.kind == StyleDiagnosticKind::InvalidDeclaration)),
            "valid overridden string assignment: {value}"
        );
    }
    for value in [
        "string()",
        "string(chapter,)",
        "string(chapter,middle)",
        "target-counter('#chapter',page,)",
        "target-text('#chapter',)",
        "leader()",
    ] {
        let html = format!("<style>p::after{{content:' tail';content:{value}}}</style><p>Body</p>");
        let output = render_html(&html, &local_options()).unwrap();
        assert!(
            output
                .diagnostics
                .iter()
                .any(|d| matches!(d, RenderDiagnostic::Css(d)
            if d.kind == StyleDiagnosticKind::InvalidDeclaration)),
            "malformed content needs recovery: {value}"
        );
        assert!(
            output.layout.unwrap().pages[0]
                .iter()
                .any(|line| line.text.contains("tail")),
            "invalid content must preserve the earlier declaration: {value}"
        );
    }
}
