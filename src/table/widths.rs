use crate::style::{BoxSizing, LengthPercentage, Size, SizeConstraints, TableLayout};
use crate::table::{Grid, TableError, finite, grow};

#[derive(Clone, Debug)]
pub(crate) struct CellWidth {
    pub min: f32,
    pub max: f32,
    pub specified: Size,
    pub minimum: Size,
    pub maximum: Size,
    pub box_sizing: BoxSizing,
    pub edges: f32,
}

impl CellWidth {
    fn outer(&self, width: f32) -> f32 {
        if self.box_sizing == BoxSizing::BorderBox {
            width
        } else {
            width + self.edges
        }
    }
    fn contributions(&self) -> (f32, f32) {
        let zero = Size::ZERO;
        let minimum = if self.minimum.is_zero() {
            &zero
        } else {
            &self.minimum
        };
        let maximum = if self.maximum.is_zero() {
            &zero
        } else {
            &self.maximum
        };
        let limits = SizeConstraints::new(
            [&Size::Auto, minimum, maximum],
            None,
            BoxSizing::ContentBox,
            0.0,
        );
        let natural = crate::style::IntrinsicWidths {
            min: (self.min - self.edges).max(0.0),
            max: (self.max - self.edges).max(0.0),
        };
        let limits = limits.with_intrinsic([&Size::Auto, minimum, maximum], natural, None);
        let constrain = |outer: f32| limits.constrain((outer - self.edges).max(0.0)) + self.edges;
        let min = constrain(self.min);
        let preferred = if let Size::Intrinsic(value) = self.specified {
            value.resolve(natural, None) + self.edges
        } else {
            match track_size(&self.specified) {
                Some(LengthPercentage::Length(width)) => self.outer(*width),
                _ => self.max,
            }
        };
        (min, constrain(preferred).max(min))
    }
    fn percentage(&self) -> f32 {
        let percent = |size: &Size| match track_size(size) {
            Some(LengthPercentage::Percentage(value)) if *value != 0.0 => Some(*value),
            _ => None,
        };
        percent(&self.minimum).unwrap_or(0.0).max(
            percent(&self.specified)
                .unwrap_or(0.0)
                .min(percent(&self.maximum).unwrap_or(f32::INFINITY)),
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ColumnWidth {
    pub start: usize,
    pub span: usize,
    pub specified: Size,
    pub minimum: Size,
    pub maximum: Size,
    pub group: bool,
}

impl ColumnWidth {
    fn percentage(&self) -> f32 {
        let percentage = |size: &Size| match track_size(size) {
            Some(LengthPercentage::Percentage(value)) => Some(*value),
            _ => None,
        };
        percentage(&self.specified)
            .unwrap_or(0.0)
            .min(percentage(&self.maximum).unwrap_or(f32::INFINITY))
    }
}

/// CSS Values 3 §8.1 lets table cells and columns treat percentage calculations as `auto`.
fn track_size(size: &Size) -> Option<&LengthPercentage> {
    match size {
        Size::Value(LengthPercentage::Calc(_)) | Size::Auto | Size::Intrinsic(_) => None,
        Size::Value(value) => Some(value),
    }
}

pub(crate) struct WidthInput<'a> {
    pub cells: &'a [CellWidth],
    pub columns: &'a [ColumnWidth],
    pub layout: TableLayout,
    /// Width inside the table's padding and borders.
    pub table: SizeConstraints,
    pub available: f32,
    pub spacing: f32,
    pub caption_min: f32,
}

pub(crate) struct ColumnWidths {
    pub widths: Vec<f32>,
    pub table: f32,
}

#[derive(Clone, Copy, Default)]
struct Track {
    min: f32,
    max: f32,
    percent: f32,
    fixed: bool,
}

impl Grid {
    pub(crate) fn column_widths(&self, input: &WidthInput<'_>) -> Result<ColumnWidths, TableError> {
        if input.cells.len() != self.cells.len() {
            return Err(TableError::MeasurementCount {
                expected: self.cells.len(),
                actual: input.cells.len(),
            });
        }
        finite([input.available, input.spacing, input.caption_min])?;
        finite(input.table.specified)?;
        finite([input.table.minimum])?;
        finite(input.table.maximum)?;
        for cell in input.cells {
            finite([cell.min, cell.max, cell.edges])?;
            finite(cell.specified.resolve(input.available))?;
            finite(cell.minimum.resolve(input.available))?;
            finite(cell.maximum.resolve(input.available))?;
        }
        for column in input.columns {
            if column.span == 0
                || column
                    .start
                    .checked_add(column.span)
                    .is_none_or(|end| end > self.columns)
            {
                return Err(TableError::InvalidColumnRange);
            }
            finite(column.specified.resolve(input.available))?;
            finite(column.minimum.resolve(input.available))?;
            finite(column.maximum.resolve(input.available))?;
        }
        if self.columns == 0 {
            return Ok(ColumnWidths {
                widths: Vec::new(),
                table: input
                    .table
                    .constrain(input.table.specified.unwrap_or(0.0))
                    .max(input.caption_min),
            });
        }
        let gaps = input.spacing * (self.columns + 1) as f32;
        let mut widths = if input.layout == TableLayout::Fixed && input.table.specified.is_some() {
            self.fixed(input, gaps)
        } else {
            self.automatic(input, gaps)
        };
        for width in &mut widths {
            *width = width.max(0.0);
        }
        let table = widths.iter().sum::<f32>() + gaps;
        finite(widths.iter().copied().chain([table]))?;
        Ok(ColumnWidths { widths, table })
    }

    fn fixed(&self, input: &WidthInput<'_>, gaps: f32) -> Vec<f32> {
        let target = input
            .table
            .constrain(input.table.specified.unwrap())
            .max(input.caption_min);
        let mut tracks = vec![None; self.columns];
        for column in input.columns.iter().filter(|column| !column.group) {
            let width = match track_size(&column.specified) {
                Some(LengthPercentage::Length(width)) => {
                    Some(width.max(column.minimum.resolve_definite(None).unwrap_or(0.0)))
                }
                Some(LengthPercentage::Percentage(_)) => Some(column.percentage() * target),
                _ => None,
            };
            if let Some(width) = width {
                for track in &mut tracks[column.start..column.start + column.span] {
                    *track = Some(width / column.span as f32);
                }
            }
        }
        for (cell, measure) in self
            .cells
            .iter()
            .zip(input.cells)
            .filter(|(cell, _)| cell.row == 0)
        {
            if let Some(width) = track_size(&measure.specified).map(|size| size.resolve(target)) {
                let width = (measure.outer(width) - input.spacing * (cell.columns - 1) as f32)
                    .max(0.0)
                    / cell.columns as f32;
                for track in &mut tracks[cell.column..cell.column + cell.columns] {
                    if track.is_none() {
                        *track = Some(width);
                    }
                }
            }
        }
        let sum: f32 = tracks.iter().flatten().sum();
        let missing = tracks.iter().filter(|track| track.is_none()).count();
        let share = (target - gaps - sum).max(0.0) / missing.max(1) as f32;
        let mut widths: Vec<_> = tracks
            .into_iter()
            .map(|track| track.unwrap_or(share))
            .collect();
        if missing == 0 {
            grow(&mut widths, (target - gaps - sum).max(0.0));
        }
        widths
    }

    fn automatic(&self, input: &WidthInput<'_>, gaps: f32) -> Vec<f32> {
        let mut tracks = vec![Track::default(); self.columns];
        let mut spanning: Vec<_> = self.cells.iter().zip(input.cells).collect();
        spanning.sort_by_key(|(cell, _)| cell.columns);
        for (cell, measure) in spanning {
            let (min, max) = measure.contributions();
            let range = cell.column..cell.column + cell.columns;
            let inner_gaps = input.spacing * (cell.columns - 1) as f32;
            for maximum in [false, true] {
                let mut values: Vec<_> = tracks[range.clone()]
                    .iter()
                    .map(|track| if maximum { track.max } else { track.min })
                    .collect();
                let required = (if maximum { max } else { min } - inner_gaps).max(0.0);
                let extra = (required - values.iter().sum::<f32>()).max(0.0);
                grow(&mut values, extra);
                for (track, value) in tracks[range.clone()].iter_mut().zip(values) {
                    if maximum {
                        track.max = value.max(track.min);
                    } else {
                        track.min = value;
                        track.max = track.max.max(value);
                    }
                }
            }
            let fixed = matches!(
                track_size(&measure.specified),
                Some(LengthPercentage::Length(_))
            );
            let percentage = measure.percentage() / cell.columns as f32;
            for track in &mut tracks[range] {
                track.fixed |= fixed;
                track.percent = track.percent.max(percentage);
            }
        }
        for column in input.columns {
            let minimum = column.minimum.resolve_definite(None).unwrap_or(0.0);
            let specified = match track_size(&column.specified) {
                Some(LengthPercentage::Length(width)) => *width,
                _ => 0.0,
            };
            let maximum = column
                .maximum
                .resolve_definite(None)
                .unwrap_or(f32::INFINITY);
            let min = minimum.max(specified) / column.span as f32;
            let max = minimum.max(specified.min(maximum)) / column.span as f32;
            for track in &mut tracks[column.start..column.start + column.span] {
                track.min = track.min.max(min);
                track.max = track.max.max(max).max(track.min);
                track.fixed |= matches!(
                    track_size(&column.specified),
                    Some(LengthPercentage::Length(_))
                );
                track.percent = track.percent.max(column.percentage() / column.span as f32);
            }
        }
        let min: f32 = tracks.iter().map(|track| track.min).sum();
        let percent_sum: f32 = tracks.iter().map(|track| track.percent).sum();
        let mut max: f32 = tracks.iter().map(|track| track.max).sum();
        for track in tracks.iter().filter(|track| track.percent > 0.0) {
            max = max.max(track.max / track.percent);
        }
        let non_percent: f32 = tracks
            .iter()
            .filter(|track| track.percent == 0.0)
            .map(|track| track.max)
            .sum();
        if non_percent > 0.0 {
            max = max.max(if percent_sum < 1.0 {
                non_percent / (1.0 - percent_sum)
            } else {
                f32::INFINITY
            });
        }
        let target = input
            .table
            .constrain(
                input
                    .table
                    .specified
                    .unwrap_or(input.available.min(max + gaps)),
            )
            .max(min + gaps)
            .max(input.caption_min);
        let available = (target - gaps).max(0.0);
        let mut widths: Vec<_> = tracks.iter().map(|track| track.min).collect();
        for (track, width) in tracks.iter().zip(&mut widths) {
            if track.percent > 0.0 {
                *width = width.max(available * track.percent / percent_sum.max(1.0));
            }
        }
        let mut remainder = (available - widths.iter().sum::<f32>()).max(0.0);
        let flex: f32 = tracks
            .iter()
            .zip(&widths)
            .filter(|(track, _)| track.percent == 0.0)
            .map(|(track, width)| (track.max - width).max(0.0))
            .sum();
        let expansion = remainder.min(flex);
        if flex > 0.0 {
            for (track, width) in tracks.iter().zip(&mut widths) {
                if track.percent == 0.0 {
                    *width += expansion * (track.max - *width).max(0.0) / flex;
                }
            }
        }
        remainder -= expansion;
        if remainder > 0.0 {
            let automatic = |track: &Track| !track.fixed && track.percent == 0.0;
            let has_auto = tracks.iter().any(automatic);
            let percent_only = tracks.iter().all(|track| track.percent > 0.0);
            let eligible = |track: &Track| {
                if has_auto {
                    automatic(track)
                } else {
                    track.percent == 0.0 || percent_sum >= 1.0 || percent_only
                }
            };
            let count = tracks.iter().filter(|track| eligible(track)).count();
            let weight: f32 = tracks
                .iter()
                .filter(|track| eligible(track))
                .map(|track| {
                    if percent_only {
                        track.percent
                    } else {
                        track.max
                    }
                })
                .sum();
            for (track, width) in tracks.iter().zip(&mut widths) {
                if eligible(track) {
                    *width += remainder
                        * if weight > 0.0 {
                            (if percent_only {
                                track.percent
                            } else {
                                track.max
                            }) / weight
                        } else {
                            1.0 / count.max(1) as f32
                        };
                }
            }
        }
        widths
    }
}
