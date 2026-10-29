//! How text is painted: fill and stroke (Compatibility §3.4.6), shadows (CSS Text
//! Decoration 3 §4) and emphasis marks (§3).
use crate::paint::{AbsoluteColor, Color};
use crate::style::{
    StyleError, box_values::unsupported, component::Component, grammar::Keyword, units::Relative,
};
use serde::Serialize;
use std::sync::Arc;

/// `<shadow-t>` is `<length>{2,3} && <color>?`.
const SHADOW_LENGTHS_SLOT: usize = 0;
const SHADOW_COLOR_SLOT: usize = 1;
const SHADOW_BLUR_INDEX: usize = 2;
/// `[ filled | open ] || <shape>` slots of `text-emphasis-style`.
const EMPHASIS_FILL_SLOT: usize = 0;
const EMPHASIS_SHAPE_SLOT: usize = 1;

/// A `<color>` of an inherited property. A value depending on `currentcolor` keeps its
/// specified form, which each element resolves against its own `color` (CSS Color 4 §4.4).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct InheritedColor {
    pub used: Color,
    #[serde(skip)]
    current: Option<Arc<Component>>,
}
impl Default for InheritedColor {
    /// `currentcolor`, as the element's colour.
    fn default() -> Self {
        Self {
            used: Color::BLACK,
            current: Some(Arc::new(Component::MathColor(
                crate::style::colors::CURRENT_COLOR.into(),
            ))),
        }
    }
}
impl InheritedColor {
    /// A colour independent of the element.
    pub(super) fn specified(used: Color) -> Self {
        Self {
            used,
            current: None,
        }
    }
    pub(super) fn depends_on_color(&self) -> bool {
        self.current.is_some()
    }
    pub(super) fn compute(
        value: &Component,
        relative: &Relative<'_>,
        current: &AbsoluteColor,
    ) -> Result<Self, StyleError> {
        Ok(Self {
            used: crate::style::colors::color(value, relative, current)?,
            current: crate::style::resolution::contains_current_color(value)
                .then(|| Arc::new(value.unwrap().clone())),
        })
    }
    /// Re-resolves a `currentcolor`-dependent value against the element's colour.
    pub(super) fn resolve(
        &mut self,
        relative: &Relative<'_>,
        current: &AbsoluteColor,
    ) -> Result<(), StyleError> {
        if let Some(value) = &self.current {
            self.used = crate::style::colors::color(value, relative, current)?;
        }
        Ok(())
    }
}

/// One `text-shadow` layer; there is no spread or inset (§4).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct TextShadow {
    pub offset: [f32; 2],
    /// Blur radius; the Gaussian's standard deviation is half of it, as for `box-shadow`.
    pub blur: f32,
    pub color: InheritedColor,
}

/// `text-emphasis-style` shapes, with their filled and open characters (§3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum EmphasisShape {
    Dot,
    Circle,
    DoubleCircle,
    Triangle,
    Sesame,
}
impl EmphasisShape {
    fn character(self, filled: bool) -> char {
        match (self, filled) {
            (Self::Dot, true) => '\u{2022}',
            (Self::Dot, false) => '\u{25E6}',
            (Self::Circle, true) => '\u{25CF}',
            (Self::Circle, false) => '\u{25CB}',
            (Self::DoubleCircle, true) => '\u{25C9}',
            (Self::DoubleCircle, false) => '\u{25CE}',
            (Self::Triangle, true) => '\u{25B2}',
            (Self::Triangle, false) => '\u{25B3}',
            (Self::Sesame, true) => '\u{FE45}',
            (Self::Sesame, false) => '\u{FE46}',
        }
    }
}

/// A computed `text-emphasis-style`: `None` is `none`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum EmphasisStyle {
    #[default]
    None,
    Shape {
        filled: bool,
        shape: EmphasisShape,
    },
    String(Box<str>),
}
impl EmphasisStyle {
    /// A shape keyword alone is filled; `filled` or `open` alone is a circle in horizontal
    /// typographic modes, the only ones the engine lays out (§3.1).
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::None) {
            return Ok(Self::None);
        }
        if let Component::String(text) = value.unwrap() {
            return Ok(Self::String(text.clone()));
        }
        let filled = match value.slot(EMPHASIS_FILL_SLOT).and_then(Component::keyword) {
            None | Some(Keyword::Filled) => true,
            Some(Keyword::Open) => false,
            Some(_) => return Err(unsupported(value)),
        };
        let shape = match value.slot(EMPHASIS_SHAPE_SLOT).and_then(Component::keyword) {
            Some(Keyword::Dot) => EmphasisShape::Dot,
            None | Some(Keyword::Circle) => EmphasisShape::Circle,
            Some(Keyword::DoubleCircle) => EmphasisShape::DoubleCircle,
            Some(Keyword::Triangle) => EmphasisShape::Triangle,
            Some(Keyword::Sesame) => EmphasisShape::Sesame,
            Some(_) => return Err(unsupported(value)),
        };
        Ok(Self::Shape { filled, shape })
    }
    /// A shape keyword's shape and whether it is filled.
    pub(crate) fn shape(&self) -> Option<(EmphasisShape, bool)> {
        match self {
            Self::Shape { filled, shape } => Some((*shape, *filled)),
            Self::None | Self::String(_) => None,
        }
    }
    /// The text drawn as each mark: a shape's character, or the string's first grapheme
    /// cluster, which the UA may truncate to (§3.1).
    pub(crate) fn mark(&self) -> Option<String> {
        use unicode_segmentation::UnicodeSegmentation;
        match self {
            Self::None => None,
            Self::Shape { filled, shape } => Some(shape.character(*filled).to_string()),
            Self::String(text) => text.graphemes(true).next().map(str::to_owned),
        }
    }
}

