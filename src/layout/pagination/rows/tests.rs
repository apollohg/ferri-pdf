use super::*;
use crate::document::StyledDocument;
use crate::layout::measure::{MeasuredParagraph, RowPaint, RowText};
use crate::resources::{FileResolver, ResourceBase};
use crate::text::TextContext;

fn find_row<'a, 'b>(block: &'b mut MeasuredBlock<'a>) -> Option<&'b mut MeasuredRow<'a>> {
    for item in &mut block.items {
        match item {
            FlowItem::Row(row) => return Some(row),
            FlowItem::Block(child) => {
                if let Some(row) = find_row(child) {
                    return Some(row);
                }
            }
            _ => {}
        }
    }
    None
}

fn take_paragraph<'a>(block: &mut MeasuredBlock<'a>) -> Option<MeasuredParagraph<'a>> {
    let index = block
        .items
        .iter()
        .position(|item| matches!(item, FlowItem::Paragraph(_)));
    if let Some(index) = index {
        let FlowItem::Paragraph(paragraph) = block.items.remove(index) else {
            unreachable!()
        };
        return Some(paragraph);
    }
    block.items.iter_mut().find_map(|item| match item {
        FlowItem::Block(child) => take_paragraph(child),
        _ => None,
    })
}

#[test]
fn after_child_row_paints_keep_canonical_text_navigation_and_stacking() {
    const TEXT_X: f32 = 3.0;
    const TEXT_TOP: f32 = 6.0;
    const ROW_TOP: f32 = 12.0;
    const CHILD_WIDTH: f32 = 50.0;
    const RULE_HEIGHT: f32 = 2.0;
    const CHILD_COLOR: [u8; 3] = [0, 0, 255];
    const RULE_COLOR: [u8; 3] = [255, 0, 0];
    const RAISED_COLOR: [u8; 3] = [0, 128, 0];
    let resolver = FileResolver::default();
    let document = StyledDocument::parse(
        "<!doctype html><style>@page{size:200pt 100pt;margin:0}html,body{margin:0}section{display:flex;margin-top:12pt}article{width:50pt;height:30pt;background:blue}article+article{position:relative;z-index:1;left:-50pt;background:green}a{display:contents;text-decoration:underline;text-shadow:2pt 1pt red;font-size:12pt}.assignment{display:contents;string-set:heading 'After'}</style><section><article><a href='https://example.test/radical'>√<span class=assignment></span></a></article><article></article></section>",
        &ResourceBase::default(), &[], &resolver,
    ).unwrap();
    let text = TextContext::new(&document.styles().fonts, &resolver);
    let area = document.page_box().geometry.content_box();
    let mut measurer = Measurer {
        table_data: Default::default(),
        native_sizing: None,
        element_targets: Default::default(),
        frame_measurements: Default::default(),
        raw_templates: Default::default(),
        raw_pagination: false,
        column_measurement: false,
        orthogonal_columns: None,
        space: Default::default(),
        work: crate::layout::work::Budget::new(crate::layout::MAX_LAYOUT_WORK),
        text: &text,
        canvas_images: false,
        motion: Rc::new(crate::layout::motion::Resources::new(
            Default::default(),
            text.fonts(),
        )),
        page_height: area.height,
        sources: crate::layout::source::SourceIds::new(document.root()),
        item_metrics: Default::default(),
        grid_parent: None,
        clearance_targets: Default::default(),
        fragmentainer: Fragmentainer::Page,
        column_flow: None,
        spanner: None,
        unbroken: false,
        named_pages: true,
        container_sizes: Default::default(),
        annotation_part: None,
        footnote_deferrals: Default::default(),
    };
    let mut measured = measurer
        .block(
            document.root(),
            area.x,
            area.width,
            false,
            true,
            Some(area.height),
        )
        .unwrap();
    let row = find_row(&mut measured).expect("real flex row");
    let paragraph = take_paragraph(&mut row.cells[0].block).expect("canonical text child");
    let line = paragraph
        .lines
        .into_iter()
        .next()
        .expect("shaped radical line");
    let source_index = paragraph.index;
    let x = paragraph.x + TEXT_X;
    let baseline = TEXT_TOP + line.ascent;
    assert_eq!(
        line.text,
        "√",
        "fixture must retain real source: {}",
        paragraph.paragraph.text()
    );
    assert_eq!(
        line.links.len(),
        1,
        "canonical source links={}, range={:?}",
        paragraph.paragraph.links().len(),
        line.range
    );
    let link = line.links[0].rect;
    let mut rule = BoxPaint::whole(
        Rect {
            x: paragraph.x,
            y: TEXT_TOP,
            width: CHILD_WIDTH,
            height: RULE_HEIGHT,
        },
        row.cells[0]
            .block
            .paint
            .as_ref()
            .expect("blue child paint")
            .decoration
            .clone(),
    );
    rule.decoration.background.rgb = RULE_COLOR;
    row.after = vec![
        RowPaint::Box(Box::new(rule)),
        RowPaint::Text(Box::new(RowText {
            source: paragraph.paragraph,
            line,
            x,
            baseline,
        })),
    ];
    let painting = crate::layout::page_paint::PagePaint::new(
        document.page_box(),
        document.canvas_background(),
        crate::layout::PageRecord::new(document.page_box())
            .unwrap()
            .output
            .bleed,
    );
    let paginator = Paginator::for_document(
        &mut measurer,
        crate::layout::physical_pages::PhysicalPages::new(&document, &resolver).unwrap(),
        painting.canvas,
        area,
    );
    let output = paginator
        .document(
            Some(measured),
            document.navigation().destinations,
            document.root().leading_events(),
        )
        .unwrap();
    let strings = output
        .string_locations
        .resolve(
            document.running_strings(),
            &crate::layout::page_counters::PageCounters::new(&output.page_boxes),
            output.pages.len(),
        )
        .expect("arranged source events must have committed page locations");
    let request = |mode| crate::style::NamedStringRequest {
        name: "heading".into(),
        mode,
    };
    assert_eq!(
        strings.text(0, &request(crate::style::NamedStringMode::Last)),
        "After",
        "the source-end inline assignment must emit on the arranged line's page"
    );
    assert_eq!(
        strings.text(0, &request(crate::style::NamedStringMode::Start)),
        "",
        "selectable text before the source-end assignment closes page-start eligibility"
    );
    let lines: Vec<_> = output
        .pages
        .iter()
        .flatten()
        .filter(|line| line.text == "√")
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "the after-child canonical paragraph must emit exactly once"
    );
    assert_eq!(
        lines[0].paragraph, source_index,
        "source identity must survive arranged glyph emission"
    );
    assert_eq!(lines[0].x, x);
    assert_eq!(lines[0].baseline, ROW_TOP + baseline);
    assert!(
        !lines[0].runs.is_empty(),
        "the selectable radical must retain shaped glyphs"
    );
    assert_eq!(
        output.links[0].len(),
        1,
        "ordinary inline navigation must accompany arranged glyphs"
    );
    let crate::layout::navigation::LinkShape::Rect(link_area) = output.links[0][0].shape else {
        panic!("untransformed link must stay rectangular");
    };
    assert_eq!(link_area.x, x + link.x);
    assert_eq!(link_area.y, ROW_TOP + baseline + link.y);
    assert!(
        output.display_lists[0]
            .iter()
            .any(|item| matches!(item, DisplayItem::TextShadow(_))),
        "ordinary text shadows must accompany arranged glyphs"
    );
    let boxes: Vec<_> = output.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Box(paint)
                if [CHILD_COLOR, RULE_COLOR, RAISED_COLOR]
                    .contains(&paint.decoration.background.rgb)
                    && paint.decoration.background.alpha > 0.0 =>
            {
                Some(paint)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        boxes
            .iter()
            .map(|paint| paint.decoration.background.rgb)
            .collect::<Vec<_>>(),
        [CHILD_COLOR, RULE_COLOR, RAISED_COLOR],
        "after-child rules cover ordinary children while positive z-index children stay above them"
    );
    assert_eq!(boxes[1].rect.y, ROW_TOP + TEXT_TOP);
}
