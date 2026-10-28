use crate::fonts::{
    DEFAULT_FAMILY, FontBook, FontError, FontFaceStyle, FontFamily, FontId, FontNamespace,
    FontSelection, FontStyle, FontWeight, FontWidth, GenericFamily, GlyphMatch,
};
use crate::resources::ResourceResolver;
use crate::svg::usvg::{Font, FontProvider};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, OnceLock},
};

#[derive(Clone)]
pub(crate) struct SvgFonts {
    book: Arc<FontBook>,
    namespace: FontNamespace,
    registry: Arc<OnceLock<Registry>>,
}
struct Registry {
    ids: Vec<fontdb::ID>,
    originals: HashMap<fontdb::ID, FontId>,
    database: Arc<fontdb::Database>,
}
impl std::fmt::Debug for SvgFonts {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SvgFonts")
            .field("namespace", &self.namespace)
            .field(
                "registered_faces",
                &self.registry.get().map(|registry| registry.ids.len()),
            )
            .finish_non_exhaustive()
    }
}
impl SvgFonts {
    pub(crate) fn new(book: Arc<FontBook>) -> Self {
        Self {
            book,
            namespace: FontNamespace::ROOT,
            registry: Arc::default(),
        }
    }
    fn registry(&self) -> &Registry {
        self.registry.get_or_init(|| {
            const FACE_INDEX: u32 = 0;
            let mut database = fontdb::Database::new();
            let ids = self
                .book
                .faces_with_ids()
                .map(|(id, face)| {
                    let descriptor = face.descriptor();
                    // Matching belongs to FontBook. These records bind stable IDs to lazy bytes.
                    database.push_face_info(fontdb::FaceInfo {
                        id: fontdb::ID::dummy(),
                        source: fontdb::Source::Binary(Arc::new(Source {
                            book: self.book.clone(),
                            id,
                        })),
                        index: FACE_INDEX,
                        families: vec![(
                            descriptor.family.clone(),
                            fontdb::Language::English_UnitedStates,
                        )],
                        post_script_name: descriptor.family.clone(),
                        style: match descriptor.style {
                            FontFaceStyle::Italic => fontdb::Style::Italic,
                            FontFaceStyle::Oblique(_) => fontdb::Style::Oblique,
                            FontFaceStyle::Normal | FontFaceStyle::Auto => fontdb::Style::Normal,
                        },
                        weight: fontdb::Weight(
                            descriptor.weight.clamp(FontWeight::NORMAL).value() as u16
                        ),
                        stretch: fontdb::Stretch::Normal,
                        monospaced: false,
                    })
                })
                .collect::<Vec<_>>();
            let originals = self
                .book
                .faces_with_ids()
                .zip(ids.iter().copied())
                .map(|((id, _), mapped)| (mapped, id))
                .collect();
            Registry {
                ids,
                originals,
                database: Arc::new(database),
            }
        })
    }
    pub(crate) fn in_namespace(&self, namespace: FontNamespace) -> Self {
        Self {
            namespace,
            ..self.clone()
        }
    }
    pub(crate) fn database(&self) -> Arc<fontdb::Database> {
        self.registry().database.clone()
    }
    pub(crate) fn provider<'a>(&self, resolver: Option<&'a dyn ResourceResolver>) -> Provider<'a> {
        Provider {
            fonts: self.clone(),
            resolver,
            failure: RefCell::new(None),
            variations: RefCell::new(HashMap::new()),
            candidates: RefCell::new(HashMap::new()),
            coverage: RefCell::new(HashMap::new()),
        }
    }
    fn load(
        &self,
        id: FontId,
        resolver: Option<&dyn ResourceResolver>,
    ) -> Result<Option<fontdb::ID>, FontError> {
        let data = match resolver {
            Some(resolver) => self.book.resolve(id, resolver)?,
            None => self.book.resolve_cached(id)?,
        };
        Ok(data.and_then(|_| self.registry().ids.get(id.index()).copied()))
    }
}
impl Default for SvgFonts {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}

struct Source {
    book: Arc<FontBook>,
    id: FontId,
}
impl AsRef<[u8]> for Source {
    fn as_ref(&self) -> &[u8] {
        self.book
            .face(self.id)
            .and_then(|face| face.data())
            .unwrap_or_default()
    }
}

