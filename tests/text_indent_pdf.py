"""Verify serialized indented glyph origins using independent font advances."""
from pathlib import Path
import tempfile
import unittest

from fontTools.ttLib import TTFont
import table_positioning_pdf
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
MARGIN = 20
WIDTH = 100
FONT_SIZE = 10
INDENT = 20
TOLERANCE = 0.001


class TextIndentPdf(unittest.TestCase):
    def test_start_margin_changes_glyph_origins_without_changing_advances(self):
        with TTFont(ROOT / 'tests/oracle/assets/DejaVuSans.ttf') as font:
            name = font.getBestCmap()[ord('M')]
            advance = font['hmtx'].metrics[name][0] * FONT_SIZE / font['head'].unitsPerEm
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-text-indent-') as directory:
            renderer = table_positioning_pdf.TablePositioningPdf()
            for direction in ('ltr', 'rtl'):
                for indent in (INDENT, -INDENT):
                    with self.subTest(direction=direction, indent=indent):
                        output = renderer.render(Path(directory),
                            f'p{{width:{WIDTH}pt;margin:0;direction:{direction};'
                            f'text-align:start;text-indent:{indent}pt each-line}}',
                            '<p>MMMM<br>MMMM</p>')
                        lines = {}
                        for glyph in pdf_content.inspect(output)['glyphs']:
                            lines.setdefault(round(glyph.matrix[5], 3), []).append(glyph)
                        self.assertEqual(len(lines), 2)
                        for glyphs in lines.values():
                            self.assertEqual(''.join(glyph.text for glyph in glyphs), 'MMMM')
                            origins = sorted(glyph.matrix[4] for glyph in glyphs)
                            expected = MARGIN + (indent if direction == 'ltr' else WIDTH - indent - 4 * advance)
                            for index, origin in enumerate(origins):
                                self.assertAlmostEqual(origin, expected + index * advance, delta=TOLERANCE)

    def test_inline_boxes_images_and_link_bounds_move_with_the_line(self):
        image = (ROOT / 'tests/oracle/assets/image-pattern.png').as_uri()
        body = ('<p><a href="mailto:reader@example.org"><span>MMMM</span>'
                f'<img src="{image}"></a></p>')
        shared = ('p{margin:0;width:100pt}a{color:black;text-decoration:underline}'
                  'span{background:yellow;border:1pt solid red}'
                  'img{width:12pt;height:12pt;vertical-align:middle}')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-text-indent-ink-') as directory:
            root = Path(directory)
            renderer = table_positioning_pdf.TablePositioningPdf()
            reference = renderer.render(root, shared + 'p{margin-left:20pt;width:80pt}', body)
            links = pdf_content.navigation(reference)['links']
            self.assertTrue(links)
            with renderer.raster(root, reference, vector_antialias=False) as raster:
                expected = raster.tobytes()
            output = renderer.render(root, shared + 'p{text-indent:20pt}', body)
            self.assertEqual(pdf_content.navigation(output)['links'], links)
            self.assertEqual(len(pdf_content.inspect(output)['images']), 1)
            with renderer.raster(root, output, vector_antialias=False) as raster:
                self.assertEqual(raster.tobytes(), expected,
                                 'indent must move every inline painting consumer consistently')

    def test_hanging_indent_survives_pdf_page_continuation(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-text-indent-pages-') as directory:
            output = table_positioning_pdf.TablePositioningPdf().render(Path(directory),
                '@page{size:180pt 68pt;margin:20pt}'
                'p{margin:0;width:65pt;font:10pt/12pt Probe;widows:1;orphans:1;text-indent:20pt hanging}',
                '<p>' + ' '.join(['MMMM'] * 8) + '</p>')
            lines = {}
            for glyph in pdf_content.inspect(output)['glyphs']:
                lines.setdefault((glyph.page, round(glyph.matrix[5], 3)), []).append(glyph)
            self.assertEqual(len({page for page, _ in lines}), 4)
            self.assertEqual(len(lines), 8)
            for index, glyphs in enumerate(lines.values()):
                self.assertAlmostEqual(min(glyph.matrix[4] for glyph in glyphs),
                                       MARGIN + (0 if index == 0 else INDENT), delta=TOLERANCE)


if __name__ == '__main__':
    unittest.main(verbosity=2)
