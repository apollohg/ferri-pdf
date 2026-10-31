use super::{MeasuredBlock, Measurer};
use crate::document::BlockBox;
use crate::layout::measure::Failure;
use crate::layout::{breaks::Fragmentainer, geometry::HorizontalBox};

const MAX_ITEM_METRICS: usize = 4096;

#[derive(Clone)]
pub(crate) struct ItemMetrics {
    pub height: f32,
    pub baseline: Option<f32>,
    pub last_baseline: Option<f32>,
    pub grid_baselines: Option<std::rc::Rc<[crate::layout::grid::BaselineContribution]>>,
    pub grid_rows: Option<std::rc::Rc<[crate::grid::SizingItem]>>,
}
impl ItemMetrics {
    pub(crate) fn of(block: &MeasuredBlock<'_>) -> Self {
        Self {
            height: block.height,
            // CSS Containment 2 §3.2: a layout containment box has no baseline.
            baseline: block
                .first_baseline()
                .filter(|_| !block.style.containment().layout),
            last_baseline: block
                .last_baseline()
                .filter(|_| !block.style.containment().layout),
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
        self.measured_from_start(self.baseline, reverse)
    }
    /// The last baseline, synthesized from the border box's line-under edge where the box
    /// has none (CSS Box Alignment 3 §9.1).
    pub(crate) fn last_baseline_from_start(&self, reverse: bool) -> f32 {
        self.measured_from_start(self.last_baseline, reverse)
    }
    fn measured_from_start(&self, baseline: Option<f32>, reverse: bool) -> f32 {
        let baseline = baseline.unwrap_or(self.height);
        if reverse {
            self.height - baseline
        } else {
            baseline
        }
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct ColumnKey {
    x: u32,
    width: u32,
    rtl: bool,
    height: Option<u32>,
    column_fragment: bool,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct OrthogonalColumnKey {
    source: usize,
    inline_size: u32,
    count: usize,
    width: u32,
    gap: u32,
    x: u32,
    rtl: bool,
    length: Option<u32>,
    principal: bool,
    block_auto: bool,
    content_max: u32,
    minimum: u32,
    maximum: Option<u32>,
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub(crate) struct MetricsKey {
    native_sizing: Option<(crate::layout::geometry::VerticalKey, bool)>,
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
    sized: bool,
    unbroken: bool,
    named_pages: bool,
    orthogonal_columns: Option<OrthogonalColumnKey>,
    space: super::space::SpaceKey,
}

impl MetricsKey {
    pub(super) fn new(
        measurer: &Measurer<'_>,
        block: &BlockBox,
        geometry: &HorizontalBox,
        height: Option<f32>,
        containing_height: Option<f32>,
    ) -> Self {
        Self {
            native_sizing: measurer
                .native_sizing
                .as_ref()
                .filter(|sizing| std::ptr::eq(sizing.source, block))
                .map(|sizing| (sizing.vertical.key(), sizing.scroll_container)),
            space: measurer.space.key(),
            grid_parent: measurer
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
            page_height: measurer.page_height.to_bits(),
            column_fragment: measurer.fragmentainer == Fragmentainer::Column,
            column: measurer.column_flow.map(|flow| ColumnKey {
                x: flow.x.to_bits(),
                width: flow.width.to_bits(),
                rtl: flow.rtl,
                height: flow.containing_height.map(f32::to_bits),
                column_fragment: flow.fragmentainer == Fragmentainer::Column,
            }),
            spanner: measurer.spanner.map(|block| block as usize),
            sized: measurer.sized_by_container(block),
            unbroken: measurer.unbroken,
            named_pages: measurer.named_pages,
            orthogonal_columns: measurer
                .orthogonal_columns
                .filter(|columns| columns.matches(block))
                .map(|columns| OrthogonalColumnKey {
                    source: columns.source as usize,
                    inline_size: columns.geometry.inline_size.to_bits(),
                    count: columns.geometry.count,
                    width: columns.geometry.width.to_bits(),
                    gap: columns.geometry.gap.to_bits(),
                    x: columns.geometry.x.to_bits(),
                    rtl: columns.geometry.rtl,
                    length: columns.length.map(f32::to_bits),
                    principal: columns.principal,
                    block_auto: columns.block_auto,
                    content_max: columns.content_max.to_bits(),
                    minimum: columns.minimum.to_bits(),
                    maximum: columns.maximum.map(f32::to_bits),
                }),
        }
    }
}

impl Measurer<'_> {
    pub(crate) fn item_metrics<'a>(
        &mut self,
        block: &'a BlockBox,
        geometry: &HorizontalBox,
        height: Option<f32>,
        containing_height: Option<f32>,
        measure: impl FnOnce(&mut Self) -> Result<MeasuredBlock<'a>, Failure>,
    ) -> Result<ItemMetrics, Failure> {
        let key = match self.cached_metrics(block, geometry, height, containing_height) {
            CachedMetrics::Hit(metrics) => return Ok(metrics),
            CachedMetrics::Miss(key) => key,
        };
        let measured = measure(self);
        self.store_metrics(key, measured)
    }

    /// The metrics measured before for `block` in the same context, or the key to keep
    /// them by, unless they are not kept. Kept off the frames that nested items' measurements
    /// stack.
    #[inline(never)]
    fn cached_metrics(
        &self,
        block: &BlockBox,
        geometry: &HorizontalBox,
        height: Option<f32>,
        containing_height: Option<f32>,
    ) -> CachedMetrics {
        // Clearance probes carry transient source-specific placement constraints.
        if !self.clearance_targets.is_empty() {
            return CachedMetrics::Miss(None);
        }
        let key = MetricsKey::new(self, block, geometry, height, containing_height);
        match self.item_metrics.get(&key) {
            Some(metrics) => CachedMetrics::Hit(metrics.clone()),
            None => CachedMetrics::Miss(Some(Box::new(key))),
        }
    }

    /// The metrics of the item `measured`, kept by `key` when there is one.
    #[inline(never)]
    fn store_metrics(
        &mut self,
        key: Option<Box<MetricsKey>>,
        measured: Result<MeasuredBlock<'_>, Failure>,
    ) -> Result<ItemMetrics, Failure> {
        let metrics = ItemMetrics::of(&measured?);
        if let Some(key) = key {
            if self.item_metrics.len() >= MAX_ITEM_METRICS {
                self.item_metrics.clear();
            }
            self.item_metrics.insert(*key, metrics.clone());
        }
        Ok(metrics)
    }
}

/// Item metrics measured before, or the key to keep them by once measured.
enum CachedMetrics {
    Hit(ItemMetrics),
    Miss(Option<Box<MetricsKey>>),
}
