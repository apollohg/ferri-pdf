//! CSS Text 4 §9.2 `hanging-punctuation`. A hanging glyph is not measured for fit, alignment,
//! justification or intrinsic sizes (CSS Text 3 §8.2), so it may sit outside the line box.
use crate::document::Paragraph;
use crate::style::HangingEnd;
use crate::text::line_boxes::Slot;
use crate::text::{GlyphRun, hyphenation::SOFT_HYPHEN};
use icu_properties::{CodePointMapData, props::GeneralCategory};
use std::ops::Range;

/// The stops and commas that may hang at the end of a line.
const STOPS_AND_COMMAS: [char; 13] = [
    '\u{002c}', '\u{002e}', '\u{060c}', '\u{06d4}', '\u{3001}', '\u{3002}', '\u{ff0c}', '\u{ff0e}',
    '\u{fe50}', '\u{fe51}', '\u{fe52}', '\u{ff61}', '\u{ff64}',
];
/// Quotes that open or close in addition to the Pi, Pf, Ps and Pe categories.
const ASCII_QUOTES: [char; 2] = ['\'', '"'];
pub(super) const IDEOGRAPHIC_SPACE: char = '\u{3000}';

/// Punctuation that may hang at a line's logical start and end, by advance.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Hangable {
    start: f32,
    end: f32,
    /// `allow-end` hangs only the part of the end glyph that does not otherwise fit.
    conditional: bool,
}

impl Hangable {
    /// Advance excluded when measuring the line for fit.
    pub(super) fn fit(self) -> f32 {
        self.start + self.end
    }

    /// Advance excluded from intrinsic sizes; a conditional glyph counts for max-content.
    pub(super) fn intrinsic(self, min_content: bool) -> f32 {
        self.start
            + if self.conditional && !min_content {
                0.0
            } else {
                self.end
            }
    }

    /// The hanging advance at the line's left and right edges, given its measured width.
    pub(super) fn hanging(self, width: f32, available: f32, rtl: bool) -> [f32; 2] {
        let end = if self.conditional {
            (width - self.start - available).clamp(0.0, self.end)
        } else {
            self.end
        };
        if rtl {
            [end, self.start]
        } else {
            [self.start, end]
        }
    }
}

fn opening(ch: char) -> bool {
    matches!(
        CodePointMapData::<GeneralCategory>::new().get(ch),
        GeneralCategory::OpenPunctuation
            | GeneralCategory::InitialPunctuation
            | GeneralCategory::FinalPunctuation
    ) || ASCII_QUOTES.contains(&ch)
        || ch == IDEOGRAPHIC_SPACE
}

fn closing(ch: char) -> bool {
    matches!(
        CodePointMapData::<GeneralCategory>::new().get(ch),
        GeneralCategory::ClosePunctuation
            | GeneralCategory::InitialPunctuation
            | GeneralCategory::FinalPunctuation
    ) || ASCII_QUOTES.contains(&ch)
}

/// The visual edge glyph cluster of a run: its paragraph offset, character and advance.
fn edge_cluster(paragraph: &Paragraph, run: &GlyphRun, right: bool) -> Option<(usize, char, f32)> {
    let edge = if right {
        run.glyphs.len().checked_sub(1)?
    } else {
        0
    };
    let range = run.glyphs[edge].text_range.clone();
    let offset = run.source_range.start + range.start;
    if run.leader || !run.source_range.contains(&offset) {
        return None;
    }
    let ch = paragraph.text()[offset..].chars().next()?;
    let width = (0..run.glyphs.len())
        .filter(|&index| run.glyphs[index].text_range == range)
        .map(|index| run.advance(index))
        .sum();
    (ch != SOFT_HYPHEN).then_some((offset, ch, width))
}

/// The punctuation at the edges of the line `range`, whose content is `visible`.
pub(super) fn hangable(
    paragraph: &Paragraph,
    range: Range<usize>,
    visible: Range<usize>,
    runs: &[GlyphRun],
    order: &[Slot],
) -> Hangable {
    let mut hangable = Hangable::default();
    if !paragraph
        .spans()
        .iter()
        .any(|span| span.style.line_breaking.hanging_punctuation != Default::default())
    {
        return hangable;
    }
    let rtl = paragraph.style().rtl;
    let edge_run = |right: bool| {
        let mut slots = order
            .iter()
            .filter(|slot| !matches!(slot, Slot::Edge { width, .. } if *width == 0.0));
        let slot = if right {
            slots.next_back()
        } else {
            slots.next()
        };
        match slot {
            Some(Slot::Run(index)) => Some(&runs[*index]),
            _ => None,
        }
    };
    if let Some((offset, ch, width)) =
        edge_run(rtl).and_then(|run| edge_cluster(paragraph, run, rtl))
        && range.start == 0
        && paragraph.first_formatted_line()
        && offset == visible.start
        && opening(ch)
        && paragraph
            .style_at(offset)
            .line_breaking
            .hanging_punctuation
            .first
    {
        hangable.start = width;
    }
    if let Some((offset, ch, width)) =
        edge_run(!rtl).and_then(|run| edge_cluster(paragraph, run, !rtl))
        && offset + ch.len_utf8() == visible.end
    {
        let hanging = paragraph.style_at(offset).line_breaking.hanging_punctuation;
        if let Some(end) = hanging.end.filter(|_| STOPS_AND_COMMAS.contains(&ch)) {
            hangable.end = width;
            hangable.conditional = end == HangingEnd::Allow;
        } else if hanging.last
            && closing(ch)
            && range.end == paragraph.text().len()
            && paragraph.last_formatted_line()
        {
            hangable.end = width;
        }
    }
    hangable
}
