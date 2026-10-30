//! Shared computed paint values and image-independent geometry algorithms.
//! Coordinates use points with positive y down; edge arrays use top/right/bottom/left order.
mod background;
mod border_image;
mod boxes;
mod calculation;
mod clip;
mod color_space;
mod decoration;
mod easing;
mod filter;
mod gradient;
mod length;
mod mask;
mod polygon;
mod raster;
mod rounded;
mod shadow;
mod shape;
#[cfg(test)]
pub(crate) use background::rounded_size;
pub(crate) use background::{
    Attachment, BackgroundImage, BackgroundSize, Backgrounds, BoxArea, INITIAL_IMAGE_SCALE,
    ImageFunction, ImageRequest, LayerPaint, Position, PositionComponent, Repeat, SizeComponent,
    TileImage, UnresolvedImage, image_size, natural_points,
};
pub(crate) use border_image::{
    BorderImage, BorderImageOutset, BorderImagePaint, BorderImageRepeat, BorderImageWidth,
    ImageSlice,
};
pub(crate) use boxes::{
    BackgroundGeometry, BoxAreas, BoxDecoration, BoxPaint, DEFAULT_BORDER_WIDTH, LegendCut,
};
pub use boxes::{Border, BorderStyle, INLINE_EDGES, Outline, OutlineStyle};
#[cfg(test)]
pub(crate) use calculation::MAX_CALC_MAGNITUDE;
pub(crate) use calculation::{BinaryFunction, Calculation, RoundingStrategy, UnaryFunction};
pub use clip::{Clip, ClipBounds};
pub(crate) use color_space::{
    AbsoluteColor, ColorSpace, HueInterpolation, Interpolation, MixItem, normalize_hue, quantize,
};
pub use decoration::{DecorationLine, DecorationPaint, DecorationStyle};
pub(crate) use easing::{Easing, LinearPoint, StepPosition};
pub use filter::FilterFunction;
pub(crate) use filter::{
    BLUR_EXTENT, ColorMatrix, IDENTITY_MATRIX, alpha_scale, compose, preserves_unit_range,
    transform as transform_color,
};
pub(crate) use gradient::GradientStop;
pub(crate) use gradient::{
    CircleSize, EllipseSize, Gradient, GradientItem, GradientShape, LineDirection, RadialEnding,
    RadialExtent, ResolvedGradient, Shading,
};
pub(crate) use image::{ImageInfo, NaturalDimensions};
pub use length::LengthPercentage;
pub use mask::{MaskComposite, MaskMode, Masks};
pub(crate) use polygon::clip_polygon;
pub(crate) use rounded::rounded_rect;
pub use rounded::{CornerShape, Radii};
mod image;
pub(crate) use raster::{bounds as shape_bounds, cover, opaque, within};
use serde::Serialize;
pub(crate) use shadow::{BoxShadow, ShadowMask, blurred_shadow, outset, outset_radii};
pub use shadow::{ShadowPaint, ShadowShape};
pub(crate) use shape::{
    AxisRadius, BasicShape, CircleRadius, ClipPath, CornerRadii, EllipseRadii, GeometryBox,
    ShapeBox, ShapeOutside,
};

/// CSS reference pixel in PDF points.
pub(crate) const POINTS_PER_PIXEL: f32 = 0.75;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Color {
    pub rgb: [u8; 3],
    pub alpha: f32,
}
impl Color {
    pub const BLACK: Self = Self {
        rgb: [0; 3],
        alpha: 1.0,
    };
    pub const WHITE: Self = Self {
        rgb: [u8::MAX; 3],
        alpha: 1.0,
    };
    pub const TRANSPARENT: Self = Self {
        rgb: [0; 3],
        alpha: 0.0,
    };
}

