use super::{
    converter::State,
    svgtree::{AId, SvgNode},
    text,
};
use crate::fonts::{self, FaceMetrics, SizeMetric};
use crate::style::{
    StyleError,
    svg_cascade::{FontUnit, FontUnits},
};
use crate::svg::usvg::{Font, FontOpticalSizing, FontProvider, WritingMode};
use std::{cell::OnceCell, sync::Arc};

pub(super) struct FontLengths<'a> {
    font: Option<Font>,
    size: f32,
    optical_size: f32,
    optical_sizing: FontOpticalSizing,
    vertical: [bool; 2],
    provider: &'a dyn FontProvider,
    database: Arc<fontdb::Database>,
    first: OnceCell<FaceMetrics>,
    advances: [OnceCell<Option<f32>>; 2],
}

impl<'a> FontLengths<'a> {
    pub(super) fn new(node: Option<SvgNode>, size: f32, state: &'a State) -> Self {
        let vertical =
            node.is_some_and(|node| text::convert_writing_mode(node) == WritingMode::TopToBottom);
        let orientation = node
            .and_then(|node| node.find_attribute::<&str>(AId::TextOrientation))
            .and_then(TextOrientation::from_name)
            .unwrap_or_default();
        use crate::style::logical::TextOrientation;
        Self {
            font: node.map(|node| text::convert_font(node, state)),
            size,
            optical_size: size
                * node.map_or(crate::style::INITIAL_ZOOM, |node| node.effective_zoom()),
            optical_sizing: if node
                .and_then(|node| node.find_attribute::<&str>(AId::FontOpticalSizing))
                == Some("none")
            {
                FontOpticalSizing::None
            } else {
                FontOpticalSizing::Auto
            },
            vertical: [
                vertical && orientation == TextOrientation::Upright,
                vertical && orientation != TextOrientation::Sideways,
            ],
            provider: state.font_provider.unwrap_or(&state.opt.font_resolver),
            database: state.opt.fontdb.clone(),
            first: OnceCell::new(),
            advances: Default::default(),
        }
    }

    fn measure<T>(
        &self,
        character: Option<char>,
        measure: impl Fn(&ttf_parser::Face<'_>) -> Option<T>,
    ) -> Option<T> {
        let font = self.font.as_ref()?;
        let mut database = self.database.clone();
        let mut id = self.provider.select_font(font, &mut database)?;
        let mut excluded = Vec::new();
        loop {
            let variations = self.provider.variations(font, id);
            let result = database
                .with_face_data(id, |bytes, index| {
                    let mut face = ttf_parser::Face::parse(bytes, index).ok()?;
                    for variation in crate::svg::usvg::text::font_variations(
                        &face,
                        &variations,
                        self.optical_size,
                        self.optical_sizing,
                    ) {
                        face.set_variation(variation.tag, variation.value);
                    }
                    measure(&face)
                })
                .flatten();
            if result.is_some() {
                return result;
            }
            let character = character?;
            excluded.push(id);
            id = self
                .provider
                .select_fallback(font, character, &excluded, &mut database)?;
            if excluded.contains(&id) {
                return None;
            }
        }
    }

    fn metrics(&self) -> &FaceMetrics {
        self.first.get_or_init(|| {
            self.measure(None, |face| Some(FaceMetrics::of(face)))
                .unwrap_or_else(|| fonts::initial_font().metrics())
        })
    }

    fn advance(&self, index: usize, character: char) -> Option<f32> {
        *self.advances[index].get_or_init(|| {
            if self.font.is_none() {
                fonts::initial_font().advance((character, self.vertical[index]))
            } else {
                self.measure(Some(character), |face| {
                    fonts::advance_in_axis(face, character, self.vertical[index])
                })
            }
        })
    }
    pub(super) fn length(&self, unit: FontUnit) -> f32 {
        let em = match unit {
            FontUnit::Em => 1.0,
            FontUnit::Ex => self.metrics().x_height,
            FontUnit::Cap => self.metrics().cap_height,
            FontUnit::Ch => self.advance(0, fonts::ZERO).unwrap_or(if self.vertical[0] {
                fonts::FALLBACK_IDEOGRAPH_ADVANCE_EM
            } else {
                fonts::FALLBACK_ZERO_ADVANCE_EM
            }),
            FontUnit::Ic => self
                .advance(1, fonts::WATER_IDEOGRAPH)
                .unwrap_or(fonts::FALLBACK_IDEOGRAPH_ADVANCE_EM),
            FontUnit::Lh => self.metrics().normal_line_height(),
        };
        self.size * em
    }
}

impl FontUnits for FontLengths<'_> {
    fn per_unit(&self, unit: FontUnit) -> Result<f32, StyleError> {
        Ok(self.length(unit) * crate::paint::POINTS_PER_PIXEL)
    }

    fn size_metric(&self, metric: SizeMetric) -> Result<f32, StyleError> {
        Ok(self.metrics().size_metric(metric))
    }
}
