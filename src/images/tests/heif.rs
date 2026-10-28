use crate::images::{Image, ImageCache};
use ferri_pdf_resources::FileResolver;
use std::{path::PathBuf, sync::Arc};

#[test]
fn lossless_heif_preserves_pixels_alpha_rotation_and_tiled_crop() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/assets");
    let source = image::open(base.join("image-pattern.png"))
        .unwrap()
        .to_rgba8();
    let mut cache = ImageCache::default();
    for (name, expected, has_alpha) in [
        ("image-pattern.heic", source.clone(), true),
        (
            "image-rotated.heic",
            image::imageops::rotate90(&source),
            true,
        ),
        ("image-grid.heic", source, false),
    ] {
        let asset = cache
            .load(name, &base.clone().into(), &FileResolver::default())
            .unwrap();
        assert!(
            Arc::ptr_eq(
                &asset,
                &cache
                    .load(name, &base.clone().into(), &FileResolver::default())
                    .unwrap()
            ),
            "{name}: repeated images must share decoded storage"
        );
        let Image::Pixels(decoded) = &*asset else {
            panic!("{name}: expected decoded pixel asset")
        };
        assert_eq!(
            decoded.size(),
            expected.dimensions(),
            "{name}: display dimensions"
        );
        assert_eq!(
            decoded.alpha().is_some(),
            has_alpha,
            "{name}: alpha presence"
        );
        for (index, pixel) in expected.pixels().enumerate() {
            assert_eq!(
                &decoded.color()[index * 3..index * 3 + 3],
                &pixel.0[..3],
                "{name}: RGB at ({},{})",
                index % expected.width() as usize,
                index / expected.width() as usize
            );
            if let Some(alpha) = decoded.alpha() {
                assert_eq!(alpha[index], pixel.0[3], "{name}: alpha at pixel {index}");
            }
        }
        assert!(
            decoded.icc().is_some(),
            "{name}: retain explicit colour interpretation"
        );
    }
}

#[test]
fn ten_bit_heif_preserves_precision_against_libheif_reference() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/assets");
    let reference = image::open(base.join("image-high-depth-reference.png"))
        .unwrap()
        .to_rgba16();
    let asset = ImageCache::default()
        .load(
            "image-high-depth.heic",
            &base.clone().into(),
            &FileResolver::default(),
        )
        .unwrap();
    let Image::Pixels(decoded) = &*asset else {
        panic!("expected pixel asset")
    };
    assert!(matches!(
        decoded.depth(),
        crate::images::PixelDepth::Sixteen
    ));
    let color: Vec<u16> = decoded
        .color()
        .as_chunks::<{ size_of::<u16>() }>()
        .0
        .iter()
        .copied()
        .map(u16::from_be_bytes)
        .collect();
    let alpha: Vec<u16> = decoded
        .alpha()
        .unwrap()
        .as_chunks::<{ size_of::<u16>() }>()
        .0
        .iter()
        .copied()
        .map(u16::from_be_bytes)
        .collect();
    // libheif left-aligns ten-bit values; the PDF samples span the full u16 range.
    let reference_sample =
        |sample: u16| ((u32::from(sample >> 6) * u32::from(u16::MAX) + 511) / 1023) as u16;
    let mut maximum = 0;
    for (index, pixel) in reference.pixels().enumerate() {
        for channel in 0..3 {
            maximum = maximum
                .max(color[index * 3 + channel].abs_diff(reference_sample(pixel.0[channel])));
        }
        maximum = maximum.max(alpha[index].abs_diff(reference_sample(pixel.0[3])));
    }
    assert!(
        maximum <= 1,
        "10-bit reference differs by {maximum} in 16-bit output"
    );
    assert!(
        color.iter().any(|sample| sample % 257 != 0),
        "must retain precision beyond eight bits"
    );
}
