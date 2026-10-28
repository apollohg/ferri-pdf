use crate::fonts::{FontError, FontStyle, FontSynthesis, FontWeight};
use serde::{Serialize, Serializer};
use std::hash::{Hash, Hasher};

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize)]
pub struct FontWidth(f32);
impl FontWidth {
    pub const ULTRA_CONDENSED: Self = Self(50.0);
    pub const EXTRA_CONDENSED: Self = Self(62.5);
    pub const CONDENSED: Self = Self(75.0);
    pub const SEMI_CONDENSED: Self = Self(87.5);
    pub const NORMAL: Self = Self(100.0);
    pub const SEMI_EXPANDED: Self = Self(112.5);
    pub const EXPANDED: Self = Self(125.0);
    pub const EXTRA_EXPANDED: Self = Self(150.0);
    pub const ULTRA_EXPANDED: Self = Self(200.0);
    pub fn new(percentage: f32) -> Result<Self, FontError> {
        if !percentage.is_finite() || percentage < 0.0 {
            return Err(FontError::InvalidWidth(percentage));
        }
        Ok(Self(if percentage == 0.0 { 0.0 } else { percentage }))
    }
    pub const fn value(self) -> f32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize)]
pub struct FontSlant(f32);
impl FontSlant {
    pub const NORMAL: Self = Self(0.0);
    pub const DEFAULT: Self = Self(14.0);
    pub const ITALIC_MATCH: Self = Self(11.0);
    pub const MIN: Self = Self(-90.0);
    pub const MAX: Self = Self(90.0);
    pub fn new(degrees: f32) -> Result<Self, FontError> {
        if !(Self::MIN.0..=Self::MAX.0).contains(&degrees) {
            return Err(FontError::InvalidSlant(degrees));
        }
        Ok(Self(if degrees == 0.0 { 0.0 } else { degrees }))
    }
    pub const fn value(self) -> f32 {
        self.0
    }
    pub fn skew(self) -> f32 {
        f64::from(self.0).to_radians().tan() as f32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontRange<T> {
    Auto,
    Values { min: T, max: T },
}
impl<T: Copy + PartialOrd> FontRange<T> {
    pub const fn single(value: T) -> Self {
        Self::Values {
            min: value,
            max: value,
        }
    }
    pub fn new(start: T, end: T) -> Self {
        if start <= end {
            Self::Values {
                min: start,
                max: end,
            }
        } else {
            Self::Values {
                min: end,
                max: start,
            }
        }
    }
    pub fn bounds(self, normal: T) -> (T, T) {
        match self {
            Self::Auto => (normal, normal),
            Self::Values { min, max } => (min, max),
        }
    }
    pub fn clamp(self, requested: T) -> T {
        match self {
            Self::Auto => requested,
            Self::Values { min, .. } if requested < min => min,
            Self::Values { max, .. } if requested > max => max,
            Self::Values { .. } => requested,
        }
    }
}
impl<T: Serialize + PartialEq> Serialize for FontRange<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Values { min, max } if min == max => min.serialize(serializer),
            Self::Values { min, max } => (min, max).serialize(serializer),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FontFaceStyle {
    Auto,
    Normal,
    Italic,
    Oblique(FontRange<FontSlant>),
}
impl From<FontStyle> for FontFaceStyle {
    fn from(style: FontStyle) -> Self {
        match style {
            FontStyle::Normal => Self::Normal,
            FontStyle::Italic => Self::Italic,
            FontStyle::Oblique => Self::Oblique(FontRange::single(FontSlant::DEFAULT)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontSelection {
    pub weight: FontWeight,
    pub style: FontStyle,
    pub width: FontWidth,
    pub slant: FontSlant,
    pub synthesis: FontSynthesis,
}
impl FontSelection {
    pub const fn new(weight: FontWeight, style: FontStyle) -> Self {
        Self {
            weight,
            style,
            width: FontWidth::NORMAL,
            slant: match style {
                FontStyle::Oblique => FontSlant::DEFAULT,
                FontStyle::Normal | FontStyle::Italic => FontSlant::NORMAL,
            },
            synthesis: FontSynthesis::ALL,
        }
    }
}
impl Default for FontSelection {
    fn default() -> Self {
        Self::new(FontWeight::NORMAL, FontStyle::Normal)
    }
}
impl Eq for FontSelection {}
impl Hash for FontSelection {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.weight.value().to_bits().hash(state);
        (self.style as u8).hash(state);
        self.width.value().to_bits().hash(state);
        self.slant.value().to_bits().hash(state);
        self.synthesis.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FontVariation {
    pub tag: [u8; 4],
    pub value: f32,
}

pub(crate) struct FontInstance<'a> {
    pub face: ttf_parser::Face<'a>,
    pub variations: std::sync::Arc<[FontVariation]>,
    pub(super) weight: f32,
    pub(super) style: FontStyle,
}
impl FontInstance<'_> {
    pub(crate) fn synthesis(&self, requested: FontSelection) -> (FontSynthesis, f32) {
        let angle = match requested.style {
            FontStyle::Normal => FontSlant::NORMAL,
            FontStyle::Italic => FontSlant::DEFAULT,
            FontStyle::Oblique => requested.slant,
        };
        let synthesis = FontSynthesis {
            small_caps: false,
            bold: requested.synthesis.bold
                && requested.weight >= FontWeight::SEMIBOLD
                && self.weight < FontWeight::SEMIBOLD.value(),
            oblique: requested.synthesis.oblique
                && self.style == FontStyle::Normal
                && angle != FontSlant::NORMAL,
        };
        (
            synthesis,
            if synthesis.oblique { angle.skew() } else { 0.0 },
        )
    }
}
