//! Collapsed border conflict resolution (CSS 2.1 §17.6.2.1) per grid edge segment.
use crate::{Grid, TableError};
use ferri_pdf_graphics::{Border, BorderStyle};
use std::cmp::Ordering;
use std::ops::Range;

/// Element kinds in ascending priority for otherwise equal borders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BorderOrigin {
    Table,
    ColumnGroup,
    Column,
    RowGroup,
    Row,
    Cell,
}

/// A table element covering a rectangle of grid slots.
/// Borders are in logical top/end/bottom/start order.
pub struct BorderBox {
    pub rows: Range<usize>,
    pub columns: Range<usize>,
    pub borders: [Border; 4],
    pub origin: BorderOrigin,
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedBorder {
    pub border: Border,
    pub origin: BorderOrigin,
    /// The element lies before the edge (above it, or towards the inline start).
    leading: bool,
}

/// §17.6.2.1 rule 3: double > solid > dashed > dotted > ridge > outset > groove > inset.
fn style_rank(style: BorderStyle) -> u8 {
    match style {
        BorderStyle::None | BorderStyle::Hidden => 0,
        BorderStyle::Inset => 1,
        BorderStyle::Groove => 2,
        BorderStyle::Outset => 3,
        BorderStyle::Ridge => 4,
        BorderStyle::Dotted => 5,
        BorderStyle::Dashed => 6,
        BorderStyle::Solid => 7,
        BorderStyle::Double => 8,
    }
}

impl ResolvedBorder {
    /// Order of §17.6.2.1: hidden, then width, style, origin, and finally the leading element.
    pub fn priority(&self, other: &Self) -> Ordering {
        let hidden = |border: &Self| border.border.style == BorderStyle::Hidden;
        let present = |border: &Self| border.border.style != BorderStyle::None;
        hidden(self)
            .cmp(&hidden(other))
            .then(present(self).cmp(&present(other)))
            .then(self.border.width.total_cmp(&other.border.width))
            .then(style_rank(self.border.style).cmp(&style_rank(other.border.style)))
            .then(self.origin.cmp(&other.origin))
            .then(self.leading.cmp(&other.leading))
    }
    /// The winner of two borders meeting on one segment; ties keep `self`.
    #[must_use]
    pub fn winner(self, other: Self) -> Self {
        if other.priority(&self) == Ordering::Greater {
            other
        } else {
            self
        }
    }
    /// Painted width; hidden and none borders paint nothing.
    pub fn width(&self) -> f32 {
        self.border.used_width()
    }
    /// Borders that paint identically and so join into one run.
    pub fn same_paint(&self, other: &Self) -> bool {
        self.border.style == other.border.style
            && self.border.width == other.border.width
            && self.border.color == other.border.color
    }
}

/// Resolved borders of every grid edge segment, in logical grid coordinates.
#[derive(Clone)]
pub struct CollapsedBorders {
    pub rows: usize,
    pub columns: usize,
    /// `rows + 1` horizontal lines of `columns` segments.
    horizontal: Vec<Option<ResolvedBorder>>,
    /// `rows` rows of `columns + 1` vertical segments.
    vertical: Vec<Option<ResolvedBorder>>,
}

const TOP: usize = 0;
const END: usize = 1;
const BOTTOM: usize = 2;
const START: usize = 3;

fn merge(slot: &mut Option<ResolvedBorder>, candidate: ResolvedBorder) {
    *slot = Some(match *slot {
        Some(current) => current.winner(candidate),
        None => candidate,
    });
}

impl CollapsedBorders {
    pub fn new(grid: &Grid, boxes: &[BorderBox]) -> Result<Self, TableError> {
        let (rows, columns) = (grid.rows, grid.columns);
        let mut resolved = Self {
            rows,
            columns,
            horizontal: vec![None; (rows + 1) * columns],
            vertical: vec![None; rows * (columns + 1)],
        };
        for element in boxes {
            if element.rows.start >= element.rows.end
                || element.columns.start >= element.columns.end
                || element.rows.end > rows
                || element.columns.end > columns
            {
                return Err(TableError::InvalidBorderRange);
            }
            let candidate = |side: usize| ResolvedBorder {
                border: element.borders[side],
                origin: element.origin,
                leading: side == END || side == BOTTOM,
            };
            for column in element.columns.clone() {
                merge(
                    &mut resolved.horizontal[element.rows.start * columns + column],
                    candidate(TOP),
                );
                merge(
                    &mut resolved.horizontal[element.rows.end * columns + column],
                    candidate(BOTTOM),
                );
            }
            for row in element.rows.clone() {
                merge(
                    &mut resolved.vertical[row * (columns + 1) + element.columns.start],
                    candidate(START),
                );
                merge(
                    &mut resolved.vertical[row * (columns + 1) + element.columns.end],
                    candidate(END),
                );
            }
        }
        // Spanning cells have no edges inside their area.
        for cell in &grid.cells {
            for row in cell.row..cell.row + cell.rows {
                for line in cell.column + 1..cell.column + cell.columns {
                    resolved.vertical[row * (columns + 1) + line] = None;
                }
            }
            for line in cell.row + 1..cell.row + cell.rows {
                for column in cell.column..cell.column + cell.columns {
                    resolved.horizontal[line * columns + column] = None;
                }
            }
        }
        Ok(resolved)
    }

