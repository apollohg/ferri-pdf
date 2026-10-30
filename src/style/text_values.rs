use crate::style::{
    StyleError, box_values::unsupported, calc::Kind, component::Component, grammar::Keyword,
    units::Relative,
};
use serde::Serialize;

pub(crate) const DEFAULT_TAB_SIZE: TabSize = TabSize::Number(8.0);

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) enum TabSize {
    Number(f32),
    Length(f32),
}
impl TabSize {
    pub(crate) fn is_zero(self) -> bool {
        matches!(self, Self::Number(0.0) | Self::Length(0.0))
    }
}
/// `text-transform` case slot.
const CASE_SLOT: usize = 0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum UnicodeBidi {
    #[default]
    Normal,
    Embed,
    Isolate,
    BidiOverride,
    IsolateOverride,
    Plaintext,
}
impl UnicodeBidi {
    /// The value where bidi isolation is forced: `normal` and `embed` isolate, and
    /// `bidi-override` isolates and overrides (CSS Ruby 1 §3.5).
    #[must_use]
    pub(crate) fn forced_isolation(self) -> Self {
        match self {
            Self::Normal | Self::Embed => Self::Isolate,
            Self::BidiOverride => Self::IsolateOverride,
            other => other,
        }
    }
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::Embed) => Ok(Self::Embed),
            Some(Keyword::Isolate) => Ok(Self::Isolate),
            Some(Keyword::BidiOverride) => Ok(Self::BidiOverride),
            Some(Keyword::IsolateOverride) => Ok(Self::IsolateOverride),
            Some(Keyword::Plaintext) => Ok(Self::Plaintext),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Hyphens {
    None,
    #[default]
    Manual,
    Auto,
}
impl Hyphens {
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::None) => Ok(Self::None),
            Some(Keyword::Manual) => Ok(Self::Manual),
            Some(Keyword::Auto) => Ok(Self::Auto),
            _ => Err(unsupported(value)),
        }
    }
}

/// `white-space-collapse` (CSS Text 4 §4.1).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WhiteSpaceCollapse {
    #[default]
    Collapse,
    Preserve,
    PreserveBreaks,
    PreserveSpaces,
    BreakSpaces,
}
impl WhiteSpaceCollapse {
    const ALL: [Self; 5] = [
        Self::Collapse,
        Self::Preserve,
        Self::PreserveBreaks,
        Self::PreserveSpaces,
        Self::BreakSpaces,
    ];
    fn keyword(self) -> Keyword {
        match self {
            Self::Collapse => Keyword::Collapse,
            Self::Preserve => Keyword::Preserve,
            Self::PreserveBreaks => Keyword::PreserveBreaks,
            Self::PreserveSpaces => Keyword::PreserveSpaces,
            Self::BreakSpaces => Keyword::BreakSpaces,
        }
    }
    pub(crate) fn from_component(value: &Component) -> Result<Self, StyleError> {
        Self::ALL
            .into_iter()
            .find(|collapse| Some(collapse.keyword()) == value.keyword())
            .ok_or_else(|| unsupported(value))
    }
    pub(crate) fn name(self) -> &'static str {
        self.keyword().name()
    }
}

/// The `white-space-collapse` and `text-wrap-mode` longhands of `white-space`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct WhiteSpace {
    pub collapse: WhiteSpaceCollapse,
    pub wrap: bool,
}
impl Default for WhiteSpace {
    fn default() -> Self {
        Self::NORMAL
    }
}
impl WhiteSpace {
    pub const NORMAL: Self = Self {
        collapse: WhiteSpaceCollapse::Collapse,
        wrap: true,
    };
    pub const NOWRAP: Self = Self {
        collapse: WhiteSpaceCollapse::Collapse,
        wrap: false,
    };
    pub const PRE: Self = Self {
        collapse: WhiteSpaceCollapse::Preserve,
        wrap: false,
    };
    pub const PRE_WRAP: Self = Self {
        collapse: WhiteSpaceCollapse::Preserve,
        wrap: true,
    };
    pub fn collapses_spaces(self) -> bool {
        self.collapse.collapses_spaces()
    }
    pub fn preserves_breaks(self) -> bool {
        self.collapse.preserves_breaks()
    }
    pub fn wraps(self) -> bool {
        self.wrap
    }
    /// CSS Text 4 §4.1.3: preserved spaces at the end of a wrapping line hang.
    pub fn hangs_spaces(self) -> bool {
        self.wrap
            && matches!(
                self.collapse,
                WhiteSpaceCollapse::Preserve | WhiteSpaceCollapse::PreserveSpaces
            )
    }
    /// `break-spaces` in a wrapping line: every preserved space is a soft wrap opportunity.
    pub fn breaks_spaces(self) -> bool {
        self.wrap && self.collapse == WhiteSpaceCollapse::BreakSpaces
    }
}
impl WhiteSpaceCollapse {
    pub fn collapses_spaces(self) -> bool {
        matches!(self, Self::Collapse | Self::PreserveBreaks)
    }
    pub fn preserves_breaks(self) -> bool {
        matches!(
            self,
            Self::Preserve | Self::PreserveBreaks | Self::BreakSpaces
        )
    }
    /// `preserve-spaces` converts tabs and segment breaks to spaces.
    pub fn converts_to_spaces(self) -> bool {
        self == Self::PreserveSpaces
    }
}

