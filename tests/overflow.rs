mod common;
use common::{PROFILE, local_options as options};
use ferri_pdf::{Clip, DisplayItem, LayoutTrace, Rect, render_html};

const TOLERANCE_PT: f32 = 0.001;
const FAR_OVERFLOW_PT: f32 = 1_100_000.0;

fn trace(css: &str, body: &str) -> LayoutTrace {
    let html = format!("<style>{PROFILE}{css}</style>{body}");
    render_html(&html, &options())
        .expect("overflow fixture must render")
        .layout
        .unwrap()
}
fn clips(items: &[DisplayItem]) -> Vec<Clip> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::PushClip(clip) => Some(*clip),
            _ => None,
        })
        .collect()
}
fn assert_rect(label: &str, actual: Rect, expected: Rect) {
    let pairs = [
        (actual.x, expected.x),
        (actual.y, expected.y),
        (actual.width, expected.width),
        (actual.height, expected.height),
    ];
    assert!(
        pairs.iter().all(|(a, e)| (a - e).abs() < TOLERANCE_PT),
        "{label}: {actual:?}, expected {expected:?}"
    );
}

#[test]
fn hidden_overflow_clips_descendants_to_the_rounded_padding_box() {
    const EXPECTED_PADDING_BOX: Rect = Rect {
        x: 39.0,
        y: 39.0,
        width: 108.0,
        height: 48.0,
    };
    const EXPECTED_INNER_RADIUS_PT: f32 = 13.0;
    let trace = trace(
        "div{width:100pt;height:40pt;overflow:hidden;padding:4pt;border:3pt solid navy;border-radius:16pt}",
        "<div><p style='background:#9cf;margin:0'>One two three four five six seven eight nine ten eleven</p></div>",
    );
    let items = &trace.display_lists[0];
    let push = items
        .iter()
        .position(|item| matches!(item, DisplayItem::PushClip(_)))
        .expect("descendants must be clipped");
    assert!(
        matches!(items[push - 1], DisplayItem::Box(_)),
        "the overflow box paints its own border outside its clip: {items:?}"
    );
    let clip = clips(items)[0];
    assert_rect("clip", clip.rect, EXPECTED_PADDING_BOX);
    assert!(
        clip.radii
            .iter()
            .flatten()
            .all(|r| (r - EXPECTED_INNER_RADIUS_PT).abs() < TOLERANCE_PT),
        "inner radii {:?}",
        clip.radii
    );
    let pushes = clips(items).len();
    let pops = items
        .iter()
        .filter(|item| matches!(item, DisplayItem::Pop))
        .count();
    assert_eq!(pushes, pops, "every clip is closed: {items:?}");
    let drawn: Vec<&str> = items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Text { line } => Some(trace.pages[0][*line].text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !drawn.iter().any(|text| text.contains("eleven")),
        "lines whose ink misses the clip are omitted: {drawn:?}"
    );
}

#[test]
fn overflow_axes_pair_and_clip_as_css_overflow_requires() {
    let trace = trace(
        "div{width:120pt;height:20pt;margin-bottom:40pt}.x{overflow-x:hidden}.y{overflow-y:hidden}.c{overflow:clip visible}.a{overflow:auto}",
        "<div class='x'>X</div><div class='y'>Y</div><div class='c'>C</div><div class='a'>A</div>",
    );
    let clips = clips(&trace.display_lists[0]);
    assert_eq!(clips.len(), 3, "auto prints its overflow: {clips:?}");
    for (clip, open) in clips
        .iter()
        .zip([[false, false], [true, false], [false, true]])
    {
        for offset in [-FAR_OVERFLOW_PT, FAR_OVERFLOW_PT] {
            let point = Rect {
                x: clip.rect.x + 1.0,
                y: clip.rect.y + 1.0,
                width: 1.0,
                height: 1.0,
            };
            assert!(
                clip.bounds().overlaps(point),
                "the padding box remains inside the clip: {clip:?}"
            );
            assert_eq!(
                clip.bounds().overlaps(Rect {
                    x: point.x + offset,
                    ..point
                }),
                open[0],
                "horizontal overflow at offset {offset}: {clip:?}"
            );
            assert_eq!(
                clip.bounds().overlaps(Rect {
                    y: point.y + offset,
                    ..point
                }),
                open[1],
                "vertical overflow at offset {offset}: {clip:?}"
            );
        }
    }
}

#[test]
fn hidden_overflow_contains_child_margins_but_clip_does_not() {
    const HIDDEN_BOX: Rect = Rect {
        x: 36.0,
        y: 94.0,
        width: 288.0,
        height: 58.0,
    };
    const CLIP_BOX_TOP_PT: f32 = 172.0;
    const CLIP_BOX_HEIGHT_PT: f32 = 18.0;
    let trace = trace(
        "div{overflow:hidden;background:#fea}div.clip{overflow:clip;background:#cef}p{margin:20pt 0}",
        "<p>Before</p><div><p>Inside</p></div><div class='clip'><p>Clipped</p></div><p>After</p>",
    );
    let boxes: Vec<Rect> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) => Some(paint.rect),
            _ => None,
        })
        .collect();
    assert_rect("hidden box keeps both child margins", boxes[0], HIDDEN_BOX);
    assert!(
        (boxes[1].y - CLIP_BOX_TOP_PT).abs() < TOLERANCE_PT
            && (boxes[1].height - CLIP_BOX_HEIGHT_PT).abs() < TOLERANCE_PT,
        "overflow:clip lets child margins collapse through: {:?}",
        boxes[1]
    );
}

#[test]
fn rounded_images_are_trimmed_to_the_content_edge_curve() {
    const CONTENT_BOX: Rect = Rect {
        x: 43.0,
        y: 43.0,
        width: 80.0,
        height: 60.0,
    };
    const CONTENT_RADIUS_PT: f32 = 13.0;
    let trace = trace(
        "img{width:80pt;height:60pt;border-radius:20pt;border:3pt solid navy;padding:4pt;display:block}",
        "<img src='../assets/image-pattern.png'>",
    );
    let clip = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Image(paint) => paint.clip,
            _ => None,
        })
        .expect("rounded replaced content must be clipped");
    assert_rect("content box", clip.rect, CONTENT_BOX);
    assert!(
        clip.radii
            .iter()
            .flatten()
            .all(|r| (r - CONTENT_RADIUS_PT).abs() < TOLERANCE_PT),
        "content-edge radii are the outer radii less border and padding: {:?}",
        clip.radii
    );
}
