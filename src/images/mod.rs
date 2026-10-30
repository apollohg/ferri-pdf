//! Image assets, intrinsic dimensions, and per-document resource reuse.
mod avif;
mod density;
mod heif;
mod pixels;
mod svg;
mod svg_fonts;
mod view;
pub(crate) use svg_fonts::SvgFonts;
use view::content_view;
pub(crate) use view::{ImageView, InvalidFragment, content_image, turns_and_flip};
pub(crate) mod widget;
use crate::fonts::{FontBook, FontNamespace};
pub(crate) use crate::paint::NaturalDimensions;
use crate::resources::{ResourceError, ResourceResolver};
pub(crate) use image::metadata::Orientation;
use image::{ImageDecoder, ImageFormat, ImageReader};
pub(crate) use pixels::{PixelDepth, PixelImage};
use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    sync::Arc,
};
pub(crate) use svg::{LayerMaskTable, SvgImage, options as svg_options};
impl crate::paint::ImageInfo for Image {
    fn natural_size(&self) -> NaturalDimensions {
        Image::natural_size(self)
    }
    fn is_raster(&self) -> bool {
        match self {
            Self::Raster(_) | Self::Pixels(_) => true,
            Self::View(view) => crate::paint::ImageInfo::is_raster(&*view.image),
            Self::Svg(_)
            | Self::Generated(_)
            | Self::Transparent
            | Self::Frame(..)
            | Self::Viewport(_)
            | Self::Widget(_)
            | Self::Element(_) => false,
        }
    }
}

/// An element as an image (CSS Images 4 §2.7): its rendering over a transparent canvas the
/// size of its decorated bounding box, which the document's final layout records.
#[derive(Debug)]
pub struct ElementImage {
    pub(crate) element: crate::dom::ElementId,
    /// The decorated bounding box's size in CSS pixels.
    pub(crate) size: [f32; 2],
}

const MAX_DECODED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// The size of replaced content with no natural dimensions (CSS Images 3 §4.1; HTML §4.8.11
/// for video).
pub(crate) const DEFAULT_OBJECT_SIZE: [f32; 2] = [300.0, 150.0];
/// Image MIME types the decoders render.
pub(crate) const SUPPORTED_TYPES: &[&str] = &[
    "image/png",
    "image/apng",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/heic",
    "image/heif",
    crate::resources::sniffing::SVG,
];
const MAX_BYTES_PER_PIXEL: u64 = 8;

#[derive(Clone, Copy, Debug)]
pub enum RasterFormat {
    Png,
    Jpeg,
    Gif,
    WebP,
}

fn raster_reader(bytes: &[u8], format: ImageFormat) -> ImageReader<Cursor<&[u8]>> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    reader
}

#[derive(Debug)]
pub struct RasterImage {
    pub bytes: Arc<Vec<u8>>,
    pub format: RasterFormat,
    pub width: u32,
    pub height: u32,
    pub orientation: Orientation,
    /// Image pixels per CSS pixel along the stored rows and columns, if the image records
    /// its resolution.
    pub density: Option<[f32; 2]>,
}
impl RasterImage {
    fn image_format(&self) -> ImageFormat {
        match self.format {
            RasterFormat::Png => ImageFormat::Png,
            RasterFormat::Jpeg => ImageFormat::Jpeg,
            RasterFormat::Gif => ImageFormat::Gif,
            RasterFormat::WebP => ImageFormat::WebP,
        }
    }
    fn validate_samples(&self) -> Result<(), ImageError> {
        let decoder = raster_reader(&self.bytes, self.image_format()).into_decoder()?;
        let bytes = decoder.total_bytes();
        if bytes > MAX_DECODED_BYTES {
            return Err(ImageError::Dimensions {
                width: self.width,
                height: self.height,
            });
        }
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(bytes as usize)
            .map_err(image::ImageError::from)?;
        samples.resize(bytes as usize, 0);
        decoder.read_image(&mut samples)?;
        Ok(())
    }

    /// The first frame's alpha samples in display orientation, row by row, with its width.
    pub(crate) fn alpha(&self) -> Result<(u32, Vec<u8>), ImageError> {
        let mut image = raster_reader(&self.bytes, self.image_format()).decode()?;
        image.apply_orientation(self.orientation);
        let rgba = image.into_rgba8();
        Ok((
            rgba.width(),
            rgba.pixels()
                .map(|pixel| pixel.0[pixels::RGB_CHANNELS])
                .collect(),
        ))
    }

