use super::common;
use super::weasy_reference;

use crate::{DisplayItem, LayoutTrace, render_html};
use common::{PROFILE, assert_pt, local_options};

const PAGE_LEFT: f32 = 36.0;
const PAGE_RIGHT: f32 = 324.0;
const FLOAT_WIDTH: f32 = 60.0;
const FLOAT_HEIGHT: f32 = 54.0;
const LINE_HEIGHT: f32 = 18.0;

#[test]
fn float_layout_matches_independently_recorded_geometry() {
    let reference = serde_json::from_str(include_str!(
        "../../tests/fixtures/flow/weasy70-floats.json"
    ))
    .expect("valid public synthetic float reference");
    weasy_reference::assert_weasy_lines(&reference);
}

fn render(css: &str, body: &str) -> LayoutTrace {
    render_html(
        &format!(
            "<!doctype html><style>{PROFILE}p{{margin:0}}.float{{width:{FLOAT_WIDTH}pt;height:{FLOAT_HEIGHT}pt}}{css}</style>{body}"
        ),
        &local_options(),
    )
    .unwrap_or_else(|error| panic!("float rendering failed for {css:?}: {error}"))
    .layout
    .unwrap()
}

fn image_paints(trace: &LayoutTrace) -> Vec<(usize, &crate::graphics::ImagePaint)> {
    trace
        .display_lists
        .iter()
        .enumerate()
        .flat_map(|(page, items)| {
            items.iter().filter_map(move |item| match item {
                DisplayItem::Image(image) => Some((page, image)),
                _ => None,
            })
        })
        .collect()
}

#[test]
fn float_paint_group_sits_between_block_backgrounds_and_inline_ink() {
    let trace = render(
        ".float{float:left;background:#ff0000}.body{background:#0000ff}.negative,.positive{position:absolute;top:0;left:0}.negative{z-index:-1;background:#00ff00}.positive{z-index:1;background:#ffff00}",
        "<div class=float><p>Float</p><div class=negative>Negative</div><div class=positive>Positive</div></div><p class=body>Body</p>",
    );
    let order: Vec<_> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                Some(format!("background {:?}", paint.decoration.background.rgb))
            }
            DisplayItem::Text { line } => Some(trace.pages[0][*line].text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        [
            "background [0, 255, 0]",
            "Negative",
            "background [0, 0, 255]",
            "background [255, 0, 0]",
            "Float",
            "Body",
            "background [255, 255, 0]",
            "Positive",
        ],
        "float contents form a paint group, but positioned descendants remain in the enclosing stacking context"
    );
}

#[test]
fn relative_ancestor_and_float_offsets_do_not_move_exclusions() {
    const PARENT_LEFT: f32 = 12.0;
    const FLOAT_LEFT: f32 = 7.0;
    const FLOAT_TOP: f32 = 9.0;
    for top in [-LINE_HEIGHT, LINE_HEIGHT] {
        let trace = render(
            &format!(
                "section{{position:relative;left:{PARENT_LEFT}pt;top:{top}pt}}.float{{float:left;position:relative;left:{FLOAT_LEFT}pt;top:{FLOAT_TOP}pt}}"
            ),
            "<section><img class=float src='../assets/image-pattern.png'><p>One</p><p>Two</p><p>Three</p><p>Four</p></section>",
        );
        assert_eq!(trace.pages[0].len(), 4);
        for (index, line) in trace.pages[0].iter().enumerate() {
            assert_pt(
                line.x,
                PAGE_LEFT + PARENT_LEFT + if index < 3 { FLOAT_WIDTH } else { 0.0 },
            );
        }
        let images = image_paints(&trace);
        assert_eq!(images.len(), 1);
        assert_pt(images[0].1.rect.x, PAGE_LEFT + PARENT_LEFT + FLOAT_LEFT);
        assert_pt(images[0].1.rect.y, PAGE_LEFT + top + FLOAT_TOP);
    }
}

#[test]
fn float_exclusions_end_at_the_margin_bottom_on_both_sides() {
    for side in ["left", "right"] {
        let trace = render(
            &format!(".float{{float:{side}}}p{{text-align:{side}}}"),
            "<img class=float src='../assets/image-pattern.png'><p>One</p><p>Two</p><p>Three</p><p>Four</p>",
        );
        assert_eq!(trace.pages.len(), 1, "{side}: no extra page");
        let lines = &trace.pages[0];
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["One", "Two", "Three", "Four"],
            "{side}: preserve source text exactly once"
        );
        for (index, line) in lines.iter().enumerate() {
            let exclusion = if index < 3 { FLOAT_WIDTH } else { 0.0 };
            if side == "left" {
                assert_pt(line.x, PAGE_LEFT + exclusion);
            } else {
                assert_pt(line.x + line.width, PAGE_RIGHT - exclusion);
            }
            assert_pt(
                line.baseline - lines[0].baseline,
                index as f32 * LINE_HEIGHT,
            );
        }
        let images: Vec<_> = trace.display_lists[0]
            .iter()
            .filter_map(|item| {
                if let DisplayItem::Image(image) = item {
                    Some(image)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(images.len(), 1, "{side}: one painted float");
        assert_pt(
            images[0].rect.x,
            if side == "left" {
                PAGE_LEFT
            } else {
                PAGE_RIGHT - FLOAT_WIDTH
            },
        );
        assert_pt(images[0].rect.y, PAGE_LEFT);
    }
}

#[test]
fn percentage_float_heights_use_the_actual_containing_block_height() {
    const CONTAINER_HEIGHT: f32 = 120.0;
    for replaced in [false, true] {
        let body = if replaced {
            "<section><img class=float src='../assets/image-pattern.png'></section>"
        } else {
            "<section><div class=float><p>Inside</p></div></section>"
        };
        let auto = render(".float{float:left;height:auto;background:red}", body);
        let height = |trace: &LayoutTrace| {
            trace.display_lists[0]
                .iter()
                .find_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                        Some(paint.rect.height)
                    }
                    _ => None,
                })
                .expect("float has a background")
        };
        for (container, expected) in [
            (String::new(), height(&auto)),
            (
                format!("height:{CONTAINER_HEIGHT}pt"),
                CONTAINER_HEIGHT / 2.0,
            ),
        ] {
            let trace = render(
                &format!("section{{{container}}}.float{{float:left;height:50%;background:red}}"),
                body,
            );
            assert_pt(height(&trace), expected);
        }
    }
}

#[test]
fn float_contents_use_their_own_float_context_for_line_fitting() {
    const OUTER_WIDTH: f32 = 120.0;
    const INNER_WIDTH: f32 = 40.0;
    let trace = render(
        &format!(
            ".outer{{float:left;width:{OUTER_WIDTH}pt}}.inner{{float:left;width:{INNER_WIDTH}pt;height:{}pt}}",
            2.0 * LINE_HEIGHT
        ),
        "<div class=outer><img class=inner src='../assets/image-pattern.png'><p>First<br>Second<br>Third</p></div><p>Body</p>",
    );
    for (text, x) in [
        ("First", PAGE_LEFT + INNER_WIDTH),
        ("Second", PAGE_LEFT + INNER_WIDTH),
        ("Third", PAGE_LEFT),
        ("Body", PAGE_LEFT + OUTER_WIDTH),
    ] {
        let lines: Vec<_> = trace.pages[0]
            .iter()
            .filter(|line| line.text == text)
            .collect();
        assert_eq!(lines.len(), 1, "{text}: one source line");
        assert_pt(lines[0].x, x);
    }
}

#[test]
fn clearing_breaks_do_not_clear_floats_that_follow_them() {
    for ending in ["After", ""] {
        let trace = render(
            ".float{float:left}",
            &format!(
                "<p>Before<br clear=all><img class=float src='../assets/image-pattern.png'>{ending}</p><p>Following</p>"
            ),
        );
        assert_eq!(trace.pages[0][0].text, "Before");
        assert_pt(trace.pages[0][0].x, PAGE_LEFT);
        let images = image_paints(&trace);
        assert_eq!(
            images.len(),
            1,
            "{ending:?}: retain the float after the hard break"
        );
        assert_pt(images[0].1.rect.y, PAGE_LEFT + LINE_HEIGHT);
        let label = if ending.is_empty() {
            "Following"
        } else {
            "After"
        };
        let after = trace.pages[0]
            .iter()
            .find(|line| line.text == label)
            .expect("text following the break exists");
        assert_pt(after.x, PAGE_LEFT + FLOAT_WIDTH);
        assert_pt(after.baseline - trace.pages[0][0].baseline, LINE_HEIGHT);
    }
}

