//! CSS Shapes 1 §3 `<basic-shape>` computed values and §6.1 `shape-outside`, with the outlines
//! basic shapes trace in their reference box.
use crate::paint::{
    BackgroundImage, BoxArea, BoxPaint, Contour, LengthPercentage, POINTS_PER_PIXEL, Position,
    RadialExtent, Radii, Rect, Shape, rounded,
};
use serde::Serialize;
use std::convert::Infallible;

/// `<shape-box>`: a `<visual-box>` or `margin-box`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum ShapeBox {
    Margin,
    Visual(BoxArea),
}

/// `<geometry-box>` (CSS Masking 1 §6.1): a `<shape-box>`, or a box of SVG content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum GeometryBox {
    Shape(ShapeBox),
    Fill,
    Stroke,
    View,
}

impl GeometryBox {
    /// The box of an element with a CSS layout box: the content box for `fill-box` and the
    /// border box for `stroke-box` and `view-box`.
    pub(crate) fn layout(self) -> ShapeBox {
        match self {
            Self::Shape(reference) => reference,
            Self::Fill => ShapeBox::Visual(BoxArea::ContentBox),
            Self::Stroke | Self::View => ShapeBox::Visual(BoxArea::BorderBox),
        }
    }
}

/// A `circle()` radius: an extent keyword, or a length whose percentages refer to the
/// reference box's direction-agnostic size (CSS Images 4 §3.2.2).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum CircleRadius {
    Extent(RadialExtent),
    Length(LengthPercentage),
}

/// One `ellipse()` radius along its axis.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum AxisRadius {
    ClosestSide,
    FarthestSide,
    Length(LengthPercentage),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum EllipseRadii {
    /// One extent for both radii, as for an elliptical radial gradient.
    Extent(RadialExtent),
    /// Horizontal then vertical.
    Axes([AxisRadius; 2]),
}

/// Corner radii as `[horizontal, vertical]`, top-left first.
pub(crate) type CornerRadii = [[LengthPercentage; 2]; 4];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum BasicShape {
    /// Insets from the top, right, bottom and left edges; `xywh()` computes to it.
    Inset {
        insets: [LengthPercentage; 4],
        radii: CornerRadii,
    },
    /// `rect()`: top, right, bottom and left edges from the top-left corner, `auto` resolved.
    Rect {
        edges: [LengthPercentage; 4],
        radii: CornerRadii,
    },
    Circle {
        radius: CircleRadius,
        center: Position,
    },
    Ellipse {
        radii: EllipseRadii,
        center: Position,
    },
    Polygon {
        even_odd: bool,
        points: Vec<[LengthPercentage; 2]>,
    },
    /// SVG path data in CSS pixels from the reference box origin, one contour per subpath.
    Path {
        even_odd: bool,
        contours: Vec<Contour>,
    },
}

impl ShapeBox {
    /// The assembled box contour, including its corner shape and signed margin offsets.
    pub(crate) fn contours<I>(self, paint: &BoxPaint<I>, margins: [f32; 4]) -> Vec<Contour> {
        if *paint.decoration.corner_shapes == [crate::paint::CornerShape::ROUND; 4] {
            let (rect, radii) = self.resolve(paint, margins);
            return vec![rounded::rounded_rect(rect, radii)];
        }
        let clip = match self {
            Self::Margin => paint.box_clip(margins.map(|margin| -margin), [true; 2]),
            Self::Visual(BoxArea::BorderBox) => paint.box_clip([0.0; 4], [true; 2]),
            Self::Visual(area) => paint.inner_clip(
                if area == BoxArea::ContentBox {
                    paint.padding
                } else {
                    [0.0; 4]
                },
                [true; 2],
            ),
        };
        clip.contours().into_owned()
    }

