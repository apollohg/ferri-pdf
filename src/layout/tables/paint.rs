use super::model::{Data, relative_offset};
use ferri_pdf_document::BlockBox;
use ferri_pdf_graphics::{BoxDecoration, BoxPaint, Rect};
use ferri_pdf_style::Display;
use std::ops::Range;

/// Horizontal grid geometry.
pub(crate) struct Columns<'g> {
    pub x: f32,
    pub table: f32,
    pub rtl: bool,
    pub starts: &'g [f32],
    pub widths: &'g [f32],
    pub collapsed: &'g [bool],
    pub horizontal: f32,
}
impl Columns<'_> {
    /// Left edge and width of `count` columns from `start`, mirrored for right-to-left tables.
    pub(super) fn span(&self, start: usize, count: usize) -> (f32, f32) {
        let visible = self.collapsed[start..start + count]
            .iter()
            .filter(|&&value| !value)
            .count();
        let outer = self.widths[start..start + count].iter().sum::<f32>()
            + self.horizontal * visible.saturating_sub(1) as f32;
        let left = if self.rtl {
            self.table - self.starts[start] - outer
        } else {
            self.starts[start]
        };
        (self.x + left, outer)
    }
}
/// Grid geometry; vertical positions are relative to the row band being painted.
pub(super) struct Areas<'g> {
    pub columns: &'g Columns<'g>,
    pub row_starts: &'g [f32],
    pub vertical: f32,
}
impl Areas<'_> {
    pub(super) fn rect(&self, columns: Range<usize>, rows: Range<usize>, band: usize) -> Rect {
        let (x, width) = self.columns.span(columns.start, columns.len());
        Rect {
            x,
            y: self.row_starts[rows.start] - self.row_starts[band],
            width,
            height: (self.row_starts[rows.end] - self.row_starts[rows.start] - self.vertical)
                .max(0.0),
        }
    }
}

/// Background colour and images of a column, row group or row; their borders do not
/// apply in the separated border model.
fn layer_decoration(source: &BlockBox) -> Option<BoxDecoration> {
    if source.is_canvas_source()
        || (!source.style().visibility.is_visible()
            && !matches!(
                source.style().display,
                Display::TableColumn | Display::TableColumnGroup
            ))
    {
        return None;
    }
    let decoration = source.decoration();
    (decoration.background.alpha > 0.0 || decoration.backgrounds.has_images()).then(|| {
        BoxDecoration {
            background: decoration.background,
            backgrounds: decoration.backgrounds.clone(),
            ..Default::default()
        }
    })
}

/// Source ranges retain the layer's positioning area independently of its cell clip.
#[derive(Clone)]
pub(super) struct CellBackground<'a> {
    ancestors: [Option<&'a BlockBox>; 2],
    cell: usize,
    columns: Range<usize>,
    rows: Range<usize>,
    decoration: BoxDecoration,
}
impl CellBackground<'_> {
    pub(super) fn paint(&self, data: &Data<'_>, areas: &Areas<'_>, band: usize) -> BoxPaint {
        let cell = &data.grid.cells[self.cell];
        let mut rect = areas.rect(
            cell.column..cell.column + cell.columns,
            cell.row..cell.row + cell.rows,
            band,
        );
        let offset = relative_offset(&self.ancestors, areas.columns.table, areas.columns.rtl);
        rect.x += offset[0];
        rect.y += offset[1];
        let mut background = areas.rect(self.columns.clone(), self.rows.clone(), band);
        background.x += offset[0];
        background.y += offset[1];
        BoxPaint {
            background_box: Some(background),
            ..BoxPaint::whole(rect, self.decoration.clone())
        }
    }
}

#[derive(Clone)]
pub(crate) struct BackgroundRows {
    pub rows: Range<usize>,
    pub cell_row: usize,
}
impl CellBackground<'_> {
    pub(super) fn rows(&self, data: &Data<'_>) -> BackgroundRows {
        BackgroundRows {
            rows: self.rows.clone(),
            cell_row: data.grid.cells[self.cell].row,
        }
    }
}

