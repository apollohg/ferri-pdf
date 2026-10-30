//! CSS Backgrounds 3 longhands and CSS Images 4 images and gradients from match trees.
use crate::paint::{
    AbsoluteColor, Attachment, BackgroundImage, BackgroundSize, BoxArea, CircleSize, EllipseSize,
    Gradient, GradientItem, GradientShape, ImageFunction, Interpolation, LengthPercentage,
    LineDirection, Position, PositionComponent, RadialEnding, RadialExtent, Repeat, SizeComponent,
};
use crate::style::{
    StyleError,
    box_values::{length_percentage, unsupported},
    calc::Kind,
    component::Component,
    grammar::{Keyword, Label, Production},
    units::Relative,
};

const TO_TOP_DEG: f32 = 0.0;
const TO_RIGHT_DEG: f32 = 90.0;
const TO_BOTTOM_DEG: f32 = 180.0;
const TO_LEFT_DEG: f32 = 270.0;
/// `[ <angle> | <zero> | to <side-or-corner> ] || <color-interpolation-method>` slots.
const DIRECTION_SLOT: usize = 0;
/// `<side-or-corner>` slots.
const HORIZONTAL_SLOT: usize = 0;
const VERTICAL_SLOT: usize = 1;
const STOP_LABELS: [Label; 2] = [
    Label::Production(Production::LinearColorStop),
    Label::Production(Production::LinearColorHint),
];
const ANGULAR_STOP_LABELS: [Label; 2] = [
    Label::Production(Production::AngularColorStop),
    Label::Production(Production::AngularColorHint),
];
const FULL_TURN_DEG: f32 = 360.0;
/// `<resolution> || type(<string>)` slots of an `<image-set-option>`.
const RESOLUTION_SLOT: usize = 0;
const TYPE_SLOT: usize = 1;
/// An `<image-set-option>` without a `<resolution>` selects as `1x` (CSS Images 4 §2.4).
const DEFAULT_RESOLUTION_DPPX: f32 = 1.0;
/// `type()` MIME type essences of the image formats the engine decodes.
const SUPPORTED_IMAGE_TYPES: [&str; 8] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/heic",
    "image/heif",
    "image/svg+xml",
];
const MIME_PARAMETER_SEPARATOR: char = ';';
const MIME_SUBTYPE_SEPARATOR: char = '/';

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
    color: &AbsoluteColor,
) -> Result<Vec<BackgroundImage>, StyleError> {
    list(value, |item| image(item, relative, color))
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

/// `<bg-image>`: `none`, a URL, `image-set()`, or a gradient.
pub(super) fn image(
    value: &Component,
    relative: &Relative<'_>,
    color: &AbsoluteColor,
) -> Result<BackgroundImage, StyleError> {
    match value.unwrap() {
        Component::Function(Keyword::ImageSet, Some(options)) => {
            image_set(options, relative, color)
        }
        _ => set_option_image(value, relative, color),
    }
}

/// An `<image>` other than `image-set()`, which cannot nest inside itself.
fn set_option_image(
    value: &Component,
    relative: &Relative<'_>,
    color: &AbsoluteColor,
) -> Result<BackgroundImage, StyleError> {
    Ok(match value.unwrap() {
        Component::Keyword(Keyword::None) => BackgroundImage::None,
        Component::Url(url) | Component::String(url) => url_image(url, None),
        Component::Function(Keyword::Image, Some(arguments)) => {
            image_function(arguments, relative, color)?
        }
        Component::Function(Keyword::ImageSet, _) => {
            return Err(StyleError::Invalid(format!(
                "image-set() nested in image-set(): {value}"
            )));
        }
        // Without script no paint worklet registers a name, so drawing the image finds no
        // document paint definition and outputs an invalid image (CSS Painting API 1 §7).
        Component::Function(Keyword::Paint, _) => BackgroundImage::Invalid,
        Component::Function(name, Some(arguments)) => {
            let repeating = matches!(
                name,
                Keyword::RepeatingLinearGradient
                    | Keyword::RepeatingRadialGradient
                    | Keyword::RepeatingConicGradient
            );
            BackgroundImage::Gradient(match name {
                Keyword::LinearGradient | Keyword::RepeatingLinearGradient => {
                    linear(arguments, relative, color, repeating)?
                }
                Keyword::RadialGradient | Keyword::RepeatingRadialGradient => {
                    radial(arguments, relative, color, repeating)?
                }
                Keyword::ConicGradient | Keyword::RepeatingConicGradient => {
                    conic(arguments, relative, color, repeating)?
                }
                _ => return Err(unsupported(value)),
            })
        }
        other => return Err(unsupported(other)),
    })
}

fn url_image(url: &str, function: Option<ImageFunction>) -> BackgroundImage {
    BackgroundImage::Url {
        url: url.to_owned(),
        resolution: None,
        scale: crate::paint::INITIAL_IMAGE_SCALE,
        function,
    }
}

/// CSS Images 4 §2.5 `image()`: a URL, which a string also writes, with its directionality
/// and fallback color; or a solid-color image.
fn image_function(
    arguments: &Component,
    relative: &Relative<'_>,
    current: &AbsoluteColor,
) -> Result<BackgroundImage, StyleError> {
    let rtl = arguments
        .find(Label::Production(Production::ImageTags))
        .and_then(Component::keyword)
        .map(|tag| tag == Keyword::Rtl);
    // `[ <image-src>? , <color>? ]!`: the color is the member that is not the source.
    let fallback = arguments
        .items()
        .last()
        .into_iter()
        .flat_map(Component::items)
        .flat_map(Component::items)
        .find(|item| {
            !matches!(
                item,
                Component::Delim(_)
                    | Component::Reference(Label::Production(Production::ImageSrc), _)
            )
        })
        .map(|color| crate::style::colors::absolute(color, relative, current))
        .transpose()?;
    Ok(
        match arguments
            .find(Label::Production(Production::ImageSrc))
            .map(Component::unwrap)
        {
            Some(Component::Url(url) | Component::String(url)) => {
                url_image(url, Some(ImageFunction { rtl, fallback }))
            }
            Some(other) => return Err(unsupported(other)),
            None => BackgroundImage::Color(fallback.ok_or_else(|| unsupported(arguments))?),
        },
    )
}

/// CSS Images 4 §2.4: options with an unsupported `type()` or an already listed resolution
/// drop out; for resolution-independent PDF output the highest resolution is chosen, and it
/// becomes a raster image's natural resolution. No remaining option is an invalid image,
/// which paints nothing, as `none` does.
fn image_set(
    options: &Component,
    relative: &Relative<'_>,
    color: &AbsoluteColor,
) -> Result<BackgroundImage, StyleError> {
    let mut candidates: Vec<(&Component, f32)> = Vec::new();
    for option in options.items() {
        let parts = option.items();
        let image = parts.first().ok_or_else(|| unsupported(option))?;
        // `[ <resolution> || type(<string>) ]?` matches as a group of at most one.
        let modifiers = parts.get(1).and_then(|optional| optional.items().first());
        if let Some(mime) = modifiers.and_then(|modifiers| modifiers.slot(TYPE_SLOT))
            && !mime.arguments().first().is_some_and(supported_image_type)
        {
            continue;
        }
        let resolution = match modifiers.and_then(|modifiers| modifiers.slot(RESOLUTION_SLOT)) {
            Some(resolution) => crate::style::values::resolution(resolution, relative)?,
            None => DEFAULT_RESOLUTION_DPPX,
        };
        if resolution <= 0.0 {
            return Err(StyleError::Invalid(format!(
                "non-positive image-set() resolution: {option}"
            )));
        }
        if candidates.iter().all(|(_, listed)| *listed != resolution) {
            candidates.push((image, resolution));
        }
    }
    let Some((image, resolution)) = candidates.into_iter().max_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return Ok(BackgroundImage::None);
    };
    Ok(match set_option_image(image, relative, color)? {
        BackgroundImage::Url {
            url,
            scale,
            function,
            ..
        } => BackgroundImage::Url {
            url,
            resolution: Some(resolution),
            scale,
            function,
        },
        other => other,
    })
}

