//! Collections that copies of a pagination state share, so that copying the state for each
//! row of columns costs nothing for the content already laid out: a change copies only the
//! page, chunk or path of the entry it changes.
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher, Hash};
use std::ops::{Index, IndexMut};
use std::rc::Rc;

#[cfg(test)]
mod tests;

/// Per-page lists: a copy costs one pointer per page.
#[derive(Clone)]
pub(super) struct PageLists<T>(Vec<Rc<Vec<T>>>);

impl<T> Default for PageLists<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T: Clone> PageLists<T> {
    /// Lists for one page, holding nothing.
    pub(super) fn first_page() -> Self {
        Self(vec![Rc::default()])
    }
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }
    pub(super) fn push(&mut self, list: Vec<T>) {
        self.0.push(Rc::new(list));
    }
    pub(super) fn iter(&self) -> impl DoubleEndedIterator<Item = &Vec<T>> + ExactSizeIterator {
        self.0.iter().map(Rc::as_ref)
    }
    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut Vec<T>> {
        self.0.iter_mut().map(Rc::make_mut)
    }
    /// The items of `page`'s list from `start` on, copying a shared list only when it has any.
    pub(super) fn items_from(&mut self, page: usize, start: usize) -> &mut [T] {
        if self.0[page].len() <= start {
            return &mut [];
        }
        &mut Rc::make_mut(&mut self.0[page])[start..]
    }
    /// The lists, copying only those another copy still shares.
    pub(super) fn into_lists(self) -> Vec<Vec<T>> {
        self.0.into_iter().map(Rc::unwrap_or_clone).collect()
    }
}

impl<T> Index<usize> for PageLists<T> {
    type Output = Vec<T>;
    fn index(&self, page: usize) -> &Vec<T> {
        &self.0[page]
    }
}

impl<T: Clone> IndexMut<usize> for PageLists<T> {
    fn index_mut(&mut self, page: usize) -> &mut Vec<T> {
        Rc::make_mut(&mut self.0[page])
    }
}

impl<'l, T: Clone> IntoIterator for &'l PageLists<T> {
    type Item = &'l Vec<T>;
    type IntoIter = std::iter::Map<std::slice::Iter<'l, Rc<Vec<T>>>, fn(&Rc<Vec<T>>) -> &Vec<T>>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(Rc::as_ref)
    }
}

/// The items a log shares in each of its full chunks.
const LOG_CHUNK: usize = 64;

/// An append-only list: a copy shares the full chunks and copies the items after them.
#[derive(Clone)]
pub(super) struct SharedLog<T> {
    /// The full chunks, last first.
    full: Option<Rc<LogChunk<T>>>,
    /// The items after the full chunks, fewer than [`LOG_CHUNK`].
    tail: Vec<T>,
    len: usize,
}

struct LogChunk<T> {
    items: Vec<T>,
    previous: Option<Rc<LogChunk<T>>>,
}

impl<T> Default for SharedLog<T> {
    fn default() -> Self {
        Self {
            full: None,
            tail: Vec::new(),
            len: 0,
        }
    }
}

impl<T> SharedLog<T> {
    pub(super) fn push(&mut self, item: T) {
        self.tail.push(item);
        self.len += 1;
        if self.tail.len() == LOG_CHUNK {
            self.full = Some(Rc::new(LogChunk {
                items: std::mem::take(&mut self.tail),
                previous: self.full.take(),
            }));
        }
    }
    pub(super) fn len(&self) -> usize {
        self.len
    }
}

impl<T: Clone> SharedLog<T> {
    /// The items in the order they were pushed, copying those another log still shares.
    pub(super) fn into_vec(mut self) -> Vec<T> {
        let mut chunks = Vec::with_capacity(self.len / LOG_CHUNK);
        let mut next = self.full.take();
        while let Some(chunk) = next {
            next = match Rc::try_unwrap(chunk) {
                Ok(mut chunk) => {
                    let previous = chunk.previous.take();
                    chunks.push(chunk.items);
                    previous
                }
                Err(shared) => {
                    let mut chunk = Some(shared.as_ref());
                    while let Some(full) = chunk {
                        chunks.push(full.items.clone());
                        chunk = full.previous.as_deref();
                    }
                    None
                }
            };
        }
        let mut items = Vec::with_capacity(self.len);
        items.extend(chunks.into_iter().rev().flatten());
        items.append(&mut self.tail);
        items
    }
}

