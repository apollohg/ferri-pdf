mod common;
use common::{PROFILE, local_options};
use ferri_pdf::{LayoutTrace, render_html};
fn render(body: &str, css: &str) -> LayoutTrace {
    render_html(
        &format!("<style>{PROFILE}p{{margin:0}}{css}</style>{body}"),
        &local_options(),
    )
    .expect("flex document must render")
    .layout
    .unwrap()
}
#[test]
fn growth_distributes_content_space_after_edges_and_gaps() {
    let t = render(
        "<section><div>A</div><div class=wide>B</div><div>C</div></section><p>After</p>",
        "section{display:flex;width:240pt;gap:12pt}section div{flex:1;padding:4pt;border:2pt solid navy}.wide{flex:2}",
    );
    for (text, x) in [("A", 42.0), ("B", 111.0), ("C", 225.0)] {
        let line = t.pages[0].iter().find(|l| l.text == text).unwrap();
        assert!(
            (line.x - x).abs() < 0.001,
            "{text}: {} expected {x}",
            line.x
        );
    }
    let after = t.pages[0].iter().find(|l| l.text == "After").unwrap();
    assert!(
        (after.baseline - 79.1543).abs() < 0.001,
        "after row: {}",
        after.baseline
    );
}
#[test]
fn weighted_shrink_wraps_each_item_at_its_resolved_width() {
    let t = render(
        "<section><div>Alpha beta gamma</div><div class=slow>Delta epsilon zeta</div></section><p>After</p>",
        "section{display:flex;width:180pt}section div{width:160pt;min-width:0;padding:3pt;border:2pt solid navy}.slow{flex-shrink:2}",
    );
    let text: Vec<_> = t.pages[0].iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        text,
        ["Alpha beta", "gamma", "Delta", "epsilon", "zeta", "After"]
    );
    let delta = &t.pages[0][2];
    assert!(
        (delta.x - 157.6667).abs() < 0.001,
        "second item x {}",
        delta.x
    );
}
#[test]
fn column_centering_and_wrap_preserve_following_flow() {
    let t = render(
        "<section><div>One</div><div>Two lines<br>Second</div><div>Three</div></section><p>After</p>",
        "section{display:flex;flex-direction:column;width:220pt;height:180pt;align-items:center;justify-content:center;gap:8pt;border:2pt solid navy}section div{padding:3pt;background:#ddeeff}",
    );
    for (text, baseline) in [
        ("One", 91.1543),
        ("Two lines", 123.1543),
        ("Second", 141.1543),
        ("Three", 173.1543),
        ("After", 233.1543),
    ] {
        let line = t.pages[0].iter().find(|l| l.text == text).unwrap();
        assert!(
            (line.baseline - baseline).abs() < 0.001,
            "{text} baseline {} expected {baseline}",
            line.baseline
        );
    }
    let body = format!(
        "<section>{}</section><p>After</p>",
        (0..6)
            .map(|i| format!("<div>Item {i}</div>"))
            .collect::<String>()
    );
    let w = render(
        &body,
        "section{display:flex;flex-wrap:wrap;width:240pt;gap:8pt 12pt}section div{flex:1 0 80pt;padding:3pt;border:2pt solid navy}",
    );
    for (i, line) in w.pages[0].iter().take(6).enumerate() {
        assert!(
            (line.x - (if i % 2 == 0 { 41.0 } else { 167.0 })).abs() < 0.001,
            "item {i} x {}",
            line.x
        );
        assert!(
            (line.baseline - (54.1543 + (i / 2) as f32 * 36.0)).abs() < 0.001,
            "item {i} baseline {}",
            line.baseline
        );
    }
}

#[test]
fn anonymous_nonbreaking_space_and_containing_direction_are_preserved() {
    let t = render(
        "<section>\u{a0}<div>A</div></section>",
        "section{display:flex;gap:10pt}",
    );
    let a = t.pages[0].iter().find(|line| line.text == "A").unwrap();
    assert!(
        (a.x - 49.81445).abs() < 0.001,
        "NBSP item and gap must contribute width: {}",
        a.x
    );
    let t = render(
        "<section><div>Item</div></section>",
        "section{display:flex}div{direction:rtl;position:relative;left:20pt;right:40pt}",
    );
    let item = t.pages[0].iter().find(|line| line.text == "Item").unwrap();
    assert!(
        (item.x - 56.0).abs() < 0.001,
        "LTR containing block must choose left: {}",
        item.x
    );
}
#[test]
fn nested_columns_preserve_geometry_without_repeated_subtree_expansion() {
    const DEPTH: usize = 18;
    let body = format!(
        "{}<p>Leaf</p>{}<p>After</p>",
        "<section>".repeat(DEPTH),
        "</section>".repeat(DEPTH)
    );
    let trace = render(
        &body,
        "section{display:flex;flex-direction:column;padding:1pt}",
    );
    assert_eq!(trace.pages.len(), 1);
    let leaf = trace.pages[0]
        .iter()
        .find(|line| line.text == "Leaf")
        .unwrap();
    let after = trace.pages[0]
        .iter()
        .find(|line| line.text == "After")
        .unwrap();
    assert!(
        (leaf.x - (36.0 + DEPTH as f32)).abs() < 0.001,
        "nested padding: {}",
        leaf.x
    );
    assert!(
        (after.baseline - (49.1543 + 18.0 + 2.0 * DEPTH as f32)).abs() < 0.001,
        "following flow: {}",
        after.baseline
    );
}

