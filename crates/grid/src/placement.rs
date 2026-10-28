//! Line-based placement and the auto-placement algorithm (CSS Grid 1 §8).
use crate::GridError;
use ferri_pdf_style::{AutoFlow, COLUMNS, GridLine, MAX_GRID_LINE, ROWS};
use std::{ops::Range, sync::Arc};

/// Start and end lines of an item, indexed by [`ROWS`] and [`COLUMNS`].
pub type ItemPlacement = [[GridLine; 2]; 2];

const LIMIT: i64 = MAX_GRID_LINE as i64;

/// Names of the explicit grid lines along one axis, one more than its explicit tracks.
pub struct AxisLines {
    pub names: Vec<Vec<Arc<str>>>,
    /// Names derived from template areas; recalculated when inherited by a subgrid.
    pub area_names: Vec<Vec<Arc<str>>>,
    /// Subgridded axes cannot create tracks outside their explicit span.
    pub subgridded: bool,
}
impl AxisLines {
    fn tracks(&self) -> i64 {
        self.names.len().saturating_sub(1) as i64
    }
    fn named(&self, line: i64, name: &str) -> bool {
        usize::try_from(line).ok().is_some_and(|line| {
            self.names
                .get(line)
                .into_iter()
                .chain(self.area_names.get(line))
                .flatten()
                .any(|candidate| &**candidate == name)
        })
    }
    /// The `count`th line named `name` from `from` (exclusive) toward `step`; implicit lines
    /// beyond the explicit grid all have the name (CSS Grid 1 §8.3).
    fn search(&self, name: &str, from: i64, step: i64, count: i64) -> i64 {
        let mut line = from;
        let mut found = 0;
        while (0..=self.tracks()).contains(&(line + step)) {
            line += step;
            if self.named(line, name) {
                found += 1;
                if found == count {
                    return line;
                }
            }
        }
        if step > 0 {
            self.tracks().max(from) + count - found
        } else {
            0.min(from) - (count - found)
        }
    }
    /// A definite line, 0 being explicit line 1; `end` selects `*-end` area lines.
    fn definite(&self, line: &GridLine, end: bool) -> Option<i64> {
        match line {
            GridLine::Name(name) => {
                let area = GridLine::area_line(name, end);
                Some(
                    (0..=self.tracks())
                        .find(|&index| self.named(index, &area))
                        .unwrap_or_else(|| self.search(name, -1, 1, 1)),
                )
            }
            GridLine::Line(index, None) => Some(if *index > 0 {
                i64::from(*index) - 1
            } else {
                self.tracks() + 1 + i64::from(*index)
            }),
            GridLine::Line(index, Some(name)) => Some(if *index > 0 {
                self.search(name, -1, 1, i64::from(*index))
            } else {
                self.search(name, self.tracks() + 1, -1, -i64::from(*index))
            }),
            GridLine::Auto | GridLine::Span(..) => None,
        }
    }
    /// The line `span` reaches from `from` toward `step`.
    fn spanned(&self, span: &GridLine, from: i64, step: i64) -> i64 {
        match span {
            GridLine::Span(count, Some(name)) => self.search(name, from, step, i64::from(*count)),
            GridLine::Span(count, None) => from + step * i64::from(*count),
            _ => from + step,
        }
    }
}

/// One axis of an in-flow item before auto-placement.
#[derive(Clone, Copy, Debug)]
enum Resolved {
    Definite(i64, i64),
    Auto(i64),
}

/// CSS Grid 1 §8.3.1: the placement of an item along one axis, ignoring auto-placement.
fn resolve(lines: &AxisLines, [start, end]: &[GridLine; 2]) -> Resolved {
    let auto_span = |span: &GridLine| match span {
        GridLine::Span(count, None) => i64::from(*count),
        _ => 1,
    };
    match (lines.definite(start, false), lines.definite(end, true)) {
        (Some(start), Some(end)) if start == end => Resolved::Definite(start, start + 1),
        (Some(start), Some(end)) => Resolved::Definite(start.min(end), start.max(end)),
        (Some(start), None) => Resolved::Definite(start, lines.spanned(end, start, 1)),
        (None, Some(end)) => Resolved::Definite(lines.spanned(start, end, -1), end),
        (None, None) => Resolved::Auto(match start {
            GridLine::Span(..) => auto_span(start),
            _ => auto_span(end),
        }),
    }
}

