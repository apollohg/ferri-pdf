//! Ruby layout (CSS Ruby 1 §§3–5). A ruby container's bases lie on the line as inline boxes;
//! each line's columns are sized to their widest content, the bases distributing the extra
//! space by `ruby-align` before the line is fitted, and its annotation boxes are then placed
//! over or under them, stacked level by level against the base container. A line breaking
//! inside a base breaks its annotations in parallel, each line holding a fragment of each.
use crate::document::{
    Paragraph,
    ruby::{Annotation, Level, Ruby, Segment},
};
use crate::fonts::{FALLBACK_IDEOGRAPH_ADVANCE_EM, WATER_IDEOGRAPH};
use crate::style::{RubyAlign, RubyMerge, Style};
use crate::text::line_boxes::{
    Arranged, AtomicFragment, BoxFragment, BoxMetrics, Slot, slot_width,
};
use crate::text::{
    GlyphRun, Line, PreparedParagraph, TextContext, TextError, autospace, hyphenation, spacing,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use unicode_linebreak::BreakOpportunity;
use unicode_script::{Script, UnicodeScript};

/// The one unit an annotation box's contents count as where they hold no text to break, and
/// its breaks: a line holds it whole or not at all.
const WHOLE_UNIT: Range<usize> = 0..1;
const WHOLE_BREAKS: [(usize, f32); 2] = [(WHOLE_UNIT.start, 0.0), (WHOLE_UNIT.end, 1.0)];
/// JIS X 4051's maximum overhang, which §5.1 suggests: half an ideographic advance in the
/// annotation's font.
const MAX_OVERHANG_IC: f32 = 0.5;
const HALF: f32 = 0.5;
/// The order of spacing inserted at one slot position, left to right.
const TRAILING_RANK: usize = 0;
const GAP_RANK: usize = 1;
const LEADING_RANK: usize = 2;
const SPACE: char = ' ';
const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

/// The spacing a line gives one of its ruby bases: the space before and after its contents
/// within its column, and how far its column extends past the ruby container's start and
/// end edges over the adjacent text it overhangs (§5.1).
#[derive(Clone, Debug)]
pub(crate) struct ColumnSpacing {
    base: usize,
    leading: f32,
    trailing: f32,
    overhang: [f32; 2],
}

/// The ruby layout of a line: the spacing of its bases, and the annotation levels of each
/// ruby segment, by ruby, segment and level, that `ruby-merge: auto` merges on it.
#[derive(Clone, Debug, Default)]
pub(crate) struct RubyLine {
    columns: Vec<ColumnSpacing>,
    merged: Vec<[usize; 3]>,
}

/// One annotation as a line places it, or a run of them merged into one (§4.2).
struct Placed {
    annotations: Range<usize>,
    /// The first and last of the line's bases it spans.
    bases: [usize; 2],
    merged: bool,
}

/// What laying out part of an annotation box's contents at its max-content width shows about
/// it: its width, its justification opportunities, and its first and last characters, which
/// adjoin the next and previous annotation of a merged run.
#[derive(Clone)]
struct Contents {
    width: f32,
    opportunities: usize,
    first: String,
    last: String,
}

/// An annotation box's contents laid out in part: the box, the part, and the bits of the
/// widths of the box and of its contents' measured blocks.
type ContentsKey = (usize, Range<usize>, Vec<u32>);

/// The ruby containers of a paragraph and what laying them out measures once.
pub(crate) struct RubyLayout<'p, 'c, 'f> {
    context: &'c TextContext<'f>,
    paragraph: &'p Paragraph,
    /// The paragraph whose annotation boxes hold the annotations' contents: `paragraph`, or
    /// the one whose first line it is the `::first-line` copy of.
    source: &'p Paragraph,
    /// Rubies nested in another's bases come before it.
    order: Vec<usize>,
    contents: RefCell<HashMap<ContentsKey, Contents>>,
    /// Each annotation box's contents, prepared to lay out in part, with their breaks.
    annotations: RefCell<HashMap<usize, Rc<AnnotationText<'p, 'c, 'f>>>>,
    spaces: RefCell<HashMap<*const Style, f32>>,
}

/// An annotation box's contents and where they can break, with the share of their text before
/// each break.
struct AnnotationText<'p, 'c, 'f> {
    prepared: PreparedParagraph<'p, 'c, 'f>,
    breaks: Vec<(usize, f32)>,
}

/// The `[first, last]` positions in `order` of the slots inside inline box `owner`.
fn slot_span(paragraph: &Paragraph, arranged: &Arranged, owner: usize) -> Option<[usize; 2]> {
    let inside = |slot: &Slot| paragraph.is_within(arranged.owner(paragraph, *slot), owner);
    Some([
        arranged.order.iter().position(inside)?,
        arranged.order.iter().rposition(inside)?,
    ])
}

/// Kana, which annotations may overhang without obscuring an ideograph's reading (§5.1).
fn kana(unit: &str) -> bool {
    unit.chars().next().is_some_and(|ch| {
        let extensions = ch.script_extension();
        extensions.contains_script(Script::Hiragana) || extensions.contains_script(Script::Katakana)
    })
}

/// The block-start and block-end padding, borders and margins of an inline box.
fn block_edges(style: &Style, basis: f32) -> [f32; 2] {
    [TOP, BOTTOM].map(|side| {
        style.padding[side].resolve(basis)
            + style.decoration.borders[side].used_width()
            + style.margin[side].resolve(basis).unwrap_or(0.0)
    })
}

/// `[above, below]` an inline box's baseline of its margin box: its content area and its
/// block-axis padding, borders and margins.
fn margin_extent(
    context: &TextContext<'_>,
    style: &Style,
    basis: f32,
) -> Result<[f32; 2], TextError> {
    let metrics = BoxMetrics::of(context, style)?;
    let [top, bottom] = block_edges(style, basis);
    Ok([metrics.ascent + top, metrics.descent + bottom])
}

impl<'p, 'c, 'f> RubyLayout<'p, 'c, 'f> {
    /// `None` where the paragraph holds no ruby.
    pub(super) fn of(
        context: &'c TextContext<'f>,
        paragraph: &'p Paragraph,
        source: &'p Paragraph,
    ) -> Option<Self> {
        let rubies = paragraph.rubies();
        if rubies.is_empty() {
            return None;
        }
        let mut order: Vec<usize> = (0..rubies.len()).collect();
        order.sort_by_key(|&index| std::cmp::Reverse(rubies[index].container));
        Some(Self {
            context,
            paragraph,
            source,
            order,
            contents: RefCell::new(HashMap::new()),
            annotations: RefCell::new(HashMap::new()),
            spaces: RefCell::new(HashMap::new()),
        })
    }

    fn rubies(&self) -> impl Iterator<Item = (usize, &'p Ruby)> {
        let rubies = self.paragraph.rubies();
        self.order.iter().map(move |&index| (index, &rubies[index]))
    }

    /// The width of a collapsible space in `style`'s font, with its spacing.
    fn space_width(&self, style: &Style) -> Result<f32, TextError> {
        let key = std::ptr::from_ref(style);
        if let Some(width) = self.spaces.borrow().get(&key) {
            return Ok(*width);
        }
        let width = autospace::advance(self.context, style, SPACE)?.unwrap_or_default()
            + style.word_spacing.resolve(style.size)
            + style.letter_spacing;
        self.spaces.borrow_mut().insert(key, width);
        Ok(width)
    }

