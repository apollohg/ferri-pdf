//! Content laid out once at the column width and sliced into column boxes at its break
//! offsets: the bodies a footnote area holds and the content of a page-margin box, which no
//! flow fragments (CSS Multi-column Layout 1 §7, CSS Fragmentation 3 §5.4).
use super::{ColumnGeometry, WrapRows};
use crate::graphics::{Border, BoxPaint};
use crate::layout::pagination::{FRAGMENTATION_TOLERANCE_PT, overflows};
use crate::style::ColumnFill;

#[cfg(test)]
mod tests;

/// Laid-out content that a column box takes whole or breaks inside.
#[derive(Clone, Copy, Debug)]
pub(in crate::layout) struct Run<'b> {
    /// The offsets in the content that the run spans.
    pub top: f32,
    pub bottom: f32,
    /// The offsets in the content where a column break may fall, ascending.
    pub breaks: &'b [f32],
    /// The block-start and block-end edges of a box that clones its decoration at breaks,
    /// which each slice starting or ending at a break repeats.
    pub cloned: [f32; 2],
    /// The repeated edges the run starts and ends with.
    pub repeated: [f32; 2],
}
impl Run<'_> {
    /// Content `height` tall that no column break falls inside.
    pub(in crate::layout) fn whole(height: f32) -> Self {
        Self {
            top: 0.0,
            bottom: height,
            breaks: &[],
            cloned: [0.0; 2],
            repeated: [0.0; 2],
        }
    }
    fn height(&self) -> f32 {
        self.bottom - self.top + self.repeated[0] + self.repeated[1]
    }
}

/// The part `[top, bottom)` of run `run` that one column box holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::layout) struct Slice {
    pub run: usize,
    pub top: f32,
    pub bottom: f32,
    pub repeated: [f32; 2],
    /// The offset of the slice from the top of its column box.
    pub y: f32,
}
impl Slice {
    pub(in crate::layout) fn height(&self) -> f32 {
        self.bottom - self.top + self.repeated[0] + self.repeated[1]
    }
}

/// A column box and the slices it holds.
#[derive(Clone, Debug, Default)]
pub(in crate::layout) struct ColumnBox {
    /// The row of columns it lies in, and its index in the row; columns past the column count
    /// overflow in the inline direction (§8.2).
    pub row: usize,
    pub index: usize,
    pub slices: Vec<Slice>,
    /// The block size its slices take.
    pub extent: f32,
}

/// How content fills its column boxes.
#[derive(Clone, Copy, Debug)]
pub(in crate::layout) struct Plan {
    pub count: usize,
    /// A fixed column height: `column-height`, the container's definite height, or the most
    /// that `column-fill: auto` fills.
    pub height: Option<f32>,
    /// The fixed height is the row's own rather than only the most its columns take.
    pub definite: bool,
    pub fill: ColumnFill,
    pub wrap: Option<WrapRows>,
}

