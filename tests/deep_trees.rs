mod common;

use common::{PROFILE, local_options};
use ferri_pdf::{RenderOptions, render_html};

#[test]
fn ordinary_formatting_depth_limit_is_safe_at_its_boundary() {
    const FORMATTING_LIMIT: usize = 512;
    const DOCUMENT_AND_PARAGRAPH_LEVELS: usize = 2;
    for (blocks, allowed) in [
        (FORMATTING_LIMIT - DOCUMENT_AND_PARAGRAPH_LEVELS, true),
        (FORMATTING_LIMIT - DOCUMENT_AND_PARAGRAPH_LEVELS + 1, false),
    ] {
        let html = format!(
            "<!doctype html>{}<p>Boundary</p>{}",
            "<div>".repeat(blocks),
            "</div>".repeat(blocks)
        );
        let result = render_html(
            &html,
            &RenderOptions {
                stylesheets: vec![PROFILE.to_owned()],
                ..local_options()
            },
        );
        if allowed {
            let trace = result
                .expect("the inclusive formatting limit remains usable")
                .layout
                .unwrap();
            assert_eq!(
                trace
                    .pages
                    .iter()
                    .flatten()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>(),
                ["Boundary"]
            );
        } else {
            assert!(
                matches!(result, Err(ferri_pdf::Error::Document(message)) if message.starts_with("formatting nesting exceeds")),
                "one further level must return the existing formatting error instead of overflowing its stack"
            );
        }
    }
}
