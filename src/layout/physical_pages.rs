use crate::document::{PageBox, StyledDocument};
use crate::layout::{LayoutError, PageRecord};
use crate::style::{PageRequest, PageSide};
use ferri_pdf_resources::ResourceResolver;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct PhysicalPages<'a, 'r> {
    pub document: &'a StyledDocument,
    resolver: &'r dyn ResourceResolver,
    pub boxes: Vec<Arc<PageBox>>,
    pub records: Vec<PageRecord>,
    pub name: Option<Arc<str>>,
}
impl<'a, 'r> PhysicalPages<'a, 'r> {
    pub(crate) fn new(
        document: &'a StyledDocument,
        resolver: &'r dyn ResourceResolver,
    ) -> Result<Self, LayoutError> {
        let first = document.resolve_page(document.page_box().request.clone(), resolver)?;
        Ok(Self {
            document,
            resolver,
            name: first.request.name.clone(),
            records: vec![PageRecord::new(&first)?],
            boxes: vec![first],
        })
    }
    pub(crate) fn resolve(&self, index: usize) -> Result<Arc<PageBox>, LayoutError> {
        self.resolve_blank(index, false)
    }
    pub(crate) fn side(&self, index: usize) -> PageSide {
        match (self.records[0].request.side, index.is_multiple_of(2)) {
            (PageSide::Left, true) | (PageSide::Right, false) => PageSide::Left,
            _ => PageSide::Right,
        }
    }
    fn resolve_blank(&self, index: usize, blank: bool) -> Result<Arc<PageBox>, LayoutError> {
        Ok(self.document.resolve_page(
            PageRequest {
                name: self.name.clone(),
                side: self.side(index),
                first: index == 0,
                blank,
            },
            self.resolver,
        )?)
    }
    pub(crate) fn ensure(&mut self, index: usize) -> Result<(), LayoutError> {
        while self.records.len() <= index {
            let page = self.resolve(self.records.len())?;
            self.records.push(PageRecord::new(&page)?);
            self.boxes.push(page);
        }
        if self.records[index].request.blank {
            let page = self.resolve(index)?;
            self.records[index] = PageRecord::new(&page)?;
            self.boxes[index] = page;
        }
        Ok(())
    }
    pub(crate) fn ensure_break(&mut self, index: usize) -> Result<(), LayoutError> {
        while self.records.len() < index {
            let page = self.resolve_blank(self.records.len(), true)?;
            self.records.push(PageRecord::new(&page)?);
            self.boxes.push(page);
        }
        self.ensure(index)?;
        Ok(())
    }
}
