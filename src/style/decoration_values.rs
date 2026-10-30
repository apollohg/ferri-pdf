//! CSS Text Decoration 3/4: decoration lines and their propagation to descendants.
use crate::paint::{Color, DecorationStyle};
use crate::style::{
    LengthPercentage, StyleError,
    box_values::{length_percentage, unsupported},
    component::Component,
    grammar::Keyword,
    units::Relative,
    values::length,
};
use serde::Serialize;

/// The UA-defined appearance of an error indication (CSS Text Decoration 4 §2.1): a wavy
/// underline in red for spelling and green for grammar.
const SPELLING_ERROR_COLOR: Color = Color {
    rgb: [255, 0, 0],
    alpha: 1.0,
};
const GRAMMAR_ERROR_COLOR: Color = Color {
    rgb: [0, 128, 0],
    alpha: 1.0,
};
const ERROR_STYLE: DecorationStyle = DecorationStyle::Wavy;

/// `spelling-error` or `grammar-error`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum ErrorIndication {
    Spelling,
    Grammar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct DecorationLines {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
    pub error: Option<ErrorIndication>,
}
impl DecorationLines {
    pub(crate) fn is_empty(self) -> bool {
        !(self.underline || self.overline || self.line_through || self.error.is_some())
    }
    /// `none | [underline || overline || line-through || blink] | spelling-error |
    /// grammar-error`; UAs need not blink.
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let mut lines = Self::default();
        for keyword in keywords {
            match keyword {
                Keyword::Underline => lines.underline = true,
                Keyword::Overline => lines.overline = true,
                Keyword::LineThrough => lines.line_through = true,
                Keyword::SpellingError => lines.error = Some(ErrorIndication::Spelling),
                Keyword::GrammarError => lines.error = Some(ErrorIndication::Grammar),
                Keyword::None | Keyword::Blink => {}
                _ => return Err(unsupported(value)),
            }
        }
        Ok(lines)
    }
}

impl TextDecoration {
    /// The style and colour of this decoration's lines: an error indication disregards
    /// `text-decoration-style` and `text-decoration-color`, as CSS Text Decoration 4 §2.1
    /// permits, so it stays distinguishable from authored underlines.
    pub(crate) fn paint(&self) -> (DecorationStyle, Color) {
        match self.lines.error {
            Some(ErrorIndication::Spelling) => (ERROR_STYLE, SPELLING_ERROR_COLOR),
            Some(ErrorIndication::Grammar) => (ERROR_STYLE, GRAMMAR_ERROR_COLOR),
            None => (self.style, self.color),
        }
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

/// The decoration lines one element specifies, with its colour resolved, and how the lines it
/// draws end and cross glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct TextDecoration {
    pub lines: DecorationLines,
    pub style: DecorationStyle,
    pub color: Color,
    /// `text-decoration-color` is `currentcolor`, which SVG text decorations take as the fill
    /// of the element declaring them (SVG 2 §11.11).
    pub current_color: bool,
    pub thickness: Option<f32>,
    /// `text-decoration-inset`.
    pub inset: DecorationInset,
    /// `text-decoration-skip: edges`, which insets the lines this box draws like `auto`.
    pub edges: bool,
    /// `text-decoration-skip-ink: all`; `auto` skips nothing, as §2.10.5 permits.
    pub skip_ink: bool,
}
impl Default for TextDecoration {
    fn default() -> Self {
        Self {
            lines: DecorationLines::default(),
            style: DecorationStyle::Solid,
            color: Color::BLACK,
            current_color: true,
            thickness: None,
            inset: DecorationInset::default(),
            edges: false,
            skip_ink: false,
        }
    }
}
impl TextDecoration {
    /// Its lengths at `zoom`.
    pub(super) fn zoomed(mut self, zoom: f32) -> Self {
        self.thickness = self.thickness.map(|width| width * zoom);
        if let DecorationInset::Lengths(lengths) = &mut self.inset {
            *lengths = lengths.map(|length| length * zoom);
        }
        self
    }
}

/// CSS Text Decoration 4 `text-decoration-inset`: how far the start and end of the lines a box
/// draws move inwards from its content edges.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) enum DecorationInset {
    /// A user agent inset that keeps adjacent decorated boxes' lines apart.
    Auto,
    /// `[start, end]` points; negative values extend the lines.
    Lengths([f32; 2]),
}
impl Default for DecorationInset {
    fn default() -> Self {
        Self::Lengths([0.0; 2])
    }
}
impl DecorationInset {
    /// `<length>{1,2} | auto`; one length insets both ends.
    pub(super) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::Auto) {
            return Ok(Self::Auto);
        }
        let lengths = value
            .items()
            .iter()
            .map(|item| length(item, relative))
            .collect::<Result<Vec<_>, _>>()?;
        match lengths[..] {
            [both] => Ok(Self::Lengths([both; 2])),
            [start, end] => Ok(Self::Lengths([start, end])),
            _ => Err(unsupported(value)),
        }
    }
}

