//! CSS Backgrounds 3 §7 box shadows: spread shapes, clip regions and blurred alpha masks.
use crate::paint::rounded::{Radii, inset, rounded_rect};
use crate::paint::{BoxPaint, Color, Contour, CornerShape, Rect};
use serde::Serialize;
use std::sync::Arc;

/// The blur is a Gaussian with a standard deviation of half the blur radius (§7.1.1).
const SIGMA_PER_BLUR: f32 = 0.5;
/// Gaussian tails beyond this many standard deviations hold under 0.2% alpha and are cut.
const BLUR_EXTENT_SIGMAS: f32 = 3.0;
/// Blurred alpha has no detail finer than its standard deviation, so the mask samples it
/// with this many pixels per deviation, at most this density and this many pixels in all.
const MASK_SIGMA_PIXELS: f32 = 6.0;
const MAX_MASK_PIXELS_PER_PT: f32 = 4.0;
const MAX_MASK_PIXELS: f32 = 4.0e6;
/// Vertical coverage samples per mask row; horizontal coverage is exact.
const COVERAGE_SUBROWS: usize = 4;
const ALPHA_MAX: f32 = 255.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BoxShadow {
    pub offset: [f32; 2],
    /// Blur radius; the Gaussian's standard deviation is half of it.
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    pub inset: bool,
}
impl BoxShadow {
    pub(crate) fn sigma(&self) -> f32 {
        self.blur * SIGMA_PER_BLUR
    }
    /// Distance beyond the spread shape that the blurred shadow reaches.
    pub(crate) fn extent(&self) -> f32 {
        self.sigma() * BLUR_EXTENT_SIGMAS
    }
}

/// Alpha of a blurred shadow sampled on `size` pixels covering `rect`, rows top to bottom.
#[derive(Debug, PartialEq)]
pub struct ShadowMask {
    pub rect: Rect,
    pub size: [u32; 2],
    pub alpha: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum ShadowShape {
    /// Unblurred shadow, filled with the even-odd rule.
    Sharp(Vec<Contour>),
    Blurred(Arc<ShadowMask>),
}

/// One shadow ready to paint inside `clip`.
#[derive(Clone, Debug)]
pub struct ShadowPaint {
    pub bounds: Rect,
    /// Area the shadow may cover, filled even-odd: outside the border box for outer shadows,
    /// the padding box for inset ones.
    pub clip: Vec<Contour>,
    pub shape: ShadowShape,
    pub color: Color,
}

/// Radii of a shape grown by `growth` (negative shrinks); a radius smaller than a positive
/// growth grows by `growth × (1 + (r / growth − 1)³)` so that sharp corners stay sharp (§7.1.1).
pub(crate) fn spread_radii(radii: Radii, growth: f32) -> Radii {
    radii.map(|corner| corner.map(|radius| crate::paint::rounded::spread_radius(radius, growth)))
}

/// Radii of a box outset by `distance` (top/right/bottom/left), each radius growing by the
/// distance along its own axis as `spread_radii` grows it; CSS Shapes 1 §5 `margin-box`.
pub(crate) fn outset_radii(radii: Radii, [top, right, bottom, left]: [f32; 4]) -> Radii {
    let growth = [[left, top], [right, top], [right, bottom], [left, bottom]];
    std::array::from_fn(|corner| {
        std::array::from_fn(|axis| {
            crate::paint::rounded::spread_radius(radii[corner][axis], growth[corner][axis])
        })
    })
}

pub(crate) fn outset(rect: Rect, distance: [f32; 4]) -> Rect {
    let [top, right, bottom, left] = distance;
    Rect {
        x: rect.x - left,
        y: rect.y - top,
        width: (rect.width + left + right).max(0.0),
        height: (rect.height + top + bottom).max(0.0),
    }
}
fn is_empty(rect: Rect) -> bool {
    rect.width <= 0.0 || rect.height <= 0.0
}

#[derive(Clone)]
enum ShadowArea {
    Rounded(Rect, Radii),
    Contours(Vec<Contour>),
}

impl ShadowArea {
    fn is_empty(&self) -> bool {
        match self {
            Self::Rounded(rect, _) => is_empty(*rect),
            Self::Contours(contours) => contours.is_empty(),
        }
    }

