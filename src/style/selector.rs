use crate::dom::{Element, Selector, TreeContext, TreeScopeId};
pub(crate) use crate::dom::{GeneratedPseudoElement, PseudoElement};
pub(super) use crate::dom::{NamespaceMap, SelectorMode, SelectorOrigin};
use crate::style::{
    StyleError,
    parser::{balanced_components, consume, raw},
};
use cssparser::{ParseError, Parser, ParserInput, Token};
pub(super) type Namespaces = std::rc::Rc<std::cell::RefCell<NamespaceMap>>;

pub(super) fn tree_context(
    origin: crate::style::cascade::Origin,
    scope: Option<TreeScopeId>,
) -> TreeContext {
    if origin == crate::style::cascade::Origin::Author {
        scope.map_or(TreeContext::Document, TreeContext::Root)
    } else {
        TreeContext::All
    }
}

pub(super) fn selector_origin(origin: crate::style::cascade::Origin) -> SelectorOrigin {
    if origin == crate::style::cascade::Origin::UserAgent {
        SelectorOrigin::UserAgent
    } else {
        SelectorOrigin::Author
    }
}

pub(super) const SELECTOR_TARGET_COUNT: usize = 17;
pub(super) fn target_index(pseudo: Option<PseudoElement>) -> usize {
    match pseudo {
        None => 0,
        Some(PseudoElement::Before) => 1,
        Some(PseudoElement::After) => 2,
        Some(PseudoElement::Marker) => 3,
        Some(PseudoElement::FirstLine) => 4,
        Some(PseudoElement::FirstLetter) => 5,
        Some(PseudoElement::FirstLetterPrefix) => 6,
        Some(PseudoElement::FirstLetterSuffix) => 7,
        Some(PseudoElement::Placeholder) => 8,
        Some(PseudoElement::FileSelectorButton) => 9,
        Some(PseudoElement::Absent(_)) => 10,
        Some(PseudoElement::DetailsContent) => {
            unreachable!("::details-content rules match its backing element as their subject")
        }
        Some(PseudoElement::FootnoteCall) => 11,
        Some(PseudoElement::FootnoteMarker) => 12,
        Some(PseudoElement::Checkmark) => 13,
        Some(PseudoElement::PickerIcon) => 14,
        Some(PseudoElement::ScrollMarkerGroup) => 15,
        Some(PseudoElement::ScrollMarker) => 16,
    }
}

const TYPE_SPECIFICITY_MASK: u32 = (1 << 10) - 1;
/// The bit offsets of the ID, class and type counts packed in a specificity.
const SPECIFICITY_SHIFTS: [u32; 3] = [20, 10, 0];
const MAX_RESOLVED_SELECTOR_DEPTH: usize = crate::style::parser::MAX_COMPONENT_DEPTH + 1;
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

/// Each count of `a` and `b`, saturating.
fn add_specificity(a: u32, b: u32) -> u32 {
    SPECIFICITY_SHIFTS
        .iter()
        .map(|shift| {
            (((a >> shift) & TYPE_SPECIFICITY_MASK) + ((b >> shift) & TYPE_SPECIFICITY_MASK))
                .min(TYPE_SPECIFICITY_MASK)
                << shift
        })
        .sum()
}

/// A pseudo-element selector followed by pseudo-classes (Selectors 4 §3.6.3).
#[derive(Clone)]
struct PseudoStates {
    /// The pseudo-element selector and its pseudo-classes as written.
    source: Box<str>,
    /// `*` with them, whose subject is the pseudo-element of any element.
    selector: Selector,
}

