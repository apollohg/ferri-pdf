//! Coordinate frames between a flow's horizontal layout space and physical page space.
use crate::graphics::{BoxAreas, BoxPaint, Clip, DisplayItem, Rect};
use crate::layout::measure::Failure;
use crate::style::logical::{self, Flow, WritingMode};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(super) struct Frame {
    pub(super) matrix: [f32; 6],
    from: Flow,
    to: Flow,
}
impl Frame {
    pub(super) fn physical(flow: Flow, size: [f32; 2]) -> Self {
        let [width, height] = size;
        let matrix = match flow.writing_mode {
            WritingMode::HorizontalTb => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            WritingMode::VerticalRl | WritingMode::SidewaysRl => [0.0, 1.0, -1.0, 0.0, width, 0.0],
            WritingMode::VerticalLr => [0.0, 1.0, 1.0, 0.0, 0.0, 0.0],
            WritingMode::SidewaysLr => [0.0, -1.0, 1.0, 0.0, 0.0, height],
        };
        Self {
            matrix,
            from: flow.horizontal(),
            to: flow,
        }
    }
    pub(super) fn for_style(style: &crate::style::Style) -> Self {
        Self::physical(
            Flow {
                writing_mode: style.layout_writing_mode,
                rtl: style.rtl,
            },
            [0.0; 2],
        )
    }
    pub(super) fn boxes(self, areas: &BoxAreas<Rect>) -> BoxAreas<Rect> {
        BoxAreas {
            border: self.rect(areas.border),
            padding: self.rect(areas.padding),
            content: self.rect(areas.content),
        }
    }
    pub(super) fn inverse(self) -> Self {
        let [a, b, c, d, e, f] = self.matrix;
        let determinant = a * d - b * c;
        Self {
            matrix: [
                d / determinant,
                -b / determinant,
                -c / determinant,
                a / determinant,
                (c * f - d * e) / determinant,
                (b * e - a * f) / determinant,
            ],
            from: self.to,
            to: self.from,
        }
    }
    pub(super) fn point(self, point: [f32; 2]) -> [f32; 2] {
        map_point(self.matrix, point)
    }
    pub(super) fn rect(self, rect: Rect) -> Rect {
        map_rect(self.matrix, rect)
    }
    pub(super) fn clip(self, clip: Clip) -> Clip {
        map_clip(self.matrix, clip)
    }
    fn areas(self, areas: &BoxAreas<Clip>) -> BoxAreas<Clip> {
        BoxAreas {
            border: self.clip(areas.border.clone()),
            padding: self.clip(areas.padding.clone()),
            content: self.clip(areas.content.clone()),
        }
    }
    pub(super) fn paint(self, paint: &mut BoxPaint) {
        if !paint.first_fragment {
            paint.legend = None;
        }
        if let Some(cut) = &mut paint.legend {
            let sides = self.to.sides(
                self.from,
                &[logical::TOP, logical::RIGHT, logical::BOTTOM, logical::LEFT],
            );
            let source_side = cut.side;
            cut.side = sides
                .iter()
                .position(|side| *side == source_side)
                .expect("a physical edge maps to one edge");
            let points = cut
                .gap
                .map(|offset| match source_side {
                    logical::TOP => [paint.rect.x + offset, paint.rect.y],
                    logical::RIGHT => [paint.rect.x + paint.rect.width, paint.rect.y + offset],
                    logical::BOTTOM => [paint.rect.x + offset, paint.rect.y + paint.rect.height],
                    logical::LEFT => [paint.rect.x, paint.rect.y + offset],
                    _ => unreachable!("legend cuts have a physical side"),
                })
                .map(|point| self.point(point));
            let physical = self.rect(paint.rect);
            let (axis, origin) = if matches!(cut.side, logical::TOP | logical::BOTTOM) {
                (0, physical.x)
            } else {
                (1, physical.y)
            };
            cut.gap = [
                points[0][axis].min(points[1][axis]) - origin,
                points[0][axis].max(points[1][axis]) - origin,
            ];
        }
        if let Some(geometry) = &mut paint.border_geometry {
            let geometry = Arc::make_mut(geometry);
            let source = self.rect(Rect {
                x: paint.rect.x + geometry.rect.x,
                y: paint.rect.y + geometry.rect.y,
                ..geometry.rect
            });
            let target = self.rect(paint.rect);
            geometry.rect = Rect {
                x: source.x - target.x,
                y: source.y - target.y,
                ..source
            };
            geometry.radii = map_radii(self.matrix, &geometry.radii);
        }
        paint.rect = self.rect(paint.rect);
        paint.background_box = paint.background_box.map(|rect| self.rect(rect));
        paint.padding = self.to.sides(self.from, &paint.padding);
        paint.margins = self.to.sides(self.from, &paint.margins);
        let sliced = self.to.sides(self.from, &paint.sliced());
        paint.first_fragment = !sliced[logical::TOP];
        paint.last_fragment = !sliced[logical::BOTTOM];
        paint.inline_edges = [!sliced[logical::LEFT], !sliced[logical::RIGHT]];
        paint.decoration = logical::decoration(&paint.decoration, self.from, self.to);
        if let Some(geometry) = &mut paint.background_geometry {
            let geometry = Arc::make_mut(geometry);
            geometry.origins = geometry.origins.as_ref().map(|areas| BoxAreas {
                border: self.rect(areas.border),
                padding: self.rect(areas.padding),
                content: self.rect(areas.content),
            });
            geometry.clips = self.areas(&geometry.clips);
            geometry.viewport = geometry.viewport.map(|rect| self.rect(rect));
        }
    }
    pub(super) fn geometry(self, geometry: &mut crate::style::PageGeometry) {
        geometry.margin = self.to.sides(self.from, &geometry.margin);
        geometry.padding = self.to.sides(self.from, &geometry.padding);
        geometry.border = self.to.sides(self.from, &geometry.border);
        if self.from.writing_mode.vertical() != self.to.writing_mode.vertical() {
            geometry.paper.swap(0, 1);
            geometry.extent.swap(0, 1);
            geometry.area.swap(0, 1);
        }
    }
    pub(super) fn items(self, items: &mut Vec<DisplayItem>) {
        if self.from == self.to {
            return;
        }
        let mut output = Vec::with_capacity(items.len());
        for mut item in std::mem::take(items) {
            match &mut item {
                DisplayItem::Frame(frame) => frame.matrix = compose(self.matrix, frame.matrix),
                DisplayItem::Box(paint) => self.paint(paint),
                DisplayItem::Image(image) => {
                    image.rect = self.rect(image.rect);
                    image.clip = image.clip.take().map(|clip| self.clip(clip));
                }
                DisplayItem::PushClip(clip) => *clip = self.clip(clip.clone()),
                DisplayItem::PushTransform(matrix) => {
                    *matrix = compose(self.matrix, compose(*matrix, self.inverse().matrix));
                }
                DisplayItem::Scene(scene) => {
                    for plane in &mut scene.planes {
                        plane.matrix = crate::style::transform::conjugated(
                            &plane.matrix,
                            self.matrix,
                            self.inverse().matrix,
                        );
                        self.items(&mut plane.items);
                    }
                }
                DisplayItem::PushProjection(projection) => {
                    *projection = crate::paint::projection::compose(
                        crate::paint::projection::from_affine(self.matrix),
                        crate::paint::projection::compose(
                            *projection,
                            crate::paint::projection::from_affine(self.inverse().matrix),
                        ),
                    );
                }
                DisplayItem::PushFilter(filter) => {
                    filter.bounds = filter.bounds.map(|rect| self.rect(rect));
                }
                DisplayItem::PushMask(mask) => {
                    for fragment in &mut mask.fragments {
                        self.paint(fragment);
                    }
                }
                DisplayItem::Backdrop(backdrop) => {
                    backdrop.border = self.clip(backdrop.border.clone());
                }
                DisplayItem::Text { .. }
                | DisplayItem::Decoration(_)
                | DisplayItem::TextShadow(_) => {
                    if let DisplayItem::TextShadow(shadow) = &mut item {
                        let [a, b, c, d, _, _] = self.inverse().matrix;
                        let [x, y] = shadow.offset;
                        shadow.offset = [a * x + c * y, b * x + d * y];
                    }
                    output.extend([
                        DisplayItem::PushTransform(self.matrix),
                        item,
                        DisplayItem::Pop,
                    ]);
                    continue;
                }
                DisplayItem::PushOpacity(_)
                | DisplayItem::PushBlend(_)
                | DisplayItem::PushBackdropRoot
                | DisplayItem::Pop => {}
            }
            output.push(item);
        }
        *items = output;
    }
}

