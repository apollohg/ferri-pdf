use crate::dom::{Directionality, Element, ElementId, QuirksMode, TreeRelation, TreeScopeId};
use cssparser::{CowRcStr, ParseError, Parser, ParserInput, SourceLocation, ToCss, Token};
use html5ever::Namespace;
use scraper::{
    ElementRef,
    selector::{CssLocalName, CssString},
};
use selectors::{
    Element as SelectorElement, OpaqueElement,
    attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint},
    bloom::BloomFilter,
    matching::{self, MatchingContext, SelectorCaches},
    parser::{self, ParseRelative, SelectorList, SelectorParseErrorKind},
    visitor::{SelectorListKind, SelectorVisitor},
};
use std::{collections::HashMap, fmt};

/// The empty key binds the default element namespace; values are literal CSS strings.
pub(crate) type NamespaceMap = HashMap<String, String>;
/// Parses the namespace binding shared by stylesheet consumers.
pub(crate) fn parse_namespace<'i>(
    input: &mut Parser<'i, '_>,
) -> Result<(String, String), cssparser::BasicParseError<'i>> {
    let prefix = input
        .try_parse(Parser::expect_ident_cloned)
        .map(|prefix| prefix.to_string())
        .unwrap_or_default();
    let namespace = input.expect_url_or_string()?.to_string();
    input.expect_exhausted()?;
    Ok((prefix, namespace))
}
pub(crate) type SelectorError<'a> = ParseError<'a, SelectorParseErrorKind<'a>>;
const PART_ATTRIBUTE: &str = "part";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScopeRoot {
    Element(ElementId),
    Document,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SelectorScope {
    pub root: Option<ScopeRoot>,
    pub origin: Option<ElementId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum TreeContext {
    Document,
    Root(TreeScopeId),
    All,
}
impl TreeContext {
    fn host(self, element: Element<'_>) -> Option<Element<'_>> {
        match self {
            Self::Root(root) => element.scope_host(root),
            Self::Document | Self::All => None,
        }
    }
    fn contains(self, element: Element<'_>) -> bool {
        match self {
            Self::Document => element.styled_by_document(),
            Self::Root(root) => {
                element.stylesheet_scope() == root
                    || self
                        .host(element)
                        .is_some_and(|host| host.id() == element.id())
            }
            Self::All => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectorMode {
    Normal,
    Nesting,
    Scope,
}
impl SelectorMode {
    fn relative(self) -> ParseRelative {
        match self {
            Self::Normal => ParseRelative::No,
            Self::Nesting => ParseRelative::ForNesting,
            Self::Scope => ParseRelative::ForScope,
        }
    }
}

const ROOT_SELECTOR_DEPTH: usize = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SelectorMetrics {
    depth: usize,
    parent_depth: Option<usize>,
    components: usize,
    parent_count: usize,
}
impl SelectorMetrics {
    fn parsed(selectors: &SelectorList<StaticSelector>) -> Self {
        let mut visitor = MetricsVisitor {
            depth: ROOT_SELECTOR_DEPTH,
            metrics: Self::default(),
        };
        for selector in selectors.slice() {
            selector.visit(&mut visitor);
        }
        visitor.metrics
    }
    fn merge(self, other: Self) -> Self {
        Self {
            depth: self.depth.max(other.depth),
            parent_depth: self
                .parent_depth
                .into_iter()
                .chain(other.parent_depth)
                .max(),
            components: self.components.saturating_add(other.components),
            parent_count: self.parent_count.saturating_add(other.parent_count),
        }
    }
    fn resolved(self, parent: Self) -> Self {
        Self {
            depth: self.depth.max(
                self.parent_depth
                    .map_or(0, |depth| depth.saturating_add(parent.depth)),
            ),
            parent_depth: self
                .parent_depth
                .zip(parent.parent_depth)
                .map(|(depth, parent)| depth.saturating_add(parent)),
            components: self
                .components
                .saturating_add(self.parent_count.saturating_mul(parent.components)),
            parent_count: self.parent_count.saturating_mul(parent.parent_count),
        }
    }
}
struct MetricsVisitor {
    depth: usize,
    metrics: SelectorMetrics,
}
impl MetricsVisitor {
    fn nested<'a>(
        &mut self,
        mut selectors: impl Iterator<Item = &'a parser::Selector<StaticSelector>>,
    ) -> bool {
        self.depth += 1;
        let result = selectors.all(|selector| selector.visit(self));
        self.depth -= 1;
        result
    }
}
impl SelectorVisitor for MetricsVisitor {
    type Impl = StaticSelector;
    fn visit_simple_selector(&mut self, component: &parser::Component<StaticSelector>) -> bool {
        self.metrics.depth = self.metrics.depth.max(self.depth);
        self.metrics.components = self.metrics.components.saturating_add(1);
        if matches!(component, parser::Component::ParentSelector) {
            self.metrics.parent_count = self.metrics.parent_count.saturating_add(1);
            self.metrics.parent_depth = Some(
                self.metrics
                    .parent_depth
                    .unwrap_or_default()
                    .max(self.depth),
            );
        }
        true
    }
    fn visit_selector_list(
        &mut self,
        _: SelectorListKind,
        selectors: &[parser::Selector<StaticSelector>],
    ) -> bool {
        self.nested(selectors.iter())
    }
    fn visit_relative_selector_list(
        &mut self,
        selectors: &[parser::RelativeSelector<StaticSelector>],
    ) -> bool {
        self.nested(selectors.iter().map(|selector| &selector.selector))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Selector {
    selectors: SelectorList<StaticSelector>,
    metrics: SelectorMetrics,
}
impl Selector {
    pub(crate) fn parse(input: &str) -> Result<Self, SelectorError<'_>> {
        Self::parse_with_namespaces(input, &NamespaceMap::new(), true)
    }

    pub(crate) fn parse_with_namespaces<'i>(
        input: &'i str,
        namespaces: &NamespaceMap,
        forgiving: bool,
    ) -> Result<Self, SelectorError<'i>> {
        Self::parse_context(input, namespaces, forgiving, SelectorMode::Normal)
    }

    pub(crate) fn parse_context<'i>(
        input: &'i str,
        namespaces: &NamespaceMap,
        forgiving: bool,
        mode: SelectorMode,
    ) -> Result<Self, SelectorError<'i>> {
        Self::parse_unresolved(input, namespaces, forgiving, mode.relative(), true)
            .map(|selector| selector.bind_context(mode))
    }

    pub(crate) fn parse_scope_boundary<'i>(
        input: &'i str,
        namespaces: &NamespaceMap,
        forgiving: bool,
        mode: SelectorMode,
    ) -> Result<Self, SelectorError<'i>> {
        Self::parse_unresolved(input, namespaces, forgiving, mode.relative(), false)
            .map(|selector| selector.bind_context(mode))
    }

    fn bind_context(self, mode: SelectorMode) -> Self {
        if mode == SelectorMode::Nesting {
            return self;
        }
        self.resolve_parent(&Self {
            selectors: SelectorList::implicit_scope(),
            metrics: SelectorMetrics {
                depth: ROOT_SELECTOR_DEPTH,
                components: 1,
                ..Default::default()
            },
        })
    }

    fn parse_unresolved<'i>(
        input: &'i str,
        namespaces: &NamespaceMap,
        forgiving: bool,
        relative: ParseRelative,
        allow_pseudo: bool,
    ) -> Result<Self, SelectorError<'i>> {
        let parser = &StaticParser {
            namespaces,
            forgiving,
        };
        let mut source = ParserInput::new(input);
        let input = &mut Parser::new(&mut source);
        let parsed = if allow_pseudo {
            SelectorList::parse(parser, input, relative)
        } else {
            SelectorList::parse_disallow_pseudo(parser, input, relative)
        };
        parsed.map(|selectors| Self {
            metrics: SelectorMetrics::parsed(&selectors),
            selectors,
        })
    }

    pub(crate) fn list(selectors: &[Self]) -> Option<Self> {
        if selectors.is_empty() {
            return None;
        }
        let metrics = selectors
            .iter()
            .fold(SelectorMetrics::default(), |metrics, selector| {
                metrics.merge(selector.metrics)
            });
        let selectors: Vec<_> = selectors
            .iter()
            .flat_map(|selector| selector.selectors.slice().iter().cloned())
            .collect();
        Some(Self {
            selectors: SelectorList::from_iter(selectors.into_iter()),
            metrics,
        })
    }

    pub(crate) fn nesting_work(&self, parent_work: usize) -> usize {
        parent_work.saturating_mul(self.metrics.components)
    }
    pub(crate) fn depth(&self) -> usize {
        self.metrics.depth
    }
    pub(crate) fn resolved_depth(&self, parent: &Self) -> usize {
        self.metrics.resolved(parent.metrics).depth
    }

    #[must_use]
    pub(crate) fn resolve_parent(&self, parent: &Self) -> Self {
        Self {
            selectors: self.selectors.replace_parent_selector(&parent.selectors),
            metrics: self.metrics.resolved(parent.metrics),
        }
    }

    /// `pseudo` of the elements this selector matches, as a nested rule's `&` represents it;
    /// `None` for the sub-pseudo-elements, which no single compound selector can follow.
    pub(crate) fn with_pseudo_element(&self, pseudo: PseudoElement) -> Option<Self> {
        let source = format!("&{}", pseudo.source());
        Self::parse_unresolved(&source, &NamespaceMap::new(), true, ParseRelative::No, true)
            .ok()
            .map(|selector| selector.resolve_parent(self))
    }

    pub(crate) fn single_specificity(&self) -> Option<u32> {
        let selectors = self.selectors.slice();
        (selectors.len() == 1).then(|| selectors[0].specificity())
    }

    #[cfg(test)]
    pub(crate) fn matches(
        &self,
        element: Element<'_>,
        caches: &mut SelectorCaches,
        scope: Option<ScopeRoot>,
    ) -> bool {
        self.matches_context(
            element,
            caches,
            SelectorScope {
                root: scope,
                origin: None,
            },
            TreeContext::Root(element.tree_scope()),
        )
    }

    pub(crate) fn matches_context(
        &self,
        element: Element<'_>,
        caches: &mut SelectorCaches,
        scope: SelectorScope,
        tree_context: TreeContext,
    ) -> bool {
        let host = tree_context.host(element);
        let is_host = host.is_some_and(|host| host.id() == element.id());
        let visible = match tree_context {
            TreeContext::Document => element.styled_by_document(),
            TreeContext::Root(root) => element.stylesheet_scope() == root || is_host,
            TreeContext::All => true,
        };
        if !visible
            && !self
                .selectors
                .slice()
                .iter()
                .any(|selector| selector.is_part() || selector.is_slotted())
        {
            return false;
        }
        let quirks = match element.tree.quirks_mode() {
            QuirksMode::Quirks => matching::QuirksMode::Quirks,
            QuirksMode::LimitedQuirks => matching::QuirksMode::LimitedQuirks,
            QuirksMode::NoQuirks => matching::QuirksMode::NoQuirks,
        };
        let mut context = MatchingContext::new(
            matching::MatchingMode::Normal,
            None,
            caches,
            quirks,
            matching::NeedsSelectorFlags::No,
            matching::MatchingForInvalidation::No,
        );
        context.current_host = host.map(|host| MatchingElement::new(host, false).opaque());
        let virtual_root = scope.root == Some(ScopeRoot::Document);
        context.scope_element = scope.root.and_then(|scope| match scope {
            ScopeRoot::Element(id) => element
                .tree
                .element(id)
                .map(|element| MatchingElement::new(element, false).opaque()),
            ScopeRoot::Document => Some(
                MatchingElement {
                    element: element.tree.root_element(),
                    virtual_root: true,
                    document: true,
                }
                .opaque(),
            ),
        });
        let element = MatchingElement::new(element, virtual_root);
        context.with_featureless(is_host, |context| {
            self.selectors.slice().iter().any(|selector| {
                if selector.is_part() || selector.is_slotted() {
                    matching::originating_elements(selector, &element, context.current_host)
                        .filter(|origin| {
                            tree_context.contains(origin.element)
                                && scope.origin.is_none_or(|id| origin.element.id() == id)
                        })
                        .any(|origin| {
                            matching::matches_selector_from_origin(
                                selector, 0, None, &element, &origin, context,
                            )
                        })
                } else {
                    visible
                        && scope.origin.is_none_or(|id| element.element.id() == id)
                        && matching::matches_selector(selector, 0, None, &element, context)
                }
            })
        })
    }

    /// Origins and extra alias hops for a single selector's scope proximity.
    pub(crate) fn scope_origins<'a>(
        &self,
        element: Element<'a>,
        tree_context: TreeContext,
    ) -> impl Iterator<Item = (Element<'a>, usize)> + Clone + use<'a> {
        let host = tree_context
            .host(element)
            .map(|host| MatchingElement::new(host, false).opaque());
        let selector = self
            .selectors
            .slice()
            .first()
            .filter(|_| self.selectors.slice().len() == 1);
        let hops = usize::from(
            selector.is_some_and(|selector| selector.is_part() || selector.is_slotted()),
        );
        let origins = selector.map(|selector| {
            matching::originating_elements(selector, &MatchingElement::new(element, false), host)
        });
        origins.into_iter().flatten().filter_map(move |origin| {
            tree_context
                .contains(origin.element)
                .then_some((origin.element, hops))
        })
    }
}

