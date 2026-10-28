use crate::{
    StyleError,
    parser::{balanced_components, consume, raw},
};
use cssparser::{ParseError, Parser, ParserInput, Token};
pub use ferri_pdf_dom::GeneratedPseudoElement as PseudoElement;
use ferri_pdf_dom::{Element, Selector, TreeContext, TreeScopeId};
pub(super) use ferri_pdf_dom::{NamespaceMap, SelectorMode};
pub(super) type Namespaces = std::rc::Rc<std::cell::RefCell<NamespaceMap>>;

pub(super) fn tree_context(
    origin: crate::cascade::Origin,
    scope: Option<TreeScopeId>,
) -> TreeContext {
    if origin == crate::cascade::Origin::Author {
        scope.map_or(TreeContext::Document, TreeContext::Root)
    } else {
        TreeContext::All
    }
}

pub(super) const SELECTOR_TARGET_COUNT: usize = 3;
pub(super) fn target_index(pseudo: Option<PseudoElement>) -> usize {
    match pseudo {
        None => 0,
        Some(PseudoElement::Before) => 1,
        Some(PseudoElement::After) => 2,
    }
}

const TYPE_SPECIFICITY_MASK: u32 = (1 << 10) - 1;
const MAX_RESOLVED_SELECTOR_DEPTH: usize = crate::parser::MAX_COMPONENT_DEPTH + 1;
const RESOLVED_SELECTOR_NESTING: &str = "resolved selector nesting";
const PSEUDO_PARENT_COMPONENTS: usize = 2; // The Is wrapper and generated pseudo node.

fn selector_depth(depth: usize) -> Result<(), StyleError> {
    if depth > MAX_RESOLVED_SELECTOR_DEPTH {
        return Err(StyleError::GrammarLimit {
            resource: RESOLVED_SELECTOR_NESTING,
            limit: MAX_RESOLVED_SELECTOR_DEPTH,
        });
    }
    Ok(())
}

#[derive(Clone)]
pub(super) struct RuleSelector {
    selector: Selector,
    specificity: u32,
    pseudo: Option<PseudoElement>,
}
impl RuleSelector {
    fn parse_context(
        source: &str,
        namespaces: &NamespaceMap,
        mode: SelectorMode,
    ) -> Result<Self, StyleError> {
        let mut source_input = ParserInput::new(source);
        let (origin, pseudo) = originating_selector(&mut Parser::new(&mut source_input))
            .map_err(|error| StyleError::Invalid(format!("selector {source}: {error:?}")))?;
        let selector = Selector::parse_context(&origin, namespaces, true, mode)
            .map_err(|error| StyleError::Invalid(format!("selector {source}: {error:?}")))?;
        Self::from_parsed(selector, pseudo)
    }
    fn from_parsed(selector: Selector, pseudo: Option<PseudoElement>) -> Result<Self, StyleError> {
        selector_depth(selector.depth())?;
        let mut specificity = selector
            .single_specificity()
            .ok_or_else(|| StyleError::Invalid("expected one selector".into()))?;
        if pseudo.is_some() {
            specificity = (specificity & !TYPE_SPECIFICITY_MASK)
                | ((specificity & TYPE_SPECIFICITY_MASK) + 1).min(TYPE_SPECIFICITY_MASK);
        }
        Ok(Self {
            selector,
            specificity,
            pseudo,
        })
    }
    pub(super) fn parent_list(selectors: &[Self]) -> Result<Selector, StyleError> {
        Selector::list(
            &selectors
                .iter()
                .map(|selector| match selector.pseudo {
                    Some(pseudo) => selector.selector.with_pseudo_element(pseudo),
                    None => selector.selector.clone(),
                })
                .collect::<Vec<_>>(),
        )
        .ok_or_else(|| StyleError::Invalid("empty nesting parent selector list".into()))
    }
    pub(super) fn nesting_work(&self, parent_work: usize) -> usize {
        self.selector.nesting_work(parent_work)
    }
    pub(super) fn parent_work(&self) -> usize {
        self.selector
            .nesting_work(1)
            .saturating_add(if self.pseudo.is_some() {
                PSEUDO_PARENT_COMPONENTS
            } else {
                0
            })
    }
    pub(super) fn resolve_parent(&self, parent: &Selector) -> Result<Self, StyleError> {
        selector_depth(self.selector.resolved_depth(parent))?;
        Self::from_parsed(self.selector.resolve_parent(parent), self.pseudo)
    }
    pub(super) fn target_index(&self) -> usize {
        target_index(self.pseudo)
    }
    pub(super) fn scope_origins<'a>(
        &self,
        element: Element<'a>,
        tree: TreeContext,
    ) -> impl Iterator<Item = (Element<'a>, usize)> + Clone + use<'a> {
        let generated = usize::from(self.pseudo.is_some());
        self.selector
            .scope_origins(element, tree)
            .map(move |(origin, hops)| (origin, hops + generated))
    }
    pub(super) fn matches(
        &self,
        element: &Element<'_>,
        pseudo: Option<PseudoElement>,
        scope: ferri_pdf_dom::SelectorScope,
        tree: TreeContext,
    ) -> Option<u32> {
        (self.pseudo == pseudo && element.matches_in_selector_scope(&self.selector, tree, scope))
            .then_some(self.specificity)
    }
}

