//! Intrinsic widths kept for measuring the same content again.
use super::{BlockSource, Measurer};
use crate::layout::LayoutError;
use crate::layout::measure::Failure;
use crate::text::IntrinsicWidths;
use std::collections::HashMap;

/// Intrinsic widths kept at most; they are all forgotten when there would be more.
const MAX_KEPT_WIDTHS: usize = 1 << 16;

/// Intrinsic widths measured, shared by every probe, as nested shrink-to-fit boxes and flex
/// and grid items measure their contents again at each level.
#[derive(Default)]
pub(in crate::layout) struct KeptWidths {
    /// Paragraphs' widths, by the paragraph's address and the widths of its atomic inlines
    /// and of their `::first-line` boxes.
    paragraphs: HashMap<(usize, Vec<u32>), IntrinsicWidths>,
    /// Boxes' content widths, by the box's address and selected style, where they depend on
    /// neither the available width nor the measurer's context.
    boxes: HashMap<(usize, usize), IntrinsicWidths>,
    /// The content widths being computed depend on the available width or on the measurer's
    /// context.
    contextual: bool,
}

impl KeptWidths {
    fn make_room(&mut self) {
        if self.paragraphs.len() + self.boxes.len() >= MAX_KEPT_WIDTHS {
            self.paragraphs.clear();
            self.boxes.clear();
        }
    }
}

fn box_key(block: BlockSource<'_>) -> (usize, usize) {
    (
        std::ptr::from_ref(block.block) as usize,
        std::ptr::from_ref(block.style()) as usize,
    )
}

impl Measurer<'_> {
    pub(crate) fn intrinsic_widths(
        &self,
        paragraph: &crate::document::Paragraph,
        atomics: &[IntrinsicWidths],
        first_line_atomics: &[IntrinsicWidths],
    ) -> Result<IntrinsicWidths, LayoutError> {
        let key = (
            std::ptr::from_ref(paragraph) as usize,
            atomics
                .iter()
                .chain(first_line_atomics)
                .flat_map(|widths| [widths.min.to_bits(), widths.max.to_bits()])
                .collect(),
        );
        if let Some(widths) = self.kept_widths.borrow().paragraphs.get(&key) {
            self.work.spend(1)?;
            return Ok(*widths);
        }
        self.work.spend(paragraph.text().len().max(1))?;
        let widths = self
            .text
            .intrinsic_widths(paragraph, atomics, first_line_atomics)?;
        let mut kept = self.kept_widths.borrow_mut();
        kept.make_room();
        kept.paragraphs.insert(key, widths);
        Ok(widths)
    }

    /// The content widths of `block` computed before, which depend on neither the available
    /// width nor the measurer's context. Kept off the frames that nested content stacks.
    #[inline(never)]
    pub(in crate::layout) fn kept_content_widths(
        &self,
        block: BlockSource<'_>,
    ) -> Option<IntrinsicWidths> {
        self.kept_widths
            .borrow()
            .boxes
            .get(&box_key(block))
            .copied()
    }

    /// Starts computing a box's content widths, returning whether those of the box it is in
    /// depended on the available width or the context so far.
    #[inline(never)]
    pub(in crate::layout) fn start_content_widths(&self) -> bool {
        crate::work::count(crate::work::Work::ContentSizes, 1);
        std::mem::take(&mut self.kept_widths.borrow_mut().contextual)
    }

    /// Keeps the content widths `widths` computed for `block`, unless they depend on the
    /// available width or the context, and returns to computing those of the box it is in,
    /// whose dependence so far was `outer`.
    #[inline(never)]
    pub(in crate::layout) fn keep_content_widths(
        &self,
        block: BlockSource<'_>,
        outer: bool,
        widths: &Result<IntrinsicWidths, Failure>,
    ) {
        let mut kept = self.kept_widths.borrow_mut();
        if kept.contextual {
            return;
        }
        kept.contextual = outer;
        if let Ok(widths) = widths {
            kept.make_room();
            kept.boxes.insert(box_key(block), *widths);
        }
    }

    /// Records that the content widths being computed depend on the available width or on
    /// the measurer's context, so that they are not kept.
    pub(in crate::layout) fn contextual_widths(&self) {
        self.kept_widths.borrow_mut().contextual = true;
    }
}
