//! Nodes whose compositing with their backdrop no PDF blend mode expresses: groups blending by
//! the Compositing 2 §9.1.14–§9.1.15 additive operators, and paint composited in linear light
//! by SVG 2 §13.9 `color-interpolation: linearRGB`. As the document's additive groups do, the
//! backdrop the node's stacking context painted before it is read back in pixels, the node is
//! composited onto it there, and the result is drawn over the vector backdrop as the colour
//! that source-over compositing turns into it. Elsewhere the read-back matches the backdrop
//! and nothing is drawn.
use crate::paint::compositing;
use crate::svg::usvg::{BlendMode, Group, ImageKind, Node, Paint, Transform};
use krilla::surface::Surface;

use crate::svg::krilla_svg::util::{UsvgTransformExt, draw_pixmap};
use crate::svg::krilla_svg::{ProcessContext, group};

/// Device pixels added around the node's bounds for its anti-aliased edges.
const EDGE_PIXELS: f32 = 1.0;
/// The most pixels a read-back spans in either direction, as for rasterized filters.
const PIXEL_LIMIT: f32 = 5000.0;

/// The stacking context content paints into: its root group, whose children paint the
/// backdrop, the transform from the current drawing space to the root's content space, and
/// how many page units a root unit spans. Surfaces measure their transforms from the start of
/// their transparency group, so the scale is carried down from the stacking context's entry.
#[derive(Clone, Copy)]
pub(crate) struct Backdrop<'r> {
    root: &'r Group,
    to_root: Transform,
    scale: f32,
}
impl<'r> Backdrop<'r> {
    /// The stacking context of `root`, whose content space `to_page` maps onto the page.
    pub(crate) fn new(root: &'r Group, to_page: Transform) -> Self {
        Self {
            root,
            to_root: Transform::identity(),
            scale: scale(to_page),
        }
    }
    /// The backdrop of the content of a non-isolated group with `transform`.
    pub(crate) fn within(self, transform: Transform) -> Self {
        Self {
            to_root: self.to_root.pre_concat(transform),
            ..self
        }
    }
    /// The stacking context of an isolated group with `transform` painting onto this backdrop.
    pub(crate) fn nested(self, root: &'r Group, transform: Transform) -> Self {
        Self {
            root,
            to_root: Transform::identity(),
            scale: self.scale * scale(self.to_root.pre_concat(transform)),
        }
    }
}

impl Backdrop<'_> {
    /// Raster pixels per unit of the current drawing space at `filter_scale` pixels per page
    /// unit.
    pub(crate) fn density(&self, filter_scale: f32) -> f32 {
        filter_scale * self.scale * scale(self.to_root)
    }
}

/// Raster pixels per unit of a space that `to_page` maps onto the page, at `filter_scale`
/// pixels per page unit.
pub(crate) fn density(to_page: Transform, filter_scale: f32) -> f32 {
    filter_scale * scale(to_page)
}

/// The larger of the axis scales of `transform`.
fn scale(transform: Transform) -> f32 {
    let (x, y) = transform.get_scale();
    x.max(y)
}

/// Whether `node`, a child of `parent`, composites with its backdrop in a way only pixels
/// express. Opaque paint composites to itself in either colour space.
pub(crate) fn in_pixels(node: &Node, parent: &Group) -> bool {
    match node {
        Node::Group(group) => {
            matches!(
                group.blend_mode(),
                BlendMode::PlusDarker | BlendMode::PlusLighter
            ) || (group.linear_light() && (group::isolates(group) || !group.filters().is_empty()))
                // Projected content composites in the pixels of its plane.
                || ((group.projection().is_some() || group.is_scene()) && any_in_pixels(group))
        }
        Node::Path(path) => {
            parent.linear_light()
                && (path
                    .fill()
                    .is_some_and(|fill| translucent(fill.paint(), fill.opacity().get()))
                    || path
                        .stroke()
                        .is_some_and(|stroke| translucent(stroke.paint(), stroke.opacity().get())))
        }
        Node::Image(image) => parent.linear_light() && !matches!(image.kind(), ImageKind::JPEG(_)),
        // Text paints as glyph runs, so it reads back whole if anything in it needs pixels.
        Node::Text(text) => any_in_pixels(text.flattened()),
    }
}

fn any_in_pixels(group: &Group) -> bool {
    group.children().iter().any(|node| {
        in_pixels(node, group) || matches!(node, Node::Group(child) if any_in_pixels(child))
    })
}

/// Paint that may leave the backdrop showing through; pattern tiles may.
fn translucent(paint: &Paint, opacity: f32) -> bool {
    opacity < 1.0
        || match paint {
            Paint::Color(_) => false,
            Paint::LinearGradient(gradient) => gradient
                .stops()
                .iter()
                .any(|stop| stop.opacity().get() < 1.0),
            Paint::RadialGradient(gradient) => gradient
                .stops()
                .iter()
                .any(|stop| stop.opacity().get() < 1.0),
            Paint::Pattern(_) => true,
        }
}

/// Draws `node`, which paints onto `backdrop`, as its composite with what the backdrop's root
/// painted before it.
pub(crate) fn render(
    node: &Node,
    backdrop: Backdrop<'_>,
    surface: &mut Surface,
    process_context: &ProcessContext,
) -> Option<()> {
    let bounds = match node {
        Node::Group(group) => group
            .placed(group.layer_bounding_box().to_rect())?
            .to_non_zero_rect()?,
        _ => node.stroke_bounding_box().to_non_zero_rect()?,
    }
    .transform(backdrop.to_root)?;
    let from_root = backdrop.to_root.invert()?;
    // Pixels per root unit at the density the SVG renderer rasterizes filters at.
    let scale = (backdrop.scale * process_context.svg_settings.filter_scale)
        .min(PIXEL_LIMIT / bounds.width().max(bounds.height()));
    let left = (bounds.left() * scale - EDGE_PIXELS).floor();
    let top = (bounds.top() * scale - EDGE_PIXELS).floor();
    let width = ((bounds.right() * scale + EDGE_PIXELS).ceil() - left) as u32;
    let height = ((bounds.bottom() * scale + EDGE_PIXELS).ceil() - top) as u32;
    let to_pixels = Transform::from_row(scale, 0.0, 0.0, scale, -left, -top);
    let layer_masks = process_context.svg_settings.layer_masks;
    let mut below = tiny_skia::Pixmap::new(width, height)?;
    crate::svg::resvg::render_prefix(
        backdrop.root,
        node,
        false,
        to_pixels,
        &mut below.as_mut(),
        layer_masks,
    );
    let mut result = tiny_skia::Pixmap::new(width, height)?;
    crate::svg::resvg::render_prefix(
        backdrop.root,
        node,
        true,
        to_pixels,
        &mut result.as_mut(),
        layer_masks,
    );
    for (pixel, below) in result.pixels_mut().iter_mut().zip(below.pixels()) {
        *pixel = if pixel == below {
            tiny_skia::PremultipliedColorU8::TRANSPARENT
        } else {
            compositing::pixel(compositing::over_color(
                compositing::sample(*pixel),
                compositing::sample(*below),
            ))
        };
    }
    let area = krilla::geom::Rect::from_xywh(
        left / scale,
        top / scale,
        width as f32 / scale,
        height as f32 / scale,
    )?;
    surface.push_transform(&from_root.to_krilla());
    draw_pixmap(&result, area, surface);
    surface.pop();
    Some(())
}
