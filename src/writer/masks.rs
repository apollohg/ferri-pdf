//! CSS Masking 1 §7 mask layers and the §8 mask border, each as a luminosity soft mask whose
//! grey is the mask value.
//! Each layer draws, over opaque black, its paints recoloured to their mask values: white
//! keeping alpha in `alpha` mode and luminance keeping alpha in `luminance` mode, so black
//! shows through as `value · alpha` exactly (§7.10.1). Images are first resolved to mask
//! values in pixels; references to SVG `mask` elements render through the SVG renderer's mask
//! implementation. Layers combine bottom-most first by separable blend modes, which on opaque
//! grey compute the compositing operators exactly (§7.8): `add` is source-over, B + A − AB,
//! which `Screen` gives; `intersect`, A·B, is `Multiply`; `exclude`, A + B − 2AB, is
//! `Exclusion`; and `subtract`, B(1 − A), is `Multiply` after `Difference` with white turns A
//! into 1 − A.
use crate::graphics::{
    BlendMode, BoxPaint, Clip, Color, DisplayItem, ImagePaint, LayerPaint, MaskComposite, MaskMode,
    MaskPaint, Radii, Rect, ResolvedGradient, TileImage,
};
use crate::images::{Image, PixelDepth, PixelImage};
use crate::writer::{
    PdfError,
    display::{Painter, Space},
    painting, raster,
};
use krilla::{
    mask::{Mask, MaskType},
    surface::Surface,
};
use std::sync::Arc;

/// Filter Effects 1 §9.6 `luminanceToAlpha` coefficients, which §7.10.1 applies to sRGB.
const LUMINANCE: [f32; 3] = [0.2125, 0.7154, 0.0721];
const CHANNEL_MAX: f32 = 255.0;
const WHITE: Color = Color {
    rgb: [u8::MAX; 3],
    alpha: 1.0,
};

pub(super) fn draw(
    painter: &mut Painter<'_, '_, '_>,
    mask: &MaskPaint,
    items: &[DisplayItem],
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    if mask.fragments.is_empty() {
        return Err(PdfError::UnplacedEffect);
    }
    let area = painting::local_bounds(space.viewport, surface)?;
    let density = painter.raster.pixels_per_point
        * space.scale
        * painting::transform_scale(surface.cur_transform());
    let mut pushes = 0;
    let masked = (|| -> Result<(), PdfError> {
        // §6.1: the clipping path clips what the masks then mask.
        if mask
            .fragments
            .iter()
            .any(|fragment| fragment.decoration.masks.clip_path.is_some())
        {
            let clip = soft_mask(painter, mask, area, surface, |_, fragment, values| {
                draw_clip_path(fragment, area, values)
            })?;
            surface.push_mask(clip);
            pushes += 1;
        }
        // §8.1: mask layers and the mask border mask one after the other.
        if mask
            .fragments
            .iter()
            .any(|fragment| fragment.decoration.masks.has_layers())
        {
            let layers = soft_mask(painter, mask, area, surface, |painter, fragment, values| {
                draw_fragment(painter, fragment, area, density, values)
            })?;
            surface.push_mask(layers);
            pushes += 1;
        }
        if mask
            .fragments
            .iter()
            .any(|fragment| fragment.decoration.masks.border.is_drawn())
        {
            let border = soft_mask(painter, mask, area, surface, |painter, fragment, values| {
                draw_border(painter, fragment, area, density, values)
            })?;
            surface.push_mask(border);
            pushes += 1;
        }
        Ok(())
    })();
    let result = masked.and_then(|()| painter.draw(items, space, surface));
    painting::pop_clips(pushes, surface);
    result
}

/// A luminosity soft mask of the values `draw` paints for each fragment over black.
fn soft_mask(
    painter: &mut Painter<'_, '_, '_>,
    mask: &MaskPaint,
    area: Rect,
    surface: &mut Surface<'_>,
    mut draw: impl FnMut(&mut Painter<'_, '_, '_>, &BoxPaint, &mut Surface<'_>) -> Result<(), PdfError>,
) -> Result<Mask, PdfError> {
    let mut builder = surface.stream_builder();
    let mut values = builder.surface();
    let drawn = (|| -> Result<(), PdfError> {
        fill(area, Color::BLACK, &mut values)?;
        // Fragments occupy disjoint areas, so their masks unite.
        for fragment in &mask.fragments {
            let pushes = painting::push_blend(BlendMode::Screen, &mut values)?;
            let result = draw(painter, fragment, &mut values);
            painting::pop_clips(pushes, &mut values);
            result?;
        }
        Ok(())
    })();
    values.finish();
    let stream = builder.finish();
    drawn?;
    Ok(Mask::new(stream, MaskType::Luminosity))
}

