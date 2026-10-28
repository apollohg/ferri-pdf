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
    floats: Vec<crate::floats::FloatContext>,
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
    table_containing: Vec<crate::tables::TableContaining<'a>>,
    pub after_events: Vec<&'a ferri_pdf_document::SemanticEvent>,
    parent: FlowPosition<'a>,
    depths: BranchDepths,
    active: Option<CellFlow<'a, 't>>,
}

impl CellContinuation<'_, '_> {
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

impl<'a, 'm, 't> Paginator<'a, 'm, 't> {
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
        layers: [Option<crate::tables::TableContaining<'a>>; 2],
    ) {
        for ancestor in layers.into_iter().flatten().filter(|layer| {
            let style = layer.source.style();
            style.position != Position::Static
                || style.is_translucent()
                || !style.decoration.shadows.is_empty()
                || !style.transform.is_empty()
        }) {
            let source = ancestor.source;
            let identity = std::ptr::from_ref(source) as usize;
            let layer = match self
                .tables
                .last()
                .and_then(|table| table.layers.get(&identity))
                .cloned()
            {
                Some(layer) => layer,
                None => {
                    if !source.style().decoration.shadows.is_empty() {
                        self.tables
                            .last_mut()
                            .expect("active table")
                            .shadows
                            .push(ancestor.clone());
                    }
                    let order = self.order();
                    // CSS Tables ignores z-index on rows and row groups.
                    let translucent = source.style().is_translucent();
                    let transformed = !source.style().transform.is_empty();
                    let clip = if transformed {
                        let node = self.clips.len();
                        self.clips.push(EffectNode {
                            parent: self.clip,
                            effect: Effect::Transform(source.style()),
                            fragments: Vec::new(),
                            areas: Vec::new(),
                        });
                        self.tables
                            .last_mut()
                            .expect("active table")
                            .transforms
                            .push((ancestor, node));
                        Some(node)
                    } else {
                        self.clip
                    };
                    let group = if translucent {
                        Some(self.opacity_group(self.group, source.style().opacity))
                    } else {
                        self.group
                    };
                    let mut paint = positioned_layer(
                        &self.context,
                        (translucent || transformed).then_some(0),
                        order,
                    );
                    if !translucent && !transformed {
                        paint.key = extend(&paint.key, FLOW_LEVEL);
                    }
                    let layer = TableLayer { paint, clip, group };
                    self.tables
                        .last_mut()
                        .expect("table cell has an active table")
                        .layers
                        .insert(identity, layer.clone());
                    layer
                }
            };
            self.context = layer.paint.context;
            self.key = layer.paint.key;
            self.group = layer.group;
            self.clip = layer.clip;
        }
    }

    pub(super) fn start_cell(
        &mut self,
        block: MeasuredBlock<'a>,
        tail: BreakTail,
        start: CellStart,
        mut float: Option<&mut super::floats::FloatPlacement>,
    ) -> Result<CellContinuation<'a, 't>, LayoutError> {
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
            self.tables
                .last()
                .map(|table| table.flow.plan.cell_layers(block.source))
                .unwrap_or([None, None])
        } else {
            [None, None]
        };
        let table_containing = layers
            .iter()
            .flatten()
            .filter(|layer| {
                layer.source.style().position != Position::Static
                    || !layer.source.style().transform.is_empty()
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut parent = self.flow_position();
        if table_cell {
            self.enter_table_layers(layers);
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
        let started: Result<CellFlow<'a, 't>, LayoutError> = (|| {
            let (mut block, mut request) = self.prepare_block(block, tail)?;
            while let Some(next) = request {
                self.new_page(next.forced, next.preserve_empty || preserve_empty)?;
                let Some(placement) = float.as_deref_mut() else {
                    break;
                };
                block = self.reposition_float(block, placement)?;
                block.break_before = ferri_pdf_style::BreakBetween::Auto;
                (block, request) = self.prepare_block(block, tail)?;
            }
            for containing in &table_containing {
                self.containers.push(Container {
                    pending: Vec::new(),
                    fixed: !containing.source.style().transform.is_empty(),
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

    pub(super) fn advance_cell(
        &mut self,
        cell: &mut CellContinuation<'a, 't>,
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
