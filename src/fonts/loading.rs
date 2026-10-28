use crate::fonts::{
    FontBook, FontData, FontError, FontFaceStyle, FontId, FontInstance, FontRange, FontSelection,
    FontSlant, FontSource, FontSources, FontStyle, FontVariation, FontWeight, matching, validate,
};
use crate::resources::ResourceResolver;

const WEIGHT_AXIS: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"wght");
const WIDTH_AXIS: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"wdth");
const SLANT_AXIS: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"slnt");
const ITALIC_AXIS: ttf_parser::Tag = ttf_parser::Tag::from_bytes(b"ital");

impl FontBook {
    pub(crate) fn instance(
        &self,
        id: FontId,
        requested: FontSelection,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<FontInstance<'_>>, FontError> {
        self.instance_data(id, requested, self.resolve(id, resolver)?)
    }

    /// Measures a face whose source list has already been attempted in this job.
    pub(crate) fn cached_instance(
        &self,
        id: FontId,
        requested: FontSelection,
    ) -> Result<Option<FontInstance<'_>>, FontError> {
        self.instance_data(id, requested, self.resolve_cached(id)?)
    }

    fn instance_data<'a>(
        &'a self,
        id: FontId,
        requested: FontSelection,
        data: Option<&'a [u8]>,
    ) -> Result<Option<FontInstance<'a>>, FontError> {
        let Some(data) = data else {
            return Ok(None);
        };
        let descriptor = &self.face(id).ok_or(FontError::UnknownFont(id))?.descriptor;
        let mut face = ttf_parser::Face::parse(data, 0).map_err(FontError::InvalidFont)?;
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
            },
            FontFaceStyle::Italic => (true, FontSlant::NORMAL),
            other => (false, matching::style_match(other, requested).1),
        };
        let mut weight = match descriptor.weight {
            FontRange::Auto => FontWeight::NORMAL.value(),
            range @ FontRange::Values { .. } => range.clamp(requested.weight).value(),
        };
        let mut style = match descriptor.style {
            FontFaceStyle::Italic => FontStyle::Italic,
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
        for variation in &variations {
            face.set_variation(ttf_parser::Tag::from_bytes(&variation.tag), variation.value)
                .ok_or(FontError::VariationAxis(variation.tag))?;
        }
        Ok(Some(FontInstance {
            face,
            variations: variations.into(),
            weight,
            style,
        }))
    }

    pub(crate) fn resolve(
        &self,
        id: FontId,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<&[u8]>, FontError> {
        let face = self.face(id).ok_or(FontError::UnknownFont(id))?;
        if let FontData::Lazy { sources, data } = &face.data
            && data.get().is_none()
        {
            let loaded = self.load_sources(sources, resolver)?;
            let _ = data.set(loaded);
        }
        Ok(face.data())
    }

    /// Reads an already attempted face without acquiring additional resources.
    pub(crate) fn resolve_cached(&self, id: FontId) -> Result<Option<&[u8]>, FontError> {
        let face = self.face(id).ok_or(FontError::UnknownFont(id))?;
        if let FontData::Lazy { data, .. } = &face.data
            && data.get().is_none()
        {
            return Err(FontError::UnresolvedFont(id));
        }
        Ok(face.data())
    }

    fn load_sources(
        &self,
        sources: &FontSources,
        resolver: &dyn ResourceResolver,
    ) -> Result<Option<Vec<u8>>, FontError> {
        for source in &sources.sources {
            let data = match source {
                FontSource::Local(name) => self.local_data(name).map(<[u8]>::to_vec),
                FontSource::Url(reference) => match resolver.resolve(reference, &sources.base) {
                    Ok(resource) => Some(resource.bytes),
                    Err(error) if error.is_unavailable() => None,
                    Err(error) => return Err(error.into()),
                },
            };
            let Some(data) = data else {
                continue;
            };
            match validate(&data) {
                Ok(()) => return Ok(Some(data)),
                Err(FontError::InvalidFont(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    fn local_data(&self, name: &str) -> Option<&[u8]> {
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
