use crate::document::navigation::{BoxNavigation, NavigationBuilder};
use crate::document::{DocumentError, Paragraph, inline::InlineCollector};
use crate::dom::{ChildNodes, DocumentTree, Element, ElementId, TreeRelation};
use crate::graphics::BoxDecoration;
use crate::style::{Display, GeneratedPseudoElement, PseudoElement, Style, StyleError, Stylesheet};

pub(super) const MAX_FORMATTING_DEPTH: usize = 512;
const DEFAULT_DENSITY: f32 = 1.0;
pub(super) const FORMATTING_NESTING_RESOURCE: &str = "formatting nesting";

/// The background colour and images of `decoration`, without its borders, outline or shadows.
pub(super) fn background_only<I: Clone>(
    decoration: &crate::paint::BoxDecoration<I>,
) -> crate::paint::BoxDecoration<I> {
    let mut backgrounds = decoration.backgrounds.clone();
    // CSS Backgrounds 4 §2.8: the canvas's painting area is not the root element's, so
    // `background-clip` has no effect there; a text clip leaves the border box.
    if backgrounds.clips_to_text() {
        for clip in &mut std::sync::Arc::make_mut(&mut backgrounds).clips {
            if let crate::paint::BackgroundClip::Painted {
                border_area: false,
                text: true,
            } = clip
            {
                *clip = crate::paint::BackgroundClip::default();
            } else if let crate::paint::BackgroundClip::Painted { text, .. } = clip {
                *text = false;
            }
        }
    }
    crate::paint::BoxDecoration {
        background: decoration.background,
        backgrounds,
        ..Default::default()
    }
}

/// PDF pages are white paper, which an opaque white canvas surface leaves unchanged.
const PAPER: crate::graphics::Color = crate::graphics::Color::WHITE;

/// The canvas surface colour lies beneath the propagated background colour and images
/// (CSS Color Adjustment 1 §2.2); `None` leaves what lies beneath visible.
fn canvas_surface(
    mut canvas: BoxDecoration,
    surface: Option<crate::graphics::Color>,
) -> BoxDecoration {
    let Some(surface) = surface else {
        return canvas;
    };
    let background = canvas.background;
    let channel = |index: usize| {
        (f32::from(background.rgb[index]) * background.alpha
            + f32::from(surface.rgb[index]) * (1.0 - background.alpha))
            .round() as u8
    };
    canvas.background = crate::graphics::Color {
        rgb: [channel(0), channel(1), channel(2)],
        alpha: surface.alpha,
    };
    canvas
}

/// The opaque canvas surface a document needs. Pages are white paper; an embedded document's
/// canvas is transparent unless its colour scheme differs from its embedding element's (CSS
/// Color Adjustment 1 §2.2).
fn surface_beneath(sheet: &Stylesheet, root: &Style) -> Option<crate::graphics::Color> {
    let surface = sheet.canvas_surface(root);
    match sheet.presentation() {
        crate::style::Presentation::Paged => (surface != PAPER).then_some(surface),
        crate::style::Presentation::Viewport { color_scheme } => {
            (sheet.used_color_scheme(&root.color_scheme) != color_scheme).then_some(surface)
        }
    }
}

#[derive(Clone)]
pub(crate) struct PageBox {
    pub request: crate::style::PageRequest,
    pub geometry: crate::style::PageGeometry,
    #[cfg(test)]
    pub style: Box<Style>,
    pub production: crate::style::PageProduction,
    pub decoration: BoxDecoration,
    /// The page context's counter properties (CSS Page 3 §6.1).
    pub counters: crate::style::CounterProperties,
    pub margins: Vec<PageMarginBox>,
    pub footnote: PageFootnoteArea,
}
/// The footnote area of a page (CSS GCPM 3 §2.4).
#[derive(Clone)]
pub(crate) struct PageFootnoteArea {
    pub style: Box<Style>,
    pub decoration: BoxDecoration,
}
#[derive(Clone)]
pub(crate) struct PageMarginBox {
    pub margin: crate::style::PageMargin,
    pub decoration: BoxDecoration,
}
pub(crate) struct Formatted {
    pub root: BlockBox,
    pub footnotes: Footnotes,
    pub navigation: NavigationBuilder,
    pub strings: crate::document::strings::StringAssignments,
    pub generated: crate::document::generated::GeneratedReferences,
    pub canvas: BoxDecoration,
    pub pages: crate::document::page::PageProvider,
    pub svg_definitions: crate::document::svg::SvgDefinitions,
    /// The document's counters where each page starts (CSS Page 3 §6.1).
    pub page_counters: Vec<(usize, crate::document::CounterSnapshot)>,
}

/// The footnote elements of a document (CSS GCPM 3 §2.2), in the order their elements end,
/// and the boxes the footnote area places them in.
#[derive(Default)]
pub(crate) struct Footnotes {
    pub elements: Vec<FootnoteElement>,
    pub bodies: Vec<FootnoteBody>,
}
#[derive(Clone)]
pub(crate) struct FootnoteElement {
    pub element: ElementId,
    pub policy: crate::style::FootnotePolicy,
    /// The body holding the footnote.
    pub body: usize,
}
/// A box of the footnote area: a block footnote's box, or an anonymous block whose inline
/// formatting context holds consecutive inline footnotes (CSS GCPM 3 §2.3).
pub(crate) struct FootnoteBody {
    pub block: BlockBox,
    /// The footnotes the body holds, in order.
    pub members: Vec<usize>,
    pub inline: bool,
}

/// Consecutive inline footnotes collected into an anonymous block while the next may join
/// its inline formatting context.
struct InlineFootnotes {
    style: Box<Style>,
    children: Vec<BlockContent>,
    inline: InlineCollector,
    members: Vec<usize>,
}
impl InlineFootnotes {
    /// The anonymous block holding the footnotes.
    fn finish(mut self) -> Result<FootnoteBody, DocumentError> {
        push_inline(&mut self.children, self.inline.finish(&self.style)?);
        mark_last_formatted_line(&mut self.children);
        Ok(FootnoteBody {
            block: BlockBox::anonymous(*self.style, self.children, None),
            members: self.members,
            inline: true,
        })
    }
}

/// Appends finished inline content to the `children` of a block: a paragraph, which holds the
/// block's first formatted line when nothing in flow precedes it, or the boxes and events of
/// content that forms no line.
fn push_inline(children: &mut Vec<BlockContent>, finished: Result<Paragraph, Vec<BlockContent>>) {
    match finished {
        Ok(mut paragraph) => {
            let first = !children
                .iter()
                .any(|child| matches!(child, BlockContent::Block(_) | BlockContent::Inline(_)));
            paragraph.set_formatted_lines(first, false);
            children.push(BlockContent::Inline(paragraph));
        }
        Err(stray) => children.extend(stray),
    }
}
/// Marks the paragraph that ends a block's in-flow `children`, if one does, as holding its
/// last formatted line.
fn mark_last_formatted_line(children: &mut [BlockContent]) {
    if let Some(BlockContent::Inline(paragraph)) = children
        .iter_mut()
        .rfind(|child| matches!(child, BlockContent::Block(_) | BlockContent::Inline(_)))
    {
        paragraph.set_formatted_lines(paragraph.first_formatted_line(), true);
    }
}

pub(crate) struct BlockBox {
    style: Box<Style>,
    flow: Option<Box<BlockBox>>,
    math: Option<Box<crate::document::mathml::Element>>,
    math_content: Option<Box<BlockBox>>,
    layout_flow: crate::style::logical::Flow,
    decoration: BoxDecoration,
    /// A float's `shape-outside` with its image loaded.
    shape_outside: crate::graphics::ShapeOutside,
    children: Vec<BlockContent>,
    marker: Option<Box<Paragraph>>,
    image: Option<std::sync::Arc<crate::images::Image>>,
    /// An unavailable HTML image whose alternative text is empty (HTML §15.4.2).
    empty_image_fallback: bool,
    /// The selected image source's pixel density, which divides its natural dimensions
    /// (HTML §4.8.4.3.7).
    density: f32,
    navigation: BoxNavigation,
    canvas_source: bool,
    event: Option<crate::document::SemanticEvent>,
    page: crate::document::PageBoundary,
    /// Generated by no element (CSS 2.1 §9.2.1.1).
    anonymous: bool,
    /// The element whose size styling consults, when the box is its principal box: a query
    /// container, or the containing block of an initial letter its padding sizes.
    query_container: Option<ElementId>,
    /// Styles of the inline boxes split around this block-level box, outermost first, whose
    /// positioning and compositing it shares (CSS 2.1 §9.2.1.1).
    enclosing_inline: Vec<std::sync::Arc<Style>>,
    initial_letter: Option<InitialLetterBox>,
    /// The rendered legend of its fieldset parent (HTML §15.3.12).
    rendered_legend: bool,
    /// The block-start and block-end sides of every column's first and last formatted lines
    /// that a multi-column container trims (CSS Inline 3 §6.2).
    column_text_trim: [bool; 2],
    /// An atomic inline's box as its block's first formatted line styles it, where the styles
    /// its contents inherit through `::first-line` change it (CSS Pseudo-Elements 4 §2.1.3).
    first_line_box: Option<Box<BlockBox>>,
}