#[test]
fn cleared_blocks_wait_for_the_last_fragment_of_prior_floats() {
    for fragments in [2, 3] {
        for empty in [false, true] {
            let cleared = if empty {
                "<div style='clear:both'></div><p>Following</p>"
            } else {
                "<p style='clear:both'>Following</p>"
            };
            let middle = "<p style='break-before:page'>FloatMiddle</p>".repeat(fragments - 2);
            let trace = render(
                "@page{size:360pt 144pt;margin:36pt}.float{float:left;height:auto}p{orphans:1;widows:1}",
                &format!(
                    "<div class=float><p>FloatFirst</p>{middle}<p style='break-before:page'>FloatLast</p></div>{cleared}"
                ),
            );
            let (page, following) = trace
                .pages
                .iter()
                .enumerate()
                .find_map(|(page, lines)| {
                    lines
                        .iter()
                        .find(|line| line.text == "Following")
                        .map(|line| (page, line))
                })
                .expect("the cleared flow must remain present");
            assert_eq!(
                page,
                fragments - 1,
                "fragments={fragments}, empty={empty}: clearance must include the float continuation"
            );
            let last = trace.pages[page]
                .iter()
                .find(|line| line.text == "FloatLast")
                .expect("last float fragment");
            assert!(
                (following.baseline - last.baseline - LINE_HEIGHT).abs() < 0.001,
                "fragments={fragments}, empty={empty}: cleared baseline {} must be one line below float baseline {}",
                following.baseline,
                last.baseline
            );
        }
    }
}

#[test]
fn float_clearance_in_a_table_cell_preserves_the_sibling_flow() {
    let trace = render(
        "@page{size:360pt 144pt;margin:36pt}.float{float:left;height:auto}table{width:100%;border-spacing:0}td{width:50%;padding:0;vertical-align:top}p{orphans:1;widows:1}",
        "<table><tr><td><div class=float><p>FloatFirst</p><p style='break-before:page'>FloatLast</p></div><div style='clear:both'></div><p>Following</p></td><td><p>SiblingFirst</p><p>SiblingNext</p></td></tr></table>",
    );
    assert_eq!(
        trace.pages.len(),
        2,
        "only the float and cleared flow continue"
    );
    for label in ["SiblingFirst", "SiblingNext", "FloatFirst"] {
        assert_eq!(
            trace.pages[0]
                .iter()
                .filter(|line| line.text == label)
                .count(),
            1,
            "{label} remains on the first page exactly once"
        );
    }
    let last = trace.pages[1]
        .iter()
        .find(|line| line.text == "FloatLast")
        .expect("float continuation");
    let following = trace.pages[1]
        .iter()
        .find(|line| line.text == "Following")
        .expect("cleared cell continuation");
    assert_pt(following.baseline - last.baseline, LINE_HEIGHT);
    assert_pt(following.x, last.x);
    assert_eq!(
        trace.pages.iter().map(Vec::len).sum::<usize>(),
        5,
        "no duplicated or lost cell text"
    );
}

#[test]
fn float_clearance_resumes_in_the_next_column() {
    const COLUMN_HEIGHT: f32 = 72.0;
    let trace = render(
        &format!(
            ".columns{{column-count:2;column-gap:0;column-fill:auto;height:{COLUMN_HEIGHT}pt}}.float{{float:left;height:auto}}p{{orphans:1;widows:1}}"
        ),
        "<div class=columns><div class=float><p>FloatFirst</p><p style='break-before:column'>FloatLast</p></div><div style='clear:both'></div><p>Following</p></div>",
    );
    assert_eq!(
        trace.pages.len(),
        1,
        "column clearance stays on the physical page"
    );
    assert_eq!(trace.pages[0].len(), 3, "each source line paints once");
    let first = trace.pages[0]
        .iter()
        .find(|line| line.text == "FloatFirst")
        .expect("first float column");
    let last = trace.pages[0]
        .iter()
        .find(|line| line.text == "FloatLast")
        .expect("second float column");
    let following = trace.pages[0]
        .iter()
        .find(|line| line.text == "Following")
        .expect("cleared column flow");
    assert!(
        last.x > first.x,
        "the forced column break advances the float"
    );
    assert_pt(following.x, last.x);
    assert_pt(following.baseline - last.baseline, LINE_HEIGHT);
}

#[test]
fn clearing_line_breaks_wait_for_deferred_floats_and_work_at_paragraph_end() {
    const SPACER_HEIGHT: f32 = 36.0;
    for ending in ["After", ""] {
        let trace = render(
            "@page{size:360pt 144pt;margin:36pt}.float{float:left}p{orphans:1;widows:1}",
            &format!(
                "<div style='height:{SPACER_HEIGHT}pt'></div><img class=float src='../assets/image-pattern.png'><p>Before<br clear=all>{ending}</p><p>Following</p>"
            ),
        );
        let before = trace.pages[0]
            .iter()
            .find(|line| line.text == "Before")
            .expect("text before clearance stays on its original page");
        let label = if ending.is_empty() {
            "Following"
        } else {
            "After"
        };
        let (page, after) = trace
            .pages
            .iter()
            .enumerate()
            .find_map(|(page, lines)| {
                lines
                    .iter()
                    .find(|line| line.text == label)
                    .map(|line| (page, line))
            })
            .expect("text after clearance is present");
        assert_eq!(
            page, 1,
            "{ending:?}: clearance waits for the deferred float on page two"
        );
        assert_pt(
            after.baseline - before.baseline,
            FLOAT_HEIGHT - SPACER_HEIGHT,
        );
        assert_pt(after.x, PAGE_LEFT);
        assert_eq!(image_paints(&trace).len(), 1);
    }
}

#[test]
fn clearing_line_breaks_keep_the_paragraph_and_clear_only_the_selected_side() {
    for side in ["left", "right"] {
        for (clear, clears) in [
            ("none", false),
            ("left", side == "left"),
            ("right", side == "right"),
            ("all", true),
        ] {
            for css in [false, true] {
                let value = if clear == "all" { "both" } else { clear };
                let br = if css {
                    format!("<br style='clear:{value}'>")
                } else {
                    format!("<br clear='{clear}'>")
                };
                let trace = render(
                    &format!(".float{{float:{side}}}p{{text-transform:uppercase}}"),
                    &format!(
                        "<img class=float src='../assets/image-pattern.png'><p>ŉ{br}after</p>"
                    ),
                );
                assert_eq!(
                    trace.pages[0].len(),
                    2,
                    "{side}/{clear}/{css}: two source lines"
                );
                assert_eq!(trace.pages[0][0].text, "ʼN");
                assert_eq!(trace.pages[0][1].text, "AFTER");
                assert_eq!(
                    trace.pages[0][0].paragraph, trace.pages[0][1].paragraph,
                    "a clearing br does not split the paragraph"
                );
                assert_pt(
                    trace.pages[0][1].baseline - trace.pages[0][0].baseline,
                    if clears { FLOAT_HEIGHT } else { LINE_HEIGHT },
                );
                assert_pt(
                    trace.pages[0][1].x,
                    PAGE_LEFT
                        + if side == "left" && !clears {
                            FLOAT_WIDTH
                        } else {
                            0.0
                        },
                );
            }
        }
    }
}

#[test]
fn clear_does_not_displace_atomic_inline_boxes() {
    for clear in ["none", "left", "both"] {
        let trace = render(
            &format!(
                ".float{{float:left}}span{{display:inline-block;clear:{clear};width:30pt;height:18pt;background:red}}"
            ),
            "<img class=float src='../assets/image-pattern.png'><p><span></span>After</p>",
        );
        let paint = trace.display_lists[0]
            .iter()
            .find_map(|item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => Some(paint),
                _ => None,
            })
            .expect("inline box has a background");
        assert_pt(paint.rect.y, PAGE_LEFT);
        assert_pt(paint.rect.x, PAGE_LEFT + FLOAT_WIDTH);
    }
}

#[test]
fn clearance_only_observes_the_requested_float_side() {
    for side in ["left", "right"] {
        for clear in ["none", "left", "right", "both"] {
            let trace = render(
                &format!(".float{{float:{side}}}.clear{{clear:{clear}}}"),
                "<img class=float src='../assets/image-pattern.png'><p>Before</p><p class=clear>After</p>",
            );
            let lines = &trace.pages[0];
            assert_eq!(lines.len(), 2, "{side}/{clear}: two source lines");
            assert_eq!((&*lines[0].text, &*lines[1].text), ("Before", "After"));
            let clears = clear == side || clear == "both";
            assert_pt(
                lines[1].baseline - lines[0].baseline,
                if clears { FLOAT_HEIGHT } else { LINE_HEIGHT },
            );
            assert_pt(
                lines[1].x,
                if side == "left" && !clears {
                    PAGE_LEFT + FLOAT_WIDTH
                } else {
                    PAGE_LEFT
                },
            );
        }
    }
}