struct StaticParser<'a> {
    namespaces: &'a NamespaceMap,
    forgiving: bool,
}
impl<'i> parser::Parser<'i> for StaticParser<'_> {
    type Impl = StaticSelector;
    type Error = SelectorParseErrorKind<'i>;

    fn parse_is_and_where(&self) -> bool {
        true
    }
    fn parse_has(&self) -> bool {
        true
    }
    fn parse_host(&self) -> bool {
        true
    }
    fn parse_slotted(&self) -> bool {
        true
    }
    fn parse_part(&self) -> bool {
        true
    }
    fn parse_parent_selector(&self) -> bool {
        true
    }
    fn parse_pseudo_element(
        &self,
        location: SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<PseudoElement, SelectorError<'i>> {
        PseudoElement::parse(&name, true).ok_or_else(|| {
            location.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(
                name,
            ))
        })
    }
    fn parse_nth_child_of(&self) -> bool {
        true
    }
    fn allow_forgiving_selectors(&self) -> bool {
        self.forgiving
    }
    fn default_namespace(&self) -> Option<Namespace> {
        self.namespaces.get("").map(|name| name.as_str().into())
    }
    fn namespace_for_prefix(&self, prefix: &CssLocalName) -> Option<Namespace> {
        self.namespaces
            .get(prefix.0.as_ref())
            .map(|name| name.as_str().into())
    }
    fn parse_non_ts_pseudo_class(
        &self,
        location: SourceLocation,
        name: CowRcStr<'i>,
    ) -> Result<StaticState, SelectorError<'i>> {
        if name.eq_ignore_ascii_case("heading") {
            return Ok(StaticState::Heading(None));
        }
        let state = match name.to_ascii_lowercase().as_str() {
            "defined" => Some(StaticState::Defined),
            "link" | "any-link" => Some(StaticState::Link),
            "target" => Some(StaticState::Target),
            "local-link" => Some(StaticState::LocalLink(None)),
            "blank" => Some(StaticState::Blank),
            "enabled" => Some(StaticState::Enabled),
            "disabled" => Some(StaticState::Disabled),
            "has-slotted" => Some(StaticState::HasSlotted),
            "paused" => Some(StaticState::Paused),
            "muted" => Some(StaticState::Muted),
            _ => FormState::parse(&name).map(StaticState::Form),
        };
        if let Some(state) = state {
            return Ok(state);
        }
        if let Some(state) = InactiveState::parse(&name) {
            return Ok(StaticState::Inactive(state));
        }
        Err(
            location.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(
                name,
            )),
        )
    }
    fn parse_non_ts_functional_pseudo_class<'t>(
        &self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _: bool,
    ) -> Result<StaticState, SelectorError<'i>> {
        if name.eq_ignore_ascii_case("heading") {
            return input
                .parse_comma_separated(|input| {
                    let token = input.next()?.clone();
                    match token {
                        Token::Number {
                            int_value: Some(level),
                            ..
                        } => Ok(level),
                        _ => Err(input.new_unexpected_token_error(token)),
                    }
                })
                .map(|levels| StaticState::Heading(Some(levels)));
        }
        if name.eq_ignore_ascii_case("state") {
            return Ok(StaticState::CustomState(
                input.expect_ident_cloned()?.to_string(),
            ));
        }
        if name.eq_ignore_ascii_case("dir") {
            return Ok(StaticState::Direction(
                input.expect_ident_cloned()?.to_ascii_lowercase(),
            ));
        }
        if name.eq_ignore_ascii_case("lang") {
            return input
                .parse_comma_separated(|input| Ok(input.expect_ident_or_string()?.to_string()))
                .map(StaticState::Language);
        }
        if name.eq_ignore_ascii_case("current") {
            let selectors = SelectorList::parse(self, input, ParseRelative::No)?;
            if selectors.slice().iter().any(|selector| {
                selector
                    .iter_raw_match_order()
                    .any(parser::Component::is_combinator)
            }) {
                return Err(input.new_custom_error(
                    SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name),
                ));
            }
            return Ok(StaticState::Current(Box::new(selectors)));
        }
        if let Some(last) = [("nth-col", false), ("nth-last-col", true)]
            .into_iter()
            .find_map(|(column, last)| name.eq_ignore_ascii_case(column).then_some(last))
        {
            let (a, b) = cssparser::parse_nth(input)?;
            return Ok(StaticState::NthColumn {
                position: parser::AnPlusB(a, b),
                last,
            });
        }
        if name.eq_ignore_ascii_case("local-link") {
            let token = input.next()?.clone();
            return match token {
                Token::Number {
                    int_value: Some(value),
                    ..
                } if value >= 0 => Ok(StaticState::LocalLink(Some(value as u32))),
                _ => Err(input.new_unexpected_token_error(token)),
            };
        }
        Err(
            input.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(
                name,
            )),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StaticSelector;
