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
