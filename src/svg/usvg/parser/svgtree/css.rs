// Copyright 2026 the ferri-pdf Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, Delimiter, ParseError, Parser, ParserInput,
    ParserState, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};
use ferri_pdf_dom::{DomError, MAX_COMPONENT_DEPTH, NamespaceMap, Selector, TreeScopeId, XmlTree};
use std::cell::{Cell, RefCell};

#[derive(Clone)]
pub(super) struct Declaration<'i> {
    pub name: CowRcStr<'i>,
    pub value: &'i str,
    pub important: bool,
}

pub(super) struct Rule<'i> {
    pub selector: Selector,
    pub declarations: Vec<Declaration<'i>>,
}

pub(super) struct StyleSheet<'i> {
    pub rules: Vec<Rule<'i>>,
    pub limit: Option<usize>,
    tree: RefCell<Option<XmlTree<'i, 'i>>>,
    instance: Cell<Option<TreeScopeId>>,
}

pub(super) struct InstanceContext<'a> {
    current: &'a Cell<Option<TreeScopeId>>,
    previous: Option<TreeScopeId>,
}

impl Drop for InstanceContext<'_> {
    fn drop(&mut self) {
        self.current.set(self.previous);
    }
}

impl<'i> StyleSheet<'i> {
    pub(super) fn new() -> Self {
        Self {
            rules: Vec::new(),
            limit: None,
            tree: RefCell::new(None),
            instance: Cell::new(None),
        }
    }
    pub(super) fn parse_more(&mut self, source: &'i str) {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let mut rules = Rules {
            namespaces: NamespaceMap::new(),
            namespaces_allowed: true,
        };
        for result in StyleSheetParser::new(&mut parser, &mut rules) {
            match result {
                Ok(rules) => self.rules.extend(rules),
                Err((error, _)) => {
                    if let Some(limit) = component_limit(&error) {
                        self.limit = Some(limit);
                    }
                }
            }
        }
        self.rules.sort_by_key(|rule| {
            rule.selector
                .single_specificity()
                .expect("one parsed selector")
        });
    }
    pub(super) fn prepare(&mut self, xml: &'i roxmltree::Document<'i>) {
        if !self.rules.is_empty() {
            *self.tree.get_mut() = Some(XmlTree::new(xml));
        }
    }
    fn matches_context(
        &self,
        rule: &Rule<'_>,
        node: roxmltree::Node<'_, '_>,
        context: Option<TreeScopeId>,
    ) -> bool {
        self.tree.borrow().as_ref().is_some_and(|tree| {
            tree.element_in(node, self.instance.get())
                .is_some_and(|element| tree.matches_element(element.id(), &rule.selector, context))
        })
    }
    pub(super) fn matching_declarations<'a, 'input>(
        &'a self,
        node: roxmltree::Node<'a, 'input>,
        context: Option<TreeScopeId>,
    ) -> impl Iterator<Item = &'a Declaration<'i>> + 'a {
        self.rules
            .iter()
            .filter(move |rule| self.matches_context(rule, node, context))
            .flat_map(|rule| &rule.declarations)
    }
    pub(super) fn create_instance(
        &self,
        source: roxmltree::Node<'_, '_>,
        host: roxmltree::Node<'_, '_>,
        node_limit: usize,
    ) -> Result<Option<TreeScopeId>, roxmltree::Error> {
        let mut tree = self.tree.borrow_mut();
        let Some(tree) = tree.as_mut() else {
            return Ok(None);
        };
        let host = tree
            .element_in(host, self.instance.get())
            .expect("native SVG instance host")
            .id();
        tree.create_instance(source, host, node_limit)
            .map_err(|error| match error {
                DomError::TreeLimit { .. } => roxmltree::Error::NodesLimitReached,
            })
    }
    pub(super) fn enter_instance(&self, instance: Option<TreeScopeId>) -> InstanceContext<'_> {
        InstanceContext {
            current: &self.instance,
            previous: self.instance.replace(instance),
        }
    }
}

pub(super) fn declarations<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<Vec<Declaration<'i>>, usize> {
    let mut declarations = Vec::new();
    for result in RuleBodyParser::new(input, &mut Declarations) {
        match result {
            Ok(declaration) => declarations.push(declaration),
            Err((error, _)) => {
                if let Some(limit) = component_limit(&error) {
                    return Err(limit);
                }
            }
        }
    }
    Ok(declarations)
}

