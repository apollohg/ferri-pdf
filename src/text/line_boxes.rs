//! Line box assembly and vertical alignment (CSS 2.1 §10.8, CSS Inline 3 §5.3).
use crate::document::Paragraph;
use crate::fonts::FaceMetrics;
use crate::style::{Baseline, BaselineShift, Style, TextEdge, VerticalAlign};
use crate::text::{GlyphRun, Line, TextContext, TextError};
use serde::Serialize;
use std::ops::Range;

/// `sub` lowers by half the parent's descent and `super` raises by half its ascent.
const SUBSCRIPT_DESCENT_FRACTION: f32 = 0.5;
const SUPERSCRIPT_ASCENT_FRACTION: f32 = 0.5;
/// Line heights this close above a whole number of steps round down to it.
const STEP_TOLERANCE_PT: f32 = 1e-3;
const SIDE_RIGHT: usize = 1;
const SIDE_LEFT: usize = 3;

/// Margin-box metrics of an atomic inline, measured by layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AtomicMetrics {
    pub width: f32,
    pub height: f32,
    /// Distance from the margin-box top to the box's baseline.
    pub baseline: f32,
    /// Whether the box has no content baseline, so that the baseline is its margin bottom.
    pub synthesized_baseline: bool,
}

/// One visually contiguous piece of an inline box on a line.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct BoxFragment {
    /// Index into the paragraph's inline boxes.
    pub index: usize,
    /// Border-box left edge and width, from the line's left edge.
    pub x: f32,
    pub width: f32,
    /// The box's baseline below the line's baseline.
    pub baseline: f32,
    /// Content-area extent above and below the box baseline, from its first available font.
    pub ascent: f32,
    pub descent: f32,
    /// Whether the left and right margin, border and padding belong to this fragment.
    pub edges: [bool; 2],
    #[serde(skip)]
    pub(crate) slots: Range<usize>,
}

#[derive(Clone)]
pub(crate) struct LinkFragment {
    pub link: usize,
    pub rect: crate::graphics::Rect,
    pub inline_box: Option<usize>,
}

pub(crate) fn links(paragraph: &Paragraph, line: &Line) -> Vec<LinkFragment> {
    let mut fragments = Vec::new();
    for link in paragraph.links() {
        for run in &line.runs {
            let range = if run.source_range.is_empty() {
                &paragraph.spans()[run.source_span].range
            } else {
                &run.source_range
            };
            let start = range.start.max(link.range.start);
            let end = range.end.min(link.range.end);
            if start >= end {
                continue;
            }
            let [left, right] = if run.source_range.is_empty() {
                [0.0, run.width]
            } else {
                let a = run.x_at(start);
                let b = run.x_at(end);
                [a.min(b), a.max(b)]
            };
            if left == right {
                continue;
            }
            let [top, bottom] = run_extent(run);
            fragments.push(LinkFragment {
                link: link.link,
                rect: crate::graphics::Rect {
                    x: run.x + left,
                    y: run.baseline_shift + top,
                    width: right - left,
                    height: bottom - top,
                },
                inline_box: paragraph.spans()[run.source_span].inline_box,
            });
        }
    }
    fragments
}

/// An atomic inline placed on a line.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct AtomicFragment {
    /// Index into the paragraph's atomic inlines.
    pub index: usize,
    /// Margin-box left edge from the line's left edge.
    pub x: f32,
    pub width: f32,
    /// The box's baseline below the line's baseline.
    pub baseline: f32,
    #[serde(skip)]
    pub(crate) offset: usize,
    #[serde(skip)]
    pub(crate) owner: Option<usize>,
    #[serde(skip)]
    pub(crate) rtl: bool,
}

/// Line content in visual order, before inline box edges are inserted.
pub(crate) enum Piece {
    Text(Box<GlyphRun>),
    Atomic {
        index: usize,
        offset: usize,
        owner: Option<usize>,
        rtl: bool,
    },
}

