//! Anchor lookup and physical box measurements (CSS Anchor Positioning 1 §2, §9).
use super::pagination::Region;
use crate::graphics::Rect;
use crate::style::anchor_values::{
    AnchorBox, AnchorFrame, AnchorFrameMap, AnchorKey, AnchorName, AnchorNode, AnchorReference,
};
use crate::style::{PageStyleTarget, Position, Style, Stylesheet};
use std::collections::{HashMap, HashSet};

type EffectChain = Vec<(usize, Effect)>;
#[derive(Clone, PartialEq)]
pub(super) enum Effect {
    Region(Region, Option<PageStyleTarget>),
    Path(Vec<Vec<crate::graphics::Point>>, Option<PageStyleTarget>),
}

impl Effect {
    fn clips(&self, own: Option<PageStyleTarget>) -> bool {
        match self {
            Self::Region(Region::Clip(_), owner) | Self::Path(_, owner) => {
                own.is_none() || *owner != own
            }
            Self::Region(Region::Transform(_), _) => false,
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Measurements {
    boxes: HashMap<PageStyleTarget, Vec<Fragment>>,
    effects: HashMap<(usize, Option<usize>), EffectChain>,
    placements: HashMap<AnchorKey, Placement>,
    ink: Vec<Ink>,
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
    ink: Rect,
    filters: std::sync::Arc<Vec<crate::graphics::FilterFunction>>,
    clip: Option<usize>,
}
#[derive(Clone)]
struct Ink {
    target: PageStyleTarget,
    page: usize,
    rect: Rect,
    clip: Option<usize>,
}

pub(super) fn box_ink(paint: &crate::graphics::BoxPaint) -> Rect {
    paint
        .shadows(false)
        .into_iter()
        .map(|shadow| shadow.bounds)
        .chain(paint.outline().map(|outline| outline.rect))
        .chain(paint.border_image_bounds())
        .fold(paint.rect, Rect::union)
}

pub(super) fn box_ink_in_frame(
    paint: &crate::graphics::BoxPaint,
    frame: super::writing::Frame,
) -> Rect {
    let mut physical = paint.clone();
    frame.paint(&mut physical);
    frame.inverse().rect(box_ink(&physical))
}

#[derive(Clone)]
struct Placement {
    columns: bool,
    fragmentainer: usize,
    clip: Option<usize>,
    page: usize,
    containing: Rect,
    flow: crate::style::logical::Flow,
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

pub(super) fn ink_key(style: &Style) -> Option<PageStyleTarget> {
    let mut origin = style.origin.node()?;
    while matches!(
        origin.pseudo,
        Some(
            crate::style::PseudoElement::Marker
                | crate::style::PseudoElement::FirstLine
                | crate::style::PseudoElement::FirstLetter
                | crate::style::PseudoElement::FirstLetterPrefix
                | crate::style::PseudoElement::FirstLetterSuffix
        )
    ) {
        origin = origin.parent.node()?;
    }
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
                ink: rect,
                filters: style.filter.clone(),
                source: source.unwrap_or(rect),
                slot,
                clip,
                columns,
                fragmentainer: page,
            });
        }
    }

    pub(super) fn ink(
        &mut self,
        target: Option<PageStyleTarget>,
        page: usize,
        rect: Rect,
        clip: Option<usize>,
    ) {
        if let Some(target) = target {
            self.ink.push(Ink {
                target,
                page,
                rect,
                clip,
            });
        }
    }

    pub(super) fn text(
        &mut self,
        target: Option<PageStyleTarget>,
        page: usize,
        run: &crate::text::GlyphRun,
        origin: crate::graphics::Point,
        clip: Option<usize>,
        frame: super::writing::Frame,
    ) {
        if let Some(ink) = run.ink_rect(origin) {
            self.ink(target, page, ink, clip);
            for shadow in run
                .shadows
                .iter()
                .filter(|shadow| shadow.color.used.alpha > 0.0)
            {
                let [a, b, c, d, _, _] = frame.inverse().matrix;
                let [x, y] = shadow.offset;
                let offset = [a * x + c * y, b * x + d * y];
                let cast = crate::paint::BoxShadow {
                    offset,
                    blur: shadow.blur,
                    spread: 0.0,
                    color: shadow.color.used,
                    inset: false,
                };
                let extent = cast.extent();
                self.ink(
                    target,
                    page,
                    Rect {
                        x: ink.x + offset[0] - extent,
                        y: ink.y + offset[1] - extent,
                        width: ink.width + 2.0 * extent,
                        height: ink.height + 2.0 * extent,
                    },
                    clip,
                );
            }
        }
    }

