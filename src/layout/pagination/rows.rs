#[allow(
    clippy::wildcard_imports,
    reason = "pagination submodules extend the parent paginator and share its scope"
)]
use super::*;
use crate::layout::measure::{MeasuredCell, RowPaint};
use std::ops::Range;

type Stretch<'a> = (
    ActiveBox<'a>,
    f32,
    Option<(usize, f32)>,
    Vec<crate::layout::tables::FragmentBackground<'a>>,
    Option<usize>,
    Vec<&'a crate::document::SemanticEvent>,
);
type Lane<'a, 't> = (
    cells::CellContinuation<'a, 't>,
    bool,
    f32,
    Option<(usize, f32)>,
    Vec<crate::layout::tables::FragmentBackground<'a>>,
    Option<usize>,
);

#[derive(Clone)]
struct GridTrace {
    starts: Vec<usize>,
    order: PaintOrders,
    source_order: HashMap<u32, usize>,
}

#[derive(Clone)]
pub(super) struct RowCursor<'a, 't> {
    row: MeasuredRow<'a>,
    tail: BreakTail,
    start: f32,
    start_page: usize,
    last_body: bool,
    repeating: bool,
    end: (usize, f32),
    stretches: Vec<Stretch<'a>>,
    source_row_geometry: Option<(usize, crate::layout::tables::RowGeometry)>,
    alignment_cache: HashMap<(u32, u32), crate::layout::tables::CellAlignments>,
    plan: Option<Rc<crate::layout::tables::TablePlan<'a>>>,
    cells: VecDeque<(MeasuredCell<'a>, PaintOrders)>,
    active_cells: VecDeque<Lane<'a, 't>>,
    origin: (usize, usize, f32),
    row_ends: BTreeMap<usize, (usize, f32)>,
    internal_breaks: bool,
    constrained_rows: BTreeSet<usize>,
    source_rows: Range<usize>,
    grid_ends: Vec<(usize, f32)>,
    trace: Option<GridTrace>,
    entered: bool,
    next_row: Option<usize>,
    boundary: bool,
    frontier: usize,
    frontier_y: f32,
    yielded: bool,
    transition: Option<BreakRequest>,
    fragment_ends: BTreeMap<usize, f32>,
    previous_unbroken: usize,
    body_end: Option<usize>,
}

impl<'a, 't> RowCursor<'a, 't>
where
    'a: 't,
    't: 'a,
{
    pub(super) fn detach_orders(&mut self, snapshots: &mut OrderSnapshots) {
        for (_, orders) in &mut self.cells {
            snapshots.detach(orders);
        }
        for (cell, ..) in &mut self.active_cells {
            cell.detach_orders(snapshots);
        }
    }
}

impl<'a> Paginator<'a, '_, 'a> {
    fn end_row_fragment(&mut self, ends: &mut BTreeMap<usize, f32>, repeating: bool) {
        if repeating {
            let band = self.bands.last_mut().unwrap();
            if let Some(page) = band.pages.get_mut(&self.page) {
                if page.footer_height > 0.0 {
                    ends.insert(self.page, self.y);
                }
                page.footer_start = self.y;
            }
        }
    }
    fn place_row_target(&mut self, target: &crate::layout::measure::RowTarget, y: f32) {
        let mut rect = target.rect;
        rect.x += self.offset[0] + target.offset[0];
        rect.y = self.top + y + self.offset[1] + target.offset[1];
        self.display_lists[self.page].push(ScopedPaint {
            native: None,
            visible: target.visible,
            order: self.orders.take(),
            scope: self.key.clone(),
            clip: self.clip,
            group: self.group,
            ink: None,
            item: DisplayItem::Box(BoxPaint::whole(rect, BoxDecoration::default())),
            navigation: target.navigation,
        });
    }
    pub(super) fn start_parallel_row(
        &mut self,
        mut row: MeasuredRow<'a>,
        tail: BreakTail,
        body_end: Option<usize>,
    ) -> RowCursor<'a, 'a> {
        if row.cells.is_empty()
            && let Some((band, plan)) = row
                .band
                .as_ref()
                .zip(self.tables.last())
                .map(|(band, table)| (band.clone(), table.flow.plan.clone()))
        {
            let layer = self.current_layer();
            for index in band {
                self.enter_table_layers(plan.row_layers(index));
                self.context = layer.context.clone();
                self.key = layer.key.clone();
                self.group = layer.group;
                self.clip = layer.clip;
            }
        }
        let previous_unbroken = self.unbroken;
        if overflows(row.height, self.height) && row.indivisible() {
            self.unbroken += 1;
        }
        self.pending_break = None;
        self.flush_margin();
        let start = self.y;
        let start_page = self.page;
        let last_body = row
            .band
            .as_ref()
            .zip(self.tables.last())
            .is_some_and(|(band, table)| band.end == table.flow.body.end);
        let repeating = self.unbroken == 0
            && row.cells.iter().any(|cell| cell.stretch)
            && row
                .band
                .as_ref()
                .zip(self.tables.last())
                .is_some_and(|(band, table)| {
                    table.flow.body.contains(&band.start)
                        && (!table.flow.header.is_empty() || !table.flow.footer.is_empty())
                });
        if repeating {
            let table = self.tables.last().unwrap();
            let header_height: f32 = table
                .flow
                .header
                .iter()
                .map(|row| row.height + table.flow.spacing)
                .sum();
            let footer_height = table.flow.footer_height;
            let footer_start = self.height
                - self.cloned_end_space()
                - self.band_footer_height(self.page)
                - footer_height;
            self.bands.push(BandContinuation {
                table: self.tables.len() - 1,
                boxes: self.boxes.len(),
                first_page: self.page,
                minimum: row.progress_height(),
                pages: [(
                    self.page,
                    BandPage {
                        header: table.flow.header.clone(),
                        footer: table.flow.footer.clone(),
                        header_start: start - header_height,
                        header_height,
                        body_start: start,
                        footer_height,
                        footer_start,
                        spacing: table.flow.spacing,
                    },
                )]
                .into_iter()
                .collect(),
            });
        }

        if let Some(table) = self.tables.last_mut() {
            for (cell, height) in std::mem::take(&mut row.cell_heights) {
                table.cell_heights[cell] = height;
            }
        }
        for (index, mut paint) in std::mem::take(&mut row.paints).into_iter().enumerate() {
            for rect in std::iter::once(&mut paint.rect).chain(paint.background_box.as_mut()) {
                rect.x += self.offset[0];
                rect.y += self.top + start + self.offset[1];
            }
            if paint.has_ink() {
                if let Some(rows) = row.background_rows.get(index) {
                    let table = self
                        .tables
                        .last_mut()
                        .expect("table layer belongs to a table");
                    table.backgrounds.push(TableBackground {
                        repeated: table.repeats(&rows.rows),
                        rows: rows.clone(),
                        page: self.page,
                        slot: self.display_lists[self.page].len(),
                        underlay: None,
                        before: 0.0,
                    });
                }
                self.display_lists[self.page].push(ScopedPaint {
                    native: None,
                    visible: true,
                    order: self.orders.take(),
                    scope: self.key.clone(),
                    clip: self.clip,
                    group: self.group,
                    ink: None,
                    item: DisplayItem::Box(paint),
                    navigation: BoxNavigation::default(),
                });
                self.page_content = true;
            }
        }
        if row.cells.iter().any(|cell| cell.stretch) {
            row.targets
                .sort_by_key(|target| std::cmp::Reverse(target.row));
        } else {
            for target in std::mem::take(&mut row.targets) {
                let y = start + target.rect.y;
                self.place_row_target(&target, y);
            }
        }
        let end = (start_page, start);
        let stretches = Vec::new();
        let source_row_geometry = None;
        let alignment_cache: HashMap<_, crate::layout::tables::CellAlignments> = HashMap::new();
        let plan = row
            .band
            .as_ref()
            .map(|_| self.tables.last().unwrap().flow.plan.clone());
        let mut source_order = HashMap::new();
        let mut cells: VecDeque<_> = std::mem::take(&mut row.cells)
            .into_iter()
            .enumerate()
            .map(|(index, cell)| {
                let order = self
                    .grid_order(cell.block.source)
                    .unwrap_or_else(|| self.order());
                if let Some(grid) = &row.grid
                    && let Some(branch) = self.orders.branch_index(&order)
                {
                    source_order.insert(branch, grid.source_order[index]);
                }
                (cell, order.descendants())
            })
            .collect();
        let trace = row.grid.as_ref().map(|_| GridTrace {
            starts: self.pages.iter().map(Vec::len).collect(),
            order: self.orders.clone(),
            source_order,
        });
        if let Some(grid) = &row.grid {
            cells.make_contiguous().sort_by_key(|(cell, _)| {
                grid.items[&std::ptr::from_ref::<BlockBox>(cell.block.source)].start
            });
        }
        let active_cells = VecDeque::new();
        let origin = (
            row.band.as_ref().map_or(0, |band| band.start),
            start_page,
            start,
        );
        let row_ends = BTreeMap::new();
        let internal_breaks = false;
        let constrained_rows: BTreeSet<_> = row
            .band
            .as_ref()
            .filter(|_| self.unbroken == 0)
            .map(|band| {
                (band.start + 1..band.end)
                    .filter(|&next| {
                        let plan = plan.as_ref().unwrap();
                        plan.row_edges(next - 1, self.measurer.fragmentainer)[1].is_forced()
                            || plan.row_edges(next, self.measurer.fragmentainer)[0].is_forced()
                            || self
                                .measurer
                                .fragmentainer
                                .keeps(plan.row_source(next).style())
                            || plan.row_keep_end(next, band.end, self.measurer.fragmentainer)
                                > next + 1
                    })
                    .collect()
            })
            .unwrap_or_default();
        let source_rows = row
            .band
            .clone()
            .unwrap_or_else(|| 0..row.grid.as_ref().map_or(1, |grid| grid.tracks.len()));
        RowCursor {
            row,
            tail,
            start,
            start_page,
            last_body,
            repeating,
            end,
            stretches,
            source_row_geometry,
            alignment_cache,
            plan,
            cells,
            active_cells,
            origin,
            row_ends,
            internal_breaks,
            constrained_rows,
            source_rows,
            grid_ends: Vec::new(),
            trace,
            entered: false,
            next_row: None,
            boundary: false,
            frontier: start_page,
            frontier_y: start,
            yielded: false,
            transition: None,
            fragment_ends: BTreeMap::new(),
            previous_unbroken,
            body_end,
        }
    }
    fn enter_row(
        &mut self,
        cursor: &mut RowCursor<'a, 'a>,
    ) -> Result<Option<BreakRequest>, LayoutError> {
        let RowCursor {
            row,
            tail,
            end,
            source_row_geometry,
            alignment_cache,
            plan,
            cells,
            active_cells,
            origin,
            internal_breaks,
            constrained_rows,
            source_rows,
            grid_ends,
            entered,
            next_row,
            boundary,
            yielded,
            transition,
            repeating,
            fragment_ends,
            ..
        } = cursor;
        let source_index = source_rows.start;
        let source_row = row.band.as_ref().map(|_| source_index);
        if let Some(grid) = &row.grid
            && transition.is_none()
        {
            let track = &grid.tracks[source_index];
            if source_index == 0 {
                self.finish_height(origin.1, origin.2, track.area.start)?;
            } else {
                let previous = &grid.tracks[source_index - 1];
                let (page, y) = grid_ends[source_index - 1];
                self.select_page(page, y);
                // Shared tracks may overlap local edge space when subgrid padding exceeds a track.
                let gap = track.area.start - previous.area.end;
                let forced = Forced::of(previous.edges[1].combine(track.edges[0]));
                let reserve = self.continuation_reserve()?;
                let fresh =
                    (self.height - self.repeated_start(page, self.boxes.len()) - reserve).max(0.0);
                let required = track.required(fresh) + reserve;
                if self.unbroken == 0
                    && (forced.is_some()
                        || (overflows(y + gap + required, self.height)
                            && !overflows(required, self.height)))
                {
                    let request = BreakRequest {
                        forced,
                        preserve_empty: false,
                    };
                    *transition = Some(request);
                    *yielded = true;
                    return Ok(Some(request));
                }
                self.finish_height(page, y, gap)?;
            }
            *origin = (source_index, self.page, self.y);
        }
        if let Some(request) = transition.take() {
            *origin = (source_index, self.page, self.y);
            *internal_breaks = true;
            if let Some(forced) = request.forced {
                for (continuation, _, _, _, _, _) in active_cells.iter_mut() {
                    continuation.force_break(forced);
                }
            }
        } else if let (Some(source_row), Some(plan)) = (source_row, plan.as_ref())
            && self.unbroken == 0
            && source_row > row.band.as_ref().unwrap().start
        {
            let before = plan.row_edges(source_row, self.measurer.fragmentainer)[0];
            let after = plan.row_edges(source_row - 1, self.measurer.fragmentainer)[1];
            let changed = self.page_name(plan.row_source(source_row).page_boundary().start.clone());
            if let Some(forced) = Forced::combine(
                Forced::of(after.combine(before)),
                changed.then_some(Forced::Page),
            ) {
                self.select_page(end.0, end.1);
                self.end_row_fragment(fragment_ends, *repeating);
                let request = BreakRequest {
                    forced: Some(forced),
                    preserve_empty: false,
                };
                *transition = Some(request);
                *yielded = true;
                return Ok(Some(request));
            }
        }
        if let (Some(source_row), Some(plan)) = (source_row, plan.as_ref()) {
            let context = self.context.clone();
            let key = self.key.clone();
            let group = self.group;
            let clip = self.clip;
            self.enter_table_layers(plan.row_layers(source_row));
            self.context = context;
            self.key = key;
            self.group = group;
            self.clip = clip;
            let position = self.flow_position();
            if source_row == origin.0 {
                self.select_page(origin.1, origin.2);
            } else {
                let table = self.tables.last().unwrap();
                if source_row_geometry
                    .as_ref()
                    .is_none_or(|(row, _)| *row != source_row)
                {
                    *source_row_geometry = Some((
                        source_row,
                        self.measurer.run(|measurer| {
                            plan.resolve_rows(
                                measurer,
                                &table.flow.heights,
                                table.flow.spacing,
                                &table.flow.geometry,
                            )
                        })?,
                    ));
                }
                let geometry = &source_row_geometry.as_ref().unwrap().1;
                self.finish_height(
                    origin.1,
                    origin.2,
                    geometry.starts[source_row] - geometry.starts[origin.0],
                )?;
            }
            let keep_end = plan.row_keep_end(
                source_row,
                row.band.as_ref().unwrap().end,
                self.measurer.fragmentainer,
            );
            let row_kept = self
                .measurer
                .fragmentainer
                .keeps(plan.row_source(source_row).style());
            if self.unbroken == 0
                && row.band.as_ref().is_some_and(|band| band.len() > 1)
                && (row_kept || keep_end > source_row + 1)
            {
                let table = self.tables.last().unwrap();
                if source_row_geometry
                    .as_ref()
                    .is_none_or(|(cached, _)| *cached != source_row)
                {
                    *source_row_geometry = Some((
                        source_row,
                        self.measurer.run(|measurer| {
                            plan.resolve_rows(
                                measurer,
                                &table.flow.heights,
                                table.flow.spacing,
                                &table.flow.geometry,
                            )
                        })?,
                    ));
                }
                let geometry = &source_row_geometry.as_ref().unwrap().1;
                let mut height =
                    geometry.starts[keep_end] - geometry.starts[source_row] - table.flow.spacing;
                let reserve = self.continuation_reserve()?;
                let available = self.height - reserve;
                let start = self.repeated_start(self.page, self.boxes.len());
                if overflows(height, available - start) && row_kept {
                    height = geometry.starts[source_row + 1]
                        - geometry.starts[source_row]
                        - table.flow.spacing;
                }
                if !overflows(height, available - start) && overflows(self.y + height, available) {
                    self.end_row_fragment(fragment_ends, *repeating);
                    *transition = Some(BreakRequest::default());
                    *yielded = true;
                    return Ok(Some(BreakRequest::default()));
                }
            }
            while row
                .targets
                .last()
                .is_some_and(|target| target.row == source_row)
            {
                let target = row.targets.pop().unwrap();
                self.place_row_target(&target, self.y);
            }
            for event in plan.row_entry_events(source_row) {
                self.string_locations.record(self.page, event);
            }
            self.restore_flow_position(position)?;
        }
        while cells.front().is_some_and(|(cell, _)| {
            if let Some(grid) = &row.grid {
                grid.items[&std::ptr::from_ref::<BlockBox>(cell.block.source)].start == source_index
            } else {
                cell.grid_cell
                    .map(|index| plan.as_ref().unwrap().cell_rows(index).start)
                    == source_row
            }
        }) {
            let (mut cell, orders) = cells.pop_front().unwrap();
            let grid_rows = row.grid.as_ref().map(|grid| {
                let range = grid.items[&std::ptr::from_ref::<BlockBox>(cell.block.source)].clone();
                cell.offset -= grid.tracks[range.start].area.start;
                range
            });
            if let Some(index) = cell.grid_cell
                && cell.stretch
                && row.band.as_ref().is_some_and(|band| band.len() > 1)
            {
                let table = self.tables.last().unwrap();
                let cell_row = table.flow.plan.cell_rows(index).start;
                // Sibling lanes share one placement decision; committed extents affect later rows.
                if source_row_geometry
                    .as_ref()
                    .is_none_or(|(source_row, _)| *source_row != cell_row)
                {
                    *source_row_geometry = Some((
                        cell_row,
                        self.measurer.run(|measurer| {
                            table.flow.plan.resolve_rows(
                                measurer,
                                &table.flow.heights,
                                table.flow.spacing,
                                &table.flow.geometry,
                            )
                        })?,
                    ));
                }
                let (_, geometry) = source_row_geometry.as_ref().unwrap();
                let start = geometry.starts[cell_row] - geometry.starts[origin.0];
                cell.paint_offset = Some(start);
                cell.offset = start + geometry.offsets[index];
            }
            self.select_page(origin.1, origin.2);
            let paint_start = if let Some(offset) = cell.paint_offset {
                self.finish_height(origin.1, origin.2, offset)?;
                Some((self.page, self.y))
            } else {
                None
            };
            if let Some(index) = cell.grid_cell
                && cell.stretch
                && cell.block.style.table.vertical_align.cell() != crate::style::VerticalAlign::Top
                && let Some((page, y)) = paint_start
            {
                let band = row.band.as_ref().expect("table cells belong to a band");
                let plan = self.tables.last().unwrap().flow.plan.clone();
                let paint_page = page;
                self.finish_extent(page, y, |paginator, page, space| {
                    let geometry = paginator.table_geometry(page)?;
                    let height = paginator.fragmentainers.height(page);
                    let key = (
                        geometry.geometry.containing_width.to_bits(),
                        height.to_bits(),
                    );
                    if !alignment_cache.contains_key(&key) {
                        let previous_height = paginator.measurer.page_height;
                        paginator.measurer.page_height = height;
                        let measured = paginator.measurer.run(|measurer| {
                            plan.cell_alignments(measurer, band.clone(), &geometry)
                        });
                        paginator.measurer.page_height = previous_height;
                        let alignments = measured?;
                        if alignment_cache.len() == MAX_TABLE_ALIGNMENT_GEOMETRIES {
                            alignment_cache.clear();
                        }
                        alignment_cache.insert(key, alignments);
                    }
                    let start = if page == paint_page { y } else { space.start };
                    let available =
                        (height - space.cloned_end - paginator.band_footer_height(page) - start)
                            .max(0.0);
                    let fresh = (height
                        - space.cloned_end
                        - paginator.band_footer_height(page)
                        - space.start)
                        .max(0.0);
                    Ok(plan.fragment_alignment(
                        &alignment_cache[&key],
                        index,
                        available,
                        fresh,
                        paginator.measurer.fragmentainer,
                    ))
                })?;
            } else {
                self.finish_height(origin.1, origin.2, cell.offset)?;
            }
            if let Some(index) = cell.grid_cell
                && cell.stretch
            {
                let geometry = self.table_geometry(self.page)?;
                let table = self.tables.last_mut().unwrap();
                if table.flow.plan.cell_geometry(index, &geometry) != cell.block.geometry {
                    let natural = self.measurer.run(|measurer| {
                        table
                            .flow
                            .plan
                            .measure_cell(measurer, index, &geometry, None)
                    })?;
                    table.flow.heights[index] = crate::layout::tables::cell_height(&natural);
                    let rows = self.measurer.run(|measurer| {
                        table.flow.plan.resolve_rows(
                            measurer,
                            &table.flow.heights,
                            table.flow.spacing,
                            &geometry.geometry,
                        )
                    })?;
                    let range = table.flow.plan.cell_rows(index);
                    let assigned =
                        rows.starts[range.end] - rows.starts[range.start] - table.flow.spacing;
                    let content_height = rows.content_height(range, table.flow.spacing);
                    cell.block = if let Some(used_height) =
                        table
                            .flow
                            .plan
                            .cell_used_height(index, content_height, &geometry)
                    {
                        self.measurer.run(|measurer| {
                            table.flow.plan.measure_cell(
                                measurer,
                                index,
                                &geometry,
                                Some(used_height),
                            )
                        })?
                    } else {
                        natural
                    };
                    cell.block.align_cell(rows.offsets[index], assigned);
                    cell.clipped_height = table
                        .flow
                        .plan
                        .cell_rows_collapsed(index)
                        .then_some((assigned - rows.offsets[index]).max(0.0));
                    cell.block.retain_fragment();
                }
            }
            self.pending_break = None;
            let cell_tail = if cell
                .grid_cell
                .zip(row.band.as_ref())
                .is_some_and(|(index, band)| {
                    plan.as_ref().unwrap().cell_rows(index).end != band.end
                }) {
                BreakTail::default()
            } else {
                *tail
            };
            let mut continuation = self.start_cell(
                cell.block,
                cell_tail,
                cells::CellStart {
                    flex_item: cell.flex_item,
                    table_cell: row.band.is_some(),
                    preserve_empty: cell.grid_cell.is_none() && cell.offset > 0.0,
                    orders: Some(orders),
                    grow_after_break: grid_rows.as_ref().is_some_and(|range| {
                        range.len() == 1 && row.grid.as_ref().unwrap().tracks[range.start].grows
                    }),
                    grid_rows,
                },
                None,
            )?;
            if let Some(height) = cell.clipped_height {
                continuation.clip_to(height);
            }
            continuation.after_events = cell.after_events;
            active_cells.push_back((
                continuation,
                cell.stretch,
                cell.after,
                paint_start,
                cell.backgrounds,
                cell.grid_cell,
            ));
        }
        *next_row = row
            .band
            .as_ref()
            .and_then(|band| (source_index + 1 < band.end).then_some(source_index + 1))
            .or_else(|| {
                row.grid.as_ref().and_then(|grid| {
                    (source_index + 1 < grid.tracks.len()).then_some(source_index + 1)
                })
            });
        *boundary = next_row.is_some_and(|next| {
            row.grid.is_some() || constrained_rows.range(next..).next().is_some()
        });
        *entered = true;
        Ok(None)
    }
    pub(super) fn resume_row(
        &mut self,
        cursor: &mut RowCursor<'a, 'a>,
    ) -> Result<Option<BreakRequest>, LayoutError> {
        if cursor.yielded {
            cursor.frontier = self.page;
            cursor.frontier_y = self.y;
            cursor.yielded = false;
        }
        while cursor.source_rows.start < cursor.source_rows.end {
            if !cursor.entered
                && let Some(request) = self.enter_row(cursor)?
            {
                return Ok(Some(request));
            }
            let RowCursor {
                row,
                end,
                stretches,
                plan,
                active_cells,
                origin,
                row_ends,
                source_rows,
                grid_ends,
                entered,
                next_row,
                boundary,
                frontier,
                frontier_y,
                yielded,
                ..
            } = cursor;
            let source_index = source_rows.start;
            let source_row = row.band.as_ref().map(|_| source_index);
            while !active_cells.is_empty() {
                if *boundary {
                    self.measurer.work.spend(active_cells.len())?;
                    if active_cells
                        .iter()
                        .all(|(continuation, _, _, _, _, index)| {
                            let range = continuation.grid_rows.clone().unwrap_or_else(|| {
                                plan.as_ref().unwrap().cell_rows(index.unwrap())
                            });
                            let page = if row.grid.is_some() {
                                row_ends
                                    .get(&(source_index + 1))
                                    .map_or(origin.1, |end| end.0.max(origin.1))
                            } else {
                                end.0.max(origin.1)
                            };
                            range.end > next_row.unwrap()
                                && continuation.next_page(&self.fragmentainers) > page
                        })
                    {
                        break;
                    }
                    let first = active_cells
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, (continuation, _, _, _, _, _))| {
                            continuation.next_page(&self.fragmentainers)
                        })
                        .unwrap()
                        .0;
                    let next = active_cells.remove(first).unwrap();
                    active_cells.push_front(next);
                }
                if self.unbroken == 0 {
                    if active_cells
                        .front()
                        .unwrap()
                        .0
                        .next_page(&self.fragmentainers)
                        > *frontier
                    {
                        self.measurer.work.spend(active_cells.len())?;
                        let first = active_cells
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, (cell, _, _, _, _, _))| {
                                cell.next_page(&self.fragmentainers)
                            })
                            .unwrap()
                            .0;
                        let next = active_cells.remove(first).unwrap();
                        active_cells.push_front(next);
                    }
                    let lane = &active_cells.front().unwrap().0;
                    if lane.next_page(&self.fragmentainers) > *frontier {
                        let mut request = lane.break_request().unwrap_or_default();
                        // Alignment can place a lane beyond an otherwise empty fragment.
                        request.preserve_empty |= lane.position().0 > *frontier;
                        self.select_page(*frontier, *frontier_y);
                        *yielded = true;
                        return Ok(Some(request));
                    }
                }
                let (mut continuation, stretch, after, paint_start, backgrounds, grid_cell) =
                    active_cells.pop_front().unwrap();
                let Some(active) = self.advance_cell(&mut continuation)? else {
                    let (page, y) = continuation.position();
                    if page == *frontier {
                        *frontier_y = frontier_y.max(y);
                    }
                    active_cells.push_back((
                        continuation,
                        stretch,
                        after,
                        paint_start,
                        backgrounds,
                        grid_cell,
                    ));
                    continue;
                };
                if self.page == *frontier {
                    *frontier_y = frontier_y.max(self.y);
                }
                let after_events = std::mem::take(&mut continuation.after_events);
                if stretch {
                    if let Some(index) = grid_cell
                        && !self
                            .tables
                            .last()
                            .unwrap()
                            .flow
                            .plan
                            .cell_has_explicit_height(index)
                    {
                        let height = active
                            .fragments
                            .iter()
                            .map(|&(page, slot)| match &self.display_lists[page][slot].item {
                                DisplayItem::Box(paint) => paint.rect.height,
                                _ => unreachable!("cell fragments are box paints"),
                            })
                            .sum();
                        self.tables.last_mut().unwrap().flow.heights[index].height = height;
                    }
                    stretches.push((
                        active,
                        after,
                        paint_start,
                        backgrounds,
                        grid_cell,
                        after_events,
                    ));
                } else {
                    for event in after_events {
                        self.string_locations.record(self.page, event);
                    }
                }
                self.y += after;
                if let Some(range) = &continuation.grid_rows
                    && range.len() == 1
                {
                    let endpoint = row_ends.entry(range.end).or_insert((self.page, self.y));
                    if self.page > endpoint.0 || (self.page == endpoint.0 && self.y > endpoint.1) {
                        *endpoint = (self.page, self.y);
                    }
                }
                if let Some(index) = grid_cell {
                    let row_end = plan.as_ref().unwrap().cell_rows(index).end;
                    let endpoint = row_ends.entry(row_end).or_insert((self.page, self.y));
                    if self.page > endpoint.0 || (self.page == endpoint.0 && self.y > endpoint.1) {
                        *endpoint = (self.page, self.y);
                    }
                }
                if self.page > end.0 || (self.page == end.0 && self.y > end.1) {
                    *end = (self.page, self.y);
                }
                self.pending.clear();
            }
            if let Some(source_row) = source_row {
                row_ends
                    .entry(source_row + 1)
                    .or_insert((origin.1, origin.2));
            }
            if let (Some(source_row), Some(plan)) = (source_row, plan.as_ref())
                && source_row + 1 < row.band.as_ref().unwrap().end
            {
                let events = plan.row_exit_events(source_row);
                if !events.is_empty() {
                    let position = self.flow_position();
                    let table = self.tables.last().unwrap();
                    let geometry = self.measurer.run(|measurer| {
                        plan.resolve_rows(
                            measurer,
                            &table.flow.heights,
                            table.flow.spacing,
                            &table.flow.geometry,
                        )
                    })?;
                    let height = geometry.starts[source_row + 1]
                        - geometry.starts[origin.0]
                        - table.flow.spacing;
                    self.finish_height(origin.1, origin.2, height)?;
                    for event in events {
                        self.string_locations.record(self.page, event);
                    }
                    self.restore_flow_position(position)?;
                }
            }
            if let Some(grid) = &row.grid {
                let track = &grid.tracks[source_index];
                self.select_page(origin.1, origin.2);
                self.finish_height(origin.1, origin.2, track.area.end - track.area.start)?;
                if track.grows
                    && let Some(&(page, y)) = row_ends.get(&(source_index + 1))
                    && (page > self.page || (page == self.page && y > self.y))
                {
                    self.select_page(page, y);
                }
                grid_ends.push((self.page, self.y));
                if self.page > end.0 || (self.page == end.0 && self.y > end.1) {
                    *end = (self.page, self.y);
                }
            }
            source_rows.start += 1;
            *entered = false;
        }
        Ok(None)
    }
    fn order_grid_text(&mut self, trace: &GridTrace) {
        for (page, lines) in self.pages.iter_mut().enumerate() {
            let start = trace.starts.get(page).copied().unwrap_or(0);
            if lines.len() <= start + 1 {
                continue;
            }
            let mut keys = vec![None; lines.len() - start];
            for paint in &self.foreground[page] {
                if let DisplayItem::Text { line } = paint.item
                    && line >= start
                {
                    keys[line - start] = trace
                        .order
                        .branch_index(&paint.order)
                        .and_then(|branch| trace.source_order.get(&branch))
                        .copied();
                }
            }
            let mut ordered: Vec<_> = lines.drain(start..).enumerate().collect();
            ordered.sort_by_key(|(index, _)| keys[*index]);
            let mut indices = vec![0; ordered.len()];
            for (index, (previous, line)) in ordered.into_iter().enumerate() {
                indices[previous] = start + index;
                lines.push(line);
            }
            for paint in &mut self.foreground[page] {
                if let DisplayItem::Text { line } = &mut paint.item
                    && *line >= start
                {
                    *line = indices[*line - start];
                }
            }
        }
    }

    pub(super) fn finish_row(&mut self, cursor: RowCursor<'a, 'a>) -> Result<(), LayoutError> {
        let RowCursor {
            mut row,
            start,
            start_page,
            last_body,
            repeating,
            end,
            stretches,
            row_ends,
            internal_breaks,
            fragment_ends,
            previous_unbroken,
            body_end,
            trace,
            grid_ends,
            ..
        } = cursor;
        let end = grid_ends.last().copied().unwrap_or(end);
        self.select_page(start_page, start);
        let assigned = row.band.as_ref().and_then(|band| {
            self.tables
                .last()
                .filter(|table| table.flow.plan.has_height_constraints)
                .map(|table| {
                    table.flow.row_starts[band.end]
                        - table.flow.row_starts[band.start]
                        - table.flow.spacing
                })
        });
        if row.band.is_none() || end.0 == start_page || assigned.is_some() {
            self.finish_height(start_page, start, assigned.unwrap_or(row.height))?;
        }
        if end.0 > self.page || (end.0 == self.page && end.1 > self.y) {
            self.select_page(end.0, end.1);
        }
        if !row.collapsed.is_empty() {
            let position = self.flow_position();
            for item in &row.collapsed {
                self.select_page(start_page, start);
                self.finish_height(start_page, start, item.offset)?;
                self.record_collapsed(item.source)?;
                for event in &item.after_events {
                    self.string_locations.record(self.page, event);
                }
            }
            self.restore_flow_position(position)?;
        }
        let mut row_end = (self.page, self.y);
        let geometry =
            if !stretches.is_empty() && row.band.as_ref().is_some_and(|band| band.len() > 1) {
                let table = self.tables.last().unwrap();
                Some(self.measurer.run(|measurer| {
                    table.flow.plan.resolve_rows(
                        measurer,
                        &table.flow.heights,
                        table.flow.spacing,
                        &table.flow.geometry,
                    )
                })?)
            } else {
                None
            };
        let mut fragments: BTreeMap<usize, Rect> = BTreeMap::new();
        for (active, after, paint_start, backgrounds, grid_cell, after_events) in stretches {
            if let Some(geometry) = &geometry
                && let Some(cell) = grid_cell
            {
                let table = self.tables.last().unwrap();
                let rows = table.flow.plan.cell_rows(cell);
                let height = geometry.starts[rows.end]
                    - geometry.starts[row.band.as_ref().unwrap().start]
                    - table.flow.spacing;
                if let Some(&(page, y)) = row_ends.get(&rows.end).filter(|_| internal_breaks) {
                    self.select_page(page, y);
                } else {
                    self.select_page(start_page, start);
                    self.finish_height(start_page, start, height)?;
                }
                if self.page > row_end.0 || (self.page == row_end.0 && self.y > row_end.1) {
                    row_end = (self.page, self.y);
                }
            }
            for event in after_events {
                self.string_locations.record(self.page, event);
            }
            for (page, mut rect) in
                self.stretch_box(active, after, paint_start, &backgrounds, &fragment_ends)?
            {
                if let Some(cell) = grid_cell
                    && self
                        .tables
                        .last()
                        .is_some_and(|table| table.flow.plan.has_scoped_internals)
                {
                    let geometry = self.table_geometry(page)?;
                    let relative = self
                        .tables
                        .last()
                        .unwrap()
                        .flow
                        .plan
                        .cell_offset(cell, geometry.geometry.width);
                    rect.x -= relative[0];
                    rect.y -= relative[1];
                }
                fragments
                    .entry(page)
                    .and_modify(|area| *area = area.union(rect))
                    .or_insert(rect);
            }
        }
        self.select_page(row_end.0, row_end.1);
        self.reflow_open_boxes()?;
        if let Some(band) = &row.band {
            let plan = self.tables.last().unwrap().flow.plan.clone();
            for event in plan.row_exit_events(band.end - 1) {
                self.string_locations.record(self.page, event);
            }
        }
        if let Some(band) = row.band
            && self.tables.last().is_some_and(|table| {
                table.flow.collapsed.is_some()
                    || table.flow.plan.has_structural_geometry
                    || table.flow.plan.has_containing_ancestors
                    || !table.shadows.is_empty()
            })
        {
            if fragments.is_empty() {
                self.tables.last_mut().unwrap().placements.push((
                    start_page,
                    BandPlacement {
                        rows: band,
                        y: self.fragmentainers.top(start_page) + start + self.offset[1],
                        slice: None,
                    },
                ));
            } else {
                let mut before = 0.0;
                for (page, rect) in fragments {
                    let geometry = self.table_geometry(page)?;
                    let table = self.tables.last_mut().unwrap();
                    if let Some(source) = &table.flow.collapsed {
                        let grid = crate::layout::tables::CollapsedGrid {
                            borders: source.borders.clone(),
                            offsets: source.offsets.clone(),
                            lines: geometry.grid_lines(table.flow.plan.source.style().used_rtl()),
                        };
                        table.grids.entry(page).or_insert(grid);
                    }
                    table.placements.push((
                        page,
                        BandPlacement {
                            rows: band.clone(),
                            y: rect.y,
                            slice: Some(before..before + rect.height),
                        },
                    ));
                    before += rect.height;
                }
            }
        }
        if repeating {
            let band = self.bands.pop().expect("active row owns its continuation");
            self.emit_band_groups(band, last_body, Forced::of(row.break_after))?;
        }
        if let Some(trace) = &trace {
            self.order_grid_text(trace);
        }
        self.paint_row_after(std::mem::take(&mut row.after), start_page, start)?;
        self.has_flow |= row.height > 0.0;
        self.at_page_start = false;
        self.pending_break = Forced::of(row.break_after);
        self.unbroken = previous_unbroken;
        if let Some(end) = body_end {
            self.tables.last_mut().unwrap().next_body = end;
        }
        Ok(())
    }

    fn paint_row_after(
        &mut self,
        paints: Vec<RowPaint<'a>>,
        page: usize,
        top: f32,
    ) -> Result<(), LayoutError> {
        if paints.is_empty() {
            return Ok(());
        }
        let end = (self.page, self.y);
        let flow = (self.has_flow, self.at_page_start, self.margin_start);
        let pending = std::mem::take(&mut self.pending);
        self.select_page(page, top);
        for paint in paints {
            self.measurer.work.spend(1)?;
            match paint {
                RowPaint::Text(text) => self.row_text(*text, top)?,
                RowPaint::Box(paint) => {
                    let mut paint = *paint;
                    for rect in
                        std::iter::once(&mut paint.rect).chain(paint.background_box.as_mut())
                    {
                        rect.x += self.offset[0];
                        rect.y += self.top + top + self.offset[1];
                    }
                    self.record_overflow(paint.rect);
                    let visible =
                        !self.force_hidden && self.boxes.last().is_none_or(|active| active.visible);
                    if visible && paint.has_ink() {
                        if let Some(geometry) = self
                            .boxes
                            .last()
                            .and_then(|active| active.geometry.as_ref())
                        {
                            self.anchors.ink(
                                crate::layout::anchors::ink_key(geometry.source.style()),
                                self.page,
                                crate::layout::anchors::box_ink_in_frame(
                                    &paint,
                                    crate::layout::writing::Frame::for_style(
                                        geometry.source.style(),
                                    ),
                                ),
                                self.clip,
                            );
                        }
                        self.page_content |= paint.rect.intersects(self.page_rect);
                        let scope = self.layer_scope(&self.current_layer());
                        self.foreground[self.page].push(ScopedPaint {
                            native: None,
                            visible,
                            order: self.orders.take(),
                            scope,
                            clip: self.clip,
                            group: self.group,
                            ink: None,
                            item: DisplayItem::Box(paint),
                            navigation: BoxNavigation::default(),
                        });
                    }
                }
            }
        }
        self.select_page(end.0, end.1);
        (self.has_flow, self.at_page_start, self.margin_start) = flow;
        self.pending = pending;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
