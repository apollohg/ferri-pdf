mod transform;
mod transform_maps;
use crate::document::DocumentError;
use crate::style::{Clear, Style, TextAlign, WhiteSpace};
use std::ops::Range;

pub(crate) const MAX_INLINE_TEXT_BYTES: usize = 64 * 1024 * 1024;
/// Stands for an atomic inline in paragraph text (CSS Text 3 §5.1).
pub(crate) const OBJECT_REPLACEMENT_CHARACTER: char = '\u{fffc}';

pub(crate) struct TextSpan {
    pub range: Range<usize>,
    pub style: Style,
    /// Innermost inline box containing the text; `None` is the paragraph's root inline box.
    pub inline_box: Option<usize>,
    /// Atomic inline whose placeholder this span holds.
    pub atomic: Option<usize>,
    pub leader: bool,
    source: usize,
}
/// Non-replaced inline box (CSS 2.1 §9.2.2), stored in tree order.
pub(crate) struct InlineBox {
    pub range: Range<usize>,
    pub style: Style,
    pub decoration: crate::graphics::BoxDecoration,
    pub parent: Option<usize>,
    /// Boxes nested inside this one, which immediately follow it in tree order.
    pub descendants: usize,
    /// Whether the box continues before its start and after its end in other paragraphs,
    /// having been split around block-level content (CSS 2.1 §9.2.1.1).
    pub continued: [bool; 2],
    pub navigation: crate::document::BoxNavigation,
    pub canvas_source: bool,
}
impl InlineBox {
    /// Non-zero padding, borders or margins keep an otherwise empty line box (CSS 2.1 §9.4.2);
    /// a split box has no inline-axis edges where it continues.
    fn has_edges(&self) -> bool {
        self.style.has_inline_edges(self.continued)
    }
}
/// A float, absolute-position anchor, or atomic-inline placeholder at a source offset.
pub(crate) struct Anchor {
    pub offset: usize,
    pub block: crate::document::BlockBox,
    /// Innermost inline box containing the anchor; `None` is the paragraph's root inline box.
    pub inline_box: Option<usize>,
}
pub(crate) struct InlineLink {
    pub range: Range<usize>,
    pub link: usize,
}
pub(crate) struct Paragraph {
    expanded_tabs: Vec<Range<usize>>,
    pub(crate) marker_image: Option<crate::graphics::BackgroundImage>,
    pub(crate) first_formatted_line: bool,
    text: String,
    style: Box<Style>,
    spans: Vec<TextSpan>,
    anchors: Vec<Anchor>,
    boxes: Vec<InlineBox>,
    atomics: Vec<Anchor>,
    optional_breaks: Vec<usize>,
    clearances: Vec<(usize, Clear, bool)>,
    links: Vec<InlineLink>,
    events: Vec<crate::document::InlineEvent>,
}
impl Paragraph {
    #[cfg(test)]
    pub(crate) fn generated(text: &str, style: &Style) -> Result<Option<Self>, DocumentError> {
        let mut collector = InlineCollector::default();
        collector.append(text, style)?;
        Ok(collector.finish(style)?.ok())
    }

    pub(crate) fn generated_output(
        output: &crate::style::GeneratedOutput,
        style: &Style,
    ) -> Result<Option<Self>, DocumentError> {
        let mut collector = InlineCollector::default();
        collector.append_generated(output, style)?;
        Ok(collector.finish(style)?.ok())
    }

