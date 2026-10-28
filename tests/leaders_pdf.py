"""Check leader glyph origins against font metrics and the specified end-edge phase."""
from pathlib import Path
import tempfile
import unittest

from fontTools.ttLib import TTFont
import table_positioning_pdf
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FONT_SIZE = 10
PAGE_WIDTH = 595
PAGE_MARGIN = 42
LINE_WIDTH = PAGE_WIDTH - 2 * PAGE_MARGIN
POSITION_TOLERANCE = 0.001


class LeadersPdf(unittest.TestCase):
    def test_periods_use_exact_font_advance_and_end_edge_phase(self):
        with TTFont(ROOT / 'tests/oracle/assets/DejaVuSans.ttf') as font:
            name = font.getBestCmap()[ord('.')]
            advance = font['hmtx'].metrics[name][0] * FONT_SIZE / font['head'].unitsPerEm
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-leaders-') as directory:
            renderer = table_positioning_pdf.TablePositioningPdf()
            output = renderer.render(Path(directory),
                f'@page{{size:{PAGE_WIDTH}pt 842pt;margin:{PAGE_MARGIN}pt}}'
                f'p{{width:{LINE_WIDTH}pt;margin:0;font:{FONT_SIZE}pt/14pt Probe}}'
                'p::after{content:leader(".") "9"}',
                '<p>Short</p><p>A longer label</p>')
            lines = {}
            for glyph in pdf_content.inspect(output)['glyphs']:
                if glyph.text == '.':
                    lines.setdefault(round(glyph.matrix[5], 3), []).append(glyph.matrix[4])
            self.assertEqual(len(lines), 2)
            for baseline, origins in lines.items():
                self.assertGreater(len(origins), 100, 'exercise accumulated advance error')
                for origin in origins:
                    slots = (PAGE_MARGIN + LINE_WIDTH - origin) / advance
                    self.assertLess(abs(slots - round(slots)) * advance, POSITION_TOLERANCE,
                                    f'leader at {origin} on baseline {baseline} lost end-edge phase')
                for left, right in zip(origins, origins[1:]):
                    self.assertAlmostEqual(right - left, advance, delta=POSITION_TOLERANCE,
                                           msg='leader advance must retain the font design-unit precision')


if __name__ == '__main__':
    unittest.main(verbosity=2)
