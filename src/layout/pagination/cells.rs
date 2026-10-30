#[allow(
    clippy::wildcard_imports,
    reason = "pagination submodules extend the parent paginator and share its scope"
)]
use super::*;

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(super) struct FlowPosition<'a> {
    page: usize,
    furthest_page: usize,
    y: f32,
    has_flow: bool,
    at_page_start: bool,
    margin_start: MarginStart,
    page_content: bool,
    pending: Vec<MarginStrut<'a>>,
    pending_break: Option<Forced>,
    floats: Vec<crate::layout::floats::FloatContext>,
    offset: [f32; 2],
    context: PaintKey,
    key: PaintKey,
    reserved_order: Option<PaintOrder>,
    orders: PaintOrders,
    flex_item: bool,
    unbroken: usize,
    clip: Option<usize>,
    group: Option<usize>,
    bottom_reserve: f32,
    box_reserve: f32,
    forced_breaks: usize,
    parallel_depth: usize,
    reflow_lanes: usize,
    named_pages: bool,
    space: crate::layout::measure::FlowSpace,
}

impl FlowPosition<'_> {
    pub(super) fn detach_orders(&mut self, snapshots: &mut OrderSnapshots) {
        snapshots.detach(&mut self.orders);
    }
}

#[derive(Clone, Copy)]
struct BranchDepths {
    boxes: usize,
    containers: usize,
    tables: usize,
    bands: usize,
}

#[derive(Clone)]
struct SuspendedBranch<'a> {
    position: FlowPosition<'a>,
    boxes: Vec<ActiveBox<'a>>,
    containers: Vec<Container<'a>>,
    tables: Vec<ActiveTable<'a>>,
    bands: Vec<BandContinuation<'a>>,
}

#[derive(Default)]
pub(super) struct CellStart {
    pub flex_item: bool,
    pub table_cell: bool,
    pub preserve_empty: bool,
    pub orders: Option<PaintOrders>,
    pub grid_rows: Option<std::ops::Range<usize>>,
    pub grow_after_break: bool,
}

#[derive(Clone)]
pub(super) struct CellContinuation<'a, 't> {
    pub grid_rows: Option<std::ops::Range<usize>>,
    remaining_height: Option<f32>,
    table_containing: Vec<crate::layout::tables::TableContaining<'a>>,
    pub after_events: Vec<&'a crate::document::SemanticEvent>,
    parent: FlowPosition<'a>,
    depths: BranchDepths,
    active: Option<CellFlow<'a, 't>>,
}

impl<'a, 't> CellContinuation<'a, 't>
where
    'a: 't,
    't: 'a,
{
    pub(super) fn clip_to(&mut self, height: f32) {
        self.remaining_height = Some(height);
        self.active
            .as_mut()
            .expect("new cell has active flow")
            .frame
            .clipped_height = Some(height);
    }

    pub(super) fn detach_orders(&mut self, snapshots: &mut OrderSnapshots) {
        snapshots.detach(&mut self.parent.orders);
        if let Some(active) = &mut self.active {
            snapshots.detach(&mut active.branch.position.orders);
            for container in &mut active.branch.containers {
                container.detach_orders(snapshots);
            }
            active.cursor.detach_orders(snapshots);
        }
    }
    pub(super) fn position(&self) -> (usize, f32) {
        let position = &self
            .active
            .as_ref()
            .expect("completed cells leave the scheduler")
            .branch
            .position;
        (position.page, position.y)
    }
    pub(super) fn next_page(&self, fragmentainers: &Fragmentainers<'_, '_>) -> usize {
        let flow = self
            .active
            .as_ref()
            .expect("completed cells leave the scheduler");
        let page = flow.branch.position.page;
        flow.request.map_or(page, |request| {
            fragmentainers.next(page, page + 1, request.forced)
        })
    }

    pub(super) fn break_request(&self) -> Option<BreakRequest> {
        self.active.as_ref().and_then(|flow| flow.request)
    }
    pub(super) fn force_break(&mut self, forced: Forced) {
        let request = self
            .active
            .as_mut()
            .and_then(|flow| flow.request.as_mut())
            .expect("crossing cells are suspended at a break");
        request.forced = Forced::combine(request.forced, Some(forced));
    }
}

#[derive(Clone)]
struct CellFlow<'a, 't> {
    frame: BlockFrame<'a>,
    cursor: Box<FlowCursor<'a, 't>>,
    branch: SuspendedBranch<'a>,
    request: Option<BreakRequest>,
}

