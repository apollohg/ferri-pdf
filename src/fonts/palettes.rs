//! COLR palette selection from the font's CPAL table, `font-palette` and
//! `@font-palette-values` (CSS Fonts 4 §9).
use crate::fonts::{FontBook, FontNamespace, matching};
use crate::paint::Color;
use serde::Serialize;
use skrifa::raw::{FontRef, TableProvider, tables::cpal::PaletteType};
use std::sync::Arc;

const DEFAULT_PALETTE: usize = 0;
const CHANNEL_MAX: f32 = 255.0;

/// A computed `font-palette`; palette names resolve per face family (§9.1).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FontPalette {
    #[default]
    Normal,
    Light,
    Dark,
    Named(Box<str>),
}

/// The `base-palette` descriptor (§9.2.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BasePalette {
    Light,
    Dark,
    Index(u32),
}

#[derive(Clone, Debug)]
pub(super) struct PaletteValues {
    namespace: FontNamespace,
    name: Box<str>,
    families: Vec<String>,
    base: BasePalette,
    overrides: Vec<(u32, Color)>,
}

/// The palettes of a face's CPAL table.
struct Palettes<'a> {
    count: usize,
    entries: usize,
    indices: &'a [skrifa::raw::types::BigEndian<u16>],
    records: &'a [skrifa::raw::tables::cpal::ColorRecord],
    types: Option<&'a [skrifa::raw::types::BigEndian<PaletteType>]>,
}
impl<'a> Palettes<'a> {
    fn of(font: &'a FontRef<'a>) -> Option<Self> {
        let cpal = font.cpal().ok()?;
        Some(Self {
            count: usize::from(cpal.num_palettes()),
            entries: usize::from(cpal.num_palette_entries()),
            indices: cpal.color_record_indices(),
            records: cpal.color_records_array()?.ok()?,
            types: cpal.palette_types_array().and_then(Result::ok),
        })
    }
    /// The first palette marked for `background`, else the default (§9.1, §9.2.2).
    fn marked(&self, background: PaletteType) -> usize {
        self.types
            .and_then(|types| {
                types
                    .iter()
                    .position(|kind| kind.get().contains(background))
            })
            .unwrap_or(DEFAULT_PALETTE)
    }
    fn base(&self, base: BasePalette) -> usize {
        match base {
            BasePalette::Light => self.marked(PaletteType::USABLE_WITH_LIGHT_BACKGROUND),
            BasePalette::Dark => self.marked(PaletteType::USABLE_WITH_DARK_BACKGROUND),
            BasePalette::Index(index) => usize::try_from(index)
                .ok()
                .filter(|index| *index < self.count)
                .unwrap_or(DEFAULT_PALETTE),
        }
    }
    fn colors(&self, palette: usize) -> Option<Vec<Color>> {
        let start = usize::from(self.indices.get(palette)?.get());
        let records = self.records.get(start..start + self.entries)?;
        Some(
            records
                .iter()
                .map(|record| Color {
                    rgb: [record.red(), record.green(), record.blue()],
                    alpha: f32::from(record.alpha()) / CHANNEL_MAX,
                })
                .collect(),
        )
    }
}

impl FontBook {
    /// Registered in document order; a later rule with the same name replaces an earlier one
    /// for the families both name.
    pub(crate) fn define_palette(
        &mut self,
        namespace: FontNamespace,
        name: &str,
        families: Vec<String>,
        base: BasePalette,
        overrides: Vec<(u32, Color)>,
    ) {
        self.palettes.push(PaletteValues {
            namespace,
            name: name.into(),
            families,
            base,
            overrides,
        });
    }

    fn palette_values(
        &self,
        mut namespace: FontNamespace,
        name: &str,
        family: &str,
    ) -> Option<&PaletteValues> {
        loop {
            if let Some(found) = self.palettes.iter().rev().find(|palette| {
                palette.namespace == namespace
                    && *palette.name == *name
                    && palette
                        .families
                        .iter()
                        .any(|candidate| matching::names_match(candidate, family))
            }) {
                return Some(found);
            }
            if namespace == FontNamespace::ROOT {
                return None;
            }
            namespace = self.namespaces[namespace.0].expect("child font namespace has a parent");
        }
    }

    /// The colours a COLR face of `family` renders with under `palette`, or `None` for the
    /// font's default palette or a face without CPAL palettes.
    pub(crate) fn palette(
        &self,
        namespace: FontNamespace,
        palette: &FontPalette,
        family: &str,
        data: &[u8],
    ) -> Option<Arc<[Color]>> {
        let (base, overrides) = match palette {
            FontPalette::Normal => return None,
            FontPalette::Light => (BasePalette::Light, &[][..]),
            FontPalette::Dark => (BasePalette::Dark, &[][..]),
            FontPalette::Named(name) => {
                let values = self.palette_values(namespace, name, family)?;
                (values.base, values.overrides.as_slice())
            }
        };
        let font = FontRef::new(data).ok()?;
        let palettes = Palettes::of(&font)?;
        let mut colors = palettes.colors(palettes.base(base))?;
        // Keys outside the palette are ignored; the last of repeated keys wins (§9.2.3).
        for (index, color) in overrides {
            if let Some(entry) = usize::try_from(*index)
                .ok()
                .and_then(|index| colors.get_mut(index))
            {
                *entry = *color;
            }
        }
        Some(colors.into())
    }
}