pub(super) fn map_point([a, b, c, d, e, f]: [f32; 6], [x, y]: [f32; 2]) -> [f32; 2] {
    [a * x + c * y + e, b * x + d * y + f]
}
pub(super) fn map_rect(matrix: [f32; 6], rect: Rect) -> Rect {
    let a = map_point(matrix, [rect.x, rect.y]);
    let b = map_point(matrix, [rect.x + rect.width, rect.y + rect.height]);
    Rect {
        x: a[0].min(b[0]),
        y: a[1].min(b[1]),
        width: (b[0] - a[0]).abs(),
        height: (b[1] - a[1]).abs(),
    }
}
pub(super) fn map_clip(matrix: [f32; 6], mut clip: Clip) -> Clip {
    let [a, _, _, d, _, _] = matrix;
    let swapped = a == 0.0 && d == 0.0;
    clip.radii = map_radii(matrix, &clip.radii);
    if let Some(outline) = &mut clip.outline {
        for contour in Arc::make_mut(outline) {
            *contour = contour.map(|point| map_point(matrix, point));
        }
    }
    if swapped {
        clip.axes.swap(0, 1);
    }
    clip.rect = map_rect(matrix, clip.rect);
    clip.reference_box = clip.reference_box.map(|rect| map_rect(matrix, rect));
    clip
}