#[test]
fn descendant_clearance_stops_top_margin_collapse_only_when_required() {
    const PARENT_MARGIN: f32 = 20.0;
    const CHILD_MARGIN: f32 = 30.0;
    const TOP_PADDING: f32 = 1.0;
    let initial_baseline = render("", "<p>Control</p>").pages[0][0].baseline;
    for depth in [0, 1, 3] {
        for clear in ["none", "right", "left", "both"] {
            let trace = render(
                &format!(
                    "main{{padding-top:{TOP_PADDING}pt}}.float{{float:left}}section{{margin-top:{PARENT_MARGIN}pt;background:red}}p{{margin-top:{CHILD_MARGIN}pt;clear:{clear}}}"
                ),
                &format!(
                    "<main><img class=float src='../assets/image-pattern.png'><section>{}<p>Cleared</p>{}</section></main>",
                    "<div>".repeat(depth),
                    "</div>".repeat(depth)
                ),
            );
            let backgrounds: Vec<_> = trace.display_lists[0]
                .iter()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                        Some(paint)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(backgrounds.len(), 1, "{clear}: one parent background");
            let clears = matches!(clear, "left" | "both");
            assert_pt(
                backgrounds[0].rect.y,
                PAGE_LEFT + TOP_PADDING + if clears { PARENT_MARGIN } else { CHILD_MARGIN },
            );
            assert_pt(
                trace.pages[0][0].baseline,
                initial_baseline + TOP_PADDING + if clears { FLOAT_HEIGHT } else { CHILD_MARGIN },
            );
            assert_pt(
                backgrounds[0].rect.height,
                if clears {
                    FLOAT_HEIGHT - PARENT_MARGIN + LINE_HEIGHT
                } else {
                    LINE_HEIGHT
                },
            );
        }
    }
}

#[test]
fn kept_blocks_move_beyond_floats_without_retaining_previous_page_clearance() {
    let initial_baseline = render("", "<p>Control</p>").pages[0][0].baseline;
    for spacer in [0, 1] {
        let trace = render(
            "@page{size:360pt 144pt;margin:36pt}main{padding-top:1pt}.float{float:left}section{break-inside:avoid;margin-top:20pt;background:red}p{clear:left;margin-top:30pt}",
            &format!(
                "<div style='height:{spacer}pt'></div><main><img class=float src='../assets/image-pattern.png'><section><p>First<br>Second</p></section></main>"
            ),
        );
        assert_eq!(
            trace.pages.len(),
            2,
            "the kept parent moves beyond the float's page"
        );
        assert!(
            trace.pages[0].is_empty(),
            "neither source line may be left behind"
        );
        assert_eq!(
            trace.pages[1]
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["First", "Second"]
        );
        assert_pt(trace.pages[1][0].baseline, initial_baseline);
        assert_pt(trace.pages[1][1].baseline, initial_baseline + LINE_HEIGHT);
        let backgrounds: Vec<_> = trace
            .display_lists
            .iter()
            .enumerate()
            .flat_map(|(page, items)| {
                items.iter().filter_map(move |item| match item {
                    DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                        Some((page, paint))
                    }
                    _ => None,
                })
            })
            .collect();
        assert_eq!(backgrounds.len(), 1, "the moved parent paints once");
        assert_eq!(backgrounds[0].0, 1);
        assert_pt(backgrounds[0].1.rect.y, PAGE_LEFT);
        assert_pt(backgrounds[0].1.rect.height, 2.0 * LINE_HEIGHT);
        let images = image_paints(&trace);
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].0, 0,
            "the unrelated float remains on its original page"
        );
    }
}

#[test]
fn local_float_clearance_recomputes_the_ancestors_collapsed_margin() {
    const PARENT_MARGIN: f32 = 20.0;
    const CHILD_MARGIN: f32 = 30.0;
    const TOP_PADDING: f32 = 1.0;
    let initial_baseline = render("", "<p>Control</p>").pages[0][0].baseline;
    for (depth, wrap_float) in [(0, false), (2, false), (0, true), (2, true)] {
        for clear in ["none", "right", "left", "both"] {
            let trace = render(
                &format!(
                    "main{{padding-top:{TOP_PADDING}pt}}.float{{float:left}}section{{margin-top:{PARENT_MARGIN}pt;background:red}}p{{margin-top:{CHILD_MARGIN}pt;clear:{clear}}}"
                ),
                &format!(
                    "<main><section>{}<img class=float src='../assets/image-pattern.png'>{}{}<p>Cleared</p>{}</section></main>",
                    if wrap_float { "<div>" } else { "" },
                    if wrap_float { "</div>" } else { "" },
                    "<div>".repeat(depth),
                    "</div>".repeat(depth)
                ),
            );
            let paint = trace.display_lists[0]
                .iter()
                .find_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                        Some(paint)
                    }
                    _ => None,
                })
                .expect("the parent background is present");
            let clears = matches!(clear, "left" | "both");
            let margin = if clears { PARENT_MARGIN } else { CHILD_MARGIN };
            assert_pt(paint.rect.y, PAGE_LEFT + TOP_PADDING + margin);
            let images = image_paints(&trace);
            assert_eq!(images.len(), 1, "{clear}: one local float");
            assert_pt(images[0].1.rect.y, paint.rect.y);
            assert_pt(
                trace.pages[0][0].baseline,
                initial_baseline + TOP_PADDING + margin + if clears { FLOAT_HEIGHT } else { 0.0 },
            );
            assert_pt(
                paint.rect.height,
                LINE_HEIGHT + if clears { FLOAT_HEIGHT } else { 0.0 },
            );
        }
    }
}

#[test]
fn local_float_clearance_is_resolved_after_a_forced_parent_break() {
    const PARENT_MARGIN: f32 = 20.0;
    const CHILD_MARGIN: f32 = 30.0;
    const NEXT_PARENT_WIDTH: f32 = 84.0;
    let trace = render(
        &format!(
            "@page{{size:240pt 240pt;margin:36pt}}@page:first{{size:360pt 240pt}}section{{break-before:page;margin-top:{PARENT_MARGIN}pt;width:50%;background:red}}.float{{float:left;width:100%;height:auto}}.cleared{{clear:both;margin-top:{CHILD_MARGIN}pt}}"
        ),
        "<p>Before</p><section><div class=float>First second third</div><p class=cleared>Cleared</p></section>",
    );
    assert_eq!(trace.pages.len(), 2, "the parent has one forced page break");
    let parent = trace.display_lists[1]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => Some(paint),
            _ => None,
        })
        .expect("parent background after the break");
    assert_pt(parent.rect.y, PAGE_LEFT + PARENT_MARGIN);
    assert_pt(parent.rect.width, NEXT_PARENT_WIDTH);
    let lines = &trace.pages[1];
    assert_eq!(
        lines.len(),
        3,
        "float rewraps at the destination width before clearance"
    );
    let first = lines
        .iter()
        .find(|line| line.text.starts_with("First"))
        .expect("first float line");
    let cleared = lines
        .iter()
        .find(|line| line.text == "Cleared")
        .expect("cleared descendant");
    assert_pt(cleared.baseline - first.baseline, 2.0 * LINE_HEIGHT);
}

#[test]
fn deferred_floats_prevent_parent_margin_collapse_only_on_cleared_sides() {
    const PARENT_HEIGHT: f32 = 120.0;
    const PARENT_MARGIN: f32 = 20.0;
    const CHILD_MARGIN: f32 = 30.0;
    const SPACER_HEIGHT: f32 = 36.0;
    for height in ["50%", "60pt"] {
        for clear in ["both", "left", "right", "none"] {
            let trace = render(
                &format!(
                    "@page{{size:360pt 144pt;margin:36pt}}section{{height:{PARENT_HEIGHT}pt;margin-top:{PARENT_MARGIN}pt;background:red}}.float{{float:left;height:{height};break-inside:avoid}}.cleared{{clear:{clear};margin-top:{CHILD_MARGIN}pt;font-size:4pt;line-height:4pt;orphans:1;widows:1}}"
                ),
                &format!(
                    "<div style='height:{SPACER_HEIGHT}pt'></div><section><div class=float></div><p class=cleared>Cleared</p></section>"
                ),
            );
            let (parent_page, parent) = trace
                .display_lists
                .iter()
                .enumerate()
                .find_map(|(page, items)| {
                    items.iter().find_map(|item| match item {
                        DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                            Some((page, paint))
                        }
                        _ => None,
                    })
                })
                .expect("parent first fragment background");
            let clears = matches!(clear, "both" | "left");
            // The separated child margin and first line cannot fit in the initial 16pt.
            assert_eq!(
                parent_page,
                usize::from(clears),
                "{height}/{clear}: the parent uses its uncollapsed first-fragment size"
            );
            assert_pt(
                parent.rect.y,
                PAGE_LEFT
                    + if clears {
                        0.0
                    } else {
                        SPACER_HEIGHT + CHILD_MARGIN
                    },
            );
            let (page, line) = trace
                .pages
                .iter()
                .enumerate()
                .find_map(|(page, lines)| {
                    lines
                        .iter()
                        .find(|line| line.text == "Cleared")
                        .map(|line| (page, line))
                })
                .expect("descendant remains present");
            assert_eq!(
                page,
                usize::from(clears),
                "{height}/{clear}: only matching clearance follows the deferred float"
            );
            if clears {
                assert!(
                    line.baseline > PAGE_LEFT + PARENT_HEIGHT / 2.0,
                    "{height}/{clear}: the cleared line starts below the 60pt float"
                );
            }
        }
    }
}

