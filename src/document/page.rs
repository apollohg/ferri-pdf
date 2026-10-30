use crate::document::BlockContent;
use crate::style::{Display, Page, Style};
use std::sync::Arc;

use crate::document::{DocumentError, PageBox, PageMarginBox, formatting::ImageResources};
use crate::resources::{ResourceBase, ResourceResolver};
use crate::style::{PageRequest, ResolvedPage, Stylesheet};
use std::{cell::RefCell, collections::HashMap};

impl PageBox {
    /// The page area's block size `block_size` as the containing block size of the content of
    /// a document whose root flows in `flow`: indefinite where the page's block size depends
    /// on that content, so that percentage heights against it compute to `auto` (CSS 2.1
    /// §10.5).
    pub(crate) fn containing_block_size(
        &self,
        flow: crate::style::logical::Flow,
        block_size: f32,
    ) -> Option<f32> {
        let [_, block_axis] = page_axes(flow);
        self.geometry.intrinsic[block_axis]
            .is_none()
            .then_some(block_size)
    }
    pub(crate) fn load(
        page: ResolvedPage,
        resources: &mut ImageResources<'_>,
        generated: &crate::document::generated::GeneratedBindings,
    ) -> Result<Self, DocumentError> {
        // CSS Page 3 Appendix A: `visibility` applies to the page box, which hidden paints
        // nothing of its own but still masks what it holds.
        let decoration = if page.style.visibility.is_visible() {
            resources.backgrounds(&page.style)?
        } else {
            crate::graphics::BoxDecoration {
                masks: resources.masks(&page.style)?,
                ..crate::graphics::BoxDecoration::default()
            }
        };
        let margins = page
            .margins
            .into_iter()
            .map(|mut margin| {
                margin.content = generated.bind(&margin.content, None)?.into_owned();
                let decoration = resources.backgrounds(&margin.style)?;
                Ok(PageMarginBox { margin, decoration })
            })
            .collect::<Result<_, DocumentError>>()?;
        let footnote = crate::document::PageFootnoteArea {
            decoration: resources.backgrounds(&page.footnote)?,
            style: page.footnote,
        };
        Ok(Self {
            request: page.request,
            geometry: page.geometry,
            counters: page.style.counters.clone(),
            style: page.style,
            production: page.production,
            decoration,
            margins,
            footnote,
        })
    }
}

/// The block size of a page area whose content-sized block axis has no measure: the largest
/// page dimension a PDF describes, in points (ISO 32000-1 Annex C.2). Content beyond it
/// continues on the next page.
const UNBOUNDED_PAGE_EXTENT: f32 = 14_400.0;

/// The physical axes, horizontal 0 and vertical 1, that are inline and block on the pages of
/// a document whose root flows in `flow`.
pub(crate) fn page_axes(flow: crate::style::logical::Flow) -> [usize; 2] {
    if flow.writing_mode.vertical() {
        [1, 0]
    } else {
        [0, 1]
    }
}

/// What a layout measured of the content of pages whose size properties take intrinsic size
/// keywords (CSS Page 3 §3).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PageContent {
    /// The root's min-content and max-content contributions, which a page area holds in its
    /// inline axis (CSS Sizing 3 §5.1).
    pub inline: Option<crate::style::IntrinsicWidths>,
    /// The block size of the content each page holds: the flow up to the forced break or end
    /// that ends the page, with its footnote area. `None` where the content continues past
    /// the page's end, so that unfragmented it would be larger.
    pub blocks: Vec<Option<f32>>,
}
impl PageContent {
    /// `page`, the page at `index` of a document whose root flows in `flow`, sized to the
    /// content it holds. A block container's min-content and max-content block sizes are the
    /// size of its content (CSS Sizing 3 §5.1); a page whose content is unmeasured or
    /// continues past its end takes the largest page.
    pub(crate) fn size(
        &self,
        page: &Arc<PageBox>,
        index: usize,
        flow: crate::style::logical::Flow,
    ) -> Result<Arc<PageBox>, DocumentError> {
        if page.geometry.intrinsic.iter().all(Option::is_none) {
            return Ok(page.clone());
        }
        let extent = self
            .blocks
            .get(index)
            .copied()
            .flatten()
            .unwrap_or(UNBOUNDED_PAGE_EXTENT);
        let [inline_axis, block_axis] = page_axes(flow);
        let mut content = [None; 2];
        content[inline_axis] = self.inline;
        content[block_axis] = Some(crate::style::IntrinsicWidths {
            min: extent,
            max: extent,
        });
        let mut sized = PageBox::clone(page);
        sized.geometry = page.geometry.sized(content)?;
        Ok(Arc::new(sized))
    }
}

