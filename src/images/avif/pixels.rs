use super::check_dimensions;
use crate::images::{ImageError, PixelDepth, PixelImage, pixels::RGB_CHANNELS};

pub(super) fn from_pixels(
    width: u32,
    height: u32,
    depth: PixelDepth,
    pixels: impl Iterator<Item = [u16; RGB_CHANNELS]>,
) -> Result<PixelImage, ImageError> {
    check_dimensions(width, height)?;
    let mut color =
        Vec::with_capacity(width as usize * height as usize * RGB_CHANNELS * depth.bytes());
    for pixel in pixels {
        for sample in pixel {
            match depth {
                PixelDepth::Eight => color.push(sample as u8),
                PixelDepth::Sixteen => color.extend_from_slice(&sample.to_be_bytes()),
            }
        }
    }
    Ok(PixelImage {
        width,
        height,
        depth,
        color,
        alpha: None,
        icc: None,
    })
}

pub(super) fn copy_tile(
    canvas: &mut PixelImage,
    tile: &PixelImage,
    x: u32,
    y: u32,
) -> Result<(), ImageError> {
    if canvas.depth.bytes() != tile.depth.bytes() {
        return Err(ImageError::Avif("inconsistent grid bit depth"));
    }
    if canvas.alpha.is_some() != tile.alpha.is_some() {
        return Err(ImageError::Avif("inconsistent grid alpha presence"));
    }
    let bytes = RGB_CHANNELS * canvas.depth.bytes();
    let width = tile.width.min(canvas.width - x) as usize;
    for row in 0..tile.height.min(canvas.height - y) as usize {
        let source = row * tile.width as usize * bytes;
        let target = ((y as usize + row) * canvas.width as usize + x as usize) * bytes;
        canvas.color[target..target + width * bytes]
            .copy_from_slice(&tile.color[source..source + width * bytes]);
        if let (Some(canvas_alpha), Some(tile_alpha)) = (&mut canvas.alpha, &tile.alpha) {
            let (source, target, count) = (
                source / RGB_CHANNELS,
                target / RGB_CHANNELS,
                width * canvas.depth.bytes(),
            );
            canvas_alpha[target..target + count]
                .copy_from_slice(&tile_alpha[source..source + count]);
        }
    }
    Ok(())
}

fn promote(image: &mut PixelImage) {
    if matches!(image.depth, PixelDepth::Eight) {
        image.color = image.color.iter().flat_map(|s| [*s, *s]).collect();
        image.depth = PixelDepth::Sixteen;
    }
}

pub(super) fn attach_alpha(
    image: &mut PixelImage,
    mut alpha: PixelImage,
    premultiplied: bool,
) -> Result<(), ImageError> {
    if image.size() != alpha.size() {
        return Err(ImageError::Avif("alpha dimensions do not match color"));
    }
    if image.depth.bytes() != alpha.depth.bytes() {
        promote(image);
        promote(&mut alpha);
    }
    let bytes = image.depth.bytes();
    let plane: Vec<u8> = alpha
        .color
        .chunks_exact(RGB_CHANNELS * bytes)
        .flat_map(|pixel| pixel[..bytes].iter().copied())
        .collect();
    if premultiplied {
        let maximum = if bytes == 1 {
            u32::from(u8::MAX)
        } else {
            u32::from(u16::MAX)
        };
        let sample = |p: &[u8]| {
            if bytes == 1 {
                u32::from(p[0])
            } else {
                u32::from(u16::from_be_bytes([p[0], p[1]]))
            }
        };
        for (pixel, a) in image
            .color
            .chunks_exact_mut(RGB_CHANNELS * bytes)
            .zip(plane.chunks_exact(bytes))
        {
            let a = sample(a);
            for channel in pixel.chunks_exact_mut(bytes) {
                let straight = crate::images::pixels::unpremultiply(sample(channel), a, maximum);
                if bytes == 1 {
                    channel[0] = straight as u8;
                } else {
                    channel.copy_from_slice(&(straight as u16).to_be_bytes());
                }
            }
        }
    }
    image.alpha = Some(plane);
    Ok(())
}

