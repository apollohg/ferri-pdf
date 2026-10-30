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
) {
    let ctx = context(pixmap.width(), pixmap.height());
    render::render_nodes(tree.root(), &ctx, transform, pixmap);
}

/// Multiplies the pixmap's alpha by the values of `mask` placed by `transform`.
pub(crate) fn apply_mask(
    mask: &crate::svg::usvg::Mask,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::Pixmap,
) {
    let ctx = context(pixmap.width(), pixmap.height());
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
) {
    let ctx = render::Context::until(
        context(pixmap.width(), pixmap.height()).max_bbox,
        node,
        inclusive,
    );
    render::render_nodes(group, &ctx, transform, pixmap);
}

fn context(width: u32, height: u32) -> render::Context<'static> {
    let max_bbox = tiny_skia::IntRect::from_xywh(
        -(width as i32) * 2,
        -(height as i32) * 2,
        width * 5,
        height * 5,
    )
    .unwrap();
    render::Context::new(max_bbox)
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

    let ctx = render::Context::new(max_bbox);
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
