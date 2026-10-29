mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{DisplayItem, LayoutTrace, render_html};

fn render(body: &str, css: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}p{{margin:0}}{css}</style>{body}"),
        &local_options(),
    )
    .expect("constrained block widths must render")
    .layout
    .unwrap()
}
fn widths(trace: &LayoutTrace) -> Vec<f32> {
    trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) => Some(paint.rect.width),
            _ => None,
        })
        .collect()
}
#[test]
fn width_constraints_resolve_percentages_and_box_sizing() {
    let trace = render(
        "<div class=minimum>Minimum</div><div class=maximum>Maximum</div><div class=conflict>Conflict</div><div class=border>Border</div>",
        "div{width:200pt;padding:4pt;border:2pt solid navy}.minimum{width:40pt;min-width:100pt}.maximum{max-width:120pt}.conflict{min-width:50%;max-width:80pt}.border{box-sizing:border-box;max-width:100pt}",
    );
    assert_eq!(
        widths(&trace),
        [112.0, 132.0, 156.0, 100.0],
        "minimum wins over maximum, with content-box edges added only once"
    );
}
#[test]
fn constrained_auto_width_recomputes_auto_margins() {
    let trace = render(
        "<section><div>Centered</div></section><article><div>Right</div></article>",
        "section,article{width:200pt}div{max-width:100pt;border:2pt solid navy}section div{margin-left:auto;margin-right:auto}article{direction:rtl}",
    );
    let paints: Vec<_> = trace.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint) => Some(paint),
            _ => None,
        })
        .collect();
    assert_eq!(
        paints[0].rect.x, 84.0,
        "remaining 96pt must split between auto margins"
    );
    assert_eq!(
        paints[1].rect.x, 132.0,
        "RTL overconstraint adjusts the left margin"
    );
}
#[test]
fn constrained_width_drives_wrapping_and_pagination() {
    let body = format!(
        "<div>{}</div><p>After</p>",
        "Alpha beta gamma delta epsilon. ".repeat(30)
    );
    let explicit = render(&body, "div{width:110pt;border:2pt solid navy;padding:3pt}");
    let limited = render(
        &body,
        "div{max-width:110pt;border:2pt solid navy;padding:3pt}",
    );
    assert!(explicit.pages.len() > 1);
    assert_eq!(limited.pages.len(), explicit.pages.len());
    for (page, (actual, expected)) in limited.pages.iter().zip(explicit.pages.iter()).enumerate() {
        assert_eq!(actual.len(), expected.len(), "page {page} line count");
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.text, expected.text, "page {page} wrapping");
            assert_eq!(
                (actual.x, actual.baseline),
                (expected.x, expected.baseline),
                "page {page} line {}",
                actual.text
            );
        }
    }
}

#[test]
fn definite_width_contributes_its_constrained_size_to_auto_tables() {
    let body =
        "<table><tr><td><div>Alpha beta gamma delta epsilon.</div></td><td>Other</td></tr></table>";
    for (constraint, expected_x) in [("max-width:100pt", 95.0), ("min-width:100pt", 145.0)] {
        let trace = render(
            body,
            &format!(
                "table{{border-spacing:0}}td{{padding:2pt;vertical-align:top;border:1pt solid navy}}td div{{width:50pt;{constraint}}}"
            ),
        );
        let other = trace.pages[0]
            .iter()
            .find(|line| line.text == "Other")
            .unwrap();
        assert!(
            (other.x - expected_x).abs() < 0.001,
            "{constraint}: second column starts at {}, expected {expected_x}",
            other.x
        );
    }
}

#[test]
fn size_shorthands_copy_omitted_axes_and_share_the_longhand_cascade_and_limits() {
    for (declarations, expected) in [
        ("size:40pt", [40.0, 40.0]),
        ("size:40pt 25pt", [40.0, 25.0]),
        ("size:30pt 20pt;width:50pt", [50.0, 20.0]),
        ("width:50pt;height:60pt;size:30pt", [30.0, 30.0]),
        ("size:1pt;min-size:20pt", [20.0, 20.0]),
        ("size:100pt;max-size:40pt", [40.0, 40.0]),
        (
            "size:200pt;min-size:45pt 20pt;max-size:35pt 30pt",
            [45.0, 30.0],
        ),
        (
            "--dimensions:40pt 25pt;size:var(--dimensions)",
            [40.0, 25.0],
        ),
        ("size:30pt 20pt;size:-1pt 50pt", [30.0, 20.0]),
    ] {
        let css = format!(".probe{{background:rgb(1,2,3);{declarations}}}");
        let trace = render("<div class=probe></div>", &css);
        let rect = common::painted_box(&trace, [1, 2, 3]);
        assert_eq!(
            [rect.width, rect.height],
            expected,
            "{declarations}: {rect:?}"
        );
    }
}

#[test]
fn size_shorthands_preserve_page_descriptors_and_inherit_both_element_axes() {
    let trace = render(
        "<section><div class=probe></div></section><p>After</p>",
        "@page{size:220pt 140pt;margin:10pt}section{size:40pt 25pt}.probe{size:inherit;background:rgb(1,2,3)}",
    );
    let rect = common::painted_box(&trace, [1, 2, 3]);
    assert_eq!(
        [trace.page_width, trace.page_height],
        [220.0, 140.0],
        "@page size keeps its descriptor grammar and page dimensions"
    );
    assert_eq!(
        [rect.x, rect.y, rect.width, rect.height],
        [10.0, 10.0, 40.0, 25.0],
        "element sizing must inherit its two longhands without altering the page descriptor"
    );
    let after = trace.pages[0]
        .iter()
        .find(|line| line.text == "After")
        .unwrap();
    assert!(
        after.baseline > rect.y + rect.height,
        "the following paragraph must flow after the sized parent: baseline={}, parent={rect:?}",
        after.baseline
    );
}
