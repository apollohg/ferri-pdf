//! Projective maps of the drawing plane (CSS Transforms 2 §4.2): a box under a perspective
//! keeps straight lines straight but scales each point by its own `w`, which a PDF content
//! matrix cannot express.
use crate::paint::{Point, Rect};

/// `[a, b, c, d, e, f, g, h, i]`: `(x, y)` maps to `((a·x + c·y + e) / w, (b·x + d·y + f) / w)`
/// with `w = g·x + h·y + i`. Points with `w ≤ 0` lie behind the viewer and do not render.
pub type Projection = [f32; 9];

pub(crate) const IDENTITY: Projection = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
/// Projected coordinates beyond this many points from the origin are at infinity: §4.2 sends
/// points with `w = 0` "much larger than the viewport size", and clipping to this horizon keeps
/// the region with `w > 0` bounded where it reaches them.
pub(crate) const HORIZON_PT: f64 = 1.0e6;
const W: usize = 6;

/// A linear function `k₀·x + k₁·y + k₂` of local points; its half-plane is where it is not
/// negative.
pub(crate) type Line = [f64; 3];

/// Relative variation of `w` across a region within which a projection is affine for print:
/// far below a device dot over any page.
const AFFINE_TOLERANCE: f64 = 1.0e-6;

/// `m` as an affine map over the convex hull of `points`, when its `w` there stays positive
/// and within `AFFINE_TOLERANCE` of its value at their centre.
pub(crate) fn affine_over(m: Projection, points: &[Point]) -> Option<[f32; 6]> {
    let count = points.len() as f32;
    let centre = points.iter().fold([0.0, 0.0], |[x, y], point| {
        [x + point[0] / count, y + point[1] / count]
    });
    let depth = weight(m, centre);
    (depth > 0.0
        && points
            .iter()
            .all(|&point| (weight(m, point) - depth).abs() <= AFFINE_TOLERANCE * depth))
    .then(|| {
        let depth = depth as f32;
        [m[0], m[1], m[2], m[3], m[4], m[5]].map(|value| value / depth)
    })
}

/// The projection of the plane `z = 0` that a column-major 4×4 matrix places.
pub(crate) fn of_plane(m: [f32; 16]) -> Projection {
    [m[0], m[1], m[4], m[5], m[12], m[13], m[3], m[7], m[15]]
}

/// The projection drawing an affine `[a, b, c, d, e, f]`.
pub(crate) fn from_affine([a, b, c, d, e, f]: [f32; 6]) -> Projection {
    [a, b, c, d, e, f, 0.0, 0.0, 1.0]
}

/// `m` as an affine matrix, when its `w` is one positive constant.
pub(crate) fn affine(m: Projection) -> Option<[f32; 6]> {
    (m[W] == 0.0 && m[W + 1] == 0.0 && m[W + 2] > 0.0).then(|| {
        let i = m[W + 2];
        [m[0] / i, m[1] / i, m[2] / i, m[3] / i, m[4] / i, m[5] / i]
    })
}

/// The rows `[x, y, w]` of `m` as 3×3 homogeneous coefficients.
fn rows(m: Projection) -> [[f64; 3]; 3] {
    let m = m.map(f64::from);
    [[m[0], m[2], m[4]], [m[1], m[3], m[5]], [m[6], m[7], m[8]]]
}

fn from_rows(r: [[f64; 3]; 3]) -> Projection {
    [
        r[0][0], r[1][0], r[0][1], r[1][1], r[0][2], r[1][2], r[2][0], r[2][1], r[2][2],
    ]
    .map(|value| value as f32)
}

/// `m` after `n`.
pub(crate) fn compose(m: Projection, n: Projection) -> Projection {
    let (m, n) = (rows(m), rows(n));
    from_rows(std::array::from_fn(|row| {
        std::array::from_fn(|column| (0..3).map(|k| m[row][k] * n[k][column]).sum())
    }))
}

