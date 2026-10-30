use super::{ActiveBox, DisplayItem, Effect, Paginator, Rect, Region};
use crate::layout::LayoutError;
use crate::paint::BoxAreas;
use crate::style::{Overflow, Style};

#[derive(Clone)]
pub(super) struct Pending {
    page: usize,
    area: Rect,
    pub(super) transforms: Vec<usize>,
}

impl<'a> Paginator<'a, '_, 'a> {
    pub(super) fn complete_motion_reference(
        &mut self,
        active: &mut ActiveBox<'_>,
        complete: bool,
    ) -> Result<(), LayoutError> {
        if let Some(node) = active.motion_reference_node {
            self.clips[node].fragments.clone_from(&active.fragments);
            if let Effect::MotionReference(current, _) = &mut self.clips[node].effect {
                *current = complete;
            }
        }
        if complete {
            for pending in std::mem::take(&mut active.motion_overflow) {
                let area = self.overflow_chain(pending.page, pending.area, &pending.transforms)?;
                if area.width > 0.0
                    && area.height > 0.0
                    && !before_start(active, &self.display_lists, pending.page, area)
                    && let Some(bounds) = active.overflow.as_mut().or(self.root_overflow.as_mut())
                {
                    bounds
                        .entry(pending.page)
                        .and_modify(|bounds| *bounds = bounds.union(area))
                        .or_insert(area);
                }
            }
        }
        Ok(())
    }
    pub(super) fn transform_overflow(
        &mut self,
        active: &ActiveBox<'_>,
        style: &Style,
        page: usize,
        area: Rect,
        boxes: BoxAreas<Rect>,
    ) -> Result<Option<Rect>, LayoutError> {
        if let Some(transform) = active.transform_node
            && self.unfinished_motion(transform)
        {
            let mut transforms = vec![transform];
            if let Some(source) = active.geometry.as_ref().map(|geometry| geometry.source)
                && let Some(table) = self.tables.last()
                && let Some(cell) = table.flow.plan.cell_index(source)
            {
                transforms.extend(
                    table
                        .flow
                        .plan
                        .cell_parents(cell)
                        .into_iter()
                        .rev()
                        .flatten()
                        .filter_map(|source| {
                            table.transforms.iter().find_map(|(ancestor, node)| {
                                std::ptr::eq(ancestor.source, source).then_some(*node)
                            })
                        }),
                );
            }
            self.defer_overflow(page, area, transforms);
            return Ok(None);
        }
        let node = active.transform_node.map(|node| &self.clips[node]);
        Ok(Some(
            if let Some(region) = self.transform_on(style, &boxes, node, page)? {
                transformed_bounds(area, region)
            } else {
                area
            },
        ))
    }

    fn unfinished_motion(&self, node: usize) -> bool {
        let Effect::Transform(_, _, Some(reference)) = self.clips[node].effect else {
            return false;
        };
        match self.clips[reference].effect {
            Effect::MotionReference(false, _) => true,
            Effect::ColumnReference => self.clips[reference].areas.is_empty(),
            _ => false,
        }
    }
    pub(super) fn complete_column_overflow(&mut self) -> Result<(), LayoutError> {
        for pending in std::mem::take(&mut self.column_motion_overflow) {
            let area = self.overflow_chain(pending.page, pending.area, &pending.transforms)?;
            if area.width > 0.0 && area.height > 0.0 {
                self.record_overflow_on(pending.page, area);
            }
        }
        Ok(())
    }
    pub(super) fn defer_overflow(
        &mut self,
        page: usize,
        area: Rect,
        transforms: Vec<usize>,
    ) -> bool {
        if !transforms.iter().any(|&node| self.unfinished_motion(node)) {
            return false;
        }
        let column = transforms.iter().any(|&node| matches!(self.clips[node].effect,Effect::Transform(_,_,Some(reference)) if matches!(self.clips[reference].effect,Effect::ColumnReference) && self.clips[reference].areas.is_empty()));
        let pending = Pending {
            page,
            area,
            transforms,
        };
        if column {
            self.column_motion_overflow.push(pending);
        } else {
            self.boxes
                .last_mut()
                .expect("an unfinished block reference belongs to an active containing box")
                .motion_overflow
                .push(pending);
        }
        true
    }
    pub(super) fn overflow_chain(
        &self,
        page: usize,
        mut area: Rect,
        transforms: &[usize],
    ) -> Result<Rect, LayoutError> {
        for &node in transforms {
            let effect = &self.clips[node];
            let Effect::Transform(style, _, _) = effect.effect else {
                unreachable!()
            };
            let boxes = effect
                .fragments
                .iter()
                .find_map(|&(p, slot)| {
                    if p != page {
                        return None;
                    }
                    match &self.display_lists[p][slot].item {
                        DisplayItem::Box(paint) => Some(paint.fragment_areas()),
                        _ => None,
                    }
                })
                .or_else(|| {
                    effect
                        .areas
                        .iter()
                        .find_map(|&(p, rect)| (p == page).then_some(BoxAreas::single(rect)))
                })
                .ok_or(LayoutError::MissingContainingFragment { page })?;
            if let Some(region) = self.transform_on(style, &boxes, Some(effect), page)? {
                area = transformed_bounds(area, region);
            }
        }
        Ok(area)
    }
    pub(super) fn record_overflow(&mut self, rect: Rect) {
        self.record_overflow_on(self.page, rect);
    }

