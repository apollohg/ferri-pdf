//! CSS Sizing 4 §3.2 `fit-content(<length-percentage>)` and CSS Values 5 §10 `calc-size()`
//! on the sizing properties, across formatting contexts.
mod common;
use common::{assert_labelled_pt, local_options, painted_box};
use ferri_pdf::Rect;
use ferri_pdf::render_html;

/// One Liberation Mono advance at 10pt.
const CELL: f32 = 1229.0 / 2048.0 * 10.0;
/// "AAAA BB": its min-content width is four cells and its max-content width seven.
const TEXT: &str = "AAAA BB";
const MIN_CONTENT: f32 = 4.0 * CELL;
const MAX_CONTENT: f32 = 7.0 * CELL;
/// The profile's page content width.
const PAGE_WIDTH: f32 = 288.0;
const LINE: f32 = 12.0;

fn probe(css: &str, body: &str) -> Rect {
    let trace = render_html(
        &format!(
            "<style>{}body{{font:10pt/{LINE}pt 'Liberation Mono'}}p,div{{margin:0}}.probe{{background:rgb(1,2,3)}}{css}</style>{body}",
            common::PROFILE
        ),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("sizing functions: {error}; CSS={css}; HTML={body}"))
    .layout
    .unwrap();
    painted_box(&trace, [1, 2, 3])
}

fn probe_text(css: &str) -> Rect {
    probe(css, &format!("<div class=probe>{TEXT}</div>"))
}

#[test]
fn fit_content_function_clamps_its_argument_between_the_content_sizes() {
    for (argument, expected) in [
        ("30pt", 30.0),
        ("1pt", MIN_CONTENT),
        ("200pt", MAX_CONTENT),
        ("10%", 28.8),
        ("calc(5% + 15pt)", 29.4),
    ] {
        for display in ["block", "inline-block", "flow-root"] {
            let css = format!(".probe{{display:{display};width:fit-content({argument})}}");
            assert_labelled_pt(&css, probe_text(&css).width, expected);
        }
    }
}

#[test]
fn fit_content_function_arguments_are_in_the_box_sizing_box() {
    let css = ".probe{width:fit-content(40pt);padding:0 5pt;border:0 solid;box-sizing:border-box}";
    assert_labelled_pt(css, probe_text(css).width, 40.0);
    let css = ".probe{width:fit-content(40pt);padding:0 5pt;box-sizing:content-box}";
    assert_labelled_pt(css, probe_text(css).width, 40.0 + 10.0);
}

#[test]
fn fit_content_function_limits_minimum_and_maximum_sizes() {
    for (css, expected) in [
        (".probe{width:1pt;min-width:fit-content(30pt)}", 30.0),
        (".probe{width:200pt;max-width:fit-content(30pt)}", 30.0),
        (
            ".probe{width:200pt;max-width:fit-content(1pt)}",
            MIN_CONTENT,
        ),
    ] {
        assert_labelled_pt(css, probe_text(css).width, expected);
    }
}

#[test]
fn fit_content_function_contributes_its_clamped_size_to_shrink_to_fit_containers() {
    for (argument, expected) in [("30pt", 30.0), ("200pt", MAX_CONTENT), ("10%", MAX_CONTENT)] {
        let css = format!(".probe{{float:left}}.inner{{width:fit-content({argument})}}");
        let rect = probe(
            &css,
            &format!("<div class=probe><div class=inner>{TEXT}</div></div>"),
        );
        assert_labelled_pt(&css, rect.width, expected);
    }
}

#[test]
fn fit_content_function_sizes_flex_and_grid_items_and_flex_bases() {
    for (container, item) in [
        ("display:flex", "width:fit-content(30pt)"),
        ("display:flex", "flex-basis:fit-content(30pt)"),
        (
            "display:grid;justify-items:start",
            "width:fit-content(30pt)",
        ),
        (
            "position:relative;height:100pt",
            "position:absolute;width:fit-content(30pt)",
        ),
    ] {
        let css = format!("section{{{container}}}.probe{{{item}}}");
        let rect = probe(
            &css,
            &format!("<section><div class=probe>{TEXT}</div></section>"),
        );
        assert_labelled_pt(&css, rect.width, 30.0);
    }
}

