mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{
    BorderStyle, BoxPaint, DisplayItem, LayoutTrace, LengthPercentage, Rect, render_html,
};

const TOLERANCE_PT: f32 = 0.001;
/// Profile page margin: the first box's border box starts here.
const PAGE_MARGIN_PT: f32 = 36.0;
const RED: [u8; 3] = [0xff, 0, 0];

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    )
    .expect("outline fixture must render")
    .layout
    .unwrap()
}

/// Box items drawing only a uniform border of `rgb`: the outlines.
fn outlines(items: &[DisplayItem], rgb: [u8; 3]) -> Vec<&BoxPaint> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint)
                if paint.decoration.background.alpha == 0.0
                    && paint
                        .decoration
                        .borders
                        .iter()
                        .all(|border| border.color.rgb == rgb) =>
            {
                Some(paint)
            }
            _ => None,
        })
        .collect()
}

fn assert_rect(label: &str, actual: Rect, expected: Rect) {
    assert!(
        [
            (actual.x, expected.x),
            (actual.y, expected.y),
            (actual.width, expected.width),
            (actual.height, expected.height),
        ]
        .iter()
        .all(|(a, e)| (a - e).abs() < TOLERANCE_PT),
        "{label}: {actual:?}, expected {expected:?}"
    );
}

fn radii(paint: &BoxPaint) -> Vec<[f32; 2]> {
    paint
        .decoration
        .radii
        .iter()
        .map(|corner| {
            corner.clone().map(|radius| match radius {
                LengthPercentage::Length(length) => length,
                other => panic!("outline radii are absolute: {other:?}"),
            })
        })
        .collect()
}

#[test]
fn outlines_surround_the_border_box_by_their_offset_without_taking_space() {
    let css = "div{width:100pt;height:40pt;border:2pt solid black}\
               .o{outline:4pt dashed #ff0000;outline-offset:3pt}";
    let output = trace(css, "<div class=o>A</div><p>After</p>");
    let [outline] = outlines(&output.display_lists[0], RED)[..] else {
        panic!("one outline: {:#?}", output.display_lists[0]);
    };
    assert_rect(
        "the outline's outer edge is offset + width outside the 104pt by 44pt border box",
        outline.rect,
        Rect {
            x: PAGE_MARGIN_PT - 7.0,
            y: PAGE_MARGIN_PT - 7.0,
            width: 104.0 + 14.0,
            height: 44.0 + 14.0,
        },
    );
    for border in &outline.decoration.borders {
        assert_eq!(
            (border.width, border.style),
            (4.0, BorderStyle::Dashed),
            "every side is the outline: {:?}",
            outline.decoration.borders
        );
    }
    let without = trace(
        "div{width:100pt;height:40pt;border:2pt solid black}",
        "<div>A</div><p>After</p>",
    );
    let after = |trace: &LayoutTrace| {
        trace.pages[0]
            .iter()
            .find(|line| line.text == "After")
            .map(|line| (line.x, line.baseline))
    };
    assert_eq!(
        after(&output),
        after(&without),
        "outlines do not influence the position of other boxes"
    );
}

#[test]
fn outlines_follow_grown_border_radii_and_clamp_negative_offsets() {
    let output = trace(
        "div{width:100pt;height:40pt;margin-bottom:20pt}\
         .r{border-radius:20pt 2pt 0 20pt/10pt 2pt 0 10pt;outline:4pt solid #ff0000;outline-offset:3pt}\
         .n{width:10pt;height:10pt;outline:4pt solid #ff0000;outline-offset:-20pt}",
        "<div class=r></div><div class=n></div>",
    );
    let found = outlines(&output.display_lists[0], RED);
    let [rounded, clamped] = found[..] else {
        panic!("two outlines: {found:#?}");
    };
    // Backgrounds 3 §7.1.1 spread: radii at least the growth add it, smaller ones grow by
    // growth × (1 + (r / growth − 1)³), and sharp corners stay sharp.
    let growth = 7.0_f32;
    let small = growth * (1.0 + (2.0 / growth - 1.0).powi(3));
    let expected = [
        [27.0, 17.0],
        [2.0 + small, 2.0 + small],
        [0.0, 0.0],
        [27.0, 17.0],
    ];
    for (corner, (actual, expected)) in radii(rounded).into_iter().zip(expected).enumerate() {
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(a, e)| (a - e).abs() < TOLERANCE_PT),
            "corner {corner}: {actual:?}, expected {expected:?}"
        );
    }
    let top = PAGE_MARGIN_PT + 60.0;
    assert_rect(
        "a large negative offset leaves each outer dimension twice the width, centred",
        clamped.rect,
        Rect {
            x: PAGE_MARGIN_PT + 1.0,
            y: top + 1.0,
            width: 8.0,
            height: 8.0,
        },
    );
}

