use crate::document::{PageBox, StyledDocument};
use crate::layout::{LayoutError, PageRecord};
use crate::resources::ResourceResolver;
use crate::style::{PageGroupPosition, PageRequest, PageSequence, PageSide};
use std::sync::Arc;

/// A page group whose box is being laid out (CSS GCPM 3 §3.2).
#[derive(Clone)]
pub(super) struct OpenGroup {
    pub owner: *const crate::document::BlockBox,
    pub name: Arc<str>,
    pub start: usize,
}

#[derive(Clone)]
pub(super) struct PhysicalPages<'a, 'r> {
    pub document: &'a StyledDocument,
    resolver: &'r dyn ResourceResolver,
    pub boxes: Vec<Arc<PageBox>>,
    pub records: Vec<PageRecord>,
    pub name: Option<Arc<str>>,
    /// Groups that start at the break being taken, before their boxes are laid out.
    pub starting: Vec<Arc<str>>,
    pub open: Vec<OpenGroup>,
}
impl<'a, 'r> PhysicalPages<'a, 'r> {
    pub(crate) fn new(
        document: &'a StyledDocument,
        resolver: &'r dyn ResourceResolver,
    ) -> Result<Self, LayoutError> {
        let first = document.sized_page(
            &document.resolve_page(document.page_box().request.clone(), resolver)?,
            0,
        )?;
        Ok(Self {
            document,
            resolver,
            name: first.request.name.clone(),
            starting: Vec::new(),
            open: Vec::new(),
            records: vec![Self::record(document, &first)?],
            boxes: vec![first],
        })
    }
    fn record(document: &StyledDocument, page: &PageBox) -> Result<PageRecord, LayoutError> {
        let mut record = PageRecord::new(page)?;
        record.reflow(crate::style::logical::Flow::used(document.root().style()));
        Ok(record)
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
    /// A blank page precedes the groups starting after it.
    fn resolve_blank(&self, index: usize, blank: bool) -> Result<Arc<PageBox>, LayoutError> {
        let sequence = self
            .document
            .styles()
            .selects_page_positions()
            .then(|| PageSequence {
                number: index + 1,
                groups: self
                    .open
                    .iter()
                    .map(|group| PageGroupPosition {
                        name: group.name.clone(),
                        number: index - group.start + 1,
                    })
                    .chain(
                        (!blank)
                            .then_some(&self.starting)
                            .into_iter()
                            .flatten()
                            .map(|name| PageGroupPosition {
                                name: name.clone(),
                                number: 1,
                            }),
                    )
                    .collect(),
            });
        let page = self.document.resolve_page(
            PageRequest {
                name: self.name.clone(),
                side: self.side(index),
                first: index == 0,
                blank,
                sequence,
            },
            self.resolver,
        )?;
        Ok(self.document.sized_page(&page, index)?)
    }
    pub(crate) fn ensure(&mut self, index: usize) -> Result<(), LayoutError> {
        while self.records.len() <= index {
            let page = self.resolve(self.records.len())?;
            self.records.push(Self::record(self.document, &page)?);
            self.boxes.push(page);
        }
        if self.records[index].request.blank {
            let page = self.resolve(index)?;
            self.records[index] = Self::record(self.document, &page)?;
            self.boxes[index] = page;
        }
        Ok(())
    }
    pub(crate) fn ensure_break(&mut self, index: usize) -> Result<(), LayoutError> {
        while self.records.len() < index {
            let page = self.resolve_blank(self.records.len(), true)?;
            self.records.push(Self::record(self.document, &page)?);
            self.boxes.push(page);
        }
        self.ensure(index)?;
        Ok(())
    }
}