impl Piece {
    pub(crate) fn source(&self) -> (Range<usize>, bool) {
        match self {
            Self::Text(run) => (run.source_range.clone(), run.rtl),
            Self::Atomic { offset, rtl, .. } => (
                *offset..*offset + crate::document::OBJECT_REPLACEMENT_CHARACTER.len_utf8(),
                *rtl,
            ),
        }
    }
}

pub(crate) fn insertion(
    sources: impl Iterator<Item = (usize, Range<usize>, bool)>,
    offset: usize,
) -> usize {
    let mut after: Option<(usize, usize, bool)> = None;
    let mut last: Option<(usize, usize, bool)> = None;
    for (index, range, rtl) in sources {
        if range.start >= offset && after.is_none_or(|(_, start, _)| range.start < start) {
            after = Some((index, range.start, rtl));
        }
        if last.is_none_or(|(_, end, _)| range.end > end) {
            last = Some((index, range.end, rtl));
        }
    }
    after
        .map(|(index, _, rtl)| index + usize::from(rtl))
        .or_else(|| last.map(|(index, _, rtl)| index + usize::from(!rtl)))
        .unwrap_or(0)
}

/// Visual order of a line's runs, atomic inlines and inline box edges.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Slot {
    Run(usize),
    Atomic(usize),
    Edge {
        owner: usize,
        left: bool,
        margin: f32,
        width: f32,
    },
    /// `text-autospace` spacing rendered within inline box `owner` (`None` is the root
    /// inline box) but outside the nested box whose content precedes it (CSS Text 4 §8.4).
    Space {
        owner: Option<usize>,
        width: f32,
    },
}

pub(crate) struct Arranged {
    pub runs: Vec<GlyphRun>,
    pub atomics: Vec<AtomicFragment>,
    pub boxes: Vec<BoxFragment>,
    pub order: Vec<Slot>,
    /// Inline boxes with content or edges on the line, in tree order.
    pub present: Vec<usize>,
    pub width: f32,
    pub ruby: crate::text::ruby::RubyLine,
}

/// What spaces a line's characters and boxes once they are arranged: `text-autospace`, and
/// the ruby columns of its ruby containers.
#[derive(Clone, Copy)]
pub(crate) struct Spacers<'s, 'p, 'c, 'f> {
    pub autospace: Option<&'s crate::text::autospace::Autospace<'p, 'c, 'f>>,
    pub ruby: Option<&'s crate::text::ruby::RubyLayout<'p, 'c, 'f>>,
}
impl Arranged {
    pub(super) fn owner(&self, paragraph: &Paragraph, slot: Slot) -> Option<usize> {
        match slot {
            Slot::Run(index) => paragraph.spans()[self.runs[index].source_span].inline_box,
            Slot::Atomic(index) => self.atomics[index].owner,
            Slot::Edge { owner, .. } => Some(owner),
            Slot::Space { owner, .. } => owner,
        }
    }
    /// Logical text range and direction of a content slot.
    fn source(&self, slot: Slot) -> Option<(Range<usize>, bool)> {
        match slot {
            Slot::Run(index) => Some((self.runs[index].source_range.clone(), self.runs[index].rtl)),
            Slot::Atomic(index) => {
                let offset = self.atomics[index].offset;
                let length = crate::document::OBJECT_REPLACEMENT_CHARACTER.len_utf8();
                Some((offset..offset + length, self.atomics[index].rtl))
            }
            Slot::Edge { .. } | Slot::Space { .. } => None,
        }
    }
    pub(super) fn insertion(&self, offset: usize) -> usize {
        insertion(
            self.order.iter().enumerate().filter_map(|(index, slot)| {
                self.source(*slot).map(|(range, rtl)| (index, range, rtl))
            }),
            offset,
        )
    }
}

pub(crate) fn slot_width(runs: &[GlyphRun], atomics: &[AtomicFragment], slot: Slot) -> f32 {
    match slot {
        Slot::Run(index) => runs[index].width,
        Slot::Atomic(index) => atomics[index].width,
        Slot::Edge { width, .. } | Slot::Space { width, .. } => width,
    }
}

