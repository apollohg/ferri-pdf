//! MIME Sniffing: the computed MIME type of a fetched resource.
use crate::resources::ResourceMetadata;

/// Bytes the sniffing tables examine (MIME Sniffing §5.2).
const RESOURCE_HEADER_BYTES: usize = 1445;
const WHITESPACE: &[u8] = b"\t\n\x0C\r ";
const TAG_TERMINATING: &[u8] = b" >";
/// The ASCII case-insensitive mask bit of a pattern byte (§6).
const CASE_INSENSITIVE: u8 = 0xDF;
const SIGNIFICANT: u8 = 0xFF;

pub(crate) const HTML: &str = "text/html";
pub(crate) const PLAIN_TEXT: &str = "text/plain";
pub(crate) const OCTET_STREAM: &str = "application/octet-stream";
pub(crate) const SVG: &str = "image/svg+xml";
const XML: &str = "text/xml";
const APPLICATION_XML: &str = "application/xml";
const XML_SUFFIX: &str = "+xml";
const JSON_SUFFIX: &str = "+json";
const JSON_TYPES: &[&str] = &["application/json", "text/json"];
const UNKNOWN_TYPES: &[&str] = &["unknown/unknown", "application/unknown", "*/*"];
pub(crate) const IMAGE_PREFIX: &str = "image/";
const AUDIO_PREFIX: &str = "audio/";
const VIDEO_PREFIX: &str = "video/";
/// MIME Sniffing §4.6 JavaScript MIME type essences.
const JAVASCRIPT_TYPES: &[&str] = &[
    "application/ecmascript",
    "application/javascript",
    "application/x-ecmascript",
    "application/x-javascript",
    "text/ecmascript",
    "text/javascript",
    "text/javascript1.0",
    "text/javascript1.1",
    "text/javascript1.2",
    "text/javascript1.3",
    "text/javascript1.4",
    "text/javascript1.5",
    "text/jscript",
    "text/livescript",
    "text/x-ecmascript",
    "text/x-javascript",
];
/// `Content-Type` values some Apache servers send for unrecognized files (§5.1).
const APACHE_BUG_CHARSETS: &[&str] = &["ISO-8859-1", "iso-8859-1", "UTF-8"];

