//! Relative and sticky positioning of non-atomic inline boxes (CSS Position 3 §3.3, §3.4) and
//! inline boxes as containing blocks of absolutely positioned boxes (CSS Position 3 §2.1).
mod common;
use common::{PROFILE, assert_labelled_pt, render_trace};
use ferri_pdf::{BoxPaint, DisplayItem, LayoutTrace, Rect};
use std::collections::BTreeMap;

const OFFSET: [f32; 2] = [7.0, 3.0];
const RED: [u8; 3] = [0xff, 0, 0];
const GREEN: [u8; 3] = [0, 0xff, 0];
const BLUE: [u8; 3] = [0, 0, 0xff];
const YELLOW: [u8; 3] = [0xff, 0xff, 0];
const MAGENTA: [u8; 3] = [0xff, 0, 0xff];
/// The span's border and padding, which its content edges lie inside.
const EDGE: f32 = 3.0;
const SPAN_EDGES: &str = "border:1pt solid black;padding:2pt";
const MARKERS: &str = ".tl{position:absolute;left:0;top:0;width:1pt;height:1pt;background:#0000ff}\
     .br{position:absolute;right:0;bottom:0;width:1pt;height:1pt;background:#ff00ff}";
const WORDS: &str = "moved words wrap over several lines of this narrow block";

fn trace(css: &str, body: &str) -> LayoutTrace {
    render_trace(&format!("<style>{PROFILE}{css}</style>{body}"))
}

fn relative(css: &str) -> String {
    format!(
        "{css}.r{{position:relative;left:{}pt;top:{}pt}}",
        OFFSET[0], OFFSET[1]
    )
}

/// Box fragments on `page` with background `rgb`, in paint order.
fn boxes(trace: &LayoutTrace, page: usize, rgb: [u8; 3]) -> Vec<BoxPaint> {
    trace.display_lists[page]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.rgb == rgb => {
                Some(paint.clone())
            }
            _ => None,
        })
        .collect()
}

fn only_box(trace: &LayoutTrace, page: usize, rgb: [u8; 3]) -> Rect {
    let found = boxes(trace, page, rgb);
    assert_eq!(
        found.len(),
        1,
        "one box #{:02x}{:02x}{:02x} on page {page}: {found:#?}",
        rgb[0],
        rgb[1],
        rgb[2]
    );
    found[0].rect
}

/// Index in paint order of the first box with background `rgb` on page one.
fn paint_index(trace: &LayoutTrace, rgb: [u8; 3]) -> usize {
    trace.display_lists[0]
        .iter()
        .position(|item| matches!(item, DisplayItem::Box(paint) if paint.decoration.background.rgb == rgb))
        .unwrap_or_else(|| panic!("no box with background {rgb:?}"))
}

/// Each glyph run's page, absolute left edge and baseline by its text, which the probes keep
/// unique.
fn runs(trace: &LayoutTrace) -> BTreeMap<String, (usize, f32, f32)> {
    let mut runs = BTreeMap::new();
    for (page, lines) in trace.pages.iter().enumerate() {
        for line in lines {
            for run in &line.runs {
                let text = run.text.trim();
                if !text.is_empty() {
                    let previous =
                        runs.insert(text.to_owned(), (page, line.x + run.x, line.baseline));
                    assert!(previous.is_none(), "run text {text:?} is not unique");
                }
            }
        }
    }
    runs
}

fn assert_moved(label: &str, before: Rect, after: Rect, offset: [f32; 2]) {
    assert_labelled_pt(&format!("{label} left"), after.x, before.x + offset[0]);
    assert_labelled_pt(&format!("{label} top"), after.y, before.y + offset[1]);
    assert_labelled_pt(&format!("{label} width"), after.width, before.width);
    assert_labelled_pt(&format!("{label} height"), after.height, before.height);
}

/// `[left, top, right, bottom]` content edges of an inline box fragment of the probe span.
fn content_edges(fragment: &BoxPaint) -> [f32; 4] {
    let rect = fragment.rect;
    [
        rect.x + if fragment.inline_edges[0] { EDGE } else { 0.0 },
        rect.y + EDGE,
        rect.x + rect.width - if fragment.inline_edges[1] { EDGE } else { 0.0 },
        rect.y + rect.height - EDGE,
    ]
}

