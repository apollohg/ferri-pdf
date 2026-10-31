use crate::document::{BlockBox, BlockContent, Paragraph};
use std::collections::{HashMap, HashSet};

/// Identities of the immutable formatting tree's paragraphs, independent of measurement probes.
#[derive(Clone)]
pub(super) struct SourceIds {
    paragraphs: HashMap<*const Paragraph, usize>,
    /// The first-line boxes registered, which the boxes they restyle alike share.
    shared: HashSet<*const BlockBox>,
}
impl SourceIds {
    pub(crate) fn new(root: &BlockBox) -> Self {
        let mut ids = Self {
            paragraphs: HashMap::new(),
            shared: HashSet::new(),
        };
        ids.register(root);
        ids
    }
    /// Adds a formatting tree outside the document root's, such as a running element's.
    pub(crate) fn register(&mut self, root: &BlockBox) {
        enum Entry<'a> {
            Block(&'a BlockBox),
            /// A first-line box, which the boxes it restyles alike share.
            Shared(&'a BlockBox),
            Paragraph(&'a Paragraph),
        }
        let ids = self;
        let mut pending = vec![Entry::Block(root)];
        while let Some(entry) = pending.pop() {
            match entry {
                Entry::Shared(block) if !ids.shared.insert(std::ptr::from_ref(block)) => {}
                Entry::Block(block) | Entry::Shared(block) => {
                    if let Some(flow) = block.flow() {
                        pending.push(Entry::Block(flow));
                    }
                    if let Some(content) = block.math_content() {
                        pending.push(Entry::Block(content));
                    }
                    if let Some(copy) = block.first_line_box() {
                        pending.push(Entry::Shared(copy));
                    }
                    if let Some(radical) = block.math().and_then(|math| math.radical.as_ref()) {
                        pending.push(Entry::Paragraph(radical));
                    }
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
                        .chain(paragraph.inline_blocks())
                        .collect::<Vec<_>>();
                    anchors.sort_by_key(|anchor| anchor.offset);
                    pending.extend(anchors.into_iter().rev().map(|anchor| {
                        if anchor.shared() {
                            Entry::Shared(anchor.block())
                        } else {
                            Entry::Block(anchor.block())
                        }
                    }));
                }
            }
        }
    }
    pub(crate) fn paragraph(&self, paragraph: &Paragraph) -> usize {
        self.paragraphs[&std::ptr::from_ref(paragraph)]
    }
    pub(crate) fn paragraph_count(&self) -> usize {
        self.paragraphs.len()
    }
}
