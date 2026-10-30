use crate::document::page::page_group;
use crate::document::{
    formatting,
    tables::{self, Content},
};
use crate::dom::{DocumentTree, Element, TreeRelation};
use crate::style::{
    BreakBetween, CaptionSide, Display, FirstPageStart, GeneratedPseudoElement, Page,
    PageSelection, PageSide, PageStyleTarget, QuoteDepth, StyleError,
};
use std::{
    cell::{Cell, OnceCell},
    rc::Rc,
    sync::Arc,
};

enum Child {
    Inline {
        whitespace: bool,
    },
    Box {
        target: PageStyleTarget,
        display: Display,
        order: i32,
        caption: CaptionSide,
        depth: usize,
        quotes: Option<Rc<QuoteEvent>>,
    },
    Anonymous {
        origin: PageStyleTarget,
        display: Display,
        children: Vec<Child>,
    },
}
impl Content for Child {
    fn display(&self) -> Option<&Display> {
        match self {
            Self::Box { display, .. } | Self::Anonymous { display, .. } => Some(display),
            Self::Inline { .. } => None,
        }
    }
    fn whitespace(&self) -> bool {
        matches!(self, Self::Inline { whitespace: true })
    }
    fn caption_side(&self) -> CaptionSide {
        match self {
            Self::Box { caption, .. } => *caption,
            _ => CaptionSide::Top,
        }
    }
}

fn anonymous(origin: PageStyleTarget, display: Display, children: Vec<Child>) -> Child {
    let children = tables::normalize_with(&display, children, &|role, children| {
        anonymous(origin, role, children)
    });
    Child::Anonymous {
        origin,
        display,
        children,
    }
}

fn target(element: Element<'_>, pseudo: Option<GeneratedPseudoElement>) -> PageStyleTarget {
    PageStyleTarget {
        element: element.id(),
        pseudo: pseudo.map(Into::into),
    }
}

enum QuoteChange {
    Content(crate::style::GeneratedContent),
    Subtree {
        target: PageStyleTarget,
        display: Display,
        depth: usize,
    },
}
struct QuoteEvent {
    previous: Option<Rc<QuoteEvent>>,
    change: QuoteChange,
    end: OnceCell<QuoteDepth>,
}
impl Drop for QuoteEvent {
    fn drop(&mut self) {
        let mut previous = self.previous.take();
        while let Some(event) = previous {
            let Ok(mut event) = Rc::try_unwrap(event) else {
                break;
            };
            previous = event.previous.take();
        }
    }
}

