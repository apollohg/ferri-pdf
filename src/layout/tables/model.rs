use crate::document::{BlockBox, BlockContent};
use crate::graphics::Border;
use crate::layout::measure::Measurer;
use crate::layout::measure::{BlockSource, Failure};
use crate::layout::{LayoutError, geometry::Edges, intrinsic};
use crate::style::{BorderCollapse, Display, LengthPercentage, Size, Style};
use crate::table::{
    BorderBox, BorderOrigin, CellWidth, CollapsedBorders, ColumnWidth, Grid, RowGroup, Span,
    TableError,
};
use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc};

pub(super) struct Group<'a> {
    pub(super) source: Option<&'a BlockBox>,
    pub(super) rows: Vec<&'a BlockBox>,
}
pub(super) struct Column<'a> {
    pub(super) source: &'a BlockBox,
    pub(super) start: usize,
    pub(super) span: usize,
}

pub(super) fn relative_offset(sources: &[Option<&BlockBox>], width: f32, rtl: bool) -> [f32; 2] {
    sources
        .iter()
        .flatten()
        .fold([0.0; 2], |mut offset, source| {
            let relative = source.style().relative_offset(width, None, rtl);
            offset[0] += relative[0];
            offset[1] += relative[1];
            offset
        })
}
#[derive(Default)]
pub(crate) struct Inputs<'a> {
    data: RefCell<HashMap<(*const BlockBox, *const Style), Rc<Data<'a>>>>,
}

impl<'a> Inputs<'a> {
    pub(crate) fn get(&self, block: impl Into<BlockSource<'a>>) -> Result<Rc<Data<'a>>, Failure> {
        let block = block.into();
        let key = (
            std::ptr::from_ref(block.block),
            std::ptr::from_ref(block.style()),
        );
        if let Some(data) = self.data.borrow().get(&key) {
            return Ok(data.clone());
        }
        let data = Rc::new(Data::new(block)?);
        self.data.borrow_mut().insert(key, data.clone());
        Ok(data)
    }
}

pub(crate) struct Data<'a> {
    pub(super) grid: Grid,
    pub(super) groups: Vec<Group<'a>>,
    pub(super) rows: Vec<&'a BlockBox>,
    pub(super) row_ranges: Vec<Range<usize>>,
    pub(super) row_groups: Vec<usize>,
    pub(super) cells: Vec<&'a BlockBox>,
    pub(super) columns: Vec<Column<'a>>,
    pub(super) collapsed_columns: Rc<[bool]>,
    pub(super) collapsed_rows: Vec<bool>,
    /// Rows whose cells all hide as empty, which take no height (CSS 2.1 §17.6.1.1).
    pub(super) hidden_rows: Vec<bool>,
    pub(super) captions: Vec<&'a BlockBox>,
    /// Resolved segment borders in the collapsing border model.
    pub(super) collapsed: Option<Rc<CollapsedBorders>>,
    rtl: bool,
}

/// Converts between physical top/right/bottom/left and logical top/end/bottom/start order;
/// the mapping is its own inverse.
fn logical<T: Copy>(sides: [T; 4], rtl: bool) -> [T; 4] {
    if rtl {
        [sides[0], sides[3], sides[2], sides[1]]
    } else {
        sides
    }
}

fn children(block: &BlockBox) -> Result<Vec<&BlockBox>, Failure> {
    block
        .in_flow_children()
        .into_iter()
        .filter(|child| !matches!(child, BlockContent::Events(_)))
        .map(|child| match child {
            BlockContent::Block(child) => Ok(child),
            BlockContent::Inline(_) | BlockContent::Positioned(_) | BlockContent::Events(_) => {
                Err(LayoutError::InvalidTableStructure.into())
            }
        })
        .collect()
}

impl<'a> Data<'a> {
    pub(crate) fn has_captions(&self) -> bool {
        !self.captions.is_empty()
    }

