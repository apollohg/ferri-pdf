use crate::dom::{DocumentTree, Selector};
use crate::style::{Page, PageRequest, PageSide, Style, Stylesheet, StylesheetSource};
use ferri_pdf_resources::FileResolver;

const TOLERANCE_PT: f32 = 0.001;
const CONTENT_LIMIT: usize = 1024;

fn sheet(css: &str) -> Stylesheet {
    Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap()
}

fn request(name: Option<&str>, side: PageSide, first: bool, blank: bool) -> PageRequest {
    PageRequest {
        name: name.map(Into::into),
        side,
        first,
        blank,
    }
}

fn assert_pt(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= TOLERANCE_PT,
        "expected {expected}pt, got {actual}pt"
    );
}

#[test]
fn selected_rules_resolve_geometry_production_and_margin_content_from_one_cascade() {
    let sheet = sheet(
        "@page{size:200pt 300pt;margin:20pt;font-size:10pt;@top-center{content:'Default'}}@page A{size:300pt 400pt;@top-center{content:'A ' counter(page)}}@page B{size:500pt 300pt;margin:10pt;padding:1vw;bleed:calc(1vw + 1pt);marks:crop;@top-center{content:'B ' counter(page) ' of ' counter(pages)}}",
    );
    let resolver = FileResolver::default();
    let root = Style::default();
    let snapshot = sheet.page;
    let first = sheet
        .resolve_page(
            &root,
            request(Some("A"), PageSide::Right, true, false),
            None,
            &resolver,
        )
        .unwrap();
    assert_eq!(first.geometry.paper, [300.0, 400.0]);
    assert_eq!(first.geometry.area, [260.0, 360.0]);
    assert_eq!(first.geometry.margin, [20.0; 4]);
    assert_eq!(first.margins.len(), 1);
    assert_eq!(
        first.margins[0]
            .content
            .page_text(1, 3, CONTENT_LIMIT)
            .unwrap(),
        "A 1"
    );
    let second = sheet
        .resolve_page(
            &root,
            request(Some("B"), PageSide::Left, false, false),
            Some(first.geometry.area),
            &resolver,
        )
        .unwrap();
    assert_eq!(second.geometry.paper, [500.0, 300.0]);
    assert_pt(second.geometry.padding[0], 2.6);
    assert_pt(second.geometry.content_box().x, 12.6);
    assert_pt(second.geometry.content_box().width, 474.8);
    assert_pt(second.production.bleed, 3.6);
    assert!(second.production.marks.crop);
    assert_eq!(
        second.margins[0]
            .content
            .page_text(2, 3, CONTENT_LIMIT)
            .unwrap(),
        "B 2 of 3"
    );
    assert_eq!(
        sheet.page, snapshot,
        "resolving a later page does not mutate the stylesheet's first/default snapshot"
    );
}

#[test]
fn selector_lists_choose_their_maximum_specificity_and_distinguish_case_blank_and_side() {
    let sheet = sheet(
        r"@page{size:200pt 300pt;margin:20pt}@page cases_page, :first{margin-left:25pt}@page ca\73 es_page:first{margin-left:12pt}@page cases_page:first:first{margin-left:10pt}@page cases_page:right{margin-left:40pt}@page cases_page:left{margin-left:30pt}@page cases_page:blank{margin-left:8pt}@page Cases_Page{margin-left:90pt}@page cases_page:left:right{margin-left:99pt}@page auto{margin-left:98pt}@page AUTO{margin-left:97pt}",
    );
    let resolver = FileResolver::default();
    for (name, side, first, blank, expected) in [
        (Some("cases_page"), PageSide::Right, true, false, 10.0),
        (Some("cases_page"), PageSide::Left, false, false, 30.0),
        (Some("cases_page"), PageSide::Right, false, false, 40.0),
        (Some("cases_page"), PageSide::Right, false, true, 8.0),
        (Some("Cases_Page"), PageSide::Right, true, false, 90.0),
        (None, PageSide::Right, true, false, 25.0),
        (None, PageSide::Left, false, false, 20.0),
        (Some("auto"), PageSide::Right, false, false, 20.0),
    ] {
        let page = sheet
            .resolve_page(
                &Style::default(),
                request(name, side, first, blank),
                None,
                &resolver,
            )
            .unwrap();
        assert_pt(page.geometry.margin[3], expected);
    }
}

