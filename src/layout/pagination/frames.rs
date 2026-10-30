//! Native flows paginate bottom-up before their paint joins the enclosing flow.
use super::{Effect, FLOW_LEVEL, MeasuredBlock, PaginationState, Paginator, PaintKey, PaintOrder};
use crate::graphics::Rect;
use crate::layout::{
    LayoutError,
    measure::{Failure, Measurer},
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

type RawFrame<'a> = PaginationState<'a, 'a, ()>;

#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) struct Key {
    frame: Rc<crate::layout::measure::FrameKey>,
    owner_geometry: Option<[u32; 4]>,
}
impl Key {
    pub(super) fn of(root: &Rc<MeasuredBlock<'_>>) -> Self {
        Self {
            frame: root
                .frame_key
                .clone()
                .expect("native frames retain their measurement key"),
            owner_geometry: root.frame_owner.as_ref().map(|owner| {
                let rect = owner.containing;
                [rect.x, rect.y, rect.width, rect.height].map(f32::to_bits)
            }),
        }
    }
}

#[derive(Default)]
pub(in crate::layout) struct Templates<'a> {
    pub(super) completed: HashMap<Key, RawFrame<'a>>,
}

enum RawOutcome<'a> {
    Complete(Box<RawFrame<'a>>),
    Needs(Vec<Rc<MeasuredBlock<'a>>>),
}

pub(super) fn prepare<'a>(
    measurer: &mut Measurer<'a>,
    root: &Rc<MeasuredBlock<'a>>,
    templates: &Rc<RefCell<Templates<'a>>>,
) -> Result<(), LayoutError> {
    measurer
        .run(|measurer| prepare_frames(measurer, root.clone(), templates).map_err(Failure::from))
}

fn prepare_frames<'a>(
    measurer: &mut Measurer<'a>,
    root: Rc<MeasuredBlock<'a>>,
    templates: &Rc<RefCell<Templates<'a>>>,
) -> Result<(), LayoutError> {
    let mut jobs = vec![root];
    while let Some(root) = jobs.pop() {
        let key = Key::of(&root);
        if templates.borrow().completed.contains_key(&key) {
            continue;
        }
        let dependencies: Vec<_> = crate::layout::measure::frame_dependencies(&root)
            .into_iter()
            .filter(|child| !templates.borrow().completed.contains_key(&Key::of(child)))
            .collect();
        if !dependencies.is_empty() {
            jobs.push(root);
            jobs.extend(dependencies.into_iter().rev());
            continue;
        }
        match render(measurer, &root, templates)? {
            RawOutcome::Complete(mut output) => {
                let mut cache = templates.borrow_mut();
                for child in std::mem::take(&mut output.frame_reads) {
                    cache.completed.remove(&child);
                }
                cache.completed.insert(key, *output);
            }
            RawOutcome::Needs(dependencies) => {
                jobs.push(root);
                jobs.extend(dependencies.into_iter().rev());
            }
        }
    }
    Ok(())
}