impl parser::SelectorImpl for StaticSelector {
    type AttrValue = CssString;
    type Identifier = CssLocalName;
    type LocalName = CssLocalName;
    type NamespacePrefix = CssLocalName;
    type NamespaceUrl = Namespace;
    type BorrowedNamespaceUrl = Namespace;
    type BorrowedLocalName = CssLocalName;
    type NonTSPseudoClass = StaticState;
    type PseudoElement = PseudoElement;
    type ExtraMatchingData<'a> = ();
}

/// States no element of a static rendering is in: nothing is hovered, focused or visited,
/// no timeline plays, no media plays or seeks (HTML §4.16.3: `:playing`, `:buffering`,
/// `:stalled` and `:seeking` need playback, and this user agent never locks the volume),
/// nothing is fullscreen or in picture-in-picture without a script request, and nothing is
/// scrolled to a snap position (CSS Scroll Snap 2 §4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InactiveState {
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    Visited,
    InterestSource,
    InterestTarget,
    Current,
    Past,
    Future,
    Playing,
    Buffering,
    Stalled,
    Seeking,
    VolumeLocked,
    Fullscreen,
    PictureInPicture,
    Snapped,
    SnappedX,
    SnappedY,
    SnappedBlock,
    SnappedInline,
}
impl InactiveState {
    const NAMES: [(&str, Self); 23] = [
        ("hover", Self::Hover),
        ("active", Self::Active),
        ("focus", Self::Focus),
        ("focus-visible", Self::FocusVisible),
        ("focus-within", Self::FocusWithin),
        ("visited", Self::Visited),
        ("interest-source", Self::InterestSource),
        ("interest-target", Self::InterestTarget),
        ("current", Self::Current),
        ("past", Self::Past),
        ("future", Self::Future),
        ("playing", Self::Playing),
        ("buffering", Self::Buffering),
        ("stalled", Self::Stalled),
        ("seeking", Self::Seeking),
        ("volume-locked", Self::VolumeLocked),
        ("fullscreen", Self::Fullscreen),
        ("picture-in-picture", Self::PictureInPicture),
        ("snapped", Self::Snapped),
        ("snapped-x", Self::SnappedX),
        ("snapped-y", Self::SnappedY),
        ("snapped-block", Self::SnappedBlock),
        ("snapped-inline", Self::SnappedInline),
    ];
    fn parse(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(candidate, _)| name.eq_ignore_ascii_case(candidate))
            .map(|(_, state)| *state)
    }
    fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, state)| *state == self)
            .map(|(name, _)| *name)
            .expect("every inactive state has a name")
    }
}