#[test]
fn calc_size_adjusts_the_size_of_its_basis() {
    for (value, expected) in [
        ("calc-size(auto, size / 2)", PAGE_WIDTH / 2.0),
        ("calc-size(auto, size - 8pt)", PAGE_WIDTH - 8.0),
        ("calc-size(min-content, size + 20pt)", MIN_CONTENT + 20.0),
        ("calc-size(max-content, size * 2)", MAX_CONTENT * 2.0),
        ("calc-size(fit-content, size + 1pt)", MAX_CONTENT + 1.0),
        ("calc-size(any, 50% + 10pt)", PAGE_WIDTH / 2.0 + 10.0),
        ("calc-size(any, 10pt)", 10.0),
        ("calc-size(60pt, size / 3)", 20.0),
        ("calc-size(50%, size + 10pt)", PAGE_WIDTH / 2.0 + 10.0),
        ("calc-size(auto, size / 2 + 10%)", PAGE_WIDTH / 2.0 + 28.8),
        (
            "calc-size(calc-size(min-content, size + 10pt), size * 2)",
            (MIN_CONTENT + 10.0) * 2.0,
        ),
        ("calc-size(auto, round(up, size, 100pt))", 300.0),
        ("calc-size(auto, min(size, 50pt))", 50.0),
    ] {
        let css = format!(".probe{{width:{value}}}");
        assert_labelled_pt(&css, probe_text(&css).width, expected);
    }
}

#[test]
fn calc_size_sees_and_gives_sizes_in_the_box_sizing_box() {
    let css = ".probe{width:calc-size(auto, size / 2);padding:0 10pt;box-sizing:border-box}";
    assert_labelled_pt(css, probe_text(css).width, PAGE_WIDTH / 2.0);
    let css = ".probe{width:calc-size(auto, size / 2);padding:0 10pt}";
    assert_labelled_pt(css, probe_text(css).width, (PAGE_WIDTH - 20.0) / 2.0 + 20.0);
}

#[test]
fn calc_size_of_an_automatic_width_follows_each_context_automatic_width() {
    for (context, automatic) in [
        ("float:left", MAX_CONTENT),
        ("display:inline-block", MAX_CONTENT),
        ("position:absolute", MAX_CONTENT),
        ("position:absolute;left:0;right:0", PAGE_WIDTH),
        ("display:table", MAX_CONTENT),
    ] {
        let css = format!(".probe{{{context};width:calc-size(auto, size + 10pt)}}");
        let body = if context == "display:table" {
            format!(
                "<div class=probe><div style='display:table-row'><div style='display:table-cell'>{TEXT}</div></div></div>"
            )
        } else {
            format!("<div class=probe>{TEXT}</div>")
        };
        assert_labelled_pt(&css, probe(&css, &body).width, automatic + 10.0);
    }
    for (container, automatic) in [
        ("display:flex", MAX_CONTENT),
        ("display:grid", PAGE_WIDTH),
        ("display:flex;flex-direction:column", PAGE_WIDTH),
    ] {
        let css = format!("section{{{container}}}.probe{{width:calc-size(auto, size + 10pt)}}");
        let rect = probe(
            &css,
            &format!("<section><div class=probe>{TEXT}</div></section>"),
        );
        assert_labelled_pt(&css, rect.width, automatic + 10.0);
    }
}