/// MIME types the file system gives local files; the HTML Standard leaves file metadata to it
/// (MIME Sniffing §5.1 "retrieved directly from the file system").
const FILE_EXTENSION_TYPES: &[(&str, &str)] = &[
    ("htm", HTML),
    ("html", HTML),
    ("xhtml", "application/xhtml+xml"),
    ("xml", XML),
    ("svg", SVG),
    ("txt", PLAIN_TEXT),
    ("css", "text/css"),
    ("js", "text/javascript"),
    ("mjs", "text/javascript"),
    ("json", "application/json"),
    ("vtt", "text/vtt"),
    ("png", "image/png"),
    ("apng", "image/apng"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("avif", "image/avif"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("bmp", "image/bmp"),
    ("ico", "image/x-icon"),
    ("pdf", "application/pdf"),
    ("mp4", "video/mp4"),
    ("webm", "video/webm"),
    ("ogg", "application/ogg"),
    ("mp3", "audio/mpeg"),
    ("wav", "audio/wave"),
];

type Row = (&'static [u8], &'static [u8], &'static str);

/// §7.1 scriptable patterns after leading whitespace: HTML tags, matched case-insensitively and
/// followed by a tag-terminating byte, and the XML declaration.
const SCRIPTABLE: &[(&str, bool, &str)] = &[
    ("<!DOCTYPE HTML", true, HTML),
    ("<HTML", true, HTML),
    ("<HEAD", true, HTML),
    ("<SCRIPT", true, HTML),
    ("<IFRAME", true, HTML),
    ("<H1", true, HTML),
    ("<DIV", true, HTML),
    ("<FONT", true, HTML),
    ("<TABLE", true, HTML),
    ("<A", true, HTML),
    ("<STYLE", true, HTML),
    ("<TITLE", true, HTML),
    ("<B", true, HTML),
    ("<BODY", true, HTML),
    ("<BR", true, HTML),
    ("<P", true, HTML),
    ("<!--", true, HTML),
    ("<?xml", false, XML),
];
const PDF_SIGNATURE: &[u8] = b"%PDF-";
const PDF: &str = "application/pdf";

const NON_SCRIPTABLE: &[Row] = &[
    (b"%!PS-Adobe-", &[SIGNIFICANT; 11], "application/postscript"),
    (b"\xFE\xFF\0\0", b"\xFF\xFF\0\0", PLAIN_TEXT),
    (b"\xFF\xFE\0\0", b"\xFF\xFF\0\0", PLAIN_TEXT),
    (b"\xEF\xBB\xBF\0", b"\xFF\xFF\xFF\0", PLAIN_TEXT),
];

/// §6.1 image type patterns.
const IMAGES: &[Row] = &[
    (b"\0\0\x01\0", &[SIGNIFICANT; 4], "image/x-icon"),
    (b"\0\0\x02\0", &[SIGNIFICANT; 4], "image/x-icon"),
    (b"BM", &[SIGNIFICANT; 2], "image/bmp"),
    (b"GIF87a", &[SIGNIFICANT; 6], "image/gif"),
    (b"GIF89a", &[SIGNIFICANT; 6], "image/gif"),
    (
        b"RIFF\0\0\0\0WEBPVP",
        b"\xFF\xFF\xFF\xFF\0\0\0\0\xFF\xFF\xFF\xFF\xFF\xFF",
        "image/webp",
    ),
    (b"\x89PNG\r\n\x1A\n", &[SIGNIFICANT; 8], "image/png"),
    (b"\xFF\xD8\xFF", &[SIGNIFICANT; 3], "image/jpeg"),
];

const RIFF_MASK: &[u8] = b"\xFF\xFF\xFF\xFF\0\0\0\0\xFF\xFF\xFF\xFF";
/// §6.2 audio and video type patterns.
const MEDIA: &[Row] = &[
    (b"FORM\0\0\0\0AIFF", RIFF_MASK, "audio/aiff"),
    (b"ID3", &[SIGNIFICANT; 3], MP3),
    (b"OggS\0", &[SIGNIFICANT; 5], "application/ogg"),
    (b"MThd\0\0\0\x06", &[SIGNIFICANT; 8], "audio/midi"),
    (b"RIFF\0\0\0\0AVI ", RIFF_MASK, "video/avi"),
    (b"RIFF\0\0\0\0WAVE", RIFF_MASK, "audio/wave"),
];
const MP3: &str = "audio/mpeg";
const MP4: &str = "video/mp4";
const WEBM: &str = "video/webm";

/// §6.4 archive type patterns.
const ARCHIVES: &[Row] = &[
    (b"\x1F\x8B\x08", &[SIGNIFICANT; 3], "application/x-gzip"),
    (b"PK\x03\x04", &[SIGNIFICANT; 4], "application/zip"),
    (
        b"Rar!\x1A\x07\0",
        &[SIGNIFICANT; 7],
        "application/x-rar-compressed",
    ),
];

const MP4_BOX: &[u8] = b"ftyp";
const MP4_BRAND: &[u8] = b"mp4";
const MP4_MINIMUM_BYTES: usize = 12;
const MP4_MAJOR_BRAND_END: usize = 16;
const BOX_ALIGNMENT: u32 = 4;
const WEBM_SIGNATURE: &[u8] = b"\x1A\x45\xDF\xA3";
const WEBM_DOCTYPE_ELEMENT: &[u8] = b"\x42\x82";
const WEBM_DOCTYPE: &[u8] = b"webm";
const WEBM_SCAN_END: usize = 38;
const VINT_MAX_LENGTH: usize = 8;
const VINT_FIRST_MASK: u8 = 0x80;
const MP3_HEADER_BYTES: usize = 4;
const MP3_RATES: [u32; 15] = [
    0, 32_000, 40_000, 48_000, 56_000, 64_000, 80_000, 96_000, 112_000, 128_000, 160_000, 192_000,
    224_000, 256_000, 320_000,
];
const MP25_RATES: [u32; 15] = [
    0, 8000, 16_000, 24_000, 32_000, 40_000, 48_000, 56_000, 64_000, 80_000, 96_000, 112_000,
    128_000, 144_000, 160_000,
];
const SAMPLE_RATES: [u32; 3] = [44_100, 48_000, 32_000];
const MP3_LAYER: u8 = 3;
const MP3_VERSION_1_SCALE: u32 = 72;
const MP3_SCALE: u32 = 144;

/// The type `metadata` declares for the resource at `url`, if any: its `Content-Type`, or what
/// the file system gives a local file.
pub(crate) fn supplied_type(url: &url::Url, metadata: &ResourceMetadata) -> Option<String> {
    if let Some(media_type) = &metadata.media_type {
        return Some(media_type.clone());
    }
    if url.scheme() != "file" {
        return None;
    }
    let extension = url.path().rsplit_once('.')?.1.to_ascii_lowercase();
    FILE_EXTENSION_TYPES
        .iter()
        .find(|(name, _)| *name == extension)
        .map(|(_, media_type)| (*media_type).to_owned())
}

/// §5.1: an HTTP `text/plain` declaration that old Apache servers send for every unknown file.
pub(crate) fn apache_bug(url: &url::Url, metadata: &ResourceMetadata) -> bool {
    matches!(url.scheme(), "http" | "https")
        && metadata.media_type.as_deref() == Some(PLAIN_TEXT)
        && metadata
            .charset
            .as_deref()
            .is_none_or(|charset| APACHE_BUG_CHARSETS.contains(&charset))
}

pub(crate) fn is_xml(essence: &str) -> bool {
    essence == XML || essence == APPLICATION_XML || essence.ends_with(XML_SUFFIX)
}
pub(crate) fn is_json(essence: &str) -> bool {
    JSON_TYPES.contains(&essence) || essence.ends_with(JSON_SUFFIX)
}
pub(crate) fn is_javascript(essence: &str) -> bool {
    JAVASCRIPT_TYPES.contains(&essence)
}

/// §7 MIME type sniffing algorithm, with the no-sniff flag unset. `supported_image` and
/// `supported_media` report which supplied image and audio/video types the user agent decodes.
pub(crate) fn computed_type(
    supplied: Option<&str>,
    apache_bug: bool,
    bytes: &[u8],
    supported_image: impl Fn(&str) -> bool,
    supported_media: impl Fn(&str) -> bool,
) -> String {
    let header = &bytes[..bytes.len().min(RESOURCE_HEADER_BYTES)];
    let Some(supplied) = supplied.filter(|essence| !UNKNOWN_TYPES.contains(essence)) else {
        return unknown(header).to_owned();
    };
    if is_xml(supplied) || supplied == HTML {
        return supplied.to_owned();
    }
    if apache_bug {
        return text_or_binary(header).to_owned();
    }
    if supplied.starts_with(IMAGE_PREFIX)
        && supported_image(supplied)
        && let Some(matched) = image_type(header)
    {
        return matched.to_owned();
    }
    if (supplied.starts_with(AUDIO_PREFIX) || supplied.starts_with(VIDEO_PREFIX))
        && supported_media(supplied)
        && let Some(matched) = media_type(header)
    {
        return matched.to_owned();
    }
    supplied.to_owned()
}

/// §6 pattern matching algorithm for patterns that ignore no leading bytes.
fn matches(input: &[u8], pattern: &[u8], mask: &[u8]) -> bool {
    input.get(..pattern.len()).is_some_and(|input| {
        input
            .iter()
            .zip(pattern)
            .zip(mask)
            .all(|((byte, pattern), mask)| byte & mask == *pattern)
    })
}

fn table(input: &[u8], rows: &[Row]) -> Option<&'static str> {
    rows.iter()
        .find(|(pattern, mask, _)| matches(input, pattern, mask))
        .map(|row| row.2)
}

fn scriptable(input: &[u8], tag: &str, html_tag: bool) -> bool {
    let start = input
        .iter()
        .position(|byte| !WHITESPACE.contains(byte))
        .unwrap_or(input.len());
    let Some(rest) = input.get(start..) else {
        return false;
    };
    let tag = tag.as_bytes();
    let Some(candidate) = rest.get(..tag.len()) else {
        return false;
    };
    let mask = |byte: u8| {
        if html_tag && byte.is_ascii_alphabetic() {
            CASE_INSENSITIVE
        } else {
            SIGNIFICANT
        }
    };
    candidate
        .iter()
        .zip(tag)
        .all(|(byte, expected)| byte & mask(*expected) == *expected)
        && (!html_tag
            || rest
                .get(tag.len())
                .is_some_and(|byte| TAG_TERMINATING.contains(byte)))
}

/// §7.1 rules for identifying an unknown MIME type, with the sniff-scriptable flag set.
fn unknown(header: &[u8]) -> &'static str {
    if let Some((_, _, media_type)) = SCRIPTABLE
        .iter()
        .find(|(tag, html_tag, _)| scriptable(header, tag, *html_tag))
    {
        return media_type;
    }
    if header.starts_with(PDF_SIGNATURE) {
        return PDF;
    }
    table(header, NON_SCRIPTABLE)
        .or_else(|| image_type(header))
        .or_else(|| media_type(header))
        .or_else(|| table(header, ARCHIVES))
        .unwrap_or_else(|| text_or_binary_tail(header))
}

/// §7.2 rules for distinguishing if a resource is text or binary.
pub(crate) fn text_or_binary(header: &[u8]) -> &'static str {
    if header.starts_with(b"\xFE\xFF")
        || header.starts_with(b"\xFF\xFE")
        || header.starts_with(b"\xEF\xBB\xBF")
    {
        return PLAIN_TEXT;
    }
    text_or_binary_tail(header)
}

