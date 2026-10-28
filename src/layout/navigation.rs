//! Page-space activation areas of hyperlinks and positions of destinations, from box
//! fragments and the clips and transforms enclosing them.
use crate::graphics::{ClipBounds, Point, Rect};
use crate::layout::pagination::Region;
use serde::Serialize;

/// Coordinates closer than this coincide when classifying and comparing areas.
const GEOMETRY_EPSILON_PT: f32 = 0.01;
const FIRST_PAGE_NUMBER: usize = 1;

/// Where a hyperlink activates on a page.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum LinkShape {
    Rect(Rect),
    /// Quadrilaterals from a box's top-left corner in the order top-left, bottom-left,
    /// bottom-right, top-right, which is counterclockwise once y points up.
    Quads(Vec<[Point; 4]>),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LinkArea {
    /// Index into the document's hyperlinks.
    pub link: usize,
    pub shape: LinkShape,
}

/// The first fragment of a destination's box.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Destination {
    pub page: usize,
    /// Top of the fragment's transformed border box, as scrolling it into view aligns it
    /// (CSSOM View 1 §7.1).
    pub top: f32,
}

/// A located HTML heading, independent of diagnostic trace retention and PDF bookmarks.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HeadingLocation {
    /// Nonempty HTML id, when present.
    pub id: Option<String>,
    /// Computed HTML heading level, including headingoffset and headingreset.
    pub level: u8,
    /// Formatting-tree descendant text with CSS whitespace collapsed; excludes pseudo content and bookmark labels.
    pub text: String,
    /// One-based PDF page number.
    pub page: usize,
    /// Top of the first fragment, in points from the top of its page.
    pub top: f32,
}

impl crate::layout::LayoutTrace {
    pub fn heading_locations(&self, headings: &[crate::document::Heading]) -> Vec<HeadingLocation> {
        headings
            .iter()
            .filter_map(|heading| {
                let destination = self
                    .destinations
                    .get(heading.destination)
                    .copied()
                    .flatten()?;
                Some(HeadingLocation {
                    id: heading.id.clone(),
                    level: heading.level,
                    text: heading.text.clone(),
                    page: destination.page + FIRST_PAGE_NUMBER,
                    top: destination.top,
                })
            })
            .collect()
    }
}

pub(crate) fn clip_links(areas: &mut Vec<LinkArea>, bounds: Rect) {
    if bounds.width <= 0.0 || bounds.height <= 0.0 {
        areas.clear();
        return;
    }
    areas.retain_mut(|area| match &mut area.shape {
        LinkShape::Rect(rect) => {
            *rect = rect.intersection(bounds);
            rect.width > 0.0 && rect.height > 0.0
        }
        LinkShape::Quads(quads) => {
            if quads.iter().flatten().all(|point| {
                point[0] >= bounds.x
                    && point[0] <= bounds.x + bounds.width
                    && point[1] >= bounds.y
                    && point[1] <= bounds.y + bounds.height
            }) {
                return true;
            }
            let mut clipped = Vec::new();
            for quad in quads.iter() {
                let polygon = clip_polygon(quad, ClipBounds::new(bounds, [true; 2]));
                if polygon.len() < 3 || signed_area(&polygon) == 0.0 {
                    continue;
                }
                match shape(&polygon) {
                    LinkShape::Rect(rect) => clipped.push(rect_corners(rect)),
                    LinkShape::Quads(quads) => clipped.extend(quads),
                }
            }
            *quads = clipped;
            !quads.is_empty()
        }
    });
}

fn rect_corners(rect: Rect) -> [Point; 4] {
    [
        [rect.x, rect.y],
        [rect.x, rect.y + rect.height],
        [rect.x + rect.width, rect.y + rect.height],
        [rect.x + rect.width, rect.y],
    ]
}

/// `rect` in page space: mapped out through `regions`, outermost first, and clipped by their
/// clips when `clip` is set. Each clip is axis-aligned in its own space, so the area stays a
/// convex polygon.
pub(crate) fn page_polygon(rect: Rect, regions: &[Region], clip: bool) -> Vec<Point> {
    let mut polygon = rect_corners(rect).to_vec();
    for region in regions.iter().rev() {
        match region {
            Region::Transform(matrix) => {
                for point in &mut polygon {
                    *point = apply(*matrix, *point);
                }
            }
            Region::Clip(region) if clip => polygon = clip_polygon(&polygon, region.bounds()),
            Region::Clip(_) => {}
        }
    }
    polygon
}