    pub(super) fn assemble(
        &mut self,
        mut source: impl FnMut(usize, usize) -> Option<(Option<Rect>, Rect)>,
    ) {
        for fragment in self.boxes.values_mut().flatten() {
            if let Some((source, ink)) = fragment.slot.and_then(|slot| source(fragment.page, slot))
            {
                if let Some(rect) = source {
                    fragment.source = rect;
                }
                fragment.ink = ink;
            }
        }
    }

    pub(super) fn effects(&mut self, mut resolve: impl FnMut(usize, Option<usize>) -> EffectChain) {
        for (page, clip) in self
            .boxes
            .values()
            .flatten()
            .map(|fragment| (fragment.page, fragment.clip))
            .chain(self.ink.iter().map(|ink| (ink.page, ink.clip)))
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
        flow: crate::style::logical::Flow,
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
        let overflows = super::positioned::overflows(selected, containing, border);
        self.placements
            .entry(AnchorKey { target: key, page })
            .or_insert_with(|| Placement {
                fragmentainer: page,
                columns,
                clip,
                page,
                containing,
                flow,
                references: style.anchor.located_references(),
                option: selected.anchor.option,
                overflows,
            });
    }

    pub(super) fn adopt(
        &mut self,
        other: &mut Self,
        at: (usize, usize, usize),
        map: impl Fn(Rect) -> Rect,
        mut clip: impl FnMut(Option<usize>) -> Option<usize>,
    ) {
        let (column, page, slots) = at;
        let (adopted, remaining): (Vec<_>, Vec<_>) = std::mem::take(&mut other.ink)
            .into_iter()
            .partition(|ink| ink.page == column);
        other.ink = remaining;
        self.ink.extend(adopted.into_iter().map(|ink| Ink {
            page,
            rect: map(ink.rect),
            clip: clip(ink.clip),
            ..ink
        }));
        for (&key, fragments) in &mut other.boxes {
            let (adopted, remaining): (Vec<_>, Vec<_>) = std::mem::take(fragments)
                .into_iter()
                .partition(|fragment| fragment.page == column);
            *fragments = remaining;
            for fragment in adopted {
                self.boxes.entry(key).or_default().push(Fragment {
                    visible: fragment.visible,
                    page,
                    source: map(fragment.source),
                    slot: fragment.slot.map(|slot| slots + slot),
                    columns: fragment.columns,
                    fragmentainer: fragment.fragmentainer,
                    filters: fragment.filters,
                    ink: map(fragment.ink),
                    rect: map(fragment.rect),
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
            placement.containing = map(placement.containing);
            placement.clip = clip(placement.clip);
            self.placements
                .entry(AnchorKey {
                    target: key.target,
                    page,
                })
                .or_insert(placement);
        }
    }

    pub(super) fn physical(
        &mut self,
        flow: crate::style::logical::Flow,
        pages: &[std::sync::Arc<crate::document::PageBox>],
    ) {
        let frame = |page: usize| super::writing::Frame::physical(flow, pages[page].geometry.paper);
        for fragment in self.boxes.values_mut().flatten() {
            let frame = frame(fragment.page);
            fragment.source = frame.rect(fragment.source);
            fragment.rect = frame.rect(fragment.rect);
            fragment.ink = frame.rect(fragment.ink);
        }
        for ink in &mut self.ink {
            ink.rect = frame(ink.page).rect(ink.rect);
        }
        for placement in self.placements.values_mut() {
            placement.containing = frame(placement.page).rect(placement.containing);
        }
        for ((page, _), chain) in &mut self.effects {
            let frame = frame(*page);
            for (_, effect) in chain {
                match effect {
                    Effect::Region(Region::Clip(clip), _) => *clip = frame.clip(*clip),
                    Effect::Region(Region::Transform(matrix), _) => {
                        *matrix = frame.inverse().into_logical(*matrix);
                    }
                    Effect::Path(polygons, _) => {
                        for point in polygons.iter_mut().flatten() {
                            *point = frame.point(*point);
                        }
                    }
                }
            }
        }
    }

    pub(super) fn payload_work(&self) -> usize {
        self.boxes
            .values()
            .map(Vec::len)
            .chain([self.ink.len()])
            .chain(
                self.placements
                    .values()
                    .map(|placement| placement.references.len().saturating_add(1)),
            )
            .fold(1, usize::saturating_add)
    }

    pub(super) fn frames(
        self,
        sheet: &Stylesheet,
        definitions: &crate::document::svg::SvgDefinitions,
        work: &super::work::Budget,
    ) -> Result<AnchorFrameMap, super::LayoutError> {
        let nodes = sheet.anchor_nodes.borrow();
        let mut frames = HashMap::new();
        for (&query, placement) in &self.placements {
            let mut anchors = Vec::new();
            for reference in &placement.references {
                work.spend(nodes.len().max(1))?;
                let selected = target(
                    &nodes,
                    sheet,
                    query.target,
                    reference,
                    &mut HashSet::new(),
                    placement.page,
                );
                let anchor = if let Some(target) = selected {
                    self.anchor_box(&nodes, query, placement, target, definitions, work)?
                } else {
                    None
                };
                anchors.push((reference.clone(), anchor));
            }
            frames.insert(
                query,
                AnchorFrame {
                    page: placement.page,
                    containing: [placement.containing.width, placement.containing.height],
                    containing_flow: placement.flow,
                    anchors,
                    option: placement.option,
                    overflows: placement.overflows,
                },
            );
        }
        Ok(frames)
    }

    fn anchor_box(
        &self,
        nodes: &HashMap<PageStyleTarget, AnchorNode>,
        query: AnchorKey,
        placement: &Placement,
        target: PageStyleTarget,
        definitions: &crate::document::svg::SvgDefinitions,
        work: &super::work::Budget,
    ) -> Result<Option<AnchorBox>, super::LayoutError> {
        let Some(fragments) = self.boxes.get(&target) else {
            return Ok(None);
        };
        let source = containing(nodes, query.target)
            .is_some_and(|cb| fragmentation(nodes, cb) == fragmentation(nodes, target));
        let repeated = nodes
            .get(&target)
            .is_some_and(|node| node.position == Position::Fixed)
            && containing(nodes, target).is_none();
        let selected = fragments
            .iter()
            .position(|fragment| {
                fragment.page == placement.page
                    && (!source
                        || (fragment.columns == placement.columns
                            && fragment.fragmentainer == placement.fragmentainer))
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
                .filter_map(|(_, effect)| match effect {
                    Effect::Region(region, _) => Some(*region),
                    Effect::Path(..) => None,
                })
                .collect();
            let mut polygon = super::navigation::page_polygon(source_rect, &regions, false);
            if !anchor_regions[common..]
                .iter()
                .any(|(_, effect)| effect.clips(Some(target)))
            {
                visible |= fragment.visible;
            } else if let Some(ink) = self.ink_bounds(nodes, target, fragment, definitions, work)? {
                let clipped = clipped(ink, &anchor_regions[common..], Some(target), work)?;
                visible |= fragment.visible
                    && clipped.iter().any(|polygon| {
                        !polygon.is_empty()
                            && (fragment.rect.width == 0.0
                                || fragment.rect.height == 0.0
                                || polygon_area(polygon) > 0.0)
                    });
            }
            for (_, region) in &cb_regions[common..] {
                if let Effect::Region(Region::Transform(matrix), _) = region {
                    let transform = kurbo::Affine::new(matrix.map(f64::from));
                    if transform.determinant() == 0.0 {
                        return Ok(None);
                    }
                    let inverse = transform.inverse();
                    for point in &mut polygon {
                        let mapped =
                            inverse * kurbo::Point::new(f64::from(point[0]), f64::from(point[1]));
                        *point = [mapped.x as f32, mapped.y as f32];
                    }
                }
            }
            if let Some(rect) = bounds(&polygon) {
                rectangle = Some(rectangle.map_or(rect, |previous| previous.union(rect)));
            }
        }
        let Some(rect) = rectangle else {
            return Ok(None);
        };
        Ok(Some(AnchorBox {
            rect: Rect {
                x: rect.x - placement.containing.x,
                y: rect.y - placement.containing.y,
                ..rect
            },
            visible: visible && nodes.get(&target).is_some_and(|node| node.visible),
        }))
    }
    fn ink_bounds(
        &self,
        nodes: &HashMap<PageStyleTarget, AnchorNode>,
        target: PageStyleTarget,
        fragment: &Fragment,
        definitions: &crate::document::svg::SvgDefinitions,
        work: &super::work::Budget,
    ) -> Result<Option<Rect>, super::LayoutError> {
        let mut ink = fragment.visible.then_some(fragment.ink);
        let anchor_regions = &self.effects[&(fragment.page, fragment.clip)];
        for (&owner, fragments) in &self.boxes {
            for child in fragments.iter().filter(|child| {
                (child.visible || !child.filters.is_empty()) && child.page == fragment.page
            }) {
                work.spend(1)?;
                if owner == target
                    || !descendant(nodes, owner, target, work)?
                    || self.filtered_ancestor(nodes, owner, target, fragment.page, false, work)?
                {
                    continue;
                }
                let source = if child.filters.is_empty() {
                    Some(child.ink)
                } else {
                    self.ink_bounds(nodes, owner, child, definitions, work)?
                };
                if let Some(rect) = source
                    && let Some(rect) =
                        self.local_ink(rect, child.page, child.clip, anchor_regions, work)?
                {
                    ink = Some(ink.map_or(rect, |ink| ink.union(rect)));
                }
            }
        }
        for item in self.ink.iter().filter(|ink| ink.page == fragment.page) {
            work.spend(1)?;
            if !descendant(nodes, item.target, target, work)?
                || self.filtered_ancestor(nodes, item.target, target, item.page, true, work)?
            {
                continue;
            }
            if let Some(rect) =
                self.local_ink(item.rect, item.page, item.clip, anchor_regions, work)?
            {
                ink = Some(ink.map_or(rect, |ink| ink.union(rect)));
            }
        }
        match ink {
            Some(ink) => filter_bounds(ink, fragment.rect, &fragment.filters, definitions, work),
            None => Ok(None),
        }
    }

    fn filtered_ancestor(
        &self,
        nodes: &HashMap<PageStyleTarget, AnchorNode>,
        owner: PageStyleTarget,
        target: PageStyleTarget,
        page: usize,
        include_owner: bool,
        work: &super::work::Budget,
    ) -> Result<bool, super::LayoutError> {
        let mut node = if include_owner {
            Some(owner)
        } else {
            nodes.get(&owner).and_then(|node| node.parent)
        };
        while let Some(key) = node {
            work.spend(1)?;
            if key == target {
                return Ok(false);
            }
            if self.boxes.get(&key).is_some_and(|fragments| {
                fragments
                    .iter()
                    .any(|fragment| fragment.page == page && !fragment.filters.is_empty())
            }) {
                return Ok(true);
            }
            node = nodes.get(&key).and_then(|node| node.parent);
        }
        Ok(false)
    }

    fn local_ink(
        &self,
        rect: Rect,
        page: usize,
        clip: Option<usize>,
        anchor_regions: &EffectChain,
        work: &super::work::Budget,
    ) -> Result<Option<Rect>, super::LayoutError> {
        let effects = &self.effects[&(page, clip)];
        let common = effects
            .iter()
            .zip(anchor_regions)
            .take_while(|(a, b)| a == b)
            .count();
        let mut pieces = clipped(rect, &effects[common..], None, work)?;
        for (_, effect) in &anchor_regions[common..] {
            if let Effect::Region(Region::Transform(matrix), _) = effect {
                let transform = kurbo::Affine::new(matrix.map(f64::from));
                if transform.determinant() == 0.0 {
                    return Ok(None);
                }
                let inverse = transform.inverse();
                for point in pieces.iter_mut().flatten() {
                    let mapped =
                        inverse * kurbo::Point::new(f64::from(point[0]), f64::from(point[1]));
                    *point = [mapped.x as f32, mapped.y as f32];
                }
            }
        }
        Ok(bounds(&pieces.into_iter().flatten().collect::<Vec<_>>()))
    }
}

fn filter_bounds(
    mut ink: Rect,
    border: Rect,
    functions: &[crate::graphics::FilterFunction],
    definitions: &crate::document::svg::SvgDefinitions,
    work: &super::work::Budget,
) -> Result<Option<Rect>, super::LayoutError> {
    use crate::graphics::FilterFunction;
    let mut references = HashMap::new();
    for function in functions {
        if let FilterFunction::Url(reference) = function {
            let Some(filter) = crate::svg::referenced_filter(reference, Some(border), definitions)
            else {
                return Ok(Some(ink));
            };
            let primitives = filter.primitives();
            let kernels = primitives
                .iter()
                .filter_map(|primitive| match primitive.kind() {
                    crate::svg::usvg::filter::Kind::ConvolveMatrix(fe) => {
                        Some(fe.matrix().data().len())
                    }
                    _ => None,
                })
                .fold(0_usize, usize::saturating_add);
            work.spend(
                primitives
                    .len()
                    .max(1)
                    .saturating_mul(primitives.len().max(1))
                    .saturating_add(kernels),
            )?;
            references.insert(reference, filter);
        }
    }
    for function in functions {
        work.spend(1)?;
        ink = match function {
            FilterFunction::Url(reference) => {
                let scale = crate::graphics::POINTS_PER_PIXEL;
                let source = crate::svg::usvg::Rect::from_xywh(
                    ink.x / scale,
                    ink.y / scale,
                    ink.width / scale,
                    ink.height / scale,
                );
                let Some(rect) = references[reference].support_bounds(source) else {
                    return Ok(None);
                };
                Rect {
                    x: rect.x() * scale,
                    y: rect.y() * scale,
                    width: rect.width() * scale,
                    height: rect.height() * scale,
                }
            }
            FilterFunction::Blur(deviation) => {
                crate::paint::outset(ink, [crate::paint::BLUR_EXTENT * deviation; 4])
            }
            FilterFunction::DropShadow {
                offset,
                deviation,
                color,
            } if color.alpha > 0.0 => {
                let shadow = crate::paint::outset(ink, [crate::paint::BLUR_EXTENT * deviation; 4]);
                ink.union(Rect {
                    x: shadow.x + offset[0],
                    y: shadow.y + offset[1],
                    ..shadow
                })
            }
            _ => ink,
        };
    }
    Ok(Some(ink))
}

fn descendant(
    nodes: &HashMap<PageStyleTarget, AnchorNode>,
    mut node: PageStyleTarget,
    ancestor: PageStyleTarget,
    work: &super::work::Budget,
) -> Result<bool, super::LayoutError> {
    loop {
        work.spend(1)?;
        if node == ancestor {
            return Ok(true);
        }
        let Some(parent) = nodes.get(&node).and_then(|node| node.parent) else {
            return Ok(false);
        };
        node = parent;
    }
}

fn polygon_area(polygon: &[crate::graphics::Point]) -> f32 {
    polygon
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let b = polygon[(i + 1) % polygon.len()];
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f32>()
        .abs()
}

fn clipped(
    rect: Rect,
    effects: &[(usize, Effect)],
    own: Option<PageStyleTarget>,
    work: &super::work::Budget,
) -> Result<Vec<Vec<crate::graphics::Point>>, super::LayoutError> {
    let mut pieces = vec![super::navigation::rect_corners(rect).to_vec()];
    for (_, effect) in effects.iter().rev() {
        if !matches!(effect, Effect::Region(Region::Transform(_), _)) && !effect.clips(own) {
            continue;
        }
        let vertices = pieces.iter().map(Vec::len).sum::<usize>().max(1);
        work.spend(match effect {
            Effect::Region(..) => vertices,
            Effect::Path(polygons, _) => {
                vertices.saturating_mul(polygons.iter().map(Vec::len).sum::<usize>().max(1))
            }
        })?;
        match effect {
            Effect::Region(region, _) => {
                pieces = pieces
                    .into_iter()
                    .map(|polygon| super::navigation::map_region(polygon, *region))
                    .filter(|polygon| !polygon.is_empty())
                    .collect();
            }
            Effect::Path(polygons, _) => {
                pieces = pieces
                    .iter()
                    .flat_map(|piece| {
                        polygons
                            .iter()
                            .map(move |boundary| super::float_area::intersect(piece, boundary))
                    })
                    .filter(|polygon| !polygon.is_empty())
                    .collect();
            }
        }
    }
    Ok(pieces)
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
