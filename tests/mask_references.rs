mod common;

use common::local_options;
use ferri_pdf::{Error, PdfError, render_html};

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
            format!("<mask id=m{level}><rect width=20 height=20 fill=white{layers}/></mask>")
        })
        .collect();
    format!(
        "<!doctype html><svg width=0 height=0><defs>{masks}</defs></svg>\
         <div style='width:20px;height:20px;background:red;mask:url(#m0)'></div>"
    )
}

#[test]
fn shared_mask_references_draw_within_a_per_page_budget() {
    const SHALLOW: usize = 4;
    const EXPONENTIAL: usize = 12;
    render_html(&doubling_masks(SHALLOW), &local_options())
        .expect("a few shared mask references render");
    let started = std::time::Instant::now();
    let result = render_html(&doubling_masks(EXPONENTIAL), &local_options());
    assert!(
        matches!(
            result,
            Err(Error::Pdf(PdfError::MaskElementDrawLimit { .. }))
        ),
        "{} mask element draws must stop at the per-page budget instead of rasterizing each one, \
         got {:?} after {:?}",
        (1usize << EXPONENTIAL) - 1,
        result.map(|_| ()),
        started.elapsed()
    );
}
