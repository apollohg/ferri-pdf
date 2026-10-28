//! Typed match trees: the specified value of a declaration as matched by its grammar.
use crate::style::{
    calc::Kind,
    grammar::{BlockKind, Keyword, Label, NODES, Node, NodeId, Range},
    units::Length,
};
use cssparser::ToCss;
use std::fmt;

const PERCENT: f32 = 100.0;
const SPACE: &str = " ";
const LIST_SEPARATOR: &str = ", ";

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
    Color(crate::paint::AbsoluteColor),
    /// A colour that depends on the element: `currentcolor`, the colour scheme, or math.
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

/// CSS text for diagnostics. Commas between list items come from the grammar that each
/// reference names; values converted at parse time, such as angles, print in canonical units.
impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.serialize(None, f)
    }
}

impl Component {
    /// `node` is the grammar node this component matched, when known.
    fn serialize(&self, node: Option<NodeId>, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let grammar = node
            .and_then(|node| matched(node, self))
            .map(|node| &NODES[usize::from(node)]);
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
            Self::Color(color) => color.to_css(f),
            Self::String(text) => cssparser::serialize_string(text, f),
            Self::Url(url) => {
                f.write_str("url(")?;
                cssparser::serialize_string(url, f)?;
                f.write_str(")")
            }
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
                    let inner = match grammar {
                        Some(Node::Function(_, inner)) => *inner,
                        _ => None,
                    };
                    body.serialize(inner, f)?;
                }
                f.write_str(")")
            }
            Self::Block(kind, body) => {
                let inner = match grammar {
                    Some(Node::Block(_, inner)) => Some(*inner),
                    _ => None,
                };
                let (open, close) = match kind {
                    BlockKind::Parenthesis => ("(", ")"),
                    BlockKind::SquareBracket => ("[", "]"),
                };
                f.write_str(open)?;
                body.serialize(inner, f)?;
                f.write_str(close)
            }
            Self::Group(items) => match grammar {
                Some(Node::Repeat { node, comma, .. }) => joined(
                    f,
                    items.iter().map(|item| (item, Some(*node))),
                    if *comma { LIST_SEPARATOR } else { SPACE },
                ),
                Some(Node::Sequence(children)) => {
                    // Grammar commas next to omitted terms are omitted from the match.
                    let mut children = children.iter().copied();
                    let aligned = items.iter().map(|item| {
                        let literal = matches!(item, Self::Delim(_));
                        let child = children.by_ref().find(|child| {
                            literal || !matches!(NODES[usize::from(*child)], Node::Delim(_))
                        });
                        (item, child)
                    });
                    joined(f, aligned, SPACE)
                }
                _ => joined(f, items.iter().map(|item| (item, None)), SPACE),
            },
            Self::Slots(slots) => {
                let alternatives = match grammar {
                    Some(Node::AnyOf(alternatives) | Node::AllOf(alternatives))
                        if alternatives.len() == slots.len() =>
                    {
                        Some(*alternatives)
                    }
                    _ => None,
                };
                joined(
                    f,
                    slots.iter().enumerate().filter_map(|(index, slot)| {
                        Some((
                            slot.as_ref()?,
                            alternatives.map(|alternatives| alternatives[index]),
                        ))
                    }),
                    SPACE,
                )
            }
            Self::Reference(label, inner) => inner.serialize(Some(target(*label)), f),
        }
    }
}

/// Joins the non-empty serializations of `items`; a literal comma takes no space before it.
fn joined<'a>(
    f: &mut fmt::Formatter<'_>,
    items: impl Iterator<Item = (&'a Component, Option<NodeId>)>,
    separator: &str,
) -> fmt::Result {
    let mut first = true;
    for (item, node) in items {
        let text = Serialized(item, node).to_string();
        if text.is_empty() {
            continue;
        }
        if !first && !matches!(item, Component::Delim(',')) {
            f.write_str(separator)?;
        }
        f.write_str(&text)?;
        first = false;
    }
    Ok(())
}

struct Serialized<'a>(&'a Component, Option<NodeId>);
impl fmt::Display for Serialized<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.serialize(self.1, f)
    }
}

/// The grammar a reference's label names.
pub(super) fn target(label: Label) -> NodeId {
    let root = match label {
        Label::Property(property) => property.data().root,
        Label::Production(production) => production.root(),
    };
    match NODES[usize::from(root)] {
        Node::Reference(found, target) if found == label => target,
        _ => root,
    }
}

/// The node, through wrappers and `|` alternatives, whose shape `component` has; `None` when
/// the alternatives leave it ambiguous.
fn matched(node: NodeId, component: &Component) -> Option<NodeId> {
    match (&NODES[usize::from(node)], component) {
        (Node::NonEmpty(inner), _) => matched(*inner, component),
        (Node::OneOf(alternatives), _) => {
            let mut fitting = alternatives
                .iter()
                .filter_map(|alternative| matched(*alternative, component));
            let first = fitting.next()?;
            fitting.next().is_none().then_some(first)
        }
        (Node::Repeat { .. } | Node::Sequence(_), Component::Group(_))
        | (Node::AnyOf(_) | Node::AllOf(_), Component::Slots(_))
        | (Node::Block(..), Component::Block(..)) => Some(node),
        (Node::Function(name, _), Component::Function(found, _)) if name == found => Some(node),
        _ => None,
    }
}
