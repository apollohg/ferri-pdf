mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{DisplayItem, LayoutTrace, Rect, render_html};

const TOLERANCE_PT: f32 = 0.001;
/// Profile page margin: the first box's border box starts here.
const PAGE_MARGIN_PT: f32 = 36.0;
const BOX_WIDTH_PT: f32 = 100.0;
const BOX_HEIGHT_PT: f32 = 40.0;
/// Border-box height plus the 8pt bottom margin between stacked boxes.
const BOX_PITCH_PT: f32 = 48.0;

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    )
    .expect("transform fixture must render")
    .layout
    .unwrap()
}
fn transforms(items: &[DisplayItem]) -> Vec<[f32; 6]> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::PushTransform(matrix) => Some(*matrix),
            _ => None,
        })
        .collect()
}
fn assert_matrix(label: &str, actual: [f32; 6], expected: [f32; 6]) {
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(a, e)| (a - e).abs() < TOLERANCE_PT),
        "{label}: {actual:?}, expected {expected:?}"
    );
}
/// Display items of one page as labels: backgrounds by colour, text by content.
fn paint_order(trace: &LayoutTrace, page: usize) -> Vec<String> {
    trace.display_lists[page]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                let [r, g, b] = paint.decoration.background.rgb;
                Some(format!("box #{r:02x}{g:02x}{b:02x}"))
            }
            DisplayItem::Text { line } => Some(format!("text {}", trace.pages[page][*line].text)),
            DisplayItem::PushTransform(_) => Some("push".into()),
            DisplayItem::Pop => Some("pop".into()),
            _ => None,
        })
        .collect()
}
fn background_rect(trace: &LayoutTrace, page: usize, rgb: [u8; 3]) -> Option<Rect> {
    trace.display_lists[page]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == rgb => Some(paint.rect),
            _ => None,
        })
}

#[test]
fn transform_lists_compose_about_the_origin_and_flatten_3d_functions() {
    let trace = trace(
        "div{width:100pt;height:40pt;margin-bottom:8pt}\
         .p{width:80pt;height:20pt;padding:5pt;border:5pt solid navy;transform:translate(10%,-50%)}\
         .o{transform:rotate(90deg);transform-origin:left top}\
         .s{transform:scale(2) translate(10pt,5pt);transform-origin:0 0}\
         .m{transform:matrix(1,0.2,0.3,1,10,20)}\
         .f{transform:rotateX(60deg) scale3d(1,0.5,2) translate3d(5pt,0,9pt)}\
         .z{transform:rotateX(45deg);transform-origin:0 0 20pt}",
        "<div class='p'></div><div class='o'></div><div class='s'></div><div class='m'></div><div class='f'></div><div class='z'></div>",
    );
    let top = |index: usize| PAGE_MARGIN_PT + BOX_PITCH_PT * index as f32;
    let centre = [PAGE_MARGIN_PT + BOX_WIDTH_PT / 2.0, BOX_HEIGHT_PT / 2.0];
    let matrices = transforms(&trace.display_lists[0]);
    assert_eq!(matrices.len(), 6, "one transform per box: {matrices:?}");
    assert_matrix(
        "percentages resolve against the border box, not the content box",
        matrices[0],
        [1.0, 0.0, 0.0, 1.0, 0.1 * BOX_WIDTH_PT, -0.5 * BOX_HEIGHT_PT],
    );
    let [ox, oy] = [PAGE_MARGIN_PT, top(1)];
    assert_matrix(
        "rotate(90deg) about the top-left corner",
        matrices[1],
        [0.0, 1.0, -1.0, 0.0, ox + oy, oy - ox],
    );
    let [ox, oy] = [PAGE_MARGIN_PT, top(2)];
    assert_matrix(
        "later functions apply first: the translation is scaled (2 0 0 2 -16 …)",
        matrices[2],
        [2.0, 0.0, 0.0, 2.0, 20.0 - ox, 10.0 - oy],
    );
    const PX: f32 = 0.75;
    let [ox, oy] = [centre[0], top(3) + centre[1]];
    assert_matrix(
        "matrix() translations are CSS pixels (1 -0.2 -0.3 1 … 2.2)",
        matrices[3],
        [
            1.0,
            0.2,
            0.3,
            1.0,
            10.0 * PX - 0.3 * oy,
            20.0 * PX - 0.2 * ox,
        ],
    );
    let (sin, cos) = 60f32.to_radians().sin_cos();
    let oy = top(4) + centre[1];
    assert_matrix(
        "3D functions compose before flattening, so scaled z translation moves y",
        matrices[4],
        [
            1.0,
            0.0,
            0.0,
            cos * 0.5,
            5.0,
            oy - cos * 0.5 * oy - sin * 2.0 * 9.0,
        ],
    );
    let (sin, cos) = 45f32.to_radians().sin_cos();
    let oy = top(5);
    assert_matrix(
        "a z origin offsets the rotated plane",
        matrices[5],
        [1.0, 0.0, 0.0, cos, 0.0, oy - cos * oy + sin * 20.0],
    );
}