#[derive(Clone)]
pub(super) struct RuleSelector {
    selector: Selector,
    specificity: u32,
    pseudo: Option<PseudoElement>,
    states: Option<PseudoStates>,
}
impl RuleSelector {
    fn parse_context(
        source: &str,
        namespaces: &NamespaceMap,
        mode: SelectorMode,
        sheet: SelectorOrigin,
    ) -> Result<Self, StyleError> {
        let mut source_input = ParserInput::new(source);
        let Originating {
            origin,
            pseudo,
            states,
        } = originating_selector(&mut Parser::new(&mut source_input))
            .map_err(|error| StyleError::Invalid(format!("selector {source}: {error:?}")))?;
        let invalid = |error| StyleError::Invalid(format!("selector {source}: {error:?}"));
        let selector =
            Selector::parse_context(&origin, namespaces, true, mode, sheet).map_err(invalid)?;
        let states = states
            .map(|states| PseudoStates::parse(states, sheet))
            .transpose()?;
        Self::from_parsed(selector, pseudo, states)
    }
    fn from_parsed(
        selector: Selector,
        pseudo: Option<PseudoElement>,
        states: Option<PseudoStates>,
    ) -> Result<Self, StyleError> {
        selector_depth(selector.depth())?;
        let single = |selector: &Selector| {
            selector
                .single_specificity()
                .ok_or_else(|| StyleError::Invalid("expected one selector".into()))
        };
        let mut specificity = single(&selector)?;
        if let Some(states) = &states {
            specificity = add_specificity(specificity, single(&states.selector)?);
        } else if let Some(pseudo) = pseudo {
            specificity = add_specificity(specificity, pseudo.compounds());
        }
        Ok(Self {
            selector,
            specificity,
            pseudo,
            states,
        })
    }
    pub(super) fn parent_list(selectors: &[Self]) -> Result<Selector, StyleError> {
        Selector::list(
            &selectors
                .iter()
                .map(|selector| match selector.pseudo {
                    Some(pseudo) => {
                        let source = selector
                            .states
                            .as_ref()
                            .map_or_else(|| pseudo.source(), |states| &states.source);
                        selector
                            .selector
                            .with_pseudo_element(source)
                            .ok_or_else(|| {
                                StyleError::Invalid(format!("nesting selector for {source}"))
                            })
                    }
                    None => Ok(selector.selector.clone()),
                })
                .collect::<Result<Vec<_>, _>>()?,
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
        Self::from_parsed(
            self.selector.resolve_parent(parent),
            self.pseudo,
            self.states.clone(),
        )
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
    /// `ancestors`, if given, holds the hashes of `element`'s shadow-including ancestors.
    pub(super) fn matches(
        &self,
        element: &Element<'_>,
        pseudo: Option<PseudoElement>,
        scope: crate::dom::SelectorScope,
        tree: TreeContext,
        ancestors: Option<&selectors::bloom::BloomFilter>,
    ) -> Option<u32> {
        (self.pseudo == pseudo
            && element.matches_in_selector_scope(&self.selector, tree, scope, ancestors)
            && self
                .states
                .as_ref()
                .is_none_or(|states| states.selector.matches_pseudo_element_states(*element)))
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
    sheet: SelectorOrigin,
    forgiving: bool,
) -> Result<Vec<RuleSelector>, ParseError<'i, StyleError>> {
    input.parse_comma_separated(|input| {
        let source = raw(input)?;
        let selector = Selector::parse_scope_boundary(&source, namespaces, forgiving, mode, sheet)
            .map_err(|error| {
                input.new_custom_error(StyleError::Invalid(format!("scope selector: {error:?}")))
            })?;
        RuleSelector::from_parsed(selector, None, None)
            .map_err(|error| input.new_custom_error(error))
    })
}

/// A pseudo-element after its first colon; the CSS 2 ones also take a single colon.
fn pseudo_element<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<PseudoElement, ParseError<'i, StyleError>> {
    let (name, double_colon) = match input.next_including_whitespace()?.clone() {
        Token::Colon => (input.next_including_whitespace()?.clone(), true),
        token => (token, false),
    };
    let not_pseudo = || StyleError::Invalid("not a pseudo-element".into());
    match name {
        Token::Ident(name) => PseudoElement::parse(&name, double_colon)
            .ok_or_else(|| input.new_custom_error(not_pseudo())),
        Token::Function(name) if double_colon => input.parse_nested_block(|arguments| {
            let pseudo = PseudoElement::parse_functional(&name, arguments)
                .map_err(|error| error.location.new_custom_error(not_pseudo()))?;
            arguments.expect_exhausted()?;
            Ok(pseudo)
        }),
        token => Err(input.new_unexpected_token_error(token)),
    }
}

impl PseudoStates {
    /// The pseudo-element selector and pseudo-classes `source`, which the selector grammar
    /// admits only where they may follow it.
    fn parse(source: &str, sheet: SelectorOrigin) -> Result<Self, StyleError> {
        let subject = format!("*{source}");
        let selector = Selector::parse_context(
            &subject,
            &NamespaceMap::new(),
            false,
            SelectorMode::Normal,
            sheet,
        )
        .map_err(|error| StyleError::Invalid(format!("selector {source}: {error:?}")))?;
        Ok(Self {
            selector,
            source: source.into(),
        })
    }
}

/// A selector split at its pseudo-element: the compound selectors of its originating element,
/// the pseudo-element, and the pseudo-element selector with the pseudo-classes after it, if
/// any follow.
struct Originating<'i> {
    origin: String,
    pseudo: Option<PseudoElement>,
    states: Option<&'i str>,
}

