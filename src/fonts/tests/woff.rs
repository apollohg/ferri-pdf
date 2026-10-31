//! WOFF 1.0, WOFF 2.0 and collection decoding against the faces they package
//! (tests/fixtures/fonts/make-woff-fonts.py) and an upstream WPT WOFF2 file.
use crate::fonts::sfnt::{FontFileError, Reader, decode, postscript_fragment};
use crate::fonts::woff2::{base128, read_255_u16};
use std::collections::BTreeMap;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fonts/");
const REGULAR: &str = "WoffTest-Regular.ttf";
const BOLD: &str = "WoffTest-Bold.ttf";
const REGULAR_NAME: &str = "WoffTest-Regular";
const BOLD_NAME: &str = "WoffTest-Bold";
const HEAD: [u8; 4] = *b"head";
/// head.checkSumAdjustment, which depends on the whole file's layout.
const CHECKSUM_ADJUSTMENT: std::ops::Range<usize> = 8..12;
const CHECKSUM_MAGIC: u32 = 0xB1B0_AFBA;
/// WOFF2 encoders set head.flags bit 11, in its high byte, to mark the transform (§5).
const HEAD_FLAGS_HIGH_BYTE: usize = 16;
const LOSSLESS_TRANSFORM_FLAG: u8 = 0x08;
const TRANSFORMED: [[u8; 4]; 3] = [*b"glyf", *b"loca", *b"hmtx"];
const WOFF_RESERVED: usize = 14;
const WOFF_TOTAL_SFNT_SIZE: usize = 16;
const WOFF2_LENGTH: usize = 8;
const WOFF2_DIRECTORY: usize = 48;
const WOFF2_TRANSFORM_VERSION_1: u8 = 0x40;
const OVERLAP_SIMPLE: u8 = 0x40;
const ARGS_ARE_WORDS: u16 = 0x0001;
const HAVE_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const HAVE_X_AND_Y_SCALE: u16 = 0x0040;
const HAVE_TWO_BY_TWO: u16 = 0x0080;
const HAVE_INSTRUCTIONS: u16 = 0x0100;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}{name}")).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn decoded(name: &str, fragment: Option<&str>) -> Vec<u8> {
    decode(fixture(name), fragment).unwrap_or_else(|error| panic!("{name} must decode: {error}"))
}

fn sum(data: &[u8]) -> u32 {
    data.chunks(4)
        .map(|word| {
            let mut padded = [0; 4];
            padded[..word.len()].copy_from_slice(word);
            u32::from_be_bytes(padded)
        })
        .fold(0, u32::wrapping_add)
}

/// Every table of `font` by tag, head without its checkSumAdjustment.
fn tables(font: &[u8]) -> BTreeMap<[u8; 4], Vec<u8>> {
    raw_tables(font)
        .into_iter()
        .map(|(tag, mut table)| {
            if tag == HEAD {
                table[CHECKSUM_ADJUSTMENT].fill(0);
            }
            (tag, table)
        })
        .collect()
}

/// Every table of `font` by tag, as stored.
fn raw_tables(font: &[u8]) -> BTreeMap<[u8; 4], Vec<u8>> {
    let face = ttf_parser::RawFace::parse(font, 0).expect("an sfnt font");
    face.table_records
        .into_iter()
        .map(|record| {
            let start = record.offset as usize;
            let table = font[start..start + record.length as usize].to_vec();
            (record.tag.to_bytes(), table)
        })
        .collect()
}

/// Checks the table checksums and head checkSumAdjustment the sfnt format requires.
fn assert_checksums(case: &str, font: &[u8]) {
    assert_eq!(
        font.len() % 4,
        0,
        "{case}: tables are padded to whole words"
    );
    let face = ttf_parser::RawFace::parse(font, 0).expect("an sfnt font");
    for record in face.table_records {
        let table = &tables(font)[&record.tag.to_bytes()];
        assert_eq!(
            sum(table),
            record.check_sum,
            "{case}: table {} checksum",
            record.tag
        );
    }
    assert_eq!(
        sum(font),
        CHECKSUM_MAGIC,
        "{case}: checkSumAdjustment makes the font sum to 0xB1B0AFBA"
    );
}

