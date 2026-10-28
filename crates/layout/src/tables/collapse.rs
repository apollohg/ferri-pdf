//! Painting of resolved collapsed borders for the table bands placed on one page.
use ferri_pdf_graphics::{Border, BorderStyle, BoxDecoration, BoxPaint, Clip, Color, Rect};
use ferri_pdf_table::{CollapsedBorders, ResolvedBorder};
use std::{ops::Range, rc::Rc};

const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;
/// Bands closer than this share a grid line.
const TOUCHING_PT: f32 = 1.0e-3;

/// Collapsed borders with the positions of their grid lines.
#[derive(Clone)]
pub(crate) struct CollapsedGrid {
    pub borders: Rc<CollapsedBorders>,
    /// Physical x of each vertical grid line, in logical column order.
    pub lines: Vec<f32>,
    /// y of each horizontal grid line relative to the first.
    pub offsets: Rc<[f32]>,
}

/// Source rows and physical origin of a band's page fragment.
#[derive(Clone)]
pub(crate) struct BandPlacement {
    pub rows: Range<usize>,
    pub y: f32,
    /// Consumed interval in the band; cuts inside rows have no horizontal border.
    pub slice: Option<Range<f32>>,
}

struct PageLine {
    y: f32,
    segments: Vec<Option<ResolvedBorder>>,
    above: Option<usize>,
    below: Option<usize>,
}
impl PageLine {
    fn merge(&mut self, segments: Vec<Option<ResolvedBorder>>) {
        for (segment, other) in self.segments.iter_mut().zip(segments) {
            *segment = match (*segment, other) {
                (Some(a), Some(b)) => Some(a.winner(b)),
                (a, b) => a.or(b),
            };
        }
    }

    /// Widest horizontal segment beside vertical grid line `at`.
    fn horizontal_crossing(&self, at: usize) -> f32 {
        [at.checked_sub(1), (at < self.segments.len()).then_some(at)]
            .into_iter()
            .flatten()
            .map(|column| width(self.segments[column]))
            .fold(0.0, f32::max)
    }
}
struct PageRow {
    row: usize,
    top: usize,
    bottom: usize,
}
/// A run of identical segments painted as one side of a box spanning it.
struct Run {
    region: RunRegion,
    rect: Rect,
    side: usize,
    /// Widths of the crossing borders at the low and high end, which mitre the run's corners.
    crossing: [f32; 2],
    border: ResolvedBorder,
}
enum RunRegion {
    Horizontal { line: usize, columns: Range<usize> },
    Vertical { rows: Range<usize>, at: usize },
}

pub(crate) struct BorderPaint {
    pub paint: BoxPaint,
    pub clip: Clip,
    pub cell: Option<usize>,
}
impl Run {
    fn paint(self) -> BoxPaint {
        let mut borders = [Border::default(); 4];
        borders[self.side] = Border {
            style: self.border.border.style.collapsed(),
            ..self.border.border
        };
        // Transparent neighbouring sides give the run the corner diagonals of a box side.
        let [low, high] = if matches!(self.side, TOP | BOTTOM) {
            [LEFT, RIGHT]
        } else {
            [TOP, BOTTOM]
        };
        for (side, width) in [low, high].into_iter().zip(self.crossing) {
            borders[side] = Border {
                width,
                style: BorderStyle::Solid,
                color: Color::TRANSPARENT,
            };
        }
        BoxPaint::whole(
            self.rect,
            BoxDecoration {
                borders,
                ..Default::default()
            },
        )
    }
}

fn width(border: Option<ResolvedBorder>) -> f32 {
    border.map_or(0.0, |border| border.width())
}
fn painted(border: Option<ResolvedBorder>) -> Option<ResolvedBorder> {
    border.filter(|border| border.width() > 0.0 && border.border.color.alpha > 0.0)
}

