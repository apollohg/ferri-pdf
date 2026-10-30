mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{DisplayItem, LayoutTrace, render_html};

fn render(body: &str, css: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}p{{margin:0}}{css}</style>{body}"),
        &local_options(),
    )
    .expect("positioned document must render")
    .layout
    .unwrap()
}

#[test]
fn nested_relative_offsets_move_paint_without_moving_following_flow() {
    let body = "<div class=outer><p>Outer</p><div class=inner>Inner</div></div><p>After</p>";
    let css = ".outer{border:2pt solid navy}.inner{background:#ddeeff}";
    let normal = render(body, css);
    let shifted = render(
        body,
        &format!(
            "{css}.outer{{position:relative;left:20pt;top:10pt}}.inner{{position:relative;right:5pt;bottom:3pt}}"
        ),
    );
    for (index, (dx, dy)) in [(20.0, 10.0), (15.0, 7.0), (0.0, 0.0)]
        .into_iter()
        .enumerate()
    {
        let before = &normal.pages[0][index];
        let after = &shifted.pages[0][index];
        assert!(
            (after.x - before.x - dx).abs() < 0.001,
            "{} horizontal offset",
            after.text
        );
        assert!(
            (after.baseline - before.baseline - dy).abs() < 0.001,
            "{} vertical offset",
            after.text
        );
    }
    let boxes = |trace: &LayoutTrace| {
        trace.display_lists[0]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Box(paint) => Some(paint.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    for (index, (before, after)) in boxes(&normal).iter().zip(boxes(&shifted)).enumerate() {
        assert_eq!(
            before.rect.height, after.rect.height,
            "box {index} keeps normal extent"
        );
    }
}

#[test]
fn relative_fragments_keep_normal_page_assignment() {
    let body = format!(
        "<div>{}</div><p>After</p>",
        (0..30).map(|n| format!("Line{n}<br>")).collect::<String>()
    );
    let css = "div{border:2pt solid navy;background:#ddeeff}";
    let normal = render(&body, css);
    let shifted = render(
        &body,
        &format!("{css}div{{position:relative;left:10pt;top:12pt}}"),
    );
    assert_eq!(normal.pages.len(), shifted.pages.len());
    for (page, (before, after)) in normal.pages.iter().zip(&shifted.pages).enumerate() {
        assert_eq!(
            before.len(),
            after.len(),
            "page {page} retains line allocation"
        );
        for (before, after) in before.iter().zip(after) {
            let dy = if before.text == "After" { 0.0 } else { 12.0 };
            assert!(
                (after.baseline - before.baseline - dy).abs() < 0.001,
                "page {page}: {}",
                after.text
            );
        }
    }
}

#[test]
fn positioned_content_paints_above_later_normal_backgrounds() {
    let trace = render(
        "<p class=shifted>Above</p><p class=cover>Below</p>",
        ".shifted{position:relative;top:18pt}.cover{background:#ffeecc}",
    );
    let list = &trace.display_lists[0];
    let cover = list
        .iter()
        .position(|item| matches!(item, DisplayItem::Box(_)))
        .unwrap();
    let text = |name| {
        list.iter().position(|item| matches!(item, DisplayItem::Text{line} if trace.pages[0][*line].text == name)).unwrap()
    };
    assert!(cover < text("Above"));
    assert!(
        text("Below") < text("Above"),
        "positioned content paints after all normal-flow content"
    );
}

#[test]
fn percentage_offsets_use_containing_dimensions_and_direction() {
    let body =
        "<section><div class=child>Child</div></section><article><div>Indefinite</div></article>";
    let css = "section{width:200pt;height:100pt}.child{direction:rtl}";
    let normal = render(body, css);
    let shifted = render(
        body,
        &format!(
            "{css}.child{{position:relative;left:10%;right:20%;top:20%;bottom:30%}}article div{{position:relative;left:10%;top:50%}}"
        ),
    );
    let before = &normal.pages[0];
    let after = &shifted.pages[0];
    assert!(
        (after[0].x - before[0].x - 20.0).abs() < 0.001,
        "containing block direction chooses left even for an RTL child"
    );
    assert!((after[0].baseline - before[0].baseline - 20.0).abs() < 0.001);
    assert!((after[1].x - before[1].x - 28.8).abs() < 0.001);
    assert_eq!(
        after[1].baseline, before[1].baseline,
        "percentage top is auto with an indefinite containing height"
    );
}

#[test]
fn outside_list_marker_follows_its_first_positioned_line() {
    let body = "<ol><li><div>Item</div></li></ol>";
    let normal = render(body, "ol{margin:0;padding-left:30pt}");
    let shifted = render(
        body,
        "ol{margin:0;padding-left:30pt}li div{position:relative;left:10pt;top:12pt}",
    );
    for (before, after) in normal.pages[0].iter().zip(&shifted.pages[0]) {
        assert!(
            (after.x - before.x - 10.0).abs() < 0.001,
            "{} follows first-line horizontal displacement",
            after.text
        );
        assert!(
            (after.baseline - before.baseline - 12.0).abs() < 0.001,
            "{} follows first-line vertical displacement",
            after.text
        );
    }
}

#[test]
fn off_page_relative_ink_does_not_collapse_flow_pages() {
    let body = format!(
        "<div>{}</div><p>After</p>",
        (0..30).map(|n| format!("Line{n}<br>")).collect::<String>()
    );
    let normal = render(&body, "");
    for top in [-1000, 1000] {
        let shifted = render(&body, &format!("div{{position:relative;top:{top}pt}}"));
        assert_eq!(
            shifted.pages.len(),
            normal.pages.len(),
            "top:{top} must retain normal-flow page count"
        );
        let after = shifted.pages.last().unwrap().last().unwrap();
        assert_eq!(after.text, "After");
        assert_eq!(
            after.baseline,
            normal.pages.last().unwrap().last().unwrap().baseline
        );
    }
}

/// The baseline sits this far below pdftotext's glyph-box top for the 12pt profile font.
const PROFILE_ASCENT_PT: f32 = 11.136;
const POSITION_TOLERANCE_PT: f32 = 0.001;

fn render_flow(body: &str, css: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}{css}</style>{body}"),
        &local_options(),
    )
    .expect("positioned document must render")
    .layout
    .unwrap()
}
fn find_line(trace: &LayoutTrace, text: &str) -> (usize, f32, f32) {
    trace
        .pages
        .iter()
        .enumerate()
        .find_map(|(page, lines)| {
            lines
                .iter()
                .find(|line| line.text == text)
                .map(|line| (page, line.x, line.baseline))
        })
        .unwrap_or_else(|| panic!("{text} must be drawn"))
}
fn assert_line(trace: &LayoutTrace, text: &str, page: usize, x: f32, glyph_top: f32) {
    let (actual_page, actual_x, baseline) = find_line(trace, text);
    let expected_baseline = glyph_top + PROFILE_ASCENT_PT;
    assert!(
        actual_page == page
            && (actual_x - x).abs() < POSITION_TOLERANCE_PT
            && (baseline - expected_baseline).abs() < POSITION_TOLERANCE_PT,
        "{text}: page {actual_page} x {actual_x} baseline {baseline}; expected page {page} x {x} baseline {expected_baseline}"
    );
}