/// `text-wrap-style` (CSS Text 4 §5.4).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TextWrapStyle {
    #[default]
    Auto,
    Balance,
    /// Equivalent to `auto` unless content is edited, which a static rendering never is.
    Stable,
    Pretty,
}
impl TextWrapStyle {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Balance) => Ok(Self::Balance),
            Some(Keyword::Stable) => Ok(Self::Stable),
            Some(Keyword::Pretty) => Ok(Self::Pretty),
            _ => Err(unsupported(value)),
        }
    }
}

/// `word-break` (CSS Text 4 §6.1).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum WordBreak {
    #[default]
    Normal,
    BreakAll,
    KeepAll,
    Manual,
    AutoPhrase,
    BreakWord,
}
impl WordBreak {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::BreakAll) => Ok(Self::BreakAll),
            Some(Keyword::KeepAll) => Ok(Self::KeepAll),
            Some(Keyword::Manual) => Ok(Self::Manual),
            Some(Keyword::AutoPhrase) => Ok(Self::AutoPhrase),
            Some(Keyword::BreakWord) => Ok(Self::BreakWord),
            _ => Err(unsupported(value)),
        }
    }
}

/// `line-break` (CSS Text 4 §6.2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LineBreak {
    #[default]
    Auto,
    Loose,
    Normal,
    Strict,
    Anywhere,
}
impl LineBreak {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::Loose) => Ok(Self::Loose),
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::Strict) => Ok(Self::Strict),
            Some(Keyword::Anywhere) => Ok(Self::Anywhere),
            _ => Err(unsupported(value)),
        }
    }
}

/// The line-end part of `hanging-punctuation` (CSS Text 4 §9.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum HangingEnd {
    Force,
    Allow,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) struct HangingPunctuation {
    pub first: bool,
    pub end: Option<HangingEnd>,
    pub last: bool,
}
impl HangingPunctuation {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::None) {
            return Ok(Self::default());
        }
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let end = if keywords.contains(&Keyword::ForceEnd) {
            Some(HangingEnd::Force)
        } else if keywords.contains(&Keyword::AllowEnd) {
            Some(HangingEnd::Allow)
        } else {
            None
        };
        let hanging = Self {
            first: keywords.contains(&Keyword::First),
            end,
            last: keywords.contains(&Keyword::Last),
        };
        if hanging == Self::default() {
            Err(unsupported(value))
        } else {
            Ok(hanging)
        }
    }
}

/// `text-justify` (CSS Text 3 §7.4); `distribute` computes to `inter-character`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TextJustify {
    #[default]
    Auto,
    None,
    InterWord,
    InterCharacter,
    /// CSS Text 4 §7.5: between CJK characters other than Bopomofo, not at word separators.
    Ruby,
}
impl TextJustify {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Auto) => Ok(Self::Auto),
            Some(Keyword::None) => Ok(Self::None),
            Some(Keyword::InterWord) => Ok(Self::InterWord),
            Some(Keyword::InterCharacter | Keyword::Distribute) => Ok(Self::InterCharacter),
            Some(Keyword::Ruby) => Ok(Self::Ruby),
            _ => Err(unsupported(value)),
        }
    }
}

/// Whether the primary subtag of content language `language` is `subtag`, ignoring case.
pub(crate) fn has_primary_language(language: &str, subtag: &str) -> bool {
    language
        .split(['-', '_'])
        .next()
        .is_some_and(|primary| primary.eq_ignore_ascii_case(subtag))
}