fn text_or_binary_tail(header: &[u8]) -> &'static str {
    if header.iter().any(|byte| is_binary_data(*byte)) {
        OCTET_STREAM
    } else {
        PLAIN_TEXT
    }
}

fn is_binary_data(byte: u8) -> bool {
    matches!(byte, 0x00..=0x08 | 0x0B | 0x0E..=0x1A | 0x1C..=0x1F)
}

/// §6.1 image type pattern matching algorithm.
pub(crate) fn image_type(header: &[u8]) -> Option<&'static str> {
    table(header, IMAGES)
}

/// §6.2 audio or video type pattern matching algorithm.
fn media_type(header: &[u8]) -> Option<&'static str> {
    table(header, MEDIA).or_else(|| {
        if is_mp4(header) {
            Some(MP4)
        } else if is_webm(header) {
            Some(WEBM)
        } else if is_mp3_without_id3(header) {
            Some(MP3)
        } else {
            None
        }
    })
}

/// §6.2.1 signature for MP4.
fn is_mp4(sequence: &[u8]) -> bool {
    if sequence.len() < MP4_MINIMUM_BYTES {
        return false;
    }
    let box_size = u32::from_be_bytes([sequence[0], sequence[1], sequence[2], sequence[3]]);
    if (sequence.len() as u64) < u64::from(box_size) || !box_size.is_multiple_of(BOX_ALIGNMENT) {
        return false;
    }
    if &sequence[4..8] != MP4_BOX {
        return false;
    }
    if &sequence[8..11] == MP4_BRAND {
        return true;
    }
    (MP4_MAJOR_BRAND_END..box_size as usize)
        .step_by(BOX_ALIGNMENT as usize)
        .any(|offset| sequence.get(offset..offset + MP4_BRAND.len()) == Some(MP4_BRAND))
}

