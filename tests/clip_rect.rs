mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{DisplayItem, LayoutTrace, Rect, render_html};

/// Profile page margin: the first box's border box starts here.
const PAGE_MARGIN_PT: f32 = 36.0;

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    )
    .expect("clip fixture must render")
    .layout
    .unwrap()
}

fn clips(items: &[DisplayItem]) -> Vec<Rect> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::PushClip(clip) => Some(clip.rect),
            _ => None,
        })
        .collect()
}

#[test]
fn clip_rect_offsets_from_the_border_box_clip_absolutely_positioned_boxes() {
    let css = "div{width:100pt;height:40pt;padding:5pt;background:teal}\
               .a{position:absolute;top:0;left:0;clip:rect(5pt, 60pt, auto, 10pt)}\
               .f{position:fixed;top:100pt;left:0;clip:rect(auto,auto,20pt,auto)}\
               .s{clip:rect(5pt, 60pt, auto, 10pt)}";
    let output = trace(
        css,
        "<div class=a>Abs<p>Child</p></div><div class=f>Fixed</div><div class=s>Static</div>",
    );
    let list = &output.display_lists[0];
    assert_eq!(
        clips(list),
        [
            Rect {
                x: PAGE_MARGIN_PT + 10.0,
                y: PAGE_MARGIN_PT + 5.0,
                width: 50.0,
                height: 45.0,
            },
            Rect {
                x: PAGE_MARGIN_PT,
                y: PAGE_MARGIN_PT + 100.0,
                width: 110.0,
                height: 20.0,
            },
        ],
        "offsets are from the border box's top-left corner and auto is the matching border edge; \
         clip does not apply to the static box"
    );
    let open = list
        .iter()
        .position(|item| matches!(item, DisplayItem::PushClip(_)))
        .expect("the absolute box is clipped");
    assert!(
        matches!(list[open + 1], DisplayItem::Box(_)),
        "the clip encloses the box's own background: {:?}",
        &list[open..]
    );
    let child = output.pages[0]
        .iter()
        .position(|line| line.text == "Child")
        .expect("the child line is laid out");
    let close = list[open..]
        .iter()
        .position(|item| matches!(item, DisplayItem::Pop))
        .map(|index| open + index)
        .expect("the clip closes");
    assert!(
        list[open..close]
            .iter()
            .any(|item| matches!(item, DisplayItem::Text { line } if *line == child)),
        "descendants are clipped with the box"
    );
}
