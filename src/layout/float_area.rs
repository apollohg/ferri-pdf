//! CSS Shapes 1 float areas: a float's `shape-outside`, expanded by `shape-margin` and clipped
//! to its margin box (§2, §6), kept as convex pieces. A line box beside the float is shortened
//! by the area's extent over the line box's whole block range.
use crate::graphics::{BackgroundImage, BoxArea, BoxPaint, Contour, Point, Rect, Shape};
use crate::images::Image;
use crate::layout::{LayoutError, work::Budget};
use crate::paint::{ShapeBox, ShapeOutside};
use crate::style::FloatSide;
use std::sync::Arc;

/// Largest distance of flattened curves and `shape-margin` arcs from the true outline.
const FLATTEN_TOLERANCE_PT: f32 = 0.05;
/// Fewest vertices approximating the circle that `shape-margin` sweeps around a point.
const MIN_MARGIN_ARC_VERTICES: usize = 8;

/// A convex polygon, or a segment or point where the shape encloses no area.
#[derive(Clone, Debug, PartialEq)]
struct Piece {
    points: Vec<Point>,
    top: f32,
    bottom: f32,
}

impl Piece {
    fn new(points: Vec<Point>) -> Option<Self> {
        let top = points.iter().map(|point| point[1]).reduce(f32::min)?;
        let bottom = points.iter().map(|point| point[1]).reduce(f32::max)?;
        Some(Self {
            points,
            top,
            bottom,
        })
    }

    /// The outermost horizontal position on the wrapping side within rows `y0..=y1`.
    fn extent(&self, [y0, y1]: [f32; 2], side: FloatSide) -> Option<f32> {
        let farther = |a: f32, b: f32| match side {
            FloatSide::Left => a.max(b),
            FloatSide::Right => a.min(b),
        };
        let mut extent: Option<f32> = None;
        let count = self.points.len();
        for index in 0..count {
            let [a, b] = [self.points[index], self.points[(index + 1) % count]];
            let low = a[1].min(b[1]).max(y0);
            let high = a[1].max(b[1]).min(y1);
            if low > high {
                continue;
            }
            let x_at = |y: f32| {
                if a[1] == b[1] {
                    farther(a[0], b[0])
                } else {
                    a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1])
                }
            };
            let edge = farther(x_at(low), x_at(high));
            extent = Some(extent.map_or(edge, |current| farther(current, edge)));
        }
        extent
    }
}

/// Whether a box whose block extent is `top..bottom` affects a line box at `y` of `height`:
/// edges that only touch it do not, and a zero-height line box meets what covers `y`.
pub(super) fn meets(top: f32, bottom: f32, y: f32, height: f32) -> bool {
    bottom > y && (top < y + height || (height == 0.0 && top <= y))
}

/// A float area, empty when its shape encloses nothing and has no block extent; each piece
/// lies within the float's margin box.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FloatArea {
    /// Sorted by top edge.
    pieces: Vec<Piece>,
    tallest: f32,
}

impl FloatArea {
    /// The area's edge on the side lines wrap around, over a line box at `y` of `height`;
    /// `None` when the line box does not meet the area.
    pub(crate) fn edge(
        &self,
        y: f32,
        height: f32,
        side: FloatSide,
        work: &Budget,
    ) -> Result<Option<f32>, LayoutError> {
        let first = self
            .pieces
            .partition_point(|piece| piece.top < y - self.tallest);
        let mut edge: Option<f32> = None;
        for piece in &self.pieces[first..] {
            work.spend(piece.points.len())?;
            if piece.top > y + height {
                break;
            }
            if !meets(piece.top, piece.bottom, y, height) {
                continue;
            }
            if let Some(extent) = piece.extent([y, y + height], side) {
                edge = Some(edge.map_or(extent, |current| match side {
                    FloatSide::Left => current.max(extent),
                    FloatSide::Right => current.min(extent),
                }));
            }
        }
        Ok(edge)
    }

    /// Line box positions where a vertex enters or leaves a line box of `height`, between
    /// which the area's edge moves linearly.
    pub(crate) fn breakpoints(&self, height: f32, output: &mut Vec<f32>) {
        for piece in &self.pieces {
            for point in &piece.points {
                output.extend([point[1], point[1] - height]);
            }
        }
    }

    pub(crate) fn bottom(&self) -> f32 {
        self.pieces
            .iter()
            .map(|piece| piece.bottom)
            .fold(f32::NEG_INFINITY, f32::max)
    }
}

