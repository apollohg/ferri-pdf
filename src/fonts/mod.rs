//! Font data ownership and CSS face matching, independent of shaping and PDF output.
mod baselines;
mod bundled;
mod feature_values;
mod installed;
mod loading;
mod matching;
mod metrics;
mod palettes;
mod selection;
mod sfnt;
mod silhouette;
mod woff;
mod woff2;

pub(crate) use baselines::BaselineTable;
pub(crate) use feature_values::FeatureValueType;
pub(crate) use loading::OPTICAL_SIZE_AXIS;
pub(crate) use metrics::{
    FALLBACK_IDEOGRAPH_ADVANCE_EM, FALLBACK_ZERO_ADVANCE_EM, FaceMetrics, InitialFont, SizeAdjust,
    SizeMetric, WATER_IDEOGRAPH, ZERO, advance, advance_in_axis, initial_font,
};
pub(crate) use palettes::{BasePalette, FontPalette, default_palette};
pub(crate) use selection::{
    FaceSettings, FontAdjustments, FontFeature, FontInstance, MetricOverrides, UnicodeRanges,
};
pub use selection::{FontFaceStyle, FontRange, FontSelection, FontSlant, FontVariation, FontWidth};
pub(crate) use silhouette::Silhouettes;

use serde::Serialize;
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// The initial `font-family`, resolved to `serif`.
pub(crate) const DEFAULT_FAMILY: GenericFamily = GenericFamily::Serif;
/// The character whose `unicode-range` coverage makes a face the first available font.
const FIRST_AVAILABLE_CHARACTER: char = ' ';

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct FontId(usize);
impl FontId {
    pub fn index(self) -> usize {
        self.0
    }
}

/// A font-name namespace within one book; independent of DOM and selector identities.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub(crate) struct FontNamespace(usize);
impl FontNamespace {
    pub(crate) const ROOT: Self = Self(0);
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    /// An italic slanted clockwise (CSS Fonts 4 §2.4).
    Left,
    /// An italic slanted counter-clockwise.
    Right,
    Oblique,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize)]
pub struct FontSynthesis {
    pub bold: bool,
    pub oblique: bool,
    pub small_caps: bool,
    /// Superscript and subscript forms for `font-variant-position`.
    pub position: bool,
}
impl FontSynthesis {
    pub const ALL: Self = Self {
        bold: true,
        oblique: true,
        small_caps: true,
        position: true,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize)]
