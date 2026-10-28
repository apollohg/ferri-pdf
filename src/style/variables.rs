use crate::style::{StyleError, wide_keywords::WideKeyword};
use cssparser::{ParseError, ParseErrorKind, Parser, ParserInput, Token, TokenSerializationType};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    ops::Range,
    rc::Rc,
    sync::Arc,
};

const MAX_VALUE_BYTES: usize = 1024 * 1024;
const CSS_STRING_DELIMITER_BYTES: usize = "\"\"".len();
const MAX_ENVIRONMENT_ENTRIES: usize = 16384;
const MAX_ENVIRONMENT_BYTES: usize = 16 * MAX_VALUE_BYTES;
const MAX_DEPENDENCY_DEPTH: usize = 256;
pub(super) const MAX_VALUE_DEPTH: usize = 128;

#[derive(Clone, Debug, PartialEq)]
enum Origin {
    Values(Range<usize>),
    Empty {
        position: usize,
        function: Option<usize>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Expanded {
    source: Arc<str>,
    taint: Option<Arc<[Origin]>>,
}
impl Expanded {
    pub(super) fn as_str(&self) -> &str {
        &self.source
    }
    pub(super) fn len(&self) -> usize {
        self.source.len()
    }
    fn storage_len(&self) -> usize {
        self.len() + self.taint.as_deref().map_or(0, size_of_val)
    }
    pub(super) fn has_taint(&self) -> bool {
        self.taint.is_some()
    }
    pub(super) fn taints(&self, range: Range<usize>) -> bool {
        self.taint.as_ref().is_some_and(|taint| {
            taint.iter().any(|origin| match origin {
                Origin::Values(origin) => origin.start < range.end && range.start < origin.end,
                Origin::Empty {
                    position,
                    function: Some(function),
                } => range.start <= *function && *position <= range.end,
                Origin::Empty { function: None, .. } => false,
            })
        })
    }

    pub(super) fn attribute_string(source: &str) -> Result<Self, StyleError> {
        if source.len() > MAX_VALUE_BYTES - CSS_STRING_DELIMITER_BYTES {
            return Err(Output::byte_limit(std::fmt::Error));
        }
        let mut output = Output::default();
        cssparser::serialize_string(source, &mut output).map_err(Output::byte_limit)?;
        Ok(Self {
            source: output.value.into(),
            taint: None,
        }
        .attribute_tainted())
    }
    pub(super) fn attribute_tainted(mut self) -> Self {
        self.taint = Some(Arc::from([if self.source.is_empty() {
            Origin::Empty {
                position: 0,
                function: None,
            }
        } else {
            Origin::Values(0..self.source.len())
        }]));
        self
    }
}

pub(super) type Environment = Arc<Variables>;
#[derive(Default, Debug, PartialEq)]
pub(super) struct Variables {
    parent: Option<Environment>,
    values: HashMap<String, Option<Expanded>>,
    bytes: usize,
    entries: usize,
}
impl Variables {
    pub(super) fn get(&self, name: &str) -> Option<&Option<Expanded>> {
        let mut current = self;
        loop {
            if let Some(value) = current.values.get(name) {
                return Some(value);
            }
            current = current.parent.as_deref()?;
        }
    }
    fn charge_bytes(&mut self, bytes: usize) -> Result<(), StyleError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|total| *total <= MAX_ENVIRONMENT_BYTES)
            .ok_or(StyleError::VariableLimit {
                resource: "environment bytes",
                limit: MAX_ENVIRONMENT_BYTES,
            })?;
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub(super) struct Value(Vec<Part>);
#[derive(Clone, Debug)]
enum Part {
    Text(String, TokenSerializationType),
    Opaque(String, TokenSerializationType),
    Block(String, TokenSerializationType, Value, &'static str),
    Reference(Reference, String, Option<Value>),
    Conditional(Box<crate::style::conditional::Expression>),
    Attribute(Box<crate::style::attribute::Attribute>),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Reference {
    Local,
    Inherited,
}
const REFERENCE_FUNCTIONS: &[(&str, Reference)] =
    &[("var", Reference::Local), ("inherit", Reference::Inherited)];
impl Reference {
    fn from_name(name: &str) -> Option<Self> {
        REFERENCE_FUNCTIONS
            .iter()
            .find_map(|(function, reference)| {
                name.eq_ignore_ascii_case(function).then_some(*reference)
            })
    }
}
#[derive(Debug)]
enum ValueError {
    Syntax,
    Depth,
    Style(StyleError),
}
impl From<StyleError> for ValueError {
    fn from(error: StyleError) -> Self {
        Self::Style(error)
    }
}
impl Value {
    pub(super) fn with_substitutions(source: &str) -> Result<Option<Self>, StyleError> {
        Self::substitutions(source, false)
    }
    pub(super) fn with_property_substitutions(source: &str) -> Result<Option<Self>, StyleError> {
        Self::substitutions(source, true)
    }
    fn substitutions(source: &str, property_syntax: bool) -> Result<Option<Self>, StyleError> {
        if !source.contains('\\')
            && !REFERENCE_FUNCTIONS
                .iter()
                .map(|(name, _)| *name)
                .chain([
                    crate::style::conditional::FUNCTION,
                    crate::style::attribute::FUNCTION,
                ])
                .any(|name| {
                    source.as_bytes().windows(name.len() + 1).any(|part| {
                        part[name.len()] == b'('
                            && part[..name.len()].eq_ignore_ascii_case(name.as_bytes())
                    })
                })
        {
            return Ok(None);
        }
        let value = Self::parse_context(source, 0, property_syntax)?
            .ok_or_else(|| StyleError::Invalid("invalid value substitution syntax".into()))?;
        Ok(value.has_substitutions().then_some(value))
    }
    pub(super) fn parse(source: &str) -> Result<Option<Self>, StyleError> {
        Self::parse_at_depth(source, 0)
    }
    pub(super) fn parse_at_depth(source: &str, depth: usize) -> Result<Option<Self>, StyleError> {
        Self::parse_context(source, depth, false)
    }
    pub(super) fn parse_context(
        source: &str,
        depth: usize,
        property_syntax: bool,
    ) -> Result<Option<Self>, StyleError> {
        if source.len() > MAX_VALUE_BYTES {
            return Err(StyleError::VariableLimit {
                resource: "specified value bytes",
                limit: MAX_VALUE_BYTES,
            });
        }

        let mut input = ParserInput::new(crate::style::parser::trim_value(source));
        match Self::tokens(&mut Parser::new(&mut input), depth, false, property_syntax) {
            Ok(value) => Ok(Some(value)),
            Err(error) => match error.kind {
                ParseErrorKind::Custom(ValueError::Depth) => Err(StyleError::VariableLimit {
                    resource: "value nesting",
                    limit: MAX_VALUE_DEPTH,
                }),
                ParseErrorKind::Custom(ValueError::Style(error)) => Err(error),
                _ => Ok(None),
            },
        }
    }
    fn tokens<'i>(
        input: &mut Parser<'i, '_>,
        depth: usize,
        in_block: bool,
        property_syntax: bool,
    ) -> Result<Self, ParseError<'i, ValueError>> {
        if depth > MAX_VALUE_DEPTH {
            return Err(input.new_custom_error(ValueError::Depth));
        }
        let mut parts = Vec::new();
        loop {
            let start = input.position();
            let token = match input.next_including_whitespace_and_comments().cloned() {
                Ok(token) => token,
                Err(error) if matches!(error.kind, cssparser::BasicParseErrorKind::EndOfInput) => {
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            let kind = if matches!(token, Token::Comment(_)) {
                TokenSerializationType::WhiteSpace
            } else {
                token.serialization_type()
            };
            match token {
                Token::Function(name)
                    if name.eq_ignore_ascii_case(crate::style::attribute::FUNCTION) =>
                {
                    let value = input.parse_nested_block(|nested| {
                        crate::style::attribute::Attribute::parse(
                            nested,
                            depth + 1,
                            property_syntax,
                        )
                        .map_err(|error| nested.new_custom_error(ValueError::Style(error)))?
                        .ok_or_else(|| nested.new_custom_error(ValueError::Syntax))
                    })?;
                    parts.push(Part::Attribute(Box::new(value)));
                }
                Token::Function(name)
                    if name.eq_ignore_ascii_case(crate::style::conditional::FUNCTION) =>
                {
                    let expression = input.parse_nested_block(|nested| {
                        crate::style::conditional::Expression::parse(
                            nested,
                            depth + 1,
                            property_syntax,
                        )
                        .map_err(|error| nested.new_custom_error(ValueError::Style(error)))?
                        .ok_or_else(|| nested.new_custom_error(ValueError::Syntax))
                    })?;
                    parts.push(Part::Conditional(Box::new(expression)));
                }
                Token::Function(name) if Reference::from_name(&name).is_some() => {
                    let reference = Reference::from_name(&name).unwrap();
                    let (name, fallback) = input.parse_nested_block(|p| {
                        let name = p.expect_ident()?.to_string();
                        if !name.starts_with("--") || name.len() == 2 {
                            return Err(p.new_custom_error(ValueError::Syntax));
                        }
                        let fallback = if p.try_parse(Parser::expect_comma).is_ok() {
                            let source = crate::style::parser::substitution_argument(
                                p,
                                reference == Reference::Local,
                            )
                            .map_err(ParseError::into)?;
                            let mut fallback = ParserInput::new(source);
                            Some(Self::tokens(
                                &mut Parser::new(&mut fallback),
                                depth + 1,
                                false,
                                property_syntax,
                            )?)
                        } else {
                            None
                        };
                        p.expect_exhausted()?;
                        Ok((name, fallback))
                    })?;
                    parts.push(Part::Reference(reference, name, fallback));
                }
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    let close = match token {
                        Token::SquareBracketBlock => "]",
                        Token::CurlyBracketBlock => "}",
                        _ => ")",
                    };
                    let open = input.slice_from(start).to_owned();
                    let guarded = property_syntax
                        && matches!(&token,Token::Function(name) if name.eq_ignore_ascii_case(crate::style::declaration::TOGGLE));
                    let value = input.parse_nested_block(|nested| {
                        if guarded {
                            let state = nested.state();
                            let valid = crate::style::declaration::toggle_argument(nested, depth)
                                .map_err(|error| {
                                nested.new_custom_error(ValueError::Style(error))
                            })?;
                            nested.reset(&state);
                            if !valid {
                                crate::style::conditions::block_source(nested);
                                return Ok(None);
                            }
                        }
                        Self::tokens(nested, depth + 1, true, property_syntax).map(Some)
                    })?;
                    if let Some(value) = value {
                        parts.push(Part::Block(open, kind, value, close));
                    } else {
                        parts.push(Part::Opaque(input.slice_from(start).to_owned(), kind));
                    }
                }
                Token::BadString(_)
                | Token::BadUrl(_)
                | Token::CloseParenthesis
                | Token::CloseSquareBracket
                | Token::CloseCurlyBracket => {
                    return Err(input.new_custom_error(ValueError::Syntax));
                }
                Token::Semicolon | Token::Delim('!') if !in_block => {
                    return Err(input.new_custom_error(ValueError::Syntax));
                }
                _ => parts.push(Part::Text(input.slice_from(start).to_owned(), kind)),
            }
        }
        Ok(Self(parts))
    }
    pub(super) fn has_substitutions(&self) -> bool {
        self.0.iter().any(|part| match part {
            Part::Reference(..) | Part::Conditional(_) | Part::Attribute(_) => true,
            Part::Block(_, _, value, _) => value.has_substitutions(),
            _ => false,
        })
    }
    pub(super) fn requires_context(&self) -> bool {
        self.0.iter().any(|part| match part {
            Part::Conditional(_) | Part::Attribute(_) => true,
            Part::Block(_, _, value, _) => value.requires_context(),
            Part::Reference(_, _, fallback) => {
                fallback.as_ref().is_some_and(Self::requires_context)
            }
            _ => false,
        })
    }
    pub(super) fn dependencies(&self, output: &mut HashSet<String>) {
        for part in &self.0 {
            match part {
                Part::Reference(reference, name, fallback) => {
                    if *reference == Reference::Local {
                        output.insert(name.clone());
                    }
                    if let Some(value) = fallback {
                        value.dependencies(output);
                    }
                }
                Part::Block(_, _, value, _) => value.dependencies(output),
                Part::Conditional(expression) => expression.dependencies(output),
                Part::Attribute(attribute) => attribute.dependencies(output),
                _ => {}
            }
        }
    }
    pub(super) fn substitute(
        &self,
        environment: &Variables,
        inherited: &Variables,
    ) -> Result<Option<Expanded>, StyleError> {
        self.substitute_with(&mut |reference, name| {
            let source = if reference == Reference::Inherited {
                inherited
            } else {
                environment
            };
            Ok(source.get(name).and_then(Clone::clone))
        })
    }
    pub(super) fn substitute_with(
        &self,
        lookup: &mut impl FnMut(Reference, &str) -> Result<Option<Expanded>, StyleError>,
    ) -> Result<Option<Expanded>, StyleError> {
        self.substitute_context(
            lookup,
            &mut |_| {
                Err(StyleError::Invalid(
                    "conditional substitution requires a computed property context".into(),
                ))
            },
            &mut |_| {
                Err(StyleError::Invalid(
                    "attribute substitution requires an element context".into(),
                ))
            },
        )
    }
    pub(super) fn substitute_context(
        &self,
        lookup: &mut impl FnMut(Reference, &str) -> Result<Option<Expanded>, StyleError>,
        test: &mut impl FnMut(
            &crate::style::conditional::Test,
        ) -> Result<crate::style::conditions::Truth, StyleError>,
        attribute: &mut impl FnMut(
            &crate::style::attribute::Attribute,
        ) -> Result<Option<Expanded>, StyleError>,
    ) -> Result<Option<Expanded>, StyleError> {
        let mut output = Output::default();
        if self.append(lookup, test, attribute, &mut output)? {
            Ok(Some(Expanded {
                source: output.value.into(),
                taint: if output.taint.is_empty() {
                    None
                } else {
                    Some(output.taint.into())
                },
            }))
        } else {
            Ok(None)
        }
    }
    fn append(
        &self,
        lookup: &mut impl FnMut(Reference, &str) -> Result<Option<Expanded>, StyleError>,
        test: &mut impl FnMut(
            &crate::style::conditional::Test,
        ) -> Result<crate::style::conditions::Truth, StyleError>,
        attribute: &mut impl FnMut(
            &crate::style::attribute::Attribute,
        ) -> Result<Option<Expanded>, StyleError>,
        output: &mut Output,
    ) -> Result<bool, StyleError> {
        for part in &self.0 {
            match part {
                Part::Text(text, kind) => output.token(text, *kind)?,
                Part::Opaque(text, kind) => {
                    output.token(text, *kind)?;
                    output.last = TokenSerializationType::Other;
                }

                Part::Attribute(value) => {
                    let Some(value) = attribute(value)? else {
                        return Ok(false);
                    };
                    output.source(&value)?;
                }

                Part::Block(open, kind, value, close) => {
                    output.token(open, *kind)?;
                    let function = *kind == TokenSerializationType::Function;
                    if function {
                        output.functions.push(output.value.len() - open.len());
                    }
                    let valid = value.append(lookup, test, attribute, output)?;
                    if function {
                        output.functions.pop();
                    }
                    if !valid {
                        return Ok(false);
                    }
                    output.token(close, TokenSerializationType::Other)?;
                }
                Part::Conditional(expression) => {
                    if let Some(value) = expression.select(test)?
                        && !value.append(lookup, test, attribute, output)?
                    {
                        return Ok(false);
                    }
                }
                Part::Reference(reference, name, fallback) => {
                    let value = if let Some(value) = lookup(*reference, name)? {
                        Some(value)
                    } else if let Some(fallback) = fallback {
                        fallback.substitute_context(lookup, test, attribute)?
                    } else {
                        None
                    };
                    let Some(value) = value else {
                        return Ok(false);
                    };
                    let value = if value.has_taint() {
                        value.attribute_tainted()
                    } else {
                        value
                    };
                    output.source(&value)?;
                }
            }
        }
        Ok(true)
    }
}
#[derive(Default)]
struct Output {
    value: String,
    taint: Vec<Origin>,
    functions: Vec<usize>,
    last: TokenSerializationType,
}
impl Output {
    fn token(&mut self, source: &str, kind: TokenSerializationType) -> Result<(), StyleError> {
        if self.last.needs_separator_when_before(kind) {
            self.append("/**/")?;
        }
        self.append(source)?;
        self.last = kind;
        Ok(())
    }
    fn source(&mut self, value: &Expanded) -> Result<(), StyleError> {
        let source = value.as_str();
        let mut raw = ParserInput::new(source);
        let input = &mut Parser::new(&mut raw);
        let mut first = TokenSerializationType::Nothing;
        let mut last = TokenSerializationType::Nothing;
        while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
            let kind = if matches!(token, Token::Comment(_)) {
                TokenSerializationType::WhiteSpace
            } else {
                token.serialization_type()
            };
            first.set_if_nothing(kind);
            last = kind;
            if matches!(
                token,
                Token::Function(_)
                    | Token::ParenthesisBlock
                    | Token::SquareBracketBlock
                    | Token::CurlyBracketBlock
            ) {
                input
                    .parse_nested_block(crate::style::parser::consume)
                    .map_err(crate::style::parser::failure)?;
                last = TokenSerializationType::Other;
            }
        }
        if first == TokenSerializationType::Nothing {
            if value.has_taint() {
                self.origin(Origin::Empty {
                    position: self.value.len(),
                    function: self.functions.last().copied(),
                });
            }
            return Ok(());
        }
        if self.last.needs_separator_when_before(first) {
            self.append("/**/")?;
        }
        let offset = self.value.len();
        self.append(source)?;
        if let Some(taint) = &value.taint {
            for origin in taint.iter() {
                self.origin(match origin {
                    Origin::Values(range) => {
                        Origin::Values(offset + range.start..offset + range.end)
                    }
                    Origin::Empty { position, function } => Origin::Empty {
                        position: offset + position,
                        function: function.map(|start| offset + start),
                    },
                });
            }
        }
        self.last = last;
        Ok(())
    }
    fn origin(&mut self, origin: Origin) {
        if let Some(last) = self.taint.last_mut() {
            if *last == origin {
                return;
            }
            if let (Origin::Values(previous), Origin::Values(next)) = (&mut *last, &origin)
                && previous.end >= next.start
            {
                previous.end = previous.end.max(next.end);
                return;
            }
        }
        self.taint.push(origin);
    }
    fn append(&mut self, source: &str) -> Result<(), StyleError> {
        std::fmt::Write::write_str(self, source).map_err(Self::byte_limit)
    }
    fn byte_limit(_: std::fmt::Error) -> StyleError {
        StyleError::VariableLimit {
            resource: "expanded value bytes",
            limit: MAX_VALUE_BYTES,
        }
    }
}
impl std::fmt::Write for Output {
    fn write_str(&mut self, source: &str) -> std::fmt::Result {
        if self
            .value
            .len()
            .checked_add(source.len())
            .is_none_or(|len| len > MAX_VALUE_BYTES)
        {
            return Err(std::fmt::Error);
        }
        self.value.push_str(source);
        Ok(())
    }
}

pub(super) fn dependencies_in(
    source: &str,
    depth: usize,
    output: &mut HashSet<String>,
) -> Result<(), StyleError> {
    fn scan(
        input: &mut Parser<'_, '_>,
        depth: usize,
        output: &mut HashSet<String>,
    ) -> Result<(), StyleError> {
        if depth > MAX_VALUE_DEPTH {
            return Err(StyleError::VariableLimit {
                resource: "value nesting",
                limit: MAX_VALUE_DEPTH,
            });
        }
        while let Ok(token) = input.next().cloned() {
            if let token @ (Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock) = token
            {
                let local = matches!(&token, Token::Function(name) if Reference::from_name(name) == Some(Reference::Local));
                input
                    .parse_nested_block(|nested| {
                        let state = nested.state();
                        if local
                            && let Ok(name) = nested.expect_ident_cloned()
                            && name.starts_with("--")
                            && name.len() > 2
                        {
                            output.insert(name.to_string());
                        }
                        nested.reset(&state);
                        scan(nested, depth + 1, output)
                            .map_err(|error| nested.new_custom_error(error))
                    })
                    .map_err(crate::style::parser::failure)?;
            }
        }
        Ok(())
    }
    let mut input = ParserInput::new(source);
    scan(&mut Parser::new(&mut input), depth, output)
}

pub(super) enum Resolution {
    Complete(Environment),
    Wide { name: String, keyword: WideKeyword },
}

pub(super) fn resolve(
    parent: &Environment,
    declarations: HashMap<String, String>,
) -> Result<Resolution, StyleError> {
    if declarations.is_empty() {
        return Ok(Resolution::Complete(parent.clone()));
    }
    let resolver = Resolver::new(parent, declarations)?;
    let evaluation = ReferencesOnly(&resolver);
    for name in resolver.names() {
        if let Some((name, keyword)) = resolver.compute(&name, &evaluation)? {
            return Ok(Resolution::Wide { name, keyword });
        }
    }
    Ok(Resolution::Complete(resolver.environment(parent)?))
}

mod cycles;
pub(super) type Values = HashMap<String, Option<Rc<Value>>>;
pub(super) enum Recascade {
    Restart,
    Source(Option<String>),
}
pub(super) trait Evaluation {
    fn enter(&self, name: &str) -> Result<(), StyleError>;
    fn leave(&self, name: &str);
    fn prepare_references(
        &self,
        value: &Value,
    ) -> Result<Option<(String, WideKeyword)>, StyleError>;
    fn substitute(&self, name: &str, value: &Value) -> Result<Option<Expanded>, StyleError>;
    fn recascade(&self, name: &str, keyword: WideKeyword) -> Result<Recascade, StyleError>;
}

pub(super) struct Resolver {
    values: RefCell<Values>,
    computed: RefCell<Variables>,
    finished: RefCell<HashSet<String>>,
    depth: Cell<usize>,
}
impl Resolver {
    pub(super) fn new(
        parent: &Environment,
        declarations: HashMap<String, String>,
    ) -> Result<Self, StyleError> {
        let mut computed = Variables {
            parent: Some(parent.clone()),
            bytes: parent.bytes,
            entries: parent.entries,
            values: HashMap::new(),
        };
        let mut values = Values::new();
        for (name, source) in declarations {
            let wide = WideKeyword::parse(&source);
            if matches!(
                wide,
                Some(
                    WideKeyword::Inherit
                        | WideKeyword::Unset
                        | WideKeyword::Revert
                        | WideKeyword::RevertLayer
                )
            ) {
                continue;
            }
            computed.entries += 1;
            if computed.entries > MAX_ENVIRONMENT_ENTRIES {
                return Err(StyleError::VariableLimit {
                    resource: "environment entries",
                    limit: MAX_ENVIRONMENT_ENTRIES,
                });
            }
            computed.charge_bytes(name.len())?;
            computed.values.insert(name.clone(), None);
            if !matches!(wide, Some(WideKeyword::Initial)) {
                values.insert(name, Value::parse(&source)?.map(Rc::new));
            }
        }
        let finished = cycles::members(&values);
        Ok(Self {
            values: RefCell::new(values),
            computed: RefCell::new(computed),
            finished: RefCell::new(finished),
            depth: Cell::new(0),
        })
    }

    pub(super) fn names(&self) -> Vec<String> {
        self.values.borrow().keys().cloned().collect()
    }

    #[expect(
        clippy::option_option,
        reason = "outer is whether the name is declared locally, inner is its value or guaranteed-invalid"
    )]
    pub(super) fn local(&self, name: &str) -> Option<Option<Expanded>> {
        self.computed.borrow().values.get(name).cloned()
    }