#[test]
fn projective_and_malformed_transforms_are_errors_not_flat_drawings() {
    for (css, expected) in [
        (
            "transform:perspective(100pt) rotateY(30deg)",
            "perspective() is unsupported: PDF matrices are affine",
        ),
        (
            "transform:matrix3d(1,0,0,0.01,0,1,0,0,0,0,1,0,0,0,0,1)",
            "projective matrix3d() is unsupported",
        ),
    ] {
        let result = render_html(
            &format!("<style>{PROFILE}div{{{css}}}</style><div>X</div>"),
            &local_options(),
        );
        assert!(
            matches!(result, Err(ferri_pdf::Error::Css(ferri_pdf::StyleError::Invalid(ref message))) if message == expected),
            "{css} must fail with {expected:?}: {:?}",
            result.err()
        );
    }
    // CSS Transforms 1 orders lengths horizontal then vertical and keeps keywords on their own
    // axis, so these are invalid and dropped, leaving the earlier origin.
    let origin = |css: &str| {
        transforms(
            &trace(
                &format!("div{{transform:rotate(90deg);{css}}}"),
                "<div>X</div>",
            )
            .display_lists[0],
        )
    };
    for invalid in ["left left", "top 10pt"] {
        assert_eq!(
            origin(&format!("transform-origin:0 0;transform-origin:{invalid}")),
            origin("transform-origin:0 0"),
            "transform-origin:{invalid} must be dropped"
        );
    }
    let trace = trace(
        "div{--t:rotate(1pt);transform:rotate(90deg);transform:var(--t)}",
        "<div>X</div>",
    );
    assert!(
        transforms(&trace.display_lists[0]).is_empty(),
        "a transform invalid at computed time is unset rather than the earlier declaration"
    );
    let substituted = render_html(
        &format!(
            "<style>{PROFILE}div{{--t:perspective(1pt);transform:rotate(90deg);transform:var(--t)}}</style><div>X</div>"
        ),
        &local_options(),
    );
    assert!(
        matches!(substituted, Err(ferri_pdf::Error::Css(ferri_pdf::StyleError::Invalid(ref message))) if message == "perspective() is unsupported: PDF matrices are affine"),
        "a valid substituted transform the engine cannot draw is an error: {:?}",
        substituted.err()
    );
}

#[test]
fn transforms_create_stacking_contexts_without_enabling_z_index() {
    let trace = trace(
        "div{height:20pt}\
         .n{position:relative;z-index:-1;transform:translateX(5pt);background:#ff9999}\
         .t{z-index:-1;transform:translateY(10pt);background:#ffeeaa}\
         .later{background:#99ccff}",
        "<div class='n'>Negative</div><div class='t'>Transformed</div><div class='later'>Later</div>",
    );
    assert_eq!(
        paint_order(&trace, 0),
        [
            "push",
            "box #ff9999",
            "text Negative",
            "pop",
            "box #99ccff",
            "text Later",
            "push",
            "box #ffeeaa",
            "text Transformed",
            "pop",
        ],
        "a positioned negative layer paints below flow; an unpositioned transform ignores z-index \
         and paints as a z-index 0 stacking context above later flow, its content transformed with it"
    );
}

