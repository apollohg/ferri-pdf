//! Shared paint, effect and navigation adoption for independently laid-out flows.
use super::{Effect, EffectNode, Group, Paginator, ScopedPaint, StickyConstraints};
use crate::graphics::{DisplayItem, Rect};
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub(super) struct NativeSpace {
    pub(super) frame: crate::layout::writing::Frame,
    placement: [f32; 6],
}

impl NativeSpace {
    pub(super) fn matrix(self) -> [f32; 6] {
        crate::layout::writing::compose(self.placement, self.frame.matrix)
    }
    pub(super) fn placed(self, matrix: [f32; 6]) -> Self {
        Self {
            placement: crate::layout::writing::compose(matrix, self.placement),
            ..self
        }
    }
    pub(super) fn point(self, point: crate::graphics::Point) -> crate::graphics::Point {
        crate::layout::writing::map_point(self.matrix(), point)
    }
    pub(super) fn rect(self, rect: Rect) -> Rect {
        crate::layout::writing::map_rect(self.matrix(), rect)
    }
    pub(super) fn box_ink(self, paint: &crate::graphics::BoxPaint) -> Rect {
        self.rect(crate::layout::anchors::box_ink_in_frame(paint, self.frame))
    }
    pub(super) fn region(self, region: super::Region) -> super::Region {
        use crate::layout::writing::{compose, map_clip};
        match region {
            super::Region::Clip(clip) => super::Region::Clip(map_clip(self.matrix(), clip)),
            super::Region::Transform(matrix) => {
                let space = kurbo::Affine::new(self.matrix().map(f64::from));
                super::Region::Transform(compose(
                    self.matrix(),
                    compose(matrix, space.inverse().as_coeffs().map(|v| v as f32)),
                ))
            }
        }
    }

    fn mapped(self, mut items: Vec<DisplayItem>, restore: DisplayItem) -> Vec<DisplayItem> {
        self.frame.items(&mut items);
        let mut mapped = Vec::with_capacity(items.len() + 2);
        mapped.push(DisplayItem::PushTransform(self.placement));
        mapped.extend(items);
        mapped.push(restore);
        mapped
    }
    pub(super) fn paint(self, item: DisplayItem) -> Vec<DisplayItem> {
        self.mapped(vec![item], DisplayItem::Pop)
    }
    pub(super) fn pushes(self, items: Vec<DisplayItem>) -> Vec<DisplayItem> {
        if items.is_empty() {
            return items;
        }
        let inverse = kurbo::Affine::new(self.placement.map(f64::from))
            .inverse()
            .as_coeffs()
            .map(|value| value as f32);
        self.mapped(items, DisplayItem::PushTransform(inverse))
    }
}

impl<'a> Paginator<'a, '_, 'a> {
    pub(super) fn place_image(
        &mut self,
        mut paint: crate::graphics::ImagePaint,
        height: f32,
        visible: bool,
        rotate: bool,
    ) {
        self.flush_margin();
        let mut normal_rect = paint.visible_rect();
        normal_rect.y += self.top + self.y;
        self.flow_content[self.page] |= normal_rect.intersects(self.page_rect);
        paint.rect.x += self.offset[0];
        paint.rect.y += self.top + self.y + self.offset[1];
        if let Some(clip) = &mut paint.clip {
            clip.translate([self.offset[0], self.top + self.y + self.offset[1]]);
        }
        if paint.visible_rect().intersects(self.page_rect)
            && paint.rect.intersects(paint.visible_rect())
        {
            let scope = self.inline_key();
            let item = if rotate {
                DisplayItem::Frame(Box::new(crate::graphics::FlowPaint {
                    matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    area: paint.rect,
                    items: vec![DisplayItem::Image(paint)],
                    lines: Vec::new(),
                }))
            } else {
                DisplayItem::Image(paint)
            };
            self.foreground[self.page].push(ScopedPaint {
                native: None,
                visible,
                order: self.orders.take(),
                scope,
                clip: self.clip,
                group: self.group,
                ink: None,
                item,
                navigation: crate::document::BoxNavigation::default(),
            });
            self.page_content = true;
        }
        self.y += height;
        self.has_flow = true;
        self.at_page_start = false;
    }

