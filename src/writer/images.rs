use crate::graphics::ImagePaint;
use crate::images::{Image as Asset, ImageView, Orientation, RasterFormat};
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

/// The orientation and scale transforms and the region clip a view draws inside.
const VIEW_PUSHES: usize = 3;

pub(super) struct ImageWriter {
    failed: HashMap<usize, String>,
    reported: HashSet<usize>,
    diagnostics: Vec<String>,
    images: HashMap<usize, Image>,
    vectors: HashMap<(usize, u32, u32), crate::svg::usvg::Tree>,
    opaque: HashMap<usize, bool>,
    /// The density images composited in pixels resolve at.
    pub(super) pixels_per_point: f32,
}
impl ImageWriter {
    pub(super) fn new(pixels_per_point: f32) -> Self {
        Self {
            failed: HashMap::new(),
            reported: HashSet::new(),
            diagnostics: Vec::new(),
            images: HashMap::new(),
            vectors: HashMap::new(),
            opaque: HashMap::new(),
            pixels_per_point,
        }
    }
    /// Whether every pixel of `image` is opaque, decoding it once.
    pub(super) fn opaque(&mut self, image: &Arc<Asset>) -> bool {
        *self
            .opaque
            .entry(Arc::as_ptr(image) as usize)
            .or_insert_with(|| image.opaque())
    }
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
                .as_ref()
                .is_some_and(|clip| clip.rect.width == 0.0 || clip.rect.height == 0.0)
        {
            return Ok(());
        }
        match paint.image.as_ref() {
            Asset::Raster(_) | Asset::Pixels(_) | Asset::Svg(_) => {
                self.draw_decoded(paint, viewport, surface)
            }
            Asset::Transparent => Ok(()),
            Asset::View(view) => self.draw_view(view, paint, viewport, surface),
            Asset::Generated(image) => self.draw_generated(image, paint, viewport, surface),
            Asset::Viewport(_) => unreachable!("painters draw nested viewports"),
            Asset::Frame(..) => unreachable!("layout replaces frames with their viewports"),
            Asset::Widget(_) => unreachable!("layout replaces widgets with their drawings"),
        }
    }

    fn draw_decoded(
        &mut self,
        paint: &ImagePaint,
        viewport: crate::graphics::Rect,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let key = Arc::as_ptr(&paint.image) as usize;
        let source_size = [paint.rect.width, paint.rect.height].map(|v| v / paint.source_scale);
        let vector_key = (key, source_size[0].to_bits(), source_size[1].to_bits());
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
                Asset::Svg(_)
                | Asset::Transparent
                | Asset::Frame(..)
                | Asset::Viewport(_)
                | Asset::Widget(_)
                | Asset::View(_)
                | Asset::Generated(_) => None,
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
            Asset::Pixels(_)
            | Asset::Transparent
            | Asset::Frame(..)
            | Asset::Viewport(_)
            | Asset::Widget(_)
            | Asset::View(_)
            | Asset::Generated(_) => Orientation::NoTransforms,
            Asset::Svg(svg) => {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    self.vectors.entry(vector_key)
                {
                    let size = source_size.map(|v| v / crate::style::POINTS_PER_PIXEL);
                    entry.insert(svg.tree(size).map_err(PdfError::Svg)?);
                }
                Orientation::NoTransforms
            }
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
        surface.push_transform(&oriented(orientation, rect));
        let unit_size = Size::from_wh(1.0, 1.0).unwrap();
        match paint.image.as_ref() {
            Asset::Raster(_) | Asset::Pixels(_) => {
                surface.draw_image(self.images[&key].clone(), unit_size);
            }
            Asset::Transparent
            | Asset::Frame(..)
            | Asset::Viewport(_)
            | Asset::Widget(_)
            | Asset::View(_)
            | Asset::Generated(_) => {
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
                        filter_scale: self.pixels_per_point,
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

    /// Draws a generated image filling the paint rectangle, as a background tile of its size.
    fn draw_generated(
        &mut self,
        image: &crate::graphics::BackgroundImage,
        paint: &ImagePaint,
        viewport: crate::graphics::Rect,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let Some(tile) = crate::paint::Source::of(image)
            .map_err(PdfError::UnresolvedImage)?
            .and_then(|source| source.tile([paint.rect.width, paint.rect.height]))
        else {
            return Ok(());
        };
        let clips = paint
            .clip
            .as_ref()
            .map(|clip| crate::writer::painting::push_clip(clip, viewport, surface))
            .transpose()?
            .unwrap_or(0);
        let drawn =
            crate::writer::backgrounds::draw_tile(&tile, paint.rect, viewport, surface, self);
        crate::writer::painting::pop_clips(clips, surface);
        drawn
    }

    /// Draws the view's region into the paint rectangle: the whole image draws scaled so that
    /// the region fills it, clipped to the region, then turned by the view's orientation.
    fn draw_view(
        &mut self,
        view: &ImageView,
        paint: &ImagePaint,
        viewport: crate::graphics::Rect,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let rect = paint.rect;
        let clips = paint
            .clip
            .as_ref()
            .map(|clip| crate::writer::painting::push_clip(clip, viewport, surface))
            .transpose()?
            .unwrap_or(0);
        let [width, height] = view.region_size([rect.width, rect.height]);
        let region = crate::graphics::Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        surface.push_transform(&oriented(view.orientation, rect));
        surface.push_transform(&Transform::from_scale(width.recip(), height.recip()));
        surface.push_clip_path(
            &crate::writer::painting::rect_path(region)?,
            &krilla::paint::FillRule::NonZero,
        );
        let drawn = self.draw(
            &ImagePaint {
                image: view.image.clone(),
                rect: view.image_rect([width, height]),
                clip: None,
                source_scale: paint.source_scale,
            },
            viewport,
            surface,
        );
        for _ in 0..VIEW_PUSHES {
            surface.pop();
        }
        crate::writer::painting::pop_clips(clips, surface);
        drawn
    }
}

/// Maps the unit square of an image's stored pixels onto `rect`, turned and flipped by
/// `orientation`.
fn oriented(orientation: Orientation, rect: crate::graphics::Rect) -> Transform {
    use Orientation::{
        FlipHorizontal, FlipVertical, NoTransforms, Rotate90, Rotate90FlipH, Rotate180, Rotate270,
        Rotate270FlipH,
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
    Transform::from_row(
        a * rect.width,
        b * rect.height,
        c * rect.width,
        d * rect.height,
        rect.x + e * rect.width,
        rect.y + f * rect.height,
    )
}
