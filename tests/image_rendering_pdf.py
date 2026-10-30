"""Check that `image-rendering` reaches every image XObject the renderer writes (CSS Images 3
§5.2): the `Interpolate` flag viewers scale by, nearest-neighbour enlargement for `pixelated`,
and the resampling `--image-dpi` applies."""
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
# One device pixel per point, so painted sizes in points are device pixels.
DEVICE_DPI = "72"
PAGE = "@page{size:400pt 400pt;margin:40pt}body{margin:0}"
RED = (230, 20, 40)
BLUE = (20, 40, 230)
# Every site paints its own image, told apart by its first pixel's colour, with its pixel
# size and the whole multiple of it that it paints at, if there is a single one.
SITES = {
    "img": ((3, 5), (9, 5), '<img src="img.png" style="display:block;width:27pt;height:25pt">'),
    "background": ((4, 5), (7, 5), '<div style="width:28pt;height:25pt;'
                   'background:url(background.png) 0 0/100% 100% no-repeat"></div>'),
    "border-image": ((6, 6), None, '<div style="width:30pt;height:30pt;border:12pt solid;'
                     'border-image:url(border.png) 2 stretch"></div>'),
    "content": ((2, 7), (6, 3), '<div class=generated style="width:12pt;height:21pt"></div>'),
    "marker": ((3, 4), (3, 3), '<ul style="margin:0 0 0 30pt"><li class=marker>item</li></ul>'),
    "svg": ((4, 3), (8, 7), '<svg width="32pt" height="21pt" viewBox="0 0 32 21" '
            'preserveAspectRatio="none"><image href="svg.png" width="32" height="21" '
            'preserveAspectRatio="none"/></svg>'),
    "cross-fade": ((4, 4), (4, 4), '<div style="width:16pt;height:16pt;'
                   'background:cross-fade(url(cross.png) 40%, red) 0 0/100% 100% no-repeat"></div>'),
    "margin": ((2, 3), None, ""),
}
STYLE = (".generated{{content:url(content.png)}}"
         ".marker{{list-style-image:image-set(url(marker.png) 0.25x)}}"
         "@page{{@top-left{{content:url(margin.png);image-rendering:{value}}}}}")
# Used `image-rendering` of each value: CSS Images 3 §5.2 has `optimizeSpeed` behave as
# `crisp-edges` and `optimizeQuality` as `smooth`; `auto` smooths.
SMOOTH, PIXELATED, CRISP = "smooth", "pixelated", "crisp-edges"
# A luminance mask layer is rasterized at the device size and embedded as grey values.
MASK_SIZE, MASK_PAINTED = (5, 4), (30, 20)
MASK = ('<div style="width:30pt;height:20pt;background:green;'
        'mask:url(mask.png) 0 0/100% 100% no-repeat luminance"></div>')
VALUES = {"auto": SMOOTH, "smooth": SMOOTH, "high-quality": SMOOTH, "optimizeQuality": SMOOTH,
          "pixelated": PIXELATED, "crisp-edges": CRISP, "optimizeSpeed": CRISP}


def checker(size, first=BLUE):
    image = Image.new("RGB", size)
    image.putdata([RED if (x + y) % 2 else first for y in range(size[1]) for x in range(size[0])])
    return image


def site_colour(index):
    """The colour of the first pixel of the image a site paints, which identifies it."""
    return (20 + 30 * index, 200 - 20 * index, 90)