pub(crate) struct PageProvider {
    first: Arc<PageBox>,
    records: RefCell<HashMap<PageRequest, Arc<PageBox>>>,
    images: std::rc::Rc<RefCell<crate::images::ImageCache>>,
    base: ResourceBase,
    generated: crate::document::generated::GeneratedBindings,
    elements: std::rc::Rc<crate::document::element_images::ElementImages>,
}
impl PageProvider {
    pub(crate) fn new(
        first: PageBox,
        images: std::rc::Rc<RefCell<crate::images::ImageCache>>,
        base: ResourceBase,
        generated: crate::document::generated::GeneratedBindings,
        elements: std::rc::Rc<crate::document::element_images::ElementImages>,
    ) -> Self {
        let first = Arc::new(first);
        Self {
            records: RefCell::new(HashMap::from([(first.request.clone(), first.clone())])),
            first,
            images,
            base,
            generated,
            elements,
        }
    }
    pub(crate) fn images(&self) -> std::rc::Rc<RefCell<crate::images::ImageCache>> {
        self.images.clone()
    }
    /// The `element()` images of the formatting that made these pages.
    pub(crate) fn elements(&self) -> &crate::document::element_images::ElementImages {
        &self.elements
    }
    pub(crate) fn first(&self) -> &Arc<PageBox> {
        &self.first
    }
    /// The images of a page-margin box's generated content on one page, each loaded as its
    /// anonymous replaced box shows it; invalid ones generate nothing (CSS Content 3 §2.2).
    pub(crate) fn margin_images(
        &self,
        output: &crate::style::GeneratedOutput,
        style: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Vec<crate::document::inline::GeneratedImage> {
        let mut cache = self.images.borrow_mut();
        ImageResources {
            base: &self.base,
            resolver,
            cache: &mut cache,
            elements: &self.elements,
        }
        .generated_images(output, style)
    }
    /// The image a page-margin box's `<content-replacement>` replaces it with (CSS Content 3
    /// §1).
    pub(crate) fn margin_replacement(
        &self,
        image: &crate::paint::BackgroundImage,
        style: &Style,
        resolver: &dyn ResourceResolver,
    ) -> Arc<crate::images::Image> {
        let mut cache = self.images.borrow_mut();
        ImageResources {
            base: &self.base,
            resolver,
            cache: &mut cache,
            elements: &self.elements,
        }
        .replacement_image(image, style)
    }
    pub(crate) fn resolve(
        &self,
        sheet: &Stylesheet,
        root: &Style,
        request: PageRequest,
        resolver: &dyn ResourceResolver,
    ) -> Result<Arc<PageBox>, DocumentError> {
        if let Some(page) = self.records.borrow().get(&request) {
            return Ok(page.clone());
        }
        let mut cache = self.images.borrow_mut();
        self.load(
            sheet,
            root,
            request,
            &mut ImageResources {
                base: &self.base,
                resolver,
                cache: &mut cache,
                elements: &self.elements,
            },
        )
    }
    /// As `resolve`, loading the page's images with `resources` while formatting holds the
    /// image cache.
    pub(crate) fn resolve_with(
        &self,
        sheet: &Stylesheet,
        root: &Style,
        request: PageRequest,
        resources: &mut ImageResources<'_>,
    ) -> Result<Arc<PageBox>, DocumentError> {
        if let Some(page) = self.records.borrow().get(&request) {
            return Ok(page.clone());
        }
        self.load(sheet, root, request, resources)
    }
    fn load(
        &self,
        sheet: &Stylesheet,
        root: &Style,
        request: PageRequest,
        resources: &mut ImageResources<'_>,
    ) -> Result<Arc<PageBox>, DocumentError> {
        let resolved = sheet.resolve_page(
            root,
            request.clone(),
            Some(self.first.geometry.area),
            resources.resolver,
        )?;
        let page = Arc::new(PageBox::load(resolved, resources, &self.generated)?);
        self.records.borrow_mut().insert(request, page.clone());
        Ok(page)
    }
}

/// Used ancestor scope and the values propagated at the box's flow boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageBoundary {
    pub used: Option<Arc<str>>,
    pub start: Option<Arc<str>>,
    pub end: Option<Arc<str>>,
    /// The page group this box creates.
    pub group: Option<Arc<str>>,
    /// The page groups that start where this box starts, outermost first: its own and those
    /// propagated from its first in-flow child, as the start name is.
    pub start_groups: Vec<Arc<str>>,
}

