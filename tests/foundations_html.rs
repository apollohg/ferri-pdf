mod common;

use common::{assert_pt, local_options};
use ferri_pdf::{
    Resource, ResourceBase, ResourceError, ResourceResolver, render_html, render_with_resolver,
};
use ferri_pdf_dom::{
    DocumentTree, HTML_NAMESPACE, MATHML_NAMESPACE, NodeData, QuirksMode, SVG_NAMESPACE, Selector,
    TreeRelation,
};
use std::cell::Cell;

fn select<'a>(tree: &'a DocumentTree, selector: &str) -> ferri_pdf_dom::Element<'a> {
    tree.select(&Selector::parse(selector).unwrap())
        .next()
        .unwrap()
}

fn formatting_element<'a>(tree: &'a DocumentTree, id: &str) -> ferri_pdf_dom::Element<'a> {
    tree.root_element()
        .as_node()
        .inclusive_descendants(TreeRelation::Formatting)
        .filter_map(|node| node.element())
        .find(|element| element.attr("id") == Some(id))
        .expect("the composed formatting tree contains the requested shadow element")
}

#[test]
fn declarative_shadow_slots_preserve_three_relationships() {
    let tree = DocumentTree::parse(
        "<!doctype html><div id=host><template shadowrootmode=open><section id=shadow><slot name=n id=named>Named fallback</slot><slot id=defaultslot>Default fallback</slot></section></template><p id=assigned slot=n>Named</p><p id=default>Default</p><p id=hidden slot=missing>Hidden</p></div>",
    ).expect("parse test HTML");
    let host = select(&tree, "#host");
    let assigned = select(&tree, "#assigned");
    let named = formatting_element(&tree, "named");
    let default_slot = formatting_element(&tree, "defaultslot");
    let shadow = formatting_element(&tree, "shadow");
    assert_eq!(assigned.tree_scope(), host.tree_scope());
    assert_eq!(named.tree_scope(), shadow.tree_scope());
    assert_ne!(assigned.tree_scope(), named.tree_scope());
    assert_eq!(shadow.shadow_host().unwrap().id(), host.id());
    assert_eq!(
        assigned.parent(TreeRelation::Selector).unwrap().id(),
        host.id()
    );
    assert_eq!(
        assigned.parent(TreeRelation::Inheritance).unwrap().id(),
        named.id()
    );
    assert_eq!(
        assigned.parent(TreeRelation::Formatting).unwrap().id(),
        named.id()
    );
    assert_eq!(
        select(&tree, "#default")
            .parent(TreeRelation::Inheritance)
            .unwrap()
            .id(),
        default_slot.id()
    );
    assert!(
        shadow.parent(TreeRelation::Selector).is_none(),
        "the shadow host is not an ordinary selector ancestor"
    );
    assert_eq!(
        shadow.parent(TreeRelation::Inheritance).unwrap().id(),
        host.id()
    );
    assert!(
        tree.select(&Selector::parse("#shadow,#named,#defaultslot").unwrap())
            .next()
            .is_none(),
        "document queries cannot enter a shadow tree"
    );
    assert_eq!(
        host.children(TreeRelation::Selector)
            .map(|element| element.attr("id").unwrap().to_owned())
            .collect::<Vec<_>>(),
        ["assigned", "default", "hidden"],
        "the consumed declarative template is absent from light children"
    );
    assert_eq!(
        host.children(TreeRelation::Formatting)
            .map(|element| element.id())
            .collect::<Vec<_>>(),
        [shadow.id()]
    );
    assert_eq!(
        named
            .children(TreeRelation::Formatting)
            .map(|element| element.id())
            .collect::<Vec<_>>(),
        [assigned.id()],
        "slot projection retains original light-node identity"
    );
}

