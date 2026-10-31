//! WOFF 2.0 (W3C Recommendation, 2024): a Brotli-compressed stream of sfnt tables, some
//! transformed, packaging one font or a collection.
mod glyf;
mod hmtx;

use crate::fonts::sfnt::{
    Block, COLLECTION_TAG, FontFileError, MAX_DECODED_BYTES, Reader, Tag, build, check_blocks,
    select_face, tag_name,
};
use brotli_decompressor::{BrotliDecompressStream, BrotliResult, BrotliState, StandardAlloc};
use std::borrow::Cow;

const GLYF: Tag = *b"glyf";
const LOCA: Tag = *b"loca";
const HMTX: Tag = *b"hmtx";
const HEAD: Tag = *b"head";
const HHEA: Tag = *b"hhea";
const MAXP: Tag = *b"maxp";
/// The tags table directory flags index (§4.1 "Known Table Tags").
const KNOWN_TAGS: [Tag; 63] = [
    *b"cmap", *b"head", *b"hhea", *b"hmtx", *b"maxp", *b"name", *b"OS/2", *b"post", *b"cvt ",
    *b"fpgm", *b"glyf", *b"loca", *b"prep", *b"CFF ", *b"VORG", *b"EBDT", *b"EBLC", *b"gasp",
    *b"hdmx", *b"kern", *b"LTSH", *b"PCLT", *b"VDMX", *b"vhea", *b"vmtx", *b"BASE", *b"GDEF",
    *b"GPOS", *b"GSUB", *b"EBSC", *b"JSTF", *b"MATH", *b"CBDT", *b"CBLC", *b"COLR", *b"CPAL",
    *b"SVG ", *b"sbix", *b"acnt", *b"avar", *b"bdat", *b"bloc", *b"bsln", *b"cvar", *b"fdsc",
    *b"feat", *b"fmtx", *b"fvar", *b"gvar", *b"hsty", *b"just", *b"lcar", *b"mort", *b"morx",
    *b"opbd", *b"prop", *b"trak", *b"Zapf", *b"Silf", *b"Glat", *b"Gloc", *b"Feat", *b"Sill",
];
/// The flags index that an explicit tag follows.
const ARBITRARY_TAG: u8 = 63;
const TAG_INDEX_MASK: u8 = 0x3f;
const TRANSFORM_SHIFT: u32 = 6;
/// glyf and loca number their null transform 3, other tables 0 (§4.1).
const GLYF_NULL_TRANSFORM: u8 = 3;
const NULL_TRANSFORM: u8 = 0;
const GLYF_TRANSFORM: u8 = 0;
const HMTX_TRANSFORM: u8 = 1;
const HEADER_LENGTH: usize = 48;
/// Collection header versions 1.0 and 2.0 (OpenType §5 "Font Collections").
const COLLECTION_VERSIONS: [u32; 2] = [0x0001_0000, 0x0002_0000];
/// The 255UInt16 codes (§3.1).
const WORD_CODE: u8 = 253;
const ONE_MORE_BYTE_CODE_1: u8 = 255;
const ONE_MORE_BYTE_CODE_2: u8 = 254;
const LOWEST_U_CODE: u16 = 253;
/// `UIntBase128` values take at most five bytes and never start with a zero digit (§3.1).
const BASE128_MAX_BYTES: usize = 5;
const BASE128_CONTINUE: u8 = 0x80;
const BASE128_DIGIT: u8 = 0x7f;
const BASE128_OVERFLOW: u32 = 0xfe00_0000;

#[derive(Clone, Copy, PartialEq)]
enum Transform {
    None,
    Glyf,
    Loca,
    Hmtx,
}

struct Entry {
    tag: Tag,
    transform: Transform,
    original: u32,
    /// Bytes the table occupies in the decompressed stream.
    stored: u32,
}

