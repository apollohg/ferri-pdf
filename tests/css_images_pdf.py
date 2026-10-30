"""Rendered CSS Images 4 `<image>` values compared, through Poppler, with equivalent documents
that draw the same ink by other means."""
import argparse
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from PIL import Image
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / 'target/debug/ferri-pdf'
FONT = ROOT / 'tests/oracle/assets/DejaVuSans.ttf'
RASTER_DPI = 72
PAGE_SIZE = 300
PAGE_MARGIN = 20
BOX = 80
FONT_SIZE = 10
LINE_HEIGHT = 14
WHITE = (255, 255, 255)


class CssImagesPdf(unittest.TestCase):
    def render(self, root, css, body):
        source = root / 'input.html'
        output = root / f'output-{len(list(root.glob("output-*.pdf")))}.pdf'
        source.write_text(f'''<!doctype html><style>
@font-face{{font-family:Probe;src:url('{FONT.as_uri()}')}}
@page{{size:{PAGE_SIZE}pt {PAGE_SIZE}pt;margin:{PAGE_MARGIN}pt}}
body{{margin:0;font:{FONT_SIZE}pt/{LINE_HEIGHT}pt Probe}}p,ul,li{{margin:0;padding:0}}
.box{{width:{BOX}pt;height:{BOX}pt}}{css}</style>{body}''')
        result = subprocess.run([str(RENDERER), str(source), str(output)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, f'render failed for {css!r}: {result.stderr}')
        return output

    def raster(self, root, output):
        subprocess.run(['pdftoppm', '-r', str(RASTER_DPI), '-aaVector', 'no', '-singlefile', '-png',
                        str(output), str(root / 'ink')], check=True, capture_output=True)
        with Image.open(root / 'ink.png') as image:
            return image.convert('RGB')

    def assertSameInk(self, root, actual, expected, message):
        """Both documents must rasterize to identical pixels."""
        actual_pixels = self.raster(root, self.render(root, *actual))
        expected_pixels = self.raster(root, self.render(root, *expected))
        if actual_pixels.tobytes() != expected_pixels.tobytes():
            differing = [(x, y) for y in range(actual_pixels.height) for x in range(actual_pixels.width)
                         if actual_pixels.getpixel((x, y)) != expected_pixels.getpixel((x, y))]
            first = differing[0]
            self.fail(f'{message}: {len(differing)} pixels differ, first at {first}: '
                      f'{actual_pixels.getpixel(first)} instead of {expected_pixels.getpixel(first)}\n'
                      f'actual: {actual}\nexpected: {expected}')

    def test_unregistered_paint_images_are_invalid_in_every_image_context(self):
        """CSS Painting API 1 §7: no worklet registers a name, so paint() is an invalid image
        (CSS Images 4 §2.1): transparent, or `none` where the context handles errors."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-paint-') as directory:
            root = Path(directory)
            box = '<div class=box></div>'
            self.assertSameInk(root, ('.box{background:paint(ripple) blue}', box),
                               ('.box{background:blue}', box),
                               'a background layer of an invalid image paints nothing')
            self.assertSameInk(root, ('.box{border:6pt dashed green;border-image:paint(ripple) 10}', box),
                               ('.box{border:6pt dashed green}', box),
                               'an invalid border image draws the border styles')
            self.assertSameInk(root, ('.box{background:blue;mask-image:paint(ripple)}', box),
                               ('', ''),
                               'an invalid mask layer is transparent black and hides the element')
            self.assertSameInk(root, ('.box{background:blue;mask-image:linear-gradient(black,black),'
                                      'paint(ripple);mask-composite:intersect}', box),
                               ('', ''),
                               'intersecting with an invalid mask layer leaves nothing')
            self.assertSameInk(root, ('.box{background:blue;mask-image:linear-gradient(black,black),'
                                      'paint(ripple)}', box),
                               ('.box{background:blue}', box),
                               'adding an invalid mask layer keeps the other layer')
            self.assertSameInk(root, ('.box{background:blue;border:4pt solid;mask-border:paint(ripple) 10}', box),
                               ('.box{background:blue;border:4pt solid}', box),
                               'an invalid mask border image does not mask')
            items = '<ul><li>One</li><li>Two</li></ul>'
            self.assertSameInk(root, ('li{list-style:inside decimal paint(ripple)}', items),
                               ('li{list-style:inside decimal}', items),
                               'an invalid list-style-image falls back to the list-style-type')
            text = 'aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt uu vv'
            self.assertSameInk(root, ('.f{float:left;width:60pt;height:60pt;shape-outside:paint(ripple)}',
                                      f'<div class=f></div><p>{text}</p>'),
                               ('', f'<p>{text}</p>'),
                               'a transparent invalid shape image leaves an empty float area')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--renderer', type=Path, default=RENDERER)
    arguments, remaining = parser.parse_known_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[sys.argv[0], *remaining])
