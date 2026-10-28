"""Rendered checks for outlines, blending and individual transform properties."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf

BODY = '<div class=box>Box</div>'
BOX = '.box{width:80pt;height:40pt;margin:60pt;background:teal;color:white}'


class VisualEffectsPdf(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix='ferri-pdf-visual-effects-')
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def pixels(self, css, body=BODY):
        output = self.renderer.render(self.root, css, body)
        with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
            return raster.copy()

    def test_individual_transforms_paint_as_the_equivalent_transform(self):
        untransformed = self.pixels(BOX).tobytes()
        for individual, transform in (
                ('translate:12pt 5pt;rotate:20deg;scale:1.5 .5',
                 'transform:translate(12pt,5pt) rotate(20deg) scale(1.5,.5)'),
                ('translate:10%;transform:rotate(-15deg) skewX(10deg)',
                 'transform:translateX(10%) rotate(-15deg) skewX(10deg)'),
                ('rotate:y 50deg;transform-origin:0 0',
                 'transform:rotateY(50deg);transform-origin:0 0')):
            with self.subTest(individual=individual):
                expected = self.pixels(BOX + f'.box{{{transform}}}').tobytes()
                self.assertNotEqual(expected, untransformed, f'{transform} must move ink')
                self.assertEqual(self.pixels(BOX + f'.box{{{individual}}}').tobytes(), expected,
                                 f'{individual} must paint exactly as {transform}')


if __name__ == '__main__':
    unittest.main()
