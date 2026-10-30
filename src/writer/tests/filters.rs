use crate::graphics::{Color, FilterFunction, Rect};
use crate::writer::filters::{function_filter, map_color};

/// Opaque and translucent samples covering both channel extremes and the middle.
const SAMPLES: [[u8; 4]; 8] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [255, 255, 255, 255],
    [0, 0, 0, 255],
    [200, 120, 40, 255],
    [30, 160, 220, 128],
    [250, 250, 10, 64],
];
/// One 8-bit step for rounding in either pipeline, plus one for premultiplied storage.
const TOLERANCE: i16 = 2;

/// Premultiplied 8-bit storage keeps a channel only to within 255/alpha levels.
fn tolerance(alpha: u8) -> i16 {
    TOLERANCE + i16::from(u8::MAX.div_ceil(alpha.max(1)))
}
/// Sample pixels are this many points wide at one pixel per point.
const AREA: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: SAMPLES.len() as f32,
    height: 1.0,
};

fn samples() -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(SAMPLES.len() as u32, 1).expect("sample pixmap");
    for (pixel, [red, green, blue, alpha]) in pixmap.pixels_mut().iter_mut().zip(SAMPLES) {
        *pixel = tiny_skia::ColorU8::from_rgba(red, green, blue, alpha).premultiply();
    }
    pixmap
}

fn demultiplied(pixmap: &tiny_skia::Pixmap) -> Vec<[u8; 4]> {
    pixmap
        .pixels()
        .iter()
        .map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect()
}

/// The spec's §13.1 markup for `function`, as an SVG `filter` element over the samples.
fn equivalent(function: &FilterFunction) -> String {
    let primitive = match *function {
        FilterFunction::Grayscale(amount) => {
            let a = 1.0 - amount.min(1.0);
            format!(
                "<feColorMatrix type='matrix' values='{} {} {} 0 0 {} {} {} 0 0 {} {} {} 0 0 0 0 0 1 0'/>",
                0.2126 + 0.7874 * a,
                0.7152 - 0.7152 * a,
                0.0722 - 0.0722 * a,
                0.2126 - 0.2126 * a,
                0.7152 + 0.2848 * a,
                0.0722 - 0.0722 * a,
                0.2126 - 0.2126 * a,
                0.7152 - 0.7152 * a,
                0.0722 + 0.9278 * a
            )
        }
        FilterFunction::Sepia(amount) => {
            let a = 1.0 - amount.min(1.0);
            format!(
                "<feColorMatrix type='matrix' values='{} {} {} 0 0 {} {} {} 0 0 {} {} {} 0 0 0 0 0 1 0'/>",
                0.393 + 0.607 * a,
                0.769 - 0.769 * a,
                0.189 - 0.189 * a,
                0.349 - 0.349 * a,
                0.686 + 0.314 * a,
                0.168 - 0.168 * a,
                0.272 - 0.272 * a,
                0.534 - 0.534 * a,
                0.131 + 0.869 * a
            )
        }
        FilterFunction::Saturate(amount) => {
            format!("<feColorMatrix type='saturate' values='{amount}'/>")
        }
        FilterFunction::HueRotate(angle) => {
            format!("<feColorMatrix type='hueRotate' values='{angle}'/>")
        }
        FilterFunction::Invert(amount) => {
            let a = amount.min(1.0);
            let table = format!("type='table' tableValues='{a} {}'", 1.0 - a);
            format!(
                "<feComponentTransfer><feFuncR {table}/><feFuncG {table}/><feFuncB {table}/></feComponentTransfer>"
            )
        }
        FilterFunction::Opacity(amount) => format!(
            "<feComponentTransfer><feFuncA type='table' tableValues='0 {}'/></feComponentTransfer>",
            amount.min(1.0)
        ),
        FilterFunction::Brightness(amount) => {
            let linear = format!("type='linear' slope='{amount}'");
            format!(
                "<feComponentTransfer><feFuncR {linear}/><feFuncG {linear}/><feFuncB {linear}/></feComponentTransfer>"
            )
        }
        FilterFunction::Contrast(amount) => {
            let linear = format!(
                "type='linear' slope='{amount}' intercept='{}'",
                -(0.5 * amount) + 0.5
            );
            format!(
                "<feComponentTransfer><feFuncR {linear}/><feFuncG {linear}/><feFuncB {linear}/></feComponentTransfer>"
            )
        }
        _ => unreachable!("only colour functions have matrix equivalents"),
    };
    // The samples are pixels of user space, so the region is given in user units.
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg'><filter id='f' filterUnits='userSpaceOnUse' \
         primitiveUnits='userSpaceOnUse' x='0' y='0' width='{}' height='1' \
         color-interpolation-filters='sRGB'>{primitive}</filter></svg>",
        SAMPLES.len()
    )
}

