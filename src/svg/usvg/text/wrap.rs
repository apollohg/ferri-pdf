//! Auto-wrapped SVG text (SVG 2 §11.4, §11.7): a text element with a wrapping area breaks each
//! of its paragraphs into lines where the document's line breaking allows, and each line
//! becomes a chunk that starts a line box and is anchored by `text-anchor` in the area.
use std::ops::Range;
use std::sync::Arc;

use fontdb::Database;
use unicode_linebreak::BreakOpportunity;

use super::layout::{self, FontsCache};
use crate::style::TextOverflow;
use crate::svg::usvg::{
    FontProvider, NonZeroRect, Strut, Text, TextAnchor, TextChunk, TextFlow, WrapWidth, Wrapping,
    WritingMode,
};

/// CSS Overflow 4 §4.1: the ellipsis `text-overflow: ellipsis` renders.
const ELLIPSIS: &str = "\u{2026}";

/// `text` with each paragraph of its linear chunks broken into lines no wider than its wrapping
/// area, and the area `text-overflow: clip` keeps. Chunks on a text path keep their layout.
/// The first line takes the characters that fit it as `::first-line` and `::first-letter`
/// style them, and the next lines continue with the rest as their own styles set it (CSS
/// Pseudo-Elements 4 §2.1).
pub(crate) fn wrap(
    text: &Text,
    wrapping: Wrapping,
    fonts: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<Text> {
    let paragraphs_of_text = paragraphs(text, &text.chunks, &text.strut, fonts, resolver, fontdb)?;
    // The chunks before the first preserved segment break may share the first line.
    let first_line = match &text.first_line {
        Some(line) => {
            let count = text
                .chunks
                .iter()
                .skip(1)
                .take_while(|chunk| chunk.line_breaks == 0)
                .count()
                + 1;
            let chunks = &line.chunks[..count.min(line.chunks.len())];
            paragraphs(text, chunks, &line.strut, fonts, resolver, fontdb)?
        }
        None => Vec::new(),
    };
    let width = match wrapping.width {
        WrapWidth::Length(width) => width,
        // The first line holds the first segment, as the first line styles it.
        WrapWidth::MinContent => paragraphs_of_text
            .iter()
            .enumerate()
            .flat_map(|(index, paragraph)| {
                let first = (index == 0).then(|| first_line.first()).flatten();
                let skipped = usize::from(first.is_some());
                first
                    .into_iter()
                    .flat_map(|line| line.segments().take(1))
                    .chain(paragraph.segments().skip(skipped))
            })
            .fold(0.0, f32::max),
    };
    let mut chunks = Vec::new();
    let mut overflows = false;
    let mut on_first_line = true;
    for (index, (paragraph, chunk)) in paragraphs_of_text.iter().zip(&text.chunks).enumerate() {
        on_first_line &= index == 0 || chunk.line_breaks == 0;
        let first = first_line.get(index).filter(|_| on_first_line);
        if !matches!(chunk.text_flow, TextFlow::Linear) {
            chunks.push(first.map_or(chunk, |line| line.chunk).clone());
            continue;
        }
        let lines = paragraph.lines(text, width, wrapping, first, fonts, resolver, fontdb)?;
        on_first_line &= lines.len() == 1;
        for (line, overflowing) in lines {
            overflows |= overflowing;
            chunks.push(line);
        }
    }
    // SVG 2 §11.10.2.10: `clip` clips overflowing text to the area whatever its `overflow`.
    let clip = (overflows && wrapping.overflow.contains(&TextOverflow::Clip))
        .then(|| clip_area(text, width, &chunks))
        .flatten();
    Some(Text {
        chunks,
        clip,
        ..text.clone()
    })
}

/// The paragraphs of `chunks`, the first chunks of `text` or of its first line, whose root
/// inline box is `strut`.
fn paragraphs<'a>(
    text: &Text,
    chunks: &'a [TextChunk],
    strut: &Strut,
    fonts: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<Vec<Paragraph<'a>>> {
    chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| match chunk.text_flow {
            TextFlow::Linear => {
                let edges = [index == 0, index + 1 == text.chunks.len()];
                Paragraph::of(text, chunk, edges, strut, fonts, resolver, fontdb)
            }
            TextFlow::Path(_) => Some(Paragraph::unbroken(chunk)),
        })
        .collect()
}

