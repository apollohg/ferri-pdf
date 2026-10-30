use crate::layout::measure::{BlockSource, Failure};
pub(super) mod intrinsic;
use crate::document::{BlockBox, BlockContent};
use crate::flex::{Item, spacing};
use crate::layout::{
    LayoutError,
    geometry::{HorizontalBox, SizeConstraints, VerticalBox},
    intrinsic as block_intrinsic,
    margin::MarginStrut,
    measure::{
        BlockEdge, CollapsedItem, FlowItem, ItemMetrics as Metrics, MeasuredBlock, MeasuredCell,
        MeasuredRow, Measurer, PositionedFlow,
    },
    positioned::StaticAlignment,
};
use crate::style::{
    Alignment, BoxSizing, BreakBetween, Display, FlexBasis, FlexWrap, OverflowPosition, Size,
};

/// The flex base size (CSS Flexbox 1 §9.2.3): `content` and unresolvable sizes use the
/// content size `content`, and `auto` the main size property's `constraints`.
pub(super) fn base_size(
    basis: &FlexBasis,
    constraints: &SizeConstraints,
    main_size: Option<f32>,
    box_sizing: BoxSizing,
    edges: f32,
    natural: crate::style::IntrinsicWidths,
    content: f32,
) -> f32 {
    let FlexBasis::Size(size) = basis else {
        return content;
    };
    if *size.basis() == Size::Auto {
        return SizeConstraints::new(
            [size, &Size::Auto, &Size::Auto],
            main_size,
            box_sizing,
            edges,
        )
        .preferred(constraints.preferred(content));
    }
    let sizes = [size, &Size::Auto, &Size::Auto];
    SizeConstraints::new(sizes, main_size, box_sizing, edges)
        .with_intrinsic(sizes, natural, main_size.map(|v| (v - edges).max(0.0)))
        .preferred(content)
}

fn item_metrics<'a>(
    measurer: &mut Measurer<'a>,
    item: &FlexItem<'a>,
    parent: &HorizontalBox,
    width: f32,
    height: Option<f32>,
    containing_height: Option<f32>,
) -> Result<Metrics, Failure> {
    let geometry = item_geometry(item, parent, 0.0, width);
    let mut space = measurer.space;
    space.indefinite_inline &= !item.inline_definite;
    measurer.with_space(space, |measurer| {
        measurer.item_metrics(
            item.source,
            &geometry,
            height,
            containing_height,
            |measurer| {
                measure_item(
                    measurer,
                    item,
                    parent,
                    0.0,
                    width,
                    height,
                    containing_height,
                )
            },
        )
    })
}

