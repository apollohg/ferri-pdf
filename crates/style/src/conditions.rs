use crate::{
    StyleError,
    media::{Feature, MediaQuery, matches_media},
    parser::{MAX_COMPONENT_DEPTH, consume},
    print::PrintEnvironment,
    supports::{SupportNames, SupportTest, SupportsCondition, matches_supports},
};
use cssparser::{Parser, ParserInput, Token};
use std::collections::{BTreeMap, HashMap};

pub(super) const MAX_CONDITION_DEPTH: usize = MAX_COMPONENT_DEPTH - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Truth {
    False,
    True,
    Unknown,
}

impl From<bool> for Truth {
    fn from(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }
}

impl Truth {
    pub(crate) fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
        }
    }
    pub(crate) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::False, _) | (_, Self::False) => Self::False,
            (Self::True, Self::True) => Self::True,
            _ => Self::Unknown,
        }
    }
    pub(crate) fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::True, _) | (_, Self::True) => Self::True,
            (Self::False, Self::False) => Self::False,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone)]
enum Term<T> {
    Constant(Truth),
    Value(T),
    Not(usize),
    And(Vec<usize>),
    Or(Vec<usize>),
}

#[derive(Clone)]
pub(super) struct Condition<T> {
    terms: Vec<Term<T>>,
}

