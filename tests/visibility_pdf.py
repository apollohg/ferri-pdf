"""Native ink checks where clipped display-list primitives are insufficient."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf


class VisibilityPdf(unittest.TestCase):
    def test_collapsed_colspan_retains_each_border_edge(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for direction in ('ltr', 'rtl'):
                for collapsed_column in (0, 1):
                    with self.subTest(direction=direction, collapsed_column=collapsed_column):
                        css = ('table{direction:' + direction + ';border-collapse:collapse;'
                               'table-layout:fixed;width:120pt}col{width:60pt}'
                               'td{height:40pt;padding:0;border:2pt solid blue;background:red}'
                               '.gone{visibility:collapse}')
                        control = renderer.render(root, css + 'table{width:60pt}',
                                                  '<table><col><tr><td></td></tr></table>')
                        with renderer.raster(root, control, vector_antialias=False) as raster:
                            expected = raster.tobytes()
                        columns = ''.join('<col class=gone>' if index == collapsed_column else '<col>'
                                          for index in range(2))
                        output = renderer.render(root, css,
                            '<table>' + columns + '<tr><td colspan=2></td></tr></table>')
                        with renderer.raster(root, output, vector_antialias=False) as raster:
                            self.assertEqual(raster.tobytes(), expected,
                                             'collapsed column lost or displaced a border edge')

    def test_reversed_flex_cross_axis_keeps_shared_glyph_baselines(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for visibility in ('visible', 'collapse'):
                with self.subTest(visibility=visibility):
                    output = renderer.render(root,
                        'section{display:flex;flex-wrap:wrap-reverse;align-items:baseline;width:200pt}'
                        'section>div{width:40pt;flex-shrink:0}.a{font-size:10pt;line-height:14pt}'
                        '.b{font-size:20pt;line-height:24pt}.gone{height:70pt;font-size:40pt;'
                        'line-height:50pt;visibility:' + visibility + '}',
                        '<section><div class=a>A</div><div class=gone>H</div><div class=b>B</div></section>')
                    glyphs = table_positioning_pdf.pdf_content.inspect(output)['glyphs']
                    origins = {glyph.text: glyph.origin for glyph in glyphs}
                    self.assertAlmostEqual(origins['A'][1], origins['B'][1], places=3)
                    if visibility == 'visible':
                        self.assertAlmostEqual(origins['H'][1], origins['A'][1], places=3)
                    else:
                        self.assertNotIn('H', origins)

    def test_positioned_child_keeps_original_percentage_dimensions_after_track_collapse(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            css = ('table{width:120pt;table-layout:fixed;border-spacing:0}'
                   'col{width:60pt}tr{height:40pt}td{padding:0;position:relative;height:80pt}'
                   '.span{visibility:visible}td div{position:absolute;top:25%;left:0;'
                   'width:50%;height:10pt;background:blue}')
            body = ('<table><col><col class=gone><tr class=gone>'
                    '<td colspan=2 rowspan=2 class=span><div></div></td></tr><tr></tr></table>')
            control = renderer.render(root, css, body)
            with renderer.raster(root, control, vector_antialias=False) as raster:
                expected = raster.tobytes()
                self.assertEqual(raster.getpixel((75, 45)), (0, 0, 255))
            actual = renderer.render(root, css + '.gone{visibility:collapse}', body)
            with renderer.raster(root, actual, vector_antialias=False) as raster:
                self.assertEqual(raster.tobytes(), expected,
                                 'track collapse changed positioned percentage geometry')

    def test_hidden_table_border_tiles_do_not_leak_into_visible_cell(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            css = '''table{visibility:hidden;border-collapse:collapse;border:5pt solid red}
tbody,tr{background:red}td{border:5pt solid red}
.visible{visibility:visible;background:green;border:6pt solid green}'''
            output = renderer.render(root, css,
                '<table><tbody><tr><td>Hidden</td><td class=visible>Visible</td></tr></tbody></table>')
            with renderer.raster(root, output, vector_antialias=False) as raster:
                pixels = list(raster.getdata())
                self.assertNotIn((255, 0, 0), pixels, 'hidden border or background leaked')
                self.assertIn((0, 128, 0), pixels, 'explicitly visible cell must paint')
            control = renderer.render(root, css + 'table{visibility:visible}',
                '<table><tbody><tr><td>Hidden</td><td class=visible>Visible</td></tr></tbody></table>')
            with renderer.raster(root, control, vector_antialias=False) as raster:
                self.assertIn((255, 0, 0), list(raster.getdata()),
                              'positive control must expose the hidden cell border and background')

    def test_collapsed_flex_subtree_has_no_glyphs_links_or_ink(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = renderer.render(root,
                'section{display:flex;width:90pt;flex-wrap:wrap;gap:10pt;align-items:start}'
                'section>div{width:40pt;flex-shrink:0}.collapsed{visibility:collapse;height:70pt;background:red}'
                '.visible{visibility:visible}p{margin:0}',
                '<section><div>A</div><div class=collapsed><a class=visible href="https://example.org/hidden">Hidden</a></div>'
                '<div>B</div></section><p>After</p>')
            glyphs = table_positioning_pdf.pdf_content.inspect(output)['glyphs']
            text = ''.join(glyph.text for glyph in glyphs)
            self.assertNotIn('Hidden', text)
            self.assertIn('After', text)
            origins = {label: next(glyph.origin for glyph in glyphs if glyph.text == label)
                       for label in ('A', 'B')}
            self.assertAlmostEqual(origins['A'][1], origins['B'][1], places=3)
            self.assertAlmostEqual(origins['B'][0] - origins['A'][0], 50, places=3)
            self.assertEqual(table_positioning_pdf.pdf_content.navigation(output)['links'], [])
            with renderer.raster(root, output, vector_antialias=False) as raster:
                self.assertNotIn((255, 0, 0), list(raster.getdata()))

    def test_wrapped_column_collapsed_string_is_emitted_on_its_fragment_page(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            body = ('<section><div>' + '<p>Line</p>' * 9
                    + '</div><div class=collapsed>Hidden</div><div>After</div>'
                      '<div class=second>Other column</div></section>')
            output = renderer.render(root,
                "@page{size:240pt 100pt;margin:10pt;@top-left{content:'Header:' string(label,last)}}"
                'p{margin:0}section{display:flex;flex-direction:column;flex-wrap:wrap;height:140pt;width:180pt}'
                'section>div{flex-shrink:0;width:80pt}.collapsed{visibility:collapse;string-set:label "Middle"}'
                '.second{height:140pt}', body)
            pages = {}
            for glyph in table_positioning_pdf.pdf_content.inspect(output)['glyphs']:
                pages.setdefault(glyph.page, []).append(glyph.text)
            text = [''.join(pages[page]) for page in sorted(pages)]
            self.assertEqual(len(text), 2)
            self.assertNotIn('Middle', text[0])
            self.assertIn('Header:Middle', text[1])
            self.assertNotIn('Hidden', ''.join(text))

    def test_collapsed_colspan_paint_stays_within_the_projected_width(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            body = '<table><col><col class=gone><tr><td colspan=2>Alpha beta gamma delta</td></tr></table>'
            for direction in ('ltr', 'rtl'):
                css = ('table{direction:' + direction + ';table-layout:fixed;width:120pt;border-spacing:0}'
                       'col{width:60pt}td{padding:0;background:red;height:50pt;vertical-align:top}')
                control = renderer.render(root, css, body)
                with renderer.raster(root, control, vector_antialias=False) as raster:
                    self.assertEqual(raster.getpixel((100, 65)), (255, 0, 0))
                output = renderer.render(root, css + '.gone{visibility:collapse}', body)
                with renderer.raster(root, output, vector_antialias=False) as raster:
                    self.assertEqual(raster.getpixel((50, 65)), (255, 0, 0))
                    outside = raster.crop((80, 0, raster.width, raster.height))
                    with outside:
                        self.assertEqual(set(outside.getdata()), {(255, 255, 255)},
                                         direction + ': cell ink escaped its reduced border box')

    def test_collapsed_rowspan_border_matches_its_remaining_row(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            css = ('table{border-collapse:collapse}tr{height:40pt}'
                   'td{padding:0;width:60pt;border:2pt solid blue;vertical-align:top;background:red}'
                   '.gone{visibility:collapse}.span{visibility:visible}')
            reference = renderer.render(root, css, '<table><tr><td>Top</td></tr></table>')
            with renderer.raster(root, reference, vector_antialias=False) as raster:
                expected = raster.tobytes()
            for row_count in (2, 3):
                for visible_row in range(row_count):
                    with self.subTest(row_count=row_count, visible_row=visible_row):
                        rows = []
                        for row in range(row_count):
                            attributes = '' if row == visible_row else ' class=gone'
                            content = (f'<td rowspan={row_count} class=span>Top</td>'
                                       if row == 0 else '')
                            rows.append(f'<tr{attributes}>{content}</tr>')
                        actual = renderer.render(root, css, '<table>' + ''.join(rows) + '</table>')
                        with renderer.raster(root, actual, vector_antialias=False) as raster:
                            self.assertEqual(raster.tobytes(), expected,
                                             'collapsed rowspan must retain every border edge')


if __name__ == '__main__':
    unittest.main()