/// `text-autospace` (CSS Text 4 §8.4): the character class boundaries that receive
/// automatic spacing, and whether a U+0020 already at such a boundary is replaced.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct TextAutospace {
    pub ideograph_alpha: bool,
    pub ideograph_numeric: bool,
    pub punctuation: bool,
    pub replace: bool,
}
impl Default for TextAutospace {
    fn default() -> Self {
        Self::NORMAL
    }
}
impl TextAutospace {
    /// `normal` behaves as `ideograph-alpha ideograph-numeric`.
    const NORMAL: Self = Self {
        ideograph_alpha: true,
        ideograph_numeric: true,
        punctuation: false,
        replace: false,
    };
    /// `none` for the `no-autospace` keyword.
    const NONE: Self = Self {
        ideograph_alpha: false,
        ideograph_numeric: false,
        punctuation: false,
        replace: false,
    };
    /// The UA's typographic choice for `auto`: inter-script spacing and the language-specific
    /// punctuation spacing, leaving existing spaces in place.
    const AUTO: Self = Self {
        punctuation: true,
        ..Self::NORMAL
    };
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        match keywords[..] {
            [Keyword::Normal] => return Ok(Self::NORMAL),
            [Keyword::Auto] => return Ok(Self::AUTO),
            [Keyword::NoAutospace] => return Ok(Self::NONE),
            _ => {}
        }
        let mut spacing = Self::NONE;
        for keyword in keywords {
            match keyword {
                Keyword::IdeographAlpha => spacing.ideograph_alpha = true,
                Keyword::IdeographNumeric => spacing.ideograph_numeric = true,
                Keyword::Punctuation => spacing.punctuation = true,
                Keyword::Replace => spacing.replace = true,
                Keyword::Insert => {}
                _ => return Err(unsupported(value)),
            }
        }
        Ok(spacing)
    }
}

/// `text-spacing-trim` (CSS Text 4 §8.5).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TextSpacingTrim {
    SpaceAll,
    #[default]
    Normal,
    SpaceFirst,
    TrimStart,
    TrimBoth,
    TrimAll,
    Auto,
}
impl TextSpacingTrim {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::SpaceAll) => Ok(Self::SpaceAll),
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::SpaceFirst) => Ok(Self::SpaceFirst),
            Some(Keyword::TrimStart) => Ok(Self::TrimStart),
            Some(Keyword::TrimBoth) => Ok(Self::TrimBoth),
            Some(Keyword::TrimAll) => Ok(Self::TrimAll),
            Some(Keyword::Auto) => Ok(Self::Auto),
            _ => Err(unsupported(value)),
        }
    }
}

/// The CSS Text 4 §8.6 character class spacing longhands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) struct TextSpacing {
    pub autospace: TextAutospace,
    pub trim: TextSpacingTrim,
}

/// `hyphenate-limit-chars` minimums for the word and either side of the hyphenation point
/// (CSS Text 4 §6.3.4); `None` is `auto`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) struct HyphenateLimitChars {
    pub word: Option<usize>,
    pub before: Option<usize>,
    pub after: Option<usize>,
}
impl HyphenateLimitChars {
    pub(super) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        let limit = |item: &Component| match item.keyword() {
            Some(Keyword::Auto) => Ok(None),
            _ => usize::try_from(crate::style::values::integer(item, relative)?)
                .map(Some)
                .map_err(|_| unsupported(item)),
        };
        let limits = value
            .items()
            .iter()
            .map(limit)
            .collect::<Result<Vec<_>, _>>()?;
        match limits[..] {
            [word] => Ok(Self {
                word,
                ..Self::default()
            }),
            [word, before] => Ok(Self {
                word,
                before,
                after: before,
            }),
            [word, before, after] => Ok(Self {
                word,
                before,
                after,
            }),
            _ => Err(unsupported(value)),
        }
    }
}

/// Inherited CSS Text 4 wrapping, hyphenation and justification controls.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct LineBreaking {
    pub word_break: WordBreak,
    pub line_break: LineBreak,
    pub wrap_style: TextWrapStyle,
    pub hanging_punctuation: HangingPunctuation,
    pub justify: TextJustify,
    /// `None` is `auto`.
    pub hyphenate_character: Option<std::sync::Arc<str>>,
    pub hyphenate_limit_chars: HyphenateLimitChars,
}

