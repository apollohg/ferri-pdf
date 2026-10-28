use crate::images::ImageError;
use std::borrow::Cow;

pub(crate) const RGB_CHANNELS: usize = 3;

#[derive(Clone, Copy, Debug)]
pub enum PixelDepth {
    Eight,
    Sixteen,
}
impl PixelDepth {
    pub fn bytes(self) -> usize {
        match self {
            Self::Eight => 1,
            Self::Sixteen => 2,
        }
    }
}

#[derive(Debug)]
pub struct PixelImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) depth: PixelDepth,
    /// RGB samples, with sixteen-bit samples in big-endian order.
    pub(crate) color: Vec<u8>,
    pub(crate) alpha: Option<Vec<u8>>,
    pub(crate) icc: Option<Vec<u8>>,
}
impl PixelImage {
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn depth(&self) -> PixelDepth {
        self.depth
    }
    pub fn color(&self) -> &[u8] {
        &self.color
    }
    pub fn alpha(&self) -> Option<&[u8]> {
        self.alpha.as_deref()
    }
    pub fn icc(&self) -> Option<&[u8]> {
        self.icc.as_deref()
    }

    pub(crate) fn png(&self) -> Result<Vec<u8>, ImageError> {
        let mut info = png::Info::with_size(self.width, self.height);
        info.bit_depth = match self.depth {
            PixelDepth::Eight => png::BitDepth::Eight,
            PixelDepth::Sixteen => png::BitDepth::Sixteen,
        };
        info.color_type = if self.alpha.is_some() {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        };
        info.icc_profile = self.icc.as_deref().map(Cow::Borrowed);
        let samples = if let Some(alpha) = &self.alpha {
            let mut rgba = Vec::with_capacity(self.color.len() + alpha.len());
            for (color, alpha) in self
                .color
                .chunks_exact(RGB_CHANNELS * self.depth.bytes())
                .zip(alpha.chunks_exact(self.depth.bytes()))
            {
                rgba.extend_from_slice(color);
                rgba.extend_from_slice(alpha);
            }
            Cow::Owned(rgba)
        } else {
            Cow::Borrowed(self.color.as_slice())
        };
        let mut output = Vec::new();
        let mut writer = png::Encoder::with_info(&mut output, info)?.write_header()?;
        writer.write_image_data(&samples)?;
        writer.finish()?;
        Ok(output)
    }
}

pub(crate) fn color_profile(
    icc: Option<&[u8]>,
    primaries: u16,
    transfer: u16,
) -> Result<Vec<u8>, ImageError> {
    const UNSPECIFIED_COLOR: u16 = 2;
    const SRGB_PRIMARIES: u16 = 1;
    const SRGB_TRANSFER: u16 = 13;
    if let Some(profile) = icc {
        if moxcms::ColorProfile::new_from_slice(profile)?.color_space != moxcms::DataColorSpace::Rgb
        {
            return Err(ImageError::NonRgbProfile);
        }
        return Ok(profile.to_vec());
    }
    let primaries = if primaries == UNSPECIFIED_COLOR {
        SRGB_PRIMARIES
    } else {
        primaries
    };
    let transfer = if transfer == UNSPECIFIED_COLOR {
        SRGB_TRANSFER
    } else {
        transfer
    };
    let unsupported = || ImageError::ColorDescription {
        primaries,
        transfer,
    };
    let profile = moxcms::ColorProfile::new_from_cicp(moxcms::CicpProfile {
        color_primaries: u8::try_from(primaries)
            .map_err(|_| unsupported())?
            .try_into()?,
        transfer_characteristics: u8::try_from(transfer)
            .map_err(|_| unsupported())?
            .try_into()?,
        matrix_coefficients: moxcms::MatrixCoefficients::Identity,
        full_range: true,
    });
    if profile.red_trc.is_none() || profile.green_trc.is_none() || profile.blue_trc.is_none() {
        return Err(unsupported());
    }
    Ok(profile.encode()?)
}

pub(crate) fn unpremultiply(sample: u32, alpha: u32, maximum: u32) -> u32 {
    (sample * maximum + alpha / 2)
        .checked_div(alpha)
        .unwrap_or(0)
        .min(maximum)
}
