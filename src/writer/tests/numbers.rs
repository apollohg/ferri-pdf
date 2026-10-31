use crate::graphics::{DisplayItem, FlowPaint, Rect};
use crate::writer::numbers::{PDF_NUMBER_BOUND, unwritable_number};

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn frame(matrix: [f32; 6], items: Vec<DisplayItem>) -> DisplayItem {
    DisplayItem::Frame(Box::new(FlowPaint {
        matrix,
        area: Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        },
        items,
        lines: Vec::new(),
    }))
}

#[test]
fn items_are_writable_only_with_finite_numbers_within_the_bound() {
    let largest_writable = PDF_NUMBER_BOUND.next_down();
    for (entry, writable) in [
        (0.0, true),
        (-largest_writable, true),
        (largest_writable, true),
        (PDF_NUMBER_BOUND, false),
        (-PDF_NUMBER_BOUND, false),
        (f32::MAX, false),
        (f32::INFINITY, false),
        (f32::NEG_INFINITY, false),
        (f32::NAN, false),
    ] {
        let mut matrix = IDENTITY;
        matrix[4] = entry;
        let found = unwritable_number(&DisplayItem::PushTransform(matrix));
        assert_eq!(
            found.is_none(),
            writable,
            "a transform translating by {entry} is {} and found {found:?}",
            if writable { "writable" } else { "unwritable" }
        );
        let found = unwritable_number(&frame(matrix, Vec::new()));
        assert_eq!(
            found.is_none(),
            writable,
            "a frame placed by {entry} is {} and found {found:?}",
            if writable { "writable" } else { "unwritable" }
        );
    }
}

#[test]
fn nested_display_lists_are_left_to_their_own_items() {
    let nested = vec![
        DisplayItem::PushTransform([f32::INFINITY; 6]),
        DisplayItem::Pop,
    ];
    assert_eq!(
        unwritable_number(&frame(IDENTITY, nested.clone())),
        None,
        "a frame's own geometry is writable whatever the items it holds, which the writer \
         checks as it draws them"
    );
    assert!(
        unwritable_number(&nested[0]).is_some_and(f64::is_infinite),
        "the nested item itself is unwritable"
    );
}
