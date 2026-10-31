//! Effects that read back what the display list painted before an element. The items since
//! the element's Backdrop Root or enclosing transparency group render to pixels in the space
//! where they started, so every paint reads back as it draws in place.
//!
//! Filter Effects 2 §2.1 `backdrop-filter`: the Backdrop Root Image clipped to each border
//! box fragment (§3) is filtered in the root's space, extended past the box by mirroring when
//! the chain blurs, and drawn behind the fragment clipped to its rounded border box, inside
//! the element's own effects.
//!
//! Compositing 2 §9.1.14 `plus-darker` and §9.1.15 `plus-lighter`: the group composites
//! additively with its backdrop, which no PDF blend mode computes; the result is drawn over the vector
//! backdrop as the colour that source-over compositing turns into that sum.
use crate::graphics::{
    BackdropPaint, BlendMode, DisplayItem, FilterFunction, FilterPaint, Projection, Rect,
};
use crate::paint::{compositing, projection};
use crate::writer::{
    PdfError,
    display::{Painter, Scopes, Space, group_length},
    filters, painting,
    raster::{self, Raster},
};
use krilla::surface::Surface;
use std::{collections::HashMap, ops::Range};

/// Pixels read back for display items, keyed by the item's address while the frame that
/// prepared them draws; a later read-back of the same items reuses them, since a re-drawn
/// item's own backdrop starts outside the slice being read back. `None` draws nothing.
#[derive(Default)]
pub(super) struct Prepared {
    values: HashMap<usize, Option<Readback>>,
    order: Vec<usize>,
}
impl Prepared {
    /// The current preparations, which `release` keeps.
    pub(super) fn mark(&self) -> usize {
        self.order.len()
    }
    /// Discards what was prepared since `mark`, whose items may no longer exist.
    pub(super) fn release(&mut self, mark: usize) {
        for key in self.order.drain(mark..) {
            self.values.remove(&key);
        }
    }
    fn insert(&mut self, item: &DisplayItem, readback: Option<Readback>) {
        let key = address(item);
        if self.values.insert(key, readback).is_none() {
            self.order.push(key);
        }
    }
    fn get(&self, item: &DisplayItem) -> Option<&Option<Readback>> {
        self.values.get(&address(item))
    }
}

fn address(item: &DisplayItem) -> usize {
    std::ptr::from_ref(item) as usize
}

/// A raster and the projection from the drawing space of the item it belongs to into the
/// raster's space.
struct Readback {
    raster: Raster,
    to_raster: Projection,
}

/// A drawing-space transform as a projection.
fn projected(transform: tiny_skia::Transform) -> Projection {
    projection::from_affine([
        transform.sx,
        transform.ky,
        transform.kx,
        transform.sy,
        transform.tx,
        transform.ty,
    ])
}

/// Prepares the backdrops of the element whose Backdrop Root opens at `root`, before anything
/// inside it draws, while the items before it are still at hand.
pub(super) fn prepare(
    painter: &mut Painter<'_, '_, '_>,
    items: &[DisplayItem],
    root: usize,
    scopes: &Scopes<'_>,
) -> Result<(), PdfError> {
    let content = root + 1;
    // A prefix being read back may end inside the root.
    let end = group_length(&items[content..]).map_or(items.len(), |length| content + length);
    let (origin, to_origin) = scopes.backdrop_root();
    for (item, backdrop, to_current) in own_backdrops(&items[content..end]) {
        if painter.prepared.get(item).is_some() {
            continue;
        }
        let to_root = projection::compose(projected(to_origin), to_current);
        let raster = filtered_backdrop(
            painter,
            &items[origin.index..root],
            origin.density,
            backdrop,
            to_root,
        )?;
        painter.prepared.insert(
            item,
            raster.map(|raster| Readback {
                raster,
                to_raster: to_root,
            }),
        );
    }
    Ok(())
}

