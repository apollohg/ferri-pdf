use std::{cell::Cell, cmp::Ordering, collections::HashMap, rc::Rc};

/// A step of a traversal path below the steps before it, which the paths through it share:
/// placing a retained lane's positions under another lane shares their steps too, so that it
/// costs the steps the lane holds rather than the length of every path.
#[derive(Debug)]
struct Step {
    parent: Option<Rc<Step>>,
    index: u32,
    /// The steps of the path up to and including this one.
    depth: usize,
}

impl Step {
    fn below(parent: Option<Rc<Step>>, index: u32) -> Rc<Self> {
        let depth = depth(parent.as_ref()) + 1;
        Rc::new(Self {
            parent,
            index,
            depth,
        })
    }
}

fn depth(steps: Option<&Rc<Step>>) -> usize {
    steps.map_or(0, |step| step.depth)
}

/// The step of `path` at `depth`, with the steps before it, for a path holding `steps` and then
/// `last`, no shorter than `depth`.
fn step_at(steps: Option<&Rc<Step>>, last: u32, depth: usize) -> (u32, Option<&Rc<Step>>) {
    let mut node = steps;
    if self::depth(node) < depth {
        return (last, node);
    }
    while let Some(step) = node
        && step.depth > depth
    {
        node = step.parent.as_ref();
    }
    node.map_or((last, None), |step| (step.index, step.parent.as_ref()))
}

/// How the paths `own` and `other`, of equal depth, compare, comparing them from their ends
/// up to the steps they share.
fn compare_steps(mut own: Option<&Rc<Step>>, mut other: Option<&Rc<Step>>) -> Ordering {
    let mut order = Ordering::Equal;
    while let (Some(step), Some(other_step)) = (own, other) {
        if Rc::ptr_eq(step, other_step) {
            break;
        }
        order = step.index.cmp(&other_step.index).then(order);
        own = step.parent.as_ref();
        other = other_step.parent.as_ref();
    }
    order
}

/// How the path of `steps` and then `last` compares with that of `other_steps` and then
/// `other_last`, step by step from the start.
fn compare_paths(
    (steps, last): (Option<&Rc<Step>>, u32),
    (other_steps, other_last): (Option<&Rc<Step>>, u32),
) -> Ordering {
    let (length, other_length) = (depth(steps) + 1, depth(other_steps) + 1);
    let common = length.min(other_length);
    let (index, before) = step_at(steps, last, common);
    let (other_index, other_before) = step_at(other_steps, other_last, common);
    compare_steps(before, other_before)
        .then(index.cmp(&other_index))
        .then(length.cmp(&other_length))
}

/// A traversal position, with retained lanes occupying disjoint ordered subtrees.
#[derive(Clone, Debug, Default)]
pub(super) struct PaintOrder {
    prefix: Option<Rc<Step>>,
    index: u32,
}

impl PaintOrder {
    pub(super) const FIRST: Self = Self {
        prefix: None,
        index: 0,
    };

    pub(super) fn descendants(&self) -> PaintOrders {
        PaintOrders {
            prefix: Some(Step::below(self.prefix.clone(), self.index)),
            next: Rc::default(),
        }
    }
}

impl Ord for PaintOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_paths(
            (self.prefix.as_ref(), self.index),
            (other.prefix.as_ref(), other.index),
        )
    }
}

impl PartialOrd for PaintOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for PaintOrder {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for PaintOrder {}

#[derive(Clone, Default)]
pub(super) struct PaintOrders {
    prefix: Option<Rc<Step>>,
    next: Rc<Cell<u32>>,
}

impl PaintOrders {
    pub(super) fn commit(&self, candidate: &Self) {
        debug_assert!(
            depth(self.prefix.as_ref()) == depth(candidate.prefix.as_ref())
                && compare_steps(self.prefix.as_ref(), candidate.prefix.as_ref())
                    == Ordering::Equal
        );
        self.next.set(self.next.get().max(candidate.next.get()));
    }

    pub(super) fn snapshot(&self) -> Self {
        Self {
            prefix: self.prefix.clone(),
            next: Rc::new(Cell::new(self.next.get())),
        }
    }

    pub(super) fn take(&self) -> PaintOrder {
        let next = self.next.get() + 1;
        self.next.set(next);
        PaintOrder {
            prefix: self.prefix.clone(),
            index: next,
        }
    }

    /// The step right after these orders' prefix on the path of `order`, if it passes through
    /// the prefix.
    pub(super) fn branch_index(&self, order: &PaintOrder) -> Option<u32> {
        let prefix = depth(self.prefix.as_ref());
        if depth(order.prefix.as_ref()) < prefix {
            return None;
        }
        let (index, before) = step_at(order.prefix.as_ref(), order.index, prefix + 1);
        (compare_steps(before, self.prefix.as_ref()) == Ordering::Equal).then_some(index)
    }

    pub(super) fn branch(&self) -> Self {
        self.take().descendants()
    }
}

/// Places the positions of a retained lane, laid out apart, under the position `lane` holds
/// in the enclosing traversal. Each step it places once, shared by the positions through it.
pub(super) struct Rebase {
    lane: Rc<Step>,
    /// The placed copy of each step, by the step it copies, which it keeps alive so that no
    /// other step takes its address.
    placed: HashMap<*const Step, (Rc<Step>, Rc<Step>)>,
}

impl Rebase {
    pub(super) fn new(lane: &PaintOrder) -> Self {
        Self {
            lane: Step::below(lane.prefix.clone(), lane.index),
            placed: HashMap::new(),
        }
    }

    fn steps(&mut self, steps: Option<&Rc<Step>>) -> Rc<Step> {
        let mut unplaced = Vec::new();
        let mut placed = self.lane.clone();
        let mut node = steps;
        while let Some(step) = node {
            if let Some((_, copy)) = self.placed.get(&Rc::as_ptr(step)) {
                placed = copy.clone();
                break;
            }
            unplaced.push(step);
            node = step.parent.as_ref();
        }
        for step in unplaced.into_iter().rev() {
            crate::work::count(crate::work::Work::PlacedStep, 1);
            placed = Step::below(Some(placed), step.index);
            self.placed
                .insert(Rc::as_ptr(step), (step.clone(), placed.clone()));
        }
        placed
    }

    pub(super) fn order(&mut self, order: &PaintOrder) -> PaintOrder {
        PaintOrder {
            prefix: Some(self.steps(order.prefix.as_ref())),
            index: order.index,
        }
    }

    pub(super) fn orders(&mut self, orders: &PaintOrders) -> PaintOrders {
        PaintOrders {
            prefix: Some(self.steps(orders.prefix.as_ref())),
            next: Rc::new(Cell::new(orders.next.get())),
        }
    }
}

#[derive(Default)]
pub(super) struct OrderSnapshots {
    counters: HashMap<*const Cell<u32>, PaintOrders>,
}
impl OrderSnapshots {
    pub(super) fn detach(&mut self, orders: &mut PaintOrders) {
        let key = Rc::as_ptr(&orders.next);
        *orders = self
            .counters
            .entry(key)
            .or_insert_with(|| orders.snapshot())
            .clone();
    }
    pub(super) fn len(&self) -> usize {
        self.counters.len()
    }
}
