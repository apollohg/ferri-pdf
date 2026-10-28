"""Compare marker painting with explicitly positioned ordinary image boxes."""
from pathlib import Path
import tempfile
import unittest

from fontTools.ttLib import TTFont
from PIL import Image
import table_positioning_pdf
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
FONT_SIZE = 10
LINE_HEIGHT = 14
PADDING = 40


class ListImagesPdf(unittest.TestCase):
    def test_inside_markers_match_atomic_inline_painting_and_links(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-list-images-') as directory:
            root = Path(directory)
            png = root / 'marker.png'
            Image.new('RGB', (16, 8), (200, 30, 70)).save(png)
            svg = root / 'marker.svg'
            svg.write_text('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 1"><rect width="2" height="1" fill="red"/></svg>')
            renderer = table_positioning_pdf.TablePositioningPdf()
            for image, size in [(f'url("{png.as_uri()}")', (12, 6)),
                                (f'url("{svg.as_uri()}")', (10, 5)),
                                ('linear-gradient(red,blue)', (10, 10))]:
                with self.subTest(image=image):
                    css = 'p,ul,li{margin:0;padding:0}a{color:black}li{list-style:inside none}'
                    link = '<a href="mailto:reader@example.org">Item</a>'
                    reference = renderer.render(root, css +
                        f'.marker{{display:inline-block;width:{size[0]}pt;height:{size[1]}pt;background-image:{image}}}',
                        '<p><span class=marker></span> ' + link + '</p>')
                    expected_links = pdf_content.navigation(reference)['links']
                    with renderer.raster(root, reference, vector_antialias=False) as raster:
                        expected = raster.tobytes()
                    output = renderer.render(root, css + f'li{{list-style-image:{image}}}',
                                             '<ul><li>' + link + '</li></ul>')
                    self.assertEqual(pdf_content.navigation(output)['links'], expected_links)
                    with renderer.raster(root, output, vector_antialias=False) as raster:
                        self.assertEqual(raster.tobytes(), expected)

    def test_outside_markers_follow_baseline_direction_transform_and_opacity(self):
        with TTFont(ROOT / 'tests/oracle/assets/DejaVuSans.ttf') as font:
            space = font['hmtx'].metrics[font.getBestCmap()[ord(' ')]][0] * FONT_SIZE / font['head'].unitsPerEm
            baseline = LINE_HEIGHT / 2 + (font['hhea'].ascent + font['hhea'].descent) * FONT_SIZE / (2 * font['head'].unitsPerEm)
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-list-images-outside-') as directory:
            root = Path(directory)
            renderer = table_positioning_pdf.TablePositioningPdf()
            for direction in ('ltr', 'rtl'):
                for effect in ('', 'transform:translate(3pt,5pt);opacity:.5'):
                    with self.subTest(direction=direction, effect=effect):
                        css = ('p,ul,li{margin:0;padding:0}'
                               f'p,li{{direction:{direction};text-align:start;{effect}}}ul{{padding-left:{PADDING}pt}}')
                        edge = 'right' if direction == 'ltr' else 'left'
                        reference = renderer.render(root, css +
                            f'p{{margin-left:{PADDING}pt;position:relative}}'
                            f'.marker{{position:absolute;{edge}:calc(100% + {space}pt);'
                            f'top:{baseline - FONT_SIZE}pt;width:1em;height:1em;background:linear-gradient(red,blue)}}',
                            '<p><span class=marker></span>Item</p>')
                        with renderer.raster(root, reference, vector_antialias=False) as raster:
                            expected = raster.tobytes()
                        output = renderer.render(root, css + 'li{list-style-image:linear-gradient(red,blue)}',
                                                 '<ul><li>Item</li></ul>')
                        with renderer.raster(root, output, vector_antialias=False) as raster:
                            self.assertEqual(raster.tobytes(), expected)

    def test_stylesheet_relative_images_are_shared_by_repeated_markers(self):
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-list-images-css-') as directory:
            root = Path(directory)
            (root / 'styles').mkdir()
            (root / 'icons').mkdir()
            Image.new('RGB', (16, 8), (200, 30, 70)).save(root / 'icons/marker.png')
            (root / 'styles/list.css').write_text('li{list-style:inside url("../icons/marker.png")}')
            output = table_positioning_pdf.TablePositioningPdf().render(root,
                'ul,li{margin:0;padding:0}',
                '<link rel="stylesheet" href="styles/list.css"><ul><li>One</li><li>Two</li></ul>')
            document = pdf_content.Document(output)
            images = [document.get(number) for number in document.offsets]
            images = [item for item in images if isinstance(item, pdf_content.Stream) and item.entries.get('Subtype') == 'Image']
            self.assertEqual(len(images), 1, 'repeated marker source must embed once')
            self.assertEqual((images[0].entries['Width'], images[0].entries['Height']), (16, 8))
            text = ''.join(glyph.text for glyph in pdf_content.inspect(output)['glyphs'])
            self.assertIn('One', text)
            self.assertIn('Two', text)


if __name__ == '__main__':
    unittest.main(verbosity=2)
