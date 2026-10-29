use super::common;

use crate::render_html;
use common::{PROFILE, local_options};

fn text(css: &str, markup: &str) -> Vec<String> {
    render_html(
        &format!("<!doctype html><style>{PROFILE}{css}</style>{markup}"),
        &local_options(),
    )
    .expect("HTML fallback must render through common styles")
    .layout
    .unwrap()
    .pages
    .into_iter()
    .flatten()
    .map(|line| line.text)
    .collect()
}

/// HTML §15.3.4: a UA rendering ruby lays annotations out apart from their base line, which
/// it places them against, adds no parentheses, and hides `rp` (§15.3.1).
#[test]
fn ruby_annotations_leave_the_base_line_and_rp_elements_are_hidden() {
    assert_eq!(
        text("", "<p>A<ruby>B<rt>C</rt><rt>D</rt></ruby>E</p>"),
        ["C", "D", "ABE"],
        "the second annotation pairs with an empty base"
    );
    assert_eq!(
        text("", "<p>A<ruby>B<rp>[</rp><rt>C</rt><rp>]</rp></ruby>D</p>"),
        ["C", "ABD"]
    );
    assert_eq!(
        text(
            "",
            "<p><ruby>B<rt>C</rt><ruby>D<rp>[</rp><rt>E</rt><rp>]</rp></ruby></ruby></p>"
        ),
        ["E", "C", "BD"]
    );
    assert_eq!(
        text("", "<p>A<rt>B</rt>C</p>"),
        ["B", "AC"],
        "a misparented annotation is wrapped in an anonymous ruby container"
    );
}

#[test]
fn ruby_annotation_pseudo_elements_cascade_and_inherit_normally() {
    assert_eq!(
        text(
            "rt::before{content:'<'}rt::after{content:'>'}",
            "<p><ruby>A<rt>B</rt></ruby></p>"
        ),
        ["<B>", "A"]
    );
    assert_eq!(
        text(
            "rt::before,rt::after{content:revert}",
            "<p><ruby>A<rt>B</rt></ruby></p>"
        ),
        ["B", "A"],
        "the UA sheet generates no fallback parentheses"
    );
    assert_eq!(
        text("rt{display:none}", "<p><ruby>A<rt>B</rt></ruby></p>"),
        ["A"]
    );
}

#[test]
fn horizontal_rules_use_html_borders_and_attribute_hints() {
    use crate::dom::{DocumentTree, Selector};
    use crate::graphics::{BorderStyle, POINTS_PER_PIXEL};
    use crate::style::{LengthPercentage, Size, Style, Stylesheet, StylesheetSource};

    for (attributes, height, border_px, solid) in [
        ("size=1", Size::Auto, [1.0, 1.0, 0.0, 1.0], false),
        (
            "size=8",
            Size::Value(LengthPercentage::Length(6.0 * POINTS_PER_PIXEL)),
            [1.0; 4],
            false,
        ),
        ("noshade size='+8junk'", Size::Auto, [4.0; 4], true),
        ("color=transparent size=8", Size::Auto, [4.0; 4], true),
        ("size=0", Size::Auto, [1.0; 4], false),
    ] {
        let tree = DocumentTree::parse(&format!("<!doctype html><hr id=target {attributes}>"))
            .expect("parse test HTML");
        let element = tree
            .select(&Selector::parse("#target").unwrap())
            .next()
            .unwrap();
        let sheet = Stylesheet::parse(&[]).unwrap();
        let style = sheet.computed(element, &Style::default()).unwrap();
        assert_eq!(style.height, height, "hr {attributes}");
        assert_eq!(
            style.decoration.borders.map(|border| border.width),
            border_px.map(|width| width * POINTS_PER_PIXEL),
            "hr {attributes}"
        );
        assert!(style.decoration.borders.iter().all(|border| border.style
            == if solid {
                BorderStyle::Solid
            } else {
                BorderStyle::Inset
            }));
    }
    let tree =
        DocumentTree::parse("<!doctype html><hr id=target color=red width='50%junk' align=left>")
            .expect("parse test HTML");
    let element = tree
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    let sheets = |css: &str| {
        Stylesheet::parse(&[StylesheetSource {
            text: css.into(),
            base: Default::default(),
        }])
        .unwrap()
    };
    let style = sheets("").computed(element, &Style::default()).unwrap();
    assert_eq!(style.width, Size::Value(LengthPercentage::Percentage(0.5)));
    assert_eq!(style.margin[3], Size::ZERO);
    assert_eq!(style.margin[1], Size::Auto);
    assert_eq!(style.color.rgb, [255, 0, 0]);
    let style = sheets("hr{width:20pt;border:0;color:blue}")
        .computed(element, &Style::default())
        .unwrap();
    assert_eq!(style.width, Size::Value(LengthPercentage::Length(20.0)));
    assert!(
        style
            .decoration
            .borders
            .iter()
            .all(|border| border.used_width() == 0.0)
    );
    assert_eq!(style.color.rgb, [0, 0, 255]);

    let output = render_html(
        &format!("<!doctype html><style>{PROFILE}</style><hr>"),
        &local_options(),
    )
    .unwrap()
    .layout
    .unwrap();
    let rules: Vec<_> = output
        .display_lists
        .iter()
        .flatten()
        .filter_map(|item| match item {
            crate::DisplayItem::Box(paint)
                if paint
                    .decoration
                    .borders
                    .iter()
                    .any(|border| border.used_width() > 0.0) =>
            {
                Some(paint)
            }
            _ => None,
        })
        .collect();
    assert_eq!(rules.len(), 1);
    common::assert_pt(rules[0].rect.height, 2.0 * POINTS_PER_PIXEL);
    common::assert_pt(rules[0].rect.width, 288.0);
}