    /// Segment on horizontal grid line `line` (`0..=rows`) over `column`.
    pub fn horizontal(&self, line: usize, column: usize) -> Option<ResolvedBorder> {
        self.horizontal[line * self.columns + column]
    }
    /// Segment on vertical grid line `line` (`0..=columns`) beside `row`.
    pub fn vertical(&self, row: usize, line: usize) -> Option<ResolvedBorder> {
        self.vertical[row * (self.columns + 1) + line]
    }
    fn width(border: Option<ResolvedBorder>) -> f32 {
        border.map_or(0.0, |border| border.width())
    }
    /// Border insets of a cell in logical order: half the first segment of each edge for
    /// spanning cells (CSS 2.1 leaves edges with several segments undefined).
    pub fn cell_insets(&self, cell: &crate::Cell) -> [f32; 4] {
        [
            self.horizontal(cell.row, cell.column),
            self.vertical(cell.row, cell.column + cell.columns),
            self.horizontal(cell.row + cell.rows, cell.column),
            self.vertical(cell.row, cell.column),
        ]
        .map(|border| Self::width(border) / 2.0)
    }
    /// Table border widths in logical order: half the first row's outer cells inline, and half
    /// the widest first and last row segments in the block axis. Wider later borders spill out.
    pub fn table_insets(&self) -> [f32; 4] {
        if self.rows == 0 || self.columns == 0 {
            return [0.0; 4];
        }
        let widest = |line: usize| {
            (0..self.columns)
                .map(|column| Self::width(self.horizontal(line, column)))
                .fold(0.0f32, f32::max)
                / 2.0
        };
        [
            widest(0),
            Self::width(self.vertical(0, self.columns)) / 2.0,
            widest(self.rows),
            Self::width(self.vertical(0, 0)) / 2.0,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RowGroup, Span};
    use ferri_pdf_graphics::Color;

    const RED: Color = Color {
        rgb: [255, 0, 0],
        alpha: 1.0,
    };
    const BLUE: Color = Color {
        rgb: [0, 0, 255],
        alpha: 1.0,
    };

    fn border(width: f32, style: BorderStyle, color: Color) -> Border {
        Border {
            width,
            style,
            color,
        }
    }
    fn none() -> Border {
        border(0.0, BorderStyle::None, Color::BLACK)
    }
    fn grid(rows: &[&[Span]]) -> Grid {
        Grid::new(
            &[RowGroup {
                rows: rows.iter().map(|row| row.to_vec()).collect(),
            }],
            0,
        )
        .unwrap()
    }
    fn single() -> Span {
        Span {
            columns: 1,
            rows: 1,
        }
    }
    fn element(
        rows: Range<usize>,
        columns: Range<usize>,
        borders: [Border; 4],
        origin: BorderOrigin,
    ) -> BorderBox {
        BorderBox {
            rows,
            columns,
            borders,
            origin,
        }
    }
    fn describe(border: Option<ResolvedBorder>) -> String {
        border.map_or("absent".into(), |b| {
            format!(
                "{:?} {}pt {:?} from {:?}",
                b.border.style, b.border.width, b.border.color.rgb, b.origin
            )
        })
    }

    #[test]
    fn hidden_beats_width_which_beats_style_which_beats_origin() {
        let grid = grid(&[&[single(), single()]]);
        let solid = |width, color| border(width, BorderStyle::Solid, color);
        let cases = [
            (
                border(1.0, BorderStyle::Hidden, Color::BLACK),
                solid(9.0, RED),
                BorderStyle::Hidden,
                "hidden suppresses even a wider solid border",
            ),
            (
                border(2.0, BorderStyle::Dotted, BLUE),
                solid(1.0, RED),
                BorderStyle::Dotted,
                "the wider dotted border beats a narrower solid one",
            ),
            (
                border(2.0, BorderStyle::Dotted, BLUE),
                border(2.0, BorderStyle::Dashed, RED),
                BorderStyle::Dashed,
                "dashed outranks dotted at equal width",
            ),
            (
                none(),
                border(0.0, BorderStyle::Solid, RED),
                BorderStyle::Solid,
                "any style other than none beats none, even at zero width",
            ),
        ];
        for (left, right, expected, reason) in cases {
            let boxes = [
                element(
                    0..1,
                    0..1,
                    [none(), left, none(), none()],
                    BorderOrigin::Cell,
                ),
                element(
                    0..1,
                    1..2,
                    [none(), none(), none(), right],
                    BorderOrigin::Cell,
                ),
            ];
            let resolved = CollapsedBorders::new(&grid, &boxes).unwrap();
            let shared = resolved.vertical(0, 1);
            assert_eq!(
                shared.map(|b| b.border.style),
                Some(expected),
                "{reason}; resolved {}",
                describe(shared)
            );
        }
    }

    #[test]
    fn equal_width_styles_rank_double_solid_dashed_dotted_ridge_outset_groove_inset() {
        use BorderStyle::*;
        const RANKED: [BorderStyle; 8] =
            [Double, Solid, Dashed, Dotted, Ridge, Outset, Groove, Inset];
        let grid = grid(&[&[single(), single()]]);
        for (index, &stronger) in RANKED.iter().enumerate() {
            for &weaker in &RANKED[index + 1..] {
                // The weaker style sits on the trailing cell, which would win a tie.
                let boxes = [
                    element(
                        0..1,
                        0..1,
                        [none(), border(3.0, stronger, RED), none(), none()],
                        BorderOrigin::Cell,
                    ),
                    element(
                        0..1,
                        1..2,
                        [none(), none(), none(), border(3.0, weaker, BLUE)],
                        BorderOrigin::Cell,
                    ),
                ];
                let resolved = CollapsedBorders::new(&grid, &boxes).unwrap();
                let shared = resolved.vertical(0, 1);
                assert_eq!(
                    shared.map(|b| b.border.style),
                    Some(stronger),
                    "{stronger:?} must beat {weaker:?}; resolved {}",
                    describe(shared)
                );
            }
        }
    }

    #[test]
    fn equal_borders_prefer_cell_then_row_group_over_column_then_leading_element() {
        let grid = grid(&[&[single(), single()], &[single(), single()]]);
        let solid = |color| border(2.0, BorderStyle::Solid, color);
        let all = |color| [solid(color); 4];
        let green = Color {
            rgb: [0, 128, 0],
            alpha: 1.0,
        };
        let orange = Color {
            rgb: [255, 165, 0],
            alpha: 1.0,
        };
        let boxes = [
            element(0..2, 0..2, all(Color::BLACK), BorderOrigin::Table),
            element(0..2, 0..1, all(green), BorderOrigin::Column),
            element(1..2, 0..2, all(orange), BorderOrigin::RowGroup),
            element(0..1, 0..1, all(RED), BorderOrigin::Cell),
            element(0..1, 1..2, all(BLUE), BorderOrigin::Cell),
        ];
        let resolved = CollapsedBorders::new(&grid, &boxes).unwrap();
        assert_eq!(
            resolved.vertical(1, 0).unwrap().border.color,
            orange,
            "row group beats column and table on the second row's start edge: {}",
            describe(resolved.vertical(1, 0))
        );
        assert_eq!(
            resolved.vertical(1, 1).unwrap().border.color,
            green,
            "only the column has an edge between the uncelled second-row slots: {}",
            describe(resolved.vertical(1, 1))
        );
        assert_eq!(
            resolved.vertical(0, 1).unwrap().border.color,
            RED,
            "between two cells of equal borders the start-side cell wins: {}",
            describe(resolved.vertical(0, 1))
        );
        assert_eq!(
            resolved.horizontal(1, 1).unwrap().border.color,
            BLUE,
            "the upper cell's bottom edge beats the row group's top edge by origin: {}",
            describe(resolved.horizontal(1, 1))
        );
    }

    #[test]
    fn spanning_cells_have_no_interior_segments_and_insets_use_their_first_segment() {
        let wide = Span {
            columns: 2,
            rows: 2,
        };
        let grid = grid(&[
            &[wide, single()],
            &[single()],
            &[single(), single(), single()],
        ]);
        let thin = border(2.0, BorderStyle::Solid, RED);
        let thick = border(6.0, BorderStyle::Solid, BLUE);
        let mut boxes: Vec<_> = grid
            .cells
            .iter()
            .map(|cell| {
                element(
                    cell.row..cell.row + cell.rows,
                    cell.column..cell.column + cell.columns,
                    [thin; 4],
                    BorderOrigin::Cell,
                )
            })
            .collect();
        boxes.push(element(0..3, 0..3, [none(); 4], BorderOrigin::Table));
        boxes.push(element(
            2..3,
            1..2,
            [thick, none(), none(), none()],
            BorderOrigin::Cell,
        ));
        let resolved = CollapsedBorders::new(&grid, &boxes).unwrap();
        for (row, line) in [(0, 1), (1, 1)] {
            assert!(
                resolved.vertical(row, line).is_none(),
                "row {row} line {line} lies inside the 2x2 cell: {}",
                describe(resolved.vertical(row, line))
            );
        }
        for column in 0..2 {
            assert!(
                resolved.horizontal(1, column).is_none(),
                "line 1 column {column} lies inside the 2x2 cell"
            );
        }
        assert_eq!(
            [0, 1].map(|column| resolved.horizontal(2, column).unwrap().border.width),
            [2.0, 6.0],
            "only the second column's segment takes the wider top border of the cell below"
        );
        assert_eq!(
            resolved.cell_insets(&grid.cells[0]),
            [1.0, 1.0, 1.0, 1.0],
            "the spanning cell's bottom inset comes from its first bottom segment, not the 6pt one"
        );
        assert_eq!(
            resolved.cell_insets(&grid.cells[4]),
            [3.0, 1.0, 1.0, 1.0],
            "the cell below takes half of its own 6pt top segment"
        );
        assert_eq!(
            resolved.table_insets(),
            [1.0, 1.0, 1.0, 1.0],
            "table widths come from the first row's outer cells and the first/last lines"
        );
    }

    #[test]
    fn table_inline_insets_ignore_wider_borders_in_later_rows() {
        let grid = grid(&[&[single()], &[single()]]);
        let solid = |width| border(width, BorderStyle::Solid, RED);
        let boxes = [
            element(0..1, 0..1, [solid(1.0); 4], BorderOrigin::Cell),
            element(
                1..2,
                0..1,
                [solid(4.0), solid(1.0), solid(8.0), solid(9.0)],
                BorderOrigin::Cell,
            ),
        ];
        let resolved = CollapsedBorders::new(&grid, &boxes).unwrap();
        assert_eq!(
            resolved.table_insets(),
            [0.5, 0.5, 4.0, 0.5],
            "the 9pt start border of the second row spills into the margin (CSS 2.1 §17.6.2)"
        );
        assert_eq!(
            resolved.cell_insets(&grid.cells[1]),
            [2.0, 0.5, 4.0, 4.5],
            "the second cell takes half of each resolved edge"
        );
    }
}
