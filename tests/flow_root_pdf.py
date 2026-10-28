"""Compare an auto-height formatting root against independently fixed geometry."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf
from oracle import pdf_content


class FlowRootPdf(unittest.TestCase):
    def test_auto_height_contains_float_but_preserves_visible_overflow(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-flow-root-') as directory:
            root = Path(directory)
            renderer = table_positioning_pdf.TablePositioningPdf()
            css = ('p{margin:0}.root{width:80pt;background:#eee}'
                   '.float{float:left;width:40pt;height:48pt;background:blue}'
                   '.overflow{width:120pt;height:12pt;background:red}')
            body = ('<div class=root><div class=float></div><div class=overflow></div></div>'
                    '<p><a href="mailto:reader@example.org">After</a></p>')
            reference = renderer.render(root, css + '.root{height:48pt}', body)
            glyphs = pdf_content.inspect(reference)['glyphs']
            links = pdf_content.navigation(reference)['links']
            with renderer.raster(root, reference, vector_antialias=False) as raster:
                pixels = raster.tobytes()
                self.assertEqual(raster.getpixel((130, 25)), (255, 0, 0),
                                 'reference must visibly overflow the 80pt root')
            for display in ('flow-root', 'block flow-root'):
                with self.subTest(display=display):
                    actual = renderer.render(root, css + f'.root{{display:{display}}}', body)
                    self.assertEqual(pdf_content.inspect(actual)['glyphs'], glyphs)
                    self.assertEqual(pdf_content.navigation(actual)['links'], links)
                    with renderer.raster(root, actual, vector_antialias=False) as raster:
                        self.assertEqual(raster.tobytes(), pixels)
