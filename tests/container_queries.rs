//! CSS Conditional 5 container queries and container query length units, evaluated against
//! the container sizes layout measures.
mod common;

use common::{assert_labelled_pt, painted_box, render_trace};
use ferri_pdf::{DisplayItem, LayoutTrace};

const PAGE: &str = "@page{size:400pt 300pt;margin:0}body{margin:0;font:10pt/12pt serif}";
const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 128, 0];
const BLUE: [u8; 3] = [0, 0, 255];

#[test]
fn zoomed_size_queries_use_the_containers_own_css_coordinates() {
    for (ancestor_zoom, container_zoom, child_zoom) in [
        (1.0, 2.0, 1.0),
        (2.0, 1.0, 0.5),
        (2.0, 2.0, 0.25),
        (1.0, 0.5, 2.0),
    ] {
        for mode in ["horizontal-tb", "vertical-rl"] {
            let trace = trace(
                &format!(
                    "section{{zoom:{ancestor_zoom}}}.c{{zoom:{container_zoom};container-type:size;width:100pt;height:60pt;font-size:10pt;writing-mode:{mode}}}.x{{zoom:{child_zoom};width:10pt;height:10pt;background:red}}@container (width:100pt) and (height:60pt) and (width:10em){{.x{{background:green}}}}"
                ),
                "<section><div class=c><div class=x></div></div></section>",
            );
            assert_eq!(
                colors(&trace),
                [GREEN],
                "container coordinate query with zoom {ancestor_zoom}/{container_zoom}/{child_zoom}, {mode}"
            );
        }
    }
}

#[test]
fn container_units_use_unzoomed_container_sizes_and_the_consumers_zoom() {
    for (ancestor_zoom, container_zoom, child_zoom) in [
        (1.0, 2.0, 1.0),
        (1.0, 2.0, 0.5),
        (2.0, 0.5, 2.0),
        (0.5, 2.0, 0.5),
    ] {
        for (mode, unit, basis) in [
            ("horizontal-tb", "cqw", 100.0),
            ("horizontal-tb", "cqh", 60.0),
            ("horizontal-tb", "cqi", 100.0),
            ("horizontal-tb", "cqb", 60.0),
            ("vertical-rl", "cqi", 60.0),
            ("vertical-rl", "cqb", 100.0),
            ("vertical-rl", "cqmin", 60.0),
            ("vertical-rl", "cqmax", 100.0),
        ] {
            let trace = trace(
                &format!(
                    "section{{zoom:{ancestor_zoom}}}.c{{zoom:{container_zoom};container-type:size;width:100pt;height:60pt;writing-mode:{mode}}}.x{{zoom:{child_zoom};width:50{unit};height:10pt;background:green}}"
                ),
                "<section><div class=c><div class=x></div></div></section>",
            );
            let rect = painted_box(&trace, GREEN);
            assert_labelled_pt(
                &format!("{unit} with zoom {ancestor_zoom}/{container_zoom}/{child_zoom}, {mode}"),
                rect.width,
                basis * 0.5 * ancestor_zoom * container_zoom * child_zoom,
            );
        }
    }
}

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_trace(&format!("<!doctype html><style>{PAGE}{css}</style>{body}"))
}

fn colors(trace: &LayoutTrace) -> Vec<[u8; 3]> {
    fn collect(items: &[DisplayItem], colors: &mut Vec<[u8; 3]>) {
        for item in items {
            match item {
                DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                    colors.push(paint.decoration.background.rgb);
                }
                DisplayItem::Frame(frame) => collect(&frame.items, colors),
                _ => {}
            }
        }
    }
    let mut colors = Vec::new();
    for page in &trace.display_lists {
        collect(page, &mut colors);
    }
    colors
}

/// Each element queries its nearest size container, whose width layout measures; the query
/// changes the descendants' styles, never the container's own size.
#[test]
fn size_queries_select_the_nearest_container_and_its_measured_width() {
    let css = ".c{container-type:inline-size}.x{height:10pt;background:red}\
               @container (width >= 300pt){.x{background:green}}\
               @container (200pt < width < 300pt){.x{background:blue}}";
    for (width, expected) in [
        ("400pt", GREEN),
        ("250pt", BLUE),
        ("60%", BLUE),
        ("200pt", RED),
    ] {
        let trace = trace(
            &format!("{css}.c{{width:{width}}}"),
            "<div class=c><div class=x></div></div>",
        );
        assert_eq!(colors(&trace), [expected], "container width {width}");
    }
}

