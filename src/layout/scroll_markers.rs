//! CSS Overflow 5 §3.1.8: the active marker of each scroll marker group, which the positions
//! of its scroll targets give at the scroll position a static rendering shows, the initial
//! one of every scroll container (CSS Overflow 3 §2.3). The root element's scroll container
//! is the viewport, whose scrollport is the page area and whose pages continue its content in
//! its block axis.
use crate::document::scroll_markers::{ScrollMarkerGroups, Scroller};
use crate::dom::{ElementId, ScrollMarkerStates, TargetPosition};
use crate::graphics::Rect;
use crate::style::anchor_values::{HORIZONTAL, VERTICAL};
use crate::style::logical::{BOTTOM, Flow, LEFT, RIGHT, TOP};
use crate::style::{PageStyleTarget, PseudoElement};
use std::collections::HashMap;

/// The scroll position a static rendering shows, measured from the scroll origin.
const INITIAL_SCROLL_POSITION: f32 = 0.0;
/// §3.1.8: targets within this fraction of the scrollport of an end of the scroll range share
/// the positions near that end.
const DISTRIBUTE_SCROLLPORT_FRACTION: f32 = 1.0 / 8.0;
const HALF: f32 = 0.5;

/// A fragment of a box as layout placed it: its page and the box's border, padding and
/// content boxes as if unfragmented.
#[derive(Clone, Copy)]
pub(super) struct PlacedBox {
    pub page: usize,
    pub border: Rect,
    pub padding: Rect,
    pub content: Rect,
}

/// A box in the document's continuous coordinates: the bounds of its fragments' border
/// boxes, and its first fragment's padding and content boxes.
struct Placed {
    border: Rect,
    padding: Rect,
    content: Rect,
}

/// Where a scroll container's contents scroll, in the coordinates of the document's flow.
struct Scrollport {
    /// The padding box.
    port: Rect,
    /// Its content box, which the end padding added to its scrollable overflow insets.
    content: Rect,
    /// The bounds of the boxes and text it contains.
    overflow: Option<Rect>,
    flow: Flow,
    padding: [f32; 4],
}

/// The pages laid end to end in the root's block axis, as a continuous medium shows them.
struct Pages {
    /// Each page area, physical on its page.
    areas: Vec<Rect>,
    /// The offset of each page's contents in the document's continuous coordinates.
    offsets: Vec<[f32; 2]>,
}
impl Pages {
    fn new(areas: Vec<Rect>, flow: Flow) -> Self {
        let block = flow.block_axis();
        let start = flow.axis_start(block);
        let mut advanced = 0.0;
        let offsets = areas
            .iter()
            .map(|area| {
                let extent = [area.width, area.height][block];
                let mut offset = [-area.x, -area.y];
                offset[block] = match start {
                    RIGHT => -(area.x + area.width) - advanced,
                    BOTTOM => -(area.y + area.height) - advanced,
                    _ => offset[block] + advanced,
                };
                advanced += extent;
                offset
            })
            .collect();
        Self { areas, offsets }
    }
    fn place(&self, page: usize, rect: Rect) -> Rect {
        let [x, y] = self.offsets[page];
        Rect {
            x: rect.x + x,
            y: rect.y + y,
            ..rect
        }
    }
}

/// The distance in the axis `start` begins from the start edge of `port` to that of `rect`.
fn start_distance(port: Rect, rect: Rect, start: usize) -> f32 {
    match start {
        TOP => rect.y - port.y,
        BOTTOM => (port.y + port.height) - (rect.y + rect.height),
        LEFT => rect.x - port.x,
        _ => (port.x + port.width) - (rect.x + rect.width),
    }
}

/// The distance in the axis `start` begins from the start edge of `port` to the end edge of
/// `rect`.
fn end_distance(port: Rect, rect: Rect, start: usize) -> f32 {
    match start {
        TOP => (rect.y + rect.height) - port.y,
        BOTTOM => (port.y + port.height) - rect.y,
        LEFT => (rect.x + rect.width) - port.x,
        _ => (port.x + port.width) - rect.x,
    }
}

