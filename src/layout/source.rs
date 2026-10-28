use crate::document::{BlockBox, BlockContent, Paragraph};
use std::collections::HashMap;

/// Identities of the immutable formatting tree, independent of measurement probes.
pub(super) struct SourceIds {
    blocks: HashMap<*const BlockBox, usize>,
    paragraphs: HashMap<*const Paragraph, usize>,
}
impl SourceIds {
    pub(crate) fn new(root: &BlockBox) -> Self {
        enum Entry<'a> {
            Block(&'a BlockBox),
            Paragraph(&'a Paragraph),
        }
        let mut ids = Self {
            blocks: HashMap::new(),
            paragraphs: HashMap::new(),
        };
        let mut pending = vec![Entry::Block(root)];
        while let Some(entry) = pending.pop() {
            match entry {
                Entry::Block(block) => {
                    let index = ids.blocks.len();
                    assert!(
                        ids.blocks.insert(block, index).is_none(),
                        "formatting boxes have one owner"
                    );
                    pending.extend(
                        block
                            .children()
                            .iter()
                            .rev()
                            .filter_map(|child| match child {
                                BlockContent::Block(block) | BlockContent::Positioned(block) => {
                                    Some(Entry::Block(block))
                                }
                                BlockContent::Inline(paragraph) => {
                                    Some(Entry::Paragraph(paragraph))
                                }
                                BlockContent::Events(_) => None,
                            }),
                    );
                    if let Some(marker) = block.marker() {
                        pending.push(Entry::Paragraph(marker));
                    }
                }
                Entry::Paragraph(paragraph) => {
                    let index = ids.paragraphs.len();
                    assert!(
                        ids.paragraphs.insert(paragraph, index).is_none(),
                        "formatting paragraphs have one owner"
                    );
                    let mut anchors = paragraph
                        .anchors()
                        .iter()
                        .chain(paragraph.atomics())
                        .collect::<Vec<_>>();
                    anchors.sort_by_key(|anchor| anchor.offset);
                    pending.extend(
                        anchors
                            .into_iter()
                            .rev()
                            .map(|anchor| Entry::Block(&anchor.block)),
                    );
                }
            }
        }
        ids
    }
    pub(crate) fn block(&self, block: &BlockBox) -> usize {
        self.blocks[&std::ptr::from_ref(block)]
    }
    pub(crate) fn paragraph(&self, paragraph: &Paragraph) -> usize {
        self.paragraphs[&std::ptr::from_ref(paragraph)]
    }
    pub(crate) fn paragraph_count(&self) -> usize {
        self.paragraphs.len()
    }
}
