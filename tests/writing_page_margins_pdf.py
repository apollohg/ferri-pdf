"""Physical page-margin slots with independently oriented text interiors."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
PAGE_PT = 240
MARGIN_PT = 60
TOLERANCE_PT = 0.02
MODES = ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]


class WritingPageMarginsPdf(unittest.TestCase):
    def render(self, rules):
        with tempfile.TemporaryDirectory(prefix="ferri-writing-margin-") as directory:
            source = Path(directory) / "input.html"
            target = Path(directory) / "output.pdf"
            source.write_text(f"<!doctype html><style>body{{margin:0}}"
                              f"@page{{size:{PAGE_PT}pt;margin:{MARGIN_PT}pt;{rules}}}</style>")
            rendered = subprocess.run([str(RENDERER), str(source), str(target)],
                                      capture_output=True, text=True, timeout=60)
            self.assertEqual(rendered.returncode, 0, rendered.stderr)
            raster = Path(directory) / "raster"
            result = subprocess.run(["pdftoppm", "-png", "-singlefile", "-r", "72",
                                     str(target), str(raster)], capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            with Image.open(raster.with_suffix(".png")) as image:
                pixels = image.convert("RGB").copy()
            return pdf_content.inspect(target)["glyphs"], pixels

    def test_margin_text_orients_inside_physical_slots(self):
        for mode in MODES:
            with self.subTest(mode=mode):
                glyphs, pixels = self.render(
                    f'@top-center{{content:"AB";font:10pt/1 serif;writing-mode:{mode};'
                    'width:80pt;padding:3pt 5pt 7pt 9pt;box-sizing:border-box;'
                    'border-style:solid;border-width:2pt 4pt 6pt 8pt;'
                    'border-color:red green blue purple}')
                self.assertEqual([glyph.text for glyph in glyphs], list("AB"))
                first, second = [glyph.matrix for glyph in glyphs]
                self.assertAlmostEqual(first[0], 0, delta=TOLERANCE_PT, msg=str(first))
                self.assertAlmostEqual(first[3], 0, delta=TOLERANCE_PT, msg=str(first))
                self.assertAlmostEqual(first[4], second[4], delta=TOLERANCE_PT)
                sign = 1 if mode == "sideways-lr" else -1
                self.assertGreater(sign * (second[5] - first[5]), 0, str(glyphs))
                for point, colour in [((100, 1), (255, 0, 0)),
                                      ((158, 20), (0, 128, 0)),
                                      ((100, 57), (0, 0, 255)),
                                      ((83, 20), (128, 0, 128))]:
                    self.assertEqual(pixels.getpixel(point), colour,
                                     f"physical border at {point} must retain its side in {mode}")

    def test_top_bottom_side_and_corner_margins_wrap_in_the_inline_dimension(self):
        for mode in MODES:
            for slot, bounds in [("top-center", (60, 0, 180, 60)),
                                 ("bottom-center", (60, 180, 180, 240)),
                                 ("left-middle", (0, 60, 60, 180)),
                                 ("right-middle", (180, 60, 240, 180)),
                                 ("top-left-corner", (0, 0, 60, 60))]:
                with self.subTest(mode=mode, slot=slot):
                    glyphs, _ = self.render(f'@{slot}{{content:"AB CD EF";font:20pt/1 serif;'
                                            f'writing-mode:{mode}}}')
                    letters = [glyph for glyph in glyphs if glyph.text.strip()]
                    self.assertEqual([glyph.text for glyph in letters], list("ABCDEF"))
                    columns = {round(glyph.matrix[4], 2) for glyph in letters}
                    if slot in ["top-center", "bottom-center", "top-left-corner"]:
                        self.assertGreater(len(columns), 1, "60pt inline height must wrap the words")
                    else:
                        self.assertEqual(len(columns), 1, "side margin's 120pt inline height fits the words")
                    x0, y0, x1, y1 = bounds
                    for glyph in letters:
                        x, y = glyph.matrix[4], PAGE_PT - glyph.matrix[5]
                        self.assertTrue(x0 <= x <= x1 and y0 <= y <= y1,
                                        f"{slot}/{mode}: glyph origin {(x, y)} outside {bounds}")

    def test_vertical_align_moves_the_content_in_the_physical_vertical_dimension(self):
        for mode in MODES:
            with self.subTest(mode=mode):
                positions = []
                for align in ["top", "middle", "bottom"]:
                    glyphs, _ = self.render('@top-center{content:"AB";font:10pt/1 serif;'
                                            f'writing-mode:{mode};vertical-align:{align}}}')
                    positions.append([glyph.matrix for glyph in glyphs])
                top, middle, bottom = positions
                for index in range(2):
                    self.assertAlmostEqual(top[index][4], bottom[index][4], delta=TOLERANCE_PT)
                    self.assertGreater(top[index][5], middle[index][5])
                    self.assertGreater(middle[index][5], bottom[index][5])
                    self.assertAlmostEqual(top[index][5] - middle[index][5],
                                           middle[index][5] - bottom[index][5], delta=TOLERANCE_PT)

    def test_upright_margin_glyphs_keep_their_orientation_and_column_centre(self):
        for mode in ["vertical-rl", "vertical-lr"]:
            with self.subTest(mode=mode):
                glyphs, _ = self.render('@right-middle{content:"AB";font:20pt/1 monospace;'
                                        f'writing-mode:{mode};text-orientation:upright}}')
                first, second = [glyph.matrix for glyph in glyphs]
                self.assertGreater(first[0], 0, str(first))
                self.assertGreater(first[3], 0, str(first))
                self.assertAlmostEqual(first[1], 0, delta=TOLERANCE_PT)
                self.assertAlmostEqual(first[2], 0, delta=TOLERANCE_PT)
                self.assertAlmostEqual(first[4], second[4], delta=TOLERANCE_PT)
                self.assertLess(second[5], first[5], "upright glyphs advance down the margin")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
