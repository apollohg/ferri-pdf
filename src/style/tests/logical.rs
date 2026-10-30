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

#[test]
fn legacy_writing_modes_compute_to_modern_values_without_changing_direction() {
    use crate::style::logical::WritingMode;
    for (aliases, expected) in [
        (
            &["lr", "lr-tb", "rl", "rl-tb"][..],
            WritingMode::HorizontalTb,
        ),
        (&["tb", "tb-rl"][..], WritingMode::VerticalRl),
    ] {
        for alias in aliases {
            for direction in ["ltr", "rtl"] {
                let actual = child(&format!(
                    "#child{{writing-mode:vertical-lr;direction:{direction};writing-mode:{alias}}}"
                ));
                assert_eq!(actual.writing_mode, expected, "{alias}/{direction}");
                assert_eq!(
                    actual.rtl,
                    direction == "rtl",
                    "{alias} preserves direction"
                );
            }
        }
    }
}

#[test]
fn glyph_orientation_is_a_single_longhand_shorthand_in_the_normal_cascade() {
    use crate::style::logical::TextOrientation::{Mixed, Sideways, Upright};
    for (value, expected) in [
        ("auto", Mixed),
        ("0", Upright),
        ("0deg", Upright),
        ("90", Sideways),
        ("90deg", Sideways),
    ] {
        let css = format!(
            "#parent{{--legacy:{value};text-orientation:sideways}}\
                           #child{{text-orientation:mixed;glyph-orientation-vertical:var(--legacy)}}"
        );
        assert_eq!(
            child(&css).text_orientation,
            expected,
            "{value}: substituted shorthand"
        );
    }
    for (declarations, expected) in [
        (
            "glyph-orientation-vertical:90;text-orientation:upright",
            Upright,
        ),
        (
            "text-orientation:upright;glyph-orientation-vertical:90",
            Sideways,
        ),
        (
            "glyph-orientation-vertical:90!important;text-orientation:upright",
            Sideways,
        ),
        ("glyph-orientation-vertical:inherit", Upright),
        ("glyph-orientation-vertical:unset", Upright),
        ("glyph-orientation-vertical:initial", Mixed),
    ] {
        assert_eq!(
            child(&format!(
                "#parent{{text-orientation:upright}}#child{{{declarations}}}"
            ))
            .text_orientation,
            expected,
            "{declarations}: cascade uses the text-orientation longhand"
        );
    }
}

#[test]
fn obsolete_glyph_orientation_rejects_other_angles_units_and_horizontal_property() {
    use crate::style::logical::TextOrientation::Upright;
    for declaration in [
        "glyph-orientation-vertical:180",
        "glyph-orientation-vertical:180deg",
        "glyph-orientation-vertical:270",
        "glyph-orientation-vertical:270deg",
        "glyph-orientation-vertical:1",
        "glyph-orientation-vertical:45deg",
        "glyph-orientation-vertical:0rad",
        "glyph-orientation-vertical:90grad",
        "glyph-orientation-vertical:.25turn",
        "glyph-orientation-vertical:sideways",
        "glyph-orientation-horizontal:90deg",
        "writing-mode:tb-lr",
    ] {
        let actual = child(&format!(
            "#child{{writing-mode:vertical-lr;text-orientation:upright;{declaration}}}"
        ));
        assert_eq!(actual.text_orientation, Upright, "invalid {declaration}");
        assert_eq!(
            actual.writing_mode,
            crate::style::logical::WritingMode::VerticalLr,
            "invalid {declaration} preserves the preceding writing-mode"
        );
    }
}

fn points(value: f32) -> Size {
    Size::Value(LengthPercentage::Length(value))
}

fn margins(style: &Style) -> [Size; 4] {
    style.margin.clone()
}

