//! The render settings JavaScript passes as JSON, checked into the library's options.

use ferri_pdf::{
    ColorScheme, ContentLoss, ContrastPreference, FontPreferences, ImageDpi, ImageOptions,
    JpegQuality, MotionPreference, PrintPreferences, PrintSettings, RenderOptions, ResourceBase,
    Url,
};
use serde::Deserialize;

use crate::resources::FetchLimits;

/// A style sheet import chain, the images and fonts its sheets name, and what those name in
/// turn take a round each; documents rarely need more than four.
const DEFAULT_FETCH_ROUNDS: u32 = 16;
const BASE_URL: &str = "baseUrl";
const FILE_ROOT: &str = "fileRoot";
const DEFAULT_FETCH_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ColorSchemeName {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ContrastName {
    #[default]
    NoPreference,
    Less,
    More,
    Custom,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ReducedMotionName {
    #[default]
    NoPreference,
    Reduce,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Settings {
    base_url: Option<String>,
    /// Whether `base_url` is a folder rather than the URL of the document.
    base_directory: bool,
    /// The `file:` URL of the folder local files are read in, instead of each base's.
    file_root: Option<String>,
    stylesheets: Vec<String>,
    user_stylesheets: Vec<String>,
    paper: Option<[f32; 2]>,
    color_scheme: ColorSchemeName,
    contrast: ContrastName,
    reduced_motion: ReducedMotionName,
    serif: Vec<String>,
    sans_serif: Vec<String>,
    monospace: Vec<String>,
    fallback: Vec<String>,
    image_dpi: Option<f32>,
    jpeg_quality: Option<u8>,
    include_layout: bool,
    allow_content_loss: bool,
    max_fetch_rounds: Option<u32>,
    max_fetch_bytes: Option<usize>,
}

/// `url` as the base of a document or the place a resource was fetched from.
pub(crate) fn parse_url(name: &str, url: &str) -> Result<Url, String> {
    Url::parse(url).map_err(|error| format!("{name} {url:?}: {error}"))
}

/// The base `url` names: a folder if `directory`, else the document at that URL.
pub(crate) fn resource_base(
    name: &str,
    url: &str,
    directory: bool,
) -> Result<ResourceBase, String> {
    let url = parse_url(name, url)?;
    Ok(if directory {
        ResourceBase::Directory(url)
    } else {
        ResourceBase::Url(url)
    })
}

impl Settings {
    pub(crate) fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|error| format!("render options: {error}"))
    }

    pub(crate) fn fetch_limits(&self) -> FetchLimits {
        FetchLimits {
            rounds: self.max_fetch_rounds.unwrap_or(DEFAULT_FETCH_ROUNDS),
            bytes: self.max_fetch_bytes.unwrap_or(DEFAULT_FETCH_BYTES),
        }
    }

    /// The folder the caller confined local files to, if any.
    pub(crate) fn file_root(&self) -> Result<Option<Url>, String> {
        self.file_root
            .as_deref()
            .map(|root| parse_url(FILE_ROOT, root))
            .transpose()
    }

    /// The library's options. The parts of a multipart render share `baseUrl` as the place
    /// their relative URLs resolve against, so there it is a folder, not any one's URL.
    pub(crate) fn into_options(self, multipart: bool) -> Result<RenderOptions, String> {
        let mut print = PrintSettings {
            preferences: PrintPreferences {
                color_scheme: match self.color_scheme {
                    ColorSchemeName::Light => ColorScheme::Light,
                    ColorSchemeName::Dark => ColorScheme::Dark,
                },
                contrast: match self.contrast {
                    ContrastName::NoPreference => ContrastPreference::NoPreference,
                    ContrastName::Less => ContrastPreference::Less,
                    ContrastName::More => ContrastPreference::More,
                    ContrastName::Custom => ContrastPreference::Custom,
                },
                reduced_motion: match self.reduced_motion {
                    ReducedMotionName::NoPreference => MotionPreference::NoPreference,
                    ReducedMotionName::Reduce => MotionPreference::Reduce,
                },
            },
            fonts: FontPreferences {
                sans_serif: self.sans_serif,
                serif: self.serif,
                monospace: self.monospace,
                fallback: self.fallback,
            },
            ..PrintSettings::default()
        };
        if let Some(paper) = self.paper {
            print.paper = paper;
        }
        print.validate().map_err(|error| error.to_string())?;
        let base = self
            .base_url
            .map(|url| resource_base(BASE_URL, &url, multipart || self.base_directory))
            .transpose()?
            .unwrap_or_default();
        Ok(RenderOptions {
            base,
            stylesheets: self.stylesheets,
            user_stylesheets: self.user_stylesheets,
            print,
            include_layout: self.include_layout,
            images: ImageOptions {
                jpeg_quality: self
                    .jpeg_quality
                    .map(JpegQuality::new)
                    .transpose()
                    .map_err(|error| format!("jpegQuality: {error}"))?,
                dpi: self
                    .image_dpi
                    .map(ImageDpi::new)
                    .transpose()
                    .map_err(|error| format!("imageDpi: {error}"))?,
            },
            content_loss: if self.allow_content_loss {
                ContentLoss::Render
            } else {
                ContentLoss::Fail
            },
        })
    }
}
