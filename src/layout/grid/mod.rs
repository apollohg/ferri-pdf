use crate::layout::measure::{BlockSource, Failure};
mod context;
pub(crate) use context::ParentContext;
mod fragmentation;
mod items;
pub(super) use fragmentation::Rows;
pub(crate) use items::BaselineContribution;
mod layout;
mod sizing;
mod topology;
mod tracks;
pub(super) use layout::measure;

use crate::document::{BlockBox, BlockContent};
use crate::grid::{AvailableSpace, SizeBounds, SizingItem, size_tracks};
use crate::layout::{LayoutError, measure::Measurer};
use crate::style::{COLUMNS, ROWS};
use crate::text::IntrinsicWidths;
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use tracks::TrackGeometry;

const MAX_COLUMN_GEOMETRIES: usize = 16;

#[derive(Clone)]
struct Plan<'a> {
    source: &'a BlockBox,
    topology: Rc<topology::Topology>,
    inherited: Option<ParentContext>,
    children: Vec<&'a BlockBox>,
    source_order: Vec<usize>,
    bounds: [SizeBounds; 2],
    alignment: [(crate::style::Alignment, crate::style::OverflowPosition); 2],
    rtl: bool,
}

impl<'a> Plan<'a> {
    fn definite_columns(&self, index: usize, width: f32, measurer: &Measurer<'_>) -> bool {
        if !measurer.space.indefinite_inline {
            return true;
        }
        let area = self.topology.placement.areas[index][COLUMNS].clone();
        let tracks = self.topology.axes[COLUMNS].sizing(Some(width));
        tracks::fixed_extent(
            &tracks[area.clone()],
            &self.topology.axes[COLUMNS].collapsed[area],
            self.gap(COLUMNS, Some(width)),
        )
        .is_some()
    }

    fn new(
        block: BlockSource<'a>,
        measurer: &Measurer<'_>,
        sizes: [Option<f32>; 2],
        bounds: [SizeBounds; 2],
    ) -> Result<Self, Failure> {
        let parent = measurer
            .grid_parent
            .as_ref()
            .filter(|parent| parent.matches(block.block))
            .cloned();
        Self::with_parent(block, measurer, sizes, bounds, parent, false)
    }

    fn with_parent(
        block: BlockSource<'a>,
        measurer: &Measurer<'_>,
        sizes: [Option<f32>; 2],
        bounds: [SizeBounds; 2],
        inherited: Option<ParentContext>,
        empty: bool,
    ) -> Result<Self, Failure> {
        let style = block.style();
        let mut children = Vec::new();
        for child in block.children().iter().filter(|_| !empty) {
            match child {
                BlockContent::Block(child) => children.push((children.len(), child)),
                BlockContent::Inline(_) => return Err(LayoutError::InvalidGridStructure.into()),
                BlockContent::Positioned(_) | BlockContent::Events(_) => {}
            }
        }
        children.sort_by_key(|(_, child)| child.style().flex.order);
        let (source_order, children): (Vec<_>, Vec<_>) = children.into_iter().unzip();
        let placements: Vec<_> = children
            .iter()
            .map(|child| sizing::placement(child.style()))
            .collect();
        let parent = inherited.as_ref();
        let topology = Rc::new(if let Some(parent) = parent {
            topology::Topology::with_parent(
                style,
                &placements,
                sizes,
                bounds,
                &measurer.work,
                Some(parent.parent()),
            )?
        } else {
            topology::Topology::new(style, &placements, sizes, bounds, &measurer.work)?
        });
        Ok(Self {
            source: block.block,
            topology,
            inherited,
            children,
            source_order,
            bounds,
            alignment: [
                (style.flex.align_content, style.flex.align_content_overflow),
                (style.flex.justify, style.flex.justify_overflow),
            ],
            rtl: style.used_rtl(),
        })
    }

    fn child_context(&self, index: usize) -> Option<ParentContext> {
        let child = self.children[index];
        [ROWS, COLUMNS]
            .iter()
            .any(|axis| sizing::shared(child.style(), *axis))
            .then(|| ParentContext::new(child, self.topology.clone(), index))
    }

