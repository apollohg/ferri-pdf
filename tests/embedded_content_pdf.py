"""Rendered checks for iframes, media elements, canvas and object fallback (HTML §15.4)."""
from html import escape
from pathlib import Path
import tempfile
import unittest

from PIL import Image
import table_positioning_pdf

GREEN = (0, 128, 0)
RED = (255, 0, 0)
WHITE = (255, 255, 255)
# Page area: 260pt square at a 20pt margin.
NESTED_BOXES = ('<style>body{margin:0}div{width:80pt;height:30pt;background:rgb(0,128,0)}'
                '.tall{width:20pt;height:200pt;margin-left:90pt;background:rgb(255,0,0)}</style>'
                '<div></div><div class=tall></div>')


class EmbeddedContentPdf(unittest.TestCase):
    def setUp(self):
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def ink(self, root, css, body):
        output = self.renderer.render(root, css, body)
        with self.renderer.raster(root, output, vector_antialias=False) as raster:
            return raster.tobytes()

    def pixel(self, root, css, body, point):
        output = self.renderer.render(root, css, body)
        with self.renderer.raster(root, output, vector_antialias=False) as raster:
            return raster.getpixel(point)

    def test_iframe_content_is_placed_in_the_content_box_and_clipped_to_it(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-iframe-clip-') as directory:
            root = Path(directory)
            frame = ('iframe{display:block;margin:10pt 0 0 15pt;width:100pt;height:50pt;'
                     'border:0;padding:5pt;background:rgb(0,0,255)}')
            reference = ('.frame{margin:10pt 0 0 15pt;width:100pt;padding:5pt;background:rgb(0,0,255)}'
                         '.viewport{height:50pt;overflow:hidden}'
                         '.viewport div{width:80pt;height:30pt;background:rgb(0,128,0)}'
                         '.viewport .tall{width:20pt;height:200pt;margin-left:90pt;background:rgb(255,0,0)}')
            expected = self.ink(root, reference, '<div class=frame><div class=viewport>'
                                '<div></div><div class=tall></div></div></div>')
            self.assertEqual(
                self.ink(root, frame, f'<iframe srcdoc="{escape(NESTED_BOXES)}"></iframe>'),
                expected,
                'nested content must start at the content edge and be clipped to the viewport, '
                'which is the content box, leaving the padding unpainted by it')

    def test_nested_media_queries_use_the_iframe_viewport_not_the_page(self):
        nested = ('<style>body{margin:0}div{height:20pt;background:rgb(255,0,0)}'
                  '@media (max-width:130pt) and (min-width:130pt){div{background:rgb(0,128,0)}}'
                  '@media (width:260pt){div{background:rgb(0,0,255)}}</style><div></div>')
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-iframe-media-') as directory:
            root = Path(directory)
            self.assertEqual(
                self.pixel(root, 'iframe{display:block;border:0;width:50%;height:40pt}',
                           f'<iframe srcdoc="{escape(nested)}"></iframe>', (30, 30)),
                GREEN, 'a 50% iframe of the 260pt page area has a 130pt viewport')

    def test_video_posters_fit_like_contained_images(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-video-poster-') as directory:
            root = Path(directory)
            poster = root / 'poster.png'
            Image.new('RGB', (40, 20), GREEN).save(poster)
            size = 'width:120pt;height:90pt;display:block'
            expected = self.ink(root, f'img{{{size};object-fit:contain}}', f'<img src="{poster.as_uri()}">')
            self.assertEqual(
                self.ink(root, f'video{{{size}}}', f'<video poster="{poster.as_uri()}" controls></video>'),
                expected,
                'the poster frame is centred at the largest size that fits, with no controls painted')
            self.assertEqual(
                self.ink(root, 'video{object-position:0 0;' + size + '}',
                         f'<video poster="{poster.as_uri()}"></video>'),
                self.ink(root, f'img{{{size};object-fit:contain;object-position:0 0}}',
                         f'<img src="{poster.as_uri()}">'),
                'object-position moves the poster like any replaced content')

    def test_canvas_and_object_fallback_render_like_their_children(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-fallback-') as directory:
            root = Path(directory)
            css = 'b{color:rgb(0,128,0)}'
            expected = self.ink(root, css, '<p>a <span>fall<b>back</b></span> z</p>')
            for markup in ('<canvas>fall<b>back</b></canvas>',
                           '<object data=missing.png><object data="data:,text"'
                           ' type=application/x-unknown>fall<b>back</b></object></object>'):
                with self.subTest(markup=markup):
                    self.assertEqual(self.ink(root, css, f'<p>a {markup} z</p>'), expected)

    def test_srcset_selects_the_densest_source_at_its_density_corrected_size(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-srcset-') as directory:
            root = Path(directory)
            small, large = root / 'small.png', root / 'large.png'
            Image.new('RGB', (40, 20), RED).save(small)
            Image.new('RGB', (160, 80), GREEN).save(large)
            expected = self.ink(root, 'img{display:block;width:60pt}', f'<img src="{large.as_uri()}">')
            for markup in (f'<img srcset="{small.as_uri()} 1x, {large.as_uri()} 2x">',
                           f'<img srcset="{small.as_uri()} 40w, {large.as_uri()} 160w" sizes="80px">',
                           f'<picture><source media="(min-width:10000pt)" srcset="{small.as_uri()}">'
                           f'<source srcset="{large.as_uri()} 2x"><img src="{small.as_uri()}"></picture>'):
                with self.subTest(markup=markup):
                    self.assertEqual(self.ink(root, 'img{display:block}', markup), expected,
                                     'the 160px image at density 2 has an 80px (60pt) natural width')


if __name__ == '__main__':
    unittest.main()
