//! Rounded box geometry: corner arcs, side regions and pattern paths.
use crate::paint::boxes::{
    DASH_GAP_WIDTHS, DASH_LENGTH_WIDTHS, dash_gap, dot_gap, ring, side_runs,
};
use crate::paint::{BorderStyle, BoxDecoration, Contour, Fill, Point, Rect, Segment, Shape};

/// Cubic control distance as a fraction of the radius.
const KAPPA: f32 = 0.552;
const ROUND_CURVATURE: f32 = 1.0;
const CORNER_FLATNESS: f64 = 0.01;
const SIDE_TANGENTS: [Point; 4] = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]];
const GAUSS_LEGENDRE: [(f64, f64); 5] = [
    (0.0, 0.568_888_888_888_888_9),
    (-0.538_469_310_105_683, 0.478_628_670_499_366_5),
    (0.538_469_310_105_683, 0.478_628_670_499_366_5),
    (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
    (0.906_179_845_938_664, 0.236_926_885_056_189_1),
];
const LENGTH_INTERVALS: usize = 16;
const CROSSING_ITERATIONS: usize = 64;

/// Corner radii as `[horizontal, vertical]` in top-left, top-right, bottom-right, bottom-left order.
pub type Radii = [[f32; 2]; 4];

#[derive(Clone, Debug)]
enum Curve {
    Path(Contour),
    Arc([Point; 4]),
}
impl Curve {
    fn start(&self) -> Point {
        match self {
            Self::Path(path) => path.start,
            Self::Arc(points) => points[0],
        }
    }
    fn end(&self) -> Point {
        match self {
            Self::Path(path) => path
                .segments
                .last()
                .map_or(path.start, |segment| segment.end()),
            Self::Arc(points) => points[3],
        }
    }
    fn is_point(&self) -> bool {
        matches!(self, Self::Path(path) if path.segments.is_empty())
    }
    fn forward(&self) -> impl Iterator<Item = Segment> + '_ {
        let (arc, path) = match self {
            Self::Arc(p) => (Some(Segment::Cubic([p[1], p[2], p[3]])), &[][..]),
            Self::Path(path) => (None, path.segments.as_slice()),
        };
        arc.into_iter().chain(path.iter().copied())
    }
    fn backward(&self) -> impl Iterator<Item = Segment> + '_ {
        let (arc, path) = match self {
            Self::Arc(p) => (Some(Segment::Cubic([p[2], p[1], p[0]])), &[][..]),
            Self::Path(path) => (None, path.segments.as_slice()),
        };
        arc.into_iter()
            .chain(path.iter().enumerate().rev().map(|(index, segment)| {
                let start = if index == 0 {
                    self.start()
                } else {
                    path[index - 1].end()
                };
                match *segment {
                    Segment::Line(_) => Segment::Line(start),
                    Segment::Cubic([a, b, _]) => Segment::Cubic([b, a, start]),
                }
            }))
    }
}

fn corner_points(rect: Rect) -> [Point; 4] {
    [
        [rect.x, rect.y],
        [rect.x + rect.width, rect.y],
        [rect.x + rect.width, rect.y + rect.height],
        [rect.x, rect.y + rect.height],
    ]
}

fn corners(rect: Rect, radii: Radii, curvatures: [f32; 4]) -> [Curve; 4] {
    let points = corner_points(rect);
    std::array::from_fn(|corner| {
        let [rx, ry] = radii[corner];
        let c = points[corner];
        if rx <= 0.0 || ry <= 0.0 {
            return Curve::Path(Contour {
                start: c,
                segments: Vec::new(),
                closed: false,
            });
        }
        let (start, end) = match corner {
            0 => ([c[0], c[1] + ry], [c[0] + rx, c[1]]),
            1 => ([c[0] - rx, c[1]], [c[0], c[1] + ry]),
            2 => ([c[0], c[1] - ry], [c[0] - rx, c[1]]),
            _ => ([c[0] + rx, c[1]], [c[0], c[1] - ry]),
        };
        if curvatures[corner] != ROUND_CURVATURE {
            return Curve::Path(superellipse(start, c, end, curvatures[corner]));
        }
        let toward = |from: Point| {
            [
                from[0] + KAPPA * (c[0] - from[0]),
                from[1] + KAPPA * (c[1] - from[1]),
            ]
        };
        Curve::Arc([start, toward(start), toward(end), end])
    })
}

/// Untrimmed corner boundaries. Callers must clip these to the target box.
fn inset_corners(rect: Rect, radii: Radii, curvatures: [f32; 4], insets: [f32; 4]) -> [Curve; 4] {
    if insets == [0.0; 4]
        && curvatures
            .iter()
            .all(|value| *value <= -1.0 || *value >= 0.0)
    {
        return corners(rect, radii, curvatures);
    }
    let (target, adjusted) = inset(rect, radii, insets);
    let mut curves = corners(target, adjusted, [ROUND_CURVATURE; 4]);
    let points = corner_points(target);
    for corner in 0..4 {
        if curvatures[corner] == ROUND_CURVATURE {
            continue;
        }
        let start_axis = corner % 2;
        let axes = [start_axis, 1 - start_axis];
        let radius = axes.map(|axis| f64::from(radii[corner][axis]));
        let coverage = 2.0
            * (f64::from(radii[corner][0]) / f64::from(rect.width))
                .min(f64::from(radii[corner][1]) / f64::from(rect.height));
        let distances = [insets[(corner + 3) % 4], insets[corner]].map(f64::from);
        let inset = std::array::from_fn(|side| {
            let distance = distances[side];
            if distance >= 0.0 || radius[side] > -distance || coverage > 1.0 {
                distance
            } else {
                distance
                    * (1.0 - (1.0 + radius[side] / distance).powi(3) * (1.0 - coverage.powi(3)))
            }
        });
        let v3 = SIDE_TANGENTS[(corner + 1) % 4];
        let v2 = SIDE_TANGENTS[corner];
        let map = |point: [f64; 2]| {
            to32(std::array::from_fn(|axis| {
                f64::from(points[corner][axis])
                    + f64::from(v3[axis]) * point[0]
                    + f64::from(v2[axis]) * point[1]
            }))
        };
        curves[corner] = aligned_corner(radius, inset, curvatures[corner], map);
    }
    curves
}

