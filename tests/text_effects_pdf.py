"""Check text strokes, shadows and emphasis marks in the serialized PDF against font metrics."""
from math import erf, sqrt
from pathlib import Path
import subprocess
import tempfile
import unittest

from fontTools.ttLib import TTFont
from PIL import Image

from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
FONT = ROOT / "tests/oracle/assets/DejaVuSans.ttf"
PAGE = "@page{size:300pt 200pt;margin:0}body{margin:0}"
SIZE_PT = 40
LEFT_PT = 20
TOP_PT = 20
LINE_HEIGHT_PT = 100
DPI = 144
PX_PER_PT = DPI / 72
# Antialiased edges and the PDF rasterizer place ink within this many pixels.
EDGE_TOLERANCE_PX = 2
# A Gaussian sampled on the shadow mask grid, then rasterized, stays this close to erf.
BLUR_ALPHA_TOLERANCE = 0.05
STROKE_ONLY, FILL_STROKE, FILL = 1, 2, 0
INK_THRESHOLD = 128


def metrics(character):
    with TTFont(FONT) as font:
        glyph = font["glyf"][font.getBestCmap()[ord(character)]]
        upem = font["head"].unitsPerEm
        return {
            "upem": upem,
            "ascent": font["hhea"].ascent / upem,
            "descent": -font["hhea"].descent / upem,
            "advance": font["hmtx"][font.getBestCmap()[ord(character)]][0] / upem,
            "box": [getattr(glyph, edge, 0) / upem for edge in ("xMin", "yMin", "xMax", "yMax")],
        }


def phi(x):
    return 0.5 * (1 + erf(x / sqrt(2)))


