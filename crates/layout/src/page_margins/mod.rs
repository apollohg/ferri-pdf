mod dimensions;
use crate::measure::Measurer;
use crate::{LayoutError, TextLine};
use dimensions::{Variable, variable};
use ferri_pdf_document::{PageBox, Paragraph};
use ferri_pdf_graphics::{BoxDecoration, BoxPaint, DisplayItem, Rect};
use ferri_pdf_style::{BoxSizing, MarginLocation, PageMargin, Size, VerticalAlign};
use ferri_pdf_text::Line;

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
struct Prepared<'a> {
    margin: &'a PageMargin,
    decoration: &'a BoxDecoration,
    paragraph: Option<Paragraph>,
    padding: [f32; 4],
    borders: [f32; 4],
    margins: [Option<f32>; 4],
    specified: [Option<f32>; 2],
    start: [f32; 2],
    content: [f32; 2],
    lines: Vec<Line>,
}
impl Prepared<'_> {
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
    fn layout(&mut self, measurer: &Measurer<'_>) -> Result<(), LayoutError> {
        self.validate()?;
        self.lines = self
            .paragraph
            .as_ref()
            .map(|p| measurer.text_lines(p, self.content[HORIZONTAL], &[]))
            .transpose()?
            .unwrap_or_default();
        Ok(())
    }
    fn height(&self) -> f32 {
        self.lines.iter().map(|line| line.height).sum()
    }
    fn paint(
        mut self,
        origin: [f32; 2],
        index: usize,
        page: &mut Vec<TextLine>,
        display: &mut Vec<DisplayItem>,
    ) -> Result<(), LayoutError> {
        self.validate()?;
        let mut used = 0.0;
        let mut retained = 0;
        for line in &self.lines {
            if self.content[VERTICAL] < 0.0
                || (retained > 0 && used + line.height > self.content[VERTICAL])
            {
                break;
            }
            used += line.height;
            retained += 1;
        }
        self.lines.truncate(retained);
        let style = &self.margin.style;
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
        if self.decoration.is_visible() {
            display.push(DisplayItem::Box(BoxPaint {
                padding: self.padding,
                ..BoxPaint::whole(rect, self.decoration.clone())
            }));
        }
        let extra = self.content[1] - self.height();
        let offset = match style.table.vertical_align {
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
        };
        let mut y = rect.y + self.padding[0] + self.borders[0] + offset;
        for line in self.lines {
            let x = rect.x
                + self.padding[3]
                + self.borders[3]
                + line.alignment_offset(self.content[0], style.rtl);
            let baseline = y + line.ascent;
            let [below, above] = crate::inline::decoration_items(&line.decorations, x, baseline);
            display.extend(below);
            display.push(DisplayItem::Text { line: page.len() });
            display.extend(above);
            page.push(TextLine {
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
        Ok(())
    }
}
fn prepare<'a>(
    margin: &'a PageMargin,
    decoration: &'a BoxDecoration,
    containing: [f32; 2],
    page: usize,
    pages: usize,
    strings: &crate::strings::PageStrings,
) -> Result<Prepared<'a>, LayoutError> {
    let style = &margin.style;
    let bases = [containing[1], containing[0], containing[1], containing[0]];
    let padding = std::array::from_fn(|i| style.padding[i].resolve(bases[i]));
    let borders = style
        .decoration
        .borders
        .map(ferri_pdf_graphics::Border::used_width);
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
    Ok(Prepared {
        margin,
        decoration,
        paragraph: Paragraph::generated_output(
            &margin.content.page_output_with_strings(
                page,
                pages,
                ferri_pdf_document::MAX_INLINE_TEXT_BYTES,
                &style.quotes,
                &mut |request| Ok(strings.text(page - 1, request)),
            )?,
            style,
        )?,
        padding,
        borders,
        margins,
        specified,
        start: [0.0; 2],
        content: [0.0; 2],
        lines: Vec::new(),
    })
}

pub(super) fn render(
    page_boxes: &[std::sync::Arc<PageBox>],
    strings: &crate::strings::PageStrings,
    measurer: &Measurer<'_>,
    first_index: usize,
    pages: &mut [Vec<TextLine>],
    display: &mut [Vec<DisplayItem>],
) -> Result<(), LayoutError> {
    let total = pages.len();
    for (number, (page_lines, display)) in pages.iter_mut().zip(display).enumerate() {
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
            let mut boxes: [Option<Prepared<'_>>; 3] = [None, None, None];
            let mut dimensions = [Variable::default(); 3];
            for (position, location) in locations.iter().enumerate() {
                let Some((margin, decoration)) = margins
                    .iter()
                    .find(|(margin, _)| margin.location == *location)
                else {
                    continue;
                };
                let mut prepared =
                    prepare(margin, decoration, containing, number + 1, total, strings)?;
                prepared.validate()?;
                let bounds = if axis == HORIZONTAL {
                    prepared.fixed(VERTICAL, containing[1]);
                    let intrinsic = prepared
                        .paragraph
                        .as_ref()
                        .map(|p| measurer.intrinsic_widths(p, &[]))
                        .transpose()?
                        .unwrap_or_default();
                    if let Size::Intrinsic(keyword) = prepared.margin.style.width {
                        let available = containing[HORIZONTAL] - prepared.outer_edges(HORIZONTAL);
                        prepared.specified[HORIZONTAL] =
                            Some(keyword.resolve(intrinsic, Some(available.max(0.0))));
                    }
                    [intrinsic.min, intrinsic.max]
                } else {
                    prepared.fixed(HORIZONTAL, containing[0]);
                    prepared.layout(measurer)?;
                    [prepared.height(); 2]
                };
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
                    if axis == HORIZONTAL {
                        prepared.layout(measurer)?;
                    }
                    let index = first_index + prepared.margin.location as usize;
                    prepared.paint(origin, index, page, display)?;
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
                let mut prepared =
                    prepare(margin, decoration, containing, number + 1, total, strings)?;
                prepared.validate()?;
                for axis in [HORIZONTAL, VERTICAL] {
                    prepared.fixed(axis, containing[axis]);
                }
                prepared.layout(measurer)?;
                prepared.paint(origin, first_index + location as usize, page, display)?;
            }
        }
    }
    Ok(())
}
