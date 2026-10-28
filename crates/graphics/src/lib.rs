//! Materialized image paint values and ordered display lists.
use ferri_pdf_images::Image;
pub use ferri_pdf_paint::{
    Attachment, BackgroundGeometry, BackgroundSize, BinaryFunction, Border, BorderImageOutset,
    BorderImageRepeat, BorderImageWidth, BorderStyle, BoxArea, BoxAreas, BoxShadow, Calculation,
    CircleSize, Clip, ClipBounds, Color, Contour, DEFAULT_BORDER_WIDTH, DecorationLine,
    DecorationPaint, DecorationStyle, EllipseSize, Fill, Gradient, GradientItem, GradientShape,
    GradientStop, INLINE_EDGES, ImagePiece, ImageSlice, LengthPercentage, LineDirection,
    MAX_CALC_MAGNITUDE, NaturalDimensions, POINTS_PER_PIXEL, Point, Position, PositionComponent,
    RadialEnding, RadialExtent, Radii, Rect, Repeat, ResolvedGradient, RoundingStrategy, Segment,
    Shading, ShadowMask, ShadowPaint, ShadowShape, Shape, SizeComponent, UnaryFunction,
    UnresolvedImage, image_size, rounded_size,
};
use serde::Serialize;
use std::sync::Arc;
pub type BackgroundImage = ferri_pdf_paint::BackgroundImage<Arc<Image>>;
pub type Backgrounds = ferri_pdf_paint::Backgrounds<Arc<Image>>;
pub type BorderImage = ferri_pdf_paint::BorderImage<Arc<Image>>;
pub type BorderImagePaint<'a> = ferri_pdf_paint::BorderImagePaint<'a, Arc<Image>>;
pub type BoxDecoration = ferri_pdf_paint::BoxDecoration<Arc<Image>>;
pub type BoxPaint = ferri_pdf_paint::BoxPaint<Arc<Image>>;
pub type LayerPaint<'a> = ferri_pdf_paint::LayerPaint<'a, Arc<Image>>;
pub type TileImage<'a> = ferri_pdf_paint::TileImage<'a, Arc<Image>>;

#[derive(Clone, Debug, Serialize)]
pub enum DisplayItem {
    Image(ImagePaint),
    Box(BoxPaint),
    Text {
        line: usize,
    },
    Decoration(DecorationPaint),
    /// Clips later items until the matching `Pop`.
    PushClip(Clip),
    /// Transforms later items by affine `[a, b, c, d, e, f]` until the matching `Pop`.
    PushTransform([f32; 6]),
    /// Composites later items as one isolated group with this constant alpha until the
    /// matching `Pop`.
    PushOpacity(f32),
    Pop,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImagePaint {
    #[serde(skip)]
    pub image: std::sync::Arc<ferri_pdf_images::Image>,
    pub rect: Rect,
    pub clip: Option<Clip>,
}
impl ImagePaint {
    pub fn visible_rect(&self) -> Rect {
        self.clip.map_or(self.rect, |clip| clip.rect)
    }
}
