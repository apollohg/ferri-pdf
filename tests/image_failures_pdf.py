"""Verify failed empty image line boxes and image break margins in produced PDFs."""
import argparse
import base64
import io
from pathlib import Path
import unittest

from PIL import Image
import writing_modes_pdf as writing


class ImageFallbackPdf(unittest.TestCase):
    render = writing.WritingModesPdf.render

    def test_empty_inline_image_preserves_collapsed_default_paragraph_margins(self):
        rules = "@page{size:300pt 200pt;margin:20pt}body{font-size:12pt;line-height:18pt}p{margin:1em 0}"
        reference = self.render("horizontal-tb", "<p>BEFORE</p><p>AFTER</p>", rules)
        control = self.render("horizontal-tb", "<p>BEFORE</p><p>AFTER</p>", rules, pixels=True)
        for middle in ["<img src=missing.png alt=''>", " \n<img src=missing.png alt=''>\n ",
                       "<img src=missing.png alt='' style='display:block'>",
                       "<div style='text-box-trim:trim-both'><img src=missing.png alt=''></div>"]:
            with self.subTest(middle=middle):
                body = f"<p>BEFORE</p>{middle}<p>AFTER</p>"
                actual = self.render("horizontal-tb", body, rules)
                self.assertEqual([(g.text, g.matrix) for g in actual["glyphs"]],
                                 [(g.text, g.matrix) for g in reference["glyphs"]],
                                 "the actual PDF text must have no extra paragraph margin or line")
                pixels = self.render("horizontal-tb", body, rules, pixels=True)
                self.assertEqual(pixels.tobytes(), control.tobytes(),
                                 "empty image fallback must match no-image PDF ink exactly")

    def test_phantom_lines_do_not_supply_atomic_or_native_baselines(self):
        cases = [
            ("last", "", "<span class=box>{image}<p>A</p></span> B"),
            ("first", ".box{baseline-source:first}", "<span class=box>{image}<p>A</p></span> B"),
            ("trailing", "", "<span class=box><p>A</p>{image}</span> B"),
            ("flex", "section{display:flex;align-items:baseline}.box{display:block}",
             "<section><span class=box>{image}<p>A</p></span><span>B</span></section>"),
            ("native-parallel", "body{writing-mode:vertical-rl}.box{writing-mode:vertical-lr;baseline-source:first}",
             "<span class=box>{image}<p>A</p></span> B"),
        ]
        for name, extra, body in cases:
            with self.subTest(name=name):
                rules = "body{font-size:12pt;line-height:18pt}p{margin:0}.box{display:inline-block}" + extra
                reference = self.render("horizontal-tb", body.replace("{image}", ""), rules)
                actual = self.render("horizontal-tb", body.replace("{image}", "<img src=missing.png alt=''>"), rules)
                self.assertEqual([g.text for g in actual["glyphs"]], [g.text for g in reference["glyphs"]])
                self.assertCountEqual([g.text for g in actual["glyphs"] if g.text.strip()], ["A", "B"],
                                      "both glyphs must survive the actual PDF")
                for glyph, expected in zip(actual["glyphs"], reference["glyphs"]):
                    self.assertEqual(glyph.matrix, expected.matrix,
                                     "an ignored source line cannot establish first or last baseline geometry")

    def test_ignored_image_lines_do_not_consume_clamp_lines_or_add_ellipsis(self):
        image = "<img src=missing.png alt=''>"
        rules = "body{font-size:12pt;line-height:18pt}p{margin:0}.c{line-clamp:1}"
        for name, before, after, second in [
            ("leading", image, "", ""),
            ("wrapped leading", f"<div>{image}</div>", "", ""),
            ("trailing", "", image, ""),
            ("nested trailing", "", f"<div><div>{image}</div></div>", ""),
            ("real clamp", image, image, "<p>Second</p>"),
            ("wrapped real clamp", f"<div>{image}</div>", f"<div>{image}</div>", "<p>Second</p>"),
        ]:
            with self.subTest(name=name):
                control = f"<div class=c><p>First</p>{second}</div><p>Outside</p>"
                body = f"<div class=c>{before}<p>First</p>{after}{second}</div><p>Outside</p>"
                reference = self.render("horizontal-tb", control, rules)
                actual = self.render("horizontal-tb", body, rules)
                self.assertEqual([(g.text, g.matrix) for g in actual["glyphs"]],
                                 [(g.text, g.matrix) for g in reference["glyphs"]],
                                 "ignored source lines cannot consume the first real line or invent an ellipsis")
                expected_pixels = self.render("horizontal-tb", control, rules, pixels=True)
                actual_pixels = self.render("horizontal-tb", body, rules, pixels=True)
                self.assertEqual(actual_pixels.tobytes(), expected_pixels.tobytes(),
                                 "clamped PDF ink must match the actual no-image control exactly")

    def test_loaded_image_margin_at_unforced_break_is_truncated(self):
        image_size = 100
        margin = 20
        with Image.new("RGB", (image_size, image_size), "blue") as image:
            stream = io.BytesIO()
            image.save(stream, format="PNG")
        source = "data:image/png;base64," + base64.b64encode(stream.getvalue()).decode("ascii")
        results = {}
        for forced in [False, True]:
            for top in [0, margin]:
                rules = ("@page{size:A4;margin:20pt}body{font-size:12pt;line-height:normal}p{margin:1em 0}"
                         f"img{{display:block;width:{image_size}pt;margin-top:{top}pt;break-before:{'page' if forced else 'auto'}}}")
                actual = self.render("horizontal-tb", f"<div style='height:760pt'></div><img src='{source}'><p>AFTER</p>", rules)
                self.assertEqual(len(actual["images"]), 1, "the image must really load and paint once")
                self.assertEqual((actual["images"][0]["width_px"], actual["images"][0]["height_px"]),
                                 (image_size, image_size), "the source really is a100×100 raster")
                self.assertEqual(actual["images"][0]["page"], 2, "the loaded image must move to page two")
                after = next(g for g in actual["glyphs"] if g.text == "A")
                self.assertEqual(after.page, 2)
                results[forced, top] = after.matrix[-1]
        self.assertEqual(results[False, margin], results[False, 0],
                         "CSS Break3 5.2 truncates the adjoining unforced-break margin")
        self.assertAlmostEqual(results[True, 0] - results[True, margin], margin, places=3,
                               msg="a forced break preserves the following image margin")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--renderer", type=Path, required=True)
    args, rest = parser.parse_known_args()
    writing.RENDERER = args.renderer.resolve()
    unittest.main(argv=[__file__, *rest])
