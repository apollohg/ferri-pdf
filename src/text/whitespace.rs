use crate::document::{OBJECT_REPLACEMENT_CHARACTER, Paragraph};
use crate::style::TextAlign;
use crate::text::{GlyphRun, line_boxes::Slot};
use icu_properties::{CodePointMapData, props::GeneralCategory};
use std::ops::Range;
use unicode_linebreak::{BreakClass, BreakOpportunity};

const NO_BREAK_SPACE: char = '\u{a0}';

/// Soft wrap and mandatory break opportunities, including the `automatic` hyphenation ones,
/// except where ruby forbids them.
pub(super) fn opportunities(
    paragraph: &Paragraph,
    automatic: &[usize],
) -> Vec<(usize, BreakOpportunity)> {
    let mut breaks = unconstrained(paragraph, automatic);
    crate::text::ruby::constrain(paragraph, &mut breaks);
    crate::text::combine::constrain(paragraph, &mut breaks, |entry| entry.0);
    breaks
}

fn unconstrained(paragraph: &Paragraph, automatic: &[usize]) -> Vec<(usize, BreakOpportunity)> {
    let mut breaks: Vec<_> = crate::text::tailoring::linebreaks(paragraph)
        .into_iter()
        .filter(|&(end, opportunity)| {
            opportunity == BreakOpportunity::Mandatory
                || (wraps(paragraph, end) && crate::text::hyphenation::allowed(paragraph, end))
        })
        .collect();
    breaks.extend(
        automatic
            .iter()
            .copied()
            .filter(|&offset| wraps(paragraph, offset))
            .map(|offset| (offset, BreakOpportunity::Allowed)),
    );
    breaks.extend(
        paragraph
            .optional_breaks()
            .iter()
            .copied()
            .filter(|&offset| offset > 0 && offset < paragraph.text().len())
            .map(|offset| (offset, BreakOpportunity::Allowed)),
    );
    let breaks_spaces = paragraph
        .spans()
        .iter()
        .any(|span| span.style.white_space.breaks_spaces());
    let categories = CodePointMapData::<GeneralCategory>::new();
    for (offset, ch) in paragraph.text().char_indices() {
        let end = offset + ch.len_utf8();
        if breaks_spaces
            && ch != NO_BREAK_SPACE
            && (ch == '\t' || categories.get(ch) == GeneralCategory::SpaceSeparator)
            && paragraph.style_at(offset).white_space.breaks_spaces()
        {
            breaks.push((end, BreakOpportunity::Allowed));
        }
        // Explicit hyphens also apply where UAX #14's Hebrew rule suppresses BA.
        if ch == crate::text::hyphenation::SOFT_HYPHEN
            && wraps(paragraph, end)
            && crate::text::hyphenation::allowed(paragraph, end)
        {
            breaks.push((end, BreakOpportunity::Allowed));
        }
    }
    if preserved_spaces_wrap_before(paragraph) {
        for span in paragraph.spans() {
            if !span.style.white_space.hangs_spaces() {
                continue;
            }
            let mut after_space = true;
            for (offset, ch) in paragraph.text()[span.range.clone()].char_indices() {
                if matches!(ch, ' ' | '\t') && !after_space {
                    breaks.push((span.range.start + offset, BreakOpportunity::Allowed));
                }
                after_space = matches!(ch, ' ' | '\t' | '\n' | '\u{2028}' | '\u{2029}');
            }
        }
    }
    for spans in paragraph.spans().windows(2) {
        let end = spans[0].range.end;
        if spans[0].style.white_space.collapses_spaces()
            && spans[0].style.white_space.wraps()
            && !spans[1].style.white_space.collapses_spaces()
            && paragraph.text().as_bytes().get(end - 1) == Some(&b' ')
            && paragraph.text().as_bytes().get(end) == Some(&b' ')
        {
            breaks.push((end, BreakOpportunity::Allowed));
        }
    }
    let text = paragraph.text();
    for atomic in paragraph.atomics() {
        let after = atomic.offset + OBJECT_REPLACEMENT_CHARACTER.len_utf8();
        let before = text[..atomic.offset].chars().next_back();
        let next = text[after..].chars().next();
        for (offset, neighbour) in [(atomic.offset, before), (after, next)] {
            if neighbour.is_some_and(|ch| !suppresses_atomic_break(ch)) && wraps(paragraph, offset)
            {
                breaks.push((offset, BreakOpportunity::Allowed));
            }
        }
    }
    let mut leaders = paragraph
        .spans()
        .iter()
        .filter(|span| span.leader)
        .peekable();
    if leaders.peek().is_some() {
        breaks.retain(|&(end, _)| {
            end == paragraph.text().len() || !paragraph.leader_at(end.saturating_sub(1))
        });
        breaks.extend(
            leaders
                .filter(|span| span.range.start > 0)
                .map(|span| (span.range.start, BreakOpportunity::Allowed)),
        );
    }
    // A mandatory break sorts first among equal offsets, so deduplication keeps it.
    breaks.sort_unstable_by_key(|&(offset, opportunity)| {
        (offset, opportunity == BreakOpportunity::Allowed)
    });
    breaks.dedup_by_key(|entry| entry.0);
    breaks
}

