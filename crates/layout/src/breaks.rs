use crate::measure::{FlowItem, MeasuredBlock, MeasuredCell, MeasuredRow};
use ferri_pdf_style::{BreakBetween, BreakInside, Style};

/// CSS Break 3 §4: fragmentation progress assumes at least one CSS pixel.
pub(super) fn fragmentainer_height(height: f32) -> f32 {
    height.max(ferri_pdf_graphics::POINTS_PER_PIXEL)
}

/// A single line or replaced element can overflow without splitting its enclosing cell.
pub(super) fn indivisible(block: &MeasuredBlock<'_>) -> bool {
    let mut blocks = vec![block];
    let mut leaves = 0usize;
    while let Some(block) = blocks.pop() {
        if block.has_forced || block.first_fragment < block.height {
            return false;
        }
        for item in &block.items {
            match item {
                FlowItem::Paragraph(paragraph) => leaves += paragraph.lines.len(),
                FlowItem::Image { .. } => leaves += 1,
                FlowItem::Block(block) | FlowItem::Spanner(block) => blocks.push(block),
                FlowItem::Space(value) | FlowItem::Edge(_, value) if *value > 0.0 => return false,
                FlowItem::Margin(margin) if margin.value() > 0.0 => return false,
                FlowItem::Row(_) | FlowItem::Columns(_) => return false,
                FlowItem::Space(_)
                | FlowItem::Edge(_, _)
                | FlowItem::Margin(_)
                | FlowItem::Positioned(_)
                | FlowItem::Events(_) => {}
            }
            if leaves > 1 {
                return false;
            }
        }
    }
    true
}

impl MeasuredRow<'_> {
    pub(super) fn progress_height(&self) -> f32 {
        if self.indivisible() {
            return self.height;
        }
        // Alignment space can fragment before the first line.
        parallel_progress(self.cells.iter(), false)
    }
    pub(super) fn indivisible(&self) -> bool {
        !self.cells.is_empty()
            && self.first_fragment >= self.height
            && !(self.band.as_ref().is_some_and(|band| band.len() > 1)
                && self.cells.iter().any(|cell| cell.stretch))
            && self.cells.iter().all(|cell| indivisible(&cell.block))
    }
}

pub(super) fn parallel_progress<'a, 'b: 'a>(
    cells: impl Iterator<Item = &'a MeasuredCell<'b>>,
    include_alignment: bool,
) -> f32 {
    cells
        .filter(|cell| cell.block.first_fragment > 0.0)
        .map(|cell| {
            let offset = if include_alignment {
                cell.offset
            } else {
                cell.paint_offset.unwrap_or(cell.offset)
            };
            offset + cell.block.first_fragment
        })
        .min_by(f32::total_cmp)
        .unwrap_or(0.0)
}

/// The fragmentainer a flow breaks into, which selects the break values that apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Fragmentainer {
    #[default]
    Page,
    Column,
}
impl Fragmentainer {
    /// `column` and `avoid-column` apply only between columns; a page break also breaks the
    /// column (CSS Fragmentation 3 §3.1).
    pub fn between(self, value: BreakBetween) -> BreakBetween {
        match (self, value) {
            (Self::Page, BreakBetween::Column | BreakBetween::AvoidColumn) => BreakBetween::Auto,
            (Self::Column, BreakBetween::AvoidColumn) => BreakBetween::Avoid,
            (_, value) => value,
        }
    }
    /// `break-before` and `break-after` of `style` in this fragmentainer.
    pub fn edges(self, style: &Style) -> [BreakBetween; 2] {
        [style.break_before, style.break_after].map(|value| self.between(value))
    }
    /// Whether `break-inside` avoids breaks into this fragmentainer.
    pub fn keeps(self, style: &Style) -> bool {
        match style.keep {
            BreakInside::Auto => false,
            BreakInside::Avoid => true,
            BreakInside::AvoidColumn => self == Self::Column,
        }
    }
}

/// A forced break; page constraints also break columns, and later side constraints win.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Forced {
    Column,
    Page,
    Left,
    Right,
    Recto,
    Verso,
}
impl Forced {
    /// The forced break of a resolved break value.
    pub fn of(value: BreakBetween) -> Option<Self> {
        match value {
            BreakBetween::Page => Some(Self::Page),
            BreakBetween::Column => Some(Self::Column),
            BreakBetween::Left => Some(Self::Left),
            BreakBetween::Right => Some(Self::Right),
            BreakBetween::Recto => Some(Self::Recto),
            BreakBetween::Verso => Some(Self::Verso),
            BreakBetween::Auto | BreakBetween::Avoid | BreakBetween::AvoidColumn => None,
        }
    }
    pub fn is_page(self) -> bool {
        self != Self::Column
    }
    pub fn side(self, rtl: bool) -> Option<ferri_pdf_style::PageSide> {
        self.value().page_side(rtl)
    }
    pub fn combine(earlier: Option<Self>, later: Option<Self>) -> Option<Self> {
        Self::of(
            earlier
                .map_or(BreakBetween::Auto, Self::value)
                .combine(later.map_or(BreakBetween::Auto, Self::value)),
        )
    }
    fn value(self) -> BreakBetween {
        match self {
            Self::Column => BreakBetween::Column,
            Self::Page => BreakBetween::Page,
            Self::Left => BreakBetween::Left,
            Self::Right => BreakBetween::Right,
            Self::Recto => BreakBetween::Recto,
            Self::Verso => BreakBetween::Verso,
        }
    }
}

