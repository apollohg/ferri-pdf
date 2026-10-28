use crate::{
    ImageError, MAX_BYTES_PER_PIXEL, MAX_DECODED_BYTES, PixelDepth, PixelImage,
    pixels::RGB_CHANNELS,
};
use heic_decoder::{
    HeifError,
    bitreader::strip_emulation_prevention,
    hevc::{
        nal::{NAL_SPS, parse_nal_header},
        params::parse_sps,
    },
    isobmff::{ParsedHeif, ResolvedImage},
};

const FILE_TYPE: &[u8; 4] = b"ftyp";
const BOX_TYPE_OFFSET: usize = 4;
const BOX_HEADER_BYTES: usize = 8;
const PIXEL_LIMIT: u64 = MAX_DECODED_BYTES / MAX_BYTES_PER_PIXEL;

pub(crate) fn matches(bytes: &[u8]) -> bool {
    bytes.get(BOX_TYPE_OFFSET..BOX_HEADER_BYTES) == Some(FILE_TYPE)
}

fn coded_pixels(image: &ResolvedImage) -> Result<u64, HeifError> {
    for nal in image.hevc_config.nalus_of_type(NAL_SPS) {
        let (header, payload) = parse_nal_header(nal)?;
        if header.nal_unit_type == NAL_SPS && header.nuh_layer_id == 0 {
            let sps = parse_sps(&strip_emulation_prevention(payload))?;
            return Ok(u64::from(sps.pic_width_in_luma_samples)
                * u64::from(sps.pic_height_in_luma_samples));
        }
    }
    Err(HeifError::MalformedHevc("missing primary-layer SPS"))
}

fn check_budget(parsed: &ParsedHeif) -> Result<(), HeifError> {
    let mut pixels = 0u64;
    for image in parsed.images.iter().chain(parsed.reference_image.iter()) {
        pixels = pixels
            .checked_add(coded_pixels(image)?)
            .filter(|pixels| *pixels <= PIXEL_LIMIT)
            .ok_or(HeifError::LimitExceeded(
                "HEIF coded images exceed renderer pixel budget",
            ))?;
    }
    if let Some(grid) = parsed.grid
        && u64::from(grid.output_width) * u64::from(grid.output_height) > PIXEL_LIMIT
    {
        return Err(HeifError::LimitExceeded(
            "HEIF grid exceeds renderer pixel budget",
        ));
    }
    if let Some(alpha) = &parsed.alpha {
        check_budget(&alpha.image)?;
    }
    Ok(())
}

pub(crate) fn decode(bytes: &[u8]) -> Result<PixelImage, ImageError> {
    let parsed = heic_decoder::isobmff::parse(bytes)?;
    check_budget(&parsed)?;
    let decoded = heic_decoder::decode_parsed(parsed, &Default::default())?;
    let nclx = decoded.color_info.unwrap_or_default();
    let icc = Some(crate::pixels::color_profile(
        decoded.metadata.icc_profile.as_deref(),
        nclx.color_primaries,
        nclx.transfer_characteristics,
    )?);
    let depth = if decoded
        .bit_depth_luma
        .max(decoded.bit_depth_chroma)
        .max(decoded.alpha.as_ref().map_or(0, |alpha| alpha.bit_depth))
        > u8::BITS
    {
        PixelDepth::Sixteen
    } else {
        PixelDepth::Eight
    };
    let component_bytes = depth.bytes();
    let pixel_count = decoded.width * decoded.height;
    let mut image = PixelImage {
        width: decoded.width as u32,
        height: decoded.height as u32,
        depth,
        color: Vec::with_capacity(pixel_count * RGB_CHANNELS * component_bytes),
        alpha: decoded
            .alpha
            .as_ref()
            .map(|_| Vec::with_capacity(pixel_count * component_bytes)),
        icc,
    };
    match depth {
        PixelDepth::Eight => {
            let rgba = decoded.to_rgba8()?;
            split_pixels(
                &mut image,
                rgba.data.into_iter().map(u16::from),
                rgba.premultiplied,
            );
        }
        PixelDepth::Sixteen => {
            let rgba = decoded.to_rgba16()?;
            split_pixels(&mut image, rgba.data.into_iter(), rgba.premultiplied);
        }
    }
    Ok(image)
}

