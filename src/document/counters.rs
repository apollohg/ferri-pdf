use crate::style::{
    CounterProperties, CounterRequest, DEFAULT_COUNTER_INCREMENT, Display, INITIAL_COUNTER_VALUE,
    LIST_ITEM_COUNTER, PageStyleTarget, StyleError,
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

const MAX_COUNTER_WORK: usize = 10_000_000;
const COUNTER_WORK_RESOURCE: &str = "counter evaluation work";

#[derive(Clone)]
struct Counter {
    name: Arc<str>,
    creator: PageStyleTarget,
    parent: Option<PageStyleTarget>,
    value: Rc<Cell<i128>>,
    reversed: bool,
    automatic: Option<Rc<RefCell<AutomaticInitial>>>,
}
struct Frame {
    target: PageStyleTarget,
    counters: Vec<Counter>,
    sibling: Option<Vec<Counter>>,
}

type CounterKey = (PageStyleTarget, Arc<str>);
#[derive(Default)]
struct AutomaticInitial {
    sum: i128,
    last_increment_negated: i128,
    set: Option<i128>,
}
impl AutomaticInitial {
    fn update(&mut self, increment: i128, set: Option<i128>) {
        if self.set.is_some() {
            return;
        }
        let negated = increment.saturating_neg();
        if negated != 0 {
            self.last_increment_negated = negated;
        }
        if let Some(value) = set {
            self.set = Some(value);
        } else {
            self.sum = self.sum.saturating_add(negated);
        }
    }
    fn value(&self) -> i128 {
        // Cancel the bounded CSS increments before adding a potentially maximal HTML value.
        self.set
            .unwrap_or(INITIAL_COUNTER_VALUE)
            .saturating_add(self.sum.saturating_add(self.last_increment_negated))
    }
}

#[derive(Default)]
pub(crate) struct Counters {
    frames: Vec<Frame>,
    previous: Vec<Counter>,
    work: Rc<Cell<usize>>,
    initials: Option<HashMap<CounterKey, i128>>,
    collecting: bool,
    automatic: Vec<(CounterKey, Rc<RefCell<AutomaticInitial>>)>,
}
impl Counters {
    fn charge(&mut self, amount: usize) -> Result<(), StyleError> {
        self.work.set(self.work.get().saturating_add(amount));
        if self.work.get() > MAX_COUNTER_WORK {
            return Err(StyleError::GrammarLimit {
                resource: COUNTER_WORK_RESOURCE,
                limit: MAX_COUNTER_WORK,
            });
        }
        Ok(())
    }
    pub(crate) fn enter(
        &mut self,
        target: PageStyleTarget,
        properties: &CounterProperties,
        display: &Display,
    ) -> Result<(), StyleError> {
        self.inherit(target)?;
        if !matches!(display, Display::None | Display::Contents) {
            for change in properties.reset.iter() {
                let key = (target, change.name.clone());
                let automatic = if change.value.is_none() && self.collecting {
                    let initial = Rc::new(RefCell::new(AutomaticInitial::default()));
                    self.automatic.push((key.clone(), initial.clone()));
                    Some(initial)
                } else {
                    None
                };
                let value = match change.value {
                    Some(value) => value,
                    None if self.collecting => INITIAL_COUNTER_VALUE,
                    None => *self
                        .initials
                        .as_ref()
                        .and_then(|values| values.get(&key))
                        .ok_or_else(|| {
                            StyleError::Invalid("automatic counter values were not prepared".into())
                        })?,
                };
                self.instantiate(change.name.clone(), value, change.reversed, automatic)?;
            }
            let implicit = if display.is_list_item()
                && !properties
                    .increment
                    .iter()
                    .any(|change| change.name.as_ref() == LIST_ITEM_COUNTER)
            {
                Some(
                    if self
                        .frames
                        .last()
                        .unwrap()
                        .counters
                        .iter()
                        .rev()
                        .find(|counter| counter.name.as_ref() == LIST_ITEM_COUNTER)
                        .is_some_and(|counter| counter.reversed)
                    {
                        -DEFAULT_COUNTER_INCREMENT
                    } else {
                        DEFAULT_COUNTER_INCREMENT
                    },
                )
            } else {
                None
            };
            if self.collecting {
                self.track_updates(properties, implicit)?;
            }
            for change in properties.increment.iter() {
                let value = self.value(change.name.clone())?;
                value.set(value.get().saturating_add(change.value));
            }
            if let Some(increment) = implicit {
                let value = self.value(Arc::from(LIST_ITEM_COUNTER))?;
                value.set(value.get().saturating_add(increment));
            }
            for change in properties.set.iter() {
                self.value(change.name.clone())?.set(change.value);
            }
        }
        self.update_previous()?;
        Ok(())
    }
    fn inherit(&mut self, target: PageStyleTarget) -> Result<(), StyleError> {
        let count = self.frames.last().map_or(0, |parent| {
            parent.sibling.as_ref().unwrap_or(&parent.counters).len()
        });
        self.charge(count.saturating_mul(self.previous.len()))?;
        let counters = self.frames.last().map_or_else(Vec::new, |parent| {
            let source = parent.sibling.as_ref().unwrap_or(&parent.counters);
            self.previous
                .iter()
                .filter(|counter| {
                    source.iter().any(|candidate| {
                        counter.name == candidate.name && counter.creator == candidate.creator
                    })
                })
                .cloned()
                .collect()
        });
        self.frames.push(Frame {
            target,
            counters,
            sibling: None,
        });
        Ok(())
    }
    fn enter_scopes(
        &mut self,
        target: PageStyleTarget,
        scopes: &crate::style::CounterScopes,
        display: &Display,
    ) -> Result<(), StyleError> {
        self.inherit(target)?;
        if !matches!(display, Display::None | Display::Contents) {
            for name in &scopes.reset {
                self.instantiate(name.clone(), INITIAL_COUNTER_VALUE, false, None)?;
            }
            for name in &scopes.updates {
                self.value(name.clone())?;
            }
            if display.is_list_item() {
                self.value(Arc::from(LIST_ITEM_COUNTER))?;
            }
        }
        self.update_previous()
    }
    fn enter_selected(
        &mut self,
        target: PageStyleTarget,
        styles: &dyn crate::style::PageSelection,
        display: &Display,
        scopes_only: bool,
    ) -> Result<(), StyleError> {
        if *display == Display::Contents {
            return self.enter(target, &CounterProperties::default(), display);
        }
        if scopes_only {
            self.enter_scopes(target, &styles.counter_scopes(target)?, display)
        } else {
            self.enter(target, &styles.counters(target)?, display)
        }
    }
    fn track_updates(
        &mut self,
        properties: &CounterProperties,
        implicit: Option<i128>,
    ) -> Result<(), StyleError> {
        let mut names: HashSet<Arc<str>> = properties
            .increment
            .iter()
            .chain(properties.set.iter())
            .map(|change| change.name.clone())
            .collect();
        if implicit.is_some() {
            names.insert(Arc::from(LIST_ITEM_COUNTER));
        }
        let counters = &self.frames.last().unwrap().counters;
        let work = names.len().saturating_mul(
            counters
                .len()
                .saturating_add(properties.increment.len())
                .saturating_add(properties.set.len()),
        );
        self.charge(work)?;
        for name in names {
            let Some(initial) = self
                .frames
                .last()
                .unwrap()
                .counters
                .iter()
                .rev()
                .find(|counter| counter.name == name)
                .and_then(|counter| counter.automatic.clone())
            else {
                continue;
            };
            let increment = properties
                .increment
                .iter()
                .filter(|change| change.name == name)
                .fold(INITIAL_COUNTER_VALUE, |sum, change| {
                    sum.saturating_add(change.value)
                });
            let increment = if name.as_ref() == LIST_ITEM_COUNTER {
                increment.saturating_add(implicit.unwrap_or(INITIAL_COUNTER_VALUE))
            } else {
                increment
            };
            let set = properties
                .set
                .iter()
                .rev()
                .find(|change| change.name == name)
                .map(|change| change.value);
            initial.borrow_mut().update(increment, set);
        }
        Ok(())
    }
    pub(crate) fn leave(&mut self) {
        let frame = self
            .frames
            .pop()
            .expect("counter scopes close in tree order");
        if let Some(parent) = self.frames.last_mut() {
            parent.sibling = Some(frame.counters);
        }
    }
    fn update_previous(&mut self) -> Result<(), StyleError> {
        let count = self.frames.last().unwrap().counters.len();
        self.charge(count)?;
        self.previous = self.frames.last().unwrap().counters.clone();
        Ok(())
    }
    fn instantiate(
        &mut self,
        name: Arc<str>,
        value: i128,
        reversed: bool,
        automatic: Option<Rc<RefCell<AutomaticInitial>>>,
    ) -> Result<Rc<Cell<i128>>, StyleError> {
        let count = self.frames.last().unwrap().counters.len();
        self.charge(count.saturating_add(1))?;
        let parent = self.frames.iter().rev().nth(1).map(|frame| frame.target);
        let frame = self.frames.last_mut().unwrap();
        if let Some(index) = frame
            .counters
            .iter()
            .rposition(|counter| counter.name == name)
            && (frame.counters[index].creator == frame.target
                || frame.counters[index].parent == parent)
        {
            frame.counters.remove(index);
        }
        let value = Rc::new(Cell::new(value));
        frame.counters.push(Counter {
            name,
            creator: frame.target,
            parent,
            value: value.clone(),
            reversed,
            automatic,
        });
        Ok(value)
    }
    fn value(&mut self, name: Arc<str>) -> Result<Rc<Cell<i128>>, StyleError> {
        self.charge(self.frames.last().unwrap().counters.len())?;
        if let Some(counter) = self
            .frames
            .last()
            .unwrap()
            .counters
            .iter()
            .rev()
            .find(|counter| counter.name == name)
        {
            return Ok(counter.value.clone());
        }
        self.instantiate(name, INITIAL_COUNTER_VALUE, false, None)
    }
    pub(crate) fn ordinal(&mut self) -> Result<i128, StyleError> {
        let value = self.value(Arc::from(LIST_ITEM_COUNTER))?.get();
        self.update_previous()?;
        Ok(value)
    }
    pub(crate) fn snapshot(
        &mut self,
        content: &crate::style::GeneratedContent,
    ) -> Result<CounterSnapshot, StyleError> {
        for request in content.counter_requests() {
            self.value(request.name.clone())?;
        }
        self.update_previous()?;
        Ok(self.capture())
    }
    pub(crate) fn capture(&self) -> CounterSnapshot {
        CounterSnapshot(
            self.frames
                .last()
                .unwrap()
                .counters
                .iter()
                .map(|counter| (counter.name.clone(), counter.value.get()))
                .collect(),
        )
    }
    pub(crate) fn text(&mut self, request: &CounterRequest) -> Result<String, StyleError> {
        self.value(request.name.clone())?;
        let text = render_counter(
            request,
            self.frames
                .last()
                .unwrap()
                .counters
                .iter()
                .filter(|counter| counter.name == request.name)
                .map(|counter| counter.value.get()),
        )?;
        self.update_previous()?;
        Ok(text)
    }
}

impl Counters {
    pub(crate) fn prepare(
        &mut self,
        properties: &CounterProperties,
        document: &crate::dom::DocumentTree,
        sheet: &crate::style::Stylesheet,
        root: &crate::style::Style,
        resolver: &dyn crate::resources::ResourceResolver,
    ) -> Result<(), StyleError> {
        if self.initials.is_some() || !properties.reset.iter().any(|reset| reset.value.is_none()) {
            return Ok(());
        }
        let state = sheet.with_document_values(document, root, resolver, |styles| {
            Self {
                collecting: true,
                work: self.work.clone(),
                ..Self::default()
            }
            .walk(document, styles, None)
        })?;
        self.initials = Some(
            state
                .automatic
                .into_iter()
                .map(|(key, initial)| (key, initial.borrow().value()))
                .collect(),
        );
        Ok(())
    }
    pub(crate) fn visit_hidden(
        &mut self,
        element: crate::dom::Element<'_>,
        styles: &dyn crate::style::PageSelection,
        mut visit: impl FnMut(
            crate::dom::Element<'_>,
            &[crate::style::StringAssignment],
            &mut Self,
        ) -> Result<(), StyleError>,
    ) -> Result<(), StyleError> {
        use crate::dom::TreeRelation;
        let mut events = vec![Some(element)];
        while let Some(event) = events.pop() {
            self.charge(1)?;
            let Some(element) = event else {
                self.leave();
                continue;
            };
            let target = PageStyleTarget {
                element: element.id(),
                pseudo: None,
            };
            self.enter(target, &CounterProperties::default(), &Display::None)?;
            let assignments = styles.string_set(target)?;
            visit(element, &assignments, self)?;
            events.push(None);
            for child in element.children(TreeRelation::Formatting).rev() {
                self.charge(1)?;
                events.push(Some(child));
            }
        }
        Ok(())
    }

    pub(crate) fn at(
        document: &crate::dom::DocumentTree,
        styles: &dyn crate::style::PageSelection,
        wanted: PageStyleTarget,
        work: Rc<Cell<usize>>,
    ) -> Result<Self, StyleError> {
        Self {
            work,
            ..Self::default()
        }
        .walk(document, styles, Some(wanted))
    }
    fn walk(
        self,
        document: &crate::dom::DocumentTree,
        styles: &dyn crate::style::PageSelection,
        wanted: Option<PageStyleTarget>,
    ) -> Result<Self, StyleError> {
        use crate::dom::{Element, TreeRelation};
        use crate::style::{Display, PseudoElement};
        enum Event<'a> {
            Element(Element<'a>, Option<Display>),
            Pseudo(Element<'a>, PseudoElement, Display),
            Leave,
        }
        let mut state = self;
        let mut events = vec![Event::Element(document.root_element(), None)];
        while let Some(event) = events.pop() {
            state.charge(1)?;
            match event {
                Event::Leave => state.leave(),
                Event::Element(element, parent_display) => {
                    let target = PageStyleTarget {
                        element: element.id(),
                        pseudo: None,
                    };
                    let mut display = styles.display(target)?;
                    let replaced = crate::document::formatting::is_replaced(element.name());
                    if let Some(parent) = &parent_display {
                        crate::document::formatting::normalize_child_display(
                            &mut display,
                            replaced,
                            parent,
                        );
                        if *parent == Display::TableColumnGroup
                            && !matches!(display, Display::Contents | Display::TableColumn)
                        {
                            continue;
                        }
                    } else {
                        crate::document::formatting::normalize_root_display(&mut display);
                    }
                    if display == Display::None {
                        state.visit_hidden(element, styles, |_, assignments, counters| {
                            for assignment in assignments {
                                counters.snapshot(&assignment.content)?;
                            }
                            Ok(())
                        })?;
                        continue;
                    }
                    state.enter_selected(target, styles, &display, wanted.is_some())?;
                    if Some(target) == wanted {
                        return Ok(state);
                    }
                    for assignment in styles.string_set(target)?.iter() {
                        state.snapshot(&assignment.content)?;
                    }
                    if let Some(label) = styles.bookmark_label(target)? {
                        state.snapshot(&label)?;
                    }
                    events.push(Event::Leave);
                    if (display == Display::Contents || !replaced)
                        && display != Display::TableColumn
                    {
                        let block_display =
                            if matches!(display, Display::Contents | Display::Inline) {
                                parent_display.unwrap_or(Display::Block)
                            } else {
                                display
                            };
                        events.push(Event::Pseudo(
                            element,
                            PseudoElement::After,
                            block_display.clone(),
                        ));
                        if element.name() != "br" {
                            for child in element.children(TreeRelation::Formatting).rev() {
                                state.charge(1)?;
                                events.push(Event::Element(child, Some(block_display.clone())));
                            }
                        }
                        events.push(Event::Pseudo(element, PseudoElement::Before, block_display));
                    }
                }
                Event::Pseudo(element, pseudo, parent_display) => {
                    let target = PageStyleTarget {
                        element: element.id(),
                        pseudo: Some(pseudo),
                    };
                    let Some(content) = styles.content(target)? else {
                        continue;
                    };
                    let mut display = styles.display(target)?;
                    crate::document::formatting::normalize_child_display(
                        &mut display,
                        false,
                        &parent_display,
                    );
                    if display == Display::None {
                        state.visit_hidden(element, styles, |_, assignments, counters| {
                            for assignment in assignments {
                                counters.snapshot(&assignment.content)?;
                            }
                            Ok(())
                        })?;
                        continue;
                    }
                    state.enter_selected(target, styles, &display, wanted.is_some())?;
                    if Some(target) == wanted {
                        return Ok(state);
                    }
                    for request in content.counter_requests() {
                        state.value(request.name.clone())?;
                    }
                    state.update_previous()?;
                    state.leave();
                }
            }
        }
        if wanted.is_some() {
            Err(StyleError::Invalid(
                "counter target has no generated box".into(),
            ))
        } else {
            Ok(state)
        }
    }
}

#[derive(Default)]
pub(crate) struct CounterSnapshot(Vec<(Arc<str>, i128)>);
impl CounterSnapshot {
    pub(crate) fn text(&self, request: &CounterRequest) -> Result<String, StyleError> {
        render_counter(
            request,
            self.0
                .iter()
                .filter(|(name, _)| *name == request.name)
                .map(|(_, value)| *value),
        )
    }
}
fn render_counter(
    request: &CounterRequest,
    mut values: impl DoubleEndedIterator<Item = i128>,
) -> Result<String, StyleError> {
    let Some(separator) = &request.separator else {
        return request
            .style
            .representation(values.next_back().unwrap_or(INITIAL_COUNTER_VALUE));
    };
    let mut text = String::new();
    let mut empty = true;
    for value in values {
        let part = request.style.representation(value)?;
        let separator = if empty { "" } else { separator.as_ref() };
        if text
            .len()
            .saturating_add(separator.len())
            .saturating_add(part.len())
            > crate::document::MAX_INLINE_TEXT_BYTES
        {
            return Err(StyleError::GeneratedContentLimit {
                limit: crate::document::MAX_INLINE_TEXT_BYTES,
            });
        }
        text.push_str(separator);
        text.push_str(&part);
        empty = false;
    }
    if empty {
        text = request.style.representation(INITIAL_COUNTER_VALUE)?;
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::CounterReset;

    #[test]
    fn independent_counter_scans_share_the_selection_work_limit() {
        let document = crate::dom::DocumentTree::parse("<p>Counter scans</p>").unwrap();
        let target = PageStyleTarget {
            element: document.root_element().id(),
            pseudo: None,
        };
        let properties = CounterProperties {
            reset: Arc::from([CounterReset {
                name: Arc::from("chapter"),
                value: Some(INITIAL_COUNTER_VALUE),
                reversed: false,
            }]),
            ..Default::default()
        };
        const SCAN_WORK: usize = 2;
        let work = Rc::new(Cell::new(MAX_COUNTER_WORK - SCAN_WORK));
        let mut first = Counters {
            work: work.clone(),
            ..Default::default()
        };
        first
            .enter(target, &properties, &Display::Block)
            .expect("the first scan fits the remaining work budget");
        first.leave();
        drop(first);
        let mut second = Counters {
            work,
            ..Default::default()
        };
        assert!(
            matches!(
                second.enter(target, &properties, &Display::Block),
                Err(StyleError::GrammarLimit {
                    resource: COUNTER_WORK_RESOURCE,
                    limit: MAX_COUNTER_WORK
                })
            ),
            "a later scan must not regain the work consumed by an earlier scan"
        );
    }
}
