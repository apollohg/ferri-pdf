use super::*;
use crate::dom::Selector;
use crate::resources::FileResolver;
use crate::style::StylesheetSource;

/// The XML of the document's first `<svg>`, styled by `css` as the document's style sheet.
fn styled(css: &str, html: &str) -> Result<String, DocumentError> {
    let tree = DocumentTree::parse(html).expect("parse test HTML");
    let sheet = Stylesheet::parse(&[StylesheetSource {
        text: css.into(),
        base: Default::default(),
    }])?;
    let resolver = FileResolver::default();
    let styling = Styling {
        sheet: &sheet,
        resolver: &resolver,
    };
    let root = sheet.computed_with_resolver(tree.root_element(), &Style::default(), &resolver)?;
    let svg = tree
        .select(&Selector::parse("svg").unwrap())
        .next()
        .expect("an svg element");
    let style = computed_in_place(svg, &root, &styling)?;
    serialize(svg, &style, &styling)
}

fn xml(css: &str, html: &str) -> String {
    styled(css, html).unwrap_or_else(|error| panic!("`{css}` styles `{html}`: {error}"))
}

/// The written attributes of the element with `id`, its `style` declarations split out.
fn element(xml: &str, id: &str) -> HashMap<String, String> {
    let document = roxmltree::Document::parse(xml)
        .unwrap_or_else(|error| panic!("serialized SVG must be well-formed XML: {error}\n{xml}"));
    let node = document
        .descendants()
        .find(|node| node.attribute(ID_ATTRIBUTE) == Some(id))
        .unwrap_or_else(|| panic!("no element #{id} in\n{xml}"));
    let mut attributes: HashMap<String, String> = node
        .attributes()
        .map(|attribute| (attribute.name().to_owned(), attribute.value().to_owned()))
        .collect();
    for declaration in attributes
        .remove(STYLE_ATTRIBUTE)
        .unwrap_or_default()
        .split(';')
        .filter(|declaration| !declaration.is_empty())
    {
        let (name, value) = declaration.split_once(':').expect("name: value");
        attributes.insert(name.to_owned(), value.to_owned());
    }
    attributes
}

fn value(xml: &str, id: &str, name: &str) -> String {
    element(xml, id)
        .remove(name)
        .unwrap_or_else(|| panic!("#{id} writes no {name} in\n{xml}"))
}

#[test]
fn legacy_text_properties_share_svg_inheritance_variables_and_author_overrides() {
    let serialized = xml(
        "svg{--legacy:90;text-orientation:upright}.rule{text-orientation:sideways}\
         #variable{glyph-orientation-vertical:var(--legacy)}\
         #initial{glyph-orientation-vertical:initial}\
         #inherit{glyph-orientation-vertical:inherit}\
         #revert{writing-mode:horizontal-tb;writing-mode:revert}",
        "<svg><text id=attribute writing-mode=tb glyph-orientation-vertical=0>A</text>\
         <text id=variable>B</text><text id=initial>C</text><text id=inherit>D</text>\
         <text id=rule class=rule glyph-orientation-vertical=0>E</text>\
         <text id=revert writing-mode=tb>F</text></svg>",
    );
    for (id, property, expected) in [
        ("attribute", "writing-mode", "vertical-rl"),
        ("attribute", "text-orientation", "upright"),
        ("variable", "text-orientation", "sideways"),
        ("initial", "text-orientation", "mixed"),
        ("inherit", "text-orientation", "upright"),
        ("rule", "text-orientation", "sideways"),
        ("revert", "writing-mode", "vertical-rl"),
    ] {
        assert_eq!(
            value(&serialized, id, property),
            expected,
            "#{id}/{property}"
        );
    }
}