#[test]
fn nested_and_named_containers_answer_their_own_queries() {
    let trace = trace(
        ".outer{container:page-layout / inline-size;width:400pt}\
         .inner{container-type:inline-size;width:100pt}\
         .x{height:10pt;background:red}\
         @container (width > 200pt){.x{background:green}}\
         @container page-layout (width > 200pt){.named{background:blue}}\
         @container page-layout (width > 200pt){@container (width < 200pt){.both{height:20pt}}}",
        "<div class=outer><div class=x></div><div class=inner><div class=x></div>\
         <div class='x named both'></div></div></div>",
    );
    assert_eq!(
        colors(&trace),
        [GREEN, RED, BLUE],
        "nearest and named containers"
    );
    assert_labelled_pt(
        "nested @container rules apply when both hold",
        painted_box(&trace, BLUE).height,
        20.0,
    );
}

/// `not`, `or`, comma-separated conditions, range syntax, `min-`/`max-` prefixes, ratios
/// and orientation, with relative units resolving against the container's font.
#[test]
fn query_syntax_evaluates_like_media_features() {
    let css = ".c{container-type:size;width:200pt;height:100pt;font-size:20pt}\
               .x{height:10pt;background:red}";
    for (query, matches) in [
        ("(width: 200pt)", true),
        ("not (width > 200pt)", true),
        ("(width > 300pt) or (height = 100pt)", true),
        ("(width > 300pt) and (height = 100pt)", false),
        ("(min-width: 10em)", true),
        ("(max-width: 9em)", false),
        ("(50pt < height <= 100pt)", true),
        ("(aspect-ratio: 2/1)", true),
        ("(aspect-ratio > 3)", false),
        ("(orientation: landscape)", true),
        ("(inline-size > 100pt) and (block-size < 200pt)", true),
        ("(unknown-feature > 1pt)", false),
        ("missing-name (width > 0pt)", false),
        ("(width > 300pt), (height > 50pt)", true),
        ("(width > calc(100pt + 50pt))", true),
    ] {
        let trace = trace(
            &format!("{css}@container {query}{{.x{{background:green}}}}"),
            "<div class=c><div class=x></div></div>",
        );
        let expected = if matches { GREEN } else { RED };
        assert_eq!(colors(&trace), [expected], "@container {query}");
    }
}

/// An `inline-size` container answers only width queries; a box that size containment
/// cannot apply to answers none.
#[test]
fn containers_answer_only_the_axes_they_contain() {
    for (container, query, matches) in [
        ("container-type:inline-size", "(width > 0pt)", true),
        ("container-type:inline-size", "(height >= 0pt)", false),
        ("container-type:size;height:10pt", "(height >= 0pt)", true),
        (
            "container-type:inline-size;display:table",
            "(width > 0pt)",
            false,
        ),
    ] {
        let trace = trace(
            &format!(
                ".c{{{container}}}.x{{height:10pt;background:red}}@container {query}{{.x{{background:green}}}}"
            ),
            "<div class=c><div class=x></div></div>",
        );
        let expected = if matches { GREEN } else { RED };
        assert_eq!(colors(&trace), [expected], "`{container}` with {query}");
    }
}

/// Style queries test the nearest ancestor's computed values, custom or standard.
#[test]
fn style_queries_compare_computed_values() {
    for (container, query, matches) in [
        ("--theme:dark", "style(--theme: dark)", true),
        ("--theme:light", "style(--theme: dark)", false),
        ("--theme:dark", "style(--theme)", true),
        ("", "style(--theme)", false),
        ("--size:20pt", "style(--size > 10pt)", true),
        ("font-size:20pt", "style(font-size: 20pt)", true),
        ("font-size:20pt", "style(font-size: 1em)", false),
        ("--a:1;--b:2", "style((--a: 1) and (--b: 3))", false),
        ("--a:1;--b:2", "style((--a: 1) or (--b: 3))", true),
        ("--a:1", "not style(--a: 1)", false),
    ] {
        let trace = trace(
            &format!(
                ".c{{{container}}}.x{{height:10pt;background:red}}@container {query}{{.x{{background:green}}}}"
            ),
            "<div class=c><div class=x></div></div>",
        );
        let expected = if matches { GREEN } else { RED };
        assert_eq!(colors(&trace), [expected], "`{container}` with {query}");
    }
}