#[test]
fn slot_fallback_and_hidden_assigned_content_have_correct_presence() {
    for (light, expected) in [
        (
            "<p slot=n>Named</p><p>Default</p><p slot=missing>Undistributed</p>",
            vec!["Before", "Named", "Default", "Duplicate fallback", "After"],
        ),
        (
            "",
            vec![
                "Before",
                "Named fallback",
                "Default fallback",
                "Duplicate fallback",
                "After",
            ],
        ),
        (
            "<p slot=n style='display:none'>Hidden assigned</p>",
            vec!["Before", "Default fallback", "Duplicate fallback", "After"],
        ),
    ] {
        let html = format!(
            "<!doctype html><style>{}p{{margin:0}}</style><div><template shadowrootmode=closed><p>Before</p><slot name=n><p>Named fallback</p></slot><slot><p>Default fallback</p></slot><slot name=n><p>Duplicate fallback</p></slot><p>After</p></template>{light}</div>",
            common::PROFILE
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            expected,
            "closed roots, first matching slots and display:none assignments must preserve composition: {light}"
        );
    }
    let html = format!(
        "<!doctype html><style>{}</style><div><template shadowrootmode=open><slot>Fallback</slot></template> </div>",
        common::PROFILE
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert!(
        trace
            .pages
            .iter()
            .flatten()
            .all(|line| !line.text.contains("Fallback")),
        "even whitespace-only assigned text suppresses slot fallback"
    );
}

#[test]
fn shadow_headings_use_host_ancestry_and_ignore_slot_offsets() {
    let tree = DocumentTree::parse(
        "<!doctype html><section headingoffset=2><div id=host headingoffset=1><template shadowrootmode=open><section headingoffset=1><h1 id=inner>Inner</h1><slot headingoffset=8></slot><div headingreset headingoffset=1><h1 id=reset>Reset</h1></div></section></template><h1 id=slotted>Light</h1></div></section>",
    ).expect("parse test HTML");
    assert_eq!(
        formatting_element(&tree, "inner").heading_level(),
        Some(5),
        "heading offsets cross the shadow root through its host"
    );
    assert_eq!(
        formatting_element(&tree, "reset").heading_level(),
        Some(2),
        "inclusive headingreset stops host ancestry"
    );
    assert_eq!(
        select(&tree, "#slotted").heading_level(),
        Some(4),
        "slot assignment must not replace the light node's HTML heading ancestry"
    );
}

#[test]
fn shadow_root_host_validation_and_ascii_mode_matching() {
    for mode in ["open", "closed", "OpEn", "CLOSED"] {
        let html = format!(
            "<!doctype html><style>{}</style><div><template shadowrootmode={mode}><p>Shadow</p></template><p>Light</p></div>",
            common::PROFILE
        );
        let trace = render_html(&html, &local_options())
            .unwrap()
            .layout
            .unwrap();
        assert_eq!(
            trace
                .pages
                .iter()
                .flatten()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            ["Shadow"],
            "enumerated shadowrootmode is ASCII case insensitive: {mode}"
        );
    }
    for host in ["a", "button", "font-face", "x!invalid-name"] {
        let tree = DocumentTree::parse(&format!("<!doctype html><{host} id=host><template shadowrootmode=open><p>Inert</p></template><p>Light</p></{host}>")).expect("parse test HTML");
        let target = select(&tree, "#host");
        assert!(
            target
                .children(TreeRelation::Selector)
                .any(|child| child.name() == "template"),
            "a disallowed host retains an ordinary template: {host}"
        );
        let text = target
            .as_node()
            .inclusive_descendants(TreeRelation::Formatting)
            .filter_map(|node| node.value().as_text().map(|text| text.to_string()))
            .collect::<String>();
        assert_eq!(
            text, "Light",
            "a disallowed host cannot activate template content: {host}"
        );
    }
    let html = format!(
        "<!doctype html><style>{}</style><div><template shadowrootmode=open><p>First</p></template><template shadowrootmode=closed><p>Second</p></template><p>Light</p></div>",
        common::PROFILE
    );
    let trace = render_html(&html, &local_options())
        .unwrap()
        .layout
        .unwrap();
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["First"],
        "a duplicate root stays an ordinary inert template"
    );
}