/// The wrapping area, infinite in the block direction, around the first line's anchor point.
fn clip_area(text: &Text, width: f32, chunks: &[TextChunk]) -> Option<NonZeroRect> {
    let first = chunks.first()?;
    let (x, y) = (first.x.unwrap_or(0.0), first.y.unwrap_or(0.0));
    let start = match first.anchor {
        TextAnchor::Start => 0.0,
        TextAnchor::Middle => -width / 2.0,
        TextAnchor::End => -width,
    };
    // The block direction's extent stands in for infinity within single-precision geometry.
    let unbounded = f32::MAX.sqrt();
    match text.writing_mode {
        WritingMode::LeftToRight => {
            NonZeroRect::from_xywh(x + start, -unbounded / 2.0, width, unbounded)
        }
        WritingMode::TopToBottom => {
            NonZeroRect::from_xywh(-unbounded / 2.0, y + start, unbounded, width)
        }
    }
}

/// One paragraph of a text element: a linear chunk, measured for breaking.
struct Paragraph<'a> {
    chunk: &'a TextChunk,
    /// The chunk as the document's line breaking sees it.
    breaking: Option<crate::document::Paragraph>,
    opportunities: crate::text::BreakOpportunities,
    /// Each cluster's source start and advance, in source order.
    advances: Vec<(usize, f32)>,
    /// The text element's `text-indent`, where it applies to the paragraph's first line.
    indent: Option<crate::style::TextIndent>,
    /// The paragraph holds the text's first and last formatted lines.
    first: bool,
    last: bool,
}

impl<'a> Paragraph<'a> {
    fn of(
        text: &Text,
        chunk: &'a TextChunk,
        [first, last]: [bool; 2],
        strut: &Strut,
        fonts: &FontsCache,
        resolver: &dyn FontProvider,
        fontdb: &mut Arc<Database>,
    ) -> Option<Self> {
        let paragraph = layout::document_paragraph(chunk);
        let advances = measure(text, chunk, strut, fonts, resolver, fontdb)?;
        let indent = text.strut.style.text_indent.clone();
        let applies = first || (indent.each_line && chunk.line_breaks > 0);
        Some(Self {
            chunk,
            opportunities: crate::text::BreakOpportunities::of(&paragraph),
            breaking: Some(paragraph),
            advances,
            indent: applies.then_some(indent),
            first,
            last,
        })
    }

    fn unbroken(chunk: &'a TextChunk) -> Self {
        Self {
            chunk,
            breaking: None,
            opportunities: crate::text::BreakOpportunities {
                automatic: Vec::new(),
                opportunities: Vec::new(),
                emergency: Vec::new(),
            },
            advances: Vec::new(),
            indent: None,
            first: false,
            last: false,
        }
    }