enum Part<'i> {
    Parentheses(&'i str),
    Function(&'i str),
}
enum Join {
    Not,
    And,
    Or,
}
enum Position {
    Root,
    Parentheses,
    Function,
}

/// Captures a block while cssparser skips its nested blocks iteratively.
pub(super) fn block_source<'i>(input: &mut Parser<'i, '_>) -> &'i str {
    let start = input.position();
    while input.next_including_whitespace_and_comments().is_ok() {}
    input.slice_from(start)
}

fn part<'i>(input: &mut Parser<'i, '_>) -> Result<Part<'i>, ()> {
    let start = input.position();
    match input.next().map_err(|_| ())?.clone() {
        Token::ParenthesisBlock => input
            .parse_nested_block(|nested| {
                Ok::<_, cssparser::ParseError<'i, ()>>(Part::Parentheses(block_source(nested)))
            })
            .map_err(|_| ()),
        Token::Function(_) => {
            input
                .parse_nested_block(|nested| {
                    block_source(nested);
                    Ok::<_, cssparser::ParseError<'i, ()>>(())
                })
                .map_err(|_| ())?;
            Ok(Part::Function(input.slice_from(start)))
        }
        _ => Err(()),
    }
}

fn level(source: &str, allow_or: bool) -> Result<(Join, Vec<Part<'_>>), ()> {
    let mut input = ParserInput::new(source);
    let input = &mut Parser::new(&mut input);
    if input
        .try_parse(|input| input.expect_ident_matching("not"))
        .is_ok()
    {
        let part = part(input)?;
        input.expect_exhausted().map_err(|_| ())?;
        return Ok((Join::Not, vec![part]));
    }
    let mut parts = vec![part(input)?];
    let mut join = None;
    while !input.is_exhausted() {
        let operator = input.expect_ident_cloned().map_err(|_| ())?;
        let next = if operator.eq_ignore_ascii_case("and") {
            false
        } else if allow_or && operator.eq_ignore_ascii_case("or") {
            true
        } else {
            return Err(());
        };
        if join.is_some_and(|previous| previous != next) {
            return Err(());
        }
        join = Some(next);
        parts.push(part(input)?);
    }
    Ok((
        if join == Some(true) {
            Join::Or
        } else {
            Join::And
        },
        parts,
    ))
}

impl<T> Condition<T> {
    pub(crate) fn constant(value: Truth) -> Self {
        Self {
            terms: vec![Term::Constant(value)],
        }
    }

    pub(crate) fn value(value: T) -> Self {
        Self {
            terms: vec![Term::Value(value)],
        }
    }

    pub(crate) fn parse(
        source: &str,
        allow_or: bool,
        fallback: Truth,
        leaf: impl Fn(&str) -> Option<T>,
    ) -> Result<Option<Self>, StyleError> {
        Self::parse_fallible(source, allow_or, fallback, |source| Ok(leaf(source)))
    }

    pub(crate) fn parse_fallible(
        source: &str,
        allow_or: bool,
        fallback: Truth,
        mut leaf: impl FnMut(&str) -> Result<Option<T>, StyleError>,
    ) -> Result<Option<Self>, StyleError> {
        let mut terms = vec![Term::Constant(fallback)];
        let mut pending = vec![(source, Position::Root, 0, 0)];
        while let Some((source, position, depth, index)) = pending.pop() {
            if depth > MAX_CONDITION_DEPTH {
                return Err(StyleError::GrammarLimit {
                    resource: "condition nesting",
                    limit: MAX_CONDITION_DEPTH,
                });
            }
            if !matches!(position, Position::Root)
                && let Some(value) = leaf(source)?
            {
                terms[index] = Term::Value(value);
                continue;
            }
            if matches!(position, Position::Function) {
                continue;
            }
            let Ok((join, parts)) = level(source, allow_or || !matches!(position, Position::Root))
            else {
                if matches!(position, Position::Root) {
                    return Ok(None);
                }
                continue;
            };
            let mut children = Vec::with_capacity(parts.len());
            for part in parts {
                let child = terms.len();
                terms.push(Term::Constant(fallback));
                children.push(child);
                let (source, position) = match part {
                    Part::Parentheses(source) => (source, Position::Parentheses),
                    Part::Function(source) => (source, Position::Function),
                };
                pending.push((source, position, depth + 1, child));
            }
            terms[index] = match join {
                Join::Not => Term::Not(children[0]),
                Join::And => Term::And(children),
                Join::Or => Term::Or(children),
            };
        }
        let mut input = ParserInput::new(source);
        match consume(&mut Parser::new(&mut input)) {
            Ok(()) => Ok(Some(Self { terms })),
            Err(error) => match crate::parser::failure(error) {
                StyleError::Invalid(_) => Ok(None),
                error => Err(error),
            },
        }
    }

    pub(crate) fn evaluate(&self, mut value: impl FnMut(&T) -> Truth) -> Truth {
        match self.try_evaluate(|term| Ok::<_, std::convert::Infallible>(value(term))) {
            Ok(value) => value,
            Err(never) => match never {},
        }
    }

    pub(crate) fn try_evaluate<E>(
        &self,
        mut value: impl FnMut(&T) -> Result<Truth, E>,
    ) -> Result<Truth, E> {
        let mut values = vec![Truth::Unknown; self.terms.len()];
        let mut pending = vec![(0, 0)];
        while let Some((index, next)) = pending.pop() {
            match &self.terms[index] {
                Term::Constant(value) => values[index] = *value,
                Term::Value(term) => values[index] = value(term)?,
                Term::Not(child) => {
                    if next == 0 {
                        pending.push((index, 1));
                        pending.push((*child, 0));
                    } else {
                        values[index] = values[*child].not();
                    }
                }
                term @ (Term::And(children) | Term::Or(children)) => {
                    let conjunction = matches!(term, Term::And(_));
                    if next == 0 {
                        values[index] = conjunction.into();
                    } else {
                        let child = values[children[next - 1]];
                        values[index] = if conjunction {
                            values[index].and(child)
                        } else {
                            values[index].or(child)
                        };
                    }
                    let determined = values[index] == Truth::from(!conjunction);
                    if !determined && next < children.len() {
                        pending.push((index, next + 1));
                        pending.push((children[next], 0));
                    }
                }
            }
        }
        Ok(values[0])
    }

    pub(crate) fn leaves(&self) -> impl Iterator<Item = &T> {
        self.terms.iter().filter_map(|term| match term {
            Term::Value(value) => Some(value),
            _ => None,
        })
    }
}

#[derive(Clone)]
enum WhenTest {
    Media(Feature),
    Supports(SupportTest),
}

#[derive(Clone)]
pub(super) struct WhenCondition {
    condition: Condition<WhenTest>,
    names: SupportNames,
}

impl WhenCondition {
    pub(crate) fn parse(source: &str) -> Result<Option<Self>, StyleError> {
        if !crate::parser::balanced_components(source)? {
            return Ok(None);
        }
        Ok(Condition::parse(source, true, Truth::Unknown, |source| {
            let mut input = ParserInput::new(source);
            let input = &mut Parser::new(&mut input);
            let Token::Function(name) = input.next().ok()?.clone() else {
                return None;
            };
            let leaf = input
                .parse_nested_block(|nested| {
                    let source = block_source(nested);
                    Ok::<_, cssparser::ParseError<'_, ()>>(if name.eq_ignore_ascii_case("media") {
                        Feature::parse(source).map(WhenTest::Media)
                    } else if name.eq_ignore_ascii_case("supports") {
                        SupportTest::declaration(source).map(WhenTest::Supports)
                    } else {
                        None
                    })
                })
                .ok()??;
            input.expect_exhausted().ok()?;
            Some(leaf)
        })?
        .map(|condition| Self {
            condition,
            names: Default::default(),
        }))
    }
}

