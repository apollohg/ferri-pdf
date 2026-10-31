//! CSS Transforms 2 §4.2 layers under a perspective whose `w` varies across them, which no
//! PDF content matrix expresses. Text stays text: each cluster of glyphs draws through the
//! projection's tangent at its centre wherever that strays less than the resampling tolerance
//! from the exact projection at the output raster density. Every other paint renders to
//! pixels in the layer's own plane and is resampled through the projection onto output
//! pixels. Groups that composite with what lies behind them, and groups holding vector text,
//! open in place around their projected content; the rest render to pixels with their content,
//! in the layer's plane, as filters and masks apply. Additive groups (Compositing 2 §9.1.14,
//! §9.1.15) composite in the plane of their enclosing group, whose content before them is
//! their backdrop.
use crate::graphics::{BlendMode, Clip, DisplayItem, Projection, Rect};
use crate::layout::TextLine;
use crate::paint::{
    projection,
    resampling::{Band, Plane, PlaneError, Resampled, horizon_square},
};
use crate::writer::{
    PdfError, bounds,
    display::{Painter, Space, group_length},
    painting,
    raster::{self, Raster},
};
use krilla::{geom::Transform, num::NormalizedF32, paint::FillRule, surface::Surface};

impl From<PlaneError> for PdfError {
    fn from(error: PlaneError) -> Self {
        match error {
            PlaneError::Size { width, height } => Self::RasterSize { width, height },
            PlaneError::Degenerate => Self::InvalidGeometry,
            PlaneError::Pixmap => Self::Raster,
        }
    }
}

/// Draws `items` through `projection`, which maps their plane into the current drawing space.
pub(super) fn draw(
    painter: &mut Painter<'_, '_, '_>,
    projection: Projection,
    items: &[DisplayItem],
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let visible = painting::local_bounds(space.viewport, surface)?;
    let plane = Plane::new(projection, visible, painter.density(space, surface));
    Layer {
        plane: plane.clone(),
        group: Group { items, plane },
        to_group: projection::IDENTITY,
    }
    .draw(painter, items, surface)
}

/// The innermost transparency group open in a layer: its content, whose items before an
/// additive group are that group's backdrop, and its plane.
#[derive(Clone)]
struct Group<'i> {
    items: &'i [DisplayItem],
    plane: Plane,
}

/// A plane under a projection into the drawing space.
struct Layer<'i> {
    plane: Plane,
    group: Group<'i>,
    /// The map from this plane to its group's.
    to_group: Projection,
}