    pub(super) fn record_overflow_on(&mut self, page: usize, rect: Rect) {
        if let Some(bounds) = self
            .boxes
            .last_mut()
            .and_then(|active| active.overflow.as_mut())
            .or(self.root_overflow.as_mut())
        {
            bounds
                .entry(page)
                .and_modify(|area| *area = area.union(rect))
                .or_insert(rect);
        }
    }

    pub(super) fn propagate_overflow(
        &mut self,
        active: &ActiveBox<'_>,
        style: &Style,
    ) -> Result<(), LayoutError> {
        if self
            .boxes
            .last()
            .is_none_or(|parent| parent.overflow.is_none())
            && self.root_overflow.is_none()
        {
            return Ok(());
        }
        for &(page, slot) in &active.fragments {
            let DisplayItem::Box(paint) = &self.display_lists[page][slot].item else {
                continue;
            };
            let area = box_bounds(active, style, paint, page);
            let boxes = paint.fragment_areas();
            let Some(area) = self.transform_overflow(active, style, page, area, boxes)? else {
                continue;
            };
            if area.width > 0.0 && area.height > 0.0 {
                if self
                    .boxes
                    .last()
                    .is_some_and(|parent| before_start(parent, &self.display_lists, page, area))
                {
                    continue;
                }
                self.boxes
                    .last_mut()
                    .and_then(|parent| parent.overflow.as_mut())
                    .or(self.root_overflow.as_mut())
                    .expect("overflow tracking was checked before propagation")
                    .entry(page)
                    .and_modify(|bounds| *bounds = bounds.union(area))
                    .or_insert(area);
            }
        }
        Ok(())
    }
}

pub(super) fn transformed_bounds(area: Rect, region: Region) -> Rect {
    crate::layout::navigation::page_polygon(area, &[region], false)
        .into_iter()
        .map(|[x, y]| Rect {
            x,
            y,
            width: 0.0,
            height: 0.0,
        })
        .fold(area, Rect::union)
}

pub(super) fn box_bounds(
    active: &ActiveBox<'_>,
    style: &Style,
    paint: &super::BoxPaint,
    page: usize,
) -> Rect {
    let border = paint.rect;
    let mut area = border;
    if let Some(descendants) = active
        .overflow
        .as_ref()
        .and_then(|bounds| bounds.get(&page))
    {
        // Scroll containers clip at the padding edge, `overflow: clip` at the overflow clip
        // edge (CSS Overflow 3 §3.2).
        let padding = descendants.intersection(paint.inner_clip([0.0; 4], [true; 2]).rect);
        let edge = descendants.intersection(style.overflow_clip_edge(paint, [true; 2]).rect);
        let mut descendants = *descendants;
        for (axis, overflow) in style.overflow.into_iter().enumerate() {
            let clipped = match overflow {
                Overflow::Visible => continue,
                Overflow::Clip => edge,
                Overflow::Hidden | Overflow::Auto | Overflow::Scroll => padding,
            };
            if axis == 0 {
                descendants.x = clipped.x;
                descendants.width = clipped.width;
            } else {
                descendants.y = clipped.y;
                descendants.height = clipped.height;
            }
        }
        if descendants.width > 0.0 && descendants.height > 0.0 {
            area = area.union(descendants);
        }
    }
    if let Some(geometry) = &active.geometry {
        let margins = style.margin.each_ref().map(|margin| {
            margin
                .resolve(geometry.horizontal.containing_width)
                .unwrap_or(0.0)
        });
        let margin = Rect {
            x: border.x - margins[3],
            y: border.y - margins[0],
            width: (border.width + margins[1] + margins[3]).max(0.0),
            height: (border.height + margins[0] + margins[2]).max(0.0),
        };
        area = area.union(margin);
    }
    area
}

fn before_start(
    parent: &ActiveBox<'_>,
    lists: &[Vec<super::ScopedPaint>],
    page: usize,
    area: Rect,
) -> bool {
    let Some((_, slot)) = parent.fragments.iter().find(|(p, _)| *p == page) else {
        return false;
    };
    let DisplayItem::Box(paint) = &lists[page][*slot].item else {
        return false;
    };
    let padding = paint.inner_clip([0.0; 4], [true; 2]).rect;
    let rtl = parent
        .geometry
        .as_ref()
        .is_some_and(|geometry| geometry.source.style().used_rtl());
    (if rtl {
        area.x >= padding.x + padding.width
    } else {
        area.x + area.width <= padding.x
    }) || area.y + area.height <= padding.y
}