/// The sfnt font the WOFF2 file `data` packages, or for a collection the face `fragment`
/// names, rejected as the specification requires.
pub(super) fn decode(data: &[u8], fragment: Option<&str>) -> Result<Vec<u8>, FontFileError> {
    let mut reader = Reader::new(data, "WOFF2 header");
    let _signature = reader.u32()?;
    let flavor = reader.tag()?;
    let length = reader.u32()?;
    let count = reader.u16()?;
    // A nonzero reserved field and a wrong totalSfntSize must not reject the file (§3.2).
    let _reserved = reader.u16()?;
    let _total_sfnt_size = reader.u32()?;
    let compressed_length = reader.u32()? as usize;
    let _version = reader.u32()?;
    let metadata = Block {
        offset: reader.u32()?,
        length: reader.u32()?,
    };
    let _metadata_original_length = reader.u32()?;
    let private = Block {
        offset: reader.u32()?,
        length: reader.u32()?,
    };
    if length as usize != data.len() {
        return Err(FontFileError::Length {
            declared: length,
            actual: data.len(),
        });
    }
    let mut reader = Reader::new(&data[HEADER_LENGTH..], "WOFF2 table directory");
    let entries = (0..count)
        .map(|_| entry(&mut reader))
        .collect::<Result<Vec<_>, _>>()?;
    let collection = flavor == COLLECTION_TAG;
    let fonts = if collection {
        reader = Reader::new(reader.rest(), "WOFF2 collection directory");
        collection_directory(&mut reader, &entries)?
    } else {
        vec![(0..count).collect()]
    };
    let start = data.len() - reader.rest().len();
    let end = start
        .checked_add(compressed_length)
        .filter(|end| *end <= data.len())
        .ok_or(FontFileError::Truncated("compressed font data"))?;
    check_blocks(data, end, metadata, private)?;
    let stream_length = entries
        .iter()
        .try_fold(0_usize, |total, entry| {
            total
                .checked_add(entry.stored as usize)
                .filter(|total| *total <= MAX_DECODED_BYTES)
        })
        .ok_or(FontFileError::TooLarge)?;
    let stream = decompress(&data[start..end], stream_length)?;
    let mut stored = Vec::with_capacity(entries.len());
    let mut offset = 0;
    for entry in &entries {
        let length = entry.stored as usize;
        stored.push(&stream[offset..offset + length]);
        offset += length;
    }
    for font in &fonts {
        check_font(font, &entries, collection)?;
    }
    let tables = reconstruct(&fonts, &entries, &stored)?;
    let faces = fonts.iter().map(|font| {
        Ok(font
            .iter()
            .map(|&index| {
                (
                    entries[usize::from(index)].tag,
                    &*tables[usize::from(index)],
                )
            })
            .collect())
    });
    let face = select_face(faces, fragment.filter(|_| collection))?;
    Ok(build(
        face.into_iter()
            .map(|(tag, table)| (tag, Cow::Borrowed(table))),
    ))
}

/// One table directory entry (§4.1).
fn entry(reader: &mut Reader<'_>) -> Result<Entry, FontFileError> {
    let flags = reader.u8()?;
    let index = flags & TAG_INDEX_MASK;
    let tag = if index == ARBITRARY_TAG {
        reader.tag()?
    } else {
        KNOWN_TAGS[usize::from(index)]
    };
    let version = flags >> TRANSFORM_SHIFT;
    let transform = match (tag, version) {
        (GLYF, GLYF_TRANSFORM) => Transform::Glyf,
        (LOCA, GLYF_TRANSFORM) => Transform::Loca,
        (GLYF | LOCA, GLYF_NULL_TRANSFORM) => Transform::None,
        (GLYF | LOCA, _) => {
            return Err(FontFileError::Transform {
                tag: tag_name(tag),
                version,
            });
        }
        (HMTX, HMTX_TRANSFORM) => Transform::Hmtx,
        (_, NULL_TRANSFORM) => Transform::None,
        _ => {
            return Err(FontFileError::Transform {
                tag: tag_name(tag),
                version,
            });
        }
    };
    let original = base128(reader)?;
    let stored = if transform == Transform::None {
        original
    } else {
        base128(reader)?
    };
    if transform == Transform::Loca && stored != 0 {
        return Err(FontFileError::GlyfPair("transformed loca has data"));
    }
    Ok(Entry {
        tag,
        transform,
        original,
        stored,
    })
}