/// An initial letter box (CSS Inline 3 §7.5): an atomic inline of its originating line that
/// does not grow it, and excludes the start of the lines after it that it reaches.
#[derive(Clone, Copy)]
pub(crate) struct InitialLetterBox {
    /// The containing block's root inline box, which sizes and positions it.
    pub surrounding: crate::text::LetterSurrounding,
    /// It arises from an atomic inline, whose content box edges are its alignment points,
    /// rather than an inline box whose contents give them (§7.4).
    pub atomic: bool,
}
pub(crate) enum BlockContent {
    Events(Vec<crate::document::SemanticEvent>),
    Block(BlockBox),
    Inline(Paragraph),
    /// Out-of-flow box (float or absolute positioning) anchored between flow blocks.
    Positioned(BlockBox),
}
impl BlockBox {
    pub(crate) fn math(&self) -> Option<&crate::document::mathml::Element> {
        self.math.as_deref()
    }
    pub(crate) fn math_content(&self) -> Option<&BlockBox> {
        self.math_content.as_deref()
    }
    pub(crate) fn math_attributes(&self) -> Option<&crate::document::mathml::Attributes> {
        self.math().map(|math| &math.attributes)
    }
    pub(super) fn set_math(&mut self, math: crate::document::mathml::Element) {
        if matches!(self.style.display, Display::BlockMath | Display::InlineMath) {
            let token = matches!(
                math.name.as_ref(),
                "mi" | "mn" | "mo" | "mtext" | "ms" | "annotation"
            );
            let children = std::mem::take(&mut self.children)
                .into_iter()
                .map(|child| {
                    if !token && let BlockContent::Inline(paragraph) = child {
                        let style = paragraph.style().anonymous(Display::Block);
                        BlockContent::Block(Self::anonymous(
                            style,
                            vec![BlockContent::Inline(paragraph)],
                            None,
                        ))
                    } else {
                        child
                    }
                })
                .collect();
            self.math_content = Some(Box::new(Self::anonymous(
                self.style.anonymous(Display::Block),
                children,
                None,
            )));
        }
        self.math = Some(Box::new(math));
    }
    pub(crate) fn flow(&self) -> Option<&BlockBox> {
        self.flow.as_deref()
    }
    pub(crate) fn layout_flow(&self) -> crate::style::logical::Flow {
        self.layout_flow
    }
    pub(super) fn reflow(&mut self, parent: crate::style::logical::Flow) {
        use crate::style::logical::{self, Flow};
        enum Entry<'a> {
            Block(&'a mut BlockBox, Flow, [Flow; 2]),
            Paragraph(&'a mut Paragraph, Flow, [Flow; 2]),
        }
        let mut pending = vec![Entry::Block(self, parent, [parent; 2])];
        while let Some(entry) = pending.pop() {
            let (block, parent, mut containers) = match entry {
                Entry::Block(block, parent, containers) => (block, parent, containers),
                Entry::Paragraph(paragraph, flow, containers) => {
                    pending.extend(
                        paragraph
                            .reflow()
                            .into_iter()
                            .map(|block| Entry::Block(block, flow, containers)),
                    );
                    continue;
                }
            };
            let parent = match block.style.position {
                crate::style::Position::Absolute => containers[0],
                crate::style::Position::Fixed => containers[1],
                _ => parent,
            };
            let computed = Flow::of(&block.style);
            if (block.style.display.is_table_internal()
                && block.style.display != Display::TableCell)
                || matches!(
                    block.style.display,
                    Display::RubyBaseContainer | Display::RubyTextContainer
                )
            {
                // Writing Modes 4 §3.1 excludes these structural boxes; descendants still inherit their computed value.
                block.style.principal_flow = Some(parent);
            }
            let own = Flow::used(&block.style);
            if block.style.contains_absolute() {
                containers[0] = own;
            }
            if block.style.contains_fixed() {
                containers[1] = own;
            }
            block.layout_flow = parent;
            block.style.anchor.physical_areas(own, parent);
            if own.writing_mode != parent.writing_mode && block.image.is_none() {
                let mut style = block.style.anonymous(block.style.display.clone());
                style.flex = block.style.flex.clone();
                style.grid = block.style.grid.clone();
                style.table = block.style.table.clone();
                style.multicol = block.style.multicol.clone();
                style.clamp = block.style.clamp.clone();
                style.text_box = block.style.text_box;
                let mut inner = Self::anonymous(style, Vec::new(), None);
                if block.style.display.is_table()
                    && block.style.table.collapse == crate::style::BorderCollapse::Collapse
                {
                    inner.style.decoration.borders = block.style.decoration.borders;
                    inner.decoration = block.decoration.clone();
                }
                inner.children = std::mem::take(&mut block.children);
                inner.marker = block.marker.take();
                block.flow = Some(Box::new(inner));
                block.style.display = if block.style.display == Display::TableCell {
                    Display::TableCell
                } else if block.style.display.is_inline_level() {
                    Display::InlineBlock
                } else {
                    Display::FlowRoot
                };
            }
            if let Some(inner) = &mut block.flow {
                inner.math = block.math.take();
                inner.math_content = block.math_content.take();
            }
            if parent.writing_mode.vertical() {
                let mut physical = (*block.style).clone();
                physical.writing_mode = parent.writing_mode;
                physical.rtl = parent.rtl;
                block.decoration =
                    logical::decoration(&block.decoration, parent, parent.horizontal());
                *block.style = logical::reflowed(&physical, parent.horizontal());
            }
            block.style.writing_mode = computed.writing_mode;
            block.style.rtl = computed.rtl;
            block.style.layout_writing_mode = parent.writing_mode;
            if let Some(inner) = &mut block.flow {
                pending.push(Entry::Block(inner, own, containers));
            }
            if let Some(content) = &mut block.math_content {
                pending.push(Entry::Block(content, own, containers));
            }
            for child in &mut block.children {
                match child {
                    BlockContent::Block(child) | BlockContent::Positioned(child) => {
                        pending.push(Entry::Block(child, own, containers));
                    }
                    BlockContent::Inline(paragraph) => {
                        pending.push(Entry::Paragraph(paragraph, own, containers));
                    }
                    BlockContent::Events(_) => {}
                }
            }
            if let Some(marker) = &mut block.marker {
                pending.push(Entry::Paragraph(marker, own, containers));
            }
        }
    }

    pub(super) fn anonymous(
        style: Style,
        children: Vec<BlockContent>,
        page_scope: Option<std::sync::Arc<str>>,
    ) -> Self {
        let decoration = style.decoration.with_image_type();
        Self {
            anonymous: true,
            ..Self::with_decoration(style, children, decoration, page_scope)
        }
    }
    /// An anonymous replaced box showing `image`, such as a part of a widget.
    pub(super) fn anonymous_replaced(
        style: Style,
        image: std::sync::Arc<crate::images::Image>,
        page_scope: Option<std::sync::Arc<str>>,
    ) -> Self {
        Self {
            image: Some(image),
            ..Self::anonymous(style, Vec::new(), page_scope)
        }
    }
    pub(crate) fn is_anonymous(&self) -> bool {
        self.anonymous
    }
    /// Adds `part` to the sequence of table parts this anonymous table wraps, whose cells
    /// share the anonymous row ending it (CSS 2.1 §17.2.1).
    fn join_table_part(&mut self, part: BlockBox, page_scope: Option<std::sync::Arc<str>>) {
        let mut parts = Vec::new();
        for child in std::mem::take(&mut self.children) {
            match child {
                BlockContent::Block(row)
                    if row.anonymous && row.style.display == Display::TableRow =>
                {
                    parts.extend(row.children);
                }
                child => parts.push(child),
            }
        }
        parts.push(BlockContent::Block(part));
        *self = Self::anonymous((*self.style).clone(), parts, page_scope);
    }
    pub(super) fn with_decoration(
        style: Style,
        children: Vec<BlockContent>,
        decoration: BoxDecoration,
        page_scope: Option<std::sync::Arc<str>>,
    ) -> Self {
        let children = if style.display.is_item_container() {
            let mut items = Vec::with_capacity(children.len());
            for child in children {
                match child {
                    BlockContent::Inline(paragraph) => {
                        if paragraph
                            .text()
                            .chars()
                            .all(crate::document::inline::is_css_whitespace)
                        {
                            items.extend(paragraph.into_non_text());
                        } else {
                            items.push(BlockContent::Block(Self::anonymous(
                                style.anonymous(Display::Block),
                                vec![BlockContent::Inline(paragraph)],
                                page_scope.clone(),
                            )));
                        }
                    }
                    child => items.push(child),
                }
            }
            items
        } else {
            children
        };
        let mut children =
            crate::document::tables::normalize(&style, children, page_scope.as_ref());
        // CSS Overflow 4 §5.1.1: children of a `-webkit-box` use its clamping values, and
        // `-webkit-discard` discards only in a vertical one.
        if style.flex.legacy_box {
            for child in &mut children {
                if let BlockContent::Block(block) = child {
                    block.style.clamp = crate::style::LineClamp {
                        continuation: match style.clamp.continuation {
                            crate::style::Continue::WebkitDiscard
                                if style.flex.direction.column() =>
                            {
                                crate::style::Continue::Discard
                            }
                            crate::style::Continue::WebkitDiscard => crate::style::Continue::Auto,
                            continuation => continuation,
                        },
                        ..style.clamp.clone()
                    };
                }
            }
        }
        let page = crate::document::PageBoundary::new(page_scope, &style, &children);
        Self {
            layout_flow: crate::style::logical::Flow::of(&style),
            style: Box::new(style),
            flow: None,
            math: None,
            math_content: None,
            decoration,
            shape_outside: crate::graphics::ShapeOutside::None,
            children,
            marker: None,
            image: None,
            empty_image_fallback: false,
            density: DEFAULT_DENSITY,
            navigation: BoxNavigation::default(),
            canvas_source: false,
            event: None,
            page,
            anonymous: false,
            query_container: None,
            enclosing_inline: Vec::new(),
            initial_letter: None,
            rendered_legend: false,
            column_text_trim: [false; 2],
            first_line_box: None,
        }
    }
    pub(crate) fn column_text_trim(&self) -> [bool; 2] {
        self.column_text_trim
    }
    /// The box laid out in place of this atomic inline when it lies on its block's first
    /// formatted line, if that line's styles change it.
    pub(crate) fn first_line_box(&self) -> Option<&BlockBox> {
        self.first_line_box.as_deref()
    }
    pub(crate) fn leading_events(&self) -> impl Iterator<Item = &crate::document::SemanticEvent> {
        self.children
            .iter()
            .take_while(|child| matches!(child, BlockContent::Events(_)))
            .flat_map(|child| match child {
                BlockContent::Events(events) => events.as_slice(),
                _ => &[],
            })
    }
    pub(crate) fn following_events<'a>(
        &'a self,
        preceding: &BlockBox,
    ) -> impl Iterator<Item = &'a crate::document::SemanticEvent> {
        let boundary = self
            .children
            .iter()
            .position(|child| match child {
                BlockContent::Block(block) | BlockContent::Positioned(block) => {
                    std::ptr::eq(block, preceding)
                }
                _ => false,
            })
            .map_or(self.children.len(), |index| index + 1);
        self.children[boundary..]
            .iter()
            .take_while(|child| matches!(child, BlockContent::Events(_)))
            .flat_map(|child| match child {
                BlockContent::Events(events) => events.as_slice(),
                _ => &[],
            })
    }
    pub(crate) fn semantic_event(&self) -> Option<&crate::document::SemanticEvent> {
        self.event.as_ref()
    }
    /// The semantic events of the box and its contents in document order, with the number of
    /// boxes visited.
    pub(crate) fn subtree_events(&self) -> (Vec<&crate::document::SemanticEvent>, usize) {
        let mut events = self.semantic_event().into_iter().collect();
        let visited = 1 + content_events(&self.children, &mut events);
        events.sort_by_key(|event| event.order.start);
        (events, visited)
    }
    pub(crate) fn image(&self) -> Option<&std::sync::Arc<crate::images::Image>> {
        self.image.as_ref()
    }
    pub(crate) fn is_unadorned_image_fallback(&self) -> bool {
        self.empty_image_fallback
            && self.style.width == crate::style::Size::Auto
            && self.style.height == crate::style::Size::Auto
            && !self.style.has_inline_edges([false; 2])
            && self.style.margin.iter().all(crate::style::Size::is_zero)
            && !self.decoration.is_visible()
    }
    pub(crate) fn image_density(&self) -> f32 {
        self.density
    }
    pub(crate) fn style(&self) -> &Style {
        &self.style
    }
    pub(crate) fn enclosing_inline(&self) -> &[std::sync::Arc<Style>] {
        &self.enclosing_inline
    }
    pub(super) fn take_enclosing_inline(&mut self) -> Vec<std::sync::Arc<Style>> {
        std::mem::take(&mut self.enclosing_inline)
    }
    pub(super) fn set_enclosing_inline(&mut self, enclosing: Vec<std::sync::Arc<Style>>) {
        self.enclosing_inline = enclosing;
    }
    pub(crate) fn decoration(&self) -> &BoxDecoration {
        &self.decoration
    }
    pub(crate) fn shape_outside(&self) -> &crate::graphics::ShapeOutside {
        &self.shape_outside
    }
    pub(crate) fn children(&self) -> &[BlockContent] {
        &self.children
    }
    /// Marks the paragraphs holding the first and last formatted lines that this box or the
    /// ancestors in `requested` trim (CSS Inline 3 §6.2): trimming passes into the first and
    /// last in-flow child blocks without padding or borders on that side, but not into or
    /// through flex, grid or table formatting contexts.
    pub(super) fn trim_text_boxes(&mut self, requested: [bool; 2]) {
        let mut pending = vec![(self, requested)];
        while let Some((block, requested)) = pending.pop() {
            // An inline initial letter's line box is trimmed to its alignment points (CSS Inline 3
            // §7.5.3).
            let requested = if block.initial_letter.is_some_and(|letter| !letter.atomic) {
                [true; 2]
            } else {
                requested
            };
            let own = block.style.text_box.trim;
            let display = &block.style.display;
            let requested = if display.is_item_container() || display.is_table() {
                [false; 2]
            } else if block.style.multicol.is_multicol() {
                // Trimming a multi-column container's first and last formatted lines trims
                // those of every column, which pagination finds.
                block.column_text_trim = [0, 1].map(|side| own[side] || requested[side]);
                [false; 2]
            } else {
                [0, 1].map(|side| own[side] || requested[side])
            };
            let in_flow: Vec<usize> = block
                .children
                .iter()
                .enumerate()
                .filter(|(_, child)| {
                    matches!(child, BlockContent::Block(_) | BlockContent::Inline(_))
                })
                .map(|(index, _)| index)
                .collect();
            let [first, last] = [in_flow.first().copied(), in_flow.last().copied()];
            if let Some(content) = &mut block.math_content {
                pending.push((content.as_mut(), [false; 2]));
            }
            for (index, child) in block.children.iter_mut().enumerate().rev() {
                let inherited = [
                    requested[0] && first == Some(index),
                    requested[1] && last == Some(index),
                ];
                match child {
                    BlockContent::Inline(paragraph) => {
                        paragraph.set_text_box_trim(inherited);
                        let start = pending.len();
                        pending.extend(
                            paragraph
                                .anchored_blocks_mut()
                                .map(|block| (block, [false; 2])),
                        );
                        pending[start..].reverse();
                    }
                    BlockContent::Block(block) => {
                        const TOP: usize = 0;
                        const BOTTOM: usize = 2;
                        let open = |side: usize| {
                            block.style.padding[side].is_zero()
                                && block.style.decoration.borders[side].used_width() == 0.0
                        };
                        let inherited = [inherited[0] && open(TOP), inherited[1] && open(BOTTOM)];
                        pending.push((block, inherited));
                    }
                    BlockContent::Positioned(block) => pending.push((block, [false; 2])),
                    BlockContent::Events(_) => {}
                }
            }
        }
    }
    /// In-flow children in flex order or table caption/group layout order.
    pub(crate) fn in_flow_children(&self) -> Vec<&BlockContent> {
        crate::document::page::in_flow_children(&self.style, &self.children)
    }
    pub(crate) fn page_boundary(&self) -> &crate::document::PageBoundary {
        &self.page
    }
    pub(crate) fn marker(&self) -> Option<&Paragraph> {
        self.marker.as_deref()
    }
    pub(crate) fn navigation(&self) -> BoxNavigation {
        self.navigation
    }
    pub(crate) fn is_canvas_source(&self) -> bool {
        self.canvas_source
    }
    pub(crate) fn initial_letter(&self) -> Option<&InitialLetterBox> {
        self.initial_letter.as_ref()
    }
    pub(crate) fn is_rendered_legend(&self) -> bool {
        self.rendered_legend
    }
    pub(crate) fn query_container(&self) -> Option<ElementId> {
        self.query_container
    }
}

/// Adds the semantic events of `children` and their contents to `events`, returning the
/// number of boxes visited.
fn content_events<'a>(
    children: &'a [BlockContent],
    events: &mut Vec<&'a crate::document::SemanticEvent>,
) -> usize {
    let mut pending = vec![children];
    let mut visited = 0;
    while let Some(children) = pending.pop() {
        for child in children {
            let blocks: Vec<&BlockBox> = match child {
                BlockContent::Events(values) => {
                    events.extend(values);
                    Vec::new()
                }
                BlockContent::Block(child) | BlockContent::Positioned(child) => vec![child],
                BlockContent::Inline(paragraph) => {
                    events.extend(paragraph.semantic_events().iter().map(|event| &event.event));
                    paragraph
                        .anchors()
                        .iter()
                        .chain(paragraph.inline_blocks())
                        .map(|anchor| &anchor.block)
                        .collect()
                }
            };
            for block in blocks {
                visited += 1;
                events.extend(block.semantic_event());
                pending.push(&block.children);
            }
        }
    }
    visited
}

/// Document-order state of the boxes being built: list counter scopes and hyperlinks.
pub(crate) struct Traversal<'a> {
    document: &'a DocumentTree,
    embedded: &'a crate::document::embedded::EmbeddedContent,
    root_style: Style,
    counters: crate::document::counters::Counters,
    navigation: NavigationBuilder,
    strings: crate::document::strings::StringAssignments,
    generated: crate::document::generated::GeneratedPass,
    quotes: crate::style::QuoteDepth,
    canvas: BoxDecoration,
    canvas_body: Option<ElementId>,
    footnotes: Footnotes,
    /// Inline footnotes that the next inline footnote may join, boxed as the traversal is
    /// held across nested formatting.
    inline_footnotes: Option<Box<InlineFootnotes>>,
    page_counters: Vec<(usize, crate::document::CounterSnapshot)>,
    /// The running elements open around the current element, innermost last.
    running: Vec<crate::style::PageStyleTarget>,
    pages: &'a crate::document::page::PageProvider,
}

impl Traversal<'_> {
    /// The style of the anonymous block holding the inline formatting context that the inline
    /// footnote `element` starts in a footnote area. It inherits from the area (CSS 2.1
    /// §9.2.1.1) of the page pagination placed it on, or until then the first page's; its
    /// slices on later pages keep the style of the box they fragment.
    fn inline_footnotes_style(
        &self,
        element: ElementId,
        sheet: &Stylesheet,
        resources: &mut ImageResources<'_>,
    ) -> Result<Style, DocumentError> {
        Ok(match self.generated.inline_footnote_page(element) {
            Some(request) => self
                .pages
                .resolve_with(sheet, &self.root_style, request.clone(), resources)?
                .footnote
                .style
                .anonymous(Display::Block),
            None => self.pages.first().footnote.style.anonymous(Display::Block),
        })
    }
    /// Ends the inline footnotes waiting for the next to join them.
    fn finish_inline_footnotes(&mut self) -> Result<(), DocumentError> {
        if let Some(open) = self.inline_footnotes.take() {
            for &member in &open.members {
                self.footnotes.elements[member].body = self.footnotes.bodies.len();
            }
            self.footnotes.bodies.push(open.finish()?);
        }
        Ok(())
    }
    /// Records the footnote `element` held by the body that the footnote area places next,
    /// returning its index.
    fn add_footnote(&mut self, element: ElementId, style: &Style) -> usize {
        self.footnotes.elements.push(FootnoteElement {
            element,
            policy: style.footnote_policy,
            body: self.footnotes.bodies.len(),
        });
        self.footnotes.elements.len() - 1
    }
    /// Binds generated content to the values pagination gives it: content inside a running
    /// element, `running` when it is one, shows the page counters of each copy of it.
    fn bind<'c>(
        &self,
        content: &'c crate::style::GeneratedContent,
        context: crate::style::PageStyleTarget,
        running: bool,
    ) -> Result<std::borrow::Cow<'c, crate::style::GeneratedContent>, DocumentError> {
        let owner = if running {
            Some(context)
        } else {
            self.running.last().copied()
        };
        Ok(match owner {
            Some(owner) => self.generated.bind_running(content, context, owner)?,
            None => self.generated.bind(content, context)?,
        })
    }
    /// The state formatting a copy of the running element `owner` showing the page counters
    /// `numbers` from its start: it shares the document's counters and navigation as they
    /// stand there and records nothing for the document.
    fn running_copy(
        &self,
        owner: crate::style::PageStyleTarget,
        numbers: crate::document::generated::PageNumbers,
    ) -> Self {
        Self {
            document: self.document,
            embedded: self.embedded,
            root_style: self.root_style.clone(),
            counters: self.counters.detached(),
            navigation: self.navigation.detached(),
            strings: crate::document::strings::StringAssignments::new(self.document),
            generated: self.generated.running_copy(numbers),
            quotes: self.quotes,
            canvas: BoxDecoration::default(),
            canvas_body: None,
            footnotes: Footnotes::default(),
            inline_footnotes: None,
            page_counters: Vec::new(),
            running: vec![owner],
            pages: self.pages,
        }
    }
    /// The state formatting a copy of the subtree starting here: it continues the document's
    /// counters, navigation, string assignments and footnotes as they stand, so that the copy's
    /// boxes take the indices the document's take, and records nothing for the document.
    fn subtree_copy(&self) -> Self {
        Self {
            document: self.document,
            embedded: self.embedded,
            root_style: self.root_style.clone(),
            counters: self.counters.detached(),
            navigation: self.navigation.detached(),
            strings: self.strings.detached(),
            generated: self.generated.detached(),
            quotes: self.quotes,
            canvas: BoxDecoration::default(),
            canvas_body: None,
            footnotes: Footnotes {
                elements: self.footnotes.elements.clone(),
                bodies: Vec::new(),
            },
            inline_footnotes: None,
            page_counters: Vec::new(),
            running: self.running.clone(),
            pages: self.pages,
        }
    }
}

/// The box of the atomic inline `frame` as the first formatted line of `block`, styled
/// `style`, gives it the styles it inherits through `::first-line` (CSS Pseudo-Elements 4
/// §2.1.3), formatted again from its start; `None` where no `::first-line` rule styles that
/// line. Kept out of `block`, whose frame nested navigables' formatting stacks.
#[inline(never)]
fn first_line_box<'a>(
    frame: &Frame<'_>,
    (style, origins): (&Style, &[ElementId]),
    sheet: &'a Stylesheet,
    traversal: &Traversal<'a>,
    resources: &mut ImageResources<'a>,
) -> Result<Option<Box<BlockBox>>, DocumentError> {
    let Some(mut styles) = crate::document::first_line::FirstLineStyles::of(
        style,
        origins,
        sheet,
        traversal.document,
        resources,
    )?
    else {
        return Ok(None);
    };
    let root = Frame::new(
        frame.element,
        Box::new(styles.style(&frame.style)?),
        frame.depth,
        frame.ordinal,
        Some(Placement::Root),
        frame.page_scope.as_ref(),
    );
    block(root, sheet, &mut traversal.subtree_copy(), resources).map(|copy| Some(Box::new(copy)))
}

