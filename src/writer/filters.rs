//! Filter Effects 1 §5 `filter`. Chains of colour matrices that PDF paints exactly stay
//! vector: `grayscale()`, `invert()`, `opacity()`, and `brightness()`, `contrast()` and
//! `saturate()` up to 100%, recolour the group's paints. Every other chain, and any group
//! with paints a matrix cannot recolour, renders to pixels in the group's own coordinates;
//! each function or referenced `filter` element then runs on them as its §13.1 equivalent
//! primitive through the SVG renderer's filter implementation, and the result draws in place:
//! `blur()`, `drop-shadow()`, `sepia()` and `hue-rotate()` other than zero, and
//! `brightness()`, `contrast()` and `saturate()` above 100%, whose results leave the
//! channel range and are clamped per pixel.
use crate::graphics::{FilterFunction, FilterPaint, POINTS_PER_PIXEL, Rect};
use crate::paint::{
    ColorMatrix as ColorMatrixValues, IDENTITY_MATRIX, alpha_scale, compose, preserves_unit_range,
    transform_color,
};
use crate::svg::usvg::filter::{
    ColorInterpolation, ColorMatrix, ColorMatrixKind, DropShadow, Filter, GaussianBlur, Input,
    Kind, Primitive,
};
use crate::writer::{
    PdfError,
    display::{Painter, Space},
    painting, raster,
};
use krilla::surface::Surface;
use std::sync::Arc;

/// The primitive result name of single-primitive function filters.
const RESULT: &str = "result";
/// Pixels a lighting primitive's surface normal kernel reads around each pixel.
const LIGHTING_KERNEL_RADIUS: f32 = 1.0;

/// One entry of the filter chain, applied to the previous entry's output.
pub(super) enum Step<'f> {
    Function(&'f FilterFunction),
    /// A referenced `filter` element; its user units are CSS pixels.
    Reference(Arc<Filter>),
}

pub(super) fn draw(
    painter: &mut Painter<'_, '_, '_>,
    filter: &FilterPaint,
    items: &[crate::graphics::DisplayItem],
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    if filter.bounds.is_none()
        && filter
            .functions
            .iter()
            .any(|function| matches!(function, FilterFunction::Url(_)))
    {
        return Err(PdfError::UnplacedEffect);
    }
    // §5: a reference to anything but a filter element ignores the whole chain.
    let Some(steps) = steps(filter, painter.raster.definitions) else {
        return painter.draw(items, space, surface);
    };
    if let Some((matrix, alpha)) = color_chain(&steps)
        && let Some((items, lines)) = recolor(painter, items, &matrix)?
    {
        return draw_recolored(painter, &items, &lines, alpha, space, surface);
    }
    let visible = painting::local_bounds(space.viewport, surface)?;
    let density = painter.raster.pixels_per_point
        * space.scale
        * painting::transform_scale(surface.cur_transform());
    // §13.2: the region covers the visible content and the ink the chain moves into it.
    let spread: f32 = steps.iter().map(|step| step.spread(density)).sum();
    let area = expand(visible, spread);
    let Some(raster) = raster::rasterize(painter, items, area, density)? else {
        return Ok(());
    };
    // Only pixels within the chain's reach of the ink, or inside a referenced filter's region
    // where its primitives may generate content, can change.
    let reach = raster.ink().map(|ink| expand(ink, spread));
    let reach = steps
        .iter()
        .filter_map(Step::region)
        .fold(reach, |reach, region| {
            Some(reach.map_or(region, |reach| reach.union(region)))
        });
    let Some(mut raster) = reach.and_then(|reach| raster.cropped(reach)) else {
        return Ok(());
    };
    apply(painter, &steps, &mut raster)?;
    match raster
        .ink()
        .and_then(|ink| raster.cropped(ink.intersection(visible)))
    {
        Some(raster) => raster::embed(&raster, surface),
        None => Ok(()),
    }
}

