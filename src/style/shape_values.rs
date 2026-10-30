//! CSS Shapes 1 `shape-outside` and its `<basic-shape>` functions from match trees.
use crate::paint::{
    AbsoluteColor, AxisRadius, BasicShape, BoxArea, CircleRadius, ClipPath, Contour, CornerRadii,
    EllipseRadii, GeometryBox, LengthPercentage, PositionComponent, RadialExtent, Segment,
    ShapeBox, ShapeOutside,
};
use crate::style::{
    StyleError, background_values,
    box_values::{length_percentage, unsupported},
    component::Component,
    grammar::{Keyword, Label, Production, Property},
    logical::{BOTTOM, RIGHT},
    properties::corner_radius,
    shorthands::{corner_values, edge_value},
    units::Relative,
};

/// `shape-outside` slots of `[ <shape-box> || <basic-shape> ]`.
const BOX_SLOT: usize = 0;
const SHAPE_SLOT: usize = 1;
/// `clip-path` slots of `[ <basic-shape> || <geometry-box> ]`.
const CLIP_SHAPE_SLOT: usize = 0;
const CLIP_BOX_SLOT: usize = 1;

pub(super) fn shape_outside(
    value: &Component,
    relative: &Relative<'_>,
    color: &AbsoluteColor,
) -> Result<ShapeOutside, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(ShapeOutside::None);
    }
    if value.slots().is_empty() {
        return Ok(ShapeOutside::Image(background_values::image(
            value, relative, color,
        )?));
    }
    let reference = value
        .slot(BOX_SLOT)
        .map(shape_box)
        .transpose()?
        .unwrap_or(ShapeBox::Margin);
    Ok(match value.slot(SHAPE_SLOT) {
        Some(shape) => ShapeOutside::Shape {
            shape: basic_shape(shape, relative)?,
            reference,
        },
        None => ShapeOutside::Box(reference),
    })
}

/// CSS Images 5 §3.1 `object-view-box`: `none`, or a `<basic-shape-rect>` resolved against
/// the element's natural size.
pub(super) fn view_box(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<BasicShape>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(None);
    }
    basic_shape(value, relative).map(Some)
}

/// A computed `clip-path`: `none`, a reference to a same-document `clipPath` element, or a
/// basic shape in its reference box.
pub(super) enum ClipPathValue {
    None,
    Reference(Box<str>),
    Shape(ClipPath),
}

