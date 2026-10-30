use crate::paint::{Contour, Rect};
use i_overlay::{
    core::{fill_rule::FillRule, overlay_rule::OverlayRule},
    float::overlay::FloatOverlay,
    i_float::adapter::FloatPointAdapter,
};

type Point = [f64; 2];
type Bounds = [[f64; 2]; 2];

// Input rounding, intersection rounding, and conversion back to physical coordinates.
const GRID_ERROR_UNITS: f64 = 4.0;
const MAPPING_ERROR_ULPS: f64 = 8.0;

/// Polygon intersection with an axis-aligned rectangle, preserving winding at split parts.
pub(crate) fn clip_polygon(points: &[Point], bounds: Bounds) -> Vec<Point> {
    let mut polygon = points.to_vec();
    for (axis, limits) in bounds.into_iter().enumerate() {
        for (edge, limit) in limits.into_iter().enumerate() {
            let keep = |point: Point| {
                if edge == 0 {
                    point[axis] >= limit
                } else {
                    point[axis] <= limit
                }
            };
            let input = std::mem::take(&mut polygon);
            for (index, &current) in input.iter().enumerate() {
                let previous = input[(index + input.len() - 1) % input.len()];
                if keep(current) != keep(previous) {
                    let [a, b] = if (limit - previous[axis]).abs() < (limit - current[axis]).abs() {
                        [previous, current]
                    } else {
                        [current, previous]
                    };
                    let t = (limit - a[axis]) / (b[axis] - a[axis]);
                    let mut crossing = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                    crossing[axis] = limit;
                    polygon.push(crossing);
                }
                if keep(current) {
                    polygon.push(current);
                }
            }
        }
    }
    polygon
}

fn rectangle([[left, right], [top, bottom]]: Bounds) -> Vec<Point> {
    vec![[left, top], [right, top], [right, bottom], [left, bottom]]
}

fn overlaps(polygon: &[Point], bounds: Bounds) -> bool {
    (0..2).all(|axis| {
        polygon.iter().any(|point| point[axis] > bounds[axis][0])
            && polygon.iter().any(|point| point[axis] < bounds[axis][1])
    })
}

fn subtract(subject: Vec<Point>, cutouts: &[Vec<Point>]) -> Vec<Vec<Point>> {
    let mut shapes = vec![vec![subject]];
    for cutout in cutouts {
        // Sequential subtraction unions cutouts regardless of their winding or overlap.
        shapes = FloatOverlay::<Point, i64>::from_subj_and_clip(&shapes, cutout)
            .overlay(OverlayRule::Difference, FillRule::NonZero);
        if shapes.is_empty() {
            break;
        }
    }
    shapes.into_iter().flatten().collect()
}

fn axis_tolerance([lower, upper]: [f64; 2], tolerance: f64) -> f64 {
    let nearest = if lower > 0.0 {
        lower
    } else if upper < 0.0 {
        -upper
    } else {
        0.0
    };
    tolerance.max(2.0 * f64::from(f32::EPSILON) * nearest)
}

fn mapping_error([lower, upper]: [f64; 2], grid: f64) -> f64 {
    GRID_ERROR_UNITS * grid
        + MAPPING_ERROR_ULPS * f64::EPSILON * (upper - lower + lower.abs().max(upper.abs()))
}

fn contours(polygons: Vec<Vec<Point>>, bounds: Bounds) -> Vec<Contour> {
    polygons
        .into_iter()
        .filter(|polygon| polygon.len() >= 3)
        .map(|polygon| {
            let points: Vec<_> = polygon
                .into_iter()
                .map(|point| {
                    std::array::from_fn(|axis| {
                        point[axis].clamp(bounds[axis][0], bounds[axis][1]) as f32
                    })
                })
                .collect();
            Contour::polygon(&points)
        })
        .collect()
}