/// §6.2.2 signature for `WebM`.
fn is_webm(sequence: &[u8]) -> bool {
    let length = sequence.len();
    if !sequence.starts_with(WEBM_SIGNATURE) {
        return false;
    }
    let mut iter = WEBM_SIGNATURE.len();
    while iter < length && iter < WEBM_SCAN_END {
        if sequence.get(iter..iter + WEBM_DOCTYPE_ELEMENT.len()) == Some(WEBM_DOCTYPE_ELEMENT) {
            iter += WEBM_DOCTYPE_ELEMENT.len();
            if iter >= length {
                return false;
            }
            iter += vint_size(&sequence[iter..]);
            if iter + WEBM_DOCTYPE.len() >= length {
                return false;
            }
            if padded_match(sequence, iter, WEBM_DOCTYPE) {
                return true;
            }
        }
        iter += 1;
    }
    false
}

/// The number size of the EBML variable-length integer at the start of `sequence`.
fn vint_size(sequence: &[u8]) -> usize {
    let mut mask = VINT_FIRST_MASK;
    let mut size = 1;
    while size < VINT_MAX_LENGTH && size < sequence.len() {
        if sequence[0] & mask != 0 {
            break;
        }
        mask >>= 1;
        size += 1;
    }
    size
}

/// `pattern` at `offset`, optionally preceded there by NUL padding.
fn padded_match(sequence: &[u8], offset: usize, pattern: &[u8]) -> bool {
    let rest = &sequence[offset..];
    let padding = rest.iter().take_while(|byte| **byte == 0).count();
    rest[padding..].starts_with(pattern)
}

