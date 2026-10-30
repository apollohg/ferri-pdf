"""Rendered CSS Masking 1 mask layers: soft-mask structure, compositing operators, modes and
SVG `mask` references, read back through Poppler and the oracle's PDF parser."""
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / 'target/debug/ferri-pdf'
RASTER_DPI = 72
PAGE_MARGIN = 20
BOX = 80
QUARTER = BOX // 4
# Eight-bit soft-mask values and the viewer's compositing.
COLOR_TOLERANCE = 3
RED = (255, 0, 0)
WHITE = (255, 255, 255)
SOLID = 'linear-gradient(black, black)'
# The top layer covers the left half and the bottom layer the top half, so the quadrants are
# both layers (top left), the bottom layer only (top right), the top layer only (bottom left)
# and neither (bottom right).
HALVES = f'{SOLID} 0 0 / 50% 100% no-repeat, {SOLID} 0 0 / 100% 50% no-repeat'
QUADRANTS = {'both': (QUARTER, QUARTER), 'bottom only': (3 * QUARTER, QUARTER),
             'top only': (QUARTER, 3 * QUARTER), 'neither': (3 * QUARTER, 3 * QUARTER)}
OPERATORS = {
    'add': {'both', 'bottom only', 'top only'},
    'subtract': {'top only'},
    'intersect': {'both'},
    'exclude': {'bottom only', 'top only'},
}


def soft_masks(path):
    document = pdf_content.Document(path)
    found = []
    for number in sorted(set(document.offsets) | set(document.compressed)):
        value = document.get(number)
        entries = value.entries if isinstance(value, pdf_content.Stream) else value
        if isinstance(entries, dict) and entries.get('Type') == 'Mask':
            found.append(entries)
    return found


