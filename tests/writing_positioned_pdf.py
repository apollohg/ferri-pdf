"""Positioned descendants and paint scopes across native writing frames."""
import argparse
from pathlib import Path
import unittest

from PIL import ImageChops

import writing_modes_pdf as writing

FRAGMENTS = {"vertical-rl": (68, 20), "vertical-lr": (20, 20),
             "sideways-rl": (68, 20), "sideways-lr": (20, 80)}
MODES = list(FRAGMENTS)


class WritingPositionedPdf(unittest.TestCase):
    def image(self, mode, body, rules):
        return writing.WritingModesPdf.render(
            self, "horizontal-tb", body,
            f"body{{position:relative;width:180pt;height:180pt}}"
            f"section{{writing-mode:{mode};width:60pt;height:80pt}}{rules}", pixels=True)

    def test_fixed_and_absolute_boxes_use_their_external_containing_block(self):
        for mode in MODES:
            for position, point, colour in [("fixed", (32, 31), (255, 0, 0)),
                                            ("absolute", (55, 40), (0, 0, 255))]:
                with self.subTest(mode=mode, position=position):
                    rules = (f"i{{position:{position};writing-mode:horizontal-tb;display:block;"
                             "width:20pt;height:12pt;"
                             + ("left:7pt;top:5pt;background:red}" if position == "fixed"
                                else "left:25pt;top:15pt;background:blue}"))
                    image = self.image(mode, "<section><i></i></section>", rules)
                    self.assertEqual(image.getpixel(point), colour,
                                     f"{position} descendant of {mode} must use the external box's physical axes")

    def test_escaping_absolute_box_keeps_opacity_but_skips_noncontaining_overflow(self):
        for mode in MODES:
            with self.subTest(mode=mode):
                image = self.image(mode, "<section><i></i></section>",
                                   "section{opacity:.5;overflow:hidden}"
                                   "i{position:absolute;writing-mode:horizontal-tb;left:120pt;top:10pt;"
                                   "width:20pt;height:12pt;background:red}")
                actual = image.getpixel((150, 35))
                expected = (255, 128, 128)
                self.assertTrue(all(abs(a - e) <= 1 for a, e in zip(actual, expected)),
                                f"{mode}: escaped box must retain ancestor opacity, rendered {actual}")

    def test_local_positioned_descendant_keeps_global_negative_stacking_order(self):
        for mode in MODES:
            with self.subTest(mode=mode):
                image = self.image(mode, "<main><section><i></i></section></main>",
                                   "main{position:relative;width:100pt;height:100pt;background:lime}"
                                   "section{position:relative}"
                                   "i{position:absolute;writing-mode:horizontal-tb;z-index:-1;"
                                   "left:10pt;top:10pt;width:20pt;height:20pt;background:red}")
                self.assertEqual(image.getpixel((40, 40)), (0, 255, 0),
                                 f"{mode}: a writing frame must not create a stacking context")

    def test_external_fixed_descendant_repeats_with_the_same_physical_text_position(self):
        for mode in MODES:
            with self.subTest(mode=mode):
                output = writing.WritingModesPdf.render(
                    self, "horizontal-tb",
                    "<section><aside>X</aside></section><p>P</p><p>R</p>",
                    f"section{{writing-mode:{mode};width:60pt;height:80pt}}"
                    "aside{position:fixed;writing-mode:horizontal-tb;left:7pt;top:5pt;"
                    "width:20pt;height:20pt}p{break-before:page}")
                fixed = [glyph for glyph in output["glyphs"] if glyph.text == "X"]
                self.assertEqual([glyph.page for glyph in fixed], [1, 2, 3],
                                 f"{mode}: escaped fixed text repeats on every physical page")
                for glyph in fixed:
                    self.assertAlmostEqual(glyph.matrix[4], 27, delta=writing.TOLERANCE_PT)
                    self.assertEqual(glyph.matrix, fixed[0].matrix,
                                     f"{mode}: page repetition preserves physical placement")

    def test_native_link_annotation_and_destination_use_physical_page_coordinates(self):
        for mode, (left, top) in FRAGMENTS.items():
            with self.subTest(mode=mode):
                navigation = writing.WritingModesPdf.render(
                    self, "horizontal-tb", '<section><a href="#mark" id="mark">X</a></section>',
                    f"section{{writing-mode:{mode};width:60pt;height:80pt}}"
                    "a{display:block;width:12pt;height:20pt}", navigation=True)
                self.assertEqual(len(navigation["links"]), 1, str(navigation))
                link = navigation["links"][0]
                self.assertEqual(link["page"], 1)
                expected = [left, writing.PAGE_PT - top - 20,
                            left + 12, writing.PAGE_PT - top]
                for actual, value in zip(link["rect"], expected):
                    self.assertAlmostEqual(actual, value, delta=writing.TOLERANCE_PT,
                                           msg=f"{mode}: native annotation maps to physical box {expected}")
                self.assertEqual(link["destination"]["page"], 1)
                self.assertAlmostEqual(link["destination"]["top"], writing.PAGE_PT - top,
                                       delta=writing.TOLERANCE_PT,
                                       msg=f"{mode}: target uses the same physical top as its link box")

    def test_native_compositing_and_physical_clip_keep_the_fragment_geometry(self):
        for mode, (left, top) in FRAGMENTS.items():
            for effect, expected in [("opacity:.5;filter:opacity(.5)", (255, 191, 191)),
                                     ("mask-image:linear-gradient(rgb(0 0 0 / .5),rgb(0 0 0 / .5))", (255, 128, 128)),
                                     ("clip-path:inset(0 6pt 0 0)", (255, 0, 0))]:
                with self.subTest(mode=mode, effect=effect):
                    image = self.image(mode, "<section><a></a></section>",
                                       "a{display:block;width:12pt;height:20pt;background:red;"
                                       + effect + "}")
                    actual = image.getpixel((left + 3, top + 10))
                    self.assertTrue(all(abs(a - e) <= 1 for a, e in zip(actual, expected)),
                                    f"{mode}: {effect} must composite at the physical fragment, got {actual}")
                    if effect.startswith("clip-path"):
                        self.assertEqual(image.getpixel((left + 9, top + 10)), (255, 255, 255),
                                         f"{mode}: physical right half is clipped")

    def test_parent_perspective_crosses_the_native_frame_in_physical_coordinates(self):
        perspective_pt, depth_pt = 100, 20
        for mode, (left, top) in FRAGMENTS.items():
            with self.subTest(mode=mode):
                image = self.image(mode, "<section><a></a></section>",
                                   f"section{{perspective:{perspective_pt}pt;perspective-origin:0 0}}"
                                   "a{display:block;width:12pt;height:20pt;background:red;"
                                   f"transform:translateZ({depth_pt}pt)}}")
                scale = perspective_pt / (perspective_pt - depth_pt)
                x = writing.MARGIN_PT + (left - writing.MARGIN_PT) * scale
                y = writing.MARGIN_PT + (top - writing.MARGIN_PT) * scale
                self.assertEqual(image.getpixel((round(x + 7), round(y + 12))), (255, 0, 0),
                                 f"{mode}: parent perspective scales the child's physical box about the section origin")
                self.assertEqual(image.getpixel((round(x + 13), round(y + 23))), (255, 0, 0),
                                 f"{mode}: perspective expands beyond the untransformed box on both axes")
                self.assertEqual(image.getpixel((round(x + 18), round(y + 12))), (255, 255, 255),
                                 f"{mode}: perspective keeps the expected 15pt physical box width")

    def test_native_table_background_layers_are_retained_under_the_cell(self):
        body = "<section><article><table><colgroup><col></colgroup><tbody><tr><td></td></tr></tbody></table></article></section>"
        geometry = "table{border-spacing:0;width:12pt;height:20pt}td{position:relative;padding:0;width:12pt;height:20pt}"
        for mode in MODES:
            for columns in [False, True]:
                rules = geometry + ("article{columns:2;column-gap:0}" if columns else "")
                expected = self.image(mode, body, rules + "td{background:red}")
                self.assertIn((255, 0, 0), expected.getdata(), f"{mode}: reference cell paints its background")
                for selector in ["table", "colgroup", "col", "tbody", "tr"]:
                    with self.subTest(mode=mode, columns=columns, background=selector):
                        actual = self.image(mode, body, rules + selector + "{background:red}")
                        difference = ImageChops.difference(actual, expected).getbbox()
                        self.assertIsNone(difference,
                                          f"{mode}: the single-cell {selector} background differs in {difference}")

    def test_physical_shadow_offsets_survive_a_horizontal_frame_in_a_vertical_page(self):
        for mode, left, top in [("vertical-rl", 160, 20), ("vertical-lr", 20, 20),
                                ("sideways-rl", 160, 20), ("sideways-lr", 20, 140)]:
            for effect in ["box-shadow:30pt 0 green", "filter:drop-shadow(30pt 0 green)"]:
                with self.subTest(mode=mode, effect=effect):
                    image = writing.WritingModesPdf.render(
                        self, mode, "<section><a></a></section>",
                        "section{writing-mode:horizontal-tb;width:60pt;height:80pt}"
                        "a{display:block;width:12pt;height:20pt;background:red;" + effect + "}",
                        pixels=True)
                    for point, expected in [((left + 6, top + 10), (255, 0, 0)),
                                            ((left + 36, top + 10), (0, 128, 0))]:
                        actual = image.getpixel(point)
                        self.assertTrue(all(abs(a - e) <= 1 for a, e in zip(actual, expected)),
                                        f"{mode}: {effect} at physical {point} expected {expected}, got {actual}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=writing.RENDERER)
    arguments = parser.parse_args()
    writing.RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