/// CSS Masking 1 §6.1 `clip-path`.
pub(super) fn clip_path(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<ClipPathValue, StyleError> {
    match value.unwrap() {
        Component::Keyword(Keyword::None) => return Ok(ClipPathValue::None),
        Component::Url(url) => {
            return crate::style::svg_values::same_document(url, value)
                .map(ClipPathValue::Reference);
        }
        _ => {}
    }
    Ok(ClipPathValue::Shape(ClipPath {
        shape: value
            .slot(CLIP_SHAPE_SLOT)
            .map(|shape| basic_shape(shape, relative))
            .transpose()?,
        reference: value
            .slot(CLIP_BOX_SLOT)
            .map(geometry_box)
            .transpose()?
            .unwrap_or(GeometryBox::Shape(ShapeBox::Visual(BoxArea::BorderBox))),
    }))
}

pub(super) fn geometry_box(value: &Component) -> Result<GeometryBox, StyleError> {
    match value.keyword() {
        Some(Keyword::FillBox) => Ok(GeometryBox::Fill),
        Some(Keyword::StrokeBox) => Ok(GeometryBox::Stroke),
        Some(Keyword::ViewBox) => Ok(GeometryBox::View),
        _ => shape_box(value).map(GeometryBox::Shape),
    }
}

fn shape_box(value: &Component) -> Result<ShapeBox, StyleError> {
    Ok(match value.keyword() {
        Some(Keyword::MarginBox) => ShapeBox::Margin,
        Some(Keyword::BorderBox) => ShapeBox::Visual(BoxArea::BorderBox),
        Some(Keyword::PaddingBox) => ShapeBox::Visual(BoxArea::PaddingBox),
        Some(Keyword::ContentBox) => ShapeBox::Visual(BoxArea::ContentBox),
        _ => return Err(unsupported(value)),
    })
}

/// A `path()` string that parses as non-empty SVG 1.1 path data; any other makes the
/// declaration invalid (CSS Shapes 1 §3.1).
pub(super) fn valid(value: &Component) -> bool {
    value
        .find(Label::Production(Production::PathFn))
        .is_none_or(|path| {
            path_string(path)
                .and_then(|source| path_data(source, PathErrors::Discard))
                .is_some()
        })
}

fn path_string(path: &Component) -> Option<&str> {
    path.arguments().iter().find_map(|argument| match argument {
        Component::String(data) => Some(&**data),
        _ => None,
    })
}

pub(super) fn basic_shape(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<BasicShape, StyleError> {
    let function = value.unwrap();
    let Component::Function(name, _) = function else {
        return Err(unsupported(value));
    };
    let arguments = function.arguments();
    let lengths = |index: usize| -> Result<Vec<LengthPercentage>, StyleError> {
        arguments.get(index).map_or(Ok(Vec::new()), |group| {
            group
                .items()
                .iter()
                .map(|item| length_percentage(item, relative))
                .collect()
        })
    };
    let center = || match function.find(Label::Production(Production::Position)) {
        Some(position) => background_values::position(position, relative),
        None => Ok([PositionComponent::CENTER; 2]),
    };
    let even_odd = || {
        function
            .find(Label::Property(Property::FillRule))
            .and_then(Component::keyword)
            == Some(Keyword::Evenodd)
    };
    Ok(match name {
        Keyword::Inset => {
            let insets = lengths(0)?;
            BasicShape::Inset {
                insets: sides(&insets).ok_or_else(|| unsupported(value))?,
                radii: radii(function, relative)?,
            }
        }
        // CSS Shapes 1 §3.2: xywh(x y w h) is inset(y calc(100% - x - w) calc(100% - y - h) x).
        Keyword::Xywh => {
            let [offsets, size] = [lengths(0)?, lengths(1)?];
            BasicShape::Inset {
                insets: [
                    offsets[1].clone(),
                    remainder(&[&offsets[0], &size[0]]),
                    remainder(&[&offsets[1], &size[1]]),
                    offsets[0].clone(),
                ],
                radii: radii(function, relative)?,
            }
        }
        // `auto` places an edge on the reference box's corresponding edge.
        Keyword::Rect => {
            let edges = arguments
                .first()
                .map_or(&[][..], Component::items)
                .iter()
                .enumerate()
                .map(|(side, edge)| match edge.keyword() {
                    Some(Keyword::Auto) => Ok(auto_edge(side)),
                    _ => length_percentage(edge, relative),
                })
                .collect::<Result<Vec<_>, StyleError>>()?;
            BasicShape::Rect {
                edges: edges.try_into().map_err(|_| unsupported(value))?,
                radii: radii(function, relative)?,
            }
        }
        Keyword::Circle => BasicShape::Circle {
            radius: match extent(function) {
                Some(extent) => CircleRadius::Extent(extent),
                None => match lengths(0)?.pop() {
                    Some(radius) => CircleRadius::Length(radius),
                    None => CircleRadius::Extent(RadialExtent::ClosestSide),
                },
            },
            center: center()?,
        },
        Keyword::Ellipse => BasicShape::Ellipse {
            radii: match (extent(function), arguments.first().map(Component::items)) {
                (Some(extent), _) => EllipseRadii::Extent(extent),
                (None, Some([axes])) => {
                    let axes = axes.items();
                    EllipseRadii::Axes([
                        axis_radius(&axes[0], relative)?,
                        axis_radius(&axes[1], relative)?,
                    ])
                }
                _ => EllipseRadii::Axes([AxisRadius::ClosestSide, AxisRadius::ClosestSide]),
            },
            center: center()?,
        },
        Keyword::Polygon => BasicShape::Polygon {
            even_odd: even_odd(),
            points: arguments
                .last()
                .map_or(&[][..], Component::items)
                .iter()
                .map(|pair| {
                    let pair = pair.items();
                    Ok([
                        length_percentage(&pair[0], relative)?,
                        length_percentage(&pair[1], relative)?,
                    ])
                })
                .collect::<Result<_, StyleError>>()?,
        },
        Keyword::Path => BasicShape::Path {
            even_odd: even_odd(),
            contours: path_string(function)
                .and_then(|source| path_data(source, PathErrors::Discard))
                .ok_or_else(|| StyleError::Invalid(format!("path data: {value}")))?,
        },
        _ => return Err(unsupported(value)),
    })
}

/// One to four values in `margin` order expanded to the four sides or corners.
pub(crate) fn sides<T: Clone>(values: &[T]) -> Option<[T; 4]> {
    (1..=4)
        .contains(&values.len())
        .then(|| std::array::from_fn(|side| values[edge_value(values.len(), side)].clone()))
}

/// A `rect()` edge of `auto`: the reference box's corresponding edge.
pub(crate) fn auto_edge(side: usize) -> LengthPercentage {
    LengthPercentage::Percentage(if side == RIGHT || side == BOTTOM {
        1.0
    } else {
        0.0
    })
}

/// `calc(100% - a - b)`.
pub(crate) fn remainder(terms: &[&LengthPercentage]) -> LengthPercentage {
    use crate::paint::Calculation;
    let mut sum = vec![Calculation::Percentage(1.0)];
    sum.extend(terms.iter().map(|term| {
        match term {
            LengthPercentage::Length(value) => Calculation::Value(*value),
            LengthPercentage::Percentage(fraction) => Calculation::Percentage(*fraction),
            LengthPercentage::Calc(calculation) => (**calculation).clone(),
        }
        .negate()
    }));
    Calculation::Sum(sum).into()
}

fn extent(function: &Component) -> Option<RadialExtent> {
    function
        .find(Label::Production(Production::RadialExtent))
        .and_then(Component::keyword)
        .map(|keyword| match keyword {
            Keyword::ClosestSide => RadialExtent::ClosestSide,
            Keyword::FarthestSide => RadialExtent::FarthestSide,
            Keyword::ClosestCorner => RadialExtent::ClosestCorner,
            _ => RadialExtent::FarthestCorner,
        })
}

fn axis_radius(value: &Component, relative: &Relative<'_>) -> Result<AxisRadius, StyleError> {
    Ok(match value.keyword() {
        Some(Keyword::ClosestSide) => AxisRadius::ClosestSide,
        Some(Keyword::FarthestSide) => AxisRadius::FarthestSide,
        _ => AxisRadius::Length(length_percentage(value, relative)?),
    })
}

/// The `round <'border-radius'>` corners of a rectangle function; square when omitted.
fn radii(function: &Component, relative: &Relative<'_>) -> Result<CornerRadii, StyleError> {
    let Some(radius) = function.find(Label::Property(Property::BorderRadius)) else {
        return Ok(std::array::from_fn(|_| [LengthPercentage::ZERO; 2]));
    };
    let corners = corner_values(radius);
    let mut radii: CornerRadii = std::array::from_fn(|_| [LengthPercentage::ZERO; 2]);
    for (corner, value) in radii.iter_mut().zip(&corners) {
        *corner = corner_radius(value, relative)?;
    }
    Ok(radii)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathErrors {
    Discard,
    Prefix,
}

/// Absolute contours; SVG references retain the valid prefix on malformed input.
pub(crate) fn path_data(source: &str, errors: PathErrors) -> Option<Vec<Contour>> {
    use svgtypes::SimplePathSegment as Command;
    let point = |x: f64, y: f64| [x as f32, y as f32];
    let mut contours: Vec<Contour> = Vec::new();
    let mut open: Option<Contour> = None;
    let mut subpath_start = [0.0; 2];
    let mut current = [0.0; 2];
    let mut commands = 0usize;
    for command in svgtypes::SimplifyingPathParser::from(source) {
        let command = match command {
            Ok(command) => command,
            Err(_) if errors == PathErrors::Discard => return None,
            Err(_) => break,
        };
        commands += 1;
        let segment = match command {
            Command::MoveTo { x, y } => {
                contours.extend(open.take());
                subpath_start = point(x, y);
                current = subpath_start;
                open = Some(Contour {
                    start: subpath_start,
                    segments: Vec::new(),
                    closed: false,
                });
                continue;
            }
            Command::ClosePath => {
                if let Some(contour) = &mut open {
                    contour.closed = true;
                }
                contours.extend(open.take());
                current = subpath_start;
                continue;
            }
            Command::LineTo { x, y } => Segment::Line(point(x, y)),
            Command::CurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => Segment::Cubic([point(x1, y1), point(x2, y2), point(x, y)]),
            Command::Quadratic { x1, y1, x, y } => {
                Segment::quadratic(current, point(x1, y1), point(x, y))
            }
        };
        current = segment.end();
        open.get_or_insert_with(|| Contour {
            start: subpath_start,
            segments: Vec::new(),
            closed: false,
        })
        .segments
        .push(segment);
    }
    contours.extend(open);
    (commands > 0).then_some(contours)
}
