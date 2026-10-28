use super::common;
use crate::render_html;
use common::{PROFILE, local_options};

fn render(css: &str, body: &str) -> crate::RenderOutput {
    render_html(
        &format!(
            "<style>{PROFILE}@page{{size:360pt 240pt;margin:24pt}}p,h2{{margin:0;font:12pt/18pt Spike}}h2{{break-after:auto}}.next{{break-before:page}}{css}</style>{body}"
        ),
        &local_options(),
    )
    .expect("target expressions must resolve before final shaping and PDF output")
}

#[test]
fn forward_targets_capture_heading_counter_text_and_committed_page() {
    let output = render(
        "body{counter-reset:chapter 4}h2{counter-increment:chapter 2}a::after{content:' ' target-counter(attr(href),chapter,upper-roman) ': ' target-text(attr(href)) ' (page ' target-counter(attr(href),page) ')'}",
        "<p><a href='#launch'>See</a></p><h2 id=launch class=next>Launch</h2><h2 class=next>Later</h2>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages.len(),
        3,
        "both authored breaks must survive target resolution"
    );
    let text: Vec<_> = trace.pages[0]
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        text,
        ["See VI: Launch (page 2)"],
        "the target's counter must be captured before the later heading increments it"
    );
    let heading = output
        .headings
        .iter()
        .find(|heading| heading.id.as_deref() == Some("launch"))
        .unwrap();
    assert_eq!(
        heading.page, 2,
        "generated page references must agree with heading destinations"
    );
}

#[test]
fn target_text_uses_shared_pseudo_content_and_whitespace_extraction() {
    let output = render(
        "h2::before{content:'  Before  '}h2::after{content:'  After  '}a::after{content:'[' target-text('#launch',before) '|' target-text('#launch') '|' target-text('#launch',after) '|' target-text('#launch',first-letter) ']'}",
        "<p><a href='#launch'>See</a></p><h2 id=launch class=next>  Launch   window </h2>",
    );
    let trace = output.layout.unwrap();
    let text: Vec<_> = trace.pages[0]
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        text,
        ["See[Before|Launch window|After|B]"],
        "target extraction must share content() whitespace and first-letter semantics"
    );
}

#[test]
fn target_counters_preserve_nested_instances_and_counter_styles() {
    let output = render(
        "body{counter-reset:section 2}section{counter-reset:section 4}a::after{content:'[' target-counters('#target',section,'.',upper-roman) ']'}",
        "<p><a href='#target'>See</a></p><section class=next><h2 id=target>Nested</h2></section>",
    );
    let trace = output.layout.unwrap();
    let text: Vec<_> = trace.pages[0]
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(
        text,
        ["See[II.IV]"],
        "nested target counters must come from the shared counter scope engine"
    );
}

#[test]
fn borrowed_layout_resolves_literal_targets_without_mutating_the_source_document() {
    let options = local_options();
    let resolver = ferri_pdf_resources::FileResolver::default();
    let html = format!(
        "<style>{PROFILE}.next{{break-before:page}}.reference::after{{content:target-counter('#target',page)}}</style><p class=reference>Page </p><p id=target class=next>Destination</p>"
    );
    let document =
        ferri_pdf_document::StyledDocument::parse(&html, &options.base, &[], &resolver).unwrap();
    for iteration in 0..2 {
        let trace = ferri_pdf_layout::layout(&document, &resolver).unwrap();
        assert_eq!(trace.pages.len(), 2, "borrowed layout pass {iteration}");
        assert_eq!(
            trace.pages[0][0].text, "Page 2",
            "literal targets must reserve destinations even without an HTML link"
        );
    }
}