fn split_pixels(image: &mut PixelImage, mut rgba: impl Iterator<Item = u16>, premultiplied: bool) {
    let max = match image.depth {
        PixelDepth::Eight => u32::from(u8::MAX),
        PixelDepth::Sixteen => u32::from(u16::MAX),
    };
    while let (Some(r), Some(g), Some(b), Some(a)) =
        (rgba.next(), rgba.next(), rgba.next(), rgba.next())
    {
        for sample in [r, g, b] {
            let value = if premultiplied {
                crate::pixels::unpremultiply(u32::from(sample), u32::from(a), max) as u16
            } else {
                sample
            };
            match image.depth {
                PixelDepth::Eight => image.color.push(value as u8),
                PixelDepth::Sixteen => image.color.extend_from_slice(&value.to_be_bytes()),
            }
        }
        if let Some(alpha) = &mut image.alpha {
            match image.depth {
                PixelDepth::Eight => alpha.push(a as u8),
                PixelDepth::Sixteen => alpha.extend_from_slice(&a.to_be_bytes()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_uses_actual_sps_nal_type_not_just_the_container_array_type() {
        let bytes = include_bytes!("../../../tests/oracle/assets/image-pattern.heic");
        let mut parsed = heic_decoder::isobmff::parse(bytes).unwrap();
        let image = &mut parsed.images[0];
        let array = image
            .hevc_config
            .arrays
            .iter_mut()
            .find(|array| array.nal_unit_type == NAL_SPS)
            .unwrap();
        for nal in &mut array.nalus {
            nal[0] = 0;
        }
        assert!(
            matches!(coded_pixels(image), Err(HeifError::MalformedHevc(_))),
            "SPS-shaped data under another NAL type must not satisfy preflight"
        );
    }

    #[test]
    fn oversized_grid_is_rejected_before_decoding_tiles() {
        let mut bytes = include_bytes!("../../../tests/oracle/assets/image-grid.heic").to_vec();
        let descriptor = [0, 0, 0, 1, 0, 80, 0, 40];
        let positions: Vec<_> = bytes
            .windows(descriptor.len())
            .enumerate()
            .filter_map(|(index, window)| (window == descriptor).then_some(index))
            .collect();
        assert_eq!(
            positions.len(),
            1,
            "fixture must have exactly one grid descriptor"
        );
        let offset = positions[0];
        let oversized_dimension = u16::try_from(PIXEL_LIMIT.isqrt() + 1)
            .expect("over-budget grid dimension must fit its descriptor")
            .to_be_bytes();
        bytes[offset + 4..offset + 6].copy_from_slice(&oversized_dimension);
        bytes[offset + 6..offset + 8].copy_from_slice(&oversized_dimension);
        assert!(
            matches!(
                decode(&bytes),
                Err(ImageError::Heif(HeifError::LimitExceeded(_)))
            ),
            "oversized canvas must fail before pixel decoding"
        );
    }

    #[test]
    fn truncated_heif_returns_a_decode_error() {
        let bytes = include_bytes!("../../../tests/oracle/assets/image-pattern.heic");
        for end in [BOX_HEADER_BYTES, bytes.len() / 2, bytes.len() - 1] {
            assert!(
                decode(&bytes[..end]).is_err(),
                "truncation at byte {end} must not silently produce a partial image"
            );
        }
    }

    #[test]
    fn premultiplied_channels_are_unassociated_at_both_depths() {
        for (depth, samples, expected_color, expected_alpha) in [
            (
                PixelDepth::Eight,
                vec![64, 32, 0, 128, 0, 0, 0, 0],
                vec![128, 64, 0, 0, 0, 0],
                vec![128, 0],
            ),
            (
                PixelDepth::Sixteen,
                vec![16384, 8192, 0, 32768, 0, 0, 0, 0],
                vec![128, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                vec![128, 0, 0, 0],
            ),
        ] {
            let mut image = PixelImage {
                width: 2,
                height: 1,
                depth,
                color: Vec::new(),
                alpha: Some(Vec::new()),
                icc: None,
            };
            split_pixels(&mut image, samples.into_iter(), true);
            assert_eq!(
                image.color, expected_color,
                "{depth:?}: RGB must not remain darkened by alpha"
            );
            assert_eq!(
                image.alpha.unwrap(),
                expected_alpha,
                "{depth:?}: opacity is unchanged"
            );
        }
    }
}