/// CSS Borders 4 border-aligned corner clip-out curve in its inward-pointing side basis.
fn aligned_corner(
    radius: [f64; 2],
    inset: [f64; 2],
    curvature: f32,
    map: impl Fn([f64; 2]) -> Point,
) -> Curve {
    let point = || {
        Curve::Path(Contour {
            start: map([0.0; 2]),
            segments: Vec::new(),
            closed: false,
        })
    };
    let [start_radius, end_radius] = radius;
    let [start_inset, end_inset] = inset;
    if radius.into_iter().any(|radius| radius <= 0.0) || curvature == f32::INFINITY {
        return point();
    }
    let power = 2.0_f64.powf(-f64::from(curvature.abs()));
    let half = if curvature < 0.0 {
        -(-std::f64::consts::LN_2 * power).exp_m1()
    } else {
        (-std::f64::consts::LN_2 * power).exp()
    };
    let control =
        (half / (std::f64::consts::SQRT_2 - 1.0) - std::f64::consts::FRAC_1_SQRT_2).clamp(0.0, 1.0);
    let difference = (end_inset - start_inset).clamp(-start_radius, end_radius);
    if curvature <= 0.0 && (difference == -start_radius || difference == end_radius) {
        return point();
    }
    let start_control = if difference == 0.0 {
        control
    } else {
        let root = (start_radius * start_radius + end_radius * end_radius
            - difference * difference)
            .max(0.0)
            .sqrt();
        let normal_x = end_radius * difference + start_radius * root;
        let normal_y = -start_radius * difference + end_radius * root;
        let bevel = start_radius * normal_y / (start_radius * normal_y + end_radius * normal_x);
        if curvature < 0.0 {
            bevel * (2.0 * control)
        } else {
            1.0 - (1.0 - bevel) * (2.0 * (1.0 - control))
        }
    };
    let end_control = 2.0 * control - start_control;
    let normalize = |vector: [f64; 2]| {
        let length = vector[0].hypot(vector[1]);
        if length == 0.0 {
            vector
        } else {
            vector.map(|component| component / length)
        }
    };
    let start_normal = normalize([
        (1.0 - start_control) * start_radius,
        start_control * end_radius,
    ]);
    let end_normal = normalize([end_control * start_radius, (1.0 - end_control) * end_radius]);
    let mut start = [
        end_radius - end_inset + start_normal[0] * start_inset,
        -start_inset + start_normal[1] * start_inset,
    ];
    let mut end = [
        -end_inset + end_normal[0] * end_inset,
        start_radius - start_inset + end_normal[1] * end_inset,
    ];
    let start_tangent = [-start_normal[1], start_normal[0]];
    let end_tangent = [end_normal[1], -end_normal[0]];
    let miter_start = if start_inset < 0.0 && start_tangent[1] != 0.0 {
        [
            start[0] - start[1] * start_tangent[0] / start_tangent[1],
            0.0,
        ]
    } else {
        start
    };
    let miter_end = if end_inset < 0.0 && end_tangent[0] != 0.0 {
        [0.0, end[1] - end[0] * end_tangent[1] / end_tangent[0]]
    } else {
        end
    };
    if curvature >= 0.0 {
        start = miter_start;
        end = miter_end;
    }
    let segment_intersection = |a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]| {
        let v = [b[0] - a[0], b[1] - a[1]];
        let w = [d[0] - c[0], d[1] - c[1]];
        line_intersection(a, v, c, w).and_then(|(t, p)| (0.0..=1.0).contains(&t).then_some(p))
    };
    let mut intersection = None;
    if curvature < 0.0 && start_inset < 0.0 && end_inset < 0.0 {
        if -end_inset >= start_radius {
            intersection = segment_intersection(miter_start, start, miter_end, end);
        }
        if intersection.is_none() && -start_inset >= end_radius {
            intersection = segment_intersection(miter_end, end, miter_start, start);
        }
    }
    let mut path = Contour {
        start: map(miter_start),
        segments: Vec::new(),
        closed: false,
    };
    if let Some(intersection) = intersection {
        path.segments.push(Segment::Line(map(intersection)));
    } else if curvature != 0.0 {
        let mut outer = [end[0], start[1]];
        let mut exponent = curvature;
        if curvature > -1.0 && curvature < 0.0 {
            let tangent =
                line_intersection(start, start_tangent, end, end_tangent).map_or(start, |(_, p)| p);
            outer = [
                start[0] + end[0] - tangent[0],
                start[1] + end[1] - tangent[1],
            ];
            exponent = -1.0;
        }
        let curve = superellipse(map(start), map(outer), map(end), exponent);
        if curve.start != path.start {
            path.segments.push(Segment::Line(curve.start));
        }
        path.segments.extend(curve.segments);
    }
    let last = path
        .segments
        .last()
        .map_or(path.start, |segment| segment.end());
    let end = map(miter_end);
    if last != end {
        path.segments.push(Segment::Line(end));
    }
    Curve::Path(path)
}

fn line_intersection(
    a: [f64; 2],
    v: [f64; 2],
    b: [f64; 2],
    w: [f64; 2],
) -> Option<(f64, [f64; 2])> {
    let determinant = v[0] * w[1] - v[1] * w[0];
    if determinant == 0.0 {
        return None;
    }
    let t = ((b[0] - a[0]) * w[1] - (b[1] - a[1]) * w[0]) / determinant;
    Some((t, [a[0] + t * v[0], a[1] + t * v[1]]))
}

/// A convex superellipse has its greatest chord deviation where its tangent is parallel
/// to that chord. Split there until the physical error fits the point-coordinate precision.
fn superellipse(start: Point, corner: Point, end: Point, curvature: f32) -> Contour {
    debug_assert!(!curvature.is_nan());
    let power = 2.0_f64.powf(-f64::from(curvature.abs()));
    let map = |mut point: [f64; 2]| {
        if curvature < 0.0 {
            point.swap(0, 1);
        }
        std::array::from_fn(|axis| {
            f64::from(start[axis])
                + point[0] * (f64::from(corner[axis]) - f64::from(start[axis]))
                + point[1] * (f64::from(end[axis]) - f64::from(corner[axis]))
        })
    };
    let scale = start
        .into_iter()
        .chain(corner)
        .chain(end)
        .map(|value| f64::from(value).abs())
        .fold(1.0_f64, f64::max);
    let tolerance = CORNER_FLATNESS.max(2.0 * f64::from(f32::EPSILON) * scale);
    let mut segments = Vec::new();
    if power == 0.0 {
        segments.push(Segment::Line(to32(map([1.0, 0.0]))));
        segments.push(Segment::Line(end));
    } else if power == 1.0 {
        segments.push(Segment::Line(end));
    } else {
        let mut pending = vec![([0.0, 0.0], [1.0, 1.0])];
        while let Some((a, b)) = pending.pop() {
            let delta: [f64; 2] = std::array::from_fn(|axis| b[axis] - a[axis]);
            if delta[0] <= 0.0 || delta[1] <= 0.0 {
                segments.push(Segment::Line(to32(map(b))));
                continue;
            }
            let log_ratio = (delta[1].ln() - delta[0].ln()) / (1.0 - power);
            let softplus = |value: f64| value.max(0.0) + (-value.abs()).exp().ln_1p();
            let middle = [
                (-power * softplus(-log_ratio)).exp(),
                -(-power * softplus(log_ratio)).exp_m1(),
            ];
            let [pa, pb, pm] = [a, b, middle].map(map);
            let chord = [pb[0] - pa[0], pb[1] - pa[1]];
            let error = ((pm[0] - pa[0]) * chord[1] - (pm[1] - pa[1]) * chord[0]).abs()
                / chord[0].hypot(chord[1]);
            if error > tolerance && middle != a && middle != b {
                pending.push((middle, b));
                pending.push((a, middle));
            } else {
                segments.push(Segment::Line(to32(pb)));
            }
        }
    }
    Contour {
        start,
        segments,
        closed: false,
    }
}

/// Insets are in top/right/bottom/left order; each radius shrinks by its adjacent inset.
/// CSS Backgrounds 3 §6.1.1: a corner radius grown by `growth` like a shadow's spread, so
/// that sharp corners stay sharp; a negative growth shrinks it.
pub(crate) fn spread_radius(radius: f32, growth: f32) -> f32 {
    let growth = if growth > 0.0 && radius < growth {
        growth * (1.0 + (radius / growth - 1.0).powi(3))
    } else {
        growth
    };
    (radius + growth).max(0.0)
}

/// `rect` and its corner radii moved inward by `insets`; negative insets move outward with
/// spread corners.
pub(crate) fn inset(rect: Rect, radii: Radii, insets: [f32; 4]) -> (Rect, Radii) {
    let [top, right, bottom, left] = insets;
    let inner = Rect {
        x: rect.x + left,
        y: rect.y + top,
        width: rect.width - left - right,
        height: rect.height - top - bottom,
    };
    let shrink = [[left, top], [right, top], [right, bottom], [left, bottom]];
    let radii = std::array::from_fn(|corner| {
        [
            spread_radius(radii[corner][0], -shrink[corner][0]),
            spread_radius(radii[corner][1], -shrink[corner][1]),
        ]
    });
    (inner, radii)
}

fn contour(curves: &[Curve; 4]) -> Contour {
    let mut segments = Vec::new();
    for (corner, curve) in curves.iter().enumerate() {
        if corner > 0 {
            segments.push(Segment::Line(curve.start()));
        }
        segments.extend(curve.forward());
    }
    Contour {
        start: curves[0].start(),
        segments,
        closed: true,
    }
}

