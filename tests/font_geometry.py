"""Check serialized font geometry independently of the renderer's layout trace."""
import argparse
import hashlib
import io
import json
import math
from pathlib import Path
import subprocess
import tempfile
import unittest
from urllib.parse import quote

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.ttLib import TTFont
from PIL import Image

from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/fonts"
RENDERER = ROOT / "target/debug/ferri-pdf"
FONT_UNIT_TOLERANCE = 1.0
MATRIX_TOLERANCE = 0.00001
PDF_POINTS_PER_INCH = 72
FONT_SIZE_PT = 20
SVG_WIDTH_PX = 100
VISIBLE_INK_LIMIT = 200


class InstanceInterpreter(pdf_content.Interpreter):
    def __init__(self, document):
        super().__init__(document)
        self.instances = {}
        self.painted_codes = []

    def font(self, resources, name):
        font = super().font(resources, name)
        reference = self.document.resolve(
            self.document.resolve(resources.get("Font", {}))
        )[name]
        identity = str(reference)
        font.name = identity
        if identity not in self.instances:
            decode = font.text
            def record(code):
                self.painted_codes.append((identity, code))
                return decode(code)
            font.text = record
        self.instances[identity] = (font, self.document.resolve(reference))
        return font

    def embedded_glyph_data(self, identity, character):
        font, _ = self.instances[identity]
        codes = [code for code, text in font.unicode.items() if text == character]
        if len(codes) != 1:
            raise AssertionError(f"expected one {character!r} character code, got {codes!r}")
        return self.embedded_glyph_code(identity, codes[0])

    def embedded_glyph_code(self, identity, code):
        _, dictionary = self.instances[identity]
        descendant = self.document.resolve(
            self.document.resolve(dictionary["DescendantFonts"])[0]
        )
        descriptor = self.document.resolve(descendant["FontDescriptor"])
        data = self.document.decode(self.document.resolve(descriptor["FontFile2"]))
        cid = int.from_bytes(code, "big")
        mapping = descendant.get("CIDToGIDMap", "Identity")
        if mapping == "Identity":
            glyph_id = cid
        else:
            mapping = self.document.decode(self.document.resolve(mapping))
            glyph_id = int.from_bytes(mapping[cid * 2:cid * 2 + 2], "big")
        return data, glyph_id

    def embedded_glyph(self, identity, character):
        data, glyph_id = self.embedded_glyph_data(identity, character)
        with TTFont(io.BytesIO(data)) as embedded:
            if "fvar" in embedded:
                raise AssertionError("the PDF must contain a static variation instance")
            name = embedded.getGlyphOrder()[glyph_id]
            glyphs = embedded.getGlyphSet()
            pen = BoundsPen(glyphs)
            glyphs[name].draw(pen)
            return embedded["head"].unitsPerEm, glyphs[name].width, pen.bounds


    def embedded_outline(self, identity, character):
        data, glyph_id = self.embedded_glyph_data(identity, character)
        return self.outline(data, glyph_id)

    def embedded_outline_code(self, identity, code):
        data, glyph_id = self.embedded_glyph_code(identity, code)
        return self.outline(data, glyph_id)

    @staticmethod
    def outline(data, glyph_id):
        with TTFont(io.BytesIO(data)) as embedded:
            glyphs = embedded.getGlyphSet()
            pen = DecomposingRecordingPen(glyphs)
            glyphs[embedded.getGlyphOrder()[glyph_id]].draw(pen)
            return json.loads(json.dumps(pen.value))


