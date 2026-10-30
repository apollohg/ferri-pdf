mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{DisplayItem, LayoutTrace, Projection, Rect, render_html};

const TOLERANCE_PT: f32 = 0.001;
/// Profile page margin: the first box's border box starts here.
const PAGE_MARGIN_PT: f32 = 36.0;
const BOX_WIDTH_PT: f32 = 100.0;
const BOX_HEIGHT_PT: f32 = 40.0;
/// Border-box height plus the 8pt bottom margin between stacked boxes.
const BOX_PITCH_PT: f32 = 48.0;
/// The column-major identity matrix of an untransformed plane.
const IDENTITY_3D: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

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

/// The projections that draw boxes whose perspective varies `w` across them.
fn projections(items: &[DisplayItem]) -> Vec<Projection> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::PushProjection(projection) => Some(*projection),
            _ => None,
        })
        .collect()
}

#[test]
fn projective_transforms_draw_through_projections_and_malformed_ones_are_dropped() {
    // The box spans 36..136 by 36..76 about its centre (86, 56). rotateY(30deg) takes a point
    // dx right of the centre to depth -sin·dx, which perspective(100pt) divides by
    // w = 1 + sin·dx / 100 (CSS Transforms 2 §4.2).
    let output = trace(
        "div{width:100pt;height:40pt;transform:perspective(100pt) rotateY(30deg)}",
        "<div>X</div>",
    );
    let [projection] = projections(&output.display_lists[0])[..] else {
        panic!(
            "one projection must draw the box: {:?}",
            output.display_lists[0]
        );
    };
    let [a, b, c, d, e, f, g, h, i] = projection;
    let (sin, cos) = 30.0_f32.to_radians().sin_cos();
    for (x, y) in [
        (36.0, 36.0),
        (136.0, 36.0),
        (136.0, 76.0),
        (36.0, 76.0),
        (86.0, 50.0),
    ] {
        let w = g * x + h * y + i;
        let actual = [(a * x + c * y + e) / w, (b * x + d * y + f) / w];
        let (dx, dy) = (x - 86.0, y - 56.0);
        let depth = 1.0 + sin * dx / 100.0;
        let expected = [86.0 + cos * dx / depth, 56.0 + dy / depth];
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() < 1e-3),
            "({x}, {y}) must project to {expected:?}, got {actual:?} from {projection:?}"
        );
    }
    for css in [
        "div{transform:matrix3d(1,0,0,0.01,0,1,0,0,0,0,1,0,0,0,0,1)}",
        "div{perspective:100pt}div div{transform:rotateX(20deg)}",
    ] {
        let output = trace(css, "<div>X<div>Y</div></div>");
        assert!(
            !projections(&output.display_lists[0]).is_empty(),
            "{css} varies w across a box, so a projection draws it"
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
    let substituted = crate::trace(
        "div{--t:perspective(10pt) rotateY(10deg);transform:rotate(90deg);transform:var(--t)}",
        "<div>X</div>",
    );
    assert!(
        transforms(&substituted.display_lists[0]).is_empty()
            && projections(&substituted.display_lists[0]).len() == 1,
        "a valid substituted perspective replaces the earlier declaration"
    );
}

#[test]
fn individual_transforms_apply_translate_rotate_scale_then_transform() {
    let equivalent = [
        (
            "translate:10pt 5%;rotate:30deg;scale:2 3",
            "transform:translate(10pt,5%) rotate(30deg) scale(2,3)",
        ),
        (
            "translate:10pt;transform:scale(2) rotate(10deg)",
            "transform:translateX(10pt) scale(2) rotate(10deg)",
        ),
        (
            "rotate:x 60deg;scale:50% 50% 2;translate:0 0 9pt",
            "transform:translateZ(9pt) rotateX(60deg) scale3d(.5,.5,2)",
        ),
        (
            "rotate:0 0 1 45deg;transform-origin:0 0",
            "transform:rotate(45deg);transform-origin:0 0",
        ),
        ("rotate:1 1 0 70deg", "transform:rotate3d(1,1,0,70deg)"),
    ];
    for (individual, transform) in equivalent {
        let matrices = |css: &str| {
            transforms(
                &trace(
                    &format!("div{{width:100pt;height:40pt;{css}}}"),
                    "<div>X</div>",
                )
                .display_lists[0],
            )
        };
        let [actual] = matrices(individual)[..] else {
            panic!("{individual} must push one transform");
        };
        let [expected] = matrices(transform)[..] else {
            panic!("{transform} must push one transform");
        };
        assert_matrix(
            &format!("{individual} equals {transform}"),
            actual,
            expected,
        );
    }
    let identity = trace(
        ".n{translate:0;background:#ffeeaa}.n,.later{height:10pt}.later{background:#99ccff}",
        "<div class='n'>A</div><div class='later'>B</div>",
    );
    assert_eq!(
        paint_order(&identity, 0),
        [
            "box #99ccff",
            "text B",
            "push",
            "box #ffeeaa",
            "text A",
            "pop"
        ],
        "translate:0 is a transform, so the box paints as a stacking context above later flow"
    );
}

#[test]
fn transform_box_selects_the_content_box_for_origin_and_percentages() {
    const PADDING_PT: f32 = 5.0;
    const BORDER_PT: f32 = 3.0;
    let output = trace(
        "div{width:100pt;height:40pt;padding:5pt;border:3pt solid navy;transform-origin:0 0}\
         .c{transform-box:content-box;translate:50% 25%;rotate:90deg}\
         .f{transform-box:fill-box;translate:50% 25%;rotate:90deg}\
         .b{transform-box:stroke-box;translate:50% 25%;rotate:90deg}",
        "<div class='c'>A</div><div class='f'>B</div><div class='b'>C</div>",
    );
    let matrices = transforms(&output.display_lists[0]);
    assert_eq!(matrices.len(), 3, "one transform per box: {matrices:?}");
    let pitch = BOX_HEIGHT_PT + 2.0 * (PADDING_PT + BORDER_PT);
    let inset = PADDING_PT + BORDER_PT;
    let rotated =
        |[ox, oy]: [f32; 2], [tx, ty]: [f32; 2]| [0.0, 1.0, -1.0, 0.0, ox + tx + oy, oy + ty - ox];
    for (index, label) in [(0, "content-box"), (1, "fill-box uses the content box")] {
        let origin = [
            PAGE_MARGIN_PT + inset,
            PAGE_MARGIN_PT + pitch * index as f32 + inset,
        ];
        assert_matrix(
            label,
            matrices[index],
            rotated(origin, [0.5 * BOX_WIDTH_PT, 0.25 * BOX_HEIGHT_PT]),
        );
    }
    let border = [BOX_WIDTH_PT + 2.0 * inset, BOX_HEIGHT_PT + 2.0 * inset];
    assert_matrix(
        "stroke-box uses the border box",
        matrices[2],
        rotated(
            [PAGE_MARGIN_PT, PAGE_MARGIN_PT + 2.0 * pitch],
            [0.5 * border[0], 0.25 * border[1]],
        ),
    );
}

#[test]
fn parent_perspective_scales_depth_translations_about_its_origin() {
    const DEPTH_PT: f32 = 100.0;
    const LIFT_PT: f32 = 50.0;
    const PARENT_HEIGHT_PT: f32 = 120.0;
    let output = trace(
        &format!(
            ".p{{height:{PARENT_HEIGHT_PT}pt;perspective:{DEPTH_PT}pt}}.o{{perspective-origin:left top}}\
             .p div{{width:40pt;height:20pt;transform:translateZ({LIFT_PT}pt)}}\
             .flat div{{transform:rotate(90deg)}}"
        ),
        "<div class='p'><div>A</div></div><div class='p o'><div>B</div></div>\
         <div class='p flat'><div>C</div></div>",
    );
    let matrices = transforms(&output.display_lists[0]);
    assert_eq!(matrices.len(), 3, "one transform per child: {matrices:?}");
    let scale = DEPTH_PT / (DEPTH_PT - LIFT_PT);
    let about = |[ox, oy]: [f32; 2]| [scale, 0.0, 0.0, scale, ox - scale * ox, oy - scale * oy];
    let width = output.page_area.width;
    assert_matrix(
        "the default origin is the centre of the parent's border box",
        matrices[0],
        about([
            PAGE_MARGIN_PT + width / 2.0,
            PAGE_MARGIN_PT + PARENT_HEIGHT_PT / 2.0,
        ]),
    );
    assert_matrix(
        "perspective-origin moves the vanishing point to the parent's corner",
        matrices[1],
        about([PAGE_MARGIN_PT, PAGE_MARGIN_PT + PARENT_HEIGHT_PT]),
    );
    let [ox, oy] = [
        PAGE_MARGIN_PT + 20.0,
        PAGE_MARGIN_PT + 2.0 * PARENT_HEIGHT_PT + 10.0,
    ];
    assert_matrix(
        "perspective leaves transforms within the drawing plane unchanged",
        matrices[2],
        [0.0, 1.0, -1.0, 0.0, ox + oy, oy - ox],
    );
}

#[test]
fn table_rows_give_their_cells_perspective_and_row_groups_give_it_to_rows_only() {
    const DEPTH_PT: f32 = 100.0;
    const LIFT_PT: f32 = 50.0;
    const ROW_WIDTH_PT: f32 = 200.0;
    const ROW_HEIGHT_PT: f32 = 60.0;
    let table = |owner: &str| {
        trace(
            &format!(
                "table{{border-spacing:0;width:{ROW_WIDTH_PT}pt;table-layout:fixed}}\
                 td{{padding:0;height:{ROW_HEIGHT_PT}pt}}{owner}{{perspective:{DEPTH_PT}pt}}\
                 .lift{{transform:translateZ({LIFT_PT}pt)}}"
            ),
            "<table><tbody><tr><td class=lift>A</td><td>B</td></tr></tbody></table>",
        )
    };
    let matrices = transforms(&table("tr").display_lists[0]);
    assert_eq!(matrices.len(), 1, "one transformed cell: {matrices:?}");
    let scale = DEPTH_PT / (DEPTH_PT - LIFT_PT);
    let [ox, oy] = [
        PAGE_MARGIN_PT + ROW_WIDTH_PT / 2.0,
        PAGE_MARGIN_PT + ROW_HEIGHT_PT / 2.0,
    ];
    assert_matrix(
        "the vanishing point is the centre of the row's box, not the cell's",
        matrices[0],
        [scale, 0.0, 0.0, scale, ox - scale * ox, oy - scale * oy],
    );
    let matrices = transforms(&table("tbody").display_lists[0]);
    assert_eq!(matrices.len(), 1, "one transformed cell: {matrices:?}");
    assert_matrix(
        "a row group's perspective applies to its rows, not to their cells",
        matrices[0],
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    );
    let output = trace(
        "tr{transform-style:preserve-3d}td{transform:rotateX(30deg)}",
        "<table><tr><td>A</td><td>B</td></tr></table>",
    );
    let planes = scene_planes(&output.display_lists[0]);
    assert_eq!(
        planes.len(),
        2,
        "each cell tilting out of its row is a plane of the row's 3D rendering context: {planes:?}"
    );
    let (sin, cos) = 30.0_f32.to_radians().sin_cos();
    for (index, plane) in planes.iter().enumerate() {
        assert!(
            (plane[5] - cos).abs() < TOLERANCE_PT
                && (plane[6] - sin).abs() < TOLERANCE_PT
                && (plane[9] + sin).abs() < TOLERANCE_PT,
            "cell {index} turns about the x axis within the row's space: {plane:?}"
        );
    }
}

/// The accumulated matrices of the planes of each 3D rendering context in `items`.
fn scene_planes(items: &[DisplayItem]) -> Vec<[f32; 16]> {
    items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Scene(scene) => Some(scene.planes.iter().map(|plane| plane.matrix)),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn boxes_behind_the_viewer_or_facing_away_with_hidden_backfaces_are_not_rendered() {
    let output = trace(
        "div{height:20pt}.behind{transform:perspective(50pt) translateZ(100pt);background:red}\
         .back{transform:rotateY(180deg);backface-visibility:hidden;background:red}\
         .front{transform:rotateY(180deg);background:blue}\
         .flipped{scale:-1 1;backface-visibility:hidden;background:lime}",
        "<div class='behind'>Behind</div><div class='back'>Back</div>\
         <div class='front'>Front</div><div class='flipped'>Flipped</div>",
    );
    let order = paint_order(&output, 0);
    assert!(
        !order
            .iter()
            .any(|label| label == "box #ff0000" || label == "text Behind" || label == "text Back"),
        "w < 0 and a hidden back face must not paint: {order:?}"
    );
    for visible in ["box #0000ff", "text Front", "box #00ff00", "text Flipped"] {
        assert!(
            order.iter().any(|label| label == visible),
            "{visible} must paint: a visible back face, and a mirror within the plane, show: {order:?}"
        );
    }
}

#[test]
fn preserve_3d_places_planes_by_their_accumulated_matrices() {
    // The child spans 36..56 vertically about its centre y = 46; rotateX(30deg) about it puts
    // y' = cos·(y - 46) + 46 and z = sin·(y - 46), with the parent's 100pt perspective about
    // its own centre applying in the context's space (CSS Transforms 2 §4.1.4).
    let output = trace(
        "div{height:20pt}.p{perspective:100pt}.s{transform-style:preserve-3d}\
         .s div{transform:rotateX(30deg)}",
        "<div class='p'><div class='s'><div>A</div>R</div></div>",
    );
    let planes = scene_planes(&output.display_lists[0]);
    assert_eq!(
        planes.len(),
        2,
        "the root's content and its tilted child each have a plane: {planes:?}"
    );
    let (sin, cos) = 30.0_f32.to_radians().sin_cos();
    let child = planes[1];
    let w = |y: f32| child[7] * y + child[15];
    let z = |y: f32| child[6] * y + child[14];
    for y in [36.0, 46.0, 56.0] {
        let depth = sin * (y - 46.0);
        let perspective = 1.0 - depth / 100.0;
        assert!(
            (z(y) - depth).abs() < 1e-3 && (w(y) - perspective).abs() < 1e-4,
            "a point at y = {y} lies at depth {depth} with w = {perspective}: {child:?}"
        );
        let projected = (child[5] * y + child[13]) / w(y);
        let expected = (cos * (y - 46.0) + 46.0 - 46.0) / perspective + 46.0;
        assert!(
            (projected - expected).abs() < 1e-3,
            "y = {y} projects to {expected}, got {projected}"
        );
    }
    let planar = |css: &str| {
        let output = trace(
            &format!("div{{height:20pt}}.s div{{transform:rotate(20deg)}}{css}"),
            "<div class='s'><div>A</div></div>",
        );
        let mut matrices = transforms(&output.display_lists[0]);
        matrices.extend(
            scene_planes(&output.display_lists[0])
                .into_iter()
                .filter(|plane| plane != &IDENTITY_3D)
                .map(|m| [m[0], m[1], m[4], m[5], m[12], m[13]]),
        );
        matrices
    };
    assert_eq!(
        planar(".s{transform-style:preserve-3d}"),
        planar(""),
        "a coplanar plane projects as the flattened transform draws"
    );
    let grouped = trace(
        "div{height:20pt}.s{transform-style:preserve-3d;opacity:.5}.s div{transform:rotateX(30deg)}",
        "<div class='s'><div>A</div></div>",
    );
    assert_eq!(
        transforms(&grouped.display_lists[0]).len(),
        1,
        "opacity is a grouping property, so the used transform-style is flat"
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
                    f32::midpoint(PAGE_MARGIN_PT, 80.0),
                    f32::midpoint(PAGE_MARGIN_PT, height / 2.0),
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
            assert_ne!(matrices, [] as [[f32; 6]; 0]);
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
        let expected = [
            0.5,
            0.0,
            0.0,
            0.5,
            f32::midpoint(x, 50.0),
            f32::midpoint(y, 20.0),
        ];
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
