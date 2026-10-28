//! Lowers parsed Value Definition Syntax into a node graph, resolving every production.
use crate::{
    BuildError,
    source::{Initial, Origin, Spec},
    syntax::{self, Bound, Term},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Types matched by Rust code instead of the data's productions (CSS Values 4 §5–§8, CSS
/// Color 4 `<color>`, CSS Syntax 3 token productions, CSS Fonts 4 `<unicode-range-token>`).
pub(crate) const BASE_TYPES: &[&str] = &[
    "angle",
    "angle-percentage",
    "attr-unit",
    "color",
    "custom-ident",
    "dashed-ident",
    "declaration-value",
    "dimension",
    "flex",
    "hash-token",
    "hex-color",
    "ident",
    "integer",
    "length",
    "length-percentage",
    "number",
    "percentage",
    "resolution",
    "string",
    "time",
    "unicode-range-token",
    "url",
    "zero",
];
/// Base types that accept range restrictions.
const NUMERIC_TYPES: &[&str] = &[
    "angle",
    "angle-percentage",
    "flex",
    "integer",
    "length",
    "length-percentage",
    "number",
    "percentage",
    "resolution",
    "time",
];
/// Unitless numeric types, whose range bounds may be any number.
const UNITLESS_TYPES: &[&str] = &["integer", "number", "percentage"];
/// `<color>` is parsed by cssparser-color first; this production validates the rest.
pub(crate) const COLOR_PRODUCTION: &str = "color";
pub(crate) const UNBOUNDED: u32 = u16::MAX as u32;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Label {
    Property(String),
    Production(String),
}

#[derive(Clone, Debug)]
pub(crate) enum Node {
    Keyword(String),
    Delim(char),
    Base {
        name: String,
        min: f64,
        max: f64,
    },
    Function {
        name: String,
        body: Option<usize>,
    },
    Block {
        open: char,
        body: usize,
    },
    Sequence(Vec<usize>),
    AllOf(Vec<usize>),
    AnyOf(Vec<usize>),
    OneOf(Vec<usize>),
    Repeat {
        node: usize,
        min: u32,
        max: u32,
        comma: bool,
    },
    NonEmpty(usize),
    Reference {
        label: Label,
        target: usize,
    },
}

pub(crate) struct CompiledProperty {
    pub(crate) name: String,
    pub(crate) syntax: String,
    pub(crate) root: usize,
    pub(crate) initial: Initial,
    pub(crate) inherited: bool,
    pub(crate) groups: Vec<String>,
    pub(crate) origin: Origin,
}

pub(crate) struct CompiledDescriptor {
    pub(crate) at_rule: String,
    pub(crate) name: String,
    pub(crate) syntax: String,
    pub(crate) root: usize,
    pub(crate) initial: String,
}

pub(crate) struct Grammar {
    pub(crate) nodes: Vec<Node>,
    pub(crate) properties: Vec<CompiledProperty>,
    pub(crate) descriptors: Vec<CompiledDescriptor>,
    /// Production name to its reference node.
    pub(crate) productions: BTreeMap<String, usize>,
    pub(crate) color: usize,
    pub(crate) keywords: BTreeSet<String>,
    pub(crate) units: Vec<String>,
}

struct Compiler<'a> {
    spec: &'a Spec,
    nodes: Vec<Node>,
    productions: HashMap<String, usize>,
    property_roots: HashMap<String, usize>,
}

