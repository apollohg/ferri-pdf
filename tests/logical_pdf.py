"""Compare flow-relative box properties with the physical properties they map to."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf

PAGE_MARGIN = table_positioning_pdf.PAGE_MARGIN
RED = (255, 0, 0)
START_BORDER = 8
END_MARGIN = 40
START_MARGIN = 10
BORDER_BOX_WIDTH = 136
TOP_MARGIN = 5
TOP_BORDER = 3
BODY = '<div class=box></div><div class=limits></div><div class=positioned></div>'
COMMON = ('body{position:relative}.box{background:yellow}.limits{background:teal}'
          '.positioned{position:absolute;background:purple}')
LOGICAL = ('.box{margin-inline:10pt 40pt;margin-block:5pt 6pt;padding-inline:6pt 20pt;'
           'padding-block:4pt 9pt;border-inline-start:8pt solid red;'
           'border-inline-end:2pt solid blue;border-block:3pt solid green;'
           'border-start-end-radius:12pt;border-end-start-radius:16pt 10pt;'
           'inline-size:100pt;block-size:50pt}'
           '.limits{inline-size:200pt;max-inline-size:60pt;min-block-size:15pt;'
           'margin-inline-start:25pt}'
           '.positioned{inset-inline-start:30pt;inset-block-start:150pt;'
           'inline-size:20pt;block-size:24pt}')


def physical(start, end, start_end, end_start):
    """The physical declarations LOGICAL maps to with the inline-start side `start`."""
    return (f'.box{{margin-{start}:10pt;margin-{end}:40pt;margin-top:5pt;margin-bottom:6pt;'
            f'padding-{start}:6pt;padding-{end}:20pt;padding-top:4pt;padding-bottom:9pt;'
            f'border-{start}:8pt solid red;border-{end}:2pt solid blue;'
            'border-top:3pt solid green;border-bottom:3pt solid green;'
            f'border-{start_end}-radius:12pt;border-{end_start}-radius:16pt 10pt;'
            'width:100pt;height:50pt}'
            f'.limits{{width:200pt;max-width:60pt;min-height:15pt;margin-{start}:25pt}}'
            f'.positioned{{{start}:30pt;top:150pt;width:20pt;height:24pt}}')


DIRECTIONS = {
    'ltr': physical('left', 'right', 'top-right', 'bottom-left'),
    'rtl': physical('right', 'left', 'top-left', 'bottom-right'),
}


class LogicalPdf(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix='ferri-pdf-logical-')
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def pixels(self, css):
        output = self.renderer.render(self.root, COMMON + css, BODY)
        with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
            return raster.copy()

    def test_flow_relative_boxes_paint_as_their_physical_counterparts(self):
        references = {}
        for direction, expected in DIRECTIONS.items():
            with self.subTest(direction=direction):
                flow = f'.box,.limits,.positioned{{direction:{direction}}}'
                reference = self.pixels(flow + expected)
                references[direction] = reference.tobytes()
                actual = self.pixels(flow + LOGICAL)
                self.assertEqual(actual.tobytes(), references[direction],
                                 f'{direction}: flow-relative margins, padding, borders, radii, '
                                 'sizes or insets painted differently from their physical '
                                 'counterparts')
                # The containing block is LTR, so CSS 2 §10.3.3 drops the right margin.
                left_margin = START_MARGIN if direction == 'ltr' else END_MARGIN
                border_x = PAGE_MARGIN + left_margin + (
                    START_BORDER // 2 if direction == 'ltr'
                    else BORDER_BOX_WIDTH - START_BORDER // 2)
                border_y = PAGE_MARGIN + TOP_MARGIN + TOP_BORDER + 10
                self.assertEqual(actual.getpixel((border_x, border_y)), RED,
                                 f'{direction}: the inline-start border must be red at '
                                 f'x={border_x}')
        self.assertNotEqual(references['ltr'], references['rtl'],
                            'the probe must distinguish the two inline directions')

    def test_later_declaration_wins_between_paired_properties(self):
        for direction, start in (('ltr', 'left'), ('rtl', 'right')):
            with self.subTest(direction=direction):
                base = f'.box{{direction:{direction};width:50pt;height:20pt}}'
                physical_last = self.pixels(
                    base + f'.box{{padding-inline-start:70pt;padding-{start}:5pt}}')
                logical_last = self.pixels(
                    base + f'.box{{padding-{start}:5pt;padding-inline-start:70pt}}')
                narrow = self.pixels(base + f'.box{{padding-{start}:5pt}}').tobytes()
                wide = self.pixels(base + f'.box{{padding-{start}:70pt}}').tobytes()
                self.assertNotEqual(narrow, wide, 'the probe padding must be visible')
                self.assertEqual(physical_last.tobytes(), narrow,
                                 f'{direction}: a later padding-{start} overrides inline-start')
                self.assertEqual(logical_last.tobytes(), wide,
                                 f'{direction}: a later inline-start overrides padding-{start}')

if __name__ == '__main__':
    unittest.main()
