//! CSS Masking 1 §6.1 `clip-path: <basic-shape> || <geometry-box>`: the value parsed into the
//! document's computed basic shapes and traced by them in the element's reference box.

use std::sync::Arc;

use cssparser::{Parser, ParserInput, Token};
use svgtypes::{Length, LengthUnit as Unit};
use tiny_skia_path::PathBuilder;

use super::converter::{self, Cache};
use super::svgtree::{AId, SvgNode};
use super::units::convert_length;
use crate::paint::{
    AxisRadius, BasicShape, BoxArea, Calculation, CircleRadius, CornerRadii, EllipseRadii,
    GeometryBox, LengthPercentage, POINTS_PER_PIXEL, Position, PositionComponent, RadialExtent,
    Segment, ShapeBox, SvgBoxes,
};
use crate::style::shape_values::{PathErrors, auto_edge, path_data, remainder, sides};
use crate::svg::usvg::{ClipPath, Fill, FillRule, Group, Node, Path, Rect, Transform, Units};

const CALC: &str = "calc";
const ROUND: &str = "round";
const AT: &str = "at";
const AUTO: &str = "auto";

/// A parsed `clip-path` basic shape; without a shape it clips to the reference box itself.
pub(crate) struct BasicShapeClip {
    shape: Option<BasicShape>,
    reference: GeometryBox,
}

/// The arguments of a basic shape function, lengths in points.
enum Item {
    Ident(String),
    Length(LengthPercentage),
    Comma,
    Slash,
    String(String),
}

/// Parses a `clip-path` value that is not a reference; `None` when it is `none` or invalid,
/// which clips nothing.
pub(crate) fn parse(
    value: &str,
    node: SvgNode,
    state: &converter::State,
) -> Option<BasicShapeClip> {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let mut shape = None;
    let mut reference = None;
    while let Ok(token) = parser.next() {
        match token.clone() {
            Token::Function(name) if shape.is_none() => {
                let items = nested(&mut parser, |block| items(block, node, state))?;
                shape = Some(self::shape(&name.to_ascii_lowercase(), &items)?);
            }
            Token::Ident(name) if reference.is_none() => {
                reference = Some(match &*name.to_ascii_lowercase() {
                    converter::FILL_BOX => GeometryBox::Fill,
                    converter::STROKE_BOX => GeometryBox::Stroke,
                    "view-box" => GeometryBox::View,
                    "content-box" => GeometryBox::Shape(ShapeBox::Visual(BoxArea::ContentBox)),
                    "padding-box" => GeometryBox::Shape(ShapeBox::Visual(BoxArea::PaddingBox)),
                    "border-box" => GeometryBox::Shape(ShapeBox::Visual(BoxArea::BorderBox)),
                    "margin-box" => GeometryBox::Shape(ShapeBox::Margin),
                    _ => return None,
                });
            }
            _ => return None,
        }
    }
    (shape.is_some() || reference.is_some()).then_some(BasicShapeClip {
        shape,
        reference: reference.unwrap_or(GeometryBox::Stroke),
    })
}

