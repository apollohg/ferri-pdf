//! Materialized image paint values and ordered display lists.
use crate::images::Image;
pub(crate) use crate::paint::{
    BackgroundGeometry, BoxArea, BoxAreas, NaturalDimensions, POINTS_PER_PIXEL, ResolvedGradient,
    Shading, ShadowMask, UnresolvedImage, image_size,
};
pub use crate::paint::{
    BlendMode, Border, BorderStyle, Clip, ClipBounds, Color, Contour, DecorationLine,
    DecorationPaint, DecorationStyle, Fill, INLINE_EDGES, LengthPercentage, Outline, OutlineStyle,
    Point, Radii, Rect, Segment, ShadowPaint, ShadowShape, Shape,
};
use serde::Serialize;
use std::sync::Arc;
pub(crate) type BackgroundImage = crate::paint::BackgroundImage<Arc<Image>>;
pub(crate) type Backgrounds = crate::paint::Backgrounds<Arc<Image>>;
pub(crate) type BorderImagePaint<'a> = crate::paint::BorderImagePaint<'a, Arc<Image>>;
pub type BoxDecoration = crate::paint::BoxDecoration<Arc<Image>>;
pub type BoxPaint = crate::paint::BoxPaint<Arc<Image>>;
pub(crate) type LayerPaint<'a> = crate::paint::LayerPaint<'a, Arc<Image>>;
pub(crate) type TileImage<'a> = crate::paint::TileImage<'a, Arc<Image>>;

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
    /// Composites later items as one isolated group blended with this mode until the matching
    /// `Pop`; `Normal` only isolates them.
    PushBlend(BlendMode),
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
