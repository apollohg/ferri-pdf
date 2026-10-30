use crate::document::Paragraph;
use crate::text::{
    AtomicMetrics, GlyphRun, Line, Piece, TextContext, TextError, autospace::Autospace, breaking,
    ellipsis, hanging, hyphenation, line_boxes, paragraph_bidi, ruby, shape_line, spacing,
    spacing_trim, whitespace,
};
use std::ops::Range;
use unicode_linebreak::{BreakClass, BreakOpportunity, break_property};

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

/// Where the document's line breaking may end a line of a paragraph (CSS Text 3 §5, CSS Text 4
/// §6): its soft wrap and forced opportunities in offset order, the automatic hyphenation ones
/// among them, and the emergency ones `overflow-wrap` adds.
pub(crate) struct BreakOpportunities {
    pub(crate) automatic: Vec<usize>,
    pub(crate) opportunities: Vec<(usize, BreakOpportunity)>,
    pub(crate) emergency: Vec<usize>,
}
impl BreakOpportunities {
    pub(crate) fn of(paragraph: &Paragraph) -> Self {
        let automatic = hyphenation::automatic(paragraph);
        Self {
            opportunities: whitespace::opportunities(paragraph, &automatic),
            emergency: whitespace::emergency_opportunities(paragraph, false),
            automatic,
        }
    }
}

/// Paragraph-wide bidi and break context, shared across fragmentainer widths.
pub(crate) struct PreparedParagraph<'p, 'context, 'font> {
    context: &'context TextContext<'font>,
    paragraph: &'p Paragraph,
    bidi: Result<crate::text::bidi::ResolvedParagraph<'p>, crate::text::bidi::ExpansionLimit>,
    traits: super::LineTraits,
    has_leaders: bool,
    /// Automatic hyphenation opportunities, which are also among `opportunities`.
    automatic: Vec<usize>,
    opportunities: Vec<(usize, BreakOpportunity)>,
    ordinary: Vec<usize>,
    emergency: Vec<usize>,
    monotone: bool,
    plans: wrapping::Plans,
    clamp: Option<ParagraphClamp>,
    ruby: Option<ruby::RubyLayout<'p, 'context, 'font>>,
    /// The `::first-line` copy, which shapes and breaks the line from the paragraph's start.
    first_line: Option<Box<Self>>,
}

/// Lines at one fragmentainer width, retaining absolute paragraph source coordinates.
pub(crate) struct ParagraphLines<'s, 'p, 'context, 'font> {
    prepared: &'s PreparedParagraph<'p, 'context, 'font>,
    position: TextPosition<'p>,
    width: f32,
    containing_width: f32,
    inline_start: f32,
    indent: f32,
    atomics: &'s [AtomicMetrics],
    atomic_widths: Vec<f32>,
    /// The metrics of the measured blocks on the first formatted line, where the atomic
    /// inlines' `::first-line` boxes replace them (CSS Pseudo-Elements 4 §2.1.3).
    first_line_atomics: &'s [AtomicMetrics],
    first_line_widths: Vec<f32>,
    autospace: Option<Autospace<'p, 'context, 'font>>,
    finished: bool,
    /// Where its lines end as at a forced break: the paragraph's end, or that of the part of
    /// it laid out alone.
    end: usize,
}

impl<'p, 'context, 'font> PreparedParagraph<'p, 'context, 'font> {
    pub(crate) fn new(context: &'context TextContext<'font>, paragraph: &'p Paragraph) -> Self {
        Self::with_source(context, paragraph, paragraph)
    }