    fn advance_frame(&mut self, height: f32) {
        self.y += height;
        self.has_flow = true;
        self.at_page_start = false;
        self.page_content = true;
    }

    pub(super) fn place_frame(
        &mut self,
        inner: std::rc::Rc<super::MeasuredBlock<'a>>,
        area: Rect,
        height: f32,
        baselines: [Option<f32>; 2],
    ) -> Result<(), crate::layout::LayoutError> {
        use crate::layout::writing::Frame;
        self.flush_margin();
        let origin = self.top + self.y + self.offset[1];
        self.record_baselines(
            self.page,
            baselines.map(|baseline| baseline.map(|y| y + origin)),
        );
        if self.measurer.column_measurement {
            self.advance_frame(height);
            return Ok(());
        }
        let own = crate::style::logical::Flow::used(inner.style);
        let parent = self
            .boxes
            .last()
            .and_then(|active| active.geometry.as_ref())
            .map_or(own, |active| active.source.layout_flow());
        let mut size = [area.width, area.height];
        if parent.writing_mode.vertical() {
            size.swap(0, 1);
        }
        let mut matrix = Frame::physical(parent, size).inverse().matrix;
        matrix[4] += area.x + self.offset[0];
        matrix[5] += area.y + self.top + self.y + self.offset[1];
        let space = NativeSpace {
            frame: Frame::physical(own, size),
            placement: matrix,
        };
        let key = super::frames::Key::of(&inner);
        let ready = self.frame_templates.borrow().completed.contains_key(&key);
        if self.template && !ready {
            self.frame_requests.push(inner);
            self.advance_frame(height);
            return Ok(());
        }
        if !ready {
            super::frames::prepare(self.measurer, inner, &self.frame_templates)?;
        }
        self.measurer.work.spend(super::frames::payload_work(
            &self.frame_templates.borrow().completed[&key],
        ))?;
        if self.template {
            self.frame_reads.push(key.clone());
        }
        let mut output = self.frame_templates.borrow().completed[&key].clone();
        let lane = self.orders.take();
        super::frames::rebase(&mut output, &self.context, &self.key, &lane);
        if let Some(bounds) = output
            .root_overflow
            .as_ref()
            .and_then(|bounds| bounds.get(&0))
        {
            self.record_overflow(space.rect(*bounds));
        }
        let lines = self.pages[self.page].len();
        self.pages[self.page].append(&mut output.pages[0]);
        let place = Placement {
            column: 0,
            page: self.page,
            shift: [0.0; 2],
            lines,
            slots: self.display_lists[self.page].len(),
            native: Some(space),
        };
        let mut effects = HashMap::new();
        for paint in std::mem::take(&mut output.display_lists[0]) {
            let paint = self.adopt(paint, &output.clips, &output.sticky, &place, &mut effects);
            self.display_lists[self.page].push(paint);
        }
        for paint in std::mem::take(&mut output.foreground[0]) {
            let paint = self.adopt(paint, &output.clips, &output.sticky, &place, &mut effects);
            self.foreground[self.page].push(paint);
        }
        let mut anchors = std::mem::take(&mut self.anchors);
        anchors.adopt(
            &mut output.anchors,
            (0, self.page, place.slots),
            |rect| space.rect(rect),
            |clip| {
                clip.map(|clip| {
                    self.effect(&output.clips, &output.sticky, clip, &place, &mut effects)
                })
                .or(self.clip)
            },
        );
        self.anchors = anchors;
        self.adopt_underlays(
            &mut output.underlays,
            &output.clips,
            &output.sticky,
            &place,
            &mut effects,
        );
        debug_assert!(
            output.underlays.is_empty(),
            "native frames have one fragmentainer"
        );
        let mut pending = output
            .containers
            .pop()
            .map(|c| c.pending)
            .unwrap_or_default();
        pending.append(&mut output.fixed);
        for mut pending in pending {
            let rect = space.rect(Rect {
                x: pending.position.left,
                y: pending.position.y,
                width: pending.position.right - pending.position.left,
                height: pending.position.bottom - pending.position.y,
            });
            pending.position.page = self.page;
            pending.position.left = rect.x;
            pending.position.right = rect.x + rect.width;
            pending.position.y = rect.y;
            pending.position.bottom = rect.y + rect.height;
            pending.position.alignment = pending
                .position
                .alignment
                .map(|alignment| alignment.mapped(space.matrix()));
            pending.group = match pending.group {
                Some(node) => {
                    Some(self.effect(&output.clips, &output.sticky, node, &place, &mut effects))
                }
                None => self.group,
            };
            pending.parent_3d = pending
                .parent_3d
                .map(|node| self.effect(&output.clips, &output.sticky, node, &place, &mut effects));
            self.enqueue(pending);
        }
        self.string_locations
            .adopt_column(&mut output.string_locations, 0, self.page);
        self.advance_frame(height);
        Ok(())
    }

