//! Available-space provenance for orthogonal auto sizing (Writing Modes 4 §7.3).
use super::BlockSource;
use crate::layout::geometry::{HorizontalBox, VerticalBox};
use crate::style::{SizeConstraints, logical::Flow};

#[derive(Clone, Copy)]
pub(in crate::layout) struct FlowSpace {
    initial: [f32; 2],
    containing: [Option<f32>; 2],
    scrollport: [Option<f32>; 2],
    pub indefinite_inline: bool,
}
impl Default for FlowSpace {
    fn default() -> Self {
        Self {
            initial: [f32::INFINITY; 2],
            containing: [None; 2],
            scrollport: [None; 2],
            indefinite_inline: false,
        }
    }
}
#[derive(Clone, Hash, PartialEq, Eq)]
pub(super) struct SpaceKey {
    initial: [u32; 2],
    containing: [Option<u32>; 2],
    scrollport: [Option<u32>; 2],
    indefinite_inline: bool,
}
impl SpaceKey {
    pub(super) fn space(&self) -> FlowSpace {
        FlowSpace {
            initial: self.initial.map(f32::from_bits),
            containing: self.containing.map(|size| size.map(f32::from_bits)),
            scrollport: self.scrollport.map(|size| size.map(f32::from_bits)),
            indefinite_inline: self.indefinite_inline,
        }
    }
}
impl FlowSpace {
    pub(super) fn new(flow: Flow, mut size: [f32; 2]) -> Self {
        if flow.writing_mode.vertical() {
            size.swap(0, 1);
        }
        Self {
            initial: size,
            containing: size.map(Some),
            ..Self::default()
        }
    }
    pub(super) fn key(self) -> SpaceKey {
        SpaceKey {
            initial: self.initial.map(f32::to_bits),
            containing: self.containing.map(|size| size.map(f32::to_bits)),
            scrollport: self.scrollport.map(|size| size.map(f32::to_bits)),
            indefinite_inline: self.indefinite_inline,
        }
    }
    pub(in crate::layout) fn fallback(self, flow: Flow, containing: Option<f32>) -> f32 {
        let axis = usize::from(flow.writing_mode.vertical());
        containing
            .or(self.containing[axis])
            .into_iter()
            .chain(self.scrollport[axis])
            .fold(self.initial[axis], f32::min)
    }
    pub(in crate::layout) fn enter<'a>(
        mut self,
        block: impl Into<BlockSource<'a>>,
        geometry: &HorizontalBox,
        containing_height: Option<f32>,
        used_height: Option<f32>,
    ) -> Self {
        let block = block.into();
        let style = block.style();
        let horizontal_edges =
            geometry.padding[1] + geometry.padding[3] + geometry.border[1] + geometry.border[3];
        let vertical_edges =
            geometry.padding[0] + geometry.padding[2] + geometry.border[0] + geometry.border[2];
        let widths = SizeConstraints::new(
            [&style.width, &style.min_width, &style.max_width],
            (!self.indefinite_inline).then_some(geometry.containing_width),
            style.box_sizing,
            horizontal_edges,
        );
        let heights = SizeConstraints::new(
            [&style.height, &style.min_height, &style.max_height],
            containing_height,
            style.box_sizing,
            vertical_edges,
        );
        let vertical =
            VerticalBox::new(style, geometry, containing_height).with_used_height(used_height);
        let mut bounds = [
            if widths.specified.is_some()
                || (!self.indefinite_inline
                    && block.flow().is_none()
                    && style.width == crate::style::Size::Auto)
            {
                Some(geometry.width)
            } else {
                widths.maximum.map(|maximum| maximum.max(widths.minimum))
            },
            vertical
                .definite()
                .or_else(|| heights.maximum.map(|maximum| maximum.max(heights.minimum))),
        ];
        if block.layout_flow().writing_mode.vertical() {
            bounds.swap(0, 1);
        }
        self.containing = bounds;
        if style.is_scroll_container() {
            let mut padding = [
                geometry.padding[1] + geometry.padding[3],
                geometry.padding[0] + geometry.padding[2],
            ];
            if block.layout_flow().writing_mode.vertical() {
                padding.swap(0, 1);
            }
            self.scrollport =
                std::array::from_fn(|axis| bounds[axis].map(|bound| bound + padding[axis]));
        }
        if widths.specified.is_some() {
            self.indefinite_inline = false;
        }
        self
    }
}
