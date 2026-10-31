mod common;
use common::{PROFILE, local_options, lossy_options};
use ferri_pdf::{DisplayItem, render_html};

fn render(body: &str, css: &str) -> ferri_pdf::LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}p{{margin:0}}{css}</style>{body}"),
        &local_options(),
    )
    .expect("height layout must render through the public library")
    .layout
    .unwrap()
}

#[test]
fn fixed_heights_keep_box_extent_separate_from_visible_overflow() {
    let trace = render(
        "<div>One<br>Two<br>Three<br>Four</div><p>After</p>",
        "div{height:20pt;border:2pt solid navy;background:#ddeeff}",
    );
    let lines = &trace.pages[0];
    assert_eq!(lines.len(), 5, "overflow must retain every text line");
    let after = lines.iter().find(|line| line.text == "After").unwrap();
    const EXPECTED_AFTER_BASELINE: f32 = 73.1543;
    assert!(
        (after.baseline - EXPECTED_AFTER_BASELINE).abs() < 0.001,
        "following baseline: {}",
        after.baseline
    );
    let paint = trace.display_lists[0]
        .iter()
        .find_map(|item| match item {
            DisplayItem::Box(paint) => Some(paint),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        paint.rect.height, 24.0,
        "decoration follows specified height, not overflowing lines"
    );
}

#[test]
fn percentage_children_use_constrained_definite_parent_height() {
    let trace = render(
        "<div class=outer><div class=inner>Half</div><p>Tail</p></div><p>After</p>",
        ".outer{height:80pt;min-height:120pt;max-height:100pt;border:2pt solid navy}.inner{height:50%}",
    );
    let lines = &trace.pages[0];
    assert!(
        (lines[1].baseline - lines[0].baseline - 60.0).abs() < 0.001,
        "percentage basis must include min-height clamping"
    );
    assert!(
        (lines[2].baseline - lines[0].baseline - 122.0).abs() < 0.001,
        "parent flow extent includes its bottom border"
    );
}

#[test]
fn overflowing_pages_do_not_move_the_normal_flow_sibling() {
    let body = format!(
        "<div>{}</div><p>After</p><p class=new-page>Forced</p>",
        (0..30)
            .map(|n| format!("Overflow {n:02}"))
            .collect::<Vec<_>>()
            .join("<br>")
    );
    let trace = render(
        &body,
        "div{height:20pt;border:2pt solid navy;background:#ddeeff}",
    );
    assert_eq!(
        trace.pages.len(),
        3,
        "forced break follows the allocated overflow pages"
    );
    assert_eq!(
        trace.pages[0]
            .iter()
            .filter(|line| line.text.starts_with("Overflow"))
            .count(),
        22
    );
    assert_eq!(trace.pages[1].len(), 8);
    let after = trace.pages[0]
        .iter()
        .find(|line| line.text == "After")
        .expect("normal-flow sibling belongs to page one");
    assert!((after.baseline - 73.1543).abs() < 0.001);
    assert_eq!(trace.pages[2][0].text, "Forced");
    assert!(
        trace.display_lists[1]
            .iter()
            .all(|item| !matches!(item, DisplayItem::Box(paint) if paint.has_ink())),
        "overflow page must not extend the fixed box decoration"
    );
}

#[test]
fn enormous_fixed_height_stops_at_the_page_limit_before_allocation() {
    let output = render_html(
        &format!("<style>{PROFILE}div{{height:1e30pt}}</style><div>Large</div><p>After</p>"),
        &lossy_options(),
    )
    .expect("enormous specified height is bounded by the page limit");
    assert!(
        output.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            ferri_pdf::RenderDiagnostic::Layout(ferri_pdf::LayoutDiagnostic::PageLimit { .. })
        )),
        "the content past the last page is reported: {:?}",
        output.diagnostics
    );
    let text: Vec<_> = output
        .layout
        .unwrap()
        .pages
        .iter()
        .flatten()
        .map(|line| line.text.clone())
        .collect();
    assert_eq!(
        text,
        ["Large"],
        "the box's text is kept and the paragraph after it lies past the last page; trailing \
         unpainted pages are not produced"
    );
}

#[test]
fn fixed_height_fragments_preserve_remaining_height_and_border_reserve() {
    let trace = render(
        "<div>Top</div><p>After</p>",
        "div{height:600pt;border:2pt solid navy;background:#ddeeff}",
    );
    assert_eq!(trace.pages.len(), 2);
    for (page, expected) in [408.0, 196.0].into_iter().enumerate() {
        let paint = trace.display_lists[page]
            .iter()
            .find_map(|item| match item {
                DisplayItem::Box(paint) => Some(paint),
                _ => None,
            })
            .unwrap();
        assert_eq!(paint.rect.height, expected, "fixed-height fragment {page}");
        assert_eq!(paint.first_fragment, page == 0);
        assert_eq!(paint.last_fragment, page == 1);
    }
    assert!(
        (trace.pages[1][0].baseline - 245.1543).abs() < 0.001,
        "following baseline: {}",
        trace.pages[1][0].baseline
    );
}

