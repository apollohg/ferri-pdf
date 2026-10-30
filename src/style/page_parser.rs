use crate::resources::ResourceBase;
use crate::style::{
    SourceLocation, StyleDiagnostic, StyleDiagnosticKind, StyleError,
    page_margins::MarginLocation,
    parser::{Declaration, Declarations, declarations, recover},
};
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser,
};
use std::cell::RefCell;

pub(super) enum PageItem {
    Declaration(Declaration),
    Margin(MarginLocation, Vec<Declaration>),
    /// `@footnote`, the page's footnote area (CSS GCPM 3 §2.4).
    Footnote(Vec<Declaration>),
}
/// The at-rules a page context holds.
pub(super) enum PageArea {
    Margin(MarginLocation),
    Footnote,
}
pub(super) const FOOTNOTE_AT_RULE: &str = "footnote";
struct PageParser<'a> {
    base: &'a ResourceBase,
    diagnostics: &'a RefCell<Vec<StyleDiagnostic>>,
    budget: &'a mut crate::style::loader::Budget,
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
    type Prelude = PageArea;
    type AtRule = PageItem;
    type Error = StyleError;
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<PageArea, ParseError<'i, StyleError>> {
        self.budget
            .rule(|| SourceLocation::new(self.base, input.current_source_location()))
            .map_err(|error| input.new_custom_error(error))?;
        input.expect_exhausted()?;
        if name.eq_ignore_ascii_case(FOOTNOTE_AT_RULE) {
            return Ok(PageArea::Footnote);
        }
        MarginLocation::parse(&name)
            .map(PageArea::Margin)
            .ok_or_else(|| {
                input.new_custom_error(StyleError::Invalid(format!(
                    "unknown page-margin rule: @{name}"
                )))
            })
    }
    fn parse_block<'t>(
        &mut self,
        area: PageArea,
        _: &ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<PageItem, ParseError<'i, StyleError>> {
        let declarations = declarations(input, self.base, self.diagnostics)?;
        Ok(match area {
            PageArea::Margin(location) => PageItem::Margin(location, declarations),
            PageArea::Footnote => PageItem::Footnote(declarations),
        })
    }
}
impl QualifiedRuleParser<'_> for PageParser<'_> {
    type Prelude = ();
    type QualifiedRule = PageItem;
    type Error = StyleError;
}
impl RuleBodyItemParser<'_, PageItem, StyleError> for PageParser<'_> {
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
    budget: &mut crate::style::loader::Budget,
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

const NTH_PSEUDO_CLASS: &str = "nth";
const NTH_GROUP_KEYWORD: &str = "of";

/// `:nth( <an+b> [of <custom-ident>]? )` (CSS GCPM 3 §3.1).
fn nth_page<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<crate::style::page_rules::NthPage, ParseError<'i, StyleError>> {
    let (a, b) = cssparser::parse_nth(input)?;
    let group = if input.is_exhausted() {
        None
    } else {
        input.expect_ident_matching(NTH_GROUP_KEYWORD)?;
        let location = input.current_source_location();
        let name = input.expect_ident_cloned()?;
        if !crate::style::matcher::is_custom_ident(&name) {
            return Err(location.new_custom_error(StyleError::Invalid(format!(
                "page group name is not a custom identifier: {name}"
            ))));
        }
        Some(std::sync::Arc::from(name.as_ref()))
    };
    input.expect_exhausted()?;
    Ok(crate::style::page_rules::NthPage {
        position: selectors::parser::AnPlusB(a, b),
        group,
    })
}

pub(super) fn page_selectors<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<crate::style::page_rules::PageSelectors, ParseError<'i, StyleError>> {
    use crate::style::page_rules::{PageSelector, PageSelectors};
    if input.is_exhausted() {
        return Ok(PageSelectors::default());
    }
    input
        .parse_comma_separated(|input| {
            input.skip_whitespace();
            let name = input
                .try_parse(Parser::expect_ident_cloned)
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
                            cssparser::Token::Function(name)
                                if name.eq_ignore_ascii_case(NTH_PSEUDO_CLASS) =>
                            {
                                selector.add_nth(input.parse_nested_block(nth_page)?);
                            }
                            token => return Err(input.new_unexpected_token_error(token)),
                        }
                    }
                    token => return Err(input.new_unexpected_token_error(token)),
                }
            }
            if selector.is_empty() {
                Err(input.new_custom_error(StyleError::Invalid("empty page selector".into())))
            } else {
                Ok(selector)
            }
        })
        .map(PageSelectors)
}