/// The used box edges a float area can derive from.
pub(crate) struct FloatBoxes<'a> {
    /// The float's border box fragment; a sliced fragment carries its assembled box.
    pub paint: &'a BoxPaint,
    /// Used margins of the box `paint` decorates.
    pub margins: [f32; 4],
    /// The fragment's margin box, which bounds the float area.
    pub bounds: Rect,
}

/// The float area `shape` defines for one float fragment; `none` defines none, which leaves
/// the margin box.
pub(crate) fn float_area(
    shape: &ShapeOutside<Arc<Image>>,
    margin: f32,
    threshold: f32,
    boxes: &FloatBoxes<'_>,
    work: &Budget,
) -> Result<Option<FloatArea>, LayoutError> {
    let mut pieces = match shape {
        ShapeOutside::None => return Ok(None),
        ShapeOutside::Box(reference) => {
            let (rect, radii) = reference_box(*reference, boxes);
            flatten(&crate::paint::rounded_rect(rect, radii))
                .into_iter()
                .filter_map(Piece::new)
                .collect()
        }
        ShapeOutside::Shape { shape, reference } => {
            let Shape::Path {
                contours, even_odd, ..
            } = shape.outline(reference_box(*reference, boxes).0)
            else {
                unreachable!("basic shapes outline as paths")
            };
            let polygons = contours.iter().flat_map(flatten).collect::<Vec<_>>();
            let mut pieces = trapezoids(&polygons, even_odd, work)?;
            if margin > 0.0 {
                pieces.extend(polygons.iter().flat_map(|polygon| {
                    polygon.iter().enumerate().filter_map(|(index, point)| {
                        Piece::new(vec![*point, polygon[(index + 1) % polygon.len()]])
                    })
                }));
            }
            pieces
        }
        ShapeOutside::Image(image) => {
            let (content, _) = reference_box(ShapeBox::Visual(BoxArea::ContentBox), boxes);
            image_pieces(image, content, threshold, work)?
        }
    };
    if margin > 0.0 {
        for piece in &mut pieces {
            work.spend(piece.points.len())?;
            *piece = Piece::new(dilate(&piece.points, margin))
                .expect("a dilated piece keeps its vertices");
        }
    }
    let mut pieces = pieces
        .into_iter()
        .filter_map(|piece| Piece::new(clip(&piece.points, boxes.bounds)))
        .filter(|piece| piece.bottom > piece.top)
        .collect::<Vec<_>>();
    pieces.sort_by(|a, b| a.top.total_cmp(&b.top));
    let tallest = pieces
        .iter()
        .map(|piece| piece.bottom - piece.top)
        .fold(0.0, f32::max);
    Ok(Some(FloatArea { pieces, tallest }))
}

/// Convex pieces of a filled path, with the same curve tolerance and fill rules as floats.
pub(super) fn filled_polygons(
    shape: &Shape,
    work: &Budget,
) -> Result<Vec<Vec<Point>>, LayoutError> {
    match shape {
        Shape::Rect(rect) => Ok(vec![super::navigation::rect_corners(*rect).to_vec()]),
        Shape::Path {
            contours, even_odd, ..
        } => {
            let polygons = contours.iter().flat_map(flatten).collect::<Vec<_>>();
            Ok(trapezoids(&polygons, *even_odd, work)?
                .into_iter()
                .map(|piece| piece.points)
                .collect())
        }
        _ => unreachable!("a CSS clip-path outlines a rectangle or filled path"),
    }
}

/// Intersection of convex filled polygons, keeping touching edges for zero-area anchors.
pub(super) fn intersect(subject: &[Point], boundary: &[Point]) -> Vec<Point> {
    if boundary.len() < 3 {
        return Vec::new();
    }
    let area: f32 = boundary
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let b = boundary[(i + 1) % boundary.len()];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if area == 0.0 {
        return Vec::new();
    }
    let mut output = subject.to_vec();
    for (index, &a) in boundary.iter().enumerate() {
        let b = boundary[(index + 1) % boundary.len()];
        let side = |p: Point| {
            ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])) * area.signum()
        };
        let input = std::mem::take(&mut output);
        for (index, &current) in input.iter().enumerate() {
            let previous = input[(index + input.len() - 1) % input.len()];
            let [before, after] = [side(previous), side(current)];
            if (before >= 0.0) != (after >= 0.0) {
                let t = before / (before - after);
                output.push([
                    previous[0] + t * (current[0] - previous[0]),
                    previous[1] + t * (current[1] - previous[1]),
                ]);
            }
            if after >= 0.0 {
                output.push(current);
            }
        }
    }
    output
}

