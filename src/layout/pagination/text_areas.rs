//! The text that `background-clip: text` layers paint within (CSS Backgrounds 4 §2.8): the
//! glyphs, decorations and emphasis marks of a box and its in-flow and floated descendants.
use super::{Effect, Paginator, Region, ScopedPaint, Subtree, TableTracks};
use crate::graphics::{DisplayItem, FlowPaint};
use crate::layout::LayoutError;
use std::{rc::Rc, sync::Arc};

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Whose text a box fragment's backgrounds clip to.
enum Owner {
    /// The box's own: the items below its node.
    Box(usize),
    /// A table track's or track group's: the items of the cells originating in it, below the
    /// node the background paints in.
    Tracks(Option<usize>, Rc<TableTracks>),
}

impl<'a> Paginator<'a, '_, 'a> {
    /// Gives every box fragment whose background clips to text the text area of its page.
    pub(super) fn attach_text_areas(
        &mut self,
        regions: &[Vec<(usize, Region)>],
    ) -> Result<(), LayoutError> {
        for page in 0..self.display_lists.len() {
            let mut owners = Vec::new();
            let underlays = self
                .underlays
                .iter()
                .filter(|((underlay_page, _), _)| *underlay_page == page)
                .flat_map(|(&(_, slot), paints)| {
                    (0..paints.len())
                        .map(move |index| (Slot::Underlay(slot, index), &paints[index]))
                });
            let flow = self.display_lists[page]
                .iter()
                .enumerate()
                .map(|(index, paint)| (Slot::Flow(index), paint));
            let foreground = self.foreground[page]
                .iter()
                .enumerate()
                .map(|(index, paint)| (Slot::Foreground(index), paint));
            for (slot, paint) in flow.chain(foreground).chain(underlays) {
                let DisplayItem::Box(fragment) = &paint.item else {
                    continue;
                };
                if !fragment.draw_background
                    || !fragment.decoration.backgrounds.clips_to_text()
                    || fragment.text_area.is_some()
                {
                    continue;
                }
                let owner = match paint.clip.map(|node| (node, &self.clips[node].effect)) {
                    Some((node, Effect::Subtree(Subtree::TextArea))) => Owner::Box(node),
                    Some((node, Effect::Subtree(Subtree::TableTracks(tracks)))) => {
                        Owner::Tracks(self.clips[node].parent, tracks.clone())
                    }
                    _ => unreachable!(
                        "a box whose backgrounds clip to text marks the text they clip to"
                    ),
                };
                owners.push((slot, owner, paint.native));
            }
            for (slot, owner, native) in owners {
                let mut area = match owner {
                    Owner::Box(node) => self.text_area(page, Some(node), None, regions)?,
                    Owner::Tracks(base, tracks) => {
                        self.text_area(page, base, Some(&tracks), regions)?
                    }
                };
                if let Some(space) = native {
                    // The owner's native space maps its box into the page; its text area
                    // already lies in the page's coordinates.
                    let inverse = kurbo::Affine::new(space.matrix().map(f64::from))
                        .inverse()
                        .as_coeffs()
                        .map(|value| value as f32);
                    area.items.insert(0, DisplayItem::PushTransform(inverse));
                    area.items.push(DisplayItem::Pop);
                }
                let paint = match slot {
                    Slot::Flow(index) => &mut self.display_lists[page][index],
                    Slot::Foreground(index) => &mut self.foreground[page][index],
                    Slot::Underlay(slot, index) => &mut self
                        .underlays
                        .get_mut(&(page, slot))
                        .expect("the underlay holds the fragment")[index],
                };
                if let DisplayItem::Box(fragment) = &mut paint.item {
                    fragment.text_area = Some(Arc::new(area));
                }
            }
        }
        Ok(())
    }

    /// The visible text items on `page` below `base` and not inside an out-of-flow box below
    /// it, and with `tracks`, inside one of their cells, opaque, inside the clips and
    /// transforms between `base` and them.
    pub(super) fn text_area(
        &self,
        page: usize,
        base: Option<usize>,
        tracks: Option<&TableTracks>,
        regions: &[Vec<(usize, Region)>],
    ) -> Result<FlowPaint, LayoutError> {
        let mut items = Vec::new();
        let mut lines = Vec::new();
        // Open clip or transform nodes and the pushes each emitted.
        let mut open: Vec<(usize, bool)> = Vec::new();
        for paint in self.display_lists[page]
            .iter()
            .chain(&self.foreground[page])
        {
            if !paint.visible
                || !matches!(
                    paint.item,
                    DisplayItem::Text { .. } | DisplayItem::Decoration(_)
                )
            {
                continue;
            }
            let Some(chain) = self.chain_below(paint, base) else {
                continue;
            };
            if let Some(tracks) = tracks
                && !chain.iter().any(|&index| {
                    matches!(&self.clips[index].effect, Effect::Subtree(cell) if tracks.holds(cell))
                })
            {
                continue;
            }
            let mut pushes = Vec::with_capacity(chain.len());
            for &index in &chain {
                pushes.push(match self.clips[index].effect {
                    Effect::PathClip(_) | Effect::Subtree(_) => None,
                    _ => Some(
                        regions[index]
                            .iter()
                            .find_map(|(fragment_page, region)| {
                                (*fragment_page == page).then(|| region.clone())
                            })
                            .ok_or(LayoutError::MissingClipFragment { page })?,
                    ),
                });
            }
            if pushes.iter().flatten().any(Region::collapses) {
                continue;
            }
            let common = open
                .iter()
                .zip(&chain)
                .take_while(|((open, _), index)| open == *index)
                .count();
            for (_, pushed) in open.drain(common..).rev() {
                if pushed {
                    items.push(DisplayItem::Pop);
                }
            }
            for (&index, region) in chain.iter().zip(pushes).skip(common) {
                open.push((index, region.is_some()));
                items.extend(region.map(Region::push));
            }
            let item = match &paint.item {
                DisplayItem::Text { line } => {
                    lines.push(self.pages[page][*line].clone().opaque());
                    DisplayItem::Text {
                        line: lines.len() - 1,
                    }
                }
                DisplayItem::Decoration(decoration) => DisplayItem::Decoration(decoration.opaque()),
                _ => unreachable!("text areas hold only text and decorations"),
            };
            match paint.native {
                Some(space) => items.extend(space.paint(item)),
                None => items.push(item),
            }
        }
        items.extend(
            open.iter()
                .filter(|(_, pushed)| *pushed)
                .map(|_| DisplayItem::Pop),
        );
        Ok(FlowPaint {
            matrix: IDENTITY,
            area: self.fragmentainers.content_box(page),
            items,
            lines,
        })
    }

    /// The effect nodes between `base` and `paint`, outermost first, when `paint` lies below
    /// `base`, or anywhere without one, outside any out-of-flow box below it.
    fn chain_below(&self, paint: &ScopedPaint, base: Option<usize>) -> Option<Vec<usize>> {
        let mut chain = Vec::new();
        let mut next = paint.clip;
        loop {
            if next == base {
                chain.reverse();
                return Some(chain);
            }
            let index = next?;
            if matches!(
                self.clips[index].effect,
                Effect::Subtree(Subtree::OutOfFlow)
            ) {
                return None;
            }
            chain.push(index);
            next = self.clips[index].parent;
        }
    }
}

/// Where a box fragment lies in a page's paints: its flow list, its foreground list, or the
/// underlays of a flow item.
#[derive(Clone, Copy)]
enum Slot {
    Flow(usize),
    Foreground(usize),
    Underlay(usize, usize),
}
