"""Native MathML geometry and selectable text, checked in the produced PDF."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import unittest
from fontTools.ttLib import TTFont
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
FONT = ROOT / "assets/fonts/DejaVuSerif.ttf"
MATH_FONT = ROOT / "tests/fixtures/fonts/MathTest.ttf"
FONT_SIZE = 20
TOLERANCE = 0.005

class MathInterpreter(pdf_content.Interpreter):
    """Allow cluster continuation glyphs only when an ActualText span covers them."""
    def __init__(self, document):
        super().__init__(document)
        self.unmapped = []
        self.wrapped = set()

    def font(self, resources, name):
        font = super().font(resources, name)
        if id(font) not in self.wrapped:
            self.wrapped.add(id(font))
            original = font.text
            def decode(code):
                if code not in font.unicode and not font.fallback_codec and not font.differences:
                    self.unmapped.append(len(self.glyphs))
                    return ''
                return original(code)
            font.text = decode
        return font


def inspect_math(output):
    document = pdf_content.Document(output)
    interpreter = MathInterpreter(document)
    for index, page in enumerate(document.pages(), start=1):
        interpreter.run_page(index, page)
    uncovered = [index for index in interpreter.unmapped
                 if not any(span.text and span.glyph_start <= index < span.glyph_end
                            for span in interpreter.replacement_text)]
    if uncovered:
        raise AssertionError(f'continuation glyphs lack ActualText coverage: {uncovered}')
    return {'glyphs': interpreter.glyphs, 'images': interpreter.images,
            'replacement_text': interpreter.replacement_text}


class MathmlPdf(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ferri-mathml-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def render(self, body, css=""):
        source = self.root / "input.html"
        output = self.root / "output.pdf"
        source.write_text(f"""<!doctype html><style>
        @font-face{{font-family:Probe;src:url('{FONT.as_uri()}')}}
        @font-face{{font-family:MathTest;src:url('{MATH_FONT.as_uri()}')}}
        @page{{size:300pt 200pt;margin:10pt}}
        body{{margin:0;font:{FONT_SIZE}pt Probe}}math{{font-family:Probe}}
        {css}</style>{body}""")
        subprocess.run([str(RENDERER), str(source), str(output)], check=True, capture_output=True)
        return output, inspect_math(output)

    def test_fraction_rule_and_gaps_follow_actual_math_table_and_text_remains_selectable(self):
        output, content = self.render("<math><mfrac><mn>1</mn><mn>234</mn></mfrac></math>")
        glyphs = {glyph.text: glyph for glyph in content['glyphs']}
        with TTFont(FONT) as font:
            units = font['head'].unitsPerEm
            constants = font['MATH'].table.MathConstants
            script_size = FONT_SIZE * constants.ScriptPercentScaleDown / 100
            cmap = font.getBestCmap()
            one = font['glyf'][cmap[ord('1')]]
            two = font['glyf'][cmap[ord('2')]]
            axis = constants.AxisHeight.Value * FONT_SIZE / units
            thickness = constants.FractionRuleThickness.Value * FONT_SIZE / units
            numerator_shift = max(constants.FractionNumeratorShiftUp.Value * FONT_SIZE / units,
                                  axis + thickness / 2 + constants.FractionNumeratorGapMin.Value * FONT_SIZE / units
                                  - one.yMin * script_size / units)
            denominator_shift = max(constants.FractionDenominatorShiftDown.Value * FONT_SIZE / units,
                                    thickness / 2 + constants.FractionDenominatorGapMin.Value * FONT_SIZE / units
                                    + two.yMax * script_size / units - axis)
            self.assertAlmostEqual(glyphs['1'].origin[1] - glyphs['2'].origin[1],
                                   numerator_shift + denominator_shift, delta=TOLERANCE,
                                   msg='fraction baselines must follow MATH shifts and minimum ink gaps')
            self.assertAlmostEqual(glyphs['1'].matrix[0], script_size, delta=TOLERANCE,
                                   msg='MATH script percentage must reach the PDF text matrix')
        self.assertFalse(content['images'], 'math must remain native vector text and rules')
        extracted = subprocess.run(['pdftotext', str(output), '-'], check=True, capture_output=True, text=True).stdout
        self.assertIn('1', extracted)
        self.assertIn('234', extracted)

    def test_stretched_fences_and_radicals_keep_font_size_and_extract_each_symbol_once(self):
        output, content = self.render('<math style="font-family:MathTest"><mo>(</mo><mroot><mfrac><mfrac><mn>1</mn><mn>2</mn></mfrac><mn>4</mn></mfrac><mn>3</mn></mroot><mo>)</mo></math>')
        self.assertFalse(content['images'], 'font assemblies must remain native selectable vector glyphs')
        replacements = content['replacement_text']
        self.assertTrue(any(item.text == '√' and item.glyph_end - item.glyph_start > 1
                            for item in replacements),
                        f'a surd assembly must map several glyph parts to one source character: {replacements}')
        self.assertTrue(any(item.text == '(' and item.glyph_end - item.glyph_start > 1
                            for item in replacements),
                        'a tall fence must be a font assembly with one source character')
        for glyph in content['glyphs']:
            if glyph.text in ('(', ')', '√'):
                self.assertAlmostEqual(glyph.matrix[0], FONT_SIZE, delta=TOLERANCE,
                                       msg='assembled symbols must retain the original horizontal font scale')
                self.assertAlmostEqual(glyph.matrix[3], FONT_SIZE, delta=TOLERANCE,
                                       msg='assembled symbols must retain the original vertical font scale')
        extracted = subprocess.run(['pdftotext', str(output), '-'], check=True, capture_output=True, text=True).stdout
        for symbol in ['(', ')', '√', '1', '2', '3', '4']:
            self.assertEqual(extracted.count(symbol), 1,
                             f'each assembled symbol and token must extract once: {extracted!r}')

    def test_math_alphabetic_baseline_matches_surrounding_inline_text(self):
        _, content = self.render('L<math><mn>8</mn></math>R')
        glyphs = {glyph.text: glyph for glyph in content['glyphs']}
        for text in ['8', 'R']:
            self.assertAlmostEqual(glyphs[text].origin[1], glyphs['L'].origin[1], delta=TOLERANCE,
                                   msg=f'{text} must share the surrounding alphabetic baseline')
        self.assertLess(glyphs['L'].origin[0], glyphs['8'].origin[0])
        self.assertLess(glyphs['8'].origin[0], glyphs['R'].origin[0])

    def test_math_is_an_atomic_fragment_at_a_page_boundary(self):
        _, content = self.render('<div style="height:75pt">P</div><math display="block"><mfrac><mn>1</mn><mn>2</mn></mfrac></math>',
                                 '@page{size:300pt 100pt;margin:10pt}')
        glyphs = {glyph.text: glyph for glyph in content['glyphs']}
        self.assertEqual(glyphs['P'].page, 1)
        self.assertEqual((glyphs['1'].page, glyphs['2'].page), (2, 2),
                         'a fraction must move to the next page as one native math box')

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--renderer', type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
