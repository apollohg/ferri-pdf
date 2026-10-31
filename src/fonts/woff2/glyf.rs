//! The transformed glyf table and the loca table it implies (WOFF 2.0 §5.1–§5.3), rebuilt
//! as OpenType glyph records (OpenType glyf).
use super::read_255_u16;
use crate::fonts::sfnt::{FontFileError, MAX_DECODED_BYTES, Reader, padded, u16_at};

/// head.indexToLocFormat's byte offset.
const HEAD_INDEX_FORMAT_OFFSET: usize = 50;
const SHORT_FORMAT: u16 = 0;
const LONG_FORMAT: u16 = 1;
const SHORT_OFFSET_LENGTH: usize = 2;
const LONG_OFFSET_LENGTH: usize = 4;
/// Short loca offsets store half the glyf offset.
const SHORT_OFFSET_SCALE: usize = 2;
const STREAM_COUNT: usize = 7;
const OVERLAP_SIMPLE_BITMAP: u16 = 1;
const COMPOSITE: i16 = -1;
/// A glyph header's xMin follows numberOfContours.
const X_MIN_OFFSET: usize = 2;
const BITS_PER_BYTE: usize = 8;
/// bboxBitmap is padded to whole 32-bit words.
const BBOX_BITMAP_WORD_BITS: usize = 32;
const BBOX_BITMAP_WORD_BYTES: usize = 4;
/// The triplet flag bit that marks an off-curve point (§5.2).
const OFF_CURVE: u8 = 0x80;
// Simple glyph flags.
const ON_CURVE: u8 = 0x01;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME_OR_POSITIVE: u8 = 0x10;
const Y_SAME_OR_POSITIVE: u8 = 0x20;
const OVERLAP_SIMPLE: u8 = 0x40;
// Composite glyph component flags and the argument bytes they imply.
const ARGS_ARE_WORDS: u16 = 0x0001;
const HAVE_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const HAVE_X_AND_Y_SCALE: u16 = 0x0040;
const HAVE_TWO_BY_TWO: u16 = 0x0080;
const HAVE_INSTRUCTIONS: u16 = 0x0100;
const COMPONENT_FLAGS_LENGTH: usize = 2;
const GLYPH_INDEX_LENGTH: usize = 2;
const WORD_ARGS_LENGTH: usize = 4;
const BYTE_ARGS_LENGTH: usize = 2;
const SCALE_LENGTH: usize = 2;
const X_AND_Y_SCALE_LENGTH: usize = 4;
const TWO_BY_TWO_LENGTH: usize = 8;

/// The rebuilt glyf and loca tables, with each glyph's xMin for the hmtx transform.
pub(super) struct Glyphs {
    pub glyf: Vec<u8>,
    pub loca: Vec<u8>,
    pub x_mins: Vec<i16>,
    pub index_format: u16,
}

/// head.indexToLocFormat.
pub(super) fn index_format(head: &[u8]) -> Option<u16> {
    u16_at(head, HEAD_INDEX_FORMAT_OFFSET)
}

/// A point as its offset from the previous one.
#[derive(Clone, Copy)]
struct Point {
    dx: i16,
    dy: i16,
    on_curve: bool,
}

/// xMin, yMin, xMax and yMax.
type BoundingBox = [i16; 4];

/// The substreams of a transformed glyf table, read glyph by glyph.
struct Streams<'a> {
    contours: Reader<'a>,
    points: Reader<'a>,
    flags: Reader<'a>,
    glyphs: Reader<'a>,
    composites: Reader<'a>,
    bboxes: Reader<'a>,
    instructions: Reader<'a>,
}

