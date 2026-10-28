use crate::{LayoutError, work::Budget};
use ferri_pdf_grid::{AxisLines, MaxSizing, MinSizing, SizeBounds, TrackSizing};
use ferri_pdf_style::{
    GridAreas, LengthPercentage, MAX_GRID_LINE, POINTS_PER_PIXEL, TrackBreadth, TrackList,
    TrackSize,
};
use std::ops::Range;

pub(super) struct Axis {
    pub lines: AxisLines,
    pub tracks: Vec<TrackSize>,
    pub collapsed: Vec<bool>,
    repeated: Range<usize>,
    fit: bool,
}

#[derive(Clone)]
pub(super) struct TrackGeometry {
    tracks: Vec<Range<f32>>,
    reverse: bool,
    gap: f32,
    collapsed: Vec<bool>,
}

impl TrackGeometry {
    pub fn gap(&self) -> f32 {
        self.gap
    }

    pub fn new(
        sizes: &[f32],
        collapsed: &[bool],
        gap: f32,
        available: Option<f32>,
        alignment: ferri_pdf_style::Alignment,
        reverse: bool,
    ) -> Self {
        use ferri_pdf_style::Alignment;
        let count = collapsed.iter().filter(|collapsed| !**collapsed).count();
        let extent = sizes.iter().sum::<f32>() + gap * count.saturating_sub(1) as f32;
        let available = available.unwrap_or(extent);
        let alignment = match (alignment, reverse) {
            (Alignment::Left, true) | (Alignment::Right, false) => Alignment::End,
            (Alignment::Left, false) | (Alignment::Right, true) => Alignment::Start,
            (alignment, _) => alignment,
        };
        let (mut cursor, between) = ferri_pdf_flex::spacing(alignment, available - extent, count);
        let mut previous = false;
        let tracks = sizes
            .iter()
            .zip(collapsed)
            .map(|(size, collapsed)| {
                if !collapsed && previous {
                    cursor += gap + between;
                }
                let start = cursor;
                cursor += size;
                previous |= !collapsed;
                if reverse {
                    available - cursor..available - start
                } else {
                    start..cursor
                }
            })
            .collect();
        Self {
            tracks,
            reverse,
            gap,
            collapsed: collapsed.to_vec(),
        }
    }

    pub fn inherit(
        &self,
        area: Range<usize>,
        gap: Option<f32>,
        edges: [f32; 2],
        reverse: bool,
    ) -> Result<Self, LayoutError> {
        if area.is_empty()
            || area.end > self.tracks.len()
            || edges.iter().any(|edge| !edge.is_finite())
            || gap.is_some_and(|gap| !gap.is_finite() || gap < 0.0)
        {
            return Err(ferri_pdf_grid::GridError::InvalidGeometry.into());
        }
        let extent = self.area(area.clone());
        let gap = gap.unwrap_or(self.gap);
        let adjustment = (gap - self.gap) / 2.0;
        let origin = extent.start + edges[0];
        let mut collapsed = self.collapsed[area.clone()].to_vec();
        let mut tracks = self.tracks[area].to_vec();
        if self.reverse {
            tracks.reverse();
            collapsed.reverse();
        }
        let last = tracks.len() - 1;
        for (index, track) in tracks.iter_mut().enumerate() {
            if collapsed[index] {
                continue;
            }
            track.start += if index == 0 { edges[0] } else { adjustment };
            track.end -= if index == last { edges[1] } else { adjustment };
            track.start -= origin;
            track.end = (track.end - origin).max(track.start);
        }
        for index in 0..tracks.len() {
            if collapsed[index] {
                let point = if index == 0 {
                    0.0
                } else {
                    tracks[index - 1].end
                };
                tracks[index] = point..point;
            }
        }
        if reverse {
            tracks.reverse();
            collapsed.reverse();
        }
        Ok(Self {
            tracks,
            reverse,
            gap,
            collapsed,
        })
    }

    pub fn area(&self, tracks: Range<usize>) -> Range<f32> {
        if tracks.is_empty() {
            return 0.0..0.0;
        }
        let start = self.boundary(tracks.start, false);
        let end = self.boundary(tracks.end, true);
        start.min(end)..start.max(end)
    }

    pub fn boundary(&self, line: usize, end: bool) -> f32 {
        if self.tracks.is_empty() {
            return 0.0;
        }
        let (track, trailing) = if end && line > 0 {
            (&self.tracks[line - 1], true)
        } else if line < self.tracks.len() {
            (&self.tracks[line], false)
        } else {
            (self.tracks.last().unwrap(), true)
        };
        if trailing ^ self.reverse {
            track.end
        } else {
            track.start
        }
    }

