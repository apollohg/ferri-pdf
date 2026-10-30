//! CSS Counter Styles 3: `@counter-style` rules, the registry they cascade into, and the
//! counter representations that list markers and `counter()` functions print.
use crate::fonts::FontNamespace;
use crate::style::{
    StyleError,
    box_values::unsupported,
    component::Component,
    declaration,
    grammar::{Descriptor, Keyword, Label, Production},
    parser::Declaration,
    units::Relative,
};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, LazyLock},
};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const COUNTER_STYLE_AT_RULE: &str = "@counter-style";
/// §2: representations longer than this many code points may use the fallback style; the
/// limit applies where the size is linear in an author-supplied number.
const MAX_LINEAR_CODEPOINTS: usize = 60;
const NONE: &str = "none";
const DECIMAL: &str = "decimal";
const DISCLOSURE_OPEN: &str = "disclosure-open";
const DISCLOSURE_CLOSED: &str = "disclosure-closed";
/// §3: valid counter style names that cannot name a rule.
const RESERVED_NAMES: [&str; 6] = [
    DECIMAL,
    "disc",
    "square",
    "circle",
    DISCLOSURE_OPEN,
    DISCLOSURE_CLOSED,
];
/// The predefined `decimal` rule comes first in the normative stylesheet.
const DECIMAL_ID: usize = 0;
const INITIAL_NEGATIVE: &str = "-";
const INITIAL_SUFFIX: &str = ". ";
/// §4: the suffix of `symbols()` styles; also the predefined symbolic styles' suffix.
const SPACE_SUFFIX: &str = " ";
const INITIAL_FIRST_SYMBOL_VALUE: i128 = 1;
const MIN_SYMBOLS: usize = 1;
const MIN_POSITIONAL_SYMBOLS: usize = 2;
/// §6.3: suitable disclosure symbols; the closed one points in the inline direction.
const DISCLOSURE_OPEN_SYMBOL: &str = "\u{25BE}";
const DISCLOSURE_CLOSED_LTR_SYMBOL: &str = "\u{25B8}";
const DISCLOSURE_CLOSED_RTL_SYMBOL: &str = "\u{25C2}";
/// §7.1: the longhand East Asian styles' range, suffix and fallback.
const LONGHAND_LIMIT: i128 = 9999;
const LONGHAND_SUFFIX: &str = "\u{3001}";
const LONGHAND_FALLBACK: &str = "cjk-decimal";
/// §7.2.
const ETHIOPIC_SUFFIX: &str = "/ ";
const ETHIOPIC_ONE: &str = "\u{1369}";
const ETHIOPIC_UNITS: [&str; 9] = [
    "\u{1369}", "\u{136A}", "\u{136B}", "\u{136C}", "\u{136D}", "\u{136E}", "\u{136F}", "\u{1370}",
    "\u{1371}",
];
const ETHIOPIC_TENS: [&str; 9] = [
    "\u{1372}", "\u{1373}", "\u{1374}", "\u{1375}", "\u{1376}", "\u{1377}", "\u{1378}", "\u{1379}",
    "\u{137A}",
];
const ETHIOPIC_HUNDRED: &str = "\u{137B}";
const ETHIOPIC_TEN_THOUSAND: &str = "\u{137C}";
const ETHIOPIC_GROUP: u128 = 100;
const DECIMAL_RADIX: u128 = 10;
/// §7: the complex predefined styles, whose own range is what `range: auto` gives an
/// extending style.
const COMPLEX_RULES: [&str; 5] = [
    "japanese-informal",
    "japanese-formal",
    "korean-hangul-formal",
    "korean-hanja-informal",
    "korean-hanja-formal",
];

/// §7.1.3: one longhand Chinese style's characters.
#[derive(Debug, PartialEq, Eq, Hash)]
struct Longhand {
    name: &'static str,
    digits: [&'static str; 10],
    /// Tens, hundreds and thousands digit markers.
    markers: [&'static str; 3],
    negative: &'static str,
    informal: bool,
}
const SIMP_INFORMAL_DIGITS: [&str; 10] = [
    "\u{96F6}", "\u{4E00}", "\u{4E8C}", "\u{4E09}", "\u{56DB}", "\u{4E94}", "\u{516D}", "\u{4E03}",
    "\u{516B}", "\u{4E5D}",
];
const INFORMAL_MARKERS: [&str; 3] = ["\u{5341}", "\u{767E}", "\u{5343}"];
const FORMAL_MARKERS: [&str; 3] = ["\u{62FE}", "\u{4F70}", "\u{4EDF}"];
const SIMP_NEGATIVE: &str = "\u{8D1F}";
const TRAD_NEGATIVE: &str = "\u{8CA0}";
const LONGHANDS: [Longhand; 5] = [
    Longhand {
        name: "simp-chinese-informal",
        digits: SIMP_INFORMAL_DIGITS,
        markers: INFORMAL_MARKERS,
        negative: SIMP_NEGATIVE,
        informal: true,
    },
    Longhand {
        name: "simp-chinese-formal",
        digits: [
            "\u{96F6}", "\u{58F9}", "\u{8D30}", "\u{53C1}", "\u{8086}", "\u{4F0D}", "\u{9646}",
            "\u{67D2}", "\u{634C}", "\u{7396}",
        ],
        markers: FORMAL_MARKERS,
        negative: SIMP_NEGATIVE,
        informal: false,
    },
    Longhand {
        name: "trad-chinese-informal",
        digits: SIMP_INFORMAL_DIGITS,
        markers: INFORMAL_MARKERS,
        negative: TRAD_NEGATIVE,
        informal: true,
    },
    // §7.1.3: identical to trad-chinese-informal, for legacy reasons.
    Longhand {
        name: "cjk-ideographic",
        digits: SIMP_INFORMAL_DIGITS,
        markers: INFORMAL_MARKERS,
        negative: TRAD_NEGATIVE,
        informal: true,
    },
    Longhand {
        name: "trad-chinese-formal",
        digits: [
            "\u{96F6}", "\u{58F9}", "\u{8CB3}", "\u{53C3}", "\u{8086}", "\u{4F0D}", "\u{9678}",
            "\u{67D2}", "\u{634C}", "\u{7396}",
        ],
        markers: FORMAL_MARKERS,
        negative: TRAD_NEGATIVE,
        informal: false,
    },
];

#[derive(Clone, Debug)]
enum Symbol {
    Text(Arc<str>),
    /// An `<image>` symbol (§3.8, at risk), as written and as computed.
    Image(Arc<str>, Arc<crate::paint::BackgroundImage>),
}
/// Symbols compare as written.
impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        self.written() == other.written()
    }
}
impl Eq for Symbol {}
impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.written().hash(state);
    }
}
impl Symbol {
    fn written(&self) -> (bool, &str) {
        match self {
            Self::Text(text) => (false, text),
            Self::Image(written, _) => (true, written),
        }
    }
    fn codepoints(&self) -> usize {
        match self {
            Self::Text(text) => text.chars().count(),
            Self::Image(..) => 1,
        }
    }
    /// Grapheme clusters, which an image counts as one of.
    fn graphemes(&self) -> usize {
        match self {
            Self::Text(text) => text.graphemes(true).count(),
            Self::Image(..) => 1,
        }
    }
    fn is_empty(&self) -> bool {
        matches!(self, Self::Text(text) if text.is_empty())
    }
    /// A descriptor's `<symbol>`; a rule has no element, so an image computes against the
    /// initial values.
    fn from_component(value: &Component) -> Result<Self, StyleError> {
        Ok(match value.unwrap() {
            Component::String(text) | Component::Ident(text) => Self::Text(Arc::from(&**text)),
            image => Self::Image(
                Arc::from(image.to_string()),
                Arc::new(crate::style::background_values::image(
                    value,
                    &Relative::initial(),
                    &crate::paint::AbsoluteColor::BLACK,
                )?),
            ),
        })
    }
}