/// Copies of the running element of `frame`, formatted from its start once for each set of
/// page counters its copies in page-margin boxes show (CSS GCPM 3 §1.2.2).
fn running_copies<'a>(
    frame: &Frame<'_>,
    sheet: &'a Stylesheet,
    traversal: &Traversal<'a>,
    resources: &mut ImageResources<'a>,
) -> Result<Vec<(crate::document::generated::PageNumbers, BlockBox)>, DocumentError> {
    let owner = crate::style::PageStyleTarget {
        element: frame.element.id(),
        pseudo: None,
    };
    if traversal.generated.formats_copy() {
        return Ok(Vec::new());
    }
    traversal
        .generated
        .running_pages(owner)
        .iter()
        .map(|&numbers| {
            let mut copy = traversal.running_copy(owner, numbers);
            // The used page name of the element is its own, or the one it inherits.
            let root = Frame::new(
                frame.element,
                frame.style.clone(),
                frame.depth,
                frame.ordinal,
                Some(Placement::Root),
                frame.page_scope.as_ref(),
            );
            Ok((numbers, block(root, sheet, &mut copy, resources)?))
        })
        .collect()
}

struct BlockBuilder<'a, 's, 'c> {
    sheet: &'a Stylesheet,
    /// The block's element, which `style` styles.
    element: ElementId,
    style: &'s Style,
    /// Whether an initial letter is sized by the block's inline size, which layout measures.
    sized: &'s mut bool,
    element_page_scope: &'s Option<std::sync::Arc<str>>,
    children: &'s mut Vec<BlockContent>,
    inline: &'s mut InlineCollector,
    run_in: &'s mut Option<RunInSequence>,
    first_line: &'s mut FirstLine,
    letter: &'s mut Option<Box<OpenLetter>>,
    ruby: &'s mut crate::document::ruby::RubyStack,
    traversal: &'c mut Traversal<'a>,
    resources: &'c mut ImageResources<'a>,
}

/// An inline initial letter whose contents are collected apart from the line it originates
/// in (CSS Inline 3 §7.5), which it joins as an atomic inline once its box closes.
struct OpenLetter {
    /// The block's inline content before the letter.
    outer: InlineCollector,
    style: Style,
    decoration: BoxDecoration,
    navigation: BoxNavigation,
    /// The style of the letter's parent, which its placeholder takes.
    parent: Style,
    /// Inline boxes opened inside the letter and still open.
    depth: usize,
}

/// A block container's first formatted line while it is still to come (CSS Pseudo-Elements 4
/// §2.1.1), which its first in-flow block-level child may hold instead.
#[derive(Default)]
struct FirstLine {
    /// The elements whose first formatted line it is, outermost first.
    origins: Option<std::rc::Rc<[ElementId]>>,
    /// Its first-letter text is still to come (CSS Pseudo-Elements 4 §2.2.2).
    letter: bool,
}
impl FirstLine {
    fn of(origins: std::rc::Rc<[ElementId]>) -> Self {
        Self {
            origins: Some(origins),
            letter: true,
        }
    }
}

/// A run-in sequence of a block's children (CSS Display 3 §5), collected apart from the
/// inline content before it until the content after it decides where it goes.
struct RunInSequence {
    preceding: InlineCollector,
    /// The sequence ran into this block and follows its marker.
    reparented: bool,
    /// Out-of-flow boxes anchored before the last run-in box ended, which belong to the
    /// sequence; later ones only follow it.
    anchors: usize,
    /// A run-in box was laid out last and `anchors` has yet to record its end.
    open: bool,
    /// Collapsible white space after the last run-in box: between run-ins it separates
    /// them, and before a block it stays behind.
    whitespace: Vec<(String, Style)>,
}

/// What a block's next child is to a run-in sequence before it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunInFollower {
    /// Collapsible white space, an out-of-flow box or another run-in: part of the sequence.
    Member,
    /// A block box that establishes no formatting context, which the sequence runs into.
    Block,
    Other,
}
/// A same-document reference is a URL fragment.
const SAME_DOCUMENT: char = '#';
/// Any referencing presentation attribute links the documents a reference names.
const LINKING_PROPERTY: &str = "filter";

pub(crate) struct ImageResources<'a> {
    pub base: &'a crate::resources::ResourceBase,
    pub resolver: &'a dyn crate::resources::ResourceResolver,
    pub cache: &'a mut crate::images::ImageCache,
    pub elements: &'a crate::document::element_images::ElementImages,
}
impl ImageResources<'_> {
    /// The image `reference` names as the element `style` uses it (CSS Images 4 §2.5, §2.7).
    /// A `url()` that cannot be loaded aborts the document; an `image()` is then invalid, with
    /// a diagnostic.
    fn load(
        &mut self,
        reference: &crate::paint::ImageReference<'_>,
        style: &Style,
    ) -> Result<Option<std::sync::Arc<crate::images::Image>>, crate::images::ImageError> {
        let request = match reference {
            crate::paint::ImageReference::Url(request) => request,
            crate::paint::ImageReference::Element(id) => return Ok(self.element(id, style)),
        };
        let namespace = style.element_font_namespace();
        let image = if request.function.is_some() {
            self.cache
                .load_optional(request.url, self.base, self.resolver, namespace)
        } else {
            Some(
                self.cache
                    .load_in_namespace(request.url, self.base, self.resolver, namespace)?,
            )
        };
        Ok(image.and_then(|image| self.view(image, request, style)))
    }
    /// An image whose fallback is chosen before layout, as a list marker's or a float shape's
    /// is: `None` when it cannot be loaded or decoded, with a diagnostic.
    pub(crate) fn load_decoded(
        &mut self,
        reference: &crate::paint::ImageReference<'_>,
        style: &Style,
    ) -> Option<std::sync::Arc<crate::images::Image>> {
        let request = match reference {
            crate::paint::ImageReference::Url(request) => request,
            crate::paint::ImageReference::Element(id) => return self.element(id, style),
        };
        let image = self.cache.load_decoded(
            request.url,
            self.base,
            self.resolver,
            style.element_font_namespace(),
        )?;
        self.view(image, request, style)
    }
    fn view(
        &mut self,
        image: std::sync::Arc<crate::images::Image>,
        request: &crate::paint::ImageRequest<'_>,
        style: &Style,
    ) -> Option<std::sync::Arc<crate::images::Image>> {
        self.cache.view(image, request, self.base, style)
    }
    /// What `element(#id)` represents for the element `style` belongs to (CSS Images 4
    /// §2.7): `None` for an invalid image.
    fn element(&mut self, id: &str, style: &Style) -> Option<std::sync::Arc<crate::images::Image>> {
        use crate::document::element_images::Resolved;
        let owner = style.origin.node().map(|node| node.element);
        let namespace = crate::fonts::FontNamespace::ROOT;
        match self.elements.resolve(id, owner) {
            Resolved::Invalid => None,
            Resolved::Rendering(element, size) => Some(std::sync::Arc::new(
                crate::images::Image::Element(crate::images::ElementImage { element, size }),
            )),
            // A paint source shows its natural appearance, without the using element's
            // `image-orientation`, `object-view-box` or `image-resolution`.
            Resolved::Image(url, density) => self
                .cache
                .load_optional(url, self.base, self.resolver, namespace)
                .map(|image| crate::images::content_image(image, &Style::default(), density)),
            Resolved::Poster(poster) => Some(
                poster
                    .and_then(|poster| {
                        self.cache
                            .load_optional(poster, self.base, self.resolver, namespace)
                    })
                    .map_or_else(
                        || std::sync::Arc::new(crate::images::Image::Transparent),
                        |image| crate::images::content_image(image, &Style::default(), None),
                    ),
            ),
            Resolved::Svg(xml, styling) => {
                match self.cache.inline_svg_in_namespace(
                    xml,
                    styling,
                    self.base,
                    self.resolver,
                    namespace,
                ) {
                    Ok(image) => Some(image),
                    Err(error) => {
                        let reference = format!("element(#{id})");
                        self.cache.record_failure(self.base, &reference, error);
                        None
                    }
                }
            }
        }
    }
    /// A `content` image of the element `style` belongs to, as its replaced box shows it;
    /// `None` for an invalid image, with a diagnostic when it cannot be loaded (CSS Content 3
    /// §2.2). The images a generated image combines are decorative.
    pub(crate) fn content_image(
        &mut self,
        image: &crate::paint::BackgroundImage,
        style: &Style,
    ) -> Option<std::sync::Arc<crate::images::Image>> {
        let namespace = style.element_font_namespace();
        // The image-set() resolution of a URL that is the content image itself.
        let resolution = match image {
            crate::paint::BackgroundImage::Url { resolution, .. } => Some(*resolution),
            _ => None,
        };
        let mut loaded = image.with_image_type();
        let Ok(()) = loaded.load(&mut |reference| {
            let request = match reference {
                crate::paint::ImageReference::Url(request) => request,
                crate::paint::ImageReference::Element(id) => {
                    return Ok::<_, std::convert::Infallible>(self.element(id, style));
                }
            };
            Ok(self
                .cache
                .load_optional(request.url, self.base, self.resolver, namespace)
                .and_then(|image| match resolution {
                    Some(resolution) => self
                        .cache
                        .content_view(image, request, self.base, style, resolution),
                    None => self.view(image, request, style),
                }))
        });
        match loaded {
            crate::graphics::BackgroundImage::Image { image, .. } => Some(image),
            generated => generated
                .is_displayable()
                .then(|| generated_image(generated, style)),
        }
    }
    /// The image a `<content-replacement>` fills its element with: an invalid image is
    /// transparent with zero natural size (CSS Content 3 §1).
    pub(crate) fn replacement_image(
        &mut self,
        image: &crate::paint::BackgroundImage,
        style: &Style,
    ) -> std::sync::Arc<crate::images::Image> {
        self.content_image(image, style).unwrap_or_else(|| {
            std::sync::Arc::new(crate::images::Image::Generated(std::sync::Arc::new(
                crate::graphics::BackgroundImage::Invalid,
            )))
        })
    }
    /// The images of generated content, each loaded as its anonymous replaced box shows it;
    /// invalid ones generate nothing (CSS Content 3 §2.2).
    pub(crate) fn generated_images(
        &mut self,
        output: &crate::style::GeneratedOutput,
        style: &Style,
    ) -> Vec<crate::document::inline::GeneratedImage> {
        output
            .images()
            .iter()
            .filter_map(|(offset, image)| {
                Some(crate::document::inline::GeneratedImage {
                    offset: *offset,
                    image: self.content_image(image, style)?,
                })
            })
            .collect()
    }
    /// Loads a float's `shape-outside` image; other boxes have no float area. An image that
    /// cannot be fetched or decoded leaves `none` (CSS Shapes 1 §6.1) and a diagnostic.
    pub(crate) fn shape_outside(&mut self, style: &Style) -> crate::graphics::ShapeOutside {
        if style.floating().is_none() {
            return crate::graphics::ShapeOutside::None;
        }
        let Ok(shape) = style.shape_outside.load_image(&mut |request| {
            Ok::<_, std::convert::Infallible>(self.load_decoded(request, style))
        });
        shape
    }
    /// Loads the box's `url()` background layers and border image; inline style URLs use the
    /// document base.
    pub(crate) fn backgrounds(&mut self, style: &Style) -> Result<BoxDecoration, DocumentError> {
        self.link_documents(style)?;
        Ok(style
            .decoration
            .load_images(|request| self.load(request, style))?)
    }
    /// Loads the CSS mask layer images of SVG content, and the other SVG documents its masks
    /// reference.
    pub(crate) fn masks(
        &mut self,
        style: &Style,
    ) -> Result<std::sync::Arc<crate::graphics::LoadedMasks>, DocumentError> {
        self.link_documents(style)?;
        let masks = crate::paint::BoxDecoration {
            masks: style.decoration.masks.clone(),
            ..crate::paint::BoxDecoration::default()
        };
        Ok(masks
            .load_images(|request| self.load(request, style))?
            .masks)
    }
    /// Loads the other SVG documents whose elements the box's filters, masks and clip path
    /// reference (Filter Effects 1 §5, CSS Masking 1 §6.1, §7.1), with the documents those
    /// reference in turn, for painting to find.
    fn link_documents(&mut self, style: &Style) -> Result<(), DocumentError> {
        let masks = &style.decoration.masks;
        let references = style
            .filter
            .iter()
            .chain(style.backdrop_filter.iter())
            .filter_map(|function| match function {
                crate::paint::FilterFunction::Url(reference) => Some(&**reference),
                _ => None,
            })
            .chain(masks.sources.iter().flatten().map(|reference| &**reference))
            .chain(masks.clip_reference.as_deref());
        for reference in references {
            if reference.starts_with(SAME_DOCUMENT) {
                continue;
            }
            let failure = std::cell::RefCell::new(None);
            let load = |url: &str| {
                self.cache
                    .linked_document(url, self.base, self.resolver)
                    .unwrap_or_else(|error| {
                        failure.borrow_mut().get_or_insert(error);
                        None
                    })
            };
            // Linking the reference loads its documents; the box's geometry is not known yet.
            crate::svg::reference_tree(
                "",
                crate::svg::DocumentStyling::default(),
                LINKING_PROPERTY,
                reference,
                None,
                &load,
            );
            if let Some(error) = failure.into_inner() {
                return Err(error.into());
            }
        }
        Ok(())
    }
}

/// A generated image as the content image of the element `style` belongs to.
fn generated_image(
    image: crate::graphics::BackgroundImage,
    style: &Style,
) -> std::sync::Arc<crate::images::Image> {
    crate::images::content_image(
        std::sync::Arc::new(crate::images::Image::Generated(std::sync::Arc::new(image))),
        style,
        None,
    )
}

/// Maps a font metrics failure while sizing an initial letter to the document error the
/// renderer reports for its style.
fn letter_text_error(error: crate::text::TextError) -> DocumentError {
    match error {
        crate::text::TextError::Font(error) => StyleError::Font(error).into(),
        error => DocumentError::Formatting(error.to_string()),
    }
}

/// The block-start and block-end sides of a box.
const BLOCK_AXIS_SIDES: [usize; 2] = [0, 2];

/// An initial letter's block-axis padding and borders, which size it (CSS Inline 3 §7.5.3),
/// with percentages of its containing block's inline size `basis`.
fn block_axis_edges(style: &Style, basis: f32) -> f32 {
    BLOCK_AXIS_SIDES
        .into_iter()
        .map(|side| {
            style.padding[side].resolve(basis) + style.decoration.borders[side].used_width()
        })
        .sum()
}

/// A segment break in document text.
const SEGMENT_BREAK: &str = "\n";

/// A typographic pseudo-element laid out as an inline box, which takes no float or position.
pub(super) fn inline_pseudo_style(mut style: Style) -> Style {
    style.display = Display::Inline;
    style.float = crate::style::Float::None;
    style.position = crate::style::Position::Static;
    style
}

