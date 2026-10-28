//! CSS Logical 1 §4: flow-relative and physical declarations cascade together as one.
use crate::dom::{DocumentTree, Selector};
use crate::paint::BorderStyle;
use crate::resources::FileResolver;
use crate::style::{
    LengthPercentage, Overflow, Size, Style, Stylesheet, StylesheetSource,
    logical::{BOTTOM, LEFT, RIGHT, TOP},
};

const TOP_LEFT: usize = 0;
const TOP_RIGHT: usize = 1;
const BOTTOM_RIGHT: usize = 2;
const BOTTOM_LEFT: usize = 3;
const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
const DOCUMENT: &str = "<section id=parent><div id=child>X</div></section>";
const BLOCKQUOTE: &str = "<section id=parent><blockquote id=child>X</blockquote></section>";
/// The user-agent 40px inline indent of `blockquote`, `dd` and lists, in points.
const INDENT: f32 = 30.0;

fn styles(css: &str, html: &str) -> (Style, Style) {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap_or_else(|error| panic!("stylesheet {css:?} failed to parse: {error}"));
    let html = DocumentTree::parse(html).expect("parse test HTML");
    let select = |id: &str| html.select(&Selector::parse(id).unwrap()).next().unwrap();
    let parent = sheet
        .computed_with_resolver(
            select("#parent"),
            &Style::default(),
            &FileResolver::default(),
        )
        .unwrap_or_else(|error| panic!("{css:?}: parent style failed: {error}"));
    let child = sheet
        .computed_with_resolver(select("#child"), &parent, &FileResolver::default())
        .unwrap_or_else(|error| panic!("{css:?}: child style failed: {error}"));
    (parent, child)
}

fn child(css: &str) -> Style {
    styles(css, DOCUMENT).1
}

fn points(value: f32) -> Size {
    Size::Value(LengthPercentage::Length(value))
}

fn margins(style: &Style) -> [Size; 4] {
    style.margin.clone()
}

#[test]
fn declaration_order_decides_between_paired_properties_in_each_direction() {
    // The CSS Logical 1 §4 example: inline-start pairs with left in LTR, inline-end in RTL.
    let declarations = "margin-inline-start:1pt;margin-left:2pt;margin-inline-end:3pt";
    let ltr = child(&format!("#child{{{declarations}}}"));
    assert_eq!(
        [ltr.margin[LEFT].clone(), ltr.margin[RIGHT].clone()],
        [points(2.0), points(3.0)],
        "LTR: margin-left follows margin-inline-start; got {:?}",
        margins(&ltr)
    );
    let rtl = child(&format!("#child{{direction:rtl;{declarations}}}"));
    assert_eq!(
        [rtl.margin[LEFT].clone(), rtl.margin[RIGHT].clone()],
        [points(3.0), points(1.0)],
        "RTL: margin-inline-end follows margin-left; got {:?}",
        margins(&rtl)
    );
    let reversed = child("#child{margin-left:2pt;margin-inline-start:1pt}");
    assert_eq!(
        reversed.margin[LEFT],
        points(1.0),
        "a later flow-relative declaration overrides the physical one; got {:?}",
        margins(&reversed)
    );
    let split = child("#child{margin-inline-start:1pt}#child{margin-left:2pt}");
    assert_eq!(
        split.margin[LEFT],
        points(2.0),
        "order of appearance spans rules; got {:?}",
        margins(&split)
    );
}

#[test]
fn importance_and_specificity_outrank_order_across_paired_properties() {
    for (css, side, expected, reason) in [
        (
            "#child{margin-left:2pt!important;margin-inline-start:1pt}",
            LEFT,
            2.0,
            "an important physical declaration beats a later flow-relative one",
        ),
        (
            "#child{margin-inline-start:1pt!important}#child{margin-left:2pt}",
            LEFT,
            1.0,
            "an important flow-relative declaration beats a later physical one",
        ),
        (
            "#child{margin-inline-start:1pt}div{margin-left:2pt}",
            LEFT,
            1.0,
            "the more specific flow-relative selector wins although it comes first",
        ),
        (
            "#child{direction:rtl;margin-inline-start:1pt!important;margin-right:5pt}",
            RIGHT,
            1.0,
            "an important inline-start declaration holds the right margin in RTL",
        ),
        (
            "#child{direction:rtl;margin-inline-start:1pt!important;margin-left:5pt}",
            LEFT,
            5.0,
            "in RTL inline-start does not compete with margin-left",
        ),
    ] {
        let style = child(css);
        assert_eq!(
            style.margin[side],
            points(expected),
            "{reason}: {css} computed margins {:?}",
            margins(&style)
        );
    }
}

#[test]
fn revert_rolls_back_the_paired_cascade_as_one() {
    let (_, style) = styles(
        "#child{margin-left:5pt;margin-inline-start:revert}",
        BLOCKQUOTE,
    );
    assert_eq!(
        style.margin[LEFT],
        points(INDENT),
        "revert on inline-start must drop the author margin-left too and reach the user-agent \
         blockquote margin; got {:?}",
        margins(&style)
    );
}