/// Form, collapse and top-layer states (Selectors 4 §11–§12, HTML §4.16.3) as the document's
/// initial state has them: no user has interacted with a control, and without script no
/// dialog is modal, no popover shows and nothing is autofilled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FormState {
    Checked,
    Unchecked,
    Indeterminate,
    Default,
    PlaceholderShown,
    Autofill,
    WebkitAutofill,
    Valid,
    Invalid,
    UserValid,
    UserInvalid,
    InRange,
    OutOfRange,
    Required,
    Optional,
    ReadOnly,
    ReadWrite,
    Open,
    Modal,
    PopoverOpen,
}
impl FormState {
    const NAMES: [(&str, Self); 20] = [
        ("checked", Self::Checked),
        ("unchecked", Self::Unchecked),
        ("indeterminate", Self::Indeterminate),
        ("default", Self::Default),
        ("placeholder-shown", Self::PlaceholderShown),
        ("autofill", Self::Autofill),
        ("-webkit-autofill", Self::WebkitAutofill),
        ("valid", Self::Valid),
        ("invalid", Self::Invalid),
        ("user-valid", Self::UserValid),
        ("user-invalid", Self::UserInvalid),
        ("in-range", Self::InRange),
        ("out-of-range", Self::OutOfRange),
        ("required", Self::Required),
        ("optional", Self::Optional),
        ("read-only", Self::ReadOnly),
        ("read-write", Self::ReadWrite),
        ("open", Self::Open),
        ("modal", Self::Modal),
        ("popover-open", Self::PopoverOpen),
    ];
    fn parse(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(candidate, _)| name.eq_ignore_ascii_case(candidate))
            .map(|(_, state)| *state)
    }
    fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, state)| *state == self)
            .map(|(name, _)| *name)
            .expect("every form state has a name")
    }
    fn matches(self, element: Element<'_>) -> bool {
        match self {
            Self::Checked => element.is_checked(),
            Self::Unchecked => element.is_checkable() && !element.is_checked(),
            Self::Indeterminate => element.is_indeterminate(),
            Self::Default => element.is_default(),
            Self::PlaceholderShown => element.shown_placeholder().is_some(),
            Self::Valid => element.validity() == Some(true),
            Self::Invalid => element.validity() == Some(false),
            Self::InRange => element.range_state() == Some(true),
            Self::OutOfRange => element.range_state() == Some(false),
            Self::Required => element.required_state() == Some(true),
            Self::Optional => element.required_state() == Some(false),
            Self::ReadWrite => element.is_read_write(),
            Self::ReadOnly => {
                element.namespace() == crate::dom::HTML_NAMESPACE && !element.is_read_write()
            }
            Self::Open => element.is_open(),
            // User validity starts false and only user interaction sets it (HTML §4.10.18.4).
            Self::Autofill
            | Self::WebkitAutofill
            | Self::UserValid
            | Self::UserInvalid
            | Self::Modal
            | Self::PopoverOpen => false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StaticState {
    Defined,
    Link,
    Target,
    LocalLink(Option<u32>),
    Direction(String),
    Language(Vec<String>),
    Blank,
    Enabled,
    Disabled,
    HasSlotted,
    Current(Box<SelectorList<StaticSelector>>),
    Heading(Option<Vec<i32>>),
    /// HTML §4.16.3: media elements are paused until playback starts, which a static
    /// rendering never does.
    Paused,
    /// HTML §4.8.11.13: a media element is muted from its `muted` content attribute.
    Muted,
    /// `:state()` (Selectors 5 §5.2): custom element states, which only script adds.
    CustomState(String),
    /// `:nth-col()` and `:nth-last-col()` (Selectors 5 §8.2–§8.3): a table cell in a column
    /// at an `An+B` position, counted from the last column when `last`.
    NthColumn {
        position: parser::AnPlusB,
        last: bool,
    },
    Inactive(InactiveState),
    Form(FormState),
}
impl parser::NonTSPseudoClass for StaticState {
    type Impl = StaticSelector;
    fn has_parent_selector(&self) -> bool {
        matches!(self, Self::Current(selectors) if selectors.slice().iter().any(parser::Selector::has_parent_selector))
    }
    fn has_scope_selector(&self) -> bool {
        matches!(self, Self::Current(selectors) if selectors.slice().iter().any(parser::Selector::has_scope_selector))
    }
    fn replace_parent_selector(&self, parent: &SelectorList<StaticSelector>) -> Option<Self> {
        match self {
            Self::Current(selectors) if self.has_parent_selector() => Some(Self::Current(
                Box::new(selectors.replace_parent_selector(parent)),
            )),
            _ => None,
        }
    }
    fn visit<V: SelectorVisitor<Impl = StaticSelector>>(&self, visitor: &mut V) -> bool {
        match self {
            Self::Current(selectors) => {
                visitor.visit_selector_list(SelectorListKind::empty(), selectors.slice())
            }
            _ => true,
        }
    }
    fn is_active_or_hover(&self) -> bool {
        matches!(
            self,
            Self::Inactive(InactiveState::Active | InactiveState::Hover)
        )
    }
    fn is_user_action_state(&self) -> bool {
        matches!(
            self,
            Self::Inactive(
                InactiveState::Active
                    | InactiveState::Hover
                    | InactiveState::Focus
                    | InactiveState::FocusVisible
                    | InactiveState::FocusWithin
                    | InactiveState::InterestSource
                    | InactiveState::InterestTarget
            )
        )
    }
}
impl ToCss for StaticState {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        match self {
            Self::Defined => dest.write_str(":defined"),
            Self::Blank => dest.write_str(":blank"),
            Self::Enabled => dest.write_str(":enabled"),
            Self::Disabled => dest.write_str(":disabled"),
            Self::HasSlotted => dest.write_str(":has-slotted"),
            Self::Paused => dest.write_str(":paused"),
            Self::Muted => dest.write_str(":muted"),
            Self::CustomState(name) => {
                dest.write_str(":state(")?;
                cssparser::serialize_identifier(name, dest)?;
                dest.write_str(")")
            }
            Self::Current(selectors) => {
                dest.write_str(":current(")?;
                selectors.to_css(dest)?;
                dest.write_str(")")
            }
            Self::Link => dest.write_str(":any-link"),
            Self::Target => dest.write_str(":target"),
            Self::LocalLink(None) => dest.write_str(":local-link"),
            Self::LocalLink(Some(value)) => write!(dest, ":local-link({value})"),
            Self::Direction(value) => write!(dest, ":dir({value})"),
            Self::Language(ranges) => {
                dest.write_str(":lang(")?;
                for (index, range) in ranges.iter().enumerate() {
                    if index != 0 {
                        dest.write_str(", ")?;
                    }
                    cssparser::serialize_string(range, dest)?;
                }
                dest.write_str(")")
            }
            Self::NthColumn { position, last } => {
                dest.write_str(if *last { ":nth-last-col(" } else { ":nth-col(" })?;
                position.to_css(dest)?;
                dest.write_str(")")
            }
            Self::Inactive(state) => write!(dest, ":{}", state.name()),
            Self::Form(state) => write!(dest, ":{}", state.name()),
            Self::Heading(None) => dest.write_str(":heading"),
            Self::Heading(Some(levels)) => {
                dest.write_str(":heading(")?;
                for (index, level) in levels.iter().enumerate() {
                    if index != 0 {
                        dest.write_str(", ")?;
                    }
                    write!(dest, "{level}")?;
                }
                dest.write_str(")")
            }
        }
    }
}

/// The `::before` and `::after` pseudo-elements, which generate content boxes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum GeneratedPseudoElement {
    Before,
    After,
}