#[test]
fn forward_reference_reflows_before_resolving_its_final_target_page() {
    let body = "<p class=reference>See</p><h2 id=target class=next>The deliberately long destination heading changes how many lines precede this target</h2>";
    let css = "@page{size:180pt 140pt;margin:20pt}.reference{width:70pt}";
    let control = render(&format!("{css}.reference::after{{content:' short'}}"), body);
    let output = render(
        &format!(
            "{css}.reference::after{{content:' ' target-text('#target') ' (page ' target-counter('#target',page) ')'}}"
        ),
        body,
    );
    let page = output
        .headings
        .iter()
        .find(|heading| heading.id.as_deref() == Some("target"))
        .unwrap()
        .page;
    assert!(
        page > control.headings[0].page,
        "expanded target text must change pagination before the reference is finalized"
    );
    let trace = output.layout.unwrap();
    let preceding = trace.pages[..page - 1]
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        preceding.contains(&format!("(page {page})")),
        "reference must use final destination page {page}, got {preceding:?}"
    );
}

#[test]
fn repeated_target_layout_reuses_loaded_images() {
    use crate::{FileResolver, Resource, ResourceBase, ResourceError, ResourceResolver};
    use std::cell::Cell;
    struct CountingResolver {
        images: Cell<usize>,
        files: FileResolver,
    }
    impl ResourceResolver for CountingResolver {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            if reference.ends_with("image-pattern.png") {
                self.images.set(self.images.get() + 1);
            }
            self.files.resolve(reference, base)
        }
    }
    let resolver = CountingResolver {
        images: Cell::new(0),
        files: FileResolver::default(),
    };
    let html = format!(
        "<style>{PROFILE}img{{width:20pt;height:20pt}}h2{{break-before:page}}p::after{{content:target-counter('#target',page)}}</style><img src='../assets/image-pattern.png'><p>Page </p><h2 id=target>Destination</h2>"
    );
    let output = crate::render_with_resolver(&html, &local_options(), &resolver).unwrap();
    assert!(
        output.layout.unwrap().pages[0]
            .iter()
            .any(|line| line.text == "Page 2")
    );
    assert_eq!(
        resolver.images.get(),
        1,
        "layout stabilization must not reload the same image on each formatting pass"
    );
}

#[test]
fn late_named_page_margins_resolve_targets_not_seen_on_the_first_page() {
    let output = render(
        "@page chapter{@top-left{content:'Target ' target-counter('#target',page) ': ' target-text('#target')}}.next{page:chapter}",
        "<p>Opening</p><h2 id=target class=next>Destination</h2>",
    );
    let trace = output.layout.unwrap();
    assert_eq!(trace.pages.len(), 2);
    assert!(
        !trace.pages[0]
            .iter()
            .any(|line| line.text.starts_with("Target "))
    );
    assert!(
        trace.pages[1]
            .iter()
            .any(|line| line.text == "Target 2: Destination"),
        "page-only target expressions must join the same resolution pass as document content"
    );
}

#[test]
fn borrowed_layout_resolves_late_margin_targets_on_repeated_calls() {
    let resolver = ferri_pdf_resources::FileResolver::default();
    let html = format!(
        "<style>{PROFILE}@page chapter{{@top-left{{content:target-text('#target') ' ' target-counter('#target',page)}}}}h2{{page:chapter;break-before:page}}</style><p>Opening</p><h2 id=target>Destination</h2>"
    );
    let document =
        ferri_pdf_document::StyledDocument::parse(&html, &local_options().base, &[], &resolver)
            .unwrap();
    for iteration in 0..2 {
        let trace = ferri_pdf_layout::layout(&document, &resolver).unwrap();
        assert_eq!(trace.pages.len(), 2);
        assert!(
            trace.pages[1]
                .iter()
                .any(|line| line.text == "Destination 2"),
            "late margin reference was lost in borrowed layout {iteration}"
        );
    }
}

