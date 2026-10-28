//! CSS Images 3 gradients: computed values and their resolution into PDF shading
//! geometry with stops normalised to the shading's own `0..=1` domain.
use crate::{Color, LengthPercentage, Point, background::Position};
use serde::Serialize;

/// Interior samples per stop pair whose premultiplied or hinted interpolation is not linear.
const SEGMENT_SAMPLES: usize = 16;
/// Repeated stops beyond this count render as the average colour (CSS Images 3 §3.4.3).
const MAX_REPEATED_STOPS: usize = 4096;
/// Substitute extent for a zero radius (CSS Images 3 §3.2.2 degenerate shapes).
const DEGENERATE_SMALL_PT: f32 = 1.0e-3;
/// Substitute extent for the other radius of a zero-width or zero-height ellipse.
const DEGENERATE_LARGE_PT: f32 = 1.0e6;
/// Minimum shading range so that coincident stops keep distinct inner and outer colours.
const MIN_RANGE_PT: f32 = 1.0e-4;
const FULL_TURN_DEG: f32 = 360.0;
const STRAIGHT_DEG: f32 = 180.0;
const HALF_WEIGHT: f32 = 0.5;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Gradient {
    pub shape: GradientShape,
    pub items: Vec<GradientItem>,
    pub repeating: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum GradientShape {
    Linear(LineDirection),
    Radial {
        ending: RadialEnding,
        center: Position,
    },
}
/// Clockwise angle from "to top" in degrees, or a corner whose angle depends on the box.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum LineDirection {
    Angle(f32),
    Corner { right: bool, bottom: bool },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum RadialExtent {
    ClosestSide,
    FarthestSide,
    ClosestCorner,
    FarthestCorner,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum RadialEnding {
    Circle(CircleSize),
    Ellipse(EllipseSize),
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum CircleSize {
    Extent(RadialExtent),
    Radius(f32),
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum EllipseSize {
    Extent(RadialExtent),
    Radii([LengthPercentage; 2]),
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum GradientItem {
    Stop {
        color: Color,
        position: Option<LengthPercentage>,
    },
    Hint(LengthPercentage),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Color,
}
/// Shading geometry in tile coordinates with positive y down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shading {
    Axial {
        start: Point,
        end: Point,
    },
    /// Concentric circles scaled vertically by `scale_y` about the centre.
    Radial {
        center: Point,
        radii: [f32; 2],
        scale_y: f32,
    },
}
/// Shading whose stops are sorted, padded beyond `0..=1`, and linear in non-premultiplied
/// colour between neighbours.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedGradient {
    pub shading: Shading,
    pub stops: Vec<GradientStop>,
}

#[derive(Clone, Copy, Debug)]
struct Placed {
    position: f32,
    color: Color,
}

impl Gradient {
    /// Resolves the gradient for an image of `size`; `None` when it has no area.
    pub fn resolve(&self, size: [f32; 2]) -> Option<ResolvedGradient> {
        let [width, height] = size;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let (ray, axis) = match &self.shape {
            GradientShape::Linear(direction) => {
                let angle = direction.angle(size).to_radians();
                let direction = [angle.sin(), -angle.cos()];
                let length = (width * direction[0]).abs() + (height * direction[1]).abs();
                let start = [
                    width / 2.0 - direction[0] * length / 2.0,
                    height / 2.0 - direction[1] * length / 2.0,
                ];
                (length, Axis::Line { start, direction })
            }
            GradientShape::Radial { ending, center } => {
                let center = [center[0].resolve(width), center[1].resolve(height)];
                let [rx, ry] = degenerate(ending.radii(center, size));
                (
                    rx,
                    Axis::Ray {
                        center,
                        radii: [rx, ry],
                    },
                )
            }
        };
        let placed = dense(&fixup(&self.items, ray));
        let (low, high) = match axis {
            Axis::Line { .. } => (0.0, ray),
            Axis::Ray { center, radii } if self.repeating => (0.0, farthest(center, radii, size)),
            Axis::Ray { .. } => {
                let first = placed[0].position.max(0.0);
                let last = placed[placed.len() - 1].position;
                if last <= 0.0 {
                    return Some(axis.solid(placed[placed.len() - 1].color));
                }
                (first, last)
            }
        };
        let high = high.max(low + MIN_RANGE_PT);
        let placed = if self.repeating {
            match repeat(&placed, low, high) {
                Some(repeated) => repeated,
                None => return Some(axis.solid(average(&placed))),
            }
        } else {
            placed
        };
        let stops = clamp(&placed, low, high)
            .into_iter()
            .map(|stop| GradientStop {
                offset: (stop.position - low) / (high - low),
                color: stop.color,
            })
            .collect();
        Some(ResolvedGradient {
            shading: axis.shading(low, high),
            stops,
        })
    }
}

impl LineDirection {
    /// Clockwise degrees from "to top"; corners point perpendicular to the other diagonal.
    pub fn angle(self, [width, height]: [f32; 2]) -> f32 {
        match self {
            Self::Angle(degrees) => degrees.rem_euclid(FULL_TURN_DEG),
            Self::Corner { right, bottom } => {
                let toward_top = height.atan2(width).to_degrees();
                match (right, bottom) {
                    (true, false) => toward_top,
                    (true, true) => STRAIGHT_DEG - toward_top,
                    (false, true) => STRAIGHT_DEG + toward_top,
                    (false, false) => FULL_TURN_DEG - toward_top,
                }
            }
        }
    }
}

impl RadialExtent {
    /// Distances from `center` to the selected sides, as `[horizontal, vertical]`.
    fn sides(self, center: Point, [width, height]: [f32; 2]) -> [f32; 2] {
        let horizontal = [center[0].abs(), (width - center[0]).abs()];
        let vertical = [center[1].abs(), (height - center[1]).abs()];
        match self {
            Self::ClosestSide | Self::ClosestCorner => [
                horizontal[0].min(horizontal[1]),
                vertical[0].min(vertical[1]),
            ],
            Self::FarthestSide | Self::FarthestCorner => [
                horizontal[0].max(horizontal[1]),
                vertical[0].max(vertical[1]),
            ],
        }
    }
    fn circle(self, center: Point, size: [f32; 2]) -> f32 {
        let [x, y] = self.sides(center, size);
        match self {
            Self::ClosestSide => x.min(y),
            Self::FarthestSide => x.max(y),
            Self::ClosestCorner | Self::FarthestCorner => x.hypot(y),
        }
    }
    /// Corner extents keep the side extents' aspect ratio and pass through the corner.
    fn ellipse(self, center: Point, size: [f32; 2]) -> [f32; 2] {
        let [x, y] = self.sides(center, size);
        match self {
            Self::ClosestSide | Self::FarthestSide => [x, y],
            Self::ClosestCorner | Self::FarthestCorner if x == 0.0 || y == 0.0 => [x, y],
            Self::ClosestCorner | Self::FarthestCorner => {
                [x * std::f32::consts::SQRT_2, y * std::f32::consts::SQRT_2]
            }
        }
    }
}

impl RadialEnding {
    fn radii(&self, center: Point, size: [f32; 2]) -> [f32; 2] {
        match self {
            Self::Circle(CircleSize::Extent(extent)) => [extent.circle(center, size); 2],
            Self::Circle(CircleSize::Radius(radius)) => [*radius; 2],
            Self::Ellipse(EllipseSize::Extent(extent)) => extent.ellipse(center, size),
            Self::Ellipse(EllipseSize::Radii([x, y])) => [x.resolve(size[0]), y.resolve(size[1])],
        }
    }
}

/// Zero radii become thin lines or tiny circles (CSS Images 3 §3.2.2).
fn degenerate([rx, ry]: [f32; 2]) -> [f32; 2] {
    match (rx > 0.0, ry > 0.0) {
        (true, true) => [rx, ry],
        (false, false) => [DEGENERATE_SMALL_PT; 2],
        (false, true) => [DEGENERATE_SMALL_PT, DEGENERATE_LARGE_PT],
        (true, false) => [DEGENERATE_LARGE_PT, DEGENERATE_SMALL_PT],
    }
}

#[derive(Clone, Copy)]
enum Axis {
    Line { start: Point, direction: Point },
    Ray { center: Point, radii: [f32; 2] },
}
impl Axis {
    fn shading(self, low: f32, high: f32) -> Shading {
        match self {
            Self::Line { start, direction } => Shading::Axial {
                start: [start[0] + direction[0] * low, start[1] + direction[1] * low],
                end: [
                    start[0] + direction[0] * high,
                    start[1] + direction[1] * high,
                ],
            },
            Self::Ray { center, radii } => Shading::Radial {
                center,
                radii: [low, high],
                scale_y: radii[1] / radii[0],
            },
        }
    }
    fn solid(self, color: Color) -> ResolvedGradient {
        ResolvedGradient {
            shading: self.shading(0.0, MIN_RANGE_PT),
            stops: vec![GradientStop { offset: 0.0, color }],
        }
    }
}

/// Largest ray distance from the centre to a tile corner, in horizontal-radius units.
fn farthest(center: Point, [rx, ry]: [f32; 2], [width, height]: [f32; 2]) -> f32 {
    [[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]]
        .iter()
        .map(|corner| (corner[0] - center[0]).hypot((corner[1] - center[1]) * rx / ry))
        .fold(0.0, f32::max)
}

enum Fixed {
    Stop(Placed),
    Hint(f32),
}

/// CSS Images 3 §3.4.3 colour stop fixup; positions are in points along the ray.
fn fixup(items: &[GradientItem], ray: f32) -> Vec<Fixed> {
    let stops = items
        .iter()
        .filter(|item| matches!(item, GradientItem::Stop { .. }))
        .count();
    let mut positions: Vec<Option<f32>> = items
        .iter()
        .map(|item| match item {
            GradientItem::Stop { position, .. } => position.as_ref().map(|p| p.resolve(ray)),
            GradientItem::Hint(position) => Some(position.resolve(ray)),
        })
        .collect();
    let mut stop_index = 0;
    for (item, position) in items.iter().zip(&mut positions) {
        if let GradientItem::Stop { .. } = item {
            if stop_index == 0 && position.is_none() {
                *position = Some(0.0);
            }
            if stop_index == stops - 1 && position.is_none() {
                *position = Some(ray);
            }
            stop_index += 1;
        }
    }
    let mut largest = f32::NEG_INFINITY;
    for position in positions.iter_mut().flatten() {
        *position = position.max(largest);
        largest = *position;
    }
    // Runs of positionless stops are spaced between the neighbouring positioned stops.
    let stop_indices: Vec<usize> = (0..items.len())
        .filter(|&index| matches!(items[index], GradientItem::Stop { .. }))
        .collect();
    let mut run = 0;
    while run < stop_indices.len() {
        if positions[stop_indices[run]].is_some() {
            run += 1;
            continue;
        }
        let before = positions[stop_indices[run - 1]].expect("the first stop has a position");
        let end = (run..stop_indices.len())
            .find(|&candidate| positions[stop_indices[candidate]].is_some())
            .expect("the last stop has a position");
        let after = positions[stop_indices[end]].expect("found a positioned stop");
        let gaps = (end - run + 1) as f32;
        for step in run..end {
            positions[stop_indices[step]] =
                Some(before + (after - before) * (step - run + 1) as f32 / gaps);
        }
        run = end;
    }
    items
        .iter()
        .zip(positions)
        .map(|(item, position)| {
            let position = position.expect("fixup positions every item");
            match item {
                GradientItem::Stop { color, .. } => Fixed::Stop(Placed {
                    position,
                    color: *color,
                }),
                GradientItem::Hint(_) => Fixed::Hint(position),
            }
        })
        .collect()
}

/// Premultiplied sRGB interpolation (CSS Images 3 §3.4.4) at `weight` from `a` to `b`.
fn mix(a: Color, b: Color, weight: f32) -> Color {
    let alpha = a.alpha + (b.alpha - a.alpha) * weight;
    if alpha <= 0.0 {
        return Color {
            rgb: a.rgb,
            alpha: 0.0,
        };
    }
    let channel = |index: usize| {
        let value = (f32::from(a.rgb[index]) * a.alpha * (1.0 - weight)
            + f32::from(b.rgb[index]) * b.alpha * weight)
            / alpha;
        value.round().clamp(0.0, f32::from(u8::MAX)) as u8
    };
    Color {
        rgb: [channel(0), channel(1), channel(2)],
        alpha,
    }
}

/// Interpolation weight at `t` for a hint at fraction `hint` of the segment.
fn hinted(t: f32, hint: f32) -> f32 {
    if hint <= 0.0 {
        1.0
    } else if hint >= 1.0 {
        0.0
    } else {
        t.powf(HALF_WEIGHT.ln() / hint.ln())
    }
}

/// Stops between which non-premultiplied linear interpolation reproduces the CSS colours.
fn dense(fixed: &[Fixed]) -> Vec<Placed> {
    fn push(stop: Placed, output: &mut Vec<Placed>) {
        if output
            .last()
            .is_none_or(|last| last.position != stop.position || last.color != stop.color)
        {
            output.push(stop);
        }
    }
    let mut output: Vec<Placed> = Vec::new();
    let mut previous: Option<Placed> = None;
    let mut hint = None;
    for item in fixed {
        let stop = match item {
            Fixed::Hint(position) => {
                hint = Some(*position);
                continue;
            }
            Fixed::Stop(stop) => *stop,
        };
        let Some(start) = previous.replace(stop) else {
            push(stop, &mut output);
            continue;
        };
        let hint = hint.take();
        let length = stop.position - start.position;
        // A transparent end takes its neighbour's colour, so only its alpha changes.
        let mut from = start;
        let mut to = stop;
        if from.color.alpha <= 0.0 {
            from.color.rgb = to.color.rgb;
        }
        if to.color.alpha <= 0.0 {
            to.color.rgb = from.color.rgb;
        }
        let fraction = hint.map(|position| {
            if length > 0.0 {
                (position - start.position) / length
            } else {
                HALF_WEIGHT
            }
        });
        let constant = from.color == to.color;
        let unhinted = fraction.is_none_or(|fraction| fraction == HALF_WEIGHT);
        // Equal alpha or equal colour keeps premultiplied interpolation linear.
        let linear = constant
            || (unhinted && (from.color.alpha == to.color.alpha || from.color.rgb == to.color.rgb));
        push(from, &mut output);
        if length > 0.0 && !linear {
            for sample in 1..SEGMENT_SAMPLES {
                let t = sample as f32 / SEGMENT_SAMPLES as f32;
                let weight = fraction.map_or(t, |fraction| hinted(t, fraction));
                push(
                    Placed {
                        position: start.position + length * t,
                        color: mix(from.color, to.color, weight),
                    },
                    &mut output,
                );
            }
        }
        push(to, &mut output);
    }
    output
}

/// Colour at `position`; `after` selects the right-hand limit at a hard stop.
fn color_at(stops: &[Placed], position: f32, after: bool) -> Color {
    let index = if after {
        stops.partition_point(|stop| stop.position <= position)
    } else {
        stops.partition_point(|stop| stop.position < position)
    };
    if index == 0 {
        return stops[0].color;
    }
    if index == stops.len() {
        return stops[index - 1].color;
    }
    let (a, b) = (stops[index - 1], stops[index]);
    if after && a.position == position {
        return a.color;
    }
    if !after && b.position == position {
        return b.color;
    }
    let t = (position - a.position) / (b.position - a.position);
    let channel = |index: usize| {
        let value = f32::from(a.color.rgb[index])
            + (f32::from(b.color.rgb[index]) - f32::from(a.color.rgb[index])) * t;
        value.round() as u8
    };
    Color {
        rgb: [channel(0), channel(1), channel(2)],
        alpha: a.color.alpha + (b.color.alpha - a.color.alpha) * t,
    }
}

/// Stops within `low..=high`, with the outer limits kept at the boundaries for padding.
fn clamp(stops: &[Placed], low: f32, high: f32) -> Vec<Placed> {
    let mut output = vec![Placed {
        position: low,
        color: color_at(stops, low, false),
    }];
    output.extend(
        stops
            .iter()
            .filter(|stop| stop.position >= low && stop.position <= high),
    );
    output.push(Placed {
        position: high,
        color: color_at(stops, high, true),
    });
    output.dedup_by(|b, a| a.position == b.position && a.color == b.color);
    output
}

/// Copies of the stop period covering `low..high`; `None` when it would exceed the limit.
fn repeat(stops: &[Placed], low: f32, high: f32) -> Option<Vec<Placed>> {
    let first = stops[0].position;
    let period = stops[stops.len() - 1].position - first;
    if period <= 0.0 {
        return None;
    }
    let start = ((low - first) / period).floor();
    let copies = ((high - first) / period).ceil() - start;
    if !copies.is_finite() || copies * stops.len() as f32 > MAX_REPEATED_STOPS as f32 {
        return None;
    }
    let mut output = Vec::with_capacity(copies as usize * stops.len());
    for copy in 0..copies as usize {
        let shift = (start + copy as f32) * period;
        output.extend(stops.iter().map(|stop| Placed {
            position: stop.position + shift,
            color: stop.color,
        }));
    }
    Some(output)
}

/// Mean premultiplied colour of one period, with coincident stops spread over unit length.
fn average(stops: &[Placed]) -> Color {
    let first = stops[0].position;
    let last = stops[stops.len() - 1].position;
    let spread: Vec<Placed> = if last > first {
        stops.to_vec()
    } else {
        let mut spread = stops.to_vec();
        let end = spread.len() - 1;
        spread[end].position = first + 1.0;
        spread
    };
    let mut sums = [0.0f32; 4];
    for pair in spread.windows(2) {
        let length = pair[1].position - pair[0].position;
        for (index, sum) in sums.iter_mut().take(3).enumerate() {
            *sum += length
                * (f32::from(pair[0].color.rgb[index]) * pair[0].color.alpha
                    + f32::from(pair[1].color.rgb[index]) * pair[1].color.alpha)
                / 2.0;
        }
        sums[3] += length * (pair[0].color.alpha + pair[1].color.alpha) / 2.0;
    }
    let total = spread[spread.len() - 1].position - spread[0].position;
    let alpha = sums[3] / total;
    if alpha <= 0.0 {
        return Color::TRANSPARENT;
    }
    Color {
        rgb: std::array::from_fn(|index| {
            (sums[index] / total / alpha)
                .round()
                .clamp(0.0, f32::from(u8::MAX)) as u8
        }),
        alpha,
    }
}
