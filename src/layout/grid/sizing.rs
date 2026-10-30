use crate::layout::geometry::SizeConstraints;
use crate::style::{COLUMNS, Style};

pub(super) fn shared(style: &Style, axis: usize) -> bool {
    style.display.is_grid() && style.grid.templates[axis].subgrid.is_some()
}

pub(super) fn placement(style: &Style) -> crate::grid::ItemPlacement {
    use crate::style::{GridLine, MAX_GRID_LINE, ROWS};
    let mut placement = style.grid.placement.clone();
    for axis in [ROWS, COLUMNS] {
        if !shared(style, axis) {
            continue;
        }
        let lines = &style.grid.templates[axis].subgrid.as_ref().unwrap().lines;
        let count = lines
            .len()
            .saturating_sub(1)
            .max(
                style
                    .grid
                    .areas
                    .as_ref()
                    .map_or(0, |areas| areas.size[axis]),
            )
            .clamp(1, MAX_GRID_LINE as usize) as u32;
        let [start, end] = &mut placement[axis];
        match (&*start, &*end) {
            (GridLine::Span(..), _) | (_, GridLine::Span(..)) => {}
            (_, GridLine::Auto) => *end = GridLine::Span(count, None),
            (GridLine::Auto, _) => *start = GridLine::Span(count, None),
            _ => {}
        }
    }
    placement
}

pub(super) fn constraints(
    style: &Style,
    axis: usize,
    basis: Option<f32>,
    edges: f32,
    inherited: bool,
) -> SizeConstraints {
    let sizes = if axis == COLUMNS {
        [&style.width, &style.min_width, &style.max_width]
    } else {
        [&style.height, &style.min_height, &style.max_height]
    };
    if inherited {
        return SizeConstraints::default();
    }
    SizeConstraints::new(sizes, basis, style.box_sizing, edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Display, LengthPercentage, ROWS, Size, Subgrid, TrackList};
    use std::sync::Arc;

    #[test]
    fn automatic_subgrid_placement_uses_local_tracks_without_overriding_explicit_spans() {
        use crate::style::GridLine;
        let mut style = Style::default();
        style.display = Display::Grid;
        style.grid.templates[COLUMNS] = Arc::new(TrackList {
            subgrid: Some(Subgrid {
                lines: vec![vec![]; 3],
                repeat: None,
            }),
            ..Default::default()
        });
        for (specified, expected) in [
            (
                [GridLine::Auto, GridLine::Auto],
                [GridLine::Auto, GridLine::Span(2, None)],
            ),
            (
                [GridLine::Line(3, None), GridLine::Auto],
                [GridLine::Line(3, None), GridLine::Span(2, None)],
            ),
            (
                [GridLine::Auto, GridLine::Line(5, None)],
                [GridLine::Span(2, None), GridLine::Line(5, None)],
            ),
            (
                [GridLine::Auto, GridLine::Span(1, None)],
                [GridLine::Auto, GridLine::Span(1, None)],
            ),
            (
                [GridLine::Line(1, None), GridLine::Line(4, None)],
                [GridLine::Line(1, None), GridLine::Line(4, None)],
            ),
        ] {
            style.grid.placement[COLUMNS] = specified;
            assert_eq!(placement(&style)[COLUMNS], expected);
        }
    }

    #[test]
    fn shared_axes_ignore_constraints_while_independent_and_standalone_axes_keep_them() {
        let mut style = Style::default();
        style.display = Display::Grid;
        style.grid.templates[COLUMNS] = Arc::new(TrackList {
            subgrid: Some(Subgrid::default()),
            ..Default::default()
        });
        let length = |value| Size::Value(LengthPercentage::Length(value));
        style.width = length(20.0);
        style.min_width = length(90.0);
        style.max_width = length(30.0);
        style.height = length(25.0);
        style.max_height = length(40.0);
        assert!(shared(&style, COLUMNS));
        assert!(!shared(&style, ROWS));
        let columns = constraints(&style, COLUMNS, Some(200.0), 10.0, true);
        assert_eq!(columns.specified, None);
        assert_eq!(
            columns.constrain(180.0),
            180.0,
            "shared tracks override min/max sizes"
        );
        let rows = constraints(&style, ROWS, Some(100.0), 10.0, false);
        assert_eq!(rows.specified, Some(25.0));
        assert_eq!(rows.constrain(80.0), 40.0);
        let standalone = constraints(&style, COLUMNS, Some(200.0), 10.0, false);
        assert_eq!(standalone.specified, Some(20.0));
        assert_eq!(standalone.constrain(180.0), 90.0);
        style.display = Display::Block;
        assert!(!shared(&style, COLUMNS));
    }
}