/// Subtract corner cutouts without allowing a global integer grid to erase local detail.
pub(super) fn subtract_from_rect(rect: Rect, cutouts: &[Contour], tolerance: f64) -> Vec<Contour> {
    let bounds = [
        [f64::from(rect.x), f64::from(rect.x + rect.width)],
        [f64::from(rect.y), f64::from(rect.y + rect.height)],
    ];
    if bounds.iter().any(|axis| axis[0] >= axis[1]) {
        return Vec::new();
    }
    let cutouts: Vec<_> = cutouts
        .iter()
        .map(|cutout| clip_polygon(&cutout.polygon_points(tolerance), bounds))
        .filter(|polygon| polygon.len() >= 3 && overlaps(polygon, bounds))
        .collect();
    let subject = rectangle(bounds);
    let adapter = FloatPointAdapter::<Point, i64>::with_iter_conservative(
        subject.iter().chain(cutouts.iter().flatten()),
    );
    let precise = bounds
        .iter()
        .all(|&axis| mapping_error(axis, adapter.inv_scale()) <= axis_tolerance(axis, tolerance))
        && subject.iter().chain(cutouts.iter().flatten()).all(|point| {
            let restored = adapter.int_to_float(&adapter.float_to_int(point));
            (0..2).all(|axis| point[axis] as f32 == restored[axis] as f32)
        });
    if precise {
        return contours(subtract(subject, &cutouts), bounds);
    }

    let mut result = Vec::new();
    let mut pending = vec![bounds];
    while let Some(cell) = pending.pop() {
        let clipped: Vec<_> = cutouts
            .iter()
            .filter(|polygon| overlaps(polygon, cell))
            .map(|polygon| clip_polygon(polygon, cell))
            .filter(|polygon| polygon.len() >= 3 && overlaps(polygon, cell))
            .collect();
        if clipped.is_empty() {
            result.extend(contours(vec![rectangle(cell)], cell));
            continue;
        }
        let normalize = |point: Point| {
            std::array::from_fn(|axis| {
                ((point[axis] - cell[axis][0]) / (cell[axis][1] - cell[axis][0])).clamp(0.0, 1.0)
            })
        };
        let restore = |point: Point| {
            std::array::from_fn(|axis| {
                if point[axis] <= 0.0 {
                    cell[axis][0]
                } else if point[axis] >= 1.0 {
                    cell[axis][1]
                } else {
                    cell[axis][0] + point[axis] * (cell[axis][1] - cell[axis][0])
                }
            })
        };
        let unit = rectangle([[0.0, 1.0]; 2]);
        let adapter = FloatPointAdapter::<Point, i64>::with_iter_conservative(unit.iter());
        let coarse_axis = (0..2).find(|&axis| {
            mapping_error(
                cell[axis],
                adapter.inv_scale() * (cell[axis][1] - cell[axis][0]),
            ) > axis_tolerance(cell[axis], tolerance)
        });
        let split = coarse_axis
            .map(|axis| {
                let middle = f64::midpoint(cell[axis][0], cell[axis][1]);
                let value = clipped
                    .iter()
                    .flatten()
                    .map(|point| point[axis])
                    .filter(|value| *value > cell[axis][0] && *value < cell[axis][1])
                    .min_by(|a, b| (a - middle).abs().total_cmp(&(b - middle).abs()))
                    .unwrap_or(middle);
                (axis, value)
            })
            .or_else(|| {
                clipped.iter().flatten().find_map(|&point| {
                    let mapped = normalize(point);
                    let restored: Point =
                        restore(adapter.int_to_float(&adapter.float_to_int(&mapped)));
                    (0..2)
                        .find(|&axis| {
                            point[axis] > cell[axis][0]
                                && point[axis] < cell[axis][1]
                                && point[axis] as f32 != restored[axis] as f32
                        })
                        .map(|axis| (axis, point[axis]))
                })
            });
        if let Some((axis, value)) = split {
            let mut lower = cell;
            let mut upper = cell;
            lower[axis][1] = value;
            upper[axis][0] = value;
            pending.extend([upper, lower]);
            continue;
        }
        let normalized: Vec<Vec<_>> = clipped
            .iter()
            .map(|polygon| polygon.iter().copied().map(normalize).collect())
            .collect();
        let polygons = subtract(unit, &normalized)
            .into_iter()
            .map(|polygon| polygon.into_iter().map(restore).collect())
            .collect();
        result.extend(contours(polygons, cell));
    }
    result
}