/// `[margin, border + padding]` of the left and right sides; `auto` margins are zero.
fn edge_widths(style: &Style, basis: f32) -> [[f32; 2]; 2] {
    let side = |index: usize| {
        [
            style.margin[index].resolve(basis).unwrap_or(0.0),
            style.padding[index].resolve(basis) + style.decoration.borders[index].used_width(),
        ]
    };
    [side(SIDE_LEFT), side(SIDE_RIGHT)]
}

/// Inline boxes overlapping `line` or empty within it, with their ancestors, in tree order.
fn present_boxes(paragraph: &Paragraph, line: Range<usize>) -> Vec<usize> {
    let boxes = paragraph.boxes();
    if boxes.is_empty() {
        return Vec::new();
    }
    let mut present = Vec::new();
    let mut chain = |mut inner: Option<usize>| {
        while let Some(index) = inner {
            present.push(index);
            inner = boxes[index].parent;
        }
    };
    let spans = paragraph.spans();
    let first = spans.partition_point(|span| span.range.end <= line.start);
    for span in spans[first..]
        .iter()
        .take_while(|span| span.range.start < line.end)
    {
        chain(span.inline_box);
    }
    let at_end = line.end == paragraph.text().len();
    let first = boxes.partition_point(|inline_box| inline_box.range.start < line.start);
    for (index, inline_box) in boxes
        .iter()
        .enumerate()
        .skip(first)
        .take_while(|(_, inline_box)| inline_box.range.start <= line.end)
    {
        if inline_box.range.is_empty() && (inline_box.range.start < line.end || at_end) {
            chain(Some(index));
        }
    }
    present.sort_unstable();
    present.dedup();
    present
}

/// Orders `pieces` with the margins, borders and padding of the inline boxes they lie in.
/// A box's left and right edges sit beside its leftmost and rightmost content on the line
/// (CSS 2.1 §8.6); a sliced box keeps only the edges of the lines where it starts or ends.
/// `spacers` space the adjoining characters they apply to and the ruby bases on the line,
/// whose annotation boxes follow its atomic inlines in `atomic_widths`.
pub(crate) fn arrange(
    paragraph: &Paragraph,
    pieces: Vec<Piece>,
    line: Range<usize>,
    atomic_widths: &[f32],
    basis: f32,
    spacers: Spacers<'_, '_, '_, '_>,
) -> Result<Arranged, TextError> {
    let mut arranged = Arranged {
        runs: Vec::new(),
        atomics: Vec::new(),
        boxes: Vec::new(),
        order: Vec::with_capacity(pieces.len()),
        present: present_boxes(paragraph, line.clone()),
        width: 0.0,
        ruby: crate::text::ruby::RubyLine::default(),
    };
    for piece in pieces {
        match piece {
            Piece::Text(run) => {
                arranged.order.push(Slot::Run(arranged.runs.len()));
                arranged.runs.push(*run);
            }
            Piece::Atomic {
                index,
                offset,
                owner,
                rtl,
            } => {
                arranged.order.push(Slot::Atomic(arranged.atomics.len()));
                arranged.atomics.push(AtomicFragment {
                    index,
                    x: 0.0,
                    width: atomic_widths[index],
                    baseline: 0.0,
                    offset,
                    owner,
                    rtl,
                });
            }
        }
    }
    let boxes = paragraph.boxes();
    let mut edges = vec![[false; 2]; arranged.present.len()];
    for (position, &index) in arranged.present.iter().enumerate().rev() {
        let inline_box = &boxes[index];
        let clone = inline_box.style.clone_decoration;
        let starts = clone || (!inline_box.continued[0] && line.start <= inline_box.range.start);
        let ends = clone || (!inline_box.continued[1] && inline_box.range.end <= line.end);
        let present = if inline_box.style.rtl {
            [ends, starts]
        } else {
            [starts, ends]
        };
        edges[position] = present;
        let widths = edge_widths(&inline_box.style, basis);
        let [left, right] = [0, 1].map(|side| {
            let [margin, inner] = widths[side];
            (present[side] && margin + inner != 0.0).then_some(Slot::Edge {
                owner: index,
                left: side == 0,
                margin,
                width: margin + inner,
            })
        });
        if left.is_none() && right.is_none() {
            continue;
        }
        let belongs = |slot: &Slot| paragraph.is_within(arranged.owner(paragraph, *slot), index);
        let first = arranged.order.iter().position(belongs);
        let last = arranged.order.iter().rposition(belongs);
        let (left_at, right_at) = if let Some((first, last)) = first.zip(last) {
            (first, last + 1)
        } else {
            let at = arranged.insertion(inline_box.range.start);
            (at, at)
        };
        if let Some(slot) = right {
            arranged.order.insert(right_at, slot);
        }
        if let Some(slot) = left {
            arranged.order.insert(left_at, slot);
        }
    }
    if let Some(autospace) = spacers.autospace {
        autospace.apply(&mut arranged)?;
    }
    if let Some(ruby) = spacers.ruby {
        arranged.ruby = ruby.space(&mut arranged, atomic_widths)?;
    }
    for (position, &index) in arranged.present.iter().enumerate() {
        let belongs: Vec<bool> = arranged
            .order
            .iter()
            .map(|slot| paragraph.is_within(arranged.owner(paragraph, *slot), index))
            .collect();
        let mut pieces = Vec::new();
        let mut start = None;
        for (slot, inside) in belongs.iter().copied().chain([false]).enumerate() {
            match (inside, start) {
                (true, None) => start = Some(slot),
                (false, Some(first)) => {
                    pieces.push(first..slot);
                    start = None;
                }
                _ => {}
            }
        }
        let count = pieces.len();
        for (piece, slots) in pieces.into_iter().enumerate() {
            arranged.boxes.push(BoxFragment {
                index,
                x: 0.0,
                width: 0.0,
                baseline: 0.0,
                ascent: 0.0,
                descent: 0.0,
                edges: [
                    piece == 0 && edges[position][0],
                    piece + 1 == count && edges[position][1],
                ],
                slots,
            });
        }
    }
    arranged.width = arranged
        .order
        .iter()
        .map(|slot| slot_width(&arranged.runs, &arranged.atomics, *slot))
        .sum();
    Ok(arranged)
}

