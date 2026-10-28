"""Inspect cross-part links against actual combined PDF page objects."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"


class MultipartNavigation(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-multipart-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)
        self.index = self.output / "index.html"
        self.body = self.output / "body.html"
        self.pdf = self.output / "combined.pdf"

    def invoke(self, *flags):
        return subprocess.run([str(RENDERER), str(self.index), str(self.pdf), "--part", str(self.body), *flags],
                              capture_output=True, text=True, timeout=60)

    def test_forward_backward_local_and_url_scoped_links_resolve_real_pages(self):
        self.index.write_text('<title>Combined index</title><style>@page{size:300pt 400pt;margin:20pt}'
                              'html,body{margin:0}p,h1{margin:0;font:12pt/18pt serif}</style><h1 id=index>Index</h1><p id=dup>Local duplicate</p>'
                              '<p><a href="#body">Forward</a></p><p><a href="body.html#dup">Explicit body duplicate</a></p>'
                              '<p><a href="#dup">Local duplicate</a></p><p><a href="#caf%C3%A9">Unicode</a></p>'
                              '<p><a href="#a%20b">Literal percent</a></p><p><a href="#legacy">Legacy</a></p>'
                              '<p><a href="#hidden">Hidden</a><a href="#missing">Missing</a></p>'
                              '<p><a href="https://example.org/outside#body">External</a></p>'
                              '<p><a href="mailto:reader@example.org">Mail</a></p>')
        self.body.write_text('<title>Body title</title><style>@page{size:360pt 500pt;margin:30pt}'
                             'html,body{margin:0}p,h1,h2{margin:0;font:20pt/24pt serif}h2{break-before:page}</style>'
                             '<h1 id=body>Body</h1><p id=dup>Body duplicate</p><p id="café">Unicode</p>'
                             '<p id="a%20b">Literal</p><a name=legacy>Legacy</a><p id=hidden style="display:none">Hidden</p>'
                             '<p><a href="#index">Backward</a></p><h2 id="a b">Decoded</h2>')
        headings = self.output / "headings.json"
        result = self.invoke("--headings", str(headings), "--trace")
        self.assertEqual(result.returncode, 0, result.stderr)
        document = pdf_content.Document(self.pdf)
        self.assertEqual([page["MediaBox"] for page in document.pages()],
                         [[0, 0, 300, 400], [0, 0, 360, 500], [0, 0, 360, 500]])
        nav = pdf_content.navigation(self.pdf)
        internal = [(link["page"], link["destination"]["page"]) for link in nav["links"] if "destination" in link]
        self.assertEqual(internal, [(1, 2), (1, 2), (1, 1), (1, 2), (1, 2), (1, 2), (2, 1)])
        destinations = [link["destination"] for link in nav["links"] if "destination" in link]
        for target, top in zip(destinations[:5], [470, 446, 362, 422, 398], strict=True):
            self.assertAlmostEqual(target["top"], top, places=3, msg="destination coordinates must use the target page geometry")
        self.assertAlmostEqual(destinations[-1]["top"], 380, places=3)
        self.assertEqual([link["uri"] for link in nav["links"] if "uri" in link],
                         ["https://example.org/outside#body", "mailto:reader@example.org"])
        self.assertEqual(nav["info"]["Title"], "Combined index")
        self.assertEqual([(node["title"], node["destination"]["page"]) for node in nav["outline"]], [("Index", 1), ("Body", 2)])
        self.assertEqual(nav["outline"][1]["children"][0]["destination"]["page"], 3)
        self.assertEqual([heading["page"] for heading in json.loads(headings.read_text())], [1, 2, 3])
        traces = json.loads(self.pdf.with_suffix(".trace.json").read_text())
        self.assertEqual([part["first_page"] for part in traces], [1, 2])
        self.assertEqual([len(part["layout"]["pages"]) for part in traces], [1, 2])

    def test_relative_fonts_and_images_stay_part_local_while_shared_dpi_uses_the_largest_size(self):
        from PIL import Image
        from fontTools.ttLib import TTFont

        folders = [self.output / "first", self.output / "second"]
        source_fonts = [ROOT / "tests/fixtures/fonts/FontStyleTest.ttf", ROOT / "tests/fixtures/fonts/SourceSerif4-Regular.ttf"]
        expected_names = []
        for folder, source_font, color, size in zip(folders, source_fonts, [(255, 0, 0), (0, 0, 255)], [(72, 48), (144, 96)], strict=True):
            folder.mkdir()
            (folder / "font.ttf").write_bytes(source_font.read_bytes())
            with TTFont(source_font) as font:
                expected_names.append(font["name"].getDebugName(6))
            Image.new("RGB", (64, 64), color).save(folder / "asset.png")
            (folder / "input.html").write_text('<style>@page{size:360pt 400pt;margin:20pt}'
                '@font-face{font-family:PartFont;src:url(font.ttf)}p{font-family:PartFont;font-size:18pt}</style>'
                '<p>AAA</p><img src="asset.png" style="width:48pt;height:48pt">'
                f'<img src="../shared.png" style="width:{size[0]}pt;height:{size[1]}pt">')
        Image.new("RGB", (384, 256), (37, 83, 149)).save(self.output / "shared.png")
        self.index, self.body = [folder / "input.html" for folder in folders]
        result = self.invoke("--image-dpi", "150")
        self.assertEqual(result.returncode, 0, result.stderr)
        document = pdf_content.Document(self.pdf)
        for page, expected in zip(document.pages(), expected_names, strict=True):
            resources = document.resolve(page["Resources"])
            fonts = document.resolve(resources["Font"])
            names = [document.resolve(reference)["BaseFont"] for reference in fonts.values()]
            self.assertTrue(names and all(name.endswith(expected) for name in names), (names, expected))
        images = [document.get(number) for number in document.offsets]
        images = [item for item in images if isinstance(item, pdf_content.Stream) and item.entries.get("Subtype") == "Image"]
        self.assertCountEqual([(item.entries["Width"], item.entries["Height"]) for item in images], [(64, 64), (64, 64), (300, 200)])
        colors = [document.decode(item) for item in images if item.entries["Width"] == 64]
        self.assertCountEqual(colors, [bytes((255, 0, 0)) * (64 * 64), bytes((0, 0, 255)) * (64 * 64)])

    def test_requested_symlink_url_controls_relative_resources_and_cross_part_links(self):
        from PIL import Image

        real = self.output / "real"
        real.mkdir()
        source = real / "source.html"
        source.write_text('<h1 id=index>Index</h1><img src="asset.png"><a href="body.html#body">Body</a>')
        self.index.symlink_to(source)
        self.body.write_text('<h1 id=body>Body</h1><a href="index.html#index">Index</a>')
        Image.new("RGB", (8, 8), (255, 0, 0)).save(self.output / "asset.png")
        Image.new("RGB", (8, 8), (0, 0, 255)).save(real / "asset.png")
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        nav = pdf_content.navigation(self.pdf)
        self.assertEqual([(link["page"], link.get("destination", {}).get("page")) for link in nav["links"]], [(1, 2), (2, 1)])
        document = pdf_content.Document(self.pdf)
        images = [document.get(number) for number in document.offsets]
        images = [item for item in images if isinstance(item, pdf_content.Stream) and item.entries.get("Subtype") == "Image"]
        self.assertEqual(len(images), 1)
        self.assertEqual(document.decode(images[0]), bytes((255, 0, 0)) * (8 * 8), "resources resolve beside the requested URL, not the symlink target")

    def test_trace_and_pdf_cannot_overwrite_each_other_through_hard_links(self):
        self.index.write_text("<h1>Index</h1>")
        self.body.write_text("<h1>Body</h1>")
        existing = b"existing output survives validation"
        self.pdf.write_bytes(existing)
        trace = self.pdf.with_suffix(".trace.json")
        os.link(self.pdf, trace)
        commands = [
            [str(RENDERER), str(self.index), str(self.pdf), "--trace"],
            [str(RENDERER), str(self.index), str(self.pdf), "--part", str(self.body), "--trace"],
        ]
        for command in commands:
            with self.subTest(command=command):
                result = subprocess.run(command, capture_output=True, text=True, timeout=60)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("aliases", result.stderr)
                self.assertEqual(self.pdf.read_bytes(), existing)
                self.assertEqual(trace.read_bytes(), existing)

    def test_stdin_stdout_and_each_parts_encoding_share_navigation(self):
        self.body = self.output / "body-café.html"
        self.body.write_bytes(b'<meta charset=windows-1252><h1 id=body>Body caf\xe9</h1><a href="#index">Index</a>')
        html = '<h1 id=index>Index café</h1><a href="#body">Body</a>'
        first = b"\xff\xfe" + html.encode("utf-16-le")
        headings = self.output / "headings.json"
        result = subprocess.run([str(RENDERER), "-", "-", "--part", str(self.body), "--headings", str(headings)],
                                input=first, capture_output=True, timeout=60, cwd=self.output)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertTrue(result.stdout.startswith(b"%PDF"))
        self.pdf.write_bytes(result.stdout)
        self.assertEqual([(link["page"], link["destination"]["page"]) for link in pdf_content.navigation(self.pdf)["links"]], [(1, 2), (2, 1)])
        self.assertEqual([(heading["text"], heading["page"]) for heading in json.loads(headings.read_text())], [("Index café", 1), ("Body café", 2)])

    def test_base_href_changes_link_resolution_without_changing_document_identity(self):
        self.body.write_text('<h1 id=dup>Body target</h1>')
        for base, expected in [("body.html", {"page": 2}), ("https://outside.example/page", {"uri": "https://outside.example/page#dup"})]:
            with self.subTest(base=base):
                self.index.write_text(f'<base href="{base}"><h1 id=dup>Local target</h1>'
                                      f'<p><a href="#dup">Base target</a></p><a href="{self.index.as_uri()}#dup">Own target</a>')
                result = self.invoke()
                self.assertEqual(result.returncode, 0, result.stderr)
                links = pdf_content.navigation(self.pdf)["links"]
                self.assertEqual(len(links), 2)
                if "uri" in expected:
                    self.assertEqual(links[0].get("uri"), expected["uri"])
                else:
                    self.assertEqual(links[0].get("destination", {}).get("page"), expected["page"])
                self.assertEqual(links[1].get("destination", {}).get("page"), 1)

    def test_duplicate_document_urls_and_outputs_aliasing_inputs_fail_without_writes(self):
        self.index.write_text("<h1>Index</h1>")
        original = b"<h1>Body</h1>"
        self.body.write_bytes(original)
        existing = b"existing PDF"
        self.pdf.write_bytes(existing)
        result = self.invoke("--part", str(self.index))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("same document URL", result.stderr)
        self.assertEqual(self.pdf.read_bytes(), existing)
        result = self.invoke("--headings", str(self.body))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("aliases", result.stderr)
        self.assertEqual(self.body.read_bytes(), original)
        self.assertEqual(self.pdf.read_bytes(), existing)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