struct FlexItem<'a> {
    source: &'a BlockBox,
    containing_rtl: bool,
    main: Item,
    cross: SizeConstraints,
    padding: [f32; 4],
    border: [f32; 4],
    margins: [Option<f32>; 4],
    align: Alignment,
    align_overflow: OverflowPosition,
    natural_cross: f32,
    natural_main: f32,
    inline_definite: bool,
}
impl FlexItem<'_> {
    fn edges(&self, column: bool) -> f32 {
        if column {
            self.padding[0] + self.padding[2] + self.border[0] + self.border[2]
        } else {
            self.padding[1] + self.padding[3] + self.border[1] + self.border[3]
        }
    }
}
fn children(block: &BlockBox) -> Result<Vec<&BlockBox>, Failure> {
    block
        .in_flow_children()
        .into_iter()
        .filter_map(|child| match child {
            BlockContent::Block(block) => Some(Ok(block)),
            BlockContent::Positioned(_) | BlockContent::Events(_) => None,
            BlockContent::Inline(_) => Some(Err(LayoutError::UnsupportedFlex {
                feature: "unnormalized inline item",
            }
            .into())),
        })
        .collect()
}
pub(super) fn alignment(value: Alignment, reverse: bool) -> Alignment {
    match (value, reverse) {
        (Alignment::Start, true) => Alignment::End,
        (Alignment::End, true) => Alignment::Start,
        (Alignment::Normal | Alignment::Auto | Alignment::Stretch, _) => Alignment::Start,
        _ => value,
    }
}
fn item_geometry(
    item: &FlexItem<'_>,
    parent: &HorizontalBox,
    left: f32,
    width: f32,
) -> HorizontalBox {
    HorizontalBox {
        containing_x: parent.x,
        containing_width: parent.width,
        containing_rtl: item.containing_rtl,
        x: parent.x + left + item.padding[3] + item.border[3],
        width,
        padding: item.padding,
        border: item.border,
        margin: [Some(0.0); 4],
    }
}
fn measure_item<'a>(
    measurer: &mut Measurer<'a>,
    item: &FlexItem<'a>,
    parent: &HorizontalBox,
    left: f32,
    width: f32,
    height: Option<f32>,
    containing_height: Option<f32>,
) -> Result<MeasuredBlock<'a>, Failure> {
    if item.source.style().display == Display::Table {
        return Err(LayoutError::UnsupportedFlex {
            feature: "table flex item sizing",
        }
        .into());
    }
    let mut space = measurer.space;
    space.indefinite_inline &= !item.inline_definite;
    measurer.with_space(space, |measurer| {
        measurer.measure_format(
            item.source,
            item_geometry(item, parent, left, width),
            true,
            containing_height,
            height,
        )
    })
}

