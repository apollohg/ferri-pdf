mod common;

use common::{local_options, lossy_options, pdf_number_faults, visible_frame_columns};
use ferri_pdf::{
    Degradation, DisplayItem, LayoutDiagnostic, PrintSettings, RenderDiagnostic, render_html,
};

/// The page limit `ferri-pdf` lays out before omitting the rest of a document.
const PAGE_LIMIT: usize = 10_000;

#[test]
fn content_past_the_page_limit_is_omitted_after_the_last_page() {
    const PAGE_HEIGHT_PT: f32 = 20.0;
    let output = render_html(
        &format!(
            "<!doctype html><style>@page{{size:100pt {PAGE_HEIGHT_PT}pt;margin:0}}html,body{{margin:0}}\
             div{{height:1e9pt;background:rgb(0,0,255)}}</style><div></div><p>after</p>"
        ),
        &lossy_options(),
    )
    .expect("a box taller than the page limit renders the pages it fills");
    assert_eq!(
        output.page_count, PAGE_LIMIT,
        "every page up to the limit is kept"
    );
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            RenderDiagnostic::Layout(LayoutDiagnostic::PageLimit { limit }) if *limit == PAGE_LIMIT
        )),
        "the omitted content is reported, got {:?}",
        output.diagnostics
    );
    let trace = output.layout.unwrap();
    for (page, items) in [0, PAGE_LIMIT - 1].map(|page| (page, &trace.display_lists[page])) {
        let fragment = items.iter().find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == [0, 0, 255] => {
                Some(paint.rect)
            }
            _ => None,
        });
        assert!(
            fragment.is_some_and(|rect| rect.y <= 0.0 && rect.height >= PAGE_HEIGHT_PT),
            "page {} holds a full fragment of the tall box, got {fragment:?}",
            page + 1
        );
    }
    assert!(
        trace
            .pages
            .iter()
            .flatten()
            .all(|line| line.text != "after"),
        "the paragraph after the box lies past the last page"
    );
}

#[test]
fn page_boxes_without_finite_geometry_take_the_initial_page_geometry() {
    // Negative margins on a page 0.01pt tall widen its page area past what layout represents.
    let output = render_html(
        "<!doctype html><style>@page{size:100pt 0.01pt;margin:-3e38pt}</style><p>text</p>",
        &local_options(),
    )
    .expect("a page box without finite geometry renders on the initial page geometry");
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            RenderDiagnostic::Degraded(Degradation::NonFinitePageGeometry)
        )),
        "the replaced geometry is reported, got {:?}",
        output.diagnostics
    );
    let trace = output.layout.unwrap();
    assert_eq!(
        [trace.page_width, trace.page_height],
        PrintSettings::default().paper,
        "the page takes the initial sheet size"
    );
    assert!(
        trace.pages.iter().flatten().any(|line| line.text == "text"),
        "the content renders on the initial page"
    );
}

#[test]
fn font_size_adjust_past_finite_sizes_omits_only_its_text_box() {
    for adjust in ["1e38", "cap-height 1e38", "ex-height 3e38"] {
        let output = render_html(
            &format!(
                "<!doctype html><style>.huge{{font-size-adjust:{adjust}}}</style>\
                 <p>before</p><p class=huge>Hello</p><p>after</p>"
            ),
            &lossy_options(),
        )
        .unwrap_or_else(|error| panic!("font-size-adjust:{adjust} must render: {error}"));
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic,
                RenderDiagnostic::Layout(LayoutDiagnostic::OmittedBox { .. })
            )),
            "font-size-adjust:{adjust}: the omitted box is reported, got {:?}",
            output.diagnostics
        );
        let texts: Vec<_> = output
            .layout
            .unwrap()
            .pages
            .into_iter()
            .flatten()
            .map(|line| line.text)
            .collect();
        assert_eq!(
            texts,
            ["before", "after"],
            "font-size-adjust:{adjust}: only the box whose text would be infinitely large is omitted"
        );
    }
}