    /// `source` holds the annotation boxes of `paragraph`, the paragraph itself or the one its
    /// `::first-line` copies.
    fn with_source(
        context: &'context TextContext<'font>,
        paragraph: &'p Paragraph,
        source: &'p Paragraph,
    ) -> Self {
        let bidi = paragraph_bidi(paragraph, &context.bidi_work);
        let has_rtl = bidi
            .as_ref()
            .is_ok_and(super::bidi::ResolvedParagraph::has_rtl);
        let BreakOpportunities {
            automatic,
            mut opportunities,
            mut emergency,
        } = BreakOpportunities::of(paragraph);
        // The `::first-line` copy cannot end its line inside a character that it and the
        // paragraph encode differently, where the paragraph could not resume.
        let resumable = |offset: usize| {
            std::ptr::eq(paragraph, source)
                || source.first_line_offsets().to_paragraph(offset).is_some()
        };
        opportunities.retain(|&(end, _)| resumable(end));
        let ordinary = opportunities
            .iter()
            .map(|&(end, _)| end)
            .filter(|&end| !hyphenation::is_soft_break(paragraph, &automatic, end))
            .collect();
        emergency.retain(|&offset| resumable(offset));
        Self {
            context,
            paragraph,
            bidi,
            traits: super::LineTraits::of(paragraph, has_rtl),
            has_leaders: paragraph.spans().iter().any(|span| span.leader),
            automatic,
            opportunities,
            ordinary,
            emergency,
            monotone: paragraph.spans().iter().all(|span| {
                span.style.letter_spacing >= 0.0
                    && span.style.word_spacing.resolve(span.style.size) >= 0.0
            }),
            plans: wrapping::Plans::default(),
            clamp: None,
            ruby: ruby::RubyLayout::of(context, paragraph, source),
            first_line: paragraph
                .first_line()
                .map(|variant| Box::new(Self::with_source(context, variant, paragraph))),
        }
    }

    /// Lines of this paragraph end at the region break of `clamp`.
    #[must_use]
    pub(crate) fn clamped(mut self, clamp: Option<ParagraphClamp>) -> Self {
        self.clamp = clamp;
        self
    }

    /// Internal opportunities and a final explicit separator exclude the ordinary end-of-text break.
    pub(crate) fn has_break_opportunities(&self) -> bool {
        self.opportunities
            .iter()
            .any(|&(end, _)| end < self.paragraph.text().len())
            || self.paragraph.text().chars().next_back().is_some_and(|ch| {
                matches!(
                    break_property(ch as u32),
                    BreakClass::Mandatory
                        | BreakClass::CarriageReturn
                        | BreakClass::LineFeed
                        | BreakClass::NextLine
                )
            })
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
        self.lines_in_block(position, width, width, [0.0; 2], atomics)
    }

