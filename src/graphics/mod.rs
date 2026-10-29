//! Materialized image paint values and ordered display lists.
use crate::images::Image;
pub(crate) use crate::paint::{
    BackgroundGeometry, BoxArea, BoxAreas, LegendCut, NaturalDimensions, POINTS_PER_PIXEL,
    ResolvedGradient, Shading, ShadowMask, UnresolvedImage, image_size,
};
pub use crate::paint::{
    BlendMode, Border, BorderStyle, Clip, ClipBounds, Color, Contour, DecorationLine,
    DecorationPaint, DecorationStyle, Fill, FilterFunction, INLINE_EDGES, LengthPercentage,
    MaskComposite, MaskMode, Masks, Outline, OutlineStyle, Point, Radii, Rect, Segment,
    ShadowPaint, ShadowShape, Shape,
};
use serde::Serialize;
use std::sync::Arc;
pub(crate) type BackgroundImage = crate::paint::BackgroundImage<Arc<Image>>;
pub(crate) type Backgrounds = crate::paint::Backgrounds<Arc<Image>>;
pub(crate) type BorderImagePaint<'a> = crate::paint::BorderImagePaint<'a, Arc<Image>>;
pub type BoxDecoration = crate::paint::BoxDecoration<Arc<Image>>;
pub type BoxPaint = crate::paint::BoxPaint<Arc<Image>>;
pub(crate) type LayerPaint<'a> = crate::paint::LayerPaint<'a, Arc<Image>>;
pub(crate) type ShapeOutside = crate::paint::ShapeOutside<Arc<Image>>;
pub(crate) type TileImage<'a> = crate::paint::TileImage<'a, Arc<Image>>;

#[derive(Clone, Debug, Serialize)]
pub enum DisplayItem {
    Image(ImagePaint),
    Box(BoxPaint),
    Text {
        line: usize,
    },
    Decoration(DecorationPaint),
    TextShadow(TextShadowPaint),
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
    /// Filters later items as one group in the current coordinates until the matching `Pop`
    /// (Filter Effects 1 §5).
    PushFilter(FilterPaint),
    /// Masks later items as one group until the matching `Pop` (CSS Masking 1 §7.10).
    PushMask(MaskPaint),
    Pop,
}

/// One `text-shadow` layer of a stretch of glyph runs sharing their shadows, with the text
/// decorations drawn over them (CSS Text Decoration 3 §4).
#[derive(Clone, Debug, Serialize)]
pub struct TextShadowPaint {
    /// The line's left edge and baseline, which the runs are placed from.
    pub origin: Point,
    pub runs: Vec<crate::text::GlyphRun>,
    /// Decorations over the runs, shadowed only between `extent`'s left and right edges.
    pub decorations: Vec<DecorationPaint>,
    pub extent: [f32; 2],
    pub offset: [f32; 2],
    /// Blur radius; the Gaussian's standard deviation is half of it.
    pub blur: f32,
    pub color: Color,
}
impl TextShadowPaint {
    pub(crate) fn translate(&mut self, [dx, dy]: [f32; 2]) {
        self.origin = [self.origin[0] + dx, self.origin[1] + dy];
        self.extent = self.extent.map(|x| x + dx);
        for decoration in &mut self.decorations {
            decoration.rect.x += dx;
            decoration.rect.y += dy;
        }
    }
}

/// The masked element's box fragments on the page, whose mask layers combine.
#[derive(Clone, Debug, Serialize)]
pub struct MaskPaint {
    pub fragments: Vec<BoxPaint>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FilterPaint {
    pub functions: Vec<FilterFunction>,
    /// The filtered element's border box on the page, which `objectBoundingBox` units of
    /// referenced filters measure.
    pub bounds: Option<Rect>,
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
