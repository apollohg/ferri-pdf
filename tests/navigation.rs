mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{Destination, LayoutTrace, LinkArea, LinkShape, Rect, render_html};

const TOLERANCE_PT: f32 = 0.01;
/// Profile page margin: the first box's border box starts here.
const PAGE_MARGIN_PT: f32 = 36.0;
/// Profile line height.
const LINE_PT: f32 = 18.0;

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    )
    .expect("navigation fixture must render")
    .layout
    .unwrap()
}

fn rects(areas: &[LinkArea], link: usize) -> Vec<Rect> {
    areas
        .iter()
        .filter(|area| area.link == link)
        .map(|area| match area.shape {
            LinkShape::Rect(rect) => rect,
            LinkShape::Quads(_) => {
                panic!("untransformed link {link} has quadrilaterals: {areas:?}")
            }
        })
        .collect()
}

fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < TOLERANCE_PT
}

#[test]
fn inline_links_cover_each_line_fragment_and_atomic_inlines_beyond_it() {
    let trace = trace(
        "p{width:150pt}.ib{display:inline-block;width:20pt;height:30pt;background:#9cf}",
        "<p>a <a href='https://wrap.example/'>link whose words wrap across lines</a></p>\
         <p><a href='https://atomic.example/'>x <span class=ib></span> <span>inner</span></a></p>",
    );
    let areas = &trace.links[0];
    let wrapped = rects(areas, 0);
    assert_eq!(wrapped.len(), 2, "one area per line fragment: {areas:?}");
    assert!(
        close(wrapped[1].y - wrapped[0].y, LINE_PT) && close(wrapped[1].x, PAGE_MARGIN_PT),
        "the second fragment starts the next line at the left edge: {wrapped:?}"
    );
    let atomic = rects(areas, 1);
    assert_eq!(
        atomic.len(),
        2,
        "the inner span's fragment lies inside the link's and is dropped; the taller atomic is kept: {atomic:?}"
    );
    assert!(
        close(atomic[1].width, 20.0) && close(atomic[1].height, 30.0),
        "the atomic inline's area is its border box: {atomic:?}"
    );
    assert!(
        atomic[1].x > atomic[0].x && atomic[1].x + atomic[1].width < atomic[0].x + atomic[0].width,
        "the atomic sits inside the line fragment horizontally: {atomic:?}"
    );
}

#[test]
fn block_links_cover_every_page_fragment_of_their_border_box() {
    let trace = trace(
        ".tall{display:block;height:600pt;padding:4pt;border:1pt solid navy;margin-left:10pt}",
        "<a class=tall href='https://block.example/'>Block</a>",
    );
    let pages: Vec<Vec<Rect>> = trace.links.iter().map(|areas| rects(areas, 0)).collect();
    assert_eq!(pages.len(), 2, "{pages:?}");
    let [first] = pages[0][..] else {
        panic!("one area on page 1: {pages:?}")
    };
    let [second] = pages[1][..] else {
        panic!("one area on page 2: {pages:?}")
    };
    let content_width = 360.0 - 2.0 * PAGE_MARGIN_PT;
    assert!(
        close(first.x, PAGE_MARGIN_PT + 10.0)
            && close(first.width, content_width - 10.0)
            && close(first.y, PAGE_MARGIN_PT),
        "the first fragment starts at the top of page 1 inside the left margin: {first:?}"
    );
    assert!(
        close(second.y, PAGE_MARGIN_PT) && close(first.height + second.height, 600.0 + 2.0 * 5.0),
        "the fragments together are the border box: {second:?}"
    );
}

