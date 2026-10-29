"""Replacement Math glyph ink must bound the shadows of actual variants and assemblies."""
import argparse
from pathlib import Path
import tempfile
import unittest

from fontTools.ttLib import TTFont

import mathml_pdf as math
import table_positioning_pdf as tables
from writing_anchors_pdf import colour_bounds

RENDERER = math.RENDERER
SHADOW_OFFSET_PT = 20
SOURCE_WIDTH_PT = 100
SOURCE_HEIGHT_PT = 100
SOURCE_DEPTH_PT = 10
BASE_HEIGHT_PT = 10
TOP_SPACE_PT = 40
PIXEL_QUANTIZATION = 1
EXTENDER_FLAG = 1
BLACK = (0, 0, 0)
RED = (255, 0, 0)


class MathmlShadowsPdf(unittest.TestCase):
    render = math.MathmlPdf.render

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ferri-mathml-shadow-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        math.RENDERER = RENDERER

    def construction(self, font, character, horizontal):
        variants = font["MATH"].table.MathVariants
        coverage = variants.HorizGlyphCoverage if horizontal else variants.VertGlyphCoverage
        constructions = variants.HorizGlyphConstruction if horizontal else variants.VertGlyphConstruction
        return constructions[coverage.glyphs.index(font.getBestCmap()[ord(character)])]

    def assembly_bounds(self, content, character, horizontal):
        with TTFont(math.MATH_FONT) as font:
            parts = self.construction(font, character, horizontal).GlyphAssembly.PartRecords
            fixed = sum(not (part.PartFlags & EXTENDER_FLAG) for part in parts)
            extenders = sum(bool(part.PartFlags & EXTENDER_FLAG) for part in parts)
            repeats, remainder = divmod(len(content["glyphs"]) - fixed, extenders)
            self.assertEqual(remainder, 0, "actual PDF parts must match the font's assembly records")
            names = [part.glyph for part in parts for _ in range(repeats if part.PartFlags & EXTENDER_FLAG else 1)]
            boxes = []
            for name, glyph in zip(names, content["glyphs"], strict=True):
                outline = font["glyf"][name]
                scale = glyph.matrix[0] / font["head"].unitsPerEm
                x, y = glyph.origin
                boxes.append((x + outline.xMin * scale, math.PAGE_HEIGHT_PT - y - outline.yMax * scale,
                              x + outline.xMax * scale, math.PAGE_HEIGHT_PT - y - outline.yMin * scale))
        return (min(box[0] for box in boxes), min(box[1] for box in boxes),
                max(box[2] for box in boxes), max(box[3] for box in boxes))

    def assert_shadow(self, body, offset, minimum_size, character, horizontal):
        output, content = self.render(body, 'math{font-family:MathTest}')
        with tables.TablePositioningPdf().raster(self.root, output, vector_antialias=False) as image:
            ink = colour_bounds(image, BLACK)
            shadow = colour_bounds(image, RED)
        self.assertIsNotNone(ink, "the actual replacement glyph must paint black ink")
        self.assertIsNotNone(shadow, "its text-shadow must paint visible red ink")
        axis = 0 if horizontal else 1
        self.assertGreater(ink[axis + 2] - ink[axis], minimum_size,
                           "the fixture must exercise enlarged replacement-glyph geometry")
        outlines = self.assembly_bounds(content, character, horizontal)
        for index, (actual, expected) in enumerate(zip(shadow, outlines)):
            self.assertAlmostEqual(actual, expected + offset[index % 2], delta=PIXEL_QUANTIZATION,
                                   msg=f"shadow bounds {shadow} must cover the translated actual font outlines {outlines}")
        return content

    def test_vertical_fence_assembly_shadow_covers_all_replacement_parts(self):
        content = self.assert_shadow(
            f'<math><mo style="text-shadow:{SHADOW_OFFSET_PT}pt 0 red">(</mo>'
            f'<mspace width="{BASE_HEIGHT_PT}pt" height="{SOURCE_HEIGHT_PT}pt" depth="{SOURCE_DEPTH_PT}pt"/></math>',
            (SHADOW_OFFSET_PT, 0), SOURCE_HEIGHT_PT, "(", False)
        self.assertGreater(len(content["glyphs"]), 1,
                           "the actual font must assemble multiple glyph parts")
        self.assertEqual([span.text for span in content["replacement_text"]], ["("],
                         "assembly parts retain one selectable source symbol")

    def test_horizontal_operator_shadow_tracks_assembled_inline_ink(self):
        content = self.assert_shadow(
            f'<math><mover accent="true"><mspace width="{SOURCE_WIDTH_PT}pt" height="{BASE_HEIGHT_PT}pt"/>'
            f'<mo stretchy="true" style="text-shadow:0 {SHADOW_OFFSET_PT}pt red">=</mo></mover></math>',
            (0, SHADOW_OFFSET_PT), SOURCE_WIDTH_PT / 2, "=", True)
        self.assertGreater(len(content["glyphs"]), 1,
                           "the inline stretch must exercise multiple replacement parts")

    def horizontal_advance(self, content):
        with TTFont(math.MATH_FONT) as font:
            last = self.construction(font, "=", True).GlyphAssembly.PartRecords[-1]
            terminal = last.FullAdvance * math.FONT_SIZE / font["head"].unitsPerEm
        return content["glyphs"][-1].origin[0] - content["glyphs"][0].origin[0] + terminal

    def test_stretched_operator_decorations_use_its_actual_advance(self):
        for decoration in ["underline", "overline"]:
            with self.subTest(decoration=decoration):
                output, content = self.render(
                    '<div style="height:40pt"></div><math><mover accent="true">'
                    '<mspace width="100pt" height="10pt"/>'
                    f'<mo stretchy="true" style="text-decoration:{decoration} red;'
                    'text-decoration-skip-ink:none">=</mo></mover></math>',
                    'math{font-family:MathTest}')
                with tables.TablePositioningPdf().raster(self.root, output, vector_antialias=False) as image:
                    red = colour_bounds(image, RED)
                self.assertIsNotNone(red, "the operator's actual decoration must paint")
                advance = self.horizontal_advance(content)
                self.assertGreater(advance, SOURCE_WIDTH_PT / 2, "the font really assembled an enlarged operator")
                self.assertAlmostEqual(red[0], content["glyphs"][0].origin[0], delta=PIXEL_QUANTIZATION)
                self.assertAlmostEqual(red[2] - red[0], advance, delta=PIXEL_QUANTIZATION,
                                       msg="decoration length follows the actual PDF part origins and terminal MATH advance")

    def test_stretched_operator_keeps_one_emphasis_mark_centered_on_its_advance(self):
        output, content = self.render(
            '<div style="height:40pt"></div><math><mover accent="true">'
            '<mspace width="100pt" height="10pt"/>'
            '<mo stretchy="true" style="text-emphasis:filled dot red;text-emphasis-skip:none">=</mo></mover></math>',
            'math{font-family:MathTest}')
        with tables.TablePositioningPdf().raster(self.root, output, vector_antialias=False) as image:
            red = colour_bounds(image, RED)
            pixels = image.load()
            occupied = [x for x in range(image.width) if any(pixels[x, y] == RED for y in range(image.height))]
        self.assertIsNotNone(red, "the original source character retains its emphasis mark")
        self.assertEqual(occupied, list(range(red[0], red[2])),
                         "one source symbol gets one contiguous emphasis mark, not one per assembly part")
        centre = content["glyphs"][0].origin[0] + self.horizontal_advance(content) / 2
        self.assertAlmostEqual((red[0] + red[2]) / 2, centre, delta=PIXEL_QUANTIZATION,
                               msg="the one mark is centered on the actual assembled glyph advance")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--renderer", type=Path, required=True)
    args, rest = parser.parse_known_args()
    RENDERER = args.renderer.resolve()
    unittest.main(argv=[__file__, *rest])