    /// The width an annotation of `segment` takes on a line holding the paragraph's part
    /// `line`, from the widths of the paragraph's measured blocks.
    fn width(
        &self,
        segment: &Segment,
        annotation: &Annotation,
        line: &Range<usize>,
        widths: &[f32],
    ) -> Result<f32, TextError> {
        if annotation.whitespace {
            return self.space_width(&annotation.style);
        }
        let Some(block) = annotation.block else {
            return Ok(0.0);
        };
        let part = self.part(segment, annotation, line);
        if part == self.whole(block) {
            return Ok(widths[self.paragraph.atomics().len() + block]);
        }
        Ok(self.contents(block, part, widths)?.width)
    }

    /// The text of annotation box `block`'s contents, which a line breaking inside its bases
    /// breaks; `None` where it has none in this writing mode.
    fn breakable(&self, block: usize) -> Option<&'p Paragraph> {
        // The first formatted line holds the contents of the box's `::first-line` copy.
        let first_line = (!std::ptr::eq(self.paragraph, self.source))
            .then(|| self.source.first_line_annotation_contents(block))
            .flatten();
        first_line
            .or_else(|| self.source.annotation_contents(block))
            .filter(|contents| !contents.text().is_empty())
    }

    /// The whole of annotation box `block`'s contents: their text, or one unit that a line
    /// holds whole where there is none to break.
    fn whole(&self, block: usize) -> Range<usize> {
        0..self
            .breakable(block)
            .map_or(WHOLE_UNIT.end, |contents| contents.text().len())
    }

    /// The contents of annotation box `block` and where they break, prepared once.
    fn annotation_text(&self, block: usize) -> Option<Rc<AnnotationText<'p, 'c, 'f>>> {
        if let Some(text) = self.annotations.borrow().get(&block) {
            return Some(text.clone());
        }
        let contents = self.breakable(block)?;
        let text = Rc::new(AnnotationText {
            prepared: self.context.prepare_paragraph(contents),
            breaks: contents_breaks(contents),
        });
        self.annotations.borrow_mut().insert(block, text.clone());
        Some(text)
    }

    /// The part of an annotation of `segment` on a line holding the paragraph's part `line`:
    /// where a line breaks strictly inside the bases it spans, its contents break at the break
    /// nearest in the proportion of content before it (§3.4.2), and contents without text go
    /// whole to one line. Inter-character annotations hold no breaks: they lie whole beside
    /// their base.
    fn part(
        &self,
        segment: &Segment,
        annotation: &Annotation,
        line: &Range<usize>,
    ) -> Range<usize> {
        let Some(block) = annotation.block else {
            return 0..0;
        };
        let whole = self.whole(block);
        let inter_character = segment.levels.iter().any(|level| {
            level.inter_character
                && level
                    .annotations
                    .iter()
                    .any(|other| std::ptr::eq(other, annotation))
        });
        if annotation.bases.is_empty() || inter_character {
            return whole;
        }
        let bases = [
            segment.bases[annotation.bases.start],
            segment.bases[annotation.bases.end - 1],
        ];
        let boxes = self.paragraph.boxes();
        let span = boxes[bases[0]].range.start..boxes[bases[1]].range.end;
        let text = self.annotation_text(block);
        let breaks = text.as_ref().map_or(&WHOLE_BREAKS[..], |text| &text.breaks);
        let at = |offset: usize| {
            if offset <= span.start {
                whole.start
            } else if offset >= span.end {
                whole.end
            } else {
                nearest(breaks, self.base_share(bases, offset)).0
            }
        };
        at(line.start)..at(line.end)
    }

    /// The share of the text of the bases from box `bases[0]` to box `bases[1]` before
    /// `offset`, measured in the source paragraph's offsets where its `::first-line` copy and
    /// it encode the text alike, so that a line of either and the next agree.
    fn base_share(&self, [first, last]: [usize; 2], offset: usize) -> f32 {
        let span = |paragraph: &Paragraph, shift: usize| {
            let boxes = paragraph.boxes();
            boxes[first - shift].range.start..boxes[last - shift].range.end
        };
        if !std::ptr::eq(self.paragraph, self.source)
            && let Some(source) = self.source.first_line_offsets().to_paragraph(offset)
        {
            let shift = self.paragraph.boxes().len() - self.source.boxes().len();
            return share(self.source.text(), span(self.source, shift), source);
        }
        share(self.paragraph.text(), span(self.paragraph, 0), offset)
    }

    /// The width, justification opportunities and edge characters of the part `part` of
    /// annotation box `block`'s contents at its max-content width, from the widths of the
    /// paragraph's measured blocks.
    fn contents(
        &self,
        block: usize,
        part: Range<usize>,
        widths: &[f32],
    ) -> Result<Contents, TextError> {
        let width = widths[self.paragraph.atomics().len() + block];
        let nested = &widths[self.source.annotation_metrics(block)];
        let key = (
            block,
            part.clone(),
            std::iter::once(width)
                .chain(nested.iter().copied())
                .map(f32::to_bits)
                .collect(),
        );
        if let Some(contents) = self.contents.borrow().get(&key) {
            return Ok(contents.clone());
        }
        // Contents without text in this writing mode are whole, without opportunities.
        let (Some(paragraph), Some(text)) = (self.breakable(block), self.annotation_text(block))
        else {
            return Ok(Contents {
                width,
                opportunities: 0,
                first: String::new(),
                last: String::new(),
            });
        };
        let lines = text
            .prepared
            .max_content_lines(part, width, nested.to_vec())?;
        let units = |run: &GlyphRun| -> Vec<String> {
            run.glyphs
                .iter()
                .map(|glyph| run.text[glyph.text_range.clone()].to_owned())
                .collect()
        };
        let mut all = lines
            .iter()
            .flat_map(|line| line.runs.iter())
            .flat_map(units);
        let first = all.next().unwrap_or_default();
        let last = all.last().unwrap_or_else(|| first.clone());
        let contents = Contents {
            width: lines.iter().map(|line| line.wrap_width).fold(0.0, f32::max),
            opportunities: lines
                .iter()
                .map(|line| spacing::opportunities(paragraph, &line.runs, &line.order).len())
                .sum(),
            first,
            last,
        };
        self.contents.borrow_mut().insert(key, contents.clone());
        Ok(contents)
    }

    /// The bases of `segment` among `present` boxes, which breaks only between bases leave
    /// contiguous: `[first, last]`.
    fn bases_on_line(bases: &[usize], present: impl Fn(usize) -> bool) -> Option<[usize; 2]> {
        Some([
            bases.iter().position(|&base| present(base))?,
            bases.iter().rposition(|&base| present(base))?,
        ])
    }

    /// How `level` of `segment` places its annotations over the bases `[first, last]` of a
    /// line holding the paragraph's part `line`, merged or not: those holding a part of their
    /// contents there.
    fn placed(
        &self,
        segment: &Segment,
        level: &Level,
        [first, last]: [usize; 2],
        line: &Range<usize>,
        merged: bool,
    ) -> Vec<Placed> {
        let on_line = |annotation: &Annotation| {
            !annotation.hidden
                && if annotation.whitespace {
                    first < annotation.bases.start && annotation.bases.start <= last
                } else {
                    annotation.bases.start <= last
                        && annotation.bases.end > first
                        && annotation.block.is_none_or(|block| {
                            let part = self.part(segment, annotation, line);
                            !part.is_empty() || part == self.whole(block)
                        })
                }
        };
        let mut placed = Vec::new();
        let mut run: Option<Placed> = None;
        for (index, annotation) in level.annotations.iter().enumerate() {
            if !on_line(annotation) {
                if let Some(done) = run.take() {
                    placed.push(done);
                }
                continue;
            }
            if merged {
                let bases = [
                    annotation.bases.start.max(first),
                    annotation
                        .bases
                        .end
                        .max(annotation.bases.start + 1)
                        .min(last + 1)
                        - 1,
                ];
                match &mut run {
                    Some(open) => {
                        open.annotations.end = index + 1;
                        open.bases[1] = open.bases[1].max(bases[1]);
                    }
                    None => {
                        run = Some(Placed {
                            annotations: index..index + 1,
                            bases,
                            merged: true,
                        });
                    }
                }
            } else if !annotation.whitespace {
                placed.push(Placed {
                    annotations: index..index + 1,
                    bases: [
                        annotation.bases.start.max(first),
                        (annotation.bases.end - 1).min(last),
                    ],
                    merged: false,
                });
            }
        }
        placed.extend(run);
        placed
    }

    /// Whether `level` of `segment` merges its annotations on a line holding the paragraph's
    /// part `line`, where the bases `[first, last]` have content widths `bases` and the gaps
    /// before them widths `gaps`: always under `merge`, and under `auto` where some annotation
    /// is wider than its bases (§4.2).
    #[expect(
        clippy::too_many_arguments,
        reason = "one segment's line geometry, which is computed together"
    )]
    fn merges(
        &self,
        segment: &Segment,
        level: &Level,
        bases: &[f32],
        gaps: &[f32],
        span: [usize; 2],
        line: &Range<usize>,
        widths: &[f32],
    ) -> Result<bool, TextError> {
        Ok(match level.style.ruby.merge {
            RubyMerge::Separate => false,
            RubyMerge::Merge => true,
            RubyMerge::Auto => {
                let mut overflows = false;
                for placed in self.placed(segment, level, span, line, false) {
                    let annotation = &level.annotations[placed.annotations.start];
                    let [start, end] = placed.bases.map(|base| base - span[0]);
                    let available: f32 = bases[start..=end].iter().sum::<f32>()
                        + gaps[start + 1..=end].iter().sum::<f32>();
                    overflows |= self.width(segment, annotation, line, widths)? > available;
                }
                overflows
            }
        })
    }

    /// Whether an inter-character annotation of `segment` over its base `base` lies on a line
    /// holding the paragraph's part `line`: with the base's fragment on its right, which
    /// holds the base's end, or its start in a right-to-left container.
    fn holds_inter_character(&self, ruby: &Ruby, base: usize, line: &Range<usize>) -> bool {
        let range = &self.paragraph.boxes()[base].range;
        if ruby_rtl(self.paragraph, ruby) {
            line.start <= range.start
        } else {
            line.end >= range.end
        }
    }

    /// Sizes the ruby columns of a line arranged from the paragraph's part `line`, and spaces
    /// its bases to fill them, before its inline box fragments are formed.
    pub(super) fn space(
        &self,
        arranged: &mut Arranged,
        line: &Range<usize>,
        widths: &[f32],
    ) -> Result<RubyLine, TextError> {
        let paragraph = self.paragraph;
        let mut ruby_line = RubyLine::default();
        for (ruby_index, ruby) in self.rubies() {
            if arranged.present.binary_search(&ruby.container).is_err() {
                continue;
            }
            for (segment_index, segment) in ruby.segments.iter().enumerate() {
                let Some(span) = Self::bases_on_line(&segment.bases, |base| {
                    arranged.present.binary_search(&base).is_ok()
                }) else {
                    continue;
                };
                let [first, last] = span;
                let slots: Vec<Option<[usize; 2]>> = segment.bases[first..=last]
                    .iter()
                    .map(|&base| slot_span(paragraph, arranged, base))
                    .collect();
                let width = |span: Option<[usize; 2]>| {
                    span.map_or(0.0, |[start, end]| {
                        arranged.order[start..=end]
                            .iter()
                            .map(|&slot| slot_width(&arranged.runs, &arranged.atomics, slot))
                            .sum()
                    })
                };
                let bases: Vec<f32> = slots.iter().map(|&span| width(span)).collect();
                // The gap before each base: what lies between it and the previous base.
                let mut gaps = vec![0.0; bases.len()];
                for index in 1..slots.len() {
                    if let (Some([_, end]), Some([start, _])) = (slots[index - 1], slots[index]) {
                        let (from, to) = if end < start {
                            (end + 1, start)
                        } else {
                            (start, end)
                        };
                        if to > from {
                            gaps[index] = width(Some([from, to - 1]));
                        }
                    }
                }
                // Cells alternate base columns and the gaps before the next ones.
                let mut cells = Vec::with_capacity(bases.len() * 2);
                for (index, base) in bases.iter().enumerate() {
                    if index > 0 {
                        cells.push(gaps[index]);
                    }
                    cells.push(*base);
                }
                let natural = cells.clone();
                let mut spanning = Vec::new();
                for (level_index, level) in segment.levels.iter().enumerate() {
                    if level.inter_character {
                        continue;
                    }
                    let merged = self.merges(segment, level, &bases, &gaps, span, line, widths)?;
                    if merged && level.style.ruby.merge == RubyMerge::Auto {
                        ruby_line
                            .merged
                            .push([ruby_index, segment_index, level_index]);
                    }
                    for placed in self.placed(segment, level, span, line, merged) {
                        let mut total = 0.0;
                        for annotation in &level.annotations[placed.annotations.clone()] {
                            total += self.width(segment, annotation, line, widths)?;
                        }
                        let [start, end] = placed.bases.map(|base| 2 * (base - first));
                        if start == end && !placed.merged {
                            cells[start] = cells[start].max(total);
                        } else {
                            spanning.push((placed.bases[1] - placed.bases[0], start, end, total));
                        }
                    }
                    // Inter-annotation white space widens the gap it pairs with.
                    if !merged {
                        for annotation in &level.annotations {
                            if annotation.whitespace
                                && !annotation.hidden
                                && first < annotation.bases.start
                                && annotation.bases.start <= last
                            {
                                let cell = 2 * (annotation.bases.start - first) - 1;
                                cells[cell] =
                                    cells[cell].max(self.width(segment, annotation, line, widths)?);
                            }
                        }
                    }
                }
                // Spanning annotations widen their columns equally, fewest bases first.
                spanning.sort_by_key(|&(count, ..)| count);
                for (_, start, end, total) in spanning {
                    let covered: f32 = cells[start..=end].iter().sum();
                    if total > covered {
                        let columns = (end - start) / 2 + 1;
                        let share = (total - covered) / columns as f32;
                        for cell in (start..=end).step_by(2) {
                            cells[cell] += share;
                        }
                    }
                }
                self.distribute(
                    arranged,
                    segment,
                    span,
                    &slots,
                    &cells,
                    &natural,
                    &mut ruby_line,
                )?;
                let mut reservations = Vec::new();
                for level in segment.levels.iter().filter(|level| level.inter_character) {
                    for annotation in level
                        .annotations
                        .iter()
                        .filter(|annotation| !annotation.hidden && !annotation.whitespace)
                    {
                        if annotation.bases.is_empty() {
                            continue;
                        }
                        let base_index = if ruby_rtl(paragraph, ruby) {
                            annotation.bases.start
                        } else {
                            annotation.bases.end - 1
                        };
                        if base_index < first || base_index > last {
                            continue;
                        }
                        let base = segment.bases[base_index];
                        if !self.holds_inter_character(ruby, base, line) {
                            continue;
                        }
                        let at = slot_span(paragraph, arranged, base).map_or_else(
                            || arranged.insertion(paragraph.boxes()[base].range.end),
                            |[_, end]| end + 1,
                        );
                        reservations.push((at, self.width(segment, annotation, line, widths)?));
                    }
                }
                reservations.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
                for (at, width) in reservations {
                    arranged.order.insert(
                        at,
                        Slot::Space {
                            owner: Some(ruby.container),
                            width,
                        },
                    );
                }
            }
        }
        self.overhang(arranged, &mut ruby_line, line, widths)?;
        Ok(ruby_line)
    }

    /// Spaces each base of the line's part of a segment, and each gap, to fill its cell.
    #[expect(
        clippy::too_many_arguments,
        reason = "one segment's line geometry, which is computed together"
    )]
    fn distribute(
        &self,
        arranged: &mut Arranged,
        segment: &Segment,
        [first, _]: [usize; 2],
        slots: &[Option<[usize; 2]>],
        cells: &[f32],
        natural: &[f32],
        ruby_line: &mut RubyLine,
    ) -> Result<(), TextError> {
        let paragraph = self.paragraph;
        let container = self.container_of(segment.bases[first]);
        // Each insertion's position, its rank among those at one position (the previous
        // base's trailing space, the gap's spacing, then the base's leading space), its
        // owner and its width.
        let mut insertions: Vec<(usize, usize, usize, f32)> = Vec::new();
        for (index, span) in slots.iter().enumerate() {
            let base = segment.bases[first + index];
            let cell = 2 * index;
            let extra = cells[cell] - natural[cell];
            let at = span.map_or_else(
                || arranged.insertion(paragraph.boxes()[base].range.start),
                |[start, _]| start,
            );
            if index > 0 {
                let gap = cells[cell - 1] - natural[cell - 1];
                if gap > 0.0 {
                    insertions.push((at, GAP_RANK, container, gap));
                }
            }
            let &Some([_, end]) = span else {
                // An empty base's column is its own spacing.
                if extra > 0.0 {
                    insertions.push((at, LEADING_RANK, base, extra));
                }
                ruby_line.columns.push(ColumnSpacing {
                    base,
                    leading: 0.0,
                    trailing: 0.0,
                    overhang: [0.0; 2],
                });
                continue;
            };
            let (leading, trailing) = if extra > 0.0 {
                let style = &paragraph.boxes()[base].style;
                let content = arranged.order[at..=end].to_vec();
                let opportunities = match style.ruby.align {
                    RubyAlign::SpaceBetween | RubyAlign::SpaceAround => {
                        spacing::opportunities(paragraph, &arranged.runs, &content)
                    }
                    RubyAlign::Start | RubyAlign::Center => Vec::new(),
                };
                let (leading, trailing, each) =
                    distribution(style.ruby.align, extra, opportunities.len());
                spacing::expand(&mut arranged.runs, &opportunities, |_| each)?;
                (leading, trailing)
            } else {
                (0.0, 0.0)
            };
            let (leading, trailing) = if paragraph.boxes()[base].style.used_rtl() {
                (trailing, leading)
            } else {
                (leading, trailing)
            };
            if leading > 0.0 {
                insertions.push((at, LEADING_RANK, container, leading));
            }
            if trailing > 0.0 {
                insertions.push((end + 1, TRAILING_RANK, container, trailing));
            }
            ruby_line.columns.push(ColumnSpacing {
                base,
                leading,
                trailing,
                overhang: [0.0; 2],
            });
        }
        // Later positions first keep earlier ones in place; at one position, what goes
        // rightmost goes first.
        insertions.sort_by_key(|&(at, rank, ..)| std::cmp::Reverse((at, rank)));
        for (at, _, owner, width) in insertions {
            arranged.order.insert(
                at,
                Slot::Space {
                    owner: Some(owner),
                    width,
                },
            );
        }
        Ok(())
    }

    /// The ruby container whose segment holds base `base`.
    fn container_of(&self, base: usize) -> usize {
        self.paragraph
            .rubies()
            .iter()
            .find(|ruby| {
                ruby.segments
                    .iter()
                    .any(|segment| segment.bases.contains(&base))
            })
            .expect("a ruby base belongs to a ruby container")
            .container
    }

    /// Lets annotations wider than their bases overhang kana adjacent to their ruby
    /// container (§5.1): by up to half an ideograph in the annotation's font, no further than
    /// the adjacent character, and only where every level over that edge allows it.
    fn overhang(
        &self,
        arranged: &mut Arranged,
        ruby_line: &mut RubyLine,
        line: &Range<usize>,
        widths: &[f32],
    ) -> Result<(), TextError> {
        let paragraph = self.paragraph;
        let rubies: Vec<&Ruby> = paragraph
            .rubies()
            .iter()
            .filter(|ruby| {
                !ruby
                    .segments
                    .iter()
                    .any(|segment| segment.levels.iter().any(|level| level.inter_character))
            })
            .collect();
        for ruby in rubies {
            let Some([start, end]) = slot_span(paragraph, arranged, ruby.container) else {
                continue;
            };
            let in_ruby = |owner: Option<usize>| {
                paragraph
                    .rubies()
                    .iter()
                    .any(|other| paragraph.is_within(owner, other.container))
            };
            for (side, (edge, neighbour)) in [(start, start.checked_sub(1)), (end, Some(end + 1))]
                .into_iter()
                .enumerate()
            {
                let Some(neighbour) = neighbour.filter(|&at| at < arranged.order.len()) else {
                    continue;
                };
                let Slot::Run(run) = arranged.order[neighbour] else {
                    continue;
                };
                let Slot::Space { width: space, .. } = arranged.order[edge] else {
                    continue;
                };
                let run = &arranged.runs[run];
                if run.leader || in_ruby(paragraph.spans()[run.source_span].inline_box) {
                    continue;
                }
                // The adjacent unit is the run's last in visual order before the container,
                // or its first after it.
                let glyph = if side == 0 {
                    run.glyphs.last()
                } else {
                    run.glyphs.first()
                };
                let Some(glyph) = glyph else {
                    continue;
                };
                if !kana(&run.text[glyph.text_range.clone()]) {
                    continue;
                }
                let base = Self::edge_base(ruby, side == 0, arranged);
                let Some(column) = base.and_then(|base| {
                    ruby_line
                        .columns
                        .iter()
                        .position(|column| column.base == base)
                }) else {
                    continue;
                };
                let Some(limit) =
                    self.overhang_limit(ruby, ruby_line.columns[column].base, line, widths)?
                else {
                    continue;
                };
                let amount = space.min(limit).min(glyph.advance(run.font_size));
                if amount <= 0.0 {
                    continue;
                }
                if let Slot::Space { width, .. } = &mut arranged.order[edge] {
                    *width -= amount;
                }
                let spacing = &mut ruby_line.columns[column];
                if side == 0 {
                    spacing.leading -= amount;
                } else {
                    spacing.trailing -= amount;
                }
                spacing.overhang[side] = amount;
            }
        }
        Ok(())
    }

    /// The base at the start or end edge of `ruby` on the line, whose leading or trailing
    /// space is that edge's first or last slot.
    fn edge_base(ruby: &Ruby, start: bool, arranged: &Arranged) -> Option<usize> {
        let present = |base: &&usize| arranged.present.binary_search(base).is_ok();
        let mut bases = ruby.segments.iter().flat_map(|segment| &segment.bases);
        if start {
            bases.find(present).copied()
        } else {
            bases.rfind(present).copied()
        }
    }

    /// How far annotations over `base` may overhang on a line holding the paragraph's part
    /// `line`: half an ideograph in the smallest of their fonts; `None` where a level over it
    /// forbids overhang.
    fn overhang_limit(
        &self,
        ruby: &Ruby,
        base: usize,
        line: &Range<usize>,
        widths: &[f32],
    ) -> Result<Option<f32>, TextError> {
        let mut limit: Option<f32> = None;
        for segment in &ruby.segments {
            let Some(index) = segment
                .bases
                .iter()
                .position(|&candidate| candidate == base)
            else {
                continue;
            };
            for level in &segment.levels {
                for annotation in &level.annotations {
                    if annotation.hidden
                        || annotation.whitespace
                        || !annotation.bases.contains(&index)
                        || self.width(segment, annotation, line, widths)? == 0.0
                    {
                        continue;
                    }
                    if !level.style.ruby.overhang {
                        return Ok(None);
                    }
                    let ideograph =
                        autospace::advance(self.context, &annotation.style, WATER_IDEOGRAPH)?
                            .unwrap_or(FALLBACK_IDEOGRAPH_ADVANCE_EM * annotation.style.size);
                    let own = MAX_OVERHANG_IC * ideograph;
                    limit = Some(limit.map_or(own, |limit: f32| limit.min(own)));
                }
            }
        }
        Ok(limit)
    }

    /// Places the annotation boxes of a positioned and vertically aligned line, whose content
    /// at `basis` percentages resolve against, from the measured `metrics` of the paragraph's
    /// atomic inlines followed by its annotation boxes.
    pub(super) fn place(
        &self,
        line: &mut Line,
        metrics: &[super::AtomicMetrics],
        basis: f32,
    ) -> Result<(), TextError> {
        let paragraph = self.paragraph;
        let widths: Vec<f32> = metrics.iter().map(|metrics| metrics.width).collect();
        let range = line.range.clone();
        let fragments: HashMap<usize, Vec<&BoxFragment>> =
            line.boxes.iter().fold(HashMap::new(), |mut map, fragment| {
                map.entry(fragment.index)
                    .or_insert_with(Vec::new)
                    .push(fragment);
                map
            });
        // Each ruby's extent with its annotations, which an outer base container contains.
        let mut extents: HashMap<usize, [f32; 2]> = HashMap::new();
        let mut placed = Vec::new();
        for (ruby_index, ruby) in self.rubies() {
            let mut whole: Option<[f32; 2]> = None;
            for (segment_index, segment) in ruby.segments.iter().enumerate() {
                let Some(span) = Self::bases_on_line(&segment.bases, |base| {
                    line.present.binary_search(&base).is_ok()
                }) else {
                    continue;
                };
                let [first, last] = span;
                let mut columns: Vec<Option<[f32; 2]>> = Vec::with_capacity(last - first + 1);
                let mut container = [f32::INFINITY, f32::NEG_INFINITY];
                for &base in &segment.bases[first..=last] {
                    let Some(pieces) = fragments.get(&base) else {
                        columns.push(None);
                        continue;
                    };
                    let style = &paragraph.boxes()[base].style;
                    // A base broken across lines has its margins only where it starts and ends.
                    let margin = |side: usize| {
                        let edge = usize::from(side == RIGHT);
                        if pieces.iter().any(|piece| piece.edges[edge]) {
                            style.margin[side].resolve(basis).unwrap_or(0.0)
                        } else {
                            0.0
                        }
                    };
                    let left = pieces
                        .iter()
                        .map(|piece| piece.x)
                        .fold(f32::INFINITY, f32::min);
                    let right = pieces
                        .iter()
                        .map(|piece| piece.x + piece.width)
                        .fold(f32::NEG_INFINITY, f32::max);
                    let spacing = line.ruby.columns.iter().find(|column| column.base == base);
                    let [leading, trailing, before, after] = spacing.map_or([0.0; 4], |spacing| {
                        [
                            spacing.leading,
                            spacing.trailing,
                            spacing.overhang[0],
                            spacing.overhang[1],
                        ]
                    });
                    columns.push(Some([
                        left - margin(LEFT) - leading - before,
                        right + margin(RIGHT) + trailing + after,
                    ]));
                    let [top, bottom] = block_edges(style, basis);
                    for piece in pieces {
                        container[0] = container[0].min(piece.baseline - piece.ascent - top);
                        container[1] = container[1].max(piece.baseline + piece.descent + bottom);
                    }
                }
                for (&inner, extent) in &extents {
                    if segment.bases[first..=last]
                        .iter()
                        .any(|&base| paragraph.is_within(Some(inner), base))
                    {
                        container[0] = container[0].min(extent[0]);
                        container[1] = container[1].max(extent[1]);
                    }
                }
                let columns = collapsed_columns(&columns, ruby_rtl(paragraph, ruby));
                if !container[0].is_finite() {
                    let [above, below] = margin_extent(
                        self.context,
                        &paragraph.boxes()[ruby.container].style,
                        basis,
                    )?;
                    container = [-above, below];
                }
                let mut over = container[0];
                let mut under = container[1];
                let mut inter_offsets = HashMap::<usize, f32>::new();
                for (level_index, level) in segment.levels.iter().enumerate() {
                    if level.inter_character {
                        for annotation in level
                            .annotations
                            .iter()
                            .filter(|annotation| !annotation.hidden && !annotation.whitespace)
                        {
                            let Some(block) = annotation.block else {
                                continue;
                            };
                            let base_index = if ruby_rtl(paragraph, ruby) {
                                annotation.bases.start
                            } else {
                                annotation.bases.end - 1
                            };
                            if base_index < first || base_index > last {
                                continue;
                            }
                            let base = segment.bases[base_index];
                            if !self.holds_inter_character(ruby, base, &range) {
                                continue;
                            }
                            let Some(pieces) = fragments.get(&base) else {
                                continue;
                            };
                            let top = pieces
                                .iter()
                                .map(|piece| piece.baseline - piece.ascent)
                                .fold(f32::INFINITY, f32::min);
                            let bottom = pieces
                                .iter()
                                .map(|piece| piece.baseline + piece.descent)
                                .fold(f32::NEG_INFINITY, f32::max);
                            let right = pieces
                                .iter()
                                .map(|piece| piece.x + piece.width)
                                .fold(f32::NEG_INFINITY, f32::max)
                                + paragraph.boxes()[base].style.margin[RIGHT]
                                    .resolve(basis)
                                    .unwrap_or(0.0);
                            let index = paragraph.atomics().len() + block;
                            let metric = metrics[index];
                            let y = if annotation.style.ruby.align == RubyAlign::Start {
                                top
                            } else {
                                f32::midpoint(top, bottom) - metric.height * HALF
                            };
                            let offset = inter_offsets.entry(base).or_default();
                            placed.push(AtomicFragment {
                                index,
                                x: right + *offset,
                                width: metric.width,
                                baseline: y + metric.baseline,
                                offset: paragraph.annotations()[block].offset,
                                owner: Some(ruby.container),
                                rtl: false,
                                part: None,
                            });
                            *offset += metric.width;
                            over = over.min(y);
                            under = under.max(y + metric.height);
                        }
                        continue;
                    }
                    let merged = level.style.ruby.merge == RubyMerge::Merge
                        || line
                            .ruby
                            .merged
                            .contains(&[ruby_index, segment_index, level_index]);
                    let placements = self.placed(segment, level, span, &range, merged);
                    if placements.is_empty() {
                        continue;
                    }
                    let mut ascent: f32 = 0.0;
                    let mut descent: f32 = 0.0;
                    for annotation in placements
                        .iter()
                        .flat_map(|placement| &level.annotations[placement.annotations.clone()])
                        .filter(|annotation| !annotation.whitespace)
                    {
                        let [above, below] = margin_extent(self.context, &annotation.style, basis)?;
                        ascent = ascent.max(above);
                        descent = descent.max(below);
                    }
                    let baseline = if level.under {
                        let baseline = under + ascent;
                        under = baseline + descent;
                        baseline
                    } else {
                        let baseline = over - descent;
                        over = baseline - ascent;
                        baseline
                    };
                    for placement in placements {
                        let [start, end] = placement.bases.map(|base| base - first);
                        let extent = columns[start..=end].iter().fold(
                            [f32::INFINITY, f32::NEG_INFINITY],
                            |[left, right], column| [left.min(column[0]), right.max(column[1])],
                        );
                        placed.extend(self.position(
                            (segment, level, &placement),
                            extent,
                            baseline,
                            (ruby_index, &range),
                            &widths,
                        )?);
                    }
                }
                whole = Some(whole.map_or([over, under], |[top, bottom]| {
                    [top.min(over), bottom.max(under)]
                }));
            }
            if let Some(whole) = whole {
                extents.insert(ruby.container, whole);
            }
        }
        line.annotations = placed;
        Ok(())
    }

    /// The annotation fragments of `placement` of `level` of `segment` over its columns'
    /// `[left, right]` extent, on its level's `baseline`, on a line of ruby `ruby` holding the
    /// paragraph's part `line`.
    fn position(
        &self,
        (segment, level, placement): (&Segment, &Level, &Placed),
        [left, right]: [f32; 2],
        baseline: f32,
        (ruby, line): (usize, &Range<usize>),
        widths: &[f32],
    ) -> Result<Vec<AtomicFragment>, TextError> {
        let annotations = &level.annotations[placement.annotations.clone()];
        let mut sizes = Vec::with_capacity(annotations.len());
        let mut parts = Vec::with_capacity(annotations.len());
        let mut total = 0.0;
        for annotation in annotations {
            let width = self.width(segment, annotation, line, widths)?;
            let part = self.part(segment, annotation, line);
            let contents = match annotation.block {
                Some(block) => Some(self.contents(block, part.clone(), widths)?),
                None => None,
            };
            total += width;
            sizes.push((width, contents));
            parts.push(part);
        }
        let extra = right - left - total;
        // A merged run distributes by its container's `ruby-align`, a lone annotation by its
        // own (§4.3).
        let (align, justify) = if placement.merged {
            (level.style.ruby.align, level.style.line_breaking.justify)
        } else {
            let style = &annotations[0].style;
            (style.ruby.align, style.line_breaking.justify)
        };
        // Opportunities within each annotation, at white space between them, and at the
        // junction of adjacent ones where their nearest common box justifies it.
        let mut junctions = vec![false; annotations.len()];
        let mut count = 0;
        for (index, (annotation, (_, contents))) in annotations.iter().zip(&sizes).enumerate() {
            if annotation.whitespace {
                count += 1;
                continue;
            }
            count += contents
                .as_ref()
                .map_or(0, |contents| contents.opportunities);
            if index > 0
                && let Some(contents) = contents
                && let Some(previous) = sizes[index - 1].1.as_ref()
                && !annotations[index - 1].whitespace
                && spacing::justifies_between(justify, &previous.last, &contents.first)
            {
                junctions[index] = true;
                count += 1;
            }
        }
        let (start, _, each) = if extra > 0.0 {
            distribution(align, extra, count)
        } else {
            (extra * HALF, extra * HALF, 0.0)
        };
        // Annotations follow their base container's direction from its start edge (§3.5).
        let rtl = ruby_rtl(self.paragraph, &self.paragraph.rubies()[ruby]);
        let mut pen = start;
        let mut fragments = Vec::new();
        for (((annotation, (width, contents)), junction), part) in
            annotations.iter().zip(sizes).zip(junctions).zip(parts)
        {
            if junction {
                pen += each;
            }
            let opportunities = if annotation.whitespace {
                1
            } else {
                contents.map_or(0, |contents| contents.opportunities)
            };
            let used = width + each * opportunities as f32;
            if let Some(block) = annotation.block {
                fragments.push(AtomicFragment {
                    index: self.paragraph.atomics().len() + block,
                    x: if rtl { right - pen - used } else { left + pen },
                    width: used,
                    baseline,
                    offset: self.paragraph.annotations()[block].offset,
                    owner: Some(self.paragraph.rubies()[ruby].container),
                    rtl,
                    part: (part != self.whole(block)).then_some(part),
                });
            }
            pen += used;
        }
        Ok(fragments)
    }

    /// The extra leading a ruby container `container` on a line holding the paragraph's part
    /// `line`, whose present inline boxes are `present`, needs over and under its base
    /// container so that identical lines would not overlap (§3.6). `container_metrics` are
    /// the container's own.
    pub(super) fn leading(
        &self,
        container: usize,
        (present, line): (&[usize], &Range<usize>),
        container_metrics: &BoxMetrics,
        (metrics, basis): (&[super::AtomicMetrics], f32),
        stacks: &mut RubyStacks,
    ) -> Result<[f32; 2], TextError> {
        let Some(index) = self
            .paragraph
            .rubies()
            .iter()
            .position(|ruby| ruby.container == container)
        else {
            return Ok([0.0; 2]);
        };
        let ruby = &self.paragraph.rubies()[index];
        let ([above, below], heights) =
            self.stack(index, (present, line), (metrics, basis), stacks)?;
        if ruby
            .segments
            .iter()
            .any(|segment| segment.levels.iter().any(|level| level.inter_character))
        {
            let extent = container_metrics.extent();
            return Ok([
                (above + heights[0] + extent[0]).max(0.0),
                (below + heights[1] - extent[1]).max(0.0),
            ]);
        }
        let extra = heights[0] + above + below + heights[1] - container_metrics.line_height();
        let annotated = heights[0] + heights[1];
        if extra <= 0.0 || annotated <= 0.0 {
            return Ok([0.0; 2]);
        }
        Ok(heights.map(|height| extra * height / annotated))
    }

    /// How far `ruby`'s base container extends `[above, below]` its baseline on a line whose
    /// present inline boxes are `present`, holding any ruby nested in its bases with their
    /// annotations, and the heights of its annotation levels over and under it.
    /// Each ruby's stack is computed once per line in `stacks`, as a ruby nested in a base
    /// lies within the bases of every ruby around it.
    fn stack(
        &self,
        index: usize,
        (present, line): (&[usize], &Range<usize>),
        (metrics, basis): (&[super::AtomicMetrics], f32),
        stacks: &mut RubyStacks,
    ) -> Result<RubyStack, TextError> {
        if let Some(stack) = stacks.get(&index) {
            return Ok(*stack);
        }
        let paragraph = self.paragraph;
        let ruby = &paragraph.rubies()[index];
        let mut heights = [0.0f32; 2];
        let mut content = [0.0f32; 2];
        for segment in &ruby.segments {
            let Some(span) =
                Self::bases_on_line(&segment.bases, |base| present.binary_search(&base).is_ok())
            else {
                continue;
            };
            for &base in &segment.bases[span[0]..=span[1]] {
                let [above, below] =
                    margin_extent(self.context, &paragraph.boxes()[base].style, basis)?;
                content = [content[0].max(above), content[1].max(below)];
                for nested in (0..paragraph.rubies().len()).filter(|&nested| {
                    paragraph.is_within(Some(paragraph.rubies()[nested].container), base)
                }) {
                    let ([above, below], [over, under]) =
                        self.stack(nested, (present, line), (metrics, basis), stacks)?;
                    content = [content[0].max(above + over), content[1].max(below + under)];
                }
            }
            let mut sides = [0.0f32; 2];
            let base_content = content;
            for level in &segment.levels {
                if level.inter_character {
                    for annotation in level
                        .annotations
                        .iter()
                        .filter(|annotation| !annotation.hidden && !annotation.whitespace)
                    {
                        let Some(block) = annotation.block else {
                            continue;
                        };
                        let base_index = if ruby_rtl(paragraph, ruby) {
                            annotation.bases.start
                        } else {
                            annotation.bases.end - 1
                        };
                        if base_index < span[0]
                            || base_index > span[1]
                            || !self.holds_inter_character(ruby, segment.bases[base_index], line)
                        {
                            continue;
                        }
                        let height = metrics[paragraph.atomics().len() + block].height;
                        let top = if annotation.style.ruby.align == RubyAlign::Start {
                            -base_content[0]
                        } else {
                            (base_content[1] - base_content[0] - height) * HALF
                        };
                        content[0] = content[0].max(-top);
                        content[1] = content[1].max(top + height);
                    }
                    continue;
                }
                let mut extent = [0.0f32; 2];
                let mut any = false;
                for placement in self.placed(segment, level, span, line, false) {
                    for annotation in &level.annotations[placement.annotations] {
                        if annotation.whitespace {
                            continue;
                        }
                        any = true;
                        let [above, below] = margin_extent(self.context, &annotation.style, basis)?;
                        extent = [extent[0].max(above), extent[1].max(below)];
                    }
                }
                if any {
                    sides[usize::from(level.under)] += extent[0] + extent[1];
                }
            }
            heights = [heights[0].max(sides[0]), heights[1].max(sides[1])];
        }
        stacks.insert(index, (content, heights));
        Ok((content, heights))
    }
}

