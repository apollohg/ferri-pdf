use crate::selector::Namespaces;
use crate::{
    SourceLocation, StyleDiagnostic, StyleDiagnosticKind, StyleError, selector::RuleSelector,
};
use crate::{conditions::block_source, declaration::CUSTOM_PROPERTY_PREFIX};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};
use ferri_pdf_resources::ResourceBase;
use std::{cell::RefCell, ops::Range, sync::Arc};

#[derive(Clone, Debug)]
pub(super) struct Declaration {
    pub(super) name: String,
    pub(super) value: String,
    pub(super) important: bool,
    pub(super) location: SourceLocation,
    pub(super) namespaces: Option<Namespaces>,
}
#[derive(Clone, Copy)]
pub(super) struct RuleContext {
    nested: bool,
    mode: crate::selector::SelectorMode,
}
impl RuleContext {
    const ROOT: Self = Self {
        nested: false,
        mode: crate::selector::SelectorMode::Normal,
    };
    const STYLE: Self = Self {
        nested: true,
        mode: crate::selector::SelectorMode::Nesting,
    };
    fn scoped(self) -> Self {
        Self {
            mode: crate::selector::SelectorMode::Scope,
            ..self
        }
    }
    fn allows_statement(self, prelude: &AtPrelude) -> bool {
        (!self.nested || matches!(prelude, AtPrelude::Layer(_)))
            && !(self.mode == crate::selector::SelectorMode::Scope
                && matches!(prelude, AtPrelude::Import(_)))
    }
}
pub(super) enum Rule {
    Namespace,
    Content(ContentRule),
    Style(NestedRule),
    Scope(crate::scope::ScopePrelude, RuleSource),
    Declarations(Vec<Declaration>, bool),
    LayerStatement(Vec<crate::layers::LayerName>),
    LayerBlock(Option<crate::layers::LayerName>, RuleSource),
    Conditional(crate::conditions::RuleQuery, bool, RuleSource),
    SupportDefinition(String, RuleSource),
    CustomMedia(String, crate::media::MediaQuery),
    Import(ImportRule, SourceLocation),
}
pub(super) struct NestedRule {
    pub selectors: Vec<RuleSelector>,
    pub source: RuleSource,
    pub nesting: bool,
}
impl NestedRule {
    pub fn work(&self, parent: &[RuleSelector]) -> usize {
        crate::selector::selector_work(&self.selectors, if self.nesting { parent } else { &[] })
    }
    pub fn location(&self) -> SourceLocation {
        self.source.location.clone()
    }
}
pub(super) struct ImportRule {
    pub reference: String,
    pub layer: Option<Option<crate::layers::LayerName>>,
    pub support: Option<crate::supports::SupportsCondition>,
    pub media: crate::media::MediaQuery,
}
pub(super) enum ContentRule {
    Style(Vec<RuleSelector>, Vec<Declaration>),
    Page {
        selectors: crate::page_rules::PageSelectors,
        items: Vec<crate::page_parser::PageItem>,
    },
    Font(Vec<Declaration>, SourceLocation),
}

#[derive(Clone)]
pub(super) struct RuleSource {
    text: Arc<str>,
    range: Range<usize>,
    location: SourceLocation,
    depth: usize,
    namespaces: Namespaces,
    context: RuleContext,
    initial_declarations: bool,
}

fn relocate(location: &mut SourceLocation, start: &SourceLocation) {
    if location.line == 1 {
        location.column += start.column - 1;
    }
    location.line += start.line - 1;
}

impl RuleSource {
    pub fn location(&self) -> SourceLocation {
        self.location.clone()
    }
    pub fn supports(self, budget: &mut crate::loader::Budget) -> Result<bool, StyleError> {
        supports_block_with_namespaces(
            &self.text[self.range],
            &self.location,
            budget,
            &self.namespaces,
            self.context,
        )
    }
    pub fn parse(
        self,
        diagnostics: &RefCell<Vec<StyleDiagnostic>>,
        budget: &mut crate::loader::Budget,
    ) -> Result<Vec<Rule>, StyleError> {
        let before = diagnostics.borrow().len();
        let mut input = ParserInput::new(&self.text[self.range.clone()]);
        let result = rules_at_depth(&mut Parser::new(&mut input), &self, diagnostics, budget)
            .map_err(failure);
        for diagnostic in &mut diagnostics.borrow_mut()[before..] {
            relocate(&mut diagnostic.location, &self.location);
        }
        match result {
            Ok(mut rules) => {
                for rule in &mut rules {
                    match rule {
                        Rule::Content(ContentRule::Style(_, declarations)) => {
                            for declaration in declarations {
                                relocate(&mut declaration.location, &self.location);
                                declaration.namespaces = Some(self.namespaces.clone());
                            }
                        }
                        Rule::Declarations(declarations, _) => {
                            for declaration in declarations {
                                relocate(&mut declaration.location, &self.location);
                                declaration.namespaces = Some(self.namespaces.clone());
                            }
                        }
                        Rule::Style(rule) => relocate(&mut rule.source.location, &self.location),
                        Rule::Content(ContentRule::Font(declarations, location)) => {
                            relocate(location, &self.location);
                            for declaration in declarations {
                                relocate(&mut declaration.location, &self.location);
                                declaration.namespaces = Some(self.namespaces.clone());
                            }
                        }
                        Rule::Content(ContentRule::Page { items, .. }) => {
                            for item in items {
                                match item {
                                    crate::page_parser::PageItem::Declaration(declaration) => {
                                        relocate(&mut declaration.location, &self.location);
                                        declaration.namespaces = Some(self.namespaces.clone());
                                    }
                                    crate::page_parser::PageItem::Margin(_, declarations) => {
                                        for declaration in declarations {
                                            relocate(&mut declaration.location, &self.location);
                                            declaration.namespaces = Some(self.namespaces.clone());
                                        }
                                    }
                                }
                            }
                        }
                        Rule::LayerBlock(_, source)
                        | Rule::Scope(_, source)
                        | Rule::Conditional(_, _, source)
                        | Rule::SupportDefinition(_, source) => {
                            relocate(&mut source.location, &self.location)
                        }
                        Rule::Import(_, location) => relocate(location, &self.location),
                        Rule::Namespace | Rule::LayerStatement(_) | Rule::CustomMedia(..) => {}
                    }
                }
                Ok(rules)
            }
            Err(mut error) => {
                if let StyleError::StylesheetLimit { location, .. } = &mut error {
                    relocate(location, &self.location);
                }
                Err(error)
            }
        }
    }
}
struct CssParser<'a> {
    base: &'a ResourceBase,
    diagnostics: &'a RefCell<Vec<StyleDiagnostic>>,
    depth: usize,
    chain: Option<cssparser::SourcePosition>,
    text: Arc<str>,
    offset: usize,
    imports_allowed: bool,
    namespaces_allowed: bool,
    namespaces: Namespaces,
    context: RuleContext,
    budget: &'a mut crate::loader::Budget,
}

