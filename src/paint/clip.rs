use crate::paint::{Contour, Point, Radii, Rect, rounded};
use serde::Serialize;
use std::{borrow::Cow, sync::Arc};

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

/// A box outline or an axis-aligned strip, optionally cut at fragment edges.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Clip {
    pub rect: Rect,
    pub radii: Radii,
    /// Nonzero-winding unbroken outline; an empty collection clips everything on both axes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<Arc<Vec<Contour>>>,
    pub axes: [bool; 2],
    /// Unbroken box bounds, cropped to `rect` after fragmentation.
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

    /// Outline before the additional rectangular fragment cut.
    pub fn contours(&self) -> Cow<'_, [Contour]> {
        self.outline.as_ref().map_or_else(
            || {
                Cow::Owned(vec![rounded::rounded_rect(
                    self.reference_box.unwrap_or(self.rect),
                    self.radii,
                )])
            },
            |outline| Cow::Borrowed(outline.as_slice()),
        )
    }

    pub fn translate(&mut self, [dx, dy]: Point) {
        if let Some(outline) = &mut self.outline {
            for contour in Arc::make_mut(outline) {
                *contour = contour.map(|[x, y]| [x + dx, y + dy]);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_outline_survives_fragmentation_and_copy_on_write_translation() {
        let islands = vec![
            Contour::polygon(&[[40.0, 0.0], [80.0, 0.0], [80.0, 30.0], [40.0, 30.0]]),
            Contour::polygon(&[[0.0, 30.0], [40.0, 30.0], [40.0, 60.0], [0.0, 60.0]]),
        ];
        let source = Clip {
            rect: Rect {
                x: 0.0,
                y: 10.0,
                width: 80.0,
                height: 40.0,
            },
            radii: Radii::default(),
            outline: Some(Arc::new(islands.clone())),
            axes: [true; 2],
            reference_box: Some(Rect {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 60.0,
            }),
        };
        assert_eq!(
            source.contours().as_ref(),
            islands,
            "fragment clips retain both unbroken islands"
        );
        let mut translated = source.clone();
        translated.translate([7.0, -3.0]);
        let expected: Vec<_> = islands
            .iter()
            .map(|contour| contour.map(|[x, y]| [x + 7.0, y - 3.0]))
            .collect();
        assert_eq!(
            translated.contours().as_ref(),
            expected,
            "every segment follows the translated reference box"
        );
        assert_eq!(
            source.contours().as_ref(),
            islands,
            "translating a shared outline must not alter its other fragments"
        );
        let clipped = translated.bounds().clip_rect(Rect {
            x: -100.0,
            y: -100.0,
            width: 300.0,
            height: 300.0,
        });
        assert_eq!(
            clipped,
            Rect {
                x: 7.0,
                y: 7.0,
                width: 80.0,
                height: 40.0
            }
        );
    }

    #[test]
    fn background_fill_uses_the_same_nonzero_outline_as_its_clip() {
        use crate::paint::{BackgroundGeometry, BoxAreas, BoxDecoration, BoxPaint, Shape};
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 80.0,
            height: 60.0,
        };
        let outline = vec![
            Contour::polygon(&[[0.0, 0.0], [60.0, 0.0], [60.0, 60.0], [0.0, 60.0]]),
            Contour::polygon(&[[20.0, 0.0], [80.0, 0.0], [80.0, 60.0], [20.0, 60.0]]),
        ];
        let mut paint: BoxPaint = BoxPaint::whole(rect, BoxDecoration::default());
        paint.decoration.background.alpha = 1.0;
        for contours in [outline, Vec::new()] {
            let clip = Clip {
                rect,
                radii: Radii::default(),
                outline: Some(Arc::new(contours.clone())),
                reference_box: None,
                axes: [true; 2],
            };
            paint.background_geometry = Some(Arc::new(BackgroundGeometry {
                origins: None,
                viewport: None,
                clips: BoxAreas {
                    border: clip.clone(),
                    padding: clip.clone(),
                    content: clip,
                },
            }));
            if contours.is_empty() {
                assert!(
                    paint.background_fill().is_none(),
                    "an empty clip must produce no background fill"
                );
            } else {
                let fill = paint.background_fill().expect("visible background");
                assert!(
                    matches!(fill.shape, Shape::Path { contours: actual, even_odd: false, clip: None } if actual == contours),
                    "overlapping contours must have the same nonzero union as the PDF clip"
                );
            }
        }
    }

    #[test]
    fn an_explicit_empty_outline_does_not_fall_back_to_the_box() {
        let clip = Clip {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 60.0,
            },
            radii: Radii::default(),
            outline: Some(Arc::new(Vec::new())),
            axes: [true; 2],
            reference_box: None,
        };
        assert!(
            clip.contours().is_empty(),
            "consumed inner areas have no painted or clipped region"
        );
    }
}
