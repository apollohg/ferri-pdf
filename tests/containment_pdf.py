"""Compare aspect ratios, containment, container queries and sticky positioning in rendered
PDFs against independently fixed geometry that draws the same pixels."""
import dataclasses
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf
from oracle import pdf_content

LINK = '<a href="mailto:reader@example.org">Stuck</a>'
# The 260pt page area holds the spacer, so the 30pt sticky box starts the second page.
SPACER = '<div style="height:255pt"></div>'
STICKY_INSET = 40
STICKY_CSS = '.sec{height:400pt;background:#ddd}.s{height:30pt;background:blue}a{color:black}'
# A shift drawn as a translation and one folded into coordinates differ in float rounding.
MATRIX_DIGITS = 6


def glyphs(output):
    return [dataclasses.replace(glyph, matrix=tuple(round(value, MATRIX_DIGITS)
                                                    for value in glyph.matrix))
            for glyph in pdf_content.inspect(output)['glyphs']]


class ContainmentPdf(unittest.TestCase):
    def assert_same_output(self, actual_css, reference_css, body, pages=1):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-containment-') as directory:
            root = Path(directory)
            renderer = table_positioning_pdf.TablePositioningPdf()
            reference = renderer.render(root, reference_css, body)
            expected_glyphs = glyphs(reference)
            expected_links = pdf_content.navigation(reference)['links']
            expected_pages = []
            for page in range(1, pages + 1):
                with renderer.raster(root, reference, vector_antialias=False, page=page) as raster:
                    expected_pages.append(raster.tobytes())
            actual = renderer.render(root, actual_css, body)
            self.assertEqual(glyphs(actual), expected_glyphs,
                             f'glyphs of `{actual_css}` against `{reference_css}`')
            self.assertEqual(pdf_content.navigation(actual)['links'], expected_links,
                             f'links of `{actual_css}` against `{reference_css}`')
            for page in range(1, pages + 1):
                with renderer.raster(root, actual, vector_antialias=False, page=page) as raster:
                    self.assertEqual(raster.tobytes(), expected_pages[page - 1],
                                     f'page {page} pixels of `{actual_css}` against '
                                     f'`{reference_css}`')
            return expected_glyphs

    def test_sticky_box_starting_a_page_shifts_like_relative_positioning(self):
        body = f'<div class=sec>{SPACER}<div class=s>{LINK}</div></div>'
        # The containing block constrains the shift whether or not it paints anything.
        for containing in ('.sec{background:#ddd}', '.sec{background:none}'):
            with self.subTest(containing):
                glyphs = self.assert_same_output(
                    STICKY_CSS + containing + f'.s{{position:sticky;top:{STICKY_INSET}pt}}',
                    STICKY_CSS + containing + f'.s{{position:relative;top:{STICKY_INSET}pt}}',
                    body, pages=2)
                self.assertEqual({glyph.page for glyph in glyphs}, {2},
                                 'the sticky box must move whole to the second page')

    def test_sticky_box_stops_at_its_containing_block_and_uses_a_scroll_container(self):
        # The 60pt containing block leaves room for a 30pt shift of the 30pt box only.
        css = '.sec{height:60pt}.s{height:30pt;background:blue}'
        self.assert_same_output(css + '.s{position:sticky;top:200pt}',
                                css + '.s{position:relative;top:30pt}',
                                '<div class=sec><div class=s></div></div>')
        # The scrollport is the scroll container's padding box, 10pt below the page area.
        css = '.scroll{overflow:hidden;height:100pt;margin-top:10pt;padding-top:5pt}' \
              '.s{height:20pt;background:blue}'
        self.assert_same_output(css + '.s{position:sticky;top:15pt}',
                                css + '.s{position:relative;top:10pt}',
                                '<div class=scroll><div class=s></div></div>')

    def test_sticky_box_already_inside_its_view_does_not_move(self):
        body = f'<div class=sec><div class=s>{LINK}</div></div>'
        self.assert_same_output(STICKY_CSS + '.s{position:sticky;top:0;bottom:0}', STICKY_CSS,
                                body)

    def test_aspect_ratio_and_contain_intrinsic_size_draw_explicit_boxes(self):
        for actual, reference in [
            ('.a{width:100pt;aspect-ratio:2;background:blue}',
             '.a{width:100pt;height:50pt;background:blue}'),
            ('.a{height:40pt;aspect-ratio:3/2;float:left;background:blue}',
             '.a{height:40pt;width:60pt;float:left;background:blue}'),
            ('.a{contain:size;contain-intrinsic-size:70pt 30pt;float:left;background:blue}',
             '.a{width:70pt;height:30pt;float:left;background:blue}'),
        ]:
            with self.subTest(actual):
                self.assert_same_output(actual, reference, '<div class=a></div>')

    def test_paint_containment_clips_and_hidden_contents_are_skipped(self):
        self.assert_same_output(
            '.c{contain:paint;width:100pt;height:40pt}.o{width:200pt;height:80pt;background:blue}',
            '.c{width:100pt;height:40pt}.o{width:100pt;height:40pt;background:blue}',
            '<div class=c><div class=o></div></div>')
        glyphs = self.assert_same_output(
            '.c{content-visibility:hidden;width:100pt;padding:5pt;background:blue}',
            '.c{width:100pt;padding:5pt;background:blue}.c *{display:none}',
            '<div class=c><span>Skipped</span></div>')
        self.assertEqual(glyphs, [], 'skipped contents draw no glyphs')

    def test_container_queries_restyle_descendants_to_the_measured_size(self):
        self.assert_same_output(
            '.c{container-type:inline-size;width:200pt}.x{height:20pt;width:25cqw;background:red}'
            '@container (width > 150pt){.x{background:green}}',
            '.c{width:200pt}.x{height:20pt;width:50pt;background:green}',
            '<div class=c><div class=x></div></div>')


if __name__ == '__main__':
    unittest.main()