#[test]
fn cyclic_target_text_fails_without_publishing_a_provisional_layout() {
    let resolver = ferri_pdf_resources::FileResolver::default();
    let html = format!(
        "<style>{PROFILE}#a::before{{content:target-text('#b',first-letter)}}#b::before{{content:target-text('#a',first-letter)}}</style><p id=a>A</p><p id=b>B</p>"
    );
    let document =
        ferri_pdf_document::StyledDocument::parse(&html, &local_options().base, &[], &resolver)
            .unwrap();
    assert!(
        matches!(
            ferri_pdf_layout::layout(&document, &resolver),
            Err(ferri_pdf_layout::LayoutError::GeneratedContentCycle)
        ),
        "mutually swapping first letters must fail instead of returning an arbitrary pass"
    );
}

#[test]
fn hidden_target_text_and_counters_use_the_shared_document_state() {
    let output = render(
        "body{counter-reset:chapter 4}#target{display:none;counter-increment:chapter}p::after{content:target-text('#target') ' ' target-counter('#target',chapter)}",
        "<p>See </p><h2 id=target>Hidden title</h2>",
    );
    assert_eq!(
        output.layout.unwrap().pages[0][0].text,
        "See Hidden title 4"
    );
}

#[test]
fn target_references_in_string_assignments_reach_page_margins() {
    let output = render(
        "p{string-set:reference content(before)}p::before{content:target-text('#target')}@page{@top-left{content:string(reference)}}",
        "<p>Opening</p><h2 id=target class=next>Destination</h2>",
    );
    assert!(
        output.layout.unwrap().pages[0]
            .iter()
            .any(|line| line.text == "Destination")
    );
}

#[test]
fn target_reference_in_an_explicit_bookmark_label_resolves() {
    let resolver = ferri_pdf_resources::FileResolver::default();
    let html = format!(
        "<style>{PROFILE}h2{{bookmark-label:target-text('#target') ' page ' target-counter('#target',page)}}.next{{break-before:page}}</style><h2>Opening</h2><p id=target class=next>Destination</p>"
    );
    let document =
        ferri_pdf_document::StyledDocument::parse(&html, &local_options().base, &[], &resolver)
            .unwrap();
    let (document, trace) = ferri_pdf_layout::layout_document(document, &resolver).unwrap();
    assert_eq!(trace.pages.len(), 2);
    assert_eq!(
        document.navigation().bookmarks[0].label,
        "Destination page 2"
    );
}

#[test]
fn target_reference_errors_retain_the_reason_without_loading_external_documents() {
    use ferri_pdf_style::{StyleError, TargetReferenceError};
    for (reference, expected) in [
        ("#absent", TargetReferenceError::MissingId),
        ("#", TargetReferenceError::MissingFragment),
        (
            "https://example.invalid/external#target",
            TargetReferenceError::ExternalDocument,
        ),
    ] {
        let html = format!(
            "<style>p::after{{content:target-text('{reference}')}}</style><p>Reference</p>"
        );
        match render_html(&html, &local_options()) {
            Err(crate::Error::Css(StyleError::TargetReference { reason, .. })) => assert_eq!(
                reason, expected,
                "wrong failure classification for {reference}"
            ),
            Err(error) => panic!("unexpected failure for {reference}: {error}"),
            Ok(_) => panic!("invalid target {reference} produced a PDF"),
        }
    }
}

#[test]
fn page_targets_use_multi_digit_values_and_final_page_totals() {
    const TARGET_PAGE: usize = 10;
    let mut body = String::from("<p class=reference>See </p>");
    for _ in 1..TARGET_PAGE - 1 {
        body.push_str("<p class=next>Intervening page</p>");
    }
    body.push_str("<h2 id=target class=next>Destination</h2><p class=next>Last page</p>");
    let output = render(
        ".reference::after{content:target-counter('#target',page) '/' target-counter('#target',pages)}",
        &body,
    );
    assert_eq!(output.page_count, TARGET_PAGE + 1);
    assert_eq!(output.headings[0].page, TARGET_PAGE);
    assert_eq!(output.layout.unwrap().pages[0][0].text, "See 10/11");
}
