//! Matches component values against compiled grammars (CSS Values 4 §2). Every node first
//! computes the set of positions where it can end; the tree is then built from the start,
//! giving each `|` its first viable alternative and each component its longest viable match.
//! Work is memoised per node and position, and recursion follows the grammar's nesting rather
//! than the number of components, so long values cannot exhaust the stack.
use crate::style::{
    StyleError,
    calc::{self, Context, Kind},
    component::Component,
    grammar::{Base, BlockKind, Keyword, Label, NODES, Node, NodeId, Production, Range, UNBOUNDED},
    parser::MAX_COMPONENT_DEPTH,
    units::{self, Dimension, Length, Relative},
    wide_keywords::WideKeyword,
};
use cssparser::{ParseError, Parser, ParserInput, Token, UnicodeRange};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    hash::{BuildHasherDefault, Hasher},
};

/// Bounds matching work for adversarial values; real declarations take a few hundred steps.
pub(super) const MAX_MATCH_STEPS: usize = 1 << 20;
const STEPS: &str = "matching steps";
/// `default` is reserved and never a `<custom-ident>` (CSS Values 4 §4.2).
const RESERVED_IDENT: &str = "default";
const PERCENT: char = '%';
const PERCENT_RANGE_SCALE: f32 = 100.0;
const URL_FUNCTION: &str = "url";
const DASHED_PREFIX: &str = "--";

#[derive(Debug)]
struct Item {
    kind: ItemKind,
    start: usize,
    end: usize,
}

#[derive(Debug)]
enum ItemKind {
    Ident {
        name: Box<str>,
        keyword: Option<Keyword>,
    },
    Function {
        name: Box<str>,
        keyword: Option<Keyword>,
        arguments: Vec<Item>,
    },
    Number {
        value: f32,
        integer: Option<i32>,
    },
    Percentage(f32),
    Dimension {
        value: f32,
        unit: Box<str>,
    },
    String(Box<str>),
    Url(Box<str>),
    Hash(Box<str>),
    Delim(char),
    Comma,
    Block(BlockKind, Vec<Item>),
    /// Tokens no grammar names, valid only inside `<declaration-value>`.
    Other,
}