    /// Line fitting width can be narrowed by floats; percentages use the full block.
    pub(crate) fn lines_in_block<'s>(
        &'s self,
        position: TextPosition<'p>,
        width: f32,
        containing_width: f32,
        insets: [f32; 2],
        atomics: &'s [AtomicMetrics],
    ) -> Result<ParagraphLines<'s, 'p, 'context, 'font>, TextError> {
        self.bidi()?;
        if !std::ptr::eq(position.paragraph, self.paragraph) {
            return Err(TextError::ParagraphMismatch);
        }
        let expected = self.paragraph.measured_blocks().len();
        if atomics.len() != expected {
            return Err(TextError::AtomicMetrics {
                expected,
                actual: atomics.len(),
            });
        }
        Ok(ParagraphLines {
            prepared: self,
            position,
            width,
            containing_width,
            inline_start: insets[usize::from(self.paragraph.style().used_rtl())],
            indent: 0.0,
            atomics,
            atomic_widths: atomics.iter().map(|metrics| metrics.width).collect(),
            first_line_atomics: atomics,
            first_line_widths: atomics.iter().map(|metrics| metrics.width).collect(),
            autospace: Autospace::of(self.context, self.paragraph),
            finished: false,
            end: self.paragraph.text().len(),
        })
    }

    /// Lines of the part `range` of the paragraph, which ends as at a forced break: the
    /// fragment of a ruby annotation over a fragment of its base (CSS Ruby 1 §3.4.2).
    pub(crate) fn lines_between<'s>(
        &'s self,
        range: Range<usize>,
        width: f32,
        atomics: &'s [AtomicMetrics],
    ) -> Result<ParagraphLines<'s, 'p, 'context, 'font>, TextError> {
        let mut lines = self.lines(self.position_at(range.start), width, atomics)?;
        lines.end = range.end;
        Ok(lines)
    }

    /// Where the part of the paragraph from `offset` starts.
    fn position_at(&self, offset: usize) -> TextPosition<'p> {
        TextPosition {
            paragraph: self.paragraph,
            offset,
            empty_consumed: true,
            line: 0,
        }
    }

    /// The lines of the part `range` of the paragraph at its max-content width, each ending at
    /// a forced break or the part's end, before vertical alignment and justification, whose
    /// percentages resolve against `basis`; `widths` are those of its measured blocks.
    pub(super) fn max_content_lines(
        &self,
        range: Range<usize>,
        basis: f32,
        widths: Vec<f32>,
    ) -> Result<Vec<Line>, TextError> {
        self.bidi()?;
        let lines = ParagraphLines {
            prepared: self,
            position: self.position_at(range.start),
            width: basis,
            containing_width: basis,
            inline_start: 0.0,
            indent: 0.0,
            atomics: &[],
            first_line_atomics: &[],
            first_line_widths: widths.clone(),
            atomic_widths: widths,
            autospace: Autospace::of(self.context, self.paragraph),
            finished: false,
            end: range.end,
        };
        let mut from = range.start;
        let mut ends: Vec<usize> = self
            .opportunities
            .iter()
            .filter(|&&(end, opportunity)| {
                opportunity == BreakOpportunity::Mandatory && from < end && end < range.end
            })
            .map(|&(end, _)| end)
            .collect();
        ends.push(range.end);
        ends.into_iter()
            .map(|to| {
                let line = lines.emit(from, to, 0.0, Vec::new());
                from = to;
                line
            })
            .collect()
    }
}

