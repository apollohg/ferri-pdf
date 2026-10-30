//! CSS Sizing 4 §4 preferred aspect ratios in every layout mode, measured from the layout.
mod common;

use common::{assert_labelled_pt, painted_box, render_trace};

const PAGE: &str = "@page{size:400pt 600pt;margin:0}body{margin:0;font:10pt/12pt serif}";
const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn boxed(css: &str, body: &str, color: [u8; 3]) -> ferri_pdf::Rect {
    let html = format!("<!doctype html><style>{PAGE}{css}</style>{body}");
    let trace = render_trace(&html);
    painted_box(&trace, color)
}

fn assert_size(label: &str, rect: ferri_pdf::Rect, width: f32, height: f32) {
    assert_labelled_pt(&format!("{label} width"), rect.width, width);
    assert_labelled_pt(&format!("{label} height"), rect.height, height);
}

#[test]
fn block_ratios_transfer_definite_sizes_between_axes() {
    for (css, width, height) in [
        ("width:200pt;aspect-ratio:2/1", 200.0, 100.0),
        ("height:50pt;aspect-ratio:1", 50.0, 50.0),
        ("height:50pt;aspect-ratio:3", 150.0, 50.0),
        // An automatic width stretches and the height follows it.
        ("aspect-ratio:4", 400.0, 100.0),
        // Both sizes definite: the ratio has no effect.
        ("width:100pt;height:30pt;aspect-ratio:1", 100.0, 30.0),
        // A degenerate ratio behaves as auto, so the height is the content's.
        ("width:100pt;aspect-ratio:0/1", 100.0, 0.0),
        // `border-box` sizing applies a `<ratio>` to the border box…
        (
            "width:100pt;padding:10pt 20pt;box-sizing:border-box;aspect-ratio:1",
            100.0,
            100.0,
        ),
        // …but `auto && <ratio>` always relates the content boxes.
        (
            "width:100pt;padding:10pt 20pt;box-sizing:border-box;aspect-ratio:auto 1",
            100.0,
            80.0,
        ),
    ] {
        let rect = boxed(
            &format!(".a{{background:red;{css}}}"),
            "<div class=a></div>",
            RED,
        );
        assert_size(css, rect, width, height);
    }
}

/// §4.3: the ratio-dependent height grows to fit the content unless `min-height` is set or
/// the box is a scroll container.
#[test]
fn table_ratios_transfer_definite_sizes_but_rows_and_columns_still_grow() {
    // CSS Sizing 4 §4.1: aspect-ratio applies to table boxes, which are no internal table
    // boxes; like a specified size, the derived one is a minimum for the table's content.
    let body = "<table class=t><tr><td><div class=c></div></td></tr></table>";
    let base = ".t{border-spacing:0;background:rgb(255,0,0)}td{padding:0}";
    for (css, width, height) in [
        (
            ".t{width:200pt;aspect-ratio:2}.c{width:10pt;height:10pt}",
            200.0,
            100.0,
        ),
        (
            ".t{height:50pt;aspect-ratio:3}.c{width:10pt;height:10pt}",
            150.0,
            50.0,
        ),
        (
            ".t{width:200pt;aspect-ratio:2}.c{width:10pt;height:150pt}",
            200.0,
            150.0,
        ),
        (
            ".t{height:50pt;aspect-ratio:1}.c{width:120pt;height:10pt}",
            120.0,
            50.0,
        ),
    ] {
        assert_size(
            css,
            boxed(&format!("{base}{css}"), body, RED),
            width,
            height,
        );
    }
}

#[test]
fn automatic_minimum_follows_the_content() {
    let tall = "<div class=a><div style='height:180pt'></div></div>";
    for (css, height) in [
        ("", 180.0),
        ("min-height:0", 100.0),
        ("overflow:hidden", 100.0),
        ("max-height:150pt", 150.0),
    ] {
        let rect = boxed(
            &format!(".a{{width:100pt;aspect-ratio:1;background:red;{css}}}"),
            tall,
            RED,
        );
        assert_size(
            &format!("content minimum with `{css}`"),
            rect,
            100.0,
            height,
        );
    }
}

/// §4.4: limits in the ratio-determining axis transfer to the other; a definite size in the
/// destination axis is unaffected.
#[test]
fn limits_transfer_through_the_ratio() {
    for (css, width, height) in [
        ("width:100pt;max-width:60pt;aspect-ratio:2", 60.0, 30.0),
        ("aspect-ratio:1;max-height:80pt;min-height:0", 80.0, 80.0),
        (
            "aspect-ratio:1;min-height:450pt;max-width:1000pt",
            450.0,
            450.0,
        ),
        // A definite `min-width` takes no transferred minimum, so the ratio gives way.
        ("aspect-ratio:1;min-height:450pt;min-width:0", 400.0, 450.0),
        ("height:40pt;min-width:100pt;aspect-ratio:1", 100.0, 40.0),
    ] {
        let rect = boxed(
            &format!(".a{{background:red;{css}}}"),
            "<div class=a></div>",
            RED,
        );
        assert_size(css, rect, width, height);
    }
}

