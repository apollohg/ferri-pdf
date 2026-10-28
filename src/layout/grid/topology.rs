use super::tracks::Axis;
use crate::grid::{AxisLines, ItemPlacement, Placement, SizeBounds};
use crate::layout::{LayoutError, work::Budget};
use ferri_pdf_style::{COLUMNS, GridAreas, ROWS, Style};

pub(super) struct Topology {
    pub lines: [AxisLines; 2],
    pub axes: [Axis; 2],
    pub placement: Placement,
    pub intrinsic_gaps: [f32; 2],
    areas: Option<GridAreas>,
    rtl: bool,
}

impl Topology {
    pub(super) fn new(
        style: &Style,
        placements: &[ItemPlacement],
        sizes: [Option<f32>; 2],
        bounds: [SizeBounds; 2],
        work: &Budget,
    ) -> Result<Self, LayoutError> {
        Self::with_parent(style, placements, sizes, bounds, work, None)
    }

    pub(super) fn with_parent(
        style: &Style,
        placements: &[ItemPlacement],
        sizes: [Option<f32>; 2],
        bounds: [SizeBounds; 2],
        work: &Budget,
        parent: Option<(&Self, usize)>,
    ) -> Result<Self, LayoutError> {
        let intrinsic_gaps = [ROWS, COLUMNS].map(|axis| {
            let normal = parent
                .filter(|_| style.grid.templates[axis].subgrid.is_some())
                .map_or(0.0, |(parent, _)| parent.intrinsic_gaps[axis]);
            style.flex.gap[axis].resolve_normal(0.0, normal)
        });
        let gaps =
            [ROWS, COLUMNS].map(|axis| style.flex.gap[axis].resolve(sizes[axis].unwrap_or(0.0)));
        let mut axes = [
            Axis::new(
                &style.grid.templates[ROWS],
                style.grid.areas.as_deref(),
                ROWS,
                sizes[ROWS],
                bounds[ROWS],
                gaps[ROWS],
                work,
            )?,
            Axis::new(
                &style.grid.templates[COLUMNS],
                style.grid.areas.as_deref(),
                COLUMNS,
                sizes[COLUMNS],
                bounds[COLUMNS],
                gaps[COLUMNS],
                work,
            )?,
        ];
        let mut areas = style.grid.areas.as_deref().cloned();
        if let Some((parent, item)) = parent {
            let area = parent
                .placement
                .areas
                .get(item)
                .ok_or(crate::grid::GridError::InvalidGeometry)?;
            let coordinates = [ROWS, COLUMNS].map(|axis| {
                let leading = parent.placement.leading[axis] as i64;
                area[axis].start as i64 - leading..area[axis].end as i64 - leading
            });
            let reverse = [false, style.rtl != parent.rtl];
            work.spend(
                parent
                    .areas
                    .as_ref()
                    .map_or(1, |areas| areas.areas.len().max(1)),
            )?;
            let inherited = parent
                .areas
                .as_ref()
                .map(|areas| crate::grid::inherited_areas(areas, &coordinates, reverse))
                .transpose()?;
            if let Some(inherited) = &inherited {
                work.spend(inherited.areas.len().max(1))?;
                if let Some(areas) = &mut areas {
                    areas.areas.extend(inherited.areas.iter().cloned());
                } else {
                    areas = Some(inherited.clone());
                }
            }
            for axis in [ROWS, COLUMNS] {
                let Some(local) = &style.grid.templates[axis].subgrid else {
                    continue;
                };
                let span = area[axis].len();
                for line in 0..=span {
                    let parent_line = if reverse[axis] {
                        coordinates[axis].end - line as i64
                    } else {
                        coordinates[axis].start + line as i64
                    };
                    let count = usize::try_from(parent_line)
                        .ok()
                        .and_then(|line| parent.lines[axis].names.get(line))
                        .map_or(0, Vec::len);
                    work.spend(
                        count
                            .saturating_add(local.line_names(span, line).len())
                            .max(1),
                    )?;
                }
                axes[axis].lines =
                    parent.lines[axis].inherit(coordinates[axis].clone(), reverse[axis], local)?;
                axes[axis].tracks = parent.axes[axis].tracks[area[axis].clone()].to_vec();
                axes[axis].collapsed = parent.axes[axis].collapsed[area[axis].clone()].to_vec();
                if reverse[axis] {
                    axes[axis].tracks.reverse();
                    axes[axis].collapsed.reverse();
                }
                if let Some(areas) = &areas {
                    for line in 0..=span {
                        work.spend(areas.areas.len().max(1))?;
                        axes[axis]
                            .lines
                            .area_names
                            .push(areas.line_names(axis, line).map(Into::into).collect());
                    }
                }
            }
        }
        let lines = axes.each_mut().map(|axis| {
            std::mem::replace(
                &mut axis.lines,
                AxisLines {
                    names: Vec::new(),
                    subgridded: false,
                    area_names: Vec::new(),
                },
            )
        });
        work.spend(placements.len().max(1))?;
        let placement = crate::grid::place(&lines, placements, style.grid.auto_flow)?;
        for axis in [ROWS, COLUMNS] {
            if lines[axis].subgridded {
                continue;
            }
            axes[axis].implicit(
                placement.tracks[axis],
                placement.leading[axis],
                &style.grid.auto_tracks[axis],
                placement.areas.iter().map(|area| area[axis].clone()),
                work,
            )?;
        }
        Ok(Self {
            lines,
            axes,
            placement,
            intrinsic_gaps,
            areas,
            rtl: style.rtl,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferri_pdf_style::{GridAreas, GridLine, NamedArea, Subgrid, TrackList, TrackSize};
    use std::sync::Arc;

    #[test]
    fn parent_aware_placement_clips_areas_reverses_names_and_bounds_implicit_tracks() {
        let mut style = Style::default();
        style.flex.gap[COLUMNS] =
            ferri_pdf_style::Gap::Length(ferri_pdf_style::LengthPercentage::Length(20.0));
        style.grid.templates[COLUMNS] = Arc::new(TrackList {
            tracks: vec![TrackSize::AUTO; 4],
            lines: (0..5)
                .map(|line| vec![Arc::from(format!("p{line}"))])
                .collect(),
            ..Default::default()
        });
        style.grid.areas = Some(Arc::new(GridAreas {
            size: [1, 4],
            areas: vec![NamedArea {
                name: "panel".into(),
                tracks: [0..1, 0..3],
            }],
        }));
        let work = Budget::new(crate::layout::MAX_LAYOUT_WORK);
        let mut item = ItemPlacement::default();
        item[COLUMNS] = [GridLine::Line(2, None), GridLine::Line(5, None)];
        let parent = Topology::new(
            &style,
            &[item],
            [None; 2],
            [SizeBounds::default(); 2],
            &work,
        )
        .unwrap();
        let mut child = Style::default();
        child.rtl = true;
        child.grid.templates = [
            Arc::new(TrackList {
                subgrid: Some(Subgrid::default()),
                ..Default::default()
            }),
            Arc::new(TrackList {
                subgrid: Some(Subgrid::default()),
                ..Default::default()
            }),
        ];
        let nested = Topology::with_parent(
            &child,
            &vec![ItemPlacement::default(); 5],
            [None; 2],
            [SizeBounds::default(); 2],
            &work,
            Some((&parent, 0)),
        )
        .unwrap();
        assert_eq!(nested.placement.tracks, [1, 3]);
        assert_eq!(nested.intrinsic_gaps[COLUMNS], 20.0);
        assert_eq!(nested.lines[COLUMNS].names[0][0].as_ref(), "p4");
        assert_eq!(nested.lines[COLUMNS].names[3][0].as_ref(), "p1");
        assert_eq!(
            nested.lines[COLUMNS].area_names[1][0].as_ref(),
            "panel-start"
        );
        assert_eq!(nested.lines[COLUMNS].area_names[3][0].as_ref(), "panel-end");
        assert!(
            nested
                .placement
                .areas
                .iter()
                .all(|area| area[ROWS] == (0..1) && area[COLUMNS].end <= 3)
        );
        child.rtl = false;
        let grandchild = Topology::with_parent(
            &child,
            &[ItemPlacement::default()],
            [None; 2],
            [SizeBounds::default(); 2],
            &work,
            Some((&nested, 1)),
        )
        .unwrap();
        assert_eq!(grandchild.placement.tracks, [1, 1]);
        assert_eq!(grandchild.intrinsic_gaps[COLUMNS], 20.0);
        assert_eq!(grandchild.lines[COLUMNS].names[0][0].as_ref(), "p2");
        assert_eq!(grandchild.lines[COLUMNS].names[1][0].as_ref(), "p3");
        assert_eq!(
            grandchild.lines[COLUMNS].area_names[0][0].as_ref(),
            "panel-start"
        );
        assert_eq!(
            grandchild.lines[COLUMNS].area_names[1][0].as_ref(),
            "panel-end"
        );
    }
}
