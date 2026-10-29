"""Rendered checks that `clip-path` basic shapes and geometry boxes clip a box and its
descendants (CSS Masking 1 §6.1), including box fragments on several pages."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf
from table_positioning_pdf import PAGE_HEIGHT, PAGE_MARGIN

RED = (255, 0, 0)
BLUE = (0, 0, 255)
WHITE = (255, 255, 255)
SIZE = 100


class ClipPathPdf(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix='ferri-pdf-clip-path-')
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def raster(self, css, body='<div class=c></div>', page=1):
        root = Path(tempfile.mkdtemp(dir=self.root))
        output = self.renderer.render(root, 'p{margin:0}' + css, body)
        with self.renderer.raster(root, output, vector_antialias=False, page=page) as image:
            return image.copy()

    def test_circle_clips_the_box_to_its_disc(self):
        image = self.raster(f'.c{{width:{SIZE}pt;height:{SIZE}pt;background:#f00;'
                            'clip-path:circle(50%)}')
        disc = self.raster(f'.c{{width:{SIZE}pt;height:{SIZE}pt;background:#f00;'
                           'border-radius:50%}')
        self.assertEqual(image.tobytes(), disc.tobytes(),
                         'the clipped box covers the pixels of a fully rounded box')
        middle = PAGE_MARGIN + SIZE // 2
        self.assertEqual(image.getpixel((middle, middle)), RED)
        self.assertEqual(image.getpixel((PAGE_MARGIN + 3, PAGE_MARGIN + 3)), WHITE,
                         'the corners outside the circle are clipped away')

    def test_overflow_clip_margin_sides_match_an_asymmetric_rounded_inset(self):
        css = ('.c{position:relative;width:100pt;height:40pt;padding:4pt;margin:30pt;'
               'border:3pt solid transparent;border-radius:16pt}'
               '.c div{position:absolute;left:-100pt;top:-100pt;'
               'width:300pt;height:300pt;background:#f00}')
        body = '<div class=c><div></div></div>'
        actual = self.raster(css + '.c{overflow:clip;'
                             'overflow-clip-margin-top:border-box 2pt;'
                             'overflow-clip-margin-right:content-box 3pt;'
                             'overflow-clip-margin-bottom:padding-box 5pt;'
                             'overflow-clip-margin-left:border-box 7pt}', body)
        expected = self.raster(css + '.c{clip-path:inset(-2pt 4pt -2pt -7pt '
                               'round 23pt 12pt 12pt 23pt / 18pt)}', body)
        self.assertEqual(actual.tobytes(), expected.tobytes(),
                         'each side origin and corner axis must reach the PDF clip path')

    def test_descendants_are_clipped_and_even_odd_polygons_leave_holes(self):
        css = (f'.c{{width:{SIZE}pt;height:{SIZE}pt;background:#f00}}'
               '.c div{width:200pt;height:20pt;background:#00f}')
        body = '<div class=c><div></div></div>'
        overflow = (PAGE_MARGIN + 150, PAGE_MARGIN + 10)
        self.assertEqual(self.raster(css, body).getpixel(overflow), BLUE,
                         'the child overflows the box unclipped')
        clipped = self.raster(css + '.c{clip-path:circle(50%)}', body)
        self.assertEqual(clipped.getpixel(overflow), WHITE, 'clip-path clips descendants too')
        holed = self.raster(css + '.c{clip-path:polygon(evenodd,0 0,100% 0,100% 100%,0 100%,'
                                  '0 0,25% 25%,75% 25%,75% 75%,25% 75%,25% 25%)}', body)
        self.assertEqual(holed.getpixel((PAGE_MARGIN + 50, PAGE_MARGIN + 50)), WHITE,
                         'the even-odd inner square is a hole')
        self.assertEqual(holed.getpixel((PAGE_MARGIN + 10, PAGE_MARGIN + 50)), RED)

    def test_geometry_boxes_select_their_edges_with_svg_boxes_mapped_for_css_boxes(self):
        css = ('.c{width:60pt;height:60pt;padding:10pt;border:10pt solid #00f;margin:10pt;'
               'background:#f00;clip-path:%s}')
        boxes = {value: self.raster(css % value).tobytes()
                 for value in ('content-box', 'fill-box', 'padding-box', 'border-box',
                               'stroke-box', 'view-box', 'inset(0)', 'none')}
        self.assertEqual(boxes['fill-box'], boxes['content-box'], 'fill-box is the content box')
        for value in ('stroke-box', 'view-box', 'inset(0)'):
            self.assertEqual(boxes[value], boxes['border-box'], f'{value} is the border box')
        self.assertEqual(boxes['border-box'], boxes['none'], 'the border box clips nothing here')
        self.assertNotEqual(boxes['padding-box'], boxes['border-box'])
        content = self.raster(css % 'content-box')
        inside_padding = (PAGE_MARGIN + 10 + 10 + 5, PAGE_MARGIN + 60)
        self.assertEqual(content.getpixel(inside_padding), WHITE)
        self.assertEqual(content.getpixel((PAGE_MARGIN + 60, PAGE_MARGIN + 60)), RED)
        # The margin box is 120pt across, so its circle reaches 60pt from the centre, 10pt
        # further than the border box's.
        probe = (PAGE_MARGIN + 11, PAGE_MARGIN + 35)
        self.assertEqual(self.raster(css % 'circle(50%)').getpixel(probe), WHITE)
        self.assertEqual(self.raster(css % 'circle(50%) margin-box').getpixel(probe), BLUE)

    def test_sliced_fragments_share_one_shape_and_cloned_fragments_have_their_own(self):
        css = (f'.c{{width:{SIZE}pt;height:500pt;background:#f00;clip-path:inset(10pt);'
               'box-decoration-break:%s}')
        last_row = PAGE_HEIGHT - PAGE_MARGIN - 1
        for decoration, expected in (('slice', RED), ('clone', WHITE)):
            with self.subTest(decoration=decoration):
                image = self.raster(css % decoration)
                self.assertEqual(image.getpixel((PAGE_MARGIN + 50, PAGE_MARGIN + 5)), WHITE,
                                 'the box starts inset on page one')
                self.assertEqual(image.getpixel((PAGE_MARGIN + 50, last_row)), expected,
                                 'only a cloned fragment is inset at the page break')


if __name__ == '__main__':
    unittest.main()
