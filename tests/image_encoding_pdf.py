"""Inspect real JPEG streams and painted pixels using independent Pillow and Poppler."""
import argparse
import io
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from urllib.parse import quote

from PIL import Image, ImageChops, ImageOps, ImageStat
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
SIZE = (384, 256)
QUALITY = 60
SOURCE_QUALITY = 97
MAX_COLOR_DRIFT = 6
MAX_MEAN_ERROR_OVER_REFERENCE = 2


class ImageCase(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-image-encoding-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)

    def source(self, mode="RGB", orientation=None, progressive=False):
        if mode in ["RGB-coded", "YCCK"]:
            filename = "rgb.jpg" if mode == "RGB-coded" else "ycck.jpg"
            fixture = ROOT / "tests/fixtures/images" / filename
            reference = json.loads(fixture.with_name("jpeg-colors.json").read_text())["files"][filename]
            self.assertEqual(hashlib.sha256(fixture.read_bytes()).hexdigest(), reference["sha256"])
            with Image.open(fixture) as original:
                self.assertEqual(original.info["adobe_transform"], reference["adobe_transform"])
            path = self.output / "source.jpg"
            path.write_bytes(fixture.read_bytes())
            return path
        rgb = Image.new("RGB", SIZE)
        rgb.putdata([((x * 5 + y) % 256, (x + y * 3) % 256, (x * 3 + y * 7) % 256)
                     for y in range(SIZE[1]) for x in range(SIZE[0])])
        original = rgb.convert(mode)
        options = {"quality": SOURCE_QUALITY, "progressive": progressive}
        if mode == "RGB":
            options["icc_profile"] = (ROOT / "vendor/krilla/icc/sRGB-v2-magic.icc").read_bytes()
        if orientation is not None:
            original = Image.new("RGB", SIZE)
            width, height = SIZE
            for box, color in [((0, 0, width // 2, height // 2), "red"), ((width // 2, 0, width, height // 2), "lime"),
                               ((0, height // 2, width // 2, height), "blue"), ((width // 2, height // 2, width, height), "yellow")]:
                original.paste(color, box)
            exif = Image.Exif()
            exif[274] = orientation
            options["exif"] = exif
        path = self.output / "source.jpg"
        original.save(path, **options)
        return path

    def render(self, body, quality=None, arguments=()):
        source, target = self.output / "input.html", self.output / "output.pdf"
        source.write_text("<!doctype html><style>@page{size:360pt 400pt;margin:10pt}html,body{margin:0;padding:0}</style>" + body)
        flags = [] if quality is None else ["--jpeg-quality", str(quality)]
        result = subprocess.run([str(RENDERER), str(source), str(target), *flags, *arguments],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        return pdf_content.Document(target)

    @staticmethod
    def images(document):
        images = []
        for number in document.offsets:
            stream = document.get(number)
            if isinstance(stream, pdf_content.Stream) and stream.entries.get("Subtype") == "Image":
                images.append(stream)
        return images

    def assert_orientation(self, path, width_css_px):
        with Image.open(path) as source:
            expected = ImageOps.exif_transpose(source).convert("RGB")
        width, height = expected.size
        prefix = self.output / "paint"
        subprocess.run(["pdftoppm", "-singlefile", "-r", "96", "-png", str(self.output / "output.pdf"), str(prefix)],
                       check=True, capture_output=True, timeout=60)
        with Image.open(prefix.with_suffix(".png")) as raster:
            page = raster.convert("RGB")
            for x, y in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]:
                color = expected.getpixel((round(width * x), round(height * y)))
                actual = page.getpixel((round(10 * 96 / 72 + width_css_px * x), round(10 * 96 / 72 + width_css_px * height / width * y)))
                self.assertTrue(all(abs(a - b) <= MAX_COLOR_DRIFT for a, b in zip(actual, color)),
                                f"source orientation: quadrant {(x, y)} expected {color}, painted {actual}")
            outside = (round(10 * 96 / 72 + width_css_px + 3), round(10 * 96 / 72 + width_css_px * height / width / 2))
            self.assertEqual(page.getpixel(outside), (255, 255, 255), "encoding cannot enlarge the painted box")


class ImageEncoding(ImageCase):
    def test_large_jpeg_axes_preserve_images_with_and_without_optimisation(self):
        for size in [(17000, 16), (16, 17000)]:
            with self.subTest(size=size):
                path = self.output / "wide.jpg"
                with Image.new("RGB", size, (19, 93, 201)) as image:
                    image.save(path, quality=SOURCE_QUALITY)
                body = '<p>Before</p><img src="wide.jpg" style="width:72pt;height:72pt;object-fit:contain"><p>After</p>'
                for arguments in [(), ("--image-dpi", "150")]:
                    images = self.images(self.render(body, QUALITY if arguments else None, arguments))
                    self.assertEqual(len(images), 1)
                    if not arguments:
                        self.assertEqual(images[0].raw, path.read_bytes())
                    with Image.open(io.BytesIO(images[0].raw)) as decoded:
                        decoded.load()
                        self.assertTrue(all(abs(a - b) <= MAX_COLOR_DRIFT for a, b in
                                            zip(decoded.convert("RGB").getpixel((0, 0)), (19, 93, 201))))

    def test_failed_strict_jpeg_optimisation_preserves_original_stream_and_dimensions(self):
        path = self.source()
        original = path.read_bytes()
        segment_end = 4 + int.from_bytes(original[4:6], "big")
        variants = {
            "truncated": original[:len(original) * 3 // 5],
            "padded": original[:segment_end] + bytes(16) + original[segment_end:],
        }
        body = '<p>Before</p><img src="source.jpg" style="width:72pt;height:48pt"><p>After</p>'
        for name, damaged in variants.items():
            path.write_bytes(damaged)
            for arguments in [(), ("--image-dpi", "150")]:
                with self.subTest(source=name, arguments=arguments):
                    images = self.images(self.render(body, QUALITY, arguments))
                    self.assertEqual(len(images), 1)
                    self.assertEqual(images[0].raw, damaged, "failed optional encoding must preserve the original stream")
                    self.assertEqual((images[0].entries["Width"], images[0].entries["Height"]), SIZE,
                                     "fallback bytes must never carry the requested reduced dimensions")

    def test_gif_frames_above_the_default_decoder_allowance_remain_embedded(self):
        size = (3600, 3500)
        path = self.output / "large.gif"
        with Image.new("P", size, 1) as image:
            image.putpalette([0, 0, 0, 19, 93, 201] + [0] * (256 * 3 - 6))
            image.save(path)
        body = '<img src="large.gif" style="width:72pt;height:70pt">'
        for arguments in [(), ("--image-dpi", "150")]:
            with self.subTest(arguments=arguments):
                document = self.render(body, QUALITY, arguments)
                images = self.images(document)
                main = next(image for image in images if image.entries.get("ColorSpace") != "DeviceGray")
                expected = (150, 146) if arguments else size
                self.assertEqual((main.entries["Width"], main.entries["Height"]), expected)
                samples = document.decode(main)
                self.assertEqual(len(samples), expected[0] * expected[1] * 3)
                self.assertEqual(samples[:3], bytes((19, 93, 201)))
                self.assertEqual(samples[-3:], bytes((19, 93, 201)))

    def test_heic_and_avif_photos_use_requested_jpeg_quality_across_painting_consumers(self):
        fixtures = ROOT / "tests/fixtures/images"
        records = json.loads((fixtures / "photo-compression.json").read_text())["files"]
        for name, metadata in records.items():
            source = fixtures / name
            self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), metadata["sha256"])
            reference_path = fixtures / metadata["reference"]
            self.assertEqual(hashlib.sha256(reference_path.read_bytes()).hexdigest(), metadata["reference_sha256"])
            (self.output / name).write_bytes(source.read_bytes())
            svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="48">'
                   f'<image href="{name}" width="64" height="48"/></svg>')
            consumers = {
                "html": f'<img src="{name}" style="width:24pt;height:18pt">',
                "background": f'<div style="width:24pt;height:18pt;background:url({name}) 0 0/100% 100%"></div>',
                "border": f'<div style="width:24pt;height:18pt;border:4pt solid;border-image:url({name}) 4 fill"></div>',
                "svg": f'<img src="data:image/svg+xml,{quote(svg)}" style="width:24pt;height:18pt">',
            }
            for consumer, body in consumers.items():
                with self.subTest(codec=name, consumer=consumer):
                    baseline = self.render(body)
                    reference = self.images(baseline)
                    self.assertEqual(len(reference), 1)
                    original = reference[0]
                    self.assertEqual(original.entries["BitsPerComponent"], 16 if "10bit" in name else 8)
                    self.assertNotEqual(original.entries.get("Filter"), "DCTDecode")
                    for arguments in [(), ("--image-dpi", "36")]:
                        with self.subTest(arguments=arguments):
                            images = self.images(self.render(body, QUALITY, arguments))
                            self.assertEqual(len(images), 1)
                            stream = images[0]
                            self.assertEqual(stream.entries.get("Filter"), "DCTDecode")
                            self.assertEqual(stream.entries["BitsPerComponent"], 8)
                            with Image.open(io.BytesIO(stream.raw)) as actual:
                                actual.load()
                                self.assertEqual(actual.size, (stream.entries["Width"], stream.entries["Height"]))
                                comparison = io.BytesIO()
                                Image.new("RGB", (1, 1)).save(comparison, format="JPEG", quality=QUALITY)
                                with Image.open(comparison) as expected:
                                    self.assertEqual(actual.quantization[0], expected.quantization[0])
                                if not arguments:
                                    self.assertEqual(actual.size, (original.entries["Width"], original.entries["Height"]))
                                    with Image.open(reference_path) as independent:
                                        samples = independent.convert("RGB")
                                    encoded_reference = io.BytesIO()
                                    samples.save(encoded_reference, format="JPEG", quality=QUALITY)
                                    with Image.open(encoded_reference) as expected:
                                        error = ImageStat.Stat(ImageChops.difference(actual.convert("RGB"), samples)).mean
                                        reference_error = ImageStat.Stat(ImageChops.difference(expected.convert("RGB"), samples)).mean
                                        self.assertTrue(all(a <= b + MAX_MEAN_ERROR_OVER_REFERENCE
                                                            for a, b in zip(error, reference_error)))

    def test_transparent_heic_avif_and_ordinary_png_stay_lossless_with_jpeg_quality(self):
        fixtures = ROOT / "tests/oracle/assets"
        for name in ("image-pattern.png", "image-high-depth.heic", "image-high-depth.avif", "image-alpha-grid.avif"):
            with self.subTest(source=name):
                (self.output / name).write_bytes((fixtures / name).read_bytes())
                body = f'<img src="{name}" style="width:24pt;height:18pt">'
                baseline = self.render(body)
                original = [(image.entries, image.raw) for image in self.images(baseline)]
                output = self.render(body, QUALITY)
                self.assertEqual([(image.entries, image.raw) for image in self.images(output)], original,
                                 "JPEG quality must not alter lossless precision, alpha or ordinary PNG sources")

    def test_quality_controls_rgb_grayscale_and_cmyk_streams_with_original_profiles(self):
        cases = [(mode, False) for mode in ["RGB", "L", "CMYK", "RGB-coded", "YCCK"]]
        cases.extend((mode, True) for mode in ["RGB", "L", "CMYK"])
        for mode, progressive in cases:
            with self.subTest(mode=mode, progressive=progressive):
                path = self.source(mode, progressive=progressive)
                body = '<img src="source.jpg" style="width:288pt;height:192pt">'
                original = self.images(self.render(body))
                self.assertEqual(len(original), 1)
                self.assertEqual(original[0].raw, path.read_bytes(), "default must pass the source JPEG through byte-exactly")
                document = self.render(body, QUALITY)
                images = self.images(document)
                self.assertEqual(len(images), 1)
                stream = images[0]
                self.assertEqual(stream.entries["Filter"], "DCTDecode")
                self.assertEqual(stream.entries.get("Decode"), original[0].entries.get("Decode"),
                                 "PDF component inversion must retain the original colour interpretation")
                if mode in ["CMYK", "YCCK"]:
                    self.assertEqual(stream.entries["ColorSpace"], original[0].entries["ColorSpace"])
                with Image.open(path) as source, Image.open(io.BytesIO(stream.raw)) as encoded:
                    self.assertEqual((stream.entries["Width"], stream.entries["Height"]), source.size)
                    expected = io.BytesIO()
                    source.save(expected, format="JPEG", quality=QUALITY)
                    with Image.open(expected) as reference:
                        self.assertEqual(encoded.quantization[0], reference.quantization[0], "serialized quality must use the independently selected quantization table")
                        baseline_error = ImageStat.Stat(ImageChops.difference(source.convert("RGB"), reference.convert("RGB"))).mean
                        actual_error = ImageStat.Stat(ImageChops.difference(source.convert("RGB"), encoded.convert("RGB"))).mean
                        for actual, baseline in zip(actual_error, baseline_error):
                            self.assertLessEqual(actual, baseline + MAX_MEAN_ERROR_OVER_REFERENCE,
                                                 f"{mode}: colour error must stay within the independently measured quality-{QUALITY} budget")
                    self.assertEqual(encoded.mode, source.mode)
                    self.assertLess(len(stream.raw), len(path.read_bytes()), "quality 60 must reduce this independently generated quality-97 source")
                    if mode == "RGB":
                        color_space = document.resolve(stream.entries["ColorSpace"])
                        self.assertEqual(color_space[0], "ICCBased")
                        self.assertEqual(document.decode(document.resolve(color_space[1])), source.info["icc_profile"])

    def test_shared_sources_across_html_svg_backgrounds_and_borders_encode_once(self):
        self.source()
        svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="48">'
               '<image href="source.jpg" width="64" height="48"/></svg>')
        body = ('<style>img{width:48pt;height:36pt}section{width:48pt;height:36pt;'
                'background:url(source.jpg) 0 0/24pt 18pt repeat;border:8pt solid transparent;'
                'border-image:url(source.jpg) 20 fill stretch}</style><img src="source.jpg">'
                '<img src="source.jpg"><section></section>'
                f'<img src="data:image/svg+xml,{quote(svg)}">')
        images = self.images(self.render(body, QUALITY))
        self.assertEqual(len(images), 1, "one shared source must retain one JPEG XObject across all painting consumers")
        with Image.open(io.BytesIO(images[0].raw)) as encoded:
            reference = io.BytesIO()
            Image.new("RGB", (1, 1)).save(reference, format="JPEG", quality=QUALITY)
            with Image.open(reference) as expected:
                self.assertEqual(encoded.quantization[0], expected.quantization[0])

    def test_reencoding_preserves_all_eight_source_orientations_in_painted_pixels(self):
        for orientation in range(1, 9):
            with self.subTest(orientation=orientation):
                path = self.source(orientation=orientation)
                self.render('<img src="source.jpg" style="display:block;width:288px;height:auto">', QUALITY)
                self.assert_orientation(path, 288)

    def test_unavailable_and_undecodable_images_keep_the_pdf_and_valid_images(self):
        with Image.new("RGB", (16, 16), (19, 93, 201)) as source:
            source.save(self.output / "valid.png")
        data = bytearray((self.output / "valid.png").read_bytes())
        start = data.index(b"IDAT") + len(b"IDAT")
        data[start] ^= 255
        (self.output / "corrupt.png").write_bytes(data)
        (self.output / "invalid.png").write_bytes(b"not an image")
        nested = '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><image href="corrupt.png" width="16" height="16"/></svg>'
        for source in ("missing.png", "invalid.png", "corrupt.png", "data:image/svg+xml," + quote(nested)):
            with self.subTest(source=source):
                document = self.render(f'<p>Before</p><img src="{source}"><img src="{source}">'
                                       '<img src="valid.png" width="16" height="16"><p>After</p>', QUALITY)
                images = self.images(document)
                self.assertEqual(len(images), 1, "omit only the failed image")
                self.assertEqual((images[0].entries["Width"], images[0].entries["Height"]), (16, 16))
                text = "".join(glyph.text for glyph in pdf_content.inspect(self.output / "output.pdf")["glyphs"])
                self.assertIn("Before", text)
                self.assertIn("After", text)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