class TextEffectsPdf(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-text-effects-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)

    def render(self, style, text):
        source, target = self.output / "input.html", self.output / "output.pdf"
        source.write_text(
            f"<!doctype html><style>@font-face{{font-family:Probe;src:url('{FONT.as_uri()}')}}"
            f"{PAGE}p{{position:absolute;left:{LEFT_PT}pt;top:{TOP_PT}pt;margin:0;"
            f"font:{SIZE_PT}pt/{LINE_HEIGHT_PT}pt Probe;{style}}}</style><p>{text}</p>")
        rendered = subprocess.run([str(RENDERER), str(source), str(target)],
                                  capture_output=True, text=True, timeout=60)
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        return target

    def raster(self, target):
        prefix = self.output / "raster"
        subprocess.run(["pdftoppm", "-singlefile", "-r", str(DPI), "-png", str(target), str(prefix)],
                       check=True, capture_output=True, timeout=60)
        with Image.open(prefix.with_suffix(".png")) as image:
            return image.convert("RGB")

    def baseline_pt(self):
        """The line's baseline: half-leading above the content area, then the ascent."""
        font = metrics("H")
        content = (font["ascent"] + font["descent"]) * SIZE_PT
        return TOP_PT + (LINE_HEIGHT_PT - content) / 2 + font["ascent"] * SIZE_PT

    def ink_box(self, image, predicate, region_pt=None):
        """Pixel bounds `[left, top, right, bottom)` of pixels satisfying `predicate`, within
        `region_pt` `[left, top, right, bottom]` in points when given."""
        width, height = image.size
        left, top, right, bottom = (0, 0, width, height) if region_pt is None else (
            int(value * PX_PER_PT) for value in region_pt)
        pixels = image.load()
        xs, ys = [], []
        for y in range(max(top, 0), min(bottom, height)):
            for x in range(max(left, 0), min(right, width)):
                if predicate(pixels[x, y]):
                    xs.append(x)
                    ys.append(y)
        self.assertTrue(xs, f"expected ink in {region_pt}")
        return [min(xs), min(ys), max(xs) + 1, max(ys) + 1]

    def assert_box(self, actual, expected_pt, label):
        expected = [value * PX_PER_PT for value in expected_pt]
        for index, (got, want) in enumerate(zip(actual, expected)):
            self.assertAlmostEqual(got, want, delta=EDGE_TOLERANCE_PX,
                                   msg=f"{label}: edge {index} at {got}px, expected {want:.2f}px "
                                       f"(actual {actual}, expected {[round(v, 2) for v in expected]})")

    def test_strokes_use_text_render_modes_and_keep_the_fill(self):
        cases = [
            ("-webkit-text-stroke:2pt red;-webkit-text-fill-color:transparent",
             STROKE_ONLY, None, (1.0, 0.0, 0.0, 1.0)),
            ("color:blue;-webkit-text-stroke-width:1pt",
             FILL_STROKE, (0.0, 0.0, 1.0, 1.0), (0.0, 0.0, 1.0, 1.0)),
            ("-webkit-text-fill-color:rgb(0 128 0 / 50%);-webkit-text-stroke:1pt black",
             FILL, (0.0, 128 / 255, 0.0, 0.5), None),
        ]
        for style, mode, fill, stroke in cases:
            with self.subTest(style=style):
                glyphs = pdf_content.glyphs(self.render(style, "H"))
                self.assertEqual([glyph.text for glyph in glyphs], ["H"],
                                 "the stroke is not a second text object")
                (glyph,) = glyphs
                self.assertEqual(glyph.render_mode, mode)
                for actual, expected, name in ((glyph.fill, fill, "fill"), (glyph.stroke, stroke, "stroke")):
                    if expected is None:
                        self.assertIsNone(actual, name)
                    else:
                        for got, want in zip(actual, expected):
                            self.assertAlmostEqual(got, want, delta=1 / 255, msg=f"{name} {actual}")

    def test_differently_translucent_strokes_outline_the_glyph_outside_text(self):
        target = self.render("-webkit-text-fill-color:rgb(0 128 0 / 50%);-webkit-text-stroke:4pt red", "H")
        box = metrics("H")["box"]
        baseline = self.baseline_pt()
        image = self.raster(target)
        stroke = self.ink_box(image, lambda p: p[0] > 200 and p[1] < 80 and p[2] < 80)
        half = 2
        self.assert_box(stroke, [LEFT_PT + box[0] * SIZE_PT - half, baseline - box[3] * SIZE_PT - half,
                                 LEFT_PT + box[2] * SIZE_PT + half, baseline - box[1] * SIZE_PT + half],
                        "a 4pt stroke centred on the H outline")

    def test_sharp_shadows_offset_the_glyph_beneath_the_text_in_reverse_order(self):
        target = self.render("color:black;text-shadow:6pt 4pt red,12pt 8pt blue", "H")
        self.assertEqual([glyph.text for glyph in pdf_content.glyphs(target)], ["H"],
                         "shadows are not text")
        box = metrics("H")["box"]
        baseline = self.baseline_pt()
        glyph = [LEFT_PT + box[0] * SIZE_PT, baseline - box[3] * SIZE_PT,
                 LEFT_PT + box[2] * SIZE_PT, baseline - box[1] * SIZE_PT]
        image = self.raster(target)
        text = self.ink_box(image, lambda p: max(p) < 60)
        self.assert_box(text, glyph, "the text")
        for (dx, dy), predicate, name in (((6, 4), lambda p: p[0] > 200 and p[1] < 60 and p[2] < 60, "red"),
                                          ((12, 8), lambda p: p[2] > 200 and p[0] < 60 and p[1] < 60, "blue")):
            shadow = self.ink_box(image, predicate)
            # Only the visible part's far edges are guaranteed: nearer ones lie under the text.
            expected = [glyph[0] + dx, glyph[1] + dy, glyph[2] + dx, glyph[3] + dy]
            self.assert_box(shadow[2:], expected[2:], f"{name} shadow far edges")
        # Where both shadows overlap the first, red, is on top.
        left_stem = (glyph[0] + 12 + 1) * PX_PER_PT, (glyph[1] + 8 + 10) * PX_PER_PT
        self.assertEqual(image.getpixel(tuple(map(int, left_stem))), (255, 0, 0),
                         "the first shadow paints over the second")
        # The red shadow's crossbar crosses the text's right stem, and continues beyond it.
        crossbar = (glyph[1] + glyph[3]) / 2 + 4
        for x, expected, reason in ((glyph[2] - 2, (0, 0, 0), "the text paints over its shadows"),
                                    (glyph[2] + 1, (255, 0, 0), "the red crossbar passes the stem")):
            self.assertEqual(image.getpixel((int(x * PX_PER_PT), int(crossbar * PX_PER_PT))),
                             expected, reason)

    def test_blurred_shadows_follow_a_gaussian_of_half_the_blur_radius(self):
        blur = 16
        sigma = blur / 2
        target = self.render(f"color:transparent;text-shadow:0 0 {blur}pt red", "█")
        box = metrics("█")["box"]
        baseline = self.baseline_pt()
        edge = LEFT_PT + box[0] * SIZE_PT
        middle = baseline - (box[1] + box[3]) / 2 * SIZE_PT
        image = self.raster(target)
        for distance in (-sigma, -sigma / 2, 0, sigma / 2, sigma, 2 * sigma):
            x = int((edge + distance) * PX_PER_PT)
            red, green, blue = image.getpixel((x, int(middle * PX_PER_PT)))
            alpha = 1 - green / 255
            centre = (x + 0.5) / PX_PER_PT - edge
            expected = phi(centre / sigma)
            self.assertAlmostEqual(alpha, expected, delta=BLUR_ALPHA_TOLERANCE,
                                   msg=f"{centre:.2f}pt inside the block edge: alpha {alpha:.3f}, "
                                       f"Gaussian of deviation {sigma}pt gives {expected:.3f}")
            self.assertEqual(red, 255, "the shadow is red over white")

    def test_shadows_cover_decorations_over_the_shadowed_text(self):
        target = self.render("color:transparent;text-decoration:underline 4pt black;"
                             "text-decoration-color:transparent;text-shadow:0 10pt red", "HH")
        image = self.raster(target)
        line = self.ink_box(image, lambda p: p[0] > 200 and p[1] < 60)
        advance = metrics("H")["advance"] * SIZE_PT
        self.assertAlmostEqual(line[0], LEFT_PT * PX_PER_PT, delta=EDGE_TOLERANCE_PX)
        self.assertAlmostEqual(line[2], (LEFT_PT + 2 * advance) * PX_PER_PT, delta=EDGE_TOLERANCE_PX,
                               msg="the underline's shadow spans the decorated text")
        self.assertGreater(line[3] / PX_PER_PT, self.baseline_pt() + 10,
                           "the underline's shadow is offset below the baseline with it")

    def test_emphasis_marks_are_half_size_paths_centred_over_their_characters(self):
        target = self.render("text-emphasis:filled dot red;color:black", "H H")
        self.assertEqual("".join(glyph.text for glyph in pdf_content.glyphs(target)), "H H",
                         "marks are not text content")
        base = metrics("H")
        dot = metrics("•")
        mark_size = SIZE_PT / 2
        baseline = self.baseline_pt()
        # The mark's content area sits on the text's: its baseline is the mark descent above.
        mark_baseline = baseline - base["ascent"] * SIZE_PT - dot["descent"] * mark_size
        image = self.raster(target)
        space = metrics(" ")["advance"] * SIZE_PT
        for index, offset in enumerate((0, base["advance"] * SIZE_PT + space)):
            centre = LEFT_PT + offset + base["advance"] * SIZE_PT / 2
            left = centre - dot["advance"] * mark_size / 2
            expected = [left + dot["box"][0] * mark_size, mark_baseline - dot["box"][3] * mark_size,
                        left + dot["box"][2] * mark_size, mark_baseline - dot["box"][1] * mark_size]
            mark = self.ink_box(image, lambda p: p[0] > 200 and p[1] < 80,
                                [centre - mark_size, baseline - 2 * SIZE_PT, centre + mark_size, baseline])
            self.assert_box(mark, expected, f"mark over character {index}")

    def test_emphasis_skip_marks_only_the_requested_character_categories(self):
        dot = metrics("•")
        mark_size = SIZE_PT / 2
        ink_offset = ((dot["box"][0] + dot["box"][2]) / 2 - dot["advance"] / 2) * mark_size
        for text, skip, indices in [("A ,+ B", "symbols", [0, 1, 2, 4, 5]),
                                    ("A ,+ B", "spaces punctuation", [0, 3, 5]),
                                    ("A ,+ B", "narrow", []), ("  ", "symbols", [0, 1])]:
            with self.subTest(skip=skip, text=text):
                centres, pen = [], LEFT_PT
                for character in text:
                    advance = metrics(character)["advance"] * SIZE_PT
                    centres.append((pen + advance / 2 + ink_offset) * PX_PER_PT)
                    pen += advance
                target = self.render("white-space:pre;text-emphasis:filled dot red;color:black;"
                                     f"text-emphasis-skip:{skip}", text)
                self.assertEqual("".join(g.text for g in pdf_content.glyphs(target)), text,
                                 "mark exclusions must not change extracted text")
                image = self.raster(target)
                pixels = image.load()
                columns = [x for x in range(image.width)
                           if any(pixels[x, y][0] > 200 and pixels[x, y][1] < 80
                                  for y in range(image.height))]
                marks = []
                for column in columns:
                    if marks and column == marks[-1][1] + 1:
                        marks[-1][1] = column
                    else:
                        marks.append([column, column])
                self.assertEqual(len(marks), len(indices),
                                 f"{skip}: red mark bounds {marks}, expected indices {indices}")
                for bounds, index in zip(marks, indices):
                    self.assertAlmostEqual(sum(bounds) / 2, centres[index], delta=EDGE_TOLERANCE_PX,
                                           msg=f"{skip}: mark must remain centred on character {index}")


if __name__ == "__main__":
    unittest.main(verbosity=2)