/// How far a ruby's base container extends `[above, below]` its baseline, with the rubies
/// nested in its bases, and the heights of its annotation levels `[over, under]` it.
type RubyStack = ([f32; 2], [f32; 2]);
/// The stacks of a line's rubies by their index in the paragraph.
pub(super) type RubyStacks = HashMap<usize, RubyStack>;

/// Whether `ruby`'s base container, which shares its container's direction, runs right to
/// left.
fn ruby_rtl(paragraph: &Paragraph, ruby: &Ruby) -> bool {
    paragraph.boxes()[ruby.container].style.used_rtl()
}

/// The `[left, right]` extents of a line's columns, where a base without a fragment on the
/// line takes an empty column at the end of the one before it in the direction `rtl` gives,
/// or at the start of the one after it.
fn collapsed_columns(columns: &[Option<[f32; 2]>], rtl: bool) -> Vec<[f32; 2]> {
    let edge = |column: [f32; 2], end: bool| {
        let x = if end == rtl { column[0] } else { column[1] };
        [x, x]
    };
    let mut filled: Vec<Option<[f32; 2]>> = columns.to_vec();
    for index in 1..filled.len() {
        if filled[index].is_none() {
            filled[index] = filled[index - 1].map(|column| edge(column, true));
        }
    }
    for index in (0..filled.len().saturating_sub(1)).rev() {
        if filled[index].is_none() {
            filled[index] = filled[index + 1].map(|column| edge(column, false));
        }
    }
    filled.into_iter().map(Option::unwrap_or_default).collect()
}

