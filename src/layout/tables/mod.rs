mod captions;
mod collapse;
mod intrinsic;
mod measure;
mod model;
mod paint;

pub(super) use captions::principal;
pub(super) use collapse::{BandPlacement, CollapsedGrid};
pub(super) use intrinsic::{content as intrinsic_content, measure as intrinsic};
pub(super) use measure::{
    CellAlignments, RowGeometry, TableContaining, TableGeometry, TablePlan, cell_height,
    collapsed_decoration, measure, measure_format, measure_placed, resume,
};

pub(super) use paint::{BackgroundRows, FragmentBackground};

pub(super) use model::Inputs;