    /// The advance hanging at the logical start and end of the line showing `visible`, which a
    /// hyphen ends where `hyphenated`: preserved spaces ending it (CSS Text 3 §4.1.3), and the
    /// punctuation `hanging-punctuation` hangs (CSS Text 4 §9.2). An `allow-end` stop or comma
    /// hangs only as far as the line overflows `available`, and wholly for fit without it.
    fn hanging(
        &self,
        paragraph: &crate::document::Paragraph,
        visible: Range<usize>,
        hyphenated: bool,
        available: Option<f32>,
    ) -> [f32; 2] {
        use crate::text::hanging::{end_hang, hangs_first};
        let text = paragraph.text();
        let punctuation =
            |offset: usize| paragraph.style_at(offset).line_breaking.hanging_punctuation;
        let start = text[visible.clone()]
            .chars()
            .next()
            .filter(|&ch| {
                self.first && visible.start == 0 && hangs_first(punctuation(visible.start), ch)
            })
            .map_or(0.0, |ch| {
                self.width(visible.start..visible.start + ch.len_utf8())
            });
        let spaces = hanging_spaces(paragraph, visible.clone());
        if spaces > 0 {
            return [start, self.width(visible.end - spaces..visible.end)];
        }
        let end = text[visible.clone()]
            .char_indices()
            .next_back()
            .filter(|_| !hyphenated)
            .and_then(|(offset, ch)| {
                let offset = visible.start + offset;
                let last = self.last && visible.end == text.len();
                let conditional = end_hang(punctuation(offset), ch, last)?;
                let width = self.width(offset..visible.end);
                Some(match available.filter(|_| conditional) {
                    // Spacing and trimming can leave the punctuation a negative advance, which
                    // does not add to the overflow.
                    Some(available) => {
                        (self.width(visible.clone()) - start - available).clamp(0.0, width.max(0.0))
                    }
                    None => width,
                })
            })
            .unwrap_or(0.0);
        [start, end]
    }

    /// The width of the source text in `range`.
    fn width(&self, range: Range<usize>) -> f32 {
        self.advances_in(range).sum()
    }