pub(super) struct Origins<'a> {
    value: &'a crate::style::variables::Expanded,
    invalid_url: Cell<bool>,
}
impl<'a> Origins<'a> {
    pub(super) fn new(value: &'a crate::style::variables::Expanded) -> Self {
        Self {
            value,
            invalid_url: Cell::new(false),
        }
    }
    pub(super) fn invalid_url(&self) -> bool {
        self.invalid_url.get()
    }
    fn url(&self, source: &str, item: &Item) {
        let Some(offset) =
            (source.as_ptr() as usize).checked_sub(self.value.as_str().as_ptr() as usize)
        else {
            return;
        };
        if offset
            .checked_add(source.len())
            .is_some_and(|end| end <= self.value.len())
            && self.value.taints(offset + item.start..offset + item.end)
        {
            self.invalid_url.set(true);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ValueContext<'a> {
    Element,
    Independent,
    Substituted(&'a Origins<'a>),
    Nested(&'a Relative<'a>),
}
impl<'a> ValueContext<'a> {
    fn relative(self) -> Relative<'a> {
        let mut relative = match self {
            Self::Element | Self::Substituted(_) => Relative::validation(),
            Self::Independent => Relative::initial(),
            Self::Nested(source) => Relative {
                tree: source.tree,
                paper: source.paper,
                work: source.work.clone(),
                initial_font: source.initial_font.clone(),
                counter_styles: source.counter_styles.clone(),
                accent_color: source.accent_color,
                ..Relative::validation()
            },
        };
        relative.grammar_only = true;
        relative
    }
}

/// Matches `source` against `root`; `None` means the value is invalid. A `<custom-ident>` cannot
/// claim an identifier that a keyword can claim at the same position (CSS Values 4 §4.2), and
/// which keywords can depends on the identifiers claimed before it: each pass excludes the
/// positions the previous pass's keywords claimed, until they agree. Positions only depend on
/// earlier ones, so every pass settles at least one more and the passes converge.
pub(crate) fn match_value(
    root: NodeId,
    source: &str,
    steps: &mut usize,
    context: ValueContext<'_>,
) -> Result<Option<Component>, StyleError> {
    let mut input = ParserInput::new(source);
    let Ok(items) = tokenize(&mut Parser::new(&mut input), 0) else {
        return Ok(None);
    };
    match_items(
        &Pattern::Atom(Operand::Static(root)),
        source,
        &items,
        steps,
        context,
    )
}

pub(super) fn match_syntax(
    syntax: &crate::style::syntax::Syntax,
    source: &str,
    steps: &mut usize,
    context: ValueContext<'_>,
) -> Result<bool, StyleError> {
    let mut raw = ParserInput::new(source);
    let Ok(items) = tokenize(&mut Parser::new(&mut raw), 0) else {
        return Ok(false);
    };
    let crate::style::syntax::Syntax::Alternatives(clauses) = syntax else {
        return Ok(crate::style::variables::Value::parse(source)?.is_some());
    };
    for clause in clauses {
        let atom = match &clause.atom {
            crate::style::syntax::Atom::Base(base) => Operand::Base(*base),
            crate::style::syntax::Atom::Production(production) => {
                Operand::Static(production.root())
            }
            crate::style::syntax::Atom::Literal(literal) => Operand::Literal(literal),
        };
        let pattern = match clause.list {
            crate::style::syntax::List::Once => Pattern::Atom(atom),
            list => Pattern::Repeat(Repeat {
                node: atom,
                min: 1,
                max: usize::from(UNBOUNDED),
                comma: list == crate::style::syntax::List::Comma,
            }),
        };
        if match_items(&pattern, source, &items, steps, context)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn match_comma_list(
    root: NodeId,
    source: &str,
) -> Result<Option<Component>, StyleError> {
    let Node::Repeat {
        node, comma: true, ..
    } = &NODES[usize::from(root)]
    else {
        return Err(StyleError::Invalid(
            "recovering list grammar must be a comma repetition".into(),
        ));
    };
    let mut input = ParserInput::new(source);
    let Ok(items) = tokenize(&mut Parser::new(&mut input), 0) else {
        return Ok(None);
    };
    let mut values = Vec::new();
    let mut steps = 0;
    for item in items.split(|item| matches!(item.kind, ItemKind::Comma)) {
        if let Some(value) = match_items(
            &Pattern::Atom(Operand::Static(*node)),
            source,
            item,
            &mut steps,
            ValueContext::Independent,
        )? {
            values.push(value);
        }
    }
    Ok((!values.is_empty()).then_some(Component::Group(values)))
}

fn check_steps(steps: usize) -> Result<(), StyleError> {
    if steps > MAX_MATCH_STEPS {
        Err(StyleError::GrammarLimit {
            resource: STEPS,
            limit: MAX_MATCH_STEPS,
        })
    } else {
        Ok(())
    }
}

fn match_items(
    root: &Pattern<'_>,
    source: &str,
    items: &[Item],
    steps: &mut usize,
    context: ValueContext<'_>,
) -> Result<Option<Component>, StyleError> {
    let mut excluded = HashSet::new();
    loop {
        let mut matcher = Matcher {
            source,
            excluded: &excluded,
            claimed: HashSet::new(),
            ends: Memo::with_capacity_and_hasher(MEMO_CAPACITY, Default::default()),
            steps: *steps,
            context,
            failure: RefCell::default(),
        };
        let matched = match root {
            Pattern::Atom(atom) => matcher.operand_ends(*atom, items, 0),
            Pattern::Repeat(repeat) => matcher.repeat_ends(repeat, items, 0),
        }
        .contains(&items.len());
        if let Some(error) = matcher.failure.get_mut().take() {
            return Err(error);
        }
        check_steps(matcher.steps)?;
        if matcher.claimed == excluded {
            let value = if matched {
                match root {
                    Pattern::Atom(atom) => matcher.operand_build(*atom, items, 0, items.len()),
                    Pattern::Repeat(repeat) => Some(Component::Group(matcher.repeat(
                        repeat,
                        items,
                        0,
                        items.len(),
                    ))),
                }
            } else {
                None
            };
            if let Some(error) = matcher.failure.get_mut().take() {
                return Err(error);
            }
            *steps = matcher.steps;
            check_steps(*steps)?;
            return Ok(value);
        }
        *steps = matcher.steps;
        excluded = matcher.claimed;
    }
}

/// Component values without whitespace; bad tokens and unmatched closers make the value invalid.
fn tokenize<'i>(
    parser: &mut Parser<'i, '_>,
    depth: usize,
) -> Result<Vec<Item>, ParseError<'i, ()>> {
    if depth > MAX_COMPONENT_DEPTH {
        return Err(parser.new_custom_error(()));
    }
    let mut items = Vec::new();
    loop {
        let start = parser.position().byte_index();
        let Ok(token) = parser.next_including_whitespace_and_comments() else {
            break;
        };
        let kind = match token.clone() {
            Token::WhiteSpace(_) | Token::Comment(_) => continue,
            Token::Ident(name) => ItemKind::Ident {
                keyword: Keyword::from_ident(&name),
                name: name.as_ref().into(),
            },
            Token::Function(name) => {
                let arguments = parser.parse_nested_block(|p| tokenize(p, depth + 1))?;
                match &arguments[..] {
                    [
                        Item {
                            kind: ItemKind::String(url),
                            ..
                        },
                    ] if name.eq_ignore_ascii_case(URL_FUNCTION) => ItemKind::Url(url.clone()),
                    _ => ItemKind::Function {
                        keyword: Keyword::from_ident(&name),
                        name: name.as_ref().into(),
                        arguments,
                    },
                }
            }
            Token::UnquotedUrl(url) => ItemKind::Url(url.as_ref().into()),
            Token::Number {
                value, int_value, ..
            } => ItemKind::Number {
                value,
                integer: int_value,
            },
            Token::Percentage { unit_value, .. } => ItemKind::Percentage(unit_value),
            Token::Dimension { value, unit, .. } => ItemKind::Dimension {
                value,
                unit: unit.as_ref().into(),
            },
            Token::QuotedString(text) => ItemKind::String(text.as_ref().into()),
            Token::Hash(hash) | Token::IDHash(hash) => ItemKind::Hash(hash.as_ref().into()),
            Token::Delim(c) => ItemKind::Delim(c),
            Token::Comma => ItemKind::Comma,
            Token::ParenthesisBlock => ItemKind::Block(
                BlockKind::Parenthesis,
                parser.parse_nested_block(|p| tokenize(p, depth + 1))?,
            ),
            Token::SquareBracketBlock => ItemKind::Block(
                BlockKind::SquareBracket,
                parser.parse_nested_block(|p| tokenize(p, depth + 1))?,
            ),
            Token::CurlyBracketBlock => {
                parser.parse_nested_block(|p| tokenize(p, depth + 1))?;
                ItemKind::Other
            }
            Token::BadUrl(_)
            | Token::BadString(_)
            | Token::CloseParenthesis
            | Token::CloseSquareBracket
            | Token::CloseCurlyBracket => return Err(parser.new_custom_error(())),
            _ => ItemKind::Other,
        };
        items.push(Item {
            kind,
            start,
            end: parser.position().byte_index(),
        });
    }
    Ok(items)
}

/// A node at a position in one token list; nested arguments are lists of their own.
type Key = (NodeId, usize, usize, usize);
type Memo<K, V> = HashMap<K, V, BuildHasherDefault<MemoHasher>>;
/// Odd 64-bit multiplier with well-mixed bits (the golden-ratio constant).
const MEMO_MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;
const MEMO_ROTATION: u32 = 5;
/// Typical declarations memoise a few dozen grammar nodes; reserving avoids rehashing.
const MEMO_CAPACITY: usize = 64;

/// Multiplicative hashing for memo keys: positions and node ids, which the matching step
/// limit already bounds, need speed rather than flood resistance.
#[derive(Default)]
struct MemoHasher(u64);
impl MemoHasher {
    fn add(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(MEMO_ROTATION) ^ value).wrapping_mul(MEMO_MULTIPLIER);
    }
}
impl Hasher for MemoHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.add(u64::from(*byte));
        }
    }
    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }
}

