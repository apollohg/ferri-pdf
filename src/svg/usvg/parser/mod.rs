// Copyright 2018 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

mod basic_shape;
mod clippath;
mod converter;
mod filter;
mod font_units;
mod image;
mod marker;
mod mask;
mod motion;
pub(crate) use motion::motion_path;
mod options;
mod paint_server;
mod shapes;
mod style;
mod svgtree;
mod switch;
mod units;
mod use_node;

mod text;
#[cfg(test)]
pub(crate) use converter::with_node_budget;
pub(crate) use converter::{Cache, MAX_REFERENCE_DEPTH, Nesting};
pub(crate) use image::ImageHrefResolver;
pub(crate) use options::Options;
pub(crate) use svgtree::{MAX_SVG_DEPTH, MAX_SVG_NODES, TreeLimits};

pub(crate) struct ParsedSvg<'input>(svgtree::Document<'input>);

/// XML parsing for SVG documents, which may declare entities in a DTD.
pub(crate) fn xml_options<'input>() -> roxmltree::ParsingOptions<'input> {
    roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    }
}

/// An SVG document that a reference into another document names, as its caller loaded it.
#[derive(Clone, Debug)]
pub struct LinkedDocument {
    /// The document's own URL, against which its relative references resolve.
    pub url: crate::resources::Url,
    pub text: std::sync::Arc<str>,
}

impl<'input> ParsedSvg<'input> {
    pub(crate) fn parse(
        xml: &roxmltree::Document<'input>,
        stylesheet: Option<&'input str>,
    ) -> Result<Self, Error> {
        let doc = svgtree::Document::parse_tree(xml, stylesheet)?;
        if let Some(error) = &doc.style_error {
            return Err(Error::Style(error.clone()));
        }
        if let Some(limit) = doc.css_limit {
            return Err(Error::CssComponentDepthLimit { limit });
        }
        Ok(Self(doc))
    }

    /// The limits that changed the parsed content, its linked documents' included.
    pub(crate) fn limits(&self) -> TreeLimits {
        self.0.limits
    }

    pub(crate) fn natural_dimensions(
        &self,
        opt: &Options,
        font_provider: Option<&dyn crate::svg::usvg::FontProvider>,
    ) -> Result<crate::paint::NaturalDimensions, Error> {
        let root = self.0.root_element();
        let state = converter::State {
            font_provider,
            ..converter::State::new(opt, opt.default_size.to_non_zero_rect(0.0, 0.0))
        };
        let dimension = |name| {
            let crate::paint::LengthPercentage::Length(value) = units::computed_length(
                root,
                root,
                name,
                crate::svg::usvg::Units::UserSpaceOnUse,
                &state,
            )?
            else {
                return None;
            };
            (value > 0.0 && value.is_finite()).then_some(value)
        };
        let width = dimension(svgtree::AId::Width);
        let height = dimension(svgtree::AId::Height);
        let ratio = width
            .zip(height)
            .map(|(width, height)| width / height)
            .or_else(|| {
                root.parse_viewbox(root.effective_zoom())
                    .map(|view| view.width() / view.height())
            });
        if let Some(error) = state.length_error.borrow_mut().take() {
            return Err(Error::Style(error));
        }
        Ok(crate::paint::NaturalDimensions {
            width,
            height,
            ratio,
        })
    }

    /// Appends the other documents that references name, as `load` returns them.
    pub(crate) fn link_documents(
        &mut self,
        load: &dyn Fn(&str) -> Option<LinkedDocument>,
    ) -> Result<(), Error> {
        self.0.link_documents(load)
    }

    pub(crate) fn to_tree(
        &self,
        opt: &Options,
        viewport: Option<crate::svg::usvg::Size>,
        images: Option<&dyn Fn(&str, &Options, Nesting) -> Option<crate::svg::usvg::ImageKind>>,
        fonts: Option<&dyn crate::svg::usvg::FontProvider>,
    ) -> Result<crate::svg::usvg::Tree, Error> {
        converter::convert_doc(&self.0, opt, viewport, images, fonts)
    }
}