/// Runs the chain on `raster`'s pixels in place; `painter` draws the CSS mask layers of SVG
/// content that referenced filters' images show.
pub(super) fn apply(
    painter: &mut Painter<'_, '_, '_>,
    steps: &[Step<'_>],
    raster: &mut raster::Raster,
) -> Result<(), PdfError> {
    let transform = raster.transform();
    let layer_masks = crate::writer::masks::SvgLayerMasks::new(painter, raster.density, &[]);
    let applied = steps.iter().try_for_each(|step| {
        match step {
            Step::Function(function) => {
                let filter = function_filter(function, raster.area)?;
                crate::svg::resvg::filter::apply(&filter, transform, &mut raster.pixmap, None);
            }
            Step::Reference(filter) => crate::svg::resvg::filter::apply(
                filter,
                transform.pre_scale(POINTS_PER_PIXEL, POINTS_PER_PIXEL),
                &mut raster.pixmap,
                Some(&layer_masks),
            ),
        }
        Ok(())
    });
    layer_masks.finish()?;
    applied
}

pub(super) fn expand(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x - by,
        y: rect.y - by,
        width: rect.width + 2.0 * by,
        height: rect.height + 2.0 * by,
    }
}

fn scaled(rect: crate::svg::usvg::NonZeroRect, factor: f32) -> Rect {
    Rect {
        x: rect.x() * factor,
        y: rect.y() * factor,
        width: rect.width() * factor,
        height: rect.height() * factor,
    }
}