/// The text of every line `html` renders, sorted, and the reasons of the boxes its layout
/// omits.
fn texts_and_omitted_boxes(html: &str) -> (Vec<String>, Vec<String>) {
    let output = render_html(html, &lossy_options())
        .unwrap_or_else(|error| panic!("{html} must render: {error}"));
    let faults = pdf_number_faults(&output.pdf);
    assert!(
        faults.is_empty(),
        "{html} must render a valid PDF:\n{}",
        faults.join("\n")
    );
    let omitted = output
        .diagnostics
        .iter()
        .filter_map(|diagnostic| match diagnostic {
            RenderDiagnostic::Layout(LayoutDiagnostic::OmittedBox { reason }) => {
                Some(reason.clone())
            }
            _ => None,
        })
        .collect();
    let mut texts: Vec<_> = output
        .layout
        .unwrap()
        .pages
        .into_iter()
        .flatten()
        .map(|line| line.text)
        .collect();
    texts.sort();
    (texts, omitted)
}

#[test]
fn running_elements_layout_cannot_form_are_omitted_with_their_margin_boxes() {
    let html = "<!doctype html><style>.r{position:running(header);font-size:calc(1e30px*1e30)}\
                @page{@top-center{content:element(header)}@bottom-center{content:'Foot'}}</style>\
                <div class=r>Running</div><p>Body</p>";
    let (texts, omitted) = texts_and_omitted_boxes(html);
    assert_eq!(
        omitted,
        ["non-finite text geometry"],
        "the margin box placing the running element is reported omitted"
    );
    assert_eq!(
        texts,
        ["Body", "Foot"],
        "only the margin box placing the running element is lost, not the body or the other \
         margin boxes"
    );
}

#[test]
fn page_margin_boxes_layout_cannot_form_are_omitted_alone() {
    const HUGE: &str = "font-size:calc(1e30px*1e30)";
    for (rule, reason) in [
        (
            format!("@top-center{{content:'Head';{HUGE}}}"),
            "non-finite text geometry",
        ),
        (
            "@top-center{content:'Head';letter-spacing:3e38px}".to_owned(),
            "non-finite text geometry",
        ),
        (
            format!("@left-middle{{content:'Side text';{HUGE}}}"),
            "non-finite text geometry",
        ),
        (
            "@top-center{content:'Head';padding-left:3e38px;padding-right:3e38px}".to_owned(),
            "non-finite page-margin geometry at TopCenter",
        ),
        (
            format!("@top-left-corner{{content:'C';{HUGE}}}"),
            "non-finite page-margin geometry at TopLeftCorner",
        ),
    ] {
        let (texts, omitted) = texts_and_omitted_boxes(&format!(
            "<!doctype html><style>@page{{{rule}@bottom-center{{content:'Foot'}}}}</style>\
             <p>Body</p>"
        ));
        assert_eq!(
            omitted,
            [reason],
            "{rule}: the omitted margin box is reported"
        );
        assert_eq!(
            texts,
            ["Body", "Foot"],
            "{rule}: only the margin box is lost, not the body or the other margin boxes"
        );
    }
}

#[test]
fn footnote_areas_layout_cannot_form_are_omitted_with_their_footnotes() {
    let html = |padding: &str| {
        format!(
            "<!doctype html><style>@page{{@footnote{{padding-top:{padding};\
             padding-bottom:{padding}}}}}</style><p>Body<span style=float:footnote>Note</span></p>"
        )
    };
    let footnotes = |html: &str| -> Vec<String> {
        let trace = render_html(html, &lossy_options())
            .unwrap_or_else(|error| panic!("{html} must render: {error}"))
            .layout
            .expect("layout requested");
        visible_frame_columns(&trace, 0)
            .into_iter()
            .flat_map(|(_, texts)| texts)
            .collect()
    };
    assert_eq!(
        footnotes(&html("1px")),
        ["1. Note"],
        "a footnote area of finite geometry holds the footnote"
    );
    let omitted = html("3e38px");
    let (texts, reasons) = texts_and_omitted_boxes(&omitted);
    assert_eq!(
        reasons,
        ["non-finite footnote area geometry"],
        "the area whose padding overflows its extent is reported omitted"
    );
    assert_eq!(texts, ["Body1"], "the body and its footnote call render");
    assert!(
        footnotes(&omitted).is_empty(),
        "the footnote is omitted with its area"
    );
}