#[test]
fn destinations_locate_the_first_fragment_of_their_element() {
    let trace = trace(
        "#rotated{transform:rotate(90deg);width:100pt;height:20pt;margin-top:10pt}",
        "<p><a href='#later'>later</a> <a href='#hidden'>hidden</a> <a href='#word'>word</a> <a href='#rotated'>rotated</a></p>\
         <div id=hidden style='display:none'>x</div>\
         <div id=later class=new-page style='padding-top:5pt'>Later <span id=word>word</span></div>\
         <div id=rotated></div>",
    );
    let [later, hidden, word, rotated] = trace.destinations[..] else {
        panic!("four destinations: {:?}", trace.destinations)
    };
    assert_eq!(
        later,
        Some(Destination {
            page: 1,
            top: PAGE_MARGIN_PT
        }),
        "the forced break puts the target's border box at the top of page 2"
    );
    assert_eq!(hidden, None, "a display:none target has no box");
    let word = word.expect("the inline target has a fragment");
    assert!(
        word.page == 1
            && word.top > PAGE_MARGIN_PT + 5.0
            && word.top < PAGE_MARGIN_PT + 5.0 + LINE_PT,
        "an inline target's top is its content area within the first line: {word:?}"
    );
    let rotated = rotated.expect("the rotated target has a box");
    // A quarter turn about the 100×20 box's centre raises its top by (100 − 20) / 2.
    let untransformed = PAGE_MARGIN_PT + 5.0 + LINE_PT + 10.0;
    assert!(
        rotated.page == 1 && close(rotated.top, untransformed - 40.0),
        "a transformed target's top is that of its transformed border box: {rotated:?}"
    );
}

#[test]
fn page_margin_box_transforms_move_and_rotate_their_running_links() {
    const SHIFT_PT: f32 = 100.0;
    let margin = |transform: &str| {
        trace(
            &format!(
                ".head{{position:running(head)}}\
                 @page{{@top-left{{content:element(head);transform:{transform}}}}}"
            ),
            "<div class=head><a href='https://head.example/'>Head</a></div><p>Body</p>",
        )
        .links[0]
            .clone()
    };
    let [plain] = rects(&margin("none"), 0)[..] else {
        panic!("one untransformed margin link: {:?}", margin("none"))
    };
    let shifted_areas = margin(&format!("translateY({SHIFT_PT}pt)"));
    let [shifted] = rects(&shifted_areas, 0)[..] else {
        panic!("one translated margin link: {shifted_areas:?}")
    };
    assert!(
        close(shifted.x, plain.x)
            && close(shifted.y, plain.y + SHIFT_PT)
            && close(shifted.width, plain.width),
        "the margin box's translation moves its link: {plain:?} -> {shifted:?}"
    );
    let rotated = margin("rotate(30deg);transform-origin:0 0");
    assert!(
        matches!(
            &rotated[..],
            [LinkArea {
                link: 0,
                shape: LinkShape::Quads(_)
            }]
        ),
        "a rotated margin box's link becomes a quadrilateral: {rotated:?}"
    );
}

#[test]
fn page_box_transforms_move_the_links_and_destinations_of_their_pages() {
    const SHIFT_PT: f32 = 100.0;
    let page = |transform: &str| {
        trace(
            &format!("@page{{transform:{transform}}}"),
            "<p><a href='#target'>Link</a></p><div id=target>Target</div>",
        )
    };
    let plain = page("none");
    let shifted = page(&format!("translateY({SHIFT_PT}pt)"));
    let ([plain_link], [shifted_link]) = (
        &rects(&plain.links[0], 0)[..],
        &rects(&shifted.links[0], 0)[..],
    ) else {
        panic!(
            "one link area per page: {:?} / {:?}",
            plain.links, shifted.links
        )
    };
    assert!(
        close(shifted_link.x, plain_link.x) && close(shifted_link.y, plain_link.y + SHIFT_PT),
        "the page box's translation moves its links: {plain_link:?} -> {shifted_link:?}"
    );
    let (Some(target), Some(moved)) = (plain.destinations[0], shifted.destinations[0]) else {
        panic!(
            "the target has a box: {:?} / {:?}",
            plain.destinations, shifted.destinations
        )
    };
    assert!(
        moved.page == target.page && close(moved.top, target.top + SHIFT_PT),
        "the page box's translation moves its destinations: {target:?} -> {moved:?}"
    );
    let rotated = page("rotate(30deg)");
    assert!(
        matches!(
            &rotated.links[0][..],
            [LinkArea {
                link: 0,
                shape: LinkShape::Quads(_)
            }]
        ),
        "a rotated page box's link becomes a quadrilateral: {:?}",
        rotated.links
    );
}

