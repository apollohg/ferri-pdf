use crate::style::{BoxSizing, Size, SizeConstraints, VerticalAlign};
use crate::table::{Grid, TableError, finite, grow};
use std::ops::Range;

#[derive(Clone, Debug, Default)]
pub(crate) struct HeightConstraint {
    pub specified: Size,
    pub minimum: Size,
    pub maximum: Size,
    pub box_sizing: BoxSizing,
    pub edges: f32,
}

impl HeightConstraint {
    fn outer(&self, natural: f32, basis: Option<f32>) -> Result<f32, TableError> {
        finite([self.edges])?;
        for size in [&self.specified, &self.minimum, &self.maximum] {
            finite(size.resolve(basis.unwrap_or(0.0)))?;
        }
        let constraints = SizeConstraints::new(
            [&self.specified, &self.minimum, &self.maximum],
            basis,
            self.box_sizing,
            self.edges,
        );
        let content = (natural - self.edges).max(0.0);
        Ok(natural.max(constraints.constrain(constraints.preferred(content)) + self.edges))
    }
    fn is_auto(&self) -> bool {
        [&self.specified, &self.minimum, &self.maximum]
            .iter()
            .all(|size| size.is_auto_block_size(false))
    }
}

#[derive(Clone)]
pub(crate) struct CellHeight {
    pub height: f32,
    pub baseline: f32,
    pub align: VerticalAlign,
    pub constraint: HeightConstraint,
}

pub(crate) struct HeightInput<'a> {
    pub cells: &'a [CellHeight],
    /// An empty slice denotes unconstrained rows.
    pub rows: &'a [HeightConstraint],
    /// Content-box constraints, including the grid's border spacing.
    pub table: SizeConstraints,
    pub spacing: f32,
    /// Rows that take no height and border spacing on one side only (CSS 2.1 §17.6.1.1); an
    /// empty slice hides none.
    pub hidden: &'a [bool],
}

impl<'a> HeightInput<'a> {
    pub(crate) fn natural(cells: &'a [CellHeight], spacing: f32) -> Self {
        Self {
            cells,
            rows: &[],
            spacing,
            hidden: &[],
            table: SizeConstraints::fixed(None, 0.0, None),
        }
    }
}

pub(crate) struct RowHeights {
    pub heights: Vec<f32>,
    pub offsets: Vec<f32>,
    pub table: f32,
}

