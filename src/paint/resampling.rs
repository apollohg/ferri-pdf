//! Rasters of a plane under a projection whose `w` varies across it (CSS Transforms 2 §4.2),
//! which no PDF content matrix or affine raster transform expresses. The plane renders to
//! pixels in bands of `w`, each resolved at the density its most magnified point needs and
//! reduced by halves along the gradient of `w`, where the magnification varies most; output
//! pixels average samples read from the reduction each one's footprint needs.
use crate::paint::{
    Rect,
    projection::{self, Line, Projection},
};

/// Largest distance, in output raster pixels, between paint drawn through the projection's
/// tangent and its exact projection: the half pixel by which resampled pixels may place an
/// edge.
pub(crate) const TOLERANCE_PIXELS: f64 = 0.5;
/// Samples per output pixel axis averaged where the projection shrinks the plane, at most.
const MAX_SUPERSAMPLES: f64 = 8.0;
/// Pixels of a band's reduction one output pixel's footprint spans along the gradient of `w`
/// before the next reduction is read instead.
const REDUCTION_SPAN: f64 = 2.0;
/// Ratio of the largest to the smallest `w` within one band of the plane rendered at one
/// density; the magnification varies by about this factor across a band.
const BAND_RATIO: f64 = std::f64::consts::SQRT_2;
/// Band pixels rendered beyond a band's edges, which samples near them read.
const BAND_MARGIN_PIXELS: f64 = 2.0;
/// Translucent samples average as premultiplied channels.
const CHANNEL_MAX: f32 = 255.0;
/// Largest raster side, in pixels, that rendering and embedding accept.
const MAX_RASTER_SIDE: f32 = u16::MAX as f32;

/// Why a plane cannot be resampled.
#[derive(Debug)]
pub(crate) enum PlaneError {
    /// The output raster would exceed `MAX_RASTER_SIDE` pixels on a side.
    Size { width: f32, height: f32 },
    /// The projection collapses the plane.
    Degenerate,
    /// A pixmap could not be allocated.
    Pixmap,
}

/// A plane that `projection` maps into a drawing space, rendered at `density` output raster
/// pixels per drawing-space unit.
#[derive(Clone)]
pub(crate) struct Plane {
    pub(crate) projection: Projection,
    /// The half-planes of the plane that project into the visible area in front of the viewer.
    pub(crate) lines: Vec<Line>,
    pub(crate) density: f32,
}

/// A band of a plane whose pixels map its points with `w` within `range`.
pub(crate) struct Band {
    range: [f64; 2],
    /// The affine map from the plane onto the band's pixel grid, whose first axis runs along
    /// the gradient of `w`.
    pub(crate) to_pixels: [f32; 6],
    pub(crate) size: [u32; 2],
    /// The drawing-space bounds of the band's projection.
    screen: Rect,
}

/// Output raster pixels over `area` of the drawing space.
pub(crate) struct Resampled {
    pub(crate) pixmap: tiny_skia::Pixmap,
    pub(crate) area: Rect,
}

impl Plane {
    /// The plane `projection` maps into a drawing space whose visible area is `visible`.
    pub(crate) fn new(projection: Projection, visible: Rect, density: f32) -> Self {
        let bounds = [
            [f64::from(visible.x), f64::from(visible.x + visible.width)],
            [f64::from(visible.y), f64::from(visible.y + visible.height)],
        ];
        Self {
            projection,
            lines: projection::within(projection, bounds).to_vec(),
            density,
        }
    }

