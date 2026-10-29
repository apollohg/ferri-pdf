"""Check anchor positioning in serialized PDF pixels, glyphs and navigation (§2–6)."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf
from oracle import pdf_content

PROFILE = ('@page{size:300pt 240pt;margin:20pt}html,body{margin:0;padding:0}'
           '.anchor{anchor-name:--a;width:60pt;height:30pt;margin-left:40pt;background:blue}'
           '.target{position:absolute;position-anchor:--a;width:20pt;height:10pt;background:red}')
BODY = '<div class=anchor>A</div><div class=target><a href="https://example.org/">T</a></div>'


class AnchorPositioningPdf(unittest.TestCase):
    def test_anchor_functions_areas_fallbacks_and_transforms_match_physical_geometry(self):
        cases = [
            ('.target{left:anchor(left);top:calc(anchor(bottom) + 5pt);'
             'width:anchor-size(width);height:anchor-size(height)}', (40, 35, 60, 30)),
            ('.target{position-area:bottom}', (60, 30, 20, 10)),
            ('.target{top:anchor(bottom);justify-self:anchor-center}', (60, 30, 20, 10)),
            ('.anchor{margin-top:170pt}.target{position-area:bottom;'
             'position-try-fallbacks:flip-block}', (60, 160, 20, 10)),
            ('@position-try --above{position-area:top}.anchor{margin-top:170pt}'
             '.target{position-area:bottom;position-try-fallbacks:--above}', (60, 160, 20, 10)),
            ('.target{position-area:left;position-try-fallbacks:right;'
             'position-try-order:most-width}', (100, 10, 20, 10)),
            ('.anchor{transform:translate(25pt,5pt)}'
             '.target{left:anchor(left);top:anchor(bottom)}', (65, 35, 20, 10)),
        ]
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-anchors-') as directory:
            root = Path(directory)
            for css, (left, top, width, height) in cases:
                with self.subTest(css=css):
                    output = renderer.render(root, PROFILE + css, BODY)
                    glyphs = [(glyph.text, glyph.origin) for glyph in pdf_content.inspect(output)['glyphs']]
                    links = pdf_content.navigation(output)['links']
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        actual = raster.tobytes()
                    reference = ('.target{position-anchor:none;position-area:none;position-try:none;'
                                 f'justify-self:normal;left:{left}pt;top:{top}pt;'
                                 f'width:{width}pt;height:{height}pt}}')
                    output = renderer.render(root, PROFILE + css + reference, BODY)
                    self.assertEqual(glyphs, [(glyph.text, glyph.origin)
                                             for glyph in pdf_content.inspect(output)['glyphs']],
                                     'the chosen geometry must move glyphs with the anchored box')
                    self.assertEqual(links, pdf_content.navigation(output)['links'],
                                     'the hyperlink must follow the anchored box')
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        self.assertEqual(actual, raster.tobytes(),
                                         'serialized anchor geometry differs from the independently placed boxes')

    def test_conditional_visibility_removes_ink_and_links(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-anchor-hidden-') as directory:
            root = Path(directory)
            for css in ('.anchor{visibility:hidden}.target{position-area:bottom}',
                        '.target{top:215pt;position-visibility:no-overflow}',
                        '.target{position-anchor:--missing;top:anchor(bottom);position-visibility:anchors-valid}'):
                with self.subTest(css=css):
                    output = renderer.render(root, PROFILE + css, BODY)
                    self.assertFalse(pdf_content.navigation(output)['links'], 'hidden anchor targets cannot expose links')
                    self.assertNotIn('T', [glyph.text for glyph in pdf_content.inspect(output)['glyphs']])
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        self.assertNotIn((255, 0, 0), list(raster.getdata()), 'hidden anchor targets cannot paint')


if __name__ == '__main__':
    unittest.main(verbosity=2)
