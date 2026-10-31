//! Page-based counters (CSS Page 3 §6.1): the `page` counter and the counters that page and
//! page-margin contexts reset, increment or set, which advance with each page box.
use crate::document::{CounterSnapshot, PageBox, PageStart};
use crate::dom::ElementId;
use crate::style::{
    AutomaticInitial, CounterProperties, CounterRequest, FOOTNOTE_COUNTER, INITIAL_COUNTER_VALUE,
    MarginLocation, PAGE_COUNTER, PAGES_COUNTER,
};
use std::sync::Arc;

/// Each page's page-context counters and the counters of its page-margin boxes.
pub(super) struct PageCounters {
    pages: Vec<PageValues>,
}
struct PageValues {
    context: CounterSnapshot,
    margins: Vec<(MarginLocation, CounterSnapshot)>,
}

impl PageCounters {
    pub(super) fn new(page_boxes: &[Arc<PageBox>]) -> Self {
        // `page` advances by one on every page unless the page context names its increment.
        let contexts: Vec<_> = page_boxes
            .iter()
            .map(|page| page.counters.with_implicit_increment(PAGE_COUNTER))
            .collect();
        let mut pages: Vec<_> = sequence(
            &contexts,
            vec![(Arc::from(PAGE_COUNTER), INITIAL_COUNTER_VALUE)],
        )
        .into_iter()
        .map(|context| PageValues {
            context,
            margins: Vec::new(),
        })
        .collect();
        // A margin context's counters are in scope for its own box only, and advance with
        // each page that generates that box.
        for location in MarginLocation::ALL {
            let (indices, contexts): (Vec<usize>, Vec<CounterProperties>) = page_boxes
                .iter()
                .enumerate()
                .filter_map(|(index, page)| {
                    page.margins
                        .iter()
                        .find(|margin| margin.margin.location == location)
                        .map(|margin| (index, margin.margin.style.counters.clone()))
                })
                .unzip();
            if contexts.iter().all(CounterProperties::is_empty) {
                continue;
            }
            for (index, values) in indices.into_iter().zip(sequence(&contexts, Vec::new())) {
                pages[index].margins.push((location, values));
            }
        }
        Self { pages }
    }

    /// The `page` counter of a page, as its page context or one of its page-margin boxes sees
    /// it.
    pub(super) fn page(&self, page: usize, location: Option<MarginLocation>) -> i128 {
        let values = &self.pages[page];
        location
            .and_then(|location| values.margin(location))
            .and_then(|margin| margin.value(PAGE_COUNTER))
            .or_else(|| values.context.value(PAGE_COUNTER))
            .expect("the page context instantiates the page counter")
    }

    /// The text of `counter()` or `counters()` in a page-margin box: the box's own counter,
    /// else the page context's, else the document's at the start of the page.
    pub(super) fn text(
        &self,
        page: usize,
        location: MarginLocation,
        request: &CounterRequest,
        document: &CounterSnapshot,
    ) -> crate::style::Representation {
        let values = &self.pages[page];
        let name = request.name.as_ref();
        values
            .page_context(location, name)
            .unwrap_or(document)
            .text(request)
    }

    /// Where each page starts in the document and the page context's changes to the
    /// footnote counter there, when document counters depend on them: the page context
    /// changes the footnote counter (CSS GCPM 3 §2.5.2), or a page-margin box shows a
    /// document counter (CSS Page 3 §6.1).
    pub(super) fn starts(
        &self,
        page_boxes: &[Arc<PageBox>],
        elements: Vec<Option<(ElementId, bool)>>,
    ) -> Vec<(ElementId, PageStart)> {
        let footnotes: Vec<_> = page_boxes
            .iter()
            .map(|page| page.counters.named(FOOTNOTE_COUNTER))
            .collect();
        let margins_read_document = page_boxes.iter().enumerate().any(|(page, page_box)| {
            page_box.margins.iter().any(|margin| {
                margin.margin.content.counter_requests().any(|request| {
                    self.pages[page]
                        .page_context(margin.margin.location, &request.name)
                        .is_none()
                })
            })
        });
        if !margins_read_document && footnotes.iter().all(CounterProperties::is_empty) {
            return Vec::new();
        }
        elements
            .into_iter()
            .zip(footnotes)
            .enumerate()
            .filter_map(|(page, (element, footnote))| {
                let (element, inside) = element?;
                Some((
                    element,
                    PageStart {
                        page,
                        inside,
                        footnote,
                        automatic: Vec::new(),
                    },
                ))
            })
            .collect()
    }
}
impl PageValues {
    fn margin(&self, location: MarginLocation) -> Option<&CounterSnapshot> {
        self.margins
            .iter()
            .find(|(margin, _)| *margin == location)
            .map(|(_, values)| values)
    }
    /// The counters holding `name` that a page-margin box sees before the document's: its
    /// own, which obscure the page context's.
    fn page_context(&self, location: MarginLocation, name: &str) -> Option<&CounterSnapshot> {
        self.margin(location)
            .filter(|margin| margin.value(name).is_some())
            .or_else(|| self.context.value(name).is_some().then_some(&self.context))
    }
}

/// The counters after each of a sequence of sibling contexts, which share one counter of each
/// name: a later reset replaces it (CSS Lists 3 §4.4). `pages` keeps the page total.
fn sequence(
    contexts: &[CounterProperties],
    mut counters: Vec<(Arc<str>, i128)>,
) -> Vec<CounterSnapshot> {
    let counted = |name: &Arc<str>| name.as_ref() != PAGES_COUNTER;
    contexts
        .iter()
        .enumerate()
        .map(|(index, properties)| {
            for reset in properties.reset.iter().filter(|reset| counted(&reset.name)) {
                let value = reset
                    .value
                    .unwrap_or_else(|| automatic(&contexts[index..], &reset.name));
                *counter(&mut counters, &reset.name) = value;
            }
            for change in properties
                .increment
                .iter()
                .filter(|change| counted(&change.name))
            {
                let value = counter(&mut counters, &change.name);
                *value = value.saturating_add(change.value);
            }
            for change in properties.set.iter().filter(|change| counted(&change.name)) {
                *counter(&mut counters, &change.name) = change.value;
            }
            CounterSnapshot::new(counters.clone())
        })
        .collect()
}

/// The counter named `name`, instantiated at zero when absent.
fn counter<'c>(counters: &'c mut Vec<(Arc<str>, i128)>, name: &Arc<str>) -> &'c mut i128 {
    let index = counters
        .iter()
        .position(|(counter, _)| counter == name)
        .unwrap_or_else(|| {
            counters.push((name.clone(), INITIAL_COUNTER_VALUE));
            counters.len() - 1
        });
    &mut counters[index].1
}

/// The initial value of a reversed counter reset without one, from the contexts in its scope:
/// those from its reset up to the next reset of the same name.
fn automatic(contexts: &[CounterProperties], name: &str) -> i128 {
    let mut initial = AutomaticInitial::default();
    for (index, properties) in contexts.iter().enumerate() {
        if index > 0
            && properties
                .reset
                .iter()
                .any(|reset| reset.name.as_ref() == name)
        {
            break;
        }
        let increment = properties
            .increment
            .iter()
            .filter(|change| change.name.as_ref() == name)
            .fold(INITIAL_COUNTER_VALUE, |sum, change| {
                sum.saturating_add(change.value)
            });
        let set = properties
            .set
            .iter()
            .rev()
            .find(|change| change.name.as_ref() == name)
            .map(|change| change.value);
        initial.update(increment, set);
    }
    initial.value()
}