    pub(super) fn environment(&self, parent: &Environment) -> Result<Environment, StyleError> {
        let values = self.computed.borrow().values.clone();
        if values.is_empty() {
            return Ok(parent.clone());
        }
        let entries = parent.entries + values.len();
        if entries > MAX_ENVIRONMENT_ENTRIES {
            return Err(StyleError::VariableLimit {
                resource: "environment entries",
                limit: MAX_ENVIRONMENT_ENTRIES,
            });
        }
        let mut computed = Variables {
            parent: Some(parent.clone()),
            values,
            entries,
            bytes: parent.bytes,
        };
        let bytes = computed
            .values
            .iter()
            .map(|(name, value)| name.len() + value.as_ref().map_or(0, Expanded::storage_len))
            .sum();
        computed.charge_bytes(bytes)?;
        Ok(Arc::new(computed))
    }

    pub(super) fn compute(
        &self,
        name: &str,
        evaluation: &impl Evaluation,
    ) -> Result<Option<(String, WideKeyword)>, StyleError> {
        if self.finished.borrow().contains(name) || !self.values.borrow().contains_key(name) {
            return Ok(None);
        }
        let depth = self.depth.get();
        if depth >= MAX_DEPENDENCY_DEPTH {
            return Err(StyleError::VariableLimit {
                resource: "dependency depth",
                limit: MAX_DEPENDENCY_DEPTH,
            });
        }
        evaluation.enter(name)?;
        self.depth.set(depth + 1);
        let result = self.compute_value(name, evaluation);
        self.depth.set(depth);
        evaluation.leave(name);
        result
    }

