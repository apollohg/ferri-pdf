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

/// The texts of the lines painted inside the frames of `page`, such as footnote bodies and
/// running elements, that the block-axis clips of the slices holding them leave visible, by
/// their left edge, each such column top to bottom.
#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn visible_frame_columns(
    trace: &ferri_pdf::LayoutTrace,
    page: usize,
) -> Vec<(f32, Vec<String>)> {
    use ferri_pdf::DisplayItem;
    fn visit(
        items: &[DisplayItem],
        lines: Option<&[ferri_pdf::TextLine]>,
        origin: [f32; 2],
        clips: &mut Vec<[f32; 2]>,
        found: &mut Vec<(f32, f32, String)>,
    ) {
        let depth = clips.len();
        let mut opened = Vec::new();
        for item in items {
            match item {
                DisplayItem::PushClip(clip) if clip.axes == [false, true] => {
                    clips.push([
                        origin[1] + clip.rect.y,
                        origin[1] + clip.rect.y + clip.rect.height,
                    ]);
                    opened.push(true);
                }
                DisplayItem::Pop => {
                    if opened.pop() == Some(true) {
                        clips.pop();
                    }
                }
                item if item.is_push() => opened.push(false),
                DisplayItem::Text { line } => {
                    let Some(lines) = lines else { continue };
                    let line = &lines[*line];
                    let baseline = origin[1] + line.baseline;
                    if clips
                        .iter()
                        .all(|[top, bottom]| (*top..*bottom).contains(&baseline))
                    {
                        found.push((origin[0] + line.x, baseline, line.text.clone()));
                    }
                }
                DisplayItem::Frame(frame) => visit(
                    &frame.items,
                    Some(&frame.lines),
                    [origin[0] + frame.matrix[4], origin[1] + frame.matrix[5]],
                    clips,
                    found,
                ),
                _ => {}
            }
        }
        clips.truncate(depth);
    }
    let mut found = Vec::new();
    visit(
        &trace.display_lists[page],
        None,
        [0.0; 2],
        &mut Vec::new(),
        &mut found,
    );
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut columns: Vec<(f32, Vec<String>)> = Vec::new();
    for (x, _, text) in found {
        match columns.last_mut() {
            Some((left, texts)) if (*left - x).abs() < 1.0 => texts.push(text),
            _ => columns.push((x, vec![text])),
        }
    }
    columns
}

/// The column rules of `page` whose color is `rgb`, painted as left borders.
#[allow(
    dead_code,
    reason = "each test binary uses a subset of the shared helpers"
)]
pub(crate) fn column_rules(
    trace: &ferri_pdf::LayoutTrace,
    page: usize,
    rgb: [u8; 3],
) -> Vec<ferri_pdf::Rect> {
    const LEFT: usize = 3;
    fn collect(items: &[ferri_pdf::DisplayItem], rgb: [u8; 3], found: &mut Vec<ferri_pdf::Rect>) {
        for item in items {
            match item {
                ferri_pdf::DisplayItem::Box(paint)
                    if paint.decoration.borders[LEFT].color.rgb == rgb
                        && paint.decoration.borders[LEFT].width > 0.0 =>
                {
                    found.push(paint.rect);
                }
                ferri_pdf::DisplayItem::Frame(frame) => {
                    let mut nested = Vec::new();
                    collect(&frame.items, rgb, &mut nested);
                    found.extend(nested.into_iter().map(|rect| ferri_pdf::Rect {
                        x: rect.x + frame.matrix[4],
                        y: rect.y + frame.matrix[5],
                        ..rect
                    }));
                }
                _ => {}
            }
        }
    }
    let mut found = Vec::new();
    collect(&trace.display_lists[page], rgb, &mut found);
    found
}