#[test]
fn transformed_boxes_contain_absolute_and_fixed_descendants() {
    const ABS: [u8; 3] = [0xff, 0xee, 0xaa];
    const FIXED: [u8; 3] = [0x99, 0xcc, 0xff];
    const MARGIN_LEFT_PT: f32 = 40.0;
    const BORDER_PT: f32 = 1.0;
    const PADDING_BOX: Rect = Rect {
        x: PAGE_MARGIN_PT + MARGIN_LEFT_PT + BORDER_PT,
        y: PAGE_MARGIN_PT + BORDER_PT,
        width: 150.0,
        height: 60.0,
    };
    const FIXED_TOP_PT: f32 = 4.0;
    const FIXED_LEFT_FRACTION: f32 = 0.1;
    let body = format!(
        "<div class='t'><div class='a'>Abs</div><div class='f'>Fixed</div></div>{}",
        (1..41)
            .map(|i| format!("<p>Para {i}</p>"))
            .collect::<String>()
    );
    let trace = trace(
        ".t{width:150pt;height:60pt;margin-left:40pt;border:1pt solid navy;transform:rotate(5deg)}\
         .a{position:absolute;right:0;bottom:0;background:#ffeeaa}\
         .f{position:fixed;left:10%;top:4pt;background:#99ccff}",
        &body,
    );
    assert!(trace.pages.len() > 1, "fixture must span pages");
    for page in 1..trace.pages.len() {
        assert!(
            background_rect(&trace, page, FIXED).is_none()
                && trace.pages[page].iter().all(|line| line.text != "Fixed"),
            "a fixed box inside a transform belongs to that box, so it is not repeated on page {page}"
        );
    }
    let abs = background_rect(&trace, 0, ABS).expect("absolute box must paint");
    assert!(
        (abs.x + abs.width - (PADDING_BOX.x + PADDING_BOX.width)).abs() < TOLERANCE_PT
            && (abs.y + abs.height - (PADDING_BOX.y + PADDING_BOX.height)).abs() < TOLERANCE_PT,
        "right:0;bottom:0 resolve against the transformed padding box {PADDING_BOX:?}, not the page: {abs:?}"
    );
    let fixed = background_rect(&trace, 0, FIXED).expect("fixed box must paint");
    assert!(
        (fixed.x - (PADDING_BOX.x + FIXED_LEFT_FRACTION * PADDING_BOX.width)).abs() < TOLERANCE_PT
            && (fixed.y - (PADDING_BOX.y + FIXED_TOP_PT)).abs() < TOLERANCE_PT,
        "fixed insets and percentages use the transformed padding box {PADDING_BOX:?}: {fixed:?}"
    );
    let order = paint_order(&trace, 0);
    let push = order
        .iter()
        .position(|label| label == "push")
        .unwrap_or_else(|| panic!("the container must push its transform: {order:?}"));
    assert_eq!(
        order[push..],
        [
            "push",
            "box #ffeeaa",
            "text Abs",
            "box #99ccff",
            "text Fixed",
            "pop"
        ],
        "positioned descendants are drawn inside their container's transform, above its flow: {order:?}"
    );
}

#[test]
fn math_functions_in_transforms_resolve_against_each_border_box() {
    const WIDE_BOX_WIDTH_PT: f32 = 200.0;
    let trace = trace(
        "div{width:100pt;height:40pt;margin-bottom:8pt}\
         .t{transform:translateX(calc(50% - 10pt))}\
         .wide{width:200pt}\
         .r{transform:rotate(calc(45deg * 2));transform-origin:calc(100% - 100pt) calc(50% - 20pt)}\
         .s{transform:scale(calc(1 + 1)) translate(calc(10pt / 2), calc(25% + 0pt));transform-origin:0 0}\
         .i{transform:translateX(5pt);transform:rotate(calc(10pt))}",
        "<div class='t'>A</div><div class='t wide'>B</div><div class='r'>C</div><div class='s'>D</div><div class='i'>E</div>",
    );
    let top = |index: usize| PAGE_MARGIN_PT + BOX_PITCH_PT * index as f32;
    let matrices = transforms(&trace.display_lists[0]);
    assert_eq!(matrices.len(), 5, "one transform per box: {matrices:?}");
    assert_matrix(
        "calc(50% - 10pt) uses the 100pt border box",
        matrices[0],
        [1.0, 0.0, 0.0, 1.0, 0.5 * BOX_WIDTH_PT - 10.0, 0.0],
    );
    assert_matrix(
        "the same declaration resolves against a 200pt border box at layout time",
        matrices[1],
        [1.0, 0.0, 0.0, 1.0, 0.5 * WIDE_BOX_WIDTH_PT - 10.0, 0.0],
    );
    let [ox, oy] = [PAGE_MARGIN_PT, top(2)];
    assert_matrix(
        "calc() angles rotate 90deg about a calc() origin at the top-left corner",
        matrices[2],
        [0.0, 1.0, -1.0, 0.0, ox + oy, oy - ox],
    );
    let [ox, oy] = [PAGE_MARGIN_PT, top(3)];
    assert_matrix(
        "calc() numbers scale a translation whose y is 25% of the 40pt height",
        matrices[3],
        [
            2.0,
            0.0,
            0.0,
            2.0,
            2.0 * 5.0 - ox,
            2.0 * 0.25 * BOX_HEIGHT_PT - oy,
        ],
    );
    assert_matrix(
        "a length where an angle is required drops the declaration, keeping the earlier one",
        matrices[4],
        [1.0, 0.0, 0.0, 1.0, 5.0, 0.0],
    );
}