impl<'a> BlockBuilder<'a, '_, '_> {
    /// Appends inline text, wrapping the first-letter text it begins with in its
    /// `::first-letter` boxes.
    fn append_text(&mut self, text: &str, style: &Style) -> Result<(), DocumentError> {
        if self.ruby.text(self.inline, text, style)? {
            return Ok(());
        }
        let overridden = self
            .ruby
            .annotation_mode()
            .filter(|mode| self.inline.open_style().is_none() && *mode != style.used_writing_mode())
            .map(|mode| {
                let mut style = style.clone();
                style.writing_mode = mode;
                style
            });
        let style = overridden.as_ref().unwrap_or(style);
        if self.ruby.in_annotation() && style.white_space.preserves_breaks() {
            for (index, part) in text.split(SEGMENT_BREAK).enumerate() {
                if index > 0 {
                    self.line_break(style)?;
                }
                self.inline.append(part, style)?;
            }
            return Ok(());
        }
        let start = self.first_letter(text, style)?;
        self.inline.append(&text[start..], style)
    }
    /// A forced line break, which inside a ruby annotation is a collapsible segment break,
    /// whatever its `white-space` (CSS Ruby 1 §2.2).
    fn line_break(&mut self, style: &Style) -> Result<(), DocumentError> {
        if self.ruby.in_annotation() {
            let mut collapsing = style.clone();
            collapsing.white_space.collapse = crate::style::WhiteSpaceCollapse::Collapse;
            return self.inline.append(SEGMENT_BREAK, &collapsing);
        }
        self.inline.line_break(style)
    }
    /// Appends generated text, as `append_text` does.
    fn append_generated(
        &mut self,
        output: &crate::style::GeneratedOutput,
        images: &[crate::document::inline::GeneratedImage],
        style: &Style,
    ) -> Result<(), DocumentError> {
        if !output.text().is_empty() || !images.is_empty() {
            self.ruby.content(self.inline)?;
        }
        // A first letter is the text before any leader or image.
        let leader = output
            .leaders()
            .first()
            .map(|range| range.start)
            .into_iter()
            .chain(images.first().map(|image| image.offset))
            .min()
            .unwrap_or(output.text().len());
        let start = self.first_letter(&output.text()[..leader], style)?;
        self.inline
            .append_generated_from(output, images, start, style)
    }
    /// Appends the first-letter text that `text`, the next inline content, begins with in its
    /// `::first-letter` boxes, returning the bytes consumed (CSS Pseudo-Elements 4 §2.2).
    fn first_letter(&mut self, text: &str, style: &Style) -> Result<usize, DocumentError> {
        if !self.first_line.letter {
            return Ok(0);
        }
        let Some(origins) = self.first_line.origins.clone().filter(|_| {
            self.inline.at_line_start()
                && self.run_in.is_none()
                && self.sheet.has_pseudo_rules(PseudoElement::FirstLetter)
        }) else {
            self.first_line.letter = false;
            return Ok(0);
        };
        let found =
            crate::document::first_letter::find(text, style.white_space.collapse, &style.language);
        let crate::document::first_letter::FirstLetter::Found {
            prefix,
            letter,
            suffix,
        } = found
        else {
            self.first_line.letter = found == crate::document::first_letter::FirstLetter::Pending;
            return Ok(0);
        };
        self.first_line.letter = false;
        self.inline.append(&text[..prefix.start], style)?;
        let mut parent = style.clone();
        let mut opened = 0;
        let mut floated = None;
        let mut initial = None;
        for &origin in origins.iter() {
            let element = self.element(origin);
            let pseudo = self
                .sheet
                .typographic_pseudo_with_resolver(
                    element,
                    &parent,
                    PseudoElement::FirstLetter,
                    self.resources.resolver,
                )?
                .used();
            let decoration = self.resources.backgrounds(&pseudo)?;
            let outer = std::mem::replace(&mut parent, inline_pseudo_style(pseudo.clone()));
            if floated.is_none() && initial.is_none() {
                if pseudo.floating().is_some() {
                    floated = Some((pseudo, decoration, std::mem::take(self.inline), opened));
                    opened = 0;
                    continue;
                }
                if pseudo.initial_letter.is_some() && self.initial_letter_allowed() {
                    initial = Some((
                        OpenLetter {
                            outer: std::mem::take(self.inline),
                            style: parent.clone(),
                            decoration,
                            navigation: self.traversal.navigation.descendant(),
                            parent: outer,
                            depth: 0,
                        },
                        opened,
                    ));
                    opened = 0;
                    continue;
                }
            }
            self.inline.open_box(
                &parent,
                decoration,
                self.traversal.navigation.descendant(),
                false,
            );
            opened += 1;
        }
        self.first_letter_part(
            &origins,
            PseudoElement::FirstLetterPrefix,
            &parent,
            &text[prefix],
        )?;
        self.inline.append(&text[letter], &parent)?;
        self.first_letter_part(
            &origins,
            PseudoElement::FirstLetterSuffix,
            &parent,
            &text[suffix.clone()],
        )?;
        for _ in 0..opened {
            self.inline.close_box();
        }
        if let Some((style, decoration, outer, outer_opened)) = floated {
            let contents = std::mem::replace(self.inline, outer);
            let children = match contents.finish(&style)? {
                Ok(paragraph) => vec![BlockContent::Inline(paragraph)],
                Err(stray) => stray,
            };
            let page_scope =
                crate::document::page::used_name(&style.page, self.element_page_scope.as_ref());
            let block = BlockBox {
                navigation: self.traversal.navigation.descendant(),
                ..BlockBox::with_decoration(style, children, decoration, page_scope)
            };
            self.out_of_flow(block);
            for _ in 0..outer_opened {
                self.inline.close_box();
            }
        }
        if let Some((letter, outer_opened)) = initial {
            self.finish_letter(letter, false)?;
            for _ in 0..outer_opened {
                self.inline.close_box();
            }
        }
        Ok(suffix.end)
    }
    /// Whether an initial letter can begin with the next inline-level box: the first-most
    /// one of the first line, outside any other initial letter (CSS Inline 3 §7.3.1).
    fn initial_letter_allowed(&self) -> bool {
        self.letter.is_none() && self.run_in.is_none() && self.inline.at_first_child()
    }
    /// Opens an inline box, which collects its contents apart as an initial letter where its
    /// style makes it one, and builds ruby structure where it is a ruby box.
    fn open_inline(
        &mut self,
        style: &Style,
        decoration: BoxDecoration,
        navigation: BoxNavigation,
        canvas_source: bool,
    ) -> Result<(), DocumentError> {
        if style.display.is_ruby() {
            return self
                .ruby
                .open(self.inline, style, decoration, navigation, self.style);
        }
        self.ruby.inline_box(self.inline)?;
        if let Some(letter) = self.letter.as_mut() {
            letter.depth += 1;
        } else if style.initial_letter.is_some() && self.initial_letter_allowed() {
            *self.letter = Some(Box::new(OpenLetter {
                parent: self.inline.open_style().unwrap_or(self.style).clone(),
                outer: std::mem::take(self.inline),
                style: style.clone(),
                decoration,
                navigation,
                depth: 0,
            }));
            return Ok(());
        }
        self.inline
            .open_box(style, decoration, navigation, canvas_source);
        Ok(())
    }
    /// Closes the inline box `open_inline` opened last, placing an initial letter it ends.
    fn close_inline(&mut self) -> Result<(), DocumentError> {
        if self.ruby.close(self.inline)? == crate::document::ruby::Closed::Ruby {
            return Ok(());
        }
        match self.letter.as_mut() {
            Some(letter) if letter.depth > 0 => letter.depth -= 1,
            Some(_) => {
                let letter = self.letter.take().expect("an open initial letter closes");
                return self.finish_letter(*letter, false);
            }
            None => {}
        }
        self.inline.close_box();
        Ok(())
    }
    /// Places an atomic inline styled by its parent `parent`, as an initial letter where its
    /// style makes it one.
    fn atomic(&mut self, mut block: BlockBox, parent: &Style) -> Result<(), DocumentError> {
        self.ruby.content(self.inline)?;
        if block.style.display.is_table_part() {
            return self.table_part(block, parent);
        }
        if block.style.initial_letter.is_none() || !self.initial_letter_allowed() {
            return self.inline.atomic(block, parent);
        }
        self.atomic_letter(&mut block)?;
        // The letter lies on the first formatted line, which lays out its `::first-line` box.
        if let Some(copy) = block
            .first_line_box
            .as_deref_mut()
            .filter(|copy| copy.style.initial_letter.is_some())
        {
            self.atomic_letter(copy)?;
        }
        self.inline.initial_letter(block, parent, false)
    }
    /// Makes the atomic inline `block` an initial letter box of this block's lines.
    fn atomic_letter(&mut self, block: &mut BlockBox) -> Result<(), DocumentError> {
        let context = crate::text::TextContext::new(&self.sheet.fonts, self.resources.resolver);
        let style = &mut block.style;
        let initial = style
            .initial_letter
            .expect("an initial letter has a size and sink");
        let surrounding = crate::text::LetterSurrounding::of(
            &context,
            self.style,
            style.initial_letter_align.alignment,
        )
        .map_err(letter_text_error)?;
        // §7.5.5: an automatic block size spans the letter's lines as border-box alignment
        // would, and is definite.
        if *style.height.basis() == crate::style::Size::Auto {
            let span = surrounding.span(initial.size);
            let height = match style.box_sizing {
                crate::style::BoxSizing::BorderBox => span,
                crate::style::BoxSizing::ContentBox => {
                    span - block_axis_edges(style, self.letter_basis(style))
                }
            };
            style.height = style.height.with_automatic(crate::style::Size::Value(
                crate::style::LengthPercentage::Length(height.max(0.0)),
            ));
        }
        block.initial_letter = Some(InitialLetterBox {
            surrounding,
            atomic: true,
        });
        Ok(())
    }
    /// The inline size of this block, the containing block of an initial letter styled
    /// `style` that its block-axis padding sizes, against which that padding's percentages
    /// resolve (CSS Box 4 §4.1). Layout measures it, so the document is formatted again
    /// until it reproduces the size used; before it has, they resolve against zero, as
    /// cyclic percentages do (CSS Sizing 3 §5.2.1).
    fn letter_basis(&mut self, style: &Style) -> f32 {
        if BLOCK_AXIS_SIDES.iter().all(|&side| {
            matches!(
                style.padding[side],
                crate::style::LengthPercentage::Length(_)
            )
        }) {
            return 0.0;
        }
        *self.sized = true;
        let Some(size) = self.sheet.query_sizes().size(self.element) else {
            return 0.0;
        };
        let inline = size[usize::from(self.style.used_writing_mode().vertical())];
        inline * self.style.effective_zoom
    }
    /// Places table part `part`, whose parent is an inline box, in the anonymous inline table
    /// of the parts just before it, or in a new one (CSS 2.1 §17.2.1).
    fn table_part(&mut self, part: BlockBox, parent: &Style) -> Result<(), DocumentError> {
        let page_scope = self.element_page_scope.clone();
        if let Some(table) = self
            .inline
            .trailing_atomic()
            .filter(|table| table.anonymous && table.style.display == Display::InlineTable)
        {
            table.join_table_part(part, page_scope);
            // White space between table parts is removed.
            self.inline.drop_pending_space();
            return Ok(());
        }
        self.inline.atomic(
            BlockBox::anonymous(
                parent.anonymous(Display::InlineTable),
                vec![BlockContent::Block(part)],
                page_scope,
            ),
            parent,
        )
    }
    /// Places the inline initial letter `letter`, whose contents the collector holds, in the
    /// content before it; one that inside marker `marker` makes ends the marker.
    fn finish_letter(&mut self, letter: OpenLetter, marker: bool) -> Result<(), DocumentError> {
        let OpenLetter {
            outer,
            mut style,
            mut decoration,
            navigation,
            parent,
            ..
        } = letter;
        let mut contents = std::mem::replace(self.inline, outer);
        // The letter lies on its block's first formatted line, so it inherits through
        // `::first-line` (CSS Pseudo-Elements 4 §2.1.1).
        if let Some(origins) = self.first_line.origins.clone()
            && let Some(mut styles) = crate::document::first_line::FirstLineStyles::of(
                self.style,
                &origins,
                self.sheet,
                self.traversal.document,
                self.resources,
            )?
        {
            let resources = &mut *self.resources;
            contents.restyle(
                |style| styles.style(style),
                |style| resources.backgrounds(style),
            )?;
            style = styles.style(&style)?;
            decoration = self.resources.backgrounds(&style)?;
        }
        // `text-indent` applies to block containers, which the letter is not (CSS Text 3 §8.1).
        style.text_indent = crate::style::TextIndent::default();
        // An automatically sized letter fits its contents, which `text-align` does not align
        // (CSS Inline 3 §7.5.6); its lines start at its start edge.
        if *style.width.basis() == crate::style::Size::Auto {
            style.align = crate::style::TextAlign::Start;
            style.align_last = None;
        }
        match contents.finish(&style)? {
            Ok(paragraph) => {
                let block = self.inline_letter_box(style, decoration, navigation, paragraph)?;
                self.inline.initial_letter(block, &parent, marker)
            }
            Err(stray) => {
                // An initial letter without contents is an empty inline box.
                self.inline.open_box(&style, decoration, navigation, false);
                for content in stray {
                    match content {
                        BlockContent::Events(events) => {
                            for event in events {
                                self.inline.semantic_event(event);
                            }
                        }
                        BlockContent::Positioned(block) => self.inline.anchor(block),
                        BlockContent::Block(_) | BlockContent::Inline(_) => {
                            unreachable!("empty inline content holds no flow boxes")
                        }
                    }
                }
                self.inline.close_box();
                Ok(())
            }
        }
    }
    /// Ends initial letter `letter` before the block-level content inside it, which splits its
    /// inline box (CSS 2.1 §9.2.1.1): the letter is its fragment on the originating line, and
    /// the fragments after the block are an ordinary inline box, whose used `initial-letter`
    /// is `normal` as it does not start its block's first line (CSS Inline 3 §7.3.1).
    fn split_letter(&mut self, letter: OpenLetter) -> Result<(), DocumentError> {
        let open = self.inline.close_open();
        let mut style = letter.style.clone();
        style.initial_letter = None;
        let (decoration, navigation) = (letter.decoration.clone(), letter.navigation);
        self.finish_letter(letter, false)?;
        self.inline.continue_box(&style, decoration, navigation);
        self.inline.reopen(open);
        Ok(())
    }
    /// An inside marker is an inline box opening the list item's inline content (CSS Lists 3
    /// §3.5): an image marker holds an inline replaced box followed by a space. A marker can
    /// be an initial letter (CSS Inline 3 §7.3.1).
    fn place_inside_marker(&mut self, marker: Paragraph) -> Result<(), DocumentError> {
        let style = &marker.style().clone();
        let navigation = self.traversal.navigation.descendant();
        let letter =
            (style.initial_letter.is_some() && self.initial_letter_allowed()).then(|| OpenLetter {
                parent: self.inline.open_style().unwrap_or(self.style).clone(),
                outer: std::mem::take(self.inline),
                style: style.clone(),
                decoration: BoxDecoration::default(),
                navigation,
                depth: 0,
            });
        if letter.is_none() {
            self.inline.open_marker(style, navigation);
        }
        if let Some(image) = marker.marker_image() {
            let [width, height] = marker.marker_image_size().expect("image marker has size");
            let mut box_style = style.anonymous(Display::InlineBlock);
            box_style.width =
                crate::style::Size::Value(crate::style::LengthPercentage::Length(width));
            box_style.height =
                crate::style::Size::Value(crate::style::LengthPercentage::Length(height));
            let mut decoration = BoxDecoration::default();
            std::sync::Arc::make_mut(&mut decoration.backgrounds).images = vec![image.clone()];
            let atomic = BlockBox::with_decoration(
                box_style,
                Vec::new(),
                decoration,
                self.element_page_scope.clone(),
            );
            self.inline.atomic(atomic, style)?;
        }
        // Content images stand at placeholders in the marker's text.
        let (text, atomics) = marker.into_text_and_atomics();
        let mut from = 0;
        for atomic in atomics {
            self.inline.append(&text[from..atomic.offset], style)?;
            self.inline.atomic(atomic.block, style)?;
            from = atomic.offset + crate::document::OBJECT_REPLACEMENT_CHARACTER.len_utf8();
        }
        self.inline.append(&text[from..], style)?;
        if let Some(letter) = letter {
            return self.finish_letter(letter, true);
        }
        self.inline.close_marker();
        Ok(())
    }
    /// The atomic box of an inline initial letter styled `style` holding `paragraph`, set at
    /// its used font size (CSS Inline 3 §7.5.3).
    fn inline_letter_box(
        &mut self,
        mut style: Style,
        decoration: BoxDecoration,
        navigation: BoxNavigation,
        mut paragraph: Paragraph,
    ) -> Result<BlockBox, DocumentError> {
        let context = crate::text::TextContext::new(&self.sheet.fonts, self.resources.resolver);
        let surrounding = crate::text::LetterSurrounding::of(
            &context,
            self.style,
            style.initial_letter_align.alignment,
        )
        .map_err(letter_text_error)?;
        let edges = if style.initial_letter_align.border_box {
            block_axis_edges(&style, self.letter_basis(&style))
        } else {
            0.0
        };
        let (scale, aligned) = crate::text::initial_letter_scale(
            &context,
            &surrounding,
            paragraph.style(),
            paragraph.text(),
            edges,
        )
        .map_err(letter_text_error)?;
        paragraph.size_initial_letter(scale, aligned);
        style.display = Display::InlineBlock;
        let page_scope =
            crate::document::page::used_name(&style.page, self.element_page_scope.as_ref());
        Ok(BlockBox {
            navigation,
            initial_letter: Some(InitialLetterBox {
                surrounding,
                atomic: false,
            }),
            ..BlockBox::with_decoration(
                style,
                vec![BlockContent::Inline(paragraph)],
                decoration,
                page_scope,
            )
        })
    }
    /// The punctuation `text` around the first letter, in the nested `::first-letter::prefix`
    /// or `::suffix` boxes of `origins` inside the innermost `::first-letter` styled `parent`.
    fn first_letter_part(
        &mut self,
        origins: &[ElementId],
        pseudo: PseudoElement,
        parent: &Style,
        text: &str,
    ) -> Result<(), DocumentError> {
        if text.is_empty() {
            return Ok(());
        }
        if !self.sheet.has_pseudo_rules(pseudo) {
            return self.inline.append(text, parent);
        }
        let mut parent = parent.clone();
        for &origin in origins {
            let element = self.element(origin);
            parent = inline_pseudo_style(
                self.sheet
                    .typographic_pseudo_with_resolver(
                        element,
                        &parent,
                        pseudo,
                        self.resources.resolver,
                    )?
                    .used(),
            );
            let decoration = self.resources.backgrounds(&parent)?;
            self.inline.open_box(
                &parent,
                decoration,
                self.traversal.navigation.descendant(),
                false,
            );
        }
        self.inline.append(text, &parent)?;
        for _ in origins {
            self.inline.close_box();
        }
        Ok(())
    }
    fn element(&self, id: ElementId) -> Element<'a> {
        self.traversal
            .document
            .element(id)
            .expect("first-line origins belong to the formatted document")
    }
    /// Finishes inline content `collected` in a block styled `style`, whose first formatted line
    /// it holds for `origins`, if any.
    fn finish_inline(
        &mut self,
        collected: InlineCollector,
        style: &Style,
        origins: Option<&[ElementId]>,
    ) -> Result<Result<Paragraph, Vec<BlockContent>>, DocumentError> {
        let (sheet, document) = (self.sheet, self.traversal.document);
        let resources = &mut *self.resources;
        collected.finish_with_first_line(style, |paragraph| match origins {
            Some(origins) => {
                crate::document::first_line::variant(paragraph, origins, sheet, document, resources)
            }
            None => Ok(None),
        })
    }
    fn flush(&mut self) -> Result<(), DocumentError> {
        if let Some(letter) = self.letter.take() {
            self.split_letter(*letter)?;
        }
        self.ruby.finish(self.inline)?;
        let collected = self.inline.split();
        let origins = self.first_line.origins.clone();
        let origins = origins.as_deref();
        let finished = self.finish_inline(collected, self.style, origins)?;
        if finished.is_ok() {
            *self.first_line = FirstLine::default();
        }
        push_inline(self.children, finished);
        Ok(())
    }
    /// The first line of a block-level child box `display` passes to it, which then holds
    /// this block's first formatted line if it comes first in flow; that line is consumed.
    fn child_first_line(&mut self, element: ElementId, display: &Display) -> FirstLine {
        let inherited = std::mem::take(self.first_line);
        let own = || std::rc::Rc::from([element]);
        if !display.is_block_level_block_container() {
            return if display.is_block_container() {
                FirstLine::of(own())
            } else {
                FirstLine::default()
            };
        }
        match inherited.origins {
            Some(origins) => FirstLine {
                origins: Some(origins.iter().copied().chain([element]).collect()),
                letter: inherited.letter,
            },
            None => FirstLine::of(own()),
        }
    }
    /// Starts, or continues, a run-in sequence with the run-in box laid out next.
    fn begin_run_in(&mut self) -> Result<(), DocumentError> {
        self.close_run_in();
        let sequence = self.run_in.get_or_insert_with(|| RunInSequence {
            preceding: std::mem::take(self.inline),
            reparented: false,
            anchors: 0,
            open: false,
            whitespace: Vec::new(),
        });
        sequence.open = true;
        for (text, style) in std::mem::take(&mut sequence.whitespace) {
            self.inline.append(&text, &style)?;
        }
        Ok(())
    }
    /// Holds collapsible white space after a run-in box until the sequence's end is known.
    fn defer_run_in_whitespace(&mut self, text: &str, style: &Style) -> bool {
        self.close_run_in();
        let Some(sequence) = self.run_in.as_mut() else {
            return false;
        };
        sequence.whitespace.push((text.to_owned(), style.clone()));
        true
    }
    fn close_run_in(&mut self) {
        if let Some(sequence) = self.run_in.as_mut().filter(|sequence| sequence.open) {
            sequence.anchors = self.inline.anchor_count();
            sequence.open = false;
        }
    }
    /// Ends an open run-in sequence before `follower`. A block box takes the sequence, which
    /// returns for its start; other content follows it in an anonymous block, or after the
    /// marker of the block it ran into.
    fn resolve_run_in(
        &mut self,
        follower: RunInFollower,
    ) -> Result<Option<InlineCollector>, DocumentError> {
        if follower == RunInFollower::Member {
            return Ok(None);
        }
        self.close_run_in();
        let Some(sequence) = self.run_in.take() else {
            return Ok(None);
        };
        let mut collected = std::mem::replace(self.inline, sequence.preceding);
        if follower == RunInFollower::Block {
            let following = collected.take_anchors_after(sequence.anchors);
            for block in following {
                self.out_of_flow(block);
            }
            return Ok(Some(collected));
        }
        if sequence.reparented {
            self.inline.extend(collected)?;
        } else {
            self.flush()?;
            *self.inline = collected;
        }
        for (text, style) in sequence.whitespace {
            self.inline.append(&text, &style)?;
        }
        Ok(None)
    }
    /// Item containers and positions outside inline boxes anchor between blocks. Boxes inside
    /// ruby annotations anchor in their base level, whose containing block they share (CSS
    /// Ruby 1 §2.1.1).
    /// A running element leaves the flow; its semantic events, and those of its contents,
    /// stay at its position, where it assigns its value.
    fn running(&mut self, element: Element<'_>, block: BlockBox) {
        for event in block.subtree_events().0 {
            self.inline.semantic_event(event.clone());
        }
        self.traversal.strings.attach_running(element.id(), block);
    }
    /// A footnote element leaves the flow for the footnote area; its call takes its place,
    /// with the semantic events of the element and its contents, the element's marking the
    /// footnote (CSS GCPM 3 §2.2).
    fn footnote(&mut self, element: Element<'_>, block: BlockBox) -> Result<(), DocumentError> {
        let index = self.traversal.add_footnote(element.id(), block.style());
        self.footnote_call(element, block.style(), block.subtree_events().0, index)?;
        self.traversal.footnotes.bodies.push(FootnoteBody {
            block,
            members: vec![index],
            inline: false,
        });
        Ok(())
    }
    /// The call of the footnote `index`, styled `style`, takes the footnote element's place
    /// with the semantic events of the element and its contents, the element's marking the
    /// footnote.
    fn footnote_call(
        &mut self,
        element: Element<'_>,
        style: &Style,
        events: Vec<&crate::document::SemanticEvent>,
        index: usize,
    ) -> Result<(), DocumentError> {
        for event in events {
            let mut event = event.clone();
            if event.context.element == element.id() && event.context.pseudo.is_none() {
                event.footnote = Some(index);
            }
            self.inline.semantic_event(event);
        }
        self.generated(element, style, GeneratedPseudoElement::FootnoteCall)
    }
    fn out_of_flow(&mut self, block: BlockBox) {
        if let Some(base) = self.ruby.base_level() {
            base.anchor(block);
        } else if self.style.display.is_item_container() || self.inline.is_between_blocks() {
            self.children.push(BlockContent::Positioned(block));
        } else {
            self.inline.anchor(block);
        }
    }
    fn generated(
        &mut self,
        element: Element<'_>,
        parent: &Style,
        pseudo: GeneratedPseudoElement,
    ) -> Result<(), DocumentError> {
        let Some((style, content)) = self.sheet.pseudo_with_resolver(
            element,
            parent,
            pseudo.into(),
            self.resources.resolver,
        )?
        else {
            return Ok(());
        };
        self.place_generated(element, parent, pseudo, style, &content)
    }
    /// Builds the boxes of a pseudo-element with computed `style`. It is apart from
    /// [`Self::generated`] so that its large frame is not on the stack while the style of a
    /// pseudo-element in a nested navigable's document computes.
    fn place_generated(
        &mut self,
        element: Element<'_>,
        parent: &Style,
        pseudo: GeneratedPseudoElement,
        mut style: Style,
        content: &crate::style::GeneratedContent,
    ) -> Result<(), DocumentError> {
        style = style.used();
        // `running()` moves the pseudo-element to page-margin boxes (CSS GCPM 3 §1.2.1).
        let running =
            style.display != Display::Contents && style.position == crate::style::Position::Running;
        let element_page_scope =
            crate::document::page::used_name(&parent.page, self.element_page_scope.as_ref());
        validate_box(&style)?;
        let replacement = content.replacement();
        normalize_child_display(
            &mut style.display,
            replacement.is_some(),
            &self.style.display,
        );
        // CSS Display 3 §2.5: `display: contents` on a replaced element computes to `none`.
        if replacement.is_some() && style.display == Display::Contents {
            return Ok(());
        }
        // The block's own pseudo-elements are its first and last children; those of inline
        // descendants lie inside the content around them.
        let block_child = std::ptr::eq(parent, self.style);
        if let Some(inner) = style.display.run_in().cloned() {
            if block_child {
                self.begin_run_in()?;
            }
            style.display = inner;
        } else if block_child {
            self.resolve_run_in(
                if style.position.leaves_flow() || style.floating().is_some() {
                    RunInFollower::Member
                } else {
                    RunInFollower::Other
                },
            )?;
        }
        let decoration = if style.display == Display::Contents {
            BoxDecoration::default()
        } else {
            self.resources.backgrounds(&style)?
        };
        self.traversal.counters.prepare(
            &style.counters,
            self.traversal.document,
            self.sheet,
            &self.traversal.root_style,
            self.resources.resolver,
        )?;
        self.traversal.counters.enter(
            crate::style::PageStyleTarget {
                element: element.id(),
                pseudo: Some(pseudo.into()),
            },
            &style.counters,
            &style.display,
            style.containment().style,
        )?;
        let context = crate::style::PageStyleTarget {
            element: element.id(),
            pseudo: Some(pseudo.into()),
        };
        // A running pseudo-element's context lies where the event placing its assignment does.
        let event = (!running
            && (content.named_string_requests().next().is_some() || content.has_page_counters()))
        .then(|| self.traversal.strings.generated_event(element, pseudo));
        let bound = self.traversal.bind(content, context, running)?;
        let quotes = self.traversal.quotes;
        let output = bound.element_output_with_counters(
            crate::document::MAX_INLINE_TEXT_BYTES,
            &mut |request| self.traversal.counters.text(request),
            &style.quotes,
            &mut self.traversal.quotes,
        )?;
        let copies = if running {
            self.generated_copies(content, context, &style.quotes, quotes)?
        } else {
            Vec::new()
        };
        let text = output.text();
        self.traversal.counters.leave();
        self.traversal
            .generated
            .generated(element, pseudo, text, &style);
        self.traversal
            .navigation
            .generated(element, pseudo, text, &style);
        self.traversal
            .strings
            .generated(element, pseudo, text, &style);
        let replacement = replacement.map(|image| self.resources.replacement_image(image, &style));
        if running {
            let name = style
                .running
                .clone()
                .expect("running() names the running element");
            let copies = copies
                .iter()
                .map(|(numbers, output)| {
                    Ok((
                        *numbers,
                        self.running_text_block(
                            output,
                            style.clone(),
                            decoration.clone(),
                            element_page_scope.as_ref(),
                        )?,
                    ))
                })
                .collect::<Result<_, DocumentError>>()?;
            let block = if let Some(image) = replacement {
                let mut block = self.generated_block(
                    style,
                    Vec::new(),
                    decoration,
                    element_page_scope.as_ref(),
                    None,
                );
                block.image = Some(image);
                block
            } else {
                self.running_text_block(&output, style, decoration, element_page_scope.as_ref())?
            };
            let event = self
                .traversal
                .strings
                .record_generated_running(element, pseudo, name, block, copies);
            self.inline.semantic_event(event);
            return Ok(());
        }
        if let Some(image) = replacement {
            let replaced = self.generated_block(
                style,
                Vec::new(),
                decoration,
                element_page_scope.as_ref(),
                event,
            );
            return self.replaced(replaced, image, parent);
        }
        let images = self.resources.generated_images(&output, &style);
        if style.display != Display::Contents
            && (style.position.is_out_of_flow() || style.floating().is_some())
        {
            let children = Paragraph::generated_output(&output, &images, &style)?
                .into_iter()
                .map(BlockContent::Inline)
                .collect();
            let shape_outside = self.resources.shape_outside(&style);
            let mut generated = self.generated_block(
                style,
                children,
                decoration,
                element_page_scope.as_ref(),
                event,
            );
            generated.shape_outside = shape_outside;
            self.out_of_flow(generated);
            return Ok(());
        }
        match &style.display {
            Display::Contents => {
                if let Some(event) = event {
                    self.inline.semantic_event(event);
                }
                self.append_generated(&output, &images, &style)?;
            }
            Display::Inline
            | Display::Ruby
            | Display::RubyBase
            | Display::RubyText
            | Display::RubyBaseContainer
            | Display::RubyTextContainer => {
                if let Some(event) = event {
                    self.inline.semantic_event(event);
                }
                self.open_inline(
                    &style,
                    decoration,
                    self.traversal.navigation.descendant(),
                    false,
                )?;
                self.append_generated(&output, &images, &style)?;
                self.close_inline()?;
            }
            // An inline box's table parts lie in an anonymous inline table (CSS 2.1 §17.2.1).
            display
                if matches!(
                    display,
                    Display::InlineMath
                        | Display::InlineBlock
                        | Display::InlineFlex
                        | Display::InlineGrid
                        | Display::InlineTable
                ) || (display.is_table_part() && self.inline.inside_inline_box()) =>
            {
                let children = Paragraph::generated_output(&output, &images, &style)?
                    .into_iter()
                    .map(BlockContent::Inline)
                    .collect();
                let atomic = self.generated_block(
                    style.clone(),
                    children,
                    decoration,
                    element_page_scope.as_ref(),
                    event,
                );
                self.atomic(atomic, parent)?;
            }
            Display::Unsupported(value) => {
                return Err(StyleError::Unsupported(format!("generated display: {value}")).into());
            }
            Display::ListItem
            | Display::FlowRootListItem
            | Display::InlineListItem
            | Display::InlineFlowRootListItem => {
                return Err(DocumentError::Formatting(
                    "generated list-item counters are not supported".into(),
                ));
            }
            Display::None => unreachable!("hidden pseudo-elements are excluded by the cascade"),
            _ => {
                self.flush()?;
                // A block-level ruby container's text is its sole ruby base, unannotated.
                if style.display == Display::BlockRuby {
                    style.display = Display::Block;
                }
                // The pseudo-element's box comes first in flow, taking the first formatted line.
                let first_line = std::mem::take(self.first_line);
                let children = if style.display.is_block_level_block_container() {
                    self.first_line_block(first_line, &output, &images, &style)?
                } else {
                    Paragraph::generated_output(&output, &images, &style)?
                        .into_iter()
                        .map(BlockContent::Inline)
                        .collect()
                };
                let mut generated = self.generated_block(
                    style,
                    children,
                    decoration,
                    element_page_scope.as_ref(),
                    event,
                );
                generated.enclosing_inline = self.inline.open_boxes();
                self.children.push(BlockContent::Block(generated));
            }
        }
        Ok(())
    }

    /// The contents of a block-level generated box holding `output`, which holds the first
    /// formatted line `first_line` passes to it.
    fn first_line_block(
        &mut self,
        first_line: FirstLine,
        output: &crate::style::GeneratedOutput,
        images: &[crate::document::inline::GeneratedImage],
        style: &Style,
    ) -> Result<Vec<BlockContent>, DocumentError> {
        let outer = std::mem::take(self.inline);
        let origins = first_line.origins.clone();
        *self.first_line = first_line;
        let appended = self.append_generated(output, images, style);
        let contents = std::mem::replace(self.inline, outer);
        *self.first_line = FirstLine::default();
        appended?;
        Ok(
            match self.finish_inline(contents, style, origins.as_deref())? {
                Ok(paragraph) => vec![BlockContent::Inline(paragraph)],
                Err(stray) => stray,
            },
        )
    }

    /// Places `block` as a replaced box showing `image`: out of flow, as an atomic inline, or
    /// as a block-level box.
    fn replaced(
        &mut self,
        mut block: BlockBox,
        image: std::sync::Arc<crate::images::Image>,
        parent: &Style,
    ) -> Result<(), DocumentError> {
        block.image = Some(image);
        if block.style.position.is_out_of_flow() || block.style.floating().is_some() {
            self.out_of_flow(block);
        } else if block.style.display.is_inline_level() {
            self.atomic(block, parent)?;
        } else {
            self.flush()?;
            block.enclosing_inline = self.inline.open_boxes();
            self.children.push(BlockContent::Block(block));
        }
        Ok(())
    }

    /// The contents an element's `content` list replaces its own with (CSS Content 3 §1).
    fn element_content(
        &mut self,
        element: Element<'_>,
        style: &Style,
        content: &crate::style::GeneratedContent,
    ) -> Result<(), DocumentError> {
        let bound = self.traversal.bind(
            content,
            crate::style::PageStyleTarget {
                element: element.id(),
                pseudo: None,
            },
            false,
        )?;
        let output = bound.element_output_with_counters(
            crate::document::MAX_INLINE_TEXT_BYTES,
            &mut |request| self.traversal.counters.text(request),
            &style.quotes,
            &mut self.traversal.quotes,
        )?;
        let images = self.resources.generated_images(&output, style);
        self.append_generated(&output, &images, style)
    }

    /// A pseudo-element's block, part of the current element's hyperlink.
    /// The generated content of the running pseudo-element `context` as each page that
    /// places it shows its page counters (CSS GCPM 3 §1.2.2), from quote depth `depth`.
    fn generated_copies(
        &mut self,
        content: &crate::style::GeneratedContent,
        context: crate::style::PageStyleTarget,
        quotes: &crate::style::Quotes,
        depth: crate::style::QuoteDepth,
    ) -> Result<
        Vec<(
            crate::document::generated::PageNumbers,
            crate::style::GeneratedOutput,
        )>,
        DocumentError,
    > {
        if self.traversal.generated.formats_copy() {
            return Ok(Vec::new());
        }
        let pages = self.traversal.generated.running_pages(context).to_vec();
        pages
            .into_iter()
            .map(|numbers| {
                let bound = self
                    .traversal
                    .generated
                    .bind_copy(content, context, numbers)?;
                let output = bound.element_output_with_counters(
                    crate::document::MAX_INLINE_TEXT_BYTES,
                    &mut |request| self.traversal.counters.text(request),
                    quotes,
                    &mut depth.clone(),
                )?;
                Ok((numbers, output))
            })
            .collect()
    }
    /// The box of a running pseudo-element holding generated `output`.
    fn running_text_block(
        &mut self,
        output: &crate::style::GeneratedOutput,
        style: Style,
        decoration: BoxDecoration,
        element_page_scope: Option<&std::sync::Arc<str>>,
    ) -> Result<BlockBox, DocumentError> {
        let images = self.resources.generated_images(output, &style);
        let children = Paragraph::generated_output(output, &images, &style)?
            .into_iter()
            .map(BlockContent::Inline)
            .collect();
        Ok(self.generated_block(style, children, decoration, element_page_scope, None))
    }
    fn generated_block(
        &self,
        style: Style,
        children: Vec<BlockContent>,
        decoration: BoxDecoration,
        element_page_scope: Option<&std::sync::Arc<str>>,
        event: Option<crate::document::SemanticEvent>,
    ) -> BlockBox {
        let page_scope = crate::document::page::used_name(&style.page, element_page_scope);
        BlockBox {
            navigation: self.traversal.navigation.descendant(),
            canvas_source: false,
            event,
            ..BlockBox::with_decoration(style, children, decoration, page_scope)
        }
    }
}