pub(super) fn measure<'a>(
    measurer: &mut Measurer<'a>,
    block: impl Into<BlockSource<'a>>,
    geometry: &HorizontalBox,
    containing_height: Option<f32>,
    used_height: Option<f32>,
) -> Result<MeasuredBlock<'a>, Failure> {
    let block = block.into();
    let style = block.style();
    let flex = &style.flex;
    let column = flex.direction.column();
    let main_reverse = flex.direction.reverse() ^ (!column && style.used_rtl());
    let cross_reverse = (column && style.used_rtl()) ^ (flex.wrap == FlexWrap::WrapReverse);
    let vertical = measurer.vertical_box(
        block,
        VerticalBox::new(style, geometry, containing_height).with_used_height(used_height),
    );
    let content_height = vertical.definite();
    let main_size = if column {
        content_height
    } else {
        (!measurer.space.indefinite_inline).then_some(geometry.width)
    };
    let cross_size = if column {
        (!measurer.space.indefinite_inline).then_some(geometry.width)
    } else {
        content_height
    };
    let main_gap = flex.gap[usize::from(!column)].resolve(main_size.unwrap_or(0.0));
    let cross_gap = flex.gap[usize::from(column)].resolve(cross_size.unwrap_or(0.0));
    let main_sides = if column { [0, 2] } else { [3, 1] };
    let cross_sides = if column { [3, 1] } else { [0, 2] };
    let mut data = Vec::new();
    for source in children(block.block)? {
        let child = source.style();
        let natural = block_intrinsic::content(source, measurer, geometry.width)?;
        let edges = measurer.box_edges(source, geometry.width)?;
        let padding = edges.padding;
        let border = edges.border;
        let margins = child.margin.each_ref().map(|v| v.resolve(geometry.width));
        let width_edges = padding[1] + padding[3] + border[1] + border[3];
        let height_edges = padding[0] + padding[2] + border[0] + border[2];
        let available_width =
            (geometry.width - width_edges - margins[1].unwrap_or(0.0) - margins[3].unwrap_or(0.0))
                .max(0.0);
        let [widths, heights] = if source.image().is_some() {
            crate::layout::images::constraints(source, geometry.width, content_height)
        } else {
            [
                SizeConstraints::new(
                    [&child.width, &child.min_width, &child.max_width],
                    Some(geometry.width),
                    child.box_sizing,
                    width_edges,
                )
                .with_intrinsic(
                    [&child.width, &child.min_width, &child.max_width],
                    natural,
                    Some(available_width),
                ),
                SizeConstraints::new(
                    [&child.height, &child.min_height, &child.max_height],
                    content_height,
                    child.box_sizing,
                    height_edges,
                ),
            ]
        };
        let (align, align_overflow) = child.flex.used_align_self(flex);
        let preliminary_width = widths.constrain(widths.preferred(
            if matches!(align, Alignment::Normal | Alignment::Stretch) {
                available_width
            } else {
                natural.max.min(available_width).max(natural.min)
            },
        ));
        let mut item = FlexItem {
            source,
            containing_rtl: style.used_rtl(),
            main: Item {
                collapsed: false,
                basis: 0.0,
                minimum: 0.0,
                maximum: None,
                grow: child.flex.grow,
                shrink: child.flex.shrink,
                edges: if column { height_edges } else { width_edges },
                margins: main_sides.map(|side| margins[side]),
            },
            cross: if column { widths } else { heights },
            padding,
            border,
            margins,
            align,
            align_overflow,
            natural_cross: preliminary_width,
            natural_main: 0.0,
            inline_definite: false,
        };
        // CSS Flexbox 1 §9.8: a stretched item of a single-line container with a definite
        // cross size has a definite cross size.
        let ratio = source
            .image()
            .is_none()
            .then(|| {
                crate::layout::ratio::PreferredRatio::of_box(
                    child,
                    &crate::layout::geometry::Edges { padding, border },
                )
            })
            .flatten();
        let definite_cross = (!column).then_some(()).and_then(|()| {
            item.cross
                .specified
                .map(|height| item.cross.constrain(height))
                .or_else(|| {
                    (flex.wrap == FlexWrap::NoWrap
                        && matches!(align, Alignment::Normal | Alignment::Stretch)
                        && margins[0].is_some()
                        && margins[2].is_some())
                    .then_some(content_height)
                    .flatten()
                    .map(|height| {
                        item.cross.constrain(
                            (height
                                - height_edges
                                - margins[0].unwrap_or(0.0)
                                - margins[2].unwrap_or(0.0))
                            .max(0.0),
                        )
                    })
                })
        });
        let natural_main = if column {
            let measured = item_metrics(
                measurer,
                &item,
                geometry,
                preliminary_width,
                None,
                content_height,
            )?;
            (measured.height - height_edges).max(0.0)
        } else if let (Some(ratio), Some(cross)) = (ratio, definite_cross) {
            // §9.2.3 B: the flex base size from the definite cross size and the ratio.
            ratio.width(cross)
        } else {
            natural.max
        };
        let constraints = SizeConstraints::new(
            if column {
                [&child.height, &child.min_height, &child.max_height]
            } else {
                [&child.width, &child.min_width, &child.max_width]
            },
            main_size,
            child.box_sizing,
            item.main.edges,
        );
        let main_natural = if column {
            crate::style::IntrinsicWidths {
                min: natural_main,
                max: natural_main,
            }
        } else {
            natural
        };
        let constraints = constraints.with_intrinsic(
            if column {
                [&child.height, &child.min_height, &child.max_height]
            } else {
                [&child.width, &child.min_width, &child.max_width]
            },
            main_natural,
            main_size.map(|v| {
                (v - item.main.edges
                    - item.main.margins[0].unwrap_or(0.0)
                    - item.main.margins[1].unwrap_or(0.0))
                .max(0.0)
            }),
        );
        // §4.5: a non-replaced item's content size suggestion is clamped by the cross limits
        // through its ratio, and its transferred size suggestion converts a definite cross size.
        let auto_min = match ratio {
            Some(ratio) if !column => {
                let content = item
                    .cross
                    .maximum
                    .map_or(natural.min, |maximum| natural.min.min(ratio.width(maximum)));
                let content = if *child.min_height.basis() == Size::Auto {
                    content
                } else {
                    content.max(ratio.width(item.cross.minimum))
                };
                item.cross.specified.map_or(content, |height| {
                    content.max(ratio.width(item.cross.constrain(height)))
                })
            }
            _ if column => natural_main,
            _ => natural.min,
        };
        let min_main = if column {
            &child.min_height
        } else {
            &child.min_width
        };
        let minimum = if *min_main.basis() == Size::Auto {
            constraints.automatic_minimum(
                constraints
                    .maximum
                    .map_or(auto_min, |v| auto_min.min(v))
                    .min(constraints.specified.unwrap_or(f32::INFINITY)),
            )
        } else {
            constraints.minimum
        };
        let basis = base_size(
            &child.flex.basis,
            &constraints,
            main_size,
            child.box_sizing,
            item.main.edges,
            main_natural,
            natural_main,
        );
        let basis_definite = match &child.flex.basis {
            FlexBasis::Size(size) if *size.basis() == Size::Auto => {
                let preferred = if column { &child.height } else { &child.width };
                preferred.resolve_definite(main_size).is_some()
                    || (ratio.is_some() && definite_cross.is_some())
            }
            FlexBasis::Size(size) => size.resolve_definite(main_size).is_some(),
            FlexBasis::Content => false,
        };
        item.inline_definite = !measurer.space.indefinite_inline
            || if column {
                child.width.resolve_definite(None).is_some()
                    || (cross_size.is_some()
                        && flex.wrap == FlexWrap::NoWrap
                        && matches!(align, Alignment::Normal | Alignment::Stretch)
                        && margins[1].is_some()
                        && margins[3].is_some())
                    || (ratio.is_some() && (main_size.is_some() || basis_definite))
            } else {
                main_size.is_some() || basis_definite
            };
        item.natural_main = natural_main;
        item.main.basis = basis;
        item.main.minimum = minimum;
        item.main.maximum = constraints.maximum;
        if main_reverse {
            item.main.margins.swap(0, 1);
        }
        data.push(item);
    }
    let mut numeric: Vec<_> = data.iter().map(|item| item.main).collect();
    struct FlexLine {
        range: std::ops::Range<usize>,
        resolved: crate::flex::Line,
        measured: Vec<Metrics>,
        cross: f32,
        baseline: f32,
    }
    let mut struts: Option<Vec<f32>> = None;
    let (lines, used_main, used_cross) = loop {
        measurer.work.spend(numeric.len().max(1))?;
        let ranges =
            crate::flex::line_ranges(&numeric, main_size, main_gap, flex.wrap != FlexWrap::NoWrap)?;
        let mut lines = Vec::new();
        let mut used_main = main_size.unwrap_or(0.0);
        for range in ranges {
            let natural_main: f32 = range
                .clone()
                .filter(|&i| !numeric[i].collapsed)
                .map(|i| {
                    data[i]
                        .main
                        .basis
                        .max(data[i].main.minimum)
                        .min(data[i].main.maximum.unwrap_or(f32::INFINITY))
                        + data[i].main.edges
                        + data[i].main.margins.into_iter().flatten().sum::<f32>()
                })
                .sum::<f32>()
                + main_gap
                    * range
                        .clone()
                        .filter(|&i| !numeric[i].collapsed)
                        .count()
                        .saturating_sub(1) as f32;
            let main = main_size.unwrap_or(natural_main);
            used_main = used_main.max(main);
            measurer.work.spend(range.len().max(1))?;
            let resolved = crate::flex::resolve_line(
                &numeric[range.clone()],
                main,
                main_gap,
                alignment(flex.justify, flex.direction.reverse()),
                flex.justify_overflow,
            )?;
            let mut measured = Vec::new();
            let mut cross = 0.0f32;
            let mut baseline = 0.0f32;
            let mut descent = 0.0f32;
            for (local, index) in range.clone().enumerate() {
                let item = &data[index];
                if numeric[index].collapsed {
                    cross = cross.max(struts.as_ref().expect("second-pass struts")[index]);
                    measured.push(Metrics {
                        grid_rows: None,
                        grid_baselines: None,
                        height: 0.0,
                        baseline: None,
                    });
                    continue;
                }
                let size = resolved.sizes[local];
                let (width, height) = if column {
                    (item.natural_cross, Some(size))
                } else {
                    (size, None)
                };
                let child = item_metrics(measurer, item, geometry, width, height, content_height)?;
                let outer_cross = if column {
                    width + item.edges(false)
                } else {
                    child.height
                };
                let margins = cross_sides.map(|side| item.margins[side].unwrap_or(0.0));
                cross = cross.max(outer_cross + margins[0] + margins[1]);
                if !column
                    && item.align == Alignment::Baseline
                    && cross_sides.iter().all(|&side| item.margins[side].is_some())
                {
                    let first = child.baseline_from_start(cross_reverse);
                    let start = usize::from(cross_reverse);
                    baseline = baseline.max(first + margins[start]);
                    descent = descent.max(child.height - first + margins[1 - start]);
                }
                measured.push(child);
            }
            cross = cross.max(baseline + descent);
            lines.push(FlexLine {
                range,
                resolved,
                measured,
                cross,
                baseline,
            });
        }
        let natural_cross = lines.iter().map(|line| line.cross).sum::<f32>()
            + cross_gap * lines.len().saturating_sub(1) as f32;
        let used_cross = if column {
            geometry.width
        } else {
            (vertical.used_height(
                natural_cross
                    + geometry.padding[0]
                    + geometry.padding[2]
                    + geometry.border[0]
                    + geometry.border[2],
            ) - geometry.padding[0]
                - geometry.padding[2]
                - geometry.border[0]
                - geometry.border[2])
                .max(0.0)
        };
        if flex.wrap == FlexWrap::NoWrap && !lines.is_empty() {
            lines[0].cross = used_cross;
        } else if matches!(flex.align_content, Alignment::Normal | Alignment::Stretch)
            && used_cross > natural_cross
            && !lines.is_empty()
        {
            let extra = (used_cross - natural_cross) / lines.len() as f32;
            for line in &mut lines {
                line.cross += extra;
            }
        }
        if struts.is_some()
            || !data
                .iter()
                .any(|item| item.source.style().visibility == crate::style::Visibility::Collapse)
        {
            break (lines, used_main, used_cross);
        }
        let mut sizes = vec![0.0; data.len()];
        for line in &lines {
            for index in line.range.clone() {
                if data[index].source.style().visibility == crate::style::Visibility::Collapse {
                    numeric[index].collapsed = true;
                    sizes[index] = line.cross;
                }
            }
        }
        struts = Some(sizes);
    };
    let total_cross = lines.iter().map(|line| line.cross).sum::<f32>()
        + cross_gap * lines.len().saturating_sub(1) as f32;
    let (mut cross_cursor, cross_between) = spacing(
        alignment(flex.align_content, flex.wrap == FlexWrap::WrapReverse),
        flex.align_content_overflow,
        used_cross - total_cross,
        lines.len(),
    );
    let top = geometry.padding[0] + geometry.border[0];
    let bottom = geometry.padding[2] + geometry.border[2];
    let mut placed = Vec::new();
    let mut column_collapsed = Vec::new();
    let mut groups = Vec::new();
    for line in lines {
        let mut collapsed = Vec::new();
        let line_start = if cross_reverse {
            used_cross - cross_cursor - line.cross
        } else {
            cross_cursor
        };
        for ((local, index), preliminary) in line.range.clone().enumerate().zip(line.measured) {
            let item = &data[index];
            if numeric[index].collapsed {
                let start = line.resolved.starts[local];
                let y = if main_reverse {
                    used_main - start
                } else {
                    start
                };
                let mut event = CollapsedItem {
                    source: item.source,
                    offset: if column { y } else { 0.0 },
                    after_events: block.block.following_events(item.source).collect(),
                };
                if column && flex.wrap == FlexWrap::NoWrap {
                    event.offset = 0.0;
                    groups.push((y, 0.0, Vec::new(), vec![event]));
                } else if column {
                    column_collapsed.push(event);
                } else {
                    collapsed.push(event);
                }
                continue;
            }
            let main = line.resolved.sizes[local];
            let mut cross = if column {
                item.natural_cross
            } else {
                (preliminary.height - item.edges(true)).max(0.0)
            };
            let margins = cross_sides.map(|side| item.margins[side]);
            let auto = margins.iter().filter(|m| m.is_none()).count();
            let edges = item.edges(!column);
            if matches!(item.align, Alignment::Normal | Alignment::Stretch)
                && item.cross.specified.is_none()
                && auto == 0
            {
                cross = item.cross.constrain(item.cross.preferred(
                    (line.cross - edges - margins.into_iter().flatten().sum::<f32>()).max(0.0),
                ));
            }
            let free = line.cross - cross - edges - margins.into_iter().flatten().sum::<f32>();
            let physical_start = usize::from(cross_reverse);
            let margin_start = margins[physical_start].unwrap_or(if free > 0.0 {
                free / auto.max(1) as f32
            } else {
                0.0
            });
            let offset = if auto > 0 {
                if free < 0.0 && margins[physical_start].is_none() {
                    0.0
                } else {
                    margin_start
                }
            } else if item.align == Alignment::Baseline && !column {
                line.baseline - preliminary.baseline_from_start(cross_reverse)
            } else {
                margin_start
                    + spacing(
                        alignment(item.align, flex.wrap == FlexWrap::WrapReverse),
                        item.align_overflow,
                        free,
                        1,
                    )
                    .0
            };
            let cross_start = line_start
                + if cross_reverse {
                    line.cross - offset - cross - edges
                } else {
                    offset
                };
            let main_start = if main_reverse {
                used_main - line.resolved.starts[local] - main - item.edges(column)
            } else {
                line.resolved.starts[local]
            };
            let (left, y, width, height) = if column {
                (cross_start, main_start, cross, Some(main))
            } else {
                (main_start, cross_start, main, Some(cross))
            };
            let stretch = !column
                && matches!(item.align, Alignment::Normal | Alignment::Stretch)
                && item.cross.specified.is_none()
                && auto == 0;
            let mut measured = measure_item(
                measurer,
                item,
                geometry,
                left,
                width,
                height,
                content_height,
            )?;
            // A preferred aspect ratio makes an automatic cross size definite.
            let ratio = crate::layout::ratio::PreferredRatio::of_box(
                item.source.style(),
                &crate::layout::geometry::Edges {
                    padding: item.padding,
                    border: item.border,
                },
            );
            if (!column
                && item.cross.specified.is_none()
                && item.cross.maximum.is_none()
                && (ratio.is_none() || stretch))
                || (column
                    && item
                        .source
                        .style()
                        .height
                        .is_auto_block_size(item.source.image().is_some())
                    && main == item.natural_main
                    && ratio.is_none())
            {
                measured.constrained = false;
            }
            let cell = MeasuredCell {
                clipped_height: None,
                after_events: block.block.following_events(item.source).collect(),
                grid_cell: None,
                backgrounds: Vec::new(),
                paint_offset: None,
                flex_item: true,
                block: measured,
                offset: if column { y } else { y - line_start },
                stretch: stretch && free <= 0.0,
                after: if column {
                    0.0
                } else {
                    item.margins[2].unwrap_or(0.0)
                },
            };
            if column && flex.wrap == FlexWrap::NoWrap {
                let extent = cell.block.height;
                groups.push((
                    y,
                    extent,
                    vec![MeasuredCell {
                        offset: 0.0,
                        ..cell
                    }],
                    Vec::new(),
                ));
            } else {
                placed.push(cell);
            }
        }
        if !column {
            groups.push((
                line_start,
                line.cross,
                std::mem::take(&mut placed),
                collapsed,
            ));
        }
        cross_cursor += line.cross + cross_gap + cross_between;
    }
    let content = if column { used_main } else { used_cross };
    let height = vertical.used_height(top + content + bottom);
    if !height.is_finite() {
        return Err(LayoutError::UnsupportedFlex {
            feature: "non-finite container size",
        }
        .into());
    }
    if column && (!placed.is_empty() || !column_collapsed.is_empty()) {
        groups.push((0.0, content, placed, column_collapsed));
    }
    groups.sort_by(|a, b| a.0.total_cmp(&b.0));
    let has_forced = groups
        .iter()
        .flat_map(|(_, _, cells, _)| cells)
        .any(|item| item.block.has_forced);
    let mut items = vec![FlowItem::Edge(BlockEdge::Start, top)];
    items.extend(
        block
            .block
            .leading_events()
            .map(|event| FlowItem::Events(std::slice::from_ref(event))),
    );
    for child in block.children() {
        if let BlockContent::Positioned(child) = child {
            let (align, align_overflow) = child.style().flex.used_align_self(flex);
            items.push(FlowItem::Positioned(PositionedFlow {
                block: child,
                left: geometry.x,
                right: geometry.x + geometry.width,
                margin: 0.0,
                alignment: Some(StaticAlignment {
                    content: [geometry.width, content],
                    row: !column,
                    main_reverse,
                    cross_reverse,
                    justify: flex.justify,
                    justify_overflow: flex.justify_overflow,
                    align,
                    align_overflow,
                }),
            }));
        }
    }
    let mut cursor = 0.0;
    for (start, extent, cells, collapsed) in groups {
        if start != cursor {
            items.push(FlowItem::Space(start - cursor));
        }
        let first_fragment = cells
            .iter()
            .map(|item| item.offset + item.block.first_fragment)
            .fold(0.0f32, f32::max)
            .min(extent);
        items.push(FlowItem::Row(Box::new(MeasuredRow {
            grid: None,
            baselines: None,
            collapsed,
            background_rows: Vec::new(),
            cell_heights: Vec::new(),
            height: extent,
            first_fragment,
            cells,
            paints: Vec::new(),
            after: Vec::new(),
            targets: Vec::new(),
            break_before: BreakBetween::Auto,
            break_after: BreakBetween::Auto,
            band: None,
        })));
        cursor = start + extent;
    }
    if cursor != content {
        items.push(FlowItem::Space(content - cursor));
    }
    let first_fragment = crate::layout::breaks::first_fragment(&items);
    items.push(FlowItem::Edge(BlockEdge::End, bottom));
    let overflow_height = crate::layout::measure::overflow_height(&items).max(height);
    let edges = measurer.fragmentainer.edges(style);
    let first_fragment = if measurer.fragmentainer.keeps(style)
        && !has_forced
        && overflow_height <= measurer.page_height
    {
        overflow_height
    } else {
        first_fragment + bottom
    };
    Ok(MeasuredBlock {
        frame_key: None,
        frame_owner: None,
        math: None,
        grid: None,
        clearance_before: None,
        clearance_target: measurer
            .clearance_targets
            .get(&std::ptr::from_ref::<BlockBox>(block.block))
            .copied(),
        source: block.block,
        geometry: geometry.clone(),
        containing_height,
        style,
        canvas_source: block.is_canvas_source(),
        offset: style.relative_offset(
            geometry.containing_width,
            containing_height,
            geometry.containing_rtl,
        ),
        before: MarginStrut::edge(block.block, 0, geometry.containing_width),
        after: MarginStrut::edge(block.block, 2, geometry.containing_width),
        through: false,
        height,
        sizing: None,
        constrained: vertical.definite().is_some() || style.containment().size[1],
        independent: true,
        overflow_height,
        first_fragment,
        has_forced,
        break_before: edges[0],
        break_after: edges[1],
        paint: measurer.box_paint(block, geometry),
        navigation: block.navigation(),
        items,
        start_space: top,
        end_space: bottom,
        repeat_space: 0.0,
        marker: None,
        table: None,
        continues: false,
        spanner_fragment: None,
    })
}
