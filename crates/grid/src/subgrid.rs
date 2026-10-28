//! Parent line correspondence for CSS Grid 2 §9.
use crate::{AxisLines, GridError};
use ferri_pdf_style::{GridAreas, MAX_GRID_LINE, NamedArea, Subgrid};
use std::ops::Range;

fn span(area: &Range<i64>) -> Result<usize, GridError> {
    area.end
        .checked_sub(area.start)
        .filter(|span| *span > 0 && *span <= i64::from(MAX_GRID_LINE))
        .and_then(|span| usize::try_from(span).ok())
        .ok_or(GridError::InvalidGeometry)
}

impl AxisLines {
    /// Inherit explicit names only. Local names use the child's own line direction.
    pub fn inherit(
        &self,
        area: Range<i64>,
        reverse: bool,
        local: &Subgrid,
    ) -> Result<Self, GridError> {
        let tracks = span(&area)?;
        let names = (0..=tracks)
            .map(|line| {
                let parent = if reverse {
                    area.end - line as i64
                } else {
                    area.start + line as i64
                };
                let mut names = usize::try_from(parent)
                    .ok()
                    .and_then(|line| self.names.get(line))
                    .cloned()
                    .unwrap_or_default();
                names.extend(local.line_names(tracks, line).iter().cloned());
                names
            })
            .collect();
        Ok(Self {
            names,
            area_names: Vec::new(),
            subgridded: true,
        })
    }
}

/// Intersect named areas in both parent axes, then express them in child coordinates.
pub fn inherited_areas(
    parent: &GridAreas,
    area: &[Range<i64>; 2],
    reverse: [bool; 2],
) -> Result<GridAreas, GridError> {
    let size = [span(&area[0])?, span(&area[1])?];
    let areas = parent
        .areas
        .iter()
        .filter_map(|named| {
            let mut tracks = [0..0, 0..0];
            for axis in 0..2 {
                let start = (named.tracks[axis].start as i64).max(area[axis].start);
                let end = (named.tracks[axis].end as i64).min(area[axis].end);
                if end <= start {
                    return None;
                }
                tracks[axis] = if reverse[axis] {
                    (area[axis].end - end) as usize..(area[axis].end - start) as usize
                } else {
                    (start - area[axis].start) as usize..(end - area[axis].start) as usize
                };
            }
            Some(NamedArea {
                name: named.name.clone(),
                tracks,
            })
        })
        .collect();
    Ok(GridAreas { size, areas })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn local_auto_fill_uses_the_inherited_span_even_when_parent_direction_reverses() {
        let parent = AxisLines {
            names: (0..6)
                .map(|i| vec![Arc::from(format!("parent{i}"))])
                .collect(),
            area_names: vec![],
            subgridded: false,
        };
        let local = Subgrid {
            lines: vec![vec![Arc::from("start")], vec![Arc::from("end")]],
            repeat: Some(ferri_pdf_style::NameRepeat {
                at: 1,
                lines: vec![vec![Arc::from("a")], vec![Arc::from("b")]],
            }),
        };
        let inherited = parent.inherit(1..5, true, &local).unwrap();
        for (line, expected) in ["start", "a", "b", "end"].iter().enumerate() {
            assert_eq!(inherited.names[line][1].as_ref(), *expected);
            assert_eq!(
                inherited.names[line][0].as_ref(),
                format!("parent{}", 5 - line)
            );
        }
        assert_eq!(inherited.names[4], vec![Arc::from("parent1")]);
    }

    #[test]
    fn explicit_names_inherit_locally_reverse_and_exclude_area_derived_names() {
        let parent = AxisLines {
            names: ["outer", "first", "middle", "last", "end"]
                .map(|name| vec![Arc::from(name)])
                .to_vec(),
            area_names: vec![vec![Arc::from("panel-start")]; 5],
            subgridded: false,
        };
        let local = Subgrid {
            lines: vec![vec![Arc::from("local")], vec![], vec![Arc::from("inner")]],
            repeat: None,
        };
        let inherited = parent.inherit(1..4, false, &local).unwrap();
        assert_eq!(
            inherited.names,
            vec![
                vec![Arc::from("first"), Arc::from("local")],
                vec![Arc::from("middle")],
                vec![Arc::from("last"), Arc::from("inner")],
                vec![Arc::from("end")]
            ]
        );
        assert_eq!(inherited.area_names, [] as [Vec<Arc<str>>; 0]);
        assert!(inherited.subgridded);
        let nested = inherited.inherit(1..3, true, &Subgrid::default()).unwrap();
        assert_eq!(
            nested.names,
            vec![
                vec![Arc::from("end")],
                vec![Arc::from("last"), Arc::from("inner")],
                vec![Arc::from("middle")]
            ]
        );
    }

    #[test]
    fn implicit_parent_lines_have_only_the_locally_assigned_names() {
        let parent = AxisLines {
            names: vec![vec![Arc::from("edge")]],
            area_names: vec![],
            subgridded: false,
        };
        let inherited = parent
            .inherit(
                -1..1,
                false,
                &Subgrid {
                    lines: vec![vec![Arc::from("before")]],
                    repeat: None,
                },
            )
            .unwrap();
        assert_eq!(
            inherited.names,
            vec![vec![Arc::from("before")], vec![Arc::from("edge")], vec![]]
        );
        assert!(matches!(
            parent.inherit(2..2, false, &Subgrid::default()),
            Err(GridError::InvalidGeometry)
        ));
        assert!(matches!(
            parent.inherit(i64::MIN..i64::MAX, false, &Subgrid::default()),
            Err(GridError::InvalidGeometry)
        ));
    }

    #[test]
    fn inherited_areas_require_two_axis_overlap_and_clip_before_reversing() {
        let parent = GridAreas {
            size: [6, 5],
            areas: vec![
                NamedArea {
                    name: "panel".into(),
                    tracks: [2..5, 1..4],
                },
                NamedArea {
                    name: "outside".into(),
                    tracks: [0..1, 2..4],
                },
            ],
        };
        let inherited = inherited_areas(&parent, &[3..6, 2..5], [false, true]).unwrap();
        assert_eq!(inherited.size, [3, 3]);
        assert_eq!(inherited.areas.len(), 1);
        assert_eq!(inherited.areas[0].tracks, [0..2, 1..3]);
        assert_eq!(
            inherited.line_names(1, 1).collect::<Vec<_>>(),
            ["panel-start"]
        );
        assert_eq!(
            inherited.line_names(1, 3).collect::<Vec<_>>(),
            ["panel-end"]
        );
        let nested = inherited_areas(&inherited, &[0..1, 2..3], [false; 2]).unwrap();
        assert_eq!(nested.areas[0].tracks, [0..1, 0..1]);
        assert_eq!(nested.line_names(1, 0).collect::<Vec<_>>(), ["panel-start"]);
    }
}
