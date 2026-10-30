mod navigation;
mod ruby;
mod scroll_markers;
mod shadow_fonts;
mod text_transform;

use super::*;
use crate::resources::FileResolver;

fn paragraphs<'a>(block: &'a BlockBox, output: &mut Vec<&'a Paragraph>) {
    for child in block.children() {
        match child {
            BlockContent::Events(_) => {}
            BlockContent::Block(child) | BlockContent::Positioned(child) => {
                paragraphs(child, output);
            }
            BlockContent::Inline(paragraph) => output.push(paragraph),
        }
    }
}

#[test]
fn html_newline_normalization_does_not_reinterpret_preserved_character_references() {
    let document = StyledDocument::parse(
        "<pre>A&#13;B\rC</pre>",
        &crate::resources::ResourceBase::default(),
        &[],
        &FileResolver::default(),
    )
    .unwrap();
    let mut content = Vec::new();
    paragraphs(document.root(), &mut content);
    assert_eq!(content.len(), 1);
    assert_eq!(
        content[0].text(),
        "A\rB\nC",
        "literal CR is normalized by HTML parsing; an entity-produced control remains a font-coverage decision, not an invented forced line break"
    );
}
