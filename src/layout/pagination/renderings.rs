//! Elements rendered as images for `element()` (CSS Images 4 §2.7): an element that generates
//! a stacking context and its descendants painted over a transparent canvas the size of its
//! decorated bounding box, without the transforms of it and its ancestors. A fragmented
//! element is displayed as if the page content areas were joined flush in the pagination
//! direction, with their inline-start edges aligned.
use super::{
    Effect, EffectNode, Located, Paginator, Region, Rendered, ScopedPaint, Subtree, clipped,
    outlines,
};
use crate::dom::ElementId;
use crate::graphics::{BoxPaint, DisplayItem, FlowPaint, Rect};
use crate::layout::LayoutError;
use std::collections::{HashMap, HashSet};

/// An element's rendering: its pages' items placed in its decorated bounding box, physical
/// and starting at the origin, which spans `size` points.
pub(crate) struct Rendering {
    pub items: Vec<DisplayItem>,
    pub size: [f32; 2],
}

/// What a layout found of the elements that `element()` images show: the renderings of those
/// that generate a stacking context, and those whose boxes generate none.
#[derive(Default)]
pub(crate) struct Renderings {
    pub stacked: HashMap<ElementId, Rendering>,
    pub unstacked: HashSet<ElementId>,
}

/// Which items an element's rendering paints, and how much of their effects.
pub(super) struct Cut<'c> {
    pub(super) element: ElementId,
    /// The groups compositing the element's stacking context, the outermost groups its
    /// items keep; none for the root element, whose items keep every group.
    pub(super) groups: &'c HashSet<usize>,
}
impl Cut<'_> {
    /// The effect nodes of `paint` below the element's box, or below the containing block
    /// of an absolutely positioned descendant outside it; `None` when `paint` lies outside
    /// the element.
    pub(super) fn chain(
        &self,
        paint: &ScopedPaint,
        nodes: &[EffectNode<'_>],
    ) -> Option<Vec<usize>> {
        let mut chain = Vec::new();
        let mut next = paint.clip;
        while let Some(index) = next {
            match &nodes[index].effect {
                Effect::Subtree(Subtree::Element(element, _)) if *element == self.element => {
                    chain.reverse();
                    return Some(chain);
                }
                Effect::Subtree(Subtree::Within(elements)) if elements.contains(&self.element) => {
                    chain.reverse();
                    return Some(chain);
                }
                _ => chain.push(index),
            }
            next = nodes[index].parent;
        }
        None
    }
    /// Whether the element's rendering keeps no group enclosing `group`.
    pub(super) fn outermost(&self, group: usize) -> bool {
        self.groups.contains(&group)
    }
}

impl<'a> Paginator<'a, '_, 'a> {
    /// The renderings of the elements that `element()` images show, from the pages' items
    /// before they are assembled; `rtl` and `vertical` describe the root's flow.
    pub(super) fn renderings(
        &self,
        regions: &[Vec<(usize, Region)>],
        bounds: &[Vec<(usize, BoxPaint)>],
        flow: crate::style::logical::Flow,
    ) -> Result<Renderings, LayoutError> {
        let mut renderings = Renderings::default();
        let mut stacked: HashMap<ElementId, (HashSet<usize>, Vec<usize>)> = HashMap::new();
        for (index, node) in self.clips.iter().enumerate() {
            match &node.effect {
                Effect::Subtree(Subtree::Element(element, Rendered::Unstacked)) => {
                    renderings.unstacked.insert(*element);
                }
                Effect::Subtree(Subtree::Element(element, Rendered::Stacked { group })) => {
                    let (groups, nodes) = stacked.entry(*element).or_default();
                    groups.extend(*group);
                    nodes.push(index);
                }
                _ => {}
            }
        }
        for (element, (groups, nodes)) in stacked {
            renderings.unstacked.remove(&element);
            let cut = Cut {
                element,
                groups: &groups,
            };
            renderings.stacked.insert(
                element,
                self.rendering(&cut, &nodes, regions, bounds, flow)?,
            );
        }
        Ok(renderings)
    }

