# HEIC/HEIF image fixtures

These synthetic fixtures were generated locally with libheif 1.23.4 and its
x265 encoder. No codec runs outside Rust in the library; libheif is only an
independent fixture generator/reference decoder.

From the repository root:

```sh
heif-enc --lossless -p chroma=444 -p x265:pools=1 -o tests/oracle/assets/image-pattern.heic tests/oracle/assets/image-pattern.png
heif-enc --lossless -p chroma=444 -p x265:pools=1 --rotate-cw 90 -o tests/oracle/assets/image-rotated.heic tests/oracle/assets/image-pattern.png
heif-enc --lossless -p chroma=444 -p x265:pools=1 --cut-tiles 64 -o tests/oracle/assets/image-grid.heic tests/oracle/assets/image-pattern.png
heif-enc -q 90 -p x265:pools=1 -o tests/oracle/assets/image-lossy.heic tests/oracle/assets/image-pattern.png
heif-enc --lossless -b 10 -p chroma=444 -p x265:pools=1 -o tests/oracle/assets/image-high-depth.heic tests/oracle/assets/image-high-depth.png
heif-convert tests/oracle/assets/image-high-depth.heic tests/oracle/assets/image-high-depth-reference.png
```

The 80×40 sixteen-bit source PNG has channels
`R=floor(x*65535/79)`, `G=floor(y*65535/39)`, `B=23456`,
`A=floor((x+y)*65535/118)`. This deliberately includes precision beyond eight bits.
The libheif reference left-aligns ten-bit samples; the renderer scales them to the
full sixteen-bit PDF range. Tests account for this representation difference and
permit at most one sixteen-bit rounding unit, not one ten-bit quantisation step.

Lossless RGB and alpha compare exactly against the original source PNG, including
rotation. The tiled fixture contains two 64×64 tiles cropped to an 80×40 canvas;
libheif's tile encoder does not attach alpha to this fixture. It is tested as RGB,
not as an alpha-grid coverage claim. The lossy fixture exercises 4:2:0 decoding
and public PDF rendering but does not establish photographic fidelity broadly.

PDF extraction confirms the lossless RGB planes match the source and the
high-depth PDF contains sixteen-bit RGB and soft-mask images. Nested HEIF inside
SVG also exercises the caller's loader and document cache.

HEIC codec correctness uses independent pixel references. Broader
camera/conformance samples and HDR colour descriptions still need validation.

SHA-256 of the checked-in codec fixtures:

- `image-grid.heic`: `e01986c605ce6a52f726129633dca7b2ea45cb9388022e17c613af1a26c66c57`
- `image-high-depth.heic`: `f3164a64054126a11e12b041cb828734a762a80ad2d7ec6bd9203abe9558cf19`
- `image-lossy.heic`: `cf6b810d4efdeadea50e18eda42b07db357fa6ecb3512f24f2b535d4e81aa741`
- `image-pattern.heic`: `415519f2d77931f929f4f64998a0867f092d379746c1a86e1d4c04e55780d950`
- `image-rotated.heic`: `db9f37d83dfa6c7de8804ed7d4355e61117b061c2de06822e2097474821715ac`
