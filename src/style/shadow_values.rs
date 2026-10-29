//! CSS Borders 4 §6: coordinated shadow lists, with offsets determining the layer count.
use crate::paint::{AbsoluteColor, BoxShadow};
use crate::style::{
    StyleError, box_values::unsupported, component::Component, grammar::Keyword,
    text_paint_values::InheritedColor, units::Relative, values::length,
};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Shadows {
    pub colors: Arc<[InheritedColor]>,
    pub offsets: Arc<[Option<[f32; 2]>]>,
    pub blurs: Arc<[f32]>,
    pub spreads: Arc<[f32]>,
    pub positions: Arc<[bool]>,
}
impl Default for Shadows {
    fn default() -> Self {
        Self {
            colors: Arc::from([InheritedColor::default()]),
            offsets: Arc::from([None]),
            blurs: Arc::from([0.0]),
            spreads: Arc::from([0.0]),
            positions: Arc::from([false]),
        }
    }
}
impl Shadows {
    pub(super) fn resolve(
        &mut self,
        relative: &Relative<'_>,
        current: &AbsoluteColor,
    ) -> Result<(), StyleError> {
        for color in Arc::make_mut(&mut self.colors) {
            color.resolve(relative, current)?;
        }
        Ok(())
    }
    pub(super) fn layers(&self) -> Vec<BoxShadow> {
        self.offsets
            .iter()
            .enumerate()
            .filter_map(|(index, offset)| {
                Some(BoxShadow {
                    offset: (*offset)?,
                    color: self.colors[index % self.colors.len()].used,
                    blur: self.blurs[index % self.blurs.len()],
                    spread: self.spreads[index % self.spreads.len()],
                    inset: self.positions[index % self.positions.len()],
                })
            })
            .collect()
    }
}

pub(super) fn colors(
    value: &Component,
    relative: &Relative<'_>,
    current: &AbsoluteColor,
) -> Result<Arc<[InheritedColor]>, StyleError> {
    value
        .items()
        .iter()
        .map(|item| InheritedColor::compute(item, relative, current))
        .collect()
}
pub(super) fn lengths(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Arc<[f32]>, StyleError> {
    value
        .items()
        .iter()
        .map(|item| length(item, relative))
        .collect()
}
pub(super) fn offsets(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Arc<[Option<[f32; 2]>]>, StyleError> {
    value
        .items()
        .iter()
        .map(|item| {
            if item.keyword() == Some(Keyword::None) {
                return Ok(None);
            }
            let lengths = item.items();
            let first = length(lengths.first().ok_or_else(|| unsupported(item))?, relative)?;
            let second = lengths
                .get(1)
                .map(|item| length(item, relative))
                .transpose()?
                .unwrap_or(first);
            Ok(Some([first, second]))
        })
        .collect()
}
pub(super) fn positions(value: &Component) -> Result<Arc<[bool]>, StyleError> {
    value
        .items()
        .iter()
        .map(|item| match item.keyword() {
            Some(Keyword::Inset) => Ok(true),
            Some(Keyword::Outset) => Ok(false),
            _ => Err(unsupported(item)),
        })
        .collect()
}
