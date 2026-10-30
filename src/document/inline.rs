mod transform;
mod transform_maps;
mod transform_math;
use crate::document::DocumentError;
use crate::style::{Clear, Style, TextAlign, WhiteSpaceCollapse};
use std::ops::Range;
use std::sync::Arc;
pub(crate) use transform::OffsetMap;
pub(crate) use transform_maps::narrow_width;

pub(crate) const MAX_INLINE_TEXT_BYTES: usize = 64 * 1024 * 1024;
/// Stands for an atomic inline in paragraph text (CSS Text 3 §5.1).
pub(crate) const OBJECT_REPLACEMENT_CHARACTER: char = '\u{fffc}';

/// A generated content image, loaded, at the offset of the generated text it stands at.
pub(crate) struct GeneratedImage {
    pub offset: usize,
    pub image: Arc<crate::images::Image>,
}

pub(crate) struct TextSpan {
    pub range: Range<usize>,
    pub style: Style,
    /// Innermost inline box containing the text; `None` is the paragraph's root inline box.
    pub inline_box: Option<usize>,
    /// Atomic inline whose placeholder this span holds.
    pub atomic: Option<usize>,
    pub leader: bool,
    source: usize,
    single_character_source: bool,
}
/// Non-replaced inline box (CSS 2.1 §9.2.2), stored in tree order.
pub(crate) struct InlineBox {
    pub math: Option<crate::document::mathml::Class>,
    pub range: Range<usize>,
    /// Shared by every box an element's inline box is split into, which it identifies.
    pub style: Arc<Style>,
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
    marker_image: Option<crate::graphics::BackgroundImage>,
    first_formatted_line: bool,
    /// Holds the last line box of its block container.
    last_formatted_line: bool,
    /// Whether the paragraph's first and last lines are a block container's first and last
    /// formatted lines that it trims with `text-box-trim` (CSS Inline 3 §6.2).
    pub(crate) text_box_trim: [bool; 2],
    text: String,
    style: Box<Style>,
    spans: Vec<TextSpan>,
    anchors: Vec<Anchor>,
    boxes: Vec<InlineBox>,
    atomics: Vec<Anchor>,
    /// Ruby annotation boxes, each anchored at the end of its annotation in the base level.
    annotations: Vec<Anchor>,
    rubies: Vec<crate::document::ruby::Ruby>,
    annotation_align: Option<crate::style::RubyAlign>,
    optional_breaks: Vec<usize>,
    clearances: Vec<(usize, Clear, bool)>,
    links: Vec<InlineLink>,
    events: Vec<crate::document::InlineEvent>,
    /// The paragraph as `::first-line` styles its first formatted line, which shares its
    /// atomic inlines and line breaks (CSS Pseudo-Elements 4 §2.1).
    first_line: Option<Box<Paragraph>>,
    /// Where `first_line`, whose `text-transform` may encode characters differently, places
    /// this paragraph's offsets.
    first_line_offsets: OffsetMap,
}
impl Paragraph {
    pub(super) fn reflow(&mut self) -> Vec<&mut crate::document::BlockBox> {
        use crate::style::logical::{self, Flow};
        let reflow = |style: &Style| logical::reflowed(style, Flow::of(style).horizontal());
        let mut paragraphs = vec![self];
        let mut blocks = Vec::new();
        while let Some(paragraph) = paragraphs.pop() {
            if paragraph.style.used_writing_mode().vertical() {
                *paragraph.style = reflow(&paragraph.style);
            }
            for span in &mut paragraph.spans {
                if span.style.used_writing_mode().vertical() {
                    span.style = reflow(&span.style);
                }
            }
            for inline in &mut paragraph.boxes {
                let flow = Flow::of(&inline.style);
                if flow.writing_mode.vertical() {
                    inline.decoration =
                        logical::decoration(&inline.decoration, flow, flow.horizontal());
                    *Arc::make_mut(&mut inline.style) = reflow(&inline.style);
                }
            }
            blocks.extend(
                paragraph
                    .anchors
                    .iter_mut()
                    .chain(&mut paragraph.atomics)
                    .chain(&mut paragraph.annotations)
                    .map(|anchor| &mut anchor.block),
            );
            if let Some(first) = &mut paragraph.first_line {
                paragraphs.push(first);
            }
        }
        blocks
    }

