//! WOFF 1.0 (W3C Recommendation, 2012): zlib-compressed sfnt tables.
use crate::fonts::sfnt::{
    Block, FontFileError, MAX_DECODED_BYTES, Reader, Tag, build, check_blocks, check_padding,
    padded, tag_name,
};
use std::borrow::Cow;

const HEADER_LENGTH: usize = 44;
const ENTRY_LENGTH: usize = 20;
/// The sfnt offset table and one table record (OpenType §5).
const SFNT_HEADER_LENGTH: u64 = 12;
const SFNT_RECORD_LENGTH: u64 = 16;
const TABLES: &str = "font tables";

struct Entry {
    tag: Tag,
    offset: usize,
    compressed: usize,
    original: usize,
}

/// The sfnt font the WOFF file `data` packages, rejected as the specification requires.
pub(super) fn decode(data: &[u8]) -> Result<Vec<u8>, FontFileError> {
    let mut header = Reader::new(data, "WOFF header");
    let _signature = header.u32()?;
    let _flavor = header.u32()?;
    let length = header.u32()?;
    let count = usize::from(header.u16()?);
    let reserved = header.u16()?;
    let total_sfnt_size = header.u32()?;
    let _version = header.u32()?;
    let metadata = Block {
        offset: header.u32()?,
        length: header.u32()?,
    };
    let _metadata_original_length = header.u32()?;
    let private = Block {
        offset: header.u32()?,
        length: header.u32()?,
    };
    if length as usize != data.len() {
        return Err(FontFileError::Length {
            declared: length,
            actual: data.len(),
        });
    }
    if reserved != 0 {
        return Err(FontFileError::Reserved(reserved));
    }
    let mut directory = Reader::new(
        data.get(HEADER_LENGTH..).unwrap_or_default(),
        "WOFF table directory",
    );
    let mut entry = || -> Result<Entry, FontFileError> {
        let entry = Entry {
            tag: directory.tag()?,
            offset: directory.u32()? as usize,
            compressed: directory.u32()? as usize,
            original: directory.u32()? as usize,
        };
        let _checksum = directory.u32()?;
        Ok(entry)
    };
    let mut entries = (0..count).map(|_| entry()).collect::<Result<Vec<_>, _>>()?;
    entries.sort_unstable_by_key(|entry| entry.tag);
    if let Some(pair) = entries.windows(2).find(|pair| pair[0].tag == pair[1].tag) {
        return Err(FontFileError::DuplicateTable(tag_name(pair[0].tag)));
    }
    let computed = entries.iter().fold(
        SFNT_HEADER_LENGTH + SFNT_RECORD_LENGTH * count as u64,
        |total, entry| total + padded(entry.original) as u64,
    );
    if computed != u64::from(total_sfnt_size) {
        return Err(FontFileError::SfntSize {
            declared: total_sfnt_size,
            computed,
        });
    }
    if computed > MAX_DECODED_BYTES as u64 {
        return Err(FontFileError::TooLarge);
    }
    // Tables follow the directory in file order, each starting at the 4-byte boundary after
    // the last, with only null padding between (§3, §6).
    entries.sort_unstable_by_key(|entry| entry.offset);
    let mut end = HEADER_LENGTH + ENTRY_LENGTH * count;
    for entry in &entries {
        let start = padded(end);
        if entry.offset != start {
            return Err(FontFileError::Layout(TABLES));
        }
        check_padding(data, end..start, TABLES)?;
        end = start
            .checked_add(entry.compressed)
            .filter(|end| *end <= data.len())
            .ok_or(FontFileError::Truncated(TABLES))?;
    }
    check_blocks(data, end, metadata, private)?;
    let tables = entries
        .iter()
        .map(|entry| {
            let stored = &data[entry.offset..entry.offset + entry.compressed];
            let table = match entry.compressed.cmp(&entry.original) {
                std::cmp::Ordering::Equal => Cow::Borrowed(stored),
                std::cmp::Ordering::Less => Cow::Owned(
                    inflate(stored, entry.original)
                        .ok_or_else(|| FontFileError::Zlib(tag_name(entry.tag)))?,
                ),
                std::cmp::Ordering::Greater => {
                    return Err(FontFileError::Zlib(tag_name(entry.tag)));
                }
            };
            Ok((entry.tag, table))
        })
        .collect::<Result<Vec<_>, FontFileError>>()?;
    Ok(build(tables))
}

/// The zlib stream `compressed` decompressed, if it is exactly one stream of `length` bytes.
fn inflate(compressed: &[u8], length: usize) -> Option<Vec<u8>> {
    let mut inflater = flate2::Decompress::new(true);
    let mut output = Vec::with_capacity(length);
    let status = inflater
        .decompress_vec(compressed, &mut output, flate2::FlushDecompress::Finish)
        .ok()?;
    (status == flate2::Status::StreamEnd
        && output.len() == length
        && inflater.total_in() == compressed.len() as u64)
        .then_some(output)
}