    /// The plane of content that `m` maps into this one.
    pub(crate) fn mapped(&self, m: Projection) -> Self {
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

    /// `TOLERANCE_PIXELS` in drawing-space units.
    pub(crate) fn tolerance(&self) -> f64 {
        TOLERANCE_PIXELS / f64::from(self.density)
    }

    /// Whether every point of `ink`, a box in the plane, lies in front of the viewer and draws
    /// through the projection's tangent at the box's centre within the tolerance.
    pub(crate) fn tangent_fits(&self, ink: Rect) -> bool {
        let front = projection::front(self.projection);
        let corners = projection::corners(ink);
        let centre = [ink.x + ink.width / 2.0, ink.y + ink.height / 2.0];
        corners
            .iter()
            .all(|&corner| projection::side(front, corner) > 0.0)
            && projection::tangent_error(self.projection, centre, &corners) <= self.tolerance()
    }

    /// The largest stretch of the projection over the part of the plane within `lines`.
    pub(crate) fn largest_stretch(&self, lines: &[Line]) -> f64 {
        let region = projection::clip_all(horizon_square(), lines);
        sample_points(&region)
            .map(|point| projection::stretch(self.projection, point))
            .fold(f64::MIN_POSITIVE, f64::max)
    }

    /// The output pixels of what `render` paints in the plane within `ink`: it renders each
    /// band's pixels, or `None` for none.
    pub(crate) fn render<E: From<PlaneError>>(
        &self,
        ink: Rect,
        mut render: impl FnMut(&Band) -> Result<Option<tiny_skia::Pixmap>, E>,
    ) -> Result<Option<Resampled>, E> {
        let region = projection::clip_all(projection::corners(ink), &self.lines);
        if region.len() < 3 {
            return Ok(None);
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
            bands.extend(self.band(&region, [low, high]));
            low = high;
        }
        let projected: Vec<_> = region
            .iter()
            .map(|&point| projection::apply(self.projection, point))
            .collect();
        let Some(bounds) = projection::bounds(&projected) else {
            return Ok(None);
        };
        // Output pixels start on the grid of the drawing space's pixels.
        let pixel = 1.0 / self.density;
        let [x, y] = [bounds.x, bounds.y].map(|start| (start / pixel).floor() * pixel);
        let target = Rect {
            x,
            y,
            width: bounds.x + bounds.width - x,
            height: bounds.y + bounds.height - y,
        };
        resample(
            self.projection,
            self.density,
            &bands,
            target,
            &self.lines,
            |band| render(band),
        )
    }

    /// The band of `region` with `w` within `range`, with pixel axes along and across the
    /// gradient of `w`, each as dense as the band's most magnified point needs along it.
    fn band(&self, region: &[[f32; 2]], range: [f64; 2]) -> Option<Band> {
        let [g, h, i] = projection::front(self.projection);
        let part = projection::clip_all(
            region.to_vec(),
            &[[g, h, i - range[0]], [-g, -h, range[1] - i]],
        );
        if part.len() < 3 {
            return None;
        }
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
            let margin = BAND_MARGIN_PIXELS / density;
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
            range,
            to_pixels,
            size,
            screen: projection::bounds(&projected)?,
        })
    }
}

impl Band {
    /// The band with margins along its first axis wide enough for samples read from its
    /// `level`th halving to find rendered pixels around them.
    fn widened(&self, level: u8) -> Self {
        let extra = (BAND_MARGIN_PIXELS * (f64::from(level).exp2() - 1.0)).ceil();
        let mut to_pixels = self.to_pixels;
        to_pixels[4] += extra as f32;
        Self {
            range: self.range,
            to_pixels,
            size: [self.size[0] + 2 * extra as u32, self.size[1]],
            screen: self.screen,
        }
    }
    /// The extent of the band's pixel grid, in pixels.
    pub(crate) fn area(&self) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: self.size[0] as f32,
            height: self.size[1] as f32,
        }
    }
}

/// `pixels`, whose pixel grid `to_pixels` maps a plane onto, resampled over `target` at
/// `density` pixels per unit of the drawing space `to_target` projects that plane into.
pub(crate) fn resample_pixmap(
    pixels: &tiny_skia::Pixmap,
    to_pixels: [f32; 6],
    to_target: Projection,
    target: Rect,
    density: f32,
) -> Result<Option<Resampled>, PlaneError> {
    let band = Band {
        range: [0.0, f64::INFINITY],
        to_pixels,
        size: [pixels.width(), pixels.height()],
        screen: target,
    };
    resample(to_target, density, &[band], target, &[], |widened| {
        let mut padded =
            tiny_skia::Pixmap::new(widened.size[0], widened.size[1]).ok_or(PlaneError::Pixmap)?;
        padded.draw_pixmap(
            (widened.to_pixels[4] - to_pixels[4]).round() as i32,
            0,
            pixels.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            tiny_skia::Transform::identity(),
            None,
        );
        Ok(Some(padded))
    })
}

/// A band's pixels and their successive halvings along its first axis.
struct Reductions {
    levels: Vec<tiny_skia::Pixmap>,
}
impl Reductions {
    fn new(pixels: tiny_skia::Pixmap) -> Self {
        let mut levels = vec![pixels];
        while let Some(last) = levels.last()
            && last.width() > 1
            && let Some(next) = halved(last)
        {
            levels.push(next);
        }
        Self { levels }
    }
}