fn apply([a, b, c, d, e, f]: [f32; 6], [x, y]: Point) -> Point {
    [a * x + c * y + e, b * x + d * y + f]
}

/// Sutherland–Hodgman clipping of a convex polygon to an axis-aligned rectangle.
fn clip_polygon(polygon: &[Point], bounds: ClipBounds) -> Vec<Point> {
    let mut output = polygon.to_vec();
    for (axis, bound, lower) in bounds.edges() {
        let inside = |point: &Point| {
            if lower {
                point[axis] >= bound
            } else {
                point[axis] <= bound
            }
        };
        let input = std::mem::take(&mut output);
        for (index, current) in input.iter().enumerate() {
            let previous = &input[(index + input.len() - 1) % input.len()];
            if inside(current) != inside(previous) {
                let t = (bound - previous[axis]) / (current[axis] - previous[axis]);
                push_distinct(
                    &mut output,
                    [
                        previous[0] + t * (current[0] - previous[0]),
                        previous[1] + t * (current[1] - previous[1]),
                    ],
                );
            }
            if inside(current) {
                push_distinct(&mut output, *current);
            }
        }
        if output.len() > 1 && same_point(output[0], output[output.len() - 1]) {
            output.pop();
        }
    }
    output
}

/// Edges crossing a clip bound at a vertex yield that vertex twice.
fn push_distinct(polygon: &mut Vec<Point>, point: Point) {
    if polygon.last().is_none_or(|last| !same_point(*last, point)) {
        polygon.push(point);
    }
}

fn same_point(a: Point, b: Point) -> bool {
    (a[0] - b[0]).abs() <= GEOMETRY_EPSILON_PT && (a[1] - b[1]).abs() <= GEOMETRY_EPSILON_PT
}

/// Twice the signed area; negative for the top-left, bottom-left, bottom-right, top-right
/// order with y down.
fn signed_area(polygon: &[Point]) -> f32 {
    (0..polygon.len())
        .map(|index| {
            let [x0, y0] = polygon[index];
            let [x1, y1] = polygon[(index + 1) % polygon.len()];
            x0 * y1 - x1 * y0
        })
        .sum()
}

/// Whether convex `outer` covers every vertex of `inner`.
fn contains(outer: &[Point], inner: &[Point]) -> bool {
    let orientation = signed_area(outer).signum();
    if orientation == 0.0 {
        return outer.len() == inner.len()
            && outer.iter().zip(inner).all(|(a, b)| same_point(*a, *b));
    }
    inner.iter().all(|point| {
        (0..outer.len()).all(|index| {
            let [x0, y0] = outer[index];
            let [x1, y1] = outer[(index + 1) % outer.len()];
            let cross = (x1 - x0) * (point[1] - y0) - (y1 - y0) * (point[0] - x0);
            let length = (x1 - x0).hypot(y1 - y0).max(f32::MIN_POSITIVE);
            cross * orientation >= -GEOMETRY_EPSILON_PT * length
        })
    })
}

/// A page's link areas: polygons covered by another area of the same link are dropped, and
/// the rest become rectangles when axis-aligned, otherwise quadrilaterals.
pub(crate) fn link_areas(polygons: &[(usize, Vec<Point>)]) -> Vec<LinkArea> {
    let kept: Vec<&(usize, Vec<Point>)> = polygons
        .iter()
        .enumerate()
        .filter(|(index, (link, polygon))| {
            !polygons
                .iter()
                .enumerate()
                .any(|(other, (other_link, outer))| {
                    other != *index
                    && other_link == link
                    && contains(outer, polygon)
                    // Of two equal areas the first is kept.
                    && !(other > *index && contains(polygon, outer))
                })
        })
        .map(|(_, area)| area)
        .collect();
    kept.into_iter()
        .map(|(link, polygon)| LinkArea {
            link: *link,
            shape: shape(polygon),
        })
        .collect()
}

