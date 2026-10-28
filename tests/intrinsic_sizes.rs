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

#[test]
fn page_box_intrinsic_sizes_are_rejected_instead_of_treated_as_auto() {
    for declaration in ["width:min-content", "min-height:max-content"] {
        let html = format!("<style>@page{{{declaration}}}</style><p>x</p>");
        let Err(error) = render_html(&html, &local_options()) else {
            panic!("{declaration}: page box keyword must stay visible as unsupported");
        };
        assert!(
            error.to_string().contains("intrinsic size keyword"),
            "{declaration}: {error}"
        );
    }
}