fn extent(rect: Rect, axis: usize) -> f32 {
    [rect.width, rect.height][axis]
}

impl Scrollport {
    /// The length of the scrollable overflow area in `axis` (CSS Overflow 3 §2.2): its
    /// padding box, the boxes and text it contains, and its end padding after them.
    fn scroll_size(&self, axis: usize) -> f32 {
        let start = self.flow.axis_start(axis);
        let padding = end_distance(self.port, self.port, start)
            - end_distance(self.port, self.content, start);
        let contents = self.overflow.map_or(0.0, |overflow| {
            end_distance(self.port, overflow, start) + padding
        });
        extent(self.port, axis).max(contents)
    }
}

/// The calculation's state for one layout.
pub(super) struct Calculation<'a> {
    groups: &'a ScrollMarkerGroups,
    boxes: HashMap<PageStyleTarget, Placed>,
    pages: Pages,
    root: ElementId,
    scrollports: HashMap<ElementId, Scrollport>,
}

impl<'a> Calculation<'a> {
    /// `boxes` holds the fragments of each box, `ink` the text and decorations each box
    /// holds, and `areas` the physical page area of each page of a document in `flow`.
    pub(super) fn new(
        groups: &'a ScrollMarkerGroups,
        boxes: &HashMap<PageStyleTarget, Vec<PlacedBox>>,
        ink: impl Iterator<Item = (PageStyleTarget, usize, Rect)>,
        (areas, flow): (Vec<Rect>, Flow),
        root: ElementId,
    ) -> Self {
        let pages = Pages::new(areas, flow);
        let boxes = boxes
            .iter()
            .filter_map(|(&target, fragments)| {
                let first = fragments.first()?;
                let border = fragments
                    .iter()
                    .map(|fragment| pages.place(fragment.page, fragment.border))
                    .reduce(Rect::union)?;
                Some((
                    target,
                    Placed {
                        border,
                        padding: pages.place(first.page, first.padding),
                        content: pages.place(first.page, first.content),
                    },
                ))
            })
            .collect();
        let mut calculation = Self {
            groups,
            boxes,
            root,
            scrollports: HashMap::new(),
            pages,
        };
        for (&element, scroller) in &groups.scrollers {
            if let Some(scrollport) = calculation.scrollport(element, scroller) {
                calculation.scrollports.insert(element, scrollport);
            }
        }
        let contents = calculation
            .boxes
            .iter()
            .map(|(&target, placed)| (target, false, placed.border))
            .chain(
                ink.map(|(target, page, rect)| (target, true, calculation.pages.place(page, rect))),
            )
            .collect::<Vec<_>>();
        for (target, text, rect) in contents {
            if rect.width <= 0.0 || rect.height <= 0.0 {
                continue;
            }
            if let Some(scrollport) = calculation
                .containing_scroller(target, text)
                .and_then(|scroller| calculation.scrollports.get_mut(&scroller))
            {
                scrollport.overflow = Some(
                    scrollport
                        .overflow
                        .map_or(rect, |overflow| overflow.union(rect)),
                );
            }
        }
        calculation
    }

    fn scrollport(&self, element: ElementId, scroller: &Scroller) -> Option<Scrollport> {
        let (port, content) = if element == self.root {
            let area = self.pages.place(0, *self.pages.areas.first()?);
            (area, area)
        } else {
            let placed = self.boxes.get(&PageStyleTarget {
                element,
                pseudo: None,
            })?;
            (placed.padding, placed.content)
        };
        // Percentages refer to the scrollport's size in the side's axis.
        let padding = std::array::from_fn(|side| {
            let axis = if matches!(side, TOP | BOTTOM) {
                VERTICAL
            } else {
                HORIZONTAL
            };
            scroller.padding[side]
                .as_ref()
                .map_or(0.0, |padding| padding.resolve(extent(port, axis)))
        });
        Some(Scrollport {
            port,
            content,
            overflow: None,
            flow: scroller.flow,
            padding,
        })
    }