/// The element's own backdrop items, outside the Backdrop Roots of its descendants, each
/// with the projection from its drawing space to the space `items` start in.
fn own_backdrops(items: &[DisplayItem]) -> Vec<(&DisplayItem, &BackdropPaint, Projection)> {
    enum Open {
        Transform(Projection),
        Root,
        Other,
    }
    let current = |open: &[Open]| {
        open.iter()
            .fold(projection::IDENTITY, |transform, scope| match scope {
                Open::Transform(matrix) => projection::compose(transform, *matrix),
                Open::Root | Open::Other => transform,
            })
    };
    let mut open = Vec::new();
    let mut backdrops = Vec::new();
    for item in items {
        match item {
            DisplayItem::PushTransform(matrix) => {
                open.push(Open::Transform(projection::from_affine(*matrix)));
            }
            DisplayItem::PushProjection(matrix) => open.push(Open::Transform(*matrix)),
            DisplayItem::PushBackdropRoot => open.push(Open::Root),
            DisplayItem::PushClip(_)
            | DisplayItem::PushOpacity(_)
            | DisplayItem::PushBlend(_)
            | DisplayItem::PushFilter(_)
            | DisplayItem::PushMask(_) => open.push(Open::Other),
            DisplayItem::Pop => {
                open.pop();
            }
            DisplayItem::Backdrop(backdrop)
                if !open.iter().any(|scope| matches!(scope, Open::Root)) =>
            {
                backdrops.push((item, backdrop, current(&open)));
            }
            DisplayItem::Scene(scene) if !open.iter().any(|scope| matches!(scope, Open::Root)) => {
                for plane in &scene.planes {
                    let placed =
                        projection::compose(current(&open), projection::of_plane(plane.matrix));
                    backdrops.extend(own_backdrops(&plane.items).into_iter().map(
                        |(item, backdrop, transform)| {
                            (item, backdrop, projection::compose(placed, transform))
                        },
                    ));
                }
            }
            DisplayItem::Backdrop(_)
            | DisplayItem::Scene(_)
            | DisplayItem::Frame(_)
            | DisplayItem::Image(_)
            | DisplayItem::Box(_)
            | DisplayItem::Text { .. }
            | DisplayItem::TextShadow(_)
            | DisplayItem::Decoration(_) => {}
        }
    }
    backdrops
}

/// The filtered Backdrop Root Image `prefix` paints behind `backdrop`'s border box, in the
/// root's space, which `to_root` maps the box's space into, or `None` when nothing draws.
fn filtered_backdrop(
    painter: &mut Painter<'_, '_, '_>,
    prefix: &[DisplayItem],
    density: f32,
    backdrop: &BackdropPaint,
    to_root: Projection,
) -> Result<Option<Raster>, PdfError> {
    let border = backdrop.border.rect;
    if !(border.width > 0.0 && border.height > 0.0) {
        return Ok(None);
    }
    let Some(area) = crate::writer::bounds::mapped(to_root, border) else {
        return Ok(None);
    };
    let filter = FilterPaint {
        functions: backdrop.functions.clone(),
        bounds: Some(area),
    };
    // Filter Effects 1 §5: a reference to anything but a filter element ignores the chain.
    let Some(steps) = filters::steps(&filter, painter.raster.definitions) else {
        return Ok(None);
    };
    let Some(image) = raster::render(painter, area, density, |painter, space, surface| {
        painter.draw_prefix(prefix, space, surface)
    })?
    else {
        return Ok(None);
    };
    let spread: f32 = steps.iter().map(|step| step.spread(density)).sum();
    let mirrored = backdrop
        .functions
        .iter()
        .any(|function| matches!(function, FilterFunction::Blur(_)));
    let mut input = clipped_input(&image, border, to_root, spread, mirrored)?;
    filters::apply(painter, &steps, &mut input)?;
    Ok(input.cropped(image.area))
}

