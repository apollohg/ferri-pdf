//! CSS Box Alignment 3 overflow positions (§4.4) and `justify-items` (§6.1, §7.1) across
//! flex, grid and block layout.
mod common;
use common::{PROFILE, assert_labelled_pt, local_options, painted_box};
use ferri_pdf::{LayoutTrace, render_html};

const SUBJECT: [u8; 3] = [255, 0, 0];
/// The page area's left and top edges in the shared profile.
const ORIGIN: f32 = 36.0;

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}.subject{{background:rgb(255,0,0)}}{css}</style>{body}"),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("alignment CSS {css:?} must render: {error}"))
    .layout
    .expect("layout was requested")
}

fn subject(css: &str, body: &str) -> ferri_pdf::Rect {
    painted_box(&render(css, body), SUBJECT)
}

#[test]
fn flex_main_axis_safe_alignment_starts_overflowing_items_at_flex_start() {
    let body = "<section><div class=subject></div></section>";
    let base =
        "section{display:flex;width:100pt;height:20pt}.subject{flex:none;width:160pt;height:10pt}";
    for (container, x) in [
        ("justify-content:center", ORIGIN - 30.0),
        ("justify-content:unsafe center", ORIGIN - 30.0),
        ("justify-content:safe center", ORIGIN),
        ("place-content:safe center", ORIGIN),
        ("place-content:safe center unsafe center", ORIGIN - 30.0),
        ("justify-content:end", ORIGIN - 60.0),
        ("justify-content:safe end", ORIGIN),
        // Flex-start is the right edge of a reversed row.
        (
            "flex-direction:row-reverse;justify-content:safe center",
            ORIGIN - 60.0,
        ),
        (
            "flex-direction:row-reverse;justify-content:unsafe center",
            ORIGIN - 30.0,
        ),
        // §4.3: distributed alignment falls back to `safe center`.
        ("justify-content:space-around", ORIGIN),
        ("justify-content:space-evenly", ORIGIN),
    ] {
        let rect = subject(&format!("{base}section{{{container}}}"), body);
        assert_labelled_pt(&format!("item x for {container}"), rect.x, x);
    }
}

#[test]
fn flex_cross_axis_safe_alignment_applies_per_item_and_per_line() {
    let body = "<section><div class=subject></div></section>";
    let base = "section{display:flex;width:100pt;height:20pt}.subject{width:10pt;height:40pt}";
    for (css, y) in [
        ("section{align-items:center}", ORIGIN - 10.0),
        ("section{align-items:safe center}", ORIGIN),
        (
            "section{align-items:safe center}.subject{align-self:unsafe center}",
            ORIGIN - 10.0,
        ),
        (
            "section{align-items:unsafe center}.subject{align-self:safe end}",
            ORIGIN,
        ),
        (".subject{align-self:unsafe end}", ORIGIN - 20.0),
        (".subject{align-self:safe end}", ORIGIN),
    ] {
        let rect = subject(&format!("{base}{css}"), body);
        assert_labelled_pt(&format!("item y for {css}"), rect.y, y);
    }
    let lines = "<section><div class=subject></div><div></div></section>";
    let wrapped = "section{display:flex;flex-wrap:wrap;width:100pt;height:20pt}section div{width:100pt;height:15pt}";
    for (content, y) in [
        ("center", ORIGIN - 5.0),
        ("unsafe center", ORIGIN - 5.0),
        ("safe center", ORIGIN),
    ] {
        let rect = subject(
            &format!("{wrapped}section{{align-content:{content}}}"),
            lines,
        );
        assert_labelled_pt(
            &format!("first line y for align-content:{content}"),
            rect.y,
            y,
        );
    }
}

