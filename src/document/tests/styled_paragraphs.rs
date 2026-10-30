//! Paragraphs built from styled spans inside an explicit tree of inline boxes.
use crate::document::Paragraph;
use crate::style::Style;
use std::{ops::Range, sync::Arc, time::Duration};

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
    // Counting each box's subtree by walking every later box's parent chain takes cubic time
    // in a chain this deep: minutes, against milliseconds for one reverse pass.
    const CHAIN: usize = 4096;
    const BOUND: Duration = Duration::from_secs(10);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let parents: Vec<_> = (0..CHAIN).map(|index| index.checked_sub(1)).collect();
        let paragraph = styled("x", &parents);
        let _ = sender.send(descendants(&paragraph));
    });
    let counts = receiver
        .recv_timeout(BOUND)
        .unwrap_or_else(|error| panic!("{CHAIN} nested boxes not built within {BOUND:?}: {error}"));
    let expected: Vec<_> = (0..CHAIN).rev().collect();
    assert_eq!(counts, expected, "a chain's boxes contain every later box");
}
