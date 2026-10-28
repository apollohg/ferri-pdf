//! CSS Text Decoration 3/4: decoration lines and their propagation to descendants.
use crate::style::{
    LengthPercentage, StyleError,
    box_values::{length_percentage, unsupported},
    component::Component,
    grammar::Keyword,
    units::Relative,
};
use ferri_pdf_paint::{Color, DecorationStyle};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct DecorationLines {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
}
impl DecorationLines {
    pub(crate) fn is_empty(self) -> bool {
        !(self.underline || self.overline || self.line_through)
    }
    /// `none | [underline || overline || line-through || blink]`; UAs need not blink.
    /// The spelling and grammar error indications are unsupported.
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let mut lines = Self::default();
        for keyword in keywords {
            match keyword {
                Keyword::Underline => lines.underline = true,
                Keyword::Overline => lines.overline = true,
                Keyword::LineThrough => lines.line_through = true,
                Keyword::None | Keyword::Blink => {}
                _ => return Err(unsupported(value)),
            }
        }
        Ok(lines)
    }
}

pub(super) fn decoration_style(value: &Component) -> Result<DecorationStyle, StyleError> {
    match value.keyword() {
        Some(Keyword::Solid) => Ok(DecorationStyle::Solid),
        Some(Keyword::Double) => Ok(DecorationStyle::Double),
        Some(Keyword::Dotted) => Ok(DecorationStyle::Dotted),
        Some(Keyword::Dashed) => Ok(DecorationStyle::Dashed),
        Some(Keyword::Wavy) => Ok(DecorationStyle::Wavy),
        _ => Err(unsupported(value)),
    }
}

/// `None` takes the thickness from the decorating box's font (`auto` and `from-font`);
/// percentages refer to the element's font size.
pub(super) fn thickness(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<f32>, StyleError> {
    match value.keyword() {
        Some(Keyword::Auto | Keyword::FromFont) => Ok(None),
        _ => Ok(Some(crate::style::box_values::font_length_percentage(
            value, relative,
        )?)),
    }
}

/// `auto` is `None`; percentages stay relative to each element's font size.
pub(super) fn underline_offset(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<LengthPercentage>, StyleError> {
    match value.keyword() {
        Some(Keyword::Auto) => Ok(None),
        _ => length_percentage(value, relative).map(Some),
    }
}

/// The decoration lines one element specifies, with its colour resolved.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct TextDecoration {
    pub lines: DecorationLines,
    pub style: DecorationStyle,
    pub color: Color,
    pub thickness: Option<f32>,
}
impl Default for TextDecoration {
    fn default() -> Self {
        Self {
            lines: DecorationLines::default(),
            style: DecorationStyle::Solid,
            color: Color::BLACK,
            thickness: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum UnderlinePosition {
    /// The font's underline position (`auto`, `from-font`, and `left`/`right` in horizontal text).
    #[default]
    Font,
    /// Below the decorating box's descent.
    Under,
}
impl UnderlinePosition {
    /// `auto | from-font | [under || [left | right]]`; left and right only affect vertical text.
    pub(super) fn from_component(value: &Component) -> Self {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        if keywords.contains(&Keyword::Under) {
            Self::Under
        } else {
            Self::Font
        }
    }
}

/// Text decoration state of a box.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct TextDecorations {
    /// Lines this element specifies; it decorates its own content with them.
    pub declared: TextDecoration,
    /// Lines propagated from ancestors, outermost first. A block container's root inline
    /// box paints these; inline boxes leave them to their ancestors.
    pub propagated: Vec<TextDecoration>,
    pub underline_position: UnderlinePosition,
    /// `None` is `auto`.
    pub underline_offset: Option<LengthPercentage>,
}
impl TextDecorations {
    pub(super) fn propagated(&self) -> Vec<TextDecoration> {
        let mut propagated = self.propagated.clone();
        if !self.declared.lines.is_empty() {
            propagated.push(self.declared);
        }
        propagated
    }
    /// Lines a block container's root inline box paints, outermost first.
    pub(crate) fn applied(&self) -> impl Iterator<Item = &TextDecoration> {
        self.propagated
            .iter()
            .chain((!self.declared.lines.is_empty()).then_some(&self.declared))
    }
}
