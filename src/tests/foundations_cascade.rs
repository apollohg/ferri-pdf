use super::common;

use crate::{render_bytes, render_html};
use common::{PROFILE, assert_pt, local_options};
use ferri_pdf_dom::{DocumentTree, Element, Selector, TreeRelation};
use ferri_pdf_style::{Display, LengthPercentage, Size, Style, Stylesheet, StylesheetSource};

fn sheet(author: &str, user: &str) -> Stylesheet {
    let source = |text: &str| StylesheetSource {
        text: text.into(),
        base: Default::default(),
    };
    Stylesheet::parse_with_origins(&[source(author)], &[source(user)], &Default::default()).unwrap()
}

fn computed(sheet: &Stylesheet, element: Element<'_>) -> Style {
    let parent = element
        .parent(TreeRelation::Inheritance)
        .map(|parent| computed(sheet, parent))
        .unwrap_or_default();
    sheet.computed(element, &parent).unwrap()
}

fn style(author: &str, user: &str, markup: &str) -> Style {
    let tree = DocumentTree::parse(&format!("<!doctype html>{markup}")).expect("parse test HTML");
    let target = tree
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    computed(&sheet(author, user), target)
}

#[test]
fn shadow_stylesheets_keep_layer_order_and_titles_in_their_own_tree() {
    let html = format!(
        "<!doctype html><meta http-equiv=default-style content=print><style title=print>{PROFILE}@layer first,second;p{{font-size:13pt;margin:0}}</style><p>Outside</p><div><template shadowrootmode=open><style title=component-a>@layer second,first;@layer first{{p{{font-size:17pt}}}}@layer second{{p{{font-size:19pt}}}}</style><p>First</p><div><template shadowrootmode=closed><style>p{{font-size:23pt}}</style><p>Nested</p></template></div><slot></slot></template><p>Light</p></div><div><template shadowrootmode=closed><style title=component-b>@layer first,second;@layer first{{p{{font-size:29pt}}}}@layer second{{p{{font-size:31pt}}}}</style><p>Second</p></template></div><p>After</p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let lines: Vec<_> = trace.pages.iter().flatten().collect();
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Outside", "First", "Nested", "Light", "Second", "After"]
    );
    for (line, expected) in lines.into_iter().zip([13.0, 17.0, 23.0, 13.0, 31.0, 13.0]) {
        assert_pt(line.font_size, expected);
    }
}

