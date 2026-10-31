use crate::fonts::{DEFAULT_FAMILY, FontNamespace};
use crate::images::svg_fonts::SvgFonts;
use crate::images::{Image, ImageCache, ImageError, RasterFormat};
use crate::resources::{Resource, ResourceBase, ResourceResolver};
use crate::svg::usvg::{ImageKind, LinkedDocument, Nesting, ParsedSvg, Tree};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::Arc,
};

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";
const DEFAULT_FONT_SIZE: f64 = 16.0;

#[derive(Debug)]
pub(crate) struct SvgSource {
    xml: String,
    base: ResourceBase,
}
impl SvgSource {
    pub(crate) fn new(xml: String, base: ResourceBase) -> Self {
        Self { xml, base }
    }
}

#[derive(Debug)]
pub struct SvgImage {
    source: Arc<SvgSource>,
    fonts: SvgFonts,
    resources: HashMap<String, ImageKind>,
    /// The documents that references into other documents name, by reference.
    documents: HashMap<String, Option<LinkedDocument>>,
    jpeg_images: HashSet<usize>,
    width: Option<f32>,
    height: Option<f32>,
    ratio: Option<f32>,
    initial_size: [f32; 2],
    initial_tree: Box<Tree>,
    /// What the document attaches to SVG content it styles.
    styling: Box<crate::svg::DocumentStyling>,
    /// The document styles the content: an `svg` element in it, not an image it references.
    styled: bool,
}

impl SvgImage {
    pub(crate) fn parse(
        resource: Resource,
        resolver: &dyn ResourceResolver,
        nesting: Nesting,
        cache: &mut ImageCache,
        namespace: FontNamespace,
    ) -> Result<Self, ImageError> {
        let xml = String::from_utf8(resource.bytes).map_err(ImageError::SvgEncoding)?;
        Self::parse_source(
            Arc::new(SvgSource {
                xml,
                base: resource.base,
            }),
            crate::svg::DocumentStyling::default(),
            false,
            resolver,
            nesting,
            cache,
            namespace,
        )
    }

