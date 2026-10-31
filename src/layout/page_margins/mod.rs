mod dimensions;
use crate::document::{BlockBox, CounterSnapshot, PageBox, Paragraph, RunningStringAssignment};
use crate::graphics::{BoxDecoration, BoxPaint, DisplayItem, FlowPaint, ImagePaint, Rect};
use crate::layout::measure::{MeasuredBlock, Measurer};
use crate::layout::multicol::{
    ColumnGeometry, WrapRows,
    slices::{Distribution, Plan, Run, line_breaks, slice_clip, windowed},
};
use crate::layout::pagination::{Group, Region, StandaloneFlow};
use crate::layout::{LayoutError, TextLine};
use crate::style::{BoxSizing, MarginLocation, PageMargin, Size, VerticalAlign};
use crate::text::Line;
use dimensions::{Variable, variable};

const HORIZONTAL: usize = 0;
const VERTICAL: usize = 1;
const START: usize = 0;
const CENTER: usize = 1;
const END: usize = 2;
const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
const GROUPS: [[MarginLocation; 3]; 4] = [
    [
        MarginLocation::TopLeft,
        MarginLocation::TopCenter,
        MarginLocation::TopRight,
    ],
    [
        MarginLocation::BottomLeft,
        MarginLocation::BottomCenter,
        MarginLocation::BottomRight,
    ],
    [
        MarginLocation::LeftTop,
        MarginLocation::LeftMiddle,
        MarginLocation::LeftBottom,
    ],
    [
        MarginLocation::RightTop,
        MarginLocation::RightMiddle,
        MarginLocation::RightBottom,
    ],
];
struct Prepared<'a, 't> {
    margin: &'a PageMargin,
    decoration: &'a BoxDecoration,
    paragraph: Option<Paragraph>,
    /// The sizes and line metrics of the paragraph's `content` images.
    images: (Vec<[f32; 2]>, Vec<crate::text::AtomicMetrics>),
    /// The image a `<content-replacement>` replaces the box with (CSS Content 3 §1).
    replacement: Option<std::sync::Arc<crate::images::Image>>,
    /// The running element `element()` places (CSS GCPM 3 §1.2.2), laid out as the box's
    /// only child.
    element: Option<&'t BlockBox>,
    /// The running element laid out alone, and the block size it takes.
    flow: Option<(StandaloneFlow, f32)>,
    /// The box's block size once fixed or resolved, which its columns fill.
    block_size: Option<f32>,
    /// The column boxes of a multi-column margin box, a block container of its generated
    /// content (CSS Page 3 §5.2), and the content laid out in them.
    columns: Option<(ColumnGeometry, Distribution)>,
    /// Text areas collect the box's text: its own backgrounds or its page box's clip to text.
    text_area: bool,
    padding: [f32; 4],
    borders: [f32; 4],
    margins: [Option<f32>; 4],
    specified: [Option<f32>; 2],
    start: [f32; 2],
    content: [f32; 2],
    lines: Vec<Line>,
}
impl<'t> Prepared<'_, 't> {
    fn inline_axis(&self) -> usize {
        if self.margin.style.used_writing_mode().vertical() {
            VERTICAL
        } else {
            HORIZONTAL
        }
    }
    fn sides(axis: usize) -> [usize; 2] {
        if axis == HORIZONTAL { [3, 1] } else { [0, 2] }
    }
    fn edges(&self, axis: usize) -> f32 {
        Self::sides(axis)
            .iter()
            .map(|&i| self.padding[i] + self.borders[i])
            .sum()
    }
    fn outer_edges(&self, axis: usize) -> f32 {
        self.edges(axis)
            + Self::sides(axis)
                .iter()
                .map(|&i| self.margins[i].unwrap_or(0.0))
                .sum::<f32>()
    }
    fn fixed(&mut self, axis: usize, available: f32) {
        let [start, end] = Self::sides(axis).map(|i| self.margins[i].unwrap_or(0.0));
        self.start[axis] = start;
        // The fixed dimension fills the margin area regardless of width/height declarations.
        self.content[axis] = available - self.edges(axis) - start - end;
        self.resolved(axis);
    }
    /// Records the content size along `axis` as resolved, which fixes the block size.
    fn resolved(&mut self, axis: usize) {
        if axis != self.inline_axis() {
            self.block_size = Some(self.content[axis]);
        }
    }
    fn variable(&mut self, axis: usize, available: f32, outer: f32, position: usize) {
        self.content[axis] = (outer - self.outer_edges(axis)).max(0.0);
        let style = &self.margin.style;
        if axis == HORIZONTAL && *style.width.basis() == Size::Auto {
            // CSS Values 5 §10.2: a `calc-size()` adjusts the automatic width once resolved.
            self.content[axis] = crate::style::SizeConstraints::new(
                [&style.width, &Size::Auto, &Size::Auto],
                Some(available),
                style.box_sizing,
                self.edges(axis),
            )
            .preferred(self.content[axis]);
        }
        self.resolved(axis);
        let offset = match position {
            START => 0.0,
            CENTER => (available - outer) / 2.0,
            END => available - outer,
            _ => unreachable!(),
        };
        self.start[axis] = offset + self.margins[Self::sides(axis)[0]].unwrap_or(0.0);
    }
    fn validate(&self) -> Result<(), LayoutError> {
        let values = self
            .padding
            .iter()
            .chain(self.borders.iter())
            .chain(self.margins.iter().flatten())
            .chain(self.specified.iter().flatten())
            .chain(self.start.iter())
            .chain(self.content.iter());
        if values.into_iter().any(|value| !value.is_finite())
            || [HORIZONTAL, VERTICAL]
                .iter()
                .any(|&axis| !self.outer_edges(axis).is_finite())
        {
            return Err(LayoutError::NonFinitePageMarginGeometry {
                location: self.margin.location,
            });
        }
        Ok(())
    }
    /// Whether the box is a multi-column container: a block container, which a box that an
    /// image replaces is not, whose column count or width is not `auto` (CSS Multi-column
    /// Layout 1 §2).
    fn multicol(&self) -> bool {
        self.margin.style.multicol.is_multicol() && self.replacement.is_none()
    }
    /// How the content fills the columns of `geometry`: those of the box's block size once
    /// that is resolved, else balanced (CSS Multi-column Layout 1 §7.1).
    fn plan(&self, geometry: &ColumnGeometry) -> Plan {
        let multicol = &self.margin.style.multicol;
        Plan {
            count: geometry.count,
            height: multicol.height.or(self.block_size),
            definite: multicol.height.is_some(),
            fill: multicol.fill,
            wrap: WrapRows::of(&self.margin.style, self.block_size),
        }
    }
    /// The content, `extent` tall with column breaks allowed at `breaks`, in the columns of
    /// `geometry`.
    fn distribute(&self, geometry: &ColumnGeometry, breaks: &[f32], extent: f32) -> Distribution {
        Distribution::of(
            &[Run {
                breaks,
                ..Run::whole(extent)
            }],
            self.plan(geometry),
        )
    }
    /// The generated lines in the columns of `geometry`.
    fn line_columns(&self, geometry: &ColumnGeometry) -> Distribution {
        let mut top = 0.0;
        let boxes: Vec<[f32; 2]> = self
            .lines
            .iter()
            .map(|line| {
                top += line.height;
                [top - line.height, top]
            })
            .collect();
        let style = &self.margin.style;
        let breaks = line_breaks(&boxes, [], [style.orphans, style.widows], top);
        self.distribute(geometry, &breaks, top)
    }
    /// Where each line's box starts in the content box, in the box's logical coordinates:
    /// stacked, or in the slice of the column box holding it.
    fn line_offsets(&self) -> Vec<[f32; 2]> {
        let mut top = 0.0;
        let tops = self.lines.iter().map(|line| {
            top += line.height;
            top - line.height
        });
        let Some((geometry, columns)) = &self.columns else {
            return tops.map(|y| [0.0, y]).collect();
        };
        let placed: Vec<_> = columns.placed(*geometry).collect();
        let mut slice = 0;
        tops.map(|y| {
            while slice + 1 < placed.len() && y >= placed[slice].0.bottom {
                slice += 1;
            }
            let (held, [x, top]) = placed[slice];
            [x, top + y - held.top]
        })
        .collect()
    }
    /// The running element laid out alone at `width` from its measurement.
    fn element_flow(
        &self,
        measurer: &mut Measurer<'t>,
        measured: MeasuredBlock<'t>,
        width: f32,
        destinations: usize,
    ) -> Result<(StandaloneFlow, f32), LayoutError> {
        // The margin box is a block formatting context root, so the element's margins stay
        // inside it.
        let extent = if measured.through {
            let mut margins = measured.before.clone();
            margins.merge(measured.after.clone());
            margins.value()
        } else {
            measured.before.value() + measured.height + measured.after.value()
        };
        let flow = crate::layout::pagination::Paginator::standalone(
            measurer,
            measured,
            Rect {
                x: 0.0,
                y: 0.0,
                width,
                height: extent,
            },
            destinations,
            self.text_area,
        )?;
        Ok((flow, extent))
    }
    fn layout(
        &mut self,
        measurer: &mut Measurer<'t>,
        destinations: usize,
    ) -> Result<(), LayoutError> {
        self.validate()?;
        let inline_axis = self.inline_axis();
        let [inline, extent] = [self.content[inline_axis], self.content[1 - inline_axis]];
        let margin = self.margin;
        let style = &margin.style;
        // CSS Overflow 4 §5.3: a margin box, a block container, discards the content after
        // its region break, which follows its columns inside its block size.
        let discards = style.clamp.continuation == crate::style::Continue::Discard;
        let geometry = self
            .multicol()
            .then(|| ColumnGeometry::resolve(style, 0.0, inline));
        let width = geometry.map_or(inline, |geometry| geometry.width);
        self.columns = None;
        if let Some(element) = self.element {
            let rtl = style.used_rtl();
            let orphans_widows = [element.style().orphans, element.style().widows];
            let clipped = |measurer: &mut Measurer<'t>, extent: f32| {
                measurer.discarded_block(element, width, rtl, extent, style.clamp.max_lines)
            };
            let measured = if discards && geometry.is_none() {
                clipped(measurer, extent)?
            } else {
                measurer.block(element, 0.0, width, rtl, false, None)?
            };
            let mut flow = self.element_flow(measurer, measured, width, destinations)?;
            if let Some(geometry) = geometry {
                let mut columns =
                    self.distribute(&geometry, &flow.0.breaks(orphans_widows, flow.1), flow.1);
                if discards {
                    let held = clipped(measurer, columns.held(geometry.count, extent))?;
                    flow = self.element_flow(measurer, held, width, destinations)?;
                    columns =
                        self.distribute(&geometry, &flow.0.breaks(orphans_widows, flow.1), flow.1);
                }
                self.columns = Some((geometry, columns));
            }
            self.flow = Some(flow);
            return Ok(());
        }
        let Some(paragraph) = &self.paragraph else {
            self.lines = Vec::new();
            return Ok(());
        };
        let atomics = &self.images.1;
        self.lines = if discards && geometry.is_none() {
            measurer.discarded_text_lines(paragraph, width, atomics, extent, &style.clamp)?
        } else {
            measurer.text_lines(paragraph, width, atomics)?
        };
        if let Some(geometry) = geometry {
            let mut columns = self.line_columns(&geometry);
            if discards {
                let held = columns.held(geometry.count, extent);
                self.lines =
                    measurer.discarded_text_lines(paragraph, width, atomics, held, &style.clamp)?;
                columns = self.line_columns(&geometry);
            }
            self.columns = Some((geometry, columns));
        }
        Ok(())
    }
    /// The content's min-content and max-content contributions in the inline axis, which a
    /// multi-column box repeats in each column (CSS Multi-column Layout 1 §3.4).
    fn intrinsic(
        &self,
        measurer: &mut Measurer<'t>,
        available: f32,
    ) -> Result<[f32; 2], LayoutError> {
        if self.replacement.is_some() {
            let inline = self.inline_axis();
            let width = self.replaced_extent(inline, self.content[1 - inline]);
            return Ok([width; 2]);
        }
        let widths = self.content_intrinsic(measurer, available)?;
        let widths = if self.multicol() {
            crate::layout::multicol::intrinsic(&self.margin.style, widths)
        } else {
            widths
        };
        Ok([widths.min, widths.max])
    }
    fn content_intrinsic(
        &self,
        measurer: &mut Measurer<'t>,
        available: f32,
    ) -> Result<crate::text::IntrinsicWidths, LayoutError> {
        if let Some(element) = self.element {
            return measurer
                .run(|measurer| crate::layout::intrinsic::measure(element, measurer, available));
        }
        let atomics: Vec<_> = self
            .images
            .0
            .iter()
            .map(|&[width, _]| crate::text::IntrinsicWidths {
                min: width,
                max: width,
            })
            .collect();
        Ok(self
            .paragraph
            .as_ref()
            .map(|p| measurer.intrinsic_widths(p, &atomics, &atomics))
            .transpose()?
            .unwrap_or_default())
    }
    fn block_extent(&self) -> f32 {
        if let Some((_, columns)) = &self.columns {
            return columns.height();
        }
        if let Some((_, extent)) = &self.flow {
            return *extent;
        }
        if self.replacement.is_some() {
            let inline = self.inline_axis();
            return self.replaced_extent(1 - inline, self.content[inline]);
        }
        self.lines.iter().map(|line| line.height).sum()
    }
    /// The replaced content's extent along physical `axis` when the other axis spans `other`:
    /// its natural size, or the size its natural ratio transfers, as for a replaced element
    /// whose other dimension is specified (CSS 2.1 §10.3.2, §10.6.2).
    fn replaced_extent(&self, axis: usize, other: f32) -> f32 {
        let image = self
            .replacement
            .as_ref()
            .expect("only a replaced margin box has replaced content");
        let natural = crate::paint::natural_points(
            image,
            crate::paint::INITIAL_IMAGE_SCALE,
            self.margin.style.effective_zoom,
        );
        let mut size = [
            crate::paint::SizeComponent::Auto,
            crate::paint::SizeComponent::Auto,
        ];
        size[1 - axis] =
            crate::paint::SizeComponent::Value(crate::paint::LengthPercentage::Length(other));
        crate::paint::image_size(
            &crate::paint::BackgroundSize::Explicit(size),
            natural,
            crate::images::DEFAULT_OBJECT_SIZE
                .map(|extent| extent * crate::style::POINTS_PER_PIXEL),
        )[axis]
    }
    /// The decoration, then the content laid out at its vertical alignment, then the outline.
    fn paint(
        mut self,
        origin: [f32; 2],
        index: usize,
        output: &mut MarginOutput<'_>,
        measurer: &mut Measurer<'t>,
    ) -> Result<(), LayoutError> {
        self.validate()?;
        let rect = Rect {
            x: origin[0] + self.start[0],
            y: origin[1] + self.start[1],
            width: self.content[0] + self.edges(0),
            height: self.content[1] + self.edges(1),
        };
        if [
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            rect.x + rect.width,
            rect.y + rect.height,
        ]
        .iter()
        .any(|value| !value.is_finite())
        {
            return Err(LayoutError::NonFinitePageMarginGeometry {
                location: self.margin.location,
            });
        }
        let margin = self.margin;
        let style = &margin.style;
        let paint = BoxPaint {
            padding: self.padding,
            ..BoxPaint::whole(rect, self.decoration.clone())
        };
        let areas = paint.fragment_areas();
        let transform = style
            .plane(
                &areas,
                output.perspective.as_ref(),
                &crate::style::motion::Reference::rect(output.page),
            )
            .map(|plane| Region::projected(style.projected(&plane, &areas)));
        if transform.as_ref().is_some_and(Region::collapses) {
            return Ok(());
        }
        // CSS Page 3 §3.1: every page-margin box establishes a stacking context, which
        // composites as one group.
        let group = Group::of(style);
        let opening: Vec<_> = transform
            .clone()
            .map(Region::push)
            .into_iter()
            .chain(group.pushes(std::slice::from_ref(&paint)))
            .collect();
        output.display.extend(opening.iter().cloned());
        let links = output.links.len();
        // Generated content takes the margin box's style, so its visibility hides the box and
        // all of that content; a running element keeps its own (CSS GCPM 3 §1.2.1).
        let visible = style.visibility.is_visible();
        let outline = paint.outline().filter(|_| visible);
        let replaced = self
            .replacement
            .take()
            .map(|image| self.replaced(image, &paint))
            .filter(|_| visible);
        // CSS Overflow 3 §3.1: the overflow clips the box's content at its padding or overflow
        // clip edge, and the box's outline paints over them.
        let clips: Vec<_> = style
            .clip_edges()
            .map(|edge| style.overflow_clip(&paint, edge))
            .collect();
        // CSS Backgrounds 4 §2.8: `background-clip: text` layers paint within the text of
        // the box's content, which a page box's text holds too.
        let clips_to_text = self.decoration.backgrounds.clips_to_text();
        let text = self.text_area;
        if visible {
            output.display.extend(group.backdrop(&paint));
        }
        let painted = (visible && self.decoration.is_visible()).then(|| {
            output.display.push(DisplayItem::Box(paint));
            output.display.len() - 1
        });
        output
            .display
            .extend(clips.iter().cloned().map(DisplayItem::PushClip));
        let content = output.display.len();
        let extent = self.block_extent();
        let text_area = if let Some((flow, _)) = self.flow.take() {
            self.paint_element(flow, extent, rect, output, measurer)
        } else if let Some(image) = replaced {
            output.display.push(DisplayItem::Image(image));
            text.then(|| text_flow(&[], &[], rect))
        } else if visible {
            self.paint_lines(rect, index, output.lines, output.display);
            text.then(|| text_flow(&output.display[content..], output.lines, rect))
        } else {
            text.then(|| text_flow(&[], &[], rect))
        }
        .map(|area| {
            wrapped_flow(
                area,
                clips.iter().cloned().map(DisplayItem::PushClip).collect(),
                rect,
            )
        });
        output
            .display
            .extend(clips.iter().map(|_| DisplayItem::Pop));
        if let (Some(page_text), Some(area)) = (&mut output.page_text, &text_area) {
            let area = wrapped_flow(
                area.clone(),
                transform.iter().cloned().map(Region::push).collect(),
                output.page,
            );
            page_text.items.push(DisplayItem::Frame(Box::new(area)));
        }
        if clips_to_text
            && let (Some(slot), Some(area)) = (painted, text_area)
            && let DisplayItem::Box(paint) = &mut output.display[slot]
        {
            paint.text_area = Some(std::sync::Arc::new(area));
        }
        // The outline paints over the margin box's content (CSS UI 4 §3.1).
        output.display.extend(outline.map(DisplayItem::Box));
        output
            .display
            .extend(opening.iter().map(|_| DisplayItem::Pop));
        if let Some(region) = transform {
            let areas = output.links.split_off(links);
            output.links.extend(transformed_links(areas, &region));
        }
        Ok(())
    }
    /// The replaced content fitted into the content box by `object-fit` and
    /// `object-position`, clipped to the content edge (CSS Images 3 §4.5); content without
    /// natural dimensions fills it.
    fn replaced(
        &self,
        image: std::sync::Arc<crate::images::Image>,
        paint: &BoxPaint,
    ) -> ImagePaint {
        let style = &self.margin.style;
        let clip = paint.inner_clip(self.padding, [true; 2]);
        let content = clip.rect;
        let box_size = [content.width, content.height];
        let [width, height] = if image.natural_size() == Default::default() {
            box_size
        } else {
            let scale = crate::style::POINTS_PER_PIXEL * style.effective_zoom;
            let natural = image
                .pixel_size(content.width / scale)
                .map(|extent| extent * scale);
            style.object_fit.size(natural, box_size)
        };
        ImagePaint {
            image,
            rect: Rect {
                x: content.x + style.object_position[0].resolve(content.width - width),
                y: content.y + style.object_position[1].resolve(content.height - height),
                width,
                height,
            },
            clip: Some(clip),
            source_scale: style.effective_zoom,
            rendering: style.image_rendering(),
        }
    }
    fn vertical_offset(&self, physical_height: f32) -> f32 {
        let extra = self.content[VERTICAL] - physical_height;
        match self.margin.style.table.vertical_align {
            VerticalAlign::Middle => extra / 2.0,
            VerticalAlign::Bottom => extra,
            VerticalAlign::Top
            | VerticalAlign::Baseline
            | VerticalAlign::BaselineMiddle
            | VerticalAlign::Sub
            | VerticalAlign::Super
            | VerticalAlign::TextTop
            | VerticalAlign::TextBottom
            | VerticalAlign::Length(_) => 0.0,
        }
    }
    fn content_origin(&self, rect: Rect, physical_height: f32) -> [f32; 2] {
        [
            rect.x + self.padding[3] + self.borders[3],
            rect.y + self.padding[0] + self.borders[0] + self.vertical_offset(physical_height),
        ]
    }
    /// A running element lays out unbroken in the content box, in the margin box's flow, or
    /// sliced into the box's column boxes; `extent` is the block size it takes there.
    fn paint_element(
        &self,
        flow: StandaloneFlow,
        extent: f32,
        rect: Rect,
        output: &mut MarginOutput<'_>,
        measurer: &mut Measurer<'t>,
    ) -> Option<FlowPaint> {
        let vertical = self.inline_axis() == VERTICAL;
        let logical = [self.content[self.inline_axis()], extent];
        let size = if vertical {
            [logical[1], logical[0]]
        } else {
            logical
        };
        let margin_flow = crate::style::logical::Flow::used(&self.margin.style);
        let frame = crate::layout::writing::Frame::physical(margin_flow, size);
        // CSS Images 4 §2.7: a running element is copied into each page that places it
        // (CSS GCPM 3 §1.2.2); `element()` images the copy on the first page showing it.
        measurer
            .element_targets
            .displayed
            .borrow_mut()
            .adopt(flow.renderings.physical(margin_flow));
        let (mut items, mut text_area, lines, mut navigation) = match &self.columns {
            None => (
                flow.items,
                flow.text_area,
                flow.lines,
                flow.navigation.into_links(),
            ),
            Some((geometry, columns)) => {
                let mut items: Vec<DisplayItem> = columns
                    .rules(geometry, self.margin.style.multicol.rule, [0.0; 2])
                    .into_iter()
                    .map(DisplayItem::Box)
                    .collect();
                let mut text = Vec::new();
                let mut navigation = Vec::new();
                // Each slice is the flow placed in its column box and clipped to the slice.
                let sliced =
                    |items: &[DisplayItem], lines: &[TextLine], origin: [f32; 2], window: Rect| {
                        let (items, lines) =
                            windowed(items, lines, [window.y, window.y + window.height]);
                        DisplayItem::Frame(Box::new(FlowPaint {
                            matrix: [1.0, 0.0, 0.0, 1.0, origin[0], origin[1]],
                            area: Rect {
                                y: 0.0,
                                height: extent,
                                ..window
                            },
                            items: std::iter::once(slice_clip(window))
                                .chain(items)
                                .chain(std::iter::once(DisplayItem::Pop))
                                .collect(),
                            lines,
                        }))
                    };
                for (slice, [x, y]) in columns.placed(*geometry) {
                    let origin = [x, y - slice.top];
                    let window = Rect {
                        x: 0.0,
                        y: slice.top,
                        width: geometry.width,
                        height: slice.bottom - slice.top,
                    };
                    items.push(sliced(&flow.items, &flow.lines, origin, window));
                    if let Some(area) = &flow.text_area {
                        text.push(sliced(&area.items, &area.lines, origin, window));
                    }
                    navigation.extend(flow.navigation.translated(origin, window).into_links());
                }
                let text_area = flow.text_area.map(|area| FlowPaint {
                    items: text,
                    lines: Vec::new(),
                    ..area
                });
                (items, text_area, Vec::new(), navigation)
            }
        };
        if vertical {
            frame.items(&mut items);
            if let Some(area) = &mut text_area {
                frame.items(&mut area.items);
            }
            for area in &mut navigation {
                match &mut area.shape {
                    crate::layout::LinkShape::Rect(rect) => *rect = frame.rect(*rect),
                    crate::layout::LinkShape::Quads(quads) => {
                        for point in quads.iter_mut().flatten() {
                            *point = frame.point(*point);
                        }
                    }
                }
            }
        }
        let origin = self.content_origin(rect, size[1]);
        for mut area in navigation {
            match &mut area.shape {
                crate::layout::LinkShape::Rect(rect) => {
                    rect.x += origin[0];
                    rect.y += origin[1];
                }
                crate::layout::LinkShape::Quads(quads) => {
                    for point in quads.iter_mut().flatten() {
                        point[0] += origin[0];
                        point[1] += origin[1];
                    }
                }
            }
            output.links.push(area);
        }
        let matrix = [1.0, 0.0, 0.0, 1.0, origin[0], origin[1]];
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: size[0],
            height: size[1],
        };
        output.display.push(DisplayItem::Frame(Box::new(FlowPaint {
            matrix,
            area,
            items,
            lines,
        })));
        text_area.map(|text| FlowPaint {
            matrix,
            area,
            ..text
        })
    }
    fn paint_lines(
        self,
        rect: Rect,
        index: usize,
        page: &mut Vec<TextLine>,
        display: &mut Vec<DisplayItem>,
    ) {
        let style = &self.margin.style;
        let vertical = self.inline_axis() == VERTICAL;
        // Lines align in their column; the columns span the content box's inline size.
        let (inline_extent, frame_extent) = match &self.columns {
            Some((geometry, _)) => (geometry.width, self.content[self.inline_axis()]),
            None if vertical => {
                let widest = self.lines.iter().map(|line| line.width).fold(0.0, f32::max);
                (widest, widest)
            }
            None => (self.content[HORIZONTAL], self.content[HORIZONTAL]),
        };
        let physical_height = if vertical {
            frame_extent
        } else {
            self.block_extent()
        };
        let content_origin = self.content_origin(rect, physical_height);
        let mut frame_lines = Vec::new();
        let mut frame_items = Vec::new();
        let (text_lines, text_items) = if vertical {
            (&mut frame_lines, &mut frame_items)
        } else {
            (&mut *page, &mut *display)
        };
        let origin = if vertical { [0.0; 2] } else { content_origin };
        if let Some((geometry, columns)) = &self.columns {
            text_items.extend(
                columns
                    .rules(geometry, style.multicol.rule, origin)
                    .into_iter()
                    .map(DisplayItem::Box),
            );
        }
        let offsets = self.line_offsets();
        for (line, offset) in self.lines.into_iter().zip(offsets) {
            let x = origin[0] + offset[0] + line.alignment_offset(inline_extent, style.used_rtl());
            let baseline = origin[1] + offset[1] + line.ascent;
            text_items.extend(crate::layout::inline::shadow_items(
                &line.runs,
                &line.decorations,
                [x, baseline],
                &|_, _| true,
            ));
            let [below, above] =
                crate::layout::inline::decoration_items(&line.decorations, x, baseline);
            text_items.extend(below);
            text_items.push(DisplayItem::Text {
                line: text_lines.len(),
            });
            text_items.extend(above);
            let paragraph = self
                .paragraph
                .as_ref()
                .expect("a margin box's lines lay out its paragraph");
            text_items.extend(
                crate::layout::markers::content_image_paints(paragraph, &line, &self.images.0)
                    .into_iter()
                    .map(|mut image| {
                        image.rect.x += x;
                        image.rect.y += baseline;
                        DisplayItem::Image(image)
                    }),
            );
            text_lines.push(TextLine {
                paragraph: index,
                text: line.text,
                x,
                baseline,
                width: line.width,
                height: line.height,
                runs: line.runs,
                font_size: style.size,
                color: style.color,
            });
        }
        if vertical {
            let size = [self.content[HORIZONTAL], frame_extent];
            crate::layout::writing::Frame::physical(crate::style::logical::Flow::used(style), size)
                .items(&mut frame_items);
            display.push(DisplayItem::Frame(Box::new(FlowPaint {
                matrix: [1.0, 0.0, 0.0, 1.0, content_origin[0], content_origin[1]],
                area: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: size[0],
                    height: size[1],
                },
                items: frame_items,
                lines: frame_lines,
            })));
        }
    }
}
/// The text and decorations among `items`, whose text lines are `lines`, drawn opaquely in a
/// flow of their own over `area`: the text a margin box's lines paint.
fn text_flow(items: &[DisplayItem], lines: &[TextLine], area: Rect) -> FlowPaint {
    let mut flow = FlowPaint {
        matrix: IDENTITY,
        area,
        items: Vec::new(),
        lines: Vec::new(),
    };
    for item in items {
        match item {
            DisplayItem::Text { line } => {
                flow.lines.push(lines[*line].clone().opaque());
                flow.items.push(DisplayItem::Text {
                    line: flow.lines.len() - 1,
                });
            }
            DisplayItem::Decoration(decoration) => {
                flow.items
                    .push(DisplayItem::Decoration(decoration.opaque()));
            }
            DisplayItem::Frame(frame) => {
                flow.items.push(DisplayItem::Frame(Box::new(FlowPaint {
                    matrix: frame.matrix,
                    ..text_flow(&frame.items, &frame.lines, frame.area)
                })));
            }
            DisplayItem::PushTransform(_) | DisplayItem::PushProjection(_) | DisplayItem::Pop => {
                flow.items.push(item.clone());
            }
            // Text areas hold text and decorations only, not images or column rules.
            DisplayItem::Image(_) | DisplayItem::TextShadow(_) | DisplayItem::Box(_) => {}
            DisplayItem::PushClip(_)
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PushBlend(_)
            | DisplayItem::PushFilter(_)
            | DisplayItem::PushMask(_)
            | DisplayItem::PushBackdropRoot
            | DisplayItem::Scene(_)
            | DisplayItem::Backdrop(_) => {
                unreachable!(
                    "margin lines paint text, shadows, decorations, images, column rules and their transforms"
                )
            }
        }
    }
    flow
}
/// `area`, a margin box's text, inside the regions `pushes` open; `bounds` holds them.
fn wrapped_flow(area: FlowPaint, pushes: Vec<DisplayItem>, bounds: Rect) -> FlowPaint {
    if pushes.is_empty() {
        return area;
    }
    let pops = pushes.len();
    FlowPaint {
        matrix: IDENTITY,
        area: bounds,
        items: pushes
            .into_iter()
            .chain(std::iter::once(DisplayItem::Frame(Box::new(area))))
            .chain(std::iter::repeat_n(DisplayItem::Pop, pops))
            .collect(),
        lines: Vec::new(),
    }
}
/// `areas`, the link areas of a box's content, mapped through the box's transform.
pub(super) fn transformed_links(
    areas: Vec<crate::layout::LinkArea>,
    region: &Region,
) -> Vec<crate::layout::LinkArea> {
    let polygons: Vec<_> = areas
        .into_iter()
        .flat_map(|area| {
            let polygons = match area.shape {
                crate::layout::LinkShape::Rect(rect) => {
                    vec![crate::layout::navigation::rect_corners(rect).to_vec()]
                }
                crate::layout::LinkShape::Quads(quads) => {
                    quads.into_iter().map(|quad| quad.to_vec()).collect()
                }
            };
            polygons.into_iter().map(move |polygon| {
                (
                    area.link,
                    crate::layout::navigation::map_region(polygon, region),
                )
            })
        })
        .collect();
    crate::layout::navigation::link_areas(&polygons)
}
/// What one page's margin boxes paint, and the page's text area when its page box's
/// backgrounds clip to text, which holds their text.
struct MarginOutput<'p> {
    /// The page box, the containing block of its margin boxes.
    page: Rect,
    /// The perspective the page box gives them.
    perspective: Option<crate::style::transform::Parent3d>,
    lines: &'p mut Vec<TextLine>,
    display: &'p mut Vec<DisplayItem>,
    links: &'p mut Vec<crate::layout::LinkArea>,
    page_text: Option<&'p mut FlowPaint>,
}
/// The page-margin boxes' shared inputs: the selected strings and running elements, the
/// document's destination count, the page-based counters and the document's counters where
/// pages start.
pub(super) struct MarginContext<'s, 't> {
    pub resolver: &'s dyn crate::resources::ResourceResolver,
    pub strings: &'s crate::layout::strings::PageStrings,
    pub running: &'t [RunningStringAssignment],
    pub destinations: usize,
    pub counters: &'s crate::layout::page_counters::PageCounters,
    pub document: &'s crate::document::StyledDocument,
}
/// The running element each page-margin box places, with the `page` counter it shows.
pub(super) fn running(
    page_boxes: &[std::sync::Arc<PageBox>],
    running: &[RunningStringAssignment],
    strings: &crate::layout::strings::PageStrings,
    counters: &crate::layout::page_counters::PageCounters,
) -> Vec<(crate::style::PageStyleTarget, i128)> {
    page_boxes
        .iter()
        .enumerate()
        .flat_map(|(page, page_box)| {
            page_box.margins.iter().filter_map(move |margin| {
                let request = margin.margin.content.running_element()?;
                let element = running[strings.element(page, request)?].element.as_ref()?;
                Some((
                    element.target,
                    counters.page(page, Some(margin.margin.location)),
                ))
            })
        })
        .collect()
}
/// Prepares a page-margin box of the page at index `page` of `pages`, whose text the page
/// box collects when `page_text`.
fn prepare<'a, 't>(
    margin: &'a PageMargin,
    decoration: &'a BoxDecoration,
    containing: [f32; 2],
    page: usize,
    pages: usize,
    context: &MarginContext<'_, 't>,
    page_text: bool,
) -> Result<Prepared<'a, 't>, LayoutError> {
    let style = &margin.style;
    let bases = [containing[1], containing[0], containing[1], containing[0]];
    let padding = std::array::from_fn(|i| style.padding[i].resolve(bases[i]));
    let borders = style
        .decoration
        .borders
        .map(crate::graphics::Border::used_width);
    let margins = std::array::from_fn(|i| style.margin[i].resolve(bases[i]));
    let mut specified = [style.width.resolve(containing[0]), None];
    if style.box_sizing == BoxSizing::BorderBox {
        for axis in [HORIZONTAL, VERTICAL] {
            let edges = Prepared::sides(axis)
                .iter()
                .map(|&i| padding[i] + borders[i])
                .sum::<f32>();
            specified[axis] = specified[axis].map(|size| (size - edges).max(0.0));
        }
    }
    let mut replacement = None;
    let (paragraph, element) = if let Some(request) = margin.content.running_element() {
        (
            None,
            context
                .strings
                .element(page, request)
                .and_then(|owner| context.running[owner].element.as_ref())
                .map(|element| {
                    element.block(context.counters.page(page, Some(margin.location)), pages)
                }),
        )
    } else if let Some(image) = margin.content.replacement() {
        replacement = Some(
            context
                .document
                .margin_replacement(image, style, context.resolver),
        );
        (None, None)
    } else {
        let output = margin.content.page_output(
            context.counters.page(page, Some(margin.location)),
            pages,
            crate::document::MAX_INLINE_TEXT_BYTES,
            &style.quotes,
            &mut |request| Ok(context.strings.text(page, request)),
            &mut |request| {
                Ok(context.counters.text(
                    page,
                    margin.location,
                    request,
                    context
                        .document
                        .page_counters(page)
                        .unwrap_or(&CounterSnapshot::default()),
                ))
            },
        )?;
        crate::document::record_text_limit(
            context.document.styles().degradations(),
            output.limited(),
        );
        let images = context
            .document
            .margin_images(&output, style, context.resolver);
        (Paragraph::generated_output(&output, &images, style)?, None)
    };
    // The paragraph holds the box's first and last formatted lines (CSS Inline 3 §6.1).
    let paragraph = paragraph.map(|mut paragraph| {
        paragraph.set_text_box_trim(style.text_box.trim);
        paragraph
    });
    let images = paragraph
        .as_ref()
        .map(crate::layout::markers::content_image_atomics)
        .unwrap_or_default();
    Ok(Prepared {
        margin,
        decoration,
        paragraph,
        images,
        replacement,
        element,
        flow: None,
        block_size: None,
        columns: None,
        text_area: decoration.backgrounds.clips_to_text() || page_text,
        padding,
        borders,
        margins,
        specified,
        start: [0.0; 2],
        content: [0.0; 2],
        lines: Vec::new(),
    })
}

