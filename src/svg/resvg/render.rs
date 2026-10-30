// Copyright 2018 the Resvg Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::svg::resvg::OptionLog;

pub(super) struct Context<'a> {
    pub max_bbox: tiny_skia::IntRect,
    /// The node rendering stops at, and whether it renders that node itself.
    until: Option<(&'a crate::svg::usvg::Node, bool)>,
    stopped: std::cell::Cell<bool>,
}

impl<'a> Context<'a> {
    pub(super) fn new(max_bbox: tiny_skia::IntRect) -> Self {
        Self {
            max_bbox,
            until: None,
            stopped: std::cell::Cell::new(false),
        }
    }

    /// A context that renders what is painted before `node`, or with `inclusive` up to and
    /// including it.
    pub(super) fn until(
        max_bbox: tiny_skia::IntRect,
        node: &'a crate::svg::usvg::Node,
        inclusive: bool,
    ) -> Self {
        Self {
            until: Some((node, inclusive)),
            ..Self::new(max_bbox)
        }
    }
}

pub(super) fn render_nodes(
    parent: &crate::svg::usvg::Group,
    ctx: &Context,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
) {
    for node in parent.children() {
        if ctx.stopped.get() {
            return;
        }
        let last = ctx
            .until
            .filter(|(until, _)| std::ptr::eq(node, *until))
            .map(|(_, inclusive)| inclusive);
        if last == Some(false) {
            ctx.stopped.set(true);
            return;
        }
        match node {
            // SVG 2 §13.9: the group's graphics children composite in its colour space.
            crate::svg::usvg::Node::Path(path) if parent.linear_light() => {
                crate::svg::resvg::path::render_in_linear_light(path, ctx, transform, pixmap);
            }
            crate::svg::usvg::Node::Image(_) if parent.linear_light() => {
                linear_layer(pixmap, |layer| render_node(node, ctx, transform, layer));
            }
            _ => render_node(node, ctx, transform, pixmap),
        }
        if last.is_some() {
            ctx.stopped.set(true);
            return;
        }
    }
}

/// Composites what `draw` paints onto `pixmap` in linear light.
pub(super) fn linear_layer(
    pixmap: &mut tiny_skia::PixmapMut,
    draw: impl FnOnce(&mut tiny_skia::PixmapMut),
) {
    let Some(mut layer) = tiny_skia::Pixmap::new(pixmap.width(), pixmap.height()) else {
        return;
    };
    draw(&mut layer.as_mut());
    draw_layer(
        layer.as_ref(),
        0,
        0,
        pixmap,
        1.0,
        crate::svg::usvg::BlendMode::Normal,
        true,
    );
}

/// Draws `layer` with its origin at `x`, `y` of `pixmap`, at `opacity`, composited by `mode`
/// in sRGB, or with `linear` in linear light. The additive operators and linear light
/// composite per pixel, which tiny-skia's 8-bit sRGB pipelines do not.
pub(super) fn draw_layer(
    layer: tiny_skia::PixmapRef,
    x: i32,
    y: i32,
    pixmap: &mut tiny_skia::PixmapMut,
    opacity: f32,
    mode: crate::svg::usvg::BlendMode,
    linear: bool,
) {
    use crate::paint::compositing;
    if let Some(blend_mode) = convert_blend_mode(mode).filter(|_| !linear) {
        pixmap.draw_pixmap(
            x,
            y,
            layer,
            &tiny_skia::PixmapPaint {
                opacity,
                blend_mode,
                quality: tiny_skia::FilterQuality::Nearest,
            },
            tiny_skia::Transform::identity(),
            None,
        );
        return;
    }
    let mode = css_blend_mode(mode);
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let columns = layer.width() as i32;
    let canvas = pixmap.pixels_mut();
    for (index, source) in layer.pixels().iter().enumerate() {
        let (column, row) = (index as i32 % columns + x, index as i32 / columns + y);
        if source.alpha() == 0 || !(0..width).contains(&column) || !(0..height).contains(&row) {
            continue;
        }
        let target = &mut canvas[(row * width + column) as usize];
        let source = compositing::sample(*source).map(|channel| channel * opacity);
        let backdrop = compositing::sample(*target);
        *target = compositing::pixel(if linear {
            compositing::from_linear(compositing::composite(
                mode,
                compositing::to_linear(source),
                compositing::to_linear(backdrop),
            ))
        } else {
            compositing::composite(mode, source, backdrop)
        });
    }
}

