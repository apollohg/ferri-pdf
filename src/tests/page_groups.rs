use super::common;

use crate::{RenderOutput, render_html};
use common::local_options;

const PROFILE: &str = "@font-face{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}@page{size:200pt 200pt;margin:20pt}html,body,div,p,table,tbody,tr,td{margin:0;padding:0;font:10pt/10pt Fixed;border-spacing:0}p,td{height:160pt}";

fn render(css: &str, body: &str) -> RenderOutput {
    render_html(
        &format!(
            "<!doctype html><style>{}{PROFILE}{css}</style>{body}",
            common::PROFILE
        ),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("page-group CSS `{css}` with `{body}`: {error}"))
}

/// A page's first text line, its paper width and its page-group positions.
type PageSummary = (String, f32, Vec<(String, usize)>);
type ExpectedPage<'a> = (&'a str, f32, &'a [(&'a str, usize)]);

fn pages(output: &RenderOutput) -> Vec<PageSummary> {
    let trace = output.layout.as_ref().unwrap();
    trace
        .page_records
        .iter()
        .zip(&trace.pages)
        .enumerate()
        .map(|(index, (record, lines))| {
            let sequence = record
                .request
                .sequence
                .as_ref()
                .expect("position-selecting page rules give every request a sequence");
            assert_eq!(
                sequence.number,
                index + 1,
                "document page numbers count physical pages"
            );
            (
                lines
                    .first()
                    .map_or_else(String::new, |line| line.text.clone()),
                record.geometry.paper[0],
                sequence
                    .groups
                    .iter()
                    .map(|group| (group.name.to_string(), group.number))
                    .collect(),
            )
        })
        .collect()
}

fn expected(pages: &[ExpectedPage<'_>]) -> Vec<PageSummary> {
    pages
        .iter()
        .map(|(text, width, groups)| {
            (
                (*text).to_owned(),
                *width,
                groups
                    .iter()
                    .map(|(name, number)| ((*name).to_owned(), *number))
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn nth_selects_document_pages_and_pages_of_nested_groups_with_their_own_specificity() {
    let output = render(
        "@page :nth(2n){size:210pt 200pt}@page :nth(1 of chapter){size:300pt 200pt}@page :nth(2 of chapter){size:310pt 200pt}@page part:nth(2){size:400pt 200pt}.chapter{page:chapter;break-before:page}.part{page:part;break-before:page}",
        "<p>Intro</p><div class=chapter><p>C1a</p><p>C1b</p><p>C1c</p></div><div class=chapter><p>C2a</p><div class=part><p>B1</p><p>B2</p></div></div><p>Tail</p>",
    );
    // Page 6 is the second chapter's second page and the part's first; `part:nth(2)` counts
    // within the part group; equal specificities fall back to source order.
    assert_eq!(
        pages(&output),
        expected(&[
            ("Intro", 200.0, &[]),
            ("C1a", 300.0, &[("chapter", 1)]),
            ("C1b", 310.0, &[("chapter", 2)]),
            ("C1c", 210.0, &[("chapter", 3)]),
            ("C2a", 300.0, &[("chapter", 1)]),
            ("B1", 310.0, &[("chapter", 2), ("part", 1)]),
            ("B2", 400.0, &[("chapter", 3), ("part", 2)]),
            ("Tail", 210.0, &[]),
        ])
    );
}

#[test]
fn groups_start_at_the_first_page_exclude_preceding_blank_pages_and_follow_table_row_groups() {
    let output = render(
        "@page :nth(1 of chapter){size:300pt 200pt}@page :nth(2 of chapter){size:310pt 200pt}@page :nth(1 of rows){size:400pt 200pt}@page :nth(3 of rows){size:410pt 200pt}.chapter{page:chapter;break-before:right}tbody.rows{page:rows;break-before:page}",
        "<div class=chapter><p>C1a</p><p>C1b</p><p>C1c</p></div><div class=chapter><p>C2a</p></div><table><tbody><tr><td>R0</td></tr></tbody><tbody class=rows><tr><td>T1</td></tr><tr><td>T2</td></tr><tr><td>T3</td></tr></tbody><tbody><tr><td>After</td></tr></tbody></table>",
    );
    assert_eq!(
        pages(&output),
        expected(&[
            ("C1a", 300.0, &[("chapter", 1)]),
            ("C1b", 310.0, &[("chapter", 2)]),
            ("C1c", 200.0, &[("chapter", 3)]),
            ("", 200.0, &[]),
            ("C2a", 300.0, &[("chapter", 1)]),
            ("R0", 200.0, &[]),
            ("T1", 400.0, &[("rows", 1)]),
            ("T2", 200.0, &[("rows", 2)]),
            ("T3", 410.0, &[("rows", 3)]),
            ("After", 200.0, &[]),
        ])
    );
    let trace = output.layout.as_ref().unwrap();
    assert!(
        trace.page_records[3].request.blank,
        "the right-page break inserts a blank page before the second chapter"
    );
}

#[test]
fn page_names_without_forced_breaks_create_no_group() {
    let output = render(
        "@page :nth(1 of chapter){size:300pt 200pt}@page chapter:nth(2){size:310pt 200pt}.chapter{page:chapter}",
        "<p>Intro</p><div class=chapter><p>C1</p><p>C2</p></div>",
    );
    // Without a group, a named `:nth()` counts document pages.
    assert_eq!(
        pages(&output),
        expected(&[
            ("Intro", 200.0, &[]),
            ("C1", 310.0, &[]),
            ("C2", 200.0, &[]),
        ])
    );
}

#[test]
fn requests_omit_positions_when_no_rule_selects_them() {
    let output = render(
        ".chapter{page:chapter;break-before:page}",
        "<p>Intro</p><div class=chapter><p>C1</p></div>",
    );
    let trace = output.layout.unwrap();
    assert!(
        trace
            .page_records
            .iter()
            .all(|record| record.request.sequence.is_none()),
        "position-free requests keep sharing page records"
    );
}
