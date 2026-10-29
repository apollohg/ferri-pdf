"""CSS Text tab stops, checked against independent font metrics and PDF glyph origins."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import unittest

from fontTools.ttLib import TTFont
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
FONT = ROOT / "assets/fonts/DejaVuSansMono.ttf"
SIZE = 12
MARGIN = 20
TOLERANCE = 0.01


class TabsPdf(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="ferri-tabs-")
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)
        with TTFont(FONT) as font:
            self.space = font["hmtx"].metrics[font.getBestCmap()[ord(" ")]][0] * SIZE / font["head"].unitsPerEm

    def glyphs(self, css, body):
        source, target = self.directory / "input.html", self.directory / "output.pdf"
        source.write_text(f"<!doctype html><style>@font-face{{font-family:TabTest;src:url('{FONT.as_uri()}')}}"
                          f"@page{{size:300pt 400pt;margin:{MARGIN}pt}}body{{margin:0}}"
                          f"p{{margin:0;font:{SIZE}pt/24pt TabTest;white-space:pre}}{css}</style>{body}")
        result = subprocess.run([str(RENDERER), str(source), str(target)], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        return pdf_content.inspect(target)["glyphs"]

    def positions(self, css, body):
        return [g.matrix[4] - MARGIN for g in self.glyphs(css, body) if g.text == "Z"]

    def assert_positions(self, actual, expected):
        self.assertEqual(len(actual), len(expected), actual)
        for observed, wanted in zip(actual, expected):
            self.assertAlmostEqual(observed, wanted, delta=TOLERANCE,
                                   msg=f"tab target origins {actual}; expected {expected}")

    def test_number_uses_stops_instead_of_fixed_spaces(self):
        self.assert_positions(self.positions("p{tab-size:4}", "<p>A\tZ<br>AAA\tZ<br>AAAA\tZ</p>"),
                              [4*self.space, 4*self.space, 8*self.space])

    def test_fractional_and_length_intervals(self):
        for value, interval in [("3.5", 3.5*self.space), ("30pt", 30), ("calc(20pt + 10pt)", 30)]:
            with self.subTest(value=value):
                self.assert_positions(self.positions(f"p{{tab-size:{value}}}", "<p>A\tZ<br>AAA\tZ</p>"),
                                      [interval, interval])

    def test_indent_and_inline_edges_count_from_block_start(self):
        self.assert_positions(self.positions("p{tab-size:40pt;text-indent:10pt}span{padding-left:5pt}",
                                             "<p><span>A</span>\tZ</p>"), [40])

    def test_near_stop_advances_to_subsequent_stop(self):
        interval = 3.25*self.space
        self.assert_positions(self.positions(f"p{{tab-size:{interval}pt}}", "<p>AAA\tZ</p>"), [2*interval])

    def test_numeric_interval_uses_block_font_and_spacing(self):
        self.assert_positions(self.positions("p{tab-size:4;letter-spacing:1pt;word-spacing:2pt}span{font-size:6pt}",
                                             "<p><span>A\tZ</span></p>"), [4*(self.space+3)])

    def test_computed_lengths_inherit_without_rescaling(self):
        self.assert_positions(self.positions("p{tab-size:3em}span{font-size:6pt}",
                                             "<p><span>A\tZ</span></p>"), [3*SIZE])

    def test_wrapped_line_restarts_tab_stops(self):
        self.assert_positions(self.positions("p{white-space:pre-wrap;width:45pt;tab-size:30pt}",
                                             "<p>A\tZ A\tZ</p>"), [30, 30])

    def test_float_band_keeps_block_tab_origin(self):
        self.assert_positions(self.positions("p{tab-size:40pt}.float{float:left;width:10pt;height:20pt}",
                                             "<div class=float></div><p>A\tZ</p>"), [40])

    def test_rtl_tabs_advance_from_right_block_edge(self):
        width = 100
        glyphs = self.glyphs(f"p{{direction:rtl;unicode-bidi:bidi-override;width:{width}pt;tab-size:30pt}}", "<p>A\tZ</p>")
        target = [g.matrix[4] - MARGIN for g in glyphs if g.text == "Z"]
        self.assert_positions(target, [width - 30 - self.space])

    def test_vertical_tabs_follow_inline_axis(self):
        for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]:
            with self.subTest(mode=mode):
                glyphs = self.glyphs(f"p{{writing-mode:{mode};height:100pt;tab-size:30pt}}", "<p>A\tZ</p>")
                a, z = ([g for g in glyphs if g.text == ch][0] for ch in "AZ")
                self.assertAlmostEqual(abs(a.matrix[5] - z.matrix[5]), 30, delta=TOLERANCE)

    def test_intrinsic_width_includes_resolved_tab_advance(self):
        self.assert_positions(self.positions("p{display:inline-block;tab-size:30pt}.after{font:12pt TabTest}",
                                             "<p>A\tZ</p><span class=after>Z</span>"), [30, 30+self.space])

    def test_break_spaces_wraps_after_each_preserved_tab(self):
        self.assert_positions(self.positions("p{white-space:break-spaces;width:31pt;tab-size:30pt}",
                                             "<p>A\tZ</p>"), [0])

    def test_link_and_transform_keep_source_after_tabs(self):
        self.assert_positions(self.positions("p{tab-size:30pt;text-transform:uppercase}a{color:red}",
                                             "<p>a\t<a href='#target'>z</a></p><p id=target>Target</p>"), [30])
        links = pdf_content.navigation(self.directory / "output.pdf")["links"]
        self.assertEqual(len(links), 1, links)
        self.assertAlmostEqual(links[0]["rect"][0], MARGIN+30, delta=TOLERANCE)
        self.assertIsNotNone(links[0]["destination"], links)

    def test_justification_keeps_tab_grid_and_fills_remaining_text(self):
        glyphs = self.glyphs("p{tab-size:30pt;width:100pt;text-align:justify-all;text-justify:inter-character}",
                             "<p>A\tZQ</p>")
        self.assert_positions([g.matrix[4]-MARGIN for g in glyphs if g.text == "Z"], [30])
        self.assert_positions([g.matrix[4]-MARGIN for g in glyphs if g.text == "Q"], [100-self.space])

    def test_zero_tab_keeps_break_opportunity(self):
        self.assert_positions(self.positions("p{white-space:break-spaces;width:8pt;tab-size:0}",
                                             "<p>A\tZ</p>"), [0])

    def test_trimmed_opening_punctuation_precedes_tab_resolution(self):
        font = ROOT / "tests/fixtures/fonts/CJKSpacingTest.otf"
        self.assert_positions(self.positions(
            f"@font-face{{font-family:CJK;src:url('{font.as_uri()}')}}"
            "p{tab-size:30pt;text-spacing-trim:trim-start}span{font:10pt CJK}",
            "<p><span>「</span>\tZ</p>"), [30])

    def test_zero_tabs_alone_do_not_create_a_line(self):
        for body in ["<p>A</p><p>\t</p><p>Z</p>", "<p>A</p><p></p><p>Z</p>"]:
            glyphs = self.glyphs("p{tab-size:0}", body)
            a, z = ([g for g in glyphs if g.text == ch][0] for ch in "AZ")
            self.assertAlmostEqual(a.matrix[5] - z.matrix[5], 24, delta=TOLERANCE)

    def test_rtl_justification_preserves_tab_stop(self):
        glyphs = self.glyphs("p{direction:rtl;unicode-bidi:bidi-override;tab-size:30pt;width:100pt;"
                             "text-align:justify-all;text-justify:inter-character}", "<p>A\tZQ</p>")
        self.assert_positions([g.matrix[4]-MARGIN for g in glyphs if g.text == "Z"], [100-30-self.space])
        self.assert_positions([g.matrix[4]-MARGIN for g in glyphs if g.text == "Q"], [0])

    def test_combined_text_resolves_horizontal_tabs_before_compression(self):
        for interval in [24, 48]:
            with self.subTest(interval=interval):
                glyphs = self.glyphs(f"p{{writing-mode:vertical-rl;height:100pt;tab-size:{interval}pt}}"
                                     "span{text-combine-upright:all}", "<p><span>A\tZ</span></p>")
                a, z = ([g for g in glyphs if g.text == ch][0] for ch in "AZ")
                self.assertAlmostEqual(a.matrix[5], z.matrix[5], delta=TOLERANCE)
                self.assertAlmostEqual(z.matrix[4]-a.matrix[4], interval*SIZE/(interval+self.space), delta=TOLERANCE)

    def test_trailing_tabs_hang_only_at_soft_or_overflowing_line_ends(self):
        cases = [("A\tZZZZZZZZZZZZZZZZZZZZ", 30, 100-self.space),
                 ("A\t", 30, 70), ("A<span>\t</span>", 130, 0)]
        for text, interval, expected in cases:
            with self.subTest(text=text, interval=interval):
                glyphs = self.glyphs(f"p{{tab-size:{interval}pt;white-space:pre-wrap;text-align:right;width:100pt}}",
                                     f"<p>{text}</p>")
                self.assert_positions([g.matrix[4]-MARGIN for g in glyphs if g.text == "A"], [expected])

    def test_inline_padding_prevents_trailing_tab_hanging(self):
        glyphs = self.glyphs("p{tab-size:30pt;white-space:pre-wrap;text-align:right;width:100pt}"
                             "span{padding-right:10pt}", "<p><span>A\t</span>ZZZZZZZZZZZZZZZZZZZZ</p>")
        self.assert_positions([g.matrix[4]-MARGIN for g in glyphs if g.text == "A"], [60])

    def test_pre_wrap_max_content_includes_final_tab(self):
        self.assert_positions(self.positions("p{display:inline-block;white-space:pre-wrap;tab-size:30pt}"
                                             ".after{font:12pt TabTest}",
                                             "<p>A\t</p><span class=after>Z</span>"), [30])

    def test_negative_trailing_space_advance_does_not_hang_or_panic(self):
        glyphs = self.glyphs("p{white-space:pre-wrap;width:100pt;word-spacing:-20pt}",
                             "<p>ZZZZZZZZZZZZZZZZZZZZ </p>")
        positions = [g.matrix[4]-MARGIN for g in glyphs if g.text == "Z"]
        self.assert_positions(positions, [index*self.space for index in range(20)])

    def test_collapsed_and_zero_tabs(self):
        self.assert_positions(self.positions("p{white-space:normal;tab-size:30pt}", "<p>A\tZ</p>"), [2*self.space])
        for value in ["0", "0pt"]:
            self.assert_positions(self.positions(f"p{{tab-size:{value}}}", "<p>A\tZ</p>"), [self.space])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    args = parser.parse_args()
    RENDERER = args.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
