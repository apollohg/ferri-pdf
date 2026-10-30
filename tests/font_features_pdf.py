"""Check CSS font feature, variation and palette effects in the serialized PDF."""
import argparse
import json
import math
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image

from font_geometry import InstanceInterpreter
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tests/fixtures/fonts"
RENDERER = ROOT / "target/debug/ferri-pdf"
SIZE_PT = 20
POSITION_TOLERANCE_PT = 0.01
FONT_UNIT_TOLERANCE = 1.0
PDF_POINTS_PER_INCH = 72
CSS_PIXELS_PER_INCH = 96
RASTER_DPI = 144
COLOUR_TOLERANCE = 2
FACES = """@font-face{{font-family:Serif4;src:url('{fixtures}/SourceSerif4-Regular.ttf')}}
@font-face{{font-family:Flex;src:url('{fixtures}/RobotoFlex.ttf')}}
@font-face{{font-family:Palette;src:url('{fixtures}/PaletteTest.ttf')}}
@font-face{{font-family:Presentation;src:url('{fixtures}/PresentationTest.ttf')}}
@page{{size:400pt 400pt;margin:20pt}}
p{{margin:0;font-size:{size}pt;line-height:40pt}}"""


def reference(name):
    return json.loads((FIXTURES / name).read_text())