fn reference_box(reference: ShapeBox, boxes: &FloatBoxes<'_>) -> (Rect, crate::graphics::Radii) {
    reference.resolve(boxes.paint, boxes.margins)
}

/// The closed polygon approximating a contour within `FLATTEN_TOLERANCE_PT`.
fn flatten(contour: &Contour) -> Vec<Vec<Point>> {
    let mut polygons: Vec<Vec<Point>> = Vec::new();
    kurbo::flatten(
        contour
            .path_elements()
            .chain((!contour.closed).then_some(kurbo::PathEl::ClosePath)),
        f64::from(FLATTEN_TOLERANCE_PT),
        |element| match element {
            kurbo::PathEl::MoveTo(start) => polygons.push(vec![[start.x as f32, start.y as f32]]),
            kurbo::PathEl::LineTo(end) => {
                if let Some(polygon) = polygons.last_mut() {
                    polygon.push([end.x as f32, end.y as f32]);
                }
            }
            _ => {}
        },
    );
    for polygon in &mut polygons {
        if polygon.len() > 1 && polygon.first() == polygon.last() {
            polygon.pop();
        }
    }
    polygons
}

/// An edge that is not horizontal, top end first, with +1 winding when drawn downwards.
#[derive(Clone, Copy)]
struct Edge {
    top: Point,
    bottom: Point,
    winding: i32,
}

impl Edge {
    fn x_at(self, y: f32) -> f32 {
        self.top[0]
            + (self.bottom[0] - self.top[0]) * (y - self.top[1]) / (self.bottom[1] - self.top[1])
    }
}

/// The region `polygons` enclose under the fill rule, as trapezoids between the rows where
/// a vertex lies or two edges cross; zero-width spans keep the edges of shapes without area.
fn trapezoids(
    polygons: &[Vec<Point>],
    even_odd: bool,
    work: &Budget,
) -> Result<Vec<Piece>, LayoutError> {
    let edges = polygons
        .iter()
        .flat_map(|polygon| {
            polygon.iter().enumerate().filter_map(|(index, &start)| {
                let end = polygon[(index + 1) % polygon.len()];
                match start[1].total_cmp(&end[1]) {
                    std::cmp::Ordering::Less => Some(Edge {
                        top: start,
                        bottom: end,
                        winding: 1,
                    }),
                    std::cmp::Ordering::Greater => Some(Edge {
                        top: end,
                        bottom: start,
                        winding: -1,
                    }),
                    std::cmp::Ordering::Equal => None,
                }
            })
        })
        .collect::<Vec<_>>();
    let mut rows = edges
        .iter()
        .flat_map(|edge| [edge.top[1], edge.bottom[1]])
        .collect::<Vec<_>>();
    work.spend(edges.len().saturating_mul(edges.len()))?;
    for (index, a) in edges.iter().enumerate() {
        for b in &edges[index + 1..] {
            let low = a.top[1].max(b.top[1]);
            let high = a.bottom[1].min(b.bottom[1]);
            if low >= high {
                continue;
            }
            let [start, end] = [a.x_at(low) - b.x_at(low), a.x_at(high) - b.x_at(high)];
            if start * end < 0.0 {
                rows.push(low + (high - low) * start / (start - end));
            }
        }
    }
    rows.sort_by(f32::total_cmp);
    rows.dedup();
    let inside = |winding: i32| {
        if even_odd {
            winding % 2 != 0
        } else {
            winding != 0
        }
    };
    let mut pieces = Vec::new();
    for strip in rows.windows(2) {
        let [top, bottom] = [strip[0], strip[1]];
        if bottom <= top {
            continue;
        }
        let middle = f32::midpoint(top, bottom);
        let mut crossing = edges
            .iter()
            .filter(|edge| edge.top[1] <= top && edge.bottom[1] >= bottom)
            .map(|edge| (edge.x_at(middle), *edge))
            .collect::<Vec<_>>();
        work.spend(crossing.len().max(1))?;
        crossing.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0;
        let mut start: Option<Edge> = None;
        for (_, edge) in crossing {
            let before = inside(winding);
            winding += edge.winding;
            match (before, inside(winding)) {
                (false, true) => start = Some(edge),
                (true, false) => {
                    let left = start.take().expect("a span starts before it ends");
                    pieces.extend(Piece::new(vec![
                        [left.x_at(top), top],
                        [edge.x_at(top), top],
                        [edge.x_at(bottom), bottom],
                        [left.x_at(bottom), bottom],
                    ]));
                }
                _ => {}
            }
        }
    }
    Ok(pieces)
}

