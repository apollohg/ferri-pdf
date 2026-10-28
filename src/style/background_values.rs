//! CSS Backgrounds 3 longhands and CSS Images 3 gradients from match trees.
use crate::style::{
    StyleError,
    box_values::{length_percentage, unsupported},
    component::Component,
    grammar::{Keyword, Label, Production},
    units::Relative,
};
use ferri_pdf_paint::{
    Attachment, BackgroundImage, BackgroundSize, BoxArea, CircleSize, Color, EllipseSize, Gradient,
    GradientItem, GradientShape, LengthPercentage, LineDirection, Position, PositionComponent,
    RadialEnding, RadialExtent, Repeat, SizeComponent,
};

const TO_TOP_DEG: f32 = 0.0;
const TO_RIGHT_DEG: f32 = 90.0;
const TO_BOTTOM_DEG: f32 = 180.0;
const TO_LEFT_DEG: f32 = 270.0;
const MIN_GRADIENT_STOPS: usize = 2;
const END: PositionComponent = PositionComponent {
    from_end: true,
    offset: LengthPercentage::ZERO,
};
/// `[ <angle> | <zero> | to <side-or-corner> ] || <color-interpolation-method>` slots.
const DIRECTION_SLOT: usize = 0;
/// `<side-or-corner>` slots.
const HORIZONTAL_SLOT: usize = 0;
const VERTICAL_SLOT: usize = 1;
const STOP_LABELS: [Label; 2] = [
    Label::Production(Production::LinearColorStop),
    Label::Production(Production::LinearColorHint),
];

/// One item per comma-separated layer.
fn list<T>(
    value: &Component,
    item: impl Fn(&Component) -> Result<T, StyleError>,
) -> Result<Vec<T>, StyleError> {
    value.items().iter().map(item).collect()
}