#[test]
fn html_parsed_svg_becomes_namespaced_xml_without_undeclared_prefixes() {
    let xml = xml(
        "",
        "<p><svg viewBox='0 0 10 10' xmlns:sodipodi='x'><sodipodi:namedview/><linearGradient id=g><stop offset=0 /></linearGradient><text>A &amp; B &lt;</text><!--note--></svg></p>",
    );
    let document = roxmltree::Document::parse(&xml).expect("well-formed");
    let root = document.root_element();
    assert_eq!(root.tag_name().namespace(), Some(SVG_NAMESPACE), "{xml}");
    assert_eq!(
        root.attribute("viewBox"),
        Some("0 0 10 10"),
        "case kept: {xml}"
    );
    assert!(
        !xml.contains("sodipodi"),
        "editor-only prefixes are dropped: {xml}"
    );
    assert!(xml.contains("<text"), "{xml}");
    assert!(
        xml.contains(">A &amp; B &lt;</text>"),
        "text is escaped: {xml}"
    );
    assert!(!xml.contains("note"), "comments are dropped: {xml}");
}

#[test]
fn presentation_attributes_rank_below_author_rules_and_above_inheritance() {
    let xml = xml(
        ".rule{fill:rgb(255,0,0)} #important{fill:rgb(0,128,0)!important}",
        "<div style='fill:rgb(0,0,255)'><svg>\
         <rect id=attribute fill='rgb(255,255,0)'/>\
         <rect id='inherited'/>\
         <rect id=rule class=rule fill='rgb(255,255,0)'/>\
         <rect id=inline class=rule style='fill:rgb(1,2,3)'/>\
         <rect id=important class=rule style='fill:rgb(1,2,3)'/>\
         </svg></div>",
    );
    for (id, expected, why) in [
        (
            "attribute",
            "rgba(255,255,0,1)",
            "a presentation attribute beats inheritance",
        ),
        (
            "inherited",
            "rgba(0,0,255,1)",
            "fill inherits from the HTML ancestor",
        ),
        (
            "rule",
            "rgba(255,0,0,1)",
            "a class rule beats the zero-specificity attribute",
        ),
        (
            "inline",
            "rgba(1,2,3,1)",
            "the style attribute beats the rule",
        ),
        (
            "important",
            "rgba(0,128,0,1)",
            "!important beats the style attribute",
        ),
    ] {
        assert_eq!(value(&xml, id, "fill"), expected, "#{id}: {why}\n{xml}");
    }
}

#[test]
fn an_invalid_presentation_attribute_specifies_the_initial_value() {
    let xml = xml(
        "",
        "<svg><g fill='rgb(0,0,255)' stroke-width='3'><rect id=invalid fill='nonsense' stroke-width='-2'/><rect id='valid'/></g></svg>",
    );
    assert_eq!(
        value(&xml, "invalid", "fill"),
        "rgba(0,0,0,1)",
        "SVG 2 §4.2: an invalid fill is the initial black, not the inherited blue\n{xml}"
    );
    assert_eq!(
        value(&xml, "invalid", "stroke-width"),
        "1px",
        "negative stroke-width is invalid\n{xml}"
    );
    assert_eq!(value(&xml, "valid", "fill"), "rgba(0,0,255,1)", "{xml}");
    assert_eq!(
        value(&xml, "valid", "stroke-width"),
        "3px",
        "unitless attribute lengths are px\n{xml}"
    );
}