/// A counter representation: its text, and the image symbols it holds at their text offsets
/// (§3.8). Where only text can stand, as in a named string, an image contributes nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Representation {
    text: String,
    images: Vec<(usize, Arc<crate::paint::BackgroundImage>)>,
}
impl Representation {
    fn of(symbol: &Symbol) -> Self {
        let mut representation = Self::default();
        representation.push(symbol);
        representation
    }
    fn repeat(symbol: &Symbol, count: usize) -> Self {
        let mut representation = Self::default();
        for _ in 0..count {
            representation.push(symbol);
        }
        representation
    }
    fn push(&mut self, symbol: &Symbol) {
        match symbol {
            Symbol::Text(text) => self.text.push_str(text),
            Symbol::Image(_, image) => self.images.push((self.text.len(), image.clone())),
        }
    }
    pub(crate) fn push_str(&mut self, text: &str) {
        self.text.push_str(text);
    }
    pub(crate) fn append(&mut self, other: Self) {
        let offset = self.text.len();
        self.images.extend(
            other
                .images
                .into_iter()
                .map(|(at, image)| (offset + at, image)),
        );
        self.text.push_str(&other.text);
    }
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
    pub(crate) fn into_text(self) -> String {
        self.text
    }
    pub(crate) fn images(&self) -> &[(usize, Arc<crate::paint::BackgroundImage>)] {
        &self.images
    }
    fn graphemes(&self) -> usize {
        self.text.graphemes(true).count() + self.images.len()
    }
    fn codepoints(&self) -> usize {
        self.text.chars().count() + self.images.len()
    }
}
impl From<String> for Representation {
    fn from(text: String) -> Self {
        Self {
            text,
            images: Vec::new(),
        }
    }
}
type Symbols = Arc<[Symbol]>;
type AdditiveSymbols = Arc<[(u128, Symbol)]>;
/// Inclusive bounds; `i128::MIN` and `i128::MAX` stand for the infinite ones.
type Ranges = Arc<[(i128, i128)]>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Disclosure {
    Open,
    Closed,
}

/// §3.1 and §7: how a counter value becomes its initial representation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Algorithm {
    Cyclic(Symbols),
    Fixed(i128, Symbols),
    Symbolic(Symbols),
    Alphabetic(Symbols),
    Numeric(Symbols),
    Additive(AdditiveSymbols),
    Disclosure(Disclosure),
    Longhand(&'static Longhand),
    Ethiopic,
}
impl Algorithm {
    /// §3.2: whether the style uses a negative sign.
    fn uses_negative(&self) -> bool {
        !matches!(
            self,
            Self::Cyclic(_) | Self::Fixed(..) | Self::Disclosure(_)
        )
    }
    /// §3.5 `range: auto` for this system.
    fn auto_range(&self) -> Ranges {
        let lower = match self {
            Self::Cyclic(_) | Self::Fixed(..) | Self::Numeric(_) | Self::Disclosure(_) => i128::MIN,
            Self::Symbolic(_) | Self::Alphabetic(_) | Self::Ethiopic => 1,
            Self::Additive(_) => 0,
            Self::Longhand(_) => return Arc::from([(-LONGHAND_LIMIT, LONGHAND_LIMIT)]),
        };
        Arc::from([(lower, i128::MAX)])
    }
    fn symbols(&self) -> Box<dyn Iterator<Item = &Symbol> + '_> {
        match self {
            Self::Cyclic(symbols)
            | Self::Fixed(_, symbols)
            | Self::Symbolic(symbols)
            | Self::Alphabetic(symbols)
            | Self::Numeric(symbols) => Box::new(symbols.iter()),
            Self::Additive(tuples) => Box::new(tuples.iter().map(|(_, symbol)| symbol)),
            Self::Disclosure(_) | Self::Longhand(_) | Self::Ethiopic => {
                Box::new(std::iter::empty())
            }
        }
    }
    /// The initial representation of `value`, or `None` when the algorithm cannot represent
    /// it. Systems that use a negative sign receive the absolute value.
    fn represent(&self, value: i128, rtl: bool) -> Option<Representation> {
        let magnitude = value.unsigned_abs();
        match self {
            Self::Cyclic(symbols) => {
                let index = (value - 1).rem_euclid(symbols.len() as i128) as usize;
                Some(Representation::of(&symbols[index]))
            }
            Self::Fixed(first, symbols) => value
                .checked_sub(*first)
                .and_then(|offset| usize::try_from(offset).ok())
                .and_then(|index| symbols.get(index))
                .map(Representation::of),
            Self::Symbolic(symbols) => symbolic(symbols, magnitude),
            Self::Alphabetic(symbols) => alphabetic(symbols, magnitude),
            Self::Numeric(symbols) => Some(numeric(symbols, magnitude)),
            Self::Additive(tuples) => additive(tuples, magnitude),
            Self::Disclosure(Disclosure::Open) => Some(DISCLOSURE_OPEN_SYMBOL.to_owned().into()),
            Self::Disclosure(Disclosure::Closed) => Some(
                if rtl {
                    DISCLOSURE_CLOSED_RTL_SYMBOL
                } else {
                    DISCLOSURE_CLOSED_LTR_SYMBOL
                }
                .to_owned()
                .into(),
            ),
            Self::Longhand(longhand) => longhand.represent(magnitude).map(Into::into),
            Self::Ethiopic => ethiopic(magnitude).map(Into::into),
        }
    }
}