#[test]
fn forced_child_breaks_use_natural_fragments_and_clone_repeats_edges() {
    let forced = render(
        "<div><p>First</p><p class=new-page>Forced</p></div><p>After</p>",
        "div{height:100pt;border:2pt solid navy;background:#ddeeff}",
    );
    assert_eq!(forced.pages.len(), 2);
    assert!(
        (forced.pages[1][1].baseline - 69.1543).abs() < 0.001,
        "forced child break must not restore the first-page cursor or clone the sliced top border: {}",
        forced.pages[1][1].baseline
    );
    let cloned = render(
        "<div>Top</div><p>After</p>",
        "div{height:600pt;padding:4pt;border:2pt solid navy;background:#ddeeff;box-decoration-break:clone}",
    );
    assert_eq!(cloned.pages.len(), 2);
    for (page, expected) in [408.0, 216.0].into_iter().enumerate() {
        let paint = cloned.display_lists[page]
            .iter()
            .find_map(|item| match item {
                DisplayItem::Box(paint) => Some(paint),
                _ => None,
            })
            .unwrap();
        assert_eq!(paint.rect.height, expected, "cloned fragment {page}");
        assert!(
            paint.first_fragment && paint.last_fragment,
            "each cloned fragment repeats both edges"
        );
    }
}

#[test]
fn page_fit_uses_visible_content_before_restoring_fixed_flow_height() {
    let trace = render(
        "<header>Lead</header><div>One<br>Two<br>Three</div><p>After</p>",
        "header{height:400pt}div{height:1pt;background:#ddeeff}",
    );
    assert_eq!(trace.pages.len(), 2);
    let page = &trace.pages[1];
    let one = page.iter().find(|line| line.text == "One").unwrap();
    let after = page.iter().find(|line| line.text == "After").unwrap();
    assert!(
        (after.baseline - one.baseline - 1.0).abs() < 0.001,
        "tiny box must move together with its first fragment"
    );
    let body = format!(
        "<header>Lead</header><section><div>{}</div></section><p>After</p>",
        (0..10)
            .map(|n| format!("Line{n}"))
            .collect::<Vec<_>>()
            .join("<br>")
    );
    let kept = render(
        &body,
        "header{height:300pt}section{break-inside:avoid}div{height:20pt;border:2pt solid navy}",
    );
    assert_eq!(
        kept.pages[0].len(),
        1,
        "ancestor keep constraint includes descendant overflow extent"
    );
    assert_eq!(kept.pages[1].len(), 11);
}

#[test]
fn page_filling_border_box_with_percentage_children_stays_on_one_page() {
    let trace = render_html(
        include_str!("oracle/fixtures/page-fill-rounding.html"),
        &local_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let texts: Vec<Vec<&str>> = trace
        .pages
        .iter()
        .map(|page| page.iter().map(|line| line.text.as_str()).collect())
        .collect();
    assert_eq!(
        texts,
        [vec!["Inner frame"], vec!["Second page"]],
        "a 210mm border-box fills the A4 landscape page exactly; only the next paragraph breaks"
    );
    let heights: Vec<Vec<f32>> = trace
        .display_lists
        .iter()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint) => Some(paint.rect.height),
                    _ => None,
                })
                .collect()
        })
        .collect();
    let millimetre = 72.0 / 25.4;
    let frame = (210.0 - 16.0) * millimetre;
    let inner = frame - 11.0 * millimetre;
    assert_eq!(heights[0].len(), 2, "page 1 box heights {heights:?}");
    for (height, expected) in heights[0].iter().zip([frame, inner]) {
        assert!(
            (height - expected).abs() < 0.01,
            "unfragmented frame heights {heights:?}, expected {frame} and {inner}"
        );
    }
    assert!(
        heights[1].is_empty(),
        "no frame fragment may spill onto page 2, got {heights:?}"
    );
}

#[test]
fn full_height_document_with_leading_collapsed_margin_does_not_add_a_blank_page() {
    let trace = render(
        "<h2>Contents</h2><p>Entry</p>",
        "@page{size:240pt 240pt;margin:10pt}html,body{margin:0;height:100%}h2{font:12pt/16pt Spike;margin:20pt 0 10pt}",
    );
    assert_eq!(
        trace.pages.len(),
        1,
        "a leading collapsed margin must not produce a trailing blank page: {:?}",
        trace
            .pages
            .iter()
            .map(|page| page
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Contents", "Entry"]
    );
}

#[test]
fn trailing_unused_height_is_omitted_without_discarding_page_content() {
    const PAGE: &str = "@page{size:240pt 240pt;margin:10pt}html,body{margin:0}section{height:240pt}p{font:12pt/16pt Spike;margin:0}";
    for (name, body, extra, expected) in [
        ("unused height", "<section><p>Top</p></section>", "", 1),
        (
            "canvas",
            "<section><p>Top</p></section>",
            "body{background:yellow}",
            1,
        ),
        (
            "painted height",
            "<section><p>Top</p></section>",
            "section{background:yellow}",
            2,
        ),
        (
            "border",
            "<section><p>Top</p></section>",
            "section{border:1pt solid blue}",
            2,
        ),
        (
            "following text",
            "<section><p>Top</p></section><p>After</p>",
            "",
            2,
        ),
        (
            "hidden following text",
            "<section><p>Top</p></section><p style='visibility:hidden'>After</p>",
            "",
            2,
        ),
        (
            "empty destination",
            "<section><p>Top</p></section><p id=end></p>",
            "",
            2,
        ),
        (
            "forced empty page",
            "<p>Top</p><div style='break-before:page'></div>",
            "",
            2,
        ),
        (
            "recto empty page",
            "<p>Top</p><div style='break-before:right'></div>",
            "",
            3,
        ),
    ] {
        let trace = render(body, &format!("{PAGE}{extra}"));
        assert_eq!(
            trace.pages.len(),
            expected,
            "case {name}: {:?}",
            trace
                .pages
                .iter()
                .map(|page| page
                    .iter()
                    .map(|line| line.text.as_str())
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            trace.display_lists.len(),
            expected,
            "case {name}: display pages"
        );
        if name == "empty destination" {
            assert_eq!(trace.destinations[0].as_ref().unwrap().page, 1);
        }
    }
}
