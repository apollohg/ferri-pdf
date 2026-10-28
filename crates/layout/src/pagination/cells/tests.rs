use super::*;
use ferri_pdf_document::StyledDocument;
use ferri_pdf_resources::{FileResolver, ResourceBase};
use ferri_pdf_text::TextContext;

fn flex_cells(block: MeasuredBlock<'_>) -> Vec<crate::measure::MeasuredCell<'_>> {
    for item in block.items {
        match item {
            FlowItem::Row(row) => return row.cells,
            FlowItem::Block(child) => {
                let cells = flex_cells(*child);
                if !cells.is_empty() {
                    return cells;
                }
            }
            _ => {}
        }
    }
    Vec::new()
}

#[test]
fn resumed_cells_keep_source_paint_order_when_a_later_lane_finishes_first() {
    const EARLIER: [u8; 3] = [0, 0, 255];
    const LATER: [u8; 3] = [255, 0, 0];
    let resolver = FileResolver::default();
    for placement in ["relative", "static", "deferred", "z-index"] {
        let html = "<!doctype html><style>@page{size:200pt 100pt;margin:0}html,body{margin:0}section{display:flex}article{width:100pt}i{display:block;height:10pt}b{display:block;position:relative;width:100pt;height:20pt;break-before:page}article:first-child b{background:blue}article:last-child b{background:red;left:-100pt}</style><section><article><i></i><b></b></article><article><i></i><b></b></article></section>";
        let html = match placement {
            "relative" => html.to_owned(),
            "z-index" => html.replace("</style>", "article:first-child b{z-index:1}article:last-child b{z-index:0}</style>"),
            "static" => html.replace("position:relative", "position:static").replace("left:-100pt", "margin-left:-100pt"),
            "deferred" => html.replace("position:relative", "position:absolute")
                .replace("article:first-child b", "article:first-child em")
                .replace("article:last-child b{background:red;left:-100pt}", "b{left:0}article:last-child em{background:red}em{display:block;position:relative;width:100pt;height:20pt}.boundary{break-before:page}")
                .replace("<b></b>", "<div class=boundary><b><em></em></b></div>"),
            _ => unreachable!(),
        };
        let document =
            StyledDocument::parse(&html, &ResourceBase::default(), &[], &resolver).unwrap();
        for schedule in ["sequential", "reversed", "reversed-start", "production"] {
            let text = TextContext::new(&document.styles().fonts, &resolver);
            let area = document.page_box().geometry.content_box();
            let mut measurer = Measurer {
                work: crate::work::Budget::new(crate::MAX_LAYOUT_WORK),
                text: &text,
                canvas_images: false,
                page_height: area.height,
                sources: crate::source::SourceIds::new(document.root()),
                item_metrics: Default::default(),
                grid_parent: None,
                clearance_targets: Default::default(),
                fragmentainer: Fragmentainer::Page,
                column_flow: None,
                spanner: None,
                unbroken: false,
                named_pages: true,
            };
            let measured = measurer
                .block(
                    document.root(),
                    area.x,
                    area.width,
                    false,
                    true,
                    Some(area.height),
                )
                .unwrap();
            let painting = crate::page_paint::PagePaint::new(
                document.page_box(),
                document.canvas_background(),
                crate::PageRecord::new(document.page_box())
                    .unwrap()
                    .output
                    .bleed,
            );
            let mut paginator = Paginator::for_document(
                &mut measurer,
                crate::physical_pages::PhysicalPages::new(&document, &resolver).unwrap(),
                painting.canvas,
                area,
            );
            let output = if schedule == "production" {
                paginator
                    .document(
                        Some(measured),
                        document.navigation().destinations,
                        document.root().leading_events(),
                    )
                    .unwrap()
            } else {
                let mut cells: Vec<_> = flex_cells(measured)
                    .into_iter()
                    .map(|cell| (cell, paginator.orders.branch()))
                    .collect();
                if schedule == "reversed-start" {
                    cells.reverse();
                }
                assert_eq!(
                    cells.len(),
                    2,
                    "fixture must produce two real measured flex lanes"
                );
                paginator.containers.push(Container {
                    pending: Vec::new(),
                    fixed: false,
                });
                if schedule == "reversed" {
                    let mut lanes = cells
                        .into_iter()
                        .map(|(cell, orders)| {
                            paginator
                                .start_cell(
                                    cell.block,
                                    BreakTail::default(),
                                    CellStart {
                                        flex_item: true,
                                        orders: Some(orders),
                                        ..CellStart::default()
                                    },
                                    None,
                                )
                                .unwrap()
                        })
                        .collect::<Vec<_>>();
                    assert!(
                        paginator.advance_cell(&mut lanes[0]).unwrap().is_none(),
                        "earlier lane must suspend before its positioned descendant"
                    );
                    while paginator.advance_cell(&mut lanes[1]).unwrap().is_none() {}
                    while paginator.advance_cell(&mut lanes[0]).unwrap().is_none() {}
                } else {
                    for (cell, orders) in cells {
                        paginator.select_page(0, 0.0);
                        let mut lane = paginator
                            .start_cell(
                                cell.block,
                                BreakTail::default(),
                                CellStart {
                                    flex_item: true,
                                    orders: Some(orders),
                                    ..CellStart::default()
                                },
                                None,
                            )
                            .unwrap();
                        while paginator.advance_cell(&mut lane).unwrap().is_none() {}
                    }
                }
                for pending in paginator.containers.pop().unwrap().pending {
                    paginator.place(pending, &Containing::Page).unwrap();
                }
                paginator
                    .document(
                        None,
                        document.navigation().destinations,
                        document.root().leading_events(),
                    )
                    .unwrap()
            };
            assert_eq!(
                output.pages.len(),
                2,
                "schedule={schedule}, placement={placement}"
            );
            let paints = output.display_lists[1]
                .iter()
                .filter_map(|item| match item {
                    DisplayItem::Box(paint)
                        if [EARLIER, LATER].contains(&paint.decoration.background.rgb) =>
                    {
                        Some(paint)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                paints.len(),
                2,
                "each resumed descendant paints once, schedule={schedule}, placement={placement}"
            );
            assert_eq!(
                paints[0].rect, paints[1].rect,
                "the two descendants must overlap exactly"
            );
            assert_eq!(
                paints
                    .iter()
                    .map(|paint| paint.decoration.background.rgb)
                    .collect::<Vec<_>>(),
                if placement == "z-index" {
                    [LATER, EARLIER]
                } else {
                    [EARLIER, LATER]
                },
                "z-index precedes source order, which must not depend on completion order, schedule={schedule}, placement={placement}"
            );
        }
    }
}
