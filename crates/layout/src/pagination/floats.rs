#[allow(
    clippy::wildcard_imports,
    reason = "pagination submodules extend the parent paginator and share its scope"
)]
use super::*;

pub(super) struct MeasuredFloat<'a> {
    block: MeasuredBlock<'a>,
    margins: [f32; 4],
    pub size: [f32; 2],
    reflow: bool,
}

#[derive(Clone, Copy)]
pub(super) struct FloatFlow {
    pub context: usize,
    pub top: f32,
}

#[derive(Clone)]
pub(super) struct FloatPlacement {
    pub fragments: Vec<(usize, Rect)>,
    pub reflow: bool,
    first_top_margin: f32,
    fragment_top_margin: f32,
    margins: [f32; 4],
    containing_width: f32,
    offset: [f32; 2],
}

impl FloatPlacement {
    pub(super) fn reflow(&mut self, geometry: &ActiveGeometry<'_>, offset: [f32; 2]) {
        self.margins = geometry
            .horizontal
            .margin
            .map(|margin| margin.unwrap_or(0.0));
        self.containing_width = geometry.horizontal.containing_width;
        self.offset = offset;
    }

    pub(super) fn begin_fragment(&mut self, start: f32, clone: bool) -> f32 {
        self.fragment_top_margin = if clone && start > 0.0 {
            self.margins[0]
        } else {
            0.0
        };
        self.fragment_top_margin
    }

    pub(super) fn record(
        &mut self,
        page: usize,
        top: f32,
        paint: &BoxPaint,
        geometry: &ActiveGeometry<'_>,
        last: bool,
    ) {
        let style = geometry.source.style();
        let relative =
            style.relative_offset(self.containing_width, geometry.containing_height, style.rtl);
        self.fragments
            .truncate(self.fragments.partition_point(|(index, _)| *index < page));
        let before = if self.fragments.is_empty() {
            self.first_top_margin
        } else {
            self.fragment_top_margin
        };
        let after = if last { self.margins[2] } else { 0.0 };
        self.fragments.push((
            page,
            Rect {
                x: paint.rect.x - self.offset[0] - relative[0] - self.margins[3],
                y: paint.rect.y - top - self.offset[1] - relative[1] - before,
                width: paint.rect.width + self.margins[1] + self.margins[3],
                height: paint.rect.height + before + after,
            },
        ));
    }
}

impl<'a> Paginator<'a, '_, '_> {
    pub(super) fn reflow_float_geometry(
        &self,
        page: usize,
        flow: &mut ActiveGeometry<'_>,
    ) -> Result<(), LayoutError> {
        let Some(side) = flow.source.style().floating() else {
            return Ok(());
        };
        let floating = flow
            .floating
            .as_mut()
            .expect("floating geometry retains its context");
        let geometry = &mut flow.horizontal;
        let margins = geometry.margin.map(|margin| margin.unwrap_or(0.0));
        let outer_width = geometry.border_box(flow.height).width + margins[1] + margins[3];
        let empty = crate::floats::FloatContext::default();
        let context = self.floats.get(floating.context).unwrap_or(&empty);
        let placed = context.place(
            page,
            0.0,
            [
                geometry.containing_x,
                geometry.containing_x + geometry.containing_width,
            ],
            [outer_width, 0.0],
            side,
            &self.measurer.work,
        )?;
        floating.top = placed.y;
        geometry.x = placed.x + margins[3] + geometry.border[3] + geometry.padding[3];
        Ok(())
    }