impl Line {
    /// Positions runs, atomic inlines and box fragments from the left edge in visual order.
    pub(crate) fn place(&mut self) {
        let mut x = 0.0;
        let mut positions = Vec::with_capacity(self.order.len());
        for &slot in &self.order {
            positions.push(x);
            match slot {
                Slot::Run(index) => self.runs[index].x = x,
                Slot::Atomic(index) => self.atomics[index].x = x,
                Slot::Edge { .. } | Slot::Space { .. } => {}
            }
            x += slot_width(&self.runs, &self.atomics, slot);
        }
        let width = |slot: Slot| slot_width(&self.runs, &self.atomics, slot);
        let own_margin = |slot: Slot, index: usize, left: bool| match slot {
            Slot::Edge {
                owner,
                left: side,
                margin,
                ..
            } if owner == index && side == left => margin,
            _ => 0.0,
        };
        let geometry: Vec<(f32, f32)> = self
            .boxes
            .iter()
            .map(|fragment| {
                let first = fragment.slots.start;
                let last = fragment.slots.end - 1;
                let left = positions[first] + own_margin(self.order[first], fragment.index, true);
                let right = positions[last] + width(self.order[last])
                    - own_margin(self.order[last], fragment.index, false);
                (left, right - left)
            })
            .collect();
        for (fragment, (x, width)) in self.boxes.iter_mut().zip(geometry) {
            fragment.x = x;
            fragment.width = width;
        }
    }
}

