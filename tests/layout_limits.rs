mod common;

use common::local_options;
use ferri_pdf::{DisplayItem, LayoutDiagnostic, RenderDiagnostic, render_html};

/// The page limit `ferri-pdf` lays out before omitting the rest of a document.
const PAGE_LIMIT: usize = 10_000;

#[test]
fn content_past_the_page_limit_is_omitted_after_the_last_page() {
    const PAGE_HEIGHT_PT: f32 = 20.0;
    let output = render_html(
        &format!(
            "<!doctype html><style>@page{{size:100pt {PAGE_HEIGHT_PT}pt;margin:0}}html,body{{margin:0}}\
             div{{height:1e9pt;background:rgb(0,0,255)}}</style><div></div><p>after</p>"
        ),
        &local_options(),
    )
    .expect("a box taller than the page limit renders the pages it fills");
    assert_eq!(
        output.page_count, PAGE_LIMIT,
        "every page up to the limit is kept"
    );
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            RenderDiagnostic::Layout(LayoutDiagnostic::PageLimit { limit }) if *limit == PAGE_LIMIT
        )),
        "the omitted content is reported, got {:?}",
        output.diagnostics
    );
    let trace = output.layout.unwrap();
    for (page, items) in [0, PAGE_LIMIT - 1].map(|page| (page, &trace.display_lists[page])) {
        let fragment = items.iter().find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == [0, 0, 255] => {
                Some(paint.rect)
            }
            _ => None,
        });
        assert!(
            fragment.is_some_and(|rect| rect.y <= 0.0 && rect.height >= PAGE_HEIGHT_PT),
            "page {} holds a full fragment of the tall box, got {fragment:?}",
            page + 1
        );
    }
    assert!(
        trace
            .pages
            .iter()
            .flatten()
            .all(|line| line.text != "after"),
        "the paragraph after the box lies past the last page"
    );
}