/// The mask border image (§8), sliced and drawn as the border image of the fragment would be,
/// as values in its mode. A slice without `fill` leaves the middle transparent black.
fn draw_border(
    painter: &mut Painter<'_, '_, '_>,
    fragment: &BoxPaint,
    area: Rect,
    density: f32,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let masks = &fragment.decoration.masks;
    if !masks.border.is_drawn() {
        return Ok(());
    }
    let mut source = fragment.clone();
    source.decoration = crate::graphics::BoxDecoration {
        borders: fragment.decoration.borders,
        border_image: masks.border.clone(),
        ..Default::default()
    };
    let Some(region) = source
        .border_image()
        .map_err(PdfError::UnresolvedImage)?
        .and_then(|paint| {
            paint
                .pieces
                .iter()
                .map(|piece| piece.area)
                .reduce(Rect::union)
        })
    else {
        return Ok(());
    };
    let item = [DisplayItem::Box(source)];
    let Some(mut raster) = raster::rasterize(painter, &item, region.intersection(area), density)?
    else {
        return Ok(());
    };
    embed_values(
        &mut raster,
        masks.border_mode == MaskMode::Luminance,
        surface,
    )
}

/// Draws `raster`'s mask values as opaque grey.
fn embed_values(
    raster: &mut raster::Raster,
    luminance: bool,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let values = mask_values(&raster.pixmap, luminance);
    for (pixel, value) in raster
        .pixmap
        .pixels_mut()
        .iter_mut()
        .zip(values.color.as_chunks::<3>().0)
    {
        *pixel = tiny_skia::ColorU8::from_rgba(value[0], value[1], value[2], u8::MAX).premultiply();
    }
    raster::embed(raster, surface)
}

fn fill(area: Rect, color: Color, surface: &mut Surface<'_>) -> Result<(), PdfError> {
    painting::draw_fill(
        crate::graphics::Fill {
            shape: crate::graphics::Shape::Rect(area),
            color,
        },
        area,
        surface,
    )
}

/// One fragment's clipping region in white over black (§6.1), cut where the fragment is
/// sliced from the rest of its box.
fn draw_clip_path(
    fragment: &BoxPaint,
    area: Rect,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    fill(area, Color::BLACK, surface)?;
    let Some(clip_path) = &fragment.decoration.masks.clip_path else {
        return Ok(());
    };
    painting::draw_fill(
        crate::graphics::Fill {
            shape: crate::graphics::Shape::Clipped {
                shape: Box::new(clip_path.outline(fragment)),
                bounds: crate::graphics::ClipBounds::new(fragment.slice_bounds(area), [true; 2]),
            },
            color: WHITE,
        },
        area,
        surface,
    )
}

/// One fragment's layers composited over black.
fn draw_fragment(
    painter: &mut Painter<'_, '_, '_>,
    fragment: &BoxPaint,
    area: Rect,
    density: f32,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let masks = &fragment.decoration.masks;
    let layers = fragment
        .mask_layers(painter.page_area)
        .map_err(PdfError::UnresolvedImage)?;
    let count = masks.layer_count();
    fill(area, Color::BLACK, surface)?;
    for index in (0..count).rev() {
        // The bottom-most layer has nothing below it to composite with (§7.8).
        let operator = if index + 1 == count {
            MaskComposite::Add
        } else {
            masks.composite(index)
        };
        if operator == MaskComposite::Subtract {
            let pushes = painting::push_blend(BlendMode::Difference, surface)?;
            let inverted = fill(area, WHITE, surface);
            painting::pop_clips(pushes, surface);
            inverted?;
        }
        let blend = match operator {
            MaskComposite::Add => BlendMode::Screen,
            MaskComposite::Subtract | MaskComposite::Intersect => BlendMode::Multiply,
            MaskComposite::Exclude => BlendMode::Exclusion,
        };
        let pushes = painting::push_blend(blend, surface)?;
        let drawn = (|| {
            fill(area, Color::BLACK, surface)?;
            if let Some(reference) = masks.source(index) {
                draw_source(
                    painter,
                    reference,
                    fragment.rect,
                    masks.mode(index),
                    area,
                    density,
                    surface,
                )
            } else if let Some((_, layer)) = layers.iter().find(|(layer, _)| *layer == index) {
                let clip = if masks.is_unclipped(index) {
                    Clip {
                        rect: area,
                        radii: Radii::default(),
                        reference_box: None,
                        axes: [true; 2],
                    }
                } else {
                    layer.clip
                };
                draw_layer(
                    painter,
                    layer,
                    clip,
                    masks.mode(index),
                    area,
                    density,
                    surface,
                )
            } else {
                Ok(())
            }
        })();
        painting::pop_clips(pushes, surface);
        drawn?;
    }
    Ok(())
}

/// A colour's mask value as an opaque grey scaled by its alpha when drawn over black.
fn mask_color(color: Color, luminance: bool) -> Color {
    if !luminance {
        return Color {
            rgb: WHITE.rgb,
            ..color
        };
    }
    let value = color
        .rgb
        .iter()
        .zip(LUMINANCE)
        .map(|(channel, weight)| f32::from(*channel) * weight)
        .sum::<f32>()
        .round() as u8;
    Color {
        rgb: [value; 3],
        alpha: color.alpha,
    }
}