    pub(super) fn adopt_underlays(
        &mut self,
        underlays: &mut HashMap<(usize, usize), Vec<ScopedPaint>>,
        nodes: &[EffectNode<'a>],
        sticky: &[StickyConstraints],
        place: &Placement,
        effects: &mut HashMap<(usize, Option<usize>), usize>,
    ) {
        let slots: Vec<_> = underlays
            .keys()
            .copied()
            .filter(|(column, _)| *column == place.column)
            .collect();
        for source in slots {
            let paints: Vec<_> = underlays
                .remove(&source)
                .unwrap()
                .into_iter()
                .map(|paint| self.adopt(paint, nodes, sticky, place, effects))
                .collect();
            self.underlays
                .entry((place.page, place.slots + source.1))
                .or_default()
                .extend(paints);
        }
    }

    /// Adopts a nested fragment's paint under the enclosing flow's effects.
    pub(super) fn adopt(
        &mut self,
        mut paint: ScopedPaint,
        nodes: &[EffectNode<'a>],
        sticky: &[StickyConstraints],
        place: &Placement,
        effects: &mut HashMap<(usize, Option<usize>), usize>,
    ) -> ScopedPaint {
        let [dx, dy] = place.shift;
        let shift = |rect: &mut Rect| {
            rect.x += dx;
            rect.y += dy;
        };
        let native = place.space(paint.native);
        if native.is_some() {
            if let DisplayItem::Text { line } = &mut paint.item {
                *line += place.lines;
            }
        } else {
            match &mut paint.item {
                DisplayItem::Frame(frame) => {
                    frame.matrix[4] += dx;
                    frame.matrix[5] += dy;
                }
                DisplayItem::Box(fragment) => {
                    shift(&mut fragment.rect);
                    fragment.background_box.as_mut().map(shift);
                }
                DisplayItem::Image(image) => {
                    shift(&mut image.rect);
                    if let Some(clip) = &mut image.clip {
                        clip.translate([dx, dy]);
                    }
                }
                DisplayItem::Decoration(decoration) => shift(&mut decoration.rect),
                DisplayItem::TextShadow(shadow) => shadow.translate([dx, dy]),
                DisplayItem::Text { line } => *line += place.lines,
                DisplayItem::Backdrop(backdrop) => backdrop.border.translate([dx, dy]),
                DisplayItem::PushClip(_)
                | DisplayItem::PushTransform(_)
                | DisplayItem::PushOpacity(_)
                | DisplayItem::PushBlend(_)
                | DisplayItem::PushFilter(_)
                | DisplayItem::PushMask(_)
                | DisplayItem::PushBackdropRoot
                | DisplayItem::Pop => {}
            }
            paint.ink.as_mut().map(shift);
        }
        paint.native = native;
        // Items outside the columns' own effects take those enclosing the flow.
        paint.clip = match paint.clip {
            Some(node) => Some(self.effect(nodes, sticky, node, place, effects)),
            None => self.clip,
        };
        paint.group = match paint.group {
            Some(node) => Some(self.effect(nodes, sticky, node, place, effects)),
            None => self.group,
        };
        paint
    }

    /// The node standing for a column's effect `node` on its page. Clips and transforms
    /// resolve from their fragment in each column; groups have no geometry.
    pub(super) fn effect(
        &mut self,
        nodes: &[EffectNode<'a>],
        sticky: &[StickyConstraints],
        node: usize,
        place: &Placement,
        effects: &mut HashMap<(usize, Option<usize>), usize>,
    ) -> usize {
        let source = &nodes[node];
        if matches!(source.effect, Effect::InheritedParent3d)
            && let Some(parent) = self.parent_3d
        {
            return parent;
        }
        // Anchored groups follow their element's clips, which resolve per column.
        let group = matches!(source.effect, Effect::Group(group) if !group.anchored());
        let key = (node, (!group).then_some(place.column));
        if let Some(&index) = effects.get(&key) {
            return index;
        }
        let grouping = matches!(source.effect, Effect::Group(_));
        let parent = match source.parent {
            Some(parent) => Some(self.effect(nodes, sticky, parent, place, effects)),
            None if grouping => self.group,
            None => self.clip,
        };
        if let Effect::Group(effect) = source.effect
            && effect.blend != crate::graphics::BlendMode::Normal
        {
            self.isolate(parent);
        }
        let adopt_fragments = |fragments: &[(usize, usize)]| -> Vec<(usize, usize)> {
            fragments
                .iter()
                .filter(|(column, _)| !group && *column == place.column)
                .map(|(_, slot)| (place.page, place.slots + slot))
                .collect()
        };
        let fragments = adopt_fragments(&source.fragments);
        let areas = source
            .areas
            .iter()
            .filter(|(column, _)| *column == place.column)
            .map(|(_, rect)| {
                let mut rect = *rect;
                if place.space(source.native).is_none() {
                    rect.x += place.shift[0];
                    rect.y += place.shift[1];
                }
                (place.page, rect)
            })
            .collect();
        let effect = match source.effect {
            Effect::Transform(style, placement, reference) => Effect::Transform(
                style,
                placement.map(|node| self.effect(nodes, sticky, node, place, effects)),
                reference.map(|node| self.effect(nodes, sticky, node, place, effects)),
            ),
            Effect::Sticky(style, index) => {
                let constraints = &sticky[index];
                let adopted = StickyConstraints {
                    containing: adopt_fragments(&constraints.containing),
                    scrollport: constraints.scrollport.as_deref().map(adopt_fragments),
                    ..constraints.clone()
                };
                let index = self.sticky.len();
                if adopted.scrollport.is_none()
                    && let Some(scroller) = self.boxes.iter_mut().rev().find(|active| {
                        active
                            .geometry
                            .as_ref()
                            .is_some_and(|geometry| geometry.source.style().is_scroll_container())
                    })
                {
                    scroller.sticky_scrolled.push(index);
                }
                self.sticky.push(adopted);
                Effect::Sticky(style, index)
            }
            Effect::Group(group) if group.anchored() => Effect::Group(Group {
                anchor: match group.anchor {
                    Some(anchor) => Some(self.effect(nodes, sticky, anchor, place, effects)),
                    None => self.clip,
                },
                ..group
            }),
            effect => effect,
        };
        let paints = source
            .paints
            .iter()
            .filter(|(column, _)| *column == place.column)
            .map(|(_, paint)| {
                let mut paint = paint.clone();
                if place.space(source.native).is_none() {
                    for rect in
                        std::iter::once(&mut paint.rect).chain(paint.background_box.as_mut())
                    {
                        rect.x += place.shift[0];
                        rect.y += place.shift[1];
                    }
                }
                (place.page, paint)
            })
            .collect();
        self.clips.push(EffectNode {
            native: place.space(source.native),
            areas,
            paints,
            parent,
            effect,
            fragments,
            context: source.context.clone(),
        });
        let index = self.clips.len() - 1;
        effects.insert(key, index);
        index
    }
}

/// Where a nested fragment's items are placed in the enclosing flow.
pub(super) struct Placement {
    pub(super) column: usize,
    pub(super) page: usize,
    pub(super) shift: [f32; 2],
    pub(super) lines: usize,
    pub(super) slots: usize,
    pub(super) native: Option<NativeSpace>,
}
impl Placement {
    fn space(&self, source: Option<NativeSpace>) -> Option<NativeSpace> {
        match (self.native, source) {
            (Some(outer), Some(inner)) => Some(inner.placed(outer.matrix())),
            (Some(outer), None) => Some(outer),
            (None, Some(inner)) => {
                Some(inner.placed([1.0, 0.0, 0.0, 1.0, self.shift[0], self.shift[1]]))
            }
            (None, None) => None,
        }
    }
}