/// §3.1.3.
fn symbolic(symbols: &[Symbol], value: u128) -> Option<Representation> {
    if value == 0 {
        return None;
    }
    let count = symbols.len() as u128;
    let symbol = &symbols[((value - 1) % count) as usize];
    let length = value.div_ceil(count);
    if length.saturating_mul(symbol.codepoints() as u128) > MAX_LINEAR_CODEPOINTS as u128 {
        return None;
    }
    Some(Representation::repeat(symbol, length as usize))
}
/// The digits of a positional representation, most significant first.
fn positional<'a>(digits: impl Iterator<Item = &'a Symbol>) -> Representation {
    let digits: Vec<_> = digits.collect();
    let mut representation = Representation::default();
    for digit in digits.into_iter().rev() {
        representation.push(digit);
    }
    representation
}
/// §3.1.4.
fn alphabetic(symbols: &[Symbol], mut value: u128) -> Option<Representation> {
    if value == 0 {
        return None;
    }
    let count = symbols.len() as u128;
    let mut digits = Vec::new();
    while value != 0 {
        value -= 1;
        digits.push(&symbols[(value % count) as usize]);
        value /= count;
    }
    Some(positional(digits.into_iter()))
}
/// §3.1.5.
fn numeric(symbols: &[Symbol], mut value: u128) -> Representation {
    if value == 0 {
        return Representation::of(&symbols[0]);
    }
    let count = symbols.len() as u128;
    let mut digits = Vec::new();
    while value != 0 {
        digits.push(&symbols[(value % count) as usize]);
        value /= count;
    }
    positional(digits.into_iter())
}
/// §3.1.6.
fn additive(tuples: &[(u128, Symbol)], mut value: u128) -> Option<Representation> {
    if value == 0 {
        return tuples
            .iter()
            .find(|(weight, _)| *weight == 0)
            .map(|(_, symbol)| Representation::of(symbol));
    }
    let mut representation = Representation::default();
    let mut codepoints = 0usize;
    for (weight, symbol) in tuples {
        if *weight == 0 || *weight > value {
            continue;
        }
        let repetitions = value / weight;
        codepoints = repetitions
            .checked_mul(symbol.codepoints() as u128)
            .and_then(|added| added.checked_add(codepoints as u128))
            .filter(|total| *total <= MAX_LINEAR_CODEPOINTS as u128)
            .map_or(MAX_LINEAR_CODEPOINTS + 1, |total| total as usize);
        if codepoints > MAX_LINEAR_CODEPOINTS {
            return None;
        }
        representation.append(Representation::repeat(symbol, repetitions as usize));
        value -= weight * repetitions;
        if value == 0 {
            return Some(representation);
        }
    }
    None
}
impl Longhand {
    /// §7.1.3, on the absolute value.
    fn represent(&self, value: u128) -> Option<String> {
        if value > LONGHAND_LIMIT as u128 {
            return None;
        }
        if value == 0 {
            return Some(self.digits[0].to_owned());
        }
        let digits: Vec<usize> = value
            .to_string()
            .bytes()
            .map(|digit| usize::from(digit - b'0'))
            .collect();
        let places = digits.len();
        // Each place: its digit, where `None` drops the digit, and its marker.
        let mut parts: Vec<(Option<usize>, Option<&str>)> = Vec::new();
        for (index, digit) in digits.into_iter().enumerate() {
            let place = places - 1 - index;
            if digit == 0 {
                if parts.last().is_none_or(|(last, _)| *last != Some(0)) {
                    parts.push((Some(0), None));
                }
                continue;
            }
            let marker = place.checked_sub(1).map(|marker| self.markers[marker]);
            parts.push((Some(digit), marker));
        }
        while parts.last().is_some_and(|(digit, _)| *digit == Some(0)) {
            parts.pop();
        }
        if self.informal && (10..20).contains(&value) {
            parts[0].0 = None;
        }
        Some(
            parts
                .into_iter()
                .flat_map(|(digit, marker)| {
                    digit
                        .map(|digit| self.digits[digit])
                        .into_iter()
                        .chain(marker)
                })
                .collect(),
        )
    }
}
/// §7.2.
fn ethiopic(value: u128) -> Option<String> {
    if value == 0 {
        return None;
    }
    if value == 1 {
        return Some(ETHIOPIC_ONE.to_owned());
    }
    let mut groups = Vec::new();
    let mut rest = value;
    while rest != 0 {
        groups.push(rest % ETHIOPIC_GROUP);
        rest /= ETHIOPIC_GROUP;
    }
    let most_significant = groups.len() - 1;
    let mut text = String::new();
    for (index, group) in groups.iter().copied().enumerate().rev() {
        let odd = index % 2 == 1;
        let removed = group == 0 || (group == 1 && (index == most_significant || odd));
        if !removed {
            let (tens, units) = (group / DECIMAL_RADIX, group % DECIMAL_RADIX);
            if tens != 0 {
                text.push_str(ETHIOPIC_TENS[tens as usize - 1]);
            }
            if units != 0 {
                text.push_str(ETHIOPIC_UNITS[units as usize - 1]);
            }
        }
        if odd && group != 0 {
            text.push_str(ETHIOPIC_HUNDRED);
        } else if !odd && index != 0 {
            text.push_str(ETHIOPIC_TEN_THOUSAND);
        }
    }
    Some(text)
}