#[test]
fn absolute_boxes_resolve_against_their_anchor_page_fragment() {
    let lines = |range: std::ops::Range<u32>| {
        range
            .map(|i| format!("<p>Line {i}</p>"))
            .collect::<String>()
    };
    let body = format!(
        "<div class='c'><div class='a'>FirstAbs</div>{}<div class='m'>MiddleAbs</div>{}<div class='b'>BottomAbs</div></div>",
        lines(1..17),
        lines(17..41)
    );
    let trace = render_flow(
        &body,
        ".c{position:relative;border:1pt solid navy}.a{position:absolute;top:50pt;left:0;background:#fea}.m{position:absolute;top:50pt;left:60pt}.b{position:absolute;bottom:2pt;right:2pt}",
    );
    let flow = render_flow(
        &format!("<div class=c>{}</div>", lines(1..41)),
        ".c{position:relative;border:1pt solid navy}",
    );
    assert_eq!(
        trace.pages.len(),
        flow.pages.len(),
        "positioned boxes never add pages"
    );
    for (page, count) in [13, 14, 13].into_iter().enumerate() {
        assert_eq!(
            trace.pages[page]
                .iter()
                .filter(|line| line.text.starts_with("Line "))
                .count(),
            count,
            "page {page}: sliced borders and adjoining margins do not consume continuation space"
        );
    }
    assert_eq!(trace.pages.len(), 3);
    assert_line(&trace, "FirstAbs", 0, 37.0, 89.0183);
    assert_line(&trace, "MiddleAbs", 1, 97.0, 88.0183);
    assert_line(&trace, "BottomAbs", 2, 254.9062, 408.0183);
}

