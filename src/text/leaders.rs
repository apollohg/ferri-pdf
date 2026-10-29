use crate::document::Paragraph;
use crate::text::{
    GlyphRun, Line, TextContext, TextError,
    line_boxes::{Slot, slot_width},
};
use std::ops::Range;

fn leader(paragraph: &Paragraph, runs: &[GlyphRun], slot: Slot) -> Option<usize> {
    match slot {
        Slot::Run(index) => {
            let span = runs[index].source_span;
            paragraph.spans()[span].leader.then_some(span)
        }
        _ => None,
    }
}
fn groups(paragraph: &Paragraph, runs: &[GlyphRun], order: &[Slot]) -> Vec<Range<usize>> {
    let mut groups = Vec::<Range<usize>>::new();
    for (index, &slot) in order.iter().enumerate() {
        let Some(span) = leader(paragraph, runs, slot) else {
            continue;
        };
        if let Some(last) = groups.last_mut()
            && last.end == index
            && leader(paragraph, runs, order[last.start]) == Some(span)
        {
            last.end += 1;
        } else {
            groups.push(index..index + 1);
        }
    }
    groups
}
fn pattern_width(runs: &[GlyphRun], order: &[Slot], group: Range<usize>) -> f32 {
    order[group]
        .iter()
        .map(|slot| match slot {
            Slot::Run(index) => runs[*index].width,
            _ => unreachable!("leader groups contain glyph runs"),
        })
        .sum()
}
fn expansion_share(
    groups: &[Range<usize>],
    runs: &[GlyphRun],
    order: &[Slot],
    remaining: f32,
) -> f32 {
    let advancing = groups
        .iter()
        .filter(|group| pattern_width(runs, order, (*group).clone()) > 0.0)
        .count();
    remaining.max(0.0) / advancing.max(1) as f32
}

fn phase(left: f32, right: f32, width: f32, available: f32, rtl: bool) -> (f32, bool) {
    let origin = if rtl { 0.0 } else { available };
    let first = origin + ((left - origin) / width).ceil() * width;
    let fits = first + width <= right + available.abs().max(1.0) * f32::EPSILON;
    (
        if fits {
            origin
        } else if rtl {
            left
        } else {
            right
        },
        fits,
    )
}

pub(super) fn wrapping_width(
    paragraph: &Paragraph,
    arranged: &crate::text::line_boxes::Arranged,
    available: f32,
    from: usize,
) -> Option<f32> {
    let groups = groups(paragraph, &arranged.runs, &arranged.order);
    let extra = expansion_share(
        &groups,
        &arranged.runs,
        &arranged.order,
        available - arranged.width,
    );
    let mut left = 0.0;
    let mut slot = 0;
    for group in groups {
        while slot < group.start {
            left += slot_width(&arranged.runs, &arranged.atomics, arranged.order[slot]);
            slot += 1;
        }
        let width = pattern_width(&arranged.runs, &arranged.order, group.clone());
        let span = leader(paragraph, &arranged.runs, arranged.order[group.start]).unwrap();
        if width > 0.0
            && paragraph.spans()[span].range.start > from
            && !phase(
                left,
                left + width + extra,
                width,
                available,
                paragraph.style().rtl,
            )
            .1
        {
            return Some(available + width);
        }
        left += width + if width > 0.0 { extra } else { 0.0 };
        slot = group.end;
    }
    None
}

pub(super) fn alone(line: &Line) -> bool {
    line.runs.iter().any(|run| run.leader)
        && !line.order.iter().any(|slot| match slot {
            Slot::Run(index) => {
                !line.runs[*index].leader && !line.runs[*index].text.trim().is_empty()
            }
            Slot::Atomic(_) => true,
            Slot::Edge { .. } | Slot::Space { .. } => false,
        })
}

pub(super) fn first_content_end(paragraph: &Paragraph, start: usize) -> Option<usize> {
    let spans = paragraph.spans();
    let first = spans.partition_point(|span| span.range.end <= start);
    spans[first..]
        .iter()
        .filter(|span| !span.leader)
        .find_map(|span| {
            let from = span.range.start.max(start);
            paragraph.text()[from..span.range.end]
                .char_indices()
                .find(|(_, ch)| !ch.is_whitespace())
                .map(|(offset, ch)| from + offset + ch.len_utf8())
        })
}