    pub(super) fn cell_ancestors(&self, index: usize) -> [Option<&'a BlockBox>; 3] {
        let row = self.grid.cells[index].row;
        [
            self.groups[self.row_groups[row]].source,
            Some(self.rows[row]),
            Some(self.cells[index]),
        ]
    }
    fn new(block: impl Into<BlockSource<'a>>) -> Result<Self, Failure> {
        let block = block.into();
        let mut groups: Vec<Group<'a>> = Vec::new();
        let mut columns = Vec::new();
        let mut captions = Vec::new();
        let mut column_count = 0usize;
        for child in children(block.block)? {
            match &child.style().display {
                Display::TableRow => {
                    if groups.last().is_none_or(|group| group.source.is_some()) {
                        groups.push(Group {
                            source: None,
                            rows: Vec::new(),
                        });
                    }
                    groups.last_mut().unwrap().rows.push(child);
                }
                role if role.is_row_group() => groups.push(Group {
                    source: Some(child),
                    rows: children(child)?,
                }),
                Display::TableColumn => {
                    let span = child.style().table.column_span;
                    columns.push(Column {
                        source: child,
                        start: column_count,
                        span,
                    });
                    column_count = column_count
                        .checked_add(span)
                        .filter(|&count| count <= crate::table::MAX_GRID_SLOTS)
                        .ok_or(TableError::GridLimit {
                            limit: crate::table::MAX_GRID_SLOTS,
                        })?;
                }
                Display::TableColumnGroup => {
                    let start = column_count;
                    let descendants = children(child)?;
                    if descendants.is_empty() {
                        column_count = column_count
                            .checked_add(child.style().table.column_span)
                            .filter(|&count| count <= crate::table::MAX_GRID_SLOTS)
                            .ok_or(TableError::GridLimit {
                                limit: crate::table::MAX_GRID_SLOTS,
                            })?;
                    } else {
                        for column in descendants {
                            let span = column.style().table.column_span;
                            columns.push(Column {
                                source: column,
                                start: column_count,
                                span,
                            });
                            column_count = column_count
                                .checked_add(span)
                                .filter(|&count| count <= crate::table::MAX_GRID_SLOTS)
                                .ok_or(TableError::GridLimit {
                                    limit: crate::table::MAX_GRID_SLOTS,
                                })?;
                        }
                    }
                    columns.push(Column {
                        source: child,
                        start,
                        span: column_count - start,
                    });
                }
                Display::TableCaption => captions.push(child),
                _ => return Err(LayoutError::InvalidTableStructure.into()),
            }
        }
        let mut rows = Vec::new();
        let mut cells = Vec::new();
        let mut row_ranges = Vec::new();
        let mut row_groups = Vec::new();
        let mut spans = Vec::new();
        for (group_index, group) in groups.iter().enumerate() {
            let mut group_rows = Vec::new();
            for &row in &group.rows {
                if row.style().display != Display::TableRow {
                    return Err(LayoutError::InvalidTableStructure.into());
                }
                rows.push(row);
                let start = cells.len();
                let mut row_cells = Vec::new();
                for cell in children(row)? {
                    if cell.style().display != Display::TableCell {
                        return Err(LayoutError::InvalidTableStructure.into());
                    }
                    row_cells.push(Span {
                        columns: cell.style().table.column_span,
                        rows: cell.style().table.row_span,
                    });
                    cells.push(cell);
                }
                row_ranges.push(start..cells.len());
                row_groups.push(group_index);
                group_rows.push(row_cells);
            }
            spans.push(RowGroup { rows: group_rows });
        }
        let grid = Grid::new(&spans, column_count)?;
        let mut collapsed_columns = vec![false; grid.columns];
        for column in &columns {
            if column.source.style().visibility == crate::style::Visibility::Collapse {
                collapsed_columns[column.start..column.start + column.span].fill(true);
            }
        }
        let collapsed_rows = rows
            .iter()
            .enumerate()
            .map(|(index, source)| {
                source.style().visibility == crate::style::Visibility::Collapse
                    || groups[row_groups[index]].source.is_some_and(|group| {
                        group.style().visibility == crate::style::Visibility::Collapse
                    })
            })
            .collect();
        let mut data = Self {
            collapsed_rows,
            hidden_rows: Vec::new(),
            collapsed_columns: collapsed_columns.into(),
            grid,
            groups,
            rows,
            row_ranges,
            row_groups,
            cells,
            columns,
            captions,
            collapsed: None,
            rtl: block.style().used_rtl(),
        };
        if block.style().table.collapse == BorderCollapse::Collapse {
            data.collapsed = Some(Rc::new(CollapsedBorders::new(
                &data.grid,
                &data.border_boxes(block.style()),
            )?));
        }
        data.hidden_rows = data
            .row_ranges
            .iter()
            .map(|cells| !cells.is_empty() && cells.clone().all(|index| data.hides_cell(index)))
            .collect();
        Ok(data)
    }

