//! Native flows paginate bottom-up before their paint joins the enclosing flow.
use super::{Effect, FLOW_LEVEL, MeasuredBlock, PaginationState, Paginator, PaintKey, PaintOrder};
use crate::graphics::Rect;
use crate::layout::{LayoutError, measure::Measurer};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

type RawFrame<'a> = PaginationState<'a, 'a, ()>;

#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) struct Key(Rc<crate::layout::measure::FrameKey>);
impl Key {
    pub(super) fn of(root: &Rc<MeasuredBlock<'_>>) -> Self {
        Self(
            root.frame_key
                .clone()
                .expect("native frames retain their measurement key"),
        )
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
    let previous = (measurer.raw_pagination, measurer.space);
    measurer.raw_pagination = true;
    measurer.space = root
        .frame_key
        .as_ref()
        .expect("native frames retain their measurement key")
        .native_space();
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
        paginator.root_overflow = Some(Default::default());
        paginator.frame_templates = templates.clone();
        paginator.parent_3d = Some(paginator.effect_node(None, Effect::InheritedParent3d));
        let mut root = root.clone();
        root.independent = false;
        paginator.pending.push(root.before.clone());
        paginator.block(root, super::BreakTail::default())?;
        if !paginator.frame_requests.is_empty() {
            return Ok(RawOutcome::Needs(paginator.frame_requests));
        }
        paginator.assemble_sliced_boxes();
        paginator.position_canvas_backgrounds();
        let mut output = paginator.with_measurer(());
        output.frame_templates = Rc::default();
        output.template = false;
        Ok(RawOutcome::Complete(Box::new(output)))
    })();
    (measurer.raw_pagination, measurer.space) = previous;
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