/// Rejects computed values that boxes cannot render.
fn validate_box(style: &Style) -> Result<(), StyleError> {
    style.display.validate()
}

/// Elements whose content is an image or a nested navigable rather than child boxes (HTML
/// §15.4). A `canvas` represents its fallback content with scripting disabled (HTML §4.12.5).
pub(super) fn is_replaced(element: Element<'_>) -> bool {
    match element.name() {
        "svg" => true,
        "img" | "iframe" | "embed" | "video" | "audio" => {
            element.namespace() == crate::dom::HTML_NAMESPACE
        }
        "object" => element.namespace() == crate::dom::HTML_NAMESPACE && element.object_embeds(),
        _ => false,
    }
}

/// The marker box of list item `element` (CSS Lists 3 §3), filled and styled by its
/// `::marker`.
fn list_marker(
    sheet: &Stylesheet,
    element: Element<'_>,
    style: &Style,
    ordinal: i128,
    traversal: &mut Traversal<'_>,
    resources: &mut ImageResources<'_>,
) -> Result<Option<Paragraph>, DocumentError> {
    if !style.display.is_list_item() {
        return Ok(None);
    }
    let Some((marker, content)) = sheet.marker_with_resolver(element, style, resources.resolver)?
    else {
        return Ok(None);
    };
    let marker = marker.used();
    let used = crate::document::lists::used_style(&marker);
    let Some(content) = content else {
        return crate::document::lists::marker(style, &used, ordinal, resources);
    };
    let bound = traversal.bind(
        &content,
        crate::style::PageStyleTarget {
            element: element.id(),
            pseudo: Some(PseudoElement::Marker),
        },
        false,
    )?;
    let output = bound.element_output_with_counters(
        crate::document::MAX_INLINE_TEXT_BYTES,
        &mut |request| traversal.counters.text(request),
        &marker.quotes,
        &mut traversal.quotes,
    )?;
    let images = resources.generated_images(&output, &used);
    Paragraph::generated_marker(&output, &images, &used)
}

