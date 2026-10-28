use crate::document::navigation::{BoxNavigation, NavigationBuilder};
use crate::document::{DocumentError, Paragraph, inline::InlineCollector};
use crate::dom::{ChildNodes, DocumentTree, Element, ElementId, TreeRelation};
use crate::graphics::BoxDecoration;
use crate::style::{Display, PseudoElement, Style, StyleError, Stylesheet};

pub(super) const MAX_FORMATTING_DEPTH: usize = 512;
pub(super) const FORMATTING_NESTING_RESOURCE: &str = "formatting nesting";

fn canvas_background(decoration: &BoxDecoration) -> BoxDecoration {
    BoxDecoration {
        background: decoration.background,
        backgrounds: decoration.backgrounds.clone(),
        ..Default::default()
    }
}

pub(crate) struct PageBox {
    pub request: crate::style::PageRequest,
    pub geometry: crate::style::PageGeometry,
    #[cfg(test)]
    pub style: Style,
    pub production: crate::style::PageProduction,
    pub decoration: BoxDecoration,
    pub margins: Vec<PageMarginBox>,
}
pub(crate) struct PageMarginBox {
    pub margin: crate::style::PageMargin,
    pub decoration: BoxDecoration,
}
pub(crate) struct Formatted {
    pub root: BlockBox,
    pub navigation: NavigationBuilder,
    pub strings: crate::document::strings::StringAssignments,
    pub generated: crate::document::generated::GeneratedReferences,
    pub canvas: BoxDecoration,
    pub pages: crate::document::page::PageProvider,
}

pub(crate) struct BlockBox {
    style: Box<Style>,
    decoration: BoxDecoration,
    children: Vec<BlockContent>,
    marker: Option<Box<Paragraph>>,
    image: Option<std::sync::Arc<crate::images::Image>>,
    navigation: BoxNavigation,
    canvas_source: bool,
    event: Option<crate::document::SemanticEvent>,
    page: crate::document::PageBoundary,
}
pub(crate) enum BlockContent {
    Events(Vec<crate::document::SemanticEvent>),
    Block(BlockBox),
    Inline(Paragraph),
    /// Out-of-flow box (float or absolute positioning) anchored between flow blocks.
    Positioned(BlockBox),
}
impl BlockBox {
    pub(super) fn anonymous(
        style: Style,
        children: Vec<BlockContent>,
        page_scope: Option<std::sync::Arc<str>>,
    ) -> Self {
        let decoration = style.decoration.with_image_type();
        Self::with_decoration(style, children, decoration, page_scope)
    }
    fn with_decoration(
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
        let children = crate::document::tables::normalize(&style, children, page_scope.as_ref());
        let page = crate::document::PageBoundary::new(page_scope, &style, &children);
        Self {
            style: Box::new(style),
            decoration,
            children,
            marker: None,
            image: None,
            navigation: BoxNavigation::default(),
            canvas_source: false,
            event: None,
            page,
        }
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
    pub(crate) fn image(&self) -> Option<&std::sync::Arc<crate::images::Image>> {
        self.image.as_ref()
    }
    pub(crate) fn style(&self) -> &Style {
        &self.style
    }
    pub(crate) fn decoration(&self) -> &BoxDecoration {
        &self.decoration
    }
    pub(crate) fn children(&self) -> &[BlockContent] {
        &self.children
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
}

/// Document-order state of the boxes being built: list counter scopes and hyperlinks.
pub(crate) struct Traversal<'a> {
    document: &'a DocumentTree,
    root_style: Style,
    counters: crate::document::counters::Counters,
    navigation: NavigationBuilder,
    strings: crate::document::strings::StringAssignments,
    generated: crate::document::generated::GeneratedPass,
    quotes: crate::style::QuoteDepth,
    canvas: BoxDecoration,
    canvas_body: Option<ElementId>,
}

struct BlockBuilder<'a, 's, 'c> {
    sheet: &'a Stylesheet,
    style: &'s Style,
    element_page_scope: &'s Option<std::sync::Arc<str>>,
    children: &'s mut Vec<BlockContent>,
    inline: &'s mut InlineCollector,
    traversal: &'c mut Traversal<'a>,
    resources: &'c mut ImageResources<'a>,
}
pub(crate) struct ImageResources<'a> {
    pub base: &'a crate::resources::ResourceBase,
    pub resolver: &'a dyn crate::resources::ResourceResolver,
    pub cache: &'a mut crate::images::ImageCache,
}
impl ImageResources<'_> {
    fn load(
        &mut self,
        reference: &str,
        style: &Style,
    ) -> Result<std::sync::Arc<crate::images::Image>, crate::images::ImageError> {
        self.cache.load_in_namespace(
            reference,
            self.base,
            self.resolver,
            style.element_font_namespace(),
        )
    }
    /// Loads the box's `url()` background layers and border image; inline style URLs use the
    /// document base.
    pub(crate) fn backgrounds(&mut self, style: &Style) -> Result<BoxDecoration, DocumentError> {
        Ok(style
            .decoration
            .load_images(|reference| self.load(reference, style))?)
    }
}

