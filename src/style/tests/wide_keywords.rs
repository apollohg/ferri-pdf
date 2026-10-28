use crate::style::{
    Display, LengthPercentage, Size, Style, Stylesheet, StylesheetSource, TableLayout,
    VerticalAlign, WhiteSpace,
};
use ferri_pdf_dom::{DocumentTree, Selector};
use ferri_pdf_resources::FileResolver;

fn styles(css: &str, html: &str) -> (Style, Style) {
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])
    .unwrap();
    let html = DocumentTree::parse(html).expect("parse test HTML");
    let select = |id: &str| html.select(&Selector::parse(id).unwrap()).next().unwrap();
    let parent = sheet
        .computed_with_resolver(
            select("#parent"),
            &Style::default(),
            &FileResolver::default(),
        )
        .unwrap();
    let child = sheet
        .computed_with_resolver(select("#child"), &parent, &FileResolver::default())
        .unwrap();
    (parent, child)
}

#[test]
fn inherited_computed_values_keep_percentages_and_unitless_leading() {
    let (parent, child) = styles(
        "#parent{font-size:20pt;line-height:1.5;width:50%;padding:2em;color:#224466}#child{font-size:10pt;line-height:inherit;width:inherit;padding:inherit;color:inherit;display:inherit}",
        "<section id=parent><div id=child>Text</div></section>",
    );
    assert_eq!(child.size, 10.0);
    assert_eq!(
        child.line_height.resolve(child.size),
        Some(15.0),
        "unitless inherited leading scales with the child's size"
    );
    assert_eq!(child.width, Size::Value(LengthPercentage::Percentage(0.5)));
    assert_eq!(
        child.padding, parent.padding,
        "computed em lengths do not resolve again on the child"
    );
    assert_eq!(child.padding[0], LengthPercentage::Length(40.0));
    assert_eq!(child.color, parent.color);
    assert_eq!(child.display, Display::Block);
    let (_, fixed) = styles(
        "#parent{font-size:20pt;line-height:30pt}#child{font-size:10pt;line-height:inherit}",
        "<section id=parent><div id=child>X</div></section>",
    );
    assert_eq!(
        fixed.line_height.resolve(fixed.size),
        Some(30.0),
        "fixed leading remains a computed length"
    );
}

#[test]
fn unset_overrides_heading_defaults_and_resets_noninherited_geometry() {
    let (parent, child) = styles(
        "#parent{font-size:15pt;color:#008800;font-weight:normal}#child{font-size:24pt;font-size:unset;font-weight:unset;color:red;color:unset;margin:20pt;margin:unset;padding:12pt;padding:initial;width:20pt;width:unset;break-after:unset;break-inside:initial}",
        "<section id=parent><h2 id=child>Heading</h2></section>",
    );
    assert_eq!(
        child.size, parent.size,
        "unset must not retain the heading's user-agent size"
    );
    assert_eq!(child.font_weight, parent.font_weight);
    assert_eq!(child.color, parent.color);
    assert_eq!(child.margin, [Size::ZERO; 4]);
    assert_eq!(child.padding, [LengthPercentage::ZERO; 4]);
    assert_eq!(child.width, Size::Auto);
    assert_eq!(child.keep, crate::style::BreakInside::Auto);
    assert_eq!(child.break_after, crate::style::BreakBetween::Auto);
}

#[test]
fn wide_shorthands_preserve_longhand_priority_and_current_color() {
    let (parent, child) = styles(
        "#parent{border:3pt solid blue;padding:2pt 4pt 6pt 8pt}#child{color:#008800;border:inherit;border-color:initial;padding:inherit!important;padding-left:30pt;background:initial}",
        "<section id=parent><div id=child>Text</div></section>",
    );
    assert_eq!(child.padding, parent.padding);
    for border in child.decoration.borders {
        assert_eq!(border.width, 3.0);
        assert_eq!(border.style, ferri_pdf_paint::BorderStyle::Solid);
        assert_eq!(
            border.color, child.color,
            "initial border color is the child's currentColor"
        );
    }
    assert_eq!(child.decoration.background.alpha, 0.0);
}

#[test]
fn wide_keywords_apply_to_table_values_and_escaped_keyword_tokens() {
    let (_, child) = styles(
        r"#parent{table-layout:fixed;vertical-align:bottom;border-spacing:3pt 7pt;white-space:pre}#child{table-layout:inherit;vertical-align:unset;border-spacing:inherit;white-space:\69 nitial/**/;box-sizing:initial;display:initial}",
        "<table id=parent><tbody><tr><td id=child>Cell</td></tr></tbody></table>",
    );
    assert_eq!(child.table.layout, TableLayout::Fixed);
    assert_eq!(
        child.table.vertical_align,
        VerticalAlign::Baseline,
        "unset overrides HTML cell alignment"
    );
    assert_eq!(child.table.spacing, [3.0, 7.0]);
    assert_eq!(child.white_space, WhiteSpace::Normal);
    assert_eq!(child.display, Display::Inline);
    assert_eq!(child.box_sizing, crate::style::BoxSizing::ContentBox);
}

#[test]
fn border_width_inheritance_uses_the_parents_computed_width() {
    let (_, child) = styles(
        "#parent{border:10pt none red}#child{border-style:solid;border-width:inherit}",
        "<section id=parent><p id=child>Text</p></section>",
    );
    assert!(
        child
            .decoration
            .borders
            .iter()
            .all(|border| border.used_width() == 0.0),
        "a none/hidden parent's computed border width is zero"
    );
}
