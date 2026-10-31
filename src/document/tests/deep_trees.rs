//! Rendering a deep tree takes work linear in its depth: what an element looks up among the
//! elements around it, and the counters it inherits, take no walk over its ancestors.
use crate::work::{self, Work};
use crate::{RenderDiagnostic, RenderOptions, render_html};

/// Lists nested this deep, and twice as deep, take twice the work.
const DEPTH: usize = 64;

/// Ordered lists nested `depth` deep, each holding one item around the next list.
fn nested_lists(depth: usize) -> String {
    format!(
        "<!doctype html><body>{}Deepest{}",
        "<ol><li>".repeat(depth),
        "</li></ol>".repeat(depth)
    )
}

/// The units of `kind` that rendering lists nested `depth` deep does, which must render
/// without leaving anything out.
fn rendering_work(kind: Work, depth: usize) -> usize {
    let (output, units) = work::measured(kind, || {
        render_html(&nested_lists(depth), &RenderOptions::default())
            .unwrap_or_else(|error| panic!("lists nested {depth} deep must render: {error}"))
    });
    let limits: Vec<_> = output
        .diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic, RenderDiagnostic::FormattingLimit { .. }))
        .collect();
    assert!(
        limits.is_empty(),
        "lists nested {depth} deep fit every formatting limit: {limits:?}"
    );
    units
}

#[test]
fn counters_of_nested_lists_take_work_linear_in_their_depth() {
    // Each list resets `list-item` inside the counters of the lists around it; copying or
    // filtering those for each element is quadratic in the depth.
    work::assert_linear("counters of nested lists", DEPTH, |depth| {
        rendering_work(Work::CounterStep, depth)
    });
}

#[test]
fn formatting_finds_scroll_containers_and_block_children_without_walking_open_boxes() {
    work::assert_linear("formatting frames of nested lists", DEPTH, |depth| {
        rendering_work(Work::OpenFrame, depth)
    });
}

#[test]
fn style_evaluation_finds_dependency_cycles_without_walking_open_nodes() {
    // An inherited value of a deep element evaluates its ancestors' values first, each open
    // while those inside it evaluate; finding a cycle by walking them is quadratic.
    work::assert_linear("evaluation nodes of nested lists", DEPTH, |depth| {
        rendering_work(Work::ActiveNode, depth)
    });
}