#[derive(Clone)]
pub(super) enum RuleQuery {
    Constant(bool),
    Media(MediaQuery),
    Supports(SupportsCondition),
    When(WhenCondition),
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(super) enum NameKind {
    Media,
    Supports,
}
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
struct NameRef<'a> {
    kind: NameKind,
    name: &'a str,
}

impl RuleQuery {
    pub(crate) fn bind(&mut self, environment: &PrintEnvironment) {
        match self {
            Self::Supports(condition) => condition.bind(&environment.supports),
            Self::When(condition) => condition.names = environment.supports.clone(),
            _ => {}
        }
    }
    pub(super) fn matches(&self, environment: &PrintEnvironment) -> bool {
        match self {
            Self::Constant(value) => *value,
            Self::Media(query) => matches_media(query, environment),
            Self::Supports(query) => matches_supports(query),
            Self::When(query) => {
                query.condition.evaluate(|test| match test {
                    WhenTest::Media(feature) => feature.evaluate(environment),
                    WhenTest::Supports(test) => test.evaluate(&query.names).into(),
                }) == Truth::True
            }
        }
    }
    fn dependencies(&self) -> Vec<NameRef<'_>> {
        match self {
            Self::Constant(_) => Vec::new(),
            Self::Media(query) => query
                .dependencies()
                .map(|name| NameRef {
                    kind: NameKind::Media,
                    name,
                })
                .collect(),
            Self::Supports(query) => query
                .dependencies()
                .map(|name| NameRef {
                    kind: NameKind::Supports,
                    name,
                })
                .collect(),
            Self::When(query) => query
                .condition
                .leaves()
                .filter_map(|test| match test {
                    WhenTest::Media(feature) => feature.dependency().map(|name| NameRef {
                        kind: NameKind::Media,
                        name,
                    }),
                    WhenTest::Supports(test) => test.name().map(|name| NameRef {
                        kind: NameKind::Supports,
                        name,
                    }),
                })
                .collect(),
        }
    }
    fn uses_paper(&self, environment: &PrintEnvironment) -> bool {
        match self {
            Self::Constant(_) => false,
            Self::Media(query) => query.uses_paper(environment),
            Self::Supports(query) => query.uses_paper(),
            Self::When(query) => query.condition.leaves().any(|test| match test {
                WhenTest::Media(feature) => feature.uses_paper(environment),
                WhenTest::Supports(test) => test.name().is_some_and(|name| {
                    environment
                        .supports
                        .borrow()
                        .get(name)
                        .is_some_and(|value| value.paper_dependent)
                }),
            }),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct NamedValue {
    pub matching: Option<bool>,
    pub paper_dependent: bool,
}
impl NamedValue {
    fn undefined(paper_dependent: bool) -> Self {
        Self {
            matching: None,
            paper_dependent,
        }
    }
}
impl PrintEnvironment {
    fn named(&self, name: NameRef<'_>) -> Option<NamedValue> {
        match name.kind {
            NameKind::Media => self.custom_media.get(name.name).copied(),
            NameKind::Supports => self.supports.borrow().get(name.name).copied(),
        }
    }
    fn define(&mut self, name: NameRef<'_>, value: NamedValue) {
        match name.kind {
            NameKind::Media => {
                self.custom_media.insert(name.name.to_owned(), value);
            }
            NameKind::Supports => {
                self.supports
                    .borrow_mut()
                    .insert(name.name.to_owned(), value);
            }
        }
    }
}

pub(super) struct RuleCondition {
    pub parent: Option<usize>,
    pub previous: Option<usize>,
    pub query: RuleQuery,
}
pub(super) struct NamedDefinition {
    pub order: Vec<usize>,
    pub condition: Option<usize>,
    pub query: RuleQuery,
}
pub(super) type Definitions = BTreeMap<NameKind, BTreeMap<String, Vec<NamedDefinition>>>;

#[derive(Clone, Copy)]
struct Gate {
    index: usize,
    negate: bool,
}
fn gates(condition: Option<usize>, conditions: &[RuleCondition]) -> Vec<Gate> {
    let mut ancestors: Vec<_> =
        std::iter::successors(condition, |&index| conditions[index].parent).collect();
    ancestors.reverse();
    let mut gates = Vec::new();
    for index in ancestors {
        let mut previous: Vec<_> = std::iter::successors(conditions[index].previous, |&previous| {
            conditions[previous].previous
        })
        .collect();
        previous.reverse();
        gates.extend(previous.into_iter().map(|index| Gate {
            index,
            negate: true,
        }));
        gates.push(Gate {
            index,
            negate: false,
        });
    }
    gates
}

struct AliasFrame<'a> {
    name: NameRef<'a>,
    candidate: usize,
    gates: Vec<Gate>,
    gate: usize,
    body: bool,
    dependencies: Vec<NameRef<'a>>,
    next: usize,
    paper_dependent: bool,
}
impl<'a> AliasFrame<'a> {
    fn new(
        name: NameRef<'a>,
        candidate: usize,
        definition: &'a NamedDefinition,
        conditions: &'a [RuleCondition],
    ) -> Self {
        let gates = gates(definition.condition, conditions);
        let body = gates.is_empty();
        let dependencies = if body {
            definition.query.dependencies()
        } else {
            conditions[gates[0].index].query.dependencies()
        };
        Self {
            name,
            candidate,
            gates,
            gate: 0,
            body,
            dependencies,
            next: 0,
            paper_dependent: false,
        }
    }
    fn query<'b>(
        &self,
        definitions: &'b Definitions,
        conditions: &'b [RuleCondition],
    ) -> &'b RuleQuery {
        if self.body {
            &definitions[&self.name.kind][self.name.name][self.candidate].query
        } else {
            &conditions[self.gates[self.gate].index].query
        }
    }
}

