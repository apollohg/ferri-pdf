//! Suspend native frame dependencies before measuring their parent again.
use super::{BlockSource, MeasuredBlock, Measurer, MetricsKey};
use crate::document::BlockBox;
use crate::layout::{LayoutError, geometry::HorizontalBox};
use std::collections::HashMap;

#[derive(Debug)]
pub(in crate::layout) enum Failure {
    Layout(LayoutError),
    FrameNeeded,
}
impl From<LayoutError> for Failure {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}
macro_rules! layout_failure {
    ($($error:ty),* $(,)?) => { $(impl From<$error> for Failure {
        fn from(error: $error) -> Self { Self::Layout(error.into()) }
    })* };
}
layout_failure!(
    crate::style::StyleError,
    crate::document::DocumentError,
    crate::images::ImageError,
    crate::text::TextError,
    crate::table::TableError,
    crate::flex::FlexError,
    crate::grid::GridError
);

#[derive(Clone, Hash, PartialEq, Eq)]
pub(in crate::layout) struct FrameKey {
    metrics: MetricsKey,
    native_space: super::space::SpaceKey,
    style: usize,
    clearance: Vec<(usize, usize)>,
}
impl FrameKey {
    pub(in crate::layout) fn native_space(&self) -> super::FlowSpace {
        self.native_space.space()
    }
    pub(in crate::layout) fn new(
        measurer: &Measurer<'_>,
        block: BlockSource<'_>,
        geometry: &HorizontalBox,
        containing: Option<f32>,
        height: Option<f32>,
    ) -> Self {
        let mut clearance: Vec<_> = measurer
            .clearance_targets
            .iter()
            .map(|(source, target)| (*source as usize, *target))
            .collect();
        clearance.sort_unstable();
        Self {
            metrics: MetricsKey::new(measurer, block.block, geometry, height, containing),
            native_space: crate::layout::writing::native_space(
                measurer.space,
                block,
                geometry,
                containing,
                height,
            )
            .key(),
            style: std::ptr::from_ref(block.style()) as usize,
            clearance,
        }
    }
}

#[derive(Clone)]
struct Context {
    page_height: f32,
    grid_parent: Option<crate::layout::grid::ParentContext>,
    clearance: HashMap<*const BlockBox, usize>,
    fragmentainer: crate::layout::breaks::Fragmentainer,
    column_flow: Option<crate::layout::multicol::ColumnFlow>,
    spanner: Option<*const BlockBox>,
    unbroken: bool,
    named_pages: bool,
    column_measurement: bool,
    orthogonal_columns: Option<crate::layout::multicol::OrthogonalColumns>,
    space: super::FlowSpace,
}
impl Context {
    fn capture(measurer: &Measurer<'_>) -> Self {
        Self {
            page_height: measurer.page_height,
            grid_parent: measurer.grid_parent.clone(),
            clearance: measurer.clearance_targets.clone(),
            fragmentainer: measurer.fragmentainer,
            column_flow: measurer.column_flow,
            spanner: measurer.spanner,
            unbroken: measurer.unbroken,
            named_pages: measurer.named_pages,
            column_measurement: measurer.column_measurement,
            orthogonal_columns: measurer.orthogonal_columns,
            space: measurer.space,
        }
    }
    fn restore(&self, measurer: &mut Measurer<'_>) {
        measurer.page_height = self.page_height;
        measurer.grid_parent.clone_from(&self.grid_parent);
        measurer.clearance_targets.clone_from(&self.clearance);
        measurer.fragmentainer = self.fragmentainer;
        measurer.column_flow = self.column_flow;
        measurer.spanner = self.spanner;
        measurer.unbroken = self.unbroken;
        measurer.named_pages = self.named_pages;
        measurer.column_measurement = self.column_measurement;
        measurer.orthogonal_columns = self.orthogonal_columns;
        measurer.space = self.space;
    }
}
#[derive(Clone)]
struct Request<'a> {
    key: FrameKey,
    source: BlockSource<'a>,
    geometry: HorizontalBox,
    containing: Option<f32>,
    height: Option<f32>,
    context: Context,
}
#[derive(Default)]
pub(in crate::layout) struct Measurements<'a> {
    request: Option<Request<'a>>,
    completed: HashMap<FrameKey, MeasuredBlock<'a>>,
    read: Vec<FrameKey>,
    depth: usize,
    protected: HashMap<FrameKey, usize>,
}

