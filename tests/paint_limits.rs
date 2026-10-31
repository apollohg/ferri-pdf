mod common;

use common::{local_options, lossy_options, pdf_number_faults};
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

/// Pixels where red paint shows at least half opaque over the white page.
fn mostly_red_pixels(output: &RenderOutput) -> usize {
    const HALF: u8 = 128;
    first_page_pixels(output)
        .0
        .iter()
        .filter(|pixel| pixel[0] == RED[0] && pixel[1] < HALF && pixel[2] < HALF)
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
    let output = render_html(&doubling_masks(EXPONENTIAL), &lossy_options())
        .expect("mask references past the budget still render");
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            RenderDiagnostic::Paint(PaintDiagnostic::MaskElementDrawLimit { .. })
        )),
        "{} mask element draws must stop at the per-page budget instead of rasterizing each one, \
         got {:?}",
        (1usize << EXPONENTIAL) - 1,
        output.diagnostics
    );
    let (shown, unmasked) = (red_pixels(&output), red_pixels(&shallow));
    assert!(
        (1..=unmasked).contains(&shown),
        "the draws within the budget show the white rectangles they reach and those past it are \
         transparent black layers adding nothing: {shown} red pixels of {unmasked}"
    );
}

#[test]
fn mask_images_too_large_to_rasterize_draw_at_a_lower_resolution() {
    const BOX: &str = "width:100px;height:100px;background:red";
    // A tile wider than the rasterizer allows, repeated down the box, covers all of it.
    let wide = "url(\"data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 \
                width=%22100000%22 height=%2210%22><rect width=%22100%%22 \
                height=%22100%%22/></svg>\")";
    let output = render_html(
        &format!("<!doctype html><div style='{BOX};mask-image:{wide}'></div>"),
        &local_options(),
    )
    .expect("a mask image larger than the rasterizer allows does not fail the document");
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            RenderDiagnostic::Paint(PaintDiagnostic::ReducedResolution)
        )),
        "the lower resolution is reported, got {:?}",
        output.diagnostics
    );
    let unmasked = render_html(
        &format!("<!doctype html><div style='{BOX}'></div>"),
        &local_options(),
    )
    .unwrap();
    // The coarser raster's last row of each 10px tile is partly covered, which fades at most
    // one of every ten page rows.
    const FADED_ROWS: f32 = 0.15;
    let (shown, whole) = (mostly_red_pixels(&output), red_pixels(&unmasked));
    assert!(
        shown <= whole && shown as f32 >= whole as f32 * (1.0 - FADED_ROWS),
        "the opaque mask image still shows the box: {shown} of its {whole} pixels"
    );
}

#[test]
fn filters_whose_raster_exceeds_the_rasterizer_are_omitted_with_their_content() {
    let output = render_html(
        "<!doctype html><div style='width:100px;height:100px;background:red;filter:blur(1e30px)'></div>\
         <div style='width:100px;height:100px;background:red'></div>",
        &lossy_options(),
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
    let faults = pdf_number_faults(&output.pdf);
    assert!(
        faults.is_empty(),
        "the overflowing transform writes no number:\n{}",
        faults.join("\n")
    );
    assert!(
        output.diagnostics.is_empty(),
        "a transform mapping onto no finite area paints nothing, as one mapping onto no area \
         does, rather than reaching the writer: {:?}",
        output.diagnostics
    );
    let (pixels, _) = first_page_pixels(&output);
    assert!(
        pixels.iter().all(|pixel| pixel[..3] != RED),
        "no part of the transformed box paints"
    );
    assert!(
        pixels.iter().any(|pixel| pixel[..3] == [0, 0, 255]),
        "the content after the overflowing box still paints"
    );
}

#[test]
fn outlines_past_finite_coordinates_are_not_drawn() {
    let output = render_html(
        "<!doctype html><div style='width:100px;height:100px;background:rgb(0,0,255);\
         outline:calc(1e38px * 100) solid red'></div>",
        &local_options(),
    )
    .expect("an outline wider than finite coordinates does not fail the document");
    let faults = pdf_number_faults(&output.pdf);
    assert!(
        faults.is_empty(),
        "the outline writes no number:\n{}",
        faults.join("\n")
    );
    assert!(
        output.diagnostics.is_empty(),
        "the outline encloses no finite area and is not drawn, leaving nothing for the writer \
         to omit: {:?}",
        output.diagnostics
    );
    let (pixels, _) = first_page_pixels(&output);
    assert!(
        pixels.iter().all(|pixel| pixel[..3] != RED),
        "no part of the outline paints"
    );
    assert!(
        pixels.iter().any(|pixel| pixel[..3] == [0, 0, 255]),
        "the box the outline surrounds still paints"
    );
}

#[test]
fn items_with_numbers_pdf_cannot_write_are_omitted_with_a_diagnostic() {
    const BLUE: [u8; 3] = [0, 0, 255];
    for unwritable in [
        "width:3e38px;height:10px;background:red",
        "width:10px;height:10px;background:red;transform:scale(1e30)",
        "width:10px;height:10px;background:red;box-shadow:0 0 0 3e38px red",
    ] {
        let html = format!(
            "<!doctype html><div style='{unwritable}'></div>\
             <div style='width:100px;height:100px;background:rgb(0,0,255)'></div>"
        );
        assert!(
            matches!(
                render_html(&html, &local_options()),
                Err(ferri_pdf::Error::ContentLost { .. })
            ),
            "{unwritable}: the omitted item loses content, which fails the default render"
        );
        let output = render_html(&html, &lossy_options())
            .unwrap_or_else(|error| panic!("{unwritable}: must render: {error}"));
        let faults = pdf_number_faults(&output.pdf);
        assert!(
            faults.is_empty(),
            "{unwritable}: the PDF holds only finite numbers within the PDF limit:\n{}",
            faults.join("\n")
        );
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic,
                RenderDiagnostic::Paint(PaintDiagnostic::Omitted { reason })
                    if reason.starts_with("paint number")
            )),
            "{unwritable}: the omitted item is reported, got {:?}",
            output.diagnostics
        );
        let (pixels, _) = first_page_pixels(&output);
        assert!(
            pixels.iter().all(|pixel| pixel[..3] != RED),
            "{unwritable}: no part of the unwritable item paints"
        );
        assert!(
            pixels.iter().any(|pixel| pixel[..3] == BLUE),
            "{unwritable}: the content after the unwritable item still paints"
        );
    }
}

#[test]
fn links_and_destinations_pdf_cannot_write_are_left_out() {
    for (link, target) in [
        ("", "transform:translateY(1e30px)"),
        ("display:inline-block;transform:translateX(1e30px)", ""),
    ] {
        let output = render_html(
            &format!(
                "<!doctype html><p><a href=#target style='{link}'>link</a></p>\
                 <h1 id=target style='{target}'>target</h1>"
            ),
            &lossy_options(),
        )
        .unwrap_or_else(|error| panic!("{link} {target}: must render: {error}"));
        let faults = pdf_number_faults(&output.pdf);
        assert!(
            faults.is_empty(),
            "{link} {target}: the link annotation, destination and bookmark write no number \
             past the PDF limit:\n{}",
            faults.join("\n")
        );
        assert!(
            output.diagnostics.iter().any(|diagnostic| matches!(
                diagnostic,
                RenderDiagnostic::Paint(PaintDiagnostic::Omitted { reason })
                    if reason.starts_with("paint number")
            )),
            "{link} {target}: what is left out is reported, got {:?}",
            output.diagnostics
        );
    }
}
