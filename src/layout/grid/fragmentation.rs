use crate::document::BlockBox;
use crate::style::BreakBetween;
use std::{collections::HashMap, ops::Range};

pub(crate) struct Row {
    pub area: Range<f32>,
    pub grows: bool,
    pub first_fragment: f32,
    /// The block size kept with the row's start at column-only breaks and at page breaks.
    pub kept_fragments: [f32; 2],
    pub edges: [BreakBetween; 2],
}

pub(crate) struct Rows {
    pub source_order: Vec<usize>,
    pub tracks: Vec<Row>,
    pub items: HashMap<*const BlockBox, Range<usize>>,
}

impl Row {
    /// The block size that must fit before a break that is a page break when `page_break` is
    /// set.
    pub(crate) fn required(&self, available: f32, page_break: bool) -> f32 {
        let kept = self.kept_fragments[usize::from(page_break)];
        if kept <= available {
            kept
        } else {
            self.first_fragment
        }
    }
}
impl Rows {
    pub(crate) fn resolve_keeps(&mut self) {
        for page_break in [false, true] {
            let slot = usize::from(page_break);
            for index in (0..self.tracks.len()).rev() {
                let mut required = self.tracks[index].first_fragment;
                if let Some(next) = self.tracks.get(index + 1)
                    && crate::layout::breaks::avoids(
                        self.tracks[index].edges[1].combine(next.edges[0]),
                        page_break,
                    )
                {
                    required = required.max(
                        next.area.start - self.tracks[index].area.start + next.kept_fragments[slot],
                    );
                }
                self.tracks[index].kept_fragments[slot] = required;
            }
        }
    }
}