struct Matcher<'s> {
    source: &'s str,
    /// Source offsets of identifiers a keyword claimed in the previous pass.
    excluded: &'s HashSet<usize>,
    /// Source offsets of identifiers a keyword claims in this pass.
    claimed: HashSet<usize>,
    ends: Memo<Key, Vec<usize>>,
    steps: usize,
    context: ValueContext<'s>,
    failure: RefCell<Option<StyleError>>,
}

#[derive(Clone, Copy)]
enum Operand<'a> {
    Static(NodeId),
    Base(Base),
    Literal(&'a str),
}
enum Pattern<'a> {
    Atom(Operand<'a>),
    Repeat(Repeat<'a>),
}
struct Repeat<'a> {
    node: Operand<'a>,
    min: usize,
    max: usize,
    comma: bool,
}
impl Repeat<'_> {
    /// Iterations after the first need a separating comma in `#` lists.
    fn start(&self, iteration: usize, items: &[Item], pos: usize) -> Option<usize> {
        if iteration > 1 && self.comma {
            matches!(items.get(pos).map(|item| &item.kind), Some(ItemKind::Comma))
                .then_some(pos + 1)
        } else {
            Some(pos)
        }
    }
    /// Beyond the minimum, an unbounded multiplier's count no longer matters.
    fn capped(&self, count: usize) -> usize {
        if usize::from(UNBOUNDED) == self.max {
            count.min(self.min)
        } else {
            count
        }
    }
}

fn complete(all: bool, alternatives: usize, used: u64) -> bool {
    if all {
        used.count_ones() as usize == alternatives
    } else {
        used != 0
    }
}