/// A counter style with every descriptor resolved (§2).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Definition {
    name: Arc<str>,
    algorithm: Algorithm,
    negative: [Symbol; 2],
    prefix: Symbol,
    suffix: Symbol,
    range: Ranges,
    /// What `range: auto` gives a style extending this one (§3.5).
    auto_range: Ranges,
    pad: (usize, Symbol),
    fallback: usize,
}
impl Definition {
    fn in_range(&self, value: i128) -> bool {
        self.range
            .iter()
            .any(|(lower, upper)| (*lower..=*upper).contains(&value))
    }
    /// §2 steps 2 to 6, or `None` when the fallback style must represent `value`.
    fn generate(&self, value: i128, rtl: bool) -> Option<Representation> {
        if !self.in_range(value) {
            return None;
        }
        let negative = value < 0 && self.algorithm.uses_negative();
        let initial = self.algorithm.represent(value, rtl)?;
        let (length, pad) = &self.pad;
        let mut difference = length.saturating_sub(initial.graphemes());
        if negative {
            for sign in &self.negative {
                difference = difference.saturating_sub(sign.graphemes());
            }
        }
        if difference > 0
            && difference
                .saturating_mul(pad.codepoints())
                .saturating_add(initial.codepoints())
                > MAX_LINEAR_CODEPOINTS
        {
            return None;
        }
        let mut representation = Representation::default();
        if negative {
            representation.push(&self.negative[0]);
        }
        representation.append(Representation::repeat(pad, difference));
        representation.append(initial);
        if negative {
            representation.push(&self.negative[1]);
        }
        Some(representation)
    }
}

/// The parsed `system` descriptor (§3.1).
#[derive(Clone, Debug)]
enum System {
    Cyclic,
    Fixed(i128),
    Symbolic,
    Alphabetic,
    Numeric,
    Additive,
    Extends(Arc<str>),
    /// A predefined style whose algorithm no `@counter-style` rule can express (§6.3, §7).
    Native(Algorithm),
}

#[derive(Clone, Debug)]
enum RangeValue {
    Auto,
    Bounds(Ranges),
}

/// The valid descriptors of one rule; later valid declarations replace earlier ones (§3).
#[derive(Clone, Debug, Default)]
struct Descriptors {
    system: Option<System>,
    symbols: Option<Symbols>,
    additive_symbols: Option<AdditiveSymbols>,
    negative: Option<[Symbol; 2]>,
    prefix: Option<Symbol>,
    suffix: Option<Symbol>,
    range: Option<RangeValue>,
    pad: Option<(usize, Symbol)>,
    fallback: Option<Arc<str>>,
}

/// One `@counter-style` rule, in the tree scope whose stylesheet declares it.
#[derive(Clone, Debug)]
pub(super) struct CounterStyleRule {
    namespace: FontNamespace,
    name: Arc<str>,
    descriptors: Descriptors,
    complex: bool,
}
impl CounterStyleRule {
    /// The rule a `@counter-style` block declares, or `None` when its name cannot name one.
    /// `important` descriptors and descriptors that fail their grammar are ignored.
    pub(super) fn parse(
        name: &str,
        declarations: &[Declaration],
        namespace: FontNamespace,
    ) -> Result<Option<Self>, StyleError> {
        let name = normalized(name);
        if name.eq_ignore_ascii_case(NONE) || RESERVED_NAMES.contains(&&*name) {
            return Ok(None);
        }
        let mut descriptors = Descriptors::parse(declarations)?;
        if let Some(System::Extends(extended)) = &mut descriptors.system {
            *extended = normalized(extended);
        }
        if let Some(fallback) = &mut descriptors.fallback {
            *fallback = normalized(fallback);
        }
        Ok(Some(Self {
            namespace,
            name,
            descriptors,
            complex: false,
        }))
    }
    /// Whether the rule defines a counter style (§3.1, §3.1.7, §3.8).
    fn defines_style(&self) -> bool {
        let descriptors = &self.descriptors;
        let symbols = descriptors
            .symbols
            .as_ref()
            .map_or(0, |symbols| symbols.len());
        match descriptors.system.as_ref().unwrap_or(&System::Symbolic) {
            System::Extends(_) => {
                descriptors.symbols.is_none() && descriptors.additive_symbols.is_none()
            }
            System::Additive => descriptors
                .additive_symbols
                .as_ref()
                .is_some_and(|tuples| !tuples.is_empty()),
            System::Cyclic | System::Fixed(_) | System::Symbolic => symbols >= MIN_SYMBOLS,
            System::Alphabetic | System::Numeric => symbols >= MIN_POSITIONAL_SYMBOLS,
            System::Native(_) => true,
        }
    }
    fn native(name: &str, algorithm: Algorithm, suffix: &str, fallback: Option<&str>) -> Self {
        let negative = match &algorithm {
            Algorithm::Longhand(longhand) => Some([
                Symbol::Text(Arc::from(longhand.negative)),
                Symbol::Text(Arc::from("")),
            ]),
            _ => None,
        };
        Self {
            namespace: FontNamespace::ROOT,
            name: Arc::from(name),
            descriptors: Descriptors {
                system: Some(System::Native(algorithm)),
                suffix: Some(Symbol::Text(Arc::from(suffix))),
                fallback: fallback.map(Arc::from),
                negative,
                ..Descriptors::default()
            },
            complex: true,
        }
    }
}