#[test]
fn percentage_float_clearance_uses_the_parents_height_during_margin_resolution() {
    const PARENT_HEIGHT: f32 = 120.0;
    const PARENT_MARGIN: f32 = 20.0;
    const CHILD_MARGIN: f32 = 30.0;
    const TOP_PADDING: f32 = 1.0;
    let baseline = render("", "<p>Control</p>").pages[0][0].baseline;
    for replaced in [false, true] {
        let float = if replaced {
            "<img class=float src='../assets/image-pattern.png'>"
        } else {
            "<div class=float></div>"
        };
        let trace = render(
            &format!(
                "main{{padding-top:{TOP_PADDING}pt}}section{{height:{PARENT_HEIGHT}pt;margin-top:{PARENT_MARGIN}pt;background:red}}.float{{float:left;height:50%}}p{{clear:both;margin-top:{CHILD_MARGIN}pt}}"
            ),
            &format!("<main><section>{float}<p>Cleared</p></section></main>"),
        );
        let parent = trace.display_lists[0]
            .iter()
            .find_map(|item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => Some(paint),
                _ => None,
            })
            .expect("parent background");
        assert_pt(parent.rect.y, PAGE_LEFT + TOP_PADDING + PARENT_MARGIN);
        assert_pt(
            trace.pages[0][0].baseline,
            baseline + TOP_PADDING + PARENT_MARGIN + PARENT_HEIGHT / 2.0,
        );
    }
}

#[test]
fn repeated_table_header_floats_do_not_exclude_body_text() {
    const ROWS: usize = 8;
    let rows = (0..ROWS)
        .map(|index| format!("<tr><td>Row{index}First<br>Row{index}Last</td></tr>"))
        .collect::<String>();
    let css = "@page{size:360pt 144pt;margin:36pt}table{width:100%;border-spacing:0}th,td{padding:0;text-align:left;font-weight:normal} .float{float:left;width:18pt;height:18pt}";
    let body = |image: &str| {
        format!(
            "<table><thead><tr><th>{image}Header</th></tr></thead><tbody>{rows}</tbody></table>"
        )
    };
    let control = render(css, &body(""));
    let trace = render(
        css,
        &body("<img class=float src='../assets/image-pattern.png'>"),
    );
    assert!(trace.pages.len() > 1, "the fixture must repeat its header");
    assert_eq!(
        trace.pages.len(),
        control.pages.len(),
        "a header float must not change body pagination"
    );
    let images = image_paints(&trace);
    assert_eq!(
        images.len(),
        trace.pages.len(),
        "one float image per repeated header"
    );
    for (page, lines) in trace.pages.iter().enumerate() {
        assert_eq!(
            images
                .iter()
                .filter(|(image_page, _)| *image_page == page)
                .count(),
            1,
            "page {page}: exactly one header float"
        );
        let actual: Vec<_> = lines
            .iter()
            .filter(|line| line.text.starts_with("Row"))
            .collect();
        let expected: Vec<_> = control.pages[page]
            .iter()
            .filter(|line| line.text.starts_with("Row"))
            .collect();
        assert_eq!(actual.len(), expected.len(), "page {page}: body line count");
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert_eq!(actual.text, expected.text, "page {page}: source order");
            assert_pt(actual.x, expected.x);
            assert_pt(actual.baseline, expected.baseline);
        }
    }
}

#[test]
fn independent_blocks_avoid_floats_without_changing_their_used_width() {
    const NARROW_WIDTH: f32 = 120.0;
    for side in ["left", "right"] {
        for width in [NARROW_WIDTH, PAGE_RIGHT - PAGE_LEFT] {
            for (kind, body, rule) in [
                (
                    "context",
                    "<section class=avoid><p>Inside</p></section>",
                    "overflow:hidden",
                ),
                (
                    "image",
                    "<img class=avoid src='../assets/image-pattern.png'>",
                    "display:block;height:18pt",
                ),
                (
                    "table",
                    "<table class=avoid><tr><td>Inside</td></tr></table>",
                    "border-spacing:0",
                ),
            ] {
                let trace = render(
                    &format!(
                        ".float{{float:{side}}}.avoid{{width:{width}pt;background:red;{rule}}}td{{padding:0}}"
                    ),
                    &format!("<img class=float src='../assets/image-pattern.png'>{body}"),
                );
                let paint = trace.display_lists[0]
                    .iter()
                    .find_map(|item| match item {
                        DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                            Some(paint)
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("missing {side}/{width}/{kind} background"));
                let below = width > PAGE_RIGHT - PAGE_LEFT - FLOAT_WIDTH;
                assert_pt(paint.rect.width, width);
                assert_pt(
                    paint.rect.x,
                    PAGE_LEFT
                        + if side == "left" && !below {
                            FLOAT_WIDTH
                        } else {
                            0.0
                        },
                );
                assert_pt(
                    paint.rect.y,
                    PAGE_LEFT + if below { FLOAT_HEIGHT } else { 0.0 },
                );
                if kind == "image" {
                    let images = image_paints(&trace);
                    assert_eq!(images.len(), 2, "{side}/{width}: both images paint once");
                    assert_pt(images[1].1.rect.x, paint.rect.x);
                    assert_pt(images[1].1.rect.y, paint.rect.y);
                } else {
                    assert_eq!(
                        trace.pages[0].len(),
                        1,
                        "{side}/{width}/{kind}: one source line"
                    );
                    assert_eq!(trace.pages[0][0].text, "Inside");
                    assert_pt(trace.pages[0][0].x, paint.rect.x);
                }
            }
        }
    }
}

#[test]
fn float_avoidance_rechecks_placement_after_a_page_break() {
    let page_height = 2.0 * PAGE_LEFT + FLOAT_HEIGHT;
    let trace = render(
        &format!(
            "@page{{size:360pt {page_height}pt;margin:{PAGE_LEFT}pt}}.float{{float:left}}section{{overflow:hidden;background:red}}"
        ),
        "<img class=float src='../assets/image-pattern.png'><section><p>Next</p></section>",
    );
    assert_eq!(
        trace.pages.len(),
        2,
        "the full-width block cannot fit beside or below the float"
    );
    assert!(trace.pages[0].is_empty());
    assert_eq!(trace.pages[1].len(), 1);
    assert_eq!(trace.pages[1][0].text, "Next");
    assert_pt(trace.pages[1][0].x, PAGE_LEFT);
    let paint = trace.display_lists[1]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => Some(paint),
            _ => None,
        })
        .expect("the context paints on page two");
    assert_pt(paint.rect.y, PAGE_LEFT);
    assert_pt(paint.rect.width, PAGE_RIGHT - PAGE_LEFT);
}

#[test]
fn formatting_context_float_enclosure_obeys_height_constraints() {
    const MINIMUM_HEIGHT: f32 = 72.0;
    const MAXIMUM_HEIGHT: f32 = 30.0;
    let initial_baseline = render("", "<p>Control</p>").pages[0][0].baseline;
    for (constraint, height) in [
        (String::new(), FLOAT_HEIGHT),
        (format!("min-height:{MINIMUM_HEIGHT}pt"), MINIMUM_HEIGHT),
        (format!("max-height:{MAXIMUM_HEIGHT}pt"), MAXIMUM_HEIGHT),
        (format!("height:{MAXIMUM_HEIGHT}pt"), MAXIMUM_HEIGHT),
    ] {
        let trace = render(
            &format!(".float{{float:left}}section{{overflow:hidden;background:red;{constraint}}}"),
            "<section><img class=float src='../assets/image-pattern.png'></section><p>After</p>",
        );
        assert_eq!(trace.pages.len(), 1, "{constraint}: one page");
        assert_eq!(trace.pages[0].len(), 1, "{constraint}: one source line");
        assert_pt(trace.pages[0][0].baseline, initial_baseline + height);
        let backgrounds: Vec<_> = trace.display_lists[0]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => Some(paint),
                _ => None,
            })
            .collect();
        assert_eq!(backgrounds.len(), 1, "{constraint}: one parent background");
        assert_pt(backgrounds[0].rect.height, height);
    }
}

