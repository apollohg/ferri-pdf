"""Check text-autospace and text-spacing-trim glyph origins in the serialized PDF.

Advances come from the fixture fonts' own metrics (FontTools) and the HarfBuzz `halt`
samples recorded beside them, independently of the renderer's shaping.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from fontTools.ttLib import TTFont

from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/fonts"
RENDERER = ROOT / "target/debug/ferri-pdf"
HALT, NO_HALT = "CJKSpacingTest.otf", "CJKSpacingTest-NoHalt.otf"
FAMILIES = {HALT: "Halt", NO_HALT: "NoHalt"}
SIZE_PT = 10
MARGIN_PT = 20
TOLERANCE_PT = 0.01
WATER = "水"
# CSS Text 4 §8.4.1: 1/8 of the ideographic advance.
INTER_SCRIPT_IC = 1 / 8
IDEOGRAPHS = set("漢字水")
LETTERS_AND_NUMERALS = set("abc123")


class Face:
    def __init__(self, name):
        with TTFont(FIXTURES / name) as font:
            self.cmap = font.getBestCmap()
            self.metrics = font["hmtx"].metrics
            self.units = font["head"].unitsPerEm
        self.halt = json.loads((FIXTURES / "CJKSpacingTest-source.json").read_text())["faces"][name]["samples"]

    def advance(self, ch):
        return self.metrics[self.cmap[ord(ch)]][0] * SIZE_PT / self.units

    def half_width(self, ch, side):
        """The [offset, advance] of `ch` set half-width: `halt` where the face has it, else
        its blank half, whose `side` is "start", "end" or "both" (a middle dot)."""
        halved = self.halt[ch]["halt"][0]
        if halved != self.halt[ch]["default"][0]:
            return [halved["offset"] * SIZE_PT / self.units, halved["advance"] * SIZE_PT / self.units]
        full = self.advance(WATER)
        return {"start": [-full / 2, full / 2], "end": [0, full / 2], "both": [-full / 4, full / 2]}[side]


class TextSpacingPdf(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-text-spacing-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)

    def lines(self, rules, body):
        """Glyphs of each line, top to bottom, each left to right."""
        source, target = self.output / "input.html", self.output / "output.pdf"
        faces = "".join(f"@font-face{{font-family:{family};src:url('{(FIXTURES / name).as_uri()}')}}"
                        for name, family in FAMILIES.items())
        source.write_text(f"<!doctype html><style>{faces}@page{{size:300pt 300pt;margin:{MARGIN_PT}pt}}"
                          f"body{{margin:0;font:{SIZE_PT}pt/20pt Halt}}p{{margin:0}}{rules}</style>{body}")
        rendered = subprocess.run([str(RENDERER), str(source), str(target)],
                                  capture_output=True, text=True, timeout=60)
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        rows = {}
        for glyph in pdf_content.inspect(target)["glyphs"]:
            rows.setdefault(round(glyph.matrix[5], 3), []).append(glyph)
        return [sorted(rows[y], key=lambda glyph: glyph.matrix[4]) for y in sorted(rows, reverse=True)]

    def assert_origins(self, label, glyphs, expected):
        self.assertEqual("".join(glyph.text for glyph in glyphs), "".join(ch for ch, _ in expected), label)
        for index, (glyph, (ch, x)) in enumerate(zip(glyphs, expected)):
            self.assertAlmostEqual(glyph.matrix[4] - MARGIN_PT, x, delta=TOLERANCE_PT,
                                   msg=f"{label}: glyph {index} {ch!r} origin")

    def test_inter_script_spacing_reaches_glyph_origins(self):
        face = Face(HALT)
        text = "漢字abc漢字123漢"
        gap = face.advance(WATER) * INTER_SCRIPT_IC
        for value, spaced in (("normal", True), ("no-autospace", False)):
            with self.subTest(value=value):
                [line] = self.lines(f"p{{text-autospace:{value}}}", f"<p>{text}</p>")
                expected, pen = [], 0
                for index, ch in enumerate(text):
                    if index and spaced and len({text[index - 1], ch} & IDEOGRAPHS) == 1 \
                            and {text[index - 1], ch} & LETTERS_AND_NUMERALS:
                        pen += gap
                    expected.append((ch, pen))
                    pen += face.advance(ch)
                self.assert_origins(value, line, expected)

    def test_replaced_space_leaves_layout_but_not_the_text(self):
        face = Face(HALT)
        gap = face.advance(WATER) * INTER_SCRIPT_IC
        [line] = self.lines("p{text-autospace:ideograph-alpha replace}", "<p>漢 abc</p>")
        start = face.advance("漢") + gap
        self.assert_origins("replace", line, [
            ("漢", 0), (" ", start), ("a", start), ("b", start + face.advance("a")),
            ("c", start + face.advance("a") + face.advance("b")),
        ])

    def test_trimmed_punctuation_origins_with_and_without_halt(self):
        # trim-both trims the opening bracket starting the line and the closing one ending
        # it; between, the opening parenthesis follows closing punctuation.
        text = "「水〕（水」"
        sides = {0: "start", 3: "start", 5: "end"}
        for name, family in FAMILIES.items():
            face = Face(name)
            with self.subTest(face=name):
                [line] = self.lines(f"p{{font-family:{family};text-spacing-trim:trim-both}}",
                                    f"<p>{text}</p>")
                expected, pen = [], 0
                for index, ch in enumerate(text):
                    offset, advance = face.half_width(ch, sides[index]) if index in sides \
                        else [0, face.advance(ch)]
                    expected.append((ch, pen + offset))
                    pen += advance
                self.assert_origins(name, line, expected)
                [spaced] = self.lines(f"p{{font-family:{family};text-spacing-trim:space-all}}",
                                      f"<p>{text}</p>")
                pen, untrimmed = 0, []
                for ch in text:
                    untrimmed.append((ch, pen))
                    pen += face.advance(ch)
                self.assert_origins(f"{name} space-all", spaced, untrimmed)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
