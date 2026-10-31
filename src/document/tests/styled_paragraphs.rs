//! Paragraphs built from styled spans inside an explicit tree of inline boxes.
use crate::document::Paragraph;
use crate::style::Style;
use crate::work::{self, Work};
use std::{ops::Range, sync::Arc};

type BoxEntry = (Range<usize>, Arc<Style>, Option<usize>);

fn styled(text: &str, parents: &[Option<usize>]) -> Paragraph {
    let style = Arc::new(Style::default());
    let boxes: Vec<BoxEntry> = parents
        .iter()
        .map(|parent| (0..text.len(), style.clone(), *parent))
        .collect();
    Paragraph::styled(
        text.to_owned(),
        &Style::default(),
        boxes,
        [(
            0..text.len(),
            Style::default(),
            parents.len().checked_sub(1),
        )],
    )
}

fn descendants(paragraph: &Paragraph) -> Vec<usize> {
    paragraph
        .boxes()
        .iter()
        .map(|inline| inline.descendants)
        .collect()
}

#[test]
fn styled_boxes_count_the_subtrees_that_follow_them() {
    // 0 ┬ 1 ─ 2
    //   └ 3
    // 4 ─ 5
    let parents = [None, Some(0), Some(1), Some(0), None, Some(4)];
    assert_eq!(
        descendants(&styled("x", &parents)),
        [3, 1, 0, 0, 1, 0],
        "each box must count exactly the boxes of its subtree for parents {parents:?}"
    );
}

#[test]
fn styled_box_subtrees_are_counted_in_linear_time() {
    // Counting each box's subtree by walking every later box's parent chain follows links
    // cubic in the depth of a chain of boxes.
    const CHAIN: usize = 64;
    let chain = |length: usize| {
        let parents: Vec<_> = (0..length).map(|index| index.checked_sub(1)).collect();
        work::measured(Work::BoxAncestor, || descendants(&styled("x", &parents)))
    };
    work::assert_linear("a chain of nested boxes", CHAIN, |length| chain(length).1);
    let expected: Vec<_> = (0..CHAIN).rev().collect();
    assert_eq!(
        chain(CHAIN).0,
        expected,
        "a chain's boxes contain every later box"
    );
}