    fn translated(self, offset: [f32; 2]) -> Self {
        match self {
            Self::Rounded(rect, radii) => Self::Rounded(rect.translated(offset), radii),
            Self::Contours(contours) => Self::Contours(
                contours
                    .into_iter()
                    .map(|contour| contour.map(|[x, y]| [x + offset[0], y + offset[1]]))
                    .collect(),
            ),
        }
    }

    fn contours(self) -> Vec<Contour> {
        match self {
            Self::Rounded(rect, radii) => vec![rounded_rect(rect, radii)],
            Self::Contours(contours) => contours,
        }
    }

    fn coverage(self, region: Rect, size: [usize; 2], pitch: [f32; 2]) -> Vec<f32> {
        let contours = match self {
            Self::Rounded(rect, radii) => return coverage(region, size, pitch, (rect, radii)),
            Self::Contours(contours) => contours,
        };
        let mut target = tiny_skia::Pixmap::new(size[0] as u32, size[1] as u32)
            .expect("shadow grid dimensions fit the pixel budget");
        let bounds = [
            [f64::from(region.x), f64::from(region.x + region.width)],
            [f64::from(region.y), f64::from(region.y + region.height)],
        ];
        let tolerance = f64::from(pitch[0].min(pitch[1])) / COVERAGE_SUBROWS as f64;
        let contours = contours
            .into_iter()
            .filter_map(|contour| {
                let polygon =
                    crate::paint::clip_polygon(&contour.polygon_points(tolerance), bounds);
                (polygon.len() >= 3).then(|| {
                    Contour::polygon(
                        &polygon
                            .into_iter()
                            .map(|point| {
                                std::array::from_fn(|axis| {
                                    ((point[axis] - bounds[axis][0]) / f64::from(pitch[axis]))
                                        as f32
                                })
                            })
                            .collect::<Vec<_>>(),
                    )
                })
            })
            .collect();
        let local = Rect {
            x: 0.0,
            y: 0.0,
            width: size[0] as f32,
            height: size[1] as f32,
        };
        crate::paint::raster::cover(
            &crate::paint::Shape::Path {
                contours,
                even_odd: false,
                clip: None,
            },
            local,
            tiny_skia::Transform::identity(),
            &mut target,
            None,
        );
        target
            .pixels()
            .iter()
            .map(|pixel| f32::from(pixel.alpha()) / ALPHA_MAX)
            .collect()
    }
}

impl<I> BoxPaint<I> {
    /// Outer or inset shadows in painting order, the last listed first (§7.1). A sliced
    /// fragment casts the shadow of its unbroken box, cut at its sliced edges.
    pub fn shadows(&self, inset_shadows: bool) -> Vec<ShadowPaint> {
        let rect = self.decoration_rect();
        if is_empty(rect) {
            return Vec::new();
        }
        let radii = self.radii();
        let mut paints = Vec::new();
        for shadow in self.decoration.shadows.iter().rev() {
            if shadow.inset != inset_shadows || shadow.color.alpha <= 0.0 {
                continue;
            }
            let paint = if shadow.inset {
                self.inset_shadow(shadow, rect, radii)
            } else {
                self.outer_shadow(shadow, rect, radii)
            };
            paints.extend(paint);
        }
        paints
    }