#[derive(Default)]
struct Outline(Vec<String>);
impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(format!("M{x},{y}"));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(format!("L{x},{y}"));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.push(format!("Q{x1},{y1},{x},{y}"));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.push(format!("C{x1},{y1},{x2},{y2},{x},{y}"));
    }
    fn close(&mut self) {
        self.0.push("Z".into());
    }
}

/// The raw glyf record of `glyph`.
fn record(font: &[u8], glyph: u16) -> &[u8] {
    let face = ttf_parser::RawFace::parse(font, 0).expect("an sfnt font");
    let table = |tag: &[u8; 4]| face.table(ttf_parser::Tag::from_bytes(tag)).expect("table");
    let long = table(b"head")[50..52] == [0, 1];
    let loca = table(b"loca");
    let offset = |glyph: usize| {
        if long {
            u32::from_be_bytes(loca[glyph * 4..glyph * 4 + 4].try_into().unwrap()) as usize
        } else {
            usize::from(u16::from_be_bytes([loca[glyph * 2], loca[glyph * 2 + 1]])) * 2
        }
    };
    let glyph = usize::from(glyph);
    &table(b"glyf")[offset(glyph)..offset(glyph + 1)]
}

fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([data[at], data[at + 1]])
}

/// A glyph record's instructions and, for a simple glyph, its first point flag.
fn hinting(record: &[u8]) -> (Vec<u8>, Option<u8>) {
    if record.is_empty() {
        return (Vec::new(), None);
    }
    let contours = i16::from_be_bytes([record[0], record[1]]);
    let mut at = 10;
    if let Ok(contours) = usize::try_from(contours) {
        at += 2 * contours;
        let length = usize::from(u16_at(record, at));
        let flag = record.get(at + 2 + length).copied();
        return (record[at + 2..at + 2 + length].to_vec(), flag);
    }
    let mut instructed = false;
    loop {
        let flags = u16_at(record, at);
        at += 4 + if flags & ARGS_ARE_WORDS == 0 { 2 } else { 4 };
        at += if flags & HAVE_SCALE != 0 {
            2
        } else if flags & HAVE_X_AND_Y_SCALE != 0 {
            4
        } else if flags & HAVE_TWO_BY_TWO != 0 {
            8
        } else {
            0
        };
        instructed |= flags & HAVE_INSTRUCTIONS != 0;
        if flags & MORE_COMPONENTS == 0 {
            break;
        }
    }
    if !instructed {
        return (Vec::new(), None);
    }
    let length = usize::from(u16_at(record, at));
    (record[at + 2..at + 2 + length].to_vec(), None)
}

/// Checks that `font` is `expected` glyph for glyph: outlines, bounds, metrics and hinting,
/// with every table other than the transformed ones unchanged.
fn assert_same_font(case: &str, font: &[u8], expected: &[u8]) {
    assert_checksums(case, font);
    let (decoded, original) = (tables(font), tables(expected));
    assert_eq!(
        decoded.keys().collect::<Vec<_>>(),
        original.keys().collect::<Vec<_>>(),
        "{case}: the same tables"
    );
    for (tag, table) in &original {
        if TRANSFORMED.contains(tag) {
            continue;
        }
        let (mut decoded, mut table) = (decoded[tag].clone(), table.clone());
        if *tag == HEAD {
            decoded[HEAD_FLAGS_HIGH_BYTE] &= !LOSSLESS_TRANSFORM_FLAG;
            table[HEAD_FLAGS_HIGH_BYTE] &= !LOSSLESS_TRANSFORM_FLAG;
        }
        assert!(
            decoded == table,
            "{case}: table {} is unchanged",
            tag.escape_ascii()
        );
    }
    let face = ttf_parser::Face::parse(font, 0).expect("the decoded font parses");
    let reference = ttf_parser::Face::parse(expected, 0).expect("the reference font parses");
    assert_eq!(
        face.number_of_glyphs(),
        reference.number_of_glyphs(),
        "{case}"
    );
    for id in 0..reference.number_of_glyphs() {
        let glyph = ttf_parser::GlyphId(id);
        let (mut outline, mut reference_outline) = (Outline::default(), Outline::default());
        let bounds = face.outline_glyph(glyph, &mut outline);
        let reference_bounds = reference.outline_glyph(glyph, &mut reference_outline);
        assert_eq!(outline.0, reference_outline.0, "{case}: glyph {id} outline");
        assert_eq!(bounds, reference_bounds, "{case}: glyph {id} bounds");
        assert_eq!(
            face.glyph_bounding_box(glyph),
            reference.glyph_bounding_box(glyph),
            "{case}: glyph {id} recorded bounding box"
        );
        assert_eq!(
            (
                face.glyph_hor_advance(glyph),
                face.glyph_hor_side_bearing(glyph)
            ),
            (
                reference.glyph_hor_advance(glyph),
                reference.glyph_hor_side_bearing(glyph)
            ),
            "{case}: glyph {id} horizontal metrics"
        );
        let (code, flag) = hinting(record(font, id));
        let (reference_code, reference_flag) = hinting(record(expected, id));
        assert_eq!(code, reference_code, "{case}: glyph {id} instructions");
        assert_eq!(
            flag.map(|flag| flag & OVERLAP_SIMPLE),
            reference_flag.map(|flag| flag & OVERLAP_SIMPLE),
            "{case}: glyph {id} OVERLAP_SIMPLE flag"
        );
    }
}

