// Copyright 2024 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::Arc;

use fontdb::{Database, ID};
use svgtypes::FontFamily;

use self::layout::DatabaseExt;
use crate::svg::usvg::{Cache, Font, FontStyle, Text};

pub(crate) mod flatten;

mod colr;
/// Provides access to the layout of a text node.
pub(crate) mod layout;
mod wrap;

pub(crate) fn font_variations(
    face: &ttf_parser::Face<'_>,
    variations: &[crate::svg::usvg::FontVariation],
    font_size: f32,
    optical_sizing: crate::svg::usvg::FontOpticalSizing,
) -> Vec<rustybuzz::Variation> {
    use crate::fonts::OPTICAL_SIZE_AXIS;
    let mut values: Vec<_> = variations
        .iter()
        .map(|value| rustybuzz::Variation {
            tag: ttf_parser::Tag::from_bytes(&value.tag),
            value: value.value,
        })
        .collect();
    if optical_sizing == crate::svg::usvg::FontOpticalSizing::Auto
        && !values.iter().any(|value| value.tag == OPTICAL_SIZE_AXIS)
        && face
            .variation_axes()
            .into_iter()
            .any(|axis| axis.tag == OPTICAL_SIZE_AXIS)
    {
        values.push(rustybuzz::Variation {
            tag: OPTICAL_SIZE_AXIS,
            value: font_size,
        });
    }
    values
}

/// A shorthand for [FontResolver]'s font selection function.
///
/// This function receives a font specification (families + a style, weight,
/// stretch triple) and a font database and should return the ID of the font
/// that shall be used (if any).
///
/// In the basic case, the function will search the existing fonts in the
/// database to find a good match, e.g. via
/// [`Database::query`](fontdb::Database::query). This is what the [default
/// implementation](FontResolver::default_font_selector) does.
///
/// Users with more complex requirements can mutate the database to load
/// additional fonts dynamically. To perform mutation, it is recommended to call
/// `Arc::make_mut` on the provided database. (This call is not done outside of
/// the callback to not needless clone an underlying shared database if no
/// mutation will be performed.) It is important that the database is only
/// mutated additively. Removing fonts or replacing the entire database will
/// break things.
pub(crate) type FontSelectionFn<'a> =
    Box<dyn Fn(&Font, &mut Arc<Database>) -> Option<ID> + Send + Sync + 'a>;

/// A shorthand for [FontResolver]'s fallback selection function.
///
/// This function receives a specific character, a list of already used fonts,
/// and a font database. It should return the ID of a font that
/// - is not any of the already used fonts
/// - is as close as possible to the first already used font (if any)
/// - supports the given character
///
/// The function can search the existing database, but can also load additional
/// fonts dynamically. See the documentation of [`FontSelectionFn`] for more
/// details.
pub(crate) type FallbackSelectionFn<'a> =
    Box<dyn Fn(char, &[ID], &mut Arc<Database>) -> Option<ID> + Send + Sync + 'a>;

/// A font resolver for `<text>` elements.
///
/// This type can be useful if you want to have an alternative font handling to
/// the default one. By default, only fonts specified upfront in
/// [`Options::fontdb`](crate::svg::usvg::Options::fontdb) will be used. This type allows
/// you to load additional fonts on-demand and customize the font selection
/// process.
pub struct FontResolver<'a> {
    /// Resolver function that will be used when selecting a specific font
    /// for a generic [`Font`] specification.
    pub select_font: FontSelectionFn<'a>,

    /// Resolver function that will be used when selecting a fallback font for a
    /// character.
    pub select_fallback: FallbackSelectionFn<'a>,
}

/// What the document's fonts give a face rendering a span's text (CSS Fonts 4 §7.2, §9.5).
#[derive(Clone, Debug, Default)]
pub struct FaceSettings {
    /// The `font-feature-settings` descriptor of the face's `@font-face` rule.
    pub(crate) descriptor: Arc<[crate::fonts::FontFeature]>,
    /// `font-variant-alternates` as the face's family defines it.
    pub(crate) alternates: Vec<crate::fonts::FontFeature>,
}

/// Synchronous caller-owned font resolution, including the original fallback request.
/// Borrowed providers need not be thread-safe.
pub trait FontProvider {
    /// What the document's fonts give face `id`, matched for `font`, rendering text whose
    /// computed style is `style`.
    #[expect(
        private_interfaces,
        reason = "the document's computed style is crate-private; only it provides faces"
    )]
    fn face_settings(&self, _: &Font, _: ID, _: &crate::style::Style) -> FaceSettings {
        FaceSettings::default()
    }
    /// Selected-face axes followed by explicit variation settings.
    fn variations(&self, font: &Font, _: ID) -> Vec<crate::svg::usvg::FontVariation> {
        font.variations().to_vec()
    }
    /// Selects the primary face for a font request.
    fn select_font(&self, font: &Font, database: &mut Arc<Database>) -> Option<ID>;
    /// Selects coverage after the listed faces failed to shape a character.
    fn select_fallback(
        &self,
        font: &Font,
        character: char,
        excluded: &[ID],
        database: &mut Arc<Database>,
    ) -> Option<ID>;
}

