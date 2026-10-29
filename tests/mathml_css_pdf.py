"""Retained MathML children, CSS geometry and navigation in the produced PDF."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import unittest

from oracle import pdf_content
import mathml_pdf as math
import table_positioning_pdf as tables
from writing_anchors_pdf import colour_bounds

RENDERER = math.RENDERER
TOLERANCE_PT = math.TOLERANCE
TOKEN_WIDTH_PT = 35
LINE_HEIGHT_PT = 20
USED_WIDTH_PT = 80
USED_HEIGHT_PT = 40
FIXED_LEFT_PT = 100
FIXED_TOP_PT = 30
FIRST_LINE_SIZE_PT = 40
SECTION_SIZE_PT = 120
PIXEL_QUANTIZATION = 1
WRITING_MODES = ("vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr")
FOREIGN_LINK = "https://example.org/math-foreign"
FIXED_LINK = "https://example.org/math-fixed"


class MathmlCssPdf(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ferri-mathml-css-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.renderer = math.MathmlPdf()
        self.renderer.root = self.root
        math.RENDERER = RENDERER

    def render(self, body, css=""):
        try:
            return self.renderer.render(body, css)
        except subprocess.CalledProcessError as error:
            self.fail(f"MathML CSS fixture failed with exit {error.returncode}: "
                      f"{error.stderr.decode(errors='replace')}\nCSS: {css}\nHTML: {body}")

    def one(self, content, text):
        glyphs = [glyph for glyph in content["glyphs"] if glyph.text == text]
        self.assertEqual(len(glyphs), 1,
                         f"{text!r} must paint once; actual glyphs: {content['glyphs']}")
        return glyphs[0]

    def assert_same_baseline(self, first, second, message):
        self.assertAlmostEqual(first.origin[1], second.origin[1],
                               delta=TOLERANCE_PT, msg=message)

    def test_generated_fraction_child_changes_native_arity_and_paints_once(self):
        _, content = self.render(
            '<math><mfrac class="generated"><mn>1</mn><mn>2</mn></mfrac></math>',
            '.generated::before{content:"3"}')
        three, one, two = (self.one(content, text) for text in ("3", "1", "2"))
        self.assertLess(three.origin[0], one.origin[0], "generated content precedes the first real child")
        self.assertLess(one.origin[0], two.origin[0], "three child boxes use row recovery")
        self.assert_same_baseline(three, one, "generated content participates in the recovered row")
        self.assert_same_baseline(one, two, "an invalid three-child fraction cannot remain stacked")
        self.assertFalse(content["images"], "recovered math stays native selectable content")

    def test_generated_math_content_uses_the_document_counter_scope(self):
        _, content = self.render(
            '<math><mrow class="generated"><mn>2</mn></mrow></math><p class="after"></p>',
            'body{counter-reset:term 7}.generated::before{counter-increment:term;'
            'content:counter(term)}.after::before{content:counter(term)}')
        counters = [glyph for glyph in content["glyphs"] if glyph.text == "8"]
        self.assertEqual(len(counters), 2,
                         f"math increments the same counter later read by HTML: {content['glyphs']}")
        self.assertFalse(any(glyph.text == "7" for glyph in content["glyphs"]),
                         "native layout must not bypass the generated-content counter event")
        self.one(content, "2")

    def test_mtext_soft_wrap_uses_css_width_and_real_line_metrics(self):
        _, content = self.render(
            '<math><mtext class="wrapped"><span>AB CD EF</span></mtext><mn>Z</mn></math>',
            f'.wrapped{{width:{TOKEN_WIDTH_PT}pt;line-height:{LINE_HEIGHT_PT}pt}}'
            '.wrapped span{white-space:normal}')
        glyphs = {text: self.one(content, text) for text in "ABCDEFZ"}
        for first, second in (("A", "B"), ("C", "D"), ("E", "F")):
            self.assert_same_baseline(glyphs[first], glyphs[second], "each unbroken word stays on one line")
        for first, second in (("A", "C"), ("C", "E")):
            self.assertAlmostEqual(glyphs[first].origin[1] - glyphs[second].origin[1],
                                   LINE_HEIGHT_PT, delta=TOLERANCE_PT,
                                   msg="mtext wraps as CSS content at the authored line height")
            self.assertAlmostEqual(glyphs[first].origin[0], glyphs[second].origin[0],
                                   delta=TOLERANCE_PT, msg="wrapped lines keep their content start")
        self.assertAlmostEqual(glyphs["Z"].origin[0] - glyphs["A"].origin[0],
                               TOKEN_WIDTH_PT, delta=TOLERANCE_PT,
                               msg="the next math child follows the used CSS width, not the unwrapped text")

    def test_mtext_forced_break_uses_the_existing_inline_formatter(self):
        _, content = self.render(
            '<math><mtext class="broken">AB<br>CD</mtext></math>',
            f'.broken{{line-height:{LINE_HEIGHT_PT}pt}}')
        a, b, c, d = (self.one(content, text) for text in "ABCD")
        self.assert_same_baseline(a, b, "text before the forced break shares its line")
        self.assert_same_baseline(c, d, "text after the forced break shares its line")
        self.assertAlmostEqual(a.origin[1] - c.origin[1], LINE_HEIGHT_PT,
                               delta=TOLERANCE_PT, msg="a real br creates exactly the authored line advance")
        self.assertAlmostEqual(a.origin[0], c.origin[0], delta=TOLERANCE_PT)

    def test_foreign_inline_content_retains_its_live_link(self):
        output, content = self.render(
            '<math><mtext><span>AB<a href="' + FOREIGN_LINK + '">CD</a></span></mtext>'
            '<mn>Z</mn></math>', 'a{color:green;text-decoration:none}')
        a, b, c, d, z = (self.one(content, text) for text in "ABCDZ")
        for first, second in ((a, b), (b, c), (c, d), (d, z)):
            self.assertLess(first.origin[0], second.origin[0], "foreign inline content keeps source order")
        links = pdf_content.navigation(output)["links"]
        self.assertEqual(len(links), 1, f"the canonical foreign link is emitted once: {links}")
        self.assertEqual(links[0]["uri"], FOREIGN_LINK)
        left, bottom, right, top = links[0]["rect"]
        self.assertGreater(left, b.origin[0], "the link does not absorb preceding unlinked text")
        for glyph in (c, d):
            self.assertLessEqual(left, glyph.origin[0] + TOLERANCE_PT)
            self.assertGreater(right, glyph.origin[0])
            self.assertLessEqual(bottom, glyph.origin[1] + TOLERANCE_PT)
            self.assertGreaterEqual(top, glyph.origin[1] - TOLERANCE_PT)

    def test_inner_css_size_changes_box_geometry_without_scaling_glyphs(self):
        output, content = self.render(
            '<math><mrow class="sized"><mn>A</mn></mrow><mn>Z</mn></math>',
            f'.sized{{width:{USED_WIDTH_PT}pt;height:{USED_HEIGHT_PT}pt;background:blue}}')
        a, z = (self.one(content, text) for text in "AZ")
        self.assertAlmostEqual(z.origin[0] - a.origin[0], USED_WIDTH_PT,
                               delta=TOLERANCE_PT, msg="the next child follows the inner used CSS width")
        self.assertAlmostEqual(a.matrix[0], math.FONT_SIZE, delta=TOLERANCE_PT)
        self.assertAlmostEqual(a.matrix[3], math.FONT_SIZE, delta=TOLERANCE_PT,
                               msg="CSS sizing changes the box, not the font's glyph scale")
        with tables.TablePositioningPdf().raster(self.root, output, vector_antialias=False) as image:
            bounds = colour_bounds(image, (0, 0, 255))
        self.assertIsNotNone(bounds, "the actual inner CSS box background paints")
        left, top, right, bottom = bounds
        width, height = right - left, bottom - top
        self.assertEqual((width, height), (USED_WIDTH_PT, USED_HEIGHT_PT),
                         "inner width and height reach the normal background painter")

    def test_fixed_foreign_child_escapes_math_and_does_not_take_an_in_flow_slot(self):
        fixed = '<a class="fixed" href="' + FIXED_LINK + '">F</a>'
        css = (f'.fixed{{position:fixed;left:{FIXED_LEFT_PT}pt;top:{FIXED_TOP_PT}pt;'
               f'line-height:{LINE_HEIGHT_PT}pt;color:black;text-decoration:none}}')
        reference, reference_content = self.render(fixed, css)
        expected = self.one(reference_content, "F")
        expected_links = pdf_content.navigation(reference)["links"]
        output, content = self.render('<math><mtext>A' + fixed + '</mtext><mn>Z</mn></math>', css)
        actual = self.one(content, "F")
        for found, value in zip(actual.matrix, expected.matrix):
            self.assertAlmostEqual(found, value, delta=TOLERANCE_PT,
                                   msg="fixed content uses the external page containing block")
        self.assertEqual(pdf_content.navigation(output)["links"], expected_links,
                         "the escaped link follows its ordinary CSS fixed placement")
        a, z = (self.one(content, text) for text in "AZ")
        _, normal = self.render('<math><mtext>A</mtext><mn>Z</mn></math>', css)
        normal_a, normal_z = (self.one(normal, text) for text in "AZ")
        self.assertAlmostEqual(z.origin[0] - a.origin[0], normal_z.origin[0] - normal_a.origin[0],
                               delta=TOLERANCE_PT, msg="an out-of-flow child does not enter native arity or width")

    def test_native_math_does_not_inherit_the_host_first_line_font(self):
        body = '<p class="chosen">H<math><mfrac linethickness="1ex"><mn>1</mn><mn>2</mn></mfrac></math></p>'
        css = f'p{{margin:0}}.chosen::first-line{{font-size:{FIRST_LINE_SIZE_PT}pt}}'
        _, explicit = self.render(body, css + f'.chosen math{{font-size:{math.FONT_SIZE}pt}}')
        expected_one, expected_two = (self.one(explicit, text) for text in "12")
        _, selected = self.render(body, css)
        self.assertAlmostEqual(self.one(selected, "H").matrix[0], FIRST_LINE_SIZE_PT,
                               delta=TOLERANCE_PT, msg="the host first-line rule is actually active")
        actual_one, actual_two = (self.one(selected, text) for text in "12")
        for found, expected in ((actual_one, expected_one), (actual_two, expected_two)):
            for index in range(4):
                self.assertAlmostEqual(found.matrix[index], expected.matrix[index], delta=TOLERANCE_PT,
                                       msg="native math retains ordinary inheritance across the atomic boundary")
        for axis in range(2):
            self.assertAlmostEqual(actual_one.origin[axis] - actual_two.origin[axis],
                                   expected_one.origin[axis] - expected_two.origin[axis],
                                   delta=TOLERANCE_PT,
                                   msg="native relative attributes retain the ordinary math font basis")

    def test_math_nowrap_is_preserved_while_its_css_width_still_applies(self):
        _, content = self.render(
            '<math><mtext class="wrapped">AB CD EF</mtext><mn>Z</mn></math>',
            f'.wrapped{{width:{TOKEN_WIDTH_PT}pt;white-space:normal;line-height:{LINE_HEIGHT_PT}pt}}')
        glyphs = {text: self.one(content, text) for text in "ABCDEFZ"}
        for text in "BCDEF":
            self.assert_same_baseline(glyphs["A"], glyphs[text],
                                      "Core treats MathML white-space as nowrap even when overridden")
        self.assertAlmostEqual(glyphs["Z"].origin[0] - glyphs["A"].origin[0], TOKEN_WIDTH_PT,
                               delta=TOLERANCE_PT, msg="nowrap does not discard the owner's specified width")

    def test_native_math_stacking_context_contains_negative_and_positive_children(self):
        body = ('<section><math id="formula"><mtext id="content"><span id="back"></span>'
                '<span id="front"></span></mtext></math><div id="cover"></div></section>')
        css = ('section{position:relative;isolation:isolate;width:100pt;height:100pt}'
               '#formula{display:block math;position:relative;margin:0;width:80pt;height:60pt;background:blue}'
               '#content{width:80pt;height:60pt}'
               '#back,#front{position:absolute;width:60pt;height:40pt}'
               '#back{left:0;top:0;background:red;z-index:-1}'
               '#front{left:20pt;top:20pt;background:green;z-index:1}'
               '#cover{position:relative;margin-top:-40pt;width:80pt;height:40pt;background:yellow}')
        reference, _ = self.render(body, css + '#formula{isolation:isolate}')
        painter = tables.TablePositioningPdf()
        with painter.raster(self.root, reference, vector_antialias=False) as image:
            expected = image.tobytes()
            colours = {colour for _, colour in image.getcolors(image.width * image.height)}
            self.assertIn((255, 0, 0), colours,
                          "negative-z content paints above its own math background")
            self.assertIn((255, 255, 0), colours, "the later outer sibling actually overlaps math")
            self.assertNotIn((0, 128, 0), colours,
                             "positive math children remain below the later outer sibling")
        actual, _ = self.render(body, css)
        with painter.raster(self.root, actual, vector_antialias=False) as image:
            self.assertEqual(image.tobytes(), expected,
                             "native math creates its required stacking context without explicit isolation")

    def test_math_keeps_horizontal_metrics_and_natural_size_in_orthogonal_flows(self):
        body = '<section><math><mfrac><mn>1</mn><mn>2</mn></mfrac></math><span>Z</span></section>'
        css = (f'section{{width:{SECTION_SIZE_PT}pt;height:{SECTION_SIZE_PT}pt}}'
               'math{background:blue}')
        output, reference = self.render(body, css)
        reference_one, reference_two = (self.one(reference, text) for text in "12")
        painter = tables.TablePositioningPdf()
        with painter.raster(self.root, output, vector_antialias=False) as image:
            reference_bounds = colour_bounds(image, (0, 0, 255))
        self.assertIsNotNone(reference_bounds, "the reference math principal box must paint")
        reference_width = reference_bounds[2] - reference_bounds[0]
        for mode in WRITING_MODES:
            for override in ("", f'math{{writing-mode:{mode}!important}}'):
                with self.subTest(mode=mode, override=bool(override)):
                    output, content = self.render(body, css + f'section{{writing-mode:{mode}}}' + override)
                    one, two, following = (self.one(content, text) for text in "12Z")
                    for actual, expected in ((one, reference_one), (two, reference_two)):
                        for index in range(4):
                            self.assertAlmostEqual(actual.matrix[index], expected.matrix[index],
                                                   delta=TOLERANCE_PT,
                                                   msg="MathML's required horizontal writing mode preserves glyph scale and orientation")
                    for axis in range(2):
                        self.assertAlmostEqual(one.origin[axis] - two.origin[axis],
                                               reference_one.origin[axis] - reference_two.origin[axis],
                                               delta=TOLERANCE_PT,
                                               msg="fraction geometry remains physical horizontal math")
                    with painter.raster(self.root, output, vector_antialias=False) as image:
                        bounds = colour_bounds(image, (0, 0, 255))
                    self.assertIsNotNone(bounds, "the orthogonal principal box actually paints")
                    self.assertAlmostEqual(bounds[2] - bounds[0], reference_width,
                                           delta=PIXEL_QUANTIZATION,
                                           msg="an atomic fraction keeps its natural width inside a finite orthogonal container")
                    self.assertLess(abs(following.origin[1] - one.origin[1]), SECTION_SIZE_PT / 2,
                                    "following text uses the fraction's natural inline allocation")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments, remaining = parser.parse_known_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__, *remaining])