#[derive(Clone, Copy)]
pub(super) enum AtRuleName {
    Page,
    Font,
    Layer,
    Scope,
    Media,
    CustomMedia,
    Supports,
    SupportsCondition,
    When,
    Else,
    Import,
    Namespace,
}
impl AtRuleName {
    fn can_nest_in_style(self) -> bool {
        matches!(
            self,
            Self::Layer
                | Self::Scope
                | Self::Media
                | Self::Supports
                | Self::SupportsCondition
                | Self::When
                | Self::Else
        )
    }
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "page" => Some(Self::Page),
            "font-face" => Some(Self::Font),
            "layer" => Some(Self::Layer),
            "scope" => Some(Self::Scope),
            "media" => Some(Self::Media),
            "custom-media" => Some(Self::CustomMedia),
            "supports" => Some(Self::Supports),
            "supports-condition" => Some(Self::SupportsCondition),
            "when" => Some(Self::When),
            "else" => Some(Self::Else),
            "import" => Some(Self::Import),
            "namespace" => Some(Self::Namespace),
            _ => None,
        }
    }
}

enum AtPrelude {
    Page(crate::page_rules::PageSelectors),
    Font,
    Layer(Vec<crate::layers::LayerName>),
    Scope(crate::scope::ScopePrelude),
    Conditional(crate::conditions::RuleQuery, bool),
    SupportDefinition(String),
    CustomMedia(String, crate::media::MediaQuery),
    Import(ImportRule),
    Namespace(String, String),
}

const MAX_GROUP_DEPTH: usize = 256;

pub(super) fn rules(
    text: Arc<str>,
    base: &ResourceBase,
    diagnostics: &RefCell<Vec<StyleDiagnostic>>,
    budget: &mut crate::loader::Budget,
) -> Result<Vec<Rule>, StyleError> {
    let range = 0..text.len();
    RuleSource {
        text,
        range,
        location: SourceLocation {
            base: base.clone(),
            line: 1,
            column: 1,
        },
        depth: 0,
        namespaces: Namespaces::default(),
        context: RuleContext::ROOT,
        initial_declarations: false,
    }
    .parse(diagnostics, budget)
}

fn rules_at_depth<'i>(
    input: &mut Parser<'i, '_>,
    source: &RuleSource,
    diagnostics: &RefCell<Vec<StyleDiagnostic>>,
    budget: &mut crate::loader::Budget,
) -> Result<Vec<Rule>, ParseError<'i, StyleError>> {
    let base = &source.location.base;
    let mut output = Vec::new();
    let mut parser = CssParser {
        base,
        diagnostics,
        depth: source.depth,
        chain: None,
        text: source.text.clone(),
        offset: source.range.start,
        imports_allowed: source.depth == 0 && !source.context.nested,
        namespaces_allowed: source.depth == 0 && !source.context.nested,
        namespaces: source.namespaces.clone(),
        context: source.context,
        budget,
    };
    if source.context.nested {
        collect_rules(
            RuleBodyParser::new(input, &mut parser),
            &mut output,
            source,
            diagnostics,
        )?;
    } else {
        collect_rules(
            StyleSheetParser::new(input, &mut parser),
            &mut output,
            source,
            diagnostics,
        )?;
    }
    Ok(output)
}
fn collect_rules<'i>(
    rules: impl Iterator<Item = Result<Rule, (ParseError<'i, StyleError>, &'i str)>>,
    output: &mut Vec<Rule>,
    context: &RuleSource,
    diagnostics: &RefCell<Vec<StyleDiagnostic>>,
) -> Result<(), ParseError<'i, StyleError>> {
    for rule in rules {
        match rule {
            Ok(Rule::Declarations(mut declarations, _)) => {
                if let Some(Rule::Declarations(previous, _)) = output.last_mut() {
                    previous.append(&mut declarations);
                } else {
                    let initial = context.initial_declarations && output.is_empty();
                    output.push(Rule::Declarations(declarations, initial));
                }
            }
            Ok(rule) => output.push(rule),
            Err((error, source)) => {
                let kind = if context.context.nested {
                    block_error_kind(source)
                } else {
                    StyleDiagnosticKind::InvalidRule
                };
                recover(error, source, &context.location.base, kind, diagnostics)?;
            }
        }
    }
    Ok(())
}
fn block_error_kind(source: &str) -> StyleDiagnosticKind {
    let mut input = ParserInput::new(source);
    let input = &mut Parser::new(&mut input);
    if matches!(
        input.next(),
        Ok(Token::AtKeyword(_) | Token::CurlyBracketBlock)
    ) {
        return StyleDiagnosticKind::InvalidRule;
    }
    while let Ok(token) = input.next() {
        if matches!(token, Token::CurlyBracketBlock) {
            return StyleDiagnosticKind::InvalidRule;
        }
    }
    StyleDiagnosticKind::InvalidDeclaration
}

fn layer_name<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<crate::layers::LayerName, ParseError<'i, StyleError>> {
    let first = input.expect_ident_cloned()?;
    let mut name = vec![first.to_string()];
    loop {
        let state = input.state();
        match input.next_including_whitespace().cloned() {
            Ok(Token::Delim('.')) => match input.next_including_whitespace()?.clone() {
                Token::Ident(segment) => name.push(segment.to_string()),
                token => return Err(input.new_unexpected_token_error(token)),
            },
            _ => {
                input.reset(&state);
                break;
            }
        }
    }
    if name
        .iter()
        .any(|segment| crate::wide_keywords::WideKeyword::parse_ident(segment).is_some())
    {
        return Err(input.new_custom_error(StyleError::Invalid(
            "CSS-wide keyword in a layer name".into(),
        )));
    }
    input.expect_exhausted()?;
    Ok(name)
}

pub(super) use cssparser::MAX_COMPONENT_DEPTH;