/// Tree-abiding and typographic pseudo-elements (CSS Pseudo-Elements 4 §§2, 4).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum PseudoElement {
    Before,
    After,
    Marker,
    FirstLine,
    FirstLetter,
    /// `::first-letter::prefix`, the punctuation preceding the first letter.
    FirstLetterPrefix,
    /// `::first-letter::suffix`, the punctuation following the first letter.
    FirstLetterSuffix,
    /// The element-backed pseudo-elements of form controls (CSS Pseudo-Elements 4 §4.2,
    /// §4.3), which style parts of a control's internal content.
    Placeholder,
    FileSelectorButton,
    Absent(AbsentPseudoElement),
}
impl From<GeneratedPseudoElement> for PseudoElement {
    fn from(pseudo: GeneratedPseudoElement) -> Self {
        match pseudo {
            GeneratedPseudoElement::Before => Self::Before,
            GeneratedPseudoElement::After => Self::After,
        }
    }
}
/// Pseudo-elements that a static rendering never generates, so that rules for them style
/// nothing. Highlights (CSS Pseudo-Elements 4 §3) paint only active highlights: nothing is
/// selected or found in the page, no script registers custom highlights, no navigation
/// applies a text directive, and this user agent checks no spelling or grammar. Without
/// script the top layer is empty, so no element has a `::backdrop` (CSS Positioned Layout
/// 4 §3.1). `::column` takes only scroll snap properties (CSS Multi-column Layout 2 §4.6),
/// which static media ignore. `::nth-fragment()` styles fragment boxes, which only a
/// `continue: fragments` element generates, and the `continue` grammar has no such value
/// (CSS Overflow 4 §5.3).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AbsentPseudoElement {
    Selection,
    TargetText,
    SpellingError,
    GrammarError,
    SearchText,
    /// `::highlight()`: every custom highlight name is unregistered, so all are alike.
    Highlight,
    Backdrop,
    Column,
    NthFragment,
}
impl AbsentPseudoElement {
    const NAMES: [(&str, Self); 7] = [
        ("selection", Self::Selection),
        ("target-text", Self::TargetText),
        ("spelling-error", Self::SpellingError),
        ("grammar-error", Self::GrammarError),
        ("search-text", Self::SearchText),
        ("backdrop", Self::Backdrop),
        ("column", Self::Column),
    ];
    fn source(self) -> &'static str {
        match self {
            Self::Selection => "::selection",
            Self::TargetText => "::target-text",
            Self::SpellingError => "::spelling-error",
            Self::GrammarError => "::grammar-error",
            Self::SearchText => "::search-text",
            Self::Highlight => "::highlight(unregistered)",
            Self::Backdrop => "::backdrop",
            Self::Column => "::column",
            Self::NthFragment => "::nth-fragment(n)",
        }
    }
}

