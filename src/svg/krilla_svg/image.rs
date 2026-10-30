use crate::paint::ImageRendering;
use crate::svg::usvg::ImageKind;
use krilla::geom::{Rect, Size};
use krilla::image::Image;
use krilla::paint::FillRule;
use krilla::surface::Surface;

use crate::svg::krilla_svg::util::{KrillaTransformExt, RectExt};
use crate::svg::krilla_svg::{ProcessContext, composite, group};

/// Render an image into a surface.
///
/// Returns `None` if the image could not be rendered.
pub(crate) fn render(
    image: &crate::svg::usvg::Image,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) -> Option<()> {
    if !image.is_visible() {
        return Some(());
    }

    let size = Size::from_wh(image.size().width(), image.size().height()).unwrap();
    let rendering = crate::svg::image_rendering(image.rendering_mode());
    let interpolate = rendering.interpolates();

    match image.kind() {
        ImageKind::JPEG(d) => {
            let image = Image::from_jpeg(d.clone().into(), interpolate);
            draw_raster(image, size, rendering, surface, process_context)?;
        }
        ImageKind::PNG(d) => {
            let mut decoded = Image::from_png(d.clone().into(), interpolate);
            if process_context
                .svg_settings
                .jpeg_images
                .is_some_and(|images| images.contains(&(std::sync::Arc::as_ptr(d) as usize)))
            {
                decoded = decoded.map(Image::with_jpeg_compression);
            }
            draw_raster(decoded, size, rendering, surface, process_context)?;
        }
        ImageKind::GIF(d) => {
            let image = Image::from_gif(d.clone().into(), interpolate);
            draw_raster(image, size, rendering, surface, process_context)?;
        }
        ImageKind::WEBP(d) => {
            let image = Image::from_webp(d.clone().into(), interpolate);
            draw_raster(image, size, rendering, surface, process_context)?;
        }
        ImageKind::SVG(t) => {
            let clip_path =
                Rect::from_xywh(0.0, 0.0, t.size().width(), t.size().height())?.to_clip_path();
            surface.push_clip_path(&clip_path, &FillRule::NonZero);
            group::render_root(t.root(), surface, process_context);
            surface.pop();
        }
    }

    Some(())
}

/// Draws `image` at `size`, enlarged first as `pixelated` enlarges it for the device pixels it
/// covers on `surface`; an enlargement that fails is reported and the image drawn as it is.
fn draw_raster(
    image: Result<Image, String>,
    size: Size,
    rendering: ImageRendering,
    surface: &mut Surface,
    process_context: &ProcessContext,
) -> Option<()> {
    let report = |error: String| {
        if let Some(errors) = process_context.svg_settings.image_errors {
            errors.borrow_mut().insert(error);
        }
    };
    let image = match image.and_then(|image| {
        image.validate()?;
        Ok(image)
    }) {
        Ok(image) => image,
        Err(error) => {
            report(error);
            return None;
        }
    };
    let density = composite::density(
        surface.cur_transform().to_usvg(),
        process_context.svg_settings.filter_scale,
    );
    let image = match rendering.nearest_multiples(
        image.size(),
        [size.width() * density, size.height() * density],
    ) {
        Some(multiples) => {
            let enlarged = image.with_nearest_multiples(multiples);
            match enlarged.validate() {
                Ok(()) => enlarged,
                Err(error) => {
                    report(error);
                    image
                }
            }
        }
        None => image,
    };
    surface.draw_image(image, size);
    Some(())
}