#[test]
fn shadow_important_order_inline_rollback_and_custom_properties() {
    for (outer, shadow, inline, expected) in [
        ("#host{font-size:17pt}", ":host{font-size:19pt}", "", 17.0),
        (
            "#host{font-size:17pt!important}",
            ":host{font-size:19pt!important}",
            "font-size:31pt!important",
            19.0,
        ),
        (
            "#host{font-size:17pt!important}",
            ":host{font-size:19pt!important;font-size:revert-layer!important}",
            "font-size:31pt!important",
            31.0,
        ),
        (
            "#host{--size:23pt}",
            ":host{font-size:var(--size)}",
            "",
            23.0,
        ),
    ] {
        let html = format!(
            "<!doctype html><style>{PROFILE}{outer}</style><div id=host style='{inline}'><template shadowrootmode=open><style>{shadow}p{{font-size:inherit;margin:0}}</style><p>Shadow sample</p></template></div>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "Shadow sample")
            .expect("the shadow tree is printed");
        assert_pt(line.font_size, expected);
    }
}

#[test]
fn slotted_hosts_order_sibling_shadow_contexts_before_specificity() {
    for (importance, expected) in [("", 23.0), ("!important", 31.0)] {
        let html = format!(
            "<!doctype html><style>{PROFILE}</style><div><template shadowrootmode=open><style>::slotted(x-child){{font-size:23pt{importance}}}</style><slot></slot></template><x-child><template shadowrootmode=open><style>:host{{font-size:31pt{importance}}}</style><p>Sibling contexts</p></template></x-child></div>"
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        let line = trace
            .pages
            .iter()
            .flatten()
            .find(|line| line.text == "Sibling contexts")
            .unwrap();
        assert_pt(line.font_size, expected);
    }
}

#[test]
fn origin_importance_inline_and_hints_share_cascade() {
    for (author, user, inline, expected) in [
        ("p{font-size:18pt}", "#target{font-size:22pt}", "", 18.0),
        (
            "#target{font-size:18pt!important}",
            "p{font-size:22pt!important}",
            "font-size:30pt!important",
            22.0,
        ),
        (
            "#target#target#target{font-size:18pt!important}",
            "",
            "font-size:30pt!important",
            30.0,
        ),
        (
            "#target#target#target{font-size:18pt}",
            "",
            "font-size:30pt",
            30.0,
        ),
    ] {
        let target = style(
            author,
            user,
            &format!("<p id=target style='{inline}'>Text</p>"),
        );
        assert_pt(target.size, expected);
    }
    let hints = "<table><tr><td id=target width=80 nowrap>Cell</table>";
    assert_eq!(
        style("", "td{width:100pt}", hints).width,
        Size::Value(LengthPercentage::Length(60.0)),
        "HTML hint outranks normal user rules"
    );
    assert_eq!(
        style("td{width:120pt;white-space:normal}", "", hints).width,
        Size::Value(LengthPercentage::Length(120.0)),
        "author rule outranks HTML hint"
    );
    let th = style(
        "th{font-weight:normal;text-align:left}",
        "",
        "<table><tr><th id=target>Header</table>",
    );
    assert_eq!(th.font_weight, ferri_pdf_fonts::FontWeight::NORMAL);
    assert_eq!(th.align, ferri_pdf_style::TextAlign::Left);
    let mut options = local_options();
    options
        .user_stylesheets
        .push("p{font-size:23pt!important;color:#123456!important}".into());
    let html = format!(
        "<style>{PROFILE}p{{font-size:17pt!important}}</style><p style='font-size:31pt!important'>Text</p>"
    );
    for output in [
        render_html(&html, &options).unwrap(),
        render_bytes(html.as_bytes(), &options).unwrap(),
    ] {
        let trace = output.layout.unwrap();
        assert_pt(trace.pages[0][0].font_size, 23.0);
        assert_eq!(trace.pages[0][0].color.rgb, [0x12, 0x34, 0x56]);
    }
}

#[test]
fn important_ua_rules_outrank_user_and_author_important_rules() {
    let target = style(
        "form{display:block!important}",
        "form{display:block!important}",
        "<table><form id=target></form><tr><td>Cell</table>",
    );
    assert_eq!(
        target.display,
        Display::None,
        "HTML's mandatory table/form UA declaration outranks both important origins"
    );
}

#[test]
fn revert_removes_the_correct_origin() {
    let markup = "<section style='font-size:11pt'><h2 id=target style='font-size:revert!important'>Heading</h2></section>";
    assert_pt(
        style("h2{font-size:29pt!important}", "h2{font-size:19pt}", markup).size,
        19.0,
    );
    assert_pt(
        style(
            "h2{font-size:29pt}",
            "h2{font-size:19pt;font-size:revert!important}",
            markup,
        )
        .size,
        16.5,
    );
    assert_pt(
        style(
            "h2{font-size:29pt;font-size:revert}",
            "",
            "<section style='font-size:11pt'><h2 id=target>Heading</h2></section>",
        )
        .size,
        16.5,
    );
    let hint = style(
        "td{width:revert}",
        "td{width:101pt}",
        "<table><tr><td id=target width=80>Cell</table>",
    );
    assert_eq!(
        hint.width,
        Size::Value(LengthPercentage::Length(101.0)),
        "author rollback removes HTML hints too"
    );
    let custom = style(
        "#target{--size:30pt;--size:revert;font-size:var(--size)}",
        "#target{--size:21pt}",
        "<p id=target>Text</p>",
    );
    assert_pt(custom.size, 21.0);
}

#[test]
fn all_and_inherited_initial_values_match_registry() {
    let target = style(
        "section{font-size:20pt;color:green;line-height:1.5;padding:9pt;width:70pt}#target{--size:17pt;direction:rtl;all:initial;font-size:var(--size)}",
        "",
        "<section><h2 id=target>Text</h2></section>",
    );
    assert_pt(target.size, 17.0);
    assert!(target.rtl, "all does not reset direction");
    assert_eq!(target.display, Display::Inline);
    assert_eq!(target.margin, [Size::ZERO; 4]);
    assert_eq!(target.padding, [LengthPercentage::ZERO; 4]);
    assert_eq!(target.color, ferri_pdf_graphics::Color::BLACK);
    assert_eq!(target.line_height, ferri_pdf_style::LineHeight::Normal);
    let inherited = style(
        "section{font-size:20pt;width:70pt;padding:9pt}#target{all:inherit}",
        "",
        "<section><h2 id=target>Text</h2></section>",
    );
    assert_pt(inherited.size, 20.0);
    assert_eq!(
        inherited.padding,
        [const { LengthPercentage::Length(9.0) }; 4]
    );
    assert_eq!(inherited.width, Size::Value(LengthPercentage::Length(70.0)));
    let anonymous = inherited.anonymous(Display::Block);
    assert_pt(anonymous.size, 20.0);
    assert_eq!(anonymous.padding, [LengthPercentage::ZERO; 4]);
    assert_eq!(anonymous.width, Size::Auto);
    let default_heading = style(
        "section{font-size:20pt}",
        "",
        "<section><h3 id=target>Text</h3></section>",
    );
    assert_pt(default_heading.size, 23.4);
    assert_eq!(default_heading.margin[0].resolve(0.0), Some(23.4));
}

#[test]
fn computed_invalid_variable_does_not_revive_old_value() {
    let target = style(
        "section{font-size:14pt}#target{font-size:25pt;font-size:var(--bad);padding:19pt;padding:var(--bad);--bad:red}",
        "#target{font-size:20pt;padding:11pt}",
        "<section><h2 id=target>Text</h2></section>",
    );
    assert_pt(target.size, 14.0);
    assert_eq!(target.padding, [LengthPercentage::ZERO; 4]);
    let target = style(
        "section{--size:13pt}#target{--size:21pt;--size:var(--cycle);--cycle:var(--size);font-size:var(--size,16pt)}",
        "#target{--size:22pt}",
        "<section><p id=target>Text</p></section>",
    );
    assert_pt(target.size, 16.0);
}

#[test]
fn pseudo_page_and_margin_boxes_use_the_same_origins_and_defaults() {
    let mut options = local_options();
    options.user_stylesheets.push("p::before{font-size:21pt!important}@page{font-size:19pt!important;@top-center{content:'M';font-size:23pt!important}}".into());
    let html = format!(
        "<style>{PROFILE}p::before{{content:'P';font-size:16pt!important}}@page{{font-size:17pt!important;@top-center{{content:'A';font-size:18pt!important}}}}</style><p>Text</p>"
    );
    let trace = render_html(&html, &options).unwrap().layout.unwrap();
    let runs: Vec<_> = trace.pages[0].iter().flat_map(|line| &line.runs).collect();
    assert!(
        runs.iter()
            .any(|run| run.text == "P" && run.font_size == 21.0),
        "user important applies to pseudo-element"
    );
    assert!(
        runs.iter()
            .any(|run| run.text == "A" && run.font_size == 23.0),
        "user important applies in page-margin context"
    );
    let tree =
        DocumentTree::parse("<!doctype html><p id=target>Text</p>").expect("parse test HTML");
    let target = tree
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    let sheet = sheet(
        "p::before{content:'A';font-size:revert}",
        "p::before{font-size:19pt}",
    );
    let parent = computed(&sheet, target);
    let (pseudo, _) = sheet
        .pseudo(target, &parent, ferri_pdf_style::PseudoElement::Before)
        .unwrap()
        .unwrap();
    assert_pt(pseudo.size, 19.0);
    assert_eq!(pseudo.margin, [Size::ZERO; 4]);
}

#[test]
fn substituted_wide_keywords_roll_back_properties_and_custom_properties() {
    let markup = "<section style='--size:17pt'><p id=target>Text</p></section>";
    assert_pt(
        style(
            "#target{font-size:31pt;font-size:var(--missing,revert)}",
            "p{font-size:19pt}",
            markup,
        )
        .size,
        19.0,
    );
    assert_pt(
        style(
            "#target{--size:31pt;--size:var(--missing,revert);font-size:var(--size)}",
            "p{--size:21pt}",
            markup,
        )
        .size,
        21.0,
    );
    assert_pt(
        style(
            "#target{--size:var(--missing,inherit);font-size:var(--size)}",
            "",
            markup,
        )
        .size,
        17.0,
    );
    assert_pt(
        style(
            "#target{font-size:var(--missing,initial)}",
            "p{font-size:19pt}",
            markup,
        )
        .size,
        12.0,
    );
}

#[test]
fn custom_property_dependencies_observe_rollback_before_substitution() {
    for (keyword, expected) in [("revert", 23.0), ("inherit", 17.0)] {
        let author = format!(
            "#target{{--seed:var(--missing,{keyword});--copy:var(--seed);\
             --copy-again:var(--copy);font-size:var(--copy-again)}}"
        );
        let target = style(
            &author,
            "p{--seed:23pt;--copy:27pt}",
            "<section style='--seed:17pt'><p id=target>Text</p></section>",
        );
        assert_pt(target.size, expected);
    }
}

#[test]
fn important_attached_revert_layer_keeps_important_stylesheet_rules() {
    let markup = "<p id=target style='font-size:var(--missing,revert-layer)!important'>Text</p>";
    assert_pt(
        style("p{font-size:22pt!important}", "p{font-size:19pt}", markup).size,
        22.0,
    );
    let markup = "<p id=target style='font-size:revert!important'>Text</p>";
    assert_pt(
        style("p{font-size:22pt!important}", "p{font-size:19pt}", markup).size,
        19.0,
    );
}

#[test]
fn revert_layer_preserves_the_independent_html_hint_origin() {
    for attached in [false, true] {
        let markup = if attached {
            "<table><tr><td id=target width=80 style='width:revert-layer'>Cell</table>"
        } else {
            "<table><tr><td id=target width=80>Cell</table>"
        };
        let author = if attached {
            "td{width:200pt}"
        } else {
            "td{width:revert-layer}"
        };
        assert_eq!(
            style(author, "td{width:101pt}", markup).width,
            Size::Value(LengthPercentage::Length(60.0)),
            "revert-layer keeps HTML hints, unlike revert (attached={attached})"
        );
    }
}

#[test]
fn html_hints_use_the_html_attribute_parsers_and_parent_context() {
    let dimensions = style(
        "",
        "",
        "<table cellspacing='+3junk' cellpadding=2><tr><td id=target width=' 120.5%junk'>Cell</table>",
    );
    assert_eq!(
        dimensions.width,
        Size::Value(LengthPercentage::Percentage(1.205))
    );
    assert_eq!(
        dimensions.padding,
        [const { LengthPercentage::Length(1.5) }; 4]
    );
    assert_eq!(dimensions.table.spacing, [2.25; 2]);
    assert_eq!(
        style(
            "",
            "td{width:21pt}",
            "<table><tr><td id=target width=0>Cell</table>"
        )
        .width,
        Size::Value(LengthPercentage::Length(21.0)),
        "a zero cell width is ignored by HTML's nonzero dimension parser"
    );
    let legacy = style(
        "",
        "",
        "<font id=target color=chucknorris size='+2'>Text</font>",
    );
    assert_eq!(legacy.color.rgb, [192, 0, 0]);
    assert_pt(legacy.size, 18.0);
    let system = style("", "", "<font id=target color=Canvas>Text</font>");
    assert_eq!(
        system.color.rgb,
        [202, 0, 160],
        "HTML legacy colours do not recognize CSS system colours"
    );
    let body = style(
        "",
        "",
        "<body id=target marginheight=10 topmargin=30 marginwidth='-2' leftmargin=40>Text</body>",
    );
    assert_eq!(body.margin[0].resolve(0.0), Some(7.5));
    assert_eq!(
        body.margin[3].resolve(0.0),
        Some(6.0),
        "an invalid first attribute falls back to the UA margin, rather than trying a later attribute"
    );
    let header = style(
        "tr{text-align:right}",
        "",
        "<table><tr><th id=target>Header</table>",
    );
    assert_eq!(header.align, ferri_pdf_style::TextAlign::Right);
}

#[test]
fn page_context_resets_and_inherits_through_the_common_registry() {
    let tree =
        DocumentTree::parse("<!doctype html><html><p>Text</p></html>").expect("parse test HTML");
    let mut styles = sheet(
        "html{margin:11pt;font-size:20pt}@page{margin:inherit;@top-center{all:inherit;content:'Header'}}",
        "",
    );
    let root = computed(&styles, tree.root_element());
    styles.resolve_page_geometry(&root).unwrap();
    assert_eq!(styles.margin, [11.0; 4]);
    let margins = styles.page_margins(&root).unwrap();
    assert_pt(margins[0].style.size, 20.0);
    let mut styles = sheet(
        "@page{all:initial;@top-center{all:initial;content:'Header'}}",
        "",
    );
    styles.resolve_page_geometry(&root).unwrap();
    assert_eq!(
        styles.margin, [0.0; 4],
        "all resets UA page margins while leaving the size descriptor alone"
    );
    let margins = styles.page_margins(&root).unwrap();
    assert_pt(margins[0].style.size, 12.0);
    assert_eq!(
        margins[0].style.display,
        Display::Block,
        "display does not apply to page-margin boxes"
    );
}

#[test]
fn page_inherits_computed_values_from_a_hidden_root() {
    let tree =
        DocumentTree::parse("<!doctype html><p>Hidden document</p>").expect("parse test HTML");
    let mut styles = sheet(
        "html{display:none;font-size:20pt;color:#123456;margin:11pt}\
         @page{margin:inherit;@top-center{content:'Header'}}",
        "",
    );
    let root = computed(&styles, tree.root_element());
    assert_eq!(root.display, Display::None);
    styles.resolve_page_geometry(&root).unwrap();
    assert_eq!(styles.margin, [11.0; 4]);
    let margins = styles.page_margins(&root).unwrap();
    assert_pt(margins[0].style.size, 20.0);
    assert_eq!(margins[0].style.color.rgb, [0x12, 0x34, 0x56]);
}

#[test]
fn unregistered_url_tokens_resolve_in_the_consuming_stylesheet() {
    use crate::{
        Resource, ResourceBase, ResourceError, ResourceResolver, Url, render_with_resolver,
    };
    use std::cell::RefCell;
    struct Sources {
        calls: RefCell<Vec<String>>,
        function: &'static str,
        variable: bool,
    }
    impl ResourceResolver for Sources {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.calls.borrow_mut().push(reference.into());
            match reference {
                "definitions.css" => Ok(Resource {
                    bytes: format!(":root{{--image:{}('tile.png')}}", self.function).into_bytes(),
                    base: Url::parse("https://example.org/definitions/style.css")
                        .unwrap()
                        .into(),
                }),
                "consumer.css" => Ok(Resource {
                    bytes: format!(
                        "p{{background-image:{};width:30pt;height:20pt}}",
                        if self.variable {
                            "var(--image)".into()
                        } else {
                            format!("{}('tile.png')", self.function)
                        }
                    )
                    .into_bytes(),
                    base: Url::parse("https://example.org/consumer/style.css")
                        .unwrap()
                        .into(),
                }),
                "https://example.org/consumer/tile.png" => Ok(Resource {
                    bytes: include_bytes!("../../tests/oracle/assets/image-pattern.png").to_vec(),
                    base: Url::parse(reference).unwrap().into(),
                }),
                _ => Err(ResourceError::Denied(base.resolve(reference)?)),
            }
        }
    }
    for (function, variable) in [("url", true), (r"u\72l", true), (r"u\72l", false)] {
        let sources = Sources {
            calls: RefCell::new(Vec::new()),
            function,
            variable,
        };
        let options = crate::RenderOptions {
            base: Url::parse("https://example.org/document.html")
                .unwrap()
                .into(),
            ..local_options()
        };
        render_with_resolver("<!doctype html><link rel=stylesheet href=definitions.css><link rel=stylesheet href=consumer.css><p>Text</p>", &options, &sources).unwrap();
        assert_eq!(
            *sources.calls.borrow(),
            [
                "definitions.css",
                "consumer.css",
                "https://example.org/consumer/tile.png"
            ],
            "custom URL provenance for {function}"
        );
    }
}

#[test]
fn quirks_presentation_enters_the_ua_origin_and_hint_origin() {
    let styles = sheet("body{font-size:24pt}", "");
    let tree = DocumentTree::parse("<table><tr><td id=target nowrap width=20>Text</table>")
        .expect("parse test HTML");
    let target = tree
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    let target = computed(&styles, target);
    assert_pt(target.size, 12.0);
    assert_eq!(target.white_space, ferri_pdf_style::WhiteSpace::Normal);
    let tree = DocumentTree::parse("<li id=target>Orphan</li>").expect("parse test HTML");
    let target = tree
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    assert_eq!(
        computed(&styles, target).list_style_position,
        ferri_pdf_style::ListStylePosition::Inside
    );
    let standard = style(
        "body{font-size:24pt}",
        "",
        "<table><tr><td id=target nowrap width=20>Text</table>",
    );
    assert_pt(standard.size, 24.0);
    assert_eq!(standard.white_space, ferri_pdf_style::WhiteSpace::Nowrap);
    let styles = sheet("td{white-space:pre}table{font-size:20pt}", "");
    let tree = DocumentTree::parse("<table><tr><td id=target nowrap width=20>Text</table>")
        .expect("parse test HTML");
    let target = tree
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    let target = computed(&styles, target);
    assert_pt(target.size, 20.0);
    assert_eq!(target.white_space, ferri_pdf_style::WhiteSpace::Pre);
}

#[test]
fn phrasing_defaults_render_through_the_common_style_path() {
    let html = format!(
        "<!doctype html><style>{PROFILE}</style><p>A<sup>2</sup><sub>3</sub><mark>M</mark><font color=chucknorris>F</font></p>"
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    let first = &trace.pages[0][0];
    assert_eq!(first.text, "A23MF");
    let sup = first.runs.iter().find(|run| run.text == "2").unwrap();
    let sub = first.runs.iter().find(|run| run.text == "3").unwrap();
    assert_pt(sup.font_size, 10.0);
    assert_pt(sub.font_size, 10.0);
    assert!(sup.baseline_shift < 0.0 && sub.baseline_shift > 0.0);
    assert_eq!(
        first
            .runs
            .iter()
            .find(|run| run.text == "F")
            .unwrap()
            .color
            .rgb,
        [192, 0, 0]
    );
}

#[test]
fn list_match_parent_resolves_alignment_in_the_parents_direction() {
    let markup = "<ol style='direction:rtl'><li id=target dir=ltr>Text</li></ol>";
    assert_eq!(
        style("", "", markup).align,
        ferri_pdf_style::TextAlign::Right
    );
    assert_eq!(
        style("li{text-align:inherit}", "", markup).align,
        ferri_pdf_style::TextAlign::Start
    );
    assert_eq!(
        style("li{text-align:match-parent}", "", markup).align,
        ferri_pdf_style::TextAlign::Right
    );
}

#[test]
fn html_table_spans_reuse_integer_prefix_parsing_and_html_bounds() {
    let target = style(
        "",
        "",
        "<table><tr><td id=target colspan='2junk' rowspan='+3tail'>Cell</table>",
    );
    assert_eq!((target.table.column_span, target.table.row_span), (2, 3));
    let target = style(
        "",
        "",
        "<table><colgroup id=target span=999999999999999999999999></table>",
    );
    assert_eq!(target.table.column_span, 1000);
    let target = style(
        "",
        "",
        "<table><tr><td id=target colspan=-1 rowspan=-0>Cell</table>",
    );
    assert_eq!((target.table.column_span, target.table.row_span), (1, 0));
}

#[test]
fn invalid_html_background_url_is_ignored_before_resource_resolution() {
    let output = render_html(
        "<!doctype html><body background='http://['><p>Text</p></body>",
        &local_options(),
    )
    .expect("HTML ignores an unparseable background URL");
    assert_eq!(output.layout.unwrap().pages[0][0].text, "Text");
}

#[test]
fn embedded_spacing_border_and_alignment_hints_use_the_common_origins() {
    use ferri_pdf_style::VerticalAlign;
    let markup = "<img id=target width=40 height=20 hspace='4%tail' vspace=2 border='+2tail' align=absmiddle>";
    let hints = style(
        "",
        "img{margin:9pt;border:4pt solid;vertical-align:top}",
        markup,
    );
    assert_eq!(
        hints.margin[1],
        Size::Value(LengthPercentage::Percentage(0.04))
    );
    assert_eq!(hints.margin[0], Size::Value(LengthPercentage::Length(1.5)));
    assert_eq!(
        hints.decoration.borders.map(|border| border.width),
        [1.5; 4]
    );
    assert_eq!(hints.table.vertical_align, VerticalAlign::Middle);
    let author = style(
        "img{margin:3pt;border:0;vertical-align:baseline}",
        "",
        markup,
    );
    assert_eq!(
        author.margin,
        [const { Size::Value(LengthPercentage::Length(3.0)) }; 4]
    );
    assert!(
        author
            .decoration
            .borders
            .iter()
            .all(|border| border.used_width() == 0.0)
    );
    assert_eq!(author.table.vertical_align, VerticalAlign::Baseline);
    let zero = style("", "img{border:4pt solid}", "<img id=target border=0>");
    assert_eq!(
        zero.decoration.borders.map(|border| border.width),
        [4.0; 4],
        "zero img border creates no presentational hint"
    );
    let button = style(
        "",
        "",
        "<input id=target type=ImAgE hspace=4 vspace=2 border=2 align=texttop>",
    );
    assert_eq!(button.margin[3], Size::Value(LengthPercentage::Length(3.0)));
    assert_eq!(button.table.vertical_align, VerticalAlign::TextTop);
    let ordinary = style(
        "",
        "",
        "<input id=target type=text hspace=4 border=2 align=texttop>",
    );
    assert_eq!(ordinary.margin, [Size::ZERO; 4]);
    assert_eq!(ordinary.table.vertical_align, VerticalAlign::Baseline);
}

#[test]
fn iframe_frameborder_uses_signed_html_integer_recovery() {
    for (value, width) in [
        ("invalid", 0.0),
        ("-0tail", 0.0),
        ("+0tail", 0.0),
        ("-1tail", 3.0),
        ("1tail", 3.0),
    ] {
        let markup = format!("<iframe id=target frameborder='{value}'></iframe>");
        let hints = style("", "iframe{border:3pt solid}", &markup);
        assert_eq!(
            hints.decoration.borders.map(|border| border.width),
            [width; 4],
            "frameborder={value}"
        );
    }
}

#[test]
fn quirks_image_spacing_is_a_ua_default_below_hspace_and_author_rules() {
    for (align, side) in [("left", 1), ("right", 3)] {
        for (hint, author, user, expected) in [
            ("", "img{float:none}", "", 2.25),
            ("", "img{float:none}", "img{margin:9pt}", 9.0),
            ("hspace=8", "img{float:none}", "img{margin:9pt}", 6.0),
            ("hspace=8", "img{float:none;margin:5pt}", "", 5.0),
        ] {
            let tree = DocumentTree::parse(&format!("<img id=target align={align} {hint}>"))
                .expect("parse test HTML");
            let selector = Selector::parse("#target").unwrap();
            let target = tree.select(&selector).next().unwrap();
            let result = computed(&sheet(author, user), target);
            assert_eq!(
                result.margin[side],
                Size::Value(LengthPercentage::Length(expected)),
                "{align}, {hint}, author={author}, user={user}"
            );
        }
    }
}

#[test]
fn nested_anonymous_layers_reverse_important_order() {
    for (css, expected) in [
        (
            "@layer a,b;@layer b{p{font-size:23pt}}@layer a{#target{font-size:19pt}}",
            23.0,
        ),
        (
            "@layer a,b;p{font-size:31pt!important}@layer b{p{font-size:23pt!important}}@layer a{p{font-size:19pt!important}}",
            19.0,
        ),
        ("p{font-size:17pt}@layer a{#target{font-size:23pt}}", 17.0),
        (
            "@layer a{@layer child{p{font-size:23pt}}p{font-size:19pt}}",
            19.0,
        ),
        (
            "@layer a{@layer child{p{font-size:23pt!important}}p{font-size:19pt!important}}",
            23.0,
        ),
        (
            "@layer a.x,a.y;@layer a.y{p{font-size:23pt}}@layer a.x{p{font-size:19pt}}",
            23.0,
        ),
        (
            "@layer a{@layer x,y;@layer y{p{font-size:23pt}}@layer x{p{font-size:19pt}}}",
            23.0,
        ),
        (
            "@layer a,b;@layer b{p{font-size:23pt}}@layer a.last{p{font-size:19pt}}",
            23.0,
        ),
        (
            "@layer a{p{font-size:19pt}}@layer b{p{font-size:23pt}}@layer a{p{font-size:31pt}}",
            23.0,
        ),
        ("@layer{p{font-size:19pt}}@layer{p{font-size:23pt}}", 23.0),
        (
            "@layer{p{font-size:19pt!important}}@layer{p{font-size:23pt!important}}",
            19.0,
        ),
        (
            r"@layer a,A;@layer A{p{font-size:23pt}}@layer \61{p{font-size:19pt}}",
            23.0,
        ),
    ] {
        assert_pt(style(css, "", "<p id=target>Text</p>").size, expected);
    }
    assert_pt(
        style(
            "@layer a{p{font-size:19pt!important}}",
            "",
            "<p id=target style='font-size:31pt!important'>Text</p>",
        )
        .size,
        31.0,
    );
    assert_pt(
        style(
            "@layer a{p{font-size:19pt!important}}",
            "",
            "<p id=target style='font-size:revert-layer!important'>Text</p>",
        )
        .size,
        19.0,
    );
    assert_pt(
        style(
            "@layer a{p{font-size:19pt}}p{font-size:23pt}",
            "",
            "<p id=target style='font-size:revert-layer'>Text</p>",
        )
        .size,
        19.0,
    );
}

#[test]
fn revert_layer_walks_candidates_without_reviving_invalid_values() {
    for (author, user, expected) in [
        (
            "@layer a{p{font-size:19pt}}@layer b{p{font-size:23pt;font-size:revert-layer}}",
            "",
            19.0,
        ),
        (
            "@layer a,b;@layer a{p{font-size:revert-layer!important}}@layer b{p{font-size:23pt!important}}",
            "",
            23.0,
        ),
        (
            "@layer a,b;@layer a{p{font-size:revert-layer!important}}@layer b{p{font-size:23pt}}p{font-size:31pt}",
            "p{font-size:17pt}",
            17.0,
        ),
        (
            "@layer a,b;@layer a{p{font-size:19pt}}@layer b{p{font-size:revert-layer!important}}p{font-size:31pt}",
            "",
            19.0,
        ),
        (
            "p{font-size:31pt!important}",
            "@layer a,b;@layer a{p{font-size:17pt}}@layer b{p{font-size:revert-layer!important}}",
            17.0,
        ),
        (
            "@layer a{p{font-size:19pt}}@layer b{p{font-size:revert-layer}}@layer c{p{font-size:revert-layer}}",
            "",
            19.0,
        ),
        (
            "@layer a{p{font-size:19pt}}@layer b{p{font-size:23pt;font-size:var(--missing)}}p{font-size:revert-layer}",
            "",
            12.0,
        ),
        (
            "@layer a{p{font-size:19pt}}@layer b{p{font-size:var(--missing,revert-layer)}}",
            "",
            19.0,
        ),
    ] {
        assert_pt(style(author, user, "<p id=target>Text</p>").size, expected);
    }
}

#[test]
fn custom_property_layer_rollback_keeps_origin_context() {
    for (author, user, expected) in [
        (
            "@layer a{p{--size:19pt}}@layer b{p{--size:23pt;--size:revert-layer}}p{font-size:var(--size)}",
            "",
            19.0,
        ),
        (
            "@layer a{p{--size:19pt}}@layer b{p{--size:var(--missing,revert-layer);--copy:var(--size)}}p{font-size:var(--copy)}",
            "",
            19.0,
        ),
        (
            "@layer a,b;@layer a{p{--size:revert-layer!important}}@layer b{p{--size:23pt!important}}p{font-size:var(--size)}",
            "",
            23.0,
        ),
        (
            "@layer a{p{--size:31pt}}p{font-size:var(--size)}",
            "@layer a,b;@layer a{p{--size:17pt}}@layer b{p{--size:revert-layer!important}}",
            17.0,
        ),
        (
            "@layer a{p{--size:19pt}}@layer b{p{--size:var(--missing)}}p{font-size:var(--size,29pt)}",
            "",
            29.0,
        ),
    ] {
        assert_pt(style(author, user, "<p id=target>Text</p>").size, expected);
    }
}

#[test]
fn layer_names_compare_decoded_identifiers_without_reparsing_them() {
    for name in [r"initial\ ", r"\69 nitial\ ", r"a\.b", "default", "none"] {
        let css = format!("@layer {name}{{p{{font-size:23pt}}}}");
        assert_pt(style(&css, "", "<p id=target>Text</p>").size, 23.0);
        assert!(sheet(&css, "").take_diagnostics().is_empty(), "{css}");
    }
}

#[test]
fn font_face_collisions_follow_layer_precedence_before_source_order() {
    use ferri_pdf_fonts::{FontBook, FontFamily, FontStyle, FontWeight};
    use ferri_pdf_resources::FileResolver;
    let css = "@layer low,high;@layer high{@font-face{font-family:Layered;src:local('DejaVu Serif')}}@layer low{@font-face{font-family:Layered;src:local('DejaVu Sans')}}";
    let styles = sheet(css, "");
    let candidates = styles.fonts.candidates(
        &[FontFamily::Named("Layered".into())],
        ferri_pdf_fonts::FontSelection::new(FontWeight::NORMAL, FontStyle::Normal),
    );
    let selected = styles
        .fonts
        .resolve(candidates[0], &FileResolver::default())
        .unwrap()
        .unwrap();
    let bundled = FontBook::default();
    let serif = bundled.candidates(
        &[FontFamily::Named("DejaVu Serif".into())],
        ferri_pdf_fonts::FontSelection::new(FontWeight::NORMAL, FontStyle::Normal),
    );
    assert!(
        selected == bundled.face(serif[0]).unwrap().data().unwrap(),
        "the high layer must select the serif face despite its earlier source position"
    );
}

#[test]
fn layer_registration_spans_stylesheets_but_is_scoped_to_each_origin() {
    let source = |text: &str| StylesheetSource {
        text: text.into(),
        base: Default::default(),
    };
    let authors = [
        source("@layer a,b;"),
        source("@layer b{p{font-size:23pt}}@layer a{p{font-size:19pt}}"),
    ];
    let users = [source(
        "@layer b,a;@layer b{p{font-size:17pt}}@layer a{p{font-size:13pt}}",
    )];
    let tree =
        DocumentTree::parse("<!doctype html><p id=target>Text</p>").expect("parse test HTML");
    let target = tree
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    let styles = Stylesheet::parse_with_origins(&authors, &users, &Default::default()).unwrap();
    assert_pt(computed(&styles, target).size, 23.0);
    let authors = [source("@layer a,b;@layer a{p{font-size:revert}}")];
    let styles = Stylesheet::parse_with_origins(&authors, &users, &Default::default()).unwrap();
    assert_pt(computed(&styles, target).size, 13.0);
}

#[test]
fn layer_rollback_is_shared_by_shorthands_pseudos_pages_and_margins() {
    let result = style(
        "@layer low{p{margin:7pt 11pt;padding:3pt}}@layer high{p{margin:19pt;padding:var(--missing);margin:var(--missing,revert-layer)}}",
        "",
        "<p id=target>Text</p>",
    );
    assert_eq!(
        result.margin.map(|value| value.resolve(0.0).unwrap()),
        [7.0, 11.0, 7.0, 11.0]
    );
    assert_eq!(
        result.padding,
        [LengthPercentage::ZERO; 4],
        "an invalid shorthand cannot revive the lower layer"
    );
    let css = "@layer low,high;@layer high{p::before{content:'High';font-size:23pt}@page{size:350pt 450pt;margin:19pt;--size:23pt;@top-center{content:'High';font-size:var(--size)}}}@layer low{p::before{content:'Low';font-size:19pt}@page{size:300pt 400pt;margin:11pt;--size:19pt;@top-center{content:'Low';font-size:var(--size)}}}@layer high{p::before{font-size:revert-layer}@page{size:revert-layer;margin:revert-layer;--size:revert-layer;@top-center{font-size:revert-layer}}}";
    let mut styles = sheet(css, "");
    let tree =
        DocumentTree::parse("<!doctype html><p id=target>Text</p>").expect("parse test HTML");
    let target = tree
        .select(&Selector::parse("#target").unwrap())
        .next()
        .unwrap();
    let root = computed(&styles, tree.root_element());
    styles.resolve_page_geometry(&root).unwrap();
    assert_eq!(styles.page, [300.0, 400.0]);
    assert_eq!(styles.margin, [11.0; 4]);
    let margins = styles.page_margins(&root).unwrap();
    assert_pt(margins[0].style.size, 19.0);
    let (pseudo, _) = styles
        .pseudo(
            target,
            &computed(&styles, target),
            ferri_pdf_style::PseudoElement::Before,
        )
        .unwrap()
        .unwrap();
    assert_pt(pseudo.size, 19.0);
}

#[test]
fn invalid_layer_syntax_recovers_without_registering_a_layer() {
    for prelude in [
        "initial",
        "a.INHERIT",
        "a .b",
        "a. b",
        "a..b",
        "a,b",
        "a,",
        "'quoted'",
    ] {
        let css = format!(
            "@layer {prelude}{{p{{font-size:31pt}}}}@layer high{{p{{font-size:23pt}}}}@layer low,high;@layer low{{p{{font-size:19pt}}}}"
        );
        let mut styles = sheet(&css, "");
        assert!(
            !styles.take_diagnostics().is_empty(),
            "invalid prelude {prelude:?} must be diagnosed"
        );
        assert_pt(style(&css, "", "<p id=target>Text</p>").size, 19.0);
    }
    assert_pt(
        style(
            "@layer a/**/./**/b{p{font-size:23pt}}",
            "",
            "<p id=target>Text</p>",
        )
        .size,
        23.0,
    );
}

#[test]
fn nested_layer_diagnostics_keep_original_line_and_column() {
    for (css, expected) in [
        (
            "@layer a{\n @layer b{\n  p{color:;}\n  @unknown;\n }\n}",
            vec![(3, 5), (4, 11)],
        ),
        ("@layer α{@layer β{p{color:;}}}", vec![(1, 21)]),
    ] {
        let diagnostics = sheet(css, "").take_diagnostics();
        let mut locations = diagnostics
            .iter()
            .map(|entry| (entry.location.line, entry.location.column))
            .collect::<Vec<_>>();
        locations.sort_unstable();
        assert_eq!(locations, expected, "{css}: {diagnostics:?}");
    }
}

#[test]
fn auto_direction_text_input_state_rules_outrank_generic_dir_isolation() {
    use ferri_pdf_style::UnicodeBidi;
    for attributes in ["", "type=text", "type=TEXT", "type=unknown"] {
        let markup = format!("<input id=target dir=AUTO {attributes} value=אב>");
        assert_eq!(
            style("", "", &markup).unicode_bidi,
            UnicodeBidi::Plaintext,
            "the resolved Text state must use the state-dependent plaintext UA rule: {attributes}"
        );
        assert_eq!(
            style("#target{unicode-bidi:normal}", "", &markup).unicode_bidi,
            UnicodeBidi::Normal,
            "the UA state rule must remain below authored CSS"
        );
    }
    for kind in ["checkbox", "hidden", "button", "date"] {
        assert_eq!(
            style("", "", &format!("<input id=target type={kind} dir=auto>")).unicode_bidi,
            UnicodeBidi::Isolate,
            "non-text input states must retain ordinary dir isolation: {kind}"
        );
    }
}
