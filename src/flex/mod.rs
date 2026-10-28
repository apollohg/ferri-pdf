//! Flexible length resolution and axis-space distribution, independent of text and pages.
use ferri_pdf_style::Alignment;
use std::ops::Range;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Item {
    /// Second-pass visibility strut: retains line membership without main-axis space.
    pub collapsed: bool,
    pub basis: f32,
    pub minimum: f32,
    pub maximum: Option<f32>,
    pub grow: f32,
    pub shrink: f32,
    pub edges: f32,
    pub margins: [Option<f32>; 2],
}
impl Item {
    fn clamp(self, value: f64) -> f64 {
        self.maximum
            .map_or(value, |v| value.min(f64::from(v)))
            .max(f64::from(self.minimum))
            .max(0.0)
    }
    fn outer(self, value: f64) -> f64 {
        value
            + f64::from(self.edges)
            + self
                .margins
                .into_iter()
                .flatten()
                .map(f64::from)
                .sum::<f64>()
    }
    fn validate(self) -> Result<(), FlexError> {
        if [self.basis, self.minimum, self.grow, self.shrink, self.edges]
            .into_iter()
            .any(|v| !v.is_finite() || v < 0.0)
            || self.maximum.is_some_and(|v| !v.is_finite() || v < 0.0)
            || self.margins.into_iter().flatten().any(|v| !v.is_finite())
        {
            return Err(FlexError::InvalidGeometry);
        }
        Ok(())
    }
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum FlexError {
    #[error("invalid or non-finite flex geometry")]
    InvalidGeometry,
}
pub(crate) struct Line {
    pub sizes: Vec<f32>,
    /// Border-box starts measured from the main-start edge.
    pub starts: Vec<f32>,
}
pub(crate) fn line_ranges(
    items: &[Item],
    available: Option<f32>,
    gap: f32,
    wrap: bool,
) -> Result<Vec<Range<usize>>, FlexError> {
    validate_container(available, gap)?;
    let mut result = Vec::new();
    let mut start = 0;
    let mut used = 0.0;
    let mut participants = 0;
    for (i, item) in items.iter().copied().enumerate() {
        item.validate()?;
        if item.collapsed {
            continue;
        }
        let outer = item.outer(item.clamp(f64::from(item.basis)));
        let next =
            used + if participants > 0 {
                f64::from(gap)
            } else {
                0.0
            } + outer;
        if wrap && i > start && available.is_some_and(|limit| next > f64::from(limit)) {
            result.push(start..i);
            start = i;
            used = outer;
            participants = 1;
        } else {
            used = next;
            participants += 1;
        }
    }
    if start < items.len() {
        result.push(start..items.len());
    }
    Ok(result)
}
fn validate_container(available: Option<f32>, gap: f32) -> Result<(), FlexError> {
    if available.is_some_and(|v| !v.is_finite() || v < 0.0) || !gap.is_finite() || gap < 0.0 {
        Err(FlexError::InvalidGeometry)
    } else {
        Ok(())
    }
}
/// Returns leading space and additional space between adjacent items.
pub(crate) fn spacing(alignment: Alignment, free: f32, count: usize) -> (f32, f32) {
    if count == 0 {
        return (0.0, 0.0);
    }
    match alignment {
        Alignment::End | Alignment::FlexEnd => (free, 0.0),
        Alignment::Center => (free / 2.0, 0.0),
        Alignment::SpaceBetween if count > 1 && free > 0.0 => (0.0, free / (count - 1) as f32),
        Alignment::SpaceAround if free > 0.0 => (free / (2 * count) as f32, free / count as f32),
        Alignment::SpaceEvenly if free > 0.0 => {
            (free / (count + 1) as f32, free / (count + 1) as f32)
        }
        Alignment::SpaceAround | Alignment::SpaceEvenly => (free / 2.0, 0.0),
        _ => (0.0, 0.0),
    }
}
pub(crate) fn resolve_line(
    items: &[Item],
    available: f32,
    gap: f32,
    alignment: Alignment,
) -> Result<Line, FlexError> {
    if !items.iter().any(|item| item.collapsed) {
        return resolve_visible_line(items, available, gap, alignment);
    }
    let visible: Vec<_> = items
        .iter()
        .copied()
        .filter(|item| !item.collapsed)
        .collect();
    let line = resolve_visible_line(&visible, available, gap, alignment)?;
    let mut sizes = Vec::with_capacity(items.len());
    let mut starts = Vec::with_capacity(items.len());
    let mut values = line.sizes.into_iter().zip(line.starts);
    let mut boundary = 0.0;
    for item in items {
        item.validate()?;
        let (size, start) = if item.collapsed {
            (0.0, boundary)
        } else {
            values
                .next()
                .expect("each participating item has a resolved position")
        };
        if !item.collapsed {
            boundary = start + size + item.edges;
        }
        sizes.push(size);
        starts.push(start);
    }
    Ok(Line { sizes, starts })
}

fn resolve_visible_line(
    items: &[Item],
    available: f32,
    gap: f32,
    alignment: Alignment,
) -> Result<Line, FlexError> {
    validate_container(Some(available), gap)?;
    for item in items {
        item.validate()?;
    }
    let gaps = f64::from(gap) * items.len().saturating_sub(1) as f64;
    let grow = items
        .iter()
        .map(|i| i.outer(i.clamp(f64::from(i.basis))))
        .sum::<f64>()
        + gaps
        < f64::from(available);
    let mut sizes: Vec<f64> = items.iter().map(|i| f64::from(i.basis)).collect();
    let mut frozen = vec![false; items.len()];
    for (index, item) in items.iter().copied().enumerate() {
        let hypothetical = item.clamp(f64::from(item.basis));
        if (if grow { item.grow } else { item.shrink }) == 0.0
            || (grow && f64::from(item.basis) > hypothetical)
            || (!grow && f64::from(item.basis) < hypothetical)
        {
            frozen[index] = true;
            sizes[index] = hypothetical;
        }
    }
    let remaining = |sizes: &[f64], frozen: &[bool]| {
        f64::from(available)
            - gaps
            - items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    item.outer(if frozen[i] {
                        sizes[i]
                    } else {
                        f64::from(item.basis)
                    })
                })
                .sum::<f64>()
    };
    let initial = remaining(&sizes, &frozen);
    while frozen.iter().any(|v| !*v) {
        let mut free = remaining(&sizes, &frozen);
        let factor_sum: f64 = items
            .iter()
            .zip(&frozen)
            .filter(|(_, f)| !**f)
            .map(|(item, _)| {
                if grow {
                    f64::from(item.grow)
                } else {
                    f64::from(item.shrink)
                }
            })
            .sum();
        if factor_sum < 1.0 && (initial * factor_sum).abs() < free.abs() {
            free = initial * factor_sum;
        }
        let weight: f64 = items
            .iter()
            .zip(&frozen)
            .filter(|(_, f)| !**f)
            .map(|(item, _)| {
                if grow {
                    f64::from(item.grow)
                } else {
                    f64::from(item.shrink) * f64::from(item.basis)
                }
            })
            .sum();
        let mut violations = vec![0.0; items.len()];
        for (i, item) in items.iter().enumerate() {
            if frozen[i] {
                continue;
            }
            let factor = if grow {
                f64::from(item.grow)
            } else {
                f64::from(item.shrink) * f64::from(item.basis)
            };
            let target = f64::from(item.basis)
                + if weight > 0.0 {
                    free * factor / weight
                } else {
                    0.0
                };
            sizes[i] = item.clamp(target);
            violations[i] = sizes[i] - target;
        }
        let total: f64 = violations.iter().sum();
        for (i, violation) in violations.iter().enumerate() {
            if total == 0.0
                || (total > 0.0 && *violation > 0.0)
                || (total < 0.0 && *violation < 0.0)
            {
                frozen[i] = true;
            }
        }
    }
    let free = f64::from(available)
        - gaps
        - items
            .iter()
            .zip(&sizes)
            .map(|(item, size)| item.outer(*size))
            .sum::<f64>();
    let auto = items
        .iter()
        .flat_map(|item| item.margins)
        .filter(Option::is_none)
        .count();
    let auto_margin = if auto > 0 && free > 0.0 {
        free / auto as f64
    } else {
        0.0
    };
    let (leading, between) = spacing(
        alignment,
        if auto_margin > 0.0 { 0.0 } else { free as f32 },
        items.len(),
    );
    let mut cursor = f64::from(leading);
    let mut starts = Vec::with_capacity(items.len());
    for (item, size) in items.iter().zip(&sizes) {
        cursor += item.margins[0].map_or(auto_margin, f64::from);
        starts.push(cursor as f32);
        cursor += size
            + f64::from(item.edges)
            + item.margins[1].map_or(auto_margin, f64::from)
            + f64::from(gap)
            + f64::from(between);
    }
    if sizes
        .iter()
        .any(|v| !v.is_finite() || *v > f64::from(f32::MAX))
        || starts.iter().any(|v| !v.is_finite())
    {
        return Err(FlexError::InvalidGeometry);
    }
    Ok(Line {
        sizes: sizes.into_iter().map(|v| v as f32).collect(),
        starts,
    })
}

