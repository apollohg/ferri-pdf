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

    def test_fixed_anchor_geometry_and_visibility_follow_each_physical_page(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        cases = [
            ('.anchor{position:fixed;left:50%;top:0;margin:0;width:50%}'
             '.target{position:fixed;position-anchor:--a;left:anchor(left);'
             'top:anchor(bottom);width:anchor-size(width)}',
             ((150, 50, 280, 60), (200, 50, 380, 60))),
            ('.anchor{margin-left:200pt;width:20pt}'
             '.target{position:fixed;position-anchor:--a;position-area:right;'
             'width:60pt;position-visibility:no-overflow}',
             (None, (240, 30, 300, 40))),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-anchor-pages-') as directory:
            root = Path(directory)
            for css, rectangles in cases:
                with self.subTest(css=css):
                    output = renderer.render(root, PROFILE + css +
                        '@page wide{size:400pt 240pt}.next{page:wide;break-before:page}',
                        ('<p>First</p>' if css.startswith('.anchor{position:fixed') else '') + BODY + '<p class=next>Next</p>')
                    glyphs = pdf_content.inspect(output)['glyphs']
                    for page, rectangle in enumerate(rectangles, start=1):
                        with renderer.raster(root, output, vector_antialias=False, page=page) as raster:
                            positions = [(x, y) for y in range(raster.height)
                                         for x in range(raster.width)
                                         if raster.getpixel((x, y)) == (255, 0, 0)]
                        bounds = ((min(x for x, _ in positions), min(y for _, y in positions),
                                   max(x for x, _ in positions) + 1, max(y for _, y in positions) + 1)
                                  if positions else None)
                        self.assertEqual(bounds, rectangle, f'physical page {page}: anchor rectangle')
                        text = [glyph for glyph in glyphs if glyph.page == page and glyph.text == 'T']
                        self.assertEqual(len(text), int(rectangle is not None),
                                         f'physical page {page}: subtree visibility must match the box')
                        if text:
                            self.assertAlmostEqual(text[0].origin[0], rectangle[0], places=3,
                                                   msg='fixed child text must follow its page-specific anchor')

    def test_anchor_visibility_uses_ink_overflow_and_intervening_clip_paths(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        cases = [
            ('section{width:20pt;height:40pt;overflow:hidden}'
             '.anchor{margin-left:30pt;width:10pt;height:10pt;box-shadow:-20pt 0 green}', True),
            ('section{width:100pt;height:100pt;clip-path:inset(100%)}'
             '.anchor{margin-left:30pt;width:10pt;height:10pt}', False),
            ("section{width:100pt;height:100pt;clip-path:path(evenodd,'M0 0H100V100H0Z M20 0H80V80H20Z')}"
             '.anchor{margin-left:30pt;width:10pt;height:10pt}', False),
        ]
        body = ('<section><div class=anchor></div></section>'
                '<div class=target><a href="https://example.org/">T</a></div>')
        target = '.target{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-anchor-ink-') as directory:
            root = Path(directory)
            for css, visible in cases:
                with self.subTest(css=css):
                    output = renderer.render(root, PROFILE + css + target, body)
                    glyphs = [glyph for glyph in pdf_content.inspect(output)['glyphs'] if glyph.text == 'T']
                    self.assertEqual(bool(glyphs), visible, 'target text follows ink visibility')
                    self.assertEqual(bool(pdf_content.navigation(output)['links']), visible,
                                     'target links follow ink visibility')
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        positions = [(x, y) for y in range(raster.height) for x in range(raster.width)
                                     if raster.getpixel((x, y)) == (255, 0, 0)]
                    if visible:
                        self.assertEqual((min(x for x, _ in positions), min(y for _, y in positions),
                                          max(x for x, _ in positions) + 1, max(y for _, y in positions) + 1),
                                         (50, 30, 70, 40),
                                         'shadow ink affects visibility while anchor() retains border geometry')
                        self.assertAlmostEqual(glyphs[0].origin[0], 50, places=3)
                    else:
                        self.assertFalse(positions, 'a clip-path-hidden anchor must force-hide target ink')

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
