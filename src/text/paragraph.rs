use crate::document::Paragraph;
use crate::text::{
    AtomicMetrics, Line, TextContext, TextError, breaking, decoration, ellipsis, hyphenation,
    line_boxes, paragraph_bidi, shape_line, spacing, whitespace,
};
use unicode_linebreak::BreakOpportunity;

/// Source progress after a successfully emitted line of its originating paragraph.
#[derive(Clone, Copy)]
pub(crate) struct TextPosition<'p> {
    paragraph: &'p Paragraph,
    offset: usize,
    empty_consumed: bool,
}

impl TextPosition<'_> {
    /// Byte offset of the next source content in the originating paragraph.
    pub(crate) fn offset(self) -> usize {
        self.offset
    }
}

/// Paragraph-wide bidi and break context, shared across fragmentainer widths.
pub(crate) struct PreparedParagraph<'p, 'context, 'font> {
    context: &'context TextContext<'font>,
    paragraph: &'p Paragraph,
    bidi: Result<crate::text::bidi::ResolvedParagraph<'p>, crate::text::bidi::ExpansionLimit>,
    has_rtl: bool,
    has_leaders: bool,
    opportunities: Vec<(usize, BreakOpportunity)>,
    ordinary: Vec<usize>,
    emergency: Vec<usize>,
    monotone: bool,
}

/// Lines at one fragmentainer width, retaining absolute paragraph source coordinates.
pub(crate) struct ParagraphLines<'s, 'p, 'context, 'font> {
    prepared: &'s PreparedParagraph<'p, 'context, 'font>,
    position: TextPosition<'p>,
    width: f32,
    containing_width: f32,
    indent: f32,
    atomics: &'s [AtomicMetrics],
    atomic_widths: Vec<f32>,
    finished: bool,
}

impl<'p, 'context, 'font> PreparedParagraph<'p, 'context, 'font> {
    pub(crate) fn new(context: &'context TextContext<'font>, paragraph: &'p Paragraph) -> Self {
        let bidi = paragraph_bidi(paragraph, &context.bidi_work);
        let has_rtl = bidi
            .as_ref()
            .is_ok_and(super::bidi::ResolvedParagraph::has_rtl);
        let opportunities = whitespace::opportunities(paragraph);
        let ordinary = opportunities
            .iter()
            .map(|&(end, _)| end)
            .filter(|&end| !hyphenation::is_soft_break(paragraph, end))
            .collect();
        Self {
            context,
            paragraph,
            bidi,
            has_rtl,
            has_leaders: paragraph.spans().iter().any(|span| span.leader),
            opportunities,
            ordinary,
            emergency: whitespace::emergency_opportunities(paragraph, false),
            monotone: paragraph.spans().iter().all(|span| {
                span.style.letter_spacing >= 0.0
                    && span.style.word_spacing.resolve(span.style.size) >= 0.0
            }),
        }
    }

    pub(crate) fn start(&self) -> TextPosition<'p> {
        TextPosition {
            paragraph: self.paragraph,
            offset: 0,
            empty_consumed: false,
        }
    }

    fn bidi(&self) -> Result<&crate::text::bidi::ResolvedParagraph<'p>, TextError> {
        if self.context.bidi_work.get().is_none() {
            return Err(crate::text::bidi::ExpansionLimit.into());
        }
        self.bidi.as_ref().map_err(|&error| error.into())
    }

    pub(crate) fn lines<'s>(
        &'s self,
        position: TextPosition<'p>,
        width: f32,
        atomics: &'s [AtomicMetrics],
    ) -> Result<ParagraphLines<'s, 'p, 'context, 'font>, TextError> {
        self.lines_in_block(position, width, width, atomics)
    }

    /// Line fitting width can be narrowed by floats; percentages use the full block.
    pub(crate) fn lines_in_block<'s>(
        &'s self,
        position: TextPosition<'p>,
        width: f32,
        containing_width: f32,
        atomics: &'s [AtomicMetrics],
    ) -> Result<ParagraphLines<'s, 'p, 'context, 'font>, TextError> {
        self.bidi()?;
        if !std::ptr::eq(position.paragraph, self.paragraph) {
            return Err(TextError::ParagraphMismatch);
        }
        if atomics.len() != self.paragraph.atomics().len() {
            return Err(TextError::AtomicMetrics {
                expected: self.paragraph.atomics().len(),
                actual: atomics.len(),
            });
        }
        Ok(ParagraphLines {
            prepared: self,
            position,
            width,
            containing_width,
            indent: 0.0,
            atomics,
            atomic_widths: atomics.iter().map(|metrics| metrics.width).collect(),
            finished: false,
        })
    }
}