/// Whether a `type()` string is a valid MIME type whose essence names a decoded format.
fn supported_image_type(mime: &Component) -> bool {
    let Component::String(mime) = mime.unwrap() else {
        return false;
    };
    let essence = mime
        .split(MIME_PARAMETER_SEPARATOR)
        .next()
        .unwrap_or_default()
        .trim();
    essence.split_once(MIME_SUBTYPE_SEPARATOR).is_some()
        && SUPPORTED_IMAGE_TYPES
            .iter()
            .any(|supported| essence.eq_ignore_ascii_case(supported))
}

/// The `<color-interpolation-method>` of a gradient, or the default for its stops.
fn interpolation(
    arguments: &Component,
    items: &[GradientItem],
) -> Result<Interpolation, StyleError> {
    let Some(method) = arguments.find(Label::Production(Production::ColorInterpolationMethod))
    else {
        return Ok(Interpolation::default_for(items.iter().filter_map(
            |item| match item {
                GradientItem::Stop { color, .. } => Some(color),
                GradientItem::Hint(_) => None,
            },
        )));
    };
    interpolation_method(method)
}

/// A matched `<color-interpolation-method>`.
pub(super) fn interpolation_method(method: &Component) -> Result<Interpolation, StyleError> {
    let mut keywords = Vec::new();
    method.keywords(&mut keywords);
    let words: Vec<&str> = keywords
        .iter()
        .filter(|keyword| **keyword != Keyword::In)
        .map(|keyword| keyword.name())
        .collect();
    crate::style::colors::interpolation_method(&words).ok_or_else(|| unsupported(method))
}

