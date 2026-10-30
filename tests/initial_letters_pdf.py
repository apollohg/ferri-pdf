"""Check initial letters and typographic pseudo-elements in the PDF against the font's own
outlines and metrics (CSS Inline 3 §7, CSS Pseudo-Elements 4 §2, CSS Lists 3 §3.1)."""
from pathlib import Path
import tempfile
import unittest

from fontTools.pens.boundsPen import BoundsPen
from fontTools.ttLib import TTFont
import table_positioning_pdf
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / 'tests/oracle/assets/DejaVuSans.ttf'
MARGIN = table_positioning_pdf.PAGE_MARGIN
FONT_SIZE = table_positioning_pdf.FONT_SIZE
LINE_HEIGHT = table_positioning_pdf.LINE_HEIGHT
TOLERANCE = 0.01
RED = (1.0, 0.0, 0.0, 1.0)
BLUE = (0.0, 0.0, 1.0, 1.0)
BLACK = (0.0, 0.0, 0.0, 1.0)
LIME_INK = (0, 255, 0)
WHITE_INK = (255, 255, 255)
FRAGMENT_PADDING = 4
TEXT = ('Lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor '
        'incididunt ut labore et dolore magna aliqua ut enim ad minim veniam quis nostrud')


def outline(character):
    """The glyph's bounds and the font's units per em."""
    with TTFont(FONT) as font:
        glyphs = font.getGlyphSet()
        pen = BoundsPen(glyphs)
        glyphs[font.getBestCmap()[ord(character)]].draw(pen)
        return pen.bounds, font['head'].unitsPerEm


def lines(glyphs):
    """Glyphs grouped by baseline, top line first, each in reading order."""
    grouped = {}
    for glyph in glyphs:
        grouped.setdefault(round(glyph.matrix[5], 3), []).append(glyph)
    return [sorted(grouped[y], key=lambda glyph: glyph.matrix[4])
            for y in sorted(grouped, reverse=True)]


