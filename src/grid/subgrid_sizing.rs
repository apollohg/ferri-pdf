use crate::grid::{GridError, SizingItem};
use std::{collections::BTreeMap, ops::Range};

/// Project child contributions and edge-spanning hypothetical items (Grid 2 §9).
pub(crate) fn subgrid_contributions(
    items: &[SizingItem],
    parent: Range<usize>,
    reverse: bool,
    edges: [f32; 2],
    gap_difference: f32,
) -> Result<Vec<SizingItem>, GridError> {
    if parent.is_empty()
        || !gap_difference.is_finite()
        || edges.iter().any(|edge| !edge.is_finite())
    {
        return Err(GridError::InvalidGeometry);
    }
    let tracks = parent.len();
    for item in items {
        if item.tracks.is_empty() || item.tracks.end > tracks {
            return Err(GridError::InvalidSpan { tracks });
        }
    }
    if items.iter().any(|item| {
        ![item.min_content, item.max_content, item.minimum]
            .into_iter()
            .chain(item.content_minimum)
            .all(f32::is_finite)
    }) {
        return Err(GridError::InvalidGeometry);
    }
    let first = items
        .iter()
        .map(|item| item.tracks.start)
        .min()
        .unwrap_or(0);
    let last = items
        .iter()
        .map(|item| item.tracks.end)
        .max()
        .unwrap_or(tracks);
    let half_gap = gap_difference / 2.0;
    let project = |item: &SizingItem, range: Range<usize>, sides: [f32; 2]| {
        let addition = sides[0] + sides[1];
        let increase = |value: f32| value + addition;
        SizingItem {
            tracks: if reverse {
                parent.end - range.end..parent.end - range.start
            } else {
                parent.start + range.start..parent.start + range.end
            },
            min_content: increase(item.min_content),
            max_content: increase(item.max_content),
            minimum: increase(item.minimum),
            content_minimum: item.content_minimum.map(increase),
        }
    };
    let mut output = Vec::with_capacity(items.len());
    let mut hypothetical: BTreeMap<(usize, usize), SizingItem> = BTreeMap::new();
    for item in items {
        let sides = [
            if item.tracks.start == 0 {
                edges[0]
            } else {
                half_gap
            },
            if item.tracks.end == tracks {
                edges[1]
            } else {
                half_gap
            },
        ];
        output.push(project(item, item.tracks.clone(), sides));
        let extend = [
            first > 0 && item.tracks.start == first,
            last < tracks && item.tracks.end == last,
        ];
        for (start, end) in [(true, false), (false, true), (true, true)] {
            if (start && !extend[0]) || (end && !extend[1]) {
                continue;
            }
            let range = if start { 0 } else { item.tracks.start }..if end {
                tracks
            } else {
                item.tracks.end
            };
            let projected = project(
                item,
                range,
                [
                    if start { edges[0] } else { sides[0] },
                    if end { edges[1] } else { sides[1] },
                ],
            );
            hypothetical
                .entry((projected.tracks.start, projected.tracks.end))
                .and_modify(|current| {
                    current.min_content = current.min_content.max(projected.min_content);
                    current.max_content = current.max_content.max(projected.max_content);
                    current.minimum = current.minimum.max(projected.minimum);
                    current.content_minimum =
                        match (current.content_minimum, projected.content_minimum) {
                            (Some(left), Some(right)) => Some(left.max(right)),
                            (left, right) => left.or(right),
                        };
                })
                .or_insert(projected);
        }
    }
    output.extend(hypothetical.into_values());
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(tracks: Range<usize>, size: f32) -> SizingItem {
        SizingItem {
            tracks,
            min_content: size,
            max_content: size,
            minimum: size,
            content_minimum: Some(size),
        }
    }
    #[test]
    fn projected_content_sizes_parent_auto_tracks_and_rejects_invalid_inputs() {
        use crate::grid::{
            AvailableSpace, MaxSizing, MinSizing, SizeBounds, TrackSizing, size_tracks,
        };
        let input = [item(0..1, 80.0), item(1..2, 60.0)];
        let projected = subgrid_contributions(&input, 0..2, true, [10.0, 20.0], -20.0).unwrap();
        let tracks = vec![
            TrackSizing {
                min: MinSizing::Auto,
                max: MaxSizing::Auto
            };
            2
        ];
        assert_eq!(
            size_tracks(
                &tracks,
                20.0,
                &projected,
                AvailableSpace::MaxContent,
                SizeBounds::default(),
                false
            )
            .unwrap(),
            vec![70.0, 80.0]
        );
        assert!(subgrid_contributions(&[item(0..3, 10.0)], 0..2, false, [0.0; 2], 0.0).is_err());
        assert!(
            subgrid_contributions(&[item(0..1, f32::NAN)], 0..2, false, [0.0; 2], 0.0).is_err()
        );
    }

    #[test]
    fn descendants_map_to_parent_tracks_with_edges_gaps_and_direction() {
        let output = subgrid_contributions(
            &[item(0..1, 80.0), item(1..2, 60.0)],
            2..4,
            true,
            [10.0, 20.0],
            -20.0,
        )
        .unwrap();
        assert_eq!(output, vec![item(3..4, 80.0), item(2..3, 70.0)]);
        let nested = subgrid_contributions(&output, 1..6, false, [0.0; 2], 10.0).unwrap();
        assert_eq!(nested[0], item(4..5, 90.0));
        assert_eq!(nested[1], item(3..4, 80.0));
        let negative =
            subgrid_contributions(&[item(0..1, 10.0)], 0..1, false, [-15.0; 2], 0.0).unwrap();
        let restored = subgrid_contributions(&negative, 0..1, false, [20.0; 2], 0.0).unwrap();
        assert_eq!(
            restored[0].min_content, 20.0,
            "nested signed margins accumulate before track sizing floors them"
        );
    }
    #[test]
    fn empty_edge_tracks_receive_hypothetical_spans_for_each_occupied_span_size() {
        let output = subgrid_contributions(
            &[item(1..3, 100.0), item(1..2, 40.0), item(2..3, 50.0)],
            0..4,
            false,
            [25.0; 2],
            0.0,
        )
        .unwrap();
        for expected in [
            item(0..2, 65.0),
            item(0..3, 125.0),
            item(1..4, 125.0),
            item(2..4, 75.0),
            item(0..4, 150.0),
        ] {
            assert!(
                output.contains(&expected),
                "missing edge contribution {expected:?}; got {output:?}"
            );
        }
    }
}
