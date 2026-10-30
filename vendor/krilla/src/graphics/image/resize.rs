use std::borrow::Cow;
use std::io::Read;

use image::imageops::{resize, FilterType};
use image::{ImageBuffer, Luma, Pixel, Primitive, Rgb, Rgba};

use super::{BitsPerComponent, ImageColorspace, SampledRepr};
use crate::stream::deflate_encode;

const MAX_RESAMPLE_WORK_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const FLOAT_CHANNEL_BYTES: u64 = 4;
const FILTER_CHANNELS: u64 = 4;

struct RawSamples {
    color: Vec<u8>,
    alpha: Option<Vec<u8>>,
    depth: BitsPerComponent,
}

/// How resampling computes each output sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Resampling {
    /// Filtered from the input samples around it.
    Smooth,
    /// Copied from the input sample whose area contains the output sample's centre, which
    /// blends no colours.
    Nearest,
}

pub(super) fn sampled(
    source: &SampledRepr,
    color_space: ImageColorspace,
    size: (u32, u32),
    target: (u32, u32),
    resampling: Resampling,
) -> Result<SampledRepr, String> {
    let (color_length, alpha_length) = working_budget(
        color_space,
        source.bits_per_component,
        source.alpha_channel.is_some(),
        size,
        target,
    )?;
    let raw = RawSamples {
        color: inflate(&source.color_channel, color_length)?,
        alpha: source
            .alpha_channel
            .as_ref()
            .zip(alpha_length)
            .map(|(data, length)| inflate(data, length))
            .transpose()?,
        depth: source.bits_per_component,
    };
    let result = resample(&raw, color_space, size, target, resampling)?;
    Ok(SampledRepr {
        color_channel: deflate_encode(&result.color),
        alpha_channel: result.alpha.as_ref().map(|alpha| deflate_encode(alpha)),
        bits_per_component: result.depth,
    })
}

pub(super) fn jpeg(
    color: Vec<u8>,
    color_space: ImageColorspace,
    size: (u32, u32),
    target: (u32, u32),
    resampling: Resampling,
) -> Result<Vec<u8>, String> {
    working_budget(color_space, BitsPerComponent::Eight, false, size, target)?;
    let raw = RawSamples {
        color,
        alpha: None,
        depth: BitsPerComponent::Eight,
    };
    Ok(resample(&raw, color_space, size, target, resampling)?.color)
}

pub(super) fn working_budget(
    color_space: ImageColorspace,
    depth: BitsPerComponent,
    alpha: bool,
    size: (u32, u32),
    target: (u32, u32),
) -> Result<(usize, Option<usize>), String> {
    let bytes: u64 = match depth {
        BitsPerComponent::Eight => 1,
        BitsPerComponent::Sixteen => 2,
    };
    let color_bytes = bytes * u64::from(color_space.num_components());
    let total_bytes = color_bytes + if alpha { bytes } else { 0 };
    let pixels = u64::from(size.0) * u64::from(size.1);
    let output_pixels = u64::from(target.0) * u64::from(target.1);
    let transient = u64::from(size.0)
        .checked_mul(u64::from(target.1))
        .and_then(|count| count.checked_mul(FILTER_CHANNELS * FLOAT_CHANNEL_BYTES));
    let storage = pixels
        .checked_add(output_pixels)
        .and_then(|count| count.checked_mul(total_bytes))
        .and_then(|count| count.checked_mul(2));
    if transient
        .zip(storage)
        .and_then(|(filter, storage)| filter.checked_add(storage))
        .is_none_or(|bytes| bytes > MAX_RESAMPLE_WORK_BYTES)
    {
        return Err(format!(
            "image resampling exceeds the {MAX_RESAMPLE_WORK_BYTES}-byte working budget"
        ));
    }
    let color_length = usize::try_from(
        pixels
            .checked_mul(color_bytes)
            .ok_or("image plane size overflows")?,
    )
    .map_err(|_| "image plane size overflows")?;
    let alpha_length = if alpha {
        Some(
            usize::try_from(
                pixels
                    .checked_mul(bytes)
                    .ok_or("image alpha size overflows")?,
            )
            .map_err(|_| "image alpha size overflows")?,
        )
    } else {
        None
    };
    Ok((color_length, alpha_length))
}

pub(super) fn inflate(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .take(expected as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|error| error.to_string())?;
    if output.len() != expected {
        return Err("image sample count does not match its dimensions".into());
    }
    Ok(output)
}

trait Channel: Primitive + Into<u32> + TryFrom<u32> + 'static {
    type Rgb: Pixel<Subpixel = Self> + 'static;
    type Rgba: Pixel<Subpixel = Self> + 'static;
    fn decode(data: &[u8]) -> Cow<'_, [Self]>;
    fn encode(data: Vec<Self>) -> Vec<u8>;
}
impl Channel for u8 {
    type Rgb = Rgb<u8>;
    type Rgba = Rgba<u8>;
    fn decode(data: &[u8]) -> Cow<'_, [Self]> {
        Cow::Borrowed(data)
    }
    fn encode(data: Vec<Self>) -> Vec<u8> {
        data
    }
}
impl Channel for u16 {
    type Rgb = Rgb<u16>;
    type Rgba = Rgba<u16>;
    fn decode(data: &[u8]) -> Cow<'_, [Self]> {
        Cow::Owned(
            data.chunks_exact(2)
                .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                .collect(),
        )
    }
    fn encode(data: Vec<Self>) -> Vec<u8> {
        data.into_iter().flat_map(u16::to_be_bytes).collect()
    }
}

