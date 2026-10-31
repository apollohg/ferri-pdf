use crate::document::{
    MAX_INLINE_TEXT_BYTES,
    content::{ElementText, GeneratedText},
    counters::{CounterSnapshot, Counters},
    navigation::FragmentIndex,
};
use crate::dom::{DocumentTree, Element, ElementId};
use crate::style::{
    CounterProperties, GeneratedContent, GeneratedPseudoElement, NamedStringRequest,
    PageStyleTarget, Style, StyleError, TargetRequest, TargetValue,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

/// The page counters of document content before pagination places it: the first page's.
const UNPAGINATED_PAGE: i128 = 1;
const UNPAGINATED_TOTAL: usize = 1;

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct GeneratedValues {
    targets: HashMap<TargetRequest, String>,
    strings: HashMap<(PageStyleTarget, NamedStringRequest), String>,
    /// Boxed: layouts of nested navigables stack the values of each enclosing layout.
    paged: Box<PagedValues>,
}

/// What pagination gives document content.
#[derive(Clone, Default, PartialEq, Eq)]
struct PagedValues {
    /// The `page` counter of the page holding each context that shows it (CSS Page 3 §6.1).
    pages: HashMap<PageStyleTarget, i128>,
    /// The page total, once document content shows a page counter.
    total: Option<usize>,
    /// The pages that start at or inside each element.
    starts: HashMap<ElementId, Vec<PageStart>>,
    /// The page counters of the pages whose margin boxes place each running element that
    /// shows them.
    running: HashMap<PageStyleTarget, Vec<PageNumbers>>,
    inline_footnotes: InlineFootnotePlacement,
}

/// Where pagination placed the inline formatting contexts of footnote areas that hold inline
/// footnotes (CSS GCPM 3 §2.3).
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct InlineFootnotePlacement {
    /// The inline footnotes that start an inline formatting context of their own rather than
    /// continue the one of the inline footnote before them. Pagination only adds to them, so
    /// that they settle.
    pub starts: HashSet<ElementId>,
    /// The page each inline formatting context starts on, by its first footnote, unless that
    /// page is the first page's.
    pub pages: HashMap<ElementId, crate::style::PageRequest>,
}

/// The page counters a copy of a running element shows: the `page` counter, the page total,
/// or both, as its content shows them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PageNumbers {
    pub page: Option<i128>,
    pub total: Option<usize>,
}
impl PageNumbers {
    /// Whether these are the page counters of a page whose `page` counter is `page` of
    /// `total` pages, as far as they show them.
    pub(crate) fn matches(self, page: i128, total: usize) -> bool {
        self.page.is_none_or(|value| value == page) && self.total.is_none_or(|value| value == total)
    }
}

/// The page counters a running element's content shows.
#[derive(Clone, Copy, Default)]
struct ShownCounters {
    page: bool,
    total: bool,
}

/// A page's start in the document, where page-context changes reach document counters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageStart {
    pub page: usize,
    /// Whether the page starts inside the element, the last to start before it, rather than
    /// at the element, the first laid out on it.
    pub inside: bool,
    /// The page context's changes to the footnote counter (CSS GCPM 3 §2.5.2).
    pub footnote: CounterProperties,
    /// The initial values of the page context's resets without one, which the previous
    /// formatting found as reversed counters' (CSS Lists 3 §4.4.2).
    pub automatic: Vec<(std::sync::Arc<str>, i128)>,
}

/// Where pagination placed the document, as generated content reads it.
pub(crate) struct Paginated<'a> {
    pub pages: usize,
    /// The `page` counter of each page.
    pub numbers: &'a dyn Fn(usize) -> i128,
    /// The page each recorded context lies on.
    pub context_page: &'a dyn Fn(PageStyleTarget) -> Option<usize>,
    pub strings:
        &'a mut dyn FnMut(PageStyleTarget, &NamedStringRequest) -> Result<String, StyleError>,
    pub starts: Vec<(ElementId, PageStart)>,
    /// The running element each page-margin box places, with the `page` counter it shows.
    pub running: Vec<(PageStyleTarget, i128)>,
    pub inline_footnotes: InlineFootnotePlacement,
}

#[derive(Default)]
struct TargetElement {
    counters: CounterSnapshot,
    before: GeneratedText,
    after: GeneratedText,
}