impl<'p> ParagraphLines<'_, 'p, '_, '_> {
    /// Resume at the last successfully emitted line, including empty inline-edge progress.
    pub(crate) fn checkpoint(&self) -> TextPosition<'p> {
        self.position
    }

    fn available(&self) -> f32 {
        self.width - self.indent
    }

    fn emit(&self, from: usize, to: usize) -> Result<Line, TextError> {
        let prepared = self.prepared;
        let bidi = prepared.bidi()?;
        let paragraph = prepared.paragraph;
        let text = paragraph.text();
        let visible = whitespace::visible(paragraph, from..to);
        let mut pieces = shape_line(
            prepared.context,
            text,
            bidi,
            visible.clone(),
            breaking::context_range(&prepared.opportunities, from..to),
            paragraph,
            prepared.has_rtl,
        )?;
        let hyphen = if visible.end == to {
            hyphenation::decorate(prepared.context, paragraph, bidi, to, &mut pieces)?
        } else {
            None
        };
        let arranged = line_boxes::arrange(
            paragraph,
            pieces,
            from..to,
            &self.atomic_widths,
            self.available(),
        );
        let measured_width = arranged.width;
        let leader_width = if prepared.has_leaders {
            crate::text::leaders::wrapping_width(paragraph, &arranged, self.available(), from)
        } else {
            None
        };
        let wrap_width = spacing::wrapping_width(
            &arranged.runs,
            whitespace::wrapping_width(paragraph, visible.end, &arranged.runs, measured_width),
            paragraph.style(),
        );
        let wrap_width = leader_width.map_or(wrap_width, |width| wrap_width.max(width));
        if !measured_width.is_finite() || !wrap_width.is_finite() || !self.available().is_finite() {
            return Err(TextError::NonFiniteGeometry);
        }
        Ok(Line {
            indent: self.indent,
            text: hyphenation::text(&text[visible], hyphen),
            range: from..to,
            width: measured_width,
            wrap_width,
            runs: arranged.runs,
            boxes: arranged.boxes,
            links: Vec::new(),
            atomics: arranged.atomics,
            ascent: 0.0,
            height: 0.0,
            ink_bounds: None,
            align: paragraph.style().align,
            decorations: Vec::new(),
            order: arranged.order,
            present: arranged.present,
            ellipsis: None,
        })
    }

    fn finalize(&self, mut line: Line, last: bool) -> Result<Line, TextError> {
        let prepared = self.prepared;
        let paragraph = prepared.paragraph;
        if prepared.has_leaders {
            crate::text::leaders::expand(prepared.context, paragraph, &mut line, self.available())?;
        }
        let newline = paragraph.text()[line.range.clone()]
            .char_indices()
            .next_back()
            .filter(|(_, ch)| matches!(ch, '\n' | '\u{2028}' | '\u{2029}'))
            .map(|(offset, _)| line.range.start + offset);
        let [ascent, descent] = line_boxes::align(
            prepared.context,
            paragraph,
            &mut line,
            self.atomics,
            newline,
        )?;
        line.ascent = ascent;
        line.height = ascent + descent;
        spacing::align(&mut line, paragraph.style(), self.available(), last)?;
        ellipsis::ellipsize(prepared.context, paragraph, &mut line, self.available())?;
        line.ink_bounds = line
            .runs
            .iter()
            .filter_map(|run| {
                run.ink_bounds
                    .map(|ink| ink.map(|y| y + run.baseline_shift))
            })
            .reduce(|[a, b], [c, d]| [a.min(c), b.max(d)]);
        line.place();
        line.links = line_boxes::links(paragraph, &line);
        line.decorations = decoration::paint(prepared.context, paragraph, &line)?;
        Ok(line)
    }

    fn next_line(&self) -> Result<Option<Line>, TextError> {
        let prepared = self.prepared;
        let paragraph = prepared.paragraph;
        let text = paragraph.text();
        let start = self.position.offset;
        if text.is_empty() {
            return if !self.position.empty_consumed && !paragraph.boxes().is_empty() {
                self.finalize(self.emit(0, 0)?, true).map(Some)
            } else {
                Ok(None)
            };
        }
        if start == text.len() {
            return Ok(None);
        }
        let mut best = None;
        let first = prepared
            .opportunities
            .partition_point(|&(end, _)| end <= start);
        for &(end, opportunity) in &prepared.opportunities[first..] {
            let mut candidate = self.emit(start, end)?;
            if opportunity == BreakOpportunity::Allowed
                && prepared.has_leaders
                && crate::text::leaders::alone(&candidate)
            {
                continue;
            }
            if candidate.wrap_width > self.available()
                && hyphenation::is_soft_break(paragraph, end)
                && let Some(&next) = prepared
                    .ordinary
                    .get(prepared.ordinary.partition_point(|&next| next <= end))
                && self.emit(start, next)?.wrap_width <= self.available()
            {
                continue;
            }
            if candidate.wrap_width > self.available()
                && let Some(previous) = best
            {
                return self.finalize(self.emit(start, previous)?, false).map(Some);
            }
            if candidate.wrap_width > self.available() {
                let content_end = if prepared.has_leaders {
                    crate::text::leaders::first_content_end(paragraph, start).unwrap_or(text.len())
                } else {
                    start
                };
                let first = prepared
                    .emergency
                    .partition_point(|&offset| offset <= start || offset < content_end);
                let last = prepared.emergency.partition_point(|&offset| offset < end);
                if first != last {
                    let (cut, line) = breaking::fit(
                        &prepared.emergency[first..last],
                        end,
                        self.available(),
                        prepared.monotone,
                        |to| self.emit(start, to),
                    )?;
                    if cut < end && !whitespace::visible(paragraph, cut..end).is_empty() {
                        return self.finalize(line, false).map(Some);
                    }
                    if cut == end {
                        candidate = line;
                    }
                }
            }
            if opportunity == BreakOpportunity::Mandatory {
                return self.finalize(candidate, true).map(Some);
            }
            best = Some(end);
        }
        best.map(|end| self.finalize(self.emit(start, end)?, true))
            .transpose()
    }
}

