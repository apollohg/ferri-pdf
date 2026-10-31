use super::common;
use crate::render_html;
use common::{PROFILE, local_options};

const HEADER_PREFIX: &str = "F[";
const STRING_PAGE_CSS: &str = "@page{size:1000pt 400pt;margin:40pt;@top-left{content:'F[' string(heading,first) ']S[' string(heading,start) ']L[' string(heading,last) ']X[' string(heading,first-except) ']'}}h2{font:12pt Spike;margin:0;break-after:auto}p{margin:0}.next{break-before:page}";

fn headers(css: &str, body: &str) -> Vec<String> {
    let html = format!("<style>{PROFILE}{STRING_PAGE_CSS}{css}</style>{body}");
    let trace = render_html(&html, &local_options())
        .expect("running strings must resolve through document assignment and committed pagination")
        .layout
        .unwrap();
    trace
        .pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            page.iter()
                .find(|line| line.text.starts_with(HEADER_PREFIX))
                .unwrap_or_else(|| {
                    panic!(
                        "page {} has no running header: {:?}",
                        index + 1,
                        page.iter()
                            .map(|line| line.text.as_str())
                            .collect::<Vec<_>>()
                    )
                })
                .text
                .clone()
        })
        .collect()
}

#[test]
fn running_string_modes_use_page_assignments_and_entry_values() {
    assert_eq!(
        headers(
            "h2{string-set:heading content(text)}.empty{string-set:heading ''}",
            "<p>Intro</p><h2>Alpha</h2><h2>Beta</h2><p class=next>Continuation</p><h2 class=next>Gamma</h2><p class=next>Before empty</p><h2 class=empty></h2><p class=next>After empty</p>"
        ),
        [
            "F[Alpha]S[]L[Beta]X[]",
            "F[Beta]S[Beta]L[Beta]X[Beta]",
            "F[Gamma]S[Gamma]L[Gamma]X[]",
            "F[]S[Gamma]L[]X[]",
            "F[]S[]L[]X[]"
        ]
    );
}

#[test]
fn hidden_head_metadata_assignments_are_available_on_the_first_page() {
    assert_eq!(
        headers(
            "title{string-set:heading content(text)}meta[name=author]{string-set:heading attr(content)}@page{@top-left{content:'F[' string(heading,first) ']L[' string(heading,last) ']X[' string(heading,first-except) ']'}}",
            "<title>Manual</title><meta name=author content=Alex><p>Body</p><p class=next>Continuation</p>"
        ),
        ["F[Manual]L[Alex]X[]", "F[Alex]L[Alex]X[Alex]"]
    );
}

#[test]
fn boxless_elements_assign_strings_but_pseudo_elements_do_not() {
    assert_eq!(
        headers(
            "section{display:contents;string-set:heading attr(data-heading)}section::before{content:'Prefix';string-set:heading 'Wrong'}",
            "<section data-heading=Visible><p>Body</p></section><p class=next>Continuation</p>"
        ),
        [
            "F[Visible]S[Visible]L[Visible]X[]",
            "F[Visible]S[Visible]L[Visible]X[Visible]"
        ]
    );
}

#[test]
fn running_strings_capture_counters_and_generated_element_content_at_assignment() {
    assert_eq!(
        headers(
            "body{counter-reset:chapter}h2{counter-increment:chapter;string-set:heading content(before) content(text) content(after)}h2::before{content:counter(chapter,upper-roman) ':'}h2::after{content:'!'}",
            "<h2>Alpha</h2><h2>Beta</h2><p class=next>Continuation</p>"
        ),
        [
            "F[I:Alpha!]S[I:Alpha!]L[II:Beta!]X[]",
            "F[II:Beta!]S[II:Beta!]L[II:Beta!]X[II:Beta!]"
        ]
    );
}

fn captured(css: &str, body: &str) -> Vec<Vec<(String, String)>> {
    let document = crate::document::StyledDocument::parse(
        &format!("<style>{PROFILE}{css}</style>{body}"),
        &local_options().base,
        &[],
        &crate::resources::FileResolver::default(),
    )
    .expect("string assignments must be captured without laying out page headers");
    document
        .running_strings()
        .iter()
        .map(|owner| {
            owner
                .values
                .iter()
                .map(|value| {
                    (
                        value.name.to_string(),
                        value
                            .content
                            .bound_page_output(2, 7, crate::document::MAX_INLINE_TEXT_BYTES)
                            .expect("captured strings retain only page-dependent expressions")
                            .into_text(),
                    )
                })
                .collect()
        })
        .collect()
}