/// The page group a box creates: one with a page name and a forced page break before it
/// (CSS GCPM 3 §3.2).
pub(crate) fn page_group(page: &Page, before: crate::style::BreakBetween) -> Option<Arc<str>> {
    match page {
        Page::Name(name) if before.forces_page() => Some(name.clone()),
        _ => None,
    }
}

pub(crate) fn used_name(page: &Page, ancestor: Option<&Arc<str>>) -> Option<Arc<str>> {
    match page {
        Page::Auto => ancestor.cloned(),
        Page::Name(name) => Some(name.clone()),
    }
}

impl PageBoundary {
    pub(crate) fn applies(display: &Display) -> bool {
        matches!(
            display,
            Display::Block
                | Display::FlowRoot
                | Display::FlowRootListItem
                | Display::Flex
                | Display::ListItem
                | Display::Table
                | Display::TableRowGroup
                | Display::TableHeaderGroup
                | Display::TableFooterGroup
                | Display::TableRow
                | Display::TableCaption
        )
    }

    pub(crate) fn new(used: Option<Arc<str>>, style: &Style, children: &[BlockContent]) -> Self {
        let children: Vec<_> = in_flow_children(style, children)
            .into_iter()
            .filter(|child| !matches!(child, BlockContent::Events(_)))
            .collect();
        let boundary = |child: &&BlockContent, start| match child {
            BlockContent::Block(child) if Self::applies(&child.style().display) => {
                let page = child.page_boundary();
                if start {
                    page.start.clone()
                } else {
                    page.end.clone()
                }
            }
            _ => used.clone(),
        };
        let group = page_group(&style.page, style.break_before);
        let mut start_groups: Vec<_> = group.iter().cloned().collect();
        if let Some(BlockContent::Block(child)) = children.first()
            && Self::applies(&child.style().display)
        {
            start_groups.extend(child.page_boundary().start_groups.iter().cloned());
        }
        Self {
            start: children
                .first()
                .map_or_else(|| used.clone(), |child| boundary(child, true)),
            end: children
                .last()
                .map_or_else(|| used.clone(), |child| boundary(child, false)),
            used,
            group,
            start_groups,
        }
    }
}

pub(crate) fn in_flow_children<'a>(
    style: &Style,
    children: &'a [BlockContent],
) -> Vec<&'a BlockContent> {
    let mut children = children
        .iter()
        .filter(|child| !matches!(child, BlockContent::Positioned(_)))
        .collect::<Vec<_>>();
    if style.display.is_item_container() {
        children.sort_by_key(|child| match child {
            BlockContent::Block(block) => block.style().flex.order,
            _ => 0,
        });
    } else if style.display.is_table() {
        crate::document::tables::order(&mut children);
    }
    children
}
