use std::{cell::Cell, cmp::Ordering, rc::Rc};

/// A traversal position, with retained lanes occupying disjoint ordered subtrees.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PaintOrder {
    prefix: Option<Rc<[u32]>>,
    index: u32,
}

impl PaintOrder {
    pub const FIRST: Self = Self {
        prefix: None,
        index: 0,
    };

    pub fn descendants(&self) -> PaintOrders {
        PaintOrders {
            prefix: Some(self.path().collect()),
            next: Rc::default(),
        }
    }

    fn path(&self) -> impl Iterator<Item = u32> + '_ {
        self.prefix
            .as_deref()
            .unwrap_or_default()
            .iter()
            .copied()
            .chain([self.index])
    }
}

impl Ord for PaintOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        self.path().cmp(other.path())
    }
}

impl PartialOrd for PaintOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Default)]
pub(super) struct PaintOrders {
    prefix: Option<Rc<[u32]>>,
    next: Rc<Cell<u32>>,
}

impl PaintOrders {
    pub fn commit(&self, candidate: &Self) {
        debug_assert_eq!(self.prefix, candidate.prefix);
        self.next.set(self.next.get().max(candidate.next.get()));
    }

    pub fn snapshot(&self) -> Self {
        Self {
            prefix: self.prefix.clone(),
            next: Rc::new(Cell::new(self.next.get())),
        }
    }

    pub fn take(&self) -> PaintOrder {
        let next = self.next.get() + 1;
        self.next.set(next);
        PaintOrder {
            prefix: self.prefix.clone(),
            index: next,
        }
    }

    pub fn branch_index(&self, order: &PaintOrder) -> Option<u32> {
        let mut path = order.path();
        for expected in self.prefix.as_deref().unwrap_or_default() {
            if path.next() != Some(*expected) {
                return None;
            }
        }
        path.next()
    }

    pub fn branch(&self) -> Self {
        self.take().descendants()
    }
}

#[derive(Default)]
pub(super) struct OrderSnapshots {
    counters: std::collections::HashMap<*const Cell<u32>, PaintOrders>,
}
impl OrderSnapshots {
    pub fn detach(&mut self, orders: &mut PaintOrders) {
        let key = Rc::as_ptr(&orders.next);
        *orders = self
            .counters
            .entry(key)
            .or_insert_with(|| orders.snapshot())
            .clone();
    }
    pub fn len(&self) -> usize {
        self.counters.len()
    }
}