    pub(super) fn avoid_floats(&mut self, block: &MeasuredBlock<'a>) -> Result<f32, LayoutError> {
        let replaced = matches!(block.items.first(), Some(FlowItem::Image { .. }));
        if (!block.independent && !replaced)
            || block.style.display.is_inline_level()
            || block.style.floating().is_some()
            || block.style.position.is_out_of_flow()
            || self.flex_item
        {
            return Ok(0.0);
        }
        let context = self.floats.last().expect("flow has a float context");
        if context.is_empty() {
            return Ok(0.0);
        }
        let border = block.geometry.border_box(block.height);
        let bounds = [
            block.geometry.containing_x,
            block.geometry.containing_x + block.geometry.containing_width,
        ];
        let margin = self.pending_value();
        let mut y = self.y + margin;
        loop {
            self.measurer.work.spend(1)?;
            let (band, next) =
                context.band(self.page, y, border.height, bounds, &self.measurer.work)?;
            if border.width <= band[1] - band[0] || next.is_none() {
                let x = if next.is_none() {
                    border.x
                } else {
                    border.x.max(band[0]).min(band[1] - border.width)
                };
                self.has_flow |= y > self.y + margin;
                self.y = y - margin;
                return Ok(x - border.x);
            }
            y = next.expect("an obstructing float has a bottom edge");
        }
    }