/// The convex polygon swept by a disc of `radius` over a convex piece (§6.3): the convex hull
/// of polygonal circles around its vertices.
fn dilate(points: &[Point], radius: f32) -> Vec<Point> {
    let step = (1.0 - FLATTEN_TOLERANCE_PT / radius)
        .clamp(-1.0, 1.0)
        .acos();
    let vertices = ((std::f32::consts::PI / step).ceil() as usize).max(MIN_MARGIN_ARC_VERTICES);
    let circle = (0..vertices)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / vertices as f32;
            [radius * angle.cos(), radius * angle.sin()]
        })
        .collect::<Vec<_>>();
    hull(
        points
            .iter()
            .flat_map(|point| {
                circle
                    .iter()
                    .map(move |offset| [point[0] + offset[0], point[1] + offset[1]])
            })
            .collect(),
    )
}

/// Andrew's monotone chain convex hull.
fn hull(mut points: Vec<Point>) -> Vec<Point> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let chain = |points: &mut dyn Iterator<Item = Point>| {
        let mut chain: Vec<Point> = Vec::new();
        for point in points {
            while let [.., o, a] = chain[..] {
                if (a[0] - o[0]) * (point[1] - o[1]) - (a[1] - o[1]) * (point[0] - o[0]) > 0.0 {
                    break;
                }
                chain.pop();
            }
            chain.push(point);
        }
        chain.pop();
        chain
    };
    let mut lower = chain(&mut points.iter().copied());
    lower.extend(chain(&mut points.iter().rev().copied()));
    lower
}

/// Sutherland–Hodgman clipping of a convex polygon to `bounds`.
fn clip(points: &[Point], bounds: Rect) -> Vec<Point> {
    let planes: [(usize, f32, bool); 4] = [
        (0, bounds.x, true),
        (0, bounds.x + bounds.width, false),
        (1, bounds.y, true),
        (1, bounds.y + bounds.height, false),
    ];
    let mut polygon = points.to_vec();
    for (axis, limit, lower) in planes {
        let keep = |point: Point| {
            if lower {
                point[axis] >= limit
            } else {
                point[axis] <= limit
            }
        };
        let input = std::mem::take(&mut polygon);
        for (index, &current) in input.iter().enumerate() {
            let previous = input[(index + input.len() - 1) % input.len()];
            if keep(current) != keep(previous) {
                let t = (limit - previous[axis]) / (current[axis] - previous[axis]);
                let mut crossing = [
                    previous[0] + (current[0] - previous[0]) * t,
                    previous[1] + (current[1] - previous[1]) * t,
                ];
                crossing[axis] = limit;
                polygon.push(crossing);
            }
            if keep(current) {
                polygon.push(current);
            }
        }
    }
    polygon
}

/// Eight-bit opacity samples covering an image's area row by row.
struct AlphaGrid {
    columns: usize,
    rows: usize,
    alpha: Vec<u8>,
}

const MAX_SAMPLE: f32 = u8::MAX as f32;
/// Gradients and SVG images are rendered at the SVG filter resolution, coarser where that
/// would take more samples than this.
const MAX_RENDERED_SAMPLES: f32 = 4_194_304.0;

/// §4: the pixels of `image`, sized to the content box, whose alpha exceeds `threshold`, as
/// rectangles merged down runs of identical rows.
fn image_pieces(
    image: &BackgroundImage,
    content: Rect,
    threshold: f32,
    work: &Budget,
) -> Result<Vec<Piece>, LayoutError> {
    let Some(grid) = alpha_grid(image, content, work)? else {
        return Ok(Vec::new());
    };
    let opaque = |sample: u8| f32::from(sample) > threshold * MAX_SAMPLE;
    let cell = [
        content.width / grid.columns as f32,
        content.height / grid.rows as f32,
    ];
    let mut pieces: Vec<Piece> = Vec::new();
    let mut open: Vec<(usize, usize, usize)> = Vec::new();
    let close = |run: (usize, usize, usize), end: usize, pieces: &mut Vec<Piece>| {
        let (start, last, first_row) = run;
        let [left, right] = [start, last].map(|column| content.x + column as f32 * cell[0]);
        let [top, bottom] = [first_row, end].map(|row| content.y + row as f32 * cell[1]);
        pieces.extend(Piece::new(vec![
            [left, top],
            [right, top],
            [right, bottom],
            [left, bottom],
        ]));
    };
    for row in 0..=grid.rows {
        let mut runs = Vec::new();
        if row < grid.rows {
            work.spend(grid.columns)?;
            let samples = &grid.alpha[row * grid.columns..(row + 1) * grid.columns];
            let mut column = 0;
            while column < grid.columns {
                if opaque(samples[column]) {
                    let start = column;
                    while column < grid.columns && opaque(samples[column]) {
                        column += 1;
                    }
                    runs.push((start, column));
                } else {
                    column += 1;
                }
            }
        }
        let continues = open.len() == runs.len()
            && open
                .iter()
                .zip(&runs)
                .all(|(open, run)| (open.0, open.1) == *run);
        if !continues {
            for run in open.drain(..) {
                close(run, row, &mut pieces);
            }
            open.extend(runs.into_iter().map(|(start, end)| (start, end, row)));
        }
    }
    Ok(pieces)
}