/// `<image>` layers are alpha masks under `match-source` (§7.2).
fn draw_layer(
    painter: &mut Painter<'_, '_, '_>,
    layer: &LayerPaint<'_>,
    clip: Clip,
    mode: MaskMode,
    area: Rect,
    density: f32,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let luminance = mode == MaskMode::Luminance;
    let grey;
    let image = match &layer.image {
        TileImage::Gradient(gradient) => TileImage::Gradient(ResolvedGradient {
            shading: gradient.shading,
            stops: gradient
                .stops
                .iter()
                .map(|stop| crate::paint::GradientStop {
                    color: mask_color(stop.color, luminance),
                    ..*stop
                })
                .collect(),
        }),
        TileImage::Image(image) => {
            let tile = Rect {
                x: 0.0,
                y: 0.0,
                ..layer.tile
            };
            let item = [DisplayItem::Image(ImagePaint {
                image: Arc::clone(image),
                rect: tile,
                clip: None,
            })];
            let Some(raster) = raster::rasterize(painter, &item, tile, density)? else {
                return Ok(());
            };
            grey = Arc::new(Image::Pixels(Arc::new(mask_values(
                &raster.pixmap,
                luminance,
            ))));
            TileImage::Image(&grey)
        }
    };
    crate::writer::backgrounds::draw_layer(
        &LayerPaint {
            image,
            clip,
            ..layer.clone()
        },
        area,
        surface,
        painter.images,
    )
}

/// Opaque grey pixels holding each pixel's mask value.
fn mask_values(pixmap: &tiny_skia::Pixmap, luminance: bool) -> PixelImage {
    let color = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let pixel = pixel.demultiply();
            let alpha = f32::from(pixel.alpha()) / CHANNEL_MAX;
            let value = if luminance {
                [pixel.red(), pixel.green(), pixel.blue()]
                    .iter()
                    .zip(LUMINANCE)
                    .map(|(channel, weight)| f32::from(*channel) * weight)
                    .sum::<f32>()
                    * alpha
            } else {
                alpha * CHANNEL_MAX
            };
            [value.round() as u8; 3]
        })
        .collect();
    PixelImage {
        width: pixmap.width(),
        height: pixmap.height(),
        depth: PixelDepth::Eight,
        color,
        alpha: None,
        icc: None,
    }
}

/// A `mask` element: it positions, sizes and clips itself (§7.9); a reference naming no mask
/// element is a transparent black layer. `mask-mode` other than `match-source` overrides its
/// `mask-type`.
fn draw_source(
    painter: &Painter<'_, '_, '_>,
    reference: &str,
    bounds: Rect,
    mode: MaskMode,
    area: Rect,
    density: f32,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let Some(mask) = painter
        .raster
        .definitions
        .get(reference)
        .filter(|definition| definition.element == "mask")
        .and_then(|definition| {
            crate::svg::reference_tree(&definition.xml, "mask", reference, Some(bounds))
        })
        .and_then(|tree| first_mask(tree.root()))
    else {
        return Ok(());
    };
    let mut mask = mask;
    match mode {
        MaskMode::Alpha => mask.kind = crate::svg::usvg::MaskType::Alpha,
        MaskMode::Luminance => mask.kind = crate::svg::usvg::MaskType::Luminance,
        MaskMode::MatchSource => {}
    }
    let columns = (area.width * density).ceil() as u32;
    let rows = (area.height * density).ceil() as u32;
    let Some(mut pixmap) = tiny_skia::Pixmap::new(columns, rows) else {
        return Ok(());
    };
    pixmap.fill(tiny_skia::Color::WHITE);
    let raster = raster::Raster {
        pixmap,
        area: Rect {
            width: columns as f32 / density,
            height: rows as f32 / density,
            ..area
        },
        density,
    };
    let transform = raster.transform().pre_scale(
        crate::graphics::POINTS_PER_PIXEL,
        crate::graphics::POINTS_PER_PIXEL,
    );
    let mut raster = raster;
    crate::svg::resvg::apply_mask(&mask, transform, &mut raster.pixmap);
    embed_values(&mut raster, false, surface)
}

fn first_mask(group: &crate::svg::usvg::Group) -> Option<crate::svg::usvg::Mask> {
    group
        .mask()
        .and_then(|mask| {
            Some(crate::svg::usvg::Mask {
                id: crate::svg::usvg::NonEmptyString::new(mask.id().to_owned())?,
                rect: mask.rect,
                kind: mask.kind,
                mask: mask.mask.clone(),
                root: mask.root.clone(),
            })
        })
        .or_else(|| {
            group.children().iter().find_map(|node| match node {
                crate::svg::usvg::Node::Group(group) => first_mask(group),
                _ => None,
            })
        })
}
