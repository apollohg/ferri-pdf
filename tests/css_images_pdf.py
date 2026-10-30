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
# One CSS pixel per raster pixel, so images at their natural size are not resampled.
RASTER_DPI = 96
PAGE_SIZE = 300
PAGE_MARGIN = 24
BOX = 80
FONT_SIZE = 10
LINE_HEIGHT = 14
WHITE = (255, 255, 255)
RED = (255, 0, 0)
BLUE = (0, 0, 255)
GREEN = (0, 128, 0)
YELLOW = (255, 255, 0)
TRANSPARENT = (0, 0, 0, 0)
QUADRANTS = [[RED, BLUE], [GREEN, YELLOW]]
POINTS_PER_INCH = 72
# JPEG blocks decode within a few levels of their colour.
COLOR_TOLERANCE = 4
# EXIF orientation tag and its value for an image to turn a quarter turn clockwise.
EXIF_ORIENTATION = 0x0112
EXIF_ROTATE_90 = 6
# Pixel width of the colour blocks in test images.
QUADRANT = 10
TEXT = ' '.join(['aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt'] * 3)


def turned(rows):
    """Colour blocks turned a quarter turn clockwise."""
    return [list(row) for row in zip(*rows[::-1])]


def flipped(rows):
    """Colour blocks flipped horizontally."""
    return [row[::-1] for row in rows]


