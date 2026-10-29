"""Rendered inline SVG styled by the document's cascade (SVG 2 §6.4): class rules, presentation
attributes, inheritance from HTML, `use` instances of a sprite, and the isolation of SVG images
from the document's style sheets, read back through Poppler."""
from pathlib import Path
import subprocess
import tempfile
import unittest
from urllib.parse import quote

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / 'target/debug/ferri-pdf'
RASTER_DPI = 72
PAGE_MARGIN = 20
ICON = 40
COLOR_TOLERANCE = 3
RED = (255, 0, 0)
GREEN = (0, 160, 0)
BLUE = (0, 0, 255)
YELLOW = (255, 255, 0)
MAGENTA = (200, 0, 200)
WHITE = (255, 255, 255)
SPRITE = (f'<svg style="display:none"><symbol id="dot" viewBox="0 0 {ICON} {ICON}">'
          f'<rect width="{ICON}" height="{ICON}" fill="currentColor"/></symbol></svg>')


class SvgCssPdf(unittest.TestCase):
    def render(self, root, css, body):
        source = root / 'input.html'
        output = root / 'output.pdf'
        source.write_text(f'''<!doctype html><style>
@page{{size:300pt 300pt;margin:{PAGE_MARGIN}pt}}
body{{margin:0;line-height:0}}svg,img{{display:block}}{css}</style>{body}''')
        completed = subprocess.run([str(RENDERER), str(source), str(output)], capture_output=True,
                                   text=True)
        self.assertEqual(completed.returncode, 0, f'render failed: {completed.stderr}')
        subprocess.run(['pdftoppm', '-r', str(RASTER_DPI), '-aaVector', 'no', '-singlefile', '-png',
                        str(output), str(root / 'ink')], check=True, capture_output=True)
        with Image.open(root / 'ink.png') as image:
            return image.convert('RGB')

    def assertPixel(self, pixels, x, y, expected, label):
        actual = pixels.getpixel((PAGE_MARGIN + x, PAGE_MARGIN + y))
        self.assertTrue(all(abs(a - e) <= COLOR_TOLERANCE for a, e in zip(actual, expected)),
                        f'{label}: rendered {actual} at ({x}, {y}) pt, expected {expected}')

    def test_class_rules_beat_presentation_attributes_and_inheritance_reaches_svg(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            pixels = self.render(
                Path(directory),
                '.icon .on{fill:rgb(0,160,0)} .icon{color:rgb(200,0,200);fill:rgb(0,0,255)}',
                f'<div class="icon"><svg width="{4 * ICON}pt" height="{ICON}pt" '
                f'viewBox="0 0 {4 * ICON} {ICON}">'
                f'<rect class="on" width="{ICON}" height="{ICON}" fill="red"/>'
                f'<rect x="{ICON}" width="{ICON}" height="{ICON}" fill="yellow"/>'
                f'<rect x="{2 * ICON}" width="{ICON}" height="{ICON}"/>'
                f'<rect x="{3 * ICON}" width="{ICON}" height="{ICON}" fill="currentColor"/>'
                '</svg></div>')
            half = ICON // 2
            self.assertPixel(pixels, half, half, GREEN, 'a class rule beats the fill attribute')
            self.assertPixel(pixels, ICON + half, half, YELLOW,
                             'the fill attribute beats the inherited fill')
            self.assertPixel(pixels, 2 * ICON + half, half, BLUE,
                             'fill inherits from the HTML ancestor')
            self.assertPixel(pixels, 3 * ICON + half, half, MAGENTA,
                             'currentColor is the color inherited from HTML')

    def test_sprite_instances_take_the_color_around_each_use_element(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            icon = f'<svg width="{ICON}pt" height="{ICON}pt"><use href="#dot"/></svg>'
            pixels = self.render(
                Path(directory), '.ok{color:rgb(0,160,0)} .bad{color:rgb(255,0,0)}',
                f'{SPRITE}<div class="ok">{icon}</div><div class="bad">{icon}</div>')
            half = ICON // 2
            self.assertPixel(pixels, half, half, GREEN, 'the first instance of the hidden sprite')
            self.assertPixel(pixels, half, ICON + half, RED, 'the second instance of the sprite')

    def test_svg_style_elements_apply_to_the_whole_document(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            pixels = self.render(
                Path(directory), '',
                '<svg width="0" height="0"><style>.panel{background:rgb(0,0,255)}</style></svg>'
                f'<div class="panel" style="width:{ICON}pt;height:{ICON}pt"></div>')
            self.assertPixel(pixels, ICON // 2, ICON // 2, BLUE,
                             'SVG 2 §6.4: an SVG style element styles HTML content too')

    def test_svg_images_keep_their_own_styles(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            image = quote(f'<svg xmlns="http://www.w3.org/2000/svg" width="{ICON}" height="{ICON}">'
                          f'<rect width="{ICON}" height="{ICON}" fill="blue"/></svg>')
            pixels = self.render(
                Path(directory), 'rect{fill:red} img{width:40pt;height:40pt}',
                f'<img src="data:image/svg+xml,{image}">'
                f'<div style="width:{ICON}pt;height:{ICON}pt;background:url(\'data:image/svg+xml,'
                f'{image}\') 0 0 / 100% 100%"></div>')
            half = ICON // 2
            self.assertPixel(pixels, half, half, BLUE,
                             'document rules do not reach an SVG image element')
            self.assertPixel(pixels, half, ICON + half, BLUE,
                             'document rules do not reach an SVG CSS image')


if __name__ == '__main__':
    unittest.main()
