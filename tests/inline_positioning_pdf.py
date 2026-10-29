"""Rendered checks that relatively and sticky positioned inline boxes move their text,
backgrounds, decorations and link areas together (CSS Position 3 §3.3, §3.4)."""
import dataclasses
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image
from oracle import pdf_content
import table_positioning_pdf
from table_positioning_pdf import OFFSET_X, OFFSET_Y, PAGE_HEIGHT

WHITE = (255, 255, 255)
# A 72 dpi raster maps points to pixels, so whole-point offsets move ink by whole pixels.
RASTER_DPI = 72
# The span wraps in the 120pt block; its text is capitals so glyphs outside it are told apart.
SPAN_CSS = ('.w{width:120pt}.r{background:#ff0000;text-decoration:underline #0000ff 1pt}'
            'a{color:black;text-decoration:none}')
SPAN_BODY = ('<div class=w>plain <span class=r>MOVED WORDS WRAP OVER LINES <a href="#t">LINK</a> '
             'END</span> tail</div><p id=t>after</p>')
RELATIVE = f'.r{{position:relative;left:{OFFSET_X}pt;top:{OFFSET_Y}pt}}'
# DejaVu Sans at 10pt/14pt: the content area is (1901 + 483) / 2048 em tall with equal
# half-leading, so the fourth line's inline box ends 3 * 14 + 12.8203125pt down. A scrollport
# this tall makes `bottom:0` lift it by exactly the shift.
STICKY_SHIFT = 14
SCROLLPORT_HEIGHT = 3 * 14 + 12.8203125 - STICKY_SHIFT
STICKY_BODY = ('<div class=scroll><div>one<br>two<br>three<br>four '
               '<span class=s><a href="mailto:reader@example.org">STUCK</a></span></div></div>')
STICKY_CSS = (f'.scroll{{overflow:hidden;height:{SCROLLPORT_HEIGHT}pt;background:#dddddd}}'
              '.s{background:#ff0000;text-decoration:underline}a{color:black}')
# A shift drawn as a translation and one folded into coordinates differ in float rounding.
MATRIX_DIGITS = 6


def raster(root, output):
    """Page one without anti-aliasing, so moved ink keeps its exact pixels."""
    subprocess.run(['pdftoppm', '-r', str(RASTER_DPI), '-aa', 'no', '-aaVector', 'no',
                    '-singlefile', '-png', str(output), str(root / 'ink')],
                   check=True, capture_output=True)
    with Image.open(root / 'ink.png') as image:
        return image.convert('RGB')


def shifted(image, dx, dy):
    moved = Image.new('RGB', image.size, WHITE)
    moved.paste(image, (dx, dy))
    return moved


def rounded(glyphs):
    return [dataclasses.replace(glyph, matrix=tuple(round(value, MATRIX_DIGITS)
                                                    for value in glyph.matrix))
            for glyph in glyphs]


class InlinePositioningPdf(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix='ferri-pdf-inline-positioning-')
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def render(self, css, body):
        """Each render in its own directory, so that compared outputs coexist."""
        return self.renderer.render(Path(tempfile.mkdtemp(dir=self.root)), css, body)

    def test_relative_span_moves_text_background_decorations_and_link_by_its_offset(self):
        normal = self.render(SPAN_CSS, SPAN_BODY)
        moved = self.render(SPAN_CSS + RELATIVE, SPAN_BODY)
        # The positioned span paints after the flow around it, each part in tree order.
        def parts(output):
            glyphs = [glyph for glyph in pdf_content.inspect(output)['glyphs']
                      if not glyph.text.isspace()]
            return {inside: [glyph for glyph in glyphs if glyph.text.isupper() == inside]
                    for inside in (True, False)}
        before, after = parts(normal), parts(moved)
        lines = {round(glyph.origin[1], 3) for glyph in before[True]}
        self.assertGreater(len(lines), 2, f'the span must wrap onto several lines: {lines}')
        for inside, (dx, dy) in ((True, (OFFSET_X, -OFFSET_Y)), (False, (0, 0))):
            self.assertEqual([glyph.text for glyph in before[inside]],
                             [glyph.text for glyph in after[inside]],
                             f'the same glyphs {"inside" if inside else "outside"} the span')
            for old, new in zip(before[inside], after[inside]):
                self.assertAlmostEqual(new.origin[0] - old.origin[0], dx, places=3,
                                       msg=f'{old.text!r} horizontal offset')
                self.assertAlmostEqual(new.origin[1] - old.origin[1], dy, places=3,
                                       msg=f'{old.text!r} vertical offset')
        [old_link] = pdf_content.navigation(normal)['links']
        [new_link] = pdf_content.navigation(moved)['links']
        for index, offset in enumerate((OFFSET_X, -OFFSET_Y, OFFSET_X, -OFFSET_Y)):
            self.assertAlmostEqual(new_link['rect'][index] - old_link['rect'][index], offset,
                                   places=3, msg=f'link rect {index}: {old_link} -> {new_link}')
        # Only the span shows, so its background, underline and text are compared alone.
        alone = 'body{visibility:hidden}.r{visibility:visible}'
        expected = shifted(raster(self.root, self.render(SPAN_CSS + alone, SPAN_BODY)),
                           OFFSET_X, OFFSET_Y)
        actual = raster(self.root, self.render(SPAN_CSS + alone + RELATIVE, SPAN_BODY))
        self.assertNotEqual(expected.getbbox(), None)
        self.assertEqual(actual.tobytes(), expected.tobytes(),
                         'the span paints exactly its unpositioned pixels moved by the offset')
        rest = 'body{visibility:visible}.r{visibility:hidden}'
        self.assertEqual(raster(self.root, self.render(SPAN_CSS + rest + RELATIVE, SPAN_BODY))
                         .tobytes(),
                         raster(self.root, self.render(SPAN_CSS + rest, SPAN_BODY)).tobytes(),
                         'content around the span does not move')

    def test_sticky_span_shifts_into_its_scrollport_like_the_equivalent_relative_offset(self):
        sticky = self.render(STICKY_CSS + '.s{position:sticky;bottom:0}', STICKY_BODY)
        relative = self.render(STICKY_CSS + f'.s{{position:relative;top:-{STICKY_SHIFT}pt}}',
                               STICKY_BODY)
        expected = rounded(pdf_content.inspect(relative)['glyphs'])
        self.assertEqual(rounded(pdf_content.inspect(sticky)['glyphs']), expected,
                         'sticky glyphs against the relative reference')
        self.assertIn('S', [glyph.text for glyph in expected])
        stuck = [glyph for glyph in expected if glyph.text == 'S'][0]
        self.assertLess(PAGE_HEIGHT - stuck.origin[1], table_positioning_pdf.PAGE_MARGIN +
                        SCROLLPORT_HEIGHT, 'the reference lifts the span into the scrollport')
        self.assertEqual(pdf_content.navigation(sticky)['links'],
                         pdf_content.navigation(relative)['links'])
        self.assertEqual(raster(self.root, sticky).tobytes(),
                         raster(self.root, relative).tobytes(),
                         'sticky pixels against the relative reference')


if __name__ == '__main__':
    unittest.main()
