mod dimensions;
use crate::document::{BlockBox, CounterSnapshot, PageBox, Paragraph, RunningStringAssignment};
use crate::graphics::{BoxDecoration, BoxPaint, DisplayItem, Rect};
use crate::layout::measure::{MeasuredBlock, Measurer};
use crate::layout::{LayoutError, TextLine};
use crate::style::{BoxSizing, MarginLocation, PageMargin, Size, VerticalAlign};
use crate::text::Line;
use dimensions::{Variable, variable};

const HORIZONTAL: usize = 0;
const VERTICAL: usize = 1;
const START: usize = 0;
const CENTER: usize = 1;
const END: usize = 2;
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
    /// The running element `element()` places (CSS GCPM 3 §1.2.2), laid out as the box's
    /// only child.
    element: Option<&'t BlockBox>,
    measured: Option<MeasuredBlock<'t>>,
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
    fn block_axis(&self) -> usize {
        1 - self.inline_axis()
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
    fn layout(&mut self, measurer: &mut Measurer<'t>) -> Result<(), LayoutError> {
        self.validate()?;
        let inline = self.content[self.inline_axis()];
        if let Some(element) = self.element {
            self.measured = Some(measurer.block(
                element,
                0.0,
                inline,
                self.margin.style.used_rtl(),
                false,
                None,
            )?);
            return Ok(());
        }
        self.lines = self
            .paragraph
            .as_ref()
            .map(|p| measurer.text_lines(p, inline, &[]))
            .transpose()?
            .unwrap_or_default();
        Ok(())
    }
    /// The content's min-content and max-content contributions in the inline axis.
    fn intrinsic(
        &self,
        measurer: &mut Measurer<'t>,
        available: f32,
    ) -> Result<[f32; 2], LayoutError> {
        if let Some(element) = self.element {
            let widths = measurer
                .run(|measurer| crate::layout::intrinsic::measure(element, measurer, available))?;
            return Ok([widths.min, widths.max]);
        }
        let intrinsic = self
            .paragraph
            .as_ref()
            .map(|p| measurer.intrinsic_widths(p, &[]))
            .transpose()?
            .unwrap_or_default();
        Ok([intrinsic.min, intrinsic.max])
    }
    fn block_extent(&self) -> f32 {
        if let Some(measured) = &self.measured {
            // The margin box is a block formatting context root, so the element's margins
            // stay inside it.
            return if measured.through {
                let mut margins = measured.before.clone();
                margins.merge(measured.after.clone());
                margins.value()
            } else {
                measured.before.value() + measured.height + measured.after.value()
            };
        }
        self.lines.iter().map(|line| line.height).sum()
    }
    /// The decoration, then the content laid out at its vertical alignment, then the outline.
    fn paint(
        mut self,
        origin: [f32; 2],
        index: usize,
        output: &mut MarginOutput<'_>,
        measurer: &mut Measurer<'t>,
        destinations: usize,
    ) -> Result<(), LayoutError> {
        self.validate()?;
        let block_axis = self.block_axis();
        let mut used = 0.0;
        let mut retained = 0;
        for line in &self.lines {
            if self.content[block_axis] < 0.0
                || (retained > 0 && used + line.height > self.content[block_axis])
            {
                break;
            }
            used += line.height;
            retained += 1;
        }
        self.lines.truncate(retained);
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
        let paint = BoxPaint {
            padding: self.padding,
            ..BoxPaint::whole(rect, self.decoration.clone())
        };
        let outline = paint.outline();
        if self.decoration.is_visible() {
            output.display.push(DisplayItem::Box(paint));
        }
        let extent = self.block_extent();
        if let Some(measured) = self.measured.take() {
            self.paint_element(measured, extent, rect, output, measurer, destinations)?;
        } else {
            self.paint_lines(rect, index, output.lines, output.display);
        }
        // The outline paints over the margin box's content (CSS UI 4 §3.1).
        output.display.extend(outline.map(DisplayItem::Box));
        Ok(())
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
    /// A running element lays out unbroken in the content box, in the margin box's flow.
    fn paint_element(
        &self,
        measured: MeasuredBlock<'t>,
        extent: f32,
        rect: Rect,
        output: &mut MarginOutput<'_>,
        measurer: &mut Measurer<'t>,
        destinations: usize,
    ) -> Result<(), LayoutError> {
        let vertical = self.inline_axis() == VERTICAL;
        let logical = [self.content[self.inline_axis()], extent];
        let size = if vertical {
            [logical[1], logical[0]]
        } else {
            logical
        };
        let flow = crate::layout::pagination::Paginator::standalone(
            measurer,
            measured,
            Rect {
                x: 0.0,
                y: 0.0,
                width: logical[0],
                height: logical[1],
            },
            destinations,
        )?;
        let frame = crate::layout::writing::Frame::physical(
            crate::style::logical::Flow::used(&self.margin.style),
            size,
        );
        let mut items = flow.items;
        let mut navigation = flow.navigation.into_links();
        if vertical {
            frame.items(&mut items);
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
        output
            .display
            .push(DisplayItem::Frame(Box::new(crate::graphics::FlowPaint {
                matrix: [1.0, 0.0, 0.0, 1.0, origin[0], origin[1]],
                area: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: size[0],
                    height: size[1],
                },
                items,
                lines: flow.lines,
            })));
        Ok(())
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
        let inline_extent = if vertical {
            self.lines.iter().map(|line| line.width).fold(0.0, f32::max)
        } else {
            self.content[HORIZONTAL]
        };
        let physical_height = if vertical {
            inline_extent
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
        let mut y = if vertical { 0.0 } else { content_origin[1] };
        for line in self.lines {
            let x = if vertical { 0.0 } else { content_origin[0] }
                + line.alignment_offset(inline_extent, style.used_rtl());
            let baseline = y + line.ascent;
            let [below, above] =
                crate::layout::inline::decoration_items(&line.decorations, x, baseline);
            text_items.extend(below);
            text_items.push(DisplayItem::Text {
                line: text_lines.len(),
            });
            text_items.extend(above);
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
            y += line.height;
        }
        if vertical {
            let size = [self.content[HORIZONTAL], inline_extent];
            crate::layout::writing::Frame::physical(crate::style::logical::Flow::used(style), size)
                .items(&mut frame_items);
            display.push(DisplayItem::Frame(Box::new(crate::graphics::FlowPaint {
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
/// What one page's margin boxes paint.
struct MarginOutput<'p> {
    lines: &'p mut Vec<TextLine>,
    display: &'p mut Vec<DisplayItem>,
    links: &'p mut Vec<crate::layout::LinkArea>,
}
/// The page-margin boxes' shared inputs: the selected strings and running elements, the
/// document's destination count, the page-based counters and the document's counters where
/// pages start.
pub(super) struct MarginContext<'s, 't> {
    pub strings: &'s crate::layout::strings::PageStrings,
    pub running: &'t [RunningStringAssignment],
    pub destinations: usize,
    pub counters: &'s crate::layout::page_counters::PageCounters,
    pub document: &'s crate::document::StyledDocument,
}
/// Prepares a page-margin box of the page at index `page` of `pages`.
fn prepare<'a, 't>(
    margin: &'a PageMargin,
    decoration: &'a BoxDecoration,
    containing: [f32; 2],
    page: usize,
    pages: usize,
    context: &MarginContext<'_, 't>,
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
    let (paragraph, element) = if let Some(request) = margin.content.running_element() {
        (
            None,
            context
                .strings
                .element(page, request)
                .and_then(|owner| context.running[owner].element.as_ref())
                .map(|element| &element.block),
        )
    } else {
        let output = margin.content.page_output(
            context.counters.page(page, Some(margin.location)),
            pages,
            crate::document::MAX_INLINE_TEXT_BYTES,
            &style.quotes,
            &mut |request| Ok(context.strings.text(page, request)),
            &mut |request| {
                context.counters.text(
                    page,
                    margin.location,
                    request,
                    context
                        .document
                        .page_counters(page)
                        .unwrap_or(&CounterSnapshot::default()),
                )
            },
        )?;
        if !output.images().is_empty() {
            return Err(crate::style::StyleError::Unsupported(
                "images in page-margin box content".into(),
            )
            .into());
        }
        (Paragraph::generated_output(&output, &[], style)?, None)
    };
    Ok(Prepared {
        margin,
        decoration,
        paragraph,
        element,
        measured: None,
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
    pages: &mut [Vec<TextLine>],
    display: &mut [Vec<DisplayItem>],
    links: &mut [Vec<crate::layout::LinkArea>],
) -> Result<(), LayoutError> {
    let total = pages.len();
    for (number, ((page_lines, display), links)) in pages
        .iter_mut()
        .zip(display)
        .zip(links.iter_mut())
        .enumerate()
    {
        let page = &page_boxes[number];
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
                let mut prepared = prepare(margin, decoration, containing, number, total, context)?;
                prepared.validate()?;
                let fixed_axis = 1 - axis;
                prepared.fixed(fixed_axis, containing[fixed_axis]);
                let bounds = if axis == prepared.inline_axis() {
                    prepared.intrinsic(measurer, containing[axis])?
                } else {
                    prepared.layout(measurer)?;
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
                    prepared.layout(measurer)?;
                    let index = first_index + prepared.margin.location as usize;
                    prepared.paint(
                        origin,
                        index,
                        &mut MarginOutput {
                            lines: page,
                            display,
                            links,
                        },
                        measurer,
                        context.destinations,
                    )?;
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
                let mut prepared = prepare(margin, decoration, containing, number, total, context)?;
                prepared.validate()?;
                for axis in [HORIZONTAL, VERTICAL] {
                    prepared.fixed(axis, containing[axis]);
                }
                prepared.layout(measurer)?;
                prepared.paint(
                    origin,
                    first_index + location as usize,
                    &mut MarginOutput {
                        lines: page,
                        display,
                        links,
                    },
                    measurer,
                    context.destinations,
                )?;
            }
        }
    }
    Ok(())
}
