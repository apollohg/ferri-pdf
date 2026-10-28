"""Check painted hyphenation strings, block overflow ellipses and hanging punctuation."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf
from oracle import pdf_content

PAGE_MARGIN = table_positioning_pdf.PAGE_MARGIN
WIDTH = 120
POSITION_TOLERANCE = 0.01


def baselines(glyphs):
    """Glyph texts grouped by baseline, top line first."""
    lines = {}
    for glyph in glyphs:
        lines.setdefault(round(glyph.origin[1], 2), []).append(glyph)
    return [sorted(lines[y], key=lambda glyph: glyph.origin[0]) for y in sorted(lines, reverse=True)]


class LineBreakingPdf(unittest.TestCase):
    def render(self, css, body):
        self.renderer = table_positioning_pdf.TablePositioningPdf()
        return pdf_content.inspect(self.renderer.render(self.root, f'p{{margin:0;width:{WIDTH}pt}}{css}', body))['glyphs']

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='ferri-pdf-line-breaking-')
        self.root = Path(self.directory.name)

    def tearDown(self):
        self.directory.cleanup()

    def test_automatic_hyphenation_paints_the_hyphenate_character_at_line_ends(self):
        for css, mark in (('', '‐'), ('p{hyphenate-character:"="}', '=')):
            with self.subTest(mark=mark):
                glyphs = self.render('p{hyphens:auto}' + css, '<p lang=en>representatives representatives representatives</p>')
                lines = baselines(glyphs)
                ends = [line[-1].text for line in lines]
                self.assertIn(mark, ends, f'line texts {["".join(g.text for g in line) for line in lines]}')
                text = ''.join(glyph.text for line in lines for glyph in line)
                self.assertEqual(text.replace(mark, '').replace(' ', ''), 'representatives' * 3)
                for line in lines:
                    if line[-1].text == mark:
                        self.assertLess(line[-1].origin[0], PAGE_MARGIN + WIDTH, 'the hyphen fits in the line')

    def test_line_clamp_paints_an_ellipsis_and_nothing_after_the_region_break(self):
        words = 'one two three four five six seven eight nine ten eleven twelve thirteen fourteen'
        glyphs = self.render('.c{line-clamp:2}', f'<p class=c>{words}</p><p>after</p>')
        lines = baselines(glyphs)
        texts = [''.join(glyph.text for glyph in line) for line in lines]
        self.assertEqual(len(lines), 3, texts)
        self.assertEqual(lines[1][-1].text, '…', texts)
        self.assertEqual(texts[2], 'after')
        self.assertLessEqual(lines[1][-1].origin[0], PAGE_MARGIN + WIDTH)
        self.assertAlmostEqual(lines[0][0].origin[1] - lines[1][0].origin[1], table_positioning_pdf.LINE_HEIGHT, delta=POSITION_TOLERANCE)
        self.assertAlmostEqual(lines[1][0].origin[1] - lines[2][0].origin[1], table_positioning_pdf.LINE_HEIGHT, delta=POSITION_TOLERANCE)

    def test_hanging_punctuation_leaves_the_letters_flush_with_the_edges(self):
        body = '<p>“Quoted words hang.”</p>'
        first = baselines(self.render('p{hanging-punctuation:first}', body))[0]
        self.assertEqual(first[0].text, '“')
        self.assertLess(first[0].origin[0], PAGE_MARGIN, 'the opening quote sits in the margin')
        self.assertAlmostEqual(first[1].origin[0], PAGE_MARGIN, delta=POSITION_TOLERANCE)
        plain = baselines(self.render('p{text-align:right}', body))[-1]
        last = baselines(self.render('p{text-align:right;hanging-punctuation:last}', body))[-1]
        self.assertEqual(last[-1].text, '”')
        self.assertAlmostEqual(last[-1].origin[0], PAGE_MARGIN + WIDTH, delta=POSITION_TOLERANCE,
                               msg='the closing quote starts at the end edge')
        shift = last[0].origin[0] - plain[0].origin[0]
        quote = PAGE_MARGIN + WIDTH - plain[-1].origin[0]
        self.assertAlmostEqual(shift, quote, delta=POSITION_TOLERANCE,
                               msg='the line moves right by the hanging advance')


if __name__ == '__main__':
    unittest.main(verbosity=2)
