mod emphasis;
mod features;
mod hyphenation;
mod tailoring;

use crate::document::Paragraph;
use crate::style::Style;

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
