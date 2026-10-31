mod common;

use common::{local_options, lossy_options};
use ferri_pdf::{
    Degradation, Error, HtmlPart, ImageError, LayoutDiagnostic, RenderDiagnostic, RenderOutput,
    render_html, render_parts,
};

const OMITTED_BOX: &str = "<!doctype html><p>before</p>\
                           <p style='font-size-adjust:1e38'>Hello</p><p>after</p>";
/// Shadow roots one level past the shadow tree nesting limit of 64.
const SHADOW_LEVELS: usize = 65;
const MEBIBYTE: usize = 1024 * 1024;
/// Copies of a mebibyte that pass the 64 MiB generated text limit.
const PAST_TEXT_LIMIT: usize = 65;
/// Block nesting past the formatting depth limit of 512, which flattens what lies deeper.
const PAST_FORMATTING_DEPTH: usize = 600;
const LOST_CONTENT_MESSAGE: &str = "content left out of the PDF";

/// The diagnostics of the `Error::ContentLost` that rendering `html` with the default policy
/// must return.
fn lost(html: &str) -> Vec<RenderDiagnostic> {
    match render_html(html, &local_options()) {
        Err(Error::ContentLost { diagnostics }) => diagnostics,
        Err(error) => panic!("content loss must fail as ContentLost, got: {error}"),
        Ok(output) => panic!(
            "the default policy must fail on lost content, rendered with {:?}",
            output.diagnostics
        ),
    }
}

fn rendered(html: &str) -> RenderOutput {
    render_html(html, &lossy_options())
        .unwrap_or_else(|error| panic!("the Render policy must render lost content: {error}"))
}

fn shadow_nesting() -> String {
    format!(
        "<!doctype html>{}<p>Deep</p>{}",
        "<div><template shadowrootmode=open>".repeat(SHADOW_LEVELS),
        "</template></div>".repeat(SHADOW_LEVELS)
    )
}

#[test]
fn an_omitted_box_fails_the_default_render_and_names_the_loss() {
    let diagnostics = lost(OMITTED_BOX);
    assert!(
        matches!(
            diagnostics.as_slice(),
            [RenderDiagnostic::Layout(
                LayoutDiagnostic::OmittedBox { .. }
            )]
        ),
        "only the omitted box is reported lost: {diagnostics:?}"
    );
    let message = Error::ContentLost { diagnostics }.to_string();
    assert!(
        message.contains(LOST_CONTENT_MESSAGE)
            && message.contains("a box layout cannot form is omitted"),
        "the error says what was lost: {message}"
    );
    let output = rendered(OMITTED_BOX);
    assert!(
        output
            .diagnostics
            .iter()
            .any(RenderDiagnostic::loses_content),
        "the Render policy keeps the loss among its diagnostics: {:?}",
        output.diagnostics
    );
}

#[test]
fn a_tree_limit_fails_the_default_render() {
    let html = shadow_nesting();
    let diagnostics = lost(&html);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, RenderDiagnostic::TreeLimit(_))),
        "the cut-off tree is reported lost: {diagnostics:?}"
    );
    assert!(
        rendered(&html).page_count > 0,
        "the Render policy still renders"
    );
}

#[test]
fn a_generated_text_cut_fails_the_default_render() {
    let value = format!("{}x", " ".repeat(MEBIBYTE - 1));
    let html = format!(
        "<style>p{{string-set:t '{value}'}}@page{{@top-center{{content:{}}}}}</style><p>Body</p>",
        "string(t) ".repeat(PAST_TEXT_LIMIT)
    );
    let diagnostics = lost(&html);
    assert!(
        diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            RenderDiagnostic::Degraded(Degradation::GeneratedTextLimit { .. })
        )),
        "the cut generated text is reported lost: {diagnostics:?}"
    );
}

#[test]
fn a_part_losing_content_fails_the_multipart_job_as_that_part() {
    let parts = [
        HtmlPart::from_html("<p>first</p>"),
        HtmlPart::from_html(OMITTED_BOX),
    ];
    match render_parts(&parts, &local_options()) {
        Err(Error::Part { part, source }) => {
            assert_eq!(part, 2, "the second part loses the content");
            assert!(
                matches!(*source, Error::ContentLost { ref diagnostics } if !diagnostics.is_empty()),
                "the part fails with its lost content: {source}"
            );
        }
        Err(error) => panic!("the losing part must fail the job, got: {error}"),
        Ok(_) => panic!("a part losing content must fail the default multipart job"),
    }
    let output = render_parts(&parts, &lossy_options())
        .unwrap_or_else(|error| panic!("the Render policy renders every part: {error}"));
    assert!(
        output.parts[1]
            .diagnostics
            .iter()
            .any(RenderDiagnostic::loses_content),
        "the loss is reported with its part: {:?}",
        output.parts[1].diagnostics
    );
}

