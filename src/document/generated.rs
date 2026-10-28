use crate::document::{
    MAX_INLINE_TEXT_BYTES,
    content::{ElementText, GeneratedText},
    counters::{CounterSnapshot, Counters},
    navigation::FragmentIndex,
};
use ferri_pdf_dom::{DocumentTree, Element, ElementId};
use ferri_pdf_style::{
    GeneratedContent, NamedStringRequest, PageStyleTarget, PseudoElement, Style, StyleError,
    TargetRequest, TargetValue,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct GeneratedValues {
    targets: HashMap<TargetRequest, String>,
    strings: HashMap<(PageStyleTarget, NamedStringRequest), String>,
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
                requests: Rc::default(),
            },
            elements: HashMap::new(),
        }
    }
    pub(crate) fn bindings(&self) -> GeneratedBindings {
        self.bindings.clone()
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
        pseudo: PseudoElement,
        text: &str,
        style: &Style,
    ) {
        let Some(target) = self.elements.get_mut(&element.id()) else {
            return;
        };
        let generated = GeneratedText::capture(text, style);
        match pseudo {
            PseudoElement::Before => target.before = generated,
            PseudoElement::After => target.after = generated,
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
        pages: usize,
        page: &mut dyn FnMut(ElementId) -> Option<usize>,
        strings: &mut dyn FnMut(PageStyleTarget, &NamedStringRequest) -> Result<String, StyleError>,
    ) -> Result<GeneratedValues, StyleError> {
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
                    let number = page(*owner).ok_or_else(|| {
                        StyleError::Invalid(format!(
                            "target {:?} has no committed page",
                            request.reference
                        ))
                    })?;
                    counter.page_text(number, pages).expect("page counter")
                }
                TargetValue::Counter(counter) => target.counters.text(counter)?,
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
        Ok(GeneratedValues {
            targets: values,
            strings: named,
        })
    }
    pub(crate) fn matches(&self, values: &GeneratedValues) -> bool {
        self.bindings.values.as_ref() == values
    }
}