    pub fn extent(&self) -> f32 {
        let area = self.area(0..self.tracks.len());
        area.end - area.start
    }
}

pub(super) fn fixed_extent(tracks: &[TrackSizing], collapsed: &[bool], gap: f32) -> Option<f32> {
    let mut extent = 0.0;
    let mut count = 0usize;
    for (track, collapsed) in tracks.iter().zip(collapsed) {
        if *collapsed {
            continue;
        }
        let (MinSizing::Fixed(min), MaxSizing::Fixed(max)) = (track.min, track.max) else {
            return None;
        };
        extent += min.max(max);
        count += 1;
    }
    Some(extent + gap * count.saturating_sub(1) as f32)
}

fn length(value: &LengthPercentage, basis: Option<f32>) -> Option<f32> {
    match value {
        LengthPercentage::Length(value) => Some(*value),
        value => basis.map(|basis| value.resolve(basis)),
    }
}

fn definite(breadth: &TrackBreadth, basis: Option<f32>) -> Option<f32> {
    match breadth {
        TrackBreadth::Length(value) => length(value, basis),
        _ => None,
    }
}

fn repeat_size(track: &TrackSize, basis: Option<f32>) -> Option<f32> {
    match track {
        TrackSize::Breadth(value) => definite(value, basis),
        TrackSize::MinMax(min, max) => {
            let min = definite(min, basis);
            definite(max, basis)
                .map(|max| max.max(min.unwrap_or(0.0)))
                .or(min)
        }
        TrackSize::FitContent(_) => None,
    }
    .map(|size| size.max(POINTS_PER_PIXEL))
}

impl Axis {
    pub fn new(
        list: &TrackList,
        areas: Option<&GridAreas>,
        axis: usize,
        size: Option<f32>,
        bounds: SizeBounds,
        gap: f32,
        work: &Budget,
    ) -> Result<Self, LayoutError> {
        let mut output = Self {
            lines: AxisLines {
                names: vec![Vec::new()],
                subgridded: false,
                area_names: Vec::new(),
            },
            tracks: Vec::new(),
            collapsed: Vec::new(),
            repeated: 0..0,
            fit: false,
        };
        let limit = MAX_GRID_LINE as usize - 1;
        let repetitions = if let Some(repeat) = &list.repeat {
            let basis = size.or(bounds.max);
            let fixed: Option<f32> = list
                .tracks
                .iter()
                .map(|track| repeat_size(track, basis))
                .sum();
            let repeated: Option<f32> = repeat
                .tracks
                .iter()
                .map(|track| repeat_size(track, basis))
                .sum();
            match (fixed, repeated) {
                (Some(fixed), Some(repeated)) if !repeat.tracks.is_empty() => {
                    let fixed = f64::from(fixed) + f64::from(gap) * list.tracks.len() as f64;
                    let repeated =
                        f64::from(repeated) + f64::from(gap) * repeat.tracks.len() as f64;
                    let count = if let Some(size) = basis {
                        ((f64::from(size) + f64::from(gap) - fixed) / repeated).floor()
                    } else if bounds.min > 0.0 {
                        ((f64::from(bounds.min) + f64::from(gap) - fixed) / repeated).ceil()
                    } else {
                        1.0
                    };
                    (count.max(1.0) as usize).min(limit)
                }
                _ => 1,
            }
        } else {
            0
        };
        for index in 0..=list.tracks.len() {
            work.spend(1)?;
            let name_index =
                index + usize::from(list.repeat.as_ref().is_some_and(|repeat| index > repeat.at));
            if let Some(names) = list.lines.get(name_index) {
                output
                    .lines
                    .names
                    .last_mut()
                    .unwrap()
                    .extend(names.iter().cloned());
            }
            if let Some(repeat) = &list.repeat
                && repeat.at == index
            {
                output.fit = repeat.fit;
                output.repeated.start = output.tracks.len();
                for _ in 0..repetitions {
                    work.spend(repeat.tracks.len().max(1))?;
                    if let Some(names) = repeat.lines.first() {
                        output
                            .lines
                            .names
                            .last_mut()
                            .unwrap()
                            .extend(names.iter().cloned());
                    }
                    for (track, names) in repeat.tracks.iter().zip(repeat.lines.iter().skip(1)) {
                        if output.tracks.len() == limit {
                            break;
                        }
                        output.tracks.push(track.clone());
                        output.lines.names.push(names.clone());
                    }
                    if output.tracks.len() == limit {
                        break;
                    }
                }
                output.repeated.end = output.tracks.len();
                if let Some(names) = list.lines.get(index + 1) {
                    output
                        .lines
                        .names
                        .last_mut()
                        .unwrap()
                        .extend(names.iter().cloned());
                }
            }
            if let Some(track) = list.tracks.get(index)
                && output.tracks.len() < limit
            {
                output.tracks.push(track.clone());
                output.lines.names.push(Vec::new());
            }
        }
        if let Some(areas) = areas {
            let count = areas.size[axis].min(limit);
            output
                .tracks
                .resize(output.tracks.len().max(count), TrackSize::AUTO);
            output
                .lines
                .names
                .resize_with(output.tracks.len() + 1, Vec::new);
            for line in 0..output.lines.names.len() {
                work.spend(areas.areas.len().max(1))?;
                output
                    .lines
                    .area_names
                    .push(areas.line_names(axis, line).map(Into::into).collect());
            }
        }
        output.collapsed.resize(output.tracks.len(), false);
        Ok(output)
    }