/// The projection undoing `m`, or `None` when `m` collapses the plane.
pub(crate) fn inverse(m: Projection) -> Option<Projection> {
    let r = rows(m);
    let cofactor = |row: usize, column: usize| {
        let (r0, r1) = ((row + 1) % 3, (row + 2) % 3);
        let (c0, c1) = ((column + 1) % 3, (column + 2) % 3);
        r[r0][c0] * r[r1][c1] - r[r0][c1] * r[r1][c0]
    };
    let determinant: f64 = (0..3)
        .map(|column| r[0][column] * cofactor(0, column))
        .sum();
    (determinant != 0.0 && determinant.is_finite()).then(|| {
        from_rows(std::array::from_fn(|row| {
            std::array::from_fn(|column| cofactor(column, row) / determinant)
        }))
    })
}

/// Whether `m` maps no area onto the plane.
pub(crate) fn degenerate(m: Projection) -> bool {
    inverse(m).is_none()
}

/// `w` at local point `p`.
pub(crate) fn weight(m: Projection, [x, y]: Point) -> f64 {
    let r = rows(m);
    r[2][0] * f64::from(x) + r[2][1] * f64::from(y) + r[2][2]
}

/// The projection of local point `p`, which must have a positive `w`.
pub(crate) fn apply(m: Projection, [x, y]: Point) -> Point {
    let r = rows(m);
    let (x, y) = (f64::from(x), f64::from(y));
    let w = r[2][0] * x + r[2][1] * y + r[2][2];
    [
        ((r[0][0] * x + r[0][1] * y + r[0][2]) / w) as f32,
        ((r[1][0] * x + r[1][1] * y + r[1][2]) / w) as f32,
    ]
}

/// The local half-planes whose intersection `m` maps into `bounds` with a positive `w`.
pub(crate) fn within(m: Projection, bounds: [[f64; 2]; 2]) -> [Line; 5] {
    let [x, y, w] = rows(m);
    let along = |row: [f64; 3], limit: f64, sign: f64| -> Line {
        std::array::from_fn(|k| sign * (row[k] - limit * w[k]))
    };
    [
        w,
        along(x, bounds[0][0], 1.0),
        along(x, bounds[0][1], -1.0),
        along(y, bounds[1][0], 1.0),
        along(y, bounds[1][1], -1.0),
    ]
}

/// The local half-planes `m` maps inside the horizon with a positive `w`.
pub(crate) fn finite(m: Projection) -> [Line; 5] {
    within(m, [[-HORIZON_PT, HORIZON_PT]; 2])
}

/// The half-plane where `m` keeps points in front of the viewer, `w ≥ 0`.
pub(crate) fn front(m: Projection) -> Line {
    rows(m)[2]
}

/// `line`, a function of the points `m` maps to, as a function of the points it maps from;
/// where `w` is positive both have the same sign.
pub(crate) fn pulled(line: Line, m: Projection) -> Line {
    let r = rows(m);
    std::array::from_fn(|column| (0..3).map(|row| line[row] * r[row][column]).sum())
}

/// The value of `line` at `p`.
pub(crate) fn side(line: Line, [x, y]: Point) -> f64 {
    line[0] * f64::from(x) + line[1] * f64::from(y) + line[2]
}

/// Sutherland–Hodgman clipping of `polygon` to the half-plane of `line`; the winding of every
/// point inside the half-plane is kept, so non-convex contours clip correctly.
pub(crate) fn clip(polygon: &[Point], line: Line) -> Vec<Point> {
    let mut output = Vec::with_capacity(polygon.len() + 1);
    for (index, &current) in polygon.iter().enumerate() {
        let previous = polygon[(index + polygon.len() - 1) % polygon.len()];
        let (now, before) = (side(line, current), side(line, previous));
        if (now >= 0.0) != (before >= 0.0) {
            let t = before / (before - now);
            output.push([
                (f64::from(previous[0]) + t * f64::from(current[0] - previous[0])) as f32,
                (f64::from(previous[1]) + t * f64::from(current[1] - previous[1])) as f32,
            ]);
        }
        if now >= 0.0 {
            output.push(current);
        }
    }
    output
}