fn resample(
    source: &RawSamples,
    color_space: ImageColorspace,
    size: (u32, u32),
    target: (u32, u32),
    resampling: Resampling,
) -> Result<RawSamples, String> {
    if resampling == Resampling::Nearest {
        let bytes = usize::from(source.depth.as_u8() / 8);
        return Ok(RawSamples {
            color: nearest(
                &source.color,
                bytes * usize::from(color_space.num_components()),
                size,
                target,
            ),
            alpha: source
                .alpha
                .as_ref()
                .map(|alpha| nearest(alpha, bytes, size, target)),
            depth: source.depth,
        });
    }
    match source.depth {
        BitsPerComponent::Eight => resample_channels::<u8>(source, color_space, size, target),
        BitsPerComponent::Sixteen => resample_channels::<u16>(source, color_space, size, target),
    }
}

/// `samples`, rows of `size` pixels of `pixel_bytes` each, resampled to `target` by
/// nearest neighbour.
fn nearest(samples: &[u8], pixel_bytes: usize, size: (u32, u32), target: (u32, u32)) -> Vec<u8> {
    let source = |index: u32, from: u32, to: u32| {
        ((u64::from(index) * u64::from(from) * 2 + u64::from(from)) / (u64::from(to) * 2)) as usize
    };
    let columns: Vec<usize> = (0..target.0)
        .map(|column| source(column, size.0, target.0) * pixel_bytes)
        .collect();
    let row_bytes = size.0 as usize * pixel_bytes;
    let mut output = Vec::with_capacity(target.0 as usize * target.1 as usize * pixel_bytes);
    for row in 0..target.1 {
        let start = source(row, size.1, target.1) * row_bytes;
        let input = &samples[start..start + row_bytes];
        for column in &columns {
            output.extend_from_slice(&input[*column..*column + pixel_bytes]);
        }
    }
    output
}

fn resample_channels<T: Channel>(
    source: &RawSamples,
    color_space: ImageColorspace,
    size: (u32, u32),
    target: (u32, u32),
) -> Result<RawSamples, String> {
    let channels = usize::from(color_space.num_components());
    let bytes_per_sample = std::mem::size_of::<T>();
    let pixels = u64::from(size.0) * u64::from(size.1);
    if pixels
        .checked_mul(channels as u64)
        .and_then(|count| count.checked_mul(bytes_per_sample as u64))
        != Some(source.color.len() as u64)
        || source
            .alpha
            .as_ref()
            .is_some_and(|alpha| pixels * bytes_per_sample as u64 != alpha.len() as u64)
    {
        return Err("image sample count does not match its dimensions".into());
    }
    let mut color = T::decode(&source.color);
    let alpha = source.alpha.as_ref().map(|data| T::decode(data));
    let maximum: u32 = T::DEFAULT_MAX_VALUE.into();
    if let Some(alpha) = &alpha {
        for (pixel, alpha) in color.to_mut().chunks_exact_mut(channels).zip(alpha.iter()) {
            let alpha: u32 = (*alpha).into();
            for component in pixel {
                let value = ((*component).into() * alpha + maximum / 2) / maximum;
                *component =
                    T::try_from(value).map_err(|_| "associated image sample is out of range")?;
            }
        }
    }
    let mut color = channels_resize(&color, channels, size, target)?;
    let alpha = alpha
        .map(|alpha| channels_resize(&alpha, 1, size, target))
        .transpose()?;
    if let Some(alpha) = &alpha {
        for (pixel, alpha) in color.chunks_exact_mut(channels).zip(alpha.iter()) {
            let alpha: u32 = (*alpha).into();
            for component in pixel {
                let value = ((*component).into() * maximum + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(maximum);
                *component =
                    T::try_from(value).map_err(|_| "unassociated image sample is out of range")?;
            }
        }
    }
    Ok(RawSamples {
        color: T::encode(color),
        alpha: alpha.map(T::encode),
        depth: source.depth,
    })
}

fn channels_resize<T: Channel>(
    data: &[T],
    channels: usize,
    size: (u32, u32),
    target: (u32, u32),
) -> Result<Vec<T>, String> {
    macro_rules! filter {
        ($pixel:ty) => {{
            let image = ImageBuffer::<$pixel, _>::from_raw(size.0, size.1, data)
                .ok_or("invalid image plane dimensions")?;
            resize(&image, target.0, target.1, FilterType::CatmullRom).into_raw()
        }};
    }
    Ok(match channels {
        1 => filter!(Luma<T>),
        3 => filter!(T::Rgb),
        4 => filter!(T::Rgba),
        _ => return Err("unsupported image channel count".into()),
    })
}