pub(super) fn map_radii<T: Clone>(matrix: [f32; 6], radii: &[[T; 2]; 4]) -> [[T; 2]; 4] {
    let [a, b, c, d, _, _] = matrix;
    let swapped = a == 0.0 && d == 0.0;
    let center = [a + c, b + d].map(|value| value / 2.0);
    let mut corners = radii.clone();
    for (source, [x, y]) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        .into_iter()
        .enumerate()
    {
        let target = match (a * x + c * y > center[0], b * x + d * y > center[1]) {
            (false, false) => 0,
            (true, false) => 1,
            (true, true) => 2,
            (false, true) => 3,
        };
        let mut radius = radii[source].clone();
        if swapped {
            radius.swap(0, 1);
        }
        corners[target] = radius;
    }
    corners
}

pub(super) fn compose([a, b, c, d, e, f]: [f32; 6], [g, h, i, j, k, l]: [f32; 6]) -> [f32; 6] {
    [
        a * g + c * h,
        b * g + d * h,
        a * i + c * j,
        b * i + d * j,
        a * k + c * l + e,
        b * k + d * l + f,
    ]
}

pub(super) fn native_space(
    space: super::measure::FlowSpace,
    block: super::measure::BlockSource<'_>,
    geometry: &super::geometry::HorizontalBox,
    containing: Option<f32>,
    height: Option<f32>,
) -> super::measure::FlowSpace {
    let inner = block.flow().expect("a flow frame has content");
    let mut space = space.enter(block, geometry, containing, height);
    if block.layout_flow().writing_mode.vertical() != inner.style().used_writing_mode().vertical() {
        let definite = super::geometry::VerticalBox::new(block.style(), geometry, containing)
            .with_used_height(height)
            .definite();
        space.indefinite_inline = containing.is_none()
            && definite.is_none()
            && !inner.style().display.is_block_container();
    }
    space
}

