use crate::dom::{DocumentTree, Selector, XmlTree};

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

fn ids_matching(html: &str, selector: &str) -> Vec<String> {
    let tree = DocumentTree::parse(&format!("<!doctype html>{html}")).expect("parse test HTML");
    matched_ids(&tree, selector)
}

fn element_by_id<'a>(tree: &'a DocumentTree, id: &str) -> crate::dom::Element<'a> {
    tree.select(&Selector::parse(&format!("#{id}")).unwrap())
        .next()
        .unwrap_or_else(|| panic!("no element #{id}"))
}

#[test]
fn checkedness_selectedness_and_defaults_follow_initial_attributes() {
    let html = "<form><input id=box type=checkbox checked><input id=off type=checkbox>\
        <input id=r1 type=radio name=g checked><input id=r2 type=radio name=g checked>\
        <input id=lone type=radio><button id=reset type=reset>R</button>\
        <button id=submit>S</button><input id=later type=submit>\
        <select id=pick><option id=o1 disabled>a</option><option id=o2>b</option></select>\
        <select id=many multiple><option id=m1 selected>a</option><option id=m2>b</option></select></form>\
        <datalist><option id=d1 selected>x</option></datalist><progress id=busy></progress><progress id=done value=1></progress>";
    assert_eq!(
        ids_matching(html, ":checked"),
        ["box", "r2", "o2", "m1", "d1"],
        "the last checked radio of a group wins; a single select picks its first enabled option"
    );
    assert_eq!(
        ids_matching(html, ":unchecked"),
        ["off", "r1", "lone", "o1", "m2"],
        ":unchecked is every other checkable element"
    );
    assert_eq!(
        ids_matching(html, ":indeterminate"),
        ["lone", "busy"],
        "a group with no checked radio and a progress bar without a value are indeterminate"
    );
    assert_eq!(
        ids_matching(html, ":default"),
        ["box", "r1", "r2", "submit", "m1", "d1"],
        "checked/selected attributes and the form's first submit button are defaults"
    );
}

#[test]
fn placeholder_required_and_mutability_states_use_attribute_applicability() {
    let html = "<input id=empty placeholder=Hint><input id=filled placeholder=Hint value=x>\
        <input id=blank placeholder=''><input id=range type=range placeholder=Hint>\
        <textarea id=area placeholder=Hint></textarea><input id=need required>\
        <input id=color type=color required><select id=sel required></select>\
        <input id=ro readonly><input id=off disabled><input id=check type=checkbox>\
        <div id=edit contenteditable><p id=child>c</p><span id=fixed contenteditable=false>f</span></div>";
    assert_eq!(
        ids_matching(html, ":placeholder-shown"),
        ["empty", "area"],
        "only a nonempty placeholder that applies shows while the value is empty"
    );
    assert_eq!(ids_matching(html, ":required"), ["need", "sel"]);
    assert!(
        !ids_matching(html, ":optional").contains(&"color".to_owned()),
        "required does not apply to colour wells, so they are neither required nor optional"
    );
    assert_eq!(
        ids_matching(html, ":read-write"),
        ["empty", "filled", "blank", "area", "need", "edit", "child"],
        "mutable text controls and editing hosts' content are read-write"
    );
    let read_only = ids_matching(html, ":read-only");
    for id in ["range", "color", "ro", "off", "check", "fixed"] {
        assert!(
            read_only.contains(&id.to_owned()),
            "{id} must be read-only: {read_only:?}"
        );
    }
}

#[test]
fn constraint_validation_covers_missing_type_range_and_step_states() {
    for (control, valid) in [
        ("<input required>", Some(false)),
        ("<input required value=x>", Some(true)),
        ("<input required readonly>", None),
        ("<input required disabled>", None),
        ("<input type=email value='a@b.example'>", Some(true)),
        ("<input type=email value='a@b..example'>", Some(false)),
        (
            "<input type=email multiple value=' a@b.c , d@e.f '>",
            Some(true),
        ),
        ("<input type=url value='https://example.test/'>", Some(true)),
        (
            "<input type=url value='https://example.test/a b'>",
            Some(false),
        ),
        ("<input type=url value='relative/path'>", Some(false)),
        ("<input type=number min=2 value=1>", Some(false)),
        ("<input type=number max=2 value=3>", Some(false)),
        ("<input type=number step=0.1 value=0.3>", Some(true)),
        ("<input type=number step=2 min=1 value=4>", Some(false)),
        ("<input type=number value=1.5>", Some(true)),
        ("<input type=number min=0 value=1.5>", Some(false)),
        ("<input type=number step=any value=1.5>", Some(true)),
        (
            "<input type=date value=2026-10-28 step=7 min=2026-10-21>",
            Some(true),
        ),
        (
            "<input type=date value=2026-10-29 step=7 min=2026-10-21>",
            Some(false),
        ),
        ("<input type=week value=2026-W02 min=2026-W03>", Some(false)),
        ("<input type=month value=2026-02 max=2026-01>", Some(false)),
        (
            "<input type=time value=23:30 min=22:00 max=02:00>",
            Some(true),
        ),
        (
            "<input type=time value=12:00 min=22:00 max=02:00>",
            Some(false),
        ),
        ("<input type=time min=00:00 value=12:00:30>", Some(false)),
        (
            "<input type=datetime-local value='2026-10-29 12:00'>",
            Some(true),
        ),
        ("<input type=checkbox required>", Some(false)),
        ("<input type=file required>", Some(false)),
        (
            "<select required><option value=''>Pick</option><option>A</option></select>",
            Some(false),
        ),
        (
            "<select required><option value=''>Pick</option><option selected>A</option></select>",
            Some(true),
        ),
        (
            "<select required><optgroup><option value=''>Pick</option></optgroup></select>",
            Some(true),
        ),
        ("<textarea required></textarea>", Some(false)),
        ("<textarea required>\n</textarea>", Some(false)),
        ("<textarea required>x</textarea>", Some(true)),
    ] {
        let html = format!(
            "<form id=form><fieldset id=set>{}</fieldset></form>",
            control.replacen('>', " id=c>", 1)
        );
        let (valid_ids, invalid_ids) = (
            ids_matching(&html, ":valid"),
            ids_matching(&html, ":invalid"),
        );
        let expected: (&[&str], &[&str]) = match valid {
            Some(true) => (&["form", "set", "c"], &[]),
            Some(false) => (&[], &["form", "set", "c"]),
            None => (&["form", "set"], &[]),
        };
        assert_eq!(
            (
                valid_ids.iter().map(String::as_str).collect::<Vec<_>>(),
                invalid_ids.iter().map(String::as_str).collect::<Vec<_>>()
            ),
            (expected.0.to_vec(), expected.1.to_vec()),
            "{control}: (:valid, :invalid) matches; barred controls match neither"
        );
    }
    assert_eq!(
        ids_matching(
            "<input id=a type=radio name=g required><input id=b type=radio name=g><input id=x type=hidden required><button id=btn type=button></button>",
            ":invalid"
        ),
        ["a", "b"],
        "one required radio makes its whole group missing; hidden inputs and plain buttons are barred"
    );
    assert_eq!(
        ids_matching(
            "<datalist><input id=inside required></datalist>",
            ":valid, :invalid"
        ),
        [] as [String; 0],
        "datalist descendants are barred from constraint validation"
    );
}