impl<'a> Paginator<'a, '_, 'a> {
    pub(super) fn flow_position(&self) -> FlowPosition<'a> {
        FlowPosition {
            page: self.page,
            furthest_page: self.furthest_page,
            y: self.y,
            has_flow: self.has_flow,
            at_page_start: self.at_page_start,
            margin_start: self.margin_start,
            page_content: self.page_content,
            pending: self.pending.clone(),
            pending_break: self.pending_break,
            floats: self.floats.clone(),
            offset: self.offset,
            context: self.context.clone(),
            key: self.key.clone(),
            reserved_order: self.reserved_order.clone(),
            orders: self.orders.clone(),
            flex_item: self.flex_item,
            unbroken: self.unbroken,
            clip: self.clip,
            group: self.group,
            bottom_reserve: self.bottom_reserve,
            box_reserve: self.reserved_box_space(),
            forced_breaks: self.forced_breaks,
            parallel_depth: self.parallel_depth,
            reflow_lanes: self.reflow_lanes,
            named_pages: self.measurer.named_pages,
            space: self.measurer.space,
        }
    }

    pub(super) fn restore_flow_position(
        &mut self,
        position: FlowPosition<'a>,
    ) -> Result<(), LayoutError> {
        let changed = self.page != position.page;
        self.select_page(position.page, position.y);
        self.furthest_page = position.furthest_page;
        self.has_flow = position.has_flow;
        self.at_page_start = position.at_page_start;
        self.margin_start = position.margin_start;
        self.page_content = position.page_content || self.page_content;
        self.pending = position.pending;
        self.pending_break = position.pending_break;
        self.trailing_line = None;
        self.floats = position.floats;
        self.offset = position.offset;
        self.context = position.context;
        self.key = position.key;
        self.reserved_order = position.reserved_order;
        self.orders = position.orders;
        self.flex_item = position.flex_item;
        self.unbroken = position.unbroken;
        self.clip = position.clip;
        self.group = position.group;
        self.bottom_reserve =
            position.bottom_reserve - position.box_reserve + self.reserved_box_space();
        self.forced_breaks = position.forced_breaks;
        self.parallel_depth = position.parallel_depth;
        self.reflow_lanes = position.reflow_lanes;
        self.measurer.named_pages = position.named_pages;
        self.measurer.space = position.space;
        if changed {
            self.reflow_open_boxes()?;
        }
        Ok(())
    }

    fn suspend_branch(&mut self, depths: BranchDepths) -> SuspendedBranch<'a> {
        SuspendedBranch {
            position: self.flow_position(),
            boxes: self.boxes.split_off(depths.boxes),
            containers: self.containers.split_off(depths.containers),
            tables: self.tables.split_off(depths.tables),
            bands: self.bands.split_off(depths.bands),
        }
    }

    fn resume_branch(&mut self, branch: SuspendedBranch<'a>) -> Result<(), LayoutError> {
        self.boxes.extend(branch.boxes);
        self.containers.extend(branch.containers);
        self.tables.extend(branch.tables);
        self.bands.extend(branch.bands);
        self.restore_flow_position(branch.position)
    }

    pub(super) fn enter_table_layers(
        &mut self,
        layers: [Option<crate::layout::tables::TableContaining<'a>>; 2],
    ) {
        // The perspective and 3D rendering context each box gives its children: the table its
        // rows and row groups, a row group its rows, and a row its cells.
        let mut parent_3d = self.tables.last().and_then(|table| table.parent_3d);
        for ancestor in layers.into_iter().flatten() {
            let style = ancestor.source.style();
            if !(style.contains_absolute()
                || style.is_grouped()
                || style.forced_stacking_context()
                || !style.decoration.shadows.is_empty()
                || style.decoration.outline.is_drawn()
                || style.decoration.border_image.is_drawn())
            {
                parent_3d = None;
                continue;
            }
            let source = ancestor.source;
            let identity = std::ptr::from_ref(source) as usize;
            let layer = if let Some(layer) = self
                .tables
                .last()
                .and_then(|table| table.layers.get(&identity))
                .cloned()
            {
                layer
            } else {
                if !source.style().decoration.shadows.is_empty()
                    || source.style().decoration.outline.is_drawn()
                    || source.style().decoration.border_image.is_drawn()
                {
                    self.tables
                        .last_mut()
                        .expect("active table")
                        .decorated
                        .push(ancestor.clone());
                }
                let order = self.order();
                // CSS Tables ignores z-index on rows and row groups.
                let grouped = source.style().is_grouped();
                let transformed = source.style().has_transform_effects();
                let isolated = grouped || transformed || source.style().forced_stacking_context();
                let reference = source
                    .style()
                    .motion
                    .active()
                    .then(|| self.motion_block_reference())
                    .flatten();
                let scene = self.scene_node(source.style(), parent_3d);
                let outer = scene.or(self.clip);
                let transform = source.style().is_transformed().then(|| {
                    let node = self.clips.len();
                    self.clips.push(EffectNode {
                        native: None,
                        paints: Vec::new(),
                        parent: outer,
                        effect: Effect::Transform(source.style(), parent_3d, reference),
                        fragments: Vec::new(),
                        areas: Vec::new(),
                        context: None,
                    });
                    self.tables
                        .last_mut()
                        .expect("active table")
                        .transforms
                        .push((ancestor.clone(), node));
                    node
                });
                let mut clip = transform.or(outer);
                if source.style().decoration.masks.clip_path.is_some() {
                    clip = Some(self.effect_node(clip, Effect::PathClip(source.style())));
                }
                let mut paint = positioned_layer(&self.context, isolated.then_some(0), order);
                let group = if isolated {
                    let group = if grouped || source.style().is_backdrop_root() {
                        Group::of(source.style())
                    } else {
                        Group::IDENTITY
                    };
                    let node = self.stacking_group(self.group, group, paint.context.clone());
                    if group.anchored() {
                        self.clips[node].effect = Effect::Group(Group {
                            anchor: clip,
                            ..group
                        });
                    }
                    Some(node)
                } else {
                    self.group
                };
                if !isolated {
                    paint.key = extend(&paint.key, FLOW_LEVEL);
                }
                if let Some(scene) = scene
                    && let Effect::Scene(root) = &mut self.clips[scene].effect
                {
                    root.group = group;
                }
                let own_parent_3d = (transformed
                    && (source.style().perspective.is_some() || source.style().preserves_3d()))
                .then(|| {
                    self.effect_node(clip, Effect::Parent3d(source.style(), transform, parent_3d))
                });
                let layer = TableLayer {
                    clip,
                    paint,
                    group,
                    parent_3d: own_parent_3d,
                };
                self.tables
                    .last_mut()
                    .expect("table cell has an active table")
                    .layers
                    .insert(identity, layer.clone());
                layer
            };
            self.context = layer.paint.context;
            self.key = layer.paint.key;
            self.group = layer.group;
            self.clip = layer.clip;
            parent_3d = layer.parent_3d;
        }
        self.parent_3d = parent_3d;
    }

    pub(super) fn start_cell(
        &mut self,
        block: MeasuredBlock<'a>,
        tail: BreakTail,
        start: CellStart,
        mut float: Option<&mut floats::FloatPlacement>,
    ) -> Result<CellContinuation<'a, 'a>, LayoutError> {
        let CellStart {
            flex_item,
            table_cell,
            preserve_empty,
            orders,
            grid_rows,
            grow_after_break,
        } = start;
        let defer_positioned = table_cell && block.style.position != Position::Static;
        let layers = if table_cell {
            self.tables.last().map_or([None, None], |table| {
                table.flow.plan.cell_layers(block.source)
            })
        } else {
            [None, None]
        };
        let table_containing = layers
            .iter()
            .flatten()
            .filter(|layer| layer.source.style().contains_absolute())
            .cloned()
            .collect::<Vec<_>>();
        let mut parent = self.flow_position();
        if table_cell {
            self.enter_table_layers(layers);
            if let Some(plan) = self.tables.last().map(|table| &table.flow.plan)
                && plan.clips_to_text
                && let Some(index) = plan.cell_index(block.source)
            {
                let cell = Subtree::TableCell {
                    table: std::ptr::from_ref(plan.source) as usize,
                    origin: plan.cell_origin(index),
                };
                self.clip = Some(self.effect_node(self.clip, Effect::Subtree(cell)));
            }
        }
        let depths = BranchDepths {
            boxes: self.boxes.len(),
            containers: self.containers.len(),
            tables: self.tables.len(),
            bands: self.bands.len(),
        };
        self.orders = orders.unwrap_or_else(|| self.orders.branch());
        self.parallel_depth += 1;
        self.reflow_lanes += usize::from(
            table_cell
                || grid_rows.is_some()
                || float.as_ref().is_some_and(|placement| placement.reflow),
        );
        self.flex_item = flex_item;
        self.measurer.named_pages &= !table_cell;
        let started: Result<CellFlow<'a, 'a>, LayoutError> = (|| {
            let (mut block, mut request) = self.prepare_block(block, tail)?;
            while let Some(next) = request {
                self.new_page(next.forced, next.preserve_empty || preserve_empty)?;
                let Some(placement) = float.as_deref_mut() else {
                    break;
                };
                block = self.reposition_float(&block, placement)?;
                block.break_before = crate::style::BreakBetween::Auto;
                (block, request) = self.prepare_block(block, tail)?;
            }
            for containing in &table_containing {
                self.containers.push(Container {
                    pending: Vec::new(),
                    fixed: containing.source.style().contains_fixed(),
                });
            }
            let mut frame = self.start_block(block)?;
            frame.grow_after_break = grow_after_break;
            frame.previous_context = parent.context.clone();
            frame.previous_key = parent.key.clone();
            frame.previous_group = parent.group;
            frame.previous_clip = parent.clip;
            frame.defer_positioned = defer_positioned;
            if let Some(placement) = float {
                self.boxes
                    .last_mut()
                    .expect("started float has an active box")
                    .float_placement = Some(placement.clone());
            }
            let cursor = FlowCursor::new(std::mem::take(&mut frame.items), tail);
            let branch = self.suspend_branch(depths);
            Ok(CellFlow {
                frame,
                cursor,
                branch,
                request: None,
            })
        })();
        self.orders = parent.orders.clone();
        self.measurer.named_pages = parent.named_pages;
        self.parallel_depth = parent.parallel_depth;
        self.reflow_lanes = parent.reflow_lanes;
        let active = started?;
        parent.forced_breaks = parent
            .forced_breaks
            .max(active.branch.position.forced_breaks);
        self.restore_flow_position(parent.clone())?;
        Ok(CellContinuation {
            grid_rows,
            remaining_height: None,
            table_containing,
            after_events: Vec::new(),
            parent,
            depths,
            active: Some(active),
        })
    }

    /// Lays out an absolutely positioned box from its start as a flow of its own, which
    /// breaks across the pages after it (CSS Position 3 §3.5.3) apart from the boxes, tables
    /// and floats of the flow that placed it.
    pub(super) fn positioned_flow(&mut self, block: MeasuredBlock<'a>) -> Result<(), LayoutError> {
        let style = block.style;
        let outer = self.suspend_branch(BranchDepths {
            boxes: 0,
            containers: self.containers.len(),
            tables: 0,
            bands: 0,
        });
        // The box's overflow reaches the boxes and root it was detached from once it ends.
        let root_overflow = self.root_overflow.take();
        self.floats = vec![crate::layout::floats::FloatContext::default()];
        let result = (|| -> Result<ActiveBox<'a>, LayoutError> {
            let mut flow = self.start_cell(
                block,
                BreakTail::default(),
                CellStart {
                    preserve_empty: true,
                    ..CellStart::default()
                },
                None,
            )?;
            loop {
                if let Some(active) = self.advance_cell(&mut flow)? {
                    break Ok(active);
                }
                self.measurer.work.spend(1)?;
            }
        })();
        self.root_overflow = root_overflow;
        self.resume_branch(outer)?;
        self.propagate_overflow(&result?, style)
    }

    pub(super) fn advance_cell(
        &mut self,
        cell: &mut CellContinuation<'a, 'a>,
    ) -> Result<Option<ActiveBox<'a>>, LayoutError> {
        let CellFlow {
            mut frame,
            cursor,
            branch,
            request,
        } = cell.active.take().expect("a completed cell is not resumed");
        cell.parent.forced_breaks = cell.parent.forced_breaks.max(self.forced_breaks);
        self.resume_branch(branch)?;
        let result = (|| {
            if let Some(request) = request {
                self.new_page(request.forced, request.preserve_empty)?;
            }
            let available = (self.height - self.bottom_reserve - self.y).max(0.0);
            let previous_unbroken = self.unbroken;
            if cell
                .remaining_height
                .is_some_and(|height| !overflows(height, available))
            {
                self.unbroken += 1;
            }
            let progress = self.resume_flow(cursor);
            self.unbroken = previous_unbroken;
            let suspended = if let Some(next) = progress? {
                Some(next)
            } else {
                self.resume_block_end(&mut frame)?
                    .map(|request| (FlowCursor::new(Vec::new(), BreakTail::default()), request))
            };
            if let Some((cursor, request)) = suspended {
                if let Some(remaining) = &mut cell.remaining_height {
                    *remaining = (*remaining - available).max(0.0);
                }
                let branch = self.suspend_branch(cell.depths);
                cell.parent.forced_breaks =
                    cell.parent.forced_breaks.max(branch.position.forced_breaks);
                self.restore_flow_position(cell.parent.clone())?;
                cell.active = Some(CellFlow {
                    frame,
                    cursor,
                    branch,
                    request: Some(request),
                });
                Ok(None)
            } else {
                let active = self.finish_block(frame)?;
                for containing in cell.table_containing.iter().rev() {
                    let pending = self
                        .containers
                        .pop()
                        .expect("cell structural container remains active")
                        .pending;
                    if !pending.is_empty() {
                        self.tables
                            .last_mut()
                            .expect("cell belongs to its active table")
                            .positioned
                            .push((containing.clone(), pending));
                    }
                }
                Ok(Some(active))
            }
        })();
        self.orders = cell.parent.orders.clone();
        self.measurer.named_pages = cell.parent.named_pages;
        self.parallel_depth = cell.parent.parallel_depth;
        self.reflow_lanes = cell.parent.reflow_lanes;
        self.forced_breaks = self.forced_breaks.max(cell.parent.forced_breaks);
        result
    }
}