    /// `styling` is what the document attaches to SVG content it styles, which `styled`
    /// tells it is.
    pub(crate) fn parse_source(
        source: Arc<SvgSource>,
        styling: crate::svg::DocumentStyling,
        styled: bool,
        resolver: &dyn ResourceResolver,
        nesting: Nesting,
        cache: &mut ImageCache,
        namespace: FontNamespace,
    ) -> Result<Self, ImageError> {
        let xml = &source.xml;
        let document = roxmltree::Document::parse(xml).map_err(ImageError::SvgXml)?;
        let root = document.root_element();
        if !root.has_tag_name((SVG_NAMESPACE, "svg")) {
            return Err(ImageError::SvgRoot);
        }
        let mut parsed = ParsedSvg::parse(&document, None).map_err(ImageError::Svg)?;
        let mut options = options(crate::images::DEFAULT_OBJECT_SIZE)?;
        options.styling = styling.clone();
        options.nesting = nesting;
        let fonts = cache.fonts.in_namespace(namespace);
        let provider = fonts.provider(Some(resolver));
        options.fontdb = fonts.database();
        let crate::images::NaturalDimensions {
            width,
            height,
            ratio,
        } = parsed
            .natural_dimensions(&options, Some(&provider))
            .map_err(ImageError::Svg)?;
        // A `use` element referencing another document is in error and not rendered (SVG 2
        // §5.6.1); the tree builder resolves references within this document only.
        for node in root
            .descendants()
            .filter(|node| node.has_tag_name((SVG_NAMESPACE, "use")))
        {
            if let Some(href) = node
                .attribute("href")
                .or_else(|| node.attribute((XLINK_NAMESPACE, "href")))
                && !href.starts_with('#')
            {
                cache.record_failure(
                    &source.base,
                    href,
                    ImageError::ExternalSvgSymbol(href.into()),
                );
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
        // An image that cannot be loaded or decoded leaves its element in error, which is not
        // rendered (SVG 2 §12.3), with a diagnostic.
        let resources = RefCell::new(HashMap::<String, Option<ImageKind>>::new());
        let documents = RefCell::new(HashMap::<String, Option<LinkedDocument>>::new());
        let jpeg_images = RefCell::new(HashSet::new());
        let cache = RefCell::new(cache);
        // An image converts where it draws, nesting in the conversion around it.
        let resolve = |href: &str, _: &crate::svg::usvg::Options<'_>, conversion| {
            if href.starts_with('#') {
                return None;
            }
            if let Some(kind) = resources.borrow().get(href) {
                return kind.clone();
            }
            let loaded =
                cache
                    .borrow_mut()
                    .load_nested(href, &source.base, resolver, conversion, namespace);
            let loaded = loaded
                .and_then(|image| image_kind(&image, &mut jpeg_images.borrow_mut()))
                .map_err(|error| cache.borrow_mut().record_failure(&source.base, href, error))
                .ok();
            resources
                .borrow_mut()
                .insert(href.to_owned(), loaded.clone());
            loaded
        };
        options.default_size = crate::svg::usvg::Size::from_wh(initial_size[0], initial_size[1])
            .ok_or(ImageError::SvgSize)?;
        let resolve_document = |reference: &str| {
            let document = cache
                .borrow()
                .linked_document(reference, &source.base, resolver);
            let document = document.unwrap_or_else(|error| {
                cache
                    .borrow_mut()
                    .record_failure(&source.base, reference, error);
                None
            });
            documents
                .borrow_mut()
                .insert(reference.to_owned(), document.clone());
            document
        };
        let linked = parsed.link_documents(&resolve_document);
        let initial_tree = linked.and_then(|()| {
            parsed.to_tree(
                &options,
                Some(options.default_size),
                Some(&resolve),
                Some(&provider),
            )
        });
        let fonts = provider.finish()?;
        let initial_tree = initial_tree.map_err(ImageError::Svg)?;
        let mut limits = parsed.limits();
        limits.truncated |= initial_tree.truncated;
        cache
            .into_inner()
            .record_limits(limits, initial_tree.limited);
        let resources = resources
            .into_inner()
            .into_iter()
            .filter_map(|(href, kind)| Some((href, kind?)))
            .collect();
        Ok(Self {
            source,
            fonts,
            resources,
            documents: documents.into_inner(),
            jpeg_images: jpeg_images.into_inner(),
            width,
            height,
            ratio,
            initial_size,
            initial_tree: Box::new(initial_tree),
            styling: Box::new(styling),
            styled,
        })
    }

    /// Whether the document styles the content: an `svg` element in it, whose content takes
    /// part in the document's 3D rendering contexts.
    pub(crate) fn is_styled(&self) -> bool {
        self.styled
    }

    /// Whether its SVG content has CSS mask layers, which paint through the display list.
    pub(crate) fn has_layer_masks(&self) -> bool {
        !self.styling.layer_masks.is_empty()
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
        options.styling = (*self.styling).clone();
        options.nesting = self.initial_tree.nesting;
        let provider = self.fonts.provider(None);
        let result = Tree::from_str_with_resolvers(
            &self.source.xml,
            &options,
            Some(options.default_size),
            &|href, _, _| self.resources.get(href).cloned(),
            &provider,
            &|reference| self.documents.get(reference).cloned().flatten(),
        );
        provider.finish()?;
        result.map_err(ImageError::Svg)
    }
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

pub(crate) fn options(size: [f32; 2]) -> Result<crate::svg::usvg::Options<'static>, ImageError> {
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
        Image::Transparent
        | Image::Frame(..)
        | Image::Viewport(_)
        | Image::Widget(_)
        | Image::View(_)
        | Image::Generated(_)
        | Image::Element(_) => {
            unreachable!("SVG image references load only image resources")
        }
    })
}
