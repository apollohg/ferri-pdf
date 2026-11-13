//! A box, its paragraph and the text spans its element's text forms share the style the
//! element computes rather than each holding a copy of it.
use crate::document::{BlockBox, BlockContent, StyledDocument};
use crate::resources::ResourceBase;
use crate::style::{Display, Style};
use std::{collections::HashSet, sync::Arc};

const ROWS: usize = 40;
const COLUMNS: usize = 5;

fn cells<'a>(block: &'a BlockBox, output: &mut Vec<&'a BlockBox>) {
    if block.style().display == Display::TableCell {
        output.push(block);
        return;
    }
    for child in block.children() {
        if let BlockContent::Block(child) | BlockContent::Positioned(child) = child {
            cells(child, output);
        }
    }
}

#[test]
fn table_cells_share_their_style_with_their_paragraphs_and_text_spans() {
    let rows: String = (0..ROWS)
        .map(|row| {
            let cells: String = (0..COLUMNS)
                .map(|column| format!("<td>r{row} c{column} text</td>"))
                .collect();
            format!("<tr>{cells}</tr>")
        })
        .collect();
    let html = format!(
        "<style>td{{border:1px solid;padding:2px}}</style><table><tbody>{rows}</tbody></table>"
    );
    let document = StyledDocument::parse(
        &html,
        &ResourceBase::default(),
        &[],
        &crate::tests::common::fixture_resolver(),
    )
    .unwrap();
    let mut found = Vec::new();
    cells(document.root(), &mut found);
    assert_eq!(found.len(), ROWS * COLUMNS, "every cell formats a box");
    let mut styles: HashSet<*const Style> = HashSet::new();
    for (index, cell) in found.iter().enumerate() {
        let [BlockContent::Inline(paragraph)] = cell.children() else {
            panic!("cell {index} must hold one paragraph of its text");
        };
        let shared = std::ptr::from_ref(cell.style());
        let paragraph_style = Arc::as_ptr(paragraph.style());
        styles.insert(shared);
        styles.insert(paragraph_style);
        styles.extend(
            paragraph
                .spans()
                .iter()
                .map(|span| Arc::as_ptr(&span.style)),
        );
        assert!(
            std::ptr::eq(paragraph_style, shared),
            "cell {index} ({:?}): its paragraph copies the cell's style instead of sharing it",
            paragraph.text()
        );
        for span in paragraph.spans() {
            assert!(
                std::ptr::eq(Arc::as_ptr(&span.style), shared),
                "cell {index} ({:?}): the span {:?} copies the cell's style instead of sharing it",
                paragraph.text(),
                &paragraph.text()[span.range.clone()]
            );
        }
    }
    assert_eq!(
        styles.len(),
        found.len(),
        "the cells, their paragraphs and their spans must hold one style per cell"
    );
}
