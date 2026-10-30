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

/// A decoded image: a raster in its orientation, or an SVG tree with the density its filters
/// rasterize at.
pub(super) enum Decoded {
    Raster(Image, Orientation),
    Vector(Arc<crate::svg::usvg::Tree>, f32),
}

/// Draws `decoded`, the image of `paint`, in the paint's rectangle and clip; `layer_masks`
/// draws the CSS mask layers of its SVG content. Returns the SVG's recoverable decoding
/// failures.
pub(super) fn draw_placed(
    paint: &ImagePaint,
    decoded: &Decoded,
    viewport: crate::graphics::Rect,
    surface: &mut Surface<'_>,
    layer_masks: Option<&dyn crate::svg::LayerMasks>,
) -> Result<std::collections::BTreeSet<String>, PdfError> {
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
    let orientation = match decoded {
        Decoded::Raster(_, orientation) => *orientation,
        Decoded::Vector(..) => Orientation::NoTransforms,
    };
    surface.push_transform(&oriented(orientation, rect));
    let unit_size = Size::from_wh(1.0, 1.0).unwrap();
    let errors = std::cell::RefCell::new(std::collections::BTreeSet::new());
    match decoded {
        Decoded::Raster(image, _) => surface.draw_image(image.clone(), unit_size),
        Decoded::Vector(tree, filter_scale) => {
            let jpeg_images = match paint.image.as_ref() {
                Asset::Svg(svg) => Some(svg.jpeg_images()),
                _ => None,
            };
            surface.draw_svg(
                tree,
                unit_size,
                crate::svg::krilla_svg::SvgSettings {
                    jpeg_images,
                    image_errors: Some(&errors),
                    filter_scale: *filter_scale,
                    layer_masks,
                    ..Default::default()
                },
            );
        }
    }
    surface.pop();
    crate::writer::painting::pop_clips(clips, surface);
    Ok(errors.into_inner())
}