fn shape(polygon: &[Point]) -> LinkShape {
    // Empty inline boxes and clipped edges leave zero-width areas with fewer corners.
    let axis_aligned = polygon.len() <= 4
        && (0..polygon.len()).all(|index| {
            let [x0, y0] = polygon[index];
            let [x1, y1] = polygon[(index + 1) % polygon.len()];
            (x1 - x0).abs() <= GEOMETRY_EPSILON_PT || (y1 - y0).abs() <= GEOMETRY_EPSILON_PT
        });
    if axis_aligned {
        let (left, right) = bounds(polygon.iter().map(|point| point[0]));
        let (top, bottom) = bounds(polygon.iter().map(|point| point[1]));
        return LinkShape::Rect(Rect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        });
    }
    let mut ordered = polygon.to_vec();
    if signed_area(&ordered) > 0.0 {
        ordered[1..].reverse();
    }
    // Fans of up to four corners from the first; a short final fan repeats its last corner.
    let last = ordered.len() - 1;
    let quads = (1..last.max(2))
        .step_by(2)
        .map(|index| {
            let corner = |offset: usize| ordered[(index + offset).min(last)];
            [ordered[0], corner(0), corner(1), corner(2)]
        })
        .collect();
    LinkShape::Quads(quads)
}

fn bounds(values: impl Iterator<Item = f32>) -> (f32, f32) {
    values.fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), value| {
        (low.min(value), high.max(value))
    })
}

