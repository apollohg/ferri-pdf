#[cfg(file_system)]
use crate::fonts::installed::InstalledSource;
use crate::fonts::{
    FontAdjustments, FontBook, FontData, FontError, FontFace, FontFaceStyle, FontFailure, FontId,
    FontInstance, FontRange, FontSelection, FontSlant, FontSource, FontSources, FontStyle,
    FontVariation, FontWeight, LoadedFont, embed_all_glyphs, matching, sfnt, validate,
};
#[cfg(file_system)]
use crate::resources::ResourceError;
use crate::resources::{ResourceBase, ResourceResolver};
#[cfg(file_system)]
use std::path::Path;
#[cfg(file_system)]
use std::sync::Arc;
use std::sync::PoisonError;

pub(super) const WEIGHT_AXIS: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"wght");
pub(super) const WIDTH_AXIS: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"wdth");
pub(super) const SLANT_AXIS: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"slnt");
pub(super) const ITALIC_AXIS: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"ital");
pub(crate) const OPTICAL_SIZE_AXIS: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"opsz");

impl FontBook {
    /// The face at `size` with the axis values of CSS Fonts 4 §7.2: matching, then the
    /// `@font-face` descriptor, optical sizing and `font-variation-settings`.
    pub(crate) fn instance(
        &self,
        id: FontId,
        requested: FontSelection,
        adjustments: &FontAdjustments,
        size: f32,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<FontInstance<'_>>, FontError> {
        self.instance_data(
            id,
            requested,
            adjustments,
            size,
            self.resolve(id, resolver)?,
        )
    }

