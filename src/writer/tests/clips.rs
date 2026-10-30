use crate::graphics::{Clip, Contour, Rect};
use crate::writer::painting;
use std::sync::Arc;

const SURFACE: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 80.0,
    height: 60.0,
};

fn render(clip: &Clip) -> Vec<u8> {
    let mut document = krilla::Document::new();
    let mut page = document.start_page_with(
        krilla::page::PageSettings::from_wh(SURFACE.width, SURFACE.height).unwrap(),
    );
    let mut surface = page.surface();
    let count = painting::push_clip(clip, SURFACE, &mut surface)
        .expect("a valid disconnected or empty outline must serialize");
    surface.draw_path(&painting::rect_path(SURFACE).unwrap());
    painting::pop_clips(count, &mut surface);
    surface.finish();
    page.finish();
    let pdf = hayro::hayro_syntax::Pdf::new(document.finish().unwrap()).unwrap();
    let pixels = hayro::render(
        &pdf.pages()[0],
        &hayro::RenderCache::new(),
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &hayro::RenderSettings::default(),
        &hayro::PixmapSettings {
            x_scale: 1.0,
            y_scale: 1.0,
            bg_color: hayro::vello_cpu::color::palette::css::TRANSPARENT,
        },
    );
    assert_eq!([pixels.width(), pixels.height()], [80, 60]);
    pixels.data_as_u8_slice().to_vec()
}

#[test]
fn pdf_clips_disconnected_islands_and_their_fragment_edges() {
    let clip = Clip {
        rect: Rect {
            y: 10.0,
            height: 40.0,
            ..SURFACE
        },
        radii: Default::default(),
        outline: Some(Arc::new(vec![
            Contour::polygon(&[[40.0, 0.0], [80.0, 0.0], [80.0, 30.0], [40.0, 30.0]]),
            Contour::polygon(&[[0.0, 30.0], [40.0, 30.0], [40.0, 60.0], [0.0, 60.0]]),
        ])),
        reference_box: Some(SURFACE),
        axes: [true; 2],
    };
    let pixels = render(&clip);
    for y in 0..60 {
        for x in 0..80 {
            let expected = if (10..50).contains(&y) && ((x >= 40 && y < 30) || (x < 40 && y >= 30))
            {
                u8::MAX
            } else {
                0
            };
            assert_eq!(
                pixels[(y * 80 + x) * 4 + 3],
                expected,
                "pixel ({x},{y}): union of the two islands intersected with the fragment"
            );
        }
    }
}

#[test]
fn pdf_empty_outline_clips_all_ink_but_an_open_axis_remains_a_strip() {
    let mut clip = Clip {
        rect: Rect {
            y: 10.0,
            height: 40.0,
            ..SURFACE
        },
        radii: Default::default(),
        outline: Some(Arc::new(Vec::new())),
        reference_box: None,
        axes: [true; 2],
    };
    assert!(
        render(&clip)
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 0),
        "an empty inner area must not restore its bounding rectangle"
    );
    clip.axes = [false, true];
    let pixels = render(&clip);
    for y in 0..60 {
        for x in 0..80 {
            let expected = if (10..50).contains(&y) { u8::MAX } else { 0 };
            assert_eq!(
                pixels[(y * 80 + x) * 4 + 3],
                expected,
                "pixel ({x},{y}): a single clipped axis ignores the corner outline"
            );
        }
    }
}

#[test]
fn pdf_clip_preserves_a_hole_when_all_contours_are_reflected() {
    let outline = vec![
        Contour::polygon(&[[0.0, 0.0], [80.0, 0.0], [80.0, 60.0], [0.0, 60.0]]),
        Contour::polygon(&[[10.0, 10.0], [10.0, 40.0], [30.0, 40.0], [30.0, 10.0]]),
    ];
    for reflected in [false, true] {
        let contours = if reflected {
            outline
                .iter()
                .map(|contour| contour.map(|[x, y]| [SURFACE.width - x, y]))
                .collect()
        } else {
            outline.clone()
        };
        let clip = Clip {
            rect: SURFACE,
            radii: Default::default(),
            outline: Some(Arc::new(contours)),
            axes: [true; 2],
            reference_box: None,
        };
        let pixels = render(&clip);
        for y in 0..60 {
            for x in 0..80 {
                let source_x = if reflected { 79 - x } else { x };
                let expected = if (10..30).contains(&source_x) && (10..40).contains(&y) {
                    0
                } else {
                    u8::MAX
                };
                assert_eq!(
                    pixels[(y * 80 + x) * 4 + 3],
                    expected,
                    "pixel ({x},{y}), reflected={reflected}: the inner contour remains a hole"
                );
            }
        }
    }
}