fn component_limit(error: &ParseError<'_, usize>) -> Option<usize> {
    match error.kind {
        cssparser::ParseErrorKind::Custom(limit) => Some(limit),
        _ => None,
    }
}

fn components<'i, 't>(input: &mut Parser<'i, 't>) -> Result<(), ParseError<'i, usize>> {
    nested_components(input, 0)
}

fn nested_components<'i, 't>(
    input: &mut Parser<'i, 't>,
    depth: usize,
) -> Result<(), ParseError<'i, usize>> {
    if depth >= MAX_COMPONENT_DEPTH {
        return Err(input.new_custom_error(MAX_COMPONENT_DEPTH));
    }
    while let Ok(token) = input.next_including_whitespace_and_comments() {
        match token {
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                input.parse_nested_block(|input| nested_components(input, depth + 1))?;
            }
            token if token.is_parse_error() => {
                let token = token.clone();
                return Err(input.new_unexpected_token_error(token));
            }
            _ => {}
        }
    }
    Ok(())
}

struct Declarations;
impl<'i> DeclarationParser<'i> for Declarations {
    type Declaration = Declaration<'i>;
    type Error = usize;
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _: &ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, usize>> {
        let start = input.position();
        let value = input.parse_until_before(Delimiter::Bang, |input| {
            components(input)?;
            Ok::<_, ParseError<'i, usize>>(input.slice_from(start))
        })?;
        let important = input.try_parse(cssparser::parse_important).is_ok();
        input.expect_exhausted()?;
        if value.trim().is_empty() && !name.starts_with("--") {
            return Err(input.new_error(cssparser::BasicParseErrorKind::QualifiedRuleInvalid));
        }
        let name = if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
            name.to_ascii_lowercase().into()
        } else {
            name
        };
        Ok(Declaration {
            name,
            value: value.trim(),
            important,
        })
    }
}
impl<'i> AtRuleParser<'i> for Declarations {
    type Prelude = ();
    type AtRule = Declaration<'i>;
    type Error = usize;
}
impl<'i> QualifiedRuleParser<'i> for Declarations {
    type Prelude = ();
    type QualifiedRule = Declaration<'i>;
    type Error = usize;
}
impl<'i> RuleBodyItemParser<'i, Declaration<'i>, usize> for Declarations {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

struct Rules {
    namespaces: NamespaceMap,
    namespaces_allowed: bool,
}
impl<'i> AtRuleParser<'i> for Rules {
    type Prelude = (String, String);
    type AtRule = Vec<Rule<'i>>;
    type Error = usize;
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, usize>> {
        if self.namespaces_allowed && name.eq_ignore_ascii_case("namespace") {
            ferri_pdf_dom::parse_namespace(input).map_err(Into::into)
        } else {
            Err(input.new_error(cssparser::BasicParseErrorKind::AtRuleInvalid(name)))
        }
    }
    fn rule_without_block(
        &mut self,
        (prefix, uri): Self::Prelude,
        _: &ParserState,
    ) -> Result<Self::AtRule, ()> {
        self.namespaces.insert(prefix, uri);
        Ok(Vec::new())
    }
}
impl<'i> QualifiedRuleParser<'i> for Rules {
    type Prelude = Vec<Selector>;
    type QualifiedRule = Vec<Rule<'i>>;
    type Error = usize;
    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, usize>> {
        input.parse_comma_separated(|input| {
            let start = input.position();
            components(input)?;
            Selector::parse_with_namespaces(input.slice_from(start).trim(), &self.namespaces, true)
                .map_err(|_| input.new_error(cssparser::BasicParseErrorKind::QualifiedRuleInvalid))
        })
    }
    fn parse_block<'t>(
        &mut self,
        selectors: Self::Prelude,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, usize>> {
        self.namespaces_allowed = false;
        let declarations = declarations(input).map_err(|limit| input.new_custom_error(limit))?;
        Ok(selectors
            .into_iter()
            .map(|selector| Rule {
                selector,
                declarations: declarations.clone(),
            })
            .collect())
    }
}
