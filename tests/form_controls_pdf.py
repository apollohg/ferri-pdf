"""Rendered checks for form control appearances and fieldsets (HTML §15.3.12, §15.5)."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf

WHITE = (255, 255, 255)
GREEN = (0, 128, 0)
RED = (255, 0, 0)
BLUE = (0, 0, 255)
# A luminance darker than any background in the probes: drawn ink.
DARK = 200
# The page area starts 20pt in; rasters are 72 dpi, so one pixel is one point.
ORIGIN = table_positioning_pdf.PAGE_MARGIN


class FormControlsPdf(unittest.TestCase):
    def setUp(self):
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def pixel(self, css, body, point):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-forms-') as directory:
            root = Path(directory)
            output = self.renderer.render(root, css, body)
            with self.renderer.raster(root, output, vector_antialias=False) as raster:
                return raster.getpixel(point)

    def test_checked_checkboxes_fill_with_the_accent_colour_only_natively(self):
        css = ('input{display:block;margin:0;width:40pt;height:40pt;accent-color:rgb(0,128,0)}')
        corner = (ORIGIN + 8, ORIGIN + 8)
        self.assertEqual(self.pixel(css, '<input type=checkbox checked>', corner), GREEN,
                         'the native checked box is filled with AccentColor')
        self.assertEqual(self.pixel(css, '<input type=checkbox>', corner), WHITE,
                         'an unchecked box shows the Field colour')
        self.assertEqual(
            self.pixel(css + 'input{appearance:none}', '<input type=checkbox checked>', corner),
            WHITE, 'primitive appearance suppresses the check state drawing')

    def test_radio_dots_and_gauge_fills_follow_their_values(self):
        css = 'input,progress,meter{display:block;margin:0;accent-color:rgb(0,0,255)}'
        self.assertEqual(
            self.pixel(css + 'input{width:40pt;height:40pt}', '<input type=radio checked>',
                       (ORIGIN + 20, ORIGIN + 20)),
            BLUE, 'a checked radio button has an accent dot at its centre')
        progress = 'progress{width:100pt;height:20pt}'
        self.assertEqual(self.pixel(css + progress, '<progress value=0.8></progress>',
                                    (ORIGIN + 70, ORIGIN + 10)),
                         BLUE, 'the bar fills to 80%')
        self.assertNotEqual(self.pixel(css + progress, '<progress value=0.5></progress>',
                                       (ORIGIN + 70, ORIGIN + 10)),
                            BLUE, 'and stops at 50%')
        meter = 'meter{width:100pt;height:20pt}'
        self.assertEqual(self.pixel(css + meter, '<meter value=0.6></meter>',
                                    (ORIGIN + 30, ORIGIN + 10)),
                         BLUE, 'a meter in its optimum region uses the accent colour')
        self.assertNotIn(
            self.pixel(css + meter, '<meter value=0.9 low=0.3 high=0.7 optimum=0.1></meter>',
                       (ORIGIN + 30, ORIGIN + 10)),
            (BLUE, WHITE), 'a value two regions from the optimum is drawn in another colour')

    def test_controls_with_their_own_vertical_writing_mode_run_top_to_bottom(self):
        # HTML §15.5.14–15: a 1em by 10em bar, its start at the top unless direction is rtl.
        css = ('progress,meter{display:block;margin:0;font-size:10pt;accent-color:rgb(0,0,255);'
               'writing-mode:vertical-lr}')
        cases = (('<progress value=0.2></progress>', 'progress', 100),
                 ('<meter value=0.2></meter>', 'meter', 50))
        for body, name, length in cases:
            near_start, near_end = (ORIGIN + 5, ORIGIN + 3), (ORIGIN + 5, ORIGIN + length - 3)
            self.assertEqual(self.pixel(css, body, near_start), BLUE,
                             f'a vertical {name} fills from its top')
            self.assertEqual(self.pixel(css, body, (ORIGIN + 5, ORIGIN + length + 3)), WHITE,
                             f'a vertical {name} is {length}pt tall and no taller')
            self.assertEqual(self.pixel(css, body, (ORIGIN + 13, ORIGIN + 3)), WHITE,
                             f'a vertical {name} is 10pt wide')
            self.assertNotEqual(self.pixel(css, body, near_end), BLUE,
                                f'a vertical {name} at 20% leaves its bottom unfilled')
            rtl = css + f'{name}{{direction:rtl}}'
            self.assertEqual(self.pixel(rtl, body, near_end), BLUE,
                             f'a right-to-left vertical {name} fills from its bottom')
            self.assertNotEqual(self.pixel(rtl, body, near_start), BLUE,
                                f'a right-to-left vertical {name} leaves its top unfilled')

    def test_fieldset_border_leaves_a_gap_behind_the_rendered_legend(self):
        css = ('fieldset{margin:0;padding:0 20pt;border:4pt solid rgb(255,0,0)}'
               'legend{padding:0;margin:0;font-size:20pt;line-height:20pt;color:white}')
        body = '<fieldset><legend>Legend</legend><div style="height:40pt"></div></fieldset>'
        # The legend's 20pt box is centred over the border, which paints at 8–12pt.
        border_y = ORIGIN + 10
        self.assertEqual(self.pixel(css, body, (ORIGIN + 10, border_y)), RED,
                         'the block-start border paints before the legend')
        self.assertEqual(self.pixel(css, body, (ORIGIN + 30, border_y)), WHITE,
                         'and not behind the legend')
        self.assertEqual(self.pixel(css, body, (ORIGIN + 10, ORIGIN + 4)), WHITE,
                         'the fieldset paints from the legend centre, not its top')

    def test_drop_down_button_belongs_to_native_and_devolved_appearance(self):
        css = ('select{display:block;width:100pt;font-size:20pt;border:0;padding:0;'
               'background:white}')
        body = '<select><option>Item</option></select>'

        def dark(appearance):
            with tempfile.TemporaryDirectory(prefix='ferri-pdf-select-') as directory:
                root = Path(directory)
                output = self.renderer.render(root, css + f'select{{appearance:{appearance}}}',
                                              body)
                with self.renderer.raster(root, output, vector_antialias=False) as raster:
                    region = raster.crop((ORIGIN + 80, ORIGIN, ORIGIN + 100, ORIGIN + 24))
                    return region.convert('L').getextrema()[0] < DARK

        self.assertTrue(dark('auto'), 'an author background devolves the box but keeps the button')
        self.assertFalse(dark('none'), 'primitive appearance has no drop-down button')


if __name__ == '__main__':
    unittest.main()
