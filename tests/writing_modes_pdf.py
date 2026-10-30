"""Verify CSS Writing Modes 4 line progression and glyph orientation in PDF content."""
import argparse
import base64
from pathlib import Path
import subprocess
import tempfile
import unittest

from oracle import pdf_content
from fontTools.ttLib import TTFont
from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
PAGE_PT = 240
MARGIN_PT = 20
FONT_PT = 20
TOLERANCE_PT = 0.01


class WritingModesPdf(unittest.TestCase):
    def render(self, mode, body="<p>AB</p><p>CD</p>", rules="", pixels=False, navigation=False,
               vector_antialias=True):
        with tempfile.TemporaryDirectory(prefix="ferri-writing-modes-") as directory:
            source = Path(directory) / "input.html"
            target = Path(directory) / "output.pdf"
            source.write_text(f"<!doctype html><style>@page{{size:{PAGE_PT}pt;margin:{MARGIN_PT}pt}}"
                              f"html{{writing-mode:{mode}}}body{{margin:0;font:{FONT_PT}pt/1 serif}}"
                              f"p{{margin:0}}{rules}</style>{body}")
            result = subprocess.run([str(RENDERER), str(source), str(target)],
                                    capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, f"{mode} must render: {result.stderr}")
            if pixels:
                prefix = Path(directory) / "raster"
                raster = subprocess.run(["pdftoppm", "-png", "-singlefile", "-r", "72",
                                         "-aaVector", "yes" if vector_antialias else "no", str(target), str(prefix)],
                                        capture_output=True, text=True, timeout=60)
                self.assertEqual(raster.returncode, 0, raster.stderr)
                with Image.open(prefix.with_suffix(".png")) as image:
                    return image.convert("RGB").copy()
            return pdf_content.navigation(target) if navigation else pdf_content.inspect(target)

    def test_columns_do_not_export_an_orthogonal_painted_text_baseline(self):
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                rules = (".columns{display:inline-block;column-count:1;column-fill:auto;width:40pt;height:40pt}"
                         f"section{{writing-mode:{mode};width:40pt;height:40pt}}")
                empty = self.render("horizontal-tb", "<div>A<span class=columns><section></section></span>B</div>", rules)["glyphs"]
                filled = self.render("horizontal-tb", "<div>A<span class=columns><section>X</section></span>B</div>", rules)["glyphs"]
                actual = {g.text:g.matrix for g in filled if g.text in "AB"}
                self.assertEqual(set(actual), set("AB"))
                for glyph in empty:
                    for value, reference in zip(actual[glyph.text], glyph.matrix):
                        self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                               msg=f"{mode}: native paint lines perpendicular to the column baseline do not change the inline block's synthesized baseline")

    def test_orthogonal_intrinsic_inline_sizes_resolve_before_frame_layout(self):
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            advance = lambda text: sum(font["hmtx"].metrics[cmap[ord(ch)]][0] for ch in text) * FONT_PT / font["head"].unitsPerEm
            sizes = {"min-content":advance("AA"), "max-content":advance("AA AA")}
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            for keyword, length in sizes.items():
                with self.subTest(mode=mode, keyword=keyword):
                    body = "<section>AA AA</section><p>Z</p>"
                    rule = f"section{{writing-mode:{mode};width:60pt;height:"
                    intrinsic = self.render("horizontal-tb", body, rule + keyword + "}")["glyphs"]
                    definite = self.render("horizontal-tb", body, rule + f"{length}pt}}")["glyphs"]
                    self.assertEqual([g.text for g in intrinsic], [g.text for g in definite])
                    for actual, expected in zip(intrinsic, definite):
                        for value, reference in zip(actual.matrix, expected.matrix):
                            self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                                   msg=f"{mode}/{keyword}: physical height uses the intrinsic inline measure")

    def test_auto_inline_orthogonal_blocks_wrap_into_columns_in_the_parent_flow(self):
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            column = max(font["hmtx"].metrics[cmap[ord(ch)]][0] for ch in "ABCD") * FONT_PT / font["head"].unitsPerEm
        inline_extent = 2 * column + FONT_PT
        body = "<section>A<br>B<br>C<br>D</section><p>Z</p>"
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            for direction in ["ltr", "rtl"]:
                with self.subTest(mode=mode, direction=direction):
                    rules = f"section{{writing-mode:{mode};direction:{direction};width:{3 * FONT_PT}pt;column-gap:{FONT_PT}pt;column-fill:auto;orphans:1;widows:1}}"
                    automatic = self.render("horizontal-tb", body, rules)["glyphs"]
                    explicit = self.render("horizontal-tb", body, rules + f"section{{height:{inline_extent}pt;column-count:2}}")["glyphs"]
                    self.assertEqual([g.text for g in automatic], list("ABCDZ"), "all automatic columns remain in the printable parent flow")
                    self.assertEqual([g.text for g in explicit], [g.text for g in automatic])
                    for actual, expected in zip(automatic, explicit):
                        for value, reference in zip(actual.matrix, expected.matrix):
                            self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                                   msg=f"{mode} {actual.text}: automatic columns match two explicit columns, including following sibling placement")

    def test_positioned_fallback_uses_ancestor_scrollports_and_the_actual_containing_block(self):
        text = " ".join(["AA"] * 8)
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            for position in ["absolute", "fixed"]:
                for scrollport in [False, True]:
                    with self.subTest(mode=mode, position=position, scrollport=scrollport):
                        ancestor = f"height:{5 * FONT_PT}pt;overflow:hidden" if scrollport else f"max-height:{FONT_PT}pt"
                        available = 5 * FONT_PT if scrollport else (PAGE_PT - 2 * MARGIN_PT if position == "fixed" else 9 * FONT_PT)
                        rules = (f".owner{{position:relative;width:{9 * FONT_PT}pt;height:{9 * FONT_PT}pt}}main{{{ancestor}}}"
                                 f"section{{position:{position};top:0;left:0;writing-mode:{mode};width:{5 * FONT_PT}pt;text-align:end}}")
                        body = f"<div class=owner><main><div><section>{text}</section></div></main></div>"
                        automatic = self.render("horizontal-tb", body, rules)["glyphs"]
                        definite = self.render("horizontal-tb", body, rules + f"section{{height:{available}pt}}")["glyphs"]
                        actual_letters = [g for g in automatic if g.text == "A"]
                        expected_letters = [g for g in definite if g.text == "A"]
                        self.assertEqual(len(actual_letters), len(expected_letters))
                        for actual, expected in zip(actual_letters, expected_letters):
                            for value, reference in zip(actual.matrix, expected.matrix):
                                self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                                       msg=f"{mode}/{position}: scrollport provenance survives deferred placement, and a non-containing DOM ancestor's maximum does not replace the actual CB")

    def test_indefinite_inline_space_reaches_content_sized_layout_items(self):
        text = " ".join(["AA"] * 8)
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            advances = {ch:font["hmtx"].metrics[cmap[ord(ch)]][0] * FONT_PT / font["head"].unitsPerEm for ch in "A Z"}
        target = 16 * advances["A"] + 7 * advances[" "]
        layouts = [
            ("flex", "display:flex", "flex:0 0 auto", f"flex:0 0 {5 * FONT_PT}pt"),
            ("intrinsic flex", "display:flex", "flex:0 0 auto;inline-size:max-content", f"flex:0 0 {5 * FONT_PT}pt;inline-size:max-content"),
            ("grid", "display:grid;grid-template-columns:max-content max-content", "", ""),
            ("table", "display:table;border-collapse:collapse", "display:table-cell;padding:0", f"display:table-cell;padding:0;inline-size:{5 * FONT_PT}pt"),
        ]
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            for label, layout, content, fixed in layouts:
                for definite in [False, True]:
                    with self.subTest(mode=mode, layout=label, definite=definite):
                        container = layout
                        if label == "grid" and definite:
                            container = f"display:grid;grid-template-columns:{5 * FONT_PT}pt max-content"
                        child = fixed if definite else content
                        shift = 5 * FONT_PT if mode == "sideways-lr" else -5 * FONT_PT
                        rules = (f"section{{writing-mode:{mode};width:{5 * FONT_PT}pt;position:relative;top:{shift}pt;{container}}}"
                                 f".item{{{child};background:red}}.next{{{content}}}")
                        contents = f"<div>{text}</div>" if label == "intrinsic flex" else text
                        body = f"<section><div class=item>{contents}</div><div class=next>Z</div></section>"
                        glyphs = self.render("horizontal-tb", body, rules)["glyphs"]
                        columns = {round(g.matrix[4], 2) for g in glyphs if g.text == "A"}
                        self.assertEqual(len(columns), 3 if definite else 2,
                                         "definite item availability uses 100pt; content-sized layouts pass indefinite provenance so block content wraps at the 200pt ICB fallback")
                        first = next(g for g in glyphs if g.text == "A")
                        following = next(g for g in glyphs if g.text == "Z")
                        extent = 5 * FONT_PT if definite else target
                        sign = 1 if mode == "sideways-lr" else -1
                        self.assertAlmostEqual(following.matrix[5] - first.matrix[5], sign * extent,
                                               delta=TOLERANCE_PT, msg="following items retain the layout model's principal box allocation")
                        if not definite:
                            image = self.render("horizontal-tb", body, rules, pixels=True)
                            distance = extent - FONT_PT / 2
                            y = MARGIN_PT + shift + (extent + advances["Z"] - distance if mode == "sideways-lr" else distance)
                            self.assertEqual(image.getpixel((int(MARGIN_PT + FONT_PT / 2), int(y))), (255,0,0),
                                             "the principal item's background covers its assigned width beyond the fallback content column")

    def test_short_content_sized_item_columns_preserve_the_assigned_model_box(self):
        preferred = 5 * FONT_PT
        layouts = [("flex", "display:flex", "flex:0 0 auto"),
                   ("grid", "display:grid;grid-template-columns:max-content max-content", ""),
                   ("table", "display:table;border-collapse:collapse", "display:table-cell;padding:0")]
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            for label, layout, item in layouts:
                with self.subTest(mode=mode, layout=label):
                    rules = (f"section{{writing-mode:{mode};width:{preferred}pt;{layout}}}"
                             f".item{{{item};column-width:{preferred}pt;text-align:end}}.next{{{item}}}")
                    glyphs = {g.text:g.matrix for g in self.render("horizontal-tb", "<section><div class=item>A</div><div class=next>Z</div></section>", rules)["glyphs"]}
                    sign = 1 if mode == "sideways-lr" else -1
                    self.assertAlmostEqual(glyphs["Z"][5] - glyphs["A"][5], sign * preferred,
                                           delta=TOLERANCE_PT, msg="a short unwrapped content column shrinks before end alignment while the following item keeps the 100pt principal allocation")

    def test_an_orthogonal_scrollport_constrains_a_second_writing_boundary(self):
        limit = 5 * FONT_PT
        text = " ".join(["AA"] * 8)
        body = f"<section><div><article>{text}</article></div></section><p>Z</p>"
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                rules = (f"section{{writing-mode:{mode};width:{limit}pt;height:{limit + 2 * FONT_PT}pt;overflow:hidden}}"
                         f"article{{writing-mode:horizontal-tb;height:{limit}pt}}")
                automatic = self.render("horizontal-tb", body, rules)["glyphs"]
                definite = self.render("horizontal-tb", body, rules + f"article{{width:{limit}pt}}")["glyphs"]
                self.assertEqual([g.text for g in automatic], [g.text for g in definite])
                for actual, expected in zip(automatic, definite):
                    for value, reference in zip(actual.matrix, expected.matrix):
                        self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                               msg=f"{mode}: the physical width of an orthogonal scrollport survives an intervening auto-sized box")

    def test_orthogonal_fallback_obeys_containing_bounds_and_the_nearest_scrollport(self):
        limit = 5 * FONT_PT
        cases = [
            ("containing maximum", f"max-height:{limit}pt", False, limit),
            ("containing minimum floor", f"max-height:{limit}pt;min-height:{limit + 2 * FONT_PT}pt", False, limit + 2 * FONT_PT),
            ("fixed ancestor scrollport", f"height:{limit}pt;overflow:hidden", True, limit),
            ("bounded ancestor scrollport", f"max-height:{limit}pt;overflow:hidden", True, limit),
            ("padded ancestor scrollport", f"height:{limit}pt;padding:5pt 7pt 15pt 9pt;border:3pt solid green;overflow:hidden", True, limit + FONT_PT),
        ]
        text = " ".join(["AA"] * 8)
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            for label, parent, nested, available in cases:
                with self.subTest(mode=mode, constraint=label):
                    section = f"<section>{text}</section>"
                    body = "<main>" + (f"<div>{section}</div>" if nested else section) + "</main><p>Z</p>"
                    rules = f"main{{{parent}}}section{{writing-mode:{mode};width:{limit}pt;text-align:end}}"
                    automatic = self.render("horizontal-tb", body, rules)["glyphs"]
                    definite = self.render("horizontal-tb", body, rules + f"section{{height:{available}pt}}")["glyphs"]
                    self.assertEqual([g.text for g in automatic], [g.text for g in definite])
                    for actual, expected in zip(automatic, definite):
                        for value, reference in zip(actual.matrix, expected.matrix):
                            self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                                   msg=f"{mode}/{label}: fallback wrapping uses the constrained physical inline axis")

    def test_automatic_columns_reflow_spanners_at_the_final_inline_extent(self):
        count = 4
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            column = sum(font["hmtx"].metrics[cmap[ord(ch)]][0] for ch in "AB") * FONT_PT / font["head"].unitsPerEm
        inline_extent = count * column + (count - 1) * FONT_PT
        body = "<section><p>A<br>B<br>C<br>D</p><h1>AB</h1><p>A<br>B<br>C<br>D</p></section><p>Z</p>"
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                rules = (f"section{{writing-mode:{mode};width:{3 * FONT_PT}pt;column-gap:{FONT_PT}pt;"
                         "column-fill:auto;orphans:1;widows:1}h1{font:inherit;margin:0;column-span:all}")
                automatic = self.render("horizontal-tb", body, rules)["glyphs"]
                explicit = self.render("horizontal-tb", body, rules + f"section{{height:{inline_extent}pt;column-count:{count}}}")["glyphs"]
                self.assertEqual([g.text for g in automatic], [g.text for g in explicit])
                for actual, expected in zip(automatic, explicit):
                    for value, reference in zip(actual.matrix, expected.matrix):
                        self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                               msg=f"{mode}/{actual.text}: spanners and their following column row use the final automatic container width")

    def test_short_auto_columns_shrink_despite_a_larger_preferred_column_width(self):
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            advance = font["hmtx"].metrics[cmap[ord("A")]][0] * FONT_PT / font["head"].unitsPerEm
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            for direction in ["ltr", "rtl"]:
                with self.subTest(mode=mode, direction=direction):
                    body = "<section>A</section><p>Z</p>"
                    rules = (f"section{{writing-mode:{mode};direction:{direction};width:{3 * FONT_PT}pt;"
                             f"column-width:{5 * FONT_PT}pt;text-align:end}}")
                    automatic = self.render("horizontal-tb", body, rules)["glyphs"]
                    definite = self.render("horizontal-tb", body, rules + f"section{{height:{advance}pt}}")["glyphs"]
                    for actual, expected in zip(automatic, definite):
                        self.assertEqual(actual.text, expected.text)
                        for value, reference in zip(actual.matrix, expected.matrix):
                            self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                                   msg=f"{mode}/{direction}: unwrapped one-column content shrinks to max-content before end alignment")

    def test_one_auto_column_shrinks_but_respects_the_physical_block_minimum(self):
        minimum = 5 * FONT_PT
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            advance = font["hmtx"].metrics[cmap[ord("A")]][0] * FONT_PT / font["head"].unitsPerEm
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                body = "<section>A</section><p>Z</p>"
                rules = f"section{{writing-mode:{mode};min-width:{minimum}pt}}"
                automatic = self.render("horizontal-tb", body, rules)["glyphs"]
                definite = self.render("horizontal-tb", body, rules + f"section{{width:{minimum}pt;height:{advance}pt}}")["glyphs"]
                for actual, expected in zip(automatic, definite):
                    self.assertEqual(actual.text, expected.text)
                    for value, reference in zip(actual.matrix, expected.matrix):
                        self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                               msg=f"{mode}: short auto content uses one inline cell and keeps block-start alignment inside min-width")

    def test_rtl_columns_start_at_the_container_end_when_gaps_overflow(self):
        container = 3 * FONT_PT
        gap = 5 * FONT_PT
        rules = (f"section{{width:{container}pt;height:{FONT_PT}pt;direction:rtl;column-count:2;"
                 f"column-gap:{gap}pt;column-fill:auto;orphans:1;widows:1}}")
        glyphs = {g.text:g.matrix for g in self.render("horizontal-tb", "<section>A<br>B</section>", rules)["glyphs"]}
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            widths = {ch:font["hmtx"].metrics[cmap[ord(ch)]][0] * FONT_PT / font["head"].unitsPerEm for ch in "AB"}
        self.assertAlmostEqual(glyphs["A"][4] + widths["A"], MARGIN_PT + container, delta=TOLERANCE_PT,
                               msg="a large gap must not move the first RTL column beyond the container's inline end")
        self.assertAlmostEqual((glyphs["A"][4] + widths["A"]) - (glyphs["B"][4] + widths["B"]), gap,
                               delta=TOLERANCE_PT, msg="overflow columns advance by the used gap when column widths collapse to zero")

    def test_indefinite_percentage_inline_size_uses_orthogonal_auto_sizing(self):
        body = "<section>A<br>B<br>C<br>D</section><p>Z</p>"
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                rules = (f"section{{writing-mode:{mode};width:60pt;column-gap:20pt;"
                         "column-fill:auto;orphans:1;widows:1}")
                automatic = self.render("horizontal-tb", body, rules)["glyphs"]
                percent = self.render("horizontal-tb", body, rules + "section{height:50%}")["glyphs"]
                self.assertEqual([g.text for g in percent], [g.text for g in automatic])
                for actual, expected in zip(percent, automatic):
                    for value, reference in zip(actual.matrix, expected.matrix):
                        self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                               msg=f"{mode}/{actual.text}: percentage physical height without a definite containing height behaves as auto, including the following sibling")

    def test_cyclic_column_gap_keeps_auto_size_and_rtl_inline_start(self):
        gap_percent = 10
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            column = max(font["hmtx"].metrics[cmap[ord(ch)]][0] for ch in "ABCD") * FONT_PT / font["head"].unitsPerEm
        used_gap = 2 * column * gap_percent / 100
        body = "<section>A<br>B<br>C<br>D</section><p>Z</p>"
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            for direction in ["ltr", "rtl"]:
                with self.subTest(mode=mode, direction=direction):
                    rules = (f"section{{writing-mode:{mode};direction:{direction};width:{3 * FONT_PT}pt;"
                             "column-fill:auto;orphans:1;widows:1;column-gap:")
                    zero = {g.text:g.matrix for g in self.render("horizontal-tb", body, rules + "0}")["glyphs"]}
                    cyclic = {g.text:g.matrix for g in self.render("horizontal-tb", body, rules + f"{gap_percent}%}}")["glyphs"]}
                    self.assertEqual(cyclic["Z"], zero["Z"], "a cyclic gap does not feed overflow back into the auto inline size")
                    for value, reference in zip(cyclic["A"], zero["A"]):
                        self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                               msg="the first column keeps the container's inline start, including RTL")
                    next_column = "D"
                    sign = 1 if mode == "sideways-lr" else -1
                    if direction == "rtl":
                        sign = -sign
                    self.assertAlmostEqual(cyclic[next_column][5] - zero[next_column][5], sign * used_gap,
                                           delta=TOLERANCE_PT, msg="the used percentage gap resolves against the determined container inline size")

    def test_sideways_modes_rotate_glyphs_and_reverse_column_progression(self):
        # Writing Modes 4 §3.2: sideways-lr rotates its horizontal typographic mode
        # counter-clockwise; sideways-rl rotates clockwise.
        for mode, sign in [("sideways-rl", -1), ("sideways-lr", 1)]:
            with self.subTest(mode=mode):
                glyphs = self.render(mode)["glyphs"]
                self.assertEqual([glyph.text for glyph in glyphs], list("ABCD"))
                first, second, third, _ = [glyph.matrix for glyph in glyphs]
                self.assertAlmostEqual(first[0], 0, delta=TOLERANCE_PT, msg=str(first))
                self.assertAlmostEqual(first[3], 0, delta=TOLERANCE_PT, msg=str(first))
                self.assertGreater(sign * first[1], 0, f"glyph orientation: {first}")
                self.assertGreater(sign * (second[5] - first[5]), 0,
                                   f"inline progression: {first}, {second}")
                self.assertGreater(sign * (third[4] - first[4]), 0,
                                   f"block progression: {first}, {third}")

    def test_orthogonal_horizontal_inline_keeps_horizontal_glyphs(self):
        glyphs = self.render("sideways-rl", "<p>A<span>BC</span>D</p>",
                             "span{writing-mode:horizontal-tb}")["glyphs"]
        text = {glyph.text: glyph.matrix for glyph in glyphs}
        self.assertEqual(set(text), set("ABCD"))
        self.assertGreater(text["B"][0], 0, f"orthogonal glyph B: {text}")
        self.assertAlmostEqual(text["B"][1], 0, delta=TOLERANCE_PT)
        self.assertGreater(text["C"][4], text["B"][4], str(text))

    def test_orthogonal_aspect_ratio_transfers_a_definite_physical_axis(self):
        size = 5 * FONT_PT
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            for axis in ["width", "height"]:
                with self.subTest(mode=mode, axis=axis):
                    rules = f"section{{writing-mode:{mode};aspect-ratio:1;{axis}:{size}pt;text-align:end;background:red}}"
                    body = "<section>A</section><p>Z</p>"
                    ratio = self.render("horizontal-tb", body, rules)["glyphs"]
                    definite = self.render("horizontal-tb", body, rules + f"section{{width:{size}pt;height:{size}pt}}")["glyphs"]
                    self.assertEqual([g.text for g in ratio], [g.text for g in definite])
                    image = self.render("horizontal-tb", body, rules, pixels=True)
                    reference = self.render("horizontal-tb", body, rules + f"section{{width:{size}pt;height:{size}pt}}", pixels=True)
                    self.assertIsNone(ImageChops.difference(image, reference).getbbox(),
                                      "the preferred-ratio principal background and text match a definite square")
                    for actual, expected in zip(ratio, definite):
                        for value, reference in zip(actual.matrix, expected.matrix):
                            self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                                   msg=f"{mode}/{axis}: preferred ratio transfers a definite axis before intrinsic shrink fitting and aligns within the resulting box")

    def test_vertical_font_baseline_table_controls_ideographic_ink_edges(self):
        font = ROOT / "tests/fixtures/fonts/CJKWritingTest.otf"
        with TTFont(font) as face:
            axis = face["BASE"].table.VertAxis
            values = axis.BaseScriptList.BaseScriptRecord[0].BaseScript.BaseValues.BaseCoord
            baselines = {tag:coord.Coordinate / face["head"].unitsPerEm
                         for tag, coord in zip(axis.BaseTagList.BaselineTag, values)}
        rules = (f"@font-face{{font-family:J;src:url('{font.as_uri()}')}}body{{font-family:J}}"
                 "p{text-box-trim:trim-both;text-box-edge:ideographic-ink}")
        for mode in ["vertical-rl", "vertical-lr"]:
            with self.subTest(mode=mode):
                glyph = self.render(mode, "<p>漢</p>", rules)["glyphs"][0]
                edge = glyph.matrix[4] + baselines["icft" if mode == "vertical-rl" else "icfb"] * FONT_PT
                expected = PAGE_PT - MARGIN_PT if mode == "vertical-rl" else MARGIN_PT
                self.assertAlmostEqual(edge, expected, delta=TOLERANCE_PT,
                                       msg="vertical BASE coordinates, rather than horizontal ink baselines, anchor the trimmed content edge")

    def test_explicit_vertical_line_height_uses_primary_box_metrics(self):
        for mode in ["vertical-rl", "vertical-lr"]:
            for orientation in ["mixed", "upright"]:
                with self.subTest(mode=mode, orientation=orientation):
                    glyphs = self.render(mode, "<p>A</p><p>A</p>", f"body{{text-orientation:{orientation}}}")["glyphs"]
                    self.assertEqual([g.text for g in glyphs], list("AA"))
                    self.assertAlmostEqual(abs(glyphs[1].matrix[4] - glyphs[0].matrix[4]), FONT_PT,
                                           delta=TOLERANCE_PT, msg="line-height:1 allocates one primary-font em; aligning a sideways glyph's central baseline does not grow that box")

    def test_sideways_text_orientation_uses_horizontal_typographic_baselines(self):
        sideways = self.render("sideways-rl")["glyphs"]
        vertical = self.render("vertical-rl", rules="body{text-orientation:sideways}")["glyphs"]
        self.assertEqual([g.text for g in vertical], [g.text for g in sideways])
        for actual, expected in zip(vertical, sideways):
            for value, reference in zip(actual.matrix, expected.matrix):
                self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                       msg=f"{actual.text}: sideways text uses the alphabetic baseline and horizontal font metrics in either writing mode")

    def test_upright_vertical_text_keeps_each_latin_glyph_upright(self):
        for mode, sign in [("vertical-rl", -1), ("vertical-lr", 1)]:
            with self.subTest(mode=mode):
                glyphs = self.render(mode, rules="body{text-orientation:upright}")["glyphs"]
                first, second, third, _ = [glyph.matrix for glyph in glyphs]
                self.assertGreater(first[0], 0, str(first))
                self.assertAlmostEqual(first[1], 0, delta=TOLERANCE_PT, msg=str(first))
                with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
                    cmap = font.getBestCmap()
                    advances = [font["hmtx"].metrics[cmap[ord(ch)]][0] * FONT_PT / font["head"].unitsPerEm
                                for ch in "AB"]
                centres = [matrix[4] + advance / 2 for matrix, advance in zip([first, second], advances)]
                self.assertAlmostEqual(*centres, delta=TOLERANCE_PT,
                                       msg=f"upright glyph advance boxes share the column centre: {first}, {second}")
                self.assertLess(second[5], first[5], "upright text progresses downwards")
                self.assertGreater(sign * (third[4] - first[4]), 0,
                                   f"column progression: {first}, {third}")

    def test_upright_uses_ltr_flow_without_changing_inherited_direction(self):
        ltr = self.render("vertical-rl", "<p>AB</p>", "body{text-orientation:upright}")["glyphs"]
        rtl = self.render("vertical-rl", "<p>AB</p>", "body{text-orientation:upright;direction:rtl}")["glyphs"]
        self.assertEqual([glyph.text for glyph in rtl], [glyph.text for glyph in ltr])
        for actual, expected in zip(rtl, ltr):
            for a, b in zip(actual.matrix, expected.matrix):
                self.assertAlmostEqual(a, b, delta=TOLERANCE_PT,
                                       msg="upright uses LTR placement even when direction computes to RTL")

    def test_escaped_flex_and_grid_static_alignment_maps_both_axes_and_edge_polarities(self):
        for display in ["flex", "grid"]:
            for mode, origin in [("vertical-rl",(20,70)), ("vertical-lr",(60,70)),
                                 ("sideways-rl",(20,70)), ("sideways-lr",(60,20))]:
                with self.subTest(display=display, mode=mode):
                    alignment = ("justify-content:flex-end;align-items:flex-end" if display == "flex"
                                 else "grid-template:1fr/1fr;justify-items:end;align-items:end")
                    rules = (f"body{{position:relative;width:180pt;height:180pt}}section{{writing-mode:{mode};display:{display};width:60pt;height:80pt;{alignment}}}"
                             "i{position:absolute;writing-mode:horizontal-tb;width:20pt;height:30pt;background:red}")
                    image = self.render("horizontal-tb", "<section><i></i></section>", rules, pixels=True)
                    red = [(x,y) for y in range(PAGE_PT) for x in range(PAGE_PT)
                           if image.getpixel((x,y)) == (255,0,0)]
                    self.assertTrue(red, "the aligned positioned box must paint")
                    x,y = origin
                    self.assertEqual((min(px for px,py in red), min(py for px,py in red),
                                      max(px for px,py in red), max(py for px,py in red)),
                                     (x,y,x+19,y+29),
                                     "static-position main/cross alignment follows the orthogonal container's physical end edges")

    def test_sticky_uses_an_external_scrollport_in_physical_coordinates(self):
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                lead = 60 if mode == "sideways-lr" else 100
                rules = (f"main{{width:120pt;height:80pt;overflow:hidden}}section{{writing-mode:{mode};width:100pt;height:180pt}}"
                         f".lead{{display:inline-block;inline-size:{lead}pt;block-size:10pt}}"
                         ".s{display:inline-block;position:sticky;bottom:0;width:20pt;height:20pt;background:red}")
                image = self.render("horizontal-tb", "<main><section><p><span class=lead></span><span class=s></span></p></section></main>", rules, pixels=True)
                red = [(x,y) for y in range(PAGE_PT) for x in range(PAGE_PT)
                       if image.getpixel((x,y)) == (255,0,0)]
                self.assertTrue(red, "the sticky box must shift into the external scrollport instead of being clipped away")
                self.assertEqual((min(y for x,y in red), max(y for x,y in red)), (80,99),
                                 "bottom:0 must align its physical bottom with the 80pt external scrollport at y=20pt")

    def test_rtl_start_alignment_reverses_the_inline_start_edge_in_each_mode(self):
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            width = sum(font["hmtx"].metrics[cmap[ord(ch)]][0] for ch in "AB") * FONT_PT / font["head"].unitsPerEm
        extra = PAGE_PT - 2 * MARGIN_PT - width
        for mode, sign in [("vertical-rl",-1), ("vertical-lr",-1),
                           ("sideways-rl",-1), ("sideways-lr",1)]:
            with self.subTest(mode=mode):
                ltr = self.render(mode, "<p>AB</p>")["glyphs"]
                rtl = self.render(mode, "<p>AB</p>", "body{direction:rtl}")["glyphs"]
                self.assertEqual([g.text for g in rtl], [g.text for g in ltr])
                for first, second in zip(ltr, rtl):
                    self.assertEqual(first.matrix[:4], second.matrix[:4], "direction does not change glyph orientation")
                    self.assertAlmostEqual(first.matrix[4], second.matrix[4], delta=TOLERANCE_PT)
                    self.assertAlmostEqual(second.matrix[5] - first.matrix[5], sign * extra,
                                           delta=TOLERANCE_PT, msg="text-align:start uses the mode's reversed RTL inline edge")

    def test_external_images_keep_physical_size_and_upright_content(self):
        svg = '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" viewBox="0 0 40 20"><path fill="red" d="M0 0h20v20H0z"/><path fill="blue" d="M20 0h20v20H20z"/></svg>'
        url = "data:image/svg+xml;base64," + base64.b64encode(svg.encode()).decode()
        for mode, origin in [("vertical-rl", (180,20)), ("sideways-rl", (180,20)),
                             ("vertical-lr", (20,20)), ("sideways-lr", (20,200))]:
            with self.subTest(mode=mode):
                image = self.render(mode, f'<img src="{url}">',
                                    "img{display:block;width:40pt;height:20pt}", pixels=True)
                x,y = origin
                self.assertEqual(image.getpixel((x+10,y+10)), (255,0,0),
                                 "the image's physical left half must remain red")
                self.assertEqual(image.getpixel((x+30,y+10)), (0,0,255),
                                 "the image's physical right half must remain blue")
                self.assertEqual(image.getpixel((x+10,y+25)), (255,255,255),
                                 "the physical height remains 20pt after flow-axis mapping")

    def test_body_writing_mode_propagates_to_the_root_pagination_flow(self):
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                inherited = self.render(mode)["glyphs"]
                body = self.render("horizontal-tb", rules=f"body{{writing-mode:{mode}}}")["glyphs"]
                self.assertEqual([(g.text, g.page, g.matrix) for g in body],
                                 [(g.text, g.page, g.matrix) for g in inherited],
                                 "the first body's used mode must govern the principal root flow")

    def test_physical_translation_does_not_turn_with_the_text_flow(self):
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                original = self.render(mode, "<p>AB</p>")["glyphs"]
                shifted = self.render(mode, "<p>AB</p>", "p{transform:translate(7pt,11pt)}")["glyphs"]
                for first, second in zip(original, shifted):
                    self.assertEqual(first.text, second.text)
                    self.assertAlmostEqual(second.matrix[4] - first.matrix[4], 7,
                                           delta=TOLERANCE_PT, msg="CSS translate X remains physical horizontal")
                    self.assertAlmostEqual(second.matrix[5] - first.matrix[5], -11,
                                           delta=TOLERANCE_PT, msg="CSS translate Y remains physical vertical")

    def test_table_cells_keep_the_table_grid_when_their_content_is_orthogonal(self):
        body = "<table><tr><td>A</td><td>B</td></tr><tr><td>C</td><td>D</td></tr></table>"
        rules = "table{border-spacing:0}td{padding:0;width:40pt;height:40pt;writing-mode:horizontal-tb}"
        glyphs = self.render("vertical-rl", body, rules)["glyphs"]
        text = {glyph.text: glyph.matrix for glyph in glyphs}
        self.assertEqual(set(text), set("ABCD"), "every orthogonal table cell must survive normalization")
        self.assertAlmostEqual(text["B"][4], text["A"][4], delta=TOLERANCE_PT)
        self.assertLess(text["B"][5], text["A"][5], "cells advance down the vertical table inline axis")
        self.assertLess(text["C"][4], text["A"][4], "rows advance towards the left")
        for matrix in text.values():
            self.assertGreater(matrix[0], 0, "cell glyphs keep their independent horizontal orientation")
            self.assertAlmostEqual(matrix[1], 0, delta=TOLERANCE_PT)

    def test_inter_character_ruby_adds_an_upright_vertical_column_after_its_base(self):
        font = ROOT / "tests/fixtures/fonts/CJKWritingTest.otf"
        rules = (f"@font-face{{font-family:J;src:url('{font.as_uri()}')}}"
                 "body{font-family:J}ruby{ruby-position:inter-character}rt{line-height:1}")
        glyphs = self.render("horizontal-tb", "<p><ruby>漢<rt>あア</rt></ruby>字</p>", rules)["glyphs"]
        text = {glyph.text: glyph.matrix for glyph in glyphs}
        self.assertEqual(set(text), set("漢あア字"))
        self.assertAlmostEqual(text["字"][4] - text["漢"][4], FONT_PT * 1.5,
                               delta=TOLERANCE_PT, msg="the annotation owns a separate half-em column")
        for ch in "あア":
            self.assertAlmostEqual(text[ch][4] - text["漢"][4], FONT_PT, delta=TOLERANCE_PT)
            self.assertGreater(text[ch][0], 0, f"{ch}: the vertical annotation stays upright")
            self.assertAlmostEqual(text[ch][1], 0, delta=TOLERANCE_PT)
        self.assertLess(text["ア"][5], text["あ"][5], "annotation glyphs progress downwards")
        self.assertAlmostEqual(text["字"][5], text["漢"][5], delta=TOLERANCE_PT)

    def test_inter_character_ruby_centres_without_justification_opportunities(self):
        font = ROOT / "tests/fixtures/fonts/CJKWritingTest.otf"
        rules = (f"@font-face{{font-family:J;src:url('{font.as_uri()}')}}"
                 "body{font-family:J}ruby{ruby-position:inter-character}rt{line-height:1}")
        body = "<p><ruby>漢<rt>あ</rt></ruby>字</p>"
        centred = {g.text:g.matrix for g in self.render("horizontal-tb", body, rules + "rt{ruby-align:center}")["glyphs"]}
        for align in ["space-between", "space-around"]:
            with self.subTest(align=align):
                actual = {g.text:g.matrix for g in self.render("horizontal-tb", body, rules + f"rt{{ruby-align:{align}}}")["glyphs"]}
                for value, reference in zip(actual["あ"], centred["あ"]):
                    self.assertAlmostEqual(value, reference, delta=TOLERANCE_PT,
                                           msg=f"ruby-align:{align} centres an annotation with no internal justification opportunities")

    def test_inter_character_ruby_space_around_reserves_half_gaps_at_both_ends(self):
        font = ROOT / "tests/fixtures/fonts/CJKWritingTest.otf"
        rules = (f"@font-face{{font-family:J;src:url('{font.as_uri()}')}}"
                 f"body{{font-family:J}}ruby{{ruby-position:inter-character}}rt{{font-size:{FONT_PT / 4}pt;line-height:1}}")
        body = "<p><ruby>漢<rt>あア</rt></ruby>字</p>"
        between = {g.text:g.matrix for g in self.render("horizontal-tb", body, rules + "rt{ruby-align:space-between}")["glyphs"]}
        around = {g.text:g.matrix for g in self.render("horizontal-tb", body, rules + "rt{ruby-align:space-around}")["glyphs"]}
        leading = between["あ"][5] - around["あ"][5]
        trailing = around["ア"][5] - between["ア"][5]
        self.assertGreater(leading, TOLERANCE_PT, "space-around moves the first upright annotation glyph down from the leading edge")
        self.assertAlmostEqual(leading, trailing, delta=TOLERANCE_PT,
                               msg="space-around reserves equal half gaps before and after the annotation")

    def test_inter_character_ruby_is_over_in_a_vertical_base_flow(self):
        font = ROOT / "tests/fixtures/fonts/CJKWritingTest.otf"
        rules = f"@font-face{{font-family:J;src:url('{font.as_uri()}')}}body{{font-family:J}}"
        body = "<p><ruby>漢<rt>あア</rt></ruby>字</p>"
        over = self.render("vertical-rl", body, rules + "ruby{ruby-position:over}")["glyphs"]
        inter = self.render("vertical-rl", body, rules + "ruby{ruby-position:inter-character}")["glyphs"]
        self.assertEqual([(glyph.text, glyph.matrix) for glyph in inter],
                         [(glyph.text, glyph.matrix) for glyph in over],
                         "inter-character has the same effect as over in a vertical ruby container")

    def test_fieldset_legend_cuts_the_physical_block_start_border(self):
        rules = ("fieldset{margin:0;padding:0;box-sizing:border-box;width:100pt;height:80pt;"
                 "border-style:solid;border-width:7pt 5pt 9pt 3pt;"
                 "border-color:blue red blue green}legend{padding:0;margin:0;inline-size:20pt}")
        for mode, gap, solid, colour in [
            ("vertical-rl", (210, 35), (210, 60), (255, 0, 0)),
            ("sideways-rl", (210, 35), (210, 60), (255, 0, 0)),
            ("vertical-lr", (30, 35), (30, 60), (0, 128, 0)),
            ("sideways-lr", (30, 200), (30, 180), (0, 128, 0)),
        ]:
            with self.subTest(mode=mode):
                image = self.render(mode, "<fieldset><legend>A</legend><p>B</p></fieldset>", rules, pixels=True)
                self.assertEqual(image.getpixel(solid), colour,
                                 f"{mode}: the asymmetric block-start border must be present away from the legend")
                self.assertNotEqual(image.getpixel(gap), colour,
                                    f"{mode}: the legend cuts that border, including reversed inline progression")

    def test_combined_digits_are_horizontal_compressed_and_consume_one_vertical_em(self):
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            advance = font["hmtx"].metrics[cmap[ord("A")]][0] * FONT_PT / font["head"].unitsPerEm
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                glyphs = self.render(mode, "<p>A<span>1234</span>B</p>",
                                     "span{text-combine-upright:all}")["glyphs"]
                by_text = {glyph.text: glyph.matrix for glyph in glyphs}
                self.assertEqual(set(by_text), set("A1234B"))
                first = by_text["1"]
                self.assertGreater(first[0], 0, str(first))
                self.assertAlmostEqual(first[1], 0, delta=TOLERANCE_PT)
                self.assertAlmostEqual(first[0], FONT_PT / 2, delta=TOLERANCE_PT,
                                       msg="four half-em figures compress horizontally to one em")
                for previous, current in zip("123", "234"):
                    self.assertGreater(by_text[current][4], by_text[previous][4], str(by_text))
                    self.assertAlmostEqual(by_text[current][5], first[5], delta=TOLERANCE_PT)
                sign = 1 if mode == "sideways-lr" else -1
                self.assertAlmostEqual(sign * (by_text["B"][5] - by_text["A"][5]),
                                       advance + FONT_PT, delta=TOLERANCE_PT,
                                       msg="combined text occupies one em of inline advance")

    def test_svg_text_combines_upright_as_document_text_does(self):
        """CSS Writing Modes 4 §9.1 in vertical SVG text, which SVG 2 §11.10 lays out with the
        document's text: a combination composes horizontally, compresses to one em and takes one
        em of the column, and an overlong `digits` sequence stays sideways."""
        with TTFont(ROOT / "assets/fonts/LiberationSerif-Regular.ttf") as font:
            cmap = font.getBestCmap()
            advance = font["hmtx"].metrics[cmap[ord("A")]][0] * FONT_PT / font["head"].unitsPerEm
        # One user unit is one point, so the font size is FONT_PT points. Each text renders
        # alone: a glyph extracts as the text of the first cluster drawing it.
        def render(text):
            return self.render(
                "horizontal-tb",
                f"<svg width='200pt' height='200pt' viewBox='0 0 200 200' "
                f"style='display:block;font:{FONT_PT}px serif;writing-mode:vertical-rl'>"
                f"{text}</svg>")["glyphs"]

        combined = render("<text x='50' y='10'>A<tspan style='text-combine-upright:all'>1234"
                          "</tspan>B</text>")
        self.assertEqual([glyph.text for glyph in combined], list("A1234B"),
                         "each glyph of the combination keeps its own text")
        a, *figures, b = [glyph.matrix for glyph in combined]
        first = figures[0]
        self.assertAlmostEqual(first[1], 0, delta=TOLERANCE_PT, msg=f"upright: {first}")
        self.assertAlmostEqual(first[0], FONT_PT / 2, delta=TOLERANCE_PT,
                               msg="four half-em figures compress horizontally to one em")
        for previous, current in zip(figures, figures[1:]):
            self.assertAlmostEqual(current[4] - previous[4], FONT_PT / 4, delta=TOLERANCE_PT,
                                   msg="compressed half-em figures advance a quarter em")
            self.assertAlmostEqual(current[5], first[5], delta=TOLERANCE_PT)
        self.assertAlmostEqual(a[5] - b[5], advance + FONT_PT,
                               delta=TOLERANCE_PT, msg="combined text occupies one em of the column")
        self.assertAlmostEqual(first[4] + FONT_PT / 2, 50 + MARGIN_PT, delta=TOLERANCE_PT,
                               msg="the composition centres on the column")
        digits = [glyph for glyph in render("<text x='50' y='10' style='text-combine-upright:digits 2'>"
                                            "12 123 4</text>") if glyph.text != " "]
        self.assertEqual([glyph.text for glyph in digits], list("121234"))
        self.assertGreater(digits[0].matrix[0], 0, "two digits combine upright")
        self.assertAlmostEqual(digits[2].matrix[0], 0, delta=TOLERANCE_PT,
                               msg="an overlong maximal sequence must remain uncombined")
        self.assertGreater(digits[-1].matrix[0], 0, "a lone digit is an upright combination")

    def test_svg_text_orders_bidi_embeddings_isolates_and_overrides_as_document_text_does(self):
        """CSS Writing Modes 4 §2.4 and SVG 2 §11.10.2.4: `unicode-bidi` on a text element and
        its tspans embeds, isolates and overrides as on the document's inline boxes, so the
        glyphs of each SVG text line take the visual order of the same paragraph."""
        cases = [
            ("", "abc <b style='unicode-bidi:bidi-override;direction:rtl'>def</b> ghi"),
            ("", "abc <b style='unicode-bidi:isolate;direction:rtl'>x אבג y</b> 12"),
            ("direction:rtl", "אב <b style='unicode-bidi:embed;direction:ltr'>1-2 c</b> גד"),
            ("unicode-bidi:bidi-override;direction:rtl", "abc <b>de</b>"),
            ("", "ab <b style='unicode-bidi:isolate-override;direction:rtl'>cd <i>ef</i></b> gh"),
            ("unicode-bidi:plaintext", "אב ab"),
            ("", "ab <b style='unicode-bidi:plaintext'>אב cd</b> ef"),
        ]
        line = 30
        paragraphs = "".join(f"<p style=\"{style}\">{text}</p>" for style, text in cases)
        texts = ""
        for index, (style, text) in enumerate(cases):
            for tag in ("b", "i"):
                text = text.replace(f"<{tag}", "<tspan").replace(f"</{tag}>", "</tspan>")
            # A right-to-left line's start is its right side.
            start = 190 if "rtl" in style else 10
            texts += f"<text x='{start}' y='{(index + 1) * line}' style=\"{style}\">{text}</text>"
        rules = "b,i{font:inherit}body{font-family:'DejaVu Sans'}"
        document = self.render("horizontal-tb", paragraphs, rules)["glyphs"]
        svg = self.render(
            "horizontal-tb",
            f"<svg width='200pt' height='{line * (len(cases) + 1)}pt' "
            f"viewBox='0 0 200 {line * (len(cases) + 1)}' style='display:block;font:{FONT_PT}px "
            f"\"DejaVu Sans\"'>{texts}</svg>", rules)["glyphs"]

        def lines(glyphs):
            rows = {}
            for glyph in glyphs:
                rows.setdefault(round(glyph.matrix[5], 2), []).append(glyph)
            # The document paints no glyphs for spaces.
            return ["".join(glyph.text for glyph in sorted(rows[y], key=lambda glyph: glyph.matrix[4])
                            if glyph.text != " ")
                    for y in sorted(rows, reverse=True)]

        expected = lines(document)
        self.assertEqual(len(expected), len(cases), expected)
        self.assertEqual(lines(svg), expected, "SVG text lines take the paragraphs' visual order")
        self.assertEqual(expected[0], "abcfedghi", "the override reverses its box only")

    def test_digits_only_combination_leaves_overlong_sequences_sideways(self):
        glyphs = self.render("vertical-rl", "<p>12 123 4</p>",
                             "body{text-combine-upright:digits 2}")["glyphs"]
        digits = [glyph for glyph in glyphs if glyph.text.isdigit()]
        self.assertEqual([glyph.text for glyph in digits], list("121234"))
        self.assertGreater(digits[0].matrix[0], 0, "two digits combine upright")
        self.assertAlmostEqual(digits[2].matrix[0], 0, delta=TOLERANCE_PT,
                               msg="an overlong maximal sequence must remain uncombined")
        self.assertGreater(digits[-1].matrix[0], 0, "a lone digit is an upright combination")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
