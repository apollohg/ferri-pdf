//! Anchor lookup and physical box measurements (CSS Anchor Positioning 1 §2, §9).
use super::pagination::Region;
type EffectChain = Vec<(usize, Region)>;
use crate::graphics::Rect;
use crate::style::anchor_values::{
    AnchorBox, AnchorFrame, AnchorFrameMap, AnchorKey, AnchorName, AnchorNode, AnchorReference,
};
use crate::style::{PageStyleTarget, Position, Style, Stylesheet};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Default)]
pub(super) struct Measurements {
    boxes: HashMap<PageStyleTarget, Vec<Fragment>>,
    effects: HashMap<(usize, Option<usize>), EffectChain>,
    placements: HashMap<AnchorKey, Placement>,
}
#[derive(Clone)]
struct Fragment {
    visible: bool,
    source: Rect,
    slot: Option<usize>,
    columns: bool,
    fragmentainer: usize,
    page: usize,
    rect: Rect,
    clip: Option<usize>,
}
#[derive(Clone)]
struct Placement {
    columns: bool,
    fragmentainer: usize,
    clip: Option<usize>,
    page: usize,
    containing: Rect,
    rtl: bool,
    references: Vec<AnchorReference>,
    option: usize,
    overflows: bool,
}

pub(super) fn key(style: &Style) -> Option<PageStyleTarget> {
    let origin = style.origin.node()?;
    Some(crate::style::anchor_values::key(
        origin.element,
        origin.pseudo,
    ))
}

impl Measurements {
    pub(super) fn record(
        &mut self,
        style: &Style,
        at: (usize, Option<usize>, bool),
        rect: Rect,
        source: Option<Rect>,
        clip: Option<usize>,
        columns: bool,
    ) {
        let (page, slot, visible) = at;
        if let Some(key) = key(style) {
            self.boxes.entry(key).or_default().push(Fragment {
                visible,
                page,
                rect,
                source: source.unwrap_or(rect),
                slot,
                clip,
                columns,
                fragmentainer: page,
            });
        }
    }

    pub(super) fn assemble(&mut self, mut source: impl FnMut(usize, usize) -> Option<Rect>) {
        for fragment in self.boxes.values_mut().flatten() {
            if let Some(rect) = fragment.slot.and_then(|slot| source(fragment.page, slot)) {
                fragment.source = rect;
            }
        }
    }

    pub(super) fn effects(
        &mut self,
        mut resolve: impl FnMut(usize, Option<usize>) -> Vec<(usize, Region)>,
    ) {
        for (page, clip) in self
            .boxes
            .values()
            .flatten()
            .map(|fragment| (fragment.page, fragment.clip))
            .chain(
                self.placements
                    .values()
                    .map(|placement| (placement.page, placement.clip)),
            )
        {
            self.effects
                .entry((page, clip))
                .or_insert_with(|| resolve(page, clip));
        }
    }

    pub(super) fn place(
        &mut self,
        style: &Style,
        selected: &Style,
        containing: (usize, Rect, bool),
        rtl: bool,
        border: Rect,
        clip: Option<usize>,
    ) {
        let (page, containing, columns) = containing;
        if !style.anchor.anchors() {
            return;
        }
        let Some(key) = key(style) else {
            return;
        };
        let overflows = super::positioned::overflows(selected, containing, rtl, border);
        self.placements
            .entry(AnchorKey { target: key, page })
            .or_insert_with(|| Placement {
                fragmentainer: page,
                columns,
                clip,
                page,
                containing,
                rtl,
                references: style.anchor.located_references(),
                option: selected.anchor.option,
                overflows,
            });
    }