/// §6.2.3 signature for MP3 without ID3.
fn is_mp3_without_id3(sequence: &[u8]) -> bool {
    if !mp3_header(sequence, 0) {
        return false;
    }
    let skipped = mp3_frame_size(sequence, 0);
    if skipped < MP3_HEADER_BYTES || skipped > sequence.len() {
        return false;
    }
    mp3_header(sequence, skipped)
}

fn mp3_header(sequence: &[u8], offset: usize) -> bool {
    let Some(header) = sequence.get(offset..offset + MP3_HEADER_BYTES) else {
        return false;
    };
    if header[0] != 0xFF || header[1] & 0xE0 != 0xE0 {
        return false;
    }
    let layer = (header[1] & 0x06) >> 1;
    let bit_rate = (header[2] & 0xF0) >> 4;
    let sample_rate = (header[2] & 0x0C) >> 2;
    layer != 0
        && bit_rate != 15
        && usize::from(sample_rate) < SAMPLE_RATES.len()
        && 4 - layer == MP3_LAYER
}

fn mp3_frame_size(sequence: &[u8], offset: usize) -> usize {
    let version = (sequence[offset + 1] & 0x18) >> 3;
    let bitrate_index = usize::from((sequence[offset + 2] & 0xF0) >> 4);
    let bitrate = if version & 0x01 == 0 {
        MP3_RATES[bitrate_index]
    } else {
        MP25_RATES[bitrate_index]
    };
    let mut sample_rate = SAMPLE_RATES[usize::from((sequence[offset + 2] & 0x0C) >> 2)];
    if version == 2 {
        sample_rate /= 2;
    } else if version == 0 {
        sample_rate /= 4;
    }
    let pad = (sequence[offset + 2] & 0x02) >> 1;
    let scale = if version == 1 {
        MP3_VERSION_1_SCALE
    } else {
        MP3_SCALE
    };
    (bitrate * scale / sample_rate + u32::from(pad)) as usize
}