/// The result of `parse` on the block after a function token.
fn nested<T>(
    parser: &mut Parser<'_, '_>,
    parse: impl FnOnce(&mut Parser<'_, '_>) -> Option<T>,
) -> Option<T> {
    parser
        .parse_nested_block(|block| Ok::<_, cssparser::ParseError<'_, ()>>(parse(block)))
        .ok()
        .flatten()
}

/// The arguments of a function block, `None` when one is not a length, a linear `calc()`, a
/// keyword, a comma, a slash or a string.
fn items(block: &mut Parser<'_, '_>, node: SvgNode, state: &converter::State) -> Option<Vec<Item>> {
    let mut items = Vec::new();
    while let Ok(token) = block.next() {
        let token = token.clone();
        items.push(match token {
            Token::Ident(name) => Item::Ident(name.to_ascii_lowercase()),
            Token::Comma => Item::Comma,
            Token::Delim('/') => Item::Slash,
            Token::QuotedString(string) => Item::String(string.to_string()),
            Token::Function(name) if name.eq_ignore_ascii_case(CALC) => {
                Item::Length(nested(block, |sum| calculation(sum, node, state))?)
            }
            token => Item::Length(length(&token, node, state)?),
        });
    }
    Some(items)
}

/// A length or percentage token, lengths in points; unitless numbers are user units.
fn length(token: &Token<'_>, node: SvgNode, state: &converter::State) -> Option<LengthPercentage> {
    let points = |value: f32, unit| {
        let length = Length::new(f64::from(value), unit);
        LengthPercentage::Length(
            convert_length(length, node, AId::Width, Units::UserSpaceOnUse, state)
                * POINTS_PER_PIXEL,
        )
    };
    Some(match token {
        Token::Number { value, .. } => points(*value, Unit::None),
        Token::Percentage { unit_value, .. } => LengthPercentage::Percentage(*unit_value),
        Token::Dimension { value, unit, .. } => points(
            *value,
            match &*unit.to_ascii_lowercase() {
                "px" => Unit::Px,
                "em" => Unit::Em,
                "ex" => Unit::Ex,
                "in" => Unit::In,
                "cm" => Unit::Cm,
                "mm" => Unit::Mm,
                "pt" => Unit::Pt,
                "pc" => Unit::Pc,
                _ => return None,
            },
        ),
        _ => return None,
    })
}

/// A `calc()` sum of lengths and percentages, as the document writes `xywh()` insets.
fn calculation(
    sum: &mut Parser<'_, '_>,
    node: SvgNode,
    state: &converter::State,
) -> Option<LengthPercentage> {
    let mut terms = Vec::new();
    let mut sign = 1.0;
    while let Ok(token) = sum.next() {
        match token.clone() {
            Token::Delim('+') => sign = 1.0,
            Token::Delim('-') => sign = -1.0,
            token => {
                terms.push(match length(&token, node, state)? {
                    LengthPercentage::Length(points) => Calculation::Value(sign * points),
                    LengthPercentage::Percentage(fraction) => {
                        Calculation::Percentage(sign * fraction)
                    }
                    LengthPercentage::Calc(_) => return None,
                });
                sign = 1.0;
            }
        }
    }
    Some(Calculation::Sum(terms).into())
}

fn shape(name: &str, items: &[Item]) -> Option<BasicShape> {
    let lengths = |items: &[Item]| {
        items
            .iter()
            .map(|item| match item {
                Item::Length(length) => Some(length.clone()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
    };
    // CSS Shapes 1 §3.1: `round <'border-radius'>` rounds the rectangle functions.
    let round = items
        .iter()
        .position(|item| matches!(item, Item::Ident(name) if name == ROUND));
    let (arguments, corners) = items.split_at(round.unwrap_or(items.len()));
    let radii = || -> Option<CornerRadii> {
        let Some((_, corners)) = corners.split_first() else {
            return Some(std::array::from_fn(|_| [LengthPercentage::ZERO; 2]));
        };
        let mut axes = corners.split(|item| matches!(item, Item::Slash));
        let horizontal = sides(&lengths(axes.next()?)?)?;
        let vertical = match axes.next() {
            Some(vertical) => sides(&lengths(vertical)?)?,
            None => horizontal.clone(),
        };
        axes.next().is_none().then(|| {
            std::array::from_fn(|corner| [horizontal[corner].clone(), vertical[corner].clone()])
        })
    };
    let even_odd = |items: &[Item]| -> (bool, usize) {
        match items {
            [Item::Ident(rule), Item::Comma, ..] if rule == "evenodd" => (true, 2),
            [Item::Ident(rule), Item::Comma, ..] if rule == "nonzero" => (false, 2),
            _ => (false, 0),
        }
    };
    Some(match name {
        "inset" => BasicShape::Inset {
            insets: sides(&lengths(arguments)?)?,
            radii: radii()?,
        },
        // CSS Shapes 1 §3.2: xywh(x y w h) is inset(y calc(100% - x - w) calc(100% - y - h) x).
        "xywh" => {
            let [x, y, width, height]: [LengthPercentage; 4] =
                lengths(arguments)?.try_into().ok()?;
            BasicShape::Inset {
                insets: [
                    y.clone(),
                    remainder(&[&x, &width]),
                    remainder(&[&y, &height]),
                    x,
                ],
                radii: radii()?,
            }
        }
        "rect" => BasicShape::Rect {
            edges: arguments
                .iter()
                .enumerate()
                .map(|(side, item)| match item {
                    Item::Length(length) => Some(length.clone()),
                    Item::Ident(name) if name == AUTO => Some(auto_edge(side)),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?
                .try_into()
                .ok()?,
            radii: radii()?,
        },
        "circle" | "ellipse" => {
            let at = items
                .iter()
                .position(|item| matches!(item, Item::Ident(name) if name == AT));
            let (size, center) = items.split_at(at.unwrap_or(items.len()));
            let center = match center.split_first() {
                Some((_, position)) => self::position(position)?,
                None => [PositionComponent::CENTER; 2],
            };
            if name == "circle" {
                BasicShape::Circle {
                    radius: match size {
                        [] => CircleRadius::Extent(RadialExtent::ClosestSide),
                        [Item::Length(radius)] => CircleRadius::Length(radius.clone()),
                        [extent] => CircleRadius::Extent(self::extent(extent)?),
                        _ => return None,
                    },
                    center,
                }
            } else {
                BasicShape::Ellipse {
                    radii: match size {
                        [] => {
                            EllipseRadii::Axes([AxisRadius::ClosestSide, AxisRadius::ClosestSide])
                        }
                        [extent] => EllipseRadii::Extent(self::extent(extent)?),
                        [x, y] => EllipseRadii::Axes([axis(x)?, axis(y)?]),
                        _ => return None,
                    },
                    center,
                }
            }
        }
        "polygon" => {
            let (even_odd, skip) = even_odd(items);
            BasicShape::Polygon {
                even_odd,
                points: items[skip..]
                    .split(|item| matches!(item, Item::Comma))
                    .map(|point| lengths(point)?.try_into().ok())
                    .collect::<Option<Vec<[LengthPercentage; 2]>>>()?,
            }
        }
        "path" => {
            let (even_odd, skip) = even_odd(items);
            let [Item::String(data)] = &items[skip..] else {
                return None;
            };
            BasicShape::Path {
                even_odd,
                contours: path_data(data, PathErrors::Discard)?,
            }
        }
        _ => return None,
    })
}

fn extent(item: &Item) -> Option<RadialExtent> {
    let Item::Ident(name) = item else {
        return None;
    };
    Some(match name.as_str() {
        "closest-side" => RadialExtent::ClosestSide,
        "farthest-side" => RadialExtent::FarthestSide,
        "closest-corner" => RadialExtent::ClosestCorner,
        "farthest-corner" => RadialExtent::FarthestCorner,
        _ => return None,
    })
}

fn axis(item: &Item) -> Option<AxisRadius> {
    match (item, extent(item)) {
        (Item::Length(length), _) => Some(AxisRadius::Length(length.clone())),
        (_, Some(RadialExtent::ClosestSide)) => Some(AxisRadius::ClosestSide),
        (_, Some(RadialExtent::FarthestSide)) => Some(AxisRadius::FarthestSide),
        _ => None,
    }
}

/// CSS Values 4 §9.1 `<position>`, as horizontal then vertical components.
fn position(items: &[Item]) -> Option<Position> {
    #[derive(Clone, Copy, PartialEq)]
    enum Axis {
        Horizontal,
        Vertical,
        Either,
    }
    let edge = |from_end| PositionComponent {
        from_end,
        offset: LengthPercentage::ZERO,
    };
    let keyword = |item: &Item| match item {
        Item::Ident(name) => match name.as_str() {
            "left" => Some((Axis::Horizontal, edge(false))),
            "right" => Some((Axis::Horizontal, edge(true))),
            "top" => Some((Axis::Vertical, edge(false))),
            "bottom" => Some((Axis::Vertical, edge(true))),
            "center" => Some((Axis::Either, PositionComponent::CENTER)),
            _ => None,
        },
        _ => None,
    };
    let length = |item: &Item| match item {
        Item::Length(length) => Some(length.clone()),
        _ => None,
    };
    let value = |item: &Item| {
        keyword(item).or_else(|| {
            Some((
                Axis::Either,
                PositionComponent {
                    from_end: false,
                    offset: length(item)?,
                },
            ))
        })
    };
    match items {
        [only] => {
            let (axis, component) = value(only)?;
            Some(if axis == Axis::Vertical {
                [PositionComponent::CENTER, component]
            } else {
                [component, PositionComponent::CENTER]
            })
        }
        [first, second] => {
            let (first_axis, first) = value(first)?;
            let (second_axis, second) = value(second)?;
            match (first_axis, second_axis) {
                (Axis::Vertical, Axis::Vertical) | (Axis::Horizontal, Axis::Horizontal) => None,
                (Axis::Vertical, _) | (_, Axis::Horizontal) => Some([second, first]),
                _ => Some([first, second]),
            }
        }
        [_, _, _] | [_, _, _, _] => {
            let mut pairs = Vec::new();
            let mut rest = items;
            while let Some((first, tail)) = rest.split_first() {
                let (axis, mut component) = keyword(first)?;
                rest = tail;
                if let Some(offset) = rest.first().and_then(length) {
                    if axis == Axis::Either {
                        return None;
                    }
                    component.offset = offset;
                    rest = &rest[1..];
                }
                pairs.push((axis, component));
            }
            match pairs.as_slice() {
                [(Axis::Vertical, y), (Axis::Horizontal | Axis::Either, x)]
                | [
                    (Axis::Horizontal | Axis::Either, x),
                    (Axis::Vertical | Axis::Either, y),
                ] => Some([x.clone(), y.clone()]),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The clip `clip` makes in `boxes`; `None` when it encloses nothing, which hides the element.
pub(crate) fn convert(
    clip: &BasicShapeClip,
    boxes: &SvgBoxes,
    state: &converter::State,
    cache: &mut Cache,
) -> Option<Arc<ClipPath>> {
    let reference = boxes.area(clip.reference);
    let (rule, data) = match &clip.shape {
        None => (
            FillRule::NonZero,
            PathBuilder::from_rect(Rect::from_xywh(
                reference.x / POINTS_PER_PIXEL,
                reference.y / POINTS_PER_PIXEL,
                reference.width / POINTS_PER_PIXEL,
                reference.height / POINTS_PER_PIXEL,
            )?),
        ),
        Some(shape) => outline(shape, reference)?,
    };
    let fill = Fill {
        rule,
        ..Fill::default()
    };
    let path = Path::new(
        String::new(),
        true,
        Some(fill),
        None,
        Default::default(),
        state.opt.shape_rendering,
        Arc::new(data),
        Transform::default(),
    )?;
    let mut root = Group::empty();
    root.children.push(Node::Path(Box::new(path)));
    root.calculate_bounding_boxes();
    Some(Arc::new(ClipPath {
        id: cache.gen_clip_path_id(),
        transform: Transform::default(),
        clip_path: None,
        root,
    }))
}

/// The outline `shape` traces in `reference`, in user units, as boxes trace it in points.
fn outline(
    shape: &BasicShape,
    reference: crate::paint::Rect,
) -> Option<(FillRule, tiny_skia_path::Path)> {
    let crate::paint::Shape::Path {
        contours, even_odd, ..
    } = shape.outline(reference)
    else {
        return None;
    };
    let user = |[x, y]: [f32; 2]| (x / POINTS_PER_PIXEL, y / POINTS_PER_PIXEL);
    let mut builder = PathBuilder::new();
    for contour in &contours {
        let (x, y) = user(contour.start);
        builder.move_to(x, y);
        for segment in &contour.segments {
            match *segment {
                Segment::Line(point) => {
                    let (x, y) = user(point);
                    builder.line_to(x, y);
                }
                Segment::Cubic(points) => {
                    let [(x1, y1), (x2, y2), (x, y)] = points.map(user);
                    builder.cubic_to(x1, y1, x2, y2, x, y);
                }
            }
        }
        if contour.closed {
            builder.close();
        }
    }
    let rule = if even_odd {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    };
    Some((rule, builder.finish()?))
}