class FontGeometry(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-font-geometry-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)

    def render(self, css, body, arguments=()):
        source, target = self.output / "input.html", self.output / "output.pdf"
        source.write_text(f"<!doctype html><style>{css}</style>{body}")
        rendered = subprocess.run(
            [str(RENDERER), str(source), str(target), *arguments],
            capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        document = pdf_content.Document(target)
        interpreter = InstanceInterpreter(document)
        for index, page in enumerate(document.pages(), start=1):
            interpreter.run_page(index, page)
        return interpreter

    def raster(self, dpi=PDF_POINTS_PER_INCH):
        prefix = self.output / "raster"
        subprocess.run(["pdftoppm", "-singlefile", "-r", str(dpi), "-png", str(self.output / "output.pdf"), str(prefix)],
                       check=True, capture_output=True, timeout=60)
        with Image.open(prefix.with_suffix(".png")) as raster:
            return raster.convert("L")

    def assert_extracted_text(self, expected):
        extracted = subprocess.run(["pdftotext", "-raw", str(self.output / "output.pdf"), "-"],
                                   check=True, capture_output=True, text=True, timeout=60).stdout.strip()
        self.assertEqual(extracted, expected)

    def assert_character_policy_geometry(self, interpreted, expected):
        reference = json.loads((FIXTURES / "character-policy-reference.json").read_text())
        source = ROOT / reference["source"]
        self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), reference["sha256"])
        self.assertEqual(len(interpreted.glyphs), len(expected))
        first_mapping = {}
        for glyph, (character, key) in zip(interpreted.glyphs, expected):
            # Shared GIDs use their first ToUnicode mapping and ActualText for later aliases.
            mapped = first_mapping.setdefault((glyph.font, key), character)
            self.assertEqual(glyph.text, mapped)
        raster = self.raster()
        for glyph, (character, key) in zip(interpreted.glyphs, expected):
            want = reference["characters"][key]
            upem, advance, bounds = interpreted.embedded_glyph(glyph.font, glyph.text)
            if "source" in want:
                self.assertEqual(hashlib.sha256((ROOT / want["source"]).read_bytes()).hexdigest(), want["sha256"])
            self.assertEqual(upem, want.get("units_per_em", reference["units_per_em"]))
            self.assertEqual(interpreted.embedded_outline(glyph.font, glyph.text), want["outline"])
            self.assertAlmostEqual(advance, want["advance"], delta=FONT_UNIT_TOLERANCE)
            self.assertIsNotNone(bounds, "missing characters must have visible embedded outlines")
            for actual, value in zip(bounds, want["bounds"]):
                self.assertAlmostEqual(actual, value, delta=FONT_UNIT_TOLERANCE)
            self.assertAlmostEqual(glyph.matrix[0], 20, delta=MATRIX_TOLERANCE)
            self.assertAlmostEqual(glyph.matrix[3], 20, delta=MATRIX_TOLERANCE)
            left, bottom, right, top = (value / upem * 20 for value in bounds)
            x, y = glyph.origin
            region = (math.floor(x + left), math.floor(raster.height - y - top),
                      math.ceil(x + right), math.ceil(raster.height - y - bottom))
            self.assertLess(raster.crop(region).getextrema()[0], VISIBLE_INK_LIMIT,
                            f"{character!r} must paint a visible symbol or its explicitly named private glyph")

    def assert_bundled_glyph(self, interpreted, glyph, filename, character="H", size=FONT_SIZE_PT):
        sources = ROOT / "assets/fonts"
        hashes = {row.split()[1]: row.split()[0] for row in (sources / "SHA256SUMS").read_text().splitlines()}
        path = sources / filename
        self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), hashes[filename])
        self.assert_font_glyph(interpreted, glyph, path, character, size)

    def assert_font_glyph(self, interpreted, glyph, path, character, size=FONT_SIZE_PT):
        self.assertEqual(glyph.text, character)
        upem, advance, _ = interpreted.embedded_glyph(glyph.font, character)
        with TTFont(path) as original:
            glyphs = original.getGlyphSet()
            expected = glyphs[original.getBestCmap()[ord(character)]]
            pen = DecomposingRecordingPen(glyphs)
            expected.draw(pen)
            self.assertEqual(upem, original["head"].unitsPerEm)
            self.assertAlmostEqual(advance, expected.width, delta=FONT_UNIT_TOLERANCE)
            self.assertEqual(interpreted.embedded_outline(glyph.font, character), json.loads(json.dumps(pen.value)))
        self.assertAlmostEqual(glyph.matrix[0], size, delta=MATRIX_TOLERANCE)
        self.assertAlmostEqual(glyph.matrix[3], size, delta=MATRIX_TOLERANCE)

    def test_svg_text_uses_document_font_faces_without_prior_html_text(self):
        sources = ROOT / "assets/fonts"
        for style, files in [("normal", ["DejaVuSansMono.ttf"]),
                             ("italic bold", ["DejaVuSansMono-BoldOblique.ttf"])]:
            filename = files[0]
            weight = "bold" if "bold" in style else "normal"
            slope = "italic" if "italic" in style else "normal"
            css = ("@page{size:240pt 200pt;margin:20pt}html,body{margin:0;padding:0}img{display:block}"
                   f"@font-face{{font-family:ChartFace;src:url('{(sources / filename).as_uri()}');"
                   f"font-weight:{weight};font-style:{slope}}}")
            for width in [100, 150]:
                with self.subTest(style=style, width=width):
                    svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="40" viewBox="0 0 100 40">'
                           f'<text x="0" y="30" font-family="ChartFace" font-size="20pt" '
                           f'font-weight="{weight}" font-style="{slope}">H</text></svg>')
                    interpreted = self.render(css, f'<img style="width:{width}px" src="data:image/svg+xml,{quote(svg)}">')
                    self.assertEqual([glyph.text for glyph in interpreted.glyphs], ["H"])
                    self.assert_bundled_glyph(interpreted, interpreted.glyphs[0], filename,
                                              size=FONT_SIZE_PT * width / SVG_WIDTH_PX)

    def test_svg_fallback_keeps_covered_glyphs_in_the_requested_face(self):
        primary = FIXTURES / "VariationSequenceTest.ttf"
        css = ("@page{size:240pt 160pt;margin:20pt}html,body{margin:0;padding:0}"
               f"@font-face{{font-family:ChartPrimary;src:url('{primary.as_uri()}')}}")
        for families, fallback in [("ChartPrimary", "DejaVuSansMono.ttf"),
                                   ("ChartPrimary,Liberation Serif", "LiberationSerif-Regular.ttf")]:
            with self.subTest(families=families):
                svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="40">'
                       f'<text x="0" y="30" font-family="{families}" font-size="20pt">A→A</text></svg>')
                interpreted = self.render(css, f'<img src="data:image/svg+xml,{quote(svg)}">',
                                          ["--font-fallback", "DejaVu Sans Mono"])
                self.assertEqual([glyph.text for glyph in interpreted.glyphs], ["A", "→", "A"])
                self.assert_font_glyph(interpreted, interpreted.glyphs[0], primary, "A")
                self.assert_bundled_glyph(interpreted, interpreted.glyphs[1], fallback, "→")
                self.assert_font_glyph(interpreted, interpreted.glyphs[2], primary, "A")
                self.assertEqual(interpreted.glyphs[0].font, interpreted.glyphs[2].font)
                self.assertNotEqual(interpreted.glyphs[0].font, interpreted.glyphs[1].font)

    def test_svg_font_environment_survives_inline_background_nested_and_shadow_images(self):
        sources = ROOT / "assets/fonts"
        css = ("@page{size:240pt 200pt;margin:20pt}html,body{margin:0;padding:0}img{display:block}"
               f"@font-face{{font-family:ChartFace;src:url('{(sources / 'DejaVuSansMono.ttf').as_uri()}')}}")
        svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="40">'
               '<text x="0" y="30" font-family="ChartFace" font-size="20pt">H</text></svg>')
        uri = "data:image/svg+xml," + quote(svg)
        nested = ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="40">'
                  f'<image href="{uri}" width="100" height="40"/></svg>')
        for kind, body in [("inline", svg),
                           ("background", f'<div style="width:100px;height:40px;background-image:url({uri})"></div>'),
                           ("nested", f'<img src="data:image/svg+xml,{quote(nested)}">')]:
            with self.subTest(kind=kind):
                interpreted = self.render(css, body)
                if kind == "background":
                    actual = self.raster(PDF_POINTS_PER_INCH * 2)
                    self.assertLess(actual.getextrema()[0], VISIBLE_INK_LIMIT, "chart label must paint visible ink")
                    reference = svg.replace('font-family="ChartFace"', 'font-family="DejaVu Sans Mono"')
                    self.render(css, f'<div style="width:100px;height:40px;background-image:url(data:image/svg+xml,{quote(reference)})"></div>')
                    self.assertEqual(actual.tobytes(), self.raster(PDF_POINTS_PER_INCH * 2).tobytes(), "pattern paint must match the independently checked physical family")
                else:
                    self.assertEqual([glyph.text for glyph in interpreted.glyphs], ["H"])
                    self.assert_bundled_glyph(interpreted, interpreted.glyphs[0], "DejaVuSansMono.ttf")
        shadow = (f'<img src="{uri}"><div><template shadowrootmode="open"><style>'
                  f"@font-face{{font-family:ChartFace;src:url('{(sources / 'DejaVuSans.ttf').as_uri()}')}}"
                  f'</style><img src="{uri}"></template></div><img src="{uri}">')
        interpreted = self.render(css, shadow)
        self.assertEqual([glyph.text for glyph in interpreted.glyphs], ["H", "H", "H"])
        for glyph, filename in zip(interpreted.glyphs, ["DejaVuSansMono.ttf", "DejaVuSans.ttf", "DejaVuSansMono.ttf"]):
            self.assert_bundled_glyph(interpreted, glyph, filename)

    def test_svg_variable_font_respects_face_ranges_and_explicit_axis_settings(self):
        source = FIXTURES / "RobotoFlex.ttf"
        css = ("@page{size:240pt 160pt;margin:20pt}html,body{margin:0;padding:0}"
               f"@font-face{{font-family:ChartVariable;src:url('{source.as_uri()}');font-weight:100 500}}")
        for explicit, expected_weight in [("", 500), ('font-variation-settings="&quot;wght&quot; 800"', 800)]:
            with self.subTest(explicit=explicit):
                svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="40">'
                       '<text x="0" y="30" font-family="ChartVariable" font-size="20pt" '
                       f'font-weight="900" font-optical-sizing="none" {explicit}>H</text></svg>')
                interpreted = self.render(css, f'<img src="data:image/svg+xml,{quote(svg)}">')
                self.assertEqual([glyph.text for glyph in interpreted.glyphs], ["H"])
                with TTFont(source) as original:
                    glyphs = original.getGlyphSet(location={"wght": expected_weight})
                    expected = glyphs[original.getBestCmap()[ord("H")]]
                    upem, advance, bounds = interpreted.embedded_glyph(interpreted.glyphs[0].font, "H")
                    pen = BoundsPen(glyphs)
                    expected.draw(pen)
                    self.assertEqual(upem, original["head"].unitsPerEm)
                    self.assertAlmostEqual(advance, expected.width, delta=FONT_UNIT_TOLERANCE)
                    for actual, value in zip(bounds, pen.bounds):
                        self.assertAlmostEqual(actual, value, delta=FONT_UNIT_TOLERANCE)

    def test_svg_fallback_reshapes_adjacent_characters_as_one_ligature_run(self):
        primary = FIXTURES / "VariationSequenceTest.ttf"
        css = ("@page{size:240pt 160pt;margin:20pt}html,body{margin:0;padding:0}"
               f"@font-face{{font-family:ChartPrimary;src:url('{primary.as_uri()}')}}")
        for text in ["AfiA", "Afia\u0346A"]:
            with self.subTest(text=text):
                svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="40">'
                       f'<text x="0" y="30" font-family="ChartPrimary" font-size="20pt">{text}</text></svg>')
                interpreted = self.render(css, f'<img src="data:image/svg+xml,{quote(svg)}">',
                                          ["--font-fallback", "DejaVu Serif", "--font-fallback", "DejaVu Sans"])
                self.assert_extracted_text(text)
                self.assertEqual(interpreted.glyphs[1].text, "fi")
                self.assert_font_glyph(interpreted, interpreted.glyphs[0], primary, "A")
                self.assert_font_glyph(interpreted, interpreted.glyphs[-1], primary, "A")
                with TTFont(ROOT / "assets/fonts/DejaVuSerif.ttf") as original:
                    glyphs = original.getGlyphSet()
                    expected = glyphs[original.getBestCmap()[ord("ﬁ")]]
                    pen = DecomposingRecordingPen(glyphs)
                    expected.draw(pen)
                    self.assertEqual(interpreted.embedded_outline(interpreted.glyphs[1].font, "fi"), json.loads(json.dumps(pen.value)))
                    if "\u0346" in text:
                        self.assertNotIn(ord("\u0346"), original.getBestCmap())
                        self.assertNotEqual(interpreted.glyphs[1].font, interpreted.glyphs[-2].font)
                if "\u0346" in text:
                    reference = json.loads((FIXTURES / "svg-cluster-reference.json").read_text())
                    source = ROOT / reference["source"]
                    self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), reference["sha256"])
                    self.assertEqual(len(interpreted.glyphs), 5)
                    self.assertEqual(interpreted.glyphs[2].font, interpreted.glyphs[3].font)
                    with TTFont(source) as original:
                        glyphs = original.getGlyphSet()
                        for index, shaped in zip([2, 3], reference["glyphs"]):
                            pen = DecomposingRecordingPen(glyphs)
                            glyphs[original.getGlyphOrder()[shaped["g"]]].draw(pen)
                            self.assertEqual(interpreted.embedded_outline_code(*interpreted.painted_codes[index]),
                                             json.loads(json.dumps(pen.value)), msg=f"painted glyph {index}: {shaped}")
                    base, mark = reference["glyphs"]
                    expected = [base["ax"] + mark["dx"] - base["dx"], mark["dy"] - base["dy"]]
                    for component, units in zip([4, 5], expected):
                        actual = interpreted.glyphs[3].matrix[component] - interpreted.glyphs[2].matrix[component]
                        self.assertAlmostEqual(actual, units * FONT_SIZE_PT / reference["units_per_em"],
                                               delta=MATRIX_TOLERANCE, msg=f"combining mark matrix component {component}")

    def test_svg_generics_share_document_font_preferences(self):
        for generic, flag, family, filename in [
                ("", "--font-serif", "DejaVu Serif", "DejaVuSerif.ttf"),
                ("sans-serif", "--font-sans-serif", "DejaVu Sans", "DejaVuSans.ttf"),
                ("serif", "--font-serif", "DejaVu Serif", "DejaVuSerif.ttf"),
                ("monospace", "--font-monospace", "DejaVu Sans Mono", "DejaVuSansMono.ttf")]:
            with self.subTest(generic=generic):
                family_attribute = f'font-family="{generic}"' if generic else ""
                svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="40">'
                       f'<text x="0" y="30" {family_attribute} font-size="20pt">H</text></svg>')
                interpreted = self.render("@page{size:240pt 160pt;margin:20pt}html,body{margin:0;padding:0}",
                                          f'<img src="data:image/svg+xml,{quote(svg)}">', [flag, family])
                self.assertEqual([glyph.text for glyph in interpreted.glyphs], ["H"])
                self.assert_bundled_glyph(interpreted, interpreted.glyphs[0], filename)

    def test_generic_preferences_choose_the_actual_normal_and_styled_fonts(self):
        cases = [("sans-serif", "--font-sans-serif", "DejaVu Sans", "DejaVuSans", "BoldOblique"),
                 ("serif", "--font-serif", "DejaVu Serif", "DejaVuSerif", "BoldItalic"),
                 ("monospace", "--font-monospace", "DejaVu Sans Mono", "DejaVuSansMono", "BoldOblique")]
        for generic, flag, family, filename, styled in cases:
            for style, suffix in [("", ""), ("italic bold ", "-" + styled)]:
                with self.subTest(generic=generic, style=style):
                    css = ("@page{size:240pt 160pt;margin:20pt}html,body,p{margin:0;padding:0;"
                           f"font:{style}20pt/24pt {generic}}}")
                    interpreted = self.render(css, "<p>H</p>", [flag, "Unavailable Family", flag, family])
                    self.assertEqual(len(interpreted.glyphs), 1)
                    self.assert_bundled_glyph(interpreted, interpreted.glyphs[0], filename + suffix + ".ttf")
                    self.assertAlmostEqual(interpreted.glyphs[0].matrix[2], 0, delta=MATRIX_TOLERANCE,
                                           msg="a real italic/oblique face must not acquire synthetic skew")

    def test_preferred_fallback_preserves_requested_fonts_and_private_use_policy(self):
        primary = FIXTURES / "VariationSequenceTest.ttf"
        css = ("@page{size:240pt 160pt;margin:20pt}html,body,p{margin:0;padding:0;font:20pt/24pt Primary}"
               f"@font-face{{font-family:Primary;src:url('{primary.as_uri()}')}}")
        flags = ["--font-fallback", "Unavailable Family", "--font-fallback", "DejaVu Sans Mono"]
        interpreted = self.render(css, "<p>A→A</p>", flags)
        self.assertEqual([glyph.text for glyph in interpreted.glyphs], ["A", "→", "A"])
        self.assert_font_glyph(interpreted, interpreted.glyphs[0], primary, "A")
        self.assert_bundled_glyph(interpreted, interpreted.glyphs[1], "DejaVuSansMono.ttf", "→")
        self.assert_font_glyph(interpreted, interpreted.glyphs[2], primary, "A")
        interpreted = self.render(css + "p{font-family:Primary,'Liberation Serif'}", "<p>A→A</p>", flags)
        self.assertEqual([glyph.text for glyph in interpreted.glyphs], ["A", "→", "A"])
        self.assert_font_glyph(interpreted, interpreted.glyphs[0], primary, "A")
        self.assert_bundled_glyph(interpreted, interpreted.glyphs[1], "LiberationSerif-Regular.ttf", "→")
        self.assert_font_glyph(interpreted, interpreted.glyphs[2], primary, "A")
        interpreted = self.render(css + "p{font-family:sans-serif}", "<p>\uef00</p>",
                                  ["--font-sans-serif", "DejaVu Sans", "--font-fallback", "DejaVu Sans"])
        self.assert_character_policy_geometry(interpreted, [("\uef00", "replacement")])
        self.assert_extracted_text("\uef00")

    def test_generic_preferences_do_not_rebind_authored_webfont_aliases(self):
        other = (ROOT / "assets/fonts/LiberationSerif-Regular.ttf").as_uri()
        css = ("@page{size:240pt 160pt;margin:20pt}html,body,p{margin:0;padding:0;font:20pt/24pt sans-serif}"
               f"@font-face{{font-family:'DejaVu Sans';src:url('{other}')}}")
        flags = ["--font-sans-serif", "DejaVu Sans"]
        interpreted = self.render(css, "<p>H</p>", flags)
        self.assert_bundled_glyph(interpreted, interpreted.glyphs[0], "DejaVuSans.ttf")
        interpreted = self.render(css + "p{font-family:'DejaVu Sans'}", "<p>H</p>", flags)
        self.assert_bundled_glyph(interpreted, interpreted.glyphs[0], "LiberationSerif-Regular.ttf")

    def test_unicode_family_names_choose_the_correct_embedded_face(self):
        reference = json.loads((FIXTURES / "font-name-matching.json").read_text())
        mono = (ROOT / "assets/fonts/LiberationMono-Regular.ttf").as_uri()
        for case in reference["cases"]:
            declared, requested = case["declared"], case["requested"]
            with self.subTest(declared=declared, requested=requested):
                css = ("@page{size:240pt 160pt;margin:20pt}"
                       f"@font-face{{font-family:'{declared}';src:url('{mono}')}}"
                       f"p{{margin:0;font:20pt/24pt '{requested}',serif}}")
                interpreted = self.render(css, "<p lang=tr>H</p>")
                self.assertEqual(len(interpreted.glyphs), 1)
                expected = "LiberationMono-Regular.ttf" if case["matches"] else "LiberationSerif-Regular.ttf"
                self.assert_bundled_glyph(interpreted, interpreted.glyphs[0], expected)
                self.assert_extracted_text("H")

    def test_unicode_names_keep_shadow_and_local_resolution_in_pdf(self):
        sources = ROOT / "assets/fonts"
        serif, mono = [(sources / filename).as_uri() for filename in
                       ["LiberationSerif-Regular.ttf", "LiberationMono-Regular.ttf"]]
        css = ("@page{size:240pt 160pt;margin:20pt}"
               f"@font-face{{font-family:STRASSE;src:url('{serif}')}}"
               "p{margin:0;font:20pt/24pt STRASSE}")
        body = ("<div><template shadowrootmode=open><style>"
                f"@font-face{{font-family:'Straße';src:url('{mono}')}}"
                "p{margin:0;font:20pt/24pt STRASSE}</style><p>H</p></template></div><p>H</p>")
        interpreted = self.render(css, body)
        self.assertEqual(len(interpreted.glyphs), 2)
        for glyph, filename in zip(interpreted.glyphs, ["LiberationMono-Regular.ttf", "LiberationSerif-Regular.ttf"]):
            self.assert_bundled_glyph(interpreted, glyph, filename)
        for local in ["DejaVu ſans", "DejaVuſans"]:
            with self.subTest(local=local):
                css = ("@page{size:240pt 160pt;margin:20pt}"
                       f"@font-face{{font-family:LocalAlias;src:local('{local}')}}"
                       "p{margin:0;font:20pt/24pt LocalAlias}")
                interpreted = self.render(css, "<p>H</p>")
                self.assertEqual(len(interpreted.glyphs), 1)
                self.assert_bundled_glyph(interpreted, interpreted.glyphs[0], "DejaVuSans.ttf")
                self.assert_extracted_text("H")

    def test_variation_sequences_embed_the_requested_glyph_and_original_unicode(self):
        reference = json.loads((FIXTURES / "VariationSequenceTest-source.json").read_text())
        face = FIXTURES / "VariationSequenceTest.ttf"
        self.assertEqual(hashlib.sha256(face.read_bytes()).hexdigest(), reference["sha256"])
        self.assertEqual(hashlib.sha256((ROOT / reference["derived_from"]).read_bytes()).hexdigest(),
                         reference["source_sha256"])
        css = ("@page{size:240pt 160pt;margin:20pt}"
               f"@font-face{{font-family:Variant;src:url('{face.as_uri()}')}}"
               "html,body,p{margin:0;padding:0;font:20pt/24pt 'Liberation Serif',Variant}")
        for sample in reference["samples"]:
            text = sample["text"]
            with self.subTest(text=text):
                interpreted = self.render(css, "<p>" + text + "</p>")
                self.assertEqual(len(interpreted.glyphs), 1)
                glyph = interpreted.glyphs[0]
                self.assertEqual(glyph.text, text)
                self.assertEqual(interpreted.embedded_outline(glyph.font, text), sample["outline"])
                upem, advance, _ = interpreted.embedded_glyph(glyph.font, text)
                self.assertEqual(upem, reference["units_per_em"])
                self.assertAlmostEqual(advance, sample["advance"], delta=FONT_UNIT_TOLERANCE)
                self.assertAlmostEqual(glyph.matrix[0], 20, delta=MATRIX_TOLERANCE)
                self.assertAlmostEqual(glyph.matrix[3], 20, delta=MATRIX_TOLERANCE)
                self.assert_extracted_text(text)

    def test_missing_symbols_keep_original_unicode_and_visible_outlines(self):
        css = "@page{size:240pt 160pt;margin:20pt}html,body,p{margin:0;padding:0;font:20pt/24pt serif}"
        text = "\u0378\u0380\U0010fffd"
        interpreted = self.render(css, "<p>" + text + "</p>")
        self.assert_character_policy_geometry(interpreted, [(character, "replacement") for character in text])
        self.assert_extracted_text(text)
        # Poppler sanitizes noncharacters; verify their serialized ActualText directly.
        text = "\u0378\U0010ffff"
        interpreted = self.render(css, "<p>" + text + "</p>")
        self.assert_character_policy_geometry(interpreted, [(character, "replacement") for character in text])
        spans = []
        for page in interpreted.document.pages():
            for reference in interpreted.document.contents(page):
                data = interpreted.document.decode(interpreted.document.resolve(reference))
                position, operands = 0, []
                while (position := pdf_content.skip_space(data, position)) < len(data):
                    value, position = pdf_content.parse(data, position)
                    if isinstance(value, pdf_content.Operator):
                        if value == "BDC" and "ActualText" in operands[-1]:
                            spans.append(pdf_content.text_string(operands[-1]["ActualText"]))
                        operands = []
                    else:
                        operands.append(value)
        self.assertEqual(spans, ["\U0010ffff"])

    def test_missing_marks_preserve_the_named_prefix_in_serialized_clusters(self):
        for families, base, key in [("'DejaVu Sans'", "\uef00", "private_use"), ("serif", "Ω", "greek_prefix")]:
            with self.subTest(families=families):
                css = ("@page{size:240pt 160pt;margin:20pt}html,body,p{margin:0;padding:0;"
                       f"font:20pt/24pt {families}}}")
                interpreted = self.render(css, "<p>" + base + "\u1ab0</p>")
                self.assert_character_policy_geometry(interpreted, [(base, key), ("\u1ab0", "replacement")])
                self.assert_extracted_text(base + "\u1ab0")

    def test_private_use_glyphs_match_only_named_families_in_their_namespace(self):
        for families, expected in [("sans-serif", "replacement"),
                                   ("'Liberation Serif',sans-serif", "replacement"),
                                   ("'DejaVu Sans'", "private_use"),
                                   ("serif,'DejaVu Sans'", "private_use")]:
            with self.subTest(families=families):
                css = ("@page{size:240pt 160pt;margin:20pt}html,body,p{margin:0;padding:0;"
                       f"font:20pt/24pt {families}}}")
                interpreted = self.render(css, "<p>\uef00</p>")
                self.assert_character_policy_geometry(interpreted, [("\uef00", expected)])
                self.assert_extracted_text("\uef00")
        shadow = (ROOT / "assets/fonts/LiberationSerif-Regular.ttf").as_uri()
        css = "@page{size:240pt 160pt;margin:20pt}html,body,p{margin:0;padding:0;font:20pt/24pt serif}"
        body = ("<div><template shadowrootmode=open><style>"
                f"@font-face{{font-family:'DejaVu Sans';src:url('{shadow}')}}"
                "p{font-family:'DejaVu Sans',sans-serif}</style><p>\uef00</p></template></div>")
        interpreted = self.render(css, body)
        self.assert_character_policy_geometry(interpreted, [("\uef00", "replacement")])
        self.assert_extracted_text("\uef00")

    def test_font_shorthand_small_caps_is_visible_in_pdf(self):
        native = (FIXTURES / "SourceSerif4-Regular.ttf").as_uri()
        synthetic = (ROOT / "assets/fonts/LiberationSerif-Regular.ttf").as_uri()
        css = f"""@page{{size:300pt 300pt;margin:20pt}}
        @font-face{{font-family:NativeCaps;src:url('{native}')}}
        @font-face{{font-family:SyntheticCaps;src:url('{synthetic}')}}
        p{{margin:0;font:small-caps 20pt/32pt SyntheticCaps}}
        .native{{font-family:NativeCaps;font-synthesis:none}}"""
        interpreted = self.render(css, "<p class=native>a</p><p>a</p><p lang=tr>Aaß iı 123</p>")
        glyphs = interpreted.glyphs
        self.assertEqual(glyphs[0].text, "a")
        self.assertEqual(glyphs[1].text, "a")
        # These GIDs come from independent HarfBuzz, not the rendering trace.
        for glyph, path, original_gid, size in [
            (glyphs[0], FIXTURES / "SourceSerif4-Regular.ttf", 1113, 20.0),
            (glyphs[1], ROOT / "assets/fonts/LiberationSerif-Regular.ttf", 36, 16.0),
        ]:
            upem, advance, bounds = interpreted.embedded_glyph(glyph.font, "a")
            with TTFont(path) as original:
                expected = original.getGlyphSet()[original.getGlyphOrder()[original_gid]]
                pen = BoundsPen(original.getGlyphSet())
                expected.draw(pen)
                self.assertEqual(upem, original["head"].unitsPerEm)
                self.assertAlmostEqual(advance, expected.width, delta=FONT_UNIT_TOLERANCE)
                for actual, want in zip(bounds, pen.bounds):
                    self.assertAlmostEqual(actual, want, delta=FONT_UNIT_TOLERANCE)
            self.assertAlmostEqual(abs(glyph.matrix[3]), size, delta=MATRIX_TOLERANCE)
        extracted = subprocess.run(
            ["pdftotext", "-enc", "UTF-8", "-raw", str(self.output / "output.pdf"), "-"],
            capture_output=True, text=True, check=True, timeout=60,
        ).stdout
        self.assertEqual(extracted.split(), ["a", "a", "Aaß", "iı", "123"])

    def test_wrapped_small_caps_preserve_original_text_across_pages(self):
        css = """@page{size:180pt 120pt;margin:10pt}
        p{margin:0;width:85pt;font:small-caps 20pt/28pt serif;
        letter-spacing:1pt;text-decoration:underline}"""
        text = "Aaß test iı words many times wrapped across narrow lines " * 4
        painted = self.render(css, f"<p lang=tr>{text}</p>").glyphs
        self.assertGreater(max(glyph.page for glyph in painted), 1)
        extracted = subprocess.run(
            ["pdftotext", "-enc", "UTF-8", "-raw", str(self.output / "output.pdf"), "-"],
            capture_output=True, text=True, check=True, timeout=60,
        ).stdout
        self.assertEqual(extracted.split(), text.split())
        self.assertTrue(all(0 <= glyph.origin[0] <= 180 for glyph in painted))
        self.assertTrue(all(0 <= glyph.origin[1] <= 120 for glyph in painted))

    def test_zero_font_size_preserves_spacing_without_painting_glyphs(self):
        css = """@page{size:300pt 200pt;margin:20pt}
        p{margin:0;font:20pt/24pt serif}
        span{font-size:0;line-height:0;letter-spacing:3pt;word-spacing:2pt}"""
        painted = self.render(css, "<p style='font:0/0 serif'>😀中文</p><p>A<span>a b😀</span>B</p>").glyphs
        self.assertEqual([glyph.text for glyph in painted], ["A", "B"])
        # Liberation Serif A advances 1479/2048 em; four 3pt letter spaces
        # and one additional 2pt word space remain in the zero-size span.
        self.assertAlmostEqual(painted[1].origin[0] - painted[0].origin[0],
                               1479 / 2048 * 20 + 14, delta=MATRIX_TOLERANCE)
        self.assertAlmostEqual(painted[1].origin[1], painted[0].origin[1],
                               delta=MATRIX_TOLERANCE)

    def test_zero_line_height_retains_visible_glyphs(self):
        css = """@page{size:300pt 200pt;margin:30pt}
        p{margin:0;font:20pt/0 serif}"""
        painted = self.render(css, "<p>A<br>B<br>C</p>").glyphs
        self.assertEqual([glyph.text for glyph in painted], ["A", "B", "C"])
        for glyph in painted:
            self.assertAlmostEqual(glyph.origin[1], painted[0].origin[1], delta=MATRIX_TOLERANCE)
            self.assertAlmostEqual(abs(glyph.matrix[3]), 20, delta=MATRIX_TOLERANCE)

    def test_ideographic_fallback_embeds_the_measured_glyphs(self):
        sources = json.loads((FIXTURES / "IcTest-source.json").read_text())
        latin = (FIXTURES / "MetricGlyphTest.ttf").as_uri()
        for filename, expected in sources["faces"].items():
            with self.subTest(font=filename):
                water = (FIXTURES / filename).as_uri()
                css = f"""@font-face{{font-family:Latin;src:url('{latin}')}}
                @font-face{{font-family:Water;src:url('{water}')}}
                @page{{size:300pt 200pt;margin:20pt}}
                p{{margin:0;font:20pt/28pt Latin,Water;width:1ic}}"""
                interpreted = self.render(css, "<p>水</p>")
                self.assertEqual([glyph.text for glyph in interpreted.glyphs], ["水"])
                upem, advance, bounds = interpreted.embedded_glyph(interpreted.glyphs[0].font, "水")
                self.assertEqual(upem, expected["units_per_em"])
                self.assertEqual(advance, expected["water_advance"])
                for actual, want in zip(bounds, expected["water_bounds"]):
                    self.assertAlmostEqual(actual, want, delta=FONT_UNIT_TOLERANCE)

    def test_page_font_size_initial_units_reach_inherited_margin_text(self):
        css = """@page{size:400pt 600pt;margin:40pt;font-size:2em;
        @top-left{content:'M';font-size:1em}}
        html{font-family:'DejaVu Sans';font-size:25pt}
        body,p{margin:0}p{font:12pt/18pt serif}"""
        painted = self.render(css, "<p>Body</p>").glyphs
        margin = [glyph for glyph in painted if glyph.text == "M"]
        self.assertEqual(len(margin), 1)
        self.assertAlmostEqual(abs(margin[0].matrix[3]), 24, delta=MATRIX_TOLERANCE)
        body = [glyph for glyph in painted if glyph.text == "B"]
        self.assertEqual(len(body), 1)
        self.assertAlmostEqual(abs(body[0].matrix[3]), 12, delta=MATRIX_TOLERANCE)

    def test_page_box_content_geometry_and_pagination_reach_pdf(self):
        for declarations, paper, content, count, pages_expected in [
            ("margin:20pt;padding:10pt;border:2pt solid", [400.0, 600.0], [32.0, 32.0, 336.0, 536.0], 45, 2),
            ("margin:auto;padding:10pt;border:2pt solid;width:200pt;height:300pt", [400.0, 600.0], [100.0, 150.0, 200.0, 300.0], 1, 1),
            ("margin:20pt;padding:10pt;border:2pt solid;width:200pt;height:300pt", [400.0, 600.0], [32.0, 32.0, 200.0, 300.0], 1, 1),
        ]:
            with self.subTest(page_box=declarations):
                css = f"""@page{{size:400pt 600pt;{declarations}}}
                body,p{{margin:0;font:8pt/12pt serif}}"""
                interpreted = self.render(css, "<p>B</p>" * count)
                pages = list(interpreted.document.pages())
                self.assertEqual(len(pages), pages_expected)
                for page in pages:
                    box = interpreted.document.resolve(page["MediaBox"])
                    for actual, expected in zip(box, [0.0, 0.0, *paper]):
                        self.assertAlmostEqual(actual, expected, delta=MATRIX_TOLERANCE)
                glyphs = interpreted.glyphs
                self.assertEqual([glyph.text for glyph in glyphs], ["B"] * count)
                x, y, width, height = content
                for glyph in glyphs:
                    self.assertAlmostEqual(glyph.origin[0], x, delta=MATRIX_TOLERANCE)
                    top_down_y = paper[1] - glyph.origin[1]
                    self.assertGreaterEqual(top_down_y, y)
                    self.assertLessEqual(top_down_y, y + height)
                if pages_expected > 1:
                    self.assertEqual(sum(glyph.page == 1 for glyph in glyphs), 44)

    def test_cross_axis_page_dependencies_reach_pdf_paper_and_margin_fonts(self):
        for size, unit, paper, page_font in [
            ("20em 600pt", "vh", [112.0, 600.0], 5.6),
            ("400pt 20em", "vw", [400.0, 72.0], 3.6),
        ]:
            with self.subTest(viewport=unit):
                css = f"""@page{{size:{size};font-size:1{unit};margin:20pt;
                @top-center{{content:'M';font-size:1em}}}}
                body,p{{margin:0;font:8pt/12pt serif}}"""
                interpreted = self.render(css, "<p>B</p>")
                pages = list(interpreted.document.pages())
                self.assertEqual(len(pages), 1)
                box = interpreted.document.resolve(pages[0]["MediaBox"])
                for actual, expected in zip(box, [0.0, 0.0, *paper]):
                    self.assertAlmostEqual(actual, expected, delta=MATRIX_TOLERANCE)
                margin = [glyph for glyph in interpreted.glyphs if glyph.text == "M"]
                body = [glyph for glyph in interpreted.glyphs if glyph.text == "B"]
                self.assertEqual(len(margin), 1)
                self.assertEqual(len(body), 1)
                self.assertAlmostEqual(abs(margin[0].matrix[3]), page_font, delta=MATRIX_TOLERANCE)
                self.assertAlmostEqual(body[0].origin[0], 20.0, delta=MATRIX_TOLERANCE)

    def test_signed_oblique_angles_and_synthesis_permissions_reach_pdf_matrices(self):
        face = (ROOT / "assets/fonts/LiberationSerif-Regular.ttf").as_uri()
        css = f"""@page{{size:300pt 400pt;margin:20pt}}
        @font-face{{font-family:Slant;src:url('{face}');font-style:normal}}
        p{{margin:0;font-family:Slant;font-size:40pt;line-height:60pt}}"""
        cases = [(0, True), (25, True), (-25, True), (25, False), (-25, False)]
        body = "".join(
            f"<p style='font-style:oblique {angle}deg;font-synthesis:"
            f"{'style' if enabled else 'none'}'>H</p>"
            for angle, enabled in cases
        )
        painted = self.render(css, body).glyphs
        self.assertEqual([glyph.text for glyph in painted], ["H"] * len(cases))
        for glyph, (angle, enabled) in zip(painted, cases):
            expected = math.tan(math.radians(angle)) if enabled else 0.0
            self.assertAlmostEqual(
                glyph.matrix[2] / glyph.matrix[3], expected,
                delta=MATRIX_TOLERANCE,
                msg=f"serialized shear for oblique {angle}deg, synthesis={enabled}",
            )

    def test_variable_instances_embed_the_independently_measured_outlines(self):
        reference = json.loads((FIXTURES / "RobotoFlex-metrics.json").read_text())
        face = (FIXTURES / "RobotoFlex.ttf").as_uri()
        css = f"""@page{{size:300pt 600pt;margin:20pt}}
        @font-face{{font-family:Variable;src:url('{face}')}}
        p{{margin:0;font-family:Variable;font-size:20pt;line-height:30pt}}"""
        body = ""
        for sample in reference["samples"]:
            axes = sample["location"]
            body += (
                f"<p style='font-weight:{axes['wght']};font-width:{axes['wdth']}%;"
                f"font-style:oblique {-axes['slnt']}deg'>H</p>"
            )
        interpreted = self.render(css, body)
        self.assertEqual(len(interpreted.glyphs), len(reference["samples"]))
        for glyph, sample in zip(interpreted.glyphs, reference["samples"]):
            with self.subTest(location=sample["location"]):
                self.assertEqual(glyph.text, "H")
                upem, advance, bounds = interpreted.embedded_glyph(glyph.font, "H")
                self.assertEqual(upem, reference["units_per_em"])
                self.assertAlmostEqual(
                    advance, sample["advance"], delta=FONT_UNIT_TOLERANCE
                )
                for actual, expected in zip(bounds, sample["bounds"]):
                    self.assertAlmostEqual(actual, expected, delta=FONT_UNIT_TOLERANCE)
                self.assertAlmostEqual(
                    glyph.matrix[2] / glyph.matrix[3], 0.0, delta=MATRIX_TOLERANCE,
                    msg="native slanted outlines must not receive an additional synthetic shear",
                )

    def test_italic_and_slant_axes_select_the_independently_shaped_glyphs(self):
        reference = json.loads((FIXTURES / "FontStyleTest-metrics.json").read_text())
        for face in reference["faces"]:
            with self.subTest(face=face["file"]):
                source = (FIXTURES / face["file"]).as_uri()
                css = f"""@page{{size:300pt 400pt;margin:20pt}}
                @font-face{{font-family:Axes;src:url('{source}')}}
                p{{margin:0;font-family:Axes;font-size:30pt;line-height:60pt}}"""
                body = "".join(
                    f"<p style='font-style:{sample['style']}'>{sample['character']}</p>"
                    for sample in face["samples"]
                )
                interpreted = self.render(css, body)
                self.assertEqual(len(interpreted.glyphs), len(face["samples"]))
                for glyph, sample in zip(interpreted.glyphs, face["samples"]):
                    with self.subTest(style=sample["style"]):
                        self.assertEqual(glyph.text, sample["character"])
                        upem, advance, bounds = interpreted.embedded_glyph(
                            glyph.font, sample["character"]
                        )
                        self.assertEqual(upem, face["units_per_em"])
                        self.assertAlmostEqual(
                            advance, sample["advance"], delta=FONT_UNIT_TOLERANCE
                        )
                        for actual, expected in zip(bounds, sample["bounds"]):
                            self.assertAlmostEqual(
                                actual, expected, delta=FONT_UNIT_TOLERANCE,
                                msg=f"native {sample['style']} glyph {sample['glyph']} outline",
                            )
                        self.assertAlmostEqual(
                            glyph.matrix[2] / glyph.matrix[3], 0.0,
                            delta=MATRIX_TOLERANCE,
                            msg="native italic and slant axes must not be replaced by synthesis",
                        )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
