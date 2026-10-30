// Copyright 2017 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/*!
[resvg](https://github.com/linebender/resvg) is an SVG rendering library.
*/

#![allow(
    clippy::pedantic,
    clippy::collapsible_if,
    clippy::field_reassign_with_default,
    clippy::identity_op,
    clippy::too_many_arguments,
    clippy::wrong_self_convention,
    elided_lifetimes_in_paths,
    reason = "inlined upstream code keeps its upstream style"
)]

mod clip;
pub(crate) mod filter;
mod geom;
mod image;
mod mask;
mod path;
mod render;

/// Renders a tree onto the pixmap.
///
/// `transform` will be used as a root transform.
/// Can be used to position SVG inside the `pixmap`.
///
/// The produced content is in the sRGB color space.
pub(crate) fn render(
    tree: &crate::svg::usvg::Tree,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
    layer_masks: Option<&dyn crate::svg::LayerMasks>,
) {
    let ctx = context(pixmap.width(), pixmap.height(), layer_masks);
    render::render_nodes(tree.root(), &ctx, transform, pixmap);
}

/// Multiplies the pixmap's alpha by the values of `mask` placed by `transform`.
pub(crate) fn apply_mask(
    mask: &crate::svg::usvg::Mask,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::Pixmap,
    layer_masks: Option<&dyn crate::svg::LayerMasks>,
) {
    let ctx = context(pixmap.width(), pixmap.height(), layer_masks);
    mask::apply(mask, &ctx, transform, pixmap);
}

/// Renders the children of `group` that paint before `node`, or with `inclusive` up to and
/// including it, with `transform` placing the group's content space onto the pixmap.
pub(crate) fn render_prefix(
    group: &crate::svg::usvg::Group,
    node: &crate::svg::usvg::Node,
    inclusive: bool,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
    layer_masks: Option<&dyn crate::svg::LayerMasks>,
) {
    let ctx = render::Context::until(
        context(pixmap.width(), pixmap.height(), layer_masks).max_bbox,
        node,
        inclusive,
        layer_masks,
    );
    render::render_nodes(group, &ctx, transform, pixmap);
}

/// Renders `nodes`, children of `parent`, with `transform` placing the parent's content space
/// onto the pixmap.
pub(crate) fn render_children(
    parent: &crate::svg::usvg::Group,
    nodes: &[crate::svg::usvg::Node],
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
    layer_masks: Option<&dyn crate::svg::LayerMasks>,
) {
    let ctx = context(pixmap.width(), pixmap.height(), layer_masks);
    render::render_children(parent, nodes, &ctx, transform, pixmap);
}

/// Renders `group`'s content with its effects, whatever its blend mode, with `transform`
/// placing that content onto the pixmap.
pub(crate) fn render_layer(
    group: &crate::svg::usvg::Group,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
    layer_masks: Option<&dyn crate::svg::LayerMasks>,
) {
    let ctx = context(pixmap.width(), pixmap.height(), layer_masks);
    render::render_layer(group, &ctx, transform, pixmap);
}

fn context(
    width: u32,
    height: u32,
    layer_masks: Option<&dyn crate::svg::LayerMasks>,
) -> render::Context<'_> {
    let max_bbox = tiny_skia::IntRect::from_xywh(
        -(width as i32) * 2,
        -(height as i32) * 2,
        width * 5,
        height * 5,
    )
    .unwrap();
    render::Context::new(max_bbox, layer_masks)
}

/// Renders a node onto the pixmap.
///
/// `transform` will be used as a root transform.
/// Can be used to position SVG inside the `pixmap`.
///
/// The expected pixmap size can be retrieved from `crate::svg::usvg::Node::abs_layer_bounding_box()`.
///
/// Returns `None` when `node` has a zero size.
///
/// The produced content is in the sRGB color space.
pub(crate) fn render_node(
    node: &crate::svg::usvg::Node,
    mut transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
    layer_masks: Option<&dyn crate::svg::LayerMasks>,
) -> Option<()> {
    let bbox = node.abs_layer_bounding_box()?;

    let target_size = tiny_skia::IntSize::from_wh(pixmap.width(), pixmap.height()).unwrap();
    let max_bbox = tiny_skia::IntRect::from_xywh(
        -(target_size.width() as i32) * 2,
        -(target_size.height() as i32) * 2,
        target_size.width() * 5,
        target_size.height() * 5,
    )
    .unwrap();

    transform = transform.pre_translate(-bbox.x(), -bbox.y());

    let ctx = render::Context::new(max_bbox, layer_masks);
    render::render_node(node, &ctx, transform, pixmap);

    Some(())
}

pub(crate) trait OptionLog {
    fn log_none<F: FnOnce()>(self, f: F) -> Self;
}

impl<T> OptionLog for Option<T> {
    #[inline]
    fn log_none<F: FnOnce()>(self, f: F) -> Self {
        self.or_else(|| {
            f();
            None
        })
    }
}