#[test]
fn absolute_static_positions_follow_inline_and_block_origins() {
    let trace = render_flow(
        "<div class='c'>Flow text <span class='a'>InlineAbs</span>after words</div><div class='c'>Flow <div class='a'>BlockAbs</div>more</div>",
        ".c{position:relative}.a{position:absolute;background:#fea}",
    );
    assert_eq!(
        trace.pages[0][0].text, "Flow text after words",
        "an out-of-flow span does not split its line"
    );
    assert_line(&trace, "InlineAbs", 0, 94.7051, 38.0183);
    assert_line(&trace, "BlockAbs", 0, 36.0, 74.0183);
}

#[test]
fn absolute_content_beyond_the_page_is_dropped_without_new_pages() {
    let trace = render_flow(
        "<div class='c'><p>One</p><div class='a'>Far</div></div>",
        ".c{position:relative}.a{position:absolute;top:500pt;left:0}",
    );
    assert_eq!(trace.pages.len(), 1);
    assert!(
        trace.pages[0].iter().all(|line| line.text != "Far"),
        "positioned content below the page is dropped"
    );
}

#[test]
fn absolute_boxes_taller_than_the_page_break_across_the_pages_after_their_position() {
    // CSS Position 3 §3.5.3: the box is positioned as if the flow were continuous, then
    // broken over fragmentainers. 20pt + 21 lines of 18pt fill the profile's 408pt page area.
    const LINES: usize = 30;
    const TOP_PT: f32 = 20.0;
    const LINE_PT: f32 = 18.0;
    const FIRST_PAGE_LINES: usize = 21;
    let lines = (0..LINES)
        .map(|line| format!("<div>Abs {line}</div>"))
        .collect::<String>();
    let trace = render_flow(
        &format!("<div class='c'><p>Flow</p><div class='a'>{lines}</div></div>"),
        &format!(".c{{position:relative}}.a{{position:absolute;top:{TOP_PT}pt;left:0}}"),
    );
    let pages: Vec<Vec<&str>> = trace
        .pages
        .iter()
        .map(|page| page.iter().map(|line| line.text.as_str()).collect())
        .collect();
    let expected: Vec<String> = (0..LINES).map(|line| format!("Abs {line}")).collect();
    assert_eq!(
        pages.len(),
        2,
        "the box continues on one more page: {pages:?}"
    );
    assert_eq!(
        pages[0][1..],
        expected[..FIRST_PAGE_LINES],
        "the first page holds the flow and the lines that fit below the box's top"
    );
    assert_eq!(
        pages[1],
        expected[FIRST_PAGE_LINES..],
        "the rest continues at the top of the next page"
    );
    let first = trace.pages[0][1].baseline;
    let continued = trace.pages[1][0].baseline;
    assert!(
        (continued - (first - TOP_PT)).abs() < 0.001,
        "the continuation starts at the page area's top: first baseline {first}, continued {continued}"
    );
    for (index, line) in trace.pages[0][1..].iter().enumerate() {
        assert!(
            (line.baseline - (first + LINE_PT * index as f32)).abs() < 0.001,
            "{} sits {LINE_PT}pt below the line before it",
            line.text
        );
    }
}

#[test]
fn absolute_boxes_with_auto_insets_self_align_in_their_static_position_rectangle() {
    // CSS Position 3 §3.5.1: end alignment ends the box where its static-position rectangle
    // ends, and center alignment centres it there inside the containing block. The 300pt by
    // 200pt containing block starts at the page area's 36pt origin; the static-position
    // rectangle spans the second 200pt line box, 40pt in from the inline start, 20pt down.
    let body = "<div class=cb><div class=st></div><div class=st><div class=a></div></div></div>";
    let base = ".cb{position:relative;width:300pt;height:200pt}.st{width:200pt;height:20pt}.a{position:absolute;width:40pt;height:20pt;background:rgb(255,0,0)}";
    for (css, expected) in [
        (".st{margin-left:40pt}", [76.0, 56.0]),
        (".st{margin-left:40pt}.a{justify-self:end}", [236.0, 56.0]),
        (
            ".st{margin-left:40pt}.a{justify-self:center}",
            [156.0, 56.0],
        ),
        (".st{margin-left:40pt}.a{align-self:end}", [76.0, 36.0]),
        (".st{margin-left:40pt}.a{align-self:center}", [76.0, 46.0]),
        (
            ".cb{direction:rtl}.st{margin-right:40pt}.a{justify-self:end}",
            [96.0, 56.0],
        ),
        (
            ".cb{direction:rtl}.st{margin-right:40pt}.a{justify-self:start}",
            [256.0, 56.0],
        ),
    ] {
        let trace = render_flow(body, &format!("{base}{css}"));
        let rect = common::painted_box(&trace, [255, 0, 0]);
        assert!(
            (rect.x - expected[0]).abs() < 0.001 && (rect.y - expected[1]).abs() < 0.001,
            "{css}: expected the box at {expected:?}, got {rect:?}"
        );
    }
}