#[test]
fn formatting_depth_limit_covers_block_inline_atomic_and_positioned_trees() {
    use ferri_pdf_document::{BlockContent, DocumentError, StyledDocument};
    const FORMATTING_LIMIT: usize = 512;
    const DOCUMENT_AND_PARAGRAPH_LEVELS: usize = 2;
    for (opening, closing) in [
        ("<div>", "</div>"),
        ("<span>", "</span>"),
        ("<div style='display:inline-block'>", "</div>"),
        ("<div style='position:absolute'>", "</div>"),
    ] {
        for (levels, allowed) in [
            (FORMATTING_LIMIT - DOCUMENT_AND_PARAGRAPH_LEVELS, true),
            (FORMATTING_LIMIT - DOCUMENT_AND_PARAGRAPH_LEVELS + 1, false),
        ] {
            eprintln!("checking formatting boundary: {levels} levels of {opening}");
            let html = format!(
                "<!doctype html>{}<p>Boundary</p>{}",
                opening.repeat(levels),
                closing.repeat(levels),
            );
            let result = StyledDocument::parse(
                &html,
                &ResourceBase::default(),
                &[],
                &ferri_pdf::FileResolver::default(),
            );
            if allowed {
                let document = result.expect("the inclusive formatting limit remains usable");
                let mut pending = vec![document.root()];
                let mut text = String::new();
                while let Some(block) = pending.pop() {
                    for content in block.children() {
                        match content {
                            BlockContent::Events(_) => {}
                            BlockContent::Block(child) | BlockContent::Positioned(child) => {
                                pending.push(child)
                            }
                            BlockContent::Inline(paragraph) => {
                                text.extend(paragraph.text().chars().filter(|ch| {
                                    *ch != ferri_pdf_document::OBJECT_REPLACEMENT_CHARACTER
                                }));
                                pending.extend(
                                    paragraph
                                        .atomics()
                                        .iter()
                                        .chain(paragraph.anchors())
                                        .map(|anchor| &anchor.block),
                                );
                            }
                        }
                    }
                }
                assert_eq!(
                    text, "Boundary",
                    "formatting {levels} levels of {opening} preserves content"
                );
            } else {
                assert!(
                    matches!(result, Err(DocumentError::Formatting(message)) if message == format!("formatting nesting exceeds {FORMATTING_LIMIT}")),
                    "one further {opening} level must return the formatting error without overflowing"
                );
            }
        }
    }
}

#[test]
fn excessive_shadow_nesting_returns_an_html_tree_error() {
    const TOO_DEEP: usize = 65;
    let html = format!(
        "<!doctype html>{}<p>Deep</p>{}",
        "<div><template shadowrootmode=open>".repeat(TOO_DEEP),
        "</template></div>".repeat(TOO_DEEP)
    );
    for error in [
        render_html(&html, &local_options()).err(),
        ferri_pdf::render_bytes(html.as_bytes(), &local_options()).err(),
    ] {
        assert!(
            matches!(
                error,
                Some(ferri_pdf::Error::Html(
                    ferri_pdf_document::DocumentError::Tree(ferri_pdf_dom::DomError::TreeLimit {
                        resource: "shadow tree nesting",
                        limit: 64
                    })
                ))
            ),
            "both HTML input paths must preserve the typed shadow-depth failure"
        );
    }
    let html = format!(
        "<!doctype html>{}<p>Deep</p>{}",
        "<div><template shadowrootmode=open>".repeat(TOO_DEEP - 1),
        "</template></div>".repeat(TOO_DEEP - 1)
    );
    DocumentTree::parse(&html).expect("parse the inclusive shadow bound");
    let trace = render_html(&html, &local_options())
        .expect("the inclusive shadow bound remains usable")
        .layout
        .unwrap();
    assert_eq!(
        trace
            .pages
            .iter()
            .flatten()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["Deep"]
    );
}

