//! The text that `background-clip: text` layers paint within (CSS Backgrounds 4 §2.8): the
//! glyphs, decorations and emphasis marks of a box and its in-flow and floated descendants.
use super::{Effect, Paginator, Region, ScopedPaint, Subtree};
use crate::graphics::{DisplayItem, FlowPaint};
use crate::layout::LayoutError;
use std::sync::Arc;

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

impl<'a> Paginator<'a, '_, 'a> {
    /// Gives every box fragment whose background clips to text the text area of its page.
    pub(super) fn attach_text_areas(
        &mut self,
        regions: &[Vec<(usize, Region)>],
    ) -> Result<(), LayoutError> {
        for page in 0..self.display_lists.len() {
            let mut owners = Vec::new();
            for (foreground, list) in [&self.display_lists[page], &self.foreground[page]]
                .into_iter()
                .enumerate()
            {
                for (slot, paint) in list.iter().enumerate() {
                    let DisplayItem::Box(fragment) = &paint.item else {
                        continue;
                    };
                    if !fragment.draw_background || !fragment.decoration.backgrounds.clips_to_text()
                    {
                        continue;
                    }
                    let node = paint
                        .clip
                        .filter(|&node| {
                            matches!(self.clips[node].effect, Effect::Subtree(Subtree::TextArea))
                        })
                        .ok_or_else(|| {
                            crate::style::StyleError::Unsupported(
                                "background-clip: text on a table cell, row, column or group"
                                    .into(),
                            )
                        })?;
                    owners.push((foreground == 1, slot, node, paint.native));
                }
            }
            for (foreground, slot, node, native) in owners {
                let mut area = self.text_area(page, node, regions)?;
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
                let list = if foreground {
                    &mut self.foreground[page]
                } else {
                    &mut self.display_lists[page]
                };
                if let DisplayItem::Box(fragment) = &mut list[slot].item {
                    fragment.text_area = Some(Arc::new(area));
                }
            }
        }
        Ok(())
    }

    /// The visible text items on `page` below `node` and not inside an out-of-flow box below
    /// it, opaque, inside the clips and transforms between `node` and them.
    pub(super) fn text_area(
        &self,
        page: usize,
        node: usize,
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
            let Some(chain) = self.chain_below(paint, node) else {
                continue;
            };
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
            if pushes.iter().flatten().any(|region| match region {
                Region::Transform([a, b, c, d, _, _]) => {
                    f64::from(*a) * f64::from(*d) - f64::from(*b) * f64::from(*c) == 0.0
                }
                Region::Clip(_) => false,
            }) {
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
                items.extend(region.map(|region| match region {
                    Region::Clip(clip) => DisplayItem::PushClip(clip),
                    Region::Transform(matrix) => DisplayItem::PushTransform(matrix),
                }));
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

    /// The effect nodes between `node` and `paint`, outermost first, when `paint` lies below
    /// `node` outside any out-of-flow box below it.
    fn chain_below(&self, paint: &ScopedPaint, node: usize) -> Option<Vec<usize>> {
        let mut chain = Vec::new();
        let mut next = paint.clip;
        while let Some(index) = next {
            if index == node {
                chain.reverse();
                return Some(chain);
            }
            if matches!(
                self.clips[index].effect,
                Effect::Subtree(Subtree::OutOfFlow)
            ) {
                return None;
            }
            chain.push(index);
            next = self.clips[index].parent;
        }
        None
    }
}