#[test]
fn table_intrinsic_width_accounts_for_inflexible_bases() {
    let trace = render(
        "<table><tr><td><section><div>A</div><div>B</div></section></td><td>Other</td></tr></table>",
        "table{border-spacing:0}td{padding:0}section{display:flex}section div{flex:0 0 80pt}",
    );
    let other = trace.pages[0]
        .iter()
        .find(|line| line.text == "Other")
        .unwrap();
    assert!(
        (other.x - 196.0).abs() < 0.001,
        "two 80pt flex bases must contribute 160pt to the cell: {}",
        other.x
    );
}

#[test]
fn parallel_items_fragment_on_the_same_pages_and_stretch_to_the_same_end() {
    let lines = |label: &str, count: usize| {
        (0..count)
            .map(|i| format!("{label} {i:02}"))
            .collect::<Vec<_>>()
            .join("<br>")
    };
    let body = format!(
        "<section><div>{}</div><div>{}</div></section><p>After</p>",
        lines("Left", 40),
        lines("Right", 30)
    );
    let trace = render(
        &body,
        "section{display:flex;gap:12pt}section div{flex:1;min-width:0;border:2pt solid navy;padding:3pt}",
    );
    assert_eq!(trace.pages.len(), 2);
    for label in ["Left", "Right"] {
        for index in 0..22 {
            assert!(
                trace.pages[0]
                    .iter()
                    .any(|line| line.text == format!("{label} {index:02}")),
                "{label} {index} missing from first parallel fragment"
            );
        }
        let first = trace.pages[1]
            .iter()
            .find(|line| line.text == format!("{label} 22"))
            .unwrap();
        assert!(
            (first.baseline - 49.1543).abs() < 0.001,
            "continuation baseline: {}",
            first.baseline
        );
    }
    let after = trace.pages[1]
        .iter()
        .find(|line| line.text == "After")
        .unwrap();
    assert!(
        (after.baseline - 378.1543).abs() < 0.001,
        "following block must account for the fragmented line: {}",
        after.baseline
    );
    let boxes: Vec<_> = trace.display_lists[1]
        .iter()
        .filter_map(|item| match item {
            ferri_pdf::DisplayItem::Box(paint) => Some(paint.rect.height),
            _ => None,
        })
        .collect();
    assert_eq!(
        boxes,
        vec![329.0, 329.0],
        "both item borders must stretch to the final line end"
    );
}

#[test]
fn variables_do_not_silently_reset_valid_unsupported_flex_values() {
    for (property, value) in [("align-self", "last baseline")] {
        for declaration in [
            format!("{property}:{value}"),
            format!("--value:{value};{property}:var(--value)"),
        ] {
            let html = format!(
                "<style>{PROFILE}section{{display:flex;{declaration}}}</style><section><div>Content</div></section>"
            );
            assert!(
                render_html(&html, &local_options()).is_err(),
                "valid unsupported value must remain visible to validation: {declaration}"
            );
        }
    }
}

#[test]
fn generated_flex_containers_normalize_text_into_items() {
    for (display, body) in [("flex", "Body"), ("inline-flex", "\u{fffc}Body")] {
        let trace = render(
            "<section>Body</section><p>After</p>",
            &format!("section::before{{display:{display};content:'Generated';padding:4pt}}"),
        );
        let words: Vec<_> = trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(words, ["Generated", body, "After"], "generated {display}");
        let generated = trace.pages[0]
            .iter()
            .find(|line| line.text == "Generated")
            .unwrap();
        common::assert_pt(generated.x, 40.0);
    }
}

