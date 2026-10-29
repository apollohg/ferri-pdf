//! CSS Masking 1 §7 mask layers: lists like the background ones, with a mode and a compositing
//! operator per layer.
use crate::paint::{
    Backgrounds, BorderImage, BorderImageRepeat, BorderImageWidth, BoxArea, ClipPath, ImageSlice,
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
    pub modes: Vec<MaskMode>,
    pub composites: Vec<MaskComposite>,
    /// `mask-border-*` (§8), sliced like a border image.
    pub border: Arc<BorderImage<I>>,
    /// `mask-border-mode`; `match-source` does not occur.
    pub border_mode: MaskMode,
    /// `clip-path` (§6), which clips the same group the mask layers mask; `None` is `none`.
    pub(crate) clip_path: Option<Arc<ClipPath>>,
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
        }
    }
}
impl<I> Masks<I> {
    /// A `mask-image` other than `none` in any layer (§7.10), a `mask-border-source` other
    /// than `none` (§8.1), or a `clip-path` other than `none` (§6.1).
    pub fn is_masked(&self) -> bool {
        self.has_layers() || self.border.is_drawn() || self.clip_path.is_some()
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
impl Masks<Infallible> {
    pub fn with_image_type<I>(&self) -> Masks<I> {
        Masks {
            layers: Arc::new(self.layers.with_image_type()),
            sources: self.sources.clone(),
            unclipped: self.unclipped.clone(),
            modes: self.modes.clone(),
            composites: self.composites.clone(),
            border: Arc::new(self.border.with_image_type()),
            border_mode: self.border_mode,
            clip_path: self.clip_path.clone(),
        }
    }
}