class ImageRenderingPdf(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-image-rendering-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def render(self, css, body, *arguments):
        source, target = self.root / "input.html", self.root / "output.pdf"
        source.write_text(f"<!doctype html><style>{PAGE}{css}</style>{body}")
        result = subprocess.run([str(RENDERER), str(source), str(target), *arguments],
                                capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("warning", result.stderr)
        return pdf_content.Document(target)

    @staticmethod
    def images(document):
        streams = (document.get(number) for number in document.offsets)
        return [stream for stream in streams if isinstance(stream, pdf_content.Stream)
                and stream.entries.get("Subtype") == "Image" and "ColorSpace" in stream.entries
                and stream.entries["ColorSpace"] != "DeviceGray"]

    def samples(self, document, stream):
        data = document.decode(stream)
        width, height = stream.entries["Width"], stream.entries["Height"]
        self.assertEqual(len(data), width * height * 3, f"8-bit RGB samples of {stream.entries}")
        return [tuple(data[index:index + 3]) for index in range(0, len(data), 3)]

    def test_every_image_paint_site_writes_its_used_interpolation(self):
        names = {}
        for index, (name, (size, _, _)) in enumerate(SITES.items()):
            checker(size, site_colour(index)).save(self.root / f"{name.split('-')[0]}.png")
            names[site_colour(index)] = name
        checker(MASK_SIZE).save(self.root / "mask.png")
        body = "".join(markup for _, _, markup in SITES.values()) + MASK
        for value, used in VALUES.items():
            with self.subTest(value=value):
                document = self.render(f"body{{image-rendering:{value}}}" + STYLE.format(value=value), body,
                                       "--image-dpi", DEVICE_DPI)
                written = {}
                masks = []
                for stream in self.images(document):
                    samples = self.samples(document, stream)
                    if all(red == green == blue for red, green, blue in samples):
                        masks.append((stream.entries, set(samples)))
                        continue
                    first = samples[0]
                    self.assertIn(first, names, f"{value}: an XObject no site paints: {stream.entries}")
                    written.setdefault(names[first], []).append(stream.entries)
                for name, (size, multiples, _) in SITES.items():
                    entries = written.get(name)
                    self.assertTrue(entries, f"{value}: {name} writes no XObject of its image; "
                                             f"images: {[s.entries for s in self.images(document)]}")
                    for entry in entries:
                        interpolate = entry.get("Interpolate", False)
                        self.assertEqual(interpolate, used != CRISP,
                                         f"{value}: {name} XObject {entry} must "
                                         f"{'' if used != CRISP else 'not '}ask viewers to interpolate")
                        scale = (entry["Width"] // size[0], entry["Height"] // size[1])
                        if used != PIXELATED:
                            self.assertEqual(scale, (1, 1), f"{value}: {name} embeds its natural size")
                        elif multiples is not None:
                            self.assertEqual(scale, multiples, f"{value}: {name} enlarges by the whole "
                                             "multiple closest to its device size")
                        else:
                            self.assertNotEqual(scale, (1, 1), f"{value}: {name} enlarges")
                [(entries, greys)] = masks
                self.assertEqual((entries["Width"], entries["Height"]), MASK_PAINTED,
                                 f"{value}: the mask layer's values at its device size")
                self.assertEqual(entries.get("Interpolate", False), used != CRISP,
                                 f"{value}: the mask layer's values keep its rendering: {entries}")
                # Only scaling that blends produces greys between the two sources' luminances;
                # pixelated reaches a whole multiple, which nearest neighbour fills alone.
                self.assertEqual(len(greys) > 2, used == SMOOTH,
                                 f"{value}: the mask image scales as {used}: greys {sorted(greys)}")

    def test_pixelated_enlarges_by_nearest_neighbour_and_crisp_edges_downsamples_by_it(self):
        source = checker((4, 3))
        source.save(self.root / "checker.png")
        enlarged = source.resize((12, 9), Image.Resampling.NEAREST)
        expected = [enlarged.getpixel((x, y)) for y in range(9) for x in range(12)]
        document = self.render("img{display:block;width:12pt;height:9pt;image-rendering:pixelated}",
                               '<img src="checker.png">', "--image-dpi", DEVICE_DPI)
        [stream] = self.images(document)
        self.assertEqual((stream.entries["Width"], stream.entries["Height"]), (12, 9))
        self.assertEqual(self.samples(document, stream), expected,
                         "each source pixel is repeated over a 3 by 3 block")
        large = checker((40, 40))
        large.save(self.root / "large.png")
        for value, blends in [("crisp-edges", False), ("smooth", True)]:
            with self.subTest(value=value):
                document = self.render(f"img{{display:block;width:10pt;height:10pt;image-rendering:{value}}}",
                                       '<img src="large.png">', "--image-dpi", DEVICE_DPI)
                [stream] = self.images(document)
                self.assertEqual((stream.entries["Width"], stream.entries["Height"]), (10, 10),
                                 "--image-dpi downsamples to the painted device size")
                colours = set(self.samples(document, stream))
                self.assertEqual(not colours <= {RED, BLUE}, blends,
                                 f"{value} downsampling {'blends' if blends else 'keeps'} source colours: {colours}")

    def test_one_source_painted_with_two_renderings_embeds_both(self):
        checker((4, 3)).save(self.root / "shared.png")
        document = self.render("img{display:block;width:12pt;height:9pt}",
                               '<img src="shared.png"><img src="shared.png" style="image-rendering:crisp-edges">'
                               '<img src="shared.png">')
        flags = sorted(stream.entries.get("Interpolate", False) for stream in self.images(document))
        self.assertEqual(flags, [False, True],
                         "the smooth paints share one XObject and the crisp-edges paint has its own")

    def test_svg_filter_rasters_scale_images_by_their_rendering(self):
        pair = Image.new("RGB", (2, 1))
        pair.putdata([RED, BLUE])
        pair.save(self.root / "pair.png")
        # 2.5 times across: nearest neighbour blends nothing; pixelated enlarges three times
        # and blends the one column where the six pixels meet; smooth blends every inner one.
        for value, blended in [("crisp-edges", 0), ("pixelated", 1), ("smooth", 3)]:
            with self.subTest(value=value):
                document = self.render(
                    "", f'<svg width="20pt" height="8pt" viewBox="0 0 20 8" '
                    f'style="display:block;image-rendering:{value}">'
                    '<filter id="f" x="0" y="0" width="1" height="1"><feOffset/></filter>'
                    '<image href="pair.png" width="5" height="4" preserveAspectRatio="none" '
                    'filter="url(#f)"/></svg>', "--image-dpi", DEVICE_DPI)
                [stream] = self.images(document)
                self.assertEqual((stream.entries["Width"], stream.entries["Height"]), (5, 4),
                                 "the filter rasterizes the image at its device size")
                width = stream.entries["Width"]
                samples = self.samples(document, stream)
                for row in range(stream.entries["Height"]):
                    pixels = samples[row * width:(row + 1) * width]
                    self.assertEqual(sum(pixel not in (RED, BLUE) for pixel in pixels), blended,
                                     f"{value}: row {row} {pixels}")


if __name__ == "__main__":
    unittest.main(verbosity=2)
