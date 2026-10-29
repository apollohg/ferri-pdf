"""Native MathML geometry and selectable text, checked in the produced PDF."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import unittest
from fontTools.ttLib import TTFont
from PIL import Image
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

    def test_paired_script_baselines_use_math_constraints_in_the_pdf(self):
        _, content = self.render('<math><msubsup><mn>8</mn><mn>1</mn><mn>2</mn></msubsup></math>')
        glyphs = {glyph.text: glyph for glyph in content['glyphs']}
        with TTFont(FONT) as font:
            constants = font['MATH'].table.MathConstants
            units = font['head'].unitsPerEm
            script_size = FONT_SIZE * constants.ScriptPercentScaleDown / 100
            cmap = font.getBestCmap()
            def ascent(character, size):
                return font['glyf'][cmap[ord(character)]].yMax * size / units
            def descent(character, size):
                return -font['glyf'][cmap[ord(character)]].yMin * size / units
            def value(name):
                return getattr(constants, name).Value * FONT_SIZE / units
            down = max(value('SubscriptShiftDown'),
                       ascent('1', script_size) - value('SubscriptTopMax'),
                       value('SubscriptBaselineDropMin') + descent('8', FONT_SIZE))
            up = max(value('SuperscriptShiftUp'),
                     value('SuperscriptBottomMin') + descent('2', script_size),
                     ascent('8', FONT_SIZE) - value('SuperscriptBaselineDropMax'))
            gap = down - ascent('1', script_size) + up - descent('2', script_size)
            if gap < value('SubSuperscriptGapMin'):
                adjustment = min(max(value('SuperscriptBottomMaxWithSubscript') -
                                     (up - descent('2', script_size)), 0),
                                 value('SubSuperscriptGapMin') - gap)
                up += adjustment
                down += max(value('SubSuperscriptGapMin') - gap - adjustment, 0)
            self.assertAlmostEqual(glyphs['8'].origin[1] - glyphs['1'].origin[1], down,
                                   delta=TOLERANCE, msg='PDF subscript baseline must follow the font constraints')
            self.assertAlmostEqual(glyphs['2'].origin[1] - glyphs['8'].origin[1], up,
                                   delta=TOLERANCE, msg='PDF superscript baseline must follow the font constraints')
            for character in ['1', '2']:
                self.assertAlmostEqual(glyphs[character].matrix[0], script_size, delta=TOLERANCE,
                                       msg='script sizing must survive PDF serialization')

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

    def test_math_css_sizes_do_not_fit_or_clip_content_with_visible_overflow(self):
        def pixels(output):
            target = self.root / 'sizing-ink'
            subprocess.run(['pdftoppm', '-r', '72', '-singlefile', '-png', str(output), str(target)],
                           check=True, capture_output=True)
            with Image.open(target.with_suffix('.png')) as image:
                return image.convert('RGB').tobytes()
        normal, _ = self.render('<math><mn>1234</mn></math>')
        expected = pixels(normal)
        constrained, _ = self.render('<math style="width:1pt;height:1pt;object-fit:none;object-position:100% 100%"><mn>1234</mn></math>')
        self.assertEqual(pixels(constrained), expected,
                         'Core 3.1.2 changes math box dimensions while preserving start-aligned content and visible overflow; image fitting properties do not apply')

    def test_intrinsic_math_width_reserves_constructions_without_scaling_the_painted_glyph(self):
        _, content = self.render(
            "<math><mo lspace='0' rspace='0'>(</mo></math>U<br>"
            "<math style='font-family:MathTest'><mo lspace='0' rspace='0'>(</mo></math>D",
            'math{width:max-content}')
        fences = [glyph for glyph in content['glyphs'] if glyph.text == '(']
        following = {glyph.text: glyph for glyph in content['glyphs'] if glyph.text in ['U', 'D']}
        with TTFont(FONT) as original, TTFont(MATH_FONT) as derived:
            original_glyph = original.getBestCmap()[ord('(')]
            expected_original = original['hmtx'][original_glyph][0] * FONT_SIZE / original['head'].unitsPerEm
            derived_glyph = derived.getBestCmap()[ord('(')]
            variants = derived['MATH'].table.MathVariants
            construction = variants.VertGlyphConstruction[variants.VertGlyphCoverage.glyphs.index(derived_glyph)]
            expected_derived = max(derived['hmtx'][part.glyph][0] for part in construction.GlyphAssembly.PartRecords) * FONT_SIZE / derived['head'].unitsPerEm
        self.assertEqual(len(fences), 2, 'each native fence must remain one selectable base glyph')
        for glyph in fences:
            self.assertAlmostEqual(glyph.matrix[0], FONT_SIZE, delta=TOLERANCE,
                                   msg='preferred width must not horizontally scale the chosen glyph')
        self.assertAlmostEqual(following['U'].origin[0] - fences[0].origin[0], expected_original, delta=TOLERANCE,
                               msg='the unmodified invalid assembly keeps its normal-glyph preferred width')
        self.assertAlmostEqual(following['D'].origin[0] - fences[1].origin[0], expected_derived, delta=TOLERANCE,
                               msg='valid assembly width must reach CSS max-content spacing in the produced PDF')

    def test_non_math_display_math_preserves_css_flow_in_the_written_pdf(self):
        _, content = self.render('A <span style="display:inline math">x y z</span> B',
                                 'body{font:10pt/12pt monospace;width:35pt}')
        glyphs = {glyph.text: glyph for glyph in content['glyphs'] if glyph.text in ['A', 'x', 'y', 'z', 'B']}
        for character in ['x', 'y']:
            self.assertAlmostEqual(glyphs[character].origin[1], glyphs['A'].origin[1], delta=TOLERANCE,
                                   msg='HTML inline math must wrap as ordinary inline flow')
        self.assertAlmostEqual(glyphs['z'].origin[1], glyphs['B'].origin[1], delta=TOLERANCE)
        self.assertAlmostEqual(glyphs['A'].origin[1] - glyphs['z'].origin[1], 12, delta=TOLERANCE,
                               msg='the inline span must fragment across the two 12pt lines')
        _, content = self.render('<p><span style="display:block math">First</span></p>',
                                 'p::first-line{color:red}')
        glyphs = content['glyphs']
        self.assertEqual(''.join(glyph.text for glyph in glyphs), 'First')
        self.assertTrue(all(glyph.fill == (1, 0, 0, 1) for glyph in glyphs),
                        'HTML block math must preserve CSS first-line propagation in serialized glyph paint')

    def test_padded_semantics_preserve_fence_assemblies_and_prefix_spacing(self):
        _, content = self.render(
            "<math style='font-family:MathTest'><mo>(</mo><mspace height='40pt' depth='10pt'/></math><br>"
            "<math style='font-family:MathTest'><mpadded><mo>(</mo></mpadded><mspace height='40pt' depth='10pt'/></math>")
        spans = [span for span in content['replacement_text'] if span.text == '(']
        self.assertEqual(len(spans), 2, 'both bare and padded fences must remain native assemblies with one ActualText cluster each')
        lengths = [span.glyph_end - span.glyph_start for span in spans]
        self.assertGreater(lengths[0], 1, 'the fixture must stretch beyond its base glyph')
        self.assertEqual(lengths[0], lengths[1], 'mpadded must forward the same block stretch target')
        _, content = self.render(
            "<math><mo>+</mo><mn>1</mn></math><br>"
            "<math><mpadded><mspace width='10pt'/></mpadded><mo>+</mo><mn>1</mn></math>")
        plus = [glyph for glyph in content['glyphs'] if glyph.text == '+']
        digit = [glyph for glyph in content['glyphs'] if glyph.text == '1']
        self.assertEqual((len(plus), len(digit)), (2, 2))
        self.assertAlmostEqual(digit[0].origin[0] - plus[0].origin[0],
                               digit[1].origin[0] - plus[1].origin[0], delta=TOLERANCE,
                               msg='space-like mpadded content must preserve prefix operator spacing in serialized glyph positions')

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
