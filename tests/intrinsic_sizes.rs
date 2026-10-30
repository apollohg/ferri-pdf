mod common;
use common::{assert_labelled_pt, local_options, painted_box};
use ferri_pdf::Rect;
use ferri_pdf::render_html;

const CELL: f32 = 1229.0 / 2048.0 * 10.0;
fn probe(css: &str, body: &str) -> Rect {
    let trace = render_html(&format!("<style>{}body{{font:10pt/12pt 'Liberation Mono'}}p,div{{margin:0}}.probe{{background:rgb(1,2,3)}}{css}</style>{body}", common::PROFILE), &local_options())
        .unwrap_or_else(|error| panic!("intrinsic sizing: {error}; CSS={css}; HTML={body}"))
        .layout
        .unwrap();
    painted_box(&trace, [1, 2, 3])
}
#[test]
fn keywords_size_content_and_do_not_subtract_border_box_edges() {
    for (value, expected) in [
        ("min-content", 4.0 * CELL),
        ("max-content", 7.0 * CELL),
        ("fit-content", 30.0),
    ] {
        for sizing in ["content-box", "border-box"] {
            let css = format!(
                "section{{width:42pt}}.probe{{width:{value};padding:4pt;border:2pt solid;box-sizing:{sizing}}}"
            );
            let rect = probe(&css, "<section><div class=probe>AAAA BB</div></section>");
            assert_labelled_pt(&css, rect.width, expected + 12.0);
        }
    }
}
#[test]
fn intrinsic_minima_and_maxima_constrain_normal_and_atomic_boxes() {
    for display in ["block", "inline-block", "flow-root"] {
        for (limits, expected) in [
            ("width:1pt;min-width:min-content", 4.0 * CELL),
            ("width:100pt;max-width:max-content", 7.0 * CELL),
            ("width:max-content;max-width:30pt", 30.0),
        ] {
            let css = format!(".probe{{display:{display};{limits}}}");
            let rect = probe(&css, "<div class=probe>AAAA BB</div>");
            assert_labelled_pt(&css, rect.width, expected);
        }
    }
}
#[test]
fn formatting_contexts_resolve_intrinsic_preferred_widths() {
    for context in [
        "",
        "float:left",
        "position:absolute;left:0;right:0",
        "display:flex",
        "display:grid",
        "display:table",
    ] {
        for (keyword, cells) in [("min-content", 4.0), ("max-content", 7.0)] {
            let body = if context == "display:table" {
                "<div class=probe><div style='display:table-row'><div style='display:table-cell'>AAAA BB</div></div></div>"
            } else {
                "<div class=probe>AAAA BB</div>"
            };
            let css = format!(".probe{{{context};width:{keyword}}}");
            assert_labelled_pt(&css, probe(&css, body).width, cells * CELL);
        }
    }
}
#[test]
fn flex_basis_and_grid_items_use_the_same_intrinsic_sizes() {
    for layout in ["display:flex", "display:grid;grid-template-columns:100pt"] {
        let css =
            format!("section{{{layout};width:100pt}}.probe{{width:min-content;flex:0 0 auto}}");
        let rect = probe(&css, "<section><div class=probe>AAAA BB</div></section>");
        assert_labelled_pt(&css, rect.width, 4.0 * CELL);
    }
    for (basis, cells) in [("min-content", 4.0), ("max-content", 7.0)] {
        let css = format!("section{{display:flex;width:100pt}}.probe{{flex:0 0 {basis}}}");
        let rect = probe(&css, "<section><div class=probe>AAAA BB</div></section>");
        assert_labelled_pt(&css, rect.width, cells * CELL);
    }
}

#[test]
fn fit_content_uses_remaining_space_and_auto_margins() {
    // Content space: 60pt - insets - 10pt margins - 6pt edges, between min (4 cells) and max (7 cells).
    for (position, expected) in [
        ("", 7.0 * CELL + 6.0),
        ("position:absolute;left:10pt;right:5pt", 35.0),
    ] {
        let css = format!(
            "section{{position:relative;width:60pt}}.probe{{{position};width:fit-content;margin:0 5pt;padding:2pt;border:1pt solid}}"
        );
        let rect = probe(&css, "<section><div class=probe>AAAA BB</div></section>");
        assert_labelled_pt(&css, rect.width, expected);
    }
    let css = "section{width:100pt}.probe{width:min-content;margin:auto}";
    let rect = probe(css, "<section><div class=probe>AAAA BB</div></section>");
    assert_labelled_pt(css, rect.x, 36.0 + (100.0 - 4.0 * CELL) / 2.0);
}

