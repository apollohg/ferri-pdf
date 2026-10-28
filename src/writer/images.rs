use crate::graphics::ImagePaint;
use crate::images::{Image as Asset, Orientation, RasterFormat};
use crate::svg::krilla_svg::SurfaceExt;
use crate::writer::PdfError;
use krilla::{
    geom::{Size, Transform},
    image::Image,
    surface::Surface,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

#[derive(Default)]
pub(super) struct ImageWriter {
    failed: HashMap<usize, String>,
    reported: HashSet<usize>,
    diagnostics: Vec<String>,
    images: HashMap<usize, Image>,
    vectors: HashMap<(usize, u32, u32), crate::svg::usvg::Tree>,
}
impl ImageWriter {
    pub(crate) fn take_diagnostics(&mut self) -> Vec<String> {
        self.reported.clear();
        std::mem::take(&mut self.diagnostics)
    }
    fn report_failure(&mut self, key: usize) {
        if self.reported.insert(key) {
            self.diagnostics.push(self.failed[&key].clone());
        }
    }

    pub(crate) fn draw(
        &mut self,
        paint: &ImagePaint,
        viewport: crate::graphics::Rect,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        if paint.rect.width == 0.0
            || paint.rect.height == 0.0
            || paint
                .clip
                .is_some_and(|clip| clip.rect.width == 0.0 || clip.rect.height == 0.0)
        {
            return Ok(());
        }
        match paint.image.as_ref() {
            Asset::Raster(_) | Asset::Pixels(_) | Asset::Svg(_) => {
                self.draw_decoded(paint, viewport, surface)
            }
            Asset::Transparent => Ok(()),
            Asset::Viewport(view) => self.draw_viewport(paint, view, viewport, surface),
            Asset::Frame(_) => unreachable!("layout replaces frames with their viewports"),
        }
    }

    /// Paints a nested document's viewport at the paint rectangle's origin.
    fn draw_viewport(
        &mut self,
        paint: &ImagePaint,
        view: &crate::layout::ViewportPaint,
        viewport: crate::graphics::Rect,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let clips = paint
            .clip
            .as_ref()
            .map(|clip| crate::writer::painting::push_clip(clip, viewport, surface))
            .transpose()?
            .unwrap_or(0);
        let offset = [paint.rect.x - view.area.x, paint.rect.y - view.area.y];
        surface.push_transform(&Transform::from_translate(offset[0], offset[1]));
        let local = crate::graphics::Rect {
            x: viewport.x - offset[0],
            y: viewport.y - offset[1],
            ..viewport
        };
        let mut text = crate::writer::text::TextWriter::new(&view.fonts);
        let drawn = crate::writer::draw_items(
            &crate::writer::Layer {
                items: &view.items,
                lines: &view.lines,
                fonts: &view.fonts,
                page: 0,
                page_area: view.area,
            },
            local,
            surface,
            self,
            &mut text,
        );
        surface.pop();
        crate::writer::painting::pop_clips(clips, surface);
        drawn
    }

    fn draw_decoded(
        &mut self,
        paint: &ImagePaint,
        viewport: crate::graphics::Rect,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let key = Arc::as_ptr(&paint.image) as usize;
        let vector_key = (key, paint.rect.width.to_bits(), paint.rect.height.to_bits());
        if self.failed.contains_key(&key) {
            self.report_failure(key);
            return Ok(());
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = self.images.entry(key) {
            let loaded = match paint.image.as_ref() {
                Asset::Raster(raster) => {
                    let data = raster.bytes.clone().into();
                    Some(match raster.format {
                        RasterFormat::Png => Image::from_png(data, false),
                        RasterFormat::Jpeg => Image::from_jpeg(data, false),
                        RasterFormat::Gif => Image::from_gif(data, false),
                        RasterFormat::WebP => Image::from_webp(data, false),
                    })
                }
                Asset::Pixels(pixels) => Some(
                    Image::from_custom(crate::writer::pixels::Pixels(pixels.clone()), false)
                        .map(Image::with_jpeg_compression),
                ),
                Asset::Svg(_) | Asset::Transparent | Asset::Frame(_) | Asset::Viewport(_) => None,
            };
            if let Some(loaded) = loaded {
                match loaded.and_then(|image| {
                    image.validate()?;
                    Ok(image)
                }) {
                    Ok(image) => {
                        entry.insert(image);
                    }
                    Err(error) => {
                        self.failed.insert(key, error);
                        self.report_failure(key);
                        return Ok(());
                    }
                }
            }
        }
        let orientation = match paint.image.as_ref() {
            Asset::Raster(raster) => raster.orientation,
            Asset::Pixels(_) | Asset::Transparent | Asset::Frame(_) | Asset::Viewport(_) => {
                NoTransforms
            }
            Asset::Svg(svg) => {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    self.vectors.entry(vector_key)
                {
                    let size = [paint.rect.width, paint.rect.height]
                        .map(|v| v / crate::style::POINTS_PER_PIXEL);
                    entry.insert(svg.tree(size).map_err(PdfError::Svg)?);
                }
                NoTransforms
            }
        };
        use Orientation::{
            FlipHorizontal, FlipVertical, NoTransforms, Rotate90, Rotate90FlipH, Rotate180,
            Rotate270, Rotate270FlipH,
        };
        let [a, b, c, d, e, f] = match orientation {
            NoTransforms => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            FlipHorizontal => [-1.0, 0.0, 0.0, 1.0, 1.0, 0.0],
            Rotate180 => [-1.0, 0.0, 0.0, -1.0, 1.0, 1.0],
            FlipVertical => [1.0, 0.0, 0.0, -1.0, 0.0, 1.0],
            Rotate90FlipH => [0.0, 1.0, 1.0, 0.0, 0.0, 0.0],
            Rotate90 => [0.0, 1.0, -1.0, 0.0, 1.0, 0.0],
            Rotate270FlipH => [0.0, -1.0, -1.0, 0.0, 1.0, 1.0],
            Rotate270 => [0.0, -1.0, 1.0, 0.0, 0.0, 1.0],
        };
        let rect = paint.rect;
        if ![rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|v| v.is_finite())
            || rect.width < 0.0
            || rect.height < 0.0
        {
            return Err(PdfError::InvalidGeometry);
        }
        let clips = paint
            .clip
            .as_ref()
            .map(|clip| crate::writer::painting::push_clip(clip, viewport, surface))
            .transpose()?
            .unwrap_or(0);
        surface.push_transform(&Transform::from_row(
            a * rect.width,
            b * rect.height,
            c * rect.width,
            d * rect.height,
            rect.x + e * rect.width,
            rect.y + f * rect.height,
        ));
        let unit_size = Size::from_wh(1.0, 1.0).unwrap();
        match paint.image.as_ref() {
            Asset::Raster(_) | Asset::Pixels(_) => {
                surface.draw_image(self.images[&key].clone(), unit_size);
            }
            Asset::Transparent | Asset::Frame(_) | Asset::Viewport(_) => {
                unreachable!("draw dispatches only decoded images here")
            }
            Asset::Svg(svg) => {
                let errors = std::cell::RefCell::new(std::collections::BTreeSet::new());
                surface.draw_svg(
                    &self.vectors[&vector_key],
                    unit_size,
                    crate::svg::krilla_svg::SvgSettings {
                        jpeg_images: Some(svg.jpeg_images()),
                        image_errors: Some(&errors),
                        ..Default::default()
                    },
                );
                let errors = errors.into_inner();
                if !errors.is_empty() && self.reported.insert(key) {
                    self.diagnostics.extend(errors);
                }
            }
        }
        surface.pop();
        crate::writer::painting::pop_clips(clips, surface);
        Ok(())
    }
}