#[test]
fn inline_float_reflows_existing_text_without_starting_a_new_paragraph() {
    let trace = render(
        ".float{float:left}",
        "<p>Before <img class=float src='../assets/image-pattern.png'>after</p>",
    );
    assert_eq!(trace.pages.len(), 1);
    let lines = &trace.pages[0];
    assert_eq!(
        lines.len(),
        1,
        "inline float must not force a line or paragraph break"
    );
    assert_eq!(
        lines[0].text, "Before after",
        "source text must occur exactly once"
    );
    assert_pt(lines[0].x, PAGE_LEFT + FLOAT_WIDTH);
}

#[test]
fn inline_float_trials_rollback_when_fragment_rules_reject_lines() {
    const SPACER_HEIGHT: f32 = 36.0;
    for (orphans, body, expected) in [
        (
            2,
            "First<br>Second<br>Third<img class=float src='../assets/image-pattern.png'><br>Fourth",
            vec![vec!["First", "Second"], vec!["Third", "Fourth"]],
        ),
        (
            3,
            "First<img class=float src='../assets/image-pattern.png'><br>Second<br>Third",
            vec![vec![], vec!["First", "Second", "Third"]],
        ),
    ] {
        let trace = render(
            &format!(
                "@page{{size:360pt 144pt;margin:36pt}}.float{{float:left}}p{{orphans:{orphans};widows:2}}"
            ),
            &format!("<div style='height:{SPACER_HEIGHT}pt'></div><p>{body}</p>"),
        );
        let actual: Vec<_> = trace
            .pages
            .iter()
            .map(|page| {
                page.iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(
            actual, expected,
            "orphans={orphans}: each source line must occur once on its chosen page"
        );
        let images = image_paints(&trace);
        assert_eq!(
            images.len(),
            1,
            "orphans={orphans}: rejected trials must not leave image paint"
        );
        assert_eq!(
            images[0].0, 1,
            "orphans={orphans}: float belongs on page two"
        );
        assert_pt(images[0].1.rect.y, PAGE_LEFT);
        for line in &trace.pages[1] {
            assert_pt(line.x, PAGE_LEFT + FLOAT_WIDTH);
        }
    }
}

#[test]
fn inline_float_rollback_replays_the_accepted_prefix_exactly_once() {
    const SPACER_HEIGHT: f32 = 36.0;
    const FIRST_WIDTH: f32 = 30.0;
    let trace = render(
        &format!(
            "@page{{size:360pt 144pt;margin:36pt}}.float{{float:left}}.first{{width:{FIRST_WIDTH}pt;height:{LINE_HEIGHT}pt}}.second{{float:right}}"
        ),
        &format!(
            "<div style='height:{SPACER_HEIGHT}pt'></div><p>First<img class='float first' src='../assets/image-pattern.png'><br>Second<br>Third<img class='float second' src='../assets/image-pattern.png'><br>Fourth</p>"
        ),
    );
    assert_eq!(trace.pages.len(), 2);
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["First", "Second"]
    );
    assert_eq!(
        trace.pages[1]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Third", "Fourth"]
    );
    let images = image_paints(&trace);
    assert_eq!(
        images.len(),
        2,
        "one accepted float and one deferred float, with no trial duplicates"
    );
    assert_eq!((images[0].0, images[1].0), (0, 1));
    assert_pt(images[0].1.rect.x, PAGE_LEFT);
    assert_pt(images[0].1.rect.y, PAGE_LEFT + SPACER_HEIGHT);
    assert_pt(images[1].1.rect.x, PAGE_RIGHT - FLOAT_WIDTH);
    assert_pt(images[1].1.rect.y, PAGE_LEFT);
    assert_pt(trace.pages[0][0].x, PAGE_LEFT + FIRST_WIDTH);
    assert_pt(trace.pages[0][1].x, PAGE_LEFT);
}

#[test]
fn atomic_float_keeps_searching_when_the_next_page_is_also_obstructed() {
    let trace = render(
        "@page{size:360pt 144pt;margin:36pt}.wide{float:left;width:100%;height:auto}.float{float:left}",
        "<div class=wide><p>One<br>Two<br>Three<br>Four<br>Five<br>Six</p></div><img class=float src='../assets/image-pattern.png'><p>Body</p>",
    );
    assert_eq!(
        trace.pages.len(),
        3,
        "the image fits only after the earlier float's continuation"
    );
    let images = image_paints(&trace);
    assert_eq!(images.len(), 1);
    assert_eq!(
        images[0].0, 2,
        "page two has only 36pt below the earlier float"
    );
    assert_pt(images[0].1.rect.y, PAGE_LEFT);
    assert_pt(images[0].1.rect.height, FLOAT_HEIGHT);
    let text: Vec<_> = trace
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.as_str())
        .collect();
    for expected in ["One", "Two", "Three", "Four", "Five", "Six", "Body"] {
        assert_eq!(
            text.iter().filter(|line| **line == expected).count(),
            1,
            "{expected}: preserve each parallel source line once"
        );
    }
}

#[test]
fn automatically_moved_float_does_not_restore_its_truncated_margin() {
    const TOP_MARGIN: f32 = 7.0;
    const SPACER_HEIGHT: f32 = 36.0;
    let trace = render(
        &format!(
            "@page{{size:360pt 144pt;margin:36pt}}.float{{float:left}}.first{{margin-top:{TOP_MARGIN}pt;break-inside:avoid}}.second{{height:{LINE_HEIGHT}pt}}"
        ),
        &format!(
            "<div style='height:{SPACER_HEIGHT}pt'></div><div class='float first'><p>First</p></div><img class='float second' src='../assets/image-pattern.png'>"
        ),
    );
    let images = image_paints(&trace);
    assert_eq!(images.len(), 1, "the later float paints once");
    assert_eq!(
        images[0].0, 1,
        "the later float follows the deferred float onto page two"
    );
    assert_pt(images[0].1.rect.y, PAGE_LEFT);
    assert_pt(images[0].1.rect.x, PAGE_LEFT + FLOAT_WIDTH);
}

#[test]
fn shifted_float_continuations_keep_cloned_top_margins() {
    const TOP_MARGIN: f32 = 7.0;
    const WIDTH: f32 = 120.0;
    for decoration in ["slice", "clone"] {
        let trace = render(
            &format!(
                "@page{{size:240pt 144pt;margin:36pt}}@page:first{{size:360pt 144pt}}.float{{float:left;width:{WIDTH}pt;height:auto}}.second{{margin-top:{TOP_MARGIN}pt;box-decoration-break:{decoration}}}p{{orphans:1;widows:1}}"
            ),
            "<div class=float><p>FirstStart</p><p style='break-before:page'>FirstEnd</p></div><div class='float second'><p>SecondStart</p><p style='break-before:page'>SecondEnd</p></div>",
        );
        assert_eq!(
            trace.pages.len(),
            2,
            "{decoration}: both continuations fit page two"
        );
        let first = trace.pages[1]
            .iter()
            .find(|line| line.text == "FirstEnd")
            .expect("first continuation");
        let second = trace.pages[1]
            .iter()
            .find(|line| line.text == "SecondEnd")
            .expect("second continuation");
        assert_pt(
            second.baseline - first.baseline,
            LINE_HEIGHT
                + if decoration == "clone" {
                    TOP_MARGIN
                } else {
                    0.0
                },
        );
    }
}

#[test]
fn intermediate_float_fragments_keep_the_used_cloned_margin() {
    const PARENT_PADDING: f32 = 10.0;
    const TOP_MARGIN: f32 = 7.0;
    const BODY_LINES: usize = 40;
    let body = (0..BODY_LINES)
        .map(|index| format!("Body{index:02}"))
        .collect::<Vec<_>>()
        .join("<br>");
    let trace = render(
        &format!(
            "@page{{size:360pt 144pt;margin:36pt}}.outer{{padding-top:{PARENT_PADDING}pt;box-decoration-break:clone}}.float{{float:left;height:auto;margin-top:{TOP_MARGIN}pt;box-decoration-break:clone}}.body{{font-size:4pt;line-height:4pt}}p{{orphans:1;widows:1}}"
        ),
        &format!(
            "<div class=outer><div class=float><p>First</p><p style='break-before:page'>Middle</p><p style='break-before:page'>Last</p></div><p class=body>{body}</p></div>"
        ),
    );
    let first_body_line = trace.pages[1]
        .iter()
        .find(|line| line.text.starts_with("Body"))
        .expect("normal flow reaches the intermediate float page");
    assert_pt(first_body_line.x, PAGE_LEFT + FLOAT_WIDTH);
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .filter(|line| line.text.starts_with("Body"))
            .count(),
        BODY_LINES,
        "all small body lines remain present exactly once"
    );
}