pub(super) fn normalize_child_display(display: &mut Display, replaced: bool, parent: &Display) {
    if replaced && let Display::RunIn(inner) = display {
        normalize_child_display(inner, true, &Display::Block);
    } else if replaced && *display == Display::BlockRuby {
        // CSS Ruby 1 §2.1: replaced elements with ruby displays take their outer display.
        *display = Display::Block;
    } else if replaced
        && !matches!(display, Display::InlineMath | Display::BlockMath)
        && (display.is_table_internal()
            || display.is_ruby()
            || display.is_atomic_inline()
            || matches!(
                display,
                Display::Table
                    | Display::TableCaption
                    | Display::Flex
                    | Display::Grid
                    | Display::ListItem
                    | Display::FlowRootListItem
                    | Display::InlineListItem
            ))
    {
        *display = Display::Inline;
    }
    if parent.is_item_container() && display.is_inline_level() {
        *display = display.blockified();
    }
}

pub(super) fn normalize_root_display(display: &mut Display) {
    *display = if *display == Display::Contents {
        Display::Block
    } else {
        display.blockified()
    };
}

fn computed_style(
    sheet: &Stylesheet,
    element: Element<'_>,
    parent: &Style,
    resolver: &dyn crate::resources::ResourceResolver,
) -> Result<Box<Style>, DocumentError> {
    sheet
        .computed_with_resolver(element, parent, resolver)
        .map(|style| style.used())
        .map(Box::new)
        .map_err(Into::into)
}

#[derive(Clone, Copy)]
enum Placement {
    Root,
    Flow,
    Positioned,
    /// Moved to page-margin boxes by `position: running()` (CSS GCPM 3 §1.2.1).
    Running,
    /// Moved to the footnote area by `float: footnote` (CSS GCPM 3 §2.2).
    Footnote,
    /// The anonymous block whose inline formatting context in the footnote area holds an
    /// inline footnote (CSS GCPM 3 §2.3).
    InlineFootnote,
    Atomic,
}

struct OpenBlock {
    decoration: BoxDecoration,
    shape_outside: crate::graphics::ShapeOutside,
    children: Vec<BlockContent>,
    inline: InlineCollector,
    marker: Option<Paragraph>,
    image: Option<std::sync::Arc<crate::images::Image>>,
    density: f32,
    navigation: BoxNavigation,
    placement: Placement,
    event: Option<crate::document::SemanticEvent>,
    run_in: Option<RunInSequence>,
    /// A run-in sequence that runs into this block, after its marker.
    pending_run_in: Option<InlineCollector>,
    first_line: FirstLine,
    /// Boxed: open blocks are moved through the formatting stack and rarely hold a letter.
    letter: Option<Box<OpenLetter>>,
    ruby: crate::document::ruby::RubyStack,
    /// The principal box of a block-level ruby container, whose inline-level ruby container
    /// holds its contents (CSS Ruby 1 §2.1.2).
    ruby_container: bool,
    /// An initial letter in its contents is sized by its inline size.
    sizes_letter: bool,
    /// The inline footnote an `InlineFootnote` block holds.
    inline_footnote: Option<Box<InlineFootnoteMember>>,
    /// An atomic inline's box on its block's first formatted line, if that line restyles it.
    first_line_box: Option<Box<BlockBox>>,
}

/// An inline footnote being collected into the anonymous block of its footnote area.
struct InlineFootnoteMember {
    /// The footnote element's style, which its call inherits.
    style: Style,
    /// The footnotes the block holds before it.
    members: Vec<usize>,
}

struct Frame<'d> {
    element: Element<'d>,
    style: Box<Style>,
    page_scope: Option<std::sync::Arc<str>>,
    depth: usize,
    math_depth: Option<usize>,
    nodes: Option<ChildNodes<'d>>,
    block: Option<OpenBlock>,
    started: bool,
    /// The quote depth before a style-contained element's subtree, restored after it.
    quotes: Option<crate::style::QuoteDepth>,
    generated: bool,
    ordinal: i128,
}
impl<'d> Frame<'d> {
    fn new(
        element: Element<'d>,
        style: Box<Style>,
        depth: usize,
        ordinal: i128,
        placement: Option<Placement>,
        ancestor_page: Option<&std::sync::Arc<str>>,
    ) -> Self {
        let page_scope = crate::document::page::used_name(&style.page, ancestor_page);
        Self {
            element,
            style,
            page_scope,
            depth,
            math_depth: None,
            nodes: None,
            block: placement.map(|placement| OpenBlock {
                decoration: BoxDecoration::default(),
                shape_outside: crate::graphics::ShapeOutside::None,
                children: Vec::new(),
                inline: InlineCollector::default(),
                marker: None,
                image: None,
                density: DEFAULT_DENSITY,
                navigation: BoxNavigation::default(),
                placement,
                event: None,
                run_in: None,
                pending_run_in: None,
                first_line: FirstLine::default(),
                letter: None,
                ruby: crate::document::ruby::RubyStack::default(),
                ruby_container: false,
                sizes_letter: false,
                inline_footnote: None,
                first_line_box: None,
            }),
            started: false,
            quotes: None,
            generated: false,
            ordinal,
        }
    }
}

/// The semantic events of a subtree that generates no boxes, whose named strings and counter
/// snapshots still record.
fn hidden<'a>(
    child: Element<'_>,
    sheet: &'a Stylesheet,
    traversal: &mut Traversal<'a>,
    resources: &ImageResources<'a>,
) -> Result<Vec<crate::document::SemanticEvent>, DocumentError> {
    let mut events = Vec::new();
    sheet.with_document_values(
        traversal.document,
        &traversal.root_style,
        resources.resolver,
        |styles| {
            traversal
                .counters
                .visit_hidden(child, styles, |element, assignments, counters| {
                    traversal.generated.capture(element, counters);
                    let assignment =
                        traversal
                            .strings
                            .record(element, assignments, counters, None)?;
                    if assignment.is_some() {
                        events.push(traversal.strings.event(element, assignment, false));
                    }
                    Ok(())
                })
        },
    )?;
    Ok(events)
}

fn active_builder<'a, 's, 'c>(
    frames: &'s mut [Frame<'_>],
    index: usize,
    sheet: &'a Stylesheet,
    traversal: &'c mut Traversal<'a>,
    resources: &'c mut ImageResources<'a>,
) -> (BlockBuilder<'a, 's, 'c>, &'s Style) {
    let (frame, descendants) = frames[index..].split_first_mut().unwrap();
    let style = descendants
        .last()
        .map_or(&*frame.style, |frame| &frame.style);
    let element_page_scope = descendants
        .last()
        .map_or(&frame.page_scope, |frame| &frame.page_scope);
    let block = frame.block.as_mut().unwrap();
    (
        BlockBuilder {
            sheet,
            element: frame.element.id(),
            style: &frame.style,
            sized: &mut block.sizes_letter,
            element_page_scope,
            children: &mut block.children,
            inline: &mut block.inline,
            run_in: &mut block.run_in,
            first_line: &mut block.first_line,
            letter: &mut block.letter,
            ruby: &mut block.ruby,
            traversal,
            resources,
        },
        style,
    )
}

/// Opens on `frames` the anonymous block of the footnote area holding the inline footnote
/// `element`, styled `style` and a child of the frame `parent`: the block the inline
/// footnotes before it are collected in, which it joins, or a new one. Kept out of `block`,
/// whose frame nested navigables' formatting stacks.
#[inline(never)]
fn open_inline_footnote<'d>(
    frames: &mut Vec<Frame<'d>>,
    (element, style): (Element<'d>, &Style),
    parent: usize,
    sheet: &Stylesheet,
    traversal: &mut Traversal<'_>,
    resources: &mut ImageResources<'_>,
) -> Result<(), DocumentError> {
    let (block_style, children, inline, members) = match traversal.inline_footnotes.take() {
        Some(open) => {
            let InlineFootnotes {
                style,
                children,
                inline,
                members,
            } = *open;
            (style, children, inline, members)
        }
        None => (
            Box::new(traversal.inline_footnotes_style(element.id(), sheet, resources)?),
            Vec::new(),
            InlineCollector::default(),
            Vec::new(),
        ),
    };
    let mut frame = Frame::new(
        element,
        block_style,
        frames[parent].depth + 1,
        0,
        Some(Placement::InlineFootnote),
        None,
    );
    frame.started = true;
    let block = frame
        .block
        .as_mut()
        .expect("an inline footnote frame builds a block");
    block.children = children;
    block.inline = inline;
    block.inline_footnote = Some(Box::new(InlineFootnoteMember {
        style: style.clone(),
        members,
    }));
    frames.push(frame);
    Ok(())
}

/// Ends the inline footnote of the anonymous block that tops `frames`: its call takes its
/// place in the block `parent` of `frames`, and the block waits for the next inline footnote
/// to join it. Kept out of `block`, whose frame nested navigables' formatting stacks.
#[inline(never)]
fn end_inline_footnote<'a>(
    frames: &mut Vec<Frame<'_>>,
    parent: usize,
    sheet: &'a Stylesheet,
    traversal: &mut Traversal<'a>,
    resources: &mut ImageResources<'a>,
) -> Result<(), DocumentError> {
    let wrapper = frames.pop().expect("the inline footnote's block is open");
    let element = wrapper.element;
    let mut block = wrapper
        .block
        .expect("an inline footnote frame builds a block");
    let member = block
        .inline_footnote
        .take()
        .expect("an inline footnote block holds its footnote");
    let index = traversal.add_footnote(element.id(), &member.style);
    let order = traversal.strings.event(element, None, false).order;
    let mut events = Vec::new();
    content_events(&block.children, &mut events);
    block.inline.content_events(&mut events);
    events.retain(|event| order.contains(&event.order.start));
    events.sort_by_key(|event| event.order.start);
    let events: Vec<_> = events.into_iter().cloned().collect();
    let (mut builder, _) = active_builder(frames, parent, sheet, traversal, resources);
    builder.footnote_call(element, &member.style, events.iter().collect(), index)?;
    // Inline footnotes inside this one end apart from it.
    traversal.finish_inline_footnotes()?;
    let mut members = member.members;
    members.push(index);
    traversal.inline_footnotes = Some(Box::new(InlineFootnotes {
        style: wrapper.style,
        children: block.children,
        inline: block.inline,
        members,
    }));
    Ok(())
}