/// Ascent, descent, line height and x-height of a box's first available font.
pub(crate) struct BoxMetrics {
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
    line_height: f32,
    pub(crate) x_height: f32,
    cap_height: f32,
    /// The font's baseline table, in em, and the used size it scales by.
    baselines: crate::fonts::BaselineTable,
    size: f32,
}
impl BoxMetrics {
    pub(crate) fn of(context: &TextContext<'_>, style: &Style) -> Result<Self, TextError> {
        let font = context.primary(style)?;
        let instance = context
            .face(font, style)?
            .ok_or(TextError::NoFontCandidates)?;
        Ok(Self::of_face(
            &FaceMetrics::of(&instance.face),
            instance.used_size(style),
            style.line_height.resolve(style.size),
        ))
    }
    /// A face's metrics at `size`; `line_height` is `None` for `normal`.
    fn of_face(metrics: &FaceMetrics, size: f32, line_height: Option<f32>) -> Self {
        let [ascent, descent, gap] =
            [metrics.ascent, metrics.descent, metrics.line_gap].map(|metric| metric * size);
        Self {
            ascent,
            descent,
            line_height: line_height.unwrap_or(ascent + descent + gap),
            x_height: metrics.x_height * size,
            cap_height: metrics.cap_height * size,
            baselines: metrics.baselines,
            size,
        }
    }
    /// How far `baseline` lies below the alphabetic baseline (CSS Inline 3 §3.3).
    pub(crate) fn baseline(&self, baseline: Baseline) -> f32 {
        let table = &self.baselines;
        match baseline {
            Baseline::Alphabetic => 0.0,
            Baseline::TextBottom => self.descent,
            Baseline::TextTop => -self.ascent,
            Baseline::Ideographic => -table.ideographic_under * self.size,
            Baseline::Middle => -self.x_height / 2.0,
            Baseline::Central => {
                -f32::midpoint(table.ideographic_over, table.ideographic_under) * self.size
            }
            Baseline::Mathematical => -table.math * self.size,
            Baseline::Hanging => -table.hanging * self.size,
        }
    }
    /// How far a `text-box-edge` metric lies below the alphabetic baseline, as the over edge
    /// or, with `under`, the under edge (CSS Inline 3 §5.2).
    pub(crate) fn edge(&self, edge: TextEdge, under: bool) -> f32 {
        let table = &self.baselines;
        match (edge, under) {
            (TextEdge::Text, false) => -self.ascent,
            (TextEdge::Text, true) => self.descent,
            (TextEdge::Cap, _) => -self.cap_height,
            (TextEdge::Ex, _) => -self.x_height,
            (TextEdge::Ideographic, false) => -table.ideographic_over * self.size,
            (TextEdge::Ideographic, true) => -table.ideographic_under * self.size,
            (TextEdge::IdeographicInk, false) => -table.ideographic_ink_over * self.size,
            (TextEdge::IdeographicInk, true) => -table.ideographic_ink_under * self.size,
            (TextEdge::Alphabetic, _) => 0.0,
            (TextEdge::Hanging, _) => -table.hanging * self.size,
        }
    }
    pub(crate) fn line_height(&self) -> f32 {
        self.line_height
    }
    fn half_leading(&self) -> f32 {
        (self.line_height - self.ascent - self.descent) / 2.0
    }
    /// Inline box extent around its baseline: the content area plus half-leading (§10.8.1).
    pub(crate) fn extent(&self) -> [f32; 2] {
        [
            -(self.ascent + self.half_leading()),
            self.descent + self.half_leading(),
        ]
    }
}

/// Glyph extent of a run with its half-leading, around its baseline.
fn run_extent(run: &GlyphRun) -> [f32; 2] {
    let metrics = run.metrics;
    let [ascent, descent, gap] =
        [metrics.ascent, metrics.descent, metrics.line_gap].map(|metric| metric * run.metric_size);
    let half = (run.leading.unwrap_or(ascent + descent + gap) - ascent - descent) / 2.0;
    [-(ascent + half), descent + half]
}