#[test]
fn floated_image_margins_follow_the_kind_of_page_break() {
    const TOP_MARGIN: f32 = 7.0;
    const SPACER_HEIGHT: f32 = 36.0;
    for forced in [false, true] {
        let trace = render(
            &format!(
                "@page{{size:360pt 144pt;margin:36pt}}.float{{float:left;margin-top:{TOP_MARGIN}pt;break-before:{}}}",
                if forced { "page" } else { "auto" }
            ),
            &format!(
                "<div style='height:{SPACER_HEIGHT}pt'></div><img class=float src='../assets/image-pattern.png'><p>Body</p>"
            ),
        );
        let images = image_paints(&trace);
        assert_eq!(images.len(), 1, "forced={forced}: one floated image");
        assert_eq!(
            images[0].0, 1,
            "forced={forced}: the float moves to page two"
        );
        assert_pt(
            images[0].1.rect.y,
            PAGE_LEFT + if forced { TOP_MARGIN } else { 0.0 },
        );
    }
}

#[test]
fn nonreplaced_float_continuations_use_each_pages_width() {
    const FIRST_WIDTH: f32 = 144.0;
    const NEXT_WIDTH: f32 = 84.0;
    const NEXT_RIGHT: f32 = 204.0;
    for side in ["left", "right"] {
        let trace = render(
            &format!(
                "@page{{size:240pt 144pt;margin:36pt}}@page:first{{size:360pt 144pt}}.float{{float:{side};width:50%;height:auto;background:red}}p{{orphans:1;widows:1}}"
            ),
            "<div class=float><p>FloatFirst</p><p style='break-before:page'>FloatLast</p></div><p>BodyOne<br>BodyTwo<br>BodyThree<br>BodyFour<br>BodyFive<br>BodySix</p>",
        );
        assert_eq!(trace.pages.len(), 2, "{side}: exactly two physical pages");
        for (page, width, right, label) in [
            (0, FIRST_WIDTH, PAGE_RIGHT, "FloatFirst"),
            (1, NEXT_WIDTH, NEXT_RIGHT, "FloatLast"),
        ] {
            let paint = trace.display_lists[page]
                .iter()
                .find_map(|item| match item {
                    DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                        Some(paint)
                    }
                    _ => None,
                })
                .expect("float fragment background");
            assert_pt(paint.rect.width, width);
            let left = if side == "left" {
                PAGE_LEFT
            } else {
                right - width
            };
            assert_pt(paint.rect.x, left);
            let line = trace.pages[page]
                .iter()
                .find(|line| line.text == label)
                .expect("float source text on the correct page");
            assert_pt(line.x, left);
            let body = trace.pages[page]
                .iter()
                .find(|line| line.text == if page == 0 { "BodyOne" } else { "BodyFive" })
                .expect("parallel body text");
            assert_pt(
                body.x,
                if side == "left" {
                    PAGE_LEFT + width
                } else {
                    PAGE_LEFT
                },
            );
        }
        assert_eq!(
            trace.pages.iter().map(Vec::len).sum::<usize>(),
            8,
            "{side}: every source line exactly once"
        );
    }
}

#[test]
fn float_continuations_obey_prior_float_placement_on_narrower_pages() {
    const NARROW_CONTENT_WIDTH: f32 = 168.0;
    const FIXED_WIDTH: f32 = 120.0;
    for (width, expected_width) in [("30%", NARROW_CONTENT_WIDTH * 0.3), ("120pt", FIXED_WIDTH)] {
        for (side, second_side) in [
            ("left", "left"),
            ("right", "right"),
            ("left", "right"),
            ("right", "left"),
        ] {
            let trace = render(
                &format!(
                    "@page{{size:240pt 144pt;margin:36pt}}@page:first{{size:360pt 144pt}}.float{{float:{side};width:{width};height:auto}}.second{{float:{second_side}}}p{{orphans:1;widows:1}}"
                ),
                "<div class=float><p>FirstStart</p><p style='break-before:page'>FirstEnd</p></div><div class='float second'><p>SecondStart</p><p style='break-before:page'>SecondEnd</p></div>",
            );
            assert_eq!(
                trace.pages.len(),
                2,
                "{width}/{side}/{second_side}: float continuations fit on two pages"
            );
            let first = trace.pages[1]
                .iter()
                .find(|line| line.text == "FirstEnd")
                .expect("first continuation");
            let second = trace.pages[1]
                .iter()
                .find(|line| line.text == "SecondEnd")
                .expect("second continuation");
            let first_x = PAGE_LEFT
                + if side == "left" {
                    0.0
                } else {
                    NARROW_CONTENT_WIDTH - expected_width
                };
            assert_pt(first.x, first_x);
            let second_edge = PAGE_LEFT
                + if second_side == "left" {
                    0.0
                } else {
                    NARROW_CONTENT_WIDTH - expected_width
                };
            if 2.0 * expected_width <= NARROW_CONTENT_WIDTH {
                assert_pt(
                    second.x,
                    if side == second_side {
                        first_x
                            + if side == "left" {
                                expected_width
                            } else {
                                -expected_width
                            }
                    } else {
                        second_edge
                    },
                );
                assert_pt(second.baseline, first.baseline);
            } else {
                assert_pt(second.x, second_edge);
                assert_pt(second.baseline, first.baseline + LINE_HEIGHT);
            }
            assert_eq!(
                trace.pages.iter().map(Vec::len).sum::<usize>(),
                4,
                "{width}/{side}/{second_side}: each source line once"
            );
        }
    }
}

#[test]
fn continued_float_text_wraps_at_the_destination_width() {
    const CONTINUED_TEXT: &str = "Final one two three four five";
    for side in ["left", "right"] {
        let css = format!(
            "@page{{size:240pt 144pt;margin:36pt}}.float{{float:{side};width:50%;height:auto}}p{{orphans:1;widows:1}}"
        );
        let control = render(
            &css,
            &format!("<div class=float><p>{CONTINUED_TEXT}</p></div>"),
        );
        let trace = render(
            &(css + "@page:first{size:360pt 144pt}"),
            &format!(
                "<div class=float><p>Start</p><p style='break-before:page'>{CONTINUED_TEXT}</p></div>"
            ),
        );
        assert_eq!(trace.pages.len(), 2, "{side}: exactly two float fragments");
        assert!(
            control.pages[0].len() > 1,
            "the control must exercise wrapping"
        );
        assert_eq!(
            trace.pages[1].len(),
            control.pages[0].len(),
            "{side}: same line count at destination width"
        );
        for (actual, expected) in trace.pages[1].iter().zip(&control.pages[0]) {
            assert_eq!(
                actual.text, expected.text,
                "{side}: source text wraps at the destination width"
            );
            assert_pt(actual.x, expected.x);
            assert_pt(actual.baseline, expected.baseline);
        }
    }
}