#[test]
fn relative_inline_fragments_move_with_their_text_without_moving_line_layout() {
    let css = ".w{width:150pt}.r{background:#ff0000}.ib{display:inline-block;background:#0000ff}";
    let body = format!(
        "<div class=w>Plain <span class=r>{WORDS} <span class=ib>atomic</span></span> tail</div>\
         <p>After</p>"
    );
    let normal = trace(css, &body);
    let moved = trace(&relative(css), &body);
    let (before, after) = (boxes(&normal, 0, RED), boxes(&moved, 0, RED));
    assert!(
        before.len() > 2,
        "the span must wrap onto several lines: {before:#?}"
    );
    assert_eq!(
        before.len(),
        after.len(),
        "relative positioning keeps fragments"
    );
    for (index, (before, after)) in before.iter().zip(&after).enumerate() {
        assert_moved(
            &format!("fragment {index}"),
            before.rect,
            after.rect,
            OFFSET,
        );
        assert_eq!(before.inline_edges, after.inline_edges);
    }
    assert_moved(
        "atomic inline inside the span",
        only_box(&normal, 0, BLUE),
        only_box(&moved, 0, BLUE),
        OFFSET,
    );
    let (before, after) = (runs(&normal), runs(&moved));
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "the same runs"
    );
    for (text, &(page, x, baseline)) in &before {
        let inside = WORDS.contains(text.as_str()) || text == "atomic";
        let offset = if inside { OFFSET } else { [0.0; 2] };
        let (moved_page, moved_x, moved_baseline) = after[text];
        assert_eq!(page, moved_page, "{text} stays on its page");
        assert_labelled_pt(&format!("{text} left"), moved_x, x + offset[0]);
        assert_labelled_pt(
            &format!("{text} baseline"),
            moved_baseline,
            baseline + offset[1],
        );
    }
}

#[test]
fn decorations_of_an_ancestor_move_with_a_relative_descendant() {
    let css = ".u{text-decoration:underline}";
    let body = "<p class=u>outer <span class=r>INNER</span> tail</p>";
    let underlines = |trace: &LayoutTrace| -> Vec<Rect> {
        let mut rects: Vec<Rect> = trace.display_lists[0]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Decoration(decoration) => Some(decoration.rect),
                _ => None,
            })
            .collect();
        rects.sort_by(|a, b| a.x.total_cmp(&b.x));
        rects
    };
    let normal = underlines(&trace(css, body));
    let moved_trace = trace(&relative(css), body);
    let moved = underlines(&moved_trace);
    assert_eq!(
        normal.len(),
        1,
        "one underline joins the plain line: {normal:#?}"
    );
    assert_eq!(
        moved.len(),
        3,
        "the underline splits around the moved text: {moved:#?}"
    );
    let (_, inner_x, _) = runs(&moved_trace)["INNER"];
    let inner = moved_trace.pages[0]
        .iter()
        .flat_map(|line| &line.runs)
        .find(|run| run.text.trim() == "INNER")
        .expect("the inner glyph run");
    assert_labelled_pt("moved segment left", moved[1].x, inner_x);
    assert_labelled_pt("moved segment width", moved[1].width, inner.width);
    assert_labelled_pt("moved segment top", moved[1].y, normal[0].y + OFFSET[1]);
    for segment in [moved[0], moved[2]] {
        assert_labelled_pt("unmoved segment top", segment.y, normal[0].y);
    }
    assert_labelled_pt("line start", moved[0].x, normal[0].x);
    assert_labelled_pt(
        "line end",
        moved[2].x + moved[2].width,
        normal[0].x + normal[0].width,
    );
    assert_labelled_pt(
        "the segments cover the unmoved line",
        moved[0].width + moved[1].width + moved[2].width,
        normal[0].width,
    );
}