/// `image` within the border quad, which `to_root` maps `border` onto, over an area grown by
/// `spread` for the filter to read: transparent outside the quad (§3), or with `mirrored`
/// the quad's content reflected across its edges, as `edgeMode="mirror"` extends it (§2.1).
fn clipped_input(
    image: &Raster,
    border: Rect,
    to_root: Projection,
    spread: f32,
    mirrored: bool,
) -> Result<Raster, PdfError> {
    let density = image.density;
    let margin = (spread * density).ceil();
    let (columns, rows) = (image.pixmap.width(), image.pixmap.height());
    let width = columns as f32 + 2.0 * margin;
    let height = rows as f32 + 2.0 * margin;
    raster::fits(width, height)?;
    let mut pixmap = tiny_skia::Pixmap::new(width as u32, height as u32).ok_or(PdfError::Raster)?;
    let area = Rect {
        x: image.area.x - margin / density,
        y: image.area.y - margin / density,
        width: width / density,
        height: height / density,
    };
    let to_box = projection::inverse(to_root).ok_or(PdfError::InvalidGeometry)?;
    let front = projection::front(to_root);
    let source = image.pixmap.pixels();
    let pixel = |value: f32, start: f32, count: u32| {
        (((value - start) * density).floor().max(0.0) as u32).min(count - 1)
    };
    let output = pixmap.pixels_mut();
    for (index, target) in output.iter_mut().enumerate() {
        let column = (index % width as usize) as f32;
        let row = (index / width as usize) as f32;
        let mut center = [
            area.x + (column + 0.5) / density,
            area.y + (row + 0.5) / density,
        ];
        let local = projection::apply(to_box, center);
        let ahead = projection::side(front, local) > 0.0;
        let inside = ahead
            && (border.x..=border.x + border.width).contains(&local[0])
            && (border.y..=border.y + border.height).contains(&local[1]);
        if !inside {
            if !mirrored || !ahead {
                continue;
            }
            center = projection::apply(
                to_root,
                [
                    mirror(local[0], border.x, border.width),
                    mirror(local[1], border.y, border.height),
                ],
            );
        }
        let x = pixel(center[0], image.area.x, columns);
        let y = pixel(center[1], image.area.y, rows);
        *target = source[(y * columns + x) as usize];
    }
    Ok(Raster {
        pixmap,
        area,
        density,
    })
}

/// `value` reflected into `start..start + length` across its ends, repeatedly.
pub(super) fn mirror(value: f32, start: f32, length: f32) -> f32 {
    let period = 2.0 * length;
    let offset = (value - start).rem_euclid(period);
    start
        + if offset > length {
            period - offset
        } else {
            offset
        }
}

/// Draws the prepared backdrop of `item`, clipped to its border box.
pub(super) fn draw(
    painter: &Painter<'_, '_, '_>,
    backdrop: &BackdropPaint,
    item: &DisplayItem,
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    // Only an element's Backdrop Root prepares its backdrops.
    let readback = painter
        .prepared
        .get(item)
        .ok_or(PdfError::InvalidGeometry)?;
    let Some(readback) = readback else {
        return Ok(());
    };
    let clips = painting::push_clip(&backdrop.border, space.viewport, surface)?;
    let drawn = embed(
        readback,
        backdrop.border.rect,
        painter.density(space, surface),
        surface,
    );
    painting::pop_clips(clips, surface);
    drawn
}

/// Draws `readback` in the item's drawing space over `area`, resampled at `density` pixels
/// per unit where the space is a perspective projection of the raster's.
fn embed(
    readback: &Readback,
    area: Rect,
    density: f32,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let to_item = projection::inverse(readback.to_raster).ok_or(PdfError::InvalidGeometry)?;
    let Some([a, b, c, d, e, f]) = projection::affine(to_item) else {
        return crate::writer::projection::embed(&readback.raster, to_item, area, density, surface);
    };
    surface.push_transform(&krilla::geom::Transform::from_row(a, b, c, d, e, f));
    let drawn = raster::embed(&readback.raster, surface);
    surface.pop();
    drawn
}