#[test]
fn floated_tables_continue_at_the_destination_width_and_side() {
    const FIRST_WIDTH: f32 = 144.0;
    const NEXT_WIDTH: f32 = 84.0;
    const NEXT_RIGHT: f32 = 204.0;
    for side in ["left", "right"] {
        let trace = render(
            &format!(
                "@page{{size:240pt 144pt;margin:36pt}}@page:first{{size:360pt 144pt}}table{{float:{side};width:50%;border-spacing:0}}td{{padding:0}}p{{orphans:1;widows:1}}"
            ),
            "<table><tr><td>FirstOne<br>FirstTwo</td></tr><tr><td>MiddleOne<br>MiddleTwo</td></tr><tr><td>LastOne<br>LastTwo</td></tr></table><p>BodyOne<br>BodyTwo<br>BodyThree<br>BodyFour<br>BodyFive<br>BodySix</p>",
        );
        assert_eq!(
            trace.pages.len(),
            2,
            "{side}: table and body occupy two pages"
        );
        eprintln!(
            "{side}: {:?}",
            trace
                .pages
                .iter()
                .map(|lines| lines
                    .iter()
                    .map(|line| (&line.text, line.x, line.baseline))
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );
        for (page, width, right, label, body_label) in [
            (0, FIRST_WIDTH, PAGE_RIGHT, "FirstOne", "BodyOne"),
            (1, NEXT_WIDTH, NEXT_RIGHT, "LastOne", "BodyFive"),
        ] {
            let cell = trace.pages[page]
                .iter()
                .find(|line| line.text == label)
                .expect("table cell on its expected page");
            assert_pt(
                cell.x,
                if side == "left" {
                    PAGE_LEFT
                } else {
                    right - width
                },
            );
            let body = trace.pages[page]
                .iter()
                .find(|line| line.text == body_label)
                .expect("parallel body on its expected page");
            assert_pt(
                body.x,
                if side == "left" {
                    PAGE_LEFT + width
                } else {
                    PAGE_LEFT
                },
            );
        }
        assert_eq!(
            trace.pages.iter().map(Vec::len).sum::<usize>(),
            12,
            "{side}: table and body text exactly once"
        );
    }
}

#[test]
fn floated_table_captions_follow_the_wrapper_across_page_sizes() {
    const FIRST_FLOAT_LEFT: f32 = 180.0;
    const NEXT_FLOAT_LEFT: f32 = 120.0;
    for side in ["left", "right"] {
        for caption_side in ["top", "bottom"] {
            let trace = render(
                &format!(
                    "@page{{size:240pt 144pt;margin:36pt}}@page:first{{size:360pt 144pt}}table{{float:{side};width:50%;border-spacing:0}}td{{padding:0}}caption{{caption-side:{caption_side};text-align:left}}p{{orphans:1;widows:1}}"
                ),
                "<table><caption>Caption</caption><tr><td>FirstOne<br>FirstTwo</td></tr><tr><td>MiddleOne<br>MiddleTwo</td></tr><tr><td>LastOne<br>LastTwo</td></tr></table>",
            );
            assert_eq!(
                trace.pages.len(),
                2,
                "{side}/{caption_side}: two wrapper fragments"
            );
            eprintln!(
                "{side}/{caption_side}: {:?}",
                trace
                    .pages
                    .iter()
                    .map(|lines| lines
                        .iter()
                        .map(|line| (&line.text, line.x, line.baseline))
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            );
            for (page, lines) in trace.pages.iter().enumerate() {
                let expected_x = if side == "left" {
                    PAGE_LEFT
                } else if page == 0 {
                    FIRST_FLOAT_LEFT
                } else {
                    NEXT_FLOAT_LEFT
                };
                for line in lines {
                    assert_pt(line.x, expected_x);
                }
            }
            let labels: Vec<_> = trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect();
            let expected = if caption_side == "top" {
                vec![
                    "Caption",
                    "FirstOne",
                    "FirstTwo",
                    "MiddleOne",
                    "MiddleTwo",
                    "LastOne",
                    "LastTwo",
                ]
            } else {
                vec![
                    "FirstOne",
                    "FirstTwo",
                    "MiddleOne",
                    "MiddleTwo",
                    "LastOne",
                    "LastTwo",
                    "Caption",
                ]
            };
            assert_eq!(
                labels, expected,
                "{side}/{caption_side}: caption and cell source order"
            );
        }
    }
}

#[test]
fn atomic_float_uses_the_dimensions_of_the_page_it_moves_to() {
    const NEXT_PAGE_WIDTH: f32 = 240.0;
    const SPACER_HEIGHT: f32 = 36.0;
    let expected_width = (NEXT_PAGE_WIDTH - 2.0 * PAGE_LEFT) / 2.0;
    for side in ["left", "right"] {
        let trace = render(
            &format!(
                "@page{{size:{NEXT_PAGE_WIDTH}pt 144pt;margin:36pt}}@page:first{{size:360pt 144pt}}.float{{float:{side};width:50%}}"
            ),
            &format!(
                "<div style='height:{SPACER_HEIGHT}pt'></div><img class=float src='../assets/image-pattern.png'><p>Body</p>"
            ),
        );
        assert_eq!(
            trace.pages.len(),
            2,
            "{side}: image moves to the narrower second page"
        );
        assert_eq!(
            trace.pages[0][0].text, "Body",
            "normal flow stays on its original page"
        );
        let images = image_paints(&trace);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].0, 1);
        assert_pt(images[0].1.rect.width, expected_width);
        assert_pt(
            images[0].1.rect.x,
            if side == "left" {
                PAGE_LEFT
            } else {
                NEXT_PAGE_WIDTH - PAGE_LEFT - expected_width
            },
        );
        assert_pt(images[0].1.rect.y, PAGE_LEFT);
        assert_pt(images[0].1.rect.height, FLOAT_HEIGHT);
    }
}

#[test]
fn later_floats_cannot_return_to_a_page_before_an_earlier_float() {
    const SPACER_HEIGHT: f32 = 36.0;
    const SECOND_WIDTH: f32 = 30.0;
    let trace = render(
        &format!(
            "@page{{size:360pt 144pt;margin:36pt}}.float{{float:left}}.second{{float:right;width:{SECOND_WIDTH}pt;height:{LINE_HEIGHT}pt}}"
        ),
        &format!(
            "<div style='height:{SPACER_HEIGHT}pt'></div><img class=float src='../assets/image-pattern.png'><img class='float second' src='../assets/image-pattern.png'><p>Body</p>"
        ),
    );
    assert_eq!(trace.pages.len(), 2);
    assert_eq!(
        trace.pages[0].len(),
        1,
        "surrounding normal flow stays on page one"
    );
    assert_eq!(trace.pages[0][0].text, "Body");
    let images = image_paints(&trace);
    assert_eq!(images.len(), 2, "each source image paints once");
    assert!(
        images.iter().all(|(page, _)| *page == 1),
        "both floats must start on page two: {:?}",
        images
            .iter()
            .map(|(page, image)| (*page, image.rect))
            .collect::<Vec<_>>()
    );
    assert_pt(images[0].1.rect.x, PAGE_LEFT);
    assert_pt(images[1].1.rect.x, PAGE_RIGHT - SECOND_WIDTH);
    for (_, image) in images {
        assert_pt(image.rect.y, PAGE_LEFT);
    }
}

#[test]
fn a_float_continuation_does_not_move_later_floats_past_its_start_page() {
    const SECOND_WIDTH: f32 = 30.0;
    for (before, page) in [("auto", 0), ("page", 1)] {
        let trace = render(
            &format!(
                "@page{{size:360pt 144pt;margin:36pt}}.float{{float:left;height:auto}}.second{{float:right;width:{SECOND_WIDTH}pt;height:{LINE_HEIGHT}pt;break-before:{before}}}"
            ),
            "<div class=float><p>First</p><p style='break-before:page'>Next</p></div><img class='float second' src='../assets/image-pattern.png'><p>Body</p>",
        );
        assert_eq!(trace.pages.len(), 2);
        assert!(trace.pages[0].iter().any(|line| line.text == "First"));
        assert!(trace.pages[1].iter().any(|line| line.text == "Next"));
        let images = image_paints(&trace);
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].0, page,
            "source order uses the first fragment; an explicit break is consumed once"
        );
        assert_pt(images[0].1.rect.x, PAGE_RIGHT - SECOND_WIDTH);
        assert_pt(images[0].1.rect.y, PAGE_LEFT);
    }
}

#[test]
fn deferred_inline_image_float_moves_to_next_page_after_first_line() {
    const PARAGRAPH_WIDTH: f32 = 100.0;
    let page_height = 2.0 * PAGE_LEFT + FLOAT_HEIGHT;
    let trace = render(
        &format!(
            "@page{{size:360pt {page_height}pt;margin:{PAGE_LEFT}pt}}p{{width:{PARAGRAPH_WIDTH}pt}}.float{{float:left}}"
        ),
        "<p>Before <img class=float src='../assets/image-pattern.png'>after</p>",
    );
    assert_eq!(
        trace.pages.len(),
        2,
        "the image cannot fit below the first line"
    );
    assert_eq!(trace.pages[0].len(), 1, "the source line stays on page one");
    assert_eq!(trace.pages[0][0].text, "Before after");
    let images = image_paints(&trace);
    assert_eq!(images.len(), 1, "the deferred image paints exactly once");
    assert_eq!(images[0].0, 1, "the atomic image moves to page two");
    assert_pt(images[0].1.rect.y, PAGE_LEFT);
    assert_pt(images[0].1.rect.height, FLOAT_HEIGHT);
}

#[test]
fn inline_float_respects_existing_line_content_and_direction() {
    const NARROW_PARAGRAPH: f32 = 100.0;
    for (direction, side, width, deferred) in [
        ("ltr", "left", PAGE_RIGHT - PAGE_LEFT, false),
        ("rtl", "right", PAGE_RIGHT - PAGE_LEFT, false),
        ("ltr", "left", NARROW_PARAGRAPH, true),
    ] {
        let trace = render(
            &format!("p{{direction:{direction};width:{width}pt}}.float{{float:{side}}}"),
            "<p>Before <img class=float src='../assets/image-pattern.png'>after</p>",
        );
        assert_eq!(
            trace.pages[0].len(),
            1,
            "{direction}/{width}: text remains on one line"
        );
        let line = &trace.pages[0][0];
        assert_eq!(line.text, "Before after");
        let images: Vec<_> = trace.display_lists[0]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Image(image) => Some(image),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 1);
        assert_pt(
            images[0].rect.y,
            PAGE_LEFT + if deferred { LINE_HEIGHT } else { 0.0 },
        );
        if side == "left" {
            assert_pt(images[0].rect.x, PAGE_LEFT);
            assert_pt(line.x, PAGE_LEFT + if deferred { 0.0 } else { FLOAT_WIDTH });
        } else {
            assert_pt(images[0].rect.x, PAGE_RIGHT - FLOAT_WIDTH);
            assert_pt(line.x + line.width, PAGE_RIGHT - FLOAT_WIDTH);
        }
    }
}