fn postscript_name(font: &[u8]) -> String {
    ttf_parser::Face::parse(font, 0)
        .expect("an sfnt font")
        .names()
        .into_iter()
        .find(|name| name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME && name.is_unicode())
        .and_then(|name| name.to_string())
        .expect("a PostScript name")
}

#[test]
fn woff_files_decode_to_their_font_with_valid_checksums() {
    let font = decoded("WoffTest-Regular.woff", None);
    let original = fixture(REGULAR);
    assert_same_font("WOFF", &font, &original);
    assert!(
        tables(&font) == tables(&original),
        "WOFF tables are restored byte for byte, transformed or not"
    );
}

#[test]
fn woff2_transforms_rebuild_outlines_bounds_metrics_hinting_and_overlap_flags() {
    let original = fixture(REGULAR);
    let overlap = ttf_parser::Face::parse(&original, 0)
        .unwrap()
        .glyph_index('O')
        .unwrap();
    assert!(
        hinting(record(&original, overlap.0))
            .1
            .is_some_and(|flag| flag & OVERLAP_SIMPLE != 0),
        "the fixture marks O as overlapping"
    );
    for name in ["WoffTest-Regular.woff2", "WoffTest-Regular-NullGlyf.woff2"] {
        assert_same_font(name, &decoded(name, None), &original);
    }
    // An upstream file made by another encoder.
    for name in ["IcTestHalfWidth", "IcTestZeroWidth"] {
        assert_same_font(
            name,
            &decoded(&format!("{name}.woff2"), None),
            &fixture(&format!("{name}.ttf")),
        );
    }
}

#[test]
fn collections_give_the_face_the_fragment_names_or_their_first() {
    for name in ["WoffTest-Collection.woff2", "WoffTest.ttc"] {
        for (fragment, reference, postscript) in [
            (None, REGULAR, REGULAR_NAME),
            (Some(REGULAR_NAME), REGULAR, REGULAR_NAME),
            (Some(BOLD_NAME), BOLD, BOLD_NAME),
        ] {
            let case = format!("{name}#{fragment:?}");
            let font = decoded(name, fragment);
            assert_eq!(postscript_name(&font), postscript, "{case}");
            assert_same_font(&case, &font, &fixture(reference));
        }
        let missing = decode(fixture(name), Some("WoffTest-Italic"));
        assert!(
            matches!(&missing, Err(FontFileError::MissingFace(face)) if face == "WoffTest-Italic"),
            "{name}: a fragment naming no face loads none (CSS Fonts 4 §4.3.2): {missing:?}"
        );
    }
    let single = decoded("WoffTest-Regular.woff2", Some(BOLD_NAME));
    assert_eq!(
        postscript_name(&single),
        REGULAR_NAME,
        "fragments select faces of collections only"
    );
}

