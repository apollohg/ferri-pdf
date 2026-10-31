//! CSS Motion Path 1 computed values and offset geometry.
use crate::paint::{BasicShape, GeometryBox, LengthPercentage, Position};
use crate::style::{
    StyleError, background_values,
    box_values::{length_percentage, unsupported},
    component::Component,
    grammar::{Keyword, Label, Production},
    shape_values,
    units::Relative,
};
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Motion {
    pub path: Arc<OffsetPath>,
    pub position: OffsetPosition,
    pub anchor: Option<Position>,
    pub distance: LengthPercentage,
    pub rotate: OffsetRotate,
}

impl Default for Motion {
    fn default() -> Self {
        Self {
            path: Arc::default(),
            position: OffsetPosition::Normal,
            anchor: None,
            distance: LengthPercentage::ZERO,
            rotate: OffsetRotate::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "offset paths are already shared through Arc"
)]
pub(crate) enum OffsetPath {
    #[default]
    None,
    Path {
        curve: Curve,
        reference: GeometryBox,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum Curve {
    Box,
    Shape {
        shape: BasicShape,
        explicit_center: bool,
    },
    Ray {
        angle: f32,
        size: RaySize,
        contain: bool,
        at: Option<Position>,
    },
    Reference(Box<str>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum RaySize {
    #[default]
    ClosestSide,
    ClosestCorner,
    FarthestSide,
    FarthestCorner,
    Sides,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) enum OffsetPosition {
    #[default]
    Normal,
    Auto,
    Position(Position),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct OffsetRotate {
    pub tangent: bool,
    pub angle: f32,
}
impl Default for OffsetRotate {
    fn default() -> Self {
        Self {
            tangent: true,
            angle: 0.0,
        }
    }
}

pub(super) fn position(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<OffsetPosition, StyleError> {
    Ok(match value.keyword() {
        Some(Keyword::Normal) => OffsetPosition::Normal,
        Some(Keyword::Auto) => OffsetPosition::Auto,
        _ => OffsetPosition::Position(background_values::position(value, relative)?),
    })
}
pub(super) fn anchor(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<Position>, StyleError> {
    if value.keyword() == Some(Keyword::Auto) {
        Ok(None)
    } else {
        background_values::position(value, relative).map(Some)
    }
}
pub(super) fn distance(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<LengthPercentage, StyleError> {
    length_percentage(value, relative)
}
pub(super) fn rotate(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<OffsetRotate, StyleError> {
    let mut output = OffsetRotate {
        tangent: false,
        angle: 0.0,
    };
    for item in crate::style::transform::values(value) {
        match item.keyword() {
            Some(Keyword::Auto) => output.tangent = true,
            Some(Keyword::Reverse) => {
                output.tangent = true;
                output.angle += std::f32::consts::PI;
            }
            _ => output.angle += crate::style::values::angle(item, relative)?.to_radians(),
        }
    }
    Ok(output)
}
pub(super) fn path(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Arc<OffsetPath>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(Arc::new(OffsetPath::None));
    }
    let reference = value
        .slot(1)
        .map(shape_values::geometry_box)
        .transpose()?
        .unwrap_or(GeometryBox::Shape(crate::paint::ShapeBox::Visual(
            crate::paint::BoxArea::BorderBox,
        )));
    let curve = match value.slot(0) {
        None => Curve::Box,
        Some(component) => match component.unwrap() {
            Component::Url(url) => Curve::Reference(url.clone()),
            Component::Function(Keyword::Ray, _) => {
                let fields = component
                    .arguments()
                    .first()
                    .ok_or_else(|| unsupported(component))?;
                let angle = fields.slot(0).ok_or_else(|| unsupported(component))?;
                let size = match component
                    .find(Label::Production(Production::RaySize))
                    .and_then(Component::keyword)
                {
                    None | Some(Keyword::ClosestSide) => RaySize::ClosestSide,
                    Some(Keyword::ClosestCorner) => RaySize::ClosestCorner,
                    Some(Keyword::FarthestSide) => RaySize::FarthestSide,
                    Some(Keyword::FarthestCorner) => RaySize::FarthestCorner,
                    Some(Keyword::Sides) => RaySize::Sides,
                    _ => return Err(unsupported(component)),
                };
                Curve::Ray {
                    angle: crate::style::values::angle(angle, relative)?.to_radians(),
                    size,
                    contain: crate::style::transform::arguments(component)
                        .iter()
                        .any(|item| item.keyword() == Some(Keyword::Contain)),
                    at: component
                        .find(Label::Production(Production::Position))
                        .map(|at| background_values::position(at, relative))
                        .transpose()?,
                }
            }
            _ => Curve::Shape {
                shape: shape_values::basic_shape(component, relative)?,
                explicit_center: component
                    .find(Label::Production(Production::Position))
                    .is_some(),
            },
        },
    };
    Ok(Arc::new(OffsetPath::Path { curve, reference }))
}

use crate::paint::{BoxAreas, Contour, CornerRadii, Point, Rect, Segment};
use kurbo::{ParamCurve, ParamCurveArclen, ParamCurveDeriv, ParamCurveExtrema};

/// Actual containing element's boxes, in the same untransformed coordinates as the moving box.
#[derive(Clone, Debug)]
pub(crate) struct Reference {
    pub areas: BoxAreas<Rect>,
    pub border_radius: Option<Arc<CornerRadii>>,
    pub path: Option<Arc<Vec<Contour>>>,
}
impl Reference {
    pub(crate) fn rect(rect: Rect) -> Self {
        Self {
            areas: BoxAreas::single(rect),
            border_radius: None,
            path: None,
        }
    }
    pub(crate) fn paint<I>(paint: &crate::paint::BoxPaint<I>) -> Self {
        Self {
            areas: paint.fragment_areas(),
            border_radius: Some(paint.decoration.radii.clone()),
            path: None,
        }
    }
    pub(crate) fn resolve(&self, reference: GeometryBox) -> Rect {
        use crate::paint::{BoxArea, ShapeBox};
        match reference.layout() {
            ShapeBox::Visual(BoxArea::BorderBox) => self.areas.border,
            ShapeBox::Visual(BoxArea::PaddingBox) => self.areas.padding,
            ShapeBox::Visual(BoxArea::ContentBox) => self.areas.content,
            ShapeBox::Margin => unreachable!("offset-path excludes margin-box"),
        }
    }
}

/// Arclength error in points, well below a print dot.
const PATH_ACCURACY: f64 = 1.0e-5;
const HALF: f32 = 0.5;

impl Motion {
    pub(crate) fn active(&self) -> bool {
        !matches!(*self.path, OffsetPath::None)
    }

    /// Offset point and orientation; anchor and transform-origin are resolved by the caller.
    pub(crate) fn placement(&self, own: Rect, containing: &Reference) -> Option<(Point, f32)> {
        let OffsetPath::Path { curve, reference } = &*self.path else {
            return None;
        };
        let reference = containing.resolve(*reference);
        let starting = match &self.position {
            OffsetPosition::Normal => None,
            OffsetPosition::Auto => Some([own.x, own.y]),
            OffsetPosition::Position(position) => Some(resolve_position(position, reference)),
        };
        let (point, direction) = if let Curve::Ray {
            angle,
            size,
            contain,
            at,
        } = curve
        {
            let start = at
                .as_ref()
                .map(|position| resolve_position(position, reference))
                .or(starting)
                .unwrap_or([
                    reference.x + reference.width * HALF,
                    reference.y + reference.height * HALF,
                ]);
            let vector = [angle.sin(), -angle.cos()];
            let sides = [
                start[0] - reference.x,
                reference.x + reference.width - start[0],
                start[1] - reference.y,
                reference.y + reference.height - start[1],
            ];
            let corners = [
                sides[0].hypot(sides[2]),
                sides[1].hypot(sides[2]),
                sides[0].hypot(sides[3]),
                sides[1].hypot(sides[3]),
            ];
            let length = match size {
                RaySize::ClosestSide => sides
                    .map(f32::abs)
                    .into_iter()
                    .fold(f32::INFINITY, f32::min),
                RaySize::FarthestSide => sides.map(f32::abs).into_iter().fold(0.0, f32::max),
                RaySize::ClosestCorner => corners.into_iter().fold(f32::INFINITY, f32::min),
                RaySize::FarthestCorner => corners.into_iter().fold(0.0, f32::max),
                RaySize::Sides if sides.iter().any(|side| *side <= 0.0) => 0.0,
                RaySize::Sides => [
                    if vector[0] > 0.0 {
                        sides[1] / vector[0]
                    } else if vector[0] < 0.0 {
                        -sides[0] / vector[0]
                    } else {
                        f32::INFINITY
                    },
                    if vector[1] > 0.0 {
                        sides[3] / vector[1]
                    } else if vector[1] < 0.0 {
                        -sides[2] / vector[1]
                    } else {
                        f32::INFINITY
                    },
                ]
                .into_iter()
                .fold(f32::INFINITY, f32::min),
            };
            let length = if *contain {
                (length - own.width.max(own.height) * HALF).max(0.0)
            } else {
                length
            };
            let distance = self.distance.resolve(length);
            let distance = if *contain {
                distance.clamp(0.0, length)
            } else if length == 0.0 {
                0.0
            } else {
                distance
            };
            (
                [
                    start[0] + vector[0] * distance,
                    start[1] + vector[1] * distance,
                ],
                vector[1].atan2(vector[0]),
            )
        } else {
            let contours = match curve {
                Curve::Box => vec![box_path(reference, containing.border_radius.as_deref())],
                Curve::Shape {
                    shape,
                    explicit_center,
                } => {
                    let mut shape = shape.clone();
                    if !*explicit_center && let Some(start) = starting {
                        match &mut shape {
                            BasicShape::Circle { center, .. }
                            | BasicShape::Ellipse { center, .. } => {
                                *center =
                                    std::array::from_fn(|axis| crate::paint::PositionComponent {
                                        from_end: false,
                                        offset: LengthPercentage::Length(
                                            start[axis] - [reference.x, reference.y][axis],
                                        ),
                                    });
                            }
                            _ => {}
                        }
                    }
                    let (mut paths, _) = shape.path(reference);
                    match shape {
                        BasicShape::Circle { .. } | BasicShape::Ellipse { .. } => {
                            for path in &mut paths {
                                const ELLIPSE_RIGHT_EDGE: usize = 3;
                                rotate_start(path, ELLIPSE_RIGHT_EDGE);
                            }
                        }
                        BasicShape::Inset { .. } | BasicShape::Rect { .. } => {
                            for path in &mut paths {
                                rectangle_start(path);
                            }
                        }
                        _ => {}
                    }
                    paths
                }
                Curve::Reference(_) => containing.path.as_ref().map_or_else(
                    || {
                        vec![Contour {
                            start: [reference.x, reference.y],
                            segments: Vec::new(),
                            closed: false,
                        }]
                    },
                    |paths| {
                        paths
                            .iter()
                            .map(|path| {
                                let place = |[x, y]: Point| {
                                    [
                                        reference.x + x * crate::paint::POINTS_PER_PIXEL,
                                        reference.y + y * crate::paint::POINTS_PER_PIXEL,
                                    ]
                                };
                                path.map(place)
                            })
                            .collect()
                    },
                ),
                Curve::Ray { .. } => unreachable!(),
            };
            sample(&contours, &self.distance)
        };
        Some((
            point,
            self.rotate.angle + if self.rotate.tangent { direction } else { 0.0 },
        ))
    }
}
fn resolve_position(position: &Position, reference: Rect) -> Point {
    [
        reference.x + position[0].resolve(reference.width),
        reference.y + position[1].resolve(reference.height),
    ]
}
fn rotate_start(path: &mut Contour, count: usize) {
    if count == 0 || path.segments.len() < count {
        return;
    }
    let original = path.start;
    path.start = path.segments[count - 1].end();
    path.segments.push(Segment::Line(original));
    path.segments.rotate_left(count);
}
fn rectangle_start(path: &mut Contour) {
    if matches!(path.segments.first(), Some(Segment::Cubic(_))) {
        rotate_start(path, 1);
    }
}
fn box_path(reference: Rect, radii: Option<&CornerRadii>) -> Contour {
    let shape = BasicShape::Inset {
        insets: [const { LengthPercentage::ZERO }; 4],
        radii: radii
            .cloned()
            .unwrap_or_else(|| std::array::from_fn(|_| [const { LengthPercentage::ZERO }; 2])),
    };
    let (mut paths, _) = shape.path(reference);
    let mut path = paths.remove(0);
    rectangle_start(&mut path);
    path
}
fn sample(contours: &[Contour], distance: &LengthPercentage) -> (Point, f32) {
    let mut curves = Vec::new();
    for contour in contours {
        for curve in kurbo::segments(contour.path_elements()) {
            let bounds = curve.bounding_box();
            let accuracy =
                crate::paint::bounded_tolerance(PATH_ACCURACY, bounds.width().max(bounds.height()));
            curves.push((curve, curve.arclen(accuracy), accuracy));
        }
    }
    let total = curves.iter().map(|(_, length, _)| length).sum::<f64>();
    if total == 0.0 {
        return (contours.first().map_or([0.0; 2], |path| path.start), 0.0);
    }
    // Complete percentage loops must share arclength precision to restart with the first tangent.
    let raw = match distance {
        LengthPercentage::Percentage(fraction) => f64::from(*fraction) * total,
        LengthPercentage::Length(length) => f64::from(*length),
        LengthPercentage::Calc(calculation) => {
            let basis = total as f32;
            let resolved = calculation.resolve(basis);
            let loops = resolved / basis;
            if loops.is_finite() && loops.fract() == 0.0 && loops * basis == resolved {
                f64::from(loops) * total
            } else {
                f64::from(resolved)
            }
        }
    };
    let mut remaining = if contours.last().is_some_and(|path| path.closed) {
        raw.rem_euclid(total)
    } else {
        raw.clamp(0.0, total)
    };
    let last = curves
        .iter()
        .rposition(|(_, length, _)| *length > 0.0)
        .unwrap_or(0);
    for (index, (curve, length, accuracy)) in curves.iter().enumerate() {
        if *length == 0.0 {
            continue;
        }
        if remaining < *length || index == last {
            let t = curve.inv_arclen(remaining.min(*length), *accuracy);
            let location = curve.eval(t);
            let tangent = match *curve {
                kurbo::PathSeg::Line(line) => line.p1 - line.p0,
                kurbo::PathSeg::Quad(quad) => quad.deriv().eval(t).to_vec2(),
                kurbo::PathSeg::Cubic(cubic) => {
                    let tangent = cubic.deriv().eval(t).to_vec2();
                    if tangent.hypot2() > 0.0 {
                        tangent
                    } else if t == 0.0 {
                        [cubic.p1, cubic.p2, cubic.p3]
                            .into_iter()
                            .map(|p| p - cubic.p0)
                            .find(|v| v.hypot2() > 0.0)
                            .unwrap_or_default()
                    } else if t == 1.0 {
                        [cubic.p2, cubic.p1, cubic.p0]
                            .into_iter()
                            .map(|p| cubic.p3 - p)
                            .find(|v| v.hypot2() > 0.0)
                            .unwrap_or_default()
                    } else {
                        cubic.deriv().deriv().eval(t).to_vec2()
                    }
                }
            };
            return (
                [location.x as f32, location.y as f32],
                tangent.atan2() as f32,
            );
        }
        remaining -= length;
    }
    unreachable!("nonzero total has a nonzero segment")
}

/// SVG's attribute cascade has already selected these values; use the CSS grammar and
/// computed handlers rather than a second offset parser.
pub(crate) fn svg_style<'a>(
    values: impl IntoIterator<Item = (&'a str, &'a str)>,
    relative: &Relative<'_>,
) -> Result<Box<crate::style::Style>, StyleError> {
    use crate::style::{
        declaration::{self, Specified},
        properties,
    };
    let mut initial = Box::<crate::style::Style>::default();
    initial.transform_origin = [const { LengthPercentage::ZERO }; 3];
    let mut style = initial.clone();
    for (name, source) in values {
        let Some(property) = crate::style::grammar::Property::from_name(name) else {
            continue;
        };
        if let Some(value) = declaration::value(property, source, &mut 0)? {
            let longhands = if property.longhands().is_empty() {
                vec![(property, Specified::Value(Arc::new(value)))]
            } else {
                crate::style::shorthands::expand(property, value)?
            };
            for (property, value) in longhands {
                properties::apply(property, &value, &mut style, &initial, relative);
            }
        }
    }
    Ok(style)
}
