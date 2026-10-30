use crate::text::skip_ink::{band_extents, subtract};

#[test]
fn a_ring_projects_its_band_crossing_including_the_filled_sides() {
    // A square ring 0..10 with a 4..6 hole, outer counter-clockwise and inner clockwise.
    let contours = vec![
        vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
        vec![[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0]],
    ];
    let mut extents = band_extents(&contours, [4.5, 5.5]);
    extents.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let covered = |x: f32| extents.iter().any(|[l, r]| *l <= x && x <= *r);
    for (x, ink) in [
        (1.0, true),
        (3.9, true),
        (5.0, false),
        (6.1, true),
        (9.9, true),
    ] {
        assert_eq!(covered(x), ink, "x {x} in band extents {extents:?}");
    }
}

#[test]
fn a_shape_wholly_inside_the_band_projects_its_full_width() {
    let contours = vec![vec![[2.0, 1.0], [8.0, 1.0], [5.0, 2.0]]];
    let extents = band_extents(&contours, [0.0, 3.0]);
    let left = extents.iter().map(|e| e[0]).fold(f32::INFINITY, f32::min);
    let right = extents
        .iter()
        .map(|e| e[1])
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (left - 2.0).abs() < 1e-4 && (right - 8.0).abs() < 1e-4,
        "extents {extents:?}"
    );
    assert_eq!(band_extents(&contours, [3.0, 4.0]), Vec::<[f32; 2]>::new());
}

#[test]
fn subtraction_leaves_the_line_outside_the_cleared_ink() {
    assert_eq!(
        subtract([0.0, 20.0], &[[5.0, 6.0], [12.0, 13.0]], 1.0),
        vec![[0.0, 4.0], [7.0, 11.0], [14.0, 20.0]]
    );
    assert_eq!(
        subtract([0.0, 20.0], &[[-5.0, 25.0]], 1.0),
        Vec::<[f32; 2]>::new()
    );
}