#[test]
fn range_states_and_open_elements() {
    let html = "<input id=low type=number min=5 value=1><input id=ok type=number min=5 value=6>\
        <input id=none type=number value=6><input id=slider type=range>\
        <details id=shown open></details><details id=hidden></details><dialog id=dialog open></dialog>";
    assert_eq!(ids_matching(html, ":in-range"), ["ok", "slider"]);
    assert_eq!(ids_matching(html, ":out-of-range"), ["low"]);
    assert_eq!(ids_matching(html, ":open"), ["shown", "dialog"]);
    assert_eq!(
        ids_matching(
            html,
            ":modal, :popover-open, :autofill, :-webkit-autofill, :user-valid, :user-invalid"
        ),
        [] as [String; 0],
        "states only script or user interaction enter never match a static document"
    );
}

#[test]
fn value_sanitization_matches_each_type_state() {
    let tree = DocumentTree::parse(
        "<!doctype html><input id=text value='a&#10;b'><input id=url type=url value=' https://x.test/ '>\
        <input id=mail type=email multiple value=' a@b.c , d@e.f'><input id=num type=number value=1e3>\
        <input id=bad type=number value='1 '><input id=local type=datetime-local value='2026-10-29 08:05:00.500'>\
        <input id=whole type=datetime-local value='2026-10-29T08:05:00'><input id=box type=checkbox>\
        <input id=color type=color value=rebeccapurple><input id=nocolor type=color value=nope>\
        <input id=mid type=range min=0 max=100 step=20 value=50><input id=default type=range min=10 max=20>\
        <input id=clamped type=range min=0 max=10 value=99><input id=reversed type=range min=10 max=0>",
    )
    .unwrap();
    for (id, expected) in [
        ("text", "ab"),
        ("url", "https://x.test/"),
        ("mail", "a@b.c,d@e.f"),
        ("num", "1e3"),
        ("bad", ""),
        ("local", "2026-10-29T08:05:00.5"),
        ("whole", "2026-10-29T08:05"),
        ("box", "on"),
        ("color", "rebeccapurple"),
        ("nocolor", "#000000"),
        ("mid", "60"),
        ("default", "15"),
        ("clamped", "10"),
        ("reversed", "10"),
    ] {
        assert_eq!(
            element_by_id(&tree, id).input_value(),
            expected,
            "sanitized value of #{id}"
        );
    }
}

#[test]
fn meter_and_progress_points_are_clamped_in_specified_order() {
    let tree = DocumentTree::parse(
        "<!doctype html><meter id=m min=10 max=5 value=20 low=1 high=0 optimum=99></meter>\
        <meter id=plain value=0.25></meter><progress id=p value=7 max=-1></progress>",
    )
    .unwrap();
    let gauge = element_by_id(&tree, "m").meter_gauge();
    assert_eq!(
        [
            gauge.minimum,
            gauge.maximum,
            gauge.value,
            gauge.low,
            gauge.high,
            gauge.optimum
        ],
        [10.0; 6],
        "a maximum below the minimum collapses every point onto the minimum: {gauge:?}"
    );
    let plain = element_by_id(&tree, "plain").meter_gauge();
    assert_eq!(plain.optimum, 0.5, "the optimum defaults to the midpoint");
    assert_eq!(plain.region(), crate::dom::GaugeRegion::Optimum);
    assert_eq!(
        element_by_id(&tree, "p").progress_position(),
        Some(1.0),
        "a non-positive max falls back to 1.0 and the value clamps to it"
    );
}