#[test]
fn inheritance_crosses_from_html_into_svg_and_currentcolor_follows_each_element() {
    let xml = xml(
        "div{color:rgb(1,2,3);font:italic 700 15pt 'Liberation Sans'}",
        "<div><svg><g color='rgb(4,5,6)'><path id=group fill=currentColor stroke='currentColor'/></g>\
         <path id=root fill='currentColor'/><text id=label>Label</text></svg></div>",
    );
    assert_eq!(
        value(&xml, "group", "fill"),
        "rgba(4,5,6,1)",
        "the color attribute of the group\n{xml}"
    );
    assert_eq!(value(&xml, "group", "stroke"), "rgba(4,5,6,1)", "{xml}");
    assert_eq!(
        value(&xml, "root", "fill"),
        "rgba(1,2,3,1)",
        "color inherited from the HTML div\n{xml}"
    );
    let text = element(&xml, "label");
    assert_eq!(text["font-size"], "20px", "15pt is 20 CSS px\n{xml}");
    assert_eq!(text["font-family"], "\"Liberation Sans\"", "{xml}");
    assert_eq!(text["font-weight"], "700", "{xml}");
    assert_eq!(text["font-style"], "italic", "{xml}");
    let light = styled(
        "p{font-weight:350}",
        "<p><svg><text id='light'>Light</text></svg></p>",
    )
    .expect("a whole weight is supported");
    assert_eq!(
        value(&light, "light", "font-weight"),
        "350",
        "CSS Fonts 4 §2.2: any whole weight reaches font matching\n{light}"
    );
    for direction in ["left", "right"] {
        let slanted = styled(
            &format!("p{{font-style:{direction}}}"),
            "<p><svg><text id='slanted'>Slanted</text></svg></p>",
        )
        .unwrap_or_else(|error| panic!("font-style:{direction} must reach SVG text: {error}"));
        assert_eq!(
            value(&slanted, "slanted", "font-style"),
            direction,
            "CSS Fonts 4 §2.4: the directional italic reaches SVG font matching\n{slanted}"
        );
    }
}

#[test]
fn document_rules_reach_svg_through_variables_media_layers_and_nesting() {
    let xml = xml(
        ":root{--accent:rgb(9,8,7)}\
         @layer base{rect{fill:rgb(255,0,0)}}\
         #unlayered{fill:rgb(0,255,0)}\
         @media print{#printed{stroke:var(--accent)}}\
         @media screen{#printed{stroke:rgb(255,0,0)}}\
         svg{& .nested{stroke-width:calc(2px + 1em)}}",
        "<div style='font-size:12px'><svg><rect id='layered'/><rect id='unlayered'/>\
         <rect id=printed class='nested'/></svg></div>",
    );
    assert_eq!(value(&xml, "layered", "fill"), "rgba(255,0,0,1)", "{xml}");
    assert_eq!(
        value(&xml, "unlayered", "fill"),
        "rgba(0,255,0,1)",
        "unlayered beats layered\n{xml}"
    );
    assert_eq!(
        value(&xml, "printed", "stroke"),
        "rgba(9,8,7,1)",
        "print media and var()\n{xml}"
    );
    assert_eq!(
        value(&xml, "printed", "stroke-width"),
        "14px",
        "nested rule, calc and em\n{xml}"
    );
}

#[test]
fn geometry_properties_override_their_attributes_on_designated_elements_only() {
    let xml = xml(
        "#wide{width:30px} #round{r:5px;cx:50%} #path{d:'M0 0L5 5'} #negative{r:-4px;rx:-1px}",
        "<svg width=100 height=50><rect id=wide x=1 y='2mm' width=10 height='20'/>\
         <circle id=round cx=1 cy=2 r='9'/><path id=path d='M1 1L2 2'/>\
         <ellipse id=negative rx=3 ry='4'/><text id=text x=7>T</text></svg>",
    );
    let wide = element(&xml, "wide");
    assert_eq!(
        wide["width"], "30px",
        "CSS width beats the attribute\n{xml}"
    );
    assert_eq!(wide["x"], "1px", "{xml}");
    assert_eq!(wide["height"], "20px", "{xml}");
    let round = element(&xml, "round");
    assert_eq!(
        (
            round["r"].as_str(),
            round["cx"].as_str(),
            round["cy"].as_str()
        ),
        ("5px", "50%", "2px"),
        "{xml}"
    );
    assert_eq!(value(&xml, "path", "d"), "M0 0L5 5", "{xml}");
    let negative = element(&xml, "negative");
    assert!(
        !negative.contains_key("rx"),
        "SVG 2 §7: a negative rx is auto, written as absent\n{xml}"
    );
    assert_eq!(negative["ry"], "4px", "{xml}");
    assert_eq!(
        value(&xml, "text", "x"),
        "7",
        "text x is not a presentation attribute\n{xml}"
    );
    let document = roxmltree::Document::parse(&xml).unwrap();
    assert_eq!(
        document.root_element().attribute("width"),
        Some("100"),
        "the root's geometry sizes its box and stays as written\n{xml}"
    );
}