#[test]
fn block_splitting_a_relative_inline_moves_with_it() {
    let css = format!(".r{{background:#ff0000}}.blk{{background:#ffff00}}{MARKERS}");
    let body = "<div>Lead <span class=r>in<div class=blk>Block<span class=tl></span></div>out</span></div>";
    let normal = trace(&css, body);
    let moved = trace(&relative(&css), body);
    assert_moved(
        "split block",
        only_box(&normal, 0, YELLOW),
        only_box(&moved, 0, YELLOW),
        OFFSET,
    );
    let (before, after) = (boxes(&normal, 0, RED), boxes(&moved, 0, RED));
    assert_eq!(
        before.len(),
        2,
        "the span's parts around the block: {before:#?}"
    );
    for (before, after) in before.iter().zip(&after) {
        assert_moved("span part", before.rect, after.rect, OFFSET);
    }
    // The box inside the block takes the span, not the block, as its containing block.
    let first = after[0].rect;
    let marker = only_box(&moved, 0, BLUE);
    assert_labelled_pt("containing block left", marker.x, first.x);
    assert_labelled_pt("containing block top", marker.y, first.y);
}

#[test]
fn inline_containing_block_runs_from_the_first_fragment_to_the_end_most_edges() {
    for rtl in [false, true] {
        let css = format!(
            ".w{{width:150pt}}.r{{position:relative;{SPAN_EDGES};background:#ff0000}}{MARKERS}"
        );
        let direction = if rtl { " dir=rtl" } else { "" };
        let body = format!(
            "<div class=w{direction}>Lead <span class=r>{WORDS}<span class=tl></span><span class=br></span></span></div>"
        );
        let trace = trace(&css, &body);
        let fragments = boxes(&trace, 0, RED);
        assert!(
            fragments.len() > 2,
            "rtl {rtl}: the span wraps: {fragments:#?}"
        );
        let edges: Vec<[f32; 4]> = fragments.iter().map(content_edges).collect();
        let first = edges[0];
        let last = edges[edges.len() - 1];
        let (left, right) = if rtl {
            (
                edges
                    .iter()
                    .map(|edge| edge[0])
                    .fold(f32::INFINITY, f32::min),
                first[2],
            )
        } else {
            (
                first[0],
                edges
                    .iter()
                    .map(|edge| edge[2])
                    .fold(f32::NEG_INFINITY, f32::max),
            )
        };
        let start = only_box(&trace, 0, BLUE);
        let end = only_box(&trace, 0, MAGENTA);
        assert_labelled_pt(&format!("rtl {rtl}: containing block left"), start.x, left);
        assert_labelled_pt(
            &format!("rtl {rtl}: containing block top"),
            start.y,
            first[1],
        );
        assert_labelled_pt(
            &format!("rtl {rtl}: containing block right"),
            end.x + end.width,
            right,
        );
        assert_labelled_pt(
            &format!("rtl {rtl}: containing block bottom"),
            end.y + end.height,
            last[3],
        );
    }
}

#[test]
fn inline_containing_block_continues_across_a_page_break() {
    const PAGE_TOP: f32 = 20.0;
    const PAGE_BOTTOM: f32 = 180.0;
    let css = format!(
        "@page{{size:300pt 200pt;margin:{PAGE_TOP}pt}}.w{{width:150pt}}\
         .r{{position:relative;{SPAN_EDGES};background:#ff0000}}{MARKERS}"
    );
    let words = WORDS.repeat(4);
    // The first marker is anchored on the span's first page, the second on its last.
    let body = format!(
        "<div class=w>Lead <span class=r><span class=br></span>{words}<span class=tl></span></span></div>"
    );
    let trace = trace(&css, &body);
    assert_eq!(trace.pages.len(), 2, "the span crosses one page break");
    let first = content_edges(&boxes(&trace, 0, RED)[0]);
    let start = only_box(&trace, 1, BLUE);
    assert_labelled_pt(
        "on the last page the containing block starts at the page top",
        start.y,
        PAGE_TOP,
    );
    assert_labelled_pt(
        "the first fragment's start edge carries over",
        start.x,
        first[0],
    );
    let end = only_box(&trace, 0, MAGENTA);
    assert_labelled_pt(
        "on the first page the containing block reaches the page bottom",
        end.y + end.height,
        PAGE_BOTTOM,
    );
    let right = (0..2)
        .flat_map(|page| boxes(&trace, page, RED))
        .map(|fragment| content_edges(&fragment)[2])
        .fold(f32::NEG_INFINITY, f32::max);
    assert_labelled_pt(
        "the end-most edge of every page's fragments",
        end.x + end.width,
        right,
    );
}