impl<T> Drop for SharedLog<T> {
    /// Drops the chunks only this log holds one by one, as dropping each from the one after it
    /// would nest as deep as the log is long.
    fn drop(&mut self) {
        let mut next = self.full.take();
        while let Some(chunk) = next {
            next = Rc::try_unwrap(chunk)
                .ok()
                .and_then(|mut chunk| chunk.previous.take());
        }
    }
}

/// The hash bits that choose a map node's child.
const MAP_BITS: u32 = 4;
const MAP_FANOUT: usize = 1 << MAP_BITS;
/// The entries a map leaf holds before it splits by the next hash bits.
const MAP_LEAF: usize = 8;

/// A hash map that keeps the first value inserted under each key: a copy shares every node,
/// and an insertion copies only the nodes on the path to its entry.
#[derive(Clone)]
pub(super) struct SharedMap<K, V> {
    root: Rc<MapNode<K, V>>,
    len: usize,
}

#[derive(Clone)]
enum MapNode<K, V> {
    Leaf(Vec<(K, V)>),
    Branch([Option<Rc<MapNode<K, V>>>; MAP_FANOUT]),
}

impl<K, V> Default for SharedMap<K, V> {
    fn default() -> Self {
        Self {
            root: Rc::new(MapNode::Leaf(Vec::new())),
            len: 0,
        }
    }
}

/// The child of a branch at `shift` that holds `hash`.
fn map_slot(hash: u64, shift: u32) -> usize {
    (hash >> shift) as usize % MAP_FANOUT
}

impl<K: Hash + Eq + Clone, V: Clone> SharedMap<K, V> {
    fn hash(key: &K) -> u64 {
        BuildHasherDefault::<DefaultHasher>::default().hash_one(key)
    }
    /// Inserts `value` under `key` unless the map holds `key` already; returns whether it did.
    pub(super) fn insert_new(&mut self, key: K, value: V) -> bool {
        let hash = Self::hash(&key);
        let mut node = Rc::make_mut(&mut self.root);
        let mut shift = 0;
        loop {
            match node {
                MapNode::Branch(children) => {
                    node = Rc::make_mut(
                        children[map_slot(hash, shift)]
                            .get_or_insert_with(|| Rc::new(MapNode::Leaf(Vec::new()))),
                    );
                    shift += MAP_BITS;
                }
                MapNode::Leaf(entries) => {
                    if entries.iter().any(|(entry, _)| *entry == key) {
                        return false;
                    }
                    entries.push((key, value));
                    self.len += 1;
                    // Keys whose hashes agree in every bit share a leaf however long.
                    if entries.len() > MAP_LEAF && shift + MAP_BITS <= u64::BITS {
                        let mut children: [Option<Rc<MapNode<K, V>>>; MAP_FANOUT] =
                            std::array::from_fn(|_| None);
                        for (key, value) in std::mem::take(entries) {
                            let child = children[map_slot(Self::hash(&key), shift)]
                                .get_or_insert_with(|| Rc::new(MapNode::Leaf(Vec::new())));
                            if let MapNode::Leaf(entries) = Rc::make_mut(child) {
                                entries.push((key, value));
                            }
                        }
                        *node = MapNode::Branch(children);
                    }
                    return true;
                }
            }
        }
    }
    /// The entries, in no particular order.
    pub(super) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        let mut entries = Vec::with_capacity(self.len);
        let mut pending = vec![self.root.as_ref()];
        while let Some(node) = pending.pop() {
            match node {
                MapNode::Leaf(leaf) => entries.extend(leaf.iter().map(|(key, value)| (key, value))),
                MapNode::Branch(children) => {
                    pending.extend(children.iter().flatten().map(Rc::as_ref));
                }
            }
        }
        entries.into_iter()
    }
}