/// `pixels` with each pair of columns averaged into one.
fn halved(pixels: &tiny_skia::Pixmap) -> Option<tiny_skia::Pixmap> {
    let (width, height) = (pixels.width(), pixels.height());
    let columns = width.div_ceil(2);
    let mut output = tiny_skia::Pixmap::new(columns, height)?;
    let source = pixels.pixels();
    let target = output.pixels_mut();
    for row in 0..height as usize {
        for column in 0..columns as usize {
            let left = source[row * width as usize + 2 * column];
            let right = source
                .get(row * width as usize + 2 * column + 1)
                .filter(|_| 2 * column + 1 < width as usize)
                .copied()
                .unwrap_or(tiny_skia::PremultipliedColorU8::TRANSPARENT);
            let mean = |a: u8, b: u8| u8::try_from((u16::from(a) + u16::from(b)).div_ceil(2));
            if let (Ok(red), Ok(green), Ok(blue), Ok(alpha)) = (
                mean(left.red(), right.red()),
                mean(left.green(), right.green()),
                mean(left.blue(), right.blue()),
                mean(left.alpha(), right.alpha()),
            ) && let Some(color) = tiny_skia::PremultipliedColorU8::from_rgba(
                red.min(alpha),
                green.min(alpha),
                blue.min(alpha),
                alpha,
            ) {
                target[row * columns as usize + column] = color;
            }
        }
    }
    Some(output)
}

/// How one output pixel samples the plane: its band, the reduction it reads and its samples
/// per axis; no samples where it shows nothing.
#[derive(Clone, Copy, Default)]
struct Footprint {
    /// The bands the pixel's samples may fall in.
    bands: [u16; 2],
    level: u8,
    samples: [u8; 2],
}

