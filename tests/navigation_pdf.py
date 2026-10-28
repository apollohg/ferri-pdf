"""Verify compact heading metadata against independently parsed PDF outlines."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
PAGE_HEIGHT_PT = 200


class HeadingNavigation(unittest.TestCase):
    def test_fragmented_rowspan_targets_follow_kept_rows(self):
        _, navigation = self.render(
            "<style>@page{size:240pt 120pt;margin:10pt}body,p{font:10pt/20pt serif}"
            "table{width:100%;border-collapse:collapse}"
            "td,th{padding:0;text-align:left;font-weight:normal;vertical-align:top;orphans:1;widows:1}"
            "tr{break-inside:avoid}</style>"
            "<p><a href='#row'>Row</a> <a href='#cell'>Cell</a></p>"
            "<table><thead><tr><th colspan=2>Header</th></tr></thead><tbody>"
            "<tr><td rowspan=2>Span</td><td>A</td></tr><tr id=row><td id=cell>B<br>C</td></tr>"
            "</tbody><tfoot><tr><td colspan=2>Footer</td></tr></tfoot></table>"
        )
        self.assertEqual(len(navigation["links"]), 2)
        for link in navigation["links"]:
            self.assertEqual(link["page"], 1)
            self.assertEqual(link["destination"]["page"], 2)
            self.assertAlmostEqual(link["destination"]["top"], 90, places=3,
                                   msg="the target starts below the repeated header on page two")

    def test_leader_and_target_counter_keep_the_full_toc_link_clickable(self):
        headings, navigation = self.render(
            "<style>a::after{content:leader('.') target-counter(attr(href),page)}</style>"
            "<p><a href='#second'>Entry</a></p><h2 id=second>Destination</h2>"
        )
        self.assertEqual([(item["id"], item["page"]) for item in headings], [("second", 2)])
        self.assertEqual(len(navigation["links"]), 1)
        link = navigation["links"][0]
        self.assertEqual(link["destination"]["page"], 2)
        self.assertAlmostEqual(link["rect"][0], 20, places=3)
        self.assertAlmostEqual(link["rect"][2], 280, places=3,
                               msg="the leader and resolved page number are part of the same link")

    def test_named_string_bookmarks_follow_the_heading_page(self):
        headings, navigation = self.render(
            "<style>h1,h2{string-set:chapter content();bookmark-label:'Chapter ' string(chapter)}</style>"
            "<h1>Alpha</h1><h2>Beta</h2>"
        )
        self.assertEqual([(item["text"], item["page"]) for item in headings], [("Alpha", 1), ("Beta", 2)])
        first = navigation["outline"][0]
        second = first["children"][0]
        self.assertEqual([(first["title"], first["destination"]["page"]),
                          (second["title"], second["destination"]["page"])],
                         [("Chapter Alpha", 1), ("Chapter Beta", 2)])

    def test_explicit_target_bookmark_label_uses_resolved_text_and_page(self):
        headings, navigation = self.render(
            "<style>h1{bookmark-label:target-text('#second') ' page ' "
            "target-counter('#second',page)}</style>"
            "<h1>Opening</h1><h2 id=second>Destination</h2>"
        )
        self.assertEqual(headings[0]["text"], "Opening")
        self.assertEqual(navigation["outline"][0]["title"], "Destination page 2")
        self.assertEqual(navigation["outline"][0]["destination"]["page"], 1)

    def test_target_generated_heading_labels_match_final_pdf_navigation(self):
        headings, navigation = self.render(
            "<style>h1{bookmark-label:content() ' ' content(after)}h1::after{content:' ' target-text('#second') ' page ' "
            "target-counter('#second',page)}</style>"
            "<h1 id=first><a href='#second'>See</a></h1>"
            "<h2 id=second>Destination</h2>"
        )
        self.assertEqual([(item["text"], item["page"]) for item in headings],
                         [("See", 1), ("Destination", 2)])
        first = navigation["outline"][0]
        second = first["children"][0]
        self.assertEqual(first["title"], "See Destination page 2")
        self.assertEqual(second["title"], headings[1]["text"])
        self.assertEqual(navigation["links"][0]["destination"], second["destination"])
        self.assertEqual(second["destination"]["page"], 2)

    def test_float_continuations_keep_links_and_heading_destinations_on_their_pages(self):
        headings, navigation = self.render(
            "<div style='float:left;width:100pt'>"
            "<h1 id=first><a href='#second'>First</a></h1>"
            "<h2 id=second>Second</h2></div>"
            "<p><a href='#first'>Body first</a></p><p><a href='#second'>Body second</a></p>"
        )
        self.assertEqual([(item["id"], item["page"]) for item in headings],
                         [("first", 1), ("second", 2)])
        first = navigation["outline"][0]
        second = first["children"][0]
        self.assertEqual([first["title"], second["title"]], ["First", "Second"])
        for heading, bookmark in zip(headings, [first, second], strict=True):
            self.assertEqual(heading["page"], bookmark["destination"]["page"])
            self.assertAlmostEqual(heading["top"], PAGE_HEIGHT_PT - bookmark["destination"]["top"], places=3)
        self.assertEqual(len(navigation["links"]), 3, "one annotation per source link, without trial duplicates")
        self.assertCountEqual([(link["page"], link["destination"]["page"]) for link in navigation["links"]],
                              [(1, 2), (1, 1), (1, 2)])
        for link in navigation["links"]:
            expected = first if link["destination"]["page"] == 1 else second
            self.assertEqual(link["destination"], expected["destination"])
        left_edges = sorted(link["rect"][0] for link in navigation["links"])
        for actual, expected in zip(left_edges, [20, 120, 120], strict=True):
            self.assertAlmostEqual(actual, expected, places=3, msg="link hit areas follow float and surrounding text positions")

    def test_floated_table_caption_links_follow_changed_page_geometry(self):
        page_height = 144
        headings, navigation = self.render(
            "<style>@page{size:240pt 144pt;margin:36pt}@page:first{size:360pt 144pt}"
            "table{float:right;width:50%;border-spacing:0}td{padding:0}"
            "caption{caption-side:bottom;text-align:left}</style>"
            "<table><caption><h1 id=caption><a href='#first'>Caption</a></h1></caption>"
            "<tr><td><h1 id=first><a href='#caption'>First</a></h1>FirstTwo</td></tr>"
            "<tr><td>MiddleOne<br>MiddleTwo</td></tr>"
            "<tr><td>LastOne<br>LastTwo</td></tr></table>"
        )
        self.assertCountEqual([(item["id"], item["page"]) for item in headings],
                              [("first", 1), ("caption", 2)])
        bookmarks = {item["title"]: item["destination"] for item in navigation["outline"]}
        self.assertEqual(set(bookmarks), {"First", "Caption"})
        self.assertEqual(len(navigation["links"]), 2, "exactly one annotation for each source link")
        for heading in headings:
            destination = bookmarks[heading["text"]]
            self.assertEqual(destination["page"], heading["page"])
            self.assertAlmostEqual(heading["top"], page_height - destination["top"], places=3)
        for page, left, target in [(1, 180, "Caption"), (2, 120, "First")]:
            link = next(item for item in navigation["links"] if item["page"] == page)
            self.assertAlmostEqual(link["rect"][0], left, places=3,
                                   msg="table and caption hit areas follow their fragment's float position")
            self.assertEqual(link["destination"], bookmarks[target])

    def render(self, body, trace=False):
        directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-heading-map-")
        self.addCleanup(directory.cleanup)
        output = Path(directory.name)
        source, pdf, headings = output / "input.html", output / "output.pdf", output / "headings.json"
        source.write_text(
            "<!doctype html><style>@page{size:300pt 200pt;margin:20pt}"
            "html,body,h1,h2,p{margin:0;padding:0;font:12pt/18pt serif}"
            "h2{break-before:page}</style>" + body
        )
        command = [str(RENDERER), str(source), str(pdf), "--headings", str(headings)]
        if trace:
            command.append("--trace")
        result = subprocess.run(command, capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(pdf.with_suffix(".trace.json").exists(), trace)
        return json.loads(headings.read_text()), pdf_content.navigation(pdf)

    def test_heading_map_and_pdf_outline_share_actual_page_destinations(self):
        for trace in [False, True]:
            with self.subTest(trace=trace):
                headings, navigation = self.render(
                    "<a href='#second' style='display:none'></a>"
                    "<h1 id=first>First heading</h1><h2 id=second>Second café</h2>", trace
                )
                self.assertEqual([(item["id"], item["level"], item["text"], item["page"]) for item in headings],
                                 [("first", 1, "First heading", 1), ("second", 2, "Second café", 2)])
                first = navigation["outline"][0]
                second = first["children"][0]
                self.assertEqual([first["title"], second["title"]], ["First heading", "Second café"])
                for heading, bookmark in zip(headings, [first, second], strict=True):
                    self.assertEqual(heading["page"], bookmark["destination"]["page"])
                    self.assertAlmostEqual(heading["top"], PAGE_HEIGHT_PT - bookmark["destination"]["top"], places=3)

    def test_heading_map_keeps_suppressed_bookmarks_and_excludes_custom_bookmarks(self):
        headings, navigation = self.render(
            "<h1 id=first>First heading</h1><h1 style='display:none'>Hidden</h1>"
            "<h2 id=second style='bookmark-level:none'>Second heading</h2>"
            "<p style='bookmark-level:1'>Custom bookmark</p>"
        )
        self.assertEqual([item["id"] for item in headings], ["first", "second"])
        self.assertEqual([item["page"] for item in headings], [1, 2])
        self.assertEqual([item["title"] for item in navigation["outline"]], ["First heading", "Custom bookmark"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