pub(super) fn balanced_components(source: &str) -> Result<bool, StyleError> {
    let mut pending = vec![(source, 0)];
    while let Some((source, depth)) = pending.pop() {
        if depth >= MAX_COMPONENT_DEPTH {
            return Err(StyleError::GrammarLimit {
                resource: "component nesting",
                limit: MAX_COMPONENT_DEPTH,
            });
        }
        let mut input = ParserInput::new(source);
        let input = &mut Parser::new(&mut input);
        while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
            match token {
                Token::BadUrl(_)
                | Token::BadString(_)
                | Token::CloseParenthesis
                | Token::CloseSquareBracket
                | Token::CloseCurlyBracket => return Ok(false),
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    let (source, end) = input
                        .parse_nested_block(|nested| {
                            let source = crate::conditions::block_source(nested);
                            Ok::<_, ParseError<'_, StyleError>>((
                                source,
                                nested.position().byte_index(),
                            ))
                        })
                        .map_err(failure)?;
                    if input.position().byte_index() == end {
                        return Ok(false);
                    }
                    pending.push((source, depth + 1));
                }
                _ => {}
            }
        }
    }
    Ok(true)
}

pub(super) fn consume<'i>(input: &mut Parser<'i, '_>) -> Result<(), ParseError<'i, StyleError>> {
    consume_nested(input, 0)
}

pub(super) fn trim_value(source: &str) -> &str {
    let mut raw = ParserInput::new(source);
    let input = &mut Parser::new(&mut raw);
    let mut first = None;
    let mut last = input.position();
    loop {
        let start = input.position();
        let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
            break;
        };
        if matches!(
            token,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        ) {
            let _ = input
                .parse_nested_block(|nested| Ok::<_, ParseError<'_, ()>>(block_source(nested)));
        }
        if !matches!(token, Token::WhiteSpace(_)) {
            first.get_or_insert(start);
            last = input.position();
        }
    }
    first.map_or("", |first| input.slice(first..last))
}

struct Argument<'i> {
    source: &'i str,
    nonempty: bool,
}

pub(super) fn declaration_argument<'i>(
    input: &mut Parser<'i, '_>,
    non_strict: bool,
) -> Result<Option<&'i str>, ParseError<'i, StyleError>> {
    let value = argument(input, non_strict)?;
    Ok(value.nonempty.then_some(value.source))
}

pub(super) fn substitution_argument<'i>(
    input: &mut Parser<'i, '_>,
    non_strict: bool,
) -> Result<&'i str, ParseError<'i, StyleError>> {
    Ok(argument(input, non_strict)?.source)
}

fn argument<'i>(
    input: &mut Parser<'i, '_>,
    non_strict: bool,
) -> Result<Argument<'i>, ParseError<'i, StyleError>> {
    if input.try_parse(Parser::expect_curly_bracket_block).is_ok() {
        let value = input.parse_nested_block(|input| argument_contents(input, true))?;
        input.expect_exhausted()?;
        Ok(value)
    } else {
        argument_contents(input, non_strict)
    }
}

fn argument_contents<'i>(
    input: &mut Parser<'i, '_>,
    allow_commas_and_braces: bool,
) -> Result<Argument<'i>, ParseError<'i, StyleError>> {
    let start = input.position();
    let mut nonempty = false;
    loop {
        let token = match input.next_including_whitespace_and_comments().cloned() {
            Ok(token) => token,
            Err(error) if matches!(error.kind, cssparser::BasicParseErrorKind::EndOfInput) => break,
            Err(error) => return Err(error.into()),
        };
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => continue,
            token @ (Token::Comma | Token::CurlyBracketBlock) if !allow_commas_and_braces => {
                return Err(input.new_unexpected_token_error(token));
            }
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => input.parse_nested_block(consume)?,
            token @ (Token::Delim('!')
            | Token::Semicolon
            | Token::BadUrl(_)
            | Token::BadString(_)
            | Token::CloseParenthesis
            | Token::CloseSquareBracket
            | Token::CloseCurlyBracket) => return Err(input.new_unexpected_token_error(token)),
            _ => {}
        }
        nonempty = true;
    }
    Ok(Argument {
        source: trim_value(input.slice_from(start)),
        nonempty,
    })
}

fn consume_nested<'i>(
    input: &mut Parser<'i, '_>,
    depth: usize,
) -> Result<(), ParseError<'i, StyleError>> {
    if depth >= MAX_COMPONENT_DEPTH {
        return Err(input.new_custom_error(StyleError::GrammarLimit {
            resource: "component nesting",
            limit: MAX_COMPONENT_DEPTH,
        }));
    }
    while !input.is_exhausted() {
        match input.next()?.clone() {
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                input.parse_nested_block(|nested| consume_nested(nested, depth + 1))?
            }
            token @ (Token::BadUrl(_)
            | Token::BadString(_)
            | Token::CloseParenthesis
            | Token::CloseSquareBracket
            | Token::CloseCurlyBracket) => return Err(input.new_unexpected_token_error(token)),
            _ => {}
        }
    }
    Ok(())
}

const URL_FUNCTION: &str = "url(";

enum UrlPart {
    Token,
    String,
}
struct UrlReference {
    range: Range<usize>,
    reference: String,
    part: UrlPart,
}

fn url_references<'i>(
    input: &mut Parser<'i, '_>,
    found: &mut Vec<UrlReference>,
) -> Result<(), ParseError<'i, String>> {
    loop {
        let start = input.position().byte_index();
        let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
            return Ok(());
        };
        match token {
            Token::UnquotedUrl(reference) => found.push(UrlReference {
                range: start..input.position().byte_index(),
                reference: reference.to_string(),
                part: UrlPart::Token,
            }),
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                input.parse_nested_block(|nested| {
                    let start = nested.position().byte_index();
                    let reference = nested.expect_string()?.to_string();
                    found.push(UrlReference {
                        range: start..nested.position().byte_index(),
                        reference,
                        part: UrlPart::String,
                    });
                    url_references(nested, found)
                })?;
            }
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                input.parse_nested_block(|nested| url_references(nested, found))?
            }
            _ => {}
        }
    }
}