/// The glyf and loca tables the transformed glyf table `data` encodes; `loca_length` is the
/// loca table's original length, which must fit the glyph count and index format (§5.3).
pub(super) fn reconstruct(data: &[u8], loca_length: u32) -> Result<Glyphs, FontFileError> {
    let mut header = Reader::new(data, "transformed glyf header");
    let _reserved = header.u16()?;
    let options = header.u16()?;
    let count = header.u16()?;
    let index_format = header.u16()?;
    let offset_length = match index_format {
        SHORT_FORMAT => SHORT_OFFSET_LENGTH,
        LONG_FORMAT => LONG_OFFSET_LENGTH,
        _ => return Err(FontFileError::Glyf("unknown indexFormat")),
    };
    if loca_length as usize != (usize::from(count) + 1) * offset_length {
        return Err(FontFileError::Glyf(
            "the loca length does not fit the glyph count",
        ));
    }
    let mut sizes = [0; STREAM_COUNT];
    for size in &mut sizes {
        *size = header.u32()? as usize;
    }
    let [
        contours,
        points,
        flags,
        glyphs,
        composites,
        bboxes,
        instructions,
    ] = sizes;
    let mut rest = Reader::new(header.rest(), "transformed glyf streams");
    let mut streams = Streams {
        contours: rest.take(contours, "nContour stream")?,
        points: rest.take(points, "nPoints stream")?,
        flags: rest.take(flags, "flag stream")?,
        glyphs: rest.take(glyphs, "glyph stream")?,
        composites: rest.take(composites, "composite stream")?,
        bboxes: rest.take(bboxes, "bbox stream")?,
        instructions: rest.take(instructions, "instruction stream")?,
    };
    let bbox_bitmap = streams
        .bboxes
        .bytes(usize::from(count).div_ceil(BBOX_BITMAP_WORD_BITS) * BBOX_BITMAP_WORD_BYTES)?;
    let overlap_bitmap = if options & OVERLAP_SIMPLE_BITMAP == 0 {
        None
    } else {
        Some(rest.bytes(usize::from(count).div_ceil(BITS_PER_BYTE))?)
    };
    if !rest.is_empty() {
        return Err(FontFileError::Layout("transformed glyf table"));
    }
    let mut glyf = Vec::new();
    let mut offsets = Vec::with_capacity(usize::from(count) + 1);
    let mut x_mins = Vec::with_capacity(usize::from(count));
    let mut points = Vec::new();
    for glyph in 0..count {
        offsets.push(glyf.len());
        let explicit = bit(bbox_bitmap, glyph);
        let invalid = |reason| FontFileError::Glyph { glyph, reason };
        let x_min = match streams.contours.i16()? {
            0 if explicit => return Err(invalid("an empty glyph has a bounding box")),
            0 => 0,
            COMPOSITE if !explicit => return Err(invalid("a composite has no bounding box")),
            COMPOSITE => streams.composite(&mut glyf)?,
            contours if contours > 0 => {
                let overlap = overlap_bitmap.is_some_and(|bitmap| bit(bitmap, glyph));
                streams.simple(glyph, contours, explicit, overlap, &mut points, &mut glyf)?
            }
            _ => return Err(invalid("its contour count is negative")),
        };
        x_mins.push(x_min);
        glyf.resize(padded(glyf.len()), 0);
        if glyf.len() > MAX_DECODED_BYTES {
            return Err(FontFileError::TooLarge);
        }
    }
    offsets.push(glyf.len());
    let mut loca = Vec::with_capacity(loca_length as usize);
    for offset in offsets {
        if index_format == SHORT_FORMAT {
            let offset = u16::try_from(offset / SHORT_OFFSET_SCALE)
                .map_err(|_| FontFileError::Glyf("glyf is too long for short loca offsets"))?;
            loca.extend_from_slice(&offset.to_be_bytes());
        } else {
            let offset = u32::try_from(offset).map_err(|_| FontFileError::TooLarge)?;
            loca.extend_from_slice(&offset.to_be_bytes());
        }
    }
    Ok(Glyphs {
        glyf,
        loca,
        x_mins,
        index_format,
    })
}

/// The xMin of each glyph of untransformed glyf and loca tables in `index_format`.
pub(super) fn x_mins(
    glyf: &[u8],
    loca: &[u8],
    index_format: Option<u16>,
) -> Result<Vec<i16>, FontFileError> {
    let unreadable = || FontFileError::Hmtx("its glyf and loca tables are unreadable");
    let offsets: Vec<usize> = match index_format {
        Some(SHORT_FORMAT) => loca
            .as_chunks::<SHORT_OFFSET_LENGTH>()
            .0
            .iter()
            .map(|offset| usize::from(u16::from_be_bytes(*offset)) * SHORT_OFFSET_SCALE)
            .collect(),
        Some(LONG_FORMAT) => loca
            .as_chunks::<LONG_OFFSET_LENGTH>()
            .0
            .iter()
            .map(|offset| u32::from_be_bytes(*offset) as usize)
            .collect(),
        _ => return Err(unreadable()),
    };
    offsets
        .windows(2)
        .map(|range| {
            if range[0] == range[1] {
                return Ok(0);
            }
            glyf.get(range[0] + X_MIN_OFFSET..)
                .and_then(<[u8]>::first_chunk)
                .filter(|_| range[0] < range[1])
                .map(|bytes| i16::from_be_bytes(*bytes))
                .ok_or_else(unreadable)
        })
        .collect()
}