/// `hyphenate-character`; `None` is `auto`.
pub(super) fn hyphenate_character(
    value: &Component,
) -> Result<Option<std::sync::Arc<str>>, StyleError> {
    match value.unwrap() {
        Component::Keyword(Keyword::Auto) => Ok(None),
        Component::String(text) => Ok(Some(std::sync::Arc::from(text.as_ref()))),
        other => Err(unsupported(other)),
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TextAlign {
    #[default]
    Start,
    End,
    Left,
    Right,
    Center,
    Justify,
}
impl TextAlign {
    pub(super) fn physical(self, rtl: bool) -> Self {
        match self {
            Self::Start if rtl => Self::Right,
            Self::Start => Self::Left,
            Self::End if rtl => Self::Left,
            Self::End => Self::Right,
            other => other,
        }
    }
    pub(crate) fn offset(self, available: f32, width: f32, rtl: bool) -> f32 {
        match self {
            Self::Right => available - width,
            Self::Start | Self::Justify if rtl => available - width,
            Self::End if !rtl => available - width,
            Self::Center => (available - width) / 2.0,
            _ => 0.0,
        }
    }
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Start) => Ok(Self::Start),
            Some(Keyword::End) => Ok(Self::End),
            Some(Keyword::Left) => Ok(Self::Left),
            Some(Keyword::Right) => Ok(Self::Right),
            Some(Keyword::Center) => Ok(Self::Center),
            Some(Keyword::Justify) => Ok(Self::Justify),
            _ => Err(unsupported(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub(crate) enum LineHeight {
    #[default]
    Normal,
    Number(f32),
    Length(f32),
}
impl LineHeight {
    pub(crate) fn resolve(self, font_size: f32) -> Option<f32> {
        match self {
            Self::Normal => None,
            Self::Number(factor) => Some(factor * font_size),
            Self::Length(length) => Some(length),
        }
    }
    pub(super) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        match value.unwrap() {
            Component::Keyword(Keyword::Normal) => Ok(Self::Normal),
            number @ (Component::Number(_) | Component::Math(_, Kind::Number, _)) => Ok(
                Self::Number(crate::style::values::number(number, relative)?),
            ),
            other => {
                let height = crate::style::box_values::font_length_percentage(other, relative)?;
                if !height.is_finite() {
                    return Err("non-finite line height".into());
                }
                Ok(Self::Length(height))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TextCase {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
    MathAuto,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) struct TextTransform {
    pub case: TextCase,
    pub full_width: bool,
    pub full_size_kana: bool,
}
impl TextTransform {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        if value.keyword() == Some(Keyword::None) {
            return Ok(Self::default());
        }
        if value.keyword() == Some(Keyword::MathAuto) {
            return Ok(Self {
                case: TextCase::MathAuto,
                ..Default::default()
            });
        }
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        let case = match value.slot(CASE_SLOT).and_then(Component::keyword) {
            Some(Keyword::Uppercase) => TextCase::Uppercase,
            Some(Keyword::Lowercase) => TextCase::Lowercase,
            Some(Keyword::Capitalize) => TextCase::Capitalize,
            None => TextCase::None,
            _ => return Err(unsupported(value)),
        };
        let transform = Self {
            case,
            full_width: keywords.contains(&Keyword::FullWidth),
            full_size_kana: keywords.contains(&Keyword::FullSizeKana),
        };
        if transform == Self::default() {
            Err(unsupported(value))
        } else {
            Ok(transform)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OverflowWrap {
    #[default]
    Normal,
    Anywhere,
    BreakWord,
}
impl OverflowWrap {
    pub(super) fn from_component(value: &Component) -> Result<Self, StyleError> {
        match value.keyword() {
            Some(Keyword::Normal) => Ok(Self::Normal),
            Some(Keyword::Anywhere) => Ok(Self::Anywhere),
            Some(Keyword::BreakWord) => Ok(Self::BreakWord),
            _ => Err(unsupported(value)),
        }
    }
}

/// Logical start-edge margin of selected formatted lines.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct TextIndent {
    pub amount: crate::style::LengthPercentage,
    pub hanging: bool,
    pub each_line: bool,
}
impl Default for TextIndent {
    fn default() -> Self {
        Self {
            amount: crate::style::LengthPercentage::ZERO,
            hanging: false,
            each_line: false,
        }
    }
}
impl TextIndent {
    pub(crate) fn from_component(
        value: &Component,
        relative: &Relative<'_>,
    ) -> Result<Self, StyleError> {
        const AMOUNT: usize = 0;
        let amount = value.slot(AMOUNT).ok_or_else(|| unsupported(value))?;
        let mut keywords = Vec::new();
        value.keywords(&mut keywords);
        Ok(Self {
            amount: crate::style::box_values::length_percentage(amount, relative)?,
            hanging: keywords.contains(&Keyword::Hanging),
            each_line: keywords.contains(&Keyword::EachLine),
        })
    }
    pub(crate) fn resolve(&self, basis: f32, first: bool, after_forced_break: bool) -> f32 {
        if (first || (self.each_line && after_forced_break)) == self.hanging {
            0.0
        } else {
            self.amount.resolve(basis)
        }
    }
}