/// The table directory indices of each font of a collection (§4.2).
fn collection_directory(
    reader: &mut Reader<'_>,
    entries: &[Entry],
) -> Result<Vec<Vec<u16>>, FontFileError> {
    if !COLLECTION_VERSIONS.contains(&reader.u32()?) {
        return Err(FontFileError::Collection(
            "unknown collection header version",
        ));
    }
    let count = read_255_u16(reader)?;
    if count == 0 {
        return Err(FontFileError::Collection("it holds no face"));
    }
    (0..count)
        .map(|_| {
            let tables = read_255_u16(reader)?;
            let _flavor = reader.u32()?;
            (0..tables)
                .map(|_| {
                    let index = read_255_u16(reader)?;
                    if usize::from(index) < entries.len() {
                        Ok(index)
                    } else {
                        Err(FontFileError::Collection(
                            "a face names a table the directory lacks",
                        ))
                    }
                })
                .collect()
        })
        .collect()
}

/// Checks that the font `font` of `entries` lists each tag once and pairs its glyf and loca
/// tables, a loca following its glyf, immediately in a collection, both transformed or
/// neither (§4.2, §5.3, §5.5).
fn check_font(font: &[u16], entries: &[Entry], collection: bool) -> Result<(), FontFileError> {
    let mut tags: Vec<_> = font
        .iter()
        .map(|index| entries[usize::from(*index)].tag)
        .collect();
    tags.sort_unstable();
    if let Some(pair) = tags.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(FontFileError::DuplicateTable(tag_name(pair[0])));
    }
    match (
        table_index(font, entries, GLYF),
        table_index(font, entries, LOCA),
    ) {
        (None, None) => Ok(()),
        (Some(glyf), Some(loca)) => {
            let follows = if collection {
                loca == glyf + 1
            } else {
                loca > glyf
            };
            let transformed = entries[glyf].transform == Transform::Glyf;
            if !follows {
                Err(FontFileError::GlyfPair("loca does not follow its glyf"))
            } else if transformed != (entries[loca].transform == Transform::Loca) {
                Err(FontFileError::GlyfPair("only one of them is transformed"))
            } else {
                Ok(())
            }
        }
        _ => Err(FontFileError::GlyfPair("a font has only one of them")),
    }
}

/// The directory index of the table `tag` of `font`.
fn table_index(font: &[u16], entries: &[Entry], tag: Tag) -> Option<usize> {
    font.iter()
        .map(|index| usize::from(*index))
        .find(|index| entries[*index].tag == tag)
}

