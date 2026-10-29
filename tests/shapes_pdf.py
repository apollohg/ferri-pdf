"""Rendered checks that text wraps around circle and polygon float areas (CSS Shapes 1): the
first glyph of each line box beside the float starts at the area's extent over that line."""
import math
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf
from oracle import pdf_content
from table_positioning_pdf import BASELINE_OFFSET, LINE_HEIGHT, PAGE_HEIGHT, PAGE_MARGIN

FLOAT_SIZE = 112
LINES_BESIDE = FLOAT_SIZE // LINE_HEIGHT
WORDS = ' '.join(['aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt'] * 6)
# Curves are flattened to within 0.05pt; glyph origins are written to three decimals.
TOLERANCE = 0.1


def circle_edge(top):
    """The right edge of the float's inscribed circle over the line box from `top`."""
    radius = center = FLOAT_SIZE / 2
    bottom = top + LINE_HEIGHT
    dy = 0 if top <= center <= bottom else min(abs(top - center), abs(bottom - center))
    return center + math.sqrt(max(radius * radius - dy * dy, 0))


def arrow_edge(top):
    """The edge of polygon(0 0, 100% 50%, 0 100%) over the line box from `top`."""
    middle = FLOAT_SIZE / 2
    bottom = top + LINE_HEIGHT
    nearest = 0 if top <= middle <= bottom else min(abs(top - middle), abs(bottom - middle))
    return FLOAT_SIZE * (1 - nearest / middle)


class ShapesPdf(unittest.TestCase):
    def line_starts(self, shape):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-shapes-') as directory:
            output = table_positioning_pdf.TablePositioningPdf().render(
                Path(directory),
                f'p{{margin:0}}.f{{float:left;width:{FLOAT_SIZE}pt;height:{FLOAT_SIZE}pt;'
                f'shape-outside:{shape};background:#ccc}}',
                f'<div class=f></div><p>{WORDS}</p>')
            glyphs = [glyph for glyph in pdf_content.inspect(output)['glyphs']
                      if glyph.page == 1 and not glyph.text.isspace()]
        starts = {}
        for glyph in glyphs:
            x, baseline = glyph.origin
            key = round(baseline, 3)
            starts[key] = min(starts.get(key, x), x)
        # Line boxes from the page area's top, in CSS coordinates below it.
        return [(PAGE_HEIGHT - baseline - PAGE_MARGIN - BASELINE_OFFSET, x - PAGE_MARGIN)
                for baseline, x in sorted(starts.items(), reverse=True)]

    def assert_wraps(self, shape, edge):
        lines = self.line_starts(shape)
        self.assertGreater(len(lines), LINES_BESIDE, f'{shape}: text continues below the float')
        for index, (top, start) in enumerate(lines[:LINES_BESIDE]):
            with self.subTest(shape=shape, line=index):
                self.assertAlmostEqual(top, index * LINE_HEIGHT, delta=0.01,
                                       msg=f'line {index} keeps its place in the flow')
                self.assertAlmostEqual(start, edge(top), delta=TOLERANCE,
                                       msg=f'line {index} over {top}pt starts at the float '
                                           f'area edge {edge(top):.3f}pt, not {start:.3f}pt')
        top, start = lines[LINES_BESIDE]
        self.assertAlmostEqual(start, 0, delta=0.001, msg=f'below the float at {top}pt')

    def test_text_wraps_around_a_circle(self):
        self.assert_wraps('circle(50%)', circle_edge)

    def test_text_wraps_around_a_polygon(self):
        self.assert_wraps('polygon(0 0,100% 50%,0 100%)', arrow_edge)


if __name__ == '__main__':
    unittest.main()
