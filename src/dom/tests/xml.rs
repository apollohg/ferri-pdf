use crate::dom::{Selector, XmlTree};

#[test]
fn xml_instance_initial_semantics_use_each_host_and_keep_source_state() {
    use crate::dom::Directionality;
    const INSTANCE_NODE_LIMIT: usize = 64;
    let source = roxmltree::Document::parse("<svg xmlns='http://www.w3.org/2000/svg' xmlns:h='http://www.w3.org/1999/xhtml' xml:lang='en'><h:div xml:lang='fr' dir='rtl'><g id='source'><text id='inherited'/><text id='explicit' xml:lang='de'/></g></h:div><h:div id='first' xml:lang='tr' dir='ltr'/><h:div id='second' xml:lang='ja' dir='rtl'/></svg>").unwrap();
    let find = |id| {
        source
            .descendants()
            .find(|node| node.attribute("id") == Some(id))
            .unwrap()
    };
    let mut tree = XmlTree::new(&source);
    for (host, language, direction) in [
        ("first", "tr", Directionality::Ltr),
        ("second", "ja", Directionality::Rtl),
    ] {
        let host_id = tree.element(find(host)).unwrap().id();
        let scope = tree
            .create_instance(find("source"), host_id, INSTANCE_NODE_LIMIT)
            .unwrap()
            .unwrap();
        let inherited = tree.element_in(find("inherited"), Some(scope)).unwrap();
        assert_eq!(
            inherited.initial_state().language.as_ref(),
            language,
            "instance language follows host {host}, not source ancestry"
        );
        assert_eq!(
            inherited.initial_state().direction,
            direction,
            "instance direction follows host {host}"
        );
        assert!(tree.matches_element(
            inherited.id(),
            &Selector::parse(&format!(":lang({language})")).unwrap(),
            None
        ));
        assert_eq!(
            tree.element_in(find("explicit"), Some(scope))
                .unwrap()
                .initial_state()
                .language
                .as_ref(),
            "de"
        );
        let original = tree.element(find("inherited")).unwrap();
        assert_eq!(
            original.initial_state().language.as_ref(),
            "fr",
            "creating an instance must not rewrite source state"
        );
        assert_eq!(original.initial_state().direction, Directionality::Rtl);
    }
}

#[test]
fn xml_source_handles_and_selector_caches_stay_with_their_own_document() {
    let source = roxmltree::Document::parse("<g><rect id='target'/></g>").unwrap();
    let other = roxmltree::Document::parse("<g><rect id='target'/></g>").unwrap();
    let tree = XmlTree::new(&source);
    let target = source.root_element().first_element_child().unwrap();
    let selector = Selector::parse("g > #target:first-child").unwrap();
    let id = tree.element(target).unwrap().id();
    assert!(tree.matches(target, &selector));
    let moved = tree;
    assert_eq!(moved.element(target).unwrap().id(), id);
    assert!(moved.matches(target, &selector));
    let foreign = other.root_element().first_element_child().unwrap();
    assert!(moved.element(foreign).is_none());
    assert!(
        !moved.matches(foreign, &selector),
        "identical foreign node indices cannot reuse this document's match cache"
    );
}

#[test]
fn xml_text_nodes_and_html_language_pragmas_keep_their_defined_semantics() {
    let source = roxmltree::Document::parse(
        "<svg xmlns='http://www.w3.org/2000/svg' xmlns:h='http://www.w3.org/1999/xhtml'><h:meta http-equiv='content-language' content='fr'/><g><!--note--><?test data?></g><g> </g><g>&#xA0;</g><g xml:lang='de-AT'/></svg>",
    )
    .unwrap();
    let tree = XmlTree::new(&source);
    let groups: Vec<_> = source
        .descendants()
        .filter(|node| node.has_tag_name("g"))
        .collect();
    let empty = Selector::parse("g:empty").unwrap();
    assert!(tree.matches(groups[0], &empty));
    assert!(tree.matches(groups[1], &empty));
    assert!(!tree.matches(groups[2], &empty));
    assert_eq!(tree.element(groups[0]).unwrap().language(), "fr");
    assert!(tree.matches(groups[0], &Selector::parse(":lang(fr)").unwrap()));
    assert!(tree.matches(groups[3], &Selector::parse(":lang(de)").unwrap()));
    assert_eq!(
        tree.element(groups[0])
            .unwrap()
            .child_nodes(crate::dom::TreeRelation::Selector)
            .count(),
        2
    );
}