#[test]
fn transformed_links_become_quadrilaterals_and_clips_trim_link_areas() {
    let trace = trace(
        ".rot{transform:rotate(30deg);width:150pt;margin:40pt 0}\
         .scale{transform:scale(2);transform-origin:0 0;width:100pt;height:40pt}\
         .clip{width:60pt;overflow:hidden;white-space:nowrap}",
        "<div class=rot><a href='https://rotated.example/'>rotated</a></div>\
         <div class=scale><a href='https://scaled.example/'>scaled</a></div>\
         <div class=clip><a href='https://clipped.example/'>clipped link with long text</a></div>",
    );
    let areas = &trace.links[0];
    let rotated = areas.iter().find(|area| area.link == 0).unwrap();
    let LinkShape::Quads(quads) = &rotated.shape else {
        panic!("a rotated link needs quadrilaterals: {rotated:?}")
    };
    let [[top_left, bottom_left, _, top_right]] = quads[..] else {
        panic!("one quadrilateral: {quads:?}")
    };
    let angle = (top_right[1] - top_left[1]).atan2(top_right[0] - top_left[0]);
    assert!(
        close(angle.to_degrees(), 30.0) && bottom_left[1] > top_left[1],
        "the quadrilateral follows the rotated fragment from its top-left corner: {quads:?}"
    );
    let [scaled] = rects(areas, 1)[..] else {
        panic!("one scaled area: {areas:?}")
    };
    assert!(
        close(scaled.x, PAGE_MARGIN_PT),
        "scaling about the left edge keeps the link at the margin: {scaled:?}"
    );
    let [clipped] = rects(areas, 2)[..] else {
        panic!("one clipped area: {areas:?}")
    };
    assert!(
        close(clipped.x, PAGE_MARGIN_PT) && close(clipped.width, 60.0),
        "the overflow clip trims the link to the container: {clipped:?}"
    );
    let [plain] = rects(
        &self::trace("", "<a href='https://scaled.example/'>scaled</a>").links[0],
        0,
    )[..] else {
        panic!("one plain area")
    };
    assert!(
        close(scaled.width, 2.0 * plain.width) && close(scaled.height, 2.0 * plain.height),
        "scale(2) doubles the area: {scaled:?} against {plain:?}"
    );
}

#[test]
fn table_rows_and_cells_locate_their_destinations() {
    let trace = trace(
        "table{border-spacing:0}td{padding:3pt}",
        "<p><a href='#row'>row</a> <a href='#cell'>cell</a></p>\
         <table><tr><td>A</td></tr><tr id=row><td>B</td><td id=cell>C</td></tr></table>",
    );
    let [row, cell] = trace.destinations[..] else {
        panic!("two destinations: {:?}", trace.destinations)
    };
    let (row, cell) = (
        row.expect("row has an area"),
        cell.expect("cell has an area"),
    );
    assert!(
        close(row.top, cell.top) && row.top > PAGE_MARGIN_PT + LINE_PT,
        "the second row and its cell share a top below the first row: {row:?} {cell:?}"
    );
}

#[test]
fn shadow_hyperlinks_reach_pdf_annotations_and_document_destinations() {
    let trace = trace(
        "",
        "<div><template shadowrootmode=closed>\
         <a href='https://shadow.example/'>Shadow</a> <a href='#target'>Jump</a>\
         <div><template shadowrootmode=open>\
         <a href='https://nested.example/'>Nested</a><slot></slot>\
         </template><a href='https://projected.example/'>Projected</a></div>\
         </template></div><p id=target class=new-page>Document target</p>",
    );
    for link in [0, 1, 2, 3] {
        let areas: Vec<_> = trace
            .links
            .iter()
            .flatten()
            .filter(|area| area.link == link)
            .collect();
        assert!(
            !areas.is_empty(),
            "shadow or projected hyperlink {link} must have a visible PDF annotation: {:?}",
            trace.links
        );
    }
    assert_eq!(
        trace.destinations,
        [Some(Destination {
            page: 1,
            top: PAGE_MARGIN_PT
        })],
        "a shadow hyperlink can reach the document target on its actual PDF page"
    );
}