/// A chain of colour-matrix functions that each keep every channel in `0..=1` and leave
/// alpha to a constant factor, composed, with that factor. Such a chain commutes with the
/// group's compositing, so it applies exactly to each paint's colour and, for alpha, to the
/// group as a constant opacity: the result stays vector, text included.
pub(super) fn color_chain(steps: &[Step<'_>]) -> Option<(ColorMatrixValues, f32)> {
    let matrix = steps
        .iter()
        .try_fold(IDENTITY_MATRIX, |chain, step| match step {
            Step::Function(function) => function
                .matrix()
                .filter(preserves_unit_range)
                .map(|matrix| compose(&chain, &matrix)),
            Step::Reference(_) => None,
        })?;
    alpha_scale(&matrix).map(|alpha| (matrix, alpha))
}

fn draw_recolored(
    painter: &mut Painter<'_, '_, '_>,
    items: &[crate::graphics::DisplayItem],
    lines: &[crate::layout::TextLine],
    alpha: f32,
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let opacity = krilla::num::NormalizedF32::new(alpha).ok_or(PdfError::InvalidAlpha(alpha))?;
    let translucent = opacity != krilla::num::NormalizedF32::ONE;
    if translucent {
        surface.push_opacity(opacity);
    }
    let mut recolored = Painter {
        lines,
        page: painter.page,
        page_area: painter.page_area,
        text: painter.text,
        fonts: painter.fonts,
        images: painter.images,
        raster: painter.raster,
        prepared: crate::writer::backdrops::Prepared::default(),
    };
    let drawn = if translucent {
        let nested = Space {
            viewport: painting::local_bounds(space.viewport, surface)?,
            scale: space.scale * painting::transform_scale(surface.cur_transform()),
        };
        recolored.draw(items, nested, surface)
    } else {
        recolored.draw(items, space, surface)
    };
    if translucent {
        surface.pop();
    }
    drawn
}

/// Display items and the text lines they draw.
type Recolored = (
    Vec<crate::graphics::DisplayItem>,
    Vec<crate::layout::TextLine>,
);

/// `items` and the text lines they draw with every colour mapped by `matrix`, or `None` when
/// something paints colours the matrix cannot reach exactly: images, gradients and border
/// images, three-dimensional border styles, whose shades scale the mapped colour, blending,
/// nested filters, backdrops, which read unmapped colours, and bitmap or SVG colour glyphs.
fn recolor(
    painter: &Painter<'_, '_, '_>,
    items: &[crate::graphics::DisplayItem],
    matrix: &ColorMatrixValues,
) -> Result<Option<Recolored>, PdfError> {
    use crate::graphics::DisplayItem;
    let map = |color: crate::graphics::Color| map_color(matrix, color);
    let mut lines = Vec::new();
    let mut mapped = Vec::with_capacity(items.len());
    for item in items {
        mapped.push(match item {
            DisplayItem::Box(paint) => match recolor_box(paint, &map) {
                Some(paint) => DisplayItem::Box(paint),
                None => return Ok(None),
            },
            DisplayItem::Text { line } => {
                let source = painter
                    .lines
                    .get(*line)
                    .ok_or(PdfError::InvalidTextReference {
                        page: painter.page,
                        line: *line,
                    })?;
                if !painter.text.paints_run_colors(&source.runs)? {
                    return Ok(None);
                }
                let mut line = source.clone();
                line.color = map(line.color);
                recolor_runs(&mut line.runs, &map);
                lines.push(line);
                DisplayItem::Text {
                    line: lines.len() - 1,
                }
            }
            DisplayItem::Decoration(decoration) => {
                let mut decoration = *decoration;
                decoration.color = map(decoration.color);
                DisplayItem::Decoration(decoration)
            }
            // A text shadow paints only its colour through the runs' silhouettes.
            DisplayItem::TextShadow(shadow) => {
                let mut shadow = shadow.clone();
                shadow.color = map(shadow.color);
                DisplayItem::TextShadow(shadow)
            }
            item @ (DisplayItem::PushClip(_)
            | DisplayItem::PushTransform(_)
            | DisplayItem::PushProjection(_)
            | DisplayItem::PushOpacity(_)
            | DisplayItem::Pop) => item.clone(),
            item @ DisplayItem::PushBackdropRoot => item.clone(),
            DisplayItem::Frame(_)
            | DisplayItem::Image(_)
            | DisplayItem::Backdrop(_)
            | DisplayItem::PushBlend(_)
            | DisplayItem::PushFilter(_)
            | DisplayItem::PushMask(_) => {
                return Ok(None);
            }
        });
    }
    Ok(Some((mapped, lines)))
}

/// Glyph fills, strokes, palettes and emphasis marks of `runs` mapped by `map`.
fn recolor_runs(
    runs: &mut [crate::text::GlyphRun],
    map: &impl Fn(crate::graphics::Color) -> crate::graphics::Color,
) {
    for run in runs {
        run.color = map(run.color);
        if let Some(palette) = &mut run.palette {
            *palette = palette.iter().copied().map(map).collect();
        }
        if let Some(stroke) = &mut run.stroke {
            stroke.color = map(stroke.color);
        }
        for mark in &mut run.mark_shapes {
            mark.color = map(mark.color);
        }
        recolor_runs(&mut run.marks, map);
    }
}

fn recolor_box(
    paint: &crate::graphics::BoxPaint,
    map: &impl Fn(crate::graphics::Color) -> crate::graphics::Color,
) -> Option<crate::graphics::BoxPaint> {
    use crate::graphics::{BorderStyle, OutlineStyle};
    let shaded = |style: BorderStyle| {
        matches!(
            style,
            BorderStyle::Groove | BorderStyle::Ridge | BorderStyle::Inset | BorderStyle::Outset
        )
    };
    let decoration = &paint.decoration;
    if decoration.backgrounds.has_images()
        || decoration.border_image.is_drawn()
        || decoration.borders.iter().any(|border| shaded(border.style))
        || matches!(decoration.outline.style, OutlineStyle::Line(style) if shaded(style))
    {
        return None;
    }
    let mut paint = paint.clone();
    let decoration = &mut paint.decoration;
    decoration.background = map(decoration.background);
    for border in &mut decoration.borders {
        border.color = map(border.color);
    }
    decoration.outline.color = map(decoration.outline.color);
    decoration.shadows = Arc::new(
        decoration
            .shadows
            .iter()
            .map(|shadow| crate::paint::BoxShadow {
                color: map(shadow.color),
                ..*shadow
            })
            .collect(),
    );
    Some(paint)
}

/// A colour's channels mapped by `matrix`; its alpha is left to the group.
pub(super) fn map_color(
    matrix: &ColorMatrixValues,
    color: crate::graphics::Color,
) -> crate::graphics::Color {
    const CHANNEL_MAX: f32 = 255.0;
    let [red, green, blue] = color.rgb.map(|channel| f32::from(channel) / CHANNEL_MAX);
    let mapped = transform_color(matrix, [red, green, blue, color.alpha]);
    crate::graphics::Color {
        rgb: [mapped[0], mapped[1], mapped[2]]
            .map(|channel| (channel.clamp(0.0, 1.0) * CHANNEL_MAX).round() as u8),
        alpha: color.alpha,
    }
}

/// The chain's steps, or `None` when a reference does not name a usable filter element.
pub(super) fn steps<'f>(
    filter: &'f FilterPaint,
    definitions: &crate::document::svg::SvgDefinitions,
) -> Option<Vec<Step<'f>>> {
    filter
        .functions
        .iter()
        .map(|function| match function {
            FilterFunction::Url(reference) => {
                crate::svg::referenced_filter(reference, filter.bounds, definitions)
                    .map(Step::Reference)
            }
            function => Some(Step::Function(function)),
        })
        .collect()
}

