"""Anchor geometry through physical root and independently paginated writing frames."""
import argparse
from pathlib import Path
import unittest
from PIL import ImageChops

import writing_modes_pdf as writing

MODES = ("vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr")
BLUE = (0, 0, 255)
RED = (255, 0, 0)


def colour_bounds(image, colour):
    positions = [(x, y) for y in range(image.height) for x in range(image.width)
                 if image.getpixel((x, y)) == colour]
    if not positions:
        return None
    return (min(x for x, _ in positions), min(y for _, y in positions),
            max(x for x, _ in positions) + 1, max(y for _, y in positions) + 1)


class WritingAnchorsPdf(unittest.TestCase):
    def test_physical_anchor_edges_and_dimensions_survive_each_native_frame(self):
        body = ('<section><div class=anchor></div></section>'
                '<a class=target href="https://example.org/"></a>')
        for mode in MODES:
            for root, child in (("horizontal-tb", mode), (mode, "horizontal-tb"), (mode, mode)):
                with self.subTest(root=root, child=child):
                    rules = (f"section{{writing-mode:{child};width:80pt;height:80pt}}"
                             ".anchor{anchor-name:--a;width:24pt;height:12pt;background:blue}"
                             ".target{position:absolute;position-anchor:--a;left:anchor(left);"
                             "top:anchor(bottom);width:anchor-size(width);height:anchor-size(height);background:red}")
                    image = writing.WritingModesPdf.render(self, root, body, rules, pixels=True)
                    anchor = colour_bounds(image, BLUE)
                    self.assertIsNotNone(anchor, "the physical blue anchor must actually paint")
                    left, top, right, bottom = anchor
                    expected = (left, bottom, right, bottom + bottom - top)
                    self.assertEqual(colour_bounds(image, RED), expected,
                                     f"{root}/{child}: anchor(left/bottom) and anchor-size(width/height) "
                                     f"must follow painted physical anchor {anchor}")
                    navigation = writing.WritingModesPdf.render(self, root, body, rules, navigation=True)
                    self.assertEqual(len(navigation["links"]), 1, str(navigation))
                    rectangle = navigation["links"][0]["rect"]
                    physical = (expected[0], writing.PAGE_PT - expected[3],
                                expected[2], writing.PAGE_PT - expected[1])
                    for actual, value in zip(rectangle, physical):
                        self.assertAlmostEqual(actual, value, delta=writing.TOLERANCE_PT,
                                               msg="the anchored link must follow the painted physical target")


    def test_physical_position_area_uses_the_same_anchor_after_root_conversion(self):
        body = '<section><div class=anchor></div></section><div class=target></div>'
        for mode in ("horizontal-tb",) + MODES:
            with self.subTest(mode=mode):
                rules = ("section{width:80pt;height:80pt}"
                         ".anchor{anchor-name:--a;width:24pt;height:12pt;background:blue}"
                         ".target{position:absolute;position-anchor:--a;position-area:bottom;"
                         "width:20pt;height:10pt;background:red}")
                image = writing.WritingModesPdf.render(self, mode, body, rules, pixels=True)
                anchor = colour_bounds(image, BLUE)
                self.assertIsNotNone(anchor, "the reference anchor must paint")
                left, _, right, bottom = anchor
                centre = (left + right) // 2
                # A target overflowing the page area shifts back into it (CSS Box Alignment 3
                # §4.4.1.2), as when the anchor ends at the page area's bottom edge.
                top = min(bottom, writing.PAGE_PT - writing.MARGIN_PT - 10)
                self.assertEqual(colour_bounds(image, RED), (centre - 10, top, centre + 10, top + 10),
                                 f"{mode}: physical bottom area must centre the target below {anchor}")


    def test_anchor_visibility_uses_physical_shadow_ink_across_native_frames(self):
        body = ('<section><div class=anchor>X</div></section>'
                '<a class=target href="https://example.org/"></a>')
        for mode in MODES:
            for root, child in (("horizontal-tb", mode), (mode, "horizontal-tb"), (mode, mode)):
                for effect in ("box-shadow:20pt 0 green", "filter:drop-shadow(20pt 0 green)", "text-shadow:20pt 0 green"):
                    with self.subTest(root=root, child=child, effect=effect):
                        rules = (f"section{{writing-mode:{child};position:relative;width:40pt;height:40pt;overflow:hidden}}"
                                 ".anchor{position:absolute;left:-20pt;top:10pt;anchor-name:--a;"
                                 "width:10pt;height:10pt;background:blue;color:blue;font:10pt/1 serif;" + effect + "}"
                                 ".target{position:fixed;position-anchor:--a;position-visibility:anchors-visible;"
                                 "left:100pt;top:100pt;width:10pt;height:10pt;background:red}")
                        image = writing.WritingModesPdf.render(self, root, body, rules, pixels=True)
                        self.assertIsNotNone(colour_bounds(image, (0, 128, 0)),
                                             "the directional shadow must actually paint inside the physical clip")
                        self.assertIsNotNone(colour_bounds(image, RED),
                                             "visible shadow ink keeps the anchored target visible")
                        navigation = writing.WritingModesPdf.render(self, root, body, rules, navigation=True)
                        self.assertEqual(len(navigation["links"]), 1,
                                         "visibility of the target link follows its shadow-visible anchor")


    def test_native_positioned_owner_uses_its_physical_padding_box(self):
        body = '<section><a class=target href="https://example.org/"></a></section>'
        for mode in MODES:
            for root, child in (("horizontal-tb", mode), (mode, "horizontal-tb"), (mode, mode)):
                for position in ("absolute", "fixed"):
                    with self.subTest(root=root, child=child, position=position):
                        rules = (f"section{{writing-mode:{child};position:relative;width:32pt;height:32pt;"
                                 "padding:4pt;border:2pt solid blue;background:silver;transform:translate(0)}"
                                 f".target{{position:{position};left:25%;top:50%;width:12pt;height:8pt;background:red}}")
                        image = writing.WritingModesPdf.render(self, root, body, rules, pixels=True)
                        owner = colour_bounds(image, BLUE)
                        self.assertIsNotNone(owner, "the real principal owner must paint its border")
                        left, top, _, _ = owner
                        expected = (left + 12, top + 22, left + 24, top + 30)
                        self.assertEqual(colour_bounds(image, RED), expected,
                                         "percentage insets use the 40pt padding box after the 2pt border")
                        navigation = writing.WritingModesPdf.render(self, root, body, rules, navigation=True)
                        self.assertEqual(len(navigation["links"]), 1, str(navigation))
                        for actual, value in zip(navigation["links"][0]["rect"],
                                                 (expected[0], writing.PAGE_PT - expected[3],
                                                  expected[2], writing.PAGE_PT - expected[1])):
                            self.assertAlmostEqual(actual, value, delta=writing.TOLERANCE_PT,
                                                   msg="native local positioning carries the physical link box")


    def test_native_grid_owner_reuses_its_positioned_grid_area(self):
        body = '<section><a class=target href="https://example.org/"></a></section>'
        for mode in MODES:
            for root, child in (("horizontal-tb", mode), (mode, "horizontal-tb"), (mode, mode)):
                for padding in ("0", "2pt 4pt 6pt 8pt"):
                    for direction in ("ltr", "rtl"):
                        with self.subTest(root=root, child=child, padding=padding, direction=direction):
                            rules = (f"section{{writing-mode:{child};direction:{direction};padding:{padding};"
                                     "position:relative;display:grid;width:40pt;height:40pt;"
                                     "grid-template-columns:20pt 20pt;grid-template-rows:20pt 20pt}"
                                     ".target{grid-column:2 / 3;grid-row:2 / 3;background:red}")
                            reference = writing.WritingModesPdf.render(self, root, body, rules, pixels=True)
                            self.assertIsNotNone(colour_bounds(reference, RED), "the independent in-flow grid cell must paint")
                            positioned = rules + ".target{position:absolute;inset:0}"
                            actual = writing.WritingModesPdf.render(self, root, body, positioned, pixels=True)
                            difference = ImageChops.difference(actual, reference).getbbox()
                            self.assertIsNone(difference,
                                              f"the native positioned grid area differs from its in-flow tracks in {difference}")
                            expected_links = writing.WritingModesPdf.render(self, root, body, rules, navigation=True)["links"]
                            actual_links = writing.WritingModesPdf.render(self, root, body, positioned, navigation=True)["links"]
                            self.assertEqual(actual_links, expected_links, "the native grid area carries the same physical link bounds")


    def test_native_physical_clip_paths_control_anchor_ink_and_links(self):
        body = ('<section><div class=anchor></div></section>'
                '<a class=target href="https://example.org/"></a>')
        for mode in MODES:
            for root, child in (("horizontal-tb", mode), (mode, "horizontal-tb"), (mode, mode)):
                for top, visible in ((10, True), (30, False)):
                    with self.subTest(root=root, child=child, top=top):
                        rules = (f"section{{writing-mode:{child};position:relative;width:40pt;height:40pt;"
                                 "clip-path:polygon(0 0,100% 0,100% 50%,0 50%)}"
                                 f".anchor{{position:absolute;left:10pt;top:{top}pt;anchor-name:--a;"
                                 "width:10pt;height:10pt;background:blue}"
                                 ".target{position:fixed;position-anchor:--a;position-visibility:anchors-visible;"
                                 "left:100pt;top:100pt;width:10pt;height:10pt;background:red}")
                        image = writing.WritingModesPdf.render(self, root, body, rules, pixels=True)
                        self.assertEqual(colour_bounds(image, BLUE) is not None, visible,
                                         "the physical clip's top half controls actual anchor paint")
                        self.assertEqual(colour_bounds(image, RED) is not None, visible,
                                         "anchor visibility follows the same physical clip-path geometry")
                        navigation = writing.WritingModesPdf.render(self, root, body, rules, navigation=True)
                        self.assertEqual(bool(navigation["links"]), visible,
                                         "a path-hidden anchor also hides the target link")


    def test_anchor_size_logical_axes_use_the_real_owner_and_query_flows(self):
        body = '<section><div class=anchor></div><div class=target></div></section>'
        for mode in MODES:
            for query in ("horizontal-tb", mode):
                base = (f"section{{writing-mode:{mode};position:relative;width:100pt;height:100pt}}"
                        ".anchor{anchor-name:--a;width:24pt;height:12pt;background:blue}"
                        f".target{{position:absolute;writing-mode:{query};position-anchor:--a;"
                        "left:anchor(left);top:anchor(bottom);background:red}")
                ruler = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                                                       base + ".target{display:none}", pixels=True)
                left, top, right, bottom = colour_bounds(ruler, BLUE)
                width, height = right - left, bottom - top
                for dimensions, size in (("block,inline", (width, height)),
                                         ("inline,block", (height, width)),
                                         ("self-block,self-inline", (height, width) if query == "horizontal-tb" else (width, height))):
                    with self.subTest(owner=mode, query=query, dimensions=dimensions):
                        x, y = dimensions.split(',')
                        rules = base + f".target{{width:anchor-size({x});height:anchor-size({y})}}"
                        image = writing.WritingModesPdf.render(self, "horizontal-tb", body, rules, pixels=True)
                        self.assertEqual(colour_bounds(image, RED),
                                         (left, bottom, left + size[0], bottom + size[1]),
                                         "block/inline use the actual CB flow; self-* use the querying box flow")

    def test_anchor_logical_edges_and_percentages_use_each_physical_axis_start(self):
        body = '<section><div class=anchor></div><div class=target></div></section>'
        for mode in MODES:
            for direction in ("ltr", "rtl"):
                base = (f"section{{writing-mode:{mode};direction:{direction};position:relative;width:100pt;height:100pt}}"
                        ".anchor{anchor-name:--a;width:24pt;height:12pt;background:blue}"
                        ".target{position:absolute;writing-mode:horizontal-tb;direction:ltr;position-anchor:--a;"
                        "width:4pt;height:4pt;background:red}")
                ruler = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                                                       base + ".target{display:none}", pixels=True)
                left, top, right, bottom = colour_bounds(ruler, BLUE)
                reverse_x = mode in ("vertical-rl", "sideways-rl")
                reverse_y = (mode == "sideways-lr") != (direction == "rtl")
                for sides, point in (("start,end", (right if reverse_x else left, top if reverse_y else bottom)),
                                     ("self-start,self-end", (left, bottom)),
                                     ("25%,25%", (right - 6 if reverse_x else left + 6,
                                                   bottom - 3 if reverse_y else top + 3))):
                    with self.subTest(owner=mode, direction=direction, sides=sides):
                        x, y = sides.split(',')
                        image = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                            base + f".target{{left:anchor({x});top:anchor({y})}}", pixels=True)
                        self.assertEqual(colour_bounds(image, RED),
                                         (point[0], point[1], point[0] + 4, point[1] + 4),
                                         "logical starts depend on the real CB flow in the property's physical axis")


    def test_logical_position_areas_match_their_physical_owner_and_self_edges(self):
        body = ('<section><div class=anchor></div>'
                '<a class=target href="https://example.org/"></a></section>')
        for mode in MODES:
            for direction in ("ltr", "rtl"):
                block = "right" if mode in ("vertical-rl", "sideways-rl") else "left"
                inline = "top" if (mode == "sideways-lr") != (direction == "rtl") else "bottom"
                for query in ("horizontal-tb", mode):
                    own_block = "top" if query == "horizontal-tb" else block
                    own_inline = "right" if query == "horizontal-tb" else ("top" if mode == "sideways-lr" else "bottom")
                    own_x = "left" if query == "horizontal-tb" else block
                    own_y_end = "bottom" if query == "horizontal-tb" else own_inline
                    cases = (("block-start inline-end", f"{block} {inline}"),
                             ("self-block-start self-inline-end", f"{own_block} {own_inline}"),
                             ("x-start y-end", f"{block} {inline}"),
                             ("start end", f"{block} {inline}"),
                             ("self-start self-end", f"{own_x} {own_y_end}"))
                    base = (f"section{{writing-mode:{mode};direction:{direction};position:relative;width:100pt;height:100pt}}"
                            ".anchor{position:absolute;anchor-name:--a;left:30pt;top:30pt;width:24pt;height:12pt;background:blue}"
                            f".target{{position:absolute;writing-mode:{query};direction:ltr;position-anchor:--a;"
                            "width:10pt;height:10pt;background:red}")
                    for logical, physical in cases:
                        with self.subTest(owner=mode, direction=direction, query=query, area=logical):
                            actual_rules = base + f".target{{position-area:{logical}}}"
                            reference_rules = base + f".target{{position-area:{physical}}}"
                            reference = writing.WritingModesPdf.render(self, "horizontal-tb", body, reference_rules, pixels=True)
                            self.assertIsNotNone(colour_bounds(reference, RED), "the explicit physical reference must paint")
                            actual = writing.WritingModesPdf.render(self, "horizontal-tb", body, actual_rules, pixels=True)
                            difference = ImageChops.difference(actual, reference).getbbox()
                            self.assertIsNone(difference, f"{logical} must equal {physical}, differing in {difference}")


    def test_try_tactics_follow_the_real_containing_block_axes(self):
        body = '<section><div class=anchor></div><div class=target></div></section>'
        for mode in MODES:
            for direction in ("ltr", "rtl"):
                right_start = mode in ("vertical-rl", "sideways-rl")
                reverse_inline = (mode == "sideways-lr") != (direction == "rtl")
                start_to_top = right_start != reverse_inline
                cases = (("flip-block", "right", "left", (80, 40), (20, 10), (20, 10)),
                         ("flip-inline", "bottom", "top", (40, 80), (10, 20), (10, 20)),
                         ("flip-start", "right", "top" if start_to_top else "bottom",
                          (80, 80 if start_to_top else 10), (20, 10), (10, 20)))
                for tactic, initial, fallback, point, size, moved in cases:
                    with self.subTest(owner=mode, direction=direction, tactic=tactic):
                        base = (f"section{{writing-mode:{mode};direction:{direction};position:relative;width:100pt;height:100pt}}"
                                f".anchor{{position:absolute;anchor-name:--a;left:{point[0]}pt;top:{point[1]}pt;"
                                "width:10pt;height:10pt;background:blue}"
                                f".target{{position:absolute;position-anchor:--a;position-area:{initial};"
                                f"width:{size[0]}pt;height:{size[1]}pt;background:red}}")
                        actual = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                            base + f".target{{position-try-fallbacks:{tactic}}}", pixels=True)
                        reference = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                            base + f"@position-try --physical{{position-area:{fallback};width:{moved[0]}pt;height:{moved[1]}pt}}"
                            ".target{position-try-fallbacks:--physical}", pixels=True)
                        self.assertIsNotNone(colour_bounds(reference, RED), "the explicit fitting fallback must paint")
                        difference = ImageChops.difference(actual, reference).getbbox()
                        self.assertIsNone(difference, f"{tactic} must choose the same physical fallback as {fallback}: {difference}")


    def test_parallel_reversed_native_owners_use_their_constrained_padding_box(self):
        body = '<section>X<a class=target href="https://example.org/"></a></section>'
        widths = ("width:100pt", "width:auto;min-width:100pt", "width:200pt;max-width:100pt")
        for root in MODES:
            for child in MODES:
                if child == root:
                    continue
                for width in widths:
                    with self.subTest(root=root, child=child, width=width):
                        rules = (f"section{{writing-mode:{child};position:relative;{width};height:60pt;"
                                 "padding:3pt 5pt 7pt 11pt;border:2pt solid blue;background:silver}"
                                 ".target{position:absolute;right:0;bottom:0;width:12pt;height:8pt;background:red}")
                        image = writing.WritingModesPdf.render(self, root, body, rules, pixels=True)
                        owner = colour_bounds(image, BLUE)
                        self.assertIsNotNone(owner, "the constrained principal owner must paint")
                        _, _, right, bottom = owner
                        self.assertEqual(colour_bounds(image, RED), (right - 14, bottom - 10, right - 2, bottom - 2),
                                         "right/bottom insets use the final principal padding box, including parallel axis reversals")

    def test_parallel_native_percentage_blocks_keep_owner_definiteness(self):
        body = '<section><div class=normal></div></section>'
        for root, child in (("vertical-rl", "vertical-lr"), ("vertical-lr", "vertical-rl"),
                            ("sideways-rl", "sideways-lr"), ("sideways-lr", "sideways-rl")):
            for width, definite in (("width:100pt", True), ("width:auto;min-width:100pt", False)):
                with self.subTest(root=root, child=child, width=width):
                    rules = (f"section{{writing-mode:{child};{width};height:60pt;"
                             "padding:3pt 5pt 7pt 11pt;border:2pt solid blue}"
                             ".normal{block-size:50%;inline-size:10pt;background:green}")
                    image = writing.WritingModesPdf.render(self, root, body, rules, pixels=True)
                    owner = colour_bounds(image, BLUE)
                    self.assertIsNotNone(owner, "the owner constraint still generates its principal box")
                    green = colour_bounds(image, (0, 128, 0))
                    if definite:
                        self.assertIsNotNone(green, "50% of a definite 100pt native block must paint")
                        self.assertEqual((green[2] - green[0], green[3] - green[1]), (50, 10),
                                         "normal percentages resolve against the actual native owner content box")
                    else:
                        self.assertIsNone(green, "a minimum alone does not make the native owner's auto block size definite")


    def test_parallel_reversed_cells_keep_natural_content_for_vertical_alignment(self):
        body = '<table><tbody><tr><td><div class=marker></div></td></tr></tbody></table>'
        for root, child in (("vertical-rl", "vertical-lr"), ("vertical-lr", "vertical-rl"),
                            ("sideways-rl", "sideways-lr"), ("sideways-lr", "sideways-rl")):
            for align in ("middle", "bottom"):
                with self.subTest(root=root, cell=child, align=align):
                    rules = (f"table{{writing-mode:{root};border-spacing:0}}"
                             f"td{{writing-mode:{child};border:2pt solid blue;padding:0;width:100pt;height:60pt;vertical-align:{align}}}"
                             ".marker{width:10pt;height:10pt;background:red}")
                    image = writing.WritingModesPdf.render(self, root, body, rules, pixels=True)
                    left, top, right, bottom = colour_bounds(image, BLUE)
                    self.assertEqual((right - left, bottom - top), (104, 64),
                                     "cell alignment must not increase its assigned principal dimensions")
                    offset = 47 if align == "middle" else (2 if root.endswith("-rl") else 92)
                    inline = 52 if child == "sideways-lr" else 2
                    self.assertEqual(colour_bounds(image, RED),
                                     (left + offset, top + inline, left + offset + 10, top + inline + 10),
                                     "the table's block-axis cell alignment uses natural content, while own inline direction is preserved")


    def test_logical_try_order_uses_owner_axes_even_for_an_orthogonal_query(self):
        body = '<section><div class=anchor></div><div class=target></div></section>'
        for mode in MODES:
            for query in ("horizontal-tb", mode):
                base = (f"section{{writing-mode:{mode};position:relative;width:100pt;height:100pt}}"
                        ".anchor{position:absolute;anchor-name:--a;left:70pt;top:20pt;width:10pt;height:10pt;background:blue}"
                        f".target{{position:absolute;writing-mode:{query};position-anchor:--a;position-area:left;"
                        "position-try-fallbacks:bottom;width:10pt;height:10pt;background:red}")
                for logical, physical in (("most-inline-size", "most-height"), ("most-block-size", "most-width")):
                    with self.subTest(owner=mode, query=query, order=logical):
                        actual = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                            base + f".target{{position-try-order:{logical}}}", pixels=True)
                        reference = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                            base + f".target{{position-try-order:{physical}}}", pixels=True)
                        self.assertIsNotNone(colour_bounds(reference, RED), "the ordered fitting reference must paint")
                        difference = ImageChops.difference(actual, reference).getbbox()
                        self.assertIsNone(difference, f"{logical} must equal {physical} for the real vertical CB: {difference}")

    def test_diagonal_tactics_preserve_anchor_size_axes_in_insets_and_margins(self):
        body = '<section><div class=anchor></div><div class=target></div></section>'
        for declaration, moved in (("left:anchor-size(width)", "top:24pt"),
                                   ("margin-left:anchor-size(width)", "margin-top:24pt")):
            with self.subTest(declaration=declaration):
                base = ("section{position:relative;width:120pt;height:120pt}"
                        ".anchor{position:absolute;anchor-name:--a;left:80pt;top:20pt;width:24pt;height:12pt;background:blue}"
                        ".target{position:absolute;position-anchor:--a;position-area:right;width:20pt;height:10pt;"
                        + declaration + ";background:red}")
                actual = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                    base + ".target{position-try-fallbacks:flip-start}", pixels=True)
                reference = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                    base + "@position-try --physical{position-area:bottom;left:auto;margin-left:0;width:10pt;height:20pt;"
                    + moved + "}.target{position-try-fallbacks:--physical}", pixels=True)
                self.assertEqual(colour_bounds(reference, RED), (107, 76, 117, 96),
                                 "the explicit physical fallback must actually be chosen and fit below the anchor")
                difference = ImageChops.difference(actual, reference).getbbox()
                self.assertIsNone(difference,
                                  f"the moved inset/margin still reads the anchor's physical width: {difference}")


    def test_diagonal_fallbacks_inherit_the_original_longhand_before_moving_it(self):
        body = '<section><div class=anchor></div><div class=target></div></section>'
        cases = (("size", "horizontal-tb", "", "width:inherit;height:20pt", "width:20pt;height:100pt"),
                 ("margin", "horizontal-tb", "margin-left:20pt;margin-top:40pt", "width:20pt;height:10pt;margin-left:inherit",
                  "width:10pt;height:20pt;margin-left:0;margin-top:20pt"),
                 ("logical-margin", "vertical-lr", "margin-left:20pt;margin-top:40pt", "width:20pt;height:10pt;margin-inline-start:inherit",
                  "width:10pt;height:20pt;margin-left:0;margin-top:40pt"))
        for name, owner, margins, declarations, inherited in cases:
            with self.subTest(case=name):
                base = (f"section{{position:absolute;left:0;top:0;writing-mode:{owner};width:100pt;height:200pt;{margins}}}"
                        ".anchor{position:absolute;anchor-name:--a;left:70pt;top:20pt;width:24pt;height:10pt;background:blue}"
                        ".target{position:absolute;writing-mode:horizontal-tb;position-anchor:--a;position-area:right;"
                        + declarations + ";background:red}")
                actual = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                    base + ".target{position-try-fallbacks:flip-start}", pixels=True)
                reference = writing.WritingModesPdf.render(self, "horizontal-tb", body,
                    base + "@position-try --physical{position-area:bottom;" + inherited + "}"
                    ".target{position-try-fallbacks:--physical}", pixels=True)
                rectangle = colour_bounds(reference, RED)
                self.assertIsNotNone(rectangle, "the explicitly inherited physical fallback must paint")
                self.assertEqual((rectangle[2] - rectangle[0], rectangle[3] - rectangle[1]),
                                 (20, 100) if name == "size" else (10, 20),
                                 "the physical fallback must actually be chosen with the intended inherited source value")
                difference = ImageChops.difference(actual, reference).getbbox()
                self.assertIsNone(difference, f"inherit must copy its original source longhand before diagonal movement: {difference}")


    def test_parallel_relative_cells_keep_the_assigned_containing_box_after_alignment(self):
        body = '<table><tbody><tr><td><div class=marker></div><a class=target href="https://example.org/"></a></td></tr></tbody></table>'
        for root, child in (("vertical-rl", "vertical-lr"), ("vertical-lr", "vertical-rl"),
                            ("sideways-rl", "sideways-lr"), ("sideways-lr", "sideways-rl")):
            for align in ("middle", "bottom"):
                with self.subTest(root=root, cell=child, align=align):
                    rules = (f"table{{writing-mode:{root};border-spacing:0}}"
                             f"td{{position:relative;writing-mode:{child};border:2pt solid blue;padding:0;width:100pt;height:60pt;vertical-align:{align}}}"
                             ".marker{width:10pt;height:10pt;background:lime}"
                             ".target{position:absolute;right:0;bottom:0;width:12pt;height:8pt;background:red}")
                    image = writing.WritingModesPdf.render(self, root, body, rules, pixels=True)
                    left, top, right, bottom = colour_bounds(image, BLUE)
                    self.assertEqual((right - left, bottom - top), (104, 64), "the cell keeps its assigned principal box")
                    expected = (right - 14, bottom - 10, right - 2, bottom - 2)
                    self.assertEqual(colour_bounds(image, RED), expected,
                                     "local absolute content uses the full cell padding box independently of natural-content alignment")
                    navigation = writing.WritingModesPdf.render(self, root, body, rules, navigation=True)
                    self.assertEqual(len(navigation["links"]), 1, str(navigation))
                    for actual, value in zip(navigation["links"][0]["rect"],
                                             (expected[0], writing.PAGE_PT - expected[3], expected[2], writing.PAGE_PT - expected[1])):
                        self.assertAlmostEqual(actual, value, delta=writing.TOLERANCE_PT,
                                               msg="the local absolute link uses the assigned cell padding box")


    def test_auto_native_cell_containing_box_uses_the_siblings_assigned_row_extent(self):
        body = ('<table><tbody><tr><td class=short><div class=marker></div>'
                '<a class=target href="https://example.org/"></a></td>'
                '<td><div class=tall></div></td></tr></tbody></table>')
        for root, child in (("vertical-rl", "vertical-lr"), ("vertical-lr", "vertical-rl"),
                            ("sideways-rl", "sideways-lr"), ("sideways-lr", "sideways-rl")):
            for align in ("top", "middle", "bottom"):
                with self.subTest(root=root, cell=child, align=align):
                    rules = (f"table{{writing-mode:{root};border-spacing:0}}"
                             "td{padding:0;border:2pt solid transparent;height:60pt}"
                             f"td.short{{position:relative;writing-mode:{child};width:auto;border-color:blue;vertical-align:{align}}}"
                             ".marker{width:10pt;height:10pt;background:lime}.tall{width:100pt;height:10pt;background:silver}"
                             ".target{position:absolute;right:0;bottom:0;width:12pt;height:8pt;background:red}")
                    image = writing.WritingModesPdf.render(self, root, body, rules, pixels=True)
                    left, top, right, bottom = colour_bounds(image, BLUE)
                    self.assertEqual((right - left, bottom - top), (104, 64),
                                     "the sibling must actually assign a 100pt content extent to the auto cell")
                    expected = (right - 14, bottom - 10, right - 2, bottom - 2)
                    self.assertEqual(colour_bounds(image, RED), expected,
                                     "the real row extent is the absolute CB even when normal cell content remains auto-sized")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=writing.RENDERER)
    arguments = parser.parse_args()
    writing.RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
