use super::{redistribute, selected_position};

const PORT: f32 = 100.0;

#[test]
fn positions_crowding_the_scroll_start_spread_over_its_first_eighth() {
    let mut positions = vec![-40.0, -10.0, 50.0, 400.0];
    redistribute(&mut positions, PORT, 200.0);
    let distribute = PORT / 8.0;
    assert_eq!(
        positions[0], 0.0,
        "the smallest position before the distribution range moves to the scroll start"
    );
    assert!(
        (positions[1] - 30.0 / (distribute + 40.0) * distribute).abs() < 1e-4,
        "the others before it spread linearly over the first eighth of the scrollport: \
         {positions:?}"
    );
    assert_eq!(positions[2], 50.0, "reachable positions stay");
    assert_eq!(
        positions[3], 200.0,
        "the largest unreachable position moves to the end of the scroll range"
    );
    assert_eq!(
        selected_position(&positions, PORT),
        0.0,
        "after redistribution only the first target lies at the initial scroll position"
    );
    assert_eq!(
        selected_position(&[-40.0, -10.0, 50.0], PORT),
        -10.0,
        "without a scroll range nothing moves, and the last position at or before the initial \
         scroll position is selected"
    );
}

#[test]
fn a_target_less_than_half_a_scrollport_ahead_is_selected_after_a_distant_one() {
    assert_eq!(
        selected_position(&[-100.0, 30.0], PORT),
        30.0,
        "the position before it lies more than half a scrollport back, and it lies less than \
         half a scrollport ahead"
    );
    assert_eq!(
        selected_position(&[-30.0, 30.0], PORT),
        -30.0,
        "a position before it within half a scrollport keeps the selection"
    );
    assert_eq!(
        selected_position(&[60.0, 70.0], PORT),
        60.0,
        "with no position at or before the scroll position, the first is selected"
    );
}
