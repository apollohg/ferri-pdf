mod common;

use common::local_options;
use ferri_pdf::render_html;

#[test]
fn unclipped_pattern_overflow_is_bounded_before_any_tile_is_listed() {
    // A tile 1e-4 units wide whose content spans 2e6 units overlaps about 2e10 neighbours.
    let html = "<!doctype html><svg width=20 height=20>\
        <pattern id=p width=0.0001 height=0.0001 patternUnits=userSpaceOnUse overflow=visible>\
        <rect x=-1000000 y=-1000000 width=2000000 height=2000000 fill=red/></pattern>\
        <rect width=20 height=20 fill='url(#p)'/></svg>";
    let started = std::time::Instant::now();
    let result = render_html(html, &local_options());
    assert!(
        result.is_ok(),
        "a pattern overflowing past the tile budget clips to its tile: {:?}",
        result.err()
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "listing the overflowing tiles must stop at the budget, took {:?}",
        started.elapsed()
    );
}