    fn inset_shadow(
        &self,
        shadow: &BoxShadow,
        unbroken: Rect,
        radii: Radii,
    ) -> Option<ShadowPaint> {
        let (unbroken_padding, padding_radii) = inset(unbroken, radii, self.inner_widths());
        let bounds = self.slice_bounds(unbroken_padding);
        if is_empty(bounds) {
            return None;
        }
        let clip = self.inner_clip([0.0; 4], [true; 2]).contours().into_owned();
        if clip.is_empty() {
            return None;
        }
        let hole = if *self.decoration.corner_shapes == [CornerShape::ROUND; 4] {
            ShadowArea::Rounded(
                outset(unbroken_padding, [-shadow.spread; 4]),
                spread_radii(padding_radii, -shadow.spread),
            )
        } else {
            ShadowArea::Contours(crate::paint::rounded::box_contours(
                unbroken,
                radii,
                self.decoration.corner_shapes.map(CornerShape::curvature),
                self.inner_widths().map(|width| width + shadow.spread),
            ))
        }
        .translated(shadow.offset);
        let hole = (!hole.is_empty()).then_some(hole);
        let shape = if shadow.blur > 0.0 {
            let region = outset(unbroken_padding, [shadow.extent(); 4]);
            ShadowShape::Blurred(Arc::new(blurred_mask(region, hole, shadow.sigma(), true)))
        } else {
            let mut contours = vec![Contour::polygon(&corners(unbroken_padding))];
            contours.extend(hole.into_iter().flat_map(ShadowArea::contours));
            ShadowShape::Sharp(contours)
        };
        Some(ShadowPaint {
            bounds,
            clip,
            shape,
            color: shadow.color,
        })
    }
    fn outer_shadow(&self, shadow: &BoxShadow, rect: Rect, radii: Radii) -> Option<ShadowPaint> {
        let shape = outset(rect, [shadow.spread; 4]).translated(shadow.offset);
        if is_empty(shape) {
            return None;
        }
        let area = if *self.decoration.corner_shapes == [CornerShape::ROUND; 4] {
            ShadowArea::Rounded(
                outset(rect, [shadow.spread; 4]),
                spread_radii(radii, shadow.spread),
            )
        } else {
            ShadowArea::Contours(crate::paint::rounded::box_contours(
                rect,
                radii,
                self.decoration.corner_shapes.map(CornerShape::curvature),
                [-shadow.spread; 4],
            ))
        }
        .translated(shadow.offset);
        if area.is_empty() {
            return None;
        }
        let ink = outset(shape, [shadow.extent(); 4]);
        let visible = self.slice_bounds(ink);
        if is_empty(visible) {
            return None;
        }
        let bounds = ink.union(rect);
        let shape = if shadow.blur > 0.0 {
            ShadowShape::Blurred(Arc::new(blurred_mask(
                ink,
                Some(area),
                shadow.sigma(),
                false,
            )))
        } else {
            ShadowShape::Sharp(area.contours())
        };
        let mut clip = vec![Contour::polygon(&corners(bounds))];
        clip.extend(
            self.box_area(crate::paint::BoxArea::BorderBox)
                .contours()
                .iter()
                .cloned(),
        );
        Some(ShadowPaint {
            bounds: self.slice_bounds(bounds),
            clip,
            shape,
            color: shadow.color,
        })
    }
}

fn corners(rect: Rect) -> [crate::paint::Point; 4] {
    [
        [rect.x, rect.y],
        [rect.x + rect.width, rect.y],
        [rect.x + rect.width, rect.y + rect.height],
        [rect.x, rect.y + rect.height],
    ]
}

/// Horizontal extent of a rounded rectangle at height `y`.
fn row_span(rect: Rect, radii: Radii, y: f32) -> Option<[f32; 2]> {
    let below_top = y - rect.y;
    let above_bottom = rect.y + rect.height - y;
    if below_top < 0.0 || above_bottom < 0.0 {
        return None;
    }
    // Horizontal inset of an elliptical corner `depth` from its straight horizontal edge.
    let corner = |[rx, ry]: [f32; 2], depth: f32| {
        if rx <= 0.0 || ry <= 0.0 || depth >= ry {
            return 0.0;
        }
        let along = (ry - depth) / ry;
        rx * (1.0 - (1.0 - along * along).max(0.0).sqrt())
    };
    let left = corner(radii[0], below_top).max(corner(radii[3], above_bottom));
    let right = corner(radii[1], below_top).max(corner(radii[2], above_bottom));
    let span = [rect.x + left, rect.x + rect.width - right];
    (span[0] < span[1]).then_some(span)
}

/// Area coverage of `shape` per pixel, exact along rows and sampled down columns.
fn coverage(region: Rect, size: [usize; 2], pitch: [f32; 2], shape: (Rect, Radii)) -> Vec<f32> {
    let [columns, rows] = size;
    let mut values = vec![0.0; columns * rows];
    let weight = 1.0 / COVERAGE_SUBROWS as f32;
    for row in 0..rows {
        for sample in 0..COVERAGE_SUBROWS {
            let y = region.y + (row as f32 + (sample as f32 + 0.5) * weight) * pitch[1];
            let Some([x0, x1]) = row_span(shape.0, shape.1, y) else {
                continue;
            };
            let from = ((x0 - region.x) / pitch[0]).max(0.0);
            let to = ((x1 - region.x) / pitch[0]).min(columns as f32);
            if from >= to {
                continue;
            }
            let line = &mut values[row * columns..(row + 1) * columns];
            for (column, value) in line
                .iter_mut()
                .enumerate()
                .take(to.ceil() as usize)
                .skip(from.floor() as usize)
            {
                let column = column as f32;
                *value += (to.min(column + 1.0) - from.max(column)) * weight;
            }
        }
    }
    values
}

/// Normalised Gaussian weights for offsets `-radius..=radius` pixels.
fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let radius = (sigma * BLUR_EXTENT_SIGMAS).ceil() as i32;
    let weights: Vec<f32> = (-radius..=radius)
        .map(|offset| (-(offset * offset) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    weights.into_iter().map(|weight| weight / total).collect()
}

/// Convolves `values` along one axis; samples beyond the grid are zero.
fn convolve(values: &[f32], size: [usize; 2], kernel: &[f32], vertical: bool) -> Vec<f32> {
    let [columns, rows] = size;
    let radius = (kernel.len() / 2) as isize;
    let (length, lines) = if vertical {
        (rows, columns)
    } else {
        (columns, rows)
    };
    let index = |line: usize, at: usize| {
        if vertical {
            at * columns + line
        } else {
            line * columns + at
        }
    };
    let mut output = vec![0.0; values.len()];
    for line in 0..lines {
        for at in 0..length {
            let mut sum = 0.0;
            for (tap, weight) in kernel.iter().enumerate() {
                let source = at as isize + tap as isize - radius;
                if (0..length as isize).contains(&source) {
                    sum += weight * values[index(line, source as usize)];
                }
            }
            output[index(line, at)] = sum;
        }
    }
    output
}

/// The shape's coverage over `region` blurred by a Gaussian of standard deviation `sigma`;
/// `invert` gives the blurred complement, the shadow around an inset shadow's hole.
fn blurred_mask(region: Rect, shape: Option<ShadowArea>, sigma: f32, invert: bool) -> ShadowMask {
    blurred(region, sigma, invert, |size, pitch| {
        shape.map_or_else(
            || vec![0.0; size[0] * size[1]],
            |shape| shape.coverage(region, size, pitch),
        )
    })
}

/// Pixels sampling `region` for a blur of standard deviation `sigma`, and their pitch.
fn mask_grid(region: Rect, sigma: f32) -> ([usize; 2], [f32; 2]) {
    let mut density = (MASK_SIGMA_PIXELS / sigma).min(MAX_MASK_PIXELS_PER_PT);
    let pixels = region.width * region.height * density * density;
    if pixels > MAX_MASK_PIXELS {
        density = if pixels.is_finite() {
            density * (MAX_MASK_PIXELS / pixels).sqrt()
        } else {
            (f64::from(MAX_MASK_PIXELS) / (f64::from(region.width) * f64::from(region.height)))
                .sqrt() as f32
        };
    }
    let limit = MAX_MASK_PIXELS as usize;
    let mut size = [region.width, region.height]
        .map(|extent| ((extent * density).ceil() as usize).clamp(1, limit));
    // Rounding a thin axis up to one pixel must not undo the area budget.
    if size[0] > limit / size[1] {
        let larger = usize::from(size[1] > size[0]);
        size[larger] = limit / size[1 - larger];
    }
    let pitch = [
        region.width / size[0] as f32,
        region.height / size[1] as f32,
    ];
    (size, pitch)
}

/// A blurred text shadow over `region`: `coverage` gives the alpha, `0..=1`, of the
/// unblurred shadow on a grid of `size` pixels whose pitch is in points, rows top to bottom.
pub(crate) fn blurred_shadow(
    region: Rect,
    sigma: f32,
    coverage: impl FnOnce([usize; 2], [f32; 2]) -> Vec<f32>,
) -> ShadowMask {
    blurred(region, sigma, false, coverage)
}

fn blurred(
    region: Rect,
    sigma: f32,
    invert: bool,
    coverage: impl FnOnce([usize; 2], [f32; 2]) -> Vec<f32>,
) -> ShadowMask {
    let (size, pitch) = mask_grid(region, sigma);
    let mut values = coverage(size, pitch);
    for (axis, vertical) in [(0, false), (1, true)] {
        values = convolve(
            &values,
            size,
            &gaussian_kernel(sigma / pitch[axis]),
            vertical,
        );
    }
    ShadowMask {
        rect: region,
        size: size.map(|extent| extent as u32),
        alpha: values
            .into_iter()
            .map(|value| {
                let value = value.clamp(0.0, 1.0);
                ((if invert { 1.0 - value } else { value }) * ALPHA_MAX).round() as u8
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standard normal cumulative distribution by Abramowitz–Stegun 7.1.26 (error < 1.5e-7).
    fn phi(x: f32) -> f32 {
        let z = x.abs() / std::f32::consts::SQRT_2;
        let t = 1.0 / (1.0 + 0.327_591_1 * z);
        let poly = t
            * (0.254_829_6
                + t * (-0.284_496_7 + t * (1.421_413_8 + t * (-1.453_152 + t * 1.061_405_4))));
        let erf = 1.0 - poly * (-(z * z)).exp();
        f32::midpoint(1.0, erf.copysign(x))
    }

    /// Alpha of the pixel containing `point`, and that pixel's centre.
    fn pixel_at(mask: &ShadowMask, point: [f32; 2]) -> (f32, [f32; 2]) {
        let pitch = [
            mask.rect.width / mask.size[0] as f32,
            mask.rect.height / mask.size[1] as f32,
        ];
        let column = ((point[0] - mask.rect.x) / pitch[0]) as usize;
        let row = ((point[1] - mask.rect.y) / pitch[1]) as usize;
        (
            f32::from(mask.alpha[row * mask.size[0] as usize + column]) / ALPHA_MAX,
            [
                mask.rect.x + (column as f32 + 0.5) * pitch[0],
                mask.rect.y + (row as f32 + 0.5) * pitch[1],
            ],
        )
    }

    fn notched_shadow(inset: bool, blur: f32, spread: f32) -> BoxPaint {
        use crate::paint::{Border, BorderStyle, BoxDecoration, CornerShape, LengthPercentage};
        BoxPaint::whole(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 80.0,
            },
            BoxDecoration {
                radii: Arc::new(std::array::from_fn(|corner| {
                    std::array::from_fn(|_| {
                        LengthPercentage::Length(if corner == 0 { 30.0 } else { 0.0 })
                    })
                })),
                corner_shapes: Arc::new([
                    CornerShape::NOTCH,
                    CornerShape::ROUND,
                    CornerShape::ROUND,
                    CornerShape::ROUND,
                ]),
                borders: [4.0, 8.0, 12.0, 16.0].map(|width| Border {
                    width,
                    style: BorderStyle::Solid,
                    color: Color::BLACK,
                }),
                shadows: Arc::new(vec![BoxShadow {
                    offset: [0.0; 2],
                    blur,
                    spread,
                    color: Color::BLACK,
                    inset,
                }]),
                ..Default::default()
            },
        )
    }

    fn inside(contours: &[Contour], point: [f64; 2]) -> bool {
        use kurbo::Shape as _;
        contours
            .iter()
            .map(|c| {
                kurbo::BezPath::from_iter(c.path_elements())
                    .winding(kurbo::Point::new(point[0], point[1]))
            })
            .sum::<i32>()
            .abs()
            % 2
            == 1
    }

    #[test]
    fn sharp_notch_shadows_keep_the_cutout_and_total_inset_distances() {
        let paint = notched_shadow(false, 0.0, 0.0);
        let outer = paint.shadows(false).pop().unwrap();
        let ShadowShape::Sharp(contours) = &outer.shape else {
            panic!("sharp shadow")
        };
        assert!(
            !inside(contours, [15.0, 15.0]),
            "outer shadow retains the notch cutout"
        );
        assert!(inside(contours, [35.0, 15.0]));
        assert!(
            inside(&outer.clip, [15.0, 15.0]),
            "shadow is allowed in the box's missing corner"
        );
        assert!(
            !inside(&outer.clip, [35.0, 15.0]),
            "shadow must not cover the box itself"
        );
        let mut paint = notched_shadow(true, 0.0, 3.0);
        Arc::make_mut(&mut paint.decoration.shadows)[0].offset = [5.0, -2.0];
        let inner = paint.shadows(true).pop().unwrap();
        assert!(
            !inside(&inner.clip, [32.0, 20.0]),
            "padding notch ends at x34: the top border offsets its vertical segment"
        );
        assert!(inside(&inner.clip, [36.0, 20.0]));
        let ShadowShape::Sharp(contours) = &inner.shape else {
            panic!("sharp shadow")
        };
        assert!(
            inside(contours, [40.0, 20.0]),
            "hole notch ends at 30+4+3+5=42"
        );
        assert!(
            !inside(contours, [44.0, 20.0]),
            "the translated inset hole is transparent"
        );
    }

    #[test]
    fn blurred_notch_alpha_matches_the_difference_of_two_rectangle_gaussians() {
        let paint = notched_shadow(false, 4.0, 0.0);
        let shadow = paint.shadows(false).pop().unwrap();
        let ShadowShape::Blurred(mask) = &shadow.shape else {
            panic!("blurred shadow")
        };
        let sigma = 2.0;
        for point in [
            [15.0, 15.0],
            [28.0, 28.0],
            [30.0, 30.0],
            [32.0, 30.0],
            [35.0, 15.0],
            [0.0, 40.0],
        ] {
            let (actual, [x, y]) = pixel_at(mask, point);
            let outer = (phi(x / sigma) - phi((x - 100.0) / sigma))
                * (phi(y / sigma) - phi((y - 80.0) / sigma));
            let cutout = (phi(x / sigma) - phi((x - 30.0) / sigma))
                * (phi(y / sigma) - phi((y - 30.0) / sigma));
            assert!(
                (actual - (outer - cutout)).abs() < 0.015,
                "notch blur at {point:?}: {actual}, expected {}",
                outer - cutout
            );
        }
    }

    #[test]
    fn outer_notch_spread_and_offset_move_the_shadow_before_box_knockout() {
        for spread in [-4.0, 4.0] {
            let mut paint = notched_shadow(false, 0.0, spread);
            Arc::make_mut(&mut paint.decoration.shadows)[0].offset = [7.0, -3.0];
            let shadow = paint.shadows(false).pop().unwrap();
            let ShadowShape::Sharp(contours) = &shadow.shape else {
                panic!("sharp shadow")
            };
            let vertex = [30.0 - spread + 7.0, 30.0 - spread - 3.0].map(f64::from);
            assert!(
                !inside(contours, [vertex[0] - 1.0, 15.0]),
                "spread {spread}: translated vertical notch edge excludes its cutout"
            );
            assert!(inside(contours, [vertex[0] + 1.0, 15.0]));
            assert!(!inside(contours, [20.0, vertex[1] - 1.0]));
            assert!(inside(contours, [20.0, vertex[1] + 1.0]));
            assert!(
                inside(&shadow.clip, [15.0, 15.0]),
                "knockout remains at the original notch, independent of spread and offset"
            );
            assert!(!inside(&shadow.clip, [50.0, 50.0]));
        }
    }

    #[test]
    fn blurred_inset_notch_uses_the_complement_of_the_translated_hole() {
        let mut paint = notched_shadow(true, 4.0, 3.0);
        Arc::make_mut(&mut paint.decoration.shadows)[0].offset = [5.0, -2.0];
        let shadow = paint.shadows(true).pop().unwrap();
        let ShadowShape::Blurred(mask) = &shadow.shape else {
            panic!("blurred shadow")
        };
        let sigma = 2.0;
        for point in [
            [40.0, 20.0],
            [42.0, 20.0],
            [44.0, 20.0],
            [42.0, 47.0],
            [50.0, 47.0],
        ] {
            let (actual, [x, y]) = pixel_at(mask, point);
            // Hole bounds [19,89]x[7,65] translated by [5,-2], notch vertex [42,47].
            let outer = (phi((x - 24.0) / sigma) - phi((x - 94.0) / sigma))
                * (phi((y - 5.0) / sigma) - phi((y - 63.0) / sigma));
            let cutout = (phi((x - 24.0) / sigma) - phi((x - 42.0) / sigma))
                * (phi((y - 5.0) / sigma) - phi((y - 47.0) / sigma));
            assert!(
                (actual - (1.0 - outer + cutout)).abs() < 0.015,
                "inset notch at {point:?}: {actual}, expected {}",
                1.0 - outer + cutout
            );
        }
    }

    #[test]
    fn disconnected_shadow_islands_blur_independently_and_slices_keep_the_unbroken_mask() {
        use crate::paint::LengthPercentage;
        let mut paint = notched_shadow(false, 4.0, -4.0);
        paint.decoration.corner_shapes = Arc::new([
            CornerShape::NOTCH,
            CornerShape::ROUND,
            CornerShape::NOTCH,
            CornerShape::ROUND,
        ]);
        paint.decoration.radii = Arc::new(std::array::from_fn(|corner| {
            std::array::from_fn(|_| {
                if corner % 2 == 0 {
                    LengthPercentage::Percentage(0.5)
                } else {
                    LengthPercentage::ZERO
                }
            })
        }));
        let whole = paint.shadows(false).pop().unwrap();
        let ShadowShape::Blurred(mask) = &whole.shape else {
            panic!("blurred shadow")
        };
        let sigma = 2.0;
        for point in [
            [50.0, 40.0],
            [54.0, 30.0],
            [40.0, 44.0],
            [60.0, 20.0],
            [30.0, 50.0],
        ] {
            let (actual, [x, y]) = pixel_at(mask, point);
            let top_right = (phi((x - 54.0) / sigma) - phi((x - 96.0) / sigma))
                * (phi((y - 4.0) / sigma) - phi((y - 36.0) / sigma));
            let bottom_left = (phi((x - 4.0) / sigma) - phi((x - 46.0) / sigma))
                * (phi((y - 44.0) / sigma) - phi((y - 76.0) / sigma));
            assert!(
                (actual - top_right - bottom_left).abs() < 0.015,
                "disconnected shadow at {point:?}: {actual}, expected {}",
                top_right + bottom_left
            );
        }
        paint.background_box = Some(paint.rect);
        paint.rect.y = 10.0;
        paint.rect.height = 60.0;
        paint.first_fragment = false;
        paint.last_fragment = false;
        let sliced = paint.shadows(false).pop().unwrap();
        let ShadowShape::Blurred(sliced_mask) = &sliced.shape else {
            panic!("blurred shadow")
        };
        assert_eq!(
            sliced_mask, mask,
            "slicing preserves the complete unbroken Gaussian mask"
        );
        assert_eq!(
            [sliced.bounds.y, sliced.bounds.height],
            [10.0, 60.0],
            "only the visible fragment bounds crop the shadow"
        );
    }

    #[test]
    fn empty_concave_padding_produces_no_inset_shadow() {
        let mut paint = notched_shadow(true, 4.0, 0.0);
        paint.decoration.corner_shapes = Arc::new([CornerShape::NOTCH; 4]);
        paint.decoration.radii = Arc::new(std::array::from_fn(|_| {
            std::array::from_fn(|_| crate::paint::LengthPercentage::Percentage(0.5))
        }));
        assert!(
            paint.shadows(true).is_empty(),
            "four half-box notches consume the whole padding region"
        );
    }

    #[test]
    fn spread_sharpens_radii_smaller_than_the_spread() {
        let grown = spread_radii([[10.0, 30.0], [0.0, 0.0], [20.0, 20.0], [5.0, 40.0]], 20.0);
        assert_eq!(
            grown,
            [[27.5, 50.0], [0.0, 0.0], [40.0, 40.0], [16.5625, 60.0]],
            "specification example r = 10, spread = 20 gives 27.5; r = 5 grows by \
             20 × (1 + (0.25 − 1)³); zero stays sharp; r ≥ spread adds the spread"
        );
        assert_eq!(
            spread_radii([[10.0, 3.0]; 4], -4.0),
            [[6.0, 0.0]; 4],
            "a negative spread subtracts and floors at zero"
        );
    }

    #[test]
    fn blurred_edges_follow_the_gaussian_of_half_the_blur_radius() {
        let sigma = 8.0 * SIGMA_PER_BLUR;
        let shape = Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 60.0,
        };
        let region = outset(shape, [sigma * BLUR_EXTENT_SIGMAS; 4]);
        let square = Some(ShadowArea::Rounded(shape, [[0.0; 2]; 4]));
        let mask = blurred_mask(region, square.clone(), sigma, false);
        let inverted = blurred_mask(region, square, sigma, true);
        for distance in [-2.0 * sigma, -sigma, 0.0, sigma, 2.0 * sigma] {
            // Along the middle of the left edge, far from the corners; inside is positive.
            let point = [shape.x + distance, shape.y + shape.height / 2.0];
            let (actual, centre) = pixel_at(&mask, point);
            let expected = phi((centre[0] - shape.x) / sigma);
            assert!(
                (actual - expected).abs() < 0.01,
                "pixel centred {}pt inside the edge: alpha {actual}, Gaussian {expected}",
                centre[0] - shape.x
            );
            let (complement, _) = pixel_at(&inverted, point);
            assert!(
                (complement - (1.0 - expected)).abs() < 0.01,
                "the same pixel around an inset hole: alpha {complement}, expected {}",
                1.0 - expected
            );
        }
        let (corner, centre) = pixel_at(&mask, [shape.x, shape.y]);
        let expected = phi((centre[0] - shape.x) / sigma) * phi((centre[1] - shape.y) / sigma);
        assert!(
            (corner - expected).abs() < 0.01,
            "a square corner is the product of the two edge profiles: {corner}, expected {expected}"
        );
    }

    #[test]
    fn mask_grid_keeps_extreme_aspect_ratios_within_the_allocation_budget() {
        for [width, height] in [[1.0e12, 1.0], [1.0, 1.0e12], [1.0e30, 1.0e30]] {
            let (size, pitch) = mask_grid(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height,
                },
                0.5,
            );
            assert!(
                size[0] as f64 * size[1] as f64 <= f64::from(MAX_MASK_PIXELS),
                "{width}x{height}: rounded grid {size:?} exceeds the shadow pixel budget"
            );
            assert!(
                pitch.iter().all(|value| value.is_finite() && *value > 0.0),
                "finite positive region needs finite positive pitch: {pitch:?}"
            );
            if width == height {
                let expected = MAX_MASK_PIXELS.sqrt() as usize;
                assert!(
                    size.iter().all(|&extent| extent >= expected - 1),
                    "overflow in physical area must not collapse a large square to a single pixel: {size:?}"
                );
            }
        }
    }

    #[test]
    fn mask_density_tracks_the_deviation_within_limits() {
        let region = Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 20.0,
        };
        assert_eq!(
            blurred_mask(region, None, 0.5, false).size,
            [200, 80],
            "small blurs sample at the {MAX_MASK_PIXELS_PER_PT} px/pt cap"
        );
        assert_eq!(
            blurred_mask(region, None, 12.0, false).size,
            [25, 10],
            "a 12pt deviation needs only {MASK_SIGMA_PIXELS} px per deviation"
        );
        let huge = Rect {
            width: 10_000.0,
            height: 10_000.0,
            ..region
        };
        let capped = blurred_mask(huge, None, 0.5, false);
        let pixels = capped.size[0] as f32 * capped.size[1] as f32;
        assert!(
            pixels <= MAX_MASK_PIXELS * 1.01,
            "pixel budget exceeded: {:?} has {pixels} pixels",
            capped.size
        );
    }

    #[test]
    fn rounded_rows_span_the_elliptical_corners() {
        let rect = Rect {
            x: 10.0,
            y: 0.0,
            width: 100.0,
            height: 40.0,
        };
        let radii = [[20.0, 10.0], [0.0, 0.0], [0.0, 0.0], [0.0, 0.0]];
        let top = row_span(rect, radii, 0.0).unwrap();
        assert!(
            (top[0] - 30.0).abs() < 1e-4 && top[1] == 110.0,
            "the top row starts where the corner arc meets the top edge: {top:?}"
        );
        let middle = row_span(rect, radii, 5.0).unwrap();
        let expected = 10.0 + 20.0 * (1.0 - 0.75_f32.sqrt());
        assert!(
            (middle[0] - expected).abs() < 1e-4,
            "half-way down the corner: {middle:?}, expected {expected}"
        );
        assert_eq!(row_span(rect, radii, 41.0), None, "below the box");
    }
}
