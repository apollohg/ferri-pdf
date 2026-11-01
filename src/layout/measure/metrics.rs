use super::{MeasuredBlock, Measurer};
use crate::document::BlockBox;
use crate::layout::measure::Failure;
use crate::layout::{breaks::Fragmentainer, geometry::HorizontalBox};

/// Items kept at most; they are all forgotten when there would be more.
const MAX_KEPT_ITEMS: usize = 1 << 16;
/// The work the layouts of the items kept took at most, which bounds the memory they hold;
/// they are all forgotten when it would be more.
const MAX_KEPT_LAYOUT_WORK: usize = 1 << 22;

/// The flex and grid items measured, by the context they were measured in: their metrics and,
/// once laid out to be placed, their layouts. Each level of a container's sizing measures the
/// items nested in it again; keeping them makes that work linear in the nesting.
#[derive(Default)]
pub(crate) struct KeptItems<'a> {
    kept: std::collections::HashMap<MetricsKey, KeptItem<'a>>,
    /// The work the layouts kept took.
    held: usize,
    /// The work of the layouts copied from those kept, which the measurements copying them
    /// took in place of laying them out.
    copied: usize,
}

struct KeptItem<'a> {
    metrics: ItemMetrics,
    layout: Option<MeasuredBlock<'a>>,
    /// The work the measurement took, the layouts it copied included.
    work: usize,
}

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
    raw_pagination: bool,
    column_measurement: bool,
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
            raw_pagination: measurer.raw_pagination,
            column_measurement: measurer.column_measurement,
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

impl<'a> Measurer<'a> {
    /// The metrics of the item `block` that `measure` measures in the context `geometry`,
    /// `height` and `containing_height` give it, as measured before in the same context.
    pub(crate) fn item_metrics(
        &mut self,
        block: &'a BlockBox,
        geometry: &HorizontalBox,
        height: Option<f32>,
        containing_height: Option<f32>,
        measure: impl FnOnce(&mut Self) -> Result<MeasuredBlock<'a>, Failure>,
    ) -> Result<ItemMetrics, Failure> {
        let key = match self.kept_metrics(block, geometry, height, containing_height) {
            Kept::Hit(metrics) => return Ok(*metrics),
            Kept::Miss(key) => key,
        };
        let start = self.item_work();
        let measured = measure(self)?;
        let metrics = ItemMetrics::of(&measured);
        self.keep_item(key, metrics.clone(), None, start);
        Ok(metrics)
    }

    /// The layout of the item `block` that `measure` lays out in the context `geometry`,
    /// `height` and `containing_height` give it, as laid out before in the same context.
    pub(crate) fn item_layout(
        &mut self,
        block: &'a BlockBox,
        geometry: &HorizontalBox,
        height: Option<f32>,
        containing_height: Option<f32>,
        measure: impl FnOnce(&mut Self) -> Result<MeasuredBlock<'a>, Failure>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let key = match self.kept_layout(block, geometry, height, containing_height) {
            Kept::Hit(measured) => return Ok(*measured),
            Kept::Miss(key) => key,
        };
        let start = self.item_work();
        let measured = measure(self)?;
        self.keep_item(key, ItemMetrics::of(&measured), Some(&measured), start);
        Ok(measured)
    }

    /// The work done and copied so far, from which an item's measurement takes its own.
    fn item_work(&self) -> usize {
        self.work
            .spent()
            .saturating_add(self.item_metrics.borrow().copied)
    }

    /// The key to keep the item by, unless it is not kept.
    fn item_key(
        &self,
        block: &BlockBox,
        geometry: &HorizontalBox,
        height: Option<f32>,
        containing_height: Option<f32>,
    ) -> Option<MetricsKey> {
        // Clearance probes carry transient source-specific placement constraints.
        self.clearance_targets
            .is_empty()
            .then(|| MetricsKey::new(self, block, geometry, height, containing_height))
    }

    /// The metrics measured before for `block` in the same context, or the key to keep them
    /// by. Kept off the frames that nested items' measurements stack.
    #[inline(never)]
    fn kept_metrics(
        &self,
        block: &BlockBox,
        geometry: &HorizontalBox,
        height: Option<f32>,
        containing_height: Option<f32>,
    ) -> Kept<ItemMetrics> {
        let key = self.item_key(block, geometry, height, containing_height);
        if let Some(item) = key.as_ref().and_then(|key| {
            self.item_metrics
                .borrow()
                .kept
                .get(key)
                .map(|item| item.metrics.clone())
        }) {
            self.work.start(block);
            return Kept::Hit(Box::new(item));
        }
        crate::work::count(crate::work::Work::ItemMeasurement, 1);
        Kept::Miss(key.map(Box::new))
    }

    /// A copy of the layout laid out before for `block` in the same context, or the key to
    /// keep it by. Kept off the frames that nested items' measurements stack.
    #[inline(never)]
    fn kept_layout(
        &self,
        block: &BlockBox,
        geometry: &HorizontalBox,
        height: Option<f32>,
        containing_height: Option<f32>,
    ) -> Kept<MeasuredBlock<'a>> {
        let key = self.item_key(block, geometry, height, containing_height);
        if let Some(key) = &key {
            let mut items = self.item_metrics.borrow_mut();
            if let Some((layout, work)) = items
                .kept
                .get(key)
                .and_then(|item| Some((item.layout.clone()?, item.work)))
            {
                items.copied = items.copied.saturating_add(work);
                self.work.start(block);
                return Kept::Hit(Box::new(layout));
            }
        }
        crate::work::count(crate::work::Work::ItemMeasurement, 1);
        Kept::Miss(key.map(Box::new))
    }

    /// Keeps the item's `metrics`, and its `layout` when it was laid out to be placed, by
    /// `key` when there is one; its measurement started when the work done and copied was
    /// `start`.
    #[inline(never)]
    fn keep_item(
        &self,
        key: Option<Box<MetricsKey>>,
        metrics: ItemMetrics,
        layout: Option<&MeasuredBlock<'a>>,
        start: usize,
    ) {
        let Some(key) = key else {
            return;
        };
        let work = self.item_work().saturating_sub(start);
        let held = if layout.is_some() { work } else { 0 };
        if held > MAX_KEPT_LAYOUT_WORK {
            return;
        }
        let mut items = self.item_metrics.borrow_mut();
        if items.kept.len() >= MAX_KEPT_ITEMS || items.held + held > MAX_KEPT_LAYOUT_WORK {
            items.kept.clear();
            items.held = 0;
        }
        let replaced = items.kept.insert(
            *key,
            KeptItem {
                metrics,
                layout: layout.cloned(),
                work,
            },
        );
        items.held += held;
        if let Some(replaced) = replaced.filter(|item| item.layout.is_some()) {
            items.held -= replaced.work;
        }
    }
}

/// What was kept for an item, or the key to keep it by once measured, unless it is not kept.
enum Kept<T> {
    Hit(Box<T>),
    Miss(Option<Box<MetricsKey>>),
}
