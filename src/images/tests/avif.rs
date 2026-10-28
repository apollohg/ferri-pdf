use crate::images::{Image, ImageCache};
use crate::resources::FileResolver;
use std::{path::PathBuf, sync::Arc};

#[test]
fn lossless_avif_preserves_rgb_alpha_rotation_and_grid() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/assets");
    let mut cache = ImageCache::default();
    for variant in [
        "pattern",
        "rotated",
        "grid",
        "alpha-grid",
        "mirrored",
        "mirror-first",
    ] {
        let name = format!("image-{variant}.avif");
        let expected = image::open(base.join(format!("image-avif-{variant}-reference.png")))
            .unwrap()
            .to_rgba8();
        let asset = cache
            .load(&name, &base.clone().into(), &FileResolver::default())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(
            Arc::ptr_eq(
                &asset,
                &cache
                    .load(&name, &base.clone().into(), &FileResolver::default())
                    .unwrap()
            ),
            "{name}: decoded cache storage"
        );
        let Image::Pixels(decoded) = &*asset else {
            panic!("{name}: expected pixel asset")
        };
        assert_eq!(
            decoded.size(),
            expected.dimensions(),
            "{name}: display dimensions"
        );
        let alpha = decoded.alpha().expect("AVIF fixtures retain alpha");
        for (index, pixel) in expected.pixels().enumerate() {
            assert_eq!(
                &decoded.color()[index * 3..index * 3 + 3],
                &pixel.0[..3],
                "{name}: RGB at pixel {index}"
            );
            assert_eq!(alpha[index], pixel.0[3], "{name}: alpha at pixel {index}");
        }
        assert!(
            decoded.icc().is_some(),
            "{name}: explicit color interpretation"
        );
    }
}

#[test]
fn ten_bit_avif_matches_independent_samples_without_eight_bit_quantization() {
    const TEN_BIT_MAX: u32 = (1 << 10) - 1;
    const ALIGNMENT_SHIFT: u32 = u16::BITS - 10;
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/assets");
    let expected = image::open(base.join("image-avif-high-depth-reference.png"))
        .unwrap()
        .to_rgba16();
    let asset = ImageCache::default()
        .load(
            "image-high-depth.avif",
            &base.into(),
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
    let reference = |sample: u16| {
        ((u32::from(sample >> ALIGNMENT_SHIFT) * u32::from(u16::MAX) + TEN_BIT_MAX / 2)
            / TEN_BIT_MAX) as u16
    };
    for (index, pixel) in expected.pixels().enumerate() {
        for channel in 0..3 {
            assert!(
                color[index * 3 + channel].abs_diff(reference(pixel.0[channel])) <= 1,
                "RGB pixel {index}, channel {channel}: {} vs {}",
                color[index * 3 + channel],
                reference(pixel.0[channel])
            );
        }
        assert!(
            alpha[index].abs_diff(reference(pixel.0[3])) <= 1,
            "alpha pixel {index}: {} vs {}",
            alpha[index],
            reference(pixel.0[3])
        );
    }
    assert!(
        color.iter().any(|sample| sample % 257 != 0),
        "retained precision beyond eight bits"
    );
}

#[test]
fn embedded_icc_takes_precedence_over_an_earlier_nclx_property() {
    use image::ImageDecoder;
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/assets");
    let mut source = image::ImageReader::open(base.join("image-avif-icc-source.png"))
        .unwrap()
        .into_decoder()
        .unwrap();
    let expected = source.icc_profile().unwrap().unwrap();
    let asset = ImageCache::default()
        .load("image-icc.avif", &base.into(), &FileResolver::default())
        .unwrap();
    let Image::Pixels(decoded) = &*asset else {
        panic!("expected pixel asset")
    };
    assert_eq!(
        decoded.icc().unwrap(),
        expected,
        "retain the exact embedded RGB profile despite nclx preceding it"
    );
}