    fn compute_value(
        &self,
        name: &str,
        evaluation: &impl Evaluation,
    ) -> Result<Option<(String, WideKeyword)>, StyleError> {
        loop {
            if self.finished.borrow().contains(name) {
                return Ok(None);
            }
            let value = self.values.borrow().get(name).cloned().flatten();
            if let Some(value) = &value
                && let Some(wide) = evaluation.prepare_references(value)?
            {
                return Ok(Some(wide));
            }
            if self.finished.borrow().contains(name) {
                return Ok(None);
            }
            let result: Option<Expanded> = match value {
                Some(value) => evaluation.substitute(name, &value)?,
                None => None,
            };
            if let Some(value) = &result
                && let Some(keyword) = WideKeyword::parse(value.as_str())
            {
                match evaluation.recascade(name, keyword)? {
                    Recascade::Restart => return Ok(Some((name.to_owned(), keyword))),
                    Recascade::Source(source) => {
                        self.replace(name, source.as_deref())?;
                        continue;
                    }
                }
            }
            if self.finished.borrow().contains(name) {
                return Ok(None);
            }
            if let Some(value) = &result {
                self.computed
                    .borrow_mut()
                    .charge_bytes(value.storage_len())?;
            }
            self.computed
                .borrow_mut()
                .values
                .insert(name.to_owned(), result);
            self.finished.borrow_mut().insert(name.to_owned());
            return Ok(None);
        }
    }

