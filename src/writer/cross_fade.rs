//! `cross-fade()` images: the weighted average of their premultiplied arguments (CSS Images 4
//! §2.6.2). PDF composites only by source-over, which yields a weighted average exactly when
//! every argument drawn above the first is opaque: each is drawn with the constant alpha of
//! its share of the weight not yet drawn above it. With more than one translucent argument no
//! sequence of PDF compositing operations sums them, so the arguments are rasterized and
//! averaged pixel by pixel.
use crate::graphics::{Rect, TileImage};
use crate::writer::{PdfError, images::ImageWriter, painting, raster};
use krilla::{num::NormalizedF32, surface::Surface};

const RGBA_CHANNELS: usize = 4;

pub(super) fn draw(
    arguments: &[(TileImage<'_>, f32)],
    rect: Rect,
    viewport: Rect,
    surface: &mut Surface<'_>,
    images: &mut ImageWriter,
) -> Result<(), PdfError> {
    let opaque: Vec<bool> = arguments
        .iter()
        .map(|(image, _)| is_opaque(image, images))
        .collect();
    let translucent = opaque.iter().filter(|opaque| !**opaque).count();
    if translucent > 1 {
        return draw_raster(arguments, rect, surface, images);
    }
    // The translucent argument, if any, is drawn first; the others keep their order.
    let mut order: Vec<usize> = (0..arguments.len()).collect();
    order.sort_by_key(|index| opaque[*index]);
    let mut remaining = 1.0;
    let mut alphas = vec![0.0; arguments.len()];
    for index in order.iter().rev() {
        let weight = arguments[*index].1;
        alphas[*index] = if remaining > 0.0 {
            (weight / remaining).min(1.0)
        } else {
            0.0
        };
        remaining -= weight;
    }
    surface.push_isolated();
    let mut drawn = Ok(());
    for index in order {
        let alpha = alphas[index];
        if alpha <= 0.0 {
            continue;
        }
        surface.push_opacity(NormalizedF32::new(alpha).ok_or(PdfError::InvalidAlpha(alpha))?);
        drawn = crate::writer::backgrounds::draw_tile(
            &arguments[index].0,
            rect,
            viewport,
            surface,
            images,
        );
        surface.pop();
        if drawn.is_err() {
            break;
        }
    }
    surface.pop();
    drawn
}

/// Whether every pixel of `image` is opaque.
fn is_opaque(image: &TileImage<'_>, images: &mut ImageWriter) -> bool {
    match image {
        TileImage::Image(image, _) => images.opaque(image),
        TileImage::Gradient(gradient) => gradient.stops.iter().all(|stop| stop.color.alpha >= 1.0),
        TileImage::Color(color) => color.alpha >= 1.0,
        TileImage::CrossFade(arguments, leftover) => {
            *leftover <= 0.0 && arguments.iter().all(|(image, _)| is_opaque(image, images))
        }
    }
}

/// Each argument rendered at the raster density of the current drawing space, the pixels
/// averaged by weight, and the average drawn in place.
fn draw_raster(
    arguments: &[(TileImage<'_>, f32)],
    rect: Rect,
    surface: &mut Surface<'_>,
    images: &mut ImageWriter,
) -> Result<(), PdfError> {
    let density = images.pixels_per_point * painting::transform_scale(surface.cur_transform());
    let mut sums: Vec<f32> = Vec::new();
    let mut frame = None;
    for (image, weight) in arguments {
        // Every argument covers the same pixels.
        let Some(rendered) = raster::render_surface(rect, density, |viewport, surface| {
            crate::writer::backgrounds::draw_tile(image, rect, viewport, surface, images)
        })?
        else {
            return Ok(());
        };
        sums.resize(rendered.pixmap.data().len(), 0.0);
        for (sum, sample) in sums.iter_mut().zip(rendered.pixmap.data()) {
            *sum += weight * f32::from(*sample);
        }
        frame = Some(rendered);
    }
    let Some(mut raster) = frame else {
        return Ok(());
    };
    for (pixel, sums) in raster
        .pixmap
        .data_mut()
        .as_chunks_mut::<RGBA_CHANNELS>()
        .0
        .iter_mut()
        .zip(sums.as_chunks::<RGBA_CHANNELS>().0)
    {
        let alpha = sums[RGBA_CHANNELS - 1].round();
        for (channel, sum) in pixel.iter_mut().zip(sums) {
            // Premultiplied colour never exceeds its alpha.
            *channel = sum.round().min(alpha) as u8;
        }
    }
    raster::embed(&raster, surface)
}