pub(crate) struct Provider<'a> {
    fonts: SvgFonts,
    resolver: Option<&'a dyn ResourceResolver>,
    failure: RefCell<Option<FontError>>,
    variations: RefCell<HashMap<Font, HashMap<fontdb::ID, Vec<crate::svg::usvg::FontVariation>>>>,
    candidates: RefCell<HashMap<Font, Arc<[FontId]>>>,
    coverage: RefCell<HashMap<(FontId, char), bool>>,
}
impl Provider<'_> {
    pub(crate) fn finish(self) -> Result<SvgFonts, FontError> {
        if let Some(error) = self.failure.into_inner() {
            return Err(error);
        }
        Ok(self.fonts)
    }
    fn select(
        &self,
        font: &Font,
        character: Option<char>,
        excluded: &[fontdb::ID],
        database: &mut Arc<fontdb::Database>,
    ) -> Option<fontdb::ID> {
        if self.failure.borrow().is_some() {
            return None;
        }
        match self.matching(font, character, excluded, database) {
            Ok(id) => id,
            Err(error) => {
                *self.failure.borrow_mut() = Some(error);
                None
            }
        }
    }
    fn matching(
        &self,
        font: &Font,
        character: Option<char>,
        excluded: &[fontdb::ID],
        database: &mut Arc<fontdb::Database>,
    ) -> Result<Option<fontdb::ID>, FontError> {
        let fonts = &self.fonts;
        let book = &fonts.book;
        *database = fonts.database();
        let selection = selection(font)?;
        let cached = self.candidates.borrow().get(font).cloned();
        let candidates = if let Some(candidates) = cached {
            candidates
        } else {
            let families = font
                .families()
                .iter()
                .map(|family| match family {
                    crate::svg::usvg::FontFamily::Serif => {
                        FontFamily::Generic(GenericFamily::Serif)
                    }
                    crate::svg::usvg::FontFamily::SansSerif => {
                        FontFamily::Generic(GenericFamily::SansSerif)
                    }
                    crate::svg::usvg::FontFamily::Monospace => {
                        FontFamily::Generic(GenericFamily::Monospace)
                    }
                    crate::svg::usvg::FontFamily::Cursive
                    | crate::svg::usvg::FontFamily::Fantasy => FontFamily::Generic(DEFAULT_FAMILY),
                    crate::svg::usvg::FontFamily::Named(name) => FontFamily::Named(name.clone()),
                })
                .collect::<Vec<_>>();
            let families = if font.uses_default_family() {
                vec![FontFamily::Generic(DEFAULT_FAMILY)]
            } else {
                families
            };
            let candidates = book.candidates_in_namespace(fonts.namespace, &families, selection);
            self.candidates
                .borrow_mut()
                .insert(font.clone(), candidates.clone());
            candidates
        };
        book.find_glyph(&candidates, |id| {
            let Some(mapped) = fonts.load(id, self.resolver)? else {
                return Ok::<_, FontError>(GlyphMatch::Unavailable);
            };
            if excluded.contains(&mapped) {
                return Ok(GlyphMatch::Missing);
            }
            if let Some(character) = character {
                let cached = self.coverage.borrow().get(&(id, character)).copied();
                let covered = if let Some(covered) = cached {
                    covered
                } else {
                    let Some(instance) = book.cached_instance(id, selection)? else {
                        return Ok(GlyphMatch::Unavailable);
                    };
                    let covered = instance
                        .face
                        .glyph_index(character)
                        .is_some_and(|glyph| glyph.0 != 0);
                    self.coverage.borrow_mut().insert((id, character), covered);
                    covered
                };
                if !covered {
                    return Ok(GlyphMatch::Missing);
                }
            }
            Ok(GlyphMatch::Found(mapped))
        })
        .map(|matched| matched.map(|(_, id)| id))
    }
}
impl FontProvider for Provider<'_> {
    fn variations(&self, font: &Font, mapped: fontdb::ID) -> Vec<crate::svg::usvg::FontVariation> {
        if let Some(values) = self
            .variations
            .borrow()
            .get(font)
            .and_then(|faces| faces.get(&mapped))
        {
            return values.clone();
        }
        let result = (|| {
            let id = self
                .fonts
                .registry()
                .originals
                .get(&mapped)
                .copied()
                .ok_or(FontError::NoAvailableFace)?;
            let instance = self
                .fonts
                .book
                .cached_instance(id, selection(font)?)?
                .ok_or(FontError::NoAvailableFace)?;
            let mut values = instance
                .variations
                .iter()
                .map(|value| crate::svg::usvg::FontVariation::new(value.tag, value.value))
                .collect::<Vec<_>>();
            for explicit in font.explicit_variations() {
                if let Some(value) = values.iter_mut().find(|value| value.tag == explicit.tag) {
                    *value = *explicit;
                } else {
                    values.push(*explicit);
                }
            }
            Ok::<_, FontError>(values)
        })();
        match result {
            Ok(values) => {
                self.variations
                    .borrow_mut()
                    .entry(font.clone())
                    .or_default()
                    .insert(mapped, values.clone());
                values
            }
            Err(error) => {
                *self.failure.borrow_mut() = Some(error);
                Vec::new()
            }
        }
    }
    fn select_font(&self, font: &Font, database: &mut Arc<fontdb::Database>) -> Option<fontdb::ID> {
        self.select(font, None, &[], database)
    }
    fn select_fallback(
        &self,
        font: &Font,
        character: char,
        excluded: &[fontdb::ID],
        database: &mut Arc<fontdb::Database>,
    ) -> Option<fontdb::ID> {
        self.select(font, Some(character), excluded, database)
    }
}

fn selection(font: &Font) -> Result<FontSelection, FontError> {
    let mut selection = FontSelection::new(
        FontWeight::new(f32::from(font.weight()))?,
        match font.style() {
            crate::svg::usvg::FontStyle::Normal => FontStyle::Normal,
            crate::svg::usvg::FontStyle::Italic => FontStyle::Italic,
            crate::svg::usvg::FontStyle::Oblique => FontStyle::Oblique,
        },
    );
    selection.width = match font.stretch() {
        crate::svg::usvg::FontStretch::UltraCondensed => FontWidth::ULTRA_CONDENSED,
        crate::svg::usvg::FontStretch::ExtraCondensed => FontWidth::EXTRA_CONDENSED,
        crate::svg::usvg::FontStretch::Condensed => FontWidth::CONDENSED,
        crate::svg::usvg::FontStretch::SemiCondensed => FontWidth::SEMI_CONDENSED,
        crate::svg::usvg::FontStretch::Normal => FontWidth::NORMAL,
        crate::svg::usvg::FontStretch::SemiExpanded => FontWidth::SEMI_EXPANDED,
        crate::svg::usvg::FontStretch::Expanded => FontWidth::EXPANDED,
        crate::svg::usvg::FontStretch::ExtraExpanded => FontWidth::EXTRA_EXPANDED,
        crate::svg::usvg::FontStretch::UltraExpanded => FontWidth::ULTRA_EXPANDED,
    };
    Ok(selection)
}
