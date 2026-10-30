use serde::Serialize;
use std::{convert::Infallible, sync::Arc};

/// How an image scales to the size it paints at: `image-rendering` as used (CSS Images 3
/// §5.2), which every image paint carries to the PDF.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageRendering {
    /// `smooth` and `high-quality`: the viewer interpolates.
    Smooth,
    /// Nearest neighbour to the closest whole multiple of the natural size, then smooth.
    Pixelated,
    /// Nearest neighbour, as a viewer scales without interpolating.
    CrispEdges,
}
impl ImageRendering {
    /// `auto` leaves the algorithm to the UA (§5.2), which smooths: printed content images
    /// are mostly photographs, what `smooth` is for, and §5.2 gives bilinear interpolation as
    /// a UA's default; SVG content resolves `auto` as `optimizeQuality`, which is `smooth`.
    pub(crate) const AUTO: Self = Self::Smooth;

    /// Whether the PDF asks the viewer to interpolate the image's samples (ISO 32000-2
    /// §8.9.5.3 `Interpolate`).
    pub(crate) fn interpolates(self) -> bool {
        self != Self::CrispEdges
    }

    /// For `pixelated`, the whole multiple of each axis's `natural` pixels closest to its
    /// `target` device pixels, at least one, which nearest neighbour enlarges the image to
    /// before it scales the rest of the way as `smooth`; `None` when no axis enlarges.
    pub(crate) fn nearest_multiples(
        self,
        natural: (u32, u32),
        target: [f32; 2],
    ) -> Option<(u32, u32)> {
        const NOT_ENLARGED: (u32, u32) = (1, 1);
        if self != Self::Pixelated {
            return None;
        }
        let multiple = |pixels: u32, target: f32| (target / pixels as f32).round().max(1.0) as u32;
        let multiples = (
            multiple(natural.0, target[0]),
            multiple(natural.1, target[1]),
        );
        (multiples != NOT_ENLARGED).then_some(multiples)
    }
}

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
    /// Whether the image has pixels, whose size a natural resolution scales.
    fn is_raster(&self) -> bool;
}
impl<T: ImageInfo> ImageInfo for Arc<T> {
    fn natural_size(&self) -> NaturalDimensions {
        (**self).natural_size()
    }
    fn is_raster(&self) -> bool {
        (**self).is_raster()
    }
}
impl ImageInfo for Infallible {
    fn natural_size(&self) -> NaturalDimensions {
        match *self {}
    }
    fn is_raster(&self) -> bool {
        match *self {}
    }
}
