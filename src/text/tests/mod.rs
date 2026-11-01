mod emphasis;
mod features;
mod hyphenation;
mod pieces;
mod skip_ink;
mod tailoring;

use crate::document::{BlockBox, BlockContent, Paragraph, StyledDocument};
use crate::resources::{FileResolver, ResourceBase};
use crate::style::Style;

/// `body` styled as a document.
fn document(body: &str) -> StyledDocument {
    StyledDocument::parse(
        body,
        &ResourceBase::default(),
        &[],
        &FileResolver::default(),
    )
    .expect("parse document")
}

/// The first paragraph of `block` in tree order.
fn first_paragraph(block: &BlockBox) -> Option<&Paragraph> {
    block.children().iter().find_map(|child| match child {
        BlockContent::Inline(paragraph) => Some(paragraph),
        BlockContent::Block(block) => first_paragraph(block),
        _ => None,
    })
}

/// A generated paragraph of `text` in `style`.
fn paragraph(text: &str, style: &Style) -> Paragraph {
    Paragraph::generated(text, style)
        .expect("generate paragraph")
        .expect("text forms a paragraph")
}

/// The text between consecutive soft wrap opportunities, joined by `|` for readable failures.
fn segments(paragraph: &Paragraph, offsets: &[usize]) -> String {
    let text = paragraph.text();
    let mut start = 0;
    let mut parts = Vec::new();
    for &offset in offsets.iter().chain(std::iter::once(&text.len())) {
        if offset > start {
            parts.push(&text[start..offset]);
            start = offset;
        }
    }
    parts.join("|")
}