/// The CSS 2 pseudo-elements, which also take the single-colon syntax (Selectors 4 §16).
const LEGACY_PSEUDO_ELEMENTS: [&str; 4] = ["before", "after", "first-line", "first-letter"];
impl PseudoElement {
    /// The pseudo-element `name` after `::`, or after `:` for the CSS 2 pseudo-elements.
    pub(crate) fn parse(name: &str, double_colon: bool) -> Option<Self> {
        if !double_colon
            && !LEGACY_PSEUDO_ELEMENTS
                .iter()
                .any(|legacy| name.eq_ignore_ascii_case(legacy))
        {
            return None;
        }
        [
            ("before", Self::Before),
            ("after", Self::After),
            ("marker", Self::Marker),
            ("first-line", Self::FirstLine),
            ("first-letter", Self::FirstLetter),
            ("placeholder", Self::Placeholder),
            ("file-selector-button", Self::FileSelectorButton),
        ]
        .into_iter()
        .chain(
            AbsentPseudoElement::NAMES
                .into_iter()
                .map(|(name, pseudo)| (name, Self::Absent(pseudo))),
        )
        .find_map(|(known, pseudo)| name.eq_ignore_ascii_case(known).then_some(pseudo))
    }
    /// The functional pseudo-element `::name(arguments)`.
    pub(crate) fn parse_functional<'i>(
        name: &str,
        arguments: &mut Parser<'i, '_>,
    ) -> Result<Self, ParseError<'i, ()>> {
        if name.eq_ignore_ascii_case("highlight") {
            arguments.expect_ident()?;
            return Ok(Self::Absent(AbsentPseudoElement::Highlight));
        }
        if name.eq_ignore_ascii_case("nth-fragment") {
            cssparser::parse_nth(arguments)?;
            return Ok(Self::Absent(AbsentPseudoElement::NthFragment));
        }
        Err(arguments.new_custom_error(()))
    }
    pub(crate) fn is_element_backed(self) -> bool {
        matches!(self, Self::Placeholder | Self::FileSelectorButton)
    }
    /// The sub-pseudo-element `::name` of this one, which only `::first-letter` has (CSS
    /// Pseudo-Elements 4 §2.2).
    pub(crate) fn sub(self, name: &str) -> Option<Self> {
        match self {
            Self::FirstLetter if name.eq_ignore_ascii_case("prefix") => {
                Some(Self::FirstLetterPrefix)
            }
            Self::FirstLetter if name.eq_ignore_ascii_case("suffix") => {
                Some(Self::FirstLetterSuffix)
            }
            _ => None,
        }
    }
    /// The pseudo-element selectors the compound holds, each adding to its specificity.
    pub(crate) fn compounds(self) -> u32 {
        match self {
            Self::FirstLetterPrefix | Self::FirstLetterSuffix => 2,
            _ => 1,
        }
    }
    fn source(self) -> &'static str {
        match self {
            Self::Before => "::before",
            Self::After => "::after",
            Self::Marker => "::marker",
            Self::FirstLine => "::first-line",
            Self::FirstLetter => "::first-letter",
            Self::FirstLetterPrefix => "::first-letter::prefix",
            Self::FirstLetterSuffix => "::first-letter::suffix",
            Self::Placeholder => "::placeholder",
            Self::FileSelectorButton => "::file-selector-button",
            Self::Absent(pseudo) => pseudo.source(),
        }
    }
}
impl parser::PseudoElement for PseudoElement {
    type Impl = StaticSelector;
    fn valid_after_slotted(&self) -> bool {
        !self.is_element_backed()
    }
    fn is_before_or_after(&self) -> bool {
        !self.is_element_backed()
    }
    fn parses_as_element_backed(&self) -> bool {
        self.is_element_backed()
    }
}
impl ToCss for PseudoElement {
    fn to_css<W: fmt::Write>(&self, dest: &mut W) -> fmt::Result {
        dest.write_str(self.source())
    }
}