    fn gap(&self, axis: usize, basis: Option<f32>) -> f32 {
        let normal = self
            .inherited
            .as_ref()
            .filter(|_| self.topology.lines[axis].subgridded)
            .map_or(0.0, |parent| parent.gap(axis));
        self.source.style().flex.gap[axis].resolve_normal(basis.unwrap_or(0.0), normal)
    }

    fn sizes(
        &self,
        axis: usize,
        basis: Option<f32>,
        contributions: &[SizingItem],
        space: AvailableSpace,
        stretch: bool,
        measurer: &Measurer<'_>,
    ) -> Result<TrackGeometry, Failure> {
        if self.topology.lines[axis].subgridded
            && let Some(parent) = &self.inherited
        {
            let gap = match &self.source.style().flex.gap[axis] {
                crate::style::Gap::Normal => None,
                gap @ crate::style::Gap::Length(_) => Some(gap.resolve(basis.unwrap_or(0.0))),
            };
            if let Some(geometry) = parent.geometry(axis, gap, axis == COLUMNS && self.rtl)? {
                return Ok(geometry);
            }
        }
        let source = &self.topology.axes[axis];
        let gap = self.gap(axis, basis);
        let mut prefix = vec![0; source.tracks.len() + 1];
        for (index, collapsed) in source.collapsed.iter().enumerate() {
            prefix[index + 1] = prefix[index] + usize::from(!collapsed);
        }
        let sizing: Vec<_> = source
            .sizing(basis)
            .into_iter()
            .zip(&source.collapsed)
            .filter_map(|(track, collapsed)| (!collapsed).then_some(track))
            .collect();
        let contributions: Vec<_> = contributions
            .iter()
            .map(|item| SizingItem {
                tracks: prefix[item.tracks.start]..prefix[item.tracks.end],
                ..item.clone()
            })
            .collect();
        measurer
            .work
            .spend(sizing.len().saturating_mul(contributions.len().max(1)))?;
        let sized = size_tracks(
            &sizing,
            gap,
            &contributions,
            space,
            self.bounds[axis],
            stretch,
        )?;
        let sizes: Vec<_> = source
            .collapsed
            .iter()
            .enumerate()
            .map(|(index, collapsed)| {
                if *collapsed {
                    0.0
                } else {
                    sized[prefix[index]]
                }
            })
            .collect();
        Ok(TrackGeometry::new(
            &sizes,
            &source.collapsed,
            gap,
            match space {
                AvailableSpace::Definite(size) => Some(size),
                _ => None,
            },
            self.alignment[axis],
            axis == COLUMNS && self.rtl,
        ))
    }
    fn column_contributions(
        &self,
        measurer: &Measurer<'a>,
        available: f32,
        parent_gap: f32,
    ) -> Result<Vec<SizingItem>, Failure> {
        let mut output = Vec::new();
        for (index, (child, area)) in self
            .children
            .iter()
            .zip(&self.topology.placement.areas)
            .enumerate()
        {
            let style = child.style();
            let box_edges = measurer.box_edges(*child, 0.0)?;
            let edges = box_edges.horizontal();
            let margins = [style.margin[1].resolve(0.0), style.margin[3].resolve(0.0)]
                .into_iter()
                .flatten()
                .sum::<f32>();
            if sizing::shared(style, COLUMNS) {
                let plan = Self::with_parent(
                    (*child).into(),
                    measurer,
                    [None; 2],
                    [SizeBounds::default(); 2],
                    self.child_context(index),
                    false,
                )?;
                let child_gap = style.flex.gap[COLUMNS].resolve_normal(0.0, parent_gap);
                let contributions = plan.column_contributions(measurer, available, child_gap)?;
                let mut sides = [3, 1].map(|side| {
                    box_edges.padding[side]
                        + box_edges.border[side]
                        + style.margin[side].resolve(0.0).unwrap_or(0.0)
                });
                if style.used_rtl() {
                    sides.reverse();
                }
                measurer
                    .work
                    .spend(contributions.len().saturating_mul(4).max(1))?;
                output.extend(crate::grid::subgrid_contributions(
                    &contributions,
                    area[COLUMNS].clone(),
                    self.rtl != style.used_rtl(),
                    sides,
                    child_gap - parent_gap,
                )?);
                output.push(SizingItem {
                    tracks: area[COLUMNS].clone(),
                    min_content: edges + margins,
                    max_content: edges + margins,
                    minimum: edges + margins,
                    content_minimum: None,
                });
                continue;
            }
            let (outer, content_minimum) = if style.display.is_table() {
                let outer = crate::layout::tables::intrinsic(*child, measurer, available)?;
                (outer, (outer.min - edges - margins).max(0.0))
            } else {
                let natural = crate::layout::intrinsic::content(*child, measurer, available)?;
                (
                    crate::layout::intrinsic::outer_widths(style, natural, box_edges),
                    natural.min,
                )
            };
            let constraints =
                crate::layout::intrinsic::width_constraints(*child, measurer, None, edges, None)?;
            output.push(SizingItem {
                tracks: area[COLUMNS].clone(),
                min_content: outer.min,
                max_content: outer.max,
                minimum: if constraints.specified.is_some() {
                    outer.min
                } else {
                    constraints.minimum + edges + margins
                },
                content_minimum: (*style.min_width.basis() == crate::style::Size::Auto
                    && matches!(
                        style.overflow[0],
                        crate::style::Overflow::Visible | crate::style::Overflow::Clip
                    ))
                .then_some(
                    constraints.automatic_minimum(
                        constraints
                            .specified
                            .unwrap_or(content_minimum)
                            .min(constraints.maximum.unwrap_or(f32::INFINITY)),
                    ) + edges
                        + margins,
                ),
            });
        }
        Ok(output)
    }
}