    pub fn pixel_size(&self) -> [f32; 2] {
        if view::transposes(self.orientation) {
            [self.height as f32, self.width as f32]
        } else {
            [self.width as f32, self.height as f32]
        }
    }
    fn parse(bytes: Vec<u8>) -> Result<Self, ImageError> {
        let format = image::guess_format(&bytes)?;
        let raster_format = match format {
            ImageFormat::Png => RasterFormat::Png,
            ImageFormat::Jpeg => RasterFormat::Jpeg,
            ImageFormat::Gif => RasterFormat::Gif,
            ImageFormat::WebP => RasterFormat::WebP,
            _ => return Err(ImageError::UnsupportedFormat(format)),
        };
        let mut decoder = raster_reader(&bytes, format).into_decoder()?;
        let (width, height) = decoder.dimensions();
        if width == 0 || height == 0 || decoder.total_bytes() > MAX_DECODED_BYTES {
            return Err(ImageError::Dimensions { width, height });
        }
        let exif = decoder.exif_metadata()?;
        let orientation = decoder.orientation()?;
        drop(decoder);
        let density = density::recorded(&bytes, raster_format, exif.as_deref());
        Ok(Self {
            bytes: Arc::new(bytes),
            format: raster_format,
            width,
            height,
            orientation,
            density,
        })
    }
}

#[derive(Debug)]
pub enum Image {
    Raster(RasterImage),
    Svg(SvgImage),
    Pixels(Arc<PixelImage>),
    /// Transparent black with no natural dimensions, as a `video` with no poster frame
    /// represents (HTML §4.8.11).
    Transparent,
    /// The document of a nested navigable, laid out once its viewport size is known, and its
    /// embedding element's used colour scheme.
    Frame(Arc<crate::document::Frame>, crate::style::ColorScheme),
    /// A nested navigable's document laid out in its viewport.
    Viewport(Arc<crate::layout::ViewportPaint>),
    /// A widget's native appearance, drawn once its box's used size is known.
    Widget(Arc<widget::Widget>),
    /// Part of another image, re-oriented, as an `<image>` uses it.
    View(ImageView),
    /// A generated `<image>` that `content` places, such as a gradient or `cross-fade()`;
    /// an invalid image replacing an element's contents has zero natural size (CSS Content 3
    /// §1).
    Generated(Arc<crate::graphics::BackgroundImage>),
    Element(ElementImage),
}
impl Image {
    pub fn natural_size(&self) -> NaturalDimensions {
        match self {
            Self::Svg(image) => image.natural_size(),
            Self::Raster(_) | Self::Pixels(_) => {
                let [width, height] = self.pixel_size(0.0);
                NaturalDimensions {
                    width: Some(width),
                    height: Some(height),
                    ratio: Some(width / height),
                }
            }
            Self::Viewport(view) => {
                let [width, height] = [view.area.width, view.area.height]
                    .map(|value| value / crate::style::POINTS_PER_PIXEL);
                NaturalDimensions {
                    width: Some(width),
                    height: Some(height),
                    ratio: (height > 0.0).then_some(width / height),
                }
            }
            Self::Transparent | Self::Frame(..) => NaturalDimensions::default(),
            Self::Widget(widget) => widget.natural_size(),
            Self::View(view) => view.natural_size(),
            Self::Generated(image) => generated_natural_size(image),
            Self::Element(element) => {
                let [width, height] = element.size;
                NaturalDimensions {
                    width: Some(width),
                    height: Some(height),
                    ratio: (height > 0.0).then_some(width / height),
                }
            }
        }
    }
    /// Whether every pixel is opaque; vector images, and images that fail to decode, are not
    /// known to be.
    pub(crate) fn opaque(&self) -> bool {
        match self {
            Self::Raster(raster) => {
                raster_reader(&raster.bytes, raster.image_format())
                    .into_decoder()
                    .is_ok_and(|decoder| !decoder.color_type().has_alpha())
                    || raster
                        .alpha()
                        .is_ok_and(|(_, alpha)| alpha.iter().all(|sample| *sample == u8::MAX))
            }
            Self::Pixels(pixels) => pixels
                .alpha()
                .is_none_or(|alpha| alpha.iter().all(|sample| *sample == u8::MAX)),
            Self::View(view) => {
                let [width, height] = view.image.pixel_size(0.0);
                view.region.is_none_or(|region| {
                    region.x >= 0.0
                        && region.y >= 0.0
                        && region.x + region.width <= width
                        && region.y + region.height <= height
                }) && view.image.opaque()
            }
            Self::Svg(_)
            | Self::Generated(_)
            | Self::Transparent
            | Self::Frame(..)
            | Self::Viewport(_)
            | Self::Widget(_)
            | Self::Element(_) => false,
        }
    }
    pub fn pixel_size(&self, available_width: f32) -> [f32; 2] {
        match self {
            Self::Raster(image) => image.pixel_size(),
            Self::Svg(image) => image.pixel_size(available_width),
            Self::Pixels(image) => [image.width as f32, image.height as f32],
            Self::Viewport(view) => [view.area.width, view.area.height]
                .map(|value| value / crate::style::POINTS_PER_PIXEL),
            Self::Transparent | Self::Frame(..) => DEFAULT_OBJECT_SIZE,
            Self::Widget(widget) => widget.natural,
            Self::Element(element) => element.size,
            Self::View(view) => view.pixel_size(available_width),
            Self::Generated(image) => crate::paint::image_size(
                &crate::paint::BackgroundSize::default(),
                generated_natural_size(image),
                DEFAULT_OBJECT_SIZE,
            ),
        }
    }
    fn parse(
        resource: crate::resources::Resource,
        resolver: &dyn ResourceResolver,
        depth: usize,
        cache: &mut ImageCache,
        namespace: FontNamespace,
    ) -> Result<Self, ImageError> {
        if avif::matches(&resource.bytes) {
            return avif::decode(&resource.bytes).map(|image| Self::Pixels(Arc::new(image)));
        }
        if heif::matches(&resource.bytes) {
            return heif::decode(&resource.bytes).map(|image| Self::Pixels(Arc::new(image)));
        }
        if image::guess_format(&resource.bytes).is_ok() {
            RasterImage::parse(resource.bytes).map(Self::Raster)
        } else {
            SvgImage::parse(resource, resolver, depth, cache, namespace).map(Self::Svg)
        }
    }
}