pub(super) fn render_node(
    node: &crate::svg::usvg::Node,
    ctx: &Context,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
) {
    match node {
        crate::svg::usvg::Node::Group(group) => {
            render_group(group, ctx, transform, pixmap);
        }
        crate::svg::usvg::Node::Path(path) => {
            crate::svg::resvg::path::render(
                path,
                tiny_skia::BlendMode::SourceOver,
                ctx,
                transform,
                pixmap,
            );
        }
        crate::svg::usvg::Node::Image(image) => {
            crate::svg::resvg::image::render(image, transform, pixmap);
        }
        crate::svg::usvg::Node::Text(text) => {
            render_group(text.flattened(), ctx, transform, pixmap);
        }
    }
}

fn render_group(
    group: &crate::svg::usvg::Group,
    ctx: &Context,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::PixmapMut,
) -> Option<()> {
    let transform = transform.pre_concat(group.transform());

    if !group.should_isolate() {
        render_nodes(group, ctx, transform, pixmap);
        return Some(());
    }

    let bbox = group.layer_bounding_box().transform(transform)?;

    let mut ibbox = if group.filters().is_empty() {
        // Convert group bbox into an integer one, expanding each side outwards by 2px
        // to make sure that anti-aliased pixels would not be clipped.
        tiny_skia::IntRect::from_xywh(
            (bbox.x().floor() as i32).checked_sub(2)?,
            (bbox.y().floor() as i32).checked_sub(2)?,
            (bbox.width().ceil() as u32).checked_add(4)?,
            (bbox.height().ceil() as u32).checked_add(4)?,
        )?
    } else {
        // The bounding box for groups with filters is special and should not be expanded by 2px,
        // because it's already acting as a clipping region.
        let bbox = bbox.to_int_rect();
        // Make sure our filter region is not bigger than 4x the canvas size.
        // This is required mainly to prevent huge filter regions that would tank the performance.
        // It should not affect the final result in any way.
        crate::svg::resvg::geom::fit_to_rect(bbox, ctx.max_bbox)?
    };

    // Make sure our layer is not bigger than 4x the canvas size.
    // This is required to prevent huge layers.
    if group.filters().is_empty() {
        ibbox = crate::svg::resvg::geom::fit_to_rect(ibbox, ctx.max_bbox)?;
    }

    let shift_ts = {
        // Original shift.
        let mut dx = bbox.x();
        let mut dy = bbox.y();

        // Account for subpixel positioned layers.
        dx -= bbox.x() - ibbox.x() as f32;
        dy -= bbox.y() - ibbox.y() as f32;

        tiny_skia::Transform::from_translate(-dx, -dy)
    };

    let transform = shift_ts.pre_concat(transform);

    let mut sub_pixmap = tiny_skia::Pixmap::new(ibbox.width(), ibbox.height())
        .log_none(|| log::warn!("Failed to allocate a group layer for: {:?}.", ibbox))?;

    render_nodes(group, ctx, transform, &mut sub_pixmap.as_mut());

    if !group.filters().is_empty() {
        for filter in group.filters() {
            crate::svg::resvg::filter::apply(filter, transform, &mut sub_pixmap);
        }
    }

    if let Some(clip_path) = group.clip_path() {
        crate::svg::resvg::clip::apply(clip_path, transform, &mut sub_pixmap);
    }

    if let Some(mask) = group.mask() {
        crate::svg::resvg::mask::apply(mask, ctx, transform, &mut sub_pixmap);
    }

    draw_layer(
        sub_pixmap.as_ref(),
        ibbox.x(),
        ibbox.y(),
        pixmap,
        group.opacity().get(),
        group.blend_mode(),
        group.linear_light(),
    );

    Some(())
}

