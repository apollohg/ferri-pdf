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
    BlendMode, BoxDecoration, BoxPaint, Clip, Color, DisplayItem, ImagePaint, LayerPaint,
    LoadedMasks, MaskComposite, MaskMode, MaskPaint, POINTS_PER_PIXEL, Rect, ResolvedGradient,
    TileImage,
};
use crate::images::{Image, PixelDepth, PixelImage};
use crate::svg::krilla_svg::UsvgTransformExt;
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
const CLIP_PATH_ELEMENT: &str = "clipPath";
/// The `mask` element and the property naming it.
const MASK_ELEMENT: &str = "mask";
const CLIP_PATH_PROPERTY: &str = "clip-path";
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
            .any(|fragment| fragment.decoration.masks.is_clipped())
        {
            let clip = soft_mask(painter, mask, area, surface, |painter, fragment, values| {
                draw_clip_path(painter, fragment, area, values)
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
    luminosity_mask(surface, |values| {
        fill(area, Color::BLACK, values)?;
        // Fragments occupy disjoint areas, so their masks unite.
        for fragment in &mask.fragments {
            let pushes = painting::push_blend(BlendMode::Screen, values)?;
            let result = draw(painter, fragment, values);
            painting::pop_clips(pushes, values);
            result?;
        }
        Ok(())
    })
}

/// A luminosity soft mask whose values are the greys `draw` paints.
fn luminosity_mask(
    surface: &mut Surface<'_>,
    draw: impl FnOnce(&mut Surface<'_>) -> Result<(), PdfError>,
) -> Result<Mask, PdfError> {
    let mut builder = surface.stream_builder();
    let mut values = builder.surface();
    let drawn = draw(&mut values);
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
    source.decoration = BoxDecoration {
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
    painter: &Painter<'_, '_, '_>,
    fragment: &BoxPaint,
    area: Rect,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    fill(area, Color::BLACK, surface)?;
    let masks = &fragment.decoration.masks;
    if let Some(reference) = &masks.clip_reference {
        surface.push_clip_path(
            &painting::rect_path(fragment.slice_bounds(area))?,
            &krilla::paint::FillRule::NonZero,
        );
        let drawn = draw_clip_element(painter, reference, fragment, area, surface);
        surface.pop();
        return drawn;
    }
    let Some(clip_path) = &masks.clip_path else {
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

/// The region a `clipPath` element clips the fragment's box to, in white (§6.1). Its user
/// space starts at the box's border-box origin, which with the border box's size is also its
/// object bounding box. A reference naming no `clipPath` element clips nothing.
fn draw_clip_element(
    painter: &Painter<'_, '_, '_>,
    reference: &str,
    fragment: &BoxPaint,
    area: Rect,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let border = fragment.decoration_rect();
    let Some(tree) = painter.raster.definitions.reference_tree(
        CLIP_PATH_ELEMENT,
        CLIP_PATH_PROPERTY,
        reference,
        Some(Rect {
            x: 0.0,
            y: 0.0,
            ..border
        }),
    ) else {
        return fill(area, WHITE, surface);
    };
    surface.push_transform(&krilla::geom::Transform::from_row(
        POINTS_PER_PIXEL,
        0.0,
        0.0,
        POINTS_PER_PIXEL,
        border.x,
        border.y,
    ));
    crate::svg::krilla_svg::render_tree(
        &tree,
        crate::svg::krilla_svg::SvgSettings::default(),
        surface,
    );
    surface.pop();
    Ok(())
}

/// One fragment's layers composited over black.
fn draw_fragment(
    painter: &mut Painter<'_, '_, '_>,
    fragment: &BoxPaint,
    area: Rect,
    density: f32,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let layers = fragment
        .mask_layers(painter.page_area)
        .map_err(PdfError::UnresolvedImage)?;
    draw_layers(
        painter,
        &fragment.decoration.masks,
        &layers,
        fragment.rect,
        area,
        density,
        &[],
        surface,
    )
}

/// `masks`'s placed `layers` composited over black; `bounds` is the object bounding box of
/// referenced `mask` elements, and `active` holds the references already being drawn.
#[expect(
    clippy::too_many_arguments,
    reason = "the layers draw in an area at a density for a chain of references"
)]
fn draw_layers(
    painter: &mut Painter<'_, '_, '_>,
    masks: &LoadedMasks,
    layers: &[(usize, LayerPaint<'_>)],
    bounds: Rect,
    area: Rect,
    density: f32,
    active: &[&str],
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
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
                    bounds,
                    masks.mode(index),
                    area,
                    density,
                    active,
                    surface,
                )
            } else if let Some((_, layer)) = layers.iter().find(|(layer, _)| *layer == index) {
                let clip = if masks.is_unclipped(index) {
                    Clip::rectangle(area)
                } else {
                    layer.clip.clone()
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
        TileImage::Color(color) => TileImage::Color(mask_color(*color, luminance)),
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
        TileImage::Image(image, scale, rendering) => {
            let tile = Rect {
                x: 0.0,
                y: 0.0,
                ..layer.tile
            };
            let item = [DisplayItem::Image(ImagePaint {
                image: Arc::clone(image),
                rect: tile,
                clip: None,
                source_scale: *scale,
                rendering: *rendering,
            })];
            let Some(raster) = raster::rasterize(painter, &item, tile, density)? else {
                return Ok(());
            };
            grey = Arc::new(Image::Pixels(Arc::new(mask_values(
                &raster.pixmap,
                luminance,
            ))));
            TileImage::Image(&grey, crate::paint::INITIAL_IMAGE_SCALE, *rendering)
        }
        TileImage::CrossFade(..) => {
            let tile = Rect {
                x: 0.0,
                y: 0.0,
                ..layer.tile
            };
            let Some(raster) =
                raster::render(painter, tile, density, |painter, space, surface| {
                    crate::writer::backgrounds::draw_tile(
                        &layer.image,
                        tile,
                        space.viewport,
                        surface,
                        painter.images,
                    )
                })?
            else {
                return Ok(());
            };
            grey = Arc::new(Image::Pixels(Arc::new(mask_values(
                &raster.pixmap,
                luminance,
            ))));
            // The raster has the drawing space's density, which draws it without scaling.
            TileImage::Image(
                &grey,
                crate::paint::INITIAL_IMAGE_SCALE,
                crate::paint::ImageRendering::AUTO,
            )
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
/// element is a transparent black layer, as is one inside the content of the element it
/// names. `mask-mode` other than `match-source` overrides its `mask-type`.
#[expect(
    clippy::too_many_arguments,
    reason = "the element draws in an area at a density for a chain of references"
)]
fn draw_source(
    painter: &mut Painter<'_, '_, '_>,
    reference: &str,
    bounds: Rect,
    mode: MaskMode,
    area: Rect,
    density: f32,
    active: &[&str],
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    if active.contains(&reference) {
        return Ok(());
    }
    painter.images.mask_element_draws += 1;
    if painter.images.mask_element_draws > MAX_MASK_ELEMENT_DRAWS_PER_PAGE {
        return Err(PdfError::MaskElementDrawLimit {
            limit: MAX_MASK_ELEMENT_DRAWS_PER_PAGE,
        });
    }
    let Some(mask) = painter
        .raster
        .definitions
        .reference_tree(MASK_ELEMENT, MASK_ELEMENT, reference, Some(bounds))
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
    let transform = raster
        .transform()
        .pre_scale(POINTS_PER_PIXEL, POINTS_PER_PIXEL);
    let mut raster = raster;
    let active: Vec<&str> = active.iter().copied().chain([reference]).collect();
    let layer_masks = SvgLayerMasks::new(painter, density, &active);
    crate::svg::resvg::apply_mask(&mask, transform, &mut raster.pixmap, Some(&layer_masks));
    layer_masks.finish()?;
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

/// Draws an SVG image whose content has CSS mask layers, which paint through `painter`.
pub(super) fn draw_svg(
    painter: &mut Painter<'_, '_, '_>,
    paint: &ImagePaint,
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let Some(decoded) = painter.images.decode(paint)? else {
        return Ok(());
    };
    let density = painter.raster.pixels_per_point * space.scale;
    let layer_masks = SvgLayerMasks::new(painter, density, &[]);
    let drawn = crate::writer::images::draw_placed(
        paint,
        &decoded,
        space.viewport,
        surface,
        Some(&layer_masks),
    );
    layer_masks.finish()?;
    let errors = drawn?;
    painter.images.report_svg_errors(paint, errors);
    Ok(())
}

/// The CSS mask layers and mask border of SVG content, painted as those of a box whose areas
/// are the content's fill, stroke and view boxes. Painting errors wait for [`Self::finish`].
pub(super) struct SvgLayerMasks<'p, 'a, 'w, 'f> {
    painter: std::cell::RefCell<&'p mut Painter<'a, 'w, 'f>>,
    /// Raster pixels per unit of the space that drawing surfaces transform from.
    density: f32,
    /// The `mask` element references being drawn.
    active: &'p [&'p str],
    error: std::cell::RefCell<Option<PdfError>>,
}

impl<'p, 'a, 'w, 'f> SvgLayerMasks<'p, 'a, 'w, 'f> {
    pub(super) fn new(
        painter: &'p mut Painter<'a, 'w, 'f>,
        density: f32,
        active: &'p [&'p str],
    ) -> Self {
        Self {
            painter: std::cell::RefCell::new(painter),
            density,
            active,
            error: std::cell::RefCell::new(None),
        }
    }

    /// The first painting error.
    pub(super) fn finish(self) -> Result<(), PdfError> {
        self.error.into_inner().map_or(Ok(()), Err)
    }

    fn record(&self, error: PdfError) {
        self.error.borrow_mut().get_or_insert(error);
    }

    /// Draws the layers' values, or with `border` the mask border's, over black in `area` of a
    /// space in points whose density is `density`, where the masked content has `boxes`.
    fn draw_values(
        &self,
        mask: &crate::svg::usvg::LayerMask,
        boxes: &crate::paint::SvgBoxes,
        border: bool,
        area: Rect,
        density: f32,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let mut painter = self.painter.borrow_mut();
        let masks = &*mask.masks;
        fill(area, Color::BLACK, surface)?;
        if border {
            // §8: the mask border image area is the border box, which is the stroke box of SVG
            // content.
            let fragment = BoxPaint::whole(
                boxes.stroke,
                BoxDecoration {
                    masks: mask.masks.clone(),
                    ..BoxDecoration::default()
                },
            );
            return draw_border(&mut painter, &fragment, area, density, surface);
        }
        let layers = masks.svg_layers(boxes).map_err(PdfError::UnresolvedImage)?;
        draw_layers(
            &mut painter,
            masks,
            &layers,
            boxes.fill,
            area,
            density,
            self.active,
            surface,
        )
    }
}

/// Each drawn `mask` element is a raster of its area, and references shared at each level of
/// nesting multiply, so a page's draws are bounded.
const MAX_MASK_ELEMENT_DRAWS_PER_PAGE: usize = 1024;
/// User units per point.
const USER_UNITS_PER_POINT: f32 = 1.0 / POINTS_PER_PIXEL;
/// The transforms into and out of the space in points the mask values draw in.
const POINT_SPACE_PUSHES: u16 = 2;

/// The rectangle of user-space `rect` in points.
fn points(rect: crate::svg::usvg::Rect) -> Rect {
    Rect {
        x: rect.x() * POINTS_PER_PIXEL,
        y: rect.y() * POINTS_PER_PIXEL,
        width: rect.width() * POINTS_PER_PIXEL,
        height: rect.height() * POINTS_PER_PIXEL,
    }
}

/// Whether `masks` draws the mask layers, then whether the mask border, which mask one after
/// the other (§8.1): `false` for the layers, `true` for the border.
fn value_masks(masks: &LoadedMasks) -> impl Iterator<Item = bool> + '_ {
    [false, true].into_iter().filter(|&border| {
        if border {
            masks.border.is_drawn()
        } else {
            masks.has_layers()
        }
    })
}

impl crate::svg::LayerMasks for SvgLayerMasks<'_, '_, '_, '_> {
    fn push(
        &self,
        mask: &crate::svg::usvg::LayerMask,
        bounds: crate::svg::usvg::Rect,
        surface: &mut Surface<'_>,
    ) -> u16 {
        // The masks keep the space in points their values draw in; the content returns to user
        // units.
        surface.push_transform(&krilla::geom::Transform::from_scale(
            USER_UNITS_PER_POINT,
            USER_UNITS_PER_POINT,
        ));
        let area = points(bounds);
        let density = self.density * painting::transform_scale(surface.cur_transform());
        let mut pushes = POINT_SPACE_PUSHES;
        for border in value_masks(&mask.masks) {
            match luminosity_mask(surface, |values| {
                self.draw_values(mask, &mask.boxes, border, area, density, values)
            }) {
                Ok(values) => {
                    surface.push_mask(values);
                    pushes += 1;
                }
                Err(error) => self.record(error),
            }
        }
        surface.push_transform(&krilla::geom::Transform::from_scale(
            POINTS_PER_PIXEL,
            POINTS_PER_PIXEL,
        ));
        pushes
    }

    fn apply(
        &self,
        mask: &crate::svg::usvg::LayerMask,
        transform: tiny_skia::Transform,
        pixmap: &mut tiny_skia::Pixmap,
    ) {
        let to_pixels = transform.pre_scale(USER_UNITS_PER_POINT, USER_UNITS_PER_POINT);
        let area = to_pixels.invert().and_then(|from_pixels| {
            tiny_skia::Rect::from_xywh(0.0, 0.0, pixmap.width() as f32, pixmap.height() as f32)?
                .transform(from_pixels)
        });
        let Some(area) = area else {
            pixmap.fill(tiny_skia::Color::TRANSPARENT);
            return;
        };
        let area = Rect {
            x: area.x(),
            y: area.y(),
            width: area.width(),
            height: area.height(),
        };
        let density = painting::transform_scale(to_pixels.to_krilla());
        for border in value_masks(&mask.masks) {
            let values = raster::render_surface(area, density, |viewport, surface| {
                self.draw_values(mask, &mask.boxes, border, viewport, density, surface)
            });
            let values = match values {
                Ok(values) => values,
                Err(error) => {
                    self.record(error);
                    continue;
                }
            };
            // The values cover the pixmap where the transform places their raster.
            let placed = values.and_then(|values| {
                let mut placed = tiny_skia::Pixmap::new(pixmap.width(), pixmap.height())?;
                placed.draw_pixmap(
                    0,
                    0,
                    values.pixmap.as_ref(),
                    &tiny_skia::PixmapPaint {
                        quality: tiny_skia::FilterQuality::Bilinear,
                        ..tiny_skia::PixmapPaint::default()
                    },
                    to_pixels.pre_concat(values.transform().invert()?),
                    None,
                );
                Some(placed)
            });
            match placed {
                Some(placed) => pixmap.apply_mask(&tiny_skia::Mask::from_pixmap(
                    placed.as_ref(),
                    tiny_skia::MaskType::Luminance,
                )),
                None => pixmap.fill(tiny_skia::Color::TRANSPARENT),
            }
        }
    }
}
