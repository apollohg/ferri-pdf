mod codec;
mod container;
mod pixels;

use crate::{
    ImageError, MAX_BYTES_PER_PIXEL, MAX_DECODED_BYTES, PixelImage,
    pixels::{RGB_CHANNELS, color_profile},
};
use avif_decode::Decoder;
use codec::decode_item;
use container::Grid;
pub(crate) use container::matches;
use pixels::{attach_alpha, copy_tile, transform};
use zenavif_parse::{AvifParser, ColorInformation, DecodeConfig};

const PIXEL_LIMIT: u64 = MAX_DECODED_BYTES / MAX_BYTES_PER_PIXEL;
const CONTAINER_LIMIT: u64 = 64 * 1024 * 1024;
const DECODER_THREADS: usize = 1;
const MAX_GRID_TILES: u32 = 1024;
pub(crate) fn decode(bytes: &[u8]) -> Result<PixelImage, ImageError> {
    if bytes.len() as u64 > CONTAINER_LIMIT {
        return Err(ImageError::Avif("container exceeds byte budget"));
    }
    let config = DecodeConfig::default()
        .with_peak_memory_limit(CONTAINER_LIMIT)
        .with_max_grid_tiles(MAX_GRID_TILES);
    let parser = AvifParser::from_bytes_with_config(bytes, &config, &zenavif_parse::Unstoppable)?;
    let mut decoder = Decoder::new(DECODER_THREADS, PIXEL_LIMIT as u32)?;
    let mut budget = PIXEL_LIMIT;
    let mut image = if parser.grid_config().is_some() {
        decode_grid(&mut decoder, &parser, false, &mut budget)?
    } else {
        decode_item(
            &mut decoder,
            &parser.primary_data()?,
            &mut budget,
            parser.color_info().is_none(),
        )?
    };
    image = transform(image, parser.transforms())?;
    if let Some(alpha) = parser.alpha_data() {
        let alpha = if parser.alpha_tile_count() > 0 {
            decode_grid(&mut decoder, &parser, true, &mut budget)?
        } else {
            decode_item(&mut decoder, &alpha?, &mut budget, false)?
        };
        let alpha = transform(alpha, parser.alpha_transforms())?;
        attach_alpha(&mut image, alpha, parser.premultiplied_alpha())?;
    }
    let (icc, primaries, transfer) = match parser.color_info() {
        Some(ColorInformation::IccProfile(icc)) => (Some(icc.as_slice()), 2, 2),
        Some(ColorInformation::Nclx {
            color_primaries,
            transfer_characteristics,
            ..
        }) => (None, *color_primaries, *transfer_characteristics),
        None => (None, 2, 2),
    };
    if parser.color_info().is_some() || image.icc.is_none() {
        image.icc = Some(color_profile(icc, primaries, transfer)?);
    }
    Ok(image)
}

fn decode_grid(
    decoder: &mut Decoder,
    parser: &AvifParser<'_>,
    alpha: bool,
    budget: &mut u64,
) -> Result<PixelImage, ImageError> {
    let descriptor = if alpha {
        parser
            .alpha_data()
            .ok_or(ImageError::Avif("missing alpha grid descriptor"))??
    } else {
        parser.primary_data()?
    };
    let grid = Grid::parse(&descriptor)?;
    let tile_count = if alpha {
        parser.alpha_tile_count()
    } else {
        parser.grid_tile_count()
    };
    check_dimensions(grid.output_width, grid.output_height)?;
    let columns = usize::from(grid.columns);
    let rows = usize::from(grid.rows);
    if columns == 0 || rows == 0 || columns * rows != tile_count {
        return Err(ImageError::Avif(
            "grid tile count does not match its dimensions",
        ));
    }
    let first = decode_tile(decoder, parser, 0, alpha, budget)?;
    let tile_width = first.width;
    let tile_height = first.height;
    let covers = |output: u32, tile: u32, count: usize| {
        u64::from(output) > u64::from(tile) * (count - 1) as u64
            && u64::from(output) <= u64::from(tile) * count as u64
    };
    if !covers(grid.output_width, tile_width, columns)
        || !covers(grid.output_height, tile_height, rows)
    {
        return Err(ImageError::Avif("grid output does not fit its tiles"));
    }
    let mut canvas = PixelImage {
        width: grid.output_width,
        height: grid.output_height,
        depth: first.depth,
        color: vec![
            0;
            grid.output_width as usize
                * grid.output_height as usize
                * RGB_CHANNELS
                * first.depth.bytes()
        ],
        alpha: first.alpha.as_ref().map(|_| {
            vec![0; grid.output_width as usize * grid.output_height as usize * first.depth.bytes()]
        }),
        icc: first.icc.clone(),
    };
    copy_tile(&mut canvas, &first, 0, 0)?;
    for index in 1..tile_count {
        let tile = decode_tile(decoder, parser, index, alpha, budget)?;
        if tile.width != tile_width || tile.height != tile_height {
            return Err(ImageError::Avif("inconsistent grid tile dimensions"));
        }
        copy_tile(
            &mut canvas,
            &tile,
            (index % columns) as u32 * tile_width,
            (index / columns) as u32 * tile_height,
        )?;
    }
    Ok(canvas)
}

fn check_dimensions(width: u32, height: u32) -> Result<(), ImageError> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > PIXEL_LIMIT {
        return Err(ImageError::Dimensions { width, height });
    }
    Ok(())
}

fn decode_tile(
    decoder: &mut Decoder,
    parser: &AvifParser<'_>,
    index: usize,
    alpha: bool,
    budget: &mut u64,
) -> Result<PixelImage, ImageError> {
    if alpha {
        return decode_item(decoder, &parser.alpha_tile_data(index)?, budget, false);
    }
    let mut color = decode_item(
        decoder,
        &parser.tile_data(index)?,
        budget,
        parser.color_info().is_none(),
    )?;
    if let Some(alpha) = parser.tile_alpha_data(index)? {
        let alpha = decode_item(decoder, &alpha, budget, false)?;
        attach_alpha(&mut color, alpha, parser.tile_premultiplied_alpha(index)?)?;
    }
    Ok(color)
}

#[cfg(test)]
mod tests;