/// CSS Backgrounds 3 §4.5: all radii scaled down together until adjacent radii fit along
/// every side of a box of `[width, height]`.
pub(crate) fn constrain(radii: Radii, size: [f32; 2]) -> Radii {
    constrain_corners(radii, size, [ROUND_CURVATURE; 4])
}

fn constrain_corners(mut radii: Radii, [width, height]: [f32; 2], curvatures: [f32; 4]) -> Radii {
    let factor = [
        (width, radii[0][0] + radii[1][0]),
        (height, radii[1][1] + radii[2][1]),
        (width, radii[2][0] + radii[3][0]),
        (height, radii[3][1] + radii[0][1]),
    ]
    .into_iter()
    .filter(|(_, sum)| *sum > 0.0)
    .map(|(length, sum)| length / sum)
    .fold(1.0_f32, f32::min);
    if factor < 1.0 {
        radii = radii.map(|corner| corner.map(|radius| radius * factor));
    }
    if curvatures.iter().all(|curvature| *curvature >= 0.0) {
        return radii;
    }
    let origins = [
        [0.0, 0.0],
        [f64::from(width), 0.0],
        [f64::from(width), f64::from(height)],
        [0.0, f64::from(height)],
    ];
    let mut reduction = 1.0_f64;
    for first in 0..2 {
        let opposite = first + 2;
        if [first, opposite]
            .into_iter()
            .any(|corner| radii[corner].iter().any(|radius| *radius <= 0.0))
            || [first, opposite]
                .into_iter()
                .all(|corner| curvatures[corner] >= 0.0)
        {
            continue;
        }
        let hulls =
            [first, opposite].map(|corner| corner_hull(corner, radii[corner], curvatures[corner]));
        let origins = [origins[first], origins[opposite]];
        if !hulls_overlap(hulls, origins, reduction) {
            continue;
        }
        let (mut lower, mut upper) = (0.0, reduction);
        for _ in 0..CROSSING_ITERATIONS {
            let middle = f64::midpoint(lower, upper);
            if hulls_overlap(hulls, origins, middle) {
                upper = middle;
            } else {
                lower = middle;
            }
        }
        reduction = lower;
    }
    if reduction == 1.0 {
        return radii;
    }
    let mut factor = reduction as f32;
    loop {
        let reduced = radii.map(|corner| corner.map(|radius| radius * factor));
        let overlaps = (0..2).any(|first| {
            let opposite = first + 2;
            if [first, opposite]
                .into_iter()
                .any(|corner| reduced[corner].iter().any(|radius| *radius <= 0.0))
            {
                return false;
            }
            let hulls = [first, opposite]
                .map(|corner| corner_hull(corner, reduced[corner], curvatures[corner]));
            hulls_overlap(hulls, [origins[first], origins[opposite]], 1.0)
        });
        if !overlaps {
            return reduced;
        }
        // Rounding the factor or its products outward must not restore an overlap.
        factor = factor.next_down();
    }
}

/// Inner hull relative to the outer corner. Concave corners use the tangent at their
/// diagonal midpoint; the corrected Borders 4 hull intersects the x=0 and y=0 axes.
fn corner_hull(corner: usize, radii: [f32; 2], curvature: f32) -> [[f64; 2]; 5] {
    let half = if curvature < 0.0 {
        -(-std::f64::consts::LN_2 * 2.0_f64.powf(f64::from(curvature))).exp_m1()
    } else {
        0.5
    };
    let [rx, ry] = radii.map(f64::from);
    [
        [1.0, 1.0],
        [1.0, 0.0],
        [2.0 * half, 0.0],
        [0.0, 2.0 * half],
        [0.0, 1.0],
    ]
    .map(|[u, v]| match corner {
        0 => [(1.0 - v) * rx, (1.0 - u) * ry],
        1 => [-(1.0 - u) * rx, (1.0 - v) * ry],
        2 => [-(1.0 - v) * rx, -(1.0 - u) * ry],
        _ => [(1.0 - u) * rx, -(1.0 - v) * ry],
    })
}

fn hulls_overlap(hulls: [[[f64; 2]; 5]; 2], origins: [[f64; 2]; 2], scale: f64) -> bool {
    let [a, b] = std::array::from_fn(|corner| {
        hulls[corner]
            .map(|point| std::array::from_fn(|axis| origins[corner][axis] + scale * point[axis]))
    });
    for polygon in [&a, &b] {
        for edge in 0..polygon.len() {
            let [start, end] = [polygon[edge], polygon[(edge + 1) % polygon.len()]];
            let normal = [end[1] - start[1], start[0] - end[0]];
            if normal == [0.0; 2] {
                continue;
            }
            let project = |polygon: [[f64; 2]; 5]| {
                polygon
                    .into_iter()
                    .map(|point| point[0] * normal[0] + point[1] * normal[1])
                    .fold([f64::INFINITY, f64::NEG_INFINITY], |[min, max], value| {
                        [min.min(value), max.max(value)]
                    })
            };
            let [arange, brange] = [a, b].map(project);
            if arange[1] <= brange[0] || brange[1] <= arange[0] {
                return false;
            }
        }
    }
    true
}

pub(crate) fn rounded_rect(rect: Rect, radii: Radii) -> Contour {
    contour(&corners(rect, radii, [ROUND_CURVATURE; 4]))
}

fn to64(point: Point) -> [f64; 2] {
    [f64::from(point[0]), f64::from(point[1])]
}
fn to32(point: [f64; 2]) -> Point {
    [point[0] as f32, point[1] as f32]
}
fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// De Casteljau split; parameters outside `0..=1` extrapolate the cubic.
fn split(points: [Point; 4], t: f64) -> ([Point; 4], [Point; 4]) {
    let p = points.map(to64);
    let a = lerp(p[0], p[1], t);
    let b = lerp(p[1], p[2], t);
    let c = lerp(p[2], p[3], t);
    let ab = lerp(a, b, t);
    let bc = lerp(b, c, t);
    let mid = lerp(ab, bc, t);
    ([p[0], a, ab, mid].map(to32), [mid, bc, c, p[3]].map(to32))
}

fn point_at(points: [Point; 4], t: f64) -> [f64; 2] {
    let p = points.map(to64);
    let u = 1.0 - t;
    let weights = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    std::array::from_fn(|axis| (0..4).map(|i| weights[i] * p[i][axis]).sum())
}

fn speed(points: [Point; 4], t: f64) -> f64 {
    let p = points.map(to64);
    let u = 1.0 - t;
    let d: [f64; 2] = std::array::from_fn(|axis| {
        3.0 * u * u * (p[1][axis] - p[0][axis])
            + 6.0 * u * t * (p[2][axis] - p[1][axis])
            + 3.0 * t * t * (p[3][axis] - p[2][axis])
    });
    d[0].hypot(d[1])
}

/// Arc length between two parameters by composite Gauss-Legendre quadrature.
fn length(points: [Point; 4], from: f64, to: f64) -> f64 {
    let step = (to - from) / LENGTH_INTERVALS as f64;
    (0..LENGTH_INTERVALS)
        .map(|interval| {
            let centre = from + step * (interval as f64 + 0.5);
            GAUSS_LEGENDRE
                .iter()
                .map(|(node, weight)| weight * speed(points, centre + node * step / 2.0))
                .sum::<f64>()
                * step
                / 2.0
        })
        .sum()
}

/// Parameter where the cubic crosses the line through `origin` along `direction`.
fn crossing(points: [Point; 4], origin: Point, direction: Point) -> f64 {
    let origin = to64(origin);
    let direction = to64(direction);
    let side = |t: f64| {
        let p = point_at(points, t);
        (p[0] - origin[0]) * direction[1] - (p[1] - origin[1]) * direction[0]
    };
    let (mut low, mut high) = (0.0, 1.0);
    let low_side = side(low);
    if low_side.signum() == side(high).signum() {
        return if low_side.abs() <= side(high).abs() {
            low
        } else {
            high
        };
    }
    for _ in 0..CROSSING_ITERATIONS {
        let middle = f64::midpoint(low, high);
        if side(middle).signum() == low_side.signum() {
            low = middle;
        } else {
            high = middle;
        }
    }
    f64::midpoint(low, high)
}

