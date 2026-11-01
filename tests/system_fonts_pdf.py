"""Check that installed fonts from --font-dir folders embed the faces font matching selects."""
import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from urllib.parse import quote

import font_geometry
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/fonts"
RENDERER = ROOT / "target/debug/ferri-pdf"
FAMILY = "System Test"
TEXT = "Hamburg"
SUBSET_TAG_SEPARATOR = "+"
FACES = ["SystemTest-Regular.ttf", "SystemTest-Bold.ttf", "SystemTest-Italic.ttf",
         "SystemTest-BoldItalic.ttf"]
COLLECTION = "SystemTest.ttc"
# The paragraph style that selects each face of `FACES`.
FACE_STYLES = ["", "font-weight:bold", "font-style:italic", "font-weight:bold;font-style:italic"]
WARNING = "warning: "


class SystemFontsPdf(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-system-fonts-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)

    def folder(self, name, files):
        folder = self.output / name
        folder.mkdir()
        for file in files:
            shutil.copy(FIXTURES / file, folder / file)
        return folder

    def run_renderer(self, body, flags, style=""):
        source, target = self.output / "input.html", self.output / "output.pdf"
        source.write_text(f"<!doctype html><base href='{FIXTURES.as_uri()}/'><style>{style}"
                          f"p{{margin:0;font-size:{font_geometry.FONT_SIZE_PT}pt}}</style>{body}")
        rendered = subprocess.run([str(RENDERER), str(source), str(target), *flags],
                                  capture_output=True, text=True, timeout=60)
        return rendered, target

    def render(self, body, flags, style=""):
        rendered, target = self.run_renderer(body, flags, style)
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        document = pdf_content.Document(target)
        interpreter = font_geometry.InstanceInterpreter(document)
        for index, page in enumerate(document.pages(), start=1):
            interpreter.run_page(index, page)
        return interpreter, rendered.stderr

    def assert_lines(self, interpreted, references, text=TEXT):
        """Line N paints `text` with the embedded subset of the face file `references[N]`."""
        glyphs = [glyph for glyph in interpreted.glyphs if glyph.text.strip()]
        self.assertEqual(len(glyphs), len(text) * len(references), [glyph.text for glyph in glyphs])
        for line, reference in enumerate(references):
            for glyph, character in zip(glyphs[line * len(text):], text):
                with self.subTest(face=reference, character=character):
                    _, dictionary = interpreted.instances[glyph.font]
                    base_font = str(dictionary["BaseFont"])
                    self.assertEqual(base_font.split(SUBSET_TAG_SEPARATOR, 1)[1], Path(reference).stem,
                                     "the PDF names the installed face")
                    font_geometry.FontGeometry.assert_font_glyph(self, interpreted, glyph,
                                                                 FIXTURES / reference, character)

    def family_paragraphs(self):
        return "".join(f"<p style=\"font-family:'{FAMILY}';{style}\">{TEXT}</p>" for style in FACE_STYLES)

    def test_family_faces_embed_from_separate_files_and_a_collection(self):
        for index, files in enumerate([FACES, [COLLECTION]]):
            with self.subTest(files=files):
                folder = self.folder(f"folder{index}", files)
                interpreted, stderr = self.render(self.family_paragraphs(), ["--font-dir", str(folder)])
                self.assertNotIn(WARNING, stderr)
                self.assert_lines(interpreted, FACES)

    def test_local_sources_embed_the_installed_face(self):
        folder = self.folder("local", [COLLECTION])
        style = ("@font-face{font-family:Bold;src:local('System Test Bold')}"
                 "@font-face{font-family:Italic;src:local(SystemTest-Italic)}")
        body = f"<p style='font-family:Bold'>{TEXT}</p><p style='font-family:Italic'>{TEXT}</p>"
        interpreted, stderr = self.render(body, ["--font-dir", str(folder)], style)
        self.assertNotIn(WARNING, stderr)
        self.assert_lines(interpreted, ["SystemTest-Bold.ttf", "SystemTest-Italic.ttf"])

    def test_svg_text_embeds_the_collection_face_it_selects(self):
        folder = self.folder("svg", [COLLECTION])
        svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="200" height="40">'
               f'<text x="0" y="30" font-family="{FAMILY}" font-size="20pt" font-weight="bold">'
               'H</text></svg>')
        interpreted, stderr = self.render(f'<img src="data:image/svg+xml,{quote(svg)}">',
                                          ["--font-dir", str(folder)])
        self.assertNotIn(WARNING, stderr)
        self.assert_lines(interpreted, ["SystemTest-Bold.ttf"], text="H")

    def test_generic_preferences_name_installed_families(self):
        folder = self.folder("preferences", FACES)
        body = f"<p style='font-family:sans-serif;font-weight:bold'>{TEXT}</p>"
        interpreted, stderr = self.render(body, ["--font-dir", str(folder), "--font-sans-serif", FAMILY])
        self.assertNotIn(WARNING, stderr)
        self.assert_lines(interpreted, ["SystemTest-Bold.ttf"])

    def test_missing_families_and_folders_warn_and_still_write_the_pdf(self):
        missing = self.output / "absent"
        body = (f"<p style=\"font-family:'Absent Family',serif\">{TEXT}</p>"
                f"<p style=\"font-family:serif,'Never Reached'\">{TEXT}</p>")
        rendered, target = self.run_renderer(body, ["--font-dir", str(missing)])
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        self.assertTrue(target.read_bytes().startswith(b"%PDF-"))
        warnings = [line for line in rendered.stderr.splitlines() if line.startswith(WARNING)]
        self.assertEqual(len(warnings), 2, rendered.stderr)
        self.assertIn('"Absent Family"', rendered.stderr)
        self.assertNotIn("Never Reached", rendered.stderr)
        self.assertIn(str(missing), rendered.stderr)

    def test_flags(self):
        rendered, target = self.run_renderer(f"<p>{TEXT}</p>", ["--system-fonts"])
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        self.assertTrue(target.read_bytes().startswith(b"%PDF-"))
        rendered, _ = self.run_renderer(f"<p>{TEXT}</p>", ["--font-dir"])
        self.assertNotEqual(rendered.returncode, 0, "--font-dir requires a folder")
        self.assertIn("usage:", rendered.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
