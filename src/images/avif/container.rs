use crate::images::ImageError;
const BRAND_LENGTH: usize = 4;
const AVIF_BRANDS: [&[u8; BRAND_LENGTH]; 2] = [b"avif", b"avis"];

pub(crate) fn matches(bytes: &[u8]) -> bool {
    const HEADER: usize = 8;
    const FILE_TYPE: &[u8] = b"ftyp";
    if bytes.get(4..HEADER) != Some(FILE_TYPE) {
        return false;
    }
    const EXTENDED_HEADER: usize = 16;
    const EXTENDED_SIZE: u32 = 1;
    const TO_END: u32 = 0;
    let (header, size) = match u32::from_be_bytes(bytes[..4].try_into().unwrap()) {
        EXTENDED_SIZE => {
            let Some(size) = bytes.get(HEADER..EXTENDED_HEADER) else {
                return false;
            };
            let Ok(size) = usize::try_from(u64::from_be_bytes(size.try_into().unwrap())) else {
                return false;
            };
            (EXTENDED_HEADER, size)
        }
        TO_END => (HEADER, bytes.len()),
        size => (HEADER, size as usize),
    };
    let Some(brands) = bytes.get(header..size) else {
        return false;
    };
    brands
        .as_chunks::<BRAND_LENGTH>()
        .0
        .iter()
        .enumerate()
        .any(|(index, brand)| index != 1 && AVIF_BRANDS.contains(&brand))
}

pub(super) struct Grid {
    pub(super) rows: u16,
    pub(super) columns: u16,
    pub(super) output_width: u32,
    pub(super) output_height: u32,
}
impl Grid {
    pub(super) fn parse(data: &[u8]) -> Result<Self, ImageError> {
        const SHORT_GRID_BYTES: usize = 8;
        const LONG_GRID_BYTES: usize = 12;
        const LARGE_DIMENSIONS: u8 = 1;
        if data.len() < SHORT_GRID_BYTES || data[0] != 0 || data[1] & !LARGE_DIMENSIONS != 0 {
            return Err(ImageError::Avif("invalid grid descriptor"));
        }
        let large = data[1] & LARGE_DIMENSIONS != 0;
        if data.len()
            != if large {
                LONG_GRID_BYTES
            } else {
                SHORT_GRID_BYTES
            }
        {
            return Err(ImageError::Avif("invalid grid descriptor length"));
        }
        let (output_width, output_height) = if large {
            (
                u32::from_be_bytes(data[4..8].try_into().unwrap()),
                u32::from_be_bytes(data[8..12].try_into().unwrap()),
            )
        } else {
            (
                u32::from(u16::from_be_bytes([data[4], data[5]])),
                u32::from(u16::from_be_bytes([data[6], data[7]])),
            )
        };
        Ok(Self {
            rows: u16::from(data[2]) + 1,
            columns: u16::from(data[3]) + 1,
            output_width,
            output_height,
        })
    }
}
