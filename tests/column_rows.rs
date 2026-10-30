//! CSS Multi-column Layout 2 §4: `column-height`, `column-wrap` and the `columns` shorthand's
//! column height, which wrap overflowing content into rows of columns.
mod common;

use common::{assert_labelled_pt, painted_box, render_trace};
use ferri_pdf::LayoutTrace;

/// Ten 10pt lines fill each 300pt × 100pt page; each column is 150pt wide.
const STYLE: &str = "@page{size:300pt 100pt;margin:0}html,body,p{margin:0;padding:0;font:10pt/10pt serif}\
    section{column-gap:0;background:rgb(1,2,3)}";
const LINE: f32 = 10.0;
const COLUMN: f32 = 150.0;
const CONTAINER: [u8; 3] = [1, 2, 3];

fn render(css: &str, lines: usize) -> LayoutTrace {
    let content: String = (0..lines).map(|index| format!("L{index}<br>")).collect();
    render_trace(&format!(
        "<!doctype html><style>{STYLE}section{{{css}}}</style><section>{content}</section><p>after</p>"
    ))
}

/// Each line's page, column and top, by its text.
fn place(trace: &LayoutTrace, text: &str) -> (usize, usize, f32) {
    trace
        .pages
        .iter()
        .enumerate()
        .find_map(|(page, lines)| {
            lines.iter().find(|line| line.text == text).map(|line| {
                (
                    page,
                    (line.x / COLUMN).round() as usize,
                    first_line_top(trace, line.baseline),
                )
            })
        })
        .unwrap_or_else(|| panic!("no line {text}"))
}

/// A line's top relative to the first line's, which starts the first page.
fn first_line_top(trace: &LayoutTrace, baseline: f32) -> f32 {
    baseline - trace.pages[0][0].baseline
}

#[test]
fn a_column_height_wraps_overflowing_content_into_rows_after_a_row_gap() {
    let trace = render("columns:2 / 30pt;row-gap:5pt", 14);
    for (text, expected) in [
        ("L0", (0, 0, 0.0)),
        ("L3", (0, 1, 0.0)),
        ("L6", (0, 0, 3.0 * LINE + 5.0)),
        ("L9", (0, 1, 3.0 * LINE + 5.0)),
        ("L12", (0, 0, 2.0 * (3.0 * LINE + 5.0))),
        ("L13", (0, 1, 2.0 * (3.0 * LINE + 5.0))),
    ] {
        assert_eq!(
            place(&trace, text),
            expected,
            "{text}: page, column and top"
        );
    }
    assert_labelled_pt(
        "three rows keep the column height, with the gaps between them",
        painted_box(&trace, CONTAINER).height,
        3.0 * 30.0 + 2.0 * 5.0,
    );
}

#[test]
fn nowrap_overflows_columns_in_the_inline_direction_at_the_column_height() {
    let trace = render("columns:2 / 30pt;column-wrap:nowrap", 14);
    for (text, column) in [("L0", 0), ("L3", 1), ("L6", 2), ("L12", 4)] {
        let (page, actual, top) = place(&trace, text);
        assert_eq!((page, actual), (0, column), "{text}: overflow column");
        assert_labelled_pt(&format!("{text}: top"), top, 0.0);
    }
    assert_labelled_pt(
        "one row as tall as the column height",
        painted_box(&trace, CONTAINER).height,
        30.0,
    );
}

#[test]
fn wrap_without_a_column_height_uses_the_definite_container_height() {
    let trace = render(
        "columns:2;height:20pt;column-wrap:wrap;row-gap:0;column-fill:auto",
        8,
    );
    for (text, expected) in [
        ("L0", (0, 0, 0.0)),
        ("L2", (0, 1, 0.0)),
        ("L4", (0, 0, 2.0 * LINE)),
        ("L6", (0, 1, 2.0 * LINE)),
    ] {
        assert_eq!(
            place(&trace, text),
            expected,
            "{text}: page, column and top"
        );
    }
    assert_labelled_pt(
        "the container keeps its height and the rows overflow it",
        painted_box(&trace, CONTAINER).height,
        20.0,
    );
}

#[test]
fn rows_move_whole_to_the_next_page_without_a_gap_before_them() {
    let trace = render("columns:2 / 40pt;row-gap:10pt;column-fill:auto", 20);
    for (text, expected) in [
        ("L0", (0, 0, 0.0)),
        ("L8", (0, 0, 50.0)),
        ("L12", (0, 1, 50.0)),
        ("L16", (1, 0, 0.0)),
    ] {
        let (page, column, _) = place(&trace, text);
        assert_eq!(
            (page, column),
            (expected.0, expected.1),
            "{text}: page and column"
        );
        if page == 0 {
            assert_labelled_pt(&format!("{text}: top"), place(&trace, text).2, expected.2);
        }
    }
    let next = trace.pages[1]
        .iter()
        .find(|line| line.text == "L16")
        .unwrap();
    assert_labelled_pt(
        "the moved row starts at the page top",
        next.baseline,
        trace.pages[0][0].baseline,
    );
}

#[test]
fn the_page_constrains_a_taller_column_height() {
    let trace = render("columns:1 / 150pt;column-fill:auto", 15);
    let (page, _, _) = place(&trace, "L10");
    assert_eq!(
        page, 1,
        "a 150pt column on a 100pt page breaks after ten lines"
    );
}

#[test]
fn the_columns_shorthand_sets_the_column_height_and_resets_column_wrap() {
    let wrapped = render("column-wrap:nowrap;columns:2 / 30pt", 14);
    assert_eq!(
        place(&wrapped, "L6").1,
        0,
        "`columns` resets column-wrap to auto, which wraps"
    );
    let reset = render("column-height:30pt;columns:2", 14);
    assert_eq!(
        place(&reset, "L7").1,
        1,
        "`columns` without a height resets column-height to auto and balances one row"
    );
}