/// Min- and max-content widths; a size-contained grid is sized as if it had no items.
pub(super) fn intrinsic<'a>(
    block: impl Into<BlockSource<'a>>,
    measurer: &Measurer<'a>,
    available: f32,
    contained: bool,
) -> Result<IntrinsicWidths, Failure> {
    let block = block.into();
    let bounds = [SizeBounds::default(); 2];
    let parent = measurer
        .grid_parent
        .as_ref()
        .filter(|parent| parent.matches(block.block))
        .cloned();
    let plan = Plan::with_parent(block, measurer, [None; 2], bounds, parent, contained)?;
    let contributions = plan.column_contributions(measurer, available, plan.gap(COLUMNS, None))?;
    let sizes = [AvailableSpace::MinContent, AvailableSpace::MaxContent].map(|space| {
        plan.sizes(COLUMNS, None, &contributions, space, false, measurer)
            .map(|tracks| tracks.extent())
    });
    let [min, max] = sizes;
    Ok(IntrinsicWidths {
        min: min?,
        max: max?,
    })
}

pub(super) struct Geometry<'a> {
    pub row_contributions: Option<Rc<[SizingItem]>>,
    pub baseline_contributions: Option<Rc<[BaselineContribution]>>,
    baseline_groups: Rc<[f32]>,
    plan: Plan<'a>,
    rows: Rc<TrackGeometry>,
    columns: RefCell<HashMap<u32, Rc<TrackGeometry>>>,
    items: HashMap<*const BlockBox, usize>,
}
impl<'a> Geometry<'a> {
    pub(crate) fn matches_parent(&self, parent: Option<&ParentContext>) -> bool {
        self.plan.inherited.as_ref() == parent
    }