fn offset(point: Point, direction: Point, distance: f32) -> Point {
    [
        point[0] + direction[0] * distance,
        point[1] + direction[1] * distance,
    ]
}

fn path_length(path: &Contour) -> f64 {
    let mut start = path.start;
    path.segments
        .iter()
        .map(|segment| {
            let measured = match *segment {
                Segment::Line(end) => {
                    f64::from(end[0] - start[0]).hypot(f64::from(end[1] - start[1]))
                }
                Segment::Cubic([a, b, end]) => length([start, a, b, end], 0.0, 1.0),
            };
            start = segment.end();
            measured
        })
        .sum()
}

/// Open corner paths are monotone across the adjacent sides' ownership diagonal.
fn split_path(path: &Contour, origin: Point, direction: Point) -> (Contour, Contour) {
    if path.segments.is_empty() {
        return (path.clone(), path.clone());
    }
    let side = |point: Point| {
        (f64::from(point[0]) - f64::from(origin[0])) * f64::from(direction[1])
            - (f64::from(point[1]) - f64::from(origin[1])) * f64::from(direction[0])
    };
    let mut selected = (0, 0.0);
    let mut nearest = side(path.start).abs();
    let mut start = path.start;
    for (index, segment) in path.segments.iter().enumerate() {
        let end = segment.end();
        let (before, after) = (side(start), side(end));
        if before == 0.0 || before.signum() != after.signum() {
            let t = match *segment {
                Segment::Line(_) => {
                    if before == 0.0 {
                        0.0
                    } else {
                        before / (before - after)
                    }
                }
                Segment::Cubic([a, b, end]) => crossing([start, a, b, end], origin, direction),
            };
            selected = (index, t);
            break;
        }
        if after.abs() < nearest {
            nearest = after.abs();
            selected = (index, 1.0);
        }
        start = end;
    }
    let (index, t) = selected;
    let start = if index == 0 {
        path.start
    } else {
        path.segments[index - 1].end()
    };
    let (middle, head, tail) = match path.segments[index] {
        Segment::Line(end) => {
            let middle = to32(lerp(to64(start), to64(end), t));
            (middle, Segment::Line(middle), Segment::Line(end))
        }
        Segment::Cubic([a, b, end]) => {
            let (head, tail) = split([start, a, b, end], t);
            (
                head[3],
                Segment::Cubic([head[1], head[2], head[3]]),
                Segment::Cubic([tail[1], tail[2], tail[3]]),
            )
        }
    };
    let mut before = path.segments[..index].to_vec();
    before.push(head);
    let mut after = vec![tail];
    after.extend_from_slice(&path.segments[index + 1..]);
    (
        Contour {
            start: path.start,
            segments: before,
            closed: false,
        },
        Contour {
            start: middle,
            segments: after,
            closed: false,
        },
    )
}

fn extend_path(path: &mut Contour, distance: f32, tangent: Point) {
    let Some(index) = path.segments.len().checked_sub(1) else {
        path.segments
            .push(Segment::Line(offset(path.start, tangent, distance)));
        return;
    };
    let start = if index == 0 {
        path.start
    } else {
        path.segments[index - 1].end()
    };
    match path.segments[index] {
        Segment::Line(end) => {
            let direction = [end[0] - start[0], end[1] - start[1]];
            let norm = direction[0].hypot(direction[1]);
            let direction = if norm > 0.0 {
                direction.map(|value| value / norm)
            } else {
                tangent
            };
            path.segments[index] = Segment::Line(offset(end, direction, distance));
        }
        Segment::Cubic([a, b, end]) => {
            let points = [start, a, b, end];
            let measured = length(points, 0.0, 1.0);
            if measured > 0.0 {
                let (head, _) = split(points, 1.0 + f64::from(distance) / measured);
                path.segments[index] = Segment::Cubic([head[1], head[2], head[3]]);
            } else {
                path.segments
                    .push(Segment::Line(offset(end, tangent, distance)));
            }
        }
    }
}

/// Rounded box layers derived from one border box and its used widths.
struct Geometry {
    rect: Rect,
    radii: Radii,
    widths: [f32; 4],
    curvatures: [f32; 4],
    outer_points: [Point; 4],
    inner_points: [Point; 4],
    outer: [Curve; 4],
    centre: [Curve; 4],
}
impl Geometry {
    fn new(rect: Rect, radii: Radii, widths: [f32; 4], curvatures: [f32; 4]) -> Self {
        let (inner_rect, _) = inset(rect, radii, widths);
        Self {
            rect,
            radii,
            widths,
            curvatures,
            outer_points: corner_points(rect),
            inner_points: corner_points(inner_rect),
            outer: inset_corners(rect, radii, curvatures, [0.0; 4]),
            centre: inset_corners(rect, radii, curvatures, widths.map(|width| width / 2.0)),
        }
    }

    /// Corner curves of the border box inset by fraction `t` of each side's width.
    fn ring(&self, t: f32) -> [Curve; 4] {
        inset_corners(
            self.rect,
            self.radii,
            self.curvatures,
            self.widths.map(|width| width * t),
        )
    }

    /// Region between the two corner diagonals that owns `side`.
    fn side_clip(&self, side: usize) -> Vec<Point> {
        let (a, b) = (side, (side + 1) % 4);
        let [outer_a, outer_b] = [self.outer_points[a], self.outer_points[b]];
        let [inner_a, inner_b] = [self.inner_points[a], self.inner_points[b]];
        if self.outer[a].is_point() && self.outer[b].is_point() {
            return vec![outer_a, outer_b, inner_b, inner_a];
        }
        let da = [inner_a[0] - outer_a[0], inner_a[1] - outer_a[1]];
        let db = [inner_b[0] - outer_b[0], inner_b[1] - outer_b[1]];
        let denominator = da[0] * db[1] - da[1] * db[0];
        if denominator.abs() <= f32::EPSILON {
            let reach = (outer_a[0] - outer_b[0]).abs()
                + (outer_a[1] - outer_b[1]).abs()
                + self.widths.iter().sum::<f32>();
            let far = |inner: Point, d: Point| {
                let scale = reach / d[0].hypot(d[1]);
                offset(inner, d, scale)
            };
            return vec![
                outer_a,
                outer_b,
                inner_b,
                far(inner_b, db),
                far(inner_a, da),
            ];
        }
        let t =
            ((outer_b[0] - outer_a[0]) * db[1] - (outer_b[1] - outer_a[1]) * db[0]) / denominator;
        vec![outer_a, outer_b, inner_b, offset(outer_a, da, t), inner_a]
    }

    /// Band of `side` between two rings, covering both adjacent arcs: outer, then reversed inner.
    fn side_shape(side: usize, outer: &[Curve; 4], inner: &[Curve; 4]) -> Contour {
        let (a, b) = (side, (side + 1) % 4);
        let mut segments = Vec::new();
        segments.extend(outer[a].forward());
        segments.push(Segment::Line(outer[b].start()));
        segments.extend(outer[b].forward());
        segments.push(Segment::Line(inner[b].end()));
        segments.extend(inner[b].backward());
        segments.push(Segment::Line(inner[a].end()));
        segments.extend(inner[a].backward());
        Contour {
            start: outer[a].start(),
            segments,
            closed: true,
        }
    }

    fn diagonal(&self, corner: usize) -> (Point, Point) {
        let outer = self.outer_points[corner];
        let inner = self.inner_points[corner];
        (outer, [inner[0] - outer[0], inner[1] - outer[1]])
    }