class InitialLettersPdf(unittest.TestCase):
    def render(self, root, css, body):
        output = table_positioning_pdf.TablePositioningPdf().render(root, css, body)
        return pdf_content.inspect(output)['glyphs']

    def test_three_line_drop_cap_spans_cap_height_to_third_baseline(self):
        (x_min, _, _, cap_top), em = outline('L')
        (_, _, _, h_top), _ = outline('H')
        cap = h_top / em
        size = (2 * LINE_HEIGHT + cap * FONT_SIZE) / cap
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-initial-letter-') as directory:
            glyphs = self.render(Path(directory),
                                 'p{width:200pt;margin:0}p::first-letter{initial-letter:3;color:red}',
                                 f'<p>{TEXT}</p>')
        letter = [glyph for glyph in glyphs if glyph.fill == RED]
        self.assertEqual([glyph.text for glyph in letter], ['L'], f'one red drop cap: {letter}')
        letter = letter[0]
        text = lines([glyph for glyph in glyphs if glyph.fill == BLACK])
        self.assertGreaterEqual(len(text), 5, 'the paragraph wraps past the letter')
        self.assertAlmostEqual(letter.matrix[3], size, delta=TOLERANCE,
                               msg='the cap height spans two line heights plus the text cap height')
        self.assertAlmostEqual(letter.matrix[5], text[2][0].matrix[5], delta=TOLERANCE,
                               msg='the letter sits on the third baseline')
        self.assertAlmostEqual(letter.matrix[5] + cap_top / em * size,
                               text[0][0].matrix[5] + cap * FONT_SIZE, delta=TOLERANCE,
                               msg="the letter's outline meets the first line's cap height")
        self.assertAlmostEqual(letter.matrix[4] + x_min / em * size, MARGIN, delta=TOLERANCE,
                               msg='its outline is kerned flush with the line start')
        beside = {round(line[0].matrix[4], 3) for line in text[:3]}
        self.assertEqual(len(beside), 1, f'the three lines beside it start together: {beside}')
        self.assertGreater(beside.pop(), letter.matrix[4] + cap_top / em * size / 2,
                           'and after the letter')
        for line in text[3:]:
            self.assertAlmostEqual(line[0].matrix[4], MARGIN, delta=TOLERANCE,
                                   msg='lines below the letter return to the start edge')

    def test_letter_too_long_for_a_line_drops_each_of_its_lines(self):
        (_, _, _, h_top), em = outline('H')
        cap = h_top / em
        size = (2 * LINE_HEIGHT + cap * FONT_SIZE) / cap
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-wrapped-letter-') as directory:
            glyphs = self.render(Path(directory),
                                 'p{width:200pt;margin:0}span{initial-letter:3;color:red}',
                                 f'<p><span>Once more</span> {TEXT}</p>')
        rows = lines([glyph for glyph in glyphs if glyph.fill == RED])
        text = lines([glyph for glyph in glyphs if glyph.fill == BLACK])
        self.assertEqual([''.join(glyph.text for glyph in row) for row in rows], ['Once', 'more'],
                         'the letter wraps onto two lines of its own')
        for row in rows:
            self.assertAlmostEqual(row[0].matrix[3], size, delta=TOLERANCE,
                                   msg='each letter line takes the letter size')
        self.assertLess(text[0][0].matrix[5] + FONT_SIZE, rows[0][0].matrix[5],
                        'normal text starts below the first letter line')
        self.assertAlmostEqual(rows[1][0].matrix[5], text[2][0].matrix[5], delta=TOLERANCE,
                               msg='the last letter line sits on the third baseline from there')
        end = rows[1][-1].matrix[4]
        for line in text[:3]:
            self.assertGreater(line[0].matrix[4], end,
                               'the lines beside the last letter line start after it')
        for line in text[3:]:
            self.assertAlmostEqual(line[0].matrix[4], MARGIN, delta=TOLERANCE,
                                   msg='lines below the letter return to the start edge')

    def test_wrapped_letter_paints_a_box_fragment_per_line(self):
        """Each line of a wrapped letter is a box fragment (§7.8.4) that `box-decoration-break`
        slices or clones at the line break (CSS Fragmentation 3 §5.4)."""
        (_, _, _, h_top), em = outline('H')
        cap = h_top / em
        size = (2 * LINE_HEIGHT + cap * FONT_SIZE) / cap
        with TTFont(FONT) as font:
            cmap, metrics = font.getBestCmap(), font['hmtx'].metrics
        def end(row):
            return row[-1].matrix[4] + metrics[cmap[ord(row[-1].text)]][0] / em * size
        for decoration, cloned in (('slice', False), ('clone', True)):
            with self.subTest(decoration=decoration), \
                    tempfile.TemporaryDirectory(prefix='ferri-pdf-letter-boxes-') as directory:
                root = Path(directory)
                renderer = table_positioning_pdf.TablePositioningPdf()
                output = renderer.render(
                    root,
                    'p{width:200pt;margin:0}span{initial-letter:3;color:red;background:lime;'
                    f'padding:{FRAGMENT_PADDING}pt;box-decoration-break:{decoration}}}',
                    f'<p><span>Once more</span> {TEXT}</p>')
                rows = lines([glyph for glyph in pdf_content.inspect(output)['glyphs']
                              if glyph.fill == RED])
                self.assertEqual([''.join(glyph.text for glyph in row) for row in rows],
                                 ['Once', 'more'])
                with renderer.raster(root, output, vector_antialias=False) as raster:
                    def ink(x, row):
                        """The ink beside a row, halfway up its cap height."""
                        y = table_positioning_pdf.PAGE_HEIGHT - row[0].matrix[5] - cap * size / 2
                        return raster.getpixel((round(x), round(y)))
                    edge = FRAGMENT_PADDING / 2
                    first, last = rows
                    self.assertEqual(ink(first[0].matrix[4] - edge, first), LIME_INK,
                                     'the first fragment has its start padding')
                    self.assertEqual(ink(end(first) + edge, first),
                                     LIME_INK if cloned else WHITE_INK,
                                     'only a cloned first fragment has end padding')
                    self.assertEqual(ink(last[0].matrix[4] - edge, last),
                                     LIME_INK if cloned else WHITE_INK,
                                     'only a cloned last fragment has start padding')
                    self.assertEqual(ink(end(last) + edge, last), LIME_INK,
                                     'the last fragment has its end padding')

    def test_percentage_padding_sizes_a_border_box_letter_by_its_containing_block(self):
        """Border-box alignment fits the letter's font inside its block-axis padding (CSS Inline
        3 §7.4), whose percentages refer to the paragraph's width."""
        sizes = {}
        for padding in ('5%', '10pt', '0pt'):
            with tempfile.TemporaryDirectory(prefix='ferri-pdf-letter-padding-') as directory:
                glyphs = self.render(Path(directory),
                                     'p{width:200pt;margin:0}p::first-letter{initial-letter:2;'
                                     f'initial-letter-align:border-box;padding:{padding};color:red}}',
                                     f'<p>{TEXT}</p>')
            letter = [glyph for glyph in glyphs if glyph.fill == RED]
            self.assertEqual([glyph.text for glyph in letter], ['L'])
            sizes[padding] = letter[0].matrix[3]
        self.assertAlmostEqual(sizes['5%'], sizes['10pt'], delta=TOLERANCE,
                               msg='5% of the 200pt paragraph pads like 10pt')
        self.assertLess(sizes['5%'], sizes['0pt'] - 1, 'the padding shrinks the letter')

    def test_first_letter_and_first_line_style_their_text(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-first-line-') as directory:
            glyphs = self.render(Path(directory),
                                 'p{width:200pt;margin:0}p::first-line{color:blue}'
                                 'p::first-letter{color:red;font-size:20pt}',
                                 f'<p>{TEXT}</p>')
        text = lines(glyphs)
        first, rest = text[0], [glyph for line in text[1:] for glyph in line]
        self.assertEqual((first[0].text, first[0].fill), ('L', RED))
        self.assertAlmostEqual(first[0].matrix[3], 20, delta=TOLERANCE)
        self.assertTrue(all(glyph.fill == BLUE and abs(glyph.matrix[3] - FONT_SIZE) < TOLERANCE
                            for glyph in first[1:]), 'the rest of the first line is blue')
        self.assertTrue(rest and all(glyph.fill == BLACK for glyph in rest),
                        'later lines keep the paragraph colour')

    def test_marker_styles_only_the_marker_text(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-marker-') as directory:
            glyphs = self.render(Path(directory),
                                 'ol{margin:0;padding-left:40pt}li{color:blue}'
                                 'li::marker{color:red;font-size:16pt}',
                                 '<ol><li>Item</li></ol>')
        marker = [glyph for glyph in glyphs if glyph.fill == RED]
        item = [glyph for glyph in glyphs if glyph.fill == BLUE]
        self.assertEqual(''.join(glyph.text for glyph in marker).strip(), '1.')
        self.assertTrue(all(abs(glyph.matrix[3] - 16) < TOLERANCE for glyph in marker))
        self.assertEqual(''.join(glyph.text for glyph in item), 'Item')
        self.assertTrue(all(abs(glyph.matrix[3] - FONT_SIZE) < TOLERANCE for glyph in item))
        self.assertLess(max(glyph.matrix[4] for glyph in marker), item[0].matrix[4],
                        'the outside marker precedes its item')
        self.assertAlmostEqual(marker[0].matrix[5], item[0].matrix[5], delta=TOLERANCE,
                               msg='the marker shares the first line baseline')


if __name__ == '__main__':
    unittest.main(verbosity=2)