#[derive(Clone)]
pub(crate) struct GeneratedBindings {
    index: Rc<FragmentIndex>,
    values: Rc<GeneratedValues>,
    strings: Rc<RefCell<HashSet<(PageStyleTarget, NamedStringRequest)>>>,
    pages: Rc<RefCell<HashSet<PageStyleTarget>>>,
    requests: Rc<RefCell<HashMap<TargetRequest, ElementId>>>,
    running: Rc<RefCell<HashMap<PageStyleTarget, ShownCounters>>>,
    /// The page counters content shows when it formats a copy of a running element.
    copy: Option<PageNumbers>,
    degradations: Rc<crate::style::Degradations>,
}
impl GeneratedBindings {
    pub(crate) fn bind<'a>(
        &self,
        content: &'a GeneratedContent,
        context: Option<PageStyleTarget>,
    ) -> Result<std::borrow::Cow<'a, GeneratedContent>, StyleError> {
        let bound = self.bind_references(content, context)?;
        // Document content shows the page counters of the page it lies on (CSS Page 3 §6.1);
        // page-margin content, without a context, renders them on its page.
        if let Some(context) = context
            && content.has_page_counters()
        {
            self.pages.borrow_mut().insert(context);
            return Self::bind_page(
                bound,
                PageNumbers {
                    page: self.values.paged.pages.get(&context).copied(),
                    total: self.values.paged.total,
                },
            );
        }
        Ok(bound)
    }
    /// Binds content inside the running element `owner`, whose copies show the page counters
    /// of the pages that place them (CSS GCPM 3 §1.2.2).
    fn bind_running<'a>(
        &self,
        content: &'a GeneratedContent,
        context: PageStyleTarget,
        owner: PageStyleTarget,
    ) -> Result<std::borrow::Cow<'a, GeneratedContent>, StyleError> {
        let bound = self.bind_references(content, Some(context))?;
        if !content.has_page_counters() {
            return Ok(bound);
        }
        let numbers = self.copy.unwrap_or_else(|| {
            let mut running = self.running.borrow_mut();
            let shown = running.entry(owner).or_default();
            shown.page |= content.shows_page();
            shown.total |= content.shows_pages();
            PageNumbers::default()
        });
        Self::bind_page(bound, numbers)
    }
    fn bind_page(
        content: std::borrow::Cow<'_, GeneratedContent>,
        numbers: PageNumbers,
    ) -> Result<std::borrow::Cow<'_, GeneratedContent>, StyleError> {
        if !content.has_page_counters() {
            return Ok(content);
        }
        Ok(std::borrow::Cow::Owned(content.bind_page_counters(
            MAX_INLINE_TEXT_BYTES,
            numbers.page.unwrap_or(UNPAGINATED_PAGE),
            numbers.total.unwrap_or(UNPAGINATED_TOTAL),
        )?))
    }
    /// Binds target references and named strings.
    fn bind_references<'a>(
        &self,
        content: &'a GeneratedContent,
        context: Option<PageStyleTarget>,
    ) -> Result<std::borrow::Cow<'a, GeneratedContent>, StyleError> {
        let mut bound = std::borrow::Cow::Borrowed(content);
        if content.target_requests().next().is_some() {
            bound = std::borrow::Cow::Owned(content.bind_targets(
                MAX_INLINE_TEXT_BYTES,
                &mut |request| {
                    let owner = match self.index.target(&request.reference) {
                        Ok(owner) => owner,
                        Err(error) => {
                            self.degradations
                                .record(crate::style::Degradation::UnresolvedTarget {
                                    reference: request.reference.to_string(),
                                    reason: error.to_string(),
                                });
                            return Ok(String::new());
                        }
                    };
                    self.requests.borrow_mut().insert(request.clone(), owner);
                    Ok(self
                        .values
                        .targets
                        .get(request)
                        .cloned()
                        .unwrap_or_default())
                },
            )?);
        }
        if let Some(context) = context
            && content.named_string_requests().next().is_some()
        {
            bound = std::borrow::Cow::Owned(bound.bind_named_strings(
                MAX_INLINE_TEXT_BYTES,
                &mut |request| {
                    let key = (context, request.clone());
                    self.strings.borrow_mut().insert(key.clone());
                    Ok(self.values.strings.get(&key).cloned().unwrap_or_default())
                },
            )?);
        }
        Ok(bound)
    }
}

