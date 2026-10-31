//! Suspend native frame dependencies before measuring their parent again.
use super::{BlockSource, MeasuredBlock, Measurer, MetricsKey};
use crate::document::BlockBox;
use crate::layout::{LayoutError, geometry::HorizontalBox};
use std::collections::{HashMap, HashSet};

const MAX_CACHED_FRAMES: usize = 4096;

#[derive(Debug)]
pub(in crate::layout) enum Failure {
    Layout(Box<LayoutError>),
    FrameNeeded,
}
impl From<LayoutError> for Failure {
    fn from(error: LayoutError) -> Self {
        Self::Layout(Box::new(error))
    }
}
macro_rules! layout_failure {
    ($($error:ty),* $(,)?) => { $(impl From<$error> for Failure {
        fn from(error: $error) -> Self { Self::Layout(Box::new(error.into())) }
    })* };
}
layout_failure!(
    crate::style::StyleError,
    crate::document::DocumentError,
    crate::images::ImageError,
    crate::text::TextError,
    crate::table::TableError,
    crate::flex::FlexError,
    crate::grid::GridError,
    crate::layout::work::Exhausted,
);

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(in crate::layout) enum RequestKind {
    Writing,
    Math(crate::layout::mathml::LayoutKey),
    Css,
}

#[derive(Default)]
pub(in crate::layout) struct Batch(bool);
impl Batch {
    pub(in crate::layout) fn collect<T>(
        &mut self,
        result: Result<T, Failure>,
    ) -> Result<Option<T>, Failure> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(Failure::FrameNeeded) => {
                self.0 = true;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
    pub(in crate::layout) fn finish(self) -> Result<(), Failure> {
        if self.0 {
            Err(Failure::FrameNeeded)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub(in crate::layout) struct FrameKey {
    kind: RequestKind,
    native_sizing: Option<(crate::layout::geometry::VerticalKey, bool)>,
    metrics: MetricsKey,
    native_space: super::space::SpaceKey,
    style: usize,
    clearance: Vec<(usize, usize)>,
}
impl FrameKey {
    pub(in crate::layout) fn native_sizing(
        &self,
    ) -> Option<(crate::layout::geometry::VerticalBox, bool)> {
        self.native_sizing
            .as_ref()
            .map(|(vertical, scroll)| (vertical.vertical(), *scroll))
    }
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
        Self::for_kind(
            measurer,
            block,
            geometry,
            containing,
            height,
            RequestKind::Writing,
        )
    }
    pub(in crate::layout) fn for_kind(
        measurer: &Measurer<'_>,
        block: BlockSource<'_>,
        geometry: &HorizontalBox,
        containing: Option<f32>,
        height: Option<f32>,
        kind: RequestKind,
    ) -> Self {
        let mut clearance: Vec<_> = measurer
            .clearance_targets
            .iter()
            .map(|(source, target)| (*source as usize, *target))
            .collect();
        clearance.sort_unstable();
        Self {
            kind,
            native_sizing: (kind == RequestKind::Writing
                && block.layout_flow().writing_mode.vertical()
                    == block
                        .flow()
                        .expect("native keys have a source flow")
                        .style()
                        .used_writing_mode()
                        .vertical())
            .then(|| {
                (
                    crate::layout::geometry::VerticalBox::new(block.style(), geometry, containing)
                        .with_used_height(height)
                        .native_root()
                        .key(),
                    block.style().is_scroll_container(),
                )
            }),
            metrics: MetricsKey::new(measurer, block.block, geometry, height, containing),
            native_space: if kind == RequestKind::Writing {
                crate::layout::writing::native_space(
                    measurer.space,
                    block,
                    geometry,
                    containing,
                    height,
                )
            } else if matches!(kind, RequestKind::Math(_)) {
                measurer.space.enter(block, geometry, containing, height)
            } else {
                measurer.space
            }
            .key(),
            style: std::ptr::from_ref(block.style()) as usize,
            clearance,
        }
    }
}

#[derive(Clone)]
struct Context {
    native_sizing: Option<std::rc::Rc<super::NativeSizing>>,
    page_height: f32,
    grid_parent: Option<crate::layout::grid::ParentContext>,
    clearance: HashMap<*const BlockBox, usize>,
    fragmentainer: crate::layout::breaks::Fragmentainer,
    column_flow: Option<crate::layout::multicol::ColumnFlow>,
    spanner: Option<*const BlockBox>,
    sized_item: Option<*const BlockBox>,
    unbroken: bool,
    named_pages: bool,
    column_measurement: bool,
    orthogonal_columns: Option<crate::layout::multicol::OrthogonalColumns>,
    space: super::FlowSpace,
}
impl Context {
    fn capture(measurer: &Measurer<'_>) -> Self {
        Self {
            native_sizing: measurer.native_sizing.clone(),
            page_height: measurer.page_height,
            grid_parent: measurer.grid_parent.clone(),
            clearance: measurer.clearance_targets.clone(),
            fragmentainer: measurer.fragmentainer,
            column_flow: measurer.column_flow,
            spanner: measurer.spanner,
            sized_item: measurer.sized_item,
            unbroken: measurer.unbroken,
            named_pages: measurer.named_pages,
            column_measurement: measurer.column_measurement,
            orthogonal_columns: measurer.orthogonal_columns,
            space: measurer.space,
        }
    }
    fn restore(&self, measurer: &mut Measurer<'_>) {
        measurer.native_sizing.clone_from(&self.native_sizing);
        measurer.page_height = self.page_height;
        measurer.grid_parent.clone_from(&self.grid_parent);
        measurer.clearance_targets.clone_from(&self.clearance);
        measurer.fragmentainer = self.fragmentainer;
        measurer.column_flow = self.column_flow;
        measurer.spanner = self.spanner;
        measurer.sized_item = self.sized_item;
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
    requests: Vec<Request<'a>>,
    queued: HashSet<FrameKey>,
    completed: HashMap<FrameKey, MeasuredBlock<'a>>,
    read: Vec<FrameKey>,
    depth: usize,
    protected: HashMap<FrameKey, usize>,
}
impl Measurements<'_> {
    fn protect(&mut self, read: &[FrameKey]) {
        for key in read {
            *self.protected.entry(key.clone()).or_default() += 1;
        }
    }
    fn release(&mut self, read: &[FrameKey]) {
        for key in read {
            let count = self
                .protected
                .get_mut(key)
                .expect("suspended reads remain protected");
            *count -= 1;
            if *count == 0 {
                self.protected.remove(key);
            }
        }
    }
}

struct Job<'a> {
    request: Option<Request<'a>>,
    dependencies: Vec<FrameKey>,
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
        self.request(source, geometry, containing, height, RequestKind::Writing)
    }
    pub(in crate::layout) fn request(
        &mut self,
        source: BlockSource<'a>,
        geometry: &HorizontalBox,
        containing: Option<f32>,
        height: Option<f32>,
        kind: RequestKind,
    ) -> Result<MeasuredBlock<'a>, Failure> {
        let key = FrameKey::for_kind(self, source, geometry, containing, height, kind);
        let mut frames = self.frame_measurements.borrow_mut();
        if let Some(measured) = frames.completed.get(&key).cloned() {
            frames.read.push(key);
            return Ok(measured);
        }
        if !frames.queued.insert(key.clone()) {
            return Err(Failure::FrameNeeded);
        }
        frames.requests.push(Request {
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
        let (nested, outer_read, outer_requests) = {
            let mut frames = self.frame_measurements.borrow_mut();
            let nested = frames.depth > 0;
            frames.depth += 1;
            let read = std::mem::take(&mut frames.read);
            if nested {
                frames.protect(&read);
            }
            frames.queued.clear();
            (nested, read, std::mem::take(&mut frames.requests))
        };
        let result = self.drive(&context, &mut compute);
        context.restore(self);
        let mut frames = self.frame_measurements.borrow_mut();
        frames.depth -= 1;
        frames.queued = outer_requests
            .iter()
            .map(|request| request.key.clone())
            .collect();
        frames.requests = outer_requests;
        if nested {
            frames.release(&outer_read);
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
        let mut jobs = vec![Job {
            request: None,
            dependencies: Vec::new(),
        }];
        let result = 'drive: {
            while let Some(Job {
                request: current,
                dependencies,
            }) = jobs.pop()
            {
                {
                    let mut frames = self.frame_measurements.borrow_mut();
                    frames.release(&dependencies);
                    frames.read.clear();
                }
                let result = if let Some(request) = &current {
                    self.measure_request(request)
                        .map(|frame| Completed::Frame(Box::new(frame)))
                } else {
                    root_context.restore(self);
                    compute(self).map(Completed::Root)
                };
                match result {
                    Err(Failure::FrameNeeded) => self.suspend(&mut jobs, current),
                    Err(Failure::Layout(error)) => break 'drive Err(*error),
                    Ok(Completed::Root(value)) => break 'drive Ok(value),
                    Ok(Completed::Frame(measured)) => self.complete(
                        current.expect("a frame request was measured").key,
                        *measured,
                    ),
                }
            }
            unreachable!("the root measurement completes the driver")
        };
        let mut frames = self.frame_measurements.borrow_mut();
        for job in jobs {
            frames.release(&job.dependencies);
        }
        result
    }
    // The helpers below keep their locals off `drive`, whose frame each nested measurement
    // stacks.
    fn measure_request(&mut self, request: &Request<'a>) -> Result<MeasuredBlock<'a>, Failure> {
        request.context.restore(self);
        match request.key.kind {
            RequestKind::Writing => crate::layout::writing::measure(
                self,
                request.source,
                &request.geometry,
                request.containing,
                request.height,
            ),
            RequestKind::Math(layout) => crate::layout::mathml::measure(
                self,
                request.source,
                &request.geometry,
                request.containing,
                request.height,
                layout,
            ),
            RequestKind::Css => self.measure_format(
                request.source,
                request.geometry.clone(),
                true,
                request.containing,
                request.height,
            ),
        }
    }
    /// Suspends the `current` measurement behind the frames it requested.
    fn suspend(&mut self, jobs: &mut Vec<Job<'a>>, current: Option<Request<'a>>) {
        let (dependencies, requests) = {
            let mut frames = self.frame_measurements.borrow_mut();
            frames.queued.clear();
            let requests = std::mem::take(&mut frames.requests);
            let mut dependencies = std::mem::take(&mut frames.read);
            dependencies.extend(requests.iter().map(|request| request.key.clone()));
            frames.protect(&dependencies);
            (dependencies, requests)
        };
        jobs.push(Job {
            request: current,
            dependencies,
        });
        assert!(
            !requests.is_empty(),
            "a suspended measurement has pending dependencies"
        );
        jobs.extend(requests.into_iter().rev().map(|request| Job {
            request: Some(request),
            dependencies: Vec::new(),
        }));
    }
    /// Caches a measured frame, evicting unprotected frames past the cache bound.
    fn complete(&mut self, key: FrameKey, measured: MeasuredBlock<'a>) {
        let mut frames = self.frame_measurements.borrow_mut();
        frames.read.clear();
        if frames.completed.len() >= MAX_CACHED_FRAMES.saturating_add(frames.protected.len()) {
            let Measurements {
                completed,
                protected,
                ..
            } = &mut *frames;
            completed.retain(|key, _| protected.contains_key(key));
        }
        frames.completed.insert(key, measured);
    }
}
