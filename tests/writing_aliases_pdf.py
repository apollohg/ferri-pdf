"""Writing Modes 4 §§3.2.1/5.1.3 compatibility through native and SVG PDF text."""
import argparse
import base64
from pathlib import Path
import tempfile
import unittest

from PIL import ImageChops
import svg_css_pdf as svg
import writing_modes_pdf as writing

RENDERER = writing.ROOT / "target/debug/ferri-pdf"
ALIASES = [(name, "horizontal-tb") for name in ["lr", "lr-tb", "rl", "rl-tb"]]
ALIASES += [(name, "vertical-rl") for name in ["tb", "tb-rl"]]
ORIENTATIONS = [("auto", "mixed"), ("0", "upright"), ("0deg", "upright"),
                ("90", "sideways"), ("90deg", "sideways")]
SVG_TEXT = '<text x="60" y="50" font-size="24" font-family="serif" {attributes}>AB</text>'
SVG_DOCUMENT = '<svg xmlns="http://www.w3.org/2000/svg" width="180" height="180">{content}</svg>'


class WritingAliasesPdf(unittest.TestCase):
    def setUp(self):
        writing.RENDERER = RENDERER
        svg.RENDERER = RENDERER

    def native(self, declaration, direction="ltr"):
        return writing.WritingModesPdf.render(
            self, "horizontal-tb", "<section>A B C D</section><p>Z</p>",
            f"section{{width:60pt;direction:{direction};{declaration}}}")["glyphs"]

    def assertGlyphs(self, actual, expected, label):
        self.assertEqual([g.text for g in actual], [g.text for g in expected], label)
        for glyph, reference in zip(actual, expected):
            for value, expected_value in zip(glyph.matrix, reference.matrix):
                self.assertAlmostEqual(value, expected_value, delta=writing.TOLERANCE_PT,
                                       msg=f"{label}: {glyph.text} matrix {glyph.matrix} != {reference.matrix}")

    def image(self, attributes, css="", context="inline"):
        content = SVG_TEXT.format(attributes=attributes)
        document = SVG_DOCUMENT.format(content=f"<style>{css}</style>{content}")
        with tempfile.TemporaryDirectory(prefix="ferri-writing-aliases-") as directory:
            root = Path(directory)
            if context == "inline":
                body = document
            elif context == "external":
                source = root / "text.svg"
                source.write_text(document)
                body = f'<img src="{source.as_uri()}">'
            else:
                encoded = base64.b64encode(document.encode()).decode()
                body = f'<img src="data:image/svg+xml;base64,{encoded}">'
            return svg.SvgCssPdf.render(self, root, "", body)

    def convert(self, root, css, body):
        return svg.SvgCssPdf.convert(self, root, css, body)

    def assertInk(self, actual, expected, label):
        self.assertIsNone(ImageChops.difference(actual, expected).getbbox(), label)

    def test_native_css_aliases_preserve_the_modern_flow_and_direction(self):
        for alias, modern in ALIASES:
            for direction in ["ltr", "rtl"]:
                with self.subTest(alias=alias, direction=direction):
                    self.assertGlyphs(self.native(f"writing-mode:{alias}", direction),
                                      self.native(f"writing-mode:{modern}", direction),
                                      "obsolete writing-mode does not invent a direction change")

    def test_native_glyph_shorthand_matches_modern_orientation(self):
        for alias, modern in ORIENTATIONS:
            with self.subTest(alias=alias):
                self.assertGlyphs(self.native(f"writing-mode:vertical-rl;glyph-orientation-vertical:{alias}"),
                                  self.native(f"writing-mode:vertical-rl;text-orientation:{modern}"),
                                  "the glyph shorthand changes the same native text orientation")

    def test_svg_modern_orientation_has_a_visible_effect_in_each_loading_context(self):
        for context in ["inline", "external", "data"]:
            with self.subTest(context=context):
                upright = self.image('writing-mode="vertical-rl" text-orientation="upright"', context=context)
                sideways = self.image('writing-mode="vertical-rl" text-orientation="sideways"', context=context)
                self.assertIsNotNone(ImageChops.difference(upright, sideways).getbbox(),
                                     "upright Latin and sideways Latin must paint differently")

    def test_svg_upright_uses_the_fonts_vertical_advance(self):
        font = writing.ROOT / "tests/fixtures/fonts/CJKWritingTest.otf"
        rules = f"@font-face{{font-family:J;src:url('{font.as_uri()}')}}"
        body = SVG_DOCUMENT.format(content=(
            '<text x="60" y="50" font-size="24" font-family="J" '
            'writing-mode="vertical-rl" text-orientation="upright">AA</text>'))
        glyphs = writing.WritingModesPdf.render(self, "horizontal-tb", body, rules)["glyphs"]
        self.assertEqual([g.text for g in glyphs], list("AA"))
        self.assertTrue(all(g.font == "CJKWritingTest-Regular" for g in glyphs), "both advances use the pinned fixture")
        self.assertAlmostEqual(glyphs[0].matrix[5] - glyphs[1].matrix[5], 18,
                               delta=writing.TOLERANCE_PT,
                               msg="the fixture has 1000-unit vertical advances at 24px = 18pt")

    def test_svg_writing_aliases_equal_modern_attributes_and_css(self):
        for context in ["inline", "external", "data"]:
            for alias, modern in ALIASES:
                for direction in ["ltr", "rtl"]:
                    with self.subTest(context=context, alias=alias, direction=direction):
                        expected = self.image(f'writing-mode="{modern}" direction="{direction}"', context=context)
                        self.assertInk(self.image(f'writing-mode="{alias}" direction="{direction}"', context=context),
                                       expected, "SVG presentational writing-mode compatibility")
                        self.assertInk(self.image(f'direction="{direction}"', f"text{{writing-mode:{alias}}}", context),
                                       expected, "SVG CSS writing-mode compatibility")

    def test_svg_glyph_shorthand_and_modern_orientation_share_cascade_state(self):
        for context in ["inline", "external", "data"]:
            for alias, modern in ORIENTATIONS:
                with self.subTest(context=context, alias=alias):
                    expected = self.image(f'writing-mode="vertical-rl" text-orientation="{modern}"', context=context)
                    self.assertInk(self.image(f'writing-mode="vertical-rl" glyph-orientation-vertical="{alias}"', context=context),
                                   expected, "SVG glyph-orientation attribute maps to modern orientation")
                    self.assertInk(self.image('writing-mode="vertical-rl"',
                                              f"text{{glyph-orientation-vertical:{alias}}}", context),
                                   expected, "SVG shorthand declaration uses modern orientation")
                    self.assertInk(self.image('writing-mode="vertical-rl" glyph-orientation-vertical="0"',
                                              f"text{{text-orientation:{modern}}}", context),
                                   expected, "author style overrides the legacy presentation attribute")

    def test_invalid_svg_glyph_declarations_preserve_the_preceding_modern_value(self):
        for context in ["inline", "external", "data"]:
            expected = self.image('writing-mode="vertical-rl" text-orientation="upright"', context=context)
            for declaration in ["glyph-orientation-vertical:180", "glyph-orientation-vertical:270deg",
                                "glyph-orientation-vertical:0rad", "glyph-orientation-vertical:90grad",
                                "glyph-orientation-vertical:.25turn", "glyph-orientation-vertical:1",
                                "glyph-orientation-horizontal:90deg"]:
                with self.subTest(context=context, declaration=declaration):
                    self.assertInk(self.image('writing-mode="vertical-rl"',
                                              f"text{{text-orientation:upright;{declaration}}}", context),
                                   expected, "invalid legacy syntax must not replace valid modern orientation")

    def test_invalid_svg_orientation_attributes_use_the_initial_value(self):
        for context in ["inline", "external", "data"]:
            with self.subTest(context=context):
                inherited = "svg{text-orientation:upright}"
                expected = self.image('writing-mode="vertical-rl" text-orientation="mixed"', inherited, context)
                self.assertInk(self.image('writing-mode="vertical-rl" glyph-orientation-vertical="180deg"', inherited, context),
                               expected, "SVG2 invalid presentation attributes specify the property's initial value")

    def test_svg_upright_forces_ltr_used_direction_and_start_anchor(self):
        for context in ["inline", "external", "data"]:
            with self.subTest(context=context):
                expected = self.image('writing-mode="vertical-rl" text-orientation="upright" direction="ltr"', context=context)
                self.assertInk(self.image('writing-mode="vertical-rl" text-orientation="upright" direction="rtl"', context=context),
                               expected, "upright keeps start anchoring at the same point while direction computes to rtl")

    def test_svg_rollback_preserves_the_legacy_ua_default_and_inline_layer_fallback(self):
        for context in ["inline", "external", "data"]:
            with self.subTest(context=context):
                expected = self.image('writing-mode="vertical-rl"', context=context)
                self.assertInk(self.image('writing-mode="tb" style="writing-mode:revert"', context=context),
                               expected, "revert keeps the mandatory UA rule for the legacy writing-mode attribute")
                expected = self.image('writing-mode="vertical-rl" text-orientation="sideways"', context=context)
                self.assertInk(self.image('writing-mode="vertical-rl" style="glyph-orientation-vertical:revert-layer"',
                                          "text{text-orientation:sideways}", context),
                               expected, "an inline shorthand's revert-layer restores the stylesheet longhand")
                expected = self.image('writing-mode="vertical-rl" text-orientation="upright"', context=context)
                self.assertInk(self.image('writing-mode="vertical-rl" glyph-orientation-vertical="0"',
                                          "text{glyph-orientation-vertical:revert-layer;writing-mode:revert-layer}", context),
                               expected, "stylesheet revert-layer restores the author presentational hint origin")

    def test_later_upright_svg_span_controls_chunk_bidi_order(self):
        font = writing.ROOT / "tests/fixtures/fonts/CJKWritingTest.otf"
        rules = f"@font-face{{font-family:J;src:url('{font.as_uri()}')}}"
        render = lambda content, orientation: writing.WritingModesPdf.render(
            self, "horizontal-tb", SVG_DOCUMENT.format(content=(
                f'<text x="60" y="50" font-size="24" font-family="J" '
                f'writing-mode="vertical-rl" text-orientation="{orientation}">{content}</text>')),
            rules)["glyphs"]
        self.assertGlyphs(render('漢<tspan text-orientation="upright">אב</tspan>', "mixed"),
                          render("漢אב", "upright"),
                          "later upright spans establish strong-LTR order before per-font glyph replacement")
        self.assertGlyphs(render('<tspan text-orientation="upright">漢</tspan><tspan text-orientation="sideways">אב</tspan>', "mixed"),
                          render("漢אב", "mixed"),
                          "an earlier upright span must not force later sideways text into LTR order")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
