//! The transformed hmtx table (WOFF 2.0 §5.4), rebuilt as an OpenType hmtx table.
use crate::fonts::sfnt::{FontFileError, Reader, u16_at};

/// hhea.numberOfHMetrics's byte offset.
const HHEA_METRICS_OFFSET: usize = 34;
/// maxp.numGlyphs's byte offset.
const MAXP_GLYPHS_OFFSET: usize = 4;
/// Flags: the proportional glyphs' lsb[] and the monospaced glyphs' leftSideBearing[] are
/// absent, each glyph's xMin standing in; the other bits are reserved.
const NO_PROPORTIONAL_BEARINGS: u8 = 0x01;
const NO_MONOSPACED_BEARINGS: u8 = 0x02;
const DEFINED_FLAGS: u8 = NO_PROPORTIONAL_BEARINGS | NO_MONOSPACED_BEARINGS;

/// The glyph and metric counts the font's hhea and maxp tables give.
#[derive(Clone, Copy)]
pub(super) struct Metrics {
    glyphs: u16,
    advances: u16,
}
impl Metrics {
    pub(super) fn new(hhea: Option<&[u8]>, maxp: Option<&[u8]>) -> Result<Self, FontFileError> {
        match (
            maxp.and_then(|maxp| u16_at(maxp, MAXP_GLYPHS_OFFSET)),
            hhea.and_then(|hhea| u16_at(hhea, HHEA_METRICS_OFFSET)),
        ) {
            (Some(glyphs), Some(advances))
                if advances <= glyphs && (advances > 0 || glyphs == 0) =>
            {
                Ok(Self { glyphs, advances })
            }
            (Some(_), Some(_)) => Err(FontFileError::Hmtx(
                "numberOfHMetrics does not fit the glyph count",
            )),
            _ => Err(FontFileError::Hmtx(
                "the font lacks readable hhea and maxp tables",
            )),
        }
    }
}

/// The hmtx table the transformed table `data` encodes, with the left side bearings it omits
/// taken from the glyphs' `x_mins`.
pub(super) fn reconstruct(
    data: &[u8],
    metrics: Metrics,
    x_mins: &[i16],
) -> Result<Vec<u8>, FontFileError> {
    let mut reader = Reader::new(data, "transformed hmtx table");
    let flags = reader.u8()?;
    if flags & !DEFINED_FLAGS != 0 || flags & DEFINED_FLAGS == 0 {
        return Err(FontFileError::Hmtx("its flags are invalid"));
    }
    let (glyphs, advances) = (usize::from(metrics.glyphs), usize::from(metrics.advances));
    let x_mins = x_mins
        .get(..glyphs)
        .ok_or(FontFileError::Hmtx("glyf has fewer glyphs than maxp"))?;
    let widths = (0..advances)
        .map(|_| reader.u16())
        .collect::<Result<Vec<_>, _>>()?;
    let mut bearings = |omitted: u8, x_mins: &[i16]| {
        if flags & omitted == 0 {
            x_mins.iter().map(|_| reader.i16()).collect()
        } else {
            Ok(x_mins.to_vec())
        }
    };
    let proportional: Vec<_> = bearings(NO_PROPORTIONAL_BEARINGS, &x_mins[..advances])?;
    let monospaced: Vec<_> = bearings(NO_MONOSPACED_BEARINGS, &x_mins[advances..])?;
    if !reader.is_empty() {
        return Err(FontFileError::Layout("transformed hmtx table"));
    }
    let mut hmtx = Vec::new();
    for (width, bearing) in widths.into_iter().zip(proportional) {
        hmtx.extend_from_slice(&width.to_be_bytes());
        hmtx.extend_from_slice(&bearing.to_be_bytes());
    }
    for bearing in monospaced {
        hmtx.extend_from_slice(&bearing.to_be_bytes());
    }
    Ok(hmtx)
}