pub(super) fn measure<'a>(
    measurer: &mut super::measure::Measurer<'a>,
    block: super::measure::BlockSource<'a>,
    geometry: &super::geometry::HorizontalBox,
    containing_height: Option<f32>,
    used_height: Option<f32>,
) -> Result<super::measure::MeasuredBlock<'a>, Failure> {
    use super::{
        geometry::{self, VerticalBox},
        margin::MarginStrut,
        measure::{FlowItem, MeasuredBlock},
    };
    let key =
        super::measure::FrameKey::new(measurer, block, geometry, containing_height, used_height);
    let inner = block.flow().expect("a flow frame has content");
    let style = block.style();
    // CSS Flexbox 1 §9.8, CSS Box Alignment 3 §6.1: a flexed or stretched item's width is
    // definite, so an orthogonal flow takes it as its used block size like a fixed `width`.
    let container_sized = measurer.sized_by_container(block.block);
    let used_width = container_sized || matches!(style.width.basis(), crate::style::Size::Value(_));
    let table_edges = if block.collapsed_table().is_some() {
        Some(
            measurer
                .table_data
                .get(inner)?
                .table_edges(inner.style(), geometry.containing_width),
        )
    } else {
        None
    };
    let native_block_edges = table_edges.map_or(0.0, |edges| {
        edges.padding[0] + edges.padding[2] + edges.border[0] + edges.border[2]
    });
    let native_inline_edges = table_edges.map_or(0.0, |edges| edges.horizontal());
    let perpendicular =
        block.layout_flow().writing_mode.vertical() != inner.style().used_writing_mode().vertical();
    let top = geometry.padding[logical::TOP] + geometry.border[logical::TOP];
    let bottom = geometry.padding[logical::BOTTOM] + geometry.border[logical::BOTTOM];
    let mut vertical =
        VerticalBox::new(style, geometry, containing_height).with_used_height(used_height);
    let mut inner_width = if perpendicular {
        vertical.definite().unwrap_or_else(|| {
            let fallback = measurer
                .space
                .fallback(Flow::used(inner.style()), containing_height);
            (fallback
                - top
                - bottom
                - geometry.margin[logical::TOP].unwrap_or(0.0)
                - geometry.margin[logical::BOTTOM].unwrap_or(0.0))
            .max(0.0)
        })
    } else {
        geometry.width
    };
    let available_inline = inner_width;
    let auto_inline = *style.height.basis() == crate::style::Size::Auto
        || (matches!(style.height.basis(), crate::style::Size::Value(_))
            && style.height.resolve_definite(containing_height).is_none());
    let automatic = perpendicular
        && vertical.definite().is_none()
        && used_height.is_none()
        && auto_inline
        && inner.style().display.is_block_container()
        && style.ruby.position != crate::style::RubyPosition::InterCharacter;
    let count = super::multicol::specified_count(style).unwrap_or(1);
    let intrinsic_gap = style.flex.gap[1].resolve_normal(0.0, style.size);
    let mut intrinsic_sizes = None;
    if perpendicular
        && style.ruby.position == crate::style::RubyPosition::InterCharacter
        && auto_inline
    {
        let sizes = super::intrinsic::content(inner, measurer, inner_width)?;
        inner_width = sizes.max.max(containing_height.unwrap_or(0.0));
    } else if perpendicular
        && used_height.is_none()
        && ((auto_inline && vertical.definite().is_none())
            || style.display == crate::style::Display::TableCell
            || [&style.height, &style.min_height, &style.max_height]
                .iter()
                .any(|size| size.intrinsic().is_some()))
    {
        let sizes = if automatic {
            super::intrinsic::flow(inner, measurer, inner_width)?
        } else {
            super::intrinsic::content(inner, measurer, inner_width)?
        };
        intrinsic_sizes = Some(sizes);
        let available = inner_width;
        inner_width = if automatic {
            super::multicol::orthogonal_width(style, sizes, available)
        } else if inner.style().display.is_block_container()
            || (inner.style().display.is_table() && containing_height.is_some())
        {
            sizes.max.min(available).max(sizes.min)
        } else if containing_height.is_none() {
            sizes.max
        } else {
            available
        };
        let constraints = geometry::SizeConstraints::new(
            [&style.height, &style.min_height, &style.max_height],
            containing_height,
            style.box_sizing,
            top + bottom,
        )
        .with_intrinsic(
            [&style.height, &style.min_height, &style.max_height],
            sizes,
            Some(available),
        );
        let preferred = constraints.specified.map_or_else(
            || constraints.preferred(inner_width),
            |specified| {
                if style.display == crate::style::Display::TableCell {
                    inner_width.max(specified)
                } else {
                    specified
                }
            },
        );
        inner_width = constraints.constrain(preferred);
        if style.display != crate::style::Display::TableCell {
            vertical = vertical.with_constraints(constraints);
        }
    }
    let column_width = if automatic {
        style.multicol.width.unwrap_or(inner_width)
    } else {
        inner_width
    };
    let mut automatic_columns = automatic.then(|| {
        let widths = intrinsic_sizes.expect("automatic columns have intrinsic widths");
        let limits = geometry::SizeConstraints::new(
            [&style.height, &style.min_height, &style.max_height],
            containing_height,
            style.box_sizing,
            top + bottom,
        )
        .with_intrinsic(
            [&style.height, &style.min_height, &style.max_height],
            widths,
            Some(available_inline),
        );
        let mut columns = super::multicol::OrthogonalColumns {
            source: std::ptr::from_ref(inner),
            principal: true,
            block_auto: !used_width && matches!(style.width.basis(), crate::style::Size::Auto),
            content_max: widths.max,
            minimum: limits.minimum,
            maximum: limits.maximum,
            geometry: super::multicol::ColumnGeometry {
                count,
                width: column_width,
                inline_size: inner_width,
                gap: intrinsic_gap,
                x: 0.0,
                rtl: inner.style().used_rtl(),
            },
            length: (used_width
                || (matches!(style.width.basis(), crate::style::Size::Auto)
                    && (style.multicol.count.is_none() || style.multicol.width.is_none())))
            .then_some(geometry.width),
        };
        columns.geometry.inline_size = columns.constrain(
            column_width * count as f32 + intrinsic_gap * count.saturating_sub(1) as f32,
        );
        columns.geometry.gap =
            style.flex.gap[1].resolve_normal(columns.geometry.inline_size, style.size);
        inner_width = columns.geometry.inline_size;
        columns
    });
    let mut measured = loop {
        measurer.work.spend(1)?;
        let inner_geometry = if let Some(edges) = table_edges {
            geometry::horizontal_with_edges(
                inner.style(),
                0.0,
                inner_width,
                inner.style().used_rtl(),
                Some(inner_width),
                edges,
            )
        } else {
            geometry::horizontal(
                inner.style(),
                0.0,
                inner_width,
                inner.style().used_rtl(),
                Some(inner_width),
            )
        };
        let saved = (
            measurer.unbroken,
            measurer.column_flow,
            measurer.page_height,
            measurer.orthogonal_columns,
            measurer.space,
            measurer.native_sizing.clone(),
        );
        measurer.unbroken = true;
        measurer.column_flow = None;
        measurer.space = key.native_space();
        measurer.page_height = if perpendicular {
            geometry.width
        } else {
            containing_height.unwrap_or(measurer.page_height)
        };
        measurer.orthogonal_columns = automatic_columns;
        measurer.native_sizing = (!perpendicular).then(|| {
            std::rc::Rc::new(super::measure::NativeSizing {
                source: std::ptr::from_ref(inner),
                vertical: vertical.clone().native_root(),
                scroll_container: style.is_scroll_container(),
            })
        });
        let containing = if perpendicular {
            Some(geometry.width)
        } else {
            vertical.definite()
        };
        let measure = |measurer: &mut super::measure::Measurer<'a>, used| {
            measurer.measure_format(inner, inner_geometry.clone(), true, containing, used)
        };
        let measured = if perpendicular {
            let used = used_width.then_some(geometry.width);
            if container_sized && inner.style().display.is_table() {
                super::tables::measure_wrapper(measurer, used, measure)
            } else {
                measure(measurer, used)
            }
        } else {
            measure(measurer, vertical.definite())
        };
        (
            measurer.unbroken,
            measurer.column_flow,
            measurer.page_height,
            measurer.orthogonal_columns,
            measurer.space,
            measurer.native_sizing,
        ) = saved;
        let measured = measured?;
        if let Some(columns) = automatic_columns {
            let content = measured
                .items
                .iter()
                .find_map(|item| match item {
                    FlowItem::Columns(columns) => Some(columns),
                    _ => None,
                })
                .expect("an automatic column flow measures columns");
            if let Some(next) = columns.adjusted(content, inner.style()) {
                inner_width = next.geometry.inline_size;
                automatic_columns = Some(next);
                continue;
            }
        }
        break measured;
    };
    if table_edges.is_some() {
        let principal = super::tables::principal(&mut measured);
        principal.navigation = block.navigation();
        principal.retain_fragment();
        if let Some(paint) = &mut principal.paint {
            paint.decoration = super::tables::collapsed_decoration(
                std::mem::take(&mut paint.decoration),
                principal.geometry.border,
            );
        }
    }
    measured.frame_key = Some(std::rc::Rc::new(key));
    let inner_height = (measured.height - native_block_edges).max(0.0);
    let geometry = if table_edges.is_some() {
        let content_width = if perpendicular {
            inner_height
        } else {
            (measured.geometry.border_box(measured.height).width - native_inline_edges).max(0.0)
        };
        geometry::horizontal_with_edges(
            style,
            geometry.containing_x,
            geometry.containing_width,
            geometry.containing_rtl,
            Some(content_width),
            geometry::Edges {
                padding: geometry.padding,
                border: geometry.border,
            },
        )
    } else if perpendicular && !used_width {
        let edges = geometry::Edges {
            padding: geometry.padding,
            border: geometry.border,
        };
        let mut constraints = geometry::SizeConstraints::new(
            [&style.width, &style.min_width, &style.max_width],
            Some(geometry.containing_width),
            style.box_sizing,
            edges.horizontal(),
        )
        .with_intrinsic(
            [&style.width, &style.min_width, &style.max_width],
            crate::text::IntrinsicWidths {
                min: inner_height,
                max: inner_height,
            },
            Some(geometry.containing_width),
        );
        super::ratio::transfer_to_width(style, &mut constraints, containing_height, &edges);
        let preferred =
            constraints.preferred(if *style.width.basis() == crate::style::Size::Auto {
                super::ratio::PreferredRatio::of_box(style, &edges)
                    .zip(vertical.definite())
                    .map_or(inner_height, |(ratio, height)| {
                        let width = ratio.width(height);
                        if super::ratio::content_minimum(style, &style.min_width) {
                            width.max(constraints.automatic_minimum(inner_height))
                        } else {
                            width
                        }
                    })
            } else {
                inner_height
            });
        geometry::horizontal_with_edges(
            style,
            geometry.containing_x,
            geometry.containing_width,
            geometry.containing_rtl,
            Some(constraints.constrain(preferred)),
            edges,
        )
    } else {
        geometry.clone()
    };
    let native_height = geometry.width + native_block_edges;
    if perpendicular && table_edges.is_none() && measured.height != native_height {
        measured.height = native_height;
        measured.overflow_height = measured.overflow_height.max(native_height);
        measured.constrained = true;
        if let Some(sizing) = &mut measured.sizing {
            sizing.used_height = Some(geometry.width);
        }
    }
    let content_height = if perpendicular {
        (measured.geometry.border_box(measured.height).width - native_inline_edges).max(0.0)
    } else {
        (measured.height - native_block_edges).max(0.0)
    };
    let natural_height = top + content_height + bottom;
    let height = if table_edges.is_some() {
        natural_height
    } else {
        vertical.used_height(natural_height)
    };
    let used_content_height = (height - top - bottom).max(0.0);
    if !perpendicular && table_edges.is_none() {
        let native_height = used_content_height + native_block_edges;
        measured.constrained |= measured.height != native_height;
        measured.height = native_height;
        measured.overflow_height = measured.overflow_height.max(native_height);
        if let Some(sizing) = &mut measured.sizing {
            sizing.used_height = Some(used_content_height);
        }
    }
    let area = if table_edges.is_some() {
        geometry.border_box(height)
    } else {
        Rect {
            x: geometry.x,
            y: top,
            width: geometry.width,
            height: used_content_height,
        }
    };
    let mut physical_size = [area.width, area.height];
    if block.layout_flow().writing_mode.vertical() {
        physical_size.swap(0, 1);
    }
    {
        let containing_height = if style.display == crate::style::Display::TableCell {
            vertical.definite().unwrap_or(used_content_height)
        } else {
            used_content_height
        };
        let own = Flow::used(measured.style);
        let size = physical_size;
        let padding = Rect {
            x: -geometry.padding[logical::LEFT],
            y: -geometry.padding[logical::TOP],
            width: geometry.width
                + geometry.padding[logical::LEFT]
                + geometry.padding[logical::RIGHT],
            height: containing_height
                + geometry.padding[logical::TOP]
                + geometry.padding[logical::BOTTOM],
        };
        let matrix = compose(
            Frame::physical(own, size).inverse().matrix,
            Frame::physical(block.layout_flow(), size).matrix,
        );
        measured.frame_owner = Some(Box::new(if table_edges.is_some() {
            let rect = measured.geometry.border_box(measured.height);
            let mut paint = measured.paint.clone().unwrap_or_else(|| {
                BoxPaint::whole(rect, crate::graphics::BoxDecoration::default())
            });
            paint.rect.height = measured.height;
            let containing =
                crate::paint::outset(rect, measured.geometry.border.map(|width| -width));
            super::measure::FrameOwner {
                source: block,
                paint,
                containing,
                padding: measured.geometry.padding,
            }
        } else {
            super::measure::FrameOwner {
                source: block,
                paint: owner_paint(block, &geometry, padding, own, size),
                containing: map_rect(matrix, padding),
                padding: own.horizontal().sides(
                    own,
                    &block
                        .layout_flow()
                        .sides(block.layout_flow().horizontal(), &geometry.padding),
                ),
            }
        }));
    }
    let baselines = if perpendicular {
        [None; 2]
    } else {
        let own = Flow::used(measured.style);
        let size = physical_size;
        let matrix = compose(
            Frame::physical(block.layout_flow(), size).inverse().matrix,
            Frame::physical(own, size).matrix,
        );
        [measured.first_baseline(), measured.last_baseline()]
            .map(|baseline| baseline.map(|y| map_point(matrix, [0.0, y])[1] + area.y))
    };
    let edges = measurer.fragmentainer.edges(style);
    Ok(MeasuredBlock {
        frame_key: None,
        frame_owner: None,
        math: None,
        grid: None,
        clearance_before: None,
        clearance_target: measurer
            .clearance_targets
            .get(&std::ptr::from_ref(block.block))
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
        before: MarginStrut::edge(block.block, logical::TOP, geometry.containing_width),
        after: MarginStrut::edge(block.block, logical::BOTTOM, geometry.containing_width),
        through: false,
        height,
        sizing: None,
        constrained: false,
        independent: true,
        overflow_height: height,
        first_fragment: height,
        has_forced: edges.iter().any(|edge| edge.is_forced()),
        break_before: edges[0],
        break_after: edges[1],
        items: vec![FlowItem::Frame {
            inner: std::rc::Rc::new(measured),
            area,
            height,
            baselines,
        }],
        paint: measurer.box_paint(block, &geometry).map(|mut paint| {
            if table_edges.is_some() {
                paint.decoration = super::tables::collapsed_decoration(
                    crate::graphics::BoxDecoration::default(),
                    geometry.border,
                );
            }
            paint
        }),
        navigation: if table_edges.is_some() {
            Default::default()
        } else {
            block.navigation()
        },
        end_space: 0.0,
        start_space: 0.0,
        repeat_space: 0.0,
        marker: None,
        table: None,
        continues: false,
        spanner_fragment: None,
        line_fragments: Vec::new(),
    })
}

