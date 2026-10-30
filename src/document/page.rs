use crate::document::BlockContent;
use crate::style::{Display, Page, Style};
use std::sync::Arc;

use crate::document::{DocumentError, PageBox, PageMarginBox, formatting::ImageResources};
use crate::resources::{ResourceBase, ResourceResolver};
use crate::style::{PageRequest, ResolvedPage, Stylesheet};
use std::{cell::RefCell, collections::HashMap};

impl PageBox {
    pub(crate) fn load(
        page: ResolvedPage,
        resources: &mut ImageResources<'_>,
        generated: &crate::document::generated::GeneratedBindings,
    ) -> Result<Self, DocumentError> {
        let decoration = resources.backgrounds(&page.style)?;
        let margins = page
            .margins
            .into_iter()
            .map(|mut margin| {
                margin.content = generated.bind(&margin.content, None)?.into_owned();
                let decoration = resources.backgrounds(&margin.style)?;
                Ok(PageMarginBox { margin, decoration })
            })
            .collect::<Result<_, DocumentError>>()?;
        Ok(Self {
            request: page.request,
            geometry: page.geometry,
            #[cfg(test)]
            style: page.style,
            production: page.production,
            decoration,
            margins,
        })
    }
}

pub(crate) struct PageProvider {
    first: Arc<PageBox>,
    records: RefCell<HashMap<PageRequest, Arc<PageBox>>>,
    images: std::rc::Rc<RefCell<crate::images::ImageCache>>,
    base: ResourceBase,
    generated: crate::document::generated::GeneratedBindings,
}
impl PageProvider {
    pub(crate) fn new(
        first: PageBox,
        images: std::rc::Rc<RefCell<crate::images::ImageCache>>,
        base: ResourceBase,
        generated: crate::document::generated::GeneratedBindings,
    ) -> Self {
        let first = Arc::new(first);
        Self {
            records: RefCell::new(HashMap::from([(first.request.clone(), first.clone())])),
            first,
            images,
            base,
            generated,
        }
    }
    pub(crate) fn images(&self) -> std::rc::Rc<RefCell<crate::images::ImageCache>> {
        self.images.clone()
    }
    pub(crate) fn first(&self) -> &PageBox {
        &self.first
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
        let resolved = sheet.resolve_page(
            root,
            request.clone(),
            Some(self.first.geometry.area),
            resolver,
        )?;
        let mut cache = self.images.borrow_mut();
        let mut resources = ImageResources {
            base: &self.base,
            resolver,
            cache: &mut cache,
        };
        let page = Arc::new(PageBox::load(resolved, &mut resources, &self.generated)?);
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
