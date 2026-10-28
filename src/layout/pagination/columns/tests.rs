use super::*;
use crate::document::StyledDocument;
use crate::resources::FileResolver;
use crate::text::TextContext;

impl<'a, 't> ColumnLayout<'a, 't> {
    fn complete(mut self, measurer: &mut Measurer<'t>) -> Result<ColumnOutput<'a>, LayoutError> {
        while self.cursor.is_some() {
            self = self.advance(measurer)?;
        }
        Ok(self.into_output(measurer))
    }
}

fn measured_columns(
    block: crate::layout::measure::MeasuredBlock<'_>,
) -> Option<MeasuredColumns<'_>> {
    for item in block.items {
        match item {
            FlowItem::Columns(columns) => return Some(*columns),
            FlowItem::Block(child) => {
                if let Some(columns) = measured_columns(*child) {
                    return Some(columns);
                }
            }
            _ => {}
        }
    }
    None
}

fn paint_signature(output: &ColumnOutput<'_>) -> Vec<String> {
    output
        .display_lists
        .iter()
        .chain(&output.foreground)
        .map(|page| {
            page.iter()
                .map(|paint| {
                    format!(
                        "{:?}/{:?}/{:?}/{:?}/{:?}",
                        paint.item, paint.scope, paint.order, paint.clip, paint.group
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

#[test]
fn retained_column_layouts_release_the_measurer_and_preserve_independent_source_state() {
    const COLUMN_HEIGHT: f32 = 30.0;
    let resolver = FileResolver::default();
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/oracle/fixtures")
        .into();
    let words = (0..80)
        .map(|index| format!("S{index:03}"))
        .collect::<Vec<_>>();
    for variant in [
        "paragraph",
        "table",
        "columns",
        "positioned-cell",
        "positioned-group",
        "nested-positioned-group",
    ] {
        let content = match variant {
            "positioned-cell" => format!(
                "<table><tr><td style='position:relative'><b><i></i></b></td><td><span>{}</span></td></tr></table>",
                words.join(" ")
            ),
            "positioned-group" => format!(
                "<table><tbody style='position:relative'><tr><td><b><i></i></b></td><td><span>{}</span></td></tr></tbody></table>",
                words.join(" ")
            ),
            "nested-positioned-group" => format!(
                "<table><tr><td><table><tbody style='position:relative'><tr><td><b><i></i></b></td><td><span>{}</span></td></tr></tbody></table></td></tr></table>",
                words.join(" ")
            ),
            "table" => format!(
                "<table><tr><td><span>{}</span></td><td><i></i></td></tr></table>",
                words.join(" ")
            ),
            "columns" => format!("<div class=nested><span>{}</span></div>", words.join(" ")),
            _ => format!("<span>{}</span>", words.join(" ")),
        };
        let document = StyledDocument::parse(
        &format!("<!doctype html><style>@font-face{{font-family:Fixed;src:url('../assets/DejaVuSans.ttf')}}@page{{size:200pt 100pt;margin:0}}html,body,section,div,p{{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}}section{{column-count:2;column-gap:10pt;column-fill:auto}}.inside{{background:#ddd;opacity:.5;overflow:hidden}}span{{background:yellow}}b{{position:absolute;width:5pt;height:5pt;background:red}}table{{width:100%;border-spacing:0;table-layout:fixed}}td{{padding:0;vertical-align:top}}.nested{{column-count:2;column-gap:5pt;column-fill:auto}}i{{display:block;height:10pt;background:blue}}</style><section><div class=inside>{content}<b></b></div></section>"),
        &base, &[], &resolver,
    ).unwrap();
        let text = TextContext::new(&document.styles().fonts, &resolver);
        let area = document.page_box().geometry.content_box();
        let mut measurer = Measurer {
            work: crate::layout::work::Budget::new(crate::layout::MAX_LAYOUT_WORK),
            text: &text,
            canvas_images: false,
            page_height: area.height,
            sources: crate::layout::source::SourceIds::new(document.root()),
            item_metrics: Default::default(),
            grid_parent: None,
            clearance_targets: Default::default(),
            fragmentainer: crate::layout::breaks::Fragmentainer::Page,
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
        let columns =
            measured_columns(measured).expect("fixture must produce a real measured column flow");
        let [ColumnSegment::Row(items)] = columns.segments.as_slice() else {
            panic!("fixture must have one row segment");
        };
        let mut rows = ColumnRows::new(columns.geometry, COLUMN_HEIGHT);
        rows.frames.push(RowFrame {
            geometry: columns.geometry,
            first: 0,
            offset: [0.0; 2],
            height: COLUMN_HEIGHT,
            unbounded: false,
            page_rect: UNPLACED_PAGE,
        });
        let start = FlowStart {
            track_overflow: false,
            offset: [0.0; 2],
            key: Rc::from([FLOW_LEVEL]),
            context: Rc::from([]),
            orders: PaintOrders::default(),
            markers: Vec::new(),
        };
        let expected = ColumnLayout::new(&mut measurer, items, rows.clone(), &start)
            .unwrap()
            .complete(&mut measurer)
            .unwrap();
        let lines = |output: &ColumnOutput<'_>| {
            output
                .pages
                .iter()
                .flatten()
                .map(|line| {
                    (
                        line.paragraph,
                        line.text.clone(),
                        line.x,
                        line.baseline,
                        line.width,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert!(
            expected.pages.len() > columns.geometry.count,
            "fixture must span multiple physical rows"
        );
        assert!(
            !expected.clips.is_empty(),
            "fixture must retain opacity and clipping state"
        );
        assert!(
            !expected.pending.is_empty(),
            "fixture must retain the external positioned anchor"
        );
        let expected_words = expected
            .pages
            .iter()
            .flatten()
            .flat_map(|line| line.text.split_whitespace())
            .collect::<Vec<_>>();
        assert_eq!(expected_words, words);
        for schedule in [[0, 1], [1, 0]] {
            let mut layouts = (0..2)
                .map(|_| {
                    Some(ColumnLayout::new(&mut measurer, items, rows.clone(), &start).unwrap())
                })
                .collect::<Vec<_>>();
            let mut outputs = Vec::new();
            let mut advances = 0;
            while layouts.iter().any(Option::is_some) {
                for index in schedule {
                    if let Some(layout) = layouts[index].take() {
                        measurer.work.spend(1).unwrap();
                        let layout = layout.advance(&mut measurer).unwrap();
                        advances += 1;
                        if layout.cursor.is_none() {
                            outputs.push(layout.into_output(&mut measurer));
                        } else {
                            layouts[index] = Some(layout);
                        }
                    }
                }
            }
            assert!(
                advances > 2,
                "both layouts must suspend, not complete in one advance"
            );
            assert_eq!(outputs.len(), 2);
            for output in outputs {
                assert_eq!(
                    lines(&output),
                    lines(&expected),
                    "schedule {schedule:?} changed source progress or line geometry"
                );
                assert_eq!(
                    paint_signature(&output),
                    paint_signature(&expected),
                    "schedule {schedule:?} changed decoration or paint order"
                );
                assert_eq!(output.pending.len(), expected.pending.len());
                for (actual, expected) in output.pending.iter().zip(&expected.pending) {
                    assert!(std::ptr::eq(actual.block, expected.block));
                    assert_eq!(
                        format!("{:?}", actual.position),
                        format!("{:?}", expected.position)
                    );
                    assert_eq!(actual.order, expected.order);
                    assert_eq!(actual.context, expected.context);
                    assert_eq!(actual.group, expected.group);
                }
                assert_eq!(
                    output
                        .clips
                        .iter()
                        .map(|clip| &clip.fragments)
                        .collect::<Vec<_>>(),
                    expected
                        .clips
                        .iter()
                        .map(|clip| &clip.fragments)
                        .collect::<Vec<_>>()
                );
            }
        }
        let checkpoint = ColumnLayout::new(&mut measurer, items, rows.clone(), &start)
            .unwrap()
            .advance(&mut measurer)
            .unwrap();
        assert!(
            checkpoint.cursor.is_some(),
            "snapshot must capture an active flow"
        );
        for trial in 0..2 {
            let mut candidate = checkpoint.snapshot(&measurer.work).unwrap();
            while candidate.cursor.is_some() {
                candidate = candidate.advance(&mut measurer).unwrap();
            }
            let output = candidate.into_output(&mut measurer);
            assert_eq!(
                lines(&output),
                lines(&expected),
                "trial {trial} changed source progress"
            );
            assert_eq!(
                paint_signature(&output),
                paint_signature(&expected),
                "trial {trial} changed paint order or decorations"
            );
        }
        let mut committed = checkpoint;
        while committed.cursor.is_some() {
            committed = committed.advance(&mut measurer).unwrap();
        }
        let output = committed.into_output(&mut measurer);
        assert_eq!(
            lines(&output),
            lines(&expected),
            "discarded trials changed committed source progress"
        );
        assert_eq!(
            paint_signature(&output),
            paint_signature(&expected),
            "discarded trials changed committed paint order"
        );
        let layout = ColumnLayout::new(&mut measurer, items, rows, &start)
            .unwrap()
            .advance(&mut measurer)
            .unwrap();
        assert!(
            layout.cursor.is_some(),
            "budget check requires a suspended source"
        );
        assert!(measurer.work.spend(crate::layout::MAX_LAYOUT_WORK).is_err());
        assert!(
            matches!(
                layout.advance(&mut measurer),
                Err(LayoutError::WorkLimit { .. })
            ),
            "resuming must use the exhausted shared budget, not a new allowance"
        );
    }
}