pub(crate) struct GeneratedPass {
    bindings: GeneratedBindings,
    elements: HashMap<ElementId, TargetElement>,
    page_initials: Vec<(usize, std::sync::Arc<str>, i128)>,
}
impl GeneratedPass {
    pub(crate) fn new(
        index: Rc<FragmentIndex>,
        values: GeneratedValues,
        degradations: Rc<crate::style::Degradations>,
    ) -> Self {
        Self {
            bindings: GeneratedBindings {
                index,
                values: Rc::new(values),
                strings: Rc::default(),
                pages: Rc::default(),
                requests: Rc::default(),
                running: Rc::default(),
                copy: None,
                degradations,
            },
            elements: HashMap::new(),
            page_initials: Vec::new(),
        }
    }
    /// The pass formatting a copy of a subtree: it binds content as this one does and
    /// records nothing for the document.
    pub(crate) fn detached(&self) -> Self {
        Self {
            bindings: self.bindings.clone(),
            elements: HashMap::new(),
            page_initials: Vec::new(),
        }
    }
    /// The pass formatting a copy of a running element showing the page counters `numbers`.
    pub(crate) fn running_copy(&self, numbers: PageNumbers) -> Self {
        let mut pass = self.detached();
        pass.bindings.copy = Some(numbers);
        pass
    }
    /// Whether this pass formats a copy of a running element.
    pub(crate) fn formats_copy(&self) -> bool {
        self.bindings.copy.is_some()
    }
    /// The page counters of the pages whose margin boxes place the running element `owner`,
    /// as far as its content shows them.
    pub(crate) fn running_pages(&self, owner: PageStyleTarget) -> &[PageNumbers] {
        self.bindings
            .values
            .paged
            .running
            .get(&owner)
            .map_or(&[], Vec::as_slice)
    }
    /// Binds content inside the running element `owner`.
    pub(crate) fn bind_running<'a>(
        &self,
        content: &'a GeneratedContent,
        context: PageStyleTarget,
        owner: PageStyleTarget,
    ) -> Result<std::borrow::Cow<'a, GeneratedContent>, StyleError> {
        self.bindings.bind_running(content, context, owner)
    }
    /// Binds content inside a running element as its copy showing `numbers` does.
    pub(crate) fn bind_copy<'a>(
        &self,
        content: &'a GeneratedContent,
        context: PageStyleTarget,
        numbers: PageNumbers,
    ) -> Result<std::borrow::Cow<'a, GeneratedContent>, StyleError> {
        GeneratedBindings::bind_page(
            self.bindings.bind_references(content, Some(context))?,
            numbers,
        )
    }
    /// Records the initial values page contexts' resets without one gave the counters they
    /// reset, by page.
    pub(crate) fn page_initials(&mut self, initials: Vec<(usize, std::sync::Arc<str>, i128)>) {
        self.page_initials = initials;
    }
    pub(crate) fn bindings(&self) -> GeneratedBindings {
        self.bindings.clone()
    }
    /// Whether the inline footnote `element` starts an inline formatting context of its own.
    pub(crate) fn starts_inline_footnotes(&self, element: ElementId) -> bool {
        self.bindings
            .values
            .paged
            .inline_footnotes
            .starts
            .contains(&element)
    }
    /// The page the inline formatting context that the inline footnote `element` starts lies
    /// on, once a pagination placed it.
    pub(crate) fn inline_footnote_page(
        &self,
        element: ElementId,
    ) -> Option<&crate::style::PageRequest> {
        self.bindings
            .values
            .paged
            .inline_footnotes
            .pages
            .get(&element)
    }
    /// The pages starting at or inside `element`.
    pub(crate) fn page_starts(&self, element: ElementId) -> &[PageStart] {
        self.bindings
            .values
            .paged
            .starts
            .get(&element)
            .map_or(&[], Vec::as_slice)
    }
    pub(crate) fn bind<'a>(
        &self,
        content: &'a GeneratedContent,
        context: PageStyleTarget,
    ) -> Result<std::borrow::Cow<'a, GeneratedContent>, StyleError> {
        self.bindings.bind(content, Some(context))
    }
    pub(crate) fn capture(&mut self, element: Element<'_>, counters: &Counters) {
        if self.bindings.index.contains(element.id()) {
            self.elements.entry(element.id()).or_default().counters = counters.capture();
        }
    }
    pub(crate) fn generated(
        &mut self,
        element: Element<'_>,
        pseudo: GeneratedPseudoElement,
        text: &str,
        style: &Style,
    ) {
        let Some(target) = self.elements.get_mut(&element.id()) else {
            return;
        };
        let generated = GeneratedText::capture(text, style);
        match pseudo {
            GeneratedPseudoElement::Before => target.before = generated,
            GeneratedPseudoElement::After => target.after = generated,
            // `content()` names only `::before` and `::after` (CSS GCPM 3 §1.1.1).
            GeneratedPseudoElement::FootnoteCall
            | GeneratedPseudoElement::FootnoteMarker
            | GeneratedPseudoElement::Checkmark
            | GeneratedPseudoElement::PickerIcon
            | GeneratedPseudoElement::ScrollMarkerGroup
            | GeneratedPseudoElement::ScrollMarker => {}
        }
    }
    pub(crate) fn finish(self) -> GeneratedReferences {
        GeneratedReferences {
            bindings: self.bindings,
            elements: self.elements,
            page_initials: self.page_initials,
        }
    }
}