impl Matcher<'_> {
    fn operand_ends(&mut self, atom: Operand<'_>, items: &[Item], pos: usize) -> Vec<usize> {
        if let Operand::Static(node) = atom {
            return self.ends(node, items, pos);
        }
        self.steps = self.steps.saturating_add(1);
        if self.steps > MAX_MATCH_STEPS {
            return Vec::new();
        }
        if items
            .get(pos)
            .is_some_and(|item| self.operand_single(atom, item).is_some())
        {
            vec![pos + 1]
        } else {
            Vec::new()
        }
    }
    fn operand_single(&mut self, atom: Operand<'_>, item: &Item) -> Option<Component> {
        match atom {
            Operand::Static(_) => unreachable!("static nodes use their grammar matcher"),
            Operand::Base(base) => self.base(base, Range::ALL, item),
            Operand::Literal(literal) => match &item.kind {
                ItemKind::Ident { name, .. } if name.eq_ignore_ascii_case(literal) => {
                    self.claimed.insert(item.start);
                    Some(Component::Ident(name.clone()))
                }
                _ => None,
            },
        }
    }
    fn operand_build(
        &mut self,
        atom: Operand<'_>,
        items: &[Item],
        pos: usize,
        end: usize,
    ) -> Option<Component> {
        match atom {
            Operand::Static(node) => self.build(node, items, pos, end),
            _ if end == pos + 1 => self.operand_single(atom, items.get(pos)?),
            _ => None,
        }
    }
    /// Sorted end positions of every match of `id` starting at `pos`.
    fn ends(&mut self, id: NodeId, items: &[Item], pos: usize) -> Vec<usize> {
        // Single-token nodes are cheaper to match than to look up.
        if matches!(
            NODES[usize::from(id)],
            Node::Keyword(_)
                | Node::Delim(_)
                | Node::Base(..)
                | Node::Function(..)
                | Node::Block(..)
        ) {
            return self.compute(id, items, pos);
        }
        let key = (id, items.as_ptr() as usize, items.len(), pos);
        if let Some(ends) = self.ends.get(&key) {
            return ends.clone();
        }
        self.steps += 1;
        if self.steps > MAX_MATCH_STEPS {
            return Vec::new();
        }
        // Reaching the same node again without consuming input (left recursion) fails.
        self.ends.insert(key, Vec::new());
        let ends = self.compute(id, items, pos);
        self.ends.insert(key, ends.clone());
        ends
    }

    fn compute(&mut self, id: NodeId, items: &[Item], pos: usize) -> Vec<usize> {
        let item = items.get(pos);
        let single = |matched: bool| if matched { vec![pos + 1] } else { Vec::new() };
        match &NODES[usize::from(id)] {
            Node::Keyword(keyword) => {
                let claimed = item.filter(|item| {
                    matches!(&item.kind, ItemKind::Ident { keyword: Some(found), .. } if found == keyword)
                });
                if let Some(item) = claimed {
                    self.claimed.insert(item.start);
                }
                single(claimed.is_some())
            }
            Node::Delim(',') => {
                let mut ends = Vec::new();
                if comma_omitted(items, pos) {
                    ends.push(pos);
                }
                if comma_consumed(items, pos) {
                    ends.push(pos + 1);
                }
                ends
            }
            Node::Delim(c) => single(matches!(
                item.map(|item| &item.kind),
                Some(ItemKind::Delim(found)) if found == c
            )),
            Node::Base(Base::DeclarationValue, _) => {
                (pos + 1..=declaration_end(items, pos)).collect()
            }
            Node::Base(Base::UnicodeRangeToken, _) => self
                .unicode_range(items, pos)
                .map(|(_, end)| end)
                .into_iter()
                .collect(),
            Node::Base(base, range) => {
                single(item.is_some_and(|item| self.base(*base, *range, item).is_some()))
            }
            Node::Function(name, body) => {
                single(item.is_some_and(|item| self.function(*name, *body, item)))
            }
            Node::Block(kind, body) => single(match item.map(|item| &item.kind) {
                Some(ItemKind::Block(found, contents)) if found == kind => {
                    self.ends(*body, contents, 0).contains(&contents.len())
                }
                _ => false,
            }),
            Node::Sequence(children) => {
                let mut positions = vec![pos];
                for child in *children {
                    positions = self.union(*child, items, &positions);
                    if positions.is_empty() {
                        break;
                    }
                }
                positions
            }
            Node::OneOf(alternatives) => {
                let mut ends = Vec::new();
                for alternative in *alternatives {
                    ends.extend(self.ends(*alternative, items, pos));
                }
                ends.sort_unstable();
                ends.dedup();
                ends
            }
            Node::AnyOf(alternatives) => self.unordered_ends(alternatives, false, items, pos),
            Node::AllOf(alternatives) => self.unordered_ends(alternatives, true, items, pos),
            Node::Repeat {
                node,
                min,
                max,
                comma,
            } => self.repeat_ends(&repeat(*node, *min, *max, *comma), items, pos),
            Node::NonEmpty(node) => {
                let mut ends = self.ends(*node, items, pos);
                ends.retain(|end| *end > pos);
                ends
            }
            Node::Reference(_, target) => self.ends(*target, items, pos),
        }
    }

    fn union(&mut self, id: NodeId, items: &[Item], starts: &[usize]) -> Vec<usize> {
        let mut ends = Vec::new();
        for start in starts {
            ends.extend(self.ends(id, items, *start));
        }
        ends.sort_unstable();
        ends.dedup();
        ends
    }

    fn unordered_ends(
        &mut self,
        alternatives: &[NodeId],
        all: bool,
        items: &[Item],
        pos: usize,
    ) -> Vec<usize> {
        let mut ends = Vec::new();
        let mut seen = HashSet::new();
        let mut pending = vec![(0u64, pos)];
        while let Some((used, at)) = pending.pop() {
            if !seen.insert((used, at)) {
                continue;
            }
            if complete(all, alternatives.len(), used) {
                ends.push(at);
            }
            for (index, alternative) in alternatives.iter().enumerate() {
                if used & (1 << index) == 0 {
                    for end in self.ends(*alternative, items, at) {
                        pending.push((used | 1 << index, end));
                    }
                }
            }
        }
        ends.sort_unstable();
        ends.dedup();
        ends
    }

    fn repeat_ends(&mut self, repeat: &Repeat<'_>, items: &[Item], pos: usize) -> Vec<usize> {
        let mut ends = Vec::new();
        if repeat.min == 0 {
            ends.push(pos);
        }
        let mut expanded = HashSet::new();
        let mut layer = vec![pos];
        let mut count = 0;
        while count < repeat.max && !layer.is_empty() {
            count += 1;
            let mut next = Vec::new();
            for at in &layer {
                if let Some(start) = repeat.start(count, items, *at) {
                    next.extend(self.operand_ends(repeat.node, items, start));
                }
            }
            next.sort_unstable();
            next.dedup();
            if count >= repeat.min {
                // A position already expanded with fewer iterations reaches no new ends.
                next.retain(|at| expanded.insert(*at));
                ends.extend(&next);
            } else if next == layer {
                // Only empty iterations remain, and they satisfy the minimum.
                ends.extend(&next);
                break;
            }
            layer = next;
        }
        ends.sort_unstable();
        ends.dedup();
        ends
    }

    /// Builds the match of `id` spanning `pos..end`, which `ends` must have found. `None` is
    /// an omitted comma.
    fn build(&mut self, id: NodeId, items: &[Item], pos: usize, end: usize) -> Option<Component> {
        let item = items.get(pos);
        Some(match &NODES[usize::from(id)] {
            Node::Keyword(keyword) => Component::Keyword(*keyword),
            Node::Delim(',') if end == pos => return None,
            Node::Delim(c) => Component::Delim(*c),
            Node::Base(Base::DeclarationValue, _) => {
                Component::Tokens(self.source[items[pos].start..items[end - 1].end].into())
            }
            Node::Base(Base::UnicodeRangeToken, _) => self.unicode_range(items, pos)?.0,
            Node::Base(base, range) => {
                if *base == Base::Url
                    && let ValueContext::Substituted(origin) = self.context
                {
                    origin.url(self.source, item?);
                }
                self.base(*base, *range, item?)?
            }
            Node::Function(name, body) => {
                let ItemKind::Function { arguments, .. } = &item?.kind else {
                    return None;
                };
                Component::Function(
                    *name,
                    body.and_then(|body| self.build(body, arguments, 0, arguments.len()))
                        .map(Box::new),
                )
            }
            Node::Block(kind, body) => {
                let ItemKind::Block(_, contents) = &item?.kind else {
                    return None;
                };
                Component::Block(
                    *kind,
                    Box::new(self.build(*body, contents, 0, contents.len())?),
                )
            }
            Node::Sequence(children) => Component::Group(self.sequence(children, items, pos, end)),
            Node::OneOf(alternatives) => {
                for alternative in *alternatives {
                    if self.ends(*alternative, items, pos).contains(&end) {
                        return self.build(*alternative, items, pos, end);
                    }
                }
                return None;
            }
            Node::AnyOf(alternatives) => {
                Component::Slots(self.unordered(alternatives, false, items, pos, end))
            }
            Node::AllOf(alternatives) => {
                Component::Slots(self.unordered(alternatives, true, items, pos, end))
            }
            Node::Repeat {
                node,
                min,
                max,
                comma,
            } => Component::Group(self.repeat(&repeat(*node, *min, *max, *comma), items, pos, end)),
            Node::NonEmpty(node) => return self.build(*node, items, pos, end),
            Node::Reference(label, target) => {
                let value = self.build(*target, items, pos, end)?;
                if matches!(
                    label,
                    Label::Production(Production::ImageSrc | Production::ImageSetOption)
                ) && let Some(item) = items.get(pos)
                    && matches!(item.kind, ItemKind::String(_))
                    && let ValueContext::Substituted(origin) = self.context
                {
                    origin.url(self.source, item);
                }
                Component::Reference(*label, Box::new(value))
            }
        })
    }

    /// Each child takes its longest match that still lets the rest reach `end`.
    fn sequence(
        &mut self,
        children: &[NodeId],
        items: &[Item],
        pos: usize,
        end: usize,
    ) -> Vec<Component> {
        let mut output = Vec::new();
        let mut at = pos;
        for (index, child) in children.iter().enumerate() {
            let rest = &children[index + 1..];
            let mut chosen = None;
            for candidate in self.ends(*child, items, at).into_iter().rev() {
                if candidate <= end && self.reaches(rest, items, candidate, end) {
                    chosen = Some(candidate);
                    break;
                }
            }
            let Some(next) = chosen else {
                break;
            };
            output.extend(self.build(*child, items, at, next));
            at = next;
        }
        output
    }

    fn reaches(&mut self, children: &[NodeId], items: &[Item], pos: usize, end: usize) -> bool {
        let mut positions = vec![pos];
        for child in children {
            positions = self.union(*child, items, &positions);
            positions.retain(|at| *at <= end);
            if positions.is_empty() {
                return false;
            }
        }
        positions.contains(&end)
    }

    /// Alternatives in grammar order, each with its longest viable match, until the group is
    /// complete at `end`.
    fn unordered(
        &mut self,
        alternatives: &[NodeId],
        all: bool,
        items: &[Item],
        pos: usize,
        end: usize,
    ) -> Vec<Option<Component>> {
        let mut slots = vec![None; alternatives.len()];
        let mut viable = Memo::default();
        let (mut used, mut at) = (0u64, pos);
        while at != end || !complete(all, alternatives.len(), used) {
            let mut chosen = None;
            'alternatives: for (index, alternative) in alternatives.iter().enumerate() {
                if used & (1 << index) != 0 {
                    continue;
                }
                for candidate in self.ends(*alternative, items, at).into_iter().rev() {
                    let state = (used | 1 << index, candidate);
                    if candidate <= end
                        && self.unordered_reaches(alternatives, all, state, items, end, &mut viable)
                    {
                        chosen = Some((index, candidate));
                        break 'alternatives;
                    }
                }
            }
            let Some((index, next)) = chosen else {
                break;
            };
            slots[index] = self.build(alternatives[index], items, at, next);
            used |= 1 << index;
            at = next;
        }
        slots
    }

    fn unordered_reaches(
        &mut self,
        alternatives: &[NodeId],
        all: bool,
        (used, at): (u64, usize),
        items: &[Item],
        end: usize,
        viable: &mut Memo<(u64, usize), bool>,
    ) -> bool {
        if let Some(known) = viable.get(&(used, at)) {
            return *known;
        }
        let mut reaches = at == end && complete(all, alternatives.len(), used);
        for (index, alternative) in alternatives.iter().enumerate() {
            if reaches {
                break;
            }
            if used & (1 << index) != 0 {
                continue;
            }
            for candidate in self.ends(*alternative, items, at) {
                if candidate <= end
                    && self.unordered_reaches(
                        alternatives,
                        all,
                        (used | 1 << index, candidate),
                        items,
                        end,
                        viable,
                    )
                {
                    reaches = true;
                    break;
                }
            }
        }
        viable.insert((used, at), reaches);
        reaches
    }

    /// Iterations take their longest viable match. Viability is computed from `end` backwards
    /// so that long repetitions need no recursion.
    fn repeat(
        &mut self,
        repeat: &Repeat<'_>,
        items: &[Item],
        pos: usize,
        end: usize,
    ) -> Vec<Component> {
        let counts = repeat.capped(repeat.max).min(end - pos + repeat.min) + 1;
        let positions = end - pos + 1;
        let mut viable = vec![vec![false; positions]; counts];
        for count in (0..counts).rev() {
            for at in (pos..=end).rev() {
                let mut reaches = at == end && count >= repeat.min;
                if !reaches
                    && count < repeat.max
                    && let Some(start) = repeat.start(count + 1, items, at)
                {
                    let next = repeat.capped(count + 1);
                    reaches = self
                        .operand_ends(repeat.node, items, start)
                        .into_iter()
                        .any(|candidate| {
                            candidate <= end
                                && (candidate > at || count < repeat.min)
                                && next < counts
                                && viable[next][candidate - pos]
                        });
                }
                viable[count][at - pos] = reaches;
            }
        }
        let mut output = Vec::new();
        let (mut count, mut at) = (0, pos);
        while !(at == end && count >= repeat.min) {
            let Some(start) = repeat.start(count + 1, items, at) else {
                break;
            };
            let next = repeat.capped(count + 1);
            let Some(candidate) = self
                .operand_ends(repeat.node, items, start)
                .into_iter()
                .rev()
                .find(|candidate| {
                    *candidate <= end
                        && (*candidate > at || count < repeat.min)
                        && next < counts
                        && viable[next][*candidate - pos]
                })
            else {
                break;
            };
            output.extend(self.operand_build(repeat.node, items, start, candidate));
            count += 1;
            at = candidate;
        }
        output
    }

    fn function(&mut self, name: Keyword, body: Option<NodeId>, item: &Item) -> bool {
        match &item.kind {
            ItemKind::Function {
                keyword: Some(found),
                arguments,
                ..
            } if *found == name => match body {
                None => arguments.is_empty(),
                Some(body) => self.ends(body, arguments, 0).contains(&arguments.len()),
            },
            _ => false,
        }
    }

    fn base(&mut self, base: Base, range: Range, item: &Item) -> Option<Component> {
        match (&item.kind, base) {
            (_, Base::Color) => self.color(item),
            (ItemKind::Function { name, .. }, _) if calc::is_math_function(name) => {
                self.math_function(base, range, item)
            }
            _ => self.single(base, range, item),
        }
    }

    /// Math functions wherever a numeric type is accepted (CSS Values 4 §10): the single
    /// integration point with `calc`, which type-checks the function against the base type
    /// here and resolves it, clamped into the grammar's range, at computed-value time.
    fn math_function(&self, base: Base, range: Range, item: &Item) -> Option<Component> {
        let (kind, percentage) = match base {
            Base::Number | Base::Integer => (Kind::Number, None),
            Base::Percentage => (Kind::Percentage, Some(Kind::Percentage)),
            Base::Length => (Kind::Length, None),
            Base::LengthPercentage => (Kind::Length, Some(Kind::Length)),
            Base::Angle => (Kind::Angle, None),
            Base::AnglePercentage => (Kind::Angle, Some(Kind::Angle)),
            Base::Time => (Kind::Time, None),
            Base::Resolution => (Kind::Resolution, None),
            _ => return None,
        };
        let source = &self.source[item.start..item.end];
        let mut input = ParserInput::new(source);
        // Validation only needs the type; initial values stand in for the element's.
        let relative = self.context.relative();
        let context = Context {
            relative: &relative,
            percentage,
            channels: &[],
        };
        match calc::math_function(&mut Parser::new(&mut input), context)?
            .and_then(|math| math.expect(&[kind]))
        {
            Ok(_) => Some(Component::Math(source.into(), kind, range)),
            Err(calc::MathError::Resolution(error @ StyleError::GrammarLimit { .. })) => {
                *self.failure.borrow_mut() = Some(error);
                None
            }
            Err(_) => None,
        }
    }

    fn single(&self, base: Base, range: Range, item: &Item) -> Option<Component> {
        let signed = |value: f32| range.contains(value).then_some(value);
        Some(match (base, &item.kind) {
            (Base::Number, ItemKind::Number { value, .. }) => Component::Number(signed(*value)?),
            (
                Base::Integer,
                ItemKind::Number {
                    integer: Some(value),
                    ..
                },
            ) => {
                signed(*value as f32)?;
                Component::Integer(*value)
            }
            (Base::Zero, ItemKind::Number { value, .. }) if *value == 0.0 => Component::Number(0.0),
            (
                Base::Percentage | Base::LengthPercentage | Base::AnglePercentage,
                ItemKind::Percentage(fraction),
            ) => {
                // Unitless percentage bounds are in percent; dimension bounds are signs.
                let scale = if base == Base::Percentage {
                    PERCENT_RANGE_SCALE
                } else {
                    1.0
                };
                signed(fraction * scale)?;
                Component::Percentage(*fraction)
            }
            (Base::Length | Base::LengthPercentage, ItemKind::Number { value, .. })
                if *value == 0.0 =>
            {
                Component::Length(Length::ZERO)
            }
            (Base::Dimension, ItemKind::Dimension { value, unit }) => {
                Component::Dimension(*value, unit.clone())
            }
            (_, ItemKind::Dimension { value, unit }) => match (base, units::dimension(unit)?) {
                (Base::Length | Base::LengthPercentage, Dimension::Length(unit)) => {
                    Component::Length(if *value == 0.0 {
                        Length::ZERO
                    } else {
                        Length {
                            value: signed(*value)?,
                            unit,
                        }
                    })
                }
                (Base::Angle | Base::AnglePercentage, Dimension::Angle(degrees)) => {
                    Component::Angle(signed(*value * degrees)?)
                }
                (Base::Time, Dimension::Time(seconds)) => {
                    Component::Time(signed(*value)? * seconds)
                }
                (Base::Resolution, Dimension::Resolution(dppx)) => {
                    Component::Resolution(signed(*value)? * dppx)
                }
                (Base::Flex, Dimension::Flex) => Component::Flex(signed(*value)?),
                _ => return None,
            },
            (Base::String, ItemKind::String(text)) => Component::String(text.clone()),
            (Base::Url, ItemKind::Url(url)) => Component::Url(url.clone()),
            (Base::Ident, ItemKind::Ident { name, .. }) => Component::Ident(name.clone()),
            (Base::CustomIdent, ItemKind::Ident { name, .. }) => {
                if WideKeyword::parse_ident(name).is_some()
                    || name.eq_ignore_ascii_case(RESERVED_IDENT)
                    || self.excluded.contains(&item.start)
                {
                    return None;
                }
                Component::Ident(name.clone())
            }
            (Base::DashedIdent, ItemKind::Ident { name, .. })
                if name.starts_with(DASHED_PREFIX) =>
            {
                Component::Ident(name.clone())
            }
            (Base::HashToken, ItemKind::Hash(hash)) => Component::Hash(hash.clone()),
            (Base::AttrUnit, ItemKind::Ident { name, .. }) if units::dimension(name).is_some() => {
                Component::Unit(name.to_ascii_lowercase().into())
            }
            (Base::AttrUnit, ItemKind::Delim(PERCENT)) => {
                Component::Unit(PERCENT.to_string().into())
            }
            _ => return None,
        })
    }

    /// Colours that depend on the element are kept as source for computed-value time.
    fn color(&mut self, item: &Item) -> Option<Component> {
        let source = &self.source[item.start..item.end];
        let parsed = crate::style::colors::parse(
            source,
            &self.context.relative(),
            &crate::paint::AbsoluteColor::BLACK,
        )
        .ok()?;
        Some(if parsed.dependent {
            Component::MathColor(source.into())
        } else {
            Component::Color(parsed.color)
        })
    }

    /// `U+…` ranges span several tokens, so cssparser re-reads them from the source.
    fn unicode_range(&self, items: &[Item], pos: usize) -> Option<(Component, usize)> {
        let start = items.get(pos)?.start;
        let text = &self.source[start..items.last()?.end];
        let mut input = ParserInput::new(text);
        let mut parser = Parser::new(&mut input);
        let range = UnicodeRange::parse(&mut parser).ok()?;
        let consumed = start + parser.position().byte_index();
        let next = items[pos..]
            .iter()
            .position(|item| item.end == consumed)
            .map(|offset| pos + offset + 1)?;
        Some((Component::UnicodeRange(range.start, range.end), next))
    }
}