struct BreakInfo {
    height: f32,
    first_fragment: f32,
    before: BreakBetween,
    after: BreakBetween,
    end_boxes: usize,
    end_space: f32,
}
#[derive(Clone, Copy, Default)]
pub(super) struct BreakTail {
    pub height: f32,
    pub end_boxes: usize,
    pub end_space: f32,
}
impl FlowItem<'_> {
    fn break_info(&self) -> Option<BreakInfo> {
        match self {
            Self::Image { height, .. } => Some(BreakInfo {
                height: *height,
                first_fragment: *height,
                before: BreakBetween::Auto,
                after: BreakBetween::Auto,
                end_boxes: 0,
                end_space: 0.0,
            }),
            Self::Block(block) | Self::Spanner(block) => Some(BreakInfo {
                height: block.height,
                first_fragment: block.first_fragment,
                before: block.break_before,
                after: block.break_after,
                end_boxes: 0,
                end_space: 0.0,
            }),
            Self::Columns(columns) => Some(BreakInfo {
                height: columns.height,
                first_fragment: columns.first_fragment,
                before: columns.break_before,
                after: columns.break_after,
                end_boxes: 0,
                end_space: 0.0,
            }),
            Self::Paragraph(paragraph) => {
                let count = if paragraph.lines.len()
                    < paragraph
                        .style
                        .orphans
                        .saturating_add(paragraph.style.widows)
                {
                    paragraph.lines.len()
                } else {
                    paragraph.style.orphans
                };
                Some(BreakInfo {
                    height: paragraph.height,
                    first_fragment: paragraph
                        .lines
                        .iter()
                        .take(count)
                        .map(|line| line.height)
                        .sum(),
                    before: BreakBetween::Auto,
                    after: BreakBetween::Auto,
                    end_boxes: 0,
                    end_space: 0.0,
                })
            }
            Self::Row(row) => Some(BreakInfo {
                height: row.height,
                first_fragment: row.first_fragment,
                before: row.break_before,
                after: row.break_after,
                end_boxes: 0,
                end_space: 0.0,
            }),
            Self::Margin(_)
            | Self::Space(_)
            | Self::Edge(_, _)
            | Self::Positioned(_)
            | Self::Events(_) => None,
        }
    }
}

/// Break values propagated from the first and last child to the box's own resolved `edges`.
pub(super) fn boundaries(
    items: &[FlowItem<'_>],
    edges: [BreakBetween; 2],
) -> (BreakBetween, BreakBetween) {
    let before = items
        .iter()
        .find_map(FlowItem::break_info)
        .map(|info| info.before);
    let after = items
        .iter()
        .rev()
        .find_map(FlowItem::break_info)
        .map(|info| info.after);
    (
        edges[0].combine(before.unwrap_or(BreakBetween::Auto)),
        edges[1].combine(after.unwrap_or(BreakBetween::Auto)),
    )
}

pub(super) fn tail_heights(items: &[FlowItem<'_>], outer_tail: BreakTail) -> Vec<BreakTail> {
    let mut tails = vec![BreakTail::default(); items.len()];
    let mut next: Option<BreakInfo> = None;
    let mut gap = 0.0;
    let mut space = 0.0;
    let mut end_boxes = 0;
    let mut end_space = 0.0;
    for (index, item) in items.iter().enumerate().rev() {
        let Some(mut info) = item.break_info() else {
            gap += item.height();
            if let FlowItem::Space(value) | FlowItem::Edge(_, value) = item {
                space += value;
            }
            if let FlowItem::Edge(crate::measure::BlockEdge::End, value) = item {
                end_boxes += 1;
                end_space += value;
            }
            continue;
        };
        let tail = match &next {
            _ if info.after.is_forced() => BreakTail::default(),
            None if outer_tail.height > 0.0 || end_boxes > 0 => BreakTail {
                // A zero closing edge does not keep the root's trailing margin.
                height: if outer_tail.height > 0.0 || end_space > 0.0 {
                    gap + outer_tail.height
                } else {
                    space
                },
                end_boxes: end_boxes + outer_tail.end_boxes,
                end_space: end_space + outer_tail.end_space,
            },
            Some(next)
                if !next.before.is_forced()
                    && (info.after == BreakBetween::Avoid
                        || next.before == BreakBetween::Avoid) =>
            {
                BreakTail {
                    height: gap + next.first_fragment,
                    end_boxes: next.end_boxes,
                    end_space: next.end_space,
                }
            }
            _ => BreakTail::default(),
        };
        tails[index] = tail;
        if info.first_fragment >= info.height {
            info.first_fragment += tail.height;
            info.end_boxes = tail.end_boxes;
            info.end_space = tail.end_space;
        }
        next = Some(info);
        gap = 0.0;
        space = 0.0;
        end_boxes = 0;
        end_space = 0.0;
    }
    tails
}

pub(super) fn first_fragment(items: &[FlowItem<'_>]) -> f32 {
    let tails = tail_heights(items, BreakTail::default());
    let mut gap = 0.0;
    for (index, item) in items.iter().enumerate() {
        let Some(info) = item.break_info() else {
            gap += item.height();
            continue;
        };
        if info.height == 0.0 && info.first_fragment == 0.0 {
            continue;
        }
        let tail = if info.first_fragment >= info.height {
            tails[index].height
        } else {
            0.0
        };
        return gap + info.first_fragment + tail;
    }
    gap
}
