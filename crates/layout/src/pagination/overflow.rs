use super::{ActiveBox, DisplayItem, Paginator, Rect, Region};
use ferri_pdf_style::{Overflow, Style};

impl Paginator<'_, '_, '_> {
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

    pub(super) fn propagate_overflow(&mut self, active: &ActiveBox<'_>, style: &Style) {
        if !self
            .boxes
            .last()
            .is_some_and(|parent| parent.overflow.is_some())
            && self.root_overflow.is_none()
        {
            return;
        }
        for &(page, slot) in &active.fragments {
            let DisplayItem::Box(paint) = &self.display_lists[page][slot].item else {
                continue;
            };
            let area = box_bounds(active, style, paint, page);
            if area.width > 0.0 && area.height > 0.0 {
                if let Some(parent) = self.boxes.last()
                    && let Some((_, parent_slot)) = parent
                        .fragments
                        .iter()
                        .find(|(fragment, _)| *fragment == page)
                    && let DisplayItem::Box(parent_paint) =
                        &self.display_lists[page][*parent_slot].item
                {
                    let padding = parent_paint.inner_clip([0.0; 4], [true; 2]).rect;
                    let rtl = parent
                        .geometry
                        .as_ref()
                        .is_some_and(|geometry| geometry.source.style().rtl);
                    let before_inline_start = if rtl {
                        area.x >= padding.x + padding.width
                    } else {
                        area.x + area.width <= padding.x
                    };
                    if before_inline_start || area.y + area.height <= padding.y {
                        continue;
                    }
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
    }
}

pub(super) fn transformed_bounds(area: Rect, matrix: [f32; 6]) -> Rect {
    crate::navigation::page_polygon(area, &[Region::Transform(matrix)], false)
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
        let padding = paint.inner_clip([0.0; 4], [true; 2]).rect;
        let clipped = descendants.intersection(padding);
        let mut descendants = *descendants;
        if style.overflow[0] != Overflow::Visible {
            descendants.x = clipped.x;
            descendants.width = clipped.width;
        }
        if style.overflow[1] != Overflow::Visible {
            descendants.y = clipped.y;
            descendants.height = clipped.height;
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
    if let Some(matrix) = style.transform_matrix(border) {
        area = transformed_bounds(area, matrix);
    }
    area
}