fn originating_selector<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<Originating<'i>, ParseError<'i, StyleError>> {
    let start = input.position();
    let mut universal = true;
    while !input.is_exhausted() {
        let state = input.state();
        match input.next_including_whitespace()?.clone() {
            Token::Colon => {
                if let Ok(mut pseudo) = input.try_parse(pseudo_element) {
                    // A real backing element is the subject of `::details-content`.
                    if pseudo == PseudoElement::DetailsContent {
                        universal = false;
                        continue;
                    }
                    if let Ok(name) = input.try_parse(|p| {
                        p.expect_colon()?;
                        p.expect_colon()?;
                        p.expect_ident_cloned()
                    }) {
                        pseudo = pseudo.sub(&name).ok_or_else(|| {
                            input.new_custom_error::<_, StyleError>(StyleError::Invalid(format!(
                                "{pseudo:?} has no ::{name} sub-pseudo-element"
                            )))
                        })?;
                    }
                    let mut origin = input.slice(start..state.position()).to_owned();
                    if universal {
                        origin.push('*');
                    }
                    let classes = input.position();
                    while !input.is_exhausted() {
                        if let Token::Function(_)
                        | Token::ParenthesisBlock
                        | Token::SquareBracketBlock = input.next_including_whitespace()?
                        {
                            input.parse_nested_block(consume)?;
                        }
                    }
                    let states =
                        (input.position() != classes).then(|| input.slice_from(state.position()));
                    return Ok(Originating {
                        origin,
                        pseudo: Some(pseudo),
                        states,
                    });
                }
                universal = false;
            }
            Token::WhiteSpace(_) | Token::Delim('>' | '+' | '~') => universal = true,
            // A column combinator, unlike a lone namespace separator, ends a compound.
            Token::Delim('|') => {
                universal = input
                    .try_parse(|input| {
                        matches!(input.next_including_whitespace(), Ok(Token::Delim('|')))
                            .then_some(())
                            .ok_or(())
                    })
                    .is_ok();
            }
            Token::Function(_) | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                input.parse_nested_block(consume)?;
                universal = false;
            }
            _ => universal = false,
        }
    }
    Ok(Originating {
        origin: input.slice_from(start).to_owned(),
        pseudo: None,
        states: None,
    })
}
pub(super) fn parse<'i>(
    input: &mut Parser<'i, '_>,
    namespaces: &NamespaceMap,
    mode: SelectorMode,
    sheet: SelectorOrigin,
) -> Result<Vec<RuleSelector>, ParseError<'i, StyleError>> {
    input.parse_comma_separated(|input| {
        let source = raw(input)?;
        RuleSelector::parse_context(&source, namespaces, mode, sheet)
            .map_err(|error| input.new_custom_error(error))
    })
}

/// `selector()` (CSS Conditional 4 §5.2) tests the selector syntax author style sheets take.
pub(super) fn supports_selectors_with_namespaces(
    source: &str,
    list: bool,
    namespaces: &NamespaceMap,
) -> bool {
    supports_selectors(
        source,
        list,
        namespaces,
        SelectorMode::Normal,
        SelectorOrigin::Author,
    )
}
pub(super) fn supports_context_selectors(
    source: &str,
    namespaces: &NamespaceMap,
    mode: SelectorMode,
    sheet: SelectorOrigin,
) -> bool {
    supports_selectors(source, true, namespaces, mode, sheet)
}
fn supports_selectors(
    source: &str,
    list: bool,
    namespaces: &NamespaceMap,
    mode: SelectorMode,
    sheet: SelectorOrigin,
) -> bool {
    if !matches!(balanced_components(source), Ok(true)) {
        return false;
    }
    let mut input = ParserInput::new(source);
    let parsed = Parser::new(&mut input).parse_comma_separated(|input| {
        let Originating { origin, states, .. } = originating_selector(input)?;
        let invalid =
            |error| input.new_custom_error(StyleError::Invalid(format!("selector: {error:?}")));
        if let Some(states) = states {
            PseudoStates::parse(states, sheet).map_err(|error| input.new_custom_error(error))?;
        }
        Selector::parse_context(&origin, namespaces, false, mode, sheet)
            .map_err(invalid)
            .and_then(|selector| {
                selector_depth(selector.depth()).map_err(|error| input.new_custom_error(error))
            })
    });
    parsed.is_ok_and(|selectors| list || selectors.len() == 1)
}
