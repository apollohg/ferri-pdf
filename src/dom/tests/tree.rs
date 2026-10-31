use crate::dom::{DocumentTree, NodeData, Selector, TreeRelation};

#[test]
fn initial_semantics_cover_light_shadow_and_inert_template_elements() {
    use crate::dom::{Directionality, InputType};
    let tree = DocumentTree::parse(
        "<!doctype html><meta http-equiv=content-language content=fr><div id=host lang=tr dir=RTL><template shadowrootmode=open><section id=shadow lang=de dir=ltr><slot id=slot dir=auto></slot></section></template><span id=light>אב</span></div><input id=input type=unknown dir=auto value='אב'><template lang=en><p id=inert lang=ja><bdi id=inertbdi>אב</bdi></p></template>",
    );
    let elements: Vec<_> = tree.shadow_including_elements().collect();
    for (id, language, direction) in [
        ("host", "tr", Directionality::Rtl),
        ("shadow", "de", Directionality::Ltr),
        ("slot", "de", Directionality::Rtl),
        ("light", "tr", Directionality::Rtl),
        ("input", "fr", Directionality::Rtl),
    ] {
        let element = *elements
            .iter()
            .find(|element| element.attr("id") == Some(id))
            .unwrap();
        let state = element.initial_state();
        assert_eq!(
            state.language.as_ref(),
            language,
            "initial language of {id}"
        );
        assert_eq!(state.direction, direction, "initial direction of {id}");
        assert!(
            element.matches(
                &Selector::parse(&format!(
                    ":lang({language}):dir({})",
                    if direction == Directionality::Rtl {
                        "rtl"
                    } else {
                        "ltr"
                    }
                ))
                .unwrap()
            ),
            "selectors must consume the derived state for {id}"
        );
    }
    let input = tree
        .select(&Selector::parse("#input").unwrap())
        .next()
        .unwrap();
    assert_eq!(
        input.initial_state().input_type,
        Some(InputType::Text),
        "invalid input type must resolve before state consumers run"
    );
    let template = *elements
        .iter()
        .find(|element| element.name() == "template")
        .unwrap();
    let inert: Vec<_> = template
        .template_contents()
        .unwrap()
        .inclusive_descendants(TreeRelation::Selector)
        .filter_map(crate::dom::Node::element)
        .collect();
    assert_eq!(inert[0].initial_state().language.as_ref(), "ja");
    assert_eq!(inert[1].initial_state().language.as_ref(), "ja");
    assert_eq!(inert[1].initial_state().direction, Directionality::Rtl);
}

#[test]
fn initial_semantics_resolve_deep_ancestry_without_recursive_state_construction() {
    use crate::dom::Directionality;
    const DEPTH: usize = 20_000;
    let markup = format!(
        "<!doctype html><div lang=ar dir=rtl>{}<span id=leaf>Text</span>{}</div>",
        "<div>".repeat(DEPTH),
        "</div>".repeat(DEPTH)
    );
    let tree = DocumentTree::parse(&markup);
    let leaf = tree
        .select(&Selector::parse("#leaf").unwrap())
        .next()
        .unwrap();
    assert_eq!(leaf.initial_state().language.as_ref(), "ar");
    assert_eq!(leaf.initial_state().direction, Directionality::Rtl);
    assert!(leaf.matches(&Selector::parse(":lang(ar):dir(rtl)").unwrap()));
}

#[test]
fn element_identity_survives_owner_moves_but_never_retargets_another_document() {
    let tree = DocumentTree::parse("<p id=first>A</p>");
    let id = tree
        .select(&Selector::parse("#first").unwrap())
        .next()
        .unwrap()
        .id();
    let moved = tree;
    assert_eq!(moved.element(id).unwrap().attr("id"), Some("first"));
    let other = DocumentTree::parse("<p id=second>B</p>");
    assert!(
        other.element(id).is_none(),
        "a foreign document handle must never resolve to an unrelated element"
    );
}

#[test]
fn non_element_nodes_are_borrowed_in_formatting_order() {
    let tree = DocumentTree::parse("<p>A<!--comment--><b>B</b>C</p>");
    let element = tree.select(&Selector::parse("p").unwrap()).next().unwrap();
    let nodes: Vec<_> = element.child_nodes(TreeRelation::Formatting).collect();
    assert_eq!(nodes.len(), 4);
    let first: &str = nodes[0].value().as_text().unwrap();
    assert_eq!(first, "A");
    assert!(
        matches!(nodes[1].value(), NodeData::Comment(comment) if comment.comment.as_ref() == "comment")
    );
    assert_eq!(nodes[2].element().unwrap().name(), "b");
    let last: &str = nodes[3].value().as_text().unwrap();
    assert_eq!(last, "C");
}

#[test]
fn quirks_matching_uses_the_parsed_document_mode() {
    let selector = Selector::parse("#case.foo").unwrap();
    let quirks = DocumentTree::parse("<p id=Case class=FOO>Q</p>");
    assert!(
        quirks.select(&selector).next().is_some(),
        "quirks ID/class matching is ASCII insensitive"
    );
    let standard = DocumentTree::parse("<!doctype html><p id=Case class=FOO>S</p>");
    assert!(
        standard.select(&selector).next().is_none(),
        "standards ID/class matching remains case sensitive"
    );
}

