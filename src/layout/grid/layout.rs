use super::{
    Plan,
    items::{Baselines, Item},
};
use crate::document::{BlockBox, BlockContent};
use crate::grid::{AvailableSpace, MaxSizing, MinSizing, SizeBounds, SizingItem};
use crate::layout::measure::BlockSource;
use crate::layout::{
    LayoutError,
    geometry::{HorizontalBox, VerticalBox},
    margin::MarginStrut,
    measure::{
        BlockEdge, FlowItem, ItemMetrics, MeasuredBlock, MeasuredCell, MeasuredRow, Measurer,
        PositionedFlow,
    },
};
use crate::style::{Alignment, BreakBetween, COLUMNS, Overflow, ROWS};

pub(crate) fn measure<'a>(
    measurer: &mut Measurer<'_>,
    block: impl Into<BlockSource<'a>>,
    geometry: &HorizontalBox,
    containing_height: Option<f32>,
    used_height: Option<f32>,
) -> Result<MeasuredBlock<'a>, LayoutError> {
    let block = block.into();
    let style = block.style();
    let top = geometry.padding[0] + geometry.border[0];
    let bottom = geometry.padding[2] + geometry.border[2];
    let inherited_rows = super::sizing::shared(style, ROWS)
        && measurer
            .grid_parent
            .as_ref()
            .is_some_and(|parent| parent.matches(block.block));
    let heights =
        super::sizing::constraints(style, ROWS, containing_height, top + bottom, inherited_rows);
    let row_bounds = SizeBounds {
        min: heights.minimum,
        max: heights.maximum,
    };
    let vertical = VerticalBox::new(style, geometry, containing_height)
        .with_constraints(heights)
        .with_used_height(used_height);
    let definite = vertical.definite();
    let plan = Plan::new(
        block,
        measurer,
        [definite, Some(geometry.width)],
        [row_bounds, SizeBounds::default()],
    )?;
    let columns = std::rc::Rc::new(plan.sizes(
        COLUMNS,
        Some(geometry.width),
        &plan.column_contributions(
            measurer,
            geometry.width,
            plan.gap(COLUMNS, Some(geometry.width)),
        )?,
        AvailableSpace::Definite(geometry.width),
        matches!(style.flex.justify, Alignment::Normal | Alignment::Stretch),
        measurer,
    )?);
    let row_sizing = plan.topology.axes[ROWS].sizing(definite);
    let intrinsic_rows: Vec<_> = row_sizing
        .iter()
        .map(|track| {
            if matches!(track.max, MaxSizing::Flex(_)) && definite.is_some() {
                matches!(track.min, MinSizing::MinContent | MinSizing::MaxContent)
            } else {
                !matches!(
                    (track.min, track.max),
                    (MinSizing::Fixed(_), MaxSizing::Fixed(_))
                )
            }
        })
        .collect();
    let cyclic_tracks: Vec<_> = plan
        .topology
        .placement
        .areas
        .iter()
        .map(|area| {
            intrinsic_rows[area[ROWS].clone()]
                .iter()
                .any(|value| *value)
        })
        .collect();
    let fixed_heights: Vec<_> = plan
        .topology
        .placement
        .areas
        .iter()
        .map(|area| {
            super::tracks::fixed_extent(
                &row_sizing[area[ROWS].clone()],
                &plan.topology.axes[ROWS].collapsed[area[ROWS].clone()],
                plan.gap(ROWS, definite),
            )
        })
        .collect();
    let mut baselines = Baselines::new(plan.topology.placement.tracks[ROWS]);
    if inherited_rows
        && let Some(groups) = plan
            .inherited
            .as_ref()
            .and_then(|parent| parent.baseline_groups(plan.gap(ROWS, definite)))
    {
        baselines.groups = groups;
    }
    let mut descendant_baselines = Vec::new();
    let mut prepared = Vec::with_capacity(plan.children.len());
    let mut contributions = Vec::with_capacity(plan.children.len());
    let mut descendants = Vec::new();
    let mut parent = geometry.clone();
    parent.containing_rtl = style.rtl;
    for (index, (source, area)) in plan
        .children
        .iter()
        .zip(&plan.topology.placement.areas)
        .enumerate()
    {
        let mut item = Item::new(
            source,
            columns.area(area[COLUMNS].clone()),
            &parent,
            [style.flex.align_items, style.grid.justify_items],
            fixed_heights[index],
            measurer,
        )?;
        item.inherit(plan.child_context(index), [None, Some(columns.clone())]);
        let measured = measurer.with_grid_parent(item.parent.clone(), |measurer| {
            measurer.item_metrics(
                source,
                &item.geometry,
                None,
                fixed_heights[index],
                |measurer| item.measure(measurer, fixed_heights[index], None),
            )
        })?;
        baselines.push(
            area[ROWS].start,
            item.baseline(&measured, cyclic_tracks[index]),
        );
        let constraints = item.height(fixed_heights[index]);
        let margins = item.margins[0].unwrap_or(0.0) + item.margins[2].unwrap_or(0.0);
        let shared = super::sizing::shared(source.style(), ROWS);
        if let Some(rows) = &measured.grid_rows {
            let sides = item.row_edges();
            let parent_gap = plan.gap(ROWS, definite);
            let child_gap = source.style().flex.gap[ROWS].resolve_normal(0.0, parent_gap);
            if let Some(items) = &measured.grid_baselines {
                measurer.work.spend(items.len().saturating_mul(4).max(1))?;
                let sizes: Vec<_> = items.iter().map(|item| item.contribution.clone()).collect();
                let projected = crate::grid::subgrid_contributions(
                    &sizes,
                    area[ROWS].clone(),
                    false,
                    sides,
                    child_gap - parent_gap,
                )?;
                for (source, contribution) in items.iter().zip(projected) {
                    let edge = if source.contribution.tracks.start == 0 {
                        sides[0]
                    } else {
                        (child_gap - parent_gap) / 2.0
                    };
                    let baseline = source.baseline + edge;
                    let row = contribution.tracks.start;
                    baselines.groups[row] = baselines.groups[row].max(baseline);
                    descendant_baselines.push((
                        index,
                        super::BaselineContribution {
                            contribution,
                            baseline,
                        },
                    ));
                }
            }
            descendants.push((
                index,
                item.project_rows(rows, area[ROWS].clone(), parent_gap, measurer)?,
            ));
        }
        let outer = if shared {
            item.vertical_edges()
        } else {
            measured.height
        } + margins;
        contributions.push(SizingItem {
            tracks: area[ROWS].clone(),
            min_content: outer,
            max_content: outer,
            minimum: if constraints.specified.is_some() {
                outer
            } else {
                constraints.minimum + item.vertical_edges() + margins
            },
            content_minimum: (!shared
                && source
                    .style()
                    .min_height
                    .is_auto_block_size(source.image().is_some())
                && matches!(
                    source.style().overflow[1],
                    Overflow::Visible | Overflow::Clip
                ))
            .then_some(outer),
        });
        prepared.push(item);
    }
    let mut baseline_items = Vec::new();
    for (index, contribution) in contributions.iter_mut().enumerate() {
        super::items::add_baseline_shim(contribution, baselines.offset(index));
        let (row, baseline) = baselines.items[index];
        if baseline.is_some() {
            baseline_items.push(super::BaselineContribution {
                contribution: contribution.clone(),
                baseline: baselines.groups[row],
            });
        }
    }
    let baseline_groups: std::rc::Rc<[f32]> = std::rc::Rc::from(baselines.groups.clone());
    let mut realign = vec![false; prepared.len()];
    for (index, mut item) in descendant_baselines {
        let target = baselines.groups[item.contribution.tracks.start];
        realign[index] |= target > item.baseline;
        super::items::add_baseline_shim(&mut item.contribution, target - item.baseline);
        item.baseline = target;
        contributions.push(item.contribution.clone());
        baseline_items.push(item);
    }
    for (index, mut projected) in descendants {
        if realign[index] {
            let item = &mut prepared[index];
            item.parent = item
                .parent
                .clone()
                .map(|parent| parent.with_baselines(baseline_groups.clone()));
            let measured = item.measure(measurer, fixed_heights[index], None)?;
            let rows = measured
                .grid
                .as_ref()
                .and_then(|grid| grid.row_contributions.as_deref())
                .ok_or(LayoutError::InvalidGridStructure)?;
            projected = item.project_rows(
                rows,
                plan.topology.placement.areas[index][ROWS].clone(),
                plan.gap(ROWS, definite),
                measurer,
            )?;
        }
        contributions.extend(projected);
    }
    let baseline_contributions = inherited_rows.then(|| std::rc::Rc::from(baseline_items));
    let row_contributions = inherited_rows.then(|| std::rc::Rc::from(contributions.clone()));
    let rows = plan.sizes(
        ROWS,
        definite,
        &contributions,
        definite.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
        matches!(
            style.flex.align_content,
            Alignment::Normal | Alignment::Stretch
        ),
        measurer,
    )?;
    let height = vertical.used_height(top + rows.extent() + bottom);
    if !height.is_finite() {
        return Err(LayoutError::NonFiniteBlockGeometry {
            block: measurer.sources.block(block.block),
        });
    }
    let content = (height - top - bottom).max(0.0);
    let rows = if definite.is_none() {
        plan.sizes(
            ROWS,
            Some(content),
            &contributions,
            AvailableSpace::Definite(content),
            matches!(
                style.flex.align_content,
                Alignment::Normal | Alignment::Stretch
            ),
            measurer,
        )?
    } else {
        rows
    };
    let rows = std::rc::Rc::new(rows);
    let constrained = definite.is_some() || rows.extent() > content;
    let mut cells = Vec::with_capacity(prepared.len());
    let mut final_baselines = Baselines::new(plan.topology.placement.tracks[ROWS]);
    final_baselines.groups.clone_from(&baselines.groups);
    for (index, (item, area)) in prepared
        .into_iter()
        .zip(&plan.topology.placement.areas)
        .enumerate()
    {
        let range = rows.area(area[ROWS].clone());
        let available = range.end - range.start;
        let mut item = if item.source.image().is_some() {
            Item::new(
                item.source,
                columns.area(area[COLUMNS].clone()),
                &parent,
                [style.flex.align_items, style.grid.justify_items],
                Some(available),
                measurer,
            )?
        } else {
            item
        };
        item.inherit(
            plan.child_context(index)
                .map(|parent| parent.with_baselines(baseline_groups.clone())),
            [Some(rows.clone()), Some(columns.clone())],
        );
        let measured = item.measure_area(measurer, available)?;
        final_baselines.push(
            area[ROWS].start,
            item.baseline(&ItemMetrics::of(&measured), cyclic_tracks[index]),
        );
        let offset = range.start + item.top(available, measured.height);
        cells.push(MeasuredCell {
            clipped_height: None,
            after_events: block.block.following_events(item.source).collect(),
            grid_cell: None,
            backgrounds: Vec::new(),
            block: measured,
            offset,
            paint_offset: None,
            stretch: false,
            after: item.margins[2].unwrap_or(0.0),
            flex_item: true,
        });
    }
    for (index, cell) in cells.iter_mut().enumerate() {
        cell.offset += final_baselines.offset(index);
    }
    let container_baselines = final_baselines.container(&cells, &plan.topology.placement.areas);
    let mut grid_rows = super::Rows {
        source_order: plan.source_order.clone(),
        tracks: plan.topology.axes[ROWS]
            .sizing(Some(content))
            .iter()
            .enumerate()
            .map(|(index, track)| super::fragmentation::Row {
                area: rows.area(index..index + 1),
                first_fragment: 0.0,
                kept_fragment: 0.0,
                grows: !matches!(track.min, MinSizing::Fixed(_))
                    || (definite.is_none() && matches!(track.max, MaxSizing::Flex(_))),
                edges: [BreakBetween::Auto; 2],
            })
            .collect(),
        items: Default::default(),
    };
    for (cell, area) in cells.iter().zip(&plan.topology.placement.areas) {
        let range = area[ROWS].clone();
        grid_rows.items.insert(
            std::ptr::from_ref::<BlockBox>(cell.block.source),
            range.clone(),
        );
        let first = &mut grid_rows.tracks[range.start];
        first.first_fragment = first.first_fragment.max(
            (cell.offset - first.area.start + cell.block.first_fragment)
                .min(first.area.end - first.area.start),
        );
        first.edges[0] = first.edges[0].combine(cell.block.break_before);
        let last = &mut grid_rows.tracks[range.end - 1].edges[1];
        *last = last.combine(cell.block.break_after);
    }
    grid_rows.resolve_keeps();
    let row_edges = [
        grid_rows
            .tracks
            .first()
            .map_or(BreakBetween::Auto, |row| row.edges[0]),
        grid_rows
            .tracks
            .last()
            .map_or(BreakBetween::Auto, |row| row.edges[1]),
    ];
    let has_forced = cells.iter().any(|cell| cell.block.has_forced)
        || grid_rows
            .tracks
            .iter()
            .any(|row| row.edges.iter().any(|edge| edge.is_forced()));
    let mut items = vec![FlowItem::Edge(BlockEdge::Start, top)];
    items.extend(
        block
            .block
            .leading_events()
            .map(|event| FlowItem::Events(std::slice::from_ref(event))),
    );
    for child in block.children() {
        if let BlockContent::Positioned(child) = child {
            items.push(FlowItem::Positioned(PositionedFlow {
                block: child,
                left: geometry.x,
                right: geometry.x + geometry.width,
                margin: 0.0,
                alignment: Some(crate::layout::positioned::StaticAlignment {
                    content: [geometry.width, content],
                    row: true,
                    main_reverse: false,
                    cross_reverse: false,
                    justify: super::items::physical_alignment(
                        if child.style().grid.justify_self == Alignment::Auto {
                            style.grid.justify_items
                        } else {
                            child.style().grid.justify_self.in_flow()
                        },
                        style.rtl,
                        child.style().rtl,
                    ),
                    align: super::items::physical_alignment(
                        if child.style().flex.align_self == Alignment::Auto {
                            style.flex.align_items
                        } else {
                            child.style().flex.align_self.in_flow()
                        },
                        false,
                        false,
                    ),
                }),
            }));
        }
    }
    let first_fragment = grid_rows.tracks.first().map_or(0.0, |row| {
        row.area.start + row.required((measurer.page_height - top - bottom).max(0.0))
    });
    items.push(FlowItem::Row(Box::new(MeasuredRow {
        grid: Some(std::rc::Rc::new(grid_rows)),
        baselines: Some(container_baselines),
        collapsed: Vec::new(),
        background_rows: Vec::new(),
        cell_heights: Vec::new(),
        height: content,
        first_fragment,
        paints: Vec::new(),
        targets: Vec::new(),
        cells,
        break_before: row_edges[0],
        break_after: row_edges[1],
        band: None,
    })));
    let first_fragment = crate::layout::breaks::first_fragment(&items) + bottom;
    items.push(FlowItem::Edge(BlockEdge::End, bottom));
    let overflow_height = crate::layout::measure::overflow_height(&items).max(height);
    let first_fragment = if measurer.fragmentainer.keeps(style)
        && !has_forced
        && overflow_height <= measurer.page_height
    {
        overflow_height
    } else {
        first_fragment
    };
    let edges = measurer.fragmentainer.edges(style);
    let (break_before, break_after) = crate::layout::breaks::boundaries(style, &items, edges);
    let item_indices = plan
        .children
        .iter()
        .enumerate()
        .map(|(index, source)| (std::ptr::from_ref::<BlockBox>(*source), index))
        .collect();
    let grid = Some(std::rc::Rc::new(super::Geometry {
        row_contributions,
        baseline_contributions,
        baseline_groups,
        plan,
        rows,
        columns: std::cell::RefCell::new([(geometry.width.to_bits(), columns)].into()),
        items: item_indices,
    }));
    Ok(MeasuredBlock {
        grid,
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
        clearance_before: None,
        clearance_target: measurer
            .clearance_targets
            .get(&std::ptr::from_ref::<BlockBox>(block.block))
            .copied(),
        through: false,
        height,
        sizing: None,
        constrained,
        independent: true,
        overflow_height,
        first_fragment,
        has_forced,
        break_before,
        break_after,
        items,
        paint: measurer.box_paint(block, geometry),
        navigation: block.navigation(),
        end_space: bottom,
        start_space: top,
        repeat_space: 0.0,
        marker: None,
        table: None,
        continues: false,
        spanner_fragment: None,
    })
}