    pub(crate) fn marker(text: String, style: &Style) -> Result<Self, DocumentError> {
        let mut style = style.clone();
        style.white_space = WhiteSpace::Pre;
        style.align = TextAlign::Start;
        style.align_last = None;
        style.text_indent = Default::default();
        let span = TextSpan {
            range: 0..text.len(),
            style: style.clone(),
            inline_box: None,
            atomic: None,
            leader: false,
            source: 0,
        };
        let mut paragraph = Self {
            expanded_tabs: Vec::new(),
            marker_image: None,
            first_formatted_line: false,
            text,
            style: Box::new(style),
            spans: vec![span],
            anchors: Vec::new(),
            boxes: Vec::new(),
            atomics: Vec::new(),
            optional_breaks: Vec::new(),
            clearances: Vec::new(),
            links: Vec::new(),
            events: Vec::new(),
        };
        transform::apply(&mut paragraph)?;
        Ok(paragraph)
    }
    pub(crate) fn marker_image(&self) -> Option<&crate::graphics::BackgroundImage> {
        self.marker_image.as_ref()
    }
    pub(crate) fn expanded_tabs(&self) -> &[Range<usize>] {
        &self.expanded_tabs
    }
    /// CSS list marker default object size is one em square.
    pub(crate) fn marker_image_size(&self) -> Option<[f32; 2]> {
        use crate::graphics::{BackgroundImage, NaturalDimensions};
        let natural = match self.marker_image.as_ref()? {
            BackgroundImage::Image { image, resolution } => {
                crate::paint::natural_points(image, *resolution)
            }
            _ => NaturalDimensions::default(),
        };
        Some(crate::graphics::image_size(
            &Default::default(),
            natural,
            [self.style.size; 2],
        ))
    }
    pub(crate) fn first_formatted_line(&self) -> bool {
        self.first_formatted_line
    }
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
    pub(crate) fn style(&self) -> &Style {
        &self.style
    }
    pub(crate) fn spans(&self) -> &[TextSpan] {
        &self.spans
    }
    pub(crate) fn anchors(&self) -> &[Anchor] {
        &self.anchors
    }
    pub(crate) fn boxes(&self) -> &[InlineBox] {
        &self.boxes
    }
    /// Atomic inlines in text order; each `offset` holds an [`OBJECT_REPLACEMENT_CHARACTER`].
    pub(crate) fn atomics(&self) -> &[Anchor] {
        &self.atomics
    }
    pub(crate) fn optional_breaks(&self) -> &[usize] {
        &self.optional_breaks
    }
    /// Float clearance at the source position after a forced line break, with its direction.
    pub(crate) fn clearance_at(&self, offset: usize) -> (Clear, bool) {
        self.clearances
            .binary_search_by_key(&offset, |value| value.0)
            .ok()
            .map_or((Clear::None, self.style.rtl), |index| {
                let (_, clear, rtl) = self.clearances[index];
                (clear, rtl)
            })
    }
    pub(crate) fn links(&self) -> &[InlineLink] {
        &self.links
    }
    /// Whether `inner` (an inline box, or `None` for the root) lies inside box `outer`.
    pub(crate) fn is_within(&self, inner: Option<usize>, outer: usize) -> bool {
        inner.is_some_and(|inner| (outer..=outer + self.boxes[outer].descendants).contains(&inner))
    }
    pub(crate) fn semantic_events(&self) -> &[crate::document::InlineEvent] {
        &self.events
    }
    pub(crate) fn into_non_text(self) -> Vec<crate::document::BlockContent> {
        non_text(self.anchors, self.events)
    }
    pub(crate) fn leader_at(&self, offset: usize) -> bool {
        self.spans
            .get(self.spans.partition_point(|span| span.range.end <= offset))
            .is_some_and(|span| span.leader)
    }
    pub(crate) fn style_at(&self, offset: usize) -> &Style {
        self.spans
            .get(self.spans.partition_point(|span| span.range.end <= offset))
            .map_or(&self.style, |span| &span.style)
    }
}

/// An offset recorded while a collapsible space was pending; it follows that space if emitted.
#[derive(Clone, Copy)]
enum Mark {
    Anchor(usize),
    BoxStart(usize),
    BoxEnd(usize),
    OptionalBreak(usize),
    LinkStart(usize),
    LinkEnd(usize),
    Event(usize),
}

#[derive(Clone, Copy)]
struct PendingSpace {
    style: usize,
    source: usize,
    inline_box: Option<usize>,
}

#[derive(Default)]
pub(crate) struct InlineCollector {
    text: String,
    expanded_tabs: Vec<Range<usize>>,
    spans: Vec<TextSpan>,
    styles: Vec<Style>,
    pending_space: Option<PendingSpace>,
    next_source: usize,
    anchors: Vec<Anchor>,
    boxes: Vec<InlineBox>,
    atomics: Vec<Anchor>,
    optional_breaks: Vec<usize>,
    clearances: Vec<(usize, Clear, bool)>,
    open_box: Option<usize>,
    after_space: Vec<Mark>,
    links: Vec<InlineLink>,
    events: Vec<crate::document::InlineEvent>,
    open_links: Vec<Option<usize>>,
}
impl InlineCollector {
    pub(crate) fn semantic_event(&mut self, event: crate::document::SemanticEvent) {
        let index = self.events.len();
        self.events.push(crate::document::InlineEvent {
            offset: self.text.len(),
            event,
            anchors_before: self.anchors.len(),
        });
        if self.pending_space.is_some() {
            self.after_space.push(Mark::Event(index));
        }
    }

