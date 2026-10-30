"""Rendered inline SVG styled by the document's cascade (SVG 2 §6.4): class rules, presentation
attributes, inheritance from HTML, `use` instances of a sprite, CSS transforms, clipping, masks,
overflow and direction on SVG content, and the isolation of SVG images from the document's style
sheets, read back through Poppler."""
import base64
import math
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from urllib.parse import quote

from PIL import Image

from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / 'target/debug/ferri-pdf'
RASTER_DPI = 72
PAGE_MARGIN = 20
PAGE = 300
ICON = 40
SQUARE = 20
COLOR_TOLERANCE = 3
RED = (255, 0, 0)
GREEN = (0, 160, 0)
BLUE = (0, 0, 255)
YELLOW = (255, 255, 0)
MAGENTA = (200, 0, 200)
WHITE = (255, 255, 255)
PAGE_RED = (200, 0, 0)
# Half-covering white over black: linear light 0.5 encodes to sRGB 0.735 (188); sRGB gives 128.
LINEAR_HALF = (188, 188, 188)
SRGB_HALF = (128, 128, 128)
# An identity filter, which hands its group to the raster filter renderer.
IDENTITY_FILTER = '<filter id="same"><feOffset dx="0" dy="0"/></filter>'
# Poppler's Splash rasterizer misplaces soft masks set under a transform; Cairo does not.
SPLASH = ('pdftoppm', '-aaVector', 'no')
CAIRO = ('pdftocairo', '-antialias', 'none')
SPRITE = (f'<svg style="display:none"><symbol id="dot" viewBox="0 0 {ICON} {ICON}">'
          f'<rect width="{ICON}" height="{ICON}" fill="currentColor"/></symbol></svg>')


