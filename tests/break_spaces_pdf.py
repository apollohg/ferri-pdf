"""Compare preserved-space wrapping with explicit independently chosen line breaks."""
from pathlib import Path
import tempfile
import unittest

from fontTools.ttLib import TTFont
import table_positioning_pdf
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FONT_SIZE = 10


class BreakSpacesPdf(unittest.TestCase):
    def test_trailing_spaces_change_alignment_link_bounds_and_page_membership(self):
        with TTFont(ROOT / 'tests/oracle/assets/DejaVuSans.ttf') as font:
            cmap = font.getBestCmap()
            units = font['head'].unitsPerEm
            advance = lambda ch: font['hmtx'].metrics[cmap[ord(ch)]][0] * FONT_SIZE / units
            width = advance('M') + 2.5 * advance(' ')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-break-spaces-') as directory:
            root = Path(directory)
            renderer = table_positioning_pdf.TablePositioningPdf()
            for align in ('left', 'right', 'center'):
                for direction in ('ltr', 'rtl'):
                    with self.subTest(align=align, direction=direction):
                        css = ('@page{size:100pt 65pt;margin:10pt}'
                               f'p{{width:{width}pt;margin:0;font:10pt/14pt Probe;'
                               f'text-align:{align};direction:{direction};orphans:1;widows:1}}')
                        # Each line fits M plus two spaces; a third space does not fit.
                        body = '<a href="mailto:reader@example.org">{}</a>'
                        expected = renderer.render(root, css + 'p{white-space:pre}',
                            '<p>' + body.format('M  \nM  \nM  \nM  ') + '</p>')
                        expected_glyphs = pdf_content.inspect(expected)['glyphs']
                        expected_links = pdf_content.navigation(expected)['links']
                        images = []
                        for page in (1, 2):
                            with renderer.raster(root, expected, page=page, vector_antialias=False) as raster:
                                images.append(raster.tobytes())
                        actual = renderer.render(root, css + 'p{white-space:break-spaces}',
                            '<p>' + body.format('M  M  M  M  ') + '</p>')
                        self.assertEqual(pdf_content.inspect(actual)['glyphs'], expected_glyphs)
                        self.assertEqual(pdf_content.navigation(actual)['links'], expected_links)
                        for page, pixels in enumerate(images, 1):
                            with renderer.raster(root, actual, page=page, vector_antialias=False) as raster:
                                self.assertEqual(raster.tobytes(), pixels)