fn linear(
    arguments: &Component,
    relative: &Relative<'_>,
    color: &AbsoluteColor,
    repeating: bool,
) -> Result<Gradient, StyleError> {
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
    let items = stop_list(syntax, relative, color, StopList::Linear)?;
    Ok(Gradient {
        shape: GradientShape::Linear(direction),
        interpolation: interpolation(arguments, &items)?,
        items,
        repeating,
    })
}

fn radial(
    arguments: &Component,
    relative: &Relative<'_>,
    color: &AbsoluteColor,
    repeating: bool,
) -> Result<Gradient, StyleError> {
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
    let items = stop_list(arguments, relative, color, StopList::Linear)?;
    Ok(Gradient {
        shape: GradientShape::Radial { ending, center },
        interpolation: interpolation(arguments, &items)?,
        items,
        repeating,
    })
}

/// CSS Images 4 §3.3: `from` rotates the gradient and `at` places its centre.
fn conic(
    arguments: &Component,
    relative: &Relative<'_>,
    color: &AbsoluteColor,
    repeating: bool,
) -> Result<Gradient, StyleError> {
    let syntax = arguments
        .find(Label::Production(Production::ConicGradientSyntax))
        .ok_or_else(|| unsupported(arguments))?;
    let from = match after_keyword(syntax, Keyword::From) {
        Some(angle) => crate::style::values::angle(angle, relative)?,
        None => 0.0,
    };
    let center = match syntax.find(Label::Production(Production::Position)) {
        Some(center) => position(center, relative)?,
        None => [PositionComponent::CENTER; 2],
    };
    let items = stop_list(syntax, relative, color, StopList::Angular)?;
    Ok(Gradient {
        shape: GradientShape::Conic { from, center },
        interpolation: interpolation(arguments, &items)?,
        items,
        repeating,
    })
}

