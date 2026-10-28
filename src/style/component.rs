//! Typed match trees: the specified value of a declaration as matched by its grammar.
use crate::style::{
    calc::Kind,
    grammar::{BlockKind, Keyword, Label, Range},
    units::Length,
};
use std::fmt;

const PERCENT: f32 = 100.0;

/// One matched grammar node. Juxtaposed and repeated groups become `Group` in source order,
/// `||` and `&&` become `Slots` indexed by grammar alternative, `|` yields the chosen
/// alternative itself, and references keep the production or property they name.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Component {
    Keyword(Keyword),
    /// HTML legacy image alignment, unavailable in authored CSS syntax.
    HtmlBaselineMiddle,
    /// Exact HTML integer hints, without CSS token numeric conversion.
    HtmlInteger(i128),
    Delim(char),
    Number(f32),
    Integer(i32),
    /// Fraction: `50%` is `0.5`.
    Percentage(f32),
    Length(Length),
    /// Degrees.
    Angle(f32),
    /// Seconds.
    Time(f32),
    /// Dots per CSS pixel.
    Resolution(f32),
    /// `fr`.
    Flex(f32),
    /// A `<dimension>` of any unit.
    Dimension(f32, Box<str>),
    Color(cssparser_color::Color),
    MathColor(Box<str>),
    String(Box<str>),
    Url(Box<str>),
    /// `<ident>`, `<custom-ident>` or `<dashed-ident>`, as written.
    Ident(Box<str>),
    Hash(Box<str>),
    /// `<attr-unit>`, lowercase; `%` for percentages.
    Unit(Box<str>),
    UnicodeRange(u32, u32),
    /// `<declaration-value>` source text.
    Tokens(Box<str>),
    /// A math function's source text, the type it resolves to, and the range it clamps into.
    Math(Box<str>, Kind, Range),
    Function(Keyword, Option<Box<Component>>),
    Block(BlockKind, Box<Component>),
    Group(Vec<Component>),
    Slots(Vec<Option<Component>>),
    Reference(Label, Box<Component>),
}

impl Component {
    /// Strips reference wrappers.
    pub(crate) fn unwrap(&self) -> &Self {
        let mut component = self;
        while let Self::Reference(_, inner) = component {
            component = inner;
        }
        component
    }
    pub(crate) fn keyword(&self) -> Option<Keyword> {
        match self.unwrap() {
            Self::Keyword(keyword) => Some(*keyword),
            _ => None,
        }
    }
    /// A group's members; any other component is a group of itself.
    pub(crate) fn items(&self) -> &[Self] {
        match self.unwrap() {
            Self::Group(items) => items,
            other => std::slice::from_ref(other),
        }
    }
    /// `||` or `&&` members by grammar position; empty for other components.
    pub(crate) fn slots(&self) -> &[Option<Self>] {
        match self.unwrap() {
            Self::Slots(slots) => slots,
            _ => &[],
        }
    }
    /// The slot at `index`, if that alternative occurred.
    pub(crate) fn slot(&self, index: usize) -> Option<&Self> {
        self.slots().get(index).and_then(Option::as_ref)
    }
    /// The first component, depth first, produced by a reference with `label`.
    pub(crate) fn find(&self, label: Label) -> Option<&Self> {
        match self {
            Self::Reference(found, inner) if *found == label => Some(inner),
            Self::Reference(_, inner) | Self::Block(_, inner) => inner.find(label),
            Self::Function(_, body) => body.as_deref().and_then(|body| body.find(label)),
            Self::Group(items) => items.iter().find_map(|item| item.find(label)),
            Self::Slots(slots) => slots.iter().flatten().find_map(|slot| slot.find(label)),
            _ => None,
        }
    }
    /// Keywords outside function arguments, in source order within groups.
    pub(crate) fn keywords(&self, output: &mut Vec<Keyword>) {
        match self {
            Self::Keyword(keyword) => output.push(*keyword),
            Self::Reference(_, inner) => inner.keywords(output),
            Self::Group(items) => items.iter().for_each(|item| item.keywords(output)),
            Self::Slots(slots) => slots
                .iter()
                .flatten()
                .for_each(|slot| slot.keywords(output)),
            _ => {}
        }
    }
    /// References with one of `labels` and their contents, in source order, without
    /// descending into function arguments or into the references found.
    pub(crate) fn references<'a>(&'a self, labels: &[Label], output: &mut Vec<(Label, &'a Self)>) {
        match self {
            Self::Reference(label, inner) if labels.contains(label) => output.push((*label, inner)),
            Self::Reference(_, inner) => inner.references(labels, output),
            Self::Group(items) => items
                .iter()
                .for_each(|item| item.references(labels, output)),
            Self::Slots(slots) => slots
                .iter()
                .flatten()
                .for_each(|slot| slot.references(labels, output)),
            _ => {}
        }
    }
    /// Function arguments matched by the function's body.
    pub(crate) fn arguments(&self) -> &[Self] {
        match self.unwrap() {
            Self::Function(_, Some(body)) => body.items(),
            _ => &[],
        }
    }
}

/// Approximate CSS for diagnostics: separators the grammar implies are not reproduced.
impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let joined = |f: &mut fmt::Formatter<'_>, items: &mut dyn Iterator<Item = &Self>| {
            for (index, item) in items.enumerate() {
                if index > 0 {
                    f.write_str(" ")?;
                }
                write!(f, "{item}")?;
            }
            Ok(())
        };
        match self {
            Self::Keyword(keyword) => f.write_str(keyword.name()),
            Self::HtmlBaselineMiddle => f.write_str("<HTML baseline middle>"),
            Self::Delim(c) => write!(f, "{c}"),
            Self::Number(value) => write!(f, "{value}"),
            Self::HtmlInteger(value) => write!(f, "{value}"),
            Self::Integer(value) => write!(f, "{value}"),
            Self::Percentage(fraction) => write!(f, "{}%", fraction * PERCENT),
            Self::Length(length) => write!(f, "{}{}", length.value, length.unit.name),
            Self::Angle(degrees) => write!(f, "{degrees}deg"),
            Self::Time(seconds) => write!(f, "{seconds}s"),
            Self::Resolution(dppx) => write!(f, "{dppx}dppx"),
            Self::Flex(value) => write!(f, "{value}fr"),
            Self::Dimension(value, unit) => write!(f, "{value}{unit}"),
            Self::Color(color) => write!(f, "{color:?}"),
            Self::String(text) => write!(f, "{text:?}"),
            Self::Url(url) => write!(f, "url({url:?})"),
            Self::Ident(text)
            | Self::Unit(text)
            | Self::Tokens(text)
            | Self::Math(text, ..)
            | Self::MathColor(text) => f.write_str(text),
            Self::Hash(hash) => write!(f, "#{hash}"),
            Self::UnicodeRange(start, end) => write!(f, "U+{start:X}-{end:X}"),
            Self::Function(name, body) => {
                write!(f, "{}(", name.name())?;
                if let Some(body) = body {
                    write!(f, "{body}")?;
                }
                f.write_str(")")
            }
            Self::Block(BlockKind::Parenthesis, body) => write!(f, "({body})"),
            Self::Block(BlockKind::SquareBracket, body) => write!(f, "[{body}]"),
            Self::Group(items) => joined(f, &mut items.iter()),
            Self::Slots(slots) => joined(f, &mut slots.iter().flatten()),
            Self::Reference(_, inner) => write!(f, "{inner}"),
        }
    }
}