    /// Dots run between the corner diagonals and overshoot the far one by half a width.
    fn dotted(&self, side: usize, width: f32) -> (Contour, f32) {
        let (a, b) = (side, (side + 1) % 4);
        let tangent = SIDE_TANGENTS[side];
        let mut segments = Vec::new();
        let (start, start_length) = match &self.centre[a] {
            Curve::Path(path) => {
                let (origin, direction) = self.diagonal(a);
                let (_, tail) = split_path(path, origin, direction);
                let measured = path_length(&tail);
                segments.extend_from_slice(&tail.segments);
                (tail.start, measured)
            }
            Curve::Arc(points) => {
                let (origin, direction) = self.diagonal(a);
                let t = crossing(*points, origin, direction);
                let (_, tail) = split(*points, t);
                segments.push(Segment::Cubic([tail[1], tail[2], tail[3]]));
                (tail[0], length(*points, t, 1.0))
            }
        };
        let straight_from = self.centre[a].end();
        let straight_to = self.centre[b].start();
        let straight =
            f64::from((straight_to[0] - straight_from[0]).hypot(straight_to[1] - straight_from[1]));
        let end_length = match &self.centre[b] {
            Curve::Path(path) => {
                let (origin, direction) = self.diagonal(b);
                let (mut head, _) = split_path(path, origin, direction);
                let measured = path_length(&head);
                extend_path(&mut head, width / 2.0, tangent);
                segments.push(Segment::Line(head.start));
                segments.extend(head.segments);
                measured
            }
            Curve::Arc(points) => {
                segments.push(Segment::Line(straight_to));
                let (origin, direction) = self.diagonal(b);
                let t = crossing(*points, origin, direction);
                let reached = length(*points, 0.0, t);
                let end = if reached > 0.0 {
                    t * (reached + f64::from(width) / 2.0) / reached
                } else {
                    t
                };
                let (head, _) = split(*points, end);
                segments.push(Segment::Cubic([head[1], head[2], head[3]]));
                reached
            }
        };
        let between = (start_length + straight + end_length) as f32;
        (
            Contour {
                start,
                segments,
                closed: false,
            },
            dot_gap(between, width),
        )
    }

    /// Dashes cover both corner arcs; the gap fits half of each corner plus the straight run.
    fn dashed(&self, side: usize, width: f32) -> (Contour, f32) {
        let (a, b) = (side, (side + 1) % 4);
        let tangent = SIDE_TANGENTS[side];
        let half = width / 2.0;
        let mut segments = Vec::new();
        let mut fitted = 0.0;
        let start = match &self.centre[a] {
            Curve::Path(path) => {
                if path.segments.is_empty() {
                    fitted += f64::from(half);
                    offset(path.start, tangent, -half)
                } else {
                    fitted += path_length(path) / 2.0;
                    segments.extend_from_slice(&path.segments);
                    path.start
                }
            }
            Curve::Arc(points) => {
                fitted += length(*points, 0.0, 1.0) / 2.0;
                segments.extend(self.centre[a].forward());
                points[0]
            }
        };
        let straight_from = self.centre[a].end();
        let straight_to = self.centre[b].start();
        fitted +=
            f64::from((straight_to[0] - straight_from[0]).hypot(straight_to[1] - straight_from[1]));
        match &self.centre[b] {
            Curve::Path(path) => {
                if path.segments.is_empty() {
                    fitted += f64::from(half);
                    segments.push(Segment::Line(offset(path.start, tangent, half)));
                } else {
                    fitted += path_length(path) / 2.0;
                    segments.push(Segment::Line(path.start));
                    segments.extend_from_slice(&path.segments);
                }
            }
            Curve::Arc(points) => {
                fitted += length(*points, 0.0, 1.0) / 2.0;
                segments.push(Segment::Line(straight_to));
                segments.extend(self.centre[b].forward());
            }
        }
        (
            Contour {
                start,
                segments,
                closed: false,
            },
            dash_gap(fitted as f32, width),
        )
    }
}

fn uniform_dashes<I>(decoration: &BoxDecoration<I>, widths: [f32; 4]) -> bool {
    let first = decoration.borders[0];
    widths
        .iter()
        .all(|width| *width > 0.0 && *width == widths[0])
        && decoration
            .borders
            .iter()
            .all(|border| border.style == BorderStyle::Dashed && border.color == first.color)
}

/// Border fills for a box with at least one rounded corner.
pub(crate) fn border_fills<I>(
    rect: Rect,
    decoration: &BoxDecoration<I>,
    widths: [f32; 4],
    inner_widths: [f32; 4],
    radii: Radii,
) -> Vec<Fill> {
    let geometry = Geometry::new(rect, radii, inner_widths, [ROUND_CURVATURE; 4]);
    let mut fills = Vec::new();
    let color = decoration.borders[0].color;
    if let Some(boundaries) = ring(decoration, widths) {
        if color.alpha > 0.0 {
            fills.push(Fill {
                shape: Shape::Path {
                    contours: boundaries
                        .into_iter()
                        .map(|t| contour(&geometry.ring(t)))
                        .collect(),
                    even_odd: true,
                    clip: None,
                },
                color,
            });
        }
        return fills;
    }
    if uniform_dashes(decoration, widths) {
        if color.alpha > 0.0 {
            let width = widths[0];
            fills.push(Fill {
                shape: Shape::Pattern {
                    contours: vec![contour(&geometry.centre)],
                    width,
                    dash: [DASH_LENGTH_WIDTHS * width, DASH_GAP_WIDTHS * width],
                    round: false,
                    clip: None,
                },
                color,
            });
        }
        return fills;
    }
    for side in side_runs(widths, |_, _| false).into_iter().flatten() {
        let border = decoration.borders[side];
        let width = widths[side];
        if width <= 0.0 || border.color.alpha <= 0.0 {
            continue;
        }
        let clip = Some(geometry.side_clip(side));
        if !matches!(border.style, BorderStyle::Dotted | BorderStyle::Dashed) {
            for band in border.bands(side) {
                fills.push(Fill {
                    shape: Shape::Path {
                        contours: vec![Geometry::side_shape(
                            side,
                            &geometry.ring(band.from),
                            &geometry.ring(band.to),
                        )],
                        even_odd: false,
                        clip: clip.clone(),
                    },
                    color: band.color,
                });
            }
            continue;
        }
        let dotted = border.style == BorderStyle::Dotted;
        let (stroke, gap) = if dotted {
            geometry.dotted(side, width)
        } else {
            geometry.dashed(side, width)
        };
        fills.push(Fill {
            shape: Shape::Pattern {
                contours: vec![stroke],
                width,
                dash: [
                    if dotted {
                        0.0
                    } else {
                        DASH_LENGTH_WIDTHS * width
                    },
                    gap,
                ],
                round: dotted,
                clip,
            },
            color: border.color,
        });
    }
    fills
}

#[cfg(test)]
mod path_tests {
    use super::*;

    const GEOMETRY_TOLERANCE: f32 = 0.000_01;

