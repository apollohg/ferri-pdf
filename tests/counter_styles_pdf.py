"""Rendered checks for CSS Counter Styles 3 markers and CSS Display 3 run-in and inline list items."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
BASE = 'p,ol,ul,li,dl,dt,dd{margin:0;padding:0}ol,ul{padding-left:40pt}'


def lines(output):
    """Glyph text per baseline, top to bottom, each in reading order."""
    rows = {}
    for glyph in pdf_content.inspect(output)['glyphs']:
        rows.setdefault(round(glyph.origin[1], 2), []).append(glyph)
    return [''.join(glyph.text for glyph in sorted(row, key=lambda glyph: glyph.origin[0]))
            for _, row in sorted(rows.items(), reverse=True)]


class CounterStylesPdf(unittest.TestCase):
    def setUp(self):
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def assert_same_ink(self, root, css, body, reference_css, reference_body, message):
        reference = self.renderer.render(root, BASE + reference_css, reference_body)
        with self.renderer.raster(root, reference, vector_antialias=False) as raster:
            expected = raster.tobytes()
        output = self.renderer.render(root, BASE + css, body)
        with self.renderer.raster(root, output, vector_antialias=False) as raster:
            self.assertEqual(raster.tobytes(), expected,
                             f'{message}; rendered lines {lines(output)} against {lines(reference)}')

    def test_custom_counter_style_markers_match_their_literal_marker_strings(self):
        rules = ('@counter-style thumbs{system:cyclic;symbols:"*" "+";suffix:" "}'
                 '@counter-style paren{system:extends decimal;prefix:"(";suffix:") "}'
                 '@counter-style corner{system:fixed;symbols:A B;suffix:": ";fallback:lower-roman}')
        items = '<li>One</li><li>Two</li><li>Three</li>'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-counter-styles-') as directory:
            root = Path(directory)
            for position in ('inside', 'outside'):
                for style, markers in [('thumbs', ['* ', '+ ', '* ']),
                                       ('paren', ['(1) ', '(2) ', '(3) ']),
                                       ('corner', ['A: ', 'B: ', 'iii: ']),
                                       ('symbols(symbolic "#")', ['# ', '## ', '### ']),
                                       ('trad-chinese-informal', ['一、', '二、', '三、'])]:
                    with self.subTest(position=position, style=style):
                        strings = ''.join(f'li:nth-child({index + 1}){{list-style-type:"{marker}"}}'
                                          for index, marker in enumerate(markers))
                        self.assert_same_ink(
                            root, rules + f'li{{list-style:{position} {style}}}', f'<ol>{items}</ol>',
                            f'li{{list-style-position:{position}}}' + strings, f'<ol>{items}</ol>',
                            'a counter style marker is its prefix, representation and suffix')

    def test_counter_functions_print_representations_without_prefix_or_suffix(self):
        rules = ('@counter-style paren{system:extends decimal;prefix:"(";suffix:") "}'
                 'h1{counter-increment:part 2;font-size:inherit;margin:0}')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-counter-functions-') as directory:
            root = Path(directory)
            self.assert_same_ink(
                root, rules + 'h1::before{content:counter(part,paren) "|" counter(part,upper-roman) '
                              '"|" counters(part,".",symbols(cyclic "x" "y")) "|"}',
                '<h1>A</h1><h1>B</h1>',
                'h1{font-size:inherit;margin:0}', '<h1>2|II|y|A</h1><h1>4|IV|y|B</h1>',
                'counter() and counters() print the bare representation')

    def test_run_in_joins_the_first_line_of_the_following_block(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-run-in-') as directory:
            root = Path(directory)
            css = 'dt{display:run-in}dt::after{content:": "}'
            self.assert_same_ink(
                root, css,
                '<dl><dt>Term</dt>\n<dd>Definition text.</dd><dt>Next</dt> <dd><p>Nested.</p></dd></dl>',
                '', '<p>Term: Definition text.</p><p>Next: Nested.</p>',
                'a run-in sequence becomes the first inline content of the following block, '
                'recursing into its first block child')
            self.assert_same_ink(
                root, 'h3{display:run-in;font-size:inherit;margin:0}td{padding:0}table{border-spacing:0}',
                '<div>Before <h3>Head</h3> after text</div><div><h3>Alone</h3><table><tr><td>Cell</td></tr></table></div>',
                'td{padding:0}table{border-spacing:0}',
                '<div>Before</div><div><b>Head</b> after text</div><div><b>Alone</b></div>'
                '<table><tr><td>Cell</td></tr></table>',
                'a run-in not followed by a block box starts an anonymous block with the inline content after it')
            self.assert_same_ink(
                root, 'dt{display:run-in;counter-increment:term}dd::before{content:counter(term) " "}',
                '<dl><dt>A</dt><dd>first</dd><dt>B</dt><dd>second</dd></dl>',
                '', '<p>A1 first</p><p>B2 second</p>',
                'a run-in precedes the block\'s ::before and keeps its counter order')
            self.assert_same_ink(
                root, 'span{display:run-in}li{list-style-position:inside}',
                '<ul><span>Run </span> <li>item</li></ul>',
                'li{list-style-position:inside}', '<ul><li>Run item</li></ul>',
                'a run-in follows the list item\'s inside marker, keeping its own trailing space only')

    def test_inline_list_items_carry_inside_markers(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-inline-list-item-') as directory:
            root = Path(directory)
            self.assert_same_ink(
                root, 'span{display:inline list-item;list-style:outside upper-alpha}',
                '<p><span>one</span> <span>two</span></p>',
                '', '<p>A. one B. two</p>',
                'an inline list item numbers its list-item counter and places the marker inside')
