"""Check CSS GCPM 3 page selection, running elements and footnotes in rendered PDFs."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

from PIL import Image
from oracle import pdf_content

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
        self.pdf = self.output / "output.pdf"
        source.write_text("<!doctype html><style>" + BASE_CSS + css + "</style>" + body)
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        return pdf_pages(self.pdf)

    def margin_words(self, pages, region):
        """Words per page inside a page-region predicate on their boxes."""
        return [[word[0] for word in words if region(word)] for _, words in pages]

    def test_running_elements_choose_assignments_per_page_and_keep_their_formatting(self):
        pages = self.render(
            "@page{size:300pt 200pt;margin:40pt 20pt;"
            "@top-left{content:element(head)}@top-right{content:element(head,last)}"
            "@bottom-left{content:element(foot,start)}"
            "@bottom-right{content:element(foot,first-except)}}"
            "h1{position:running(head);padding-left:3pt;color:rgb(0,0,255);font-weight:normal}"
            "h1 i{color:rgb(255,0,0);font-style:normal}"
            ".foot{position:running(foot)}p{height:60pt}",
            "<h1>Alpha <i>Red</i></h1><div class=foot>FootA</div><p>p1</p><p>p2</p>"
            "<p>p3</p><h1>Beta</h1><p>p4</p>"
            "<p>p5</p><div class=foot>FootB</div><p>p6</p>"
            "<div><div class=foot>FootC</div><p>p7</p></div>")
        self.assertEqual(len(pages), 4)
        top_left = self.margin_words(pages, lambda word: word[2] < 40 and word[1] < 150)
        top_right = self.margin_words(pages, lambda word: word[2] < 40 and word[1] >= 150)
        bottom_left = self.margin_words(pages, lambda word: word[2] > 160 and word[1] < 150)
        bottom_right = self.margin_words(pages, lambda word: word[2] > 160 and word[1] >= 150)
        self.assertEqual(top_left, [["Alpha", "Red"], ["Beta"], ["Beta"], ["Beta"]],
                         "first: the page's first assignment, else the entry value")
        self.assertEqual(top_right, [["Alpha", "Red"], ["Beta"], ["Beta"], ["Beta"]],
                         "last: the exit value")
        self.assertEqual(bottom_left, [[], ["FootA"], ["FootA"], ["FootC"]],
                         "start: the first assignment only when its element starts the page")
        self.assertEqual(bottom_right, [[], ["FootA"], [], []],
                         "first-except: empty on pages that assign the element")
        body = self.margin_words(pages, lambda word: 40 <= word[2] <= 160)
        self.assertEqual(body, [["p1", "p2"], ["p3", "p4"], ["p5", "p6"], ["p7"]],
                         "running elements leave the normal flow")
        alpha = next(word for word in pages[0][1] if word[0] == "Alpha")
        self.assertAlmostEqual(alpha[1], 23, delta=POSITION_TOLERANCE,
                               msg="the element's padding offsets it inside the margin box")
        self.assertAlmostEqual((alpha[2] + alpha[4]) / 2, 20, delta=1,
                               msg="the element is vertically centred in the top margin")
        fills = {(glyph.page, glyph.text): glyph.fill[:3]
                 for glyph in pdf_content.inspect(self.pdf)["glyphs"]}
        self.assertEqual(fills[(1, "A")], (0, 0, 1), "the element keeps its own colour")
        self.assertEqual(fills[(1, "R")], (1, 0, 0), "descendants keep their formatting")

    def raster(self, page):
        """The page rendered at one pixel per point."""
        prefix = self.output / f"page-{page}"
        subprocess.run(["pdftoppm", "-r", "72", "-f", str(page), "-l", str(page), "-png",
                        str(self.pdf), str(prefix)], check=True, capture_output=True, timeout=60)
        with Image.open(next(self.output.glob(f"page-{page}*.png"))) as image:
            return image.convert("RGB")

    def colored_rows(self, image, color):
        return sorted({y for y in range(image.height) for x in range(image.width)
                       if image.getpixel((x, y)) == color})

    def test_footnotes_leave_calls_and_stack_in_the_area_at_the_page_area_bottom(self):
        pages = self.render(
            "@page{size:300pt 200pt;margin:20pt;"
            "@footnote{border-top:2pt solid rgb(255,0,0);padding-top:4pt}}"
            ".fn{float:footnote;font:8pt/10pt Probe}p{margin-bottom:6pt}",
            "<p>Alpha<span class=fn>One note.</span> text.</p>"
            "<p>Beta<span class=fn>Two note.</span> text.</p>")
        self.assertEqual(len(pages), 1)
        words = pages[0][1]
        self.assertEqual([word[0] for word in words],
                         ["Alpha", "1", "text.", "Beta", "2", "text.", "1.", "One", "note.",
                          "2.", "Two", "note."],
                         "calls replace the footnote elements, whose bodies follow in order")
        notes = [word for word in words if word[0] in ("1.", "2.")]
        self.assertAlmostEqual(notes[1][2] - notes[0][2], 10, delta=POSITION_TOLERANCE,
                               msg="block footnotes stack one line box apart")
        self.assertTrue(all(abs(word[1] - 20) <= POSITION_TOLERANCE for word in notes),
                        "footnote markers start at the area's content edge")
        self.assertLessEqual(notes[1][4], 180, "the area ends at the page area's bottom")
        separator = self.colored_rows(self.raster(1), (255, 0, 0))
        self.assertEqual(separator, [154, 155],
                         "the area's top border lies above 4pt padding and two 10pt lines")
        glyphs = [glyph for glyph in pdf_content.inspect(self.pdf)["glyphs"]
                  if glyph.page == 1 and glyph.text in ("a", "1")]
        # PDF space rises from the page bottom: the body's first line has the highest baselines.
        body_baseline = max(glyph.matrix[5] for glyph in glyphs if glyph.text == "a")
        call_baseline = max(glyph.matrix[5] for glyph in glyphs if glyph.text == "1")
        self.assertGreater(call_baseline, body_baseline,
                           "the call is superscripted by font-variant-position: super")

    def test_footnotes_beyond_max_height_continue_on_the_next_page(self):
        lines = "<br>".join(f"A{index}" for index in range(1, 8))
        note = "<br>".join(f"N{index}" for index in range(1, 7))
        pages = self.render(
            "@page{size:200pt 140pt;margin:20pt;@footnote{max-height:40pt}}"
            "html,body,p,span{font:10pt/10pt Probe}p{widows:1;orphans:1}"
            ".fn{float:footnote;color:rgb(255,0,0)}",
            f"<p>{lines}<span class=fn>{note}</span><br>A8<br>A9</p>")
        self.assertEqual(len(pages), 2)
        first = self.colored_rows(self.raster(1), (255, 0, 0))
        second = self.colored_rows(self.raster(2), (255, 0, 0))
        self.assertTrue(first and 90 <= first[0] and first[-1] < 120,
                        f"page 1 shows the footnote's first lines below A7 only: {first}")
        self.assertTrue(second and 90 <= second[0] and second[-1] < 120,
                        f"page 2 continues the footnote at its area's top: {second}")
        body = [word[0] for word in pages[1][1] if word[0].startswith("A")]
        self.assertEqual(body, ["A8", "A9"], "the flow resumes above the continued footnote")

    def test_footnote_policies_move_the_call_line_or_paragraph_with_the_footnote(self):
        lines = "<br>".join(f"A{index}" for index in range(1, 8))
        note = "<br>".join(f"N{index}" for index in range(1, 7))
        css = ("@page{size:200pt 140pt;margin:20pt}"
               "html,body,p,span{font:10pt/10pt Probe}p{widows:1;orphans:1}"
               ".fn{float:footnote}")
        body = f"<p>P0</p><p>{lines}<span class=fn>{note}</span><br>A8</p>"
        for policy, first in (("auto", ["P0", "A1", "A2", "A3", "A4", "A5", "A6", "A7"]),
                              ("line", ["P0", "A1", "A2", "A3", "A4", "A5", "A6"]),
                              ("block", ["P0"])):
            with self.subTest(policy=policy):
                pages = self.render(css + f".fn{{footnote-policy:{policy}}}", body)
                self.assertEqual([word[0] for word in pages[0][1] if word[0][0] in "AP"],
                                 first)

    def test_compact_footnotes_share_a_line_when_they_fit(self):
        pages = self.render(
            "@page{size:300pt 200pt;margin:20pt}"
            ".fn{float:footnote;footnote-display:compact;margin-right:6pt}",
            "<p>Alpha<span class=fn>One.</span> and<span class=fn>Two.</span></p>"
            "<p>Gamma<span class=fn>A footnote too long to share its line with any other "
            "footnote at all.</span></p>")
        words = {word[0]: word for word in pages[0][1]}
        self.assertAlmostEqual(words["1."][2], words["2."][2], delta=POSITION_TOLERANCE,
                               msg="short compact footnotes share a line")
        self.assertAlmostEqual(words["2."][1], words["One."][3] + 6, delta=0.5,
                               msg="the second follows the first's margin box")
        self.assertGreater(words["3."][2], words["1."][2],
                           "a footnote wider than the area's remainder starts a line")

    def columns(self, image, matches):
        return sorted({x for y in range(image.height) for x in range(image.width)
                       if matches(image.getpixel((x, y)))})

    def test_vertical_footnotes_lie_at_the_block_end_with_physical_area_edges(self):
        self.render(
            "@page{size:200pt 150pt;margin:20pt;@footnote{border-top:2pt solid rgb(0,255,0);"
            "border-right:2pt solid rgb(255,0,0);padding-right:4pt}}"
            "html{writing-mode:vertical-rl}.fn{float:footnote;color:rgb(0,0,255)}",
            "<p>Body<span class=fn>Note</span> text</p>")
        image = self.raster(1)
        # Antialiased glyph edges blend with the white page; their cores keep the blue hue.
        blue = self.columns(image, lambda pixel: pixel[2] > 200 and max(pixel[:2]) < 120)
        note = [x for x in blue if x < 100]
        call = [x for x in blue if x >= 100]
        self.assertTrue(note and note[0] >= 20 and note[-1] < 30,
                        f"the note's 10pt line lies at the page area's left, its block end: {blue}")
        self.assertTrue(call and call[-1] <= 180,
                        f"the call, which inherits the note's colour, stays in the first line: {blue}")
        self.assertEqual(self.columns(image, lambda pixel: pixel == (255, 0, 0)), [34, 35],
                         "border-right is the area's physical right edge after 4pt padding")
        self.assertEqual(self.colored_rows(image, (0, 255, 0)), [20, 21],
                         "border-top is the area's physical top edge at the page area's top")

    def test_running_elements_inside_fragmented_paragraphs_assign_where_their_line_falls(self):
        lines = "<br>".join(f"L{index}" for index in range(1, 10))
        pages = self.render(
            "@page{size:200pt 100pt;margin:30pt 20pt;@top-left{content:element(mark,last)}}"
            ".mark{position:running(mark)}p{widows:1;orphans:1}",
            f"<p>{lines}<span class=mark>Mark</span><br>L10<br>L11</p>")
        margin = self.margin_words(pages, lambda word: word[2] < 30)
        body = self.margin_words(pages, lambda word: word[2] >= 30)
        self.assertEqual(body, [["L1", "L2", "L3", "L4"], ["L5", "L6", "L7", "L8"],
                                ["L9", "L10", "L11"]])
        self.assertEqual(margin, [[], [], ["Mark"]],
                         "an inline running element assigns on the page of its line")

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