    /// The advances of the characters in `range`.
    fn advances_in(&self, range: Range<usize>) -> impl Iterator<Item = f32> + '_ {
        let start = self
            .advances
            .partition_point(|(offset, _)| *offset < range.start);
        self.advances[start..]
            .iter()
            .take_while(move |(offset, _)| *offset < range.end)
            .map(|(_, advance)| *advance)
    }

    /// The widths of the text between consecutive break opportunities, whose maximum is the
    /// paragraph's min-content inline size.
    fn segments(&self) -> impl Iterator<Item = f32> + '_ {
        let paragraph = self.breaking.as_ref();
        let mut start = 0;
        self.opportunities
            .opportunities
            .iter()
            .filter_map(move |&(end, _)| {
                let paragraph = paragraph?;
                let range = crate::text::whitespace::visible(paragraph, start..end);
                start = end;
                Some(self.width(range))
            })
    }

    /// The paragraph broken into lines no wider than `width` where possible, each with whether
    /// it overflows the wrapping area. Where the paragraph starts the text's first line,
    /// `first_line` is the paragraph as that line styles it, which sets the line.
    #[expect(
        clippy::too_many_arguments,
        reason = "line breaking reads the area, both stylings and the font machinery"
    )]
    fn lines(
        &self,
        text_element: &Text,
        width: f32,
        wrapping: Wrapping,
        first_line: Option<&Self>,
        fonts: &FontsCache,
        resolver: &dyn FontProvider,
        fontdb: &mut Arc<Database>,
    ) -> Option<Vec<(TextChunk, bool)>> {
        let Some(text) = self.breaking.as_ref().map(crate::document::Paragraph::text) else {
            return Some(Vec::new());
        };
        let mut lines = Vec::new();
        let mut start = 0;
        let mut first = true;
        while first || start < text.len() {
            let styled = first_line.filter(|_| first).unwrap_or(self);
            let paragraph = styled.breaking.as_ref()?;
            let indent = styled
                .indent
                .as_ref()
                .filter(|_| first)
                .map_or(0.0, |indent| {
                    // The document's lengths are points; user units are CSS pixels.
                    let points = crate::style::POINTS_PER_PIXEL;
                    indent.resolve(width * points, true, styled.chunk.line_breaks > 0) / points
                });
            let available = width - indent;
            let end = styled.fit(paragraph, start, available, fonts, resolver, fontdb)?;
            let hyphen =
                crate::text::hyphenation::string(paragraph, &styled.opportunities.automatic, end)
                    .filter(|_| end < text.len());
            let visible = crate::text::whitespace::visible(paragraph, start..end);
            let hanging = styled.hanging(
                paragraph,
                visible.clone(),
                hyphen.is_some(),
                Some(available),
            );
            let mut used = styled.width(visible.clone()) - hanging[0] - hanging[1]
                + hyphen.map_or(0.0, |hyphen| {
                    styled.extra_width(hyphen, visible.end, fonts, resolver, fontdb)
                });
            let mut line = styled.line(visible.clone(), hyphen, hanging, first);
            // CSS Text 4 §8.5.1: closing punctuation ending a line that does not otherwise fit
            // may be set half-width.
            if used > available {
                let advance = |line: &TextChunk, fontdb: &mut Arc<Database>| {
                    layout::spaced_clusters(text_element, line, fonts, resolver, fontdb)
                        .map_or(0.0, |clusters| {
                            clusters.iter().map(|cluster| cluster.advance).sum::<f32>()
                        })
                };
                let untrimmed = advance(&line, fontdb);
                line.overflowing = true;
                used += advance(&line, fontdb) - untrimmed;
            }
            let overflows = used > available;
            if overflows {
                let fitting = Fitting {
                    available,
                    first,
                    overflow: wrapping.overflow,
                };
                styled.ellipsize(&mut line, visible, fitting, fonts, resolver, fontdb)?;
            }
            line.inline_offset = offset(line.anchor, line.rtl, width, indent);
            lines.push((line, overflows));
            first = false;
            if end <= start {
                break;
            }
            start = end;
        }
        Some(lines)
    }

    /// Where the line starting at `start` ends: at the last opportunity whose line fits in
    /// `available`, at a forced break, else at the last emergency opportunity that fits, else
    /// at the first opportunity, overflowing (CSS Text 3 §5.2, CSS Text 4 §6.5).
    fn fit(
        &self,
        paragraph: &crate::document::Paragraph,
        start: usize,
        available: f32,
        fonts: &FontsCache,
        resolver: &dyn FontProvider,
        fontdb: &mut Arc<Database>,
    ) -> Option<usize> {
        let text = paragraph.text();
        let fits = |end: usize, fonts: &FontsCache, fontdb: &mut Arc<Database>| {
            let visible = crate::text::whitespace::visible(paragraph, start..end);
            let hyphen =
                crate::text::hyphenation::string(paragraph, &self.opportunities.automatic, end)
                    .filter(|_| end < text.len());
            let [start_hang, end_hang] =
                self.hanging(paragraph, visible.clone(), hyphen.is_some(), None);
            let hyphen = hyphen.map_or(Some(0.0), |hyphen| {
                Some(self.extra_width(hyphen, visible.end, fonts, resolver, fontdb))
            });
            hyphen.is_some_and(|hyphen| {
                self.width(visible.clone()) - start_hang - end_hang + hyphen <= available
            })
        };
        let mut chosen = None;
        let mut first = None;
        for &(end, opportunity) in &self.opportunities.opportunities {
            if end <= start {
                continue;
            }
            first.get_or_insert(end);
            if fits(end, fonts, fontdb) {
                chosen = Some(end);
                if opportunity == BreakOpportunity::Mandatory {
                    break;
                }
            } else {
                break;
            }
        }
        if let Some(end) = chosen {
            return Some(end);
        }
        let limit = first.unwrap_or(text.len());
        let emergency = self
            .opportunities
            .emergency
            .iter()
            .copied()
            .filter(|&end| end > start && end < limit);
        let mut fitting = None;
        let mut earliest = None;
        for end in emergency {
            earliest.get_or_insert(end);
            if fits(end, fonts, fontdb) {
                fitting = Some(end);
            } else {
                break;
            }
        }
        Some(fitting.or(earliest).unwrap_or(limit))
    }

    /// The width a string set in the style of the character ending at `offset` adds.
    fn extra_width(
        &self,
        extra: &str,
        offset: usize,
        fonts: &FontsCache,
        resolver: &dyn FontProvider,
        fontdb: &mut Arc<Database>,
    ) -> f32 {
        let spans = &self.chunk.spans;
        let Some(span) = spans
            .get(spans.partition_point(|span| span.end < offset))
            .filter(|span| span.start < offset)
            .or_else(|| spans.first())
        else {
            return 0.0;
        };
        layout::styled_clusters(self.chunk, span, extra, fonts, resolver, fontdb)
            .map_or(0.0, |clusters| {
                clusters.iter().map(|cluster| cluster.advance).sum()
            })
    }

    /// The chunk showing `range` of the paragraph's text, without conditional hyphens, then
    /// `hyphen`; only the first line keeps the paragraph's position and forced breaks.
    fn line(
        &self,
        range: Range<usize>,
        hyphen: Option<&str>,
        hanging: [f32; 2],
        first: bool,
    ) -> TextChunk {
        let source = &self.chunk.text;
        let mut text = String::new();
        let mut spans: Vec<crate::svg::usvg::TextSpan> = Vec::new();
        let first_span = self
            .chunk
            .spans
            .partition_point(|span| span.end <= range.start);
        for span in self.chunk.spans[first_span..]
            .iter()
            .take_while(|span| span.start < range.end)
        {
            let start = span.start.max(range.start);
            let end = span.end.min(range.end);
            if start >= end {
                continue;
            }
            let from = text.len();
            text.push_str(&crate::text::hyphenation::text(&source[start..end], None));
            spans.push(crate::svg::usvg::TextSpan {
                start: from,
                end: text.len(),
                ..span.clone()
            });
        }
        if let (Some(hyphen), Some(last)) = (hyphen, spans.last_mut()) {
            text.push_str(hyphen);
            last.end = text.len();
        }
        spans.retain(|span| span.start < span.end);
        TextChunk {
            x: if first { self.chunk.x } else { None },
            y: if first { self.chunk.y } else { None },
            line_breaks: if first { self.chunk.line_breaks } else { 1 },
            hanging,
            soft_wrapped: !first,
            ..self.chunk.holding(text, spans)
        }
    }

    /// CSS Overflow 4 §4.1: a line overflowing its area at a line edge whose `text-overflow`
    /// is `ellipsis` hides characters from that edge until an ellipsis, in the text element's
    /// style, fits beside the rest.
    fn ellipsize(
        &self,
        line: &mut TextChunk,
        visible: Range<usize>,
        fitting: Fitting,
        fonts: &FontsCache,
        resolver: &dyn FontProvider,
        fontdb: &mut Arc<Database>,
    ) -> Option<()> {
        let Fitting {
            available,
            first,
            overflow,
        } = fitting;
        // The line overflows at its end edge, the start of the anchor's alignment.
        let end_side = match line.anchor {
            TextAnchor::Start => 1,
            TextAnchor::End => 0,
            TextAnchor::Middle => usize::from(!line.rtl),
        };
        if overflow[end_side] != TextOverflow::Ellipsis {
            return Some(());
        }
        // The physical end edge is the logical end unless the base direction opposes it.
        let logical_end = (end_side == 1) != line.rtl;
        let ellipsis = self.extra_width(ELLIPSIS, visible.end, fonts, resolver, fontdb);
        let room = available - ellipsis;
        let text = &self.chunk.text;
        let mut kept = visible.clone();
        let boundaries: Vec<usize> = text[visible.clone()]
            .char_indices()
            .map(|(offset, _)| visible.start + offset)
            .chain([visible.end])
            .collect();
        // The width left as each character is hidden, in double precision so that neither the
        // sum nor the subtractions drift.
        let width_of = |range| self.advances_in(range).map(f64::from).sum::<f64>();
        let room = f64::from(room);
        let mut width = width_of(kept.clone());
        if logical_end {
            while kept.end > kept.start && width > room {
                let end = boundaries[boundaries.partition_point(|&offset| offset < kept.end) - 1];
                width -= width_of(end..kept.end);
                kept.end = end;
            }
        } else {
            while kept.start < kept.end && width > room {
                let start = boundaries[boundaries.partition_point(|&offset| offset <= kept.start)];
                width -= width_of(kept.start..start);
                kept.start = start;
            }
        }
        let mut shown = self.line(kept, None, [0.0; 2], first);
        let style_span = shown
            .spans
            .first()
            .cloned()
            .or_else(|| self.chunk.spans.first().cloned())?;
        let marker = crate::svg::usvg::TextSpan {
            start: 0,
            end: ELLIPSIS.len(),
            decoration: crate::svg::usvg::TextDecoration {
                underline: None,
                overline: None,
                line_through: None,
            },
            ..style_span
        };
        if logical_end {
            let start = shown.text.len();
            shown.text.push_str(ELLIPSIS);
            shown.spans.push(crate::svg::usvg::TextSpan {
                start,
                end: shown.text.len(),
                ..marker
            });
        } else {
            for span in &mut shown.spans {
                span.start += ELLIPSIS.len();
                span.end += ELLIPSIS.len();
            }
            shown.text.insert_str(0, ELLIPSIS);
            shown.spans.insert(0, marker);
        }
        shown.x = line.x;
        shown.y = line.y;
        *line = shown;
        Some(())
    }
}