struct Selection<'a> {
    document: &'a DocumentTree,
    generated: &'a crate::document::generated::GeneratedPass,
    styles: &'a dyn PageSelection,
    quotes: Option<Rc<QuoteEvent>>,
    quote_scan: Option<QuoteDepth>,
    counter_work: Rc<Cell<usize>>,
    /// The boxes that start at the first page's start, outermost first.
    chain: Vec<PageStyleTarget>,
}
impl Selection<'_> {
    /// Whether `element` is a replaced element, by its kind or its `<content-replacement>`
    /// (CSS Content 3 §1).
    fn replaced(&self, element: Element<'_>) -> Result<bool, StyleError> {
        Ok(formatting::is_replaced(element)
            || self
                .styles
                .content(target(element, None))?
                .is_some_and(|content| content.replacement().is_some()))
    }
    fn record_quotes(&mut self, change: QuoteChange) {
        self.quotes = Some(Rc::new(QuoteEvent {
            previous: self.quotes.take(),
            change,
            end: OnceCell::new(),
        }));
    }
    fn quote_depth(&self) -> Result<QuoteDepth, StyleError> {
        let mut pending = Vec::new();
        let mut current = self.quotes.as_deref();
        let mut depth = QuoteDepth::default();
        while let Some(event) = current {
            if let Some(cached) = event.end.get() {
                depth = *cached;
                break;
            }
            pending.push(event);
            current = event.previous.as_deref();
        }
        for event in pending.into_iter().rev() {
            match &event.change {
                QuoteChange::Content(content) => content.advance_quote_depth(&mut depth),
                QuoteChange::Subtree {
                    target,
                    display,
                    depth: nesting,
                } => {
                    let mut scan = Selection {
                        document: self.document,
                        generated: self.generated,
                        styles: self.styles,
                        quotes: None,
                        quote_scan: Some(depth),
                        counter_work: self.counter_work.clone(),
                        chain: Vec::new(),
                    };
                    let mut collector = Collector::new(*target, display.clone());
                    collector.quotes_only = true;
                    collector.contents(&mut scan, *target, *nesting)?;
                    depth = scan
                        .quote_scan
                        .expect("quote traversal retains its accumulator");
                }
            }
            assert!(event.end.set(depth).is_ok());
        }
        Ok(depth)
    }
    fn element(&self, target: PageStyleTarget) -> Element<'_> {
        self.document
            .element(target.element)
            .expect("generated targets belong to their document")
    }
    fn used(&self, mut target: PageStyleTarget) -> Result<Option<Arc<str>>, StyleError> {
        loop {
            if let Page::Name(name) = self.styles.page(target)? {
                return Ok(Some(name));
            }
            if target.pseudo.take().is_some() {
                continue;
            }
            let Some(parent) = self.element(target).parent(TreeRelation::Inheritance) else {
                return Ok(None);
            };
            target.element = parent.id();
        }
    }
    fn start(
        &mut self,
        mut child: Child,
        collect_breaks: bool,
    ) -> Result<(PageStyleTarget, BreakBetween), StyleError> {
        let mut before = BreakBetween::Auto;
        loop {
            let (origin, display, mut children) = match child {
                Child::Inline { .. } => {
                    unreachable!("anonymous inline runs use their containing box scope")
                }
                Child::Anonymous {
                    origin,
                    display,
                    children,
                } => (origin, display, children),
                Child::Box {
                    target,
                    display,
                    depth,
                    quotes,
                    ..
                } => {
                    self.quotes = quotes;
                    self.chain.push(target);
                    if collect_breaks {
                        before = before.combine(self.styles.break_before(target)?);
                    }
                    if target.pseudo.is_some()
                        || self.replaced(self.element(target))?
                        || display == Display::TableRow
                        || (display.is_list_item() && self.styles.inside_marker(target)?)
                    {
                        return Ok((target, before));
                    }
                    let mut collector = Collector::new(target, display.clone());
                    collector.contents(self, target, depth)?;
                    collector.flush(self, false)?;
                    let children =
                        tables::normalize_with(&display, collector.children, &|role, children| {
                            anonymous(target, role, children)
                        });
                    (target, display, children)
                }
            };
            let mut ordered = children.iter().collect::<Vec<_>>();
            if display.is_item_container() {
                ordered.sort_by_key(|child| match child {
                    Child::Box { order, .. } => *order,
                    _ => 0,
                });
            } else if display.is_table() {
                tables::order(&mut ordered);
            }
            let first = ordered.first().and_then(|first| {
                children
                    .iter()
                    .position(|child| std::ptr::eq(child, *first))
            });
            match first.map(|index| children.swap_remove(index)) {
                Some(next)
                    if next
                        .display()
                        .is_some_and(crate::document::PageBoundary::applies) =>
                {
                    child = next;
                }
                _ => return Ok((origin, before)),
            }
        }
    }
}