#[test]
fn flow_relative_inherit_reads_the_parents_corresponding_property() {
    let parent = "#parent{margin-left:4pt;margin-right:6pt;padding-left:1pt;\
                  border-left:2pt solid;border-top-left-radius:3pt}";
    let inherited = "margin-inline-start:inherit;padding-inline-start:inherit;\
                     border-inline-start-width:inherit;border-inline-start-style:inherit;\
                     border-start-start-radius:inherit";
    let (_, rtl) = styles(
        &format!("{parent}#child{{direction:rtl;{inherited}}}"),
        DOCUMENT,
    );
    assert_eq!(
        margins(&rtl),
        [points(0.0), points(4.0), points(0.0), points(0.0)],
        "an RTL child's inline-start (right) margin inherits the LTR parent's inline-start \
         (left) margin"
    );
    assert_eq!(rtl.padding[RIGHT], LengthPercentage::Length(1.0));
    assert_eq!(rtl.padding[LEFT], LengthPercentage::ZERO);
    let right = rtl.decoration.borders[RIGHT];
    assert_eq!(
        (right.width, right.style),
        (2.0, BorderStyle::Solid),
        "border inline-start inherits from the parent's left border, got {:?}",
        rtl.decoration.borders
    );
    assert_eq!(
        rtl.decoration.radii[TOP_RIGHT],
        [LengthPercentage::Length(3.0), LengthPercentage::Length(3.0)],
        "the RTL start-start corner is top-right and inherits the parent's top-left; got {:?}",
        rtl.decoration.radii
    );
    assert_eq!(rtl.decoration.radii[TOP_LEFT], [LengthPercentage::ZERO; 2]);
    let (_, physical) = styles(
        &format!("{parent}#child{{direction:rtl;margin-left:inherit}}"),
        DOCUMENT,
    );
    assert_eq!(
        physical.margin[LEFT],
        points(4.0),
        "physical inheritance stays physical; got {:?}",
        margins(&physical)
    );
    let (_, ltr) = styles(
        &format!("{parent}#child{{margin-inline-end:inherit}}"),
        DOCUMENT,
    );
    assert_eq!(
        ltr.margin[RIGHT],
        points(6.0),
        "same-direction inheritance; got {:?}",
        margins(&ltr)
    );
}

#[test]
fn flow_relative_toggle_compares_with_the_parents_corresponding_value() {
    let (_, style) = styles(
        "#parent{margin-left:4pt;margin-right:6pt}\
         #child{direction:rtl;margin-inline-start:toggle(4pt,8pt)}",
        DOCUMENT,
    );
    assert_eq!(
        style.margin[RIGHT],
        points(8.0),
        "the parent's inline-start margin is 4pt, so the toggle advances to 8pt; got {:?}",
        margins(&style)
    );
}

#[test]
fn all_inherit_sets_physical_longhands_last() {
    let (parent, style) = styles(
        "#parent{margin:1pt 6pt 2pt 4pt;border-radius:1pt 2pt 3pt 4pt}\
         #child{direction:rtl;all:inherit}",
        DOCUMENT,
    );
    assert_eq!(
        margins(&style),
        margins(&parent),
        "all:inherit must leave each margin inheriting its physical counterpart"
    );
    // Name order alone would put border-end-*-radius after border-bottom-*-radius.
    assert_eq!(
        style.decoration.radii, parent.decoration.radii,
        "all:inherit must leave each corner radius inheriting its physical counterpart"
    );
}