#[test]
fn absolute_boxes_with_auto_insets_shrink_to_their_static_position_containing_block() {
    // CSS Position 3 §3.5.1, §4.3: with both insets auto, an automatic width fits the content
    // to the inset-modified containing block the self-alignment derives from the
    // static-position rectangle, which spans 40pt to 240pt of the 300pt containing block at
    // the page area's 36pt origin. Two 150pt atomic inlines need 300pt on one line.
    let body = "<div class=cb><div class=st><div class=a><i></i><i></i></div></div></div>";
    let base = ".cb{position:relative;width:300pt;height:200pt}.st{width:200pt;height:20pt;margin-left:40pt}.a{position:absolute;background:rgb(255,0,0)}i{display:inline-block;width:150pt;height:10pt}";
    for (css, expected_x, expected_width) in [
        ("", 76.0, 260.0),
        (".a{justify-self:end}", 36.0, 240.0),
        (".a{justify-self:center}", 36.0, 280.0),
        (
            ".cb{direction:rtl}.st{margin:0 40pt 0 0}.a{justify-self:end}",
            96.0,
            240.0,
        ),
    ] {
        let trace = render_flow(body, &format!("{base}{css}"));
        let rect = common::painted_box(&trace, [255, 0, 0]);
        assert!(
            (rect.x - expected_x).abs() < 0.001 && (rect.width - expected_width).abs() < 0.001,
            "{css}: expected the box at x {expected_x} with width {expected_width}, got {rect:?}"
        );
    }
}

#[test]
fn stacking_contexts_paint_negative_flow_and_positive_layers_in_order() {
    let trace = render_flow(
        "<div class='c'><div class='a'>Above</div><div class='b'>Below</div><div class='n'>Negative</div><p class='flow'>Flow</p></div>",
        ".c{position:relative;height:80pt}.a,.b,.n{position:absolute;width:120pt;height:40pt}.a{background:#ffeeaa;z-index:2}.b{background:#99ccff;z-index:1}.n{background:#ff9999;z-index:-1}.flow{position:relative;background:#ccffcc}",
    );
    let order: Vec<[u8; 3]> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => {
                Some(paint.decoration.background.rgb)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        [
            [0xff, 0x99, 0x99],
            [0xcc, 0xff, 0xcc],
            [0x99, 0xcc, 0xff],
            [0xff, 0xee, 0xaa]
        ],
        "negative z-index, then positioned flow with z auto, then z 1 and z 2"
    );
}

#[test]
fn fixed_boxes_repeat_on_every_page() {
    let body = format!(
        "<div class='f'>Fixed</div>{}",
        (1..41)
            .map(|i| format!("<p>Para {i}</p>"))
            .collect::<String>()
    );
    let trace = render_flow(&body, ".f{position:fixed;top:0;right:0}");
    assert!(trace.pages.len() > 1, "fixture must span pages");
    for (page, lines) in trace.pages.iter().enumerate() {
        assert!(
            lines.iter().any(|line| line.text == "Fixed"),
            "CSS 2.1 §9.6.1 renders fixed boxes on page {page} too"
        );
    }
}

#[test]
fn relative_table_wrappers_move_captions_and_grid_once_without_moving_flow() {
    const SHIFT_X: f32 = 13.0;
    const SHIFT_Y: f32 = 9.0;
    for collapse in ["separate", "collapse"] {
        for caption in ["top", "bottom"] {
            let body = "<table><caption>Caption</caption><tr><td>A</td><td>B</td></tr></table><p>After</p>";
            let css = format!(
                "table{{width:160pt;border-collapse:{collapse};border-spacing:0;border:2pt solid}}td{{padding:3pt;border:1pt solid}}caption{{caption-side:{caption}}}"
            );
            let normal = render(body, &css);
            let shifted = render(
                body,
                &format!("{css}table{{position:relative;left:{SHIFT_X}pt;top:{SHIFT_Y}pt}}"),
            );
            assert_eq!(normal.pages.len(), shifted.pages.len());
            for (before, after) in normal.pages[0].iter().zip(&shifted.pages[0]) {
                assert_eq!(before.text, after.text, "{collapse}/{caption}: line order");
                let (dx, dy) = if before.text == "After" {
                    (0.0, 0.0)
                } else {
                    (SHIFT_X, SHIFT_Y)
                };
                common::assert_pt(after.x - before.x, dx);
                common::assert_pt(after.baseline - before.baseline, dy);
            }
            assert_eq!(normal.pages[0].len(), shifted.pages[0].len());
        }
    }
}