#[test]
fn grid_safe_alignment_starts_overflowing_items_and_tracks_at_the_inline_start() {
    let body = "<section><div class=subject></div></section>";
    let base = "section{display:grid;grid-template-columns:100pt;width:100pt}.subject{width:160pt;height:10pt}";
    for (css, x) in [
        (".subject{justify-self:center}", ORIGIN - 30.0),
        (".subject{justify-self:unsafe center}", ORIGIN - 30.0),
        (".subject{justify-self:safe center}", ORIGIN),
        ("section{justify-items:safe center}", ORIGIN),
        ("section{place-items:unsafe center safe center}", ORIGIN),
        (
            "section{place-items:safe center}.subject{place-self:unsafe center}",
            ORIGIN - 30.0,
        ),
        (
            "section{justify-items:safe center}.subject{justify-self:unsafe center}",
            ORIGIN - 30.0,
        ),
        // The inline start of a right-to-left grid is its right edge.
        (
            "section{direction:rtl}.subject{justify-self:safe center}",
            ORIGIN - 60.0,
        ),
    ] {
        let rect = subject(&format!("{base}{css}"), body);
        assert_labelled_pt(&format!("item x for {css}"), rect.x, x);
    }
    let tracks =
        "section{display:grid;grid-template-columns:160pt;width:100pt}.subject{height:10pt}";
    for (content, x) in [
        ("center", ORIGIN - 30.0),
        ("unsafe center", ORIGIN - 30.0),
        ("safe center", ORIGIN),
    ] {
        let rect = subject(
            &format!("{tracks}section{{justify-content:{content}}}"),
            body,
        );
        assert_labelled_pt(&format!("track x for justify-content:{content}"), rect.x, x);
    }
    let rows = "section{display:grid;grid-template-rows:40pt;height:20pt}.subject{height:10pt}";
    for (content, y) in [("end", ORIGIN - 20.0), ("safe end", ORIGIN)] {
        let rect = subject(&format!("{rows}section{{align-content:{content}}}"), body);
        assert_labelled_pt(&format!("track y for align-content:{content}"), rect.y, y);
    }
}

#[test]
fn block_container_content_overflows_safely_without_an_overflow_position() {
    // CSS Box Alignment 3 §5.1.1: without an <overflow-position>, a block container's content
    // alignment is safe, whether or not the container scrolls.
    let body = "<section><div class=subject></div></section>";
    let base = "section{height:20pt}.subject{height:60pt}";
    for (css, y) in [
        ("section{align-content:center}", ORIGIN),
        ("section{align-content:end}", ORIGIN),
        ("section{align-content:center;overflow:hidden}", ORIGIN),
        ("section{align-content:safe end}", ORIGIN),
        ("section{align-content:unsafe center}", ORIGIN - 20.0),
        ("section{align-content:unsafe end}", ORIGIN - 40.0),
    ] {
        let rect = subject(&format!("{base}{css}"), body);
        assert_labelled_pt(&format!("content y for {css}"), rect.y, y);
    }
}

#[test]
fn block_level_boxes_align_by_their_parent_justify_items_and_overflow_position() {
    let body = "<section><div class=subject></div></section>";
    let base = "section{width:100pt}.subject{width:160pt;height:10pt}";
    for (css, x) in [
        (".subject{justify-self:center}", ORIGIN - 30.0),
        (".subject{justify-self:unsafe center}", ORIGIN - 30.0),
        (".subject{justify-self:safe center}", ORIGIN),
        ("section{justify-items:safe end}", ORIGIN),
        ("section{justify-items:end}", ORIGIN - 60.0),
    ] {
        let rect = subject(&format!("{base}{css}"), body);
        assert_labelled_pt(&format!("block x for {css}"), rect.x, x);
    }
    let nested = "<section><div class=outer><div class=subject></div></div></section>";
    let base = "section{width:200pt}.outer{width:200pt}.subject{width:50pt;height:10pt}";
    for (css, x) in [
        // A non-legacy value aligns only the children; the grandchild starts.
        ("section{justify-items:center}", ORIGIN),
        // `legacy` values inherit through `justify-items: legacy` (§7.1).
        ("section{justify-items:legacy center}", ORIGIN + 75.0),
        ("section{justify-items:right legacy}", ORIGIN + 150.0),
        ("section{justify-items:legacy left}", ORIGIN),
        (
            "section{justify-items:legacy right}.outer{justify-items:legacy}",
            ORIGIN + 150.0,
        ),
        (
            "section{justify-items:legacy right}.outer{justify-items:normal}",
            ORIGIN,
        ),
        (
            "section{justify-items:legacy right}.subject{justify-self:start}",
            ORIGIN,
        ),
    ] {
        let rect = subject(&format!("{base}{css}"), nested);
        assert_labelled_pt(&format!("nested block x for {css}"), rect.x, x);
    }
}

