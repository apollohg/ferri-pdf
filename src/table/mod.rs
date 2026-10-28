//! Table grid placement, constrained track sizing and collapsed borders, independent of PDF output.
mod collapse;
mod heights;
mod widths;

pub(crate) use collapse::{BorderBox, BorderOrigin, CollapsedBorders, ResolvedBorder};
pub(crate) use heights::{CellHeight, HeightConstraint, HeightInput, RowHeights};
pub(crate) use widths::{CellWidth, ColumnWidth, ColumnWidths, WidthInput};

pub(crate) const MAX_GRID_SLOTS: usize = 1_000_000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    pub columns: usize,
    /// Zero extends to the end of the containing row group.
    pub rows: usize,
}

pub(crate) struct RowGroup {
    pub rows: Vec<Vec<Span>>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Cell {
    pub row: usize,
    pub column: usize,
    pub rows: usize,
    pub columns: usize,
}

pub(crate) struct Grid {
    pub cells: Vec<Cell>,
    pub rows: usize,
    pub columns: usize,
}

impl Grid {
    pub(crate) fn new(groups: &[RowGroup], declared_columns: usize) -> Result<Self, TableError> {
        let rows = groups
            .iter()
            .try_fold(0usize, |rows, group| rows.checked_add(group.rows.len()))
            .ok_or(TableError::GridLimit {
                limit: MAX_GRID_SLOTS,
            })?;
        let mut cells = Vec::new();
        let mut occupied = Vec::new();
        let mut row = 0;
        let mut columns = declared_columns;
        if columns > MAX_GRID_SLOTS
            || rows
                .checked_mul(columns)
                .is_none_or(|slots| slots > MAX_GRID_SLOTS)
        {
            return Err(TableError::GridLimit {
                limit: MAX_GRID_SLOTS,
            });
        }
        for group in groups {
            let group_end = row + group.rows.len();
            for input in &group.rows {
                let mut column = 0;
                for span in input {
                    while occupied.get(column).is_some_and(|&end| end > row) {
                        column += 1;
                    }
                    if span.columns == 0 {
                        return Err(TableError::ZeroColumnSpan);
                    }
                    let end = column
                        .checked_add(span.columns)
                        .filter(|&end| end <= MAX_GRID_SLOTS)
                        .ok_or(TableError::GridLimit {
                            limit: MAX_GRID_SLOTS,
                        })?;
                    columns = columns.max(end);
                    if rows
                        .checked_mul(columns)
                        .is_none_or(|slots| slots > MAX_GRID_SLOTS)
                    {
                        return Err(TableError::GridLimit {
                            limit: MAX_GRID_SLOTS,
                        });
                    }
                    occupied.resize(occupied.len().max(end), 0);
                    let row_end = if span.rows == 0 {
                        group_end
                    } else {
                        row.saturating_add(span.rows).min(group_end)
                    };
                    for until in &mut occupied[column..end] {
                        *until = (*until).max(row_end);
                    }
                    cells.push(Cell {
                        row,
                        column,
                        rows: row_end - row,
                        columns: span.columns,
                    });
                    column = end;
                }
                row += 1;
            }
        }
        Ok(Self {
            cells,
            rows,
            columns,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TableError {
    #[error("table grid exceeds {limit} slots")]
    GridLimit { limit: usize },
    #[error("table cell has zero column span")]
    ZeroColumnSpan,
    #[error("table geometry is non-finite or negative")]
    InvalidGeometry,
    #[error("table has {expected} cells but {actual} measurements")]
    MeasurementCount { expected: usize, actual: usize },
    #[error("table has {expected} rows but {actual} row constraints")]
    RowMeasurementCount { expected: usize, actual: usize },
    #[error("column constraint extends outside the table grid")]
    InvalidColumnRange,
    #[error("border source extends outside the table grid")]
    InvalidBorderRange,
}

fn finite(values: impl IntoIterator<Item = f32>) -> Result<(), TableError> {
    if values
        .into_iter()
        .any(|value| !value.is_finite() || value < 0.0)
    {
        Err(TableError::InvalidGeometry)
    } else {
        Ok(())
    }
}

fn grow(tracks: &mut [f32], extra: f32) {
    let weight: f32 = tracks.iter().sum();
    let count = tracks.len() as f32;
    for track in tracks {
        *track += extra
            * if weight > 0.0 {
                *track / weight
            } else {
                1.0 / count
            };
    }
}