/// Draws the additive `operator` group whose push and pop are at `group`'s ends.
pub(super) fn additive(
    painter: &mut Painter<'_, '_, '_>,
    operator: BlendMode,
    items: &[DisplayItem],
    group: Range<usize>,
    scopes: &Scopes<'_>,
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let push = &items[group.start];
    if painter.prepared.get(push).is_none() {
        let composite = composite(painter, operator, items, group, scopes, space, surface)?;
        painter.prepared.insert(
            push,
            composite.map(|raster| Readback {
                raster,
                to_raster: projection::IDENTITY,
            }),
        );
    }
    match painter.prepared.get(push) {
        Some(Some(readback)) => {
            let area = readback.raster.area;
            embed(readback, area, readback.raster.density, surface)
        }
        _ => Ok(()),
    }
}

/// The group's content and, behind it, the content of its enclosing group so far, composited
/// by `operator` and expressed as the colour to draw over that backdrop.
fn composite(
    painter: &mut Painter<'_, '_, '_>,
    operator: BlendMode,
    items: &[DisplayItem],
    group: Range<usize>,
    scopes: &Scopes<'_>,
    space: Space,
    surface: &Surface<'_>,
) -> Result<Option<Raster>, PdfError> {
    let visible = painting::local_bounds(space.viewport, surface)?;
    let density = painter.density(space, surface);
    let content = &items[group.start + 1..group.end];
    let Some(source) = raster::rasterize(painter, content, visible, density)? else {
        return Ok(None);
    };
    let Some(mut source) = source.ink().and_then(|ink| source.cropped(ink)) else {
        return Ok(None);
    };
    let (origin, to_origin) = scopes.group();
    let from_origin = to_origin.invert().ok_or(PdfError::InvalidGeometry)?;
    let prefix = &items[origin.index..group.start];
    let backdrop = raster::render(painter, source.area, density, |painter, space, surface| {
        surface.push_transform(&krilla::geom::Transform::from_row(
            from_origin.sx,
            from_origin.ky,
            from_origin.kx,
            from_origin.sy,
            from_origin.tx,
            from_origin.ty,
        ));
        let drawn = painter.draw_prefix(prefix, space, surface);
        surface.pop();
        drawn
    })?
    .ok_or(PdfError::Raster)?;
    composite_onto(operator, &mut source.pixmap, &backdrop.pixmap)?;
    Ok(Some(source))
}

/// Replaces each pixel of `source` with the colour that source-over compositing onto the
/// pixel of `backdrop` at the same place turns into their additive `operator` composite;
/// `backdrop` must cover `source`.
pub(super) fn composite_onto(
    operator: BlendMode,
    source: &mut tiny_skia::Pixmap,
    backdrop: &tiny_skia::Pixmap,
) -> Result<(), PdfError> {
    let (columns, backdrop_columns) = (source.width() as usize, backdrop.width() as usize);
    if backdrop_columns < columns || backdrop.height() < source.height() {
        return Err(PdfError::Raster);
    }
    let behind = backdrop.pixels();
    for (index, pixel) in source.pixels_mut().iter_mut().enumerate() {
        let below = behind[index / columns * backdrop_columns + index % columns];
        *pixel = compositing::pixel(over_color(
            operator,
            compositing::sample(*pixel),
            compositing::sample(below),
        ));
    }
    Ok(())
}

/// The premultiplied colour that source-over compositing onto premultiplied `backdrop` turns
/// into the additive `operator`'s result.
pub(super) fn over_color(
    operator: BlendMode,
    source: compositing::Sample,
    backdrop: compositing::Sample,
) -> compositing::Sample {
    compositing::over_color(compositing::composite(operator, source, backdrop), backdrop)
}
