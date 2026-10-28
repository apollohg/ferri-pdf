"""Compare transformed PDF text, painting and links with explicit Unicode input."""
import copy
from pathlib import Path
import tempfile
import unittest

from fontTools.ttLib import TTFont
import table_positioning_pdf
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]


class TextTransformPdf(unittest.TestCase):
    def test_combined_transforms_match_explicit_unicode_through_wrapping_and_links(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-width-kana-') as directory:
            root = Path(directory)
            font_path = root / 'transform.ttf'
            with TTFont(ROOT / 'tests/oracle/assets/DejaVuSans.ttf') as font:
                cmap = font.getBestCmap()
                order = font.getGlyphOrder().copy()
                for character, original in [('Ｓ', 'S'), ('Ａ', 'A'), ('ア', 'B'), ('Ｂ', 'C')]:
                    source_name = cmap[ord(original)]
                    name = f'transform{ord(character):X}'
                    font['glyf'][name] = copy.deepcopy(font['glyf'][source_name])
                    font['hmtx'][name] = font['hmtx'][source_name]
                    order.append(name)
                    for table in font['cmap'].tables:
                        if table.isUnicode() and table.format in (4, 12):
                            table.cmap[ord(character)] = name
                font.setGlyphOrder(order)
                font.save(font_path)
            renderer = table_positioning_pdf.TablePositioningPdf()
            css = (f'@font-face{{font-family:Transform;src:url("{font_path.as_uri()}")}}'
                   'p{font:10pt/12pt Transform;width:45pt;margin:0}'
                   'span{color:red}a{color:black;text-decoration:underline}')
            source = '<p><a href="mailto:reader@example.org">ß <span>a</span> ｧ b ß a ｧ b</a></p>'
            literal = source.replace('ß', 'ＳＳ').replace('<span>a</span>', '<span>Ａ</span>').replace('ｧ', 'ア').replace(' b', ' Ｂ').replace(' a ', ' Ａ ')
            reference = renderer.render(root, css, literal)
            expected_glyphs = pdf_content.inspect(reference)['glyphs']
            expected_links = pdf_content.navigation(reference)['links']
            self.assertTrue(expected_links)
            with renderer.raster(root, reference, vector_antialias=False) as raster:
                expected_ink = raster.tobytes()
            output = renderer.render(root, css + 'p{text-transform:uppercase full-width full-size-kana}', source)
            actual = pdf_content.inspect(output)['glyphs']
            self.assertEqual([(g.text, g.matrix, g.fill) for g in actual],
                             [(g.text, g.matrix, g.fill) for g in expected_glyphs])
            self.assertGreater(len({g.matrix[5] for g in actual}), 1, 'exercise transformed line wrapping')
            self.assertEqual(pdf_content.navigation(output)['links'], expected_links)
            with renderer.raster(root, output, vector_antialias=False) as raster:
                self.assertEqual(raster.tobytes(), expected_ink)


if __name__ == '__main__':
    unittest.main(verbosity=2)