impl Descriptors {
    fn parse(declarations: &[Declaration]) -> Result<Self, StyleError> {
        let mut descriptors = Self::default();
        for declaration in declarations {
            if declaration.important {
                continue;
            }
            let Some(descriptor) = Descriptor::find(COUNTER_STYLE_AT_RULE, &declaration.name)
            else {
                continue;
            };
            let Some(value) = declaration::descriptor(descriptor, &declaration.value)? else {
                continue;
            };
            descriptors.apply(descriptor, &value)?;
        }
        Ok(descriptors)
    }
    fn apply(&mut self, descriptor: Descriptor, value: &Component) -> Result<(), StyleError> {
        let relative = Relative::initial();
        match descriptor {
            Descriptor::CounterStyleSystem => self.system = Some(system(value, &relative)?),
            Descriptor::CounterStyleSymbols => {
                self.symbols = Some(
                    symbol_list(value)
                        .map(Symbol::from_component)
                        .collect::<Result<_, _>>()?,
                );
            }
            Descriptor::CounterStyleAdditiveSymbols => {
                if let Some(tuples) = additive_symbols(value, &relative)? {
                    self.additive_symbols = Some(tuples);
                }
            }
            Descriptor::CounterStyleNegative => {
                let mut signs = symbol_list(value).map(Symbol::from_component);
                let first = signs.next().ok_or_else(|| unsupported(value))??;
                self.negative = Some([first, signs.next().transpose()?.unwrap_or(empty_symbol())]);
            }
            Descriptor::CounterStylePrefix => self.prefix = Some(Symbol::from_component(value)?),
            Descriptor::CounterStyleSuffix => self.suffix = Some(Symbol::from_component(value)?),
            Descriptor::CounterStyleRange => {
                if let Some(range) = range(value, &relative)? {
                    self.range = Some(range);
                }
            }
            Descriptor::CounterStylePad => {
                let length = crate::style::values::integer(
                    value.slot(0).ok_or_else(|| unsupported(value))?,
                    &relative,
                )?;
                let symbol = value.slot(1).ok_or_else(|| unsupported(value))?;
                self.pad = Some((
                    usize::try_from(length).map_err(|_| unsupported(value))?,
                    Symbol::from_component(symbol)?,
                ));
            }
            Descriptor::CounterStyleFallback => self.fallback = Some(style_name(value)?),
            // §3.9: the spoken form of a counter is for speech synthesis and assistive
            // technology, not the printed representation.
            Descriptor::CounterStyleSpeakAs => {}
            _ => unreachable!("only @counter-style descriptors are looked up"),
        }
        Ok(())
    }
}

/// Whether a descriptor value that matches its grammar takes effect: weights and ranges must
/// be ordered.
pub(super) fn supported(descriptor: Descriptor, value: &Component) -> bool {
    let mut descriptors = Descriptors::default();
    descriptors.apply(descriptor, value).is_ok()
        && match descriptor {
            Descriptor::CounterStyleAdditiveSymbols => descriptors.additive_symbols.is_some(),
            Descriptor::CounterStyleRange => descriptors.range.is_some(),
            _ => true,
        }
}

fn empty_symbol() -> Symbol {
    Symbol::Text(Arc::from(""))
}
fn symbol_list(value: &Component) -> impl Iterator<Item = &Component> {
    let mut found = Vec::new();
    value.references(&[Label::Production(Production::Symbol)], &mut found);
    found.into_iter().map(|(_, symbol)| symbol)
}
fn style_name(value: &Component) -> Result<Arc<str>, StyleError> {
    match value
        .find(Label::Production(Production::CounterStyleName))
        .unwrap_or(value)
        .unwrap()
    {
        Component::Ident(name) => Ok(Arc::from(&**name)),
        _ => Err(unsupported(value)),
    }
}
fn system(value: &Component, relative: &Relative<'_>) -> Result<System, StyleError> {
    let mut keywords = Vec::new();
    value.keywords(&mut keywords);
    Ok(match keywords.first() {
        Some(Keyword::Cyclic) => System::Cyclic,
        Some(Keyword::Numeric) => System::Numeric,
        Some(Keyword::Alphabetic) => System::Alphabetic,
        Some(Keyword::Symbolic) => System::Symbolic,
        Some(Keyword::Additive) => System::Additive,
        Some(Keyword::Fixed) => System::Fixed(
            leaves(value)
                .into_iter()
                .find(|item| item.keyword().is_none())
                .map(|first| crate::style::values::integer(first, relative))
                .transpose()?
                .map_or(INITIAL_FIRST_SYMBOL_VALUE, i128::from),
        ),
        Some(Keyword::Extends) => System::Extends(style_name(value)?),
        _ => return Err(unsupported(value)),
    })
}
/// `None` when the weights do not strictly descend, which invalidates the declaration (§3.8).
fn additive_symbols(
    value: &Component,
    relative: &Relative<'_>,
) -> Result<Option<AdditiveSymbols>, StyleError> {
    let mut tuples: Vec<(u128, Symbol)> = Vec::new();
    for tuple in value.items() {
        let weight = crate::style::values::integer(
            tuple.slot(0).ok_or_else(|| unsupported(tuple))?,
            relative,
        )?;
        let weight = u128::try_from(weight).map_err(|_| unsupported(tuple))?;
        if tuples.last().is_some_and(|(last, _)| *last <= weight) {
            return Ok(None);
        }
        let symbol = tuple.slot(1).ok_or_else(|| unsupported(tuple))?;
        tuples.push((weight, Symbol::from_component(symbol)?));
    }
    Ok(Some(tuples.into()))
}
/// `None` when a lower bound exceeds its upper bound, which invalidates the declaration
/// (§3.5).
fn range(value: &Component, relative: &Relative<'_>) -> Result<Option<RangeValue>, StyleError> {
    if value.keyword() == Some(Keyword::Auto) {
        return Ok(Some(RangeValue::Auto));
    }
    let mut bounds = Vec::new();
    for pair in value.items() {
        let [lower, upper] = pair.items() else {
            return Err(unsupported(pair));
        };
        let bound = |component: &Component, infinite: i128| match component.keyword() {
            Some(Keyword::Infinite) => Ok(infinite),
            _ => crate::style::values::integer(component, relative).map(i128::from),
        };
        let (lower, upper) = (bound(lower, i128::MIN)?, bound(upper, i128::MAX)?);
        if lower > upper {
            return Ok(None);
        }
        bounds.push((lower, upper));
    }
    Ok(Some(RangeValue::Bounds(bounds.into())))
}

/// §3: names this specification defines are ASCII lowercased wherever they name a style.
/// The predefined rules are written in lowercase and need no normalization.
fn normalized(name: &str) -> Arc<str> {
    let lower = name.to_ascii_lowercase();
    if PREDEFINED
        .names
        .contains_key(&(FontNamespace::ROOT, Arc::from(lower.as_str())))
    {
        Arc::from(lower)
    } else {
        Arc::from(name)
    }
}