/// How `ruby-align` spreads `extra` space around content with `count` justification
/// opportunities: the space before and after it, and what each opportunity takes (§4.3).
/// Without opportunities, the justifying values centre the content.
pub(super) fn distribution(align: RubyAlign, extra: f32, count: usize) -> (f32, f32, f32) {
    match align {
        RubyAlign::Start => (0.0, extra, 0.0),
        RubyAlign::SpaceBetween if count > 0 => (0.0, 0.0, extra / count as f32),
        RubyAlign::SpaceAround if count > 0 => {
            let each = extra / (count + 1) as f32;
            (each * HALF, each * HALF, each)
        }
        RubyAlign::Center | RubyAlign::SpaceBetween | RubyAlign::SpaceAround => {
            (extra * HALF, extra * HALF, 0.0)
        }
    }
}

/// Applies ruby's line breaking rules (§3.4) to the paragraph's soft wrap opportunities:
/// between adjacent bases, the line breaking rules of the base text apply as between inline
/// boxes, so that the ruby container, their nearest common ancestor, decides whether they wrap
/// (CSS Text 3 §5.1); between the bases an annotation spans there is none, nor inside a base,
/// except where `joints` lets a line break it with its annotations.
pub(super) fn constrain(paragraph: &Paragraph, breaks: &mut Vec<(usize, BreakOpportunity)>) {
    if paragraph.rubies().is_empty() {
        return;
    }
    let joints = joints(paragraph, breaks);
    let linebreaks = crate::text::tailoring::linebreaks(paragraph);
    for ruby in paragraph.rubies() {
        if !paragraph.boxes()[ruby.container].style.white_space.wraps() {
            continue;
        }
        let bases: Vec<usize> = ruby
            .segments
            .iter()
            .flat_map(|segment| segment.bases.iter().copied())
            .collect();
        for pair in bases.windows(2) {
            let [before, after] = [pair[0], pair[1]].map(|base| &paragraph.boxes()[base].range);
            breaks.extend(
                linebreaks
                    .iter()
                    .filter(|&&(offset, opportunity)| {
                        opportunity == BreakOpportunity::Allowed
                            && before.end <= offset
                            && offset <= after.start
                    })
                    .copied(),
            );
        }
    }
    breaks.sort_unstable_by_key(|&(offset, opportunity)| {
        (offset, opportunity == BreakOpportunity::Allowed)
    });
    breaks.dedup_by_key(|entry| entry.0);
    let enclosed = enclosed(paragraph);
    breaks.retain(|&(offset, opportunity)| {
        opportunity == BreakOpportunity::Mandatory
            || joints.binary_search(&offset).is_ok()
            || !encloses(&enclosed, offset)
    });
}