/// Resolves relative `url()` references against the declaring stylesheet, as computed
/// values require; absolute and `data:` references are unchanged.
pub(super) fn absolute_urls(
    declaration: &mut Declaration,
    base: &ferri_pdf_resources::ResourceBase,
) -> Result<(), crate::StyleError> {
    if !declaration.value.contains('\\')
        && !declaration
            .value
            .to_ascii_lowercase()
            .contains(URL_FUNCTION)
    {
        return Ok(());
    }
    let mut found = Vec::new();
    let mut value = declaration.value.clone();
    {
        let mut input = ParserInput::new(&declaration.value);
        url_references(&mut Parser::new(&mut input), &mut found)
            .map_err(|e| format!("url() in {}: {e:?}", declaration.name))?;
    }
    for UrlReference {
        range,
        reference,
        part,
    } in found.into_iter().rev()
    {
        if reference.is_empty() || ferri_pdf_resources::Url::parse(&reference).is_ok() {
            continue;
        }
        let absolute = base.resolve(&reference)?;
        let mut serialized = String::new();
        cssparser::serialize_string(absolute.as_str(), &mut serialized)
            .expect("writing a CSS URL string cannot fail");
        if matches!(part, UrlPart::Token) {
            serialized = format!("url({serialized})");
        }
        value.replace_range(range, &serialized);
    }
    declaration.value = value;
    Ok(())
}

pub(super) fn raw<'i>(input: &mut Parser<'i, '_>) -> Result<String, ParseError<'i, StyleError>> {
    let start = input.position();
    consume(input)?;
    Ok(input.slice_from(start).trim().to_owned())
}

pub(super) struct Declarations<'a> {
    pub(super) base: &'a ResourceBase,
}
impl<'i> DeclarationParser<'i> for Declarations<'_> {
    type Declaration = Declaration;
    type Error = StyleError;
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        state: &ParserState,
    ) -> Result<Declaration, ParseError<'i, StyleError>> {
        if name == CUSTOM_PROPERTY_PREFIX {
            return Err(
                input.new_custom_error(StyleError::Invalid("reserved custom property name".into()))
            );
        }
        let start = input.position();
        let mut end = None;
        let mut components = 0;
        let mut contains_curly = false;
        loop {
            let state = input.state();
            let token = match input.next_including_whitespace_and_comments().cloned() {
                Ok(Token::WhiteSpace(_) | Token::Comment(_)) => continue,
                Ok(token) => token,
                Err(error) if matches!(error.kind, cssparser::BasicParseErrorKind::EndOfInput) => {
                    break;
                }
                Err(error) => return Err(error.into()),
            };
            if !matches!(token, Token::Delim('!')) {
                components += 1;
                contains_curly |= matches!(token, Token::CurlyBracketBlock);
            }
            match token {
                Token::Delim('!') => {
                    input.reset(&state);
                    cssparser::parse_important(input)?;
                    input.expect_exhausted()?;
                    end = Some(state.position());
                }
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => input.parse_nested_block(consume)?,
                token @ (Token::BadUrl(_)
                | Token::BadString(_)
                | Token::CloseParenthesis
                | Token::CloseSquareBracket
                | Token::CloseCurlyBracket) => return Err(input.new_unexpected_token_error(token)),
                _ => {}
            }
        }
        if !name.starts_with(CUSTOM_PROPERTY_PREFIX) && contains_curly && components != 1 {
            return Err(input.new_custom_error(StyleError::Invalid(
                "a curly block must be the whole non-custom property value".into(),
            )));
        }
        let value = trim_value(input.slice(start..end.unwrap_or(input.position()))).to_owned();
        Ok(Declaration {
            name: if name.starts_with(CUSTOM_PROPERTY_PREFIX) {
                name.to_string()
            } else {
                name.to_ascii_lowercase()
            },
            value,
            important: end.is_some(),
            location: SourceLocation::new(self.base, state.source_location()),
            namespaces: None,
        })
    }
}

pub(super) fn recover<'i>(
    error: ParseError<'i, StyleError>,
    source: &str,
    base: &ResourceBase,
    kind: StyleDiagnosticKind,
    diagnostics: &RefCell<Vec<StyleDiagnostic>>,
) -> Result<(), ParseError<'i, StyleError>> {
    if !matches!(
        &error.kind,
        cssparser::ParseErrorKind::Basic(_)
            | cssparser::ParseErrorKind::Custom(StyleError::Invalid(_))
    ) {
        return Err(error);
    }
    diagnostics.borrow_mut().push(StyleDiagnostic {
        location: SourceLocation::new(base, error.location),
        kind,
        context: format!("{source}: {:?}", error.kind),
    });
    Ok(())
}

pub(super) fn failure(error: ParseError<'_, StyleError>) -> StyleError {
    match error.kind {
        cssparser::ParseErrorKind::Custom(error) => error,
        _ => StyleError::Invalid(format!("{error:?}")),
    }
}

impl<'i> AtRuleParser<'i> for Declarations<'_> {
    type Prelude = ();
    type AtRule = Declaration;
    type Error = StyleError;
}
impl<'i> QualifiedRuleParser<'i> for Declarations<'_> {
    type Prelude = ();
    type QualifiedRule = Declaration;
    type Error = StyleError;
}
impl<'i> RuleBodyItemParser<'i, Declaration, StyleError> for Declarations<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