/// §6 and §7: the predefined styles, as rules of the user-agent origin.
static PREDEFINED_RULES: LazyLock<Vec<CounterStyleRule>> = LazyLock::new(|| {
    let base = crate::resources::ResourceBase::default();
    let diagnostics = std::cell::RefCell::default();
    let rules = crate::style::parser::rules(
        Arc::from(include_str!("counter-styles.css")),
        &base,
        crate::style::selector::SelectorOrigin::UserAgent,
        &diagnostics,
        &mut crate::style::loader::Budget::unlimited(),
    )
    .expect("the predefined counter styles parse");
    assert!(
        diagnostics.borrow().is_empty(),
        "the predefined counter styles are valid: {:?}",
        diagnostics.borrow()
    );
    let mut output: Vec<CounterStyleRule> = rules
        .into_iter()
        .map(|rule| match rule {
            crate::style::parser::Rule::Content(
                crate::style::parser::ContentRule::CounterStyle(name, declarations, _),
            ) => {
                let mut rule = CounterStyleRule {
                    namespace: FontNamespace::ROOT,
                    name: Arc::from(name.as_str()),
                    descriptors: Descriptors::parse(&declarations)
                        .expect("predefined descriptors compute"),
                    complex: false,
                };
                rule.complex = COMPLEX_RULES.contains(&&*rule.name);
                rule
            }
            _ => unreachable!("the predefined stylesheet holds only @counter-style rules"),
        })
        .collect();
    assert_eq!(&*output[DECIMAL_ID].name, DECIMAL);
    output.extend([
        CounterStyleRule::native(
            DISCLOSURE_OPEN,
            Algorithm::Disclosure(Disclosure::Open),
            SPACE_SUFFIX,
            None,
        ),
        CounterStyleRule::native(
            DISCLOSURE_CLOSED,
            Algorithm::Disclosure(Disclosure::Closed),
            SPACE_SUFFIX,
            None,
        ),
        CounterStyleRule::native(
            "ethiopic-numeric",
            Algorithm::Ethiopic,
            ETHIOPIC_SUFFIX,
            None,
        ),
    ]);
    output.extend(LONGHANDS.iter().map(|longhand| {
        CounterStyleRule::native(
            longhand.name,
            Algorithm::Longhand(longhand),
            LONGHAND_SUFFIX,
            Some(LONGHAND_FALLBACK),
        )
    }));
    output
});

static PREDEFINED: LazyLock<Arc<CounterStyles>> =
    LazyLock::new(|| Arc::new(CounterStyles::build(&PREDEFINED_RULES, HashMap::new())));

/// The counter styles available to one document: the predefined ones and its
/// `@counter-style` rules, keyed by tree scope (§3).
#[derive(Debug)]
pub(crate) struct CounterStyles {
    definitions: Vec<Option<Definition>>,
    names: HashMap<(FontNamespace, Arc<str>), usize>,
    parents: HashMap<FontNamespace, FontNamespace>,
}
impl CounterStyles {
    pub(crate) fn predefined() -> Arc<Self> {
        PREDEFINED.clone()
    }
    /// `rules` in ascending cascade priority; `parents` maps each shadow tree's scope to its
    /// host's.
    pub(super) fn new(
        rules: Vec<CounterStyleRule>,
        parents: HashMap<FontNamespace, FontNamespace>,
    ) -> Self {
        let all: Vec<CounterStyleRule> = PREDEFINED_RULES.iter().cloned().chain(rules).collect();
        Self::build(&all, parents)
    }
    fn build(rules: &[CounterStyleRule], parents: HashMap<FontNamespace, FontNamespace>) -> Self {
        let mut names = HashMap::new();
        for (index, rule) in rules.iter().enumerate() {
            if rule.defines_style() {
                names.insert((rule.namespace, rule.name.clone()), index);
            }
        }
        let mut styles = Self {
            definitions: vec![None; rules.len()],
            names,
            parents,
        };
        let bases: Vec<Option<usize>> = rules
            .iter()
            .map(|rule| match &rule.descriptors.system {
                Some(System::Extends(name)) => {
                    Some(styles.lookup(rule.namespace, name).unwrap_or(DECIMAL_ID))
                }
                _ => None,
            })
            .collect();
        let cyclic = extension_cycles(&bases, &styles.names);
        let mut winners: Vec<usize> = styles.names.values().copied().collect();
        winners.sort_unstable();
        for winner in winners {
            let mut pending = vec![winner];
            while let Some(&index) = pending.last() {
                if styles.definitions[index].is_some() {
                    pending.pop();
                    continue;
                }
                let base = bases[index].map(|base| if cyclic[index] { DECIMAL_ID } else { base });
                if let Some(base) = base.filter(|base| styles.definitions[*base].is_none()) {
                    pending.push(base);
                    continue;
                }
                let definition = styles.define(&rules[index], base);
                styles.definitions[index] = Some(definition);
                pending.pop();
            }
        }
        styles
    }
    /// §3.1.7: an extended style supplies the algorithm and every unspecified descriptor.
    fn define(&self, rule: &CounterStyleRule, base: Option<usize>) -> Definition {
        let descriptors = &rule.descriptors;
        let base = base.map(|base| self.definition(base));
        let algorithm = match (base, &descriptors.system) {
            (Some(base), _) => base.algorithm.clone(),
            (None, Some(System::Native(algorithm))) => algorithm.clone(),
            (None, system) => {
                let symbols = descriptors.symbols.clone().unwrap_or_default();
                match system.as_ref().unwrap_or(&System::Symbolic) {
                    System::Cyclic => Algorithm::Cyclic(symbols),
                    System::Fixed(first) => Algorithm::Fixed(*first, symbols),
                    System::Alphabetic => Algorithm::Alphabetic(symbols),
                    System::Numeric => Algorithm::Numeric(symbols),
                    System::Additive => Algorithm::Additive(
                        descriptors.additive_symbols.clone().unwrap_or_default(),
                    ),
                    System::Symbolic | System::Extends(_) | System::Native(_) => {
                        Algorithm::Symbolic(symbols)
                    }
                }
            }
        };
        let auto_range =
            base.map_or_else(|| algorithm.auto_range(), |base| base.auto_range.clone());
        let range = match &descriptors.range {
            Some(RangeValue::Auto) => auto_range.clone(),
            Some(RangeValue::Bounds(bounds)) => bounds.clone(),
            None => base.map_or_else(|| auto_range.clone(), |base| base.range.clone()),
        };
        let text = |value: &str| Symbol::Text(Arc::from(value));
        let fallback = descriptors.fallback.as_ref().map_or_else(
            || base.map_or(DECIMAL_ID, |base| base.fallback),
            |name| self.lookup(rule.namespace, name).unwrap_or(DECIMAL_ID),
        );
        Definition {
            name: rule.name.clone(),
            negative: descriptors.negative.clone().unwrap_or_else(|| {
                base.map_or_else(
                    || [text(INITIAL_NEGATIVE), empty_symbol()],
                    |base| base.negative.clone(),
                )
            }),
            prefix: descriptors
                .prefix
                .clone()
                .unwrap_or_else(|| base.map_or_else(empty_symbol, |base| base.prefix.clone())),
            suffix: descriptors.suffix.clone().unwrap_or_else(|| {
                base.map_or_else(|| text(INITIAL_SUFFIX), |base| base.suffix.clone())
            }),
            pad: descriptors.pad.clone().unwrap_or_else(|| {
                base.map_or_else(|| (0, empty_symbol()), |base| base.pad.clone())
            }),
            auto_range: if rule.complex {
                range.clone()
            } else {
                auto_range
            },
            range,
            fallback,
            algorithm,
        }
    }
    fn definition(&self, id: usize) -> &Definition {
        self.definitions[id]
            .as_ref()
            .expect("named counter styles are defined")
    }
    /// The style `name` refers to from `namespace`: the nearest tree scope's (CSS Scoping 1
    /// §3.5), the predefined ones last.
    fn lookup(&self, namespace: FontNamespace, name: &str) -> Option<usize> {
        let name: Arc<str> = Arc::from(name);
        let mut namespace = namespace;
        loop {
            if let Some(id) = self.names.get(&(namespace, name.clone())) {
                return Some(*id);
            }
            namespace = *self.parents.get(&namespace)?;
        }
    }
    /// §2: the representation of `value` in style `id`, through its fallbacks.
    fn represent(&self, id: usize, value: i128, rtl: bool) -> Representation {
        let mut visited = Vec::new();
        let mut current = id;
        loop {
            if let Some(representation) = self.definition(current).generate(value, rtl) {
                return representation;
            }
            visited.push(current);
            let fallback = self.definition(current).fallback;
            current = if visited.contains(&fallback) {
                DECIMAL_ID
            } else {
                fallback
            };
            if visited.contains(&current) {
                return self.decimal(value);
            }
        }
    }
    fn decimal(&self, value: i128) -> Representation {
        self.definition(DECIMAL_ID)
            .generate(value, false)
            .expect("decimal represents every value")
    }
    /// Whether some representation through style `id`'s fallbacks can be empty.
    fn may_be_empty(&self, id: usize) -> bool {
        let mut visited = Vec::new();
        let mut current = id;
        while !visited.contains(&current) {
            let definition = self.definition(current);
            if definition.algorithm.symbols().any(Symbol::is_empty) {
                return true;
            }
            visited.push(current);
            current = definition.fallback;
        }
        false
    }
}