/// The destination a box fragment's border box gives on `page`.
pub(crate) fn destination(page: usize, rect: Rect, regions: &[Region]) -> Destination {
    let polygon = page_polygon(rect, regions, false);
    Destination {
        page,
        top: bounds(polygon.iter().map(|point| point[1])).0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::{Clip, Radii};

    const RECT: Rect = Rect {
        x: 10.0,
        y: 20.0,
        width: 40.0,
        height: 10.0,
    };

    fn clip(rect: Rect) -> Region {
        Region::Clip(Clip {
            rect,
            radii: Radii::default(),
            reference_box: None,
            axes: [true; 2],
        })
    }

    #[test]
    fn page_bleed_clips_rotated_links_without_filling_their_cut_corners() {
        const VISIBLE_AREA_PT2: f32 = 92.0;
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let mut areas = vec![
            LinkArea {
                link: 0,
                shape: LinkShape::Quads(vec![[[-3.0, 5.0], [5.0, 13.0], [13.0, 5.0], [5.0, -3.0]]]),
            },
            LinkArea {
                link: 1,
                shape: LinkShape::Rect(Rect {
                    x: 20.0,
                    y: 20.0,
                    width: 5.0,
                    height: 5.0,
                }),
            },
        ];
        clip_links(&mut areas, bounds);
        assert_eq!(
            areas.len(),
            1,
            "a fully trimmed link cannot activate: {areas:?}"
        );
        let LinkShape::Quads(quads) = &areas[0].shape else {
            panic!("the cut diamond remains a polygon")
        };
        assert!(
            quads
                .iter()
                .flatten()
                .all(|point| (0.0..=10.0).contains(&point[0]) && (0.0..=10.0).contains(&point[1])),
            "every clickable vertex stays inside bleed: {quads:?}"
        );
        let area: f32 = quads.iter().map(|quad| signed_area(quad).abs() / 2.0).sum();
        assert_eq!(
            area, VISIBLE_AREA_PT2,
            "the clipped octagon omits four 2×2 triangular corners: {quads:?}"
        );
    }

    #[test]
    fn clipped_untransformed_area_stays_a_rectangle() {
        let polygon = page_polygon(
            RECT,
            &[clip(Rect {
                x: 0.0,
                y: 0.0,
                width: 30.0,
                height: 100.0,
            })],
            true,
        );
        assert_eq!(
            link_areas(&[(0, polygon)]),
            vec![LinkArea {
                link: 0,
                shape: LinkShape::Rect(Rect {
                    width: 20.0,
                    ..RECT
                })
            }]
        );
    }

    #[test]
    fn rotated_area_is_one_counterclockwise_quadrilateral() {
        let (sin, cos) = std::f32::consts::FRAC_PI_6.sin_cos();
        let polygon = page_polygon(
            RECT,
            &[Region::Transform([cos, sin, -sin, cos, 0.0, 0.0])],
            true,
        );
        let LinkShape::Quads(quads) = &link_areas(&[(3, polygon)])[0].shape else {
            panic!("a rotated rectangle needs quadrilaterals");
        };
        assert_eq!(quads.len(), 1, "{quads:?}");
        let expected = [[10.0, 20.0], [10.0, 30.0], [50.0, 30.0], [50.0, 20.0]]
            .map(|point| apply([cos, sin, -sin, cos, 0.0, 0.0], point));
        for (actual, expected) in quads[0].iter().zip(expected) {
            assert!(
                (actual[0] - expected[0]).abs() < 1e-4 && (actual[1] - expected[1]).abs() < 1e-4,
                "{quads:?} against {expected:?}"
            );
        }
    }

    #[test]
    fn rotated_area_clipped_outside_its_transform_becomes_quadrilaterals_covering_the_overlap() {
        let polygon = page_polygon(
            Rect {
                x: -10.0,
                y: -10.0,
                width: 20.0,
                height: 20.0,
            },
            &[
                clip(Rect {
                    x: 0.0,
                    y: -20.0,
                    width: 40.0,
                    height: 40.0,
                }),
                Region::Transform([
                    std::f32::consts::FRAC_1_SQRT_2,
                    std::f32::consts::FRAC_1_SQRT_2,
                    -std::f32::consts::FRAC_1_SQRT_2,
                    std::f32::consts::FRAC_1_SQRT_2,
                    0.0,
                    0.0,
                ]),
            ],
            true,
        );
        assert_eq!(polygon.len(), 3, "half of a diamond: {polygon:?}");
        let LinkShape::Quads(quads) = &link_areas(&[(0, polygon.clone())])[0].shape else {
            panic!("the clipped diamond is not axis-aligned");
        };
        assert!(
            quads.iter().flatten().all(|point| point[0] >= -1e-4),
            "every corner lies inside the clip: {quads:?}"
        );
        assert!(
            signed_area(&quads[0]) < 0.0,
            "quadrilaterals keep the top-left, bottom-left order: {quads:?}"
        );
    }

    #[test]
    fn hexagonal_overlap_splits_into_two_quadrilaterals() {
        let hexagon = vec![
            [0.0, 0.0],
            [-1.0, 1.0],
            [0.0, 2.0],
            [2.0, 2.0],
            [3.0, 1.0],
            [2.0, 0.0],
        ];
        let LinkShape::Quads(quads) = &link_areas(&[(0, hexagon.clone())])[0].shape else {
            panic!("a hexagon is not a rectangle");
        };
        assert_eq!(
            quads,
            &vec![
                [hexagon[0], hexagon[1], hexagon[2], hexagon[3]],
                [hexagon[0], hexagon[3], hexagon[4], hexagon[5]],
            ]
        );
    }

    #[test]
    fn areas_inside_another_area_of_the_same_link_are_dropped() {
        let outer = page_polygon(RECT, &[], true);
        let inner = page_polygon(
            Rect {
                x: 20.0,
                width: 5.0,
                ..RECT
            },
            &[],
            true,
        );
        let taller = page_polygon(
            Rect {
                y: 15.0,
                height: 20.0,
                ..RECT
            },
            &[],
            true,
        );
        let areas = link_areas(&[
            (0, outer.clone()),
            (0, inner.clone()),
            (1, inner),
            (0, taller),
            (0, outer),
        ]);
        let summary: Vec<_> = areas
            .iter()
            .map(|area| match area.shape {
                LinkShape::Rect(rect) => (area.link, rect.y, rect.height),
                LinkShape::Quads(_) => panic!("untransformed areas are rectangles"),
            })
            .collect();
        assert_eq!(
            summary,
            vec![(1, 20.0, 10.0), (0, 15.0, 20.0)],
            "the inner and duplicate areas of link 0 fall inside its taller area; link 1 is separate"
        );
    }

    #[test]
    fn destination_is_the_top_of_the_transformed_box() {
        let turned = destination(
            2,
            RECT,
            &[Region::Transform([0.0, 1.0, -1.0, 0.0, 0.0, 0.0])],
        );
        assert_eq!(turned, Destination { page: 2, top: 10.0 });
        assert_eq!(destination(0, RECT, &[]).top, RECT.y);
    }
}