#[test]
fn xml_instances_preserve_source_identity_with_bounded_distinct_shadow_scopes() {
    use crate::dom::{DomError, TreeRelation};
    const INSTANCE_NODES: usize = 3;
    let source = roxmltree::Document::parse(
        "<svg><g class='outside'><g id='branch'><g/><rect id='target'/></g></g><use id='first'/><use id='second'/></svg>",
    ).unwrap();
    let foreign = roxmltree::Document::parse("<svg><rect/></svg>").unwrap();
    let find = |id| {
        source
            .descendants()
            .find(|node| node.attribute("id") == Some(id))
            .unwrap()
    };
    let branch = find("branch");
    let target = find("target");
    let mut tree = XmlTree::new(&source);
    let original = tree.element(target).unwrap().id();
    let first_host = tree.element(find("first")).unwrap().id();
    let second_host = tree.element(find("second")).unwrap().id();
    let ancestor = Selector::parse(".outside #target:nth-child(2)").unwrap();
    assert!(tree.matches(target, &ancestor));
    assert!(
        tree.create_instance(foreign.root_element(), first_host, INSTANCE_NODES)
            .unwrap()
            .is_none()
    );
    let first = tree
        .create_instance(branch, first_host, INSTANCE_NODES)
        .unwrap()
        .unwrap();
    let instance = tree.element_in(target, Some(first)).unwrap();
    assert_ne!(instance.id(), original);
    assert_eq!(instance.tree_scope(), first);
    assert!(!tree.matches_element(instance.id(), &ancestor, None));
    assert!(tree.matches_element(
        instance.id(),
        &Selector::parse("#branch > #target:nth-child(2)").unwrap(),
        None
    ));
    let root = tree.element_in(branch, Some(first)).unwrap();
    assert!(root.parent(TreeRelation::Selector).is_none());
    assert_eq!(
        root.parent(TreeRelation::Inheritance).unwrap().id(),
        first_host
    );
    assert!(tree.matches_element(
        root.id(),
        &Selector::parse(":host(#first) > #branch").unwrap(),
        None
    ));
    assert!(matches!(
        tree.create_instance(branch, second_host, INSTANCE_NODES),
        Err(DomError::TreeLimit {
            limit: INSTANCE_NODES,
            ..
        })
    ));
    assert!(
        tree.element(find("second"))
            .unwrap()
            .shadow_root()
            .is_none(),
        "budget denial must not attach a partial instance"
    );
    assert!(tree.matches(target, &ancestor));
    let second = tree
        .create_instance(branch, second_host, INSTANCE_NODES * 2)
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    assert_ne!(
        tree.element_in(target, Some(first)).unwrap().id(),
        tree.element_in(target, Some(second)).unwrap().id()
    );
    assert_eq!(tree.element(target).unwrap().id(), original);
    assert!(
        tree.matches(target, &ancestor),
        "growing the instance graph cannot retain stale source-match pointers"
    );
    assert!(
        tree.element_in(find("first"), Some(second)).is_none(),
        "source nodes outside the referenced subtree are absent from the instance"
    );
    assert!(
        tree.element_in(foreign.root_element(), Some(first))
            .is_none()
    );
    let foreign_tree = XmlTree::new(&foreign);
    assert!(!tree.matches_element(
        foreign_tree.element(foreign.root_element()).unwrap().id(),
        &Selector::parse("*").unwrap(),
        None
    ));
}

#[test]
fn tree_counting_positions_are_invalidated_when_an_xml_instance_changes_the_flat_tree() {
    use crate::dom::TreeRelation;
    const INSTANCE_NODE_LIMIT: usize = 32;
    let source = roxmltree::Document::parse("<svg><defs><g id='source'><rect/><circle/></g></defs><use id='host'><fallback/><fallback/></use></svg>").unwrap();
    let root = source.root_element();
    let referenced = root
        .first_element_child()
        .unwrap()
        .first_element_child()
        .unwrap();
    let host = root.last_element_child().unwrap();
    let fallback = host.first_element_child().unwrap();
    let mut tree = XmlTree::new(&source);
    let host_id = tree.element(host).unwrap().id();
    let before = tree.element(fallback).unwrap();
    assert_eq!(before.sibling_position(TreeRelation::Inheritance), (1, 2));
    assert_eq!(before.sibling_position(TreeRelation::Selector), (1, 2));
    let scope = tree
        .create_instance(referenced, host_id, INSTANCE_NODE_LIMIT)
        .unwrap()
        .unwrap();
    let after = tree.element(fallback).unwrap();
    assert!(
        after.parent(TreeRelation::Inheritance).is_none(),
        "the instance replaces the host's original flat children"
    );
    assert_eq!(
        after.sibling_position(TreeRelation::Inheritance),
        (1, 1),
        "cached positions cannot survive a change in parentage"
    );
    assert_eq!(
        after.sibling_position(TreeRelation::Selector),
        (1, 2),
        "the light tree remains a distinct relationship"
    );
    let circle = tree
        .element_in(referenced.last_element_child().unwrap(), Some(scope))
        .unwrap();
    assert_eq!(
        circle.sibling_position(TreeRelation::Inheritance),
        (2, 2),
        "cloned nodes have their own positions in the instance"
    );
}