#[test]
fn clear_attributes_enter_the_cascade_and_clear_matching_floats() {
    const FLOAT_HEIGHT: f32 = 54.0;
    const LINE_HEIGHT: f32 = 18.0;
    for value in ["left", "RIGHT", "all", "both"] {
        for side in ["left", "right"] {
            for override_clear in [false, true] {
                let rule = if override_clear { "br{clear:none}" } else { "" };
                let markup = format!(
                    "<div style='float:{side};width:60pt;height:{FLOAT_HEIGHT}pt'></div><p>A<br clear={value}>B</p>"
                );
                let trace = render_html(
                    &format!("<!doctype html><style>{PROFILE}p{{margin:0}}{rule}</style>{markup}"),
                    &local_options(),
                )
                .unwrap()
                .layout
                .unwrap();
                let lines = &trace.pages[0];
                assert_eq!(
                    lines
                        .iter()
                        .map(|line| line.text.as_str())
                        .collect::<Vec<_>>(),
                    ["A", "B"]
                );
                let clears = !override_clear
                    && (value.eq_ignore_ascii_case(side) || matches!(value, "all" | "both"));
                common::assert_pt(
                    lines[1].baseline - lines[0].baseline,
                    if clears { FLOAT_HEIGHT } else { LINE_HEIGHT },
                );
            }
        }
    }
    assert_eq!(text("", "<p>A<br clear=invalid>B</p>"), ["A", "B"]);
}

#[test]
fn quirks_margin_rules_distinguish_text_comments_blank_nodes_and_cell_edges() {
    use crate::dom::{DocumentTree, Selector};
    use crate::style::{LengthPercentage, Size, Style, Stylesheet, StylesheetSource};

    for (markup, top, bottom) in [
        ("<p id=target>X</p>", 0.0, 12.0),
        ("<!--comment--> \n<p id=target></p>", 0.0, 0.0),
        ("text<p id=target>X</p>", 12.0, 12.0),
        (
            "<span style='display:none'></span><p id=target>X</p>",
            12.0,
            12.0,
        ),
        (
            "<table><tr><td><span>A</span><p id=target></p></td></tr></table>",
            0.0,
            0.0,
        ),
        (
            "<table><tr><td><span>A</span><p id=target>X</p></td></tr></table>",
            12.0,
            0.0,
        ),
        (
            "<table><tr><td><p id=target>X</p>tail</td></tr></table>",
            0.0,
            12.0,
        ),
        ("<!doctype html><p id=target>X</p>", 12.0, 12.0),
    ] {
        let tree = DocumentTree::parse(markup).expect("parse test HTML");
        let target = tree
            .select(&Selector::parse("#target").unwrap())
            .next()
            .unwrap();
        let sheets = |css: &str| {
            Stylesheet::parse(&[StylesheetSource {
                text: css.into(),
                base: Default::default(),
            }])
            .unwrap()
        };
        let style = sheets("").computed(target, &Style::default()).unwrap();
        assert_eq!(
            style.margin[0],
            Size::Value(LengthPercentage::Length(top)),
            "top: {markup}"
        );
        assert_eq!(
            style.margin[2],
            Size::Value(LengthPercentage::Length(bottom)),
            "bottom: {markup}"
        );
        let style = sheets("p{margin:7pt}")
            .computed(target, &Style::default())
            .unwrap();
        assert_eq!(
            style.margin,
            [const { Size::Value(LengthPercentage::Length(7.0)) }; 4],
            "author declarations override quirks UA defaults: {markup}"
        );
    }
}

#[test]
fn hidden_legacy_metadata_defaults_are_overridable_ua_rules() {
    let markup = "<p>Before</p><noembed>Plugin</noembed><noframes>Frames</noframes><datalist><option>Choice</option></datalist><p>After</p>";
    assert_eq!(text("", markup), ["Before", "After"]);
    assert_eq!(
        text("noembed,noframes,datalist{display:block}", markup),
        ["Before", "Plugin", "Frames", "Choice", "After"]
    );
}