    /// The box's rectangle and corner radii in `paint`, a sliced fragment's assembled box,
    /// with `margins` around the border box for `margin-box` (CSS Shapes 1 §5).
    pub(crate) fn resolve<I>(self, paint: &BoxPaint<I>, margins: [f32; 4]) -> (Rect, Radii) {
        match self {
            Self::Margin => (
                crate::paint::outset(paint.decoration_rect(), margins),
                crate::paint::outset_radii(paint.radii(), margins),
            ),
            Self::Visual(BoxArea::BorderBox) => (paint.decoration_rect(), paint.radii()),
            Self::Visual(area) => {
                let padding = if area == BoxArea::ContentBox {
                    paint.padding
                } else {
                    [0.0; 4]
                };
                let clip = paint.inner_clip(padding, [true; 2]);
                (clip.reference_box.unwrap_or(clip.rect), clip.radii)
            }
        }
    }
}

/// CSS Masking 1 §6.1 `clip-path`: a basic shape in its reference box, `border-box` unless
/// specified, or the reference box alone with its corner curvature.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ClipPath {
    pub shape: Option<BasicShape>,
    pub reference: GeometryBox,
}

impl ClipPath {
    /// The clipping region of a box fragment, whose used margins `paint` carries.
    pub(crate) fn outline<I>(&self, paint: &BoxPaint<I>) -> Shape {
        let reference = self.reference.layout();
        match &self.shape {
            Some(shape) => shape.outline(reference.resolve(paint, paint.margins).0),
            None => Shape::Path {
                contours: reference.contours(paint, paint.margins),
                even_odd: false,
                clip: None,
            },
        }
    }
}

/// The `shape-outside` computed value; `I` is a loaded image.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(bound(serialize = ""))]
pub(crate) enum ShapeOutside<I = Infallible> {
    #[default]
    None,
    Box(ShapeBox),
    /// A basic shape in its reference box, `margin-box` unless specified.
    Shape {
        shape: BasicShape,
        reference: ShapeBox,
    },
    Image(BackgroundImage<I>),
}