/// Baseline offset from the parent's baseline, positive downwards, for a box whose extent
/// around its own baseline is `[top, bottom]`. `vertical-align`, a longhand of the pinned
/// grammar, keeps its CSS 2.1 meaning: its alignment keywords take the place of
/// `alignment-baseline`, and its shifts add to `baseline-shift` (CSS Inline 3 §4.2).
fn offset(
    align: &Alignment<'_>,
    (parent, dominant): Parent<'_>,
    [top, bottom]: [f32; 2],
    line_height: impl Fn() -> Result<f32, TextError>,
) -> Result<f32, TextError> {
    let shift = |shift: &BaselineShift| -> Result<f32, TextError> {
        Ok(match shift {
            BaselineShift::Raise(length) => -length.resolve(line_height()?),
            BaselineShift::Sub => parent.descent * SUBSCRIPT_DESCENT_FRACTION,
            BaselineShift::Super => -parent.ascent * SUPERSCRIPT_ASCENT_FRACTION,
        })
    };
    let aligned = || {
        let baseline = align.baseline.alignment.unwrap_or(dominant);
        parent.baseline(baseline) - (align.own)(baseline)
    };
    let legacy = match align.vertical {
        VerticalAlign::Top | VerticalAlign::Bottom => return Ok(0.0),
        VerticalAlign::Baseline => aligned(),
        VerticalAlign::Sub => aligned() + shift(&BaselineShift::Sub)?,
        VerticalAlign::Super => aligned() + shift(&BaselineShift::Super)?,
        VerticalAlign::Length(length) => aligned() + shift(&BaselineShift::Raise(length.clone()))?,
        VerticalAlign::TextTop => -parent.ascent - top,
        VerticalAlign::TextBottom => parent.descent - bottom,
        VerticalAlign::Middle => -parent.x_height / 2.0 - f32::midpoint(top, bottom),
        VerticalAlign::BaselineMiddle => -(top + bottom) / 2.0,
    };
    Ok(legacy + shift(&align.baseline.shift)?)
}

/// How a box aligns in its parent: its legacy `vertical-align`, its `alignment-baseline`
/// and `baseline-shift`, and where each of its baselines lies below its alphabetic one.
struct Alignment<'a> {
    vertical: &'a VerticalAlign,
    baseline: &'a crate::style::BaselineAlignment,
    own: &'a dyn Fn(Baseline) -> f32,
}

/// A parent's metrics and the dominant baseline it aligns its children by.
type Parent<'a> = (&'a BoxMetrics, Baseline);

/// Vertical extent of boxes aligned together: the root inline box, or one `top`/`bottom` subtree.
struct Extent {
    top: f32,
    bottom: f32,
    to_bottom: bool,
}

/// Shift of a box's baseline within its alignment subtree (`None` is the root subtree).
#[derive(Clone, Copy)]
struct Aligned {
    shift: f32,
    tree: Option<usize>,
}
const ROOT: Aligned = Aligned {
    shift: 0.0,
    tree: None,
};

