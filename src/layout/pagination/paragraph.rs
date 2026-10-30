#[allow(
    clippy::wildcard_imports,
    reason = "pagination submodules extend the parent paginator and share its scope"
)]
use super::*;

#[derive(Clone, Copy)]
struct LineGeometry {
    x: f32,
    width: f32,
    y: f32,
}

/// An exclusion placed while fitting the lines of a fragment, kept only if the line it
/// belongs to is.
enum FloatTrial {
    /// The float of the paragraph's `anchor`th float anchor, placed at `y`.
    Float { anchor: usize, line: usize, y: f32 },
    /// The exclusion of the initial letter on the paragraph's first line.
    Letter {
        side: crate::style::FloatSide,
        margin: Rect,
    },
}
impl FloatTrial {
    fn line(&self) -> usize {
        match self {
            Self::Float { line, .. } => *line,
            Self::Letter { .. } => 0,
        }
    }
}

/// Retains source progress and inline paint state between physical page fragments.
#[derive(Clone)]
pub(super) struct ParagraphCursor<'a, 't> {
    index: usize,
    source: &'a crate::document::Paragraph,
    style: &'a crate::style::Style,
    prepared: Option<Rc<crate::text::PreparedParagraph<'a, 't, 't>>>,
    positions: std::vec::IntoIter<crate::text::TextPosition<'a>>,
    position: crate::text::TextPosition<'a>,
    lines: std::vec::IntoIter<crate::text::Line>,
    x: f32,
    width: f32,
    containing_height: Option<f32>,
    candidate_complete: bool,
    candidate_page: usize,
    geometry: VecDeque<LineGeometry>,
    anchors: std::iter::Peekable<std::vec::IntoIter<crate::layout::measure::LineAnchor<'a>>>,
    atomics: Vec<Option<crate::layout::inline::MeasuredAtomic<'a>>>,
    metrics: Vec<crate::text::AtomicMetrics>,
    consumed_anchors: usize,
    consumed_events: usize,
    consumed_floats: usize,
    line_index: usize,
    layers: InlineLayers<'a>,
    clamp: Option<crate::text::ParagraphClamp>,
    /// The atomic inline index of an initial letter on the first line, until it is placed.
    letter: Option<usize>,
    /// Whether a line of the paragraph has been placed.
    started: bool,
}

/// Where an initial letter on a fitted first line excludes the lines after it (CSS Inline 3
/// §7.5): its margin box below the line, at the line's start side.
fn letter_exclusion(
    extent: crate::layout::inline::LetterExtent,
    width: f32,
    fragment: &crate::text::AtomicFragment,
    line: &crate::text::Line,
    placement: LineGeometry,
    rtl: bool,
) -> Option<(crate::style::FloatSide, Rect)> {
    let x = placement.x + line.alignment_offset(placement.width, rtl) + fragment.x;
    let top = placement.y + line.height;
    let bottom = placement.y + extent.bottom;
    let side = if rtl {
        crate::style::FloatSide::Right
    } else {
        crate::style::FloatSide::Left
    };
    (bottom > top).then_some((
        side,
        Rect {
            x: if rtl { x } else { x - extent.kerning },
            y: top,
            width: width + extent.kerning,
            height: bottom - top,
        },
    ))
}

impl<'a> Paginator<'a, '_, 'a> {
    fn place_inline_float(
        &mut self,
        measured: floats::MeasuredFloat<'a>,
        anchor: &crate::document::Anchor,
        geometry: [f32; 3],
        layers: &InlineLayers<'a>,
    ) -> Result<(), LayoutError> {
        let chain = inline_chain(layers.paragraph, anchor.inline_box);
        let chain_layers = self.chain_layers(&chain, layers.containing);
        let saved_context = self.context.clone();
        let saved_key = self.key.clone();
        let saved_group = self.group;
        let saved_clip = self.clip;
        let saved_offset = self.offset;
        let saved_has_flow = self.has_flow;
        if let Some(layer) = chain_layers.last() {
            self.context = layer.context.clone();
            self.key = layer.key.clone();
            self.group = layer.group;
            self.clip = layer.clip;
            self.offset[0] += layer.offset[0];
            self.offset[1] += layer.offset[1];
        }
        let opened = self.open_inline_containers(&chain, &chain_layers);
        // Candidate lines preceding this float have not been painted yet.
        self.has_flow |= geometry[2] > self.y;
        let result = self.place_measured_float(measured, geometry[0], geometry[1], geometry[2]);
        self.close_inline_containers(&opened);
        self.context = saved_context;
        self.key = saved_key;
        self.group = saved_group;
        self.clip = saved_clip;
        self.offset = saved_offset;
        self.has_flow = saved_has_flow;
        result
    }

