use crate::{
    SourceLocation, StyleDiagnostic, StyleDiagnosticKind, StyleError,
    page_margins::MarginLocation,
    parser::{Declaration, Declarations, declarations, recover},
};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};
use ferri_pdf_resources::ResourceBase;
use std::cell::RefCell;

pub(super) enum PageItem {
    Declaration(Declaration),
    Margin(MarginLocation, Vec<Declaration>),
}
struct PageParser<'a> {
    base: &'a ResourceBase,
    diagnostics: &'a RefCell<Vec<StyleDiagnostic>>,
    budget: &'a mut crate::loader::Budget,
}
impl<'i> DeclarationParser<'i> for PageParser<'_> {
    type Declaration = PageItem;
    type Error = StyleError;
    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        state: &ParserState,
    ) -> Result<PageItem, ParseError<'i, StyleError>> {
        Declarations { base: self.base }
            .parse_value(name, input, state)
            .map(PageItem::Declaration)
    }
}
impl<'i> AtRuleParser<'i> for PageParser<'_> {
    type Prelude = MarginLocation;
    type AtRule = PageItem;
    type Error = StyleError;
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<MarginLocation, ParseError<'i, StyleError>> {
        self.budget
            .rule(|| SourceLocation::new(self.base, input.current_source_location()))
            .map_err(|error| input.new_custom_error(error))?;
        input.expect_exhausted()?;
        MarginLocation::parse(&name).ok_or_else(|| {
            input.new_custom_error(StyleError::Invalid(format!(
                "unknown page-margin rule: @{name}"
            )))
        })
    }
    fn parse_block<'t>(
        &mut self,
        location: MarginLocation,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<PageItem, ParseError<'i, StyleError>> {
        Ok(PageItem::Margin(
            location,
            declarations(input, self.base, self.diagnostics)?,
        ))
    }
}
impl<'i> QualifiedRuleParser<'i> for PageParser<'_> {
    type Prelude = ();
    type QualifiedRule = PageItem;
    type Error = StyleError;
}
impl<'i> RuleBodyItemParser<'i, PageItem, StyleError> for PageParser<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}
pub(super) fn page_items<'i>(
    input: &mut Parser<'i, '_>,
    base: &ResourceBase,
    diagnostics: &RefCell<Vec<StyleDiagnostic>>,
    budget: &mut crate::loader::Budget,
) -> Result<Vec<PageItem>, ParseError<'i, StyleError>> {
    let mut items = Vec::new();
    for item in RuleBodyParser::new(
        input,
        &mut PageParser {
            base,
            diagnostics,
            budget,
        },
    ) {
        match item {
            Ok(item) => items.push(item),
            Err((error, source)) => recover(
                error,
                source,
                base,
                StyleDiagnosticKind::InvalidDeclaration,
                diagnostics,
            )?,
        }
    }
    Ok(items)
}

pub(super) fn page_selectors<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<crate::page_rules::PageSelectors, ParseError<'i, StyleError>> {
    use crate::page_rules::{PageSelector, PageSelectors};
    if input.is_exhausted() {
        return Ok(PageSelectors::default());
    }
    input
        .parse_comma_separated(|input| {
            input.skip_whitespace();
            let name = input
                .try_parse(|input| input.expect_ident_cloned())
                .ok()
                .map(|name| std::sync::Arc::from(name.as_ref()));
            let mut selector = PageSelector::new(name);
            while !input.is_exhausted() {
                let token = input.next_including_whitespace()?.clone();
                match token {
                    cssparser::Token::WhiteSpace(_) => {
                        input.expect_exhausted()?;
                        break;
                    }
                    cssparser::Token::Colon => {
                        let token = input.next_including_whitespace()?.clone();
                        match token {
                            cssparser::Token::Ident(name) if selector.add_pseudo(&name) => {}
                            token => return Err(input.new_unexpected_token_error(token)),
                        }
                    }
                    token => return Err(input.new_unexpected_token_error(token)),
                }
            }
            if !selector.is_empty() {
                Ok(selector)
            } else {
                Err(input.new_custom_error(StyleError::Invalid("empty page selector".into())))
            }
        })
        .map(PageSelectors)
}
