"""Inspect Grid pixels, image placement, glyphs and links in actual PDF output."""
from pathlib import Path
import tempfile
import unittest

from PIL import Image, ImageChops

import table_positioning_pdf
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RED = (255, 0, 0)
BLUE = (0, 0, 255)
WHITE = (255, 255, 255)


class GridPdf(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='ferri-pdf-grid-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def test_track_boxes_and_image_pixels_match_explicit_geometry(self):
        image = (ROOT / 'tests/oracle/assets/image-pattern.png').as_uri()
        body = f'<section><div class=first></div><img src="{image}"><div class=last></div></section>'
        shared = '.first{background:red}.last{background:blue}img{object-fit:fill}'
        reference = self.renderer.render(self.root, shared +
            'section{position:relative;width:200pt;height:90pt}'
            'section>*{position:absolute}.first{left:0;top:0;width:60pt;height:40pt}'
            'img{left:70pt;top:0;width:130pt;height:40pt}'
            '.last{left:0;top:50pt;width:200pt;height:40pt}', body)
        with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
            expected = raster.tobytes()
        output = self.renderer.render(self.root, shared +
            'section{display:grid;width:200pt;grid-template-columns:60pt 1fr;'
            'grid-template-rows:40pt 40pt;gap:10pt}'
            'img{width:100%;height:100%}.last{grid-column:1/-1}', body)
        self.assertEqual(len(pdf_content.inspect(output)['images']), 1)
        with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
            self.assertEqual(raster.tobytes(), expected,
                             'Grid boxes and decoded image placement differ from explicit track coordinates')

    def test_overlap_obeys_order_and_nonpositioned_z_index(self):
        for z_index, expected in [('', RED), ('.first{z-index:0}.last{z-index:1}', BLUE)]:
            with self.subTest(z_index=z_index):
                output = self.renderer.render(self.root,
                    'section{display:grid;width:100pt;grid-template-columns:100pt;grid-template-rows:60pt}'
                    'section>div{grid-area:1/1}.first{order:2;background:red}'
                    '.last{order:1;background:blue}' + z_index,
                    '<section><div class=first></div><div class=last></div></section>')
                with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
                    self.assertEqual(raster.getpixel((30, 30)), expected)
                    self.assertEqual(raster.getpixel((125, 30)), WHITE)

    def test_grid_effects_match_explicit_geometry_and_link_bounds(self):
        body = ('<section><div class=first><i></i></div>'
                '<div class=last><a href="mailto:reader@example.org">Link</a></div></section>')
        shared = ('section{width:100pt;height:40pt;overflow:hidden}'
                  '.first{position:relative;overflow:hidden;border-radius:8pt;background:red}'
                  'i{position:absolute;left:30pt;top:10pt;width:80pt;height:20pt;background:blue}'
                  '.last{background:lime;transform:translateX(-15pt)}a{color:black;text-decoration:none}')
        for effect in ('transform:translate(7pt,9pt)', 'opacity:.5', 'transform:translate(7pt,9pt);opacity:.5'):
            with self.subTest(effect=effect):
                effects = shared + 'section{' + effect + '}'
                reference = self.renderer.render(self.root, effects +
                    'section{position:relative}section>div{position:absolute;width:60pt;height:40pt}'
                    '.first{left:0;top:0}.last{left:70pt;top:0}', body)
                with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
                    expected = raster.copy()
                    self.addCleanup(expected.close)
                links = pdf_content.navigation(reference)['links']
                output = self.renderer.render(self.root, effects +
                    'section{display:grid;grid-template-columns:60pt 60pt;column-gap:10pt}', body)
                self.assertEqual(pdf_content.navigation(output)['links'], links)
                with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
                    self.assertIsNone(ImageChops.difference(raster, expected).getbbox(),
                                      'Grid transforms, opacity and clipping must retain owning geometry')

    def test_contents_preserves_rendered_heading_destination_after_a_break(self):
        output = self.renderer.render(self.root,
            '@page{size:240pt 120pt;margin:10pt}section{display:grid;grid-template-columns:100pt}'
            '.contents{display:contents}h2{margin:0;font:10pt/14pt Probe;break-before:page}'
            'a{color:black;text-decoration:none}',
            '<p><a href="#group">Group</a> <a href="#heading">Heading</a></p>'
            '<section><div>Lead</div><div class=contents id=group><h2 id=heading>Target</h2></div></section>')
        self.assertEqual(len(list(pdf_content.Document(output).pages())), 2)
        links = pdf_content.navigation(output)['links']
        self.assertEqual(len(links), 1,
                         'only the rendered heading has a target box; its contents ancestor has none')
        for link in links:
            self.assertEqual(link['destination']['page'], 2,
                             'heading destination must follow its Grid item across the break')
            self.assertAlmostEqual(link['destination']['top'], 110.0, places=3)
        outline = pdf_content.navigation(output)['outline']
        self.assertEqual([item['title'] for item in outline], ['Target'])
        self.assertEqual(outline[0]['destination'], links[0]['destination'])

    def test_percentage_height_image_keeps_ratio_and_grid_alignment(self):
        body = ('<section><svg xmlns="http://www.w3.org/2000/svg" width="80" height="40">'
                '<rect width="40" height="40" fill="red"/>'
                '<rect x="40" width="40" height="40" fill="blue"/></svg></section>')
        reference = self.renderer.render(self.root,
            'section{position:relative;width:100pt;height:80pt}'
            'svg{position:absolute;left:10pt;top:20pt;width:80pt;height:40pt}', body)
        with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
            expected = raster.copy()
            self.addCleanup(expected.close)
        output = self.renderer.render(self.root,
            'section{display:grid;width:100pt;grid-template-columns:100pt;grid-template-rows:80pt;'
            'justify-items:center;align-items:center}svg{height:50%}', body)
        with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
            self.assertIsNone(ImageChops.difference(raster, expected).getbbox(),
                              'percentage-height image must preserve ratio and center in its Grid area')

    def test_fragmented_absolute_grid_areas_paint_on_their_anchor_pages(self):
        shared = ('@page{size:240pt 140pt;margin:10pt}p{margin:0}'
                  '.direct{background:red}.nested{background:blue}')
        reference = self.renderer.render(self.root, shared +
            'section{position:relative;height:70pt}section+section{break-before:page}'
            '.direct,.nested{position:absolute;left:90pt;top:40pt;width:120pt;height:40pt}',
            '<section><p>First</p><div class=direct>Direct</div></section>'
            '<section><p>Later</p><div class=nested>Nested</div></section>')
        expected = []
        for page in (1, 2):
            with self.renderer.raster(self.root, reference, vector_antialias=False, page=page) as raster:
                expected.append(raster.copy())
                self.addCleanup(expected[-1].close)
        output = self.renderer.render(self.root, shared +
            'section{display:grid;position:relative;grid-template-columns:80pt 120pt;'
            'grid-template-rows:30pt 40pt;gap:10pt}.later{grid-area:2/1;break-before:page}'
            '.direct,.nested{position:absolute;grid-column:2/3;grid-row:2/3;inset:0}',
            '<section><div>First</div><div class=later>Later<div class=nested>Nested</div></div>'
            '<div class=direct>Direct</div></section>')
        self.assertEqual(len(list(pdf_content.Document(output).pages())), 2)
        for page, reference in enumerate(expected, start=1):
            with self.renderer.raster(self.root, output, vector_antialias=False, page=page) as raster:
                self.assertIsNone(ImageChops.difference(raster, reference).getbbox(),
                                  f'absolute Grid area must retain track coordinates on anchor page {page}')

    def test_absolute_auto_edges_follow_visible_descendant_overflow(self):
        body = '<section><div class=item><div class=desc></div></div><div class=abs></div></section>'
        for item, descendant, edge in (
            ('width:160pt', '', 160),
            ('', 'width:160pt', 160),
            ('overflow:hidden', 'width:160pt;position:relative;left:200pt', 100),
            ('', 'width:160pt;transform:translateX(30pt)', 190),
            ('width:160pt;margin-right:20pt', '', 180),
            ('position:relative', 'position:absolute;left:0;top:0;width:160pt', 160),
            ('position:relative;overflow:hidden', 'position:absolute;left:0;top:0;width:160pt', 100),
            ('position:relative;transform:translateX(30pt)', 'position:absolute;left:0;top:0;width:160pt', 190),
        ):
            with self.subTest(item=item, descendant=descendant):
                shared = ('section{position:relative;width:100pt;height:40pt}'
                          f'.item{{{item}}}.desc{{height:20pt;{descendant}}}'
                          '.abs{position:absolute;top:0;width:5pt;height:5pt;background:red}')
                reference = self.renderer.render(self.root, shared + f'.abs{{left:{edge - 5}pt}}', body)
                with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
                    self.assertEqual(raster.getpixel((20 + edge - 3, 22)), RED)
                    expected = raster.copy()
                    self.addCleanup(expected.close)
                output = self.renderer.render(self.root, shared +
                    'section{display:grid;grid-template-columns:100pt;grid-template-rows:40pt}.abs{right:0}', body)
                with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
                    self.assertIsNone(ImageChops.difference(raster, expected).getbbox(),
                                      'automatic Grid edge must include only reachable, unclipped overflow')

    def test_auto_edges_exclude_wholly_unreachable_but_keep_crossing_overflow(self):
        body = '<section><div><div class=desc></div></div><div class=abs></div></section>'
        for left, bottom in ((-200, 40), (-160, 40), (-159, 120)):
            with self.subTest(left=left):
                shared = ('section{position:relative;width:100pt;height:40pt}'
                          f'.desc{{position:relative;left:{left}pt;top:100pt;width:160pt;height:20pt}}'
                          '.abs{position:absolute;right:0;width:5pt;height:5pt;background:red}')
                reference = self.renderer.render(self.root, shared + f'.abs{{top:{bottom - 5}pt}}', body)
                with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
                    self.assertEqual(raster.getpixel((117, 20 + bottom - 3)), RED)
                    expected = raster.copy()
                    self.addCleanup(expected.close)
                output = self.renderer.render(self.root, shared +
                    'section{display:grid;grid-template-columns:100pt;grid-template-rows:40pt}.abs{bottom:0}', body)
                with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
                    self.assertIsNone(ImageChops.difference(raster, expected).getbbox())

    def test_nested_column_overflow_is_shifted_and_clipped_by_its_owner(self):
        for prefix, clipping, edge in (
            ('', '', 160),
            ('<div style="height:40pt"></div>', '', 210),
            ('<div style="height:40pt"></div>', 'overflow:hidden;', 100),
        ):
            with self.subTest(prefix=prefix, clipping=clipping):
                body = (f'<section><div class=columns>{prefix}<div class=wide></div></div>'
                        '<div class=abs></div></section>')
                shared = ('section{position:relative;width:100pt;height:40pt}'
                          f'.columns{{{clipping}columns:2;column-gap:0;height:40pt;column-fill:auto}}'
                          '.wide{width:160pt;height:20pt}'
                          '.abs{position:absolute;top:0;width:5pt;height:5pt;background:red}')
                reference = self.renderer.render(self.root, shared + f'.abs{{left:{edge - 5}pt}}', body)
                with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
                    self.assertEqual(raster.getpixel((20 + edge - 3, 22)), RED)
                    expected = raster.copy()
                    self.addCleanup(expected.close)
                output = self.renderer.render(self.root, shared +
                    'section{display:grid;grid-template-columns:100pt;grid-template-rows:40pt}.abs{right:0}', body)
                with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
                    self.assertIsNone(ImageChops.difference(raster, expected).getbbox())

    def test_absolute_children_interleave_with_grid_items_in_paint_order(self):
        table = '<table><tr><td></td></tr></table>'
        marker = '<div class=marker></div>'
        for before, item_order, absolute_order, color in (
            (True, 0, 0, RED), (False, 0, 0, BLUE),
            (True, 1, 0, BLUE), (False, -1, 0, RED),
            (True, 0, -10, RED), (False, 0, 10, BLUE),
        ):
            with self.subTest(before=before, item_order=item_order, absolute_order=absolute_order):
                output = self.renderer.render(self.root,
                    'section{display:grid;position:relative;width:100pt;grid-template-columns:100pt;grid-template-rows:40pt}'
                    f'table{{width:160pt;height:40pt;border-spacing:0;transform:translateX(30pt);order:{item_order}}}'
                    'td{padding:0;background:blue}'
                    f'.marker{{position:absolute;left:50pt;top:10pt;width:5pt;height:5pt;background:red;order:{absolute_order}}}',
                    '<section>' + (table + marker if before else marker + table) + '</section>')
                with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
                    self.assertEqual(raster.getpixel((72, 32)), color)

    def test_transformed_table_parts_expand_grid_auto_edges(self):
        for selector, edge in (('table', 190), ('tbody', 190), ('tr', 190), ('td', 190), ('tbody,tr', 220)):
            with self.subTest(selector=selector):
                shared = ('section{position:relative;width:100pt;height:40pt}'
                          'table{width:160pt;height:40pt;border-spacing:0}td{padding:0;height:20pt;background:blue}'
                          f'{selector}{{transform:translateX(30pt)}}'
                          '.marker{position:absolute;top:25pt;width:5pt;height:5pt;background:red}')
                body = ('<section><table><tbody><tr><td></td></tr></tbody></table>'
                        '<div class=marker></div></section>')
                reference = self.renderer.render(self.root, shared + f'.marker{{left:{edge - 5}pt}}', body)
                with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
                    self.assertEqual(raster.getpixel((20 + edge - 3, 47)), RED)
                    expected = raster.copy()
                    self.addCleanup(expected.close)
                output = self.renderer.render(self.root, shared +
                    'section{display:grid;grid-template-columns:100pt;grid-template-rows:40pt}.marker{right:0}', body)
                with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
                    self.assertIsNone(ImageChops.difference(raster, expected).getbbox())

    def test_fragmented_items_conserve_glyphs_and_resolve_links(self):
        output = self.renderer.render(self.root,
            '@page{size:240pt 100pt;margin:10pt}p{margin:0}'
            'section{display:grid;grid-template-columns:100pt 100pt}'
            '.next{break-before:page}a{color:black;text-decoration:none}',
            '<section><div><p><a href="#target">Start</a></p>'
            '<p class=next id=target>Target</p></div>'
            '<div><a href="mailto:reader@example.org">Peer</a></div><div>After</div></section>')
        document = pdf_content.Document(output)
        self.assertEqual(len(list(document.pages())), 2)
        glyphs = pdf_content.inspect(output)['glyphs']
        for page, expected in [(1, 'StartPeer'), (2, 'TargetAfter')]:
            self.assertEqual(''.join(glyph.text for glyph in glyphs if glyph.page == page), expected,
                             'fragmented text must paint once, without loss or reordering')
        links = pdf_content.navigation(output)['links']
        self.assertEqual(len(links), 2)
        internal = next(link for link in links if 'destination' in link)
        self.assertEqual((internal['page'], internal['destination']['page']), (1, 2))
        self.assertAlmostEqual(internal['destination']['top'], 90.0, places=3)
        external = next(link for link in links if 'uri' in link)
        self.assertEqual((external['page'], external['uri']), (1, 'mailto:reader@example.org'))

    def test_row_span_and_forced_boundaries_keep_independent_pdf_destinations(self):
        output = self.renderer.render(self.root,
            '@page{size:240pt 160pt;margin:10pt}p{margin:0}a{color:black;text-decoration:none}'
            'section{display:grid;grid-template-columns:100pt 100pt;grid-template-rows:40pt 40pt 40pt;row-gap:10pt}'
            '.span{grid-column:1;grid-row:1/4}.first{grid-area:1/2}'
            '.second{grid-area:2/2;break-before:page}.third{grid-area:3/2}.next{break-before:page}',
            '<section><div class=span><p>SpanA</p><p class=next>SpanB</p>'
            '<p class=next id=target>SpanC</p></div>'
            '<div class=first><a href="#target">PeerA</a></div>'
            '<div class=second><a href="#target">PeerB</a></div>'
            '<div class=third><a href="#target">PeerC</a></div></section><p>After</p>')
        self.assertEqual(len(list(pdf_content.Document(output).pages())), 3)
        glyphs = pdf_content.inspect(output)['glyphs']
        for page, expected in enumerate(('SpanAPeerA', 'SpanBPeerBPeerCAfter', 'SpanC'), start=1):
            self.assertEqual(''.join(glyph.text for glyph in glyphs if glyph.page == page), expected,
                             f'spanning continuation and peer text must remain independent on page {page}')
        links = pdf_content.navigation(output)['links']
        self.assertEqual([link['page'] for link in links], [1, 2, 2])
        for link in links:
            self.assertEqual(link['destination']['page'], 3)
            self.assertAlmostEqual(link['destination']['top'], 150.0, places=3)

    def test_fragmented_grid_item_decorations_match_block_slice_and_clone(self):
        body = '<section><div class=item>' + ''.join(f'<p>Line{index}</p>' for index in range(8)) + '</div></section>'
        for decoration in ('slice', 'clone'):
            with self.subTest(decoration=decoration):
                shared = ('@page{size:200pt 100pt;margin:10pt}p{margin:0;widows:1;orphans:1}'
                          '.item{padding:5pt;border:3pt solid red;box-decoration-break:' + decoration + '}')
                reference = self.renderer.render(self.root, shared + '.item{width:84pt}', body)
                page_count = len(list(pdf_content.Document(reference).pages()))
                self.assertGreater(page_count, 1, 'the reference must fragment')
                expected = []
                for page in range(1, page_count + 1):
                    with self.renderer.raster(self.root, reference, vector_antialias=False, page=page) as raster:
                        expected.append(raster.tobytes())
                output = self.renderer.render(self.root, shared +
                    'section{display:grid;width:100pt;grid-template-columns:100pt}', body)
                self.assertEqual(len(list(pdf_content.Document(output).pages())), page_count)
                for page, pixels in enumerate(expected, start=1):
                    with self.renderer.raster(self.root, output, vector_antialias=False, page=page) as raster:
                        with Image.frombytes('RGB', raster.size, pixels) as reference:
                            difference = ImageChops.difference(raster, reference).getbbox()
                            actual_edge = [y for y in range(raster.height) if raster.getpixel((10, y)) == RED]
                            expected_edge = [y for y in range(reference.height) if reference.getpixel((10, y)) == RED]
                            self.assertIsNone(difference,
                                f'{decoration} fragment {page}: differing bounds {difference}, '
                                f'border end {max(actual_edge, default=None)} vs {max(expected_edge, default=None)}')

    def test_fragmentation_preserves_explicit_grid_item_height_constraints(self):
        body = '<section><div class=item>' + ''.join(f'<p>Line{index}</p>' for index in range(8)) + '</div></section>'
        for constraint in ('height:40pt', 'max-height:40pt', 'height:100pt', 'max-height:100pt', 'max-height:160pt'):
            with self.subTest(constraint=constraint):
                shared = ('@page{size:200pt 100pt;margin:10pt}p{margin:0;widows:1;orphans:1}'
                          '.item{padding:5pt;border:3pt solid red;' + constraint + '}')
                reference = self.renderer.render(self.root, shared + '.item{width:84pt}', body)
                expected = []
                for page in range(1, len(list(pdf_content.Document(reference).pages())) + 1):
                    with self.renderer.raster(self.root, reference, vector_antialias=False, page=page) as raster:
                        expected.append(raster.copy())
                        self.addCleanup(expected[-1].close)
                self.assertGreater(len(expected), 1)
                output = self.renderer.render(self.root, shared +
                    'section{display:grid;width:100pt;grid-template-columns:100pt}', body)
                self.assertEqual(len(list(pdf_content.Document(output).pages())), len(expected))
                for page, reference in enumerate(expected, start=1):
                    with self.renderer.raster(self.root, output, vector_antialias=False, page=page) as raster:
                        self.assertIsNone(ImageChops.difference(raster, reference).getbbox(),
                                          f'{constraint} fragment {page} must retain its constrained border')

    def test_grid_columns_preserve_ink_and_cross_page_destinations(self):
        shared = ('@page{size:420pt 100pt;margin:10pt}'
                  'a{color:black;text-decoration:none}.item{background:#eee}')
        def content(index):
            return f'<a id="item{index}" href="#item10">Item{index:02}</a>'
        reference_body = ''
        for start, end in ((0, 8), (8, 12)):
            reference_body += '<section class=page>'
            for index in range(start, end):
                x = ((index // 4) % 2) * 210 + (index % 2) * 100
                y = ((index // 2) % 2) * 40
                reference_body += (f'<div class=item style="left:{x}pt;top:{y}pt">'
                                   + content(index) + '</div>')
            reference_body += '</section>'
        reference = self.renderer.render(self.root, shared +
            '.page{position:relative;height:80pt}.page+.page{break-before:page}'
            '.item{position:absolute;width:90pt;height:40pt}', reference_body)
        expected = []
        for page in (1, 2):
            with self.renderer.raster(self.root, reference, vector_antialias=False, page=page) as raster:
                expected.append(raster.copy())
                self.addCleanup(expected[-1].close)
        expected_links = pdf_content.navigation(reference)['links']
        output = self.renderer.render(self.root, shared +
            '.columns{column-count:2;column-gap:20pt;column-fill:auto}'
            '.grid{display:grid;grid-template-columns:1fr 1fr;grid-auto-rows:40pt;column-gap:10pt}',
            '<section class=columns><div class=grid>' +
            ''.join('<div class=item>' + content(index) + '</div>' for index in range(12)) +
            '</div></section>')
        self.assertEqual(len(list(pdf_content.Document(output).pages())), 2)
        self.assertEqual(pdf_content.navigation(output)['links'], expected_links,
                         'column translation must preserve link rectangles and target coordinates')
        for page, reference in enumerate(expected, start=1):
            with self.renderer.raster(self.root, output, vector_antialias=False, page=page) as raster:
                self.assertIsNone(ImageChops.difference(raster, reference).getbbox(),
                                  f'Grid column ink differs from explicit geometry on page {page}')

    def test_cyclic_row_percentages_paint_overflow_without_growing_container(self):
        body = '<section><div class=first>A</div><div class=last>B</div></section><p>After</p>'
        for tracks, gap, offset in (('50% 50%', '0', 30), ('75% 75%', '0', 45), ('auto auto', '10%', 26)):
            with self.subTest(tracks=tracks, gap=gap):
                shared = ('p{margin:0;position:relative}section{width:100pt;background:lime}'
                          '.first{height:20pt;background:red}.last{height:40pt;background:blue}')
                reference = self.renderer.render(self.root, shared +
                    'section{position:relative;height:60pt}section>div{position:absolute;width:100pt}'
                    f'.first{{top:0}}.last{{top:{offset}pt}}', body)
                with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
                    expected = raster.copy()
                    self.addCleanup(expected.close)
                output = self.renderer.render(self.root, shared +
                    f'section{{display:grid;grid-template-rows:{tracks};row-gap:{gap}}}', body)
                with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
                    self.assertIsNone(ImageChops.difference(raster, expected).getbbox(),
                                      'cyclic tracks must overflow the intrinsic container without moving following flow')

    def test_continued_grid_ink_matches_destination_page_width(self):
        continuation = 'SecondA alpha beta gamma delta epsilon zeta eta theta'
        for first_width, later_width in ((220, 420), (420, 220)):
            with self.subTest(first_width=first_width, later_width=later_width):
                shared = (f'@page{{size:{later_width}pt 180pt;margin:10pt}}'
                          'p{margin:0}section{display:grid;grid-template-columns:1fr 1fr;gap:10pt}')
                reference = self.renderer.render(self.root, shared,
                    f'<section><div><p>{continuation}</p></div><div><p>SecondB</p></div></section>')
                with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
                    expected = raster.copy()
                self.addCleanup(expected.close)
                output = self.renderer.render(self.root, shared +
                    f'@page:first{{size:{first_width}pt 180pt}}.next{{break-before:page}}',
                    f'<section><div><p>FirstA</p><p class=next>{continuation}</p></div>'
                    '<div><p>FirstB</p><p class=next>SecondB</p></div></section>')
                self.assertEqual(len(list(pdf_content.Document(output).pages())), 2)
                with self.renderer.raster(self.root, output, vector_antialias=False, page=2) as raster:
                    self.assertEqual(raster.size, expected.size)
                    self.assertIsNone(ImageChops.difference(raster, expected).getbbox(),
                                      'continued Grid ink must match fresh destination-width layout')

    def test_page_break_suppresses_grid_gap_in_painted_pixels(self):
        output = self.renderer.render(self.root,
            '@page{size:200pt 120pt;margin:10pt}'
            'section{display:grid;grid-template-columns:100pt;grid-template-rows:90pt 20pt;row-gap:20pt}'
            '.first{background:red}.last{background:blue}',
            '<section><div class=first></div><div class=last></div></section>')
        self.assertEqual(len(list(pdf_content.Document(output).pages())), 2)
        with self.renderer.raster(self.root, output, vector_antialias=False, page=2) as raster:
            self.assertEqual(raster.getpixel((20, 9)), WHITE)
            self.assertEqual(raster.getpixel((20, 10)), BLUE)
            self.assertEqual(raster.getpixel((20, 29)), BLUE)
            self.assertEqual(raster.getpixel((20, 30)), WHITE)


if __name__ == '__main__':
    unittest.main(verbosity=2)