impl BlockBuilder<'_, '_, '_> {
    fn flush(&mut self) -> Result<(), DocumentError> {
        match self.inline.split().finish(self.style)? {
            Ok(mut paragraph) => {
                paragraph.first_formatted_line = !self
                    .children
                    .iter()
                    .any(|child| matches!(child, BlockContent::Block(_) | BlockContent::Inline(_)));
                self.children.push(BlockContent::Inline(paragraph));
            }
            Err(stray) => self.children.extend(stray),
        }
        Ok(())
    }
    /// Item containers and positions outside inline boxes anchor between blocks.
    fn out_of_flow(&mut self, mut block: BlockBox) {
        if self.style.display.is_item_container() || self.inline.is_between_blocks() {
            block.style.opacity *= self.inline.open_opacity();
            self.children.push(BlockContent::Positioned(block));
        } else {
            self.inline.anchor(block);
        }
    }
    fn generated(
        &mut self,
        element: Element<'_>,
        parent: &Style,
        pseudo: PseudoElement,
    ) -> Result<(), DocumentError> {
        let Some((mut style, content)) =
            self.sheet
                .pseudo_with_resolver(element, parent, pseudo, self.resources.resolver)?
        else {
            return Ok(());
        };
        let element_page_scope =
            crate::document::page::used_name(&parent.page, self.element_page_scope.as_ref());
        validate_display(&style.display)?;
        normalize_child_display(&mut style.display, false, &self.style.display);
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
                pseudo: Some(pseudo),
            },
            &style.counters,
            &style.display,
        )?;
        let context = crate::style::PageStyleTarget {
            element: element.id(),
            pseudo: Some(pseudo),
        };
        let event = content
            .named_string_requests()
            .next()
            .is_some()
            .then(|| self.traversal.strings.generated_event(element, pseudo));
        let bound = self.traversal.generated.bind(&content, context)?;
        let output = bound.element_output_with_counters(
            crate::document::MAX_INLINE_TEXT_BYTES,
            &mut |request| self.traversal.counters.text(request),
            &style.quotes,
            &mut self.traversal.quotes,
        )?;
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
        if style.display != Display::Contents
            && (style.position.is_out_of_flow() || style.floating().is_some())
        {
            let children = Paragraph::generated_output(&output, &style)?
                .into_iter()
                .map(BlockContent::Inline)
                .collect();
            self.out_of_flow(self.generated_block(
                style,
                children,
                decoration,
                element_page_scope.as_ref(),
                event,
            ));
            return Ok(());
        }
        match &style.display {
            Display::Contents => {
                if let Some(event) = event {
                    self.inline.semantic_event(event);
                }
                self.inline.append_generated(&output, &style)?;
            }
            Display::Inline => {
                validate_inline(&style)?;
                if let Some(event) = event {
                    self.inline.semantic_event(event);
                }
                self.inline.open_box(
                    &style,
                    decoration,
                    self.traversal.navigation.descendant(),
                    false,
                );
                self.inline.append_generated(&output, &style)?;
                self.inline.close_box();
            }
            Display::InlineBlock
            | Display::InlineFlex
            | Display::InlineGrid
            | Display::InlineTable => {
                let children = Paragraph::generated_output(&output, &style)?
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
                self.inline.atomic(atomic, parent)?;
            }
            Display::Unsupported(value) => {
                return Err(StyleError::Unsupported(format!("generated display: {value}")).into());
            }
            Display::ListItem | Display::FlowRootListItem => {
                return Err(DocumentError::Formatting(
                    "generated list-item counters are not supported".into(),
                ));
            }
            Display::None => unreachable!("hidden pseudo-elements are excluded by the cascade"),
            _ => {
                self.flush()?;
                style.opacity *= self.inline.open_opacity();
                let children = Paragraph::generated_output(&output, &style)?
                    .into_iter()
                    .map(BlockContent::Inline)
                    .collect();
                let generated = self.generated_block(
                    style,
                    children,
                    decoration,
                    element_page_scope.as_ref(),
                    event,
                );
                self.children.push(BlockContent::Block(generated));
            }
        }
        Ok(())
    }

    /// A pseudo-element's block, part of the current element's hyperlink.
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