    fn replace(&self, name: &str, source: Option<&str>) -> Result<(), StyleError> {
        let keyword = source.and_then(WideKeyword::parse);
        if source.is_none() || matches!(keyword, Some(WideKeyword::Inherit | WideKeyword::Unset)) {
            self.values.borrow_mut().remove(name);
            let mut computed = self.computed.borrow_mut();
            if let Some(value) = computed.values.remove(name) {
                computed.entries -= 1;
                computed.bytes -= name.len() + value.map_or(0, |value| value.storage_len());
            }
            self.finished.borrow_mut().insert(name.to_owned());
            return Ok(());
        }
        if keyword == Some(WideKeyword::Initial) {
            self.values.borrow_mut().remove(name);
            self.computed
                .borrow_mut()
                .values
                .insert(name.to_owned(), None);
            self.finished.borrow_mut().insert(name.to_owned());
            return Ok(());
        }
        let value = Value::parse(source.expect("noninitial custom source exists"))?.map(Rc::new);
        self.values.borrow_mut().insert(name.to_owned(), value);
        let cyclic = cycles::members(&self.values.borrow());
        let mut computed = self.computed.borrow_mut();
        for name in &cyclic {
            if let Some(Some(value)) = computed.values.insert(name.clone(), None) {
                computed.bytes -= value.storage_len();
            }
        }
        self.finished.borrow_mut().extend(cyclic);
        Ok(())
    }
}

