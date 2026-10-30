"""Rendered inline SVG styled by the document's cascade (SVG 2 §6.4): class rules, presentation
attributes, inheritance from HTML, `use` instances of a sprite, CSS transforms, clipping, masks,
overflow and direction on SVG content, and the isolation of SVG images from the document's style
sheets, read back through Poppler."""
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

    def render(self, root, css, body):
        completed, output = self.convert(root, css, body)
        self.assertEqual(completed.returncode, 0, f'render failed: {completed.stderr}')
        subprocess.run(['pdftoppm', '-r', str(RASTER_DPI), '-aaVector', 'no', '-singlefile', '-png',
                        str(output), str(root / 'ink')], check=True, capture_output=True)
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