    pub(super) fn resolve_clearance_margins(
        &mut self,
        block: MeasuredBlock<'a>,
    ) -> Result<MeasuredBlock<'a>, LayoutError> {
        if block.clearance_before.is_none() || block.independent || block.start_space != 0.0 {
            return Ok(block);
        }
        let hypothetical = self.y + self.pending_value();
        let mut snapshot = None;
        let result = (|| -> Result<_, LayoutError> {
            let mut pending = vec![(block.items.iter(), &block)];
            let mut targets = HashMap::new();
            while let Some((items, parent)) = pending.last_mut() {
                self.measurer.work.spend(1)?;
                let Some(item) = items.next() else {
                    pending.pop();
                    continue;
                };
                match item {
                    FlowItem::Edge(_, space) if *space == 0.0 => {}
                    FlowItem::Positioned(value) => {
                        if value.block.style().floating().is_some() {
                            if snapshot.is_none() {
                                snapshot = Some(self.snapshot()?);
                            }
                            let height = crate::geometry::VerticalBox::new(
                                parent.style,
                                &parent.geometry,
                                parent.containing_height,
                            )
                            .definite();
                            let width = value.right - value.left;
                            let mut measured = self.measure_float(value.block, width, height)?;
                            // The preflight parent is measured but not yet active.
                            measured.reflow = false;
                            self.place_measured_float(
                                measured,
                                value.left,
                                width,
                                hypothetical + value.margin,
                            )?;
                        }
                    }
                    FlowItem::Block(child) => {
                        let end = self
                            .floats
                            .last()
                            .expect("flow has a float context")
                            .clearance_end(
                                self.page,
                                child.style.clear,
                                child.style.rtl,
                                &self.measurer.work,
                            )?;
                        if end
                            .is_some_and(|(page, bottom)| page > self.page || bottom > hypothetical)
                            && child.clearance_target.is_none()
                        {
                            targets.insert(std::ptr::from_ref::<BlockBox>(child.source), self.page);
                            break;
                        }
                        if !child.through {
                            pending.clear();
                        }
                        if (child.through || child.clearance_before.is_some())
                            && !child.independent
                            && child.start_space == 0.0
                        {
                            pending.push((child.items.iter(), child));
                        }
                    }
                    _ => break,
                }
            }
            Ok(targets)
        })();
        if let Some(snapshot) = snapshot {
            self.restore_state(snapshot);
        }
        let targets = result?;
        if targets.is_empty() {
            return Ok(block);
        }
        let previous = std::mem::replace(&mut self.measurer.clearance_targets, targets);
        let result = self.measurer.format_block(
            block.source,
            block.geometry.clone(),
            false,
            block.containing_height,
            None,
        );
        self.measurer.clearance_targets = previous;
        let mut resolved = result?;
        resolved.clearance_before = block.clearance_before;
        let mut margin = resolved
            .clearance_before
            .as_deref()
            .cloned()
            .unwrap_or_default();
        margin.merge(resolved.before.clone());
        if let Some(index) = self
            .pending
            .iter()
            .rposition(|value| value.belongs_to(block.source))
        {
            self.pending[index] = margin.for_clearance(block.source);
        }
        Ok(resolved)
    }

    pub(super) fn float_enclosure(
        &self,
        frame: &BlockFrame<'a>,
    ) -> Result<Option<f32>, LayoutError> {
        if !frame.independent {
            return Ok(None);
        }
        let Some((mut page, mut bottom)) = self
            .floats
            .last()
            .expect("flow has a float context")
            .end(&self.measurer.work)?
        else {
            return Ok(None);
        };
        let active = self.boxes.last().expect("enclosing block is active");
        bottom += active.end_space;
        if self.page > page || (self.page == page && self.y > bottom) {
            (page, bottom) = (self.page, self.y);
        }
        let mut natural = bottom;
        for fragment in frame.start_page..page {
            self.measurer.work.spend(1)?;
            natural += self.fragmentainers.height(fragment)
                - self.cloned_end_space()
                - self.band_footer_height(fragment);
            if fragment > frame.start_page {
                natural -= self.repeated_start(fragment, self.boxes.len());
            }
        }
        natural -= if page == frame.start_page {
            frame.start_y
        } else {
            frame.start_y + self.repeated_start(page, self.boxes.len())
        };
        Ok(Some(natural.max(0.0)))
    }

    pub(super) fn clear_block(&mut self, block: &MeasuredBlock<'a>) -> Result<(), LayoutError> {
        if block.style.display.is_inline_level() || self.flex_item {
            return Ok(());
        }
        let Some(before) = &block.clearance_before else {
            return Ok(());
        };
        if self.unbroken == 0 {
            while self
                .floats
                .last()
                .expect("flow has a float context")
                .clearance_end(
                    self.page,
                    block.style.clear,
                    block.style.rtl,
                    &self.measurer.work,
                )?
                .is_some_and(|(page, _)| page > self.page)
            {
                self.new_page(None, true)?;
            }
        }
        let hypothetical = self.y + self.pending_value();
        let mut bottom = self
            .floats
            .last()
            .expect("flow has a float context")
            .clearance(
                self.page,
                hypothetical,
                block.style.clear,
                block.style.rtl,
                &self.measurer.work,
            )?;
        let targeted = block.clearance_target == Some(self.page);
        if targeted {
            bottom = self
                .floats
                .last()
                .expect("flow has a float context")
                .clearance(
                    self.page,
                    f32::NEG_INFINITY,
                    block.style.clear,
                    block.style.rtl,
                    &self.measurer.work,
                )?;
        }
        if !bottom.is_finite() || (!targeted && bottom <= hypothetical) {
            return Ok(());
        }
        let width = block.geometry.containing_width;
        let before = before.resolve(width, self.measurer)?.value();
        let after = block.before.resolve(width, self.measurer)?.value();
        let separated = before + after;
        // CSS 2.1 §9.5.2: clearance itself may be negative after margins stop collapsing.
        let clearance = (bottom - self.y - separated)
            .max(if targeted { bottom } else { hypothetical } - self.y - separated);
        self.pending.clear();
        self.y += separated + clearance;
        self.at_page_start = false;
        Ok(())
    }

    pub(super) fn measure_float(
        &mut self,
        source: &'a BlockBox,
        width: f32,
        containing_height: Option<f32>,
    ) -> Result<MeasuredFloat<'a>, LayoutError> {
        let style = source.style();
        let previous = self.measurer.fragmentainer;
        self.measurer.fragmentainer = self.fragmentainers.kind();
        let measured = crate::inline::measure_shrink_to_fit(
            self.measurer,
            source,
            width,
            containing_height,
            style.rtl,
        );
        self.measurer.fragmentainer = previous;
        let (block, geometry) = measured?;
        let margins = geometry.margin.map(|margin| margin.unwrap_or(0.0));
        let size = [
            geometry.border_box(block.height).width + margins[1] + margins[3],
            block.height + margins[0] + margins[2],
        ];
        Ok(MeasuredFloat {
            reflow: true,
            block,
            margins,
            size,
        })
    }

    pub(super) fn place_float(
        &mut self,
        source: &'a BlockBox,
        left: f32,
        width: f32,
        y: f32,
    ) -> Result<(), LayoutError> {
        let measured = self.measure_float(source, width, self.containing_geometry().3)?;
        self.place_measured_float(measured, left, width, y)
    }

    pub(super) fn place_measured_float(
        &mut self,
        mut measured: MeasuredFloat<'a>,
        mut left: f32,
        mut width: f32,
        mut y: f32,
    ) -> Result<(), LayoutError> {
        let parent = self.flow_position();
        if let Some((page, start)) = self
            .floats
            .last()
            .expect("flow has a float context")
            .last_start()
            && page > self.page
        {
            self.select_page(page, start);
            self.reflow_open_boxes()?;
            (left, width, _, _) = self.containing_geometry();
            let reflow = measured.reflow;
            measured =
                self.measure_float(measured.block.source, width, self.containing_geometry().3)?;
            measured.reflow = reflow;
            y = start;
        }
        let (measured, mut placement) = self.position_float(measured, left, width, y)?;
        let style = measured.style;
        let side = style.floating().expect("float source has an active side");
        let mut flow = self.start_cell(
            measured,
            BreakTail::default(),
            cells::CellStart::default(),
            Some(&mut placement),
        )?;
        let active = loop {
            self.measurer.work.spend(1)?;
            if let Some(active) = self.advance_cell(&mut flow)? {
                break active;
            }
        };
        let exclusions = active
            .float_placement
            .expect("floating boxes retain fragment geometry")
            .fragments;
        self.restore_flow_position(parent)?;
        let context = self.floats.last_mut().expect("flow has a float context");
        context.insert(side, exclusions);
        Ok(())
    }
    fn position_float(
        &mut self,
        measured: MeasuredFloat<'a>,
        left: f32,
        width: f32,
        y: f32,
    ) -> Result<(MeasuredBlock<'a>, FloatPlacement), LayoutError> {
        let MeasuredFloat {
            block: mut measured,
            mut margins,
            mut size,
            reflow,
        } = measured;
        if self.at_page_start && matches!(self.margin_start, MarginStart::Unforced) {
            size[1] -= margins[0];
            margins[0] = 0.0;
        }
        let style = measured.style;
        let side = style.floating().expect("float source has an active side");
        let context = self.floats.last().expect("flow has a float context");
        let y = context.clearance(self.page, y, style.clear, style.rtl, &self.measurer.work)?;
        let placed = context.place(
            self.page,
            y,
            [left, left + width],
            size,
            side,
            &self.measurer.work,
        )?;
        let placement = FloatPlacement {
            fragments: Vec::new(),
            reflow,
            first_top_margin: margins[0],
            fragment_top_margin: margins[0],
            margins,
            containing_width: width,
            offset: self.offset,
        };
        measured.retain_fragment();
        self.y = placed.y + margins[0];
        self.offset[0] += placed.x;
        self.pending.clear();
        self.pending_break = None;
        Ok((measured, placement))
    }

    pub(super) fn reposition_float(
        &mut self,
        block: &MeasuredBlock<'a>,
        placement: &mut FloatPlacement,
    ) -> Result<MeasuredBlock<'a>, LayoutError> {
        self.reflow_box_geometry()?;
        let (left, width, _, _) = self.containing_geometry();
        let mut measured = self.measure_float(block.source, width, self.containing_geometry().3)?;
        measured.reflow = placement.reflow;
        let (block, updated) = self.position_float(measured, left, width, self.y)?;
        *placement = updated;
        Ok(block)
    }
}