#[test]
fn legacy_text_alignment_keywords_apply_only_to_their_defined_elements() {
    use crate::dom::{DocumentTree, Selector};
    use crate::style::{Style, Stylesheet, TextAlign};

    for (markup, expected) in [
        ("<div id=target align=middle>X</div>", TextAlign::Center),
        ("<div id=target align=absmiddle>X</div>", TextAlign::Right),
        ("<p id=target align=middle>X</p>", TextAlign::Right),
        ("<h2 id=target align=absmiddle>X</h2>", TextAlign::Right),
        (
            "<table><tr><td id=target align=absmiddle>X</td></tr></table>",
            TextAlign::Center,
        ),
        (
            "<table><tr id=target align=middle><td>X</td></tr></table>",
            TextAlign::Center,
        ),
    ] {
        let tree =
            DocumentTree::parse(&format!("<!doctype html>{markup}")).expect("parse test HTML");
        let target = tree
            .select(&Selector::parse("#target").unwrap())
            .next()
            .unwrap();
        let mut parent = Style::default();
        parent.align = TextAlign::Right;
        let style = Stylesheet::parse(&[])
            .unwrap()
            .computed(target, &parent)
            .unwrap();
        assert_eq!(style.align, expected, "{markup}");
    }
}

#[test]
fn header_alignment_default_obeys_parent_and_cascade_rollback() {
    use crate::dom::{DocumentTree, Selector};
    use crate::style::{Style, Stylesheet, TextAlign};

    let tree =
        DocumentTree::parse("<!doctype html><table><tr><th id=target>Heading</th></tr></table>")
            .unwrap();
    let target = tree
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    for (css, parent_align, expected) in [
        ("", TextAlign::Start, TextAlign::Center),
        ("", TextAlign::End, TextAlign::End),
        ("th{text-align:left}", TextAlign::Start, TextAlign::Left),
        (
            "th{text-align:left;text-align:revert}",
            TextAlign::Start,
            TextAlign::Center,
        ),
        (
            "th{text-align:left;text-align:revert}",
            TextAlign::End,
            TextAlign::End,
        ),
        (
            "@layer base{th{text-align:right}}@layer top{th{text-align:revert-layer}}",
            TextAlign::Start,
            TextAlign::Right,
        ),
        ("th{text-align:inherit}", TextAlign::Start, TextAlign::Start),
    ] {
        let mut parent = Style::default();
        parent.align = parent_align;
        let sheet = Stylesheet::parse(&[crate::style::StylesheetSource {
            text: css.into(),
            base: Default::default(),
        }])
        .unwrap();
        let computed = sheet.computed(target, &parent).unwrap();
        assert_eq!(
            computed.align, expected,
            "header alignment for {css:?} with parent {parent_align:?}"
        );
    }
}

#[test]
fn user_header_alignment_preserves_direction_author_rules_and_origin_rollback() {
    use common::assert_pt;

    const CELL_WIDTH: f32 = 200.0;
    const PAGE_LEFT: f32 = 36.0;
    for direction in ["ltr", "rtl"] {
        for (author, alignment) in [
            ("", "start"),
            ("th{text-align:right}", "right"),
            ("th{text-align:right;text-align:revert}", "start"),
            ("th{text-align:center}", "center"),
        ] {
            let html = format!(
                "<!doctype html><style>{PROFILE}table{{width:{CELL_WIDTH}pt;table-layout:fixed;border-spacing:0}}th{{padding:0;font-weight:normal}}tr{{direction:{direction}}}{author}</style><table><tr><th>Header</th></tr></table>"
            );
            let mut options = local_options();
            options
                .user_stylesheets
                .push("th{text-align:inherit}".into());
            let trace = render_html(&html, &options)
                .unwrap_or_else(|error| panic!("{direction}/{author}: {error}"))
                .layout
                .unwrap();
            let lines: Vec<_> = trace.pages.iter().flatten().collect();
            assert_eq!(
                lines.len(),
                1,
                "expected one header for {direction}/{author}"
            );
            let line = lines[0];
            assert_eq!(line.text, "Header");
            let spare = CELL_WIDTH - line.width;
            let offset = match (alignment, direction) {
                ("center", _) => spare / 2.0,
                ("right", _) | ("start", "rtl") => spare,
                _ => 0.0,
            };
            assert_pt(line.x, PAGE_LEFT + offset);
        }
    }
}

