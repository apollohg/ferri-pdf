//! `last baseline` alignment (CSS Box Alignment 3 §4.2, §9.3): boxes align their last
//! baselines within the group of their endmost row or line, which takes the safe end
//! fallback alignment; boxes without a group fall back to safe end alignment.
mod common;
use common::{assert_labelled_pt, local_options, painted_box};
use ferri_pdf::LayoutTrace;

const CONTAINER: [u8; 3] = [1, 2, 3];
const ITEM: [u8; 3] = [4, 5, 6];
/// Two items: three small lines ending in "C", and one large line "D" with a bottom margin.
const ITEMS: &str = "<div class=c><div class=a>A<br>B<br>C</div><div class=b>D</div></div>";
const STYLE: &str = "body{margin:0}div{margin:0}.c{background:rgb(1,2,3)}.a{font:10pt/12pt serif}.b{font:20pt/24pt serif;margin-bottom:5pt}";

fn render(css: &str, body: &str) -> LayoutTrace {
    ferri_pdf::render_html(
        &format!("<style>{}{STYLE}{css}</style>{body}", common::PROFILE),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("last baseline: {error}; CSS={css}; HTML={body}"))
    .layout
    .unwrap()
}

/// The page-space baseline of the line holding `text`.
fn baseline(trace: &LayoutTrace, text: &str) -> f32 {
    trace.pages[0]
        .iter()
        .find(|line| line.text == text)
        .unwrap_or_else(|| panic!("a line {text:?}"))
        .baseline
}

#[test]
fn flex_grid_and_table_rows_align_last_baselines() {
    for (css, body) in [
        (
            ".c{display:flex;align-items:last baseline}",
            ITEMS.to_owned(),
        ),
        (
            ".c{display:flex;align-items:start}.a,.b{align-self:last baseline}",
            ITEMS.to_owned(),
        ),
        (
            ".c{display:grid;grid-template-columns:auto auto;align-items:last baseline}",
            ITEMS.to_owned(),
        ),
        (
            "td{align-content:last baseline;padding:0}",
            "<table class=c><tr><td class=a>A<br>B<br>C</td><td class=b>D</td></tr></table>"
                .to_owned(),
        ),
    ] {
        let trace = render(css, &body);
        let [last, single] = [baseline(&trace, "C"), baseline(&trace, "D")];
        assert_labelled_pt(&format!("{css}: last baselines"), last, single);
        assert!(
            baseline(&trace, "A") < single - 20.0,
            "{css}: the first lines keep their own positions above the shared last baseline"
        );
    }
}

#[test]
fn the_last_baseline_group_sits_at_the_end_of_its_line() {
    // A taller first-baseline sibling makes the line taller than the last-baseline group.
    let trace = render(
        ".c{display:flex}.a,.b{align-self:last baseline}.b{background:rgb(7,8,9)}.t{align-self:baseline;height:200pt;width:10pt;background:rgb(4,5,6)}",
        "<div class=c><div class=a>A<br>B<br>C</div><div class=b>D</div><div class=t></div></div>",
    );
    let container = painted_box(&trace, CONTAINER);
    let deepest = painted_box(&trace, [7, 8, 9]);
    assert_labelled_pt(
        "the deepest last-baseline item's margin box ends at the line's end",
        container.y + container.height - (deepest.y + deepest.height),
        5.0,
    );
    assert_labelled_pt(
        "C shares D's last baseline at the line end",
        baseline(&trace, "C"),
        baseline(&trace, "D"),
    );
}

#[test]
fn boxes_without_a_baseline_group_fall_back_to_safe_end() {
    let item = "<div class=c><div class=t></div></div>";
    let target = ".t{width:10pt;height:10pt;background:rgb(4,5,6)}";
    for (css, horizontal) in [
        (".c{height:100pt;align-content:last baseline}", false),
        (
            ".c{display:flex;flex-direction:column;width:100pt}.t{align-self:last baseline}",
            true,
        ),
        (
            ".c{display:grid;width:100pt}.t{justify-self:last baseline}",
            true,
        ),
    ] {
        let trace = render(&format!("{target}{css}"), item);
        let container = painted_box(&trace, CONTAINER);
        let box_ = painted_box(&trace, ITEM);
        let gap = if horizontal {
            container.x + container.width - (box_.x + box_.width)
        } else {
            container.y + container.height - (box_.y + box_.height)
        };
        assert_labelled_pt(&format!("{css}: at the end"), gap, 0.0);
    }
    let css = format!("{target}.c{{height:5pt;align-content:last baseline}}");
    let trace = render(&css, item);
    assert_labelled_pt(
        "overflowing content stays at the start",
        painted_box(&trace, ITEM).y,
        painted_box(&trace, CONTAINER).y,
    );
}