#[test]
fn relative_inline_stacks_with_its_context_unless_its_z_index_is_an_integer() {
    let css = ".bg{background:#ffff00}.r{background:#ff0000}\
               .ib{display:inline-block;background:#0000ff}\
               .neg{position:relative;z-index:-1;background:#00ff00}";
    let body = "<div class=bg><span class=r>R <span class=neg>N</span></span><span class=ib>I</span></div>";
    let normal = trace(css, body);
    assert!(
        paint_index(&normal, RED) < paint_index(&normal, BLUE),
        "an unpositioned inline paints in tree order"
    );
    let auto = trace(&relative(css), body);
    assert!(
        paint_index(&auto, BLUE) < paint_index(&auto, RED),
        "a relative inline paints after in-flow inline content (CSS 2.1 Appendix E step 8)"
    );
    assert!(
        paint_index(&auto, GREEN) < paint_index(&auto, YELLOW),
        "without its own stacking context, a negative descendant paints below the block"
    );
    let stacked = trace(&relative(&format!("{css}.r{{z-index:0}}")), body);
    assert!(
        paint_index(&stacked, YELLOW) < paint_index(&stacked, GREEN),
        "an integer z-index makes a stacking context holding its negative descendant"
    );
    assert!(paint_index(&stacked, GREEN) < paint_index(&stacked, RED));
}

#[test]
fn sticky_inline_shifts_into_its_scrollport_and_is_a_stacking_context() {
    let css = ".scroll{overflow:hidden;height:40pt;background:#ffff00}\
               .s{position:sticky;bottom:0;background:#ff0000}\
               .neg{position:relative;z-index:-1;background:#00ff00}";
    let body = "<div class=scroll><div>one<br>two<br>three <span class=s>stuck <span class=neg>N</span></span></div></div>";
    let trace = trace(css, body);
    let mut transforms = Vec::new();
    let mut open = Vec::new();
    let mut sticky = None;
    for item in &trace.display_lists[0] {
        match item {
            DisplayItem::PushTransform(matrix) => {
                open.push(true);
                transforms.push(*matrix);
            }
            DisplayItem::PushClip(_)
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PushBlend(_)
            | DisplayItem::PushFilter(_)
            | DisplayItem::PushMask(_) => open.push(false),
            DisplayItem::Pop => {
                open.pop();
            }
            DisplayItem::Box(paint) if paint.decoration.background.rgb == RED => {
                assert!(
                    open.contains(&true),
                    "the sticky fragment paints inside its shift"
                );
                sticky = Some(paint.rect);
            }
            _ => {}
        }
    }
    let sticky = sticky.expect("the sticky fragment paints");
    let scroller = only_box(&trace, 0, YELLOW);
    let shift = transforms
        .last()
        .expect("the sticky shift is a translation");
    assert_eq!(&shift[..4], &[1.0, 0.0, 0.0, 1.0], "a pure translation");
    assert_labelled_pt("no horizontal shift", shift[4], 0.0);
    assert_labelled_pt(
        "the fragment's bottom meets the scrollport's",
        sticky.y + sticky.height + shift[5],
        scroller.y + scroller.height,
    );
    assert!(
        paint_index(&trace, YELLOW) < paint_index(&trace, GREEN),
        "a sticky box is always a stacking context, holding its negative descendant above the block"
    );
    assert!(
        paint_index(&trace, GREEN) < paint_index(&trace, RED),
        "negative descendants precede the inline box's own background (CSS 2.1 Appendix E)"
    );
}