/// CSS Text 3 §5.1: atomic inlines have soft wrap opportunities on both sides, except
/// beside glue, word joiners and zero-width joiners other than U+00A0.
fn suppresses_atomic_break(ch: char) -> bool {
    ch != NO_BREAK_SPACE
        && matches!(
            unicode_linebreak::break_property(ch as u32),
            BreakClass::NonBreakingGlue | BreakClass::WordJoiner | BreakClass::ZeroWidthJoiner
        )
}

fn preserved_spaces_wrap_before(paragraph: &Paragraph) -> bool {
    paragraph.style().align == TextAlign::Justify
        && paragraph.style().white_space.collapses_spaces()
}

fn wraps(paragraph: &Paragraph, end: usize) -> bool {
    let left = paragraph.style_at(end - 1);
    left.white_space.wraps() || paragraph.style_at(end).white_space.wraps()
}

pub(super) fn visible(paragraph: &Paragraph, range: Range<usize>) -> Range<usize> {
    let text = paragraph.text();
    let mut start = range.start;
    let mut end = range.end;
    for (offset, ch) in text[range.clone()].char_indices() {
        if paragraph.leader_at(range.start + offset)
            || ch != ' '
            || !paragraph
                .style_at(range.start + offset)
                .white_space
                .collapses_spaces()
        {
            break;
        }
        start = range.start + offset + ch.len_utf8();
    }
    for (offset, ch) in text[start..end].char_indices().rev() {
        let index = start + offset;
        if paragraph.leader_at(index)
            || !matches!(ch, '\n' | '\u{2028}' | '\u{2029}')
                && (ch != ' ' || !paragraph.style_at(index).white_space.collapses_spaces())
        {
            break;
        }
        end = index;
    }
    start..end
}

pub(super) fn wrapping_width(
    paragraph: &Paragraph,
    end: usize,
    runs: &[GlyphRun],
    order: &[Slot],
    width: f32,
) -> f32 {
    if preserved_spaces_wrap_before(paragraph) {
        return width;
    }
    let rtl = paragraph.style().used_rtl();
    let mut hanging = 0.0;
    'trailing: for index in 0..order.len() {
        let index = if rtl { index } else { order.len() - 1 - index };
        let run = match order[index] {
            Slot::Edge { width: 0.0, .. } => continue,
            Slot::Run(index) => &runs[index],
            _ => break,
        };
        if run.leader || run.combined {
            break;
        }
        for index in 0..run.glyphs.len() {
            let index = if rtl {
                index
            } else {
                run.glyphs.len() - 1 - index
            };
            let glyph = &run.glyphs[index];
            let offset = run.source_range.start + glyph.text_range.start;
            if offset >= end || !run.source_range.contains(&offset) {
                break 'trailing;
            }
            if !matches!(paragraph.text().as_bytes()[offset], b' ' | b'\t')
                || !paragraph.style_at(offset).white_space.hangs_spaces()
            {
                break 'trailing;
            }
            hanging += glyph.advance(run.font_size);
        }
    }
    width - hanging
}

pub(super) fn emergency_opportunities(paragraph: &Paragraph, intrinsic: bool) -> Vec<usize> {
    use crate::style::OverflowWrap;
    use unicode_segmentation::UnicodeSegmentation;
    // CSS Text 4 §6.1: `word-break: break-word` acts as `overflow-wrap: anywhere`.
    let allows = |style: &crate::style::Style| {
        style.white_space.wraps()
            && match style.overflow_wrap {
                _ if style.line_breaking.word_break == crate::style::WordBreak::BreakWord => true,
                OverflowWrap::Normal => false,
                OverflowWrap::Anywhere => true,
                OverflowWrap::BreakWord => !intrinsic,
            }
    };
    if !paragraph.spans().iter().any(|span| allows(&span.style)) {
        return Vec::new();
    }
    let mut opportunities = paragraph
        .text()
        .grapheme_indices(true)
        .skip(1)
        .map(|(offset, _)| offset)
        .filter(|&offset| {
            !paragraph.leader_at(offset - 1)
                && (allows(paragraph.style_at(offset - 1)) || allows(paragraph.style_at(offset)))
        })
        .collect();
    crate::text::ruby::constrain_emergency(paragraph, &mut opportunities);
    crate::text::combine::constrain(paragraph, &mut opportunities, |offset| *offset);
    opportunities
}
