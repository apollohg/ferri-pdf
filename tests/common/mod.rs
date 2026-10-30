use ferri_pdf::RenderOptions;
use std::path::Path;

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) const PROFILE: &str = include_str!("../oracle/fixtures/profile.css");

pub(crate) fn local_options() -> RenderOptions {
    RenderOptions {
        base: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/oracle/fixtures")
            .into(),
        include_layout: true,
        ..Default::default()
    }
}

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn render_trace(html: &str) -> ferri_pdf::LayoutTrace {
    ferri_pdf::render_html(html, &local_options())
        .expect("render test HTML")
        .layout
        .expect("layout requested")
}

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn assert_pt(actual: f32, expected: f32) {
    assert_labelled_pt("value", actual, expected);
}

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn assert_labelled_pt(label: &str, actual: f32, expected: f32) {
    const POINT_TOLERANCE: f32 = 0.001;
    assert!(
        (actual - expected).abs() <= POINT_TOLERANCE,
        "{label}: expected {expected} pt, got {actual} pt (tolerance {POINT_TOLERANCE} pt)"
    );
}

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn painted_box(trace: &ferri_pdf::LayoutTrace, color: [u8; 3]) -> ferri_pdf::Rect {
    let boxes: Vec<_> = trace
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            ferri_pdf::DisplayItem::Box(paint)
                if paint.decoration.background.rgb == color
                    && paint.decoration.background.alpha == 1.0 =>
            {
                Some(paint.rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        boxes.len(),
        1,
        "expected one {color:?} box, found {boxes:?}"
    );
    boxes[0]
}

#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
/// The boxes painted with background `rgb`, mapped through the transforms and frames that
/// hold them into page coordinates.
pub(crate) fn page_boxes(trace: &ferri_pdf::LayoutTrace, rgb: [u8; 3]) -> Vec<ferri_pdf::Rect> {
    use ferri_pdf::DisplayItem;
    const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    fn compose([a, b, c, d, e, f]: [f32; 6], [g, h, i, j, k, l]: [f32; 6]) -> [f32; 6] {
        [
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ]
    }
    fn point([a, b, c, d, e, f]: [f32; 6], [x, y]: [f32; 2]) -> [f32; 2] {
        [a * x + c * y + e, b * x + d * y + f]
    }
    fn collect(
        items: &[DisplayItem],
        outer: [f32; 6],
        rgb: [u8; 3],
        found: &mut Vec<ferri_pdf::Rect>,
    ) {
        let mut stack = vec![outer];
        for item in items {
            let matrix = *stack
                .last()
                .expect("the outer transform stays on the stack");
            match item {
                DisplayItem::Box(paint)
                    if paint.decoration.background.rgb == rgb
                        && paint.decoration.background.alpha == 1.0 =>
                {
                    let rect = paint.rect;
                    let [first, second] = [
                        point(matrix, [rect.x, rect.y]),
                        point(matrix, [rect.x + rect.width, rect.y + rect.height]),
                    ];
                    found.push(ferri_pdf::Rect {
                        x: first[0].min(second[0]),
                        y: first[1].min(second[1]),
                        width: (first[0] - second[0]).abs(),
                        height: (first[1] - second[1]).abs(),
                    });
                }
                DisplayItem::Frame(frame) => {
                    collect(&frame.items, compose(matrix, frame.matrix), rgb, found);
                }
                DisplayItem::PushTransform(transform) => stack.push(compose(matrix, *transform)),
                DisplayItem::PushClip(_)
                | DisplayItem::PushOpacity(_)
                | DisplayItem::PushBlend(_)
                | DisplayItem::PushFilter(_)
                | DisplayItem::PushMask(_)
                | DisplayItem::PushBackdropRoot => stack.push(matrix),
                DisplayItem::Pop => {
                    stack.pop();
                }
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    for page in &trace.display_lists {
        collect(page, IDENTITY, rgb, &mut found);
    }
    found
}
