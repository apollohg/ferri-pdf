//! The work rendering does, counted where tests read it to check how it grows with the content.

/// A kind of work rendering does.
#[derive(Clone, Copy)]
pub(crate) enum Work {
    /// An SVG element converting, inside measurements included.
    Conversion,
    /// A node a walk over an SVG tree and its resources visits.
    Visit,
    /// A byte of SVG text the shaper shapes.
    ShapedByte,
    /// A span an SVG text chunk built from another holds.
    HeldSpan,
    /// An SVG text character whose code point position is recorded or looked up.
    CodePoint,
    /// A character advance summed while hiding an overflowing SVG text line's characters.
    HiddenAdvance,
    /// The offset of a tile whose overflowing content a pattern tile shows.
    OverflowTile,
    /// A parent link followed while counting the inline boxes in each box's subtree.
    BoxAncestor,
    /// A text span a search for the span at an offset of a paragraph compares.
    SpanProbe,
    /// An ancestor selector matching, or the ancestor filter of the element it matches, steps to.
    SelectorAncestor,
    /// An open formatting frame a lookup on behalf of the element being formatted reads.
    OpenFrame,
    /// A node a style evaluation looks up among those it has open, to find dependency cycles.
    ActiveNode,
    /// A counter that work on an element's counters steps over, as the counter budget charges.
    CounterStep,
    /// A step of a paint order path that placing a retained lane's paints in the enclosing
    /// traversal copies.
    PlacedStep,
}

#[cfg(test)]
const KINDS: usize = 14;

#[cfg(test)]
thread_local! {
    static COUNTS: std::cell::Cell<[usize; KINDS]> = const { std::cell::Cell::new([0; KINDS]) };
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

/// Asserts that the work `measure` counts for content of `size`, and of twice that, grows
/// linearly; `what` names the content.
#[cfg(test)]
pub(crate) fn assert_linear(what: &str, size: usize, measure: impl Fn(usize) -> usize) {
    const GROWTH: f64 = 2.2;
    let [single, double] = [size, 2 * size].map(measure);
    assert!(
        single > 0 && double as f64 <= GROWTH * single as f64,
        "{what}: {single} units of work for {size}, {double} for {}, more than {GROWTH} times \
         as many",
        2 * size
    );
}