pub(super) fn render<'t>(
    page_boxes: &[std::sync::Arc<PageBox>],
    context: &MarginContext<'_, 't>,
    measurer: &mut Measurer<'t>,
    first_index: usize,
    (pages, display, links, text_areas, underlays): crate::layout::PageOutputs<'_>,
) -> Result<(), LayoutError> {
    let total = pages.len();
    for (number, ((((page_lines, display), links), page_text), underlay)) in pages
        .iter_mut()
        .zip(display)
        .zip(links.iter_mut())
        .zip(text_areas.iter_mut())
        .zip(underlays.iter_mut())
        .enumerate()
    {
        let page = &page_boxes[number];
        let perspective = crate::layout::page_paint::page_perspective(page);
        let margins: Vec<_> = page
            .margins
            .iter()
            .map(|margin| (&margin.margin, &margin.decoration))
            .collect();
        if margins.is_empty() {
            continue;
        }
        let page_box = page.geometry.margin_box();
        let [width, height] = [page_box.width, page_box.height];
        let [top, right, bottom, left] = page.geometry.margin;
        let groups = [
            ([left, 0.0], [width - left - right, top], HORIZONTAL),
            (
                [left, height - bottom],
                [width - left - right, bottom],
                HORIZONTAL,
            ),
            ([0.0, top], [left, height - top - bottom], VERTICAL),
            (
                [width - right, top],
                [right, height - top - bottom],
                VERTICAL,
            ),
        ];
        let page = page_lines;
        let mut stacked = Vec::with_capacity(margins.len());
        for (locations, (origin, containing, axis)) in GROUPS.iter().zip(groups) {
            let mut boxes: [Option<Prepared<'_, '_>>; 3] = [None, None, None];
            let mut dimensions = [Variable::default(); 3];
            for (position, location) in locations.iter().enumerate() {
                let Some((margin, decoration)) = margins
                    .iter()
                    .find(|(margin, _)| margin.location == *location)
                else {
                    continue;
                };
                measurer.work.spend(1)?;
                let mut prepared = prepare(
                    margin,
                    decoration,
                    containing,
                    number,
                    total,
                    context,
                    page_text.is_some(),
                )?;
                prepared.validate()?;
                let fixed_axis = 1 - axis;
                prepared.fixed(fixed_axis, containing[fixed_axis]);
                let bounds = if axis == prepared.inline_axis() {
                    prepared.intrinsic(measurer, containing[axis])?
                } else {
                    prepared.layout(measurer, context.destinations)?;
                    [prepared.block_extent(); 2]
                };
                if axis == HORIZONTAL && prepared.margin.style.width.intrinsic().is_some() {
                    let available = containing[axis] - prepared.outer_edges(axis);
                    let style = &prepared.margin.style;
                    let sizes = [&style.width, &Size::Auto, &Size::Auto];
                    prepared.specified[axis] = crate::style::SizeConstraints::new(
                        sizes,
                        Some(containing[axis]),
                        style.box_sizing,
                        prepared.edges(axis),
                    )
                    .with_intrinsic(
                        sizes,
                        crate::style::IntrinsicWidths {
                            min: bounds[0],
                            max: bounds[1],
                        },
                        Some(available.max(0.0)),
                    )
                    .specified;
                }
                let edges = prepared.outer_edges(axis);
                dimensions[position] = Variable {
                    min: bounds[0] + edges,
                    max: bounds[1] + edges,
                    specified: prepared.specified[axis].map(|size| size + edges),
                    generated: true,
                };
                boxes[position] = Some(prepared);
            }
            let sizes = variable(dimensions, containing[axis]);
            for (position, prepared) in boxes.into_iter().enumerate() {
                if let Some(mut prepared) = prepared {
                    prepared.variable(axis, containing[axis], sizes[position], position);
                    prepared.layout(measurer, context.destinations)?;
                    let margin = prepared.margin;
                    let mut items = Vec::new();
                    prepared.paint(
                        origin,
                        first_index + margin.location as usize,
                        &mut MarginOutput {
                            page: page_box,
                            perspective,
                            lines: page,
                            display: &mut items,
                            links,
                            page_text: page_text.as_mut(),
                        },
                        measurer,
                    )?;
                    stacked.push((margin, items));
                }
            }
        }
        for (location, origin, containing) in [
            (MarginLocation::TopLeftCorner, [0.0, 0.0], [left, top]),
            (
                MarginLocation::TopRightCorner,
                [width - right, 0.0],
                [right, top],
            ),
            (
                MarginLocation::BottomLeftCorner,
                [0.0, height - bottom],
                [left, bottom],
            ),
            (
                MarginLocation::BottomRightCorner,
                [width - right, height - bottom],
                [right, bottom],
            ),
        ] {
            if let Some((margin, decoration)) = margins
                .iter()
                .find(|(margin, _)| margin.location == location)
            {
                let mut prepared = prepare(
                    margin,
                    decoration,
                    containing,
                    number,
                    total,
                    context,
                    page_text.is_some(),
                )?;
                prepared.validate()?;
                for axis in [HORIZONTAL, VERTICAL] {
                    prepared.fixed(axis, containing[axis]);
                }
                prepared.layout(measurer, context.destinations)?;
                let mut items = Vec::new();
                prepared.paint(
                    origin,
                    first_index + location as usize,
                    &mut MarginOutput {
                        page: page_box,
                        perspective,
                        lines: page,
                        display: &mut items,
                        links,
                        page_text: page_text.as_mut(),
                    },
                    measurer,
                )?;
                stacked.push((*margin, items));
            }
        }
        // CSS Page 3 §3.1: margin boxes stack by `z-index`, then in their default painting
        // order, before the document's canvas when negative and over its content otherwise.
        stacked.sort_by_key(|(margin, _)| {
            (
                margin.style.z_index.unwrap_or_default(),
                margin.location.painting_order(),
            )
        });
        for (margin, items) in stacked {
            if margin.style.z_index.unwrap_or_default() < 0 {
                underlay.extend(items);
            } else {
                display.extend(items);
            }
        }
    }
    Ok(())
}
