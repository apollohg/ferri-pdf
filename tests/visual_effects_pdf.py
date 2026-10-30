"""Rendered checks for outlines, blending and individual transform properties."""
import math
from pathlib import Path
import tempfile
import unittest

from oracle import pdf_content
import table_positioning_pdf

BODY = '<div class=box>Box</div>'
BOX = '.box{width:80pt;height:40pt;margin:60pt;background:teal;color:white}'
NORMAL = 'Normal'
RED = (255, 0, 0)
WHITE = (255, 255, 255)
TEAL = (0, 128, 128)


def groups(path):
    """Form XObjects drawn on page one as nested `(blend mode, isolated, children)` tuples,
    with the blend mode of the graphics state each is drawn in (PDF 32000 §11.6.3)."""
    document = pdf_content.Document(path)

    def walk(data, resources):
        resources = document.resolve(resources) or {}
        states = document.resolve(resources.get('ExtGState')) or {}
        forms = document.resolve(resources.get('XObject')) or {}
        found, operands, stack, blend, position = [], [], [], NORMAL, 0
        while (position := pdf_content.skip_space(data, position)) < len(data):
            value, position = pdf_content.parse(data, position)
            if not isinstance(value, pdf_content.Operator):
                operands.append(value)
                continue
            if value == 'q':
                stack.append(blend)
            elif value == 'Q':
                blend = stack.pop()
            elif value == 'gs':
                state = document.resolve(states[operands[0]])
                blend = str(state.get('BM', blend))
            elif value == 'Do':
                form = document.resolve(forms[operands[0]])
                if form.entries.get('Subtype') == 'Form':
                    group = document.resolve(form.entries.get('Group')) or {}
                    found.append((blend, bool(group.get('I')),
                                  walk(document.decode(form), form.entries.get('Resources'))))
            operands = []
        return found

    page = document.pages()[0]
    data = b''.join(document.decode(document.resolve(reference))
                    for reference in document.contents(page))
    return walk(data, page.get('Resources'))


def blend_modes(tree):
    """Every blend mode a group is drawn with, depth first."""
    return [mode for blend, _, children in tree for mode in [blend, *blend_modes(children)]]


