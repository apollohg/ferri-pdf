//! CSS Transforms 2 §4.2 SVG content under a perspective whose `w` varies across it, drawn as
//! the document draws such layers: text clusters draw through the projection's tangent where
//! that strays within the resampling tolerance, groups holding such text or blending with
//! what lies below them open in place, and the rest renders to pixels in the content's plane,
//! resampled through the projection. 3D rendering contexts (§4.1.2) draw the depth-sorted
//! pieces of their planes likewise.
use crate::paint::{
    Projection, depth,
    projection::{self, affine_over},
    resampling::{Plane, PlaneError, Resampled},
};
use crate::svg::krilla_svg::util::{UsvgTransformExt, convert_blend_mode, draw_pixmap};
use crate::svg::krilla_svg::{ProcessContext, composite::Backdrop, group, text};
use crate::svg::usvg::{BlendMode, Group, Node, Transform};
use krilla::{num::NormalizedF32, paint::FillRule, surface::Surface};

/// Draws `group`, whose content `projection` maps into the current space, at `density` raster
/// pixels per unit of that space.
pub(crate) fn render(
    group: &Group,
    projection: Projection,
    density: f32,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    if let Some(plane) = plane(group, projection, density, process_context) {
        draw(group, &plane, surface, process_context);
    }
}

/// Draws the 3D rendering context whose planes are `group`'s children, which paint onto
/// `backdrop`, at `density` raster pixels per unit of the current space.
pub(crate) fn render_scene(
    group: &Group,
    backdrop: Result<Backdrop<'_>, Transform>,
    density: f32,
    surface: &mut Surface,
    process_context: &mut ProcessContext,
) {
    let planes = group.scene_planes();
    let geometry: Vec<_> = planes
        .iter()
        .map(|(plane, matrix)| (*matrix, plane.content_ink()))
        .collect();
    for piece in depth::back_to_front(&geometry) {
        let (content, matrix) = planes[piece.plane];
        let m = projection::of_plane(matrix);
        let outline: Vec<_> = piece
            .polygon
            .iter()
            .map(|&point| projection::apply(m, point))
            .collect();
        if piece.cut {
            let mut path = krilla::geom::PathBuilder::new();
            for (index, [x, y]) in outline.into_iter().enumerate() {
                if index == 0 {
                    path.move_to(x, y);
                } else {
                    path.line_to(x, y);
                }
            }
            path.close();
            let Some(path) = path.finish() else {
                continue;
            };
            surface.push_clip_path(&path, &FillRule::NonZero);
        }
        match affine_over(m, &piece.polygon) {
            Some([a, b, c, d, e, f]) => {
                let transform = Transform::from_row(a, b, c, d, e, f);
                surface.push_transform(&transform.to_krilla());
                let within = backdrop.map(|backdrop| backdrop.within(transform));
                group::render_affine(content, within, surface, process_context);
                surface.pop();
            }
            None => render(content, m, density, surface, process_context),
        }
        if piece.cut {
            surface.pop();
        }
    }
}

/// The plane of `group`'s content that `projection` maps into the current space, visible where
/// it projects into the tree's canvas.
fn plane(
    group: &Group,
    projection: Projection,
    density: f32,
    process_context: &ProcessContext,
) -> Option<Plane> {
    // A projected group's own transform is the identity: its absolute transform is its
    // parent's, from the current space to the canvas.
    let from_canvas = group.abs_transform().invert()?;
    let viewport = process_context.viewport;
    let visible = crate::svg::usvg::Rect::from_xywh(0.0, 0.0, viewport.width(), viewport.height())?
        .transform(from_canvas)?;
    Some(Plane::new(
        projection,
        crate::paint::Rect {
            x: visible.x(),
            y: visible.y(),
            width: visible.width(),
            height: visible.height(),
        },
        density,
    ))
}

/// The placement of `group`'s content in its parent's space as a projection.
fn placement(group: &Group) -> Projection {
    let transform = group.transform();
    group.projection().unwrap_or_else(|| {
        projection::from_affine([
            transform.sx,
            transform.ky,
            transform.kx,
            transform.sy,
            transform.tx,
            transform.ty,
        ])
    })
}

/// Whether `group` draws its effects in its plane's pixels, with all its content.
fn rasterized(group: &Group) -> bool {
    !group.filters().is_empty()
        || group.clip_path().is_some()
        || group.mask().is_some()
        || group.layer_mask().is_some()
        || group.is_scene()
}