    fn rendering(
        &self,
        cut: &Cut<'_>,
        element_nodes: &[usize],
        regions: &[Vec<(usize, Region)>],
        bounds: &[Vec<(usize, BoxPaint)>],
        flow: crate::style::logical::Flow,
    ) -> Result<Rendering, LayoutError> {
        let mut pages = Vec::new();
        let mut decorated: Option<Rect> = None;
        let mut joined = 0.0;
        for page in 0..self.display_lists.len() {
            let area = self.fragmentainers.content_box(page);
            let anchor = if flow.rtl {
                area.x + area.width
            } else {
                area.x
            };
            let offset = [-anchor, joined - area.y];
            joined += area.height;
            for rect in self.principal_fragments(element_nodes, page) {
                let rect = Rect {
                    x: rect.x + offset[0],
                    y: rect.y + offset[1],
                    ..rect
                };
                decorated = Some(decorated.map_or(rect, |bounds| bounds.union(rect)));
            }
            let mut members: Vec<ScopedPaint> = self.display_lists[page]
                .iter()
                .enumerate()
                .flat_map(|(slot, item)| {
                    self.underlays
                        .get(&(page, slot))
                        .into_iter()
                        .flatten()
                        .chain(std::iter::once(item))
                })
                .chain(&self.foreground[page])
                .filter(|paint| cut.chain(paint, &self.clips).is_some())
                .cloned()
                .collect();
            if members.is_empty() {
                continue;
            }
            let outlined = outlines(&members, &self.clips);
            members.extend(outlined);
            members.sort_by(|a, b| a.scope.cmp(&b.scope).then_with(|| a.order.cmp(&b.order)));
            let mut located = Located {
                links: Vec::new(),
                destinations: Vec::new(),
                destination_bounds: Vec::new(),
            };
            let items = clipped(
                members,
                page,
                None,
                &self.clips,
                regions,
                bounds,
                &mut located,
                Some(cut),
            )?;
            let (items, lines) = own_lines(items, &self.pages[page]);
            pages.push((offset, area, items, lines));
        }
        let decorated = decorated.unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
        let mut items: Vec<DisplayItem> = pages
            .into_iter()
            .map(|([x, y], area, items, lines)| {
                DisplayItem::Frame(Box::new(FlowPaint {
                    matrix: [1.0, 0.0, 0.0, 1.0, x - decorated.x, y - decorated.y],
                    area,
                    items,
                    lines,
                }))
            })
            .collect();
        let logical = [decorated.width, decorated.height];
        let frame = crate::layout::writing::Frame::physical(flow, logical);
        frame.items(&mut items);
        let physical = frame.rect(Rect {
            x: 0.0,
            y: 0.0,
            width: logical[0],
            height: logical[1],
        });
        Ok(Rendering {
            items,
            size: [physical.width, physical.height],
        })
    }

    /// The border image areas of the element's principal box fragments on `page`, which its
    /// decorated bounding box contains.
    fn principal_fragments(&self, element_nodes: &[usize], page: usize) -> Vec<Rect> {
        element_nodes
            .iter()
            .flat_map(|&node| {
                let node = &self.clips[node];
                node.fragments
                    .iter()
                    .filter(|(fragment_page, _)| *fragment_page == page)
                    .filter_map(|&(_, slot)| match &self.display_lists[page][slot].item {
                        DisplayItem::Box(paint) => Some(paint.border_image_area()),
                        _ => None,
                    })
                    .chain(
                        node.paints
                            .iter()
                            .filter(|(fragment_page, _)| *fragment_page == page)
                            .map(|(_, paint)| paint.border_image_area()),
                    )
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

/// `items` with the lines of `page` their text items draw, renumbered into lines of their own.
fn own_lines(
    items: Vec<DisplayItem>,
    page: &[crate::layout::TextLine],
) -> (Vec<DisplayItem>, Vec<crate::layout::TextLine>) {
    let mut lines = Vec::new();
    let items = items
        .into_iter()
        .map(|item| match item {
            DisplayItem::Text { line } => {
                lines.push(page[line].clone());
                DisplayItem::Text {
                    line: lines.len() - 1,
                }
            }
            item => item,
        })
        .collect();
    (items, lines)
}
