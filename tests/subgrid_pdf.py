"""Compare subgrid PDF painting with independently positioned reference boxes."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]


class SubgridPdf(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix='ferri-pdf-subgrid-')
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def test_shared_axes_preserve_image_pixels_and_link_bounds(self):
        image = (ROOT / 'tests/oracle/assets/image-pattern.png').as_uri()
        body = ('<section><div class=sub><div class=first>'
                '<a href="mailto:reader@example.org">Link</a></div>'
                f'<img src="{image}"><div class=last></div></div></section>')
        shared = ('.first{background:red}.last{background:blue}'
                  'a{color:black;text-decoration:none}img{object-fit:fill}')
        for effect in ('', 'transform:translate(7pt,9pt);opacity:.5'):
            with self.subTest(effect=effect):
                reference = self.renderer.render(self.root, shared +
                    'section{width:200pt;height:90pt;' + effect + '}'
                    '.sub{position:relative;width:200pt;height:90pt}'
                    '.sub>*{position:absolute}.first{left:0;top:0;width:60pt;height:40pt}'
                    'img{left:70pt;top:0;width:130pt;height:40pt}'
                    '.last{left:0;top:50pt;width:200pt;height:40pt}', body)
                links = pdf_content.navigation(reference)['links']
                self.assertEqual(len(links), 1)
                with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
                    expected = raster.tobytes()
                for axes in ('columns', 'rows', 'both'):
                    with self.subTest(axes=axes):
                        columns = 'subgrid' if axes != 'rows' else '60pt 130pt'
                        rows = 'subgrid' if axes != 'columns' else '40pt 40pt'
                        output = self.renderer.render(self.root, shared +
                            'section{display:grid;width:200pt;grid-template-columns:60pt 130pt;'
                            'grid-template-rows:40pt 40pt;gap:10pt;' + effect + '}'
                            '.sub{display:grid;grid-column:1/-1;grid-row:1/-1;'
                            f'grid-template-columns:{columns};grid-template-rows:{rows};gap:10pt}}'
                            'img{width:100%;height:100%}.last{grid-column:1/-1}', body)
                        self.assertEqual(len(pdf_content.inspect(output)['images']), 1)
                        self.assertEqual(pdf_content.navigation(output)['links'], links)
                        with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
                            self.assertEqual(raster.tobytes(), expected,
                                             f'{axes} subgrid changed image, text or box painting')


    def test_opposite_direction_inherits_physical_tracks_and_reverses_local_lines(self):
        body = '<section><div class=sub><div class=first></div><div class=last></div></div></section>'
        shared = '.first{background:red}.last{background:blue}'
        for parent, child, first_x, first_width, last_x, last_width in (
            ('rtl', 'ltr', 0, 130, 140, 60),
            ('ltr', 'rtl', 70, 130, 0, 60),
        ):
            with self.subTest(parent=parent, child=child):
                reference = self.renderer.render(self.root, shared +
                    '.sub{position:relative;width:200pt;height:40pt}.sub>*{position:absolute;top:0;height:40pt}'
                    f'.first{{left:{first_x}pt;width:{first_width}pt}}'
                    f'.last{{left:{last_x}pt;width:{last_width}pt}}', body)
                with self.renderer.raster(self.root, reference, vector_antialias=False) as raster:
                    expected = raster.tobytes()
                output = self.renderer.render(self.root, shared +
                    'section{display:grid;width:200pt;grid-template-columns:60pt 130pt;'
                    f'grid-template-rows:40pt;gap:10pt;direction:{parent}}}'
                    '.sub{display:grid;grid-column:1/-1;grid-template-columns:subgrid;'
                    f'direction:{child}}}.first{{grid-column:1}}.last{{grid-column:2}}', body)
                with self.renderer.raster(self.root, output, vector_antialias=False) as raster:
                    self.assertEqual(raster.tobytes(), expected,
                                     'opposite directions must preserve physical parent track boundaries')


    def test_shared_rows_fragment_like_explicit_page_content(self):
        items = ''.join(f'<div class="item row{index}">Row {index}</div>' for index in range(4))
        body = '<section><div class=sub>' + items + '</div></section>'
        shared = ('@page{size:240pt 100pt;margin:10pt}.item{height:40pt}'
                  '.row0,.row2{background:red}.row1,.row3{background:blue}')
        reference = self.renderer.render(self.root, shared + 'section{width:100pt}', body)
        self.assertEqual(len(list(pdf_content.Document(reference).pages())), 2)
        expected = []
        for page in (1, 2):
            with self.renderer.raster(self.root, reference, vector_antialias=False, page=page) as raster:
                expected.append(raster.tobytes())
        output = self.renderer.render(self.root, shared +
            'section{display:grid;width:100pt;grid-template-columns:100pt;'
            'grid-template-rows:repeat(4,40pt)}'
            '.sub{display:grid;grid-row:1/-1;grid-template-rows:subgrid}', body)
        self.assertEqual(len(list(pdf_content.Document(output).pages())), 2)
        for page, pixels in enumerate(expected, start=1):
            with self.renderer.raster(self.root, output, vector_antialias=False, page=page) as raster:
                self.assertEqual(raster.tobytes(), pixels,
                                 f'shared row fragmentation changed text or backgrounds on page {page}')


if __name__ == '__main__':
    unittest.main(verbosity=2)