    /// Excludes the lines after the fitted first line `line`, at `placement`, that its
    /// initial letter `letter` reaches, once per fragment's trials.
    fn place_letter_trial(
        &mut self,
        letter: Option<usize>,
        atomics: &[Option<crate::layout::inline::MeasuredAtomic<'a>>],
        line: &crate::text::Line,
        placement: LineGeometry,
        rtl: bool,
        trials: &mut Vec<FloatTrial>,
    ) {
        let Some(index) = letter else {
            return;
        };
        if trials
            .iter()
            .any(|trial| matches!(trial, FloatTrial::Letter { .. }))
        {
            return;
        }
        let atomic = atomics[index]
            .as_ref()
            .expect("the initial letter is measured until its line is placed");
        let fragment = line
            .atomics
            .iter()
            .find(|fragment| fragment.index == index)
            .expect("the initial letter lies on the first line");
        if let Some((side, margin)) = letter_exclusion(
            atomic.letter.expect("an initial letter has an extent"),
            atomic.metrics.width,
            fragment,
            line,
            placement,
            rtl,
        ) {
            self.floats
                .last_mut()
                .expect("flow has a float context")
                .insert_letter(self.page, side, margin);
            trials.push(FloatTrial::Letter { side, margin });
        }
    }

    fn fit_float_line(
        &self,
        prepared: &crate::text::PreparedParagraph<'a, 'a, 'a>,
        position: crate::text::TextPosition<'a>,
        metrics: &[crate::text::AtomicMetrics],
        bounds: [f32; 2],
        mut y: f32,
    ) -> Result<
        Option<(
            crate::text::Line,
            crate::text::TextPosition<'a>,
            LineGeometry,
        )>,
        LayoutError,
    > {
        let context = self.floats.last().expect("flow has a float context");
        let mut height = 0.0f32;
        loop {
            self.measurer.work.spend(metrics.len().max(1))?;
            let (band, met) =
                context.line_band(self.page, y, height, bounds, &self.measurer.work)?;
            let width = (band[1] - band[0]).max(0.0);
            let mut lines = prepared.lines_in_block(
                position,
                width,
                bounds[1] - bounds[0],
                [band[0] - bounds[0], bounds[1] - band[1]],
                metrics,
            )?;
            let Some(line) = lines.next() else {
                return Ok(None);
            };
            let line = line?;
            self.measurer.work.spend(line.range.len().max(1))?;
            if line.height > height {
                height = line.height;
                continue;
            }
            if line.width + line.indent > width && met {
                y = context.line_fit(
                    self.page,
                    y,
                    height,
                    bounds,
                    line.width + line.indent,
                    &self.measurer.work,
                )?;
                height = 0.0;
                continue;
            }
            return Ok(Some((
                line,
                lines.checkpoint(),
                LineGeometry {
                    x: band[0],
                    width,
                    y,
                },
            )));
        }
    }
    pub(super) fn paragraph_break(&mut self) -> Option<BreakRequest> {
        let scope = self
            .boxes
            .iter()
            .rev()
            .find_map(|active| active.page_scope.clone());
        let changed = scope.is_some_and(|name| self.page_name(name, &[]));
        Forced::combine(self.pending_break, changed.then_some(Forced::Page))
            .filter(|_| self.unbroken == 0 && self.has_flow)
            .map(|forced| BreakRequest {
                forced: Some(forced),
                preserve_empty: false,
            })
    }
    pub(super) fn start_paragraph(
        &mut self,
        paragraph: MeasuredParagraph<'a>,
    ) -> ParagraphCursor<'a, 'a> {
        if !paragraph.is_phantom() {
            self.pending_break = None;
            self.flush_margin();
        }
        let prepared = None;
        let mut positions = paragraph.positions.into_iter();
        let position = positions
            .next()
            .expect("paragraph measurement has a start checkpoint");
        let lines = paragraph.lines.into_iter();
        let x = paragraph.x;
        let width = paragraph.width;
        let containing_height = paragraph.containing_height;
        let candidate_complete = true;
        let candidate_page = self.page;
        let style = paragraph.style;
        let mut anchors = paragraph.anchors.into_iter().peekable();
        let atomics = paragraph.atomics;
        let mut metrics = Vec::new();
        crate::layout::inline::paragraph_metrics(paragraph.paragraph, &atomics, &mut metrics);
        let consumed_anchors = 0;
        let consumed_events = 0;
        let line_index = 0;
        let layers = InlineLayers::new(
            paragraph.paragraph,
            InlineContaining {
                width,
                height: containing_height,
                rtl: style.used_rtl(),
            },
        );
        if lines.as_slice().is_empty() {
            for anchor in anchors.by_ref() {
                self.line_anchor(&anchor, x, width, 0.0, 0.0, &layers);
            }
        }
        let source = paragraph.paragraph;
        let index = paragraph.index;
        let clamp = paragraph.clamp;
        let letter = source
            .atomics()
            .first()
            .filter(|atomic| atomic.offset == 0 && atomic.block.initial_letter().is_some())
            .map(|_| 0);
        ParagraphCursor {
            index,
            source,
            style,
            prepared,
            positions,
            position,
            lines,
            x,
            width,
            containing_height,
            candidate_complete,
            candidate_page,
            geometry: VecDeque::new(),
            anchors,
            atomics,
            metrics,
            consumed_anchors,
            consumed_events,
            consumed_floats: 0,
            line_index,
            layers,
            clamp,
            letter,
            started: false,
        }
    }
    pub(super) fn paragraph_fragment(
        &mut self,
        cursor: Box<ParagraphCursor<'a, 'a>>,
        tail: BreakTail,
    ) -> Result<Option<(Box<ParagraphCursor<'a, 'a>>, BreakRequest)>, LayoutError> {
        let text = self.measurer.text;
        let ParagraphCursor {
            index,
            source,
            style,
            mut prepared,
            mut positions,
            mut position,
            mut lines,
            mut x,
            mut width,
            mut containing_height,
            mut candidate_complete,
            mut candidate_page,
            mut geometry,
            mut anchors,
            mut atomics,
            mut metrics,
            mut consumed_anchors,
            mut consumed_events,
            mut consumed_floats,
            mut line_index,
            mut layers,
            clamp,
            mut letter,
            mut started,
        } = *cursor;
        let float_anchors: Vec<_> = source
            .anchors()
            .iter()
            .filter(|anchor| anchor.block.style().floating().is_some())
            .collect();
        let has_inline_floats = consumed_floats < float_anchors.len() || letter.is_some();
        let mut snapshot = has_inline_floats.then(|| self.snapshot()).transpose()?;
        let saved_layers = has_inline_floats.then(|| layers.clone());
        let mut trials: Vec<FloatTrial> = Vec::new();
        let mut request = BreakRequest::default();
        'fragment: {
            if lines.as_slice().is_empty() && candidate_complete {
                break 'fragment;
            }
            self.measurer.work.spend(1)?;
            let reserve = self.continuation_reserve()?;
            let closing_reserve = self.closing_reserve(tail, reserve)?;
            {
                let (left, destination_width, height) =
                    self.paragraph_geometry()
                        .unwrap_or((x, width, containing_height));
                x = left;
                let floating = has_inline_floats
                    || !self
                        .floats
                        .last()
                        .expect("flow has a float context")
                        .is_empty();
                if floating
                    || width != destination_width
                    || (!atomics.is_empty() && containing_height != height)
                    || (!candidate_complete && candidate_page != self.page)
                {
                    width = destination_width;
                    containing_height = height;
                    for (index, atomic) in atomics.iter_mut().enumerate() {
                        if atomic.is_some() {
                            let measured = self.measurer.run(|measurer| {
                                crate::layout::inline::measure(
                                    measurer,
                                    &source.inline_block(index).block,
                                    width,
                                    height,
                                    style.used_rtl(),
                                )
                            })?;
                            *atomic = Some(measured);
                        }
                    }
                    crate::layout::inline::paragraph_metrics(source, &atomics, &mut metrics);
                    if prepared.is_none() {
                        self.measurer.work.spend(source.text().len().max(1))?;
                    }
                    self.measurer.work.spend(metrics.len().max(1))?;
                    let prepared = prepared.get_or_insert_with(|| {
                        Rc::new(text.prepare_paragraph(source).clamped(clamp.clone()))
                    });
                    let mut stream = prepared.lines(position, width, &metrics)?;
                    geometry.clear();
                    let mut float_position = position;
                    let mut remainder = Vec::new();
                    let mut checkpoints = Vec::new();
                    candidate_complete = true;
                    candidate_page = self.page;
                    let mut extent = self.y + reserve;
                    let mut available: usize = 0;
                    let mut overflow = false;
                    let mut line_clearance_y = None;
                    loop {
                        let (line, checkpoint) = if floating {
                            let cleared_y = if let Some(y) = line_clearance_y {
                                y
                            } else {
                                let (clear, rtl) = source.clearance_at(float_position.offset());
                                let y = self
                                    .floats
                                    .last()
                                    .expect("flow has a float context")
                                    .clearance_end(self.page, clear, rtl, &self.measurer.work)?
                                    .map_or(extent - reserve, |(page, bottom)| {
                                        if page > self.page {
                                            self.height.max(extent - reserve)
                                        } else {
                                            bottom.max(extent - reserve)
                                        }
                                    });
                                // An initial letter's line clears earlier initial letters
                                // (CSS Inline 3 §7.9.2) and moves down for the letter to
                                // stay below the content edge (§7.6.1).
                                let y = match letter.and_then(|index| atomics[index].as_ref()) {
                                    Some(atomic) if remainder.is_empty() => self
                                        .floats
                                        .last()
                                        .expect("flow has a float context")
                                        .letter_clearance(self.page, y, &self.measurer.work)?
                                        .max(
                                            extent - reserve
                                                + atomic
                                                    .letter
                                                    .expect("an initial letter has an extent")
                                                    .rise(),
                                        ),
                                    _ => y,
                                };
                                line_clearance_y = Some(y);
                                y
                            };
                            let Some((line, checkpoint, placement)) = self.fit_float_line(
                                prepared,
                                float_position,
                                &metrics,
                                [x, x + width],
                                cleared_y,
                            )?
                            else {
                                break;
                            };
                            let next_anchor = consumed_floats
                                + trials
                                    .iter()
                                    .filter(|trial| matches!(trial, FloatTrial::Float { .. }))
                                    .count();
                            if let Some(anchor) = float_anchors.get(next_anchor).filter(|anchor| {
                                anchor.offset < line.range.end
                                    || (anchor.offset == line.range.end
                                        && !source.text()[line.range.clone()]
                                            .ends_with(['\n', '\u{2028}', '\u{2029}']))
                            }) {
                                let measured =
                                    self.measure_float(&anchor.block, width, containing_height)?;
                                let prefix = (line.x_at(source, anchor.offset, style.used_rtl())
                                    - line.x_at(source, line.range.start, style.used_rtl()))
                                .abs();
                                let occupied =
                                    prefix + if prefix > 0.0 { line.indent } else { 0.0 };
                                let y = placement.y
                                    + if measured.size[0] + occupied > placement.width {
                                        line.height
                                    } else {
                                        0.0
                                    };
                                // A float after the first line clears the initial letter
                                // beside it, which then excludes the lines it reaches.
                                if y > placement.y && remainder.is_empty() {
                                    self.place_letter_trial(
                                        letter,
                                        &atomics,
                                        &line,
                                        placement,
                                        style.used_rtl(),
                                        &mut trials,
                                    );
                                }
                                self.place_inline_float(measured, anchor, [x, width, y], &layers)?;
                                trials.push(FloatTrial::Float {
                                    anchor: next_anchor,
                                    line: remainder.len(),
                                    y,
                                });
                                continue;
                            }
                            if remainder.is_empty() {
                                self.place_letter_trial(
                                    letter,
                                    &atomics,
                                    &line,
                                    placement,
                                    style.used_rtl(),
                                    &mut trials,
                                );
                            }
                            extent = placement.y + reserve;
                            geometry.push_back(placement);
                            float_position = checkpoint;
                            line_clearance_y = None;
                            (line, checkpoint)
                        } else {
                            let Some(line) = stream.next() else {
                                break;
                            };
                            (line?, stream.checkpoint())
                        };
                        self.measurer.work.spend(line.range.len().max(1))?;
                        extent += line.height;
                        overflow |= overflows(extent, self.height);
                        if !overflow {
                            available += 1;
                        }
                        remainder.push(line);
                        checkpoints.push(checkpoint);
                        if overflow && remainder.len() > available.saturating_add(style.widows) {
                            candidate_complete = false;
                            break;
                        }
                    }
                    anchors = crate::layout::measure::line_anchors(
                        source,
                        &remainder,
                        candidate_complete,
                    )
                    .into_iter()
                    .skip(consumed_anchors)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .peekable();
                    line_index = 0;
                    positions = checkpoints.into_iter();
                    lines = remainder.into_iter();
                }
            }
            layers.set_containing(InlineContaining {
                width,
                height: containing_height,
                rtl: style.used_rtl(),
            });
            let remaining = lines.as_slice().len();
            let mut height = 0.0;
            // The footnote area grows with the footnotes of the lines the page takes; a line
            // whose footnotes must stay with it does not fit without them (CSS GCPM 3 §2.8).
            let mut growth = 0.0;
            let mut pending_footnotes = Vec::new();
            let mut footnote_events = consumed_events;
            let mut kept_footnotes = None;
            // The line of a nested layout that an earlier pagination moved off this page.
            let mut deferred_line = None;
            let mut available = 0;
            for (index, line) in lines.as_slice().iter().enumerate() {
                height = geometry
                    .get(index)
                    .map_or(height + line.height, |placement| {
                        placement.y - self.y + line.height
                    });
                if overflows(self.y + height + reserve + growth, self.height) {
                    break;
                }
                let footnotes = line_footnotes(source, line, &mut footnote_events);
                if let Some(policy) = self.deferred_policy(&footnotes) {
                    kept_footnotes = Some(policy);
                    deferred_line = Some(index);
                    break;
                }
                if !footnotes.is_empty() && self.places_footnotes() {
                    match self.footnote_growth(&footnotes, &mut pending_footnotes)? {
                        Some(added)
                            if !overflows(
                                self.y + height + reserve + growth + added,
                                self.height,
                            ) =>
                        {
                            growth += added;
                        }
                        _ => match self.strictest_policy(&footnotes) {
                            // Its footnotes then fill the rest of the page and continue after.
                            crate::style::FootnotePolicy::Auto => {
                                available += 1;
                                break;
                            }
                            policy => {
                                kept_footnotes = Some(policy);
                                break;
                            }
                        },
                    }
                }
                available += 1;
            }
            let mut take = if self.unbroken > 0 {
                remaining
            } else {
                available.min(remaining)
            };
            let next_geometry = if take > 0 && take < remaining && style.widows > 1 {
                self.next_paragraph_geometry()?
                    .filter(|(next_width, next_height)| {
                        *next_width != width
                            || (!atomics.is_empty() && *next_height != containing_height)
                    })
            } else {
                None
            };
            if take < remaining {
                if take < style.orphans
                    || (next_geometry.is_none()
                        && remaining < style.orphans.saturating_add(style.widows))
                {
                    take = 0;
                } else if next_geometry.is_none() && remaining - take < style.widows {
                    take = remaining - style.widows;
                }
            }
            if take > 0
                && let Some((next_width, next_height)) = next_geometry
            {
                let mut next_metrics = metrics.clone();
                let mut next_atomics = Vec::with_capacity(atomics.len());
                for (index, atomic) in atomics.iter().enumerate() {
                    next_atomics.push(
                        atomic
                            .is_some()
                            .then(|| {
                                self.measurer.run(|measurer| {
                                    crate::layout::inline::measure(
                                        measurer,
                                        &source.inline_block(index).block,
                                        next_width,
                                        next_height,
                                        style.used_rtl(),
                                    )
                                })
                            })
                            .transpose()?,
                    );
                }
                crate::layout::inline::paragraph_metrics(source, &next_atomics, &mut next_metrics);
                if prepared.is_none() {
                    self.measurer.work.spend(source.text().len().max(1))?;
                }
                let prepared = prepared.get_or_insert_with(|| {
                    Rc::new(text.prepare_paragraph(source).clamped(clamp.clone()))
                });
                while take > 0 {
                    self.measurer.work.spend(next_metrics.len().max(1))?;
                    let checkpoint = positions.as_slice()[take - 1];
                    let mut next_lines = prepared.lines(checkpoint, next_width, &next_metrics)?;
                    let mut count = 0;
                    for line in next_lines.by_ref().take(style.widows) {
                        self.measurer.work.spend(line?.range.len().max(1))?;
                        count += 1;
                    }
                    if count >= style.widows {
                        break;
                    }
                    take -= 1;
                    if take < style.orphans {
                        take = 0;
                    }
                }
            }
            if self.unbroken == 0
                && candidate_complete
                && take == remaining
                && overflows(self.y + height + growth + closing_reserve, self.height)
            {
                take = if remaining >= style.orphans.saturating_add(style.widows) {
                    remaining - style.widows
                } else {
                    0
                };
            }
            // An initial letter is monolithic, and breaks between the lines beside it are
            // avoided (CSS Inline 3 §7.9.4).
            if take > 0
                && self.unbroken == 0
                && self.has_flow
                && let Some(atomic) = letter.and_then(|index| atomics[index].as_ref())
                && let Some(first) = geometry.front()
            {
                let bottom = first.y
                    + atomic
                        .letter
                        .expect("an initial letter has an extent")
                        .bottom;
                let beside = 1 + geometry
                    .iter()
                    .skip(1)
                    .filter(|line| line.y < bottom)
                    .count();
                if take < beside.min(remaining) || overflows(bottom, self.height) {
                    take = 0;
                }
            }
            // A deferred line may also break before the content of its nested layout, when
            // content precedes it on its physical page.
            let has_flow = if deferred_line.is_some() {
                self.has_page_flow()
            } else {
                self.has_flow
            };
            // `footnote-policy: block` breaks before the paragraph holding the call.
            if kept_footnotes == Some(crate::style::FootnotePolicy::Block) && !started && has_flow {
                take = 0;
            }
            if take == 0 && !has_flow {
                take = available.min(remaining).max(1);
            }
            let accepted = trials.partition_point(|trial| trial.line() < take);
            if accepted < trials.len() {
                self.restore_state(snapshot.take().expect("inline float trial has a snapshot"));
                layers = saved_layers.expect("inline float trial saves its layers");
                for trial in &trials[..accepted] {
                    match *trial {
                        FloatTrial::Float { anchor, y, .. } => {
                            let anchor = float_anchors[anchor];
                            let measured =
                                self.measure_float(&anchor.block, width, containing_height)?;
                            self.place_inline_float(measured, anchor, [x, width, y], &layers)?;
                        }
                        FloatTrial::Letter { side, margin } => self
                            .floats
                            .last_mut()
                            .expect("flow has a float context")
                            .insert_letter(self.page, side, margin),
                    }
                }
            }
            consumed_floats += trials[..accepted]
                .iter()
                .filter(|trial| matches!(trial, FloatTrial::Float { .. }))
                .count();
            // The break before a deferred line is a forced page break (CSS GCPM 3 §2.8),
            // which in columns leaves their row.
            if deferred_line.is_some_and(|line| take <= line)
                && matches!(self.fragmentainers, Fragmentainers::Columns(_))
            {
                request.forced = Some(Forced::Page);
            }
            if take == 0 {
                break 'fragment;
            }
            let fragment_trim = self.fragment_text_trim();
            for mut line in lines.by_ref().take(take) {
                self.measurer.work.spend(line.range.len().max(1))?;
                // CSS Inline 3 §6.2: the first and last formatted lines of each column, and of
                // each fragment of a cloned box, trim unless the paragraph already trims them.
                let trimmed = [
                    source.text_box_trim[0] && line.range.start == 0,
                    source.text_box_trim[1] && line.range.end == source.text().len(),
                ];
                if !line.phantom
                    && fragment_trim[0]
                    && !trimmed[0]
                    && !self.has_flow
                    && self.y <= FRAGMENTATION_TOLERANCE_PT
                {
                    crate::text::trim_line(text, style, &mut line, [true, false])?;
                }
                let end_trim = if !line.phantom && fragment_trim[1] && !trimmed[1] {
                    let mut trimmed_line = line.clone();
                    crate::text::trim_line(text, style, &mut trimmed_line, [false, true])?;
                    line.height - trimmed_line.height
                } else {
                    0.0
                };
                position = positions
                    .next()
                    .expect("each measured line has its source checkpoint");
                let (line_x, line_width) = if let Some(placement) = geometry.pop_front() {
                    self.y = placement.y;
                    (placement.x, placement.width)
                } else {
                    (x, width)
                };
                let offset = line.alignment_offset(line_width, style.used_rtl());
                while let Some(anchor) = anchors.next_if(|anchor| anchor.line == line_index) {
                    consumed_anchors += 1;
                    self.line_anchor(&anchor, line_x, line_width, offset, line.height, &layers);
                }
                let footnotes = self.line_events(source, &line, &mut consumed_events);
                line_index += 1;
                let baseline = self.top + self.y + line.ascent + self.offset[1];
                if !line.phantom {
                    self.record_baselines(self.page, [Some(baseline); 2]);
                }
                self.record_overflow(Rect {
                    x: line_x + offset + self.offset[0],
                    y: self.top + self.y + self.offset[1],
                    width: line.width,
                    height: line.height,
                });
                self.record_line_box(line.height);
                let markers: Vec<_> = self
                    .boxes
                    .iter_mut()
                    .filter_map(|active| {
                        active
                            .marker
                            .take()
                            .filter(|_| active.visible)
                            .map(|mut marker| {
                                marker.fragment.x += self.offset[0] - active.offset[0];
                                marker
                            })
                    })
                    .collect();
                for mut marker in markers {
                    marker.fragment.baseline = baseline;
                    self.emit_marker(*marker);
                }
                let line_layers = layers.line(&line);
                self.inline_content(
                    &line,
                    line_width,
                    line_x + offset,
                    &mut atomics,
                    line_layers,
                )?;
                if !line.phantom {
                    self.record_baselines(self.page, [Some(baseline); 2]);
                }
                letter = None;
                self.emit_paragraph_line(
                    index,
                    style,
                    &mut line,
                    [line_x + offset + self.offset[0], baseline],
                    line_layers,
                );
                self.y += line.height;
                started = true;
                self.trailing_line = (end_trim != 0.0).then_some((self.y, end_trim));
                if !line.phantom && (line.height > 0.0 || !line.text.is_empty()) {
                    self.has_flow = true;
                    self.at_page_start = false;
                }
                if !footnotes.is_empty() && self.places_footnotes() {
                    self.place_footnotes(&footnotes)?;
                }
            }
        }
        let mut trailing_clearance = false;
        if lines.as_slice().is_empty() && candidate_complete {
            let (clear, rtl) = source.clearance_at(source.text().len());
            if let Some((page, bottom)) = self
                .floats
                .last()
                .expect("flow has a float context")
                .clearance_end(self.page, clear, rtl, &self.measurer.work)?
            {
                trailing_clearance =
                    self.unbroken == 0 && (page > self.page || overflows(bottom, self.height));
                self.y = self.y.max(if trailing_clearance {
                    self.height
                } else {
                    bottom
                });
            }
        }
        if !lines.as_slice().is_empty() || !candidate_complete || trailing_clearance {
            let cursor = Box::new(ParagraphCursor {
                index,
                source,
                style,
                prepared,
                positions,
                position,
                lines,
                x,
                width,
                containing_height,
                candidate_complete,
                candidate_page,
                geometry,
                anchors,
                atomics,
                metrics,
                consumed_anchors,
                consumed_events,
                consumed_floats,
                line_index,
                layers,
                clamp,
                letter,
                started,
            });
            return Ok(Some((cursor, request)));
        }
        if let Some((left, destination_width, height)) = self.paragraph_geometry() {
            x = left;
            width = destination_width;
            containing_height = height;
        }
        for anchor in &float_anchors[consumed_floats..] {
            self.measurer.work.spend(1)?;
            let measured = self.measure_float(&anchor.block, width, containing_height)?;
            self.place_inline_float(measured, anchor, [x, width, self.y], &layers)?;
        }
        for event in &source.semantic_events()[consumed_events..] {
            self.string_locations.record(self.page, &event.event);
        }
        self.finish_inline_backgrounds(&layers);
        Ok(None)
    }
    /// Records the semantic events of a placed line; returns the footnotes it calls.
    fn line_events(
        &mut self,
        source: &crate::document::Paragraph,
        line: &crate::text::Line,
        consumed_events: &mut usize,
    ) -> Vec<usize> {
        let text_start = (!line.phantom)
            .then(|| {
                source.text()[line.range.clone()]
                    .char_indices()
                    .find(|(_, ch)| !crate::document::is_css_whitespace(*ch))
                    .map(|(offset, _)| line.range.start + offset)
            })
            .flatten();
        let mut footnotes = Vec::new();
        for event in line_semantic_events(source, line, *consumed_events) {
            if text_start.is_some_and(|start| start < event.offset) {
                self.string_locations.text(self.page);
            }
            self.string_locations.record(self.page, &event.event);
            footnotes.extend(event.event.footnote);
            *consumed_events += 1;
        }
        if !line.phantom {
            self.string_locations.text(self.page);
        }
        footnotes
    }

