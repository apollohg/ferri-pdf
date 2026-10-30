//! Absolutely positioned boxes: CSS 2.1 §10.3.7 widths and §10.6.4 heights.
use crate::document::BlockBox;
use crate::graphics::Rect;
use crate::layout::geometry::{HorizontalBox, SizeConstraints};
use crate::layout::measure::Failure;
use crate::layout::measure::{MeasuredBlock, Measurer};
use crate::style::{Alignment, OverflowPosition, Style};

/// Hypothetical normal-flow position: margin-box edges `left`/`right` and top edge `y`.
#[derive(Clone, Copy, Debug)]
pub(super) struct StaticPosition {
    pub page: usize,
    pub left: f32,
    pub right: f32,
    pub y: f32,
    pub bottom: f32,
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
    pub justify_overflow: OverflowPosition,
    pub align: Alignment,
    pub align_overflow: OverflowPosition,
}
impl StaticAlignment {
    pub(super) fn mapped(mut self, [a, b, c, d, _, _]: [f32; 6]) -> Self {
        let reversed = if self.row {
            [a + b < 0.0, c + d < 0.0]
        } else {
            [c + d < 0.0, a + b < 0.0]
        };
        if a == 0.0 && d == 0.0 {
            self.content.swap(0, 1);
            self.row = !self.row;
        }
        let opposite = |alignment| match alignment {
            Alignment::Start => Alignment::End,
            Alignment::End => Alignment::Start,
            Alignment::SelfStart => Alignment::SelfEnd,
            Alignment::SelfEnd => Alignment::SelfStart,
            Alignment::Left => Alignment::Right,
            Alignment::Right => Alignment::Left,
            other => other,
        };
        self.main_reverse ^= reversed[0];
        self.cross_reverse ^= reversed[1];
        if reversed[0] {
            self.justify = opposite(self.justify);
        }
        if reversed[1] {
            self.align = opposite(self.align);
        }
        self
    }
    fn offset(free: f32, alignment: Alignment, overflow: OverflowPosition, reverse: bool) -> f32 {
        let (start, end) = if reverse { (free, 0.0) } else { (0.0, free) };
        if overflow.starts(free) {
            return start;
        }
        match alignment {
            Alignment::Center
            | Alignment::AnchorCenter
            | Alignment::SpaceAround
            | Alignment::SpaceEvenly => free / 2.0,
            Alignment::End | Alignment::SelfEnd | Alignment::Right => free,
            Alignment::FlexEnd => end,
            // The safe end fallback of last-baseline alignment (CSS Box Alignment 3 §4.2).
            Alignment::LastBaseline if free < 0.0 => start,
            Alignment::LastBaseline => end,
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
        offsets[main] = Self::offset(
            free[main],
            self.justify,
            self.justify_overflow,
            self.main_reverse,
        );
        offsets[cross] = Self::offset(
            free[cross],
            self.align,
            self.align_overflow,
            self.cross_reverse,
        );
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
    /// The space a size between auto insets fills, where it is not the space from the static
    /// position to the end edge: an inset-modified containing block derived from the
    /// static-position rectangle (CSS Position 3 §3.5.1).
    static_space: Option<f32>,
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
    fn static_space(&self) -> f32 {
        self.static_space.unwrap_or(self.length - self.static_start)
    }
    fn solve(&self, size: Option<f32>, auto_size: &mut dyn FnMut(f32) -> f32) -> Solved {
        let [start, end] = self.insets;
        let length = self.length;
        let edges = self.edges;
        let zero = |margin: Option<f32>| margin.unwrap_or(0.0);
        let (start, size, margins) = match (start, size, end) {
            (None, None, None) => {
                let margins = self.margins.map(zero);
                let start = self.static_start;
                let size = auto_size(self.static_space() - margins[0] - margins[1] - edges);
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
        let size = constraints.constrain(constraints.preferred(tentative.size));
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
    measurer: &mut Measurer<'a>,
    block: &'a BlockBox,
    cb: Rect,
    rtl: bool,
    position: &StaticPosition,
    page: Option<usize>,
) -> Result<(MeasuredBlock<'a>, f32), Failure> {
    let unbroken = std::mem::replace(&mut measurer.unbroken, true);
    let base = block.style();
    let styles = page
        .and_then(|page| {
            base.anchor
                .pages
                .iter()
                .find(|(candidate, _)| *candidate == page)
                .map(|(_, style)| style)
        })
        .unwrap_or(base);
    let placed = (|| {
        if styles.anchor.options.is_empty() {
            solve(measurer, block, styles, cb, rtl, position)
        } else {
            let mut options: Vec<_> = styles.anchor.options.iter().collect();
            if let Some(axis) = styles.anchor.order {
                options.sort_by(|a, b| {
                    let extent = |style| {
                        let rect = inset_modified(style, cb);
                        if axis.axis(block.layout_flow(), block.layout_flow())
                            == crate::style::anchor_values::HORIZONTAL
                        {
                            rect.width
                        } else {
                            rect.height
                        }
                    };
                    extent(b).total_cmp(&extent(a))
                });
            }
            let mut chosen = None;
            for option in options {
                let (measured, top) = solve(measurer, block, option, cb, rtl, position)?;
                let mut border = measured.geometry.border_box(measured.height);
                border.x += measured.offset[0];
                border.y = top;
                if !overflows(option, cb, border) {
                    chosen = Some((measured, top));
                    break;
                }
            }
            match chosen {
                Some(chosen) => Ok(chosen),
                None => solve(measurer, block, styles, cb, rtl, position),
            }
        }
    })();
    measurer.unbroken = unbroken;
    placed
}

fn area(style: &Style, original: Rect) -> Option<Rect> {
    let frame = style.anchor.frame.as_deref()?;
    let anchor = frame.anchor(&style.anchor.default_reference()?)?;
    let mut rect = style.anchor.area?.rect(
        [original.width, original.height],
        anchor.rect,
        crate::style::logical::Flow::used(style),
        frame.containing_flow,
    );
    rect.x += original.x;
    rect.y += original.y;
    Some(rect)
}

pub(super) fn inset_modified(style: &Style, original: Rect) -> Rect {
    let area = area(style, original).unwrap_or(original);
    let [top, right, bottom, left] = style
        .inset
        .each_ref()
        .map(|inset| inset.resolve(area.width).unwrap_or_default());
    // Block-axis percentages refer to the containing block height.
    let top = style.inset[0].resolve(area.height).unwrap_or(top);
    let bottom = style.inset[2].resolve(area.height).unwrap_or(bottom);
    Rect {
        x: area.x + left,
        y: area.y + top,
        width: area.width - left - right,
        height: area.height - top - bottom,
    }
}

pub(super) fn overflows(style: &Style, cb: Rect, border: Rect) -> bool {
    let bounds = inset_modified(style, cb);
    let margin = style
        .margin
        .each_ref()
        .map(|margin| margin.resolve(cb.width).unwrap_or_default());
    bounds.width < 0.0
        || bounds.height < 0.0
        || super::pagination::overflows(bounds.x, border.x - margin[3])
        || super::pagination::overflows(bounds.y, border.y - margin[0])
        || super::pagination::overflows(
            border.x + border.width + margin[1],
            bounds.x + bounds.width,
        )
        || super::pagination::overflows(
            border.y + border.height + margin[2],
            bounds.y + bounds.height,
        )
}

fn solve<'a>(
    measurer: &mut Measurer<'a>,
    block: &'a BlockBox,
    style: &'a Style,
    original: Rect,
    rtl: bool,
    position: &StaticPosition,
) -> Result<(MeasuredBlock<'a>, f32), Failure> {
    let anchor = style
        .anchor
        .frame
        .as_deref()
        .and_then(|frame| frame.anchor(&style.anchor.default_reference()?))
        .map(|anchor| anchor.rect);
    let area = area(style, original);
    let cb = area.unwrap_or(original);
    let static_position = StaticPosition {
        left: cb.x,
        right: cb.x + cb.width,
        y: cb.y,
        ..*position
    };
    let position = if area.is_some() {
        &static_position
    } else {
        position
    };
    let source = crate::layout::measure::BlockSource {
        block,
        selected: style,
    };
    let width = cb.width;
    let height = cb.height;
    let table = if style.display.is_table() {
        Some(crate::layout::tables::TablePlan::new(
            measurer,
            source,
            Some(height),
            None,
            crate::layout::geometry::horizontal_with_edges,
        )?)
    } else {
        None
    };
    let edges = if let Some(plan) = &table {
        plan.edges(width)
    } else {
        measurer.box_edges(source, width)?
    };
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
            crate::layout::ratio::width_from_height(measurer, source, width, Some(height), &edges)?
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
    // §3.5.1: end and center self-alignment between auto insets size the box in the
    // inset-modified containing block its static-position rectangle derives.
    let static_space =
        (area.is_none() && position.alignment.is_none() && insets.iter().all(Option::is_none))
            .then(|| static_self_alignment(style.grid.justify_self, !rtl))
            .flatten()
            .map(|alignment| {
                let [low, high] = static_containing(
                    [0.0, width],
                    [position.left - cb.x, position.right - cb.x],
                    alignment,
                    !rtl,
                );
                high - low
            });
    let mut horizontal = Axis {
        length: width,
        insets,
        margins,
        edges: horizontal_edges,
        static_start,
        static_space,
        clamp_margins: true,
    };
    let shrinks = width_constraints.specified.is_none() && insets.iter().any(Option::is_none);
    let intrinsic = if shrinks
        || [&style.width, &style.min_width, &style.max_width]
            .iter()
            .any(|size| size.intrinsic().is_some())
    {
        Some(crate::layout::intrinsic::content(source, measurer, width)?)
    } else {
        None
    };
    if let Some(natural) = intrinsic {
        let space = match insets {
            [None, None] => horizontal.static_space(),
            [start, end] => width - start.unwrap_or(0.0) - end.unwrap_or(0.0),
        };
        let available =
            space - horizontal_edges - margins[0].unwrap_or(0.0) - margins[1].unwrap_or(0.0);
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
            stretch_margins: [margin[0], margin[2]].map(|margin| margin.unwrap_or(0.0)),
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
    let heights = [&style.height, &style.min_height, &style.max_height];
    let mut height_constraints =
        SizeConstraints::new(heights, Some(height), style.box_sizing, vertical_edges).with_stretch(
            heights,
            Some(
                (height
                    - top.resolve(height).unwrap_or(0.0)
                    - bottom.resolve(height).unwrap_or(0.0)
                    - margin[0].unwrap_or(0.0)
                    - margin[2].unwrap_or(0.0)
                    - vertical_edges)
                    .max(0.0),
            ),
        );
    // An automatic height takes the ratio-derived height, grown to the content (CSS Sizing 4
    // §4.2–4.3), over the inset-modified containing block's.
    let ratio = !is_table
        && height_constraints.specified.is_none()
        && crate::layout::ratio::PreferredRatio::of_box(style, &edges).is_some();
    let ratio_measured = if ratio {
        Some(measurer.measure_format(source, geometry.clone(), false, Some(height), None)?)
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
        static_space: None,
        clamp_margins: false,
    };
    let natural = if table.is_some() {
        table
    } else if ratio_measured.is_some() {
        ratio_measured
    } else if height_constraints.specified.is_none() && vertical.insets.iter().any(Option::is_none)
    {
        Some(measurer.measure_format(source, geometry.clone(), false, Some(height), None)?)
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
    let mut measured = match natural {
        Some(measured) if Some(placed.size) == natural_height => measured,
        _ => measurer.measure_format(source, geometry, false, Some(height), Some(placed.size))?,
    };
    measured.style = style;
    let mut top = cb.y + placed.start;
    let mut border = measured.geometry.border_box(measured.height);
    border.y = top;
    let defaults = style.anchor.area.filter(|_| area.is_some()).map(|area| {
        area.alignment(
            crate::style::logical::Flow::used(style),
            style.anchor.frame.as_ref().map_or_else(
                || crate::style::logical::Flow {
                    rtl,
                    ..Default::default()
                },
                |frame| frame.containing_flow,
            ),
        )
    });
    for axis in [
        crate::style::anchor_values::HORIZONTAL,
        crate::style::anchor_values::VERTICAL,
    ] {
        use crate::style::anchor_values::AreaAlignment;
        let horizontal = axis == crate::style::anchor_values::HORIZONTAL;
        let (alignment, overflow) = if horizontal {
            (style.grid.justify_self, style.grid.justify_self_overflow)
        } else {
            (style.flex.align_self, style.flex.align_self_overflow)
        };
        let (insets, margins) = if horizontal {
            (
                [left.resolve(cb.width), right.resolve(cb.width)],
                [margin[3], margin[1]],
            )
        } else {
            (
                [style.inset[0].resolve(cb.height), bottom.resolve(cb.height)],
                [margin[0], margin[2]],
            )
        };
        // CSS Position 3 §4.3: a box with an inset in the axis and no auto margin aligns in
        // its inset-modified containing block, whose auto inset is zero (§3.5.1).
        let inset_aligned =
            insets.iter().any(Option::is_some) && margins.iter().all(Option::is_some);
        // §3.5.1: with both insets auto, end and center alignment derive the inset-modified
        // containing block from the static-position rectangle; start alignment is the static
        // position itself. Flex and grid static positions align by their container instead.
        let static_aligned =
            area.is_none() && position.alignment.is_none() && insets.iter().all(Option::is_none);
        // The physical side the axis's end edge lies on: right or bottom unless right-to-left.
        let end_high = !(horizontal && rtl);
        let alignment = match alignment {
            Alignment::Normal | Alignment::Auto => match defaults.map(|defaults| defaults[axis]) {
                Some(AreaAlignment::Start) => Alignment::Left,
                Some(AreaAlignment::End) => Alignment::Right,
                Some(AreaAlignment::Center) => Alignment::Center,
                Some(AreaAlignment::AnchorCenter) => Alignment::AnchorCenter,
                None => continue,
            },
            Alignment::AnchorCenter => Alignment::AnchorCenter,
            other if area.is_some() || inset_aligned => other,
            other if static_aligned => match static_self_alignment(other, end_high) {
                Some(alignment) => alignment,
                None => continue,
            },
            _ => continue,
        };
        let (start, length, size, before, after, inset_start, inset_end, original_edges) =
            if horizontal {
                (
                    cb.x,
                    cb.width,
                    border.width,
                    margin[3].unwrap_or_default(),
                    margin[1].unwrap_or_default(),
                    left.resolve(cb.width).unwrap_or_default(),
                    right.resolve(cb.width).unwrap_or_default(),
                    [original.x, original.x + original.width],
                )
            } else {
                (
                    cb.y,
                    cb.height,
                    border.height,
                    margin[0].unwrap_or_default(),
                    margin[2].unwrap_or_default(),
                    style.inset[0].resolve(cb.height).unwrap_or_default(),
                    bottom.resolve(cb.height).unwrap_or_default(),
                    [original.y, original.y + original.height],
                )
            };
        let (start, available) = if static_aligned {
            let [low, high] = static_containing(
                [start, start + length],
                if horizontal {
                    [position.left, position.right]
                } else {
                    [position.y, position.y]
                },
                alignment,
                end_high,
            );
            (low, high - low)
        } else {
            (start + inset_start, length - inset_start - inset_end)
        };
        let outer = size + before + after;
        let free = available - outer;
        let aligned = match alignment {
            Alignment::AnchorCenter if anchor.is_some() => {
                let anchor = anchor.expect("anchor-center has an anchor");
                let center = if horizontal {
                    original_edges[0] + anchor.x + anchor.width / 2.0
                } else {
                    original_edges[0] + anchor.y + anchor.height / 2.0
                };
                center - outer / 2.0
            }
            Alignment::Center | Alignment::AnchorCenter => start + free / 2.0,
            Alignment::Right | Alignment::End | Alignment::SelfEnd | Alignment::FlexEnd => {
                start + free
            }
            _ => start,
        };
        let target = before
            + if overflow == OverflowPosition::Unsafe {
                aligned
            } else {
                contained_start(
                    aligned,
                    outer,
                    [start, start + available],
                    [
                        original_edges[0].min(start),
                        original_edges[1].max(start + available),
                    ],
                    horizontal && rtl,
                )
            };
        if horizontal {
            measured.offset[0] += target - border.x;
        } else {
            top = target;
        }
    }
    Ok((measured, top))
}

/// The physical `Left`, `Right` or `Center` that `alignment` aligns a box with both insets
/// auto to in its static-position rectangle, whose end edge is the high one when `end_high`;
/// `None` where it aligns at the static position itself (CSS Position 3 §3.5.1).
fn static_self_alignment(alignment: Alignment, end_high: bool) -> Option<Alignment> {
    match alignment {
        Alignment::Center => Some(Alignment::Center),
        Alignment::Right if end_high => Some(Alignment::Right),
        Alignment::Left if !end_high => Some(Alignment::Left),
        Alignment::End | Alignment::SelfEnd | Alignment::FlexEnd => Some(if end_high {
            Alignment::Right
        } else {
            Alignment::Left
        }),
        _ => None,
    }
}

/// CSS Position 3 §3.5.1: the physical edges of the inset-modified containing block, inside
/// the containing block's `edges`, of a box with both insets auto aligned to the physical
/// `Left`, `Right` or `Center` of its static-position rectangle `rect`; the end edge is the
/// high one when `end_high`. A negative size grows back to zero from the end inset.
fn static_containing(
    edges: [f32; 2],
    rect: [f32; 2],
    alignment: Alignment,
    end_high: bool,
) -> [f32; 2] {
    match alignment {
        Alignment::Right => [edges[0], rect[1].max(edges[0])],
        Alignment::Left => [rect[0].min(edges[1]), edges[1]],
        _ => {
            let center = f32::midpoint(rect[0], rect[1]);
            let [low, high] = [(center - edges[0]).abs(), (edges[1] - center).abs()];
            let from_low = if end_high { low <= high } else { low < high };
            if from_low {
                [edges[0], edges[0] + 2.0 * low]
            } else {
                [edges[1] - 2.0 * high, edges[1]]
            }
        }
    }
}

/// CSS Box Alignment 3 §4.4.1.2: the default and `safe` margin-box start of an absolutely
/// positioned box of `outer` size aligned at `aligned`: kept inside the inset-modified
/// containing block's edges `inset` if it fits, else covering them within the overflow
/// limit rect's edges `limit`, else start-aligned (from the end edge when `end_start`).
fn contained_start(
    aligned: f32,
    outer: f32,
    inset: [f32; 2],
    limit: [f32; 2],
    end_start: bool,
) -> f32 {
    if outer <= inset[1] - inset[0] {
        aligned.max(inset[0]).min(inset[1] - outer)
    } else if outer <= limit[1] - limit[0] {
        aligned
            .max((inset[1] - outer).max(limit[0]))
            .min(inset[0].min(limit[1] - outer))
    } else if end_start {
        limit[1] - outer
    } else {
        limit[0]
    }
}