/// Preserve at least one track when an area lies wholly outside its permitted grid.
fn clamp(start: i64, end: i64, bounds: &Range<i64>) -> (i64, i64) {
    if start >= bounds.end {
        (bounds.end - 1, bounds.end)
    } else if end <= bounds.start {
        (bounds.start, bounds.start + 1)
    } else {
        (start.max(bounds.start), end.min(bounds.end))
    }
}

/// Occupied cells: per row of the flow (grow) axis, the occupied ranges across it.
#[derive(Default)]
struct Occupancy {
    rows: Vec<Vec<Range<usize>>>,
}
impl Occupancy {
    fn overlaps(&self, grow: &Range<usize>, across: &Range<usize>) -> bool {
        grow.clone().any(|row| {
            self.rows.get(row).is_some_and(|occupied| {
                occupied
                    .iter()
                    .any(|range| range.start < across.end && across.start < range.end)
            })
        })
    }
    fn occupy(&mut self, grow: &Range<usize>, across: &Range<usize>) {
        if self.rows.len() < grow.end {
            self.rows.resize_with(grow.end, Vec::new);
        }
        for row in grow.clone() {
            self.rows[row].push(across.clone());
        }
    }
}

pub struct Placement {
    /// Tracks of each item, indexed by [`ROWS`] and [`COLUMNS`], from the implicit grid start.
    pub areas: Vec<[Range<usize>; 2]>,
    /// Tracks of the implicit grid per axis.
    pub tracks: [usize; 2],
    /// Implicit tracks before the explicit grid per axis.
    pub leading: [usize; 2],
}

