"""Stacking contexts and containing blocks from will-change and view-transition-name."""
from pathlib import Path
import tempfile
import unittest

import table_positioning_pdf

BOXES = ('div{width:60pt;height:40pt}.a{background:red}'
         '.b{background:blue;margin:-20pt 0 0 20pt}')
NESTED = ('div{width:60pt;height:40pt}.a{background:red}'
          '.c{position:relative;z-index:-1;background:blue;margin-left:20pt;height:20pt}')
CONTAINED = ('.a{margin:30pt 0 0 50pt;width:80pt;height:60pt;background:yellow}'
             '.c{position:absolute;top:5pt;left:5pt;width:10pt;height:10pt;background:blue}'
             '.f{position:fixed;top:20pt;left:20pt;width:10pt;height:10pt;background:green}')


class WillChangePdf(unittest.TestCase):
    def setUp(self):
        self.renderer = table_positioning_pdf.TablePositioningPdf()

    def ink(self, root, css, body):
        output = self.renderer.render(root, css, body)
        with self.renderer.raster(root, output, vector_antialias=False) as raster:
            return raster.tobytes()

    def assert_ink(self, root, css, body, reference, message):
        self.assertEqual(self.ink(root, css, body), self.ink(root, reference, body), message)

    def test_hints_and_view_transition_names_stack_like_opacity(self):
        body = '<div class=a></div><div class=b></div>'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-will-change-stacking-') as directory:
            root = Path(directory)
            flow = self.ink(root, BOXES, body)
            layered = self.ink(root, BOXES + '.a{position:relative}', body)
            self.assertNotEqual(flow, layered, 'the probe must distinguish the paint layers')
            for declaration in ('will-change:opacity', 'will-change:transform',
                                'will-change:background,mask', 'will-change:position',
                                'view-transition-name:hero'):
                with self.subTest(declaration=declaration):
                    self.assertEqual(self.ink(root, BOXES + f'.a{{{declaration}}}', body), layered,
                                     f'{declaration}: the earlier box paints above later in-flow content')
            for declaration in ('will-change:top', 'will-change:z-index', 'will-change:--custom',
                                'will-change:scroll-position,contents', 'view-transition-name:none'):
                with self.subTest(declaration=declaration):
                    self.assertEqual(self.ink(root, BOXES + f'.a{{{declaration}}}', body), flow,
                                     f'{declaration}: no stacking context on an unpositioned block')

    def test_stacking_contexts_hold_negative_z_index_descendants(self):
        body = '<div class=a><div class=c></div></div>'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-will-change-z-') as directory:
            root = Path(directory)
            escaped = self.ink(root, NESTED + '.a{position:relative}', body)
            contained = self.ink(root, NESTED + '.a{position:relative;z-index:0}', body)
            self.assertNotEqual(escaped, contained, 'the probe must distinguish stacking contexts')
            for declaration in ('position:relative;will-change:z-index', 'will-change:opacity',
                                'view-transition-name:match-element'):
                with self.subTest(declaration=declaration):
                    self.assertEqual(self.ink(root, NESTED + f'.a{{{declaration}}}', body), contained,
                                     f'{declaration}: a negative z-index child paints above the background')

    def test_hints_generate_containing_blocks_for_positioned_descendants(self):
        body = '<div class=a><span class=c></span><span class=f></span></div>'
        with tempfile.TemporaryDirectory(prefix='ferri-pdf-will-change-containing-') as directory:
            root = Path(directory)
            self.assert_ink(root, CONTAINED + '.a{will-change:transform}', body,
                            CONTAINED + '.a{transform:translate(0)}',
                            'will-change: transform contains absolute and fixed descendants')
            self.assert_ink(root, CONTAINED + '.a{will-change:filter}', body,
                            CONTAINED + '.a{transform:translate(0)}',
                            'will-change: filter contains absolute and fixed descendants')
            self.assert_ink(root, CONTAINED + '.a{will-change:position}', body,
                            CONTAINED + '.a{position:relative}',
                            'will-change: position contains only absolute descendants')