fn validate_display(display: &Display) -> Result<(), StyleError> {
    display.validate()
}

fn validate_inline(style: &Style) -> Result<(), DocumentError> {
    if style.position != crate::style::Position::Static {
        return Err(DocumentError::Formatting(
            "inline positioning is not supported".into(),
        ));
    }
    Ok(())
}

fn unsupported_element(tag: &str) -> bool {
    matches!(
        tag,
        "math"
            | "input"
            | "textarea"
            | "select"
            | "button"
            | "video"
            | "audio"
            | "canvas"
            | "iframe"
            | "object"
            | "embed"
            | "picture"
            | "fieldset"
    )
}

/// Elements whose content is an image rather than child boxes.
pub(super) fn is_replaced(tag: &str) -> bool {
    matches!(tag, "img" | "svg")
}

pub(super) fn normalize_child_display(display: &mut Display, replaced: bool, parent: &Display) {
    if replaced
        && (display.is_table_internal()
            || display.is_atomic_inline()
            || matches!(
                display,
                Display::Table
                    | Display::TableCaption
                    | Display::Flex
                    | Display::Grid
                    | Display::ListItem
                    | Display::FlowRootListItem
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
        .map(Box::new)
        .map_err(Into::into)
}

#[derive(Clone, Copy)]
enum Placement {
    Root,
    Flow,
    Positioned,
    Atomic,
}

struct OpenBlock {
    decoration: BoxDecoration,
    children: Vec<BlockContent>,
    inline: InlineCollector,
    marker: Option<Paragraph>,
    image: Option<std::sync::Arc<crate::images::Image>>,
    navigation: BoxNavigation,
    placement: Placement,
    event: Option<crate::document::SemanticEvent>,
}

struct Frame<'d> {
    element: Element<'d>,
    style: Box<Style>,
    page_scope: Option<std::sync::Arc<str>>,
    depth: usize,
    nodes: Option<ChildNodes<'d>>,
    block: Option<OpenBlock>,
    started: bool,
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
            nodes: None,
            block: placement.map(|placement| OpenBlock {
                decoration: BoxDecoration::default(),
                children: Vec::new(),
                inline: InlineCollector::default(),
                marker: None,
                image: None,
                navigation: BoxNavigation::default(),
                placement,
                event: None,
            }),
            started: false,
            generated: false,
            ordinal,
        }
    }
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
            style: &frame.style,
            element_page_scope,
            children: &mut block.children,
            inline: &mut block.inline,
            traversal,
            resources,
        },
        style,
    )
}