    pub(crate) fn first_line(&self) -> Option<&Self> {
        self.first_line.as_deref()
    }
    pub(crate) fn first_line_offsets(&self) -> &OffsetMap {
        &self.first_line_offsets
    }
    pub(crate) fn set_text_box_trim(&mut self, trim: [bool; 2]) {
        self.text_box_trim = trim;
        if let Some(variant) = &mut self.first_line {
            variant.text_box_trim = trim;
        }
    }
    /// Lays the paragraph out as an inline initial letter's contents (CSS Inline 3 §7.5.3):
    /// its text at used font sizes `ratio` times their computed values, and its root inline
    /// box's text box edges the `edges` that hold the letter's alignment points, which its
    /// trimmed line box takes.
    pub(crate) fn size_initial_letter(&mut self, ratio: f32, edges: [crate::style::TextEdge; 2]) {
        self.style.size *= ratio;
        self.style.text_box.edges = edges;
        for span in &mut self.spans {
            span.style.size *= ratio;
        }
        for inline_box in &mut self.boxes {
            Arc::make_mut(&mut inline_box.style).size *= ratio;
        }
    }
    /// Records whether the paragraph holds its block container's first and last lines.
    pub(crate) fn set_formatted_lines(&mut self, first: bool, last: bool) {
        self.first_formatted_line = first;
        self.last_formatted_line = last;
        if let Some(variant) = &mut self.first_line {
            variant.first_formatted_line = first;
            variant.last_formatted_line = last;
        }
    }
    /// A copy of this paragraph before `text-transform` for its first formatted line: the
    /// `::first-line` boxes `lines` (outermost first, styled and decorated) wrap its contents,
    /// its root inline box takes `root`, and its spans, inline boxes and atomic inlines take
    /// `spans`, `boxes` and `atomics` (its atomic inlines, then its ruby annotations) in order.
    pub(crate) fn first_line_copy(
        &self,
        root: Style,
        lines: Vec<(Style, crate::graphics::BoxDecoration)>,
        spans: Vec<Style>,
        boxes: Vec<(Style, crate::graphics::BoxDecoration)>,
        atomics: Vec<Style>,
    ) -> Self {
        let count = lines.len();
        let shift = |inline_box: Option<usize>| {
            inline_box.map_or(count.checked_sub(1), |index| Some(index + count))
        };
        let end = self.text.len();
        let descendants = self.boxes.len();
        let mut copied: Vec<InlineBox> = lines
            .into_iter()
            .enumerate()
            .map(|(index, (style, decoration))| InlineBox {
                math: None,
                range: 0..end,
                style: style.into(),
                decoration,
                parent: index.checked_sub(1),
                descendants: count - 1 - index + descendants,
                continued: [false; 2],
                navigation: crate::document::BoxNavigation::default(),
                canvas_source: false,
            })
            .collect();
        copied.extend(
            self.boxes
                .iter()
                .zip(boxes)
                .map(|(inline_box, (style, decoration))| InlineBox {
                    math: inline_box.math.clone(),
                    range: inline_box.range.clone(),
                    style: style.into(),
                    decoration,
                    parent: shift(inline_box.parent),
                    descendants: inline_box.descendants,
                    continued: inline_box.continued,
                    navigation: inline_box.navigation,
                    canvas_source: inline_box.canvas_source,
                }),
        );
        let mut copy = Self {
            annotation_align: self.annotation_align,
            marker_image: None,
            first_formatted_line: self.first_formatted_line,
            last_formatted_line: self.last_formatted_line,
            text_box_trim: self.text_box_trim,
            text: self.text.clone(),
            style: Box::new(root),
            spans: self
                .spans
                .iter()
                .zip(spans)
                .map(|(span, style)| TextSpan {
                    range: span.range.clone(),
                    style,
                    inline_box: shift(span.inline_box),
                    atomic: span.atomic,
                    leader: span.leader,
                    source: span.source,
                    single_character_source: span.single_character_source,
                })
                .collect(),
            anchors: Vec::new(),
            boxes: copied,
            atomics: Vec::new(),
            annotations: Vec::new(),
            rubies: self
                .rubies
                .iter()
                .map(|ruby| ruby.shifted(count, 0))
                .collect(),
            optional_breaks: self.optional_breaks.clone(),
            clearances: self.clearances.clone(),
            links: self
                .links
                .iter()
                .map(|link| InlineLink {
                    range: link.range.clone(),
                    link: link.link,
                })
                .collect(),
            events: Vec::new(),
            first_line: None,
            first_line_offsets: OffsetMap::default(),
        };
        let mut styles = atomics.into_iter();
        let mut restyled = |anchors: &[Anchor]| -> Vec<Anchor> {
            anchors
                .iter()
                .zip(styles.by_ref())
                .map(|(anchor, style)| Anchor {
                    offset: anchor.offset,
                    block: crate::document::BlockBox::anonymous(style, Vec::new(), None),
                    inline_box: shift(anchor.inline_box),
                })
                .collect()
        };
        copy.atomics = restyled(&self.atomics);
        copy.annotations = restyled(&self.annotations);
        copy
    }
    pub(crate) fn math_children(&self) -> Vec<crate::document::mathml::Class> {
        math_children(&self.boxes, &self.atomics, &self.spans, None)
    }
    pub(crate) fn generated(text: &str, style: &Style) -> Result<Option<Self>, DocumentError> {
        let mut collector = InlineCollector::default();
        collector.append(text, style)?;
        Ok(collector.finish(style)?.ok())
    }

