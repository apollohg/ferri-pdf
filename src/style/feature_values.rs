//! The `@font-feature-values` rule body (CSS Fonts 4 §6.9.1): feature value blocks of
//! named indices, with invalid blocks and declarations ignored individually.
use crate::fonts::FeatureValueType;
use crate::resources::ResourceBase;
use crate::style::{StyleDiagnostic, StyleDiagnosticKind, StyleError, parser::recover};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, Token,
};
use std::cell::RefCell;

/// The one descriptor of the rule body besides the feature value blocks (§4.9.1).
const FONT_DISPLAY: &str = "font-display";
const FONT_DISPLAY_VALUES: [&str; 5] = ["auto", "block", "swap", "fallback", "optional"];

/// One font feature value declaration of a feature value block.
pub(super) struct FeatureValueDefinition {
    pub(super) kind: FeatureValueType,
    pub(super) name: String,
    pub(super) values: Vec<u32>,
}

enum Item {
    /// `font-display` selects a loading timeline (§4.9); fonts load before paged layout, so it
    /// has no effect on the printed result.
    Display,
    Block(Vec<FeatureValueDefinition>),
}

struct Body<'a> {
    base: &'a ResourceBase,
    diagnostics: &'a RefCell<Vec<StyleDiagnostic>>,
}

pub(super) fn feature_values<'i>(
    input: &mut Parser<'i, '_>,
    base: &ResourceBase,
    diagnostics: &RefCell<Vec<StyleDiagnostic>>,
) -> Result<Vec<FeatureValueDefinition>, ParseError<'i, StyleError>> {
    let mut definitions = Vec::new();
    for item in RuleBodyParser::new(input, &mut Body { base, diagnostics }) {
        match item {
            Ok(Item::Block(block)) => definitions.extend(block),
            Ok(Item::Display) => {}
            Err((error, source)) => recover(
                error,
                source,
                base,
                StyleDiagnosticKind::InvalidRule,
                diagnostics,
            )?,
        }
    }
    Ok(definitions)
}

fn invalid<'i>(input: &Parser<'i, '_>, context: &str) -> ParseError<'i, StyleError> {
    input.new_custom_error(StyleError::Invalid(format!(
        "@font-feature-values {context}"
    )))
}

impl<'i> DeclarationParser<'i> for Body<'_> {
    type Declaration = Item;
    type Error = StyleError;
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _: &ParserState,
    ) -> Result<Item, ParseError<'i, StyleError>> {
        if !name.eq_ignore_ascii_case(FONT_DISPLAY) {
            return Err(invalid(input, &format!("descriptor {name}")));
        }
        let value = input.expect_ident_cloned()?;
        input.expect_exhausted()?;
        if !FONT_DISPLAY_VALUES
            .iter()
            .any(|keyword| value.eq_ignore_ascii_case(keyword))
        {
            return Err(invalid(input, &format!("font-display {value}")));
        }
        Ok(Item::Display)
    }
}

impl<'i> AtRuleParser<'i> for Body<'_> {
    type Prelude = FeatureValueType;
    type AtRule = Item;
    type Error = StyleError;
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<FeatureValueType, ParseError<'i, StyleError>> {
        let kind = FeatureValueType::from_name(&name)
            .ok_or_else(|| invalid(input, &format!("block @{name}")))?;
        input.expect_exhausted()?;
        Ok(kind)
    }
    fn parse_block<'t>(
        &mut self,
        kind: FeatureValueType,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Item, ParseError<'i, StyleError>> {
        let mut definitions = Vec::new();
        for item in RuleBodyParser::new(input, &mut Block { kind }) {
            match item {
                Ok(definition) => definitions.push(definition),
                Err((error, source)) => recover(
                    error,
                    source,
                    self.base,
                    StyleDiagnosticKind::InvalidDeclaration,
                    self.diagnostics,
                )?,
            }
        }
        Ok(Item::Block(definitions))
    }
}

impl QualifiedRuleParser<'_> for Body<'_> {
    type Prelude = ();
    type QualifiedRule = Item;
    type Error = StyleError;
}

impl RuleBodyItemParser<'_, Item, StyleError> for Body<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

struct Block {
    kind: FeatureValueType,
}

impl<'i> DeclarationParser<'i> for Block {
    type Declaration = FeatureValueDefinition;
    type Error = StyleError;
    /// Names are any case-sensitive identifier; values are one or more non-negative integers
    /// in the count the block type allows (§6.9.1, §6.9.2).
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _: &ParserState,
    ) -> Result<FeatureValueDefinition, ParseError<'i, StyleError>> {
        let mut values = Vec::new();
        while !input.is_exhausted() {
            let location = input.current_source_location();
            match input.next()? {
                Token::Number {
                    int_value: Some(value),
                    ..
                } => values.push(
                    u32::try_from(*value)
                        .map_err(|_| invalid(input, &format!("negative index for {name}")))?,
                ),
                token => {
                    let token = token.clone();
                    return Err(location.new_unexpected_token_error(token));
                }
            }
        }
        if !self.kind.accepts(&values) {
            return Err(invalid(input, &format!("value count for {name}")));
        }
        Ok(FeatureValueDefinition {
            kind: self.kind,
            name: name.to_string(),
            values,
        })
    }
}

impl AtRuleParser<'_> for Block {
    type Prelude = ();
    type AtRule = FeatureValueDefinition;
    type Error = StyleError;
}

impl QualifiedRuleParser<'_> for Block {
    type Prelude = ();
    type QualifiedRule = FeatureValueDefinition;
    type Error = StyleError;
}

impl RuleBodyItemParser<'_, FeatureValueDefinition, StyleError> for Block {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}