    /// The nearest scroll container whose scrollable overflow the box of `target`, or the
    /// `text` its boxes hold, lies in.
    fn containing_scroller(&self, target: PageStyleTarget, text: bool) -> Option<ElementId> {
        let element = target.element;
        let scroller = |element| {
            if element == self.root {
                Some(self.root)
            } else {
                self.groups
                    .nearest
                    .get(&element)
                    .map(|&(scroller, _)| scroller)
            }
        };
        match target.pseudo {
            None if !text => scroller(element),
            // A group lies beside its scroll container, and a marker in its group.
            Some(PseudoElement::ScrollMarkerGroup) => scroller(element),
            Some(PseudoElement::ScrollMarker) => scroller(element).map(|owner| {
                self.groups
                    .scrollers
                    .get(&owner)
                    .and_then(|owner| owner.parent)
                    .unwrap_or(owner)
            }),
            // An element's other pseudo-elements and its text lie inside it.
            _ if self.groups.scrollers.contains_key(&element) => Some(element),
            _ => scroller(element),
        }
    }

    /// The tree order of `element`, which has a box.
    fn order(&self, element: ElementId) -> usize {
        if element == self.root {
            return 0;
        }
        self.groups
            .nearest
            .get(&element)
            .map_or(usize::MAX, |&(_, order)| order + 1)
    }

    /// The scroll containers from `element`'s nearest ancestor one outward, ending with the
    /// root's.
    fn chain(&self, element: ElementId) -> Vec<ElementId> {
        let mut chain = Vec::new();
        let mut next = if element == self.root {
            Some(self.root)
        } else {
            self.groups
                .nearest
                .get(&element)
                .map(|&(scroller, _)| scroller)
        };
        while let Some(scroller) = next {
            chain.push(scroller);
            next = self
                .groups
                .scrollers
                .get(&scroller)
                .and_then(|scroller| scroller.parent);
        }
        chain
    }

    /// The box among `element` and its ancestors whose nearest scroll container is
    /// `scroller`, if `scroller` contains `element`.
    fn child_of(&self, element: ElementId, scroller: ElementId) -> Option<ElementId> {
        let mut current = element;
        let chain = self.chain(element);
        for &ancestor in &chain {
            if ancestor == scroller {
                return Some(current);
            }
            current = ancestor;
        }
        None
    }

    /// The position of each group's scroll markers relative to its active marker.
    pub(super) fn states(&self) -> ScrollMarkerStates {
        let mut states = ScrollMarkerStates::new();
        for group in &self.groups.groups {
            let mut targets: Vec<ElementId> = Vec::new();
            for &(_, target) in &group.markers {
                if !targets.contains(&target) && self.target_box(target).is_some() {
                    targets.push(target);
                }
            }
            let start = group.scroller.or_else(|| {
                let mut chains = group
                    .markers
                    .iter()
                    .map(|(marker, _)| marker.element)
                    .chain(targets.iter().copied())
                    .filter(|&element| self.order(element) != usize::MAX)
                    .map(|element| self.chain(element));
                let first = chains.next()?;
                let rest: Vec<_> = chains.collect();
                first
                    .into_iter()
                    .find(|scroller| rest.iter().all(|chain| chain.contains(scroller)))
            });
            let Some(active) = start.and_then(|start| self.active_target(start, &targets)) else {
                continue;
            };
            let Some(selected) = group
                .markers
                .iter()
                .position(|&(_, target)| target == active)
            else {
                continue;
            };
            for (index, (marker, _)) in group.markers.iter().enumerate() {
                states.insert(
                    *marker,
                    match index.cmp(&selected) {
                        std::cmp::Ordering::Less => TargetPosition::Before,
                        std::cmp::Ordering::Equal => TargetPosition::Current,
                        std::cmp::Ordering::Greater => TargetPosition::After,
                    },
                );
            }
        }
        states
    }

    fn target_box(&self, element: ElementId) -> Option<Rect> {
        self.boxes
            .get(&PageStyleTarget {
                element,
                pseudo: None,
            })
            .map(|placed| placed.border)
    }