#[test]
fn page_candidates_preserve_important_layer_inversion_and_descriptor_rollback() {
    let css = "@layer base,override;@page{margin:20pt}@layer base{@page{size:200pt 300pt}@page chosen{size:300pt 400pt;margin-left:10pt!important;@top-center{content:'Base'!important}}}@layer override{@page chosen{size:500pt 600pt;margin-left:30pt!important;@top-center{content:'Override'!important}}@page chosen{size:revert-layer}}@page chosen{margin-left:40pt!important;@top-center{content:'Unlayered'!important}}";
    let parsed = sheet(css);
    let page = parsed
        .resolve_page(
            &Style::default(),
            request(Some("chosen"), PageSide::Right, true, false),
            None,
            &FileResolver::default(),
        )
        .unwrap();
    assert_eq!(page.geometry.paper, [300.0, 400.0]);
    assert_pt(page.geometry.margin[3], 10.0);
    assert_eq!(
        page.margins[0]
            .content
            .page_text(1, 1, CONTENT_LIMIT)
            .unwrap(),
        "Base"
    );
    let unlayered = sheet(&format!("{css}@page{{size:200pt 300pt}}"));
    let page = unlayered
        .resolve_page(
            &Style::default(),
            request(Some("chosen"), PageSide::Right, true, false),
            None,
            &FileResolver::default(),
        )
        .unwrap();
    assert_eq!(
        page.geometry.paper,
        [200.0, 300.0],
        "normal unlayered declarations outrank layers before selector specificity"
    );
}

#[test]
fn unmatched_named_descriptors_do_not_block_other_page_types() {
    let sheet =
        sheet("@page{size:200pt 300pt;margin:20pt}@page unused{page-orientation:rotate-left}");
    let resolver = FileResolver::default();
    let ordinary = sheet
        .resolve_page(&Style::default(), PageRequest::default(), None, &resolver)
        .unwrap();
    assert_eq!(ordinary.geometry.paper, [200.0, 300.0]);
}

#[test]
fn user_important_page_values_outrank_more_specific_author_rules() {
    let sheet = Stylesheet::parse_with_origins(
        &[StylesheetSource{text:"@page chosen{size:900pt 800pt!important;margin:90pt!important;@top-center{content:'Author'!important}}".into(),base:Default::default()}],
        &[StylesheetSource{text:"@page{size:300pt 400pt!important;margin:10pt!important;@top-center{content:'User'!important}}".into(),base:Default::default()}],
        &Default::default(),
    ).unwrap();
    let page = sheet
        .resolve_page(
            &Style::default(),
            request(Some("chosen"), PageSide::Right, true, false),
            None,
            &FileResolver::default(),
        )
        .unwrap();
    assert_eq!(page.geometry.paper, [300.0, 400.0]);
    assert_eq!(page.geometry.margin, [10.0; 4]);
    assert_eq!(
        page.margins[0]
            .content
            .page_text(1, 1, CONTENT_LIMIT)
            .unwrap(),
        "User"
    );
}

#[test]
fn page_specificity_components_saturate_without_overflowing_into_the_name_component() {
    const EXCESS_PSEUDOS: usize = 4096;
    let sheet = sheet(&format!(
        "@page{{size:200pt 300pt;margin:20pt}}@page {}{{margin-left:99pt}}@page chosen{{margin-left:11pt}}",
        ":first".repeat(EXCESS_PSEUDOS)
    ));
    let page = sheet
        .resolve_page(
            &Style::default(),
            request(Some("chosen"), PageSide::Right, true, false),
            None,
            &FileResolver::default(),
        )
        .unwrap();
    assert_pt(page.geometry.margin[3], 11.0);
}

#[test]
fn the_computed_page_property_keeps_auto_distinct_from_explicit_inheritance() {
    let sheet = sheet(
        "#parent{page:cases_page}#inherited{page:inherit}#unset{page:unset}#variable{--page:other;page:var(--page)}#invalid{page:cases_page;page:auto auto}#missing{page:cases_page;page:var(--missing)}#query{page:cases_page;margin-left:if(style(page:cases_page):10pt;else:90pt)}",
    );
    let tree=DocumentTree::parse("<section id=parent><p id=auto></p><p id=inherited></p><p id=unset></p><p id=variable></p><p id=invalid></p><p id=missing></p><p id=query></p></section>").unwrap();
    let element = |name: &str| {
        tree.select(&Selector::parse(&format!("#{name}")).unwrap())
            .next()
            .unwrap()
    };
    let resolver = FileResolver::default();
    let parent = sheet
        .computed_with_resolver(element("parent"), &Style::default(), &resolver)
        .unwrap();
    assert_eq!(parent.page, Page::Name("cases_page".into()));
    assert_eq!(
        parent.anonymous(crate::style::Display::Block).page,
        Page::Auto
    );
    for (id, expected) in [
        ("auto", Page::Auto),
        ("inherited", parent.page.clone()),
        ("unset", Page::Auto),
        ("variable", Page::Name("other".into())),
        ("invalid", parent.page.clone()),
        ("missing", Page::Auto),
    ] {
        let child = sheet
            .computed_with_resolver(element(id), &parent, &resolver)
            .unwrap();
        assert_eq!(child.page, expected, "computed page for {id}");
    }
    let queried = sheet
        .computed_with_resolver(element("query"), &parent, &resolver)
        .unwrap();
    assert_pt(queried.margin[3].resolve(100.0).unwrap(), 10.0);
}