    fn emit_paragraph_line(
        &mut self,
        index: usize,
        style: &crate::style::Style,
        line: &mut crate::text::Line,
        origin: [f32; 2],
        line_layers: &InlineLayers<'a>,
    ) {
        let paragraph = line_layers.paragraph;
        // Runs inside inline boxes with their own layers paint in them.
        let parts = self.split_line(
            line_layers,
            std::mem::take(&mut line.runs),
            &line.decorations,
        );
        let grouped = parts.len() > 1;
        let decorates = |run: &crate::text::GlyphRun, decoration: &DecorationPaint| {
            decoration.owner.is_none_or(|owner| {
                paragraph.is_within(paragraph.spans()[run.source_span].inline_box, owner)
            })
        };
        for part in parts {
            if self.force_hidden || (grouped && part.runs.is_empty() && part.decorations.is_empty())
            {
                continue;
            }
            let ink_origin = [
                origin[0] + part.layer.offset[0],
                origin[1] + part.layer.offset[1],
            ];
            for run in &part.runs {
                let style = &paragraph.spans()[run.source_span].style;
                self.anchors.text(
                    crate::layout::anchors::ink_key(style),
                    self.page,
                    run,
                    ink_origin,
                    part.layer.clip,
                    crate::layout::writing::Frame::for_style(style),
                );
            }
            for decoration in &part.decorations {
                let style = decoration.owner.map_or_else(
                    || paragraph.style(),
                    |owner| &paragraph.boxes()[owner].style,
                );
                for fill in decoration.fills() {
                    if let Some(rect) = crate::paint::shape_bounds(&fill.shape) {
                        self.anchors.ink(
                            crate::layout::anchors::ink_key(style),
                            self.page,
                            Rect {
                                x: rect.x + ink_origin[0],
                                y: rect.y + ink_origin[1],
                                ..rect
                            },
                            part.layer.clip,
                        );
                    }
                }
            }
            self.emit_line(
                TextLine {
                    paragraph: index,
                    text: line.text.clone(),
                    x: origin[0],
                    baseline: origin[1],
                    width: line.width,
                    height: line.height,
                    runs: part.runs,
                    font_size: style.size,
                    color: style.color,
                },
                line.ink_bounds,
                &part.decorations,
                &part.layer,
                &decorates,
            );
        }
    }

