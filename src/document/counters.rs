use crate::style::{
    AutomaticInitial, CounterProperties, CounterRequest, DEFAULT_COUNTER_INCREMENT, Display,
    INITIAL_COUNTER_VALUE, LIST_ITEM_COUNTER, PageStyleTarget, StyleError,
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

pub(crate) const MAX_COUNTER_WORK: usize = 10_000_000;
pub(crate) const COUNTER_WORK_RESOURCE: &str = "counter evaluation work";

#[derive(Clone)]
struct Counter {
    name: Arc<str>,
    creator: PageStyleTarget,
    parent: Option<PageStyleTarget>,
    value: Rc<Cell<i128>>,
    reversed: bool,
    automatic: Option<Rc<RefCell<AutomaticInitial>>>,
    /// The style containment scopes enclosing the counter's creator.
    scope: usize,
}
/// A CSS counters set, innermost counter first. The sets of elements share the counters they
/// inherit from one another, so that inheriting and keeping a set costs the counters in which
/// the sets differ, not how many counters are in scope.
#[derive(Clone, Default)]
struct CounterList(Option<Rc<CounterNode>>);
struct CounterNode {
    counter: Counter,
    outer: CounterList,
    len: usize,
}
impl CounterList {
    fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |node| node.len)
    }
    fn nodes(&self) -> impl Iterator<Item = &Rc<CounterNode>> {
        std::iter::successors(self.0.as_ref(), |node| node.outer.0.as_ref())
    }
    /// The counters, innermost first.
    fn iter(&self) -> impl Iterator<Item = &Counter> {
        self.nodes().map(|node| &node.counter)
    }
    /// The counters, outermost first.
    fn outermost_first(&self) -> Vec<&Counter> {
        let mut counters: Vec<_> = self.iter().collect();
        counters.reverse();
        counters
    }
    /// The innermost counter named `name`, with its position from the innermost, and the
    /// counters stepped over to find it, or to find there is none.
    fn innermost(&self, name: &str) -> (Option<(usize, &Counter)>, usize) {
        let mut steps = 0;
        for (position, counter) in self.iter().enumerate() {
            steps += 1;
            if counter.name.as_ref() == name {
                return (Some((position, counter)), steps);
            }
        }
        (None, steps)
    }
    fn push(&mut self, counter: Counter) {
        let outer = std::mem::take(self);
        let len = outer.len() + 1;
        *self = Self(Some(Rc::new(CounterNode {
            counter,
            outer,
            len,
        })));
    }
    /// Removes the counter `position` counters from the innermost, copying those inside it.
    fn remove(&mut self, position: usize) {
        let (inner, mut list) = {
            let mut nodes = self.nodes();
            let inner: Vec<Counter> = nodes
                .by_ref()
                .take(position)
                .map(|node| node.counter.clone())
                .collect();
            let Some(removed) = nodes.next() else {
                return;
            };
            (inner, removed.outer.clone())
        };
        for counter in inner.into_iter().rev() {
            list.push(counter);
        }
        *self = list;
    }
    /// The counters of `self` that `source` holds too, in the order of `self`, and the
    /// counters stepped over to find them: those outside the counters both share.
    fn retained_in(&self, source: &Self) -> (Self, usize) {
        fn same(own: Option<&Rc<CounterNode>>, other: Option<&Rc<CounterNode>>) -> bool {
            match (own, other) {
                (Some(own), Some(other)) => Rc::ptr_eq(own, other),
                (own, other) => own.is_none() && other.is_none(),
            }
        }
        let len = |node: Option<&Rc<CounterNode>>| node.map_or(0, |node| node.len);
        let (mut own, mut other) = (self.0.as_ref(), source.0.as_ref());
        let mut unshared = Vec::new();
        let mut held = HashSet::new();
        let mut steps = 0;
        while !same(own, other) {
            steps += 1;
            let (own_len, other_len) = (len(own), len(other));
            if own_len >= other_len
                && let Some(node) = own
            {
                unshared.push(&node.counter);
                own = node.outer.0.as_ref();
            }
            if other_len >= own_len
                && let Some(node) = other
            {
                held.insert((node.counter.name.as_ref(), node.counter.creator));
                other = node.outer.0.as_ref();
            }
        }
        let mut list = Self(own.cloned());
        for counter in unshared.into_iter().rev() {
            if held.contains(&(counter.name.as_ref(), counter.creator)) {
                list.push(counter.clone());
            }
        }
        (list, steps)
    }
    /// A copy of the counters with values of their own, copying each node once among all the
    /// lists `copies` records.
    fn detached(
        &self,
        copies: &mut HashMap<*const CounterNode, CounterList>,
        cells: &mut HashMap<*const Cell<i128>, Rc<Cell<i128>>>,
    ) -> Self {
        let mut pending = Vec::new();
        let mut list = Self::default();
        for node in self.nodes() {
            if let Some(copy) = copies.get(&Rc::as_ptr(node)) {
                list = copy.clone();
                break;
            }
            pending.push(node);
        }
        for node in pending.into_iter().rev() {
            let counter = &node.counter;
            list.push(Counter {
                value: cells
                    .entry(Rc::as_ptr(&counter.value))
                    .or_insert_with(|| Rc::new(Cell::new(counter.value.get())))
                    .clone(),
                ..counter.clone()
            });
            copies.insert(Rc::as_ptr(node), list.clone());
        }
        list
    }
}
/// Drops the nodes no other list shares one at a time: dropping a deep list node within node
/// would take stack in proportion to its length.
impl Drop for CounterList {
    fn drop(&mut self) {
        let mut next = self.0.take();
        while let Some(node) = next {
            next = Rc::try_unwrap(node)
                .ok()
                .and_then(|mut node| node.outer.0.take());
        }
    }
}