/// Removes the emergency wrap opportunities inside ruby bases and the bases annotations span,
/// where only the soft wrap opportunities that `joints` chooses break lines (§3.4).
pub(super) fn constrain_emergency(paragraph: &Paragraph, offsets: &mut Vec<usize>) {
    if paragraph.rubies().is_empty() {
        return;
    }
    let enclosed = enclosed(paragraph);
    offsets.retain(|&offset| !encloses(&enclosed, offset));
}

/// The parts of the paragraph that a line breaks only with ruby annotations: each base, and
/// the bases each spanning annotation spans.
fn enclosed(paragraph: &Paragraph) -> Vec<Range<usize>> {
    let range = |base: usize| paragraph.boxes()[base].range.clone();
    let mut enclosed = Vec::new();
    for segment in paragraph.rubies().iter().flat_map(|ruby| &ruby.segments) {
        enclosed.extend(segment.bases.iter().map(|&base| range(base)));
        for annotation in segment.levels.iter().flat_map(|level| &level.annotations) {
            if annotation.hidden || annotation.whitespace || annotation.bases.len() < 2 {
                continue;
            }
            enclosed.push(
                range(segment.bases[annotation.bases.start]).start
                    ..range(segment.bases[annotation.bases.end - 1]).end,
            );
        }
    }
    enclosed
}