impl ShapeOutside<Infallible> {
    /// An image that cannot be loaded and has no fallback color leaves `none` (§6.1).
    pub(crate) fn load_image<I, E>(
        &self,
        load: &mut impl FnMut(&crate::paint::ImageRequest<'_>) -> Result<Option<I>, E>,
    ) -> Result<ShapeOutside<I>, E> {
        Ok(match self {
            Self::None => ShapeOutside::None,
            Self::Box(reference) => ShapeOutside::Box(*reference),
            Self::Shape { shape, reference } => ShapeOutside::Shape {
                shape: shape.clone(),
                reference: *reference,
            },
            Self::Image(image) => {
                let mut loaded = image.with_image_type();
                loaded.load(load)?;
                if matches!(image, BackgroundImage::Url { .. })
                    && matches!(loaded, BackgroundImage::Invalid)
                {
                    ShapeOutside::None
                } else {
                    ShapeOutside::Image(loaded)
                }
            }
        })
    }
}

/// The length of a box's diagonal divided by √2 (CSS Shapes 1 §1.3).
fn direction_agnostic([width, height]: [f32; 2]) -> f32 {
    width.hypot(height) / std::f32::consts::SQRT_2
}

/// CSS Backgrounds 3 §4.5: radii resolved against `basis`, scaled down together until
/// adjacent radii fit along every side of `size`.
fn used_radii(radii: &CornerRadii, basis: [f32; 2], size: [f32; 2]) -> Radii {
    let resolved = std::array::from_fn(|corner| {
        let [horizontal, vertical] = &radii[corner];
        let used = [horizontal.resolve(basis[0]), vertical.resolve(basis[1])];
        if used[0] > 0.0 && used[1] > 0.0 {
            used
        } else {
            [0.0; 2]
        }
    });
    rounded::constrain(resolved, size)
}

/// A pair of opposite insets reduced proportionally when they exceed `extent` (§3.1).
fn fitted([start, end]: [f32; 2], extent: f32) -> [f32; 2] {
    let sum = start + end;
    if sum > extent && sum > 0.0 {
        let scale = extent / sum;
        [start * scale, end * scale]
    } else {
        [start, end]
    }
}

fn ellipse(center: [f32; 2], [rx, ry]: [f32; 2]) -> Contour {
    rounded::rounded_rect(
        Rect {
            x: center[0] - rx,
            y: center[1] - ry,
            width: 2.0 * rx,
            height: 2.0 * ry,
        },
        [[rx, ry]; 4],
    )
}

impl AxisRadius {
    fn resolve(&self, center: f32, extent: f32) -> f32 {
        let sides = [center.abs(), (extent - center).abs()];
        match self {
            Self::ClosestSide => sides[0].min(sides[1]),
            Self::FarthestSide => sides[0].max(sides[1]),
            Self::Length(length) => length.resolve(extent),
        }
    }
}

impl BasicShape {
    /// The closed outline the shape traces in `reference` and whether it fills even-odd.
    pub(crate) fn outline(&self, reference: Rect) -> Shape {
        let (mut contours, even_odd) = self.path(reference);
        for contour in &mut contours {
            contour.closed = true;
        }
        Shape::Path {
            contours,
            even_odd,
            clip: None,
        }
    }
    /// Equivalent path, retaining explicit SVG subpath closure.
    pub(crate) fn path(&self, reference: Rect) -> (Vec<Contour>, bool) {
        let size = [reference.width, reference.height];
        let origin = [reference.x, reference.y];
        let (contours, even_odd) = match self {
            Self::Inset { insets, radii } => {
                let [top, bottom] = fitted(
                    [insets[0].resolve(size[1]), insets[2].resolve(size[1])],
                    size[1],
                );
                let [left, right] = fitted(
                    [insets[3].resolve(size[0]), insets[1].resolve(size[0])],
                    size[0],
                );
                let rect = Rect {
                    x: origin[0] + left,
                    y: origin[1] + top,
                    width: (size[0] - left - right).max(0.0),
                    height: (size[1] - top - bottom).max(0.0),
                };
                (vec![rounded_inset(rect, radii, size)], false)
            }
            Self::Rect { edges, radii } => {
                let top = edges[0].resolve(size[1]);
                let left = edges[3].resolve(size[0]);
                let right = edges[1].resolve(size[0]).max(left);
                let bottom = edges[2].resolve(size[1]).max(top);
                let rect = Rect {
                    x: origin[0] + left,
                    y: origin[1] + top,
                    width: right - left,
                    height: bottom - top,
                };
                (vec![rounded_inset(rect, radii, size)], false)
            }
            Self::Circle { radius, center } => {
                let center = [center[0].resolve(size[0]), center[1].resolve(size[1])];
                let radius = match radius {
                    CircleRadius::Extent(extent) => extent.circle(center, size),
                    CircleRadius::Length(length) => length.resolve(direction_agnostic(size)),
                };
                (
                    vec![ellipse(
                        [origin[0] + center[0], origin[1] + center[1]],
                        [radius; 2],
                    )],
                    false,
                )
            }
            Self::Ellipse { radii, center } => {
                let center = [center[0].resolve(size[0]), center[1].resolve(size[1])];
                let radii = match radii {
                    EllipseRadii::Extent(extent) => extent.ellipse(center, size),
                    EllipseRadii::Axes([x, y]) => {
                        [x.resolve(center[0], size[0]), y.resolve(center[1], size[1])]
                    }
                };
                (
                    vec![ellipse(
                        [origin[0] + center[0], origin[1] + center[1]],
                        radii,
                    )],
                    false,
                )
            }
            Self::Polygon { even_odd, points } => {
                let points = points
                    .iter()
                    .map(|[x, y]| {
                        [
                            origin[0] + x.resolve(size[0]),
                            origin[1] + y.resolve(size[1]),
                        ]
                    })
                    .collect::<Vec<_>>();
                (vec![Contour::polygon(&points)], *even_odd)
            }
            Self::Path { even_odd, contours } => {
                let place = |[x, y]: [f32; 2]| {
                    [
                        origin[0] + x * POINTS_PER_PIXEL,
                        origin[1] + y * POINTS_PER_PIXEL,
                    ]
                };
                let contours = contours.iter().map(|contour| contour.map(place)).collect();
                (contours, *even_odd)
            }
        };
        (contours, even_odd)
    }
}

/// Inset rectangle radii: percentages of the reference box, constrained to the rectangle.
fn rounded_inset(rect: Rect, radii: &CornerRadii, reference: [f32; 2]) -> Contour {
    rounded::rounded_rect(
        rect,
        used_radii(radii, reference, [rect.width, rect.height]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::{PositionComponent, Segment};

    const REFERENCE: Rect = Rect {
        x: 10.0,
        y: 20.0,
        width: 100.0,
        height: 200.0,
    };

    fn square() -> CornerRadii {
        std::array::from_fn(|_| [LengthPercentage::ZERO; 2])
    }

    fn contours(shape: &BasicShape) -> Vec<Contour> {
        let Shape::Path { contours, .. } = shape.outline(REFERENCE) else {
            unreachable!("basic shapes outline as paths")
        };
        contours
    }

    #[test]
    fn rectangle_functions_resolve_overlapping_edges_as_their_examples_show() {
        let percent = LengthPercentage::Percentage;
        let point = LengthPercentage::Length;
        for (label, shape, equivalent) in [
            (
                "§3.1: inset(75% 0 50% 0) reduces its vertical insets to 60% and 40%",
                BasicShape::Inset {
                    insets: [percent(0.75), point(0.0), percent(0.5), point(0.0)],
                    radii: square(),
                },
                BasicShape::Inset {
                    insets: [percent(0.6), point(0.0), percent(0.4), point(0.0)],
                    radii: square(),
                },
            ),
            (
                "§3.1: rect(10px 0 0 20px) floors its right and bottom edges to rect(10px 20px \
                 10px 20px)",
                BasicShape::Rect {
                    edges: [point(7.5), point(0.0), point(0.0), point(15.0)],
                    radii: square(),
                },
                BasicShape::Rect {
                    edges: [point(7.5), point(15.0), point(7.5), point(15.0)],
                    radii: square(),
                },
            ),
        ] {
            assert_eq!(contours(&shape), contours(&equivalent), "{label}");
        }
    }

    #[test]
    fn circle_percentages_and_extents_resolve_against_the_reference_box() {
        let center = [PositionComponent::START; 2];
        for (label, radius, expected) in [
            // §1.3: the direction-agnostic size of a 100 × 200 box is √(100² + 200²) / √2.
            (
                "50% of the direction-agnostic size",
                CircleRadius::Length(LengthPercentage::Percentage(0.5)),
                0.5 * 100.0_f32.hypot(200.0) / std::f32::consts::SQRT_2,
            ),
            (
                "closest-side from the top-left corner",
                CircleRadius::Extent(RadialExtent::ClosestSide),
                0.0,
            ),
            (
                "farthest-corner from the top-left corner",
                CircleRadius::Extent(RadialExtent::FarthestCorner),
                100.0_f32.hypot(200.0),
            ),
        ] {
            let contour = &contours(&BasicShape::Circle {
                radius,
                center: center.clone(),
            })[0];
            let right = contour
                .segments
                .iter()
                .flat_map(|segment| match segment {
                    Segment::Line(point) => vec![*point],
                    Segment::Cubic(points) => points.to_vec(),
                })
                .map(|point| point[0])
                .fold(contour.start[0], f32::max);
            assert!(
                (right - REFERENCE.x - expected).abs() < 1e-3,
                "{label}: the circle reaches {right}, expected radius {expected}"
            );
        }
    }
}
