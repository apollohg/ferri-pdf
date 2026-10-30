"""Native PDF comparisons for zoom across HTML and embedded drawing coordinates."""
from pathlib import Path
import tempfile
import unittest

import embedded_content_pdf


class ZoomPdf(unittest.TestCase):
    def test_zoomed_ruby_and_initial_letters_match_physical_typography(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('ruby', '<ruby>AB<rt>cd</rt>EF<rt>gh</rt></ruby>', 'rt{font-size:50%;color:red}'),
            ('inter-character ruby', '<ruby>AB<rt>cd</rt>EF<rt>gh</rt></ruby>',
             'rt{ruby-position:inter-character;font-size:50%;color:red}'),
            ('initial letter', '<p>Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda.</p>',
             '.sample p::first-letter{initial-letter:3;color:red}'),
            ('first-line initial letter', '<p>Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda.</p>',
             '.sample p::first-line{color:blue}.sample p::first-letter{initial-letter:2;color:red}'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-zoom-used-typography-') as directory:
            root = Path(directory)
            for name, contents, rules in cases:
                with self.subTest(name=name):
                    body = f'<div class="sample">{contents}</div><p>After</p>'
                    expected = renderer.ink(root, rules + 'p{margin:0}.sample{font-size:20pt;line-height:24pt;width:240pt}', body)
                    self.assertTrue(any(r > g and r > b for r, g, b in zip(expected[::3], expected[1::3], expected[2::3])),
                                    'reference annotation or initial letter must paint red glyphs')
                    actual = renderer.ink(root, rules + 'p{margin:0}.sample{zoom:2;font-size:10pt;line-height:12pt;width:120pt}', body)
                    self.assertTrue(actual == expected,
                        'anonymous typography must keep layout adjustments without repeating zoom projection')

    def test_svg_text_position_lists_resolve_complete_calculations(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        font = embedded_content_pdf.table_positioning_pdf.ROOT / 'tests/oracle/assets/DejaVuSans.ttf'
        cases = [
            ('x="calc(2px + 3px), calc(10% + 5px)" y="20 20"', 'x="10 30" y="40 40"'),
            ('x="min(5px, 10px) max(5px, 15px)" y="20,20"', 'x="10 30" y="40 40"'),
            ('x="calc(1 + 4) calc(10 + 5)" y="20 20"', 'x="10 30" y="40 40"'),
            ('x="5 15" y="20" dx="calc(1px + 1px) calc(-1px - 1px)" '
             'dy="calc(1px + 1px), calc(-1px - 1px)"', 'x="10 30" y="40" dx="4 -4" dy="4 -4"'),
            ('x="5 invalid 15" y="20"', 'y="40"'),
            ('x="5,,15" y="20"', 'y="40"'),
            ('x="5," y="20"', 'y="40"'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-position-list-') as directory:
            root = Path(directory)
            def ink(name, attributes, style):
                path = root / name
                path.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">'
                    f'<style>@font-face{{font-family:Probe;src:url("{font.as_uri()}")}}</style>'
                    f'<text font-family="Probe" {attributes} style="{style}">AB</text></svg>')
                return renderer.ink(root, 'img{display:block}', f'<img src="{path.as_uri()}">')
            for source, physical in cases:
                with self.subTest(positions=source):
                    actual = ink('actual.svg', source, 'font-size:10px;zoom:2')
                    expected = ink('reference.svg', physical, 'font-size:20px')
                    self.assertTrue(actual == expected,
                                    'position lists must preserve whole math functions and reject invalid lists without retaining a prefix')

    def test_inherited_svg_dash_lists_use_consumer_zoom_and_preserve_percentages(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('4 2', '8 4'),
            ('5% 2px', '10 4'),
            ('1em .5em', '20 10'),
            ('calc(2px + 2px), calc(1px + 1px)', '8 4'),
            ('calc(5% + 2px) 2', '14 4'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-dash-list-') as directory:
            root = Path(directory)
            def ink(name, body):
                path = root / name
                path.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">'
                                + body + '</svg>')
                return renderer.ink(root, 'img{display:block}', f'<img src="{path.as_uri()}">')
            for source, physical in cases:
                for inheritance in ('', 'stroke-dasharray:inherit;stroke-width:inherit',
                                    'stroke-dasharray:unset;stroke-width:unset'):
                    with self.subTest(dashes=source, inheritance=inheritance):
                        actual = ink('actual.svg',
                            f'<g font-size="10" stroke="green" stroke-width=".2em" stroke-dasharray="{source}">'
                            f'<g style="font-size:30px;{inheritance}">'
                            f'<path style="zoom:2;font-size:40px;{inheritance}" d="M5 10H80"/></g></g>')
                        expected = ink('reference.svg',
                            f'<path stroke="green" stroke-width="4" stroke-dasharray="{physical}" d="M10 20H160"/>')
                        self.assertTrue(actual == expected,
                                        'inherited dash lengths must use the declaring font, consuming zoom, and unchanged percentage basis')

    def test_zoomed_svg_backgrounds_and_masks_keep_the_source_viewport(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-layer-zoom-') as directory:
            root = Path(directory)
            content = '<rect x="5" y="5" width="30" height="10" fill="green"/>'
            plain, viewbox = root / 'plain.svg', root / 'viewbox.svg'
            for path, attribute in [(plain, ''), (viewbox, 'viewBox="0 0 80 40"')]:
                path.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40" {attribute}>{content}</svg>')
            for property in ('background', 'mask'):
                with self.subTest(property=property):
                    common = f'background-color:blue;{property}-repeat:no-repeat'
                    expected = renderer.ink(root,
                                             f'div{{width:200pt;height:100pt;{common};{property}-image:url({viewbox.as_uri()});{property}-size:120pt 60pt}}',
                                             '<div></div>')
                    actual = renderer.ink(root,
                                           f'div{{zoom:2;width:100pt;height:50pt;{common};{property}-image:url({plain.as_uri()})}}',
                                           '<div></div>')
                    self.assertEqual(actual, expected,
                                     'zoom must scale SVG layer coordinates as well as the tile rectangle')

    def test_svg_descendant_zoom_scales_lengths_without_scaling_percentages(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-descendant-zoom-') as directory:
            root = Path(directory)
            for outer_zoom in (1, 2):
                for width, expected_width in (("20px", "40px"), ("25%", "25%")):
                    with self.subTest(outer_zoom=outer_zoom, width=width):
                        css = f'svg{{display:block;zoom:{outer_zoom}}}'
                        expected = renderer.ink(root, css,
                            '<svg width="120" height="80"><rect x="10" y="10" '
                            f'width="{expected_width}" height="20" fill="green"/></svg>')
                        actual = renderer.ink(root, css,
                            '<svg width="120" height="80"><g style="zoom:2">'
                            f'<rect x="5" y="5" width="{width}" height="10" fill="green"/>'
                            '</g></svg>')
                        self.assertEqual(actual, expected,
                                         'descendant zoom must project absolute SVG lengths once and preserve percentages')

    def test_svg_zoom_scales_paths_and_strokes_together(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        paths = [
            'M5 5h20v10h-20z',
            'M5 20q10 -20 20 0t20 0',
            'M5 20a10 5 30 0 1 20 0l5 10z',
            'M5 5h20v10z M30 20l10 0 0 10z',
            'M5 5h20v10z invalid',
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-path-zoom-') as directory:
            root = Path(directory)
            for path in paths:
                for outer_zoom in (1, 2):
                    with self.subTest(path=path, outer_zoom=outer_zoom):
                        css = f'svg{{display:block;zoom:{outer_zoom}}}'
                        shape = f'<path d="{path}" fill="green" stroke="blue" stroke-width="1"/>'
                        expected = renderer.ink(root, css,
                            f'<svg width="120" height="80"><g transform="scale(2)">{shape}</g></svg>')
                        actual = renderer.ink(root, css,
                            f'<svg width="120" height="80"><g style="zoom:2">{shape}</g></svg>')
                        self.assertEqual(actual, expected,
                                         'path coordinates, arcs, valid prefixes and stroke lengths must scale together')

    def test_svg_zoom_scales_text_position_lists(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('x="5 20" y="20" dx="1 2" dy="1 0"',
             'x="10 40" y="40" dx="2 4" dy="2 0"'),
            ('x="5% 20%" y=".8em" dx=".1em .2em"',
             'x="5% 20%" y=".8em" dx=".1em .2em"'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-text-zoom-') as directory:
            root = Path(directory)
            for attributes, expected_attributes in cases:
                with self.subTest(attributes=attributes):
                    expected = renderer.ink(root, 'svg{display:block}',
                        '<svg width="160" height="100">'
                        f'<text {expected_attributes} font-size="20">AB</text></svg>')
                    actual = renderer.ink(root, 'svg{display:block}',
                        '<svg width="160" height="100"><g style="zoom:2">'
                        f'<text {attributes} font-size="10">AB</text></g></svg>')
                    self.assertEqual(actual, expected,
                                     'SVG text positions must scale absolute lengths, preserving percentages and em bases')

    def test_svg_zoom_scales_lines_polygons_and_text_length(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('<line x1="5" y1="5" x2="35" y2="25" stroke="blue"/>',
             '<line x1="10" y1="10" x2="70" y2="50" stroke="blue" stroke-width="2"/>'),
            ('<line x1="5%" y1="5%" x2="35%" y2="25%" stroke="blue"/>',
             '<line x1="5%" y1="5%" x2="35%" y2="25%" stroke="blue" stroke-width="2"/>'),
            ('<polygon points="5,5 30,5 20,25" fill="green"/>',
             '<polygon points="10,10 60,10 40,50" fill="green"/>'),
            ('<polyline points="5,5 30,5 20,25" fill="none" stroke="blue"/>',
             '<polyline points="10,10 60,10 40,50" fill="none" stroke="blue" stroke-width="2"/>'),
            ('<text x="5" y="20" textLength="40" lengthAdjust="spacingAndGlyphs" font-size="10">ABCD</text>',
             '<text x="10" y="40" textLength="80" lengthAdjust="spacingAndGlyphs" font-size="20">ABCD</text>'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-attributes-zoom-') as directory:
            root = Path(directory)
            for shape, reference in cases:
                for outer_zoom in (1, 2):
                    with self.subTest(shape=shape, outer_zoom=outer_zoom):
                        css = f'svg{{display:block;zoom:{outer_zoom}}}'
                        expected = renderer.ink(root, css,
                            f'<svg width="160" height="100">{reference}</svg>')
                        actual = renderer.ink(root, css,
                            f'<svg width="160" height="100"><g style="zoom:2">{shape}</g></svg>')
                        self.assertEqual(actual, expected,
                                         'SVG attributes must project geometry and text constraints consistently')

    def test_svg_zoom_scales_text_path_offsets(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-text-path-zoom-') as directory:
            root = Path(directory)
            for offset, expected_offset in (("10", "20"), ("10%", "10%"), ("1em", "1em")):
                with self.subTest(offset=offset):
                    expected = renderer.ink(root, 'svg{display:block}',
                        '<svg width="300" height="100"><defs><path id="curve" d="M0 40H240"/></defs>'
                        f'<text font-size="20"><textPath href="#curve" startOffset="{expected_offset}">AB</textPath></text></svg>')
                    actual = renderer.ink(root, 'svg{display:block}',
                        '<svg width="300" height="100"><g style="zoom:2"><defs><path id="curve" d="M0 20H120"/></defs>'
                        f'<text font-size="10"><textPath href="#curve" startOffset="{offset}">AB</textPath></text></g></svg>')
                    self.assertEqual(actual, expected,
                                     'textPath offsets must use zoomed path geometry and font size without scaling percentages twice')

    def test_standalone_svg_zoom_uses_the_css_cascade(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('g{zoom:2}', '<g>{shape}</g>', 2),
            ('g{--factor:200%;zoom:var(--factor)}', '<g>{shape}</g>', 2),
            ('g{zoom:calc(1 + 1);zoom:-2}', '<g>{shape}</g>', 2),
            ('.outer{zoom:2}.inner{zoom:inherit}', '<g class="outer"><g class="inner">{shape}</g></g>', 4),
            ('.outer{zoom:2}.inner{zoom:.5}', '<g class="outer"><g class="inner">{shape}</g></g>', 1),
            ('g{zoom:0}', '<g>{shape}</g>', 1),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-standalone-svg-zoom-') as directory:
            root = Path(directory)
            for css, wrapper, factor in cases:
                with self.subTest(css=css):
                    shape = '<rect x="5" y="5" width="20" height="10" fill="green"/>'
                    source = root / 'zoom.svg'
                    source.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                                      f'<style>{css}</style>{wrapper.format(shape=shape)}</svg>')
                    expected = renderer.ink(root, 'svg{display:block}',
                        '<svg width="160" height="100">'
                        f'<rect x="{5 * factor}" y="{5 * factor}" width="{20 * factor}" height="{10 * factor}" fill="green"/></svg>')
                    actual = renderer.ink(root, 'img{display:block}', f'<img src="{source.as_uri()}">')
                    self.assertEqual(actual, expected,
                                     'standalone SVG zoom must honor variables, calc, invalid declarations and explicit inheritance')

    def test_standalone_svg_zoom_scales_inherited_strokes_and_fonts(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-standalone-svg-lengths-') as directory:
            root = Path(directory)
            source = root / 'zoom.svg'
            font = embedded_content_pdf.table_positioning_pdf.ROOT / 'tests/oracle/assets/DejaVuSans.ttf'
            source.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                f'<style>@font-face{{font-family:Probe;src:url("{font.as_uri()}")}}</style>'
                '<g font-family="Probe" stroke="blue" stroke-width="2" font-size="10"><g style="zoom:2">'
                '<rect x="5" y="5" width="25%" height="10" fill="green"/>'
                '<text x="5" y="40" stroke="none">AB</text></g></g></svg>')
            expected = renderer.ink(root, 'svg{display:block}',
                '<svg width="160" height="100"><rect x="10" y="10" width="25%" height="20" '
                'fill="green" stroke="blue" stroke-width="4"/>'
                '<text x="10" y="80" font-family="Probe" font-size="20">AB</text></svg>')
            actual = renderer.ink(root, 'img{display:block}', f'<img src="{source.as_uri()}">')
            self.assertEqual(actual, expected,
                             'inherited absolute strokes and fonts use the consuming element zoom; percentages retain their basis')

    def test_svg_calculated_fonts_and_baseline_shifts_match_physical_text(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        font = embedded_content_pdf.table_positioning_pdf.ROOT / 'tests/oracle/assets/DejaVuSans.ttf'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-baseline-zoom-') as directory:
            root = Path(directory)
            expected = renderer.ink(root, 'svg{display:block}',
                '<svg width="160" height="100"><text x="10" y="70" '
                'font-family="Probe" font-size="20">AB</text></svg>')
            for shift in ('5px', 'calc(2px + 3px)', 'calc(25% + 2.5px)'):
                with self.subTest(shift=shift):
                    source = root / 'text.svg'
                    source.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                        f'<style>@font-face{{font-family:Probe;src:url("{font.as_uri()}")}}</style>'
                        '<text x="5" y="40" font-family="Probe" style="font-size:calc(5px + 5px);zoom:2">'
                        f'<tspan baseline-shift="{shift}">AB</tspan></text></svg>')
                    actual = renderer.ink(root, 'img{display:block}', f'<img src="{source.as_uri()}">')
                    self.assertTrue(actual == expected,
                                    'calculated font metrics and baseline shifts must use the same zoomed coordinate space')

    def test_svg_calculated_text_path_offsets_use_path_length_for_percentages(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        font = embedded_content_pdf.table_positioning_pdf.ROOT / 'tests/oracle/assets/DejaVuSans.ttf'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-textpath-calc-') as directory:
            root = Path(directory)
            reference = root / 'reference.svg'
            reference.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                f'<style>@font-face{{font-family:Probe;src:url("{font.as_uri()}")}}</style>'
                '<defs><path id="p" d="M0 80H160"/></defs><text font-family="Probe" font-size="20">'
                '<textPath href="#p" startOffset="20">AB</textPath></text></svg>')
            expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
            for offset in ('10px', 'calc(5px + 5px)', 'calc(10% + 2px)'):
                with self.subTest(offset=offset):
                    source = root / 'text.svg'
                    source.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                        f'<style>@font-face{{font-family:Probe;src:url("{font.as_uri()}")}}</style>'
                        '<defs><path id="p" d="M0 80H160"/></defs>'
                        '<text font-family="Probe" font-size="10" style="zoom:2">'
                        f'<textPath href="#p" startOffset="{offset}">AB</textPath></text></svg>')
                    actual = renderer.ink(root, 'img{display:block}', f'<img src="{source.as_uri()}">')
                    self.assertTrue(actual == expected,
                                    'text-path percentages must use path length while absolute terms use text zoom')

    def test_standalone_svg_paths_and_points_match_inline_zoom(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        shapes = [
            '<path d="M5 5h20v10h-20z" fill="green" stroke="blue"/>',
            '<path d="M5 20q10 -20 20 0t20 0" fill="none" stroke="blue"/>',
            '<polygon points="5,5 30,5 20,25" fill="green"/>',
            '<polyline points="5,5 30,5 20,25" fill="none" stroke="blue"/>',
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-standalone-svg-paths-') as directory:
            root = Path(directory)
            for shape in shapes:
                with self.subTest(shape=shape):
                    body = f'<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100"><g style="zoom:2">{shape}</g></svg>'
                    source = root / 'zoom.svg'
                    source.write_text(body)
                    expected = renderer.ink(root, 'svg{display:block}', body)
                    actual = renderer.ink(root, 'img{display:block}', f'<img src="{source.as_uri()}">')
                    self.assertEqual(actual, expected,
                                     'standalone and document-styled SVG must agree on path and point zoom')

    def test_standalone_svg_zoom_scales_transform_lengths(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('transform="translate(10 5) scale(1.5)"', ''),
            ('transform="rotate(30 10 10)"', ''),
            ('', 'transform:translate(10px,5px) rotate(30deg)'),
            ('transform="rotate(30)"', 'transform-origin:10px 5px'),
            ('transform="rotate(30)"', 'transform-origin:50% 50%;transform-box:fill-box'),
            ('', 'translate:10px 5px;rotate:30deg;scale:1.5;transform-origin:0 0'),
            ('', 'translate:1em .5em;rotate:30deg;transform-origin:0 0'),
            ('', 'offset-path:path("M0 0L100 0");offset-distance:20px;offset-anchor:0 0;offset-rotate:0deg'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-standalone-svg-transforms-') as directory:
            root = Path(directory)
            for attributes, css in cases:
                with self.subTest(attributes=attributes, css=css):
                    body = ('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                            f'''<g style="zoom:2;font-size:10px"><rect {attributes} style='{css}' '''
                            'x="5" y="5" width="20" height="10" fill="green"/></g></svg>')
                    source = root / 'zoom.svg'
                    source.write_text(body)
                    expected = renderer.ink(root, 'svg{display:block}', body)
                    actual = renderer.ink(root, 'img{display:block}', f'<img src="{source.as_uri()}">')
                    self.assertEqual(actual, expected,
                                     'standalone transforms must match inline zoom for lengths, origins, angles and factors')

    def test_svg_zoom_resolves_filter_primitives_in_target_coordinates(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('userSpaceOnUse', '<feOffset dx="5" dy="2"/>', '<feOffset dx="10" dy="4"/>'),
            ('objectBoundingBox', '<feOffset dx=".1" dy=".2"/>', '<feOffset dx=".1" dy=".2"/>'),
            ('userSpaceOnUse', '<feGaussianBlur stdDeviation="1 2"/>', '<feGaussianBlur stdDeviation="2 4"/>'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-filter-zoom-') as directory:
            root = Path(directory)
            for units, primitive, reference in cases:
                for external in (False, True):
                    with self.subTest(units=units, primitive=primitive, external=external):
                        def content(effect, shape):
                            return ('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                                    f'<defs><filter id="f" primitiveUnits="{units}" x="-50%" y="-50%" width="200%" height="200%">{effect}</filter></defs>{shape}</svg>')
                        reference_file = root / 'reference.svg'
                        reference_file.write_text(content(reference,
                            '<rect x="40" y="40" width="40" height="20" fill="green" filter="url(#f)"/>'))
                        expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference_file.as_uri()}">')
                        body = content(primitive,
                            '<rect style="zoom:2" x="20" y="20" width="20" height="10" fill="green" filter="url(#f)"/>')
                        if external:
                            source = root / 'filter.svg'
                            source.write_text(body)
                            body = f'<img src="{source.as_uri()}">'
                        actual = renderer.ink(root, 'svg,img{display:block}', body)
                        self.assertEqual(actual, expected,
                                         'user-space filter lengths use target zoom; object-bounding-box fractions do not')

    def test_svg_filter_cache_distinguishes_target_zoom(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-filter-cache-') as directory:
            root = Path(directory)
            defs = ('<defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="160" height="100">'
                    '<feOffset dx="10" dy="0"/></filter></defs>')
            shapes = ('<rect x="20" y="20" width="40" height="20" fill="green" filter="url(#f)"/>'
                      '<rect style="zoom:2" x="10" y="10" width="20" height="10" fill="blue" filter="url(#f)"/>')
            reference = root / 'reference.svg'
            reference.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">' + defs +
                '<defs><filter id="second" filterUnits="userSpaceOnUse" x="0" y="0" width="320" height="200">'
                '<feOffset dx="20" dy="0"/></filter></defs>' +
                shapes.split('<rect style=')[0] +
                '<rect x="20" y="20" width="40" height="20" fill="blue" filter="url(#second)"/></svg>')
            expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
            body = '<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">' + defs + shapes + '</svg>'
            for external in (False, True):
                with self.subTest(external=external):
                    source = root / 'actual.svg'
                    source.write_text(body)
                    actual = renderer.ink(root, 'svg,img{display:block}', f'<img src="{source.as_uri()}">' if external else body)
                    self.assertEqual(actual, expected,
                                     'reusing one user-space filter must not reuse geometry resolved at another target zoom')

    def test_svg_filter_regions_use_target_font_metrics(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-filter-font-') as directory:
            root = Path(directory)
            source = root / 'actual.svg'
            source.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                '<defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="4em" height="100">'
                '<feOffset dx="0"/></filter></defs>'
                '<rect font-size="10" x="20" y="20" width="40" height="20" fill="green" filter="url(#f)"/>'
                '<rect font-size="20" x="20" y="20" width="40" height="20" fill="blue" filter="url(#f)"/></svg>')
            reference = root / 'reference.svg'
            reference.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                '<defs><filter id="f" filterUnits="userSpaceOnUse" x="0" y="0" width="80" height="100">'
                '<feOffset dx="0"/></filter></defs>'
                '<rect x="20" y="20" width="40" height="20" fill="blue" filter="url(#f)"/></svg>')
            expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
            for external in (False, True):
                with self.subTest(external=external):
                    body = f'<img src="{source.as_uri()}">' if external else source.read_text()
                    actual = renderer.ink(root, 'svg,img{display:block}', body)
                    self.assertEqual(actual, expected,
                                     'filter-region em units and cache identity must use each target font, even at equal zoom and bounds')

    def test_svg_gradients_use_target_zoom_and_preserve_bounding_box_units(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('linearGradient', 'gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="40" y2="0"',
             'gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="80" y2="0"'),
            ('radialGradient', 'gradientUnits="userSpaceOnUse" cx="20" cy="20" r="20"',
             'gradientUnits="userSpaceOnUse" cx="40" cy="40" r="40"'),
            ('linearGradient', 'x1="0" x2="1"', 'x1="0" x2="1"'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-gradient-zoom-') as directory:
            root = Path(directory)
            for tag, attributes, reference_attributes in cases:
                for external in (False, True):
                    with self.subTest(tag=tag, attributes=attributes, external=external):
                        def content(attrs, shapes):
                            return ('<svg xmlns="http://www.w3.org/2000/svg" width="120" height="100">'
                                f'<defs><{tag} id="paint" {attrs}><stop stop-color="red"/>'
                                f'<stop offset="1" stop-color="blue"/></{tag}></defs>{shapes}</svg>')
                        plain = '<rect width="80" height="80" fill="url(#paint)"/>'
                        reference = root / 'reference.svg'
                        reference.write_text(content(reference_attributes, plain))
                        expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
                        body = content(attributes, plain + '<rect style="zoom:2" width="40" height="40" fill="url(#paint)"/>')
                        source = root / 'actual.svg'
                        source.write_text(body)
                        actual = renderer.ink(root, 'svg,img{display:block}', f'<img src="{source.as_uri()}">' if external else body)
                        self.assertEqual(actual, expected,
                                         'gradient coordinates and cached variants must use target zoom; radial focus defaults must not scale twice')

    def test_svg_pattern_tiles_use_target_coordinates_and_keep_content_units(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('userSpaceOnUse', 'x="2" y="3" width="10" height="10"',
             'x="4" y="6" width="20" height="20"', '', '5'),
            ('userSpaceOnUse', 'width="10" height="10"',
             'width="20" height="20"', 'viewBox="0 0 10 10"', '5'),
            ('objectBoundingBox', 'width=".25" height=".25"',
             'width=".25" height=".25"', 'patternContentUnits="objectBoundingBox"', '.125'),
            ('objectBoundingBox', 'width=".25" height=".25"',
             'width=".25" height=".25"', 'viewBox="0 0 10 10"', '5'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-pattern-zoom-') as directory:
            root = Path(directory)
            for units, attrs, expected_attrs, content_attrs, size in cases:
                for external in (False, True):
                    with self.subTest(units=units, attrs=attrs, content=content_attrs, external=external):
                        def pattern(identifier, geometry):
                            return (f'<pattern id="{identifier}" patternUnits="{units}" {geometry} {content_attrs}>'
                                    f'<rect width="{size}" height="{size}" fill="green"/></pattern>')
                        def svg(defs, shapes):
                            return ('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                                    f'<defs>{defs}</defs>{shapes}</svg>')
                        first = '<rect width="40" height="40" fill="url(#p)"/>'
                        reference = root / 'reference.svg'
                        reference.write_text(svg(pattern('p', attrs) + pattern('second', expected_attrs), first +
                            '<rect x="60" width="80" height="80" fill="url(#second)"/>'))
                        expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
                        body = svg(pattern('p', attrs), first +
                            '<rect style="zoom:2" x="30" width="40" height="40" fill="url(#p)"/>')
                        source = root / 'actual.svg'
                        source.write_text(body)
                        actual = renderer.ink(root, 'svg,img{display:block}',
                                              f'<img src="{source.as_uri()}">' if external else body)
                        self.assertEqual(actual, expected,
                            'pattern tile lengths use target zoom; content units and viewBox retain their own coordinate mapping')

    def test_svg_mask_regions_use_target_zoom_and_font_metrics(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('userSpaceOnUse', 'x="5" y="5" width="10" height="10"',
             'x="5" y="5" width="10" height="10"', 'x="10" y="10" width="20" height="20"'),
            ('userSpaceOnUse', 'x="0" y="0" width="2em" height="2em"',
             'x="0" y="0" width="20" height="20"', 'x="0" y="0" width="80" height="80"'),
            ('objectBoundingBox', 'x=".1" y=".1" width=".5" height=".5"',
             'x=".1" y=".1" width=".5" height=".5"', 'x=".1" y=".1" width=".5" height=".5"'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-mask-zoom-') as directory:
            root = Path(directory)
            for units, attrs, first_attrs, second_attrs in cases:
                for external in (False, True):
                    with self.subTest(units=units, attrs=attrs, external=external):
                        def mask(identifier, geometry):
                            return (f'<mask id="{identifier}" maskUnits="{units}" {geometry}>'
                                    '<rect width="200" height="200" fill="white"/></mask>')
                        def svg(defs, shapes):
                            return ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">'
                                    f'<defs>{defs}</defs>{shapes}</svg>')
                        first = '<rect style="font-size:10px" width="80" height="80" fill="red" mask="url(#m)"/>'
                        reference = root / 'reference.svg'
                        reference.write_text(svg(mask('m', first_attrs) + mask('second', second_attrs), first +
                            '<rect width="80" height="80" fill="blue" mask="url(#second)"/>'))
                        expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
                        body = svg(mask('m', attrs), first +
                            '<rect style="zoom:2;font-size:20px" width="40" height="40" fill="blue" mask="url(#m)"/>')
                        source = root / 'actual.svg'
                        source.write_text(body)
                        actual = renderer.ink(root, 'svg,img{display:block}',
                                              f'<img src="{source.as_uri()}">' if external else body)
                        self.assertEqual(actual, expected,
                            'one mask on equal-sized targets must resolve each region with target zoom and font metrics')

    def test_svg_clip_cache_distinguishes_percentage_viewports(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-clip-viewport-') as directory:
            root = Path(directory)
            def svg(defs, second):
                return ('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                        f'<defs>{defs}</defs>'
                        '<svg width="40" height="40"><rect width="40" height="40" fill="green" clip-path="url(#c)"/></svg>'
                        f'<svg x="60" width="80" height="80"><rect width="80" height="80" fill="blue" clip-path="url(#{second})"/></svg></svg>')
            reference = root / 'reference.svg'
            reference.write_text(svg(
                '<clipPath id="c"><rect width="20" height="40"/></clipPath>'
                '<clipPath id="second"><rect width="40" height="80"/></clipPath>', 'second'))
            expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
            body = svg('<clipPath id="c"><rect width="50%" height="100%"/></clipPath>', 'c')
            for external in (False, True):
                with self.subTest(external=external):
                    source = root / 'actual.svg'
                    source.write_text(body)
                    actual = renderer.ink(root, 'svg,img{display:block}',
                                          f'<img src="{source.as_uri()}">' if external else body)
                    self.assertEqual(actual, expected,
                        'cached clip geometry must preserve each referencing SVG viewport percentage basis')

    def test_svg_filter_light_positions_use_target_coordinates(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        lights = [
            ('userSpaceOnUse', '<fePointLight x="15" y="25" z="30"/>',
             '<fePointLight x="30" y="50" z="60"/>'),
            ('userSpaceOnUse', '<feSpotLight x="15" y="25" z="30" pointsAtX="20" pointsAtY="30" pointsAtZ="0" specularExponent="2" limitingConeAngle="60"/>',
             '<feSpotLight x="30" y="50" z="60" pointsAtX="40" pointsAtY="60" pointsAtZ="0" specularExponent="2" limitingConeAngle="60"/>'),
            ('objectBoundingBox', '<fePointLight x=".25" y=".5" z="1"/>',
             '<fePointLight x="30" y="60" z="40"/>'),
            ('objectBoundingBox', '<feSpotLight x=".25" y=".5" z="1" pointsAtX=".5" pointsAtY=".5" pointsAtZ="0" specularExponent="2" limitingConeAngle="60"/>',
             '<feSpotLight x="30" y="60" z="40" pointsAtX="40" pointsAtY="60" pointsAtZ="0" specularExponent="2" limitingConeAngle="60"/>'),
            ('userSpaceOnUse', '<feDistantLight azimuth="30" elevation="45"/>',
             '<feDistantLight azimuth="30" elevation="45"/>'),
        ]
        cases = []
        for tag in ('feDiffuseLighting', 'feSpecularLighting'):
            for units, light, reference in lights:
                cases.append((units, f'<{tag} surfaceScale="3">{light}</{tag}>',
                              f'<{tag} surfaceScale="3">{reference}</{tag}>'))
        for kind in ('turbulence', 'fractalNoise'):
            primitive = f'<feTurbulence baseFrequency=".1 .2" numOctaves="2" seed="3" type="{kind}"/>'
            cases.append(('userSpaceOnUse', primitive, primitive))
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-light-zoom-') as directory:
            root = Path(directory)
            for units, primitive, reference_primitive in cases:
                for external in (False, True):
                    with self.subTest(units=units, primitive=primitive, external=external):
                        def svg(effect, primitive_units, shape):
                            return ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">'
                                    f'<defs><filter id="f" primitiveUnits="{primitive_units}">{effect}</filter></defs>{shape}</svg>')
                        reference = root / 'reference.svg'
                        reference.write_text(svg(reference_primitive, 'userSpaceOnUse',
                            '<rect x="20" y="40" width="40" height="40" fill="green" filter="url(#f)"/>'))
                        expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
                        body = svg(primitive, units,
                            '<rect style="zoom:2" x="10" y="20" width="20" height="20" fill="green" filter="url(#f)"/>')
                        source = root / 'actual.svg'
                        source.write_text(body)
                        actual = renderer.ink(root, 'svg,img{display:block}',
                                              f'<img src="{source.as_uri()}">' if external else body)
                        self.assertEqual(actual, expected,
                            'light positions follow target coordinates; surfaceScale, angles, exponents and turbulence frequencies do not acquire zoom')

    def test_svg_marker_definition_zoom_preserves_viewbox_mapping(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-marker-zoom-') as directory:
            root = Path(directory)
            for units in ('userSpaceOnUse', 'strokeWidth'):
                for viewbox in (False, True):
                    for external in (False, True):
                        with self.subTest(units=units, viewbox=viewbox, external=external):
                            def svg(zoomed):
                                size, child, refx, refy = (10, 5, 2, 3) if zoomed else (20, 10, 4, 6)
                                view = f'viewBox="0 0 {size} {size}"' if viewbox else ''
                                style = 'style="zoom:2"' if zoomed else ''
                                return ('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                                        f'<defs><marker id="m" {style} markerUnits="{units}" markerWidth="{size}" markerHeight="{size}" '
                                        f'refX="{refx}" refY="{refy}" {view} orient="auto">'
                                        f'<rect width="{child}" height="{child}" fill="green"/></marker></defs>'
                                        '<path d="M20 30H70" fill="none" stroke="black" stroke-width="2" marker-start="url(#m)" marker-end="url(#m)"/></svg>')
                            reference = root / 'reference.svg'
                            reference.write_text(svg(False))
                            expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
                            body = svg(True)
                            source = root / 'actual.svg'
                            source.write_text(body)
                            actual = renderer.ink(root, 'svg,img{display:block}',
                                                  f'<img src="{source.as_uri()}">' if external else body)
                            self.assertEqual(actual, expected,
                                'marker zoom must project its viewport, reference point, viewBox and children in the same coordinate system')

    def test_svg_zoomed_viewboxes_preserve_absolute_and_percentage_geometry(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-viewbox-zoom-') as directory:
            root = Path(directory)
            for context in ('root', 'root-natural', 'nested', 'symbol'):
                for percent in (False, True):
                    for external in ((True,) if context.startswith('root') else (False, True)):
                        with self.subTest(context=context, percent=percent, external=external):
                            def svg(zoomed):
                                factor = 1 if zoomed else 2
                                style = 'style="zoom:2"' if zoomed else ''
                                view = f'viewBox="{5 * factor} {10 * factor} {60 * factor} {50 * factor}"'
                                width = '25%' if percent else str(20 * factor)
                                child = f'<rect x="{10 * factor}" y="{15 * factor}" width="{width}" height="{10 * factor}" fill="green"/>'
                                dimensions = f'width="{60 * factor}" height="{50 * factor}"'
                                if context.startswith('root'):
                                    sheet = ''
                                    if context == 'root-natural' and zoomed:
                                        style = ''
                                        sheet = '<style>svg{--root-zoom:200%;zoom:var(--root-zoom)}</style>'
                                    return f'<svg xmlns="http://www.w3.org/2000/svg" {style} {dimensions} {view}>{sheet}{child}</svg>'
                                outer = '<svg xmlns="http://www.w3.org/2000/svg" width="160" height="120">'
                                position = f'x="{5 * factor}" y="{5 * factor}"'
                                if context == 'nested':
                                    return f'{outer}<svg {style} {position} {dimensions} {view}>{child}</svg></svg>'
                                return (f'{outer}<defs><symbol id="s" {view}>{child}</symbol></defs>'
                                        f'<use href="#s" {style} {position} {dimensions}/></svg>')
                            reference = root / 'reference.svg'
                            reference.write_text(svg(False))
                            css = 'svg,img{display:block}' + ('img{width:120px;height:100px}' if context == 'root' else '')
                            expected = renderer.ink(root, css, f'<img src="{reference.as_uri()}">')
                            body = svg(True)
                            source = root / 'actual.svg'
                            source.write_text(body)
                            actual = renderer.ink(root, css, f'<img src="{source.as_uri()}">' if external else body)
                            self.assertEqual(actual, expected,
                                'viewBox origins, dimensions and descendant lengths must share zoom; percentages use the resulting viewport')

    def test_svg_pattern_definition_zoom_preserves_viewbox_mapping(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-pattern-viewbox-zoom-') as directory:
            root = Path(directory)
            def svg(zoomed):
                factor = 1 if zoomed else 2
                style = 'style="zoom:2"' if zoomed else ''
                return ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">'
                        f'<defs><pattern id="p" {style} patternUnits="userSpaceOnUse" width="20" height="20" '
                        f'viewBox="{2 * factor} {3 * factor} {10 * factor} {10 * factor}">'
                        f'<rect x="{2 * factor}" y="{3 * factor}" width="{5 * factor}" height="{5 * factor}" fill="green"/>'
                        '</pattern></defs><rect width="80" height="80" fill="url(#p)"/></svg>')
            reference = root / 'reference.svg'
            reference.write_text(svg(False))
            expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
            for external in (False, True):
                with self.subTest(external=external):
                    body = svg(True)
                    source = root / 'actual.svg'
                    source.write_text(body)
                    actual = renderer.ink(root, 'svg,img{display:block}',
                                          f'<img src="{source.as_uri()}">' if external else body)
                    self.assertEqual(actual, expected,
                        'pattern definition zoom must keep its viewBox and child coordinates in the same space')

    def test_svg_resource_percentages_use_the_resource_viewbox(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-resource-percent-') as directory:
            root = Path(directory)
            for kind in ('marker', 'pattern', 'symbol'):
                for zoom in (1, 2):
                    for external in (False, True):
                        with self.subTest(kind=kind, zoom=zoom, external=external):
                            def svg(percent):
                                size = '50%' if percent else '5'
                                child = f'<rect width="{size}" height="{size}" fill="green"/>'
                                style = f'style="zoom:{zoom}"'
                                if kind == 'marker':
                                    defs = f'<marker id="r" {style} markerUnits="userSpaceOnUse" markerWidth="20" markerHeight="20" viewBox="0 0 10 10">{child}</marker>'
                                    shape = '<path d="M10 20H40" stroke="black" marker-end="url(#r)"/>'
                                elif kind == 'pattern':
                                    defs = f'<pattern id="r" {style} patternUnits="userSpaceOnUse" width="20" height="20" viewBox="0 0 10 10">{child}</pattern>'
                                    shape = '<rect width="80" height="80" fill="url(#r)"/>'
                                else:
                                    defs = f'<symbol id="r" viewBox="0 0 10 10">{child}</symbol>'
                                    shape = f'<use href="#r" {style} width="40" height="40"/>'
                                return ('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="100">'
                                        f'<defs>{defs}</defs>{shape}</svg>')
                            reference = root / 'reference.svg'
                            reference.write_text(svg(False))
                            expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
                            body = svg(True)
                            source = root / 'actual.svg'
                            source.write_text(body)
                            actual = renderer.ink(root, 'svg,img{display:block}',
                                                  f'<img src="{source.as_uri()}">' if external else body)
                            self.assertEqual(actual, expected,
                                '50% of a ten-unit resource viewBox must equal five local units, including under zoom')

    def test_referenced_pattern_viewbox_uses_the_consuming_definition_zoom(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-pattern-reference-zoom-') as directory:
            root = Path(directory)
            for source_zoom in (0.5, 3):
                for external in (False, True):
                    with self.subTest(source_zoom=source_zoom, external=external):
                        def svg(inherited):
                            view = 'viewBox="1 2 10 10"'
                            reference = 'href="#base"' if inherited else view
                            return ('<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">'
                                    f'<defs><pattern id="base" style="zoom:{source_zoom}" {view}/>'
                                    f'<pattern id="p" {reference} style="zoom:2" patternUnits="userSpaceOnUse" width="20" height="20">'
                                    '<rect x="1" y="2" width="4" height="3" fill="green"/></pattern></defs>'
                                    '<rect width="80" height="80" fill="url(#p)"/></svg>')
                        reference = root / 'reference.svg'
                        reference.write_text(svg(False))
                        expected = renderer.ink(root, 'img{display:block}', f'<img src="{reference.as_uri()}">')
                        body = svg(True)
                        source = root / 'actual.svg'
                        source.write_text(body)
                        actual = renderer.ink(root, 'svg,img{display:block}',
                                              f'<img src="{source.as_uri()}">' if external else body)
                        self.assertEqual(actual, expected,
                            'referencing a viewBox copies its value, without adopting the source pattern zoom')

    def test_external_motion_paths_resolve_their_selected_font(self):
        from fontTools.ttLib import TTFont
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        font_path = embedded_content_pdf.table_positioning_pdf.ROOT / 'assets/fonts/DejaVuSans.ttf'
        with TTFont(font_path) as font:
            x_height = font['glyf']['x'].yMax / font['head'].unitsPerEm
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-external-motion-fonts-') as directory:
            root = Path(directory)
            for font_size in [10, None]:
                width = 8 * (font_size or 16) * x_height
                font_attribute = f'font-size="{font_size}"' if font_size is not None else ''
                for svg_consumer in [False, True]:
                    for expression in ['8ex', 'calc(8ex)']:
                        with self.subTest(svg_consumer=svg_consumer, expression=expression, font_size=font_size):
                            def ink(name, x2):
                                route = root / (name + '-route.svg')
                                route.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" font-family="MotionFace" {font_attribute}>'
                                    f'<line id="route" x1="0" y1="20" x2="{x2}" y2="20"/></svg>')
                                motion = f'offset-path:url("{route.as_uri()}#route");offset-distance:100%;offset-anchor:0 0'
                                css = f'@font-face{{font-family:MotionFace;src:url("{font_path.as_uri()}")}}'
                                if svg_consumer:
                                    path = root / (name + '.svg')
                                    path.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">'
                                        f'''<rect width="10" height="10" fill="green" style='{motion};font-size:30px'/></svg>''')
                                    return renderer.ink(root, css + 'img{display:block}', f'<img src="{path.as_uri()}">')
                                return renderer.ink(root, css + f'div{{width:10px;height:10px;background:green;font-size:30px;{motion}}}', '<div></div>')
                            expected = ink('reference', width)
                            self.assertIn(bytes((0, 128, 0)), expected, 'reference motion target must paint')
                            self.assertEqual(ink('actual', expression), expected,
                                'external SVG path geometry must use its selected font for either consumer')

    def test_svg_transforms_and_motion_use_current_and_root_font_metrics(self):
        from fontTools.ttLib import TTFont
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        assets = embedded_content_pdf.table_positioning_pdf.ROOT / 'assets/fonts'
        def x_height(filename):
            with TTFont(assets / filename) as font:
                return font['glyf']['x'].yMax / font['head'].unitsPerEm
        current_ex = 10 * x_height('DejaVuSans.ttf')
        root_ex = 20 * x_height('DejaVuSerif.ttf')
        cases = [
            ('translate:1rem 0', 'translate:20px 0'),
            ('transform:translateX(calc(1rem + 2px))', 'transform:translateX(22px)'),
            ('translate:8ex 0', f'translate:{8 * current_ex}px 0'),
            ('translate:calc(2rex) 0', f'translate:{2 * root_ex}px 0'),
            ('rotate:90deg;transform-origin:1rem 0', 'rotate:90deg;transform-origin:20px 0'),
            ("offset-path:path('M0 20H100');offset-anchor:0 0;offset-distance:1rem",
             "offset-path:path('M0 20H100');offset-anchor:0 0;offset-distance:20px"),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-motion-fonts-') as directory:
            root = Path(directory)
            for zoom in [1, 2]:
                for actual_style, reference_style in cases:
                    with self.subTest(zoom=zoom, style=actual_style):
                        def ink(name, style):
                            path = root / name
                            path.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" '
                                'font-family="DejaVu Serif" font-size="20">'
                                f'<rect x="30" y="10" width="10" height="10" fill="green" style="font-family:DejaVu Sans;font-size:10px;zoom:{zoom};{style}"/></svg>')
                            return renderer.ink(root, 'img{display:block}', f'<img src="{path.as_uri()}">')
                        expected = ink('reference.svg', reference_style)
                        self.assertIn(bytes((0, 128, 0)), expected, 'reference geometry must remain visible')
                        self.assertEqual(ink('actual.svg', actual_style), expected,
                            'SVG transforms and motion must resolve lengths in their declaring font context')

    def test_svg_font_units_follow_variable_font_optical_sizing(self):
        from fontTools.ttLib import TTFont
        from fontTools.varLib.instancer import instantiateVariableFont
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        font_path = embedded_content_pdf.table_positioning_pdf.ROOT / 'tests/fixtures/fonts/RobotoFlex.ttf'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-optical-units-') as directory:
            root = Path(directory)
            for sizing, variation, optical in [('auto', '', 40), ('none', '', 14), ('auto', "font-variation-settings:'opsz' 8", 8)]:
                with TTFont(font_path) as font:
                    instance = instantiateVariableFont(font, {'opsz': optical}, inplace=False)
                    glyph = instance.getBestCmap()[ord('0')]
                    width = 8 * 40 * instance['hmtx'][glyph][0] / instance['head'].unitsPerEm
                for operation in ['width', 'translate']:
                    for expression in ['8ch', 'calc(8ch)']:
                        with self.subTest(sizing=sizing, variation=variation, expression=expression, operation=operation):
                            def ink(name, length):
                                path = root / name
                                geometry = length if operation == 'width' else '10'
                                translation = f'translate:{length} 0;' if operation == 'translate' else ''
                                path.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="300" height="100">'
                                    f'<rect width="{geometry}" height="20" fill="green" style="font-family:Probe;font-size:40px;font-optical-sizing:{sizing};{translation}{variation}"/></svg>')
                                css = f'@font-face{{font-family:Probe;src:url("{font_path.as_uri()}")}}img{{display:block}}'
                                return renderer.ink(root, css, f'<img src="{path.as_uri()}">')
                            self.assertEqual(ink('actual.svg', expression), ink('reference.svg', f'{width}px'),
                                'metric lengths must use automatic or explicit optical axes like rendered glyphs')

    def test_svg_font_metrics_control_shapes_and_natural_image_size(self):
        from fontTools.ttLib import TTFont
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        assets = embedded_content_pdf.table_positioning_pdf.ROOT / 'assets/fonts'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-font-metrics-') as directory:
            root = Path(directory)
            for family, filename in [('DejaVu Sans', 'DejaVuSans.ttf'), ('DejaVu Serif', 'DejaVuSerif.ttf')]:
                with TTFont(assets / filename) as font:
                    x_height = font['glyf']['x'].yMax / font['head'].unitsPerEm
                for natural in [False, True]:
                    for expression in ['8ex', 'calc(8ex)', 'calc(4ex + 4ex)']:
                        with self.subTest(family=family, natural=natural, expression=expression):
                            width = 8 * 10 * 2 * x_height
                            style = f'font-family:{family};font-size:10px;zoom:2'
                            reference = root / 'reference.svg'
                            actual = root / 'actual.svg'
                            opening = '<svg xmlns="http://www.w3.org/2000/svg" '
                            if natural:
                                reference.write_text(opening + f'width="{width}" height="20">'
                                    '<rect width="100%" height="100%" fill="green"/></svg>')
                                actual.write_text(opening + f'width="{expression}" height="10" style="{style}">'
                                    '<rect width="100%" height="100%" fill="green"/></svg>')
                            else:
                                reference.write_text(opening + f'width="200" height="100"><rect width="{width}" height="20" fill="green"/></svg>')
                                actual.write_text(opening + f'width="200" height="100"><rect width="{expression}" height="10" style="{style}" fill="green"/></svg>')
                            def ink(path):
                                return renderer.ink(root, 'img{display:block}', f'<img src="{path.as_uri()}"><p>AFTER</p>')
                            self.assertEqual(ink(actual), ink(reference),
                                'selected font metrics must determine both geometry and following HTML flow')

    def test_svg_natural_css_and_font_dimensions_control_image_layout(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ("width='4em' height='2em' style='font-size:10px;zoom:2'", ''),
            ("width='4em' height='2em' style='font-size:calc(5px + 5px);zoom:2'", ''),
            ("width='2em' height='1em' style='font-size:calc(1rem + 4px);zoom:2'", ''),
            ("style='width:calc(2em + 20px);height:calc(1em + 10px);font-size:10px;zoom:2'", ''),
            ("width='10' height='10'", 'svg{width:40px;height:20px;zoom:2}'),
            ("width='10' height='10'", 'svg{--unit:4em;width:var(--unit);height:2em;font-size:10px;zoom:200%}'),
            ("width='4em' viewBox='0 0 40 20' style='font-size:10px;zoom:2'", ''),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-natural-css-') as directory:
            root = Path(directory)
            def svg(attrs, css):
                return (f'<svg xmlns="http://www.w3.org/2000/svg" {attrs}><style>{css}</style>'
                        '<rect width="100%" height="100%" fill="green"/></svg>')
            reference = root / 'reference.svg'
            reference.write_text(svg('width="80" height="40"', ''))
            def document(path):
                return f'<img src="{path.as_uri()}"><p>AFTER</p>'
            expected = renderer.ink(root, 'img{display:block}', document(reference))
            for attrs, css in cases:
                with self.subTest(attrs=attrs, css=css):
                    source = root / 'actual.svg'
                    source.write_text(svg(attrs, css))
                    actual = renderer.ink(root, 'img{display:block}', document(source))
                    self.assertEqual(actual, expected,
                        'natural dimensions must size both the SVG artwork and the following HTML flow consistently')

    def test_svg_calculated_geometry_preserves_percentage_bases_under_zoom(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('<rect style="zoom:2;width:calc(50% + 10px);height:calc(25% + 5px)" '
             'x="calc(5px + 5px)" y="calc(2px + 3px)" rx="calc(1px + 2px)" fill="green"/>',
             '<rect x="20" y="10" width="120" height="35" rx="6" fill="green"/>'),
            ('<g font-size="20"><rect style="font-size:calc(50% + 5px);zoom:2" '
             'width="2em" height="1em" fill="green"/></g>',
             '<rect width="60" height="30" fill="green"/>'),
            ('<g font-size="20"><g style="font-size:calc(50% + 5px)">'
             '<rect style="font-size:inherit;zoom:2" width="2em" height="1em" fill="green"/></g></g>',
             '<rect width="60" height="30" fill="green"/>'),
            ('<style>svg{font-size:20px}</style><g font-size="40">'
             '<rect style="font-size:calc(1rem + 5px);zoom:2" width="2em" height="1em" fill="green"/></g>',
             '<rect width="100" height="50" fill="green"/>'),
            ('<g font-size="40"><rect style="font-size:medium;zoom:2" '
             'width="2em" height="1em" fill="green"/></g>',
             '<rect width="64" height="32" fill="green"/>'),
            ('<g stroke="green" stroke-width="calc(1px + 1px)"><path style="zoom:2" '
             'd="M10 10H60"/></g>',
             '<path stroke="green" stroke-width="4" d="M20 20H120"/>'),
            ('<image href="image.svg" style="zoom:2" width="calc(20px + 20px)"/>',
             '<rect width="80" height="40" fill="green"/>'),
            ('<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x2="calc(20px + 20px)">'
             '<stop stop-color="green"/><stop offset="1" stop-color="blue"/></linearGradient></defs>'
             '<rect style="zoom:2" width="100" height="50" fill="url(#g)"/>',
             '<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x2="80">'
             '<stop stop-color="green"/><stop offset="1" stop-color="blue"/></linearGradient></defs>'
             '<rect width="200" height="100" fill="url(#g)"/>'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-calculated-') as directory:
            root = Path(directory)
            (root / 'image.svg').write_text('<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">'
                                           '<rect width="20" height="10" fill="green"/></svg>')
            def render(name, content):
                source = root / name
                source.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">'
                                  + content + '</svg>')
                return renderer.ink(root, 'img{display:block}', f'<img src="{source.as_uri()}">')
            for actual, expected in cases:
                with self.subTest(source=actual):
                    self.assertTrue(render('actual.svg', actual) == render('reference.svg', expected),
                                    'calculated SVG lengths must scale absolute terms once and keep percentage bases')

    def test_native_math_zoom_scales_fraction_radical_and_scripts(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        body = ('<math><mrow><mfrac><mi>x</mi><mi>y</mi></mfrac>'
                '<msqrt><mi>z</mi></msqrt><msubsup><mi>a</mi><mn>1</mn><mn>2</mn></msubsup>'
                '</mrow></math>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-math-zoom-') as directory:
            root = Path(directory)
            expected = renderer.ink(root,
                                     'math{font-size:20pt;line-height:normal}mfrac{padding-inline:2px}', body)
            actual = renderer.ink(root,
                                   'math{zoom:2;font-size:10pt;line-height:normal}mfrac{padding-inline:1px}', body)
            self.assertEqual(actual, expected,
                             'native MathML glyphs, rules, radicals and script positioning must scale together')

    def test_inline_svg_zoom_scales_geometry_with_and_without_a_viewbox(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        content = ('<rect x="5" y="5" width="30" height="10" fill="green"/>'
                   '<text x="5" y="30" font-size="10">Svg</text>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-svg-zoom-') as directory:
            root = Path(directory)
            for viewbox in ('', 'viewBox="0 0 80 40"'):
                with self.subTest(viewbox=viewbox):
                    reference = content if viewbox else f'<g transform="scale(2)">{content}</g>'
                    expected = renderer.ink(root, 'svg{display:block}',
                                             f'<svg width="160" height="80" {viewbox}>{reference}</svg>')
                    actual = renderer.ink(root, 'svg{display:block;zoom:2}',
                                           f'<svg width="80" height="40" {viewbox}>{content}</svg>')
                    self.assertEqual(actual, expected,
                                     'SVG user coordinates and glyphs must scale once with outer zoom')

    def test_mathml_length_attributes_scale_with_zoom(self):
        renderer = embedded_content_pdf.EmbeddedContentPdf()
        renderer.setUp()
        cases = [
            ('<mfrac linethickness="2pt"><mi>x</mi><mi>y</mi></mfrac>',
             '<mfrac linethickness="4pt"><mi>x</mi><mi>y</mi></mfrac>'),
            ('<mpadded height="20pt" depth="2pt" lspace="3pt" voffset="1pt"><mi>x</mi></mpadded>',
             '<mpadded height="40pt" depth="4pt" lspace="6pt" voffset="2pt"><mi>x</mi></mpadded>'),
            ('<mo lspace="3pt" rspace="2pt">+</mo><mi>x</mi>',
             '<mo lspace="6pt" rspace="4pt">+</mo><mi>x</mi>'),
            ('<mpadded height="150%" lspace=".3em" voffset=".1em"><mi>x</mi></mpadded>',
             '<mpadded height="150%" lspace=".3em" voffset=".1em"><mi>x</mi></mpadded>'),
        ]
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-math-attribute-zoom-') as directory:
            root = Path(directory)
            for actual_body, expected_body in cases:
                with self.subTest(math=actual_body):
                    expected = renderer.ink(root,
                                             'math{font-size:20pt;line-height:normal}mfrac{padding-inline:2px}',
                                             f'<math>{expected_body}</math>')
                    actual = renderer.ink(root,
                                           'math{zoom:2;font-size:10pt;line-height:normal}mfrac{padding-inline:1px}',
                                           f'<math>{actual_body}</math>')
                    self.assertEqual(actual, expected,
                                     'MathML length attributes must use the same zoom projection as CSS lengths')


if __name__ == '__main__':
    unittest.main()