/// A computed `text-emphasis-position`; `left` and `right` place marks only in vertical
/// typographic modes (§3.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) enum EmphasisSide {
    /// `auto`: over the text, except under it in Chinese content (CSS Text Decoration 4
    /// Editor's Draft, `text-emphasis-position: auto`).
    #[default]
    Auto,
    Over,
    Under,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct EmphasisPosition {
    pub side: EmphasisSide,
    pub left: bool,
}
impl EmphasisPosition {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let mut position = Self::default();
        for keyword in keywords {
            match keyword {
                Keyword::Auto => position.side = EmphasisSide::Auto,
                Keyword::Over => position.side = EmphasisSide::Over,
                Keyword::Under => position.side = EmphasisSide::Under,
                Keyword::Left => position.left = true,
                Keyword::Right => position.left = false,
                _ => return Err(unsupported(value)),
            }
        }
        Ok(position)
    }
    /// Whether marks sit under horizontal text of content language `language`.
    pub(crate) fn under(self, language: &str) -> bool {
        const CHINESE: &str = "zh";
        match self.side {
            EmphasisSide::Over => false,
            EmphasisSide::Under => true,
            EmphasisSide::Auto => crate::style::has_primary_language(language, CHINESE),
        }
    }
}

/// Character classes omitted by `text-emphasis-skip` (CSS Text Decoration 4 §3.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct EmphasisSkip {
    pub spaces: bool,
    pub punctuation: bool,
    pub symbols: bool,
    pub narrow: bool,
}
impl Default for EmphasisSkip {
    fn default() -> Self {
        Self {
            spaces: true,
            punctuation: true,
            symbols: false,
            narrow: false,
        }
    }
}
impl EmphasisSkip {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let mut skip = Self {
            spaces: false,
            punctuation: false,
            symbols: false,
            narrow: false,
        };
        for keyword in keywords {
            match keyword {
                Keyword::Spaces => skip.spaces = true,
                Keyword::Punctuation => skip.punctuation = true,
                Keyword::Symbols => skip.symbols = true,
                Keyword::Narrow => skip.narrow = true,
                _ => return Err(unsupported(value)),
            }
        }
        Ok(skip)
    }
}

/// Inherited text paint of an element.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct TextPaint {
    /// `-webkit-text-fill-color`.
    pub fill: InheritedColor,
    /// `-webkit-text-stroke-width`, centred on glyph outlines; zero paints no stroke.
    pub stroke_width: f32,
    pub stroke_color: InheritedColor,
    /// `text-shadow` layers, the topmost first.
    pub shadows: Arc<[TextShadow]>,
    pub emphasis: EmphasisStyle,
    pub emphasis_color: InheritedColor,
    pub emphasis_position: EmphasisPosition,
    pub emphasis_skip: EmphasisSkip,
}
impl TextPaint {
    /// Resolves the `currentcolor`-dependent colours once the element's colour is known.
    pub(super) fn resolve(
        &mut self,
        relative: &Relative<'_>,
        current: &AbsoluteColor,
    ) -> Result<(), StyleError> {
        self.fill.resolve(relative, current)?;
        self.stroke_color.resolve(relative, current)?;
        self.emphasis_color.resolve(relative, current)?;
        resolve_shadows(&mut self.shadows, relative, current)
    }
}

pub(super) fn resolve_shadows(
    shadows: &mut Arc<[TextShadow]>,
    relative: &Relative<'_>,
    current: &AbsoluteColor,
) -> Result<(), StyleError> {
    if shadows.iter().any(|shadow| shadow.color.current.is_some()) {
        let mut resolved = shadows.to_vec();
        for shadow in &mut resolved {
            shadow.color.resolve(relative, current)?;
        }
        *shadows = resolved.into();
    }
    Ok(())
}

/// `none | <shadow-t>#`; a missing colour is `currentcolor` (CSS Backgrounds 3 §7.1).
pub(super) fn text_shadows(
    value: &Component,
    relative: &Relative<'_>,
    current: &AbsoluteColor,
) -> Result<Arc<[TextShadow]>, StyleError> {
    if value.keyword() == Some(Keyword::None) {
        return Ok(Arc::from([]));
    }
    value
        .items()
        .iter()
        .map(|item| {
            let lengths = item
                .slot(SHADOW_LENGTHS_SLOT)
                .ok_or_else(|| unsupported(item))?
                .items()
                .iter()
                .map(|length| crate::style::values::length(length, relative))
                .collect::<Result<Vec<_>, _>>()?;
            let blur = lengths.get(SHADOW_BLUR_INDEX).copied().unwrap_or(0.0);
            if blur < 0.0 {
                return Err(format!("negative text-shadow blur radius: {item}").into());
            }
            let color = match item
                .slot(SHADOW_COLOR_SLOT)
                .and_then(|color| color.items().first())
            {
                Some(color) => InheritedColor::compute(color, relative, current)?,
                None => InheritedColor {
                    used: current.used(),
                    ..InheritedColor::default()
                },
            };
            Ok(TextShadow {
                offset: [lengths[0], lengths[1]],
                blur,
                color,
            })
        })
        .collect()
}

/// `-webkit-text-stroke-width`: a non-negative length.
pub(super) fn stroke_width(value: &Component, relative: &Relative<'_>) -> Result<f32, StyleError> {
    let width = crate::style::values::length(value, relative)?;
    if width < 0.0 {
        return Err(format!("negative -webkit-text-stroke-width: {value}").into());
    }
    Ok(width)
}