/// The output pixels over `target` at `density` pixels per unit. Each averages samples at
/// the plane points `projection` maps its samples from, where they lie within `lines` in
/// front of the viewer, read from the pixels `render` gives the band holding each.
fn resample<E: From<PlaneError>>(
    projection: Projection,
    density: f32,
    bands: &[Band],
    target: Rect,
    lines: &[Line],
    mut render: impl FnMut(&Band) -> Result<Option<tiny_skia::Pixmap>, E>,
) -> Result<Option<Resampled>, E> {
    let columns = (target.width * density).ceil();
    let rows = (target.height * density).ceil();
    if !(columns >= 1.0 && rows >= 1.0) || bands.is_empty() {
        return Ok(None);
    }
    if columns > MAX_RASTER_SIDE || rows > MAX_RASTER_SIDE {
        return Err(PlaneError::Size {
            width: columns,
            height: rows,
        }
        .into());
    }
    let inverse = projection::inverse(projection).ok_or(PlaneError::Degenerate)?;
    let to_plane = Homogeneous::new(inverse);
    let front = projection::front(projection);
    let inside = |point: [f64; 2]| {
        point_side(front, point) > 0.0 && lines.iter().all(|&line| point_side(line, point) >= 0.0)
    };
    let band_of = |weight: f64| {
        bands
            .partition_point(|band| band.range[1] < weight)
            .min(bands.len() - 1)
    };
    let width = columns as usize;
    let pixel = 1.0 / f64::from(density);
    let corner = |index: usize| {
        [
            f64::from(target.x) + (index % width) as f64 * pixel,
            f64::from(target.y) + (index / width) as f64 * pixel,
        ]
    };
    let footprints: Vec<Footprint> = (0..width * rows as usize)
        .map(|index| {
            let origin = corner(index);
            let middle = [origin[0] + pixel / 2.0, origin[1] + pixel / 2.0];
            let centre = to_plane.apply(middle);
            if !inside(centre) {
                return Footprint::default();
            }
            let weights = [[0.0, 0.0], [pixel, 0.0], [0.0, pixel], [pixel, pixel]]
                .map(|[x, y]| point_side(front, to_plane.apply([origin[0] + x, origin[1] + y])));
            let ahead = weights.iter().all(|&weight| weight > 0.0);
            let range = if ahead {
                [
                    band_of(weights.iter().copied().fold(f64::INFINITY, f64::min)),
                    band_of(weights.iter().copied().fold(0.0, f64::max)),
                ]
            } else {
                [0, bands.len() - 1]
            };
            let band = &bands[band_of(point_side(front, centre))];
            let [a, b, c, d, _, _] = band.to_pixels.map(f64::from);
            // Plane displacements per output pixel step along x and along y.
            let [dx, dy] = to_plane.steps(middle, pixel);
            let span = |[x, y]: [f64; 2]| [a * x + c * y, b * x + d * y];
            let (step_x, step_y) = (span(dx), span(dy));
            let along = step_x[0].abs() + step_y[0].abs();
            let level = (along / REDUCTION_SPAN).log2().ceil().max(0.0);
            let reduction = level.exp2();
            let count = |[first, second]: [f64; 2]| {
                (first / reduction)
                    .hypot(second)
                    .ceil()
                    .clamp(1.0, MAX_SUPERSAMPLES) as u8
            };
            Footprint {
                bands: range.map(|band| band as u16),
                level: level as u8,
                samples: [count(step_x), count(step_y)],
            }
        })
        .collect();
    let mut levels = vec![0_u8; bands.len()];
    for footprint in footprints
        .iter()
        .filter(|footprint| footprint.samples[0] > 0)
    {
        let [first, last] = footprint.bands.map(usize::from);
        for level in &mut levels[first..=last] {
            *level = (*level).max(footprint.level);
        }
    }
    let mut sums = vec![[0.0_f32; 4]; footprints.len()];
    for (band_index, (band, level)) in bands.iter().zip(levels).enumerate() {
        let band = band.widened(level);
        let Some(pixels) = render(&band)? else {
            continue;
        };
        let reductions = Reductions::new(pixels);
        let [a, b, c, d, e, f] = band.to_pixels.map(f64::from);
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
                let footprint = footprints[index];
                let [first, last] = footprint.bands.map(usize::from);
                if footprint.samples[0] == 0 || !(first..=last).contains(&band_index) {
                    continue;
                }
                let level = usize::from(footprint.level).min(reductions.levels.len() - 1);
                let reduced = &reductions.levels[level];
                let shrink = (level as f64).exp2();
                let [across, down] = footprint.samples.map(f64::from);
                let origin = corner(index);
                for sample in 0..u16::from(footprint.samples[0]) * u16::from(footprint.samples[1]) {
                    let point = to_plane.apply([
                        origin[0]
                            + (f64::from(sample % u16::from(footprint.samples[0])) + 0.5) * pixel
                                / across,
                        origin[1]
                            + (f64::from(sample / u16::from(footprint.samples[0])) + 0.5) * pixel
                                / down,
                    ]);
                    let weight = point_side(front, point);
                    if (first != last && !(band.range[0]..=band.range[1]).contains(&weight))
                        || !inside(point)
                    {
                        continue;
                    }
                    let value = bilinear(
                        reduced,
                        [
                            ((a * point[0] + c * point[1] + e) / shrink) as f32,
                            (b * point[0] + d * point[1] + f) as f32,
                        ],
                    );
                    for (total, value) in sums[index].iter_mut().zip(value) {
                        *total += value;
                    }
                }
            }
        }
    }
    let mut pixmap =
        tiny_skia::Pixmap::new(columns as u32, rows as u32).ok_or(PlaneError::Pixmap)?;
    for ((output, sum), footprint) in pixmap.pixels_mut().iter_mut().zip(&sums).zip(&footprints) {
        let count = f32::from(footprint.samples[0]) * f32::from(footprint.samples[1]);
        if count == 0.0 {
            continue;
        }
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
    Ok(Some(Resampled {
        pixmap,
        area: Rect {
            x: target.x,
            y: target.y,
            width: columns / density,
            height: rows / density,
        },
    }))
}

/// A projection's homogeneous rows, applied in double precision.
struct Homogeneous {
    rows: [[f64; 3]; 3],
}
impl Homogeneous {
    fn new(m: Projection) -> Self {
        let m = m.map(f64::from);
        Self {
            rows: [[m[0], m[2], m[4]], [m[1], m[3], m[5]], [m[6], m[7], m[8]]],
        }
    }
    fn apply(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        let [r0, r1, r2] = &self.rows;
        let w = r2[0] * x + r2[1] * y + r2[2];
        [
            (r0[0] * x + r0[1] * y + r0[2]) / w,
            (r1[0] * x + r1[1] * y + r1[2]) / w,
        ]
    }
    /// The displacements of the image of `point` per `step` along x and along y.
    fn steps(&self, [x, y]: [f64; 2], step: f64) -> [[f64; 2]; 2] {
        let [r0, r1, r2] = &self.rows;
        let w = r2[0] * x + r2[1] * y + r2[2];
        let [u, v] = self.apply([x, y]);
        std::array::from_fn(|axis| {
            [
                (r0[axis] - u * r2[axis]) / w * step,
                (r1[axis] - v * r2[axis]) / w * step,
            ]
        })
    }
}