#[test]
fn assignments_capture_content_counters_empty_values_and_boxless_owners() {
    assert_eq!(
        captured(
            "body{counter-reset:n}section{display:contents;string-set:heading 'Group'}h2{counter-increment:n;string-set:heading content(before) content(text) content(after), folio counter(n) '/' counter(page) '/' counter(pages)}h2::before{content:counter(n) ':';string-set:heading 'Wrong'}h2::after{content:'!'}span{string-set:heading ''}",
            "<section><h2>Alpha</h2><h2>Beta</h2><span></span></section>"
        ),
        vec![
            vec![("heading".into(), "Group".into())],
            vec![
                ("heading".into(), "1:Alpha!".into()),
                ("folio".into(), "1/2/7".into())
            ],
            vec![
                ("heading".into(), "2:Beta!".into()),
                ("folio".into(), "2/2/7".into())
            ],
            vec![("heading".into(), String::new())],
        ]
    );
}

#[test]
fn hidden_assignments_use_lazy_values_without_loading_hidden_resources() {
    assert_eq!(
        captured(
            "title{string-set:heading content()}meta[name=author]{string-set:author attr(content)}.hidden{display:none;counter-reset:n 99}.hidden img{string-set:heading attr(alt);width:999unknown}.hidden span{string-set:heading counter(n) content(text)}body{counter-reset:n 3}",
            "<title>Manual</title><meta name=author content=Alex><p>Before</p><div class=hidden><img src=missing.png alt=Portrait><span> Hidden </span></div><p>After</p>"
        ),
        vec![
            vec![("heading".into(), "Manual".into())],
            vec![("author".into(), "Alex".into())],
            vec![("heading".into(), "Portrait".into())],
            vec![("heading".into(), "3Hidden".into())],
        ]
    );
}

#[test]
fn line_break_and_hidden_root_assignments_are_not_dropped() {
    assert_eq!(
        captured(
            "br{string-set:heading content(before) 'Break' content(after)}br::before{content:'['}br::after{content:']'}",
            "<p>Before<br>After</p>"
        ),
        vec![vec![("heading".into(), "[Break]".into())]]
    );
    assert_eq!(
        captured(
            "html{display:none;string-set:heading 'Root'}title{string-set:heading content()}span{string-set:heading content()}",
            "<title>Manual</title><span>Hidden body</span>"
        ),
        vec![
            vec![("heading".into(), "Root".into())],
            vec![("heading".into(), "Manual".into())],
            vec![("heading".into(), "Hidden body".into())],
        ]
    );
}

#[test]
fn fragmented_assignments_keep_their_original_page_counter_and_do_not_repeat() {
    assert_eq!(
        headers(
            "section{string-set:heading 'Page ' counter(page) '/' counter(pages)}",
            "<section><p>First</p><p class=next>Second</p><p class=next>Third</p></section>"
        ),
        [
            "F[Page 1/3]S[Page 1/3]L[Page 1/3]X[]",
            "F[Page 1/3]S[Page 1/3]L[Page 1/3]X[Page 1/3]",
            "F[Page 1/3]S[Page 1/3]L[Page 1/3]X[Page 1/3]",
        ]
    );
}

#[test]
fn hidden_assignment_before_a_forced_break_stays_on_the_preceding_page() {
    assert_eq!(
        headers(
            ".hidden{display:none;string-set:heading 'Metadata'}h2{string-set:heading content()}",
            "<p>First</p><span class=hidden></span><h2 class=next>Second</h2>"
        ),
        [
            "F[Metadata]S[]L[Metadata]X[]",
            "F[Second]S[Second]L[Second]X[]"
        ]
    );
}

#[test]
fn empty_inline_assignments_follow_transformed_text_without_adding_a_line() {
    assert_eq!(
        headers(
            "p{text-transform:uppercase}span{string-set:heading 'Inline'}",
            "<p>ß <span></span>end</p><p class=next>After</p>"
        ),
        [
            "F[Inline]S[]L[Inline]X[]",
            "F[Inline]S[Inline]L[Inline]X[Inline]"
        ]
    );
}

#[test]
fn start_uses_the_first_assignment_on_the_first_element_path() {
    assert_eq!(
        headers(
            "section{string-set:heading 'Section'}h2{string-set:heading content()}",
            "<section><h2>Heading</h2><p>Body</p></section>"
        ),
        ["F[Section]S[Section]L[Heading]X[]"]
    );
}