    pub(crate) fn generated_output(
        output: &crate::style::GeneratedOutput,
        images: &[GeneratedImage],
        style: &Style,
    ) -> Result<Option<Self>, DocumentError> {
        let mut collector = InlineCollector::default();
        collector.append_generated(output, images, style)?;
        Ok(collector.finish(style)?.ok())
    }

    /// The marker box holding its marker string or the `content` of its `::marker` (CSS
    /// Lists 3 §3.2), after its white space is processed; `None` where none remains.
    pub(crate) fn generated_marker(
        output: &crate::style::GeneratedOutput,
        images: &[GeneratedImage],
        style: &Style,
    ) -> Result<Option<Self>, DocumentError> {
        Self::marker_with(style, |collector, style| {
            collector.append_generated(output, images, style)
        })
    }
    fn marker_with(
        style: &Style,
        fill: impl FnOnce(&mut InlineCollector, &Style) -> Result<(), DocumentError>,
    ) -> Result<Option<Self>, DocumentError> {
        let style = Self::marker_style(style);
        let mut collector = InlineCollector::default();
        fill(&mut collector, &style)?;
        Ok(collector.finish(&style)?.ok().map(|mut paragraph| {
            paragraph.first_formatted_line = false;
            paragraph.last_formatted_line = false;
            paragraph
        }))
    }
    /// An image marker: the image followed by a space, which never starts its line.
    pub(crate) fn image_marker(
        image: crate::graphics::BackgroundImage,
        style: &Style,
    ) -> Result<Self, DocumentError> {
        let style = Self::marker_style(style);
        let text = String::from(' ');
        let span = TextSpan {
            range: 0..text.len(),
            style: style.clone(),
            inline_box: None,
            atomic: None,
            leader: false,
            source: 0,
            single_character_source: true,
        };
        let mut paragraph = Self {
            marker_image: Some(image),
            annotation_align: None,
            first_formatted_line: false,
            last_formatted_line: false,
            text_box_trim: [false; 2],
            text,
            style: Box::new(style),
            spans: vec![span],
            anchors: Vec::new(),
            boxes: Vec::new(),
            atomics: Vec::new(),
            annotations: Vec::new(),
            rubies: Vec::new(),
            optional_breaks: Vec::new(),
            clearances: Vec::new(),
            links: Vec::new(),
            events: Vec::new(),
            first_line: None,
            first_line_offsets: OffsetMap::default(),
        };
        transform::apply(&mut paragraph, false)?;
        Ok(paragraph)
    }
    /// The marker box is a block container whose lone line starts at its start edge.
    fn marker_style(style: &Style) -> Style {
        let mut style = style.clone();
        style.align = TextAlign::Start;
        style.align_last = None;
        style.text_indent = Default::default();
        style
    }
    pub(crate) fn marker_image(&self) -> Option<&crate::graphics::BackgroundImage> {
        self.marker_image.as_ref()
    }
    /// CSS list marker default object size is one em square.
    pub(crate) fn marker_image_size(&self) -> Option<[f32; 2]> {
        let area = [self.style.size; 2];
        let natural = crate::paint::Source::of(self.marker_image.as_ref()?)
            .ok()
            .flatten()
            .map(|source| source.natural(area))
            .unwrap_or_default();
        Some(crate::graphics::image_size(
            &Default::default(),
            natural,
            area,
        ))
    }
    pub(crate) fn first_formatted_line(&self) -> bool {
        self.first_formatted_line
    }
    pub(crate) fn last_formatted_line(&self) -> bool {
        self.last_formatted_line
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
    /// Ruby annotation boxes, which the paragraph's rubies index.
    pub(crate) fn annotations(&self) -> &[Anchor] {
        &self.annotations
    }
    pub(super) fn align_annotation(&mut self, align: crate::style::RubyAlign) {
        self.annotation_align = Some(align);
        if let Some(first_line) = &mut self.first_line {
            first_line.align_annotation(align);
        }
    }
    pub(crate) fn annotation_align(&self) -> Option<crate::style::RubyAlign> {
        self.annotation_align
    }
    pub(crate) fn rubies(&self) -> &[crate::document::ruby::Ruby] {
        &self.rubies
    }
    pub(crate) fn inter_character_base_style(&self, atomic: usize) -> Option<&Style> {
        let annotation = atomic.checked_sub(self.atomics.len())?;
        for ruby in &self.rubies {
            for segment in &ruby.segments {
                for level in &segment.levels {
                    if !level.inter_character {
                        continue;
                    }
                    if let Some(annotation) = level
                        .annotations
                        .iter()
                        .find(|item| item.block == Some(annotation))
                    {
                        let base = if self.boxes[ruby.container].style.used_rtl() {
                            annotation.bases.start
                        } else {
                            annotation.bases.end - 1
                        };
                        return Some(&self.boxes[segment.bases[base]].style);
                    }
                }
            }
        }
        None
    }

    /// The style of the annotation box that annotation box `index` lays out.
    pub(crate) fn annotation_box_style(&self, index: usize) -> &Style {
        self.rubies
            .iter()
            .flat_map(|ruby| &ruby.segments)
            .flat_map(|segment| &segment.levels)
            .flat_map(|level| &level.annotations)
            .find(|annotation| annotation.block == Some(index))
            .map(|annotation| &*annotation.style)
            .expect("each annotation box belongs to a ruby annotation")
    }
    /// The boxes laid out unbroken in its lines: atomic inlines, then ruby annotations.
    pub(crate) fn inline_blocks(&self) -> impl Iterator<Item = &Anchor> {
        self.atomics.iter().chain(&self.annotations)
    }
    /// The box at `index` among `inline_blocks`.
    pub(crate) fn inline_block(&self, index: usize) -> &Anchor {
        self.atomics
            .get(index)
            .unwrap_or_else(|| &self.annotations[index - self.atomics.len()])
    }
    /// The boxes anchored in the paragraph: floats, positioned boxes, atomic inlines and ruby
    /// annotations.
    pub(crate) fn anchored_blocks_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut crate::document::BlockBox> {
        self.anchors
            .iter_mut()
            .chain(&mut self.atomics)
            .chain(&mut self.annotations)
            .map(|anchor| &mut anchor.block)
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
    /// The innermost relatively or sticky positioned box holding `inner` (an inline box, or
    /// `None` for the root) inside box `outer`, whose decorations over `inner` move with it
    /// (CSS Text Decoration 3 §2).
    pub(crate) fn positioned_within(
        &self,
        inner: Option<usize>,
        outer: Option<usize>,
    ) -> Option<usize> {
        std::iter::successors(inner, |&index| self.boxes[index].parent)
            .take_while(|&index| Some(index) != outer)
            .find(|&index| self.boxes[index].style.position != crate::style::Position::Static)
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
    /// The text and the atomic inlines whose placeholders it holds, in text order.
    pub(crate) fn into_text_and_atomics(self) -> (String, Vec<Anchor>) {
        (self.text, self.atomics)
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
    single_character_source: bool,
    inline_box: Option<usize>,
}

#[derive(Default)]
pub(crate) struct InlineCollector {
    text: String,
    spans: Vec<TextSpan>,
    styles: Vec<Style>,
    pending_space: Option<PendingSpace>,
    next_source: usize,
    anchors: Vec<Anchor>,
    boxes: Vec<InlineBox>,
    atomics: Vec<Anchor>,
    annotations: Vec<Anchor>,
    rubies: Vec<crate::document::ruby::Ruby>,
    optional_breaks: Vec<usize>,
    clearances: Vec<(usize, Clear, bool)>,
    open_box: Option<usize>,
    after_space: Vec<Mark>,
    links: Vec<InlineLink>,
    events: Vec<crate::document::InlineEvent>,
    open_links: Vec<Option<usize>>,
    /// Where an inside marker box ends, which content selecting a first letter ignores
    /// (CSS Pseudo-Elements 4 §2.2.2).
    marker_end: usize,
    /// Where a sunken initial letter ends, after which collapsible white space is collapsed
    /// away (CSS Inline 3 §7.8.1).
    sunken_letter_end: Option<usize>,
}
impl InlineCollector {
    /// Opens the inline marker box of a list item (CSS Lists 3 §3.5).
    pub(crate) fn open_marker(
        &mut self,
        style: &Style,
        navigation: crate::document::BoxNavigation,
    ) {
        self.open_box(
            style,
            crate::graphics::BoxDecoration::default(),
            navigation,
            false,
        );
    }
    pub(crate) fn close_marker(&mut self) {
        self.close_box();
        self.marker_end = self.text.len();
    }
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
            .is_none_or(|last| !last.same_text_style(style) || !last.origin.same(&style.origin))
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
        images: &[GeneratedImage],
        style: &Style,
    ) -> Result<(), DocumentError> {
        self.append_generated_from(output, images, 0, style)
    }
    /// Appends the generated text after its first `start` bytes, which hold no leader and
    /// precede its images, and each image as an anonymous inline replaced element (CSS
    /// Content 3 §2.2).
    pub(crate) fn append_generated_from(
        &mut self,
        output: &crate::style::GeneratedOutput,
        images: &[GeneratedImage],
        start: usize,
        style: &Style,
    ) -> Result<(), DocumentError> {
        let text = output.text();
        let mut from = start;
        let mut leaders = output.leaders().iter().peekable();
        let mut images = images.iter().peekable();
        loop {
            let leader = leaders.peek().map(|range| range.start);
            // An image at a leader's start precedes it.
            let image_next = match (leader, images.peek().map(|image| image.offset)) {
                (None, None) => break,
                (Some(leader), Some(image)) => image <= leader,
                (Some(_), None) => false,
                (None, Some(_)) => true,
            };
            if image_next {
                let image = images.next().expect("an image is next");
                self.append(&text[from..image.offset], style)?;
                self.atomic(
                    crate::document::BlockBox::anonymous_replaced(
                        style.anonymous(crate::style::Display::Inline),
                        image.image.clone(),
                        None,
                    ),
                    style,
                )?;
                from = image.offset;
            } else {
                let range = leaders.next().expect("a leader is next");
                self.append(&text[from..range.start], style)?;
                self.append_inner(&text[range.clone()], style, true)?;
                from = range.end;
            }
        }
        self.append(&text[from..], style)
    }
    /// Whether no in-flow content but an inside marker precedes the next content on the first
    /// line, where a first letter can begin (CSS Pseudo-Elements 4 §2.2.2).
    pub(crate) fn at_line_start(&self) -> bool {
        self.text.len() == self.marker_end
    }
    /// Whether the next inline-level box is the first-most of the first line: only its open
    /// first-child inline box ancestors precede it, where an initial letter can begin (CSS
    /// Inline 3 §7.3.1).
    pub(crate) fn at_first_child(&self) -> bool {
        let mut open = 0;
        let mut inner = self.open_box;
        while let Some(index) = inner {
            if self.boxes[index].continued[0] {
                return false;
            }
            open += 1;
            inner = self.boxes[index].parent;
        }
        self.text.is_empty() && self.atomics.is_empty() && open == self.boxes.len()
    }
    /// The style of the innermost open inline box, which content appended next lies in.
    pub(crate) fn open_style(&self) -> Option<&Style> {
        self.open_box.map(|index| &*self.boxes[index].style)
    }
    /// The innermost open inline box.
    pub(crate) fn innermost_box(&self) -> Option<usize> {
        self.open_box
    }
    /// Adds a ruby annotation box, returning its index among the annotations.
    pub(crate) fn push_annotation(&mut self, block: crate::document::BlockBox) -> usize {
        self.annotations.push(Anchor {
            offset: self.text.len(),
            block,
            inline_box: self.open_box,
        });
        self.annotations.len() - 1
    }
    /// Records a ruby container whose boxes this collector holds.
    pub(crate) fn push_ruby(&mut self, ruby: crate::document::ruby::Ruby) {
        self.rubies.push(ruby);
    }
    /// Places an initial letter box as an atomic inline styled by its parent `style`; one
    /// that inside marker `marker` makes ends the marker (CSS Inline 3 §7.3.1). The inline
    /// boxes open around it, which nothing precedes, start after it: it lies outside their
    /// start edges (§7.8.3).
    pub(crate) fn initial_letter(
        &mut self,
        block: crate::document::BlockBox,
        style: &Style,
        marker: bool,
    ) -> Result<(), DocumentError> {
        let sunken = block
            .style()
            .initial_letter
            .is_some_and(|initial| initial.sink > 1);
        let start = self.text.len();
        let ancestors = std::mem::take(&mut self.open_box);
        self.atomic(block, style)?;
        self.open_box = ancestors;
        for inline_box in &mut self.boxes {
            if inline_box.range.start == start {
                inline_box.range.start = self.text.len();
            }
        }
        if marker {
            self.marker_end = self.text.len();
        }
        if sunken {
            self.sunken_letter_end = Some(self.text.len());
        }
        Ok(())
    }
    /// Gives the collected content, which the first formatted line alone holds, the styles
    /// `restyle` computes through `::first-line`, decorating its inline boxes with
    /// `decorate` (CSS Pseudo-Elements 4 §2.1.1).
    pub(crate) fn restyle(
        &mut self,
        mut restyle: impl FnMut(&Style) -> Result<Style, DocumentError>,
        mut decorate: impl FnMut(&Style) -> Result<crate::graphics::BoxDecoration, DocumentError>,
    ) -> Result<(), DocumentError> {
        for style in &mut self.styles {
            *style = restyle(style)?;
        }
        for span in &mut self.spans {
            span.style = restyle(&span.style)?;
        }
        for inline_box in &mut self.boxes {
            inline_box.style = restyle(&inline_box.style)?.into();
            inline_box.decoration = decorate(&inline_box.style)?;
        }
        Ok(())
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
        let single_character_source = text.chars().nth(1).is_none();
        for ch in text.chars() {
            if ch == '\n' && style.white_space.preserves_breaks() {
                self.drop_pending_space();
                self.push('\n', index, source, single_character_source, inline_box)?;
            } else if collapses_character(ch, style.white_space.collapse) {
                // Collapsible white space after an inside marker is discarded, as its `pre`
                // white space intends (CSS Lists 3 §3.1.1, issues 4448 and 4891).
                if self.pending_space.is_none()
                    && (leader
                        || (self.text.len() != self.marker_end
                            && self.sunken_letter_end != Some(self.text.len())
                            && !self.text.ends_with('\n')))
                {
                    self.pending_space = Some(PendingSpace {
                        style: index,
                        source,
                        single_character_source,
                        inline_box,
                    });
                }
            } else if style.white_space.collapse.converts_to_spaces() && matches!(ch, '\n' | '\t') {
                self.emit_pending_space()?;
                self.push(' ', index, source, single_character_source, inline_box)?;
            } else {
                self.emit_pending_space()?;
                self.push(ch, index, source, single_character_source, inline_box)?;
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
            self.push(
                ' ',
                space.style,
                space.source,
                space.single_character_source,
                space.inline_box,
            )?;
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
    pub(crate) fn drop_pending_space(&mut self) {
        self.pending_space = None;
        self.after_space.clear();
    }
    pub(crate) fn anchor_count(&self) -> usize {
        self.anchors.len()
    }
    /// Removes the out-of-flow boxes anchored after the first `count`, which lie at the end.
    pub(crate) fn take_anchors_after(&mut self, count: usize) -> Vec<crate::document::BlockBox> {
        self.after_space
            .retain(|mark| !matches!(mark, Mark::Anchor(index) if *index >= count));
        self.anchors
            .split_off(count)
            .into_iter()
            .map(|anchor| anchor.block)
            .collect()
    }
    /// Appends `other`, collected apart with every box and link closed, as if its content had
    /// been appended here (CSS Display 3 §5 run-in boxes).
    pub(crate) fn extend(&mut self, other: Self) -> Result<(), DocumentError> {
        debug_assert!(
            other.open_box.is_none() && other.open_links.is_empty(),
            "a collector is appended after its boxes close"
        );
        if other.text.is_empty()
            && other.boxes.is_empty()
            && other.anchors.is_empty()
            && other.annotations.is_empty()
            && other.events.is_empty()
        {
            return Ok(());
        }
        self.emit_pending_space()?;
        let offset = self.text.len();
        if offset
            .checked_add(other.text.len())
            .is_none_or(|length| length > MAX_INLINE_TEXT_BYTES)
        {
            return Err(DocumentError::TextExpansionLimit {
                limit: MAX_INLINE_TEXT_BYTES,
            });
        }
        let parent = self.open_box;
        let boxes = self.boxes.len();
        let anchors = self.anchors.len();
        let atomics = self.atomics.len();
        let annotations = self.annotations.len();
        let breaks = self.optional_breaks.len();
        let links = self.links.len();
        let events = self.events.len();
        let styles = self.styles.len();
        let sources = self.next_source;
        let inline_box = |inner: Option<usize>| inner.map_or(parent, |inner| Some(inner + boxes));
        let shift = |range: Range<usize>| range.start + offset..range.end + offset;
        self.text.push_str(&other.text);
        self.spans
            .extend(other.spans.into_iter().map(|span| TextSpan {
                range: shift(span.range),
                inline_box: inline_box(span.inline_box),
                atomic: span.atomic.map(|atomic| atomic + atomics),
                source: span.source + sources,
                ..span
            }));
        self.styles.extend(other.styles);
        self.pending_space = other.pending_space.map(|space| PendingSpace {
            style: space.style + styles,
            source: space.source + sources,
            single_character_source: space.single_character_source,
            inline_box: inline_box(space.inline_box),
        });
        self.after_space
            .extend(other.after_space.into_iter().map(|mark| match mark {
                Mark::Anchor(index) => Mark::Anchor(index + anchors),
                Mark::BoxStart(index) => Mark::BoxStart(index + boxes),
                Mark::BoxEnd(index) => Mark::BoxEnd(index + boxes),
                Mark::OptionalBreak(index) => Mark::OptionalBreak(index + breaks),
                Mark::LinkStart(index) => Mark::LinkStart(index + links),
                Mark::LinkEnd(index) => Mark::LinkEnd(index + links),
                Mark::Event(index) => Mark::Event(index + events),
            }));
        self.next_source += other.next_source;
        let anchor = |anchor: Anchor| Anchor {
            offset: anchor.offset + offset,
            inline_box: inline_box(anchor.inline_box),
            block: anchor.block,
        };
        self.anchors.extend(other.anchors.into_iter().map(anchor));
        self.atomics.extend(other.atomics.into_iter().map(anchor));
        self.annotations
            .extend(other.annotations.into_iter().map(anchor));
        self.rubies.extend(
            other
                .rubies
                .iter()
                .map(|ruby| ruby.shifted(boxes, annotations)),
        );
        self.boxes
            .extend(other.boxes.into_iter().map(|inner| InlineBox {
                range: shift(inner.range),
                parent: inline_box(inner.parent),
                ..inner
            }));
        self.optional_breaks
            .extend(other.optional_breaks.into_iter().map(|at| at + offset));
        self.clearances.extend(
            other
                .clearances
                .into_iter()
                .map(|(at, clear, rtl)| (at + offset, clear, rtl)),
        );
        self.links
            .extend(other.links.into_iter().map(|link| InlineLink {
                range: shift(link.range),
                link: link.link,
            }));
        self.events.extend(
            other
                .events
                .into_iter()
                .map(|event| crate::document::InlineEvent {
                    offset: event.offset + offset,
                    anchors_before: event.anchors_before + anchors,
                    event: event.event,
                }),
        );
        Ok(())
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
        self.open_continued(
            Arc::new(style.clone()),
            decoration,
            navigation,
            false,
            canvas_source,
        );
    }
    fn open_continued(
        &mut self,
        style: Arc<Style>,
        decoration: crate::graphics::BoxDecoration,
        navigation: crate::document::BoxNavigation,
        continued: bool,
        canvas_source: bool,
    ) {
        let index = self.boxes.len();
        self.mark(Mark::BoxStart(index));
        self.boxes.push(InlineBox {
            math: None,
            range: self.text.len()..self.text.len(),
            style,
            decoration,
            parent: self.open_box,
            descendants: 0,
            continued: [continued, false],
            navigation,
            canvas_source,
        });
        self.open_box = Some(index);
    }
    pub(super) fn prepare_math_inline(&mut self, element: crate::dom::Element<'_>, style: &Style) {
        if element.namespace() != crate::dom::MATHML_NAMESPACE {
            return;
        }
        let Some(index) = self.open_box else {
            return;
        };
        let children = math_children(&self.boxes, &self.atomics, &self.spans, Some(index));
        self.boxes[index].math = crate::document::mathml::inline_class(element, style, children);
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
                inline_box.math.clone(),
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
        for (style, decoration, navigation, canvas_source, math) in open.into_iter().rev() {
            next.open_continued(style, decoration, navigation, true, canvas_source);
            next.boxes
                .last_mut()
                .expect("a continued box was opened")
                .math = math;
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
        let collapses_spaces = block.is_unadorned_image_fallback()
            && block.children().is_empty()
            && [&block.style().min_width, &block.style().min_height]
                .into_iter()
                .all(|size| *size == crate::style::Size::Auto || size.is_zero())
            && block.style().initial_letter.is_none();
        // Keep the pending space across an invisible replacement, while retaining its
        // placeholder for destinations, semantic events and phantom-line handling.
        if !collapses_spaces {
            self.emit_pending_space()?;
        }
        let index = self.style_index(style);
        let source = self.next_source();
        let offset = self.text.len();
        self.push(
            OBJECT_REPLACEMENT_CHARACTER,
            index,
            source,
            false,
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
    /// Styles of the open inline boxes, outermost first, which block-level content splits
    /// (CSS 2.1 §9.2.1.1).
    pub(crate) fn open_boxes(&self) -> Vec<Arc<Style>> {
        let mut open: Vec<_> =
            std::iter::successors(self.open_box, |&index| self.boxes[index].parent)
                .map(|index| Arc::clone(&self.boxes[index].style))
                .collect();
        open.reverse();
        open
    }
    /// Content appended now lies in an inline box.
    pub(crate) fn inside_inline_box(&self) -> bool {
        self.open_box.is_some()
    }
    /// The atomic inline last appended, if it lies in the open inline box and nothing but
    /// collapsible white space follows it.
    pub(crate) fn trailing_atomic(&mut self) -> Option<&mut crate::document::BlockBox> {
        let (open_box, end) = (self.open_box, self.text.len());
        self.atomics
            .last_mut()
            .filter(|anchor| {
                anchor.inline_box == open_box
                    && anchor.offset + OBJECT_REPLACEMENT_CHARACTER.len_utf8() == end
            })
            .map(|anchor| &mut anchor.block)
    }
    /// An anchor can sit between blocks only outside an open inline box.
    pub(crate) fn is_between_blocks(&self) -> bool {
        self.text.is_empty() && self.open_box.is_none()
    }
    pub(crate) fn line_break(&mut self, style: &Style) -> Result<(), DocumentError> {
        self.drop_pending_space();
        let index = self.style_index(style);
        let source = self.next_source();
        self.push('\n', index, source, false, self.open_box)?;
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
        single_character_source: bool,
        inline_box: Option<usize>,
    ) -> Result<(), DocumentError> {
        if self
            .text
            .len()
            .checked_add(ch.len_utf8())
            .is_none_or(|length| length > MAX_INLINE_TEXT_BYTES)
        {
            return Err(DocumentError::TextExpansionLimit {
                limit: MAX_INLINE_TEXT_BYTES,
            });
        }
        let style = &self.styles[index];
        let start = self.text.len();
        self.text.push(ch);
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
                single_character_source,
            });
        }
        Ok(())
    }
    /// Returns the paragraph, or the anchors of an empty run as block-level boxes.
    pub(crate) fn finish(
        self,
        style: &Style,
    ) -> Result<Result<Paragraph, Vec<crate::document::BlockContent>>, DocumentError> {
        self.finish_with_first_line(style, |_| Ok(None))
    }
    /// As `finish`, attaching the copy of the paragraph that `first_line` styles for its first
    /// formatted line, given the paragraph before `text-transform` (CSS Pseudo-Elements 4 §2.1).
    pub(crate) fn finish_with_first_line(
        self,
        style: &Style,
        first_line: impl FnOnce(&Paragraph) -> Result<Option<Paragraph>, DocumentError>,
    ) -> Result<Result<Paragraph, Vec<crate::document::BlockContent>>, DocumentError> {
        if ((self.text.is_empty() && self.annotations.is_empty())
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
            annotation_align: None,
            marker_image: None,
            first_formatted_line: true,
            last_formatted_line: true,
            text_box_trim: [false; 2],
            text: self.text,
            style: Box::new(style.clone()),
            spans: self.spans,
            anchors: self.anchors,
            boxes: self.boxes,
            atomics: self.atomics,
            annotations: self.annotations,
            rubies: self.rubies,
            optional_breaks: self.optional_breaks,
            clearances: self.clearances,
            links: self.links,
            events: self.events,
            first_line: None,
            first_line_offsets: OffsetMap::default(),
        };
        let variant = first_line(&paragraph)?;
        let units = transform::apply(&mut paragraph, variant.is_some())?;
        if let Some(mut variant) = variant {
            let copy = transform::apply(&mut variant, true)?;
            paragraph.first_line_offsets =
                OffsetMap::between([&copy, &units], [&variant.text, &paragraph.text]);
            paragraph.first_line = Some(Box::new(variant));
        }
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

pub(crate) fn collapses_character(ch: char, collapse: WhiteSpaceCollapse) -> bool {
    collapse.collapses_spaces()
        && is_css_whitespace(ch)
        && !(ch == '\n' && collapse.preserves_breaks())
}

fn math_children(
    boxes: &[InlineBox],
    atomics: &[Anchor],
    spans: &[TextSpan],
    parent: Option<usize>,
) -> Vec<crate::document::mathml::Class> {
    use crate::document::mathml::{Class, block_class};
    let mut children: Vec<_> = boxes
        .iter()
        .filter(|child| child.parent == parent)
        .map(|child| (child.range.start, child.math.clone().unwrap_or_default()))
        .collect();
    children.extend(
        atomics
            .iter()
            .filter(|child| child.inline_box == parent)
            .map(|child| (child.offset, block_class(&child.block))),
    );
    children.extend(
        spans
            .iter()
            .filter(|span| {
                span.inline_box == parent && span.atomic.is_none() && !span.range.is_empty()
            })
            .map(|span| {
                (
                    span.range.start,
                    Class {
                        space_like: true,
                        operator: None,
                    },
                )
            }),
    );
    children.sort_by_key(|(offset, _)| *offset);
    children.into_iter().map(|(_, class)| class).collect()
}
