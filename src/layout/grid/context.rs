use super::{topology::Topology, tracks::TrackGeometry};
use crate::layout::{LayoutError, measure::Measurer};
use ferri_pdf_document::BlockBox;
use ferri_pdf_style::ROWS;
use std::{
    hash::{Hash, Hasher},
    rc::Rc,
};

/// Own the parent plan while cached measurements refer to its identity.
#[derive(Clone)]
pub(crate) struct ParentContext {
    target: usize,
    topology: Rc<Topology>,
    item: usize,
    tracks: [Option<Rc<TrackGeometry>>; 2],
    edges: [[f32; 2]; 2],
    baselines: Option<Rc<[f32]>>,
}
impl ParentContext {
    pub(super) fn new(target: &BlockBox, topology: Rc<Topology>, item: usize) -> Self {
        Self {
            target: std::ptr::from_ref(target) as usize,
            topology,
            item,
            tracks: [None, None],
            edges: [[0.0; 2]; 2],
            baselines: None,
        }
    }
    pub(super) fn with_geometry(
        mut self,
        tracks: [Option<Rc<TrackGeometry>>; 2],
        edges: [[f32; 2]; 2],
    ) -> Self {
        self.tracks = tracks;
        self.edges = edges;
        self
    }
    pub(super) fn with_baselines(mut self, baselines: Rc<[f32]>) -> Self {
        self.baselines = Some(baselines);
        self
    }
    pub(super) fn baseline_groups(&self, gap: f32) -> Option<Vec<f32>> {
        let baselines = self.baselines.as_ref()?;
        let area = self.topology.placement.areas[self.item][ROWS].clone();
        Some(
            baselines[area]
                .iter()
                .enumerate()
                .map(|(row, baseline)| {
                    baseline
                        - if row == 0 {
                            self.edges[ROWS][0]
                        } else {
                            (gap - self.gap(ROWS)) / 2.0
                        }
                })
                .collect(),
        )
    }
    pub(super) fn content_size(&self, axis: usize) -> Option<f32> {
        self.tracks[axis].as_ref().map(|tracks| {
            let area = tracks.area(self.topology.placement.areas[self.item][axis].clone());
            (area.end - area.start - self.edges[axis].iter().sum::<f32>()).max(0.0)
        })
    }
    pub(super) fn gap(&self, axis: usize) -> f32 {
        self.tracks[axis]
            .as_ref()
            .map_or(self.topology.intrinsic_gaps[axis], |tracks| tracks.gap())
    }
    pub(super) fn geometry(
        &self,
        axis: usize,
        gap: Option<f32>,
        reverse: bool,
    ) -> Result<Option<TrackGeometry>, LayoutError> {
        self.tracks[axis]
            .as_ref()
            .map(|tracks| {
                tracks.inherit(
                    self.topology.placement.areas[self.item][axis].clone(),
                    gap,
                    self.edges[axis],
                    reverse,
                )
            })
            .transpose()
    }
    pub(super) fn parent(&self) -> (&Topology, usize) {
        (&self.topology, self.item)
    }
    pub(crate) fn matches(&self, block: &BlockBox) -> bool {
        self.target == std::ptr::from_ref(block) as usize
    }
}
impl PartialEq for ParentContext {
    fn eq(&self, other: &Self) -> bool {
        self.target == other.target
            && self.item == other.item
            && Rc::ptr_eq(&self.topology, &other.topology)
            && match (&self.baselines, &other.baselines) {
                (Some(left), Some(right)) => Rc::ptr_eq(left, right),
                (None, None) => true,
                _ => false,
            }
            && self.edges.map(|axis| axis.map(f32::to_bits))
                == other.edges.map(|axis| axis.map(f32::to_bits))
            && self
                .tracks
                .iter()
                .zip(&other.tracks)
                .all(|(left, right)| match (left, right) {
                    (Some(left), Some(right)) => Rc::ptr_eq(left, right),
                    (None, None) => true,
                    _ => false,
                })
    }
}
impl Eq for ParentContext {}
impl Hash for ParentContext {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.target.hash(state);
        self.item.hash(state);
        self.baselines.as_ref().map(Rc::as_ptr).hash(state);
        Rc::as_ptr(&self.topology).hash(state);
        self.edges.map(|axis| axis.map(f32::to_bits)).hash(state);
        for tracks in &self.tracks {
            tracks.as_ref().map(Rc::as_ptr).hash(state);
        }
    }
}
impl Measurer<'_> {
    pub(crate) fn with_grid_parent<T>(
        &mut self,
        parent: Option<ParentContext>,
        measure: impl FnOnce(&mut Self) -> Result<T, LayoutError>,
    ) -> Result<T, LayoutError> {
        let previous = std::mem::replace(&mut self.grid_parent, parent);
        let result = measure(self);
        self.grid_parent = previous;
        result
    }
}
