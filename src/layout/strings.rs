use crate::document::{RunningStringAssignment, SemanticEvent};
use crate::dom::ElementId;
use crate::layout::page_counters::PageCounters;
use crate::style::{NamedStringMode, NamedStringRequest, PageStyleTarget, StyleError};
use std::{collections::HashMap, ops::Range};

#[derive(Clone, Default)]
struct PageStart {
    first: Option<Range<usize>>,
    closed: bool,
}
#[derive(Clone, Copy)]
struct Location {
    page: usize,
    start: bool,
}
#[derive(Clone)]
enum SemanticAction {
    Element(SemanticEvent),
    Text,
}
#[derive(Clone, Default)]
pub(super) struct StringLocations {
    trace: Option<Vec<Vec<SemanticAction>>>,
    locations: HashMap<PageStyleTarget, usize>,
    assignments: HashMap<usize, Location>,
    starts: HashMap<usize, PageStart>,
    /// Footnote calls in recording order with their pages (CSS GCPM 3 §2.2).
    footnotes: Vec<(usize, usize)>,
    /// The first and last elements in document order recorded on each page, by their tree
    /// positions.
    bounds: HashMap<usize, [(usize, ElementId); 2]>,
}
impl StringLocations {
    pub(crate) fn last_page(&self) -> usize {
        self.locations
            .values()
            .chain(self.starts.keys())
            .copied()
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn retain_column_events(&mut self) {
        self.trace = Some(Vec::new());
    }
    fn trace(&mut self, page: usize, action: SemanticAction) {
        if let Some(trace) = &mut self.trace {
            if trace.len() <= page {
                trace.resize_with(page + 1, Vec::new);
            }
            trace[page].push(action);
        }
    }
    pub(crate) fn adopt_column(&mut self, source: &mut Self, column: usize, page: usize) {
        let Some(events) = source
            .trace
            .as_mut()
            .and_then(|trace| trace.get_mut(column))
        else {
            return;
        };
        for event in std::mem::take(events) {
            match event {
                SemanticAction::Element(event) => self.record(page, &event),
                SemanticAction::Text => self.text(page),
            }
        }
    }

    pub(crate) fn record(&mut self, page: usize, event: &SemanticEvent) {
        if self.locations.contains_key(&event.context) {
            return;
        }
        self.locations.insert(event.context, page);
        self.footnotes
            .extend(event.footnote.map(|footnote| (footnote, page)));
        self.trace(page, SemanticAction::Element(event.clone()));
        // A pseudo-element's position matters only where it assigns a running element.
        let pseudo = event.context.pseudo.is_some();
        if pseudo && event.assignment.is_none() {
            return;
        }
        if !pseudo {
            let position = (event.order.start, event.context.element);
            let [first, last] = self.bounds.entry(page).or_insert([position; 2]);
            if position.0 < first.0 {
                *first = position;
            }
            if position.0 > last.0 {
                *last = position;
            }
        }
        let state = self.starts.entry(page).or_default();
        let start = event.visible
            && !state.closed
            && state.first.as_ref().is_none_or(|first| {
                first.start <= event.order.start && event.order.end <= first.end
            });
        if start {
            state.first = Some(event.order.clone());
        }
        if let Some(assignment) = event.assignment {
            self.assignments
                .entry(assignment)
                .or_insert(Location { page, start });
        }
    }
    /// The footnote calls recorded after the first `start`, with their pages.
    pub(crate) fn footnote_calls(&self, start: usize) -> &[(usize, usize)] {
        self.footnotes.get(start..).unwrap_or_default()
    }
    pub(crate) fn text(&mut self, page: usize) {
        let state = self.starts.entry(page).or_default();
        if !state.closed {
            state.closed = true;
            self.trace(page, SemanticAction::Text);
        }
    }
    pub(crate) fn resolve(
        self,
        assignments: &[RunningStringAssignment],
        numbers: &PageCounters,
        pages: usize,
        degradations: &crate::style::Degradations,
    ) -> Result<PageStrings, StyleError> {
        if self.assignments.len() != assignments.len() {
            return Err(StyleError::Invalid(
                "running string assignment has no committed page location".into(),
            ));
        }
        let mut output = PageStrings {
            names: HashMap::new(),
            elements: HashMap::new(),
            locations: self.locations,
            bounds: self.bounds,
        };
        for (index, location) in self.assignments {
            if location.page >= pages {
                return Err(StyleError::Invalid(
                    "running string assignment is outside the page sequence".into(),
                ));
            }
            if let Some(element) = &assignments[index].element {
                output
                    .elements
                    .entry(element.name.clone())
                    .or_default()
                    .push(PageValue {
                        page: location.page,
                        owner: index,
                        start: location.start,
                        value: (),
                    });
            }
            for assignment in &assignments[index].values {
                let value = assignment.content.bound_page_output(
                    numbers.page(location.page, None),
                    pages,
                    crate::document::MAX_INLINE_TEXT_BYTES,
                )?;
                crate::document::record_text_limit(degradations, value.limited());
                output
                    .names
                    .entry(assignment.name.clone())
                    .or_default()
                    .push(PageValue {
                        page: location.page,
                        owner: index,
                        start: location.start,
                        value: value.into_text(),
                    });
            }
        }
        for values in output.names.values_mut() {
            values.sort_by_key(|value| (value.page, value.owner));
        }
        for values in output.elements.values_mut() {
            values.sort_by_key(|value| (value.page, value.owner));
        }
        Ok(output)
    }
}
struct PageValue<T> {
    page: usize,
    owner: usize,
    start: bool,
    value: T,
}
impl<T> PageValue<T> {
    /// The assignment a page uses (CSS GCPM 3 §1.1.2): `None` is the empty value.
    fn select(values: &[Self], page: usize, mode: NamedStringMode) -> Option<&Self> {
        let first = values.partition_point(|value| value.page < page);
        let end = values.partition_point(|value| value.page <= page);
        let entry = first.checked_sub(1).map(|index| &values[index]);
        let current = &values[first..end];
        match mode {
            NamedStringMode::First => current.first().or(entry),
            NamedStringMode::Start => current.first().filter(|value| value.start).or(entry),
            NamedStringMode::Last => current.last().or(entry),
            NamedStringMode::FirstExcept if !current.is_empty() => None,
            NamedStringMode::FirstExcept => entry,
        }
    }
}
pub(super) struct PageStrings {
    names: HashMap<std::sync::Arc<str>, Vec<PageValue<String>>>,
    /// Running elements by name, with their owners' assignment indices.
    elements: HashMap<std::sync::Arc<str>, Vec<PageValue<()>>>,
    locations: HashMap<PageStyleTarget, usize>,
    bounds: HashMap<usize, [(usize, ElementId); 2]>,
}
impl PageStrings {
    /// The page a context lies on; a pseudo-element without a recorded position, such as
    /// `::marker`, lies where its element starts.
    pub(crate) fn page_of(&self, context: PageStyleTarget) -> Option<usize> {
        self.locations.get(&context).copied().or_else(|| {
            self.locations
                .get(&PageStyleTarget {
                    pseudo: None,
                    ..context
                })
                .copied()
        })
    }
    /// Where each page starts in the document: at the first element in document order laid
    /// out on it, or else inside the last element to start on an earlier page (`true`).
    pub(crate) fn page_starts(&self, pages: usize) -> Vec<Option<(ElementId, bool)>> {
        let mut last: Option<(usize, ElementId)> = None;
        (0..pages)
            .map(|page| {
                let bounds = self.bounds.get(&page);
                let start = match bounds {
                    Some([(_, first), _]) => Some((*first, false)),
                    None => last.map(|(_, element)| (element, true)),
                };
                if let Some([_, page_last]) = bounds
                    && last.is_none_or(|(order, _)| page_last.0 > order)
                {
                    last = Some(*page_last);
                }
                start
            })
            .collect()
    }
    pub(crate) fn at_context(
        &self,
        context: PageStyleTarget,
        request: &NamedStringRequest,
    ) -> Result<String, StyleError> {
        let page = self.page_of(context).ok_or_else(|| {
            StyleError::Invalid(format!(
                "named string {:?} has no committed use location",
                request.name
            ))
        })?;
        Ok(self.text(page, request))
    }

    pub(crate) fn text(&self, page: usize, request: &NamedStringRequest) -> String {
        self.names
            .get(request.name.as_ref())
            .and_then(|values| PageValue::select(values, page, request.mode))
            .map_or_else(String::new, |value| value.value.clone())
    }

    /// The assignment index of the running element `element()` places on a page (CSS GCPM 3
    /// §1.2.2), which selects among assignments as `string()` does.
    pub(crate) fn element(&self, page: usize, request: &NamedStringRequest) -> Option<usize> {
        self.elements
            .get(request.name.as_ref())
            .and_then(|values| PageValue::select(values, page, request.mode))
            .map(|value| value.owner)
    }
}
