"""Rendered Filter Effects 1 `filter`: which functions stay vector, where rasters land, and
the colours the §13.1 equivalents produce, read back through Poppler."""
from pathlib import Path
import subprocess
import tempfile
import unittest
from html import escape

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / 'target/debug/ferri-pdf'
RASTER_DPI = 72
PAGE_MARGIN = 20
BOX_WIDTH = 100
BOX_HEIGHT = 60
# Eight-bit rounding in the filter and the viewer's resampling of the embedded raster.
COLOR_TOLERANCE = 3
RED = (255, 0, 0)
WHITE = (255, 255, 255)


def sepia(rgb):
    """Filter Effects 1 §13.1.2 at amount 1, clamped to the channel range."""
    rows = [[0.393, 0.769, 0.189], [0.349, 0.686, 0.168], [0.272, 0.534, 0.131]]
    return tuple(min(255, round(sum(weight * channel for weight, channel in zip(row, rgb))))
                 for row in rows)


def grayscale(rgb):
    """Filter Effects 1 §13.1.1 at amount 1."""
    return (round(0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]),) * 3


class FiltersPdf(unittest.TestCase):
    def render(self, root, css, body, page='300pt 400pt'):
        source = root / 'input.html'
        output = root / 'output.pdf'
        source.write_text(f'''<!doctype html><style>
@page{{size:{page};margin:{PAGE_MARGIN}pt}}
body{{margin:0}}div{{margin:0}}.box{{width:{BOX_WIDTH}pt;height:{BOX_HEIGHT}pt}}{css}</style>{body}''')
        subprocess.run([str(RENDERER), str(source), str(output)], check=True, capture_output=True)
        return output

    def raster(self, root, output, page=1):
        subprocess.run(['pdftoppm', '-r', str(RASTER_DPI), '-f', str(page), '-l', str(page),
                        '-aaVector', 'no', '-singlefile', '-png', str(output), str(root / 'ink')],
                       check=True, capture_output=True)
        with Image.open(root / 'ink.png') as image:
            return image.convert('RGB')

    def test_spotlight_translation_uses_each_filter_region_axis(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-spotlight-translation-') as directory:
            root = Path(directory)
            def patch(dx, dy):
                svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="100pt" height="100pt" viewBox="0 0 100 100">'
                       '<defs><filter id="light"><feDiffuseLighting surfaceScale="3">'
                       f'<feSpotLight x="{30 + dx}" y="{20 + dy}" z="50" pointsAtX="{20 + dx}" pointsAtY="{20 + dy}" '
                       'pointsAtZ="0" specularExponent="2" limitingConeAngle="60"/>'
                       '</feDiffuseLighting></filter></defs>'
                       f'<rect x="{10 + dx}" y="{10 + dy}" width="30" height="30" fill="green" filter="url(#light)"/></svg>')
                source = root / 'light.svg'
                source.write_text(svg)
                output = self.render(root, 'img{display:block}', f'<img src="{source.as_uri()}">')
                with self.raster(root, output) as raster:
                    return raster.crop((PAGE_MARGIN + 5 + dx, PAGE_MARGIN + 5 + dy,
                                        PAGE_MARGIN + 45 + dx, PAGE_MARGIN + 45 + dy))
            with patch(0, 0) as expected, patch(20, 30) as actual:
                self.assertGreater(len(expected.getcolors(expected.width * expected.height)), 2,
                                   'the comparison must contain visible, varying spotlight illumination')
                self.assertEqual(actual.tobytes(), expected.tobytes(),
                                 'moving a shape and its light together must preserve illumination when x and y translations differ')

    def images(self, output):
        listing = subprocess.run(['pdfimages', '-list', str(output)], check=True,
                                 capture_output=True, text=True).stdout.splitlines()[2:]
        return [line.split() for line in listing if line.split()[2] == 'image']

    def text(self, output):
        return subprocess.run(['pdftotext', str(output), '-'], check=True, capture_output=True,
                              text=True).stdout

    def assertNear(self, actual, expected, label, tolerance=COLOR_TOLERANCE):
        self.assertTrue(all(abs(a - e) <= tolerance for a, e in zip(actual, expected)),
                        f'{label}: rendered {actual}, expected {expected} (tolerance {tolerance})')

    def test_blur_rasterizes_in_place_and_spreads_past_the_box(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
            root = Path(directory)
            output = self.render(root, '.box{background:red;filter:blur(4pt)}',
                                 '<div class=box></div>')
            images = self.images(output)
            self.assertEqual(len(images), 1, f'one raster for the filtered group: {images}')
            pixels = self.raster(root, output)
            centre = (PAGE_MARGIN + BOX_WIDTH // 2, PAGE_MARGIN + BOX_HEIGHT // 2)
            self.assertNear(pixels.getpixel(centre), RED, 'blurred box centre')
            edge = pixels.getpixel((PAGE_MARGIN + BOX_WIDTH + 2, centre[1]))
            self.assertTrue(edge != WHITE and edge[0] == 255 and 0 < edge[1] < 255,
                            f'two points past the edge the blur spreads red: {edge}')
            self.assertEqual(pixels.getpixel((PAGE_MARGIN + BOX_WIDTH + 20, centre[1])), WHITE,
                             'five standard deviations away nothing is painted')

    def test_range_preserving_matrices_keep_vector_text_and_recolour_exactly(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
            root = Path(directory)
            output = self.render(
                root, '.box{background:red;color:blue;filter:grayscale(1) invert(0) opacity(1)}',
                '<div class=box>Vector words</div>')
            self.assertEqual(self.images(output), [], 'grayscale() stays vector')
            self.assertIn('Vector words', self.text(output), 'filtered text remains text')
            pixels = self.raster(root, output)
            self.assertNear(pixels.getpixel((PAGE_MARGIN + BOX_WIDTH - 5, PAGE_MARGIN + BOX_HEIGHT - 5)),
                            grayscale(RED), 'grayscale() of the red background')

    def test_clamping_matrices_rasterize_with_the_spec_colours(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
            root = Path(directory)
            output = self.render(root, '.box{background:red;filter:sepia(1)}',
                                 '<div class=box></div>')
            self.assertEqual(len(self.images(output)), 1,
                             'sepia() leaves the channel range, so it filters pixels')
            pixels = self.raster(root, output)
            self.assertNear(pixels.getpixel((PAGE_MARGIN + BOX_WIDTH // 2, PAGE_MARGIN + BOX_HEIGHT // 2)),
                            sepia(RED), 'sepia() of red')

    def test_drop_shadow_paints_the_offset_alpha_in_its_colour_below_the_content(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
            root = Path(directory)
            output = self.render(root, '.box{background:red;filter:drop-shadow(10pt 12pt blue)}',
                                 '<div class=box></div>')
            pixels = self.raster(root, output)
            self.assertNear(pixels.getpixel((PAGE_MARGIN + BOX_WIDTH + 5, PAGE_MARGIN + BOX_HEIGHT + 6)),
                            (0, 0, 255), 'the shadow beyond the box')
            self.assertNear(pixels.getpixel((PAGE_MARGIN + BOX_WIDTH - 5, PAGE_MARGIN + BOX_HEIGHT - 5)),
                            RED, 'the content above its shadow')
            self.assertEqual(pixels.getpixel((PAGE_MARGIN + 5, PAGE_MARGIN + BOX_HEIGHT + 6)), WHITE,
                             'a sharp shadow starts at its offset')

    def test_referenced_svg_filter_resolves_its_region_against_the_border_box(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
            root = Path(directory)
            # The default filter region is the bounding box grown by 10% on every side.
            output = self.render(
                root, '.box{background:red;filter:url(#flood)}',
                '<svg width=0 height=0 style=position:absolute><filter id=flood><feFlood flood-color=lime '
                'flood-opacity=1/></filter></svg><div class=box></div>')
            pixels = self.raster(root, output)
            inside = (PAGE_MARGIN + BOX_WIDTH + BOX_WIDTH // 20, PAGE_MARGIN + BOX_HEIGHT // 2)
            outside = (PAGE_MARGIN + BOX_WIDTH + BOX_WIDTH // 5, PAGE_MARGIN + BOX_HEIGHT // 2)
            self.assertNear(pixels.getpixel(inside), (0, 255, 0), 'flood inside the 120% region')
            self.assertEqual(pixels.getpixel(outside), WHITE, 'nothing past the filter region')

    def test_reference_to_a_missing_filter_ignores_the_chain(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
            root = Path(directory)
            output = self.render(root, '.box{background:red;filter:url(#absent) sepia(1)}',
                                 '<div class=box></div>')
            self.assertEqual(self.images(output), [], 'Filter Effects 1 §5 ignores the whole chain')
            pixels = self.raster(root, output)
            self.assertNear(pixels.getpixel((PAGE_MARGIN + 5, PAGE_MARGIN + 5)), RED,
                            'unfiltered content')

    def test_ancestor_clip_cuts_the_filtered_result(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
            root = Path(directory)
            output = self.render(
                root, '.clip{overflow:hidden;width:60pt;height:40pt}'
                      '.box{background:blue;filter:blur(3pt)}',
                '<div class=clip><div class=box></div></div>')
            pixels = self.raster(root, output)
            self.assertNear(pixels.getpixel((PAGE_MARGIN + 58, PAGE_MARGIN + 20)), (0, 0, 255),
                            'blurred content inside the clip stays solid away from the box edges')
            self.assertEqual(pixels.getpixel((PAGE_MARGIN + 62, PAGE_MARGIN + 20)), WHITE,
                             'the clip applies after the filter, so no blur crosses it')

    def test_blended_descendants_composite_before_the_filter(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
            root = Path(directory)
            output = self.render(
                root, '.box{background:yellow;filter:grayscale(1)}'
                      '.blend{mix-blend-mode:multiply;background:cyan;width:50pt;height:60pt}',
                '<div class=box><div class=blend></div></div>')
            self.assertEqual(len(self.images(output)), 1,
                             'blending does not commute with the matrix, so the group rasterizes')
            pixels = self.raster(root, output)
            middle = PAGE_MARGIN + BOX_HEIGHT // 2
            self.assertNear(pixels.getpixel((PAGE_MARGIN + 25, middle)), grayscale((0, 255, 0)),
                            'grayscale() of cyan multiplied onto yellow')
            self.assertNear(pixels.getpixel((PAGE_MARGIN + 75, middle)), grayscale((255, 255, 0)),
                            'grayscale() of the unblended yellow')

    def test_text_shadows_take_the_filter_in_both_paths(self):
        # The shadow of an 8pt underline is a band 30pt below it.
        css = ('p{margin:0;font-size:20pt;text-decoration:underline red 8pt;'
               'text-decoration-skip-ink:none;text-shadow:0 30pt red;filter:FILTER}')
        for function, colour, rasterized in (('grayscale(1)', grayscale(RED), False),
                                             ('sepia(1)', sepia(RED), True)):
            with self.subTest(filter=function), \
                    tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
                root = Path(directory)
                output = self.render(root, css.replace('FILTER', function), '<p>Shadowed</p>')
                self.assertEqual(bool(self.images(output)), rasterized,
                                 f'{function}: raster only where the matrix leaves the range')
                pixels = self.raster(root, output)
                column = [pixels.getpixel((PAGE_MARGIN + 20, y))
                          for y in range(PAGE_MARGIN, PAGE_MARGIN + 80)]
                self.assertTrue(
                    sum(all(abs(a - e) <= COLOR_TOLERANCE for a, e in zip(pixel, colour))
                        for pixel in column) >= 12,
                    f'{function}: underline and its shadow both take {colour}: {column}')

    def test_iframe_content_filters_inside_and_outside_its_document(self):
        nested = ('<style>body{margin:0}div{width:40pt;height:40pt;background:red}'
                  '.inner{filter:sepia(1)}</style><div class=inner></div><div></div>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
            root = Path(directory)
            output = self.render(
                root, 'iframe{border:0;width:60pt;height:90pt;display:block}'
                      '.outer{filter:invert(1) sepia(1)}',
                f'<iframe srcdoc="{escape(nested)}"></iframe>'
                f'<div class=outer><iframe srcdoc="{escape(nested)}"></iframe></div>')
            pixels = self.raster(root, output)
            self.assertNear(pixels.getpixel((PAGE_MARGIN + 20, PAGE_MARGIN + 20)), sepia(RED),
                            "a filter in the nested document's own display list")
            self.assertNear(pixels.getpixel((PAGE_MARGIN + 20, PAGE_MARGIN + 60)), RED,
                            'unfiltered nested content')
            inverted = (0, 255, 255)
            self.assertNear(pixels.getpixel((PAGE_MARGIN + 20, PAGE_MARGIN + 90 + 60)),
                            sepia(inverted), 'an enclosing filter rasterizes the nested document')

    def test_filtered_box_split_across_pages_filters_each_fragment(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-filters-') as directory:
            root = Path(directory)
            output = self.render(root, '.tall{height:500pt;background:red;filter:sepia(1)}',
                                 '<div class=tall></div>')
            pages = [int(image[0]) for image in self.images(output)]
            self.assertEqual(pages, [1, 2], 'one raster per page fragment')
            for page in (1, 2):
                pixels = self.raster(root, output, page)
                self.assertNear(pixels.getpixel((PAGE_MARGIN + 10, PAGE_MARGIN + 10)), sepia(RED),
                                f'sepia() of the fragment on page {page}')


if __name__ == '__main__':
    unittest.main()
