//! Ruby layout (CSS Ruby 1 §§3–5). A ruby container's bases lie on the line as inline boxes;
//! each line's columns are sized to their widest content, the bases distributing the extra
//! space by `ruby-align` before the line is fitted, and its annotation boxes are then placed
//! over or under them, stacked level by level against the base container.
use crate::document::{
    Paragraph,
    ruby::{Annotation, Level, Ruby},
};
use crate::fonts::{FALLBACK_IDEOGRAPH_ADVANCE_EM, WATER_IDEOGRAPH};
use crate::style::{RubyAlign, RubyMerge, Style};
use crate::text::line_boxes::{
    Arranged, AtomicFragment, BoxFragment, BoxMetrics, Slot, slot_width,
};
use crate::text::{GlyphRun, Line, TextContext, TextError, autospace, spacing};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use unicode_linebreak::BreakOpportunity;
use unicode_script::{Script, UnicodeScript};

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

/// What laying out an annotation box's contents shows about it: its justification
/// opportunities, and its first and last characters, which adjoin the next and previous
/// annotation of a merged run.
#[derive(Clone)]
struct Contents {
    opportunities: usize,
    first: String,
    last: String,
}

/// The ruby containers of a paragraph and what laying them out measures once.
pub(crate) struct RubyLayout<'p, 'c, 'f> {
    context: &'c TextContext<'f>,
    paragraph: &'p Paragraph,
    /// The paragraph whose annotation boxes hold the annotations' contents: `paragraph`, or
    /// the one whose first line it is the `::first-line` copy of.
    source: &'p Paragraph,
    /// Rubies nested in another's bases come before it.
    order: Vec<usize>,
    contents: RefCell<HashMap<usize, Contents>>,
    spaces: RefCell<HashMap<*const Style, f32>>,
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