#[test]
fn wrapped_column_offsets_keep_their_pages_before_sibling_ink_is_painted() {
    let trace = render(
        &format!(
            "<section>{}</section>",
            (0..8)
                .map(|index| format!("<div>Item{index}</div>"))
                .collect::<String>()
        ),
        "@page{size:240pt 120pt;margin:20pt}section{display:flex;flex-flow:column wrap;height:160pt;width:200pt}section>div{height:40pt;flex:none}",
    );
    let pages: Vec<Vec<_>> = trace
        .pages
        .iter()
        .map(|page| page.iter().map(|line| line.text.as_str()).collect())
        .collect();
    assert_eq!(
        pages,
        [
            vec!["Item0", "Item1", "Item4", "Item5"],
            vec!["Item2", "Item3", "Item6", "Item7"]
        ]
    );
    for (before, after) in [
        ("Item0", "Item2"),
        ("Item1", "Item3"),
        ("Item4", "Item6"),
        ("Item5", "Item7"),
    ] {
        let first = trace.pages[0]
            .iter()
            .find(|line| line.text == before)
            .unwrap();
        let second = trace.pages[1]
            .iter()
            .find(|line| line.text == after)
            .unwrap();
        common::assert_pt(first.x, second.x);
        common::assert_pt(first.baseline, second.baseline);
    }
}

#[test]
fn table_items_flex_their_table_box_and_count_captions_as_its_edges() {
    let trace = render(
        "<section class=row><table class=grow><tr><td>A</td></tr></table><div class=tall></div></section>\
         <section class=column><table class=fill><caption>Caption</caption><tr><td>B</td></tr></table></section>",
        "td{padding:0}table{border-spacing:0}caption{height:20pt}.row{display:flex;width:200pt}\
         .grow{flex:1;background:rgb(255,0,0)}.tall{width:50pt;height:40pt;background:rgb(0,0,255)}\
         .column{display:flex;flex-direction:column;width:100pt;height:120pt}\
         .fill{flex:1;background:rgb(0,255,0)}",
    );
    let grown = common::painted_box(&trace, [255, 0, 0]);
    common::assert_labelled_pt("grown table width", grown.width, 150.0);
    common::assert_labelled_pt("stretched table height", grown.height, 40.0);
    let caption = trace.pages[0]
        .iter()
        .find(|line| line.text == "Caption")
        .expect("caption line");
    let filled = common::painted_box(&trace, [0, 255, 0]);
    let column_top = 36.0 + 40.0;
    let caption_height = 20.0;
    common::assert_labelled_pt(
        "table box below its caption",
        filled.y,
        column_top + caption_height,
    );
    common::assert_labelled_pt(
        "table box leaves the caption its share of the flexed item",
        filled.height,
        120.0 - caption_height,
    );
    assert!(
        caption.baseline < filled.y,
        "caption baseline {} must sit above the table box at {}",
        caption.baseline,
        filled.y
    );
}

#[test]
fn orthogonal_items_take_their_flexed_and_stretched_widths_as_block_sizes() {
    // CSS Flexbox 1 §9.8: the flexed main size and the stretched cross size are definite, so
    // a vertical item lays out its block axis at that physical width, as with a fixed width;
    // a table item's captions count as edges of its table box (§4).
    let trace = render(
        "<section class=row><div class='v grow'>A</div><div class=fixed></div></section>\
         <section class=row><table class='v grow'><caption>C</caption><tr><td>B</td></tr></table><div class=fixed></div></section>\
         <section class=column><div class='v stretch'>D</div></section>",
        "td{padding:0;background:rgb(255,0,255)}table{border-spacing:0}\
         caption{width:20pt;background:rgb(255,255,0)}.v{writing-mode:vertical-rl}\
         .row{display:flex;width:200pt;height:40pt}.fixed{width:50pt}\
         .grow{flex:1}div.grow{background:rgb(255,0,0)}\
         .column{display:flex;flex-direction:column;width:200pt}\
         .stretch{height:30pt;background:rgb(0,0,255)}",
    );
    let grown = common::painted_box(&trace, [255, 0, 0]);
    common::assert_labelled_pt("flexed vertical block width", grown.width, 150.0);
    let cells = common::page_boxes(&trace, [255, 0, 255]);
    let captions = common::page_boxes(&trace, [255, 255, 0]);
    assert_eq!(
        (cells.len(), captions.len()),
        (1, 1),
        "one cell and one caption: {cells:?} {captions:?}"
    );
    let (cell, caption) = (cells[0], captions[0]);
    let caption_width = 20.0;
    common::assert_labelled_pt("flexed table cell left edge", cell.x, 36.0);
    common::assert_labelled_pt(
        "flexed table cell fills the table box beside the caption",
        cell.width,
        150.0 - caption_width,
    );
    common::assert_labelled_pt("cell stretched to the row height", cell.height, 40.0);
    common::assert_labelled_pt(
        "block-start caption at the right of the flexed item",
        caption.x,
        36.0 + 150.0 - caption_width,
    );
    common::assert_labelled_pt("caption width", caption.width, caption_width);
    let stretched = common::painted_box(&trace, [0, 0, 255]);
    common::assert_labelled_pt("stretched vertical block width", stretched.width, 200.0);
    common::assert_labelled_pt("stretched vertical block height", stretched.height, 30.0);
}
