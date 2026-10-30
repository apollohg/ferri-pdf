//! The `stretch` sizing keyword (CSS Sizing 3 §3.2, §4.2): the margin box fills the
//! containing block where percentages resolve, and otherwise behaves as `auto`, `0` or
//! `none`.
mod common;
use common::{assert_labelled_pt, local_options, painted_box};
use ferri_pdf::{LayoutTrace, Rect};

const PARENT: [u8; 3] = [1, 2, 3];
const TARGET: [u8; 3] = [4, 5, 6];
const MARGIN: f32 = 10.0;
const PADDING: f32 = 5.0;
const BORDER: f32 = 1.0;
/// The parent's content box.
const WIDTH: f32 = 300.0;
const HEIGHT: f32 = 200.0;

fn render(css: &str, body: &str) -> LayoutTrace {
    ferri_pdf::render_html(
        &format!(
            "<style>{}body{{margin:0}}div{{margin:0}}.p{{background:rgb(1,2,3)}}.t{{background:rgb(4,5,6);margin:{MARGIN}pt;padding:{PADDING}pt;border:{BORDER}pt solid transparent}}{css}</style>{body}",
            common::PROFILE
        ),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("stretch: {error}; CSS={css}; HTML={body}"))
    .layout
    .unwrap()
}

/// The target's border box.
fn target(css: &str, body: &str) -> Rect {
    painted_box(&render(css, body), TARGET)
}

const NESTED: &str = "<div class=p><div class=t>x</div></div>";
/// The border box a stretch-fit size gives within the parent's content box.
const STRETCHED_WIDTH: f32 = WIDTH - 2.0 * MARGIN;
const STRETCHED_HEIGHT: f32 = HEIGHT - 2.0 * MARGIN;

#[test]
fn stretch_fills_the_containing_block_with_the_margin_box() {
    for (css, expected) in [
        (
            format!(".p{{width:{WIDTH}pt}}.t{{float:left;width:stretch}}"),
            STRETCHED_WIDTH,
        ),
        (
            format!(".p{{width:{WIDTH}pt}}.t{{display:inline-block;width:stretch}}"),
            STRETCHED_WIDTH,
        ),
        (
            format!(".p{{width:{WIDTH}pt}}.t{{display:table;width:stretch}}"),
            STRETCHED_WIDTH,
        ),
        (
            format!(".p{{width:{WIDTH}pt}}.t{{width:stretch;box-sizing:border-box}}"),
            STRETCHED_WIDTH,
        ),
        (
            format!(".p{{width:{WIDTH}pt}}.t{{float:left;width:10pt;min-width:stretch}}"),
            STRETCHED_WIDTH,
        ),
        (
            format!(".p{{width:{WIDTH}pt}}.t{{width:500pt;max-width:stretch}}"),
            STRETCHED_WIDTH,
        ),
        (
            format!(
                ".p{{width:{WIDTH}pt;position:relative}}.t{{position:absolute;left:0;width:stretch}}"
            ),
            STRETCHED_WIDTH,
        ),
        (
            format!(".p{{width:{WIDTH}pt;display:flex}}.t{{width:stretch}}"),
            STRETCHED_WIDTH,
        ),
        (
            format!(".p{{width:{WIDTH}pt;display:grid;justify-items:start}}.t{{width:stretch}}"),
            STRETCHED_WIDTH,
        ),
    ] {
        assert_labelled_pt(&css, target(&css, NESTED).width, expected);
    }
}

#[test]
fn stretch_fills_a_definite_block_size() {
    for (css, expected) in [
        (
            format!(
                ".p{{height:{HEIGHT}pt;border-top:1pt solid;border-bottom:1pt solid}}.t{{height:stretch}}"
            ),
            STRETCHED_HEIGHT,
        ),
        // Margins that would collapse through the parent's edges are treated as zero.
        (
            format!(".p{{height:{HEIGHT}pt}}.t{{height:stretch}}"),
            HEIGHT,
        ),
        (
            format!(".p{{height:{HEIGHT}pt;padding-top:1pt}}.t{{height:stretch}}"),
            HEIGHT - MARGIN,
        ),
        (
            format!(".p{{height:{HEIGHT}pt;display:flow-root}}.t{{height:stretch}}"),
            STRETCHED_HEIGHT,
        ),
        (
            format!(
                ".p{{height:{HEIGHT}pt;position:relative}}.t{{position:absolute;top:0;height:stretch}}"
            ),
            STRETCHED_HEIGHT,
        ),
        (
            format!(
                ".p{{height:{HEIGHT}pt;display:flex;flex-direction:column}}.t{{height:stretch}}"
            ),
            STRETCHED_HEIGHT,
        ),
        (
            format!(".p{{height:{HEIGHT}pt;display:grid;align-items:start}}.t{{height:stretch}}"),
            STRETCHED_HEIGHT,
        ),
        (
            format!(".p{{height:{HEIGHT}pt;display:flex;align-items:start}}.t{{height:stretch}}"),
            STRETCHED_HEIGHT,
        ),
    ] {
        assert_labelled_pt(&css, target(&css, NESTED).height, expected);
    }
}

#[test]
fn stretch_without_a_definite_containing_size_behaves_as_auto_zero_or_none() {
    let auto = target(".t{display:inline-block}", NESTED);
    for css in [
        ".t{display:inline-block;height:stretch}",
        ".t{display:inline-block;min-height:stretch}",
        ".t{display:inline-block;max-height:stretch}",
    ] {
        let stretched = target(css, NESTED);
        assert_labelled_pt(&format!("{css}: height"), stretched.height, auto.height);
    }
    // Inside a shrink-to-fit float, the width contributes as `auto`.
    let contained = |declarations: &str| {
        let trace = render(
            &format!(".p{{float:left}}.t{{{declarations}}}"),
            "<div class=p><div class=t>wide content</div></div>",
        );
        painted_box(&trace, PARENT).width
    };
    assert_labelled_pt(
        "stretch contributes as auto",
        contained("width:stretch"),
        contained(""),
    );
    assert_labelled_pt(
        "a stretch minimum contributes as zero",
        contained("min-width:stretch"),
        contained(""),
    );
}