/// Compositing 1 §10 `<blend-mode>`: the separable and non-separable modes, each identical to
/// the PDF blend mode of the same name (PDF 32000 §11.3.5), and the Compositing 2 §9.1.14 and
/// §9.1.15 `plus-darker` and `plus-lighter` operators, which no PDF blend mode expresses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    PlusDarker,
    PlusLighter,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self {
            x,
            y,
            width: (self.x + self.width).max(other.x + other.width) - x,
            height: (self.y + self.height).max(other.y + other.height) - y,
        }
    }
    /// The overlap of two rectangles, with zero width or height when they do not overlap.
    #[must_use]
    pub fn intersection(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Self {
            x,
            y,
            width: ((self.x + self.width).min(other.x + other.width) - x).max(0.0),
            height: ((self.y + self.height).min(other.y + other.height) - y).max(0.0),
        }
    }
    pub fn intersects(self, other: Self) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && other.width > 0.0
            && other.height > 0.0
            && self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
}

pub type Point = [f32; 2];

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Segment {
    Line(Point),
    Cubic([Point; 3]),
}
impl Segment {
    pub(crate) fn end(self) -> Point {
        match self {
            Self::Line(point) | Self::Cubic([_, _, point]) => point,
        }
    }
    pub(crate) fn quadratic(from: Point, control: Point, end: Point) -> Self {
        const QUADRATIC_TO_CUBIC: f32 = 2.0 / 3.0;
        let toward = |from: Point| {
            std::array::from_fn(|axis| {
                from[axis] + QUADRATIC_TO_CUBIC * (control[axis] - from[axis])
            })
        };
        Self::Cubic([toward(from), toward(end), end])
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Contour {
    pub start: Point,
    pub segments: Vec<Segment>,
    pub closed: bool,
}
impl Contour {
    pub(crate) fn path_elements(&self) -> impl Iterator<Item = kurbo::PathEl> + Clone + '_ {
        let point = |[x, y]: Point| kurbo::Point::new(f64::from(x), f64::from(y));
        std::iter::once(kurbo::PathEl::MoveTo(point(self.start)))
            .chain(self.segments.iter().map(move |segment| match *segment {
                Segment::Line(end) => kurbo::PathEl::LineTo(point(end)),
                Segment::Cubic([a, b, end]) => {
                    kurbo::PathEl::CurveTo(point(a), point(b), point(end))
                }
            }))
            .chain(self.closed.then_some(kurbo::PathEl::ClosePath))
    }

    /// Polygon vertices approximating the filled contour, without a repeated closing point.
    pub(crate) fn polygon_points(&self, tolerance: f64) -> Vec<[f64; 2]> {
        let mut points = Vec::new();
        kurbo::flatten(self.path_elements(), tolerance, |element| match element {
            kurbo::PathEl::MoveTo(point) | kurbo::PathEl::LineTo(point) => {
                points.push([point.x, point.y]);
            }
            _ => {}
        });
        if points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        points
    }

    pub(crate) fn map(&self, place: impl Fn(Point) -> Point) -> Self {
        Self {
            start: place(self.start),
            closed: self.closed,
            segments: self
                .segments
                .iter()
                .map(|segment| match *segment {
                    Segment::Line(point) => Segment::Line(place(point)),
                    Segment::Cubic(points) => Segment::Cubic(points.map(&place)),
                })
                .collect(),
        }
    }
    pub fn polygon(points: &[Point]) -> Self {
        Self {
            start: points[0],
            segments: points[1..].iter().copied().map(Segment::Line).collect(),
            closed: true,
        }
    }
}

/// Polygon clips use the even-odd rule.
#[derive(Clone, Debug, Serialize)]
pub enum Shape {
    Rect(Rect),
    Clipped {
        shape: Box<Shape>,
        bounds: ClipBounds,
    },
    ContourClipped {
        shape: Box<Shape>,
        contours: Vec<Contour>,
        even_odd: bool,
    },
    Path {
        contours: Vec<Contour>,
        even_odd: bool,
        clip: Option<Vec<Point>>,
    },
    Pattern {
        contours: Vec<Contour>,
        width: f32,
        dash: [f32; 2],
        round: bool,
        clip: Option<Vec<Point>>,
    },
    /// Solid butt-capped stroke along open contours.
    Stroke {
        contours: Vec<Contour>,
        width: f32,
    },
}
#[derive(Clone, Debug, Serialize)]
pub struct Fill {
    pub shape: Shape,
    pub color: Color,
}