#[test]
fn structural_table_positioned_children_use_anonymous_cell_fixup() {
    const ANCHOR: &str = "<span class=anchor>Anchor</span>";
    const ROW: &str = "<div class=row><div class=cell>A</div><div class=cell>B</div></div>";
    for parent in ["root", "group", "row"] {
        for insets in ["", "left:7pt;top:9pt", "right:10%;bottom:20%"] {
            for collapse in ["separate", "collapse"] {
                let css = format!(
                    ".root{{display:table;position:relative;width:160pt;border-spacing:3pt;border-collapse:{collapse}}}.group{{display:table-row-group}}.row{{display:table-row}}.cell{{display:table-cell;padding:0}}.anchor{{position:absolute;white-space:nowrap;{insets}}}"
                );
                let wrap = |anchor: &str| match parent {
                    "root" => format!("{ROW}{anchor}{ROW}"),
                    "group" => format!("<div class=group>{ROW}{anchor}{ROW}</div>"),
                    _ => format!(
                        "<div class=row><div class=cell>A</div>{anchor}<div class=cell>B</div></div>"
                    ),
                };
                let explicit = format!("<div class=cell>{ANCHOR}</div>");
                let explicit = if parent == "row" {
                    explicit
                } else {
                    format!("<div class=row>{explicit}</div>")
                };
                let body =
                    |contents: String| format!("<div class=root>{contents}</div><p>After</p>");
                let expected = render(&body(wrap(&explicit)), &css);
                let actual = render(&body(wrap(ANCHOR)), &css);
                assert_eq!(actual.pages.len(), expected.pages.len());
                for (actual, expected) in actual.pages.iter().zip(&expected.pages) {
                    assert_eq!(actual.len(), expected.len(), "{parent}/{insets}/{collapse}");
                    for (actual, expected) in actual.iter().zip(expected) {
                        assert_eq!(actual.text, expected.text, "{parent}/{insets}/{collapse}");
                        common::assert_pt(actual.x, expected.x);
                        common::assert_pt(actual.baseline, expected.baseline);
                        common::assert_pt(actual.width, expected.width);
                    }
                }
                assert!(
                    actual
                        .pages
                        .iter()
                        .flatten()
                        .any(|line| line.text == "Anchor")
                );
            }
        }
    }
}

#[test]
fn table_fixup_retains_fixed_and_generated_anchors_but_discards_column_children() {
    let body = "<div class=root><span class=fixed>Fixed</span><div class=column><span class=absolute>DiscardColumn</span></div><div class=columns><span class=absolute>DiscardGroup</span></div><div class=row><div class=cell>A</div></div></div><p class=next>After</p>";
    let css = ".root{display:table;position:relative;width:160pt;border-spacing:0}.root::before{content:'Generated';position:absolute;left:7pt;top:9pt}.column{display:table-column}.columns{display:table-column-group}.row{display:table-row}.cell{display:table-cell}.fixed{position:fixed;left:13pt;top:17pt}.absolute{position:absolute}.next{break-before:page}";
    let trace = render(body, css);
    assert_eq!(trace.pages.len(), 2);
    for (page, lines) in trace.pages.iter().enumerate() {
        assert_eq!(
            lines.iter().filter(|line| line.text == "Fixed").count(),
            1,
            "fixed child must repeat exactly once on page {page}"
        );
        assert!(
            lines.iter().all(|line| !line.text.starts_with("Discard")),
            "table fixup must discard column and non-column colgroup children"
        );
    }
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    assert_eq!(
        lines.iter().filter(|line| line.text == "Generated").count(),
        1
    );
    let (_, ax, ay) = find_line(&trace, "A");
    let (_, gx, gy) = find_line(&trace, "Generated");
    common::assert_pt(gx - ax, 7.0);
    common::assert_pt(gy - ay, 9.0);
}

