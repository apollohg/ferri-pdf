"""Check anchor positioning in serialized PDF pixels, glyphs and navigation (§2–6)."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf
from oracle import pdf_content

PROFILE = ('@page{size:300pt 240pt;margin:20pt}html,body{margin:0;padding:0}'
           '.anchor{anchor-name:--a;width:60pt;height:30pt;margin-left:40pt;background:blue}'
           '.target{position:absolute;position-anchor:--a;width:20pt;height:10pt;background:red}')
BODY = '<div class=anchor>A</div><div class=target><a href="https://example.org/">T</a></div>'


class AnchorPositioningPdf(unittest.TestCase):
    def test_anchor_functions_areas_fallbacks_and_transforms_match_physical_geometry(self):
        cases = [
            ('.target{left:anchor(left);top:calc(anchor(bottom) + 5pt);'
             'width:anchor-size(width);height:anchor-size(height)}', (40, 35, 60, 30)),
            ('.target{position-area:bottom}', (60, 30, 20, 10)),
            ('.target{top:anchor(bottom);justify-self:anchor-center}', (60, 30, 20, 10)),
            ('.anchor{margin-top:170pt}.target{position-area:bottom;'
             'position-try-fallbacks:flip-block}', (60, 160, 20, 10)),
            ('@position-try --above{position-area:top}.anchor{margin-top:170pt}'
             '.target{position-area:bottom;position-try-fallbacks:--above}', (60, 160, 20, 10)),
            ('.target{position-area:left;position-try-fallbacks:right;'
             'position-try-order:most-width}', (100, 10, 20, 10)),
            ('.anchor{transform:translate(25pt,5pt)}'
             '.target{left:anchor(left);top:anchor(bottom)}', (65, 35, 20, 10)),
        ]
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-anchors-') as directory:
            root = Path(directory)
            for css, (left, top, width, height) in cases:
                with self.subTest(css=css):
                    output = renderer.render(root, PROFILE + css, BODY)
                    glyphs = [(glyph.text, glyph.origin) for glyph in pdf_content.inspect(output)['glyphs']]
                    links = pdf_content.navigation(output)['links']
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        actual = raster.tobytes()
                    reference = ('.target{position-anchor:none;position-area:none;position-try:none;'
                                 f'justify-self:normal;left:{left}pt;top:{top}pt;'
                                 f'width:{width}pt;height:{height}pt}}')
                    output = renderer.render(root, PROFILE + css + reference, BODY)
                    self.assertEqual(glyphs, [(glyph.text, glyph.origin)
                                             for glyph in pdf_content.inspect(output)['glyphs']],
                                     'the chosen geometry must move glyphs with the anchored box')
                    self.assertEqual(links, pdf_content.navigation(output)['links'],
                                     'the hyperlink must follow the anchored box')
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        self.assertEqual(actual, raster.tobytes(),
                                         'serialized anchor geometry differs from the independently placed boxes')

    def test_fixed_anchor_geometry_and_visibility_follow_each_physical_page(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        cases = [
            ('.anchor{position:fixed;left:50%;top:0;margin:0;width:50%}'
             '.target{position:fixed;position-anchor:--a;left:anchor(left);'
             'top:anchor(bottom);width:anchor-size(width)}',
             ((150, 50, 280, 60), (200, 50, 380, 60))),
            ('.anchor{margin-left:200pt;width:20pt}'
             '.target{position:fixed;position-anchor:--a;position-area:right;'
             'width:60pt;position-visibility:no-overflow}',
             (None, (240, 30, 300, 40))),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-anchor-pages-') as directory:
            root = Path(directory)
            for css, rectangles in cases:
                with self.subTest(css=css):
                    output = renderer.render(root, PROFILE + css +
                        '@page wide{size:400pt 240pt}.next{page:wide;break-before:page}',
                        ('<p>First</p>' if css.startswith('.anchor{position:fixed') else '') + BODY + '<p class=next>Next</p>')
                    glyphs = pdf_content.inspect(output)['glyphs']
                    for page, rectangle in enumerate(rectangles, start=1):
                        with renderer.raster(root, output, vector_antialias=False, page=page) as raster:
                            positions = [(x, y) for y in range(raster.height)
                                         for x in range(raster.width)
                                         if raster.getpixel((x, y)) == (255, 0, 0)]
                        bounds = ((min(x for x, _ in positions), min(y for _, y in positions),
                                   max(x for x, _ in positions) + 1, max(y for _, y in positions) + 1)
                                  if positions else None)
                        self.assertEqual(bounds, rectangle, f'physical page {page}: anchor rectangle')
                        text = [glyph for glyph in glyphs if glyph.page == page and glyph.text == 'T']
                        self.assertEqual(len(text), int(rectangle is not None),
                                         f'physical page {page}: subtree visibility must match the box')
                        if text:
                            self.assertAlmostEqual(text[0].origin[0], rectangle[0], places=3,
                                                   msg='fixed child text must follow its page-specific anchor')

    def test_anchor_visibility_uses_ink_overflow_and_intervening_clip_paths(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        cases = [
            ('section{width:20pt;height:40pt;overflow:hidden}'
             '.anchor{margin-left:30pt;width:10pt;height:10pt;box-shadow:-20pt 0 green}', True),
            ('section{width:100pt;height:100pt;clip-path:inset(100%)}'
             '.anchor{margin-left:30pt;width:10pt;height:10pt}', False),
            ("section{width:100pt;height:100pt;clip-path:path(evenodd,'M0 0H100V100H0Z M20 0H80V80H20Z')}"
             '.anchor{margin-left:30pt;width:10pt;height:10pt}', False),
        ]
        body = ('<section><div class=anchor></div></section>'
                '<div class=target><a href="https://example.org/">T</a></div>')
        target = '.target{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-anchor-ink-') as directory:
            root = Path(directory)
            for css, visible in cases:
                with self.subTest(css=css):
                    output = renderer.render(root, PROFILE + css + target, body)
                    glyphs = [glyph for glyph in pdf_content.inspect(output)['glyphs'] if glyph.text == 'T']
                    self.assertEqual(bool(glyphs), visible, 'target text follows ink visibility')
                    self.assertEqual(bool(pdf_content.navigation(output)['links']), visible,
                                     'target links follow ink visibility')
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        positions = [(x, y) for y in range(raster.height) for x in range(raster.width)
                                     if raster.getpixel((x, y)) == (255, 0, 0)]
                    if visible:
                        self.assertEqual((min(x for x, _ in positions), min(y for _, y in positions),
                                          max(x for x, _ in positions) + 1, max(y for _, y in positions) + 1),
                                         (50, 30, 70, 40),
                                         'shadow ink affects visibility while anchor() retains border geometry')
                        self.assertAlmostEqual(glyphs[0].origin[0], 50, places=3)
                    else:
                        self.assertFalse(positions, 'a clip-path-hidden anchor must force-hide target ink')

    def test_filter_support_moves_anchor_ink_through_real_pdf_filters(self):
        cases = [
            ('drop-shadow(-20pt 0 green)', '', True),
            ('blur(8pt)', '', True),
            ('url(#ink)', '<feColorMatrix/>', False),
            ('url(#ink)', "<feFlood x='40' y='26.666667' width='10' height='10' result='band'/><feComposite in='band' in2='band' operator='arithmetic' k1='-1'/>", False),
            ('url(#ink)', "<feFlood x='40' y='26.666667' width='10' height='10' result='band'/><feComposite in='band' in2='band' operator='arithmetic' k1='1' k2='-1'/>", False),
            ('url(#ink)', "<feSpecularLighting lighting-color='black' specularConstant='1'><feDistantLight elevation='90'/></feSpecularLighting>", False),
            ('url(#ink)', "<feDiffuseLighting lighting-color='black' diffuseConstant='1'><feDistantLight elevation='90'/></feDiffuseLighting>", True),
            ('url(#ink)', "<feDiffuseLighting diffuseConstant='0'><feDistantLight elevation='90'/></feDiffuseLighting>", True),
            ('url(#ink)', "<feOffset dx='30'/>", False),
            ('url(#ink)', "<feOffset dx='-26.666667'/>", True),
            ('url(#ink)', "<feGaussianBlur stdDeviation='8'/>", True),
            ('url(#ink)', "<feFlood x='40' y='26.666667' width='10' height='10' flood-color='green'/>", True),
            ('url(#ink)', "<feImage x='40' y='26.666667' width='10' height='10' href='data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2210%22 height=%2210%22%3E%3Crect width=%2210%22 height=%2210%22 fill=%22green%22/%3E%3C/svg%3E'/>", True),
        ]
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-anchor-filters-') as directory:
            root = Path(directory)
            for function, primitive, visible in cases:
                with self.subTest(function=function, primitive=primitive):
                    css = ('section{width:20pt;height:40pt;overflow:hidden}'
                           f'.anchor{{margin-left:30pt;width:10pt;height:10pt;filter:{function}}}'
                           '.target{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}')
                    body = ("<svg style='display:none'><defs><filter id='ink' filterUnits='userSpaceOnUse' "
                            f"x='0' y='0' width='300' height='300'>{primitive}</filter></defs></svg>"
                            '<section><div class=anchor></div></section>'
                            '<div class=target><a href="https://example.org/">T</a></div>')
                    output = renderer.render(root, PROFILE + css, body)
                    glyphs = [glyph for glyph in pdf_content.inspect(output)['glyphs'] if glyph.text == 'T']
                    self.assertEqual(bool(glyphs), visible, 'filter support must govern target text')
                    self.assertEqual(bool(pdf_content.navigation(output)['links']), visible)
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        red = [(x, y) for y in range(raster.height) for x in range(raster.width)
                               if raster.getpixel((x, y)) == (255, 0, 0)]
                        green = [(x, y) for y in range(20, 60) for x in range(20, 40)
                                 if raster.getpixel((x, y))[1] > raster.getpixel((x, y))[0]
                                 and raster.getpixel((x, y))[1] > raster.getpixel((x, y))[2]]
                    self.assertEqual(bool(red), visible, 'filter support must govern serialized target ink')
                    if visible:
                        self.assertEqual((min(x for x, _ in red), min(y for _, y in red),
                                          max(x for x, _ in red) + 1, max(y for _, y in red) + 1),
                                         (50, 30, 70, 40))
                        if 'Flood' in primitive or 'Image' in primitive or 'drop-shadow' in function:
                            self.assertTrue(green, 'the filter must actually generate ink inside the overflow clip')

    def test_structural_table_anchor_boxes_exist_without_background_paints(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        css = ('.anchor{width:auto;height:auto;margin:0;background:transparent}'
               'table{width:100pt;table-layout:fixed;border-spacing:0}'
               'col:first-child{width:40pt}td{height:20pt;padding:0}'
               '.target{left:anchor(left,140pt);top:anchor(bottom,100pt);'
               'width:anchor-size(width,99pt);height:anchor-size(height,9pt);position-visibility:always}')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-table-anchors-') as directory:
            root = Path(directory)
            for collapse in ('separate', 'collapse'):
                for source, (left, top, width, height) in (
                        ('tr', (0, 20, 100, 20)), ('tbody', (0, 40, 100, 40)),
                        ('thead', (0, 40, 100, 40)), ('tfoot', (0, 40, 100, 40)),
                        ('col', (0, 40, 40, 40)), ('colgroup', (0, 40, 100, 40))):
                    with self.subTest(collapse=collapse, source=source):
                        group = source if source in ('thead', 'tfoot') else 'tbody'
                        def attr(tag):
                            return ' class=anchor' if source == tag else ''
                        body = (f"<table style='border-collapse:{collapse}'><colgroup{attr('colgroup')}>"
                                f"<col{attr('col')}><col></colgroup><{group}{attr(group)}>"
                                f"<tr{attr('tr')}><td></td><td></td></tr><tr><td></td><td></td></tr>"
                                f"</{group}></table><div class=target><a href='https://example.org/'>T</a></div>")
                        output = renderer.render(root, PROFILE + css, body)
                        glyphs = pdf_content.inspect(output)['glyphs']
                        links = pdf_content.navigation(output)['links']
                        with renderer.raster(root, output, vector_antialias=False) as raster:
                            actual = raster.tobytes()
                        explicit = (f'.target{{position-anchor:none;left:{left}pt;top:{top}pt;'
                                    f'width:{width}pt;height:{height}pt}}')
                        output = renderer.render(root, PROFILE + css + explicit, body)
                        self.assertEqual(glyphs, pdf_content.inspect(output)['glyphs'])
                        self.assertEqual(links, pdf_content.navigation(output)['links'])
                        with renderer.raster(root, output, vector_antialias=False) as raster:
                            self.assertEqual(actual, raster.tobytes(),
                                             'structural anchor geometry must move pixels, glyphs and links together')

    def test_table_anchor_effects_and_fragmented_source_geometry_reach_pdf(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        base = ('.anchor{width:auto;height:auto;margin:0;background:transparent}'
                'table{width:100pt;border-spacing:0}td{height:20pt;padding:0}'
                '.target{left:anchor(left);top:anchor(bottom);width:anchor-size(width);'
                'position-visibility:anchors-visible}')
        body = ('<section><table><tbody><tr class=anchor><td></td></tr></tbody></table></section>'
                '<div class=target><a href="https://example.org/">T</a></div>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-table-anchor-effects-') as directory:
            root = Path(directory)
            for effect, visible, expected in (
                    ('tbody{transform:translate(30pt,5pt)}tr{position:relative;left:7pt}', True, (57, 45, 157, 55)),
                    ('tbody{clip-path:inset(100%)}', False, None),
                    ('tbody{border:10pt solid;clip-path:padding-box}', True, (20, 40, 120, 50)),
                    ('section{overflow:hidden;width:20pt;height:40pt}'
                     'table{margin-left:30pt;width:10pt}tr{box-shadow:-20pt 0 green}', True, (50, 40, 60, 50)),
                    ('section{overflow:hidden;width:20pt;height:40pt}'
                     'table{margin-left:30pt;width:10pt}tr{filter:drop-shadow(-20pt 0 green)}'
                     'td{background:blue}', True, (50, 40, 60, 50)),
                    ('section{overflow:hidden;width:20pt;height:40pt}'
                     'table{margin-left:30pt;width:10pt}tr{filter:opacity(0)}'
                     'td{background:blue}', False, None)):
                with self.subTest(effect=effect):
                    output = renderer.render(root, PROFILE + base + effect, body)
                    self.assertEqual(bool(pdf_content.navigation(output)['links']), visible)
                    self.assertEqual(any(g.text == 'T' for g in pdf_content.inspect(output)['glyphs']), visible)
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        red = [(x, y) for y in range(raster.height) for x in range(raster.width)
                               if raster.getpixel((x, y)) == (255, 0, 0)]
                        if 'drop-shadow' in effect:
                            self.assertTrue(any(all(abs(a - b) <= 1 for a, b in zip(raster.getpixel((x, y)), (0, 128, 0)))
                                                for y in range(20, 40) for x in range(20, 40)),
                                            'the row filter must generate real green ink inside the clip')
                    bounds = ((min(x for x, y in red), min(y for x, y in red),
                               max(x for x, y in red)+1, max(y for x, y in red)+1) if red else None)
                    self.assertEqual(bounds, expected)
            output = renderer.render(root, PROFILE + base +
                '.anchor{height:20pt;transform:translate(7pt,5pt)}.target{position-visibility:always}',
                '<table><tbody><tr class=anchor></tr></tbody></table><div class=target></div>')
            with renderer.raster(root, output, vector_antialias=False) as raster:
                red = [(x, y) for y in range(raster.height) for x in range(raster.width)
                       if raster.getpixel((x, y)) == (255, 0, 0)]
            self.assertEqual((min(x for x, y in red), min(y for x, y in red),
                              max(x for x, y in red)+1, max(y for x, y in red)+1), (27, 45, 127, 55))
            body = ('<section><table><tbody class=anchor>' + '<tr><td></td></tr>' * 5 +
                    '</tbody></table><div class=target></div></section>')
            base += ('td{height:60pt}.target{left:0;top:0;height:anchor-size(height);'
                     'background:linear-gradient(red,yellow)}')
            for containing, height in (('section{position:relative}', 300), ('', 200)):
                with self.subTest(containing=containing):
                    output = renderer.render(root, PROFILE + base + containing, body)
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        actual = raster.tobytes()
                    explicit = f'.target{{position-anchor:none;width:100pt;height:{height}pt}}'
                    output = renderer.render(root, PROFILE + base + containing + explicit, body)
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        self.assertEqual(actual, raster.tobytes(),
                                         'fragmented table anchor gradient must use the selected source or AABB height')

    def test_zero_height_table_columns_and_groups_keep_geometry(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        css = ('.anchor{width:auto;height:auto;margin:0;background:transparent}'
               'table{width:100pt;border-spacing:0}col:first-child{width:40pt}'
               '.target{left:anchor(left,140pt);top:anchor(bottom,100pt);'
               'width:anchor-size(width,99pt);height:calc(10pt + anchor-size(height,9pt));'
               'position-visibility:always}')
        target = "<div class=target><a href='https://example.org/'>T</a></div>"
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-zero-table-anchors-') as directory:
            root = Path(directory)
            def compare(extra, body, left, top, width, page=1):
                output = renderer.render(root, PROFILE + css + extra, body + target)
                glyphs = pdf_content.inspect(output)['glyphs']
                links = pdf_content.navigation(output)['links']
                if '<thead>' in body:
                    for marker in ('H', 'F'):
                        self.assertEqual(sum(g.text == marker for g in glyphs), 2,
                                         f'empty-group forced break must repeat table {marker}')
                self.assertTrue(any(g.text == 'T' and g.page == page for g in glyphs),
                                'the target must paint on its actual flow page')
                with renderer.raster(root, output, vector_antialias=False, page=page) as raster:
                    actual = raster.tobytes()
                explicit = (f'.target{{position-anchor:none;left:{left}pt;top:{top}pt;'
                            f'width:{width}pt;height:10pt}}')
                output = renderer.render(root, PROFILE + css + extra + explicit, body + target)
                self.assertEqual(glyphs, pdf_content.inspect(output)['glyphs'])
                self.assertEqual(links, pdf_content.navigation(output)['links'])
                with renderer.raster(root, output, vector_antialias=False, page=page) as raster:
                    self.assertEqual(actual, raster.tobytes(),
                                     'zero-height principal boxes remain valid anchor references')
            for layout in ('fixed', 'auto'):
                for collapse in ('separate', 'collapse'):
                    for source, left, width in (('col', 0, 40), ('second', 40, 60),
                            ('colgroup', 0, 100), ('tbody', 0, 100), ('thead', 0, 100), ('tfoot', 0, 100)):
                        with self.subTest(layout=layout, collapse=collapse, source=source):
                            group = source if source in ('thead', 'tfoot') else 'tbody'
                            def attr(tag):
                                return ' class=anchor' if source == tag else ''
                            body = (f"<table style='table-layout:{layout};border-collapse:{collapse}'>"
                                    f"<colgroup{attr('colgroup')}><col{attr('col')}><col{attr('second')}>"
                                    f"</colgroup><{group}{attr(group)}></{group}></table>")
                            compare('', body, left, 0, width)
            cases = (
                ('', '<table><tbody class=anchor></tbody></table>', 0, 0, 100, 1),
                ('', '<table><caption style="height:15pt;padding:0"></caption>'
                 '<colgroup class=anchor><col><col></colgroup></table>', 0, 15, 100, 1),
                ('td{height:20pt;padding:0}', '<table><tbody><tr><td></td></tr></tbody>'
                 '<tbody class=anchor></tbody><tbody><tr><td></td></tr></tbody></table>', 0, 20, 100, 1),
                ('table{height:80pt}', '<table><colgroup><col class=anchor><col></colgroup></table>', 0, 0, 40, 1),
                ('table{direction:rtl}', '<table><colgroup><col class=anchor><col></colgroup></table>', 60, 0, 40, 1),
                ('.anchor{transform:translate(7pt,5pt)}', '<table><tbody class=anchor></tbody></table>', 7, 5, 100, 1),
                ('td{height:20pt;padding:0}.anchor{break-before:page}',
                 '<table><tbody><tr><td>A</td></tr></tbody><tbody class=anchor></tbody>'
                 '<tbody><tr><td>B</td></tr></tbody></table>', 0, 0, 100, 2),
                ('td{height:20pt;padding:0}.anchor{break-after:page}',
                 '<table><tbody><tr><td>A</td></tr></tbody><tbody class=anchor></tbody>'
                 '<tbody><tr><td>B</td></tr></tbody></table>', 0, 20, 100, 2),
                ('td{height:20pt;padding:0}.anchor{break-before:page}'
                 'thead td,tfoot td{height:10pt;font:5pt/10pt monospace}',
                 '<table><thead><tr><td>H</td></tr></thead><tbody><tr><td>A</td></tr></tbody>'
                 '<tbody class=anchor></tbody><tbody><tr><td>B</td></tr></tbody>'
                 '<tfoot><tr><td>F</td></tr></tfoot></table>', 0, 10, 100, 2),
                ('td{height:20pt;padding:0}.anchor{break-before:page}'
                 'thead td,tfoot td{height:10pt;font:5pt/10pt monospace}',
                 '<table><thead><tr><td>H</td></tr></thead><tbody><tr><td>A</td></tr></tbody>'
                 '<tbody class=anchor></tbody><tfoot><tr><td>F</td></tr></tfoot></table>', 0, 10, 100, 2),
                ('td{height:20pt;padding:0}.anchor{break-before:page}'
                 'thead td,tfoot td{height:10pt;font:5pt/10pt monospace}',
                 '<table><thead><tr><td>H</td></tr></thead><tbody><tr><td>A</td></tr></tbody>'
                 '<tbody class=anchor></tbody><tbody></tbody>'
                 '<tfoot><tr><td>F</td></tr></tfoot></table>', 0, 10, 100, 2),
                ('td{height:20pt;padding:0}.next{break-before:page}',
                 '<table><tbody><tr><td></td></tr></tbody><tbody class=anchor></tbody>'
                 '<tbody><tr class=next><td></td></tr></tbody></table>', 0, 20, 100, 2),
            )
            for extra, body, left, top, width, page in cases:
                with self.subTest(extra=extra, body=body):
                    compare(extra, body, left, top, width, page)

    def test_drop_shadow_preserves_midtones_in_each_filter_color_space(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        css = ('@page{size:140pt 100pt;margin:20pt}html,body{margin:0}'
               'div{width:12pt;height:20pt;background:red}')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-shadow-color-') as directory:
            root = Path(directory)
            for space in ('sRGB', 'linearRGB'):
                with self.subTest(color_interpolation_filters=space):
                    body = ("<svg style='display:none'><defs><filter id='shadow' "
                            f"color-interpolation-filters='{space}' x='-100%' y='-100%' width='500%' height='300%'>"
                            "<feDropShadow dx='40' dy='0' stdDeviation='0' flood-color='#408080'/>"
                            "</filter></defs></svg><div style='filter:url(#shadow)'></div>")
                    output = renderer.render(root, css, body)
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        actual = raster.getpixel((55, 25))
                        for channel, expected in zip(actual, (64, 128, 128)):
                            self.assertLessEqual(abs(channel - expected), 1,
                                                 f'{space} must preserve the declared sRGB shadow midtone: {actual}')
                    output = renderer.render(root, css, "<div style='filter:drop-shadow(30pt 0 #408080)'></div>")
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        actual = raster.getpixel((55, 25))
                        for channel, expected in zip(actual, (64, 128, 128)):
                            self.assertLessEqual(abs(channel - expected), 1,
                                                 f'CSS drop-shadow must preserve its declared midtone: {actual}')

    def test_zero_radius_morphology_preserves_filtered_input_ink(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        css = ('section{width:20pt;height:40pt;overflow:hidden}'
               '.anchor{margin-left:30pt;width:10pt;height:10pt;filter:url(#ink)}'
               '.target{left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible}')
        def body(primitive):
            return ("<svg style='display:none'><defs><filter id='ink' filterUnits='userSpaceOnUse' "
                    "x='0' y='0' width='300' height='300'><feOffset dx='-26.666667'/>"
                    f"{primitive}</filter></defs></svg><section><div class=anchor></div></section>"
                    '<div class=target><a href="https://example.org/">T</a></div>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-anchor-morphology-') as directory:
            root = Path(directory)
            output = renderer.render(root, PROFILE + css, body(''))
            with renderer.raster(root, output, vector_antialias=False) as raster:
                expected = raster.tobytes()
                self.assertTrue(any(raster.getpixel((x, y)) == (0, 0, 255)
                                    for y in range(20, 30) for x in range(20, 40)),
                                'the offset reference must actually paint blue ink inside the clip')
            for radius in ('0', '0 2', '2 0', '-1 2', '2 broken', '2 3 broken'):
                with self.subTest(radius=radius):
                    output = renderer.render(root, PROFILE + css,
                                             body(f"<feMorphology operator='dilate' radius='{radius}'/>"))
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        self.assertEqual(raster.tobytes(), expected,
                                         'zero in either radius must return the filter input image')
                    self.assertIn('T', [glyph.text for glyph in pdf_content.inspect(output)['glyphs']])
                    self.assertTrue(pdf_content.navigation(output)['links'])

    def test_conditional_visibility_removes_ink_and_links(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-anchor-hidden-') as directory:
            root = Path(directory)
            for css in ('.anchor{visibility:hidden}.target{position-area:bottom}',
                        '.target{top:215pt;position-visibility:no-overflow}',
                        '.target{position-anchor:--missing;top:anchor(bottom);position-visibility:anchors-valid}'):
                with self.subTest(css=css):
                    output = renderer.render(root, PROFILE + css, BODY)
                    self.assertFalse(pdf_content.navigation(output)['links'], 'hidden anchor targets cannot expose links')
                    self.assertNotIn('T', [glyph.text for glyph in pdf_content.inspect(output)['glyphs']])
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        self.assertNotIn((255, 0, 0), list(raster.getdata()), 'hidden anchor targets cannot paint')


if __name__ == '__main__':
    unittest.main(verbosity=2)
