use ferri_pdf_document::{RunningStringAssignment, SemanticEvent};
use ferri_pdf_style::{NamedStringMode, NamedStringRequest, PageStyleTarget, StyleError};
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
        self.trace(page, SemanticAction::Element(event.clone()));
        if event.context.pseudo.is_some() {
            return;
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
        pages: usize,
    ) -> Result<PageStrings, StyleError> {
        if self.assignments.len() != assignments.len() {
            return Err(StyleError::Invalid(
                "running string assignment has no committed page location".into(),
            ));
        }
        let mut output = PageStrings {
            names: HashMap::new(),
            locations: self.locations,
        };
        for (index, location) in self.assignments {
            if location.page >= pages {
                return Err(StyleError::Invalid(
                    "running string assignment is outside the page sequence".into(),
                ));
            }
            for assignment in &assignments[index].values {
                let text = assignment.content.page_text(
                    location.page + 1,
                    pages,
                    ferri_pdf_document::MAX_INLINE_TEXT_BYTES,
                )?;
                output
                    .names
                    .entry(assignment.name.clone())
                    .or_default()
                    .push(PageValue {
                        page: location.page,
                        owner: index,
                        start: location.start,
                        text,
                    });
            }
        }
        for values in output.names.values_mut() {
            values.sort_by_key(|value| (value.page, value.owner));
        }
        Ok(output)
    }
}
struct PageValue {
    page: usize,
    owner: usize,
    start: bool,
    text: String,
}
pub(super) struct PageStrings {
    names: HashMap<std::sync::Arc<str>, Vec<PageValue>>,
    locations: HashMap<PageStyleTarget, usize>,
}
impl PageStrings {
    pub(crate) fn at_context(
        &self,
        context: PageStyleTarget,
        request: &NamedStringRequest,
    ) -> Result<String, StyleError> {
        let page = self.locations.get(&context).ok_or_else(|| {
            StyleError::Invalid(format!(
                "named string {:?} has no committed use location",
                request.name
            ))
        })?;
        Ok(self.text(*page, request))
    }

    pub(crate) fn text(&self, page: usize, request: &NamedStringRequest) -> String {
        let Some(values) = self.names.get(request.name.as_ref()) else {
            return String::new();
        };
        let first = values.partition_point(|value| value.page < page);
        let end = values.partition_point(|value| value.page <= page);
        let entry = first
            .checked_sub(1)
            .map_or("", |index| values[index].text.as_str());
        let current = &values[first..end];
        match request.mode {
            NamedStringMode::First => current.first().map_or(entry, |value| value.text.as_str()),
            NamedStringMode::Start => current
                .first()
                .filter(|value| value.start)
                .map_or(entry, |value| value.text.as_str()),
            NamedStringMode::Last => current.last().map_or(entry, |value| value.text.as_str()),
            NamedStringMode::FirstExcept if !current.is_empty() => "",
            NamedStringMode::FirstExcept => entry,
        }
        .to_owned()
    }
}