pub(super) fn expand(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    line: &mut Line,
    available: f32,
) -> Result<(), TextError> {
    let groups = groups(paragraph, &line.runs, &line.order);
    if groups.is_empty() {
        return Ok(());
    }
    if alone(line) {
        for run in &mut line.runs {
            if run.leader {
                line.width -= run.width;
                run.width = 0.0;
                run.text.clear();
                run.glyphs.clear();
            }
        }
        line.text.clear();
        return Ok(());
    }
    let extra = expansion_share(&groups, &line.runs, &line.order, available - line.width);
    let old_order = std::mem::take(&mut line.order);
    let mut runs = Vec::new();
    let mut mapped = vec![0; old_order.len() + 1];
    let mut position = 0.0;
    let mut remaining = crate::document::MAX_INLINE_TEXT_BYTES.saturating_sub(line.text.len());
    let mut index = 0;
    let mut groups = groups.into_iter().peekable();
    while index < old_order.len() {
        mapped[index] = line.order.len();
        if let Some(group) = groups.next_if(|group| group.start == index) {
            let pattern: Vec<_> = old_order[group.clone()]
                .iter()
                .map(|slot| match slot {
                    Slot::Run(index) => &line.runs[*index],
                    _ => unreachable!("leader groups contain glyph runs"),
                })
                .collect();
            let width: f32 = pattern.iter().map(|run| run.width).sum();
            let extent = width + if width > 0.0 { extra } else { 0.0 };
            if width > 0.0 && width.is_finite() && extent.is_finite() {
                let left = position;
                let right = position + extent;
                let origin = phase(left, right, width, available, paragraph.style().rtl).0;
                let start = ((left - origin) / width).floor();
                let end = ((right - origin) / width).ceil();
                let count = (end - start).max(0.0) as usize;
                let bytes: usize = pattern.iter().map(|run| run.text.len()).sum();
                remaining = count
                    .checked_mul(bytes)
                    .and_then(|bytes| remaining.checked_sub(bytes))
                    .ok_or(TextError::LeaderExpansionLimit {
                        limit: crate::document::MAX_INLINE_TEXT_BYTES,
                    })?;
                let glyphs: usize = pattern.iter().map(|run| run.glyphs.len()).sum();
                context.spend_matching_work(
                    count
                        .checked_mul(glyphs)
                        .ok_or(TextError::NonFiniteGeometry)?,
                )?;
                let mut painted = left;
                for repeat in 0..count {
                    let mut at = origin + (start + repeat as f32) * width;
                    for run in &pattern {
                        let mut cropped = (*run).clone();
                        cropped.glyphs.clear();
                        let mut first = None;
                        let mut last = at;
                        for glyph in &run.glyphs {
                            let end = at + glyph.advance(run.font_size);
                            if at.min(end) >= left - available.abs().max(1.0) * f32::EPSILON
                                && at.max(end) <= right + available.abs().max(1.0) * f32::EPSILON
                            {
                                first.get_or_insert(at);
                                last = end;
                                cropped.glyphs.push(glyph.clone());
                            }
                            at = end;
                        }
                        if let Some(first) = first {
                            push_gap(&mut runs, &mut line.order, run, first - painted);
                            let lo = cropped
                                .glyphs
                                .iter()
                                .map(|g| g.text_range.start)
                                .min()
                                .unwrap();
                            let hi = cropped
                                .glyphs
                                .iter()
                                .map(|g| g.text_range.end)
                                .max()
                                .unwrap();
                            cropped.text.truncate(hi);
                            cropped.text.drain(..lo);
                            for glyph in &mut cropped.glyphs {
                                glyph.text_range.start -= lo;
                                glyph.text_range.end -= lo;
                            }
                            cropped.width = last - first;
                            line.order.push(Slot::Run(runs.len()));
                            runs.push(cropped);
                            painted = last;
                        }
                    }
                }
                push_gap(&mut runs, &mut line.order, pattern[0], right - painted);
            } else {
                for run in pattern {
                    line.order.push(Slot::Run(runs.len()));
                    runs.push(run.clone());
                }
            }
            for mapped in &mut mapped[group.start + 1..group.end] {
                *mapped = line.order.len();
            }
            index = group.end;
            position += extent;
        } else {
            let slot = old_order[index];
            position += slot_width(&line.runs, &line.atomics, slot);
            line.order.push(match slot {
                Slot::Run(index) => {
                    runs.push(line.runs[index].clone());
                    Slot::Run(runs.len() - 1)
                }
                other => other,
            });
            index += 1;
        }
    }
    mapped[old_order.len()] = line.order.len();
    for fragment in &mut line.boxes {
        fragment.slots = mapped[fragment.slots.start]..mapped[fragment.slots.end];
    }
    line.runs = runs;
    if !position.is_finite() {
        return Err(TextError::NonFiniteGeometry);
    }
    line.width = position;
    let visible = crate::text::whitespace::visible(paragraph, line.range.clone());
    let mut leaders = std::collections::BTreeMap::<usize, Vec<&GlyphRun>>::new();
    for run in &line.runs {
        if run.leader {
            leaders.entry(run.source_span).or_default().push(run);
        }
    }
    let mut text = String::new();
    let mut source = visible.start;
    let mut original = 0;
    for (index, mut runs) in leaders {
        let span = &paragraph.spans()[index];
        let before =
            crate::text::hyphenation::text(&paragraph.text()[source..span.range.start], None).len();
        text.push_str(&line.text[original..original + before]);
        original += before
            + crate::text::hyphenation::text(&paragraph.text()[span.range.clone()], None).len();
        if span.style.rtl {
            runs.reverse();
        }
        for run in runs {
            text.push_str(&run.text);
        }
        source = span.range.end;
    }
    text.push_str(&line.text[original..]);
    line.text = text;
    Ok(())
}

fn push_gap(runs: &mut Vec<GlyphRun>, order: &mut Vec<Slot>, pattern: &GlyphRun, width: f32) {
    if width <= 0.0 {
        return;
    }
    let mut gap = pattern.clone();
    gap.text.clear();
    gap.glyphs.clear();
    gap.width = width;
    order.push(Slot::Run(runs.len()));
    runs.push(gap);
}