#[test]
fn side_radius_shorthands_order_their_two_ellipses_in_each_direction() {
    for (name, ltr, rtl) in [
        (
            "border-top-radius",
            [TOP_LEFT, TOP_RIGHT],
            [TOP_LEFT, TOP_RIGHT],
        ),
        (
            "border-right-radius",
            [TOP_RIGHT, BOTTOM_RIGHT],
            [TOP_RIGHT, BOTTOM_RIGHT],
        ),
        (
            "border-bottom-radius",
            [BOTTOM_LEFT, BOTTOM_RIGHT],
            [BOTTOM_LEFT, BOTTOM_RIGHT],
        ),
        (
            "border-left-radius",
            [TOP_LEFT, BOTTOM_LEFT],
            [TOP_LEFT, BOTTOM_LEFT],
        ),
        (
            "border-block-start-radius",
            [TOP_LEFT, TOP_RIGHT],
            [TOP_RIGHT, TOP_LEFT],
        ),
        (
            "border-block-end-radius",
            [BOTTOM_LEFT, BOTTOM_RIGHT],
            [BOTTOM_RIGHT, BOTTOM_LEFT],
        ),
        (
            "border-inline-start-radius",
            [TOP_LEFT, BOTTOM_LEFT],
            [TOP_RIGHT, BOTTOM_RIGHT],
        ),
        (
            "border-inline-end-radius",
            [TOP_RIGHT, BOTTOM_RIGHT],
            [TOP_LEFT, BOTTOM_LEFT],
        ),
    ] {
        for (direction, corners) in [("ltr", ltr), ("rtl", rtl)] {
            for (value, first, second) in [
                ("10pt", [10.0, 10.0], [10.0, 10.0]),
                ("10pt 20pt", [10.0, 10.0], [20.0, 20.0]),
                ("10pt 20pt / 30pt 40pt", [10.0, 30.0], [20.0, 40.0]),
            ] {
                let css = format!("#child{{direction:{direction};{name}:{value}}}");
                let style = child(&css);
                let mut expected = [const { [LengthPercentage::ZERO; 2] }; 4];
                expected[corners[0]] = first.map(LengthPercentage::Length);
                expected[corners[1]] = second.map(LengthPercentage::Length);
                assert_eq!(
                    *style.decoration.radii, expected,
                    "{css}: each side sets only its two corners, in specification order"
                );
            }
        }
    }
}

