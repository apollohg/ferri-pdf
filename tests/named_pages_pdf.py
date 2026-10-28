"""Check mixed named-page output independently of the layout trace."""
import argparse
import json
import re
from pathlib import Path
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

from PIL import Image
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
PAGE_EXPECTATIONS = [(300, 400, 20, 2, (255, 0, 0)),
                     (500, 300, 10, 4, (0, 0, 255)),
                     (300, 400, 20, 2, (255, 0, 0))]
FIXED_SIZE_PT = 6
PROBE_FIRST_BASELINE_PT = 18.4617919921875
PROBE_LINE_HEIGHT_PT = 10


def pdf_text_pages(pdf):
    xml = subprocess.run(["pdftotext", "-bbox", str(pdf), "-"],
                         check=True, capture_output=True, timeout=60).stdout
    return ET.fromstring(xml).findall(".//{*}page")


class NamedPages(unittest.TestCase):
    def test_full_height_document_omits_unused_sheet_before_counting_and_fixed_paint(self):
        reference = {
            "font_path": "tests/oracle/assets/DejaVuSans.ttf",
            "base_css": "@page{size:240pt 240pt;margin:20pt;font:10pt Probe;"
                        "@bottom-center{content:counter(page) '/' counter(pages)}}"
                        "html,body{margin:0}body{font:12pt/16pt Probe;background:#eeeeee}"
                        "h2{font:12pt/16pt Probe;margin:20pt 0 10pt}p{margin:0}"
                        "footer{position:fixed;bottom:0;right:0}",
        }
        body = "<h2>Contents</h2><p>Entry</p><footer>Fixed</footer>"
        control = self.render_reference(reference, {"css": "", "body": body})
        pages = self.render_reference(reference, {
            "css": "html,body{height:100%}", "body": body,
        })
        self.assertEqual(len(pages), 1, "unused body height must not add a physical sheet")
        words = lambda page: [(word.text, word.attrib) for word in page.findall(".//{*}word")]
        self.assertEqual(words(pages[0]), words(control[0]),
                         "removing unused height must preserve all native glyph coordinates")
        self.assertCountEqual([word.text for word in pages[0].findall(".//{*}word")],
                              ["Contents", "Entry", "Fixed", "1/1"])

    def render_reference(self, reference, case):
        font = (ROOT / reference["font_path"]).as_uri()
        source = self.output / "reference.html"
        source.write_text(
            "<!doctype html><style>@font-face{font-family:Probe;src:url('" + font + "')}"
            + reference["base_css"] + case["css"] + "</style>" + case["body"]
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        return pdf_text_pages(self.pdf)

    def test_predefined_page_counter_styles_reach_pdf_text(self):
        reference = {
            "font_path": "tests/oracle/assets/DejaVuSans.ttf",
            "base_css": "@page{size:300pt 200pt;margin:36pt;font:10pt Probe}"
                        "html,body{margin:0}body{font:10pt Probe}p+p{break-before:page}",
        }
        case = {
            "css": "@page{@top-left{content:counter(page,lower-roman) '|' counter(page,upper-alpha) '|' counter(page,lower-greek) '|' counter(page,decimal-leading-zero)}"
                   "@bottom-left{content:'Total:' counter(pages,upper-roman)}"
                   "@bottom-right{content:'[' counter(page,disc) counter(page,circle) counter(page,square) counter(page,none) ']'}}",
            "body": "<p>Body</p><p>Body</p><p>Body</p>",
        }
        pages = self.render_reference(reference, case)
        self.assertEqual(len(pages), 3)
        for number, (page, heading) in enumerate(zip(pages, ["i|A|α|01", "ii|B|β|02", "iii|C|γ|03"]), 1):
            words = [word.text for word in page.findall(".//{*}word")]
            self.assertCountEqual(words, ["Body", heading, "Total:III", "[•◦▪]"],
                                  f"page {number}: counter representations must reach the PDF without marker suffixes")

    def test_document_counters_and_reversed_lists_reach_pdf_and_bookmarks(self):
        reference = {
            "font_path": "tests/oracle/assets/DejaVuSans.ttf",
            "base_css": "@page{size:300pt 300pt;margin:36pt}html,body{margin:0}"
                        "body{font:10pt Probe;counter-reset:chapter}h1{font:10pt Probe;margin:0}"
                        "h1+h1{break-before:page}",
        }
        case = {
            "css": "h1{counter-increment:chapter;bookmark-label:counter(chapter,upper-roman) ' ' content(text)}"
                   "h1::before{content:counter(chapter,upper-roman) ': '}"
                   "li::before{content:'[' counter(list-item) '] '}",
            "body": "<h1>First</h1><h1>Second</h1><ol reversed start=4><li>Alpha</li><li>Beta</li></ol>",
        }
        pages = self.render_reference(reference, case)
        self.assertEqual(len(pages), 2)
        self.assertEqual([word.text for word in pages[0].findall(".//{*}word")], ["I:", "First"])
        self.assertEqual([word.text for word in pages[1].findall(".//{*}word")],
                         ["II:", "Second", "4.", "[4]", "Alpha", "3.", "[3]", "Beta"])
        outline = pdf_content.navigation(self.pdf)["outline"]
        self.assertEqual([(item["title"], item["destination"]["page"]) for item in outline],
                         [("I First", 1), ("II Second", 2)])

    def test_first_letter_generated_text_reaches_running_headers_and_bookmarks(self):
        reference = {
            "font_path": "tests/oracle/assets/DejaVuSans.ttf",
            "base_css": "@page{size:300pt 200pt;margin:36pt;font:10pt Probe}"
                        "html,body{margin:0}body{font:10pt Probe}h2{font:10pt Probe;margin:0}",
        }
        case = {
            "css": "h2{string-set:heading-initial content(first-letter);bookmark-label:content(first-letter)}"
                   "h2::before{content:'Note: '}@page{@top-left{content:string(heading-initial)}}",
            "body": "<h2>Launch</h2>",
        }
        pages = self.render_reference(reference, case)
        self.assertEqual(len(pages), 1)
        self.assertCountEqual([word.text for word in pages[0].findall(".//{*}word")],
                              ["N", "Note:", "Launch"])
        outline = pdf_content.navigation(self.pdf)["outline"]
        self.assertEqual([(item["title"], item["destination"]["page"]) for item in outline],
                         [("N", 1)], "bookmark and running string must share first-letter extraction")

    def test_running_strings_capture_counters_and_resolve_each_pdf_page(self):
        margin_pt = 36
        page_height_pt = 300
        reference = {
            "font_path": "tests/oracle/assets/DejaVuSans.ttf",
            "base_css": f"@page{{size:500pt {page_height_pt}pt;margin:{margin_pt}pt;font:10pt Probe}}"
                        "html,body{margin:0}body{font:10pt Probe;counter-reset:chapter}"
                        "h2{font:10pt Probe;margin:0;break-after:auto}.next{break-before:page}",
        }
        case = {
            "css": "title{string-set:document content()}"
                   "h2{counter-increment:chapter;string-set:heading counter(chapter,upper-roman) ':' content(),folio 'Assigned' counter(page) '/' counter(pages)}"
                   "@page{@top-left{content:'F[' string(heading,first) ']S[' string(heading,start) ']L[' string(heading,last) ']X[' string(heading,first-except) ']'}"
                   "@top-right{content:string(document)}@bottom-left{content:string(folio)}}",
            "body": "<title>MANUAL</title><p>Intro</p><h2>Alpha</h2><p class=next>Continuation</p><h2 class=next>Beta</h2>",
        }
        pages = self.render_reference(reference, case)
        expected = [
            ("F[I:Alpha]S[]L[I:Alpha]X[]", "Assigned1/3", ["Intro", "Alpha"]),
            ("F[I:Alpha]S[I:Alpha]L[I:Alpha]X[I:Alpha]", "Assigned1/3", ["Continuation"]),
            ("F[II:Beta]S[II:Beta]L[II:Beta]X[]", "Assigned3/3", ["Beta"]),
        ]
        self.assertEqual(len(pages), len(expected))
        for number, (page, (header, footer, body)) in enumerate(zip(pages, expected), 1):
            words = page.findall(".//{*}word")
            self.assertCountEqual([word.text for word in words], [header, footer, "MANUAL", *body],
                                  f"page {number}: assignments and carried page counters must reach PDF text")
            heading = next(word for word in words if word.text == header)
            folio = next(word for word in words if word.text == footer)
            self.assertLess(float(heading.attrib["yMax"]), margin_pt, f"page {number}: header belongs in the top margin")
            self.assertGreater(float(folio.attrib["yMin"]), page_height_pt - margin_pt, f"page {number}: folio belongs in the bottom margin")

    def test_initial_and_fragmented_margins_match_independent_native_baselines(self):
        reference = json.loads((ROOT / "tests/fixtures/flow/weasy70-margins.json").read_text())
        initial_margin_pt = 12
        control = next(case for case in reference["cases"] if case["name"] == "reset-body")
        baseline = control["pages"][0][0]["baseline"]
        automatic_columns = {**reference["observations"][0], "pages": [
            [{"text": "First", "baseline": baseline + initial_margin_pt},
             {"text": "Second", "baseline": baseline}],
            [{"text": "Third", "baseline": baseline}, {"text": "Fourth", "baseline": baseline}]]}
        self.assert_native_reference_baselines(reference, reference["cases"] + [automatic_columns])

    def test_list_default_breaks_preserve_native_content_and_reference_baselines(self):
        reference = json.loads((ROOT / "tests/fixtures/flow/weasy70-list-breaks.json").read_text())
        self.assert_native_reference_baselines(reference, reference["cases"])

    def test_oversized_avoidance_tries_a_fresh_native_page_or_column(self):
        reference = json.loads((ROOT / "tests/fixtures/flow/weasy70-avoid.json").read_text())
        cases = []
        for case in reference["cases"]:
            if case["name"] == "positioned-lead":
                # CSS 2.2 E.2 paints positioned auto layers after normal flow.
                first = case["pages"][0]
                case = {**case, "pages": [first[1:] + first[:1], *case["pages"][1:]]}
            cases.append(case)
        self.assert_native_reference_baselines(reference, cases)

    def test_forced_descendant_keeps_its_fitting_native_prefix_and_source(self):
        reference = json.loads((ROOT / "tests/fixtures/flow/weasy70-avoid.json").read_text())
        lead_height_pt = 30
        labels = [["Lead", "A00", "A01", "A02", "A03", "A04"],
                  ["A05", "A06", "A07", "A08", "A09", "A10", "A11", "A12", "A13", "A14"],
                  ["After"]]
        pages = []
        for number, page in enumerate(labels):
            pages.append([{"text": label, "baseline": PROBE_FIRST_BASELINE_PT + (
                lead_height_pt + (index - 1) * PROBE_LINE_HEIGHT_PT if number == 0 and index > 0
                else index * PROBE_LINE_HEIGHT_PT)} for index, label in enumerate(page)])
        case = {**reference["observations"][0], "pages": pages}
        self.assert_native_reference_baselines(reference, [case])

    def test_avoided_bottom_caption_starts_fresh_and_keeps_its_final_line_with_after(self):
        reference = json.loads((ROOT / "tests/fixtures/flow/weasy70-avoid.json").read_text())
        case = next(case for case in reference["observations"] if case["name"] == "caption-bottom-avoid")
        caption_words, words_per_wide_line, words_per_narrow_line, wide_lines = 60, 6, 3, 9
        labels = [f"S{index:03}" for index in range(caption_words)]
        boundary = words_per_wide_line * wide_lines
        texts = [["Cell"], [" ".join(labels[index:index + words_per_wide_line])
                           for index in range(0, boundary, words_per_wide_line)],
                 [" ".join(labels[index:index + words_per_narrow_line])
                  for index in range(boundary, caption_words, words_per_narrow_line)] + ["After"]]
        pages = [[{"text": text, "baseline": PROBE_FIRST_BASELINE_PT + index * PROBE_LINE_HEIGHT_PT}
                  for index, text in enumerate(page)] for page in texts]
        self.assert_native_reference_baselines(reference, [{**case, "pages": pages}])
        for page, width, bottom, color in [(1, 118, 20, (0, 255, 0)),
                                            (2, 198, 110, (0, 0, 255)),
                                            (3, 118, 30, (0, 0, 255))]:
            prefix = self.output / f"avoided-caption-{page}"
            subprocess.run(["pdftoppm", "-f", str(page), "-l", str(page), "-singlefile", "-r", "72",
                            "-png", str(self.pdf), str(prefix)], check=True, capture_output=True, timeout=60)
            with Image.open(prefix.with_suffix(".png")) as rendered:
                raster = rendered.convert("RGB")
                self.assertEqual(raster.getpixel((width, 12)), color, f"page {page} start")
                self.assertEqual(raster.getpixel((width, bottom - 2)), color, f"page {page} end")
                self.assertEqual(raster.getpixel((width, bottom + 2)), (255, 255, 255), f"page {page} below end")
                self.assertEqual(raster.getpixel((width + 4, 12)), (255, 255, 255))

    def assert_native_reference_baselines(self, reference, cases):
        baseline_tolerance_pt = 0.03
        for case in cases:
            with self.subTest(name=case["name"]):
                pages = self.render_reference(reference, case)
                self.assertEqual(len(pages), len(case["pages"]))
                characters = [(character, glyph) for glyph in pdf_content.glyphs(self.pdf)
                              for character in glyph.text]
                for number, (page, expected) in enumerate(zip(pages, case["pages"]), 1):
                    painted = [(character, glyph) for character, glyph in characters if glyph.page == number]
                    text = "".join(character for character, _ in painted)
                    self.assertEqual(text, "".join(line["text"] for line in expected),
                                     f"{case['name']} page {number}: every authored glyph exactly once")
                    for line in expected:
                        self.assertEqual(text.count(line["text"]), 1, text)
                        glyph = painted[text.index(line["text"])][1]
                        baseline = float(page.attrib["height"]) - glyph.origin[1]
                        self.assertAlmostEqual(baseline, line["baseline"], delta=baseline_tolerance_pt,
                                               msg=f"{case['name']} page {number} {line['text']}")

    def test_initial_collapsed_margins_position_native_background_ink(self):
        reference = json.loads((ROOT / "tests/fixtures/flow/weasy70-margins.json").read_text())
        tops = {"ua-body": 16, "reset-body": 10, "ua-heading": 24.94,
                "body-heading-collapse": 30, "nested-empty-collapse": 40,
                "mixed-sign-collapse": 26, "negative-start": 5,
                "body-barrier": 33, "root-body-margins": 31,
                "percentage-collapsed": 30.8}
        for case in reference["cases"]:
            if case["name"] not in tops:
                continue
            with self.subTest(name=case["name"]):
                self.render_reference(reference, {**case, "css": case["css"] + "p,h2{background:blue}"})
                prefix = self.output / "margin-ink"
                subprocess.run(["pdftoppm", "-f", "1", "-l", "1", "-singlefile", "-r", "72",
                                "-png", str(self.pdf), str(prefix)],
                               check=True, capture_output=True, timeout=60)
                top = int(tops[case["name"]])
                with Image.open(prefix.with_suffix(".png")) as rendered:
                    raster = rendered.convert("RGB")
                    self.assertEqual(raster.getpixel((100, top - 2)), (255, 255, 255))
                    self.assertEqual(raster.getpixel((100, top + 2)), (0, 0, 255))
                    self.assertEqual(raster.getpixel((100, top + 12)), (0, 0, 255))

    def test_ordinary_rows_split_against_independent_weasyprint_70_page_content(self):
        self.assert_table_reference("weasy70-rows.json")

    def test_sliced_cell_edges_keep_intermediate_capacity_and_final_padding(self):
        self.assert_table_reference("weasy70-cell-edges.json")

    def test_column_spanners_preserve_every_authored_line_and_final_padding(self):
        reference = json.loads((ROOT / "tests/fixtures/tables/weasy70-cell-edges.json").read_text())
        expected = {
            "columns-nonfinal-spanner": [[f"A{i:02}" for i in range(19)] + ["B00"],
                                          ["Span"] + [f"C{i:02}" for i in range(18)],
                                          ["C18", "After"]],
            "columns-spanner-ending-block": [["A00", "Span", "B00"], ["C00", "C01", "After"]],
        }
        cases = {case["name"]: case for case in reference["observations"]}
        for name, labels in expected.items():
            with self.subTest(name=name):
                pages = self.render_reference(reference, cases[name])
                self.assertEqual(len(pages), len(labels))
                for index, page in enumerate(pages):
                    self.assertCountEqual([word.text for word in page.findall(".//{*}word")], labels[index],
                                          f"{name} page {index}: authored source cannot disappear around a spanner")

    def assert_table_reference(self, filename):
        reference = json.loads((ROOT / "tests/fixtures/tables" / filename).read_text())
        for case in reference["cases"]:
            with self.subTest(name=case["name"]):
                pages = self.render_reference(reference, case)
                self.assertEqual(len(pages), len(case["pages"]))
                for index, (page, expected) in enumerate(zip(pages, case["pages"])):
                    self.assertCountEqual([word.text for word in page.findall(".//{*}word")],
                                          [line["text"] for line in expected],
                                          f"{case['name']} page {index} lost, repeated or misplaced content")

    def test_rowspan_cells_keep_independent_native_line_breaks(self):
        reference = json.loads((ROOT / "tests/fixtures/tables/weasy70-rows.json").read_text())
        case = reference["parallel_flows"][0]
        pages = self.render_reference(reference, case)
        self.assertEqual(len(pages), 2)
        labels = [[word.text for word in page.findall(".//{*}word")] for page in pages]
        self.assertCountEqual(labels[0], ["Lead", "A0", "A1", "A2", "A3", "A4",
                                               "B0", "B1", "B2", "B3"])
        self.assertCountEqual(labels[1], ["A5", "A6", "A7", "C0", "C1", "After"])

    def test_internal_rowspan_boundaries_change_pages_without_replaying_content(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        for constraint, width, second in [("break-before:page", 240, 1),
                                          ("page:wide", 400, 1),
                                          ("break-before:right", 240, 2),
                                          ("page:wide;break-before:right", 400, 2)]:
            for count in [1, 60]:
                with self.subTest(constraint=constraint, count=count):
                    words = [f"S{index:03}" for index in range(count)]
                    source = self.output / "internal-rowspan.html"
                    source.write_text(
                        "<!doctype html><style>"
                        f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                        "@page{size:240pt 120pt;margin:10pt}@page wide{size:400pt 120pt}"
                        "html,body,p,td{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                        "table{width:100%;table-layout:fixed;border-spacing:0}td{vertical-align:top}"
                        ".span{background:#ddd;border:1pt solid red}"
                        f".second{{{constraint}}}</style><table><tr><td class=span rowspan=2>" +
                        " ".join(words) + "</td><td>First</td></tr><tr class=second><td>Second</td>"
                        "</tr></table><p>After</p>"
                    )
                    result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                            capture_output=True, text=True, timeout=60)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    pages = pdf_text_pages(self.pdf)
                    labels = [[word.text for word in page.findall(".//{*}word")] for page in pages]
                    self.assertEqual([index for index, text in enumerate(labels) if "Second" in text], [second])
                    self.assertEqual(float(pages[second].attrib["width"]), width)
                    self.assertEqual([word for text in labels for word in text if word in words], words)
                    self.assertEqual(sum(text.count("After") for text in labels), 1)
                    if second == 2:
                        self.assertEqual(labels[1], [])
                    for number in range(second + 1):
                        prefix = self.output / f"internal-rowspan-{number}"
                        subprocess.run(["pdftoppm", "-f", str(number + 1), "-l", str(number + 1),
                                        "-singlefile", "-r", "72", "-png", str(self.pdf), str(prefix)],
                                       check=True, capture_output=True, timeout=60)
                        with Image.open(prefix.with_suffix(".png")) as rendered:
                            raster = rendered.convert("RGB")
                            if second == 2 and number == 1:
                                self.assertEqual(raster.getextrema(), ((255, 255),) * 3,
                                                 "spanning decoration must skip the forced blank sheet")
                            elif number == 0:
                                self.assertEqual(raster.getpixel((10, 60)), (255, 0, 0),
                                                 "the spanning border must reach the first page boundary")
                                self.assertEqual(raster.getpixel((100, 100)), (221, 221, 221))

    def test_nested_tables_yield_to_the_enclosing_named_row(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = [f"S{index:03}" for index in range(60)]
        for nested in ["paragraph", "rows", "rowspan"]:
            with self.subTest(nested=nested):
                if nested == "rows":
                    inner = "".join(f"<tr><td>{word}</td></tr>" for word in words)
                elif nested == "rowspan":
                    inner = ("<tr><td rowspan=2>" + " ".join(words) +
                             "</td><td>InnerFirst</td></tr><tr class=inner-next><td>InnerSecond</td></tr>")
                else:
                    inner = "<tr><td>" + " ".join(words) + "</td></tr>"
                source = self.output / "nested-row-yield.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:240pt 120pt;margin:10pt}@page wide{size:400pt 120pt}"
                    "html,body,p,td{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                    "table{width:100%;table-layout:fixed;border-spacing:0}td{vertical-align:top}"
                    ".second{page:wide}.inner-next{break-before:page}.inner td{background:#ddd}"
                    "</style><table><tr><td rowspan=2><table class=inner>" + inner +
                    "</table></td><td>First</td></tr><tr class=second><td>Second</td></tr></table><p>After</p>"
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                pages = pdf_text_pages(self.pdf)
                labels = [[word.text for word in page.findall(".//{*}word")] for page in pages]
                self.assertEqual([index for index, text in enumerate(labels) if "Second" in text], [1])
                self.assertEqual(float(pages[1].attrib["width"]), 400)
                self.assertEqual([word for text in labels for word in text if word in words], words)
                if nested != "rowspan":
                    prefix = self.output / "nested-row-wide"
                    subprocess.run(["pdftoppm", "-f", "2", "-l", "2", "-singlefile", "-r", "72",
                                    "-png", str(self.pdf), str(prefix)], check=True, capture_output=True, timeout=60)
                    with Image.open(prefix.with_suffix(".png")) as rendered:
                        self.assertEqual(rendered.convert("RGB").getpixel((190, 15)), (221, 221, 221),
                                         "the resumed inner cell must use the wider outer column")

    def test_columns_yield_to_the_enclosing_named_row(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = [f"S{index:03}" for index in range(120)]
        for variant in ["auto", "balance", "spanner", "forced"]:
            for constraint, second in [("page:wide", 1), ("page:wide;break-before:right", 2)]:
                with self.subTest(variant=variant, constraint=constraint):
                    if variant == "spanner":
                        content = '<p class=spanner>' + " ".join(words) + "</p>"
                    elif variant == "forced":
                        content = ("<p>" + " ".join(words[:10]) + "</p><p class=forced>" +
                                   " ".join(words[10:]) + "</p>")
                    else:
                        content = " ".join(words)
                    fill = "balance" if variant == "balance" else "auto"
                    source = self.output / "columns-row-yield.html"
                    source.write_text(
                        "<!doctype html><style>"
                        f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                        "@page{size:240pt 120pt;margin:10pt}@page wide{size:400pt 120pt}"
                        "html,body,div,p,td{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                        "table{width:100%;table-layout:fixed;border-spacing:0}td{vertical-align:top}"
                        f".second{{{constraint}}}.columns{{column-count:2;column-gap:10pt;column-fill:{fill};column-rule:2pt solid red}}"
                        ".spanner{column-span:all}.forced{break-before:page}</style>"
                        "<table><tr><td rowspan=2><div class=columns>" + content +
                        "</div></td><td>First</td></tr><tr class=second><td>Second</td></tr></table><p>After</p>"
                    )
                    result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                            capture_output=True, text=True, timeout=60)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    pages = pdf_text_pages(self.pdf)
                    labels = [[word.text for word in page.findall(".//{*}word")] for page in pages]
                    self.assertEqual([index for index, text in enumerate(labels) if "Second" in text], [second])
                    self.assertEqual(float(pages[second].attrib["width"]), 400)
                    self.assertEqual([word for text in labels for word in text if word in words], words)
                    if variant in ["auto", "balance"]:
                        body = [word for word in pages[second].findall(".//{*}word") if word.text in words]
                        self.assertTrue(any(abs(float(word.attrib["xMin"]) - 110) < 0.01 for word in body),
                                        "the wider page must place its second column at x=110pt")
                        first_y = float(body[0].attrib["yMin"])
                        first_column_line = [word for word in body
                                             if abs(float(word.attrib["yMin"]) - first_y) < 0.01
                                             and float(word.attrib["xMin"]) < 100]
                        self.assertEqual(len(first_column_line), 3,
                                         "90pt columns must rebreak source into three-word lines")
                    if variant == "auto":
                        prefix = self.output / "columns-wide-rule"
                        subprocess.run(["pdftoppm", "-f", str(second + 1), "-l", str(second + 1),
                                        "-singlefile", "-r", "72", "-png", str(self.pdf), str(prefix)],
                                       check=True, capture_output=True, timeout=60)
                        with Image.open(prefix.with_suffix(".png")) as rendered:
                            self.assertEqual(rendered.convert("RGB").getpixel((104, 50)), (255, 0, 0),
                                             "the rule must move into the wider destination gap")
                    for label in ["First", "Second", "After"]:
                        self.assertEqual(sum(text.count(label) for text in labels), 1, label)
                    if second == 2:
                        self.assertEqual(labels[1], [])

    def test_short_cells_stretch_inside_destination_columns(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = [f"S{index:03}" for index in range(90)]
        for columns, count in [("column-count:2", 2), ("column-width:50pt", 3)]:
            for direction in ["ltr", "rtl"]:
                for alignment in ["top", "middle", "bottom"]:
                    with self.subTest(columns=columns, direction=direction, alignment=alignment):
                        source = self.output / "column-cell-stretch.html"
                        source.write_text(
                            "<!doctype html><style>"
                            f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                            "@page{size:240pt 120pt;margin:10pt}@page wide{size:400pt 120pt}"
                            "@page wide:right{size:240pt 120pt}"
                            "html,body,div,section,p,td{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                            "table{width:100%;table-layout:fixed;border-spacing:0}td{vertical-align:top}"
                            ".second{page:wide}"
                            f".columns{{{columns};column-gap:10pt;column-fill:auto;direction:{direction};position:relative;left:10%}}"
                            ".inner-wrap{position:relative;left:5pt}"
                            f".short{{background:blue;vertical-align:{alignment}}}</style>"
                            "<table><tr><td rowspan=2><div class=columns><section class=inner-wrap><table><tr><td>" +
                            " ".join(words) + "</td><td class=short>Short</td></tr></table></section></div></td>"
                            "<td>First</td></tr><tr class=second><td>Second</td></tr></table><p>After</p>"
                        )
                        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                                capture_output=True, text=True, timeout=60)
                        self.assertEqual(result.returncode, 0, result.stderr)
                        pages = pdf_text_pages(self.pdf)
                        labels = [[word.text for word in page.findall(".//{*}word")] for page in pages]
                        self.assertEqual([index for index, text in enumerate(labels) if "Second" in text], [1])
                        # Poppler spatial ordering and adjoining cells do not preserve source word boundaries.
                        emitted = "".join(glyph.text for glyph in pdf_content.glyphs(self.pdf))
                        self.assertEqual(re.findall(r"S\d{3}", emitted), words)
                        self.assertEqual(emitted.count("Short"), 1)
                        prefix = self.output / "column-cell-stretch-wide"
                        subprocess.run(["pdftoppm", "-f", "2", "-l", "2", "-singlefile", "-r", "72",
                                        "-png", str(self.pdf), str(prefix)], check=True, capture_output=True, timeout=60)
                        column_width = 200 / count - 10
                        with Image.open(prefix.with_suffix(".png")) as rendered:
                            raster = rendered.convert("RGB")
                            for column in range(count):
                                x = 10 + 24 + column * (column_width + 10)
                                if direction == "ltr":
                                    x += column_width / 2
                                self.assertEqual(raster.getpixel((int(x + column_width / 2 - 2), 50)),
                                                 (0, 0, 255), "the short-cell background must reach its destination edge")
                                self.assertNotEqual(raster.getpixel((int(x - 2), 50)), (0, 0, 255),
                                                    "the short-cell background must start at its destination edge")

    def test_split_column_margins_resolve_against_destination_boxes(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        for margins, gap in [
                ("section{margin-bottom:10%}", 19),
                ("section{margin-bottom:5pt}.tail{margin-bottom:10%}", 19),
                ("section{margin-bottom:10%}.tail{margin-bottom:7pt}", 19),
                ("section{width:50%}.tail{margin-bottom:20%}", 19),
                ("section{margin-bottom:calc(10% - 2pt)}", 17),
                ("section{margin-bottom:-10%}.tail{margin-bottom:2pt}", 3),
                ("section{margin-bottom:10%}.tail{margin-bottom:-2pt}", 17)]:
            words = [f"S{index:03}" for index in range(20 if "width:50%" in margins else 40)]
            for direction in ["ltr", "rtl"]:
                with self.subTest(margins=margins, direction=direction):
                    source = self.output / "split-column-margins.html"
                    source.write_text(
                        "<!doctype html><style>"
                        f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                        "@page{size:240pt 120pt;margin:10pt}@page wide{size:400pt 120pt}"
                        "html,body,div,section,p,td{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                        "table{width:100%;table-layout:fixed;border-spacing:0}td{vertical-align:top}"
                        ".second{page:wide}.spanner{column-span:all}"
                        f".columns{{column-count:2;column-gap:10pt;column-fill:auto;direction:{direction}}}"
                        + margins + "</style><table><tr><td rowspan=2><div class=columns><section><p>" +
                        " ".join(words) + "</p><p class=spanner>Heading</p><p class=tail>Tail</p></section>"
                        "<p>Following</p></div></td><td>First</td></tr><tr class=second><td>Second</td></tr></table>"
                    )
                    result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                            capture_output=True, text=True, timeout=60)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    characters = [(character, glyph) for glyph in pdf_content.glyphs(self.pdf)
                                  for character in glyph.text]
                    emitted = "".join(character for character, _ in characters)
                    self.assertEqual(re.findall(r"S\d{3}", emitted), words)
                    for label in ["Tail", "Following", "Heading", "Second"]:
                        self.assertEqual(emitted.count(label), 1, label)
                    tail = characters[emitted.index("Tail")][1]
                    following = characters[emitted.index("Following")][1]
                    self.assertEqual((tail.page, following.page), (2, 2))
                    self.assertAlmostEqual(tail.origin[1] - following.origin[1], gap, places=4)

    def test_active_cloned_edges_use_destination_page_reservations(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        for name, nested, flowed, count, gap in [
                ("plain", False, False, 5, 68),
                ("flowed", False, True, 5, 68),
                ("nested", True, False, 8, 52)]:
            with self.subTest(name=name):
                labels = [f"Line{index:02}" for index in range(1, 20)] if flowed else ["Top"]
                body = '<div class=box>' + '<br>'.join(labels) + '</div><p>After</p>'
                if nested:
                    body = '<div class=outer>' + body + '</div>'
                height = 270 if nested else 250
                source = self.output / "active-clone-reserve.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:240pt 120pt;margin:10pt}@page:left{size:400pt 120pt}"
                    "html,body,div,p{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                    f".box{{height:{height}pt;box-sizing:border-box;padding:10% 0;"
                    "background:blue;box-decoration-break:clone}"
                    ".outer{padding:2% 0;box-decoration-break:clone}</style>" + body
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                pages = pdf_text_pages(self.pdf)
                self.assertEqual(len(pages), count)
                characters = [(character, glyph) for glyph in pdf_content.glyphs(self.pdf)
                              for character in glyph.text]
                emitted = "".join(character for character, _ in characters)
                self.assertEqual(emitted, "".join(labels) + "After")
                first = characters[0][1]
                after = characters[emitted.index("After")][1]
                self.assertEqual((first.page, after.page), (1, count))
                self.assertAlmostEqual(first.origin[1] - after.origin[1], gap, places=4)
                for index, page in enumerate(pages):
                    self.assertEqual(float(page.attrib["width"]), 240 if index % 2 == 0 else 400)
                    top = 10 + ((4.4 if index % 2 == 0 else 7.6) if nested else 0)
                    extent = ((70.8 if index + 1 == count else
                               (91.2 if index % 2 == 0 else 84.8)) if nested else
                              (90 if index + 1 == count else 100))
                    prefix = self.output / f"active-clone-reserve-{index}"
                    subprocess.run(["pdftoppm", "-f", str(index + 1), "-l", str(index + 1),
                                    "-singlefile", "-r", "72", "-png", str(self.pdf), str(prefix)],
                                   check=True, capture_output=True, timeout=60)
                    with Image.open(prefix.with_suffix(".png")) as rendered:
                        raster = rendered.convert("RGB")
                        for y in [round(top) + 2, round(top + extent) - 2]:
                            self.assertEqual(raster.getpixel((150, y)), (0, 0, 255),
                                             f"{name}/page {index + 1}: cloned blue edge missing")
                        for y in [round(top) - 2, round(top + extent) + 2]:
                            self.assertNotEqual(raster.getpixel((150, y)), (0, 0, 255),
                                                f"{name}/page {index + 1}: blue edge exceeds its fragment")

    def test_caption_entries_use_retained_wrapper_geometry_and_margin_sources(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        cases = [(side, direction, half, boundary, False)
                 for side in ["top", "bottom"] for direction in ["ltr", "rtl"] for half in [False, True]
                 for boundary in (["before", "after"] if side == "top" else ["before"])]
        cases.extend((side, "ltr", False, "before", True) for side in ["top", "bottom"])
        for side, direction, half, boundary, inner in cases:
            with self.subTest(side=side, direction=direction, half=half, boundary=boundary, inner=inner):
                width = "width:50%;" if half else ""
                forced = ".changed{break-before:page}" if boundary == "before" else ".lead{break-after:page}"
                content = '<p class=inner>Caption</p>' if inner else 'Caption'
                source = self.output / "caption-entry.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:240pt 120pt;margin:10pt}@page:left{size:400pt 120pt}"
                    "html,body,p,caption,td{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                    f"table{{width:50%;table-layout:fixed;border-spacing:0;direction:{direction}}}"
                    "caption,td{text-align:left}.lead{margin-bottom:30pt}.inner{margin:5pt 0 7pt}"
                    f".changed{{{width}caption-side:{side};margin:10% 0;background:blue}}"
                    ".after{background:lime}" + forced + "</style><table><caption class=lead>Before</caption>"
                    '<caption class=changed>' + content + '</caption><tr><td>Cell</td></tr></table>'
                    '<p class=after>After</p>'
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                sheets = pdf_text_pages(self.pdf)
                self.assertEqual([float(page.attrib["width"]) for page in sheets], [240, 400])
                characters = [(character, glyph) for glyph in pdf_content.glyphs(self.pdf) for character in glyph.text]
                emitted = "".join(character for character, _ in characters)
                self.assertEqual(emitted, "BeforeCaptionCellAfter" if side == "top" else "BeforeCellCaptionAfter")
                first = characters[0][1]
                caption = characters[emitted.index("Caption")][1]
                after = characters[emitted.index("After")][1]
                self.assertEqual((caption.page, after.page), (2, 2))
                self.assertAlmostEqual(first.origin[1] - caption.origin[1], 24 if inner else 19, places=4)
                left = 105 if half and direction == "rtl" else 10
                self.assertAlmostEqual(caption.origin[0], left, places=4)
                after_gap = (58 if side == "top" else 48) + (12 if inner else 0)
                self.assertAlmostEqual(first.origin[1] - after.origin[1], after_gap, places=4)
                prefix = self.output / "caption-entry-wide"
                subprocess.run(["pdftoppm", "-f", "2", "-l", "2", "-singlefile", "-r", "72",
                                "-png", str(self.pdf), str(prefix)], check=True, capture_output=True, timeout=60)
                with Image.open(prefix.with_suffix(".png")) as rendered:
                    raster = rendered.convert("RGB")
                    size = 95 if half else 190
                    height = 22 if inner else 10
                    center = left + size // 2
                    self.assertEqual(raster.getpixel((center, 29 + height // 2)), (0, 0, 255))
                    for point in [(left - 2, 32), (left + size + 2, 32), (center, 27), (center, 29 + height + 2)]:
                        self.assertNotEqual(raster.getpixel(point), (0, 0, 255), "caption must use destination bounds")
                    self.assertEqual(raster.getpixel((300, 10 + after_gap + 2)), (0, 255, 0))
                    self.assertNotEqual(raster.getpixel((300, 10 + after_gap - 2)), (0, 255, 0))

    def test_active_caption_continuations_reflow_before_and_after_the_grid(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        labels = [f"S{index:03}" for index in range(60)]
        for side, first_lines, height in [("top", 10, 50), ("bottom", 9, 60)]:
            with self.subTest(side=side):
                source = self.output / "active-caption.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:240pt 120pt;margin:10pt}@page:left{size:400pt 120pt}"
                    "html,body,p,caption,td{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                    "table{width:50%;table-layout:fixed;border-spacing:0}caption,td{text-align:left}"
                    f"caption{{caption-side:{side};background:blue;break-inside:auto}}td{{background:lime}}</style>"
                    "<table><caption>" + " ".join(labels) + "</caption><tr><td>Cell</td></tr></table><p>After</p>"
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(len(pdf_text_pages(self.pdf)), 2)
                characters = [(character, glyph) for glyph in pdf_content.glyphs(self.pdf) for character in glyph.text]
                emitted = "".join(character for character, _ in characters)
                matches = list(re.finditer(r"S\d{3}", emitted))
                self.assertEqual([match.group() for match in matches], labels)
                for label in ["Cell", "After"]:
                    self.assertEqual(emitted.count(label), 1)
                line_counts = [{}, {}]
                for match in matches:
                    glyph = characters[match.start()][1]
                    y = round(glyph.origin[1], 4)
                    counts = line_counts[glyph.page - 1]
                    counts[y] = counts.get(y, 0) + 1
                self.assertEqual(list(line_counts[0].values()), [3] * first_lines)
                self.assertEqual(list(line_counts[1].values()), [6] * 5 if side == "top" else [6, 6, 6, 6, 6, 3])
                cell = characters[emitted.index("Cell")][1]
                after = characters[emitted.index("After")][1]
                self.assertEqual(cell.page, 2 if side == "top" else 1)
                self.assertEqual(after.page, 2)
                self.assertAlmostEqual(characters[0][1].origin[1] - after.origin[1], 60, places=4)
                for page in sorted({2, cell.page}):
                    prefix = self.output / f"active-caption-{page}"
                    subprocess.run(["pdftoppm", "-f", str(page), "-l", str(page), "-singlefile", "-r", "72",
                                    "-png", str(self.pdf), str(prefix)], check=True, capture_output=True, timeout=60)
                    with Image.open(prefix.with_suffix(".png")) as rendered:
                        raster = rendered.convert("RGB")
                        if page == 2:
                            self.assertEqual(raster.getpixel((198, 10 + height - 2)), (0, 0, 255))
                            self.assertNotEqual(raster.getpixel((202, 20)), (0, 0, 255))
                            self.assertNotEqual(raster.getpixel((150, 10 + height + 2)), (0, 0, 255))
                        if page == cell.page:
                            right, y = (200, 65) if side == "top" else (120, 15)
                            self.assertEqual(raster.getpixel((right - 2, y)), (0, 255, 0))
                            self.assertNotEqual(raster.getpixel((right + 2, y)), (0, 255, 0))

    def test_caption_margins_contribute_once_to_wrapper_geometry(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        cases = []
        for top_after, bottom_before, bottom_after in [(7, 0, 0), (-3, 0, 0), (7, 5, 11), (-3, 5, -2)]:
            styles = (f".top{{margin-bottom:{top_after}pt}}"
                      f".bottom{{caption-side:bottom;margin:{bottom_before}pt 0 {bottom_after}pt}}")
            body = '<caption class=top>Top</caption><caption class=bottom>Bottom</caption>'
            cases.append((styles, body, ["Top", "Cell", "Bottom", "After"],
                          [0, 10 + top_after, 20 + top_after + bottom_before,
                           30 + top_after + bottom_before + bottom_after]))
        for margins, gaps in [([7, 3, 5, 4, 8, 2, 6], [0, 17, 32, 46, 64, 80]),
                              ([-3, 5, -2, -1, -4, 2, -1], [0, 12, 20, 29, 37, 46])]:
            first_after, second_before, second_after, bottom_before, bottom_after, last_before, last_after = margins
            styles = (f".first{{margin-bottom:{first_after}pt}}"
                      f".second{{margin:{second_before}pt 0 {second_after}pt}}"
                      f".bottom{{caption-side:bottom;margin:{bottom_before}pt 0 {bottom_after}pt}}"
                      f".last{{caption-side:bottom;margin:{last_before}pt 0 {last_after}pt}}")
            body = ('<caption class=first>TopA</caption><caption class=second>TopB</caption>'
                    '<caption class=bottom>BottomA</caption><caption class=last>BottomB</caption>')
            cases.append((styles, body, ["TopA", "TopB", "Cell", "BottomA", "BottomB", "After"], gaps))
        for styles, body, labels, gaps in cases:
            with self.subTest(styles=styles):
                source = self.output / "caption-margins.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:240pt 180pt;margin:10pt}"
                    "html,body,caption,td,p{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                    "table{width:100pt;border-spacing:0}caption{text-align:left;background:blue}"
                    "p{background:lime}" + styles + "</style><table>" + body +
                    "<tr><td>Cell</td></tr></table><p>After</p>"
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(len(pdf_text_pages(self.pdf)), 1)
                characters = [(character, glyph) for glyph in pdf_content.glyphs(self.pdf)
                              for character in glyph.text]
                emitted = "".join(character for character, _ in characters)
                self.assertEqual(emitted, "".join(labels))
                first = characters[0][1]
                for label, gap in zip(labels, gaps):
                    glyph = characters[emitted.index(label)][1]
                    self.assertAlmostEqual(first.origin[1] - glyph.origin[1], gap, places=4, msg=label)
                prefix = self.output / "caption-margins-after"
                subprocess.run(["pdftoppm", "-singlefile", "-r", "72", "-png", str(self.pdf), str(prefix)],
                               check=True, capture_output=True, timeout=60)
                with Image.open(prefix.with_suffix(".png")) as rendered:
                    raster = rendered.convert("RGB")
                    for offset in [2, 8]:
                        self.assertEqual(raster.getpixel((150, 10 + gaps[-1] + offset)), (0, 255, 0))
                    for offset in [-2, 12]:
                        self.assertNotEqual(raster.getpixel((150, 10 + gaps[-1] + offset)), (0, 255, 0))

    def test_active_percentage_padding_follows_destination_containing_width(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        cases = [("block", half, decoration, count, pages, gap, extent)
                 for half, decoration, count, pages, gap, extent in [
                (False, "slice", 9, 2, 37, 48),
                (False, "slice", 10, 2, 47, 58),
                (False, "clone", 9, 2, 76, 87),
                (False, "clone", 10, 3, -11, 97),
                (True, "slice", 9, 2, 23.5, 29),
                (True, "slice", 10, 2, 23.5, 29),
                (True, "clone", 9, 2, 33, 38.5),
                (True, "clone", 10, 2, 43, 48.5)]]
        cases.extend(("table", False, decoration, count, pages, gap, extent)
                     for decoration, count, pages, gap, extent in [("slice", 9, 2, 37, 48),
                                                                  ("slice", 10, 2, 47, 58),
                                                                  ("clone", 9, 2, 76, 87),
                                                                  ("clone", 10, 3, -11, 97)])
        for kind, half, decoration, count, pages, gap, extent in cases:
            with self.subTest(kind=kind, half=half, decoration=decoration, count=count):
                labels = [f"Line{index:02}" for index in range(1, count + 1)]
                outer = ".outer{width:50%}" if half else ""
                if kind == "table":
                    body = '<table class=box>' + ''.join(f'<tr><td>{label}</td></tr>' for label in labels) + '</table>'
                else:
                    body = '<div class=box>' + '<br>'.join(labels) + '</div>'
                source = self.output / "active-padding.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:240pt 120pt;margin:10pt}@page:left{size:400pt 120pt}"
                    "html,body,div,p,td{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                    "table{width:100%;table-layout:fixed;border-spacing:0}td{vertical-align:top}"
                    f"{outer}.box{{padding:5% 0 10%;background:blue;box-decoration-break:{decoration}}}</style>"
                    "<div class=outer>" + body + "<p>After</p></div>"
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                sheets = pdf_text_pages(self.pdf)
                self.assertEqual(len(sheets), pages)
                self.assertEqual([float(page.attrib["width"]) for page in sheets],
                                 [240 if index % 2 == 0 else 400 for index in range(pages)])
                characters = [(character, glyph) for glyph in pdf_content.glyphs(self.pdf)
                              for character in glyph.text]
                emitted = "".join(character for character, _ in characters)
                self.assertEqual(emitted, "".join(labels) + "After")
                first = characters[0][1]
                after = characters[emitted.index("After")][1]
                self.assertEqual((first.page, after.page), (1, pages))
                self.assertAlmostEqual(first.origin[1] - after.origin[1], gap, places=4)
                prefix = self.output / "active-padding-final"
                subprocess.run(["pdftoppm", "-f", "2", "-l", "2", "-singlefile", "-r", "72",
                                "-png", str(self.pdf), str(prefix)],
                               check=True, capture_output=True, timeout=60)
                with Image.open(prefix.with_suffix(".png")) as rendered:
                    raster = rendered.convert("RGB")
                    self.assertEqual(raster.getpixel((150, round(10 + extent) - 2)), (0, 0, 255))
                    self.assertNotEqual(raster.getpixel((150, round(10 + extent) + 2)), (0, 0, 255))

    def test_active_height_constraints_recalculate_on_destination_pages(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        cases = [(decoration, constraints, "", "padding:10% 0", 120, count, extent, gap, ["Top"])
                 for decoration, constraints, count, extent, gap in [
                     ("clone", "height:250pt", 7, 9954 / 163, 6368 / 163),
                     ("clone", "min-height:250pt", 7, 9954 / 163, 6368 / 163),
                     ("clone", "height:500pt;max-height:250pt", 7, 9954 / 163, 6368 / 163),
                     ("clone", "height:100pt;min-height:250pt;max-height:200pt", 7, 9954 / 163, 6368 / 163),
                     ("slice", "height:330pt", 4, 16622 / 187, 12508 / 187)]]
        for factor in [1, 2]:
            parent = f"html,body{{height:100%}}.outer{{height:{100 // factor}%}}"
            for constraints in [f"height:{250 * factor}%", f"min-height:{250 * factor}%",
                                f"height:{500 * factor}%;max-height:{250 * factor}%",
                                f"height:{100 * factor}%;min-height:{250 * factor}%;max-height:{200 * factor}%"]:
                cases.append(("slice", constraints, parent, "padding:0", 180, 3, 50, 50, ["Top"]))
        for constraints, left_height, count, extent in [("min-height:150%", 180, 2, 120),
                                                       ("max-height:250%", 80, 3, 20)]:
            cases.append(("slice", constraints, "html,body,.outer{height:100%}", "padding:0",
                          left_height, count, extent, extent,
                          [f"Line{index:02}" for index in range(1, 21)]))
        for decoration, constraints, parent, padding, left_height, count, extent, gap, labels in cases:
            with self.subTest(decoration=decoration, constraints=constraints, parent=parent):
                source = self.output / "active-height.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:240pt 120pt;margin:10pt}"
                    f"@page:left{{size:400pt {left_height}pt}}"
                    "html,body,div,p{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                    f"{parent}.box{{{constraints};{padding};box-sizing:content-box;"
                    f"background:blue;box-decoration-break:{decoration}}}</style>"
                    "<div class=outer><div class=box>" + "<br>".join(labels) + "</div><p>After</p></div>"
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                pages = pdf_text_pages(self.pdf)
                self.assertEqual(len(pages), count)
                characters = [(character, glyph) for glyph in pdf_content.glyphs(self.pdf)
                              for character in glyph.text]
                emitted = "".join(character for character, _ in characters)
                self.assertEqual(emitted, "".join(labels) + "After")
                top = characters[0][1]
                after = characters[emitted.index("After")][1]
                self.assertEqual((top.page, after.page), (1, count))
                top_baseline = float(pages[top.page - 1].attrib["height"]) - top.origin[1]
                after_baseline = float(pages[after.page - 1].attrib["height"]) - after.origin[1]
                self.assertAlmostEqual(after_baseline - top_baseline, gap, places=4)
                for index, page in enumerate(pages):
                    height = 120 if index % 2 == 0 else left_height
                    self.assertEqual((float(page.attrib["width"]), float(page.attrib["height"])),
                                     (240 if index % 2 == 0 else 400, height))
                    bottom = 10 + (extent if index + 1 == count else height - 20)
                    prefix = self.output / f"active-height-{index}"
                    subprocess.run(["pdftoppm", "-f", str(index + 1), "-l", str(index + 1),
                                    "-singlefile", "-r", "72", "-png", str(self.pdf), str(prefix)],
                                   check=True, capture_output=True, timeout=60)
                    with Image.open(prefix.with_suffix(".png")) as rendered:
                        raster = rendered.convert("RGB")
                        self.assertEqual(raster.getpixel((150, round(bottom) - 2)), (0, 0, 255))
                        self.assertNotEqual(raster.getpixel((150, round(bottom) + 2)), (0, 0, 255))

    def test_spanner_split_height_constraints_use_destination_padding(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        cloned_border_gap = 90 + 2 * 18 - 100
        for constraints, sliced, cloned in [
                ("height:100pt;box-sizing:border-box", (0, 90), (1, cloned_border_gap)),
                ("min-height:100pt;box-sizing:border-box", (0, 90), (1, cloned_border_gap)),
                ("height:100pt;box-sizing:content-box", (1, 136 * (1 - 10 / 120) - 100), (1, 136 * (1 - 10 / 120) + 2 * 18 - 100)),
                ("max-height:40pt;box-sizing:border-box", (0, 40 * (1 - 10 / 30)), (0, 40 * (1 - 10 / 30))),
                ("min-height:100pt;max-height:80pt;box-sizing:border-box", (0, 90), (1, cloned_border_gap)),
                ("height:50pt;min-height:100pt;box-sizing:border-box", (0, 90), (1, cloned_border_gap)),
                ("max-height:60pt;box-sizing:border-box", (0, 28), (0, 28)),
                ("height:auto", (0, 28), (0, 28))]:
            for decoration, (column, extent) in [("slice", sliced), ("clone", cloned)]:
                with self.subTest(constraints=constraints, decoration=decoration):
                    source = self.output / "spanner-height.html"
                    source.write_text(
                        "<!doctype html><style>"
                        f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                        "@page{size:240pt 120pt;margin:10pt}@page wide{size:400pt 120pt}"
                        "html,body,div,section,p,td{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                        "table{width:100%;table-layout:fixed;border-spacing:0}td{vertical-align:top}"
                        ".second{page:wide}.columns{column-count:2;column-gap:10pt;column-fill:auto}"
                        ".lead{line-height:80pt}.spanner{column-span:all;line-height:20pt}"
                        f"section{{{constraints};padding:20% 0;background:blue;box-decoration-break:{decoration}}}</style>"
                        "<table><tr><td rowspan=2><div class=columns><p class=lead>Lead</p><section>"
                        "<p class=spanner>Heading</p><p>Tail</p></section><p>Following</p></div></td>"
                        "<td>First</td></tr><tr class=second><td>Second</td></tr></table>"
                    )
                    result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                            capture_output=True, text=True, timeout=60)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    characters = [(character, glyph) for glyph in pdf_content.glyphs(self.pdf)
                                  for character in glyph.text]
                    emitted = "".join(character for character, _ in characters)
                    for label in ["Lead", "Heading", "Tail", "Following", "First", "Second"]:
                        self.assertEqual(emitted.count(label), 1, label)
                    tail = characters[emitted.index("Tail")][1]
                    following = characters[emitted.index("Following")][1]
                    self.assertEqual((tail.page, following.page), (2, 2))
                    self.assertEqual(characters[emitted.index("Second")][1].page, 2,
                                     "the enclosing row must choose the wider second page")
                    self.assertAlmostEqual(following.origin[0], 10 + column * 100, places=4)
                    self.assertAlmostEqual(tail.origin[1] - following.origin[1], extent, places=4)
                    first = characters[emitted.index("First")][1]
                    ascent = 120 - first.origin[1] - 10
                    end = 120 - tail.origin[1] - ascent + extent
                    prefix = self.output / "spanner-height-wide"
                    subprocess.run(["pdftoppm", "-f", "2", "-l", "2", "-singlefile", "-r", "72",
                                    "-png", str(self.pdf), str(prefix)], check=True, capture_output=True, timeout=60)
                    with Image.open(prefix.with_suffix(".png")) as rendered:
                        raster = rendered.convert("RGB")
                        self.assertEqual(raster.getpixel((95 + column * 100, int(end - 2))),
                                         (0, 0, 255), "decoration must reach the constrained normal-flow end")
                        self.assertEqual(raster.getpixel((95 + column * 100, int(end + 2))),
                                         (255, 255, 255), "decoration must stop at the constrained normal-flow end")

    def test_definite_columns_leave_height_for_cloned_rows_after_spanners(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        for sizing in ["border-box", "content-box"]:
            for fill in ["auto", "balance"]:
                for spanners in [1, 2]:
                    with self.subTest(sizing=sizing, fill=fill, spanners=spanners):
                        middle = "<p class=spanner>MiddleHeading</p><p>Middle</p>" if spanners == 2 else ""
                        source = self.output / "definite-cloned-spanners.html"
                        source.write_text(
                            "<!doctype html><style>"
                            f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                            "@page{size:300pt 400pt;margin:10pt}"
                            "html,body,div,section,p{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                            f".columns{{column-count:2;column-gap:10pt;column-fill:{fill};height:200pt;background:lime}}"
                            f"section{{height:50%;box-sizing:{sizing};padding:10pt;background:blue;box-decoration-break:clone}}"
                            ".spanner{column-span:all}</style><p>Start</p><div class=columns><section>"
                            "<p>Before</p><p class=spanner>Heading</p>" + middle +
                            "<p>Tail</p></section><p>Following</p></div><p>Outside</p>"
                        )
                        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                                capture_output=True, text=True, timeout=60)
                        self.assertEqual(result.returncode, 0, result.stderr)
                        characters = [(character, glyph) for glyph in pdf_content.glyphs(self.pdf)
                                      for character in glyph.text]
                        emitted = "".join(character for character, _ in characters)
                        pages = pdf_text_pages(self.pdf)
                        self.assertEqual(len(pages), 1)
                        words = [word.text for page in pages for word in page.findall(".//{*}word")]
                        labels = ["Start", "Before", "Heading", "Tail", "Following", "Outside"]
                        if spanners == 2:
                            labels.extend(["MiddleHeading", "Middle"])
                        for label in labels:
                            self.assertEqual(words.count(label), 1, label)
                        start = characters[emitted.index("Start")][1]
                        outside = characters[emitted.index("Outside")][1]
                        self.assertEqual((start.page, outside.page), (1, 1))
                        self.assertAlmostEqual(start.origin[1] - outside.origin[1], 210, places=4)
                        prefix = self.output / "definite-cloned-spanners"
                        subprocess.run(["pdftoppm", "-f", "1", "-l", "1", "-singlefile", "-r", "72",
                                        "-png", str(self.pdf), str(prefix)], check=True, capture_output=True, timeout=60)
                        with Image.open(prefix.with_suffix(".png")) as rendered:
                            raster = rendered.convert("RGB")
                            self.assertEqual(raster.getpixel((285, 219)), (0, 255, 0))
                            self.assertEqual(raster.getpixel((285, 222)), (255, 255, 255))

    def test_definite_height_yields_to_the_enclosing_named_row(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        source = self.output / "extent-row-yield.html"
        source.write_text(
            "<!doctype html><style>"
            f"@font-face{{font-family:Fixed;src:url('{font}')}}"
            "@page{size:240pt 120pt;margin:10pt}@page wide{size:400pt 120pt}"
            "html,body,div,p,td{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
            "table{width:100%;table-layout:fixed;border-spacing:0}td{vertical-align:top}"
            ".second{page:wide}.tall{height:250pt;background:#ddd}"
            "</style><table><tr><td rowspan=2><div class=tall>Start</div><p>End</p></td>"
            "<td>First</td></tr><tr class=second><td>Second</td></tr></table><p>After</p>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        pages = pdf_text_pages(self.pdf)
        labels = [[word.text for word in page.findall(".//{*}word")] for page in pages]
        self.assertEqual([index for index, text in enumerate(labels) if "Second" in text], [1])
        self.assertEqual([index for index, text in enumerate(labels) if "End" in text], [2])
        self.assertEqual(float(pages[1].attrib["width"]), 400)
        for label in ["Start", "End", "First", "Second", "After"]:
            self.assertEqual(sum(text.count(label) for text in labels), 1, label)
        for number, point in [(2, (190, 100)), (3, (190, 59))]:
            prefix = self.output / f"extent-row-{number}"
            subprocess.run(["pdftoppm", "-f", str(number), "-l", str(number), "-singlefile",
                            "-r", "72", "-png", str(self.pdf), str(prefix)],
                           check=True, capture_output=True, timeout=60)
            with Image.open(prefix.with_suffix(".png")) as rendered:
                self.assertEqual(rendered.convert("RGB").getpixel(point), (221, 221, 221),
                                 "the retained extent must fill the selected wider cell")

    def test_resumed_cell_preserves_percentage_clone_reserves(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = [f"L{index:02}" for index in range(14)]
        source = self.output / "cell-clone-reserves.html"
        source.write_text(
            "<!doctype html><style>"
            f"@font-face{{font-family:Fixed;src:url('{font}')}}"
            "@page{size:240pt 100pt;margin:10pt}@page:left{size:400pt 100pt}"
            "html,body,section,p{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
            "section{padding-bottom:10%;box-decoration-break:clone}"
            "table{width:100%;border-spacing:0}td{padding:0;vertical-align:top}"
            "</style><section><table><tr><td>" +
            "".join(f"<p>{word}</p>" for word in words) + "</td></tr></table></section>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        pages = pdf_text_pages(self.pdf)
        self.assertEqual(len(pages), 3)
        for page, width, expected in zip(pages, [240, 400, 240],
                                         [words[:5], words[5:9], words[9:]], strict=True):
            self.assertEqual(float(page.attrib["width"]), width)
            self.assertEqual([word.text for word in page.findall(".//{*}word")], expected)

    def test_row_group_breaks_propagate_to_table_borders_and_following_flow(self):
        source = self.output / "table-group-breaks.html"
        source.write_text(
            "<!doctype html><style>@page{size:240pt 200pt;margin:20pt}"
            "html,body,p,table,td{margin:0;padding:0;font:10pt/10pt serif}"
            "table{width:100%;border-spacing:0;border-top:6pt solid red;break-after:right}"
            "tbody{break-before:right;break-after:left}"
            "</style><p>Before</p><table><tbody><tr><td>First</td></tr></tbody></table><p>After</p>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        pages = pdf_text_pages(self.pdf)
        self.assertEqual(len(pages), 4)
        self.assertEqual([[word.text for word in page.findall(".//{*}word")] for page in pages],
                         [["Before"], [], ["First"], ["After"]])
        for number in range(1, len(pages) + 1):
            prefix = self.output / f"table-group-border-{number}"
            subprocess.run(["pdftoppm", "-f", str(number), "-l", str(number),
                            "-singlefile", "-r", "72", "-png", str(self.pdf), str(prefix)],
                           check=True, capture_output=True, timeout=60)
            with Image.open(prefix.with_suffix(".png")) as rendered:
                raster = rendered.convert("RGB")
                self.assertEqual(raster.getpixel((200, 22)),
                                 (255, 0, 0) if number == 3 else (255, 255, 255),
                                 "the table top border moves with its first row")

    def test_named_table_rows_remeasure_columns_on_each_page(self):
        for target in ["tr", "tbody"]:
            rows = []
            for name, left, right in [("A", "First", "RightA"),
                                      ("B", "Second", "RightB"),
                                      ("A", "Third", "RightC")]:
                row = f"<tr class={name}><td>{left}</td><td>{right}</td></tr>"
                rows.append(row if target == "tr" else f"<tbody class={name}>{row}</tbody>")
            source = self.output / f"named-table-{target}.html"
            source.write_text(
                "<!doctype html><style>"
                "@page{margin:20pt}@page A{size:240pt 200pt}"
                "@page B{size:400pt 200pt}"
                "html,body,table,td{margin:0;padding:0;font:10pt/10pt serif}"
                "table{width:100%;table-layout:fixed;border-spacing:0;break-inside:avoid}"
                f"{target}.A{{page:A}}{target}.B{{page:B}}"
                "</style><table>" + "".join(rows) + "</table>"
            )
            result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                    capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            pages = pdf_text_pages(self.pdf)
            self.assertEqual(len(pages), 3, target)
            for number, (page, width, text) in enumerate(zip(
                    pages, [240, 400, 240],
                    [["First", "RightA"], ["Second", "RightB"], ["Third", "RightC"]],
                    strict=True), 1):
                with self.subTest(target=target, page=number):
                    words = page.findall(".//{*}word")
                    self.assertEqual(float(page.attrib["width"]), width)
                    self.assertEqual([word.text for word in words], text)
                    self.assertAlmostEqual(float(words[0].attrib["xMin"]), 20, places=3)
                    self.assertAlmostEqual(float(words[1].attrib["xMin"]), width / 2, places=3)

    def test_named_side_breaks_use_pending_blank_type_and_restore_parent(self):
        for direction, side in [("ltr", "right"), ("rtl", "left")]:
            source = self.output / f"named-blank-{direction}.html"
            source.write_text(
                "<!doctype html><style>"
                "@page{margin:20pt;font:10pt serif}"
                "@page Parent{size:360pt 200pt;background:green}"
                "@page A{size:240pt 200pt;background:white}"
                "@page B{size:300pt 200pt;background:blue}"
                "@page B:blank{size:320pt 180pt;background:red;"
                "@top-center{content:'Blank'}}"
                "html,body,section,p{margin:0;padding:0;font:10pt/10pt serif}"
                f"body{{page:Parent;direction:{direction}}}"
                f".before{{page:A}}.after{{page:B;break-before:{side}}}"
                "</style><section><p class=before>Before</p>"
                "<p class=after>After</p>Return</section>"
            )
            result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                    capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            pages = pdf_text_pages(self.pdf)
            self.assertEqual(len(pages), 4, direction)
            expected = [
                (240, 200, "Before", (255, 255, 255)),
                (320, 180, "Blank", (255, 0, 0)),
                (300, 200, "After", (0, 0, 255)),
                (360, 200, "Return", (0, 128, 0)),
            ]
            for number, (page, (width, height, text, color)) in enumerate(
                    zip(pages, expected, strict=True), 1):
                with self.subTest(direction=direction, page=number):
                    self.assertEqual(float(page.attrib["width"]), width)
                    self.assertEqual(float(page.attrib["height"]), height)
                    self.assertEqual([word.text for word in page.findall(".//{*}word")], [text])
                    prefix = self.output / f"named-blank-{direction}-{number}"
                    subprocess.run(["pdftoppm", "-f", str(number), "-l", str(number),
                                    "-singlefile", "-r", "72", "-png", str(self.pdf), str(prefix)],
                                   check=True, capture_output=True, timeout=60)
                    with Image.open(prefix.with_suffix(".png")) as rendered:
                        raster = rendered.convert("RGB")
                        self.assertEqual(raster.size, (width, height))
                        self.assertEqual(raster.getpixel((width // 2, height // 2)), color)

    def test_parallel_content_repaints_an_inserted_blank_pages_canvas(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = [f"B{number:03}" for number in range(60)]
        forced = "<td><p>Before</p><p class=after>After</p></td>"
        flowing = f"<td>{' '.join(words)}</td>"
        for case, cells in enumerate([forced + flowing, flowing + forced]):
            source = self.output / f"parallel-blank-{case}.html"
            source.write_text(
                "<!doctype html><style>"
                f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                "@page{size:300pt 80pt;margin:10pt}@page:blank{size:100pt 50pt}"
                "html,body,p{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                "body{background:blue}table{width:100%;table-layout:fixed;border-spacing:0}"
                "td{padding:0;vertical-align:top}.after{break-before:right}"
                f"</style><table><tr>{cells}</tr></table>"
            )
            result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                    capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            pages = pdf_text_pages(self.pdf)
            self.assertEqual(len(pages), 3)
            text = [[word.text for word in page.findall(".//{*}word")] for page in pages]
            self.assertEqual([word for page in text for word in page if re.fullmatch(r"B\d{3}", word)], words)
            self.assertTrue(any(re.fullmatch(r"B\d{3}", word) for word in text[1]))
            self.assertEqual(float(pages[1].attrib["width"]), 300)
            prefix = self.output / f"parallel-blank-{case}"
            subprocess.run(["pdftoppm", "-f", "2", "-l", "2", "-singlefile", "-r", "72",
                            "-png", str(self.pdf), str(prefix)], check=True, capture_output=True,
                           timeout=60)
            with Image.open(prefix.with_suffix(".png")) as rendered:
                page = rendered.convert("RGB")
                self.assertEqual(page.size, (300, 80))
                self.assertEqual(page.getpixel((285, 69)), (0, 0, 255),
                                 "canvas must cover the destination area, beyond the old blank page")
                self.assertEqual(page.getpixel((295, 69)), (255, 255, 255))

    def test_first_printed_side_selects_geometry_and_freezes_viewport_units(self):
        cases = [
            ("section:first-child{break-before:left}", 300, 500),
            ("section:first-child{break-before:right}", 500, 300),
            ("section:first-child{break-before:page}", 300, 500),
            ("html{direction:rtl}", 300, 500),
            ("html{direction:ltr}body{direction:rtl}", 300, 500),
        ]
        for case, (css, first_width, second_width) in enumerate(cases):
            source = self.output / f"leading-side-{case}.html"
            source.write_text(
                "<!doctype html><style>"
                "@page{size:240pt 200pt;margin:10pt}"
                "@page A:left{size:300pt 200pt}@page A:right{size:500pt 200pt}"
                "html,body,section,p{margin:0;padding:0;font:10pt/10pt serif}"
                "body{page:A}section+section{break-before:page}"
                "div{width:10vw;height:6pt;background:red}"
                f"{css}</style><section><p>First</p><div></div></section>"
                "<section><p>Second</p><div></div></section>"
            )
            result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                    capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            pages = pdf_text_pages(self.pdf)
            with self.subTest(css=css):
                self.assertEqual(len(pages), 2, "leading empty sheets must be suppressed")
                for number, (page, width, text) in enumerate(
                        zip(pages, [first_width, second_width], ["First", "Second"], strict=True), 1):
                    self.assertEqual(float(page.attrib["width"]), width)
                    self.assertEqual([word.text for word in page.findall(".//{*}word")], [text])
                    prefix = self.output / f"leading-side-{case}-{number}"
                    subprocess.run(["pdftoppm", "-f", str(number), "-l", str(number),
                                    "-singlefile", "-r", "72", "-png", str(self.pdf), str(prefix)],
                                   check=True, capture_output=True, timeout=60)
                    with Image.open(prefix.with_suffix(".png")) as rendered:
                        raster = rendered.convert("RGB")
                        red_width = sum(raster.getpixel((x, 22)) == (255, 0, 0)
                                        for x in range(raster.width))
                        self.assertEqual(red_width, (first_width - 20) / 10,
                                         "10vw retains the first selected content width on later pages")

    def test_forced_right_break_inserts_a_styled_blank_pdf_page(self):
        source = self.output / "forced-side.html"
        source.write_text(
            "<!doctype html><style>"
            "@page{size:240pt 200pt;margin:20pt}"
            "@page:blank{size:320pt 180pt;background:red;"
            "@top-center{content:'Blank';font:10pt serif}}"
            "html,body,p{margin:0;padding:0;font:10pt/10pt serif}"
            "p+p{break-before:right}</style><p>Before</p><p>After</p>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        pages = pdf_text_pages(self.pdf)
        self.assertEqual(len(pages), 3)
        for page, words, size in zip(pages, [["Before"], ["Blank"], ["After"]],
                                     [(240, 200), (320, 180), (240, 200)], strict=True):
            self.assertEqual([word.text for word in page.findall(".//{*}word")], words)
            self.assertEqual((float(page.attrib["width"]), float(page.attrib["height"])), size)
        prefix = self.output / "blank-page"
        subprocess.run(["pdftoppm", "-f", "2", "-l", "2", "-singlefile", "-r", "72",
                        "-png", str(self.pdf), str(prefix)], check=True, capture_output=True,
                       timeout=60)
        with Image.open(prefix.with_suffix(".png")) as rendered:
            page = rendered.convert("RGB")
            self.assertEqual(page.size, (320, 180))
            self.assertEqual(page.getpixel((160, 90)), (255, 0, 0))

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-named-pages-")
        self.addCleanup(directory.cleanup)
        self.output = Path(directory.name)
        self.pdf = self.output / "output.pdf"
        self.headings = self.output / "headings.json"
        source = self.output / "input.html"
        source.write_text(
            "<!doctype html><style>"
            "@page A{size:300pt 400pt;margin:20pt;bleed:2pt;background:red}"
            "@page B{size:500pt 300pt;margin:10pt;bleed:4pt;background:blue}"
            "html,body,section,h1{margin:0;padding:0;font:10pt/12pt serif}"
            ".a{page:A}.b{page:B}section+section{break-before:page}"
            f".fixed{{position:fixed;right:0;bottom:0;width:{FIXED_SIZE_PT}pt;"
            f"height:{FIXED_SIZE_PT}pt;background:black}}"
            "</style><div class=fixed></div>"
            "<section class=a><h1 id=first>First</h1></section>"
            "<section class=b><h1 id=second>Second</h1></section>"
            "<section class=a><h1 id=third>Third</h1></section>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf),
                                 "--headings", str(self.headings)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_each_physical_page_has_its_selected_production_boxes(self):
        document = pdf_content.Document(self.pdf)
        pages = document.pages()
        self.assertEqual(len(pages), len(PAGE_EXPECTATIONS))
        for number, (page, (width, height, _, bleed, _)) in enumerate(zip(pages, PAGE_EXPECTATIONS, strict=True), 1):
            for name, expected in [("MediaBox", [-bleed, -bleed, width + bleed, height + bleed]),
                                   ("TrimBox", [0, 0, width, height]),
                                   ("BleedBox", [-bleed, -bleed, width + bleed, height + bleed])]:
                with self.subTest(page=number, box=name):
                    self.assertEqual(document.resolve(page[name]), expected)

    def test_heading_map_and_pdf_destinations_use_each_pages_height_and_margin(self):
        headings = json.loads(self.headings.read_text())
        outline = pdf_content.navigation(self.pdf)["outline"]
        self.assertEqual([item["page"] for item in headings], [1, 2, 3])
        self.assertEqual([item["title"] for item in outline], ["First", "Second", "Third"])
        for heading, bookmark, (_, height, margin, _, _) in zip(headings, outline, PAGE_EXPECTATIONS, strict=True):
            with self.subTest(heading=heading["id"]):
                self.assertAlmostEqual(heading["top"], margin, places=3)
                self.assertEqual(bookmark["destination"]["page"], heading["page"])
                self.assertAlmostEqual(bookmark["destination"]["top"], height - margin, places=3)

    def test_page_background_and_fixed_box_follow_each_page_geometry(self):
        raster = self.output / "page"
        subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                       check=True, capture_output=True, timeout=60)
        paths = sorted(self.output.glob("page-*.png"))
        self.assertEqual(len(paths), len(PAGE_EXPECTATIONS))
        for number, (path, (width, height, margin, bleed, color)) in enumerate(zip(paths, PAGE_EXPECTATIONS, strict=True), 1):
            with Image.open(path) as source:
                image = source.convert("RGB")
                with self.subTest(page=number):
                    self.assertEqual(image.size, (width + 2 * bleed, height + 2 * bleed))
                    self.assertEqual(image.getpixel((0, 0)), color)
                    center = (width - margin - FIXED_SIZE_PT // 2 + bleed,
                              height - margin - FIXED_SIZE_PT // 2 + bleed)
                    self.assertEqual(image.getpixel(center), (0, 0, 0))

    def test_collapsed_table_borders_follow_each_physical_page_width(self):
        source = self.output / "table.html"
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        rows = "".join(f"<tr><td>Row{number:02}</td></tr>" for number in range(12))
        source.write_text(
            "<!doctype html><style>"
            f"@font-face{{font-family:Fixed;src:url('{font}')}}"
            "@page{size:300pt 80pt;margin:10pt;background:white}"
            "@page:left{size:500pt 80pt}html,body{margin:0;padding:0}"
            "table{width:100%;box-sizing:border-box;table-layout:fixed;border-collapse:collapse}"
            "td{padding:0;border:2pt solid red;font:10pt/10pt Fixed}"
            f"</style><table>{rows}</table>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        raster = self.output / "table"
        subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                       check=True, capture_output=True, timeout=60)
        paths = sorted(self.output.glob("table-*.png"))
        self.assertEqual(len(paths), 3)
        for number, (path, width) in enumerate(zip(paths, [300, 500, 300], strict=True), 1):
            with Image.open(path) as rendered:
                page = rendered.convert("RGB")
                with self.subTest(page=number):
                    self.assertEqual(page.size, (width, 80))
                    self.assertEqual(page.getpixel((10, 15)), (255, 0, 0))
                    self.assertEqual(page.getpixel((width - 11, 15)), (255, 0, 0))
                    self.assertEqual(page.getpixel((width - 8, 15)), (255, 255, 255))
                    if width == 500:
                        self.assertEqual(page.getpixel((289, 15)), (255, 255, 255))

    def test_fragmented_cell_background_uses_each_pages_width_and_consumed_height(self):
        source = self.output / "cell.html"
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = " ".join(f"A{number:03}" for number in range(240))
        source.write_text(
            "<!doctype html><style>"
            f"@font-face{{font-family:Fixed;src:url('{font}')}}"
            "@page{size:300pt 80pt;margin:10pt;background:white}@page:left{size:500pt 80pt}"
            "html,body{margin:0;padding:0}table{width:100%;table-layout:fixed;border-spacing:0}"
            "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;background:red}"
            f"</style><table><tr><td>{words}</td></tr></table>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        raster = self.output / "cell"
        subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                       check=True, capture_output=True, timeout=60)
        paths = sorted(self.output.glob("cell-*.png"))
        self.assertEqual(len(paths), 4)
        for number, (path, width) in enumerate(zip(paths, [300, 500, 300, 500], strict=True), 1):
            with Image.open(path) as rendered:
                page = rendered.convert("RGB")
                with self.subTest(page=number):
                    self.assertEqual(page.size, (width, 80))
                    self.assertEqual(page.getpixel((width - 12, 15)), (255, 0, 0))
                    self.assertEqual(page.getpixel((width - 8, 15)), (255, 255, 255))
                    self.assertEqual(page.getpixel((width - 12, 65)),
                                     (255, 255, 255) if number == 4 else (255, 0, 0))

    def test_table_layer_gradient_is_sliced_across_actual_cell_fragments(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = " ".join(f"A{number:03}" for number in range(240))
        for layer in ["colgroup", "col", "tbody", "tr"]:
            with self.subTest(layer=layer):
                source = self.output / f"{layer}.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:300pt 80pt;margin:10pt;background:white}@page:left{size:500pt 80pt}"
                    "html,body{margin:0;padding:0}table{width:100%;table-layout:fixed;border-spacing:0}"
                    "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                    f"{layer}{{background-image:linear-gradient(red 50%,blue 50%)}}"
                    "</style><table><colgroup><col></colgroup><tbody><tr>"
                    f"<td>{words}</td></tr></tbody></table>"
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                raster = self.output / layer
                subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                               check=True, capture_output=True, timeout=60)
                paths = sorted(self.output.glob(f"{layer}-*.png"))
                self.assertEqual(len(paths), 4)
                for number, (path, width) in enumerate(zip(paths, [300, 500, 300, 500], strict=True), 1):
                    with Image.open(path) as rendered:
                        page = rendered.convert("RGB")
                        with self.subTest(page=number):
                            self.assertEqual(page.getpixel((width - 12, 15)),
                                             (255, 0, 0) if number <= 2 else (0, 0, 255))
                            self.assertEqual(page.getpixel((width - 8, 15)), (255, 255, 255))
                            self.assertEqual(page.getpixel((width - 12, 65)),
                                             [(255, 0, 0), (0, 0, 255), (0, 0, 255), (255, 255, 255)][number - 1])

    def test_following_row_shares_reconciled_group_gradient(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = " ".join(f"A{number:03}" for number in range(240))
        for layer in ["colgroup", "col", "tbody"]:
            with self.subTest(layer=layer):
                source = self.output / f"group-{layer}.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:300pt 80pt;margin:10pt;background:white}@page:left{size:500pt 80pt}"
                    "html,body{margin:0;padding:0}table{width:100%;table-layout:fixed;border-spacing:0}"
                    "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
                    f"{layer}{{background-image:linear-gradient(red 97%,blue 97%)}}"
                    "</style><table><colgroup><col></colgroup><tbody>"
                    f"<tr><td>{words}</td></tr><tr><td>End</td></tr></tbody></table>"
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(len(pdf_content.Document(self.pdf).pages()), 4)
                raster = self.output / f"group-{layer}"
                subprocess.run(["pdftoppm", "-f", "4", "-l", "4", "-r", "72", "-png",
                                str(self.pdf), str(raster)], check=True, capture_output=True, timeout=60)
                with Image.open(self.output / f"group-{layer}-4.png") as rendered:
                    page = rendered.convert("RGB")
                    self.assertEqual(page.getpixel((488, 42)), (255, 0, 0))
                    self.assertEqual(page.getpixel((488, 48)), (0, 0, 255))
                    self.assertEqual(page.getpixel((488, 52)), (255, 255, 255))

    def test_collapsed_cell_sides_follow_all_physical_fragments(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = " ".join(f"A{number:03}" for number in range(240))
        source = self.output / "collapsed-cell.html"
        source.write_text(
            "<!doctype html><style>"
            f"@font-face{{font-family:Fixed;src:url('{font}')}}"
            "@page{size:300pt 80pt;margin:10pt;background:white}@page:left{size:500pt 80pt}"
            "html,body{margin:0;padding:0}table{width:100%;table-layout:fixed;border-collapse:collapse}"
            "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;"
            "border-left:2pt solid red;border-right:2pt solid red}"
            f"</style><table><tr><td>{words}</td></tr></table>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        raster = self.output / "collapsed-cell"
        subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                       check=True, capture_output=True, timeout=60)
        paths = sorted(self.output.glob("collapsed-cell-*.png"))
        self.assertEqual(len(paths), 4)
        for number, (path, width) in enumerate(zip(paths, [300, 500, 300, 500], strict=True), 1):
            with Image.open(path) as rendered:
                page = rendered.convert("RGB")
                with self.subTest(page=number):
                    for x in [10, width - 11]:
                        self.assertEqual(page.getpixel((x, 15)), (255, 0, 0))
                        self.assertEqual(page.getpixel((x, 65)),
                                         (255, 255, 255) if number == 4 else (255, 0, 0))
                    self.assertEqual(page.getpixel((width - 8, 15)), (255, 255, 255))
                    self.assertEqual(page.getpixel((10, 72)), (255, 255, 255))

    def test_active_row_repeats_groups_once_and_keeps_cell_paint_between_them(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        left = [f"A{number:03}" for number in range(80)]
        right = [f"B{number:03}" for number in range(40)]
        source = self.output / "active-groups.html"
        source.write_text(
            "<!doctype html><style>"
            f"@font-face{{font-family:Fixed;src:url('{font}')}}"
            "@page{size:300pt 80pt;margin:10pt;background:white}@page:left{size:500pt 80pt}"
            "html,body{margin:0;padding:0}table{width:100%;table-layout:fixed;border-spacing:0}"
            "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}"
            "thead{background:red}tfoot{background:lime}tbody td+td{background:blue}"
            "</style><table><thead><tr><td>HeadLeft</td><td>HeadRight</td></tr></thead>"
            f"<tbody><tr><td>{' '.join(left)}</td><td>{' '.join(right)}</td></tr></tbody>"
            "<tfoot><tr><td>FootLeft</td><td>FootRight</td></tr></tfoot></table>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        raster = self.output / "active-groups"
        subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                       check=True, capture_output=True, timeout=60)
        paths = sorted(self.output.glob("active-groups-*.png"))
        self.assertEqual(len(paths), 4)
        extracted = []
        for number, (path, width) in enumerate(zip(paths, [300, 500, 300, 500], strict=True), 1):
            with Image.open(path) as rendered:
                page = rendered.convert("RGB")
                with self.subTest(page=number):
                    self.assertEqual(page.getpixel((width - 12, 15)), (255, 0, 0))
                    self.assertEqual(page.getpixel((width - 12, 25)), (0, 0, 255))
                    self.assertEqual(page.getpixel((width - 12, 45 if number == 4 else 65)), (0, 255, 0))
                    self.assertEqual(page.getpixel((width - 12, 55 if number == 4 else 75)), (255, 255, 255))
                    text = subprocess.run(["pdftotext", "-f", str(number), "-l", str(number),
                                           "-layout", str(self.pdf), "-"], check=True, capture_output=True,
                                          text=True, timeout=60).stdout
                    for label in ["HeadLeft", "HeadRight", "FootLeft", "FootRight"]:
                        self.assertEqual(text.count(label), 1, text)
                    extracted.extend(re.findall(r"[AB]\d{3}", text))
        self.assertEqual([token for token in extracted if token.startswith("A")], left)
        self.assertEqual([token for token in extracted if token.startswith("B")], right)

    def test_nested_tables_keep_repeated_groups_separate_and_full_width(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        active_words = [f"A{number:03}" for number in range(90)]
        rows_words = [f"R{number:02}" for number in range(12)]
        cases = [
            ("active", f"<tr><td>{' '.join(active_words)}</td></tr>", active_words, [300, 500]),
            ("rows", "".join(f"<tr><td>{word}</td></tr>" for word in rows_words), rows_words, [300, 500, 300]),
        ]
        for name, body, expected, widths in cases:
            with self.subTest(case=name):
                source = self.output / f"nested-{name}.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:300pt 100pt;margin:10pt;background:white}@page:left{size:500pt 100pt}"
                    "html,body{margin:0;padding:0}table{width:100%;table-layout:fixed;border-spacing:0}"
                    "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}"
                    ".outer>thead{background:red}.outer>tfoot{background:lime}"
                    ".inner>thead{background:yellow}.inner>tfoot{background:blue}"
                    "</style><table class=outer><thead><tr><td>OuterHead</td></tr></thead>"
                    "<tbody><tr><td><table class=inner><thead><tr><td>InnerHead</td></tr></thead>"
                    f"<tbody>{body}</tbody><tfoot><tr><td>InnerFoot</td></tr></tfoot></table>"
                    "</td></tr></tbody><tfoot><tr><td>OuterFoot</td></tr></tfoot></table>"
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                raster = self.output / f"nested-{name}"
                subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                               check=True, capture_output=True, timeout=60)
                paths = sorted(self.output.glob(f"nested-{name}-*.png"))
                self.assertEqual(len(paths), len(widths))
                consumed = []
                for number, (path, width) in enumerate(zip(paths, widths, strict=True), 1):
                    with Image.open(path) as rendered:
                        page = rendered.convert("RGB")
                        with self.subTest(page=number):
                            for y, rgb in [(15, (255, 0, 0)), (25, (255, 255, 0)),
                                           (35, (255, 255, 255)), (75, (0, 0, 255)), (85, (0, 255, 0))]:
                                self.assertEqual(page.getpixel((width - 12, y)), rgb)
                            self.assertEqual(page.getpixel((width - 8, 75)), (255, 255, 255))
                            text = subprocess.run(["pdftotext", "-f", str(number), "-l", str(number),
                                                   "-layout", str(self.pdf), "-"], check=True, capture_output=True,
                                                  text=True, timeout=60).stdout
                            for label in ["OuterHead", "InnerHead", "InnerFoot", "OuterFoot"]:
                                self.assertEqual(text.count(label), 1, text)
                            consumed.extend(re.findall(r"A\d{3}|R\d{2}", text))
                self.assertEqual(consumed, expected)

    def test_rowspan_cells_stop_and_resume_at_actual_row_boundaries(self):
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        cases = [
            ("short", 90, 0, "Top", "", [300, 300, 300, 300]),
            ("reflow", 120, 60, None, "@page:left{size:500pt 80pt}", [300, 500, 300, 500]),
        ]
        for name, left_count, right_count, label, page_css, widths in cases:
            with self.subTest(case=name):
                left = [f"A{number:03}" for number in range(left_count)]
                right = [f"B{number:03}" for number in range(right_count)]
                source = self.output / f"span-{name}.html"
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:300pt 80pt;margin:10pt;background:white}"
                    f"{page_css}table{{width:100%;table-layout:fixed;border-spacing:0}}"
                    "html,body{margin:0;padding:0}"
                    "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}"
                    ".first{background:blue}"
                    + (".last{background:lime}" if name == "reflow" else "")
                    + f"</style><table><tr><td rowspan=2>{' '.join(left)}</td>"
                    + f"<td class=first>{label or ' '.join(right)}</td></tr>"
                    + "<tr><td class=last>Bottom</td></tr></table>"
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                raster = self.output / f"span-{name}"
                subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                               check=True, capture_output=True, timeout=60)
                paths = sorted(self.output.glob(f"span-{name}-*.png"))
                self.assertEqual(len(paths), len(widths))
                consumed = []
                for number, (path, width) in enumerate(zip(paths, widths, strict=True), 1):
                    with Image.open(path) as rendered:
                        page = rendered.convert("RGB")
                        with self.subTest(page=number):
                            # CSS Tables 3: distribute span height over the row minima.
                            total_height = 230 if name == "short" else 210
                            first_height = total_height / 2 if name == "short" else total_height * 110 / 120
                            page_capacity, margin = 60, 10
                            for y in (15, 25, 63, 67):
                                offset = (number - 1) * page_capacity + y - margin
                                if offset < first_height:
                                    expected = (0, 0, 255)
                                elif name == "reflow" and offset < total_height:
                                    expected = (0, 255, 0)
                                else:
                                    expected = (255, 255, 255)
                                self.assertEqual(page.getpixel((width - 12, y)), expected,
                                                 f"{name}, page {number}, y={y}: row boundary")
                            text = subprocess.run(["pdftotext", "-f", str(number), "-l", str(number),
                                                   "-layout", str(self.pdf), "-"], check=True, capture_output=True,
                                                  text=True, timeout=60).stdout
                            self.assertEqual(text.count("Bottom"), int(number == (3 if name == "short" else 4)))
                            consumed.extend(re.findall(r"[AB]\d{3}", text))
                self.assertEqual([word for word in consumed if word.startswith("A")], left)
                self.assertEqual([word for word in consumed if word.startswith("B")], right)

    def test_short_cell_stretches_through_narrower_page_after_its_text_ends(self):
        source = self.output / "short-cell.html"
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        left = " ".join(f"A{number:03}" for number in range(90))
        right = " ".join(f"B{number:03}" for number in range(40))
        source.write_text(
            "<!doctype html><style>"
            f"@font-face{{font-family:Fixed;src:url('{font}')}}"
            "@page{size:300pt 80pt;margin:10pt;background:white}"
            "@page:left{size:500pt 100pt;margin:20pt 10pt}"
            "html,body{margin:0;padding:0}table{width:100%;table-layout:fixed;border-spacing:0}"
            "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}"
            f"td+td{{background:blue}}</style><table><tr><td>{left}</td><td>{right}</td></tr></table>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        raster = self.output / "short-cell"
        subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                       check=True, capture_output=True, timeout=60)
        paths = sorted(self.output.glob("short-cell-*.png"))
        self.assertEqual(len(paths), 3)
        for number, (path, width, height, top, bottom) in enumerate(zip(
                paths, [300, 500, 300], [80, 100, 80], [10, 20, 10], [70, 80, 60], strict=True), 1):
            with Image.open(path) as rendered:
                page = rendered.convert("RGB")
                with self.subTest(page=number):
                    self.assertEqual(page.size, (width, height))
                    self.assertEqual(page.getpixel((width - 12, top + 5)), (0, 0, 255))
                    self.assertEqual(page.getpixel((width - 12, bottom - 5)), (0, 0, 255))
                    self.assertEqual(page.getpixel((width - 8, top + 5)), (255, 255, 255))
                    self.assertEqual(page.getpixel((width - 12, bottom + 3)), (255, 255, 255))
                    if number == 3:
                        self.assertEqual(page.getpixel((160, 15)), (0, 0, 255))

    def test_aligned_cell_background_precedes_text_without_moving_text(self):
        source = self.output / "aligned-cell.html"
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        lines = "<br>".join(f"Line{number:02}" for number in range(18))
        for alignment, text_page in [("middle", 2), ("bottom", 3)]:
            source.write_text(
                "<!doctype html><style>"
                f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                "@page{size:300pt 80pt;margin:10pt;background:white}"
                "html,body{margin:0;padding:0}table{width:100%;table-layout:fixed;border-spacing:0}"
                "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}"
                f"td+td{{background:blue;vertical-align:{alignment}}}"
                f"</style><table><tr><td>{lines}</td><td>Short</td></tr></table>"
            )
            result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                    capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            raster = self.output / alignment
            subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                           check=True, capture_output=True, timeout=60)
            paths = sorted(self.output.glob(f"{alignment}-*.png"))
            self.assertEqual(len(paths), 3)
            for number, path in enumerate(paths, 1):
                with self.subTest(alignment=alignment, page=number):
                    with Image.open(path) as rendered:
                        page = rendered.convert("RGB")
                        self.assertEqual(page.getpixel((288, 15)), (0, 0, 255))
                        self.assertEqual(page.getpixel((288, 65)), (0, 0, 255))
                        self.assertEqual(page.getpixel((295, 15)), (255, 255, 255))
                    text = subprocess.run(["pdftotext", "-f", str(number), "-l", str(number),
                                           str(self.pdf), "-"], check=True, capture_output=True,
                                          text=True, timeout=60).stdout
                    self.assertEqual(text.count("Short"), int(number == text_page))

    def test_split_rowspan_alignment_is_independent_of_sibling_order(self):
        source = self.output / "aligned-rowspan.html"
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = [f"A{number:03}" for number in range(90)]
        long = f"<td rowspan=2>{' '.join(words)}</td>"
        for alignment, target, offset in [
                ("middle", 2, 85 * (1 - 60 / 145)),
                ("bottom", 4, 170 * (1 - 120 / 290 - 60 / 170))]:
            placements = []
            for short_first in [False, True]:
                short = f'<td rowspan=2 style="vertical-align:{alignment}">Short</td>'
                cells = short + long if short_first else long + short
                source.write_text(
                    "<!doctype html><style>"
                    f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                    "@page{size:300pt 80pt;margin:10pt}@page:left{size:500pt 80pt}"
                    "html,body{margin:0;padding:0}"
                    "table{width:100%;table-layout:fixed;border-spacing:0}"
                    "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}"
                    f"</style><table><tr>{cells}<td>Top</td></tr>"
                    "<tr><td>Bottom</td></tr></table>"
                )
                result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                pages = pdf_text_pages(self.pdf)
                emitted = [(page, word) for page, node in enumerate(pages, 1)
                           for word in node.findall(".//{*}word")]
                with self.subTest(alignment=alignment, short_first=short_first):
                    self.assertEqual([word.text for _, word in emitted
                                      if word.text.startswith("A0")], words)
                    short = [(page, float(word.attrib["yMin"])) for page, word in emitted
                             if word.text == "Short"]
                    self.assertEqual(len(short), 1)
                    self.assertEqual(len(pages), 4)
                    self.assertEqual(short[0][0], target)
                    first = next(word for _, word in emitted if word.text == "A000")
                    self.assertAlmostEqual(short[0][1] - float(first.attrib["yMin"]),
                                           offset, places=4)
                    placements.append((len(pages), short[0]))
            with self.subTest(alignment=alignment):
                self.assertEqual(placements[0], placements[1])

    def test_alignment_padding_progress_adapts_to_page_width_and_height(self):
        source = self.output / "alignment-progress.html"
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = [f"A{number:03}" for number in range(90)]
        for alignment, wide_height, count, target, offset in [
                ("middle", 80, 3, 2, 25), ("bottom", 80, 3, 3, 40),
                ("middle", 60, 4, 2, 25), ("bottom", 60, 4, 4, 10)]:
            source.write_text(
                "<!doctype html><style>"
                f"@font-face{{font-family:Fixed;src:url('{font}')}}"
                "@page{size:300pt 80pt;margin:10pt}"
                f"@page:left{{size:500pt {wide_height}pt}}"
                "html,body{margin:0;padding:0}"
                "table{width:100%;table-layout:fixed;border-spacing:0}"
                "td{padding:0;font:10pt/10pt Fixed;widows:1;orphans:1;vertical-align:top}"
                f"td+td{{vertical-align:{alignment};background:blue}}</style>"
                f"<table><tr><td>{' '.join(words)}</td><td>Short</td></tr></table>"
            )
            result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                    capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            pages = pdf_text_pages(self.pdf)
            emitted = [(page, word) for page, node in enumerate(pages, 1)
                       for word in node.findall(".//{*}word")]
            with self.subTest(alignment=alignment, wide_height=wide_height):
                self.assertEqual(len(pages), count)
                self.assertEqual([word.text for _, word in emitted
                                  if word.text.startswith("A0")], words)
                first = next(word for _, word in emitted if word.text == "A000")
                short = [(page, word) for page, word in emitted if word.text == "Short"]
                self.assertEqual(len(short), 1)
                self.assertEqual(short[0][0], target)
                self.assertAlmostEqual(float(short[0][1].attrib["yMin"])
                                       - float(first.attrib["yMin"]), offset, places=4)
                raster = self.output / f"alignment-{alignment}-{wide_height}"
                subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                               check=True, capture_output=True, timeout=60)
                paths = sorted(self.output.glob(f"{raster.name}-*.png"))
                self.assertEqual(len(paths), count)
                for number, path in enumerate(paths, 1):
                    width = 300 if number % 2 else 500
                    height = 80 if number % 2 else wide_height
                    bottom = (60 if wide_height == 80 else 30) if number == count else height - 10
                    with Image.open(path) as rendered:
                        page = rendered.convert("RGB")
                        self.assertEqual(page.size, (width, height))
                        self.assertEqual(page.getpixel((width - 12, 15)), (0, 0, 255))
                        self.assertEqual(page.getpixel((width - 12, bottom - 3)), (0, 0, 255))
                        self.assertEqual(page.getpixel((width - 12, bottom + 3)), (255, 255, 255))

    def test_forced_cell_break_rewraps_new_block_and_preserves_neighbour(self):
        source = self.output / "forced-cell.html"
        font = (ROOT / "tests/oracle/assets/DejaVuSans.ttf").as_uri()
        words = [f"A{number:03}" for number in range(32)]
        source.write_text(
            "<!doctype html><style>"
            f"@font-face{{font-family:Fixed;src:url('{font}')}}"
            "@page{size:300pt 80pt;margin:10pt;background:white}@page:left{size:500pt 80pt}"
            "html,body,p{margin:0;padding:0;font:10pt/10pt Fixed;widows:1;orphans:1}"
            "table{width:100%;table-layout:fixed;border-spacing:0}"
            "td{padding:0;vertical-align:top;background:red}td+td{background:blue}"
            ".next{break-before:page}</style><table><tr><td><p>Before</p>"
            f"<p class=next>{' '.join(words)}</p></td><td>Neighbour</td></tr></table>"
        )
        result = subprocess.run([str(RENDERER), str(source), str(self.pdf)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        raster = self.output / "forced-cell"
        subprocess.run(["pdftoppm", "-r", "72", "-png", str(self.pdf), str(raster)],
                       check=True, capture_output=True, timeout=60)
        paths = sorted(self.output.glob("forced-cell-*.png"))
        self.assertEqual(len(paths), 2)
        for number, path in enumerate(paths, 1):
            text = subprocess.run(["pdftotext", "-f", str(number), "-l", str(number),
                                   str(self.pdf), "-"], check=True, capture_output=True,
                                  text=True, timeout=60).stdout.split()
            with self.subTest(page=number):
                self.assertEqual(text, ["Before", "Neighbour"] if number == 1 else words)
                with Image.open(path) as rendered:
                    page = rendered.convert("RGB")
                    width = 300 if number == 1 else 500
                    self.assertEqual(page.size, (width, 80))
                    self.assertEqual(page.getpixel((width - 12, 15)), (0, 0, 255))
                    self.assertEqual(page.getpixel((width - 12, 65)),
                                     (0, 0, 255) if number == 1 else (255, 255, 255))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