fn repeat(node: NodeId, min: u16, max: u16, comma: bool) -> Repeat<'static> {
    Repeat {
        node: Operand::Static(node),
        min: usize::from(min),
        max: usize::from(max),
        comma,
    }
}

/// Grammar commas are omitted when the terms on either side are (CSS Values 4 §2.6): at the
/// start of a list, after another comma, or at its end.
fn comma_omitted(items: &[Item], pos: usize) -> bool {
    pos == 0 || pos == items.len() || is_comma(items.get(pos - 1))
}
/// A written comma separates terms, so it cannot start or end a list or follow another.
fn comma_consumed(items: &[Item], pos: usize) -> bool {
    pos > 0
        && !is_comma(items.get(pos - 1))
        && is_comma(items.get(pos))
        && items
            .get(pos + 1)
            .is_some_and(|next| !matches!(next.kind, ItemKind::Comma))
}
fn is_comma(item: Option<&Item>) -> bool {
    matches!(item.map(|item| &item.kind), Some(ItemKind::Comma))
}

/// `<declaration-value>` stops before a top-level `!` (CSS Syntax 3 §8.2).
fn declaration_end(items: &[Item], pos: usize) -> usize {
    items[pos..]
        .iter()
        .position(|item| matches!(item.kind, ItemKind::Delim('!')))
        .map_or(items.len(), |offset| pos + offset)
}