#[test]
fn outlines_paint_last_in_their_stacking_context_under_the_box_effects() {
    let output = trace(
        "div{height:20pt}.o{outline:2pt solid #ff0000;background:#ffff00}\
         .later{background:#0000ff;margin-top:-10pt}\
         .t{opacity:.5;outline:2pt solid #00ff00;background:#00ffff}",
        "<div class=o>Outlined</div><div class=later>Later</div><div class=t>Group</div><div class=later>Last</div>",
    );
    let labels: Vec<String> = output.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                let [r, g, b] = paint.decoration.background.rgb;
                Some(format!("box #{r:02x}{g:02x}{b:02x}"))
            }
            DisplayItem::Box(paint) => {
                let [r, g, b] = paint.decoration.borders[0].color.rgb;
                Some(format!("outline #{r:02x}{g:02x}{b:02x}"))
            }
            DisplayItem::PushOpacity(_) => Some("group".into()),
            DisplayItem::Pop => Some("pop".into()),
            _ => None,
        })
        .collect();
    assert_eq!(
        labels,
        [
            "box #ffff00",
            "box #0000ff",
            "box #0000ff",
            "group",
            "box #00ffff",
            "outline #00ff00",
            "pop",
            "outline #ff0000",
        ],
        "a flow box's outline follows everything in the root stacking context, overlapping \
         later siblings; a stacking context's own outline is last inside its group"
    );
}

#[test]
fn each_fragment_has_its_own_closed_outline() {
    let output = trace(
        "@page{size:200pt 120pt;margin:20pt}div{height:150pt;outline:1pt solid #ff0000}\
         span{outline:1pt solid #0000ff}p{width:60pt}",
        "<p>One <span>two three four</span></p><div></div>",
    );
    let spans = outlines(&output.display_lists[0], [0, 0, 0xff]);
    assert!(
        spans.len() > 1,
        "the inline box broken across lines has an outline per line fragment: {spans:#?}"
    );
    let pages: Vec<usize> = (0..output.display_lists.len())
        .filter(|&page| !outlines(&output.display_lists[page], RED).is_empty())
        .collect();
    assert!(
        pages.len() > 1,
        "the block split across pages has an outline on each page: {pages:?}"
    );
    for page in pages {
        let [outline] = outlines(&output.display_lists[page], RED)[..] else {
            panic!("one block outline on page {page}");
        };
        assert!(
            outline.first_fragment
                && outline.last_fragment
                && outline
                    .decoration
                    .borders
                    .iter()
                    .all(|border| border.width == 1.0),
            "the outline is fully connected on page {page}, not open at the break: {outline:?}"
        );
    }
}

#[test]
fn outlines_of_rows_page_boxes_and_margin_boxes_paint() {
    let output = trace(
        "@page{outline:3pt solid #ff0000;@top-center{content:'Head';outline:1pt solid #0000ff}}\
         table{border-spacing:0}td{padding:0;width:20pt;height:10pt}\
         tr{outline:2pt solid #00ff00}",
        "<table><tr><td>A</td></tr><tr><td>B</td></tr></table>",
    );
    let list = &output.display_lists[0];
    assert_eq!(
        outlines(list, [0, 0xff, 0]).len(),
        2,
        "each row has an outline: {list:#?}"
    );
    assert_eq!(
        outlines(list, [0, 0, 0xff]).len(),
        1,
        "the margin box outline"
    );
    let page = outlines(list, RED);
    let [page_outline] = page[..] else {
        panic!("one page outline: {page:#?}");
    };
    assert!(
        matches!(list.last(), Some(DisplayItem::Box(last)) if std::ptr::eq(last, page_outline)),
        "the page outline paints over the page content and margin boxes"
    );
}