/// Whether `group`, whose content `plane` projects, opens in place around its content: it
/// holds vector text, or content blending with what lies below it.
fn opens(group: &Group, plane: &Plane, process_context: &mut ProcessContext) -> bool {
    !rasterized(group)
        && group.children().iter().any(|node| match node {
            Node::Text(text) => text::fits(text, plane, process_context),
            Node::Group(child) => {
                child.blend_mode() != BlendMode::Normal
                    || opens(child, &plane.mapped(placement(child)), process_context)
            }
            Node::Path(_) | Node::Image(_) => false,
        })
}

/// Draws `group`, whose content `plane` projects into the current space.
fn draw(group: &Group, plane: &Plane, surface: &mut Surface, process_context: &mut ProcessContext) {
    let blend = group.blend_mode();
    surface.push_blend_mode(convert_blend_mode(&blend));
    if !opens(group, plane, process_context) {
        let layer_masks = process_context.svg_settings.layer_masks;
        if let Some(layer) = pixels(plane, group.content_ink(), |transform, pixels| {
            crate::svg::resvg::render_layer(group, transform, pixels, layer_masks);
        }) {
            embed(&layer, surface);
        }
        surface.pop();
        return;
    }
    let opacity = NormalizedF32::new(group.opacity().get()).unwrap_or(NormalizedF32::ONE);
    surface.push_opacity(opacity);
    if group::isolates(group) {
        surface.push_isolated();
    }
    let children = group.children();
    let mut run = 0;
    for (index, node) in children.iter().enumerate() {
        let opened = match node {
            Node::Text(text) => text::fits(text, plane, process_context),
            Node::Group(child) => {
                !rasterized(child)
                    && (child.blend_mode() != BlendMode::Normal
                        || opens(child, &plane.mapped(placement(child)), process_context))
            }
            Node::Path(_) | Node::Image(_) => false,
        };
        if !opened {
            continue;
        }
        draw_run(
            group,
            &children[run..index],
            plane,
            surface,
            process_context,
        );
        match node {
            Node::Text(text) => {
                text::render_projected(text, plane.projection, surface, process_context);
            }
            Node::Group(child) => {
                draw(
                    child,
                    &plane.mapped(placement(child)),
                    surface,
                    process_context,
                );
            }
            Node::Path(_) | Node::Image(_) => {}
        }
        run = index + 1;
    }
    draw_run(group, &children[run..], plane, surface, process_context);
    if group::isolates(group) {
        surface.pop();
    }
    surface.pop();
    surface.pop();
}

/// Draws `nodes`, children of `group` whose content `plane` projects, from pixels of the plane.
fn draw_run(
    group: &Group,
    nodes: &[Node],
    plane: &Plane,
    surface: &mut Surface,
    process_context: &ProcessContext,
) {
    if nodes.is_empty() {
        return;
    }
    let layer_masks = process_context.svg_settings.layer_masks;
    if let Some(layer) = pixels(plane, group.content_ink(), |transform, pixels| {
        crate::svg::resvg::render_children(group, nodes, transform, pixels, layer_masks);
    }) {
        embed(&layer, surface);
    }
}

/// The pixels of what `draw` paints within `ink` of `plane`, resampled through its projection;
/// `draw` takes the transform from the plane onto a band's pixels.
fn pixels(
    plane: &Plane,
    ink: Option<crate::paint::Rect>,
    draw: impl Fn(Transform, &mut tiny_skia::PixmapMut),
) -> Option<Resampled> {
    let rendered = plane.render(ink?, |band| -> Result<_, PlaneError> {
        let mut pixels =
            tiny_skia::Pixmap::new(band.size[0], band.size[1]).ok_or(PlaneError::Pixmap)?;
        let [a, b, c, d, e, f] = band.to_pixels;
        draw(Transform::from_row(a, b, c, d, e, f), &mut pixels.as_mut());
        Ok(Some(pixels))
    });
    match rendered {
        Ok(resampled) => resampled,
        Err(error) => {
            log::warn!("Failed to resample projected SVG content: {error:?}.");
            None
        }
    }
}

/// Draws resampled output pixels in place.
fn embed(layer: &Resampled, surface: &mut Surface) {
    if let Some(area) = krilla::geom::Rect::from_xywh(
        layer.area.x,
        layer.area.y,
        layer.area.width,
        layer.area.height,
    ) {
        draw_pixmap(&layer.pixmap, area, surface);
    }
}