#[test]
fn host_argument_metrics_include_native_function_boundaries() {
    for source in [":host(:is(.a))", ":host-context(:is(.a))"] {
        let selector = Selector::parse(source).expect("standard native host argument");
        assert_eq!(
            selector.depth(),
            3,
            "a host argument and :is add separate levels: {source}"
        );
        assert_eq!(
            selector.nesting_work(1),
            3,
            "native host, :is and class components are counted: {source}"
        );
    }
}

#[test]
fn alias_generated_selectors_use_the_native_grammar() {
    for source in [
        "::slotted(p)::before",
        "::slotted(p)::after",
        "x-host::part(label)::before",
    ] {
        assert!(
            Selector::parse(source).is_ok(),
            "tree-abiding generated pseudo-element: {source}"
        );
    }
    for source in [
        "::slotted(p).picked",
        "x-host::part(label).picked",
        "::slotted(p > q)",
    ] {
        assert!(
            Selector::parse(source).is_err(),
            "invalid alias grammar: {source}"
        );
    }
}

#[test]
fn encapsulation_positions_include_sibling_shadow_and_inert_tree_scopes() {
    let tree = DocumentTree::parse(
        "<!doctype html><div id=outer><template shadowrootmode=open><slot></slot></template><div id=light><template shadowrootmode=closed><p>Inner</p></template></div></div><template id=inert><p>Inert</p></template>",
    );
    let find = |selector: &str| {
        tree.select(&Selector::parse(selector).unwrap())
            .next()
            .unwrap()
    };
    let outer = find("#outer");
    let light = find("#light");
    let slot = outer
        .shadow_root()
        .unwrap()
        .children(TreeRelation::Selector)
        .find_map(crate::dom::Node::element)
        .unwrap();
    let inner = light
        .shadow_root()
        .unwrap()
        .children(TreeRelation::Selector)
        .find_map(crate::dom::Node::element)
        .unwrap();
    let inert = find("#inert")
        .template_contents()
        .unwrap()
        .children(TreeRelation::Selector)
        .find_map(crate::dom::Node::element)
        .unwrap();
    assert!(outer.is_in_document_tree());
    assert!(!slot.is_in_document_tree());
    assert!(!inert.is_in_document_tree());
    assert_eq!(outer.encapsulation_order(), 0);
    assert_eq!(light.encapsulation_order(), 0);
    assert!(
        slot.encapsulation_order() < inner.encapsulation_order(),
        "the assigning root precedes the slotted host's own root even though both have physical shadow depth1"
    );
    assert!(inner.encapsulation_order() < inert.encapsulation_order());
    assert_eq!(
        outer.scope_host(slot.tree_scope()).unwrap().id(),
        outer.id()
    );
    let foreign =
        DocumentTree::parse("<div><template shadowrootmode=open><p>Foreign</p></template></div>");
    assert!(
        foreign
            .root_element()
            .scope_host(slot.tree_scope())
            .is_none()
    );
}

/// The `id`s of the elements holding a shadow tree, clones of an element sharing its `id`.
fn shadow_hosts(tree: &DocumentTree) -> Vec<&str> {
    tree.shadow_including_elements()
        .filter(|element| element.shadow_root().is_some())
        .map(|element| element.attr("id").unwrap_or_default())
        .collect()
}

#[test]
fn use_elements_get_shadow_trees_where_they_render() {
    let tree = DocumentTree::parse(
        "<svg><rect id=r width=1 height=1 />\
         <defs><use id=defined href='#r'/><g><use id=grouped href='#r'/></g>\
         <clipPath><use id=clipping href='#r'/></clipPath><use id=imaged href='#r'/>\
         <filter><feImage href='#imaged'/></filter></defs>\
         <symbol id=s><use id=symbolized href='#r'/></symbol>\
         <use id=placed href='#s'/><use id=rendered href='#r'/></svg>",
    );
    let mut hosts = shadow_hosts(&tree);
    hosts.sort_unstable();
    assert_eq!(
        hosts,
        ["clipping", "imaged", "placed", "rendered", "symbolized"],
        "SVG 2 §5.3, §5.5: content of `defs` and `symbol` renders only through clones, which \
         hold their own shadow trees (the last `symbolized` is the clone in #placed); clip \
         path content and the element `feImage` references render where they are"
    );
}

#[test]
fn a_chain_of_use_elements_in_definitions_gets_shadow_trees_in_linear_number() {
    // Each `use` element of the chain holding a shadow tree, with the nested ones in its clone,
    // makes the shadow trees quadratic in the chain's length.
    const CHAIN: usize = 50;
    let uses: String = (1..=CHAIN)
        .map(|index| format!("<use id=u{index} href='#u{}'/>", index - 1))
        .collect();
    let tree = DocumentTree::parse(&format!(
        "<svg><defs><rect id=u0 width=1 height=1 />{uses}</defs><use id=rendered href='#u{CHAIN}'/></svg>"
    ));
    let hosts = shadow_hosts(&tree);
    assert_eq!(
        hosts.len(),
        CHAIN + 1,
        "shadow trees for a chain of {CHAIN} references"
    );
    let expected: Vec<String> = std::iter::once("rendered".to_owned())
        .chain((1..=CHAIN).rev().map(|index| format!("u{index}")))
        .collect();
    assert_eq!(
        hosts, expected,
        "the rendered use and the clone of each use it reaches hold one shadow tree each"
    );
}
