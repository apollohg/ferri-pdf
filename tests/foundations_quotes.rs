mod common;

use common::{PROFILE, local_options};
use ferri_pdf::{LayoutTrace, render_html};

fn trace(css: &str, markup: &str) -> LayoutTrace {
    render_html(
        &format!("<!doctype html><html lang=en><style>{PROFILE}p{{margin:0}}{css}</style>{markup}"),
        &local_options(),
    )
    .expect("quotation content must render through the common cascade")
    .layout
    .unwrap()
}

fn lines(trace: &LayoutTrace) -> Vec<&str> {
    trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect()
}

#[test]
fn html_quotes_use_generated_content_and_repeat_the_last_pair() {
    let output = trace(
        "",
        "<p><q>One <q>two <q>three</q></q></q></p><p><q>Four</q></p>",
    );
    assert_eq!(lines(&output), ["“One ‘two ‘three’’”", "“Four”"]);
    let output = trace(
        "body{--marks:'[[' ']]' '{' '}';quotes:var(--marks)}q.special::before{content:'override'}",
        "<p><q>One <q>two</q></q> <q class=special>Three</q></p>",
    );
    assert_eq!(lines(&output), ["[[One {two}]] overrideThree"]);
}

#[test]
fn phantom_quotes_and_none_change_depth_in_generated_tree_order() {
    let output = trace(
        "body{quotes:'[' ']' '{' '}'}\
         .open::before{content:no-open-quote}.close::after{content:no-close-quote}\
         .underflow::before{content:close-quote 'A' no-close-quote}\
         .hidden{display:none}.hidden::before{content:open-quote}\
         .silent{quotes:none}.silent q{quotes:'[' ']' '{' '}'}",
        "<p class=underflow><span class=open></span><q>B</q><span class=close></span>\
         <span class=hidden></span><q>C</q> <q class=silent>D<q>E</q></q><q>F</q></p>",
    );
    assert_eq!(lines(&output), ["A{B}[C] D{E}[F]"]);
}

#[test]
fn automatic_quotes_follow_locale_and_match_parent_keeps_its_language() {
    let output = trace(
        ".fixed::before,.fixed::after{quotes:match-parent}",
        "<p lang=de><q>A</q> <q lang=en>B</q> <q lang=en class=fixed>C</q></p>\
         <p lang=de-CH><q>D</q></p><p lang=fr><q>E</q></p><p lang=en-GB><q>F</q></p><p lang=fr-CH><q>G <q>H</q></q></p>",
    );
    assert_eq!(
        lines(&output),
        ["„A“ “B” „C“", "„D“", "«E»", "“F”", "«G ‹H›»"]
    );
}

#[test]
fn quotes_initial_inherit_and_page_margins_share_defaulting() {
    let output = trace(
        "body{quotes:'[' ']'}q.reset{quotes:initial}\
         q.inherit{quotes:inherit}@page{@top-center{quotes:'<' '>';content:open-quote counter(page) close-quote}}",
        "<p><q>One</q> <q class=reset>Two</q> <q class=inherit>Three</q></p>\
         <p class=new-page><q>Four</q></p>",
    );
    for (index, page) in output.pages.iter().enumerate() {
        assert!(
            page.iter()
                .any(|line| line.text == format!("<{}>", index + 1))
        );
    }
    assert!(lines(&output).contains(&"[One] “Two” [Three]"));
    assert!(lines(&output).contains(&"[Four]"));
}

#[test]
fn bookmark_content_uses_rendered_quote_depth_without_changing_it() {
    let markup = "<p><q>A<q style='bookmark-level:1;bookmark-label:content(before) \"|\" content(after)'>B</q></q></p><p><q>C</q></p>";
    let css = "body{quotes:'[' ']' '{' '}'}";
    let html = format!("<!doctype html><style>{PROFILE}{css}</style>{markup}");
    let document = ferri_pdf_document::StyledDocument::parse(
        &html,
        &local_options().base,
        &[],
        &ferri_pdf_resources::FileResolver::default(),
    )
    .unwrap();
    assert_eq!(document.navigation().bookmarks[0].label, "{|}");
    assert_eq!(lines(&trace(css, markup)), ["[A{B}]", "[C]"]);
}

#[test]
fn automatic_quotes_keep_region_when_a_script_or_extension_is_present() {
    let output = trace(
        "",
        "<p lang=fr-Latn-CH><q>A <q>B</q></q></p><p lang=FR-ch-u-nu-latn><q>C <q>D</q></q></p>",
    );
    assert_eq!(lines(&output), ["«A ‹B›»", "«C ‹D›»"]);
}