pub(super) fn transform(
    image: PixelImage,
    operations: &[zenavif_parse::ImageTransform],
) -> Result<PixelImage, ImageError> {
    use zenavif_parse::ImageTransform;
    if operations.is_empty() {
        return Ok(image);
    }
    let (mut width, mut height) = (image.width, image.height);
    let mut map = CoordinateMap::IDENTITY;
    for operation in operations {
        let next = match operation {
            ImageTransform::Crop(crop) => {
                let integer = |n: i64, d: u32| -> Result<i64, ImageError> {
                    if d == 0 || n % i64::from(d) != 0 {
                        return Err(ImageError::Avif("non-integral clean aperture"));
                    }
                    Ok(n / i64::from(d))
                };
                let w = integer(i64::from(crop.width_n), crop.width_d)?;
                let h = integer(i64::from(crop.height_n), crop.height_d)?;
                let x_twice = i64::from(width) - w
                    + integer(2 * i64::from(crop.horiz_off_n), crop.horiz_off_d)?;
                let y_twice = i64::from(height) - h
                    + integer(2 * i64::from(crop.vert_off_n), crop.vert_off_d)?;
                if w <= 0
                    || h <= 0
                    || x_twice < 0
                    || y_twice < 0
                    || x_twice % 2 != 0
                    || y_twice % 2 != 0
                    || x_twice / 2 + w > i64::from(width)
                    || y_twice / 2 + h > i64::from(height)
                {
                    return Err(ImageError::Avif("clean aperture outside image"));
                }
                (width, height) = (w as u32, h as u32);
                CoordinateMap {
                    x: x_twice / 2,
                    y: y_twice / 2,
                    ..CoordinateMap::IDENTITY
                }
            }
            ImageTransform::Rotate(rotation) => {
                let next = match rotation.angle {
                    0 => CoordinateMap::IDENTITY,
                    90 => CoordinateMap {
                        xx: 0,
                        xy: -1,
                        yx: 1,
                        yy: 0,
                        x: i64::from(width) - 1,
                        y: 0,
                    },
                    180 => CoordinateMap {
                        xx: -1,
                        xy: 0,
                        yx: 0,
                        yy: -1,
                        x: i64::from(width) - 1,
                        y: i64::from(height) - 1,
                    },
                    270 => CoordinateMap {
                        xx: 0,
                        xy: 1,
                        yx: -1,
                        yy: 0,
                        x: 0,
                        y: i64::from(height) - 1,
                    },
                    _ => return Err(ImageError::Avif("invalid rotation")),
                };
                if rotation.angle == 90 || rotation.angle == 270 {
                    (width, height) = (height, width);
                }
                next
            }
            ImageTransform::Mirror(mirror) => match mirror.axis {
                0 => CoordinateMap {
                    yy: -1,
                    y: i64::from(height) - 1,
                    ..CoordinateMap::IDENTITY
                },
                1 => CoordinateMap {
                    xx: -1,
                    x: i64::from(width) - 1,
                    ..CoordinateMap::IDENTITY
                },
                _ => return Err(ImageError::Avif("invalid mirror direction")),
            },
        };
        map = map.compose(next);
    }
    let bytes = image.depth.bytes();
    let mut output = PixelImage {
        width,
        height,
        depth: image.depth,
        color: Vec::with_capacity(width as usize * height as usize * RGB_CHANNELS * bytes),
        alpha: image
            .alpha
            .as_ref()
            .map(|_| Vec::with_capacity(width as usize * height as usize * bytes)),
        icc: image.icc,
    };
    for y in 0..height {
        for x in 0..width {
            let sx = map.xx * i64::from(x) + map.xy * i64::from(y) + map.x;
            let sy = map.yx * i64::from(x) + map.yy * i64::from(y) + map.y;
            let source = (sy as usize * image.width as usize + sx as usize) * bytes;
            output.color.extend_from_slice(
                &image.color[source * RGB_CHANNELS..(source + bytes) * RGB_CHANNELS],
            );
            if let (Some(input), Some(alpha)) = (&image.alpha, &mut output.alpha) {
                alpha.extend_from_slice(&input[source..source + bytes]);
            }
        }
    }
    Ok(output)
}

#[derive(Clone, Copy)]
struct CoordinateMap {
    xx: i64,
    xy: i64,
    yx: i64,
    yy: i64,
    x: i64,
    y: i64,
}
impl CoordinateMap {
    const IDENTITY: Self = Self {
        xx: 1,
        xy: 0,
        yx: 0,
        yy: 1,
        x: 0,
        y: 0,
    };
    fn compose(self, next: Self) -> Self {
        Self {
            xx: self.xx * next.xx + self.xy * next.yx,
            xy: self.xx * next.xy + self.xy * next.yy,
            yx: self.yx * next.xx + self.yy * next.yx,
            yy: self.yx * next.xy + self.yy * next.yy,
            x: self.xx * next.x + self.xy * next.y + self.x,
            y: self.yx * next.x + self.yy * next.y + self.y,
        }
    }
}