#[test]
fn table_and_flex_structure_preserves_hidden_assignments_without_extra_items() {
    for container in ["display:table", "display:flex"] {
        let css = format!(
            ".container{{{container}}}.row{{display:table-row}}.cell{{display:table-cell;string-set:heading 'Cell'}}.hidden{{display:none;string-set:heading 'Hidden'}}@page{{@top-left{{content:'F[' string(heading,first) ']L[' string(heading,last) ']'}}}}"
        );
        let body = if container == "display:table" {
            "<div class=container><div class=row><span class=hidden></span><div class=cell>Body</div></div></div>"
        } else {
            "<div class=container><span class=hidden></span><div class=cell>Body</div></div>"
        };
        assert_eq!(
            headers(&css, body),
            ["F[Hidden]L[Cell]"],
            "semantic assignment order for {container}"
        );
    }
}

#[test]
fn neutral_assignments_between_fragmented_rows_and_flex_items_keep_their_boundary_page() {
    for (container, row, cell) in [
        ("display:table", "display:table-row", "display:table-cell"),
        (
            "display:flex;flex-direction:column;align-items:flex-start",
            "display:block",
            "display:block",
        ),
    ] {
        let css = format!(
            ".container{{{container}}}.row{{{row}}}.cell{{{cell}}}.hidden{{display:none;string-set:heading 'Between ' counter(page)}}.last{{string-set:heading 'Last ' counter(page)}}@page{{@top-left{{content:'F[' string(heading,first) ']L[' string(heading,last) ']'}}}}"
        );
        assert_eq!(
            headers(
                &css,
                "<div class=container><div class=row><div class=cell>First</div></div><span class=hidden></span><div class='row next'><div class=cell>Second</div></div><span class='hidden last'></span></div>"
            ),
            ["F[Between 1]L[Between 1]", "F[Last 2]L[Last 2]"],
            "boundary positions for {container}"
        );
    }
}

#[test]
fn hidden_root_still_supplies_page_margin_strings() {
    assert_eq!(
        headers(
            "html{display:none}title{string-set:heading content()}@page{@top-left{content:'F[' string(heading) ']'}}",
            "<title>Hidden document title</title><p>Hidden body</p>"
        ),
        ["F[Hidden document title]"]
    );
}

#[test]
fn table_columns_and_empty_groups_assign_without_grid_cells() {
    assert_eq!(
        headers(
            ".table{display:table}.columns{display:table-column-group;string-set:heading 'Columns'}.column{display:table-column;string-set:heading 'Column'}.empty{display:table-row-group;string-set:heading 'Empty'}.row{display:table-row}.cell{display:table-cell}@page{@top-left{content:'F[' string(heading,first) ']L[' string(heading,last) ']'}}",
            "<div class=table><div class=columns><div class=column></div></div><div class=empty></div><div class=row><div class=cell>Body</div></div></div>"
        ),
        ["F[Columns]L[Empty]"]
    );
}

#[test]
fn multicolumn_assignments_use_physical_pages_after_balancing() {
    assert_eq!(
        headers(
            ".columns{columns:2;column-gap:20pt}h2{string-set:heading content()}@page{@top-left{content:'F[' string(heading,first) ']L[' string(heading,last) ']'}}",
            "<div class=columns><h2>Alpha</h2><p>First column content</p><h2 class=next>Beta</h2><p>Next physical page</p></div>"
        ),
        ["F[Alpha]L[Alpha]", "F[Beta]L[Beta]"]
    );
}

#[test]
fn assignments_between_rows_in_one_kept_band_precede_internal_forced_breaks() {
    assert_eq!(
        headers(
            ".table{display:table}.group{display:table-row-group;break-inside:avoid;string-set:heading 'Group'}.row{display:table-row}.cell{display:table-cell}.hidden{display:none;string-set:heading 'Between ' counter(page)}.next{string-set:heading 'Second ' counter(page)}@page{@top-left{content:'F[' string(heading,first) ']L[' string(heading,last) ']'}}",
            "<div class=table><div class=group><div class=row><div class=cell>First</div></div><span class=hidden></span><div class='row next'><div class=cell>Second</div></div></div></div>"
        ),
        ["F[Group]L[Between 1]", "F[Second 2]L[Second 2]"]
    );
}

#[test]
fn repeated_table_headers_do_not_repeat_string_assignments() {
    assert_eq!(
        headers(
            "thead{string-set:heading 'Header ' counter(page)}table{width:100%}",
            "<table><thead><tr><td>Column</td></tr></thead><tbody><tr><td>First</td></tr><tr class=next><td>Second</td></tr><tr class=next><td>Third</td></tr></tbody></table>"
        ),
        [
            "F[Header 1]S[Header 1]L[Header 1]X[]",
            "F[Header 1]S[Header 1]L[Header 1]X[Header 1]",
            "F[Header 1]S[Header 1]L[Header 1]X[Header 1]",
        ]
    );
}

