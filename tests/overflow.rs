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
fn clips(items: &[DisplayItem]) -> Vec<&Clip> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::PushClip(clip) => Some(clip),
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
            DisplayItem::Image(paint) => paint.clip.as_ref(),
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

/// CSS Overflow 3 §3.2: `overflow: clip` and paint containment clip at the overflow clip
/// edge, the chosen box edge expanded by the margin with corners shaped like a box-shadow
/// spread from the border edge; scroll containers keep clipping at the padding edge.
#[test]
fn overflow_clip_margin_moves_only_the_overflow_clip_edge() {
    // The border box is 36..150 by 36..90 with 3pt borders and 4pt padding.
    const BOX: &str =
        "div{width:100pt;height:40pt;padding:4pt;border:3pt solid navy;border-radius:16pt}";
    const BODY: &str = "<div><p style='margin:0'>One two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen</p></div>";
    let rect = |x: f32, y: f32, width: f32, height: f32| Rect {
        x,
        y,
        width,
        height,
    };
    for (css, expected) in [
        (
            "div{overflow:clip;overflow-clip-margin:5pt}",
            vec![(rect(34.0, 34.0, 118.0, 58.0), [true; 2], 18.0)],
        ),
        (
            "div{overflow:clip;overflow-clip-margin:content-box}",
            vec![(rect(43.0, 43.0, 100.0, 40.0), [true; 2], 9.0)],
        ),
        (
            "div{overflow:clip;overflow-clip-margin:10pt border-box}",
            vec![(rect(26.0, 26.0, 134.0, 74.0), [true; 2], 26.0)],
        ),
        (
            // A radius smaller than the spread grows less: 4 + 10 × (1 + (0.4 − 1)³).
            "div{overflow:clip;overflow-clip-margin:border-box 10pt;border-radius:4pt}",
            vec![(rect(26.0, 26.0, 134.0, 74.0), [true; 2], 11.84)],
        ),
        (
            "div{overflow-x:clip;overflow-clip-margin:5pt}",
            vec![(rect(34.0, 34.0, 118.0, 58.0), [true, false], 0.0)],
        ),
        (
            "div{overflow:hidden;overflow-clip-margin:20pt}",
            vec![(rect(39.0, 39.0, 108.0, 48.0), [true; 2], 13.0)],
        ),
        (
            "div{overflow:hidden;contain:paint;overflow-clip-margin:content-box}",
            vec![
                (rect(39.0, 39.0, 108.0, 48.0), [true; 2], 13.0),
                (rect(43.0, 43.0, 100.0, 40.0), [true; 2], 9.0),
            ],
        ),
        (
            "div{contain:paint;overflow-clip-margin:2pt}",
            vec![(rect(37.0, 37.0, 112.0, 52.0), [true; 2], 15.0)],
        ),
    ] {
        let trace = trace(&format!("{BOX}{css}"), BODY);
        let clips = clips(&trace.display_lists[0]);
        assert_eq!(clips.len(), expected.len(), "{css}: clips pushed {clips:?}");
        for (clip, (rect, axes, radius)) in clips.iter().zip(expected) {
            assert_rect(css, clip.rect, rect);
            assert_eq!(clip.axes, axes, "{css}: clipped axes");
            assert!(
                clip.radii
                    .iter()
                    .flatten()
                    .all(|r| (r - radius).abs() < 0.01),
                "{css}: radii {:?}, expected {radius}",
                clip.radii
            );
        }
    }
}

