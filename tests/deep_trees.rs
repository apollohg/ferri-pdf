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
