//! CSS Transforms 2 §4.2 layers under a perspective whose `w` varies across them, which no
//! PDF content matrix expresses. Text stays text: each cluster of glyphs draws through the
//! projection's tangent at its centre wherever that strays less than `TOLERANCE_PIXELS` from
//! the exact projection at the output raster density. Every other paint renders to pixels in
//! the layer's own plane, in bands of `w` each resolved at the density its most magnified
//! point needs, and is resampled through the projection onto output pixels, averaging enough
//! samples where the projection shrinks the plane. Groups that composite with what lies behind
//! them, and groups holding vector text, open in place around their projected content; the
//! rest render to pixels with their content, in the layer's plane, as filters and masks apply.
use crate::graphics::{BlendMode, Clip, DisplayItem, Projection, Rect};
use crate::layout::TextLine;
use crate::paint::projection::{self, Line};
use crate::writer::{
    PdfError, bounds,
    display::{Painter, Space, group_length},
    painting,
    raster::{self, Raster},
};
use krilla::{geom::Transform, num::NormalizedF32, paint::FillRule, surface::Surface};

/// Largest distance, in output raster pixels, between a text cluster drawn through the
/// projection's tangent and its exact projection: the half pixel by which the resampled
/// raster of the layer's other paints may place an edge.
const TOLERANCE_PIXELS: f64 = 0.5;
/// Samples per axis averaged into one output pixel where the projection shrinks the plane,
/// at most.
const MAX_SUPERSAMPLES: f64 = 8.0;
/// Ratio of the largest to the smallest `w` within one band of the plane rendered at one
/// density; the magnification varies by about this factor across a band.
const BAND_RATIO: f64 = std::f64::consts::SQRT_2;
/// Local raster pixels rendered beyond a band's edges, which samples near them read.
const BAND_MARGIN_PIXELS: f32 = 2.0;
/// Translucent samples average as premultiplied channels.
const CHANNEL_MAX: f32 = 255.0;

/// Draws `items` through `projection`, which maps their plane into the current drawing space.
pub(super) fn draw(
    painter: &mut Painter<'_, '_, '_>,
    projection: Projection,
    items: &[DisplayItem],
    space: Space,
    surface: &mut Surface<'_>,
) -> Result<(), PdfError> {
    let visible = painting::local_bounds(space.viewport, surface)?;
    let bounds = [
        [f64::from(visible.x), f64::from(visible.x + visible.width)],
        [f64::from(visible.y), f64::from(visible.y + visible.height)],
    ];
    Layer {
        projection,
        lines: projection::within(projection, bounds).to_vec(),
        density: painter.density(space, surface),
    }
    .draw(painter, items, surface)
}

/// A plane under a projection into the drawing space.
struct Layer {
    projection: Projection,
    /// The half-planes of the plane that project into the visible area in front of the viewer.
    lines: Vec<Line>,
    /// Output raster pixels per drawing-space unit.
    density: f32,
}

