use super::{MeasuredBlock, Measurer};
use crate::layout::{LayoutError, breaks::Fragmentainer, geometry::HorizontalBox};
use ferri_pdf_document::BlockBox;

const MAX_ITEM_METRICS: usize = 4096;

#[derive(Clone)]
pub(crate) struct ItemMetrics {
    pub height: f32,
    pub baseline: Option<f32>,
    pub grid_baselines: Option<std::rc::Rc<[crate::layout::grid::BaselineContribution]>>,
    pub grid_rows: Option<std::rc::Rc<[crate::grid::SizingItem]>>,
}
impl ItemMetrics {
    pub(crate) fn of(block: &MeasuredBlock<'_>) -> Self {
        Self {
            height: block.height,
            baseline: block.first_baseline(),
            grid_baselines: block
                .grid
                .as_ref()
                .and_then(|grid| grid.baseline_contributions.clone()),
            grid_rows: block
                .grid
                .as_ref()
                .and_then(|grid| grid.row_contributions.clone()),
        }
    }
    pub(crate) fn baseline_from_start(&self, reverse: bool) -> f32 {
        let baseline = self.baseline.unwrap_or(self.height);
        if reverse {
            self.height - baseline
        } else {
            baseline
        }
    }
}

#[derive(Hash, PartialEq, Eq)]
struct ColumnKey {
    x: u32,
    width: u32,
    rtl: bool,
    height: Option<u32>,
    column_fragment: bool,
}

#[derive(Hash, PartialEq, Eq)]
pub(crate) struct MetricsKey {
    block: usize,
    grid_parent: Option<crate::layout::grid::ParentContext>,
    x: u32,
    width: u32,
    parent_x: u32,
    parent_width: u32,
    parent_rtl: bool,
    padding: [u32; 4],
    border: [u32; 4],
    margin: [Option<u32>; 4],
    height: Option<u32>,
    containing_height: Option<u32>,
    page_height: u32,
    column_fragment: bool,
    column: Option<ColumnKey>,
    spanner: Option<usize>,
    unbroken: bool,
    named_pages: bool,
}

impl Measurer<'_> {
    pub(crate) fn item_metrics<'a>(
        &mut self,
        block: &'a BlockBox,
        geometry: &HorizontalBox,
        height: Option<f32>,
        containing_height: Option<f32>,
        measure: impl FnOnce(&mut Self) -> Result<MeasuredBlock<'a>, LayoutError>,
    ) -> Result<ItemMetrics, LayoutError> {
        // Clearance probes carry transient source-specific placement constraints.
        if !self.clearance_targets.is_empty() {
            return measure(self).map(|block| ItemMetrics::of(&block));
        }
        let key = MetricsKey {
            grid_parent: self
                .grid_parent
                .as_ref()
                .filter(|parent| parent.matches(block))
                .cloned(),
            block: std::ptr::from_ref(block) as usize,
            x: geometry.x.to_bits(),
            width: geometry.width.to_bits(),
            parent_x: geometry.containing_x.to_bits(),
            parent_width: geometry.containing_width.to_bits(),
            parent_rtl: geometry.containing_rtl,
            padding: geometry.padding.map(f32::to_bits),
            border: geometry.border.map(f32::to_bits),
            margin: geometry.margin.map(|value| value.map(f32::to_bits)),
            height: height.map(f32::to_bits),
            containing_height: containing_height.map(f32::to_bits),
            page_height: self.page_height.to_bits(),
            column_fragment: self.fragmentainer == Fragmentainer::Column,
            column: self.column_flow.map(|flow| ColumnKey {
                x: flow.x.to_bits(),
                width: flow.width.to_bits(),
                rtl: flow.rtl,
                height: flow.containing_height.map(f32::to_bits),
                column_fragment: flow.fragmentainer == Fragmentainer::Column,
            }),
            spanner: self.spanner.map(|block| block as usize),
            unbroken: self.unbroken,
            named_pages: self.named_pages,
        };
        if let Some(metrics) = self.item_metrics.get(&key) {
            return Ok(metrics.clone());
        }
        let metrics = ItemMetrics::of(&measure(self)?);
        if self.item_metrics.len() >= MAX_ITEM_METRICS {
            self.item_metrics.clear();
        }
        self.item_metrics.insert(key, metrics.clone());
        Ok(metrics)
    }
}