/// The grid area of every item in order-modified document order, and the implicit grid.
pub fn place(
    lines: &[AxisLines; 2],
    items: &[ItemPlacement],
    flow: AutoFlow,
) -> Result<Placement, GridError> {
    let bounds = lines.each_ref().map(|axis| {
        if axis.subgridded {
            0..axis.tracks().min(LIMIT)
        } else {
            -LIMIT..LIMIT
        }
    });
    if bounds.iter().any(Range::is_empty) {
        return Err(GridError::InvalidGeometry);
    }
    let (grow, across) = if flow.column {
        (COLUMNS, ROWS)
    } else {
        (ROWS, COLUMNS)
    };
    let resolved: Vec<[Resolved; 2]> = items
        .iter()
        .map(|item| {
            [
                resolve(&lines[ROWS], &item[ROWS]),
                resolve(&lines[COLUMNS], &item[COLUMNS]),
            ]
        })
        .collect();
    // Implicit tracks before the explicit grid come only from definite lines.
    let mut first = [0i64; 2];
    let mut last = [lines[ROWS].tracks(), lines[COLUMNS].tracks()].map(|line| line.min(LIMIT));
    for item in &resolved {
        for axis in [ROWS, COLUMNS] {
            if let Resolved::Definite(start, end) = item[axis] {
                let (start, end) = clamp(start, end, &bounds[axis]);
                first[axis] = first[axis].min(start);
                last[axis] = last[axis].max(end);
            }
        }
    }
    let offset = first.map(|line| -line);
    let normalized = |axis: usize, start: i64, end: i64| -> Range<usize> {
        let (start, end) = clamp(start, end, &bounds[axis]);
        (start + offset[axis]) as usize..(end + offset[axis]) as usize
    };
    let mut width = (last[across] - first[across]) as usize;
    let mut areas: Vec<[Option<Range<usize>>; 2]> = vec![[None, None]; items.len()];
    let mut occupancy = Occupancy::default();
    // Step 1: fully definite items.
    for (index, item) in resolved.iter().enumerate() {
        if let [Resolved::Definite(rs, re), Resolved::Definite(cs, ce)] = *item {
            let area = [normalized(ROWS, rs, re), normalized(COLUMNS, cs, ce)];
            occupancy.occupy(&area[grow], &area[across]);
            areas[index] = area.map(Some);
        }
    }
    let max_line = |axis: usize| (bounds[axis].end + offset[axis]) as usize;
    let bounded = |axis: usize, range: Range<usize>| {
        normalized(
            axis,
            range.start as i64 - offset[axis],
            range.end as i64 - offset[axis],
        )
    };
    let auto_span = |item: &[Resolved; 2], axis: usize| match item[axis] {
        Resolved::Auto(span) => (span as usize).min(max_line(axis)),
        Resolved::Definite(..) => 0,
    };
    // Step 2: items locked to a track of the flow (grow) axis.
    let mut row_cursors: Vec<(usize, usize)> = Vec::new();
    for (index, item) in resolved.iter().enumerate() {
        let (Resolved::Definite(start, end), Resolved::Auto(_)) = (item[grow], item[across]) else {
            continue;
        };
        let rows = normalized(grow, start, end);
        let span = auto_span(item, across);
        let mut column = if flow.dense {
            0
        } else {
            row_cursors
                .iter()
                .find(|(row, _)| *row == rows.start)
                .map_or(0, |(_, column)| *column)
        };
        while occupancy.overlaps(&rows, &(column..column + span)) && column < max_line(across) {
            column += 1;
        }
        let columns = bounded(across, column..column + span);
        width = width.max(columns.end);
        match row_cursors.iter_mut().find(|(row, _)| *row == rows.start) {
            Some(cursor) => cursor.1 = columns.end,
            None => row_cursors.push((rows.start, columns.end)),
        }
        occupancy.occupy(&rows, &columns);
        let mut area = [None, None];
        area[grow] = Some(rows);
        area[across] = Some(columns);
        areas[index] = area;
    }
    // Step 3: the implicit grid is wide enough for the widest auto-placed span.
    for item in &resolved {
        if let (Resolved::Auto(_), Resolved::Auto(_)) = (item[grow], item[across]) {
            width = width.max(auto_span(item, across));
        }
    }
    width = width.min(max_line(across));
    // Step 4: the remaining items, with the auto-placement cursor.
    let mut cursor = (0usize, 0usize);
    for (index, item) in resolved.iter().enumerate() {
        if areas[index][grow].is_some() {
            continue;
        }
        let rows_span = auto_span(item, grow);
        let (rows, columns) = match item[across] {
            Resolved::Definite(start, end) => {
                let columns = normalized(across, start, end);
                let mut row = if flow.dense {
                    0
                } else {
                    if columns.start < cursor.1 {
                        cursor.0 += 1;
                    }
                    cursor.0
                };
                while occupancy.overlaps(&(row..row + rows_span), &columns) && row < max_line(grow)
                {
                    row += 1;
                }
                cursor = (row, columns.start);
                (bounded(grow, row..row + rows_span), columns)
            }
            Resolved::Auto(_) => {
                let span = auto_span(item, across);
                let (mut row, mut column) = if flow.dense { (0, 0) } else { cursor };
                loop {
                    if column + span > width {
                        row += 1;
                        column = 0;
                        continue;
                    }
                    if !occupancy.overlaps(&(row..row + rows_span), &(column..column + span))
                        || row >= max_line(grow)
                    {
                        break;
                    }
                    column += 1;
                }
                cursor = (row, column);
                (
                    bounded(grow, row..row + rows_span),
                    bounded(across, column..column + span),
                )
            }
        };
        occupancy.occupy(&rows, &columns);
        let mut area = [None, None];
        area[grow] = Some(rows);
        area[across] = Some(columns);
        areas[index] = area;
    }
    let areas: Vec<[Range<usize>; 2]> = areas
        .into_iter()
        .map(|[rows, columns]| [rows.unwrap_or_default(), columns.unwrap_or_default()])
        .collect();
    let mut tracks = [
        (last[ROWS] - first[ROWS]) as usize,
        (last[COLUMNS] - first[COLUMNS]) as usize,
    ];
    tracks[across] = tracks[across].max(width);
    for area in &areas {
        for axis in [ROWS, COLUMNS] {
            tracks[axis] = tracks[axis].max(area[axis].end);
        }
    }
    if areas
        .iter()
        .any(|area| area[ROWS].is_empty() || area[COLUMNS].is_empty())
    {
        return Err(GridError::InvalidGeometry);
    }
    Ok(Placement {
        areas,
        tracks,
        leading: offset.map(|value| value as usize),
    })
}