    /// The scroll target among `targets` the scroll positions of `scroller` and the scroll
    /// containers inside it that contain targets make active.
    fn active_target(&self, mut scroller: ElementId, targets: &[ElementId]) -> Option<ElementId> {
        loop {
            let mut candidates: Vec<(ElementId, Rect)> = Vec::new();
            for &target in targets {
                if let Some(candidate) = self.child_of(target, scroller)
                    && !candidates.iter().any(|&(known, _)| known == candidate)
                    && let Some(rect) = self.target_box(candidate)
                {
                    candidates.push((candidate, rect));
                }
            }
            candidates.sort_by_key(|&(candidate, _)| self.order(candidate));
            let active = self.select(scroller, candidates)?;
            let contains_targets = active != scroller
                && self.scrollports.contains_key(&active)
                && targets
                    .iter()
                    .any(|&target| target != active && self.child_of(target, active).is_some());
            if !contains_targets {
                return Some(active);
            }
            scroller = active;
        }
    }

    /// The candidate, a box `scroller` contains in tree order with its border box, at the
    /// eventual scroll position in its primary and then its secondary axis; the first where
    /// several are.
    fn select(
        &self,
        scroller: ElementId,
        mut candidates: Vec<(ElementId, Rect)>,
    ) -> Option<ElementId> {
        let scrollport = self.scrollports.get(&scroller)?;
        let block = scrollport.flow.block_axis();
        for axis in [block, 1 - block] {
            let start = scrollport.flow.axis_start(axis);
            let port = extent(scrollport.port, axis);
            let mut positions: Vec<f32> = candidates
                .iter()
                .map(|&(candidate, rect)| {
                    let margins = self
                        .groups
                        .margins
                        .get(&candidate)
                        .copied()
                        .unwrap_or_default();
                    start_distance(scrollport.port, rect, start)
                        - margins[start]
                        - scrollport.padding[start]
                })
                .collect();
            let range = scrollport.scroll_size(axis) - port;
            if range > 0.0 {
                redistribute(&mut positions, port, range);
            }
            let selected = selected_position(&positions, port);
            // The secondary axis decides between the candidates the primary axis selects.
            candidates = candidates
                .into_iter()
                .zip(positions)
                .filter(|&(_, position)| position == selected)
                .map(|(candidate, _)| candidate)
                .collect();
        }
        candidates.first().map(|&(candidate, _)| candidate)
    }
}

/// §3.1.8: spreads the positions of targets that no scroll position reaches, or that crowd an
/// end of the scroll `range`, over the part of the range near that end.
fn redistribute(positions: &mut [f32], port: f32, range: f32) {
    let distribute = (DISTRIBUTE_SCROLLPORT_FRACTION * port).min(range * HALF);
    let before = positions
        .iter()
        .copied()
        .filter(|&position| position < distribute)
        .fold(f32::INFINITY, f32::min);
    let end = range - distribute;
    let after = positions
        .iter()
        .copied()
        .filter(|&position| position > end)
        .fold(f32::NEG_INFINITY, f32::max);
    for position in positions.iter_mut() {
        if *position < distribute {
            *position = (*position - before) / (distribute - before) * distribute;
        } else if *position > end {
            *position = (*position - end) / (after - end) * distribute + end;
        }
    }
}

/// §3.1.8: the largest position at or before the scroll position, or one whose nearest
/// smaller position lies more than half a scrollport before it while it lies less than half
/// a scrollport after it; otherwise the first.
fn selected_position(positions: &[f32], port: f32) -> f32 {
    let position = INITIAL_SCROLL_POSITION;
    let mut sorted = positions.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted.dedup();
    sorted
        .iter()
        .enumerate()
        .rev()
        .find(|&(index, &candidate)| {
            candidate <= position
                || (index > 0
                    && sorted[index - 1] < position - port * HALF
                    && candidate < position + port * HALF)
        })
        .map(|(_, &candidate)| candidate)
        .or_else(|| sorted.first().copied())
        .unwrap_or(position)
}

#[cfg(test)]
mod tests;
