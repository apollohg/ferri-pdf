"""Check ruby annotation glyphs over Japanese base text in the serialized PDF (CSS Ruby 1).

Sizes, advances and vertical metrics come from the fixture font itself (FontTools), so the
expected placement is independent of the renderer's shaping and layout.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile
import unittest

from fontTools.ttLib import TTFont

from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / "tests/fixtures/fonts/CJKSpacingTest.otf"
RENDERER = ROOT / "target/debug/ferri-pdf"
SIZE_PT = 20
# The UA style sheet sets annotations at half size (CSS Ruby 1 §A.1).
ANNOTATION_PT = SIZE_PT / 2
LINE_HEIGHT_PT = 40
MARGIN_PT = 20
PAGE_PT = 300
TOLERANCE_PT = 0.01


class Face:
    def __init__(self):
        with TTFont(FONT) as font:
            self.cmap = font.getBestCmap()
            self.metrics = font["hmtx"].metrics
            self.units = font["head"].unitsPerEm
            self.ascent = font["hhea"].ascent / self.units
            self.descent = -font["hhea"].descent / self.units

    def advance(self, ch, size):
        return self.metrics[self.cmap[ord(ch)]][0] * size / self.units


class RubyPdf(unittest.TestCase):
    def setUp(self):
        self.face = Face()
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-ruby-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)

    def glyphs(self, rules, body):
        """Each glyph's text, size and origin from the page's top-left content corner."""
        source, target = self.output / "input.html", self.output / "output.pdf"
        source.write_text(f"<!doctype html><style>@font-face{{font-family:J;src:url('{FONT.as_uri()}')}}"
                          f"@page{{size:{PAGE_PT}pt {PAGE_PT}pt;margin:{MARGIN_PT}pt}}"
                          f"body{{margin:0;font:{SIZE_PT}pt/{LINE_HEIGHT_PT}pt J}}p{{margin:0}}{rules}"
                          f"</style>{body}")
        rendered = subprocess.run([str(RENDERER), str(source), str(target)],
                                  capture_output=True, text=True, timeout=60)
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        return [(glyph.text, glyph.matrix[3], glyph.matrix[4] - MARGIN_PT,
                 PAGE_PT - MARGIN_PT - glyph.matrix[5])
                for glyph in pdf_content.inspect(target)["glyphs"]]

    def split(self, glyphs):
        base = [glyph for glyph in glyphs if abs(glyph[1] - SIZE_PT) < TOLERANCE_PT]
        annotations = [glyph for glyph in glyphs if abs(glyph[1] - ANNOTATION_PT) < TOLERANCE_PT]
        self.assertEqual(len(base) + len(annotations), len(glyphs),
                         f"every glyph is base or annotation size: {glyphs}")
        return base, annotations

    def test_annotation_kana_centre_over_their_kanji_on_the_base_container(self):
        base, annotations = self.split(self.glyphs("", "<p><ruby>漢<rt>あ</rt>字<rt>アア</rt></ruby></p>"))
        self.assertEqual([glyph[0] for glyph in base], ["漢", "字"])
        self.assertEqual([glyph[0] for glyph in annotations], ["あ", "ア", "ア"])
        kanji = self.face.advance("漢", SIZE_PT)
        kana = self.face.advance("あ", ANNOTATION_PT)
        expected = [(kanji - kana) / 2, kanji, kanji + kana]
        for glyph, x in zip(annotations, expected):
            self.assertAlmostEqual(glyph[2], x, delta=TOLERANCE_PT, msg=f"{glyph[0]} origin x")
        # The annotation's content area sits on the base's: its descent below its baseline
        # meets the base's ascent above the base baseline (CSS Ruby 1 §3.1.2).
        rise = self.face.ascent * SIZE_PT + self.face.descent * ANNOTATION_PT
        for glyph in annotations:
            self.assertAlmostEqual(base[0][3] - glyph[3], rise, delta=TOLERANCE_PT,
                                   msg=f"{glyph[0]} baseline above the base's")

    def test_under_annotations_hang_from_the_base_container(self):
        base, annotations = self.split(self.glyphs("ruby{ruby-position:under}",
                                                   "<p><ruby>漢<rt>あ</rt></ruby></p>"))
        drop = self.face.descent * SIZE_PT + self.face.ascent * ANNOTATION_PT
        self.assertAlmostEqual(annotations[0][3] - base[0][3], drop, delta=TOLERANCE_PT)

    def test_wide_annotation_overhangs_adjacent_kana_by_half_an_annotation_ideograph(self):
        base, annotations = self.split(self.glyphs("", "<p>あ<ruby>漢<rt>あああ</rt></ruby>あ</p>"))
        kana = self.face.advance("あ", SIZE_PT)
        small = self.face.advance("あ", ANNOTATION_PT)
        extra = 3 * small - self.face.advance("漢", SIZE_PT)
        overhang = min(small / 2, extra / 2)
        self.assertEqual([glyph[0] for glyph in base], ["あ", "漢", "あ"])
        self.assertAlmostEqual(base[1][2], kana + extra / 2 - overhang, delta=TOLERANCE_PT,
                               msg="the base keeps its column less the overhang")
        self.assertAlmostEqual(base[2][2], 2 * kana + extra - 2 * overhang, delta=TOLERANCE_PT)
        self.assertAlmostEqual(annotations[0][2], kana - overhang, delta=TOLERANCE_PT,
                               msg="the annotation reaches over the preceding kana")
        self.assertAlmostEqual(annotations[-1][2] + small, base[2][2] + overhang, delta=TOLERANCE_PT,
                               msg="and over the following kana")

    def test_base_breaking_across_lines_breaks_its_annotation_in_proportion(self):
        """CSS Ruby 1 §3.4.2: six tenths of the base on the first line hold three fifths of the
        annotation, and each fragment spreads by `space-around` over its own base fragment."""
        kanji = self.face.advance("漢", SIZE_PT)
        base, annotations = self.split(self.glyphs(
            f"p{{width:{6 * kanji}pt}}rb,rt{{white-space:normal}}",
            "<p><ruby><rb>漢字漢字漢字漢字漢字</rb><rt>あああああ</rt></ruby></p>"))
        rows = {}
        for glyph in base + annotations:
            rows.setdefault(round(glyph[3], 2), []).append(glyph)
        lines = [sorted(rows[y], key=lambda glyph: glyph[2]) for y in sorted(rows)]
        self.assertEqual(["".join(glyph[0] for glyph in line) for line in lines],
                         ["あああ", "漢字漢字漢字", "ああ", "漢字漢字"],
                         "each annotation fragment lies over its base fragment")
        kana = self.face.advance("あ", ANNOTATION_PT)
        for annotation, fragment in ((lines[0], lines[1]), (lines[2], lines[3])):
            self.assertAlmostEqual(fragment[0][2], 0, delta=TOLERANCE_PT,
                                   msg="each base fragment starts its line")
            column = len(fragment) * kanji
            each = (column - len(annotation) * kana) / len(annotation)
            for index, glyph in enumerate(annotation):
                self.assertAlmostEqual(glyph[2], each / 2 + index * (kana + each),
                                       delta=TOLERANCE_PT,
                                       msg=f"kana {index} of {len(annotation)} in a {column}pt column")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