#[test]
fn flow_relative_shorthands_map_through_the_element_direction() {
    let css = "margin-inline:1pt 2pt;margin-block:3pt 4pt;padding-inline:5pt 6pt;\
               padding-block:7pt;inset-inline:8pt 9pt;inset-block:10pt auto;\
               border-inline-start:2pt solid rgb(255,0,0);border-inline-end-width:3pt;\
               border-inline-end-style:dashed;border-block:1pt dotted rgb(0,0,255);\
               border-start-end-radius:4pt;border-end-start-radius:5pt 6pt;\
               inline-size:50pt;block-size:20pt;min-inline-size:11pt;min-block-size:12pt;\
               max-inline-size:60pt;max-block-size:30pt;overflow-inline:clip;\
               overflow-block:visible";
    for (direction, start, end, start_end_corner, end_start_corner) in [
        ("ltr", LEFT, RIGHT, TOP_RIGHT, BOTTOM_LEFT),
        ("rtl", RIGHT, LEFT, TOP_LEFT, BOTTOM_RIGHT),
    ] {
        let style = child(&format!("#child{{direction:{direction};{css}}}"));
        let context = format!("{direction}: {style:?}");
        assert_eq!(style.margin[start], points(1.0), "{context}");
        assert_eq!(style.margin[end], points(2.0), "{context}");
        assert_eq!(
            [style.margin[TOP].clone(), style.margin[BOTTOM].clone()],
            [points(3.0), points(4.0)],
            "{context}"
        );
        assert_eq!(
            style.padding[start],
            LengthPercentage::Length(5.0),
            "{context}"
        );
        assert_eq!(
            style.padding[end],
            LengthPercentage::Length(6.0),
            "{context}"
        );
        assert_eq!(
            style.padding[TOP],
            LengthPercentage::Length(7.0),
            "{context}"
        );
        assert_eq!(
            style.padding[BOTTOM],
            LengthPercentage::Length(7.0),
            "{context}"
        );
        assert_eq!(style.inset[start], points(8.0), "{context}");
        assert_eq!(style.inset[end], points(9.0), "{context}");
        assert_eq!(
            [style.inset[TOP].clone(), style.inset[BOTTOM].clone()],
            [points(10.0), Size::Auto],
            "{context}"
        );
        let borders = style.decoration.borders;
        assert_eq!(
            (borders[start].width, borders[start].style),
            (2.0, BorderStyle::Solid),
            "{context}"
        );
        assert_eq!(borders[start].color.rgb, RED, "{context}");
        assert_eq!(
            (borders[end].width, borders[end].style),
            (3.0, BorderStyle::Dashed),
            "{context}"
        );
        for side in [TOP, BOTTOM] {
            assert_eq!(
                (
                    borders[side].width,
                    borders[side].style,
                    borders[side].color.rgb
                ),
                (1.0, BorderStyle::Dotted, BLUE),
                "{context}"
            );
        }
        let radii = &style.decoration.radii;
        assert_eq!(
            radii[start_end_corner],
            [LengthPercentage::Length(4.0), LengthPercentage::Length(4.0)],
            "{context}"
        );
        assert_eq!(
            radii[end_start_corner],
            [LengthPercentage::Length(5.0), LengthPercentage::Length(6.0)],
            "{context}"
        );
        for corner in [TOP_LEFT, TOP_RIGHT, BOTTOM_RIGHT, BOTTOM_LEFT]
            .into_iter()
            .filter(|corner| ![start_end_corner, end_start_corner].contains(corner))
        {
            assert_eq!(radii[corner], [LengthPercentage::ZERO; 2], "{context}");
        }
        assert_eq!(
            [&style.width, &style.height],
            [&points(50.0), &points(20.0)],
            "{context}"
        );
        assert_eq!(
            [&style.min_width, &style.min_height],
            [&points(11.0), &points(12.0)],
            "{context}"
        );
        assert_eq!(
            [&style.max_width, &style.max_height],
            [&points(60.0), &points(30.0)],
            "{context}"
        );
        assert_eq!(
            style.overflow,
            [Overflow::Clip, Overflow::Visible],
            "{context}"
        );
    }
}

#[test]
fn substituted_flow_relative_shorthands_find_their_own_longhands() {
    let style = child("#child{--inline:1pt 2pt;direction:rtl;margin-inline:var(--inline)}");
    assert_eq!(
        [style.margin[RIGHT].clone(), style.margin[LEFT].clone()],
        [points(1.0), points(2.0)],
        "var() in margin-inline expands to inline-start then inline-end; got {:?}",
        margins(&style)
    );
}

#[test]
fn contextual_values_pair_flow_relative_and_physical_declarations() {
    let style = child(
        "#child{--k:a;direction:rtl;margin-left:2pt;\
         margin-inline-end:if(style(--k:a):5pt;else:1pt);\
         margin-inline-start:if(style(--k:b):7pt;else:3pt)}",
    );
    assert_eq!(
        [style.margin[LEFT].clone(), style.margin[RIGHT].clone()],
        [points(5.0), points(3.0)],
        "the later inline-end declaration wins margin-left in RTL; got {:?}",
        margins(&style)
    );
}

#[test]
fn supports_queries_accept_flow_relative_properties() {
    let style = child(
        "@supports (margin-inline-start:1pt) and (border-block:1pt solid) \
         and (inline-size:10pt) {#child{margin-top:9pt}}",
    );
    assert_eq!(
        style.margin[TOP],
        points(9.0),
        "@supports must report flow-relative properties as supported; got {:?}",
        margins(&style)
    );
}

#[test]
fn user_agent_indents_sit_on_the_inline_start_side() {
    for (direction, start, end) in [("ltr", LEFT, RIGHT), ("rtl", RIGHT, LEFT)] {
        let (_, list) = styles(
            "",
            &format!("<section id=parent dir={direction}><ul id=child><li>X</ul></section>"),
        );
        assert_eq!(
            [list.padding[start].clone(), list.padding[end].clone()],
            [LengthPercentage::Length(INDENT), LengthPercentage::ZERO],
            "{direction}: a list indents its inline-start side; got {:?}",
            list.padding
        );
        let (_, definition) = styles(
            "",
            &format!("<dl id=parent dir={direction}><dd id=child>X</dd></dl>"),
        );
        assert_eq!(
            [
                definition.margin[start].clone(),
                definition.margin[end].clone()
            ],
            [points(INDENT), points(0.0)],
            "{direction}: dd indents its inline-start side; got {:?}",
            margins(&definition)
        );
    }
}