class VisualEffectsPdf(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix='ferri-pdf-visual-effects-')
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def pixels(self, css, body=BODY):
        output = self.renderer.render(self.root, css, body)
        with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
            return raster.copy()

    def test_individual_transforms_paint_as_the_equivalent_transform(self):
        untransformed = self.pixels(BOX).tobytes()
        for individual, transform in (
                ('translate:12pt 5pt;rotate:20deg;scale:1.5 .5',
                 'transform:translate(12pt,5pt) rotate(20deg) scale(1.5,.5)'),
                ('translate:10%;transform:rotate(-15deg) skewX(10deg)',
                 'transform:translateX(10%) rotate(-15deg) skewX(10deg)'),
                ('rotate:y 50deg;transform-origin:0 0',
                 'transform:rotateY(50deg);transform-origin:0 0')):
            with self.subTest(individual=individual):
                expected = self.pixels(BOX + f'.box{{{transform}}}').tobytes()
                self.assertNotEqual(expected, untransformed, f'{transform} must move ink')
                self.assertEqual(self.pixels(BOX + f'.box{{{individual}}}').tobytes(), expected,
                                 f'{individual} must paint exactly as {transform}')

    def test_perspective_projects_ink_text_and_links_through_the_homography(self):
        # The page content starts at (20, 20); .p is 200pt by 100pt with a 300pt perspective
        # about its centre (120, 70), and .t turns 40° about the same centre. A point dx, dy
        # from it goes to depth z = -sin·dx, so w = 1 + sin·dx / 300 (CSS Transforms 2 §4.2).
        left, top, width, height, depth, angle = 20, 20, 200, 100, 300, math.radians(40)
        centre = (left + width / 2, top + height / 2)

        def project(x, y):
            dx, dy = x - centre[0], y - centre[1]
            w = 1 + math.sin(angle) * dx / depth
            return centre[0] + math.cos(angle) * dx / w, centre[1] + dy / w

        css = ('.p{perspective:%dpt;width:%dpt;height:%dpt}'
               '.t{height:100%%;background:teal;color:white}a{color:white}'
               % (depth, width, height))
        body = '<div class=p><div class=t>Tilt <a href="https://example.com/">far</a> away</div></div>'
        flat = self.renderer.render(self.root, css, body)
        flat_glyphs = [(g.text, g.origin) for g in pdf_content.inspect(flat)['glyphs']]
        flat_link = pdf_content.navigation(flat)['links'][0]['rect']
        tilted = self.renderer.render(self.root, css + '.t{transform:rotateY(40deg)}', body)
        page_height = table_positioning_pdf.PAGE_HEIGHT
        glyphs = [(g.text, g.origin) for g in pdf_content.inspect(tilted)['glyphs']]
        self.assertEqual([text for text, _ in glyphs], [text for text, _ in flat_glyphs],
                         'every glyph stays text, in order, under the perspective')
        for (text, (x, y)), (_, actual) in zip(flat_glyphs, glyphs):
            expected_x, expected_y = project(x, page_height - y)
            expected = (expected_x, page_height - expected_y)
            self.assertLess(math.dist(actual, expected), 0.15,
                            f'{text!r} at {(x, y)} must draw at its projection {expected}, '
                            f'within half a 288 dpi pixel: got {actual}')
        x0, y1, x1, y0 = flat_link
        corners = [project(x, page_height - y) for x, y in ((x0, y0), (x1, y0), (x1, y1), (x0, y1))]
        quads = pdf_content.navigation(tilted)['links'][0]['quads']
        self.assertIsNotNone(quads, 'a projected link area is a quadrilateral')
        actual = sorted(zip(quads[0::2], quads[1::2]))
        expected = sorted((x, page_height - y) for x, y in corners)
        for point, wanted in zip(actual, expected):
            self.assertLess(math.dist(point, wanted), 0.05,
                            f'link corners {actual} must be the projected corners {expected}')
        with self.renderer.raster(self.root, tilted, vector_antialias=False) as pixels:
            inset, outset = 4, 4
            for (x, y), sign in (((left, top), 1), ((left + width, top), -1),
                                 ((left + width, top + height), -1), ((left, top + height), 1)):
                vertical = 1 if y == top else -1
                for (px, py), wanted, where in (
                        (project(x + sign * inset, y + vertical * 30), TEAL, 'inside'),
                        (project(x - sign * outset, y + vertical * 30), WHITE, 'outside')):
                    found = pixels.getpixel((int(px), int(py)))
                    self.assertTrue(all(abs(a - b) <= 2 for a, b in zip(found, wanted)),
                                    f'{where} the projected edge near corner {(x, y)}, '
                                    f'({px:.1f}, {py:.1f}) must be {wanted}, got {found}')

    def test_outline_paints_outside_the_offset_following_the_grown_radius(self):
        # The border box spans 80..160 by 80..120; a 4pt outline 3pt outside it spans 73..167
        # by 73..127 with 19pt outer corner radii (12pt + 3pt + 4pt).
        pixels = self.pixels(BOX + '.box{border-radius:12pt;outline:4pt solid red;'
                                   'outline-offset:3pt}')
        for (x, y), expected, reason in (
                ((120, 75), RED, 'the outline band above the box'),
                ((165, 100), RED, 'the outline band right of the box'),
                ((120, 78), WHITE, 'the offset leaves a gap outside the border edge'),
                ((74, 74), WHITE, 'the outer corner is rounded'),
                ((80, 80), RED, 'the band follows the corner curve diagonally'),
                ((100, 100), TEAL, 'the box itself')):
            self.assertEqual(pixels.getpixel((x, y)), expected, f'{reason} at {(x, y)}')
        flow = self.pixels(BOX + '.box{outline:4pt solid red}', BODY + '<p>After</p>')
        plain = self.pixels(BOX, BODY + '<p>After</p>')
        lower = (20, 130, 300, 300)
        self.assertEqual(flow.crop(lower).tobytes(), plain.crop(lower).tobytes(),
                         'outlines take no space, so later content stays in place')

    def test_position_longhands_place_layers_as_the_shorthand_does(self):
        layer = (BOX + '.box{background-image:linear-gradient(red,red),linear-gradient(blue,blue);'
                 'background-size:20pt 10pt;background-repeat:no-repeat}')
        expected = self.pixels(layer + '.box{background-position:right 5pt top 10pt,'
                                       'left 5pt bottom 0}')
        self.assertNotEqual(expected.tobytes(), self.pixels(layer).tobytes(),
                            'the probe positions must move the layers')
        actual = self.pixels(layer + '.box{background-position-x:right 5pt,5pt;'
                                     'background-position-y:10pt,bottom}')
        self.assertEqual(actual.tobytes(), expected.tobytes(),
                         'background-position-x and -y must place each layer as the shorthand')

    def groups(self, css, body):
        return groups(self.renderer.render(self.root, css, body))

    def test_blended_element_is_a_blend_group_inside_its_isolated_stacking_context(self):
        body = ('<div class=context><div class=under></div><div class=blend>B</div></div>'
                '<div class=after></div>')
        css = ('div{height:30pt}.context{position:relative;z-index:1;height:60pt;background:yellow}'
               '.under{background:#ff8080}.blend{mix-blend-mode:multiply;background:#8080ff;'
               'margin-top:-15pt}.after{background:lime}')
        pixels = self.pixels(css, body)
        # B × S per channel (Compositing 1 §9.1.1), over each backdrop in the stacking context.
        for (x, y), expected, backdrop in (((60, 45), (128, 64, 128), '#ff8080'),
                                           ((60, 60), (128, 128, 0), 'yellow')):
            actual = pixels.getpixel((x, y))
            self.assertTrue(all(abs(a - e) <= 1 for a, e in zip(actual, expected)),
                            f'#8080ff multiplied over {backdrop} at {(x, y)} must be {expected}, '
                            f'got {actual}')
        tree = self.groups(css, body)
        self.assertEqual(len(tree), 1, f'one group for the stacking context: {tree}')
        context_blend, context_isolated, children = tree[0]
        self.assertEqual((context_blend, context_isolated), (NORMAL, True),
                         'the z-index stacking context composites normally as an isolated group, '
                         f'so its content cannot blend with the page below it: {tree}')
        self.assertEqual([(blend, isolated) for blend, isolated, _ in children],
                         [('Multiply', True)],
                         f'the blended element is an isolated group drawn with /BM /Multiply: {tree}')
        self.assertEqual(self.groups(css.replace('mix-blend-mode:multiply;', ''), body), [],
                         'without blending neither stacking context needs a group')

    def test_every_separable_and_non_separable_mode_maps_to_its_pdf_blend_mode(self):
        for css, pdf in (('multiply', 'Multiply'), ('screen', 'Screen'), ('overlay', 'Overlay'),
                         ('darken', 'Darken'), ('lighten', 'Lighten'),
                         ('color-dodge', 'ColorDodge'), ('color-burn', 'ColorBurn'),
                         ('hard-light', 'HardLight'), ('soft-light', 'SoftLight'),
                         ('difference', 'Difference'), ('exclusion', 'Exclusion'),
                         ('hue', 'Hue'), ('saturation', 'Saturation'), ('color', 'Color'),
                         ('luminosity', 'Luminosity')):
            with self.subTest(mode=css):
                tree = self.groups(f'div{{height:20pt;background:red;mix-blend-mode:{css}}}',
                                   '<div></div>')
                self.assertEqual(blend_modes(tree), [pdf],
                                 f'mix-blend-mode:{css} blends with the page as /BM /{pdf}')

    def test_isolation_and_opacity_keep_descendants_from_blending_with_the_page(self):
        body = '<div class=outer><div class=inner>A</div></div>'
        for outer, expected in (('isolation:isolate', [(NORMAL, True)]),
                                ('opacity:.5', [(NORMAL, True)])):
            with self.subTest(outer=outer):
                tree = self.groups(f'div{{height:20pt}}.outer{{{outer}}}'
                                   '.inner{mix-blend-mode:difference;background:red}', body)
                self.assertEqual([(blend, isolated) for blend, isolated, _ in tree], expected,
                                 f'{outer} isolates its content: {tree}')
                self.assertEqual(blend_modes(tree[0][2]), ['Difference'],
                                 f'the descendant blends inside the isolated group: {tree}')

    def test_background_layers_blend_with_lower_layers_inside_an_isolated_group(self):
        tree = self.groups('div{height:40pt;background-color:yellow;'
                           'background-image:linear-gradient(red,blue),'
                           'linear-gradient(green,white);'
                           'background-blend-mode:screen,multiply}', '<div></div>')
        self.assertEqual(len(tree), 1, f'one isolated group holds the background: {tree}')
        self.assertEqual(tree[0][:2], (NORMAL, True),
                         f'background layers must not blend with content behind the box: {tree}')
        document = pdf_content.Document(self.root / 'output.pdf')
        states = [document.resolve(state) for page in document.pages()
                  for reference in pdf_content.content_streams(document)
                  for state in (document.resolve((document.resolve(
                      document.resolve(reference).entries.get('Resources')) or {})
                      .get('ExtGState')) or {}).values()]
        modes = sorted({str(state['BM']) for state in states if 'BM' in state})
        self.assertEqual(modes, ['Multiply', 'Screen'],
                         'each image layer is drawn with its own blend mode')


if __name__ == '__main__':
    unittest.main()