/// The value of `line` at `p`.
fn point_side(line: Line, [x, y]: [f64; 2]) -> f64 {
    line[0] * x + line[1] * y + line[2]
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
pub(crate) fn horizon_square() -> Vec<[f32; 2]> {
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

#[cfg(test)]
mod tests {
    use super::*;

    const DENSITY: f32 = 4.0;
    const DEPTH: f32 = 100.0;
    const COLOR: [u8; 4] = [40, 120, 200, 255];
    /// A tall plane receding to the right: across it `w` grows almost sixfold and the
    /// perspective converges its far corners, so bands read several halvings of their pixels.
    const PLANE: Rect = Rect {
        x: 0.0,
        y: -200.0,
        width: 400.0,
        height: 400.0,
    };

    /// `perspective(100pt) rotateY(80deg)` about the origin: `(x, y, 0)` turns to
    /// `(cos·x, y, -sin·x)`, whose `w` is `1 + sin·x / 100`.
    fn receding() -> Projection {
        let (sin, cos) = 80.0_f32.to_radians().sin_cos();
        [cos, 0.0, 0.0, 1.0, 0.0, 0.0, sin / DEPTH, 0.0, 1.0]
    }

    /// The pixels of `PLANE` filled with `COLOR` over `band`.
    fn filled(band: &Band) -> Result<Option<tiny_skia::Pixmap>, PlaneError> {
        let mut pixmap =
            tiny_skia::Pixmap::new(band.size[0], band.size[1]).ok_or(PlaneError::Pixmap)?;
        let [a, b, c, d, e, f] = band.to_pixels;
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(COLOR[0], COLOR[1], COLOR[2], COLOR[3]);
        let rect = tiny_skia::Rect::from_xywh(PLANE.x, PLANE.y, PLANE.width, PLANE.height)
            .ok_or(PlaneError::Degenerate)?;
        let transform = tiny_skia::Transform::from_row(a, b, c, d, e, f);
        pixmap.fill_rect(rect, &paint, transform, None);
        Ok(Some(pixmap))
    }

    #[test]
    fn a_uniform_plane_resamples_to_its_colour_without_seams_between_bands() {
        let m = receding();
        let visible = Rect {
            x: -1000.0,
            y: -1000.0,
            width: 2000.0,
            height: 2000.0,
        };
        let mut bands = 0;
        let resampled = Plane::new(m, visible, DENSITY)
            .render(PLANE, |band| {
                bands += 1;
                filled(band)
            })
            .expect("the plane resamples")
            .expect("the plane covers output pixels");
        assert!(
            bands > 3,
            "w varies sixfold, so several bands render: {bands}"
        );
        let inverse = projection::inverse(m).expect("the plane is invertible");
        let pixel = 1.0 / DENSITY;
        let width = resampled.pixmap.width() as usize;
        let within = |[x, y]: [f32; 2]| {
            (PLANE.x..=PLANE.x + PLANE.width).contains(&x)
                && (PLANE.y..=PLANE.y + PLANE.height).contains(&y)
        };
        let (mut inside, mut outside, mut failures) = (0, 0, Vec::new());
        for (index, found) in resampled.pixmap.pixels().iter().enumerate() {
            let x = resampled.area.x + (index % width) as f32 * pixel;
            let y = resampled.area.y + (index / width) as f32 * pixel;
            // The plane points under the pixel and its neighbours, which samples may blend in.
            let around: Vec<[f32; 2]> = [[-1.0, -1.0], [2.0, -1.0], [2.0, 2.0], [-1.0, 2.0]]
                .iter()
                .map(|[dx, dy]| projection::apply(inverse, [x + dx * pixel, y + dy * pixel]))
                .collect();
            let found = [found.red(), found.green(), found.blue(), found.alpha()];
            if around.iter().all(|&point| within(point)) {
                inside += 1;
                if found
                    .iter()
                    .zip(COLOR)
                    .any(|(&value, wanted)| value.abs_diff(wanted) > 1)
                {
                    failures.push(format!("({x}, {y}) inside the plane is {found:?}"));
                }
            } else if around.iter().all(|&[_, y]| y.abs() > PLANE.height / 2.0) {
                outside += 1;
                if found[3] != 0 {
                    failures.push(format!("({x}, {y}) beyond the plane is {found:?}"));
                }
            }
        }
        assert!(
            inside > 10_000 && outside > 1_000,
            "checked {inside} pixels inside the plane and {outside} beyond it"
        );
        assert!(
            failures.is_empty(),
            "{} pixels of {inside} inside must be {COLOR:?} and of {outside} beyond transparent:\n{}",
            failures.len(),
            failures[..failures.len().min(20)].join("\n")
        );
    }
}
