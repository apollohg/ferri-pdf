//! Background image layers: single tiles are drawn in place and repeated tiles become
//! tiling patterns; gradients are vector axial or radial shadings.
use crate::writer::{
    PdfError,
    images::ImageWriter,
    painting::{self, rect_path},
};
use ferri_pdf_graphics::{
    BorderImagePaint, BoxPaint, ImagePaint, LayerPaint, Rect, ResolvedGradient, Shading, TileImage,
};
use krilla::{
    color::rgb,
    geom::Transform,
    num::NormalizedF32,
    paint::{Fill, FillRule, LinearGradient, Pattern, RadialGradient, SpreadMethod, Stop},
    surface::Surface,
};

pub(super) fn draw(
    paint: &BoxPaint,
    page_area: Rect,
    viewport: Rect,
    surface: &mut Surface<'_>,
    images: &mut ImageWriter,
) -> Result<(), PdfError> {
    let layers = paint
        .background_layers(page_area)
        .map_err(PdfError::UnresolvedImage)?;
    for layer in &layers {
        draw_layer(layer, viewport, surface, images)?;
    }
    Ok(())
}

/// The painting area restricted to the tile's row or column on axes that do not repeat.
fn strip(layer: &LayerPaint<'_>) -> Option<Rect> {
    let area = layer.clip.rect;
    let tile = layer.tile;
    let mut bounds = [
        [area.x, area.x + area.width],
        [area.y, area.y + area.height],
    ];
    for (axis, [start, end]) in [
        [tile.x, tile.x + tile.width],
        [tile.y, tile.y + tile.height],
    ]
    .into_iter()
    .enumerate()
    {
        if !layer.repeat[axis] {
            bounds[axis] = [bounds[axis][0].max(start), bounds[axis][1].min(end)];
        }
    }
    let [[left, right], [top, bottom]] = bounds;
    (right > left && bottom > top).then_some(Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

fn draw_layer(
    layer: &LayerPaint<'_>,
    viewport: Rect,
    surface: &mut Surface<'_>,
    images: &mut ImageWriter,
) -> Result<(), PdfError> {
    if layer.clip.rect.width <= 0.0 || layer.clip.rect.height <= 0.0 {
        return Ok(());
    }
    let clips = painting::push_clip(&layer.clip, viewport, surface)?;
    let result = if layer.repeat.contains(&true) {
        draw_pattern(layer, surface, images)
    } else {
        draw_tile(&layer.image, layer.tile, viewport, surface, images)
    };
    painting::pop_clips(clips, surface);
    result
}

fn draw_pattern(
    layer: &LayerPaint<'_>,
    surface: &mut Surface<'_>,
    images: &mut ImageWriter,
) -> Result<(), PdfError> {
    let Some(area) = strip(layer) else {
        return Ok(());
    };
    let tile = Rect {
        x: 0.0,
        y: 0.0,
        ..layer.tile
    };
    let mut builder = surface.stream_builder();
    let mut tile_surface = builder.surface();
    // A form gives nested shading patterns the tile's default user space.
    let isolated = matches!(layer.image, TileImage::Gradient(_));
    if isolated {
        tile_surface.push_isolated();
    }
    let drawn = draw_tile(&layer.image, tile, tile, &mut tile_surface, images);
    if isolated {
        tile_surface.pop();
    }
    tile_surface.finish();
    let stream = builder.finish();
    drawn?;
    let pattern = Pattern {
        stream,
        transform: Transform::from_translate(layer.tile.x, layer.tile.y),
        width: layer.step[0],
        height: layer.step[1],
    };
    surface.set_fill(Some(Fill {
        paint: pattern.into(),
        ..Default::default()
    }));
    surface.draw_path(&rect_path(area)?);
    Ok(())
}

/// Border image pieces: each tile maps its image slice onto the tile, clipped to the tile
/// within its region, inside the fragment clip of a sliced box.
pub(super) fn draw_border_image(
    paint: &BorderImagePaint<'_>,
    viewport: Rect,
    surface: &mut Surface<'_>,
    images: &mut ImageWriter,
) -> Result<(), PdfError> {
    let fragment = paint.clip.map(rect_path).transpose()?;
    if let Some(path) = &fragment {
        surface.push_clip_path(path, &FillRule::NonZero);
    }
    let image = Rect {
        x: 0.0,
        y: 0.0,
        width: paint.size[0],
        height: paint.size[1],
    };
    let mut result = Ok(());
    for piece in &paint.pieces {
        for tile in &piece.tiles {
            let visible = tile.intersection(piece.area);
            if visible.width <= 0.0 || visible.height <= 0.0 {
                continue;
            }
            let clip = rect_path(visible)?;
            let scale = [
                tile.width / piece.source.width,
                tile.height / piece.source.height,
            ];
            surface.push_clip_path(&clip, &FillRule::NonZero);
            surface.push_transform(&Transform::from_row(
                scale[0],
                0.0,
                0.0,
                scale[1],
                tile.x - piece.source.x * scale[0],
                tile.y - piece.source.y * scale[1],
            ));
            result = draw_tile(&paint.image, image, viewport, surface, images);
            surface.pop();
            surface.pop();
            if result.is_err() {
                break;
            }
        }
    }
    if fragment.is_some() {
        surface.pop();
    }
    result
}

fn draw_tile(
    image: &TileImage<'_>,
    rect: Rect,
    viewport: Rect,
    surface: &mut Surface<'_>,
    images: &mut ImageWriter,
) -> Result<(), PdfError> {
    match image {
        TileImage::Image(image) => images.draw(
            &ImagePaint {
                image: (*image).clone(),
                rect,
                clip: None,
            },
            viewport,
            surface,
        ),
        TileImage::Gradient(gradient) => {
            surface.set_fill(Some(gradient_fill(gradient, [rect.x, rect.y])?));
            surface.draw_path(&rect_path(rect)?);
            Ok(())
        }
    }
}

fn normalized(value: f32) -> Result<NormalizedF32, PdfError> {
    NormalizedF32::new(value.clamp(0.0, 1.0)).ok_or(PdfError::InvalidGeometry)
}

fn gradient_fill(gradient: &ResolvedGradient, [x, y]: [f32; 2]) -> Result<Fill, PdfError> {
    if let [only] = gradient.stops.as_slice() {
        return painting::fill(only.color);
    }
    let stops = gradient
        .stops
        .iter()
        .map(|stop| {
            Ok(Stop {
                offset: normalized(stop.offset)?,
                color: rgb::Color::new(stop.color.rgb[0], stop.color.rgb[1], stop.color.rgb[2])
                    .into(),
                opacity: NormalizedF32::new(stop.color.alpha)
                    .ok_or(PdfError::InvalidAlpha(stop.color.alpha))?,
            })
        })
        .collect::<Result<Vec<_>, PdfError>>()?;
    let paint = match gradient.shading {
        Shading::Axial { start, end } => LinearGradient {
            x1: x + start[0],
            y1: y + start[1],
            x2: x + end[0],
            y2: y + end[1],
            transform: Transform::identity(),
            spread_method: SpreadMethod::Pad,
            stops,
            anti_alias: false,
        }
        .into(),
        Shading::Radial {
            center,
            radii,
            scale_y,
        } => {
            let center = [x + center[0], y + center[1]];
            RadialGradient {
                fx: center[0],
                fy: center[1],
                fr: radii[0],
                cx: center[0],
                cy: center[1],
                cr: radii[1],
                transform: Transform::from_row(
                    1.0,
                    0.0,
                    0.0,
                    scale_y,
                    0.0,
                    center[1] * (1.0 - scale_y),
                ),
                spread_method: SpreadMethod::Pad,
                stops,
                anti_alias: false,
            }
            .into()
        }
    };
    Ok(Fill {
        paint,
        ..Default::default()
    })
}