#[test]
fn replaced_intrinsic_sizes_preserve_aspect_ratio_and_precision() {
    let svg = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='80' height='40'%3E%3Crect width='80' height='40' fill='red'/%3E%3C/svg%3E";
    for size in [
        "width:min-content",
        "width:max-content",
        "width:fit-content",
        "height:min-content",
        "height:max-content",
    ] {
        let css = format!(
            ".probe{{display:block;{size};padding:2pt;border:1pt solid;box-sizing:border-box}}"
        );
        let rect = probe(&css, &format!("<img class=probe src=\"{svg}\">"));
        assert_labelled_pt(&format!("{css} width"), rect.width, 66.0);
        assert_labelled_pt(&format!("{css} height"), rect.height, 36.0);
    }
}

#[test]
fn intrinsic_cell_widths_and_limits_affect_column_contributions() {
    for rule in ["width:min-content", "max-width:min-content"] {
        let css = format!("table{{border-spacing:0;font:inherit}}td{{padding:0;{rule}}}");
        let rect = probe(&css, "<table class=probe><tr><td>AAAA BB</td></tr></table>");
        assert_labelled_pt(&css, rect.width, 4.0 * CELL);
    }
}

#[test]
fn automatic_block_heights_and_column_flex_bases_retain_content_height() {
    for keyword in ["min-content", "max-content", "fit-content"] {
        for css in [
            format!(".probe{{width:25pt;height:{keyword}}}"),
            format!(
                "section{{display:flex;flex-direction:column;height:100pt}}.probe{{width:25pt;flex:0 0 {keyword}}}"
            ),
        ] {
            let rect = probe(&css, "<section><div class=probe>AAAA BB</div></section>");
            assert_labelled_pt(&css, rect.height, 24.0);
        }
    }
}

#[test]
fn captions_page_margin_boxes_and_collapsed_margins_use_intrinsic_widths() {
    let css = "table{font:inherit}caption{width:min-content;background:rgb(1,2,3)}";
    let rect = probe(
        css,
        "<table><caption>AAAA BB</caption><tr><td>AAAAAAAAAAAAAAAAAAAA</td></tr></table>",
    );
    assert_labelled_pt(css, rect.width, 4.0 * CELL);
    for (width, expected) in [
        ("min-content", 4.0 * CELL + 20.0),
        ("max-content", 7.0 * CELL + 20.0),
        ("fit-content", 7.0 * CELL + 20.0),
    ] {
        let css = format!(
            "@page{{@top-left{{content:'AAAA BB';font:10pt 'Liberation Mono';width:{width};padding:0 10pt;background:rgb(1,2,3)}}}}"
        );
        assert_labelled_pt(&css, probe(&css, "<p>x</p>").width, expected);
    }
    let css = ".parent{width:min-content}.child{margin-top:10%}";
    let rect = probe(
        css,
        "<div class=parent><div class='child probe'>AAAA BB</div></div>",
    );
    assert_labelled_pt(css, rect.y, 36.0 + 0.4 * CELL);
}

#[test]
fn replaced_flex_items_with_intrinsic_cross_sizes_do_not_stretch() {
    let svg = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='80' height='40'%3E%3C/svg%3E";
    for (layout, height, expected) in [
        (
            "display:flex;align-items:stretch;height:100pt",
            "min-content",
            30.0,
        ),
        (
            "display:flex;align-items:stretch;height:100pt",
            "auto",
            100.0,
        ),
        (
            "display:grid;grid-template-rows:100pt;align-items:stretch",
            "min-content",
            30.0,
        ),
    ] {
        let css = format!("section{{{layout}}}.probe{{height:{height}}}");
        let rect = probe(
            &css,
            &format!("<section><img class=probe src=\"{svg}\"></section>"),
        );
        assert_labelled_pt(&css, rect.height, expected);
    }
}

#[test]
fn non_replaced_block_axis_keywords_behave_as_auto_in_grid_and_tables() {
    for height in ["auto", "min-content", "max-content", "fit-content"] {
        let css =
            format!("section{{display:grid;grid-template-rows:100pt}}.probe{{height:{height}}}");
        let rect = probe(&css, "<section><div class=probe>x</div></section>");
        assert_labelled_pt(&css, rect.height, 100.0);
        let css = format!("table{{font:inherit;border-spacing:0}}td{{padding:0;height:{height}}}");
        let rect = probe(&css, "<table class=probe><tr><td>x</td></tr></table>");
        assert_labelled_pt(&css, rect.height, 12.0);
    }
}

const PAGE: &str = "@page{size:200pt 100pt;margin:10pt}";

/// A page's area and sheet, each `[width, height]`.
type PageSizes = ([f32; 2], [f32; 2]);

/// The sizes of each page of a rendering with the probe's font.
fn pages(css: &str, body: &str) -> Result<Vec<PageSizes>, ferri_pdf::Error> {
    let trace = render_html(&format!("<style>{}body{{margin:0;font:10pt/12pt 'Liberation Mono'}}p{{margin:0}}{PAGE}{css}</style>{body}", common::PROFILE), &local_options())?
        .layout
        .unwrap();
    Ok(trace
        .page_records
        .iter()
        .map(|record| (record.geometry.area, record.geometry.paper))
        .collect())
}

