use crate::fonts::{DEFAULT_FAMILY, FontNamespace};
use crate::images::svg_fonts::SvgFonts;
use crate::images::{Image, ImageCache, ImageError, RasterFormat};
use crate::resources::{Resource, ResourceBase, ResourceResolver};
use crate::svg::usvg::{ImageKind, Tree};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::Arc,
};
use svgtypes::{Length, LengthUnit, ViewBox};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
const PIXELS_PER_INCH: f64 = 96.0;
const POINTS_PER_INCH: f64 = 72.0;
const MILLIMETRES_PER_INCH: f64 = 25.4;
const CENTIMETRES_PER_INCH: f64 = 2.54;
const POINTS_PER_PICA: f64 = 12.0;
const DEFAULT_FONT_SIZE: f64 = 16.0;
const MAX_RESOURCE_DEPTH: usize = 32;

#[derive(Debug)]
pub(crate) struct SvgSource {
    xml: String,
    base: ResourceBase,
}

#[derive(Debug)]
pub struct SvgImage {
    source: Arc<SvgSource>,
    fonts: SvgFonts,
    resources: HashMap<String, ImageKind>,
    motion_resources: HashMap<String, Option<Arc<str>>>,
    jpeg_images: HashSet<usize>,
    width: Option<f32>,
    height: Option<f32>,
    ratio: Option<f32>,
    initial_size: [f32; 2],
    initial_tree: Box<Tree>,
}

impl SvgImage {
    pub(crate) fn parse(
        resource: Resource,
        resolver: &dyn ResourceResolver,
        depth: usize,
        cache: &mut ImageCache,
        namespace: FontNamespace,
    ) -> Result<Self, ImageError> {
        validate_depth(depth)?;
        let xml = String::from_utf8(resource.bytes).map_err(ImageError::SvgEncoding)?;
        Self::parse_source(
            Arc::new(SvgSource {
                xml,
                base: resource.base,
            }),
            resolver,
            depth,
            cache,
            namespace,
        )
    }

    pub(crate) fn parse_source(
        source: Arc<SvgSource>,
        resolver: &dyn ResourceResolver,
        depth: usize,
        cache: &mut ImageCache,
        namespace: FontNamespace,
    ) -> Result<Self, ImageError> {
        validate_depth(depth)?;
        let xml = &source.xml;
        let document = roxmltree::Document::parse(xml).map_err(ImageError::SvgXml)?;
        let root = document.root_element();
        if !root.has_tag_name((SVG_NAMESPACE, "svg")) {
            return Err(ImageError::SvgRoot);
        }
        let width = dimension(root.attribute("width"));
        let height = dimension(root.attribute("height"));
        let view_box = root
            .attribute("viewBox")
            .and_then(|v| v.parse::<ViewBox>().ok());
        let ratio = width.zip(height).map(|(w, h)| w / h).or_else(|| {
            view_box
                .filter(|v| v.w > 0.0 && v.h > 0.0)
                .map(|v| (v.w / v.h) as f32)
        });
        for node in root
            .descendants()
            .filter(|node| node.has_tag_name((SVG_NAMESPACE, "use")))
        {
            if let Some(href) = node
                .attribute("href")
                .or_else(|| node.attribute((XLINK_NAMESPACE, "href")))
                && !href.starts_with('#')
            {
                return Err(ImageError::ExternalSvgSymbol(href.into()));
            }
        }
        let initial_size = pixel_size(
            crate::images::NaturalDimensions {
                width,
                height,
                ratio,
            },
            crate::images::DEFAULT_OBJECT_SIZE[0],
        );
        let fonts = cache.fonts.in_namespace(namespace);
        let provider = fonts.provider(Some(resolver));
        let resources = RefCell::new(HashMap::<String, ImageKind>::new());
        let motion_resources = RefCell::new(HashMap::<String, Option<Arc<str>>>::new());
        let jpeg_images = RefCell::new(HashSet::new());
        let failure = RefCell::new(None::<ImageError>);
        let cache = RefCell::new(cache);
        let resolve = |href: &str, _: &crate::svg::usvg::Options<'_>| {
            if href.starts_with('#') || failure.borrow().is_some() {
                return None;
            }
            if let Some(kind) = resources.borrow().get(href) {
                return Some(kind.clone());
            }
            let loaded = cache
                .borrow_mut()
                .load_nested(href, &source.base, resolver, depth + 1, namespace)
                .and_then(|image| image_kind(&image, &mut jpeg_images.borrow_mut()));
            match loaded {
                Ok(kind) => {
                    resources.borrow_mut().insert(href.to_owned(), kind.clone());
                    Some(kind)
                }
                Err(error) => {
                    *failure.borrow_mut() = Some(error);
                    None
                }
            }
        };
        let mut options = options(initial_size)?;
        options.fontdb = fonts.database();
        let resolve_motion = |reference: &str| {
            if let Some(source) = motion_resources.borrow().get(reference) {
                return source.clone();
            }
            let document = resolver
                .resolve(reference, &source.base)
                .ok()
                .and_then(|resource| String::from_utf8(resource.bytes).ok())
                .map(Arc::<str>::from);
            motion_resources
                .borrow_mut()
                .insert(reference.to_owned(), document.clone());
            document
        };
        let initial_tree = Tree::from_str_with_resolvers(
            xml,
            &options,
            Some(options.default_size),
            &resolve,
            &provider,
            &resolve_motion,
        );
        let fonts = provider.finish()?;
        if let Some(error) = failure.into_inner() {
            return Err(error);
        }
        let initial_tree = initial_tree.map_err(ImageError::Svg)?;
        let resources = resources.into_inner();
        Ok(Self {
            source,
            fonts,
            resources,
            motion_resources: motion_resources.into_inner(),
            jpeg_images: jpeg_images.into_inner(),
            width,
            height,
            ratio,
            initial_size,
            initial_tree: Box::new(initial_tree),
        })
    }

    pub(crate) fn source(&self) -> Arc<SvgSource> {
        self.source.clone()
    }

    /// PNG resources originating from decoded photos, keyed by their shared data identity.
    pub fn jpeg_images(&self) -> &HashSet<usize> {
        &self.jpeg_images
    }

    pub fn natural_size(&self) -> crate::images::NaturalDimensions {
        crate::images::NaturalDimensions {
            width: self.width,
            height: self.height,
            ratio: self.ratio,
        }
    }
    pub fn pixel_size(&self, available_width: f32) -> [f32; 2] {
        pixel_size(self.natural_size(), available_width)
    }

    pub fn tree(&self, size: [f32; 2]) -> Result<Tree, ImageError> {
        if size == self.initial_size {
            return Ok((*self.initial_tree).clone());
        }
        let mut options = options(size)?;
        options.fontdb = self.initial_tree.fontdb().clone();
        let provider = self.fonts.provider(None);
        let result = Tree::from_str_with_resolvers(
            &self.source.xml,
            &options,
            Some(options.default_size),
            &|href, _| self.resources.get(href).cloned(),
            &provider,
            &|reference| self.motion_resources.get(reference).cloned().flatten(),
        );
        provider.finish()?;
        result.map_err(ImageError::Svg)
    }
}