pub(super) fn images(
    value: &Component,
    relative: &Relative<'_>,
    color: Color,
) -> Result<Vec<BackgroundImage>, StyleError> {
    list(value, |item| image(item, relative, color))
}
pub(super) fn positions(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Vec<Position>, StyleError> {
    list(value, |item| position(item, relative))
}
pub(super) fn sizes(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Vec<BackgroundSize>, StyleError> {
    list(value, |item| size(item, relative))
}
pub(super) fn repeats(value: &Component) -> Result<Vec<[Repeat; 2]>, StyleError> {
    list(value, repeat)
}
pub(super) fn boxes(value: &Component) -> Result<Vec<BoxArea>, StyleError> {
    list(value, box_area)
}
pub(super) fn attachments(value: &Component) -> Result<Vec<Attachment>, StyleError> {
    list(value, attachment)
}

/// `<bg-image>`: `none`, a URL, or a linear or radial gradient.
pub(super) fn image(
    value: &Component,
    relative: &Relative<'_>,
    color: Color,
) -> Result<BackgroundImage, StyleError> {
    Ok(match value.unwrap() {
        Component::Keyword(Keyword::None) => BackgroundImage::None,
        Component::Url(url) => BackgroundImage::Url(url.to_string()),
        Component::Function(name, Some(arguments)) => {
            let repeating = matches!(
                name,
                Keyword::RepeatingLinearGradient | Keyword::RepeatingRadialGradient
            );
            BackgroundImage::Gradient(match name {
                Keyword::LinearGradient | Keyword::RepeatingLinearGradient => {
                    linear(arguments, relative, color, repeating)?
                }
                Keyword::RadialGradient | Keyword::RepeatingRadialGradient => {
                    radial(arguments, relative, color, repeating)?
                }
                _ => return Err(unsupported(value)),
            })
        }
        other => return Err(unsupported(other)),
    })
}

/// Interpolation in other colour spaces is not modelled.
fn srgb_only(arguments: &Component) -> Result<(), StyleError> {
    match arguments.find(Label::Production(Production::ColorInterpolationMethod)) {
        Some(method) => Err(unsupported(method)),
        None => Ok(()),
    }
}

fn linear(
    arguments: &Component,
    relative: &Relative<'_>,
    color: Color,
    repeating: bool,
) -> Result<Gradient, StyleError> {
    srgb_only(arguments)?;
    let syntax = arguments
        .find(Label::Production(Production::LinearGradientSyntax))
        .ok_or_else(|| unsupported(arguments))?;
    let direction = match syntax
        .items()
        .first()
        .and_then(|prelude| prelude.items().first())
        .and_then(|slots| slots.slot(DIRECTION_SLOT))
    {
        None => LineDirection::Angle(TO_BOTTOM_DEG),
        Some(direction) => match direction.find(Label::Production(Production::SideOrCorner)) {
            Some(side) => {
                let horizontal = side.slot(HORIZONTAL_SLOT).and_then(Component::keyword);
                let vertical = side.slot(VERTICAL_SLOT).and_then(Component::keyword);
                match (horizontal, vertical) {
                    (Some(h), Some(v)) => LineDirection::Corner {
                        right: h == Keyword::Right,
                        bottom: v == Keyword::Bottom,
                    },
                    (Some(Keyword::Right), None) => LineDirection::Angle(TO_RIGHT_DEG),
                    (Some(_), None) => LineDirection::Angle(TO_LEFT_DEG),
                    (None, Some(Keyword::Bottom)) => LineDirection::Angle(TO_BOTTOM_DEG),
                    _ => LineDirection::Angle(TO_TOP_DEG),
                }
            }
            None => LineDirection::Angle(crate::style::values::angle(direction, relative)?),
        },
    };
    Ok(Gradient {
        shape: GradientShape::Linear(direction),
        items: stop_list(syntax, relative, color)?,
        repeating,
    })
}

fn radial(
    arguments: &Component,
    relative: &Relative<'_>,
    color: Color,
    repeating: bool,
) -> Result<Gradient, StyleError> {
    srgb_only(arguments)?;
    let circle = arguments
        .find(Label::Production(Production::RadialShape))
        .and_then(Component::keyword)
        .map(|shape| shape == Keyword::Circle);
    let size = arguments.find(Label::Production(Production::RadialSize));
    let extent = size
        .and_then(|size| size.find(Label::Production(Production::RadialExtent)))
        .and_then(Component::keyword)
        .map(|keyword| match keyword {
            Keyword::ClosestSide => RadialExtent::ClosestSide,
            Keyword::FarthestSide => RadialExtent::FarthestSide,
            Keyword::ClosestCorner => RadialExtent::ClosestCorner,
            _ => RadialExtent::FarthestCorner,
        });
    let radii = match size.filter(|_| extent.is_none()) {
        Some(size) => size
            .items()
            .iter()
            .map(|radius| length_percentage(radius, relative))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    // CSS Images 3 §3.2.1: one length is a circle's radius, two are an ellipse's.
    let ending = match (circle, extent, radii.as_slice()) {
        (Some(true) | None, None, [LengthPercentage::Length(radius)]) => {
            RadialEnding::Circle(CircleSize::Radius(*radius))
        }
        (Some(false) | None, None, [x, y]) => {
            RadialEnding::Ellipse(EllipseSize::Radii([x.clone(), y.clone()]))
        }
        (Some(true), extent, []) => RadialEnding::Circle(CircleSize::Extent(
            extent.unwrap_or(RadialExtent::FarthestCorner),
        )),
        (Some(false) | None, extent, []) => RadialEnding::Ellipse(EllipseSize::Extent(
            extent.unwrap_or(RadialExtent::FarthestCorner),
        )),
        _ => return Err(format!("invalid radial gradient size: {arguments}").into()),
    };
    let center = match arguments.find(Label::Production(Production::Position)) {
        Some(center) => position(center, relative)?,
        None => [PositionComponent::CENTER; 2],
    };
    Ok(Gradient {
        shape: GradientShape::Radial { ending, center },
        items: stop_list(arguments, relative, color)?,
        repeating,
    })
}

/// `<color-stop-list>`: stops with up to two positions and the hints between them.
fn stop_list(
    syntax: &Component,
    relative: &Relative<'_>,
    current: Color,
) -> Result<Vec<GradientItem>, StyleError> {
    let list = syntax
        .find(Label::Production(Production::ColorStopList))
        .ok_or_else(|| unsupported(syntax))?;
    let mut found = Vec::new();
    list.references(&STOP_LABELS, &mut found);
    let mut items = Vec::new();
    for (label, item) in found {
        if label == STOP_LABELS[1] {
            items.push(GradientItem::Hint(length_percentage(item, relative)?));
            continue;
        }
        let parts = item.items();
        let color = crate::style::colors::color(&parts[0], relative, current)?;
        let positions = match item.find(Label::Production(Production::ColorStopLength)) {
            Some(lengths) => lengths
                .items()
                .iter()
                .map(|position| length_percentage(position, relative).map(Some))
                .collect::<Result<Vec<_>, _>>()?,
            None => vec![None],
        };
        items.extend(
            positions
                .into_iter()
                .map(|position| GradientItem::Stop { color, position }),
        );
    }
    let stops = items
        .iter()
        .filter(|item| matches!(item, GradientItem::Stop { .. }))
        .count();
    if stops < MIN_GRADIENT_STOPS {
        return Err(StyleError::Unsupported(format!(
            "gradient with fewer than {MIN_GRADIENT_STOPS} colour stops: {list}"
        )));
    }
    Ok(items)
}

/// Keywords and offsets in source order, outside nested slots.
fn terms(component: &Component) -> Vec<&Component> {
    match component {
        Component::Group(items) => items.iter().flat_map(terms).collect(),
        Component::Reference(_, inner) => terms(inner),
        other => vec![other],
    }
}

/// One axis: an edge keyword with an optional offset, `center`, or an offset from the start.
fn axis(part: &Component, relative: &Relative<'_>) -> Result<PositionComponent, StyleError> {
    let parts = terms(part);
    let offset = parts
        .iter()
        .find(|term| term.keyword().is_none())
        .map(|offset| length_percentage(offset, relative))
        .transpose()?;
    Ok(match parts.first().and_then(|term| term.keyword()) {
        Some(Keyword::Center) => PositionComponent::CENTER,
        Some(Keyword::Right | Keyword::Bottom) => PositionComponent {
            offset: offset.unwrap_or(LengthPercentage::ZERO),
            ..END
        },
        _ => PositionComponent {
            from_end: false,
            offset: offset.unwrap_or(LengthPercentage::ZERO),
        },
    })
}

/// `<bg-position>` or `<position>`: one value, a horizontal-vertical pair, or keyword groups
/// in either order.
pub(super) fn position(value: &Component, relative: &Relative<'_>) -> Result<Position, StyleError> {
    let value = value.unwrap();
    if let Component::Slots(slots) = value {
        let side = |slot: &Option<Component>| {
            slot.as_ref()
                .map_or(Ok(PositionComponent::CENTER), |part| axis(part, relative))
        };
        return Ok([side(&slots[0])?, side(&slots[1])?]);
    }
    let parts = terms(value);
    match parts.as_slice() {
        [single] if matches!(single.keyword(), Some(Keyword::Top | Keyword::Bottom)) => {
            Ok([PositionComponent::CENTER, axis(single, relative)?])
        }
        [single] => Ok([axis(single, relative)?, PositionComponent::CENTER]),
        [x, y] => Ok([axis(x, relative)?, axis(y, relative)?]),
        _ => Err(unsupported(value)),
    }
}

/// `<bg-size>`: `cover`, `contain`, or a width and an optional height.
fn size(value: &Component, relative: &Relative<'_>) -> Result<BackgroundSize, StyleError> {
    match value.keyword() {
        Some(Keyword::Cover) => return Ok(BackgroundSize::Cover),
        Some(Keyword::Contain) => return Ok(BackgroundSize::Contain),
        _ => {}
    }
    let component = |part: &Component| -> Result<SizeComponent, StyleError> {
        match part.keyword() {
            Some(Keyword::Auto) => Ok(SizeComponent::Auto),
            _ => length_percentage(part, relative).map(SizeComponent::Value),
        }
    };
    let parts = terms(value);
    Ok(BackgroundSize::Explicit([
        component(parts[0])?,
        parts
            .get(1)
            .map_or(Ok(SizeComponent::Auto), |height| component(height))?,
    ]))
}

/// `<repeat-style>`: `repeat-x`, `repeat-y`, or one keyword per axis.
fn repeat(value: &Component) -> Result<[Repeat; 2], StyleError> {
    let keyword = |part: &Component| match part.keyword() {
        Some(Keyword::Repeat) => Ok(Repeat::Repeat),
        Some(Keyword::Space) => Ok(Repeat::Space),
        Some(Keyword::Round) => Ok(Repeat::Round),
        Some(Keyword::NoRepeat) => Ok(Repeat::NoRepeat),
        _ => Err(unsupported(part)),
    };
    match value.keyword() {
        Some(Keyword::RepeatX) => return Ok([Repeat::Repeat, Repeat::NoRepeat]),
        Some(Keyword::RepeatY) => return Ok([Repeat::NoRepeat, Repeat::Repeat]),
        _ => {}
    }
    let parts = terms(value);
    let x = keyword(parts[0])?;
    Ok([x, parts.get(1).map_or(Ok(x), |y| keyword(y))?])
}

/// `<visual-box>`; `background-clip`'s `border-area` and `text` are not modelled.
fn box_area(value: &Component) -> Result<BoxArea, StyleError> {
    match value.keyword() {
        Some(Keyword::BorderBox) => Ok(BoxArea::BorderBox),
        Some(Keyword::PaddingBox) => Ok(BoxArea::PaddingBox),
        Some(Keyword::ContentBox) => Ok(BoxArea::ContentBox),
        _ => Err(unsupported(value)),
    }
}

fn attachment(value: &Component) -> Result<Attachment, StyleError> {
    match value.keyword() {
        Some(Keyword::Scroll) => Ok(Attachment::Scroll),
        Some(Keyword::Fixed) => Ok(Attachment::Fixed),
        Some(Keyword::Local) => Ok(Attachment::Local),
        _ => Err(unsupported(value)),
    }
}