pub struct FontWeight(f32);
impl FontWeight {
    pub const MIN: Self = Self(1.0);
    pub const MAX: Self = Self(1000.0);
    pub const THIN: Self = Self(100.0);
    pub const NORMAL: Self = Self(400.0);
    pub const MEDIUM: Self = Self(500.0);
    pub const SEMIBOLD: Self = Self(600.0);
    pub const BOLD: Self = Self(700.0);
    pub const BLACK: Self = Self(900.0);
    const RELATIVE_NORMAL_LIMIT: f32 = 350.0;
    const RELATIVE_BOLD_LIMIT: f32 = 550.0;
    const RELATIVE_HEAVY_LIMIT: f32 = 750.0;
    pub fn new(value: f32) -> Result<Self, FontError> {
        if !(Self::MIN.0..=Self::MAX.0).contains(&value) {
            return Err(FontError::InvalidWeight(value));
        }
        Ok(Self(value))
    }
    pub const fn value(self) -> f32 {
        self.0
    }
    #[must_use]
    pub fn bolder(self) -> Self {
        if self >= Self::BLACK {
            return self;
        }
        if self.0 < Self::RELATIVE_NORMAL_LIMIT {
            Self::NORMAL
        } else if self.0 < Self::RELATIVE_BOLD_LIMIT {
            Self::BOLD
        } else {
            Self::BLACK
        }
    }
    #[must_use]
    pub fn lighter(self) -> Self {
        if self < Self::THIN {
            return self;
        }
        if self.0 < Self::RELATIVE_BOLD_LIMIT {
            Self::THIN
        } else if self.0 < Self::RELATIVE_HEAVY_LIMIT {
            Self::NORMAL
        } else {
            Self::BOLD
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GenericFamily {
    Math,
    SansSerif,
    Serif,
    Monospace,
}
impl GenericFamily {
    /// Arial-, Times New Roman- and Courier New-compatible faces first, then wider `DejaVu`
    /// coverage.
    fn bundled_names(self) -> &'static [&'static str] {
        match self {
            Self::SansSerif => &[bundled::LIBERATION_SANS, bundled::DEJAVU_SANS],
            Self::Serif => &[bundled::LIBERATION_SERIF, bundled::DEJAVU_SERIF],
            Self::Math => &[bundled::DEJAVU_SERIF],
            Self::Monospace => &[bundled::LIBERATION_MONO, bundled::DEJAVU_SANS_MONO],
        }
    }
    pub fn primary_name(self) -> &'static str {
        self.bundled_names()[0]
    }
}

/// Ordered installed-family preferences, and the font folders installed families come from.
/// Empty lists retain the bundled defaults.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FontPreferences {
    pub sans_serif: Vec<String>,
    pub serif: Vec<String>,
    pub monospace: Vec<String>,
    /// Installed families tried after the complete CSS font-family list.
    pub fallback: Vec<String>,
    /// Whether the fonts in the operating system's font folders are installed families.
    pub system_fonts: bool,
    /// Further folders, searched with their subfolders, whose fonts are installed families.
    pub font_dirs: Vec<PathBuf>,
}
impl FontPreferences {
    /// Whether no family preference is set.
    pub fn is_empty(&self) -> bool {
        self.sans_serif.is_empty()
            && self.serif.is_empty()
            && self.monospace.is_empty()
            && self.fallback.is_empty()
    }
    fn generic(&self, family: GenericFamily) -> &[String] {
        match family {
            GenericFamily::Math => &[],
            GenericFamily::SansSerif => &self.sans_serif,
            GenericFamily::Serif => &self.serif,
            GenericFamily::Monospace => &self.monospace,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum FontFamily {
    Named(String),
    Generic(GenericFamily),
}

#[derive(Clone, Debug, Serialize)]
pub struct FontDescriptor {
    pub family: String,
    pub weight: FontRange<FontWeight>,
    pub width: FontRange<FontWidth>,
    pub style: FontFaceStyle,
}

/// Availability of a face when looking for a glyph or a complete shaping cluster.
pub(crate) enum GlyphMatch<T> {
    Unavailable,
    Missing,
    Found(T),
}

/// Sorted source offsets, including the end, shared by a run's shaped glyphs.
pub(crate) fn cluster_boundaries(clusters: impl Iterator<Item = usize>, end: usize) -> Vec<usize> {
    let mut boundaries: Vec<_> = clusters.collect();
    boundaries.push(end);
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
}

#[derive(Clone)]
enum FontData {
    Bundled(&'static [u8]),
    #[cfg(test)]
    Loaded(Arc<Vec<u8>>),
    Lazy {
        sources: Arc<FontSources>,
        data: Arc<OnceLock<Option<LoadedFont>>>,
    },
    Installed(Arc<installed::InstalledSource>),
}

/// Where a face comes from, in the order in which faces of one family name hide later ones.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum FaceOrigin {
    Author,
    Bundled,
    Installed,
}

/// The data of the first `@font-face` source that loaded.
struct LoadedFont {
    data: Vec<u8>,
    /// The `url()` or installed file it came from; `None` for a bundled `local()` face.
    reference: Option<String>,
}

pub(crate) enum FontSource {
    Url(String),
    Local(String),
}

pub(crate) struct FontSources {
    pub sources: Vec<FontSource>,
    pub base: crate::resources::ResourceBase,
}

#[derive(Clone)]
pub(crate) struct FontFace {
    descriptor: FontDescriptor,
    settings: FaceSettings,
    data: FontData,
    namespace: FontNamespace,
}
impl FontFace {
    pub(crate) fn descriptor(&self) -> &FontDescriptor {
        &self.descriptor
    }
    pub(crate) fn settings(&self) -> &FaceSettings {
        &self.settings
    }
    pub(crate) fn data(&self) -> Option<&[u8]> {
        match &self.data {
            FontData::Bundled(bytes) => Some(bytes),
            #[cfg(test)]
            FontData::Loaded(bytes) => Some(bytes),
            FontData::Lazy { data, .. } => data
                .get()
                .and_then(Option::as_ref)
                .map(|loaded| loaded.data.as_slice()),
            FontData::Installed(source) => source.data(),
        }
    }
    /// The index of the face in its data, which is a collection only for installed faces.
    pub(crate) fn index(&self) -> u32 {
        match &self.data {
            FontData::Installed(source) => source.index,
            _ => 0,
        }
    }
    fn origin(&self) -> FaceOrigin {
        match self.data {
            FontData::Bundled(_) => FaceOrigin::Bundled,
            FontData::Installed(_) => FaceOrigin::Installed,
            #[cfg(test)]
            FontData::Loaded(_) => FaceOrigin::Author,
            FontData::Lazy { .. } => FaceOrigin::Author,
        }
    }
}

#[derive(Clone)]
pub(crate) struct FontBook {
    faces: Vec<FontFace>,
    /// The installed fonts, which `faces` holds at `installed_faces`, after the bundled ones.
    installed: Option<Arc<installed::InstalledFonts>>,
    installed_faces: Range<usize>,
    namespaces: Vec<Option<FontNamespace>>,
    preferences: FontPreferences,
    feature_values: Vec<feature_values::FeatureValue>,
    palettes: Vec<palettes::PaletteValues>,
    /// The `@font-face` sources loading skipped, shared by the book's copies.
    failures: Arc<Mutex<Vec<FontFailure>>>,
    /// Whether each loaded face embeds at the variation values it was instanced at.
    instances: Arc<Mutex<HashMap<InstanceKey, bool>>>,
    /// Families that a used font list names, with no family before them usable, that no face
    /// has; not yet reported.
    missing_families: Arc<Mutex<Vec<String>>>,
    /// Configured font folders that cannot be read, not yet reported.
    unreadable_directories: Arc<Mutex<Vec<(PathBuf, std::io::Error)>>>,
}

/// A face and the bits of its variation values.
type InstanceKey = (FontId, Vec<([u8; 4], u32)>);
impl Default for FontBook {
    fn default() -> Self {
        Self::with_preferences(FontPreferences::default())
    }
}
impl FontBook {
    pub(crate) fn with_preferences(preferences: FontPreferences) -> Self {
        let mut faces = bundled::faces();
        let installed = installed::installed(&preferences);
        let start = faces.len();
        if let Some(installed) = &installed {
            faces.extend(installed.book_faces());
        }
        let unreadable_directories = preferences
            .font_dirs
            .iter()
            .filter_map(|directory| {
                std::fs::read_dir(directory)
                    .err()
                    .map(|error| (directory.clone(), error))
            })
            .collect();
        Self {
            installed_faces: start..faces.len(),
            faces,
            installed,
            namespaces: vec![None],
            preferences,
            feature_values: Vec::new(),
            palettes: Vec::new(),
            failures: Arc::default(),
            instances: Arc::default(),
            missing_families: Arc::default(),
            unreadable_directories: Arc::new(Mutex::new(unreadable_directories)),
        }
    }
    /// The `@font-face` sources and installed files loading has skipped since the last call.
    pub(crate) fn take_failures(&self) -> Vec<FontFailure> {
        std::mem::take(&mut self.failures.lock().unwrap_or_else(PoisonError::into_inner))
    }
    /// The missing families font lists have named since the last call.
    pub(crate) fn take_missing_families(&self) -> Vec<String> {
        std::mem::take(
            &mut self
                .missing_families
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }
    /// The configured font folders that could not be read, since the last call.
    pub(crate) fn take_unreadable_directories(&self) -> Vec<(PathBuf, std::io::Error)> {
        std::mem::take(
            &mut self
                .unreadable_directories
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }
    fn generic_names(&self, generic: GenericFamily) -> impl Iterator<Item = &str> {
        self.preferences
            .generic(generic)
            .iter()
            .map(String::as_str)
            .chain(generic.bundled_names().iter().copied())
    }
    /// The candidates that can be the first available font: those whose `unicode-range`
    /// includes U+0020 (CSS Fonts 4 §5).
    pub(crate) fn first_available_candidates<'c>(
        &'c self,
        candidates: &'c [FontId],
    ) -> impl Iterator<Item = FontId> + 'c {
        candidates.iter().copied().filter(|&id| {
            self.face(id)
                .is_none_or(|face| face.settings.covers(FIRST_AVAILABLE_CHARACTER))
        })
    }
    /// A family's first available face fixes its descriptor before glyph fallback proceeds.
    /// Faces whose `unicode-range` excludes part of `text` are skipped unloaded (§4.5).
    pub(crate) fn find_glyph<T, E: From<FontError>>(
        &self,
        candidates: &[FontId],
        text: &str,
        mut probe: impl FnMut(FontId) -> Result<GlyphMatch<T>, E>,
    ) -> Result<Option<(FontId, T)>, E> {
        let mut matched: Vec<&FontDescriptor> = Vec::new();
        for &id in candidates {
            let face = self.face(id).ok_or(FontError::UnknownFont(id))?;
            if !face.settings.covers_text(text) {
                continue;
            }
            let descriptor = face.descriptor();
            let previous = matched
                .iter()
                .find(|previous| matching::names_match(&previous.family, &descriptor.family));
            if previous.is_some_and(|previous| {
                previous.style != descriptor.style
                    || previous.weight != descriptor.weight
                    || previous.width != descriptor.width
            }) {
                continue;
            }
            match probe(id)? {
                GlyphMatch::Unavailable => {}
                GlyphMatch::Found(value) => return Ok(Some((id, value))),
                GlyphMatch::Missing if previous.is_none() => matched.push(descriptor),
                GlyphMatch::Missing => {}
            }
        }
        Ok(None)
    }
    pub(crate) fn child_namespace(&mut self, parent: FontNamespace) -> FontNamespace {
        assert!(
            parent.0 < self.namespaces.len(),
            "parent namespace belongs to this book"
        );
        let namespace = FontNamespace(self.namespaces.len());
        self.namespaces.push(Some(parent));
        namespace
    }
    #[cfg(test)]
    pub(crate) fn register(
        &mut self,
        descriptor: FontDescriptor,
        data: Vec<u8>,
    ) -> Result<FontId, FontError> {
        validate(&data, 0)?;
        let id = FontId(self.faces.len());
        self.faces.push(FontFace {
            descriptor,
            settings: FaceSettings::default(),
            data: FontData::Loaded(Arc::new(data)),
            namespace: FontNamespace::ROOT,
        });
        Ok(id)
    }
    #[cfg(test)]
    pub(crate) fn declare(&mut self, descriptor: FontDescriptor, sources: FontSources) -> FontId {
        self.declare_in_namespace(
            FontNamespace::ROOT,
            descriptor,
            FaceSettings::default(),
            sources,
        )
    }
    pub(crate) fn declare_in_namespace(
        &mut self,
        namespace: FontNamespace,
        descriptor: FontDescriptor,
        settings: FaceSettings,
        sources: FontSources,
    ) -> FontId {
        let id = FontId(self.faces.len());
        self.faces.push(FontFace {
            descriptor,
            settings,
            namespace,
            data: FontData::Lazy {
                sources: Arc::new(sources),
                data: Arc::default(),
            },
        });
        id
    }
    pub(crate) fn face(&self, id: FontId) -> Option<&FontFace> {
        self.faces.get(id.0)
    }
    pub(crate) fn faces(&self) -> &[FontFace] {
        &self.faces
    }
    pub(crate) fn faces_with_ids(&self) -> impl Iterator<Item = (FontId, &FontFace)> {
        self.faces
            .iter()
            .enumerate()
            .map(|(index, face)| (FontId(index), face))
    }
    pub(crate) fn candidates(
        &self,
        families: &[FontFamily],
        selection: FontSelection,
    ) -> Arc<[FontId]> {
        self.candidates_in_namespace(FontNamespace::ROOT, families, selection)
    }
    pub(crate) fn candidates_in_namespace(
        &self,
        namespace: FontNamespace,
        families: &[FontFamily],
        selection: FontSelection,
    ) -> Arc<[FontId]> {
        self.matching_candidates(namespace, families, selection, true)
    }

    /// Private-use characters can match only explicitly named families.
    pub(crate) fn named_candidates_in_namespace(
        &self,
        namespace: FontNamespace,
        families: &[FontFamily],
        selection: FontSelection,
    ) -> Arc<[FontId]> {
        self.matching_candidates(namespace, families, selection, false)
    }

    /// The bundled face has a visible replacement symbol, independent of author overrides.
    pub(crate) fn missing_glyph_font(&self) -> Option<FontId> {
        self.faces.iter().enumerate().find_map(|(index, face)| {
            (matches!(face.data, FontData::Bundled(_))
                && face.descriptor.family == bundled::DEJAVU_SANS
                && face.descriptor.style == FontFaceStyle::Normal
                && face.descriptor.weight.bounds(FontWeight::NORMAL)
                    == (FontWeight::NORMAL, FontWeight::NORMAL))
                .then_some(FontId(index))
        })
    }

    /// The ascending IDs of the faces in `namespace` named `name`.
    fn named_faces(&self, namespace: FontNamespace, name: &str) -> Vec<FontId> {
        let named = |(index, face): (usize, &FontFace)| {
            (face.namespace == namespace && matching::names_match(&face.descriptor.family, name))
                .then_some(FontId(index))
        };
        let Range { start, end } = self.installed_faces;
        let installed = self
            .installed
            .as_ref()
            .filter(|_| namespace == FontNamespace::ROOT)
            .map_or(&[][..], |installed| installed.family(name));
        self.faces
            .iter()
            .enumerate()
            .take(start)
            .filter_map(named)
            .chain(installed.iter().map(|position| FontId(start + position)))
            .chain(self.faces.iter().enumerate().skip(end).filter_map(named))
            .collect()
    }

    fn matching_candidates(
        &self,
        namespace: FontNamespace,
        families: &[FontFamily],
        selection: FontSelection,
        fallback: bool,
    ) -> Arc<[FontId]> {
        let mut result = Vec::new();
        let mut append = |name: &str, namespace, installed_only| {
            let namespace = self.family_namespace(name, namespace);
            let named = self.named_faces(namespace, name);
            let mut candidates =
                matching::candidates(&self.faces, &named, selection, installed_only);
            if selection.style != FontStyle::Normal {
                candidates.extend(matching::candidates(
                    &self.faces,
                    &named,
                    FontSelection {
                        style: FontStyle::Normal,
                        slant: FontSlant::NORMAL,
                        ..selection
                    },
                    installed_only,
                ));
            }
            let found = !candidates.is_empty();
            for id in candidates {
                if !result.contains(&id) {
                    result.push(id);
                }
            }
            found
        };
        // A named family no face has is reported only while no earlier family matched.
        let mut matched = false;
        let mut missing = Vec::new();
        for family in families {
            match family {
                FontFamily::Named(name) => {
                    let found = append(name, namespace, false);
                    if !found && !matched {
                        missing.push(name);
                    }
                    matched |= found;
                }
                FontFamily::Generic(generic) if fallback => {
                    matched = true;
                    for name in self.generic_names(*generic) {
                        append(name, FontNamespace::ROOT, true);
                    }
                }
                FontFamily::Generic(_) => {}
            }
        }
        if !fallback {
            return result.into();
        }
        if !missing.is_empty() {
            let mut reported = self
                .missing_families
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for name in missing {
                if !reported
                    .iter()
                    .any(|known| matching::names_match(known, name))
                {
                    reported.push(name.clone());
                }
            }
        }
        for name in &self.preferences.fallback {
            append(name, FontNamespace::ROOT, true);
        }
        // Glyphs absent from the whole list use the initial family, then any bundled face.
        for generic in [
            DEFAULT_FAMILY,
            GenericFamily::SansSerif,
            GenericFamily::Serif,
            GenericFamily::Monospace,
        ] {
            for name in self.generic_names(generic) {
                append(name, FontNamespace::ROOT, true);
            }
        }
        result.into()
    }

    fn family_namespace(&self, name: &str, mut namespace: FontNamespace) -> FontNamespace {
        while namespace != FontNamespace::ROOT {
            if self.faces.iter().any(|face| {
                face.namespace == namespace
                    && !matches!(face.data, FontData::Bundled(_))
                    && matching::names_match(&face.descriptor.family, name)
            }) {
                return namespace;
            }
            namespace = self.namespaces[namespace.0].expect("child font namespace has a parent");
        }
        FontNamespace::ROOT
    }
}

/// Rejects face `index` of `data` if the renderer cannot both shape and embed it, as user
/// agents sanitize fonts before use (CSS Fonts 4 §4.3): a font that fails is invalid, so
/// loading moves on to the face's next source.
fn validate(data: &[u8], index: u32) -> Result<(), FontError> {
    let face = ttf_parser::Face::parse(data, index).map_err(FontError::InvalidFont)?;
    embed_all_glyphs(data, index, &face, &[]).map_err(FontError::Unembeddable)
}

/// Embeds a subset of every glyph of `face`, face `index` of `data`, at `variations`, as the
/// PDF writer embeds the glyphs it uses, so that success covers each subset the PDF needs.
fn embed_all_glyphs(
    data: &[u8],
    index: u32,
    face: &ttf_parser::Face<'_>,
    variations: &[FontVariation],
) -> Result<(), subsetter::Error> {
    let glyphs: Vec<u16> = (0..face.number_of_glyphs()).collect();
    let coordinates: Vec<_> = variations
        .iter()
        .map(|variation| (subsetter::Tag::new(&variation.tag), variation.value))
        .collect();
    subsetter::subset_with_variations(
        data,
        index,
        &coordinates,
        &subsetter::GlyphRemapper::new_from_glyphs_sorted(&glyphs),
    )
    .map(|_| ())
}

/// An `@font-face` source skipped because it could not be fetched or is not a usable font, or
/// an instance of its font that cannot be embedded.
#[derive(Debug, thiserror::Error)]
#[error("font {reference}: {error}")]
pub struct FontFailure {
    pub base: crate::resources::ResourceBase,
    pub reference: String,
    #[source]
    pub error: FontError,
}

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("{0}")]
    Resource(#[from] crate::resources::ResourceError),
    #[error("font ID {0:?} is absent from the font book")]
    UnknownFont(FontId),
    #[error("font ID {0:?} has not been resolved in this job")]
    UnresolvedFont(FontId),
    #[error("invalid font: {0}")]
    InvalidFont(#[source] ttf_parser::FaceParsingError),
    #[error("{0}")]
    File(#[from] sfnt::FontFileError),
    #[error("font cannot be embedded: {0}")]
    Unembeddable(#[source] subsetter::Error),
    #[error(
        "font instance {variations:?} cannot be embedded, so its text uses the default instance: {error}"
    )]
    UnembeddableInstance {
        variations: Vec<FontVariation>,
        #[source]
        error: subsetter::Error,
    },
    #[error("font weight must be between 1 and 1000: {0}")]
    InvalidWeight(f32),
    #[error("font width must be a finite nonnegative percentage: {0}")]
    InvalidWidth(f32),
    #[error("font slant must be between -90 and 90 degrees: {0}")]
    InvalidSlant(f32),
    #[error("no font face in the family list is available")]
    NoAvailableFace,
}

#[cfg(test)]
mod tests;

/// Whether the face's substitution or positioning features include `tag`.
pub(crate) fn has_feature(face: &ttf_parser::Face<'_>, tag: [u8; 4]) -> bool {
    let tag = ttf_parser::Tag::from_bytes(&tag);
    [face.tables().gsub, face.tables().gpos]
        .into_iter()
        .flatten()
        .any(|table| table.features.find(tag).is_some())
}
