use super::{check_dimensions, pixels::from_pixels};
use crate::images::{
    ImageError, PixelDepth, PixelImage,
    pixels::{RGB_CHANNELS, color_profile},
};
use avif_decode::{Decoder, Image};
use zenavif_parse::AV1Metadata;

pub(super) fn decode_item(
    decoder: &mut Decoder,
    bytes: &[u8],
    budget: &mut u64,
    use_sequence_color: bool,
) -> Result<PixelImage, ImageError> {
    let metadata = AV1Metadata::parse_av1_bitstream(bytes)?;
    let width = metadata.max_frame_width.get();
    let height = metadata.max_frame_height.get();
    check_dimensions(width, height)?;
    *budget = budget
        .checked_sub(u64::from(width) * u64::from(height) * u64::from(frame_count(bytes)?))
        .ok_or(ImageError::Avif("coded images exceed pixel budget"))?;
    let decoded = decoder.decode(bytes, None, false)?;
    macro_rules! convert {
        ($image:expr, $depth:expr, $pixel:expr) => {{
            let image = $image;
            from_pixels(
                image.width() as u32,
                image.height() as u32,
                $depth,
                image.pixels().map($pixel),
            )
        }};
    }
    let mut image = match decoded {
        Image::Rgb8(i) => convert!(i, PixelDepth::Eight, |p| [
            u16::from(p.r),
            u16::from(p.g),
            u16::from(p.b)
        ]),
        Image::Rgb16(i) => convert!(i, PixelDepth::Sixteen, |p| [p.r, p.g, p.b]),
        Image::Gray8(i) => convert!(i, PixelDepth::Eight, |p| [u16::from(p.value());
            RGB_CHANNELS]),
        Image::Gray16(i) => convert!(i, PixelDepth::Sixteen, |p| [p.value(); RGB_CHANNELS]),
        Image::Rgba8(_) | Image::Rgba16(_) => {
            Err(ImageError::Avif("unexpected alpha in separate AV1 item"))
        }
    }?;
    let (primaries, transfer) = decoder.color_description().unwrap_or((2, 2));
    if use_sequence_color {
        image.icc = Some(color_profile(None, primaries, transfer)?);
    }
    Ok(image)
}

fn frame_count(mut bytes: &[u8]) -> Result<u32, ImageError> {
    const MAX_SPATIAL_LAYERS: u32 = 4;
    const OBU_EXTENSION: u8 = 4;
    const OBU_SIZE_FIELD: u8 = 2;
    const OBU_FRAME_HEADER: u8 = 3;
    const OBU_FRAME: u8 = 6;
    const OBU_SEQUENCE_HEADER: u8 = 1;
    const LEB_CONTINUATION: u8 = 128;
    const LEB_PAYLOAD: u8 = 127;
    const LEB_BITS: u32 = 7;
    const MAX_LEB_BYTES: u32 = 8;
    let mut frames = 0;
    let mut sequence_seen = false;
    while let Some((&header, rest)) = bytes.split_first() {
        if header & 0x81 != 0 {
            return Err(ImageError::Avif("invalid AV1 OBU header"));
        }
        bytes = rest;
        if header & OBU_EXTENSION != 0 {
            bytes = bytes
                .get(1..)
                .ok_or(ImageError::Avif("truncated OBU extension"))?;
        }
        let kind = (header >> 3) & 15;
        if kind == OBU_SEQUENCE_HEADER {
            if sequence_seen {
                return Err(ImageError::Avif("multiple sequence headers in AVIF item"));
            }
            sequence_seen = true;
        }
        if kind == OBU_FRAME_HEADER || kind == OBU_FRAME {
            frames += 1;
            if frames > MAX_SPATIAL_LAYERS {
                return Err(ImageError::Avif("too many frames in AVIF item"));
            }
        }
        if header & OBU_SIZE_FIELD == 0 {
            break;
        }
        let mut length = 0u64;
        for index in 0..MAX_LEB_BYTES {
            let (&byte, rest) = bytes
                .split_first()
                .ok_or(ImageError::Avif("truncated OBU length"))?;
            bytes = rest;
            length |= u64::from(byte & LEB_PAYLOAD) << (index * LEB_BITS);
            if byte & LEB_CONTINUATION == 0 {
                break;
            }
            if index + 1 == MAX_LEB_BYTES {
                return Err(ImageError::Avif("invalid OBU length"));
            }
        }
        let length =
            usize::try_from(length).map_err(|_| ImageError::Avif("OBU length overflow"))?;
        bytes = bytes
            .get(length..)
            .ok_or(ImageError::Avif("truncated OBU payload"))?;
    }
    if frames == 0 {
        return Err(ImageError::Avif("missing AV1 frame"));
    }
    Ok(frames)
}
