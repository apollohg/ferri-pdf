use crate::writer::backdrops::{mirror, over_color};

/// Premultiplied samples: transparent, opaque extremes, translucent and nearly opaque.
const COLOURS: [[f32; 4]; 8] = [
    [0.0, 0.0, 0.0, 0.0],
    [1.0, 1.0, 1.0, 1.0],
    [0.0, 0.0, 0.0, 1.0],
    [0.4, 0.2, 0.0, 1.0],
    [0.25, 0.0, 0.25, 0.5],
    [0.1, 0.3, 0.05, 0.3],
    [0.6, 0.1, 0.9, 0.95],
    [0.02, 0.0, 0.01, 0.02],
];
const EPSILON: f32 = 1.0e-5;

/// Compositing 2 §9.1.15 applied to every premultiplied channel.
fn plus_lighter(source: [f32; 4], backdrop: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|channel| (source[channel] + backdrop[channel]).min(1.0))
}

/// Compositing 1 §9.1.4 source-over on premultiplied colours.
fn source_over(source: [f32; 4], backdrop: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|channel| source[channel] + backdrop[channel] * (1.0 - source[3]))
}

#[test]
fn drawing_the_over_colour_onto_the_backdrop_gives_plus_lighter() {
    for source in COLOURS {
        for backdrop in COLOURS {
            let over = over_color(source, backdrop);
            assert!(
                (0.0..=1.0).contains(&over[3])
                    && over[..3].iter().all(|c| (0.0..=over[3]).contains(c)),
                "{over:?} drawn for source {source:?} on {backdrop:?} must be a premultiplied colour"
            );
            let drawn = source_over(over, backdrop);
            let expected = plus_lighter(source, backdrop);
            assert!(
                drawn
                    .iter()
                    .zip(expected)
                    .all(|(a, e)| (a - e).abs() < EPSILON),
                "source {source:?} on {backdrop:?}: drawing {over:?} gives {drawn:?}, \
                 plus-lighter is {expected:?}"
            );
        }
    }
}

#[test]
fn mirroring_reflects_pixel_centres_across_each_edge_and_repeats() {
    let cases = [
        // Inside stays put.
        (4.5, 4.5),
        // Half a pixel past either edge lands half a pixel inside it.
        (-0.5, 0.5),
        (10.5, 9.5),
        // Beyond a whole length the reflection reflects again.
        (-10.5, 9.5),
        (21.5, 1.5),
        (-25.0, 5.0),
    ];
    for (value, expected) in cases {
        let mirrored = mirror(value, 0.0, 10.0);
        assert!(
            (mirrored - expected).abs() < EPSILON,
            "{value} mirrored into 0..10 is {mirrored}, expected {expected}"
        );
    }
    assert!(
        (mirror(48.0, 50.0, 200.0) - 52.0).abs() < EPSILON,
        "an offset box mirrors about its own edge"
    );
}
