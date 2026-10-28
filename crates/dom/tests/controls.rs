use ferri_pdf_dom::{DocumentTree, Selector, XmlTree};

#[test]
fn initial_selection_uses_the_defined_option_list_and_local_disabled_state() {
    let source = roxmltree::Document::parse("<html xmlns='http://www.w3.org/1999/xhtml'><select id='select' disabled=''><optgroup disabled=''><optgroup><option id='nested-group'/></optgroup><option id='group-disabled'/></optgroup><hr><option id='hr-child'/></hr><datalist><option id='datalist-child'/></datalist><select><option id='nested-select'/></select><div><option id='first' value='chosen'><option id='option-child'/></option><option id='last'/></div></select></html>").unwrap();
    let tree = XmlTree::new(&source);
    let select = source
        .descendants()
        .find(|node| node.attribute("id") == Some("select"))
        .unwrap();
    let selected: Vec<_> = tree
        .element(select)
        .unwrap()
        .initial_selected_options()
        .iter()
        .map(|element| element.attr("id").unwrap().to_owned())
        .collect();
    assert_eq!(
        selected,
        ["first"],
        "select option traversal must prune nested groups, controls and option descendants, and ignore the select's own disabled state when choosing its initial option"
    );
    assert!(tree.matches(select, &Selector::parse(":disabled").unwrap()));
    assert!(
        !tree.matches(select, &Selector::parse(":blank").unwrap()),
        "a disabled select can still have a nonempty selected value"
    );
}

#[test]
fn xml_instance_controls_do_not_inherit_disabled_state_across_the_shadow_root() {
    const INSTANCE_NODE_LIMIT: usize = 8;
    let source = roxmltree::Document::parse("<html xmlns='http://www.w3.org/1999/xhtml'><fieldset disabled=''><input id='source'/></fieldset><fieldset id='host' disabled=''/></html>").unwrap();
    let input = source
        .descendants()
        .find(|node| node.attribute("id") == Some("source"))
        .unwrap();
    let host = source
        .descendants()
        .find(|node| node.attribute("id") == Some("host"))
        .unwrap();
    let mut tree = XmlTree::new(&source);
    let host_id = tree.element(host).unwrap().id();
    let scope = tree
        .create_instance(input, host_id, INSTANCE_NODE_LIMIT)
        .unwrap()
        .unwrap();
    let cloned = tree.element_in(input, Some(scope)).unwrap();
    assert_eq!(
        cloned.initial_state().disabled,
        Some(false),
        "the host is an inheritance parent but is not a DOM ancestor of its shadow input"
    );
    assert!(cloned.matches(&Selector::parse(":enabled").unwrap()));
    assert!(
        tree.matches(input, &Selector::parse(":disabled").unwrap()),
        "source state must remain unchanged after instance construction"
    );
}

fn matched_ids(tree: &DocumentTree, selector: &str) -> Vec<String> {
    tree.select(&Selector::parse(selector).expect("supported control state selector"))
        .filter_map(|element| element.attr("id").map(str::to_owned))
        .collect()
}

#[test]
fn disabled_fieldsets_exempt_only_their_first_legend_and_keep_outer_disabling() {
    let tree = DocumentTree::parse("<!doctype html><div id=unrelated disabled></div><fieldset id=outer disabled=false><legend><input id=first><fieldset id=nested disabled><legend><input id=nested-first></legend><input id=nested-body></fieldset></legend><legend><input id=later></legend><input id=body><fieldset id=inherited><legend><input id=inherited-first></legend></fieldset></fieldset><input id=hidden type=hidden><button id=button disabled></button><textarea id=textarea></textarea>").unwrap();
    assert_eq!(
        matched_ids(&tree, ":disabled"),
        [
            "outer",
            "nested",
            "nested-body",
            "later",
            "body",
            "inherited",
            "inherited-first",
            "button"
        ]
    );
    assert_eq!(
        matched_ids(&tree, ":enabled"),
        ["first", "nested-first", "hidden", "textarea"]
    );
    assert!(
        matched_ids(&tree, "#unrelated:is(:enabled,:disabled)").is_empty(),
        "a disabled attribute does not make an arbitrary element a form control"
    );
}