impl CollapsedGrid {
    pub(crate) fn positioned_paints(
        &self,
        bands: &[BandPlacement],
        x_offset: f32,
        used_offsets: &[f32],
        work: &crate::work::Budget,
        owner: impl Fn(usize, usize) -> (Option<usize>, [f32; 2]),
    ) -> Result<Vec<BorderPaint>, crate::LayoutError> {
        let (lines, rows) = self.page_lines(bands, used_offsets);
        let mut runs = Vec::new();
        self.horizontal_runs(&lines, &rows, x_offset, &mut runs);
        self.vertical_runs(&lines, &rows, x_offset, &mut runs);
        runs.sort_by(|a, b| a.border.priority(&b.border));
        let mut result = Vec::new();
        for run in runs {
            let tiles: Vec<_> = match &run.region {
                RunRegion::Horizontal { line, columns } => [lines[*line].above, lines[*line].below]
                    .into_iter()
                    .flatten()
                    .flat_map(|row| {
                        let start = columns.start.saturating_sub(1);
                        let end = (columns.end + 1).min(self.borders.columns);
                        (start..end).map(move |column| (row, column))
                    })
                    .collect(),
                RunRegion::Vertical { rows: covered, at } => (covered.start.saturating_sub(1)
                    ..(covered.end + 1).min(rows.len()))
                    .flat_map(|row| {
                        [
                            at.checked_sub(1),
                            (*at < self.borders.columns).then_some(*at),
                        ]
                        .into_iter()
                        .flatten()
                        .map(move |column| (row, column))
                    })
                    .collect(),
            };
            let paint = run.paint();
            for (row, column) in tiles {
                work.spend(1)?;
                let source = &rows[row];
                let edges = [column, column + 1].map(|at| {
                    let outer = self.lines[at] == self.lines[0]
                        || self.lines[at] == self.lines[self.borders.columns];
                    (
                        self.lines[at] + x_offset,
                        if outer {
                            width(self.borders.vertical(source.row, at)) / 2.0
                        } else {
                            0.0
                        },
                        outer,
                    )
                });
                let [left, right] = if self.lines.first() <= self.lines.last() {
                    edges
                } else {
                    [edges[1], edges[0]]
                };
                let top = lines[source.top].y
                    - if lines[source.top].above.is_none() {
                        width(lines[source.top].segments[column]) / 2.0
                    } else {
                        0.0
                    };
                let bottom = lines[source.bottom].y
                    + if lines[source.bottom].below.is_none() {
                        width(lines[source.bottom].segments[column]) / 2.0
                    } else {
                        0.0
                    };
                let (cell, offset) = owner(source.row, column);
                let mut moved = paint.clone();
                moved.rect.x += offset[0];
                moved.rect.y += offset[1];
                let left_edge = left.0 - left.1 + offset[0];
                let right_edge = right.0 + right.1 + offset[0];
                let top_edge = top + offset[1];
                let bottom_edge = bottom + offset[1];
                let left_edge = if left.2 {
                    left_edge.min(moved.rect.x)
                } else {
                    left_edge
                };
                let right_edge = if right.2 {
                    right_edge.max(moved.rect.x + moved.rect.width)
                } else {
                    right_edge
                };
                let top_edge = if lines[source.top].above.is_none() {
                    top_edge.min(moved.rect.y)
                } else {
                    top_edge
                };
                let bottom_edge = if lines[source.bottom].below.is_none() {
                    bottom_edge.max(moved.rect.y + moved.rect.height)
                } else {
                    bottom_edge
                };
                let rect = Rect {
                    x: left_edge,
                    y: top_edge,
                    width: right_edge - left_edge,
                    height: bottom_edge - top_edge,
                };
                if rect.x >= moved.rect.x + moved.rect.width
                    || rect.x + rect.width <= moved.rect.x
                    || rect.y >= moved.rect.y + moved.rect.height
                    || rect.y + rect.height <= moved.rect.y
                {
                    continue;
                }
                // Cut only where the run crosses its owner's edge, retaining exterior ink.
                let left = rect.x.min(moved.rect.x);
                let top = rect.y.min(moved.rect.y);
                let right = (rect.x + rect.width).max(moved.rect.x + moved.rect.width);
                let bottom = (rect.y + rect.height).max(moved.rect.y + moved.rect.height);
                result.push(BorderPaint {
                    paint: moved,
                    clip: Clip {
                        rect,
                        radii: Default::default(),
                        axes: [false; 2],
                        reference_box: Some(Rect {
                            x: left,
                            y: top,
                            width: right - left,
                            height: bottom - top,
                        }),
                    },
                    cell,
                });
            }
        }
        Ok(result)
    }
    /// One paint per run of identical segments along a grid line, weakest first. Runs reach the
    /// outer edge of the borders crossing their ends and are mitred against them, painting
    /// table sides as follows: horizontal runs as top sides except at the fragment's last line,
    /// vertical runs as right sides except at the leftmost line.
    pub(crate) fn paints(
        &self,
        bands: &[BandPlacement],
        x_offset: f32,
        used_offsets: &[f32],
    ) -> Vec<BoxPaint> {
        let (lines, rows) = self.page_lines(bands, used_offsets);
        let mut runs = Vec::new();
        self.horizontal_runs(&lines, &rows, x_offset, &mut runs);
        self.vertical_runs(&lines, &rows, x_offset, &mut runs);
        runs.sort_by(|a, b| a.border.priority(&b.border));
        runs.into_iter().map(Run::paint).collect()
    }