/// Whether the bit for `glyph` is set; glyph 0 is the first byte's most significant bit.
fn bit(bitmap: &[u8], glyph: u16) -> bool {
    let glyph = usize::from(glyph);
    bitmap
        .get(glyph / BITS_PER_BYTE)
        .is_some_and(|byte| byte & (0x80 >> (glyph % BITS_PER_BYTE)) != 0)
}

impl Streams<'_> {
    fn bounding_box(&mut self) -> Result<BoundingBox, FontFileError> {
        Ok([
            self.bboxes.i16()?,
            self.bboxes.i16()?,
            self.bboxes.i16()?,
            self.bboxes.i16()?,
        ])
    }

    /// Appends a composite glyph's record to `glyf` and returns its xMin.
    fn composite(&mut self, glyf: &mut Vec<u8>) -> Result<i16, FontFileError> {
        let bounding_box = self.bounding_box()?;
        let components = self.composites.rest();
        let mut length = 0;
        let mut instructed = false;
        loop {
            let flags = self.composites.u16()?;
            let arguments = GLYPH_INDEX_LENGTH
                + if flags & ARGS_ARE_WORDS == 0 {
                    BYTE_ARGS_LENGTH
                } else {
                    WORD_ARGS_LENGTH
                }
                + if flags & HAVE_SCALE != 0 {
                    SCALE_LENGTH
                } else if flags & HAVE_X_AND_Y_SCALE != 0 {
                    X_AND_Y_SCALE_LENGTH
                } else if flags & HAVE_TWO_BY_TWO != 0 {
                    TWO_BY_TWO_LENGTH
                } else {
                    0
                };
            self.composites.bytes(arguments)?;
            length += COMPONENT_FLAGS_LENGTH + arguments;
            instructed |= flags & HAVE_INSTRUCTIONS != 0;
            if flags & MORE_COMPONENTS == 0 {
                break;
            }
        }
        glyf.extend_from_slice(&COMPOSITE.to_be_bytes());
        push_bounding_box(glyf, bounding_box);
        glyf.extend_from_slice(&components[..length]);
        if instructed {
            self.push_instructions(glyf)?;
        }
        Ok(bounding_box[0])
    }

    /// Appends the simple glyph `glyph` of `contours` contours to `glyf` and returns its xMin;
    /// `points` is scratch space.
    fn simple(
        &mut self,
        glyph: u16,
        contours: i16,
        explicit: bool,
        overlap: bool,
        points: &mut Vec<Point>,
        glyf: &mut Vec<u8>,
    ) -> Result<i16, FontFileError> {
        let invalid = |reason| FontFileError::Glyph { glyph, reason };
        let mut end_points = Vec::with_capacity(contours.unsigned_abs().into());
        let mut count = 0_u32;
        for _ in 0..contours {
            let contour = read_255_u16(&mut self.points)?;
            if contour == 0 {
                return Err(invalid("a contour has no points"));
            }
            count += u32::from(contour);
            let Ok(end) = u16::try_from(count - 1) else {
                return Err(invalid("it has more points than glyf can index"));
            };
            end_points.push(end);
        }
        points.clear();
        let (mut x, mut y) = (0_i32, 0_i32);
        let mut computed = [i16::MAX, i16::MAX, i16::MIN, i16::MIN];
        for &flag in self.flags.bytes(count as usize)? {
            let (dx, dy) = triplet(flag & !OFF_CURVE, &mut self.glyphs)?;
            let (Ok(dx), Ok(dy)) = (i16::try_from(dx), i16::try_from(dy)) else {
                return Err(invalid("a coordinate delta exceeds 16 bits"));
            };
            (x, y) = (x + i32::from(dx), y + i32::from(dy));
            let (Ok(x), Ok(y)) = (i16::try_from(x), i16::try_from(y)) else {
                return Err(invalid("a coordinate exceeds 16 bits"));
            };
            computed = [
                computed[0].min(x),
                computed[1].min(y),
                computed[2].max(x),
                computed[3].max(y),
            ];
            points.push(Point {
                dx,
                dy,
                on_curve: flag & OFF_CURVE == 0,
            });
        }
        let bounding_box = if explicit {
            self.bounding_box()?
        } else {
            computed
        };
        glyf.extend_from_slice(&contours.to_be_bytes());
        push_bounding_box(glyf, bounding_box);
        for end in end_points {
            glyf.extend_from_slice(&end.to_be_bytes());
        }
        self.push_instructions(glyf)?;
        push_points(glyf, points, overlap);
        Ok(bounding_box[0])
    }

    /// Appends instructionLength and the instructions it counts.
    fn push_instructions(&mut self, glyf: &mut Vec<u8>) -> Result<(), FontFileError> {
        let length = read_255_u16(&mut self.glyphs)?;
        glyf.extend_from_slice(&length.to_be_bytes());
        glyf.extend_from_slice(self.instructions.bytes(usize::from(length))?);
        Ok(())
    }
}