pub(super) struct ImageWriter {
    failed: HashMap<usize, String>,
    reported: HashSet<usize>,
    diagnostics: Vec<String>,
    /// Rasters by image and whether viewers interpolate them.
    images: HashMap<(usize, bool), Image>,
    /// Those rasters enlarged by whole multiples, as `pixelated` draws them.
    enlargements: HashMap<(usize, (u32, u32)), Image>,
    vectors: HashMap<(usize, u32, u32), Arc<crate::svg::usvg::Tree>>,
    opaque: HashMap<usize, bool>,
    /// The density images composited in pixels resolve at.
    pub(super) pixels_per_point: f32,
    /// The page points one unit of the current drawing space spans, which renderings drawn as
    /// images resolve their raster effects at.
    pub(super) scale: f32,
    /// The renderings of the elements `element()` images show, from the layout being written.
    pub(super) elements: HashMap<crate::dom::ElementId, Arc<crate::layout::ViewportPaint>>,
    /// The `mask` elements drawn on the current page, at every level of nesting.
    pub(super) mask_element_draws: usize,
}
impl ImageWriter {
    pub(super) fn new(pixels_per_point: f32) -> Self {
        Self {
            failed: HashMap::new(),
            reported: HashSet::new(),
            diagnostics: Vec::new(),
            images: HashMap::new(),
            enlargements: HashMap::new(),
            vectors: HashMap::new(),
            opaque: HashMap::new(),
            pixels_per_point,
            scale: 1.0,
            mask_element_draws: 0,
            elements: HashMap::new(),
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
            Asset::Viewport(view) => self.draw_rendering(paint, &view.clone(), viewport, surface),
            Asset::Element(element) => {
                let rendering = self
                    .elements
                    .get(&element.element)
                    .cloned()
                    .ok_or(PdfError::MissingElementRendering)?;
                self.draw_rendering(paint, &rendering, viewport, surface)
            }
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
        if let Asset::Svg(svg) = paint.image.as_ref()
            && svg.has_layer_masks()
        {
            return Err(PdfError::DetachedSvgMasks);
        }
        let Some(mut decoded) = self.decode(paint)? else {
            return Ok(());
        };
        if let Decoded::Raster(image, orientation) = &mut decoded {
            *image = self.enlarged(paint, image, *orientation, surface);
        }
        let errors = draw_placed(paint, &decoded, viewport, surface, None)?;
        self.report_svg_errors(paint, errors);
        Ok(())
    }

    /// `image`, the raster of `paint`, enlarged as `pixelated` enlarges it for the device
    /// pixels the paint covers on `surface`.
    fn enlarged(
        &mut self,
        paint: &ImagePaint,
        image: &Image,
        orientation: Orientation,
        surface: &Surface<'_>,
    ) -> Image {
        let density = self.pixels_per_point
            * self.scale
            * crate::writer::painting::transform_scale(surface.cur_transform());
        let painted = [paint.rect.width, paint.rect.height].map(|extent| extent * density);
        // The stored pixels' axes, which a quarter turn swaps.
        let target = if crate::images::transposes(orientation) {
            [painted[1], painted[0]]
        } else {
            painted
        };
        let Some(multiples) = paint.rendering.nearest_multiples(image.size(), target) else {
            return image.clone();
        };
        let key = (Arc::as_ptr(&paint.image) as usize, multiples);
        if let Some(enlarged) = self.enlargements.get(&key) {
            return enlarged.clone();
        }
        let enlarged = image.with_nearest_multiples(multiples);
        let enlarged = match enlarged.validate() {
            Ok(()) => enlarged,
            Err(error) => {
                self.diagnostics.push(error);
                image.clone()
            }
        };
        self.enlargements.insert(key, enlarged.clone());
        enlarged
    }

    /// Decodes `paint`'s image once, or its SVG tree at the paint's size; `None` when it fails
    /// to decode, which is reported.
    pub(super) fn decode(&mut self, paint: &ImagePaint) -> Result<Option<Decoded>, PdfError> {
        let key = Arc::as_ptr(&paint.image) as usize;
        let interpolate = paint.rendering.interpolates();
        let source_size = [paint.rect.width, paint.rect.height].map(|v| v / paint.source_scale);
        let vector_key = (key, source_size[0].to_bits(), source_size[1].to_bits());
        if self.failed.contains_key(&key) {
            self.report_failure(key);
            return Ok(None);
        }
        if let std::collections::hash_map::Entry::Vacant(entry) =
            self.images.entry((key, interpolate))
        {
            let loaded = match paint.image.as_ref() {
                Asset::Raster(raster) => {
                    let data = raster.bytes.clone().into();
                    Some(match raster.format {
                        RasterFormat::Png => Image::from_png(data, interpolate),
                        RasterFormat::Jpeg => Image::from_jpeg(data, interpolate),
                        RasterFormat::Gif => Image::from_gif(data, interpolate),
                        RasterFormat::WebP => Image::from_webp(data, interpolate),
                    })
                }
                Asset::Pixels(pixels) => Some(
                    Image::from_custom(crate::writer::pixels::Pixels(pixels.clone()), interpolate)
                        .map(Image::with_jpeg_compression),
                ),
                Asset::Svg(_)
                | Asset::Transparent
                | Asset::Frame(..)
                | Asset::Viewport(_)
                | Asset::Widget(_)
                | Asset::Element(_)
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
                        return Ok(None);
                    }
                }
            }
        }
        Ok(Some(match paint.image.as_ref() {
            Asset::Raster(raster) => {
                Decoded::Raster(self.images[&(key, interpolate)].clone(), raster.orientation)
            }
            Asset::Pixels(_) => Decoded::Raster(
                self.images[&(key, interpolate)].clone(),
                Orientation::NoTransforms,
            ),
            Asset::Svg(svg) => {
                let tree = match self.vectors.entry(vector_key) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.get().clone(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        let size = source_size.map(|v| v / crate::style::POINTS_PER_PIXEL);
                        entry
                            .insert(Arc::new(svg.tree(size).map_err(PdfError::Svg)?))
                            .clone()
                    }
                };
                Decoded::Vector(tree, self.pixels_per_point)
            }
            Asset::Transparent
            | Asset::Frame(..)
            | Asset::Viewport(_)
            | Asset::Widget(_)
            | Asset::Element(_)
            | Asset::View(_)
            | Asset::Generated(_) => {
                unreachable!("draw dispatches only decoded images here")
            }
        }))
    }

    /// Reports an SVG image's recoverable decoding failures once.
    pub(super) fn report_svg_errors(
        &mut self,
        paint: &ImagePaint,
        errors: std::collections::BTreeSet<String>,
    ) {
        if !errors.is_empty() && self.reported.insert(Arc::as_ptr(&paint.image) as usize) {
            self.diagnostics.extend(errors);
        }
    }

    /// Draws a laid-out rendering filling the paint rectangle.
    fn draw_rendering(
        &mut self,
        paint: &ImagePaint,
        view: &crate::layout::ViewportPaint,
        viewport: crate::graphics::Rect,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let space = crate::writer::display::Space {
            viewport,
            scale: self.scale,
        };
        crate::writer::display::draw_viewport(paint, view, self, space, surface)
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
                rendering: paint.rendering,
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