    #[test]
    fn unequal_thin_corner_offsets_retain_small_finite_curvature() {
        // High-precision evaluation of the border-aligned normals at curvature -1e-6.
        const EXPECTED: [f64; 2] = [0.044_877_496_983_655_67, 7.999_974_124_648_716];
        const ENDPOINT_ERROR: f64 = 0.000_001;
        let curves = inset_corners(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 2000.0,
                height: 2000.0,
            },
            [[1000.0, 0.000_1]; 4],
            [-0.000_001; 4],
            [2.0, 4.0, 6.0, 8.0],
        );
        for (actual, expected) in curves[0].start().into_iter().map(f64::from).zip(EXPECTED) {
            assert!(
                (actual - expected).abs() < ENDPOINT_ERROR,
                "finite curvature must retain its amplified normal change: {actual} vs {expected}"
            );
        }
    }

    #[test]
    fn machine_small_negative_curvatures_converge_to_the_bevel_offset() {
        const LIMIT_ERROR: f64 = 0.001;
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 2000.0,
            height: 2000.0,
        };
        for radius in [[20.0; 2], [0.000_1, 1000.0], [1000.0, 0.000_1]] {
            for insets in [[0.0; 4], [4.0; 4], [-4.0; 4], [2.0, 4.0, 6.0, 8.0]] {
                let bevels = inset_corners(rect, [radius; 4], [0.0; 4], insets);
                for curvature in [-f32::MIN_POSITIVE, -f32::EPSILON] {
                    let curves = inset_corners(rect, [radius; 4], [curvature; 4], insets);
                    for (curve, bevel) in curves.iter().zip(&bevels) {
                        if bevel.is_point() {
                            assert!(curve.is_point());
                            continue;
                        }
                        let [a, b] = [bevel.start(), bevel.end()].map(to64);
                        let d = [b[0] - a[0], b[1] - a[1]];
                        let length = d[0].hypot(d[1]);
                        for point in
                            std::iter::once(curve.start()).chain(curve.forward().map(Segment::end))
                        {
                            let point = to64(point);
                            let v = [point[0] - a[0], point[1] - a[1]];
                            let distance = (v[0] * d[1] - v[1] * d[0]).abs() / length;
                            let along = (v[0] * d[0] + v[1] * d[1]) / length;
                            assert!(
                                distance <= LIMIT_ERROR
                                    && along >= -LIMIT_ERROR
                                    && along <= length + LIMIT_ERROR,
                                "bevel limit: curvature={curvature}, radius={radius:?}, insets={insets:?}, point={point:?}, distance={distance}, along={along}, length={length}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn near_bevel_and_extreme_offset_curves_stay_finite_and_bounded() {
        for curvature in [
            -f32::MIN_POSITIVE,
            -0.000_001,
            -0.01,
            -0.5,
            0.000_001,
            0.5,
            2.0,
            -128.0,
            128.0,
        ] {
            for radii in [[0.000_1, 1000.0], [1000.0, 0.000_1], [20.0, 20.0]] {
                for insets in [[0.0; 4], [4.0; 4], [-40.0; 4], [25.0, 30.0, 4.0, 0.0]] {
                    let curves = inset_corners(
                        Rect {
                            x: -10.0,
                            y: 20.0,
                            width: 2000.0,
                            height: 2000.0,
                        },
                        [radii; 4],
                        [curvature; 4],
                        insets,
                    );
                    for (corner, curve) in curves.iter().enumerate() {
                        let points: Vec<_> = std::iter::once(curve.start())
                            .chain(curve.forward().map(Segment::end))
                            .collect();
                        assert!(
                            points.len() < 16384,
                            "unbounded offset path: {curvature}, {radii:?}, {insets:?}"
                        );
                        for point in points {
                            assert!(
                                point
                                    .into_iter()
                                    .all(|value| value.is_finite() && value.abs() < 4000.0),
                                "offset excursion: curvature={curvature}, corner={corner}, radii={radii:?}, insets={insets:?}, point={point:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn unequal_insets_beyond_the_radius_keep_the_full_border_thickness() {
        let curves = inset_corners(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 200.0,
            },
            [[20.0; 2]; 4],
            [f32::NEG_INFINITY; 4],
            [25.0, 30.0, 0.0, 0.0],
        );
        assert_eq!(
            curves[1].forward().next(),
            Some(Segment::Line([155.0, 50.0])),
            "signed adjusted radii must not discard inset distances beyond the radius"
        );
    }

    #[test]
    fn shaped_outsets_include_the_corners_box_coverage() {
        for (size, expected) in [
            ([40.0, 40.0], [[20.0, -40.0], [80.0, 20.0]]),
            ([80.0, 60.0], [[64.375, -40.0], [120.0, 15.625]]),
        ] {
            let curves = inset_corners(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: size[0],
                    height: size[1],
                },
                [[20.0; 2]; 4],
                [2.0; 4],
                [-40.0; 4],
            );
            assert_eq!(
                [curves[1].start(), curves[1].end()],
                expected,
                "large outset uses both the radius and its coverage of the border box"
            );
        }
    }

    #[test]
    fn scoop_offsets_preserve_the_circle_and_outset_tangent_extensions() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 60.0,
        };
        for inset in [4.0, -4.0] {
            let curves = inset_corners(rect, [[20.0; 2]; 4], [-1.0; 4], [inset; 4]);
            let Curve::Path(path) = &curves[1] else {
                panic!("scoop path expected")
            };
            let radius = f64::from(20.0 + inset);
            let mut points: Vec<_> = std::iter::once(path.start)
                .chain(path.segments.iter().map(|segment| segment.end()))
                .collect();
            if inset < 0.0 {
                assert_eq!(
                    points.remove(0),
                    [64.0, -4.0],
                    "outset starts on the extended top edge"
                );
                assert_eq!(
                    points.pop(),
                    Some([84.0, 16.0]),
                    "outset ends on the extended right edge"
                );
            }
            for point in points {
                let distance = (f64::from(point[0]) - 80.0).hypot(f64::from(point[1]));
                assert!(
                    (distance - radius).abs() < f64::from(GEOMETRY_TOLERANCE),
                    "scoop inset={inset}, point={point:?}, radius={distance}, expected={radius}"
                );
            }
        }
    }

    #[test]
    fn unequal_notch_insets_follow_each_adjacent_border() {
        let curves = inset_corners(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 60.0,
            },
            [[20.0; 2]; 4],
            [f32::NEG_INFINITY; 4],
            [2.0, 4.0, 6.0, 8.0],
        );
        let expected = [[22.0, 28.0], [58.0, 24.0], [54.0, 36.0], [26.0, 32.0]];
        for (curve, expected) in curves.iter().zip(expected) {
            assert_eq!(
                curve.forward().next(),
                Some(Segment::Line(expected)),
                "the indentation carries the adjacent border's thickness"
            );
        }
    }

    #[test]
    fn large_concave_outsets_remove_the_self_intersecting_tails() {
        let curves = inset_corners(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 60.0,
            },
            [[20.0; 2]; 4],
            [f32::NEG_INFINITY; 4],
            [-30.0; 4],
        );
        let curve = &curves[1];
        assert_eq!(curve.start(), [90.0, -30.0]);
        assert_eq!(
            curve.forward().collect::<Vec<_>>(),
            [Segment::Line([90.0, -10.0]), Segment::Line([110.0, -10.0])]
        );
        for curvature in [-0.5, -1.0, -2.0] {
            let curves = inset_corners(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 80.0,
                    height: 60.0,
                },
                [[20.0; 2]; 4],
                [curvature; 4],
                [-40.0; 4],
            );
            let Curve::Path(path) = &curves[1] else {
                panic!("concave clip-out expected")
            };
            assert_eq!(
                path.segments.len(),
                2,
                "outset={curvature} must cut intersecting miter tails"
            );
        }
    }

    #[test]
    fn consumed_concave_corners_leave_no_cutout() {
        for curvature in [f32::NEG_INFINITY, -1.0, -0.5, 0.0] {
            let curves = inset_corners(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 80.0,
                    height: 60.0,
                },
                [[20.0; 2]; 4],
                [curvature; 4],
                [25.0, 0.0, 0.0, 0.0],
            );
            assert!(
                curves[1].is_point(),
                "top inset consumes the entire corner: {curvature}"
            );
            assert_eq!(curves[1].start(), [80.0, 25.0]);
        }
    }

    #[test]
    fn concave_inner_corners_expand_the_cutout_by_the_border_width() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 60.0,
        };
        let geometry = Geometry::new(rect, [[20.0; 2]; 4], [4.0; 4], [f32::NEG_INFINITY; 4]);
        let corners = geometry.ring(1.0);
        let expected = [
            ([0.0, 24.0], [24.0, 24.0], [24.0, 0.0]),
            ([56.0, 0.0], [56.0, 24.0], [80.0, 24.0]),
            ([80.0, 36.0], [56.0, 36.0], [56.0, 60.0]),
            ([24.0, 60.0], [24.0, 36.0], [0.0, 36.0]),
        ];
        for (corner, (start, middle, end)) in corners.iter().zip(expected) {
            assert_eq!(corner.start(), start);
            assert_eq!(
                corner.forward().collect::<Vec<_>>(),
                [Segment::Line(middle), Segment::Line(end)],
                "notch inner boundary must retain border thickness along the concavity"
            );
        }
    }

    #[test]
    fn bevel_inner_edge_has_the_requested_perpendicular_distance() {
        let curves = inset_corners(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 60.0,
            },
            [[20.0; 2]; 4],
            [0.0; 4],
            [4.0; 4],
        );
        let curve = &curves[1];
        let shift = 4.0 / std::f32::consts::SQRT_2;
        let expected = [[60.0 - shift, shift], [80.0 - shift, 20.0 + shift]];
        for (actual, expected) in [curve.start(), curve.end()].into_iter().zip(expected) {
            for axis in 0..2 {
                assert!(
                    (actual[axis] - expected[axis]).abs() < GEOMETRY_TOLERANCE,
                    "bevel normal offset: {actual:?} vs {expected:?}"
                );
            }
        }
    }

    #[test]
    fn asymmetric_notch_reduction_remains_disjoint_after_float_rounding() {
        let radii = [[40.127_224; 2], [0.0; 2], [54.841_87; 2], [0.0; 2]];
        let actual = constrain_corners(radii, [80.0; 2], [f32::NEG_INFINITY; 4]);
        let combined = f64::from(actual[0][0]) + f64::from(actual[2][0]);
        assert!(
            combined <= 80.0,
            "rounded opposite notch widths overlap: {actual:?}, sum={combined}"
        );
        assert_eq!(
            constrain_corners(actual, [80.0; 2], [f32::NEG_INFINITY; 4]),
            actual,
            "used radii must be stable on a second constraint pass"
        );
    }

    #[test]
    fn opposite_hulls_use_each_corners_curvature() {
        for first in 0..4 {
            let opposite = (first + 2) % 4;
            let mut radii = [[0.0; 2]; 4];
            radii[first] = [64.0, 48.0];
            radii[opposite] = [64.0, 48.0];
            for (shapes, expected) in [
                ([f32::NEG_INFINITY, 0.0], [160.0 / 3.0, 40.0]),
                (
                    [-1.0; 2],
                    [
                        40.0 * std::f32::consts::SQRT_2,
                        30.0 * std::f32::consts::SQRT_2,
                    ],
                ),
            ] {
                let mut curvatures = [ROUND_CURVATURE; 4];
                curvatures[first] = shapes[0];
                curvatures[opposite] = shapes[1];
                let actual = constrain_corners(radii, [80.0, 60.0], curvatures);
                for corner in [first, opposite] {
                    for (value, expected) in actual[corner].into_iter().zip(expected) {
                        assert!(
                            (value - expected).abs() < GEOMETRY_TOLERANCE,
                            "corner={corner}, shapes={shapes:?}, actual={actual:?}, expected={expected}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn opposite_notches_reduce_every_radius_by_one_common_factor() {
        for first in 0..2 {
            let mut radii = [[4.0, 3.0]; 4];
            radii[first] = [64.0, 48.0];
            radii[first + 2] = [64.0, 48.0];
            let mut curvatures = [ROUND_CURVATURE; 4];
            curvatures[first] = f32::NEG_INFINITY;
            curvatures[first + 2] = f32::NEG_INFINITY;
            let actual = constrain_corners(radii, [80.0, 60.0], curvatures);
            let expected = radii.map(|radii| radii.map(|radius| radius * 0.625));
            assert_eq!(
                actual,
                expected,
                "opposite corners {first}/{} must meet without overlap",
                first + 2
            );
            let touching = constrain_corners(actual, [80.0, 60.0], curvatures);
            assert_eq!(touching, actual, "touching hulls must not shrink again");
        }
    }

    #[test]
    fn concave_radius_constraints_preserve_axis_separated_and_sharp_corners() {
        let separated = [[70.0, 10.0], [0.0; 2], [70.0, 10.0], [0.0; 2]];
        assert_eq!(
            constrain_corners(separated, [80.0, 60.0], [f32::NEG_INFINITY; 4]),
            separated,
            "opposite notches separated vertically may overlap horizontally"
        );
        let sharp = [[70.0, 50.0], [0.0; 2], [0.0, 50.0], [0.0; 2]];
        assert_eq!(
            constrain_corners(sharp, [80.0, 60.0], [f32::NEG_INFINITY; 4]),
            sharp,
            "zero-radius corners have no concave hull"
        );
    }

    #[test]
    fn extreme_curvatures_produce_finite_monotone_bounded_paths() {
        for curvature in [
            f32::MIN_POSITIVE,
            0.000_001,
            0.999_999,
            1.000_001,
            32.0,
            128.0,
            1023.0,
            f32::MAX,
        ] {
            for sign in [-1.0, 1.0] {
                for radius in [0.000_001, 1.0, 1000.0, 1.0e20_f32] {
                    let rect = Rect {
                        x: -radius,
                        y: 0.0,
                        width: radius,
                        height: radius,
                    };
                    let curves = corners(rect, [[radius; 2]; 4], [sign * curvature; 4]);
                    let Curve::Path(path) = &curves[1] else {
                        panic!("non-round path expected")
                    };
                    assert!(
                        path.segments.len() < 16384,
                        "unbounded subdivision: {curvature}, {radius}: {}",
                        path.segments.len()
                    );
                    let mut previous = path.start;
                    for segment in &path.segments {
                        let point = segment.end();
                        assert!(
                            point.into_iter().all(f32::is_finite),
                            "nonfinite point: {curvature}, {radius}: {point:?}"
                        );
                        assert!(
                            point[0] >= previous[0] && point[1] >= previous[1],
                            "nonmonotone: {curvature}, {radius}: {previous:?} -> {point:?}"
                        );
                        assert!(
                            point[0] <= 0.0 && point[1] <= radius,
                            "outside corner: {point:?}"
                        );
                        previous = point;
                    }
                    assert_eq!(previous, [0.0, radius]);
                }
            }
        }
    }

    #[test]
    fn fractional_superellipses_match_independent_parametric_samples() {
        const SAMPLE_COUNT: usize = 2048;
        for curvature in [0.01_f32, 0.5, 1.5, 2.3, 8.0, -0.01, -0.5, -1.0, -2.3, -8.0] {
            for [rx, ry] in [[25.0, 15.0], [200.0, 2.0], [0.002, 150.0]] {
                let rect = Rect {
                    x: 10.0,
                    y: -20.0,
                    width: rx * 2.0,
                    height: ry * 2.0,
                };
                let curves = corners(rect, [[rx, ry]; 4], [curvature; 4]);
                let exponent = 2.0_f64.powf(f64::from(curvature.abs()));
                let scale = f64::from(rect.width.max(rect.height) + rect.x.abs() + rect.y.abs());
                let max_error = 0.01 + 2.0 * f64::from(f32::EPSILON) * scale;
                for (corner, curve) in curves.iter().enumerate() {
                    let Curve::Path(path) = curve else {
                        panic!("fractional corner requires a piecewise contour: {curvature}")
                    };
                    assert!(
                        path.segments.len() < 512,
                        "excessive subdivision: {curvature}, {rx}x{ry}: {}",
                        path.segments.len()
                    );
                    let points: Vec<_> = std::iter::once(path.start)
                        .chain(path.segments.iter().map(|segment| segment.end()))
                        .map(to64)
                        .collect();
                    // Sample both Cartesian axes so near-angular endpoint layers are covered.
                    for sample in 0..=SAMPLE_COUNT {
                        let t = sample as f64 / SAMPLE_COUNT as f64;
                        for mut p in [
                            [t, 1.0 - (1.0 - t.powf(exponent)).powf(1.0 / exponent)],
                            [(1.0 - (1.0 - t).powf(exponent)).powf(1.0 / exponent), t],
                        ] {
                            if curvature < 0.0 {
                                p.swap(0, 1);
                            }
                            let [x, y] = match corner {
                                0 => [p[1], 1.0 - p[0]],
                                1 => [1.0 + p[0], p[1]],
                                2 => [2.0 - p[1], 1.0 + p[0]],
                                _ => [1.0 - p[0], 2.0 - p[1]],
                            };
                            let p = [
                                f64::from(rect.x) + x * f64::from(rx),
                                f64::from(rect.y) + y * f64::from(ry),
                            ];
                            let distance = points
                                .windows(2)
                                .map(|edge| {
                                    let [a, b] = [edge[0], edge[1]];
                                    let d = [b[0] - a[0], b[1] - a[1]];
                                    let norm = d[0] * d[0] + d[1] * d[1];
                                    let t = if norm == 0.0 {
                                        0.0
                                    } else {
                                        ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / norm
                                    }
                                    .clamp(0.0, 1.0);
                                    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
                                })
                                .fold(f64::INFINITY, f64::min);
                            assert!(
                                distance <= max_error,
                                "curvature={curvature}, corner={corner}, radii={rx}x{ry}, sample={sample}, point={p:?}, distance={distance}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn angular_corners_follow_box_orientation_and_keep_zero_radii_sharp() {
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            width: 80.0,
            height: 60.0,
        };
        let starts = [[10.0, 35.0], [65.0, 20.0], [90.0, 65.0], [35.0, 80.0]];
        let ends = [[35.0, 20.0], [90.0, 35.0], [65.0, 80.0], [10.0, 65.0]];
        let notches = [[35.0, 35.0], [65.0, 35.0], [65.0, 65.0], [35.0, 65.0]];
        let squares = [[10.0, 20.0], [90.0, 20.0], [90.0, 80.0], [10.0, 80.0]];
        for curvature in [0.0, f32::NEG_INFINITY, f32::INFINITY] {
            let curves = corners(rect, [[25.0, 15.0]; 4], [curvature; 4]);
            for (index, curve) in curves.iter().enumerate() {
                assert_eq!(
                    curve.start(),
                    starts[index],
                    "curvature={curvature}, corner={index}"
                );
                let expected = if curvature == 0.0 {
                    vec![Segment::Line(ends[index])]
                } else {
                    let vertex = if curvature.is_sign_positive() {
                        squares[index]
                    } else {
                        notches[index]
                    };
                    vec![Segment::Line(vertex), Segment::Line(ends[index])]
                };
                assert_eq!(
                    curve.forward().collect::<Vec<_>>(),
                    expected,
                    "curvature={curvature}, corner={index}"
                );
            }
            for curve in corners(rect, [[0.0, 15.0]; 4], [curvature; 4]) {
                assert!(
                    curve.is_point(),
                    "zero horizontal radius must leave a sharp corner"
                );
            }
        }
    }

    #[test]
    fn cubic_corner_cut_and_extension_preserve_the_parametric_curve() {
        let path = Contour {
            start: [0.0, 0.0],
            segments: vec![
                Segment::Cubic([[3.0, 3.0], [6.0, 6.0], [9.0, 9.0]]),
                Segment::Line([15.0, 9.0]),
            ],
            closed: false,
        };
        let (mut head, tail) = split_path(&path, [4.5, 0.0], [0.0, 1.0]);
        assert_eq!(
            head.segments,
            [Segment::Cubic([[1.5, 1.5], [3.0, 3.0], [4.5, 4.5]])]
        );
        assert_eq!(tail.start, [4.5, 4.5]);
        assert_eq!(
            tail.segments,
            [
                Segment::Cubic([[6.0, 6.0], [7.5, 7.5], [9.0, 9.0]]),
                Segment::Line([15.0, 9.0]),
            ]
        );
        extend_path(&mut head, 1.5 * std::f32::consts::SQRT_2, [1.0, 0.0]);
        let Segment::Cubic(points) = head.segments[0] else {
            panic!("extension keeps a cubic")
        };
        for (actual, expected) in points.into_iter().zip([[2.0, 2.0], [4.0, 4.0], [6.0, 6.0]]) {
            for axis in 0..2 {
                assert!(
                    (actual[axis] - expected[axis]).abs() < GEOMETRY_TOLERANCE,
                    "straight cubic extension: {actual:?} versus {expected:?}"
                );
            }
        }
    }

    #[test]
    fn corner_vertex_and_zero_length_segments_do_not_lose_path_length() {
        let path = Contour {
            start: [0.0, 0.0],
            segments: vec![
                Segment::Line([0.0, 0.0]),
                Segment::Line([9.0, 0.0]),
                Segment::Line([9.0, 9.0]),
            ],
            closed: false,
        };
        let (head, tail) = split_path(&path, [9.0, 0.0], [0.0, 1.0]);
        assert_eq!(tail.start, [9.0, 0.0]);
        assert_eq!([path_length(&head), path_length(&tail)], [9.0, 9.0]);
        let (head, tail) = split_path(&path, [20.0, 0.0], [0.0, 1.0]);
        assert_eq!(
            [path_length(&head), path_length(&tail)],
            [9.0, 9.0],
            "equal-distance endpoints choose the first nearest corner vertex"
        );
    }

    #[test]
    fn patterned_border_splits_piecewise_corners_at_side_diagonals() {
        let mut geometry = Geometry::new(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 60.0,
            },
            [[0.0; 2]; 4],
            [4.0; 4],
            [ROUND_CURVATURE; 4],
        );
        for (corner, start, end) in [
            (0, [2.0, 20.0], [20.0, 2.0]),
            (1, [80.0, 2.0], [98.0, 20.0]),
        ] {
            geometry.centre[corner] = Curve::Path(Contour {
                start,
                segments: vec![Segment::Line(end)],
                closed: false,
            });
        }
        let (dotted, _) = geometry.dotted(0, 4.0);
        assert_eq!(
            dotted.start,
            [11.0, 11.0],
            "first side diagonal cuts the bevel halfway"
        );
        let end = dotted.segments.last().unwrap().end();
        let extension = std::f32::consts::SQRT_2;
        assert_eq!(
            end,
            [89.0 + extension, 11.0 + extension],
            "last dot extends half its width along the bevel"
        );
        let (dashed, _) = geometry.dashed(0, 4.0);
        assert_eq!(dashed.start, [2.0, 20.0]);
        assert_eq!(
            dashed.segments,
            [
                Segment::Line([20.0, 2.0]),
                Segment::Line([80.0, 2.0]),
                Segment::Line([98.0, 20.0]),
            ],
            "dashes cover the full adjacent corner paths"
        );
    }

    #[test]
    fn piecewise_corner_cut_keeps_both_sides_and_reverses_controls() {
        let path = Contour {
            start: [0.0, 0.0],
            segments: vec![Segment::Line([10.0, 0.0]), Segment::Line([10.0, 10.0])],
            closed: false,
        };
        let (head, tail) = split_path(&path, [5.0, 0.0], [0.0, 1.0]);
        assert_eq!(head.start, path.start);
        assert_eq!(head.segments, [Segment::Line([5.0, 0.0])]);
        assert_eq!(tail.start, [5.0, 0.0]);
        assert_eq!(tail.segments, path.segments);
        assert_eq!(path_length(&head), 5.0);
        assert_eq!(path_length(&tail), 15.0);
        let curve = Curve::Path(Contour {
            start: [2.0, 3.0],
            segments: vec![
                Segment::Line([4.0, 5.0]),
                Segment::Cubic([[6.0, 7.0], [8.0, 9.0], [10.0, 11.0]]),
            ],
            closed: false,
        });
        assert_eq!(
            curve.backward().collect::<Vec<_>>(),
            [
                Segment::Cubic([[8.0, 9.0], [6.0, 7.0], [4.0, 5.0]]),
                Segment::Line([2.0, 3.0]),
            ]
        );
    }
}
