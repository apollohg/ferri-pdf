use crate::dom::{DocumentTree, Selector};
use crate::resources::FileResolver;
use crate::style::{Page, PageSide, PageStyleTarget, StyleError, Stylesheet, StylesheetSource};

fn sheet(css: &str) -> Stylesheet {
    Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap()
}

#[test]
fn selected_geometry_and_tree_attribute_custom_values_share_the_dependency_graph() {
    let tree =
        DocumentTree::parse("<body><section data-name=A><p>Text</p></section></body>").unwrap();
    let selected = tree.select(&Selector::parse("p").unwrap()).next().unwrap();
    let mut sheet = sheet(
        "@page{size:200pt 300pt;margin:10pt}@page A{size:400pt 500pt;margin:20pt}html{padding-left:10vw}section{--name:attr(data-name <custom-ident>)}p{page:var(--name)}",
    );
    let root = sheet
        .resolve_document_root_with_pages(
            &tree,
            &FileResolver::default(),
            &|styles| {
                let Page::Name(name) = styles.page(PageStyleTarget {
                    element: selected.id(),
                    pseudo: None,
                })?
                else {
                    panic!("the inherited attribute value must retain its section receiver");
                };
                Ok(Some(name))
            },
            &|_| Ok(PageSide::Right),
        )
        .unwrap();
    assert_eq!(sheet.page, [400.0, 500.0]);
    assert_eq!(root.padding[3].resolve(0.0), 36.0);
}

#[test]
fn selected_page_cycle_records_the_named_property_and_viewport_request() {
    let tree = DocumentTree::parse("<p>Text</p>").unwrap();
    let selected = tree.select(&Selector::parse("p").unwrap()).next().unwrap();
    let mut sheet =
        sheet("@page A{size:400pt 500pt}p{page:if(style(width:100pt):A;else:B);width:10vw}");
    let error = sheet
        .resolve_document_root_with_pages(
            &tree,
            &FileResolver::default(),
            &|styles| match styles.page(PageStyleTarget {
                element: selected.id(),
                pseudo: None,
            })? {
                Page::Name(name) => Ok(Some(name)),
                Page::Auto => Ok(None),
            },
            &|_| Ok(PageSide::Right),
        )
        .unwrap_err();
    let StyleError::DependencyCycle {
        sources,
        dependencies,
    } = error
    else {
        panic!("{error}");
    };
    assert_ne!(sources, [] as [crate::style::SourceLocation; 0]);
    assert!(
        dependencies
            .iter()
            .any(|dependency| dependency == "first-page-start"),
        "{dependencies:?}"
    );
    assert!(
        dependencies
            .iter()
            .any(|dependency| std::path::Path::new(dependency)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("page"))),
        "{dependencies:?}"
    );
    assert!(
        dependencies
            .iter()
            .any(|dependency| std::path::Path::new(dependency)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("width"))),
        "{dependencies:?}"
    );
}

#[test]
fn selecting_a_deep_target_prepares_ancestors_without_recursive_stack_growth() {
    const ANCESTORS: usize = 1_024;
    let html = format!(
        "{}<p>Text</p>{}",
        "<section>".repeat(ANCESTORS),
        "</section>".repeat(ANCESTORS)
    );
    let tree = DocumentTree::parse(&html).unwrap();
    let selected = tree.select(&Selector::parse("p").unwrap()).next().unwrap();
    let mut sheet = sheet(
        "@page A{size:400pt 500pt}html{white-space:pre-wrap;--page-name:A}p{page:var(--page-name)}",
    );
    sheet
        .resolve_document_root_with_pages(
            &tree,
            &FileResolver::default(),
            &|styles| {
                let target = PageStyleTarget {
                    element: selected.id(),
                    pseudo: None,
                };
                assert_eq!(
                    styles.white_space_collapse(target)?,
                    crate::style::WhiteSpaceCollapse::Preserve,
                    "deep inherited whitespace retains the root declaration"
                );
                match styles.page(target)? {
                    Page::Name(name) => Ok(Some(name)),
                    Page::Auto => panic!("deep target lost its explicit page name"),
                }
            },
            &|_| Ok(PageSide::Right),
        )
        .unwrap();
    assert_eq!(sheet.page, [400.0, 500.0]);
}