impl Grid {
    pub(crate) fn row_heights(&self, input: &HeightInput<'_>) -> Result<RowHeights, TableError> {
        let cells = input.cells;
        if cells.len() != self.cells.len() {
            return Err(TableError::MeasurementCount {
                expected: self.cells.len(),
                actual: cells.len(),
            });
        }
        for count in [input.rows.len(), input.hidden.len()] {
            if count != 0 && count != self.rows {
                return Err(TableError::RowMeasurementCount {
                    expected: self.rows,
                    actual: count,
                });
            }
        }
        let hidden = |row: usize| input.hidden.get(row).copied().unwrap_or(false);
        let open_rows = |rows: Range<usize>| rows.filter(|&row| !hidden(row)).count();
        // A hidden row keeps the spacing on one side only.
        let span_spacing =
            |rows: Range<usize>| input.spacing * open_rows(rows).saturating_sub(1) as f32;
        finite([input.spacing, input.table.minimum])?;
        finite(input.table.specified)?;
        finite(input.table.maximum)?;
        let mut baselines = vec![0.0f32; self.rows];
        let mut auto = vec![true; self.rows];
        for (row, constraint) in input.rows.iter().enumerate() {
            auto[row] = constraint.is_auto();
        }
        for (cell, measure) in self.cells.iter().zip(cells) {
            finite([measure.height, measure.baseline])?;
            if measure.align == VerticalAlign::Baseline {
                baselines[cell.row] = baselines[cell.row].max(measure.baseline);
            }
            if !measure.constraint.is_auto() {
                auto[cell.row..cell.row + cell.rows].fill(false);
            }
        }
        let mut spanning: Vec<_> = self
            .cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.rows > 1)
            .map(|(index, _)| index)
            .collect();
        spanning.sort_by_key(|&index| self.cells[index].rows);
        let measure_rows = |basis: Option<f32>| -> Result<Vec<f32>, TableError> {
            let mut heights = vec![0.0f32; self.rows];
            for (row, constraint) in input.rows.iter().enumerate() {
                if !hidden(row) {
                    heights[row] = constraint.outer(0.0, basis)?;
                }
            }
            for (cell, measure) in self.cells.iter().zip(cells) {
                if cell.rows == 1 && !hidden(cell.row) {
                    let baseline = if measure.align == VerticalAlign::Baseline {
                        baselines[cell.row] - measure.baseline
                    } else {
                        0.0
                    };
                    heights[cell.row] = heights[cell.row]
                        .max(measure.constraint.outer(measure.height, basis)? + baseline);
                }
            }
            for &index in &spanning {
                let cell = &self.cells[index];
                let measure = &cells[index];
                let range = cell.row..cell.row + cell.rows;
                let open: Vec<usize> = range.clone().filter(|&row| !hidden(row)).collect();
                if open.is_empty() {
                    continue;
                }
                let height = heights[range.clone()].iter().sum::<f32>() + span_spacing(range);
                let baseline = if measure.align == VerticalAlign::Baseline {
                    baselines[cell.row] - measure.baseline
                } else {
                    0.0
                };
                let required = measure.constraint.outer(measure.height, basis)? + baseline;
                let mut tracks: Vec<f32> = open.iter().map(|&row| heights[row]).collect();
                grow(&mut tracks, (required - height).max(0.0));
                for (row, track) in open.into_iter().zip(tracks) {
                    heights[row] = track;
                }
            }
            finite(heights.iter().copied())?;
            Ok(heights)
        };
        let mut heights = measure_rows(None)?;
        let gaps = if self.rows == 0 {
            0.0
        } else {
            input.spacing * (open_rows(0..self.rows) + 1) as f32
        };
        let base = heights.iter().sum::<f32>();
        let natural = base + gaps;
        let table = natural.max(input.table.constrain(input.table.preferred(natural)));
        finite([table])?;
        if self.rows > 0 && table > natural {
            let target = table - gaps;
            let reference = measure_rows(Some(table))?;
            let reference: Vec<_> = reference
                .into_iter()
                .zip(&heights)
                .map(|(reference, &base)| reference.max(base))
                .collect();
            let sum = reference.iter().sum::<f32>();
            finite([sum])?;
            if target <= sum {
                let weight = if sum > base {
                    (target - base) / (sum - base)
                } else {
                    0.0
                };
                for (height, reference) in heights.iter_mut().zip(reference) {
                    *height += (reference - *height) * weight;
                }
            } else {
                let open = |row: usize| !hidden(row);
                let count = (0..self.rows).filter(|&row| open(row) && auto[row]).count();
                let grown = if count == 0 {
                    open_rows(0..self.rows)
                } else {
                    count
                };
                let increment = (target - sum) / grown.max(1) as f32;
                for (row, (height, reference)) in heights.iter_mut().zip(reference).enumerate() {
                    *height = reference
                        + if open(row) && (count == 0 || auto[row]) {
                            increment
                        } else {
                            0.0
                        };
                }
            }
        }
        let offsets = self
            .cells
            .iter()
            .zip(cells)
            .map(|(cell, measure)| {
                let rows = cell.row..cell.row + cell.rows;
                // A cell of hidden rows only has no height to align in.
                if open_rows(rows.clone()) == 0 {
                    return 0.0;
                }
                let height = heights[rows.clone()].iter().sum::<f32>() + span_spacing(rows);
                match measure.align {
                    VerticalAlign::Top => 0.0,
                    VerticalAlign::Middle => (height - measure.height) / 2.0,
                    VerticalAlign::Bottom => height - measure.height,
                    VerticalAlign::Baseline
                    | VerticalAlign::BaselineMiddle
                    | VerticalAlign::Sub
                    | VerticalAlign::Super
                    | VerticalAlign::TextTop
                    | VerticalAlign::TextBottom
                    | VerticalAlign::Length(_) => baselines[cell.row] - measure.baseline,
                }
            })
            .collect::<Vec<_>>();
        finite(heights.iter().chain(&offsets).copied())?;
        Ok(RowHeights {
            heights,
            offsets,
            table,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::{RowGroup, Span};

    #[test]
    fn table_target_distributes_auto_rows_and_interpolates_percentages() {
        use crate::style::{LengthPercentage, Size};
        let grid = Grid::new(
            &[RowGroup {
                rows: vec![
                    vec![Span {
                        columns: 1,
                        rows: 1,
                    }],
                    vec![Span {
                        columns: 1,
                        rows: 1,
                    }],
                ],
            }],
            0,
        )
        .unwrap();
        let cells = [10.0, 30.0].map(|height| CellHeight {
            height,
            baseline: 0.0,
            align: VerticalAlign::Top,
            constraint: HeightConstraint::default(),
        });
        let length = |value| HeightConstraint {
            specified: Size::Value(LengthPercentage::Length(value)),
            ..HeightConstraint::default()
        };
        let percent = |value| HeightConstraint {
            specified: Size::Value(LengthPercentage::Percentage(value)),
            ..HeightConstraint::default()
        };
        for (rows, expected) in [
            (
                [HeightConstraint::default(), HeightConstraint::default()],
                [40.0, 60.0],
            ),
            ([length(40.0), HeightConstraint::default()], [40.0, 60.0]),
            ([percent(0.25), HeightConstraint::default()], [25.0, 75.0]),
            ([percent(0.8), percent(0.8)], [45.0, 55.0]),
            ([length(20.0), length(40.0)], [40.0, 60.0]),
        ] {
            let result = grid
                .row_heights(&HeightInput {
                    rows: &rows,
                    table: SizeConstraints::fixed(Some(100.0), 0.0, None),
                    ..HeightInput::natural(&cells, 0.0)
                })
                .unwrap();
            assert_eq!(result.heights, expected, "row constraints: {rows:?}");
            assert_eq!(result.table, 100.0);
        }
    }

    #[test]
    fn table_constraints_preserve_intrinsic_floor_spacing_and_empty_height() {
        let grid = Grid::new(
            &[RowGroup {
                rows: vec![
                    vec![Span {
                        columns: 1,
                        rows: 1,
                    }],
                    vec![Span {
                        columns: 1,
                        rows: 1,
                    }],
                ],
            }],
            0,
        )
        .unwrap();
        let cells = [10.0, 30.0].map(|height| CellHeight {
            height,
            baseline: 0.0,
            align: VerticalAlign::Top,
            constraint: HeightConstraint::default(),
        });
        for (specified, minimum, maximum, expected) in [
            (Some(20.0), 0.0, None, [10.0, 30.0]),
            (None, 100.0, Some(20.0), [34.0, 54.0]),
            (Some(100.0), 0.0, Some(60.0), [14.0, 34.0]),
            (None, 0.0, Some(0.0), [10.0, 30.0]),
        ] {
            let result = grid
                .row_heights(&HeightInput {
                    table: SizeConstraints::fixed(specified, minimum, maximum),
                    ..HeightInput::natural(&cells, 4.0)
                })
                .unwrap();
            assert_eq!(
                result.heights, expected,
                "specified={specified:?}, minimum={minimum}, maximum={maximum:?}"
            );
            assert_eq!(result.table, expected.iter().sum::<f32>() + 12.0);
        }
        let empty = Grid::new(&[], 0).unwrap();
        let result = empty
            .row_heights(&HeightInput {
                table: SizeConstraints::fixed(Some(100.0), 0.0, None),
                ..HeightInput::natural(&[], 4.0)
            })
            .unwrap();
        assert_eq!(result.table, 100.0);
        assert_eq!(result.heights, [] as [f32; 0]);
    }

    #[test]
    fn cell_constraints_include_box_edges_and_keep_natural_content() {
        use crate::style::LengthPercentage;
        let grid = Grid::new(
            &[RowGroup {
                rows: vec![
                    vec![Span {
                        columns: 1,
                        rows: 1,
                    }],
                    vec![Span {
                        columns: 1,
                        rows: 1,
                    }],
                ],
            }],
            0,
        )
        .unwrap();
        for (sizing, natural, minimum, maximum, expected) in [
            (
                BoxSizing::ContentBox,
                10.0,
                Size::Auto,
                Size::Auto,
                [50.0, 50.0],
            ),
            (
                BoxSizing::BorderBox,
                10.0,
                Size::Auto,
                Size::Auto,
                [40.0, 60.0],
            ),
            (
                BoxSizing::BorderBox,
                70.0,
                Size::Auto,
                Size::Auto,
                [70.0, 30.0],
            ),
            (
                BoxSizing::BorderBox,
                10.0,
                Size::Value(LengthPercentage::Length(60.0)),
                Size::Value(LengthPercentage::Length(20.0)),
                [60.0, 40.0],
            ),
        ] {
            let cells = [
                CellHeight {
                    height: natural,
                    baseline: 0.0,
                    align: VerticalAlign::Bottom,
                    constraint: HeightConstraint {
                        specified: Size::Value(LengthPercentage::Length(40.0)),
                        minimum,
                        maximum,
                        box_sizing: sizing,
                        edges: 10.0,
                    },
                },
                CellHeight {
                    height: 30.0,
                    baseline: 0.0,
                    align: VerticalAlign::Top,
                    constraint: HeightConstraint::default(),
                },
            ];
            let result = grid
                .row_heights(&HeightInput {
                    table: SizeConstraints::fixed(Some(100.0), 0.0, None),
                    ..HeightInput::natural(&cells, 0.0)
                })
                .unwrap();
            assert_eq!(
                result.heights, expected,
                "sizing={sizing:?}, natural={natural}"
            );
            assert_eq!(result.offsets[0], expected[0] - natural);
        }
    }

    #[test]
    fn spanning_height_grows_rows_proportionally_and_updates_alignment() {
        let grid = Grid::new(
            &[RowGroup {
                rows: vec![
                    vec![
                        Span {
                            columns: 1,
                            rows: 2,
                        },
                        Span {
                            columns: 1,
                            rows: 1,
                        },
                    ],
                    vec![Span {
                        columns: 1,
                        rows: 1,
                    }],
                ],
            }],
            0,
        )
        .unwrap();
        for (natural, spacing, spanning, expected) in [
            ([30.0, 10.0], 0.0, 80.0, [60.0, 20.0]),
            ([30.0, 10.0], 4.0, 84.0, [60.0, 20.0]),
            ([0.0, 0.0], 4.0, 84.0, [40.0, 40.0]),
        ] {
            let measured = [
                CellHeight {
                    height: spanning,
                    baseline: 0.0,
                    align: VerticalAlign::Top,
                    constraint: HeightConstraint::default(),
                },
                CellHeight {
                    height: natural[0],
                    baseline: 0.0,
                    align: VerticalAlign::Middle,
                    constraint: HeightConstraint::default(),
                },
                CellHeight {
                    height: natural[1],
                    baseline: 0.0,
                    align: VerticalAlign::Bottom,
                    constraint: HeightConstraint::default(),
                },
            ];
            let result = grid
                .row_heights(&HeightInput::natural(&measured, spacing))
                .unwrap();
            assert_eq!(
                result.heights, expected,
                "natural={natural:?}, spacing={spacing}, rowspan={spanning}"
            );
            assert_eq!(
                result.offsets,
                [
                    0.0,
                    (expected[0] - natural[0]) / 2.0,
                    expected[1] - natural[1]
                ],
                "alignment must use the distributed row heights"
            );
        }
    }
}
