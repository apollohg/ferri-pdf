"""Principal table geometry and collapsed borders across native writing frames."""
import argparse
from pathlib import Path
import unittest

from PIL import Image, ImageChops

import writing_modes_pdf as writing

MODES = ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]


class WritingTableBordersPdf(unittest.TestCase):
    def assert_native_table(self, mode, table_rules="", cell_rules="", container="", collapse="collapse",
                            reference=None, caption="", caption_rules="", direction="ltr",
                            native_root="horizontal-tb"):
        body = (f'<table>{caption}<tr><td><a id=target href="#target"><div></div></a>'
                '</td></tr></table>')
        rules = (f"html{{direction:{direction}}}body{{width:180pt;height:180pt;{container}}}"
                 f"table{{writing-mode:{mode};border-collapse:{collapse};"
                 f"position:relative;background:yellow;border:8pt solid blue;{table_rules}}}"
                 f"td{{border:12pt solid red;padding:0;{cell_rules}}}"
                 "a{display:block}div{width:40pt;height:20pt;background:lime}"
                 + caption_rules)
        results = []
        for root in [mode, native_root]:
            effective = rules + (f"table{{{reference}}}" if root == mode and reference else "")
            image = writing.WritingModesPdf.render(self, root, body, effective, pixels=True, vector_antialias=False)
            box = ImageChops.difference(image, Image.new("RGB", image.size, "white")).getbbox()
            self.assertIsNotNone(box, "the table must actually paint its border and content")
            if root == "horizontal-tb" and direction == "ltr":
                self.assertGreaterEqual(box[0], writing.MARGIN_PT - 1,
                                        "the full used table wrapper starts inside the parent's allocated span")
            ink = image.crop(box)
            navigation = writing.WritingModesPdf.render(self, root, body, effective, navigation=True)
            self.assertEqual(len(navigation["links"]), 1, str(navigation))
            link = navigation["links"][0]
            rect = link["rect"]
            relative = [rect[0] - box[0], writing.PAGE_PT - rect[3] - box[1],
                        rect[2] - rect[0], rect[3] - rect[1],
                        writing.PAGE_PT - link["destination"]["top"] - box[1]]
            results.append((ink, relative))
        expected, actual = results
        self.assertEqual(actual[0].size, expected[0].size,
                         "a native table uses the solver border extents exactly once")
        self.assertEqual(actual[0].tobytes(), expected[0].tobytes(),
                         "native and same-flow tables keep identical winning border, content, and caption ink")
        for value, reference in zip(actual[1], expected[1]):
            self.assertAlmostEqual(value, reference, delta=writing.TOLERANCE_PT,
                                   msg="link and destination geometry keep the table solver content origin")

    def test_native_principal_table_matches_same_flow_collapsed_border_geometry(self):
        for mode in MODES:
            for table_border, cell_border in [(8, 12), (12, 8)]:
                with self.subTest(mode=mode, table_border=table_border, cell_border=cell_border):
                    self.assert_native_table(mode, f"border-width:{table_border}pt", f"border-width:{cell_border}pt")

    def test_native_principal_table_resolves_constraints_with_winning_half_edges(self):
        cases = ["box-sizing:border-box;width:90pt;height:70pt",
                 "box-sizing:border-box;width:90pt;height:70pt;min-width:100pt;max-height:65pt",
                 "box-sizing:content-box;width:70pt;height:50pt",
                 "box-sizing:border-box;width:20pt;height:10pt"]
        for mode in MODES:
            for rules in cases:
                with self.subTest(mode=mode, rules=rules):
                    self.assert_native_table(mode, rules)

    def test_native_table_principal_background_and_shadow_paint_once(self):
        for mode in MODES:
            with self.subTest(mode=mode):
                self.assert_native_table(mode, "box-shadow:4pt 0 0 black")

    def test_native_principal_table_ratio_uses_winning_border_box_edges(self):
        for mode in MODES:
            with self.subTest(mode=mode):
                self.assert_native_table(mode, "box-sizing:border-box;width:90pt;aspect-ratio:3/2",
                             reference="height:60pt")

    def test_parallel_native_table_uses_the_table_inline_size(self):
        pairs = {"vertical-rl": "vertical-lr", "vertical-lr": "vertical-rl",
                 "sideways-rl": "sideways-lr", "sideways-lr": "sideways-rl"}
        for mode, parent in pairs.items():
            for rules in ["", "box-sizing:border-box;width:90pt;height:70pt"]:
                with self.subTest(mode=mode, parent=parent, rules=rules):
                    self.assert_native_table(mode, rules, native_root=parent)

    def test_native_principal_table_keeps_caption_paint_outside_the_principal_border(self):
        for mode in MODES:
            for side in ["top", "bottom"]:
                for sizing in ["", "box-sizing:border-box;width:90pt;height:70pt"]:
                    with self.subTest(mode=mode, side=side, sizing=sizing):
                        self.assert_native_table(mode, sizing, caption="<caption><b></b></caption>",
                                     caption_rules=f"caption{{caption-side:{side};margin:3pt 5pt;background:cyan}}"
                                                   "caption b{display:block;width:10pt;height:10pt;background:magenta}")

    def test_native_principal_table_preserves_asymmetric_edges_and_rtl(self):
        for mode in MODES:
            for direction in ["ltr", "rtl"]:
                with self.subTest(mode=mode, direction=direction):
                    self.assert_native_table(mode, f"direction:{direction};border-width:4pt 8pt 12pt 16pt",
                                 "border-width:10pt 14pt 6pt 2pt", direction=direction)

    def test_native_table_owner_uses_the_winning_padding_edge_in_each_layout_model(self):
        for mode in MODES:
            for container in ["", "display:grid;grid-template:120pt / 120pt;align-items:start;justify-items:start",
                              "display:flex;align-items:flex-start"]:
                for table_border in [0, 8]:
                    with self.subTest(mode=mode, container=container, table_border=table_border):
                        width, height, cell_border, marker = 90, 70, 12, 4
                        rules = (f"body{{width:180pt;height:180pt;{container}}}"
                                 f"table{{writing-mode:{mode};position:relative;border-collapse:collapse;"
                                 f"border:{table_border}pt solid blue;box-sizing:border-box;width:{width}pt;height:{height}pt}}"
                                 f"td{{border:{cell_border}pt solid red;padding:0}}div{{width:40pt;height:20pt;background:lime}}"
                                 f"i{{position:absolute;right:0;bottom:0;width:{marker}pt;height:{marker}pt;background:black}}")
                        image = writing.WritingModesPdf.render(
                            self, "horizontal-tb", "<table><tr><td><div></div><i></i></td></tr></table>", rules, pixels=True)
                        border_half = max(table_border, cell_border) // 2
                        right = writing.MARGIN_PT + width - border_half
                        bottom = writing.MARGIN_PT + height - border_half
                        self.assertEqual(image.getpixel((right - marker // 2, bottom - marker // 2)), (0, 0, 0),
                                         "positioned content ends at the winning half-border padding edge")
                        self.assertEqual(image.getpixel((right + marker // 2, bottom - marker // 2)), (255, 0, 0),
                                         "the winning border remains outside the table padding edge")

    def test_native_table_caption_wrapper_establishes_the_positioned_containing_box(self):
        width, height, caption_extent, margin, marker = 90, 70, 10, 5, 4
        for mode in MODES:
            for side in ["top", "bottom"]:
                with self.subTest(mode=mode, side=side):
                    rules = (f"table{{writing-mode:{mode};position:relative;border-collapse:collapse;"
                             f"border:8pt solid blue;box-sizing:border-box;width:{width}pt;height:{height}pt}}"
                             "td{border:12pt solid red;padding:0}div{width:40pt;height:20pt;background:lime}"
                             f"caption{{caption-side:{side};margin:3pt {margin}pt;background:cyan}}"
                             f"caption b{{display:block;width:{caption_extent}pt;height:10pt;background:magenta}}"
                             f"i{{position:absolute;right:0;bottom:0;width:{marker}pt;height:{marker}pt;background:black}}")
                    image = writing.WritingModesPdf.render(
                        self, "horizontal-tb", "<table><caption><b></b></caption><tr><td><div></div><i></i></td></tr></table>",
                        rules, pixels=True)
                    right = writing.MARGIN_PT + width + caption_extent + 2 * margin
                    bottom = writing.MARGIN_PT + height
                    self.assertEqual(image.getpixel((right - marker // 2, bottom - marker // 2)), (0, 0, 0),
                                     "the positioned table's wrapper includes caption margins and has no principal borders")

    def test_native_table_and_caption_keep_their_actual_link_fragments(self):
        for mode in MODES:
            for side in ["top", "bottom"]:
                with self.subTest(mode=mode, side=side):
                    body = ('<a href="#target"><table id=target><caption><b></b></caption>'
                            '<tr><td><div></div></td></tr></table></a>')
                    rules = (f"body{{width:180pt;height:180pt}}table{{writing-mode:{mode};"
                             "border-collapse:collapse;border:8pt solid blue;background:yellow}"
                             "td{border:12pt solid red;padding:0}div{width:40pt;height:20pt;background:lime}"
                             f"caption{{caption-side:{side};margin:3pt 5pt;background:cyan}}"
                             "caption b{display:block;width:10pt;height:10pt;background:magenta}")
                    results = []
                    for root in [mode, "horizontal-tb"]:
                        image = writing.WritingModesPdf.render(self, root, body, rules, pixels=True,
                                                              vector_antialias=False)
                        box = ImageChops.difference(image, Image.new("RGB", image.size, "white")).getbbox()
                        navigation = writing.WritingModesPdf.render(self, root, body, rules, navigation=True)
                        links = []
                        for link in navigation["links"]:
                            rect = link["rect"]
                            links.append((rect[0] - box[0], writing.PAGE_PT - rect[3] - box[1],
                                          rect[2] - rect[0], rect[3] - rect[1],
                                          writing.PAGE_PT - link["destination"]["top"] - box[1]))
                        results.append(sorted(links))
                    self.assertEqual(len(results[0]), 2, "the control has actual table and caption fragments")
                    self.assertEqual(len(results[1]), len(results[0]),
                                     "the native transport must not activate the blank caption margins")
                    for actual, expected in zip(results[1], results[0]):
                        for value, reference in zip(actual, expected):
                            self.assertAlmostEqual(value, reference, delta=writing.TOLERANCE_PT,
                                                   msg="actual principal and caption links retain their own rectangles and table destination")

    def test_native_principal_table_inline_grid_and_separated_controls(self):
        for mode in MODES:
            for table, container, collapse in [("display:inline-table", "", "collapse"),
                                               ("", "display:grid;grid-template:120pt / 120pt;align-items:start;justify-items:start", "collapse"),
                                               ("padding:3pt", "", "separate")]:
                with self.subTest(mode=mode, table=table, container=container, collapse=collapse):
                    self.assert_native_table(mode, table, container=container, collapse=collapse)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--renderer", type=Path, required=True)
    args, rest = parser.parse_known_args()
    writing.RENDERER = args.renderer.resolve()
    unittest.main(argv=[__file__, *rest])
