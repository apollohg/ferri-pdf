use super::offset;
use crate::graphics::Rect;
use crate::style::{LengthPercentage, Size, Style};

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn sticky(insets: [Option<LengthPercentage>; 4]) -> Style {
    let mut style = Style::default();
    style.position = crate::style::Position::Sticky;
    style.inset = insets.map(|inset| inset.map_or(Size::Auto, Size::Value));
    style
}

fn length(value: f32) -> LengthPercentage {
    LengthPercentage::Length(value)
}

/// CSS Position 3 §3.4: a top inset pushes the box down to the sticky view rectangle, but
/// never lets its margin box leave the containing block.
#[test]
fn top_inset_pushes_down_until_the_containing_block_ends() {
    let scrollport = rect(0.0, 0.0, 400.0, 600.0);
    let style = sticky([Some(length(30.0)), None, None, None]);
    for (border_top, containing_bottom, margin_bottom, expected) in [
        (10.0, 500.0, 0.0, 20.0),
        (40.0, 500.0, 0.0, 0.0),
        (10.0, 70.0, 0.0, 10.0),
        (10.0, 70.0, 5.0, 5.0),
        (10.0, 55.0, 0.0, 0.0),
    ] {
        let border = rect(0.0, border_top, 100.0, 50.0);
        let containing = rect(0.0, 0.0, 400.0, containing_bottom);
        let [x, y] = offset(
            &style,
            border,
            [0.0, 0.0, margin_bottom, 0.0],
            containing,
            scrollport,
            false,
        );
        assert_eq!(
            [x, y],
            [0.0, expected],
            "border top {border_top}, containing bottom {containing_bottom}, margin bottom \
             {margin_bottom}"
        );
    }
}

/// A bottom inset only ever pulls the box up, within the containing block's start.
#[test]
fn bottom_inset_pulls_up_within_the_containing_block() {
    let scrollport = rect(0.0, 0.0, 400.0, 600.0);
    let style = sticky([None, None, Some(length(20.0)), None]);
    let containing = rect(0.0, 500.0, 400.0, 300.0);
    let border = rect(0.0, 560.0, 100.0, 40.0);
    assert_eq!(
        offset(&style, border, [0.0; 4], containing, scrollport, false),
        [0.0, -20.0],
        "the box's bottom must meet the view rectangle 20 above the scrollport bottom"
    );
    let border = rect(0.0, 510.0, 100.0, 120.0);
    assert_eq!(
        offset(&style, border, [0.0; 4], containing, scrollport, false),
        [0.0, -10.0],
        "the shift stops at the containing block's top edge"
    );
}

/// The end-edge inset gives way when the view rectangle is smaller than the border box, so
/// a box taller than the scrollport is only constrained by its start inset.
#[test]
fn end_inset_gives_way_for_a_large_box() {
    let scrollport = rect(0.0, 0.0, 400.0, 100.0);
    let style = sticky([Some(length(20.0)), None, Some(length(20.0)), None]);
    let border = rect(0.0, 0.0, 100.0, 200.0);
    let containing = rect(0.0, 0.0, 400.0, 1000.0);
    assert_eq!(
        offset(&style, border, [0.0; 4], containing, scrollport, false),
        [0.0, 20.0],
        "the reduced bottom inset leaves the top inset in force"
    );
}

/// Percentage insets refer to the scrollport, and in a right-to-left containing block the
/// inline end edge that gives way is the left one.
#[test]
fn horizontal_insets_use_the_scrollport_and_the_containing_direction() {
    let scrollport = rect(100.0, 0.0, 200.0, 600.0);
    let containing = rect(0.0, 0.0, 1000.0, 600.0);
    let style = sticky([None, None, None, Some(LengthPercentage::Percentage(0.1))]);
    assert_eq!(
        offset(
            &style,
            rect(110.0, 0.0, 50.0, 10.0),
            [0.0; 4],
            containing,
            scrollport,
            false
        ),
        [10.0, 0.0],
        "10% of the 200-wide scrollport keeps the box 20 inside its left edge"
    );
    let style = sticky([None, Some(length(10.0)), None, Some(length(10.0))]);
    let wide = rect(50.0, 0.0, 400.0, 10.0);
    assert_eq!(
        offset(&style, wide, [0.0; 4], containing, scrollport, true),
        [-50.0, 0.0],
        "in a right-to-left block the left inset gives way and the right one pulls the box \
         left until its margin box meets the containing block's left edge"
    );
    assert_eq!(
        offset(&style, wide, [0.0; 4], containing, scrollport, false),
        [60.0, 0.0],
        "in a left-to-right block the right inset gives way and the left one pushes"
    );
}