    pub(super) fn adopt_column(
        &mut self,
        other: &mut Self,
        at: (usize, usize, usize),
        shift: [f32; 2],
        mut clip: impl FnMut(Option<usize>) -> Option<usize>,
    ) {
        let (column, page, slots) = at;
        for (&key, fragments) in &mut other.boxes {
            let (adopted, remaining): (Vec<_>, Vec<_>) = std::mem::take(fragments)
                .into_iter()
                .partition(|fragment| fragment.page == column);
            *fragments = remaining;
            for fragment in adopted {
                self.boxes.entry(key).or_default().push(Fragment {
                    visible: fragment.visible,
                    page,
                    source: Rect {
                        x: fragment.source.x + shift[0],
                        y: fragment.source.y + shift[1],
                        ..fragment.source
                    },
                    slot: fragment.slot.map(|slot| slots + slot),
                    columns: fragment.columns,
                    fragmentainer: fragment.fragmentainer,
                    rect: Rect {
                        x: fragment.rect.x + shift[0],
                        y: fragment.rect.y + shift[1],
                        ..fragment.rect
                    },
                    clip: clip(fragment.clip),
                });
            }
        }
        let keys: Vec<_> = other
            .placements
            .iter()
            .filter_map(|(&key, placement)| (placement.page == column).then_some(key))
            .collect();
        for key in keys {
            let mut placement = other
                .placements
                .remove(&key)
                .expect("the selected column has a placement");
            placement.page = page;
            placement.containing.x += shift[0];
            placement.containing.y += shift[1];
            placement.clip = clip(placement.clip);
            self.placements
                .entry(AnchorKey {
                    target: key.target,
                    page,
                })
                .or_insert(placement);
        }
    }

    pub(super) fn frames(self, sheet: &Stylesheet) -> AnchorFrameMap {
        let nodes = sheet.anchor_nodes.borrow();
        self.placements
            .iter()
            .map(|(&query, placement)| {
                let anchors = placement
                    .references
                    .iter()
                    .map(|reference| {
                        let target = target(
                            &nodes,
                            sheet,
                            query.target,
                            reference,
                            &mut HashSet::new(),
                            placement.page,
                        );
                        let anchor = target.and_then(|target| {
                            let fragments = self.boxes.get(&target)?;
                            let source = containing(&nodes, query.target).is_some_and(|cb| {
                                fragmentation(&nodes, cb) == fragmentation(&nodes, target)
                            });
                            let repeated = nodes
                                .get(&target)
                                .is_some_and(|node| node.position == Position::Fixed)
                                && containing(&nodes, target).is_none();
                            let selected = fragments
                                .iter()
                                .position(|fragment| {
                                    fragment.page == placement.page
                                        && (!source
                                            || (fragment.columns == placement.columns
                                                && fragment.fragmentainer
                                                    == placement.fragmentainer))
                                })
                                .unwrap_or_default();
                            let cb_regions = &self.effects[&(placement.page, placement.clip)];
                            let mut rectangle: Option<Rect> = None;
                            let mut visible = false;
                            for (_, fragment) in fragments
                                .iter()
                                .enumerate()
                                .filter(|(index, _)| (!source && !repeated) || *index == selected)
                            {
                                let source_rect = if source {
                                    fragment.source
                                } else {
                                    fragment.rect
                                };
                                let anchor_regions = &self.effects[&(fragment.page, fragment.clip)];
                                let common = anchor_regions
                                    .iter()
                                    .zip(cb_regions)
                                    .take_while(|(anchor, cb)| anchor == cb)
                                    .count();
                                let regions: Vec<_> = anchor_regions[common..]
                                    .iter()
                                    .map(|(_, region)| *region)
                                    .collect();
                                let mut polygon =
                                    super::navigation::page_polygon(source_rect, &regions, false);
                                let clipped =
                                    super::navigation::page_polygon(fragment.rect, &regions, true);
                                visible |= fragment.visible
                                    && !clipped.is_empty()
                                    && (fragment.rect.width == 0.0
                                        || fragment.rect.height == 0.0
                                        || bounds(&clipped).is_some_and(|rect| {
                                            rect.width > 0.0 && rect.height > 0.0
                                        }));
                                for (_, region) in &cb_regions[common..] {
                                    if let Region::Transform(matrix) = region {
                                        let transform = kurbo::Affine::new(matrix.map(f64::from));
                                        if transform.determinant() == 0.0 {
                                            return None;
                                        }
                                        let inverse = transform.inverse();
                                        for point in &mut polygon {
                                            let mapped = inverse
                                                * kurbo::Point::new(
                                                    f64::from(point[0]),
                                                    f64::from(point[1]),
                                                );
                                            *point = [mapped.x as f32, mapped.y as f32];
                                        }
                                    }
                                }
                                if let Some(rect) = bounds(&polygon) {
                                    rectangle = Some(
                                        rectangle.map_or(rect, |previous| previous.union(rect)),
                                    );
                                }
                            }
                            let rect = rectangle?;
                            Some(AnchorBox {
                                rect: Rect {
                                    x: rect.x - placement.containing.x,
                                    y: rect.y - placement.containing.y,
                                    ..rect
                                },
                                visible: visible
                                    && nodes.get(&target).is_some_and(|node| node.visible),
                            })
                        });
                        (reference.clone(), anchor)
                    })
                    .collect();
                (
                    query,
                    AnchorFrame {
                        page: placement.page,
                        containing: [placement.containing.width, placement.containing.height],
                        containing_rtl: placement.rtl,
                        anchors,
                        option: placement.option,
                        overflows: placement.overflows,
                    },
                )
            })
            .collect()
    }
}

