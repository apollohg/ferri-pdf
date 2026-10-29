mod common;

use ferri_pdf::{
    FileResolver, HtmlPart, Resource, ResourceBase, ResourceError, ResourceResolver,
    render_parts_with_resolver, render_with_resolver,
};

const BROKEN_IMAGE: &str = "broken.png";

#[test]
fn empty_alternative_text_of_a_failed_inline_image_does_not_create_a_line() {
    const LINE_HEIGHT: f32 = 28.0;
    let render = |content: &str, image| {
        render_with_resolver(
            &format!(
                "<!doctype html><style>{}body{{font-size:20pt;line-height:{LINE_HEIGHT}pt}}p{{margin:0}}</style>{content}<p>After</p>",
                common::PROFILE
            ),
            &common::local_options(),
            &ImageResolver(image),
        )
        .unwrap()
        .layout
        .unwrap()
    };
    let reference = render("", None);
    let baseline = reference.pages[0].last().unwrap().baseline;
    let cases = [
        ("empty", "<img src='broken.png' alt=''>", 0.0),
        ("whitespace", " \n <img src='broken.png' alt=''> \n ", 0.0),
        ("absent", "<img src='broken.png'>", LINE_HEIGHT),
        (
            "text",
            "<img src='broken.png' alt='Alternative'>",
            LINE_HEIGHT,
        ),
        (
            "block",
            "<img src='broken.png' alt='' style='display:block'>",
            0.0,
        ),
        (
            "forced-break",
            "<img src='broken.png' alt=''><br>",
            LINE_HEIGHT,
        ),
        (
            "preserved-space",
            "<span style='white-space:pre'> <img src='broken.png' alt=''></span>",
            LINE_HEIGHT,
        ),
    ];
    let advances: Vec<_> = cases
        .iter()
        .map(|(name, content, _)| {
            let trace = render(content, None);
            (*name, trace.pages[0].last().unwrap().baseline - baseline)
        })
        .collect();
    let expected: Vec<_> = cases
        .iter()
        .map(|(name, _, advance)| (*name, *advance))
        .collect();
    assert_eq!(
        advances, expected,
        "an empty-alt image fallback must not add a 28pt line"
    );
    for css in [
        "width:20pt",
        "height:40pt",
        "width:0;height:0",
        "padding:2pt",
        "border:2pt solid red",
        "margin:2pt",
        "outline:2pt solid red",
        "background:red",
        "margin-top:2pt;margin-bottom:-2pt",
        "padding:2pt;margin:-2pt",
    ] {
        let failed = render(
            &format!("<img src='broken.png' alt='' style='{css}'>"),
            None,
        );
        let control = render(
            &format!("<span style='display:inline-block;{css}'></span>"),
            None,
        );
        common::assert_labelled_pt(
            &format!("explicit image geometry or paint {css} retains its inline line"),
            failed.pages[0].last().unwrap().baseline,
            control.pages[0].last().unwrap().baseline,
        );
    }
    let valid = render(
        "<img src='broken.png' alt='' style='width:20pt;height:20pt'>",
        Some(include_bytes!("oracle/assets/image-pattern.png").to_vec()),
    );
    assert!(valid.pages[0].last().unwrap().baseline > baseline);
    assert!(
        valid
            .display_lists
            .iter()
            .flatten()
            .any(|item| matches!(item, ferri_pdf::DisplayItem::Image(_))),
        "a loaded image with empty alt still paints"
    );
}

#[test]
fn empty_failed_inline_images_do_not_separate_adjoining_paragraph_margins() {
    let baselines = |image: &str| {
        let output = render_with_resolver(
            &format!("<!doctype html><style>{} @page{{size:300pt 200pt;margin:20pt}}body{{font-size:12pt}}p{{margin:1em 0}}</style><p>BEFORE</p>{image}<p>AFTER</p>", common::PROFILE),
            &common::local_options(), &ImageResolver(None),
        ).expect("missing empty-alt image must retain surrounding paragraphs");
        let trace = output.layout.unwrap();
        ["BEFORE", "AFTER"].map(|label| {
            trace.pages[0]
                .iter()
                .find(|line| line.text == label)
                .unwrap_or_else(|| panic!("fixture lost {label}: {:?}", trace.pages[0]))
                .baseline
        })
    };
    let reference = baselines("");
    let block = baselines("<img src=broken.png alt='' style='display:block'>");
    let inline = baselines("<img src=broken.png alt=''>");
    assert_eq!(
        block, reference,
        "empty block fallback permits adjoining margins"
    );
    assert_eq!(
        inline, reference,
        "an ignored zero-height inline fallback line must not split the default12pt paragraph margin strut; inline={inline:?}, block={block:?}, absent={reference:?}"
    );
    assert_eq!(
        baselines("<div style='text-box-trim:trim-both'><img src=broken.png alt=''></div>"),
        reference,
        "text-box trimming cannot create geometry for an ignored image line"
    );
}