fn owner_paint(
    source: super::measure::BlockSource<'_>,
    geometry: &super::geometry::HorizontalBox,
    padding: Rect,
    own: Flow,
    size: [f32; 2],
) -> BoxPaint {
    let mut paint = BoxPaint {
        padding: geometry.padding,
        ..BoxPaint::whole(
            crate::paint::outset(padding, geometry.border),
            source.style().decoration.with_image_type(),
        )
    };
    Frame::physical(source.layout_flow(), size).paint(&mut paint);
    Frame::physical(own, size).inverse().paint(&mut paint);
    paint
}

pub(super) fn owner_decoration(
    block: &mut super::measure::MeasuredBlock<'_>,
    decoration: &crate::graphics::BoxDecoration,
) {
    let parent = block.source.layout_flow();
    for item in &mut block.items {
        if let super::measure::FlowItem::Frame { inner, .. } = item {
            let own = Flow::used(inner.style);
            let inner = std::rc::Rc::make_mut(inner);
            if let Some(owner) = inner.frame_owner.as_mut() {
                let physical = logical::decoration(decoration, parent.horizontal(), parent);
                owner.paint.decoration = logical::decoration(&physical, own, own.horizontal());
            }
        }
    }
}

pub(super) fn align_cell_frame(
    block: &mut super::measure::MeasuredBlock<'_>,
    offset: f32,
    assigned_height: f32,
) {
    if block.source.flow().is_none() {
        return;
    }
    let parent = block.source.layout_flow();
    let geometry = &block.geometry;
    let padding = Rect {
        x: -geometry.padding[logical::LEFT],
        y: -geometry.padding[logical::TOP] - offset,
        width: geometry.width + geometry.padding[logical::LEFT] + geometry.padding[logical::RIGHT],
        height: (assigned_height
            - geometry.border[logical::TOP]
            - geometry.border[logical::BOTTOM])
            .max(0.0),
    };
    for item in &mut block.items {
        if let super::measure::FlowItem::Frame { inner, area, .. } = item {
            if inner.frame_owner.is_none() {
                continue;
            }
            let own = Flow::used(inner.style);
            let mut size = [area.width, area.height];
            if parent.writing_mode.vertical() {
                size.swap(0, 1);
            }
            let matrix = compose(
                Frame::physical(own, size).inverse().matrix,
                Frame::physical(parent, size).matrix,
            );
            let containing = map_rect(matrix, padding);
            if inner
                .frame_owner
                .as_ref()
                .is_some_and(|owner| owner.containing == containing)
            {
                continue;
            }
            let inner = std::rc::Rc::make_mut(inner);
            let owner = inner
                .frame_owner
                .as_mut()
                .expect("a native cell owner was retained");
            owner.containing = containing;
            owner.paint.rect = crate::paint::outset(
                containing,
                owner
                    .paint
                    .decoration
                    .borders
                    .map(crate::paint::Border::used_width),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_jobs_include_atomic_annotations_table_groups_and_column_spanners() {
        use crate::document::{BlockContent, StyledDocument};
        use crate::layout::measure::{Measurer, frame_dependencies};
        use crate::resources::{FileResolver, ResourceBase};
        let html = "<!doctype html><style>@page{size:600pt;margin:20pt}body{margin:0;font:12pt/1 serif}.v{writing-mode:vertical-rl}.flex{display:flex}.grid{display:grid}table{border-spacing:0}td{padding:0}.columns{columns:2;height:60pt}p{margin:0}.all{column-span:all}ruby{ruby-position:inter-character}</style>            <p><span class=v>A</span></p>            <div class=flex><div><span class=v>B</span></div></div>            <div class=grid><div><span class=v>C</span></div></div>            <table><thead><tr><td><span class=v>D</span></td></tr></thead><tbody><tr><td><span class=v>E</span></td></tr></tbody><tfoot><tr><td><span class=v>F</span></td></tr></tfoot></table>            <div class=columns><p><span class=v>G</span></p><p class=all><span class=v>H</span></p><p><span class=v>I</span></p></div>            <p><ruby>J<rt>K</rt></ruby></p>";
        let resolver = FileResolver::default();
        let document =
            StyledDocument::parse(html, &ResourceBase::default(), &[], &resolver).unwrap();
        let text = crate::text::TextContext::new(&document.styles().fonts, &resolver);
        let area = document.page_box().geometry.content_box();
        let mut measurer = Measurer::new(
            &text,
            document.root(),
            [area.width, area.height],
            crate::layout::work::Budget::new(crate::layout::MAX_LAYOUT_WORK),
        );
        let root = measurer
            .block(
                document.root(),
                area.x,
                area.width,
                false,
                true,
                Some(area.height),
            )
            .unwrap();
        let labels: Vec<_> = frame_dependencies(&root)
            .into_iter()
            .map(|frame| {
                let mut pending = vec![frame.source];
                let mut label = String::new();
                while let Some(block) = pending.pop() {
                    for content in block.children() {
                        match content {
                            BlockContent::Inline(paragraph) => label.push_str(paragraph.text()),
                            BlockContent::Block(child) => pending.push(child),
                            BlockContent::Positioned(_) | BlockContent::Events(_) => {}
                        }
                    }
                }
                label
            })
            .collect();
        assert_eq!(
            labels,
            ["A", "B", "C", "D", "E", "F", "G", "H", "I", "K"],
            "each independently painted inline, cell, spanner and ruby annotation contributes one native job; repeated table bands are deduplicated"
        );
    }

    #[test]
    fn alternating_frame_measurements_complete_on_the_default_test_stack() {
        use crate::document::StyledDocument;
        use crate::layout::measure::{FlowItem, Measurer};
        use crate::resources::{FileResolver, ResourceBase};
        const DEPTH: usize = 254;
        let body: String = (0..DEPTH)
            .map(|index| format!("<section class={}>", if index % 2 == 0 { "v" } else { "h" }))
            .collect();
        let html = format!(
            "<!doctype html><style>@page{{size:240pt;margin:20pt}}body{{margin:0}}section{{width:40pt;height:40pt}}.v{{writing-mode:vertical-rl}}.h{{writing-mode:horizontal-tb}}</style>{body}A{}",
            "</section>".repeat(DEPTH)
        );
        let resolver = FileResolver::default();
        let document =
            StyledDocument::parse(&html, &ResourceBase::default(), &[], &resolver).unwrap();
        let text = crate::text::TextContext::new(&document.styles().fonts, &resolver);
        let area = document.page_box().geometry.content_box();
        let mut measurer = Measurer::new(
            &text,
            document.root(),
            [area.width, area.height],
            crate::layout::work::Budget::new(crate::layout::MAX_LAYOUT_WORK),
        );
        let root = measurer
            .block(
                document.root(),
                area.x,
                area.width,
                false,
                true,
                Some(area.height),
            )
            .unwrap();
        let mut pending = vec![&root];
        let mut frames = 0;
        let mut paragraphs = Vec::new();
        while let Some(block) = pending.pop() {
            for item in &block.items {
                match item {
                    FlowItem::Block(child) => pending.push(child),
                    FlowItem::Frame { inner, .. } => {
                        frames += 1;
                        pending.push(inner);
                    }
                    FlowItem::Paragraph(paragraph) => paragraphs.push(paragraph.paragraph.text()),
                    _ => {}
                }
            }
        }
        assert_eq!(
            frames, DEPTH,
            "every alternating flow boundary must be measured once in the retained tree"
        );
        assert_eq!(
            paragraphs,
            ["A"],
            "the leaf text survives all suspended frame dependencies"
        );
    }

    #[test]
    fn native_writing_keeps_the_original_shaped_outline_reference() {
        use crate::paint::{
            BorderStyle, BoxDecoration, Color, CornerShape, LengthPercentage, Outline, OutlineStyle,
        };
        let paint = BoxPaint::whole(
            Rect {
                x: 10.0,
                y: 20.0,
                width: 60.0,
                height: 80.0,
            },
            BoxDecoration {
                radii: Arc::new(std::array::from_fn(|corner| {
                    [
                        LengthPercentage::Length(10.0 + corner as f32),
                        LengthPercentage::Length(20.0 + corner as f32),
                    ]
                })),
                corner_shapes: Arc::new([
                    CornerShape::BEVEL,
                    CornerShape::NOTCH,
                    CornerShape::ROUND,
                    CornerShape::SQUARE,
                ]),
                outline: Outline {
                    width: 4.0,
                    offset: 6.0,
                    style: OutlineStyle::Line(BorderStyle::Solid),
                    color: Color::BLACK,
                },
                ..Default::default()
            },
        );
        for mode in [
            WritingMode::HorizontalTb,
            WritingMode::VerticalRl,
            WritingMode::VerticalLr,
            WritingMode::SidewaysRl,
            WritingMode::SidewaysLr,
        ] {
            for rtl in [false, true] {
                let frame = Frame::physical(
                    Flow {
                        writing_mode: mode,
                        rtl,
                    },
                    [120.0, 140.0],
                );
                let mut actual = paint.outline().unwrap();
                frame.paint(&mut actual);
                let mut original = paint.clone();
                frame.paint(&mut original);
                let expected = original.outline().unwrap();
                let source = actual
                    .border_geometry
                    .as_ref()
                    .expect("shaped outlines retain their reference");
                let reference = expected.border_geometry.as_ref().unwrap();
                assert_eq!(
                    source.rect, reference.rect,
                    "mode={mode:?},rtl={rtl}: reference position"
                );
                assert_eq!(
                    source.radii, reference.radii,
                    "mode={mode:?},rtl={rtl}: original radii"
                );
                assert_eq!(actual.rect, expected.rect);
                assert_eq!(
                    actual.decoration.corner_shapes,
                    expected.decoration.corner_shapes
                );
            }
        }
    }

    #[test]
    fn native_writing_maps_every_explicit_clip_segment() {
        use crate::graphics::{Contour, Segment};
        let outline = vec![
            Contour {
                start: [10.0, 20.0],
                segments: vec![
                    Segment::Cubic([[12.0, 20.0], [14.0, 23.0], [16.0, 25.0]]),
                    Segment::Line([10.0, 25.0]),
                ],
                closed: true,
            },
            Contour::polygon(&[[30.0, 40.0], [35.0, 40.0], [35.0, 45.0]]),
        ];
        let source = Clip {
            rect: Rect {
                x: 10.0,
                y: 20.0,
                width: 40.0,
                height: 60.0,
            },
            radii: Default::default(),
            outline: Some(Arc::new(outline.clone())),
            axes: [true; 2],
            reference_box: None,
        };
        for mode in [
            WritingMode::VerticalRl,
            WritingMode::SidewaysRl,
            WritingMode::VerticalLr,
            WritingMode::SidewaysLr,
        ] {
            let frame = Frame::physical(
                Flow {
                    writing_mode: mode,
                    rtl: false,
                },
                [100.0, 120.0],
            );
            let mapped = frame.clip(source.clone());
            let expected: Vec<_> = outline
                .iter()
                .map(|contour| contour.map(|point| map_point(frame.matrix, point)))
                .collect();
            assert_eq!(
                mapped.contours().as_ref(),
                expected,
                "{mode:?}: all islands, control points and endpoints must follow the native frame"
            );
            assert_eq!(
                frame.inverse().clip(mapped),
                source,
                "{mode:?}: inverse projection must restore the complete outline"
            );
        }
    }

    #[test]
    fn native_clip_chains_preserve_each_asymmetric_corner_and_fragment_axis() {
        let source = Clip {
            rect: Rect {
                x: 13.0,
                y: 17.0,
                width: 40.0,
                height: 60.0,
            },
            reference_box: Some(Rect {
                x: 10.0,
                y: 15.0,
                width: 50.0,
                height: 70.0,
            }),
            radii: [[1.0, 2.0], [3.0, 4.0], [5.0, 6.0], [7.0, 8.0]],
            outline: None,
            axes: [true, false],
        };
        for (mode, order) in [
            (WritingMode::VerticalRl, [3, 0, 1, 2]),
            (WritingMode::SidewaysRl, [3, 0, 1, 2]),
            (WritingMode::VerticalLr, [0, 3, 2, 1]),
            (WritingMode::SidewaysLr, [1, 2, 3, 0]),
        ] {
            let frame = Frame::physical(
                Flow {
                    writing_mode: mode,
                    rtl: false,
                },
                [100.0, 120.0],
            );
            let mapped = map_clip(frame.matrix, source.clone());
            assert_eq!(
                mapped.radii,
                order.map(|index| [source.radii[index][1], source.radii[index][0]]),
                "{mode:?}: each physical corner keeps its own radii"
            );
            assert_eq!(
                mapped.axes,
                [false, true],
                "{mode:?}: the clipped strip transposes"
            );
            assert_eq!(
                [mapped.rect.width, mapped.rect.height],
                [60.0, 40.0],
                "{mode:?}: fragment dimensions transpose"
            );
            assert_eq!(
                frame.inverse().clip(mapped),
                source,
                "{mode:?}: composing the inverse restores rounded fragment geometry and its reference box"
            );
        }
    }
}