#[test]
fn html_tree_recovery_preserves_namespaces_and_quirks() {
    let tree = DocumentTree::parse(
        "<!doctype html><table id=t>outside<tr><td id=cell>A<td>B</table><p id=duplicate CLASS=first class=second>C</p><svg><circle id=circle /><foreignObject><p id=foreign>HTML</p></foreignObject></svg><math><mi id=mi>x</mi><annotation-xml encoding=text/html><div id=annotation>HTML</div></annotation-xml></math>",
    ).expect("parse test HTML");
    assert_eq!(tree.quirks_mode(), QuirksMode::NoQuirks);
    assert_eq!(
        DocumentTree::parse("<p>quirks")
            .expect("parse test HTML")
            .quirks_mode(),
        QuirksMode::Quirks
    );
    let cell = select(&tree, "#cell");
    let row = cell.parent(TreeRelation::Selector).unwrap();
    assert_eq!(row.name(), "tr");
    let group = row.parent(TreeRelation::Selector).unwrap();
    assert_eq!(group.name(), "tbody");
    assert_eq!(
        group.parent(TreeRelation::Selector).unwrap().id(),
        select(&tree, "#t").id()
    );
    let body = select(&tree, "body");
    assert_eq!(
        body.child_nodes(TreeRelation::Selector)
            .find_map(|node| node.value().as_text().map(|text| text.as_ref())),
        Some("outside")
    );
    assert_eq!(select(&tree, "#duplicate").attr("class"), Some("first"));
    assert_eq!(select(&tree, "#circle").namespace(), SVG_NAMESPACE);
    assert_eq!(select(&tree, "#foreign").namespace(), HTML_NAMESPACE);
    assert_eq!(select(&tree, "#mi").namespace(), MATHML_NAMESPACE);
    assert_eq!(select(&tree, "#annotation").namespace(), HTML_NAMESPACE);
    let malformed = DocumentTree::parse("<b><i>A</b>B</i>").expect("parse test HTML");
    assert_eq!(
        select(&malformed, "body > b > i")
            .text()
            .collect::<String>(),
        "A"
    );
    assert_eq!(
        select(&malformed, "body > i").text().collect::<String>(),
        "B"
    );
}

#[test]
fn scripting_disabled_noscript_and_template_parse_correctly() {
    let tree = DocumentTree::parse(
        "<head><noscript><style id=head>p{font-size:18pt}</style></noscript></head><body><noscript><p id=fallback>fallback</p></noscript><template id=t><style id=inert>p{display:none}</style><p id=inside>inert</p></template><p id=live>live</p>",
    ).expect("parse test HTML");
    assert_eq!(
        select(&tree, "#fallback").text().collect::<String>(),
        "fallback"
    );
    assert!(
        tree.select(&Selector::parse("#head").unwrap())
            .next()
            .is_some()
    );
    assert!(
        tree.select(&Selector::parse("#inside, #inert").unwrap())
            .next()
            .is_none(),
        "inert fragment nodes are outside document queries"
    );
    let template = select(&tree, "#t");
    for relation in [
        TreeRelation::Selector,
        TreeRelation::Inheritance,
        TreeRelation::Formatting,
    ] {
        assert_eq!(
            template.children(relation).count(),
            0,
            "template contents are not template children"
        );
    }
    let contents = template.template_contents().unwrap();
    assert!(matches!(contents.value(), NodeData::Fragment));
    assert!(contents.parent(TreeRelation::Selector).is_none());
    let inside = contents
        .children(TreeRelation::Selector)
        .filter_map(|node| node.element())
        .find(|element| element.attr("id") == Some("inside"))
        .unwrap();
    assert!(inside.parent(TreeRelation::Selector).is_none());
    assert!(
        !inside.matches(&Selector::parse("#t p").unwrap()),
        "template is not a selector ancestor of its contents"
    );
}

#[test]
fn ordinary_tree_relations_preserve_existing_rendering() {
    let tree =
        DocumentTree::parse("<section id=parent><p id=child>A<em id=em>B</em>C</p></section>")
            .expect("parse test HTML");
    let child = select(&tree, "#child");
    let parent = select(&tree, "#parent");
    for relation in [
        TreeRelation::Selector,
        TreeRelation::Inheritance,
        TreeRelation::Formatting,
    ] {
        assert_eq!(child.parent(relation).unwrap().id(), parent.id());
        assert_eq!(
            child
                .children(relation)
                .map(|element| element.name())
                .collect::<Vec<_>>(),
            ["em"]
        );
    }
    let output = render_html(&format!("<style>{}p{{font-size:18pt;line-height:22pt;margin:0}}</style><section><p>A<em>B</em>C</p><p>D</p></section>", common::PROFILE), &local_options()).unwrap();
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["ABC", "D"]
    );
    assert_pt(trace.pages[0][0].font_size, 18.0);
    assert_pt(
        trace.pages[0][1].baseline - trace.pages[0][0].baseline,
        22.0,
    );
}