pub(super) fn resolve_selectors(
    parent: &[RuleSelector],
    selectors: &[RuleSelector],
) -> Result<Vec<RuleSelector>, StyleError> {
    if parent.is_empty() {
        return Ok(selectors.to_vec());
    }
    let parent = RuleSelector::parent_list(parent)?;
    selectors
        .iter()
        .map(|selector| selector.resolve_parent(&parent))
        .collect()
}

pub(super) fn selector_work(selectors: &[RuleSelector], parent: &[RuleSelector]) -> usize {
    let parent_work = parent.iter().fold(0usize, |work, selector| {
        work.saturating_add(selector.parent_work())
    });
    selectors.iter().fold(0usize, |work, selector| {
        work.saturating_add(selector.nesting_work(parent_work.max(1)))
    })
}

pub(super) fn scope_boundary<'i>(
    input: &mut Parser<'i, '_>,
    namespaces: &NamespaceMap,
    mode: SelectorMode,
    forgiving: bool,
) -> Result<Vec<RuleSelector>, ParseError<'i, StyleError>> {
    input.parse_comma_separated(|input| {
        let source = raw(input)?;
        let selector = Selector::parse_scope_boundary(&source, namespaces, forgiving, mode)
            .map_err(|error| {
                input.new_custom_error(StyleError::Invalid(format!("scope selector: {error:?}")))
            })?;
        RuleSelector::from_parsed(selector, None).map_err(|error| input.new_custom_error(error))
    })
}

fn originating_selector<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(String, Option<PseudoElement>), ParseError<'i, StyleError>> {
    let start = input.position();
    let mut universal = true;
    while !input.is_exhausted() {
        let state = input.state();
        match input.next_including_whitespace()?.clone() {
            Token::Colon => {
                let parsed = input.try_parse(|p| {
                    let token = p.next_including_whitespace()?.clone();
                    let name = match token {
                        Token::Colon => p.next_including_whitespace()?.clone(),
                        token => token,
                    };
                    match name {
                        Token::Ident(name) => PseudoElement::parse(&name).ok_or_else(|| {
                            p.new_custom_error::<_, StyleError>(StyleError::Invalid(
                                "not a generated pseudo-element".into(),
                            ))
                        }),
                        token => Err(p.new_unexpected_token_error(token)),
                    }
                });
                if let Ok(pseudo) = parsed {
                    let mut source = input.slice(start..state.position()).to_owned();
                    if universal {
                        source.push('*');
                    }
                    input.expect_exhausted()?;
                    return Ok((source, Some(pseudo)));
                }
                universal = false;
            }
            Token::WhiteSpace(_) | Token::Delim('>' | '+' | '~') => universal = true,
            Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                input.parse_nested_block(consume)?;
                universal = false;
            }
            _ => universal = false,
        }
    }
    Ok((input.slice_from(start).to_owned(), None))
}
pub(super) fn parse<'i>(
    input: &mut Parser<'i, '_>,
    namespaces: &NamespaceMap,
    mode: SelectorMode,
) -> Result<Vec<RuleSelector>, ParseError<'i, StyleError>> {
    input.parse_comma_separated(|input| {
        let source = raw(input)?;
        RuleSelector::parse_context(&source, namespaces, mode)
            .map_err(|error| input.new_custom_error(error))
    })
}

pub(super) fn supports_selectors_with_namespaces(
    source: &str,
    list: bool,
    namespaces: &NamespaceMap,
) -> bool {
    supports_selectors(source, list, namespaces, SelectorMode::Normal)
}
pub(super) fn supports_context_selectors(
    source: &str,
    namespaces: &NamespaceMap,
    mode: SelectorMode,
) -> bool {
    supports_selectors(source, true, namespaces, mode)
}
fn supports_selectors(
    source: &str,
    list: bool,
    namespaces: &NamespaceMap,
    mode: SelectorMode,
) -> bool {
    if !matches!(balanced_components(source), Ok(true)) {
        return false;
    }
    let mut input = ParserInput::new(source);
    let parsed = Parser::new(&mut input).parse_comma_separated(|input| {
        let (origin, _) = originating_selector(input)?;
        Selector::parse_context(&origin, namespaces, false, mode)
            .map_err(|error| {
                input.new_custom_error(StyleError::Invalid(format!("selector: {error:?}")))
            })
            .and_then(|selector| {
                selector_depth(selector.depth()).map_err(|error| input.new_custom_error(error))
            })
    });
    parsed.is_ok_and(|selectors| list || selectors.len() == 1)
}
