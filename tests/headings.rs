use ferri_pdf::{RenderOptions, render_html};

const CSS: &str = "@page{size:200pt 200pt;margin:10pt}html,body,h1,h2,h3,p{margin:0;padding:0;font:10pt/12pt serif}";

#[test]
fn heading_locations_join_destinations_and_do_not_require_layout_trace_or_pdf_bookmarks() {
    let html = format!(
        "<style>{CSS}#second{{break-before:page;bookmark-level:none}}.hidden{{display:none}}</style><a href='#second'></a><h1 id=first> First <em>heading</em> </h1><h1 class=hidden>Hidden</h1><h2 id=second>Second heading</h2><p style='bookmark-level:1'>Custom bookmark</p>"
    );
    let compact = render_html(&html, &RenderOptions::default()).unwrap();
    assert!(compact.layout.is_none());
    assert_eq!(
        compact.headings.len(),
        2,
        "visible HTML headings are independent of CSS bookmark suppression and non-heading bookmarks"
    );
    for (heading, id, level, text, page) in [
        (&compact.headings[0], "first", 1, "First heading", 1),
        (&compact.headings[1], "second", 2, "Second heading", 2),
    ] {
        assert_eq!(heading.id.as_deref(), Some(id));
        assert_eq!(heading.level, level);
        assert_eq!(heading.text, text);
        assert_eq!(
            heading.page, page,
            "heading destinations are joined by index rather than zipped with discovery order"
        );
        assert!(
            (heading.top - 10.0).abs() < 0.01,
            "heading top is in y-down PDF points: {heading:?}"
        );
    }
    let traced = render_html(
        &html,
        &RenderOptions {
            include_layout: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        traced.headings, compact.headings,
        "diagnostic retention cannot affect the compact heading map"
    );
}

#[test]
fn heading_locations_keep_html_levels_and_first_fragments_of_split_headings() {
    let text = std::iter::repeat_n("Heading", 180)
        .collect::<Vec<_>>()
        .join(" ");
    let html = format!(
        "<style>{CSS}@page{{size:150pt 90pt}}h1,h2{{break-inside:auto;break-after:auto}}h2{{break-before:page}}</style><h1>{text}</h1><section headingoffset=2><h2 id=last>Final heading</h2></section>"
    );
    let output = render_html(&html, &RenderOptions::default()).unwrap();
    assert!(
        output.page_count > 2,
        "the first heading must really fragment"
    );
    assert_eq!(
        output.headings.len(),
        2,
        "one record per heading, not one per page fragment"
    );
    assert_eq!(output.headings[0].text, text);
    assert_eq!(output.headings[0].id, None);
    assert_eq!(output.headings[0].page, 1);
    assert_eq!(
        output.headings[1].level, 4,
        "the existing HTML heading-level algorithm supplies semantic levels"
    );
    assert_eq!(output.headings[1].page, output.page_count);
}