#[test]
fn relative_position_does_not_apply_to_table_columns_or_column_groups() {
    let body =
        "<table><colgroup><col><col></colgroup><tr><td>A</td><td>B</td></tr></table><p>After</p>";
    for collapse in ["separate", "collapse"] {
        let css = format!(
            "table{{width:160pt;border-collapse:{collapse};border-spacing:3pt}}td{{padding:3pt;border:2pt solid}}col{{background:blue}}colgroup{{background:red}}"
        );
        let normal = render(body, &css);
        for selector in ["col", "colgroup", "col,colgroup"] {
            let positioned = render(
                body,
                &format!("{css}{selector}{{position:relative;left:13pt;top:9pt}}"),
            );
            assert_eq!(
                serde_json::to_value(&positioned).unwrap(),
                serde_json::to_value(&normal).unwrap(),
                "{collapse}/{selector}: non-applicable positioning must preserve ink, flow and navigation"
            );
        }
    }
}

#[test]
fn relative_tables_resolve_offsets_from_the_containing_box() {
    const CONTAINING_WIDTH: f32 = 200.0;
    const CONTAINING_HEIGHT: f32 = 180.0;
    for rtl in [false, true] {
        for definite in [false, true] {
            let body =
                "<section><table><tr><td>A</td><td>B</td></tr></table><p>After</p></section>";
            let height = if definite {
                format!("height:{CONTAINING_HEIGHT}pt;")
            } else {
                String::new()
            };
            let direction = if rtl { "rtl" } else { "ltr" };
            let css = format!(
                "section{{width:{CONTAINING_WIDTH}pt;{height}direction:{direction}}}table{{width:100pt;border-spacing:0}}td{{padding:0}}"
            );
            let normal = render(body, &css);
            let shifted = render(
                body,
                &format!("{css}table{{position:relative;left:10%;right:5%;top:10%}}"),
            );
            for (before, after) in normal.pages[0].iter().zip(&shifted.pages[0]) {
                let dx = if before.text == "After" {
                    0.0
                } else if rtl {
                    -CONTAINING_WIDTH * 0.05
                } else {
                    CONTAINING_WIDTH * 0.1
                };
                let dy = if before.text != "After" && definite {
                    CONTAINING_HEIGHT * 0.1
                } else {
                    0.0
                };
                common::assert_pt(after.x - before.x, dx);
                common::assert_pt(after.baseline - before.baseline, dy);
            }
            assert_eq!(normal.pages[0].len(), shifted.pages[0].len());
        }
    }
}

#[test]
fn absolute_tables_retain_row_geometry_and_caption_layout() {
    for collapse in ["separate", "collapse"] {
        for caption in ["top", "bottom"] {
            let body = "<table><caption>Caption</caption><tr><td>A</td><td>B</td></tr></table><p>After</p>";
            let css = format!(
                "table{{width:160pt;border-collapse:{collapse};border-spacing:0;border:2pt solid}}td{{padding:3pt;border:1pt solid}}caption{{caption-side:{caption}}}"
            );
            let normal = render(body, &css);
            let positioned = render(
                body,
                &format!("{css}table{{position:absolute;left:13pt;top:9pt}}"),
            );
            let find = |trace: &LayoutTrace, text: &str| {
                let line = trace.pages[0]
                    .iter()
                    .find(|line| line.text == text)
                    .unwrap();
                (line.x, line.baseline)
            };
            let (ax, ay) = find(&positioned, "A");
            let (bx, by) = find(&positioned, "B");
            let (normal_ax, normal_ay) = find(&normal, "A");
            let (normal_bx, _) = find(&normal, "B");
            common::assert_pt(ay, by);
            common::assert_pt(bx - ax, normal_bx - normal_ax);
            let (_, caption_y) = find(&positioned, "Caption");
            let (_, normal_caption_y) = find(&normal, "Caption");
            common::assert_pt(caption_y - ay, normal_caption_y - normal_ay);
            let following_only = render("<p>After</p>", &css);
            common::assert_pt(
                find(&positioned, "After").1,
                find(&following_only, "After").1,
            );
        }
    }
}