#[test]
fn transforms_overflow_masks_clips_and_direction_reach_the_renderer_as_computed() {
    let xml = xml(
        "#scaled{transform:scale(2,3);transform-origin:center;transform-box:fill-box} \
         #shown{overflow:visible} #round{clip-path:circle(50% at right 10% top 5px) view-box} \
         #corner{clip-path:polygon(evenodd,0 0,100% 0,0 100%)} #masked{mask:url(#m)} \
         #referenced{clip-path:url(#c)} #inset{clip-path:inset(1px 2px 3px) fill-box} \
         #even{clip-path:inset(4% round 2px 3px / 1px)} #box{clip-path:xywh(1px 2% 3px 4%)} \
         #ellipse{clip-path:ellipse(farthest-corner at 10px 20%) content-box} \
         #hebrew{direction:rtl}",
        "<svg width=100 height=50><rect id=scaled /><rect id=moved transform='translate(5 6) scale(2)'/>\
         <svg id=shown /><svg id=hidden /><rect id=round /><rect id=corner /><rect id=masked />\
         <rect id=referenced /><rect id=inset /><rect id=even /><rect id=box /><rect id=ellipse />\
         <rect id=plain /><text id=hebrew>T</text></svg>",
    );
    let scaled = element(&xml, "scaled");
    assert_eq!(
        (
            scaled["transform"].as_str(),
            scaled["transform-origin"].as_str(),
            scaled["transform-box"].as_str()
        ),
        ("matrix(2 0 0 3 0 0)", "50% 50%", "fill-box"),
        "CSS Transforms 1 §6: the renderer places the origin in the fill box
{xml}"
    );
    let moved = element(&xml, "moved");
    assert_eq!(
        (
            moved["transform"].as_str(),
            moved["transform-origin"].as_str()
        ),
        ("matrix(2 0 0 2 5 6)", "0px 0px"),
        "SVG 2 §6.8: the attribute is a presentation hint about the user agent's 0 0 origin
{xml}"
    );
    assert!(
        !moved.contains_key("transform-box"),
        "view-box is the renderer's own reference
{xml}"
    );
    assert_eq!(value(&xml, "shown", "overflow"), "visible", "{xml}");
    assert_eq!(
        value(&xml, "hidden", "overflow"),
        "hidden",
        "SVG 2 §6.8: nested svg elements clip by default
{xml}"
    );
    assert_eq!(
        value(&xml, "round", "clip-path"),
        "circle(50% at right 10% top 5px) view-box",
        "{xml}"
    );
    assert_eq!(
        value(&xml, "corner", "clip-path"),
        "polygon(evenodd, 0px 0px, 100% 0px, 0px 100%) stroke-box",
        "CSS Masking 1 §6.1: border-box, the stroke box on SVG content, is the default
{xml}"
    );
    assert_eq!(
        value(&xml, "inset", "clip-path"),
        "inset(1px 2px 3px 2px) fill-box",
        "CSS Shapes 1 §3.1: a missing left side copies the right\n{xml}"
    );
    assert_eq!(
        value(&xml, "even", "clip-path"),
        "inset(4% 4% 4% 4% round 2px 3px 2px 3px / 1px 1px 1px 1px) stroke-box",
        "one inset serves every side, and the corners expand like border-radius\n{xml}"
    );
    assert_eq!(
        value(&xml, "box", "clip-path"),
        "inset(2% calc(100% + -4px) 94% 1px) stroke-box",
        "CSS Shapes 1 §3.2: xywh() computes to an inset whose far sides are sums\n{xml}"
    );
    assert_eq!(
        value(&xml, "ellipse", "clip-path"),
        "ellipse(farthest-corner at left 10px top 20%) fill-box",
        "content-box is SVG content's fill box\n{xml}"
    );
    assert_eq!(
        value(&xml, "referenced", "clip-path"),
        "url(\"#c\")",
        "{xml}"
    );
    assert_eq!(value(&xml, "masked", "mask"), "url(\"#m\")", "{xml}");
    let plain = element(&xml, "plain");
    assert_eq!(
        (
            plain["clip-path"].as_str(),
            plain["mask"].as_str(),
            plain.get("transform").map(String::as_str)
        ),
        ("none", "none", None),
        "{xml}"
    );
    assert_eq!(value(&xml, "hebrew", "direction"), "rtl", "{xml}");
}