/// Extents of the root subtree and of each `top`/`bottom`-aligned subtree on a line.
struct Subtrees {
    root: Extent,
    trees: Vec<Extent>,
}
impl Subtrees {
    fn include(&mut self, placed: Aligned, [top, bottom]: [f32; 2]) {
        let extent = match placed.tree {
            Some(tree) => &mut self.trees[tree],
            None => &mut self.root,
        };
        extent.top = extent.top.min(placed.shift + top);
        extent.bottom = extent.bottom.max(placed.shift + bottom);
    }
    /// Aligns a box with `extent` around its baseline inside `parent`, and includes it.
    fn place(
        &mut self,
        align: &Alignment<'_>,
        parent: (Aligned, Parent<'_>),
        extent: [f32; 2],
        line_height: impl Fn() -> Result<f32, TextError>,
    ) -> Result<Aligned, TextError> {
        let placed = match align.vertical {
            VerticalAlign::Top | VerticalAlign::Bottom => {
                self.trees.push(Extent {
                    top: extent[0],
                    bottom: extent[1],
                    to_bottom: *align.vertical == VerticalAlign::Bottom,
                });
                Aligned {
                    shift: 0.0,
                    tree: Some(self.trees.len() - 1),
                }
            }
            _ => Aligned {
                shift: parent.0.shift + offset(align, parent.1, extent, line_height)?,
                tree: parent.0.tree,
            },
        };
        self.include(placed, extent);
        Ok(placed)
    }
}

/// Aligns the line's boxes vertically and returns the line box `[ascent, descent]`.
/// `newline` is the offset of a forced break ending the line, whose strut also counts.
/// `ruby` adds the leading its ruby containers' annotations need, with percentages against
/// `basis`.
pub(crate) fn align(
    context: &TextContext<'_>,
    paragraph: &Paragraph,
    line: &mut Line,
    atomics: &[AtomicMetrics],
    newline: Option<usize>,
    (ruby, basis): (Option<&crate::text::ruby::RubyLayout<'_, '_, '_>>, f32),
) -> Result<[f32; 2], TextError> {
    // CSS 2.1 §9.4.2: anchors retain their inline scope without creating a line strut.
    let empty_images = newline.is_none()
        && line.runs.is_empty()
        && line.annotations.is_empty()
        && !line.atomics.is_empty()
        && line.atomics.iter().all(|fragment| {
            let block = &paragraph.atomics()[fragment.index].block;
            let metrics = atomics[fragment.index];
            block.is_empty_image_fallback()
                && block.style().width == crate::style::Size::Auto
                && block.style().height == crate::style::Size::Auto
                && !block.style().has_inline_edges([false; 2])
                && block.style().margin.iter().all(crate::style::Size::is_zero)
                && !block.decoration().is_visible()
                && metrics.width == 0.0
                && metrics.height == 0.0
        });
    if (paragraph.text().is_empty() || empty_images)
        && paragraph
            .boxes()
            .iter()
            .all(|inline| !inline.style.has_inline_edges(inline.continued))
    {
        return Ok([0.0, 0.0]);
    }
    let root_metrics = BoxMetrics::of(context, paragraph.style())?;
    let [top, bottom] = root_metrics.extent();
    let mut subtrees = Subtrees {
        root: Extent {
            top,
            bottom,
            to_bottom: false,
        },
        trees: Vec::new(),
    };
    let root_dominant = paragraph.style().baseline.dominant;
    let present = std::mem::take(&mut line.present);
    let mut aligned: Vec<(Aligned, BoxMetrics, Baseline)> = Vec::with_capacity(present.len());
    let boxes = paragraph.boxes();
    // Content and boxes only belong to boxes present on their line, whose ancestors are present.
    let position = |owner: usize| {
        present
            .binary_search(&owner)
            .expect("inline content belongs to boxes present on its line")
    };
    for &index in &present {
        let style = &boxes[index].style;
        let metrics = BoxMetrics::of(context, style)?;
        let parent = match boxes[index].parent {
            Some(parent) => {
                let (placed, metrics, dominant) = &aligned[position(parent)];
                (*placed, (metrics, *dominant))
            }
            None => (ROOT, (&root_metrics, root_dominant)),
        };
        let own = |baseline| metrics.baseline(baseline);
        let mut extent = metrics.extent();
        if let Some(ruby) = ruby {
            let [over, under] = ruby.leading(index, &present, &metrics, basis)?;
            extent[0] -= over;
            extent[1] += under;
        }
        let placed = subtrees.place(
            &Alignment {
                vertical: &style.table.vertical_align,
                baseline: &style.baseline,
                own: &own,
            },
            parent,
            extent,
            || Ok(metrics.line_height),
        )?;
        aligned.push((placed, metrics, style.baseline.dominant));
    }
    let owner = |owner: Option<usize>| {
        owner.map_or((ROOT, &root_metrics, root_dominant), |owner| {
            let (placed, metrics, dominant) = &aligned[position(owner)];
            (*placed, metrics, *dominant)
        })
    };
    let mut runs = Vec::with_capacity(line.runs.len());
    for run in &line.runs {
        let span = &paragraph.spans()[run.source_span];
        let (placed, metrics, dominant) = owner(span.inline_box);
        // Each glyph's dominant baseline lies on its box's (CSS Inline 3 §4.1).
        let glyphs = BoxMetrics::of_face(&run.metrics, run.metric_size, None);
        let placed = Aligned {
            shift: placed.shift + metrics.baseline(dominant) - glyphs.baseline(dominant),
            ..placed
        };
        let mut extent = if span.style.line_height == crate::style::LineHeight::Normal {
            run_extent(run)
        } else {
            // CSS Inline 3 excludes fallback metrics from non-normal layout bounds.
            BoxMetrics::of(context, &span.style)?.extent()
        };
        if let Some((leading, under)) = crate::text::emphasis::leading(context, &span.style, run)? {
            extent[usize::from(under)] += if under { leading } else { -leading };
        }
        subtrees.include(placed, extent);
        runs.push(placed);
    }
    if let Some(offset) = newline {
        let span = &paragraph.spans()[paragraph
            .spans()
            .partition_point(|span| span.range.end <= offset)];
        subtrees.include(
            owner(span.inline_box).0,
            BoxMetrics::of(context, &span.style)?.extent(),
        );
    }
    let mut atomic_placements = Vec::with_capacity(line.atomics.len());
    for fragment in &line.atomics {
        let metrics = atomics[fragment.index];
        let style = paragraph.atomics()[fragment.index].block.style();
        let (placed, parent, dominant) = owner(fragment.owner);
        let font = BoxMetrics::of(context, style)?;
        // Without a content baseline the margin box gives the baselines (CSS Inline 3 §A.3).
        let own = |baseline| {
            if !metrics.synthesized_baseline {
                return font.baseline(baseline);
            }
            match baseline {
                Baseline::Alphabetic | Baseline::TextBottom | Baseline::Ideographic => 0.0,
                Baseline::Central | Baseline::Mathematical | Baseline::Middle => {
                    -metrics.height / 2.0
                }
                Baseline::TextTop | Baseline::Hanging => -metrics.height,
            }
        };
        atomic_placements.push(subtrees.place(
            &Alignment {
                vertical: &style.table.vertical_align,
                baseline: &style.baseline,
                own: &own,
            },
            (placed, (parent, dominant)),
            [-metrics.baseline, metrics.height - metrics.baseline],
            || Ok(font.line_height),
        )?);
    }
    let Subtrees { root, trees } = subtrees;
    let ascent = -root.top;
    let height = trees
        .iter()
        .map(|tree| tree.bottom - tree.top)
        .fold(root.bottom - root.top, f32::max);
    // `line-height-step` rounds the height up to whole steps, centring the line in the extra
    // space as an inline box would that does not move the baseline (CSS Rhythmic Sizing 1 §3).
    let step = paragraph.style().line_height_step;
    let (ascent, height) = if step > 0.0 {
        let steps = ((height - STEP_TOLERANCE_PT) / step).ceil();
        let rounded = steps * step;
        (ascent + (rounded - height) / 2.0, rounded)
    } else {
        (ascent, height)
    };
    // Baseline-aligned content stays at the top when aligned subtrees enlarge the line.
    let descent = height - ascent;
    let baseline = |placed: Aligned| {
        placed.shift
            + match placed.tree {
                None => 0.0,
                Some(tree) if trees[tree].to_bottom => descent - trees[tree].bottom,
                Some(tree) => -ascent - trees[tree].top,
            }
    };
    for (run, placed) in line.runs.iter_mut().zip(runs) {
        run.baseline_shift = baseline(placed);
    }
    for (fragment, placed) in line.atomics.iter_mut().zip(atomic_placements) {
        fragment.baseline = baseline(placed);
    }
    for fragment in &mut line.boxes {
        let (placed, metrics, _) = &aligned[position(fragment.index)];
        fragment.baseline = baseline(*placed);
        // A trimmed side's content edge is its `text-box-edge` metric (CSS Inline 3 §6.2).
        let text_box = &paragraph.boxes()[fragment.index].style.text_box;
        let [ascent, descent] = [false, true].map(|under| {
            let side = usize::from(under);
            let edge = if text_box.trim[side] {
                text_box.edges[side]
            } else {
                TextEdge::Text
            };
            metrics.edge(edge, under).abs()
        });
        fragment.ascent = ascent;
        fragment.descent = descent;
    }
    line.present = present;
    Ok([ascent, descent])
}