#[test]
fn fixed_tables_replicate_the_grid_without_occupying_body_flow() {
    let trace = render(
        "<table><caption>Caption</caption><tr><td>A</td><td>B</td></tr></table><p>First</p><p class=next>Second</p>",
        "table{position:fixed;left:13pt;top:9pt;width:160pt;border-spacing:0}td{padding:0}.next{break-before:page}",
    );
    assert_eq!(trace.pages.len(), 2);
    for (page, lines) in trace.pages.iter().enumerate() {
        let a = lines.iter().find(|line| line.text == "A").unwrap();
        let b = lines.iter().find(|line| line.text == "B").unwrap();
        common::assert_pt(a.baseline, b.baseline);
        assert!(
            b.x > a.x,
            "page {page}: fixed table columns must remain side by side"
        );
        assert_eq!(
            lines.iter().filter(|line| line.text == "Caption").count(),
            1
        );
    }
}

#[test]
fn positioned_table_descendants_use_the_caption_wrapper_containing_block() {
    for position in ["relative", "absolute", "fixed"] {
        for caption in ["top", "bottom"] {
            let trace = render(
                "<table><caption>Caption</caption><tr><td>A<span>Overlay</span></td><td>B</td></tr></table>",
                &format!(
                    "table{{position:{position};width:160pt;left:13pt;top:9pt;border-spacing:0}}td{{padding:0}}caption{{caption-side:{caption}}}span{{position:absolute;left:0;top:0}}"
                ),
            );
            let find = |name| {
                trace.pages[0]
                    .iter()
                    .find(|line| line.text == name)
                    .unwrap()
            };
            let top = if caption == "top" { "Caption" } else { "A" };
            common::assert_pt(find("Overlay").baseline, find(top).baseline);
            common::assert_pt(find("Overlay").x, find("A").x);
        }
    }
}

#[test]
fn relative_table_fragments_keep_page_assignment_and_shift_every_fragment() {
    let rows = (0..24)
        .map(|index| format!("<tr><td>Row{index}</td><td>B{index}</td></tr>"))
        .collect::<String>();
    for caption in ["", "<caption>Caption</caption>"] {
        let body = format!("<table>{caption}{rows}</table><p>After</p>");
        let css = "@page{size:240pt 160pt;margin:20pt}table{width:160pt;border-collapse:collapse}td{padding:3pt;border:1pt solid}";
        let normal = render(&body, css);
        let shifted = render(
            &body,
            &format!("{css}table{{position:relative;left:13pt;top:9pt}}"),
        );
        assert!(normal.pages.len() > 1);
        assert_eq!(normal.pages.len(), shifted.pages.len());
        for (before, after) in normal.pages.iter().zip(&shifted.pages) {
            assert_eq!(before.len(), after.len());
            for (a, b) in before.iter().zip(after) {
                assert_eq!(a.text, b.text);
                let (dx, dy) = if a.text == "After" {
                    (0.0, 0.0)
                } else {
                    (13.0, 9.0)
                };
                common::assert_pt(b.x - a.x, dx);
                common::assert_pt(b.baseline - a.baseline, dy);
            }
        }
    }
}

#[test]
fn absolute_table_end_insets_use_the_expanded_intrinsic_grid_width() {
    const CONTAINING_WIDTH: f32 = 200.0;
    const END_INSET: f32 = 5.0;
    for collapse in ["separate", "collapse"] {
        let body = "<section><table><tr><td>UnbrokenAlpha</td><td>UnbrokenBeta</td></tr></table></section>";
        let css = format!(
            "section{{position:relative;width:{CONTAINING_WIDTH}pt}}table{{width:1pt;border:2pt solid;border-collapse:{collapse};border-spacing:0;background:blue}}td{{padding:0;white-space:nowrap}}"
        );
        let normal = render(body, &css);
        let positioned = render(
            body,
            &format!("{css}table{{position:absolute;right:{END_INSET}pt;top:0}}"),
        );
        let bounds = |trace: &LayoutTrace| {
            trace.display_lists[0]
                .iter()
                .find_map(|item| {
                    if let DisplayItem::Box(paint) = item {
                        (paint.decoration.background.alpha > 0.0).then_some(paint.rect)
                    } else {
                        None
                    }
                })
                .unwrap()
        };
        let a = bounds(&normal);
        let b = bounds(&positioned);
        assert!(
            b.width > 1.0,
            "{collapse}: the table must retain its intrinsic minimum"
        );
        common::assert_pt(b.width, a.width);
        common::assert_pt(b.x + b.width, a.x + CONTAINING_WIDTH - END_INSET);
    }
}

