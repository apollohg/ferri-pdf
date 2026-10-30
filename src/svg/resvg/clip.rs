// Copyright 2019 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::svg::resvg::render::Context;

pub(super) fn apply(
    clip: &crate::svg::usvg::ClipPath,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::Pixmap,
) {
    let mut clip_pixmap = tiny_skia::Pixmap::new(pixmap.width(), pixmap.height()).unwrap();
    clip_pixmap.fill(tiny_skia::Color::BLACK);

    draw_children(
        clip.root(),
        tiny_skia::BlendMode::Clear,
        transform.pre_concat(clip.transform()),
        &mut clip_pixmap.as_mut(),
    );

    if let Some(clip) = clip.clip_path() {
        apply(clip, transform, pixmap);
    }

    let mut mask = tiny_skia::Mask::from_pixmap(clip_pixmap.as_ref(), tiny_skia::MaskType::Alpha);
    mask.invert();
    pixmap.apply_mask(&mask);
}

fn draw_children(
    parent: &crate::svg::usvg::Group,
    mode: tiny_skia::BlendMode,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
) {
    for child in parent.children() {
        match child {
            crate::svg::usvg::Node::Path(path) => {
                if !path.is_visible() {
                    continue;
                }

                // We could use any values here. They will not be used anyway.
                let ctx = Context::new(tiny_skia::IntRect::from_xywh(0, 0, 1, 1).unwrap(), None);

                crate::svg::resvg::path::fill_path(path, mode, &ctx, transform, pixmap);
            }
            crate::svg::usvg::Node::Text(text) => {
                draw_children(text.flattened(), mode, transform, pixmap);
            }
            crate::svg::usvg::Node::Group(group) if group.projection().is_some() => {
                draw_projected(group, mode, transform, pixmap);
            }
            crate::svg::usvg::Node::Group(group) => {
                let transform = transform.pre_concat(group.transform());

                if let Some(clip) = group.clip_path() {
                    // If a `clipPath` child also has a `clip-path`
                    // then we should render this child on a new canvas,
                    // clip it, and only then draw it to the `clipPath`.
                    clip_group(group, clip, transform, pixmap);
                } else {
                    draw_children(group, mode, transform, pixmap);
                }
            }
            _ => {}
        }
    }
}

/// Draws the clipping shapes of `group`, whose content its projection maps into the space
/// `transform` places on the pixmap, from pixels of its plane (CSS Transforms 2 §11): their
/// coverage clears the clip or, with `SourceOver`, paints it.
fn draw_projected(
    group: &crate::svg::usvg::Group,
    mode: tiny_skia::BlendMode,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
) -> Option<()> {
    let ctx = Context::new(tiny_skia::IntRect::from_xywh(0, 0, 1, 1)?, None);
    let plane = crate::svg::resvg::render::canvas_plane(
        crate::svg::resvg::render::placement(group),
        transform,
        pixmap,
    );
    let coverage = crate::svg::resvg::render::plane_pixels(
        &plane,
        group.layer_bounding_box(),
        &ctx,
        |_, transform, pixels| {
            if let Some(clip) = group.clip_path() {
                clip_group(group, clip, transform, pixels);
            } else {
                draw_children(group, tiny_skia::BlendMode::SourceOver, transform, pixels);
            }
        },
    )?;
    let blend_mode = if mode == tiny_skia::BlendMode::Clear {
        tiny_skia::BlendMode::DestinationOut
    } else {
        mode
    };
    pixmap.draw_pixmap(
        coverage.area.x.round() as i32,
        coverage.area.y.round() as i32,
        coverage.pixmap.as_ref(),
        &tiny_skia::PixmapPaint {
            blend_mode,
            ..tiny_skia::PixmapPaint::default()
        },
        tiny_skia::Transform::identity(),
        None,
    );
    Some(())
}

fn clip_group(
    children: &crate::svg::usvg::Group,
    clip: &crate::svg::usvg::ClipPath,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
) -> Option<()> {
    let mut clip_pixmap = tiny_skia::Pixmap::new(pixmap.width(), pixmap.height()).unwrap();

    draw_children(
        children,
        tiny_skia::BlendMode::SourceOver,
        transform,
        &mut clip_pixmap.as_mut(),
    );
    apply(clip, transform, &mut clip_pixmap);

    let mut paint = tiny_skia::PixmapPaint::default();
    paint.blend_mode = tiny_skia::BlendMode::Xor;
    pixmap.draw_pixmap(
        0,
        0,
        clip_pixmap.as_ref(),
        &paint,
        tiny_skia::Transform::identity(),
        None,
    );

    Some(())
}