#[test]
fn nonpositive_float_outer_heights_do_not_shorten_lines() {
    for (height, bottom) in [
        (0.0, 0.0),
        (LINE_HEIGHT, LINE_HEIGHT),
        (LINE_HEIGHT, 2.0 * LINE_HEIGHT),
    ] {
        let trace = render(
            &format!(".float{{float:left;height:{height}pt;margin-bottom:-{bottom}pt}}"),
            "<div class=float>Float</div><p>First</p><p>Second</p>",
        );
        for label in ["First", "Second"] {
            let line = trace.pages[0]
                .iter()
                .find(|line| line.text == label)
                .unwrap_or_else(|| panic!("missing {label} for height {height}"));
            assert_pt(line.x, PAGE_LEFT);
        }
    }
}

#[test]
fn a_body_break_reuses_the_next_page_allocated_by_a_float() {
    for boundary in ["page", "left", "right"] {
        let trace = render(
            "@page{size:360pt 144pt;margin:36pt}.float{float:left;height:auto}p{orphans:1;widows:1}",
            &format!(
                "<div class=float><p>FloatFirst</p><p style='break-before:page'>FloatNext</p></div><p>BodyFirst</p><p style='break-before:{boundary}'>BodyNext</p>"
            ),
        );
        let expected_page = if boundary == "right" { 2 } else { 1 };
        let (page, body) = trace
            .pages
            .iter()
            .enumerate()
            .find_map(|(page, lines)| {
                lines
                    .iter()
                    .find(|line| line.text == "BodyNext")
                    .map(|line| (page, line))
            })
            .expect("body continuation");
        assert_eq!(
            page, expected_page,
            "{boundary}: page allocation must not advance the surrounding source cursor"
        );
        assert_eq!(
            trace.pages.len(),
            expected_page + 1,
            "{boundary}: no extra page"
        );
        if expected_page == 1 {
            let float = trace.pages[page]
                .iter()
                .find(|line| line.text == "FloatNext")
                .expect("parallel float continuation");
            assert_pt(body.baseline, float.baseline);
            assert_pt(body.x, PAGE_LEFT + FLOAT_WIDTH);
        } else {
            assert_pt(body.x, PAGE_LEFT);
        }
        assert_eq!(
            trace.pages.iter().map(Vec::len).sum::<usize>(),
            4,
            "{boundary}: each source line once"
        );
    }
}

#[test]
fn a_forced_break_inside_a_float_preserves_both_parallel_flows() {
    let trace = render(
        "@page{size:360pt 144pt;margin:36pt}.float{float:left;height:auto}",
        "<div class=float><p>FloatFirst</p><p style='break-before:page'>FloatNext</p></div><p>BodyFirst</p><p>BodyNext</p>",
    );
    assert_eq!(
        trace.pages.len(),
        2,
        "the float continuation must allocate its page"
    );
    for (page, expected) in [
        (0, vec!["BodyFirst", "BodyNext", "FloatFirst"]),
        (1, vec!["FloatNext"]),
    ] {
        let mut labels = trace.pages[page]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        labels.sort_unstable();
        assert_eq!(
            labels, expected,
            "page {page}: each parallel source must paint exactly once"
        );
    }
    for label in ["BodyFirst", "BodyNext"] {
        assert_pt(
            trace.pages[0]
                .iter()
                .find(|line| line.text == label)
                .unwrap()
                .x,
            PAGE_LEFT + FLOAT_WIDTH,
        );
    }
    assert_pt(trace.pages[1][0].x, PAGE_LEFT);
}

#[test]
fn cleared_image_float_uses_the_destination_page_geometry() {
    const DESTINATION_WIDTH: f32 = 168.0;
    for side in ["left", "right"] {
        let trace = render(
            &format!(
                "@page{{size:240pt 144pt;margin:36pt}}@page:first{{size:360pt 144pt}}.prior{{float:left;width:60pt}}img{{float:{side};clear:both;width:50%;height:54pt}}p{{orphans:1;widows:1}}"
            ),
            "<div class=prior><p>First</p><p style='break-before:page'>Last</p></div><img src='../assets/image-pattern.png'>",
        );
        let images = image_paints(&trace);
        assert_eq!(images.len(), 1, "{side}: cleared image paints once");
        assert_eq!(
            images[0].0, 1,
            "{side}: image clears the final float fragment"
        );
        assert_pt(images[0].1.rect.width, DESTINATION_WIDTH / 2.0);
        assert_pt(images[0].1.rect.height, FLOAT_HEIGHT);
        assert_pt(images[0].1.rect.y, PAGE_LEFT + LINE_HEIGHT);
        assert_pt(
            images[0].1.rect.x,
            PAGE_LEFT
                + if side == "right" {
                    DESTINATION_WIDTH / 2.0
                } else {
                    0.0
                },
        );
    }
}

#[test]
fn floats_that_fit_a_fresh_page_move_before_fragmenting() {
    const PAGE_MARGIN: f32 = 20.0;
    const PAGE_CONTENT_HEIGHT: f32 = 160.0;
    for side in ["left", "right"] {
        for height in ["height:150pt", "height:auto"] {
            let body = format!(
                "<div style='height:100pt'></div><div class=float>{}</div><p>Beside</p><p style='clear:both'>After</p>",
                (0..8)
                    .map(|index| format!("<p>Float{index}</p>"))
                    .collect::<String>()
            );
            let trace = render(
                &format!(
                    "@page{{size:240pt 200pt;margin:{PAGE_MARGIN}pt}}.float{{float:{side};width:100pt;{height};background:yellow}}p{{orphans:1;widows:1}}"
                ),
                &body,
            );
            let (page, first) = trace
                .pages
                .iter()
                .enumerate()
                .find_map(|(page, lines)| {
                    lines
                        .iter()
                        .find(|line| line.text == "Float0")
                        .map(|line| (page, line))
                })
                .expect("first float line must survive");
            assert_eq!(
                page, 1,
                "{side}/{height}: a float fitting a fresh page must move as a whole"
            );
            assert!(
                first.baseline < PAGE_MARGIN + LINE_HEIGHT,
                "{side}/{height}: first baseline {}",
                first.baseline
            );
            for index in 0..8 {
                assert_eq!(
                    trace.pages[1]
                        .iter()
                        .filter(|line| line.text == format!("Float{index}"))
                        .count(),
                    1,
                    "{side}/{height}: float line {index} must appear exactly once on its destination page"
                );
            }
            assert!(
                trace.pages[0].iter().any(|line| line.text == "Beside"),
                "{side}/{height}: surrounding flow must remain on its own page"
            );
            let paint = trace.display_lists[1]
                .iter()
                .find_map(|item| match item {
                    DisplayItem::Box(paint) if paint.has_ink() => Some(paint),
                    _ => None,
                })
                .expect("float background");
            assert_pt(paint.rect.y, PAGE_MARGIN);
            assert!(
                paint.rect.y + paint.rect.height <= PAGE_MARGIN + PAGE_CONTENT_HEIGHT,
                "{side}/{height}: float background must fit its destination page: {:?}",
                paint.rect
            );
        }
    }
}

#[test]
fn deferred_float_remeasures_before_splitting_on_a_shorter_narrower_page() {
    const DESTINATION_LEFT: f32 = 20.0;
    const DESTINATION_WIDTH: f32 = 140.0;
    const FLOAT_LINES: usize = 8;
    for side in ["left", "right"] {
        let trace = render(
            &format!(
                "@page{{size:180pt 140pt;margin:20pt}}@page:first{{size:240pt 200pt}}.float{{float:{side};width:50%;height:150pt;background:yellow}}p{{orphans:1;widows:1}}"
            ),
            &format!(
                "<div style='height:100pt'></div><div class=float>{}</div>",
                (0..FLOAT_LINES)
                    .map(|index| format!("<p>Line{index}</p>"))
                    .collect::<String>()
            ),
        );
        assert!(
            trace.pages[0].is_empty(),
            "{side}: float must leave its first page"
        );
        assert_eq!(
            trace.pages.len(),
            3,
            "{side}: the shorter destination must split the float"
        );
        let labels: Vec<_> = trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(
            labels,
            (0..FLOAT_LINES)
                .map(|index| format!("Line{index}"))
                .collect::<Vec<_>>(),
            "{side}: every float line must survive in source order"
        );
        for page in &trace.pages[1..] {
            for line in page {
                assert_pt(
                    line.x,
                    DESTINATION_LEFT
                        + if side == "right" {
                            DESTINATION_WIDTH / 2.0
                        } else {
                            0.0
                        },
                );
            }
        }
    }
}
