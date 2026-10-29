// Copyright 2018 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

mod basic_shape;
mod clippath;
mod converter;
mod filter;
mod image;
mod marker;
mod mask;
mod options;
mod paint_server;
mod shapes;
mod style;
mod svgtree;
mod switch;
mod units;
mod use_node;

mod text;
pub(crate) use converter::Cache;
pub(crate) use image::ImageHrefResolver;
pub(crate) use options::Options;

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
        Self::from_str_impl(text, opt, None, None, None)
    }

    /// Parses with a caller-owned resolver receiving original image references after styling.
    /// The callback is synchronous and need not be thread-safe.
    /// An explicit viewport overrides used sizing without changing source attributes.
    pub fn from_str_with_image_resolver(
        text: &str,
        opt: &Options,
        viewport: Option<crate::svg::usvg::Size>,
        resolver: &dyn Fn(&str, &Options) -> Option<crate::svg::usvg::ImageKind>,
    ) -> Result<Self, Error> {
        Self::from_str_impl(text, opt, viewport, Some(resolver), None)
    }

    /// Parses with borrowed image and font providers without retaining either provider.
    pub fn from_str_with_resolvers(
        text: &str,
        opt: &Options,
        viewport: Option<crate::svg::usvg::Size>,
        images: &dyn Fn(&str, &Options) -> Option<crate::svg::usvg::ImageKind>,
        fonts: &dyn crate::svg::usvg::FontProvider,
    ) -> Result<Self, Error> {
        Self::from_str_impl(text, opt, viewport, Some(images), Some(fonts))
    }

    fn from_str_impl(
        text: &str,
        opt: &Options,
        viewport: Option<crate::svg::usvg::Size>,
        resolver: Option<&dyn Fn(&str, &Options) -> Option<crate::svg::usvg::ImageKind>>,
        fonts: Option<&dyn crate::svg::usvg::FontProvider>,
    ) -> Result<Self, Error> {
        let xml_opt = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };

        let doc =
            roxmltree::Document::parse_with_options(text, xml_opt).map_err(Error::ParsingFailed)?;

        let doc = svgtree::Document::parse_tree(&doc, opt.style_sheet.as_deref())?;
        converter::convert_doc(&doc, opt, viewport, resolver, fonts)
    }

    /// Parses `Tree` from `roxmltree::Document`.
    pub fn from_xmltree(doc: &roxmltree::Document, opt: &Options) -> Result<Self, Error> {
        let doc = svgtree::Document::parse_tree(doc, opt.style_sheet.as_deref())?;
        converter::convert_doc(&doc, opt, None, None, None)
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