impl Step<'_> {
    /// How far the step moves ink, in drawing units, at `density` pixels per unit.
    /// The region of a referenced filter element, where its primitives may generate content.
    pub(super) fn region(&self) -> Option<Rect> {
        match self {
            Self::Reference(filter) => Some(scaled(filter.rect(), POINTS_PER_PIXEL)),
            Self::Function(_) => None,
        }
    }
    pub(super) fn spread(&self, density: f32) -> f32 {
        match self {
            Self::Function(function) => function.spread(),
            Self::Reference(filter) => filter
                .primitives()
                .iter()
                .map(|primitive| primitive_spread(primitive, filter.rect(), density))
                .sum::<f32>(),
        }
    }
}

/// A primitive's reach in drawing units; its own units are CSS pixels, and kernels that
/// address whole pixels reach as far at `density`.
fn primitive_spread(
    primitive: &Primitive,
    region: crate::svg::usvg::NonZeroRect,
    density: f32,
) -> f32 {
    let pixel = 1.0 / density;
    let blur = |x: strict_num::PositiveF32, y: strict_num::PositiveF32| {
        crate::paint::BLUR_EXTENT * x.get().max(y.get()) * POINTS_PER_PIXEL
    };
    match primitive.kind() {
        Kind::GaussianBlur(blur_kind) => blur(blur_kind.std_dev_x, blur_kind.std_dev_y),
        Kind::DropShadow(shadow) => {
            shadow.dx.abs().max(shadow.dy.abs()) * POINTS_PER_PIXEL
                + blur(shadow.std_dev_x, shadow.std_dev_y)
        }
        Kind::Offset(offset) => offset.dx.abs().max(offset.dy.abs()) * POINTS_PER_PIXEL,
        Kind::Morphology(morphology) => {
            morphology.radius_x.get().max(morphology.radius_y.get()) * POINTS_PER_PIXEL
        }
        Kind::DisplacementMap(map) => map.scale.abs() / 2.0 * POINTS_PER_PIXEL,
        Kind::ConvolveMatrix(convolve) => {
            convolve.matrix.columns.max(convolve.matrix.rows) as f32 * pixel
        }
        Kind::DiffuseLighting(_) | Kind::SpecularLighting(_) => LIGHTING_KERNEL_RADIUS * pixel,
        // Tiling repeats the input anywhere in the filter region.
        Kind::Tile(_) => region.width().max(region.height()) * POINTS_PER_PIXEL,
        Kind::Blend(_)
        | Kind::ColorMatrix(_)
        | Kind::ComponentTransfer(_)
        | Kind::Composite(_)
        | Kind::Flood(_)
        | Kind::Image(_)
        | Kind::Merge(_)
        | Kind::Turbulence(_) => 0.0,
    }
}

/// §13.1: a function as its one-primitive filter over `area`, operating in sRGB (§5).
pub(super) fn function_filter(function: &FilterFunction, area: Rect) -> Result<Filter, PdfError> {
    let rect = crate::svg::usvg::NonZeroRect::from_xywh(area.x, area.y, area.width, area.height)
        .ok_or(PdfError::InvalidGeometry)?;
    let deviation =
        |value: f32| strict_num::PositiveF32::new(value).ok_or(PdfError::InvalidGeometry);
    let kind = match *function {
        FilterFunction::Blur(value) => Kind::GaussianBlur(GaussianBlur {
            input: Input::SourceGraphic,
            std_dev_x: deviation(value)?,
            std_dev_y: deviation(value)?,
        }),
        FilterFunction::DropShadow {
            offset: [dx, dy],
            deviation: value,
            color,
        } => Kind::DropShadow(DropShadow {
            input: Input::SourceGraphic,
            dx,
            dy,
            std_dev_x: deviation(value)?,
            std_dev_y: deviation(value)?,
            color: crate::svg::usvg::Color::new_rgb(color.rgb[0], color.rgb[1], color.rgb[2]),
            opacity: crate::svg::usvg::Opacity::new_clamped(color.alpha),
        }),
        ref function => Kind::ColorMatrix(ColorMatrix {
            input: Input::SourceGraphic,
            kind: ColorMatrixKind::Matrix(
                function.matrix().ok_or(PdfError::InvalidGeometry)?.to_vec(),
            ),
        }),
    };
    Ok(Filter {
        id: crate::svg::usvg::NonEmptyString::new(RESULT.into())
            .ok_or(PdfError::InvalidGeometry)?,
        rect,
        primitives: vec![Primitive {
            rect,
            color_interpolation: ColorInterpolation::SRGB,
            result: RESULT.into(),
            kind,
        }],
    })
}