fn validate_depth(depth: usize) -> Result<(), ImageError> {
    if depth > MAX_RESOURCE_DEPTH {
        return Err(ImageError::SvgResourceDepth(MAX_RESOURCE_DEPTH));
    }
    Ok(())
}

fn pixel_size(dimensions: crate::images::NaturalDimensions, available_width: f32) -> [f32; 2] {
    match (dimensions.width, dimensions.height, dimensions.ratio) {
        (Some(w), Some(h), _) => [w, h],
        (Some(w), None, Some(r)) => [w, w / r],
        (None, Some(h), Some(r)) => [h * r, h],
        (None, None, Some(r)) => [available_width, available_width / r],
        (w, h, None) => [
            w.unwrap_or(crate::images::DEFAULT_OBJECT_SIZE[0]),
            h.unwrap_or(crate::images::DEFAULT_OBJECT_SIZE[1]),
        ],
    }
}

fn options(size: [f32; 2]) -> Result<crate::svg::usvg::Options<'static>, ImageError> {
    Ok(crate::svg::usvg::Options {
        fontdb: Arc::new(fontdb::Database::new()),
        font_family: DEFAULT_FAMILY.primary_name().into(),
        font_size: DEFAULT_FONT_SIZE as f32,
        default_size: crate::svg::usvg::Size::from_wh(size[0], size[1])
            .ok_or(ImageError::SvgSize)?,
        image_href_resolver: crate::svg::usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            resolve_data: Box::new(|_, _, _| None),
        },
        ..Default::default()
    })
}

fn image_kind(image: &Image, jpeg_images: &mut HashSet<usize>) -> Result<ImageKind, ImageError> {
    Ok(match image {
        Image::Raster(raster) => match raster.format {
            RasterFormat::Png => ImageKind::PNG(raster.bytes.clone()),
            RasterFormat::Jpeg => ImageKind::JPEG(raster.bytes.clone()),
            RasterFormat::Gif => ImageKind::GIF(raster.bytes.clone()),
            RasterFormat::WebP => ImageKind::WEBP(raster.bytes.clone()),
        },
        Image::Svg(svg) => {
            jpeg_images.extend(svg.jpeg_images().iter().copied());
            ImageKind::SVG(svg.tree(svg.pixel_size(crate::images::DEFAULT_OBJECT_SIZE[0]))?)
        }
        Image::Pixels(pixels) => {
            let data = Arc::new(pixels.png()?);
            jpeg_images.insert(Arc::as_ptr(&data) as usize);
            ImageKind::PNG(data)
        }
        Image::Transparent | Image::Frame(..) | Image::Viewport(_) | Image::Widget(_) => {
            unreachable!("SVG image references load only image resources")
        }
    })
}

fn dimension(value: Option<&str>) -> Option<f32> {
    let length = value?.parse::<Length>().ok()?;
    let factor = match length.unit {
        LengthUnit::None | LengthUnit::Px => 1.0,
        LengthUnit::In => PIXELS_PER_INCH,
        LengthUnit::Cm => PIXELS_PER_INCH / CENTIMETRES_PER_INCH,
        LengthUnit::Mm => PIXELS_PER_INCH / MILLIMETRES_PER_INCH,
        LengthUnit::Pt => PIXELS_PER_INCH / POINTS_PER_INCH,
        LengthUnit::Pc => PIXELS_PER_INCH * POINTS_PER_PICA / POINTS_PER_INCH,
        LengthUnit::Em => DEFAULT_FONT_SIZE,
        LengthUnit::Ex => DEFAULT_FONT_SIZE / 2.0,
        LengthUnit::Percent => return None,
    };
    let value = (length.number * factor) as f32;
    (value > 0.0 && value.is_finite()).then_some(value)
}
