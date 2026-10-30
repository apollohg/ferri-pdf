//! CSS Masking 1 §7 mask layers: lists like the background ones, with a mode and a compositing
//! operator per layer.
use crate::paint::{
    Backgrounds, BorderImage, BorderImageRepeat, BorderImageWidth, BoxArea, Clip, ClipPath,
    GeometryBox, ImageInfo, ImageSlice, LayerPaint, Rect, ShapeBox, UnresolvedImage,
    background::{cycle, place_layers},
};
use serde::Serialize;
use std::{convert::Infallible, sync::Arc};

/// `<masking-mode>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum MaskMode {
    Alpha,
    Luminance,
    #[default]
    MatchSource,
}

/// `<compositing-operator>`: how a layer combines with the layers below it (§7.8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum MaskComposite {
    #[default]
    Add,
    Subtract,
    Intersect,
    Exclude,
}

/// The initial `mask-origin` and `mask-clip`.
const BORDER_BOX: GeometryBox = GeometryBox::Shape(ShapeBox::Visual(BoxArea::BorderBox));

/// Computed `mask-*` longhands; layer `i` uses item `i` of each list, cycling short lists.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(bound(serialize = ""))]
pub struct Masks<I = Infallible> {
    /// `mask-image`, `-position`, `-size`, `-repeat`, `-origin` and `-clip` as background
    /// layers; a layer naming a `mask` element holds no image here.
    pub layers: Arc<Backgrounds<I>>,
    /// Same-document references to SVG `mask` elements, by `mask-image` layer.
    pub sources: Vec<Option<Box<str>>>,
    /// `mask-clip: no-clip`, cycling with the clip list.
    pub unclipped: Vec<bool>,
    /// `mask-origin` and `mask-clip` boxes as computed, whose fill, stroke and view boxes SVG
    /// content without a CSS layout box positions and clips by (§7.5, §7.6); `layers` holds
    /// the boxes of elements with one.
    pub(crate) origin_boxes: Vec<GeometryBox>,
    pub(crate) clip_boxes: Vec<GeometryBox>,
    pub modes: Vec<MaskMode>,
    pub composites: Vec<MaskComposite>,
    /// `mask-border-*` (§8), sliced like a border image.
    pub border: Arc<BorderImage<I>>,
    /// `mask-border-mode`; `match-source` does not occur.
    pub border_mode: MaskMode,
    /// `clip-path` (§6), which clips the same group the mask layers mask; `None` is `none`.
    pub(crate) clip_path: Option<Arc<ClipPath>>,
    /// A `clip-path` reference to an SVG `clipPath` element (§6.1).
    pub(crate) clip_reference: Option<Box<str>>,
}
impl<I> Default for Masks<I> {
    fn default() -> Self {
        Self {
            layers: Arc::new(Backgrounds {
                origins: vec![BoxArea::BorderBox],
                ..Backgrounds::default()
            }),
            sources: vec![None],
            unclipped: vec![false],
            origin_boxes: vec![BORDER_BOX],
            clip_boxes: vec![BORDER_BOX],
            modes: vec![MaskMode::default()],
            composites: vec![MaskComposite::default()],
            border: Arc::new(BorderImage {
                slice: [ImageSlice::Number(0.0); 4],
                widths: std::array::from_fn(|_| BorderImageWidth::Auto),
                repeat: [BorderImageRepeat::Stretch; 2],
                ..BorderImage::default()
            }),
            border_mode: MaskMode::Alpha,
            clip_path: None,
            clip_reference: None,
        }
    }
}
impl<I> Masks<I> {
    /// A `mask-image` other than `none` in any layer (§7.10), a `mask-border-source` other
    /// than `none` (§8.1), or a `clip-path` other than `none` (§6.1).
    pub fn is_masked(&self) -> bool {
        self.has_layers() || self.border.is_drawn() || self.is_clipped()
    }
    /// A `clip-path` other than `none`.
    pub fn is_clipped(&self) -> bool {
        self.clip_path.is_some() || self.clip_reference.is_some()
    }
    pub fn has_layers(&self) -> bool {
        self.layers.has_images() || self.sources.iter().any(Option::is_some)
    }
    pub fn layer_count(&self) -> usize {
        self.layers.images.len()
    }
    pub fn source(&self, layer: usize) -> Option<&str> {
        self.sources.get(layer).and_then(Option::as_deref)
    }
    pub fn mode(&self, layer: usize) -> MaskMode {
        self.modes[layer % self.modes.len()]
    }
    pub fn composite(&self, layer: usize) -> MaskComposite {
        self.composites[layer % self.composites.len()]
    }
    pub fn is_unclipped(&self, layer: usize) -> bool {
        self.unclipped[layer % self.unclipped.len()]
    }
}
/// The fill, stroke and view boxes of SVG content, which has no CSS layout box, in points.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SvgBoxes {
    pub(crate) fill: Rect,
    pub(crate) stroke: Rect,
    pub(crate) view: Rect,
}
impl SvgBoxes {
    /// The box a `<geometry-box>` names on SVG content: content and padding boxes are its fill
    /// box, and border and margin boxes its stroke box (§6.1, §7.5).
    pub(crate) fn area(&self, reference: GeometryBox) -> Rect {
        match reference {
            GeometryBox::Fill
            | GeometryBox::Shape(ShapeBox::Visual(BoxArea::ContentBox | BoxArea::PaddingBox)) => {
                self.fill
            }
            GeometryBox::View => self.view,
            GeometryBox::Stroke | GeometryBox::Shape(_) => self.stroke,
        }
    }
}

impl<I: ImageInfo> Masks<I> {
    /// The image layers of SVG content with `boxes`, bottom-most first by index. Its
    /// `mask-origin` content, padding and border boxes are its fill box (§7.6).
    pub(crate) fn svg_layers(
        &self,
        boxes: &SvgBoxes,
    ) -> Result<Vec<(usize, LayerPaint<'_, I>)>, UnresolvedImage> {
        place_layers(
            &self.layers,
            |index| match cycle(&self.origin_boxes, index) {
                GeometryBox::Shape(_) => boxes.fill,
                reference => boxes.area(*reference),
            },
            |index| Clip::rectangle(boxes.area(*cycle(&self.clip_boxes, index))),
        )
    }
}

impl Masks<Infallible> {
    pub fn with_image_type<I>(&self) -> Masks<I> {
        Masks {
            layers: Arc::new(self.layers.with_image_type()),
            sources: self.sources.clone(),
            unclipped: self.unclipped.clone(),
            origin_boxes: self.origin_boxes.clone(),
            clip_boxes: self.clip_boxes.clone(),
            modes: self.modes.clone(),
            composites: self.composites.clone(),
            border: Arc::new(self.border.with_image_type()),
            border_mode: self.border_mode,
            clip_path: self.clip_path.clone(),
            clip_reference: self.clip_reference.clone(),
        }
    }
}