#[test]
fn fragments_must_be_possible_postscript_names() {
    for (reference, expected) in [
        ("fonts.ttc", None),
        ("fonts.woff2#WoffTest-Bold", Some("WoffTest-Bold")),
        ("data:font/collection;base64,AAAA#Face", Some("Face")),
        ("fonts.ttc#Caret%5Estick", Some("Caret^stick")),
    ] {
        assert_eq!(
            postscript_fragment(reference).unwrap().as_deref(),
            expected,
            "{reference}"
        );
    }
    let long = format!("fonts.ttc#{}", "N".repeat(64));
    for reference in [
        "fonts.ttc#",
        "fonts.ttc#Two%20Words",
        "fonts.ttc#Slash/Name",
        "fonts.ttc#100%25",
        "fonts.ttc#Caf%C3%A9",
        long.as_str(),
    ] {
        assert!(
            matches!(
                postscript_fragment(reference),
                Err(FontFileError::Fragment(_))
            ),
            "{reference}: a PostScript name is 1-63 printable ASCII characters other than [](){{}}<>/% (RFC 8081 §4.2)"
        );
    }
}

#[test]
fn woff2_integers_reject_non_minimal_and_overlong_encodings() {
    for (bytes, expected) in [
        (&[0x3f][..], Some(63)),
        (&[0x81, 0x00], Some(128)),
        (&[0x8f, 0xff, 0xff, 0xff, 0x7f], Some(u32::MAX)),
        (&[0x80, 0x3f], None),
        (&[0x90, 0x80, 0x80, 0x80, 0x00], None),
        (&[0x81, 0x80, 0x80, 0x80, 0x80, 0x00], None),
        (&[0x81], None),
    ] {
        let value = base128(&mut Reader::new(bytes, "test")).ok();
        assert_eq!(value, expected, "UIntBase128 {bytes:02x?} (WOFF 2.0 §3.1)");
    }
    for (bytes, expected) in [
        (&[252][..], 252),
        (&[255, 0], 253),
        (&[255, 253], 506),
        (&[254, 0], 506),
        (&[253, 1, 250], 506),
        (&[254, 255], 761),
        (&[253, 255, 255], u16::MAX),
    ] {
        let value = read_255_u16(&mut Reader::new(bytes, "test")).ok();
        assert_eq!(value, Some(expected), "255UInt16 {bytes:?} (WOFF 2.0 §3.1)");
    }
}

#[test]
fn malformed_woff_structures_are_rejected() {
    let woff = fixture("WoffTest-Regular.woff");
    let woff2 = fixture("WoffTest-Regular.woff2");
    let set = |data: &[u8], at: usize, bytes: &[u8]| {
        let mut changed = data.to_vec();
        changed[at..at + bytes.len()].copy_from_slice(bytes);
        changed
    };
    let mut extended = woff2.clone();
    extended.push(0);
    let length = u32::try_from(extended.len()).unwrap().to_be_bytes();
    let extended = set(&extended, WOFF2_LENGTH, &length);
    let sfnt_size = u32::from_be_bytes(woff[16..20].try_into().unwrap()) + 4;
    for (case, data, expected) in [
        (
            "WOFF reserved field",
            set(&woff, WOFF_RESERVED, &[0, 1]),
            "WOFF header reserved field is 1, not zero",
        ),
        (
            "WOFF totalSfntSize",
            set(&woff, WOFF_TOTAL_SFNT_SIZE, &sfnt_size.to_be_bytes()),
            "WOFF totalSfntSize",
        ),
        (
            "WOFF trailing byte",
            [woff.as_slice(), &[0]].concat(),
            "font file declares",
        ),
        (
            "WOFF2 data after the private block",
            extended,
            "font file has data outside its blocks, or overlapping blocks, at its end",
        ),
        (
            "WOFF2 unknown transform",
            set(
                &woff2,
                WOFF2_DIRECTORY,
                &[woff2[WOFF2_DIRECTORY] | WOFF2_TRANSFORM_VERSION_1],
            ),
            "has unknown transform version 1",
        ),
    ] {
        let error = decode(data, None).map(|_| ()).unwrap_err().to_string();
        assert!(error.contains(expected), "{case}: {error}");
    }
}