    fn style_index(&mut self, style: &Style) -> usize {
        if self
            .styles
            .last()
            .is_none_or(|last| !last.same_text_style(style))
        {
            self.styles.push(style.clone());
        }
        self.styles.len() - 1
    }
    fn next_source(&mut self) -> usize {
        self.next_source += 1;
        self.next_source - 1
    }
    pub(crate) fn append_generated(
        &mut self,
        output: &crate::style::GeneratedOutput,
        style: &Style,
    ) -> Result<(), DocumentError> {
        let mut from = 0;
        for range in output.leaders() {
            self.append(&output.text()[from..range.start], style)?;
            self.append_inner(&output.text()[range.clone()], style, true)?;
            from = range.end;
        }
        self.append(&output.text()[from..], style)
    }
    pub(crate) fn append(&mut self, text: &str, style: &Style) -> Result<(), DocumentError> {
        self.append_inner(text, style, false)
    }
    fn append_inner(
        &mut self,
        text: &str,
        style: &Style,
        leader: bool,
    ) -> Result<(), DocumentError> {
        if leader {
            self.emit_pending_space()?;
        }
        let first_span = self.spans.len();
        let index = self.style_index(style);
        let source = self.next_source();
        let inline_box = self.open_box;
        for ch in text.chars() {
            if ch == '\n' && style.white_space.preserves_breaks() {
                self.drop_pending_space();
                self.push('\n', index, source, 1, inline_box)?;
            } else if collapses_character(ch, style.white_space) {
                if self.pending_space.is_none()
                    && (leader || (!self.text.is_empty() && !self.text.ends_with('\n')))
                {
                    self.pending_space = Some(PendingSpace {
                        style: index,
                        source,
                        inline_box,
                    });
                }
            } else {
                self.emit_pending_space()?;
                if ch == '\t' {
                    let start = self.text.len();
                    self.push(' ', index, source, style.tab_size, inline_box)?;
                    if start < self.text.len() {
                        self.expanded_tabs.push(start..self.text.len());
                    }
                } else {
                    self.push(ch, index, source, 1, inline_box)?;
                }
            }
        }
        if leader {
            self.emit_pending_space()?;
            for span in &mut self.spans[first_span..] {
                span.leader = true;
            }
        }
        Ok(())
    }
    fn emit_pending_space(&mut self) -> Result<(), DocumentError> {
        if let Some(space) = self.pending_space.take() {
            self.push(' ', space.style, space.source, 1, space.inline_box)?;
            let end = self.text.len();
            for mark in std::mem::take(&mut self.after_space) {
                match mark {
                    Mark::Event(index) => self.events[index].offset = end,
                    Mark::Anchor(index) => self.anchors[index].offset = end,
                    Mark::BoxStart(index) => self.boxes[index].range.start = end,
                    Mark::BoxEnd(index) => self.boxes[index].range.end = end,
                    Mark::OptionalBreak(index) => self.optional_breaks[index] = end,
                    Mark::LinkStart(index) => self.links[index].range.start = end,
                    Mark::LinkEnd(index) => self.links[index].range.end = end,
                }
            }
        }
        Ok(())
    }
    fn drop_pending_space(&mut self) {
        self.pending_space = None;
        self.after_space.clear();
    }
    fn mark(&mut self, mark: Mark) {
        if self.pending_space.is_some() {
            self.after_space.push(mark);
        }
    }
    /// Anchors an out-of-flow box at the current inline source position.
    pub(crate) fn anchor(&mut self, block: crate::document::BlockBox) {
        self.mark(Mark::Anchor(self.anchors.len()));
        self.anchors.push(Anchor {
            offset: self.text.len(),
            block,
            inline_box: self.open_box,
        });
    }
    /// Opens a non-replaced inline box styled `style`; content appended next lies inside it.
    pub(crate) fn open_box(
        &mut self,
        style: &Style,
        decoration: crate::graphics::BoxDecoration,
        navigation: crate::document::BoxNavigation,
        canvas_source: bool,
    ) {
        self.open_continued(style, decoration, navigation, false, canvas_source);
    }
    fn open_continued(
        &mut self,
        style: &Style,
        decoration: crate::graphics::BoxDecoration,
        navigation: crate::document::BoxNavigation,
        continued: bool,
        canvas_source: bool,
    ) {
        let index = self.boxes.len();
        self.mark(Mark::BoxStart(index));
        self.boxes.push(InlineBox {
            range: self.text.len()..self.text.len(),
            style: style.clone(),
            decoration,
            parent: self.open_box,
            descendants: 0,
            continued: [continued, false],
            navigation,
            canvas_source,
        });
        self.open_box = Some(index);
    }
    pub(crate) fn close_box(&mut self) {
        let index = self
            .open_box
            .expect("inline boxes close in the order they were opened");
        self.mark(Mark::BoxEnd(index));
        let count = self.boxes.len();
        let inline_box = &mut self.boxes[index];
        inline_box.range.end = self.text.len();
        inline_box.descendants = count - index - 1;
        self.open_box = inline_box.parent;
    }
    pub(crate) fn open_contents(&mut self, link: Option<usize>) {
        let index = link.map(|link| {
            let index = self.links.len();
            self.mark(Mark::LinkStart(index));
            self.links.push(InlineLink {
                range: self.text.len()..self.text.len(),
                link,
            });
            index
        });
        self.open_links.push(index);
    }
    pub(crate) fn close_contents(&mut self) {
        if let Some(index) = self
            .open_links
            .pop()
            .expect("contents scopes close in tree order")
        {
            self.mark(Mark::LinkEnd(index));
            self.links[index].range.end = self.text.len();
        }
    }
    /// Ends this run of inline content before block-level content, returning it for
    /// `finish`. Open inline boxes are split: they close here and reopen in the next run.
    pub(crate) fn split(&mut self) -> Self {
        let links: Vec<_> = self
            .open_links
            .iter()
            .map(|index| index.map(|index| self.links[index].link))
            .collect();
        for _ in &links {
            self.close_contents();
        }
        let mut open = Vec::new();
        let mut inner = self.open_box;
        while let Some(index) = inner {
            let inline_box = &mut self.boxes[index];
            open.push((
                inline_box.style.clone(),
                inline_box.decoration.clone(),
                inline_box.navigation,
                inline_box.canvas_source,
            ));
            inline_box.continued[1] = true;
            inner = inline_box.parent;
        }
        for _ in &open {
            self.close_box();
        }
        let mut next = Self {
            next_source: self.next_source,
            ..Self::default()
        };
        for (style, decoration, navigation, canvas_source) in open.iter().rev() {
            next.open_continued(style, decoration.clone(), *navigation, true, *canvas_source);
        }
        for link in links {
            next.open_contents(link);
        }
        std::mem::replace(self, next)
    }
    /// Places an atomic inline in the line, as a placeholder styled by its parent `style`.
    pub(crate) fn atomic(
        &mut self,
        block: crate::document::BlockBox,
        style: &Style,
    ) -> Result<(), DocumentError> {
        self.emit_pending_space()?;
        let index = self.style_index(style);
        let source = self.next_source();
        let offset = self.text.len();
        self.push(
            OBJECT_REPLACEMENT_CHARACTER,
            index,
            source,
            1,
            self.open_box,
        )?;
        let span = self
            .spans
            .last_mut()
            .expect("the placeholder was just pushed");
        span.atomic = Some(self.atomics.len());
        self.atomics.push(Anchor {
            offset,
            block,
            inline_box: self.open_box,
        });
        Ok(())
    }
    /// Product of the open inline boxes' opacities. Block-level content splitting them is a
    /// fragment of each, so it takes their opacity as its own group.
    pub(crate) fn open_opacity(&self) -> f32 {
        let mut opacity = 1.0;
        let mut open = self.open_box;
        while let Some(index) = open {
            opacity *= self.boxes[index].style.opacity;
            open = self.boxes[index].parent;
        }
        opacity
    }
    /// An anchor can sit between blocks only outside an open inline box.
    pub(crate) fn is_between_blocks(&self) -> bool {
        self.text.is_empty() && self.open_box.is_none()
    }
    pub(crate) fn line_break(&mut self, style: &Style) -> Result<(), DocumentError> {
        self.drop_pending_space();
        let index = self.style_index(style);
        let source = self.next_source();
        self.push('\n', index, source, 1, self.open_box)?;
        if style.clear != Clear::None {
            self.clearances
                .push((self.text.len(), style.clear, style.rtl));
        }
        Ok(())
    }
    pub(crate) fn optional_break(&mut self, style: &Style) {
        if style.white_space.wraps() {
            self.mark(Mark::OptionalBreak(self.optional_breaks.len()));
            self.optional_breaks.push(self.text.len());
        }
    }
    fn push(
        &mut self,
        ch: char,
        index: usize,
        source: usize,
        count: usize,
        inline_box: Option<usize>,
    ) -> Result<(), DocumentError> {
        if count == 0 {
            return Ok(());
        }
        if count
            .checked_mul(ch.len_utf8())
            .and_then(|bytes| self.text.len().checked_add(bytes))
            .is_none_or(|length| length > MAX_INLINE_TEXT_BYTES)
        {
            return Err(DocumentError::TextExpansionLimit {
                limit: MAX_INLINE_TEXT_BYTES,
            });
        }
        let style = &self.styles[index];
        let start = self.text.len();
        self.text.extend(std::iter::repeat_n(ch, count));
        if let Some(last) = self.spans.last_mut()
            && last.source == source
            && last.style.same_text_style(style)
        {
            last.range.end = self.text.len();
        } else {
            self.spans.push(TextSpan {
                range: start..self.text.len(),
                style: style.clone(),
                inline_box,
                atomic: None,
                leader: false,
                source,
            });
        }
        Ok(())
    }
    /// Returns the paragraph, or the anchors of an empty run as block-level boxes.
    pub(crate) fn finish(
        self,
        style: &Style,
    ) -> Result<Result<Paragraph, Vec<crate::document::BlockContent>>, DocumentError> {
        if (self.text.is_empty()
            || (self.spans.iter().any(|span| span.leader)
                && self
                    .spans
                    .iter()
                    .all(|span| span.leader || self.text[span.range.clone()].trim().is_empty())))
            && !self.boxes.iter().any(InlineBox::has_edges)
            && !self
                .anchors
                .iter()
                .any(|anchor| anchor.inline_box.is_some())
        {
            return Ok(Err(non_text(self.anchors, self.events)));
        }
        let mut paragraph = Paragraph {
            expanded_tabs: self.expanded_tabs,
            marker_image: None,
            first_formatted_line: true,
            text: self.text,
            style: Box::new(style.clone()),
            spans: self.spans,
            anchors: self.anchors,
            boxes: self.boxes,
            atomics: self.atomics,
            optional_breaks: self.optional_breaks,
            clearances: self.clearances,
            links: self.links,
            events: self.events,
        };
        transform::apply(&mut paragraph)?;
        Ok(Ok(paragraph))
    }
}

fn non_text(
    anchors: Vec<Anchor>,
    events: Vec<crate::document::InlineEvent>,
) -> Vec<crate::document::BlockContent> {
    let mut output = Vec::new();
    let mut events = events.into_iter().peekable();
    for (index, anchor) in anchors.into_iter().enumerate() {
        let pending: Vec<_> =
            std::iter::from_fn(|| events.next_if(|event| event.anchors_before <= index))
                .map(|event| event.event)
                .collect();
        if !pending.is_empty() {
            output.push(crate::document::BlockContent::Events(pending));
        }
        output.push(crate::document::BlockContent::Positioned(anchor.block));
    }
    let remaining: Vec<_> = events.map(|event| event.event).collect();
    if !remaining.is_empty() {
        output.push(crate::document::BlockContent::Events(remaining));
    }
    output
}

pub(crate) fn is_css_whitespace(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\r' | '\n' | '\u{c}')
}

pub(crate) fn collapses_character(ch: char, white_space: WhiteSpace) -> bool {
    white_space.collapses_spaces()
        && is_css_whitespace(ch)
        && !(ch == '\n' && white_space.preserves_breaks())
}