    pub(crate) fn reparent(
        self: &Rc<Self>,
        parent: Option<ParentContext>,
        measurer: &Measurer<'_>,
    ) -> Result<Rc<Self>, Failure> {
        if self.matches_parent(parent.as_ref()) {
            return Ok(self.clone());
        }
        measurer.work.spend(self.plan.children.len().max(1))?;
        let mut plan = self.plan.clone();
        plan.inherited = parent;
        let rows = if plan.topology.lines[ROWS].subgridded {
            let basis = plan
                .inherited
                .as_ref()
                .and_then(|parent| parent.content_size(ROWS));
            Rc::new(plan.sizes(
                ROWS,
                basis,
                self.row_contributions.as_deref().unwrap_or_default(),
                basis.map_or(AvailableSpace::MaxContent, AvailableSpace::Definite),
                false,
                measurer,
            )?)
        } else {
            self.rows.clone()
        };
        Ok(Rc::new(Self {
            plan,
            rows,
            columns: RefCell::new(HashMap::new()),
            items: self.items.clone(),
            row_contributions: self.row_contributions.clone(),
            baseline_contributions: self.baseline_contributions.clone(),
            baseline_groups: self.baseline_groups.clone(),
        }))
    }

    fn columns(&self, width: f32, measurer: &Measurer<'a>) -> Result<Rc<TrackGeometry>, Failure> {
        if let Some(columns) = self.columns.borrow().get(&width.to_bits()) {
            return Ok(columns.clone());
        }
        let columns = Rc::new(self.plan.sizes(
            COLUMNS,
            Some(width),
            &self.plan.column_contributions(
                measurer,
                width,
                self.plan.gap(COLUMNS, Some(width)),
            )?,
            AvailableSpace::Definite(width),
            matches!(
                self.plan.source.style().flex.justify,
                crate::style::Alignment::Normal | crate::style::Alignment::Stretch
            ),
            measurer,
        )?);
        let mut cache = self.columns.borrow_mut();
        if cache.len() == MAX_COLUMN_GEOMETRIES {
            cache.clear();
        }
        cache.insert(width.to_bits(), columns.clone());
        Ok(columns)
    }

    pub(crate) fn item(
        &self,
        source: &'a BlockBox,
        parent: &crate::layout::geometry::HorizontalBox,
        measurer: &Measurer<'a>,
    ) -> Result<Option<(items::Item<'a>, f32)>, Failure> {
        let Some(&index) = self.items.get(&std::ptr::from_ref::<BlockBox>(source)) else {
            return Ok(None);
        };
        let columns = self.columns(parent.width, measurer)?;
        let area = &self.plan.topology.placement.areas[index];
        let mut parent = parent.clone();
        parent.containing_rtl = self.plan.rtl;
        let style = self.plan.source.style();
        let row = self.rows.area(area[ROWS].clone());
        let mut item = items::Item::new(
            source,
            columns.area(area[COLUMNS].clone()),
            &parent,
            style,
            Some(row.end - row.start),
            measurer,
        )?;
        item.inline_definite = self.plan.definite_columns(index, parent.width, measurer);
        item.inherit(
            self.plan
                .child_context(index)
                .map(|parent| parent.with_baselines(self.baseline_groups.clone())),
            [Some(self.rows.clone()), Some(columns)],
        );
        Ok(Some((item, row.end - row.start)))
    }