class MasksPdf(unittest.TestCase):
    def render(self, root, css, body):
        source = root / 'input.html'
        output = root / 'output.pdf'
        source.write_text(f'''<!doctype html><style>
@page{{size:300pt 300pt;margin:{PAGE_MARGIN}pt}}
body{{margin:0}}div{{margin:0}}.box{{width:{BOX}pt;height:{BOX}pt;background:red}}{css}</style>{body}''')
        subprocess.run([str(RENDERER), str(source), str(output)], check=True, capture_output=True)
        return output

    def raster(self, root, output):
        subprocess.run(['pdftoppm', '-r', str(RASTER_DPI), '-aaVector', 'no', '-singlefile', '-png',
                        str(output), str(root / 'ink')], check=True, capture_output=True)
        with Image.open(root / 'ink.png') as image:
            return image.convert('RGB')

    def pixel(self, pixels, x, y):
        return pixels.getpixel((PAGE_MARGIN + x, PAGE_MARGIN + y))

    def assertNear(self, actual, expected, label):
        self.assertTrue(all(abs(a - e) <= COLOR_TOLERANCE for a, e in zip(actual, expected)),
                        f'{label}: rendered {actual}, expected {expected}')

    def test_mask_is_a_luminosity_soft_mask_over_vector_content(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-masks-') as directory:
            root = Path(directory)
            output = self.render(root, '.box{mask-image:linear-gradient(to right, black, transparent)}',
                                 '<div class=box>Masked words</div>')
            masks = soft_masks(output)
            self.assertTrue(any(mask.get('S') == 'Luminosity' for mask in masks),
                            f'the mask layers form one luminosity soft mask: {masks}')
            text = subprocess.run(['pdftotext', str(output), '-'], check=True,
                                  capture_output=True, text=True).stdout
            self.assertIn('Masked words', text, 'masking keeps text vector')
            pixels = self.raster(root, output)
            for x in (0, BOX // 4, BOX // 2, BOX - 1):
                # The gradient's alpha at the pixel centre is the mask value.
                cleared = round(255 * (x + 0.5) / BOX)
                self.assertNear(self.pixel(pixels, x, BOX - 5), (255, cleared, cleared),
                                f'alpha mask value {1 - (x + 0.5) / BOX:.3f} at {x}pt')

    def test_compositing_operators_combine_layers_by_porter_duff_rules(self):
        for operator, visible in OPERATORS.items():
            with self.subTest(operator=operator), \
                    tempfile.TemporaryDirectory(prefix='ferri-pdf-masks-') as directory:
                root = Path(directory)
                output = self.render(root, f'.box{{mask:{HALVES};mask-composite:{operator}}}',
                                     '<div class=box></div>')
                pixels = self.raster(root, output)
                for quadrant, (x, y) in QUADRANTS.items():
                    expected = RED if quadrant in visible else WHITE
                    self.assertNear(self.pixel(pixels, x, y), expected,
                                    f'mask-composite:{operator} in the {quadrant} quadrant')

    def test_luminance_mode_scales_by_luminance_and_alpha_mode_ignores_colour(self):
        grey = 'linear-gradient(rgb(128,128,128), rgb(128,128,128))'
        for mode, expected in (('luminance', (255, 127, 127)), ('alpha', RED),
                               ('match-source', RED)):
            with self.subTest(mode=mode), \
                    tempfile.TemporaryDirectory(prefix='ferri-pdf-masks-') as directory:
                root = Path(directory)
                output = self.render(root, f'.box{{mask:{grey} {mode}}}', '<div class=box></div>')
                pixels = self.raster(root, output)
                self.assertNear(self.pixel(pixels, BOX // 2, BOX // 2), expected,
                                f'mask-mode:{mode} of mid grey')

    def test_mask_clip_and_origin_place_layers_on_the_box_areas(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-masks-') as directory:
            root = Path(directory)
            output = self.render(
                root, '.box{padding:10pt;width:60pt;height:60pt;'
                      f'mask:{SOLID} content-box}}', '<div class=box></div>')
            pixels = self.raster(root, output)
            self.assertNear(self.pixel(pixels, 5, 40), WHITE, 'padding outside mask-clip:content-box')
            self.assertNear(self.pixel(pixels, 15, 40), RED, 'content box inside the clip')
            unclipped = self.render(root, f'.box{{padding:10pt;width:60pt;height:60pt;mask:{SOLID} '
                                          'content-box no-clip}', '<div class=box></div>')
            self.assertNear(self.pixel(self.raster(root, unclipped), 5, 40), RED,
                            'no-clip lets the repeated layer reach the padding')
            unrepeated = self.render(root, f'.box{{padding:10pt;width:60pt;height:60pt;mask:{SOLID} '
                                           'content-box no-clip no-repeat}', '<div class=box></div>')
            self.assertNear(self.pixel(self.raster(root, unrepeated), 5, 40), WHITE,
                            'one tile sized to the content box leaves the padding unmasked-in')

    def test_image_layers_mask_by_their_pixels_alpha(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-masks-') as directory:
            root = Path(directory)
            # Left half opaque white, right half transparent.
            image = Image.new('RGBA', (10, 10), (255, 255, 255, 0))
            image.paste((255, 255, 255, 255), (0, 0, 5, 10))
            image.save(root / 'half.png')
            output = self.render(root, '.box{mask:url(half.png) 0 0 / 100% 100%}',
                                 '<div class=box></div>')
            pixels = self.raster(root, output)
            self.assertNear(self.pixel(pixels, QUARTER, BOX // 2), RED, 'opaque image half')
            self.assertNear(self.pixel(pixels, 3 * QUARTER, BOX // 2), WHITE,
                            'transparent image half')

    def test_mask_border_slices_like_a_border_image_and_needs_fill_for_the_middle(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-masks-') as directory:
            root = Path(directory)
            Image.new('RGBA', (30, 30), (255, 255, 255, 255)).save(root / 'opaque.png')
            for slice_value, middle in (('10', WHITE), ('10 fill', RED)):
                with self.subTest(slice=slice_value):
                    output = self.render(
                        root, f'.box{{mask-border:url(opaque.png) {slice_value} / 20pt}}',
                        '<div class=box></div>')
                    pixels = self.raster(root, output)
                    self.assertNear(self.pixel(pixels, 10, BOX // 2), RED,
                                    f'mask-border-slice:{slice_value} edge piece')
                    self.assertNear(self.pixel(pixels, BOX // 2, BOX // 2), middle,
                                    f'mask-border-slice:{slice_value} middle piece')
            combined = self.render(
                root, f'.box{{mask-border:url(opaque.png) 10 fill / 20pt;'
                      f'mask:{SOLID} 0 0 / 50% 100% no-repeat}}', '<div class=box></div>')
            pixels = self.raster(root, combined)
            self.assertNear(self.pixel(pixels, 10, BOX // 2), RED, 'both masks keep the left edge')
            self.assertNear(self.pixel(pixels, BOX - 10, BOX // 2), WHITE,
                            'the mask layer removes the right half the mask border keeps')

    def test_sliced_inline_boxes_mask_and_clip_their_unbroken_box(self):
        # CSS Masking 1 §7.6: box-decoration-break decides the boxes the mask positioning area
        # covers; a sliced inline box is masked and clipped as if laid out unbroken, its two
        # 60pt fragments end to end, while a cloned one masks each fragment on its own.
        first_right, second_left = (45, 16), (15, 46)
        for effect in ('mask-image:linear-gradient(to right,black 50%,transparent 50%);'
                       'mask-repeat:no-repeat', 'clip-path:inset(0 50% 0 0)'):
            for breaking, first, second in (('slice', RED, WHITE), ('clone', WHITE, RED)):
                with self.subTest(effect=effect, breaking=breaking), \
                        tempfile.TemporaryDirectory(prefix='ferri-pdf-masks-') as directory:
                    root = Path(directory)
                    output = self.render(
                        root,
                        'div{font:20pt/30pt serif}i{display:inline-block;width:60pt;height:10pt;'
                        f'background:red}}span{{box-decoration-break:{breaking};{effect}}}',
                        '<div><span><i></i><br><i></i></span></div>')
                    pixels = self.raster(root, output)
                    self.assertNear(self.pixel(pixels, *first_right), first,
                                    f'{breaking} {effect}: right half of the first fragment')
                    self.assertNear(self.pixel(pixels, *second_left), second,
                                    f'{breaking} {effect}: left half of the second fragment')

    def test_svg_mask_reference_positions_itself_against_the_border_box(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-masks-') as directory:
            root = Path(directory)
            output = self.render(
                root, '.box{mask-image:url(#m)}',
                '<svg width=0 height=0 style=position:absolute><mask id=m '
                'maskContentUnits=objectBoundingBox><rect width=0.5 height=1 fill="white"/>'
                '</mask></svg><div class=box></div>')
            pixels = self.raster(root, output)
            self.assertNear(self.pixel(pixels, QUARTER, BOX // 2), RED, 'inside the mask rectangle')
            self.assertNear(self.pixel(pixels, 3 * QUARTER, BOX // 2), WHITE,
                            'outside the mask rectangle')
            missing = self.render(root, '.box{mask-image:url(#absent)}', '<div class=box></div>')
            self.assertNear(self.pixel(self.raster(root, missing), QUARTER, BOX // 2), WHITE,
                            'a reference naming no mask element is a transparent black layer')


if __name__ == '__main__':
    unittest.main()