fn block<'a>(
    element: Element<'_>,
    style: Box<Style>,
    sheet: &'a Stylesheet,
    depth: usize,
    ordinal: i128,
    traversal: &mut Traversal<'a>,
    resources: &mut ImageResources<'a>,
) -> Result<BlockBox, DocumentError> {
    let mut frames = vec![Frame::new(
        element,
        style,
        depth,
        ordinal,
        Some(Placement::Root),
        None,
    )];
    let mut blocks = vec![0];
    loop {
        let index = frames.len() - 1;
        let block_index = *blocks.last().unwrap();
        if !frames[index].started {
            let frame = &mut frames[index];
            let element = frame.element;
            traversal.generated.capture(element, &traversal.counters);
            if let Some(block) = &mut frame.block {
                block.decoration = resources.backgrounds(&frame.style)?;
                if index == 0 {
                    traversal.canvas = canvas_background(&block.decoration);
                }
                block.image = match element.name() {
                    "img" => {
                        let image = element.attr("src").and_then(|reference| {
                            resources.cache.load_optional(
                                reference,
                                resources.base,
                                resources.resolver,
                                frame.style.element_font_namespace(),
                            )
                        });
                        if image.is_none()
                            && let Some(alt) = element.attr("alt")
                        {
                            block.inline.append(alt, &frame.style)?;
                        }
                        image
                    }
                    "svg" => Some(resources.cache.inline_svg_in_namespace(
                        crate::document::svg::serialize(element),
                        resources.base,
                        resources.resolver,
                        frame.style.element_font_namespace(),
                    )?),
                    _ => None,
                };
                block.marker = frame
                    .style
                    .display
                    .is_list_item()
                    .then(|| crate::document::lists::marker(&frame.style, frame.ordinal, resources))
                    .transpose()?
                    .flatten();
                let assignment = traversal.strings.record(
                    element,
                    &frame.style.string_set,
                    &mut traversal.counters,
                    block.marker.as_ref().map(Paragraph::text),
                )?;
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
                if frame.style.has_inside_marker()
                    && let Some(marker) = block.marker.take()
                {
                    if let Some(image) = marker.marker_image() {
                        let [width, height] =
                            marker.marker_image_size().expect("image marker has size");
                        let mut style = frame.style.anonymous(Display::InlineBlock);
                        style.width = crate::style::Size::Value(
                            crate::style::LengthPercentage::Length(width),
                        );
                        style.height = crate::style::Size::Value(
                            crate::style::LengthPercentage::Length(height),
                        );
                        let mut decoration = BoxDecoration::default();
                        std::sync::Arc::make_mut(&mut decoration.backgrounds).images =
                            vec![image.clone()];
                        let atomic = BlockBox::with_decoration(
                            style,
                            Vec::new(),
                            decoration,
                            frame.page_scope.clone(),
                        );
                        block.inline.atomic(atomic, marker.style())?;
                        block.inline.append(marker.text(), marker.style())?;
                    } else {
                        block.inline.append(marker.text(), &frame.style)?;
                    }
                }
                frame.generated =
                    block.image.is_none() && frame.style.display != Display::TableColumn;
            } else {
                let assignment = traversal.strings.record(
                    element,
                    &frame.style.string_set,
                    &mut traversal.counters,
                    None,
                )?;
                let event = traversal.strings.event(element, assignment, true);
                let mut navigation = traversal.navigation.enter(element);
                traversal.navigation.record(
                    element,
                    &frame.style,
                    None,
                    traversal.quotes,
                    if frame.style.bookmark.level.is_some() {
                        traversal.counters.snapshot(&frame.style.bookmark.label)?
                    } else {
                        Default::default()
                    },
                    &mut navigation,
                );
                let canvas_source = index == 0 || traversal.canvas_body == Some(element.id());
                let (builder, style) =
                    active_builder(&mut frames, block_index, sheet, traversal, resources);
                builder.inline.semantic_event(event);
                if style.display == Display::Contents {
                    builder.inline.open_contents(navigation.link);
                } else {
                    let decoration = builder.resources.backgrounds(style)?;
                    builder
                        .inline
                        .open_box(style, decoration, navigation, canvas_source);
                }
                frames[index].generated = true;
            }
            if frames[index].generated || element.name() == "br" {
                if frames[index].depth > MAX_FORMATTING_DEPTH {
                    return Err(DocumentError::Formatting(format!(
                        "{FORMATTING_NESTING_RESOURCE} exceeds {MAX_FORMATTING_DEPTH}"
                    )));
                }
                let (mut builder, style) =
                    active_builder(&mut frames, block_index, sheet, traversal, resources);
                builder.generated(element, style, PseudoElement::Before)?;
                if element.name() == "br" {
                    builder.inline.line_break(style)?;
                    builder.generated(element, style, PseudoElement::After)?;
                    frames[index].generated = false;
                } else {
                    if element.namespace() == crate::dom::HTML_NAMESPACE && element.name() == "wbr"
                    {
                        builder.inline.optional_break(style);
                    }
                    frames[index].nodes = Some(element.child_nodes(TreeRelation::Formatting));
                }
            }
            frames[index].started = true;
            continue;
        }
        if let Some(node) = frames[index].nodes.as_mut().and_then(Iterator::next) {
            if let Some(text) = node.value().as_text() {
                let (builder, style) =
                    active_builder(&mut frames, block_index, sheet, traversal, resources);
                builder.inline.append(text, style)?;
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
                    canvas_background(&resources.backgrounds(&child_style)?)
                };
            }
            validate_display(&child_style.display)?;
            let replaced = is_replaced(child.name());
            normalize_child_display(
                &mut child_style.display,
                replaced,
                &frames[block_index].style.display,
            );
            if frames[block_index].style.display == Display::TableColumnGroup
                && child_style.display != Display::Contents
                && child_style.display != Display::TableColumn
            {
                continue;
            }
            if child_style.display == Display::None {
                let mut events = Vec::new();
                sheet.with_document_values(
                    traversal.document,
                    &traversal.root_style,
                    resources.resolver,
                    |styles| {
                        traversal.counters.visit_hidden(
                            child,
                            styles,
                            |element, assignments, counters| {
                                traversal.generated.capture(element, counters);
                                let assignment = traversal.strings.record(
                                    element,
                                    assignments,
                                    counters,
                                    None,
                                )?;
                                if assignment.is_some() {
                                    events
                                        .push(traversal.strings.event(element, assignment, false));
                                }
                                Ok(())
                            },
                        )
                    },
                )?;
                let (builder, _) =
                    active_builder(&mut frames, block_index, sheet, traversal, resources);
                for event in events {
                    builder.inline.semantic_event(event);
                }
                continue;
            }
            if child_style.display != Display::Contents && unsupported_element(child.name()) {
                return Err(DocumentError::Formatting(format!(
                    "unsupported formatting element: {}",
                    child.name()
                )));
            }
            traversal.counters.prepare(
                &child_style.counters,
                traversal.document,
                sheet,
                &traversal.root_style,
                resources.resolver,
            )?;
            traversal.counters.enter(
                crate::style::PageStyleTarget {
                    element: child.id(),
                    pseudo: None,
                },
                &child_style.counters,
                &child_style.display,
            )?;
            let child_depth = frames[index].depth + 1;
            let ancestor_page = frames[index].page_scope.clone();
            let (mut builder, _) =
                active_builder(&mut frames, block_index, sheet, traversal, resources);
            let placement = if child_style.display == Display::Contents {
                None
            } else if child_style.position.is_out_of_flow() || child_style.floating().is_some() {
                Some(Placement::Positioned)
            } else {
                match &child_style.display {
                    Display::None => unreachable!("hidden elements are skipped before traversal"),
                    Display::Contents => unreachable!("boxless elements have no placement"),
                    Display::Block
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
                        builder.flush()?;
                        child_style.opacity *= builder.inline.open_opacity();
                        Some(Placement::Flow)
                    }
                    Display::InlineBlock
                    | Display::InlineFlex
                    | Display::InlineGrid
                    | Display::InlineTable => Some(Placement::Atomic),
                    Display::Inline if replaced => Some(Placement::Atomic),
                    Display::Inline => {
                        validate_inline(&child_style)?;
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
                            builder.generated(child, &child_style, PseudoElement::Before)?;
                            builder.inline.line_break(&child_style)?;
                            builder.generated(child, &child_style, PseudoElement::After)?;
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
            if placement.is_some() {
                blocks.push(frames.len());
            }
            frames.push(Frame::new(
                child,
                child_style,
                child_depth,
                ordinal,
                placement,
                ancestor_page.as_ref(),
            ));
            continue;
        }
        let element = frames[index].element;
        let generated = frames[index].generated;
        let contents = frames[index].style.display == Display::Contents;
        let (mut builder, style) =
            active_builder(&mut frames, block_index, sheet, traversal, resources);
        if generated {
            builder.generated(element, style, PseudoElement::After)?;
        }
        if index != block_index {
            if contents {
                builder.inline.close_contents();
            } else {
                builder.inline.close_box();
            }
            builder.traversal.navigation.leave();
            frames.pop();
            traversal.counters.leave();
            continue;
        }
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
        let state = frame.block.unwrap();
        let canvas_source = index == 0 || traversal.canvas_body == Some(element.id());
        let mut block = BlockBox::with_decoration(
            *frame.style,
            state.children,
            state.decoration,
            frame.page_scope,
        );
        block.marker = state.marker.map(Box::new);
        block.image = state.image;
        block.navigation = state.navigation;
        block.event = state.event;
        block.canvas_source = canvas_source;
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
            Placement::Flow => builder.children.push(BlockContent::Block(block)),
            Placement::Positioned => builder.out_of_flow(block),
            Placement::Atomic => builder.inline.atomic(block, parent)?,
            Placement::Root => unreachable!("root returned before attaching to a parent"),
        }
        traversal.counters.leave();
    }
}