/// `polygon` clipped to every half-plane of `lines`.
pub(crate) fn clip_all(polygon: Vec<Point>, lines: &[Line]) -> Vec<Point> {
    lines
        .iter()
        .fold(polygon, |polygon, &line| clip(&polygon, line))
}

/// The corners of `rect`, clockwise from its top-left on the page.
pub(crate) fn corners(rect: Rect) -> Vec<Point> {
    let [right, bottom] = [rect.x + rect.width, rect.y + rect.height];
    vec![
        [rect.x, rect.y],
        [right, rect.y],
        [right, bottom],
        [rect.x, bottom],
    ]
}

/// The projection of `polygon`, cut where it reaches behind the viewer or past the horizon.
pub(crate) fn map_polygon(m: Projection, polygon: Vec<Point>) -> Vec<Point> {
    clip_all(polygon, &finite(m))
        .into_iter()
        .map(|point| apply(m, point))
        .collect()
}

/// The bounds of `points`, or `None` without any.
pub(crate) fn bounds(points: &[Point]) -> Option<Rect> {
    let first = *points.first()?;
    let [left, top, right, bottom] = points.iter().fold(
        [first[0], first[1], first[0], first[1]],
        |[left, top, right, bottom], [x, y]| {
            [left.min(*x), top.min(*y), right.max(*x), bottom.max(*y)]
        },
    );
    Some(Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

/// The Jacobian `[a, b, c, d]` of `m` at local point `p`, where `w` is positive.
fn jacobian(m: Projection, p: Point) -> [f64; 4] {
    let [x, y, w] = rows(m);
    let at = |row: [f64; 3]| row[0] * f64::from(p[0]) + row[1] * f64::from(p[1]) + row[2];
    let (px, py, pw) = (at(x), at(y), at(w));
    let (qx, qy) = (px / pw, py / pw);
    [
        (x[0] - qx * w[0]) / pw,
        (y[0] - qy * w[0]) / pw,
        (x[1] - qx * w[1]) / pw,
        (y[1] - qy * w[1]) / pw,
    ]
}

/// The affine map touching `m` at local point `p`, whose `w` must be positive: equal to it
/// there with the same derivative.
pub(crate) fn tangent(m: Projection, p: Point) -> [f32; 6] {
    let [a, b, c, d] = jacobian(m, p);
    let [qx, qy] = apply(m, p).map(f64::from);
    let (x, y) = (f64::from(p[0]), f64::from(p[1]));
    [a, b, c, d, qx - a * x - c * y, qy - b * x - d * y].map(|value| value as f32)
}

/// The larger singular value of the linear map `[a, b, c, d]`: how far it stretches lengths
/// at most.
fn largest_singular_value([a, b, c, d]: [f64; 4]) -> f64 {
    let (e, f) = (f64::midpoint(a, d), f64::midpoint(a, -d));
    let (g, h) = (f64::midpoint(b, c), f64::midpoint(b, -c));
    e.hypot(h) + f.hypot(g)
}

/// The largest factor by which `m` stretches lengths at local point `p`.
pub(crate) fn stretch(m: Projection, p: Point) -> f64 {
    largest_singular_value(jacobian(m, p))
}

/// The factor by which `m` stretches lengths along unit `direction` at local point `p`.
pub(crate) fn stretch_along(m: Projection, p: Point, [x, y]: [f64; 2]) -> f64 {
    let [a, b, c, d] = jacobian(m, p);
    (a * x + c * y).hypot(b * x + d * y)
}

/// A bound on how far the tangent of `m` at `p` strays from `m` over the convex hull of
/// `points`, which must all have a positive `w`: at `p + δ` the two differ by exactly
/// `|J·δ| · |∇w·δ| / w(p + δ)`, where `J` is the Jacobian at `p`, and each factor is extreme
/// at a vertex.
pub(crate) fn tangent_error(m: Projection, p: Point, points: &[Point]) -> f64 {
    let [a, b, c, d] = jacobian(m, p);
    let gradient = rows(m)[2];
    let (mut stretch, mut climb, mut lowest) = (0.0_f64, 0.0_f64, f64::INFINITY);
    for &point in points {
        let (dx, dy) = (f64::from(point[0] - p[0]), f64::from(point[1] - p[1]));
        stretch = stretch.max((a * dx + c * dy).hypot(b * dx + d * dy));
        climb = climb.max((gradient[0] * dx + gradient[1] * dy).abs());
        lowest = lowest.min(weight(m, point));
    }
    stretch * climb / lowest
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEPTH: f32 = 400.0;
    const ORIGIN: [f32; 2] = [100.0, 50.0];

    /// `perspective(400pt) rotateY(-40deg)` about `ORIGIN`: `(x, y, 0)` turns to
    /// `(cos·x, y, sin·x)`, whose `w` is `1 + sin·x / 400`.
    fn tilted() -> Projection {
        let (sin, cos) = (-40.0_f32).to_radians().sin_cos();
        let [x, y] = ORIGIN;
        compose(
            from_affine([1.0, 0.0, 0.0, 1.0, x, y]),
            compose(
                [cos, 0.0, 0.0, 1.0, 0.0, 0.0, sin / DEPTH, 0.0, 1.0],
                from_affine([1.0, 0.0, 0.0, 1.0, -x, -y]),
            ),
        )
    }

    #[test]
    fn tangent_error_bounds_the_measured_deviation() {
        let m = tilted();
        let centre = [60.0, 30.0];
        let tangent = tangent(m, centre);
        let corners = corners(Rect {
            x: 40.0,
            y: 10.0,
            width: 40.0,
            height: 40.0,
        });
        let bound = tangent_error(m, centre, &corners);
        let mut measured = 0.0_f64;
        for step_x in 0..=20 {
            for step_y in 0..=20 {
                let point = [40.0 + 2.0 * step_x as f32, 10.0 + 2.0 * step_y as f32];
                let exact = apply(m, point);
                let [a, b, c, d, e, f] = tangent;
                let approximate = [
                    a * point[0] + c * point[1] + e,
                    b * point[0] + d * point[1] + f,
                ];
                measured = measured.max(f64::from(
                    (exact[0] - approximate[0]).hypot(exact[1] - approximate[1]),
                ));
            }
        }
        assert!(
            measured <= bound * (1.0 + 1e-4) && measured >= bound * 0.25,
            "tangent deviation {measured}pt over the square must stay within, and near, its bound {bound}pt"
        );
    }

    #[test]
    fn inverse_and_clipping_respect_the_viewer_plane() {
        let m = tilted();
        let inverse = inverse(m).expect("a tilted plane is invertible");
        for point in [[0.0, 0.0], [150.0, 80.0], [-30.0, 200.0]] {
            let there = apply(m, point);
            let back = apply(inverse, there);
            assert!(
                (back[0] - point[0]).abs() < 1e-3 && (back[1] - point[1]).abs() < 1e-3,
                "{point:?} maps to {there:?} and back to {back:?}"
            );
        }
        // w = 0 where x = 100 + depth / sin 40°; beyond it the plane is behind the viewer.
        let edge = ORIGIN[0] + DEPTH / 40.0_f32.to_radians().sin();
        let polygon = corners(Rect {
            x: 0.0,
            y: 0.0,
            width: 2.0 * edge,
            height: 10.0,
        });
        let kept = clip(&polygon, rows(m)[2]);
        for point in &kept {
            assert!(
                point[0] <= edge + 1e-2,
                "{point:?} lies behind the viewer, past x = {edge}"
            );
        }
        assert_eq!(
            kept.len(),
            4,
            "cutting a rectangle across keeps a quadrilateral: {kept:?}"
        );
        let mapped = map_polygon(m, polygon);
        assert!(
            mapped.iter().all(|point| point
                .iter()
                .all(|v| f64::from(v.abs()) <= HORIZON_PT * 1.001)),
            "points running to infinity stop at the horizon: {mapped:?}"
        );
    }
}
