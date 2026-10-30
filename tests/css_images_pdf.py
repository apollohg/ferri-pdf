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
# EXIF resolution tags and the unit that makes them dots per inch.
EXIF_X_RESOLUTION = 0x011a
EXIF_Y_RESOLUTION = 0x011b
EXIF_RESOLUTION_UNIT = 0x0128
EXIF_INCHES = 2
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

    def assertSameInterior(self, root, actual, expected, message):
        """Every pixel of the expected document whose neighbours all share its colour has that
        colour in the actual one too: viewers smooth image edges differently by how an image
        is drawn, so edges are not compared."""
        actual_pixels = self.raster(root, self.render(root, *actual))
        expected_pixels = self.raster(root, self.render(root, *expected))
        width, height = expected_pixels.size
        differing = [(x, y) for y in range(1, height - 1) for x in range(1, width - 1)
                     if len({expected_pixels.getpixel((x + dx, y + dy))
                             for dx in (-1, 0, 1) for dy in (-1, 0, 1)}) == 1
                     and actual_pixels.getpixel((x, y)) != expected_pixels.getpixel((x, y))]
        if differing:
            first = differing[0]
            self.fail(f'{message}: {len(differing)} interior pixels differ, first at {first}: '
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

    def assertCentre(self, root, css, expected, message, tolerance=COLOR_TOLERANCE):
        """The centre of an 80pt box styled by `css` shows `expected` over the white page."""
        pixels = self.raster(root, self.render(root, f'.box{{{css}}}', '<div class=box></div>'))
        centre = (PAGE_MARGIN + BOX // 2) * RASTER_DPI // POINTS_PER_INCH
        actual = pixels.getpixel((centre, centre))
        self.assertTrue(all(abs(a - b) <= tolerance for a, b in zip(actual, expected)),
                        f'{message}: {actual} instead of {expected}; {css}')

    def test_cross_fade_averages_premultiplied_arguments(self):
        """CSS Images 4 §2.6.2: the weighted average, in premultiplied sRGB, of the arguments and
        of transparent black for the share they leave below 100%; shares summing past 100%
        scale down to it. One translucent argument composes in PDF, more are rasterized."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-cross-fade-') as directory:
            root = Path(directory)
            for image, expected, message in [
                    ('cross-fade(red 30%, blue)', (77, 0, 179), 'an omitted percentage takes the rest'),
                    ('cross-fade(red 100%, blue 100%)', (128, 0, 128), 'shares past 100% scale down'),
                    ('cross-fade(blue 50%)', (128, 128, 255), 'the leftover share is transparent'),
                    ('cross-fade(rgb(0 0 255 / 50%) 50%, red 50%)', (191, 64, 128),
                     'a translucent argument under an opaque one'),
                    ('cross-fade(red 50%, rgb(0 0 255 / 50%) 50%)', (191, 64, 128),
                     'argument order does not matter'),
                    ('cross-fade(rgb(255 0 0 / 50%), rgb(0 0 255 / 50%))', (191, 128, 191),
                     'two translucent arguments average without compounding'),
                    ('cross-fade(rgb(255 0 0) 40%, rgb(0 255 0 / .5) 20%, rgb(0 0 255 / 0) 40%)',
                     (230, 153, 128), 'the specification example is rgb(204 51 0 / .5)'),
                    ('cross-fade(linear-gradient(red, red) 25%, cross-fade(blue, lime) 75%)',
                     (64, 96, 96), 'gradients and nested cross-fades are images'),
                    ('cross-fade(image(red) 50%, image("missing.png") 50%)', (255, 128, 128),
                     'an invalid argument is transparent')]:
                self.assertCentre(root, f'background:{image}', expected, message)
            self.assertCentre(root, 'background:cross-fade(rgb(255 0 0 / 50%), rgb(0 0 255 / 50%));'
                                    'transform:scale(0.5)', (191, 128, 191),
                              'a rasterized cross-fade follows the drawing transform')
            self.assertCentre(root, 'background:cross-fade(black 50%) blue', (0, 0, 128),
                              'the result composites over the layers below')
            self.assertCentre(root, 'background:blue;mask-image:cross-fade(black 50%)', (128, 128, 255),
                              'a mask layer keeps half of the box')
            self.assertCentre(root, 'background:blue;mask-image:cross-fade(black 50%, white 25%)',
                              (64, 64, 255), 'an alpha mask layer averages the arguments\' alpha')
            self.assertCentre(root, 'background:blue;mask-image:cross-fade(black 50%, white 25%);'
                                    'mask-mode:luminance', (191, 191, 255),
                              'a luminance mask layer averages the arguments\' luminance')

    def test_cross_fade_sizes_by_its_images_in_every_context(self):
        """CSS Images 4 §2.6.1: the natural size is the weighted average of the arguments with
        natural dimensions; colours do not take part. Each argument fills the result."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-cross-fade-size-') as directory:
            root = Path(directory)
            small = root / 'small.png'
            Image.new('RGB', (10, 10), RED).save(small)
            large = root / 'large.png'
            Image.new('RGB', (30, 50), BLUE).save(large)
            for image, expected in [
                    (f'cross-fade(url("{small.as_uri()}"), url("{large.as_uri()}"))', (20, 30)),
                    (f'cross-fade(url("{small.as_uri()}") 75%, url("{large.as_uri()}") 25%)', (15, 20)),
                    (f'cross-fade(url("{small.as_uri()}") 25%, lime)', (10, 10)),
                    (f'cross-fade(url("{small.as_uri()}") 50%, linear-gradient(red, blue))', (10, 10))]:
                self.assertInkSize(root, f'.box{{background:{image} no-repeat}}', '<div class=box></div>',
                                   expected, f'background {image}')
            quadrants = blocks(root / 'quadrants.png', QUADRANTS)
            self.assertBlocks(root, f'.box{{background:cross-fade(url("{quadrants}") 100%) 0 0 no-repeat}}',
                              '<div class=box></div>', QUADRANTS, 'a whole share draws the image itself')
            box = '<div class=box></div>'
            self.assertSameInk(root, (f'.box{{border:7.5pt solid;border-image:cross-fade(url("{quadrants}")) 10}}', box),
                               (f'.box{{border:7.5pt solid;border-image:url("{quadrants}") 10}}', box),
                               'border images slice the cross-fade')
            items = '<ul><li>One</li><li>Two</li></ul>'
            self.assertSameInk(root, (f'ul{{padding:0 40pt}}li{{list-style-image:cross-fade(url("{quadrants}"))}}', items),
                               (f'ul{{padding:0 40pt}}li{{list-style-image:url("{quadrants}")}}', items),
                               'list markers size and draw the cross-fade')
            body = f'<div class=f></div><p>{TEXT}</p>'
            float_box = '.f{float:left;width:80pt;height:40pt;shape-image-threshold:0.5;'
            for shape, expected in [('cross-fade(red 60%)', 'content-box'), ('cross-fade(red 40%)', 'inset(50%)'),
                                    ('cross-fade(rgb(0 0 0 / 50%), red)', 'content-box')]:
                self.assertSameInk(root, (f'{float_box}shape-outside:{shape}}}', body),
                                   (f'{float_box}shape-outside:{expected}}}', body),
                                   f'shape-outside: {shape} wraps like {expected}')

    def test_content_images_replace_elements_and_fill_generated_content(self):
        """CSS Content 3 §1: a single `<image>` makes an element or pseudo-element a replaced
        element; images in a content list are anonymous inline replaced elements, and a list
        on an element replaces its children."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-content-images-') as directory:
            root = Path(directory)
            quadrants = blocks(root / 'quadrants.png', QUADRANTS)
            img = f'<img src="{quadrants}">'
            for css, body, expected, message in [
                    (f'div{{content:url("{quadrants}")}}', '<div>Text</div>',
                     ('img{display:block}', img), 'an element shows its replacement image'),
                    (f'div{{content:url("{quadrants}");width:40px;height:30px;border:2px solid blue}}',
                     '<div>Text</div>',
                     ('img{display:block;width:40px;height:30px;border:2px solid blue}', img),
                     'a replaced element takes its box\'s sizes'),
                    (f'span{{content:image-set(url("{quadrants}") 2x)}}', 'a<span>Text</span>b',
                     ('', f'a<img srcset="{quadrants} 2x">b'), 'image-set() chooses the natural resolution'),
                    (f'span{{content:url("{quadrants}");object-view-box:inset(0 10px 10px 0)}}',
                     'a<span>Text</span>b',
                     ('img{object-view-box:inset(0 10px 10px 0)}', f'a{img}b'),
                     'the element\'s object-view-box applies'),
                    ('div{content:linear-gradient(red, red)}', '<div>Text</div>',
                     ('div{width:300px;height:150px;background:red}', '<div></div>'),
                     'a generated image takes the default object size'),
                    ('div{content:image("missing.png");border:2px solid}', '<div>Text</div>',
                     ('div{width:0;height:0;border:2px solid}', '<div></div>'),
                     'an invalid replacement has zero natural size'),
                    (f'div{{content:"A" url("{quadrants}") "B"}}', '<div>Text <b>more</b></div>',
                     ('', f'<div>A{img}B</div>'), 'a content list replaces an element\'s children'),
                    ('div{content:none}', '<div>Text</div>', ('', '<div>Text</div>'),
                     'none on an element renders it normally'),
                    (f'p::before{{content:"x" url("{quadrants}") image("missing.png") "y"}}', '<p>Text</p>',
                     ('', f'<p>x{img}yText</p>'), 'list images are inline and invalid ones are skipped'),
                    (f'p::before{{content:url("{quadrants}");width:40px;height:30px}}', '<p>Text</p>',
                     ('img{width:40px;height:30px}', f'<p>{img}Text</p>'),
                     'a single image makes a pseudo-element a replaced element'),
                    (f'p::after{{content:url("{quadrants}");display:block}}', '<p>Text</p>',
                     ('img{display:block}', f'<p>Text</p>{img}'), 'a block-level replaced pseudo-element'),
                    ('p::before{content:cross-fade(red, blue) "x"}', '<p>Text</p>',
                     ('img{width:300px;height:150px;background:cross-fade(red, blue)}',
                      '<p><img src="data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22/%3E">xText</p>'),
                     'generated images in content lists')]:
                self.assertSameInk(root, (css, body), expected, message)
            # CSS Lists 3 §3.2: a marker image is followed by a space.
            items = '<ul><li>One</li></ul>'
            for position in ('outside', 'inside'):
                self.assertSameInterior(
                    root, (f'ul{{padding-left:60pt}}li{{list-style-position:{position}}}'
                           f'li::marker{{content:url("{quadrants}") " "}}', items),
                    (f'ul{{padding-left:60pt}}li{{list-style:{position} url("{quadrants}")}}', items),
                    f'an {position} marker\'s content image draws as its marker image')

    def test_page_margin_boxes_show_content_images(self):
        """CSS Content 3 §1: a page-margin box's single `<image>` replaces the box with it, which
        fills the box the page-margin algorithm sizes from its natural size; images in a
        content list are anonymous inline replaced elements, as in the same content placed as
        a running element (CSS GCPM 3 §1.2)."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-margin-images-') as directory:
            root = Path(directory)
            quadrants = blocks(root / 'quadrants.png', QUADRANTS)
            font = f'font:{FONT_SIZE}pt/{LINE_HEIGHT}pt Probe'
            running = f'#h{{position:running(h);{font}}}'
            for margin, expected, message in [
                    (f'content:"A" url("{quadrants}") "B" counter(page)',
                     f'<div id=h>A<img src="{quadrants}">B1</div>', 'images inline with text'),
                    (f'content:url("{quadrants}");border:2pt solid blue',
                     f'<img id=h src="{quadrants}" style="display:block;width:100%;height:{PAGE_MARGIN - 4}pt">',
                     'a replaced margin box'),
                    (f'content:url("{quadrants}");object-fit:contain;padding:2pt',
                     f'<img id=h src="{quadrants}" style="display:block;width:100%;'
                     f'height:{PAGE_MARGIN - 4}pt;object-fit:contain">',
                     'object-fit fits the replacement into the content box')]:
                border = 'border:2pt solid blue;' if 'border' in margin else ''
                padding = 'padding:2pt;' if 'padding' in margin else ''
                self.assertSameInk(
                    root, (f'@page{{@top-left{{{margin};{font}}}}}', '<p>Body</p>'),
                    (f'@page{{@top-left{{content:element(h);{border}{padding}{font}}}}}{running}',
                     f'{expected}<p>Body</p>'),
                    message)

    def test_counter_style_image_symbols_render_as_inline_images(self):
        """CSS Counter Styles 3 §3.8: image symbols, wherever a symbol stands, are anonymous
        inline replaced boxes in markers and in counter() and counters() content."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-counter-images-') as directory:
            root = Path(directory)
            red, blue, green = (blocks(root / f'{name}.png', [[color]])
                                for name, color in [('red', RED), ('blue', BLUE), ('green', GREEN)])
            rules = (f'@counter-style pic{{system:cyclic;symbols:url("{red}") url("{blue}");suffix:" "}}'
                     f'@counter-style bits{{system:numeric;symbols:url("{green}") url("{blue}");'
                     f'prefix:url("{red}");suffix:" "}}'
                     f'@counter-style mixed{{system:fixed;symbols:"a" url("{blue}");suffix:"."}}')
            items = '<ul><li>One</li><li>Two</li><li>Three</li></ul>'
            for position in ('outside', 'inside'):
                base = f'ul{{padding-left:60pt}}li{{list-style-position:{position}}}'
                for style, markers in [
                        ('pic', [f'url("{red}") " "', f'url("{blue}") " "', f'url("{red}") " "']),
                        ('bits', [f'url("{red}") url("{blue}") " "',
                                  f'url("{red}") url("{blue}") url("{green}") " "',
                                  f'url("{red}") url("{blue}") url("{blue}") " "']),
                        ('mixed', ['"a."', f'url("{blue}") "."', '"3."'])]:
                    expected = ''.join(f'li:nth-child({index + 1})::marker{{content:{marker}}}'
                                       for index, marker in enumerate(markers))
                    self.assertSameInterior(root, (rules + base + f'li{{list-style-type:{style}}}', items),
                                            (base + expected, items),
                                            f'{position} {style} markers hold their image symbols')
            headings = '<h1>A</h1><h1>B</h1>'
            heading = 'h1{counter-increment:c;font-size:inherit;margin:0}'
            self.assertSameInterior(
                root, (rules + heading + 'h1::before{content:counter(c,pic) "|" counters(c,".",bits) "|"}',
                       headings),
                (heading + f'h1:first-child::before{{content:url("{red}") "|" url("{blue}") "|"}}'
                           f'h1:last-child::before{{content:url("{blue}") "|" url("{blue}") url("{green}") "|"}}',
                 headings),
                'counter() and counters() print image symbols, without a marker\'s prefix and suffix')

    def test_border_area_clips_layers_to_the_painted_border(self):
        """CSS Backgrounds 4: `background-clip: border-area` paints only where the border styles
        paint, whatever the border colour's transparency; the colour follows the bottom layer.
        Viewers rasterize a mask's edges unlike a stroke's, so edges are not compared."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-border-area-') as directory:
            root = Path(directory)
            box = '<div class=box></div>'
            # A groove paints its whole width, in two shades.
            for border, painted in [('6pt dashed', '6pt dashed'), ('9pt double', '9pt double'),
                                    ('6pt dotted', '6pt dotted'), ('8pt groove', '8pt solid')]:
                self.assertSameInterior(
                    root, (f'.box{{border:{border} transparent;background:blue;background-clip:border-area}}', box),
                    (f'.box{{border:{painted} blue}}', box), f'the background colour fills a {border} border')
                self.assertSameInterior(
                    root, (f'.box{{border:{border} rgb(0 0 0 / 0);border-radius:12pt;padding:4pt;'
                           'background-image:linear-gradient(red,red),linear-gradient(blue,blue);'
                           'background-clip:content-box,border-area}', box),
                    (f'.box{{border:{painted} blue;border-radius:12pt;padding:4pt;'
                     'background:linear-gradient(red,red) content-box}', box),
                    f'one layer of several fills a rounded {border} border')

    def assertInkSize(self, root, css, body, expected, message):
        """Everything not white on the first page spans `expected` CSS pixels. Resolutions that
        are not exact binary fractions, and the whole pixels per metre PNG records, leave an
        image's edge just past a raster pixel boundary, so it may cover one more pixel."""
        pixels = self.raster(root, self.render(root, css, body))
        ink = [(x, y) for y in range(pixels.height) for x in range(pixels.width)
               if pixels.getpixel((x, y)) != WHITE]
        self.assertTrue(ink, f'{message}: nothing painted')
        actual = tuple(max(point[axis] for point in ink) - min(point[axis] for point in ink) + 1
                       for axis in range(2))
        self.assertTrue(all(0 <= a - e <= 1 for a, e in zip(actual, expected)),
                        f'{message}: ink is {actual} pixels, not {expected}')

    def test_image_resolution_sets_raster_natural_sizes(self):
        """CSS Images 4 §6.1: explicit resolutions, recorded ones with from-image, and the
        resolution image-set() or srcset chooses, which image-resolution leaves alone."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-image-resolution-') as directory:
            root = Path(directory)
            size = (48, 24)
            plain = root / 'plain.png'
            Image.new('RGB', size, RED).save(plain)
            recorded = root / 'recorded.png'
            Image.new('RGB', size, RED).save(recorded, dpi=(288, 288))
            anamorphic = root / 'anamorphic.png'
            Image.new('RGB', size, RED).save(anamorphic, dpi=(192, 96))
            jfif = root / 'jfif.jpg'
            Image.new('RGB', size, RED).save(jfif, quality=100, dpi=(192, 192))
            exif_only = root / 'exif.jpg'
            exif = Image.new('RGB', size, RED).getexif()
            exif[EXIF_X_RESOLUTION] = 288
            exif[EXIF_Y_RESOLUTION] = 288
            exif[EXIF_RESOLUTION_UNIT] = EXIF_INCHES
            Image.new('RGB', size, RED).save(exif_only, quality=100, exif=exif)
            turned_exif = root / 'turned.jpg'
            exif[EXIF_ORIENTATION] = EXIF_ROTATE_90
            exif[EXIF_Y_RESOLUTION] = 96
            Image.new('RGB', size, RED).save(turned_exif, quality=100, exif=exif)
            svg = root / 'vector.svg'
            svg.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="48" height="24">'
                           '<rect width="48" height="24" fill="red"/></svg>')
            for source, value, expected in [
                    (plain, '1dppx', (48, 24)), (plain, '2dppx', (24, 12)), (plain, '192dpi snap', (24, 12)),
                    (plain, 'from-image', (48, 24)), (plain, 'from-image 3dppx', (16, 8)),
                    (recorded, '1dppx', (48, 24)), (recorded, 'from-image', (16, 8)),
                    (recorded, '2dppx from-image', (16, 8)), (recorded, 'snap from-image', (16, 8)),
                    (anamorphic, 'from-image', (24, 24)), (jfif, 'from-image', (24, 12)),
                    (exif_only, 'from-image', (16, 8)), (turned_exif, 'from-image', (24, 16)),
                    (svg, '2dppx', (48, 24))]:
                self.assertInkSize(root, f'img{{display:block;image-resolution:{value}}}',
                                   f'<img src="{source.as_uri()}">', expected,
                                   f'{source.name} with image-resolution: {value}')
            self.assertInkSize(root, 'img{display:block;image-resolution:2dppx}',
                               f'<img srcset="{plain.as_uri()} 3x">', (16, 8),
                               'a srcset density is the image\'s own resolution')
            self.assertInkSize(root, 'div{image-resolution:2dppx}img{display:block}',
                               f'<div><img src="{plain.as_uri()}"></div>', (24, 12), 'the property inherits')
            for image, expected in [(f'url("{plain.as_uri()}")', (24, 12)),
                                    (f'image("{plain.as_uri()}#xywh=0,0,24,24")', (12, 12)),
                                    (f'image-set(url("{plain.as_uri()}") 3x)', (16, 8)),
                                    (f'url("{recorded.as_uri()}")', (24, 12))]:
                self.assertInkSize(root, f'.box{{image-resolution:2dppx;background:{image} no-repeat}}',
                                   '<div class=box></div>', expected, f'background {image} at 2dppx')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--renderer', type=Path, default=RENDERER)
    arguments, remaining = parser.parse_known_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[sys.argv[0], *remaining])