/// The room an overflowing line has, whether it is its paragraph's first, and the
/// `text-overflow` of each line edge.
#[derive(Clone, Copy)]
struct Fitting {
    available: f32,
    first: bool,
    overflow: [TextOverflow; 2],
}

/// The bytes of preserved spaces ending `range` that hang (CSS Text 3 §4.1.3).
fn hanging_spaces(paragraph: &crate::document::Paragraph, range: Range<usize>) -> usize {
    let text = paragraph.text();
    text[range.clone()]
        .char_indices()
        .rev()
        .take_while(|&(offset, ch)| {
            matches!(ch, ' ' | '\t')
                && paragraph
                    .style_at(range.start + offset)
                    .white_space
                    .hangs_spaces()
        })
        .map(|(_, ch)| ch.len_utf8())
        .sum()
}

/// How far along the inline axis a line anchored by `anchor` moves from the anchor point, in
/// an area `width` wide whose start side `text-indent` narrows by `indent`: the area lies
/// after, around or before the point, and the line at its start, middle or end (SVG 2 §11.4.1).
fn offset(anchor: TextAnchor, rtl: bool, width: f32, indent: f32) -> f32 {
    let area = match anchor {
        TextAnchor::Start => [0.0, width],
        TextAnchor::Middle => [-width / 2.0, width / 2.0],
        TextAnchor::End => [-width, 0.0],
    };
    let [start, end] = if rtl {
        [area[0], area[1] - indent]
    } else {
        [area[0] + indent, area[1]]
    };
    match anchor {
        TextAnchor::Start => start,
        TextAnchor::Middle => (start + end) / 2.0,
        TextAnchor::End => end,
    }
}

/// Each cluster's source start and advance in `chunk` as layout spaces them, in source order.
fn measure(
    text: &Text,
    chunk: &TextChunk,
    strut: &Strut,
    fonts: &FontsCache,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<Vec<(usize, f32)>> {
    let mut clusters = layout::spaced_clusters(text, chunk, fonts, resolver, fontdb)?;
    if let Some(font) = fonts.get(&strut.font) {
        layout::apply_tab_stops(chunk, strut, font, fonts, 0.0, &mut clusters);
    }
    let mut advances: Vec<_> = clusters
        .iter()
        .map(|cluster| (cluster.byte_idx.value(), cluster.advance))
        .collect();
    advances.sort_by_key(|(offset, _)| *offset);
    Some(advances)
}