#[test]
fn use_instances_match_their_own_tree_and_inherit_from_the_use_element() {
    // SVG 2 §5.5.3 example "Use-changed-styles".
    let xml = xml(
        "circle{stroke-opacity:0.7} .special circle{stroke:rgb(0,128,0)} use{stroke:rgb(128,0,128);fill:rgb(255,165,0)}",
        "<svg><g class=special style='fill:rgb(0,0,255)'><circle id=c cy=50 cx=50 r=40 stroke-width='20'/></g>\
         <use href='#c' x=100 /></svg>",
    );
    let original = element(&xml, "c");
    assert_eq!(
        original["fill"], "rgba(0,0,255,1)",
        "the original inherits the group's fill\n{xml}"
    );
    assert_eq!(original["stroke"], "rgba(0,128,0,1)", "{xml}");
    let instance = element(&xml, &format!("{INSTANCE_ID_PREFIX}0"));
    assert_eq!(
        instance["fill"], "rgba(255,165,0,1)",
        "the instance inherits from use\n{xml}"
    );
    assert_eq!(
        instance["stroke"], "rgba(128,0,128,1)",
        "the complex selector no longer matches the instance\n{xml}"
    );
    assert_eq!(
        instance["stroke-opacity"], "0.7",
        "the simple selector still matches\n{xml}"
    );
    assert_eq!(
        instance["stroke-width"], "20px",
        "the presentation attribute is cloned\n{xml}"
    );
    assert!(
        xml.contains(&format!("href=\"#{INSTANCE_ID_PREFIX}0\"")),
        "the use element references its styled instance\n{xml}"
    );
}

#[test]
fn sprites_in_other_svg_elements_and_nested_uses_get_instances_and_cycles_render_nothing() {
    let html = "<svg style='display:none'><symbol id=icon><circle id=dot r=4 fill='currentColor'/>\
                <use id=inner href='#square'/></symbol><rect id=square width=2 height='2'/>\
                <g id=loop><use id=back href='#loop'/></g></svg>\
                <p style='color:rgb(10,20,30)'><svg><use id=first href='#icon'/><use id=cycle href='#loop'/>\
                <use id=missing href='#nowhere'/></svg></p>";
    let tree = DocumentTree::parse(html).expect("parse");
    let selector = Selector::parse("svg").unwrap();
    let second = tree.select(&selector).nth(1).unwrap();
    let sheet = Stylesheet::parse(&[]).unwrap();
    let resolver = FileResolver::default();
    let styling = Styling {
        sheet: &sheet,
        resolver: &resolver,
    };
    let root = sheet
        .computed_with_resolver(tree.root_element(), &Style::default(), &resolver)
        .unwrap();
    let style = computed_in_place(second, &root, &styling).unwrap();
    let xml = serialize(second, &style, &styling).unwrap();
    assert_eq!(
        value(&xml, "dot", "fill"),
        "rgba(10,20,30,1)",
        "the sprite's clone inherits the color around its use element, not the hidden sprite's\n{xml}"
    );
    let document = roxmltree::Document::parse(&xml).unwrap();
    let inner = element(&xml, "inner")[HREF_ATTRIBUTE].clone();
    let nested = document
        .descendants()
        .find(|node| Some(node.attribute(ID_ATTRIBUTE)) == Some(inner.strip_prefix('#')))
        .unwrap_or_else(|| panic!("the nested use references a written instance {inner}\n{xml}"));
    assert_eq!(
        (nested.tag_name().name(), nested.attribute("width")),
        ("rect", Some("2px")),
        "the nested use gets its own instance of the square\n{xml}"
    );
    let uses: Vec<_> = document
        .descendants()
        .filter(|node| node.has_tag_name("use"))
        .filter_map(|node| node.attribute(ID_ATTRIBUTE))
        .collect();
    assert_eq!(
        uses,
        ["first", "cycle", "inner"],
        "SVG 2 §5.5.1: the clone of #back, whose instance would contain itself, and the \
         unresolved #missing render nothing; #cycle renders the group around it\n{xml}"
    );
}

