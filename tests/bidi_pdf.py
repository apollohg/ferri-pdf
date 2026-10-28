"""Verify native bidi glyph order, mirroring and logical PDF text against Unicode data."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.ttLib import TTFont

from font_geometry import InstanceInterpreter
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
REFERENCE = ROOT / "tests/fixtures/bidi/character-reference.json"
FONT = ROOT / "crates/fonts/assets/fonts/DejaVuSans.ttf"
FONT_SHA256 = "3fdf69cabf06049ea70a00b5919340e2ce1e6d02b0cc3c4b44fb6801bd1e0d22"
FONT_SIZE_PT = 20
BIDI_FORMATTING = set(map(chr, [0x200E, 0x200F, *range(0x202A, 0x202F), *range(0x2066, 0x206A)]))


def outline(font, name):
    glyphs = font.getGlyphSet()
    pen = DecomposingRecordingPen(glyphs)
    glyphs[name].draw(pen)
    return json.loads(json.dumps(pen.value))


class BidiInterpreter(InstanceInterpreter):
    def show(self, page, graphics, text_matrix, strings):
        codes = [code for item in strings if isinstance(item, bytes)
                 for code in graphics["font"].codes(item)]
        first = len(self.glyphs)
        matrix = super().show(page, graphics, text_matrix, strings)
        painted = self.glyphs[first:]
        if graphics["mode"] != 3:
            if len(painted) != len(codes):
                raise AssertionError("each painted glyph must retain its actual encoded CID")
            for glyph, code in zip(painted, codes):
                glyph.code = code
        return matrix


class BidiPdf(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-bidi-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)
        self.assertEqual(hashlib.sha256(FONT.read_bytes()).hexdigest(), FONT_SHA256)

    def render(self, css, body):
        source, target = self.output / "input.html", self.output / "output.pdf"
        source.write_text("<!doctype html><style>@page{size:600pt 500pt;margin:20pt}"
                          "html,body,p{margin:0;padding:0;font:20pt/24pt 'DejaVu Sans'}"
                          + css + "</style>" + body)
        result = subprocess.run([str(RENDERER), str(source), str(target)], capture_output=True,
                                text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        document = pdf_content.Document(target)
        interpreted = BidiInterpreter(document)
        for index, page in enumerate(document.pages(), start=1):
            interpreted.run_page(index, page)
        return interpreted

    def assert_glyphs(self, interpreted, expected):
        self.assertEqual(len(interpreted.glyphs), len(expected))
        with TTFont(FONT) as original:
            for glyph, character in zip(interpreted.glyphs, expected):
                data, gid = interpreted.embedded_glyph_code(glyph.font, glyph.code)
                with TTFont(io.BytesIO(data)) as embedded:
                    name = embedded.getGlyphOrder()[gid]
                    want = original.getBestCmap()[ord(character)]
                    self.assertEqual(embedded["head"].unitsPerEm, original["head"].unitsPerEm)
                    self.assertEqual(outline(embedded, name), outline(original, want),
                                     f"actual painted outline for {character!r}")
                    self.assertAlmostEqual(embedded.getGlyphSet()[name].width,
                                           original.getGlyphSet()[want].width, delta=1)
                self.assertAlmostEqual(glyph.matrix[0], FONT_SIZE_PT, delta=0.00001)
                self.assertAlmostEqual(glyph.matrix[3], FONT_SIZE_PT, delta=0.00001)
        for left, right in zip(interpreted.glyphs, interpreted.glyphs[1:]):
            if left.page == right.page and abs(left.origin[1] - right.origin[1]) < 0.00001:
                self.assertLess(left.origin[0], right.origin[0], "reference glyphs must paint in left-to-right visual order")

    def assert_logical_text(self, text):
        inspected = pdf_content.inspect(self.output / "output.pdf")
        spans = [span.text for span in inspected["replacement_text"]]
        self.assertEqual("\n".join(spans), text, "serialized logical text must retain exact original Unicode")
        # Poppler applies another bidi pass to mixed-script ActualText during export.
        if text.isascii():
            extracted = subprocess.run(["pdftotext", "-raw", str(self.output / "output.pdf"), "-"],
                                       check=True, capture_output=True, text=True, timeout=60).stdout
            extracted = "".join(character for character in extracted if character not in BIDI_FORMATTING).strip()
            self.assertEqual(extracted, text)

    def test_replacement_text_retains_selectable_bounds_and_page_coordinates(self):
        page_height = 500
        font_rule = f"@font-face{{font-family:Probe;src:url('{FONT.as_uri()}')}}section{{font-family:Probe}}p,span,i{{font-family:inherit}}"
        for css, body in [
            ("", "Alpha"),
            ("section{opacity:.5}", "Alpha"),
            ("section{position:relative;left:12pt;top:8pt}", "Alpha"),
            (font_rule + "i{font-style:oblique 20deg}", "<i>Al</i>pha"),
            (font_rule + "i{font-style:oblique 20deg}", "Al<i>pha</i>"),
            (font_rule.replace(FONT.as_uri(), (ROOT / "tests/oracle/assets/STIXGeneral.ttf").as_uri()), "Alpha"),
        ]:
            with self.subTest(css=css, body=body):
                interpreted = self.render(css, f"<p>Header</p><section><p><span dir=ltr>{body}</span></p><p>Beta</p></section>")
                self.assertEqual(len(interpreted.glyphs), len("HeaderAlphaBeta"),
                                 "replacement-text metadata must not add glyphs")
                extracted = subprocess.run(
                    ["pdftotext", "-bbox", str(self.output / "output.pdf"), "-"],
                    check=True, capture_output=True, text=True, timeout=60,
                ).stdout
                words = ET.fromstring(extracted).findall(".//{*}word")
                self.assertCountEqual([word.text for word in words], ["Header", "Alpha", "Beta"])
                for label in ["Alpha", "Beta"]:
                    word = next(word for word in words if word.text == label)
                    start = len("Header") + (0 if label == "Alpha" else len("Alpha"))
                    glyphs = interpreted.glyphs[start:start + len(label)]
                    left, top, right, bottom = [float(word.attrib[key]) for key in ("xMin", "yMin", "xMax", "yMax")]
                    baseline = page_height - glyphs[0].origin[1]
                    self.assertAlmostEqual(left, glyphs[0].origin[0], places=3)
                    self.assertLess(top, baseline, f"{label}: selection starts above the painted baseline")
                    self.assertGreater(bottom, baseline, f"{label}: selection includes the font descent")
                    self.assertGreater(right, glyphs[-1].origin[0], f"{label}: selection reaches the final glyph")
                    self.assertAlmostEqual(bottom - top, FONT_SIZE_PT, places=3,
                                           msg=f"{label}: selection uses the active font's full em height")

    def test_css_bidi_paints_unicode_reference_order_and_preserves_source(self):
        reference = json.loads(REFERENCE.read_text())
        for case in reference["cases"]:
            with self.subTest(unicode_line=case["line"]):
                css = ["p{direction:ltr}", "p{direction:rtl}", "p{unicode-bidi:plaintext}"][case["direction"]]
                interpreted = self.render(css, "<p>" + case["body"] + "</p>")
                codepoints, _, _, levels, order = case["record"].split(";")
                codepoints, levels = codepoints.split(), levels.split()
                expected = []
                for value in order.split():
                    index = int(value)
                    point = codepoints[index]
                    character = chr(int(point, 16))
                    if character in BIDI_FORMATTING:
                        continue
                    if int(levels[index]) % 2:
                        point = reference["mirrors"].get(point, point)
                    expected.append(chr(int(point, 16)))
                self.assert_glyphs(interpreted, expected)
                self.assert_logical_text(case["logical"])

    def test_override_reopens_on_forced_breaks_without_injecting_source_controls(self):
        interpreted = self.render("", "<p><span style='direction:rtl;unicode-bidi:isolate-override'>abc<br>DEF</span></p>")
        self.assert_glyphs(interpreted, "cbaFED")
        self.assert_logical_text("abc\nDEF")

    def test_bidi_scopes_include_generated_and_slotted_content(self):
        css = "bdo::before{content:'12'}bdo::after{content:'34'}"
        body = ("<div><template shadowrootmode=open><p dir=ltr>X <slot></slot> Y</p>"
                "</template><bdo dir=rtl>ABC</bdo></div>")
        interpreted = self.render(css, body)
        self.assert_glyphs(interpreted, "X 43CBA21 Y")
        self.assert_logical_text("X 12ABC34 Y")

    def test_logical_text_survives_an_opacity_group(self):
        interpreted = self.render("section{opacity:.5}", "<section><p><bdo dir=rtl>ABC</bdo></p></section>")
        self.assert_glyphs(interpreted, "CBA")
        self.assert_logical_text("ABC")

    def test_visual_hebrew_encoding_uses_ua_order_and_authored_rollback(self):
        for label, css, expected in [
            ("ISO-8859-8", "", "X אב Y"),
            ("ISO-8859-8-I", "", "X בא Y"),
            ("visual", "p{unicode-bidi:normal}", "X בא Y"),
            ("visual", "p{unicode-bidi:revert}", "X אב Y"),
        ]:
            with self.subTest(label=label, css=css):
                interpreted = self.render(css, f"<meta charset={label}><p>X &#x5d0;&#x5d1; Y</p>")
                self.assert_glyphs(interpreted, expected)
                self.assert_logical_text("X אב Y")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments, remaining = parser.parse_known_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__, *remaining], verbosity=2)
