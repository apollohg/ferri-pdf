//! Shared computed paint values and image-independent geometry algorithms.
//! Coordinates use points with positive y down; edge arrays use top/right/bottom/left order.
mod background;
mod border_image;
mod boxes;
mod calculation;
mod clip;
mod decoration;
mod easing;
mod gradient;
mod length;
mod rounded;
mod shadow;
pub use background::{
    Attachment, BackgroundImage, BackgroundSize, Backgrounds, BoxArea, LayerPaint, Position,
    PositionComponent, Repeat, SizeComponent, TileImage, UnresolvedImage, image_size, rounded_size,
};
pub use border_image::{
    BorderImage, BorderImageOutset, BorderImagePaint, BorderImageRepeat, BorderImageWidth,
    ImagePiece, ImageSlice,
};
pub use boxes::{
    BackgroundGeometry, Border, BorderStyle, BoxAreas, BoxDecoration, BoxPaint,
    DEFAULT_BORDER_WIDTH, INLINE_EDGES,
};
pub use calculation::{
    BinaryFunction, Calculation, MAX_CALC_MAGNITUDE, RoundingStrategy, UnaryFunction,
};
pub use clip::{Clip, ClipBounds};
pub use decoration::{DecorationLine, DecorationPaint, DecorationStyle};
pub use easing::{Easing, LinearPoint, StepPosition};
pub use gradient::{
    CircleSize, EllipseSize, Gradient, GradientItem, GradientShape, GradientStop, LineDirection,
    RadialEnding, RadialExtent, ResolvedGradient, Shading,
};
pub use image::{ImageInfo, NaturalDimensions};
pub use length::LengthPercentage;
pub use rounded::Radii;
mod image;
use serde::Serialize;
pub use shadow::{BoxShadow, ShadowMask, ShadowPaint, ShadowShape};

/// CSS reference pixel in PDF points.
pub const POINTS_PER_PIXEL: f32 = 0.75;

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
    pub const TRANSPARENT: Self = Self {
        rgb: [0; 3],
        alpha: 0.0,
    };
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
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Contour {
    pub start: Point,
    pub segments: Vec<Segment>,
    pub closed: bool,
}
impl Contour {
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