#[test]
fn overflow_clip_margin_sides_use_independent_visual_box_origins() {
    const BOX: &str = "#box{width:100pt;height:40pt;padding:4pt;border:3pt solid navy;border-radius:16pt;overflow:clip}";
    const BODY: &str =
        "<section><div id=box><p style='margin:0'>Overflowing content</p></div></section>";
    for (declarations, expected, radii) in [
        (
            "#box{overflow-clip-margin-top:border-box 2pt;overflow-clip-margin-right:content-box 3pt;overflow-clip-margin-bottom:padding-box 5pt;overflow-clip-margin-left:border-box 7pt}",
            [29.0, 34.0, 117.0, 58.0],
            [[23.0, 18.0], [12.0, 18.0], [12.0, 18.0], [23.0, 18.0]],
        ),
        (
            "#box{direction:rtl;overflow-clip-margin:content-box 1pt;overflow-clip-margin-block:border-box 2pt;overflow-clip-margin-inline-start:padding-box 5pt;overflow-clip-margin-inline-end:border-box 7pt}",
            [29.0, 34.0, 123.0, 58.0],
            [[23.0, 18.0], [18.0, 18.0], [18.0, 18.0], [23.0, 18.0]],
        ),
        (
            "section{overflow-clip-margin:content-box 8pt}#box{overflow-clip-margin:inherit;overflow-clip-margin-top:2pt border-box;overflow-clip-margin-left:initial}",
            [39.0, 34.0, 112.0, 57.0],
            [[13.0, 18.0], [17.0, 18.0], [17.0, 17.0], [13.0, 17.0]],
        ),
        (
            "#box{--edge:content-box 1pt;overflow-clip-margin-left:border-box 7pt!important;overflow-clip-margin:var(--edge)}",
            [29.0, 42.0, 115.0, 42.0],
            [[23.0, 10.0], [10.0, 10.0], [10.0, 10.0], [23.0, 10.0]],
        ),
        (
            "section{overflow-clip-margin-inline-start:border-box 7pt}#box{direction:rtl;overflow-clip-margin-inline-start:inherit;overflow-clip-margin-block:content-box 1pt}",
            [39.0, 42.0, 118.0, 42.0],
            [[13.0, 10.0], [23.0, 10.0], [23.0, 10.0], [13.0, 10.0]],
        ),
        (
            "section{overflow-clip-margin:border-box 30pt}",
            [39.0, 39.0, 108.0, 48.0],
            [[13.0, 13.0]; 4],
        ),
    ] {
        let result = trace(&format!("{BOX}{declarations}"), BODY);
        let clips = clips(&result.display_lists[0]);
        assert_eq!(clips.len(), 1, "{declarations}: {clips:?}");
        let [x, y, width, height] = expected;
        assert_rect(
            declarations,
            clips[0].rect,
            Rect {
                x,
                y,
                width,
                height,
            },
        );
        for (actual, expected) in clips[0].radii.iter().flatten().zip(radii.iter().flatten()) {
            assert!(
                (actual - expected).abs() < TOLERANCE_PT,
                "{declarations}: corner radii {:?}, expected {radii:?}",
                clips[0].radii
            );
        }
    }
}

#[test]
fn clip_margin_shorthands_reject_multiple_offsets_and_reset_their_sides() {
    const BOX: &str =
        "div{width:100pt;height:40pt;padding:4pt;border:3pt solid navy;overflow:clip}";
    for declarations in [
        "overflow-clip-margin:content-box 1pt;overflow-clip-margin-inline:8pt 9pt;overflow-clip-margin-block:-1pt",
        "overflow-clip-margin:content-box 1pt;overflow-clip-margin:2pt 3pt;overflow-clip-margin-top:-2pt",
        "overflow-clip-margin-left:30pt;overflow-clip-margin-inline:content-box 1pt;overflow-clip-margin-block:content-box 1pt",
    ] {
        let result = trace(&format!("{BOX}div{{{declarations}}}"), "<div>Text</div>");
        let clips = clips(&result.display_lists[0]);
        assert_eq!(clips.len(), 1, "{declarations}: {clips:?}");
        assert_rect(
            declarations,
            clips[0].rect,
            Rect {
                x: 42.0,
                y: 42.0,
                width: 102.0,
                height: 42.0,
            },
        );
    }
}