#[test]
fn side_radius_shorthands_share_corner_cascade_and_inherit_corresponding_corners() {
    let css = "#parent{border-radius:1pt 2pt 3pt 4pt / 5pt 6pt 7pt 8pt}\
               #child{direction:rtl;border-block-start-radius:inherit;\
               border-top-left-radius:9pt 10pt;border-block-end-radius:11pt;\
               border-bottom-radius:-1pt 12pt}";
    let style = child(css);
    assert_eq!(
        *style.decoration.radii,
        [
            [
                LengthPercentage::Length(9.0),
                LengthPercentage::Length(10.0)
            ],
            [LengthPercentage::Length(1.0), LengthPercentage::Length(5.0)],
            [const { LengthPercentage::Length(11.0) }; 2],
            [const { LengthPercentage::Length(11.0) }; 2],
        ],
        "{css}: logical inheritance maps through the parent flow, later physical corners win, and an invalid pair changes neither corner"
    );
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

#[test]
fn scroll_and_overscroll_logical_properties_cascade_with_their_physical_pairs() {
    use crate::style::grammar::Property as P;
    let length = |style: &Style, property| match property {
        P::ScrollMarginLeft => Some(style.scroll_margin[LEFT]),
        P::ScrollMarginRight => Some(style.scroll_margin[RIGHT]),
        P::ScrollPaddingTop
        | P::ScrollPaddingBottom
        | P::ScrollPaddingLeft
        | P::ScrollPaddingRight => {
            let side = match property {
                P::ScrollPaddingTop => TOP,
                P::ScrollPaddingBottom => BOTTOM,
                P::ScrollPaddingLeft => LEFT,
                _ => RIGHT,
            };
            match &style.scroll_padding[side] {
                Some(LengthPercentage::Length(value)) => Some(*value),
                None => None,
                other => panic!("{property:?} must compute to a length, got {other:?}"),
            }
        }
        other => panic!("{other:?} is not a scroll margin or padding side"),
    };
    let declarations = "scroll-margin-inline-start:1pt;scroll-margin-left:2pt;\
        scroll-margin-inline-end:3pt;scroll-padding-block:4pt 5pt";
    let ltr = child(&format!("#child{{{declarations}}}"));
    assert_eq!(
        [
            length(&ltr, P::ScrollMarginLeft),
            length(&ltr, P::ScrollMarginRight)
        ],
        [Some(2.0), Some(3.0)],
        "LTR: scroll-margin-left follows scroll-margin-inline-start"
    );
    assert_eq!(
        [
            length(&ltr, P::ScrollPaddingTop),
            length(&ltr, P::ScrollPaddingBottom)
        ],
        [Some(4.0), Some(5.0)],
        "the block shorthand sets the block-start then the block-end side"
    );
    let rtl = child(&format!("#child{{direction:rtl;{declarations}}}"));
    assert_eq!(
        [
            length(&rtl, P::ScrollMarginLeft),
            length(&rtl, P::ScrollMarginRight)
        ],
        [Some(3.0), Some(1.0)],
        "RTL: inline-end maps to left, where the later inline-end declaration wins"
    );
    let (parent, inherited) = styles(
        "#parent{scroll-padding-left:9pt;scroll-padding-right:6pt}\
         #child{direction:rtl;scroll-padding-inline-end:inherit}",
        DOCUMENT,
    );
    assert_eq!(length(&parent, P::ScrollPaddingRight), Some(6.0));
    assert_eq!(
        length(&inherited, P::ScrollPaddingLeft),
        Some(6.0),
        "inherit takes the LTR parent's inline-end, its right side, into the RTL child's left"
    );
    let overscroll = child("#child{overscroll-behavior-x:contain;overscroll-behavior-block:none}");
    let keyword = |property| {
        overscroll
            .inert
            .get(property)
            .map(|value| format!("{value:?}"))
    };
    assert_eq!(
        [
            keyword(P::OverscrollBehaviorX),
            keyword(P::OverscrollBehaviorY)
        ],
        [
            Some("Specified(Keyword(Contain))".to_owned()),
            Some("Specified(Keyword(None))".to_owned())
        ],
        "overscroll-behavior-block pairs with the vertical axis"
    );
}

#[test]
fn vertical_modes_map_logical_sides_dimensions_and_inherited_pairs() {
    for (mode, block_start, block_end, inline_start, inline_end) in [
        ("vertical-rl", RIGHT, LEFT, TOP, BOTTOM),
        ("vertical-lr", LEFT, RIGHT, TOP, BOTTOM),
        ("sideways-rl", RIGHT, LEFT, TOP, BOTTOM),
        ("sideways-lr", LEFT, RIGHT, BOTTOM, TOP),
    ] {
        for rtl in [false, true] {
            let direction = if rtl { "rtl" } else { "ltr" };
            let css = format!(
                "#parent{{writing-mode:{mode};direction:{direction};margin-block-start:11pt;\
                margin-block-end:13pt;margin-inline-start:17pt;margin-inline-end:19pt;\
                inline-size:101pt;block-size:103pt}}\
                #child{{margin-block-start:inherit;margin-inline-start:inherit;\
                block-size:inherit;inline-size:inherit}}"
            );
            let (parent, child) = styles(&css, DOCUMENT);
            let (inline_start, inline_end) = if rtl {
                (inline_end, inline_start)
            } else {
                (inline_start, inline_end)
            };
            for (side, expected) in [
                (block_start, 11.0),
                (block_end, 13.0),
                (inline_start, 17.0),
                (inline_end, 19.0),
            ] {
                assert_eq!(
                    parent.margin[side],
                    points(expected),
                    "{mode}/{direction} physical side {side}: {:?}",
                    parent.margin
                );
            }
            assert_eq!(
                child.margin[block_start],
                points(11.0),
                "{mode}/{direction}: inherited block-start"
            );
            assert_eq!(
                child.margin[inline_start],
                points(17.0),
                "{mode}/{direction}: inherited inline-start"
            );
            assert_eq!(
                [child.width, child.height],
                [points(103.0), points(101.0)],
                "{mode}/{direction}: physical dimensions"
            );
        }
    }
}

#[test]
fn corner_shapes_compute_keywords_math_and_four_value_order() {
    for (value, expected) in [
        ("round", [1.0; 4]),
        ("scoop", [-1.0; 4]),
        ("bevel", [0.0; 4]),
        ("squircle", [2.0; 4]),
        ("notch", [f32::NEG_INFINITY; 4]),
        ("square", [f32::INFINITY; 4]),
        ("superellipse(-0.5)", [-0.5; 4]),
        ("superellipse(calc(3 / 2))", [1.5; 4]),
        ("superellipse(infinity)", [f32::INFINITY; 4]),
        ("superellipse(-infinity)", [f32::NEG_INFINITY; 4]),
        ("round bevel scoop squircle", [1.0, 0.0, -1.0, 2.0]),
        ("round bevel scoop", [1.0, 0.0, -1.0, 0.0]),
        ("round bevel", [1.0, 0.0, 1.0, 0.0]),
    ] {
        let style = child(&format!("#child{{corner-shape:{value}}}"));
        assert_eq!(
            style
                .decoration
                .corner_shapes
                .map(crate::paint::CornerShape::curvature),
            expected,
            "{value}"
        );
        assert!(
            crate::style::declaration::supports_declaration("corner-shape", value),
            "supports {value}"
        );
    }
    for value in [
        "superellipse(1px)",
        "superellipse()",
        "superellipse(1,2)",
        "bevel square notch round scoop",
        "infinity",
    ] {
        let style = child(&format!(
            "#child{{corner-shape:bevel;corner-shape:{value}}}"
        ));
        assert_eq!(
            style
                .decoration
                .corner_shapes
                .map(crate::paint::CornerShape::curvature),
            [0.0; 4],
            "invalid {value}"
        );
        assert!(
            !crate::style::declaration::supports_declaration("corner-shape", value),
            "unsupported {value}"
        );
    }
}

#[test]
fn combined_corner_shorthands_set_and_reset_both_families() {
    let cases = [
        ("corner-top-left", vec![0]),
        ("corner-top-right", vec![1]),
        ("corner-bottom-right", vec![2]),
        ("corner-bottom-left", vec![3]),
        ("corner-start-start", vec![0]),
        ("corner-start-end", vec![1]),
        ("corner-end-start", vec![3]),
        ("corner-end-end", vec![2]),
        ("corner-top", vec![0, 1]),
        ("corner-right", vec![1, 2]),
        ("corner-bottom", vec![3, 2]),
        ("corner-left", vec![0, 3]),
        ("corner-block-start", vec![0, 1]),
        ("corner-block-end", vec![3, 2]),
        ("corner-inline-start", vec![0, 3]),
        ("corner-inline-end", vec![1, 2]),
        ("corner", vec![0, 1, 2, 3]),
    ];
    for (name, corners) in cases {
        for (value, shape, radius) in [
            ("bevel 10pt", 0.0, 10.0),
            ("10pt bevel", 0.0, 10.0),
            ("bevel", 0.0, 0.0),
            ("10pt", 1.0, 10.0),
        ] {
            let css = format!("#child{{corner-shape:scoop;border-radius:7pt;{name}:{value}}}");
            let style = child(&css);
            for corner in 0..4 {
                let affected = corners.contains(&corner);
                assert_eq!(
                    style.decoration.corner_shapes[corner].curvature(),
                    if affected { shape } else { -1.0 },
                    "{css},corner={corner}"
                );
                assert_eq!(
                    style.decoration.radii[corner],
                    std::array::from_fn(|_| LengthPercentage::Length(if affected {
                        radius
                    } else {
                        7.0
                    })),
                    "{css},corner={corner}"
                );
            }
            assert!(
                crate::style::declaration::supports_declaration(name, value),
                "supports {name}:{value}"
            );
        }
    }
    let style = child("#child{corner:bevel scoop square notch 1pt 2pt 3pt 4pt / 5pt 6pt 7pt 8pt}");
    assert_eq!(
        style
            .decoration
            .corner_shapes
            .map(crate::paint::CornerShape::curvature),
        [0.0, -1.0, f32::INFINITY, f32::NEG_INFINITY]
    );
    assert_eq!(
        *style.decoration.radii,
        [[1.0, 5.0], [2.0, 6.0], [3.0, 7.0], [4.0, 8.0]].map(|r| r.map(LengthPercentage::Length))
    );
}

#[test]
fn corner_shapes_map_logical_longhands_and_side_shorthands_in_every_flow() {
    for (mode, ltr, rtl) in [
        ("horizontal-tb", [0, 1, 3, 2], [1, 0, 2, 3]),
        ("vertical-rl", [1, 2, 0, 3], [2, 1, 3, 0]),
        ("vertical-lr", [0, 3, 1, 2], [3, 0, 2, 1]),
        ("sideways-rl", [1, 2, 0, 3], [2, 1, 3, 0]),
        ("sideways-lr", [3, 0, 2, 1], [0, 3, 1, 2]),
    ] {
        for (direction, corners) in [("ltr", ltr), ("rtl", rtl)] {
            let flow = format!("writing-mode:{mode};direction:{direction}");
            for (name, indices) in [
                ("block-start", [0, 1]),
                ("block-end", [2, 3]),
                ("inline-start", [0, 2]),
                ("inline-end", [1, 3]),
            ] {
                for combined in [false, true] {
                    let property = if combined {
                        format!("corner-{name}")
                    } else {
                        format!("corner-{name}-shape")
                    };
                    let value = if combined {
                        "bevel scoop 10pt 20pt / 30pt 40pt"
                    } else {
                        "bevel scoop"
                    };
                    let css = format!("#child{{{flow};{property}:{value}}}");
                    let style = child(&css);
                    let mut shapes = [1.0; 4];
                    shapes[corners[indices[0]]] = 0.0;
                    shapes[corners[indices[1]]] = -1.0;
                    assert_eq!(
                        style
                            .decoration
                            .corner_shapes
                            .map(crate::paint::CornerShape::curvature),
                        shapes,
                        "{css}"
                    );
                    if combined {
                        let mut radii = [const { [LengthPercentage::ZERO; 2] }; 4];
                        radii[corners[indices[0]]] = [
                            LengthPercentage::Length(10.0),
                            LengthPercentage::Length(30.0),
                        ];
                        radii[corners[indices[1]]] = [
                            LengthPercentage::Length(20.0),
                            LengthPercentage::Length(40.0),
                        ];
                        assert_eq!(*style.decoration.radii, radii, "{css}");
                    }
                }
            }
            for (index, name) in ["start-start", "start-end", "end-start", "end-end"]
                .into_iter()
                .enumerate()
            {
                let css = format!("#child{{{flow};corner-{name}-shape:bevel}}");
                let mut expected = [1.0; 4];
                expected[corners[index]] = 0.0;
                assert_eq!(
                    child(&css)
                        .decoration
                        .corner_shapes
                        .map(crate::paint::CornerShape::curvature),
                    expected,
                    "{css}"
                );
            }
        }
    }
}

#[test]
fn corner_shapes_share_cascade_inheritance_variables_and_zoom_rules() {
    let (parent, style) = styles(
        "#parent{corner:bevel scoop squircle notch 10pt}#child{corner:inherit;corner-top-left:square 3pt;corner-bottom-right:unset}",
        DOCUMENT,
    );
    assert_eq!(
        parent
            .decoration
            .corner_shapes
            .map(crate::paint::CornerShape::curvature),
        [0.0, -1.0, 2.0, f32::NEG_INFINITY]
    );
    assert_eq!(
        style
            .decoration
            .corner_shapes
            .map(crate::paint::CornerShape::curvature),
        [f32::INFINITY, -1.0, 1.0, f32::NEG_INFINITY]
    );
    assert_eq!(
        style.decoration.radii[0],
        [LengthPercentage::Length(3.0), LengthPercentage::Length(3.0)]
    );
    assert_eq!(style.decoration.radii[2], [LengthPercentage::ZERO; 2]);
    for (declarations, expected) in [
        ("corner-shape:bevel;all:initial", [1.0; 4]),
        ("corner-shape:bevel;corner-shape:var(--missing)", [1.0; 4]),
        (
            "--shape:bevel scoop;--radius:3pt / 4pt;corner:var(--shape) var(--radius);zoom:2",
            [0.0, -1.0, 0.0, -1.0],
        ),
        (
            "corner-top-left-shape:bevel!important;corner-start-start-shape:scoop",
            [0.0, 1.0, 1.0, 1.0],
        ),
        (
            "corner-top-left-shape:bevel;corner-start-start-shape:scoop",
            [-1.0, 1.0, 1.0, 1.0],
        ),
    ] {
        let css = format!("#child{{{declarations}}}");
        assert_eq!(
            child(&css)
                .decoration
                .corner_shapes
                .map(crate::paint::CornerShape::curvature),
            expected,
            "{css}"
        );
    }
}