/// Main-axis intrinsic size, using each item's content contribution.
pub(crate) fn intrinsic_line(
    items: &[Item],
    contributions: &[f32],
    gap: f32,
) -> Result<f32, FlexError> {
    validate_container(None, gap)?;
    if items.len() != contributions.len() {
        return Err(FlexError::InvalidGeometry);
    }
    let filtered_items;
    let filtered_contributions;
    let (items, contributions) = if items.iter().any(|item| item.collapsed) {
        filtered_items = items
            .iter()
            .copied()
            .filter(|item| !item.collapsed)
            .collect::<Vec<_>>();
        filtered_contributions = items
            .iter()
            .zip(contributions)
            .filter_map(|(item, value)| (!item.collapsed).then_some(*value))
            .collect::<Vec<_>>();
        (filtered_items.as_slice(), filtered_contributions.as_slice())
    } else {
        (items, contributions)
    };
    let mut chosen = f64::NEG_INFINITY;
    for (item, contribution) in items.iter().copied().zip(contributions) {
        item.validate()?;
        if !contribution.is_finite() || *contribution < 0.0 {
            return Err(FlexError::InvalidGeometry);
        }
        let difference = f64::from(*contribution) - f64::from(item.basis);
        let fraction = if difference > 0.0 {
            difference / f64::from(item.grow).max(1.0)
        } else if difference < 0.0 {
            let scaled = f64::from(item.shrink).max(1.0) * f64::from(item.basis);
            if scaled > 0.0 {
                difference / scaled
            } else {
                f64::NEG_INFINITY
            }
        } else {
            0.0
        };
        chosen = chosen.max(fraction);
    }
    if !chosen.is_finite() {
        chosen = 0.0;
    }
    let mut size = f64::from(gap) * items.len().saturating_sub(1) as f64;
    for item in items.iter().copied() {
        let factor = if chosen >= 0.0 {
            f64::from(item.grow)
        } else {
            f64::from(item.shrink) * f64::from(item.basis)
        };
        size += item.outer(item.clamp(f64::from(item.basis) + factor * chosen));
    }
    if !size.is_finite() || size > f64::from(f32::MAX) {
        return Err(FlexError::InvalidGeometry);
    }
    Ok(size.max(0.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(basis: f32, grow: f32, shrink: f32, minimum: f32, maximum: Option<f32>) -> Item {
        Item {
            collapsed: false,
            basis,
            grow,
            shrink,
            minimum,
            maximum,
            edges: 0.0,
            margins: [Some(0.0); 2],
        }
    }

    #[test]
    fn collapsed_items_keep_line_membership_without_gaps_or_flexible_space() {
        let mut collapsed = item(300.0, 10.0, 1.0, 200.0, None);
        collapsed.collapsed = true;
        collapsed.edges = 30.0;
        collapsed.margins = [None, None];
        let items = [
            item(40.0, 0.0, 0.0, 0.0, None),
            collapsed,
            item(40.0, 0.0, 0.0, 0.0, None),
        ];
        let lines = line_ranges(&items, Some(90.0), 10.0, true).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], 0..3);
        assert_eq!(
            line_ranges(&items, Some(80.0), 10.0, true).unwrap(),
            [0..2, 2..3],
            "the zero-size strut remains on the preceding line"
        );
        let line = resolve_line(&items, 100.0, 10.0, Alignment::SpaceBetween).unwrap();
        assert_eq!(line.sizes, [40.0, 0.0, 40.0]);
        assert_eq!(line.starts[0], 0.0);
        assert_eq!(
            line.starts[2], 60.0,
            "collapsed item must not consume an auto margin or alignment slot"
        );
        let empty = resolve_line(&[collapsed], 100.0, 10.0, Alignment::SpaceAround).unwrap();
        assert_eq!(empty.sizes, [0.0]);
        assert_eq!(empty.starts, [0.0]);
        assert_eq!(
            intrinsic_line(&items, &[40.0, 300.0, 40.0], 10.0).unwrap(),
            90.0
        );
        assert_eq!(intrinsic_line(&[collapsed], &[300.0], 10.0).unwrap(), 0.0);
    }

    #[test]
    fn freezing_redistributes_space_after_minimum_and_maximum_violations() {
        let items = [
            item(50.0, 1.0, 1.0, 0.0, Some(60.0)),
            item(50.0, 1.0, 1.0, 100.0, None),
            item(50.0, 1.0, 1.0, 0.0, None),
        ];
        let grown = resolve_line(&items, 300.0, 0.0, Alignment::FlexStart).unwrap();
        assert_eq!(
            grown.sizes,
            [60.0, 120.0, 120.0],
            "the capped first item must release space to unfrozen items"
        );
        let shrunk = resolve_line(&items, 150.0, 0.0, Alignment::FlexStart).unwrap();
        assert_eq!(
            shrunk.sizes,
            [25.0, 100.0, 25.0],
            "the minimum-constrained item must not absorb shrinking"
        );
        assert_eq!(shrunk.starts, [0.0, 25.0, 125.0]);
    }

    #[test]
    fn fractional_growth_leaves_space_for_auto_margins() {
        let mut first = item(40.0, 0.25, 1.0, 0.0, None);
        first.margins = [None, None];
        let resolved = resolve_line(
            &[first, item(40.0, 0.25, 1.0, 0.0, None)],
            200.0,
            0.0,
            Alignment::SpaceBetween,
        )
        .unwrap();
        assert_eq!(resolved.sizes, [70.0, 70.0]);
        assert_eq!(
            resolved.starts,
            [30.0, 130.0],
            "auto margins absorb the residual free space before justification"
        );
    }

    #[test]
    fn wrapping_uses_hypothetical_outer_sizes_and_keeps_oversized_items() {
        let items = [
            item(40.0, 1.0, 1.0, 80.0, None),
            item(40.0, 1.0, 1.0, 80.0, None),
            item(200.0, 0.0, 0.0, 0.0, None),
        ];
        assert_eq!(
            line_ranges(&items, Some(165.0), 10.0, true).unwrap(),
            vec![0..1, 1..2, 2..3]
        );
        assert_eq!(line_ranges(&items, None, 10.0, true).unwrap(), vec![0..3]);
    }
}