/// What the pinned grammar's `text-decoration-skip` makes the lines of decorating ancestors
/// skip in an element's content; CSS Text Decoration 4 §2.10 splits these into
/// `text-decoration-skip-self` (`objects`), `-skip-spaces` (the space keywords) and
/// `-skip-box` (`box-decoration`). `edges` belongs to the lines the element draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DecorationSkip {
    /// The margin box of an atomic inline.
    pub objects: bool,
    /// Every spacer, with its adjacent letter and word spacing.
    pub spaces: bool,
    /// Spacers at the start or the end of a line.
    pub leading_spaces: bool,
    pub trailing_spaces: bool,
    /// The margins, borders and padding of a descendant inline box.
    pub box_decoration: bool,
}
impl Default for DecorationSkip {
    /// The initial `objects`.
    fn default() -> Self {
        Self {
            objects: true,
            spaces: false,
            leading_spaces: false,
            trailing_spaces: false,
            box_decoration: false,
        }
    }
}
impl DecorationSkip {
    /// The skipped content and whether `edges` is given; `none` skips nothing.
    pub(super) fn from_component(value: &Component) -> Result<(Self, bool), StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let mut skip = Self {
            objects: false,
            ..Self::default()
        };
        let mut edges = false;
        for keyword in keywords {
            match keyword {
                Keyword::None => {}
                Keyword::Objects => skip.objects = true,
                Keyword::Spaces => skip.spaces = true,
                Keyword::LeadingSpaces => skip.leading_spaces = true,
                Keyword::TrailingSpaces => skip.trailing_spaces = true,
                Keyword::BoxDecoration => skip.box_decoration = true,
                Keyword::Edges => edges = true,
                _ => return Err(unsupported(value)),
            }
        }
        Ok((skip, edges))
    }
}

/// `text-underline-position`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct UnderlinePosition {
    /// `under`: below the decorating box's descent rather than at the font's position.
    pub under: bool,
    /// `right` (`true`) or `left`, the side underlines take in vertical typographic modes.
    pub right: Option<bool>,
}
impl UnderlinePosition {
    /// `auto | [ from-font | under ] || [ left | right ]`.
    pub(super) fn from_component(value: &Component) -> Self {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        Self {
            under: keywords.contains(&Keyword::Under),
            right: if keywords.contains(&Keyword::Right) {
                Some(true)
            } else if keywords.contains(&Keyword::Left) {
                Some(false)
            } else {
                None
            },
        }
    }
}

/// Text decoration state of a box.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct TextDecorations {
    /// Lines this element specifies; it decorates its own content with them.
    pub declared: TextDecoration,
    /// Lines propagated from ancestors, outermost first. A block container's root inline
    /// box paints these; inline boxes leave them to their ancestors. Thicknesses are used points.
    pub propagated: Vec<TextDecoration>,
    pub underline_position: UnderlinePosition,
    /// `None` is `auto`.
    pub underline_offset: Option<LengthPercentage>,
    /// What lines of decorating ancestors skip in this element's content.
    pub skip: DecorationSkip,
}
impl TextDecorations {
    pub(super) fn propagated(&self, zoom: f32) -> Vec<TextDecoration> {
        let mut propagated = self.propagated.clone();
        if !self.declared.lines.is_empty() {
            propagated.push(self.declared.zoomed(zoom));
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
