//! Materialized image paint values and ordered display lists.
use ferri_pdf_images::Image;
pub(crate) use ferri_pdf_paint::{
    BackgroundGeometry, BoxArea, BoxAreas, NaturalDimensions, POINTS_PER_PIXEL, ResolvedGradient,
    Shading, ShadowMask, UnresolvedImage, image_size,
};
pub use ferri_pdf_paint::{
    Border, BorderStyle, Clip, ClipBounds, Color, Contour, DecorationLine, DecorationPaint,
    DecorationStyle, Fill, INLINE_EDGES, LengthPercentage, Point, Radii, Rect, Segment,
    ShadowPaint, ShadowShape, Shape,
};
use serde::Serialize;
use std::sync::Arc;
pub(crate) type BackgroundImage = ferri_pdf_paint::BackgroundImage<Arc<Image>>;
pub(crate) type Backgrounds = ferri_pdf_paint::Backgrounds<Arc<Image>>;
pub(crate) type BorderImagePaint<'a> = ferri_pdf_paint::BorderImagePaint<'a, Arc<Image>>;
pub type BoxDecoration = ferri_pdf_paint::BoxDecoration<Arc<Image>>;
pub type BoxPaint = ferri_pdf_paint::BoxPaint<Arc<Image>>;
pub(crate) type LayerPaint<'a> = ferri_pdf_paint::LayerPaint<'a, Arc<Image>>;
pub(crate) type TileImage<'a> = ferri_pdf_paint::TileImage<'a, Arc<Image>>;

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
    pub image: Arc<Image>,
    pub rect: Rect,
    pub clip: Option<Clip>,
}
impl ImagePaint {
    pub fn visible_rect(&self) -> Rect {
        self.clip.map_or(self.rect, |clip| clip.rect)
    }
}