/// List of all errors.
#[derive(Debug)]
pub enum Error {
    /// Only UTF-8 content are supported.
    NotAnUtf8Str,

    /// Compressed SVG must use the GZip algorithm.
    MalformedGZip,

    /// We do not allow SVG with more than 1_000_000 elements for security reasons.
    ElementsLimitReached,
    /// CSS component nesting exceeded the shared structural parsing budget.
    CssComponentDepthLimit {
        /// Maximum accepted structural component depth.
        limit: usize,
    },

    /// SVG doesn't have a valid size.
    ///
    /// Occurs when width and/or height are <= 0.
    ///
    /// Also occurs if width, height and viewBox are not set.
    InvalidSize,
    Style(std::sync::Arc<crate::style::StyleError>),

    /// Failed to parse an SVG data.
    ParsingFailed(roxmltree::Error),
}

impl From<roxmltree::Error> for Error {
    fn from(e: roxmltree::Error) -> Self {
        Error::ParsingFailed(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            Error::NotAnUtf8Str => {
                write!(f, "provided data has not an UTF-8 encoding")
            }
            Error::MalformedGZip => {
                write!(f, "provided data has a malformed GZip content")
            }
            Error::ElementsLimitReached => {
                write!(f, "the maximum number of SVG elements has been reached")
            }
            Error::CssComponentDepthLimit { limit } => {
                write!(f, "SVG CSS component depth exceeds {limit}")
            }
            Error::Style(ref error) => write!(f, "SVG style: {error}"),
            Error::InvalidSize => {
                write!(f, "SVG has an invalid size")
            }
            Error::ParsingFailed(ref e) => {
                write!(f, "SVG data parsing failed cause {}", e)
            }
        }
    }
}

impl std::error::Error for Error {}

pub(crate) trait OptionLog {
    fn log_none<F: FnOnce()>(self, f: F) -> Self;
}

impl<T> OptionLog for Option<T> {
    #[inline]
    fn log_none<F: FnOnce()>(self, f: F) -> Self {
        self.or_else(|| {
            f();
            None
        })
    }
}

impl crate::svg::usvg::Tree {
    /// Parses `Tree` from an SVG data.
    ///
    /// Can contain an SVG string or a gzip compressed data.
    pub fn from_data(data: &[u8], opt: &Options) -> Result<Self, Error> {
        if data.starts_with(&[0x1f, 0x8b]) {
            let data = decompress_svgz(data)?;
            let text = std::str::from_utf8(&data).map_err(|_| Error::NotAnUtf8Str)?;
            Self::from_str(text, opt)
        } else {
            let text = std::str::from_utf8(data).map_err(|_| Error::NotAnUtf8Str)?;
            Self::from_str(text, opt)
        }
    }

    /// Similar to the `from_data` method, except that it ignores all `image` elements linking to
    /// external files, as required by the SVG specification when SVG files are loaded
    /// for `<image href="..." />` tags.
    pub fn from_data_nested(data: &[u8], opt: &Options) -> Result<Self, Error> {
        let nested_opt = Options {
            resources_dir: None,
            dpi: opt.dpi,
            font_size: opt.font_size,
            languages: opt.languages.clone(),
            shape_rendering: opt.shape_rendering,
            text_rendering: opt.text_rendering,
            image_rendering: opt.image_rendering,
            default_size: opt.default_size,
            image_href_resolver: ImageHrefResolver {
                resolve_data: Box::new(|a, b, c| (opt.image_href_resolver.resolve_data)(a, b, c)),
                // External images should be ignored.
                resolve_string: Box::new(|_, _| None),
            },
            // In the referenced SVG, we start with the unmodified user-provided
            // fontdb, not the one from the cache.
            fontdb: opt.fontdb.clone(),
            // Can't clone the resolver, so we create a new one that forwards to it.
            font_resolver: crate::svg::usvg::FontResolver {
                select_font: Box::new(|font, db| (opt.font_resolver.select_font)(font, db)),
                select_fallback: Box::new(|c, used_fonts, db| {
                    (opt.font_resolver.select_fallback)(c, used_fonts, db)
                }),
            },
            ..Options::default()
        };

        Self::from_data(data, &nested_opt)
    }

