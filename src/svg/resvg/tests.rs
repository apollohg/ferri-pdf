const SIZE: u32 = 100;
const CENTER: f32 = 50.0;
const RADIUS: f32 = 30.0;
const TOLERANCE: u8 = 8;

/// The colour the raster renderer paints at `angle` degrees about the centre of a square filled
/// with a sweep from red to blue over `start` to `end` degrees, padded.
fn sweep_colour(start: f32, end: f32, angle: f32) -> [u8; 3] {
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{SIZE}' height='{SIZE}'>\
         <{element} id='s' cx='{CENTER}' cy='{CENTER}' {start_name}='{start}' \
         {end_name}='{end}' gradientUnits='userSpaceOnUse'><stop offset='0' stop-color='red'/>\
         <stop offset='1' stop-color='blue'/></{element}>\
         <rect width='{SIZE}' height='{SIZE}' fill='url(#s)'/></svg>",
        element = crate::svg::SWEEP_GRADIENT_ELEMENT,
        start_name = crate::svg::SWEEP_START_ANGLE_ATTRIBUTE,
        end_name = crate::svg::SWEEP_END_ANGLE_ATTRIBUTE,
    );
    let tree = crate::svg::usvg::Tree::from_str(&svg, &crate::svg::usvg::Options::default())
        .expect("the sweep fixture parses");
    let mut pixmap = tiny_skia::Pixmap::new(SIZE, SIZE).unwrap();
    super::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
        None,
    );
    let (sin, cos) = angle.to_radians().sin_cos();
    let pixel = pixmap
        .pixel(
            (CENTER + cos * RADIUS) as u32,
            (CENTER + sin * RADIUS) as u32,
        )
        .unwrap();
    [pixel.red(), pixel.green(), pixel.blue()]
}

#[test]
fn raster_sweeps_map_every_angular_range_onto_their_colour_line() {
    // Ranges starting at zero or spanning a whole turn are those tiny-skia's stage selection
    // would leave unscaled or unrotated.
    for (start, end) in [(0.0, 90.0), (90.0, 450.0), (45.0, 135.0), (90.0, 0.0)] {
        // A point's angle lies within one turn from the positive x axis.
        for angle in [30.0_f32, 60.0, 100.0, 200.0, 300.0] {
            let t = ((angle - start) / (end - start)).clamp(0.0, 1.0);
            let expected = [
                (255.0 * (1.0 - t)).round() as u8,
                0,
                (255.0 * t).round() as u8,
            ];
            let actual = sweep_colour(start, end, angle);
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, e)| a.abs_diff(e) <= TOLERANCE),
                "a sweep over {start}..{end} degrees paints {actual:?} at {angle} degrees, \
                 expected {expected:?}"
            );
        }
    }
}
