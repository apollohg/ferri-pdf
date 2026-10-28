use super::common;

use crate::document::{BlockBox, BlockContent, DocumentError, StyledDocument};
use crate::resources::FileResolver;
use crate::style::StyleError;
use common::{PROFILE, local_options};

const PAGE_RULES: &str = "@page{size:200pt 300pt;margin:10pt}@page A{size:400pt 500pt;margin:20pt}@page B{size:600pt 700pt;margin:30pt}";

#[test]
fn first_page_side_and_name_are_selected_before_viewport_lengths() {
    use crate::style::PageSide;
    for (direction, before, side) in [
        ("ltr", "left", PageSide::Left),
        ("ltr", "right", PageSide::Right),
        ("ltr", "page", PageSide::Left),
        ("rtl", "auto", PageSide::Left),
        ("rtl", "recto", PageSide::Left),
        ("rtl", "verso", PageSide::Right),
        ("rtl", "page", PageSide::Right),
    ] {
        let document = parse(
            &format!("@page A:left{{size:300pt 400pt;margin:10pt}}@page A:right{{size:500pt 400pt;margin:10pt}}html{{direction:{direction};padding-left:10vw}}p{{page:A;break-before:{before};margin-left:10vw}}"),
            "<p>Visible</p>",
        ).unwrap_or_else(|error| panic!("{direction}/{before}: {error}"));
        let expected_width = if side == PageSide::Left { 300.0 } else { 500.0 };
        assert_eq!(
            document.page_box().request.side,
            side,
            "{direction}/{before}"
        );
        assert_eq!(document.styles().page, [expected_width, 400.0]);
        let viewport_tenth = (expected_width - 20.0) / 10.0;
        assert_eq!(
            document.root().style().padding[3].resolve(0.0),
            viewport_tenth
        );
        assert_eq!(
            block_with_text(document.root(), "Visible").style().margin[3].resolve(0.0),
            Some(viewport_tenth)
        );
    }
}

#[test]
fn leading_side_selection_reports_its_viewport_cycle() {
    let Err(error) = parse(
        "@page:left{size:300pt 400pt}p{break-before:if(style(width:100pt):left;else:right);width:10vw}",
        "<p>Coupled side</p>",
    ) else {
        panic!("side-dependent geometry must not be resolved by guessing a side")
    };
    let DocumentError::Style(StyleError::DependencyCycle {
        sources,
        dependencies,
    }) = error
    else {
        panic!("expected a source-bearing side cycle, got {error}");
    };
    assert_ne!(sources, [] as [crate::style::SourceLocation; 0]);
    for expected in ["first-page-side", ".break-before", ".width"] {
        assert!(
            dependencies.iter().any(|node| node.ends_with(expected)),
            "missing {expected}: {dependencies:?}"
        );
    }
}

#[test]
fn body_progression_retains_root_computed_direction_and_selected_viewport() {
    let document = parse(
        "@page:left{size:300pt 400pt;margin:10pt}html{direction:ltr;padding-left:10vw}body{direction:rtl}",
        "<p>Visible</p>",
    ).unwrap();
    assert!(
        !document.root().style().rtl,
        "propagation must not rewrite computed root direction"
    );
    assert!(block_with_text(document.root(), "Visible").style().rtl);
    assert!(document.styles().page_progression_rtl());
    assert_eq!(document.root().style().padding[3].resolve(0.0), 28.0);
}

#[test]
fn side_independent_geometry_can_supply_viewport_values_during_selection() {
    for css in [
        "p{break-before:if(style(width:18pt):left;else:right);width:10vw}",
        "body{direction:if(style(width:18pt):rtl;else:ltr);width:10vw}",
    ] {
        let document = StyledDocument::parse(
            &format!("<!doctype html><style>{PROFILE}@page{{size:200pt 300pt;margin:10pt}}{css}</style><p>Visible</p>"),
            &local_options().base, &[], &FileResolver::default(),
        ).unwrap_or_else(|error| panic!("independent geometry must not produce a false cycle for {css}: {error}"));
        assert_eq!(
            document.page_box().request.side,
            crate::style::PageSide::Left
        );
        assert_eq!(document.styles().page, [200.0, 300.0]);
    }
}

#[test]
fn independent_page_name_can_supply_geometry_to_side_selection() {
    let document = parse(
        "body{page:A;direction:if(style(width:36pt):rtl;else:ltr);width:10vw}",
        "<p>Visible</p>",
    )
    .unwrap_or_else(|error| {
        panic!("independent page A must supply the viewport for direction: {error}")
    });
    assert_eq!(document.styles().page, [400.0, 500.0]);
    assert_eq!(
        document.page_box().request.side,
        crate::style::PageSide::Left
    );
}