/// The term after `keyword` in a juxtaposed group, outside function arguments.
fn after_keyword(component: &Component, keyword: Keyword) -> Option<&Component> {
    match component.unwrap() {
        Component::Group(items) => items
            .windows(2)
            .find(|pair| pair[0].keyword() == Some(keyword))
            .map(|pair| &pair[1])
            .or_else(|| items.iter().find_map(|item| after_keyword(item, keyword))),
        Component::Slots(slots) => slots
            .iter()
            .flatten()
            .find_map(|slot| after_keyword(slot, keyword)),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum StopList {
    /// `<color-stop-list>`: lengths and percentages along the gradient line.
    Linear,
    /// `<angular-color-stop-list>`: angles and percentages of a turn, kept as fractions.
    Angular,
}

/// An `<angle-percentage>` or `<zero>` as a fraction of a full turn.
fn turn_fraction(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    match value.unwrap() {
        Component::Percentage(fraction) => Ok(*fraction),
        Component::Math(source, Kind::Angle, range) => {
            Ok(
                crate::style::values::math(source, relative, Kind::Angle, Some(Kind::Angle))?
                    .resolve(FULL_TURN_DEG, *range)
                    / FULL_TURN_DEG,
            )
        }
        _ => Ok(crate::style::values::angle(value, relative)? / FULL_TURN_DEG),
    }
}

/// A colour stop list: stops with up to two positions and the hints between them.
fn stop_list(
    syntax: &Component,
    relative: &Relative<'_>,
    current: &AbsoluteColor,
    kind: StopList,
) -> Result<Vec<GradientItem>, StyleError> {
    let (list_label, labels, positions_label) = match kind {
        StopList::Linear => (
            Production::ColorStopList,
            STOP_LABELS,
            Production::ColorStopLength,
        ),
        StopList::Angular => (
            Production::AngularColorStopList,
            ANGULAR_STOP_LABELS,
            Production::ColorStopAngle,
        ),
    };
    let list = syntax
        .find(Label::Production(list_label))
        .ok_or_else(|| unsupported(syntax))?;
    let position = |value: &Component| match kind {
        StopList::Linear => length_percentage(value, relative),
        StopList::Angular => turn_fraction(value, relative).map(LengthPercentage::Percentage),
    };
    let mut found = Vec::new();
    list.references(&labels, &mut found);
    let mut items = Vec::new();
    for (label, item) in found {
        if label == labels[1] {
            items.push(GradientItem::Hint(position(item)?));
            continue;
        }
        let parts = item.items();
        let color = crate::style::colors::absolute(&parts[0], relative, current)?;
        let positions = match item.find(Label::Production(positions_label)) {
            Some(lengths) => lengths
                .items()
                .iter()
                .map(|value| position(value).map(Some))
                .collect::<Result<Vec<_>, _>>()?,
            None => vec![None],
        };
        items.extend(
            positions
                .into_iter()
                .map(|position| GradientItem::Stop { color, position }),
        );
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
/// The flow-relative edges of CSS Backgrounds 4 compute to the physical edge they name in the
/// element's horizontal writing mode, where x is the inline axis and y the block axis.
fn axis(
    part: &Component,
    relative: &Relative<'_>,
    rtl: bool,
) -> Result<PositionComponent, StyleError> {
    let parts = terms(part);
    let from_end = match parts.first().and_then(|term| term.keyword()) {
        Some(Keyword::Center) => return Ok(PositionComponent::CENTER),
        Some(Keyword::XStart) => rtl,
        Some(Keyword::XEnd) => !rtl,
        Some(Keyword::Right | Keyword::Bottom | Keyword::YEnd) => true,
        _ => false,
    };
    let offset = parts
        .iter()
        .find(|term| term.keyword().is_none())
        .map(|offset| length_percentage(offset, relative))
        .transpose()?
        .unwrap_or(LengthPercentage::ZERO);
    Ok(PositionComponent { from_end, offset })
}

/// `<bg-position>` or `<position>`: one value, a horizontal-vertical pair, or keyword groups
/// in either order.
pub(super) fn position(value: &Component, relative: &Relative<'_>) -> Result<Position, StyleError> {
    let [x, y] = position_axes(value)?;
    let side = |part: Option<&Component>| {
        part.map_or(Ok(PositionComponent::CENTER), |part| {
            axis(part, relative, false)
        })
    };
    Ok([side(x)?, side(y)?])
}

/// The horizontal and vertical parts of a `<bg-position>` or `<position>`; `None` is `center`.
pub(super) fn position_axes(value: &Component) -> Result<[Option<&Component>; 2], StyleError> {
    let value = value.unwrap();
    if let Component::Slots(slots) = value {
        return Ok([slots[0].as_ref(), slots[1].as_ref()]);
    }
    let parts = terms(value);
    match parts.as_slice() {
        [single] if matches!(single.keyword(), Some(Keyword::Top | Keyword::Bottom)) => {
            Ok([None, Some(single)])
        }
        [single] => Ok([Some(single), None]),
        [x, y] => Ok([Some(x), Some(y)]),
        _ => Err(unsupported(value)),
    }
}

/// `background-position-x` or `-y` (CSS Backgrounds 4): per layer, `center` or an edge with
/// an offset from it.
pub(super) fn axis_positions(
    value: &Component,
    relative: &Relative<'_>,
    rtl: bool,
) -> Result<Vec<PositionComponent>, StyleError> {
    list(value, |item| axis(item, relative, rtl))
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
pub(super) fn box_area(value: &Component) -> Result<BoxArea, StyleError> {
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