#[test]
fn table_alignment_hints_center_and_float_real_boxes() {
    let output = render_html(&format!("<!doctype html><style>{PROFILE}table{{width:100pt}}</style><table align=center><tr><td>Cell</td></tr></table>"), &local_options()).unwrap().layout.unwrap();
    common::assert_pt(output.pages[0][0].x, 132.25);
    const TABLE_WIDTH: f32 = 100.0;
    const PAGE_LEFT: f32 = 36.0;
    const PAGE_RIGHT: f32 = 324.0;
    for align in ["left", "RIGHT"] {
        let markup = format!("<table align={align}><tr><td>Cell</td></tr></table><p>Body</p>");
        let trace = render_html(
            &format!("<!doctype html><style>{PROFILE}table{{width:{TABLE_WIDTH}pt;border-spacing:0}}td{{padding:0}}p{{margin:0}}</style>{markup}"),
            &local_options(),
        ).unwrap().layout.unwrap();
        let cell = trace.pages[0]
            .iter()
            .find(|line| line.text == "Cell")
            .unwrap();
        let body = trace.pages[0]
            .iter()
            .find(|line| line.text == "Body")
            .unwrap();
        common::assert_pt(
            cell.x,
            if align == "left" {
                PAGE_LEFT
            } else {
                PAGE_RIGHT - TABLE_WIDTH
            },
        );
        common::assert_pt(
            body.x,
            if align == "left" {
                PAGE_LEFT + TABLE_WIDTH
            } else {
                PAGE_LEFT
            },
        );
        common::assert_pt(cell.baseline, body.baseline);
        assert_eq!(text("table{float:none}", &markup), ["Cell", "Body"]);
    }
}

#[test]
fn heading_defaults_and_bookmarks_follow_computed_levels_and_author_cascade() {
    let html = format!(
        "<!doctype html><style>{PROFILE}\
         h1{{font-size:27pt}}#reverted{{font-size:revert;bookmark-level:revert}}\
         #custom{{font-size:17pt;bookmark-level:2}}</style><body headingoffset=2>\
         <h1 id=reverted>Offset</h1><div headingreset headingoffset=2><h2>Reset</h2></div>\
         <h6 headingoffset=1000>Capped</h6><h1 id=custom>Custom</h1>"
    );
    let document = crate::document::StyledDocument::parse(
        &html,
        &local_options().base,
        &[],
        &crate::resources::FileResolver::default(),
    )
    .unwrap();
    assert_eq!(
        document
            .navigation()
            .bookmarks
            .iter()
            .map(|bookmark| (bookmark.label.as_str(), bookmark.level))
            .collect::<Vec<_>>(),
        [("Offset", 3), ("Reset", 4), ("Capped", 9), ("Custom", 2)]
    );
    let output = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    for (text, size) in [
        ("Offset", 14.04),
        ("Reset", 12.0),
        ("Capped", 8.04),
        ("Custom", 17.0),
    ] {
        let line = output
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == text)
            .unwrap();
        common::assert_pt(line.runs[0].font_size, size);
    }
}

#[test]
fn legacy_image_alignment_centers_the_painted_box_on_the_correct_font_line() {
    let html = format!(
        "<!doctype html><style>{PROFILE}p{{margin:0}}</style>\
        <p>A<img src='../assets/image-pattern.png' width=40 height=20 border=2 hspace=4 vspace=2 align=middle>Z</p>\
        <p>B<img src='../assets/image-pattern.png' width=40 height=20 border=2 hspace=4 vspace=2 align=absmiddle>Z</p>"
    );
    let output = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let images: Vec<_> = output.display_lists[0]
        .iter()
        .filter_map(|item| match item {
            crate::DisplayItem::Image(paint) => Some(paint),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), 2);
    for image in &images {
        common::assert_pt(image.rect.width, 30.0);
        common::assert_pt(image.rect.height, 15.0);
    }
    common::assert_pt(
        images[0].rect.y + images[0].rect.height / 2.0,
        output.pages[0][0].baseline,
    );
    const DEJAVU_X_HEIGHT_PT: f32 = 6.5625;
    common::assert_pt(
        images[1].rect.y + images[1].rect.height / 2.0,
        output.pages[0][1].baseline - DEJAVU_X_HEIGHT_PT / 2.0,
    );
    for align in ["left", "RIGHT"] {
        let html = format!(
            "<!doctype html><style>{PROFILE}p{{margin:0}}</style><p><img src='../assets/image-pattern.png' width=80 height=72 align={align}>Body</p>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let images: Vec<_> = trace.display_lists[0]
            .iter()
            .filter_map(|item| match item {
                crate::DisplayItem::Image(paint) => Some(paint),
                _ => None,
            })
            .collect();
        assert_eq!(images.len(), 1, "{align}: image hint paints once");
        common::assert_pt(images[0].rect.x, if align == "left" { 36.0 } else { 264.0 });
        common::assert_pt(images[0].rect.y, 36.0);
        common::assert_pt(
            trace.pages[0][0].x,
            if align == "left" { 96.0 } else { 36.0 },
        );
        assert_eq!(trace.pages[0][0].text, "Body");
    }
}
