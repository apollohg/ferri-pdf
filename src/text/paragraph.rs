use crate::document::Paragraph;
use crate::text::{
    AtomicMetrics, GlyphRun, Line, Piece, TextContext, TextError, breaking, decoration, ellipsis,
    hanging, hyphenation, line_boxes, paragraph_bidi, shape_line, spacing, whitespace,
};
use unicode_linebreak::BreakOpportunity;

mod wrapping;

/// Source progress after a successfully emitted line of its originating paragraph.
#[derive(Clone, Copy)]
pub(crate) struct TextPosition<'p> {
    paragraph: &'p Paragraph,
    offset: usize,
    empty_consumed: bool,
    /// Lines emitted before this position.
    line: usize,
}

/// A region break after the paragraph's `lines`th line (CSS Overflow 4 §5.2): later content
/// is discarded, and the last line ends with the block overflow `ellipsis`, if any.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParagraphClamp {
    pub lines: usize,
    pub ellipsis: Option<crate::style::BlockEllipsis>,
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
    /// Automatic hyphenation opportunities, which are also among `opportunities`.
    automatic: Vec<usize>,
    opportunities: Vec<(usize, BreakOpportunity)>,
    ordinary: Vec<usize>,
    emergency: Vec<usize>,
    monotone: bool,
    plans: wrapping::Plans,
    clamp: Option<ParagraphClamp>,
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
        let automatic = hyphenation::automatic(paragraph);
        let opportunities = whitespace::opportunities(paragraph, &automatic);
        let ordinary = opportunities
            .iter()
            .map(|&(end, _)| end)
            .filter(|&end| !hyphenation::is_soft_break(paragraph, &automatic, end))
            .collect();
        Self {
            context,
            paragraph,
            bidi,
            has_rtl,
            has_leaders: paragraph.spans().iter().any(|span| span.leader),
            automatic,
            opportunities,
            ordinary,
            emergency: whitespace::emergency_opportunities(paragraph, false),
            monotone: paragraph.spans().iter().all(|span| {
                span.style.letter_spacing >= 0.0
                    && span.style.word_spacing.resolve(span.style.size) >= 0.0
            }),
            plans: wrapping::Plans::default(),
            clamp: None,
        }
    }

    /// Lines of this paragraph end at the region break of `clamp`.
    #[must_use]
    pub(crate) fn clamped(mut self, clamp: Option<ParagraphClamp>) -> Self {
        self.clamp = clamp;
        self
    }

    pub(crate) fn start(&self) -> TextPosition<'p> {
        TextPosition {
            paragraph: self.paragraph,
            offset: 0,
            empty_consumed: false,
            line: 0,
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

    /// A line from `from` to `to`, before vertical alignment and justification, whose start
    /// edge has `indent`, ending with the `trailing` runs at its inline end.
    fn emit(
        &self,
        from: usize,
        to: usize,
        indent: f32,
        trailing: Vec<GlyphRun>,
    ) -> Result<Line, TextError> {
        let available = self.width - indent;
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
            hyphenation::decorate(
                prepared.context,
                paragraph,
                &prepared.automatic,
                bidi,
                to,
                &mut pieces,
            )?
        } else {
            None
        };
        if paragraph.style().rtl {
            pieces.splice(0..0, trailing.into_iter().rev().map(Piece::Text));
        } else {
            pieces.extend(trailing.into_iter().map(Piece::Text));
        }
        let arranged =
            line_boxes::arrange(paragraph, pieces, from..to, &self.atomic_widths, available);
        let measured_width = arranged.width;
        let leader_width = if prepared.has_leaders {
            crate::text::leaders::wrapping_width(paragraph, &arranged, available, from)
        } else {
            None
        };
        let wrap_width = spacing::wrapping_width(
            paragraph,
            &arranged.runs,
            whitespace::wrapping_width(paragraph, visible.end, &arranged.runs, measured_width),
        );
        let wrap_width = leader_width.map_or(wrap_width, |width| wrap_width.max(width));
        let hangable = hanging::hangable(
            paragraph,
            from..to,
            visible.clone(),
            &arranged.runs,
            &arranged.order,
        );
        let wrap_width = wrap_width - hangable.fit();
        if !measured_width.is_finite() || !wrap_width.is_finite() || !available.is_finite() {
            return Err(TextError::NonFiniteGeometry);
        }
        Ok(Line {
            indent,
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
            hangable,
            hanging: [0.0; 2],
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
        let trim = [
            paragraph.text_box_trim[0] && line.range.start == 0,
            paragraph.text_box_trim[1] && line.range.end == paragraph.text().len(),
        ];
        if trim.contains(&true) {
            // The trimmed side moves to the root inline box's `text-box-edge` metric.
            let root = line_boxes::BoxMetrics::of(prepared.context, paragraph.style())?;
            let edges = paragraph.style().text_box.edges;
            if trim[0] {
                let over = -root.edge(edges[0], false);
                line.height -= line.ascent - over;
                line.ascent = over;
            }
            if trim[1] {
                line.height = line.ascent + root.edge(edges[1], true);
            }
        }
        line.hanging = line
            .hangable
            .hanging(line.width, self.available(), paragraph.style().rtl);
        let hanging = line.hanging[0] + line.hanging[1];
        line.width -= hanging;
        spacing::align(&mut line, paragraph, self.available(), last)?;
        // Hanging punctuation is not measured, so it alone does not make the line overflow.
        let overflows = line.width > self.available();
        line.width += hanging;
        if overflows {
            ellipsis::ellipsize(prepared.context, paragraph, &mut line, self.available())?;
        }
        line.place();
        crate::text::emphasis::place(prepared.context, paragraph, &mut line)?;
        line.ink_bounds = line
            .runs
            .iter()
            .flat_map(|run| std::iter::once(run).chain(&run.marks))
            .filter_map(|run| {
                run.ink_bounds
                    .map(|ink| ink.map(|y| y + run.baseline_shift))
            })
            .reduce(|[a, b], [c, d]| [a.min(c), b.max(d)]);
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
                self.finalize(self.emit(0, 0, self.indent, Vec::new())?, true)
                    .map(Some)
            } else {
                Ok(None)
            };
        }
        if start == text.len() {
            return Ok(None);
        }
        if let Some((end, last)) = self.planned_end(start)? {
            return self
                .finalize(self.emit(start, end, self.indent, Vec::new())?, last)
                .map(Some);
        }
        self.greedy(start, self.indent)?
            .map(|(line, last)| self.finalize(line, last))
            .transpose()
    }

    /// CSS Overflow 4 §4.2: the line before a region break holds the block overflow ellipsis
    /// after the last soft wrap opportunity that leaves room for it, or only the ellipsis.
    /// Emergency opportunities of `overflow-wrap` are not candidates.
    fn ellipsized_line(
        &self,
        ellipsis: &crate::style::BlockEllipsis,
    ) -> Result<Option<Line>, TextError> {
        let prepared = self.prepared;
        let paragraph = prepared.paragraph;
        let start = self.position.offset;
        let natural = if let Some(planned) = self.planned_end(start)? {
            Some(planned)
        } else {
            self.greedy(start, self.indent)?
                .map(|(line, last)| (line.range.end, last))
        };
        let Some((natural_end, last)) = natural else {
            return Ok(None);
        };
        let text = match ellipsis {
            crate::style::BlockEllipsis::None => return self.next_line(),
            crate::style::BlockEllipsis::Auto => None,
            crate::style::BlockEllipsis::String(text) => Some(text.as_ref()),
        };
        let mut runs = ellipsis::shape(prepared.context, paragraph.style(), text)?;
        // The ellipsis inline has `line-height: 0`, so it cannot make the line taller.
        for run in &mut runs {
            run.leading = Some(0.0);
        }
        let with_ellipsis = |end: usize| {
            let span = paragraph
                .spans()
                .partition_point(|span| span.range.end < end)
                .min(paragraph.spans().len().saturating_sub(1));
            let mut runs = runs.clone();
            for run in &mut runs {
                run.source_range = end..end;
                run.source_span = span;
            }
            self.emit(start, end, self.indent, runs)
        };
        let first = prepared
            .opportunities
            .partition_point(|&(end, _)| end <= start);
        let last_candidate = prepared
            .opportunities
            .partition_point(|&(end, _)| end <= natural_end);
        let suffix: String = runs.iter().map(|run| run.text.as_str()).collect();
        let mut chosen = None;
        for &(end, _) in prepared.opportunities[first..last_candidate].iter().rev() {
            let line = with_ellipsis(end)?;
            if line.wrap_width <= self.available() {
                chosen = Some((line, last && end == natural_end));
                break;
            }
        }
        let (line, last) = match chosen {
            Some(chosen) => chosen,
            None => (with_ellipsis(start)?, false),
        };
        let mut line = self.finalize(line, last)?;
        line.text.push_str(&suffix);
        Ok(Some(line))
    }

    /// The first-fit line from `start` whose start edge has `indent`, and whether it is the
    /// last line before a forced break or the paragraph end.
    fn greedy(&self, start: usize, indent: f32) -> Result<Option<(Line, bool)>, TextError> {
        let prepared = self.prepared;
        let paragraph = prepared.paragraph;
        let text = paragraph.text();
        let available = self.width - indent;
        let mut best = None;
        let first = prepared
            .opportunities
            .partition_point(|&(end, _)| end <= start);
        for &(end, opportunity) in &prepared.opportunities[first..] {
            let mut candidate = self.emit(start, end, indent, Vec::new())?;
            if opportunity == BreakOpportunity::Allowed
                && prepared.has_leaders
                && crate::text::leaders::alone(&candidate)
            {
                continue;
            }
            if candidate.wrap_width > available
                && hyphenation::is_soft_break(paragraph, &prepared.automatic, end)
                && let Some(&next) = prepared
                    .ordinary
                    .get(prepared.ordinary.partition_point(|&next| next <= end))
                && self.emit(start, next, indent, Vec::new())?.wrap_width <= available
            {
                continue;
            }
            if candidate.wrap_width > available
                && let Some(previous) = best
            {
                return Ok(Some((
                    self.emit(start, previous, indent, Vec::new())?,
                    false,
                )));
            }
            if candidate.wrap_width > available {
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
                        available,
                        prepared.monotone,
                        |to| self.emit(start, to, indent, Vec::new()),
                    )?;
                    if cut < end && !whitespace::visible(paragraph, cut..end).is_empty() {
                        return Ok(Some((line, false)));
                    }
                    if cut == end {
                        candidate = line;
                    }
                }
            }
            if opportunity == BreakOpportunity::Mandatory {
                return Ok(Some((candidate, true)));
            }
            best = Some(end);
        }
        best.map(|end| Ok((self.emit(start, end, indent, Vec::new())?, true)))
            .transpose()
    }
}

impl Iterator for ParagraphLines<'_, '_, '_, '_> {
    type Item = Result<Line, TextError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let clamp = self.prepared.clamp.as_ref();
        if clamp.is_some_and(|clamp| self.position.line >= clamp.lines) {
            self.finished = true;
            return None;
        }
        self.indent = indent_at(
            self.prepared.paragraph,
            &self.prepared.opportunities,
            self.position.offset,
            self.containing_width,
        );
        let line = match clamp {
            Some(ParagraphClamp {
                lines,
                ellipsis: Some(ellipsis),
            }) if *lines == self.position.line + 1 => self.ellipsized_line(ellipsis),
            _ => self.next_line(),
        };
        match line {
            Ok(Some(line)) => {
                self.position.offset = line.range.end;
                self.position.empty_consumed = true;
                self.position.line += 1;
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