fn functions() -> Vec<FilterFunction> {
    vec![
        FilterFunction::Grayscale(1.0),
        FilterFunction::Grayscale(0.3),
        FilterFunction::Grayscale(2.0),
        FilterFunction::Sepia(1.0),
        FilterFunction::Sepia(0.4),
        FilterFunction::Saturate(0.25),
        FilterFunction::Saturate(2.5),
        FilterFunction::HueRotate(90.0),
        FilterFunction::HueRotate(-200.0),
        FilterFunction::Invert(1.0),
        FilterFunction::Invert(0.3),
        FilterFunction::Opacity(0.5),
        FilterFunction::Brightness(0.5),
        FilterFunction::Brightness(1.75),
        FilterFunction::Contrast(0.5),
        FilterFunction::Contrast(2.0),
    ]
}

#[test]
fn colour_functions_match_their_svg_filter_equivalents() {
    for function in functions() {
        let mut ours = samples();
        let filter = function_filter(&function, AREA).expect("function filter");
        crate::svg::resvg::filter::apply(
            &filter,
            tiny_skia::Transform::identity(),
            &mut ours,
            None,
        );
        let markup = equivalent(&function);
        // The referencing rectangle is in CSS pixels; userSpaceOnUse units ignore it.
        let tree = crate::svg::reference_tree(
            &markup,
            crate::images::LayerMaskTable::default(),
            "filter",
            "#f",
            Some(AREA),
            &|_| None,
        )
        .unwrap_or_else(|| panic!("{function:?}: spec markup must parse: {markup}"));
        let spec = crate::svg::first_filter(tree.root())
            .unwrap_or_else(|| panic!("{function:?}: spec markup must give a filter: {markup}"));
        let mut theirs = samples();
        crate::svg::resvg::filter::apply(
            &spec,
            tiny_skia::Transform::identity(),
            &mut theirs,
            None,
        );
        for ((input, ours), theirs) in SAMPLES
            .iter()
            .zip(demultiplied(&ours))
            .zip(demultiplied(&theirs))
        {
            assert!(
                ours.iter()
                    .zip(theirs)
                    .all(|(a, b)| (i16::from(*a) - i16::from(b)).abs() <= tolerance(input[3])),
                "{function:?} on {input:?}: matrix gives {ours:?}, the §13.1 markup {markup} gives {theirs:?}"
            );
        }
    }
}

#[test]
fn only_range_preserving_colour_chains_stay_vector_and_match_the_raster_result() {
    let vector = [
        FilterFunction::Grayscale(1.0),
        FilterFunction::Grayscale(0.3),
        // Amounts above one clamp to one (§6.1).
        FilterFunction::Grayscale(2.0),
        FilterFunction::Saturate(0.25),
        FilterFunction::Invert(1.0),
        FilterFunction::Invert(0.3),
        FilterFunction::Opacity(0.5),
        FilterFunction::Brightness(0.5),
        FilterFunction::Contrast(0.5),
        FilterFunction::HueRotate(0.0),
    ];
    for function in functions() {
        let chain = crate::writer::filters::color_chain(&[crate::writer::filters::Step::Function(
            &function,
        )]);
        assert_eq!(
            chain.is_some(),
            vector.contains(&function),
            "{function:?}: only matrices keeping every channel in 0..=1 commute with compositing \
             and may recolour paints instead of rasterizing"
        );
        let Some((matrix, alpha)) = chain else {
            continue;
        };
        let mut raster = samples();
        crate::svg::resvg::filter::apply(
            &function_filter(&function, AREA).expect("function filter"),
            tiny_skia::Transform::identity(),
            &mut raster,
            None,
        );
        for (input, filtered) in SAMPLES.iter().zip(demultiplied(&raster)) {
            let [red, green, blue, opacity] = *input;
            let mapped = map_color(
                &matrix,
                Color {
                    rgb: [red, green, blue],
                    alpha: f32::from(opacity) / 255.0,
                },
            );
            let recoloured = [
                mapped.rgb[0],
                mapped.rgb[1],
                mapped.rgb[2],
                (mapped.alpha * alpha * 255.0).round() as u8,
            ];
            assert!(
                recoloured
                    .iter()
                    .zip(filtered)
                    .all(|(a, b)| (i16::from(*a) - i16::from(b)).abs() <= tolerance(opacity)),
                "{function:?} on {input:?}: recoloured paint {recoloured:?} differs from the \
                 filtered pixel {filtered:?}"
            );
        }
    }
}

#[test]
fn colour_chains_compose_in_function_order() {
    let invert = FilterFunction::Invert(1.0);
    let half = FilterFunction::Brightness(0.5);
    let (matrix, _) = crate::writer::filters::color_chain(&[
        crate::writer::filters::Step::Function(&invert),
        crate::writer::filters::Step::Function(&half),
    ])
    .expect("in-range chain");
    let white = map_color(
        &matrix,
        Color {
            rgb: [255; 3],
            alpha: 1.0,
        },
    );
    assert_eq!(
        white.rgb, [0; 3],
        "invert() then brightness(0.5) turns white black; the reverse order would give 128"
    );
    let black = map_color(&matrix, Color::BLACK);
    assert_eq!(
        black.rgb, [128; 3],
        "invert() then brightness(0.5) halves the inverted black"
    );
}