/// Whether `offset` lies strictly inside one of the `enclosed` parts.
fn encloses(enclosed: &[Range<usize>], offset: usize) -> bool {
    enclosed
        .iter()
        .any(|range| range.start < offset && offset < range.end)
}

/// One of a ruby base's break opportunities: where it lies, where each annotation paired
/// with the base breaks there, how far those breaks lie from it in proportion, and whether it
/// is forced.
struct BaseBreak {
    offset: usize,
    splits: Vec<usize>,
    distance: f32,
    forced: bool,
}

/// The soft wrap opportunities of `breaks` inside ruby bases where a line may break a base
/// with its annotations (§3.4.2). The base and every annotation paired with it alone must
/// allow wrapping, and each annotation must hold a break opportunity; there each annotation
/// breaks at its break nearest in the proportion of content before it. Of the base's
/// opportunities that break the annotations alike, the one nearest in proportion is chosen;
/// nearest first, each is kept only where every annotation breaks strictly inside its
/// contents, after where it breaks at the kept and forced breaks before and before where it
/// does at those after, so that no annotation fragment is empty.
fn joints(paragraph: &Paragraph, breaks: &[(usize, BreakOpportunity)]) -> Vec<usize> {
    let text = paragraph.text();
    let mut joints = Vec::new();
    for segment in paragraph.rubies().iter().flat_map(|ruby| &ruby.segments) {
        for (index, &base) in segment.bases.iter().enumerate() {
            let Some(annotations) = parallel(paragraph, segment, index) else {
                continue;
            };
            let range = paragraph.boxes()[base].range.clone();
            let mut events: Vec<BaseBreak> = Vec::new();
            for &(offset, opportunity) in breaks
                .iter()
                .filter(|&&(offset, _)| range.start < offset && offset < range.end)
            {
                let share = share(text, range.clone(), offset);
                let nearest: Vec<(usize, f32)> = annotations
                    .iter()
                    .map(|breaks| nearest(breaks, share))
                    .collect();
                let event = BaseBreak {
                    offset,
                    splits: nearest.iter().map(|&(split, _)| split).collect(),
                    distance: nearest.iter().map(|&(_, at)| (at - share).abs()).sum(),
                    forced: opportunity == BreakOpportunity::Mandatory,
                };
                match events.last_mut() {
                    Some(last)
                        if !event.forced
                            && !last.forced
                            && !event.splits.is_empty()
                            && last.splits == event.splits =>
                    {
                        if event.distance < last.distance {
                            *last = event;
                        }
                    }
                    _ => events.push(event),
                }
            }
            let ends: Vec<usize> = annotations
                .iter()
                .map(|breaks| breaks.last().map_or(0, |&(end, _)| end))
                .collect();
            let mut chosen: Vec<&BaseBreak> = events.iter().filter(|event| event.forced).collect();
            let mut soft: Vec<&BaseBreak> = events.iter().filter(|event| !event.forced).collect();
            soft.sort_by(|a, b| a.distance.total_cmp(&b.distance));
            for event in soft {
                let inside = event
                    .splits
                    .iter()
                    .zip(&ends)
                    .all(|(&split, &end)| 0 < split && split < end);
                let ordered = chosen.iter().all(|other| {
                    let (before, after) = if other.offset < event.offset {
                        (&other.splits, &event.splits)
                    } else {
                        (&event.splits, &other.splits)
                    };
                    before
                        .iter()
                        .zip(after)
                        .all(|(before, after)| before < after)
                });
                if inside && ordered {
                    chosen.push(event);
                    joints.push(event.offset);
                }
            }
        }
    }
    joints.sort_unstable();
    joints
}