    pub(super) fn row_text(
        &mut self,
        mut text: crate::layout::measure::RowText<'a>,
        row_top: f32,
    ) -> Result<(), LayoutError> {
        let glyphs = text
            .line
            .runs
            .iter()
            .map(|run| run.glyphs().len())
            .sum::<usize>();
        self.measurer.work.spend(
            glyphs
                .saturating_add(text.line.text.len())
                .saturating_add(text.line.boxes.len())
                .saturating_add(text.line.links.len())
                .saturating_add(text.line.decorations.len())
                .saturating_add(text.source.semantic_events().len())
                .max(1),
        )?;
        self.y = row_top + text.baseline - text.line.ascent;
        let mut consumed_events = 0;
        let footnotes = self.line_events(text.source, &text.line, &mut consumed_events);
        if !footnotes.is_empty() && self.places_footnotes() {
            self.place_footnotes(&footnotes)?;
        }
        let baseline = self.top + row_top + text.baseline + self.offset[1];
        self.record_baselines(self.page, [Some(baseline); 2]);
        self.record_overflow(Rect {
            x: text.x + self.offset[0],
            y: self.top + self.y + self.offset[1],
            width: text.line.width,
            height: text.line.height,
        });
        self.record_line_box(text.line.height);
        let containing = InlineContaining::of(self.containing_geometry());
        let mut layers = InlineLayers::new(text.source, containing);
        let line_layers = layers.line(&text.line);
        self.inline_content(&text.line, containing.width, text.x, &mut [], line_layers)?;
        let index = self.measurer.sources.paragraph(text.source);
        self.emit_paragraph_line(
            index,
            text.source.style(),
            &mut text.line,
            [text.x + self.offset[0], baseline],
            line_layers,
        );
        self.finish_inline_backgrounds(&layers);
        Ok(())
    }

    fn finish_inline_backgrounds(&mut self, layers: &InlineLayers<'a>) {
        for (inline_box, fragments) in layers.painted() {
            if !inline_box.style.clone_decoration {
                assemble_background(
                    fragments,
                    &mut self.foreground,
                    true,
                    inline_box.style.used_rtl(),
                );
            }
        }
    }
}

/// The semantic events of a paragraph's line from the `consumed`th on.
fn line_semantic_events<'p>(
    source: &'p crate::document::Paragraph,
    line: &crate::text::Line,
    consumed: usize,
) -> impl Iterator<Item = &'p crate::document::InlineEvent> {
    let end = line.range.end;
    let last = end == source.text().len();
    source
        .semantic_events()
        .iter()
        .skip(consumed)
        .take_while(move |event| event.offset < end || last)
}

/// The footnotes a candidate line calls, advancing `consumed` past its events.
fn line_footnotes(
    source: &crate::document::Paragraph,
    line: &crate::text::Line,
    consumed: &mut usize,
) -> Vec<usize> {
    let events: Vec<_> = line_semantic_events(source, line, *consumed).collect();
    *consumed += events.len();
    events
        .into_iter()
        .filter_map(|event| event.event.footnote)
        .collect()
}