fn assert_page_areas(css: &str, body: &str, expected: &[[f32; 2]]) {
    let pages = pages(css, body).unwrap_or_else(|error| panic!("{css}: {error}"));
    assert_eq!(
        pages.len(),
        expected.len(),
        "{css}: page areas {pages:?}, expected {expected:?}"
    );
    for (index, ((area, _), expected)) in pages.iter().zip(expected).enumerate() {
        for axis in 0..2 {
            assert_labelled_pt(
                &format!("{css}: page {index} area {pages:?}"),
                area[axis],
                expected[axis],
            );
        }
    }
}

/// CSS Page 3 §3 sizes the page box as a block box in normal flow whose content is the root:
/// its inline size takes the root's contributions.
#[test]
fn page_widths_take_the_root_contributions() {
    for (sizes, width) in [
        ("width:min-content", 4.0 * CELL),
        ("width:max-content", 7.0 * CELL),
        ("width:fit-content", 7.0 * CELL),
        ("width:fit-content(30pt)", 30.0),
        ("width:10pt;min-width:max-content", 7.0 * CELL),
        ("max-width:min-content", 4.0 * CELL),
        (
            "width:calc-size(min-content,size + 10pt)",
            4.0 * CELL + 10.0,
        ),
    ] {
        assert_page_areas(
            &format!("@page{{{sizes}}}"),
            "<p>AAAA BB</p>",
            &[[width, 80.0]],
        );
    }
}

/// A block container's min-content and max-content block sizes are its content's (CSS Sizing
/// 3 §5.1): each page takes the block size of the content it holds up to its forced break.
#[test]
fn page_heights_take_the_content_each_page_holds() {
    assert_page_areas(
        "@page{height:max-content}",
        "<p>A</p><p style=break-before:page>B</p><p>C</p><p>D</p>",
        &[[180.0, 12.0], [180.0, 36.0]],
    );
    assert_page_areas(
        "@page{height:min-content}",
        "<p>A<span style=float:footnote>Note</span></p>",
        &[[180.0, 24.0]],
    );
    // Lines that continue on later pages find the width of a page not yet reached.
    assert_page_areas(
        "@page{block-size:max-content;max-inline-size:min-content}",
        "<p>AAAA BB</p>",
        &[[4.0 * CELL, 24.0]],
    );
    // In vertical writing modes the height is the inline axis and the width the block axis.
    assert_page_areas(
        "@page{height:fit-content}html{writing-mode:vertical-rl}",
        "<p>A</p><p>B</p>",
        &[[180.0, CELL]],
    );
    assert_page_areas(
        "@page{width:max-content}html{writing-mode:vertical-rl}",
        "<p>A</p><p>B</p>",
        &[[24.0, 80.0]],
    );
}

/// Content larger than a page's limit continues on the next page, whose own content sizes it.
#[test]
fn content_sized_pages_fragment_at_their_limits() {
    let lines: String = (0..10).map(|line| format!("<p>L{line}</p>")).collect();
    assert_page_areas(
        "@page{max-height:max-content}",
        &lines,
        &[[180.0, 80.0], [180.0, 48.0]],
    );
    let css = "@page{height:50pt;min-height:max-content}";
    assert_page_areas(css, &lines, &[[180.0, 120.0]]);
    let pages = pages(css, &lines).unwrap();
    assert_eq!(
        pages[0].1,
        [200.0, 140.0],
        "the sheet grows to the page box: {pages:?}"
    );
}

/// The page area's block size depends on its content, so percentage heights against it
/// compute to `auto` (CSS 2.1 §10.5); viewport units take the page area the styles saw, with
/// content-sized page sizes as `auto`, which the page cannot depend on.
#[test]
fn content_sized_pages_break_cycles_through_their_content() {
    assert_page_areas(
        "@page{height:max-content}html{height:100%}",
        "<p>A</p>",
        &[[180.0, 12.0]],
    );
    assert_page_areas(
        "@page{height:max-content}div{height:50vh}",
        "<div></div>",
        &[[180.0, 40.0]],
    );
    assert_page_areas(
        "@page{width:max-content}div{width:50vw}",
        "<div></div>",
        &[[90.0, 80.0]],
    );
}

/// A footnote area limited to half the page area its own height sizes has no settled layout.
#[test]
fn content_sized_pages_report_layouts_that_do_not_settle() {
    let error = pages(
        "@page{height:max-content;@footnote{max-height:50%}}",
        "<p>A<span style=float:footnote>B<br>C<br>D<br>E</span></p>",
    )
    .expect_err("the footnote area never fits the page it sizes");
    assert!(
        error.to_string().contains("intrinsically sized pages"),
        "{error}"
    );
}
