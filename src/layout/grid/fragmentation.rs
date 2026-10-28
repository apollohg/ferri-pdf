use crate::document::BlockBox;
use crate::style::BreakBetween;
use std::{collections::HashMap, ops::Range};

pub(crate) struct Row {
    pub area: Range<f32>,
    pub grows: bool,
    pub first_fragment: f32,
    pub kept_fragment: f32,
    pub edges: [BreakBetween; 2],
}

pub(crate) struct Rows {
    pub source_order: Vec<usize>,
    pub tracks: Vec<Row>,
    pub items: HashMap<*const BlockBox, Range<usize>>,
}

impl Row {
    pub(crate) fn required(&self, available: f32) -> f32 {
        if self.kept_fragment <= available {
            self.kept_fragment
        } else {
            self.first_fragment
        }
    }
}
impl Rows {
    pub(crate) fn resolve_keeps(&mut self) {
        for index in (0..self.tracks.len()).rev() {
            let mut required = self.tracks[index].first_fragment;
            if let Some(next) = self.tracks.get(index + 1)
                && self.tracks[index].edges[1].combine(next.edges[0]) == BreakBetween::Avoid
            {
                required = required
                    .max(next.area.start - self.tracks[index].area.start + next.kept_fragment);
            }
            self.tracks[index].kept_fragment = required;
        }
    }
}
