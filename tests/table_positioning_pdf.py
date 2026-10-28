"""Independent glyph, link and ink checks for positioned table wrappers."""
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / 'target/debug/ferri-pdf'
PAGE_HEIGHT = 300
PAGE_MARGIN = 20
OFFSET_X = 13
OFFSET_Y = 9
TABLE_WIDTH = 160
LINE_HEIGHT = 14
FONT_SIZE = 10
BASELINE_OFFSET = LINE_HEIGHT / 2 + (1901 - 483) * FONT_SIZE / (2 * 2048)


class TablePositioningPdf(unittest.TestCase):
    def test_group_shadow_fragment_edges_obey_slice_and_clone(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for decoration in ('slice', 'clone'):
                with self.subTest(decoration=decoration):
                    output = self.render(root,
                        '@page{size:200pt 100pt;margin:10pt}'
                        'table{width:100pt;border-spacing:0}td{height:40pt;padding:0}'
                        'tbody{box-shadow:0 0 0 3pt red;box-decoration-break:' + decoration + '}',
                        '<table><tbody>' + '<tr><td></td></tr>' * 4 + '</tbody></table>')
                    for page in (1, 2):
                        with self.raster(root, output, vector_antialias=False, page=page) as raster:
                            self.assertEqual(raster.getpixel((8, 50)), (255, 0, 0),
                                             'side shadow must continue on each fragment')
                            cut = (50, 92) if page == 1 else (50, 8)
                            expected = (255, 0, 0) if decoration == 'clone' else (255, 255, 255)
                            self.assertEqual(raster.getpixel(cut), expected,
                                             'slice must not create a new shadow edge at a page break')

    def test_structural_shadows_match_single_box_shapes_without_cell_seams(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for selector, height, count in [('tr', 25, 2), ('tbody', 50, 1)]:
                for model, shadow in [('separate', '4pt 3pt 0 2pt red'),
                                      ('separate', 'inset 0 0 0 3pt red'),
                                      ('collapse', 'inset 0 0 0 3pt red')]:
                    with self.subTest(selector=selector, model=model, shadow=shadow):
                        reference = self.render(root,
                            f'.shape{{width:100pt;height:{height}pt;box-shadow:{shadow}}}',
                            '<div class=shape></div>' * count)
                        with self.raster(root, reference, vector_antialias=False) as raster:
                            expected = raster.tobytes()
                            self.assertIn((255, 0, 0), list(raster.getdata()))
                        output = self.render(root,
                            f'table{{width:100pt;table-layout:fixed;border-spacing:0;border-collapse:{model}}}'
                            f'td{{height:25pt;padding:0}}{selector}{{box-shadow:{shadow}}}',
                            '<table><tbody><tr><td></td><td></td></tr><tr><td></td><td></td></tr></tbody></table>')
                        with self.raster(root, output, vector_antialias=False) as raster:
                            self.assertEqual(raster.tobytes(), expected,
                                             'structural shadow shape or layering differs from its owning box')

    def test_structural_inset_shadows_follow_background_and_cell_layer_order(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            body = '<table><tbody><tr><td></td><td></td></tr></tbody></table>'
            for selector in ('tr', 'tbody'):
                for model in ('separate', 'collapse'):
                    with self.subTest(selector=selector, model=model):
                        css = (f'table{{width:100pt;table-layout:fixed;border-spacing:0;border-collapse:{model}}}'
                               f'td{{height:25pt;padding:0}}{selector}{{background:yellow;box-shadow:inset 0 0 0 3pt red}}')
                        transparent = self.render(root, css, body)
                        with self.raster(root, transparent, vector_antialias=False) as raster:
                            self.assertEqual(raster.getpixel((21, 21)), (255, 0, 0))
                            self.assertEqual(raster.getpixel((70, 32)), (255, 255, 0))
                        opaque = self.render(root, css + 'td{background:green}', body)
                        with self.raster(root, opaque, vector_antialias=False) as raster:
                            self.assertEqual(raster.getpixel((21, 21)), (0, 128, 0),
                                             'cell background must cover its ancestor inset shadow')
                            self.assertEqual(raster.getpixel((70, 32)), (0, 128, 0))

    def test_structural_shadows_keep_opacity_transform_and_nested_table_ownership(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for selector, height, count in [('tr', 25, 2), ('tbody', 50, 1)]:
                for nested in (False, True):
                    with self.subTest(selector=selector, nested=nested):
                        effects = 'box-shadow:4pt 3pt 2pt 2pt red;opacity:.5;transform:translate(7pt,9pt)'
                        reference = self.render(root,
                            f'.shape{{width:100pt;height:{height}pt;{effects}}}',
                            '<div class=shape></div>' * count)
                        with self.raster(root, reference, vector_antialias=False) as raster:
                            expected = raster.tobytes()
                        body = '<table class=inner><tbody><tr><td></td><td></td></tr><tr><td></td><td></td></tr></tbody></table>'
                        if nested:
                            body = '<table class=outer><tr><td>' + body + '</td></tr></table>'
                        output = self.render(root,
                            'table{width:100pt;border-spacing:0}td{padding:0}'
                            '.outer{border-collapse:collapse}.inner{border-collapse:separate}'
                            f'.inner td{{height:25pt}}.inner>{"tbody>tr" if selector == "tr" else "tbody"}{{{effects}}}', body)
                        with self.raster(root, output, vector_antialias=False) as raster:
                            self.assertEqual(raster.tobytes(), expected,
                                             'structural effect ownership changed the shadow pixels')

    def test_hidden_and_collapsed_structural_shadows_emit_no_ink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            body = '<table><tbody><tr><td></td><td></td></tr><tr><td></td><td></td></tr></tbody></table>'
            for selector in ('tr', 'tbody'):
                for visibility in ('hidden', 'collapse'):
                    with self.subTest(selector=selector, visibility=visibility):
                        css = ('table{width:100pt;border-spacing:0}td{height:25pt;padding:0}'
                               f'{selector}{{visibility:{visibility}}}')
                        control = self.render(root, css, body)
                        with self.raster(root, control, vector_antialias=False) as raster:
                            expected = raster.tobytes()
                        output = self.render(root, css + f'{selector}{{box-shadow:4pt 3pt 2pt 2pt red,inset 0 0 0 3pt blue}}', body)
                        with self.raster(root, output, vector_antialias=False) as raster:
                            self.assertEqual(raster.tobytes(), expected,
                                             'non-visible structural box regained shadow ink')

    def test_collapsed_group_cannot_restore_visible_child_row_shadows(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            css = ('table{width:100pt;border-spacing:0}td{height:25pt;padding:0}'
                   'tbody{visibility:collapse}tr{visibility:visible}')
            body = '<table><tbody><tr><td></td><td></td></tr></tbody></table>'
            control = self.render(root, css, body)
            with self.raster(root, control, vector_antialias=False) as raster:
                expected = raster.tobytes()
            output = self.render(root, css + 'tr{box-shadow:4pt 3pt 2pt 2pt red}', body)
            with self.raster(root, output, vector_antialias=False) as raster:
                self.assertEqual(raster.tobytes(), expected,
                                 'visible children cannot undo structural group collapse')

    def render(self, root, css, body):
        font = ROOT / 'tests/oracle/assets/DejaVuSans.ttf'
        source = root / 'input.html'
        output = root / 'output.pdf'
        source.write_text(f'''<!doctype html><style>
@font-face{{font-family:Probe;src:url('{font.as_uri()}')}}
@page{{size:300pt {PAGE_HEIGHT}pt;margin:{PAGE_MARGIN}pt}}
body{{margin:0;font:{FONT_SIZE}pt/{LINE_HEIGHT}pt Probe}}
{css}</style>{body}''')
        subprocess.run([str(RENDERER), str(source), str(output)], check=True, capture_output=True)
        return output

    def raster(self, root, output, *, vector_antialias=True, page=1):
        subprocess.run(['pdftoppm', '-r', '72', '-aaVector', 'yes' if vector_antialias else 'no',
                        '-f', str(page), '-singlefile', '-png',
                        str(output), str(root / 'ink')], check=True, capture_output=True)
        with Image.open(root / 'ink.png') as image:
            return image.convert('RGB')

    def test_collapsed_row_transform_moves_border_glyph_and_link_together(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            css = f'''table{{width:{TABLE_WIDTH}pt;border-collapse:collapse;background:blue}}
td{{padding:0;border:2pt solid black;background:yellow;height:40pt;vertical-align:top}}
a{{color:black;text-decoration:none}}p{{margin:0}}'''
            body = '<table><tr><td><a href="#target">A</a></td></tr></table><p id=target>Z</p>'
            normal = self.render(root, css, body)
            before = {g.text: g.origin for g in pdf_content.inspect(normal)['glyphs']}
            before_link = pdf_content.navigation(normal)['links'][0]['rect']
            border_x, border_y = PAGE_MARGIN + TABLE_WIDTH // 2, PAGE_MARGIN + 1
            with self.raster(root, normal, vector_antialias=False) as pixels:
                self.assertEqual(pixels.getpixel((border_x, border_y)), (0, 0, 0))
            moved = self.render(root, css + f'tr{{transform:translate({OFFSET_X}pt,{OFFSET_Y}pt)}}', body)
            after = {g.text: g.origin for g in pdf_content.inspect(moved)['glyphs']}
            self.assertAlmostEqual(after['A'][0] - before['A'][0], OFFSET_X, places=3)
            self.assertAlmostEqual(after['A'][1] - before['A'][1], -OFFSET_Y, places=3)
            self.assertEqual(after['Z'], before['Z'], 'following flow must remain unchanged')
            after_link = pdf_content.navigation(moved)['links'][0]['rect']
            self.assertAlmostEqual(after_link[0] - before_link[0], OFFSET_X, places=3)
            with self.raster(root, moved, vector_antialias=False) as pixels:
                self.assertEqual(pixels.getpixel((border_x + OFFSET_X, border_y + OFFSET_Y)), (0, 0, 0))
                self.assertEqual(pixels.getpixel((border_x, border_y)), (0, 0, 255),
                                 'the old border location must reveal the table background')

    def test_scaled_group_composites_background_glyphs_and_links_about_group_centre(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            row_height = 40
            output = self.render(root, f'''
table{{width:{TABLE_WIDTH}pt;border-spacing:0;background:blue}}
tbody{{background:yellow;transform:scale(.5)}}tr{{height:{row_height}pt}}
td{{padding:0;vertical-align:top}}a{{color:black;text-decoration:none}}p{{margin:0}}
''', '<table><tbody><tr><td><a href="#target">A</a></td></tr>'
                '<tr><td>B</td></tr></tbody></table><p id=target>Z</p>')
            left = PAGE_MARGIN + TABLE_WIDTH / 4
            top = PAGE_MARGIN + row_height / 2
            glyphs = pdf_content.inspect(output)['glyphs']
            found = [g for g in glyphs if g.text == 'A']
            self.assertEqual(len(found), 1)
            self.assertAlmostEqual(found[0].origin[0], left, places=3)
            self.assertAlmostEqual(found[0].origin[1],
                                   PAGE_HEIGHT - top - BASELINE_OFFSET / 2, places=3)
            links = pdf_content.navigation(output)['links']
            self.assertEqual(len(links), 1)
            self.assertAlmostEqual(links[0]['rect'][0], left, places=3)
            with self.raster(root, output, vector_antialias=False) as pixels:
                self.assertEqual(pixels.getpixel((PAGE_MARGIN + 5, int(top + 10))), (0, 0, 255))
                self.assertEqual(pixels.getpixel((int(left + TABLE_WIDTH / 4), int(top + 10))),
                                 (255, 255, 0))

    def test_row_transform_spanning_named_pages_moves_links_with_each_page_width(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = self.render(root, '''
@page{size:240pt 140pt;margin:20pt}@page wide{size:400pt 140pt;margin:20pt}
table{width:100%;border-spacing:0}.first{transform:translate(50%,0)}
.wide{page:wide;break-before:page}td{padding:0;vertical-align:top}
.tail{break-before:page}p{margin:0}a{color:black;text-decoration:none}
''', '<table><tbody><tr class=first><td rowspan=2><p><a href="#target">S</a></p>'
                '<p class=tail><a href="#target">T</a></p></td><td>A</td></tr>'
                '<tr class=wide><td>B</td></tr></tbody></table><p id=target>Z</p>')
            glyphs = pdf_content.inspect(output)['glyphs']
            expected_x = {'S': 120, 'T': 200}
            for label, x in expected_x.items():
                found = [g for g in glyphs if g.text == label]
                self.assertEqual(len(found), 1, label)
                self.assertAlmostEqual(found[0].origin[0], x, places=3, msg=label)
            links = pdf_content.navigation(output)['links']
            self.assertEqual(len(links), 2)
            self.assertEqual(sorted(round(link['rect'][0], 3) for link in links),
                             sorted(expected_x.values()))

    def test_singular_row_transform_hides_pdf_ink_and_links(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = self.render(root,
                f'table{{width:{TABLE_WIDTH}pt;border-spacing:0}}td{{padding:0;height:{LINE_HEIGHT}pt}}tr:first-child{{transform:scale(0)}}',
                '<table><tr><td><a href="#target">H</a></td></tr><tr><td>V</td></tr>'
                '</table><p id=target>Z</p>')
            glyphs = pdf_content.inspect(output)['glyphs']
            self.assertTrue(any(g.text == 'V' for g in glyphs))
            self.assertTrue(any(g.text == 'Z' for g in glyphs))
            self.assertEqual(pdf_content.navigation(output)['links'], [])
            with self.raster(root, output, vector_antialias=False) as pixels:
                actual = pixels.tobytes()
            reference = self.render(root,
                f'table{{width:{TABLE_WIDTH}pt;border-spacing:0}}td{{padding:0;height:{LINE_HEIGHT}pt}}',
                '<table><tr><td></td></tr><tr><td>V</td></tr>'
                '</table><p id=target>Z</p>')
            with self.raster(root, reference, vector_antialias=False) as pixels:
                self.assertEqual(actual, pixels.tobytes(), 'singular row must leave no visible ink')

    def test_transformed_group_contains_fixed_descendants_beyond_positioned_row(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            row_x, row_y = 7, 5
            css = f'''table{{width:{TABLE_WIDTH}pt;border-spacing:0}}td{{padding:0;height:40pt}}
tbody{{transform:translate({OFFSET_X}pt,{OFFSET_Y}pt);background:yellow}}
tr{{position:relative;left:{row_x}pt;top:{row_y}pt}}
.absolute{{position:absolute;left:0;top:0}}.fixed{{position:fixed;left:0;top:0}}
a{{color:black;text-decoration:none}}p{{margin:0}}'''
            output = self.render(root, css,
                '<table><tbody><tr><td>A<a class=absolute href="#target">B</a>'
                '<a class=fixed href="#target">F</a></td></tr></tbody></table><p id=target>Z</p>')
            glyphs = pdf_content.inspect(output)['glyphs']
            for label, x, y in [('B', row_x, row_y), ('F', 0, 0)]:
                found = [g for g in glyphs if g.text == label]
                self.assertEqual(len(found), 1, label)
                self.assertAlmostEqual(found[0].origin[0], PAGE_MARGIN + OFFSET_X + x, places=3)
                self.assertAlmostEqual(found[0].origin[1],
                    PAGE_HEIGHT - PAGE_MARGIN - OFFSET_Y - y - BASELINE_OFFSET, places=3)
            links = pdf_content.navigation(output)['links']
            self.assertEqual(len(links), 2)
            self.assertEqual(sorted(round(link['rect'][0], 3) for link in links),
                             [PAGE_MARGIN + OFFSET_X, PAGE_MARGIN + OFFSET_X + row_x])
            self.assertTrue(all(link['destination']['page'] == 1 for link in links))

    def test_collapsed_internal_outer_shadows_do_not_change_pdf_ink(self):
        body = '<table><tbody><tr><td>A</td><td>B</td></tr></tbody></table>'
        for collapse in ('collapse', 'separate'):
            with self.subTest(collapse=collapse), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                css = f'''table{{width:{TABLE_WIDTH}pt;border-collapse:{collapse};border-spacing:0;
background:blue;box-shadow:3pt 3pt black}}td{{padding:0;background:yellow;height:40pt;
box-shadow:inset 0 0 0 2pt green}}'''
                baseline = self.render(root, css, body)
                with self.raster(root, baseline, vector_antialias=False) as pixels:
                    expected = pixels.tobytes()
                    self.assertEqual(pixels.getpixel((PAGE_MARGIN + 1, PAGE_MARGIN + 30)),
                                     (0, 128, 0), 'inset shadow must remain visible')
                extra = 'td{box-shadow:inset 0 0 0 2pt green,12pt 6pt 2pt red}'
                if collapse == 'collapse':
                    extra += 'tr,tbody{box-shadow:12pt 6pt 2pt red}'
                actual = self.render(root, css + extra, body)
                with self.raster(root, actual, vector_antialias=False) as pixels:
                    if collapse == 'collapse':
                        self.assertEqual(pixels.tobytes(), expected,
                                         'inapplicable outer shadows changed collapsed-table ink')
                    else:
                        self.assertNotEqual(pixels.tobytes(), expected,
                                            'separated cell shadows must still paint')

    def test_structural_opacity_composites_overlapping_cells_once(self):
        cell_width = TABLE_WIDTH // 2
        overlap = cell_width // 2
        for collapse in ('separate', 'collapse'):
            for selector in ('tr', 'tbody'):
                with self.subTest(collapse=collapse, selector=selector), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    output = self.render(root, f'''
table{{width:{TABLE_WIDTH}pt;table-layout:fixed;border-spacing:0;border-collapse:{collapse};background:blue}}
{selector}{{opacity:.5}}td{{padding:0;background:red;height:40pt;vertical-align:top}}
.second{{position:relative;left:-{overlap}pt}}a{{color:black;text-decoration:none}}
''', '<table><tbody><tr><td id=target>A</td><td class=second><a href="#target">B</a></td></tr></tbody></table>')
                    glyphs = pdf_content.inspect(output)['glyphs']
                    label = [g for g in glyphs if g.text == 'B']
                    self.assertEqual(len(label), 1)
                    expected_x = PAGE_MARGIN + cell_width - overlap
                    self.assertAlmostEqual(label[0].origin[0], expected_x, places=3)
                    links = pdf_content.navigation(output)['links']
                    self.assertEqual(len(links), 1)
                    self.assertAlmostEqual(links[0]['rect'][0], expected_x, places=3)
                    self.assertEqual(links[0]['destination']['page'], 1)
                    with self.raster(root, output, vector_antialias=False) as pixels:
                        samples = [pixels.getpixel((PAGE_MARGIN + x, PAGE_MARGIN + 30))
                                   for x in (overlap // 2, overlap + overlap // 2,
                                             cell_width + overlap // 2)]
                        self.assertEqual(samples[0], samples[1], 'overlap must not accumulate opacity')
                        self.assertEqual(samples[1], samples[2], 'both cells share the same group')
                        for actual, expected in zip(samples[0], (128, 0, 127)):
                            self.assertLessEqual(abs(actual - expected), 1)
                        self.assertEqual(pixels.getpixel((PAGE_MARGIN + TABLE_WIDTH - 2,
                                                          PAGE_MARGIN + 30)), (0, 0, 255),
                                         'table background stays outside structural opacity')

    def test_nested_structural_opacity_includes_collapsed_cell_borders(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = self.render(root, f'''
table{{width:{TABLE_WIDTH}pt;table-layout:fixed;border-collapse:collapse;background:blue}}
tbody,tr,td{{opacity:.5}}td{{padding:0;border:2pt solid black;background:red;height:40pt;vertical-align:top}}
''', '<table><tbody><tr><td>A</td><td>B</td></tr></tbody></table>')
            with self.raster(root, output, vector_antialias=False) as pixels:
                for y, expected in ((PAGE_MARGIN + 1, (0, 0, 223)),
                                    (PAGE_MARGIN + 30, (32, 0, 223))):
                    actual = pixels.getpixel((PAGE_MARGIN + 30, y))
                    for channel, value in zip(actual, expected):
                        self.assertLessEqual(abs(channel - value), 2,
                                             f'border/fill opacity at y={y}: {actual}, expected {expected}')

    def test_body_opacity_survives_repeated_group_pages(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = self.render(root, f'''
@page{{size:300pt 100pt;margin:20pt}}table{{height:200pt;width:{TABLE_WIDTH}pt;border-spacing:0;background:blue}}
td{{padding:0;font:10pt/10pt Probe}}thead tr,tfoot tr{{height:10pt}}
thead,tfoot{{background:yellow}}tbody{{opacity:.5}}tbody td{{background:red}}p{{margin:0}}
''', '<table><thead><tr><td>H</td></tr></thead><tbody><tr><td>A</td></tr>'
     '<tr><td>B</td></tr></tbody><tfoot><tr><td>F</td></tr></tfoot></table><p>D</p>')
            glyphs = pdf_content.inspect(output)['glyphs']
            self.assertEqual({g.page for g in glyphs}, set(range(1, 6)))
            for page in range(1, 6):
                for label in ('H', 'F'):
                    self.assertEqual(sum(g.page == page and g.text == label for g in glyphs), 1)
                with self.raster(root, output, page=page, vector_antialias=False) as pixels:
                    x = PAGE_MARGIN + TABLE_WIDTH - 3
                    self.assertEqual(pixels.getpixel((x, 25)), (255, 255, 0))
                    for actual, expected in zip(pixels.getpixel((x, 35)), (128, 0, 127)):
                        self.assertLessEqual(abs(actual - expected), 1, f'page {page}: body opacity')
                    footer_y = 55 if page == 5 else 75
                    self.assertEqual(pixels.getpixel((x, footer_y)), (255, 255, 0),
                                     f'page {page}: footer is outside tbody opacity')

    def test_static_cell_transform_moves_its_glyph_link_and_background(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = self.render(root, f'''
table{{width:{TABLE_WIDTH}pt;border-spacing:0;background:blue}}
td{{padding:0;transform:translate({OFFSET_X}pt,{OFFSET_Y}pt);background:red;height:40pt;vertical-align:top}}
a{{color:black;text-decoration:none}}p{{margin:0}}
''', '<table><tr><td><a href="#target">A</a></td></tr></table><p id=target>D</p>')
            glyphs = pdf_content.inspect(output)['glyphs']
            label = [g for g in glyphs if g.text == 'A']
            self.assertEqual(len(label), 1)
            self.assertAlmostEqual(label[0].origin[0], PAGE_MARGIN + OFFSET_X, places=3)
            self.assertAlmostEqual(label[0].origin[1],
                                   PAGE_HEIGHT - PAGE_MARGIN - OFFSET_Y - BASELINE_OFFSET, places=3)
            links = pdf_content.navigation(output)['links']
            self.assertEqual(len(links), 1)
            self.assertAlmostEqual(links[0]['rect'][0], PAGE_MARGIN + OFFSET_X, places=3)
            with self.raster(root, output, vector_antialias=False) as pixels:
                self.assertEqual(pixels.getpixel((PAGE_MARGIN + 2, PAGE_MARGIN + 30)), (0, 0, 255))
                self.assertEqual(pixels.getpixel((PAGE_MARGIN + OFFSET_X + TABLE_WIDTH - 2,
                                                  PAGE_MARGIN + OFFSET_Y + 30)), (255, 0, 0))

    def test_root_and_column_width_limits_move_glyphs_links_and_ink_together(self):
        constrained_width = 60
        first_column = 40
        for collapse in ('separate', 'collapse'):
            for position, left in (('static', PAGE_MARGIN), ('absolute', 220)):
                with self.subTest(collapse=collapse, position=position), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    output = self.render(root, f'''
table{{position:{position};right:0;top:0;width:100pt;max-width:{constrained_width}pt;
border-collapse:{collapse};border-spacing:0;background:blue}}
.first{{width:30pt;min-width:{first_column}pt;max-width:20pt}}
td{{padding:0}}a{{color:black;text-decoration:none}}
''', '<table><col class=first><col><tr><td id=target>A</td>'
     '<td><a href="#target">B</a></td></tr></table>')
                    glyphs = pdf_content.inspect(output)['glyphs']
                    for label, x in (('A', left), ('B', left + first_column)):
                        matches = [g for g in glyphs if g.text == label]
                        self.assertEqual(len(matches), 1)
                        self.assertAlmostEqual(matches[0].origin[0], x, places=3)
                    links = pdf_content.navigation(output)['links']
                    self.assertEqual(len(links), 1)
                    self.assertAlmostEqual(links[0]['rect'][0], left + first_column, places=3)
                    self.assertEqual(links[0]['destination']['page'], 1)
                    with self.raster(root, output) as pixels:
                        y = PAGE_MARGIN + LINE_HEIGHT - 2
                        self.assertEqual(pixels.getpixel((left + constrained_width - 2, y)),
                                         (0, 0, 255), 'background reaches constrained right edge')
                        self.assertEqual(pixels.getpixel((left + constrained_width + 2, y)),
                                         (255, 255, 255), 'background stops at constrained right edge')

    def test_percentage_relayout_preserves_the_cell_origin_and_grid_extent(self):
        assigned_height = 100
        child_offset = assigned_height - LINE_HEIGHT
        for collapse in ('separate', 'collapse'):
            with self.subTest(collapse=collapse), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                output = self.render(root, f'''
table{{height:{assigned_height}pt;width:{TABLE_WIDTH}pt;table-layout:fixed;border-spacing:0;
border-collapse:{collapse};background:blue}}td{{padding:0;vertical-align:bottom}}
.child{{height:50%;background:red}}.other{{height:40pt}}p{{margin:0}}
''', '<table><tr><td><div class=child>X</div></td><td><div class=other></div></td></tr></table><p>D</p>')
                glyphs = pdf_content.inspect(output)['glyphs']
                for label, offset in (('X', child_offset), ('D', assigned_height)):
                    matches = [g for g in glyphs if g.text == label]
                    self.assertEqual(len(matches), 1, f'{collapse}: {label} occurs once')
                    self.assertAlmostEqual(matches[0].origin[1],
                                           PAGE_HEIGHT - PAGE_MARGIN - offset - BASELINE_OFFSET,
                                           places=3, msg=f'{collapse}: {label} baseline')
                with self.raster(root, output) as pixels:
                    child_x = PAGE_MARGIN + TABLE_WIDTH // 2 - 3
                    child_top = PAGE_MARGIN + child_offset
                    for y, color in ((child_top - 2, (0, 0, 255)),
                                     (child_top + 2, (255, 0, 0)),
                                     (child_top + assigned_height // 2 - 2, (255, 0, 0)),
                                     (child_top + assigned_height // 2 + 2, (255, 255, 255))):
                        self.assertEqual(pixels.getpixel((child_x, y)), color,
                                         f'{collapse}: percentage child extent at y={y}')
                    self.assertEqual(pixels.getpixel((PAGE_MARGIN + TABLE_WIDTH - 3,
                                                     PAGE_MARGIN + assigned_height + 2)),
                                     (255, 255, 255), 'overflow must not extend the grid background')

    def test_assigned_body_height_keeps_repeated_group_space(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = self.render(root, f'''
@page{{size:300pt 100pt;margin:{PAGE_MARGIN}pt}}
table{{height:200pt;width:{TABLE_WIDTH}pt;border-spacing:0}}td{{padding:0;font:10pt/10pt Probe}}
thead tr,tfoot tr{{height:10pt}}.first{{background:blue}}.second{{background:lime}}p{{margin:0}}
''', '<table><thead><tr><td>H</td></tr></thead><tbody><tr><td class=first>A</td></tr>'
     '<tr><td class=second>B</td></tr></tbody><tfoot><tr><td>F</td></tr></tfoot></table><p>D</p>')
            glyphs = pdf_content.inspect(output)['glyphs']
            self.assertEqual({g.page for g in glyphs}, set(range(1, 6)))
            for page in range(1, 6):
                for label in ('H', 'F'):
                    self.assertEqual(sum(g.page == page and g.text == label for g in glyphs), 1,
                                     f'page {page}: repeated {label}')
            for label in ('A', 'B', 'D'):
                self.assertEqual(sum(g.text == label for g in glyphs), 1, f'unique body {label}')
            for page, samples in ((1, ((35, (0, 0, 255)), (75, (255, 255, 255)))),
                                  (3, ((35, (0, 0, 255)), (45, (0, 255, 0)))),
                                  (5, ((45, (0, 255, 0)), (55, (255, 255, 255))))):
                with self.raster(root, output, page=page) as pixels:
                    for y, color in samples:
                        self.assertEqual(pixels.getpixel((PAGE_MARGIN + TABLE_WIDTH - 3, y)), color,
                                         f'page {page}: assigned body/group boundary at y={y}')

    def test_rowspan_distribution_matches_glyph_and_background_boundaries(self):
        span_height, first_minimum, second_minimum = 80, 40, 20
        first_height = span_height * first_minimum / (first_minimum + second_minimum)
        for collapse in ('separate', 'collapse'):
            with self.subTest(collapse=collapse), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                output = self.render(root, f'''
table{{width:{TABLE_WIDTH}pt;table-layout:fixed;border-spacing:0;border-collapse:{collapse}}}
td{{padding:0;vertical-align:top}}.span{{height:{span_height}pt}}
.first{{height:{first_minimum}pt}}.second{{height:{second_minimum}pt}}
.blue{{background:blue}}.green{{background:lime}}p{{margin:0}}
''', '<table><tr><td rowspan=2><div class=span>A</div></td>'
     '<td class=blue><div class=first>B</div></td></tr>'
     '<tr><td class=green><div class=second>C</div></td></tr></table><p>D</p>')
                glyphs = pdf_content.inspect(output)['glyphs']
                for label, offset in (('B', 0), ('C', first_height), ('D', span_height)):
                    matches = [g for g in glyphs if g.text == label]
                    self.assertEqual(len(matches), 1, f'{collapse}: {label} occurs once')
                    self.assertAlmostEqual(matches[0].origin[1],
                                           PAGE_HEIGHT - PAGE_MARGIN - offset - BASELINE_OFFSET,
                                           places=3, msg=f'{collapse}: {label} baseline')
                boundary = PAGE_MARGIN + first_height
                with self.raster(root, output) as pixels:
                    sample_x = PAGE_MARGIN + TABLE_WIDTH - 3
                    for y, color in ((int(boundary) - 2, (0, 0, 255)),
                                     (int(boundary) + 2, (0, 255, 0)),
                                     (PAGE_MARGIN + span_height + 2, (255, 255, 255))):
                        self.assertEqual(pixels.getpixel((sample_x, y)), color,
                                         f'{collapse}: background boundary at y={y}')

    def test_caption_grid_links_and_background_follow_the_positioned_wrapper(self):
        for position in ('relative', 'absolute', 'fixed'):
            with self.subTest(position=position), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                output = self.render(root, f'''table{{position:{position};left:{OFFSET_X}pt;top:{OFFSET_Y}pt;width:{TABLE_WIDTH}pt;
table-layout:fixed;border-spacing:0;background:blue}}td{{padding:0}}a{{color:black;text-decoration:none}}
p{{margin:0}}.next{{break-before:page}}''', '''<table><caption>C</caption><tr><td><a href="#target">A</a></td>
<td id=target>B</td></tr></table><p>After</p><p class=next>Second</p>''')
                glyphs = pdf_content.inspect(output)['glyphs']
                navigation = pdf_content.navigation(output)
                pages = (1, 2) if position == 'fixed' else (1,)
                expected_x = PAGE_MARGIN + OFFSET_X
                expected_caption_y = PAGE_HEIGHT - PAGE_MARGIN - OFFSET_Y - BASELINE_OFFSET
                for page in pages:
                    def glyph(text):
                        matches = [g for g in glyphs if g.page == page and g.text == text]
                        self.assertEqual(len(matches), 1, f'{position}, page {page}: glyph {text}')
                        return matches[0]
                    self.assertAlmostEqual(glyph('C').origin[1], expected_caption_y, places=3)
                    self.assertAlmostEqual(glyph('B').origin[1], expected_caption_y - LINE_HEIGHT, places=3)
                    self.assertAlmostEqual(glyph('B').origin[0], expected_x + TABLE_WIDTH / 2, places=3)
                    links = [link for link in navigation['links'] if link['page'] == page]
                    self.assertEqual(len(links), 1, f'{position}, page {page}: table link')
                    self.assertAlmostEqual(links[0]['rect'][0], expected_x, places=3)
                    self.assertEqual(links[0]['destination']['page'], 1)
                with self.raster(root, output) as pixels:
                    sample_x = expected_x + TABLE_WIDTH - 3
                    caption_y = PAGE_MARGIN + OFFSET_Y + LINE_HEIGHT // 2
                    self.assertEqual(pixels.getpixel((sample_x, caption_y)), (255, 255, 255),
                                     'table background must exclude its caption')
                    self.assertEqual(pixels.getpixel((sample_x, caption_y + LINE_HEIGHT)), (0, 0, 255),
                                     'grid background must move with its positioned wrapper')

    def test_structural_children_keep_wrapper_insets_and_clickable_links(self):
        anchor = '<a class=anchor href="#target">X</a>'
        cells = '<div class=cell id=target>A</div><div class=cell>B</div>'
        row = f'<div class=row>{cells}</div>'
        for parent in ('root', 'group', 'row'):
            with self.subTest(parent=parent), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                contents = row + anchor
                if parent == 'group':
                    contents = f'<div class=group>{contents}</div>'
                elif parent == 'row':
                    contents = f'<div class=row>{cells}{anchor}</div>'
                output = self.render(root, f'''
.root{{display:table;position:relative;width:{TABLE_WIDTH}pt;border-spacing:0}}
.caption{{display:table-caption}}.row{{display:table-row}}
.group{{display:table-row-group}}.cell{{display:table-cell;padding:0}}
.anchor{{position:absolute;left:{OFFSET_X}pt;top:{OFFSET_Y}pt;color:black;text-decoration:none}}''',
                    f'<div class=root><div class=caption>C</div>{contents}</div>')
                glyphs = pdf_content.inspect(output)['glyphs']
                for text, x, y in [('X', PAGE_MARGIN + OFFSET_X, PAGE_MARGIN + OFFSET_Y),
                                   ('C', PAGE_MARGIN, PAGE_MARGIN),
                                   ('A', PAGE_MARGIN, PAGE_MARGIN + LINE_HEIGHT)]:
                    matches = [glyph for glyph in glyphs if glyph.text == text]
                    self.assertEqual(len(matches), 1, f'{parent}: {text} must paint once')
                    self.assertAlmostEqual(matches[0].origin[0], x, places=3)
                    self.assertAlmostEqual(matches[0].origin[1], PAGE_HEIGHT - y - BASELINE_OFFSET, places=3)
                links = pdf_content.navigation(output)['links']
                self.assertEqual(len(links), 1)
                self.assertAlmostEqual(links[0]['rect'][0], PAGE_MARGIN + OFFSET_X, places=3)
                self.assertEqual(links[0]['destination']['page'], 1)

    def test_relative_column_styles_preserve_glyphs_links_and_ink(self):
        body = '<table><colgroup><col><col></colgroup><tr><td><a href="#target">A</a></td><td id=target>B</td></tr></table>'
        for collapse in ('separate', 'collapse'):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                css = f'table{{width:{TABLE_WIDTH}pt;border-collapse:{collapse};border-spacing:3pt}}td{{padding:3pt;border:2pt solid}}col{{background:blue}}colgroup{{background:red}}'
                output = self.render(root, css, body)
                glyphs = pdf_content.inspect(output)['glyphs']
                links = pdf_content.navigation(output)['links']
                with self.raster(root, output) as pixels:
                    ink = pixels.tobytes()
                for selector in ('col', 'colgroup', 'col,colgroup'):
                    with self.subTest(collapse=collapse, selector=selector):
                        output = self.render(root, css + f'{selector}{{position:relative;left:{OFFSET_X}pt;top:{OFFSET_Y}pt}}', body)
                        self.assertEqual(pdf_content.inspect(output)['glyphs'], glyphs)
                        self.assertEqual(pdf_content.navigation(output)['links'], links)
                        with self.raster(root, output) as pixels:
                            self.assertEqual(pixels.tobytes(), ink)

    def test_relative_cells_move_resolved_collapsed_border_coverage(self):
        body = '<table><tr><td>A</td><td>B</td></tr></table><p>After</p>'
        css = f'table{{width:{TABLE_WIDTH}pt;table-layout:fixed;border-collapse:collapse}}td{{padding:3pt;border:2pt solid red}}p{{margin:0}}'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shifted = f'position:relative;left:{OFFSET_X}pt;top:{OFFSET_Y}pt'
            output = self.render(root, css + f'table{{{shifted}}}', body)
            glyphs = pdf_content.inspect(output)['glyphs']
            # Per-cell clipping changes junction antialiasing; compare exact vector coverage.
            with self.raster(root, output, vector_antialias=False) as pixels:
                expected = pixels.tobytes()
            output = self.render(root, css + f'td{{{shifted}}}', body)
            self.assertEqual(pdf_content.inspect(output)['glyphs'], glyphs)
            with self.raster(root, output, vector_antialias=False) as pixels:
                self.assertTrue(pixels.tobytes() == expected,
                                'moving every cell must move the resolved border ink with the text')

    def test_structural_z_index_does_not_trap_positioned_cell_descendants(self):
        rows = '<tr class=first><td>A</td></tr><tr class=second><td>B</td></tr>'
        groups = '<tbody class=first><tr><td>A</td></tr></tbody><tbody class=second><tr><td>B</td></tr></tbody>'
        for body in (rows, groups):
            for collapse in ('separate', 'collapse'):
                for z_index in ('1', 'auto'):
                    with self.subTest(body=body, collapse=collapse, z_index=z_index), tempfile.TemporaryDirectory() as directory:
                        root = Path(directory)
                        output = self.render(root, f'''
table{{width:{TABLE_WIDTH}pt;border-spacing:0;border-collapse:{collapse}}}
td{{padding:0;line-height:{LINE_HEIGHT}pt}}
.first{{position:relative;z-index:2}}.first td{{background:blue}}
.second{{position:relative;z-index:{z_index};top:-{LINE_HEIGHT}pt}}
.second td{{position:relative;z-index:100;background:red}}
''', f'<table>{body}</table>')
                        with self.raster(root, output) as pixels:
                            self.assertEqual(pixels.getpixel((PAGE_MARGIN + TABLE_WIDTH - 3,
                                                             PAGE_MARGIN + LINE_HEIGHT // 2)),
                                             (255, 0, 0),
                                             'CSS Tables ignores z-index on rows and groups; cell contexts remain in the parent context')

    def test_relative_collapsed_borders_preserve_conflicts_styles_and_rtl(self):
        body = '<table><tr><td rowspan=2>A</td><td>B</td></tr><tr><td>C</td></tr></table>'
        for direction in ('ltr', 'rtl'):
            for style in ('solid', 'double', 'dashed', 'dotted'):
                with self.subTest(direction=direction, style=style), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    css = f'''table{{width:{TABLE_WIDTH}pt;table-layout:fixed;direction:{direction};border-collapse:collapse}}
td{{padding:3pt;border:2pt {style} red}}td:first-child{{border-left-width:6pt}}'''
                    shift = f'position:relative;left:{OFFSET_X}pt;top:{OFFSET_Y}pt'
                    output = self.render(root, css + f'table{{{shift}}}', body)
                    with self.raster(root, output, vector_antialias=False) as pixels:
                        expected = pixels.tobytes()
                    output = self.render(root, css + f'td{{{shift}}}', body)
                    with self.raster(root, output, vector_antialias=False) as pixels:
                        self.assertTrue(pixels.tobytes() == expected,
                                        'displacement must preserve resolved borders and dash phase')

    def test_relative_cell_moves_only_its_share_of_a_collapsed_border(self):
        body = '<table><tr><td>A</td><td>B</td></tr></table>'
        css = f'table{{width:{TABLE_WIDTH}pt;table-layout:fixed;border-collapse:collapse}}td{{padding:3pt;border:2pt solid red}}'
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def red_pixels(output):
                with self.raster(root, output) as pixels:
                    return {(x, y) for y in range(pixels.height) for x in range(pixels.width)
                            if pixels.getpixel((x, y)) == (255, 0, 0)}

            normal = red_pixels(self.render(root, css, body))
            self.assertTrue(normal, 'baseline must contain visible resolved borders')
            middle = (min(x for x, _ in normal) + max(x for x, _ in normal) + 1) / 2
            expected = {(x + OFFSET_X, y + OFFSET_Y) if x < middle else (x, y)
                        for x, y in normal}
            moved = red_pixels(self.render(root,
                css + f'td:first-child{{position:relative;left:{OFFSET_X}pt;top:{OFFSET_Y}pt}}', body))
            self.assertEqual(moved, expected,
                             'the shared border must retain its stationary half and move its displaced half')

    def test_structural_z_index_is_ignored_but_cell_z_index_controls_background_order(self):
        for selector, expected in (('tr', (255, 0, 0)), ('tbody', (255, 0, 0)),
                                   ('td', (0, 0, 255))):
            with self.subTest(selector=selector), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                output = self.render(root, f'''
table{{width:{TABLE_WIDTH}pt;border-spacing:0}}td{{padding:0}}
{selector}{{position:relative}}
.first {selector}{{z-index:2;background:blue}}
.second {selector}{{z-index:1;background:red;top:-{LINE_HEIGHT}pt}}
''', '<div class=first><table><tbody><tr><td>A</td></tr></tbody></table></div>'
     '<div class=second><table><tbody><tr><td>B</td></tr></tbody></table></div>')
                with self.raster(root, output) as pixels:
                    self.assertEqual(pixels.getpixel((PAGE_MARGIN + TABLE_WIDTH - 3,
                                                     PAGE_MARGIN + LINE_HEIGHT // 2)),
                                     expected,
                                     'track and group z-index is ignored; table cells retain ordinary z-index ordering')

    def test_relative_groups_preserve_repeated_headers_rowspans_and_links(self):
        lines = '<br>'.join(f'<a href="#target">Line {index}</a>' for index in range(40))
        body = ('<table><thead><tr><td>Header</td><td>Head</td></tr></thead>'
                '<tfoot><tr><td>Footer</td><td>Foot</td></tr></tfoot>'
                f'<tbody><tr><td rowspan=2>{lines}</td><td>First</td></tr>'
                '<tr><td id=target>Target</td></tr></tbody></table><p>After</p>')
        for collapse in ('separate', 'collapse'):
            with self.subTest(collapse=collapse), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                css = f'table{{width:{TABLE_WIDTH}pt;table-layout:fixed;border-collapse:{collapse};border-spacing:0}}td{{padding:3pt;border:2pt solid}}p{{margin:0}}'
                shift = f'position:relative;left:{OFFSET_X}pt;top:{OFFSET_Y}pt'
                output = self.render(root, css + f'table{{{shift}}}', body)
                def glyph_positions(output):
                    return sorted((glyph.page, glyph.text, glyph.origin)
                                  for glyph in pdf_content.inspect(output)['glyphs'])
                expected = glyph_positions(output)
                links = pdf_content.navigation(output)['links']
                self.assertGreater(len({page for page, _, _ in expected}), 1,
                                   'the reference must exercise repeated and split fragments')
                self.assertEqual(len(links), 40)
                output = self.render(root, css + f'thead,tbody,tfoot{{{shift}}}', body)
                self.assertEqual(glyph_positions(output), expected)
                self.assertEqual(pdf_content.navigation(output)['links'], links)

    def test_transformed_caption_and_grid_share_one_opacity_group(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = self.render(root, f'''table{{width:{TABLE_WIDTH}pt;table-layout:fixed;
border-spacing:0;opacity:.5;transform:translate({OFFSET_X}pt,{OFFSET_Y}pt);background:red}}
caption{{background:red}}td{{padding:0}}''',
                '<table><caption>C</caption><tr><td>A</td></tr></table>')
            with self.raster(root, output) as pixels:
                sample_x = PAGE_MARGIN + OFFSET_X + TABLE_WIDTH - 3
                caption_y = PAGE_MARGIN + OFFSET_Y + LINE_HEIGHT // 2
                for y in (caption_y, caption_y + LINE_HEIGHT):
                    actual = pixels.getpixel((sample_x, y))
                    self.assertTrue(all(abs(a - b) <= 1 for a, b in zip(actual, (255, 128, 128))),
                                    f'one half-opacity red group at {(sample_x, y)}: {actual}')
                self.assertEqual(pixels.getpixel((PAGE_MARGIN + 1, PAGE_MARGIN + 1)), (255, 255, 255),
                                 'the caption must move with the transformed table')

    def test_wrapper_clips_overflow_from_both_caption_and_grid(self):
        for translated in (False, True):
            with self.subTest(translated=translated), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                transform = f'transform:translate({OFFSET_X}pt,{OFFSET_Y}pt)' if translated else ''
                output = self.render(root, f'''table{{width:{TABLE_WIDTH}pt;table-layout:fixed;
border-spacing:0;overflow:hidden;{transform}}}td{{padding:0}}
caption,td div{{position:relative;background:red}}
span{{position:absolute;left:150pt;top:0;width:30pt;height:5pt;background:blue}}''',
                    '<table><caption>C<span></span></caption><tr><td><div>A<span></span></div></td></tr></table>')
                left = PAGE_MARGIN + (OFFSET_X if translated else 0)
                top = PAGE_MARGIN + (OFFSET_Y if translated else 0)
                with self.raster(root, output) as pixels:
                    for y in (top + 2, top + LINE_HEIGHT + 2):
                        self.assertEqual(pixels.getpixel((left + TABLE_WIDTH - 5, y)), (0, 0, 255),
                                         'overflow inside the wrapper edge must remain visible')
                        self.assertEqual(pixels.getpixel((left + TABLE_WIDTH + 5, y)), (255, 255, 255),
                                         'caption and grid overflow must stop at the same wrapper edge')

    def test_transform_origin_uses_the_full_caption_and_grid_height(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            css = f'table{{width:{TABLE_WIDTH}pt;table-layout:fixed;border-spacing:0}}td{{padding:0}}'
            body = '<table><caption>C</caption><tr><td>A</td></tr></table>'
            normal = pdf_content.inspect(self.render(root, css, body))['glyphs']
            scaled = pdf_content.inspect(self.render(root, css +
                'table{transform:scale(2);transform-origin:0 100%}', body))['glyphs']
            self.assertEqual([g.text for g in normal], [g.text for g in scaled])
            origin_y = PAGE_HEIGHT - (PAGE_MARGIN + 2 * LINE_HEIGHT)
            for before, after in zip(normal, scaled):
                self.assertAlmostEqual(after.origin[0], 2 * before.origin[0] - PAGE_MARGIN, places=3)
                self.assertAlmostEqual(after.origin[1], 2 * before.origin[1] - origin_y, places=3,
                                       msg=f'{before.text}: scale around the wrapper bottom, not the grid bottom')