#[test]
fn shrink_to_fit_and_positioned_boxes_use_the_ratio() {
    for (css, width, height) in [
        ("float:left;height:40pt;aspect-ratio:3/2", 60.0, 40.0),
        (
            "display:inline-block;height:30pt;aspect-ratio:2",
            60.0,
            30.0,
        ),
        ("position:absolute;height:40pt;aspect-ratio:2", 80.0, 40.0),
        (
            "position:absolute;left:0;right:200pt;aspect-ratio:4",
            200.0,
            50.0,
        ),
        (
            "position:absolute;inset:0 100pt 0 0;aspect-ratio:1",
            300.0,
            300.0,
        ),
    ] {
        let rect = boxed(
            &format!(".a{{background:red;{css}}}"),
            "<div class=a></div><p>text</p>",
            RED,
        );
        assert_size(css, rect, width, height);
    }
}

/// CSS Flexbox 1 §9.2.3 and §9.8: a stretched item of a single-line container with a
/// definite cross size takes its flex base size through the ratio; without a definite cross
/// size the main size sets the cross size.
#[test]
fn flex_items_size_through_the_ratio() {
    let row = boxed(
        ".f{display:flex;height:60pt}.f div{aspect-ratio:2;background:red}",
        "<div class=f><div></div></div>",
        RED,
    );
    assert_size("stretched row item", row, 120.0, 60.0);
    let centered = boxed(
        ".f{display:flex;align-items:flex-start}.f div{width:80pt;aspect-ratio:4;background:red}",
        "<div class=f><div></div></div>",
        RED,
    );
    assert_size("start-aligned row item", centered, 80.0, 20.0);
    let column = boxed(
        ".f{display:flex;flex-direction:column;width:200pt}.f div{aspect-ratio:4;background:red}",
        "<div class=f><div></div></div>",
        RED,
    );
    assert_size("stretched column item", column, 200.0, 50.0);
}

/// CSS Grid 2 §6.2: `normal` alignment sizes an item with a ratio as a block-level box, so
/// it is not stretched to its row, while `stretch` still stretches it.
#[test]
fn grid_items_with_ratios_are_not_stretched_by_normal_alignment() {
    let css = ".g{display:grid;grid-template-columns:100pt 100pt;grid-template-rows:200pt}\
               .r{aspect-ratio:2;background:red}.b{aspect-ratio:2;align-self:stretch;background:blue}";
    let html = format!(
        "<!doctype html><style>{PAGE}{css}</style><div class=g><div class=r></div><div class=b></div></div>"
    );
    let trace = render_trace(&html);
    assert_size("normal grid item", painted_box(&trace, RED), 100.0, 50.0);
    assert_size(
        "stretched grid item",
        painted_box(&trace, BLUE),
        100.0,
        200.0,
    );
}

/// Replaced elements: `auto && <ratio>` keeps the natural ratio, a lone `<ratio>` overrides
/// it, and size containment drops the natural size and ratio (CSS Containment 2 §3.1).
#[test]
fn replaced_elements_use_the_preferred_ratio() {
    // image-pattern.png is 80×40 pixels.
    let image = "<img src='../assets/image-pattern.png'>";
    for (css, width, height) in [
        ("width:100pt", 100.0, 50.0),
        ("width:100pt;aspect-ratio:1", 100.0, 100.0),
        ("width:100pt;aspect-ratio:auto 1", 100.0, 50.0),
        ("height:30pt;aspect-ratio:3", 90.0, 30.0),
        ("width:100pt;contain:size", 100.0, 0.0),
        ("width:100pt;contain:size;aspect-ratio:1", 100.0, 100.0),
        ("width:100pt;contain:size;aspect-ratio:auto 2", 100.0, 50.0),
    ] {
        let rect = boxed(
            &format!("img{{display:block;background:red;{css}}}"),
            image,
            RED,
        );
        assert_size(css, rect, width, height);
    }
}

#[test]
fn intrinsic_contributions_take_the_ratio_width_from_a_definite_height() {
    let rect = boxed(
        ".outer{float:left;background:red}.inner{height:30pt;aspect-ratio:3}",
        "<div class=outer><div class=inner></div></div>",
        RED,
    );
    assert_size("float sized by its child's ratio", rect, 90.0, 30.0);
}