#[test]
fn a_float_moved_to_the_next_page_assigns_only_after_placement() {
    assert_eq!(
        headers(
            ".spacer{height:300pt}.floater{float:left;width:140pt;height:60pt;string-set:heading 'Float ' counter(page)}",
            "<p class=spacer>Before</p><h2 class=floater>Floating heading</h2><p>After</p>"
        ),
        ["F[]S[]L[]X[]", "F[Float 2]S[Float 2]L[Float 2]X[]"]
    );
}

#[test]
fn assignments_after_table_captions_keep_their_page_boundary() {
    assert_eq!(
        headers(
            ".table{display:table}.caption{display:table-caption}.row{display:table-row;break-before:page}.cell{display:table-cell}.hidden{display:none;string-set:heading 'Caption' counter(page)}",
            "<div class=table><div class=caption>Caption</div><div class=hidden></div><div class=row><div class=cell>Body</div></div></div>"
        ),
        [
            "F[Caption1]S[]L[Caption1]X[]",
            "F[Caption1]S[Caption1]L[Caption1]X[Caption1]"
        ]
    );
}

#[test]
fn named_and_blank_pages_carry_strings_without_reassigning_them() {
    assert_eq!(
        headers(
            "h2{string-set:heading content() counter(page)}.next{page:chapter;break-before:right}@page chapter{size:900pt 400pt}",
            "<h2>Alpha</h2><h2 class=next>Beta</h2>"
        ),
        [
            "F[Alpha1]S[Alpha1]L[Alpha1]X[]",
            "F[Alpha1]S[Alpha1]L[Alpha1]X[Alpha1]",
            "F[Beta3]S[Beta3]L[Beta3]X[]",
        ]
    );
}

#[test]
fn first_letter_includes_in_flow_generated_text_and_preserves_punctuation_boundaries() {
    for (css, body, expected) in [
        ("h2::before{content:'Note: '}", "<h2>Launch</h2>", "N"),
        ("h2::before{content:'«'}", "<h2> Launch</h2>", "« L"),
        ("h2::after{content:'After'}", "<h2></h2>", "A"),
        (
            "h2::before{content:'Wrong';position:absolute}",
            "<h2>Launch</h2>",
            "L",
        ),
        (
            "h2::before{content:'Wrong';float:left}",
            "<h2>Launch</h2>",
            "L",
        ),
    ] {
        let css = format!("h2{{string-set:heading content(first-letter)}}{css}");
        let expected = format!("F[{expected}]S[{expected}]L[{expected}]X[]");
        assert_eq!(
            headers(&css, body),
            [expected],
            "first-letter source for {css}"
        );
    }
}

#[test]
fn empty_table_group_breaks_keep_string_events_on_the_groups_page() {
    let css = ".table{display:table;width:100pt;border-spacing:0}.group{display:table-row-group}.row{display:table-row}.cell{display:table-cell;height:20pt;padding:0}.first{string-set:heading 'First'}.empty{string-set:heading 'Empty ' counter(page)}.inside{display:none;string-set:inner 'Inner ' counter(page)}.after{display:none;string-set:following 'After ' counter(page)}@page{@top-left{content:'F[' string(heading,first) ']L[' string(heading,last) ']I[' string(inner,last) ']A[' string(following,last) ']'}}";
    let body = "<div class=table><div class=group><div class=row><div class='cell first'>A</div></div></div><div class='group empty'><span class=inside></span></div><span class=after></span><div class=group><div class=row><div class=cell>B</div></div></div></div>";
    for anchor in ["", "anchor-name:--empty;"] {
        for (edge, expected) in [
            (
                "before",
                [
                    "F[First]L[First]I[]A[]",
                    "F[Empty 2]L[Empty 2]I[Inner 2]A[After 2]",
                ],
            ),
            (
                "after",
                [
                    "F[First]L[Empty 1]I[Inner 1]A[After 1]",
                    "F[Empty 1]L[Empty 1]I[Inner 1]A[After 1]",
                ],
            ),
        ] {
            assert_eq!(
                headers(&format!("{css}.empty{{{anchor}break-{edge}:page}}"), body),
                expected,
                "empty group break-{edge}, anchor property {anchor:?}"
            );
        }
    }
}

const MEBIBYTE: usize = 1024 * 1024;
/// Copies of a mebibyte that pass the generated text limit.
const PAST_TEXT_LIMIT: usize = crate::document::MAX_INLINE_TEXT_BYTES / MEBIBYTE + 1;
const LIMIT_CSS: &str = "@page{size:2000pt 200pt;margin:40pt;@top-center{white-space:nowrap;font:8pt Spike}}body{font:8pt Spike}";