#[test]
fn positioned_auto_width_tables_do_not_activate_fixed_column_layout() {
    for position in ["absolute", "fixed"] {
        let body = "<table><tr><td>A</td><td>B</td></tr><tr><td>UnbrokenLongFirstColumn</td><td>C</td></tr></table>";
        let css = format!(
            "table{{position:{position};left:0;top:0;width:auto;border-spacing:0}}td{{padding:0;white-space:nowrap}}"
        );
        let automatic = render(body, &format!("{css}table{{table-layout:auto}}"));
        let fixed = render(body, &format!("{css}table{{table-layout:fixed}}"));
        assert_eq!(automatic.pages.len(), fixed.pages.len());
        for (a, b) in automatic.pages[0].iter().zip(&fixed.pages[0]) {
            assert_eq!(a.text, b.text);
            common::assert_pt(a.x, b.x);
            common::assert_pt(a.baseline, b.baseline);
        }
        assert_eq!(automatic.pages[0].len(), fixed.pages[0].len());
    }
}

#[test]
fn absolute_table_flex_static_position_uses_its_intrinsic_width() {
    const CONTAINING_WIDTH: f32 = 240.0;
    let body = "<section><table><tr><td>UnbrokenTableContent</td></tr></table></section>";
    let css = format!(
        "section{{position:relative;display:flex;justify-content:center;width:{CONTAINING_WIDTH}pt;height:100pt}}table{{position:absolute;width:1pt;border-spacing:0;background:blue}}td{{padding:0;white-space:nowrap}}"
    );
    let trace = render(body, &css);
    let table = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) if paint.decoration.background.alpha > 0.0 => Some(paint.rect),
            _ => None,
        })
        .unwrap();
    let anchored = render(body, &format!("{css}table{{left:0}}"));
    let start = anchored.pages[0][0].x;
    common::assert_pt(table.x + table.width / 2.0, start + CONTAINING_WIDTH / 2.0);
}

#[test]
fn relative_table_offsets_follow_named_page_width_without_double_shifting_captions() {
    let body = "<section><table><caption>Caption</caption><tr><td>A</td><td>B</td></tr><tr class=next><td>C</td><td>D</td></tr></table></section>";
    let css = "@page{size:240pt 160pt;margin:20pt}@page wide{size:340pt 160pt;margin:20pt}section{height:100pt}table{width:100%;border-spacing:0}td{padding:0}.next{page:wide;break-before:page}";
    let normal = render(body, css);
    let shifted = render(
        body,
        &format!("{css}table{{position:relative;left:10%;top:10%}}"),
    );
    assert_eq!(normal.pages.len(), 2);
    assert_eq!(shifted.pages.len(), 2);
    for (page, (before, after)) in normal.pages.iter().zip(&shifted.pages).enumerate() {
        assert_eq!(before.len(), after.len());
        let dx = [20.0, 30.0][page];
        for (a, b) in before.iter().zip(after) {
            assert_eq!(a.text, b.text);
            common::assert_pt(b.x - a.x, dx);
            common::assert_pt(b.baseline - a.baseline, 10.0);
        }
    }
}

#[test]
fn absolutely_positioned_boxes_self_align_in_their_inset_modified_containing_block() {
    const CONTAINER_PT: f32 = 200.0;
    const BOX_PT: f32 = 50.0;
    for (declarations, expected, reason) in [
        (
            "inset:0;place-self:center",
            [75.0, 75.0],
            "centred in both axes",
        ),
        (
            "left:20pt;top:0;justify-self:end",
            [150.0, 0.0],
            "the auto right inset is zero, so end alignment meets the containing block's end",
        ),
        (
            "left:0;right:0;top:0;margin:0 auto;justify-self:end",
            [75.0, 0.0],
            "auto margins take precedence over self-alignment",
        ),
        (
            "left:20pt;top:30pt",
            [20.0, 30.0],
            "normal alignment keeps the insets",
        ),
    ] {
        let trace = render(
            "<div class=cb><div class=a></div></div>",
            &format!(
                "body{{margin:0}}.cb{{position:relative;width:{CONTAINER_PT}pt;height:{CONTAINER_PT}pt}}\
                 .a{{position:absolute;width:{BOX_PT}pt;height:{BOX_PT}pt;background:red;{declarations}}}"
            ),
        );
        let placed = common::painted_box(&trace, [255, 0, 0]);
        let origin = trace.page_area;
        common::assert_labelled_pt(
            &format!("{declarations} x ({reason})"),
            placed.x - origin.x,
            expected[0],
        );
        common::assert_labelled_pt(
            &format!("{declarations} y ({reason})"),
            placed.y - origin.y,
            expected[1],
        );
    }
}