/// The breaks of the contents of each annotation paired with base `index` of `segment`
/// alone, where the base and they all allow wrapping and each holds a break opportunity, so
/// that a line may break inside the base (§3.4.2); inter-character annotations hold none.
fn parallel(
    paragraph: &Paragraph,
    segment: &Segment,
    index: usize,
) -> Option<Vec<Vec<(usize, f32)>>> {
    if !paragraph.boxes()[segment.bases[index]]
        .style
        .white_space
        .wraps()
    {
        return None;
    }
    segment
        .levels
        .iter()
        .map(|level| {
            let annotation = level.annotations.iter().find(|annotation| {
                !annotation.whitespace && annotation.bases == (index..index + 1)
            })?;
            let contents = paragraph.annotation_contents(annotation.block?)?;
            if level.inter_character || !annotation.style.white_space.wraps() {
                return None;
            }
            Some(contents_breaks(contents)).filter(|breaks| breaks.len() > WHOLE_BREAKS.len())
        })
        .collect()
}

/// Where an annotation box's contents can break: their start, each break opportunity inside
/// them, and their end, with the share of their text before each.
fn contents_breaks(contents: &Paragraph) -> Vec<(usize, f32)> {
    let text = contents.text();
    let whole = 0..text.len();
    let mut breaks = vec![(0, 0.0)];
    breaks.extend(
        super::whitespace::opportunities(contents, &hyphenation::automatic(contents))
            .into_iter()
            .filter(|&(offset, _)| 0 < offset && offset < text.len())
            .map(|(offset, _)| (offset, share(text, whole.clone(), offset))),
    );
    breaks.push((text.len(), 1.0));
    breaks
}

/// The share of the characters of the part `range` of `text` that lie before `offset`.
fn share(text: &str, range: Range<usize>, offset: usize) -> f32 {
    let total = text[range.clone()].chars().count();
    if total == 0 {
        return 0.0;
    }
    text[range.start..offset].chars().count() as f32 / total as f32
}

/// The break of `breaks` whose share lies nearest `share`, the first of equally near ones,
/// with its share.
fn nearest(breaks: &[(usize, f32)], share: f32) -> (usize, f32) {
    breaks
        .iter()
        .copied()
        .min_by(|a, b| (a.1 - share).abs().total_cmp(&(b.1 - share).abs()))
        .unwrap_or_default()
}