#[test]
fn phantom_image_lines_preserve_semantic_events_and_destinations() {
    let output = render_with_resolver(
        &format!("<!doctype html><style>{}@page{{size:300pt 200pt;margin:20pt;@top-left{{content:string(label,last)}}}}p{{margin:1em 0}}</style><p>BEFORE</p><img id=empty src=broken.png alt='' style='string-set:label \"Empty\"'><p><a href='#empty'>AFTER</a></p>", common::PROFILE),
        &common::local_options(), &ImageResolver(None),
    ).expect("phantom image must retain its semantic source");
    let trace = output.layout.unwrap();
    assert!(
        trace.pages[0].iter().any(|line| line.text == "Empty"),
        "string assignment on ignored image line must reach the page header"
    );
    assert_eq!(
        trace.links[0].len(),
        1,
        "later source link must remain clickable"
    );
    assert!(
        trace.destinations.iter().any(Option::is_some),
        "the failed image's destination must remain addressable"
    );
}

#[test]
fn zero_height_real_text_still_separates_block_margins() {
    const MARGIN: f32 = 12.0;
    let baseline = |middle: &str| {
        let output = render_with_resolver(
            &format!("<!doctype html><style>{}body{{font-size:0;line-height:0}}p{{font-size:12pt;line-height:18pt;margin:12pt 0}}</style><p>BEFORE</p>{middle}<p>AFTER</p>", common::PROFILE),
            &common::local_options(), &ImageResolver(None),
        ).unwrap();
        output.layout.unwrap().pages[0]
            .iter()
            .find(|line| line.text == "AFTER")
            .unwrap()
            .baseline
    };
    let reference = baseline("");
    common::assert_labelled_pt(
        "genuine zero-height text keeps its line box",
        baseline("<span>X</span>"),
        reference + MARGIN,
    );
    common::assert_labelled_pt(
        "zero-height phantom image does not keep a line box",
        baseline("<img src=broken.png alt=''>"),
        reference,
    );
    common::assert_labelled_pt(
        "authored zero dimensions retain a genuine atomic line",
        baseline("<img src=broken.png alt='' style='width:0;height:0'>"),
        reference + MARGIN,
    );
}

#[test]
fn phantom_image_lines_do_not_supply_atomic_baselines() {
    for (placement, css, body) in [
        (
            "inline-block-last",
            "",
            "<span class=box>{image}<p>A</p></span> B",
        ),
        (
            "inline-block-first",
            ".box{baseline-source:first}",
            "<span class=box>{image}<p>A</p></span> B",
        ),
        (
            "inline-block-trailing",
            "",
            "<span class=box><p>A</p>{image}</span> B",
        ),
        (
            "flex-first",
            "section{display:flex;align-items:baseline}.box{display:block}",
            "<section><span class=box>{image}<p>A</p></span><span>B</span></section>",
        ),
    ] {
        let render = |image| {
            let body = body.replace("{image}", image);
            render_with_resolver(&format!("<!doctype html><style>{}p{{margin:0}}.box{{display:inline-block}}{css}</style>{body}", common::PROFILE), &common::local_options(), &ImageResolver(None)).unwrap().layout.unwrap()
                .pages[0].iter().flat_map(|line| line.runs.iter().filter(|run| matches!(run.text.trim(), "A" | "B")).map(|run| (run.text.trim().to_owned(), line.baseline + run.baseline_shift))).collect::<Vec<_>>()
        };
        let control = render("");
        let actual = render("<img src=broken.png alt=''>");
        assert_eq!(
            control.len(),
            2,
            "{placement}: both baseline comparison glyphs must exist"
        );
        assert_eq!(
            actual.len(),
            2,
            "{placement}: neither source glyph may disappear"
        );
        assert_eq!(
            actual, control,
            "{placement}: ignored source line must not supply a first/last atomic baseline"
        );
    }
}