/// §3.1.7: styles in an `extends` cycle extend `decimal` instead.
fn extension_cycles(
    bases: &[Option<usize>],
    names: &HashMap<(FontNamespace, Arc<str>), usize>,
) -> Vec<bool> {
    let winning: std::collections::HashSet<usize> = names.values().copied().collect();
    let mut cyclic = vec![false; bases.len()];
    let mut settled = vec![false; bases.len()];
    for start in 0..bases.len() {
        let mut path = Vec::new();
        let mut current = Some(start);
        while let Some(index) = current {
            if settled[index] || !winning.contains(&index) {
                break;
            }
            if let Some(position) = path.iter().position(|visited| *visited == index) {
                for member in &path[position..] {
                    cyclic[*member] = true;
                }
                break;
            }
            path.push(index);
            current = bases[index];
        }
        for index in path {
            settled[index] = true;
        }
    }
    cyclic
}

/// A computed `<counter-style>`: a name resolved in the document's styles, an anonymous
/// `symbols()` style (§4), or `none`, which represents nothing.
#[derive(Clone)]
pub(crate) enum CounterStyle {
    None,
    Named {
        styles: Arc<CounterStyles>,
        id: usize,
        name: Arc<str>,
    },
    Anonymous(Arc<Definition>),
}
/// The identity that equality compares, without the document's styles.
impl std::fmt::Debug for CounterStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str(NONE),
            Self::Named { id, name, .. } => write!(f, "{name}#{id}"),
            Self::Anonymous(definition) => definition.fmt(f),
        }
    }
}
impl PartialEq for CounterStyle {
    /// Predefined rules occupy the same positions in every document's styles, so equal names
    /// with equal positions are the same style.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) => true,
            (
                Self::Named { id, name, .. },
                Self::Named {
                    id: other_id,
                    name: other_name,
                    ..
                },
            ) => id == other_id && name == other_name,
            (Self::Anonymous(definition), Self::Anonymous(other)) => definition == other,
            _ => false,
        }
    }
}
impl Eq for CounterStyle {}
impl Hash for CounterStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::None => {}
            Self::Named { id, name, .. } => (id, name).hash(state),
            Self::Anonymous(definition) => definition.hash(state),
        }
    }
}
impl serde::Serialize for CounterStyle {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::None => serializer.serialize_str(NONE),
            Self::Named { name, .. } => serializer.serialize_str(name),
            Self::Anonymous(definition) => serializer.serialize_str(&definition.name),
        }
    }
}
impl CounterStyle {
    /// A predefined style by name.
    pub(crate) fn predefined(name: &str) -> Self {
        Scope::predefined().named(name)
    }
    /// The counter representation of `value` (§2), without prefix or suffix, as `counter()`
    /// prints it.
    pub(crate) fn representation(&self, value: i128) -> Representation {
        self.generate(value, false)
    }
    /// The marker of a list item whose `list-item` counter is `value` (CSS Lists 3 §3.4):
    /// the representation between the specified style's prefix and suffix.
    pub(crate) fn marker(&self, value: i128, rtl: bool) -> Representation {
        let Some(definition) = self.own_definition() else {
            return Representation::default();
        };
        let mut representation = Representation::of(&definition.prefix);
        representation.append(self.generate(value, rtl));
        representation.push(&definition.suffix);
        representation
    }
    /// Whether a representation can be the empty string, so that it creates no box.
    pub(crate) fn may_be_empty(&self) -> bool {
        match self {
            Self::None => true,
            Self::Named { styles, id, .. } => styles.may_be_empty(*id),
            Self::Anonymous(definition) => definition.algorithm.symbols().any(Symbol::is_empty),
        }
    }
    fn own_definition(&self) -> Option<&Definition> {
        match self {
            Self::None => None,
            Self::Named { styles, id, .. } => Some(styles.definition(*id)),
            Self::Anonymous(definition) => Some(definition),
        }
    }
    fn generate(&self, value: i128, rtl: bool) -> Representation {
        match self {
            Self::None => Representation::default(),
            Self::Named { styles, id, .. } => styles.represent(*id, value, rtl),
            Self::Anonymous(definition) => definition
                .generate(value, rtl)
                .unwrap_or_else(|| PREDEFINED.decimal(value)),
        }
    }
}