#[test]
fn template_styles_and_resources_remain_inert_even_with_display_override() {
    struct DenyAll(Cell<usize>);
    impl ResourceResolver for DenyAll {
        fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
            self.0.set(self.0.get() + 1);
            Err(ResourceError::Denied(base.resolve(reference)?))
        }
    }
    let resolver = DenyAll(Cell::new(0));
    let output = render_with_resolver("<style>template{display:block}p{font-size:18pt}</style><template><link rel=stylesheet href=forbidden.css><style>p{display:none}</style><p>hidden</p></template><noscript><p>fallback</p></noscript><p>visible</p>", &local_options(), &resolver).unwrap();
    let trace = output.layout.unwrap();
    assert_eq!(
        trace.pages[0]
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["fallback", "visible"]
    );
    assert_eq!(
        resolver.0.get(),
        0,
        "inert sources must never reach resolver policy"
    );
    assert!(output.diagnostics.is_empty());
}

#[test]
fn quirks_mode_reaches_selector_matching_in_rendering() {
    for (doctype, expected) in [("", 18.0), ("<!doctype html>", 10.0)] {
        let output = render_html(&format!("{doctype}<style>p{{font-size:10pt}}.foo{{font-size:18pt}}</style><p class=FOO>case</p>"), &local_options()).unwrap();
        assert_pt(output.layout.unwrap().pages[0][0].font_size, expected);
    }
}

#[test]
fn language_and_direction_use_dom_parents_while_css_inherits_through_slots() {
    let tree=DocumentTree::parse("<!doctype html><div id=host lang=en dir=ltr><template shadowrootmode=open><section lang=fr dir=rtl><slot></slot></section></template><span id=assigned>ABC</span></div>").unwrap();
    let assigned = select(&tree, "#assigned");
    assert_eq!(
        assigned.language(),
        "en",
        "HTML language must not come from the assigned slot's CSS inheritance parent"
    );
    assert_eq!(
        assigned.directionality(),
        ferri_pdf_dom::Directionality::Ltr,
        "HTML directionality must retain the light-tree parent even though CSS direction inherits through the slot"
    );
    assert_eq!(
        assigned.parent(TreeRelation::Inheritance).unwrap().name(),
        "slot"
    );
}

#[test]
fn automatic_direction_scans_dom_text_and_uses_shadow_slot_host_direction() {
    let host=DocumentTree::parse("<!doctype html><div id=host dir=auto><template shadowrootmode=open><p>Latin shadow text</p><slot></slot></template><span>אב</span></div>").unwrap();
    assert_eq!(
        select(&host, "#host").directionality(),
        ferri_pdf_dom::Directionality::Rtl,
        "shadow text must not replace the host's own descendant text in dir=auto discovery"
    );
    let fallback=DocumentTree::parse("<!doctype html><div dir=rtl><template shadowrootmode=open><p id=automatic dir=auto><slot>Latin fallback</slot></p></template></div>").unwrap();
    assert_eq!(
        formatting_element(&fallback, "automatic").directionality(),
        ferri_pdf_dom::Directionality::Rtl,
        "a descendant slot in a shadow tree contributes its host direction before scanning fallback text"
    );
    let assigned=DocumentTree::parse("<!doctype html><div><template shadowrootmode=open><p><slot id=automatic dir=auto></slot></p></template><span dir=ltr>Latin isolated</span><span>אב</span></div>").unwrap();
    assert_eq!(
        formatting_element(&assigned, "automatic").directionality(),
        ferri_pdf_dom::Directionality::Rtl,
        "slot auto direction must use assigned nodes and exclude a directionally isolated assigned root"
    );
}
