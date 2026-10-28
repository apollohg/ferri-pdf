use crate::graphics::ShadowMask;
use ferri_pdf_images::{PixelDepth, PixelImage};
use krilla::image::{BitsPerComponent, CustomImage, ImageColorspace};
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(Clone)]
pub(super) struct Pixels(pub Arc<PixelImage>);
impl Hash for Pixels {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}
impl CustomImage for Pixels {
    fn color_channel(&self) -> &[u8] {
        self.0.color()
    }
    fn alpha_channel(&self) -> Option<&[u8]> {
        self.0.alpha()
    }
    fn bits_per_component(&self) -> BitsPerComponent {
        match self.0.depth() {
            PixelDepth::Eight => BitsPerComponent::Eight,
            PixelDepth::Sixteen => BitsPerComponent::Sixteen,
        }
    }
    fn size(&self) -> (u32, u32) {
        self.0.size()
    }
    fn icc_profile(&self) -> Option<&[u8]> {
        self.0.icc()
    }
    fn color_space(&self) -> ImageColorspace {
        ImageColorspace::Rgb
    }
}

/// A blurred shadow's alpha as a grey image for a luminosity soft mask. Masks are built
/// while painting and dropped after, so they are identified by content.
#[derive(Clone)]
pub(super) struct MaskPixels(pub Arc<ShadowMask>);
impl Hash for MaskPixels {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.size.hash(state);
        self.0.alpha.hash(state);
    }
}
impl CustomImage for MaskPixels {
    fn color_channel(&self) -> &[u8] {
        &self.0.alpha
    }
    fn alpha_channel(&self) -> Option<&[u8]> {
        None
    }
    fn bits_per_component(&self) -> BitsPerComponent {
        BitsPerComponent::Eight
    }
    fn size(&self) -> (u32, u32) {
        (self.0.size[0], self.0.size[1])
    }
    fn icc_profile(&self) -> Option<&[u8]> {
        None
    }
    fn color_space(&self) -> ImageColorspace {
        ImageColorspace::Luma
    }
}
