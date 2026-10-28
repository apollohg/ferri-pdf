"""Check physical raster resolution through serialized PDF images, never layout traces."""
import argparse
import hashlib
import io
import json
import math
from pathlib import Path
import struct
import subprocess
import unittest
import zlib
from urllib.parse import quote

from PIL import Image, ImageChops, ImageStat
import image_encoding_pdf as encoding

DPI = 150


class ImageResolution(encoding.ImageCase):
    def rendered_images(self, body, dpi=DPI):
        document = self.render(body, encoding.QUALITY, ["--image-dpi", str(dpi)])
        return document, self.images(document)

    def assert_dimensions(self, images, expected):
        self.assertEqual(len(images), len(expected), "shared input must retain one raster per distinct source")
        self.assertCountEqual([(image.entries["Width"], image.entries["Height"]) for image in images], expected)
        for image in images:
            if image.entries.get("Filter") == "DCTDecode":
                with Image.open(io.BytesIO(image.raw)) as decoded:
                    self.assertEqual(decoded.size, (image.entries["Width"], image.entries["Height"]),
                                     "the actual JPEG samples must match the reduced PDF dimensions")

    def test_resample_budget_falls_back_to_original_samples_and_dimensions(self):
        size = (1170, 73000)
        color = (19, 93, 201)
        with Image.new("RGB", size, color) as image:
            image.save(self.output / "scroll.png")
        document = self.render('<p>Before</p><img src="scroll.png" '
                               'style="width:510pt;height:72pt;object-fit:cover"><p>After</p>',
                               encoding.QUALITY, ("--image-dpi", "150"))
        images = self.images(document)
        self.assertEqual(len(images), 1, "budget fallback must retain the image")
        main = images[0]
        self.assertEqual((main.entries["Width"], main.entries["Height"]), size,
                         "unresampled samples require the original dimensions")
        samples = document.decode(main)
        self.assertEqual(len(samples), size[0] * size[1] * len(color))
        self.assertEqual(samples[:3], bytes(color))
        self.assertEqual(samples[-3:], bytes(color))

    def test_reduced_jpeg_colour_modes_preserve_quality_profiles_and_pixels(self):
        cases = [(mode, False) for mode in ["RGB", "L", "CMYK", "RGB-coded", "YCCK"]]
        cases.extend((mode, True) for mode in ["RGB", "L", "CMYK"])
        target = (25, 19)
        for mode, progressive in cases:
            with self.subTest(mode=mode, progressive=progressive):
                path = self.source(mode, progressive=progressive)
                document, images = self.rendered_images('<img src="source.jpg" style="width:12pt;height:9pt">')
                self.assert_dimensions(images, [target])
                stream = images[0]
                self.assertEqual(stream.entries["Filter"], "DCTDecode")
                with Image.open(path) as source, Image.open(io.BytesIO(stream.raw)) as actual:
                    resized = source.resize(target, Image.Resampling.BICUBIC)
                    reference_bytes = io.BytesIO()
                    resized.save(reference_bytes, format="JPEG", quality=encoding.QUALITY)
                    with Image.open(reference_bytes) as reference:
                        self.assertEqual(actual.mode, source.mode)
                        self.assertEqual(actual.quantization[0], reference.quantization[0])
                        if mode == "CMYK":
                            for table in actual.quantization.values():
                                self.assertEqual(table, reference.quantization[0], "ink channels must retain luminance detail")
                        baseline_error = ImageStat.Stat(ImageChops.difference(resized.convert("RGB"), reference.convert("RGB"))).mean
                        actual_error = ImageStat.Stat(ImageChops.difference(resized.convert("RGB"), actual.convert("RGB"))).mean
                        for observed, baseline in zip(actual_error, baseline_error):
                            self.assertLessEqual(observed, baseline + encoding.MAX_MEAN_ERROR_OVER_REFERENCE,
                                                 f"{mode}: resized colour error exceeds independent quality budget")
                    if mode == "RGB":
                        color_space = document.resolve(stream.entries["ColorSpace"])
                        self.assertEqual(color_space[0], "ICCBased")
                        self.assertEqual(document.decode(document.resolve(color_space[1])), source.info["icc_profile"])

    def test_invalid_dpi_rejects_before_overwriting_output(self):
        source, target = self.output / "input.html", self.output / "output.pdf"
        source.write_text("<p>Resolution validation</p>")
        preserved = b"existing PDF content"
        for value in ["0", "-1", "NaN", "inf", "-inf", "1e100", "invalid"]:
            with self.subTest(dpi=value):
                target.write_bytes(preserved)
                result = subprocess.run([str(encoding.RENDERER), str(source), str(target), "--image-dpi", value],
                                        capture_output=True, text=True, timeout=60)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("--image-dpi", result.stderr)
                self.assertEqual(target.read_bytes(), preserved)

    def test_largest_shared_use_survives_both_orders_and_page_boundaries(self):
        self.source()
        small = '<img src="source.jpg" style="display:block;width:72pt;height:48pt">'
        large = '<img src="source.jpg" style="display:block;width:144pt;height:96pt">'
        for first, second in [(small, large), (large, small)]:
            with self.subTest(first=first):
                _, images = self.rendered_images(first + '<section style="break-before:page">' + second + '</section>')
                self.assert_dimensions(images, [(300, 200)])
                with Image.open(io.BytesIO(images[0].raw)) as actual:
                    reference = io.BytesIO()
                    Image.new("RGB", (1, 1)).save(reference, format="JPEG", quality=encoding.QUALITY)
                    with Image.open(reference) as expected:
                        self.assertEqual(actual.quantization[0], expected.quantization[0],
                                         "DPI reduction must retain the requested JPEG quality")

    def test_affine_axes_survive_opacity_and_svg_resource_boundaries(self):
        self.source()
        cases = [
            ("scale(2)", (300, 200)),
            ("matrix(0,2,-2,0,0,0)", (300, 200)),
            ("matrix(1,.75,-.75,1,0,0)", (188, 125)),
            ("scale(2,.5)", (300, 50)),
        ]
        for transform, expected in cases:
            with self.subTest(transform=transform):
                body = (f'<section style="margin-left:110pt;opacity:.5;transform:{transform};transform-origin:0 0">'
                        '<img src="source.jpg" style="display:block;width:72pt;height:48pt"></section>')
                _, images = self.rendered_images(body)
                self.assert_dimensions(images, [expected])
        child = '<svg xmlns="http://www.w3.org/2000/svg" width="96" height="64"><image href="source.jpg" width="96" height="64"/></svg>'
        (self.output / "child.svg").write_text(child)
        for child_reference in ["source.jpg", "child.svg"]:
            with self.subTest(child=child_reference):
                svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="96" height="64">'
                       f'<g transform="scale(2)" opacity=".5"><image href="{child_reference}" width="96" height="64"/></g></svg>')
                _, images = self.rendered_images(f'<img src="data:image/svg+xml,{quote(svg)}" style="width:72pt;height:48pt">')
                self.assert_dimensions(images, [(300, 200)])

    def test_svg_mask_images_follow_the_composed_physical_transform(self):
        self.source()
        svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="192" height="128">'
               '<defs><mask id="coverage" maskUnits="userSpaceOnUse" x="0" y="0" width="96" height="64">'
               '<image href="source.jpg" width="96" height="64"/></mask></defs>'
               '<g transform="scale(2)"><rect width="96" height="64" fill="blue" mask="url(#coverage)"/></g></svg>')
        _, images = self.rendered_images(f'<img src="data:image/svg+xml,{quote(svg)}" style="width:144pt;height:96pt">')
        self.assert_dimensions(images, [(300, 200)])

    def test_transformed_patterns_and_border_slices_use_their_painted_scale(self):
        self.source()
        cases = [
            ('width:72pt;height:48pt;background:url(source.jpg) 0 0/72pt 48pt repeat;'
             'transform:scale(2);transform-origin:0 0;opacity:.5', (300, 200)),
            ('width:72pt;height:48pt;border:36pt solid transparent;border-image-source:url(source.jpg);'
             'border-image-slice:50%;border-image-width:36pt;border-image-repeat:stretch', (150, 150)),
        ]
        for style, expected in cases:
            with self.subTest(style=style):
                _, images = self.rendered_images(f'<section style="{style}"></section>')
                self.assert_dimensions(images, [expected])

    def test_each_axis_never_upsamples_and_fractional_extents_round_up(self):
        self.source()
        for size, expected in [((200, 160), (384, 256)), ((200, 48), (384, 100)),
                               ((72.25, 48.25), (151, 101))]:
            with self.subTest(size=size):
                _, images = self.rendered_images(f'<img src="source.jpg" style="width:{size[0]}pt;height:{size[1]}pt">')
                self.assert_dimensions(images, [expected])

    def test_raw_source_axes_follow_all_eight_orientation_transforms(self):
        for orientation in range(1, 9):
            with self.subTest(orientation=orientation):
                path = self.source(orientation=orientation)
                _, images = self.rendered_images('<img src="source.jpg" style="width:72pt;height:auto">')
                expected = (150, 100) if orientation <= 4 else (225, 150)
                self.assert_dimensions(images, [expected])
                self.assert_orientation(path, 96)

    def test_alpha_is_resized_with_associated_colour_without_transparent_fringe(self):
        source = Image.new("RGBA", encoding.SIZE, (255, 0, 0, 0))
        source.paste((0, 0, 255, 255), (encoding.SIZE[0] // 2, 0, *encoding.SIZE))
        source.save(self.output / "alpha.png")
        document, images = self.rendered_images('<img src="alpha.png" style="width:72pt;height:48pt">')
        self.assert_dimensions(images, [(150, 100), (150, 100)])
        main = next(image for image in images if "SMask" in image.entries)
        mask = document.resolve(main.entries["SMask"])
        self.assertEqual(main.entries["BitsPerComponent"], 8)
        alpha, color = document.decode(mask), document.decode(main)
        self.assertEqual(len(alpha), 150 * 100)
        self.assertTrue(any(0 < value < 255 for value in alpha), "a filtered edge must retain fractional coverage")
        self.assertEqual(len(color), len(alpha) * 3)
        for index, value in enumerate(alpha):
            if value:
                self.assertEqual(tuple(color[index * 3:index * 3 + 3]), (0, 0, 255),
                                 f"transparent red contaminated visible blue at sample {index}, alpha {value}")

    def test_gif_webp_and_grayscale_png_use_the_same_resolution_policy(self):
        for format_name, mode, color in [("GIF", "RGB", (37, 83, 149)),
                                         ("WEBP", "RGB", (37, 83, 149)),
                                         ("PNG", "L", 83)]:
            with self.subTest(format=format_name):
                path = self.output / ("source." + format_name.lower())
                Image.new(mode, encoding.SIZE, color).save(path, format=format_name, lossless=True)
                document, images = self.rendered_images(f'<img src="{path.name}" style="width:72pt;height:48pt">')
                main = next(image for image in images if image.entries.get("ColorSpace") != "DeviceGray") if mode == "RGB" else images[0]
                self.assertEqual((main.entries["Width"], main.entries["Height"]), (150, 100))
                self.assertEqual(main.entries["BitsPerComponent"], 8)
                channels = 3 if mode == "RGB" else 1
                samples = document.decode(main)
                self.assertEqual(len(samples), 150 * 100 * channels)
                expected = bytes(color) if mode == "RGB" else bytes([color])
                self.assertEqual(samples, expected * (150 * 100), "lossless source colour must survive DPI reduction")

    def test_bitmap_glyph_shared_with_small_image_retains_source_resolution(self):
        from fontTools.ttLib import TTFont, newTable
        from fontTools.ttLib.tables.sbixGlyph import Glyph
        from fontTools.ttLib.tables.sbixStrike import Strike

        path = self.output / "glyph.png"
        Image.new("RGB", encoding.SIZE, (37, 83, 149)).save(path)
        with TTFont(encoding.ROOT / "tests/fixtures/fonts/FontStyleTest.ttf", recalcTimestamp=False) as font:
            glyph_name = font.getBestCmap()[ord("A")]
            strike = Strike(ppem=encoding.SIZE[1])
            strike.glyphs[glyph_name] = Glyph(glyphName=glyph_name, graphicType="png ", imageData=path.read_bytes())
            font["sbix"] = newTable("sbix")
            font["sbix"].strikes = {strike.ppem: strike}
            font.save(self.output / "bitmap.ttf")
        style = ('<style>@font-face{font-family:BitmapDpi;src:url(bitmap.ttf)}'
                 'p{font-family:BitmapDpi;font-size:96pt}</style>')
        small = '<img src="glyph.png" style="width:12pt;height:9pt">'
        glyph = '<p>A</p>'
        for body in [small + glyph, glyph + small, glyph]:
            with self.subTest(body=body):
                _, images = self.rendered_images(style + body)
                self.assert_dimensions(images, [encoding.SIZE])

    def test_sixteen_bit_samples_remain_sixteen_bit_after_reduction(self):
        fixture = encoding.ROOT / "tests/oracle/assets/image-high-depth.png"
        data = fixture.read_bytes()
        width, height, depth, color_type = struct.unpack(">IIBB", data[16:26])
        self.assertEqual((depth, color_type), (16, 6), "fixture must contain 16-bit RGBA samples")
        (self.output / "high.png").write_bytes(data)
        document, images = self.rendered_images(f'<img src="high.png" style="width:{width / 2}pt;height:{height / 2}pt">', dpi=72)
        target = (math.ceil(width / 2), math.ceil(height / 2))
        self.assert_dimensions(images, [target, target])
        main = next(image for image in images if "SMask" in image.entries)
        mask = document.resolve(main.entries["SMask"])
        for stream, channels in [(main, 3), (mask, 1)]:
            self.assertEqual(stream.entries["BitsPerComponent"], 16)
            samples = document.decode(stream)
            self.assertEqual(len(samples), target[0] * target[1] * channels * 2)
            self.assertTrue(any(samples[1::2]), "resizing cannot quantize the low bytes away")


class LargePhotos(encoding.ImageCase):
    def test_phone_resolution_heic_and_avif_match_independent_decode(self):
        fixtures = encoding.ROOT / "tests/fixtures/images"
        reference = json.loads((fixtures / "large-photos.json").read_text())
        for name, metadata in reference["files"].items():
            data = (fixtures / name).read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), metadata["sha256"])
            (self.output / name).write_bytes(data)
            for downsample in (False, True):
                with self.subTest(codec=name, downsample=downsample):
                    arguments = ['--image-dpi', str(DPI)] if downsample else []
                    document = self.render(f'<img src="{name}" style="width:72pt;height:72pt">',
                                           arguments=arguments)
                    images = self.images(document)
                    self.assertEqual(len(images), 1)
                    stream = images[0]
                    size = (DPI, DPI) if downsample else tuple(reference["size"])
                    self.assertEqual((stream.entries['Width'], stream.entries['Height']), size)
                    self.assertEqual(stream.entries['BitsPerComponent'], 8)
                    self.assertEqual(stream.entries['ColorSpace'], 'DeviceRGB')
                    samples = document.decode(stream)
                    self.assertEqual(len(samples), size[0] * size[1] * 3)
                    for channel, expected in enumerate(metadata["decoded_extrema"]):
                        plane = samples[channel::3]
                        actual = (min(plane), max(plane))
                        self.assertTrue(all(abs(a - b) <= encoding.MAX_COLOR_DRIFT
                                            for a, b in zip(actual, expected)),
                                        f"channel {channel}: {actual} versus independent {expected}")

    def test_large_sixteen_bit_transparent_png_retains_samples(self):
        size = (5750, 5750)
        rgba = (12001, 24003, 48007, 32009)
        row = b"\0" + struct.pack(">4H", *rgba) * size[0]
        compressor = zlib.compressobj()
        compressed = b"".join(compressor.compress(row) for _ in range(size[1]))
        compressed += compressor.flush()

        def chunk(kind, data):
            return (struct.pack(">I", len(data)) + kind + data
                    + struct.pack(">I", zlib.crc32(kind + data)))

        source = (b"\x89PNG\r\n\x1a\n"
                  + chunk(b"IHDR", struct.pack(">IIBBBBB", *size, 16, 6, 0, 0, 0))
                  + chunk(b"IDAT", compressed) + chunk(b"IEND", b""))
        (self.output / "large.png").write_bytes(source)
        document = self.render('<img src="large.png" style="width:72pt;height:72pt">',
                               arguments=['--image-dpi', str(DPI)])
        images = self.images(document)
        self.assertEqual(len(images), 2)
        main = next(image for image in images if "SMask" in image.entries)
        mask = document.resolve(main.entries["SMask"])
        for stream, expected in [(main, rgba[:3]), (mask, rgba[3:])]:
            self.assertEqual((stream.entries['Width'], stream.entries['Height']), (DPI, DPI))
            self.assertEqual(stream.entries['BitsPerComponent'], 16)
            samples = document.decode(stream)
            self.assertEqual(len(samples), DPI * DPI * len(expected) * 2)
            for actual in struct.iter_unpack('>' + 'H' * len(expected), samples):
                self.assertTrue(all(abs(a - b) <= 2 for a, b in zip(actual, expected)),
                                f"16-bit color/alpha changed: {actual} versus {expected}")

    def test_phone_resolution_jpegs_render_with_and_without_downsampling(self):
        phone_sizes = [(8000, 6000), (10000, 5000), (12000, 9000), (16384, 12288)]
        target = (DPI, DPI)
        for size in phone_sizes:
            source = self.output / 'source.jpg'
            with Image.new('RGB', size, (19, 93, 201)) as photo:
                photo.save(source, format='JPEG', quality=encoding.SOURCE_QUALITY)
            for downsample in (False, True):
                with self.subTest(size=size, downsample=downsample):
                    arguments = ['--image-dpi', str(DPI)] if downsample else []
                    document = self.render('<img src="source.jpg" style="width:72pt;height:72pt">',
                                           arguments=arguments)
                    images = self.images(document)
                    self.assertEqual(len(images), 1)
                    image = images[0]
                    expected = target if downsample else size
                    self.assertEqual((image.entries['Width'], image.entries['Height']), expected)
                    self.assertEqual(image.entries['Filter'], 'DCTDecode')
                    if downsample:
                        with Image.open(io.BytesIO(image.raw)) as decoded:
                            self.assertEqual(decoded.size, target)
                            pixel = decoded.convert('RGB').getpixel((DPI // 2, DPI // 2))
                            self.assertTrue(all(abs(a - b) <= encoding.MAX_COLOR_DRIFT
                                                for a, b in zip(pixel, (19, 93, 201))), pixel)
                    else:
                        self.assertEqual(image.raw, source.read_bytes(),
                                         'default encoding must preserve the original JPEG stream')


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=encoding.RENDERER)
    arguments = parser.parse_args()
    encoding.RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