/// The tiny-skia blend mode compositing like `mode`, which the additive operators lack.
fn convert_blend_mode(mode: crate::svg::usvg::BlendMode) -> Option<tiny_skia::BlendMode> {
    Some(match mode {
        crate::svg::usvg::BlendMode::Normal => tiny_skia::BlendMode::SourceOver,
        crate::svg::usvg::BlendMode::Multiply => tiny_skia::BlendMode::Multiply,
        crate::svg::usvg::BlendMode::Screen => tiny_skia::BlendMode::Screen,
        crate::svg::usvg::BlendMode::Overlay => tiny_skia::BlendMode::Overlay,
        crate::svg::usvg::BlendMode::Darken => tiny_skia::BlendMode::Darken,
        crate::svg::usvg::BlendMode::Lighten => tiny_skia::BlendMode::Lighten,
        crate::svg::usvg::BlendMode::ColorDodge => tiny_skia::BlendMode::ColorDodge,
        crate::svg::usvg::BlendMode::ColorBurn => tiny_skia::BlendMode::ColorBurn,
        crate::svg::usvg::BlendMode::HardLight => tiny_skia::BlendMode::HardLight,
        crate::svg::usvg::BlendMode::SoftLight => tiny_skia::BlendMode::SoftLight,
        crate::svg::usvg::BlendMode::Difference => tiny_skia::BlendMode::Difference,
        crate::svg::usvg::BlendMode::Exclusion => tiny_skia::BlendMode::Exclusion,
        crate::svg::usvg::BlendMode::Hue => tiny_skia::BlendMode::Hue,
        crate::svg::usvg::BlendMode::Saturation => tiny_skia::BlendMode::Saturation,
        crate::svg::usvg::BlendMode::Color => tiny_skia::BlendMode::Color,
        crate::svg::usvg::BlendMode::Luminosity => tiny_skia::BlendMode::Luminosity,
        crate::svg::usvg::BlendMode::PlusDarker | crate::svg::usvg::BlendMode::PlusLighter => {
            return None;
        }
    })
}

fn css_blend_mode(mode: crate::svg::usvg::BlendMode) -> crate::paint::BlendMode {
    use crate::paint::BlendMode as Css;
    match mode {
        crate::svg::usvg::BlendMode::Normal => Css::Normal,
        crate::svg::usvg::BlendMode::Multiply => Css::Multiply,
        crate::svg::usvg::BlendMode::Screen => Css::Screen,
        crate::svg::usvg::BlendMode::Overlay => Css::Overlay,
        crate::svg::usvg::BlendMode::Darken => Css::Darken,
        crate::svg::usvg::BlendMode::Lighten => Css::Lighten,
        crate::svg::usvg::BlendMode::ColorDodge => Css::ColorDodge,
        crate::svg::usvg::BlendMode::ColorBurn => Css::ColorBurn,
        crate::svg::usvg::BlendMode::HardLight => Css::HardLight,
        crate::svg::usvg::BlendMode::SoftLight => Css::SoftLight,
        crate::svg::usvg::BlendMode::Difference => Css::Difference,
        crate::svg::usvg::BlendMode::Exclusion => Css::Exclusion,
        crate::svg::usvg::BlendMode::Hue => Css::Hue,
        crate::svg::usvg::BlendMode::Saturation => Css::Saturation,
        crate::svg::usvg::BlendMode::Color => Css::Color,
        crate::svg::usvg::BlendMode::Luminosity => Css::Luminosity,
        crate::svg::usvg::BlendMode::PlusDarker => Css::PlusDarker,
        crate::svg::usvg::BlendMode::PlusLighter => Css::PlusLighter,
    }
}