impl Layer<'_> {
    /// The plane of content that `m` maps into this one.
    fn mapped(&self, m: Projection) -> Self {
        Self {
            plane: self.plane.mapped(m),
            group: self.group.clone(),
            to_group: projection::compose(self.to_group, m),
        }
    }

    /// This plane holding the transparency group whose content is `items`.
    fn grouped<'g>(&self, items: &'g [DisplayItem]) -> Layer<'g> {
        Layer {
            plane: self.plane.clone(),
            group: Group {
                items,
                plane: self.plane.clone(),
            },
            to_group: projection::IDENTITY,
        }
    }

    fn draw(
        &self,
        painter: &mut Painter<'_, '_, '_>,
        items: &[DisplayItem],
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let mut run = 0;
        let mut index = 0;
        while let Some(item) = items.get(index) {
            match item {
                DisplayItem::Text { line }
                    if self.vector(painter, line_at(painter.lines, *line)?)? =>
                {
                    self.rasterize(painter, &items[run..index], surface)?;
                    let line = line_at(painter.lines, *line)?;
                    let drawn = painter
                        .text
                        .draw_projected(line, &self.plane.projection, surface);
                    painter.record_unwritable_runs();
                    drawn?;
                    index += 1;
                    run = index;
                }
                DisplayItem::Pop => return Err(PdfError::InvalidGeometry),
                DisplayItem::PushClip(_)
                | DisplayItem::PushTransform(_)
                | DisplayItem::PushProjection(_)
                | DisplayItem::PushOpacity(_)
                | DisplayItem::PushBlend(_)
                | DisplayItem::PushFilter(_)
                | DisplayItem::PushMask(_)
                | DisplayItem::PushBackdropRoot => {
                    let end = index + 1 + group_length(&items[index + 1..])?;
                    let content = &items[index + 1..end];
                    if self.opens(painter, item, content)? {
                        self.rasterize(painter, &items[run..index], surface)?;
                        self.open(painter, item, content, item, surface)?;
                        run = end + 1;
                    }
                    index = end + 1;
                }
                DisplayItem::Text { .. }
                | DisplayItem::Image(_)
                | DisplayItem::Frame(_)
                | DisplayItem::Scene(_)
                | DisplayItem::Box(_)
                | DisplayItem::Decoration(_)
                | DisplayItem::TextShadow(_)
                | DisplayItem::Backdrop(_) => index += 1,
            }
        }
        self.rasterize(painter, &items[run..], surface)
    }

    /// Whether the group `push` opens around `content` must open in place: it composites with
    /// what lies behind it, or holds vector text or such a group outside an isolated one.
    fn opens(
        &self,
        painter: &Painter<'_, '_, '_>,
        push: &DisplayItem,
        content: &[DisplayItem],
    ) -> Result<bool, PdfError> {
        Ok(match push {
            DisplayItem::PushBlend(mode) if *mode != BlendMode::Normal => true,
            DisplayItem::PushBackdropRoot => hoisted_blend(content)?.is_some(),
            DisplayItem::PushFilter(_) | DisplayItem::PushMask(_) => false,
            DisplayItem::PushTransform(matrix) => self
                .mapped(projection::from_affine(*matrix))
                .holds_open(painter, content)?,
            DisplayItem::PushProjection(matrix) => {
                self.mapped(*matrix).holds_open(painter, content)?
            }
            _ => self.holds_open(painter, content)?,
        })
    }

    /// Whether `items` hold vector text or a group that must open in place.
    fn holds_open(
        &self,
        painter: &Painter<'_, '_, '_>,
        items: &[DisplayItem],
    ) -> Result<bool, PdfError> {
        let mut index = 0;
        while let Some(item) = items.get(index) {
            index += 1;
            match item {
                DisplayItem::Text { line } => {
                    if self.vector(painter, line_at(painter.lines, *line)?)? {
                        return Ok(true);
                    }
                }
                DisplayItem::PushClip(_)
                | DisplayItem::PushTransform(_)
                | DisplayItem::PushProjection(_)
                | DisplayItem::PushOpacity(_)
                | DisplayItem::PushBlend(_)
                | DisplayItem::PushFilter(_)
                | DisplayItem::PushMask(_)
                | DisplayItem::PushBackdropRoot => {
                    let end = index + group_length(&items[index..])?;
                    if self.opens(painter, item, &items[index..end])? {
                        return Ok(true);
                    }
                    index = end + 1;
                }
                _ => {}
            }
        }
        Ok(false)
    }

    /// Opens the group `push` around its projected `content`, which item `at` of the walked
    /// display list opens.
    fn open(
        &self,
        painter: &mut Painter<'_, '_, '_>,
        push: &DisplayItem,
        content: &[DisplayItem],
        at: &DisplayItem,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        match push {
            DisplayItem::PushTransform(matrix) => self
                .mapped(projection::from_affine(*matrix))
                .draw(painter, content, surface),
            DisplayItem::PushProjection(matrix) => {
                self.mapped(*matrix).draw(painter, content, surface)
            }
            DisplayItem::PushOpacity(alpha) => {
                let opacity = NormalizedF32::new(*alpha).ok_or(PdfError::InvalidAlpha(*alpha))?;
                surface.push_opacity(opacity);
                let drawn = self.grouped(content).draw(painter, content, surface);
                surface.pop();
                drawn
            }
            DisplayItem::PushBlend(mode @ (BlendMode::PlusDarker | BlendMode::PlusLighter)) => {
                self.additive(painter, *mode, content, at, surface)
            }
            DisplayItem::PushBlend(mode) => {
                let pops = painting::push_blend(*mode, surface)?;
                let drawn = self.grouped(content).draw(painter, content, surface);
                painting::pop_clips(pops, surface);
                drawn
            }
            DisplayItem::PushClip(clip) => {
                self.push_clip(clip, surface)?;
                let drawn = self.draw(painter, content, surface);
                surface.pop();
                drawn
            }
            DisplayItem::PushBackdropRoot => {
                let (mode, inner) = hoisted_blend(content)?.ok_or(PdfError::InvalidGeometry)?;
                // The root only marks where backdrops start, so it opens inside the blend.
                let mut root = Vec::with_capacity(inner.len() + 2);
                root.push(DisplayItem::PushBackdropRoot);
                root.extend_from_slice(inner);
                root.push(DisplayItem::Pop);
                self.open(painter, &DisplayItem::PushBlend(mode), &root, at, surface)
            }
            DisplayItem::PushFilter(_) | DisplayItem::PushMask(_) => Err(PdfError::InvalidGeometry),
            DisplayItem::Text { .. }
            | DisplayItem::Image(_)
            | DisplayItem::Frame(_)
            | DisplayItem::Scene(_)
            | DisplayItem::Box(_)
            | DisplayItem::Decoration(_)
            | DisplayItem::TextShadow(_)
            | DisplayItem::Backdrop(_)
            | DisplayItem::Pop => Err(PdfError::InvalidGeometry),
        }
    }

    /// Draws the additive `operator` group of `content`, which item `at` opens: in the plane of
    /// the enclosing group it composites with that group's content before `at`, and draws as
    /// the colour that source-over compositing onto that content turns into the result.
    fn additive(
        &self,
        painter: &mut Painter<'_, '_, '_>,
        operator: BlendMode,
        content: &[DisplayItem],
        at: &DisplayItem,
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        let prefix = &self.group.items[..self.group.offset(at)?];
        let placed: Vec<DisplayItem> = if self.to_group == projection::IDENTITY {
            content.to_vec()
        } else {
            let push = projection::affine(self.to_group).map_or(
                DisplayItem::PushProjection(self.to_group),
                DisplayItem::PushTransform,
            );
            std::iter::once(push)
                .chain(content.iter().cloned())
                .chain(std::iter::once(DisplayItem::Pop))
                .collect()
        };
        let definitions = painter.raster.definitions;
        let plane = &self.group.plane;
        let Some(ink) = bounds::ink(&placed, painter.lines, definitions, plane.density)? else {
            return Ok(());
        };
        let scale = 1.0 / painter.raster.pixels_per_point;
        let resampled = plane.render(ink, |band| -> Result<_, PdfError> {
            let Some(mut source) = band_pixels(painter, band, scale, |painter, space, surface| {
                painter.draw(&placed, space, surface)
            })?
            else {
                return Ok(None);
            };
            let backdrop = band_pixels(painter, band, scale, |painter, space, surface| {
                painter.draw_prefix(prefix, space, surface)
            })?
            .ok_or(PdfError::Raster)?;
            crate::writer::backdrops::composite_onto(operator, &mut source, &backdrop)?;
            Ok(Some(source))
        })?;
        embed_resampled(resampled, plane.density, surface)
    }

    /// Whether every cluster of `line` lies in front of the viewer and draws through the
    /// projection's tangent at its centre within the tolerance.
    fn vector(&self, painter: &Painter<'_, '_, '_>, line: &TextLine) -> Result<bool, PdfError> {
        let origin = [line.x, line.baseline];
        for run in line.runs.iter().flat_map(crate::text::GlyphRun::with_marks) {
            if !run.mark_shapes.is_empty() {
                return Ok(false);
            }
            for pen in run.copies(origin) {
                let Some(clusters) = painter.text.clusters(run, origin, pen)? else {
                    return Ok(false);
                };
                if !clusters
                    .iter()
                    .all(|cluster| self.plane.tangent_fits(cluster.ink))
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Clips the drawing space to the projection of `clip`'s outline within the layer.
    fn push_clip(&self, clip: &Clip, surface: &mut Surface<'_>) -> Result<(), PdfError> {
        let mut lines = self.plane.lines.clone();
        lines.extend(clip.bounds().edges().map(|(axis, bound, lower)| {
            let sign = if lower { 1.0 } else { -1.0 };
            let mut line = [0.0; 3];
            line[axis] = sign;
            line[2] = -sign * f64::from(bound);
            line
        }));
        let polygons: Vec<Vec<[f32; 2]>> = if clip.axes == [true; 2] {
            let tolerance = self.plane.tolerance() / self.plane.largest_stretch(&lines);
            clip.contours()
                .iter()
                .map(|contour| {
                    contour
                        .polygon_points(tolerance)
                        .into_iter()
                        .map(|point| point.map(|value| value as f32))
                        .collect()
                })
                .collect()
        } else {
            vec![horizon_square()]
        };
        let contours: Vec<crate::graphics::Contour> = polygons
            .into_iter()
            .map(|polygon| projection::clip_all(polygon, &lines))
            .filter(|polygon| polygon.len() >= 3)
            .map(|polygon| {
                let projected: Vec<_> = polygon
                    .into_iter()
                    .map(|point| projection::apply(self.plane.projection, point))
                    .collect();
                crate::graphics::Contour::polygon(&projected)
            })
            .collect();
        let path = if contours.is_empty() {
            painting::rect_path(Rect {
                width: 0.0,
                height: 0.0,
                ..clip.rect
            })?
        } else {
            painting::contour_path(&contours)?
        };
        surface.push_clip_path(&path, &FillRule::NonZero);
        Ok(())
    }

    /// Renders `items` in the plane and draws them resampled through the projection.
    fn rasterize(
        &self,
        painter: &mut Painter<'_, '_, '_>,
        items: &[DisplayItem],
        surface: &mut Surface<'_>,
    ) -> Result<(), PdfError> {
        if items.is_empty() {
            return Ok(());
        }
        let definitions = painter.raster.definitions;
        let density = self.plane.density;
        let Some(ink) = bounds::ink(items, painter.lines, definitions, density)? else {
            return Ok(());
        };
        let scale = 1.0 / painter.raster.pixels_per_point;
        let resampled = self.plane.render(ink, |band| {
            band_pixels(painter, band, scale, |painter, space, surface| {
                painter.draw(items, space, surface)
            })
        })?;
        embed_resampled(resampled, density, surface)
    }
}

impl Group<'_> {
    /// The index of `item`, which must be one of the group's items.
    fn offset(&self, item: &DisplayItem) -> Result<usize, PdfError> {
        let address = std::ptr::from_ref(item) as usize;
        let first = self.items.as_ptr() as usize;
        address
            .checked_sub(first)
            .map(|bytes| bytes / size_of::<DisplayItem>())
            .filter(|&offset| offset < self.items.len())
            .ok_or(PdfError::InvalidGeometry)
    }
}

/// The pixels of what `draw` paints over `band`, in a space whose units span `scale` points.
fn band_pixels(
    painter: &mut Painter<'_, '_, '_>,
    band: &Band,
    scale: f32,
    draw: impl FnOnce(&mut Painter<'_, '_, '_>, Space, &mut Surface<'_>) -> Result<(), PdfError>,
) -> Result<Option<tiny_skia::Pixmap>, PdfError> {
    let [a, b, c, d, e, f] = band.to_pixels;
    let raster = raster::render_surface(band.area(), 1.0, |viewport, surface| {
        surface.push_transform(&Transform::from_row(a, b, c, d, e, f));
        let drawn = draw(painter, Space { viewport, scale }, surface);
        surface.pop();
        drawn
    })?;
    Ok(raster.map(|raster| raster.pixmap))
}

/// Draws `resampled` output pixels, at `density` pixels per unit, in place.
fn embed_resampled(
    resampled: Option<Resampled>,
    density: f32,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    match resampled {
        Some(Resampled { pixmap, area }) => raster::embed(
            &Raster {
                pixmap,
                area,
                density,
            },
            surface,
        ),
        None => Ok(()),
    }
}

/// Draws `raster`, whose drawing space `to_target` projects into the current one, over
/// `target` at `density` pixels per unit.
pub(super) fn embed(
    raster: &Raster,
    to_target: Projection,
    target: Rect,
    density: f32,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let to_pixels = [
        raster.density,
        0.0,
        0.0,
        raster.density,
        -raster.area.x * raster.density,
        -raster.area.y * raster.density,
    ];
    let resampled = crate::paint::resampling::resample_pixmap(
        &raster.pixmap,
        to_pixels,
        to_target,
        target,
        density,
    )?;
    embed_resampled(resampled, density, surface)
}

/// The text line a display item draws.
fn line_at(lines: &[TextLine], line: usize) -> Result<&TextLine, PdfError> {
    lines.get(line).ok_or(PdfError::InvalidGeometry)
}

/// A Backdrop Root group's content that is wholly one group blending with what lies behind
/// it, as an element with both opens them: that blend and its content.
fn hoisted_blend(content: &[DisplayItem]) -> Result<Option<(BlendMode, &[DisplayItem])>, PdfError> {
    let Some(DisplayItem::PushBlend(mode)) = content.first() else {
        return Ok(None);
    };
    if *mode == BlendMode::Normal {
        return Ok(None);
    }
    let end = 1 + group_length(&content[1..])?;
    Ok((end + 1 == content.len()).then(|| (*mode, &content[1..end])))
}