#[test]
fn independent_page_side_can_supply_geometry_to_name_selection() {
    let document = StyledDocument::parse(
        &format!("<!doctype html><style>{PROFILE}@page{{size:200pt 300pt;margin:10pt}}@page:left{{size:300pt 400pt}}body{{direction:rtl}}p{{page:if(style(width:28pt):A;else:B);width:10vw}}</style><p>Visible</p>"),
        &local_options().base, &[], &FileResolver::default(),
    ).unwrap_or_else(|error| panic!("independent left page must supply the viewport for page name: {error}"));
    assert_eq!(document.styles().page, [300.0, 400.0]);
    assert_eq!(document.page_box().request.name.as_deref(), Some("A"));
    assert_eq!(
        document.page_box().request.side,
        crate::style::PageSide::Left
    );
}

#[test]
fn inactive_side_qualified_page_names_do_not_create_viewport_cycles() {
    let document = StyledDocument::parse(
        &format!("<!doctype html><style>{PROFILE}@page{{size:200pt 300pt;margin:10pt}}@page A:left{{size:400pt 500pt}}p{{page:if(style(width:18pt):A;else:B);width:10vw}}</style><p>Visible</p>"),
        &local_options().base, &[], &FileResolver::default(),
    ).unwrap_or_else(|error| panic!("a left-only named rule cannot affect the fixed right first page: {error}"));
    assert_eq!(document.styles().page, [200.0, 300.0]);
    assert_eq!(document.page_box().request.name.as_deref(), Some("A"));
    assert_eq!(
        document.page_box().request.side,
        crate::style::PageSide::Right
    );
}

fn parse(css: &str, body: &str) -> Result<StyledDocument, DocumentError> {
    StyledDocument::parse(
        &format!("<!doctype html><style>{PROFILE}{PAGE_RULES}{css}</style>{body}"),
        &local_options().base,
        &[],
        &FileResolver::default(),
    )
}

fn block_with_text<'a>(root: &'a BlockBox, text: &str) -> &'a BlockBox {
    let mut pending = vec![root];
    while let Some(block) = pending.pop() {
        for child in block.children() {
            match child {
                BlockContent::Inline(paragraph) if paragraph.text() == text => return block,
                BlockContent::Block(child) => pending.push(child),
                _ => {}
            }
        }
    }
    panic!("no block directly contains {text:?}");
}

#[test]
fn first_page_geometry_precedes_viewport_values_on_ancestors_and_descendants() {
    let document = parse(
        "html{padding-left:10vw}body{display:contents;page:A}.hidden{display:none;page:B}p{margin-left:10vw}",
        "<div class=hidden>Hidden</div><p>Visible</p>",
    ).unwrap();
    assert_eq!(document.styles().page, [400.0, 500.0]);
    assert_eq!(
        document.root().style().padding[3].resolve(0.0),
        36.0,
        "root viewport lengths use the first selected 360pt content width"
    );
    assert_eq!(
        block_with_text(document.root(), "Visible").style().margin[3].resolve(0.0),
        Some(36.0)
    );
}

#[test]
fn generated_before_and_order_modified_items_select_the_first_page_without_loading_unused_assets() {
    let generated = parse(
        "body{page:A}body::before{display:block;content:'Before';page:B}@page unused{background:url(definitely-missing-page.png)}",
        "<p>Body</p>",
    ).unwrap();
    assert_eq!(generated.styles().page, [600.0, 700.0]);
    let flex = parse(
        "body{display:flex}p{page:A}.first{order:-1;page:B}",
        "<p>Later</p><p class=first>First</p>",
    )
    .unwrap();
    assert_eq!(flex.styles().page, [600.0, 700.0]);
}

#[test]
fn first_page_selection_reports_a_source_bearing_viewport_dependency_cycle() {
    let Err(error) = parse(
        "p{page:if(style(width: 100pt): A; else: B);width:10vw}",
        "<p>Coupled selection</p>",
    ) else {
        panic!("page selection and its own viewport cannot be resolved by guessing a default page")
    };
    let DocumentError::Style(StyleError::DependencyCycle {
        sources,
        dependencies,
    }) = error
    else {
        panic!("expected a typed first-page dependency cycle, got {error}");
    };
    assert!(
        !sources.is_empty(),
        "the cycle must retain authored page/width locations"
    );
    assert!(
        dependencies
            .iter()
            .any(|node| node.contains("first-page-start")),
        "{dependencies:?}"
    );
    assert!(
        dependencies.iter().any(|node| std::path::Path::new(node)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("page"))),
        "{dependencies:?}"
    );
    assert!(
        dependencies.iter().any(|node| std::path::Path::new(node)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("width"))),
        "{dependencies:?}"
    );
}

