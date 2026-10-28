//! Grid item placement and track sizing (CSS Grid 1 §8, §11), independent of text and pages.
mod placement;
mod sizing;
mod subgrid;
mod subgrid_sizing;
pub use subgrid::inherited_areas;
pub use subgrid_sizing::subgrid_contributions;

pub use placement::{AxisLines, ItemPlacement, Placement, place};
pub use sizing::{
    AvailableSpace, MaxSizing, MinSizing, SizeBounds, SizingItem, TrackSizing, size_tracks,
};

#[derive(Debug, thiserror::Error)]
pub enum GridError {
    #[error("invalid or non-finite grid geometry")]
    InvalidGeometry,
    #[error("grid item spans outside its {tracks} tracks")]
    InvalidSpan { tracks: usize },
}
