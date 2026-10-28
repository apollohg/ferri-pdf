use crate::text::{Line, TextError};

pub(super) fn fit(
    boundaries: &[usize],
    end: usize,
    width: f32,
    monotone: bool,
    emit: impl Fn(usize) -> Result<Line, TextError>,
) -> Result<(usize, Line), TextError> {
    let offset = |index| boundaries.get(index).copied().unwrap_or(end);
    let mut low = 0;
    let mut best = emit(offset(low))?;
    if !monotone {
        for index in 1..=boundaries.len() {
            let candidate = emit(offset(index))?;
            if candidate.wrap_width <= width {
                low = index;
                best = candidate;
            }
        }
        return Ok((offset(low), best));
    }
    if best.wrap_width > width {
        return Ok((offset(low), best));
    }
    let mut high = 1;
    loop {
        high = high.min(boundaries.len());
        let candidate = emit(offset(high))?;
        if candidate.wrap_width > width {
            break;
        }
        best = candidate;
        low = high;
        if high == boundaries.len() {
            return Ok((end, best));
        }
        high = high.saturating_mul(2);
    }
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        let candidate = emit(offset(middle))?;
        if candidate.wrap_width <= width {
            low = middle;
            best = candidate;
        } else {
            high = middle;
        }
    }
    Ok((offset(low), best))
}

pub(super) fn context_range(
    opportunities: &[(usize, unicode_linebreak::BreakOpportunity)],
    range: std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    let first = opportunities.partition_point(|&(offset, _)| offset <= range.start);
    let last = opportunities.partition_point(|&(offset, _)| offset < range.end);
    let start = first
        .checked_sub(1)
        .map_or(0, |index| opportunities[index].0);
    let end = opportunities
        .get(last)
        .map_or(range.end, |&(offset, _)| offset);
    start..end
}