struct ReferencesOnly<'a>(&'a Resolver);
impl Evaluation for ReferencesOnly<'_> {
    fn enter(&self, _name: &str) -> Result<(), StyleError> {
        Ok(())
    }
    fn leave(&self, _name: &str) {}
    fn prepare_references(
        &self,
        value: &Value,
    ) -> Result<Option<(String, WideKeyword)>, StyleError> {
        let mut dependencies = HashSet::new();
        value.dependencies(&mut dependencies);
        for name in dependencies {
            if let Some(wide) = self.0.compute(&name, self)? {
                return Ok(Some(wide));
            }
        }
        Ok(None)
    }
    fn substitute(&self, _name: &str, value: &Value) -> Result<Option<Expanded>, StyleError> {
        value.substitute_with(&mut |reference, name| {
            let computed = self.0.computed.borrow();
            let source = if reference == Reference::Inherited {
                computed
                    .parent
                    .as_deref()
                    .expect("custom properties have an inherited environment")
            } else {
                &computed
            };
            Ok(source.get(name).and_then(Clone::clone))
        })
    }
    fn recascade(&self, _name: &str, _keyword: WideKeyword) -> Result<Recascade, StyleError> {
        Ok(Recascade::Restart)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Style, Stylesheet, StylesheetSource};
    use ferri_pdf_dom::{DocumentTree, Selector};

    fn attribute_environment(source: &str) -> Environment {
        let sheet = Stylesheet::parse(&[StylesheetSource {
            text: format!("p{{--value:{source}}}"),
            base: Default::default(),
        }])
        .unwrap();
        let tree = DocumentTree::parse("<p data-empty='' data-token='t'>Attribute</p>").unwrap();
        let element = tree.select(&Selector::parse("p").unwrap()).next().unwrap();
        sheet
            .computed(element, &Style::default())
            .unwrap()
            .custom_properties
    }

    #[test]
    fn repeated_empty_attributes_share_one_origin_at_the_same_position() {
        const REPETITIONS: usize = 256;
        let environment = attribute_environment(&"attr(data-empty *)".repeat(REPETITIONS));
        let value = environment.get("--value").unwrap().as_ref().unwrap();
        assert_eq!(value.as_str(), "");
        assert_eq!(
            value.taint.as_deref(),
            Some(
                [Origin::Empty {
                    position: 0,
                    function: None
                }]
                .as_slice()
            ),
            "repeated zero-byte substitutions must not grow retained provenance at one position"
        );
    }

    #[test]
    fn attribute_environment_budget_includes_retained_origins() {
        const REPETITIONS: usize = 8;
        let environment = attribute_environment(&"attr(data-token *) x ".repeat(REPETITIONS));
        let value = environment.get("--value").unwrap().as_ref().unwrap();
        let origins = value.taint.as_deref().unwrap();
        assert_eq!(
            origins.len(),
            REPETITIONS,
            "authored tokens between attributes keep their distinct provenance"
        );
        let retained = "--value".len() + value.len() + size_of_val(origins);
        assert_eq!(
            environment.bytes,
            retained,
            "the shared environment quota must include retained attribute metadata: source={} origins={}",
            value.len(),
            origins.len()
        );
    }

    #[test]
    fn inherited_attribute_metadata_reaches_the_environment_quota_before_text() {
        const ORIGINS_PER_ELEMENT: usize = 32 * 1024;
        const ELEMENTS: usize =
            MAX_ENVIRONMENT_BYTES / (ORIGINS_PER_ELEMENT * size_of::<Origin>()) + 1;
        let source = "var(--token) x ".repeat(ORIGINS_PER_ELEMENT);
        let sheet = Stylesheet::parse(&[StylesheetSource {
            text: format!("div{{--token:attr(data-token *);--value:{source}}}"),
            base: Default::default(),
        }])
        .unwrap();
        let html = "<div data-token=t>".repeat(ELEMENTS) + &"</div>".repeat(ELEMENTS);
        let tree = DocumentTree::parse(&html).unwrap();
        let selector = Selector::parse("div").unwrap();
        let mut parent = Style::default();
        let mut text_bytes = 0;
        for (index, element) in tree.select(&selector).enumerate() {
            match sheet.computed(element, &parent) {
                Ok(style) => {
                    let value = style
                        .custom_properties
                        .get("--value")
                        .unwrap()
                        .as_ref()
                        .unwrap();
                    assert_eq!(value.taint.as_deref().unwrap().len(), ORIGINS_PER_ELEMENT);
                    text_bytes += value.len() + "t".len() + "--token".len() + "--value".len();
                    parent = style;
                }
                Err(StyleError::VariableLimit {
                    resource: "environment bytes",
                    limit: MAX_ENVIRONMENT_BYTES,
                }) => {
                    assert!(
                        text_bytes < MAX_ENVIRONMENT_BYTES,
                        "provenance must trigger the shared quota while text still fits: element={index}, text_bytes={text_bytes}"
                    );
                    return;
                }
                other => panic!(
                    "unexpected result before provenance quota at element {index}: {other:?}"
                ),
            }
        }
        panic!(
            "retained attribute provenance bypassed the environment quota across {ELEMENTS} inherited environments"
        );
    }

    #[test]
    fn literal_attribute_serialization_bounds_escaped_output_including_quotes() {
        let value = Expanded::attribute_string(&"x".repeat(MAX_VALUE_BYTES - 2)).unwrap();
        assert_eq!(value.len(), MAX_VALUE_BYTES);
        let error = Expanded::attribute_string(&"\\".repeat(MAX_VALUE_BYTES / 2)).unwrap_err();
        assert!(
            matches!(
                error,
                StyleError::VariableLimit {
                    resource: "expanded value bytes",
                    limit: MAX_VALUE_BYTES
                }
            ),
            "CSS escaping must use the same bounded output writer: {error:?}"
        );
    }

    #[test]
    fn attribute_url_roles_distinguish_image_sources_from_metadata_and_fallback_color() {
        use crate::style::{declaration, grammar::Property};
        for (clean, tainted) in [
            ("image-set('t' 1x)", "image-set(attr(data-token) 1x)"),
            ("image('t')", "image(attr(data-token))"),
        ] {
            for (source, valid) in [(clean, true), (tainted, false)] {
                let environment = attribute_environment(source);
                let value = environment.get("--value").unwrap().as_ref().unwrap();
                assert_eq!(
                    declaration::expand_expanded(Property::BackgroundImage, value)
                        .unwrap()
                        .is_some(),
                    valid,
                    "semantic URL-role validation must distinguish clean and tainted image sources before painting capability checks: {source}"
                );
            }
        }
        for source in [
            "image-set('safe.png' type(attr(data-token)) 1x)",
            "image('safe.png',attr(missing <color>,red))",
        ] {
            let environment = attribute_environment(source);
            let value = environment.get("--value").unwrap().as_ref().unwrap();
            assert!(value.has_taint());
            assert!(
                declaration::expand_expanded(Property::BackgroundImage, value)
                    .unwrap()
                    .is_some(),
                "tainted non-URL operands must not contaminate an independent authored URL: {source}"
            );
        }
    }
}
