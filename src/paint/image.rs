use std::{convert::Infallible, sync::Arc};

/// Natural dimensions in CSS pixels and natural aspect ratio; SVG may lack any of them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NaturalDimensions {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub ratio: Option<f32>,
}
impl NaturalDimensions {
    /// An aspect ratio without a natural width or height, as a `viewBox`-only SVG.
    pub fn ratio_only(&self) -> bool {
        self.width.is_none() && self.height.is_none() && self.ratio.is_some()
    }
}
/// Intrinsic metadata used by image-independent sizing and paint algorithms.
pub trait ImageInfo {
    fn natural_size(&self) -> NaturalDimensions;
}
impl<T: ImageInfo> ImageInfo for Arc<T> {
    fn natural_size(&self) -> NaturalDimensions {
        (**self).natural_size()
    }
}
impl ImageInfo for Infallible {
    fn natural_size(&self) -> NaturalDimensions {
        match *self {}
    }
}