#[test]
fn recoveries_render_under_the_default_policy() {
    for (name, html) in [
        (
            "invalid CSS",
            "<style>p{color:;width:12qq}@media ((({}</style><p>text</p>".to_owned(),
        ),
        (
            "a missing image",
            "<img src=missing.png alt='Alt text'>".to_owned(),
        ),
        (
            "unsupported CSS",
            "<style>p{scroll-initial-target:nearest}</style><p>text</p>".to_owned(),
        ),
        (
            "nesting flattened past the formatting depth",
            format!(
                "{}<p>deep</p>{}",
                "<div>".repeat(PAST_FORMATTING_DEPTH),
                "</div>".repeat(PAST_FORMATTING_DEPTH)
            ),
        ),
    ] {
        let output = render_html(&html, &local_options())
            .unwrap_or_else(|error| panic!("{name} must render under the default policy: {error}"));
        assert!(
            !output.diagnostics.is_empty()
                && !output
                    .diagnostics
                    .iter()
                    .any(RenderDiagnostic::loses_content),
            "{name} is recovered and reported without content loss: {:?}",
            output.diagnostics
        );
    }
}

#[test]
fn the_command_line_fails_on_lost_content_unless_allowed() {
    const ALLOW_FLAG: &str = "--allow-content-loss";
    let directory =
        std::env::temp_dir().join(format!("ferri-pdf-content-loss-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("input.html"), OMITTED_BOX).unwrap();
    let run = |flags: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_ferri-pdf"))
            .args(["input.html", "output.pdf"])
            .args(flags)
            .current_dir(&directory)
            .output()
            .unwrap()
    };
    let failed = run(&[]);
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(
        !failed.status.success() && stderr.contains(LOST_CONTENT_MESSAGE),
        "the default run fails and lists the loss: {}\n{stderr}",
        failed.status
    );
    assert!(
        !directory.join("output.pdf").exists(),
        "a failed run writes no PDF"
    );
    let allowed = run(&[ALLOW_FLAG]);
    let stderr = String::from_utf8_lossy(&allowed.stderr);
    assert!(
        allowed.status.success() && stderr.contains("warning: a box layout cannot form"),
        "{ALLOW_FLAG} renders and warns of the loss: {}\n{stderr}",
        allowed.status
    );
    assert!(
        std::fs::read(directory.join("output.pdf"))
            .unwrap()
            .starts_with(b"%PDF-"),
        "{ALLOW_FLAG} writes the PDF"
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

/// A data URL of a GIF whose logical screen, 65535 by 65535, needs more than the 2 GiB the
/// renderer decodes.
fn oversized_gif_url() -> String {
    const SCREEN_SIZE: std::ops::Range<usize> = 6..10;
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 255, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Gif,
        )
        .expect("encode fixture image");
    bytes[SCREEN_SIZE].fill(u8::MAX);
    let encoded: String = bytes.iter().map(|byte| format!("%{byte:02X}")).collect();
    format!("data:image/gif,{encoded}")
}

fn past_decoding_budget(diagnostic: &RenderDiagnostic) -> bool {
    matches!(
        diagnostic,
        RenderDiagnostic::UnavailableImage(failure)
            if matches!(failure.error, ImageError::Dimensions { .. })
    )
}

#[test]
fn an_image_past_the_decoding_budget_fails_the_default_render() {
    let html = format!("<p>before</p><img src='{}' alt=huge>", oversized_gif_url());
    let diagnostics = lost(&html);
    assert!(
        matches!(diagnostics.as_slice(), [diagnostic] if past_decoding_budget(diagnostic)),
        "only the image the decoding budget leaves out is reported lost: {diagnostics:?}"
    );
    let output = rendered(&html);
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| past_decoding_budget(diagnostic) && diagnostic.loses_content()),
        "the Render policy renders and reports the image left out: {:?}",
        output.diagnostics
    );
}