#[test]
fn first_selection_matches_normalized_inline_table_and_flex_boundaries() {
    for (label, css, html, expected) in [
        (
            "empty inline",
            "body{page:A}p{page:B}",
            "<span></span><p>Body</p>",
            [600.0, 700.0],
        ),
        (
            "split trailing edge",
            "body{page:A}span{padding-right:1pt}b{display:block;page:B}",
            "<span><b>Body</b></span>",
            [600.0, 700.0],
        ),
        (
            "split leading edge",
            "body{page:A}span{padding-left:1pt}b{display:block;page:B}",
            "<span><b>Body</b></span>",
            [400.0, 500.0],
        ),
        (
            "anonymous cell",
            "body{page:A}.cell{display:table-cell;page:B}",
            "<div class=cell>Cell</div>",
            [400.0, 500.0],
        ),
        (
            "table header",
            "body{page:A}thead{page:B}",
            "<table><tfoot><tr><td>Footer</td></tr></tfoot><tbody><tr><td>Body</td></tr></tbody><thead><tr><td>Header</td></tr></thead></table>",
            [600.0, 700.0],
        ),
        (
            "flex whitespace",
            "body{display:flex;page:A;white-space:pre}p{page:B}",
            "   <p>Body</p>",
            [600.0, 700.0],
        ),
        (
            "unused descendant viewport",
            "body{page:if(style(width:100pt):A;else:B);width:10vw}p{page:A}",
            "<p>First</p>",
            [400.0, 500.0],
        ),
        (
            "later display",
            "p{page:A}.later{display:if(style(width:36pt):none;else:block);width:10vw}",
            "<p>First</p><div class=later>Later</div>",
            [400.0, 500.0],
        ),
    ] {
        let document = parse(css, html).unwrap_or_else(|error| panic!("{label}: {error}"));
        assert_eq!(document.styles().page, expected, "{label}");
    }
}

#[test]
fn page_name_independent_geometry_can_supply_a_viewport_during_selection() {
    let document = StyledDocument::parse(
        &format!("<!doctype html><style>{PROFILE}@page{{size:200pt 300pt;margin:10pt}}span{{padding-left:10vw}}p{{page:if(style(width:100pt):A;else:B);width:10vw}}</style><span></span><p>Body</p>"),
        &local_options().base, &[], &FileResolver::default(),
    ).unwrap();
    assert_eq!(document.styles().page, [200.0, 300.0]);
}

#[test]
fn inside_list_markers_precede_named_child_blocks() {
    for (kind, expected) in [("decimal", [400.0, 500.0]), ("none", [600.0, 700.0])] {
        let document = parse(
            &format!(
                "ol{{page:A}}li{{list-style-position:inside;list-style-type:{kind}}}p{{page:B}}"
            ),
            "<ol><li><p>Body</p></li></ol>",
        )
        .unwrap();
        assert_eq!(document.styles().page, expected, "list-style-type:{kind}");
    }
}

#[test]
fn unrelated_parent_alignment_does_not_block_header_cell_page_selection() {
    let document = parse(
        "table,tbody,tr,th{display:block}tr{width:10vw;text-align:if(style(width:100pt):center;else:start)}th{page:B}",
        "<table><tr><th>Header</th></tr></table>",
    ).unwrap();
    assert_eq!(document.styles().page, [600.0, 700.0]);
}

#[test]
fn preceding_positioned_content_updates_quote_depth_before_first_flow_selection() {
    let document = parse(
        "body{page:A;quotes:'' '' '[' ']'} .phantom::before{content:no-open-quote}q{display:contents}q>span{display:block;page:B}",
        "<div style='position:absolute'><span class=phantom></span></div><q><span>Body</span></q>",
    ).unwrap();
    assert_eq!(document.root().page_boundary().start.as_deref(), Some("A"));
    assert_eq!(document.styles().page, [400.0, 500.0]);
}

#[test]
fn reordered_flex_first_item_keeps_quote_depth_from_document_order() {
    for (prefix, expected) in [(true, [400.0, 500.0]), (false, [600.0, 700.0])] {
        let phantom = "<aside><span class=phantom></span></aside>";
        let selected = "<main><q><span>Body</span></q></main>";
        let html = if prefix {
            format!("{phantom}{selected}")
        } else {
            format!("{selected}{phantom}")
        };
        let document = parse(
            "body{display:flex;quotes:'' '' '[' ']'}aside{order:1}.phantom::before{content:no-open-quote}main{order:-1;page:A}q{display:contents}q>span{display:block;page:B}",
            &html,
        ).unwrap();
        assert_eq!(
            document.styles().page,
            expected,
            "quote prefix before selected flex item: {prefix}"
        );
        let expected_name = if prefix { "A" } else { "B" };
        assert_eq!(
            document.root().page_boundary().start.as_deref(),
            Some(expected_name),
            "selector and final generated tree must agree"
        );
    }
}
