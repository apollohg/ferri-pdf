//! `initial-letter` and `initial-letter-align` (CSS Inline 3 §§7.3–7.4).
use crate::style::{
    StyleError, box_values::unsupported, component::Component, grammar::Keyword, units::Relative,
};
use serde::Serialize;

/// The size and sink of an initial letter, in lines.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct InitialLetter {
    pub size: f32,
    pub sink: u32,
}
impl InitialLetter {
    /// `None` for `normal`. An omitted sink is `drop`, the size floored to a positive whole
    /// number; `raise` is a sink of one.
    pub(super) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Option<Self>, StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        if keywords == [Keyword::Normal] {
            return Ok(None);
        }
        let operands = operands(value);
        let size = operands
            .first()
            .map(|size| crate::style::values::number(size, relative))
            .transpose()?
            .ok_or_else(|| unsupported(value))?;
        let drop = || size.floor().max(1.0) as u32;
        let sink = match (operands.get(1), keywords.first()) {
            (Some(sink), _) => u32::try_from(crate::style::values::integer(sink, relative)?)
                .map_err(|_| unsupported(value))?,
            (None, Some(Keyword::Raise)) => 1,
            (None, Some(Keyword::Drop) | None) => drop(),
            _ => return Err(unsupported(value)),
        };
        Ok(Some(Self { size, sink }))
    }
}

/// The non-keyword values of a match tree, in source order.
fn operands(value: &Component) -> Vec<&Component> {
    match value.unwrap() {
        Component::Group(items) => items.iter().flat_map(operands).collect(),
        Component::Slots(slots) => slots.iter().flatten().flat_map(operands).collect(),
        Component::Keyword(_) => Vec::new(),
        other => vec![other],
    }
}

/// The surrounding text's alignment points an initial letter aligns to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum LetterAlignment {
    /// The cap height and the alphabetic baseline.
    #[default]
    Alphabetic,
    /// The ideographic-ink-over and ideographic-ink-under baselines.
    Ideographic,
    /// The hanging and alphabetic baselines.
    Hanging,
    /// The over and under half-leading edges.
    Leading,
}

/// A computed `initial-letter-align`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct InitialLetterAlign {
    /// The letter aligns its border box edges, rather than points its contents determine.
    pub border_box: bool,
    pub alignment: LetterAlignment,
}
impl InitialLetterAlign {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let mut align = Self::default();
        for keyword in keywords {
            match keyword {
                Keyword::BorderBox => align.border_box = true,
                Keyword::Alphabetic => align.alignment = LetterAlignment::Alphabetic,
                Keyword::Ideographic => align.alignment = LetterAlignment::Ideographic,
                Keyword::Hanging => align.alignment = LetterAlignment::Hanging,
                Keyword::Leading => align.alignment = LetterAlignment::Leading,
                _ => return Err(unsupported(value)),
            }
        }
        Ok(align)
    }
}