pub(super) fn declarations<'i>(
    input: &mut Parser<'i, '_>,
    base: &ResourceBase,
    diagnostics: &RefCell<Vec<StyleDiagnostic>>,
) -> Result<Vec<Declaration>, ParseError<'i, StyleError>> {
    let mut output = Vec::new();
    for item in RuleBodyParser::new(input, &mut Declarations { base }) {
        match item {
            Ok(declaration) => output.push(declaration),
            Err((error, source)) => recover(
                error,
                source,
                base,
                StyleDiagnosticKind::InvalidDeclaration,
                diagnostics,
            )?,
        }
    }
    Ok(output)
}
fn at_prelude<'i>(
    name: CowRcStr<'i>,
    input: &mut Parser<'i, '_>,
    chain: &mut Option<cssparser::SourcePosition>,
    namespaces: &Namespaces,
    mode: crate::selector::SelectorMode,
    forgiving: bool,
) -> Result<AtPrelude, ParseError<'i, StyleError>> {
    let previous = chain.take();
    let kind = AtRuleName::parse(&name).ok_or_else(|| {
        input.new_custom_error(StyleError::Invalid(format!("unknown at-rule: @{name}")))
    })?;
    if matches!(kind, AtRuleName::Scope) {
        return crate::scope::ScopePrelude::parse(input, &namespaces.borrow(), mode, forgiving)
            .map(AtPrelude::Scope);
    }
    if matches!(kind, AtRuleName::Import) {
        return import_prelude(input, namespaces).map(AtPrelude::Import);
    }
    if matches!(kind, AtRuleName::Namespace) {
        let (prefix, namespace) = ferri_pdf_dom::parse_namespace(input)?;
        return Ok(AtPrelude::Namespace(prefix, namespace));
    }
    if matches!(kind, AtRuleName::Else)
        && !previous.is_some_and(|end| follows_conditional(input.slice(end..input.position())))
    {
        return Err(input.new_custom_error(StyleError::Invalid("orphan @else".into())));
    }
    if matches!(
        kind,
        AtRuleName::Media | AtRuleName::Supports | AtRuleName::When | AtRuleName::Else
    ) {
        let empty = input.is_exhausted();
        let source = crate::conditions::block_source(input);
        let query = match kind {
            AtRuleName::Media => crate::conditions::RuleQuery::Media(
                crate::media::MediaQuery::parse(source)
                    .map_err(|error| input.new_custom_error(error))?,
            ),
            AtRuleName::Supports => crate::conditions::RuleQuery::Supports(
                crate::supports::SupportsCondition::parse_with_namespaces(source, namespaces)
                    .map_err(|error| input.new_custom_error(error))?
                    .ok_or_else(|| {
                        input.new_custom_error(StyleError::Invalid("supports condition".into()))
                    })?,
            ),
            _ if empty && matches!(kind, AtRuleName::Else) => {
                crate::conditions::RuleQuery::Constant(true)
            }
            _ => crate::conditions::RuleQuery::When(
                crate::conditions::WhenCondition::parse(source)
                    .map_err(|error| input.new_custom_error(error))?
                    .ok_or_else(|| {
                        input.new_custom_error(StyleError::Invalid("combined condition".into()))
                    })?,
            ),
        };
        return Ok(AtPrelude::Conditional(
            query,
            matches!(kind, AtRuleName::Else),
        ));
    }
    if matches!(kind, AtRuleName::SupportsCondition) {
        let name = input.expect_ident_cloned()?;
        if !name.starts_with(CUSTOM_PROPERTY_PREFIX) {
            return Err(input.new_custom_error(StyleError::Invalid(
                "supports condition requires an extension name".into(),
            )));
        }
        input.expect_exhausted()?;
        return Ok(AtPrelude::SupportDefinition(name.to_string()));
    }
    if matches!(kind, AtRuleName::CustomMedia) {
        let name = input.expect_ident_cloned()?;
        if !name.starts_with(CUSTOM_PROPERTY_PREFIX) {
            return Err(input.new_custom_error(StyleError::Invalid(
                "custom media requires an extension name".into(),
            )));
        }
        let start = input.state();
        let query = if let Ok(value) = input.try_parse(|input| input.expect_ident_cloned()) {
            if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
                input.expect_exhausted()?;
                crate::media::MediaQuery::constant(value.eq_ignore_ascii_case("true"))
            } else {
                input.reset(&start);
                crate::media::MediaQuery::parse(crate::conditions::block_source(input))
                    .map_err(|error| input.new_custom_error(error))?
            }
        } else {
            crate::media::MediaQuery::parse(crate::conditions::block_source(input))
                .map_err(|error| input.new_custom_error(error))?
        };
        return Ok(AtPrelude::CustomMedia(name.to_string(), query));
    }
    if matches!(kind, AtRuleName::Layer) {
        let names = if input.is_exhausted() {
            Vec::new()
        } else {
            input.parse_comma_separated(layer_name)?
        };
        return Ok(AtPrelude::Layer(names));
    }
    if matches!(kind, AtRuleName::Page) {
        return crate::page_parser::page_selectors(input).map(AtPrelude::Page);
    }
    input.expect_exhausted()?;
    Ok(AtPrelude::Font)
}

fn import_prelude<'i>(
    input: &mut Parser<'i, '_>,
    namespaces: &Namespaces,
) -> Result<ImportRule, ParseError<'i, StyleError>> {
    let reference = input.expect_url_or_string()?.to_string();
    let layer = if input
        .try_parse(|input| input.expect_ident_matching("layer"))
        .is_ok()
    {
        Some(None)
    } else if input
        .try_parse(|input| input.expect_function_matching("layer"))
        .is_ok()
    {
        Some(input.parse_nested_block(|nested| {
            if nested.is_exhausted() {
                Ok(None)
            } else {
                layer_name(nested).map(Some)
            }
        })?)
    } else {
        None
    };
    let support = if input
        .try_parse(|input| input.expect_function_matching("supports"))
        .is_ok()
    {
        Some(input.parse_nested_block(|nested| {
            let source = block_source(nested);
            let declaration = crate::supports::SupportTest::declaration(source);
            let wrapped;
            let source = if declaration.is_some() {
                wrapped = format!("({source})");
                wrapped.as_str()
            } else {
                source
            };
            crate::supports::SupportsCondition::parse_with_namespaces(source, namespaces)
                .map_err(|error| nested.new_custom_error(error))?
                .ok_or_else(|| {
                    nested.new_custom_error(StyleError::Invalid("import supports condition".into()))
                })
        })?)
    } else {
        None
    };
    let media = crate::media::MediaQuery::parse(block_source(input))
        .map_err(|error| input.new_custom_error(error))?;
    Ok(ImportRule {
        reference,
        layer,
        support,
        media,
    })
}

fn statement(prelude: AtPrelude, location: SourceLocation) -> Result<Rule, ()> {
    match prelude {
        AtPrelude::Layer(names) if !names.is_empty() => Ok(Rule::LayerStatement(names)),
        AtPrelude::CustomMedia(name, query) => Ok(Rule::CustomMedia(name, query)),
        AtPrelude::Import(import) => Ok(Rule::Import(import, location)),
        _ => Err(()),
    }
}

