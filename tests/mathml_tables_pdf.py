"""MathML Core table grouping, CSS cells, alignment and native selectable output."""
import argparse
from pathlib import Path
import tempfile
import unittest

from fontTools.ttLib import TTFont
from oracle import pdf_content
import mathml_pdf as math
import table_positioning_pdf as tables
from writing_anchors_pdf import colour_bounds

RENDERER = math.RENDERER
CELL_WIDTH_PT = 120
TABLE_HEIGHT_PT = 80
PIXEL_TOLERANCE_PT = 1


class MathmlTablesPdf(unittest.TestCase):
    render = math.MathmlPdf.render

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ferri-mathml-tables-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        math.RENDERER = RENDERER

    def one(self, content, text):
        glyphs = [glyph for glyph in content["glyphs"] if glyph.text == text]
        self.assertEqual(len(glyphs), 1, f"{text!r} paints once: {content['glyphs']}")
        return glyphs[0]

    def test_cell_children_form_one_anonymous_math_row(self):
        _, content = self.render('<math><mtable><mtr><mtd><mn>A</mn><mn>B</mn></mtd>'
                                 '</mtr></mtable></math>', 'math{font-family:MathTest}')
        a, b = (self.one(content, text) for text in "AB")
        self.assertAlmostEqual(a.origin[1], b.origin[1], delta=math.TOLERANCE,
                               msg="mtd wraps block-level math children in one native row")
        self.assertLess(a.origin[0], b.origin[0], "the anonymous row keeps source inline order")
        self.assertFalse(content["images"], "table math remains selectable native text")

    def test_default_table_style_is_compact_and_author_math_style_overrides_it(self):
        body = ('<math display=block><mfrac><mn>A</mn><mn>B</mn></mfrac>'
                '<mtable><mtr><mtd><mfrac><mn>C</mn><mn>D</mn></mfrac></mtd></mtr></mtable></math>')
        with TTFont(math.MATH_FONT) as font:
            script = font["MATH"].table.MathConstants.ScriptPercentScaleDown / 100
        for override, expected in [("", math.FONT_SIZE * script), ("mtable{math-style:normal}", math.FONT_SIZE)]:
            with self.subTest(override=override):
                _, content = self.render(body, 'math{font-family:MathTest}' + override)
                outside, inside = self.one(content, "A"), self.one(content, "C")
                self.assertAlmostEqual(outside.matrix[0], math.FONT_SIZE, delta=math.TOLERANCE)
                self.assertAlmostEqual(inside.matrix[0], expected, delta=math.TOLERANCE,
                                       msg="mtable compact style participates in real script font sizing and CSS cascade")

    def test_cells_center_the_anonymous_row_and_honor_author_text_alignment(self):
        body = '<math><mtable><mtr><mtd><mn>A</mn></mtd></mtr></mtable></math>'
        with TTFont(math.MATH_FONT) as font:
            glyph = font.getBestCmap()[ord("A")]
            advance = font["hmtx"].metrics[glyph][0] * math.FONT_SIZE / font["head"].unitsPerEm
        for direction in ["ltr", "rtl"]:
            for alignment in [None, "left", "center", "right", "start", "end"]:
                with self.subTest(direction=direction, alignment=alignment):
                    css = (f'math{{font-family:MathTest;direction:{direction}}}'
                           f'mtd{{width:{CELL_WIDTH_PT}pt;padding:0;background:red}}')
                    if alignment:
                        css += f'mtd{{text-align:{alignment}}}'
                    output, content = self.render(body, css)
                    a = self.one(content, "A")
                    with tables.TablePositioningPdf().raster(self.root, output, vector_antialias=False) as image:
                        bounds = colour_bounds(image, (255, 0, 0))
                    self.assertIsNotNone(bounds, "the ordinary table cell retains its assigned content box")
                    left, _, right, _ = bounds
                    self.assertEqual(right - left, CELL_WIDTH_PT)
                    free = CELL_WIDTH_PT - advance
                    used = alignment or "center"
                    expected = free / 2 if used == "center" else (free if used == "right"
                        or (used == "start" and direction == "rtl")
                        or (used == "end" and direction == "ltr") else 0)
                    self.assertAlmostEqual(a.origin[0] - left, expected, delta=math.TOLERANCE,
                                           msg="the natural math row is positioned inside, not resized to, the assigned CSS cell")

    def test_cell_default_padding_uses_real_ex_em_and_author_override(self):
        body = ('<math><mtable><mtr><mtd><mspace width="10pt" height="10pt" '
                'style="background:blue"/></mtd></mtr></mtable></math>')
        with TTFont(math.MATH_FONT) as font:
            x = font["glyf"][font.getBestCmap()[ord("x")]]
            ex = x.yMax * math.FONT_SIZE / font["head"].unitsPerEm
        for override, inline_padding, block_padding in [("", 0.4 * math.FONT_SIZE, 0.5 * ex),
                                                       ("mtd{padding:0}", 0, 0)]:
            with self.subTest(override=override):
                output, _ = self.render(body, 'math{font-family:MathTest}mtd{background:red}' + override)
                with tables.TablePositioningPdf().raster(self.root, output, vector_antialias=False) as image:
                    blue = colour_bounds(image, (0, 0, 255))
                    red = colour_bounds(image, (255, 0, 0))
                self.assertIsNotNone(blue, "the native space keeps its authored used box")
                if inline_padding:
                    self.assertIsNotNone(red, "default cell padding must surround the native math row")
                    self.assertAlmostEqual(red[2] - red[0], 10 + 2 * inline_padding, delta=PIXEL_TOLERANCE_PT)
                    self.assertAlmostEqual(red[3] - red[1], 10 + 2 * block_padding, delta=PIXEL_TOLERANCE_PT)
                    self.assertAlmostEqual(blue[0] - red[0], inline_padding, delta=PIXEL_TOLERANCE_PT)
                else:
                    self.assertIsNone(red, "author padding:0 replaces the user-agent default")

    def test_cell_span_attributes_reuse_html_slot_semantics(self):
        cases = [("columnspan=2", "table-column-span:2"),
                 ("rowspan=2", "table-row-span:2"),
                 ("rowspan=0", "table-row-span:0"),
                 ("columnspan=2 rowspan=0", "table-column-span:2;table-row-span:0"),
                 ('columnspan=" +2suffix"', "table-column-span:2"),
                 ("columnspan=0 rowspan=-3", "table-column-span:1;table-row-span:1")]
        template = ('<math><mtable><mtr><mtd class=span %s><mn>A</mn></mtd>'
                    '<mtd><mn>B</mn></mtd></mtr><mtr><mtd><mn>C</mn></mtd><mtd><mn>D</mn></mtd>'
                    '<mtd><mn>E</mn></mtd></mtr><mtr><mtd><mn>F</mn></mtd><mtd><mn>G</mn></mtd>'
                    '<mtd><mn>H</mn></mtd></mtr></mtable></math>')
        css = f'math{{font-family:MathTest}}mtable{{width:{CELL_WIDTH_PT}pt}}mtd{{padding:0}}'
        for attributes, reference in cases:
            with self.subTest(attributes=attributes):
                _, expected = self.render(template % "", css + f'.span{{{reference}}}')
                _, actual = self.render(template % attributes, css)
                self.assertEqual([glyph.text for glyph in actual["glyphs"]],
                                 [glyph.text for glyph in expected["glyphs"]],
                                 "spans retain every canonical cell's selectable contents")
                for glyph, control in zip(actual["glyphs"], expected["glyphs"]):
                    for value, reference in zip(glyph.matrix, control.matrix):
                        self.assertAlmostEqual(value, reference, delta=math.TOLERANCE,
                                               msg="MathML spans use the existing CSS table slots, including rowspan=0")

    def test_cell_row_stretches_operators_like_an_explicit_math_row(self):
        content = '<mo>(</mo><mfrac><mn>1</mn><mn>2</mn></mfrac><mo>)</mo>'
        css = 'math{font-family:MathTest}mtd{padding:0}'
        _, expected = self.render('<math><mrow>' + content + '</mrow></math>', css)
        _, actual = self.render('<math><mtable><mtr><mtd>' + content + '</mtd></mtr></mtable></math>', css)
        self.assertEqual([glyph.text for glyph in actual["glyphs"]],
                         [glyph.text for glyph in expected["glyphs"]])
        one = self.one(actual, "1")
        control_one = self.one(expected, "1")
        for glyph, control in zip(actual["glyphs"], expected["glyphs"]):
            for value, reference in zip(glyph.matrix[:4], control.matrix[:4]):
                self.assertAlmostEqual(value, reference, delta=math.TOLERANCE,
                                       msg="anonymous row grouping retains operator stretch and script glyph metrics")
            for axis in [0, 1]:
                self.assertAlmostEqual(glyph.origin[axis] - one.origin[axis],
                                       control.origin[axis] - control_one.origin[axis], delta=math.TOLERANCE,
                                       msg="mtd grouping shares the real mrow spacing and stretch algorithm")

    def test_inline_math_cell_operands_stretch_inside_the_required_row(self):
        content = ('<mo style="display:inline math">(</mo>'
                   '<mspace width="10pt" height="70pt" depth="10pt"/>'
                   '<mo style="display:inline math">)</mo>')
        css = 'math{font-family:MathTest}mtd{padding:0}'
        _, expected = self.render('<math><mrow>' + content + '</mrow></math>', css)
        _, actual = self.render('<math><mtable><mtr><mtd>' + content + '</mtd></mtr></mtable></math>', css)
        self.assertGreater(len(expected["glyphs"]), 2,
                           "the independent explicit row must exercise real tall fence assemblies")
        self.assertEqual([glyph.text for glyph in actual["glyphs"]],
                         [glyph.text for glyph in expected["glyphs"]],
                         "inline math operands must enter the anonymous row before inline atomic collection")
        origin = actual["glyphs"][0].origin
        reference_origin = expected["glyphs"][0].origin
        for glyph, control in zip(actual["glyphs"], expected["glyphs"]):
            for found, reference in zip(glyph.matrix[:4], control.matrix[:4]):
                self.assertAlmostEqual(found, reference, delta=math.TOLERANCE)
            for axis in [0, 1]:
                self.assertAlmostEqual(glyph.origin[axis] - origin[axis],
                                       control.origin[axis] - reference_origin[axis], delta=math.TOLERANCE)

    def test_anonymous_cell_row_does_not_add_a_stacking_context(self):
        output, _ = self.render(
            '<math><mtable><mtr><mtd><mspace width="20pt" height="20pt" '
            'style="position:relative;z-index:-1;background:blue"/></mtd></mtr></mtable></math>',
            'mtd{width:40pt;height:40pt;padding:0;background:red}')
        with tables.TablePositioningPdf().raster(self.root, output, vector_antialias=False) as image:
            red = colour_bounds(image, (255, 0, 0))
            blue = colour_bounds(image, (0, 0, 255))
        self.assertIsNotNone(red, "the actual CSS cell background paints")
        self.assertIsNone(blue, "the required anonymous mrow must not trap a negative-z descendant above the cell background")

    def test_cell_generated_content_and_foreign_link_keep_canonical_sources(self):
        body = ('<div><math><mtable><mtr><mtd class=numbered><mtext>'
                '<a href="#target" id=target>Q</a></mtext></mtd></mtr></mtable></math>'
                '<p class=after></p></div>')
        output, content = self.render(body, 'div{counter-reset:item 7}'
                                     '.numbered::before{counter-increment:item;content:counter(item)}'
                                     '.after::before{content:counter(item)}'
                                     f'mtd{{width:{CELL_WIDTH_PT}pt}}')
        counters = [glyph for glyph in content["glyphs"] if glyph.text == "8"]
        self.assertEqual(len(counters), 2, "the cell increments the same document counter read by following HTML")
        self.assertFalse(any(glyph.text == "7" for glyph in content["glyphs"]))
        q = self.one(content, "Q")
        links = pdf_content.navigation(output)["links"]
        self.assertEqual(len(links), 1, f"the canonical foreign link paints once: {links}")
        left, bottom, right, top = links[0]["rect"]
        self.assertLessEqual(left, q.origin[0] + math.TOLERANCE)
        self.assertGreater(right, q.origin[0])
        self.assertLessEqual(bottom, q.origin[1] + math.TOLERANCE)
        self.assertGreaterEqual(top, q.origin[1] - math.TOLERANCE)
        self.assertEqual(links[0]["destination"]["page"], q.page)

    def test_table_intrinsic_contribution_matches_css_table_for_percentage_cells(self):
        for keyword in ["min-content", "max-content"]:
            for percentages in ["", ".a{width:90%}.b{width:10%}"]:
                with self.subTest(keyword=keyword, percentages=percentages):
                    css = ('body{font-family:MathTest}math{font-family:MathTest}'
                           f'.host{{display:inline-block;width:{keyword};background:blue}}'
                           'td,mtd{padding:0}table,mtable{border-spacing:0}' + percentages)
                    bodies = ['<table><tr><td class=a>A</td><td class=b>B</td></tr></table>',
                              '<math><mtable><mtr><mtd class=a><mn>A</mn></mtd>'
                              '<mtd class=b><mn>B</mn></mtd></mtr></mtable></math>']
                    widths = []
                    for body in bodies:
                        output, _ = self.render('<div class=host>' + body + '</div>', css)
                        with tables.TablePositioningPdf().raster(self.root, output, vector_antialias=False) as image:
                            bounds = colour_bounds(image, (0, 0, 255))
                        self.assertIsNotNone(bounds, "the min/max-content host exposes the intrinsic contribution")
                        widths.append(bounds[2] - bounds[0])
                    self.assertEqual(widths[1], widths[0],
                                     "Math adoption preserves CSS table intrinsic widths independently of its used width")

    def test_css_table_baseline_sets_remain_the_first_row_baseline(self):
        _, content = self.render(
            '<math style="display:block"><mtext style="display:inline">R</mtext>'
            '<mtable><mtr><mtd><mn>A</mn></mtd></mtr><mtr><mtd><mn>B</mn></mtd></mtr></mtable></math>',
            f'math{{font-family:MathTest}}mtable{{height:{TABLE_HEIGHT_PT}pt}}mtd{{padding:0}}')
        r, a = self.one(content, "R"), self.one(content, "A")
        self.assertAlmostEqual(r.origin[1], a.origin[1], delta=math.TOLERANCE,
                               msg="ordinary CSS layout consumes the table's first row baseline; only Math adoption applies axis centering")

    def test_foreign_fixed_content_escapes_the_cell_row_without_taking_width(self):
        fixed = '<a class=fixed href="https://example.org/math-table-fixed">F</a>'
        for mode in ["horizontal-tb", "vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                css = (f'html{{writing-mode:{mode}}}.fixed{{position:fixed;writing-mode:horizontal-tb;'
                       'left:100pt;top:30pt;font-size:20pt;line-height:20pt;text-decoration:none}')
                reference, reference_content = self.render(fixed, css)
                expected = self.one(reference_content, "F")
                expected_links = pdf_content.navigation(reference)["links"]
                body = '<math><mtable><mtr><mtd><mtext>Q' + fixed + '</mtext></mtd></mtr></mtable></math>'
                output, content = self.render(body, css)
                actual = self.one(content, "F")
                for found, value in zip(actual.matrix, expected.matrix):
                    self.assertAlmostEqual(found, value, delta=math.TOLERANCE,
                                           msg="the anonymous cell row preserves the external fixed containing block")
                self.assertEqual(pdf_content.navigation(output)["links"], expected_links)
                self.one(content, "Q")

    def test_table_center_uses_the_actual_math_axis_after_css_table_sizing(self):
        body = ('<math><mn>X</mn><mtable><mtr><mtd><mn>1</mn></mtd></mtr>'
                '<mtr><mtd><mn>2</mn></mtd></mtr></mtable></math>')
        fallback = math.ROOT / "assets/fonts/LiberationSerif-Regular.ttf"
        for family, path in [("MathTest", math.MATH_FONT), ("MathFallback", fallback)]:
            with self.subTest(family=family), TTFont(path) as font:
                units = font["head"].unitsPerEm
                if "MATH" in font:
                    axis = font["MATH"].table.MathConstants.AxisHeight.Value * math.FONT_SIZE / units
                else:
                    axis = font["OS/2"].sxHeight * math.FONT_SIZE / units / 2
                css = (f'@font-face{{font-family:MathFallback;src:url("{fallback.as_uri()}")}}'
                       f'math{{font-family:{family}}}mtable{{height:{TABLE_HEIGHT_PT}pt;background:blue}}'
                       'mtd{padding:0}')
                output, content = self.render(body, css)
                x = self.one(content, "X")
                with tables.TablePositioningPdf().raster(self.root, output, vector_antialias=False) as image:
                    bounds = colour_bounds(image, (0, 0, 255))
                self.assertIsNotNone(bounds, "the ordinary CSS table background must paint")
                _, top, _, bottom = bounds
                self.assertAlmostEqual(bottom - top, TABLE_HEIGHT_PT, delta=PIXEL_TOLERANCE_PT,
                                       msg="the existing table solver retains authored used height")
                alphabetic = math.PAGE_HEIGHT_PT - x.origin[1]
                self.assertAlmostEqual((top + bottom) / 2, alphabetic - axis, delta=PIXEL_TOLERANCE_PT,
                                       msg="the table center uses MATH AxisHeight or the defined non-MATH x-height fallback")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--renderer", type=Path, required=True)
    args, rest = parser.parse_known_args()
    RENDERER = args.renderer.resolve()
    unittest.main(argv=[__file__, *rest])