pub(super) fn resolve_names(
    definitions: &Definitions,
    conditions: &[RuleCondition],
    environment: &mut PrintEnvironment,
) {
    for (&kind, names) in definitions {
        for (name, candidates) in names {
            let name = NameRef { kind, name };
            if environment.named(name).is_some() {
                continue;
            }
            let candidate = candidates.len() - 1;
            let mut stack = vec![AliasFrame::new(
                name,
                candidate,
                &candidates[candidate],
                conditions,
            )];
            let mut active = HashMap::from([(name, 0)]);
            while let Some(frame) = stack.last_mut() {
                if let Some(&dependency) = frame.dependencies.get(frame.next) {
                    frame.next += 1;
                    if environment.named(dependency).is_some() {
                        continue;
                    }
                    if let Some(&start) = active.get(&dependency) {
                        let paper = stack[start..].iter().any(|frame| {
                            frame.paper_dependent
                                || frame.query(definitions, conditions).uses_paper(environment)
                        });
                        for frame in stack.drain(start..) {
                            environment.define(frame.name, NamedValue::undefined(paper));
                            active.remove(&frame.name);
                        }
                        continue;
                    }
                    let Some(candidates) = definitions
                        .get(&dependency.kind)
                        .and_then(|names| names.get(dependency.name))
                    else {
                        environment.define(dependency, NamedValue::undefined(false));
                        continue;
                    };
                    let candidate = candidates.len() - 1;
                    active.insert(dependency, stack.len());
                    stack.push(AliasFrame::new(
                        dependency,
                        candidate,
                        &candidates[candidate],
                        conditions,
                    ));
                    continue;
                }
                let query = frame.query(definitions, conditions);
                frame.paper_dependent |= query.uses_paper(environment);
                if !frame.body {
                    let gate = frame.gates[frame.gate];
                    if query.matches(environment) == gate.negate {
                        if frame.candidate > 0 {
                            let candidate = frame.candidate - 1;
                            let paper = frame.paper_dependent;
                            *frame = AliasFrame::new(
                                frame.name,
                                candidate,
                                &definitions[&frame.name.kind][frame.name.name][candidate],
                                conditions,
                            );
                            frame.paper_dependent = paper;
                        } else {
                            environment
                                .define(frame.name, NamedValue::undefined(frame.paper_dependent));
                            active.remove(&frame.name);
                            stack.pop();
                        }
                        continue;
                    }
                    frame.gate += 1;
                    frame.body = frame.gate == frame.gates.len();
                    frame.dependencies = frame.query(definitions, conditions).dependencies();
                    frame.next = 0;
                    continue;
                }
                environment.define(
                    frame.name,
                    NamedValue {
                        matching: Some(query.matches(environment)),
                        paper_dependent: frame.paper_dependent,
                    },
                );
                active.remove(&frame.name);
                stack.pop();
            }
        }
    }
}