impl<'i> AtRuleParser<'i> for CssParser<'_> {
    type Prelude = AtPrelude;
    type AtRule = Rule;
    type Error = StyleError;
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<AtPrelude, ParseError<'i, StyleError>> {
        self.budget
            .rule(|| SourceLocation::new(self.base, input.current_source_location()))
            .map_err(|error| input.new_custom_error(error))?;
        if self.context.nested
            && AtRuleName::parse(&name).is_some_and(|kind| !kind.can_nest_in_style())
        {
            self.chain = None;
            return Err(input.new_custom_error(StyleError::Invalid(format!(
                "@{name} cannot be nested in a style rule"
            ))));
        }
        at_prelude(
            name,
            input,
            &mut self.chain,
            &self.namespaces,
            self.context.mode,
            true,
        )
    }
    fn rule_without_block(&mut self, prelude: AtPrelude, state: &ParserState) -> Result<Rule, ()> {
        if !self.context.allows_statement(&prelude) {
            return Err(());
        }
        if let AtPrelude::Namespace(prefix, namespace) = prelude {
            if !self.namespaces_allowed {
                return Err(());
            }
            self.namespaces.borrow_mut().insert(prefix, namespace);
            self.imports_allowed = false;
            return Ok(Rule::Namespace);
        }
        if matches!(prelude, AtPrelude::Import(_)) && !self.imports_allowed {
            return Err(());
        }
        let preserves = matches!(prelude, AtPrelude::Import(_) | AtPrelude::Layer(_));
        let rule = statement(
            prelude,
            SourceLocation::new(self.base, state.source_location()),
        )?;
        self.imports_allowed &= preserves;
        self.namespaces_allowed &= preserves;
        Ok(rule)
    }
    fn parse_block<'t>(
        &mut self,
        prelude: AtPrelude,
        state: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Rule, ParseError<'i, StyleError>> {
        let preserves = matches!(prelude, AtPrelude::SupportDefinition(_));
        let rule = match prelude {
            AtPrelude::Scope(scope) => {
                let mut source = self.group(input)?;
                source.context = self.context.scoped();
                Rule::Scope(scope, source)
            }
            AtPrelude::Page(selectors) => Rule::Content(ContentRule::Page {
                selectors,
                items: crate::page_parser::page_items(
                    input,
                    self.base,
                    self.diagnostics,
                    self.budget,
                )?,
            }),
            AtPrelude::Font => Rule::Content(ContentRule::Font(
                declarations(input, self.base, self.diagnostics)?,
                SourceLocation::new(self.base, state.source_location()),
            )),
            AtPrelude::Layer(mut names) => {
                if names.len() > 1 {
                    return Err(input.new_custom_error(StyleError::Invalid(
                        "multiple names in a layer block".into(),
                    )));
                }
                Rule::LayerBlock(names.pop(), self.group(input)?)
            }
            AtPrelude::Conditional(query, alternative) => {
                let source = self.group(input)?;
                self.chain = Some(input.position());
                Rule::Conditional(query, alternative, source)
            }
            AtPrelude::SupportDefinition(name) => {
                let source = self.group(input)?;
                self.chain = Some(input.position());
                Rule::SupportDefinition(name, source)
            }
            AtPrelude::Import(_) | AtPrelude::CustomMedia(..) | AtPrelude::Namespace(..) => {
                return Err(input.new_custom_error(StyleError::Invalid(
                    "statement at-rule cannot have a block".into(),
                )));
            }
        };
        self.imports_allowed &= preserves;
        self.namespaces_allowed &= preserves;
        Ok(rule)
    }
}

