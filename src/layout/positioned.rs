//! Absolutely positioned boxes: CSS 2.1 §10.3.7 widths and §10.6.4 heights.
use crate::document::BlockBox;
use crate::graphics::Rect;
use crate::layout::LayoutError;
use crate::layout::geometry::{HorizontalBox, SizeConstraints};
use crate::layout::measure::{MeasuredBlock, Measurer};
use crate::style::{Alignment, Style};

/// Hypothetical normal-flow position: margin-box edges `left`/`right` and top edge `y`.
#[derive(Clone, Copy, Debug)]
pub(super) struct StaticPosition {
    pub page: usize,
    pub left: f32,
    pub right: f32,
    pub y: f32,
    pub alignment: Option<StaticAlignment>,
}

/// Alignment of a positioned child in its parent’s static-position rectangle.
#[derive(Clone, Copy, Debug)]
pub(super) struct StaticAlignment {
    pub content: [f32; 2],
    pub row: bool,
    pub main_reverse: bool,
    pub cross_reverse: bool,
    pub justify: Alignment,
    pub align: Alignment,
}
impl StaticAlignment {
    fn offset(free: f32, alignment: Alignment, reverse: bool) -> f32 {
        let (start, end) = if reverse { (free, 0.0) } else { (0.0, free) };
        match alignment {
            Alignment::Center | Alignment::SpaceAround | Alignment::SpaceEvenly => free / 2.0,
            Alignment::End | Alignment::SelfEnd | Alignment::Right => free,
            Alignment::FlexEnd => end,
            Alignment::Start | Alignment::SelfStart | Alignment::Left => 0.0,
            Alignment::Auto
            | Alignment::Normal
            | Alignment::FlexStart
            | Alignment::Stretch
            | Alignment::Baseline
            | Alignment::SpaceBetween => start,
        }
    }
    /// Offsets of the margin box inside the container content box.
    fn offsets(&self, size: [f32; 2]) -> [f32; 2] {
        let free = [self.content[0] - size[0], self.content[1] - size[1]];
        let (main, cross) = if self.row { (0, 1) } else { (1, 0) };
        let mut offsets = [0.0; 2];
        offsets[main] = Self::offset(free[main], self.justify, self.main_reverse);
        offsets[cross] = Self::offset(free[cross], self.align, self.cross_reverse);
        offsets
    }
}

/// One axis of the absolute-position equation, in start-to-end order.
struct Axis {
    length: f32,
    insets: [Option<f32>; 2],
    margins: [Option<f32>; 2],
    edges: f32,
    static_start: f32,
    /// Horizontal auto margins may not go negative (§10.3.7); vertical ones may.
    clamp_margins: bool,
}
struct Solved {
    /// Offset of the border box from the containing block start.
    start: f32,
    size: f32,
    margins: [f32; 2],
}
impl Axis {
    fn solve(&self, size: Option<f32>, auto_size: &mut dyn FnMut(f32) -> f32) -> Solved {
        let [start, end] = self.insets;
        let length = self.length;
        let edges = self.edges;
        let zero = |margin: Option<f32>| margin.unwrap_or(0.0);
        let (start, size, margins) = match (start, size, end) {
            (None, None, None) => {
                let margins = self.margins.map(zero);
                let start = self.static_start;
                let size = auto_size(length - start - margins[0] - margins[1] - edges);
                (start, size, margins)
            }
            (Some(start), Some(size), Some(end)) => {
                let remainder = length - start - end - size - edges;
                let margins = match self.margins {
                    [None, None] => {
                        let half = remainder / 2.0;
                        if self.clamp_margins && half < 0.0 {
                            [0.0, remainder]
                        } else {
                            [half, half]
                        }
                    }
                    [None, Some(after)] => [remainder - after, after],
                    [Some(before), None] => [before, remainder - before],
                    [Some(before), Some(after)] => [before, after],
                };
                (start, size, margins)
            }
            (None, None, Some(end)) => {
                let margins = self.margins.map(zero);
                let size = auto_size(length - end - margins[0] - margins[1] - edges);
                (
                    length - end - margins[0] - margins[1] - edges - size,
                    size,
                    margins,
                )
            }
            (None, Some(size), None) => (self.static_start, size, self.margins.map(zero)),
            (Some(start), None, None) => {
                let margins = self.margins.map(zero);
                let size = auto_size(length - start - margins[0] - margins[1] - edges);
                (start, size, margins)
            }
            (None, Some(size), Some(end)) => {
                let margins = self.margins.map(zero);
                (
                    length - end - margins[0] - margins[1] - edges - size,
                    size,
                    margins,
                )
            }
            (Some(start), None, Some(end)) => {
                let margins = self.margins.map(zero);
                let size = (length - start - end - margins[0] - margins[1] - edges).max(0.0);
                (start, size, margins)
            }
            (Some(start), Some(size), None) => (start, size, self.margins.map(zero)),
        };
        Solved {
            start: start + margins[0],
            size,
            margins,
        }
    }
    /// Tentative size, then the rules again with max- and min-size as the computed size.
    fn constrained(
        &self,
        constraints: &SizeConstraints,
        auto_size: &mut dyn FnMut(f32) -> f32,
    ) -> Solved {
        let tentative = self.solve(constraints.specified, auto_size);
        let size = constraints.constrain(tentative.size);
        if size == tentative.size {
            tentative
        } else {
            self.solve(Some(size), auto_size)
        }
    }
}