/// The natural size in CSS pixels of a generated image, whose `cross-fade()` arguments the
/// default object size sizes.
fn generated_natural_size(image: &crate::graphics::BackgroundImage) -> NaturalDimensions {
    if matches!(image, crate::graphics::BackgroundImage::Invalid) {
        return NaturalDimensions {
            width: Some(0.0),
            height: Some(0.0),
            ratio: None,
        };
    }
    let area = DEFAULT_OBJECT_SIZE.map(|extent| extent * crate::paint::POINTS_PER_PIXEL);
    let natural = crate::paint::Source::of(image)
        .ok()
        .flatten()
        .map(|source| source.natural(area))
        .unwrap_or_default();
    NaturalDimensions {
        width: natural
            .width
            .map(|width| width / crate::paint::POINTS_PER_PIXEL),
        height: natural
            .height
            .map(|height| height / crate::paint::POINTS_PER_PIXEL),
        ratio: natural.ratio,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("SVG font: {0}")]
    Font(#[from] crate::fonts::FontError),
    #[error("invalid AVIF container: {0}")]
    AvifContainer(#[from] zenavif_parse::Error),
    #[error("invalid AV1 image: {0}")]
    AvifDecode(#[from] avif_decode::Error),
    #[error("invalid AVIF image: {0}")]
    Avif(&'static str),
    #[error("invalid HEIF image: {0}")]
    Heif(#[from] heic_decoder::HeifError),
    #[error("decoded RGB image has a non-RGB ICC profile")]
    NonRgbProfile,
    #[error("image color profile: {0}")]
    ColorProfile(#[from] moxcms::CmsError),
    #[error("unsupported image color description: primaries {primaries}, transfer {transfer}")]
    ColorDescription { primaries: u16, transfer: u16 },
    #[error("encoding nested image: {0}")]
    PngEncoding(#[from] png::EncodingError),
    #[error("external SVG symbol references are not supported: {0}")]
    ExternalSvgSymbol(String),
    #[error("SVG is not UTF-8: {0}")]
    SvgEncoding(#[source] std::string::FromUtf8Error),
    #[error("invalid SVG XML: {0}")]
    SvgXml(#[source] roxmltree::Error),
    #[error("image XML root is not an SVG element")]
    SvgRoot,
    #[error("invalid SVG: {0}")]
    Svg(#[source] crate::svg::usvg::Error),
    #[error("SVG viewport must have positive finite dimensions")]
    SvgSize,
    #[error("nested SVG images exceed resource depth {0}")]
    SvgResourceDepth(usize),
    #[error("{0}")]
    Resource(#[from] ResourceError),
    #[error("invalid image: {0}")]
    Decode(#[from] image::ImageError),
    #[error("unsupported image format: {0:?}")]
    UnsupportedFormat(ImageFormat),
    #[error("image dimensions exceed the decoding budget: {width} by {height}")]
    Dimensions { width: u32, height: u32 },
    #[error("{0}")]
    Fragment(#[from] InvalidFragment),
}

#[derive(Debug, thiserror::Error)]
#[error("image {reference} is unavailable: {error}")]
pub struct ImageFailure {
    pub base: crate::resources::ResourceBase,
    pub reference: String,
    #[source]
    pub error: ImageError,
}

/// The SVG documents that references into other documents name, by absolute URL without
/// fragment; `None` for one that is unavailable or not text. Formatting loads those of box
/// references, which the SVG definitions read when painting.
pub(crate) type LinkedDocuments =
    Arc<std::sync::Mutex<HashMap<String, Option<crate::svg::usvg::LinkedDocument>>>>;

#[derive(Default)]
pub(crate) struct ImageCache {
    validated_markers: HashSet<usize>,
    failed: HashSet<(FontNamespace, crate::resources::ResourceBase, String)>,
    failures: Vec<ImageFailure>,
    fonts: SvgFonts,
    svg_sources: HashMap<crate::resources::ResourceBase, HashMap<String, Arc<svg::SvgSource>>>,
    linked: LinkedDocuments,
    images: HashMap<
        FontNamespace,
        HashMap<crate::resources::ResourceBase, HashMap<String, Arc<Image>>>,
    >,
}
impl ImageCache {
    pub(crate) fn with_fonts(fonts: FontBook) -> Self {
        Self {
            fonts: SvgFonts::new(Arc::new(fonts)),
            validated_markers: HashSet::new(),
            failed: HashSet::new(),
            failures: Vec::new(),
            svg_sources: HashMap::new(),
            linked: LinkedDocuments::default(),
            images: HashMap::new(),
        }
    }
    /// The SVG documents that references into other documents name.
    pub(crate) fn linked_documents(&self) -> LinkedDocuments {
        self.linked.clone()
    }
    /// The SVG document whose elements `reference` names (SVG 2 §5.3.2), loaded once. An
    /// unavailable resource, or one that is not UTF-8 text, names no element, so each
    /// property's fallback for an invalid reference applies; a resource that the resolver's
    /// policy denies or that exceeds its limits is an error.
    pub(crate) fn linked_document(
        &self,
        reference: &str,
        base: &crate::resources::ResourceBase,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<crate::svg::usvg::LinkedDocument>, ImageError> {
        let mut requested = base.resolve(reference)?;
        requested.set_fragment(None);
        let key = requested.to_string();
        if let Some(document) = self
            .linked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
        {
            return Ok(document.clone());
        }
        let document = match resolver.resolve(reference, base) {
            Ok(resource) => {
                let mut url = match resource.base {
                    crate::resources::ResourceBase::Url(url) => url,
                    crate::resources::ResourceBase::Directory(_) => requested,
                };
                url.set_fragment(None);
                String::from_utf8(resource.bytes).ok().map(|text| {
                    crate::svg::usvg::LinkedDocument {
                        url,
                        text: text.into(),
                    }
                })
            }
            Err(error) if error.is_unavailable() => None,
            Err(error) => return Err(error.into()),
        };
        self.linked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, document.clone());
        Ok(document)
    }
    #[cfg(test)]
    pub(crate) fn load(
        &mut self,
        reference: &str,
        base: &crate::resources::ResourceBase,
        resolver: &dyn ResourceResolver,
    ) -> Result<Arc<Image>, ImageError> {
        self.load_in_namespace(reference, base, resolver, FontNamespace::ROOT)
    }
    pub(crate) fn load_in_namespace(
        &mut self,
        reference: &str,
        base: &crate::resources::ResourceBase,
        resolver: &dyn ResourceResolver,
        namespace: FontNamespace,
    ) -> Result<Arc<Image>, ImageError> {
        self.load_nested(reference, base, resolver, 0, namespace)
    }
    /// Load an HTML image, retaining a diagnostic instead of aborting on resource failure.
    pub(crate) fn load_optional(
        &mut self,
        reference: &str,
        base: &crate::resources::ResourceBase,
        resolver: &dyn ResourceResolver,
        namespace: FontNamespace,
    ) -> Option<Arc<Image>> {
        let key = (namespace, base.clone(), reference.to_owned());
        if self.failed.contains(&key) {
            return None;
        }
        match self.load_in_namespace(reference, base, resolver, namespace) {
            Ok(image) => Some(image),
            Err(error) => {
                self.failed.insert(key);
                self.failures.push(ImageFailure {
                    base: base.clone(),
                    reference: reference.to_owned(),
                    error,
                });
                None
            }
        }
    }
    /// An image whose fallback is chosen before layout, as a list marker's or a float
    /// shape's is: it fails here on corrupt pixels too, retaining a diagnostic.
    pub(crate) fn load_decoded(
        &mut self,
        reference: &str,
        base: &crate::resources::ResourceBase,
        resolver: &dyn ResourceResolver,
        namespace: FontNamespace,
    ) -> Option<Arc<Image>> {
        let image = self.load_optional(reference, base, resolver, namespace)?;
        let key = Arc::as_ptr(&image) as usize;
        if !self.validated_markers.contains(&key) {
            if let Image::Raster(raster) = image.as_ref()
                && let Err(error) = raster.validate_samples()
            {
                self.failed
                    .insert((namespace, base.clone(), reference.to_owned()));
                self.failures.push(ImageFailure {
                    base: base.clone(),
                    reference: reference.to_owned(),
                    error,
                });
                return None;
            }
            self.validated_markers.insert(key);
        }
        Some(image)
    }
    /// `image` as `request` uses it on the element `style` belongs to (see [`view::view`]), or `None` with a
    /// diagnostic when `image()` names a fragment it cannot use.
    pub(crate) fn view(
        &mut self,
        image: Arc<Image>,
        request: &crate::paint::ImageRequest<'_>,
        base: &crate::resources::ResourceBase,
        style: &crate::style::Style,
    ) -> Option<Arc<Image>> {
        self.viewed(view::view(image, request, style), request, base)
    }
    /// `image` as `request` uses it in `content` (see [`view::content_view`]), likewise.
    pub(crate) fn content_view(
        &mut self,
        image: Arc<Image>,
        request: &crate::paint::ImageRequest<'_>,
        base: &crate::resources::ResourceBase,
        style: &crate::style::Style,
        resolution: Option<f32>,
    ) -> Option<Arc<Image>> {
        self.viewed(
            content_view(image, request, style, resolution),
            request,
            base,
        )
    }
    fn viewed(
        &mut self,
        viewed: Result<Arc<Image>, InvalidFragment>,
        request: &crate::paint::ImageRequest<'_>,
        base: &crate::resources::ResourceBase,
    ) -> Option<Arc<Image>> {
        viewed
            .map_err(|error| {
                self.failures.push(ImageFailure {
                    base: base.clone(),
                    reference: request.url.to_owned(),
                    error: error.into(),
                });
            })
            .ok()
    }
    /// Retains a diagnostic for an image that could not be made, which renders as invalid.
    pub(crate) fn record_failure(
        &mut self,
        base: &crate::resources::ResourceBase,
        reference: &str,
        error: ImageError,
    ) {
        self.failures.push(ImageFailure {
            base: base.clone(),
            reference: reference.to_owned(),
            error,
        });
    }
    pub(crate) fn take_failures(&mut self) -> Vec<ImageFailure> {
        std::mem::take(&mut self.failures)
    }
    /// Parses an inline `<svg>` element serialized as XML; its references resolve against `base`.
    #[cfg(test)]
    pub(crate) fn inline_svg(
        &mut self,
        xml: String,
        base: &crate::resources::ResourceBase,
        resolver: &dyn ResourceResolver,
    ) -> Result<Arc<Image>, ImageError> {
        self.inline_svg_in_namespace(xml, Default::default(), base, resolver, FontNamespace::ROOT)
    }
    /// SVG content the document styles, with the CSS mask layers its `mask` values name.
    pub(crate) fn inline_svg_in_namespace(
        &mut self,
        xml: String,
        layer_masks: LayerMaskTable,
        base: &crate::resources::ResourceBase,
        resolver: &dyn ResourceResolver,
        namespace: FontNamespace,
    ) -> Result<Arc<Image>, ImageError> {
        Ok(Arc::new(Image::Svg(SvgImage::parse_source(
            Arc::new(svg::SvgSource::new(xml, base.clone())),
            layer_masks,
            resolver,
            0,
            self,
            namespace,
        )?)))
    }
    pub(crate) fn load_nested(
        &mut self,
        reference: &str,
        base: &crate::resources::ResourceBase,
        resolver: &dyn ResourceResolver,
        depth: usize,
        namespace: FontNamespace,
    ) -> Result<Arc<Image>, ImageError> {
        if let Some(image) = self
            .images
            .get(&namespace)
            .and_then(|scopes| scopes.get(base))
            .and_then(|images| images.get(reference))
        {
            return Ok(image.clone());
        }
        if namespace != FontNamespace::ROOT
            && let Some(image) = self
                .images
                .get(&FontNamespace::ROOT)
                .and_then(|images| images.get(base))
                .and_then(|images| images.get(reference))
            && !matches!(image.as_ref(), Image::Svg(_))
        {
            return Ok(image.clone());
        }
        let source = self
            .svg_sources
            .get(base)
            .and_then(|sources| sources.get(reference))
            .cloned();
        let image = if let Some(source) = source {
            Image::Svg(SvgImage::parse_source(
                source,
                Arc::new([]),
                resolver,
                depth,
                self,
                namespace,
            )?)
        } else {
            let resource = resolver.resolve(reference, base)?;
            Image::parse(resource, resolver, depth, self, namespace)?
        };
        Ok(self.insert(reference, base, image, namespace))
    }
    /// Decodes an image resource its embedding element has already fetched, retaining a
    /// diagnostic instead of aborting when it cannot be rendered.
    pub(crate) fn decode_optional(
        &mut self,
        reference: &str,
        base: &crate::resources::ResourceBase,
        resource: crate::resources::Resource,
        resolver: &dyn ResourceResolver,
        namespace: FontNamespace,
    ) -> Option<Arc<Image>> {
        match Image::parse(resource, resolver, 0, self, namespace) {
            Ok(image) => Some(self.insert(reference, base, image, namespace)),
            Err(error) => {
                self.failures.push(ImageFailure {
                    base: base.clone(),
                    reference: reference.to_owned(),
                    error,
                });
                None
            }
        }
    }
    fn insert(
        &mut self,
        reference: &str,
        base: &crate::resources::ResourceBase,
        image: Image,
        namespace: FontNamespace,
    ) -> Arc<Image> {
        let image = Arc::new(image);
        let namespace = match image.as_ref() {
            Image::Svg(svg) => {
                self.svg_sources
                    .entry(base.clone())
                    .or_default()
                    .insert(reference.to_owned(), svg.source());
                namespace
            }
            Image::Raster(_) | Image::Pixels(_) => FontNamespace::ROOT,
            Image::Transparent
            | Image::Frame(..)
            | Image::Viewport(_)
            | Image::Widget(_)
            | Image::View(_)
            | Image::Generated(_)
            | Image::Element(_) => {
                unreachable!("image resources parse to decoded images")
            }
        };
        self.images
            .entry(namespace)
            .or_default()
            .entry(base.clone())
            .or_default()
            .insert(reference.to_owned(), image.clone());
        image
    }
}

#[cfg(test)]
mod tests;