impl CssParser<'_> {
    fn group<'i>(
        &self,
        input: &mut Parser<'i, '_>,
    ) -> Result<RuleSource, ParseError<'i, StyleError>> {
        self.group_contents(input, true)
    }
    fn group_contents<'i>(
        &self,
        input: &mut Parser<'i, '_>,
        increase_depth: bool,
    ) -> Result<RuleSource, ParseError<'i, StyleError>> {
        if increase_depth && self.depth >= MAX_GROUP_DEPTH {
            return Err(input.new_custom_error(StyleError::GrammarLimit {
                resource: "stylesheet group nesting",
                limit: MAX_GROUP_DEPTH,
            }));
        }
        let location = SourceLocation::new(self.base, input.current_source_location());
        let start = self.offset + input.position().byte_index();
        block_source(input);
        let end = self.offset + input.position().byte_index();
        Ok(RuleSource {
            text: self.text.clone(),
            range: start..end,
            location,
            depth: self.depth + usize::from(increase_depth),
            namespaces: self.namespaces.clone(),
            context: self.context,
            initial_declarations: false,
        })
    }
}
impl<'i> QualifiedRuleParser<'i> for CssParser<'_> {
    type Prelude = Vec<RuleSelector>;
    type QualifiedRule = Rule;
    type Error = StyleError;
    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, StyleError>> {
        self.chain = None;
        self.budget
            .rule(|| SourceLocation::new(self.base, input.current_source_location()))
            .map_err(|error| input.new_custom_error(error))?;
        crate::selector::parse(input, &self.namespaces.borrow(), self.context.mode)
    }

    fn parse_block<'t>(
        &mut self,
        selectors: Self::Prelude,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Rule, ParseError<'i, StyleError>> {
        self.imports_allowed = false;
        self.namespaces_allowed = false;
        let mut source = self.group_contents(input, self.context.nested)?;
        source.context = RuleContext::STYLE;
        source.initial_declarations = true;
        Ok(Rule::Style(NestedRule {
            selectors,
            source,
            nesting: self.context.mode == crate::selector::SelectorMode::Nesting,
        }))
    }
}
impl<'i> DeclarationParser<'i> for CssParser<'_> {
    type Declaration = Rule;
    type Error = StyleError;
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        state: &ParserState,
    ) -> Result<Rule, ParseError<'i, StyleError>> {
        self.chain = None;
        Declarations { base: self.base }
            .parse_value(name, input, state)
            .map(|declaration| Rule::Declarations(vec![declaration], false))
    }
}
impl<'i> RuleBodyItemParser<'i, Rule, StyleError> for CssParser<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nesting_style_depth_has_a_typed_bound() {
        let depth = MAX_GROUP_DEPTH + 2;
        let source = format!("{}color:red{}", ".item{".repeat(depth), "}".repeat(depth));
        let result = crate::Stylesheet::parse(&[crate::StylesheetSource {
            text: source,
            base: Default::default(),
        }]);
        assert!(
            matches!(
                result,
                Err(StyleError::GrammarLimit {
                    resource: "stylesheet group nesting",
                    limit: MAX_GROUP_DEPTH
                })
            ),
            "nested style rules must be bounded before recursive selector preparation"
        );
    }
    #[test]
    fn nesting_resolved_selector_depth_is_bounded_before_ast_binding() {
        const FUNCTION_DEPTH: usize = 80;
        const STYLE_DEPTH: usize = 4;
        let selector = format!(
            "{}&{}",
            ":is(".repeat(FUNCTION_DEPTH),
            ")".repeat(FUNCTION_DEPTH)
        );
        let source = format!(
            ".item{{{}color:red{}}}",
            format!("{selector}{{").repeat(STYLE_DEPTH),
            "}".repeat(STYLE_DEPTH)
        );
        let result = crate::Stylesheet::parse(&[crate::StylesheetSource {
            text: source,
            base: Default::default(),
        }]);
        assert!(
            matches!(
                result,
                Err(StyleError::GrammarLimit {
                    resource: "resolved selector nesting",
                    ..
                })
            ),
            "bounded source groups/functions must not combine into an unbounded resolved selector stack"
        );
    }
    #[test]
    fn nesting_resolved_depth_limit_is_inclusive() {
        const PARENT_FUNCTION_DEPTH: usize = 128;
        for (child_depth, allowed) in [(127, true), (128, false)] {
            let parent = format!(
                "{}.item{}",
                ":is(".repeat(PARENT_FUNCTION_DEPTH),
                ")".repeat(PARENT_FUNCTION_DEPTH)
            );
            let child = format!("{}&{}", ":is(".repeat(child_depth), ")".repeat(child_depth));
            let result = crate::Stylesheet::parse(&[crate::StylesheetSource {
                text: format!("{parent}{{{child}{{color:red}}}}"),
                base: Default::default(),
            }]);
            if allowed {
                let sheet = result.expect("a selector at the inclusive resolved limit parses");
                let tree = ferri_pdf_dom::DocumentTree::parse("<!doctype html><html class=item>")
                    .expect("parse test HTML");
                let style = sheet
                    .computed(tree.root_element(), &crate::Style::default())
                    .expect("match a selector at the inclusive resolved limit");
                assert_eq!(
                    style.color.rgb,
                    [255, 0, 0],
                    "a resolved selector at the inclusive depth limit remains usable"
                );
            } else {
                assert!(
                    matches!(
                        result,
                        Err(StyleError::GrammarLimit {
                            resource: "resolved selector nesting",
                            ..
                        })
                    ),
                    "one additional resolved selector level is rejected before binding"
                );
            }
        }
    }
    #[test]
    fn nesting_source_selector_component_limit_is_safe_for_matching() {
        let depth = MAX_COMPONENT_DEPTH - 1;
        let selector = format!("{}.item{}", ":is(".repeat(depth), ")".repeat(depth));
        let sheet = crate::Stylesheet::parse(&[crate::StylesheetSource {
            text: format!("{selector}{{color:red}}"),
            base: Default::default(),
        }])
        .expect("the existing component-depth limit remains usable");
        let tree = ferri_pdf_dom::DocumentTree::parse("<!doctype html><html class=item>")
            .expect("parse test HTML");
        let style = sheet
            .computed(tree.root_element(), &crate::Style::default())
            .expect("match a selector at its component-depth limit");
        assert_eq!(
            style.color.rgb,
            [255, 0, 0],
            "deep selectors must parse and match safely on the normal library test thread"
        );
    }

    #[test]
    fn scope_nesting_uses_the_common_group_bound_and_matches_without_recursion() {
        for (depth, allowed) in [(MAX_GROUP_DEPTH, true), (MAX_GROUP_DEPTH + 1, false)] {
            let result = crate::Stylesheet::parse(&[crate::StylesheetSource {
                text: format!(
                    "{}p{{color:red}}{}",
                    "@scope (:scope){".repeat(depth),
                    "}".repeat(depth)
                ),
                base: Default::default(),
            }]);
            if allowed {
                let sheet = result.expect("scope groups at the inclusive bound parse");
                let tree = ferri_pdf_dom::DocumentTree::parse("<!doctype html><p>Scoped</p>")
                    .expect("parse test HTML");
                let target = tree
                    .elements()
                    .find(|element| element.name() == "p")
                    .unwrap();
                let style = sheet
                    .computed(target, &crate::Style::default())
                    .expect("match the deepest allowed scope chain");
                assert_eq!(
                    style.color.rgb,
                    [255, 0, 0],
                    "scope ancestry must not overflow or lose the innermost rule at the inclusive bound"
                );
            } else {
                assert!(
                    matches!(
                        result,
                        Err(StyleError::GrammarLimit {
                            resource: "stylesheet group nesting",
                            limit: MAX_GROUP_DEPTH
                        })
                    ),
                    "a further scope level uses the existing typed group bound"
                );
            }
        }
    }

    #[test]
    fn layer_group_nesting_has_a_typed_bound() {
        for (depth, allowed) in [(MAX_GROUP_DEPTH, true), (MAX_GROUP_DEPTH + 1, false)] {
            let source = format!(
                "{}p{{color:red}}{}",
                "@layer{".repeat(depth),
                "}".repeat(depth)
            );
            let result = crate::Stylesheet::parse(&[crate::StylesheetSource {
                text: source,
                base: Default::default(),
            }]);
            if allowed {
                assert!(result.is_ok());
            } else {
                assert!(matches!(
                    result,
                    Err(StyleError::GrammarLimit {
                        resource: "stylesheet group nesting",
                        limit: MAX_GROUP_DEPTH
                    })
                ));
            }
        }
    }
}

fn follows_conditional(source: &str) -> bool {
    let mut input = ParserInput::new(source);
    let input = &mut Parser::new(&mut input);
    matches!(input.next(), Ok(Token::CloseCurlyBracket))
        && matches!(input.next(), Ok(Token::AtKeyword(name)) if name.eq_ignore_ascii_case("else"))
        && input.is_exhausted()
}

