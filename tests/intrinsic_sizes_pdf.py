"""Compare intrinsic sizing keywords with independently measured explicit widths."""
from pathlib import Path
import tempfile
import unittest

from fontTools.ttLib import TTFont
import table_positioning_pdf
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
MONO = ROOT / 'assets/fonts/DejaVuSansMono.ttf'
FONT_SIZE = 10
LONGEST_WORD = 'AAAA'
TEXT = 'AAAA BB'
PAGES = 2
# One word per min-content line: 30 lines of 14pt fill one 260pt page area and part of another.
REPEATS = 15


class IntrinsicSizesPdf(unittest.TestCase):
    def test_keywords_match_explicit_widths_across_contexts_and_pages(self):
        with TTFont(MONO) as font:
            cmap = font.getBestCmap()
            units = font['head'].unitsPerEm
            advance = lambda text: sum(
                font['hmtx'].metrics[cmap[ord(ch)]][0] for ch in text) * FONT_SIZE / units
            minimum, maximum = advance(LONGEST_WORD), advance(TEXT)
        link = f'<a href="mailto:reader@example.org">{TEXT}</a>'
        base = (f"@font-face{{font-family:Mono;src:url('{MONO.as_uri()}')}}"
                '.probe{font-family:Mono;background:rgb(1,2,3)}p,div{margin:0}'
                'a{color:black;text-decoration:none}')
        long = ' '.join([TEXT] * REPEATS)
        cases = [
            ('block min-content', '', 'width:min-content', f'width:{minimum}pt',
             f'<div class=probe>{link}</div>'),
            ('inline-block max-content', '', 'display:inline-block;width:max-content',
             f'display:inline-block;width:{maximum}pt', f'<div class=probe>{link}</div>'),
            # 40pt less both insets leaves 25pt, between the min- and max-content widths.
            ('absolute fit-content', 'section{position:relative;width:40pt;height:20pt}',
             'position:absolute;left:10pt;right:5pt;width:fit-content',
             'position:absolute;left:10pt;right:5pt;width:25pt',
             f'<section><div class=probe>{link}</div></section>'),
            ('flex basis min-content', 'section{display:flex;width:200pt}',
             'flex:0 0 min-content', f'flex:0 0 {minimum}pt',
             f'<section><div class=probe>{link}</div></section>'),
            ('grid max-content', 'section{display:grid;grid-template-columns:200pt}',
             'width:max-content', f'width:{maximum}pt',
             f'<section><div class=probe>{link}</div></section>'),
            ('caption min-content', 'table{border-spacing:0}td{padding:0}caption{font-family:Mono}',
             'width:min-content', f'width:{minimum}pt',
             f'<table><caption class=probe>{link}</caption><tr><td>{long[:60]}</td></tr></table>'),
            ('paged min-content', '', 'width:min-content', f'width:{minimum}pt',
             f'<div class=probe><a href="mailto:reader@example.org">{long}</a></div>'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-intrinsic-sizes-') as directory:
            root = Path(directory)
            renderer = table_positioning_pdf.TablePositioningPdf()
            for name, context, keyword, explicit, body in cases:
                with self.subTest(name):
                    reference = renderer.render(root, f'{base}{context}.probe{{{explicit}}}', body)
                    expected = self.evidence(renderer, root, reference)
                    actual = renderer.render(root, f'{base}{context}.probe{{{keyword}}}', body)
                    self.assertEqual(self.evidence(renderer, root, actual), expected,
                                     f'{keyword} must match {explicit}')
                    if name == 'paged min-content':
                        pages = {glyph.page for glyph in pdf_content.inspect(actual)['glyphs']}
                        self.assertEqual(len(pages), PAGES, 'content must continue on a second page')

    def evidence(self, renderer, root, output):
        glyphs = pdf_content.inspect(output)['glyphs']
        rasters = []
        for page in sorted({glyph.page for glyph in glyphs}):
            with renderer.raster(root, output, page=page, vector_antialias=False) as raster:
                rasters.append(raster.tobytes())
        return glyphs, pdf_content.navigation(output)['links'], rasters


if __name__ == '__main__':
    unittest.main()