/// Where `<counter-style>` values resolve: a document's styles, from one tree scope.
#[derive(Clone, Copy)]
pub(crate) struct Scope<'a> {
    pub(crate) styles: &'a Arc<CounterStyles>,
    pub(crate) namespace: FontNamespace,
}
impl Scope<'static> {
    /// The predefined styles alone.
    pub(crate) fn predefined() -> Self {
        Self {
            styles: &PREDEFINED,
            namespace: FontNamespace::ROOT,
        }
    }
}
impl<'a> Scope<'a> {
    /// The document's styles as the element in `namespace` sees them.
    pub(crate) fn of(relative: &'a Relative<'_>, namespace: FontNamespace) -> Self {
        Self {
            styles: relative.counter_styles.as_ref().unwrap_or(&PREDEFINED),
            namespace,
        }
    }
    /// The computed value of a `<counter-style>` (§5); unknown names act as `decimal`.
    pub(crate) fn resolve(&self, value: &Component) -> Result<CounterStyle, StyleError> {
        if let Some(Component::Ident(name)) = value
            .find(Label::Production(Production::CounterStyleName))
            .map(Component::unwrap)
        {
            return Ok(self.named(name));
        }
        match value
            .find(Label::Production(Production::SymbolsFn))
            .unwrap_or(value)
            .unwrap()
        {
            Component::Function(Keyword::Symbols, Some(arguments)) => {
                anonymous(arguments).map(|definition| CounterStyle::Anonymous(Arc::new(definition)))
            }
            _ => Err(unsupported(value)),
        }
    }
    pub(crate) fn named(&self, name: &str) -> CounterStyle {
        if name.eq_ignore_ascii_case(NONE) {
            return CounterStyle::None;
        }
        let name = normalized(name);
        let id = self
            .styles
            .lookup(self.namespace, &name)
            .unwrap_or(DECIMAL_ID);
        CounterStyle::Named {
            styles: self.styles.clone(),
            id,
            name,
        }
    }
}

/// §4: `symbols()` with too few symbols for an alphabetic or numeric system is invalid.
pub(super) fn valid(value: &Component) -> bool {
    let mut valid = true;
    // An image that cannot be computed is no grammar error; computing the style reports it.
    visit_functions(value, &mut |arguments| {
        valid &= !matches!(symbols_arguments(arguments), Ok(None));
    });
    valid
}
fn visit_functions(value: &Component, visit: &mut dyn FnMut(&Component)) {
    match value {
        Component::Function(Keyword::Symbols, Some(arguments)) => visit(arguments),
        Component::Function(_, Some(inner))
        | Component::Reference(_, inner)
        | Component::Block(_, inner) => visit_functions(inner, visit),
        Component::Group(items) => items.iter().for_each(|item| visit_functions(item, visit)),
        Component::Slots(slots) => slots
            .iter()
            .flatten()
            .for_each(|slot| visit_functions(slot, visit)),
        _ => {}
    }
}
/// The members of nested juxtaposed groups, in source order.
fn leaves(value: &Component) -> Vec<&Component> {
    fn collect<'a>(value: &'a Component, output: &mut Vec<&'a Component>) {
        match value.unwrap() {
            Component::Group(items) => items.iter().for_each(|item| collect(item, output)),
            leaf => output.push(leaf),
        }
    }
    let mut output = Vec::new();
    collect(value, &mut output);
    output
}
/// The system `symbols()` names, if any, and its symbols.
type SymbolsArguments = (Option<Keyword>, Vec<Symbol>);
/// The arguments of `symbols()`, `None` when too few symbols make it invalid.
fn symbols_arguments(arguments: &Component) -> Result<Option<SymbolsArguments>, StyleError> {
    let mut system = None;
    let mut symbols = Vec::new();
    for item in leaves(arguments) {
        match item.keyword() {
            Some(keyword) => system = Some(keyword),
            None => symbols.push(Symbol::from_component(item)?),
        }
    }
    let minimum = match system {
        Some(Keyword::Alphabetic | Keyword::Numeric) => MIN_POSITIONAL_SYMBOLS,
        _ => MIN_SYMBOLS,
    };
    Ok((symbols.len() >= minimum).then_some((system, symbols)))
}
/// §4: the anonymous style `symbols()` defines.
fn anonymous(arguments: &Component) -> Result<Definition, StyleError> {
    let (system, symbols) = symbols_arguments(arguments)?.ok_or_else(|| unsupported(arguments))?;
    let symbols: Symbols = symbols.into();
    let algorithm = match system {
        Some(Keyword::Cyclic) => Algorithm::Cyclic(symbols),
        Some(Keyword::Numeric) => Algorithm::Numeric(symbols),
        Some(Keyword::Alphabetic) => Algorithm::Alphabetic(symbols),
        Some(Keyword::Fixed) => Algorithm::Fixed(INITIAL_FIRST_SYMBOL_VALUE, symbols),
        _ => Algorithm::Symbolic(symbols),
    };
    let range = algorithm.auto_range();
    Ok(Definition {
        name: Arc::from(format!("symbols({arguments})")),
        negative: [Symbol::Text(Arc::from(INITIAL_NEGATIVE)), empty_symbol()],
        prefix: empty_symbol(),
        suffix: Symbol::Text(Arc::from(SPACE_SUFFIX)),
        auto_range: range.clone(),
        range,
        pad: (0, empty_symbol()),
        fallback: DECIMAL_ID,
        algorithm,
    })
}