/// Formats the element of the `root` frame and its subtree.
fn block<'a>(
    root: Frame<'_>,
    sheet: &'a Stylesheet,
    traversal: &mut Traversal<'a>,
    resources: &mut ImageResources<'a>,
) -> Result<BlockBox, DocumentError> {
    let element = root.element;
    let mut frames = vec![root];
    if frames[0].style.display.is_block_container() {
        frames[0]
            .block
            .as_mut()
            .expect("the root frame is a block")
            .first_line = FirstLine::of(std::rc::Rc::from([element.id()]));
    }
    let mut blocks = vec![0];
    loop {
        let index = frames.len() - 1;
        let mut block_index = *blocks.last().unwrap();
        if !frames[index].started {
            // The atomic inlines of a block's first formatted line inherit through
            // `::first-line`, but only layout finds the line each one lies on.
            let first_line = match (blocks.len().checked_sub(2), &frames[index].block) {
                (Some(parent), Some(block)) if matches!(block.placement, Placement::Atomic) => {
                    let parent = &frames[blocks[parent]];
                    let origins = parent
                        .block
                        .as_ref()
                        .and_then(|block| block.first_line.origins.clone());
                    match origins {
                        Some(origins) => first_line_box(
                            &frames[index],
                            (&parent.style, &origins),
                            sheet,
                            traversal,
                            resources,
                        )?,
                        None => None,
                    }
                }
                _ => None,
            };
            let frame = &mut frames[index];
            if let Some(block) = &mut frame.block {
                block.first_line_box = first_line;
            }
            let element = frame.element;
            // CSS Containment 2 §3.3: quote depth changes are scoped to the subtree.
            frame.quotes = frame.style.containment().style.then_some(traversal.quotes);
            traversal.generated.capture(element, &traversal.counters);
            let copies = if frame
                .block
                .as_ref()
                .is_some_and(|block| matches!(block.placement, Placement::Running))
            {
                running_copies(frame, sheet, traversal, resources)?
            } else {
                Vec::new()
            };
            if let Some(block) = &mut frame.block {
                // CSS Containment 2 §4: an element skipping its contents generates no boxes,
                // pseudo-elements or replaced fallback for them.
                let skips = frame.style.skips_contents() && frame.style.containment().layout;
                block.decoration = resources.backgrounds(&frame.style)?;
                block.shape_outside = resources.shape_outside(&frame.style);
                if index == 0 {
                    traversal.canvas = background_only(&block.decoration);
                }
                // CSS Content 3 §1: an element's `<content-replacement>` replaces whatever it
                // would otherwise render.
                let replacement = if skips {
                    None
                } else {
                    frame
                        .style
                        .content_replacement()
                        .map(|image| resources.replacement_image(image, &frame.style))
                };
                let control = if skips || replacement.is_some() {
                    None
                } else if let Some(frameset) = crate::document::frameset::content(
                    element,
                    &frame.style,
                    sheet,
                    traversal.embedded,
                    frame.page_scope.as_ref(),
                ) {
                    Some(frameset)
                } else {
                    crate::document::controls::content(
                        element,
                        &frame.style,
                        sheet,
                        resources,
                        frame.page_scope.as_ref(),
                    )?
                };
                let rendered_control = control.is_some();
                block.image = if let Some(control) = control {
                    block.children.extend(control.children);
                    control.image
                } else if replacement.is_some() {
                    replacement
                } else {
                    match element.name() {
                        "img" => {
                            let image = sheet.image_source(element).and_then(|source| {
                                block.density = source.density.unwrap_or(DEFAULT_DENSITY);
                                resources
                                    .cache
                                    .load_optional(
                                        &source.url,
                                        resources.base,
                                        resources.resolver,
                                        frame.style.element_font_namespace(),
                                    )
                                    .map(|image| {
                                        crate::images::content_image(
                                            image,
                                            &frame.style,
                                            source.density,
                                        )
                                    })
                            });
                            if image.is_none()
                                && !skips
                                && let Some(alt) = element.attr("alt")
                            {
                                block.inline.append(alt, &frame.style)?;
                            }
                            image
                        }
                        "svg" => {
                            let resolver = resources.resolver;
                            let svg = crate::document::svg::serialize(
                                element,
                                &frame.style,
                                &crate::document::svg::Styling { sheet, resolver },
                                &mut |style| resources.masks(style),
                            )?;
                            Some(resources.cache.inline_svg_in_namespace(
                                svg.xml,
                                svg.styling,
                                resources.base,
                                resolver,
                                frame.style.element_font_namespace(),
                            )?)
                        }
                        "video" if is_replaced(element) => Some(crate::document::media::poster(
                            element,
                            &frame.style,
                            resources,
                        )),
                        "audio" if is_replaced(element) => Some(
                            crate::document::media::audio_controls(&frame.style, resources)?,
                        ),
                        _ if is_replaced(element) => traversal.embedded.content(element.id()).map(
                            |embedded| match embedded {
                                crate::document::embedded::Embedded::Image(image) => {
                                    crate::images::content_image(image.clone(), &frame.style, None)
                                }
                                crate::document::embedded::Embedded::Navigable(navigable) => {
                                    std::sync::Arc::new(crate::images::Image::Frame(
                                        navigable.clone(),
                                        sheet.used_color_scheme(&frame.style.color_scheme),
                                    ))
                                }
                            },
                        ),
                        _ => None,
                    }
                };
                block.marker = if skips {
                    None
                } else {
                    list_marker(
                        sheet,
                        element,
                        &frame.style,
                        frame.ordinal,
                        traversal,
                        resources,
                    )?
                };
                let mut assignment = traversal.strings.record(
                    element,
                    &frame.style.string_set,
                    &mut traversal.counters,
                    block.marker.as_ref().map(Paragraph::text),
                )?;
                if let (Placement::Running, Some(name)) = (block.placement, &frame.style.running) {
                    assignment = Some(traversal.strings.record_running(
                        element,
                        assignment,
                        name.clone(),
                        copies,
                    ));
                }
                block.event = Some(traversal.strings.event(element, assignment, true));
                block.navigation = traversal.navigation.enter(element);
                traversal.navigation.record(
                    element,
                    &frame.style,
                    block.marker.as_ref().map(Paragraph::text),
                    traversal.quotes,
                    if frame.style.bookmark.level.is_some() {
                        traversal.counters.snapshot(&frame.style.bookmark.label)?
                    } else {
                        Default::default()
                    },
                    &mut block.navigation,
                );
                if frame.style.display == Display::BlockRuby {
                    frame.style.display = Display::Block;
                    block.ruby_container = true;
                }
                let inside_marker = frame
                    .style
                    .has_inside_marker()
                    .then(|| block.marker.take())
                    .flatten();
                if let Some(marker) = inside_marker {
                    let (mut builder, _) =
                        active_builder(&mut frames, index, sheet, traversal, resources);
                    builder.place_inside_marker(marker)?;
                }
                let frame = &mut frames[index];
                let block = frame.block.as_mut().expect("a block frame builds a block");
                if let Some(run_in) = block.pending_run_in.take() {
                    block.run_in = Some(RunInSequence {
                        preceding: std::mem::replace(&mut block.inline, run_in),
                        reparented: true,
                        anchors: block.inline.anchor_count(),
                        open: false,
                        whitespace: Vec::new(),
                    });
                }
                if block.ruby_container {
                    let container = crate::document::ruby::anonymous(&frame.style, Display::Ruby);
                    block.ruby.open(
                        &mut block.inline,
                        &container,
                        BoxDecoration::default(),
                        BoxNavigation::default(),
                        &frame.style,
                    )?;
                }
                frame.generated = !skips
                    && !rendered_control
                    && block.image.is_none()
                    && frame.style.display != Display::TableColumn;
            } else {
                // CSS Lists 3 §3.5: an inline list item's marker is always inside.
                let marker = list_marker(
                    sheet,
                    element,
                    &frame.style,
                    frame.ordinal,
                    traversal,
                    resources,
                )?;
                let assignment = traversal.strings.record(
                    element,
                    &frame.style.string_set,
                    &mut traversal.counters,
                    marker.as_ref().map(Paragraph::text),
                )?;
                let event = traversal.strings.event(element, assignment, true);
                let mut navigation = traversal.navigation.enter(element);
                traversal.navigation.record(
                    element,
                    &frame.style,
                    marker.as_ref().map(Paragraph::text),
                    traversal.quotes,
                    if frame.style.bookmark.level.is_some() {
                        traversal.counters.snapshot(&frame.style.bookmark.label)?
                    } else {
                        Default::default()
                    },
                    &mut navigation,
                );
                let canvas_source = index == 0 || traversal.canvas_body == Some(element.id());
                let (mut builder, style) =
                    active_builder(&mut frames, block_index, sheet, traversal, resources);
                builder.inline.semantic_event(event);
                if style.display == Display::Contents {
                    builder.inline.open_contents(navigation.link);
                } else {
                    let decoration = builder.resources.backgrounds(style)?;
                    builder.open_inline(style, decoration, navigation, canvas_source)?;
                    if let Some(marker) = marker {
                        builder.place_inside_marker(marker)?;
                    }
                }
                frames[index].generated = true;
            }
            if frames[index].generated || element.name() == "br" {
                if frames[index].depth > MAX_FORMATTING_DEPTH {
                    return Err(DocumentError::Formatting(format!(
                        "{FORMATTING_NESTING_RESOURCE} exceeds {MAX_FORMATTING_DEPTH}"
                    )));
                }
                let footnote = frames[index]
                    .block
                    .as_ref()
                    .is_some_and(|block| matches!(block.placement, Placement::Footnote))
                    || index
                        .checked_sub(1)
                        .and_then(|wrapper| frames[wrapper].block.as_ref())
                        .is_some_and(|block| block.inline_footnote.is_some());
                let (mut builder, style) =
                    active_builder(&mut frames, block_index, sheet, traversal, resources);
                // The marker begins the footnote element's content (CSS GCPM 3 §2.7).
                if footnote {
                    builder.generated(element, style, GeneratedPseudoElement::FootnoteMarker)?;
                }
                if element.generates_checkmark() {
                    builder.generated(element, style, GeneratedPseudoElement::Checkmark)?;
                }
                builder.generated(element, style, GeneratedPseudoElement::Before)?;
                if element.name() == "br" {
                    builder.line_break(style)?;
                    builder.generated(element, style, GeneratedPseudoElement::After)?;
                    frames[index].generated = false;
                } else {
                    if element.namespace() == crate::dom::HTML_NAMESPACE && element.name() == "wbr"
                    {
                        builder.inline.optional_break(style);
                    }
                    if let Some(content) = style.content_list() {
                        // CSS Content 3 §1: the content list replaces the element's contents,
                        // which generate no boxes, as if they were `display: none`.
                        builder.element_content(element, style, content)?;
                        for child in element.children(TreeRelation::Formatting) {
                            for event in hidden(child, sheet, builder.traversal, builder.resources)?
                            {
                                builder.inline.semantic_event(event);
                            }
                        }
                    } else {
                        frames[index].nodes = Some(element.child_nodes(TreeRelation::Formatting));
                    }
                }
            }
            frames[index].started = true;
            continue;
        }
        if let Some(node) = frames[index].nodes.as_mut().and_then(Iterator::next) {
            // Children of the block and of its boxless descendants are the block's children.
            let block_child = frames[block_index + 1..=index]
                .iter()
                .all(|frame| frame.style.display == Display::Contents);
            if let Some(text) = node.value().as_text() {
                let (mut builder, style) =
                    active_builder(&mut frames, block_index, sheet, traversal, resources);
                if block_child {
                    if text.chars().all(|ch| {
                        crate::document::inline::collapses_character(ch, style.white_space.collapse)
                    }) {
                        if builder.defer_run_in_whitespace(text, style) {
                            continue;
                        }
                    } else {
                        builder.resolve_run_in(RunInFollower::Other)?;
                    }
                }
                builder.append_text(text, style)?;
                continue;
            }
            let Some(child) = node.element() else {
                continue;
            };
            let parent = &frames[index].style;
            let mut child_style = computed_style(sheet, child, parent, resources.resolver)?;
            if traversal.canvas_body == Some(child.id()) {
                traversal.canvas = if child_style.display == Display::None {
                    BoxDecoration::default()
                } else {
                    background_only(&resources.backgrounds(&child_style)?)
                };
            }
            validate_box(&child_style)?;
            let replaced = is_replaced(child) || child_style.content_replacement().is_some();
            // CSS Display 3 §2.5: `display: contents` on a replaced element computes to `none`.
            if replaced && child_style.display == Display::Contents {
                child_style.display = Display::None;
            }
            normalize_child_display(
                &mut child_style.display,
                replaced,
                &frames[block_index].style.display,
            );
            crate::document::controls::used_style(child, &mut child_style);
            crate::document::frameset::used_style(child, &mut child_style, sheet);
            if frames[block_index].style.display == Display::TableColumnGroup
                && child_style.display != Display::Contents
                && child_style.display != Display::TableColumn
            {
                continue;
            }
            if child_style.display == Display::None {
                let events = hidden(child, sheet, traversal, resources)?;
                let (builder, _) =
                    active_builder(&mut frames, block_index, sheet, traversal, resources);
                for event in events {
                    builder.inline.semantic_event(event);
                }
                continue;
            }
            let counters = if child_style.footnote() && child_style.display != Display::Contents {
                child_style
                    .counters
                    .with_implicit_increment(crate::style::FOOTNOTE_COUNTER)
            } else {
                child_style.counters.clone()
            };
            traversal.counters.prepare(
                &counters,
                traversal.document,
                sheet,
                &traversal.root_style,
                resources.resolver,
            )?;
            traversal.counters.enter_at_pages(
                crate::style::PageStyleTarget {
                    element: child.id(),
                    pseudo: None,
                },
                &counters,
                &child_style.display,
                child_style.containment().style,
                traversal.generated.page_starts(child.id()),
                &mut traversal.page_counters,
            )?;
            let child_depth = frames[index].depth + 1;
            let math_depth = frames[index]
                .math_depth
                .map(|depth| depth + 1)
                .or_else(|| (child.namespace() == crate::dom::MATHML_NAMESPACE).then_some(0));
            if math_depth.is_some_and(|depth| depth >= crate::document::mathml::MAX_DEPTH) {
                return Err(DocumentError::Formatting(format!(
                    "MathML nesting exceeds {}",
                    crate::document::mathml::MAX_DEPTH
                )));
            }
            let ancestor_page = frames[index].page_scope.clone();
            let math_cell = crate::document::mathml::table::is_cell(
                frames[block_index].element,
                &frames[block_index].style.display,
            );
            if child_style.footnote() && child_style.display != Display::Contents {
                let inline = child_style.footnote_display == crate::style::FootnoteDisplay::Inline;
                // Consecutive inline footnotes share an inline formatting context, unless
                // pagination found that this one cannot continue the line before it.
                if !inline || traversal.generated.starts_inline_footnotes(child.id()) {
                    traversal.finish_inline_footnotes()?;
                }
                if inline {
                    open_inline_footnote(
                        &mut frames,
                        (child, &child_style),
                        index,
                        sheet,
                        traversal,
                        resources,
                    )?;
                    block_index = frames.len() - 1;
                    blocks.push(block_index);
                    // The anonymous block holds the footnote as an inline-level box.
                    child_style.float = crate::style::Float::None;
                }
            }
            let (mut builder, _) =
                active_builder(&mut frames, block_index, sheet, traversal, resources);
            let run_in = child_style.display.run_in().cloned();
            let merged = if !block_child {
                None
            } else if run_in.is_some() {
                builder.begin_run_in()?;
                None
            } else {
                builder.resolve_run_in(
                    if child_style.display == Display::Contents
                        || child_style.position.leaves_flow()
                        || child_style.floating().is_some()
                        || child_style.footnote()
                    {
                        RunInFollower::Member
                    } else if matches!(child_style.display, Display::Block | Display::ListItem)
                        && !replaced
                        && !child_style.establishes_formatting_context()
                    {
                        RunInFollower::Block
                    } else {
                        RunInFollower::Other
                    },
                )?
            };
            // A run-in box is the inline-level box it holds, wherever it runs.
            if let Some(inner) = run_in {
                child_style.display = inner;
            }
            if child_style.display == Display::Inline
                && builder
                    .ruby
                    .annotation_mode()
                    .is_some_and(|mode| mode != child_style.used_writing_mode())
            {
                child_style.display = Display::InlineBlock;
            }
            let placement = if child_style.display == Display::Contents {
                None
            } else if child_style.position == crate::style::Position::Running {
                Some(Placement::Running)
            } else if child_style.footnote() {
                Some(Placement::Footnote)
            } else if child_style.position.is_out_of_flow() || child_style.floating().is_some() {
                Some(Placement::Positioned)
            } else {
                match &child_style.display {
                    Display::None => unreachable!("hidden elements are skipped before traversal"),
                    Display::Contents => unreachable!("boxless elements have no placement"),
                    Display::BlockMath
                    | Display::Block
                    | Display::FlowRoot
                    | Display::Flex
                    | Display::Grid
                    | Display::ListItem
                    | Display::FlowRootListItem
                    | Display::Table
                    | Display::TableRowGroup
                    | Display::TableHeaderGroup
                    | Display::TableFooterGroup
                    | Display::TableRow
                    | Display::TableCell
                    | Display::TableColumn
                    | Display::TableColumnGroup
                    | Display::TableCaption => {
                        // CSS 2.1 §17.2.1: the anonymous table wrapping table parts whose
                        // parent is an inline box is an inline table.
                        if child_style.display.is_table_part() && builder.inline.inside_inline_box()
                        {
                            Some(Placement::Atomic)
                        } else {
                            builder.flush()?;
                            Some(Placement::Flow)
                        }
                    }
                    // A ruby holds only inline-level content (CSS Ruby 1 §2.2), so a math
                    // child it inlinified stays an atomic inline in it.
                    Display::InlineMath | Display::InlineTable
                        if (matches!(
                            builder.style.display,
                            Display::BlockMath | Display::InlineMath
                        ) || math_cell)
                            && !child_style.inlinified() =>
                    {
                        builder.flush()?;
                        Some(Placement::Flow)
                    }
                    Display::InlineMath
                    | Display::InlineBlock
                    | Display::InlineFlex
                    | Display::InlineGrid
                    | Display::InlineTable
                    | Display::InlineFlowRootListItem => Some(Placement::Atomic),
                    Display::RunIn(_) => unreachable!("run-in boxes take their inner display"),
                    Display::Inline if replaced => Some(Placement::Atomic),
                    Display::InlineListItem
                    | Display::Ruby
                    | Display::RubyBase
                    | Display::RubyText
                    | Display::RubyBaseContainer
                    | Display::RubyTextContainer => None,
                    Display::BlockRuby => {
                        builder.flush()?;
                        Some(Placement::Flow)
                    }
                    Display::Inline => {
                        if child.name() == "br" {
                            builder
                                .traversal
                                .generated
                                .capture(child, &builder.traversal.counters);
                            let assignment = builder.traversal.strings.record(
                                child,
                                &child_style.string_set,
                                &mut builder.traversal.counters,
                                None,
                            )?;
                            builder.inline.semantic_event(
                                builder.traversal.strings.event(child, assignment, true),
                            );
                            builder.generated(
                                child,
                                &child_style,
                                GeneratedPseudoElement::Before,
                            )?;
                            builder.line_break(&child_style)?;
                            builder.generated(
                                child,
                                &child_style,
                                GeneratedPseudoElement::After,
                            )?;
                            builder.traversal.counters.leave();
                            continue;
                        }
                        None
                    }
                    Display::Unsupported(value) => {
                        return Err(StyleError::Unsupported(format!(
                            "display {value} on {}",
                            child.name()
                        ))
                        .into());
                    }
                }
            };
            let ordinal = if child_style.display.is_list_item() {
                builder.traversal.counters.ordinal()?
            } else {
                0
            };
            let first_line = match placement {
                Some(Placement::Flow) => builder.child_first_line(child.id(), &child_style.display),
                Some(_) if child_style.display.is_block_container() => {
                    FirstLine::of(std::rc::Rc::from([child.id()]))
                }
                _ => FirstLine::default(),
            };
            if placement.is_some() {
                blocks.push(frames.len());
            }
            if matches!(placement, Some(Placement::Running)) {
                traversal.running.push(crate::style::PageStyleTarget {
                    element: child.id(),
                    pseudo: None,
                });
            }
            frames.push(Frame::new(
                child,
                child_style,
                child_depth,
                ordinal,
                placement,
                ancestor_page.as_ref(),
            ));
            frames
                .last_mut()
                .expect("the child frame was appended")
                .math_depth = math_depth;
            if let Some(block) = frames.last_mut().and_then(|frame| frame.block.as_mut()) {
                block.first_line = first_line;
            }
            if let Some(run_in) = merged {
                frames
                    .last_mut()
                    .and_then(|frame| frame.block.as_mut())
                    .expect("a block box takes the run-in sequence")
                    .pending_run_in = Some(run_in);
            }
            continue;
        }
        let element = frames[index].element;
        if frames[index]
            .block
            .as_ref()
            .is_some_and(|block| block.inline_footnote.is_some())
        {
            blocks.pop();
            let parent = *blocks
                .last()
                .expect("an inline footnote has a containing block");
            end_inline_footnote(&mut frames, parent, sheet, traversal, resources)?;
            continue;
        }
        let generated = frames[index].generated;
        let contents = frames[index].style.display == Display::Contents;
        let quotes = frames[index].quotes;
        let (mut builder, style) =
            active_builder(&mut frames, block_index, sheet, traversal, resources);
        if generated {
            builder.generated(element, style, GeneratedPseudoElement::After)?;
            if element.generates_picker_icon() {
                builder.generated(element, style, GeneratedPseudoElement::PickerIcon)?;
            }
        }
        if let Some(depth) = quotes {
            builder.traversal.quotes = depth;
        }
        if index != block_index {
            if contents {
                builder.inline.close_contents();
            } else {
                builder.inline.prepare_math_inline(element, style);
                builder.close_inline()?;
            }
            builder.traversal.navigation.leave();
            frames.pop();
            traversal.counters.leave();
            continue;
        }
        builder.resolve_run_in(RunInFollower::Other)?;
        if frames[index]
            .block
            .as_ref()
            .is_some_and(|block| block.ruby_container)
        {
            let (builder, _) =
                active_builder(&mut frames, block_index, sheet, traversal, resources);
            builder.ruby.close(builder.inline)?;
        }
        let (mut builder, _) =
            active_builder(&mut frames, block_index, sheet, traversal, resources);
        builder.flush()?;
        let frame = &mut frames[index];
        let block = frame.block.as_mut().unwrap();
        if block.marker.is_some() && block.children.is_empty() {
            block.inline.line_break(&frame.style)?;
            let (mut builder, _) =
                active_builder(&mut frames, block_index, sheet, traversal, resources);
            builder.flush()?;
        }
        traversal.navigation.leave();
        let frame = frames.pop().unwrap();
        blocks.pop();
        let mut state = frame.block.unwrap();
        mark_last_formatted_line(&mut state.children);
        let canvas_source = index == 0 || traversal.canvas_body == Some(element.id());
        let rendered_legend = crate::document::controls::is_rendered_legend(element, &frame.style);
        let (style, children) = crate::document::controls::fieldset(
            element,
            *frame.style,
            state.children,
            frame.page_scope.as_ref(),
        );
        let mut block =
            BlockBox::with_decoration(style, children, state.decoration, frame.page_scope);
        block.rendered_legend = rendered_legend;
        block.marker = state.marker.map(Box::new);
        block.shape_outside = state.shape_outside;
        block.image = state.image;
        block.empty_image_fallback = element.namespace() == crate::dom::HTML_NAMESPACE
            && element.name() == "img"
            && element.attr("alt") == Some("")
            && block.image.is_none();
        block.density = state.density;
        block.first_line_box = state.first_line_box;
        block.navigation = state.navigation;
        block.event = state.event;
        block.canvas_source = canvas_source;
        // Layout measures the size of query containers and of the containing blocks of
        // initial letters that size by it.
        block.query_container =
            (block.style.containment().query_container || state.sizes_letter).then(|| element.id());
        if crate::document::mathml::table::is_cell(element, &block.style().display) {
            let children = std::mem::take(&mut block.children);
            block.children = crate::document::mathml::table::cell(
                block.style(),
                children,
                sheet,
                resources.resolver,
            )?;
        }
        crate::document::mathml::prepare(element, &mut block, sheet, resources.resolver)?;
        if matches!(state.placement, Placement::Root) {
            return Ok(block);
        }
        let (mut builder, parent) = active_builder(
            &mut frames,
            *blocks.last().unwrap(),
            sheet,
            traversal,
            resources,
        );
        match state.placement {
            Placement::Flow => {
                block.enclosing_inline = builder.inline.open_boxes();
                builder.children.push(BlockContent::Block(block));
            }
            Placement::Positioned => builder.out_of_flow(block),
            Placement::Running => {
                builder.traversal.running.pop();
                builder.running(element, block);
            }
            Placement::Footnote => builder.footnote(element, block)?,
            Placement::InlineFootnote => {
                unreachable!("inline footnote blocks finish as their footnotes end")
            }
            Placement::Atomic => builder.atomic(block, parent)?,
            Placement::Root => unreachable!("root returned before attaching to a parent"),
        }
        traversal.counters.leave();
    }
}

