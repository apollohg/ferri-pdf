//! Static grammar tables compiled by `build/` from the CSS data in `spec/css`.

pub(crate) type NodeId = u16;
/// `Repeat::max` for multipliers without an upper bound.
pub(crate) const UNBOUNDED: u16 = u16::MAX;

/// Range restrictions (CSS Values 4 §2.4.1) share the math functions' clamping range.
pub(crate) use crate::calc::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockKind {
    Parenthesis,
    SquareBracket,
}

/// The production or property a reference node names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Label {
    Property(Property),
    Production(Production),
}

#[derive(Debug)]
pub(crate) enum Node {
    Keyword(Keyword),
    Delim(char),
    Base(Base, Range),
    Function(Keyword, Option<NodeId>),
    Block(BlockKind, NodeId),
    Sequence(&'static [NodeId]),
    AllOf(&'static [NodeId]),
    AnyOf(&'static [NodeId]),
    OneOf(&'static [NodeId]),
    Repeat {
        node: NodeId,
        min: u16,
        max: u16,
        comma: bool,
    },
    NonEmpty(NodeId),
    Reference(Label, NodeId),
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Specification,
    Errata,
    Extension,
}

#[derive(Debug)]
pub(crate) enum Initial {
    Value(&'static str),
    /// A description, such as "depends on user agent", in place of a value.
    Prose,
    Longhands(&'static [Property]),
}

#[derive(Debug)]
pub(crate) struct PropertyData {
    #[cfg(test)]
    pub(crate) syntax: &'static str,
    pub(crate) root: NodeId,
    pub(crate) initial: Initial,
    pub(crate) inherited: bool,
    /// The modules defining the property.
    #[cfg(test)]
    pub(crate) groups: &'static [&'static str],
    #[cfg(test)]
    pub(crate) origin: Origin,
}

#[derive(Debug)]
pub(crate) struct DescriptorData {
    pub(crate) descriptor: Descriptor,
    pub(crate) at_rule: &'static str,
    pub(crate) name: &'static str,
    #[cfg(test)]
    pub(crate) syntax: &'static str,
    pub(crate) root: NodeId,
    pub(crate) initial: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/grammar.rs"));

impl Keyword {
    pub(crate) fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    /// ASCII case-insensitive keyword lookup.
    pub(crate) fn from_ident(ident: &str) -> Option<Self> {
        let index = if ident.bytes().any(|b| b.is_ascii_uppercase()) {
            Self::NAMES.binary_search(&ident.to_ascii_lowercase().as_str())
        } else {
            Self::NAMES.binary_search(&ident)
        };
        index.ok().map(|index| Self::ALL[index])
    }
}

impl Property {
    pub(crate) fn data(self) -> &'static PropertyData {
        &PROPERTIES[self as usize]
    }
    pub(crate) fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
    /// Property names are ASCII case-insensitive; the parser lowercases them.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::NAMES
            .binary_search(&name)
            .ok()
            .map(|index| Self::ALL[index])
    }
    pub(crate) fn inherited(self) -> bool {
        self.data().inherited
    }
    /// The longhands a shorthand sets; empty for longhands.
    pub(crate) fn longhands(self) -> &'static [Self] {
        match self.data().initial {
            Initial::Longhands(longhands) => longhands,
            Initial::Value(_) | Initial::Prose => &[],
        }
    }
}

impl Descriptor {
    pub(crate) fn data(self) -> &'static DescriptorData {
        &DESCRIPTORS[self as usize]
    }
    pub(crate) fn find(at_rule: &str, name: &str) -> Option<Self> {
        DESCRIPTORS
            .iter()
            .find(|data| data.at_rule == at_rule && data.name == name)
            .map(|data| data.descriptor)
    }
}
