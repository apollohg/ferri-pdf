//! Font files as `@font-face` sources deliver them: bare sfnt fonts, collections, and the
//! WOFF 1.0 and WOFF 2.0 packagings, each turned into the one sfnt font the source names.
use crate::fonts::{woff, woff2};
use std::borrow::Cow;

pub(super) type Tag = [u8; 4];

const WOFF_SIGNATURE: &[u8; 4] = b"wOFF";
const WOFF2_SIGNATURE: &[u8; 4] = b"wOF2";
pub(super) const COLLECTION_TAG: Tag = *b"ttcf";
const NAME_TABLE: Tag = *b"name";
/// sfnt tables and WOFF blocks start on 4-byte boundaries.
const ALIGNMENT: usize = 4;
/// Bounds the memory a small compressed file can claim once decoded.
pub(super) const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;
/// PostScript names are at most 63 printable ASCII characters other than these (RFC 8081
/// §4.2, after OpenType `name` ID 6).
const POSTSCRIPT_NAME_MAX_LENGTH: usize = 63;
const POSTSCRIPT_NAME_FORBIDDEN: &[u8] = b"[](){}<>/%";
const FRAGMENT_SEPARATOR: char = '#';

/// Why a font file holds no usable font.
#[derive(Debug, thiserror::Error)]
pub enum FontFileError {
    #[error("font file ends inside its {0}")]
    Truncated(&'static str),
    #[error("font file declares {declared} bytes but holds {actual}")]
    Length { declared: u32, actual: usize },
    #[error("font file has data outside its blocks, or overlapping blocks, at its {0}")]
    Layout(&'static str),
    #[error("decoded font would exceed {MAX_DECODED_BYTES} bytes")]
    TooLarge,
    #[error("font file lists table {0} more than once")]
    DuplicateTable(String),
    #[error("WOFF header reserved field is {0}, not zero")]
    Reserved(u16),
    #[error("WOFF totalSfntSize is {declared}, but its tables need {computed}")]
    SfntSize { declared: u32, computed: u64 },
    #[error("WOFF table {0} is not a zlib stream of its original length")]
    Zlib(String),
    #[error("WOFF2 table directory has an invalid UIntBase128 length")]
    Base128,
    #[error("WOFF2 font data is not a Brotli stream of the table directory's total length")]
    Brotli,
    #[error("WOFF2 table {tag} has unknown transform version {version}")]
    Transform { tag: String, version: u8 },
    #[error("WOFF2 glyf and loca tables are not a transformed pair: {0}")]
    GlyfPair(&'static str),
    #[error("WOFF2 transformed glyf table is invalid: {0}")]
    Glyf(&'static str),
    #[error("WOFF2 glyph {glyph} is invalid: {reason}")]
    Glyph { glyph: u16, reason: &'static str },
    #[error("WOFF2 transformed hmtx table is invalid: {0}")]
    Hmtx(&'static str),
    #[error("font collection is invalid: {0}")]
    Collection(&'static str),
    #[error("fragment {0:?} cannot be a PostScript name")]
    Fragment(String),
    #[error("font collection has no face whose PostScript name is {0:?}")]
    MissingFace(String),
}

/// The tag as text, for diagnostics.
pub(super) fn tag_name(tag: Tag) -> String {
    tag.escape_ascii().to_string()
}

/// The big-endian `uint16` at `offset` in `table`.
pub(super) fn u16_at(table: &[u8], offset: usize) -> Option<u16> {
    table
        .get(offset..)
        .and_then(<[u8]>::first_chunk)
        .map(|bytes| u16::from_be_bytes(*bytes))
}

/// `length` rounded up to the next 4-byte boundary.
pub(super) fn padded(length: usize) -> usize {
    length.next_multiple_of(ALIGNMENT)
}

/// The face of `data` that the `@font-face` source with `fragment` names, as a bare sfnt font:
/// WOFF and WOFF2 files decoded (WOFF 1.0, WOFF 2.0), a face of a collection chosen by the
/// fragment (CSS Fonts 4 §4.3.2); other data is returned for font parsing to judge.
pub(super) fn decode(data: Vec<u8>, fragment: Option<&str>) -> Result<Vec<u8>, FontFileError> {
    match data.first_chunk::<4>() {
        Some(WOFF_SIGNATURE) => woff::decode(&data),
        Some(WOFF2_SIGNATURE) => woff2::decode(&data, fragment),
        Some(&COLLECTION_TAG) => collection_face(&data, fragment),
        _ => Ok(data),
    }
}

/// The PostScript name the fragment of the source URL `reference` names, if it has one;
/// a fragment that cannot be one means the source is skipped unfetched (CSS Fonts 4 §4.3.2).
/// Characters that URLs cannot carry are percent-escaped (RFC 8081 §4.2).
pub(super) fn postscript_fragment(reference: &str) -> Result<Option<String>, FontFileError> {
    let Some((_, fragment)) = reference.split_once(FRAGMENT_SEPARATOR) else {
        return Ok(None);
    };
    let name = percent_encoding::percent_decode_str(fragment).collect::<Vec<u8>>();
    let valid = !name.is_empty()
        && name.len() <= POSTSCRIPT_NAME_MAX_LENGTH
        && name
            .iter()
            .all(|byte| byte.is_ascii_graphic() && !POSTSCRIPT_NAME_FORBIDDEN.contains(byte));
    if valid {
        Ok(Some(String::from_utf8_lossy(&name).into_owned()))
    } else {
        Err(FontFileError::Fragment(fragment.to_owned()))
    }
}

/// A face's tables by tag.
pub(super) type FaceTables<'a> = Vec<(Tag, &'a [u8])>;

/// The face of a collection that `fragment` names by PostScript name, or its first face
/// without one (CSS Fonts 4 §4.3.2; RFC 8081 §4.2).
pub(super) fn select_face<'a>(
    faces: impl IntoIterator<Item = Result<FaceTables<'a>, FontFileError>>,
    fragment: Option<&str>,
) -> Result<FaceTables<'a>, FontFileError> {
    for face in faces {
        let face = face?;
        let Some(name) = fragment else {
            return Ok(face);
        };
        let names = face
            .iter()
            .find(|(tag, _)| *tag == NAME_TABLE)
            .and_then(|(_, data)| ttf_parser::name::Table::parse(data));
        if names.is_some_and(|names| {
            names
                .names
                .into_iter()
                .any(|record| is_named(&record, name))
        }) {
            return Ok(face);
        }
    }
    match fragment {
        Some(name) => Err(FontFileError::MissingFace(name.to_owned())),
        None => Err(FontFileError::Collection("it holds no face")),
    }
}

/// Whether `record` is a PostScript name equal to `name`. Macintosh records are Mac Roman,
/// which PostScript names' ASCII shares.
fn is_named(record: &ttf_parser::name::Name<'_>, name: &str) -> bool {
    if record.name_id != ttf_parser::name_id::POST_SCRIPT_NAME {
        return false;
    }
    if record.is_unicode() {
        record.to_string().is_some_and(|value| value == name)
    } else {
        record.platform_id == ttf_parser::PlatformId::Macintosh && record.name == name.as_bytes()
    }
}

/// The face of the TrueType or OpenType collection `data` that `fragment` names.
fn collection_face(data: &[u8], fragment: Option<&str>) -> Result<Vec<u8>, FontFileError> {
    let count = ttf_parser::fonts_in_collection(data)
        .ok_or(FontFileError::Truncated("collection header"))?;
    let faces = (0..count).map(|index| {
        let face = ttf_parser::RawFace::parse(data, index)
            .map_err(|_| FontFileError::Collection("a face's table directory is unreadable"))?;
        face.table_records
            .into_iter()
            .map(|record| {
                let start = record.offset as usize;
                data.get(start..start.saturating_add(record.length as usize))
                    .map(|table| (record.tag.to_bytes(), table))
                    .ok_or(FontFileError::Truncated("collection face's tables"))
            })
            .collect()
    });
    Ok(build(
        select_face(faces, fragment)?
            .into_iter()
            .map(|(tag, table)| (tag, Cow::Borrowed(table))),
    ))
}

/// An sfnt font of `tables`, with its table directory, checksums and `head`
/// checkSumAdjustment computed (OpenType §5 "Calculating checksums").
pub(super) fn build<'a>(tables: impl IntoIterator<Item = (Tag, Cow<'a, [u8]>)>) -> Vec<u8> {
    let mut builder = write_fonts::FontBuilder::new();
    for (tag, data) in tables {
        builder.add_raw(write_fonts::types::Tag::new(&tag), data);
    }
    builder.build()
}

/// Offset and length of a WOFF extended metadata or private data block; both zero when absent.
#[derive(Clone, Copy)]
pub(super) struct Block {
    pub offset: u32,
    pub length: u32,
}

/// Checks that the optional metadata and private blocks follow the font data ending at `end`
/// on 4-byte boundaries, in that order, and that the file ends with the last: WOFF 1.0 §3, §7
/// and §8, which WOFF 2.0 §3, §6 and §7 share. Only null padding may come between.
pub(super) fn check_blocks(
    data: &[u8],
    mut end: usize,
    metadata: Block,
    private: Block,
) -> Result<(), FontFileError> {
    const METADATA: &str = "metadata block";
    const PRIVATE: &str = "private data block";
    let mut private_last = false;
    for (block, what) in [(metadata, METADATA), (private, PRIVATE)] {
        if block.offset == 0 && block.length == 0 {
            continue;
        }
        let start = padded(end);
        if block.offset as usize != start || block.length == 0 {
            return Err(FontFileError::Layout(what));
        }
        check_padding(data, end..start, what)?;
        end = start
            .checked_add(block.length as usize)
            .filter(|end| *end <= data.len())
            .ok_or(FontFileError::Truncated(what))?;
        private_last = what == PRIVATE;
    }
    let file_end = if private_last { end } else { padded(end) };
    if data.len() < end || data.len() > file_end {
        return Err(FontFileError::Layout("end"));
    }
    check_padding(data, end..data.len(), "end")
}

/// Checks that the bytes of `data` in `range` are null padding.
pub(super) fn check_padding(
    data: &[u8],
    range: std::ops::Range<usize>,
    what: &'static str,
) -> Result<(), FontFileError> {
    match data.get(range) {
        Some(padding) if padding.iter().all(|byte| *byte == 0) => Ok(()),
        Some(_) => Err(FontFileError::Layout(what)),
        None => Err(FontFileError::Truncated(what)),
    }
}

/// Reads the big-endian fields of one structure of a font file.
pub(super) struct Reader<'a> {
    data: &'a [u8],
    /// The structure being read, which a truncation error names.
    what: &'static str,
}
impl<'a> Reader<'a> {
    pub(super) fn new(data: &'a [u8], what: &'static str) -> Self {
        Self { data, what }
    }
    pub(super) fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    /// The unread bytes.
    pub(super) fn rest(&self) -> &'a [u8] {
        self.data
    }
    pub(super) fn bytes(&mut self, count: usize) -> Result<&'a [u8], FontFileError> {
        let (bytes, rest) = self
            .data
            .split_at_checked(count)
            .ok_or(FontFileError::Truncated(self.what))?;
        self.data = rest;
        Ok(bytes)
    }
    /// The next `count` bytes as a reader of the structure `what`.
    pub(super) fn take(
        &mut self,
        count: usize,
        what: &'static str,
    ) -> Result<Reader<'a>, FontFileError> {
        Ok(Reader::new(self.bytes(count)?, what))
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], FontFileError> {
        let (bytes, rest) = self
            .data
            .split_first_chunk::<N>()
            .ok_or(FontFileError::Truncated(self.what))?;
        self.data = rest;
        Ok(*bytes)
    }
    pub(super) fn u8(&mut self) -> Result<u8, FontFileError> {
        self.array().map(u8::from_be_bytes)
    }
    pub(super) fn u16(&mut self) -> Result<u16, FontFileError> {
        self.array().map(u16::from_be_bytes)
    }
    pub(super) fn i16(&mut self) -> Result<i16, FontFileError> {
        self.array().map(i16::from_be_bytes)
    }
    pub(super) fn u32(&mut self) -> Result<u32, FontFileError> {
        self.array().map(u32::from_be_bytes)
    }
    pub(super) fn tag(&mut self) -> Result<Tag, FontFileError> {
        self.array()
    }
}