#[derive(Clone)]
pub(crate) struct FragmentBackground<'a> {
    source: CellBackground<'a>,
    ancestors: [Option<&'a BlockBox>; 3],
    columns: Range<usize>,
    pub rows: BackgroundRows,
}
impl<'a> FragmentBackground<'a> {
    pub(crate) fn ancestors(&self) -> [Option<&'a BlockBox>; 2] {
        self.source.ancestors
    }
    pub(super) fn new(source: CellBackground<'a>, data: &Data<'a>) -> Self {
        let rows = source.rows(data);
        let cell = &data.grid.cells[source.cell];
        Self {
            ancestors: data.cell_ancestors(source.cell),
            columns: cell.column..cell.column + cell.columns,
            source,
            rows,
        }
    }
    pub(crate) fn paint(&self, mut rect: Rect, columns: &Columns<'_>) -> BoxPaint {
        let cell_offset = relative_offset(&self.ancestors, columns.table, columns.rtl);
        let layer_offset = relative_offset(&self.source.ancestors, columns.table, columns.rtl);
        let parent_x =
            rect.x - columns.span(self.columns.start, self.columns.len()).0 - cell_offset[0];
        rect.x += layer_offset[0] - cell_offset[0];
        rect.y += layer_offset[1] - cell_offset[1];
        let (x, width) = columns.span(self.source.columns.start, self.source.columns.len());
        BoxPaint {
            background_box: Some(Rect {
                x: x + parent_x + layer_offset[0],
                width,
                ..rect
            }),
            ..BoxPaint::whole(rect, self.source.decoration.clone())
        }
    }
}

struct ColumnLayer {
    columns: Range<usize>,
    decoration: BoxDecoration,
}

pub(super) struct Layers {
    columns: [Vec<ColumnLayer>; 2],
    pub(super) group_rows: Vec<Range<usize>>,
}

impl Layers {
    pub(super) fn new(data: &Data<'_>) -> Self {
        let mut columns = [Vec::new(), Vec::new()];
        for column in &data.columns {
            let Some(decoration) = layer_decoration(column.source) else {
                continue;
            };
            let layer = usize::from(column.source.style().display != Display::TableColumnGroup);
            columns[layer].push(ColumnLayer {
                columns: column.start..column.start + column.span,
                decoration,
            });
        }
        let group_rows = (0..data.groups.len())
            .map(|group| {
                let first = data.row_groups.iter().position(|&g| g == group);
                let last = data.row_groups.iter().rposition(|&g| g == group);
                match (first, last) {
                    (Some(first), Some(last)) => first..last + 1,
                    _ => 0..0,
                }
            })
            .collect();
        Self {
            columns,
            group_rows,
        }
    }

    pub(super) fn backgrounds<'a>(
        &self,
        data: &Data<'a>,
        cells: &[usize],
    ) -> Vec<CellBackground<'a>> {
        let mut paints = Vec::new();
        let all_rows = 0..data.grid.rows;
        for columns in &self.columns {
            for &index in cells {
                let cell = &data.grid.cells[index];
                let first = columns.partition_point(|layer| layer.columns.end <= cell.column);
                if let Some(layer) = columns.get(first)
                    && layer.columns.start <= cell.column
                {
                    paints.push(CellBackground {
                        cell: index,
                        ancestors: [None; 2],
                        columns: layer.columns.clone(),
                        rows: all_rows.clone(),
                        decoration: layer.decoration.clone(),
                    });
                }
            }
        }
        let all_columns = 0..data.grid.columns;
        for &index in cells {
            let cell = &data.grid.cells[index];
            let group = data.row_groups[cell.row];
            if let Some(source) = data.groups[group].source
                && let Some(decoration) = layer_decoration(source)
            {
                paints.push(CellBackground {
                    cell: index,
                    ancestors: [Some(source), None],
                    columns: all_columns.clone(),
                    rows: self.group_rows[group].clone(),
                    decoration,
                });
            }
        }
        for &index in cells {
            let cell = &data.grid.cells[index];
            if let Some(decoration) = layer_decoration(data.rows[cell.row]) {
                paints.push(CellBackground {
                    cell: index,
                    ancestors: [
                        data.groups[data.row_groups[cell.row]].source,
                        Some(data.rows[cell.row]),
                    ],
                    columns: all_columns.clone(),
                    rows: cell.row..cell.row + 1,
                    decoration,
                });
            }
        }
        paints
    }
}
