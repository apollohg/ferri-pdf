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
    /// The `page` counter of the page holding each context that shows it (CSS Page 3 §6.1).
    pages: HashMap<PageStyleTarget, i128>,
    /// The page total, once document content shows a page counter.
    total: Option<usize>,
    /// The pages that start at or inside each element.
    starts: HashMap<ElementId, Vec<PageStart>>,
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
}
impl GeneratedBindings {
    pub(crate) fn bind<'a>(
        &self,
        content: &'a GeneratedContent,
        context: Option<PageStyleTarget>,
    ) -> Result<std::borrow::Cow<'a, GeneratedContent>, StyleError> {
        let mut bound = std::borrow::Cow::Borrowed(content);
        if content.target_requests().next().is_some() {
            bound = std::borrow::Cow::Owned(content.bind_targets(
                MAX_INLINE_TEXT_BYTES,
                &mut |request| {
                    let owner = self.index.target(&request.reference)?;
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
        // Document content shows the page counters of the page it lies on (CSS Page 3 §6.1);
        // page-margin content, without a context, renders them on its page.
        if let Some(context) = context
            && content.has_page_counters()
        {
            self.pages.borrow_mut().insert(context);
            bound = std::borrow::Cow::Owned(
                bound.bind_page_counters(
                    MAX_INLINE_TEXT_BYTES,
                    self.values
                        .pages
                        .get(&context)
                        .copied()
                        .unwrap_or(UNPAGINATED_PAGE),
                    self.values.total.unwrap_or(UNPAGINATED_TOTAL),
                )?,
            );
        }
        Ok(bound)
    }
}

pub(crate) struct GeneratedPass {
    bindings: GeneratedBindings,
    elements: HashMap<ElementId, TargetElement>,
}
impl GeneratedPass {
    pub(crate) fn new(index: Rc<FragmentIndex>, values: GeneratedValues) -> Self {
        Self {
            bindings: GeneratedBindings {
                index,
                values: Rc::new(values),
                strings: Rc::default(),
                pages: Rc::default(),
                requests: Rc::default(),
            },
            elements: HashMap::new(),
        }
    }
    pub(crate) fn bindings(&self) -> GeneratedBindings {
        self.bindings.clone()
    }
    /// The pages starting at or inside `element`.
    pub(crate) fn page_starts(&self, element: ElementId) -> &[PageStart] {
        self.bindings
            .values
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
            GeneratedPseudoElement::FootnoteCall | GeneratedPseudoElement::FootnoteMarker => {}
        }
    }
    pub(crate) fn finish(self) -> GeneratedReferences {
        GeneratedReferences {
            bindings: self.bindings,
            elements: self.elements,
        }
    }
}

pub(crate) struct GeneratedReferences {
    bindings: GeneratedBindings,
    elements: HashMap<ElementId, TargetElement>,
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
        for (element, start) in starts {
            page_starts.entry(element).or_default().push(start);
        }
        Ok(GeneratedValues {
            targets: values,
            strings: named,
            pages: page_numbers,
            total: (!contexts.is_empty()).then_some(pages),
            starts: page_starts,
        })
    }
    pub(crate) fn matches(&self, values: &GeneratedValues) -> bool {
        self.bindings.values.as_ref() == values
    }
    pub(crate) fn values(&self) -> GeneratedValues {
        self.bindings.values.as_ref().clone()
    }
}
