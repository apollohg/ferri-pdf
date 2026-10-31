# Image encoding references

`rgb.jpg` and `ycck.jpg` are synthetic gradients generated directly by
libjpeg-turbo 3.2.0. They deliberately use RGB components (Adobe transform 0)
and YCCK components (Adobe transform 2), rather than the usual YCbCr JPEG.
`jpeg-colors.json` records source-generator and file digests, quality, dimensions,
and independent Pillow inspection. No application image or external dataset is
included; the authored generator and its outputs use the project's MIT licence.

Regenerate with `python3 tests/fixtures/images/make-jpeg-colors.py`. This requires
a C compiler, pkg-config and libjpeg-turbo development files at the recorded
version. These tools generate references only; production encoding is pure Rust,
and the PDF tests use the checked-in files without compiling or calling libjpeg.

`tests/image_encoding_pdf.py` separately generates RGB/YCbCr, grayscale and CMYK
sources with Pillow. It compares actual embedded JPEG quantization with Pillow
at the requested quality, measures colour error against independently re-encoded
references, checks ICC bytes and PDF component inversion, verifies all eight EXIF
orientations in Poppler pixels, and checks sharing across HTML/SVG/backgrounds
and border images. Existing default output, alpha and sixteen-bit PNG paths are
checked through the public Rust API.

`photo-48mp.heic` and `photo-48mp.avif` contain synthetic 8000-by-6000 solid RGB
images. Regenerate them with `python3 tests/fixtures/images/make-large-photos.py`
using Pillow and the recorded libheif/x265/AOM versions. The generator also
decodes each file independently with `heif-convert` and records channel extrema,
source dimensions, command arguments and SHA-256 digests in `large-photos.json`.
The compressed fixtures occupy about 12 KiB; full-size pixels are generated only
in temporary storage. They and their generator use the project's MIT licence.

`LargePhotos` in `tests/image_resolution_pdf.py` checks both original-resolution
and DPI-reduced PDFs against these independent samples. Running the regression
requires no HEIF/AVIF encoder or libheif: production decoding remains native Rust.

## Small photo compression fixtures

`make-photo-compression.py` generates synthetic 128×96 RGB gradients as opaque
8-bit and 10-bit HEIC/AVIF, then decodes independent PNG references with libheif.
`photo-compression.json` records tool version, encoder arguments and SHA-256 hashes.
The native tests verify PDF DCT samples, quality tables and precision conversion
against those references across HTML, CSS backgrounds/borders and nested SVG.
Run the generator with Pillow, `heif-enc` and `heif-convert` installed.

## Damaged AV1 data

`av1-frame-error.avif` is `tests/oracle/assets/image-pattern.avif` with byte 460,
inside the colour item's AV1 tile data, XORed with `0xff`. Its frame fails to decode;
`tests/resource_failures.rs` checks that it renders as a broken image.
