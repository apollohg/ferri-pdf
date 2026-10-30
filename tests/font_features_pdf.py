"""Check CSS font feature, variation and palette effects in the serialized PDF."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image

from font_geometry import InstanceInterpreter
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/fonts"
RENDERER = ROOT / "target/debug/ferri-pdf"
SIZE_PT = 20
POSITION_TOLERANCE_PT = 0.01
FONT_UNIT_TOLERANCE = 1.0
PDF_POINTS_PER_INCH = 72
RASTER_DPI = 144
COLOUR_TOLERANCE = 2
FACES = """@font-face{{font-family:Serif4;src:url('{fixtures}/SourceSerif4-Regular.ttf')}}
@font-face{{font-family:Flex;src:url('{fixtures}/RobotoFlex.ttf')}}
@font-face{{font-family:Palette;src:url('{fixtures}/PaletteTest.ttf')}}
@font-face{{font-family:Presentation;src:url('{fixtures}/PresentationTest.ttf')}}
@page{{size:400pt 400pt;margin:20pt}}
p{{margin:0;font-size:{size}pt;line-height:40pt}}"""


def reference(name):
    return json.loads((FIXTURES / name).read_text())


class FontFeaturesPdf(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-font-features-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)

    def render(self, rules, body):
        source, target = self.output / "input.html", self.output / "output.pdf"
        faces = FACES.format(fixtures=FIXTURES.as_uri(), size=SIZE_PT)
        source.write_text(f"<!doctype html><style>{faces}{rules}</style>{body}")
        rendered = subprocess.run([str(RENDERER), str(source), str(target)],
                                  capture_output=True, text=True, timeout=60)
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        document = pdf_content.Document(target)
        interpreter = InstanceInterpreter(document)
        for index, page in enumerate(document.pages(), start=1):
            interpreter.run_page(index, page)
        return interpreter

    def raster(self):
        prefix = self.output / "raster"
        subprocess.run(["pdftoppm", "-singlefile", "-r", str(RASTER_DPI), "-png",
                        str(self.output / "output.pdf"), str(prefix)],
                       check=True, capture_output=True, timeout=60)
        with Image.open(prefix.with_suffix(".png")) as raster:
            return raster.convert("RGB")

    def lines(self, glyphs):
        rows = {}
        for glyph in glyphs:
            rows.setdefault(round(glyph.origin[1], 3), []).append(glyph)
        return [rows[y] for y in sorted(rows, reverse=True)]

    def assert_advances(self, line, sample):
        """Origins of consecutive glyphs are the independently shaped advances apart."""
        shaped = reference("FontFeatures-reference.json")
        record = shaped["samples"][sample]
        upem = shaped["fonts"][record["font"]]["units_per_em"]
        expected = [glyph["ax"] / upem * SIZE_PT for glyph in record["glyphs"]]
        self.assertEqual(len(line), len(expected), sample)
        for index, (left, right) in enumerate(zip(line, line[1:])):
            self.assertAlmostEqual(right.origin[0] - left.origin[0], expected[index],
                                   delta=POSITION_TOLERANCE_PT, msg=f"{sample} glyph {index}")

    def test_figure_spacing_and_kerning_reach_glyph_positions(self):
        cases = [
            ("font-family:Serif4;font-variant-numeric:tabular-nums", "1011", "tabular_figures"),
            ("font-family:Serif4;font-variant-numeric:proportional-nums", "1011", "proportional_figures"),
            ("font-family:'Liberation Serif'", "AVA", "kerned"),
            ("font-family:'Liberation Serif';font-kerning:none", "AVA", "unkerned"),
        ]
        body = "".join(f"<p style=\"{style}\">{text}</p>" for style, text, _ in cases)
        lines = self.lines(self.render("", body).glyphs)
        self.assertEqual(len(lines), len(cases))
        for line, (style, text, sample) in zip(lines, cases):
            with self.subTest(style=style):
                self.assertEqual("".join(glyph.text for glyph in line), text)
                self.assert_advances(line, sample)

    def test_adjusted_and_synthesized_sizes_reach_text_matrices(self):
        metrics = reference("FontUnit-metrics.json")["faces"]["InitialSerif"]
        x_height = metrics["x_height"] / metrics["units_per_em"]
        body = ("<p style=\"font-family:'Liberation Serif';font-size-adjust:0.5\">x</p>"
                "<p style=\"font-family:'Liberation Serif'\">4<span style=\"font-variant-position:super\">4x</span></p>")
        # Glyphs paint in document order; the raised run does not share the base line's origin.
        adjusted, base, superscript, letter = self.render("", body).glyphs
        self.assertAlmostEqual(adjusted.matrix[0], SIZE_PT * 0.5 / x_height, delta=POSITION_TOLERANCE_PT)
        self.assertAlmostEqual(base.matrix[0], SIZE_PT, delta=POSITION_TOLERANCE_PT)
        for glyph in (superscript, letter):
            self.assertLess(glyph.matrix[0], SIZE_PT, "synthesized superscripts are reduced")
            self.assertGreater(glyph.origin[1], base.origin[1], "synthesized superscripts are raised")
        self.assertAlmostEqual(superscript.origin[1], letter.origin[1], delta=POSITION_TOLERANCE_PT,
                               msg="the whole run shares one raised baseline")

    def test_svg_text_renders_caps_as_document_text_does(self):
        """CSS Fonts 4 §6.6: a face's own capitals, or capitals synthesized from uppercase forms
        scaled down, for SVG text exactly as for the document's lines."""
        values = ("small-caps", "all-small-caps", "petite-caps", "all-petite-caps", "unicase",
                  "titling-caps")
        sample = "Ab"
        line_pitch = 40
        height = line_pitch * len(values)
        for family in ("Serif4", "'Liberation Serif'"):
            paragraphs = "".join(f"<p style=\"font-family:{family};font-variant-caps:{caps}\">"
                                 f"{sample}</p>" for caps in values)
            texts = "".join(f"<text y=\"{(index + 1) * line_pitch}\" style=\"font-family:{family};"
                            f"font-variant-caps:{caps};font-size:{SIZE_PT}px\">{sample}</text>"
                            for index, caps in enumerate(values))
            document = self.lines(self.render("", paragraphs).glyphs)
            svg = self.lines(self.render("svg{display:block}",
                                         f"<svg width=\"300pt\" height=\"{height}pt\" "
                                         f"viewBox=\"0 0 300 {height}\">{texts}</svg>").glyphs)
            self.assertEqual((len(document), len(svg)), (len(values), len(values)),
                             "one line per paragraph and per SVG text")
            for expected, actual, caps in zip(document, svg, values):
                with self.subTest(family=family, caps=caps):
                    self.assertEqual("".join(glyph.text for glyph in actual), sample)
                    self.assertEqual([round(glyph.matrix[0], 3) for glyph in actual],
                                     [round(glyph.matrix[0], 3) for glyph in expected],
                                     "SVG glyphs take the document's synthesized sizes")
                    advances = [[right.origin[0] - left.origin[0]
                                 for left, right in zip(line, line[1:])]
                                for line in (expected, actual)]
                    for wanted, got in zip(*advances):
                        self.assertAlmostEqual(got, wanted, delta=POSITION_TOLERANCE_PT,
                                               msg=f"SVG advances {advances[1]} match {advances[0]}")
            if family == "'Liberation Serif'":
                sizes = [round(glyph.matrix[0], 3) for glyph in svg[values.index("all-small-caps")]]
                self.assertEqual(sizes, [round(SIZE_PT * 0.8, 3)] * 2,
                                 "Liberation Serif has no c2sc or smcp, so both letters are "
                                 "synthesized")

    def test_optical_size_and_presentation_select_the_embedded_glyphs(self):
        optical = reference("RobotoFlex-optical.json")
        advances = {json.dumps(sample["location"], sort_keys=True): sample["advance"]
                    for sample in optical["samples"]}
        presentation = {sample["text"]: sample for sample in reference("PresentationTest-source.json")["samples"]}
        body = ("<p style=\"font-family:Flex\">H</p>"
                "<p style=\"font-family:Flex;font-optical-sizing:none\">H</p>"
                "<p style=\"font-family:Presentation;font-variant-emoji:emoji\">❤</p>")
        interpreted = self.render("", body)
        automatic, disabled, emoji = self.lines(interpreted.glyphs)
        for (glyph,), location in ((automatic, {"opsz": SIZE_PT}), (disabled, {"opsz": 14})):
            upem, advance, _ = interpreted.embedded_glyph(glyph.font, "H")
            self.assertEqual(upem, optical["units_per_em"])
            self.assertAlmostEqual(advance, advances[json.dumps(location, sort_keys=True)],
                                   delta=FONT_UNIT_TOLERANCE, msg=f"embedded instance at {location}")
        (heart,) = emoji
        self.assertEqual(heart.text, "❤")
        data, glyph_id = interpreted.embedded_glyph_data(heart.font, "❤")
        self.assertTrue(data)
        _, advance, _ = interpreted.embedded_glyph(heart.font, "❤")
        self.assertEqual(advance, presentation["❤️"]["advance"],
                         "the emoji presentation glyph is embedded for the original character")

    def test_palettes_paint_the_selected_colours(self):
        fixture = reference("PaletteTest-source.json")
        palettes = [[tuple(colour[:3]) for colour in palette] for palette in fixture["palettes"]]
        foreground = (0, 0, 255)
        rules = ("@font-palette-values --custom{font-family:Palette;base-palette:dark;"
                 "override-colors:1 rgb(255,0,255)}")
        cases = [
            ("normal", palettes[0]),
            ("light", palettes[1]),
            ("dark", palettes[2]),
            ("--custom", [palettes[2][0], (255, 0, 255)]),
        ]
        body = "".join(f"<p style=\"font-family:Palette;color:rgb{foreground};font-palette:{value}\">AHI</p>"
                       for value, _ in cases)
        interpreted = self.render(rules, body)
        raster = self.raster()
        scale = RASTER_DPI / PDF_POINTS_PER_INCH
        for line, (value, colours) in zip(self.lines(interpreted.glyphs), cases):
            for glyph, expected in zip(line, [*colours, foreground]):
                with self.subTest(palette=value, glyph=glyph.text):
                    x, y = glyph.origin
                    painted = {}
                    # Sample a box above the baseline that the glyph's stems cross.
                    for dx in range(int(2 * scale), int(10 * scale)):
                        for dy in range(int(2 * scale), int(12 * scale)):
                            pixel = raster.getpixel((int(x * scale) + dx, raster.height - int(y * scale) - dy))
                            if pixel != (255, 255, 255):
                                painted[pixel] = painted.get(pixel, 0) + 1
                    self.assertTrue(painted, "the glyph paints")
                    dominant = max(painted, key=painted.get)
                    for actual, wanted in zip(dominant, expected):
                        self.assertLessEqual(abs(actual - wanted), COLOUR_TOLERANCE, f"{dominant} != {expected}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