fn bounds(points: &[[f32; 2]]) -> Option<Rect> {
    points
        .iter()
        .map(|[x, y]| Rect {
            x: *x,
            y: *y,
            width: 0.0,
            height: 0.0,
        })
        .reduce(Rect::union)
}

fn ancestors(
    nodes: &HashMap<PageStyleTarget, AnchorNode>,
    start: PageStyleTarget,
) -> impl Iterator<Item = PageStyleTarget> + '_ {
    std::iter::successors(Some(start), |current| {
        nodes.get(current).and_then(|node| node.parent)
    })
}

fn containing(
    nodes: &HashMap<PageStyleTarget, AnchorNode>,
    element: PageStyleTarget,
) -> Option<PageStyleTarget> {
    let node = nodes.get(&element)?;
    let parent = node.parent?;
    if !node.position.is_out_of_flow() {
        return Some(parent);
    }
    ancestors(nodes, parent).find(|parent| {
        nodes.get(parent).is_some_and(|parent| {
            if node.position == Position::Fixed {
                parent.contains_fixed
            } else {
                parent.contains_absolute
            }
        })
    })
}

fn acceptable(
    nodes: &HashMap<PageStyleTarget, AnchorNode>,
    mut candidate: PageStyleTarget,
    query: PageStyleTarget,
) -> bool {
    let query_containing = containing(nodes, query);
    let Some(query_node) = nodes.get(&query) else {
        return false;
    };
    loop {
        if candidate == query {
            return false;
        }
        let Some(node) = nodes.get(&candidate) else {
            return false;
        };
        let cb = containing(nodes, candidate);
        if cb == query_containing
            && (!node.position.is_out_of_flow() || node.order < query_node.order)
        {
            return true;
        }
        let Some(parent) = cb else {
            return false;
        };
        candidate = parent;
    }
}

fn fragmentation(
    nodes: &HashMap<PageStyleTarget, AnchorNode>,
    element: PageStyleTarget,
) -> Option<PageStyleTarget> {
    ancestors(nodes, element)
        .skip(1)
        .find(|ancestor| nodes.get(ancestor).is_some_and(|node| node.multicol))
}

fn boundary(
    nodes: &HashMap<PageStyleTarget, AnchorNode>,
    element: PageStyleTarget,
    name: &AnchorName,
) -> Option<PageStyleTarget> {
    ancestors(nodes, element).find(|ancestor| {
        nodes
            .get(ancestor)
            .is_some_and(|node| node.scope.covers(name))
    })
}

fn target(
    nodes: &HashMap<PageStyleTarget, AnchorNode>,
    sheet: &Stylesheet,
    query: PageStyleTarget,
    reference: &AnchorReference,
    seen: &mut HashSet<PageStyleTarget>,
    page: usize,
) -> Option<PageStyleTarget> {
    match reference {
        AnchorReference::Implicit => None,
        AnchorReference::Parent => {
            if !seen.insert(query) {
                return None;
            }
            let parent = nodes.get(&query)?.parent?;
            let node = nodes.get(&parent)?;
            let reference = node
                .page_defaults
                .iter()
                .find(|(candidate, _)| *candidate == page)
                .map_or(node.default.as_ref(), |(_, reference)| reference.as_ref())?;
            let anchor = target(nodes, sheet, parent, reference, seen, page)?;
            acceptable(nodes, anchor, query).then_some(anchor)
        }
        AnchorReference::Named(name) => {
            let matching = |candidate: &PageStyleTarget| {
                nodes.get(candidate).is_some_and(|node| {
                    node.has_box
                        && node.names.iter().any(|anchor| {
                            anchor.name == name.name
                                && sheet.scope_includes(anchor.scope, name.scope)
                                && boundary(nodes, *candidate, anchor)
                                    == boundary(nodes, query, anchor)
                        })
                        && acceptable(nodes, *candidate, query)
                })
            };
            ancestors(nodes, query).skip(1).find(matching).or_else(|| {
                nodes
                    .keys()
                    .filter(|candidate| matching(candidate))
                    .max_by_key(|candidate| nodes[candidate].order)
                    .copied()
            })
        }
    }
}