/// Measures and places `block` against containing block padding box `cb`, unbroken.
/// Returns the measured box and the y of its border-box top.
pub(super) fn place<'a>(
    measurer: &mut Measurer<'_>,
    block: &'a BlockBox,
    cb: Rect,
    rtl: bool,
    position: &StaticPosition,
) -> Result<(MeasuredBlock<'a>, f32), LayoutError> {
    let unbroken = std::mem::replace(&mut measurer.unbroken, true);
    let placed = solve(measurer, block, cb, rtl, position);
    measurer.unbroken = unbroken;
    placed
}

fn solve<'a>(
    measurer: &mut Measurer<'_>,
    block: &'a BlockBox,
    cb: Rect,
    rtl: bool,
    position: &StaticPosition,
) -> Result<(MeasuredBlock<'a>, f32), LayoutError> {
    let style: &Style = block.style();
    let width = cb.width;
    let height = cb.height;
    let table = if style.display.is_table() {
        Some(crate::layout::tables::TablePlan::new(
            measurer,
            block,
            Some(height),
            None,
            crate::layout::geometry::horizontal_with_edges,
        )?)
    } else {
        None
    };
    let edges = table.as_ref().map_or_else(
        || crate::layout::geometry::Edges::of(style, width),
        |plan| plan.edges(width),
    );
    let padding = edges.padding;
    let border = edges.border;
    let margin = style.margin.each_ref().map(|m| m.resolve(width));
    let [top, right, bottom, left] = &style.inset;
    let horizontal_edges = padding[1] + padding[3] + border[1] + border[3];
    let vertical_edges = padding[0] + padding[2] + border[0] + border[2];
    let mut width_constraints = SizeConstraints::new(
        [&style.width, &style.min_width, &style.max_width],
        Some(width),
        style.box_sizing,
        horizontal_edges,
    );
    // CSS Sizing 4 §4.2 and §4.4: a definite height gives an automatic width through the
    // preferred aspect ratio; otherwise the height's limits transfer into the width's.
    if table.is_none() {
        if let Some(used) =
            crate::layout::ratio::width_from_height(measurer, block, width, Some(height), &edges)?
        {
            width_constraints.specified = Some(used);
        } else {
            crate::layout::ratio::transfer_to_width(
                style,
                &mut width_constraints,
                Some(height),
                &edges,
            );
        }
    }
    let static_left = position.left - cb.x;
    let static_right = cb.x + width - position.right;
    let (insets, margins, static_start) = if rtl {
        (
            [right.resolve(width), left.resolve(width)],
            [margin[1], margin[3]],
            static_right,
        )
    } else {
        (
            [left.resolve(width), right.resolve(width)],
            [margin[3], margin[1]],
            static_left,
        )
    };
    let mut horizontal = Axis {
        length: width,
        insets,
        margins,
        edges: horizontal_edges,
        static_start,
        clamp_margins: true,
    };
    let shrinks = width_constraints.specified.is_none() && insets.iter().any(Option::is_none);
    let intrinsic = if shrinks
        || [&style.width, &style.min_width, &style.max_width]
            .iter()
            .any(|size| matches!(size, crate::style::Size::Intrinsic(_)))
    {
        Some(crate::layout::intrinsic::content(block, measurer, width)?)
    } else {
        None
    };
    if let Some(natural) = intrinsic {
        let available = width
            - horizontal_edges
            - insets[0].unwrap_or(if insets[1].is_none() {
                static_start
            } else {
                0.0
            })
            - insets[1].unwrap_or(0.0)
            - margins[0].unwrap_or(0.0)
            - margins[1].unwrap_or(0.0);
        width_constraints = width_constraints.with_intrinsic(
            [&style.width, &style.min_width, &style.max_width],
            natural,
            Some(available.max(0.0)),
        );
    }
    let mut shrink_to_fit = |available: f32| {
        intrinsic.map_or(available.max(0.0), |sizes| {
            sizes.max.min(sizes.min.max(available))
        })
    };
    let aligned_start = |solved: &Solved| {
        position.alignment.as_ref().map(|alignment| {
            let outer = solved.size + horizontal_edges + solved.margins[0] + solved.margins[1];
            let offset = alignment.offsets([outer, 0.0])[0];
            if rtl {
                static_right + (alignment.content[0] - outer - offset)
            } else {
                static_left + offset
            }
        })
    };
    let mut solved = horizontal.constrained(&width_constraints, &mut shrink_to_fit);
    if let Some(start) = aligned_start(&solved) {
        horizontal.static_start = start;
        solved = horizontal.constrained(&width_constraints, &mut shrink_to_fit);
    }
    let box_geometry = |solved: &Solved| {
        let border_left = if rtl {
            width - solved.start - solved.size - horizontal_edges
        } else {
            solved.start
        };
        let [margin_start, margin_end] = solved.margins;
        let (margin_left, margin_right) = if rtl {
            (margin_end, margin_start)
        } else {
            (margin_start, margin_end)
        };
        HorizontalBox {
            containing_x: cb.x,
            containing_width: width,
            containing_rtl: rtl,
            x: cb.x + border_left + border[3] + padding[3],
            width: solved.size,
            padding,
            border,
            margin: [margin[0], Some(margin_right), margin[2], Some(margin_left)],
        }
    };
    let mut geometry = box_geometry(&solved);
    let table = if let Some(plan) = table {
        let resolved = plan.resolve_positioned(measurer, &geometry)?;
        let actual_width = resolved.geometry.width;
        let resolved = if actual_width == solved.size {
            resolved
        } else {
            solved = horizontal.solve(Some(actual_width), &mut shrink_to_fit);
            if let Some(start) = aligned_start(&solved) {
                horizontal.static_start = start;
                solved = horizontal.solve(Some(actual_width), &mut shrink_to_fit);
            }
            geometry = box_geometry(&solved);
            plan.resolve_positioned(measurer, &geometry)?
        };
        Some(plan.measure_wrapped(measurer, &resolved)?.0)
    } else {
        None
    };
    let is_table = table.is_some();
    let mut height_constraints = SizeConstraints::new(
        [&style.height, &style.min_height, &style.max_height],
        Some(height),
        style.box_sizing,
        vertical_edges,
    );
    // An automatic height takes the ratio-derived height, grown to the content (CSS Sizing 4
    // §4.2–4.3), over the inset-modified containing block's.
    let ratio = !is_table
        && height_constraints.specified.is_none()
        && crate::layout::ratio::PreferredRatio::of_box(style, &edges).is_some();
    let ratio_measured = if ratio {
        Some(measurer.format_block(block, geometry.clone(), false, Some(height), None)?)
    } else {
        None
    };
    if let Some(measured) = &ratio_measured {
        height_constraints.specified = Some((measured.height - vertical_edges).max(0.0));
        height_constraints.minimum = 0.0;
        height_constraints.maximum = None;
    }
    let mut vertical = Axis {
        length: height,
        insets: [top.resolve(height), bottom.resolve(height)],
        margins: [margin[0], margin[2]],
        edges: vertical_edges,
        static_start: position.y - cb.y,
        clamp_margins: false,
    };
    let natural = if table.is_some() {
        table
    } else if ratio_measured.is_some() {
        ratio_measured
    } else if height_constraints.specified.is_none() && vertical.insets.iter().any(Option::is_none)
    {
        Some(measurer.format_block(block, geometry.clone(), false, Some(height), None)?)
    } else {
        None
    };
    let natural_height = natural
        .as_ref()
        .map(|measured| measured.height - vertical_edges);
    let mut content_height = |_: f32| natural_height.unwrap_or(0.0);
    if let Some(alignment) = &position.alignment {
        let size = vertical
            .constrained(&height_constraints, &mut content_height)
            .size;
        let outer = size + vertical_edges + margin[0].unwrap_or(0.0) + margin[2].unwrap_or(0.0);
        vertical.static_start += alignment.offsets([0.0, outer])[1];
    }
    let placed = if is_table {
        vertical.solve(natural_height, &mut content_height)
    } else {
        vertical.constrained(&height_constraints, &mut content_height)
    };
    let measured = match natural {
        Some(measured) if Some(placed.size) == natural_height => measured,
        _ => measurer.format_block(block, geometry, false, Some(height), Some(placed.size))?,
    };
    Ok((measured, cb.y + placed.start))
}