#[test]
fn options_and_groups_include_select_disabling_without_fieldset_shortcuts() {
    let tree = DocumentTree::parse("<!doctype html><select id=select disabled><optgroup id=group><option id=option>A</option></optgroup></select><select id=enabled-select><optgroup id=disabled-group disabled><option id=group-option>B</option></optgroup><optgroup id=enabled-group><option id=explicit disabled>C</option><option id=enabled-option>D</option></optgroup></select><fieldset id=fieldset disabled><option id=standalone-option>X</option><optgroup id=standalone-group></optgroup></fieldset><svg><g id=foreign disabled=''/></svg>").unwrap();
    assert_eq!(
        matched_ids(&tree, ":disabled"),
        [
            "select",
            "group",
            "option",
            "disabled-group",
            "group-option",
            "explicit",
            "fieldset"
        ]
    );
    assert_eq!(
        matched_ids(&tree, ":enabled"),
        [
            "enabled-select",
            "enabled-group",
            "enabled-option",
            "standalone-option",
            "standalone-group"
        ]
    );
}

#[test]
fn disabled_state_uses_dom_ancestry_across_shadow_and_slot_relationships() {
    for (outer, inner, disabled, enabled) in [
        (
            "disabled",
            "",
            vec!["outer", "light"],
            vec!["inner", "shadow"],
        ),
        (
            "",
            "disabled",
            vec!["inner", "shadow"],
            vec!["outer", "light"],
        ),
    ] {
        let tree = DocumentTree::parse(&format!("<!doctype html><fieldset id=outer {outer}><div><template shadowrootmode=open><fieldset id=inner {inner}><legend>Legend</legend><slot></slot><input id=shadow></fieldset></template><input id=light></div></fieldset>")).unwrap();
        for (selector, expected) in [(":disabled", disabled), (":enabled", enabled)] {
            let mut actual: Vec<_> = tree
                .shadow_including_elements()
                .filter(|element| element.matches(&Selector::parse(selector).unwrap()))
                .filter_map(|element| element.attr("id"))
                .collect();
            actual.sort_unstable();
            let mut expected = expected;
            expected.sort_unstable();
            assert_eq!(
                actual, expected,
                "{selector} must follow DOM descendants rather than CSS slot inheritance; outer={outer:?}, inner={inner:?}"
            );
        }
    }
}

#[test]
fn option_disabled_ancestry_stops_at_the_first_defined_boundary_in_xml() {
    let source = roxmltree::Document::parse("<html xmlns='http://www.w3.org/1999/xhtml'><optgroup disabled=''><optgroup><option id='nearest-enabled'/></optgroup><datalist><option id='datalist-boundary'/></datalist><hr><option id='hr-boundary'/></hr><option><option id='option-boundary'/></option><div><option id='disabled'/></div></optgroup><select disabled=''><optgroup><option id='select-disabled'/></optgroup></select></html>").unwrap();
    let tree = XmlTree::new(&source);
    for (id, disabled) in [
        ("nearest-enabled", false),
        ("datalist-boundary", false),
        ("hr-boundary", false),
        ("option-boundary", false),
        ("disabled", true),
        ("select-disabled", true),
    ] {
        let node = source
            .descendants()
            .find(|node| node.attribute("id") == Some(id))
            .unwrap();
        assert_eq!(
            tree.matches(node, &Selector::parse(":disabled").unwrap()),
            disabled,
            "option disabling boundary for {id}"
        );
        assert_eq!(
            tree.matches(node, &Selector::parse(":enabled").unwrap()),
            !disabled,
            "eligible option enabled state for {id}"
        );
    }
}