#[derive(Clone, Copy, Debug)]
// A virtual document retains its tree through the document element.
struct MatchingElement<'a> {
    element: Element<'a>,
    virtual_root: bool,
    document: bool,
}
impl<'a> MatchingElement<'a> {
    fn new(element: Element<'a>, virtual_root: bool) -> Self {
        Self {
            element,
            virtual_root,
            document: false,
        }
    }
    fn with_element(&self, element: Element<'a>) -> Self {
        Self::new(element, self.virtual_root)
    }
}
impl SelectorElement for MatchingElement<'_> {
    type Impl = StaticSelector;
    fn opaque(&self) -> OpaqueElement {
        if self.document {
            OpaqueElement::new(
                self.element
                    .raw
                    .parent()
                    .expect("the document element has a document parent")
                    .value(),
            )
        } else {
            OpaqueElement::new(self.element.as_node().value())
        }
    }
    fn parent_element(&self) -> Option<Self> {
        if self.document {
            return None;
        }
        self.element
            .parent(TreeRelation::Selector)
            .map(|parent| self.with_element(parent))
            .or_else(|| {
                (self.virtual_root
                    && self
                        .element
                        .raw
                        .parent()
                        .is_some_and(|parent| parent.value().is_document()))
                .then_some(Self {
                    document: true,
                    ..*self
                })
            })
    }
    fn is_virtual_scope_root(&self) -> bool {
        self.document
    }
    fn parent_node_is_shadow_root(&self) -> bool {
        !self.document
            && self.element.shadow_host().is_some()
            && self
                .element
                .raw
                .parent()
                .is_some_and(|parent| parent.value().is_fragment())
    }
    fn containing_shadow_host(&self) -> Option<Self> {
        (!self.document)
            .then(|| self.element.shadow_host())
            .flatten()
            .map(|host| self.with_element(host))
    }
    fn is_pseudo_element(&self) -> bool {
        false
    }
    fn prev_sibling_element(&self) -> Option<Self> {
        if self.document {
            return None;
        }
        self.element
            .raw
            .prev_siblings()
            .find_map(ElementRef::wrap)
            .map(|raw| {
                self.with_element(Element {
                    tree: self.element.tree,
                    raw,
                })
            })
    }
    fn next_sibling_element(&self) -> Option<Self> {
        if self.document {
            return None;
        }
        self.element
            .raw
            .next_siblings()
            .find_map(ElementRef::wrap)
            .map(|raw| {
                self.with_element(Element {
                    tree: self.element.tree,
                    raw,
                })
            })
    }
    fn column_elements(&self) -> Vec<Self> {
        if self.document {
            return Vec::new();
        }
        self.element
            .column_elements()
            .into_iter()
            .map(|column| self.with_element(column))
            .collect()
    }
    fn first_element_child(&self) -> Option<Self> {
        if self.document {
            return Some(self.with_element(self.element));
        }
        self.element
            .children(TreeRelation::Selector)
            .next()
            .map(|child| self.with_element(child))
    }
    fn is_html_element_in_html_document(&self) -> bool {
        !self.document && self.element.is_html_element_in_html_document()
    }
    fn has_local_name(&self, name: &CssLocalName) -> bool {
        !self.document && self.element.value().name.local == name.0
    }
    fn has_namespace(&self, namespace: &Namespace) -> bool {
        !self.document && self.element.value().name.ns == *namespace
    }
    fn is_same_type(&self, other: &Self) -> bool {
        !self.document
            && !other.document
            && self.element.name() == other.element.name()
            && self.element.namespace() == other.element.namespace()
    }
    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&Namespace>,
        local_name: &CssLocalName,
        operation: &AttrSelectorOperation<&CssString>,
    ) -> bool {
        !self.document
            && self.element.value().attrs.iter().any(|(name, value)| {
                !matches!(*ns, NamespaceConstraint::Specific(url) if *url != name.ns)
                    && local_name.0 == name.local
                    && operation.eval_str(value)
            })
    }
    fn match_non_ts_pseudo_class(
        &self,
        state: &StaticState,
        _: &mut MatchingContext<'_, StaticSelector>,
    ) -> bool {
        if self.document {
            return false;
        }
        let element = self.element;
        match state {
            StaticState::Defined => element.is_defined(),
            StaticState::Blank => element.is_blank_control(),
            StaticState::Enabled => element.initial_state().disabled == Some(false),
            StaticState::Disabled => element.initial_state().disabled == Some(true),
            StaticState::HasSlotted => element
                .tree
                .shadow
                .populated_slots
                .contains(&element.raw.id()),
            StaticState::Link => element.hyperlink().is_some(),
            StaticState::Target => element.is_target(),
            StaticState::LocalLink(segments) => element.is_local_link(*segments),
            StaticState::Direction(value) => match value.as_str() {
                "ltr" => element.directionality() == Directionality::Ltr,
                "rtl" => element.directionality() == Directionality::Rtl,
                _ => false,
            },
            StaticState::Language(ranges) => ranges
                .iter()
                .any(|range| crate::dom::language::matches(element.language(), range)),
            StaticState::Heading(levels) => element.heading_level().is_some_and(|level| {
                levels
                    .as_ref()
                    .is_none_or(|levels| levels.contains(&i32::from(level)))
            }),
            StaticState::Form(state) => state.matches(element),
            StaticState::NthColumn { position, last } => {
                element.cell_columns().is_some_and(|columns| {
                    columns.range.clone().any(|column| {
                        let index = if *last {
                            columns.count - column
                        } else {
                            column + 1
                        };
                        i32::try_from(index).is_ok_and(|index| position.matches_index(index))
                    })
                })
            }
            StaticState::Paused => element.is_media(),
            StaticState::Muted => element.is_media() && element.attr("muted").is_some(),
            StaticState::Inactive(_) | StaticState::Current(_) | StaticState::CustomState(_) => {
                false
            }
        }
    }
    fn match_pseudo_element(
        &self,
        _: &PseudoElement,
        _: &mut MatchingContext<'_, StaticSelector>,
    ) -> bool {
        false
    }
    fn apply_selector_flags(&self, _: matching::ElementSelectorFlags) {}
    fn is_link(&self) -> bool {
        !self.document && self.element.hyperlink().is_some()
    }
    fn is_html_slot_element(&self) -> bool {
        !self.document
            && self
                .element
                .tree
                .shadow
                .slots
                .contains_key(&self.element.raw.id())
    }
    fn assigned_slot(&self) -> Option<Self> {
        (!self.document)
            .then(|| self.element.alias_slot())
            .flatten()
            .map(|slot| self.with_element(slot))
    }
    fn has_id(&self, id: &CssLocalName, case: CaseSensitivity) -> bool {
        !self.document
            && self
                .element
                .value()
                .id()
                .is_some_and(|value| case.eq(id.0.as_bytes(), value.as_bytes()))
    }
    fn has_class(&self, name: &CssLocalName, case: CaseSensitivity) -> bool {
        !self.document
            && self
                .element
                .value()
                .classes()
                .any(|class| case.eq(class.as_bytes(), name.0.as_bytes()))
    }
    fn has_custom_state(&self, _: &CssLocalName) -> bool {
        false
    }
    fn imported_part(&self, name: &CssLocalName) -> Option<CssLocalName> {
        self.imported_parts(name).into_iter().next()
    }
    fn imported_parts(&self, name: &CssLocalName) -> Vec<CssLocalName> {
        if self.document {
            return Vec::new();
        }
        self.element
            .tree
            .shadow
            .exports
            .get(&self.element.raw.id())
            .and_then(|names| names.get(name.0.as_ref()))
            .into_iter()
            .flatten()
            .map(|name| name.as_str().into())
            .collect()
    }
    fn is_part(&self, name: &CssLocalName) -> bool {
        !self.document
            && self.element.attr(PART_ATTRIBUTE).is_some_and(|parts| {
                parts
                    .split(crate::dom::state::is_html_whitespace)
                    .any(|part| part == name.0.as_ref())
            })
    }
    fn is_empty(&self) -> bool {
        !self.document
            && !self
                .element
                .child_nodes(TreeRelation::Selector)
                .any(|node| {
                    node.element().is_some()
                        || node.value().as_text().is_some_and(|text| {
                            !text.chars().all(crate::dom::state::is_html_whitespace)
                        })
                })
    }
    fn is_root(&self) -> bool {
        !self.document
            && self
                .element
                .raw
                .parent()
                .is_some_and(|parent| parent.value().is_document())
    }
    fn add_element_unique_hashes(&self, _: &mut BloomFilter) -> bool {
        false
    }
}
