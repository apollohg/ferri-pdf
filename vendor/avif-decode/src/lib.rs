//! Decoding AVIF images (AV1 + HEIF) straight to uncompressed pixels (RGBA, etc).
//! Decoding is done by [rav1d](https://lib.rs/crates/rav1d), a Rust port of dav1d, so there's no C library to build.
//!
//! (an older version of this library supports libaom instead)
use crate::image::{Planes, write_alpha8, write_alpha16};
use crate::rav1d::Rav1dDecoder;
use crate::color::MatrixCoefficients;
use imgref::ImgVec;
use rgb::*;
use std::collections::TryReserveError;
use std::io;

use quick_error::quick_error;

mod color;
mod rav1d;
mod image;

quick_error! {
    #[derive(Debug)]
    #[non_exhaustive]
    pub enum Error {
        Io(err: io::Error) {
            display("{}", err)
            from()
        }
        Rav1d(errno: u32) {
            display("rav1d error ({})", errno)
        }
        Yuv(err: yuv::YuvError) {
            display("{}", err)
            from()
        }
        Unsupported(msg: &'static str) {
            display("{}", msg)
        }
        InternalError(msg: &'static str) {
            display("{}", msg)
        }
    }
}

impl From<TryReserveError> for Error {
    fn from(_: TryReserveError) -> Self {
        Self::Io(io::ErrorKind::OutOfMemory.into())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub enum Image {
    Rgb8(ImgVec<Rgb<u8>>),
    Rgb16(ImgVec<Rgb<u16>>),
    Rgba8(ImgVec<Rgba<u8>>),
    Rgba16(ImgVec<Rgba<u16>>),
    Gray8(ImgVec<Gray<u8>>),
    Gray16(ImgVec<Gray<u16>>),
}


/// Bounded AV1 still-image decoder. Container handling belongs to the caller.
pub struct Decoder {
    decoder: Rav1dDecoder,
    color_description: Option<(u16, u16)>,
}
impl Decoder {
    pub fn new(threads: usize, pixel_limit: u32) -> Result<Self> {
        if pixel_limit == 0 {
            return Err(Error::Unsupported("pixel limit must be positive"));
        }
        Ok(Self { decoder: Rav1dDecoder::new(threads, pixel_limit)?, color_description: None })
    }

    pub fn color_description(&self) -> Option<(u16, u16)> { self.color_description }

    pub fn decode(&mut self, primary: &[u8], alpha: Option<&[u8]>, premultiplied: bool) -> Result<Image> {
        // The decoder recycles buffers, so can't have both color and alpha without copying,
        // therefore conversion will put placeholders and then update alpha
        let has_alpha = alpha.is_some();
        let color = self.decoder.decode_frame(primary)?;
        self.color_description = color.color_description();
        let mut img = match color.planes()? {
            Planes::YuvPlanes8 { y, u, v, chroma_sampling } => {
                color.yuv_to_rgb8(y, u, v, chroma_sampling, has_alpha)?
            },
            Planes::Mono8(y) => color.yuv_to_gray8(y, has_alpha)?,
            Planes::Mono16(y, depth) => color.yuv_to_gray16(y, depth, has_alpha)?,
            Planes::YuvPlanes16 { y, u, v, chroma_sampling, depth } => {
                color.yuv_to_rgb16(y, u, v, depth, chroma_sampling, has_alpha)?
            },
        };
        let color_mc = color.matrix_coefficients().unwrap_or(MatrixCoefficients::Identity);

        if let Some(alpha_av1) = alpha {
            let alpha = self.decoder.decode_frame(&alpha_av1)?;
            let range = alpha.range();
            if let Some(alpha_mc) = alpha.matrix_coefficients().filter(|&mc| mc != MatrixCoefficients::Identity)
                && color_mc != alpha_mc {
                return Err(Error::Unsupported("alpha image has color info"));
            }
            match alpha.planes()? {
                Planes::YuvPlanes8 { y, .. } | Planes::Mono8(y) => {
                    write_alpha8(&mut img, y, range, premultiplied)?;
                },
                Planes::YuvPlanes16 { y, depth, .. } | Planes::Mono16(y, depth) => {
                    write_alpha16(&mut img, y, depth, range, premultiplied)?;
                },
            }
        } else if has_alpha {
            return Err(Error::InternalError("alpha item missing"));
        }
        Ok(img)
    }
}