#[derive(Clone, Copy)]
enum ProbeContext {
    Style(RuleContext, bool),
    Font,
    Page,
    Margin,
}
enum ProbeItem<'i> {
    Checked(bool),
    Body(ProbeContext, &'i str, SourceLocation),
}
struct SupportProbe<'a> {
    start: &'a SourceLocation,
    chain: Option<cssparser::SourcePosition>,
    context: ProbeContext,
    budget: &'a mut crate::loader::Budget,
    namespaces: &'a Namespaces,
}
impl SupportProbe<'_> {
    fn location(start: &SourceLocation, input: &Parser<'_, '_>) -> SourceLocation {
        let mut location = SourceLocation::new(&start.base, input.current_source_location());
        relocate(&mut location, start);
        location
    }
}
impl<'i> DeclarationParser<'i> for SupportProbe<'_> {
    type Declaration = ProbeItem<'i>;
    type Error = StyleError;
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        state: &ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, StyleError>> {
        let declaration = Declarations {
            base: &self.start.base,
        }
        .parse_value(name, input, state)?;
        let supported = match self.context {
            ProbeContext::Font => {
                crate::stylesheet::supports_descriptor("@font-face", &declaration)
            }
            ProbeContext::Page => crate::stylesheet::supports_descriptor("@page", &declaration),
            ProbeContext::Style(..) | ProbeContext::Margin => {
                crate::declaration::supports_declaration(&declaration.name, &declaration.value)
            }
        };
        self.chain = None;
        Ok(ProbeItem::Checked(supported))
    }
}
impl<'i> AtRuleParser<'i> for SupportProbe<'_> {
    type Prelude = (AtPrelude, ProbeContext);
    type AtRule = ProbeItem<'i>;
    type Error = StyleError;
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, StyleError>> {
        self.budget
            .rule(|| Self::location(self.start, input))
            .map_err(|error| input.new_custom_error(error))?;
        if matches!(self.context, ProbeContext::Page)
            && crate::MarginLocation::parse(&name).is_some()
        {
            input.expect_exhausted()?;
            self.chain = None;
            return Ok((AtPrelude::Font, ProbeContext::Margin));
        }
        if matches!(
            self.context,
            ProbeContext::Font | ProbeContext::Page | ProbeContext::Margin
        ) {
            return Err(
                input.new_custom_error(StyleError::Invalid("at-rule in descriptor context".into()))
            );
        }
        if matches!(self.context, ProbeContext::Style(context, _) if context.nested)
            && AtRuleName::parse(&name).is_some_and(|kind| !kind.can_nest_in_style())
        {
            return Err(input.new_custom_error(StyleError::Invalid(format!(
                "@{name} cannot be nested in a style rule"
            ))));
        }
        let ProbeContext::Style(rules, _) = self.context else {
            unreachable!("descriptor contexts reject nested at-rules")
        };
        let prelude = at_prelude(
            name,
            input,
            &mut self.chain,
            self.namespaces,
            rules.mode,
            false,
        )?;
        let context = match prelude {
            AtPrelude::Font => ProbeContext::Font,
            AtPrelude::Page(_) => ProbeContext::Page,
            AtPrelude::Scope(_) => ProbeContext::Style(rules.scoped(), rules.nested),
            _ => self.context,
        };
        Ok((prelude, context))
    }
    fn rule_without_block(
        &mut self,
        prelude: Self::Prelude,
        _: &ParserState,
    ) -> Result<Self::AtRule, ()> {
        if matches!(self.context, ProbeContext::Style(context, _) if !context.allows_statement(&prelude.0))
        {
            return Err(());
        }
        statement(
            prelude.0,
            SourceLocation {
                base: self.start.base.clone(),
                line: 1,
                column: 1,
            },
        )
        .map(|_| ProbeItem::Checked(true))
    }
    fn parse_block<'t>(
        &mut self,
        (prelude, context): Self::Prelude,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, StyleError>> {
        if matches!(
            prelude,
            AtPrelude::CustomMedia(..) | AtPrelude::Import(_) | AtPrelude::Namespace(..)
        ) || matches!(&prelude,AtPrelude::Layer(names) if names.len()>1)
        {
            return Ok(ProbeItem::Checked(false));
        }
        let location = Self::location(self.start, input);
        let source = block_source(input);
        if matches!(
            prelude,
            AtPrelude::Conditional(..) | AtPrelude::SupportDefinition(_)
        ) {
            self.chain = Some(input.position());
        }
        Ok(ProbeItem::Body(context, source, location))
    }
}
impl<'i> QualifiedRuleParser<'i> for SupportProbe<'_> {
    type Prelude = bool;
    type QualifiedRule = ProbeItem<'i>;
    type Error = StyleError;
    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, StyleError>> {
        self.chain = None;
        self.budget
            .rule(|| Self::location(self.start, input))
            .map_err(|error| input.new_custom_error(error))?;
        let source = block_source(input);
        let ProbeContext::Style(context, _) = self.context else {
            unreachable!("qualified descriptor rules are disabled")
        };
        Ok(crate::selector::supports_context_selectors(
            source,
            &self.namespaces.borrow(),
            context.mode,
        ))
    }
    fn parse_block<'t>(
        &mut self,
        supported: bool,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, StyleError>> {
        if supported {
            let location = Self::location(self.start, input);
            Ok(ProbeItem::Body(
                ProbeContext::Style(RuleContext::STYLE, true),
                block_source(input),
                location,
            ))
        } else {
            Ok(ProbeItem::Checked(false))
        }
    }
}
impl<'i> RuleBodyItemParser<'i, ProbeItem<'i>, StyleError> for SupportProbe<'_> {
    fn parse_declarations(&self) -> bool {
        match self.context {
            ProbeContext::Style(_, allowed) => allowed,
            _ => true,
        }
    }
    fn parse_qualified(&self) -> bool {
        matches!(self.context, ProbeContext::Style(..))
    }
}
fn supports_block_with_namespaces(
    source: &str,
    start: &SourceLocation,
    budget: &mut crate::loader::Budget,
    namespaces: &Namespaces,
    context: RuleContext,
) -> Result<bool, StyleError> {
    if !balanced_components(source)? {
        return Ok(false);
    }
    let mut pending = vec![(ProbeContext::Style(context, true), source, start.clone(), 0)];
    while let Some((context, source, start, depth)) = pending.pop() {
        if depth >= MAX_GROUP_DEPTH {
            return Err(StyleError::GrammarLimit {
                resource: "support probe nesting",
                limit: MAX_GROUP_DEPTH,
            });
        }
        let mut input = ParserInput::new(source);
        let mut parser = SupportProbe {
            start: &start,
            chain: None,
            context,
            budget,
            namespaces,
        };
        for item in RuleBodyParser::new(&mut Parser::new(&mut input), &mut parser) {
            match item {
                Ok(ProbeItem::Checked(true)) => {}
                Ok(ProbeItem::Body(context, source, location)) => {
                    pending.push((context, source, location, depth + 1))
                }
                Ok(ProbeItem::Checked(false)) => return Ok(false),
                Err((error, _)) => match failure(error) {
                    StyleError::Invalid(_) => return Ok(false),
                    error => return Err(error),
                },
            }
        }
    }
    Ok(true)
}