    /// Grid lines in page order. A band starting where the previous one ends shares that line,
    /// and non-adjacent grid lines meeting there (repeated groups, breaks) resolve per segment.
    fn page_lines(
        &self,
        bands: &[BandPlacement],
        used_offsets: &[f32],
    ) -> (Vec<PageLine>, Vec<PageRow>) {
        let mut lines: Vec<PageLine> = Vec::new();
        let mut rows: Vec<PageRow> = Vec::new();
        let horizontal = |line: usize| -> Vec<_> {
            (0..self.borders.columns)
                .map(|column| self.borders.horizontal(line, column))
                .collect()
        };
        for band in bands {
            let offsets = if band.slice.is_some() {
                used_offsets
            } else {
                &self.offsets
            };
            let base = offsets[band.rows.start];
            let extent = band
                .slice
                .clone()
                .unwrap_or(0.0..offsets[band.rows.end] - base);
            for row in band.rows.clone() {
                let row_top = offsets[row] - base;
                let row_bottom = offsets[row + 1] - base;
                let top = row_top.max(extent.start);
                let bottom = row_bottom.min(extent.end);
                let empty = row_top == row_bottom;
                if band.slice.is_some() && (bottom < top || (bottom == top && !empty)) {
                    continue;
                }
                let top_segments = if (top - row_top).abs() <= TOUCHING_PT {
                    horizontal(row)
                } else {
                    vec![None; self.borders.columns]
                };
                let bottom_segments = if (bottom - row_bottom).abs() <= TOUCHING_PT {
                    horizontal(row + 1)
                } else {
                    vec![None; self.borders.columns]
                };
                let y = band.y + top - extent.start;
                let top_index = if let Some(previous) = lines
                    .last_mut()
                    .filter(|previous| (previous.y - y).abs() <= TOUCHING_PT)
                {
                    previous.merge(top_segments);
                    if !empty {
                        previous.below = Some(rows.len());
                    }
                    lines.len() - 1
                } else {
                    lines.push(PageLine {
                        y,
                        segments: top_segments,
                        above: None,
                        below: (!empty).then_some(rows.len()),
                    });
                    lines.len() - 1
                };
                if empty {
                    lines[top_index].merge(bottom_segments);
                    continue;
                }
                lines.push(PageLine {
                    y: band.y + bottom - extent.start,
                    segments: bottom_segments,
                    above: Some(rows.len()),
                    below: None,
                });
                rows.push(PageRow {
                    row,
                    top: top_index,
                    bottom: lines.len() - 1,
                });
            }
        }
        (lines, rows)
    }

    /// Widest vertical segment meeting horizontal `line` at vertical grid line `at`.
    fn vertical_crossing(&self, line: &PageLine, rows: &[PageRow], at: usize) -> f32 {
        [line.above, line.below]
            .into_iter()
            .flatten()
            .map(|row| width(self.borders.vertical(rows[row].row, at)))
            .fold(0.0, f32::max)
    }

    fn horizontal_runs(
        &self,
        lines: &[PageLine],
        rows: &[PageRow],
        x_offset: f32,
        runs: &mut Vec<Run>,
    ) {
        for (index, line) in lines.iter().enumerate() {
            let side = if index + 1 == lines.len() {
                BOTTOM
            } else {
                TOP
            };
            let mut column = 0;
            while column < line.segments.len() {
                let Some(border) = painted(line.segments[column]) else {
                    column += 1;
                    continue;
                };
                let start = column;
                while column + 1 < line.segments.len()
                    && painted(line.segments[column + 1])
                        .is_some_and(|next| next.same_paint(&border))
                {
                    column += 1;
                }
                column += 1;
                let ends = [start, column]
                    .map(|at| (self.lines[at], self.vertical_crossing(line, rows, at)));
                let [low, high] = if ends[0].0 <= ends[1].0 {
                    ends
                } else {
                    [ends[1], ends[0]]
                };
                let thickness = border.width();
                let left = x_offset + low.0 - low.1 / 2.0;
                runs.push(Run {
                    region: RunRegion::Horizontal {
                        line: index,
                        columns: start..column,
                    },
                    rect: Rect {
                        x: left,
                        y: line.y - thickness / 2.0,
                        width: x_offset + high.0 + high.1 / 2.0 - left,
                        height: thickness,
                    },
                    side,
                    crossing: [low.1, high.1],
                    border,
                });
            }
        }
    }

    fn vertical_runs(
        &self,
        lines: &[PageLine],
        rows: &[PageRow],
        x_offset: f32,
        runs: &mut Vec<Run>,
    ) {
        let leftmost = (0..self.lines.len())
            .min_by(|a, b| self.lines[*a].total_cmp(&self.lines[*b]))
            .unwrap_or(0);
        for at in 0..self.lines.len() {
            let side = if at == leftmost { LEFT } else { RIGHT };
            let mut index = 0;
            while index < rows.len() {
                let Some(border) = painted(self.borders.vertical(rows[index].row, at)) else {
                    index += 1;
                    continue;
                };
                let start = index;
                while index + 1 < rows.len()
                    && rows[index + 1].top == rows[index].bottom
                    && painted(self.borders.vertical(rows[index + 1].row, at))
                        .is_some_and(|next| next.same_paint(&border))
                {
                    index += 1;
                }
                let crossing = [
                    lines[rows[start].top].horizontal_crossing(at),
                    lines[rows[index].bottom].horizontal_crossing(at),
                ];
                let low = lines[rows[start].top].y - crossing[0] / 2.0;
                let high = lines[rows[index].bottom].y + crossing[1] / 2.0;
                index += 1;
                let thickness = border.width();
                runs.push(Run {
                    region: RunRegion::Vertical {
                        rows: start..index,
                        at,
                    },
                    rect: Rect {
                        x: x_offset + self.lines[at] - thickness / 2.0,
                        y: low,
                        width: thickness,
                        height: high - low,
                    },
                    side,
                    crossing,
                    border,
                });
            }
        }
    }
}