    /// Measures a face whose source list has already been attempted in this job, as font
    /// matching and its `@font-face` rule select it.
    pub(crate) fn cached_instance(
        &self,
        id: FontId,
        requested: FontSelection,
    ) -> Result<Option<FontInstance<'_>>, FontError> {
        self.instance_data(
            id,
            requested,
            &FontAdjustments::none(),
            0.0,
            self.resolve_cached(id)?,
        )
    }

    fn instance_data<'a>(
        &'a self,
        id: FontId,
        requested: FontSelection,
        adjustments: &FontAdjustments,
        size: f32,
        data: Option<&'a [u8]>,
    ) -> Result<Option<FontInstance<'a>>, FontError> {
        let Some(data) = data else {
            return Ok(None);
        };
        let face_entry = self.face(id).ok_or(FontError::UnknownFont(id))?;
        let descriptor = &face_entry.descriptor;
        let index = face_entry.index();
        let mut face = ttf_parser::Face::parse(data, index).map_err(FontError::InvalidFont)?;
        let italic_axis = face
            .variation_axes()
            .into_iter()
            .any(|axis| axis.tag == ITALIC_AXIS);
        let (use_italic, angle) = match descriptor.style {
            FontFaceStyle::Auto => match requested.style {
                FontStyle::Italic if italic_axis => (true, FontSlant::NORMAL),
                FontStyle::Italic => (false, FontSlant::ITALIC_MATCH),
                FontStyle::Normal => (false, FontSlant::NORMAL),
                FontStyle::Oblique => (false, requested.slant),
                // An `ital` axis gives no slant direction; `slnt` gives one.
                style @ (FontStyle::Left | FontStyle::Right) => (
                    false,
                    FontSlant::ITALIC_MATCH.directed(style.italic_direction()),
                ),
            },
            FontFaceStyle::Italic | FontFaceStyle::Left | FontFaceStyle::Right => {
                (true, FontSlant::NORMAL)
            }
            other => (false, matching::style_match(other, requested).1),
        };
        let mut weight = match descriptor.weight {
            FontRange::Auto => FontWeight::NORMAL.value(),
            range @ FontRange::Values { .. } => range.clamp(requested.weight).value(),
        };
        let mut style = match descriptor.style {
            FontFaceStyle::Italic => FontStyle::Italic,
            FontFaceStyle::Left => FontStyle::Left,
            FontFaceStyle::Right => FontStyle::Right,
            FontFaceStyle::Oblique(_) if angle != FontSlant::NORMAL => FontStyle::Oblique,
            _ => FontStyle::Normal,
        };
        let mut variations = Vec::new();
        for axis in face.variation_axes() {
            let value = match axis.tag {
                WEIGHT_AXIS => descriptor.weight.clamp(requested.weight).value(),
                WIDTH_AXIS => descriptor.width.clamp(requested.width).value(),
                ITALIC_AXIS if use_italic => 1.0,
                SLANT_AXIS if !use_italic => -angle.value(),
                _ => continue,
            }
            .clamp(axis.min_value, axis.max_value);
            if axis.tag == WEIGHT_AXIS {
                weight = value;
            }
            if descriptor.style == FontFaceStyle::Auto {
                if axis.tag == ITALIC_AXIS && value != 0.0 {
                    style = FontStyle::Italic;
                }
                if axis.tag == SLANT_AXIS && value != 0.0 {
                    style = FontStyle::Oblique;
                }
            }
            variations.push(FontVariation {
                tag: axis.tag.to_bytes(),
                value,
            });
        }
        let (default_weight, default_style) = (weight, style);
        let optical = adjustments.optical_sizing.then_some(FontVariation {
            tag: OPTICAL_SIZE_AXIS.to_bytes(),
            value: size,
        });
        // Later settings replace earlier ones; axes the font lacks are ignored (CSS Fonts 4 §8.2).
        for setting in face_entry
            .settings
            .variations
            .iter()
            .chain(&optical)
            .chain(adjustments.variations.iter())
        {
            let Some(axis) = face
                .variation_axes()
                .into_iter()
                .find(|axis| axis.tag.to_bytes() == setting.tag)
            else {
                continue;
            };
            let value = setting.value.clamp(axis.min_value, axis.max_value);
            match variations.iter_mut().find(|v| v.tag == setting.tag) {
                Some(existing) => existing.value = value,
                None => variations.push(FontVariation {
                    tag: setting.tag,
                    value,
                }),
            }
        }
        // A face whose variation data cannot take a value keeps that axis's default; one that
        // cannot be embedded at its values keeps every axis's default.
        variations.retain(|variation| {
            face.set_variation(ttf_parser::Tag::from_bytes(&variation.tag), variation.value)
                .is_some()
        });
        if !variations.is_empty() && !self.embeds_instance(id, data, &face, &variations) {
            face = ttf_parser::Face::parse(data, index).map_err(FontError::InvalidFont)?;
            variations.clear();
            (weight, style) = (default_weight, default_style);
        }
        Ok(Some(FontInstance {
            face,
            variations: variations.into(),
            metric_overrides: face_entry.settings.metric_overrides,
            weight,
            style,
        }))
    }

    /// Whether the face has the `opsz` axis that `font-optical-sizing` sets.
    pub(crate) fn has_optical_axis(
        &self,
        id: FontId,
        resolver: &dyn ResourceResolver,
    ) -> Result<bool, FontError> {
        let Some(data) = self.resolve(id, resolver)? else {
            return Ok(false);
        };
        let index = self.face(id).map_or(0, FontFace::index);
        Ok(ttf_parser::Face::parse(data, index)
            .map_err(FontError::InvalidFont)?
            .variation_axes()
            .into_iter()
            .any(|axis| axis.tag == OPTICAL_SIZE_AXIS))
    }

    pub(crate) fn resolve(
        &self,
        id: FontId,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<&[u8]>, FontError> {
        let face = self.face(id).ok_or(FontError::UnknownFont(id))?;
        match &face.data {
            FontData::Lazy { sources, data } if data.get().is_none() => {
                let _ = data.set(self.load_sources(sources, resolver));
            }
            #[cfg(file_system)]
            FontData::Installed(source) => self.load_installed(source),
            _ => {}
        }
        Ok(face.data())
    }

    /// Reads an installed face's file and checks the face when it is first used; a file that
    /// cannot be read or a face that is not a usable font is recorded, and the face is
    /// unavailable, so matching moves on to the next candidate.
    #[cfg(file_system)]
    fn load_installed(&self, source: &InstalledSource) {
        source.usable.get_or_init(|| {
            let file = source
                .file
                .get_or_init(|| match std::fs::read(&source.path) {
                    Ok(data) => Some(Arc::new(data)),
                    Err(error) => {
                        self.record_failure(
                            installed_reference(&source.path),
                            ResourceError::Read {
                                path: source.path.to_path_buf(),
                                source: error,
                            }
                            .into(),
                        );
                        None
                    }
                });
            let Some(data) = file else {
                return false;
            };
            validate(data, source.index)
                .map_err(|error| self.record_failure(installed_reference(&source.path), error))
                .is_ok()
        });
    }

    fn record_failure(&self, (base, reference): (ResourceBase, String), error: FontError) {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FontFailure {
                base,
                reference,
                error,
            });
    }

    /// Whether the author or installed face `id`, whose `data` is `face`, embeds at
    /// `variations`; checked once per instance, recording why not. Bundled faces always embed.
    fn embeds_instance(
        &self,
        id: FontId,
        data: &[u8],
        face: &ttf_parser::Face<'_>,
        variations: &[FontVariation],
    ) -> bool {
        let Some(entry) = self.face(id) else {
            return true;
        };
        let Some(reference) = data_reference(entry) else {
            return true;
        };
        let key = (
            id,
            variations
                .iter()
                .map(|variation| (variation.tag, variation.value.to_bits()))
                .collect(),
        );
        if let Some(embeds) = self
            .instances
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            return *embeds;
        }
        let embedded = embed_all_glyphs(data, entry.index(), face, variations);
        let embeds = embedded.is_ok();
        if let Err(error) = embedded {
            self.record_failure(
                reference,
                FontError::UnembeddableInstance {
                    variations: variations.to_vec(),
                    error,
                },
            );
        }
        self.instances
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, embeds);
        embeds
    }

    /// Reads an already attempted author face without acquiring additional resources; an
    /// installed face, which needs no resolver, is read when first used.
    pub(crate) fn resolve_cached(&self, id: FontId) -> Result<Option<&[u8]>, FontError> {
        let face = self.face(id).ok_or(FontError::UnknownFont(id))?;
        match &face.data {
            FontData::Lazy { data, .. } if data.get().is_none() => {
                return Err(FontError::UnresolvedFont(id));
            }
            #[cfg(file_system)]
            FontData::Installed(source) => self.load_installed(source),
            _ => {}
        }
        Ok(face.data())
    }

    /// The first of `sources` that loads as a valid font (CSS Fonts 4 §4.3); a URL that
    /// cannot be fetched, policy denies, or that holds no valid font is skipped, recorded.
    /// WOFF and WOFF2 files are decoded and collections give the face the URL's fragment
    /// names (§4.3.2) before validation.
    fn load_sources(
        &self,
        sources: &FontSources,
        resolver: &dyn ResourceResolver,
    ) -> Option<LoadedFont> {
        for source in &sources.sources {
            let reference = match source {
                FontSource::Local(name) => match self.local_font(name) {
                    Some(loaded) => return Some(loaded),
                    None => continue,
                },
                FontSource::Url(reference) => reference,
            };
            let loaded = sfnt::postscript_fragment(reference)
                .map_err(FontError::from)
                .and_then(|fragment| {
                    let resource = resolver.resolve(reference, &sources.base)?;
                    let data = sfnt::decode(resource.bytes, fragment.as_deref())?;
                    validate(&data, 0).map(|()| data)
                });
            match loaded {
                Ok(data) => {
                    return Some(LoadedFont {
                        data,
                        reference: Some(reference.clone()),
                    });
                }
                Err(error) => self.record_failure((sources.base.clone(), reference.clone()), error),
            }
        }
        None
    }

    /// The bundled, else installed, face whose full or PostScript name is `name` (CSS Fonts 4
    /// §4.3.1), as a single font; a face of a collection is taken out of it.
    fn local_font(&self, name: &str) -> Option<LoadedFont> {
        if let Some(data) = self.bundled_local(name) {
            return Some(LoadedFont {
                data: data.to_vec(),
                reference: None,
            });
        }
        #[cfg(not(file_system))]
        return None;
        #[cfg(file_system)]
        self.installed_local(name)
    }

    /// The installed face whose full or PostScript name is `name`, as a single font.
    #[cfg(file_system)]
    fn installed_local(&self, name: &str) -> Option<LoadedFont> {
        let position = self.installed.as_ref()?.named(name)?;
        let face = self.faces.get(self.installed_faces.start + position)?;
        let FontData::Installed(source) = &face.data else {
            return None;
        };
        self.load_installed(source);
        let reference = installed_reference(&source.path);
        let single = sfnt::single_face(source.data()?, source.index)
            .map_err(FontError::from)
            .and_then(|data| validate(&data, 0).map(|()| data));
        match single {
            Ok(data) => Some(LoadedFont {
                data,
                reference: Some(reference.1),
            }),
            Err(error) => {
                self.record_failure(reference, error);
                None
            }
        }
    }

    fn bundled_local(&self, name: &str) -> Option<&'static [u8]> {
        self.faces.iter().find_map(|face| {
            let FontData::Bundled(data) = &face.data else {
                return None;
            };
            let parsed = ttf_parser::Face::parse(data, 0).ok()?;
            let matched = parsed.names().into_iter().any(|record| {
                matches!(
                    record.name_id,
                    ttf_parser::name_id::FULL_NAME | ttf_parser::name_id::POST_SCRIPT_NAME
                ) && record
                    .to_string()
                    .is_some_and(|value| matching::names_match(&value, name))
            });
            matched.then_some(*data)
        })
    }
}

/// The base and reference a failure of the installed font file `path` names.
#[cfg(file_system)]
fn installed_reference(path: &Path) -> (ResourceBase, String) {
    (
        ResourceBase::Directory(path.parent().unwrap_or(path).to_path_buf()),
        path.display().to_string(),
    )
}

/// Where an author or installed face's data came from, which its failures name; `None` for
/// bundled data, which always embeds.
fn data_reference(face: &FontFace) -> Option<(ResourceBase, String)> {
    match &face.data {
        FontData::Lazy { sources, data } => Some((
            sources.base.clone(),
            data.get()?.as_ref()?.reference.clone()?,
        )),
        #[cfg(file_system)]
        FontData::Installed(source) => Some(installed_reference(&source.path)),
        FontData::Bundled(_) => None,
        #[cfg(test)]
        FontData::Loaded(_) => None,
    }
}