struct OpenInline {
    target: PageStyleTarget,
    continued: bool,
}
enum CollectStep<'a> {
    Contents(PageStyleTarget, usize),
    Children(PageStyleTarget, usize, crate::dom::ChildNodes<'a>),
    After(Element<'a>, usize),
    CloseInline(PageStyleTarget),
    RestoreDisplay(Display),
}
struct Collector {
    origin: PageStyleTarget,
    display: Display,
    children: Vec<Child>,
    inline: Vec<OpenInline>,
    text: bool,
    whitespace: bool,
    quotes_only: bool,
}
impl Collector {
    fn new(origin: PageStyleTarget, display: Display) -> Self {
        Self {
            origin,
            display,
            children: Vec::new(),
            inline: Vec::new(),
            text: false,
            whitespace: true,
            quotes_only: false,
        }
    }
    fn ready(&self) -> bool {
        if self.quotes_only || self.display.is_item_container() || self.display.is_table() {
            return false;
        }
        if self.display.is_row_group() {
            return self.children.iter().any(|child| !child.whitespace());
        }
        match self.children.first() {
            Some(Child::Inline { .. }) => true,
            Some(child)
                if !child.display().is_some_and(|display| {
                    display.is_table_child() || *display == Display::TableCell
                }) =>
            {
                true
            }
            Some(_) => {
                self.children.iter().any(|child| {
                    !child.display().is_some_and(|display| {
                        display.is_table_child() || *display == Display::TableCell
                    }) && !child.whitespace()
                }) || self.text && !self.whitespace
            }
            None => self.text,
        }
    }
    fn append(
        &mut self,
        selection: &Selection<'_>,
        target: PageStyleTarget,
        text: &str,
    ) -> Result<(), StyleError> {
        if text.is_empty() {
            return Ok(());
        }
        let whitespace = selection.styles.white_space_collapse(target)?;
        for ch in text.chars() {
            let visible = !crate::document::inline::collapses_character(ch, whitespace);
            if visible && (ch != '\t' || !selection.styles.tab_size(target)?.is_zero()) {
                self.text = true;
                self.whitespace &= crate::document::inline::is_css_whitespace(ch);
            }
        }
        Ok(())
    }
    fn flush(&mut self, selection: &Selection<'_>, split: bool) -> Result<(), StyleError> {
        for inline in &mut self.inline {
            self.text |= selection
                .styles
                .inline_edges(inline.target, [inline.continued, split])?;
            inline.continued |= split;
        }
        if self.text && !(self.display.is_item_container() && self.whitespace) {
            let child = Child::Inline {
                whitespace: self.whitespace,
            };
            self.children.push(if self.display.is_item_container() {
                anonymous(self.origin, Display::Block, vec![child])
            } else {
                child
            });
        }
        self.text = false;
        self.whitespace = true;
        Ok(())
    }
    fn contents(
        &mut self,
        selection: &mut Selection<'_>,
        source: PageStyleTarget,
        depth: usize,
    ) -> Result<(), StyleError> {
        let mut pending = vec![CollectStep::Contents(source, depth)];
        while let Some(step) = pending.pop() {
            if self.ready()
                && !matches!(
                    step,
                    CollectStep::CloseInline(_) | CollectStep::RestoreDisplay(_)
                )
            {
                continue;
            }
            match step {
                CollectStep::Contents(source, depth) => {
                    if depth > formatting::MAX_FORMATTING_DEPTH {
                        return Err(StyleError::GrammarLimit {
                            resource: formatting::FORMATTING_NESTING_RESOURCE,
                            limit: formatting::MAX_FORMATTING_DEPTH,
                        });
                    }
                    let element = selection
                        .document
                        .element(source.element)
                        .expect("generated target exists");
                    if selection.replaced(element)? || self.display == Display::TableColumn {
                        continue;
                    }
                    self.generated(
                        selection,
                        element,
                        GeneratedPseudoElement::Before,
                        depth + 1,
                    )?;
                    if self.ready() {
                        continue;
                    }
                    // CSS Content 3 §1: a content list replaces the element's children.
                    if let Some(content) = selection.styles.content(source)? {
                        if let Some(text) =
                            self.content_text(selection, element, source, true, content)?
                        {
                            self.append_content(selection, source, text)?;
                        }
                        pending.push(CollectStep::After(element, depth + 1));
                    } else if element.name() == "br" {
                        self.text = true;
                        self.generated(
                            selection,
                            element,
                            GeneratedPseudoElement::After,
                            depth + 1,
                        )?;
                    } else {
                        pending.push(CollectStep::After(element, depth + 1));
                        pending.push(CollectStep::Children(
                            source,
                            depth,
                            element.child_nodes(TreeRelation::Formatting),
                        ));
                    }
                }
                CollectStep::Children(parent, depth, mut nodes) => {
                    let Some(node) = nodes.next() else {
                        continue;
                    };
                    pending.push(CollectStep::Children(parent, depth, nodes));
                    if let Some(text) = node.value().as_text() {
                        if !self.quotes_only {
                            self.append(selection, parent, text)?;
                        }
                        continue;
                    }
                    let Some(element) = node.element() else {
                        continue;
                    };
                    let source = target(element, None);
                    let mut display = selection.styles.display(source)?;
                    display.validate()?;
                    let replaced = selection.replaced(element)?;
                    // CSS Display 3 §2.5: `display: contents` on a replaced element is `none`.
                    if replaced && display == Display::Contents {
                        display = Display::None;
                    }
                    formatting::normalize_child_display(&mut display, replaced, &self.display);
                    if display == Display::None
                        || (self.display == Display::TableColumnGroup
                            && !matches!(display, Display::Contents | Display::TableColumn))
                    {
                        continue;
                    }
                    if display == Display::Contents {
                        pending.push(CollectStep::Contents(source, depth + 1));
                    } else if self.quotes_only {
                        if !display.is_inline_box() || replaced {
                            let previous = std::mem::replace(&mut self.display, display);
                            pending.push(CollectStep::RestoreDisplay(previous));
                        }
                        pending.push(CollectStep::Contents(source, depth + 1));
                    } else if selection.styles.position(source)?.leaves_flow() {
                        selection.record_quotes(QuoteChange::Subtree {
                            target: source,
                            display,
                            depth: depth + 1,
                        });
                    } else if display.is_inline_box() && !replaced {
                        self.inline.push(OpenInline {
                            target: source,
                            continued: false,
                        });
                        pending.push(CollectStep::CloseInline(source));
                        pending.push(CollectStep::Contents(source, depth + 1));
                    } else {
                        self.box_or_atomic(selection, source, display, depth + 1)?;
                    }
                }
                CollectStep::After(element, depth) => {
                    self.generated(selection, element, GeneratedPseudoElement::After, depth)?;
                }
                CollectStep::CloseInline(source) => {
                    let inline = self
                        .inline
                        .pop()
                        .expect("inline frames close in tree order");
                    if !self.ready() {
                        self.text |= selection
                            .styles
                            .inline_edges(source, [inline.continued, false])?;
                    }
                }
                CollectStep::RestoreDisplay(display) => {
                    self.display = display;
                }
            }
        }
        Ok(())
    }
    fn generated(
        &mut self,
        selection: &mut Selection<'_>,
        element: Element<'_>,
        pseudo: GeneratedPseudoElement,
        depth: usize,
    ) -> Result<(), StyleError> {
        let source = target(element, Some(pseudo));
        let Some(content) = selection.styles.content(source)? else {
            return Ok(());
        };
        let mut display = selection.styles.display(source)?;
        display.validate()?;
        let replaced = content.replacement().is_some();
        formatting::normalize_child_display(&mut display, replaced, &self.display);
        if display == Display::None || (replaced && display == Display::Contents) {
            return Ok(());
        }
        let inline = !replaced && (display == Display::Contents || display.is_inline_box());
        let Some(text) = self.content_text(selection, element, source, inline, content)? else {
            return Ok(());
        };
        if display != Display::Contents && selection.styles.position(source)?.leaves_flow() {
            return Ok(());
        }
        if inline {
            self.append_content(selection, source, text)?;
            if display.is_inline_box() {
                self.text |= selection.styles.inline_edges(source, [false; 2])?;
            }
            return Ok(());
        }
        self.box_or_atomic(selection, source, display, depth)
    }
    /// The text a content list gives `source`, an element or its pseudo-element, when it lays
    /// out inline, and whether it holds images, which are atomic inlines; `None` while only
    /// quotes are traced. Records the quote depth it changes.
    fn content_text(
        &mut self,
        selection: &mut Selection<'_>,
        element: Element<'_>,
        source: PageStyleTarget,
        inline: bool,
        content: crate::style::GeneratedContent,
    ) -> Result<Option<(String, bool)>, StyleError> {
        if self.quotes_only {
            content.advance_quote_depth(
                selection
                    .quote_scan
                    .as_mut()
                    .expect("quote-only traversal has an accumulator"),
            );
            return Ok(None);
        }
        let bound = selection.generated.bind(&content, source)?;
        let text = if inline {
            let (quotes, mut depth) = if content.has_quote_actions() {
                (selection.styles.quotes(source)?, selection.quote_depth()?)
            } else {
                (Default::default(), QuoteDepth::default())
            };
            if content.counter_requests().next().is_some() {
                let mut counters = None;
                bound.element_text_with_counters(
                    crate::document::MAX_INLINE_TEXT_BYTES,
                    &mut |request| {
                        if let Some(text) = request.box_generation_text() {
                            return Ok(text.into());
                        }
                        if counters.is_none() {
                            counters = Some(crate::document::counters::Counters::at(
                                selection.document,
                                selection.styles,
                                source,
                                selection.counter_work.clone(),
                            )?);
                        }
                        counters.as_mut().unwrap().text(request)
                    },
                    &quotes,
                    &mut depth,
                )?
            } else {
                bound.element_text_with_quotes(
                    element,
                    crate::document::MAX_INLINE_TEXT_BYTES,
                    &quotes,
                    &mut depth,
                )?
            }
        } else {
            String::new()
        };
        let images = content.has_images();
        if content.has_quote_actions() {
            selection.record_quotes(QuoteChange::Content(content));
        }
        Ok(Some((text, images)))
    }
    /// Appends a content list's inline text and images.
    fn append_content(
        &mut self,
        selection: &Selection<'_>,
        source: PageStyleTarget,
        (text, images): (String, bool),
    ) -> Result<(), StyleError> {
        self.append(selection, source, &text)?;
        if images {
            self.text = true;
            self.whitespace = false;
        }
        Ok(())
    }
    fn box_or_atomic(
        &mut self,
        selection: &mut Selection<'_>,
        source: PageStyleTarget,
        display: Display,
        depth: usize,
    ) -> Result<(), StyleError> {
        if display.is_inline_level() {
            self.text = true;
            self.whitespace = false;
            if source.pseudo.is_none() {
                selection.record_quotes(QuoteChange::Subtree {
                    target: source,
                    display,
                    depth,
                });
            }
            return Ok(());
        }
        self.flush(selection, true)?;
        let order = if self.display.is_item_container() {
            selection.styles.order(source)?
        } else {
            0
        };
        let caption = if display == Display::TableCaption {
            selection.styles.caption_side(source)?
        } else {
            CaptionSide::Top
        };
        self.children.push(Child::Box {
            target: source,
            display: display.clone(),
            order,
            caption,
            depth,
            quotes: selection.quotes.clone(),
        });
        if source.pseudo.is_none() {
            selection.record_quotes(QuoteChange::Subtree {
                target: source,
                display,
                depth,
            });
        }
        Ok(())
    }
}

impl<'a> Selection<'a> {
    fn new(
        document: &'a DocumentTree,
        styles: &'a dyn PageSelection,
        generated: &'a crate::document::generated::GeneratedPass,
    ) -> Self {
        Self {
            document,
            styles,
            generated,
            quotes: None,
            quote_scan: None,
            counter_work: Rc::default(),
            chain: Vec::new(),
        }
    }
    fn first(
        &mut self,
        collect_breaks: bool,
    ) -> Result<Option<(PageStyleTarget, BreakBetween)>, StyleError> {
        let root = target(self.document.root_element(), None);
        let mut display = self.styles.display(root)?;
        display.validate()?;
        if display == Display::None {
            return Ok(None);
        }
        formatting::normalize_root_display(&mut display);
        self.start(
            Child::Box {
                target: root,
                display,
                order: 0,
                caption: CaptionSide::Top,
                depth: 0,
                quotes: None,
            },
            collect_breaks,
        )
        .map(Some)
    }
}

pub(super) fn select_name(
    document: &DocumentTree,
    styles: &dyn PageSelection,
    generated: &crate::document::generated::GeneratedPass,
    select_groups: bool,
) -> Result<FirstPageStart, StyleError> {
    let mut selection = Selection::new(document, styles, generated);
    let Some((target, _)) = selection.first(false)? else {
        return Ok(FirstPageStart::default());
    };
    let mut groups = Vec::new();
    for target in selection.chain.iter().filter(|_| select_groups) {
        let page = styles.page(*target)?;
        if matches!(page, Page::Name(_))
            && let Some(group) = page_group(&page, styles.break_before(*target)?)
        {
            groups.push(group);
        }
    }
    Ok(FirstPageStart {
        name: selection.used(target)?,
        groups,
    })
}

pub(super) fn select_side(
    document: &DocumentTree,
    styles: &dyn PageSelection,
    generated: &crate::document::generated::GeneratedPass,
) -> Result<PageSide, StyleError> {
    let rtl = styles.progression_rtl()?;
    let before = Selection::new(document, styles, generated)
        .first(true)?
        .map_or(BreakBetween::Auto, |(_, before)| before);
    Ok(before.page_side(rtl).unwrap_or_else(|| {
        let left = rtl != matches!(before, BreakBetween::Page | BreakBetween::Always);
        if left {
            PageSide::Left
        } else {
            PageSide::Right
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_history_releases_long_unique_tails_and_shared_checkpoints() {
        const EVENT_COUNT: usize = 30_000;
        let document = DocumentTree::parse("<p>quote history</p>").unwrap();
        let target = target(document.root_element(), None);
        let mut history = None;
        let mut checkpoint = None;
        for index in 0..EVENT_COUNT {
            history = Some(Rc::new(QuoteEvent {
                previous: history,
                change: QuoteChange::Subtree {
                    target,
                    display: Display::Block,
                    depth: 0,
                },
                end: OnceCell::new(),
            }));
            if index == EVENT_COUNT / 2 {
                checkpoint = history.clone();
            }
        }
        let oldest = Rc::downgrade(checkpoint.as_ref().unwrap());
        drop(history);
        assert_eq!(
            oldest.strong_count(),
            1,
            "dropping the tail preserves the live checkpoint"
        );
        drop(checkpoint);
        assert!(
            oldest.upgrade().is_none(),
            "the final checkpoint releases its prefix"
        );
    }
}