struct ImageResolver(Option<Vec<u8>>);
impl ResourceResolver for ImageResolver {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        if reference == BROKEN_IMAGE {
            return self
                .0
                .as_ref()
                .map(|bytes| Resource {
                    bytes: bytes.clone(),
                    base: base.clone(),
                })
                .ok_or_else(|| ResourceError::Read {
                    path: BROKEN_IMAGE.into(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "synthetic missing image",
                    ),
                });
        }
        FileResolver::default().resolve(reference, base)
    }
}

fn corrupt_png_samples() -> Vec<u8> {
    let mut bytes = include_bytes!("oracle/assets/image-pattern.png").to_vec();
    let start = bytes
        .windows(4)
        .position(|chunk| chunk == b"IDAT")
        .expect("PNG has image data");
    bytes[start + 4] ^= u8::MAX;
    bytes
}

#[test]
fn unavailable_and_corrupt_images_do_not_discard_surrounding_document_content() {
    for (case, bytes) in [
        ("missing", None),
        ("header", Some(b"broken image".to_vec())),
        ("samples", Some(corrupt_png_samples())),
    ] {
        let html = format!(
            "<style>{}</style><p>Before</p><img src='{BROKEN_IMAGE}'><img src='{BROKEN_IMAGE}'><p>After</p><img src='../assets/image-pattern.png' width=20>",
            common::PROFILE
        );
        let output = render_with_resolver(&html, &common::local_options(), &ImageResolver(bytes))
            .unwrap_or_else(|error| {
                panic!("{case}: one failed image must not abort the PDF: {error}")
            });
        assert!(output.pdf.starts_with(b"%PDF-"));
        assert_eq!(
            output.diagnostics.len(),
            1,
            "{case}: report each failed source once"
        );
        let trace = output.layout.unwrap();
        for label in ["Before", "After"] {
            assert!(
                trace.pages.iter().flatten().any(|line| line.text == label),
                "{case}: lost {label}"
            );
        }
    }
}

#[test]
fn image_failure_diagnostics_are_retained_for_each_multipart_document() {
    let html = format!(
        "<style>{}</style><img src='{BROKEN_IMAGE}'><p>Part</p>",
        common::PROFILE
    );
    let options = common::local_options();
    let parts = [HtmlPart::from_html(&html), HtmlPart::from_html(&html)];
    let output = render_parts_with_resolver(
        &parts,
        &options,
        &ImageResolver(Some(corrupt_png_samples())),
    )
    .unwrap();
    assert_eq!(output.parts.len(), 2);
    for (index, part) in output.parts.iter().enumerate() {
        assert_eq!(
            part.diagnostics.len(),
            1,
            "part {index} must retain its image diagnostic"
        );
    }
}

#[test]
fn absent_or_unavailable_image_sources_preserve_alternative_text() {
    for image in [
        "<img alt='Alternative'>",
        "<img src='broken.png' alt='Alternative'>",
    ] {
        let html = format!(
            "<style>{}</style><p>Before</p>{image}<p>After</p>",
            common::PROFILE
        );
        let output =
            render_with_resolver(&html, &common::local_options(), &ImageResolver(None)).unwrap();
        assert!(
            output
                .layout
                .unwrap()
                .pages
                .iter()
                .flatten()
                .any(|line| line.text == "Alternative")
        );
    }
}

#[test]
fn corrupt_marker_samples_choose_the_text_fallback_before_layout() {
    for position in ["inside", "outside"] {
        for marker in ["decimal", "none"] {
            let html = format!(
                "<style>{}li{{list-style-position:{position};list-style-type:{marker};list-style-image:url('{BROKEN_IMAGE}')}}</style><ol><li>Before</li><li>After</li></ol>",
                common::PROFILE
            );
            let output = render_with_resolver(
                &html,
                &common::local_options(),
                &ImageResolver(Some(corrupt_png_samples())),
            )
            .unwrap();
            assert_eq!(output.diagnostics.len(), 1);
            let trace = output.layout.unwrap();
            let text = trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<String>();
            assert!(
                text.contains("Before") && text.contains("After"),
                "{text:?}"
            );
            assert_eq!(
                text.contains("1. "),
                marker == "decimal",
                "fallback {position}/{marker}: {text:?}"
            );
            assert_eq!(
                text.contains("2. "),
                marker == "decimal",
                "fallback {position}/{marker}: {text:?}"
            );
        }
    }
}
