mod common;
use common::{PROFILE, local_options};
use ferri_pdf::render_html;

fn render(css: &str, body: &str) -> ferri_pdf::RenderOutput {
    render_html(&format!("<style>{PROFILE}@page{{size:500pt 300pt;margin:36pt}}p,h2{{margin:0;font:12pt/18pt Spike}}h2{{string-set:heading content();break-after:auto}}.next{{break-before:page}}{css}</style>{body}"), &local_options())
        .expect("body string references must resolve before final shaping")
}

#[test]
fn body_strings_select_all_modes_on_the_page_of_each_use() {
    let output = render(
        ".reference::after{content:'F[' string(heading,first) ']S[' string(heading,start) ']L[' string(heading,last) ']X[' string(heading,first-except) ']'}",
        "<p class=reference></p><h2>Alpha</h2><h2>Beta</h2><p class='reference next'></p><h2 class=next>Gamma</h2><p class=reference></p>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 3);
    for (page, expected) in [
        "F[Alpha]S[]L[Beta]X[]",
        "F[Beta]S[Beta]L[Beta]X[Beta]",
        "F[Gamma]S[Gamma]L[Gamma]X[]",
    ]
    .iter()
    .enumerate()
    {
        assert!(
            trace.pages[page].iter().any(|line| line.text == *expected),
            "page {} must resolve its own assignments: {:?}",
            page + 1,
            trace.pages[page]
                .iter()
                .map(|line| &line.text)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn after_pseudo_uses_its_own_page_after_the_originating_element_fragments() {
    let output = render(
        "section::before{content:'Before:' string(heading)}section::after{content:'After:' string(heading)}",
        "<section><h2>Alpha</h2><h2 class=next>Beta</h2></section>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 2);
    assert!(
        trace.pages[0]
            .iter()
            .any(|line| line.text == "Before:Alpha")
    );
    assert!(
        trace.pages[1].iter().any(|line| line.text == "After:Beta"),
        "after content must not use the originating section's first page"
    );
}

#[test]
fn initially_empty_string_content_retains_a_location_for_later_resolution() {
    let output = render(
        ".reference::after{content:string(heading)}",
        "<p>Opening</p><p class=reference></p><h2>Later title</h2>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 1);
    assert_eq!(
        trace.pages[0]
            .iter()
            .filter(|line| line.text == "Later title")
            .count(),
        2,
        "empty provisional generated content must become both reference and heading"
    );
}

#[test]
fn generated_box_kinds_locate_strings_on_their_actual_page() {
    for display in [
        "display:inline",
        "display:block",
        "display:inline-block",
        "float:left",
        "position:absolute",
    ] {
        let output = render(
            &format!(
                ".next{{position:relative}}.reference::after{{{display};content:'Reference:' string(heading)}}"
            ),
            "<h2>Alpha</h2><section class=next><h2>Beta</h2><p class=reference></p></section>",
        );
        let trace = output.layout.unwrap();
        assert!(
            trace.pages[1]
                .iter()
                .any(|line| line.text == "Reference:Beta"),
            "{display} must locate its use on page 2"
        );
        assert!(
            !trace.pages[0]
                .iter()
                .any(|line| line.text.starts_with("Reference:"))
        );
    }
}

#[test]
fn absent_named_string_in_an_empty_document_is_an_empty_value() {
    let output = render("p::before{content:string(absent)}", "<p></p>");
    assert_eq!(output.page_count, 1);
    assert!(output.layout.unwrap().pages[0].is_empty());
}

#[test]
fn named_string_bookmark_labels_resolve_on_the_heading_page() {
    let resolver = ferri_pdf_resources::FileResolver::default();
    let html = format!(
        "<style>{PROFILE}h2{{string-set:heading content();bookmark-label:'Chapter ' string(heading)}}h2+h2{{break-before:page}}</style><h2>Alpha</h2><h2>Beta</h2>"
    );
    let document =
        ferri_pdf_document::StyledDocument::parse(&html, &local_options().base, &[], &resolver)
            .unwrap();
    let (document, trace) = ferri_pdf_layout::layout_document(document, &resolver).unwrap();
    assert_eq!(trace.pages.len(), 2);
    let labels: Vec<_> = document
        .navigation()
        .bookmarks
        .iter()
        .map(|bookmark| bookmark.label.as_str())
        .collect();
    assert_eq!(labels, ["Chapter Alpha", "Chapter Beta"]);
}

#[test]
fn string_expansion_and_target_pages_stabilize_together() {
    let heading = "A deliberately long chapter heading expands the reference over several lines";
    let output = render(
        "@page{size:360pt 160pt;margin:20pt}.reference{width:80pt}.reference::after{content:string(heading) ' (page ' target-counter('#target',page) ')'}",
        &format!(
            "<h2>{heading}</h2><p class=reference></p><h2 id=target class=next>Destination</h2>"
        ),
    );
    let page = output
        .headings
        .iter()
        .find(|heading| heading.id.as_deref() == Some("target"))
        .unwrap()
        .page;
    assert!(
        page > 2,
        "expanded reference must change the destination page"
    );
    let trace = output.layout.unwrap();
    let text = trace.pages[..page - 1]
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.contains(&format!("(page {page})")),
        "final target page missing from {text:?}"
    );
}

#[test]
fn body_string_reflow_cycles_fail_without_publishing_a_provisional_pdf() {
    let resolver = ferri_pdf_resources::FileResolver::default();
    let html = format!(
        "<style>{PROFILE}@page{{size:360pt 120pt;margin:20pt}}p,h2{{margin:0;font:12pt/18pt Spike}}h2{{string-set:heading content();break-after:auto}}p{{width:60pt}}p::after{{content:string(heading)}}</style><p>See </p><h2>A long destination heading that cannot fit in the reference width</h2>"
    );
    let document =
        ferri_pdf_document::StyledDocument::parse(&html, &local_options().base, &[], &resolver)
            .unwrap();
    assert!(
        matches!(
            ferri_pdf_layout::layout(&document, &resolver),
            Err(ferri_pdf_layout::LayoutError::GeneratedContentCycle)
        ),
        "moving the only assignment off the reference page must not freeze a guessed value"
    );
}

#[test]
fn borrowed_layout_repeats_body_string_resolution_without_mutating_the_source() {
    let resolver = ferri_pdf_resources::FileResolver::default();
    let html = format!(
        "<style>{PROFILE}h2{{string-set:heading content()}}p::after{{content:string(heading)}}</style><p>See </p><h2>Title</h2>"
    );
    let document =
        ferri_pdf_document::StyledDocument::parse(&html, &local_options().base, &[], &resolver)
            .unwrap();
    for iteration in 0..2 {
        let trace = ferri_pdf_layout::layout(&document, &resolver).unwrap();
        assert!(
            trace.pages[0].iter().any(|line| line.text == "See Title"),
            "borrowed pass {iteration}"
        );
    }
}

#[test]
fn transformed_body_strings_use_the_shared_inline_text_pipeline() {
    let output = render(
        ".reference::after{content:string(heading);text-transform:uppercase}",
        "<p class=reference></p><h2>A mixed Case title</h2>",
    );
    assert!(
        output.layout.unwrap().pages[0]
            .iter()
            .any(|line| line.text == "A MIXED CASE TITLE")
    );
}

#[test]
fn float_deferred_past_its_source_page_uses_the_destination_page_strings() {
    let output = render(
        "@page{size:360pt 120pt;margin:20pt}.filler{height:40pt}.reference::after{float:left;width:100pt;height:48pt;break-inside:avoid;content:'Float:' string(heading)}",
        "<h2>Alpha</h2><div class=filler></div><p class=reference></p><h2>Beta</h2>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 2);
    assert!(
        trace.pages[0].iter().any(|line| line.text == "Beta"),
        "the following heading fits the source page"
    );
    assert!(
        trace.pages[1].iter().any(|line| line.text == "Float:Beta"),
        "the deferred float uses the next page's carried string: {:?}",
        trace
            .pages
            .iter()
            .map(|page| page.iter().map(|line| &line.text).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
}

#[test]
fn columns_resolve_body_strings_against_the_physical_page() {
    let output = render(
        "section{column-count:2;column-fill:auto;height:60pt}.second{break-before:column}.reference::after{content:'Reference:' string(heading)}",
        "<section><h2>Alpha</h2><h2 class=second>Beta</h2><p class=reference></p></section>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 1);
    assert!(
        trace.pages[0]
            .iter()
            .any(|line| line.text == "Reference:Alpha"),
        "a second column must not be mistaken for a second physical page"
    );
}

#[test]
fn replicated_fixed_content_keeps_its_first_generated_box_value() {
    let output = render(
        ".fixed{position:fixed;top:100pt;left:0}.fixed::after{content:'Fixed:' string(heading)}",
        "<h2>Alpha</h2><h2 class=next>Beta</h2><div class=fixed></div>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 2);
    for (page, lines) in trace.pages.iter().enumerate() {
        assert!(
            lines.iter().any(|line| line.text == "Fixed:Alpha"),
            "replica on page {} must retain the first generated box value: {:?}",
            page + 1,
            lines.iter().map(|line| &line.text).collect::<Vec<_>>()
        );
    }
}