pub(crate) fn compile(spec: &Spec) -> Result<Grammar, BuildError> {
    let mut compiler = Compiler {
        spec,
        nodes: Vec::new(),
        productions: HashMap::new(),
        property_roots: HashMap::new(),
    };
    let mut properties = Vec::new();
    for (name, property) in &spec.properties {
        let root = compiler.property_root(name)?;
        if let Initial::Longhands(longhands) = &property.initial {
            for longhand in longhands {
                if !spec.properties.contains_key(longhand) {
                    return Err(BuildError::Unresolved {
                        context: format!("{name} longhands"),
                        name: longhand.clone(),
                    });
                }
            }
        }
        properties.push(CompiledProperty {
            name: name.clone(),
            syntax: property.syntax.clone(),
            root,
            initial: match &property.initial {
                Initial::Value(value) => Initial::Value(value.clone()),
                Initial::Prose(text) => Initial::Prose(text.clone()),
                Initial::Longhands(longhands) => Initial::Longhands(longhands.clone()),
            },
            inherited: property.inherited,
            groups: property.groups.clone(),
            origin: property.origin,
        });
    }
    let mut descriptors = Vec::new();
    for descriptor in &spec.descriptors {
        let context = format!("{} {}", descriptor.at_rule, descriptor.name);
        let root = compiler.syntax(&context, &descriptor.syntax)?;
        descriptors.push(CompiledDescriptor {
            at_rule: descriptor.at_rule.clone(),
            name: descriptor.name.clone(),
            syntax: descriptor.syntax.clone(),
            root,
            initial: descriptor.initial.clone(),
        });
    }
    let color = compiler.production(COLOR_PRODUCTION, COLOR_PRODUCTION)?;
    let nodes = compiler.nodes;
    for property in &mut properties {
        if let Initial::Value(value) = &property.initial
            && is_prose(value, &reachable_keywords(&nodes, property.root))
        {
            property.initial = Initial::Prose(value.clone());
        }
    }
    let keywords = nodes
        .iter()
        .filter_map(|node| match node {
            Node::Keyword(name) | Node::Function { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    let productions = compiler.productions.into_iter().collect();
    Ok(Grammar {
        nodes,
        properties,
        descriptors,
        productions,
        color,
        keywords,
        units: spec.units.clone(),
    })
}

impl Compiler<'_> {
    fn push(&mut self, node: Node) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    fn syntax(&mut self, context: &str, text: &str) -> Result<usize, BuildError> {
        let term = syntax::parse(text).map_err(|error| BuildError::Syntax {
            context: context.to_owned(),
            text: text.to_owned(),
            error,
        })?;
        self.term(context, &term)
    }

    /// A property's grammar root: a reference labelled with the property, shared by every
    /// `<'property'>` reference so that recursive grammars terminate.
    fn property_root(&mut self, name: &str) -> Result<usize, BuildError> {
        if let Some(root) = self.property_roots.get(name) {
            return Ok(*root);
        }
        let syntax = self
            .spec
            .properties
            .get(name)
            .ok_or_else(|| BuildError::Unresolved {
                context: "property reference".into(),
                name: format!("'{name}'"),
            })?
            .syntax
            .clone();
        let root = self.push(Node::Delim(char::REPLACEMENT_CHARACTER));
        self.property_roots.insert(name.to_owned(), root);
        let target = self.syntax(name, &syntax)?;
        self.nodes[root] = Node::Reference {
            label: Label::Property(name.to_owned()),
            target,
        };
        Ok(root)
    }

    /// The reference node for a named production, compiled once so recursion terminates.
    fn production(&mut self, context: &str, name: &str) -> Result<usize, BuildError> {
        if let Some(node) = self.productions.get(name) {
            return Ok(*node);
        }
        let text = self
            .spec
            .syntaxes
            .get(name)
            .ok_or_else(|| BuildError::Unresolved {
                context: context.to_owned(),
                name: name.to_owned(),
            })?
            .clone();
        let reference = self.push(Node::Delim(char::REPLACEMENT_CHARACTER));
        self.productions.insert(name.to_owned(), reference);
        let target = self.syntax(name, &text)?;
        self.nodes[reference] = Node::Reference {
            label: Label::Production(name.to_owned()),
            target,
        };
        Ok(reference)
    }

    fn terms(&mut self, context: &str, terms: &[Term]) -> Result<Vec<usize>, BuildError> {
        terms.iter().map(|term| self.term(context, term)).collect()
    }

    fn term(&mut self, context: &str, term: &Term) -> Result<usize, BuildError> {
        let node = match term {
            Term::Keyword(name) => Node::Keyword(name.clone()),
            Term::Literal(c) => Node::Delim(*c),
            Term::Type { name, range } => return self.reference(context, name, range.as_ref()),
            Term::Property(name) => return self.property_root(name),
            Term::Function { name, body } => Node::Function {
                name: name.clone(),
                body: body
                    .as_deref()
                    .map(|body| self.term(context, body))
                    .transpose()?,
            },
            Term::Block { open, body } => Node::Block {
                open: *open,
                body: self.term(context, body)?,
            },
            Term::Sequence(terms) => Node::Sequence(self.terms(context, terms)?),
            Term::AllOf(terms) => Node::AllOf(self.terms(context, terms)?),
            Term::AnyOf(terms) => Node::AnyOf(self.terms(context, terms)?),
            Term::OneOf(terms) => Node::OneOf(self.terms(context, terms)?),
            Term::Repeat {
                term,
                min,
                max,
                comma,
            } => {
                let max = max.unwrap_or(UNBOUNDED);
                if max > UNBOUNDED {
                    return Err(BuildError::Invalid {
                        context: context.to_owned(),
                        message: format!("multiplier maximum {max} exceeds {UNBOUNDED}"),
                    });
                }
                Node::Repeat {
                    node: self.term(context, term)?,
                    min: *min,
                    max,
                    comma: *comma,
                }
            }
            Term::NonEmpty(term) => Node::NonEmpty(self.term(context, term)?),
        };
        Ok(self.push(node))
    }

    fn reference(
        &mut self,
        context: &str,
        name: &str,
        range: Option<&(Bound, Bound)>,
    ) -> Result<usize, BuildError> {
        if BASE_TYPES.contains(&name) {
            let (min, max) = match range {
                None => (f64::NEG_INFINITY, f64::INFINITY),
                Some(_) if !NUMERIC_TYPES.contains(&name) => {
                    return Err(BuildError::Invalid {
                        context: context.to_owned(),
                        message: format!("range on non-numeric type <{name}>"),
                    });
                }
                Some((min, max)) => (bound(context, name, min)?, bound(context, name, max)?),
            };
            return Ok(self.push(Node::Base {
                name: name.to_owned(),
                min,
                max,
            }));
        }
        if range.is_some() {
            return Err(BuildError::Invalid {
                context: context.to_owned(),
                message: format!("range on production <{name}>"),
            });
        }
        self.production(context, name)
    }
}

/// Angles use degrees; other dimensional bounds currently express signs.
fn bound(context: &str, name: &str, bound: &Bound) -> Result<f64, BuildError> {
    if UNITLESS_TYPES.contains(&name) && !bound.unit.is_empty() {
        return Err(BuildError::Invalid {
            context: context.to_owned(),
            message: format!("unit {} on unitless <{name}> bound", bound.unit),
        });
    }
    if name == "angle"
        && let Some(degrees) = crate::angles::degrees_per_unit(&bound.unit)
    {
        return Ok(bound.value * degrees);
    }
    if !UNITLESS_TYPES.contains(&name) && bound.value.is_finite() && bound.value != 0.0 {
        return Err(BuildError::Invalid {
            context: context.to_owned(),
            message: format!("non-zero dimension bound {} on <{name}>", bound.value),
        });
    }
    Ok(bound.value)
}

fn children(node: &Node) -> Vec<usize> {
    match node {
        Node::Keyword(_) | Node::Delim(_) | Node::Base { .. } => Vec::new(),
        Node::Function { body, .. } => body.iter().copied().collect(),
        Node::Block { body: node, .. }
        | Node::Repeat { node, .. }
        | Node::NonEmpty(node)
        | Node::Reference { target: node, .. } => vec![*node],
        Node::Sequence(children)
        | Node::AllOf(children)
        | Node::AnyOf(children)
        | Node::OneOf(children) => children.clone(),
    }
}

/// mdn-data writes descriptions as one camel-case word; a grammar keyword is a real value.
fn is_prose(value: &str, keywords: &[String]) -> bool {
    value.chars().all(|c| c.is_ascii_alphabetic())
        && value.chars().any(|c| c.is_ascii_uppercase())
        && !keywords
            .iter()
            .any(|keyword| keyword.eq_ignore_ascii_case(value))
}

fn reachable_keywords(nodes: &[Node], root: usize) -> Vec<String> {
    let mut keywords = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Node::Keyword(name) = &nodes[id] {
            keywords.insert(name.clone());
        }
        pending.extend(children(&nodes[id]));
    }
    keywords.into_iter().collect()
}