pub(super) struct Applicability {
    pub matching: bool,
    pub paper_dependent: bool,
    taken: bool,
}
pub(super) fn applicability(
    conditions: &[RuleCondition],
    environment: &PrintEnvironment,
) -> Vec<Applicability> {
    let mut result: Vec<Applicability> = Vec::with_capacity(conditions.len());
    extend_applicability(conditions, environment, &mut result);
    result
}
pub(super) fn extend_applicability(
    conditions: &[RuleCondition],
    environment: &PrintEnvironment,
    result: &mut Vec<Applicability>,
) {
    for condition in &conditions[result.len()..] {
        let previous = condition.previous.map(|index| &result[index]);
        let taken = previous.is_some_and(|previous| previous.taken);
        let matching = !taken && condition.query.matches(environment);
        result.push(Applicability {
            matching: matching
                && condition
                    .parent
                    .is_none_or(|parent| result[parent].matching),
            taken: taken || matching,
            paper_dependent: condition
                .parent
                .is_some_and(|parent| result[parent].paper_dependent)
                || previous.is_some_and(|previous| previous.paper_dependent)
                || condition.query.uses_paper(environment),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{calc::Work, grammar::Property, matcher::MAX_MATCH_STEPS};

    #[test]
    fn determined_boolean_conditions_do_not_request_unused_computed_values() {
        for (source, width, expected) in [
            ("(width) and (height)", Truth::False, Truth::False),
            ("(width) or (height)", Truth::True, Truth::True),
            (
                "((width) and (height)) or (color)",
                Truth::False,
                Truth::True,
            ),
            ("future-query() and (width)", Truth::False, Truth::False),
        ] {
            let condition = Condition::parse(source, true, Truth::Unknown, Property::from_name)
                .unwrap()
                .unwrap();
            let work = Work::default();
            let actual = condition.try_evaluate(|property| match property {
                Property::Width => Ok(width),
                Property::Color => Ok(Truth::True),
                Property::Height => work.charge(MAX_MATCH_STEPS + 1).map(|()| Truth::True),
                _ => unreachable!("only the named query properties are parsed"),
            });
            assert_eq!(
                actual.unwrap_or_else(|error| panic!("unneeded query in {source}: {error}")),
                expected
            );
        }
    }

    #[test]
    fn unresolved_boolean_conditions_preserve_required_computation_errors() {
        for source in [
            "not (height)",
            "future-query() or (height)",
            "(width) and (height)",
        ] {
            let condition = Condition::parse(source, true, Truth::Unknown, Property::from_name)
                .unwrap()
                .unwrap();
            let work = Work::default();
            let actual = condition.try_evaluate(|property| {
                if *property == Property::Width {
                    Ok(Truth::True)
                } else {
                    work.charge(MAX_MATCH_STEPS + 1).map(|()| Truth::True)
                }
            });
            assert!(
                matches!(
                    actual,
                    Err(StyleError::GrammarLimit {
                        limit: MAX_MATCH_STEPS,
                        ..
                    })
                ),
                "a required computation must retain its typed limit in {source}: {actual:?}"
            );
        }
    }
}