struct Frame {
    target: PageStyleTarget,
    counters: CounterList,
    sibling: Option<CounterList>,
    /// Style containment scopes enclosing the element: increments and sets inside one only
    /// update counters created inside it (CSS Containment 2 §3.3).
    scope: usize,
    /// The element's style containment scopes its descendants.
    scoping: bool,
}

type CounterKey = (PageStyleTarget, Arc<str>);
#[derive(Default)]
pub(crate) struct Counters {
    frames: Vec<Frame>,
    previous: CounterList,
    work: Rc<Cell<usize>>,
    initials: Option<HashMap<CounterKey, i128>>,
    collecting: bool,
    automatic: Vec<(CounterKey, Rc<RefCell<AutomaticInitial>>)>,
    page_resets: Vec<PageReset>,
}

/// A page context's reset of a counter without a value, whose initial value as a reversed
/// counter follows from the updates of the counter it resets up to its next reset (CSS Lists 3
/// §4.4.2).
struct PageReset {
    page: usize,
    name: Arc<str>,
    counter: Rc<Cell<i128>>,
    initial: AutomaticInitial,
    open: bool,
}
impl Counters {
    /// A copy whose counters change apart from these, for formatting a subtree again from
    /// where these stand. It shares their work budget.
    pub(crate) fn detached(&self) -> Self {
        let (mut copies, mut cells) = (HashMap::new(), HashMap::new());
        let mut copy = |counters: &CounterList| counters.detached(&mut copies, &mut cells);
        Self {
            frames: self
                .frames
                .iter()
                .map(|frame| Frame {
                    target: frame.target,
                    counters: copy(&frame.counters),
                    sibling: frame.sibling.as_ref().map(&mut copy),
                    scope: frame.scope,
                    scoping: frame.scoping,
                })
                .collect(),
            previous: copy(&self.previous),
            work: self.work.clone(),
            initials: self.initials.clone(),
            collecting: false,
            automatic: Vec::new(),
            page_resets: Vec::new(),
        }
    }
    /// Charges `amount` units of counter work; false once the budget is spent, past which
    /// counters are no longer created, updated or inherited, and read as zero.
    fn charge(&mut self, amount: usize) -> bool {
        crate::work::count(crate::work::Work::CounterStep, amount);
        self.work.set(self.work.get().saturating_add(amount));
        !self.exhausted()
    }
    /// Whether counter evaluation ran out of work.
    pub(crate) fn exhausted(&self) -> bool {
        self.work.get() > MAX_COUNTER_WORK
    }
    pub(crate) fn enter(
        &mut self,
        target: PageStyleTarget,
        properties: &CounterProperties,
        display: &Display,
        scoping: bool,
    ) -> Result<(), StyleError> {
        self.inherit(target, scoping);
        self.apply(target, properties, display)
    }
    /// Enters an element where pages start: pages starting at it start with the counters it
    /// inherits, and pages starting inside it with those it has after its own changes.
    pub(crate) fn enter_at_pages(
        &mut self,
        target: PageStyleTarget,
        properties: &CounterProperties,
        display: &Display,
        scoping: bool,
        starts: &[crate::document::PageStart],
        snapshots: &mut Vec<(usize, CounterSnapshot)>,
    ) -> Result<(), StyleError> {
        self.inherit(target, scoping);
        self.start_pages(starts.iter().filter(|start| !start.inside), snapshots);
        self.apply(target, properties, display)?;
        self.start_pages(starts.iter().filter(|start| start.inside), snapshots);
        Ok(())
    }
    /// Applies each page context's changes to the footnote counter where its page starts (CSS
    /// GCPM 3 §2.5.2) and records the document's counters there, which page-margin boxes see
    /// (CSS Page 3 §6.1).
    pub(crate) fn start_pages<'s>(
        &mut self,
        starts: impl Iterator<Item = &'s crate::document::PageStart>,
        snapshots: &mut Vec<(usize, CounterSnapshot)>,
    ) {
        for start in starts {
            for reset in start.footnote.reset.iter() {
                let counter = self.updated(reset.name.clone());
                for open in self
                    .page_resets
                    .iter_mut()
                    .filter(|reset| Rc::ptr_eq(&reset.counter, &counter))
                {
                    open.open = false;
                }
                // The previous formatting found the initial value of a reset without one.
                let value = reset.value.unwrap_or_else(|| {
                    self.page_resets.push(PageReset {
                        page: start.page,
                        name: reset.name.clone(),
                        counter: counter.clone(),
                        initial: AutomaticInitial::default(),
                        open: true,
                    });
                    start
                        .automatic
                        .iter()
                        .find(|(name, _)| *name == reset.name)
                        .map_or(INITIAL_COUNTER_VALUE, |(_, value)| *value)
                });
                counter.set(value);
            }
            self.update(&start.footnote);
            snapshots.push((start.page, self.capture()));
        }
        self.update_previous();
    }
    /// Applies the increments, then the sets, of `properties` to the counters they update,
    /// which the page resets of those counters accumulate.
    fn update(&mut self, properties: &CounterProperties) {
        let mut increments = Vec::new();
        for change in properties.increment.iter() {
            let value = self.updated(change.name.clone());
            value.set(value.get().saturating_add(change.value));
            increments.push((value, change.value));
        }
        let mut sets = Vec::new();
        for change in properties.set.iter() {
            let value = self.updated(change.name.clone());
            value.set(change.value);
            sets.push((value, change.value));
        }
        for reset in self.page_resets.iter_mut().filter(|reset| reset.open) {
            let updates = |changes: &[(Rc<Cell<i128>>, i128)]| {
                changes
                    .iter()
                    .filter(|(counter, _)| Rc::ptr_eq(counter, &reset.counter))
                    .map(|(_, value)| *value)
                    .collect::<Vec<_>>()
            };
            let (increment, set) = (updates(&increments), updates(&sets));
            if !increment.is_empty() || !set.is_empty() {
                reset.initial.update(
                    increment
                        .into_iter()
                        .fold(INITIAL_COUNTER_VALUE, i128::saturating_add),
                    set.last().copied(),
                );
            }
        }
    }
    /// The initial value each page context's reset without one gives the counter it resets, as
    /// a reversed counter, by page.
    pub(crate) fn page_initials(&self) -> Vec<(usize, Arc<str>, i128)> {
        self.page_resets
            .iter()
            .map(|reset| (reset.page, reset.name.clone(), reset.initial.value()))
            .collect()
    }
    fn apply(
        &mut self,
        target: PageStyleTarget,
        properties: &CounterProperties,
        display: &Display,
    ) -> Result<(), StyleError> {
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
                self.instantiate(change.name.clone(), value, change.reversed, automatic);
            }
            let implicit = if display.is_list_item()
                && !properties
                    .increment
                    .iter()
                    .any(|change| change.name.as_ref() == LIST_ITEM_COUNTER)
            {
                let (list_item, steps) = self
                    .frames
                    .last()
                    .unwrap()
                    .counters
                    .innermost(LIST_ITEM_COUNTER);
                let reversed = list_item.is_some_and(|(_, counter)| counter.reversed);
                self.charge(steps);
                Some(if reversed {
                    -DEFAULT_COUNTER_INCREMENT
                } else {
                    DEFAULT_COUNTER_INCREMENT
                })
            } else {
                None
            };
            if self.collecting {
                self.track_updates(properties, implicit);
            }
            if let Some(increment) = implicit {
                let value = self.updated(Arc::from(LIST_ITEM_COUNTER));
                value.set(value.get().saturating_add(increment));
            }
            self.update(properties);
        }
        self.update_previous();
        Ok(())
    }
    /// Opens the counters of `target`: those of its parent, or of its preceding sibling once
    /// there is one, that the element preceding it holds, with their values there (CSS Lists 3
    /// §4.5).
    fn inherit(&mut self, target: PageStyleTarget, scoping: bool) {
        let (inherited, steps) = self.frames.last().map_or_else(Default::default, |parent| {
            self.previous
                .retained_in(parent.sibling.as_ref().unwrap_or(&parent.counters))
        });
        let counters = if self.charge(steps) {
            inherited
        } else {
            CounterList::default()
        };
        let scope = self
            .frames
            .last()
            .map_or(0, |parent| parent.scope + usize::from(parent.scoping));
        self.frames.push(Frame {
            target,
            counters,
            sibling: None,
            scope,
            scoping,
        });
    }
    fn enter_scopes(
        &mut self,
        target: PageStyleTarget,
        scopes: &crate::style::CounterScopes,
        display: &Display,
        scoping: bool,
    ) {
        self.inherit(target, scoping);
        if !matches!(display, Display::None | Display::Contents) {
            for name in &scopes.reset {
                self.instantiate(name.clone(), INITIAL_COUNTER_VALUE, false, None);
            }
            for name in &scopes.updates {
                self.updated(name.clone());
            }
            if display.is_list_item() {
                self.updated(Arc::from(LIST_ITEM_COUNTER));
            }
        }
        self.update_previous();
    }
    fn enter_selected(
        &mut self,
        target: PageStyleTarget,
        styles: &dyn crate::style::PageSelection,
        display: &Display,
        scopes_only: bool,
    ) -> Result<(), StyleError> {
        let scoping = styles.style_containment(target)?;
        if *display == Display::Contents {
            return self.enter(target, &CounterProperties::default(), display, scoping);
        }
        if scopes_only {
            self.enter_scopes(target, &styles.counter_scopes(target)?, display, scoping);
            Ok(())
        } else {
            self.enter(target, &styles.counters(target)?, display, scoping)
        }
    }
    fn track_updates(&mut self, properties: &CounterProperties, implicit: Option<i128>) {
        let mut names: HashSet<Arc<str>> = properties
            .increment
            .iter()
            .chain(properties.set.iter())
            .map(|change| change.name.clone())
            .collect();
        if implicit.is_some() {
            names.insert(Arc::from(LIST_ITEM_COUNTER));
        }
        for name in names {
            let (counter, steps) = self.frames.last().unwrap().counters.innermost(&name);
            let initial = counter.and_then(|(_, counter)| counter.automatic.clone());
            if !self.charge(
                steps
                    .saturating_add(properties.increment.len())
                    .saturating_add(properties.set.len()),
            ) {
                return;
            }
            let Some(initial) = initial else {
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
    fn update_previous(&mut self) {
        self.charge(1);
        self.previous = self.frames.last().unwrap().counters.clone();
    }
    fn instantiate(
        &mut self,
        name: Arc<str>,
        value: i128,
        reversed: bool,
        automatic: Option<Rc<RefCell<AutomaticInitial>>>,
    ) -> Rc<Cell<i128>> {
        let parent = self.frames.iter().rev().nth(1).map(|frame| frame.target);
        let frame = self.frames.last().unwrap();
        // A new counter replaces the element's own counter of its name, or its preceding
        // sibling's.
        let (replaced, steps) = frame.counters.innermost(&name);
        let replaced = replaced
            .filter(|(_, counter)| counter.creator == frame.target || counter.parent == parent)
            .map(|(position, _)| position);
        if !self.charge(steps.saturating_add(1)) {
            return Rc::new(Cell::new(value));
        }
        let frame = self.frames.last_mut().unwrap();
        if let Some(position) = replaced {
            frame.counters.remove(position);
        }
        let value = Rc::new(Cell::new(value));
        frame.counters.push(Counter {
            name,
            creator: frame.target,
            parent,
            value: value.clone(),
            reversed,
            automatic,
            scope: frame.scope,
        });
        value
    }
    /// The counter an increment or set updates: the innermost of its name created inside the
    /// element's innermost style containment scope, or else a new one (CSS Containment 2
    /// §3.3). Reading counter values is not scoped.
    fn updated(&mut self, name: Arc<str>) -> Rc<Cell<i128>> {
        let frame = self.frames.last().unwrap();
        let (counter, steps) = frame.counters.innermost(&name);
        let found = counter
            .filter(|(_, counter)| counter.scope == frame.scope)
            .map(|(_, counter)| counter.value.clone());
        self.found_or_instantiated(name, found, steps)
    }
    fn value(&mut self, name: Arc<str>) -> Rc<Cell<i128>> {
        let (counter, steps) = self.frames.last().unwrap().counters.innermost(&name);
        let found = counter.map(|(_, counter)| counter.value.clone());
        self.found_or_instantiated(name, found, steps)
    }
    /// The value of the counter `name` that `steps` found, or else of a new one.
    fn found_or_instantiated(
        &mut self,
        name: Arc<str>,
        found: Option<Rc<Cell<i128>>>,
        steps: usize,
    ) -> Rc<Cell<i128>> {
        if !self.charge(steps) {
            return Rc::new(Cell::new(INITIAL_COUNTER_VALUE));
        }
        found.unwrap_or_else(|| self.instantiate(name, INITIAL_COUNTER_VALUE, false, None))
    }
    pub(crate) fn ordinal(&mut self) -> i128 {
        let value = self.value(Arc::from(LIST_ITEM_COUNTER)).get();
        self.update_previous();
        value
    }
    pub(crate) fn snapshot(&mut self, content: &crate::style::GeneratedContent) -> CounterSnapshot {
        for request in content.counter_requests() {
            self.value(request.name.clone());
        }
        self.update_previous();
        self.capture()
    }
    pub(crate) fn capture(&self) -> CounterSnapshot {
        CounterSnapshot(
            self.frames
                .last()
                .unwrap()
                .counters
                .outermost_first()
                .into_iter()
                .map(|counter| (counter.name.clone(), counter.value.get()))
                .collect(),
        )
    }
    pub(crate) fn text(&mut self, request: &CounterRequest) -> crate::style::Representation {
        self.value(request.name.clone());
        let counters = &self.frames.last().unwrap().counters;
        // `counter()` shows the innermost counter of the name, `counters()` each of them.
        let text = if request.separator.is_some() {
            render_counter(
                request,
                counters
                    .outermost_first()
                    .into_iter()
                    .filter(|counter| counter.name == request.name)
                    .map(|counter| counter.value.get()),
            )
        } else {
            render_counter(
                request,
                counters
                    .innermost(&request.name)
                    .0
                    .map(|(_, counter)| counter.value.get())
                    .into_iter(),
            )
        };
        self.update_previous();
        text
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
            self.charge(1);
            let Some(element) = event else {
                self.leave();
                continue;
            };
            let target = PageStyleTarget {
                element: element.id(),
                pseudo: None,
            };
            self.enter(target, &CounterProperties::default(), &Display::None, false)?;
            let assignments = styles.string_set(target)?;
            visit(element, &assignments, self)?;
            events.push(None);
            for child in element.children(TreeRelation::Formatting).rev() {
                self.charge(1);
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
        use crate::style::{Display, GeneratedPseudoElement};
        enum Event<'a> {
            Element(Element<'a>, Option<Display>),
            Pseudo(Element<'a>, GeneratedPseudoElement, Display),
            /// The content list replacing an element's children (CSS Content 3 §1), which
            /// generate no boxes.
            Content(Element<'a>, crate::style::GeneratedContent),
            Leave,
        }
        let mut state = self;
        let mut events = vec![Event::Element(document.root_element(), None)];
        while let Some(event) = events.pop() {
            state.charge(1);
            match event {
                Event::Leave => state.leave(),
                Event::Element(element, parent_display) => {
                    let target = PageStyleTarget {
                        element: element.id(),
                        pseudo: None,
                    };
                    let mut display = styles.display(target)?;
                    // CSS Content 3 §1: a content list replaces the element's contents, and a
                    // single image makes it a replaced element.
                    let content = styles.content(target)?;
                    let replaced = crate::document::formatting::is_replaced(element)
                        || content
                            .as_ref()
                            .is_some_and(|content| content.replacement().is_some());
                    if replaced && display == Display::Contents {
                        display = Display::None;
                    }
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
                                counters.snapshot(&assignment.content);
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
                        state.snapshot(&assignment.content);
                    }
                    if let Some(label) = styles.bookmark_label(target)? {
                        state.snapshot(&label);
                    }
                    events.push(Event::Leave);
                    if (display == Display::Contents || !replaced)
                        && display != Display::TableColumn
                    {
                        let block_display =
                            if display == Display::Contents || display.is_inline_box() {
                                parent_display.unwrap_or(Display::Block)
                            } else {
                                display
                            };
                        if element.generates_picker_icon() {
                            events.push(Event::Pseudo(
                                element,
                                GeneratedPseudoElement::PickerIcon,
                                block_display.clone(),
                            ));
                        }
                        events.push(Event::Pseudo(
                            element,
                            GeneratedPseudoElement::After,
                            block_display.clone(),
                        ));
                        if let Some(content) = content {
                            events.push(Event::Content(element, content));
                        } else if element.name() != "br" {
                            for child in element.children(TreeRelation::Formatting).rev() {
                                state.charge(1);
                                events.push(Event::Element(child, Some(block_display.clone())));
                            }
                        }
                        events.push(Event::Pseudo(
                            element,
                            GeneratedPseudoElement::Before,
                            block_display.clone(),
                        ));
                        if element.generates_checkmark() {
                            events.push(Event::Pseudo(
                                element,
                                GeneratedPseudoElement::Checkmark,
                                block_display,
                            ));
                        }
                    }
                }
                Event::Content(element, content) => {
                    for request in content.counter_requests() {
                        state.value(request.name.clone());
                    }
                    for child in element.children(TreeRelation::Formatting) {
                        state.visit_hidden(child, styles, |_, assignments, counters| {
                            for assignment in assignments {
                                counters.snapshot(&assignment.content);
                            }
                            Ok(())
                        })?;
                    }
                }
                Event::Pseudo(element, pseudo, parent_display) => {
                    let target = PageStyleTarget {
                        element: element.id(),
                        pseudo: Some(pseudo.into()),
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
                                counters.snapshot(&assignment.content);
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
                        state.value(request.name.clone());
                    }
                    state.update_previous();
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CounterSnapshot(Vec<(Arc<str>, i128)>);
impl CounterSnapshot {
    /// Counters outermost first, as `counters()` lists them.
    pub(crate) fn new(counters: Vec<(Arc<str>, i128)>) -> Self {
        Self(counters)
    }
    /// The value of the innermost counter named `name`.
    pub(crate) fn value(&self, name: &str) -> Option<i128> {
        self.0
            .iter()
            .rev()
            .find(|(counter, _)| counter.as_ref() == name)
            .map(|(_, value)| *value)
    }
    pub(crate) fn text(&self, request: &CounterRequest) -> crate::style::Representation {
        render_counter(
            request,
            self.0
                .iter()
                .filter(|(name, _)| *name == request.name)
                .map(|(_, value)| *value),
        )
    }
}
/// The representation of `values`; `counters()` stops joining them once past
/// `MAX_INLINE_TEXT_BYTES`, which the content showing it cuts at its limit.
fn render_counter(
    request: &CounterRequest,
    mut values: impl DoubleEndedIterator<Item = i128>,
) -> crate::style::Representation {
    let Some(separator) = &request.separator else {
        return request
            .style
            .representation(values.next_back().unwrap_or(INITIAL_COUNTER_VALUE));
    };
    let mut joined = crate::style::Representation::default();
    let mut empty = true;
    for value in values {
        if joined.text().len() > crate::document::MAX_INLINE_TEXT_BYTES {
            break;
        }
        if !empty {
            joined.push_str(separator);
        }
        joined.append(request.style.representation(value));
        empty = false;
    }
    if empty {
        joined = request.style.representation(INITIAL_COUNTER_VALUE);
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::CounterReset;

    #[test]
    fn independent_counter_scans_share_the_selection_work_limit() {
        const CHAPTER: i128 = 5;
        const SCAN_WORK: usize = 2;
        let document = crate::dom::DocumentTree::parse("<p>Counter scans</p>");
        let target = PageStyleTarget {
            element: document.root_element().id(),
            pseudo: None,
        };
        let properties = CounterProperties {
            reset: Arc::from([CounterReset {
                name: Arc::from("chapter"),
                value: Some(CHAPTER),
                reversed: false,
            }]),
            ..Default::default()
        };
        let work = Rc::new(Cell::new(MAX_COUNTER_WORK - SCAN_WORK));
        let mut first = Counters {
            work: work.clone(),
            ..Default::default()
        };
        first
            .enter(target, &properties, &Display::Block, false)
            .expect("the first scan fits the remaining work budget");
        assert!(!first.exhausted(), "the first scan fits the budget");
        first.leave();
        drop(first);
        let mut second = Counters {
            work,
            ..Default::default()
        };
        second
            .enter(target, &properties, &Display::Block, false)
            .expect("counters past the work budget degrade instead of failing");
        assert!(
            second.exhausted(),
            "a later scan must not regain the work consumed by an earlier scan"
        );
        let chapter = second.value(Arc::from("chapter")).get();
        assert_eq!(
            chapter, INITIAL_COUNTER_VALUE,
            "past the budget the reset of chapter to {CHAPTER} is not evaluated, so it reads zero"
        );
    }
}