class FontFeaturesPdf(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-font-features-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)

    def render(self, rules, body):
        source, target = self.output / "input.html", self.output / "output.pdf"
        faces = FACES.format(fixtures=FIXTURES.as_uri(), size=SIZE_PT)
        source.write_text(f"<!doctype html><style>{faces}{rules}</style>{body}")
        rendered = subprocess.run([str(RENDERER), str(source), str(target)],
                                  capture_output=True, text=True, timeout=60)
        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        document = pdf_content.Document(target)
        interpreter = InstanceInterpreter(document)
        for index, page in enumerate(document.pages(), start=1):
            interpreter.run_page(index, page)
        return interpreter

    def raster(self):
        prefix = self.output / "raster"
        subprocess.run(["pdftoppm", "-singlefile", "-r", str(RASTER_DPI), "-png",
                        str(self.output / "output.pdf"), str(prefix)],
                       check=True, capture_output=True, timeout=60)
        with Image.open(prefix.with_suffix(".png")) as raster:
            return raster.convert("RGB")

    def lines(self, glyphs):
        rows = {}
        for glyph in glyphs:
            rows.setdefault(round(glyph.origin[1], 3), []).append(glyph)
        return [rows[y] for y in sorted(rows, reverse=True)]

    def assert_advances(self, line, sample):
        """Origins of consecutive glyphs are the independently shaped advances apart."""
        shaped = reference("FontFeatures-reference.json")
        record = shaped["samples"][sample]
        upem = shaped["fonts"][record["font"]]["units_per_em"]
        expected = [glyph["ax"] / upem * SIZE_PT for glyph in record["glyphs"]]
        self.assertEqual(len(line), len(expected), sample)
        for index, (left, right) in enumerate(zip(line, line[1:])):
            self.assertAlmostEqual(right.origin[0] - left.origin[0], expected[index],
                                   delta=POSITION_TOLERANCE_PT, msg=f"{sample} glyph {index}")

    def test_figure_spacing_and_kerning_reach_glyph_positions(self):
        cases = [
            ("font-family:Serif4;font-variant-numeric:tabular-nums", "1011", "tabular_figures"),
            ("font-family:Serif4;font-variant-numeric:proportional-nums", "1011", "proportional_figures"),
            ("font-family:'Liberation Serif'", "AVA", "kerned"),
            ("font-family:'Liberation Serif';font-kerning:none", "AVA", "unkerned"),
        ]
        body = "".join(f"<p style=\"{style}\">{text}</p>" for style, text, _ in cases)
        lines = self.lines(self.render("", body).glyphs)
        self.assertEqual(len(lines), len(cases))
        for line, (style, text, sample) in zip(lines, cases):
            with self.subTest(style=style):
                self.assertEqual("".join(glyph.text for glyph in line), text)
                self.assert_advances(line, sample)

    def test_adjusted_and_synthesized_sizes_reach_text_matrices(self):
        metrics = reference("FontUnit-metrics.json")["faces"]["InitialSerif"]
        x_height = metrics["x_height"] / metrics["units_per_em"]
        body = ("<p style=\"font-family:'Liberation Serif';font-size-adjust:0.5\">x</p>"
                "<p style=\"font-family:'Liberation Serif'\">4<span style=\"font-variant-position:super\">4x</span></p>")
        # Glyphs paint in document order; the raised run does not share the base line's origin.
        adjusted, base, superscript, letter = self.render("", body).glyphs
        self.assertAlmostEqual(adjusted.matrix[0], SIZE_PT * 0.5 / x_height, delta=POSITION_TOLERANCE_PT)
        self.assertAlmostEqual(base.matrix[0], SIZE_PT, delta=POSITION_TOLERANCE_PT)
        for glyph in (superscript, letter):
            self.assertLess(glyph.matrix[0], SIZE_PT, "synthesized superscripts are reduced")
            self.assertGreater(glyph.origin[1], base.origin[1], "synthesized superscripts are raised")
        self.assertAlmostEqual(superscript.origin[1], letter.origin[1], delta=POSITION_TOLERANCE_PT,
                               msg="the whole run shares one raised baseline")

    def shaped(self, rules, body, samples):
        """The painted glyphs of `samples`, in document order, each with its outline."""
        interpreted = self.render(rules, body)
        glyphs = [(glyph, interpreted.embedded_outline_code(identity, code))
                  for glyph, (identity, code) in zip(interpreted.glyphs, interpreted.painted_codes)]
        runs = []
        for sample in samples:
            run, text = [], ""
            while text != sample:
                self.assertTrue(glyphs, f"no glyphs left for {sample!r}; painted {text!r}")
                glyph = glyphs.pop(0)
                run.append(glyph)
                text += glyph[0].text
                self.assertTrue(sample.startswith(text), f"{text!r} is not a prefix of {sample!r}")
            runs.append(run)
        self.assertFalse(glyphs, "every painted glyph belongs to a sample")
        return runs

    def assert_svg_matches_document(self, cases, rules=""):
        """SVG 2 §11.10: each case's SVG text shapes as its paragraph does, with the same glyph
        outlines at the same sizes, advances and baseline offsets."""
        # User units are CSS pixels, so SVG font sizes compute as the paragraphs' do.
        line_pitch = 40
        height = line_pitch * len(cases)
        pixels = PDF_POINTS_PER_INCH / CSS_PIXELS_PER_INCH
        samples = [plain + rest for plain, _, rest in map(self.parts, cases)]
        paragraphs = "".join(
            f"<p style=\"{style}\">{plain}<span style=\"{inner}\">{rest}</span></p>"
            for (style, _), (plain, inner, rest) in zip(cases, map(self.parts, cases)))
        texts = "".join(
            f"<text y=\"{(index + 1) * line_pitch / pixels}\" style=\"{style};font-size:{SIZE_PT}pt\">"
            f"{plain}<tspan style=\"{inner}\">{rest}</tspan></text>"
            for index, ((style, _), (plain, inner, rest)) in enumerate(zip(cases, map(self.parts, cases))))
        document = self.shaped(rules, paragraphs, samples)
        svg = self.shaped(rules + "svg{display:block}",
                          f"<svg width=\"300pt\" height=\"{height}pt\" "
                          f"viewBox=\"0 0 {300 / pixels} {height / pixels}\">"
                          f"{texts}</svg>", samples)
        for (style, _), expected, actual in zip(cases, document, svg):
            with self.subTest(style=style):
                self.assertEqual([glyph.text for glyph, _ in actual],
                                 [glyph.text for glyph, _ in expected], "glyph clusters")
                self.assertEqual([outline for _, outline in actual],
                                 [outline for _, outline in expected], "glyph outlines")
                self.assertEqual([round(glyph.matrix[0], 3) for glyph, _ in actual],
                                 [round(glyph.matrix[0], 3) for glyph, _ in expected], "glyph sizes")
                for axis, label in ((0, "advances"), (1, "baseline offsets")):
                    offsets = [[glyph.origin[axis] - line[0][0].origin[axis] for glyph, _ in line]
                               for line in (expected, actual)]
                    for wanted, got in zip(*offsets):
                        self.assertAlmostEqual(got, wanted, delta=POSITION_TOLERANCE_PT,
                                               msg=f"SVG {label} {offsets[1]} match {offsets[0]}")

    @staticmethod
    def parts(case):
        """A sample's plain text, the inner span's style and its text: `plain|style|text`."""
        _, sample = case
        plain, inner, rest = (sample.split("|") + ["", ""])[:3]
        return plain, inner, rest

    def test_svg_text_renders_caps_as_document_text_does(self):
        """CSS Fonts 4 §6.6: a face's own capitals, or capitals synthesized from uppercase forms
        scaled down, for SVG text exactly as for the document's lines."""
        values = ("small-caps", "all-small-caps", "petite-caps", "all-petite-caps", "unicase",
                  "titling-caps")
        for family in ("Serif4", "'Liberation Serif'"):
            self.assert_svg_matches_document(
                [(f"font-family:{family};font-variant-caps:{caps}", "Ab") for caps in values])
        self.assert_svg_matches_document(
            [("font-family:'Liberation Serif';font-variant-caps:all-small-caps", "Ab"),
             ("font-family:'Liberation Serif';font-variant-caps:all-small-caps;"
              "font-synthesis-small-caps:none", "Ab")])
        (synthesized,), = self.shaped(
            "svg{display:block}",
            f"<svg width=\"300pt\" height=\"40pt\" viewBox=\"0 0 300 40\"><text y=\"30\" "
            f"style=\"font:{SIZE_PT}px 'Liberation Serif';font-variant-caps:all-small-caps\">A</text>"
            "</svg>", ["A"])
        self.assertAlmostEqual(synthesized[0].matrix[0], SIZE_PT * 0.8, delta=POSITION_TOLERANCE_PT,
                               msg="Liberation Serif has no c2sc, so the capital is synthesized")

    def test_svg_text_selects_features_as_document_text_does(self):
        """CSS Fonts 4 §6–§7 and CSS Fonts 5 §2.5 on SVG text: numeric, ligature, alternate,
        position and language features, kerning and size adjustment, in the same precedence."""
        rules = "@font-feature-values Serif4{@styleset{alt:1 2}}"
        self.assert_svg_matches_document([
            ("font-family:Serif4;font-variant-numeric:tabular-nums", "1011"),
            ("font-family:Serif4;font-variant-numeric:oldstyle-nums slashed-zero", "1029"),
            ("font-family:Serif4;font-variant-numeric:diagonal-fractions", "1/2"),
            ("font-family:Serif4;font-feature-settings:'liga' 0", "fifl"),
            ("font-family:Serif4;font-variant-ligatures:no-common-ligatures", "fifl"),
            ("font-family:Serif4;font-variant-alternates:styleset(alt)", "agyQ"),
            ("font-family:Serif4", "x|font-variant-position:super|2x"),
            ("font-family:'Liberation Serif'", "x|font-variant-position:sub|2x"),
            ("font-family:Serif4;font-variant-caps:small-caps;font-language-override:'TRK'", "if"),
            ("font-family:'Liberation Serif';font-size-adjust:0.5", "x"),
            ("font-family:'Liberation Serif';font-kerning:none", "AVA"),
            ("font-family:'Liberation Serif'", "AVA"),
        ], rules)

    def test_svg_text_cases_characters_where_their_sources_are_positioned(self):
        """CSS Text 3 §2.1 on SVG text: casing is language-sensitive and may expand characters,
        while SVG 2 §11.5 positions the characters as they were before the transform."""
        pixels = PDF_POINTS_PER_INCH / CSS_PIXELS_PER_INCH
        texts = ('<text x="10 110 210" y="40" style="text-transform:uppercase">ßab</text>'
                 '<text x="10" y="80" lang="tr" style="text-transform:uppercase">iı</text>'
                 '<text x="10" y="120" style="text-transform:capitalize">ab <tspan>cd</tspan></text>'
                 '<text x="10" y="160" style="text-transform:lowercase">AB<tspan '
                 'style="text-transform:none">C</tspan></text>')
        lines = self.lines(self.render(
            "svg{display:block}",
            f"<svg width=\"300pt\" height=\"150pt\" viewBox=\"0 0 400 200\" "
            f"style=\"font:20px 'Liberation Serif'\">{texts}</svg>").glyphs)
        self.assertEqual(["".join(glyph.text for glyph in line) for line in lines],
                         ["SSAB", "İI", "Ab Cd", "abC"],
                         "uppercase expands ß, Turkish dots i, capitalize starts each word and "
                         "a span's own value replaces the inherited one")
        sharp, second, capital_a, capital_b = lines[0]
        for glyph, position in ((capital_a, 110), (capital_b, 210)):
            self.assertAlmostEqual(glyph.origin[0] - sharp.origin[0], (position - 10) * pixels,
                                   delta=POSITION_TOLERANCE_PT,
                                   msg=f"{glyph.text} stays at its source character's x")
        self.assertTrue(sharp.origin[0] < second.origin[0] < capital_a.origin[0],
                        "the second S of ß follows the first instead of taking a position")

    def test_svg_text_preserves_white_space_as_the_cascade_computes_it(self):
        """SVG 2 §11.6.1 and §11.10.3 with CSS Text 3 §4: preserved segment breaks start line
        boxes stacked by line-height, preserved tabs advance to tab stops, `pre-line` collapses
        spaces, and `xml:space="preserve"` maps to `white-space-collapse: preserve-spaces`."""
        pixels = PDF_POINTS_PER_INCH / CSS_PIXELS_PER_INCH
        size, line_height, tab_size = 20, 30, 4
        space = 0.6 * size
        texts = (f'<text x="10" y="30" style="white-space:pre;tab-size:{tab_size}">a\tb\n  c\n\nd'
                 '<tspan style="tab-size:1">\te</tspan></text>'
                 '<text x="300" y="30" style="white-space:pre-line">x   y \n   z</text>'
                 '<text x="10" y="180" xml:space="preserve">p  q</text>'
                 '<text x="300" y="180" xml:space="preserve" style="white-space:normal">p  q</text>')
        glyphs = self.render(
            f"svg{{display:block}}text{{font:{size}px 'Liberation Mono';line-height:{line_height}px}}",
            f"<svg width=\"300pt\" height=\"150pt\" viewBox=\"0 0 400 200\">{texts}</svg>").glyphs
        origin = {}
        for glyph in glyphs:
            origin.setdefault(glyph.text, []).append(
                (round(glyph.origin[0] / pixels, 2), round(-glyph.origin[1] / pixels, 2)))
        (a,), (b,), (c,), (d,) = origin["a"], origin["b"], origin["c"], origin["d"]
        self.assertEqual(b[0] - a[0], tab_size * space, "the tab reaches the first stop")
        (e,) = origin["e"]
        self.assertEqual(e[0] - d[0], 2 * space,
                         "a tab takes its own tab-size in the text's spaces (CSS Text 3 §4.2), "
                         "skipping a stop nearer than half a '0'")
        self.assertEqual((c[0] - a[0], c[1] - a[1]), (2 * space, line_height),
                         "the next line starts at the text's x, its leading spaces kept")
        self.assertEqual((d[0] - a[0], d[1] - a[1]), (0, 3 * line_height),
                         "an empty line box takes the strut's line-height")
        (x,), (y,), (z,) = origin["x"], origin["y"], origin["z"]
        self.assertEqual((y[0] - x[0], z[0] - x[0], z[1] - x[1]), (2 * space, 0, line_height),
                         "pre-line collapses spaces, including those beside the break")
        p, q = origin["p"], origin["q"]
        self.assertEqual([q[0] - p[0] for p, q in zip(p, q)], [3 * space, 2 * space],
                         "xml:space keeps both spaces unless white-space overrides it")

    def test_svg_text_shifts_aligns_and_decorates_as_the_cascade_computes(self):
        """CSS Inline 3 §4.2 and SVG 2 §11.11 from the document's cascade: percentage shifts
        refer to the line height, presentation attributes take part, and each declared line is
        painted in the fill of the element declaring it."""
        pixels = PDF_POINTS_PER_INCH / CSS_PIXELS_PER_INCH
        line_height = 40
        texts = ('<text x="10" y="50">a<tspan style="baseline-shift:50%">b</tspan>'
                 '<tspan baseline-shift="super">c</tspan>'
                 '<tspan style="alignment-baseline:central;font-size:40px">e</tspan></text>'
                 '<text x="10" y="120" style="text-decoration:underline;fill:rgb(0,0,255)">u'
                 '<tspan style="fill:rgb(255,0,0);text-decoration-line:line-through">v</tspan>'
                 '</text>')
        glyphs = self.render(
            f"svg{{display:block}}text{{font:20px 'Liberation Serif';line-height:{line_height}px}}",
            f"<svg width=\"300pt\" height=\"150pt\" viewBox=\"0 0 400 200\">{texts}</svg>").glyphs
        rise = {glyph.text: glyph.origin[1] / pixels for glyph in glyphs}
        self.assertAlmostEqual(rise["b"] - rise["a"], line_height / 2, delta=POSITION_TOLERANCE_PT,
                               msg="50% raises by half the used line height")
        self.assertGreater(rise["c"], rise["a"], "the presentation attribute's super raises")
        metrics = reference("FontUnit-metrics.json")["faces"]["InitialSerif"]
        central = (metrics["ascender"] + metrics["descender"]) / 2 / metrics["units_per_em"]
        self.assertAlmostEqual(rise["a"] - rise["e"], central * (40 - 20),
                               delta=POSITION_TOLERANCE_PT,
                               msg="the tspan's central baseline lies on its parent's")
        raster = self.raster()
        scale = RASTER_DPI / PDF_POINTS_PER_INCH
        u, v = (next(glyph for glyph in glyphs if glyph.text == text) for text in "uv")

        def dominant(x0, x1, below, above):
            found = {}
            for x in range(int(x0 * scale), int(x1 * scale)):
                for y in range(int(below * scale), int(above * scale)):
                    pixel = raster.getpixel((x, raster.height - y))
                    if pixel != (255, 255, 255):
                        found[pixel] = found.get(pixel, 0) + 1
            return max(found, key=found.get) if found else None

        self.assertEqual(dominant(u.origin[0], v.origin[0] + 5, u.origin[1] - 3, u.origin[1]),
                         (0, 0, 255), "the text's underline spans both letters in its fill")
        self.assertEqual(dominant(v.origin[0] + 1, v.origin[0] + 6, v.origin[1] + 3,
                                  v.origin[1] + 7),
                         (255, 0, 0), "the tspan's line-through is in its own fill")

    def test_svg_text_vertical_align_shifts_as_the_documents_line_boxes_do(self):
        """CSS 2.1 §10.8.1 `vertical-align` on SVG text content (CSS Inline 3 §4.2): each value
        raises a smaller, taller-lined tspan above the text's baseline as it raises the same
        inline box above a paragraph line's, `top` and `bottom` aligning with line box edges."""
        rules = ("svg{display:block}text,p{font:20px 'Liberation Serif';line-height:30px}"
                 "span,tspan{font-size:12px;line-height:40px}")

        def rise(document):
            origins = {glyph.text: glyph.origin[1] for glyph in document.glyphs}
            return origins["b"] - origins["a"]

        for value in ("super", "sub", "6pt", "25%", "text-top", "text-bottom", "middle", "top",
                      "bottom"):
            with self.subTest(value=value):
                expected = rise(self.render(
                    rules, f'<p>a<span style="vertical-align:{value}">b</span></p>'))
                actual = rise(self.render(
                    rules, f'<svg width="300pt" height="100pt"><text x="10" y="50">a<tspan '
                           f'style="vertical-align:{value}">b</tspan></text></svg>'))
                self.assertAlmostEqual(actual, expected, delta=POSITION_TOLERANCE_PT,
                                       msg=f"vertical-align:{value} raises the SVG tspan by "
                                           f"{actual:.3f}pt, the paragraph's span by "
                                           f"{expected:.3f}pt")

    def test_svg_text_wraps_in_its_inline_size_as_paragraphs_wrap(self):
        """SVG 2 §11.4, §11.7: a text element's `inline-size` makes a wrapping area in which
        its text breaks into lines where, and as far apart as, a paragraph that wide breaks
        and stacks them, each line anchored by `text-anchor`; hyphenation, `text-indent` and
        `text-overflow` apply as to the paragraph."""
        width, left, centre = 150, 30, 105
        words = "Wrapped words fill each line of the area before breaking, hyphenating extraordinarily"
        rules = ("svg,p{display:block;margin:0}text,p{font:20px 'Liberation Serif';"
                 "line-height:30px}")
        cases = [
            ("", "", words),
            ("text-indent:25px", "text-indent:25px", words),
            ("hyphens:auto", "hyphens:auto", words),
            ("text-anchor:middle", "text-align:center", words),
            ("white-space:nowrap", "white-space:nowrap;overflow:hidden", words),
            ("white-space:nowrap;text-overflow:ellipsis",
             "white-space:nowrap;overflow:hidden;text-overflow:ellipsis", words),
            # CSS Text 4 §9.2: hanging punctuation is not measured for fit or alignment.
            ("hanging-punctuation:first", "hanging-punctuation:first", f"\u201c{words}"),
            ("text-anchor:end;hanging-punctuation:last", "text-align:right;hanging-punctuation:last",
             f"{words}\u201d"),
            ("hanging-punctuation:force-end", "hanging-punctuation:force-end",
             "Commas, stops. Lists, then, hang, past, each, edge, of, the, area."),
            ("hanging-punctuation:allow-end", "hanging-punctuation:allow-end",
             "Commas, stops. Lists, then, hang, past, each, edge, of, the, area."),
        ]
        for svg_style, html_style, text in cases:
            with self.subTest(style=svg_style):
                x = centre if "middle" in svg_style else left + width if "end" in svg_style \
                    and "text-anchor" in svg_style else left
                expected = self.render(
                    rules, f'<p lang="en" style="position:absolute;left:{left}px;top:0;'
                           f'width:{width}px;{html_style}">{text}</p>').glyphs
                actual = self.render(
                    rules, f'<svg width="300pt" height="300pt" style="position:absolute;left:0;'
                           f'top:0" lang="en"><text x="{x}" y="30" style="inline-size:{width}px;'
                           f'{svg_style}">{text}</text></svg>').glyphs
                lines = lambda glyphs: ["".join(glyph.text for glyph in line)
                                        for line in self.lines(glyphs)]
                self.assertEqual(lines(actual), lines(expected),
                                 "the SVG text breaks into the paragraph's lines")
                for got, want in zip(self.lines(actual), self.lines(expected)):
                    self.assertAlmostEqual(
                        got[0].origin[0], want[0].origin[0], delta=POSITION_TOLERANCE_PT,
                        msg=f"line {''.join(g.text for g in want)!r} starts where the "
                            f"paragraph's does")
                baselines = lambda glyphs: [line[0].origin[1] for line in self.lines(glyphs)]
                steps = lambda values: [b - a for a, b in zip(values, values[1:])]
                self.assertEqual([round(step, 2) for step in steps(baselines(actual))],
                                 [round(step, 2) for step in steps(baselines(expected))],
                                 "lines stack as the paragraph's line boxes do")
                if svg_style == "white-space:nowrap":
                    # SVG 2 §11.10.2.10: the initial `clip` hides what overflows the area.
                    raster = self.raster()
                    scale = RASTER_DPI / PDF_POINTS_PER_INCH
                    pixels = PDF_POINTS_PER_INCH / CSS_PIXELS_PER_INCH
                    origin = actual[0].origin
                    edge = origin[0] + width * pixels
                    rows = range(int((raster.height / scale - origin[1] - SIZE_PT) * scale),
                                 int((raster.height / scale - origin[1] + SIZE_PT / 4) * scale))

                    def ink(columns):
                        return [x for x in columns for y in rows
                                if max(raster.getpixel((x, y))) < 128]

                    self.assertTrue(ink(range(int(origin[0] * scale), int(edge * scale))),
                                    "the line's kept text is inked")
                    self.assertEqual(ink(range(int(edge * scale) + COLOUR_TOLERANCE,
                                               raster.width)), [],
                                     "no ink beyond the wrapping area's right edge")

    def test_pre_formatted_svg_text_anchors_without_its_hanging_punctuation(self):
        """CSS Text 4 §9.2 on SVG text without a wrapping area: punctuation that hangs is not
        measured for `text-anchor` alignment, so a closing quote ending the last line hangs
        past an end anchor and an opening quote starting the first line before a start one."""
        anchor = 200
        texts = (f'<text x="{anchor}" y="30" style="text-anchor:end">ab\u201d</text>'
                 f'<text x="{anchor}" y="60" style="text-anchor:end;hanging-punctuation:last">'
                 'ab\u201d</text>'
                 f'<text x="{anchor}" y="90" style="hanging-punctuation:first">\u201cab</text>'
                 f'<text x="{anchor}" y="120">ab</text>')
        plain, hanging, opening, start = self.lines(self.render(
            "svg{display:block}",
            f"<svg width=\"300pt\" height=\"100pt\" viewBox=\"0 0 400 133\" "
            f"style=\"font:20px 'Liberation Serif'\">{texts}</svg>").glyphs)
        origin = start[0].origin[0]
        quote = origin - plain[-1].origin[0]
        self.assertAlmostEqual(hanging[-1].origin[0], origin, delta=POSITION_TOLERANCE_PT,
                               msg="the hanging quote starts at the end anchor")
        self.assertAlmostEqual(hanging[0].origin[0] - plain[0].origin[0], quote,
                               delta=POSITION_TOLERANCE_PT,
                               msg="the line moves right by the hanging advance")
        self.assertLess(opening[0].origin[0], origin, "the opening quote hangs before the anchor")
        self.assertAlmostEqual(opening[1].origin[0], origin, delta=POSITION_TOLERANCE_PT,
                               msg="the letters start at the start anchor")

    def test_optical_size_and_presentation_select_the_embedded_glyphs(self):
        optical = reference("RobotoFlex-optical.json")
        advances = {json.dumps(sample["location"], sort_keys=True): sample["advance"]
                    for sample in optical["samples"]}
        presentation = {sample["text"]: sample for sample in reference("PresentationTest-source.json")["samples"]}
        body = ("<p style=\"font-family:Flex\">H</p>"
                "<p style=\"font-family:Flex;font-optical-sizing:none\">H</p>"
                "<p style=\"font-family:Presentation;font-variant-emoji:emoji\">❤</p>")
        interpreted = self.render("", body)
        automatic, disabled, emoji = self.lines(interpreted.glyphs)
        for (glyph,), location in ((automatic, {"opsz": SIZE_PT}), (disabled, {"opsz": 14})):
            upem, advance, _ = interpreted.embedded_glyph(glyph.font, "H")
            self.assertEqual(upem, optical["units_per_em"])
            self.assertAlmostEqual(advance, advances[json.dumps(location, sort_keys=True)],
                                   delta=FONT_UNIT_TOLERANCE, msg=f"embedded instance at {location}")
        (heart,) = emoji
        self.assertEqual(heart.text, "❤")
        data, glyph_id = interpreted.embedded_glyph_data(heart.font, "❤")
        self.assertTrue(data)
        _, advance, _ = interpreted.embedded_glyph(heart.font, "❤")
        self.assertEqual(advance, presentation["❤️"]["advance"],
                         "the emoji presentation glyph is embedded for the original character")

    def test_palettes_paint_the_selected_colours(self):
        fixture = reference("PaletteTest-source.json")
        palettes = [[tuple(colour[:3]) for colour in palette] for palette in fixture["palettes"]]
        foreground = (0, 0, 255)
        rules = ("@font-palette-values --custom{font-family:Palette;base-palette:dark;"
                 "override-colors:1 rgb(255,0,255)}")
        cases = [
            ("normal", palettes[0]),
            ("light", palettes[1]),
            ("dark", palettes[2]),
            ("--custom", [palettes[2][0], (255, 0, 255)]),
        ]
        body = "".join(f"<p style=\"font-family:Palette;color:rgb{foreground};font-palette:{value}\">AHI</p>"
                       for value, _ in cases)
        interpreted = self.render(rules, body)
        raster = self.raster()
        for line, (value, colours) in zip(self.lines(interpreted.glyphs), cases):
            for glyph, expected in zip(line, [*colours, foreground]):
                with self.subTest(palette=value, glyph=glyph.text):
                    self.assert_glyph_colour(raster, glyph.origin, expected)

    def assert_glyph_colour(self, raster, origin, expected):
        """The dominant colour of a box above the baseline at `origin`, which the stems of a
        glyph drawn there cross, is `expected`."""
        scale = RASTER_DPI / PDF_POINTS_PER_INCH
        x, y = origin
        painted = {}
        for dx in range(int(2 * scale), int(10 * scale)):
            for dy in range(int(2 * scale), int(12 * scale)):
                pixel = raster.getpixel((int(x * scale) + dx, raster.height - int(y * scale) - dy))
                if pixel != (255, 255, 255):
                    painted[pixel] = painted.get(pixel, 0) + 1
        self.assertTrue(painted, f"the glyph at {origin} paints")
        dominant = max(painted, key=painted.get)
        for actual, wanted in zip(dominant, expected):
            self.assertLessEqual(abs(actual - wanted), COLOUR_TOLERANCE, f"{dominant} != {expected}")

    def test_svg_text_paints_the_selected_palettes_in_both_renderers(self):
        """CSS Fonts 4 §9 on SVG text: `font-palette` and `@font-palette-values` colour COLR
        glyphs, whose foreground entry takes the fill, whether the text is drawn as vector text or
        rasterized beneath a filter."""
        fixture = reference("PaletteTest-source.json")
        palettes = [[tuple(colour[:3]) for colour in palette] for palette in fixture["palettes"]]
        foreground = (0, 0, 255)
        rules = ("@font-palette-values --custom{font-family:Palette;base-palette:dark;"
                 "override-colors:1 rgb(255,0,255)}svg{display:block}")
        cases = [
            ("normal", palettes[0]),
            ("light", palettes[1]),
            ("dark", palettes[2]),
            ("--custom", [palettes[2][0], (255, 0, 255)]),
        ]
        pixels = PDF_POINTS_PER_INCH / CSS_PIXELS_PER_INCH
        # The filter's identity offset rasterizes the second column, which paints no glyphs;
        # the doubled size keeps a pixel of each stem unblended at the raster's resolution.
        raster_offset, line_pitch, width = 180, 60, 360
        height = line_pitch * (len(cases) + 1)
        texts = ""
        for index, (value, _) in enumerate(cases):
            y = (index + 1) * line_pitch / pixels
            texts += (f"<text y=\"{y}\" style=\"font-palette:{value}\">AHI</text>"
                      f"<text x=\"{raster_offset / pixels}\" y=\"{y}\" filter=\"url(#raster)\" "
                      f"style=\"font-palette:{value}\">AHI</text>")
        body = (f"<svg width=\"{width}pt\" height=\"{height}pt\" "
                f"viewBox=\"0 0 {width / pixels} {height / pixels}\" "
                f"style=\"font:{2 * SIZE_PT}pt Palette;fill:rgb{foreground}\">"
                f"<filter id=\"raster\"><feOffset dx=\"0\"/></filter>{texts}</svg>")
        interpreted = self.render(rules, body)
        raster = self.raster()
        lines = self.lines(interpreted.glyphs)
        self.assertEqual(len(lines), len(cases), "only the unfiltered column paints glyphs")
        for line, (value, colours) in zip(lines, cases):
            self.assertEqual("".join(glyph.text for glyph in line), "AHI")
            for glyph, expected in zip(line, [*colours, foreground]):
                for renderer, dx in (("vector", 0), ("raster", raster_offset)):
                    with self.subTest(palette=value, glyph=glyph.text, renderer=renderer):
                        self.assert_glyph_colour(
                            raster, (glyph.origin[0] + dx, glyph.origin[1]), expected)

    def test_svg_text_presents_emoji_as_document_text_does(self):
        """CSS Fonts 4 §9.3 on SVG text: `font-variant-emoji` selects the presentation
        sequence of the first face having it, a later family's where the first lacks it, and is
        ignored where no face has it."""
        self.assert_svg_matches_document([
            ("font-family:Presentation;font-variant-emoji:emoji", "❤"),
            ("font-family:Presentation;font-variant-emoji:text", "☕"),
            ("font-family:Presentation;font-variant-emoji:unicode", "❤"),
            ("font-family:'DejaVu Sans',Presentation;font-variant-emoji:emoji", "❤"),
            ("font-family:'DejaVu Sans';font-variant-emoji:emoji", "❤"),
        ])

    def test_svg_text_synthesizes_bold_and_oblique_as_document_text_does(self):
        """CSS Fonts 4 §5.2 and §5.4 on SVG text: a face lacking the requested weight or
        slant is emboldened by a second, offset copy of each glyph and slanted by a shear, both
        as the document's text draws them, unless `font-synthesis` forbids it."""
        face = (ROOT / "assets/fonts/LiberationSerif-Regular.ttf").as_uri()
        rules = f"@font-face{{font-family:Regular;src:url('{face}')}}svg{{display:block}}"
        cases = [
            ("font-weight:bold", True, 0.0),
            ("font-style:oblique 20deg", False, math.tan(math.radians(20))),
            ("font-weight:bold;font-style:oblique -10deg", True, math.tan(math.radians(-10))),
            ("font-weight:bold;font-synthesis-weight:none", False, 0.0),
            ("font-style:oblique 20deg;font-synthesis-style:none", False, 0.0),
            ("font-style:italic", False, None),
        ]
        pixels = PDF_POINTS_PER_INCH / CSS_PIXELS_PER_INCH
        paragraphs = "".join(f"<p style=\"font-family:Regular;{style}\">Hb</p>" for style, _, _ in cases)
        texts = "".join(
            f"<text y=\"{(index + 1) * 40 / pixels}\" style=\"font:{SIZE_PT}pt Regular;{style}\">Hb</text>"
            for index, (style, _, _) in enumerate(cases))
        height = 40 * len(cases)
        documents = self.lines(self.render(rules, paragraphs).glyphs)
        svgs = self.lines(self.render(rules, f"<svg width=\"300pt\" height=\"{height}pt\" "
                                             f"viewBox=\"0 0 {300 / pixels} {height / pixels}\">"
                                             f"{texts}</svg>").glyphs)
        self.assertEqual((len(documents), len(svgs)), (len(cases), len(cases)))
        for (style, bold, shear), document, svg in zip(cases, documents, svgs):
            with self.subTest(style=style):
                document = sorted(document, key=lambda glyph: glyph.origin[0])
                svg = sorted(svg, key=lambda glyph: glyph.origin[0])
                copies = 2 if bold else 1
                self.assertEqual([glyph.text for glyph in svg],
                                 [character for character in "Hb" for _ in range(copies)],
                                 "synthetic bold draws each glyph twice")
                self.assertEqual([glyph.text for glyph in svg], [glyph.text for glyph in document])
                for actual, expected in zip(svg, document):
                    self.assertAlmostEqual(actual.matrix[2] / actual.matrix[3],
                                           expected.matrix[2] / expected.matrix[3],
                                           delta=POSITION_TOLERANCE_PT, msg="the document's shear")
                    self.assertAlmostEqual(actual.origin[0] - svg[0].origin[0],
                                           expected.origin[0] - document[0].origin[0],
                                           delta=POSITION_TOLERANCE_PT, msg="the document's copies")
                if shear is None:
                    self.assertGreater(svg[0].matrix[2] / svg[0].matrix[3], 0,
                                       "italic synthesizes a forward slant")
                else:
                    self.assertAlmostEqual(svg[0].matrix[2] / svg[0].matrix[3], shear,
                                           delta=POSITION_TOLERANCE_PT, msg="the oblique's shear")
                if bold:
                    self.assertAlmostEqual(svg[1].origin[0] - svg[0].origin[0], 0.02 * SIZE_PT,
                                           delta=POSITION_TOLERANCE_PT,
                                           msg="the bold copy is offset by 0.02em")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