    pub(crate) fn area(
        &self,
        padding_box: crate::graphics::Rect,
        padding: [f32; 4],
        overflow: Option<crate::graphics::Rect>,
        item: &crate::grid::ItemPlacement,
        measurer: &Measurer<'a>,
    ) -> Result<crate::graphics::Rect, Failure> {
        let lines = self
            .plan
            .topology
            .placement
            .absolute(&self.plan.topology.lines, item);
        let columns = self.columns(
            (padding_box.width - padding[1] - padding[3]).max(0.0),
            measurer,
        )?;
        let tracks = [&self.rows, &*columns];
        let mut edges = [
            [padding_box.y, padding_box.y + padding_box.height],
            [padding_box.x, padding_box.x + padding_box.width],
        ];
        if let Some(overflow) = overflow {
            edges[ROWS][1] = edges[ROWS][1].max(overflow.y + overflow.height + padding[2]);
            if self.plan.rtl {
                edges[COLUMNS][0] = edges[COLUMNS][0].min(overflow.x - padding[3]);
            } else {
                edges[COLUMNS][1] = edges[COLUMNS][1].max(overflow.x + overflow.width + padding[1]);
            }
        }
        for axis in [ROWS, COLUMNS] {
            let origin = if axis == ROWS {
                padding_box.y + padding[0]
            } else {
                padding_box.x + padding[3]
            };
            let extent = tracks[axis].area(0..self.plan.topology.placement.tracks[axis]);
            if axis == COLUMNS && self.plan.rtl {
                edges[axis][0] = edges[axis][0].min(origin + extent.start - padding[3]);
            } else {
                let padding_end = if axis == ROWS { padding[2] } else { padding[1] };
                edges[axis][1] = edges[axis][1].max(origin + extent.end + padding_end);
            }
            for (end, line) in lines[axis].iter().enumerate() {
                let physical = if axis == COLUMNS && self.plan.rtl {
                    1 - end
                } else {
                    end
                };
                if let Some(line) = line {
                    edges[axis][physical] = origin + tracks[axis].boundary(*line, end == 1);
                }
            }
        }
        Ok(crate::graphics::Rect {
            x: edges[COLUMNS][0],
            y: edges[ROWS][0],
            width: (edges[COLUMNS][1] - edges[COLUMNS][0]).max(0.0),
            height: (edges[ROWS][1] - edges[ROWS][0]).max(0.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::StyledDocument;
    use crate::resources::{FileResolver, ResourceBase};
    use crate::text::TextContext;

    #[test]
    fn item_metrics_distinguish_parent_plans_and_restore_context_after_errors() {
        with_probe(
            "<style>.probe{grid-template-columns:1fr;height:20pt}</style><div class=probe></div>",
            |block, measurer| {
                let build_parent = || {
                    Rc::new(
                        topology::Topology::new(
                            block.style(),
                            &[Default::default()],
                            [None; 2],
                            [SizeBounds::default(); 2],
                            &crate::layout::work::Budget::new(crate::layout::MAX_LAYOUT_WORK),
                        )
                        .unwrap(),
                    )
                };
                let first = ParentContext::new(block, build_parent(), 0);
                let second = ParentContext::new(block, build_parent(), 0);
                let geometry =
                    crate::layout::geometry::horizontal(block.style(), 0.0, 100.0, false, None);
                fn measure<'a>(
                    measurer: &mut Measurer<'a>,
                    block: &'a BlockBox,
                    geometry: &crate::layout::geometry::HorizontalBox,
                    parent: ParentContext,
                ) -> Result<crate::layout::measure::ItemMetrics, Failure> {
                    measurer.with_grid_parent(Some(parent), |measurer| {
                        measurer.item_metrics(block, geometry, None, None, |measurer| {
                            measurer.measure_format(block, geometry.clone(), true, None, None)
                        })
                    })
                }
                assert_eq!(
                    measure(measurer, block, &geometry, first.clone())
                        .unwrap()
                        .height,
                    20.0
                );
                measurer.work = crate::layout::work::Budget::new(0);
                assert_eq!(
                    measure(measurer, block, &geometry, first.clone())
                        .unwrap()
                        .height,
                    20.0,
                    "identical inherited context must reuse measurement"
                );
                let geometry_context = first.clone().with_geometry(
                    [
                        None,
                        Some(Rc::new(TrackGeometry::new(
                            &[100.0],
                            &[false],
                            0.0,
                            None,
                            (
                                crate::style::Alignment::Start,
                                crate::style::OverflowPosition::Default,
                            ),
                            false,
                        ))),
                    ],
                    [[0.0; 2]; 2],
                );
                assert!(
                    matches!(
                        measure(measurer, block, &geometry, geometry_context),
                        Err(Failure::Layout(LayoutError::WorkLimit { .. }))
                    ),
                    "resolved parent geometry must also distinguish cached measurements"
                );
                measurer.grid_parent = Some(first.clone());
                assert!(
                    matches!(
                        measure(measurer, block, &geometry, second),
                        Err(Failure::Layout(LayoutError::WorkLimit { .. }))
                    ),
                    "a different parent plan must not reuse cached metrics"
                );
                assert!(
                    measurer.grid_parent.as_ref() == Some(&first),
                    "failed measurement must restore the outer parent context"
                );
            },
        );
    }

    #[test]
    fn cached_metrics_retain_descendant_row_contributions_without_remeasurement() {
        with_probe(
            "<style>.probe{display:grid;grid-template-columns:1fr;height:20pt}</style><div class=probe></div>",
            |block, measurer| {
                let geometry =
                    crate::layout::geometry::horizontal(block.style(), 0.0, 100.0, false, None);
                let contributions: Rc<[SizingItem]> = Rc::from(vec![SizingItem {
                    tracks: 0..2,
                    min_content: 30.0,
                    max_content: 40.0,
                    minimum: 10.0,
                    content_minimum: Some(30.0),
                }]);
                let first = measurer
                    .item_metrics(block, &geometry, None, None, |measurer| {
                        let mut measured =
                            measurer.measure_format(block, geometry.clone(), true, None, None)?;
                        Rc::get_mut(measured.grid.as_mut().expect("grid geometry"))
                            .expect("fresh measurement owns its geometry")
                            .row_contributions = Some(contributions.clone());
                        Ok(measured)
                    })
                    .unwrap();
                measurer.work = crate::layout::work::Budget::new(0);
                let cached = measurer
                    .item_metrics(block, &geometry, None, None, |_| {
                        panic!("cache hit must not remeasure descendants")
                    })
                    .unwrap();
                for metrics in [first, cached] {
                    let rows = metrics
                        .grid_rows
                        .expect("row contributions survive extraction and caching");
                    assert!(
                        Rc::ptr_eq(&rows, &contributions),
                        "share the immutable contribution buffer"
                    );
                    assert_eq!(rows[0].tracks, 0..2);
                    assert_eq!(rows[0].max_content, 40.0);
                }
            },
        );
    }

    #[test]
    fn item_metrics_reuse_measurement_but_resolve_new_percentage_height_bases() {
        with_probe(
            "<style>.probe{grid-template-columns:1fr;height:50%}</style><div class=probe></div>",
            |block, measurer| {
                let geometry =
                    crate::layout::geometry::horizontal(block.style(), 0.0, 100.0, false, None);
                fn measure<'a>(
                    measurer: &mut Measurer<'a>,
                    block: &'a BlockBox,
                    geometry: &crate::layout::geometry::HorizontalBox,
                    basis: f32,
                ) -> Result<crate::layout::measure::ItemMetrics, Failure> {
                    measurer.item_metrics(block, geometry, None, Some(basis), |measurer| {
                        measurer.measure_format(block, geometry.clone(), true, Some(basis), None)
                    })
                }
                let first = measure(measurer, block, &geometry, 100.0).unwrap();
                assert_eq!(first.height, 50.0);
                measurer.work = crate::layout::work::Budget::new(0);
                assert_eq!(
                    measure(measurer, block, &geometry, 100.0).unwrap().height,
                    first.height,
                    "an identical request must reuse the completed measurement"
                );
                assert!(
                    matches!(
                        measure(measurer, block, &geometry, 200.0),
                        Err(Failure::Layout(LayoutError::WorkLimit { .. }))
                    ),
                    "a different percentage basis must run measurement again"
                );
                measurer.work = crate::layout::work::Budget::new(crate::layout::MAX_LAYOUT_WORK);
                assert_eq!(
                    measure(measurer, block, &geometry, 200.0).unwrap().height,
                    100.0
                );
                assert_eq!(
                    measure(measurer, block, &geometry, 100.0).unwrap().height,
                    50.0
                );
            },
        );
    }

    fn with_probe(html: &str, check: impl for<'a> FnOnce(&'a BlockBox, &mut Measurer<'a>)) {
        let resolver = FileResolver::default();
        let document =
            StyledDocument::parse(html, &ResourceBase::default(), &[], &resolver).unwrap();
        let text = TextContext::new(&document.styles().fonts, &resolver);
        let mut measurer = Measurer {
            native_sizing: None,
            table_data: Default::default(),
            frame_measurements: Default::default(),
            raw_templates: Default::default(),
            raw_pagination: false,
            column_measurement: false,
            orthogonal_columns: None,
            space: Default::default(),
            work: crate::layout::work::Budget::new(crate::layout::MAX_LAYOUT_WORK),
            text: &text,
            canvas_images: false,
            motion: Rc::new(crate::layout::motion::Resources::new(
                Default::default(),
                text.fonts(),
            )),
            page_height: document.page_box().geometry.content_box().height,
            sources: crate::layout::source::SourceIds::new(document.root()),
            clearance_targets: Default::default(),
            item_metrics: Default::default(),
            grid_parent: None,
            fragmentainer: crate::layout::breaks::Fragmentainer::Page,
            column_flow: None,
            spanner: None,
            unbroken: false,
            named_pages: true,
            container_sizes: Default::default(),
        };
        let mut pending = vec![document.root()];
        let block = loop {
            let block = pending.pop().expect("styled probe exists");
            if !block.style().grid.templates[COLUMNS].tracks.is_empty() {
                break block;
            }
            pending.extend(block.children().iter().filter_map(|child| match child {
                BlockContent::Block(child) => Some(child),
                _ => None,
            }));
        };
        check(block, &mut measurer);
    }

    #[test]
    fn container_baselines_follow_grid_order_and_synthesize_empty_item_baselines() {
        with_probe(
            "<style>.probe{grid-template-columns:80pt;grid-template-rows:40pt 40pt}</style><section class=probe><div style='grid-row:2;height:20pt'></div><div style='grid-row:1;height:10pt'></div></section>",
            |block, measurer| {
                let geometry = crate::layout::geometry::horizontal(
                    block.style(),
                    0.0,
                    80.0,
                    false,
                    Some(80.0),
                );
                let measured = measure(measurer, block, &geometry, None, None).unwrap();
                assert_eq!(
                    measured.first_baseline(),
                    Some(10.0),
                    "first grid row, not first DOM child"
                );
                assert_eq!(
                    measured.last_baseline(),
                    Some(60.0),
                    "last row offset plus its border-edge baseline"
                );
            },
        );
    }

    #[test]
    fn measured_items_use_grid_areas_for_padding_spans_and_auto_margins() {
        use crate::layout::measure::FlowItem;
        for (css, body, expected) in [
            (
                "grid-template-columns:80pt;grid-template-rows:80pt",
                "<table style='border-spacing:0'><tr><td style='padding:0'><div style='height:10pt'></div></td></tr></table>",
                vec![(0.0, 0.0, 80.0, 80.0)],
            ),
            (
                "grid-template-columns:60pt 180pt;grid-template-rows:40pt",
                "<div style='padding:10%;height:10pt'></div><div style='width:40pt;height:10pt;margin-left:auto'></div>",
                vec![(0.0, 0.0, 60.0, 22.0), (200.0, 0.0, 40.0, 10.0)],
            ),
            (
                "grid-template-columns:80pt 120pt;grid-template-rows:30pt 40pt;gap:10pt",
                "<div style='grid-row:1 / span 2'></div><div></div><div></div>",
                vec![
                    (0.0, 0.0, 80.0, 80.0),
                    (90.0, 0.0, 120.0, 30.0),
                    (90.0, 40.0, 120.0, 40.0),
                ],
            ),
            (
                "grid-template-columns:80pt 120pt;grid-template-rows:30pt 40pt;gap:10pt;grid-template-areas:'head head' 'side main'",
                "<div style='grid-area:head'></div><div style='grid-area:side'></div><div style='grid-area:main'></div>",
                vec![
                    (0.0, 0.0, 210.0, 30.0),
                    (0.0, 40.0, 80.0, 40.0),
                    (90.0, 40.0, 120.0, 40.0),
                ],
            ),
        ] {
            let html =
                format!("<style>.probe{{{css}}}</style><section class=probe>{body}</section>");
            with_probe(&html, |block, measurer| {
                let geometry = crate::layout::geometry::horizontal(
                    block.style(),
                    0.0,
                    240.0,
                    false,
                    Some(240.0),
                );
                let measured = measure(measurer, block, &geometry, None, None).unwrap();
                let mut actual = Vec::new();
                let mut y = 0.0;
                for flow in &measured.items {
                    if let FlowItem::Row(row) = flow {
                        for cell in &row.cells {
                            let rect = cell.block.geometry.border_box(cell.block.height);
                            actual.push((rect.x, y + cell.offset, rect.width, rect.height));
                        }
                    }
                    y += flow.height();
                }
                assert_eq!(actual, expected, "{css}: area-relative item border boxes");
            });
        }
    }

    #[test]
    fn computed_fractional_tracks_produce_physical_grid_areas() {
        for direction in ["ltr", "rtl"] {
            let html = format!(
                "<style>.probe{{direction:{direction};grid-template-columns:60pt 1fr 2fr;column-gap:10pt}}</style><section class=probe><div></div><div></div><div></div></section>"
            );
            with_probe(&html, |block, measurer| {
                let width = 240.0;
                let plan = Plan::new(
                    block.into(),
                    measurer,
                    [None, Some(width)],
                    [SizeBounds::default(); 2],
                )
                .unwrap();
                let contributions = plan
                    .column_contributions(measurer, width, plan.gap(COLUMNS, Some(width)))
                    .unwrap();
                let tracks = plan
                    .sizes(
                        COLUMNS,
                        Some(width),
                        &contributions,
                        AvailableSpace::Definite(width),
                        true,
                        measurer,
                    )
                    .unwrap();
                for (item, expected) in [
                    0.0..60.0,
                    70.0..(70.0 + 160.0 / 3.0),
                    (80.0 + 160.0 / 3.0)..240.0,
                ]
                .into_iter()
                .enumerate()
                {
                    let expected = if direction == "rtl" {
                        width - expected.end..width - expected.start
                    } else {
                        expected
                    };
                    let actual = tracks.area(plan.topology.placement.areas[item][COLUMNS].clone());
                    assert!(
                        (actual.start - expected.start).abs() < 0.001
                            && (actual.end - expected.end).abs() < 0.001,
                        "{direction} item {item}: {actual:?}, expected {expected:?}"
                    );
                }
            });
        }
    }

    #[test]
    fn intrinsic_tracks_measure_authored_item_widths_and_ignore_out_of_flow_items() {
        for (overflow, first) in [
            ("visible", "<div style='width:40pt'></div>"),
            ("hidden", "<div style='width:40pt'></div>"),
            (
                "visible",
                "<table style='border-spacing:0'><tr><td style='padding:0'><div style='width:15pt'></div></td><td style='padding:0'><div style='width:25pt'></div></td></tr></table>",
            ),
        ] {
            let html = format!(
                "<style>.probe{{grid-template-columns:auto auto;column-gap:10pt}}.probe>div{{height:10pt;overflow:{overflow}}}</style><section class=probe>{first}<div style='width:80pt'></div><div style='position:absolute;width:400pt'></div></section>"
            );
            with_probe(&html, |block, measurer| {
                let widths = intrinsic(block, measurer, 300.0, false).unwrap();
                assert_eq!(
                    [widths.min, widths.max],
                    [130.0, 130.0],
                    "overflow={overflow}, first={first}: two preferred-width items plus one gap, no absolute contribution"
                );
                let plan = Plan::new(
                    block.into(),
                    measurer,
                    [None, Some(300.0)],
                    [SizeBounds::default(); 2],
                )
                .unwrap();
                assert_eq!(plan.children.len(), 2);
                assert_eq!(plan.topology.placement.areas, [[0..1, 0..1], [0..1, 1..2]]);
            });
        }
    }
}