pub(crate) struct GeneratedReferences {
    bindings: GeneratedBindings,
    elements: HashMap<ElementId, TargetElement>,
    page_initials: Vec<(usize, std::sync::Arc<str>, i128)>,
}
impl GeneratedReferences {
    pub(crate) fn bind<'a>(
        &self,
        content: &'a GeneratedContent,
        context: PageStyleTarget,
    ) -> Result<std::borrow::Cow<'a, GeneratedContent>, StyleError> {
        self.bindings.bind(content, Some(context))
    }

    pub(crate) fn resolve(
        &self,
        document: &DocumentTree,
        element_page: &mut dyn FnMut(ElementId) -> Option<usize>,
        paginated: Paginated<'_>,
    ) -> Result<GeneratedValues, StyleError> {
        let Paginated {
            pages,
            numbers,
            context_page,
            strings,
            starts,
            running,
            mut inline_footnotes,
        } = paginated;
        let mut values = HashMap::new();
        for (request, owner) in self.bindings.requests.borrow().iter() {
            let target = self.elements.get(owner).ok_or_else(|| {
                StyleError::Invalid(format!(
                    "target {:?} has no captured element state",
                    request.reference
                ))
            })?;
            let text = match &request.value {
                TargetValue::Counter(counter) if counter.is_page_counter() => {
                    let page = element_page(*owner).ok_or_else(|| {
                        StyleError::Invalid(format!(
                            "target {:?} has no committed page",
                            request.reference
                        ))
                    })?;
                    counter
                        .page_text(numbers(page), pages)
                        .expect("page counter")
                }
                TargetValue::Counter(counter) => target.counters.text(counter)?.into_text(),
                TargetValue::Text(source) => ElementText::new(
                    document
                        .element(*owner)
                        .expect("target belongs to this document"),
                    None,
                    &target.before,
                    &target.after,
                    None,
                )
                .resolve(*source)?,
            };
            values.insert(request.clone(), text);
        }
        let mut named = HashMap::new();
        for (context, request) in self.bindings.strings.borrow().iter() {
            named.insert((*context, request.clone()), strings(*context, request)?);
        }
        let contexts = self.bindings.pages.borrow();
        let mut page_numbers = HashMap::new();
        for context in contexts.iter() {
            let page = context_page(*context).ok_or_else(|| {
                StyleError::Invalid("page counter content has no committed page".into())
            })?;
            page_numbers.insert(*context, numbers(page));
        }
        let mut page_starts: HashMap<_, Vec<_>> = HashMap::new();
        for (element, mut start) in starts {
            start.automatic = self
                .page_initials
                .iter()
                .filter(|(page, ..)| *page == start.page)
                .map(|(_, name, value)| (name.clone(), *value))
                .collect();
            page_starts.entry(element).or_default().push(start);
        }
        let shown = self.bindings.running.borrow();
        let mut running_pages: HashMap<_, Vec<_>> = HashMap::new();
        for (owner, page) in running {
            let Some(shown) = shown.get(&owner) else {
                continue;
            };
            let numbers = PageNumbers {
                page: shown.page.then_some(page),
                total: shown.total.then_some(pages),
            };
            let keys = running_pages.entry(owner).or_default();
            if !keys.contains(&numbers) {
                keys.push(numbers);
            }
        }
        Ok(GeneratedValues {
            targets: values,
            strings: named,
            paged: Box::new(PagedValues {
                pages: page_numbers,
                total: (!contexts.is_empty()).then_some(pages),
                starts: page_starts,
                running: running_pages,
                inline_footnotes: {
                    inline_footnotes
                        .starts
                        .extend(&self.bindings.values.paged.inline_footnotes.starts);
                    inline_footnotes
                },
            }),
        })
    }
    pub(crate) fn matches(&self, values: &GeneratedValues) -> bool {
        self.bindings.values.as_ref() == values
    }
    pub(crate) fn values(&self) -> GeneratedValues {
        self.bindings.values.as_ref().clone()
    }
}