def blocks(path, rows):
    """A PNG of QUADRANT-pixel colour blocks, one row of blocks per item of `rows`."""
    image = Image.new('RGBA', (QUADRANT * len(rows[0]), QUADRANT * len(rows)))
    for row, colors in enumerate(rows):
        for column, color in enumerate(colors):
            image.paste(color, (column * QUADRANT, row * QUADRANT,
                                (column + 1) * QUADRANT, (row + 1) * QUADRANT))
    image.save(path)
    return path.as_uri()


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

    def assertBlocks(self, root, css, body, rows, message):
        """The page shows an image of colour blocks `rows` at the page area's top left, one raster
        pixel per image pixel, and white just past its last row and column. Block centres are
        sampled because viewers smooth image pixels along block edges differently when an
        image is flipped or clipped."""
        pixels = self.raster(root, self.render(root, '.box{background-color:white}' + css, body))
        origin = PAGE_MARGIN * RASTER_DPI // POINTS_PER_INCH
        centre = QUADRANT // 2
        expected = {(column, row): color for row, colors in enumerate(rows)
                    for column, color in enumerate(colors)}
        expected[(len(rows[0]), 0)] = WHITE
        expected[(0, len(rows))] = WHITE
        for (column, row), color in expected.items():
            point = (origin + column * QUADRANT + centre, origin + row * QUADRANT + centre)
            actual = pixels.getpixel(point)
            self.assertTrue(all(abs(a - b) <= COLOR_TOLERANCE for a, b in zip(actual, color)),
                            f'{message}: block {column},{row} at {point} is {actual}, not {color[:3]}; {css}')

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

    def test_image_function_colors_fallbacks_and_fragments(self):
        """CSS Images 4 §2.5: solid colors, fallback colors for unavailable URLs, and the
        regions xywh media fragments select."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-image-function-') as directory:
            root = Path(directory)
            quadrants = blocks(root / 'quadrants.png', QUADRANTS)
            box = '<div class=box></div>'
            translucent = 'rgb(0 0 255 / 50%)'
            self.assertSameInk(
                root, (f'.box{{background:image({translucent}) 10pt 15pt / 30pt 20pt no-repeat,'
                       'linear-gradient(red,red)}', box),
                (f'.box{{background:linear-gradient({translucent},{translucent}) 10pt 15pt / 30pt 20pt '
                 'no-repeat,linear-gradient(red,red)}', box),
                'a solid-color image is a layer with no natural dimensions over the layers below')
            self.assertSameInk(root, ('.box{background-image:image("missing.png", blue)}', box),
                               ('.box{background-image:image(blue)}', box),
                               'an unavailable URL renders the fallback color')
            self.assertSameInk(root, ('.box{background:image("missing.png") red}', box),
                               ('.box{background:red}', box),
                               'an unavailable URL without a fallback is an invalid image')
            for fragment, rows, message in [
                    ('#xywh=10,0,10,10', [[BLUE]], 'a pixel fragment selects one block'),
                    ('#xywh=pixel:0,10,10,10', [[GREEN]], 'the pixel unit is explicit'),
                    ('#xywh=percent:0,50,100,50', [[GREEN, YELLOW]], 'a percent fragment selects a row'),
                    ('#t=5&xywh=10,10,50,50', [[YELLOW]], 'a region past the image is clipped to it')]:
                self.assertBlocks(root, f'.box{{background:image("{quadrants}{fragment}") 0 0 no-repeat}}',
                                  box, rows, message)
            for fragment in ('#foo', '#xywh=0,0,0,5', '#xywh=20,0,5,5', '#xywh=em:0,0,5,5',
                             '#xywh=1,2,3'):
                self.assertSameInk(root, (f'.box{{background:image("{quadrants}{fragment}") red}}', box),
                                   ('.box{background:red}', box),
                                   f'image() with fragment {fragment} is an invalid image')
                self.assertSameInk(root, (f'.box{{background:image("{quadrants}{fragment}", blue) red}}', box),
                                   ('.box{background:image(blue) red}', box),
                                   f'image() with fragment {fragment} renders its fallback')
                self.assertSameInk(root, (f'.box{{background:url("{quadrants}{fragment}") 0 0 no-repeat}}', box),
                                   (f'.box{{background:url("{quadrants}") 0 0 no-repeat}}', box),
                                   f'url() ignores fragment {fragment}')
            self.assertBlocks(root, f'.box{{background:url("{quadrants}#xywh=10,0,10,10") 0 0 no-repeat}}',
                              box, [[BLUE]], 'url() applies a fragment it understands')
            # A ten-pixel slice of the ten-pixel region fills the corner of a ten-pixel border.
            self.assertBlocks(root, f'.box{{border:7.5pt solid;border-image:image("{quadrants}#xywh=0,10,10,10") 10}}',
                              box, [[GREEN]], 'border images slice the selected region')

    def test_directional_images_flip_against_the_element_direction(self):
        """CSS Images 4 §2.5.4: an image tagged ltr or rtl flips in the inline direction on an
        element of the opposite direction, horizontally or, in vertical writing, vertically."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-image-direction-') as directory:
            root = Path(directory)
            quadrants = blocks(root / 'quadrants.png', QUADRANTS)
            box = '<div class=box></div>'
            mirrored = [[BLUE, RED], [YELLOW, GREEN]]
            for tag, direction, rows in [('ltr', 'ltr', QUADRANTS), ('rtl', 'ltr', mirrored),
                                         ('ltr', 'rtl', mirrored), ('rtl', 'rtl', QUADRANTS)]:
                self.assertBlocks(root, f'.box{{direction:{direction};background:image({tag} "{quadrants}") 0 0 no-repeat}}',
                                  box, rows, f'image({tag}) on a {direction} element')
            self.assertBlocks(root, f'.box{{direction:rtl;background:url("{quadrants}") 0 0 no-repeat}}',
                              box, QUADRANTS, 'url() has no directionality')
            self.assertBlocks(root, f'.box{{writing-mode:vertical-rl;direction:rtl;'
                                    f'background:image(ltr "{quadrants}") 0 0 no-repeat}}',
                              box, [[GREEN, YELLOW], [RED, BLUE]],
                              'a vertical inline axis flips the image vertically')
            self.assertBlocks(root, f'.box{{direction:rtl;background:image(ltr "{quadrants}#xywh=0,0,20,10") 0 0 no-repeat}}',
                              box, [[BLUE, RED]], 'the selected region flips as a whole')
            for direction, order in [('ltr', [RED, BLUE]), ('rtl', [BLUE, RED])]:
                output = self.render(root, f'ul{{padding:0 40pt}}li{{direction:{direction};'
                                           f'list-style-image:image(ltr "{quadrants}")}}',
                                     '<ul><li>Item</li></ul>')
                pixels = self.raster(root, output)
                columns = {color: min(x for y in range(pixels.height) for x in range(pixels.width)
                                      if pixels.getpixel((x, y)) == color) for color in order}
                self.assertLess(columns[order[0]], columns[order[1]],
                                f'the {direction} list item marker shows {order} from left to right: {columns}')

    def test_shape_images_use_fragments_direction_and_fallbacks(self):
        """CSS Shapes 1 §4 float areas from image() alpha: the selected region, its flip, and
        `none` for an unavailable URL without a fallback color (§6.1)."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-image-shapes-') as directory:
            root = Path(directory)
            half = blocks(root / 'half.png', [[RED, TRANSPARENT]])
            body = f'<div class=f></div><p>{TEXT}</p>'
            float_box = '.f{float:left;width:80pt;height:40pt;'
            for shape, expected in [
                    (f'image("{half}")', 'inset(0 50% 0 0)'),
                    (f'image(rtl "{half}")', 'inset(0 0 0 50%)'),
                    (f'image("{half}#xywh=10,0,10,10")', 'inset(50%)'),
                    (f'image("{half}#xywh=0,0,10,10")', 'content-box'),
                    ('image("missing.png")', 'none'),
                    ('image("missing.png", red)', 'content-box'),
                    ('image(transparent)', 'inset(50%)')]:
                self.assertSameInk(root, (f'{float_box}shape-outside:{shape}}}', body),
                                   (f'{float_box}shape-outside:{expected}}}', body),
                                   f'shape-outside: {shape} wraps like {expected}')

    def test_image_orientation_turns_content_images_and_unrotates_decorative_ones(self):
        """CSS Images 3 §5.1: angles round to quarter turns, flip follows the turn, both apply
        to content images only; none ignores EXIF orientation in content and decorative images."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-image-orientation-') as directory:
            root = Path(directory)
            grid = [[RED, BLUE], [GREEN, YELLOW], [BLUE, BLUE]]
            stored = blocks(root / 'grid.png', grid)
            body = f'<img src="{stored}">'
            for value, rows in [('from-image', grid), ('none', grid), ('0deg', grid),
                                ('90deg', turned(grid)), ('45deg', turned(grid)),
                                ('-45deg', grid), ('-90deg', turned(turned(turned(grid)))),
                                ('450deg', turned(grid)), ('0.5turn', turned(turned(grid))),
                                ('flip', flipped(grid)), ('90deg flip', flipped(turned(grid))),
                                ('flip 270deg', flipped(turned(turned(turned(grid)))))]:
                self.assertBlocks(root, f'img{{display:block;image-orientation:{value}}}', body, rows,
                                  f'image-orientation: {value}')
            self.assertBlocks(root, '.box{image-orientation:90deg flip;'
                                    f'background:url("{stored}") 0 0 no-repeat}}',
                              '<div class=box></div>', grid, 'angles do not turn decorative images')
            self.assertBlocks(root, 'div{image-orientation:90deg}img{display:block}', f'<div>{body}</div>',
                              turned(grid), 'the property inherits')

            exif = Image.new('RGB', (2 * QUADRANT, 3 * QUADRANT))
            for row, colors in enumerate(grid):
                for column, color in enumerate(colors):
                    exif.paste(color, (column * QUADRANT, row * QUADRANT,
                                       (column + 1) * QUADRANT, (row + 1) * QUADRANT))
            metadata = exif.getexif()
            metadata[EXIF_ORIENTATION] = EXIF_ROTATE_90
            exif.save(root / 'exif.jpg', quality=100, exif=metadata)
            photo = (root / 'exif.jpg').as_uri()
            for value, rows in [('from-image', turned(grid)), ('none', grid),
                                ('flip', flipped(grid)), ('180deg', turned(turned(grid)))]:
                self.assertBlocks(root, f'img{{display:block;image-orientation:{value}}}',
                                  f'<img src="{photo}">', rows, f'EXIF photo with image-orientation: {value}')
            for value, rows in [('from-image', turned(grid)), ('none', grid), ('flip', turned(grid))]:
                self.assertBlocks(root, f'.box{{image-orientation:{value};background:url("{photo}") 0 0 no-repeat}}',
                                  '<div class=box></div>', rows, f'EXIF background with image-orientation: {value}')
            self.assertBlocks(root, '.box{direction:rtl;background:image(ltr "' + photo +
                              '#xywh=0,0,30,10") 0 0 no-repeat}', '<div class=box></div>',
                              [[RED, GREEN, BLUE]], 'fragments select from the EXIF-oriented image, then flip')

    def test_object_view_box_replaces_natural_size_with_its_rectangle(self):
        """CSS Images 5 §3.1: the view box resolves against the natural size, becomes the natural
        size, and shows the content that falls inside it, transparent where it reaches past."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-object-view-box-') as directory:
            root = Path(directory)
            grid = [[RED, BLUE], [GREEN, YELLOW], [BLUE, BLUE]]
            stored = blocks(root / 'grid.png', grid)
            body = f'<img src="{stored}">'
            for value, rows in [
                    ('none', grid),
                    ('inset(0 10px 20px 0)', [[RED]]),
                    ('inset(0 50% 66.67% 0 round 5px)', [[RED]]),
                    ('xywh(10px 10px 10px 20px)', [[YELLOW], [BLUE]]),
                    ('rect(10px auto auto 0)', [[GREEN, YELLOW], [BLUE, BLUE]]),
                    ('inset(-10px 0 0 0)', [[WHITE, WHITE], *grid])]:
                self.assertBlocks(root, f'img{{display:block;object-view-box:{value}}}', body, rows,
                                  f'object-view-box: {value}')
            self.assertBlocks(root, 'img{display:block;image-orientation:90deg;object-view-box:inset(0 0 10px 0)}',
                              body, turned(grid)[:1], 'the view box applies to the oriented image')
            self.assertBlocks(root, 'img{display:block;width:10px;height:10px;object-view-box:xywh(5px 0 5px 5px)}',
                              f'<img srcset="{stored} 2x">', [[BLUE]],
                              'lengths are CSS pixels of the density-corrected natural size')
            pixels = self.raster(root, self.render(
                root, 'img{display:block;object-view-box:inset(0 10px 20px 0 round 5px)}', body))
            corner = PAGE_MARGIN * RASTER_DPI // POINTS_PER_INCH
            self.assertEqual(pixels.getpixel((corner, corner)), RED,
                             'a rounded view box does not clip its corners')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--renderer', type=Path, default=RENDERER)
    arguments, remaining = parser.parse_known_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[sys.argv[0], *remaining])