fn reports_text_limit(diagnostics: &[crate::RenderDiagnostic]) -> bool {
    diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic,
            crate::RenderDiagnostic::Degraded(crate::style::Degradation::GeneratedTextLimit { limit })
                if *limit == crate::document::MAX_INLINE_TEXT_BYTES
        )
    })
}

/// The text of the first page's line starting with `prefix`, rendering `css` and `body`.
fn limited_render(css: &str, body: &str, prefix: &str) -> String {
    let html = format!("<style>{PROFILE}{LIMIT_CSS}{css}</style>{body}");
    let output = render_html(&html, &local_options())
        .expect("generated text past its limit must be cut, not fail the render");
    assert!(
        reports_text_limit(&output.diagnostics),
        "the cut generated text is reported: {:?}",
        output.diagnostics
    );
    let pages = output.layout.expect("layout requested").pages;
    pages[0]
        .iter()
        .find(|line| line.text.starts_with(prefix))
        .unwrap_or_else(|| {
            panic!(
                "no line starts with {prefix:?}: {:?}",
                pages[0].iter().map(|line| &line.text).collect::<Vec<_>>()
            )
        })
        .text
        .clone()
}

#[test]
fn counters_joined_past_the_text_limit_are_cut_in_running_strings_instead_of_failing() {
    // Each nested `div` resets its own `c`, so `counters()` joins one value per level with
    // mebibyte separators: past the limit after the 65th level.
    let separator = " ".repeat(MEBIBYTE);
    let header = limited_render(
        &format!(
            "div{{counter-reset:c}}#last{{string-set:t counters(c,'{separator}')}}@page{{@top-center{{content:'A' string(t) 'Z'}}}}"
        ),
        &format!(
            "{}<p id=last>Body</p>{}",
            "<div>".repeat(PAST_TEXT_LIMIT),
            "</div>".repeat(PAST_TEXT_LIMIT)
        ),
        "A",
    );
    // The limit keeps 64 values and their separators, which collapse to spaces; the header's
    // own prefix pushes its last separator's final space, and the `Z` after it, out.
    let kept = vec!["0"; PAST_TEXT_LIMIT - 1].join(" ");
    assert_eq!(header, format!("A{kept}"), "the cut header text");
}

#[test]
fn margin_boxes_repeating_strings_past_the_text_limit_cut_their_content_instead_of_failing() {
    // Each value fills a mebibyte, so the header passes the limit inside its last copy.
    let value = format!("{}x", " ".repeat(MEBIBYTE - 1));
    let header = limited_render(
        &format!(
            "p{{string-set:t '{value}'}}@page{{@top-center{{content:'A' {} 'Z'}}}}",
            "string(t) ".repeat(PAST_TEXT_LIMIT - 1)
        ),
        "<p>Body</p>",
        "A",
    );
    let kept = vec!["x"; PAST_TEXT_LIMIT - 2].join(" ");
    assert_eq!(
        header,
        format!("A {kept}"),
        "the copy the limit cuts loses its `x`, and the `Z` after it is left out"
    );
}

#[test]
fn element_text_repeated_past_the_text_limit_is_cut_in_running_strings_and_bookmarks() {
    let css = format!(
        "h1{{content-visibility:hidden;string-set:t {sources};bookmark-label:{sources}}}",
        sources = "content(text) ".repeat(PAST_TEXT_LIMIT)
    );
    let body = format!("<h1>{}</h1><p>Body</p>", "x".repeat(MEBIBYTE));
    let html = format!("<style>{PROFILE}{LIMIT_CSS}{css}</style>{body}");
    let output = render_html(&html, &local_options())
        .expect("element text repeated past its limit must be cut, not fail the render");
    assert!(
        reports_text_limit(&output.diagnostics),
        "the cut running string and bookmark label are reported: {:?}",
        output.diagnostics
    );
    let strings = captured(&css, &body);
    let value = &strings[0][0].1;
    assert!(
        value.len() == crate::document::MAX_INLINE_TEXT_BYTES && value.bytes().all(|b| b == b'x'),
        "the running string keeps the text up to the limit: {} bytes",
        value.len()
    );
    let document = crate::document::StyledDocument::parse(
        &html,
        &local_options().base,
        &[],
        &crate::resources::FileResolver::default(),
    )
    .expect("bookmark labels past the limit are cut, not failed");
    let label = &document.navigation().bookmarks[0].label;
    assert!(
        label.len() == crate::document::MAX_INLINE_TEXT_BYTES && label.bytes().all(|b| b == b'x'),
        "the bookmark label keeps the text up to the limit: {} bytes",
        label.len()
    );
}