/// Custom properties in a size query come from the query container (CSS Conditional 5
/// §6.1).
#[test]
fn size_query_variables_substitute_from_the_container() {
    let trace = trace(
        ".c{container-type:inline-size;width:200pt}.a{--limit:100pt}.b{--limit:300pt}\
         .x{height:10pt;background:red}@container (width > var(--limit)){.x{background:green}}",
        "<div class='c a'><div class=x></div></div><div class='c b'><div class=x></div></div>",
    );
    assert_eq!(colors(&trace), [GREEN, RED]);
}

/// Container query length units measure the nearest container in each axis, or the page
/// area without one.
#[test]
fn container_units_measure_the_nearest_container() {
    let trace = trace(
        ".c{container-type:size;width:200pt;height:100pt}\
         .w{container-type:inline-size;width:80pt}\
         .a{width:50cqw;height:10cqh;background:red}\
         .b{width:50cqi;height:10cqb;background:blue}\
         .g{width:10cqmax;height:10cqmin;background:green}",
        "<div class=c><div class=a></div><div class=w><div class=b></div></div></div>\
         <div class=g></div>",
    );
    let a = painted_box(&trace, RED);
    assert_labelled_pt("cqw", a.width, 100.0);
    assert_labelled_pt("cqh", a.height, 10.0);
    let b = painted_box(&trace, BLUE);
    assert_labelled_pt("cqi from the inline-size container", b.width, 40.0);
    assert_labelled_pt("cqb skips the inline-size container", b.height, 10.0);
    let g = painted_box(&trace, GREEN);
    assert_labelled_pt(
        "cqmax without a container uses the page area",
        g.width,
        40.0,
    );
    assert_labelled_pt(
        "cqmin without a container uses the page area",
        g.height,
        30.0,
    );
}

/// A container's size can depend on another container's queried descendants: the grid row
/// that stretches size container `b` grows with `a`'s queried content, so restyling repeats
/// until every consulted size is the one layout measures.
#[test]
fn dependent_containers_settle() {
    let trace = trace(
        ".g{display:grid;grid-template-columns:200pt 200pt}\
         .a{container-type:inline-size}.b{container-type:size}\
         .grow{height:10pt}@container (width > 150pt){.grow{height:80pt}}\
         .x{height:5pt;background:red}@container (height > 50pt){.x{background:green}}",
        "<div class=g><div class=a><div class=grow></div></div><div class=b><div class=x></div></div></div>",
    );
    assert_eq!(
        colors(&trace),
        [GREEN],
        "`b` stretches to the 80pt row that `a`'s queried content makes"
    );
}

/// Pagination still applies inside containers: an inline-size container fragments, and
/// its queried descendants keep their styles on every page.
#[test]
fn containers_fragment_across_pages() {
    let trace = trace(
        ".c{container-type:inline-size;width:300pt}.x{height:200pt;background:red}\
         @container (width > 200pt){.x{background:green}}",
        "<div class=c><div class=x></div><div class=x></div></div>",
    );
    assert_eq!(trace.pages.len(), 2, "two 200pt boxes need two 300pt pages");
    assert_eq!(
        colors(&trace),
        [GREEN; 3],
        "the second box fragments across the break and stays green on both pages"
    );
}

/// Page rules have no query container, so inside `@container` they never apply.
#[test]
fn page_rules_inside_container_rules_do_not_apply() {
    let trace = trace(
        "@container (width > 0pt){@page{size:100pt 100pt}}",
        "<p>text</p>",
    );
    assert_labelled_pt("page width", trace.page_width, 400.0);
}