class SvgCssPdf(unittest.TestCase):
    def test_quoted_filter_references_preserve_fragment_ids(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-filter-urls-') as directory:
            root = Path(directory)
            for identifier in ('f', 'f space', 'f)', 'fé'):
                for external in (False, True):
                    with self.subTest(identifier=identifier, external=external):
                        reference = root / 'reference.svg'
                        reference.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="80">'
                            '<defs><filter id="f" x="-100%" y="-100%" width="300%" height="300%">'
                            '<feOffset dx="10" dy="4"/></filter></defs>'
                            '<rect x="20" y="20" width="20" height="10" fill="green" filter="url(#f)"/></svg>')
                        with self.render(root, '', f'<img src="{reference.as_uri()}">') as expected:
                            expected_ink = expected.tobytes()
                            self.assertIn((0, 128, 0),
                                          [color for _, color in expected.getcolors(PAGE * PAGE)],
                                          'reference filter must paint')
                        body = ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="80">'
                                f'<defs><filter id="{identifier}" x="-100%" y="-100%" width="300%" height="300%">'
                                '<feOffset dx="10" dy="4"/></filter></defs>'
                                f"""<rect x="20" y="20" width="20" height="10" fill="green" filter="url('#{identifier}')"/></svg>""")
                        if external:
                            source = root / 'filter.svg'
                            source.write_text(body)
                            body = f'<img src="{source.as_uri()}">'
                        with self.render(root, '', body) as actual:
                            self.assertEqual(actual.tobytes(), expected_ink,
                                             'quoted fragment IDs must survive SVG parsing')

    def convert(self, root, css, body):
        source = root / 'input.html'
        output = root / 'output.pdf'
        source.write_text(f'''<!doctype html><style>
@page{{size:{PAGE}pt {PAGE}pt;margin:{PAGE_MARGIN}pt}}
body{{margin:0;line-height:0}}svg,img{{display:block}}{css}</style>{body}''')
        completed = subprocess.run([str(RENDERER), str(source), str(output)], capture_output=True,
                                   text=True)
        return completed, output

    def render(self, root, css, body, rasterizer=SPLASH):
        completed, output = self.convert(root, css, body)
        self.assertEqual(completed.returncode, 0, f'render failed: {completed.stderr}')
        subprocess.run([*rasterizer, '-r', str(RASTER_DPI), '-singlefile', '-png', str(output),
                        str(root / 'ink')], check=True, capture_output=True)
        with Image.open(root / 'ink.png') as image:
            return image.convert('RGB')

    def ink_columns(self, pixels, left, top, width, height, color):
        """The page columns, in points from the content edge, holding `color` in the region."""
        return [x for x in range(left, left + width)
                if any(all(abs(a - e) <= COLOR_TOLERANCE
                           for a, e in zip(pixels.getpixel((PAGE_MARGIN + x, PAGE_MARGIN + y)),
                                           color))
                       for y in range(top, top + height))]

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

    def test_overflow_shows_pattern_tiles_and_sliced_images_beyond_their_bounds(self):
        """SVG 2 §3.11, §12.3 and §14.3.2: `overflow: visible` on a pattern renders each tile's
        overflowing content over the following tiles, and on an image the sliced parts beyond
        its positioning rectangle; the user agent's `hidden` clips both."""
        stripe = Image.new('RGB', (2, 1), RED)
        encoded = io.BytesIO()
        stripe.save(encoded, format='PNG')
        source = 'data:image/png;base64,' + base64.b64encode(encoded.getvalue()).decode()
        tile = (f'width="{SQUARE}" height="{SQUARE}" patternUnits="userSpaceOnUse"><rect '
                f'x="15" width="10" height="{SQUARE}" fill="rgb(0,0,255)"/></pattern>')
        image = (f'width="{SQUARE}" height="{SQUARE}" preserveAspectRatio="xMidYMid slice" '
                 f'href="{source}"/>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-overflow-') as directory:
            pixels = self.render(
                Path(directory), '.shown{overflow:visible}',
                f'<svg width="{PAGE - 2 * PAGE_MARGIN}pt" height="100pt" '
                f'viewBox="0 0 {PAGE - 2 * PAGE_MARGIN} 100">'
                f'<pattern id="shown" class="shown" {tile}<pattern id="clipped" {tile}'
                f'<rect width="{4 * SQUARE}" height="{SQUARE}" fill="url(#shown)"/>'
                f'<rect y="{2 * SQUARE}" width="{4 * SQUARE}" height="{SQUARE}" '
                f'fill="url(#clipped)"/>'
                f'<image class="shown" x="{6 * SQUARE}" {image}'
                f'<image x="{6 * SQUARE}" y="{2 * SQUARE}" {image}</svg>')
            middle = SQUARE // 2
            self.assertPixel(pixels, SQUARE + 2, middle, BLUE,
                             'the previous tile\'s stripe overflows into the next tile')
            self.assertPixel(pixels, SQUARE + 2, 2 * SQUARE + middle, WHITE,
                             'a hidden pattern clips each stripe to its own tile')
            for row, label in ((0, 'visible'), (2 * SQUARE, 'hidden')):
                self.assertPixel(pixels, 6 * SQUARE + middle, row + middle, RED,
                                 f'the {label} image fills its rectangle')
            self.assertPixel(pixels, 6 * SQUARE - 5, middle, RED,
                             'the sliced image shows beyond its rectangle')
            self.assertPixel(pixels, 6 * SQUARE - 5, 2 * SQUARE + middle, WHITE,
                             'a hidden image is clipped to its rectangle')

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

    def test_css_transforms_clips_masks_and_normalize_overflow_reach_svg_content(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            offset = (ICON - SQUARE) // 2
            pixels = self.render(
                Path(directory),
                # normalize.css: nested and inline svg elements clip their overflow.
                'svg:not(:root){overflow:hidden}'
                '.spin{transform:rotate(45deg);transform-origin:center;transform-box:fill-box}'
                '.still{transform:none} .round{clip-path:circle(50%)} .half{mask:url(#left)}'
                f'.framed{{clip-path:xywh(10px 10px {SQUARE}px {SQUARE}px round 5px)}}',
                f'<svg width="{4 * ICON}pt" height="{2 * ICON}pt" '
                f'viewBox="0 0 {4 * ICON} {2 * ICON}">'
                f'<mask id="left"><rect x="{3 * ICON}" width="{SQUARE}" height="{ICON}" '
                'fill="white"/></mask>'
                f'<rect class="spin" x="{offset}" y="{offset}" width="{SQUARE}" height="{SQUARE}" '
                'fill="rgb(0,160,0)"/>'
                f'<svg x="{ICON}" width="{SQUARE}" height="{SQUARE}">'
                f'<rect width="{ICON}" height="{ICON}" fill="blue"/></svg>'
                f'<rect class="round" x="{2 * ICON}" width="{ICON}" height="{ICON}" '
                'fill="rgb(0,160,0)"/>'
                f'<rect class="half" x="{3 * ICON}" width="{ICON}" height="{ICON}" fill="blue"/>'
                f'<rect class="still" y="{ICON}" width="{ICON}" height="{ICON}" fill="blue" '
                f'transform="translate({2 * ICON} 0)"/>'
                f'<rect class="framed" x="{ICON}" y="{ICON}" width="{ICON}" height="{ICON}" '
                'fill="rgb(0,160,0)"/>'
                '</svg>')
            half = ICON // 2
            self.assertPixel(pixels, half, offset - 3, GREEN,
                             'CSS Transforms 1 §6: the square turns about its fill-box centre, '
                             'its corner rising above its untransformed top edge')
            self.assertPixel(pixels, offset + 1, offset + 1, WHITE,
                             'the turned square leaves its original corner')
            self.assertPixel(pixels, ICON + SQUARE // 2, SQUARE // 2, BLUE,
                             'the nested svg draws its content')
            self.assertPixel(pixels, ICON + SQUARE + 5, SQUARE + 5, WHITE,
                             'normalize.css: the nested svg clips content past its viewport')
            self.assertPixel(pixels, 2 * ICON + half, half, GREEN,
                             'the circle clip keeps the centre')
            self.assertPixel(pixels, 2 * ICON + 2, 2, WHITE,
                             'CSS Masking 1 §6.1: circle(50%) clips the corners')
            self.assertPixel(pixels, 3 * ICON + SQUARE // 2, half, BLUE, 'the mask shows its half')
            self.assertPixel(pixels, 3 * ICON + SQUARE + SQUARE // 2, half, WHITE,
                             'the mask hides the other half')
            self.assertPixel(pixels, half, ICON + half, BLUE,
                             'SVG 2 §6.6: transform:none beats the transform attribute')
            self.assertPixel(pixels, 2 * ICON + half, ICON + half, WHITE,
                             'the transform attribute no longer moves the rectangle')
            self.assertPixel(pixels, ICON + half, ICON + half, GREEN, 'xywh() keeps its square')
            self.assertPixel(pixels, ICON + 10, ICON + half, GREEN,
                             'the square starts 10px into the rectangle')
            self.assertPixel(pixels, ICON + 9, ICON + half, WHITE,
                             'xywh() clips the rectangle outside its square')
            self.assertPixel(pixels, ICON + 10, ICON + 10, WHITE,
                             'CSS Shapes 1 §3.1: round 5px clips the square\'s corner')

    def test_reference_boxes_place_use_transforms_percentages_and_one_axis_overflow(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            pixels = self.render(
                Path(directory),
                '.turn{transform:rotate(90deg);transform-origin:center;transform-box:fill-box}'
                '.shift{transform:translate(50%)} .wide{overflow-x:hidden;overflow-y:visible}',
                f'<svg width="{4 * ICON}pt" height="{2 * ICON}pt" '
                f'viewBox="0 0 {4 * ICON} {2 * ICON}">'
                '<defs><rect id="bar" y="15" width="40" height="10" fill="blue"/></defs>'
                '<use class="turn" href="#bar"/>'
                '<rect class="shift" y="50" width="20" height="20" fill="rgb(0,160,0)"/>'
                '<svg class="wide" x="40" y="40" width="20" height="10">'
                '<rect x="-10" y="-10" width="40" height="40" fill="red"/></svg>'
                '</svg>')
            self.assertPixel(pixels, 20, 3, BLUE,
                             'CSS Transforms 1 §6: the use element turns about the centre of its '
                             'instance, whose fill box it measures')
            self.assertPixel(pixels, 3, 20, WHITE, 'the turned bar leaves its left end')
            self.assertPixel(pixels, 90, 60, GREEN,
                             'translate(50%) moves by half the 160-unit view box')
            self.assertPixel(pixels, 10, 60, WHITE, 'the translated square leaves its place')
            self.assertPixel(pixels, 50, 65, RED,
                             'SVG 2 §3.11: overflow-y: visible shows content below the viewport')
            self.assertPixel(pixels, 35, 45, WHITE, 'overflow-x: hidden clips the left')
            self.assertPixel(pixels, 65, 45, WHITE, 'overflow-x: hidden clips the right')

    def test_linear_rgb_gradients_interpolate_in_linear_light(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            pixels = self.render(
                Path(directory), '#light{color-interpolation:linearRGB}',
                f'<svg width="{4 * ICON}pt" height="{ICON}pt" viewBox="0 0 {4 * ICON} {ICON}">'
                '<linearGradient id="light"><stop offset="0" stop-color="black"/>'
                '<stop offset="1" stop-color="white"/></linearGradient>'
                '<linearGradient id="plain"><stop offset="0" stop-color="black"/>'
                '<stop offset="1" stop-color="white"/></linearGradient>'
                f'<rect width="{2 * ICON}" height="{SQUARE}" fill="url(#light)"/>'
                f'<rect y="{SQUARE}" width="{2 * ICON}" height="{SQUARE}" fill="url(#plain)"/>'
                '</svg>')
            # Half way, linear light 0.5 encodes to sRGB 0.735 (188); sRGB gives 0.5 (128).
            for y, expected, space in ((SQUARE // 2, 188, 'linear light'),
                                       (SQUARE + SQUARE // 2, 128, 'sRGB')):
                grey = pixels.getpixel((PAGE_MARGIN + ICON, PAGE_MARGIN + y))
                self.assertTrue(all(abs(channel - expected) <= 4 for channel in grey),
                                f'SVG 2 §13.9: the gradient interpolates in {space}: {grey} '
                                f'half way, expected {expected}')

    def test_additive_blending_reads_back_the_svg_stacking_context(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            pixels = self.render(
                Path(directory), f'body{{background:rgb{PAGE_RED}}}',
                f'<svg width="{4 * ICON}pt" height="{2 * ICON}pt" '
                f'viewBox="0 0 {4 * ICON} {2 * ICON}"><defs>{IDENTITY_FILTER}'
                f'<rect id="add" width="{SQUARE}" height="{ICON}" fill="rgb(0,100,0)" '
                'style="mix-blend-mode:plus-lighter"/></defs>'
                f'<rect width="{ICON}" height="{ICON}" fill="rgb(100,50,0)"/>'
                f'<rect x="{SQUARE}" width="{ICON}" height="{ICON}" fill="rgb(100,100,100)" '
                'style="mix-blend-mode:plus-lighter"/>'
                f'<rect x="{2 * ICON}" width="{ICON}" height="{ICON}" fill="rgb(200,150,100)"/>'
                f'<rect x="{2 * ICON + SQUARE}" width="{ICON}" height="{ICON}" '
                'fill="rgb(100,100,100)" style="mix-blend-mode:plus-darker"/>'
                f'<rect y="{ICON}" width="{4 * ICON}" height="{ICON}" fill="rgb(0,0,100)"/>'
                f'<g transform="translate(5 {ICON + 5}) scale(2)"><rect width="5" height="5" '
                'fill="rgb(0,100,0)" style="mix-blend-mode:plus-lighter"/></g>'
                f'<use href="#add" x="{ICON}" y="{ICON}"/>'
                f'<g filter="url(#same)"><rect x="{2 * ICON}" y="{ICON}" width="{SQUARE}" '
                f'height="{ICON}" fill="rgb(100,0,0)"/><rect x="{2 * ICON}" y="{ICON}" '
                f'width="{SQUARE}" height="{ICON}" fill="rgb(0,100,0)" '
                'style="mix-blend-mode:plus-lighter"/></g></svg>')
            self.assertPixel(pixels, 30, 20, (200, 150, 100),
                             'Compositing 2 §9.1.15: plus-lighter adds the colours it covers')
            self.assertPixel(pixels, 50, 20, (100, 100, 100),
                             'SVG 2 §3.4.1: the outermost svg is an isolated stacking context, so '
                             'plus-lighter adds nothing of the page behind it')
            self.assertPixel(pixels, 10, 20, (100, 50, 0),
                             'the vector backdrop beside the additive rectangle is untouched')
            self.assertPixel(pixels, 110, 20, (45, 0, 0),
                             'Compositing 2 §9.1.14: plus-darker subtracts both complements')
            self.assertPixel(pixels, 10, ICON + 10, (0, 100, 100),
                             'the read-back follows the group transform onto its backdrop')
            self.assertPixel(pixels, 20, ICON + 10, (0, 0, 100),
                             'the scaled square ends at 15pt')
            self.assertPixel(pixels, ICON + 10, ICON + 20, (0, 100, 0),
                             'SVG 2 §3.4.1: a use element is an isolated stacking context')
            self.assertPixel(pixels, 2 * ICON + 10, ICON + 20, (100, 100, 0),
                             'the raster filter renderer adds the colours inside the group')

    def test_linear_rgb_composites_translucent_paint_in_linear_light(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            translucent = io.BytesIO()
            Image.new('RGBA', (1, 1), (255, 255, 255, 128)).save(translucent, 'PNG')
            png = base64.b64encode(translucent.getvalue()).decode()
            image = quote(f'<svg xmlns="http://www.w3.org/2000/svg" width="{ICON}" '
                          f'height="{ICON}"><rect width="{ICON}" height="{ICON}"/>'
                          f'<rect width="{ICON}" height="{ICON}" fill="white" fill-opacity="0.5" '
                          'color-interpolation="linearRGB"/></svg>')
            pixels = self.render(
                Path(directory), '.light{color-interpolation:linearRGB} img{width:40pt}',
                f'<svg width="{4 * ICON}pt" height="{2 * ICON}pt" '
                f'viewBox="0 0 {4 * ICON} {2 * ICON}"><defs>{IDENTITY_FILTER}</defs>'
                f'<rect width="{4 * ICON}" height="{2 * ICON}"/>'
                f'<rect class="light" width="{ICON}" height="{ICON}" fill="white" '
                'fill-opacity="0.5"/>'
                f'<rect x="{ICON}" width="{ICON}" height="{ICON}" fill="white" '
                'fill-opacity="0.5"/>'
                f'<g class="light" opacity="0.5"><rect x="{2 * ICON}" width="{ICON}" '
                f'height="{ICON}" fill="white"/></g>'
                f'<image class="light" x="{3 * ICON}" width="{ICON}" height="{ICON}" '
                f'preserveAspectRatio="none" href="data:image/png;base64,{png}"/>'
                f'<g filter="url(#same)"><rect y="{ICON}" width="{ICON}" height="{ICON}"/>'
                f'<rect class="light" y="{ICON}" width="{ICON}" height="{ICON}" fill="white" '
                'fill-opacity="0.5"/></g>'
                f'<rect class="light" x="{ICON}" y="{ICON}" width="{ICON}" height="{ICON}" '
                'fill="white" fill-opacity="0.5" stroke="white" stroke-opacity="0.5" '
                'stroke-width="10"/></svg>'
                f'<img src="data:image/svg+xml,{image}">')
            self.assertPixel(pixels, 20, 20, LINEAR_HALF,
                             'SVG 2 §13.9: translucent fill composites in linear light')
            self.assertPixel(pixels, ICON + 20, 20, SRGB_HALF,
                             'the initial sRGB composites as PDF transparency does')
            self.assertPixel(pixels, 2 * ICON + 20, 20, LINEAR_HALF,
                             'a translucent group composites in linear light')
            self.assertPixel(pixels, 3 * ICON + 20, 20, LINEAR_HALF,
                             'a translucent image composites in linear light')
            self.assertPixel(pixels, 20, ICON + 20, LINEAR_HALF,
                             'the raster filter renderer composites in linear light')
            # The fill, then the stroke over it, each half white in linear light: 0.75.
            self.assertPixel(pixels, ICON + 2, ICON + 20, (225, 225, 225),
                             'fill and stroke composite one after the other')
            self.assertPixel(pixels, 20, 2 * ICON + 20, LINEAR_HALF,
                             'an SVG image composites its linearRGB paint in linear light')

    def test_stroke_color_paints_where_the_stroke_paint_paints_nothing(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            pixels = self.render(
                Path(directory), 'rect{stroke-color:blue;stroke-width:10}',
                f'<svg width="{4 * ICON}pt" height="{ICON}pt" viewBox="0 0 {4 * ICON} {ICON}">'
                f'<rect x="5" y="5" width="{SQUARE}" height="{SQUARE}" fill="none"/>'
                f'<rect x="{ICON + 5}" y="5" width="{SQUARE}" height="{SQUARE}" fill="none" '
                'stroke="yellow"/>'
                f'<rect x="{2 * ICON + 5}" y="5" width="{SQUARE}" height="{SQUARE}" fill="none" '
                'stroke="url(#missing)"/></svg>')
            self.assertPixel(pixels, 5, 15, BLUE,
                             'Fill and Stroke 3 §4.4.1: stroke-color strokes a shape whose stroke '
                             'is none')
            self.assertPixel(pixels, ICON + 5, 15, YELLOW,
                             'the stroke paint wins over the stroke colour')
            self.assertPixel(pixels, 2 * ICON + 5, 15, BLUE,
                             'a paint server reference naming nothing falls back to the colour')

    def test_vector_effects_constrain_strokes_and_user_spaces_in_the_host_space(self):
        # Without a viewBox, user units are the host space's CSS pixels, three quarters of a point.
        def pt(units):
            return round(units * 0.75)
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            pixels = self.render(
                Path(directory), '.nss{vector-effect:non-scaling-stroke}',
                '<svg width="200pt" height="200pt">'
                '<g transform="scale(4 1)"><rect class="nss" x="5" y="10" width="20" height="40" '
                'fill="none" stroke="blue" stroke-width="4"/></g>'
                '<g transform="translate(100 0) scale(3)"><rect x="0" y="0" width="20" '
                'height="20" fill="rgb(0,160,0)" style="vector-effect:non-scaling-size"/></g>'
                '<g transform="translate(150 100) rotate(45)"><rect width="30" height="30" '
                'fill="yellow" vector-effect="non-rotation"/></g>'
                '<g transform="translate(100 150) scale(2)"><rect transform="translate(10 120)" '
                'width="20" height="20" fill="rgb(200,0,200)" vector-effect="fixed-position"/></g>'
                '</svg>')
            self.assertPixel(pixels, pt(20), pt(30), BLUE,
                             'SVG 2 §13.6: the stroke stays centred on the scaled outline')
            self.assertPixel(pixels, pt(14), pt(30), WHITE,
                             'non-scaling-stroke: the horizontally scaled side keeps a 4-unit width')
            self.assertPixel(pixels, pt(104), pt(4), GREEN,
                             'SVG 2 §8.13.1: non-scaling-size keeps the origin the CTM gives')
            self.assertPixel(pixels, pt(126), pt(26), WHITE,
                             'non-scaling-size: the square keeps its 20-unit size, unscaled')
            self.assertPixel(pixels, pt(178), pt(102), YELLOW,
                             'non-rotation: the square stays axis-aligned at its origin')
            self.assertPixel(pixels, pt(30), pt(140), MAGENTA,
                             'fixed-position: the element\'s translation places it in the host '
                             'space, at the size the CTM gives')

    def test_svg_text_matches_fractional_weights_widths_and_oblique_angles(self):
        def ink_width(style):
            with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
                pixels = self.render(
                    Path(directory), f'text{{{style}}}',
                    f'<svg width="{4 * ICON}pt" height="{2 * ICON}pt" '
                    f'viewBox="0 0 {4 * ICON} {2 * ICON}">'
                    '<text x="10" y="60" font-family="DejaVu Sans" font-size="60" '
                    'fill="blue">I</text></svg>')
                columns = self.ink_columns(pixels, 0, 0, 4 * ICON, 2 * ICON, BLUE)
                self.assertTrue(columns, f'{style}: the I renders')
                return max(columns) - min(columns) + 1
        upright = ink_width('font-style:oblique 0deg')
        self.assertGreater(ink_width('font-style:oblique 20deg'), upright + 3,
                           'CSS Fonts 4 §5.2: a 20deg oblique matches the sloped face, which '
                           '0deg does not')
        self.assertGreater(ink_width('font-weight:650.5'), upright + 3,
                           'a fractional weight above 600 matches the bold face')
        self.assertEqual(ink_width('font-weight:450.5;font-stretch:87.5%'), upright,
                         'a weight below 500 and a narrower width keep the regular face, the '
                         'only width the family has')

    def test_right_to_left_pages_anchor_svg_icon_text_at_its_start_on_the_right(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            anchor = ICON - 2
            pixels = self.render(
                Path(directory), '',
                '<html dir="rtl" lang="he">'
                f'<svg width="{ICON}pt" height="{ICON}pt" viewBox="0 0 {ICON} {ICON}">'
                f'<text x="{anchor}" y="{ICON - 10}" font-size="16" fill="blue">שלום</text>'
                '</svg>')
            left = PAGE - 2 * PAGE_MARGIN - ICON
            columns = self.ink_columns(pixels, left, 0, ICON, ICON, BLUE)
            self.assertTrue(columns, 'the icon text renders inside the right-aligned svg')
            self.assertLessEqual(max(columns), left + anchor + 1,
                                 'SVG 2 §11.10.1.1: in right-to-left text, start anchors the '
                                 f'right edge at x; ink reaches column {max(columns)}')
            self.assertLess(min(columns), left + anchor - SQUARE,
                            'the word extends leftwards from its anchor')

    def test_clip_path_element_references_clip_boxes(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            pixels = self.render(
                Path(directory), 'div{height:%dpt;width:%dpt}' % (ICON, 2 * ICON),
                '<svg width="0" height="0" style="position:absolute">'
                f'<clipPath id="disc"><circle cx="{ICON // 2}" cy="{ICON // 2}" r="{SQUARE // 2}"/>'
                '</clipPath><clipPath id="right" clipPathUnits="objectBoundingBox">'
                '<rect x="0.5" width="0.5" height="1"/></clipPath></svg>'
                '<div style="background:blue;clip-path:url(#disc)"></div>'
                '<div style="background:rgb(0,160,0);clip-path:url(#right)"></div>'
                '<div style="background:red;clip-path:url(#missing)"></div>')
            half = ICON // 2
            self.assertPixel(pixels, half, half, BLUE,
                             'CSS Masking 1 §6.1: the clipPath circle keeps its centre')
            self.assertPixel(pixels, half, 2, WHITE,
                             'user space starts at the border box, so the circle clips the top')
            self.assertPixel(pixels, ICON + half, half, WHITE,
                             'the circle clips the rest of the box')
            self.assertPixel(pixels, half, ICON + half, WHITE,
                             'objectBoundingBox units: the left half of the box is clipped')
            self.assertPixel(pixels, ICON + half, ICON + half, GREEN,
                             'objectBoundingBox units: the right half of the box remains')
            self.assertPixel(pixels, half, 2 * ICON + half, RED,
                             'a reference naming no clipPath element clips nothing')

    def test_references_into_other_documents_load_them_with_their_own_styles(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            root = Path(directory)
            (root / 'defs.svg').write_text(
                '<svg xmlns="http://www.w3.org/2000/svg">'
                '<style>.green stop{stop-color:rgb(0,160,0)}</style>'
                '<linearGradient id="paint" class="green"><stop offset="0"/><stop offset="1"/>'
                '</linearGradient>'
                '<linearGradient id="nested" href="more.svg#blue"/>'
                f'<marker id="dot" markerWidth="{SQUARE}" markerHeight="{SQUARE}" refX="0" refY="0" '
                f'markerUnits="userSpaceOnUse"><rect width="{SQUARE}" height="{SQUARE}" fill="blue"/>'
                '</marker>'
                '<clipPath id="left" clipPathUnits="objectBoundingBox">'
                '<rect width="0.5" height="1"/></clipPath>'
                '<mask id="top" maskContentUnits="objectBoundingBox">'
                '<rect width="1" height="0.5" fill="white"/></mask>'
                '<filter id="flood" x="0" y="0" width="1" height="1">'
                '<feFlood flood-color="rgb(0,160,0)"/></filter>'
                '<rect width="1000" height="1000" fill="rgb(200,0,200)"/></svg>')
            (root / 'more.svg').write_text(
                '<svg xmlns="http://www.w3.org/2000/svg"><linearGradient id="blue">'
                '<stop offset="0" stop-color="blue"/><stop offset="1" stop-color="blue"/>'
                '</linearGradient></svg>')
            pixels = self.render(
                root, 'stop{stop-color:red} .box{width:%dpt;height:%dpt;background:red}'
                % (ICON, ICON),
                f'<svg width="{4 * ICON}pt" height="{ICON}pt" viewBox="0 0 {4 * ICON} {ICON}">'
                f'<rect width="{ICON}" height="{ICON}" fill="url(defs.svg#paint)"/>'
                f'<rect x="{ICON}" width="{ICON}" height="{ICON}" fill="url(defs.svg#nested)"/>'
                f'<rect x="{2 * ICON}" width="{ICON}" height="{SQUARE}" '
                'style="fill:url(missing.svg#paint) blue"/>'
                f'<rect x="{2 * ICON}" y="{SQUARE}" width="{ICON}" height="{SQUARE}" '
                'fill="url(defs.svg#absent)"/>'
                f'<path d="M{3 * ICON} 0 h{SQUARE}" stroke="none" '
                'style="marker-start:url(defs.svg#dot)"/></svg>'
                '<div class="box" style="clip-path:url(defs.svg#left)"></div>'
                '<div class="box" style="mask:url(defs.svg#top)"></div>'
                '<div class="box" style="filter:url(defs.svg#flood)"></div>')
            half = ICON // 2
            quarter = ICON // 4
            self.assertPixel(pixels, half, half, GREEN,
                             'SVG 2 §5.3.2: the referenced document\'s own style sheet styles its '
                             'gradient, and the referencing document\'s does not')
            self.assertPixel(pixels, ICON + half, half, BLUE,
                             'a gradient template in a third document resolves against the '
                             'referenced document')
            self.assertPixel(pixels, 2 * ICON + half, quarter, BLUE,
                             'SVG 2 §13.2: a document that does not load gives the fallback colour')
            self.assertPixel(pixels, 2 * ICON + half, SQUARE + quarter, WHITE,
                             'a reference naming no element without a fallback paints none, and '
                             'the content of the referenced document is not rendered')
            self.assertPixel(pixels, 3 * ICON + quarter, quarter, BLUE,
                             'the marker comes from the other document')
            self.assertPixel(pixels, quarter, ICON + half, RED,
                             'CSS Masking 1 §6.1: the box keeps the left half of the clipPath')
            self.assertPixel(pixels, ICON - quarter, ICON + half, WHITE,
                             'the clipPath removes the right half of the box')
            self.assertPixel(pixels, half, 2 * ICON + quarter, RED,
                             'CSS Masking 1 §7.1: the mask shows the top half of the box')
            self.assertPixel(pixels, half, 3 * ICON - quarter, WHITE,
                             'the mask hides the bottom half of the box')
            self.assertPixel(pixels, half, 3 * ICON + half, GREEN,
                             'Filter Effects 1 §5: the referenced filter floods the box')

    def test_calc_size_calculates_svg_geometry_from_its_basis(self):
        """CSS Values 5 §10.2: a calc-size() behaves as its basis, whose size the calculation
        then adjusts: auto is no width on a rect and the natural size on an image (SVG 2 §7.8),
        and a percentage basis is the viewport's."""
        band = 20
        square = quote(f'<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">'
                       f'<rect width="10" height="10" fill="rgb(0,160,0)"/></svg>')
        standalone = quote('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 20">'
                           '<rect style="width:calc-size(auto, size + 30px);height:20px" '
                           'fill="blue"/></svg>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            pixels = self.render(
                Path(directory),
                '.auto{width:calc-size(auto, size + 20px);height:calc-size(any, 20px)}'
                '.share{width:calc-size(50%, size - 10px)}'
                '.doubled{width:calc-size(min-content, size * 2);height:max-content}'
                'img{width:160pt;height:20pt}',
                f'<svg width="160pt" height="{3 * band}pt" viewBox="0 0 160 {3 * band}">'
                '<rect class="auto" fill="blue"/>'
                f'<rect class="share" y="{band}" height="{band}" fill="blue"/>'
                f'<image class="doubled" y="{2 * band}" preserveAspectRatio="none" '
                f'href="data:image/svg+xml,{square}"/></svg>'
                f'<img src="data:image/svg+xml,{standalone}">')
            self.assertEqual(self.ink_columns(pixels, 0, 0, 160, band, BLUE), list(range(20)),
                             'an auto basis gives a rect no width, which the calculation adds to')
            self.assertEqual(self.ink_columns(pixels, 0, band, 160, band, BLUE), list(range(70)),
                             'a 50% basis is half the viewport before the calculation')
            self.assertEqual(self.ink_columns(pixels, 0, 2 * band, 160, 10, GREEN),
                             list(range(20)),
                             'an intrinsic basis behaves as auto: the natural width, doubled')
            self.assertEqual(self.ink_columns(pixels, 0, 2 * band + 10, 160, 5, GREEN), [],
                             'max-content behaves as auto: the image height stays natural')
            self.assertEqual(self.ink_columns(pixels, 0, 3 * band, 160, band, BLUE),
                             list(range(30)), 'a standalone SVG image calculates its sizes too')

    def assertSpan(self, columns, start, end, label):
        """`columns` run from `start` up to `end`, give or take the column at either edge, which
        the resampled soft mask may only partly cover."""
        self.assertTrue(columns and abs(columns[0] - start) <= 1
                        and abs(columns[-1] - (end - 1)) <= 1
                        and columns == list(range(columns[0], columns[-1] + 1)),
                        f'{label}: ink columns {columns}, expected {start} to {end - 1}')

    def test_css_mask_layers_mask_svg_content_in_its_boxes(self):
        """CSS Masking 1 §7, §8 on SVG content: image, gradient and mask element layers in
        the fill, stroke and view boxes, mask-mode and mask-composite, a mask border, and the
        raster renderer under a filter and inside a referenced mask element."""
        cell = 30
        half = cell // 2
        left = 'linear-gradient(to right, black 50%, transparent 50%)'
        top = 'linear-gradient(black 50%, transparent 50%)'
        canvas = Image.new('RGBA', (2, 1))
        canvas.putpixel((0, 0), (0, 0, 0, 255))
        encoded = io.BytesIO()
        canvas.save(encoded, 'PNG')
        image = (f'mask-image:url(data:image/png;base64,{base64.b64encode(encoded.getvalue()).decode()});'
                 'mask-size:100% 100%;image-rendering:pixelated')
        layers = [
            ('image', image),
            ('gradient', f'mask-image:{left}'),
            ('intersect', f'mask:linear-gradient(black,black) 0 0/50% 100% no-repeat,{left};'
                          'mask-composite:intersect'),
            ('element', f'mask:url(#half),{top};mask-composite:intersect'),
            ('luminance', 'mask-image:linear-gradient(to right, white 50%, black 50%);'
                          'mask-mode:luminance'),
            ('subtract', 'mask-image:linear-gradient(black,black),'
                         'linear-gradient(to right, transparent 50%, black 50%);'
                         'mask-composite:subtract'),
            ('border', f'mask-border:{left} 50% fill/{half}px'),
        ]
        rules = ''.join(f'.{name}{{{declarations}}}' for name, declarations in layers)
        shapes = ''.join(f'<rect class="{name}" x="{index * cell}" width="{cell}" '
                         f'height="{cell}" fill="rgb(0,160,0)"/>'
                         for index, (name, _) in enumerate(layers))
        width = len(layers) * cell
        boxes = (f'<rect class="fill" x="10" y="{cell + 10}" width="{half}" height="{half}" '
                 'fill="rgb(0,160,0)" stroke="blue" stroke-width="10"/>'
                 f'<rect class="view" y="{2 * cell}" width="{width}" height="{cell}" '
                 'fill="rgb(0,160,0)"/>'
                 f'<g filter="url(#same)"><rect class="image" x="{cell}" y="{cell}" '
                 f'width="{cell}" height="{cell}" fill="rgb(0,160,0)"/></g>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-css-') as directory:
            pixels = self.render(
                Path(directory),
                rules + '.fill{mask-image:linear-gradient(black,black);mask-clip:fill-box}'
                '.view{mask:linear-gradient(black,black) 0 0/50% 100% no-repeat view-box}'
                f'.masked{{width:{cell}pt;height:{cell}pt;background:rgb(0,160,0);'
                'mask:url(#nested)}',
                f'<svg width="{width}pt" height="{3 * cell}pt" viewBox="0 0 {width} {3 * cell}">'
                f'<defs>{IDENTITY_FILTER}'
                f'<mask id="half"><rect x="{3 * cell}" width="{half}" height="{cell}" '
                'fill="white"/></mask>'
                f'<mask id="nested" maskContentUnits="objectBoundingBox">'
                '<rect class="image" width="1" height="1" fill="white"/></mask>'
                f'</defs>{shapes}{boxes}</svg><div class="masked"></div>',
                CAIRO)
            for index, (name, _) in enumerate(layers):
                with self.subTest(layer=name):
                    self.assertSpan(self.ink_columns(pixels, index * cell, 0, cell, half, GREEN),
                                    index * cell, index * cell + half,
                                    f'{name}: only the left half of the square shows')
            self.assertEqual(self.ink_columns(pixels, 3 * cell, half + 1, cell, half - 1, GREEN),
                             [], 'intersect with the top half hides the bottom half')
            self.assertPixel(pixels, 12, cell + 17, BLUE,
                             'mask-clip: fill-box keeps the stroke inside the fill box')
            self.assertPixel(pixels, 7, cell + 17, WHITE,
                             'mask-clip: fill-box clips the stroke outside the fill box')
            self.assertSpan(self.ink_columns(pixels, 0, 2 * cell, width, cell, GREEN), 0,
                            width // 2, 'mask-origin: view-box sizes the layer from the viewport')
            self.assertSpan(self.ink_columns(pixels, cell, cell, cell, cell, GREEN), cell,
                            cell + half, 'the raster renderer masks content under a filter')
            self.assertSpan(self.ink_columns(pixels, 0, 3 * cell, cell, cell, GREEN), 0, half,
                            'a mask element masks with the CSS mask layers of its content')

    def assertProjectedEdges(self, pixels, project, rect, inside, label):
        """Pixels just inside each projected corner region of `rect` show `inside`, and pixels
        just outside it the page."""
        left, top, right, bottom = rect
        for (x, y), sign in (((left, top), 1), ((right, top), -1), ((right, bottom), -1),
                             ((left, bottom), 1)):
            vertical = 1 if y == top else -1
            for offset, wanted, where in ((4, inside, 'inside'), (-4, WHITE, 'outside')):
                px, py = project(x + sign * offset, y + vertical * (bottom - top) / 4)
                self.assertPixel(pixels, int(px), int(py), wanted,
                                 f'{label}: {where} the projected edge near corner {(x, y)}, '
                                 f'at ({px:.1f}, {py:.1f})')

    def test_perspective_projects_svg_content_through_the_homography(self):
        # CSS Transforms 2 §4.1.1, §8, §11: a rectangle turned 40° about (100, 50), on the y axis,
        # puts a point dx from it at depth -sin·dx; a perspective d about origin p then draws
        # it at p + (point - p) / w with w = 1 + sin·dx / d. The first svg gives its children
        # its 300pt perspective about its view-box centre; in the second, taller, a g
        # gives its own about the origin. One user unit is one point.
        width, height, depth = 200, 100, 300

        def projector(origin, top):
            sin, cos = math.sin(math.radians(40)), math.cos(math.radians(40))

            def project(x, y):
                dx = x - width / 2
                w = 1 + sin * dx / depth
                return (origin[0] + (width / 2 + cos * dx - origin[0]) / w,
                        top + origin[1] + (y - origin[1]) / w)
            return project

        svg = ('<svg width="{0}pt" height="{1}pt" viewBox="0 0 {0} {1}" class="{2}">{3}</svg>')
        rect = (f'<rect width="{width}" height="{height}" fill="blue" '
                'style="transform:rotateY(40deg);transform-origin:100px 50px"/>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-3d-') as directory:
            pixels = self.render(
                Path(directory),
                f'.p{{perspective:{depth}px}}g{{perspective:{depth}px;perspective-origin:0 0}}',
                svg.format(width, height, 'p', rect)
                + svg.format(width, height * 3 // 2, '', f'<g>{rect}</g>'))
            self.assertProjectedEdges(pixels, projector((width / 2, height / 2), 0),
                                      (0, 0, width, height), BLUE,
                                      'the svg element\'s perspective')
            self.assertProjectedEdges(pixels, projector((0, 0), height),
                                      (0, 0, width, height), BLUE,
                                      'the g element\'s perspective about its origin')

    def test_projected_svg_text_stays_text_at_its_projection(self):
        # The text turns 25° about (100, 50) under a 600pt perspective about the same point: a
        # glyph origin dx from it draws at x = 100 + cos·dx / w, y = 50 + dy / w, with
        # w = 1 + sin·dx / 600, within the half pixel at 288 dpi that keeps it vector text.
        width, height, depth, angle = 200, 100, 600, math.radians(25)
        css = f'svg{{perspective:{depth}px}}'
        body = (f'<svg width="{width}pt" height="{height}pt" viewBox="0 0 {width} {height}">'
                '<text x="20" y="50" font-size="14"{}>Tilted plane</text></svg>')
        turn = ' style="transform:rotateY(25deg);transform-origin:100px 50px"'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-3d-') as directory:
            root = Path(directory)
            flat_glyphs = [(glyph.text, glyph.origin)
                           for glyph in pdf_content.inspect(self.output(root, css, body.format('')))['glyphs']]
            tilted = pdf_content.inspect(self.output(root, css, body.format(turn)))['glyphs']
        self.assertEqual([glyph.text for glyph in tilted], [text for text, _ in flat_glyphs],
                         'every glyph stays text, in order, under the perspective')
        for (text, (x, y)), glyph in zip(flat_glyphs, tilted):
            dx, dy = x - PAGE_MARGIN - width / 2, PAGE - y - PAGE_MARGIN - height / 2
            w = 1 + math.sin(angle) * dx / depth
            expected = (PAGE_MARGIN + width / 2 + math.cos(angle) * dx / w,
                        PAGE - (PAGE_MARGIN + height / 2 + dy / w))
            self.assertLess(math.dist(glyph.origin, expected), 0.15,
                            f'{text!r} at {(x, y)} must draw at its projection {expected}: '
                            f'got {glyph.origin}')

    def test_svg_3d_rendering_contexts_sort_planes_by_depth_and_hide_back_faces(self):
        # CSS Transforms 2 §4.1.2, §11: the g's preserve-3d puts both rectangles, turned ±40° about
        # (100, 50) under the svg's 300pt perspective, in one 3D rendering context, so each
        # point shows the nearer plane whatever the painting order. A point dx from the centre
        # on the plane turned by θ lies at depth -sin θ·dx.
        width, height, depth = 200, 100, 300
        centre = (width / 2, height / 2)

        def nearest(screen_x):
            found = []
            for angle, colour in ((40, RED), (-40, BLUE)):
                sin, cos = math.sin(math.radians(angle)), math.cos(math.radians(angle))
                offset = screen_x - centre[0]
                dx = offset / (cos - sin * offset / depth)
                w = 1 + sin * dx / depth
                if w > 0 and abs(dx) <= width / 2:
                    found.append((-sin * dx / w, colour))
            return max(found)[1] if found else WHITE

        rect = (f'<rect width="{width}" height="{height}" fill="{{}}" '
                'style="transform:rotateY({}deg);transform-origin:100px 50px"/>')
        planes = [rect.format('red', 40), rect.format('blue', -40)]
        # The card turns its front face away, which hides it; its back, turned again inside the
        # card's context, faces the viewer (§10). A flat rectangle turned away hides too.
        card = ('<g style="transform-style:preserve-3d;transform:rotateY(180deg);'
                'transform-origin:20px 20px">'
                '<rect width="40" height="40" fill="red" style="backface-visibility:hidden"/>'
                '<rect width="40" height="40" fill="blue" style="backface-visibility:hidden;'
                'transform:rotateY(180deg);transform-origin:20px 20px"/></g>'
                '<rect x="60" width="40" height="40" fill="red" style="backface-visibility:'
                'hidden;transform:rotateY(180deg);transform-origin:80px 20px"/>')
        svg = (f'<svg width="{width}pt" height="{height}pt" viewBox="0 0 {width} {height}">'
               '{}</svg>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-3d-') as directory:
            for order in (planes, planes[::-1]):
                pixels = self.render(
                    Path(directory), f'svg{{perspective:{depth}px}}',
                    svg.format(f'<g style="transform-style:preserve-3d">{"".join(order)}</g>')
                    + svg.format(card))
                for offset in (-70, -40, -10, 10, 40, 70):
                    x = int(centre[0] + offset)
                    self.assertPixel(pixels, x, int(centre[1]), nearest(x + 0.5),
                                     f'the nearest plane shows at x = {x} whatever the order')
                self.assertPixel(pixels, 20, height + 20, BLUE,
                                 'only the card\'s back face, facing the viewer, paints')
                self.assertPixel(pixels, 80, height + 20, WHITE,
                                 'a flat rectangle facing away with a hidden back face paints '
                                 'nothing')

    def test_clip_paths_ignore_3d_transform_lists(self):
        # CSS Transforms 2 §11: a clipPath ignores a transform list holding a 3D function, so
        # its 20pt square clips untransformed; a 2D list still moves it.
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-3d-') as directory:
            pixels = self.render(
                Path(directory), '#tilt{transform:rotateY(60deg)}#moved{transform:translate(50px)}',
                f'<svg width="{2 * ICON}pt" height="{ICON}pt" viewBox="0 0 {2 * ICON} {ICON}">'
                f'<clipPath id="tilt"><rect width="{SQUARE}" height="{SQUARE}"/></clipPath>'
                f'<clipPath id="moved"><rect width="{SQUARE}" height="{SQUARE}"/></clipPath>'
                f'<rect width="{ICON}" height="{ICON}" fill="blue" clip-path="url(#tilt)"/>'
                f'<rect x="{ICON}" width="{ICON}" height="{ICON}" fill="blue" '
                'clip-path="url(#moved)"/></svg>')
            self.assertPixel(pixels, SQUARE - 2, SQUARE // 2, BLUE,
                             'the clip keeps its whole untransformed square')
            self.assertPixel(pixels, SQUARE + 2, SQUARE // 2, WHITE,
                             'the clip still clips past its square')
            self.assertPixel(pixels, ICON + 5, SQUARE // 2, WHITE,
                             'the 2D list moves its clip 50pt right, off the start of the second square')
            self.assertPixel(pixels, ICON + SQUARE, SQUARE // 2, BLUE,
                             'the moved clip keeps 50pt to 70pt')

    def output(self, root, css, body):
        """The PDF rendered from `css` and `body`."""
        completed, output = self.convert(root, css, body)
        self.assertEqual(completed.returncode, 0, f'render failed: {completed.stderr}')
        rendered = root / f'{len(list(root.glob("*.pdf")))}.pdf'
        output.rename(rendered)
        return rendered

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