fn push_bounding_box(glyf: &mut Vec<u8>, bounding_box: BoundingBox) {
    for value in bounding_box {
        glyf.extend_from_slice(&value.to_be_bytes());
    }
}

/// The x and y offsets of one point, from its triplet flag (without the on-curve bit) and the
/// glyph stream bytes it implies: the §5.2 "Triplet Encoding" table, computed as the
/// reference decoder does. Each flag range has one byte count and coordinate layout.
fn triplet(flag: u8, glyphs: &mut Reader<'_>) -> Result<(i32, i32), FontFileError> {
    const X_ONLY: i32 = 10;
    const NIBBLES: i32 = 20;
    const BYTES: i32 = 84;
    const TWELVE_BITS: i32 = 120;
    const WORDS: i32 = 124;
    let flag = i32::from(flag);
    let signed = |flag: i32, magnitude: i32| if flag & 1 == 0 { -magnitude } else { magnitude };
    let mut byte = || glyphs.u8().map(i32::from);
    Ok(match flag {
        0..X_ONLY => (0, signed(flag, ((flag & 0x0e) << 7) + byte()?)),
        X_ONLY..NIBBLES => (signed(flag, (((flag - X_ONLY) & 0x0e) << 7) + byte()?), 0),
        NIBBLES..BYTES => {
            let index = flag - NIBBLES;
            let packed = byte()?;
            (
                signed(flag, 1 + (index & 0x30) + (packed >> 4)),
                signed(flag >> 1, 1 + ((index & 0x0c) << 2) + (packed & 0x0f)),
            )
        }
        BYTES..TWELVE_BITS => {
            let index = flag - BYTES;
            (
                signed(flag, 1 + ((index / 12) << 8) + byte()?),
                signed(flag >> 1, 1 + (((index % 12) >> 2) << 8) + byte()?),
            )
        }
        TWELVE_BITS..WORDS => {
            let (high, packed, low) = (byte()?, byte()?, byte()?);
            (
                signed(flag, (high << 4) + (packed >> 4)),
                signed(flag >> 1, ((packed & 0x0f) << 8) + low),
            )
        }
        _ => {
            let (x_high, x_low, y_high, y_low) = (byte()?, byte()?, byte()?, byte()?);
            (
                signed(flag, (x_high << 8) + x_low),
                signed(flag >> 1, (y_high << 8) + y_low),
            )
        }
    })
}

/// Appends the flags, run-length encoded, then the x and y coordinates of `points`, each in
/// its shortest form, as the reference decoder does.
fn push_points(glyf: &mut Vec<u8>, points: &[Point], overlap: bool) {
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    let mut previous = None;
    let mut repeats = 0_u8;
    for (index, point) in points.iter().enumerate() {
        let mut flag = if point.on_curve { ON_CURVE } else { 0 };
        if overlap && index == 0 {
            flag |= OVERLAP_SIMPLE;
        }
        flag |= push_coordinate(&mut xs, point.dx, X_SHORT, X_SAME_OR_POSITIVE);
        flag |= push_coordinate(&mut ys, point.dy, Y_SHORT, Y_SAME_OR_POSITIVE);
        if previous == Some(flag) && repeats != u8::MAX {
            if let Some(last) = glyf.last_mut() {
                *last |= REPEAT;
            }
            repeats += 1;
        } else {
            if repeats != 0 {
                glyf.push(repeats);
            }
            glyf.push(flag);
            repeats = 0;
        }
        previous = Some(flag);
    }
    if repeats != 0 {
        glyf.push(repeats);
    }
    glyf.extend_from_slice(&xs);
    glyf.extend_from_slice(&ys);
}

/// Appends `delta` in its shortest form and returns the flags describing it.
fn push_coordinate(coordinates: &mut Vec<u8>, delta: i16, short: u8, same_or_positive: u8) -> u8 {
    if delta == 0 {
        return same_or_positive;
    }
    if let Ok(magnitude) = u8::try_from(delta.unsigned_abs()) {
        coordinates.push(magnitude);
        return short | if delta > 0 { same_or_positive } else { 0 };
    }
    coordinates.extend_from_slice(&delta.to_be_bytes());
    0
}
