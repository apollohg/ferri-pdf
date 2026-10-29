"""Math content placement inside CSS constrained native boxes."""
import argparse
from pathlib import Path
import unittest

import mathml_css_pdf as css
import table_positioning_pdf as tables
from writing_anchors_pdf import colour_bounds

RENDERER = css.RENDERER
TOLERANCE_PT = css.TOLERANCE_PT
USED_WIDTH_PT = css.USED_WIDTH_PT
OWNER_HEIGHT_PT = css.USED_HEIGHT_PT
CHILD_WIDTH_PT = 20
CHILD_HEIGHT_PT = 20
CONTAINER_WIDTH_PT = 160
CONTAINER_HEIGHT_PT = 100
TRY_LEFT_PT = 145
ANCHOR_SEED_PT = 1
RELATIVE_HEIGHT_PERCENT = 50
PERCENT_DENOMINATOR = 100
MAX_WIDTH_PROBE_PT = 120
NARROW_WIDTH_PT = 4
PADDING_PT = 6
BORDER_PT = 4
OWNER_BORDER_PT = 1
PAGE_MARGIN_PT = 10
RED = (255, 0, 0)
BLUE = (0, 0, 255)


class MathmlSizingPdf(unittest.TestCase):
    def setUp(self):
        self.helper = css.MathmlCssPdf()
        css.RENDERER = RENDERER
        self.helper.setUp()
        self.addCleanup(self.helper.doCleanups)

    def test_fraction_and_limit_content_centers_in_the_used_css_width(self):
        for tag in ("mfrac", "munder", "mover", "munderover"):
            children = "<mn>1</mn><mn>2</mn>" + (
                "<mn>3</mn>" if tag == "munderover" else "")
            for direction in ("ltr", "rtl"):
                for size in (f"width:{USED_WIDTH_PT}pt",
                             f"min-width:{USED_WIDTH_PT}pt",
                             f"width:{MAX_WIDTH_PROBE_PT}pt;max-width:{USED_WIDTH_PT}pt",
                             f"width:{USED_WIDTH_PT}pt;box-sizing:border-box;"
                             f"padding:{PADDING_PT}pt;border:{BORDER_PT}pt solid blue",
                             f"width:{NARROW_WIDTH_PT}pt"):
                    with self.subTest(tag=tag, direction=direction, size=size):
                        body = f"<math><{tag}>{children}</{tag}><mn>Z</mn></math>"
                        rules = f"{tag}{{direction:{direction}}}"
                        _, natural = self.helper.render(body, rules)
                        _, sized = self.helper.render(body, rules + f"{tag}{{{size}}}")
                        extra = (self.helper.one(sized, "Z").origin[0]
                                 - self.helper.one(natural, "Z").origin[0])
                        self.assertNotEqual(extra, 0, "the used CSS width must reach the parent row")
                        for text in ("1", "2", "3") if tag == "munderover" else ("1", "2"):
                            before = self.helper.one(natural, text)
                            after = self.helper.one(sized, text)
                            self.assertAlmostEqual(
                                after.origin[0] - before.origin[0], extra / 2,
                                delta=TOLERANCE_PT,
                                msg=f"{tag}/{text}: Core centers math content in the CSS content box")
                            self.assertEqual(after.matrix[:4], before.matrix[:4],
                                             "CSS sizing must not scale the glyphs")

    def test_row_content_keeps_the_inline_start_edge_in_both_directions(self):
        body = "<math><mrow><mn>1</mn><mn>2</mn></mrow><mn>Z</mn></math>"
        for direction in ("ltr", "rtl"):
            with self.subTest(direction=direction):
                rules = f"mrow{{direction:{direction}}}"
                _, natural = self.helper.render(body, rules)
                _, sized = self.helper.render(body, rules + f"mrow{{width:{USED_WIDTH_PT}pt}}")
                extra = (self.helper.one(sized, "Z").origin[0]
                         - self.helper.one(natural, "Z").origin[0])
                for text in ("1", "2"):
                    self.assertAlmostEqual(
                        self.helper.one(sized, text).origin[0]
                        - self.helper.one(natural, text).origin[0],
                        extra if direction == "rtl" else 0,
                        delta=TOLERANCE_PT,
                        msg="Core aligns ordinary math content to the actual inline-start edge")

    def test_fraction_rule_spans_the_css_content_box(self):
        output, _ = self.helper.render(
            '<math><mfrac linethickness="1pt"><mn>1</mn><mn>2</mn></mfrac></math>',
            f"mfrac{{width:{USED_WIDTH_PT}pt;padding:0;color:red}}mn{{color:transparent}}")
        with tables.TablePositioningPdf().raster(
                self.helper.root, output, vector_antialias=False) as image:
            bounds = colour_bounds(image, RED)
        self.assertIsNotNone(bounds, "the fraction rule must paint independently of its child text")
        self.assertEqual((bounds[0], bounds[2]),
                         (PAGE_MARGIN_PT, PAGE_MARGIN_PT + USED_WIDTH_PT),
                         "Core requires the rule to span the full CSS content box")

    def test_block_math_root_centers_its_formula_with_an_author_inline_override(self):
        _, reference = self.helper.render("<math><mn>A</mn><mn>Z</mn></math>")
        advance = (self.helper.one(reference, "Z").origin[0]
                   - self.helper.one(reference, "A").origin[0])
        for display, expected in (("", (USED_WIDTH_PT - advance) / 2),
                                  ("math{display:inline math}", 0)):
            with self.subTest(override=display):
                _, content = self.helper.render(
                    '<math display="block"><mn>A</mn></math>',
                    f"body{{width:{USED_WIDTH_PT}pt}}" + display)
                self.assertAlmostEqual(
                    self.helper.one(content, "A").origin[0] - PAGE_MARGIN_PT,
                    expected, delta=TOLERANCE_PT,
                    msg="Core block math centers the formula; an author inline display cancels it")

    def test_preferred_aspect_ratio_sizes_native_math_without_scaling_glyphs(self):
        cases = (
            ("height:40pt", 80),
            ("height:40pt;max-width:60pt", 60),
            ("height:40pt;min-width:100pt", 100),
            ("height:40pt;box-sizing:border-box;padding:6pt;border:4pt solid red", 80),
            ("min-height:40pt", 80),
            ("height:40pt;width:30pt", 30),
        )
        for sizing, expected in cases:
            with self.subTest(sizing=sizing):
                style = f"aspect-ratio:2;{sizing};background:blue"
                _, control = self.helper.render(f'<span style="display:inline-block;{style}">A</span>Z')
                _, actual = self.helper.render(f'<math><mn style="{style}">A</mn><mn>Z</mn></math>')
                self.assertAlmostEqual(self.helper.one(control, "Z").origin[0] - PAGE_MARGIN_PT,
                                       expected, delta=TOLERANCE_PT,
                                       msg="the ordinary CSS control must satisfy the independent ratio/constraint width")
                self.assertAlmostEqual(self.helper.one(actual, "Z").origin[0],
                                       self.helper.one(control, "Z").origin[0], delta=TOLERANCE_PT,
                                       msg="native math must retain the CSS preferred-ratio width")
                self.assertEqual(self.helper.one(actual, "A").matrix[:4],
                                 self.helper.one(control, "A").matrix[:4],
                                 "a preferred ratio sizes the box without scaling the math glyph")

    def test_ratio_height_definiteness_and_automatic_minimum_reach_css_fallback(self):
        for child, expected_height in (("height:50%", 20), ("height:100pt", 100)):
            with self.subTest(child=child):
                style = "width:80pt;aspect-ratio:2;background:red"
                content = f'<span style="display:block;width:20pt;{child};background:blue"></span>'
                dimensions = {}
                for name, body in (("css", f'<span style="display:inline-block;{style}">{content}</span>Z'),
                                   ("math", f'<math><mtext style="{style}">{content}</mtext><mn>Z</mn></math>')):
                    output, _ = self.helper.render(body)
                    with tables.TablePositioningPdf().raster(
                            self.helper.root, output, vector_antialias=False) as image:
                        bounds = colour_bounds(image, BLUE)
                        owner = colour_bounds(image, RED)
                    self.assertIsNotNone(bounds, f"{name}: the fallback child must paint")
                    self.assertIsNotNone(owner, f"{name}: the ratio owner must paint its used box")
                    dimensions[name] = (bounds[2] - bounds[0], bounds[3] - bounds[1], owner[3] - owner[1])
                self.assertEqual(dimensions["css"], (20, expected_height, 40 if child == "height:50%" else 100),
                                 "the independent CSS ratio control must preserve percent basis/content minimum")
                self.assertEqual(dimensions["math"], dimensions["css"],
                                 "native ratio sizing must preserve both definiteness and an automatic content minimum")

    def test_nowrap_token_spaces_do_not_replace_ink_metrics_with_a_css_strut(self):
        reference = None
        for tag, text in (("mtext", "AA"), ("mtext", "A A"),
                          ("mtext", "A&#160;A"), ("mn", "A A")):
            with self.subTest(tag=tag, text=text):
                output, _ = self.helper.render(
                    f"<math><{tag}>{text}</{tag}></math>", f"{tag}{{background:blue}}")
                with tables.TablePositioningPdf().raster(
                        self.helper.root, output, vector_antialias=False) as image:
                    bounds = colour_bounds(image, BLUE)
                self.assertIsNotNone(bounds, "the token's native CSS box must paint")
                vertical = bounds[1], bounds[3]
                if reference is None:
                    reference = vertical
                self.assertEqual(
                    vertical, reference,
                    "Core nowrap text has no soft break opportunity; spaces do not add font struts")

    def test_native_child_css_sizes_reach_flex_table_and_shrink_to_fit_intrinsics(self):
        layouts = ("width:min-content", "float:left", "display:flex;width:min-content",
                   "display:table;width:min-content")
        sizes = (f"width:{USED_WIDTH_PT}pt", f"min-width:{USED_WIDTH_PT}pt",
                 f"width:{MAX_WIDTH_PROBE_PT}pt;max-width:{USED_WIDTH_PT}pt",
                 f"width:{USED_WIDTH_PT}pt;box-sizing:border-box;"
                 f"padding:{PADDING_PT}pt;border:{BORDER_PT}pt solid red")
        for layout in layouts:
            for size in sizes:
                with self.subTest(layout=layout, size=size):
                    measured = {}
                    for kind in ("css", "math"):
                        child = (f'<span style="display:inline-block;{size}">A</span>'
                                 if kind == "css" else
                                 f'<math><mn style="{size}">A</mn></math>')
                        if layout.startswith("display:table"):
                            child = ('<div style="display:table-row">'
                                     f'<div style="display:table-cell">{child}</div></div>')
                        output, content = self.helper.render(
                            f'<div class="probe">{child}</div>',
                            f'.probe{{{layout};border-top:{OWNER_BORDER_PT}pt solid blue}}')
                        with tables.TablePositioningPdf().raster(
                                self.helper.root, output, vector_antialias=False) as image:
                            bounds = colour_bounds(image, BLUE)
                        self.assertIsNotNone(bounds, "the intrinsic-size owner's top border must paint")
                        measured[kind] = (bounds[2] - bounds[0], self.helper.one(content, "A").origin[0])
                    self.assertEqual(measured["css"][0], USED_WIDTH_PT,
                                     "the independent CSS control reserves the full used width")
                    self.assertEqual(measured["math"][0], measured["css"][0],
                                     "native child CSS sizes participate in parent intrinsic sizing")
                    self.assertAlmostEqual(measured["math"][1], measured["css"][1],
                                           delta=TOLERANCE_PT,
                                           msg="intrinsic underallocation must not move glyphs off page")

    def test_css_fallback_preserves_distinct_min_and_max_content_widths(self):
        widths = {}
        for sizing in ("min-content", "max-content"):
            for kind in ("css", "math"):
                with self.subTest(sizing=sizing, kind=kind):
                    child = '<span style="white-space:normal">AB CD</span>'
                    child = (f"<math><mtext>{child}</mtext></math>" if kind == "math"
                             else f'<span style="display:inline-block">{child}</span>')
                    output, content = self.helper.render(
                        f'<div class="host">{child}</div>',
                        f'.host{{width:{sizing};border-top:{OWNER_BORDER_PT}pt solid blue}}')
                    with tables.TablePositioningPdf().raster(
                            self.helper.root, output, vector_antialias=False) as image:
                        bounds = colour_bounds(image, BLUE)
                    self.assertIsNotNone(bounds, "the intrinsic owner's border must paint")
                    widths[kind, sizing] = bounds[2] - bounds[0]
                    for text in "ABCD":
                        self.helper.one(content, text)
        self.assertLess(widths["css", "min-content"], widths["css", "max-content"],
                        "the independent CSS text control has a real soft break opportunity")
        for sizing in ("min-content", "max-content"):
            self.assertEqual(widths["math", sizing], widths["css", sizing],
                             "CSS fallback contributes its intrinsic widths, not its used width")

    def test_css_fallback_preserves_percentage_height_definiteness(self):
        for height in ("height", "min-height"):
            for mode in ("horizontal-tb", "vertical-rl"):
                with self.subTest(height=height, mode=mode):
                    child = (f'<span style="display:block;width:{CHILD_WIDTH_PT}pt;'
                             f'height:{RELATIVE_HEIGHT_PERCENT}%;background:blue;'
                             f'writing-mode:{mode}"></span>')
                    bounds = {}
                    for kind in ("css", "math"):
                        body = (f'<span class="owner" style="display:inline-block">{child}</span>'
                                if kind == "css" else f'<math><mtext class="owner">{child}</mtext></math>')
                        output, _ = self.helper.render(
                            body, f'.owner{{width:{USED_WIDTH_PT}pt;{height}:{OWNER_HEIGHT_PT}pt}}')
                        with tables.TablePositioningPdf().raster(
                                self.helper.root, output, vector_antialias=False) as image:
                            bounds[kind] = colour_bounds(image, BLUE)
                    if height == "height":
                        self.assertIsNotNone(bounds["css"], "a definite owner resolves the child's percentage")
                        self.assertEqual(bounds["css"][2] - bounds["css"][0], CHILD_WIDTH_PT)
                        self.assertEqual(bounds["css"][3] - bounds["css"][1],
                                         OWNER_HEIGHT_PT * RELATIVE_HEIGHT_PERCENT // PERCENT_DENOMINATOR)
                    else:
                        self.assertIsNone(bounds["css"], "an automatic height stays indefinite under min-height")
                    self.assertEqual(bounds["math"], bounds["css"],
                                     "transparent retained content must preserve the real owner's definiteness")

    def test_css_fallback_resolves_percentage_width_against_its_constrained_owner(self):
        rules = (
            (f"width:{USED_WIDTH_PT}pt", USED_WIDTH_PT),
            (f"min-width:{USED_WIDTH_PT}pt", USED_WIDTH_PT),
            (f"width:{MAX_WIDTH_PROBE_PT}pt;max-width:{USED_WIDTH_PT}pt", USED_WIDTH_PT),
            (f"width:{USED_WIDTH_PT}pt;box-sizing:border-box;padding:{PADDING_PT}pt;"
             f"border:{BORDER_PT}pt solid red", USED_WIDTH_PT - 2 * (PADDING_PT + BORDER_PT)),
            ("width:auto", None),
            (f"width:{RELATIVE_HEIGHT_PERCENT}%", CONTAINER_WIDTH_PT * RELATIVE_HEIGHT_PERCENT
             // PERCENT_DENOMINATOR),
        )
        for owner, content_width in rules:
            for mode in ("horizontal-tb", "vertical-rl"):
                with self.subTest(owner=owner, mode=mode):
                    child = (f'<span style="display:block;width:{RELATIVE_HEIGHT_PERCENT}%;'
                             f'height:{CHILD_HEIGHT_PT}pt;background:blue;writing-mode:{mode}"></span>')
                    bounds = {}
                    for kind in ("css", "math"):
                        body = (f'<span class="owner" style="display:inline-block">{child}</span>'
                                if kind == "css" else f'<math><mtext class="owner">{child}</mtext></math>')
                        output, _ = self.helper.render(
                            f'<div class="container">{body}</div>',
                            f'.container{{width:{CONTAINER_WIDTH_PT}pt}}'
                            f'.container math{{width:{CONTAINER_WIDTH_PT}pt}}'
                            f'.owner{{{owner};height:{OWNER_HEIGHT_PT}pt}}')
                        with tables.TablePositioningPdf().raster(
                                self.helper.root, output, vector_antialias=False) as image:
                            bounds[kind] = colour_bounds(image, BLUE)
                    if content_width is None and mode == "horizontal-tb":
                        self.assertIsNone(bounds["css"], "an unconstrained cyclic percentage has no width")
                    elif content_width is not None:
                        self.assertIsNotNone(bounds["css"], "the constrained CSS owner resolves its child width")
                        self.assertEqual(bounds["css"][2] - bounds["css"][0],
                                         content_width * RELATIVE_HEIGHT_PERCENT // PERCENT_DENOMINATOR)
                    self.assertEqual(bounds["math"], bounds["css"],
                                     "CSS fallback must use the actual owner content width, including min/max and box-sizing")

    def test_css_fallback_intrinsic_keyword_uses_the_selected_position_try_style(self):
        body = (f'<section><math><mspace width="{ANCHOR_SEED_PT}pt" height="{ANCHOR_SEED_PT}pt"/>'
                '<mtext class="target"><span style="white-space:normal">AB CD</span>'
                '</mtext></math></section>')
        rules = (f'section{{position:relative;width:{CONTAINER_WIDTH_PT}pt;height:{CONTAINER_HEIGHT_PT}pt}}'
                 '@position-try --fit{left:0;width:max-content}'
                 f'.target{{position:absolute;left:{TRY_LEFT_PT}pt;top:0;'
                 'width:min-content;position-try-fallbacks:--fit;background:blue}')
        _, expected = self.helper.render(
            body, rules + '.target{left:0;width:max-content;position-try-fallbacks:none}')
        self.helper.assert_same_baseline(self.helper.one(expected, "A"), self.helper.one(expected, "C"),
                                         "the literal accepted max-content style has one line")
        _, actual = self.helper.render(body, rules)
        for text in "ABCD":
            for axis in range(2):
                self.assertAlmostEqual(
                    self.helper.one(actual, text).origin[axis],
                    self.helper.one(expected, text).origin[axis], delta=TOLERANCE_PT,
                    msg="fallback content uses the selected max-content width rather than canonical min-content")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    args, remaining = parser.parse_known_args()
    RENDERER = args.renderer.resolve()
    unittest.main(argv=[__file__, *remaining])