#[test]
fn calc_size_of_an_automatic_height_adjusts_the_content_height() {
    for (value, expected) in [
        ("calc-size(auto, size * 2)", 2.0 * LINE),
        ("calc-size(auto, round(up, size, 50pt))", 50.0),
        ("calc-size(auto, size + 10%)", LINE),
        ("calc-size(min-content, size + 5pt)", LINE + 5.0),
    ] {
        let css = format!(".probe{{height:{value}}}");
        assert_labelled_pt(&css, probe_text(&css).height, expected);
    }
    let css = ".probe{min-height:calc-size(auto, size + 30pt)}";
    assert_labelled_pt(css, probe(css, "<div class=probe></div>").height, 30.0);
    let css = "section{height:200pt}.probe{height:calc-size(auto, size + 10%)}";
    let rect = probe(
        css,
        &format!("<section><div class=probe>{TEXT}</div></section>"),
    );
    assert_labelled_pt(css, rect.height, LINE + 20.0);
}

#[test]
fn calc_size_stretch_basis_fills_the_available_space() {
    let css = ".probe{float:left;width:calc-size(stretch, size - 8pt);margin-left:2pt}";
    assert_labelled_pt(css, probe_text(css).width, PAGE_WIDTH - 2.0 - 8.0);
    let css = "section{height:200pt}.probe{height:calc-size(stretch, size - 20pt);margin-top:5pt}";
    let rect = probe(
        css,
        &format!("<section><div class=probe>{TEXT}</div></section>"),
    );
    assert_labelled_pt(css, rect.height, 200.0 - 5.0 - 20.0);
}

#[test]
fn calc_size_contributes_its_adjusted_basis_to_shrink_to_fit_containers() {
    for (value, expected) in [
        ("calc-size(auto, size + 10pt)", MAX_CONTENT + 10.0),
        ("calc-size(min-content, size * 2)", MIN_CONTENT * 2.0),
        ("calc-size(any, 33pt)", 33.0),
    ] {
        let css = format!(".probe{{float:left}}.inner{{width:{value}}}");
        let rect = probe(
            &css,
            &format!("<div class=probe><div class=inner>{TEXT}</div></div>"),
        );
        assert_labelled_pt(&css, rect.width, expected);
    }
}

#[test]
fn calc_size_adjusts_flex_bases_and_automatic_minimum_sizes() {
    let css = "section{display:flex}.probe{flex-basis:calc-size(auto, size + 10pt);flex-shrink:0}";
    let rect = probe(
        css,
        &format!("<section><div class=probe>{TEXT}</div></section>"),
    );
    assert_labelled_pt(css, rect.width, MAX_CONTENT + 10.0);
    let css = "section{display:flex;width:10pt}.probe{min-width:calc-size(auto, size + 6pt)}";
    let rect = probe(
        css,
        &format!("<section><div class=probe>{TEXT}</div></section>"),
    );
    assert_labelled_pt(css, rect.width, MIN_CONTENT + 6.0);
}

#[test]
fn calc_size_substitution_is_bounded() {
    const NESTING: usize = 16;
    let mut value = "calc-size(auto, size + size)".to_owned();
    for _ in 0..NESTING {
        value = format!("calc-size({value}, size + size)");
    }
    let css = format!(".probe{{width:20pt;width:{value}}}");
    assert_labelled_pt(
        "a calc-size() whose substitution doubles at every level is invalid",
        probe_text(&css).width,
        20.0,
    );
    let css = ".probe{width:calc-size(calc-size(calc-size(auto, size / 2), size / 2), size + 1pt)}";
    assert_labelled_pt(css, probe_text(css).width, PAGE_WIDTH / 4.0 + 1.0);
}

#[test]
fn maximum_sizes_reject_an_automatic_calc_size_basis() {
    let css = ".probe{max-width:calc-size(auto, size / 2)}";
    assert_labelled_pt(css, probe_text(css).width, PAGE_WIDTH);
    let css = ".probe{max-width:calc-size(calc-size(auto, size), size / 2)}";
    assert_labelled_pt(css, probe_text(css).width, PAGE_WIDTH);
    let css = ".probe{max-width:calc-size(max-content, size / 2)}";
    assert_labelled_pt(css, probe_text(css).width, MAX_CONTENT / 2.0);
}