/// Static media never scroll (CSS Overflow 3 §3.1.3): scroll containers have no reachable
/// or scrolled overflow and nothing snaps, so only `none` holds, and only for the boxes each
/// feature describes; a boolean context is false. Only `scroll-state` containers answer.
#[test]
fn scroll_state_queries_see_an_unscrolled_document() {
    const SCROLLER: &str = "container-type:scroll-state;overflow:auto;height:20pt";
    const PLAIN: &str = "container-type:scroll-state";
    const SIZE: &str = "container-type:inline-size;overflow:auto;height:20pt";
    for (container, query, matches) in [
        (SCROLLER, "scroll-state(scrollable: none)", true),
        (SCROLLER, "scroll-state(scrollable: bottom)", false),
        (SCROLLER, "scroll-state(scrollable: y)", false),
        (SCROLLER, "scroll-state(scrollable)", false),
        (SCROLLER, "not scroll-state(scrollable)", true),
        (PLAIN, "scroll-state(scrollable: none)", false),
        (SCROLLER, "scroll-state(scrolled: none)", true),
        (SCROLLER, "scroll-state(scrolled: block-end)", false),
        (PLAIN, "scroll-state(scrolled: none)", false),
        (PLAIN, "scroll-state(snapped: none)", true),
        (PLAIN, "scroll-state(snapped: both)", false),
        (PLAIN, "scroll-state(snapped)", false),
        (PLAIN, "scroll-state(stuck: none)", false),
        (PLAIN, "scroll-state(stuck: top)", false),
        (
            PLAIN,
            "scroll-state((snapped: none) and (stuck: left))",
            false,
        ),
        (
            PLAIN,
            "scroll-state((snapped: none) or (stuck: left))",
            true,
        ),
        (SIZE, "scroll-state(scrollable: none)", false),
        (SCROLLER, "scroll-state(scrollable: sideways)", false),
    ] {
        let trace = trace(
            &format!(
                ".c{{{container}}}.x{{height:10pt;background:red}}@container {query}{{.x{{background:green}}}}"
            ),
            "<div class=c><div class=x></div></div>",
        );
        let expected = if matches { GREEN } else { RED };
        assert_eq!(colors(&trace), [expected], "`{container}` with {query}");
    }
}

/// A sticky container's stuck state depends on its shift, which differs on each page a
/// static rendering fragments it across.
#[test]
fn stuck_queries_on_sticky_containers_are_unsupported() {
    let error = ferri_pdf::render_html(
        "<style>.c{container-type:scroll-state;position:sticky;top:0}@container scroll-state(stuck: top){p{color:red}}</style><div class=c><p>x</p></div>",
        &ferri_pdf::RenderOptions::default(),
    )
    .err()
    .expect("a sticky query container's stuck state aborts the document");
    assert!(
        error.to_string().contains("scroll-state(stuck)"),
        "the error names the query: {error}"
    );
}

/// `attr()` and `if()` in a query substitute from the query container, not the element
/// styled (CSS Conditional 5 §6.1).
#[test]
fn query_substitutions_read_the_container_attributes_and_conditions() {
    let trace = trace(
        ".c{container-type:inline-size;width:200pt;--mode:print}\
         .x{height:10pt;background:red}\
         @container (width > attr(data-limit <length>)){.x{background:green}}\
         @container style(--copy: attr(data-copy)){.y{background:blue}}\
         @container style(--mode: if(media(print): print; else: screen)){.z{background:green}}",
        "<div class=c data-limit=100pt data-copy=yes style='--copy:\"yes\"'><div class=x data-limit=300pt></div></div>\
         <div class=c data-limit=300pt><div class=x data-limit=100pt></div></div>\
         <div class=c data-copy=no style='--copy:\"yes\"'><div class='x y'></div></div>\
         <div class=c data-limit=300pt data-copy=yes style='--copy:\"yes\"'><div class='x y'></div></div>\
         <div class=c><div class='x z'></div></div>",
    );
    assert_eq!(colors(&trace), [GREEN, RED, RED, BLUE, GREEN]);
}

#[test]
fn vertical_query_containers_distinguish_physical_sizes_from_logical_axes() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        let css = format!(
            ".c{{writing-mode:{mode};container-type:size;width:80pt;height:120pt}}            .x{{background:red;width:10pt;height:10pt}}            @container (width:80pt) and (height:120pt) and (inline-size:120pt) and (block-size:80pt){{.x{{background:green}}}}"
        );
        let trace = trace(&css, "<div class=c><div class=x></div></div>");
        assert_eq!(
            colors(&trace),
            [GREEN],
            "{mode}: logical queries must use the container's axes while width/height stay physical"
        );
    }
}