#[test]
fn table_row_and_group_transforms_use_the_structural_reference_box() {
    for (selector, height) in [("tr", 40.0), ("tbody", 80.0)] {
        for (transform, expected) in [
            (
                "translate(50%,25%)",
                [1.0, 0.0, 0.0, 1.0, 80.0, height / 4.0],
            ),
            (
                "scale(.5)",
                [
                    0.5,
                    0.0,
                    0.0,
                    0.5,
                    (PAGE_MARGIN_PT + 80.0) / 2.0,
                    (PAGE_MARGIN_PT + height / 2.0) / 2.0,
                ],
            ),
        ] {
            let selector = if selector == "tr" {
                "tr:first-child"
            } else {
                selector
            };
            let output = trace(
                &format!(
                    "table{{width:160pt;border-spacing:0}}tr{{height:40pt}}td{{padding:0}}{selector}{{transform:{transform}}}"
                ),
                "<table><tbody><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></tbody></table><p>After</p>",
            );
            let matrices = transforms(&output.display_lists[0]);
            assert!(!matrices.is_empty());
            for matrix in matrices {
                assert_matrix(selector, matrix, expected);
            }
        }
    }
}

#[test]
fn structural_transforms_in_columns_use_the_placed_column_origin() {
    let output = trace(
        ".columns{column-count:2;column-gap:20pt;width:220pt;column-fill:auto;height:80pt}table{width:100%;border-spacing:0}tr{height:40pt;transform:scale(.5)}td{padding:0}",
        "<div class=columns><table><tr><td>A</td></tr><tr><td>B</td></tr><tr><td>C</td></tr><tr><td>D</td></tr></table></div>",
    );
    let matrices = transforms(&output.display_lists[0]);
    for (x, y) in [(36.0, 36.0), (36.0, 76.0), (156.0, 36.0), (156.0, 76.0)] {
        let expected = [0.5, 0.0, 0.0, 0.5, (x + 50.0) / 2.0, (y + 20.0) / 2.0];
        assert!(
            matrices.iter().any(|matrix| matrix
                .iter()
                .zip(expected)
                .all(|(a, b)| (a - b).abs() < TOLERANCE_PT)),
            "missing column transform {expected:?}: {matrices:?}"
        );
    }
}

#[test]
fn singular_transforms_omit_paint_but_mirrored_boxes_remain_visible() {
    let output = trace(
        ".hidden{transform:scaleX(0);background:red}.mirrored{transform:scaleX(-1);background:blue}div{height:40pt}",
        "<div class=hidden>Hidden<span style='transform:translate(5pt,0);display:inline-block'>Child</span></div><div class=mirrored>Visible</div>",
    );
    let order = paint_order(&output, 0);
    assert!(
        !order.iter().any(|name| name.starts_with("text Hidden")
            || name == "text Child"
            || name == "box #ff0000"),
        "singular subtree painted: {order:?}"
    );
    assert!(order.iter().any(|name| name == "text Visible"));
    assert!(order.iter().any(|name| name == "box #0000ff"));
}