fn render<'a>(
    measurer: &mut Measurer<'a>,
    root: &MeasuredBlock<'a>,
    templates: &Rc<RefCell<Templates<'a>>>,
) -> Result<RawOutcome<'a>, LayoutError> {
    let previous = (
        measurer.raw_pagination,
        measurer.space,
        measurer.native_sizing.clone(),
    );
    measurer.raw_pagination = true;
    measurer.space = root
        .frame_key
        .as_ref()
        .expect("native frames retain their measurement key")
        .native_space();
    measurer.native_sizing = root
        .frame_key
        .as_ref()
        .and_then(|key| key.native_sizing())
        .map(|(vertical, scroll_container)| {
            Rc::new(crate::layout::measure::NativeSizing {
                source: std::ptr::from_ref(root.source),
                vertical,
                scroll_container,
            })
        });
    let result = (|| {
        let mut paginator = Paginator::for_frame(
            measurer,
            Rect {
                x: 0.0,
                y: 0.0,
                width: root.geometry.containing_width,
                height: root.height,
            },
        );
        paginator.template = true;
        if let Some(owner) = &root.frame_owner {
            paginator.containers[0].fixed = owner.source.style().contains_fixed();
            let node = paginator.effect_node(None, Effect::MotionReference(true, None));
            paginator.clips[node].paints.push((0, owner.paint.clone()));
            paginator.motion_root = Some((root.source, node));
        }
        paginator.root_overflow = Some(Default::default());
        paginator.frame_templates = templates.clone();
        paginator.parent_3d = Some(paginator.effect_node(None, Effect::InheritedParent3d));
        let mut content = root.clone();
        content.independent = false;
        paginator.pending.push(content.before.clone());
        paginator.block(content, super::BreakTail::default())?;
        if let Some(owner) = &root.frame_owner
            && (owner.source.style().contains_absolute() || owner.source.style().contains_fixed())
        {
            loop {
                let pending = std::mem::take(&mut paginator.containers[0].pending);
                if pending.is_empty() {
                    break;
                }
                for pending in pending {
                    let rect = if let Some(grid) = &root.grid {
                        let overflow = paginator
                            .root_overflow
                            .as_ref()
                            .and_then(|overflow| overflow.get(&0))
                            .copied();
                        paginator.measurer.run(|measurer| {
                            grid.area(
                                owner.containing,
                                owner.padding,
                                overflow,
                                &pending.block.style().grid.placement,
                                measurer,
                            )
                        })?
                    } else {
                        owner.containing
                    };
                    paginator.place(
                        pending,
                        &super::Containing::Rect {
                            rect,
                            paint: Box::new(owner.paint.clone()),
                            radius: None,
                            flow: crate::style::logical::Flow::used(owner.source.style()),
                            clip: None,
                        },
                    )?;
                }
            }
        }
        if !paginator.frame_requests.is_empty() {
            return Ok(RawOutcome::Needs(paginator.frame_requests));
        }
        paginator.assemble_sliced_boxes();
        paginator.complete_column_motion()?;
        paginator.position_canvas_backgrounds();
        let mut output = paginator.with_measurer(());
        output.frame_templates = Rc::default();
        output.template = false;
        Ok(RawOutcome::Complete(Box::new(output)))
    })();
    (
        measurer.raw_pagination,
        measurer.space,
        measurer.native_sizing,
    ) = previous;
    result
}

pub(super) fn payload_work(output: &RawFrame<'_>) -> usize {
    let paints = output
        .display_lists
        .iter()
        .chain(&output.foreground)
        .chain(output.underlays.values())
        .flatten()
        .map(|paint| paint.scope.len().saturating_add(1));
    let effects = output.clips.iter().map(|node| {
        node.fragments
            .len()
            .saturating_add(node.areas.len())
            .saturating_add(node.paints.len())
            .saturating_add(node.context.as_ref().map_or(0, |key| key.len()))
            .saturating_add(usize::from(matches!(
                node.effect,
                Effect::Transform(_, _, Some(_))
            )))
            .saturating_add(1)
    });
    let lines = output.pages.iter().flatten().map(|line| {
        line.runs
            .iter()
            .map(|run| run.glyphs().len())
            .fold(line.text.len().saturating_add(1), usize::saturating_add)
    });
    let pending = output
        .containers
        .iter()
        .flat_map(|c| &c.pending)
        .chain(&output.fixed)
        .map(|pending| pending.context.len().saturating_add(1));
    paints
        .chain(effects)
        .chain(lines)
        .chain(pending)
        .chain(
            output
                .column_motion_overflow
                .iter()
                .map(|pending| pending.transforms.len().saturating_add(1)),
        )
        .chain([output.anchors.payload_work()])
        .fold(1, usize::saturating_add)
}

pub(super) fn rebase(
    output: &mut RawFrame<'_>,
    context: &PaintKey,
    flow: &PaintKey,
    lane: &PaintOrder,
) {
    let key = |source: &PaintKey| -> PaintKey {
        let (prefix, rest) = if source.first() == Some(&FLOW_LEVEL) {
            (flow, &source[1..])
        } else {
            (context, source.as_ref())
        };
        prefix
            .iter()
            .cloned()
            .chain(rest.iter().map(|(z, order)| (*z, order.within(lane))))
            .collect()
    };
    for paint in output
        .display_lists
        .iter_mut()
        .chain(&mut output.foreground)
        .chain(output.underlays.values_mut())
        .flatten()
    {
        paint.scope = key(&paint.scope);
        paint.order = paint.order.within(lane);
    }
    for node in &mut output.clips {
        node.context = node.context.as_ref().map(key);
    }
    for pending in output
        .containers
        .iter_mut()
        .flat_map(|c| &mut c.pending)
        .chain(&mut output.fixed)
    {
        pending.context = key(&pending.context);
        pending.order = pending.order.within(lane);
        pending.orders = pending.orders.within(lane);
    }
}
