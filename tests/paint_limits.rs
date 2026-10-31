mod common;

use common::local_options;
use ferri_pdf::{PaintDiagnostic, RenderDiagnostic, RenderOutput, render_html};

const RED: [u8; 3] = [255, 0, 0];

/// The first page's RGBA pixels at one pixel per point, and its width in pixels.
fn first_page_pixels(output: &RenderOutput) -> (Vec<[u8; 4]>, usize) {
    let pdf = hayro::hayro_syntax::Pdf::new(output.pdf.clone()).expect("the output is a PDF");
    let pixels = hayro::render(
        &pdf.pages()[0],
        &hayro::RenderCache::new(),
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &hayro::RenderSettings::default(),
        &hayro::PixmapSettings {
            x_scale: 1.0,
            y_scale: 1.0,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    (
        pixels.data_as_u8_slice().as_chunks::<4>().0.to_vec(),
        usize::from(pixels.width()),
    )
}

fn red_pixels(output: &RenderOutput) -> usize {
    first_page_pixels(output)
        .0
        .iter()
        .filter(|pixel| pixel[..3] == RED)
        .count()
}

/// SVG `mask` elements whose content is masked twice by the next element, so drawing the first
/// draws the last `2^(depth - 1)` times.
fn doubling_masks(depth: usize) -> String {
    let masks: String = (0..depth)
        .map(|level| {
            let layers = if level + 1 < depth {
                format!(
                    " style='mask:url(#m{next}),url(#m{next})'",
                    next = level + 1
                )
            } else {
                String::new()
            };
            format!(
                "<mask id=m{level} maskContentUnits=objectBoundingBox>\
                 <rect width=1 height=1 fill='white'{layers}/></mask>"
            )
        })
        .collect();
    format!(
        "<!doctype html><svg width=0 height=0><defs>{masks}</defs></svg>\
         <div style='width:20px;height:20px;background:red;mask:url(#m0)'></div>"
    )
}

#[test]
fn shared_mask_references_past_the_page_budget_draw_as_transparent_black_layers() {
    const SHALLOW: usize = 4;
    const EXPONENTIAL: usize = 12;
    let shallow = render_html(&doubling_masks(SHALLOW), &local_options())
        .expect("a few shared mask references render");
    assert!(
        shallow.diagnostics.is_empty() && red_pixels(&shallow) > 0,
        "white masks within the budget show the box: {:?}",
        shallow.diagnostics
    );
    let started = std::time::Instant::now();
    let output = render_html(&doubling_masks(EXPONENTIAL), &local_options())
        .expect("mask references past the budget still render");
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            RenderDiagnostic::Paint(PaintDiagnostic::MaskElementDrawLimit { .. })
        )),
        "{} mask element draws must stop at the per-page budget instead of rasterizing each one, \
         got {:?} after {:?}",
        (1usize << EXPONENTIAL) - 1,
        output.diagnostics,
        started.elapsed()
    );
    let (shown, unmasked) = (red_pixels(&output), red_pixels(&shallow));
    assert!(
        (1..=unmasked).contains(&shown),
        "the draws within the budget show the white rectangles they reach and those past it are \
         transparent black layers adding nothing: {shown} red pixels of {unmasked}"
    );
}

#[test]
fn filters_whose_raster_exceeds_the_rasterizer_are_omitted_with_their_content() {
    let output = render_html(
        "<!doctype html><div style='width:100px;height:100px;background:red;filter:blur(1e30px)'></div>\
         <div style='width:100px;height:100px;background:red'></div>",
        &local_options(),
    )
    .expect("an unrasterizable filter does not fail the document");
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            RenderDiagnostic::Paint(PaintDiagnostic::Omitted { reason }) if reason.contains("raster")
        )),
        "the omitted filter is reported, got {:?}",
        output.diagnostics
    );
    let (pixels, width) = first_page_pixels(&output);
    let red_rows: Vec<_> = pixels
        .chunks(width)
        .enumerate()
        .filter(|(_, row)| row.iter().any(|pixel| pixel[..3] == RED))
        .map(|(row, _)| row)
        .collect();
    const PX_PER_PT: f32 = 0.75;
    let box_rows = (100.0 * PX_PER_PT) as usize;
    assert!(
        red_rows.len().abs_diff(box_rows) <= 1,
        "only the unfiltered box is painted, {box_rows} rows tall, got {} red rows",
        red_rows.len()
    );
    assert!(
        red_rows.first().is_some_and(|first| *first >= box_rows),
        "the filtered box above it is omitted, red rows start at {:?}",
        red_rows.first()
    );
}

#[test]
fn transforms_beyond_finite_coordinates_render_without_their_content() {
    let output = render_html(
        "<!doctype html><div style='transform:matrix3d(1e30,0,0,0,0,1e30,0,0,0,0,1,1e30,0,0,0,1e-30);width:100px;height:100px;background:red'>x</div>\
         <div style='width:100px;height:100px;background:blue'></div>",
        &local_options(),
    )
    .expect("a transform that overflows every coordinate does not fail the document");
    let (pixels, _) = first_page_pixels(&output);
    assert!(
        pixels.iter().any(|pixel| pixel[..3] == [0, 0, 255]),
        "the content after the overflowing box still paints"
    );
}