impl Placement {
    /// CSS Grid 1 §9.1: lines of an absolutely positioned child in implicit-grid indices;
    /// `None` is the padding edge, which `auto`, lone spans and lines outside the implicit
    /// grid contribute.
    pub fn absolute(
        &self,
        lines: &[AxisLines; 2],
        item: &ItemPlacement,
    ) -> [[Option<usize>; 2]; 2] {
        [ROWS, COLUMNS].map(|axis| {
            let [start, end] = &item[axis];
            let axis_lines = &lines[axis];
            let exists = |line: &i64| {
                (0..=self.tracks[axis] as i64).contains(&(line + self.leading[axis] as i64))
            };
            let (start, end) = match (
                axis_lines.definite(start, false).filter(exists),
                axis_lines.definite(end, true).filter(exists),
            ) {
                (Some(start), Some(end)) if start == end => (Some(start), None),
                (Some(start), Some(end)) => (Some(start.min(end)), Some(start.max(end))),
                (Some(start), None) if matches!(end, GridLine::Span(..)) => {
                    (Some(start), Some(axis_lines.spanned(end, start, 1)))
                }
                (None, Some(end)) if matches!(start, GridLine::Span(..)) => {
                    (Some(axis_lines.spanned(start, end, -1)), Some(end))
                }
                (start, end) => (start, end),
            };
            let line = |line: Option<i64>| {
                line.filter(exists)
                    .map(|line| (line + self.leading[axis] as i64) as usize)
            };
            [line(start), line(end)]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(tracks: usize) -> AxisLines {
        AxisLines {
            names: vec![Vec::new(); tracks + 1],
            subgridded: false,
            area_names: Vec::new(),
        }
    }
    fn auto() -> ItemPlacement {
        Default::default()
    }

    #[test]
    fn subgrid_placement_clamps_definite_lines_and_auto_spans_without_implicit_tracks() {
        for column in [false, true] {
            for dense in [false, true] {
                let lines = [axis(2), axis(2)].map(|mut axis| {
                    axis.subgridded = true;
                    axis
                });
                let outside = [GridLine::Line(8, None), GridLine::Span(3, None)];
                let before = [GridLine::Line(-8, None), GridLine::Span(3, None)];
                let oversized = [GridLine::Auto, GridLine::Span(8, None)];
                let items = [
                    [outside.clone(), outside],
                    [before.clone(), before],
                    [oversized.clone(), oversized],
                    auto(),
                ];
                let placed = place(&lines, &items, AutoFlow { column, dense }).unwrap();
                assert_eq!(
                    placed.tracks,
                    [2, 2],
                    "subgrid cannot add tracks: column={column}, dense={dense}"
                );
                assert_eq!(placed.leading, [0, 0]);
                assert_eq!(placed.areas[0], [1..2, 1..2]);
                assert_eq!(placed.areas[1], [0..1, 0..1]);
                for area in &placed.areas {
                    assert!(
                        area.iter().all(|range| !range.is_empty() && range.end <= 2),
                        "unclamped area: {area:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn subgrid_keeps_only_the_shared_axis_bounded() {
        for column in [false, true] {
            let shared = if column { ROWS } else { COLUMNS };
            let grow = 1 - shared;
            let mut lines = [axis(2), axis(2)];
            lines[shared].subgridded = true;
            for dense in [false, true] {
                let placed = place(&lines, &vec![auto(); 7], AutoFlow { column, dense }).unwrap();
                assert_eq!(placed.tracks[shared], 2);
                assert_eq!(placed.tracks[grow], 4);
                for (index, area) in placed.areas.iter().enumerate() {
                    assert_eq!(area[shared], index % 2..index % 2 + 1);
                    assert_eq!(area[grow], index / 2..index / 2 + 1);
                }
            }
        }
    }

    #[test]
    fn subgrid_resolves_hypothetical_named_lines_then_clamps_to_its_only_track() {
        let mut lines = [axis(1), axis(1)];
        lines[COLUMNS].subgridded = true;
        let name: Arc<str> = "outside".into();
        let items = [
            columns(GridLine::Name(name.clone()), GridLine::Auto),
            columns(GridLine::Line(-3, Some(name.clone())), GridLine::Auto),
            columns(GridLine::Line(1, None), GridLine::Span(4, Some(name))),
        ];
        let placed = place(&lines, &items, AutoFlow::default()).unwrap();
        assert_eq!(placed.tracks, [3, 1]);
        assert_eq!(placed.leading, [0, 0]);
        for (row, area) in placed.areas.iter().enumerate() {
            assert_eq!(area, &[row..row + 1, 0..1]);
        }
    }

    #[test]
    fn an_empty_subgridded_axis_is_invalid_even_without_children() {
        for shared in [ROWS, COLUMNS] {
            let mut lines = [axis(0), axis(0)];
            lines[shared].subgridded = true;
            assert!(matches!(
                place(&lines, &[], AutoFlow::default()),
                Err(GridError::InvalidGeometry)
            ));
        }
    }

    #[test]
    fn locked_subgrid_items_overlap_the_last_track_when_the_axis_is_full() {
        for column in [false, true] {
            for dense in [false, true] {
                let grow = if column { COLUMNS } else { ROWS };
                let across = 1 - grow;
                let mut lines = [axis(2), axis(2)];
                lines[across].subgridded = true;
                let mut item = auto();
                item[grow] = [GridLine::Line(1, None), GridLine::Auto];
                let placed = place(
                    &lines,
                    &[item.clone(), item.clone(), item],
                    AutoFlow { column, dense },
                )
                .unwrap();
                assert_eq!(placed.tracks, [2, 2]);
                for (index, area) in placed.areas.iter().enumerate() {
                    let start = index.min(1);
                    assert_eq!(area[grow], 0..1);
                    assert_eq!(area[across], start..start + 1);
                }
            }
        }
    }
    fn columns(start: GridLine, end: GridLine) -> ItemPlacement {
        [[GridLine::Auto, GridLine::Auto], [start, end]]
    }

    #[test]
    fn dense_packing_backfills_holes_that_sparse_packing_leaves() {
        let wide = columns(GridLine::Span(2, None), GridLine::Auto);
        let items = [wide.clone(), wide, auto(), auto()];
        let lines = [axis(0), axis(3)];
        let sparse = place(&lines, &items, AutoFlow::default()).unwrap();
        let dense = place(
            &lines,
            &items,
            AutoFlow {
                column: false,
                dense: true,
            },
        )
        .unwrap();
        assert_eq!(
            sparse.areas,
            vec![[0..1, 0..2], [1..2, 0..2], [1..2, 2..3], [2..3, 0..1]],
            "sparse placement never moves the cursor back before the wide second item"
        );
        assert_eq!(
            dense.areas,
            vec![[0..1, 0..2], [1..2, 0..2], [0..1, 2..3], [1..2, 2..3]],
            "dense placement fills the first-row hole beside the first wide item"
        );
        assert_eq!(sparse.tracks, [3, 3]);
        assert_eq!(dense.tracks, [2, 3]);
    }

    #[test]
    fn named_areas_and_lines_resolve_with_implicit_tracks_on_both_sides() {
        let named = |names: &[&[&str]]| AxisLines {
            subgridded: false,
            area_names: Vec::new(),
            names: names
                .iter()
                .map(|line| line.iter().map(|name| Arc::from(*name)).collect())
                .collect(),
        };
        let rows = named(&[&["main-start"], &["main-end"]]);
        let cols = named(&[&["a"], &["b", "main-start"], &["a"], &["main-end"]]);
        let name = |name: &str| GridLine::Name(Arc::from(name));
        let items = [
            [[name("main"), name("main")], [name("main"), name("main")]],
            columns(GridLine::Line(2, Some(Arc::from("a"))), GridLine::Auto),
            columns(GridLine::Line(-5, None), GridLine::Auto),
            columns(name("missing"), GridLine::Auto),
            columns(
                GridLine::Span(3, Some(Arc::from("a"))),
                GridLine::Line(4, None),
            ),
        ];
        let placement = place(&[rows, cols], &items, AutoFlow::default()).unwrap();
        assert_eq!(
            placement.leading,
            [0, 1],
            "line -5 of three explicit columns adds one implicit column before them"
        );
        assert_eq!(
            placement.areas[0],
            [0..1, 2..4],
            "`main` uses the main-start and main-end lines in both axes"
        );
        assert_eq!(
            placement.areas[1][COLUMNS],
            3..4,
            "the second `a` line is explicit line 3"
        );
        assert_eq!(placement.areas[2][COLUMNS], 0..1);
        assert_eq!(
            placement.areas[3][COLUMNS],
            5..6,
            "a missing name is the first implicit line after the explicit grid"
        );
        assert_eq!(
            placement.areas[4][COLUMNS],
            0..4,
            "a named span counts implicit lines before the grid once explicit ones run out"
        );
        assert_eq!(
            placement.tracks,
            [4, 6],
            "column-locked items after the `main` area each move the cursor to a new row"
        );
    }

    #[test]
    fn column_flow_places_row_locked_items_after_the_cursor() {
        let row = |line| {
            [
                [GridLine::Line(line, None), GridLine::Auto],
                [GridLine::Auto, GridLine::Auto],
            ]
        };
        let items = [auto(), auto(), auto(), row(1), auto()];
        let placement = place(
            &[axis(2), axis(0)],
            &items,
            AutoFlow {
                column: true,
                dense: false,
            },
        )
        .unwrap();
        assert_eq!(
            placement.areas,
            vec![
                [0..1, 0..1],
                [1..2, 0..1],
                [0..1, 1..2],
                [0..1, 2..3],
                [1..2, 2..3]
            ],
            "the row-1 item skips the column whose first row the third item occupies"
        );
    }

    #[test]
    fn locked_axis_auto_placement_clamps_at_the_grid_limit() {
        for column in [false, true] {
            for dense in [false, true] {
                let (grow, across) = if column {
                    (COLUMNS, ROWS)
                } else {
                    (ROWS, COLUMNS)
                };
                let mut item = auto();
                item[grow] = [GridLine::Line(1, None), GridLine::Auto];
                item[across] = [GridLine::Span(MAX_GRID_LINE as u32, None), GridLine::Auto];
                let placement = place(
                    &[axis(0), axis(0)],
                    &[item.clone(), item],
                    AutoFlow { column, dense },
                )
                .unwrap();
                let limit = MAX_GRID_LINE as usize;
                assert_eq!(
                    placement.tracks[across], limit,
                    "column={column}, dense={dense}: automatic tracks must respect the configured limit"
                );
                assert_eq!(placement.areas[0][across], 0..limit);
                assert_eq!(
                    placement.areas[1][across],
                    limit - 1..limit,
                    "column={column}, dense={dense}: a fully occupied axis puts the next area outside the limit, so it moves into the final track"
                );
            }
        }
    }

    #[test]
    fn automatic_spans_cannot_allocate_beyond_the_grid_limit() {
        for column in [false, true] {
            for dense in [false, true] {
                let grow = if column { COLUMNS } else { ROWS };
                let mut item = auto();
                item[grow] = [
                    GridLine::Span(MAX_GRID_LINE as u32 + 1, None),
                    GridLine::Auto,
                ];
                let placement = place(
                    &[axis(0), axis(0)],
                    &[item.clone(), item],
                    AutoFlow { column, dense },
                )
                .unwrap();
                let limit = MAX_GRID_LINE as usize;
                assert_eq!(
                    placement.tracks[grow], limit,
                    "column={column}, dense={dense}: growing automatic tracks must remain bounded"
                );
                assert!(
                    placement
                        .areas
                        .iter()
                        .all(|area| !area[grow].is_empty() && area[grow].end <= limit),
                    "column={column}, dense={dense}: areas {:?}",
                    placement.areas
                );
            }
        }
    }

    #[test]
    fn auto_placement_clamps_crossing_spans_in_implicit_grid_coordinates() {
        for column in [false, true] {
            for dense in [false, true] {
                let (grow, across) = if column {
                    (COLUMNS, ROWS)
                } else {
                    (ROWS, COLUMNS)
                };
                let limit = MAX_GRID_LINE as usize;
                let mut prefix = auto();
                prefix[grow] = [GridLine::Line(1, None), GridLine::Auto];
                prefix[across] = [
                    GridLine::Span(MAX_GRID_LINE as u32, None),
                    GridLine::Line(1, None),
                ];
                let mut item = auto();
                item[grow] = prefix[grow].clone();
                item[across] = [
                    GridLine::Span(2 * MAX_GRID_LINE as u32, None),
                    GridLine::Auto,
                ];
                let placement = place(
                    &[axis(0), axis(0)],
                    &[prefix, item],
                    AutoFlow { column, dense },
                )
                .unwrap();
                assert_eq!(
                    placement.leading[across], limit,
                    "column={column}, dense={dense}: retain negative implicit tracks"
                );
                assert_eq!(placement.areas[0][across], 0..limit);
                assert_eq!(
                    placement.areas[1][across],
                    limit..2 * limit,
                    "column={column}, dense={dense}: a crossing span keeps its start and truncates only its end"
                );
                assert_eq!(placement.tracks[across], 2 * limit);
            }
        }
    }

    #[test]
    fn explicit_track_counts_cannot_enlarge_the_limited_grid() {
        let limit = MAX_GRID_LINE as usize;
        let placement = place(
            &[axis(limit + 1), axis(limit + 1)],
            &[auto()],
            AutoFlow::default(),
        )
        .unwrap();
        assert_eq!(
            placement.tracks,
            [limit, limit],
            "lines outside the configured grid limit must not allocate explicit tracks"
        );
        assert_eq!(placement.areas, [[0..1, 0..1]]);
    }

    #[test]
    fn equal_absolute_lines_remove_the_end_without_growing_the_grid() {
        let mut lines = [axis(2), axis(2)];
        for axis in &mut lines {
            axis.names[1].push("middle".into());
        }
        let placement = place(&lines, &[], AutoFlow::default()).unwrap();
        for pair in [
            [GridLine::Line(2, None), GridLine::Line(2, None)],
            [GridLine::Line(2, None), GridLine::Line(-2, None)],
            [
                GridLine::Name("middle".into()),
                GridLine::Name("middle".into()),
            ],
        ] {
            let item = [pair.clone(), pair.clone()];
            assert_eq!(
                placement.absolute(&lines, &item),
                [[Some(1), None]; 2],
                "equal resolved lines {pair:?}: end must use the padding edge"
            );
            let in_flow = place(&lines, &[item], AutoFlow::default()).unwrap();
            assert_eq!(
                in_flow.areas,
                [[1..2, 1..2]],
                "in-flow auto end still spans one track"
            );
        }
        assert_eq!(
            placement.tracks,
            [2, 2],
            "absolute placement cannot add implicit tracks"
        );
    }

    #[test]
    fn absolute_spans_cannot_resolve_from_nonexistent_anchor_lines() {
        let lines = [axis(2), axis(2)];
        let placement = place(&lines, &[], AutoFlow::default()).unwrap();
        for pair in [
            [GridLine::Line(-4, None), GridLine::Span(2, None)],
            [GridLine::Span(2, None), GridLine::Line(4, None)],
            [GridLine::Span(2, None), GridLine::Name("missing".into())],
        ] {
            assert_eq!(
                placement.absolute(&lines, &[pair.clone(), pair.clone()]),
                [[None, None]; 2],
                "nonexistent anchor must become auto before resolving span: {pair:?}"
            );
        }
        let grown = place(
            &lines,
            &[std::array::from_fn(|_| {
                [GridLine::Line(-4, None), GridLine::Line(4, None)]
            })],
            AutoFlow::default(),
        )
        .unwrap();
        assert_eq!(
            grown.absolute(
                &lines,
                &std::array::from_fn(|_| [GridLine::Line(-4, None), GridLine::Span(2, None)])
            ),
            [[Some(0), Some(2)]; 2],
            "an anchor in existing implicit tracks must remain usable"
        );
    }

    #[test]
    fn absolute_lines_use_padding_edges_outside_the_implicit_grid() {
        let placement = place(&[axis(2), axis(2)], &[auto()], AutoFlow::default()).unwrap();
        let item = [
            [GridLine::Line(2, None), GridLine::Auto],
            [GridLine::Line(5, None), GridLine::Span(1, None)],
        ];
        assert_eq!(
            placement.absolute(&[axis(2), axis(2)], &item),
            [[Some(1), None], [None, None]],
            "auto ends are padding edges and lines beyond the implicit grid become auto"
        );
    }
}