pub(crate) fn build(
    html: &DocumentTree,
    sheet: &mut Stylesheet,
    base: &crate::resources::ResourceBase,
    resolver: &dyn crate::resources::ResourceResolver,
    navigation: NavigationBuilder,
    values: crate::document::generated::GeneratedValues,
    images: Option<std::rc::Rc<std::cell::RefCell<crate::images::ImageCache>>>,
) -> Result<Formatted, DocumentError> {
    let mut generated =
        crate::document::generated::GeneratedPass::new(navigation.targets.clone(), values);
    let element = html.root_element();
    let mut style = sheet
        .resolve_document_root_with_pages(
            html,
            resolver,
            &|styles| crate::document::first_page::select_name(html, styles, &generated),
            &|styles| crate::document::first_page::select_side(html, styles, &generated),
        )
        .map_err(|error| match error {
            StyleError::GrammarLimit {
                resource: FORMATTING_NESTING_RESOURCE,
                limit,
            } => {
                DocumentError::Formatting(format!("{FORMATTING_NESTING_RESOURCE} exceeds {limit}"))
            }
            error => DocumentError::Style(error),
        })?;
    let sheet = &*sheet;
    let images = images.unwrap_or_else(|| {
        std::rc::Rc::new(std::cell::RefCell::new(
            crate::images::ImageCache::with_fonts(sheet.fonts.clone()),
        ))
    });
    let mut cache = images.borrow_mut();
    let mut resources = ImageResources {
        base,
        resolver,
        cache: &mut cache,
    };
    let area = sheet.page_content_box();
    let resolved = sheet.resolve_page(
        &style,
        sheet.first_page_request().clone(),
        Some([area.width, area.height]),
        resolver,
    )?;
    let bindings = generated.bindings();
    let page = PageBox::load(resolved, &mut resources, &bindings)?;
    validate_display(&style.display)?;
    if style.display == Display::None {
        let mut strings = crate::document::strings::StringAssignments::new(html);
        let mut counters = crate::document::counters::Counters::default();
        let mut events = Vec::new();
        sheet.with_document_values(html, &style, resolver, |styles| {
            counters.visit_hidden(element, styles, |element, assignments, counters| {
                generated.capture(element, counters);
                let assignment = strings.record(element, assignments, counters, None)?;
                if assignment.is_some() {
                    events.push(strings.event(element, assignment, false));
                }
                Ok(())
            })
        })?;
        return Ok(Formatted {
            root: BlockBox::anonymous(style, vec![BlockContent::Events(events)], None),
            navigation,
            strings,
            generated: generated.finish(),
            canvas: BoxDecoration::default(),
            pages: crate::document::page::PageProvider::new(
                page,
                images.clone(),
                base.clone(),
                bindings,
            ),
        });
    }
    normalize_root_display(&mut style.display);
    let mut traversal = Traversal {
        document: html,
        root_style: style.clone(),
        counters: Default::default(),
        navigation,
        strings: crate::document::strings::StringAssignments::new(html),
        generated,
        quotes: Default::default(),
        canvas: BoxDecoration::default(),
        canvas_body: (style.decoration.background.alpha == 0.0
            && !style.decoration.backgrounds.has_images())
        .then(|| html.body_element().map(Element::id))
        .flatten(),
    };
    traversal
        .counters
        .prepare(&style.counters, html, sheet, &style, resolver)?;
    traversal.counters.enter(
        crate::style::PageStyleTarget {
            element: element.id(),
            pseudo: None,
        },
        &style.counters,
        &style.display,
    )?;
    let ordinal = if style.display.is_list_item() {
        traversal.counters.ordinal()?
    } else {
        0
    };
    let root = block(
        element,
        Box::new(style),
        sheet,
        0,
        ordinal,
        &mut traversal,
        &mut resources,
    )?;
    Ok(Formatted {
        root,
        navigation: traversal.navigation,
        strings: traversal.strings,
        generated: traversal.generated.finish(),
        canvas: traversal.canvas,
        pages: crate::document::page::PageProvider::new(
            page,
            images.clone(),
            base.clone(),
            bindings,
        ),
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