enum Completed<'a, T> {
    Root(T),
    Frame(Box<MeasuredBlock<'a>>),
}

impl<'a> Measurer<'a> {
    pub(in crate::layout) fn frame(
        &mut self,
        source: BlockSource<'a>,
        geometry: &HorizontalBox,
        containing: Option<f32>,
        height: Option<f32>,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let key = FrameKey::new(self, source, geometry, containing, height);
        let mut frames = self.frame_measurements.borrow_mut();
        if let Some(measured) = frames.completed.get(&key).cloned() {
            frames.read.push(key);
            return Ok(measured);
        }
        frames.request = Some(Request {
            key,
            source,
            geometry: geometry.clone(),
            containing,
            height,
            context: Context::capture(self),
        });
        Err(Failure::FrameNeeded)
    }

    pub(in crate::layout) fn run<T>(
        &mut self,
        mut compute: impl FnMut(&mut Self) -> Result<T, Failure>,
    ) -> Result<T, LayoutError> {
        let context = Context::capture(self);
        let (nested, outer_read) = {
            let mut frames = self.frame_measurements.borrow_mut();
            let nested = frames.depth > 0;
            frames.depth += 1;
            let read = std::mem::take(&mut frames.read);
            if nested {
                for key in &read {
                    *frames.protected.entry(key.clone()).or_default() += 1;
                }
            }
            (nested, read)
        };
        let result = self.drive(&context, &mut compute);
        context.restore(self);
        let mut frames = self.frame_measurements.borrow_mut();
        frames.depth -= 1;
        if nested {
            for key in &outer_read {
                let count = frames
                    .protected
                    .get_mut(key)
                    .expect("enclosing reads were protected");
                *count -= 1;
                if *count == 0 {
                    frames.protected.remove(key);
                }
            }
            let inner_read = std::mem::replace(&mut frames.read, outer_read);
            frames.read.extend(inner_read);
        } else {
            *frames = Measurements::default();
        }
        result
    }
    fn drive<T>(
        &mut self,
        root_context: &Context,
        compute: &mut impl FnMut(&mut Self) -> Result<T, Failure>,
    ) -> Result<T, LayoutError> {
        let mut current: Option<Request<'a>> = None;
        let mut parents = Vec::new();
        loop {
            self.frame_measurements.borrow_mut().read.clear();
            let result = if let Some(request) = &current {
                request.context.restore(self);
                crate::layout::writing::measure(
                    self,
                    request.source,
                    &request.geometry,
                    request.containing,
                    request.height,
                )
                .map(|frame| Completed::Frame(Box::new(frame)))
            } else {
                root_context.restore(self);
                compute(self).map(Completed::Root)
            };
            match result {
                Err(Failure::FrameNeeded) => {
                    parents.push(current.take());
                    current = self.frame_measurements.borrow_mut().request.take();
                }
                Err(Failure::Layout(error)) => return Err(error),
                Ok(Completed::Root(value)) => return Ok(value),
                Ok(Completed::Frame(measured)) => {
                    let request = current.take().expect("a frame request was measured");
                    let mut frames = self.frame_measurements.borrow_mut();
                    for key in std::mem::take(&mut frames.read) {
                        if !frames.protected.contains_key(&key) {
                            frames.completed.remove(&key);
                        }
                    }
                    frames.completed.insert(request.key, *measured);
                    current = parents.pop().expect("every frame has a waiting caller");
                }
            }
        }
    }
}
