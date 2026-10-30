//! Auto-wrapped SVG text (SVG 2 §11.4, §11.7): a text element with a wrapping area breaks each
//! of its paragraphs into lines where the document's line breaking allows, and each line
//! becomes a chunk that starts a line box and is anchored by `text-anchor` in the area.
use std::ops::Range;
use std::sync::Arc;

use fontdb::Database;
use unicode_linebreak::BreakOpportunity;

use super::layout::{self, FontsCache, ResolvedFont};
use crate::style::TextOverflow;
use crate::svg::usvg::{
    FontProvider, NonZeroRect, Text, TextAnchor, TextChunk, TextFlow, WrapWidth, Wrapping,
    WritingMode,
};

/// CSS Overflow 4 §4.1: the ellipsis `text-overflow: ellipsis` renders.
const ELLIPSIS: &str = "\u{2026}";

/// `text` with each paragraph of its linear chunks broken into lines no wider than its wrapping
/// area, and the area `text-overflow: clip` keeps. Chunks on a text path keep their layout.
pub(crate) fn wrap(
    text: &Text,
    wrapping: Wrapping,
    fonts: &FontsCache,
    strut: Option<&ResolvedFont>,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<Text> {
    let mut paragraphs = Vec::new();
    for (index, chunk) in text.chunks.iter().enumerate() {
        paragraphs.push(match chunk.text_flow {
            TextFlow::Linear => {
                Paragraph::of(text, chunk, index == 0, fonts, strut, resolver, fontdb)?
            }
            TextFlow::Path(_) => Paragraph::unbroken(chunk),
        });
    }
    let width = match wrapping.width {
        WrapWidth::Length(width) => width,
        WrapWidth::MinContent => paragraphs
            .iter()
            .flat_map(|paragraph| paragraph.segments())
            .fold(0.0, f32::max),
    };
    let mut chunks = Vec::new();
    let mut overflows = false;
    for (paragraph, chunk) in paragraphs.iter().zip(&text.chunks) {
        if !matches!(chunk.text_flow, TextFlow::Linear) {
            chunks.push(chunk.clone());
            continue;
        }
        for (line, overflowing) in paragraph.lines(width, wrapping, fonts, resolver, fontdb)? {
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
}

impl<'a> Paragraph<'a> {
    fn of(
        text: &Text,
        chunk: &'a TextChunk,
        first: bool,
        fonts: &FontsCache,
        strut: Option<&ResolvedFont>,
        resolver: &dyn FontProvider,
        fontdb: &mut Arc<Database>,
    ) -> Option<Self> {
        let paragraph = crate::document::Paragraph::styled(
            chunk.text.clone(),
            &text.strut.style,
            chunk
                .spans
                .iter()
                .map(|span| (span.start..span.end, (*span.style).clone())),
        );
        let advances = measure(text, chunk, fonts, strut, resolver, fontdb)?;
        let indent = text.strut.style.text_indent.clone();
        let applies = first || (indent.each_line && chunk.line_breaks > 0);
        Some(Self {
            chunk,
            opportunities: crate::text::BreakOpportunities::of(&paragraph),
            breaking: Some(paragraph),
            advances,
            indent: applies.then_some(indent),
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
        }
    }

    /// The width of the source text in `range`.
    fn width(&self, range: Range<usize>) -> f32 {
        let start = self
            .advances
            .partition_point(|(offset, _)| *offset < range.start);
        self.advances[start..]
            .iter()
            .take_while(|(offset, _)| *offset < range.end)
            .map(|(_, advance)| advance)
            .sum()
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
    /// it overflows the wrapping area.
    fn lines(
        &self,
        width: f32,
        wrapping: Wrapping,
        fonts: &FontsCache,
        resolver: &dyn FontProvider,
        fontdb: &mut Arc<Database>,
    ) -> Option<Vec<(TextChunk, bool)>> {
        let Some(paragraph) = &self.breaking else {
            return Some(Vec::new());
        };
        let text = paragraph.text();
        let mut lines = Vec::new();
        let mut start = 0;
        let mut first = true;
        while first || start < text.len() {
            let indent = self
                .indent
                .as_ref()
                .filter(|_| first)
                .map_or(0.0, |indent| {
                    // The document's lengths are points; user units are CSS pixels.
                    let points = crate::style::POINTS_PER_PIXEL;
                    indent.resolve(width * points, true, self.chunk.line_breaks > 0) / points
                });
            let available = width - indent;
            let end = self.fit(paragraph, start, available, fonts, resolver, fontdb)?;
            let hyphen =
                crate::text::hyphenation::string(paragraph, &self.opportunities.automatic, end)
                    .filter(|_| end < text.len());
            let visible = crate::text::whitespace::visible(paragraph, start..end);
            let hanging = hanging(paragraph, visible.clone());
            let used = self.width(visible.start..visible.end - hanging)
                + hyphen.map_or(0.0, |hyphen| {
                    self.extra_width(hyphen, visible.end, fonts, resolver, fontdb)
                });
            let mut line = self.line(visible.clone(), hyphen, hanging, first);
            let overflows = used > available;
            if overflows {
                let fitting = Fitting {
                    available,
                    first,
                    overflow: wrapping.overflow,
                };
                self.ellipsize(&mut line, visible, fitting, fonts, resolver, fontdb)?;
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
            let hanging = hanging(paragraph, visible.clone());
            let hyphen =
                crate::text::hyphenation::string(paragraph, &self.opportunities.automatic, end)
                    .filter(|_| end < text.len())
                    .map_or(Some(0.0), |hyphen| {
                        Some(self.extra_width(hyphen, visible.end, fonts, resolver, fontdb))
                    });
            hyphen.is_some_and(|hyphen| {
                self.width(visible.start..visible.end - hanging) + hyphen <= available
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
        let Some(span) = self
            .chunk
            .spans
            .iter()
            .find(|span| span.start < offset && offset <= span.end)
            .or_else(|| self.chunk.spans.first())
        else {
            return 0.0;
        };
        let chunk = TextChunk {
            text: extra.to_owned(),
            spans: vec![crate::svg::usvg::TextSpan {
                start: 0,
                end: extra.len(),
                ..span.clone()
            }],
            ..self.chunk.clone()
        };
        layout::process_chunk(&chunk, WritingMode::LeftToRight, fonts, resolver, fontdb)
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
        hanging: usize,
        first: bool,
    ) -> TextChunk {
        let source = &self.chunk.text;
        let mut text = String::new();
        let mut spans: Vec<crate::svg::usvg::TextSpan> = Vec::new();
        for span in &self.chunk.spans {
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
            spans,
            text,
            hanging,
            ..self.chunk.clone()
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
        if logical_end {
            while kept.end > kept.start && self.width(kept.clone()) > room {
                kept.end = boundaries[boundaries.partition_point(|&offset| offset < kept.end) - 1];
            }
        } else {
            while kept.start < kept.end && self.width(kept.clone()) > room {
                kept.start = boundaries[boundaries.partition_point(|&offset| offset <= kept.start)];
            }
        }
        let mut shown = self.line(kept, None, 0, first);
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
fn hanging(paragraph: &crate::document::Paragraph, range: Range<usize>) -> usize {
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
    fonts: &FontsCache,
    strut: Option<&ResolvedFont>,
    resolver: &dyn FontProvider,
    fontdb: &mut Arc<Database>,
) -> Option<Vec<(usize, f32)>> {
    let mut clusters = layout::spaced_clusters(text, chunk, fonts, resolver, fontdb)?;
    if let Some(strut) = strut {
        layout::apply_tab_stops(chunk, &text.strut, strut, fonts, 0.0, &mut clusters);
    }
    let mut advances: Vec<_> = clusters
        .iter()
        .map(|cluster| (cluster.byte_idx.value(), cluster.advance))
        .collect();
    advances.sort_by_key(|(offset, _)| *offset);
    Some(advances)
}