#[test]
fn aligned_block_level_boxes_size_automatic_widths_as_fit_content() {
    let body = "<section><div class=subject><span></span></div></section>";
    let rect = subject(
        "section{width:200pt;justify-items:center}span{display:inline-block;width:40pt;height:10pt}",
        body,
    );
    assert_labelled_pt("fit-content width", rect.width, 40.0);
    assert_labelled_pt("centred x", rect.x, ORIGIN + 80.0);
    let rect = subject(
        "section{width:200pt;justify-items:stretch}span{display:inline-block;width:40pt;height:10pt}",
        body,
    );
    assert_labelled_pt("stretched width", rect.width, 200.0);
    assert_labelled_pt("stretched x", rect.x, ORIGIN);
}

#[test]
fn table_cells_align_content_by_align_content_over_vertical_align() {
    const CELL_PT: f32 = 100.0;
    const SUBJECT_PT: f32 = 20.0;
    for (declarations, offset) in [
        ("align-content:end;vertical-align:top", CELL_PT - SUBJECT_PT),
        ("align-content:start;vertical-align:bottom", 0.0),
        (
            "align-content:center;vertical-align:top",
            (CELL_PT - SUBJECT_PT) / 2.0,
        ),
        (
            "align-content:space-evenly;vertical-align:top",
            (CELL_PT - SUBJECT_PT) / 2.0,
        ),
        ("vertical-align:bottom", CELL_PT - SUBJECT_PT),
    ] {
        let trace = render(
            &format!(
                "body{{margin:0}}table{{border-spacing:0}}\
                 td{{height:{CELL_PT}pt;padding:0;{declarations}}}\
                 .subject{{width:{SUBJECT_PT}pt;height:{SUBJECT_PT}pt}}"
            ),
            "<table><tr><td><div class=subject></div></td></tr></table>",
        );
        assert_labelled_pt(
            &format!("{declarations}: content offset in the cell"),
            painted_box(&trace, SUBJECT).y - ORIGIN,
            offset,
        );
    }
}

#[test]
fn block_content_space_crossing_page_ends_breaks_inside_instead_of_dropping_lines() {
    // The profile's page area is 408pt tall; ten 18pt lines follow 820pt of space, which
    // leaves 4pt of it on the third page above them (CSS Break 3 §4.4).
    const LINES: usize = 10;
    const LINE_PT: f32 = 18.0;
    const SPACE_PT: f32 = 820.0;
    const PAGE_AREA_PT: f32 = 408.0;
    let body = format!(
        "<section>{}</section>",
        (0..LINES)
            .map(|line| format!("<div>Line {line}</div>"))
            .collect::<String>()
    );
    let first_baseline = render("", "<div>Line 0</div>").pages[0][0].baseline;
    let content = LINE_PT * LINES as f32;
    for css in [
        format!(
            "section{{height:{}pt;align-content:end}}",
            SPACE_PT + content
        ),
        format!(
            "section{{height:{}pt;align-content:center}}",
            2.0 * SPACE_PT + content
        ),
        format!("section{{padding-top:{SPACE_PT}pt}}"),
    ] {
        let trace = render(&css, &body);
        let pages: Vec<Vec<&str>> = trace
            .pages
            .iter()
            .map(|page| page.iter().map(|line| line.text.as_str()).collect())
            .collect();
        assert!(
            pages.len() >= 3 && pages[..2].iter().all(Vec::is_empty),
            "{css}: the space fills the first two pages, got {pages:?}"
        );
        let lines = &trace.pages[2];
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            (0..LINES)
                .map(|line| format!("Line {line}"))
                .collect::<Vec<_>>(),
            "{css}: every line follows the space on the third page, pages {pages:?}"
        );
        for (index, line) in lines.iter().enumerate() {
            assert_labelled_pt(
                &format!("{css}: baseline of line {index}"),
                line.baseline,
                first_baseline + SPACE_PT - 2.0 * PAGE_AREA_PT + LINE_PT * index as f32,
            );
        }
    }
}
