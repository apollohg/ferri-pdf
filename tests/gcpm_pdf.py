"""Check CSS GCPM 3 page selection, running elements and footnotes in rendered PDFs."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
FONT = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
BASE_CSS = ("@font-face{font-family:Probe;src:url('" + FONT + "')}"
            "html,body,div,p,h1{margin:0;padding:0;font:10pt/10pt Probe}")
POSITION_TOLERANCE = 0.01


def pdf_pages(pdf):
    """Each page's size and words with their boxes."""
    xml = subprocess.run(["pdftotext", "-bbox", str(pdf), "-"],
                         check=True, capture_output=True, timeout=60).stdout
    pages = []
    for page in ET.fromstring(xml).findall(".//{*}page"):
        words = [(word.text, *(float(word.attrib[key]) for key in ("xMin", "yMin", "xMax", "yMax")))
                 for word in page.findall(".//{*}word")]
        pages.append(((float(page.attrib["width"]), float(page.attrib["height"])), words))
    return pages


class GcpmPdf(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-gcpm-")
        self.output = Path(self.directory.name)

    def tearDown(self):
        self.directory.cleanup()

    def render(self, css, body):
        source = self.output / "source.html"
        pdf = self.output / "output.pdf"
        source.write_text("<!doctype html><style>" + BASE_CSS + css + "</style>" + body)
        result = subprocess.run([str(RENDERER), str(source), str(pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        return pdf_pages(pdf)

    def test_nth_page_selectors_size_and_label_pages_of_documents_and_groups(self):
        pages = self.render(
            "@page{size:200pt 200pt;margin:20pt;@top-left{content:'p' counter(page)}}"
            "@page :nth(2n){size:210pt 200pt}"
            "@page :nth(1 of chapter){size:300pt 200pt;margin-left:40pt;"
            "@top-left{content:'open'}}"
            "@page :nth(2 of chapter){size:310pt 200pt}"
            "@page part:nth(2){size:400pt 200pt}"
            ".chapter{page:chapter;break-before:right}.part{page:part;break-before:page}"
            "p{height:160pt}",
            "<div class=chapter><p>C1a</p><p>C1b</p><p>C1c</p></div>"
            "<div class=chapter><p>C2a</p><div class=part><p>B1</p><p>B2</p></div></div>"
            "<p>Tail</p>")
        expected = [((300, "open"), "C1a"), ((310, "p2"), "C1b"), ((200, "p3"), "C1c"),
                    ((210, "p4"), None), ((300, "open"), "C2a"), ((310, "p6"), "B1"),
                    ((400, "p7"), "B2"), ((210, "p8"), "Tail")]
        self.assertEqual(len(pages), len(expected), [page[1] for page in pages])
        for number, (((width, height), words), ((expected_width, label), body)) in enumerate(
                zip(pages, expected), 1):
            with self.subTest(page=number):
                self.assertEqual((width, height), (expected_width, 200))
                texts = [word[0] for word in words]
                self.assertEqual(texts, [label] + ([body] if body else []),
                                 "a blank page before a right-page group start keeps the "
                                 "document count but joins no group")
                left = 40 if label == "open" else 20
                body_words = [word for word in words if word[0] == body]
                for word in body_words:
                    self.assertAlmostEqual(word[1], left, delta=POSITION_TOLERANCE,
                                           msg=f"page {number} content edge")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