/// A document and the resources its elements loaded before formatting.
pub(crate) struct Input<'a> {
    pub html: &'a DocumentTree,
    pub embedded: &'a crate::document::embedded::EmbeddedContent,
    pub images: &'a std::rc::Rc<std::cell::RefCell<crate::images::ImageCache>>,
    /// What the last formatting and layout found of the document's `element()` images, and
    /// the SVG content that formatting serialized.
    pub elements: &'a crate::document::element_images::ElementMeasurements,
    pub definitions: &'a crate::document::svg::SvgDefinitions,
}

/// Formats the document. The root's style, its first page and its boxes are each built by a
/// function of their own, whose locals leave the stack before the next begins: nested
/// navigables format on the stack of their parent's layout.
pub(crate) fn build(
    input: &Input<'_>,
    sheet: &mut Stylesheet,
    base: &crate::resources::ResourceBase,
    resolver: &dyn crate::resources::ResourceResolver,
    navigation: NavigationBuilder,
    values: crate::document::generated::GeneratedValues,
) -> Result<Formatted, DocumentError> {
    sheet.anchor_nodes.borrow_mut().clear();
    let generated =
        crate::document::generated::GeneratedPass::new(navigation.targets.clone(), values);
    let mut style = root_style(input.html, sheet, resolver, &generated)?;
    let sheet = &*sheet;
    let elements = std::rc::Rc::new(crate::document::element_images::ElementImages::new(
        input.html,
        sheet,
        navigation.targets.clone(),
        input.elements.clone(),
        input.definitions,
    ));
    let resources = Resources {
        base,
        resolver,
        elements: &elements,
    };
    let pages = first_pages(input, sheet, &style, &resources, generated.bindings())?;
    *style = style.used();
    validate_box(&style)?;
    if style.display == Display::None {
        hidden_root(
            input, sheet, &style, &resources, navigation, generated, pages,
        )
    } else {
        format_root(
            input, sheet, style, &resources, navigation, generated, pages,
        )
    }
}

/// What the parts of `build` load images with.
struct Resources<'r> {
    base: &'r crate::resources::ResourceBase,
    resolver: &'r dyn crate::resources::ResourceResolver,
    elements: &'r std::rc::Rc<crate::document::element_images::ElementImages>,
}

/// The root element's computed style, with the first page selected in the same graph.
fn root_style(
    html: &DocumentTree,
    sheet: &mut Stylesheet,
    resolver: &dyn crate::resources::ResourceResolver,
    generated: &crate::document::generated::GeneratedPass,
) -> Result<Box<Style>, DocumentError> {
    sheet
        .resolve_document_root_with_pages(
            html,
            resolver,
            &|styles, groups| {
                crate::document::first_page::select_name(html, styles, generated, groups)
            },
            &|styles| crate::document::first_page::select_side(html, styles, generated),
        )
        .map(Box::new)
        .map_err(|error| match error {
            StyleError::GrammarLimit {
                resource: FORMATTING_NESTING_RESOURCE,
                limit,
            } => {
                DocumentError::Formatting(format!("{FORMATTING_NESTING_RESOURCE} exceeds {limit}"))
            }
            error => DocumentError::Style(error),
        })
}

/// The document's page boxes, starting with its first page's.
fn first_pages(
    input: &Input<'_>,
    sheet: &Stylesheet,
    root: &Style,
    resources: &Resources<'_>,
    bindings: crate::document::generated::GeneratedBindings,
) -> Result<crate::document::page::PageProvider, DocumentError> {
    let area = sheet.page_content_box();
    let resolved = sheet.resolve_page(
        root,
        sheet.first_page_request().clone(),
        Some([area.width, area.height]),
        resources.resolver,
    )?;
    let mut cache = input.images.borrow_mut();
    let mut images = ImageResources {
        base: resources.base,
        resolver: resources.resolver,
        cache: &mut cache,
        elements: resources.elements,
    };
    let page = PageBox::load(resolved, &mut images, &bindings)?;
    Ok(crate::document::page::PageProvider::new(
        page,
        input.images.clone(),
        resources.base.clone(),
        bindings,
        resources.elements.clone(),
    ))
}

/// A document whose root generates no boxes: only its named strings and counters apply.
fn hidden_root(
    input: &Input<'_>,
    sheet: &Stylesheet,
    style: &Style,
    resources: &Resources<'_>,
    navigation: NavigationBuilder,
    mut generated: crate::document::generated::GeneratedPass,
    pages: crate::document::page::PageProvider,
) -> Result<Formatted, DocumentError> {
    let html = input.html;
    let resolver = resources.resolver;
    let element = html.root_element();
    let mut strings = crate::document::strings::StringAssignments::new(html);
    let mut counters = crate::document::counters::Counters::default();
    let mut events = Vec::new();
    sheet.with_document_values(html, style, resolver, |styles| {
        counters.visit_hidden(element, styles, |element, assignments, counters| {
            generated.capture(element, counters);
            let assignment = strings.record(element, assignments, counters, None)?;
            if assignment.is_some() {
                events.push(strings.event(element, assignment, false));
            }
            Ok(())
        })
    })?;
    let canvas = canvas_surface(BoxDecoration::default(), surface_beneath(sheet, style));
    let mut cache = input.images.borrow_mut();
    let linked = cache.linked_documents();
    let mut images = ImageResources {
        base: resources.base,
        resolver,
        cache: &mut cache,
        elements: resources.elements,
    };
    let svg_definitions = crate::document::svg::SvgDefinitions::collect(
        html,
        style,
        &crate::document::svg::Styling { sheet, resolver },
        linked,
        &mut |style| images.masks(style),
    )?;
    Ok(Formatted {
        root: BlockBox::anonymous(style.clone(), vec![BlockContent::Events(events)], None),
        footnotes: Footnotes::default(),
        navigation,
        strings,
        generated: generated.finish(),
        canvas,
        pages,
        svg_definitions,
        page_counters: Vec::new(),
    })
}

/// The boxes of a document whose root generates them.
fn format_root(
    input: &Input<'_>,
    sheet: &Stylesheet,
    mut style: Box<Style>,
    resources: &Resources<'_>,
    navigation: NavigationBuilder,
    generated: crate::document::generated::GeneratedPass,
    pages: crate::document::page::PageProvider,
) -> Result<Formatted, DocumentError> {
    let Input {
        html,
        embedded,
        images,
        ..
    } = *input;
    let resolver = resources.resolver;
    let element = html.root_element();
    let mut cache = images.borrow_mut();
    let mut resources = ImageResources {
        base: resources.base,
        resolver,
        cache: &mut cache,
        elements: resources.elements,
    };
    normalize_root_display(&mut style.display);
    let mut traversal = Traversal {
        document: html,
        embedded,
        root_style: (*style).clone(),
        counters: Default::default(),
        navigation,
        strings: crate::document::strings::StringAssignments::new(html),
        generated,
        quotes: Default::default(),
        canvas: BoxDecoration::default(),
        footnotes: Footnotes::default(),
        inline_footnotes: None,
        page_counters: Vec::new(),
        running: Vec::new(),
        pages: &pages,
        canvas_body: (style.decoration.background.alpha == 0.0
            && !style.decoration.backgrounds.has_images())
        .then(|| html.body_element().map(Element::id))
        .flatten(),
    };
    // The footnote counter is predefined: the root instantiates it (CSS GCPM 3 §2.5).
    let counters = style
        .counters
        .with_implicit_reset(crate::style::FOOTNOTE_COUNTER);
    traversal
        .counters
        .prepare(&counters, html, sheet, &style, resolver)?;
    traversal.counters.enter(
        crate::style::PageStyleTarget {
            element: element.id(),
            pseudo: None,
        },
        &counters,
        &style.display,
        style.containment().style,
    )?;
    // Pages start inside the root, after it instantiates the predefined footnote counter.
    traversal.counters.start_pages(
        traversal.generated.page_starts(element.id()).iter(),
        &mut traversal.page_counters,
    )?;
    let ordinal = if style.display.is_list_item() {
        traversal.counters.ordinal()?
    } else {
        0
    };
    let surface = surface_beneath(sheet, &style);
    let linked = resources.cache.linked_documents();
    let svg_definitions = crate::document::svg::SvgDefinitions::collect(
        html,
        &style,
        &crate::document::svg::Styling { sheet, resolver },
        linked,
        &mut |style| resources.masks(style),
    )?;
    let mut root = block(
        Frame::new(element, style, 0, ordinal, Some(Placement::Root), None),
        sheet,
        &mut traversal,
        &mut resources,
    )?;
    traversal.finish_inline_footnotes()?;
    root.trim_text_boxes([false; 2]);
    for block in traversal.strings.running_blocks_mut().chain(
        traversal
            .footnotes
            .bodies
            .iter_mut()
            .map(|body| &mut body.block),
    ) {
        block.trim_text_boxes([false; 2]);
    }
    // Writing Modes 4 §8: HTML's first body supplies the root's used flow after style computation.
    if let Some(body) = html.body_element() {
        let principal = sheet.computed_with_resolver(body, root.style(), resolver)?;
        root.style.principal_flow = Some(crate::style::logical::Flow::of(&principal));
    }
    root.reflow(crate::style::logical::Flow::used(root.style()));
    // Running elements and footnotes lay out in page areas, which take the page's flow.
    for block in traversal.strings.running_blocks_mut().chain(
        traversal
            .footnotes
            .bodies
            .iter_mut()
            .map(|body| &mut body.block),
    ) {
        block.reflow(crate::style::logical::Flow::used(root.style()));
    }
    let initials = traversal.counters.page_initials();
    traversal.generated.page_initials(initials);
    let Traversal {
        footnotes,
        navigation,
        strings,
        generated,
        canvas,
        page_counters,
        ..
    } = traversal;
    Ok(Formatted {
        root,
        footnotes,
        navigation,
        strings,
        generated: generated.finish(),
        canvas: canvas_surface(canvas, surface),
        pages,
        svg_definitions,
        page_counters,
    })
}

#[cfg(test)]
mod normalization_tests {
    use super::*;

    #[test]
    fn grid_normalization_preserves_inner_layout_and_replaced_content() {
        for (mut display, expected) in [
            (Display::Grid, Display::Grid),
            (Display::InlineGrid, Display::Grid),
            (Display::InlineFlex, Display::Flex),
            (Display::InlineTable, Display::Table),
            (Display::Contents, Display::Block),
        ] {
            let authored = display.clone();
            normalize_root_display(&mut display);
            assert_eq!(display, expected, "root {authored:?}");
        }
        for parent in [Display::Block, Display::Grid, Display::InlineGrid] {
            for mut display in [Display::Grid, Display::InlineGrid] {
                let authored = display.clone();
                normalize_child_display(&mut display, true, &parent);
                let expected = if parent.is_item_container() {
                    Display::Block
                } else {
                    Display::Inline
                };
                assert_eq!(display, expected, "replaced {authored:?} in {parent:?}");
            }
        }
    }
}
