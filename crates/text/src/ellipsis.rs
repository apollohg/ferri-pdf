//! CSS Overflow 3 `text-overflow: ellipsis` (§3.1): hides whole grapheme clusters and
//! atomic inlines at a line's inline end so an ellipsis styled by the block fits.
use crate::line_boxes::{Slot, slot_width};
use crate::shaping::MissingGlyphs;
use crate::{GlyphRun, Line, TextContext, TextError};
use ferri_pdf_document::Paragraph;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

const ELLIPSIS: &str = "\u{2026}";
/// The permitted substitute when no font has an ellipsis character.
const THREE_DOTS: &str = "...";

/// Where truncation stops, in slots visited from the line's inline start.
struct Cut {
    /// Slots before this one are kept whole.
    slot: usize,
    /// Glyphs kept of the run at `slot`, which is otherwise hidden.
    partial: Option<Range<usize>>,
}

/// Ellipses a line that overflows `available` at its inline end, before it is placed.
/// Hidden content keeps its layout: only the line's painted content changes. A line whose
/// first character or atomic inline cannot precede the ellipsis is left to be clipped.
pub(crate) fn ellipsize(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    line: &mut Line,
    available: f32,
) -> Result<(), TextError> {
    let style = paragraph.style();
    if !style.ellipsizes() || line.width <= available {
        return Ok(());
    }
    let rtl = style.rtl;
    let mut ellipsis = match context.shape(
        ELLIPSIS,
        0..ELLIPSIS.len(),
        rtl,
        style,
        MissingGlyphs::Reject,
    ) {
        Err(TextError::MissingGlyph { .. }) => context.shape(
            THREE_DOTS,
            0..THREE_DOTS.len(),
            rtl,
            style,
            MissingGlyphs::Substitute,
        )?,
        shaped => shaped?,
    };
    let limit = available - ellipsis.iter().map(|run| run.width).sum::<f32>();
    let Some(cut) = find_cut(line, limit, rtl) else {
        return Ok(());
    };
    let visual: Vec<usize> = if rtl {
        (0..line.order.len()).rev().collect()
    } else {
        (0..line.order.len()).collect()
    };
    let mut kept: Vec<usize> = visual[..cut.slot].to_vec();
    if cut.partial.is_some() {
        kept.push(visual[cut.slot]);
    }
    if rtl {
        kept.reverse();
    }
    // The ellipsis stands for the hidden content, at the end of the line's text.
    let hidden_span = match line.order[visual[cut.slot]] {
        Slot::Run(index) => line.runs[index].source_span,
        _ => paragraph.spans().len().saturating_sub(1),
    };
    for run in &mut ellipsis {
        run.source_range = line.range.end..line.range.end;
        run.source_span = hidden_span;
    }
    let mut runs = Vec::new();
    let mut atomics = Vec::new();
    let mut order = Vec::with_capacity(kept.len() + ellipsis.len());
    let mut position = vec![None; line.order.len()];
    for &old in &kept {
        position[old] = Some(order.len());
        order.push(match line.order[old] {
            Slot::Run(index) => {
                let mut run = line.runs[index].clone();
                if old == visual[cut.slot]
                    && let Some(glyphs) = cut.partial.clone()
                {
                    run.glyphs = run.glyphs[glyphs].to_vec();
                    run.width = (0..run.glyphs.len()).map(|glyph| run.advance(glyph)).sum();
                }
                runs.push(run);
                Slot::Run(runs.len() - 1)
            }
            Slot::Atomic(index) => {
                atomics.push(line.atomics[index].clone());
                Slot::Atomic(atomics.len() - 1)
            }
            edge @ Slot::Edge { .. } => edge,
        });
    }
    line.ellipsis = Some(runs.len());
    let ellipsis_slots = (runs.len()..runs.len() + ellipsis.len()).map(Slot::Run);
    if rtl {
        order.splice(0..0, ellipsis_slots);
        position
            .iter_mut()
            .flatten()
            .for_each(|slot| *slot += ellipsis.len());
    } else {
        order.extend(ellipsis_slots);
    }
    runs.extend(ellipsis);
    let hidden_side = usize::from(!rtl);
    line.boxes.retain_mut(|fragment| {
        let slots: Vec<usize> = fragment
            .slots
            .clone()
            .filter_map(|old| position[old])
            .collect();
        let (Some(&first), Some(&last)) = (slots.first(), slots.last()) else {
            return false;
        };
        if slots.len() < fragment.slots.len() {
            fragment.edges[hidden_side] = false;
        }
        fragment.slots = first..last + 1;
        true
    });
    line.width = order
        .iter()
        .map(|&slot| slot_width(&runs, &atomics, slot))
        .sum();
    line.runs = runs;
    line.atomics = atomics;
    line.order = order;
    Ok(())
}

/// The first unit, visited from the inline start, whose far edge passes `limit`. Box start
/// edges left without content are hidden with it.
fn find_cut(line: &Line, limit: f32, rtl: bool) -> Option<Cut> {
    let mut lefts = Vec::with_capacity(line.order.len());
    let mut x = 0.0;
    for &slot in &line.order {
        lefts.push(x);
        x += slot_width(&line.runs, &line.atomics, slot);
    }
    let visual: Vec<usize> = if rtl {
        (0..line.order.len()).rev().collect()
    } else {
        (0..line.order.len()).collect()
    };
    let mut content = false;
    for (step, &index) in visual.iter().enumerate() {
        let slot = line.order[index];
        let width = slot_width(&line.runs, &line.atomics, slot);
        let near = if rtl {
            line.width - lefts[index] - width
        } else {
            lefts[index]
        };
        if near + width <= limit {
            content |= !matches!(slot, Slot::Edge { .. });
            continue;
        }
        let partial = match slot {
            Slot::Run(run) => fitting_clusters(&line.runs[run], limit - near, rtl),
            _ => None,
        };
        if !content && partial.is_none() {
            return None;
        }
        let mut step = step;
        if partial.is_none() {
            while step > 0
                && matches!(line.order[visual[step - 1]], Slot::Edge { left, .. } if left != rtl)
            {
                step -= 1;
            }
        }
        return Some(Cut {
            slot: step,
            partial,
        });
    }
    None
}

/// Glyphs of `run` from the inline start (its right end when `from_right`) that fit in
/// `room`, ending at a grapheme cluster boundary; `None` when not even one cluster fits.
fn fitting_clusters(run: &GlyphRun, room: f32, from_right: bool) -> Option<Range<usize>> {
    let count = run.glyphs.len();
    let boundaries: Vec<usize> = run
        .text
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(run.text.len()))
        .collect();
    let glyph = |step: usize| if from_right { count - 1 - step } else { step };
    let mut width = 0.0;
    let mut fitting = 0;
    for step in 0..count {
        width += run.advance(glyph(step));
        if width > room {
            break;
        }
        let ends_cluster = step + 1 == count || {
            let [this, next] = [step, step + 1].map(|step| &run.glyphs[glyph(step)].text_range);
            this != next && boundaries.contains(&this.start.max(next.start))
        };
        if ends_cluster {
            fitting = step + 1;
        }
    }
    (fitting > 0).then(|| {
        if from_right {
            count - fitting..count
        } else {
            0..fitting
        }
    })
}
