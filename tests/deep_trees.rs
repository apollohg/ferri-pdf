mod common;

use common::{PROFILE, local_options};
use ferri_pdf::{RenderDiagnostic, RenderOptions, render_html};

#[test]
fn ordinary_formatting_depth_limit_flattens_deeper_content() {
    const FORMATTING_LIMIT: usize = 512;
    const DOCUMENT_AND_PARAGRAPH_LEVELS: usize = 2;
    for (blocks, flattened) in [
        (FORMATTING_LIMIT - DOCUMENT_AND_PARAGRAPH_LEVELS, false),
        (FORMATTING_LIMIT - DOCUMENT_AND_PARAGRAPH_LEVELS + 1, true),
    ] {
        let html = format!(
            "<!doctype html>{}<p>Boundary</p>{}",
            "<div>".repeat(blocks),
            "</div>".repeat(blocks)
        );
        let output = render_html(
            &html,
            &RenderOptions {
                stylesheets: vec![PROFILE.to_owned()],
                ..local_options()
            },
        )
        .unwrap_or_else(|error| panic!("{blocks} nested blocks must render: {error}"));
        let trace = output.layout.expect("layout requested");
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Boundary"],
            "{blocks} nested blocks keep their content"
        );
        let reported = output
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, RenderDiagnostic::FormattingLimit { .. }));
        assert_eq!(
            reported, flattened,
            "{blocks} nested blocks report flattening the content past the limit: {:?}",
            output.diagnostics
        );
    }
}

const SVG_NESTING: &str = "SVG element nesting";

/// SVG content, named, nested a given number of levels deep.
type NestedSvg = (&'static str, fn(usize) -> String);

#[test]
fn svg_nesting_depth_limit_is_safe_at_its_boundary() {
    const SVG_LIMIT: usize = 128;
    // Each structure nests its innermost element `levels + 1` below the root `svg` element,
    // and recurses with the largest frames per level as it converts and renders.
    let structures: [NestedSvg; 4] = [
        ("nested svg elements", |levels| {
            format!(
                "{}<rect width='5' height='5'/>{}",
                "<svg>".repeat(levels),
                "</svg>".repeat(levels)
            )
        }),
        ("nested spans", |levels| {
            format!(
                "<text y='20'>{}x{}</text>",
                "<tspan>".repeat(levels),
                "</tspan>".repeat(levels)
            )
        }),
        ("nested links in text", |levels| {
            format!(
                "<text y='20'>{}x{}</text>",
                "<a>".repeat(levels),
                "</a>".repeat(levels)
            )
        }),
        ("nested groups with opacity", |levels| {
            format!(
                "{}<rect width='5' height='5'/>{}",
                "<g opacity='.9'>".repeat(levels),
                "</g>".repeat(levels)
            )
        }),
    ];
    for (name, content) in structures {
        for (levels, flattened) in [(SVG_LIMIT - 1, false), (SVG_LIMIT, true)] {
            let html = format!(
                "<!doctype html><svg width='200' height='100'>{}</svg>",
                content(levels)
            );
            let output = render_html(&html, &local_options())
                .unwrap_or_else(|error| panic!("{name} {levels} deep must render: {error}"));
            let limited: Vec<_> = output
                .diagnostics
                .iter()
                .filter(|diagnostic| matches!(diagnostic, RenderDiagnostic::FormattingLimit { .. }))
                .collect();
            if flattened {
                assert!(
                    matches!(
                        limited.as_slice(),
                        [RenderDiagnostic::FormattingLimit {
                            resource: SVG_NESTING,
                            limit: SVG_LIMIT,
                            ..
                        }]
                    ),
                    "{name} one level past the limit must render flattened with one diagnostic: {limited:?}"
                );
            } else {
                assert!(
                    limited.is_empty(),
                    "{name} at the inclusive limit must render whole: {limited:?}"
                );
            }
        }
    }
}

#[test]
fn deeply_nested_svg_text_renders_flattened() {
    // Nested far past the limit, beside many sibling spans.
    const NESTED: usize = 200;
    const SIBLINGS: usize = 400;
    let html = format!(
        "<!doctype html><svg width=200 height=100><text x=0 y=20>{}x{}{}</text></svg>",
        "<tspan>".repeat(NESTED),
        "</tspan>".repeat(NESTED),
        "<tspan>y</tspan>".repeat(SIBLINGS)
    );
    let output = render_html(&html, &local_options()).expect("deep SVG text must render");
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            RenderDiagnostic::FormattingLimit {
                resource: SVG_NESTING,
                ..
            }
        )),
        "flattening must be diagnosed: {:?}",
        output.diagnostics
    );
}
