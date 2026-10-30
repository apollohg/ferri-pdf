use super::{Distribution, Plan, Run, Slice};
use crate::layout::multicol::WrapRows;
use crate::style::ColumnFill;

const LINE: f32 = 10.0;

/// Line bottoms of content `lines` lines tall, ending at its end.
fn lines(lines: usize) -> Vec<f32> {
    (1..=lines).map(|line| line as f32 * LINE).collect()
}

fn run(breaks: &[f32]) -> Run<'_> {
    Run {
        top: 0.0,
        bottom: *breaks.last().expect("content has a line"),
        breaks,
        cloned: [0.0; 2],
        repeated: [0.0; 2],
    }
}

fn plan(count: usize) -> Plan {
    Plan {
        count,
        height: None,
        definite: false,
        fill: ColumnFill::Balance,
        wrap: None,
    }
}

/// Each column's `[row, index]` and its slices as `(run, top, bottom)`.
type Layout = Vec<([usize; 2], Vec<(usize, f32, f32)>)>;

fn layout(distribution: &Distribution) -> Layout {
    distribution
        .columns
        .iter()
        .map(|column| {
            (
                [column.row, column.index],
                column
                    .slices
                    .iter()
                    .map(|slice| (slice.run, slice.top, slice.bottom))
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn balancing_takes_the_least_height_that_holds_the_content_in_the_count() {
    // Seven lines and a three-line run in three columns: ten lines need columns at least four
    // lines tall, and a column three lines tall would leave a line over.
    let [first, second] = [lines(7), lines(3)];
    let distribution = Distribution::of(&[run(&first), run(&second)], plan(3));
    assert_eq!(
        layout(&distribution),
        [
            ([0, 0], vec![(0, 0.0, 40.0)]),
            ([0, 1], vec![(0, 40.0, 70.0), (1, 0.0, 10.0)]),
            ([0, 2], vec![(1, 10.0, 30.0)]),
        ],
        "{distribution:?}"
    );
    assert_eq!(distribution.rows, [40.0], "{distribution:?}");
    assert!(!distribution.overflows);
}

#[test]
fn one_column_stacks_every_run_whole() {
    let [first, second] = [lines(7), lines(3)];
    let distribution = Distribution::of(&[run(&first), Run::whole(25.0), run(&second)], plan(1));
    assert_eq!(
        layout(&distribution),
        [([0, 0], vec![(0, 0.0, 70.0), (1, 0.0, 25.0), (2, 0.0, 30.0)])],
        "{distribution:?}"
    );
    assert_eq!(distribution.height(), 125.0);
}

#[test]
fn cloned_edges_wrap_each_slice_that_a_break_ends_or_starts() {
    // CSS Fragmentation 3 §5.4: a 4pt start edge and 6pt end edge repeat at each break. Six
    // lines with both edges take 70pt; two balanced columns are 40pt tall: the first holds
    // the start edge, three lines and the repeated end edge.
    let breaks = [14.0, 24.0, 34.0, 44.0, 54.0, 64.0];
    let content = Run {
        top: 0.0,
        bottom: 70.0,
        breaks: &breaks,
        cloned: [4.0, 6.0],
        repeated: [0.0; 2],
    };
    let distribution = Distribution::of(&[content], plan(2));
    let slices: Vec<Slice> = distribution
        .columns
        .iter()
        .flat_map(|column| column.slices.clone())
        .collect();
    assert_eq!(
        slices,
        [
            Slice {
                run: 0,
                top: 0.0,
                bottom: 34.0,
                repeated: [0.0, 6.0],
                y: 0.0,
            },
            Slice {
                run: 0,
                top: 34.0,
                bottom: 70.0,
                repeated: [4.0, 0.0],
                y: 0.0,
            },
        ],
        "{distribution:?}"
    );
    assert_eq!(distribution.rows, [40.0], "{distribution:?}");
}

#[test]
fn auto_fill_fills_columns_sequentially_and_reports_columns_past_the_count() {
    let content = lines(9);
    let sequential = Distribution::of(
        &[run(&content)],
        Plan {
            height: Some(40.0),
            fill: ColumnFill::Auto,
            ..plan(2)
        },
    );
    assert_eq!(
        layout(&sequential),
        [
            ([0, 0], vec![(0, 0.0, 40.0)]),
            ([0, 1], vec![(0, 40.0, 80.0)]),
            ([0, 2], vec![(0, 80.0, 90.0)]),
        ],
        "the ninth line overflows into a third column: {sequential:?}"
    );
    assert!(sequential.overflows);
    assert_eq!(sequential.rows, [40.0], "the fullest column's extent");
    let balanced = Distribution::of(
        &[run(&content)],
        Plan {
            height: Some(60.0),
            definite: true,
            ..plan(2)
        },
    );
    assert_eq!(
        layout(&balanced),
        [
            ([0, 0], vec![(0, 0.0, 50.0)]),
            ([0, 1], vec![(0, 50.0, 90.0)]),
        ],
        "balanced inside a definite height: {balanced:?}"
    );
    assert_eq!(balanced.rows, [60.0], "the definite height is the row's");
    assert!(!balanced.overflows);
}

#[test]
fn wrapped_rows_fill_sequentially_and_balance_the_last() {
    // CSS Multi-column Layout 2 §4.4: twelve lines in rows of two 40pt columns with a 5pt
    // gap: the first row holds eight lines, the last balances the remaining four.
    let content = lines(12);
    let distribution = Distribution::of(
        &[run(&content)],
        Plan {
            wrap: Some(WrapRows {
                height: 40.0,
                gap: 5.0,
            }),
            ..plan(2)
        },
    );
    assert_eq!(
        layout(&distribution),
        [
            ([0, 0], vec![(0, 0.0, 40.0)]),
            ([0, 1], vec![(0, 40.0, 80.0)]),
            ([1, 0], vec![(0, 80.0, 100.0)]),
            ([1, 1], vec![(0, 100.0, 120.0)]),
        ],
        "{distribution:?}"
    );
    assert_eq!(distribution.height(), 85.0);
    assert_eq!(distribution.row_offset(1), 45.0);
}

#[test]
fn content_no_column_can_hold_overflows_its_column_whole() {
    // A 50pt run without breaks in 30pt columns takes a column of its own.
    let distribution = Distribution::of(
        &[Run::whole(50.0)],
        Plan {
            height: Some(30.0),
            fill: ColumnFill::Auto,
            ..plan(2)
        },
    );
    assert_eq!(
        layout(&distribution),
        [([0, 0], vec![(0, 0.0, 50.0)])],
        "{distribution:?}"
    );
    assert!(distribution.overflows);
}
