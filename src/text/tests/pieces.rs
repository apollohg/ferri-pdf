//! Pieces of spans a text context shapes once for the layouts that repeat them.
use super::{document, first_paragraph};
use crate::text::TextContext;
use crate::work::{self, Work};

/// Words in two styles, with ideographs the primary face lacks so that glyph matching probes
/// other faces and spends work.
const BODY: &str = "<p>alpha 漢字 beta <b>gamma 水</b> delta epsilon <i>zeta 字 eta</i> theta \
                    iota kappa 漢 lambda mu</p>";
/// Narrow enough that the words take several lines.
const WIDTH: f32 = 60.0;

#[test]
fn a_paragraph_laid_out_again_at_one_width_reshapes_nothing_once_its_pieces_repeat() {
    // Flex and grid items lay paragraphs out again at widths they used before. The pieces
    // that end candidate lines shape alike each time, so the text context keeps them once
    // they shape a second time, and a reused piece spends the glyph matching work again.
    let document = document(BODY);
    let paragraph = first_paragraph(document.root()).unwrap();
    let resolver = crate::tests::common::fixture_resolver();
    let context = TextContext::new(&document.styles().fonts, &resolver);
    let layout = || {
        let before = context.matching_work.get().unwrap();
        let (lines, runs) = work::measured(Work::ShapedRun, || {
            context.layout_paragraph(paragraph, WIDTH, &[]).unwrap()
        });
        let glyphs: Vec<Vec<u32>> = lines
            .iter()
            .map(|line| {
                line.runs
                    .iter()
                    .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.id))
                    .collect()
            })
            .collect();
        (glyphs, runs, before - context.matching_work.get().unwrap())
    };
    let (first, first_runs, first_work) = layout();
    let (second, second_runs, second_work) = layout();
    let (third, third_runs, third_work) = layout();
    assert!(first.len() > 1, "the words take {} lines", first.len());
    assert!(
        first_runs > 0 && first_work > 0,
        "the first layout shapes {first_runs} runs and spends {first_work} matching work"
    );
    assert!(
        second_runs < first_runs,
        "the second layout reshapes only the pieces ending candidate lines: {second_runs} of \
         {first_runs} runs"
    );
    assert_eq!(third_runs, 0, "the third layout reuses every piece");
    assert_eq!(
        (&second, &third),
        (&first, &first),
        "every layout sets the same glyphs"
    );
    assert_eq!(
        (second_work, third_work),
        (first_work, first_work),
        "every layout spends the same glyph matching work"
    );
}
