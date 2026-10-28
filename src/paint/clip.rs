use crate::paint::{Contour, Point, Radii, Rect, rounded};
use serde::Serialize;

/// Lower and upper bounds on each physical axis; `None` leaves an edge open.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ClipBounds {
    limits: [[Option<f32>; 2]; 2],
}

impl ClipBounds {
    pub fn new(rect: Rect, axes: [bool; 2]) -> Self {
        let bounds = ranges(rect);
        Self {
            limits: std::array::from_fn(|axis| {
                if axes[axis] {
                    bounds[axis].map(Some)
                } else {
                    [None; 2]
                }
            }),
        }
    }

    fn intersection(self, other: Self) -> Self {
        Self {
            limits: std::array::from_fn(|axis| {
                std::array::from_fn(|edge| {
                    match (self.limits[axis][edge], other.limits[axis][edge]) {
                        (Some(a), Some(b)) => Some(if edge == 0 { a.max(b) } else { a.min(b) }),
                        (a, b) => a.or(b),
                    }
                })
            }),
        }
    }

    /// Resolves open edges only within an actual finite drawing surface.
    pub fn clip_rect(self, rect: Rect) -> Rect {
        let bounds = ranges(rect);
        let ranges: [[f32; 2]; 2] = std::array::from_fn(|axis| {
            [
                self.limits[axis][0].map_or(bounds[axis][0], |v| v.max(bounds[axis][0])),
                self.limits[axis][1].map_or(bounds[axis][1], |v| v.min(bounds[axis][1])),
            ]
        });
        Rect {
            x: ranges[0][0],
            y: ranges[1][0],
            width: (ranges[0][1] - ranges[0][0]).max(0.0),
            height: (ranges[1][1] - ranges[1][0]).max(0.0),
        }
    }

    /// Closed intervals retain zero-width ink that touches a clipped edge.
    pub fn overlaps(self, rect: Rect) -> bool {
        let bounds = ranges(rect);
        self.limits.iter().zip(bounds).all(|(limits, range)| {
            limits[0].is_none_or(|lower| range[1] >= lower)
                && limits[1].is_none_or(|upper| range[0] <= upper)
        })
    }

    pub fn edges(self) -> impl Iterator<Item = (usize, f32, bool)> {
        self.limits
            .into_iter()
            .enumerate()
            .flat_map(|(axis, range)| {
                range
                    .into_iter()
                    .enumerate()
                    .filter_map(move |(edge, bound)| bound.map(|value| (axis, value, edge == 0)))
            })
    }
}

/// A rounded rectangle or an axis-aligned strip, optionally cut at fragment edges.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Clip {
    pub rect: Rect,
    pub radii: Radii,
    pub axes: [bool; 2],
    /// Unbroken rounded rectangle, cropped to `rect` after fragmentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_box: Option<Rect>,
}

impl Clip {
    /// Only changed edges cut the outline; coincident edges retain its antialiasing.
    pub fn fragment_bounds(&self) -> Option<ClipBounds> {
        self.reference_box.and_then(|reference| {
            let rect = ranges(self.rect);
            let whole = ranges(reference);
            let limits = std::array::from_fn(|axis| {
                [
                    (rect[axis][0] > whole[axis][0]).then_some(rect[axis][0]),
                    (rect[axis][1] < whole[axis][1]).then_some(rect[axis][1]),
                ]
            });
            limits
                .iter()
                .flatten()
                .any(Option::is_some)
                .then_some(ClipBounds { limits })
        })
    }

    pub fn bounds(&self) -> ClipBounds {
        let bounds = ClipBounds::new(self.reference_box.unwrap_or(self.rect), self.axes);
        self.fragment_bounds()
            .map_or(bounds, |fragment| bounds.intersection(fragment))
    }

    /// Rounded outline before the additional rectangular fragment cut.
    pub fn contour(&self) -> Contour {
        rounded::rounded_rect(self.reference_box.unwrap_or(self.rect), self.radii)
    }

    pub fn translate(&mut self, [dx, dy]: Point) {
        for rect in std::iter::once(&mut self.rect).chain(self.reference_box.as_mut()) {
            rect.x += dx;
            rect.y += dy;
        }
    }
}

fn ranges(rect: Rect) -> [[f32; 2]; 2] {
    [
        [rect.x, rect.x + rect.width],
        [rect.y, rect.y + rect.height],
    ]
}
