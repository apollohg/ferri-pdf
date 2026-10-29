"""Motion Path 1 static placement: PDF pixels, glyphs and link activation geometry."""
from pathlib import Path
import tempfile
import unittest

from oracle import pdf_content
import table_positioning_pdf

PROFILE = ('@page{size:300pt 240pt;margin:20pt}html,body{margin:0;padding:0}'
           'section{position:relative;width:200pt;height:100pt}'
           '#box{width:20pt;height:10pt;margin-left:40pt;background:red;font:8pt/10pt monospace}'
           '#after{width:20pt;height:5pt;background:blue}')
BODY = ('<section><div id=box><a href="https://example.org/">T</a></div>'
        '<div id=after></div></section>')


class MotionPathPdf(unittest.TestCase):
    def test_path_distance_preserves_flow_and_moves_glyphs_ink_and_links(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-motion-') as directory:
            root = Path(directory)
            reference = renderer.render(root, PROFILE + '#box{transform:translate(10pt,-5pt)}', BODY)
            glyphs = pdf_content.inspect(reference)['glyphs']
            links = pdf_content.navigation(reference)['links']
            with renderer.raster(root, reference, vector_antialias=False) as raster:
                expected = raster.tobytes()
                self.assertEqual(raster.getpixel((85, 18)), (255, 0, 0))
                self.assertEqual(raster.getpixel((25, 32)), (0, 0, 255))
            actual = renderer.render(root, PROFILE +
                "#box{offset-path:path('M0 0 H160');offset-position:0 0;"
                "offset-distance:50%;offset-anchor:center;offset-rotate:0deg}", BODY)
            self.assertEqual(pdf_content.inspect(actual)['glyphs'], glyphs,
                             'glyph matrices must include the same path transform as box ink')
            self.assertEqual(pdf_content.navigation(actual)['links'], links,
                             'link activation must follow the motion path')
            with renderer.raster(root, actual, vector_antialias=False) as raster:
                self.assertEqual(raster.tobytes(), expected,
                                 'motion must move the red box without moving the following blue box')

    def test_stylesheet_svg_motion_uses_the_svg_viewport_and_own_fill_box_anchor(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        body = '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><circle class="moving" cx="25" cy="35" r="5" fill="red"/></svg>'
        css = PROFILE + 'svg .moving{transform-origin:0 0;transform-box:fill-box;'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-motion-svg-') as directory:
            root = Path(directory)
            reference = renderer.render(root, css + 'transform:translate(60px,10px)}', body)
            with renderer.raster(root, reference, vector_antialias=False) as raster:
                expected = raster.tobytes()
                self.assertEqual(raster.getpixel((84,54)), (255,0,0))
            actual = renderer.render(root, css + "offset-path:path('M0 40 H160');offset-distance:50%;offset-anchor:0 0;offset-rotate:0deg}", body)
            with renderer.raster(root, actual, vector_antialias=False) as raster:
                self.assertEqual(raster.tobytes(), expected, 'stylesheet motion must survive SVG computed-presentation serialization and use actual shape bounds')

    def test_svg_distance_math_preserves_percentages_and_numeric_factors(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        body = '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><circle class="moving" cx="25" cy="35" r="5" fill="red"/></svg>'
        css = PROFILE + 'svg .moving{transform-origin:0 0;transform-box:fill-box;'
        for distance, translate in [('min(40px,50%)',20), ('clamp(10px,50%,60px)',40), ('calc(2 * min(10px,50%))',0)]:
            with self.subTest(distance=distance), tempfile.TemporaryDirectory(prefix='ferri-pdf-motion-math-') as directory:
                root = Path(directory)
                reference = renderer.render(root, css + f'transform:translate({translate}px,10px)' + '}', body)
                with renderer.raster(root, reference, vector_antialias=False) as raster:
                    expected = raster.tobytes()
                actual = renderer.render(root, css + f"offset-path:path('M0 40 H160');offset-distance:{distance};offset-anchor:0 0;offset-rotate:0deg" + '}', body)
                with renderer.raster(root, actual, vector_antialias=False) as raster:
                    self.assertEqual(raster.tobytes(), expected, 'computed SVG motion math must retain the path-length percentage basis')

    def test_svg_image_stylesheet_deferred_shorthand_uses_the_same_path(self):
        from urllib.parse import quote
        renderer = table_positioning_pdf.TablePositioningPdf()
        def body(declarations):
            svg = '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><style>.moving{transform-origin:0 0;transform-box:fill-box;' + declarations + '}</style><circle class="moving" cx="25" cy="35" r="5" fill="red"/></svg>'
            return '<img src="data:image/svg+xml,' + quote(svg, safe='') + '">'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-motion-svg-var-') as directory:
            root = Path(directory)
            reference = renderer.render(root, PROFILE + 'img{display:block}', body('transform:matrix(1,0,0,1,60,10)'))
            with renderer.raster(root, reference, vector_antialias=False) as raster:
                expected = raster.tobytes()
                self.assertEqual(raster.getpixel((84,54)), (255,0,0))
            actual = renderer.render(root, PROFILE + 'img{display:block}', body("--route:path('M0 40 H160');offset:var(--route) 50% 0deg / 0 0"))
            with renderer.raster(root, actual, vector_antialias=False) as raster:
                self.assertEqual(raster.tobytes(), expected, 'SVG image stylesheets retain deferred offset shorthand semantics')

    def test_svg_variables_share_inheritance_importance_fallback_cycles_and_computed_unset(self):
        from urllib.parse import quote
        renderer = table_positioning_pdf.TablePositioningPdf()
        def body(declarations):
            svg = '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><style>g{font:normal 14px serif}' + declarations + '</style><g fill="navy" stroke="black"><rect class="box" x="20" y="30" width="40" height="20"/><text class="caption" x="20" y="75">VARS</text></g></svg>'
            return '<img src="data:image/svg+xml,' + quote(svg, safe='') + '">'
        cases = [
            ('g{--ink:#008000;--stroke:2px;--alpha:0.5}.box{fill:var(--ink);stroke-width:var(--stroke);opacity:var(--alpha)}', '.box{fill:#008000;stroke-width:2px;opacity:0.5}'),
            ('.box{fill:red;fill:var(--missing);opacity:0;opacity:var(--missing)}', '.box{fill:navy;opacity:1}'),
            ('g{--a:var(--b);--b:var(--a)}.box{fill:var(--a,#008000);opacity:var(--b,0.5)}', '.box{fill:#008000;opacity:0.5}'),
            ('.box{--ink:red!important;--ink:blue;fill:var(--ink)}', '.box{fill:red}'),
            ('g{--ink:red!important}.box{--ink:blue;fill:var(--ink)}', '.box{fill:blue}'),
            ('g{fill:#008000!important}.box{fill:inherit;fill:blue}', '.box{fill:blue}'),
            ('.caption{font:normal 10px monospace;font:var(--missing)}', '.caption{font:normal 14px serif}'),
            ('.caption{--font:normal 12px monospace;font:var(--font)}', '.caption{font:normal 12px monospace}'),
            ('.box{--bad:bogus;fill:red;fill:var(--bad)}', '.box{fill:navy}'),
            ('.box{--Ink:#008000;--ink:blue;FiLl:var(--Ink);stroke:var(--ink);stroke-width:2px}', '.box{fill:#008000;stroke:blue;stroke-width:2px}'),
            ('.box{fill:red;fill:var(bad)}', '.box{fill:red}'),
            ('g{--unused:var(bad)}.box{fill:#008000}', '.box{fill:#008000}'),
            ('.box{--bad:var(bad);fill:var(--bad,#008000)}', '.box{fill:#008000}'),
            ('.box{--ink:#008000;--ink:var(bad);fill:var(--ink)}', '.box{fill:#008000}'),
        ]
        for actual_css, reference_css in cases:
            with self.subTest(css=actual_css), tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-vars-') as directory:
                root = Path(directory)
                reference = renderer.render(root, PROFILE + 'img{display:block}', body(reference_css))
                with renderer.raster(root, reference, vector_antialias=False) as raster:
                    expected = raster.tobytes()
                    self.assertGreater(sum(pixel != (255,255,255) for pixel in raster.getdata()), 200, 'independent literal reference must paint the rectangle and text')
                actual = renderer.render(root, PROFILE + 'img{display:block}', body(actual_css))
                with renderer.raster(root, actual, vector_antialias=False) as raster:
                    self.assertEqual(raster.tobytes(), expected, 'all native SVG attributes share the core custom-property cascade semantics')


    def test_svg_motion_wide_values_and_invalid_literals_preserve_cascade(self):
        from urllib.parse import quote
        renderer = table_positioning_pdf.TablePositioningPdf()
        def body(css, inline=''):
            svg = '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><style>g{--ink:navy}.box{transform-box:fill-box;transform-origin:0 0}' + css + '</style><g><rect class="box" x="20" y="30" width="40" height="20" fill="red" style="' + inline + '"/></g></svg>'
            return '<img src="data:image/svg+xml,' + quote(svg, safe='') + '">'
        motion = ".box{offset-path:path('M0 40 H160');offset-distance:50%;offset-anchor:0 0;offset-rotate:0deg}"
        cases = [
            (motion + '.box{offset-distance:broken}', '', '.box{transform:matrix(1,0,0,1,60,10)}'),
            (motion + '.box{offset-distance:var(--missing)}', '', '.box{transform:matrix(1,0,0,1,-20,10)}'),
            ("g{offset-distance:50%;offset-anchor:0 0;offset-rotate:0deg}.box{offset-path:path('M0 40 H160');offset-distance:inherit;offset-anchor:inherit;offset-rotate:inherit}", '', '.box{transform:matrix(1,0,0,1,60,10)}'),
            ('.box{--ink:#008000;fill:var(--ink)}', '--ink:revert-layer', '.box{fill:#008000}'),
            ('.box{--ink:#008000;fill:var(--ink)}', '--ink:var(--missing,revert-layer)', '.box{fill:#008000}'),
            ('.box{--base:#008000;--ink:var(--base);fill:var(--ink)}', '--ink:revert-layer', '.box{fill:#008000}'),
            ('.box{--base:#008000;--ink:var(--base);fill:var(--ink)}', '--ink:var(--missing,revert-layer)', '.box{fill:#008000}'),
            ('.box{--ink:revert;fill:var(--ink)}', '--ink:var(--missing,revert-layer)', '.box{fill:navy}'),
            ('.box{--ink:#008000;fill:var(--ink)}', '--ink:initial', '.box{fill:black}'),
            ('.box{--ink:#008000;fill:var(--ink)}', '--ink:revert', '.box{fill:navy}'),
        ]
        for css, inline, control in cases:
            with self.subTest(css=css, inline=inline), tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-wide-') as directory:
                root = Path(directory)
                reference = renderer.render(root, PROFILE + 'img{display:block}', body(control))
                with renderer.raster(root, reference, vector_antialias=False) as raster:
                    expected = raster.tobytes()
                actual = renderer.render(root, PROFILE + 'img{display:block}', body(css, inline))
                with renderer.raster(root, actual, vector_antialias=False) as raster:
                    self.assertEqual(raster.tobytes(), expected, 'CSS-wide values and invalid declarations retain the native SVG cascade')


    def test_motion_effects_share_clipping_grouping_shadow_and_navigation(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        css = PROFILE + 'section{width:80pt;height:40pt;overflow:hidden;border-radius:8pt;clip-path:inset(0 round 8pt)}#box{opacity:0.6;box-shadow:15pt 0 4pt #008000}'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-motion-effects-') as directory:
            root = Path(directory)
            reference = renderer.render(root, css + '#box{transform:translate(10pt,-5pt)}', BODY)
            links = pdf_content.navigation(reference)['links']
            glyphs = pdf_content.inspect(reference)['glyphs']
            with renderer.raster(root, reference, vector_antialias=False) as raster:
                expected = raster.tobytes()
            actual = renderer.render(root, css + "#box{offset-path:path('M0 0 H160');offset-distance:50%;offset-anchor:center;offset-rotate:0deg}", BODY)
            self.assertEqual(pdf_content.navigation(actual)['links'], links, 'motion links share ancestor clip and opacity groups')
            self.assertEqual(pdf_content.inspect(actual)['glyphs'], glyphs)
            with renderer.raster(root, actual, vector_antialias=False) as raster:
                self.assertEqual(raster.tobytes(), expected, 'box ink, shadow and glyphs share the motion effect through clipping')

    def test_motion_anchor_geometry_and_ink_visibility_follow_the_shared_effect(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        body = '<section><div class="anchor"></div></section><div class="target"><a href="https://example.org/">T</a></div>'
        css = PROFILE + 'section{position:static;width:40pt;height:40pt;overflow:hidden}.anchor{anchor-name:--motion;width:10pt;height:10pt;background:blue}.target{position:absolute;position-anchor:--motion;left:anchor(left);top:anchor(bottom);position-visibility:anchors-visible;width:20pt;height:10pt;background:red;font:8pt/10pt monospace}'
        for shadow, visible in [('', False), ('box-shadow:-50pt 0 green', True)]:
            with self.subTest(shadow=shadow), tempfile.TemporaryDirectory(prefix='ferri-pdf-motion-anchor-') as directory:
                root = Path(directory)
                reference = renderer.render(root, css + '.anchor{transform:translate(60pt,0);' + shadow + '}', body)
                links = pdf_content.navigation(reference)['links']
                glyphs = pdf_content.inspect(reference)['glyphs']
                self.assertEqual(any(glyph.text == 'T' for glyph in glyphs), visible, 'literal transform control establishes ink visibility independently')
                with renderer.raster(root, reference, vector_antialias=False) as raster:
                    expected = raster.tobytes()
                    if visible:
                        self.assertEqual(raster.getpixel((85,35)), (255,0,0), 'anchor() uses the moved border geometry despite shadow-only visibility')
                actual = renderer.render(root, css + ".anchor{offset-path:path('M80 0 H160');offset-anchor:0 0;offset-rotate:0deg;" + shadow + '}', body)
                self.assertEqual(pdf_content.navigation(actual)['links'], links)
                self.assertEqual(pdf_content.inspect(actual)['glyphs'], glyphs)
                with renderer.raster(root, actual, vector_antialias=False) as raster:
                    self.assertEqual(raster.tobytes(), expected, 'motion changes both anchor geometry and separate ink visibility through the common effect chain')


    def test_native_motion_uses_physical_containing_geometry_and_preserves_effects(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        modes = ['vertical-rl', 'vertical-lr', 'sideways-rl', 'sideways-lr']
        body = '<section id="cb"><div id="moving"><a href="https://example.org/native">T</a></div></section>'
        page = '@page{size:300pt 240pt;margin:20pt}html,body{margin:0;padding:0}body{width:240pt;height:180pt}a{color:black;text-decoration:none}'
        box = '#cb{position:relative;width:140pt;height:100pt;margin-left:15pt;margin-top:10pt;background:blue}#moving{position:absolute;left:10pt;top:15pt;writing-mode:horizontal-tb;width:20pt;height:12pt;background:red;font:8pt/10pt monospace;transform-origin:0 0;transform-box:border-box}'
        def bounds(image, colour):
            points = [(x,y) for y in range(image.height) for x in range(image.width) if image.getpixel((x,y)) == colour]
            self.assertTrue(points, f'baseline must paint the independent {colour} geometry')
            return (min(x for x,y in points),min(y for x,y in points),max(x for x,y in points)+1,max(y for x,y in points)+1)
        for mode in modes:
            for kind in ['root', 'orthogonal', 'nested-columns', 'rounded-owner']:
                with self.subTest(mode=mode, kind=kind), tempfile.TemporaryDirectory(prefix='ferri-pdf-native-motion-') as directory:
                    root = Path(directory)
                    css = page + box + (f'html{{writing-mode:{mode}}}' if kind == 'root' else f'#cb{{writing-mode:{mode}}}')
                    content = body
                    if kind == 'nested-columns':
                        css += '.outer{columns:2;column-gap:10pt;width:250pt;height:170pt}#cb{columns:2;column-gap:10pt}#moving{position:relative;left:0;top:0}'
                        content = '<article class="outer">' + body + '</article>'
                    if kind == 'rounded-owner':
                        css += '#cb{padding:3pt 7pt 11pt 13pt;border:2pt solid blue;border-radius:10% 20% 30% 40% / 15% 25% 35% 45%}#moving{box-shadow:6pt 3pt #008000}'
                    baseline = renderer.render(root, css, content)
                    with renderer.raster(root, baseline, vector_antialias=False) as raster:
                        cb = bounds(raster, (0,0,255))
                        own = bounds(raster, (255,0,0))
                    if kind == 'nested-columns':
                        column_height = ((cb[3]-cb[1])-10)/2
                        top = cb[1] + (column_height+10 if mode == 'sideways-lr' else 0)
                        point = (cb[0]+(cb[2]-cb[0])/2+20, top+column_height/2)
                        path = 'ellipse(20pt 10pt at 50% 50%)'
                        distance = '0%'
                    elif kind == 'rounded-owner':
                        padding_width = 140+13+7
                        point = (cb[0]+2+padding_width*0.1, cb[1]+2)
                        path = 'padding-box'
                        distance = '0%'
                    else:
                        point = (cb[0]+90, cb[1]+22.5)
                        path = "path('M40 30 H200')"
                        distance = '50%'
                    dx,dy = point[0]-own[0],point[1]-own[1]
                    reference = renderer.render(root, css+f'#moving{{transform:translate({dx}pt,{dy}pt)}}', content)
                    glyphs = pdf_content.inspect(reference)['glyphs']
                    links = pdf_content.navigation(reference)['links']
                    self.assertTrue(links, 'independent native transform control must retain its live link')
                    with renderer.raster(root, reference, vector_antialias=False) as raster:
                        expected = raster.tobytes()
                        self.assertAlmostEqual(bounds(raster,(255,0,0))[0], round(point[0]), delta=1)
                    actual = renderer.render(root, css+f'#moving{{offset-path:{path};offset-distance:{distance};offset-anchor:0 0;offset-rotate:0deg}}', content)
                    self.assertEqual(pdf_content.navigation(actual)['links'], links, 'native motion links use the actual physical reference geometry')
                    self.assertEqual(pdf_content.inspect(actual)['glyphs'], glyphs, 'native glyphs follow the common physical motion matrix')
                    with renderer.raster(root, actual, vector_antialias=False) as raster:
                        self.assertEqual(raster.tobytes(), expected, f'{mode} {kind}: own/reference geometry, percentage radii and physical shadow must share one basis')


    def test_native_collapsed_cell_motion_reuses_the_used_border_and_zero_radius(self):
        renderer = table_positioning_pdf.TablePositioningPdf()
        body = '<table><tr><td id="cell"><div id="moving"><a href="https://example.org/cell">T</a></div></td></tr></table>'
        page = '@page{size:300pt 240pt;margin:20pt}html,body{margin:0;padding:0}table{border-collapse:collapse}td{position:relative;width:140pt;height:100pt;padding:3pt 7pt 11pt 13pt;border:8pt solid black;border-radius:20pt;background:blue;background-clip:content-box}#moving{position:absolute;left:35pt;top:25pt;writing-mode:horizontal-tb;direction:ltr;width:20pt;height:12pt;background:red;font:8pt/10pt monospace;transform-origin:0 0;transform-box:border-box}a{color:black;text-decoration:none}'
        def corner(image, colour):
            points = [(x,y) for y in range(image.height) for x in range(image.width) if image.getpixel((x,y)) == colour]
            self.assertTrue(points, f'collapsed cell control must paint {colour}')
            return min(x for x,y in points),min(y for x,y in points)
        for mode in ['vertical-rl','vertical-lr','sideways-rl','sideways-lr']:
            for direction in ['ltr','rtl']:
                for area,insets in [('content-box',(0,0)),('padding-box',(13,3)),('border-box',(17,7))]:
                    with self.subTest(mode=mode,direction=direction,area=area), tempfile.TemporaryDirectory(prefix='ferri-pdf-native-cell-motion-') as directory:
                        root = Path(directory)
                        css = page + f'table{{direction:{direction}}}#cell{{writing-mode:{mode}}}'
                        baseline = renderer.render(root,css,body)
                        with renderer.raster(root,baseline,vector_antialias=False) as raster:
                            content = corner(raster,(0,0,255))
                            own = corner(raster,(255,0,0))
                        point = content[0]-insets[0],content[1]-insets[1]
                        reference = renderer.render(root,css+f'#moving{{transform:translate({point[0]-own[0]}pt,{point[1]-own[1]}pt)}}',body)
                        links = pdf_content.navigation(reference)['links']
                        glyphs = pdf_content.inspect(reference)['glyphs']
                        self.assertTrue(links,'literal collapsed-cell control retains its link')
                        with renderer.raster(root,reference,vector_antialias=False) as raster:
                            expected = raster.tobytes()
                        actual = renderer.render(root,css+f'#moving{{offset-path:{area};offset-anchor:0 0;offset-rotate:0deg}}',body)
                        self.assertEqual(pdf_content.navigation(actual)['links'],links,'native cell motion uses conflict-resolved half borders and ignores collapsed radii')
                        self.assertEqual(pdf_content.inspect(actual)['glyphs'],glyphs)
                        with renderer.raster(root,actual,vector_antialias=False) as raster:
                            self.assertEqual(raster.tobytes(),expected,'collapsed reference boxes share the used cell decoration across both directions')


if __name__ == '__main__':
    unittest.main(verbosity=2)
