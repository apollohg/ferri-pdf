//! The work SVG conversion does, counted where tests read it to check that it grows linearly
//! with the content, and the node budget conversion keeps, which tests can lower.

/// A kind of work conversion does.
#[derive(Clone, Copy)]
pub(crate) enum Work {
    /// An element converting, inside measurements included.
    Conversion,
    /// A node a walk over the tree and its resources visits.
    Visit,
    /// A byte of text the shaper shapes.
    ShapedByte,
    /// A span a chunk built from another holds.
    HeldSpan,
    /// A character whose code point position is recorded or looked up.
    CodePoint,
    /// A character advance summed while hiding an overflowing line's characters.
    HiddenAdvance,
}

#[cfg(test)]
const KINDS: usize = 6;

#[cfg(test)]
thread_local! {
    static COUNTS: std::cell::Cell<[usize; KINDS]> = const { std::cell::Cell::new([0; KINDS]) };
    static NODE_BUDGET: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Counts `units` of `work`.
#[inline]
pub(crate) fn count(work: Work, units: usize) {
    #[cfg(test)]
    COUNTS.with(|counts| {
        let mut all = counts.get();
        all[work as usize] += units;
        counts.set(all);
    });
    #[cfg(not(test))]
    let _ = (work, units);
}

/// The most elements conversion converts, each as many times as references draw it.
pub(crate) fn node_budget() -> usize {
    #[cfg(test)]
    if let Some(budget) = NODE_BUDGET.with(std::cell::Cell::get) {
        return budget;
    }
    super::MAX_SVG_NODES
}

/// What `run` returns, with the units of `work` it did on this thread.
#[cfg(test)]
pub(crate) fn measured<T>(work: Work, run: impl FnOnce() -> T) -> (T, usize) {
    let before = COUNTS.with(std::cell::Cell::get)[work as usize];
    let output = run();
    (
        output,
        COUNTS.with(std::cell::Cell::get)[work as usize] - before,
    )
}

/// What `run` returns converting at most `budget` elements on this thread.
#[cfg(test)]
pub(crate) fn with_node_budget<T>(budget: usize, run: impl FnOnce() -> T) -> T {
    let outer = NODE_BUDGET.with(|limit| limit.replace(Some(budget)));
    let output = run();
    NODE_BUDGET.with(|limit| limit.set(outer));
    output
}
