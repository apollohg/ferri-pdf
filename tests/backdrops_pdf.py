"""Rendered Filter Effects 2 `backdrop-filter` and Compositing 2 `plus-lighter`, read back
through Poppler: where the filtered backdrop lands, which Backdrop Roots bound it, and the
exact colours both effects produce."""
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / 'target/debug/ferri-pdf'
RASTER_DPI = 72
PAGE = 300
# Eight-bit rounding of the read-back raster and the viewer's resampling of it.
COLOR_TOLERANCE = 3
WHITE = (255, 255, 255)
BLACK = (0, 0, 0)
STRIPE = 10
# Ten-point black and white stripes over a white canvas: black where `x // STRIPE` is even.
STRIPES = (f'html{{background:white}}.stripes{{position:absolute;left:0;top:0;display:flex}}'
           f'.stripes>div{{width:{STRIPE}pt;height:{PAGE}pt;background:white}}'
           '.stripes>div:nth-child(odd){background:black}')
STRIPE_ELEMENTS = '<div class=stripes>' + '<div></div>' * (PAGE // STRIPE) + '</div>'
PANEL = (50, 50, 200, 100)
BLUR = 'blur(6pt)'


def stripe(x):
    return BLACK if (x // STRIPE) % 2 == 0 else WHITE


class BackdropsPdf(unittest.TestCase):
    def render(self, root, css, body):
        source = root / 'input.html'
        output = root / 'output.pdf'
        source.write_text(f'''<!doctype html><style>
@page{{size:{PAGE}pt {PAGE}pt;margin:0}}body{{margin:0}}{STRIPES}
.panel{{position:absolute;left:{PANEL[0]}pt;top:{PANEL[1]}pt;width:{PANEL[2]}pt;
height:{PANEL[3]}pt}}{css}</style>{body}''')
        result = subprocess.run([str(RENDERER), str(source), str(output)], capture_output=True,
                                text=True)
        self.assertEqual(result.returncode, 0, f'renderer failed: {result.stderr}')
        return output

    def raster(self, root, output):
        subprocess.run(['pdftoppm', '-r', str(RASTER_DPI), '-aaVector', 'no', '-singlefile',
                        '-png', str(output), str(root / 'ink')], check=True, capture_output=True)
        with Image.open(root / 'ink.png') as image:
            return image.convert('RGB')

    def assertNear(self, actual, expected, label, tolerance=COLOR_TOLERANCE):
        self.assertTrue(all(abs(a - e) <= tolerance for a, e in zip(actual, expected)),
                        f'{label}: rendered {actual}, expected {expected} (tolerance {tolerance})')

    def assertBlurred(self, pixels, label):
        """Across a stripe pair in the panel's middle row, the blur leaves no pure stripe."""
        row = PANEL[1] + PANEL[3] // 2
        values = [pixels.getpixel((x, row))[0] for x in range(140, 160)]
        self.assertTrue(all(40 < value < 215 for value in values),
                        f'{label}: a {BLUR} backdrop mixes the stripes, row {row}: {values}')

    def assertUnblurred(self, pixels, label):
        row = PANEL[1] + PANEL[3] // 2
        for x in (145, 155):
            self.assertEqual(pixels.getpixel((x, row)), stripe(x),
                             f'{label}: the stripe at x={x} shows through unfiltered')

    def test_blurred_backdrop_fills_the_rounded_border_box_of_a_translucent_panel(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-backdrop-') as directory:
            root = Path(directory)
            output = self.render(
                root,
                f'.panel{{backdrop-filter:{BLUR};background:rgba(255,0,0,0.2);'
                'border-radius:20pt}',
                STRIPE_ELEMENTS + '<div class=panel></div>')
            pixels = self.raster(root, output)
            self.assertBlurred(pixels, 'translucent panel')
            centre = pixels.getpixel((150, 100))
            self.assertTrue(centre[0] > centre[1] + 30 and abs(centre[1] - centre[2]) <= 2,
                            f'the 20% red background paints over the blurred grey: {centre}')
            for point in [(48, 100), (252, 100), (150, 48), (150, 152)]:
                self.assertEqual(pixels.getpixel(point), stripe(point[0]),
                                 f'outside the border box at {point} the stripes are untouched')
            # Inside the border box's bounds but outside the 20pt corner radius.
            for point in [(51, 51), (248, 51), (51, 148), (248, 148)]:
                self.assertEqual(pixels.getpixel(point), stripe(point[0]),
                                 f'the rounded corner at {point} clips the backdrop')

    def test_blur_mirrors_the_border_box_instead_of_reading_past_it(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-backdrop-') as directory:
            root = Path(directory)
            # A white area exactly under the panel on a black canvas, blurred then inverted to
            # opaque black: reading past the border box would blur black in, and a transparent
            # margin would leave the edges translucent over the white, both lighter than black.
            output = self.render(
                root,
                'html{background:black}.light{position:absolute;'
                f'left:{PANEL[0]}pt;top:{PANEL[1]}pt;width:{PANEL[2]}pt;height:{PANEL[3]}pt;'
                f'background:white}}.panel{{backdrop-filter:{BLUR} invert(1)}}',
                '<div class=light></div><div class=panel></div>')
            pixels = self.raster(root, output)
            for point in [(50, 100), (52, 100), (150, 50), (150, 149), (249, 100), (249, 149)]:
                self.assertNear(pixels.getpixel(point), BLACK,
                                f'§2.1 mirrors the white border box across its edges at {point}')
            self.assertEqual(pixels.getpixel((48, 100)), BLACK, 'outside the box stays black')

    def test_backdrop_roots_bound_what_the_backdrop_reads(self):
        roots = {
            'will-change: opacity': 'will-change:opacity',
            'opacity below one': 'opacity:0.999',
            'filter': 'filter:grayscale(0)',
            'view-transition-name': 'view-transition-name:shelf',
            'mix-blend-mode': 'mix-blend-mode:multiply',
            'backdrop-filter': 'backdrop-filter:opacity(0)',
        }
        for label, declaration in roots.items():
            with self.subTest(root=label), tempfile.TemporaryDirectory(
                    prefix='ferri-pdf-backdrop-') as directory:
                root = Path(directory)
                output = self.render(
                    root,
                    f'.root{{position:absolute;left:0;top:0;width:{PAGE}pt;height:{PAGE}pt;'
                    f'{declaration}}}.panel{{backdrop-filter:{BLUR}}}',
                    STRIPE_ELEMENTS + '<div class=root><div class=panel></div></div>')
                self.assertUnblurred(self.raster(root, output), label)

    def test_content_inside_the_backdrop_root_before_the_element_is_its_backdrop(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-backdrop-') as directory:
            root = Path(directory)
            output = self.render(
                root,
                f'.root{{position:absolute;left:0;top:0;will-change:opacity}}'
                f'.panel{{backdrop-filter:{BLUR}}}',
                '<div class=root>' + STRIPE_ELEMENTS + '<div class=panel></div></div>')
            self.assertBlurred(self.raster(root, output), 'stripes painted inside the root')

    def test_transforms_and_stacking_without_a_backdrop_root_keep_the_page_backdrop(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-backdrop-') as directory:
            root = Path(directory)
            output = self.render(
                root,
                f'.root{{position:absolute;left:0;top:0;z-index:1;transform:translateX(20pt)}}'
                f'.panel{{backdrop-filter:{BLUR};transform:translateY(100pt)}}',
                STRIPE_ELEMENTS + '<div class=root><div class=panel></div></div>')
            pixels = self.raster(root, output)
            row = PANEL[1] + 100 + PANEL[3] // 2
            values = [pixels.getpixel((x, row))[0] for x in range(160, 180)]
            self.assertTrue(all(40 < value < 215 for value in values),
                            f'the translated panel blurs the page stripes below it: {values}')
            for x in (55, 65):
                self.assertEqual(pixels.getpixel((x, row)), stripe(x),
                                 f'left of the translated border box x={x} is untouched')
            self.assertUnblurred(pixels, 'the untransformed position')

    def test_the_element_opacity_applies_to_its_filtered_backdrop(self):
        cases = {'': BLACK, 'opacity:0.5;': (128, 128, 128)}
        for declaration, expected in cases.items():
            with self.subTest(declaration=declaration), tempfile.TemporaryDirectory(
                    prefix='ferri-pdf-backdrop-') as directory:
                root = Path(directory)
                output = self.render(root, f'.panel{{backdrop-filter:invert(1);{declaration}}}',
                                     '<div class=panel></div>')
                self.assertNear(self.raster(root, output).getpixel((150, 100)), expected,
                                f'white page inverted behind a panel with {declaration!r}')

    def test_plus_lighter_adds_premultiplied_colours(self):
        cases = {
            'sum over an opaque backdrop': ('rgb(100,50,0)', 'rgb(100,100,100)', (200, 150, 100)),
            'sum clamps to one': ('rgb(200,200,50)', 'rgb(100,30,100)', (255, 230, 150)),
        }
        for label, (below, above, expected) in cases.items():
            with self.subTest(label), tempfile.TemporaryDirectory(
                    prefix='ferri-pdf-plus-lighter-') as directory:
                root = Path(directory)
                output = self.render(
                    root,
                    f'.below{{position:absolute;left:0;top:0;width:100pt;height:40pt;'
                    f'background:{below}}}.above{{width:50pt;height:40pt;background:{above};'
                    'mix-blend-mode:plus-lighter}',
                    '<div class=below><div class=above></div></div>')
                pixels = self.raster(root, output)
                self.assertNear(pixels.getpixel((25, 20)), expected, label)
                self.assertEqual(pixels.getpixel((75, 20)),
                                 tuple(int(part) for part in below[4:-1].split(',')),
                                 f'{label}: the backdrop beside the source is unchanged')

    def test_plus_lighter_cross_fade_within_an_isolated_group(self):
        """Compositing 2 §9.1.15's example: two half-opaque layers at half opacity sum to
        `rgb(50% 0% 50% / 50%)`, which over the white page is `rgb(75% 50% 75%)`; source-over
        would give `rgb(75% 56% 81%)`."""
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-plus-lighter-') as directory:
            root = Path(directory)
            output = self.render(
                root,
                '.container{display:grid;isolation:isolate}'
                '.container>div{width:100pt;height:100pt;grid-area:1/1;opacity:0.5}'
                '.from{background:rgb(100% 0% 0% / 50%)}'
                '.to{background:rgb(0% 0% 100% / 50%);mix-blend-mode:plus-lighter}',
                '<div class=container><div class=from></div><div class=to></div></div>')
            self.assertNear(self.raster(root, output).getpixel((50, 50)), (191, 128, 191),
                            'plus-lighter cross-fade over white')

    def test_plus_darker_subtracts_the_complements_of_both_layers(self):
        """Compositing 2 §9.1.14 read as Core Graphics' `kCGBlendModePlusDarker`: over an
        opaque backdrop each channel is `max(0, 1 − ((1 − cs) + (1 − cb)))`."""
        cases = {
            'darker sum': ('rgb(200,150,255)', 'rgb(150,250,100)', (95, 145, 100)),
            'sum clamps to zero': ('rgb(100,0,255)', 'rgb(100,255,0)', (0, 0, 0)),
        }
        for label, (below, above, expected) in cases.items():
            with self.subTest(label), tempfile.TemporaryDirectory(
                    prefix='ferri-pdf-plus-darker-') as directory:
                root = Path(directory)
                output = self.render(
                    root,
                    f'.below{{position:absolute;left:0;top:0;width:100pt;height:40pt;'
                    f'background:{below}}}.above{{width:50pt;height:40pt;background:{above};'
                    'mix-blend-mode:plus-darker}',
                    '<div class=below><div class=above></div></div>')
                pixels = self.raster(root, output)
                self.assertNear(pixels.getpixel((25, 20)), expected, label)
                self.assertEqual(pixels.getpixel((75, 20)),
                                 tuple(int(part) for part in below[4:-1].split(',')),
                                 f'{label}: the backdrop beside the source is unchanged')
        with self.subTest('over the white page'), tempfile.TemporaryDirectory(
                prefix='ferri-pdf-plus-darker-') as directory:
            root = Path(directory)
            output = self.render(root, '.above{width:50pt;height:40pt;'
                                 'background:rgb(10,120,250);mix-blend-mode:plus-darker}',
                                 '<div class=above></div>')
            self.assertNear(self.raster(root, output).getpixel((25, 20)), (10, 120, 250),
                            'white is the identity of plus-darker')

if __name__ == '__main__':
    unittest.main()