#[test]
fn svg_style_elements_are_left_to_the_document_cascade() {
    let xml = xml("", "<svg><style>rect{fill:red}</style><rect id='r'/></svg>");
    assert!(
        !xml.contains("<style") && !xml.contains("rect{"),
        "SVG 2 §6.4: the document cascade applies the sheet; the renderer must not apply it again\n{xml}"
    );
}

#[test]
fn properties_the_svg_renderer_cannot_take_from_the_cascade_are_unsupported() {
    for (css, expected) in [
        ("pattern{overflow:visible}", "always clips"),
        ("rect{mask:url(#a),url(#b)}", "mask layers"),
        (
            "text{font-variant-caps:all-small-caps}",
            "font-variant-caps",
        ),
        (
            "rect{color-interpolation:linearRGB;opacity:0.5}",
            "linearRGB compositing",
        ),
        (
            "rect{mix-blend-mode:plus-lighter}",
            "additive mix-blend-mode",
        ),
        ("rect{cx:calc(10% + 1px)}", ""),
    ] {
        match styled(
            css,
            "<svg><rect id=r /><text>T</text><use href='#r'/><svg/><pattern/></svg>",
        ) {
            Err(DocumentError::Style(crate::style::StyleError::Unsupported(message))) => {
                assert!(
                    message.contains(expected),
                    "`{css}` reports {message:?}, expected {expected:?}"
                );
            }
            Ok(xml) if expected.is_empty() => {
                assert!(
                    !xml.contains("calc"),
                    "`{css}` applies only to circles and ellipses: {xml}"
                );
            }
            other => panic!("`{css}` must be unsupported on SVG content, got {other:?}"),
        }
    }
}

#[test]
fn referenced_filters_preserve_their_primitives_and_target_reference() {
    let serialized = xml(
        "",
        r#"<svg><defs><filter id="f"><feOffset dx="5" dy="2"/></filter></defs><rect id="target" width="20" height="10" filter="url(#f)"/></svg>"#,
    );
    assert_eq!(
        value(&serialized, "target", "filter"),
        "url(\"#f\")",
        "{serialized}"
    );
    let document = roxmltree::Document::parse(&serialized).unwrap();
    let primitive = document
        .descendants()
        .find(|node| node.has_tag_name("feOffset"))
        .expect("filter primitive");
    assert_eq!(primitive.attribute("dx"), Some("5"));
    assert_eq!(primitive.attribute("dy"), Some("2"));
}

#[test]
fn paint_server_references_accept_absent_and_explicit_fallbacks() {
    for property in ["fill", "stroke"] {
        for fallback in ["", " none", " rgba(255,0,0,1)"] {
            let paint = format!("url(\"#paint\"){fallback}");
            let css = format!("rect{{{property}:{paint}}}");
            let serialized = xml(&css, "<svg><rect id=target width=10 height=10 /></svg>");
            assert_eq!(
                value(&serialized, "target", property),
                paint,
                "{css}\n{serialized}"
            );
        }
    }
}
