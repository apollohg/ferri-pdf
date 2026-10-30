"""Corner shapes compared with independent vector contours in the rendered PDF."""
from pathlib import Path
import tempfile
import unittest

import embedded_content_pdf


class CornerShapesPdf(unittest.TestCase):
    def reference(self, renderer, root, points, size=(80, 60)):
        polygon = ','.join(f'{x}pt {y}pt' for x, y in points)
        return renderer.ink(root,
            f'div{{width:{size[0]}pt;height:{size[1]}pt;background:red;clip-path:polygon(evenodd,{polygon})}}',
            '<div></div>')

    def test_opposite_notches_reduce_radii_before_the_cutouts_overlap(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-corner-overlap-') as directory:
            root = Path(directory)
            expected = self.reference(renderer, root,
                [(40,0),(80,0),(80,30),(40,30),(40,60),(0,60),(0,30),(40,30)])
            actual = renderer.ink(root,
                'div{width:80pt;height:60pt;background:red;corner-shape:notch;'
                'border-top-left-radius:80%;border-bottom-right-radius:80%}', '<div></div>')
            self.assertTrue(actual == expected, 'both opposing corner areas must scale to 50%')

    def test_angular_corners_match_explicit_polygon_contours(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('bevel', 'superellipse(0)', [(20,0),(60,0),(80,20),(80,40),(60,60),(20,60),(0,40),(0,20)]),
            ('notch', 'superellipse(-infinity)', [(20,0),(60,0),(60,20),(80,20),(80,40),(60,40),(60,60),(20,60),(20,40),(0,40),(0,20),(20,20)]),
            ('square', 'superellipse(infinity)', [(0,0),(80,0),(80,60),(0,60)]),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-corner-shape-') as directory:
            root = Path(directory)
            for keyword, parameter, points in cases:
                expected = self.reference(renderer, root, points)
                self.assertTrue(any(r > g and r > b for r, g, b in zip(expected[::3], expected[1::3], expected[2::3])),
                                'the independent reference must paint a visible shape')
                for shape in (keyword, parameter):
                    with self.subTest(shape=shape):
                        actual = renderer.ink(root,
                            f'div{{width:80pt;height:60pt;background:red;border-radius:20pt;corner-shape:{shape}}}',
                            '<div></div>')
                        self.assertTrue(actual == expected, 'corner shape must change the complete background contour')

    def test_shaped_float_wraps_like_the_independent_notch_polygon(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        body = '<div class=float></div>' + '<p>' + 'word ' * 90 + '</p>'
        common = ('.float{float:left;width:80pt;height:60pt}'
                  'p{margin:0;font:10pt/10pt Probe}')
        polygon = ('polygon(20pt 0,60pt 0,60pt 20pt,80pt 20pt,80pt 40pt,'
                   '60pt 40pt,60pt 60pt,20pt 60pt,20pt 40pt,0 40pt,0 20pt,20pt 20pt)')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-corner-float-') as directory:
            root = Path(directory)
            for margin in (0, 3):
                with self.subTest(margin=margin):
                    rules = common + f'.float{{shape-margin:{margin}pt}}'
                    expected = renderer.ink(root, rules + f'.float{{shape-outside:{polygon}}}', body)
                    actual = renderer.ink(root, rules + '.float{corner:notch 20pt;shape-outside:border-box}', body)
                    self.assertTrue(actual == expected, 'box-valued float shape must retain the concave cuts')

    def test_geometry_box_clip_uses_corner_shape_but_explicit_inset_does_not(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        common = ('div{width:80pt;height:60pt;corner:bevel 20pt}'
                  'span{display:block;width:80pt;height:60pt;background:red}')
        references = [
            ('border-box', [(20,0),(60,0),(80,20),(80,40),(60,60),(20,60),(0,40),(0,20)]),
            ('inset(0)', [(0,0),(80,0),(80,60),(0,60)]),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-corner-clip-') as directory:
            root = Path(directory)
            for clip, points in references:
                with self.subTest(clip=clip):
                    expected = self.reference(renderer, root, points)
                    actual = renderer.ink(root, common + f'div{{clip-path:{clip}}}', '<div><span></span></div>')
                    self.assertTrue(actual == expected)

    def test_concave_outline_offsets_match_independent_rectangular_rings(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        # Source box [20,20]-[120,100], two opposite half-box notches.
        outer = [(60,10),(130,10),(130,70),(80,70),(80,110),(10,110),(10,50),(60,50),(60,10)]
        inner = [(64,14),(126,14),(126,66),(76,66),(76,106),(14,106),(14,54),(64,54),(64,14)]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-corner-outline-') as directory:
            root = Path(directory)
            expected = self.reference(renderer, root, outer + inner + [outer[0]], (140,120))
            actual = renderer.ink(root,
                '.stage{padding:20pt;width:100pt;height:80pt}'
                '.box{width:100pt;height:80pt;corner-shape:notch square;'
                'border-radius:50%;outline:4pt solid red;outline-offset:6pt}',
                '<div class=stage><div class=box></div></div>')
            self.assertTrue(actual == expected, 'outlines must offset the original contour')


if __name__ == '__main__':
    unittest.main()