impl Layer {
    /// The plane of content that `m` maps into this one.
    fn mapped(&self, m: Projection) -> Self {
        let mut lines: Vec<Line> = self
            .lines
            .iter()
            .map(|&line| projection::pulled(line, m))
            .collect();
        lines.push(projection::front(m));
        Self {
            projection: projection::compose(self.projection, m),
            lines,
            density: self.density,
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
                    painter
                        .text
                        .draw_projected(line, &self.projection, surface)?;
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
                        self.open(painter, item, content, surface)?;
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

    /// Opens the group `push` around its projected `content`.
    fn open(
        &self,
        painter: &mut Painter<'_, '_, '_>,
        push: &DisplayItem,
        content: &[DisplayItem],
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
                let drawn = self.draw(painter, content, surface);
                surface.pop();
                drawn
            }
            DisplayItem::PushBlend(BlendMode::PlusDarker | BlendMode::PlusLighter) => {
                Err(PdfError::ProjectedAdditive)
            }
            DisplayItem::PushBlend(mode) => {
                let pops = painting::push_blend(*mode, surface)?;
                let drawn = self.draw(painter, content, surface);
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
                self.open(painter, &DisplayItem::PushBlend(mode), &root, surface)
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

    /// Whether every cluster of `line` lies in front of the viewer and draws through the
    /// projection's tangent at its centre within the tolerance.
    fn vector(&self, painter: &Painter<'_, '_, '_>, line: &TextLine) -> Result<bool, PdfError> {
        let tolerance = TOLERANCE_PIXELS / f64::from(self.density);
        let front = projection::front(self.projection);
        let origin = [line.x, line.baseline];
        for run in line.runs.iter().flat_map(crate::text::GlyphRun::with_marks) {
            if !run.mark_shapes.is_empty() {
                return Ok(false);
            }
            for pen in run.copies(origin) {
                let Some(clusters) = painter.text.clusters(run, origin, pen)? else {
                    return Ok(false);
                };
                for cluster in clusters {
                    let corners = projection::corners(cluster.ink);
                    let centre = [
                        cluster.ink.x + cluster.ink.width / 2.0,
                        cluster.ink.y + cluster.ink.height / 2.0,
                    ];
                    if !corners
                        .iter()
                        .all(|&corner| projection::side(front, corner) > 0.0)
                        || projection::tangent_error(self.projection, centre, &corners) > tolerance
                    {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// Clips the drawing space to the projection of `clip`'s outline within the layer.
    fn push_clip(&self, clip: &Clip, surface: &mut Surface<'_>) -> Result<(), PdfError> {
        let mut lines = self.lines.clone();
        lines.extend(clip.bounds().edges().map(|(axis, bound, lower)| {
            let sign = if lower { 1.0 } else { -1.0 };
            let mut line = [0.0; 3];
            line[axis] = sign;
            line[2] = -sign * f64::from(bound);
            line
        }));
        let polygons: Vec<Vec<[f32; 2]>> = if clip.axes == [true; 2] {
            let tolerance =
                TOLERANCE_PIXELS / f64::from(self.density) / self.largest_stretch(&lines);
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
                    .map(|point| projection::apply(self.projection, point))
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

    /// The largest stretch of the projection over the part of the plane within `lines`.
    fn largest_stretch(&self, lines: &[Line]) -> f64 {
        let region = projection::clip_all(horizon_square(), lines);
        sample_points(&region)
            .map(|point| projection::stretch(self.projection, point))
            .fold(f64::MIN_POSITIVE, f64::max)
    }

    /// Renders `items` in the plane band by band and draws them resampled through the
    /// projection.
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
        let Some(ink) = bounds::ink(items, painter.lines, definitions, self.density)? else {
            return Ok(());
        };
        let region = projection::clip_all(projection::corners(ink), &self.lines);
        if region.len() < 3 {
            return Ok(());
        }
        let front = projection::front(self.projection);
        let weights = region.iter().map(|&point| projection::side(front, point));
        let lowest = weights.clone().fold(f64::INFINITY, f64::min);
        let highest = weights.fold(0.0, f64::max);
        let mut bands = Vec::new();
        // A vertex exactly at the horizon would need endless bands.
        let mut low = lowest.max(highest * f64::EPSILON);
        while low < highest {
            let high = (low * BAND_RATIO).min(highest);
            let lines = vec![
                [front[0], front[1], front[2] - low],
                [-front[0], -front[1], high - front[2]],
            ];
            bands.extend(self.band(&region, lines));
            low = high;
        }
        let projected: Vec<_> = region
            .iter()
            .map(|&point| projection::apply(self.projection, point))
            .collect();
        let Some(target) = projection::bounds(&projected) else {
            return Ok(());
        };
        let scale = 1.0 / painter.raster.pixels_per_point;
        let raster = resample(
            self.projection,
            self.density,
            &bands,
            target,
            &self.lines,
            |band| band.render(painter, items, scale),
        )?;
        match raster {
            Some(raster) => raster::embed(&raster, surface),
            None => Ok(()),
        }
    }

    /// The band of `region` within `lines`, with pixel axes along and across the gradient of
    /// `w`, each as dense as the band's most magnified point needs along it.
    fn band(&self, region: &[[f32; 2]], lines: Vec<Line>) -> Option<Band> {
        let part = projection::clip_all(region.to_vec(), &lines);
        if part.len() < 3 {
            return None;
        }
        let [g, h, _] = projection::front(self.projection);
        let length = g.hypot(h);
        let along = if length > 0.0 {
            [g / length, h / length]
        } else {
            [1.0, 0.0]
        };
        let axes = [along, [-along[1], along[0]]];
        let mut to_pixels = [0.0_f32; 6];
        let mut size = [0_u32; 2];
        for (index, axis) in axes.into_iter().enumerate() {
            let density = f64::from(self.density)
                * sample_points(&part)
                    .map(|point| projection::stretch_along(self.projection, point, axis))
                    .fold(f64::MIN_POSITIVE, f64::max);
            let margin = f64::from(BAND_MARGIN_PIXELS) / density;
            let offsets = part
                .iter()
                .map(|point| axis[0] * f64::from(point[0]) + axis[1] * f64::from(point[1]));
            let start = offsets.clone().fold(f64::INFINITY, f64::min) - margin;
            let end = offsets.fold(f64::NEG_INFINITY, f64::max) + margin;
            size[index] = ((end - start) * density).ceil() as u32;
            to_pixels[index] = (density * axis[0]) as f32;
            to_pixels[index + 2] = (density * axis[1]) as f32;
            to_pixels[index + 4] = (-density * start) as f32;
        }
        let projected: Vec<_> = part
            .iter()
            .map(|&point| projection::apply(self.projection, point))
            .collect();
        Some(Band {
            lines,
            to_pixels,
            size,
            screen: projection::bounds(&projected)?,
        })
    }
}

/// A band of a plane whose pixels map its points within `lines`.
struct Band {
    lines: Vec<Line>,
    /// The affine map from the plane onto the band's pixel grid.
    to_pixels: [f32; 6],
    size: [u32; 2],
    /// The drawing-space bounds of the band's projection.
    screen: Rect,
}

impl Band {
    /// The pixels of `items` over the band, drawn in a space whose units span `scale` points.
    fn render(
        &self,
        painter: &mut Painter<'_, '_, '_>,
        items: &[DisplayItem],
        scale: f32,
    ) -> Result<Option<tiny_skia::Pixmap>, PdfError> {
        let [a, b, c, d, e, f] = self.to_pixels;
        let area = Rect {
            x: 0.0,
            y: 0.0,
            width: self.size[0] as f32,
            height: self.size[1] as f32,
        };
        let raster = raster::render_surface(area, 1.0, |viewport, surface| {
            surface.push_transform(&Transform::from_row(a, b, c, d, e, f));
            let drawn = painter.draw(items, Space { viewport, scale }, surface);
            surface.pop();
            drawn
        })?;
        Ok(raster.map(|raster| raster.pixmap))
    }
    fn holds(&self, point: [f32; 2]) -> bool {
        self.lines
            .iter()
            .all(|&line| projection::side(line, point) >= 0.0)
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
    let band = Band {
        lines: Vec::new(),
        to_pixels: [
            raster.density,
            0.0,
            0.0,
            raster.density,
            -raster.area.x * raster.density,
            -raster.area.y * raster.density,
        ],
        size: [raster.pixmap.width(), raster.pixmap.height()],
        screen: target,
    };
    match resample(to_target, density, &[band], target, &[], |_| {
        Ok(Some(raster.pixmap.clone()))
    })? {
        Some(raster) => raster::embed(&raster, surface),
        None => Ok(()),
    }
}

/// The output pixels over `target` at `density` pixels per unit. Each averages samples at
/// the plane points `projection` maps its samples from, where they lie within `lines` in
/// front of the viewer, read from the pixels `render` gives the band holding each.
fn resample(
    projection: Projection,
    density: f32,
    bands: &[Band],
    target: Rect,
    lines: &[Line],
    mut render: impl FnMut(&Band) -> Result<Option<tiny_skia::Pixmap>, PdfError>,
) -> Result<Option<Raster>, PdfError> {
    let columns = (target.width * density).ceil();
    let rows = (target.height * density).ceil();
    if !(columns >= 1.0 && rows >= 1.0) || bands.is_empty() {
        return Ok(None);
    }
    let limit = f32::from(u16::MAX);
    if columns > limit || rows > limit {
        return Err(PdfError::RasterSize {
            width: columns,
            height: rows,
        });
    }
    let inverse = projection::inverse(projection).ok_or(PdfError::InvalidGeometry)?;
    let front = projection::front(projection);
    let inside = |point: [f32; 2]| {
        projection::side(front, point) > 0.0
            && lines
                .iter()
                .all(|&line| projection::side(line, point) >= 0.0)
    };
    let width = columns as usize;
    let pixel = 1.0 / density;
    let corner = |index: usize| {
        [
            target.x + (index % width) as f32 * pixel,
            target.y + (index / width) as f32 * pixel,
        ]
    };
    // Enough samples per axis that each spans at most one pixel of the band it reads.
    let samples: Vec<u16> = (0..width * rows as usize)
        .map(|index| {
            let centre = projection::apply(inverse, corner(index).map(|value| value + pixel / 2.0));
            if !inside(centre) {
                return 0;
            }
            bands
                .iter()
                .find(|band| band.holds(centre))
                .map_or(0, |band| {
                    let [a, b, c, d, _, _] = band.to_pixels.map(f64::from);
                    let reach = projection::reach(projection, centre, [a, b, c, d]);
                    (reach / f64::from(density))
                        .ceil()
                        .clamp(1.0, MAX_SUPERSAMPLES) as u16
                })
        })
        .collect();
    let mut sums = vec![[0.0_f32; 4]; samples.len()];
    for band in bands {
        let Some(pixels) = render(band)? else {
            continue;
        };
        let [a, b, c, d, e, f] = band.to_pixels;
        let span = |start: f32, extent: f32, origin: f32, count: f32| {
            let first = ((start - origin) * density).floor().clamp(0.0, count) as usize;
            let last = ((start + extent - origin) * density)
                .ceil()
                .clamp(0.0, count) as usize;
            first..last
        };
        let screen = band.screen;
        for row in span(screen.y, screen.height, target.y, rows) {
            for column in span(screen.x, screen.width, target.x, columns) {
                let index = row * width + column;
                let count = samples[index];
                let step = pixel / f32::from(count.max(1));
                let origin = corner(index);
                for sample in 0..count * count {
                    let point = projection::apply(
                        inverse,
                        [
                            origin[0] + (f32::from(sample % count) + 0.5) * step,
                            origin[1] + (f32::from(sample / count) + 0.5) * step,
                        ],
                    );
                    if inside(point) && band.holds(point) {
                        let value = bilinear(
                            &pixels,
                            [
                                a * point[0] + c * point[1] + e,
                                b * point[0] + d * point[1] + f,
                            ],
                        );
                        for (total, value) in sums[index].iter_mut().zip(value) {
                            *total += value;
                        }
                    }
                }
            }
        }
    }
    let mut pixmap = tiny_skia::Pixmap::new(columns as u32, rows as u32).ok_or(PdfError::Raster)?;
    for ((output, sum), count) in pixmap.pixels_mut().iter_mut().zip(&sums).zip(&samples) {
        if *count == 0 {
            continue;
        }
        let count = f32::from(*count) * f32::from(*count);
        let alpha = (sum[3] / count).round().min(CHANNEL_MAX);
        let channel = |value: f32| (value / count).round().min(alpha) as u8;
        if let Some(color) = tiny_skia::PremultipliedColorU8::from_rgba(
            channel(sum[0]),
            channel(sum[1]),
            channel(sum[2]),
            alpha as u8,
        ) {
            *output = color;
        }
    }
    Ok(Some(Raster {
        pixmap,
        area: Rect {
            x: target.x,
            y: target.y,
            width: columns * pixel,
            height: rows * pixel,
        },
        density,
    }))
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

/// The premultiplied channels of `pixels` interpolated at pixel-grid point `point`; outside
/// the grid is transparent.
fn bilinear(pixels: &tiny_skia::Pixmap, point: [f32; 2]) -> [f32; 4] {
    let x = point[0] - 0.5;
    let y = point[1] - 0.5;
    let (left, top) = (x.floor(), y.floor());
    let (dx, dy) = (x - left, y - top);
    let (width, height) = (pixels.width() as f32, pixels.height() as f32);
    let data = pixels.pixels();
    let mut value = [0.0; 4];
    for (column, row, weight) in [
        (left, top, (1.0 - dx) * (1.0 - dy)),
        (left + 1.0, top, dx * (1.0 - dy)),
        (left, top + 1.0, (1.0 - dx) * dy),
        (left + 1.0, top + 1.0, dx * dy),
    ] {
        if column < 0.0 || row < 0.0 || column >= width || row >= height {
            continue;
        }
        let pixel = data[row as usize * width as usize + column as usize];
        for (channel, sample) in
            value
                .iter_mut()
                .zip([pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()])
        {
            *channel += weight * f32::from(sample);
        }
    }
    value
}

/// A square of the plane reaching the horizon, which bounds every region of it worth drawing.
fn horizon_square() -> Vec<[f32; 2]> {
    let reach = projection::HORIZON_PT as f32;
    projection::corners(Rect {
        x: -reach,
        y: -reach,
        width: 2.0 * reach,
        height: 2.0 * reach,
    })
}

/// The vertices of `polygon` and the midpoints of its edges, where the stretch of a
/// projection over it is sampled.
fn sample_points(polygon: &[[f32; 2]]) -> impl Iterator<Item = [f32; 2]> + '_ {
    polygon.iter().enumerate().flat_map(|(index, &point)| {
        let next = polygon[(index + 1) % polygon.len()];
        [
            point,
            [
                f32::midpoint(point[0], next[0]),
                f32::midpoint(point[1], next[1]),
            ],
        ]
    })
}