fn alpha_grid(
    image: &BackgroundImage,
    content: Rect,
    work: &Budget,
) -> Result<Option<AlphaGrid>, LayoutError> {
    if content.width <= 0.0 || content.height <= 0.0 {
        return Ok(None);
    }
    let density = crate::svg::krilla_svg::SvgSettings::default()
        .filter_scale
        .min((MAX_RENDERED_SAMPLES / (content.width * content.height)).sqrt());
    let samples = |extent: f32| ((extent * density).ceil() as usize).max(1);
    let grid = match image {
        BackgroundImage::None | BackgroundImage::Url { .. } => {
            unreachable!("floats load their shape-outside image")
        }
        BackgroundImage::Gradient(gradient) => {
            let Some(resolved) = gradient.resolve([content.width, content.height]) else {
                return Ok(None);
            };
            let [columns, rows] = [samples(content.width), samples(content.height)];
            work.spend(columns.saturating_mul(rows))?;
            let alpha = (0..rows)
                .flat_map(|row| (0..columns).map(move |column| (row, column)))
                .map(|(row, column)| {
                    let alpha = resolved.alpha_at([
                        (column as f32 + 0.5) * content.width / columns as f32,
                        (row as f32 + 0.5) * content.height / rows as f32,
                    ]);
                    (alpha * MAX_SAMPLE).round() as u8
                })
                .collect();
            AlphaGrid {
                columns,
                rows,
                alpha,
            }
        }
        BackgroundImage::Image { image, .. } => match &**image {
            Image::Raster(raster) => {
                let (columns, samples) = raster.alpha()?;
                work.spend(samples.len())?;
                AlphaGrid {
                    columns: columns as usize,
                    rows: samples.len() / columns as usize,
                    alpha: samples,
                }
            }
            Image::Pixels(pixels) => {
                let (columns, rows) = pixels.size();
                let count = columns as usize * rows as usize;
                work.spend(count)?;
                let alpha = match pixels.alpha() {
                    // Sixteen-bit samples are big-endian; the high byte orders them.
                    Some(alpha) => alpha
                        .chunks_exact(pixels.depth().bytes())
                        .map(|sample| sample[0])
                        .collect(),
                    None => vec![u8::MAX; count],
                };
                AlphaGrid {
                    columns: columns as usize,
                    rows: rows as usize,
                    alpha,
                }
            }
            Image::Svg(svg) => {
                let [columns, rows] = [samples(content.width), samples(content.height)];
                work.spend(columns.saturating_mul(rows))?;
                let size = [content.width, content.height]
                    .map(|extent| extent / crate::paint::POINTS_PER_PIXEL);
                let tree = svg.tree(size)?;
                let mut pixmap = tiny_skia::Pixmap::new(columns as u32, rows as u32)
                    .ok_or(crate::images::ImageError::SvgSize)?;
                let tree_size = tree.size();
                crate::svg::resvg::render(
                    &tree,
                    tiny_skia::Transform::from_scale(
                        columns as f32 / tree_size.width(),
                        rows as f32 / tree_size.height(),
                    ),
                    &mut pixmap.as_mut(),
                );
                AlphaGrid {
                    columns,
                    rows,
                    alpha: pixmap.pixels().iter().map(|pixel| pixel.alpha()).collect(),
                }
            }
            Image::Transparent => return Ok(None),
            Image::Frame(..) | Image::Viewport(_) | Image::Widget(_) => {
                return Err(crate::style::StyleError::Unsupported(
                    "shape-outside: an image of a nested document or form control".into(),
                )
                .into());
            }
        },
    };
    Ok(Some(grid))
}
