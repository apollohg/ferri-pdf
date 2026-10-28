use super::*;
use zenavif_parse::AV1Metadata;

const PATTERN: &[u8] = include_bytes!("../../../tests/oracle/assets/image-pattern.avif");
const ALPHA_GRID: &[u8] = include_bytes!("../../../tests/oracle/assets/image-alpha-grid.avif");

#[test]
fn decoder_enforces_coded_frame_limit_before_allocating_pixels() {
    let parser = AvifParser::from_bytes(PATTERN).unwrap();
    let data = parser.primary_data().unwrap();
    let mut decoder = Decoder::new(DECODER_THREADS, 1).unwrap();
    assert!(
        matches!(
            decoder.decode(&data, None, false),
            Err(avif_decode::Error::Rav1d(_))
        ),
        "80x40 coded frame must exceed a one-pixel decoder limit"
    );
}

#[test]
fn repeated_frames_are_charged_and_excessive_frames_are_rejected() {
    let parser = AvifParser::from_bytes(PATTERN).unwrap();
    let data = parser.primary_data().unwrap();
    const SEQUENCE_HEADER_WITH_SIZE: u8 = 10;
    const TEMPORAL_DELIMITER: &[u8] = &[18, 0];
    let sequence = data.strip_prefix(TEMPORAL_DELIMITER).unwrap();
    assert_eq!(sequence[0], SEQUENCE_HEADER_WITH_SIZE);
    assert!(
        sequence[1] < 128,
        "fixture uses a one-byte sequence OBU length"
    );
    let frame = &sequence[2 + usize::from(sequence[1])..];
    let frames = |count: usize| [&data[..], &frame.repeat(count - 1)].concat();
    let metadata = AV1Metadata::parse_av1_bitstream(&data).unwrap();
    let single_frame_pixels =
        u64::from(metadata.max_frame_width.get()) * u64::from(metadata.max_frame_height.get());
    let mut decoder = Decoder::new(DECODER_THREADS, PIXEL_LIMIT as u32).unwrap();
    let error =
        decode_item(&mut decoder, &frames(2), &mut { single_frame_pixels }, true).unwrap_err();
    assert!(
        matches!(error, ImageError::Avif("coded images exceed pixel budget")),
        "{error}"
    );
    let error = decode_item(&mut decoder, &frames(5), &mut { PIXEL_LIMIT }, true).unwrap_err();
    let changing_sequences =
        decode_item(&mut decoder, &data.repeat(2), &mut { PIXEL_LIMIT }, true).unwrap_err();
    assert!(
        matches!(
            changing_sequences,
            ImageError::Avif("multiple sequence headers in AVIF item")
        ),
        "{changing_sequences}"
    );
    assert!(
        matches!(error, ImageError::Avif("too many frames in AVIF item")),
        "{error}"
    );
}

#[test]
fn grid_dimensions_are_checked_before_tile_decode() {
    let mut bytes = ALPHA_GRID.to_vec();
    let idat = bytes.windows(4).position(|part| part == b"idat").unwrap();
    const GRID_DIMENSION_OFFSET: usize = 8;
    bytes[idat + GRID_DIMENSION_OFFSET..idat + GRID_DIMENSION_OFFSET + 4].fill(u8::MAX);
    let error = decode(&bytes).unwrap_err();
    assert!(
        matches!(
            error,
            ImageError::Dimensions {
                width: 65535,
                height: 65535
            }
        ),
        "{error}"
    );
}

fn boxed(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
    const BOX_HEADER_BYTES: usize = 8;
    let mut output = ((payload.len() + BOX_HEADER_BYTES) as u32)
        .to_be_bytes()
        .to_vec();
    output.extend_from_slice(&kind);
    output.extend_from_slice(payload);
    output
}

fn container(meta: &[u8]) -> Vec<u8> {
    let mut output = boxed(*b"ftyp", b"avif\0\0\0\0avif");
    output.extend(boxed(*b"meta", &[&[0; 4], meta].concat()));
    output
}

#[test]
fn repeated_icc_associations_cannot_amplify_metadata_without_limit() {
    const PROFILE_BYTES: usize = 1024 * 1024;
    const ASSOCIATIONS: u32 = 65;
    let mut profile = b"prof".to_vec();
    profile.resize(PROFILE_BYTES, 0);
    let ipco = boxed(*b"ipco", &boxed(*b"colr", &profile));
    let mut ipma = vec![0; 4];
    ipma.extend_from_slice(&ASSOCIATIONS.to_be_bytes());
    for _ in 0..ASSOCIATIONS {
        ipma.extend_from_slice(&[0, 1, 1, 1]);
    }
    let bytes = container(&boxed(*b"iprp", &[ipco, boxed(*b"ipma", &ipma)].concat()));
    let Err(error) = AvifParser::from_bytes(&bytes) else {
        panic!("unbounded ICC copies accepted")
    };
    assert!(
        matches!(error, zenavif_parse::Error::ResourceLimitExceeded(_)),
        "{error}"
    );
}

#[test]
fn zero_width_iloc_records_cannot_expand_unbounded_extent_vectors() {
    const ITEMS: u16 = 64;
    let mut iloc = vec![0; 6];
    iloc.extend_from_slice(&ITEMS.to_be_bytes());
    for id in 1..=ITEMS {
        iloc.extend_from_slice(&id.to_be_bytes());
        iloc.extend_from_slice(&[0; 2]);
        iloc.extend_from_slice(&u16::MAX.to_be_bytes());
    }
    let bytes = container(&boxed(*b"iloc", &iloc));
    let Err(error) = AvifParser::from_bytes(&bytes) else {
        panic!("unbounded extent vectors accepted")
    };
    assert!(
        matches!(error, zenavif_parse::Error::ResourceLimitExceeded(_)),
        "{error}"
    );
}