/// Breaking a ruby base, with its annotations, across lines (§3.4.2).
pub(super) const WITHIN_BASES: TextError = TextError::UnsupportedRuby {
    feature: "line breaking inside a ruby base",
};

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

    /// The width an annotation takes on a line, from the measured widths of the paragraph's
    /// atomic inlines followed by its annotation boxes.
    fn width(&self, annotation: &Annotation, widths: &[f32]) -> Result<f32, TextError> {
        if annotation.whitespace {
            return self.space_width(&annotation.style);
        }
        Ok(annotation
            .block
            .map_or(0.0, |block| widths[self.paragraph.atomics().len() + block]))
    }

    /// The justification opportunities and edge characters of annotation box `block`, from
    /// its contents laid out at their measured `width`.
    fn contents(&self, block: usize, width: f32) -> Result<Contents, TextError> {
        if let Some(contents) = self.contents.borrow().get(&block) {
            return Ok(contents.clone());
        }
        let anchor = &self.source.annotations()[block];
        let mut contents = Contents {
            opportunities: 0,
            first: String::new(),
            last: String::new(),
        };
        if let Some(crate::document::BlockContent::Inline(paragraph)) =
            anchor.block.children().first()
        {
            let atomics = vec![super::AtomicMetrics::default(); paragraph.inline_blocks().count()];
            let lines = self.context.layout_paragraph(paragraph, width, &atomics)?;
            for line in &lines {
                contents.opportunities +=
                    spacing::opportunities(paragraph, &line.runs, &line.order).len();
            }
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
            contents.first = all.next().unwrap_or_default();
            contents.last = all.last().unwrap_or_else(|| contents.first.clone());
        }
        self.contents.borrow_mut().insert(block, contents.clone());
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

    /// How `level` places its annotations over the line's bases `[first, last]`, given each
    /// base's content width and the widths of the gaps before them; `merge` decides whether
    /// `ruby-merge: auto` merges them.
    fn placed(level: &Level, [first, last]: [usize; 2], merged: bool) -> Vec<Placed> {
        let on_line = |annotation: &Annotation| {
            !annotation.hidden
                && if annotation.whitespace {
                    first < annotation.bases.start && annotation.bases.start <= last
                } else {
                    annotation.bases.start <= last && annotation.bases.end > first
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
                    bases: [annotation.bases.start, annotation.bases.end - 1],
                    merged: false,
                });
            }
        }
        placed.extend(run);
        placed
    }

    /// Whether `level` merges its annotations on a line where the bases `[first, last]` have
    /// content widths `bases` and the gaps before them widths `gaps`: always under `merge`,
    /// and under `auto` where some annotation is wider than its bases (§4.2).
    fn merges(
        &self,
        level: &Level,
        bases: &[f32],
        gaps: &[f32],
        span: [usize; 2],
        widths: &[f32],
    ) -> Result<bool, TextError> {
        Ok(match level.style.ruby.merge {
            RubyMerge::Separate => false,
            RubyMerge::Merge => true,
            RubyMerge::Auto => {
                let mut overflows = false;
                for placed in Self::placed(level, span, false) {
                    let annotation = &level.annotations[placed.annotations.start];
                    let [start, end] = placed.bases.map(|base| base - span[0]);
                    let available: f32 = bases[start..=end].iter().sum::<f32>()
                        + gaps[start + 1..=end].iter().sum::<f32>();
                    overflows |= self.width(annotation, widths)? > available;
                }
                overflows
            }
        })
    }

    /// Sizes the ruby columns of an arranged line and spaces its bases to fill them, before
    /// its inline box fragments are formed.
    pub(super) fn space(
        &self,
        arranged: &mut Arranged,
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
                    let merged = self.merges(level, &bases, &gaps, span, widths)?;
                    if merged && level.style.ruby.merge == RubyMerge::Auto {
                        ruby_line
                            .merged
                            .push([ruby_index, segment_index, level_index]);
                    }
                    for placed in Self::placed(level, span, merged) {
                        let mut total = 0.0;
                        for annotation in &level.annotations[placed.annotations.clone()] {
                            total += self.width(annotation, widths)?;
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
                                cells[cell] = cells[cell].max(self.width(annotation, widths)?);
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
            }
        }
        self.overhang(arranged, &mut ruby_line, widths)?;
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
        segment: &crate::document::ruby::Segment,
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
        widths: &[f32],
    ) -> Result<(), TextError> {
        let paragraph = self.paragraph;
        let rubies: Vec<&Ruby> = paragraph.rubies().iter().collect();
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
                    self.overhang_limit(ruby, ruby_line.columns[column].base, widths)?
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

    /// How far annotations over `base` may overhang: half an ideograph in the smallest of
    /// their fonts; `None` where a level over it forbids overhang.
    fn overhang_limit(
        &self,
        ruby: &Ruby,
        base: usize,
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
                        || self.width(annotation, widths)? == 0.0
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
                let Some(span) =
                    Self::bases_on_line(&segment.bases, |base| fragments.contains_key(&base))
                else {
                    continue;
                };
                let [first, last] = span;
                let mut columns = Vec::with_capacity(last - first + 1);
                let mut container = [f32::INFINITY, f32::NEG_INFINITY];
                for &base in &segment.bases[first..=last] {
                    let pieces = &fragments[&base];
                    let style = &paragraph.boxes()[base].style;
                    let margin = |side: usize| style.margin[side].resolve(basis).unwrap_or(0.0);
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
                    columns.push([
                        left - margin(LEFT) - leading - before,
                        right + margin(RIGHT) + trailing + after,
                    ]);
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
                let mut over = container[0];
                let mut under = container[1];
                for (level_index, level) in segment.levels.iter().enumerate() {
                    let merged = level.style.ruby.merge == RubyMerge::Merge
                        || line
                            .ruby
                            .merged
                            .contains(&[ruby_index, segment_index, level_index]);
                    let placements = Self::placed(level, span, merged);
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
                        let extent = [columns[start][0], columns[end][1]];
                        placed.extend(self.position(
                            level,
                            &placement,
                            extent,
                            baseline,
                            ruby.container,
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

    /// The annotation fragments of `placement` over its columns' `[left, right]` extent, on
    /// its level's `baseline`.
    fn position(
        &self,
        level: &Level,
        placement: &Placed,
        [left, right]: [f32; 2],
        baseline: f32,
        container: usize,
        widths: &[f32],
    ) -> Result<Vec<AtomicFragment>, TextError> {
        let annotations = &level.annotations[placement.annotations.clone()];
        let mut sizes = Vec::with_capacity(annotations.len());
        let mut total = 0.0;
        for annotation in annotations {
            let width = self.width(annotation, widths)?;
            let contents = match annotation.block {
                Some(block) => Some(self.contents(block, width)?),
                None => None,
            };
            total += width;
            sizes.push((width, contents));
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
        let (leading, _, each) = if extra > 0.0 {
            distribution(align, extra, count)
        } else {
            (extra * HALF, extra * HALF, 0.0)
        };
        let mut x = left + leading;
        let mut fragments = Vec::new();
        for ((annotation, (width, contents)), junction) in
            annotations.iter().zip(sizes).zip(junctions)
        {
            if junction {
                x += each;
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
                    x,
                    width: used,
                    baseline,
                    offset: self.paragraph.annotations()[block].offset,
                    owner: Some(container),
                    rtl: false,
                });
            }
            x += used;
        }
        Ok(fragments)
    }

    /// The extra leading a ruby container `container` on a line, whose present inline boxes
    /// are `present`, needs over and under its base container so that identical lines would
    /// not overlap (§3.6). `container_metrics` are the container's own.
    pub(super) fn leading(
        &self,
        container: usize,
        present: &[usize],
        container_metrics: &BoxMetrics,
        basis: f32,
    ) -> Result<[f32; 2], TextError> {
        let Some(ruby) = self
            .paragraph
            .rubies()
            .iter()
            .find(|ruby| ruby.container == container)
        else {
            return Ok([0.0; 2]);
        };
        let ([above, below], heights) = self.stack(ruby, present, basis)?;
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
    fn stack(
        &self,
        ruby: &Ruby,
        present: &[usize],
        basis: f32,
    ) -> Result<([f32; 2], [f32; 2]), TextError> {
        let paragraph = self.paragraph;
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
                for nested in paragraph
                    .rubies()
                    .iter()
                    .filter(|nested| paragraph.is_within(Some(nested.container), base))
                {
                    let ([above, below], [over, under]) = self.stack(nested, present, basis)?;
                    content = [content[0].max(above + over), content[1].max(below + under)];
                }
            }
            let mut sides = [0.0f32; 2];
            for level in &segment.levels {
                let mut extent = [0.0f32; 2];
                let mut any = false;
                for placement in Self::placed(level, span, false) {
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
        Ok((content, heights))
    }
}

/// How `ruby-align` spreads `extra` space around content with `count` justification
/// opportunities: the space before and after it, and what each opportunity takes (§4.3).
/// Without opportunities, the justifying values centre the content.
fn distribution(align: RubyAlign, extra: f32, count: usize) -> (f32, f32, f32) {
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
/// except where §3.4.2 lets it break with its annotations because they too wrap and have soft
/// wrap opportunities of their own.
pub(super) fn constrain(paragraph: &Paragraph, breaks: &mut Vec<(usize, BreakOpportunity)>) {
    if paragraph.rubies().is_empty() {
        return;
    }
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
    let mut forbidden: Vec<Range<usize>> = Vec::new();
    for segment in paragraph.rubies().iter().flat_map(|ruby| &ruby.segments) {
        let range = |base: usize| paragraph.boxes()[base].range.clone();
        for (index, &base) in segment.bases.iter().enumerate() {
            let breakable = segment.levels.iter().all(|level| {
                level
                    .annotations
                    .iter()
                    .find(|annotation| !annotation.whitespace && annotation.bases.contains(&index))
                    .and_then(|annotation| annotation.block)
                    .is_some_and(|block| annotation_wraps(paragraph, block))
            });
            if !breakable {
                forbidden.push(range(base));
            }
        }
        for annotation in segment.levels.iter().flat_map(|level| &level.annotations) {
            if annotation.hidden || annotation.whitespace || annotation.bases.len() < 2 {
                continue;
            }
            forbidden.push(
                range(segment.bases[annotation.bases.start]).start
                    ..range(segment.bases[annotation.bases.end - 1]).end,
            );
        }
    }
    breaks.retain(|&(offset, opportunity)| {
        opportunity == BreakOpportunity::Mandatory
            || !forbidden
                .iter()
                .any(|range| range.start < offset && offset < range.end)
    });
}

/// Whether a line ending at `end` breaks inside a ruby base: at a forced break, or where
/// §3.4.2 lets its annotations break with it, which is not supported.
pub(super) fn ends_inside_base(paragraph: &Paragraph, end: usize) -> bool {
    paragraph
        .rubies()
        .iter()
        .flat_map(|ruby| &ruby.segments)
        .flat_map(|segment| &segment.bases)
        .any(|&base| {
            let range = &paragraph.boxes()[base].range;
            range.start < end && end < range.end
        })
}

/// Whether annotation box `block` allows wrapping and has a soft wrap opportunity inside.
fn annotation_wraps(paragraph: &Paragraph, block: usize) -> bool {
    let Some(crate::document::BlockContent::Inline(contents)) =
        paragraph.annotations()[block].block.children().first()
    else {
        return false;
    };
    let automatic = crate::text::hyphenation::automatic(contents);
    let end = contents.text().len();
    super::whitespace::opportunities(contents, &automatic)
        .iter()
        .any(|&(offset, _)| offset > 0 && offset < end)
}