/// Content laid out in rows of column boxes.
#[derive(Clone, Debug)]
pub(in crate::layout) struct Distribution {
    pub columns: Vec<ColumnBox>,
    /// The block size of each row of columns.
    pub rows: Vec<f32>,
    pub row_gap: f32,
    /// The content needs columns past the count, or a slice is taller than its column.
    pub overflows: bool,
}
impl Distribution {
    /// Lays `runs` out in column boxes as `plan` fills them (§7.1): sequentially where the
    /// column height is fixed and `column-fill` is `auto`, else balanced in the last row.
    pub(in crate::layout) fn of(runs: &[Run<'_>], plan: Plan) -> Self {
        let mut distribution = Self {
            columns: Vec::new(),
            rows: Vec::new(),
            row_gap: plan.wrap.map_or(0.0, |wrap| wrap.gap),
            overflows: false,
        };
        let Some(start) = Cursor::start(runs) else {
            return distribution;
        };
        let balanced = plan.fill != ColumnFill::Auto;
        if let Some(wrap) = plan.wrap {
            let mut next = Some(start);
            while let Some(cursor) = next {
                let row = fill(runs, cursor, wrap.height, Some(plan.count));
                next = row.end;
                let row = if next.is_none() && balanced {
                    balance(runs, cursor, plan.count, Some(wrap.height))
                } else {
                    row
                };
                distribution.push_row(row, wrap.height);
            }
            return distribution;
        }
        let Some(height) = plan.height else {
            let row = balance(runs, start, plan.count, None);
            let extent = row.extent();
            distribution.push_row(row, extent);
            return distribution;
        };
        let sequential = fill(runs, start, height, None);
        let fits = sequential.columns.len() <= plan.count && !sequential.forced;
        let row = if fits && balanced {
            balance(runs, start, plan.count, Some(height))
        } else {
            sequential
        };
        let extent = if plan.definite { height } else { row.extent() };
        distribution.push_row(row, extent);
        distribution.overflows |= !fits;
        distribution
    }
    fn push_row(&mut self, filled: Filled, height: f32) {
        let row = self.rows.len();
        self.overflows |= filled.forced;
        self.columns.extend(
            filled
                .columns
                .into_iter()
                .map(|column| ColumnBox { row, ..column }),
        );
        self.rows.push(height);
    }
    /// The block size of the rows with the gaps between them.
    pub(in crate::layout) fn height(&self) -> f32 {
        self.rows.iter().sum::<f32>() + self.row_gap * self.rows.len().saturating_sub(1) as f32
    }
    /// The offset of row `row` from the top of the first.
    pub(in crate::layout) fn row_offset(&self, row: usize) -> f32 {
        self.rows[..row].iter().sum::<f32>() + self.row_gap * row as f32
    }
    /// The rules of `geometry`'s gaps between adjacent columns of a row that both hold
    /// content (§4), as tall as the row, from `origin`, the top left of the content box.
    pub(in crate::layout) fn rules(
        &self,
        geometry: &ColumnGeometry,
        rule: Border,
        origin: [f32; 2],
    ) -> Vec<BoxPaint> {
        if rule.used_width() <= 0.0 {
            return Vec::new();
        }
        self.columns
            .windows(2)
            .filter(|pair| {
                pair[0].row == pair[1].row && pair.iter().all(|column| !column.slices.is_empty())
            })
            .map(|pair| {
                let row = pair[0].row;
                super::rule_paint(
                    rule,
                    origin[0] + geometry.rule_x(pair[0].index),
                    origin[1] + self.row_offset(row),
                    self.rows[row],
                )
            })
            .collect()
    }
}

/// How far content is laid out: the run, the offset in it and the edge its next slice
/// repeats at its start.
#[derive(Clone, Copy, Debug)]
struct Cursor {
    run: usize,
    top: f32,
    repeated: f32,
}
impl Cursor {
    fn start(runs: &[Run<'_>]) -> Option<Self> {
        runs.first().map(|run| Self {
            run: 0,
            top: run.top,
            repeated: run.repeated[0],
        })
    }
}

/// Column boxes filled from a cursor, where the content they do not hold resumes, and whether
/// a column holds a slice taller than itself.
struct Filled {
    columns: Vec<ColumnBox>,
    end: Option<Cursor>,
    forced: bool,
}
impl Filled {
    fn extent(&self) -> f32 {
        self.columns
            .iter()
            .map(|column| column.extent)
            .fold(0.0, f32::max)
    }
}

/// Fills column boxes `height` tall in order from `cursor`, at most `limit` of them; a column
/// that no slice fits takes the next one whole, so that filling advances.
fn fill(runs: &[Run<'_>], mut cursor: Cursor, height: f32, limit: Option<usize>) -> Filled {
    let mut columns = vec![ColumnBox::default()];
    let mut forced = false;
    loop {
        let run = &runs[cursor.run];
        let column = columns.last_mut().expect("a column is open");
        let room = height - column.extent;
        let whole = run.bottom - cursor.top + cursor.repeated + run.repeated[1];
        let candidates = &run.breaks[run.breaks.partition_point(|&offset| offset <= cursor.top)
            ..run.breaks.partition_point(|&offset| offset < run.bottom)];
        let slice = if overflows(whole, room) {
            let fitting = candidates.partition_point(|&offset| {
                !overflows(offset - cursor.top + cursor.repeated + run.cloned[1], room)
            });
            if fitting > 0 {
                Some((candidates[fitting - 1], run.cloned[1]))
            } else if column.slices.is_empty() {
                forced = true;
                Some(
                    candidates
                        .first()
                        .map_or((run.bottom, run.repeated[1]), |&offset| {
                            (offset, run.cloned[1])
                        }),
                )
            } else {
                None
            }
        } else {
            Some((run.bottom, run.repeated[1]))
        };
        if let Some((bottom, end)) = slice {
            let slice = Slice {
                run: cursor.run,
                top: cursor.top,
                bottom,
                repeated: [cursor.repeated, end],
                y: column.extent,
            };
            column.extent += slice.height();
            column.slices.push(slice);
            if bottom >= run.bottom {
                let Some(next) = runs.get(cursor.run + 1) else {
                    return Filled {
                        columns,
                        end: None,
                        forced,
                    };
                };
                cursor = Cursor {
                    run: cursor.run + 1,
                    top: next.top,
                    repeated: next.repeated[0],
                };
                continue;
            }
            cursor.top = bottom;
            cursor.repeated = run.cloned[0];
        }
        if limit.is_some_and(|limit| columns.len() >= limit) {
            return Filled {
                columns,
                end: Some(cursor),
                forced,
            };
        }
        columns.push(ColumnBox {
            index: columns.len(),
            ..ColumnBox::default()
        });
    }
}

/// Fills `count` column boxes from `cursor` to the content's end at the least height, at most
/// `cap`, that holds it (§7.1), or sequentially at that most when none does.
fn balance(runs: &[Run<'_>], cursor: Cursor, count: usize, cap: Option<f32>) -> Filled {
    let current = &runs[cursor.run];
    let total = current.bottom - cursor.top
        + cursor.repeated
        + current.repeated[1]
        + runs[cursor.run + 1..].iter().map(Run::height).sum::<f32>();
    let mut high = cap.map_or(total, |cap| cap.min(total));
    let holds = |height: f32| {
        let filled = fill(runs, cursor, height, Some(count));
        (filled.end.is_none() && !filled.forced).then_some(filled)
    };
    let Some(mut best) = holds(high) else {
        return fill(runs, cursor, high, Some(count));
    };
    if count == 1 {
        return best;
    }
    let mut low = 0.0;
    while high - low > FRAGMENTATION_TOLERANCE_PT {
        let middle = low + (high - low) / 2.0;
        if middle == low || middle == high {
            break;
        }
        match holds(middle) {
            Some(filled) => {
                high = middle;
                best = filled;
            }
            None => low = middle,
        }
    }
    best
}
