"""Check that WOFF, WOFF2 and collection @font-face sources embed their packaged faces."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import unittest

import font_geometry
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/fonts"
RENDERER = ROOT / "target/debug/ferri-pdf"
REGULAR = FIXTURES / "WoffTest-Regular.ttf"
BOLD = FIXTURES / "WoffTest-Bold.ttf"
TEXT = "Hamburg É"
SUBSET_TAG_SEPARATOR = "+"
SOURCES = [
    ("url('WoffTest-Regular.woff') format('woff')", REGULAR),
    ("url('WoffTest-Regular.woff2') format('woff2')", REGULAR),
    ("url('WoffTest-Regular-NullGlyf.woff2') format('woff2')", REGULAR),
    ("url('WoffTest-Collection.woff2') format('woff2')", REGULAR),
    ("url('WoffTest-Collection.woff2#WoffTest-Bold') format('woff2')", BOLD),
    ("url('WoffTest.ttc#WoffTest-Bold') format('collection')", BOLD),
]


class WebFontsPdf(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-web-fonts-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)

    def render(self, faces, body):
        source, target = self.output / "input.html", self.output / "output.pdf"
        source.write_text(f"<!doctype html><base href='{FIXTURES.as_uri()}/'><style>{faces}"
                          f"p{{margin:0;font-size:{font_geometry.FONT_SIZE_PT}pt}}</style>{body}")
        rendered = subprocess.run([str(RENDERER), str(source), str(target)],
                                  capture_output=True, text=True, timeout=60)
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        document = pdf_content.Document(target)
        interpreter = font_geometry.InstanceInterpreter(document)
        for index, page in enumerate(document.pages(), start=1):
            interpreter.run_page(index, page)
        return interpreter, rendered.stderr

    def assert_lines(self, interpreted, expected):
        """Each line paints `TEXT` with the embedded subset of its reference face."""
        glyphs = [glyph for glyph in interpreted.glyphs if glyph.text.strip()]
        characters = [character for character in TEXT if character.strip()]
        self.assertEqual(len(glyphs), len(characters) * len(expected))
        for line, (case, reference) in enumerate(expected):
            for glyph, character in zip(glyphs[line * len(characters):], characters):
                with self.subTest(source=case, character=character):
                    _, dictionary = interpreted.instances[glyph.font]
                    base_font = str(dictionary["BaseFont"])
                    self.assertEqual(base_font.split(SUBSET_TAG_SEPARATOR, 1)[1], reference.stem,
                                     "the PDF names the packaged face")
                    font_geometry.FontGeometry.assert_font_glyph(self, interpreted, glyph, reference, character)

    def test_packaged_faces_embed_their_own_glyphs(self):
        faces = "".join(f"@font-face{{font-family:Web{index};src:{source}}}"
                        for index, (source, _) in enumerate(SOURCES))
        body = "".join(f"<p style='font-family:Web{index}'>{TEXT}</p>" for index in range(len(SOURCES)))
        interpreted, stderr = self.render(faces, body)
        self.assertEqual(stderr.count("warning"), 0, stderr)
        self.assert_lines(interpreted, SOURCES)

    def test_damaged_packages_fall_back_to_the_next_source(self):
        intact = (FIXTURES / "WoffTest-Regular.woff2").read_bytes()
        damaged = self.output / "damaged.woff2"
        damaged.write_bytes(intact[:len(intact) // 2])
        faces = (f"@font-face{{font-family:Web;src:url('{damaged.as_uri()}') format('woff2'),"
                 f"url('WoffTest-Bold.ttf')}}")
        interpreted, stderr = self.render(faces, f"<p style='font-family:Web'>{TEXT}</p>")
        self.assertIn("damaged.woff2", stderr, "the skipped source is reported")
        self.assert_lines(interpreted, [("damaged WOFF2", BOLD)])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