/// Every table of `entries` with its transform reversed (§5): each transformed glyf, its loca
/// beside it, then each transformed hmtx from the glyph bounds of the first font using it.
fn reconstruct<'a>(
    fonts: &[Vec<u16>],
    entries: &[Entry],
    stored: &[&'a [u8]],
) -> Result<Vec<Cow<'a, [u8]>>, FontFileError> {
    let mut tables: Vec<_> = stored.iter().copied().map(Cow::Borrowed).collect();
    let mut x_mins = vec![None; entries.len()];
    for font in fonts {
        let index = |tag: Tag| table_index(font, entries, tag);
        let (Some(glyf), Some(loca)) = (index(GLYF), index(LOCA)) else {
            continue;
        };
        if entries[glyf].transform == Transform::Glyf && x_mins[glyf].is_none() {
            let glyphs = glyf::reconstruct(stored[glyf], entries[loca].original)?;
            let head = index(HEAD).map(|head| &*tables[head]);
            if head.and_then(glyf::index_format) != Some(glyphs.index_format) {
                return Err(FontFileError::Glyf(
                    "indexFormat differs from the head table's",
                ));
            }
            tables[glyf] = Cow::Owned(glyphs.glyf);
            tables[loca] = Cow::Owned(glyphs.loca);
            x_mins[glyf] = Some(glyphs.x_mins);
        }
    }
    let mut reconstructed_hmtx = vec![false; entries.len()];
    for font in fonts {
        let index = |tag: Tag| table_index(font, entries, tag);
        let Some(hmtx) = index(HMTX).filter(|hmtx| entries[*hmtx].transform == Transform::Hmtx)
        else {
            continue;
        };
        if reconstructed_hmtx[hmtx] {
            continue;
        }
        let table = |tag: Tag| index(tag).map(|index| &*tables[index]);
        let (Some(glyf), Some(loca)) = (index(GLYF), index(LOCA)) else {
            return Err(FontFileError::Hmtx("the font has no glyf table"));
        };
        let x_mins = match &x_mins[glyf] {
            Some(x_mins) => Cow::Borrowed(x_mins.as_slice()),
            None => Cow::Owned(glyf::x_mins(
                &tables[glyf],
                &tables[loca],
                table(HEAD).and_then(glyf::index_format),
            )?),
        };
        let metrics = hmtx::Metrics::new(table(HHEA), table(MAXP))?;
        tables[hmtx] = Cow::Owned(hmtx::reconstruct(stored[hmtx], metrics, &x_mins)?);
        reconstructed_hmtx[hmtx] = true;
    }
    Ok(tables)
}

/// The Brotli stream `compressed`, which must decompress to exactly `length` bytes (§5).
fn decompress(compressed: &[u8], length: usize) -> Result<Vec<u8>, FontFileError> {
    let mut state = BrotliState::new(
        StandardAlloc::default(),
        StandardAlloc::default(),
        StandardAlloc::default(),
    );
    let mut output = vec![0; length];
    let (mut available_in, mut input_offset) = (compressed.len(), 0);
    let (mut available_out, mut output_offset, mut total_out) = (length, 0, 0);
    let result = BrotliDecompressStream(
        &mut available_in,
        &mut input_offset,
        compressed,
        &mut available_out,
        &mut output_offset,
        &mut output,
        &mut total_out,
        &mut state,
    );
    match result {
        BrotliResult::ResultSuccess if available_in == 0 && output_offset == length => Ok(output),
        _ => Err(FontFileError::Brotli),
    }
}

/// A 255UInt16 (§3.1).
pub(super) fn read_255_u16(reader: &mut Reader<'_>) -> Result<u16, FontFileError> {
    Ok(match reader.u8()? {
        WORD_CODE => reader.u16()?,
        ONE_MORE_BYTE_CODE_1 => LOWEST_U_CODE + u16::from(reader.u8()?),
        ONE_MORE_BYTE_CODE_2 => 2 * LOWEST_U_CODE + u16::from(reader.u8()?),
        code => u16::from(code),
    })
}

/// A `UIntBase128` (§3.1), rejected with leading zeros, over five bytes, or over 2³² − 1.
pub(super) fn base128(reader: &mut Reader<'_>) -> Result<u32, FontFileError> {
    let mut value = 0_u32;
    for position in 0..BASE128_MAX_BYTES {
        let byte = reader.u8()?;
        if (position == 0 && byte == BASE128_CONTINUE) || value & BASE128_OVERFLOW != 0 {
            return Err(FontFileError::Base128);
        }
        value = (value << 7) | u32::from(byte & BASE128_DIGIT);
        if byte & BASE128_CONTINUE == 0 {
            return Ok(value);
        }
    }
    Err(FontFileError::Base128)
}
