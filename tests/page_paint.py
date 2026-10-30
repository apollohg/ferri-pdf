"""Verify native page painting through Poppler pixels, independently of layout traces."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from PIL import Image, ImageChops
from oracle import pdf_content

ROOT = Path(__file__).resolve().parents[1]
RENDERER = ROOT / "target/debug/ferri-pdf"
PAGE_BACKGROUND = (16, 32, 48)
CANVAS_BACKGROUND = (64, 80, 96)
PAGE_BORDER = (204, 34, 0)
CONTENT_BACKGROUND = (112, 128, 144)
PIXEL_TOLERANCE = 1
INK_TOLERANCE = 64
FAR_CLIP_OFFSET_PT = 1_100_000


class PagePaint(unittest.TestCase):
    def test_float_fitting_a_fresh_page_moves_with_its_entire_background(self):
        for side, probe_x in [("left", 70), ("right", 170)]:
            with self.subTest(side=side):
                pages = self.render(
                    "@page{margin:20pt;padding:0;border:0;background:white}"
                    "body{font:12pt/18pt serif}"
                    f".float{{float:{side};width:100pt;height:150pt;background:yellow}}",
                    "<div style='height:100pt'></div><div class=float>Float</div><p>Beside</p>",
                    page_size=(240, 200),
                )
                self.assertEqual(len(pages), 2)
                self.pixel(pages[0], (probe_x, 150), (255, 255, 255),
                           "a deferred float must not paint at its source position")
                for y in [40, 165]:
                    self.pixel(pages[1], (probe_x, y), (255, 255, 0),
                               "the entire fixed-height float belongs on page two")
                self.pixel(pages[1], (probe_x, 175), (255, 255, 255),
                           "float background must end before the page margin")
                text = subprocess.run(
                    ["pdftotext", str(self.output / "output.pdf"), "-"],
                    capture_output=True, text=True, check=True, timeout=60,
                ).stdout.split("\f")
                self.assertEqual(text[0].split(), ["Beside"])
                self.assertEqual(text[1].split(), ["Float"])

    def test_inline_float_uses_its_translucent_ancestors_stacking_context(self):
        for before, after in [("Text", ""), ("", "Text"), ("", "")]:
            with self.subTest(before=before, after=after):
                page = self.render(
                    "@page{margin:0;padding:0;border:0;background:white}"
                    ".layer{opacity:.5}.float{float:left;width:60pt;height:54pt;background:red}"
                    ".negative{position:absolute;left:0;top:0;width:60pt;height:54pt;background:blue;z-index:-1}"
                    ".after{height:10pt;background:lime}",
                    f"<p><span class=layer>{before}<span class=float><span class=negative></span></span>{after}</span></p><div class=after></div>",
                    page_size=(120, 100),
                )[0]
                self.pixel(page, (30, 30), (255, 128, 128),
                           "float background covers negative descendants inside the shared opacity group")
                if not before and not after:
                    self.pixel(page, (90, 5), (0, 255, 0),
                               "an otherwise empty inline context adds no line height")

    def test_float_paint_group_preserves_background_inline_and_positioned_layers(self):
        page = self.render(
            "@page{margin:0;padding:0;border:0;background:white}"
            ".float{float:left;width:60pt;height:54pt;background:red}"
            ".body{height:54pt;background:blue}"
            ".ink{display:inline-block;width:20pt;height:20pt;margin-left:-60pt;background:yellow}"
            ".above{position:absolute;left:30pt;top:0;width:10pt;height:10pt;background:lime;z-index:1}",
            "<div class=float><div class=above></div></div><p class=body><span class=ink></span></p>",
            page_size=(120, 100),
        )[0]
        for point, color, reason in [
            ((10, 10), (255, 255, 0), "inline ink paints over the float"),
            ((35, 5), (0, 255, 0), "positioned descendant escapes the float paint group"),
            ((45, 30), (255, 0, 0), "float paints over the later block background"),
            ((90, 30), (0, 0, 255), "ordinary block background remains full width"),
        ]:
            self.pixel(page, point, color, reason)

    def test_floated_table_and_caption_paint_follow_fragment_widths(self):
        red, blue, white = (255, 0, 0), (0, 0, 255), (255, 255, 255)
        pages = self.render(
            "@page{margin:36pt;padding:0;border:0;background:white}@page:left{margin-right:156pt}"
            "table{float:right;width:50%;border-spacing:0;background:red}"
            "table,caption,td{font:12pt/18pt serif}td{padding:0}"
            "caption{caption-side:bottom;text-align:left;background:blue}",
            "<table><caption>Caption</caption><tr><td>FirstOne<br>FirstTwo</td></tr>"
            "<tr><td>MiddleOne<br>MiddleTwo</td></tr><tr><td>LastOne<br>LastTwo</td></tr></table>",
            page_size=(360, 144),
        )
        self.assertEqual(len(pages), 2, "the floated table and caption occupy two pages")
        for page, point, color, reason in [
            (0, (320, 50), red, "first table fragment reaches its right edge"),
            (0, (170, 50), white, "first float's left edge stays at 180pt"),
            (1, (200, 50), red, "continued grid follows the narrower page content area"),
            (1, (110, 50), white, "continued float's left edge stays at 120pt"),
            (1, (200, 80), blue, "bottom caption shares its wrapper's continuation position"),
            (1, (200, 100), white, "table background does not extend below its caption"),
        ]:
            self.pixel(pages[page], point, color, reason)

    def test_split_cell_edges_preserve_cloned_and_final_padding_in_native_ink(self):
        reference = json.loads((ROOT / "tests/fixtures/tables/weasy70-cell-edges.json").read_text())
        blue, red, white, lime = (0, 0, 255), (255, 0, 0), (255, 255, 255), (0, 255, 0)
        probes = {
            "nested-ends": (3, [(0, (100, 105), red), (2, (100, 15), red),
                                (2, (100, 22), blue), (2, (100, 50), blue),
                                (2, (100, 60), white)]),
            "columns-sliced-end": (4, [(0, (100, 105), blue), (1, (100, 105), blue),
                                       (2, (100, 95), blue), (2, (100, 105), white)]),
            "clone-bottom": (3, [(0, (100, 95), blue), (0, (100, 105), white),
                                 (1, (100, 105), white), (2, (100, 75), blue),
                                 (2, (100, 85), white)]),
            "clone-both": (4, [(0, (100, 15), white), (0, (100, 25), blue),
                               (0, (100, 95), blue), (0, (100, 105), white),
                               (3, (100, 15), white), (3, (100, 25), blue),
                               (3, (100, 35), white)]),
            "columns-nonfinal-block": (2, [(0, (100, 105), blue), (1, (100, 35), blue),
                                            (1, (100, 45), white)]),
            "columns-first-progress": (2, [(0, (100, 103), blue), (1, (100, 85), blue),
                                           (1, (100, 95), white)]),
            "columns-final-19": (2, [(0, (100, 105), blue), (1, (100, 15), blue),
                                      (1, (100, 25), white)]),
            "columns-footer-final-47": (4, [(0, (100, 15), lime), (0, (100, 95), blue),
                                             (0, (100, 105), lime), (1, (100, 105), lime),
                                             (2, (100, 105), lime), (3, (100, 25), blue),
                                             (3, (100, 35), white), (3, (100, 45), lime)]),
            "outer-table-end": (2, [(0, (100, 105), blue), (1, (100, 15), blue), (1, (100, 25), red)]),
            "nested-table-end": (2, [(0, (100, 105), blue), (1, (100, 15), blue), (1, (100, 25), red)]),
        }
        cases = {case["name"]: case for case in reference["cases"] + reference["observations"]}
        for name, (count, points) in probes.items():
            with self.subTest(name=name):
                case = cases[name]
                css = ("@page{padding:0;border:0;background:white}"
                       "@font-face{font-family:Probe;src:url('"
                       + (ROOT / reference["font_path"]).as_uri() + "')}"
                       + reference["base_css"] + case["css"]
                       + "td:first-child{background:blue;background-clip:content-box}"
                       ".inner{background:red;background-clip:content-box}"
                       "td.outer{background:red;background-clip:border-box}"
                       "thead td:first-child,tfoot td:first-child{background:lime;background-clip:border-box}"
                       + ("section{background:red}" if name == "outer-table-end" else ""))
                pages = self.render(css, case["body"], page_size=(240, 120))
                self.assertEqual(len(pages), count, f"{name}: physical page count")
                for page, position, color in points:
                    self.pixel(pages[page], position, color,
                               f"{name} page {page}: content and decoration edge must remain distinct")
                text = subprocess.run(["pdftotext", str(self.output / "output.pdf"), "-"],
                                      capture_output=True, text=True, check=True, timeout=60).stdout
                repeated = ("Head", "Foot") if name == "columns-footer-final-47" else ()
                for index, page in enumerate(text.split("\f")[:count]):
                    for label in repeated:
                        self.assertEqual(page.split().count(label), 1,
                                         f"{name} page {index}: repeated {label} must stay with the body")
                self.assertCountEqual([word for word in text.split() if word not in repeated],
                                      [line["text"] for page in case["pages"] for line in page
                                       if line["text"] not in repeated],
                                      f"{name}: native PDF lost or duplicated authored content")

    def test_table_cell_constraints_and_percentage_distribution_reach_native_ink(self):
        reference = json.loads((ROOT / "tests/fixtures/tables/weasy70-widths.json").read_text())
        font = ROOT / reference["font_path"]
        self.assertEqual(hashlib.sha256(font.read_bytes()).hexdigest(), reference["font_sha256"])
        for case in reference["cases"]:
            with self.subTest(name=case["name"]):
                css = ("@page{padding:0;border:0;background:white}"
                       "@font-face{font-family:Probe;src:url('" + font.as_uri() + "')}"
                       + reference["base_css"] + case["css"])
                page = self.render(css, case["body"])[0]
                boundary = 20 + case["widths"]["c0"]
                self.pixel(page, (round(boundary - 3), 22), (255, 0, 0),
                           "the first cell must paint to the independently measured boundary")
                self.pixel(page, (round(boundary + 3), 22), (0, 0, 255),
                           "the second cell must start at the independently measured boundary")

    def test_initial_disabled_selectors_control_native_ink(self):
        css = ("@page{margin:20pt;padding:0;border:0;background:white}"
               "fieldset{display:none}.state{width:20pt;height:20pt;background:red}"
               "body:has(#first:enabled) #legend-result{background:#00ff00}"
               "body:has(#ordinary:disabled) #body-result{background:#0000ff}")
        body = ("<fieldset disabled><legend><input id=first></legend><input id=ordinary></fieldset>"
                "<div class=state id=legend-result></div><div class=state id=body-result></div>")
        page = self.render(css, body)[0]
        self.pixel(page, (30, 30), (0, 255, 0), "the first legend exemption must reach serialized paint")
        self.pixel(page, (30, 50), (0, 0, 255), "ordinary disabled descendants must reach serialized paint")

    def test_replaced_objects_clip_to_content_instead_of_painting_padding(self):
        source = (ROOT / "tests/oracle/assets/image-pattern.png").as_uri()
        css = ("@page{margin:20pt;padding:0;border:0;background:white}"
               "img{display:block;width:100pt;height:80pt;padding:10pt;border:2pt solid blue;"
               "background:#ffcc00;object-fit:cover}")
        page = self.render(css, f"<img src='{source}'>")[0]
        for point in [(25, 60), (136, 60), (80, 25), (80, 118)]:
            self.pixel(page, point, (255, 204, 0), "fitted replaced content must not paint its padding")
        self.pixel(page, (50, 50), (255, 0, 0), "the centered cover image must still paint inside content")

    def test_object_positions_control_native_ink_and_compose_with_curves_and_overflow(self):
        source = (ROOT / "tests/oracle/assets/image-pattern.png").as_uri()
        css = ("@page{margin:20pt;padding:0;border:0;background:white}"
               "img{display:block;width:100pt;height:80pt;padding:10pt;border:2pt solid blue;"
               "background:#ffcc00;object-fit:none}")
        yellow, white = (255, 204, 0), (255, 255, 255)
        red, blue = (255, 0, 0), (0, 0, 255)
        # The fixture's lower-right RGBA is (0,128,0,128), composited over yellow.
        green = (127, 166, 0)
        for declarations, body, points in [
            ("img{object-fit:cover;object-position:right top}", f"<img src='{source}'>",
             [((40, 50), red), ((80, 40), blue), ((80, 100), green), ((25, 60), yellow), ((136, 60), yellow)]),
            ("img{object-position:right 10pt bottom 20pt}", f"<img src='{source}'>",
             [((70, 70), red), ((110, 70), blue), ((110, 85), green), ((40, 40), yellow)]),
            ("img{object-fit:fill;object-position:10pt -5pt}", f"<img src='{source}'>",
             [((60, 50), red), ((110, 50), blue), ((110, 90), green), ((35, 50), yellow), ((60, 110), yellow)]),
            ("img{border-radius:30pt;object-position:right bottom}", f"<img src='{source}'>",
             [((80, 90), red), ((110, 90), blue), ((110, 100), green), ((130, 110), yellow)]),
            ("section{width:70pt;height:70pt;overflow:hidden}img{object-position:right bottom}",
             f"<section><img src='{source}'></section>",
             [((80, 85), red), ((100, 85), white), ((80, 95), white)]),
        ]:
            with self.subTest(declarations=declarations):
                page = self.render(css + declarations, body)[0]
                for point, color in points:
                    self.pixel(page, point, color, "positioned object ink must follow content-box placement and clips")

    def test_attribute_values_reach_native_page_pseudo_geometry_and_url_security(self):
        css=("html{--paper:attr(data-paper '<length>+');--heading:attr(data-heading)}"
             "@page{size:var(--paper);margin:10pt;padding:0;border:0;background:white;"
             "@top-center{font:8pt/10pt serif;content:var(--heading)}}"
             ".outer{--size:attr(data-size <length>);width:100pt;margin-left:20pt;background:red}"
             ".box{width:var(--size);height:attr(data-height <length>);"
             "margin-left:calc(3pt + attr(data-offset <length>));background-color:attr(data-color <color>);"
             "background-image:linear-gradient(red,blue);background-image:attr(data-asset <url>)}"
             ".box::before{content:attr(data-label);display:block;width:attr(data-line <length>);height:2pt;background:lime}"
             ".next{width:attr(data-size <length>);height:10pt;background:purple}")
        body=("<html data-paper='200pt 300pt' data-heading=Header><body>"
              "<div class=outer data-size=80pt><div class=box data-size=99pt data-height=20pt "
              "data-offset=2pt data-color=blue data-asset='url(missing-secret.png)' data-label='' data-line=30pt></div>"
              "<div class=next data-size=70pt></div></div>")
        actual=self.render(css,body,output_size=(200,300))
        self.assertEqual(len(actual),1)
        for point,color in [((40,11),(0,255,0)),((70,11),(0,0,255)),((110,20),(0,0,255)),
                            ((120,20),(255,0,0)),((135,20),(255,255,255)),
                            ((40,35),(128,0,128)),((105,35),(255,0,0))]:
            self.pixel(actual[0],point,color,"attributes must control native ink without loading an attribute-derived URL")
        literal=("html{--paper:200pt 300pt;--heading:'Header'}.outer{--size:80pt}"
                 ".box{width:80pt;height:20pt;margin-left:5pt;background-color:blue;background-image:none}"
                 ".box::before{content:'';width:30pt}.next{width:70pt}")
        expected=self.render(css+literal,body,output_size=(200,300))
        self.assertEqual(len(expected),1)
        difference=ImageChops.difference(actual[0],expected[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "attribute output must match independently authored page, pseudo and element values")

    def test_conditional_values_reach_page_pseudo_and_element_paint(self):
        css=("html{color:#123456}"
             "@page{size:if(media(print):200pt 300pt;else:400pt 500pt);margin:10pt;padding:0;border:0;background:white;"
             "@top-center{font:8pt/10pt serif;content:if(media(print):'Header';else:'Wrong')}}"
             ".outer{width:80pt;margin-left:20pt;background:red;--word:print;--n:100.00}"
             ".box{width:if(style(--n:100.0):99pt;supports(display:flex):50pt;else:70pt);"
             "margin-left:calc(3pt + if(media(print):2pt;else:99pt));height:if(style(width:50pt):20pt;else:99pt);"
             "background-color:if(style(height:20pt):blue;else:red)}"
             ".box::before{content:if(style(--word:print):'';else:'Wrong');display:block;"
             "width:if(style(height:2pt):30pt;else:99pt);height:2pt;background:lime}"
             ".next{--a:if(media(print):70pt;else:var(--b));--b:if(style(--a:70pt):10pt;else:99pt);"
             "width:var(--a);height:var(--b);background:purple}")
        body="<div class=outer><div class=box></div><div class=next></div></div>"
        actual=self.render(css,body,output_size=(200,300))
        self.assertEqual(len(actual),1)
        for point,color in [((40,11),(0,255,0)),((70,11),(0,0,255)),((80,20),(0,0,255)),
                            ((90,20),(255,0,0)),((115,20),(255,255,255)),((40,35),(128,0,128)),((105,35),(255,0,0))]:
            self.pixel(actual[0],point,color,"computed conditional branches must control native ink and geometry")
        literal=("@page{size:200pt 300pt;@top-center{content:'Header'}}"
                 ".box{width:50pt;margin-left:5pt;height:20pt;background-color:blue}"
                 ".box::before{content:'';width:30pt}.next{width:70pt;height:10pt}")
        expected=self.render(css+literal,body,output_size=(200,300))
        self.assertEqual(len(expected),1)
        difference=ImageChops.difference(actual[0],expected[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "conditional native output must match independently selected literal values")

    def test_toggle_uses_computed_parent_values_for_geometry_and_content(self):
        css=("@page{size:200pt 300pt;margin:10pt;padding:0;border:0;background:white;"
             "@top-center{font:8pt/10pt serif;content:toggle(inherit,'Header')}}"
             ".outer{width:80pt;margin-left:20pt;background:red}"
             ".box{width:toggle(80pt,50pt);margin-left:toggle(20pt,5pt);height:20pt;background-color:toggle(red,blue)}"
             ".box::before{content:toggle(inherit,'');display:block;width:toggle(50pt,30pt);height:2pt;background:lime}"
             ".next{width:toggle(80pt,70pt);height:10pt;background:purple}")
        body="<div class=outer><div class=box></div><div class=next></div></div>"
        actual=self.render(css,body,output_size=(200,300))
        self.assertEqual(len(actual),1)
        for point,color in [((40,11),(0,255,0)),((70,11),(0,0,255)),((80,20),(0,0,255)),
                            ((90,20),(255,0,0)),((115,20),(255,255,255)),((40,35),(128,0,128)),((105,35),(255,0,0))]:
            self.pixel(actual[0],point,color,"toggle() must select after the first computed parent match, including generated content")
        literal=("@page{@top-center{content:'Header'}}.box{width:50pt;margin-left:5pt;background-color:blue}"
                 ".box::before{content:'';width:30pt}.next{width:70pt}")
        expected=self.render(css+literal,body,output_size=(200,300))
        self.assertEqual(len(expected),1)
        difference=ImageChops.difference(actual[0],expected[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "computed toggle selections must match independently sized literal boxes and content")

    def test_toggle_normalizes_display_and_overflow_before_comparing(self):
        css=("@page{size:200pt 300pt;margin:10pt;padding:0;border:0;background:white}"
             ".host{position:relative;width:120pt;height:50pt}"
             ".row{position:absolute;left:0;top:0;display:toggle(inline,inline-flex,block)}"
             ".row>div{width:20pt;height:10pt}.a{background:blue}.b{background:lime}"
             ".cliphost{width:80pt;height:30pt;margin-top:10pt;overflow:auto}"
             ".crop{width:20pt;height:10pt;overflow-x:toggle(visible,clip);overflow-y:toggle(hidden,scroll)}"
             ".crop>div{width:50pt;height:10pt;background:purple}")
        body=("<div class=host><div class=row><div class=a></div><div class=b></div></div></div>"
              "<div class=cliphost><div class=crop><div></div></div></div>")
        actual=self.render(css,body,output_size=(200,300))
        self.assertEqual(len(actual),1)
        for point,color in [((15,15),(0,0,255)),((35,15),(0,255,0)),
                            ((25,75),(128,0,128)),((40,75),(255,255,255))]:
            self.pixel(actual[0],point,color,"display blockification and overflow pairing must precede computed-value comparison")
        literal=".row{display:flex}.crop{overflow:hidden}"
        expected=self.render(css+literal,body,output_size=(200,300))
        self.assertEqual(len(expected),1)
        difference=ImageChops.difference(actual[0],expected[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "normalized toggle values must match literal flex layout and clipping")

    def test_inherited_values_use_parent_and_originating_element_environments(self):
        css=("html{--sheet:200pt 300pt}@page{--sheet:400pt 500pt;size:inherit(--sheet);"
             "margin:10pt;padding:0;border:0;background:white}.outer{--w:80pt}"
             ".box{--w:120pt;width:inherit(--w);height:20pt;background:red}"
             ".box::before{content:'';display:block;width:inherit(--w);height:2pt;background:lime}"
             ".next{width:inherit(--missing,70pt);height:10pt;background:blue}")
        body="<div class=outer><div class=box></div><div class=next></div></div>"
        actual=self.render(css,body,output_size=(200,300))
        self.assertEqual(len(actual),1)
        for point,color in [((125,11),(0,255,0)),((135,11),(255,255,255)),
                            ((85,20),(255,0,0)),((95,20),(255,255,255)),
                            ((75,35),(0,0,255)),((85,35),(255,255,255))]:
            self.pixel(actual[0],point,color,"inherit() must read the parent environment, including the pseudo's originating element")
        literal="@page{size:200pt 300pt}.box{width:80pt}.box::before{width:120pt}.next{width:70pt}"
        expected=self.render(css+literal,body,output_size=(200,300))
        self.assertEqual(len(expected),1)
        difference=ImageChops.difference(actual[0],expected[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "inherited token values must match independently sized page and element boxes")

    def test_calc_mix_easing_and_deferred_values_match_literal_geometry(self):
        css=("@page{size:calc-mix(.5,100pt,300pt) calc-mix(.5,200pt,400pt);margin:10pt;padding:0;border:0;background:white}"
             ".host{width:160pt}.host>div{height:10pt}"
             ".a{width:calc-mix(.5,20pt,50%);background:red}"
             ".b{width:calc-mix(.125 by cubic-bezier(0,0,0,1),20pt,100pt);background:blue}"
             ".b::before{content:'';display:block;width:calc-mix(calc(sibling-index() / 4),0pt,20pt);height:2pt;background:lime}"
             ".c{width:calc-mix(.5 by steps(4,jump-none),10pt,70pt);background:green}"
             ".d{width:calc-mix(.875 by linear(0,.25 25% 75%,1),10pt,90pt);background:purple}"
             ".e{width:calc-mix(calc(50% / 100pt) by steps(4,end),10pt,90pt);background:black}")
        body="<div class=host><div class=a></div><div class=b></div><div class=c></div><div class=d></div><div class=e></div></div>"
        actual=self.render(css,body,output_size=(200,300))
        self.assertEqual(len(actual),1)
        for point,color in [((55,15),(255,0,0)),((65,15),(255,255,255)),((15,21),(0,255,0)),
                            ((65,25),(0,0,255)),((75,25),(255,255,255)),((55,35),(0,128,0)),
                            ((65,35),(255,255,255)),((65,45),(128,0,128)),((75,45),(255,255,255)),
                            ((75,55),(0,0,0)),((85,55),(255,255,255))]:
            self.pixel(actual[0],point,color,"interpolation and easing must resolve with the used percentage and originating-element bases")
        literal="@page{size:200pt 300pt}.a{width:50pt}.b{width:60pt}.b::before{width:10pt}.c{width:50pt}.d{width:60pt}.e{width:70pt}"
        expected=self.render(css+literal,body,output_size=(200,300))
        self.assertEqual(len(expected),1)
        difference=ImageChops.difference(actual[0],expected[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "the complete native interpolation result must match independently calculated literal sizes")

    def test_media_progress_uses_physical_media_bases_and_page_dependencies(self):
        css = ("@page{size:calc(media-progress(color,0,16) * 400pt) "
               "calc(media-progress(horizontal-viewport-segments,0,2) * 600pt);"
               "margin:10pt;padding:0;border:0;background:white}html,body{font-size:80pt}"
               ".bar{font-size:100pt;width:calc(round(down,media-progress(width,0pt,100em) * 100pt,1pt));"
               "height:calc(media-progress(color,0,16) * 40pt);background:rgb(calc(media-progress(color,0,16) * 200) 0 0)}"
               ".bar::before{content:'';display:block;width:calc(media-progress(color,0,16) * 20pt);height:2pt;background:green}"
               ".next{width:10pt;height:10pt;background:blue}"
               "@media(color > calc(media-progress(width,0pt,1000pt) * 10)){"
               ".next{width:30pt}@page{size:400pt 500pt}}")
        body = "<div class=bar></div><div class=next></div>"
        actual = self.render(css,body,output_size=(200,300))
        self.assertEqual(len(actual),1)
        for point,color in [((15,11),(0,128,0)),((30,20),(100,0,0)),((60,20),(255,255,255)),
                            ((20,35),(0,0,255)),((45,35),(255,255,255))]:
            self.pixel(actual[0],point,color,"media progress must retain physical paper, initial fonts, and indirect page dependencies")
        literal=("@page{size:200pt 300pt}.bar{width:49pt;height:20pt;background:rgb(100,0,0)}"
                 ".bar::before{width:10pt}.next{width:30pt}")
        expected=self.render(css+literal,body,output_size=(200,300))
        self.assertEqual(len(expected),1)
        difference=ImageChops.difference(actual[0],expected[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "media-derived values must match independently calculated literal geometry")

    def test_tree_counting_uses_flat_siblings_and_originating_pseudo_elements(self):
        css = ("@page{size:calc(sibling-count() * 200pt) calc(sibling-index() * 300pt);"
               "margin:0;padding:0;border:0;background:white}"
               ".bar{display:block;width:calc(sibling-count() * 30pt);height:10pt;"
               "margin-left:calc(sibling-index() * 5pt);background:rgb(calc(sibling-index() * 100) 0 0)}"
               "#c::before{display:block;content:'';width:calc(sibling-index() * 4pt);height:2pt;background:blue}")
        body = ("<div><template shadowrootmode=open><slot name=one></slot><slot name=two></slot></template>"
                "<div class=bar id=a slot=two></div><div class=bar id=b slot=one></div>"
                "<div class=bar id=c slot=two></div></div>")
        actual = self.render(css, body, output_size=(200,300))
        self.assertEqual(len(actual),1)
        for point,color in [((10,5),(100,0,0)),((40,5),(255,255,255)),
                            ((50,15),(100,0,0)),((68,15),(255,255,255)),
                            ((14,21),(0,0,255)),((60,25),(200,0,0)),((75,25),(255,255,255))]:
            self.pixel(actual[0],point,color,"flat sibling groups and the pseudo's originating element must control native ink")
        literal = ("@page{size:200pt 300pt}.bar{width:60pt;margin-left:5pt;background:rgb(100,0,0)}"
                   "#b{width:30pt}#c{margin-left:10pt;background:rgb(200,0,0)}#c::before{width:8pt}")
        expected = self.render(css+literal,body,output_size=(200,300))
        self.assertEqual(len(expected),1)
        difference = ImageChops.difference(actual[0],expected[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "tree-counting functions must match the independently sized literal boxes")

    def test_comma_argument_wrappers_preserve_native_lists_and_fallbacks(self):
        css = ("@page{size:first-valid({invalid-size},{200pt 300pt});margin:var(--missing,{10pt});"
               "padding:0;border:0;background:white}"
               ".box{width:first-valid({red},{80pt});height:var(--missing,{20pt});"
               "background-image:first-valid({linear-gradient(to right,transparent 50%,red 50%),"
               "linear-gradient(blue,blue)},none)}"
               ".next{width:var(--missing,{}) 30pt;height:10pt;background:green}")
        body = "<div class=box></div><div class=next></div>"
        actual = self.render(css,body,output_size=(200,300))
        self.assertEqual(len(actual),1)
        for point,color in [((20,20),(0,0,255)),((70,20),(255,0,0)),
                            ((20,35),(0,128,0)),((45,35),(255,255,255))]:
            self.pixel(actual[0],point,color,"argument wrappers must preserve the complete selected list and fallback tokens")
        expected = self.render(css + "@page{size:200pt 300pt;margin:10pt}"
                               ".box{width:80pt;height:20pt;background-image:linear-gradient(to right,transparent 50%,red 50%),linear-gradient(blue,blue)}"
                               ".next{width:30pt}",body,output_size=(200,300))
        self.assertEqual(len(expected),1)
        difference=ImageChops.difference(actual[0],expected[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "wrapped arguments must match the literal page and layered background reference")

    def test_first_valid_controls_native_page_and_property_values(self):
        css = ("@page{size:first-valid(invalid-size,200pt 300pt);margin:first-valid(red,10pt);"
               "padding:0;border:0;background:white;bleed:first-valid(red,4pt);marks:first-valid(red,none)}"
               ".box{width:first-valid(red,calc(50% - 10pt));height:first-valid(red,20pt);"
               "background:first-valid(invalid-background,red);border:first-valid(invalid-border,2pt solid blue)}"
               ".template{--choice:first-valid(red,30pt);width:var(--choice);height:10pt;background:var(--choice)}")
        body = "<div class=box></div><div class=template></div>"
        actual = self.render(css, body, output_size=(208,308))
        self.assertEqual(len(actual),1)
        for point, color in [((14,14),(0,0,255)),((20,20),(255,0,0)),((97,20),(0,0,255)),
                             ((99,20),(255,255,255)),((20,42),(255,0,0)),((45,42),(255,255,255))]:
            self.pixel(actual[0],point,color,"selected page, shorthand, deferred, and custom-property values must reach native paint")
        document = pdf_content.Document(self.output / "output.pdf")
        page = document.pages()[0]
        self.assertEqual(document.resolve(page['MediaBox']),[-4,-4,204,304])
        self.assertEqual(document.resolve(page['TrimBox']),[0,0,200,300])
        reference_css = (css + "@page{size:200pt 300pt;margin:10pt;bleed:4pt;marks:none}"
                         ".box{width:calc(50% - 10pt);height:20pt;background:red;border:2pt solid blue}"
                         ".template{width:30pt;background:red}")
        expected = self.render(reference_css,body,output_size=(208,308))
        self.assertEqual(len(expected),1)
        difference = ImageChops.difference(actual[0],expected[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "first-valid must match the independent literal page and property reference")
        marked = self.render(css + "@page{marks:first-valid(red,crop)}",body,output_size=(244,344))
        document = pdf_content.Document(self.output / "output.pdf")
        self.assertEqual(document.resolve(document.pages()[0]['MediaBox']),[-22,-22,222,322])
        self.assertNotEqual(marked[0].crop((0,0,244,22)).getextrema(),((255,255),)*3,
                            "the selected crop marks must paint beyond the trim and bleed")
        reference = self.render(reference_css + "@page{marks:crop}",body,output_size=(244,344))
        difference = ImageChops.difference(marked[0],reference[0])
        self.assertLessEqual(max(high for _,high in difference.getextrema()),PIXEL_TOLERANCE,
                             "selected printer marks must match literal marks")

    def test_large_finite_math_matches_literal_positions_angles_and_flex_weights(self):
        css = ("@page{margin:0;padding:0;border:0;background:white}"
               ".host{position:relative;left:calc(20000000pt);width:0;height:20pt}"
               ".ink{position:absolute;left:-20000000pt;width:32pt;height:20pt;background:red}"
               ".edge{left:calc(infinity * 1pt)}.edge .ink{left:-3.4028235e38pt}"
               ".row{display:flex;width:96pt;height:20pt}"
               ".large{flex:calc(20000000) 0 0;background:blue}"
               ".small{flex:10000000 0 0;background:green}"
               ".turn{position:absolute;left:100pt;top:20pt;width:20pt;height:10pt;"
               "transform-origin:0 0;transform:rotate(calc(20000000deg));background:purple}")
        body = ("<div class=host><div class=ink></div></div>"
                "<div class=row><div class=large></div><div class=small></div></div>"
                "<div class='host edge'><div class=ink></div></div><div class=turn></div>")
        actual = self.render(css, body)
        expected = self.render(css + ".host{left:20000000pt}.edge{left:3.4028235e38pt}.large{flex-grow:20000000}"
                               ".turn{transform:rotate(20000000deg)}", body)
        self.assertEqual(len(actual), 1)
        self.assertEqual(len(expected), 1)
        for point, color in [((10, 10), (255, 0, 0)), ((60, 30), (0, 0, 255)), ((80, 30), (0, 128, 0)),
                             ((10, 50), (255, 0, 0))]:
            with self.subTest(point=point):
                self.pixel(actual[0], point, color, "wrapping a finite value in calc must preserve its layout effect")
        difference = ImageChops.difference(actual[0], expected[0])
        self.assertLessEqual(max(high for _, high in difference.getextrema()), PIXEL_TOLERANCE,
                             "large math lengths, angles, and numbers must match their literal forms")

    def test_math_special_values_match_independent_literal_widths(self):
        cases = [
            ("calc(7pt + pow(NaN, 0) * 1pt)", 0),
            ("calc(7pt + pow(1, infinity) * 1pt)", 0),
            ("calc(7pt + sign(hypot(infinity, NaN)) * 1pt)", 0),
            ("calc(7pt + sign(round(infinity, NaN)) * 1pt)", 0),
            ("calc(20pt + sign(1 / mod(-4, 2)) * 10pt)", 30),
            ("calc(20pt + sign(1 / min(0, -1 * 0)) * 10pt)", 10),
            ("calc(20pt + sign(log(0, .5)) * 10pt)", 10),
            ("calc(round(up, 1e-30pt, 1e30pt) / 1e30pt * 20pt)", 20),
            ("calc(round(1pt, 1e-40pt) * 20)", 20),
            ("calc(7pt + pow(0% / 0pt, 0) * 1pt)", 0),
            ("calc(7pt + sign(log((50% - 50%) / 1pt, .5)) * 1pt)", 6),
            ("calc(round(up, 50% * 1e-30, 1e30pt) / 1e30pt * 20pt)", 20),
        ]
        css = ("@page{margin:0;padding:0;border:0;background:white}"
               ".case{height:10pt;background:red}")
        rules = "".join(f".case{index}{{width:{source}}}" for index, (source, _) in enumerate(cases))
        body = "".join(f"<div class='case case{index}'></div>" for index in range(len(cases)))
        actual = self.render(css + rules, body)
        reference_rules = "".join(f".case{index}{{width:{width}pt}}" for index, (_, width) in enumerate(cases))
        expected = self.render(css + reference_rules, body)
        self.assertEqual(len(actual), 1)
        self.assertEqual(len(expected), 1)
        for index, (source, width) in enumerate(cases):
            with self.subTest(expression=source):
                window = (0, index * 10, 100, (index + 1) * 10)
                difference = ImageChops.difference(actual[0].crop(window), expected[0].crop(window))
                self.assertLessEqual(max(high for _, high in difference.getextrema()), PIXEL_TOLERANCE,
                                     f"{source} must paint the independently calculated width {width}pt")

    def test_integer_math_orders_boxes_in_the_full_integer_domain(self):
        pages = self.render("@page{margin:0;padding:0;border:0;background:white}"
                            ".row{display:flex;width:60pt}.row>div{width:20pt;height:20pt}"
                            ".large{order:calc(20000000);background:red}"
                            ".middle{order:15000000;background:blue}"
                            ".round{order:calc(8388609);background:green}"
                            ".next{order:8388610;background:yellow}",
                            "<div class=row><div class=large></div><div class=middle></div></div>"
                            "<div class=row><div class=next></div><div class=round></div></div>")
        self.assertEqual(len(pages), 1)
        for point, expected in [((10, 10), (0, 0, 255)), ((30, 10), (255, 0, 0)),
                                ((10, 30), (0, 128, 0)), ((30, 30), (255, 255, 0))]:
            with self.subTest(point=point):
                self.pixel(pages[0], point, expected, "calculated integer order must agree with literal integer order")

    def test_progress_drives_native_deferred_geometry_and_alpha(self):
        css = ("@page{margin:0;padding:0;border:0;background:white}html{font-size:12pt;line-height:30pt}"
                            ".wide{width:200pt}.narrow{width:100pt}.zero{width:0}"
                            ".case{width:calc(progress(50%,0pt,200pt)*100pt);height:20pt;"
                            "background:red;opacity:progress(1,0,2)}"
                            ".equal{width:calc(7pt + progress(2,2,2)*1pt);height:20pt;background:blue}"
                            ".dynamic{font-size:20pt;width:80pt;height:20pt;"
                            "background:rgb(calc(1em/1rem*100) 0 0);"
                            "border:2pt solid rgb(calc(progress(2em,1rem,4rem)*255) 0 0)}"
                            ".gradient{font-size:20pt;width:20pt;height:20pt;"
                            "background-image:linear-gradient(rgb(calc(1em/1rem*100) 0 0),rgb(calc(1em/1rem*100) 0 0))}"
                            ".shadow{font-size:20pt;width:10pt;height:10pt;"
                            "box-shadow:12pt 0 0 rgb(calc(1em/1rem*100) 0 0)}"
                            ".line{font-size:20pt;line-height:40pt;width:20pt;height:20pt;"
                            "color:rgb(calc(1lh/1rlh*100) 0 0);background:currentcolor}")
        body = ("<div class=wide><div class=case></div></div>"
                            "<div class=narrow><div class=case></div></div>"
                            "<div class=zero><div class=case></div></div><div class=equal></div>"
                            "<div class=dynamic></div><div class=gradient></div><div class=shadow></div>"
                            "<div class=line></div>")
        pages = self.render(css, body)
        self.assertEqual(len(pages), 1)
        for point in [(10, 10), (49, 10), (10, 30), (24, 30)]:
            self.pixel(pages[0], point, (255, 128, 128), "progress computes geometry and alpha with the actual basis")
        for point in [(51, 10), (26, 30), (1, 50), (7, 70)]:
            self.pixel(pages[0], point, (255, 255, 255), "computed edges and a zero basis clip no extra pixels")
        self.pixel(pages[0], (5, 70), (0, 0, 255), "equal progress endpoints return zero before surrounding addition")
        for point, expected, reason in [
            ((1, 81), (198, 0, 0), "border progress uses the element and root fonts"),
            ((3, 83), (167, 0, 0), "background math uses the element and root fonts"),
            ((10, 114), (167, 0, 0), "gradient channels use the element and root fonts"),
            ((16, 129), (167, 0, 0), "shadow channels use the element and root fonts"),
            ((10, 144), (133, 0, 0), "foreground math uses the computed element line height"),
        ]:
            self.pixel(pages[0], point, expected, reason)
        reference = self.render(css + ".wide .case{width:50pt;opacity:.5}.narrow .case{width:25pt;opacity:.5}"
                                ".zero .case{width:0;opacity:.5}.equal{width:7pt}"
                                ".dynamic{background:rgb(167 0 0);border-color:rgb(198 0 0)}"
                                ".gradient{background-image:linear-gradient(rgb(167 0 0),rgb(167 0 0))}"
                                ".shadow{box-shadow:12pt 0 0 rgb(167 0 0)}.line{color:rgb(133 0 0)}", body)
        self.assertEqual(len(reference), 1)
        difference = ImageChops.difference(pages[0], reference[0])
        self.assertLessEqual(max(high for _, high in difference.getextrema()), PIXEL_TOLERANCE,
                             "math output must match independently calculated literal geometry, alpha, and colours")

    def test_open_overflow_axis_preserves_transformed_ink_and_links(self):
        for opacity in [1, 0.5]:
            for axis in [0, 1]:
                for bleed in [0, 6]:
                    with self.subTest(opacity=opacity, axis=axis, bleed=bleed):
                        overflow = "overflow-x:visible;overflow-y:clip" if axis == 0 else "overflow-x:clip;overflow-y:visible"
                        size = "width:5pt;height:25pt" if axis == 0 else "width:25pt;height:5pt"
                        base = (f"@page{{margin:0;padding:0;border:0;background:white;bleed:{bleed}pt}}"
                                f".host{{position:absolute;left:0;top:0;{size};{overflow};opacity:{opacity}}}"
                                ".child{position:absolute;left:10pt;top:10pt;width:20pt;height:20pt;"
                                "background:red;border-radius:4pt}")
                        body = "<div class=host><a class=child href='https://example.org/'></a></div>"
                        output_size = (200 + 2 * bleed, 300 + 2 * bleed)
                        reference = self.render(base, body, (200, 300), output_size=output_size)[0]
                        self.pixel(reference, (20 + bleed, 20 + bleed),
                                   (255, 0, 0) if opacity == 1 else (255, 128, 128),
                                   "reference content must visibly cross the open edge")
                        closed_edge = (20 + bleed, 26 + bleed) if axis == 0 else (26 + bleed, 20 + bleed)
                        self.pixel(reference, closed_edge, (255, 255, 255), "the other axis must still clip")
                        move = ((f"transform:translateX(-{FAR_CLIP_OFFSET_PT}pt)", f"left:{FAR_CLIP_OFFSET_PT + 10}pt")
                                if axis == 0 else
                                (f"transform:translateY(-{FAR_CLIP_OFFSET_PT}pt)", f"top:{FAR_CLIP_OFFSET_PT + 10}pt"))
                        pages = self.render(base + f".host{{{move[0]}}}.child{{{move[1]}}}", body,
                                            (200, 300), output_size=output_size)
                        difference = ImageChops.difference(reference, pages[0])
                        self.assertLessEqual(max(high for _, high in difference.getextrema()), PIXEL_TOLERANCE,
                                             "open-axis overflow must not acquire a coordinate limit")
                        document = pdf_content.Document(self.output / "output.pdf")
                        annotations = document.resolve(document.pages()[0].get("Annots", []))
                        self.assertEqual(len(annotations), 1, "visible transformed links retain their hit area")
                        expected = [10, 275, 30, 290] if axis == 0 else [10, 270, 25, 290]
                        self.assertEqual(document.resolve(document.resolve(annotations[0])["Rect"]), expected)

    def test_singular_open_axis_clips_remain_empty_inside_opacity_groups(self):
        for opacity in [1, 0.5]:
            with self.subTest(opacity=opacity):
                pages = self.render("@page{margin:0;padding:0;border:0;background:white}"
                                    ".host{position:absolute;width:5pt;height:25pt;"
                                    f"overflow-x:visible;overflow-y:clip;transform:scaleX(0);opacity:{opacity}}}"
                                    ".child{position:absolute;left:10pt;top:10pt;width:20pt;height:20pt;background:red}",
                                    "<div class=host><a class=child href='https://example.org/'></a></div>", (200, 300))
                self.assertEqual(pages[0].getextrema(), ((255, 255),) * 3)
                document = pdf_content.Document(self.output / "output.pdf")
                self.assertEqual(document.resolve(document.pages()[0].get("Annots", [])), [])

    def test_fragment_clips_preserve_translated_decorations_without_a_coordinate_limit(self):
        css = ("@page{margin:0;padding:0;border:0;background:white}"
               ".frame{position:relative;width:100pt}"
               ".box{width:80pt;height:180pt;border:5pt dashed #cc2200;border-radius:14pt;"
               "background:linear-gradient(#102030,#708090);box-shadow:3pt 2pt 4pt #405060}")
        body = "<div class=frame><div class=box></div></div>"
        reference = self.render(css, body, (160, 80))
        self.assertEqual(len(reference), 3)
        shifted = self.render(css + f".frame{{left:{FAR_CLIP_OFFSET_PT}pt;transform:translateX(-{FAR_CLIP_OFFSET_PT}pt)}}", body, (160, 80))
        self.assertEqual(len(shifted), len(reference))
        for index, (expected, actual) in enumerate(zip(reference, shifted)):
            difference = ImageChops.difference(expected, actual)
            self.assertLessEqual(max(high for _, high in difference.getextrema()), PIXEL_TOLERANCE,
                                 f"page {index}: whole-box decoration clips must retain open edges")

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ferri-pdf-page-paint-")
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name)

    def render(self, css, body, page_size=(400, 600), output_size=None):
        source, target = self.output / "input.html", self.output / "output.pdf"
        source.write_text(
            "<!doctype html><style>"
            f"@page{{size:{page_size[0]}pt {page_size[1]}pt;margin:20pt;padding:10pt;"
            "border:2pt solid #cc2200;background:#102030}"
            "html,body,p{margin:0;padding:0}"
            f"{css}</style>{body}"
        )
        result = subprocess.run(
            [str(RENDERER), str(source), str(target)],
            capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        raster = self.output / "page"
        for path in self.output.glob("page-*.png"):
            path.unlink()
        subprocess.run(
            ["pdftoppm", "-r", "72", "-png", str(target), str(raster)],
            capture_output=True, text=True, check=True, timeout=60,
        )
        pages = []
        for path in sorted(self.output.glob("page-*.png")):
            with Image.open(path) as image:
                pages.append(image.convert("RGB"))
        self.assertTrue(pages, "native PDF must produce a page raster")
        self.assertTrue(all(page.size == (output_size or page_size) for page in pages),
                        f"raster sizes {[page.size for page in pages]}, expected {output_size or page_size}")
        return pages

    def pixel(self, page, position, color, reason):
        actual = page.getpixel(position)
        self.assertTrue(
            all(abs(got - expected) <= PIXEL_TOLERANCE
                for got, expected in zip(actual, color)),
            f"{reason}: pixel {position} is {actual}, expected {color}",
        )

    def pdf_dictionaries(self):
        document = pdf_content.Document(self.output / "output.pdf")
        dictionaries = []
        for number in set(document.offsets) | set(document.compressed):
            obj = document.get(number)
            dictionaries.append(obj.entries if isinstance(obj, pdf_content.Stream) else obj)
        return [obj for obj in dictionaries if isinstance(obj, dict)]

    def assert_vector_gradient(self):
        dictionaries = self.pdf_dictionaries()
        subtypes = [obj.get("Subtype") for obj in dictionaries]
        self.assertNotIn("Image", subtypes, "gradient tiles must remain vector graphics")
        self.assertTrue(any(isinstance(obj, dict) and "ShadingType" in obj for obj in dictionaries))

    def test_page_canvas_border_content_and_margin_order(self):
        pages = self.render(
            "@page{background-clip:content-box;"
            "@top-left{content:'';width:50pt;background:#ffee00}}"
            "html{background:#405060}"
            "p{width:100pt;height:12pt;background:#708090}",
            "<p></p>",
        )
        self.assertEqual(len(pages), 1, "page paint must not generate extra pages")
        page = pages[0]
        for position, color, reason in [
            ((5, 5), PAGE_BACKGROUND, "page background includes margins"),
            ((25, 10), (255, 238, 0), "margin box paints above the page"),
            ((25, 50), CANVAS_BACKGROUND, "canvas covers page padding"),
            ((21, 50), PAGE_BORDER, "border paints above canvas"),
            ((40, 40), CONTENT_BACKGROUND, "content paints above page layers"),
        ]:
            self.pixel(page, position, color, reason)

    def margin_box(self, declarations, extra="", body=""):
        return self.render(
            "@page{margin:40pt;padding:0;border:0;background:white;"
            f"@top-left{{{declarations}}}{extra}}}",
            body,
            page_size=(200, 200),
        )[0]

    def ink(self, page, box, color):
        """Pixels within `box` (left, top, right, bottom) near `color`."""
        region = page.crop(box)
        return sum(
            count
            for count, pixel in region.getcolors(region.width * region.height)
            if all(abs(got - expected) <= INK_TOLERANCE for got, expected in zip(pixel, color))
        )

    def test_page_margin_box_visibility_overflow_shadow_and_stacking_reach_native_ink(self):
        red, blue, lime, white = (255, 0, 0), (0, 0, 255), (0, 255, 0), (255, 255, 255)
        hidden = self.margin_box(
            "content:'WWW';font:20pt serif;color:red;background:blue;visibility:hidden",
            "@top-right{content:'';width:40pt;background:lime}",
        )
        self.pixel(hidden, (45, 5), white, "a hidden margin box paints no background")
        self.assertEqual(self.ink(hidden, (40, 0, 100, 40), red), 0,
                         "a hidden margin box paints none of its generated text")
        self.pixel(hidden, (155, 5), lime, "a visible sibling margin box still paints")
        for overflow, expected in [("visible", True), ("hidden", False), ("clip", False)]:
            with self.subTest(overflow=overflow):
                page = self.margin_box(
                    "content:'WWWWWWWWWW';white-space:nowrap;width:30pt;font:30pt serif;"
                    f"color:blue;overflow:{overflow}"
                )
                self.assertGreater(self.ink(page, (40, 0, 70, 40), blue), 0,
                                   "text inside the margin box paints")
                self.assertEqual(self.ink(page, (75, 0, 160, 40), blue) > 0, expected,
                                 f"overflow:{overflow} decides whether text past the box paints")
        shadowed = self.margin_box("content:'W';font:30pt serif;color:red;"
                                   "text-shadow:60pt 0 blue")
        self.assertGreater(self.ink(shadowed, (100, 0, 160, 40), blue), 0,
                           "margin box text paints its text shadow")
        for z_index, overlap, reason in [
            ("auto", blue, "top-center paints after top-left in the default order"),
            ("2", red, "a higher z-index paints top-left over top-center"),
        ]:
            with self.subTest(z_index=z_index):
                page = self.margin_box(
                    f"content:'';background:red;margin-right:-60pt;z-index:{z_index}",
                    "@top-center{content:'';width:60pt;background:blue}",
                )
                self.pixel(page, (85, 20), overlap, reason)
        behind = self.margin_box(
            "content:'';background:red;margin-bottom:-60pt;z-index:-1",
            body="<div style='height:120pt;background:blue'></div>",
        )
        self.pixel(behind, (50, 20), red, "a negative z-index margin box paints in its margin")
        self.pixel(behind, (60, 80), blue, "document content paints over a negative z-index margin box")

    def test_page_margin_box_effects_reach_native_ink(self):
        white, red = (255, 255, 255), (255, 0, 0)
        for declarations, extra, probes in [
            ("opacity:.5", "", [((60, 20), (255, 128, 128), "opacity composites the box")]),
            ("filter:grayscale(1)", "", [((60, 20), (54, 54, 54), "filters apply to the box")]),
            ("mix-blend-mode:multiply", "background:lime;",
             [((60, 20), (0, 0, 0), "the box blends with the page background")]),
            ("clip-path:inset(0 50% 0 0)", "",
             [((50, 20), red, "the clip path keeps the left half"),
              ((140, 20), white, "the clip path removes the right half")]),
            ("mask-image:linear-gradient(transparent,transparent)", "",
             [((60, 20), white, "a transparent mask hides the box")]),
            ("transform:translate(20pt,60pt)", "",
             [((60, 20), white, "the transform moves the box off its margin"),
              ((70, 80), red, "the transform places the box over the page area")]),
            ("backdrop-filter:invert(1);background:transparent", "background:lime;",
             [((60, 20), (255, 0, 255), "the backdrop filter inverts the page behind the box")]),
        ]:
            with self.subTest(declarations=declarations):
                page = self.render(
                    f"@page{{margin:40pt;padding:0;border:0;{extra or 'background:white;'}"
                    f"@top-left{{content:'';background:red;{declarations}}}}}",
                    "", page_size=(200, 200),
                )[0]
                for position, color, reason in probes:
                    self.pixel(page, position, color, reason)

    def test_footnote_area_visibility_and_effects_reach_native_ink(self):
        white, red, blue = (255, 255, 255), (255, 0, 0), (0, 0, 255)
        for declarations, page_background, probes in [
            ("visibility:hidden", "white", [((150, 172), white, "a hidden area paints no background")]),
            ("opacity:.5", "white", [((150, 172), (255, 128, 128), "opacity composites the area")]),
            ("filter:grayscale(1)", "white", [((150, 172), (54, 54, 54), "filters apply to the area")]),
            ("clip-path:inset(0 50% 0 0)", "white",
             [((60, 172), red, "the clip path keeps the left half"),
              ((150, 172), white, "the clip path removes the right half")]),
            ("mask-image:linear-gradient(transparent,transparent)", "white",
             [((150, 172), white, "a transparent mask hides the area")]),
            ("mix-blend-mode:multiply", "lime", [((150, 172), (0, 0, 0), "the area blends with the page")]),
            ("backdrop-filter:invert(1);background:transparent", "lime",
             [((150, 172), (255, 0, 255), "the backdrop filter inverts the page behind the area")]),
        ]:
            with self.subTest(declarations=declarations):
                page = self.render(
                    f"@page{{margin:20pt;padding:0;border:0;background:{page_background};"
                    f"@footnote{{background:red;{declarations}}}}}"
                    ".note{float:footnote;color:blue}",
                    "<p>Text<span class=note>Note</span></p>", page_size=(200, 200),
                )[0]
                for position, color, reason in probes:
                    self.pixel(page, position, color, reason)
                if declarations == "visibility:hidden":
                    self.assertGreater(self.ink(page, (20, 160, 100, 180), blue), 0,
                                       "footnote bodies keep their own visibility")

    def test_hidden_page_box_paints_neither_itself_nor_inheriting_margin_boxes(self):
        page = self.render(
            "@page{visibility:hidden;background:blue;border:4pt solid red;outline:4pt solid red;"
            "@top-left{content:'';background:red}"
            "@top-right{content:'';background:lime;visibility:visible}}"
            "div{height:20pt;background:yellow}",
            "<div></div>", page_size=(200, 200),
        )[0]
        for position, color, reason in [
            ((5, 5), (255, 255, 255), "the hidden page box paints no background"),
            ((21, 50), (255, 255, 255), "the hidden page box paints no border"),
            ((40, 10), (255, 255, 255), "margin boxes inherit the page's visibility"),
            ((160, 10), (0, 255, 0), "a margin box can be made visible again"),
            ((60, 40), (255, 255, 0), "document content does not inherit from the page"),
        ]:
            self.pixel(page, position, color, reason)

    def test_propagated_image_uses_actual_root_content_origin(self):
        for source in ("html", "body"):
            with self.subTest(source=source):
                page = self.render(
                    "@page{border-color:transparent}"
                    "html{width:120pt;height:80pt;padding:7pt;"
                    "border:3pt solid transparent}"
                    f"{source}{{background:linear-gradient(red,red) "
                    "content-box no-repeat;background-size:10pt 10pt}",
                    "<p></p>",
                )[0]
                self.pixel(page, (45, 45), (255, 0, 0), "root content origin includes root edges")
                self.pixel(page, (35, 35), PAGE_BACKGROUND, "canvas origin is not the page origin")
                self.pixel(page, (55, 45), PAGE_BACKGROUND, "image keeps its authored size")

    def test_rounded_page_border_and_canvas_clip(self):
        page = self.render(
            "@page{border-radius:20pt}html{background:#405060}", "",
        )[0]
        self.pixel(page, (21, 21), PAGE_BACKGROUND, "canvas and border respect the rounded corner")
        self.pixel(page, (21, 50), PAGE_BORDER, "straight border remains visible")
        self.pixel(page, (45, 45), CANVAS_BACKGROUND, "canvas paints inside the curve")

    def test_constrained_page_box_does_not_paint_unused_sheet_space(self):
        page = self.render("@page{width:200pt;height:300pt}", "")[0]
        self.pixel(page, (250, 350), PAGE_BACKGROUND, "background covers the 264pt by 364pt margin box")
        self.pixel(page, (300, 350), (255, 255, 255), "unused requested sheet width remains outside the page box")
        self.pixel(page, (250, 400), (255, 255, 255), "unused requested sheet height remains outside the page box")

    def test_page_border_image_replaces_border_style_on_each_page(self):
        pages = self.render(
            "@page{border-image:linear-gradient(#00aa00,#00aa00) 1}"
            "html{background:#405060}p{height:12pt}"
            ".next{break-before:page}",
            "<p></p><p class=next></p>",
        )
        self.assertEqual(len(pages), 2)
        for page in pages:
            self.pixel(page, (21, 50), (0, 170, 0), "border image replaces red border")
            self.pixel(page, (25, 50), CANVAS_BACKGROUND, "border image leaves canvas visible")
            self.pixel(page, (5, 5), PAGE_BACKGROUND, "page background repeats on every page")

    def test_canvas_gradient_keeps_the_slice_across_a_forced_break(self):
        for decoration in ("slice", "clone"):
            with self.subTest(decoration=decoration):
                pages = self.render(
                    "html{padding:7pt;border:3pt solid transparent;"
                    "background:linear-gradient(red,blue) content-box no-repeat;"
                    f"box-decoration-break:{decoration}}}"
                    "p{height:12pt}.next{break-before:page}",
                    "<p></p><p class=next></p>",
                )
                self.assertEqual(len(pages), 2)
                if decoration == "slice":
                    gradient_height, gradient_top = 538.0, -494.0
                    progress = (35.5 - gradient_top) / gradient_height
                    color = (round(255 * (1 - progress)), 0, round(255 * progress))
                else:
                    color = PAGE_BACKGROUND
                self.pixel(pages[1], (50, 35), color,
                           "slice continues the unbroken image; clone restarts after its top edges")

    def test_canvas_images_ignore_root_and_body_transforms(self):
        cases = [
            ("scale(.5)", (38, 38)),
            ("translate(20pt,10pt)", (65, 55)),
            ("matrix(1,.5,.25,1,0,0)", (70, 84)),
            ("rotate(90deg) translate(10pt,-50pt)", (65, 65)),
            ("translateX(100pt) scale(-1,1)", (110, 50)),
        ]
        for source in ("html", "body"):
            for transform, transformed_only in cases:
                with self.subTest(source=source, transform=transform):
                    page = self.render(
                        "html{width:120pt;height:80pt;padding:7pt;"
                        "border:3pt solid transparent;transform-origin:0 0;"
                        f"transform:{transform}}}"
                        f"{source}{{background:linear-gradient(red,red) "
                        "content-box no-repeat;background-size:20pt 30pt}"
                        "body{transform:translateX(70pt)}", "<p></p>",
                    )[0]
                    self.pixel(page, (45, 50), (255, 0, 0), "propagated image keeps its original root positioning area")
                    self.pixel(page, transformed_only, PAGE_BACKGROUND, "root and body transforms do not move canvas images")
                    self.pixel(page, (21, 50), PAGE_BORDER, "page border is not transformed")

    def test_canvas_repeats_keep_their_pitch_under_root_transforms(self):
        for alpha in (1.0, 0.5):
            with self.subTest(alpha=alpha):
                page = self.render(
                    "html{width:120pt;height:80pt;padding:7pt;border:3pt solid transparent;"
                    "transform-origin:0 0;transform:scale(.5);"
                    f"background:#405060 linear-gradient(to right,rgba(255,0,0,{alpha}) 50%,rgba(0,0,255,{alpha}) 50%) content-box;"
                    "background-size:20pt 20pt}", "",
                )[0]
                red, blue = [tuple(round(a * alpha + b * (1 - alpha)) for a, b in zip(color, CANVAS_BACKGROUND)) for color in [(255, 0, 0), (0, 0, 255)]]
                self.pixel(page, (351, 555), red, "canvas pattern repeats beyond the root box")
                self.pixel(page, (355, 555), blue, "canvas keeps its authored 20pt pitch")
                self.pixel(page, (345, 555), red, "canvas pitch is independent of the root scale")
                self.pixel(page, (25, 555), PAGE_BACKGROUND, "content-box canvas clip stays in page coordinates")
                self.assert_vector_gradient()

    def test_canvas_single_axis_repeats_ignore_root_rotation(self):
        for repeat, inside, outside in [
            ("repeat-x", (100, 55), (55, 555)),
            ("repeat-y", (55, 120), (350, 60)),
        ]:
            with self.subTest(repeat=repeat):
                page = self.render(
                    "html{width:120pt;height:80pt;padding:7pt;border:3pt solid transparent;"
                    "transform-origin:0 0;transform:rotate(90deg) translate(10pt,-50pt);"
                    f"background:linear-gradient(red,red) content-box {repeat};"
                    "background-size:20pt 30pt}", "",
                )[0]
                self.pixel(page, inside, (255, 0, 0), "canvas repeat axes stay in page coordinates")
                self.pixel(page, outside, PAGE_BACKGROUND, "root rotation does not rotate the canvas strip")

    def test_fixed_canvas_ignores_fragment_transforms_and_keeps_page_positioning(self):
        pages = self.render(
            "html{width:120pt;padding:7pt;border:3pt solid transparent;transform:scale(.5);"
            "background:linear-gradient(red,red) content-box fixed right bottom no-repeat;"
            "background-size:20pt 30pt}p{height:12pt}.next{break-before:page}",
            "<p></p><p class=next></p>",
        )
        self.assertEqual(len(pages), 2)
        for page, transformed_only in zip(pages, [(230, 425), (230, 295)]):
            self.pixel(page, (350, 545), (255, 0, 0), "fixed canvas tile keeps page positioning on every fragment")
            self.pixel(page, transformed_only, PAGE_BACKGROUND, "fragment transform origins do not move canvas images")

    def test_singular_root_keeps_canvas_color_and_images_but_hides_content(self):
        for background in ("red", "red linear-gradient(blue,blue)"):
            with self.subTest(background=background):
                page = self.render(f"html{{width:120pt;height:80pt;transform:scale(0);background:{background}}}p{{font:8pt/12pt serif}}", "<p>Hidden</p>")[0]
                color = (255, 0, 0) if background == "red" else (0, 0, 255)
                self.pixel(page, (50, 50), color, "canvas background is independent of root invertibility")
                self.pixel(page, (92, 72), color, "collapsed document text must not cover the canvas")
                self.pixel(page, (21, 50), PAGE_BORDER, "page paint remains independent of the root")

    def test_transformed_element_gradient_tiles_keep_their_clip_and_opacity(self):
        for opacity in (1.0, 0.5):
            with self.subTest(opacity=opacity):
                page = self.render(
                    "div{width:100pt;height:80pt;padding:10pt;transform:scale(.5);"
                    "transform-origin:0 0;background:linear-gradient(to right,red 50%,blue 50%);"
                    f"background-size:20pt 20pt;opacity:{opacity}}}", "<div></div>",
                )[0]
                for position, color in [((45, 50), (255, 0, 0)), ((49, 50), (0, 0, 255))]:
                    expected = tuple(round(a * opacity + b * (1 - opacity)) for a, b in zip(color, PAGE_BACKGROUND))
                    self.pixel(page, position, expected, "ordinary transformed gradient uses the same vector tile path")
                self.pixel(page, (100, 50), PAGE_BACKGROUND, "ordinary element clip transforms with its box")
                self.assert_vector_gradient()

    def test_positive_bleed_extends_page_background_and_pdf_production_boxes(self):
        pages = self.render(
            "@page{margin:20pt;padding:0;border:0;bleed:8pt;background:#102030;"
            "background-image:linear-gradient(red,red);background-size:4pt 4pt;"
            "background-position:right bottom;background-repeat:no-repeat;background-attachment:fixed}"
            "p{font:12pt/18pt serif}", "<p>Ink</p>", (200, 300), (216, 316),
        )
        self.assertEqual(len(pages), 1)
        for point in [(0, 0), (215, 0), (0, 315), (215, 315)]:
            self.pixel(pages[0], point, PAGE_BACKGROUND, "page background fills the complete bleed area")
        self.pixel(pages[0], (205, 305), (255, 0, 0), "fixed page image stays positioned against trim, not enlarged media")
        document = pdf_content.Document(self.output / "output.pdf")
        page = document.pages()[0]
        for name, expected in [("MediaBox", [-8, -8, 208, 308]),
                               ("TrimBox", [0, 0, 200, 300]),
                               ("BleedBox", [-8, -8, 208, 308])]:
            self.assertEqual(document.resolve(page[name]), expected, name)

    def test_bleed_preserves_images_and_links_entirely_outside_the_trim(self):
        image = ROOT / "tests/oracle/assets/border-image-nine.png"
        pages = self.render(
            "@page{margin:20pt;padding:0;border:0;bleed:8pt;background:#102030}"
            ".bleed{position:fixed;left:-28pt;top:10pt;width:7pt;height:21pt;display:block}"
            "a.bleed{top:50pt;background:magenta}.reference{left:0pt}",
            f"<img class=bleed src='{image.as_uri()}'><img class='bleed reference' src='{image.as_uri()}'>"
            "<a class=bleed href='https://example.org/'></a>",
            (200, 300), (216, 316),
        )
        outside = pages[0].crop((0, 38, 7, 59))
        reference = pages[0].crop((28, 38, 35, 59))
        self.assertTrue(any(low < high for low, high in reference.getextrema()),
                        "the untrimmed reference must contain the original image pattern")
        difference = ImageChops.difference(outside, reference)
        self.assertLessEqual(max(high for _, high in difference.getextrema()), PIXEL_TOLERANCE,
                             "the complete off-trim image must match its untrimmed counterpart")
        self.pixel(pages[0], (3, 88), (255, 0, 255), "off-trim element remains inside the positive bleed")
        document = pdf_content.Document(self.output / "output.pdf")
        page = document.pages()[0]
        annotations = [document.resolve(item) for item in document.resolve(page.get("Annots", []))]
        self.assertTrue(annotations, "the hyperlink in the visible bleed remains active")
        self.assertTrue(any(document.resolve(annotation["Rect"])[2] < 0 for annotation in annotations),
                        "the complete link lies outside trim and inside the media box")

    def test_bleed_preserves_constrained_page_and_original_sheet_geometry(self):
        pages = self.render("@page{width:200pt;height:300pt;bleed:8pt;"
                            "background-image:linear-gradient(red,red);background-size:4pt 4pt;"
                            "background-position:right bottom;background-repeat:no-repeat;background-attachment:fixed}",
                            "", (400, 600), (408, 608))
        self.pixel(pages[0], (279, 379), PAGE_BACKGROUND, "bleed follows the constrained page margin box")
        self.pixel(pages[0], (280, 380), (255, 255, 255), "unused sheet outside bleed stays blank")
        self.pixel(pages[0], (269, 369), (255, 0, 0), "fixed page image uses constrained trim geometry")
        document = pdf_content.Document(self.output / "output.pdf")
        page = document.pages()[0]
        self.assertEqual(document.resolve(page["MediaBox"]), [-8, 0, 400, 608])
        self.assertEqual(document.resolve(page["TrimBox"]), [0, 236, 264, 600])
        self.assertEqual(document.resolve(page["BleedBox"]), [-8, 228, 272, 608])

    def test_inverted_page_margin_extents_produce_valid_empty_output(self):
        for declarations, expected_trim in [
                ("width:100pt;margin-left:-100pt;margin-right:-100pt", [0, 0, 0, 600]),
                ("height:100pt;margin-top:-100pt;margin-bottom:-100pt", [0, 600, 400, 600]),
                ("width:100pt;height:100pt;margin:-100pt", [0, 600, 0, 600])]:
            with self.subTest(declarations=declarations):
                pages = self.render("@page{margin:0;padding:0;border:0;" + declarations +
                                    ";@top-left{content:'H';font:8pt/10pt serif}}",
                                    "<p>A</p><p style='break-before:page'>B</p>"
                                    "<div style='position:fixed;left:0;top:0;width:100pt;"
                                    "height:100pt;background:magenta'></div>")
                self.assertEqual(len(pages), 2)
                document = pdf_content.Document(self.output / "output.pdf")
                for raster, page in zip(pages, document.pages()):
                    self.assertEqual(raster.getextrema(), ((255, 255),) * 3,
                                     "page, canvas, content and margin ink obey the empty axis")
                    self.assertEqual(document.resolve(page["MediaBox"]), [0, 0, 400, 600])
                    self.assertEqual(document.resolve(page["TrimBox"]), expected_trim)
                    self.assertEqual(document.resolve(page["BleedBox"]), expected_trim)

        pages = self.render("@page{width:100pt;margin:0 -100pt;padding:0;border:0;bleed:6pt}",
                            "", output_size=(406, 612))
        self.pixel(pages[0], (1, 300), PAGE_BACKGROUND, "bleed can extend beyond an empty trim axis")
        self.pixel(pages[0], (11, 300), PAGE_BACKGROUND, "both sides of the zero-width trim have bleed")
        self.pixel(pages[0], (12, 300), (255, 255, 255), "ink still stops at the bleed edge")
        document = pdf_content.Document(self.output / "output.pdf")
        page = document.pages()[0]
        self.assertEqual(document.resolve(page["MediaBox"]), [-6, -6, 400, 606])
        self.assertEqual(document.resolve(page["TrimBox"]), [0, 0, 0, 600])
        self.assertEqual(document.resolve(page["BleedBox"]), [-6, -6, 6, 606])

    def test_page_percentages_keep_the_size_descriptor_basis_after_constraints(self):
        for declarations, area_width, trim, output_size in [
                ("width:50%;height:50%", 200, (360, 540), (400, 600)),
                ("max-width:200pt;max-height:300pt", 200, (360, 540), (400, 600)),
                ("width:200pt;height:300pt;box-sizing:border-box", 120, (280, 420), (400, 600)),
                ("width:500pt;height:700pt", 500, (660, 940), (660, 940))]:
            with self.subTest(declarations=declarations):
                pages = self.render("@page{margin:10%;padding:10%;border:0;" + declarations +
                                    ";background-image:linear-gradient(red,red);background-size:4pt 4pt;"
                                    "background-position:right bottom;background-repeat:no-repeat;"
                                    "background-attachment:fixed}"
                                    ".content{width:100vw;height:10pt;background:#708090}",
                                    "<div class=content></div>", output_size=output_size)
                self.assertEqual(len(pages), 1)
                self.pixel(pages[0], (79, 121), PAGE_BACKGROUND, "horizontal insets remain 80pt")
                self.pixel(pages[0], (81, 119), PAGE_BACKGROUND, "vertical insets remain 120pt")
                self.pixel(pages[0], (81, 121), CONTENT_BACKGROUND, "document starts inside the original percentage insets")
                self.pixel(pages[0], (79 + area_width, 121), CONTENT_BACKGROUND, "viewport width uses the resolved content area")
                self.pixel(pages[0], (80 + area_width, 121), PAGE_BACKGROUND, "content ends at the resolved viewport edge")
                self.pixel(pages[0], (trim[0] - 3, trim[1] - 3), (255, 0, 0),
                           "fixed backgrounds use the resulting page margin box")
                if trim != output_size:
                    self.pixel(pages[0], (trim[0] + 1, trim[1] + 1), (255, 255, 255),
                               "unused sheet is outside the resized page box")
                document = pdf_content.Document(self.output / "output.pdf")
                page = document.pages()[0]
                self.assertEqual(document.resolve(page["MediaBox"]), [0, 0, *output_size])
                self.assertEqual(document.resolve(page["TrimBox"]),
                                 [0, output_size[1] - trim[1], trim[0], output_size[1]])

    def test_zero_authored_size_keeps_empty_trim_on_a_valid_pdf_sheet(self):
        pages = self.render("@page{size:0;margin:0;padding:0;border:0}",
                            "<p>A<br>B</p>", output_size=(596, 842))
        self.assertEqual(len(pages), 2, "one indivisible line per one-pixel progress fragment")
        document = pdf_content.Document(self.output / "output.pdf")
        for raster, page in zip(pages, document.pages()):
            self.assertEqual(raster.getextrema(), ((255, 255),) * 3,
                             "zero-area bleed clips all page ink")
            media = document.resolve(page["MediaBox"])
            self.assertEqual(media[:2], [0, 0])
            self.assertAlmostEqual(media[2], 595.2756, places=3)
            self.assertAlmostEqual(media[3], 841.8898, places=3)
            for name in ("TrimBox", "BleedBox"):
                self.assertEqual(document.resolve(page[name]), [0, media[3], 0, media[3]])

        pages = self.render("@page{size:0;width:200pt;height:300pt;margin:10%;padding:10%;border:0}",
                            "<p>A</p>", output_size=(200, 300))
        self.assertEqual(len(pages), 1)
        self.pixel(pages[0], (1, 1), PAGE_BACKGROUND,
                   "zero preferred size remains the percentage base even on a larger sheet")
        document = pdf_content.Document(self.output / "output.pdf")
        self.assertEqual(document.resolve(document.pages()[0]["TrimBox"]), [0, 0, 200, 300])

    def test_zero_page_content_area_still_paints_its_margin_box(self):
        pages = self.render("@page{width:0;height:0;margin:20pt;padding:0;border:0;background:#102030;"
                            "background-image:linear-gradient(red,red);background-size:4pt 4pt;"
                            "background-position:right bottom;background-repeat:no-repeat;background-attachment:fixed}", "")
        self.assertEqual(len(pages), 1)
        self.pixel(pages[0], (5, 5), PAGE_BACKGROUND, "a zero content box still has page margins")
        self.pixel(pages[0], (37, 37), (255, 0, 0), "fixed page image uses the 40pt margin box")
        self.pixel(pages[0], (50, 50), (255, 255, 255), "unused sheet stays outside the zero-area page")
        document = pdf_content.Document(self.output / "output.pdf")
        self.assertEqual(document.resolve(document.pages()[0]["TrimBox"]), [0, 560, 40, 600])

    def test_negative_bleed_clips_background_content_and_hyperlinks(self):
        pages = self.render(
            "@page{margin:20pt;padding:0;border:0;bleed:-6pt;background:#102030;"
            "@top-left{content:'';width:50pt;background:yellow}}"
            "a{position:absolute;display:block;left:-20pt;top:-20pt;"
            "width:20pt;height:20pt;background:magenta}",
            "<a href='https://example.org/'>Ink</a>", (200, 300),
        )
        for point in [(0, 0), (5, 10), (10, 5), (195, 100), (100, 295)]:
            self.pixel(pages[0], point, (255, 255, 255), "nothing paints outside negative bleed")
        self.pixel(pages[0], (6, 18), (255, 0, 255), "content resumes at the inset bleed edge")
        self.pixel(pages[0], (100, 100), PAGE_BACKGROUND, "interior page background remains")
        self.pixel(pages[0], (30, 8), (255, 255, 0), "margin boxes obey the same bleed clip")
        document = pdf_content.Document(self.output / "output.pdf")
        page = document.pages()[0]
        self.assertEqual(document.resolve(page["MediaBox"]), [0, 0, 200, 300])
        self.assertEqual(document.resolve(page["BleedBox"]), [6, 6, 194, 294])
        annotations = [document.resolve(item) for item in document.resolve(page["Annots"])]
        self.assertTrue(annotations, "the visible part of the hyperlink remains active")
        for annotation in annotations:
            left, bottom, right, top = document.resolve(annotation["Rect"])
            self.assertGreaterEqual(left, 6)
            self.assertGreaterEqual(bottom, 6)
            self.assertLessEqual(right, 194)
            self.assertLessEqual(top, 294)

    def test_collapsed_bleed_and_hidden_root_keep_valid_output(self):
        pages = self.render("@page{margin:20pt;padding:0;border:0;bleed:-1000pt;background:red}",
                            "<a href='https://example.org/'>Clipped</a>", (200, 300))
        self.assertEqual(pages[0].getextrema(), ((255, 255),) * 3,
                         "negative bleed may collapse the ink area without invalidating the page")
        document = pdf_content.Document(self.output / "output.pdf")
        page = document.pages()[0]
        self.assertEqual(document.resolve(page["BleedBox"]), [100, 100, 100, 200])
        self.assertFalse(page.get("Annots"), "fully clipped links must not activate")
        pages = self.render("@page{margin:20pt;padding:0;border:0;bleed:8pt;background:#102030}"
                            "html{display:none}", "<p>Hidden</p>", (200, 300), (216, 316))
        self.assertEqual(len(pages), 1)
        self.pixel(pages[0], (0, 0), PAGE_BACKGROUND, "page production survives a hidden document root")

    def test_marked_multipage_destinations_keep_original_page_coordinates(self):
        pages = self.render("@page{margin:20pt;padding:0;border:0;marks:crop}"
                            "p{font:12pt/18pt serif}",
                            "<p><a href='#target'>Jump</a></p>"
                            "<p id=target style='break-before:page'>Target</p>",
                            (200, 300), (248, 348))
        self.assertEqual(len(pages), 2, "printer margins must not add or remove content pages")
        document = pdf_content.Document(self.output / "output.pdf")
        for page in document.pages():
            self.assertEqual(document.resolve(page["MediaBox"]), [-24, -24, 224, 324])
            self.assertEqual(document.resolve(page["TrimBox"]), [0, 0, 200, 300])
            self.assertEqual(document.resolve(page["BleedBox"]), [-6, -6, 206, 306])
        navigation = pdf_content.navigation(self.output / "output.pdf")
        self.assertTrue(navigation["links"], "the first page retains its internal link")
        for link in navigation["links"]:
            self.assertEqual(link["destination"], {"page": 2, "top": 280},
                             "destination conversion uses the original 300pt page surface")
            self.assertEqual(link["rect"][0], 20,
                             "PDF annotations stay in the same coordinates as document ink")

    def test_crop_and_cross_marks_stay_outside_automatic_bleed(self):
        pages = self.render("@page{margin:20pt;padding:0;border:0;"
                            "marks:cross crop;background:#102030}", "", (200, 300), (248, 348))
        document = pdf_content.Document(self.output / "output.pdf")
        page = document.pages()[0]
        self.assertEqual(document.resolve(page["TrimBox"]), [0, 0, 200, 300])
        self.assertEqual(document.resolve(page["BleedBox"]), [-6, -6, 206, 306])
        self.assertEqual(document.resolve(page["MediaBox"]), [-24, -24, 224, 324])
        self.pixel(pages[0], (18, 18), PAGE_BACKGROUND, "crop marks select six-point automatic bleed")
        self.pixel(pages[0], (17, 17), (255, 255, 255), "paper outside bleed remains blank")
        crop_regions = [(23, 3, 25, 15), (223, 3, 225, 15),
                        (23, 333, 25, 345), (223, 333, 225, 345),
                        (3, 23, 15, 25), (3, 323, 15, 325),
                        (233, 23, 245, 25), (233, 323, 245, 325)]
        cross_regions = [(119, 6, 129, 16), (119, 332, 129, 342),
                         (6, 169, 16, 179), (232, 169, 242, 179)]
        for region in crop_regions + cross_regions:
            mark = pages[0].crop(region)
            pixels = [mark.getpixel((x, y)) for x in range(mark.width) for y in range(mark.height)]
            self.assertTrue(any(max(pixel) < 224 for pixel in pixels),
                            f"native vector printer mark is missing from {region}")
        self.assertNotIn("Image", [obj.get("Subtype") for obj in self.pdf_dictionaries()],
                         "printer marks must remain native vector shapes")

    def test_sliced_decorations_match_windows_of_the_unbroken_box(self):
        cases = [
            ("solid", ""),
            ("dashed", "border-style:dashed"),
            ("square-dashed", "border-radius:0;border-style:dashed"),
            ("dotted", "border-style:dotted"),
            ("double", "border-style:double"),
            ("gradient", "background:linear-gradient(red,blue) padding-box no-repeat"),
            ("asymmetric", "border-radius:60% 20% / 40% 30%"),
            ("outer-sharp", "box-shadow:8pt 6pt 0 3pt blue"),
            ("inset-sharp", "box-shadow:inset 4pt 6pt 0 2pt green"),
            ("outer-blur", "box-shadow:8pt 6pt 8pt 3pt blue"),
            ("inset-blur", "box-shadow:inset 4pt 6pt 8pt 2pt green"),
            ("overflow", "border:0;background:none;overflow:hidden"),
        ]
        for fragmentation in ("pages", "columns", "column-rows"):
            columns = fragmentation != "pages"
            rows = fragmentation == "column-rows"
            for name, decoration in cases:
                with self.subTest(decoration=name, fragmentation=fragmentation):
                    css = ("@page{margin:20pt;padding:0;border:0;background:#f0f0f0}"
                           ".lead{height:120pt}.box{width:100pt;height:230pt;"
                           "box-sizing:border-box;border:4pt solid red;border-radius:60pt;"
                           f"background:yellow;{decoration}}}"
                           ".fill{height:230pt;background:magenta}")
                    body = "<div class=lead></div><div class=box>" + ("<div class=fill></div>" if name == "overflow" else "") + "</div>"
                    size = (380, 200) if rows else ((560, 200) if columns else (200, 200))
                    count, height = (2, "auto") if rows else (3, "160pt")
                    css += f".columns{{column-count:{count};column-gap:20pt;column-fill:auto;height:{height}}}"
                    source_body = f"<div class=columns>{body}</div>" if columns else body
                    fragmented = self.render(css, source_body, size)
                    self.assertEqual(len(fragmented), 2 if rows else (1 if columns else 3),
                                     "40pt + 160pt + 30pt must preserve the 230pt border box")
                    if "blur" in name:
                        images = [obj for obj in self.pdf_dictionaries() if obj.get("Subtype") == "Image"]
                        self.assertEqual(len(images), 1, "all fragments must share the complete shadow mask resource")
                    for index, top in enumerate((120, -40, -200)):
                        page = index // count if rows else (0 if columns else index)
                        left = (index % count) * 180 if columns else 0
                        window = (left, 20, left + 180, 180)
                        # Same paper and device origin keep Poppler's image/edge sampling identical.
                        whole = self.render(css + f".box{{position:absolute;left:{left}pt;top:{top}pt}}",
                                            body.replace("<div class=lead></div>", ""), size)
                        self.assertEqual(len(whole), 1, "the absolute reference must remain unbroken")
                        difference = ImageChops.difference(fragmented[page].crop(window), whole[0].crop(window))
                        maximum = max(high for _, high in difference.getextrema())
                        self.assertLessEqual(maximum, PIXEL_TOLERANCE,
                                             f"{name} page {page}: sliced decoration differs from the corresponding unbroken window by {maximum} RGB levels; bounds {difference.getbbox()}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renderer", type=Path, default=RENDERER)
    arguments = parser.parse_args()
    RENDERER = arguments.renderer.resolve()
    unittest.main(argv=[__file__], verbosity=2)
