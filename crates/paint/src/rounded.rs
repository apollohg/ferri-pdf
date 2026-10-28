//! Rounded box geometry: corner arcs, side regions and pattern paths.
use crate::boxes::{DASH_GAP_WIDTHS, DASH_LENGTH_WIDTHS, dash_gap, dot_gap, ring, side_runs};
use crate::{BorderStyle, BoxDecoration, Contour, Fill, Point, Rect, Segment, Shape};

/// Cubic control distance as a fraction of the radius.
const KAPPA: f32 = 0.552;
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

#[derive(Clone, Copy, Debug)]
enum Curve {
    Point(Point),
    Arc([Point; 4]),
}
impl Curve {
    fn start(self) -> Point {
        match self {
            Self::Point(point) => point,
            Self::Arc(points) => points[0],
        }
    }
    fn end(self) -> Point {
        match self {
            Self::Point(point) => point,
            Self::Arc(points) => points[3],
        }
    }
    fn forward(self) -> Option<Segment> {
        match self {
            Self::Point(_) => None,
            Self::Arc(p) => Some(Segment::Cubic([p[1], p[2], p[3]])),
        }
    }
    fn backward(self) -> Option<Segment> {
        match self {
            Self::Point(_) => None,
            Self::Arc(p) => Some(Segment::Cubic([p[2], p[1], p[0]])),
        }
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

fn corners(rect: Rect, radii: Radii) -> [Curve; 4] {
    let points = corner_points(rect);
    std::array::from_fn(|corner| {
        let [rx, ry] = radii[corner];
        let c = points[corner];
        if rx <= 0.0 || ry <= 0.0 {
            return Curve::Point(c);
        }
        let (start, end) = match corner {
            0 => ([c[0], c[1] + ry], [c[0] + rx, c[1]]),
            1 => ([c[0] - rx, c[1]], [c[0], c[1] + ry]),
            2 => ([c[0], c[1] - ry], [c[0] - rx, c[1]]),
            _ => ([c[0] + rx, c[1]], [c[0], c[1] - ry]),
        };
        let toward = |from: Point| {
            [
                from[0] + KAPPA * (c[0] - from[0]),
                from[1] + KAPPA * (c[1] - from[1]),
            ]
        };
        Curve::Arc([start, toward(start), toward(end), end])
    })
}

/// Insets are in top/right/bottom/left order; each radius shrinks by its adjacent inset.
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
            (radii[corner][0] - shrink[corner][0]).max(0.0),
            (radii[corner][1] - shrink[corner][1]).max(0.0),
        ]
    });
    (inner, radii)
}

fn contour(curves: [Curve; 4]) -> Contour {
    let mut segments = Vec::new();
    for (corner, curve) in curves.into_iter().enumerate() {
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

pub(crate) fn rounded_rect(rect: Rect, radii: Radii) -> Contour {
    contour(corners(rect, radii))
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
        let middle = (low + high) / 2.0;
        if side(middle).signum() == low_side.signum() {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

fn offset(point: Point, direction: Point, distance: f32) -> Point {
    [
        point[0] + direction[0] * distance,
        point[1] + direction[1] * distance,
    ]
}

/// Rounded box layers derived from one border box and its used widths.
struct Geometry {
    rect: Rect,
    radii: Radii,
    widths: [f32; 4],
    outer_points: [Point; 4],
    inner_points: [Point; 4],
    outer: [Curve; 4],
    centre: [Curve; 4],
}
impl Geometry {
    fn new(rect: Rect, radii: Radii, widths: [f32; 4]) -> Self {
        let (inner_rect, _) = inset(rect, radii, widths);
        let (centre_rect, centre_radii) = inset(rect, radii, widths.map(|width| width / 2.0));
        Self {
            rect,
            radii,
            widths,
            outer_points: corner_points(rect),
            inner_points: corner_points(inner_rect),
            outer: corners(rect, radii),
            centre: corners(centre_rect, centre_radii),
        }
    }

    /// Corner curves of the border box inset by fraction `t` of each side's width.
    fn ring(&self, t: f32) -> [Curve; 4] {
        let (rect, radii) = inset(self.rect, self.radii, self.widths.map(|width| width * t));
        corners(rect, radii)
    }

    /// Region between the two corner diagonals that owns `side`.
    fn side_clip(&self, side: usize) -> Vec<Point> {
        let (a, b) = (side, (side + 1) % 4);
        let [outer_a, outer_b] = [self.outer_points[a], self.outer_points[b]];
        let [inner_a, inner_b] = [self.inner_points[a], self.inner_points[b]];
        if matches!(self.outer[a], Curve::Point(_)) && matches!(self.outer[b], Curve::Point(_)) {
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
    fn side_shape(side: usize, outer: [Curve; 4], inner: [Curve; 4]) -> Contour {
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
        let (start, start_length) = match self.centre[a] {
            Curve::Point(point) => (point, 0.0),
            Curve::Arc(points) => {
                let (origin, direction) = self.diagonal(a);
                let t = crossing(points, origin, direction);
                let (_, tail) = split(points, t);
                segments.push(Segment::Cubic([tail[1], tail[2], tail[3]]));
                (tail[0], length(points, t, 1.0))
            }
        };
        let straight_from = self.centre[a].end();
        let straight_to = self.centre[b].start();
        let straight =
            f64::from((straight_to[0] - straight_from[0]).hypot(straight_to[1] - straight_from[1]));
        let end_length = match self.centre[b] {
            Curve::Point(point) => {
                segments.push(Segment::Line(offset(point, tangent, width / 2.0)));
                0.0
            }
            Curve::Arc(points) => {
                segments.push(Segment::Line(straight_to));
                let (origin, direction) = self.diagonal(b);
                let t = crossing(points, origin, direction);
                let reached = length(points, 0.0, t);
                let end = if reached > 0.0 {
                    t * (reached + f64::from(width) / 2.0) / reached
                } else {
                    t
                };
                let (head, _) = split(points, end);
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
        let start = match self.centre[a] {
            Curve::Point(point) => {
                fitted += f64::from(half);
                offset(point, tangent, -half)
            }
            Curve::Arc(points) => {
                fitted += length(points, 0.0, 1.0) / 2.0;
                segments.extend(self.centre[a].forward());
                points[0]
            }
        };
        let straight_from = self.centre[a].end();
        let straight_to = self.centre[b].start();
        fitted +=
            f64::from((straight_to[0] - straight_from[0]).hypot(straight_to[1] - straight_from[1]));
        match self.centre[b] {
            Curve::Point(point) => {
                fitted += f64::from(half);
                segments.push(Segment::Line(offset(point, tangent, half)));
            }
            Curve::Arc(points) => {
                fitted += length(points, 0.0, 1.0) / 2.0;
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
    let geometry = Geometry::new(rect, radii, inner_widths);
    let mut fills = Vec::new();
    let color = decoration.borders[0].color;
    if let Some(boundaries) = ring(decoration, widths) {
        if color.alpha > 0.0 {
            fills.push(Fill {
                shape: Shape::Path {
                    contours: boundaries
                        .into_iter()
                        .map(|t| contour(geometry.ring(t)))
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
                    contours: vec![contour(geometry.centre)],
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
                            geometry.ring(band.from),
                            geometry.ring(band.to),
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