impl<'s, 'p, 'context, 'font> ParagraphLines<'s, 'p, 'context, 'font> {
    /// Resume at the last successfully emitted line, including empty inline-edge progress.
    pub(crate) fn checkpoint(&self) -> TextPosition<'p> {
        self.position
    }

    /// Lays the first formatted line out with `atomics`, the metrics of the measured blocks
    /// with those of the atomic inlines' `::first-line` boxes in their place.
    pub(crate) fn with_first_line_atomics(
        mut self,
        atomics: &'s [AtomicMetrics],
    ) -> Result<Self, TextError> {
        if atomics.len() != self.atomics.len() {
            return Err(TextError::AtomicMetrics {
                expected: self.atomics.len(),
                actual: atomics.len(),
            });
        }
        self.first_line_widths = atomics.iter().map(|metrics| metrics.width).collect();
        self.first_line_atomics = atomics;
        Ok(self)
    }

    /// The metrics and widths of the measured blocks on the line from `start`.
    fn atomics_at(&self, start: usize) -> (&'s [AtomicMetrics], &[f32]) {
        if std::ptr::eq(self.prepared_at(start), self.prepared) {
            (self.atomics, &self.atomic_widths)
        } else {
            (self.first_line_atomics, &self.first_line_widths)
        }
    }

    /// The paragraph shaping and breaking the line from `start`: its `::first-line` copy for
    /// the first formatted line (CSS Pseudo-Elements 4 §2.1). Offsets on that line are the
    /// copy's, and source positions the paragraph's.
    fn prepared_at(&self, start: usize) -> &'s PreparedParagraph<'p, 'context, 'font> {
        match &self.prepared.first_line {
            Some(first_line) if start == 0 => first_line,
            _ => self.prepared,
        }
    }

    /// Where the `::first-line` copy laying out the line from `start`, if any, places the
    /// paragraph's offsets.
    fn copy_offsets(&self, start: usize) -> Option<&'p crate::document::OffsetMap> {
        (!std::ptr::eq(self.prepared_at(start), self.prepared))
            .then(|| self.prepared.paragraph.first_line_offsets())
    }

    /// The paragraph offset of `offset` on the line from `start`.
    fn source_offset(&self, start: usize, offset: usize) -> usize {
        self.copy_offsets(start).map_or(offset, |offsets| {
            offsets
                .to_paragraph(offset)
                .expect("the first-line copy breaks only where the paragraph resumes")
        })
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
        let prepared = self.prepared_at(from);
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
            &prepared.traits,
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
        if paragraph.style().used_rtl() {
            pieces.splice(
                0..0,
                trailing
                    .into_iter()
                    .rev()
                    .map(|run| Piece::Text(Box::new(run))),
            );
        } else {
            pieces.extend(trailing.into_iter().map(|run| Piece::Text(Box::new(run))));
        }
        let mut arranged = line_boxes::arrange(
            paragraph,
            pieces,
            from..to,
            self.atomics_at(from).1,
            available,
            line_boxes::Spacers {
                autospace: self.autospace.as_ref(),
                ruby: prepared.ruby.as_ref(),
            },
        )?;
        let end_trim = spacing_trim::trim(
            prepared.context,
            paragraph,
            &mut arranged,
            starts_block_line(paragraph, &prepared.opportunities, from),
        )?;
        super::tabs::resolve(
            prepared.context,
            paragraph,
            &mut arranged,
            self.inline_start + indent,
        )?;
        let fit = |arranged: &line_boxes::Arranged| {
            let leader_width = if prepared.has_leaders {
                crate::text::leaders::wrapping_width(paragraph, arranged, available, from)
            } else {
                None
            };
            let wrap_width = spacing::wrapping_width(
                paragraph,
                &arranged.runs,
                whitespace::wrapping_width(
                    paragraph,
                    visible.end,
                    &arranged.runs,
                    &arranged.order,
                    arranged.width,
                ),
            );
            let wrap_width = leader_width.map_or(wrap_width, |width| wrap_width.max(width));
            let hangable = hanging::hangable(
                paragraph,
                prepared.traits.hangs,
                from..to,
                visible.clone(),
                &arranged.runs,
                &arranged.order,
            );
            (wrap_width - hangable.fit(), hangable)
        };
        let (mut wrap_width, mut hangable) = fit(&arranged);
        if wrap_width > available
            && let Some(end_trim) = end_trim
        {
            end_trim.apply(&mut arranged);
            (wrap_width, hangable) = fit(&arranged);
        }
        let measured_width = arranged.width;
        if !measured_width.is_finite() || !wrap_width.is_finite() || !available.is_finite() {
            return Err(TextError::NonFiniteGeometry);
        }
        Ok(Line {
            phantom: false,
            wrapped: false,
            indent,
            text: hyphenation::text(&text[visible], hyphen),
            range: from..to,
            width: measured_width,
            wrap_width,
            runs: arranged.runs,
            boxes: arranged.boxes,
            links: Vec::new(),
            atomics: arranged.atomics,
            annotations: Vec::new(),
            ruby: arranged.ruby,
            ascent: 0.0,
            height: 0.0,
            ink_bounds: None,
            // Alignment belongs to the block container, not its first line.
            align: self.prepared.paragraph.style().align,
            decorations: Vec::new(),
            order: arranged.order,
            present: arranged.present,
            ellipsis: None,
            hangable,
            hanging: [0.0; 2],
            first_line_styles: !std::ptr::eq(prepared, self.prepared),
        })
    }

    fn finalize(&self, mut line: Line, last: bool) -> Result<Line, TextError> {
        let prepared = self.prepared_at(line.range.start);
        let paragraph = prepared.paragraph;
        line.wrapped = !last && !after_forced_break(&prepared.opportunities, line.range.end);
        let (atomics, _) = self.atomics_at(line.range.start);

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
            atomics,
            newline,
            (prepared.ruby.as_ref(), self.containing_width),
        )?;
        line.ascent = ascent;
        line.height = ascent + descent;
        let trim = [
            paragraph.text_box_trim[0] && line.range.start == 0,
            paragraph.text_box_trim[1] && line.range.end == paragraph.text().len(),
        ];
        if !line.phantom {
            trim_line(prepared.context, paragraph.style(), &mut line, trim)?;
        }
        line.hanging =
            line.hangable
                .hanging(line.width, self.available(), paragraph.style().used_rtl());
        if line.wrapped || line.width > self.available() {
            let end = whitespace::visible(paragraph, line.range.clone()).end;
            let trailing = line.width
                - whitespace::wrapping_width(paragraph, end, &line.runs, &line.order, line.width);
            let trailing = if line.wrapped {
                trailing
            } else {
                (line.width - line.hanging.iter().sum::<f32>() - self.available())
                    .clamp(0.0, trailing.max(0.0))
            };
            let side = usize::from(!paragraph.style().used_rtl());
            line.hanging[side] += trailing;
        }
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
        prepared.context.place_line_emphasis(paragraph, &mut line)?;
        if let Some(ruby) = &prepared.ruby {
            ruby.place(&mut line, atomics, self.containing_width)?;
        }
        // Annotation boxes whose contents hold boxes laid out apart have no first-line copy.
        let source = self.prepared.paragraph;
        if line.first_line_styles
            && let Some(fragment) = line.annotations.iter().find(|fragment| {
                let index = fragment.index - source.atomics().len();
                source.annotations()[index]
                    .block()
                    .first_line_box()
                    .is_none()
                    && !source
                        .annotation_box_style(index)
                        .same_text_style(paragraph.annotations()[index].block().style())
            })
        {
            return Err(TextError::FirstLineAnnotation {
                index: fragment.index - source.atomics().len(),
            });
        }
        prepared.context.finish_line_paint(paragraph, &mut line)?;
        line.range = self.source_offset(line.range.start, line.range.start)
            ..self.source_offset(line.range.start, line.range.end);
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
        if start == self.end {
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
        let prepared = self.prepared_at(self.position.offset);
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
        let prepared = self.prepared_at(start);
        let paragraph = prepared.paragraph;
        let text = paragraph.text();
        let available = self.width - indent;
        let mut best = None;
        let first = prepared
            .opportunities
            .partition_point(|&(end, _)| end <= start);
        for &(end, opportunity) in &prepared.opportunities[first..] {
            if end > self.end {
                break;
            }
            // The end of a part laid out alone breaks its last line.
            let opportunity = if end == self.end {
                BreakOpportunity::Mandatory
            } else {
                opportunity
            };
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

/// Whether a line starting at `offset` follows a forced line break.
fn after_forced_break(opportunities: &[(usize, BreakOpportunity)], offset: usize) -> bool {
    offset != 0
        && opportunities
            .binary_search_by_key(&offset, |(end, _)| *end)
            .is_ok_and(|index| opportunities[index].1 == BreakOpportunity::Mandatory)
}

/// Whether a line starting at `offset` is its block container's first line or follows a
/// forced line break.
pub(super) fn starts_block_line(
    paragraph: &Paragraph,
    opportunities: &[(usize, BreakOpportunity)],
    offset: usize,
) -> bool {
    (offset == 0 && paragraph.first_formatted_line()) || after_forced_break(opportunities, offset)
}

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
    let forced = indent.each_line && after_forced_break(opportunities, offset);
    indent.resolve(basis, first, forced)
}

/// Trims the sides of `line` that `trim` names, block-start then block-end, to the
/// `text-box-edge` metrics of its root inline box (CSS Inline 3 §6.2).
pub(crate) fn trim_line(
    context: &TextContext<'_>,
    style: &crate::style::Style,
    line: &mut Line,
    trim: [bool; 2],
) -> Result<(), TextError> {
    if !trim.contains(&true) {
        return Ok(());
    }
    let root = line_boxes::BoxMetrics::of(context, style)?;
    let edges = style.text_box.edges;
    if trim[0] {
        let over = -root.edge(edges[0], false);
        line.height -= line.ascent - over;
        line.ascent = over;
    }
    if trim[1] {
        line.height = line.ascent + root.edge(edges[1], true);
    }
    Ok(())
}