    pub fn implicit(
        &mut self,
        count: usize,
        leading: usize,
        cycle: &[TrackSize],
        areas: impl Iterator<Item = Range<usize>>,
        work: &Budget,
    ) -> Result<(), LayoutError> {
        work.spend(count)?;
        let explicit = std::mem::take(&mut self.tracks);
        self.tracks = (0..count)
            .map(|index| {
                if let Some(track) = index
                    .checked_sub(leading)
                    .and_then(|index| explicit.get(index))
                {
                    return track.clone();
                }
                if cycle.is_empty() {
                    return TrackSize::AUTO;
                }
                let index = if index < leading {
                    (cycle.len() - (leading - index) % cycle.len()) % cycle.len()
                } else {
                    (index - leading - explicit.len()) % cycle.len()
                };
                cycle[index].clone()
            })
            .collect();
        self.collapsed = vec![false; count];
        if self.fit {
            let repeated = self.repeated.start + leading..self.repeated.end + leading;
            self.collapsed[repeated].fill(true);
            for area in areas {
                work.spend(area.len())?;
                self.collapsed[area].fill(false);
            }
        }
        Ok(())
    }

    pub fn sizing(&self, basis: Option<f32>) -> Vec<TrackSizing> {
        self.tracks
            .iter()
            .map(|track| {
                let minimum = |value: &TrackBreadth| match value {
                    TrackBreadth::Length(value) => length(value, basis)
                        .map_or(MinSizing::Auto, |value| MinSizing::Fixed(value.max(0.0))),
                    TrackBreadth::MinContent => MinSizing::MinContent,
                    TrackBreadth::MaxContent => MinSizing::MaxContent,
                    TrackBreadth::Auto | TrackBreadth::Flex(_) => MinSizing::Auto,
                };
                let maximum = |value: &TrackBreadth| match value {
                    TrackBreadth::Length(value) => length(value, basis)
                        .map_or(MaxSizing::Auto, |value| MaxSizing::Fixed(value.max(0.0))),
                    TrackBreadth::MinContent => MaxSizing::MinContent,
                    TrackBreadth::MaxContent => MaxSizing::MaxContent,
                    TrackBreadth::Auto => MaxSizing::Auto,
                    TrackBreadth::Flex(value) => MaxSizing::Flex(*value),
                };
                match track {
                    TrackSize::Breadth(value) => TrackSizing {
                        min: minimum(value),
                        max: maximum(value),
                    },
                    TrackSize::MinMax(min, max) => TrackSizing {
                        min: minimum(min),
                        max: maximum(max),
                    },
                    TrackSize::FitContent(limit) => TrackSizing {
                        min: MinSizing::Auto,
                        max: length(limit, basis).map_or(MaxSizing::MaxContent, |limit| {
                            MaxSizing::FitContent(limit.max(0.0))
                        }),
                    },
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn inherited_geometry_keeps_parent_track_centres_with_child_gaps_and_edges() {
        use ferri_pdf_style::Alignment;
        let parent = TrackGeometry::new(
            &[100.0, 150.0],
            &[false; 2],
            50.0,
            None,
            Alignment::Start,
            false,
        );
        let normal = parent.inherit(0..2, None, [0.0; 2], false).unwrap();
        assert_eq!(normal.area(0..1), 0.0..100.0);
        assert_eq!(normal.area(1..2), 150.0..300.0);
        let zero = parent
            .inherit(0..2, Some(0.0), [10.0, 20.0], false)
            .unwrap();
        assert_eq!(zero.area(0..1), 0.0..115.0);
        assert_eq!(zero.area(1..2), 115.0..270.0);
        let reverse = parent
            .inherit(0..2, Some(70.0), [10.0, 20.0], true)
            .unwrap();
        assert_eq!(reverse.area(0..1), 150.0..270.0);
        assert_eq!(reverse.area(1..2), 0.0..80.0);
        let nested = reverse.inherit(0..1, None, [5.0, 7.0], false).unwrap();
        assert_eq!(nested.area(0..1), 0.0..108.0);
        assert!(parent.inherit(0..3, None, [0.0; 2], false).is_err());
        let collapsed = TrackGeometry::new(
            &[100.0, 0.0, 150.0],
            &[false, true, false],
            50.0,
            None,
            Alignment::Start,
            false,
        );
        let child = collapsed.inherit(0..3, Some(0.0), [0.0; 2], false).unwrap();
        assert_eq!(child.area(0..1), 0.0..125.0);
        assert_eq!(child.area(1..2), 125.0..125.0);
        assert_eq!(child.area(2..3), 125.0..300.0);
    }

    #[test]
    fn area_derived_names_are_distinct_from_explicit_names_for_subgrid_inheritance() {
        let areas = ferri_pdf_style::GridAreas {
            size: [1, 2],
            areas: vec![ferri_pdf_style::NamedArea {
                name: "panel".into(),
                tracks: [0..1, 0..2],
            }],
        };
        let axis = super::Axis::new(
            &ferri_pdf_style::TrackList::default(),
            Some(&areas),
            ferri_pdf_style::COLUMNS,
            None,
            ferri_pdf_grid::SizeBounds::default(),
            0.0,
            &crate::work::Budget::new(crate::MAX_LAYOUT_WORK),
        )
        .unwrap();
        assert!(
            axis.lines.names.iter().all(Vec::is_empty),
            "area-derived names must not masquerade as explicit inherited names"
        );
        assert_eq!(
            axis.lines.area_names[0],
            vec![std::sync::Arc::<str>::from("panel-start")]
        );
        assert_eq!(
            axis.lines.area_names[2],
            vec![std::sync::Arc::<str>::from("panel-end")]
        );
    }
    use super::*;
    use ferri_pdf_style::{AutoRepeat, COLUMNS};

    #[test]
    fn positioned_line_edges_exclude_gutters_on_the_outside_of_an_area() {
        for reverse in [false, true] {
            let tracks = TrackGeometry::new(
                &[60.0, 120.0],
                &[false; 2],
                10.0,
                Some(240.0),
                ferri_pdf_style::Alignment::Center,
                reverse,
            );
            let expected = if reverse {
                [215.0, 155.0, 145.0, 25.0]
            } else {
                [25.0, 85.0, 95.0, 215.0]
            };
            assert_eq!(
                [
                    tracks.boundary(0, false),
                    tracks.boundary(1, true),
                    tracks.boundary(1, false),
                    tracks.boundary(2, true)
                ],
                expected,
                "reverse={reverse}: a start uses the following track edge, an end uses the preceding one"
            );
            assert_eq!(tracks.boundary(0, true), expected[0]);
            assert_eq!(tracks.boundary(2, false), expected[3]);
        }
    }

    #[test]
    fn track_areas_include_internal_gaps_and_respect_direction_and_alignment() {
        for reverse in [false, true] {
            let tracks = TrackGeometry::new(
                &[60.0, 160.0 / 3.0, 320.0 / 3.0],
                &[false; 3],
                10.0,
                Some(240.0),
                ferri_pdf_style::Alignment::Start,
                reverse,
            );
            let area = tracks.area(1..3);
            let expected = if reverse { 0.0..170.0 } else { 70.0..240.0 };
            assert!(
                (area.start - expected.start).abs() < 0.001,
                "{area:?}, reverse={reverse}"
            );
            assert!(
                (area.end - expected.end).abs() < 0.001,
                "{area:?}, reverse={reverse}"
            );
            assert!((tracks.extent() - 240.0).abs() < 0.001);
        }
        for alignment in [
            ferri_pdf_style::Alignment::SpaceBetween,
            ferri_pdf_style::Alignment::Center,
        ] {
            let tracks = TrackGeometry::new(
                &[0.0, 50.0, 0.0, 50.0, 0.0],
                &[true, false, true, false, true],
                10.0,
                Some(240.0),
                alignment,
                false,
            );
            let expected = if alignment == ferri_pdf_style::Alignment::SpaceBetween {
                [0.0..50.0, 190.0..240.0]
            } else {
                [65.0..115.0, 125.0..175.0]
            };
            assert_eq!(tracks.area(1..2), expected[0]);
            assert_eq!(tracks.area(3..4), expected[1]);
            assert_eq!(tracks.area(1..4), expected[0].start..expected[1].end);
        }
        let empty = TrackGeometry::new(
            &[0.0; 3],
            &[true; 3],
            10.0,
            Some(240.0),
            ferri_pdf_style::Alignment::SpaceEvenly,
            false,
        );
        assert_eq!(empty.extent(), 0.0, "collapsed tracks retain no gutters");
    }

    fn fixed(value: f32) -> TrackSize {
        TrackSize::Breadth(TrackBreadth::Length(LengthPercentage::Length(value)))
    }

    #[test]
    fn repeat_counts_honor_definite_maximum_and_minimum_sizes() {
        let list = TrackList {
            subgrid: None,
            lines: vec![
                vec!["before".into()],
                vec!["after".into()],
                vec!["end".into()],
            ],
            tracks: vec![fixed(20.0)],
            repeat: Some(AutoRepeat {
                at: 0,
                fit: false,
                lines: vec![vec!["start".into()], vec!["finish".into()]],
                tracks: vec![fixed(50.0)],
            }),
        };
        let work = Budget::new(crate::MAX_LAYOUT_WORK);
        for (size, bounds, expected) in [
            (Some(250.0), SizeBounds::default(), 4),
            (
                None,
                SizeBounds {
                    min: 0.0,
                    max: Some(250.0),
                },
                4,
            ),
            (
                None,
                SizeBounds {
                    min: 250.0,
                    max: None,
                },
                5,
            ),
            (None, SizeBounds::default(), 2),
        ] {
            let axis = Axis::new(&list, None, COLUMNS, size, bounds, 10.0, &work).unwrap();
            assert_eq!(
                axis.tracks.len(),
                expected,
                "size={size:?}, bounds={bounds:?}: includes the nonrepeated 20pt track"
            );
            assert_eq!(axis.lines.names[0], vec!["before".into(), "start".into()]);
            assert_eq!(
                axis.lines.names[expected - 1],
                vec!["finish".into(), "after".into()]
            );
            assert_eq!(axis.lines.names[expected], vec!["end".into()]);
        }
    }

    #[test]
    fn implicit_track_cycles_run_backwards_before_the_explicit_grid() {
        let work = Budget::new(crate::MAX_LAYOUT_WORK);
        let list = TrackList {
            subgrid: None,
            lines: vec![Vec::new(); 2],
            tracks: vec![fixed(50.0)],
            repeat: None,
        };
        let mut axis = Axis::new(
            &list,
            None,
            COLUMNS,
            None,
            SizeBounds::default(),
            0.0,
            &work,
        )
        .unwrap();
        axis.implicit(
            6,
            3,
            &[fixed(10.0), fixed(20.0)],
            std::iter::once(0..6),
            &work,
        )
        .unwrap();
        assert_eq!(axis.tracks, [20.0, 10.0, 20.0, 50.0, 10.0, 20.0].map(fixed));
    }

    #[test]
    fn auto_fit_retains_spanned_tracks_and_collapses_only_empty_repetitions() {
        let work = Budget::new(crate::MAX_LAYOUT_WORK);
        let list = TrackList {
            subgrid: None,
            lines: vec![Vec::new(); 2],
            tracks: Vec::new(),
            repeat: Some(AutoRepeat {
                at: 0,
                fit: true,
                lines: vec![Vec::new(); 2],
                tracks: vec![fixed(50.0)],
            }),
        };
        let mut axis = Axis::new(
            &list,
            None,
            COLUMNS,
            Some(230.0),
            SizeBounds::default(),
            10.0,
            &work,
        )
        .unwrap();
        assert_eq!(axis.tracks.len(), 4);
        axis.implicit(6, 1, &[TrackSize::AUTO], std::iter::once(2..4), &work)
            .unwrap();
        assert_eq!(
            axis.collapsed,
            [false, true, false, false, true, false],
            "implicit tracks do not collapse and a spanning item occupies every crossed repeated track"
        );
    }
}