#[test]
fn xml_namespace_declarations_are_local_attributes_with_normalized_values() {
    const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";
    const XML_URI: &str = "http://www.w3.org/XML/1998/namespace";
    let source = roxmltree::Document::parse_with_options(
        "<!DOCTYPE root [<!ENTITY ns 'urn:entity'>]><root xmlns='urn:root' xmlns:s = \"&ns;\" data='xmlns:fake=\"x\" />' xmlns:xml='http://www.w3.org/XML/1998/namespace'><child xmlns:s='urn:entity'/><child/><child xmlns=''/><child\nxmlns:t='urn:t'\n/></root>",
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .unwrap();
    let tree = XmlTree::new(&source);
    let declarations = |node: roxmltree::Node<'_, '_>| {
        let mut declarations: Vec<_> = tree
            .element(node)
            .unwrap()
            .value()
            .attrs
            .iter()
            .filter(|(name, _)| name.ns.as_ref() == XMLNS_NAMESPACE)
            .map(|(name, value)| (name.local.to_string(), value.to_string()))
            .collect();
        declarations.sort();
        eprintln!("{:?} declarations: {declarations:?}", node.tag_name());
        declarations
    };
    let owned = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<Vec<_>>()
    };
    let root = source.root_element();
    assert_eq!(
        declarations(root),
        owned(&[("s", "urn:entity"), ("xml", XML_URI), ("xmlns", "urn:root")]),
        "quoted attribute text must not be read as a declaration"
    );
    let children: Vec<_> = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect();
    assert_eq!(
        declarations(children[0]),
        owned(&[("s", "urn:entity")]),
        "redundant redeclarations stay local attributes"
    );
    assert!(
        declarations(children[1]).is_empty(),
        "inherited bindings are not local attributes"
    );
    assert_eq!(declarations(children[2]), owned(&[("xmlns", "")]));
    assert_eq!(declarations(children[3]), owned(&[("t", "urn:t")]));
}

#[test]
fn has_selectors_keep_matching_once_their_subtree_filters_are_built() {
    // An anchor builds a filter of its subtree's hashes the second time a relative selector
    // searches it with the same caches, as a two-argument `:has()` and a repeated pass do; a
    // hash the filter lacked would reject a present descendant or sibling.
    let source = roxmltree::Document::parse("<svg xmlns='http://www.w3.org/2000/svg'><g id='outer'><g id='inner' class='Box'><circle id='dot' class='dot'/></g><rect id='after'/></g><g id='empty'/></svg>").unwrap();
    let tree = XmlTree::new(&source);
    // The caches key relative selectors by address, so every selector outlives the matching.
    let selectors = [
        ("g:has(.absent, .dot)", &["outer", "inner"][..]),
        ("g:has(#absent, #dot)", &["outer", "inner"]),
        ("g:has(> line, > circle)", &["inner"]),
        ("g:has(.absent, .Box)", &["outer"]),
        ("g:has(+ line, + rect)", &["inner"]),
        ("g:has(~ line, ~ g)", &["outer"]),
    ]
    .map(|(selector, expected)| (selector, Selector::parse(selector).unwrap(), expected));
    for (selector, parsed, expected) in &selectors {
        for pass in 0..2 {
            let matched: Vec<_> = source
                .descendants()
                .filter(|node| node.is_element() && tree.matches(*node, parsed))
                .filter_map(|node| node.attribute("id"))
                .collect();
            assert_eq!(&matched, expected, "{selector} on pass {pass}");
        }
    }
}