impl Iterator for ParagraphLines<'_, '_, '_, '_> {
    type Item = Result<Line, TextError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        self.indent = indent_at(
            self.prepared.paragraph,
            &self.prepared.opportunities,
            self.position.offset,
            self.containing_width,
        );
        match self.next_line() {
            Ok(Some(line)) => {
                self.position.offset = line.range.end;
                self.position.empty_consumed = true;
                Some(Ok(line))
            }
            Ok(None) => {
                self.finished = true;
                None
            }
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }
}

impl std::iter::FusedIterator for ParagraphLines<'_, '_, '_, '_> {}

pub(super) fn indent_at(
    paragraph: &Paragraph,
    opportunities: &[(usize, BreakOpportunity)],
    offset: usize,
    basis: f32,
) -> f32 {
    let indent = &paragraph.style().text_indent;
    if indent.amount == crate::style::LengthPercentage::ZERO {
        return 0.0;
    }
    let first = offset == 0 && (paragraph.first_formatted_line() || indent.each_line);
    let forced = indent.each_line
        && offset != 0
        && opportunities
            .binary_search_by_key(&offset, |(end, _)| *end)
            .is_ok_and(|index| opportunities[index].1 == BreakOpportunity::Mandatory);
    indent.resolve(basis, first, forced)
}
