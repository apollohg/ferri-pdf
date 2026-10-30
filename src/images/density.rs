//! The resolution raster image formats record (CSS Images 4 §6.1 `from-image`): PNG `pHYs`,
//! the JPEG JFIF density, else EXIF `XResolution` and `YResolution`.
use crate::images::RasterFormat;
use std::io::Cursor;

const CSS_PIXELS_PER_INCH: f32 = 96.0;
const CENTIMETRES_PER_INCH: f32 = 2.54;
const METRES_PER_INCH: f32 = 0.0254;
/// JPEG markers (ITU T.81 §B.1.1.3) and the JFIF `APP0` segment (JFIF 1.02 §5).
const JPEG_SOI: [u8; 2] = [0xff, 0xd8];
const JPEG_MARKER: u8 = 0xff;
const JPEG_SOS: u8 = 0xda;
const JPEG_APP0: u8 = 0xe0;
const JFIF_IDENTIFIER: &[u8] = b"JFIF\0";
/// JFIF density units.
const JFIF_DOTS_PER_INCH: u8 = 1;
const JFIF_DOTS_PER_CENTIMETRE: u8 = 2;
/// TIFF byte orders, IFD entry layout and the resolution tags EXIF uses (TIFF 6.0 §2, §8).
const LITTLE_ENDIAN: [u8; 4] = [0x49, 0x49, 42, 0];
const BIG_ENDIAN: [u8; 4] = [0x4d, 0x4d, 0, 42];
const IFD_OFFSET: usize = 4;
const ENTRY_COUNT_BYTES: usize = 2;
const ENTRY_BYTES: usize = 12;
const ENTRY_VALUE: usize = 8;
const X_RESOLUTION: u16 = 0x011a;
const Y_RESOLUTION: u16 = 0x011b;
const RESOLUTION_UNIT: u16 = 0x0128;
const RATIONAL: u16 = 5;
const SHORT: u16 = 3;
const UNIT_INCH: u16 = 2;
const UNIT_CENTIMETRE: u16 = 3;

/// Image pixels per CSS pixel along the stored pixel rows and columns, if the image records
/// an absolute resolution.
pub(crate) fn recorded(
    bytes: &[u8],
    format: RasterFormat,
    exif: Option<&[u8]>,
) -> Option<[f32; 2]> {
    let native = match format {
        RasterFormat::Png => png_density(bytes),
        RasterFormat::Jpeg => jfif_density(bytes),
        RasterFormat::Gif | RasterFormat::WebP => None,
    };
    native.or_else(|| exif.and_then(exif_density))
}

fn per_inch(dots: f32, inches_per_unit: f32) -> Option<f32> {
    let dppx = dots / inches_per_unit / CSS_PIXELS_PER_INCH;
    (dppx.is_finite() && dppx > 0.0).then_some(dppx)
}

fn png_density(bytes: &[u8]) -> Option<[f32; 2]> {
    let reader = png::Decoder::new(Cursor::new(bytes)).read_info().ok()?;
    let dimensions = reader.info().pixel_dims?;
    if dimensions.unit != png::Unit::Meter {
        return None;
    }
    Some([
        per_inch(dimensions.xppu as f32, 1.0 / METRES_PER_INCH)?,
        per_inch(dimensions.yppu as f32, 1.0 / METRES_PER_INCH)?,
    ])
}

/// The density of a JFIF `APP0` segment among the segments before the scan data.
fn jfif_density(bytes: &[u8]) -> Option<[f32; 2]> {
    let mut at = JPEG_SOI.len();
    while bytes.get(at) == Some(&JPEG_MARKER) {
        let marker = *bytes.get(at + 1)?;
        if marker == JPEG_SOS {
            return None;
        }
        let length = usize::from(u16::from_be_bytes([
            *bytes.get(at + 2)?,
            *bytes.get(at + 3)?,
        ]));
        let segment = bytes.get(at + 4..at + 2 + length)?;
        if marker == JPEG_APP0
            && let Some(header) = segment.strip_prefix(JFIF_IDENTIFIER)
        {
            let [_, _, unit, x_high, x_low, y_high, y_low, ..] = *header else {
                return None;
            };
            let inches_per_unit = match unit {
                JFIF_DOTS_PER_INCH => 1.0,
                JFIF_DOTS_PER_CENTIMETRE => 1.0 / CENTIMETRES_PER_INCH,
                _ => return None,
            };
            return Some([
                per_inch(
                    f32::from(u16::from_be_bytes([x_high, x_low])),
                    inches_per_unit,
                )?,
                per_inch(
                    f32::from(u16::from_be_bytes([y_high, y_low])),
                    inches_per_unit,
                )?,
            ]);
        }
        at += 2 + length;
    }
    None
}

/// The resolution tags of the first IFD of a TIFF-structured EXIF chunk.
fn exif_density(chunk: &[u8]) -> Option<[f32; 2]> {
    let little = match chunk.get(..IFD_OFFSET)? {
        header if header == LITTLE_ENDIAN => true,
        header if header == BIG_ENDIAN => false,
        _ => return None,
    };
    let u16_at = |at: usize| {
        let bytes = [*chunk.get(at)?, *chunk.get(at + 1)?];
        Some(if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let u32_at = |at: usize| {
        let bytes: [u8; 4] = chunk.get(at..at + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    let ifd = u32_at(IFD_OFFSET)? as usize;
    let entries = u16_at(ifd)?;
    let mut resolution = [None; 2];
    let mut unit = UNIT_INCH;
    for index in 0..usize::from(entries) {
        let entry = ifd + ENTRY_COUNT_BYTES + index * ENTRY_BYTES;
        let (tag, kind) = (u16_at(entry)?, u16_at(entry + 2)?);
        match (tag, kind) {
            (X_RESOLUTION | Y_RESOLUTION, RATIONAL) => {
                let value = u32_at(entry + ENTRY_VALUE)? as usize;
                let (numerator, denominator) = (u32_at(value)?, u32_at(value + 4)?);
                resolution[usize::from(tag == Y_RESOLUTION)] =
                    (denominator != 0).then(|| numerator as f32 / denominator as f32);
            }
            (RESOLUTION_UNIT, SHORT) => unit = u16_at(entry + ENTRY_VALUE)?,
            _ => {}
        }
    }
    let inches_per_unit = match unit {
        UNIT_INCH => 1.0,
        UNIT_CENTIMETRE => 1.0 / CENTIMETRES_PER_INCH,
        _ => return None,
    };
    Some([
        per_inch(resolution[0]?, inches_per_unit)?,
        per_inch(resolution[1]?, inches_per_unit)?,
    ])
}
