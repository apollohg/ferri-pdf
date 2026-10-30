//! CSS Fragmentation 4 §3: `always`, `all`, `avoid-page` and the region break values in a
//! paged flow and in multi-column flows nested in it.
mod common;

use common::render_trace;

/// Ten 10pt lines fill each 200pt × 100pt page and each 100pt-wide column.
const STYLE: &str = "@font-face{font-family:Spike;src:url('../assets/DejaVuSans.ttf')}\
    @page{size:200pt 100pt;margin:0}\
    html,body,p,div{margin:0;padding:0;font:10pt/10pt Spike;orphans:1;widows:1}\
    section{columns:2;column-gap:0;column-fill:auto}";
const COLUMN_WIDTH: f32 = 100.0;

/// Where a line landed: its page and its column.
#[derive(Debug, PartialEq, Eq)]
struct Place {
    page: usize,
    column: usize,
}

fn places(body: &str) -> Vec<(String, Place)> {
    let trace = render_trace(&format!("<!doctype html><style>{STYLE}</style>{body}"));
    trace
        .pages
        .iter()
        .enumerate()
        .flat_map(|(page, lines)| {
            lines.iter().map(move |line| {
                (
                    line.text.clone(),
                    Place {
                        page,
                        column: (line.x / COLUMN_WIDTH) as usize,
                    },
                )
            })
        })
        .collect()
}

fn place_of(body: &str, text: &str) -> Place {
    let places = places(body);
    places
        .into_iter()
        .find_map(|(line, place)| (line == text).then_some(place))
        .unwrap_or_else(|| panic!("no line {text:?} in {body}"))
}

fn filler(count: usize) -> String {
    (0..count).map(|index| format!("<p>F{index}</p>")).collect()
}

#[test]
fn always_breaks_the_innermost_fragmentation_context_and_all_breaks_every_one() {
    let in_columns = |value: &str| {
        place_of(
            &format!("<section><p>A</p><p style='break-before:{value}'>B</p></section>"),
            "B",
        )
    };
    assert_eq!(
        in_columns("always"),
        Place { page: 0, column: 1 },
        "`always` inside a multi-column container is a column break"
    );
    assert_eq!(in_columns("column"), Place { page: 0, column: 1 });
    assert_eq!(
        in_columns("all"),
        Place { page: 1, column: 0 },
        "`all` breaks the column and the page containing it"
    );
    assert_eq!(in_columns("page"), Place { page: 1, column: 0 });
    for value in ["always", "all"] {
        assert_eq!(
            place_of(
                &format!("<p>A</p><p style='break-before:{value}'>B</p>"),
                "B"
            ),
            Place { page: 1, column: 0 },
            "`{value}` in the page flow is a page break"
        );
        assert_eq!(
            place_of(
                &format!("<p style='break-after:{value}'>A</p><p>B</p>"),
                "B"
            ),
            Place { page: 1, column: 0 },
            "`break-after: {value}` in the page flow is a page break"
        );
    }
}

#[test]
fn avoid_page_keeps_boxes_together_across_page_breaks() {
    let heading = |value: &str| {
        place_of(
            &format!("{}<p style='break-after:{value}'>H</p><p>N</p>", filler(9)),
            "H",
        )
    };
    assert_eq!(heading("auto"), Place { page: 0, column: 0 });
    assert_eq!(
        heading("avoid-page"),
        Place { page: 1, column: 0 },
        "the heading moves to the page of the paragraph it must stay with"
    );
    let before = place_of(
        &format!(
            "{}<p>H</p><p style='break-before:avoid-page'>N</p>",
            filler(9)
        ),
        "H",
    );
    assert_eq!(before, Place { page: 1, column: 0 });
    let kept = |value: &str| {
        place_of(
            &format!(
                "{}<div style='break-inside:{value}'><p>K0</p><p>K1</p><p>K2</p></div>",
                filler(8)
            ),
            "K0",
        )
    };
    assert_eq!(kept("auto"), Place { page: 0, column: 0 });
    assert_eq!(kept("avoid-page"), Place { page: 1, column: 0 });
}

#[test]
fn avoid_page_between_columns_applies_only_where_the_column_break_is_a_page_break() {
    let heading = |filled: usize, value: &str| {
        place_of(
            &format!(
                "<section>{}<p style='break-after:{value}'>H</p><p>N</p></section>",
                filler(filled)
            ),
            "H",
        )
    };
    assert_eq!(
        heading(9, "avoid-page"),
        Place { page: 0, column: 0 },
        "a break into the second column on the same page is not a page break"
    );
    assert_eq!(
        heading(9, "avoid"),
        Place { page: 0, column: 1 },
        "`avoid` keeps the heading with its paragraph at any break"
    );
    assert_eq!(
        heading(19, "avoid-page"),
        Place { page: 1, column: 0 },
        "leaving the last column of the page is also a page break"
    );
    assert_eq!(heading(19, "avoid-column"), Place { page: 1, column: 0 });
    let kept = |filled: usize, value: &str| {
        place_of(
            &format!(
                "<section>{}<div style='break-inside:{value}'><p>K0</p><p>K1</p><p>K2</p></div></section>",
                filler(filled)
            ),
            "K0",
        )
    };
    assert_eq!(
        kept(8, "avoid-page"),
        Place { page: 0, column: 0 },
        "the box may break between the columns of one page"
    );
    assert_eq!(kept(8, "avoid-column"), Place { page: 0, column: 1 });
    assert_eq!(
        kept(18, "avoid-page"),
        Place { page: 1, column: 0 },
        "the box may not break from the last column of the page into the next page"
    );
}

#[test]
fn region_break_values_have_no_effect_without_regions() {
    let body = |before: &str, inside: &str| {
        format!(
            "<section>{}<div style='break-before:{before};break-after:{before};break-inside:{inside}'><p>K0</p><p>K1</p></div><p>N</p></section>{}",
            filler(9),
            filler(12)
        )
    };
    let reference = places(&body("auto", "auto"));
    assert_eq!(places(&body("region", "auto")), reference);
    assert_eq!(places(&body("avoid-region", "avoid-region")), reference);
}
