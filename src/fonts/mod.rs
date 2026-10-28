//! Font data ownership and CSS face matching, independent of shaping and PDF output.
mod bundled;
mod feature_values;
mod loading;
mod matching;
mod metrics;
mod palettes;
mod selection;

pub(crate) use feature_values::FeatureValueType;
pub(crate) use metrics::{
    FALLBACK_IDEOGRAPH_ADVANCE_EM, FALLBACK_ZERO_ADVANCE_EM, FaceMetrics, InitialFont, SizeAdjust,
    SizeMetric, WATER_IDEOGRAPH, ZERO, advance, initial_font,
};
pub(crate) use palettes::{BasePalette, FontPalette};
pub(crate) use selection::{FaceSettings, FontAdjustments, FontFeature, FontInstance};
pub use selection::{FontFaceStyle, FontRange, FontSelection, FontSlant, FontVariation, FontWidth};

use serde::Serialize;
use std::sync::{Arc, OnceLock};

/// The initial `font-family`, resolved to `serif`.
pub(crate) const DEFAULT_FAMILY: GenericFamily = GenericFamily::Serif;

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
            Self::Monospace => &[bundled::LIBERATION_MONO, bundled::DEJAVU_SANS_MONO],
        }
    }
    pub fn primary_name(self) -> &'static str {
        self.bundled_names()[0]
    }
}

/// Ordered installed-family preferences. Empty lists retain the bundled defaults.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FontPreferences {
    pub sans_serif: Vec<String>,
    pub serif: Vec<String>,
    pub monospace: Vec<String>,
    /// Installed families tried after the complete CSS font-family list.
    pub fallback: Vec<String>,
}
impl FontPreferences {
    pub fn is_empty(&self) -> bool {
        self.sans_serif.is_empty()
            && self.serif.is_empty()
            && self.monospace.is_empty()
            && self.fallback.is_empty()
    }
    fn generic(&self, family: GenericFamily) -> &[String] {
        match family {
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
        data: Arc<OnceLock<Option<Vec<u8>>>>,
    },
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
            FontData::Lazy { data, .. } => data.get().and_then(Option::as_deref),
        }
    }
}

#[derive(Clone)]
pub(crate) struct FontBook {
    faces: Vec<FontFace>,
    namespaces: Vec<Option<FontNamespace>>,
    preferences: FontPreferences,
    feature_values: Vec<feature_values::FeatureValue>,
    palettes: Vec<palettes::PaletteValues>,
}
impl Default for FontBook {
    fn default() -> Self {
        Self::with_preferences(FontPreferences::default())
    }
}
impl FontBook {
    pub(crate) fn with_preferences(preferences: FontPreferences) -> Self {
        Self {
            faces: bundled::faces(),
            namespaces: vec![None],
            preferences,
            feature_values: Vec::new(),
            palettes: Vec::new(),
        }
    }
    fn generic_names(&self, generic: GenericFamily) -> impl Iterator<Item = &str> {
        self.preferences
            .generic(generic)
            .iter()
            .map(String::as_str)
            .chain(generic.bundled_names().iter().copied())
    }
    /// A family's first available face fixes its descriptor before glyph fallback proceeds.
    pub(crate) fn find_glyph<T, E: From<FontError>>(
        &self,
        candidates: &[FontId],
        mut probe: impl FnMut(FontId) -> Result<GlyphMatch<T>, E>,
    ) -> Result<Option<(FontId, T)>, E> {
        let mut matched: Vec<&FontDescriptor> = Vec::new();
        for &id in candidates {
            let descriptor = self
                .face(id)
                .ok_or(FontError::UnknownFont(id))?
                .descriptor();
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
        validate(&data)?;
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
            let mut candidates =
                matching::candidates(&self.faces, namespace, name, selection, installed_only);
            if selection.style != FontStyle::Normal {
                candidates.extend(matching::candidates(
                    &self.faces,
                    namespace,
                    name,
                    FontSelection {
                        style: FontStyle::Normal,
                        slant: FontSlant::NORMAL,
                        ..selection
                    },
                    installed_only,
                ));
            }
            for id in candidates {
                if !result.contains(&id) {
                    result.push(id);
                }
            }
        };
        for family in families {
            match family {
                FontFamily::Named(name) => append(name, namespace, false),
                FontFamily::Generic(generic) if fallback => {
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

fn validate(data: &[u8]) -> Result<(), FontError> {
    ttf_parser::Face::parse(data, 0)
        .map(|_| ())
        .map_err(FontError::InvalidFont)
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
    #[error("font weight must be between 1 and 1000: {0}")]
    InvalidWeight(f32),
    #[error("font width must be a finite nonnegative percentage: {0}")]
    InvalidWidth(f32),
    #[error("font slant must be between -90 and 90 degrees: {0}")]
    InvalidSlant(f32),
    #[error("cannot apply font variation axis {0:?}")]
    VariationAxis([u8; 4]),
    #[error("adjusted font size is not finite: {0}")]
    InvalidSize(f32),
    #[error("no font face in the family list is available")]
    NoAvailableFace,
}

#[cfg(test)]
mod tests;