impl FontProvider for FontResolver<'_> {
    fn select_font(&self, font: &Font, database: &mut Arc<Database>) -> Option<ID> {
        (self.select_font)(font, database)
    }
    fn select_fallback(
        &self,
        _: &Font,
        character: char,
        excluded: &[ID],
        database: &mut Arc<Database>,
    ) -> Option<ID> {
        (self.select_fallback)(character, excluded, database)
    }
}

impl Default for FontResolver<'_> {
    fn default() -> Self {
        FontResolver {
            select_font: FontResolver::default_font_selector(),
            select_fallback: FontResolver::default_fallback_selector(),
        }
    }
}

impl FontResolver<'_> {
    /// Creates a default font selection resolver.
    ///
    /// The default implementation forwards to
    /// [`query`](fontdb::Database::query) on the font database specified in the
    /// [`Options`](crate::svg::usvg::Options).
    pub fn default_font_selector() -> FontSelectionFn<'static> {
        Box::new(move |font, fontdb| {
            let mut name_list = Vec::new();
            for family in &font.families {
                name_list.push(match family {
                    FontFamily::Serif => fontdb::Family::Serif,
                    FontFamily::SansSerif => fontdb::Family::SansSerif,
                    FontFamily::Cursive => fontdb::Family::Cursive,
                    FontFamily::Fantasy => fontdb::Family::Fantasy,
                    FontFamily::Monospace => fontdb::Family::Monospace,
                    FontFamily::Named(s) => fontdb::Family::Name(s),
                });
            }

            // Use the default font as fallback.
            name_list.push(fontdb::Family::Serif);

            let stretch = crate::svg::usvg::nearest_stretch(font.width);

            let style = match font.style {
                FontStyle::Normal => fontdb::Style::Normal,
                FontStyle::Italic | FontStyle::Left | FontStyle::Right => fontdb::Style::Italic,
                FontStyle::Oblique => fontdb::Style::Oblique,
            };

            let query = fontdb::Query {
                families: &name_list,
                weight: fontdb::Weight(font.weight.value().round() as u16),
                stretch,
                style,
            };

            let id = fontdb.query(&query);
            if id.is_none() {
                log::warn!(
                    "No match for '{}' font-family.",
                    font.families
                        .iter()
                        .map(|f| f.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }

            id
        })
    }

    /// Creates a default font fallback selection resolver.
    ///
    /// The default implementation searches through the entire `fontdb`
    /// to find a font that has the correct style and supports the character.
    pub fn default_fallback_selector() -> FallbackSelectionFn<'static> {
        Box::new(|c, exclude_fonts, fontdb| {
            let base_font_id = exclude_fonts[0];

            // Iterate over fonts and check if any of them support the specified char.
            for face in fontdb.faces() {
                // Ignore fonts, that were used for shaping already.
                if exclude_fonts.contains(&face.id) {
                    continue;
                }

                // Check that the new face has the same style.
                let base_face = fontdb.face(base_font_id)?;
                if base_face.style != face.style
                    && base_face.weight != face.weight
                    && base_face.stretch != face.stretch
                {
                    continue;
                }

                if !fontdb.has_char(face.id, c) {
                    continue;
                }

                let base_family = base_face
                    .families
                    .iter()
                    .find(|f| f.1 == fontdb::Language::English_UnitedStates)
                    .unwrap_or(&base_face.families[0]);

                let new_family = face
                    .families
                    .iter()
                    .find(|f| f.1 == fontdb::Language::English_UnitedStates)
                    .unwrap_or(&base_face.families[0]);

                log::warn!("Fallback from {} to {}.", base_family.0, new_family.0);
                return Some(face.id);
            }

            None
        })
    }
}

impl std::fmt::Debug for FontResolver<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FontResolver { .. }")
    }
}

/// Convert a text into its paths. This is done in two steps:
/// 1. We convert the text into glyphs and position them according to the rules specified
///    in the SVG specification. While doing so, we also calculate the text bbox (which
///    is not based on the outlines of a glyph, but instead the glyph metrics as well
///    as decoration spans).
/// 2. We convert all of the positioned glyphs into outlines.
///
/// Returns the layers the text's shadows cast beneath it.
pub(crate) fn convert(
    text: &mut Text,
    resolver: &dyn FontProvider,
    cache: &mut Cache,
) -> Option<Vec<crate::svg::usvg::Node>> {
    let (text_fragments, bbox, clip) = layout::layout_text(text, resolver, &mut cache.fontdb)?;
    text.layouted = text_fragments;
    text.clip = clip;
    text.bounding_box = bbox.to_rect();
    text.abs_bounding_box = bbox.transform(text.abs_transform)?.to_rect();

    let (group, stroke_bbox, shadows) = flatten::flatten(text, cache)?;
    text.flattened = Box::new(group);
    text.stroke_bounding_box = stroke_bbox.to_rect();
    text.abs_stroke_bounding_box = stroke_bbox.transform(text.abs_transform)?.to_rect();

    Some(shadows)
}