    /// Whether the cell paints no borders or backgrounds because `empty-cells: hide` applies
    /// to it and it has no visible content: no children, or `visibility: hidden` (CSS 2.1
    /// §17.6.1.1).
    pub(super) fn hides_cell(&self, index: usize) -> bool {
        let cell = self.cells[index];
        self.collapsed.is_none()
            && !cell.style().table.empty_cells
            && (cell.children().is_empty() || !cell.style().visibility.is_visible())
    }

    fn border_boxes(&self, table: &Style) -> Vec<BorderBox> {
        let (rows, columns) = (self.grid.rows, self.grid.columns);
        if rows == 0 || columns == 0 {
            return Vec::new();
        }
        let element = |style: &Style, rows, columns, origin| BorderBox {
            rows,
            columns,
            borders: logical(style.decoration.borders, self.rtl),
            origin,
        };
        let mut boxes = vec![element(table, 0..rows, 0..columns, BorderOrigin::Table)];
        for column in &self.columns {
            let style = column.source.style();
            let origin = if style.display == Display::TableColumnGroup {
                BorderOrigin::ColumnGroup
            } else {
                BorderOrigin::Column
            };
            boxes.push(element(
                style,
                0..rows,
                column.start..column.start + column.span,
                origin,
            ));
        }
        let mut first = 0;
        for group in &self.groups {
            let end = first + group.rows.len();
            if let Some(source) = group.source.filter(|_| end > first) {
                boxes.push(element(
                    source.style(),
                    first..end,
                    0..columns,
                    BorderOrigin::RowGroup,
                ));
            }
            first = end;
        }
        for (index, row) in self.rows.iter().enumerate() {
            boxes.push(element(
                row.style(),
                index..index + 1,
                0..columns,
                BorderOrigin::Row,
            ));
        }
        for (cell, source) in self.grid.cells.iter().zip(&self.cells) {
            boxes.push(element(
                source.style(),
                cell.row..cell.row + cell.rows,
                cell.column..cell.column + cell.columns,
                BorderOrigin::Cell,
            ));
        }
        boxes
    }

    /// Used border widths of a cell: half its collapsed edges, or its own borders.
    pub(super) fn cell_borders(&self, index: usize) -> [f32; 4] {
        match &self.collapsed {
            Some(collapsed) => logical(collapsed.cell_insets(&self.grid.cells[index]), self.rtl),
            None => self.cells[index]
                .style()
                .decoration
                .borders
                .map(Border::used_width),
        }
    }

    /// Used table padding and border widths; collapsed tables have no padding (CSS 2.1 §17.6.2).
    pub(crate) fn table_edges(&self, style: &Style, reference: f32) -> Edges {
        match &self.collapsed {
            Some(collapsed) => Edges {
                padding: [0.0; 4],
                border: logical(collapsed.table_insets(), self.rtl),
            },
            None => Edges::of(style, reference),
        }
    }

    pub(super) fn widths(
        &self,
        measurer: &Measurer<'a>,
        reference: f32,
        available: f32,
    ) -> Result<Vec<CellWidth>, Failure> {
        self.cells
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                let content = intrinsic::content(*cell, measurer, available)?;
                let style = cell.style();
                let border = self.cell_borders(index);
                let edges = style.padding[1].resolve(reference)
                    + style.padding[3].resolve(reference)
                    + border[1]
                    + border[3];
                Ok(CellWidth {
                    min: content.min + edges,
                    max: content.max + edges,
                    specified: style.width.clone(),
                    minimum: style.min_width.clone(),
                    maximum: style.max_width.clone(),
                    box_sizing: style.box_sizing,
                    edges,
                })
            })
            .collect()
    }
    pub(super) fn column_widths(&self) -> Vec<ColumnWidth> {
        self.columns
            .iter()
            .map(|column| {
                let scale = |size: &Size| match size {
                    Size::Value(LengthPercentage::Length(value)) => {
                        Size::Value(LengthPercentage::Length(value * column.span as f32))
                    }
                    Size::Value(LengthPercentage::Percentage(value)) => {
                        Size::Value(LengthPercentage::Percentage(value * column.span as f32))
                    }
                    other => other.clone(),
                };
                ColumnWidth {
                    start: column.start,
                    span: column.span,
                    specified: scale(&column.source.style().width),
                    minimum: scale(&column.source.style().min_width),
                    maximum: scale(&column.source.style().max_width),
                    group: column.source.style().display == Display::TableColumnGroup,
                }
            })
            .collect()
    }
}