    /// Parses `Tree` from an SVG string.
    pub fn from_str(text: &str, opt: &Options) -> Result<Self, Error> {
        Self::from_str_impl(text, opt, None, None, None, None)
    }

    /// Parses with a caller-owned resolver receiving original image references after styling.
    /// The callback is synchronous and need not be thread-safe.
    /// An explicit viewport overrides used sizing without changing source attributes.
    pub fn from_str_with_image_resolver(
        text: &str,
        opt: &Options,
        viewport: Option<crate::svg::usvg::Size>,
        resolver: &dyn Fn(&str, &Options, Nesting) -> Option<crate::svg::usvg::ImageKind>,
    ) -> Result<Self, Error> {
        Self::from_str_impl(text, opt, viewport, Some(resolver), None, None)
    }

    /// Parses with borrowed image, font and linked-document providers without retaining them.
    pub fn from_str_with_resolvers(
        text: &str,
        opt: &Options,
        viewport: Option<crate::svg::usvg::Size>,
        images: &dyn Fn(&str, &Options, Nesting) -> Option<crate::svg::usvg::ImageKind>,
        fonts: &dyn crate::svg::usvg::FontProvider,
        documents: &dyn Fn(&str) -> Option<LinkedDocument>,
    ) -> Result<Self, Error> {
        Self::from_str_impl(
            text,
            opt,
            viewport,
            Some(images),
            Some(fonts),
            Some(documents),
        )
    }

    /// Parses with the documents that references into other documents name.
    pub(crate) fn from_str_with_documents(
        text: &str,
        opt: &Options,
        documents: &dyn Fn(&str) -> Option<LinkedDocument>,
    ) -> Result<Self, Error> {
        Self::from_str_impl(text, opt, None, None, None, Some(documents))
    }

    fn from_str_impl(
        text: &str,
        opt: &Options,
        viewport: Option<crate::svg::usvg::Size>,
        resolver: Option<&dyn Fn(&str, &Options, Nesting) -> Option<crate::svg::usvg::ImageKind>>,
        fonts: Option<&dyn crate::svg::usvg::FontProvider>,
        documents: Option<&dyn Fn(&str) -> Option<LinkedDocument>>,
    ) -> Result<Self, Error> {
        let doc = roxmltree::Document::parse_with_options(text, xml_options())
            .map_err(Error::ParsingFailed)?;

        let mut doc = ParsedSvg::parse(&doc, opt.style_sheet.as_deref())?;
        if let Some(documents) = documents {
            doc.link_documents(documents)?;
        }
        doc.to_tree(opt, viewport, resolver, fonts)
    }

    /// Parses `Tree` from `roxmltree::Document`.
    pub fn from_xmltree(doc: &roxmltree::Document, opt: &Options) -> Result<Self, Error> {
        let doc = ParsedSvg::parse(doc, opt.style_sheet.as_deref())?;
        doc.to_tree(opt, None, None, None)
    }
}

/// Decompresses an SVGZ file.
pub(crate) fn decompress_svgz(data: &[u8]) -> Result<Vec<u8>, Error> {
    use std::io::Read;

    let mut decoder = flate2::read::GzDecoder::new(data);
    let mut decoded = Vec::with_capacity(data.len() * 2);
    decoder
        .read_to_end(&mut decoded)
        .map_err(|_| Error::MalformedGZip)?;
    Ok(decoded)
}

#[inline]
pub(crate) fn f32_bound(min: f32, val: f32, max: f32) -> f32 {
    debug_assert!(min.is_finite());
    debug_assert!(val.is_finite());
    debug_assert!(max.is_finite());

    if val > max {
        max
    } else if val < min {
        min
    } else {
        val
    }
}
